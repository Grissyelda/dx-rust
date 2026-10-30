//! Builds a renderable level from a Deus Ex map and the user's texture packages.
//!
//! This crate is engine-agnostic: it produces plain vertex/index arrays and RGBA mip chains in
//! Bevy-style coordinates (Y up, meters), which a renderer can upload directly.
//!
//! Unreal units are converted at `METERS_PER_UNIT`. Deus Ex uses 16 units per foot, so one unit is
//! about 1.9 cm and a 1.8 m character is roughly 95 units tall.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use unreal_pkg::objects::{self, pf, Palette, Texture};
use unreal_pkg::props::{self, Value};
use unreal_pkg::{Package, Reader};

pub mod collision;
pub mod conversations;
pub mod actors;
pub mod lighting;
pub mod movers;
pub mod nav;
pub mod firetex;
pub mod skincolor;
pub mod dds;

pub const METERS_PER_UNIT: f32 = 1.0 / 52.5;

/// Convert an Unreal position (X, Y, Z-up, left-handed) to Bevy space (X, Y-up, Z) in meters.
pub fn ue_to_bevy(p: [f32; 3]) -> [f32; 3] {
    [p[0] * METERS_PER_UNIT, p[2] * METERS_PER_UNIT, p[1] * METERS_PER_UNIT]
}

/// Inverse of `ue_to_bevy`.
pub fn bevy_to_ue(p: [f32; 3]) -> [f32; 3] {
    [p[0] / METERS_PER_UNIT, p[2] / METERS_PER_UNIT, p[1] / METERS_PER_UNIT]
}

/// How a surface should be blended when drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SurfaceClass {
    Opaque,
    /// Color-keyed transparency (palette index 0 is transparent).
    Masked,
    Translucent,
    /// Water-like translucent surface (drawn with alpha blending).
    Water,
    /// Multiplies what is behind it (Unreal STY_Modulated / PF_Modulated), e.g. sunglasses lenses.
    Modulated,
}

/// One texture with its full mip chain, decoded to RGBA8.
#[derive(Debug, Clone)]
pub struct TextureImage {
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// Mip 0 first. Each entry is `w * h * 4` bytes for that level (RGBA8), or with `bc1` the level's DXT1 blocks
    /// as stored (8 bytes per 4 x 4 block), handed to the GPU still compressed.
    pub mips: Vec<Vec<u8>>,
    pub bc1: bool,
    /// The mips are DXT5 (BC3) blocks as stored (16 bytes per 4 x 4 block): DDS override packs.
    pub bc3: bool,
}

impl TextureImage {
    /// Has see-through texels (alpha 0): RGBA with a zero alpha, or DXT1 blocks in 3-colour mode using index 3.
    pub fn has_alpha_holes(&self) -> bool {
        let Some(m) = self.mips.first() else { return false };
        if self.bc3 {
            m.chunks_exact(16).any(|b| b[0] == 0 || b[1] == 0)
        } else if self.bc1 {
            bc1_has_holes(m)
        } else {
            m.chunks_exact(4).any(|p| p[3] == 0)
        }
    }
}

/// DXT1 blocks that decode to transparent texels (colour0 <= colour1 and an index of 3).
pub fn bc1_has_holes(blocks: &[u8]) -> bool {
    blocks.chunks_exact(8).any(|b| {
        let c0 = u16::from_le_bytes([b[0], b[1]]);
        let c1 = u16::from_le_bytes([b[2], b[3]]);
        if c0 > c1 {
            return false;
        }
        let idx = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
        (0..16).any(|k| (idx >> (k * 2)) & 3 == 3)
    })
}

/// The GPU takes DXT1 (BC1) textures as they are stored (set by the viewer when the device supports BC formats):
/// no decoding at load time and an eighth of the video memory of RGBA8.
static BC1_OK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_bc1_supported(on: bool) {
    BC1_OK.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn bc1_supported() -> bool {
    BC1_OK.load(std::sync::atomic::Ordering::Relaxed)
}

/// A DXT1 texture's mip chain as BC1 blocks: mip 0 a multiple of 4 on both sides, each level the right size;
/// the chain stops at the first level that doesn't fit. None for other formats or sizes.
fn bc1_mips(tex: &Texture) -> Option<Vec<Vec<u8>>> {
    if tex.format != unreal_pkg::objects::tex_format::DXT1 {
        return None;
    }
    let (w0, h0) = (tex.width() as usize, tex.height() as usize);
    if w0 == 0 || h0 == 0 || w0 % 4 != 0 || h0 % 4 != 0 || w0 > 8192 || h0 > 8192 {
        return None;
    }
    let mut out = Vec::new();
    for (k, m) in tex.mips.iter().enumerate() {
        let (w, h) = ((w0 >> k).max(1), (h0 >> k).max(1));
        if m.width as usize != w || m.height as usize != h {
            break;
        }
        let need = w.div_ceil(4) * h.div_ceil(4) * 8;
        if m.data.len() < need {
            break;
        }
        out.push(m.data[..need].to_vec());
    }
    (!out.is_empty()).then_some(out)
}

/// All triangles sharing one texture and blend class.
#[derive(Debug, Clone)]
pub struct Batch {
    /// Index into `Level::textures`; `None` draws with the placeholder material.
    pub texture: Option<usize>,
    pub class: SurfaceClass,
    /// True for geometry in the sky room (drawn as a distant backdrop around the camera).
    pub sky: bool,
    /// Texture scroll in UV units per second (from PF_AutoUPan/PF_AutoVPan and the zone's pan speed).
    pub uv_scroll: [f32; 2],
    /// PF_Mirrored: the face's plane in Bevy space (unit normal, distance in meters along it), for a reflection.
    pub mirror: Option<[f32; 4]>,
    /// Water drawn solid (a zone-portal water sheet without PF_Translucent, like Battery Park's harbour): you
    /// can't see below it from above.
    pub water_opaque: bool,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Lightmap atlas coordinates (second UV set).
    pub uvs1: Vec<[f32; 2]>,
    /// Per-vertex lighting (linear RGBA) for meshes lit like Unreal actors; empty for lightmapped BSP.
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct PlayerStart {
    /// Bevy-space position in meters.
    pub position: [f32; 3],
    /// Yaw in radians (0 = looking along +Unreal X).
    pub yaw: f32,
}

#[derive(Debug, Default, Clone)]
pub struct LevelStats {
    pub nodes: usize,
    pub triangles: usize,
    pub textures: usize,
    pub missing_textures: usize,
    pub skipped_invisible: usize,
    pub skipped_backdrop: usize,
    pub actors: usize,
}

#[derive(Debug, Clone)]
pub struct Level {
    pub name: String,
    pub textures: Vec<TextureImage>,
    pub batches: Vec<Batch>,
    pub player_start: Option<PlayerStart>,
    /// Placed mesh actors (decorations, pickups, characters) in actor-local space.
    pub mesh_actors: Vec<actors::MeshActor>,
    /// Doors, lifts and other movers.
    pub movers: Vec<movers::MoverInfo>,
    /// Static lights (for lighting actors).
    pub lights: Vec<lighting::LightInfo>,
    /// Lights drawn with a corona (bCorona + Skin): position, texture path, colour, DrawScale.
    pub coronas: Vec<Corona>,
    /// Faces textured from a "Ladder" texture group (Unreal units): touching one lets you climb.
    pub ladders: Vec<LadderFace>,
    /// Upward-facing BSP faces and their texture group ("Metal", "Wood", "Earth"...): footstep sounds by floor.
    pub floors: Vec<FloorFace>,
    /// Walls and ceilings (for GetWallMaterial): plane, 3D bounds, texture group.
    pub walls: Vec<WallFace>,
    pub floor_groups: Vec<String>,
    /// AmbientSound actors (looping positional sounds) and the level's music (Song path, SongSection).
    pub ambient_sounds: Vec<actors::AmbientSoundInfo>,
    pub song: Option<(String, u8)>,
    /// Every placed actor with its effective properties (class defaults + placed values).
    pub actors: Vec<actors::ActorInfo>,
    /// PickupDistributor.NanoKeyData: (ScriptedPawnTag, KeyID, Description) - nanokeys pawns carry from the start.
    pub nano_keys: Vec<(String, String, String)>,
    /// Per BSP zone: water (WaterZone / bWaterZone) and its fluid friction.
    pub zone_water: Vec<Option<f32>>,
    /// Per BSP zone: open to the sky (it has fake-backdrop faces looking into the sky room; the sky room itself
    /// excluded) - where weather falls.
    pub zone_sky: Vec<bool>,
    /// AI path network (NavigationPoints + ReachSpecs).
    pub nav: nav::NavGraph,
    /// Blocking actor cylinders (decorations, pawns), Unreal units.
    pub blockers: Vec<collision::Cylinder>,
    /// Cylinders that block pawns but not the player (BlockMonsters: bBlockActors without bBlockPlayers).
    pub monster_blocks: Vec<collision::Cylinder>,
    /// Actors (by `actors` index) whose cylinder blocks only players (BlockPlayer: bBlockPlayers without
    /// bBlockActors). Shots, projectiles, sight lines and other pawns pass through them.
    pub players_only: std::collections::HashSet<usize>,
    /// World collision (Unreal units).
    pub collision: collision::BspCollision,
    /// All surface lightmaps packed into one RGBA8 (sRGB-encoded) atlas.
    pub lightmap: Option<lighting::Atlas>,
    /// Bevy-space meters location of the sky zone, if any; sky batches are positioned in the same space.
    pub sky_origin: Option<[f32; 3]>,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub stats: LevelStats,
    /// The map package's name table (struct properties hold names as indices into it).
    pub names: Vec<String>,
}

/// Loads texture packages lazily and decodes textures.
pub struct TexLib {
    dir: PathBuf,
    /// The game's System folder: textures embedded in code packages (mesh skins) live in `.u` files there.
    sys_dir: PathBuf,
    pkgs: HashMap<String, Option<Package>>,
    /// New Vision (an HD texture mod): its packages of the same names, opened lazily.
    hd_pkgs: HashMap<String, Option<Package>>,
    /// While set, textures come from an HD texture mod where it has them (level surfaces and mesh skins; not
    /// sprites or the UI, whose sizes come from the picture).
    pub hd_scope: bool,
    /// Textures taken from the HD texture mod so far.
    pub hd_used: usize,
    /// Textures taken from a DDS override pack so far.
    pub dds_used: usize,
    /// Texture properties read without decoding pixels: (USize, VSize, bMasked).
    info: HashMap<String, Option<(u32, u32, bool)>>,
}

impl TexLib {
    /// Texture loader for a game install (looks in `Textures/*.utx`, then `System/*.u`).
    pub fn new(game_root: &Path) -> TexLib {
        TexLib { dir: textures_dir(game_root), sys_dir: game_root.join("System"), pkgs: HashMap::new(), hd_pkgs: HashMap::new(), hd_scope: false, hd_used: 0, dds_used: 0, info: HashMap::new() }
    }

    /// A texture's size (USize / VSize) and bMasked from its properties alone - no pixels are decoded.
    pub fn info(&mut self, path: &str) -> Option<(u32, u32, bool)> {
        let key = path.to_ascii_lowercase();
        if let Some(v) = self.info.get(&key) {
            return *v;
        }
        let v = (|| {
            let (pkg_name, rest) = path.split_once('.')?;
            let pkg = self.package(pkg_name)?;
            let e = pkg.find_export_of_class(rest, "Texture")?;
            if !pkg.class_name(e.class_index).contains("Texture") {
                return None;
            }
            let (_, ps) = objects::open_object(pkg, e).ok()?;
            let ip = |n: &str| match unreal_pkg::props::find(&ps, n) {
                Some(unreal_pkg::props::Value::Int(v)) if *v > 0 => *v as u32,
                _ => 0,
            };
            let masked = matches!(unreal_pkg::props::find(&ps, "bMasked"), Some(unreal_pkg::props::Value::Bool(true)));
            Some((ip("USize"), ip("VSize"), masked))
        })();
        self.info.insert(key, v);
        v
    }

    /// A texture's width (USize) without decoding it.
    pub fn width(&mut self, path: &str) -> Option<u32> {
        match self.info(path) {
            Some((w, _, _)) if w > 0 => Some(w),
            _ => self.load(path).map(|(t, _)| t.width()),
        }
    }

    /// The game's System folder.
    pub fn system_dir(&self) -> &Path {
        &self.sys_dir
    }

    /// Decode a texture by full path (`Package.Group.Name`) with all mips.
    pub fn load_image(&mut self, path: &str) -> Option<TextureImage> {
        let (t, mips) = self.load_ext(path, false)?;
        Some(TextureImage { name: path.to_string(), width: t.width(), height: t.height(), mips, bc1: false, bc3: false })
    }

    /// A WetTexture (procedural ripples over a SourceTexture) as a still: the source picture through the wet
    /// texture's own palette. Effects such as gas clouds use these.
    pub fn load_wet(&mut self, path: &str) -> Option<TextureImage> {
        let (pkg_name, rest) = path.split_once('.')?;
        let pkg = self.package(pkg_name)?;
        let e = pkg.find_export_of_class(rest, "WetTexture")?;
        if pkg.class_name(e.class_index) != "WetTexture" {
            return None;
        }
        let wet = objects::read_texture(pkg, e).ok()?;
        let src_ref = match unreal_pkg::props::find(&wet.props, "SourceTexture") {
            Some(unreal_pkg::props::Value::Object(i)) if *i > 0 => *i,
            _ => return None,
        };
        let se = pkg.exports.get(src_ref as usize - 1)?;
        let mut src = objects::read_texture(pkg, se).ok()?;
        src.palette_ref = wet.palette_ref;
        let palette = self.palette_of(pkg_name, src.palette_ref);
        let mut mips = Vec::new();
        for level in 0..src.mips.len() {
            match src.mip_to_rgba8(level, palette.as_ref()) {
                Some(m) => mips.push(m),
                None => break,
            }
        }
        if mips.is_empty() {
            return None;
        }
        Some(TextureImage { name: path.to_string(), width: src.width(), height: src.height(), mips, bc1: false, bc3: false })
    }

    /// Like `load_image` but palette index 0 is always transparent (UI icons are drawn masked).
    pub fn load_image_masked(&mut self, path: &str) -> Option<TextureImage> {
        let (t, mips) = self.load_ext(path, true)?;
        Some(TextureImage { name: path.to_string(), width: t.width(), height: t.height(), mips, bc1: false, bc3: false })
    }

    fn package(&mut self, name: &str) -> Option<&Package> {
        let key = name.to_ascii_lowercase();
        if !self.pkgs.contains_key(&key) {
            let p = Package::open(self.dir.join(format!("{name}.utx")))
                .or_else(|_| Package::open(self.sys_dir.join(format!("{name}.u"))))
                .ok();
            self.pkgs.insert(key.clone(), p);
        }
        self.pkgs.get(&key).and_then(|p| p.as_ref())
    }

    pub fn load(&mut self, path: &str) -> Option<(Texture, Vec<Vec<u8>>)> {
        self.load_ext(path, false)
    }

    fn palette_of(&mut self, pkg_name: &str, palette_ref: i32) -> Option<Palette> {
        let pkg = self.package(pkg_name)?;
        if palette_ref > 0 {
            let pe = pkg.exports.get(palette_ref as usize - 1)?;
            objects::read_palette(pkg, pe).ok()
        } else if palette_ref < 0 {
            let full = pkg.object_path(palette_ref);
            let (ppkg, prest) = full.split_once('.')?;
            let (ppkg, prest) = (ppkg.to_string(), prest.to_string());
            let pk = self.package(&ppkg)?;
            let pe = pk.find_export_by_path(&prest)?;
            objects::read_palette(pk, pe).ok()
        } else {
            None
        }
    }

    /// A procedural FireTexture, ready to animate (None for ordinary textures).
    pub fn load_fire(&mut self, path: &str) -> Option<firetex::FireTex> {
        let (pkg_name, rest) = path.split_once('.')?;
        let pkg = self.package(pkg_name)?;
        let e = pkg.find_export_of_class(rest, "Texture")?;
        if pkg.class_name(e.class_index) != "FireTexture" {
            return None;
        }
        let tex = objects::read_texture(pkg, e).ok()?;
        let pal = self.palette_of(pkg_name, tex.palette_ref)?;
        let mut colors = [[0u8; 4]; 256];
        for (i, c) in pal.colors.iter().take(256).enumerate() {
            colors[i] = [c[0], c[1], c[2], 255];
        }
        let ip = |n: &str, d: i32| match unreal_pkg::props::find(&tex.props, n) {
            Some(unreal_pkg::props::Value::Int(v)) => *v,
            Some(unreal_pkg::props::Value::Byte(v)) => *v as i32,
            _ => d,
        };
        // FireTexture's class default is bRising = False (only non-default values are saved, so a missing
        // property means false).
        let rising = matches!(unreal_pkg::props::find(&tex.props, "bRising"), Some(unreal_pkg::props::Value::Bool(true)));
        let w = ip("UClamp", tex.width() as i32).max(1) as usize;
        let h = ip("VClamp", tex.height() as i32).max(1) as usize;
        let mut f = firetex::FireTex::new(w, h, &tex.sparks, colors, ip("RenderHeat", 255) as u8, rising, ip("SparksLimit", 1024) as usize);
        // Stand-ins where the spark simulation falls short (see firetex::Mode).
        let lower = rest.to_ascii_lowercase();
        if lower.ends_with("flame_b") {
            f.mode = firetex::Mode::Ball;
        } else if lower.ends_with("flmethrwr_flme") {
            f.mode = firetex::Mode::Pilot;
        }
        Some(f)
    }

    /// A texture's animation (Texture.AnimNext chain): every frame's path from this one round the loop, and the
    /// frame rates (MinFrameRate, MaxFrameRate). None when the texture doesn't animate.
    pub fn anim_chain(&mut self, path: &str) -> Option<(Vec<String>, f32, f32)> {
        let mut frames = vec![path.to_string()];
        let mut rates = (0.0, 0.0);
        let mut cur = path.to_string();
        for i in 0..64 {
            let (pkg_name, rest) = cur.split_once('.')?;
            let (pkg_name, rest) = (pkg_name.to_string(), rest.to_string());
            let pkg = self.package(&pkg_name)?;
            let rest = rest.as_str();
            let e = pkg.find_export_of_class(rest, "Texture")?;
            let (_r, ps) = objects::open_object(pkg, e).ok()?;
            let fl = |n: &str| match props::find(&ps, n) {
                Some(Value::Float(v)) => *v,
                _ => 0.0,
            };
            if i == 0 {
                rates = (fl("MinFrameRate"), fl("MaxFrameRate"));
            }
            let next = match props::find(&ps, "AnimNext") {
                Some(Value::Object(o)) if *o != 0 => *o,
                _ => break,
            };
            let np = pkg.object_path(next);
            // Imports name their own package; exports are relative to this one.
            let full = if next > 0 { format!("{pkg_name}.{np}") } else { np };
            if full.eq_ignore_ascii_case(path) || frames.iter().any(|f| f.eq_ignore_ascii_case(&full)) {
                break;
            }
            frames.push(full.clone());
            cur = full;
        }
        (frames.len() > 1).then_some((frames, rates.0, rates.1))
    }

    fn hd_package(&mut self, name: &str) -> Option<&Package> {
        let key = name.to_ascii_lowercase();
        if !self.hd_pkgs.contains_key(&key) {
            let p = unreal_pkg::paths::hd_texture_dirs()
                .iter()
                .find_map(|d| unreal_pkg::paths::find_ci(d, &format!("{name}.utx")))
                .and_then(|f| Package::open(f).ok());
            self.hd_pkgs.insert(key.clone(), p);
        }
        self.hd_pkgs.get(&key).and_then(|p| p.as_ref())
    }

    /// The HD texture mod's version of a texture, when it has one stored compressed (New Vision keeps the game's
    /// palettized mips and adds a DXT1 set at up to 1024 x 1024; its USize / VSize stay the game's).
    fn load_hd_tex(&mut self, path: &str, force_mask: bool) -> Option<(Texture, Vec<Vec<u8>>)> {
        if !self.hd_scope || !unreal_pkg::paths::has_hd_textures() {
            return None;
        }
        let (pkg_name, rest) = path.split_once('.')?;
        let pkg = self.hd_package(pkg_name)?;
        let e = pkg.find_export_of_class(rest, "Texture")?;
        if pkg.class_name(e.class_index) != "Texture" {
            return None;
        }
        let mut tex = objects::read_texture(pkg, e).ok()?;
        if tex.format == unreal_pkg::objects::tex_format::P8 {
            return None;
        }
        if force_mask {
            tex.masked = true;
        }
        let mut mips = Vec::new();
        for level in 0..tex.mips.len() {
            match tex.mip_to_rgba8(level, None) {
                Some(m) => mips.push(m),
                None => break,
            }
        }
        if mips.is_empty() {
            return None;
        }
        // Masked textures: DXT1's punch-through alpha already marks the see-through texels; opaque ones keep
        // alpha 255 everywhere.
        if !tex.masked {
            for m in &mut mips {
                for px in m.chunks_exact_mut(4) {
                    px[3] = 255;
                }
            }
        }
        Some((tex, mips))
    }

    /// A texture's size as the game lays it out (USize / VSize): UVs on level surfaces are in these texels even
    /// when an HD texture with more pixels is drawn.
    pub fn logical_size(t: &Texture) -> (u32, u32) {
        let ip = |n: &str, d: u32| match unreal_pkg::props::find(&t.props, n) {
            Some(unreal_pkg::props::Value::Int(v)) if *v > 0 => *v as u32,
            _ => d,
        };
        (ip("USize", t.width()), ip("VSize", t.height()))
    }

    /// A texture for the GPU: DXT1 ones (the HD mod's version first) stay compressed when the device takes BC1
    /// and nothing has to see through blocks the game draws opaque; the rest decode to RGBA8 as `load_ext`.
    pub fn load_gpu(&mut self, path: &str, force_mask: bool) -> Option<(Texture, TextureImage)> {
        if let Some(r) = self.load_dds(path, force_mask, true) {
            return Some(r);
        }
        if bc1_supported() {
            let hd = if self.hd_scope && unreal_pkg::paths::has_hd_textures() { self.read_raw(path, true) } else { None };
            let from_hd = hd.is_some();
            if let Some(tex) = hd.or_else(|| self.read_raw(path, false)) {
                if let Some(mips) = bc1_mips(&tex) {
                    if tex.masked || force_mask || !bc1_has_holes(&mips[0]) {
                        if from_hd {
                            self.hd_used += 1;
                        }
                        let img = TextureImage { name: path.to_string(), width: tex.width(), height: tex.height(), mips, bc1: true, bc3: false };
                        return Some((tex, img));
                    }
                }
            }
        }
        let (mut t, mips) = self.load_ext(path, force_mask)?;
        if force_mask {
            // The texture's own bMasked (force_mask only decides how it decodes).
            t.masked = unreal_pkg::props::find(&t.props, "bMasked").is_some_and(|v| matches!(v, unreal_pkg::props::Value::Bool(true)));
        }
        let img = TextureImage { name: path.to_string(), width: t.width(), height: t.height(), mips, bc1: false, bc3: false };
        Some((t, img))
    }

    /// The texture object as stored (the HD mod's package with `hd`).
    fn read_raw(&mut self, path: &str, hd: bool) -> Option<Texture> {
        let (pkg_name, rest) = path.split_once('.')?;
        let pkg = if hd { self.hd_package(pkg_name)? } else { self.package(pkg_name)? };
        let e = pkg.find_export_of_class(rest, "Texture")?;
        if pkg.class_name(e.class_index) != "Texture" {
            return None;
        }
        objects::read_texture(pkg, e).ok()
    }

    /// A DDS override pack's version of a texture (unreal_pkg::paths::dds_for), with the game texture's properties
    /// (its USize / VSize still lay out the UVs) and mips sized like the picture. `gpu`: DXT1 / DXT5 stay
    /// compressed when the device takes them; everything else decodes to RGBA8.
    pub fn load_dds(&mut self, path: &str, force_mask: bool, gpu: bool) -> Option<(Texture, TextureImage)> {
        if !self.hd_scope {
            return None;
        }
        let file = unreal_pkg::paths::dds_for(path)?;
        let mut tex = self.read_raw(path, false)?;
        let bytes = std::fs::read(&file).ok()?;
        let d = dds::parse(&bytes)?;
        if force_mask {
            tex.masked = true;
        }
        let (bc1, bc3, mips) = match d.format {
            dds::Format::Bc1 if gpu && bc1_supported() => (true, false, d.mips.clone()),
            dds::Format::Bc3 if gpu && bc1_supported() => (false, true, d.mips.clone()),
            _ => {
                let mut m = dds::decode_all(&d);
                // Masked textures see through where alpha is low; opaque ones ignore alpha.
                if !tex.masked {
                    for lvl in &mut m {
                        for px in lvl.chunks_exact_mut(4) {
                            px[3] = 255;
                        }
                    }
                }
                (false, false, m)
            }
        };
        if mips.is_empty() {
            return None;
        }
        // The picture's sizes (callers take width / height from the texture); data stays with the image.
        let (mut w, mut h) = (d.width.max(1), d.height.max(1));
        tex.mips = mips
            .iter()
            .map(|_| {
                let m = unreal_pkg::objects::Mip { width: w, height: h, data: Vec::new() };
                w = (w / 2).max(1);
                h = (h / 2).max(1);
                m
            })
            .collect();
        self.dds_used += 1;
        let img = TextureImage { name: path.to_string(), width: d.width, height: d.height, mips, bc1, bc3 };
        Some((tex, img))
    }

    fn load_ext(&mut self, path: &str, force_mask: bool) -> Option<(Texture, Vec<Vec<u8>>)> {
        if let Some((t, img)) = self.load_dds(path, force_mask, false) {
            return Some((t, img.mips));
        }
        if let Some(hd) = self.load_hd_tex(path, force_mask) {
            self.hd_used += 1;
            return Some(hd);
        }
        let (pkg_name, rest) = path.split_once('.')?;
        let pkg = self.package(pkg_name)?;
        let e = pkg.find_export_of_class(rest, "Texture")?;
        if !pkg.class_name(e.class_index).contains("Texture") {
            return None;
        }
        let mut tex = objects::read_texture(pkg, e).ok()?;
        if force_mask {
            tex.masked = true;
        }
        let palette = self.palette_of(pkg_name, tex.palette_ref);
        let mut mips = Vec::new();
        for level in 0..tex.mips.len() {
            match tex.mip_to_rgba8(level, palette.as_ref()) {
                Some(m) => mips.push(m),
                None => break,
            }
        }
        if mips.is_empty() {
            return None;
        }
        Some((tex, mips))
    }
}

/// Find the game's `Textures` folder given the game root (the folder that contains `Maps`).
pub fn textures_dir(game_root: &Path) -> PathBuf {
    game_root.join("Textures")
}

/// A climbable face: its plane (unit normal, distance) and outline points.
#[derive(Clone, Debug)]
pub struct LadderFace {
    pub normal: [f64; 3],
    pub dist: f64,
    pub points: Vec<[f64; 3]>,
}

impl LadderFace {
    /// Is a cylinder (centre, radius, half-height) touching this face (within `slack` of its plane, inside its
    /// outline when projected onto it)?
    pub fn touches(&self, c: [f64; 3], radius: f64, half_height: f64, slack: f64) -> bool {
        let n = self.normal;
        let d = c[0] * n[0] + c[1] * n[1] + c[2] * n[2] - self.dist;
        if d.abs() > radius + slack {
            return false;
        }
        // Project the centre onto the plane and test against the polygon edges (convex), allowing the
        // cylinder's size as margin.
        let p = [c[0] - n[0] * d, c[1] - n[1] * d, c[2] - n[2] * d];
        let k = self.points.len();
        for i in 0..k {
            let a = self.points[i];
            let b = self.points[(i + 1) % k];
            let e = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            // Inward edge normal = n x e (or e x n depending on winding): use the polygon centre to orient.
            let mut en = [n[1] * e[2] - n[2] * e[1], n[2] * e[0] - n[0] * e[2], n[0] * e[1] - n[1] * e[0]];
            let cen = self.points.iter().fold([0.0; 3], |acc, q| [acc[0] + q[0] / k as f64, acc[1] + q[1] / k as f64, acc[2] + q[2] / k as f64]);
            let to_c = [cen[0] - a[0], cen[1] - a[1], cen[2] - a[2]];
            if en[0] * to_c[0] + en[1] * to_c[1] + en[2] * to_c[2] < 0.0 {
                en = [-en[0], -en[1], -en[2]];
            }
            let l = (en[0] * en[0] + en[1] * en[1] + en[2] * en[2]).sqrt().max(1e-9);
            let dist = ((p[0] - a[0]) * en[0] + (p[1] - a[1]) * en[1] + (p[2] - a[2]) * en[2]) / l;
            let margin = if en[2].abs() / l > 0.7 { half_height } else { radius };
            if dist < -margin {
                return false;
            }
        }
        true
    }
}

/// A floor polygon: plane, outline (Unreal units) and index into `Level::floor_groups`.
#[derive(Clone, Debug)]
pub struct FloorFace {
    pub normal: [f64; 3],
    pub dist: f64,
    pub points: Vec<[f64; 3]>,
    pub group: u16,
    pub min: [f64; 2],
    pub max: [f64; 2],
}

/// A wall or ceiling polygon's plane, bounds and texture group.
#[derive(Clone, Debug)]
pub struct WallFace {
    pub normal: [f64; 3],
    pub dist: f64,
    pub group: u16,
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Level {
    /// DeusExWeapon.GetWallMaterial: the texture group of the surface hit at `pos` with normal `normal`.
    pub fn wall_group(&self, pos: [f64; 3], normal: [f64; 3]) -> Option<&str> {
        if normal[2] > 0.5 {
            return self.floor_group([pos[0], pos[1], pos[2] + 1.0], 4.0);
        }
        let mut best: Option<(f64, u16)> = None;
        for w in &self.walls {
            let n = w.normal;
            if n[0] * normal[0] + n[1] * normal[1] + n[2] * normal[2] < 0.9 {
                continue;
            }
            if (0..3).any(|a| pos[a] < w.min[a] - 2.0 || pos[a] > w.max[a] + 2.0) {
                continue;
            }
            let off = (n[0] * pos[0] + n[1] * pos[1] + n[2] * pos[2] - w.dist).abs();
            if off < 3.0 && best.map_or(true, |(b, _)| off < b) {
                best = Some((off, w.group));
            }
        }
        best.and_then(|(_, g)| self.floor_groups.get(g as usize)).map(|s| s.as_str())
    }

    /// DeusExPlayer.GetFloorMaterial: the texture group of the floor under `pos`, searching `down` units below.
    pub fn floor_group(&self, pos: [f64; 3], down: f64) -> Option<&str> {
        let mut best: Option<(f64, u16)> = None;
        for f in &self.floors {
            if pos[0] < f.min[0] || pos[0] > f.max[0] || pos[1] < f.min[1] || pos[1] > f.max[1] {
                continue;
            }
            let n = f.normal;
            // Height of the plane at (x, y).
            let z = (f.dist - n[0] * pos[0] - n[1] * pos[1]) / n[2];
            let below = pos[2] - z;
            if below < -2.0 || below > down {
                continue;
            }
            if !point_in_xy(&f.points, pos[0], pos[1]) {
                continue;
            }
            if best.map_or(true, |(b, _)| below < b) {
                best = Some((below, f.group));
            }
        }
        best.and_then(|(_, g)| self.floor_groups.get(g as usize)).map(|s| s.as_str())
    }
}

fn point_in_xy(pts: &[[f64; 3]], x: f64, y: f64) -> bool {
    let mut inside = false;
    let k = pts.len();
    let mut j = k - 1;
    for i in 0..k {
        let (a, b) = (pts[i], pts[j]);
        if (a[1] > y) != (b[1] > y) && x < (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
        j = i;
    }
    inside
}

#[derive(Clone, Debug)]
pub struct Corona {
    pub pos: [f32; 3],
    pub texture: String,
    /// hsb(LightHue, LightSaturation, full brightness).
    pub color: [f32; 3],
    pub draw_scale: f32,
}

fn find_coronas(pkg: &Package, classes: &mut unreal_pkg::classes::ClassDb) -> Vec<Corona> {
    let mut out = Vec::new();
    for e in &pkg.exports {
        let c = pkg.class_name(e.class_index);
        if !c.to_ascii_lowercase().contains("light") {
            continue;
        }
        let Some(ps) = classes.actor_properties(pkg, e) else { continue };
        if !matches!(props::find(&ps, "bCorona"), Some(Value::Bool(true))) {
            continue;
        }
        let Some(Value::Vector(pos)) = props::find(&ps, "Location") else { continue };
        let texture = match props::find(&ps, "Skin") {
            Some(Value::Path(p)) => p.clone(),
            Some(Value::Object(i)) if *i != 0 => pkg.object_path(*i),
            _ => continue,
        };
        let byte = |n: &str, d: u8| match props::find(&ps, n) {
            Some(Value::Byte(b)) => *b,
            _ => d,
        };
        let color = lighting::hsv_white(byte("LightHue", 0), byte("LightSaturation", 255));
        let draw_scale = match props::find(&ps, "DrawScale") {
            Some(Value::Float(f)) => *f,
            _ => 1.0,
        };
        out.push(Corona { pos: *pos, texture, color, draw_scale });
    }
    out
}

/// Load a level by map file name (for example `01_NYC_UNATCOIsland.dx`) from a game root.
pub fn load_level(game_root: &Path, map_file: &str) -> Result<Level, String> {
    // Map names in exits and scripts don't always match the file's case (05_NYC_UNATCOMJ12Lab vs ...lab.dx); on a
    // case-sensitive file system look the file up (a mod overlay's copy first).
    let map_path = unreal_pkg::paths::find_ci(&game_root.join("Maps"), map_file).unwrap_or_else(|| game_root.join("Maps").join(map_file));
    let pkg = Package::open(&map_path).map_err(|e| format!("cannot open {}: {e}", map_path.display()))?;
    let mut classes = unreal_pkg::classes::ClassDb::open(&game_root.join("System"));
    build_level(&pkg, map_file, &textures_dir(game_root), &mut classes)
}

pub fn build_level(
    pkg: &Package,
    name: &str,
    tex_dir: &Path,
    classes: &mut unreal_pkg::classes::ClassDb,
) -> Result<Level, String> {
    // The level's own BSP is the Model with the most nodes.
    let mut best: Option<objects::Model> = None;
    for e in &pkg.exports {
        if pkg.class_name(e.class_index) != "Model" {
            continue;
        }
        if let Ok(m) = objects::read_model(pkg, e) {
            if best.as_ref().map(|b| m.nodes.len() > b.nodes.len()).unwrap_or(true) {
                best = Some(m);
            }
        }
    }
    let model = best.ok_or_else(|| "map contains no BSP model".to_string())?;

    let sky_loc = find_sky_location(pkg);
    // The BSP zones whose ZoneInfo is a SkyZoneInfo: faces seen from inside them are the sky room.
    let sky_zones: Vec<usize> = model
        .zone_actors
        .iter()
        .enumerate()
        .filter(|(_, &za)| za > 0 && pkg.exports.get(za as usize - 1).is_some_and(|e| pkg.class_name(e.class_index) == "SkyZoneInfo"))
        .map(|(i, _)| i)
        .collect();
    // Per-zone texture pan speeds (zone 0 / no actor: ZoneInfo defaults of 1.0).
    let zone_pan: Vec<[f32; 2]> = model
        .zone_actors
        .iter()
        .map(|&za| {
            if za <= 0 {
                return [1.0, 1.0];
            }
            let Some(e) = pkg.exports.get(za as usize - 1) else { return [1.0, 1.0] };
            let ps = classes.actor_properties(pkg, e).unwrap_or_default();
            let g = |n: &str| match props::find(&ps, n) {
                Some(Value::Float(v)) => *v,
                _ => 1.0,
            };
            [g("TexUPanSpeed"), g("TexVPanSpeed")]
        })
        .collect();
    let mut zone_sky: Vec<bool> = vec![false; model.zone_actors.len().max(1)];
    let zone_water: Vec<Option<f32>> = model
        .zone_actors
        .iter()
        .map(|&za| {
            if za <= 0 {
                return None;
            }
            let e = pkg.exports.get(za as usize - 1)?;
            let ps = classes.actor_properties(pkg, e).unwrap_or_default();
            let water = pkg.class_name(e.class_index) == "WaterZone" || matches!(props::find(&ps, "bWaterZone"), Some(Value::Bool(true)));
            if !water {
                return None;
            }
            Some(match props::find(&ps, "ZoneFluidFriction") {
                Some(Value::Float(v)) => *v,
                _ => 1.2,
            })
        })
        .collect();
    let baked = lighting::bake(pkg, &model, classes);
    let mut stats = LevelStats { nodes: model.nodes.len(), ..Default::default() };
    let sys_dir = tex_dir.parent().map(|p| p.join("System")).unwrap_or_default();
    let mut lib = TexLib { dir: tex_dir.to_path_buf(), sys_dir, pkgs: HashMap::new(), hd_pkgs: HashMap::new(), hd_scope: false, hd_used: 0, dds_used: 0, info: HashMap::new() };
    let mut textures: Vec<TextureImage> = Vec::new();
    // material object ref -> (texture index, masked)
    let mut tex_cache: HashMap<i32, Option<(usize, bool)>> = HashMap::new();
    // The game's texel size of each level texture (UVs use it; an HD texture has more pixels).
    let mut logical: HashMap<usize, (f32, f32)> = HashMap::new();
    lib.hd_scope = true;
    let mut batch_index: HashMap<(Option<usize>, SurfaceClass, bool, [i32; 2], Option<[i32; 4]>, bool), usize> = HashMap::new();
    let mut batches: Vec<Batch> = Vec::new();
    let mut bmin = [f32::MAX; 3];
    let mut bmax = [f32::MIN; 3];

    let mut ladders: Vec<LadderFace> = Vec::new();
    let mut ladder_mat: HashMap<i32, bool> = HashMap::new();
    let mut floors: Vec<FloorFace> = Vec::new();
    let mut walls: Vec<WallFace> = Vec::new();
    let mut floor_groups: Vec<String> = Vec::new();
    let mut floor_mat: HashMap<i32, u16> = HashMap::new();
    for node in &model.nodes {
        if node.num_vertices < 3 {
            continue;
        }
        let Some(surf) = model.surfaces.get(node.surf as usize) else { continue };
        // Ladders: any face whose texture sits in a group called "Ladder" (CoreTexMetal.Ladder.ladder_a ...).
        let is_ladder = *ladder_mat.entry(surf.material).or_insert_with(|| {
            surf.material != 0 && pkg.object_path(surf.material).split('.').rev().nth(1).map_or(false, |g| g.eq_ignore_ascii_case("Ladder"))
        });
        if is_ladder {
            let pts: Vec<[f64; 3]> = (0..node.num_vertices as usize)
                .filter_map(|k| model.vertices.get(node.vert_pool as usize + k).and_then(|v| model.points.get(v.vertex as usize)))
                .map(|q| [q[0] as f64, q[1] as f64, q[2] as f64])
                .collect();
            let n = [node.plane[0] as f64, node.plane[1] as f64, node.plane[2] as f64];
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-9);
            if pts.len() >= 3 && (n[2] / l).abs() < 0.7 {
                let n = [n[0] / l, n[1] / l, n[2] / l];
                let dist = pts[0][0] * n[0] + pts[0][1] * n[1] + pts[0][2] * n[2];
                ladders.push(LadderFace { normal: n, dist, points: pts });
            }
        }
        // Floors (for footstep sounds): solid faces pointing up, tagged with their texture's group.
        {
            let n = [node.plane[0] as f64, node.plane[1] as f64, node.plane[2] as f64];
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-9);
            if n[2] / l <= 0.5 && surf.poly_flags & (pf::NOT_SOLID | pf::PORTAL) == 0 && surf.material != 0 {
                let pts: Vec<[f64; 3]> = (0..node.num_vertices as usize)
                    .filter_map(|k| model.vertices.get(node.vert_pool as usize + k).and_then(|v| model.points.get(v.vertex as usize)))
                    .map(|q| [q[0] as f64, q[1] as f64, q[2] as f64])
                    .collect();
                if pts.len() >= 3 {
                    let group = *floor_mat.entry(surf.material).or_insert_with(|| {
                        let g = pkg.object_path(surf.material).split('.').rev().nth(1).unwrap_or("").to_string();
                        match floor_groups.iter().position(|x| *x == g) {
                            Some(i) => i as u16,
                            None => {
                                floor_groups.push(g);
                                (floor_groups.len() - 1) as u16
                            }
                        }
                    });
                    let nn = [n[0] / l, n[1] / l, n[2] / l];
                    let dist = pts[0][0] * nn[0] + pts[0][1] * nn[1] + pts[0][2] * nn[2];
                    let mut min = [f64::MAX; 3];
                    let mut max = [f64::MIN; 3];
                    for p in &pts {
                        for a in 0..3 {
                            min[a] = min[a].min(p[a]);
                            max[a] = max[a].max(p[a]);
                        }
                    }
                    walls.push(WallFace { normal: nn, dist, group, min, max });
                }
            }
            if n[2] / l > 0.5 && surf.poly_flags & (pf::NOT_SOLID | pf::PORTAL) == 0 {
                let pts: Vec<[f64; 3]> = (0..node.num_vertices as usize)
                    .filter_map(|k| model.vertices.get(node.vert_pool as usize + k).and_then(|v| model.points.get(v.vertex as usize)))
                    .map(|q| [q[0] as f64, q[1] as f64, q[2] as f64])
                    .collect();
                if pts.len() >= 3 {
                    let group = *floor_mat.entry(surf.material).or_insert_with(|| {
                        let g = if surf.material == 0 { String::new() } else { pkg.object_path(surf.material).split('.').rev().nth(1).unwrap_or("").to_string() };
                        match floor_groups.iter().position(|x| *x == g) {
                            Some(i) => i as u16,
                            None => {
                                floor_groups.push(g);
                                (floor_groups.len() - 1) as u16
                            }
                        }
                    });
                    let nn = [n[0] / l, n[1] / l, n[2] / l];
                    let dist = pts[0][0] * nn[0] + pts[0][1] * nn[1] + pts[0][2] * nn[2];
                    let min = [pts.iter().map(|p| p[0]).fold(f64::MAX, f64::min), pts.iter().map(|p| p[1]).fold(f64::MAX, f64::min)];
                    let max = [pts.iter().map(|p| p[0]).fold(f64::MIN, f64::max), pts.iter().map(|p| p[1]).fold(f64::MIN, f64::max)];
                    floors.push(FloorFace { normal: nn, dist, points: pts, group, min, max });
                }
            }
        }
        // Zone portals are normally invisible, but water surfaces sit on zone borders and carry the
        // portal bit as well as the translucent bit, so keep portal faces that are translucent.
        // Some maps' water sheets are portals without PF_Translucent (Battery Park): drawn solid. A portal face
        // with a WetTexture / water material is water either way.
        let water_mat = surf.poly_flags & pf::PORTAL != 0 && surf.material != 0 && pkg.object_path(surf.material).to_ascii_lowercase().contains("water");
        let water_portal = surf.poly_flags & pf::PORTAL != 0 && (surf.poly_flags & pf::TRANSLUCENT != 0 || water_mat);
        if surf.poly_flags & pf::INVISIBLE != 0 || (surf.poly_flags & pf::PORTAL != 0 && !water_portal) {
            stats.skipped_invisible += 1;
            continue;
        }
        if surf.poly_flags & pf::FAKE_BACKDROP != 0 {
            for z in node.zone {
                if z > 0 && !sky_zones.contains(&(z as usize)) {
                    if let Some(f) = zone_sky.get_mut(z as usize) {
                        *f = true;
                    }
                }
            }
        }
        if surf.poly_flags & pf::FAKE_BACKDROP != 0 && std::env::var("DXL_SHOW_BACKDROP").is_err() {
            stats.skipped_backdrop += 1;
            continue;
        }
        // Faces in the sky zone belong to the sky room (by the node's zones when the map has them; by distance
        // from the SkyZoneInfo otherwise - that guess put walls of compact maps like the training course in
        // the sky room).
        let is_sky = if !sky_zones.is_empty() {
            let z = node.zone;
            let zi = |v: i32| (v > 0).then_some(v as usize);
            match (zi(z[1]), zi(z[0])) {
                (Some(f), _) => sky_zones.contains(&f),
                (None, Some(b)) => sky_zones.contains(&b),
                _ => false,
            }
        } else { match sky_loc {
            Some(c) => {
                let mut cen = [0f32; 3];
                let nv = node.num_vertices as usize;
                for k in 0..nv {
                    if let Some(q) = model.vertices.get(node.vert_pool as usize + k).and_then(|v| model.points.get(v.vertex as usize)) {
                        for a in 0..3 {
                            cen[a] += q[a] / nv as f32;
                        }
                    }
                }
                let d = ((cen[0] - c[0]).powi(2) + (cen[1] - c[1]).powi(2) + (cen[2] - c[2]).powi(2)).sqrt();
                d < 5000.0
            }
            None => false,
        } };

        let cached = tex_cache
            .entry(surf.material)
            .or_insert_with(|| {
                let path = pkg.object_path(surf.material);
                let Some((t, mips)) = lib.load_gpu(&path, false) else {
                    // Procedural textures (water, fire, wet) have no pixel data in the package: the original
                    // engine animates them at runtime. A WetTexture (the water surfaces) is its SourceTexture
                    // rippled; the viewer's water shader does the rippling, so take the source picture.
                    if let Some(img) = lib.load_wet(&path) {
                        let idx = textures.len();
                        textures.push(img);
                        return Some((idx, false));
                    }
                    if path.to_ascii_lowercase().contains("water") {
                        let idx = textures.len();
                        textures.push(synth_water(&path));
                        return Some((idx, false));
                    }
                    return None;
                };
                let idx = textures.len();
                let (lw, lh) = TexLib::logical_size(&t);
                logical.insert(idx, (lw as f32, lh as f32));
                textures.push(TextureImage { name: path.clone(), width: t.width(), height: t.height(), mips: mips.mips, bc1: mips.bc1, bc3: mips.bc3 });
                Some((idx, t.masked))
            })
            .clone();
        let (tex_idx, tex_masked) = match cached {
            Some((i, m)) => (Some(i), m),
            None => {
                stats.missing_textures += 1;
                (None, false)
            }
        };
        let (tw, th) = tex_idx
            .map(|i| logical.get(&i).copied().unwrap_or((textures[i].width as f32, textures[i].height as f32)))
            .unwrap_or((256.0, 256.0));

        let is_water = tex_idx.map(|i| textures[i].name.to_ascii_lowercase().contains("water")).unwrap_or(false);
        let water_opaque = is_water && surf.poly_flags & pf::PORTAL != 0 && surf.poly_flags & pf::TRANSLUCENT == 0;
        let class = if is_water && (surf.poly_flags & pf::TRANSLUCENT != 0 || water_opaque) {
            SurfaceClass::Water
        } else if surf.poly_flags & (pf::TRANSLUCENT | pf::MODULATED) != 0 {
            SurfaceClass::Translucent
        } else if surf.poly_flags & pf::MASKED != 0 || tex_masked {
            SurfaceClass::Masked
        } else {
            SurfaceClass::Opaque
        };

        // Auto-panning (e.g. drifting clouds): Unreal pans 64 texels per second times the zone's speed.
        let zone_speed = zone_pan.get(node.zone[1].max(0) as usize).copied().unwrap_or([1.0, 1.0]);
        let uv_scroll = [
            if surf.poly_flags & 0x200 != 0 { -64.0 * zone_speed[0] / tw } else { 0.0 },
            if surf.poly_flags & 0x400 != 0 { -64.0 * zone_speed[1] / th } else { 0.0 },
        ];
        let scroll_key = [(uv_scroll[0] * 10000.0) as i32, (uv_scroll[1] * 10000.0) as i32];
        let mirror = (surf.poly_flags & 0x0800_0000 != 0 && !is_sky).then(|| {
            let v = [node.plane[0], node.plane[2], node.plane[1]];
            let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
            [v[0] / l, v[1] / l, v[2] / l, node.plane[3] / l * METERS_PER_UNIT]
        });
        let mirror_key = mirror.map(|m| [(m[0] * 1000.0) as i32, (m[1] * 1000.0) as i32, (m[2] * 1000.0) as i32, (m[3] * 100.0) as i32]);
        let bi = *batch_index.entry((tex_idx, class, is_sky, scroll_key, mirror_key, water_opaque)).or_insert_with(|| {
            batches.push(Batch {
                texture: tex_idx,
                class,
                sky: is_sky,
                uv_scroll,
                mirror,
                water_opaque,
                positions: Vec::new(),
                normals: Vec::new(),
                uvs: Vec::new(),
                uvs1: Vec::new(),
                colors: Vec::new(),
                indices: Vec::new(),
            });
            batches.len() - 1
        });
        let b = &mut batches[bi];

        let (Some(base), Some(tu), Some(tv)) = (
            model.points.get(surf.p_base as usize).copied(),
            model.vectors.get(surf.v_texture_u as usize).copied(),
            model.vectors.get(surf.v_texture_v as usize).copied(),
        ) else {
            continue;
        };
        let n = {
            let v = ue_to_bevy([node.plane[0], node.plane[1], node.plane[2]]);
            // ue_to_bevy scales by meters; renormalize to a unit vector.
            let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
            [v[0] / l, v[1] / l, v[2] / l]
        };

        let first = b.positions.len() as u32;
        let mut ok = true;
        for k in 0..node.num_vertices as usize {
            let Some(v) = model.vertices.get(node.vert_pool as usize + k) else { ok = false; break };
            let Some(p) = model.points.get(v.vertex as usize).copied() else { ok = false; break };
            let d = [p[0] - base[0], p[1] - base[1], p[2] - base[2]];
            let u = (d[0] * tu[0] + d[1] * tu[1] + d[2] * tu[2] + surf.pan_u as f32) / tw;
            let vv = (d[0] * tv[0] + d[1] * tv[1] + d[2] * tv[2] + surf.pan_v as f32) / th;
            let bp = ue_to_bevy(p);
            if !is_sky {
                for a in 0..3 {
                    bmin[a] = bmin[a].min(bp[a]);
                    bmax[a] = bmax[a].max(bp[a]);
                }
            }
            b.positions.push(bp);
            b.normals.push(n);
            b.uvs.push([u, vv]);
            b.uvs1.push(baked.uv1(&model, node.surf as usize, p));
        }
        if !ok {
            b.positions.truncate(first as usize);
            b.normals.truncate(first as usize);
            b.uvs.truncate(first as usize);
            b.uvs1.truncate(first as usize);
            continue;
        }
        // Bevy front faces are counter-clockwise around the normal; Unreal's polys mostly run the other way.
        let flip = {
            // Newell's normal over the whole polygon (BSP splits leave collinear vertices).
            let cnt = node.num_vertices as u32;
            let mut w = [0.0f32; 3];
            for k in 0..cnt {
                let a = b.positions[(first + k) as usize];
                let c = b.positions[(first + (k + 1) % cnt) as usize];
                w[0] += (a[1] - c[1]) * (a[2] + c[2]);
                w[1] += (a[2] - c[2]) * (a[0] + c[0]);
                w[2] += (a[0] - c[0]) * (a[1] + c[1]);
            }
            w[0] * n[0] + w[1] * n[1] + w[2] * n[2] < 0.0
        };
        for k in 1..node.num_vertices as u32 - 1 {
            b.indices.extend_from_slice(&if flip { [first, first + k + 1, first + k] } else { [first, first + k, first + k + 1] });
            stats.triangles += 1;
        }
    }
    let lights = lighting::collect_lights(pkg, &model, classes);
    let (mesh_actors, actor_list) = actors::build(pkg, classes, &mut lib, &mut textures);
    let nav = nav::NavGraph::build(pkg, classes);
    let movers = {
        let mut resolve = |material: i32| -> Option<(usize, bool, f32, f32)> {
            let entry = tex_cache.entry(material).or_insert_with(|| {
                let path = pkg.object_path(material);
                let (t, mips) = lib.load_gpu(&path, false)?;
                let idx = textures.len();
                let (lw, lh) = TexLib::logical_size(&t);
                logical.insert(idx, (lw as f32, lh as f32));
                textures.push(TextureImage { name: path.clone(), width: t.width(), height: t.height(), mips: mips.mips, bc1: mips.bc1, bc3: mips.bc3 });
                Some((idx, t.masked))
            });
            let (i, m) = (*entry)?;
            let (w, h) = logical.get(&i).copied().unwrap_or((textures[i].width as f32, textures[i].height as f32));
            Some((i, m, w, h))
        };
        movers::build(pkg, &actor_list, &mut resolve)
    };
    stats.textures = textures.len();

    // Actors: find the player start and count placed actors.
    let mut player_start = None;
    for e in &pkg.exports {
        let class = pkg.class_name(e.class_index);
        if e.size == 0 || !matches!(class, "PlayerStart") {
            continue;
        }
        let mut r = Reader::new(pkg.export_data(e), 0);
        if e.flags & props::RF_HAS_STACK != 0 && props::skip_state_frame(&mut r).is_err() {
            continue;
        }
        let Ok(ps) = props::read_properties(pkg, &mut r) else { continue };
        if let Some(Value::Vector(loc)) = props::find(&ps, "Location") {
            let yaw = match props::find(&ps, "Rotation") {
                Some(Value::Rotator(rot)) => rot[1] as f32 / 65536.0 * std::f32::consts::TAU,
                _ => 0.0,
            };
            player_start = Some(PlayerStart { position: ue_to_bevy(*loc), yaw });
            break;
        }
    }
    stats.actors = pkg.exports.len();

    if batches.is_empty() {
        return Err("level produced no geometry".to_string());
    }
    let coronas = find_coronas(pkg, classes);
    let (ambient_sounds, song) = actors::sound_actors(pkg, classes);
    let mut nano_keys = Vec::new();
    for a in actor_list.iter().filter(|a| a.class == "PickupDistributor") {
        for p in &a.props {
            if p.name != "NanoKeyData" {
                continue;
            }
            if let Value::Struct(_, b) = &p.value {
                let mut r = Reader::new(b, 0);
                let name = |r: &mut Reader| -> Option<String> {
                    let i = r.compact_index().ok()?;
                    pkg.names.get(i as usize).cloned()
                };
                if let (Some(tag), Some(key), Ok(desc)) = (name(&mut r), name(&mut r), r.fstring()) {
                    if tag != "None" && !tag.is_empty() {
                        nano_keys.push((tag, key, desc));
                    }
                }
            }
        }
    }
    if lib.hd_used > 0 {
        println!("level: {} HD textures", lib.hd_used);
    }
    Ok(Level {
        nano_keys,
        name: name.to_string(),
        coronas,
        ladders,
        floors,
        walls,
        floor_groups,
        ambient_sounds,
        song,
        textures,
        batches,
        player_start,
        lightmap: Some(baked.into_atlas()),
        collision: collision::BspCollision::new(&model),
        blockers: actors::blockers(&actor_list),
        players_only: actors::players_only(&actor_list),
        monster_blocks: actors::monster_blocks(&actor_list),
        mesh_actors,
        movers,
        nav,
        zone_water,
        zone_sky,
        lights,
        actors: actor_list,
        sky_origin: sky_loc.map(ue_to_bevy),
        bounds_min: bmin,
        bounds_max: bmax,
        stats,
        names: pkg.names.clone(),
    })
}

/// Tileable 256x256 water texture with a full mip chain (sum of periodic sines).
fn synth_water(name: &str) -> TextureImage {
    let n = 256usize;
    let tau = std::f32::consts::TAU;
    let mut base = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let (fx, fy) = (x as f32 / n as f32, y as f32 / n as f32);
            let mut v = 0.0;
            for (i, &(kx, ky, ph)) in [(3.0, 2.0, 0.0), (5.0, -4.0, 1.3), (-7.0, 6.0, 2.1), (11.0, 9.0, 0.7), (2.0, -13.0, 4.0)].iter().enumerate() {
                v += ((fx * kx + fy * ky) * tau + ph).sin() / (1.0 + i as f32 * 0.6);
            }
            let t = (v * 0.18 + 0.5).clamp(0.0, 1.0);
            let o = (y * n + x) * 4;
            base[o] = (20.0 + 45.0 * t) as u8;
            base[o + 1] = (60.0 + 80.0 * t) as u8;
            base[o + 2] = (85.0 + 90.0 * t) as u8;
            base[o + 3] = 255;
        }
    }
    let mut mips = vec![base];
    let mut sz = n;
    while sz > 1 {
        let prev = mips.last().unwrap().clone();
        let ns = sz / 2;
        let mut m = vec![0u8; ns * ns * 4];
        for y in 0..ns {
            for x in 0..ns {
                for c in 0..4 {
                    let g = |xx: usize, yy: usize| prev[(yy * sz + xx) * 4 + c] as u32;
                    m[(y * ns + x) * 4 + c] = ((g(2 * x, 2 * y) + g(2 * x + 1, 2 * y) + g(2 * x, 2 * y + 1) + g(2 * x + 1, 2 * y + 1)) / 4) as u8;
                }
            }
        }
        mips.push(m);
        sz = ns;
    }
    TextureImage { name: name.to_string(), width: n as u32, height: n as u32, mips, bc1: false, bc3: false }
}

/// Location (Unreal units) of the first SkyZoneInfo actor.
fn find_sky_location(pkg: &Package) -> Option<[f32; 3]> {
    for e in &pkg.exports {
        if e.size == 0 || pkg.class_name(e.class_index) != "SkyZoneInfo" {
            continue;
        }
        let mut r = Reader::new(pkg.export_data(e), 0);
        if e.flags & props::RF_HAS_STACK != 0 && props::skip_state_frame(&mut r).is_err() {
            continue;
        }
        if let Ok(ps) = props::read_properties(pkg, &mut r) {
            if let Some(Value::Vector(loc)) = props::find(&ps, "Location") {
                return Some(*loc);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_conversion_swaps_axes() {
        let b = ue_to_bevy([52.5, 105.0, 157.5]);
        assert!((b[0] - 1.0).abs() < 1e-5 && (b[1] - 3.0).abs() < 1e-5 && (b[2] - 2.0).abs() < 1e-5);
    }
}
