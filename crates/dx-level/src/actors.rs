//! Placed actors: effective properties for every actor, and baked geometry for mesh actors.
//!
//! Mesh actors are posed at their saved animation frame, transformed exactly like Unreal 1 places a mesh
//! (`Location + PrePivot`, actor rotation, mesh `RotOrigin`, `Scale * DrawScale`, minus mesh `Origin`),
//! and lit per vertex from the level's static lights the way the original lit actors (distance falloff
//! and facing, no shadows). Results are batched by texture in world space for the renderer.

use crate::lighting::{LightInfo, LIGHT_GAIN};
use crate::{SurfaceClass, TexLib, TextureImage};
use std::collections::HashMap;
use unreal_pkg::classes::ClassDb;
use unreal_pkg::mesh::{self, LodMesh};
use unreal_pkg::props::{find, Property, Value};
use unreal_pkg::Package;

/// Classes that are not actors (or are drawn elsewhere) and are skipped outright.
const SKIP: &[&str] = &[
    "Model", "Polys", "Brush", "Level", "LevelSummary", "TextBuffer", "Palette", "Texture",
    "LevelInfo", "ZoneInfo", "SkyZoneInfo", "WaterZone", "PathNode", "PatrolPoint", "Light", "AmbientSound",
    // Editor viewport cameras saved in the map; the game destroys them on load.
    "Camera",
];

#[derive(Debug, Clone)]
pub struct ActorInfo {
    pub name: String,
    pub class: String,
    /// Export index in the map package (1-based object index).
    pub export: i32,
    pub props: Vec<Property>,
}

impl ActorInfo {
    pub fn location(&self) -> [f32; 3] {
        match find(&self.props, "Location") {
            Some(Value::Vector(v)) => *v,
            _ => [0.0; 3],
        }
    }
}

fn f(props: &[Property], name: &str, d: f32) -> f32 {
    match find(props, name) {
        Some(Value::Float(v)) => *v,
        _ => d,
    }
}
fn b(props: &[Property], name: &str) -> u8 {
    match find(props, name) {
        Some(Value::Byte(v)) => *v,
        _ => 0,
    }
}
fn flag(props: &[Property], name: &str) -> bool {
    matches!(find(props, name), Some(Value::Bool(true)))
}
fn path(props: &[Property], name: &str, index: u32) -> Option<String> {
    props.iter().find(|p| p.index == index && p.name.eq_ignore_ascii_case(name)).and_then(|p| match &p.value {
        Value::Path(s) if s != "None" => Some(s.clone()),
        _ => None,
    })
}

/// Unreal rotator (pitch, yaw, roll in 65536ths) to axis vectors (forward X, right Y, up Z), Unreal space.
pub fn rotator_axes(rot: [i32; 3]) -> [[f32; 3]; 3] {
    let k = std::f32::consts::TAU / 65536.0;
    let (sp, cp) = (rot[0] as f32 * k).sin_cos();
    let (sy, cy) = (rot[1] as f32 * k).sin_cos();
    let (sr, cr) = (rot[2] as f32 * k).sin_cos();
    [
        [cp * cy, cp * sy, sp],
        [sr * sp * cy - cr * sy, sr * sp * sy + cr * cy, -sr * cp],
        [-(cr * sp * cy + sr * sy), cy * sr - cr * sp * sy, cr * cp],
    ]
}

fn apply(axes: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    let mut o = [0.0; 3];
    for a in 0..3 {
        o[a] = axes[0][a] * v[0] + axes[1][a] * v[1] + axes[2][a] * v[2];
    }
    o
}

struct MeshCache {
    meshes: HashMap<String, Option<(LodMesh, String)>>, // path -> (mesh, package name)
}

impl MeshCache {
    fn get(&mut self, classes: &ClassDb, full: &str) -> Option<&(LodMesh, String)> {
        if !self.meshes.contains_key(full) {
            let v = (|| {
                let (pk, rest) = full.split_once('.')?;
                let pkg = classes.package(pk)?;
                let e = pkg.find_export_of_class(rest, "Mesh")?;
                Some((mesh::read_lod_mesh(&pkg, e).ok()?, pk.to_string()))
            })();
            self.meshes.insert(full.to_string(), v);
        }
        self.meshes.get(full).and_then(|v| v.as_ref())
    }
}

/// One textured piece of a mesh actor, in actor-local Unreal space (after mesh origin, scale, RotOrigin and
/// DrawScale; before the actor's own rotation and location).
#[derive(Debug, Clone)]
pub struct MeshPart {
    pub texture: Option<usize>,
    pub class: SurfaceClass,
    /// Drawn at full brightness (PF_Unlit material or bUnlit actor).
    pub unlit: bool,
    /// PF_Environment (or the actor's bMeshEnviroMap): the texture is sphere-mapped from the view-space normal
    /// (the shiny metal of the Deus Ex logo).
    pub env: bool,
    /// Drawn from both sides. Mesh sections always are here; a mover's BSP faces only with PF_TwoSided - the
    /// rest are culled from behind like the original's (a door panel whose faces point inward is see-through).
    pub two_sided: bool,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    /// For animated meshes: the mesh vertex each part vertex comes from (index into an `AnimMesh` frame).
    pub vert_ids: Vec<u32>,
    /// The mesh texture slots (MultiSkins indices) drawn by this part.
    pub slots: Vec<u32>,
}

/// All animation frames of one mesh in mesh-local space (origin, scale and RotOrigin applied; DrawScale not).
#[derive(Debug)]
pub struct AnimMesh {
    pub frame_verts: usize,
    pub frames: usize,
    /// `frames * frame_verts` positions.
    pub verts: Vec<[f32; 3]>,
    pub seqs: Vec<unreal_pkg::mesh::AnimSeq>,
    /// The weapon-attachment triangle (the mesh's first special face) for every frame, mesh-local space.
    pub weapon_tri: Vec<[[f32; 3]; 3]>,
}

impl AnimMesh {
    /// Interpolated vertex `v` at fractional frame `f` within [start, start+count).
    pub fn sample(&self, start: usize, count: usize, f: f32, v: usize) -> [f32; 3] {
        let count = count.max(1);
        let f = f.rem_euclid(count as f32);
        let i0 = f.floor() as usize % count;
        let i1 = (i0 + 1) % count;
        let t = f - f.floor();
        let a = self.verts.get((start + i0) * self.frame_verts + v).copied().unwrap_or([0.0; 3]);
        let b = self.verts.get((start + i1) * self.frame_verts + v).copied().unwrap_or(a);
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
    }

    /// Where a held weapon goes at fractional frame `f` of [start, start+count): (origin, x, y, z axes) in
    /// mesh-local space, built from the special triangle the way Unreal's mesh renderer places `Weapon`.
    pub fn weapon_coords(&self, start: usize, count: usize, f: f32) -> Option<[[f32; 3]; 4]> {
        if self.weapon_tri.is_empty() {
            return None;
        }
        let count = count.max(1);
        let f = f.rem_euclid(count as f32);
        let i0 = f.floor() as usize % count;
        let i1 = (i0 + 1) % count;
        let t = f - f.floor();
        let a = self.weapon_tri.get(start + i0)?;
        let b = self.weapon_tri.get(start + i1).unwrap_or(a);
        let v = |k: usize| glam::Vec3::from(a[k]).lerp(glam::Vec3::from(b[k]), t);
        let (v0, v1, v2) = (v(0), v(1), v(2));
        let x = (v1 - v0).normalize_or_zero();
        let y = (v0 - v2).cross(x).normalize_or_zero();
        let z = x.cross(y);
        let o = (v0 + v2) * 0.5;
        Some([o.to_array(), x.to_array(), y.to_array(), z.to_array()])
    }
}

/// A looping animation on a placed actor.
#[derive(Debug, Clone)]
pub struct ActorAnim {
    pub mesh: std::sync::Arc<AnimMesh>,
    pub draw_scale: f32,
    pub seq_start: usize,
    pub seq_frames: usize,
    /// Frames per second.
    pub rate: f32,
    pub start_frame: f32,
}

/// A placed actor drawn with a mesh.
#[derive(Debug, Clone)]
pub struct MeshActor {
    /// Index into `Level::actors`.
    pub actor: usize,
    pub parts: Vec<MeshPart>,
    /// Unreal-space drawing origin (Location + PrePivot).
    pub location: [f32; 3],
    pub rotation: [i32; 3],
    pub scale_glow: f32,
    pub anim: Option<ActorAnim>,
}

/// Property values the game sets on actors of a class before they're built (e.g. PlayerSkin: DeusExPlayer
/// .UpdatePlayerSkin gives Paul, his carcass and JC's double the skin the player chose): (class, property, index,
/// value). They're ClassDb's "travel" override layer, so class defaults read anywhere carry them too.
pub fn set_overrides(v: Vec<(String, String, u32, Value)>) {
    unreal_pkg::classes::set_override_layer("travel", v);
}

/// What a class's BeginPlay / PostBeginPlay changes on its own properties before anything reads them:
/// MilitaryBot / SecurityBot2 SkinColor skins, and the female pain / death cries of Human* pawns with bIsFemale.
pub fn class_begin_play(classes: &mut ClassDb, class: &str, props: &mut Vec<Property>) {
    let set = |props: &mut Vec<Property>, name: &str, index: u32, v: Value| {
        props.retain(|p| !(p.name.eq_ignore_ascii_case(name) && p.index == index));
        props.push(Property { name: name.into(), index, value: v });
    };
    crate::skincolor::apply(classes, class, props);
    if class.eq_ignore_ascii_case("AlarmLight") && !matches!(find(props, "bIsOn"), Some(Value::Bool(true))) {
        // AlarmLight.BeginPlay, off: the lens is masked black and it doesn't turn (the light goes out at run time,
        // projgen.rs, which also keeps the lit lens to put back: AlarmOnSkin).
        if let Some(Value::Path(on)) = find(props, "MultiSkins").and_then(|_| props.iter().find(|p| p.name.eq_ignore_ascii_case("MultiSkins") && p.index == 1).map(|p| &p.value)).cloned() {
            set(props, "AlarmOnSkin", 0, Value::Path(on));
        }
        set(props, "MultiSkins", 1, Value::Path("DeusExItems.Skins.BlackMaskTex".into()));
        set(props, "bFixedRotationDir", 0, Value::Bool(false));
    }
    let skin = match find(props, "SkinColor") {
        Some(Value::Byte(b)) => (*b).min(1),
        _ => 0,
    };
    if class.eq_ignore_ascii_case("MilitaryBot") {
        set(props, "Skin", 0, Value::Path(format!("DeusExCharacters.Skins.MilitaryBotTex{}", skin + 1)));
    } else if class.eq_ignore_ascii_case("SecurityBot2") {
        set(props, "MultiSkins", 1, Value::Path(format!("DeusExCharacters.Skins.SecurityBot2Tex{}", skin + 1)));
    }
    if matches!(find(props, "bIsFemale"), Some(Value::Bool(true))) {
        let chain = classes.ancestry(class);
        if chain.iter().any(|c| matches!(c.as_str(), "humanmilitary" | "humanthug" | "humancivilian")) {
            set(props, "HitSound1", 0, Value::Path("DeusExSounds.Player.FemalePainMedium".into()));
            set(props, "HitSound2", 0, Value::Path("DeusExSounds.Player.FemalePainLarge".into()));
            set(props, "Die", 0, Value::Path("DeusExSounds.Player.FemaleDeath".into()));
        }
    }
}

/// A mod's extra actors for a map: (map name) -> [(class, name, full properties incl. Location)].
pub type ExtraActors = Box<dyn Fn(&str) -> Vec<(String, String, Vec<Property>)> + Send + Sync>;
static EXTRA_ACTORS: std::sync::RwLock<Option<ExtraActors>> = std::sync::RwLock::new(None);

pub fn set_extra_actors(f: Option<ExtraActors>) {
    if let Ok(mut h) = EXTRA_ACTORS.write() {
        *h = f;
    }
}

pub fn build(
    pkg: &Package,
    classes: &mut ClassDb,
    lib: &mut TexLib,
    textures: &mut Vec<TextureImage>,
) -> (Vec<MeshActor>, Vec<ActorInfo>) {
    let mut actors = Vec::new();
    for (i, e) in pkg.exports.iter().enumerate() {
        let class = pkg.class_name(e.class_index).to_string();
        if e.size == 0 || e.class_index == 0 || SKIP.contains(&class.as_str()) {
            continue;
        }
        let Some(mut props) = classes.actor_properties(pkg, e) else { continue };
        // References to imported objects (a ClassProximityType, a FragmentClass, a sound set in the map) read as
        // their path, the same as class defaults do.
        for p in props.iter_mut() {
            if let unreal_pkg::props::Value::Object(o) = p.value {
                if o < 0 {
                    p.value = unreal_pkg::props::Value::Path(pkg.object_path(o));
                }
            }
        }
        if find(&props, "Location").is_none() {
            continue;
        }
        class_begin_play(classes, &class, &mut props);
        actors.push(ActorInfo { name: e.name.clone(), class, export: i as i32 + 1, props });
    }
    // Actors a mod adds to this map (Augmentique's outfit boxes, from GMDX Augmented's copy of the map).
    if let Ok(h) = EXTRA_ACTORS.read() {
        if let Some(f) = h.as_ref() {
            for (class, name, props) in f(pkg.self_name.as_deref().unwrap_or("")) {
                actors.push(ActorInfo { name, class, export: -1, props });
            }
        }
    }

    let mut cache = MeshCache { meshes: HashMap::new() };
    let mut anim_cache: HashMap<String, std::sync::Arc<AnimMesh>> = HashMap::new();
    let mut tex_by_path: HashMap<String, Option<(usize, bool)>> = HashMap::new();
    let mut out = Vec::new();

    for (ai, a) in actors.iter().enumerate() {
        let p = &a.props;
        // DT_Mesh = 2
        // bInWorld=False pawns are built too: hidden and out of collision until EnterWorld (dxview ai::OutOfWorld).
        // bHidden ones too: mission scripts show them later (a keypad, a book, a body); dxview draws them hidden.
        if b(p, "DrawType") != 2 {
            continue;
        }
        let Some(mesh_path) = path(p, "Mesh", 0) else { continue };
        let dbg = std::env::var("DXL_DEBUG").is_ok_and(|c| c == a.class);
        let Some((m, mesh_pkg)) = cache.get(classes, &mesh_path) else {
            if dbg { eprintln!("{}: mesh {mesh_path} not loaded", a.class); }
            continue;
        };
        let (m, mesh_pkg) = (m.clone(), mesh_pkg.clone());
        if m.faces.is_empty() || m.frame_verts == 0 {
            if dbg { eprintln!("{}: faces {} frame_verts {}", a.class, m.faces.len(), m.frame_verts); }
            continue;
        }

        // Saved sequence, else an idle pose (frame 0 of "All" is the modeling reference pose).
        // Classes whose auto state loops an animation from the start (AttackHelicopter/BlackHelicopter 'Fly' spins
        // the rotors, Trash and tarps blow in the wind).
        let auto_loop = match a.class.as_str() {
            "AttackHelicopter" | "BlackHelicopter" => Some("Fly"),
            "Trash" | "HKMarketTarp" => Some("Blowing"),
            "BobPageAugmented" => Some("Idle"),
            _ => None,
        };
        let seq = match (auto_loop.and_then(|n| m.seq(n)), find(p, "AnimSequence")) {
            (Some(s), _) => Some(s),
            (None, Some(Value::Name(n))) if m.seq(n).is_some() => m.seq(n),
            _ => ["BreatheLight", "Still", "Idle1"].iter().find_map(|n| m.seq(n)),
        };
        let frame = seq
            .map(|s| s.start_frame.max(0) as usize + (f(p, "AnimFrame", 0.0).clamp(0.0, 0.999) * s.num_frames.max(1) as f32) as usize)
            .unwrap_or(0);

        let loc = a.location();
        let pre = match find(p, "PrePivot") {
            Some(Value::Vector(v)) => *v,
            _ => [0.0; 3],
        };
        let rot = match find(p, "Rotation") {
            Some(Value::Rotator(r)) => *r,
            _ => [0; 3],
        };
        let draw_scale = f(p, "DrawScale", 1.0);
        let style = b(p, "Style");
        let actor_unlit = flag(p, "bUnlit");

        let mut resolve = |tp: &str| -> Option<(usize, bool)> {
            tex_by_path
                .entry(tp.to_string())
                .or_insert_with(|| {
                    // Decode with palette index 0 see-through: sections drawn PF_Masked (the combat knife's
                    // guard) need it even when the texture isn't flagged bMasked; opaque sections ignore alpha.
                    // Procedural FireTextures (the aug canister's Nano_SFX_A, the bio cell's BioCell_SFX) have no
                    // stored picture: a warmed-up frame stands in, and dxview animates it by name.
                    if lib.load(tp).is_none() {
                        let mut f = lib.load_fire(tp)?;
                        for _ in 0..40 {
                            f.step();
                        }
                        let mut data = Vec::new();
                        f.rgba(&mut data);
                        textures.push(TextureImage { name: tp.to_string(), width: f.width as u32, height: f.height as u32, mips: vec![data] });
                        return Some((textures.len() - 1, false));
                    }
                    let masked = lib.load(tp)?.0.masked;
                    let img = lib.load_image_masked(tp)?;
                    textures.push(TextureImage { name: tp.to_string(), width: img.width, height: img.height, mips: img.mips });
                    Some((textures.len() - 1, masked))
                })
                .clone()
        };
        let parts = mesh_parts(&m, &mesh_pkg, classes, p, style, actor_unlit, draw_scale, frame, &mut resolve);
        if parts.is_empty() {
            if dbg { eprintln!("{}: no parts", a.class); }
            continue;
        }
        // Looping animation for actors whose current sequence has several frames (idle breathing etc.).
        // Pawns always get an animation player (their AI changes sequences); they idle with BreatheLight.
        let is_pawn = find(p, "GroundSpeed").is_some();
        let anim_seq = if is_pawn { m.seq("BreatheLight").or(seq) } else { seq };
        // Decorations with more than one sequence (faucets, switches, toilets...) get one too, to play On/Off/Flush.
        let posable = m.anim_seqs.len() > 1;
        let anim = anim_seq.filter(|s| is_pawn || posable || (s.num_frames > 1 && s.rate > 0.0)).map(|s| {
            let am = anim_cache
                .entry(mesh_path.clone())
                .or_insert_with(|| std::sync::Arc::new(anim_mesh(&m)))
                .clone();
            ActorAnim {
                mesh: am,
                draw_scale,
                seq_start: s.start_frame.max(0) as usize,
                seq_frames: s.num_frames as usize,
                rate: s.rate,
                // Desynchronise identical idles a little, like actors that started at different times.
                start_frame: (ai as f32 * 0.37) % (s.num_frames.max(1) as f32),
            }
        });
        out.push(MeshActor {
            anim,
            actor: ai,
            parts,
            location: [loc[0] + pre[0], loc[1] + pre[1], loc[2] + pre[2]],
            rotation: rot,
            scale_glow: f(p, "ScaleGlow", 1.0),
        });
    }
    (out, actors)
}


/// Build the textured parts of a mesh posed at `frame`, in actor-local Unreal space.
/// `resolve` maps a texture path to (texture index, masked).
#[allow(clippy::too_many_arguments)]
pub fn mesh_parts(
    m: &LodMesh,
    mesh_pkg: &str,
    classes: &ClassDb,
    p: &[Property],
    style: u8,
    actor_unlit: bool,
    draw_scale: f32,
    frame: usize,
    resolve: &mut dyn FnMut(&str) -> Option<(usize, bool)>,
) -> Vec<MeshPart> {
    let mesh_axes = rotator_axes(m.rot_origin);
    let to_local = |v: [f32; 3]| {
        let s = [(v[0] - m.origin[0]) * m.scale[0] * draw_scale, (v[1] - m.origin[1]) * m.scale[1] * draw_scale, (v[2] - m.origin[2]) * m.scale[2] * draw_scale];
        apply(&mesh_axes, s)
    };
        let mut parts: Vec<MeshPart> = Vec::new();
        let mut part_index: HashMap<(Option<usize>, SurfaceClass, bool, bool), usize> = HashMap::new();
        let actor_env = matches!(find(p, "bMeshEnviroMap"), Some(Value::Bool(true)));
        for face in &m.faces {
            let Some(mat) = m.materials.get(face.material as usize) else { continue };
            if mat.poly_flags & 0x1 != 0 {
                continue; // PF_Invisible
            }
            // Texture order as in Unreal: MultiSkins[i]; else for slot 0 the actor Skin; else the mesh's own.
            let ti = mat.texture_index.max(0) as u32;
            let mesh_tex = m.textures.get(ti as usize).filter(|&&r| r != 0).and_then(|&r| {
                let mp = classes.package(mesh_pkg)?;
                Some(if r > 0 { format!("{mesh_pkg}.{}", mp.object_path(r)) } else { mp.object_path(r) })
            });
            let tex_path = path(p, "MultiSkins", ti)
                .or_else(|| if ti == 0 { path(p, "Skin", 0) } else { None })
                .or(mesh_tex)
                .or_else(|| path(p, "Skin", 0));
            let tex = tex_path.and_then(|tp| resolve(&tp));
            let (tidx, masked) = match tex {
                Some((i, m)) => (Some(i), m),
                None => (None, false),
            };
            let class = if style == 4 || mat.poly_flags & 0x40 != 0 {
                SurfaceClass::Modulated
            } else if style == 3 || mat.poly_flags & 0x4 != 0 {
                SurfaceClass::Translucent
            } else if style == 2 || mat.poly_flags & 0x2 != 0 || masked {
                SurfaceClass::Masked
            } else {
                SurfaceClass::Opaque
            };
            let unlit = actor_unlit || mat.poly_flags & 0x0040_0000 != 0 || class == SurfaceClass::Modulated;
            let env = actor_env || mat.poly_flags & 0x10 != 0;
            let pi = *part_index.entry((tidx, class, unlit, env)).or_insert_with(|| {
                parts.push(MeshPart { texture: tidx, class, unlit, env, two_sided: true, positions: Vec::new(), normals: Vec::new(), uvs: Vec::new(), indices: Vec::new(), vert_ids: Vec::new(), slots: Vec::new() });
                parts.len() - 1
            });
            let part = &mut parts[pi];
            if !part.slots.contains(&ti) {
                part.slots.push(ti);
            }

            let w: Vec<_> = face.wedges.iter().map(|&wi| m.wedges.get(wi as usize).copied()).collect();
            if w.iter().any(|x| x.is_none()) {
                continue;
            }
            let pts: Vec<[f32; 3]> = w.iter().map(|x| to_local(m.vertex(frame, x.unwrap().vertex as usize))).collect();
            let e1 = [pts[1][0] - pts[0][0], pts[1][1] - pts[0][1], pts[1][2] - pts[0][2]];
            let e2 = [pts[2][0] - pts[0][0], pts[2][1] - pts[0][1], pts[2][2] - pts[0][2]];
            let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
            let nl = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-9);
            let n = [n[0] / nl, n[1] / nl, n[2] / nl];
            let base = part.positions.len() as u32;
            for (k, pt) in pts.iter().enumerate() {
                let wd = w[k].unwrap();
                part.positions.push(*pt);
                part.normals.push(n);
                part.uvs.push([wd.u as f32 / 255.0, wd.v as f32 / 255.0]);
                part.vert_ids.push(wd.vertex as u32);
            }
            part.indices.extend_from_slice(&[base, base + 1, base + 2]);
        }
    // Smooth vertex normals (Unreal lights meshes per vertex with normals averaged over the faces that share the
    // vertex): sum each mesh vertex's face normals, weighted by face area, across every part.
    let mut sums: HashMap<u32, [f32; 3]> = HashMap::new();
    for part in &parts {
        for tri in part.indices.chunks(3) {
            let [a, b, c] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
            let (pa, pb, pc) = (part.positions[a], part.positions[b], part.positions[c]);
            let e1 = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
            let e2 = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
            // Unnormalised cross product: its length is twice the area.
            let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
            for &v in &[a, b, c] {
                let s = sums.entry(part.vert_ids[v]).or_insert([0.0; 3]);
                for k in 0..3 {
                    s[k] += n[k];
                }
            }
        }
    }
    for part in parts.iter_mut() {
        for (i, id) in part.vert_ids.iter().enumerate() {
            if let Some(s) = sums.get(id) {
                let l = (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt();
                if l > 1e-9 {
                    part.normals[i] = [s[0] / l, s[1] / l, s[2] / l];
                }
            }
        }
    }
    parts
}

/// All frames of a mesh in mesh-local space (for animation playback).
pub fn anim_mesh(m: &LodMesh) -> AnimMesh {
    let mesh_axes = rotator_axes(m.rot_origin);
    let fv = m.frame_verts.saturating_sub(m.special_verts);
    let mut verts = Vec::with_capacity(fv * m.anim_frames);
    for fr in 0..m.anim_frames {
        for v in 0..fv {
            let raw = m.vertex(fr, v);
            let sc = [(raw[0] - m.origin[0]) * m.scale[0], (raw[1] - m.origin[1]) * m.scale[1], (raw[2] - m.origin[2]) * m.scale[2]];
            verts.push(apply(&mesh_axes, sc));
        }
    }
    let mut weapon_tri = Vec::new();
    if let Some(sf) = m.special_faces.first() {
        for fr in 0..m.anim_frames {
            let corner = |k: usize| {
                let raw = m.verts.get(fr * m.frame_verts + sf.wedges[k] as usize).copied().unwrap_or([0.0; 3]);
                let sc = [(raw[0] - m.origin[0]) * m.scale[0], (raw[1] - m.origin[1]) * m.scale[1], (raw[2] - m.origin[2]) * m.scale[2]];
                apply(&mesh_axes, sc)
            };
            weapon_tri.push([corner(0), corner(1), corner(2)]);
        }
    }
    AnimMesh { frame_verts: fv, frames: m.anim_frames, verts, seqs: m.anim_seqs.clone(), weapon_tri }
}

/// Load a mesh by path (e.g. `DeusExItems.Glock`) from the code packages.
pub fn load_mesh(classes: &ClassDb, full: &str) -> Option<(LodMesh, String)> {
    let (pk, rest) = full.split_once('.')?;
    let pkg = classes.package(pk)?;
    let e = pkg.find_export_of_class(rest, "Mesh")?;
    Some((mesh::read_lod_mesh(&pkg, e).ok()?, pk.to_string()))
}

/// Unreal-style actor lighting for one part placed at `location`/`rotation`: per-vertex distance falloff
/// and facing from static lights, no shadows. Returns linear RGBA vertex colors.
pub fn light_part(part: &MeshPart, location: [f32; 3], rotation: [i32; 3], scale_glow: f32, lights: &[LightInfo]) -> Vec<[f32; 4]> {
    if part.unlit {
        return vec![[1.0, 1.0, 1.0, 1.0]; part.positions.len()];
    }
    let axes = rotator_axes(rotation);
    let near: Vec<&LightInfo> = lights
        .iter()
        .filter(|l| {
            let d = ((l.pos[0] - location[0]).powi(2) + (l.pos[1] - location[1]).powi(2) + (l.pos[2] - location[2]).powi(2)).sqrt();
            d < l.radius + 512.0
        })
        .collect();
    part.positions
        .iter()
        .zip(&part.normals)
        .map(|(lp, ln)| {
            let r = apply(&axes, *lp);
            let pt = [location[0] + r[0], location[1] + r[1], location[2] + r[2]];
            let n = apply(&axes, *ln);
            let mut c = [0.0f32; 3];
            for l in &near {
                let to = [l.pos[0] - pt[0], l.pos[1] - pt[1], l.pos[2] - pt[2]];
                let d = (to[0] * to[0] + to[1] * to[1] + to[2] * to[2]).sqrt();
                if d >= l.radius {
                    continue;
                }
                let ndl = ((to[0] * n[0] + to[1] * n[1] + to[2] * n[2]) / d.max(1e-3)).abs();
                let att = (1.0 - d / l.radius) * (0.25 + 0.75 * ndl);
                for ch in 0..3 {
                    c[ch] += l.color[ch] * att;
                }
            }
            let lin = |v: f32| (v * LIGHT_GAIN * scale_glow).clamp(0.0, 1.0).powf(2.2);
            [lin(c[0]), lin(c[1]), lin(c[2]), 1.0]
        })
        .collect()
}

/// The lights that reach an actor, each dimmed by how much of it the light can see (Unreal traces from each light
/// to the actor when lighting it, so actors go dark in shadow): rays to the top, middle and bottom of its
/// collision cylinder against the level.
pub fn shadowed_lights(col: &crate::collision::BspCollision, lights: &[LightInfo], center: [f32; 3], half_height: f32) -> Vec<LightInfo> {
    let c = glam::DVec3::new(center[0] as f64, center[1] as f64, center[2] as f64);
    let h = half_height.max(4.0) as f64;
    let samples = [c + glam::DVec3::new(0.0, 0.0, 0.8 * h), c, c - glam::DVec3::new(0.0, 0.0, 0.6 * h)];
    lights
        .iter()
        .filter_map(|l| {
            let lp = glam::DVec3::new(l.pos[0] as f64, l.pos[1] as f64, l.pos[2] as f64);
            if lp.distance(c) > (l.radius + 512.0) as f64 {
                return None;
            }
            // Start a little out from the light: lamps sit against (or just inside) their fixtures.
            let seen = samples
                .iter()
                .filter(|s| {
                    let d = (**s - lp).normalize_or_zero();
                    col.trace(lp + d * 8.0, **s, glam::DVec3::ZERO).fraction >= 0.995
                })
                .count();
            if seen == 0 {
                return None;
            }
            let k = seen as f32 / samples.len() as f32;
            Some(LightInfo { pos: l.pos, color: [l.color[0] * k, l.color[1] * k, l.color[2] * k], radius: l.radius })
        })
        .collect()
}

/// Unreal rotator to a rotation in Bevy space (Y up), as a column-major 3x3 matrix (x, y, z axes).
pub fn rotator_to_bevy_mat3(rot: [i32; 3]) -> [[f32; 3]; 3] {
    // Bevy axis i corresponds to Unreal axis perm[i]; R_bevy = P * R_ue * P with P swapping Y and Z.
    let ax = rotator_axes(rot); // Unreal images of the Unreal basis vectors
    let ue_basis_image = |ue_axis: usize| -> [f32; 3] { let v = ax[ue_axis]; [v[0], v[2], v[1]] };
    // Bevy X = Unreal X, Bevy Y = Unreal Z, Bevy Z = Unreal Y.
    [ue_basis_image(0), ue_basis_image(2), ue_basis_image(1)]
}

/// Collision cylinders of actors that block the player (`bCollideActors` and `bBlockPlayers`).
/// ScriptedPawns placed with `bInWorld=False` wait outside the world until a mission script calls
/// EnterWorld (e.g. Liberty Island's post-mission troops).
pub fn out_of_world(p: &[Property]) -> bool {
    matches!(find(p, "bInWorld"), Some(Value::Bool(false)))
}

pub fn blockers(actors: &[ActorInfo]) -> Vec<crate::collision::Cylinder> {
    actors
        .iter()
        .enumerate()
        .filter(|(_, a)| flag(&a.props, "bCollideActors") && flag(&a.props, "bBlockPlayers"))
        // Movers collide through their brush, not a cylinder.
        .filter(|(_, a)| find(&a.props, "Brush").is_none())
        .filter_map(|(i, a)| {
            let r = f(&a.props, "CollisionRadius", 0.0) as f64;
            let h = f(&a.props, "CollisionHeight", 0.0) as f64;
            if r <= 0.0 || h <= 0.0 {
                return None;
            }
            let l = a.location();
            // Out of the world: no collision until EnterWorld gives it back (radius from the actor's props).
            // Hidden (bHidden) ones are kept out of the way the same until a script shows them.
            // (Only drawn meshes: BlockPlayer / BlockAll are bHidden editor sprites and always block.)
            let hidden_mesh = flag(&a.props, "bHidden") && matches!(find(&a.props, "DrawType"), Some(Value::Byte(2)));
            let r = if out_of_world(&a.props) || hidden_mesh { 0.0 } else { r };
            Some(crate::collision::Cylinder { center: glam::DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64), radius: r, height: h, actor: i })
        })
        .collect()
}

/// Blocking actors that stop only players (bBlockPlayers, not bBlockActors): BlockPlayer.
pub fn players_only(actors: &[ActorInfo]) -> std::collections::HashSet<usize> {
    actors
        .iter()
        .enumerate()
        .filter(|(_, a)| flag(&a.props, "bCollideActors") && flag(&a.props, "bBlockPlayers") && !flag(&a.props, "bBlockActors"))
        .filter(|(_, a)| find(&a.props, "Brush").is_none() && find(&a.props, "GroundSpeed").is_none())
        .map(|(i, _)| i)
        .collect()
}

/// BlockMonsters and the like: block other actors (pawns) but let the player through.
pub fn monster_blocks(actors: &[ActorInfo]) -> Vec<crate::collision::Cylinder> {
    actors
        .iter()
        .enumerate()
        .filter(|(_, a)| flag(&a.props, "bCollideActors") && flag(&a.props, "bBlockActors") && !flag(&a.props, "bBlockPlayers"))
        .filter(|(_, a)| find(&a.props, "Brush").is_none() && find(&a.props, "GroundSpeed").is_none())
        .filter(|(_, a)| a.class == "BlockMonsters")
        .filter_map(|(i, a)| {
            let r = f(&a.props, "CollisionRadius", 0.0) as f64;
            let h = f(&a.props, "CollisionHeight", 0.0) as f64;
            let l = a.location();
            (r > 0.0 && h > 0.0).then(|| crate::collision::Cylinder { center: glam::DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64), radius: r, height: h, actor: i })
        })
        .collect()
}

/// A placed AmbientSound actor (skipped by [`build`], which only keeps actors that are drawn or touched).
#[derive(Debug, Clone)]
pub struct AmbientSoundInfo {
    /// Unreal units.
    pub pos: [f64; 3],
    /// Sound object path ("Ambient.Ambient.WindLoop" or a sound embedded in the map, "MyLevel...." resolved to the
    /// map's package name).
    pub sound: String,
    pub radius: u8,
    pub volume: u8,
    pub pitch: u8,
}

/// The AmbientSound actors of a map, plus LevelInfo's Song (object path) and SongSection.
pub fn sound_actors(pkg: &Package, classes: &mut ClassDb) -> (Vec<AmbientSoundInfo>, Option<(String, u8)>) {
    let mut out = Vec::new();
    let mut song = None;
    for e in pkg.exports.iter() {
        let class = pkg.class_name(e.class_index);
        if e.size == 0 || !matches!(class, "AmbientSound" | "LevelInfo") {
            continue;
        }
        let Some(props) = classes.actor_properties(pkg, e) else { continue };
        let byte = |n: &str, d: u8| match find(&props, n) {
            Some(Value::Byte(b)) => *b,
            _ => d,
        };
        if class == "LevelInfo" {
            if let Some(Value::Path(s)) = find(&props, "Song") {
                song = Some((s.clone(), byte("SongSection", 0)));
            }
            continue;
        }
        let Some(Value::Path(sound)) = find(&props, "AmbientSound") else { continue };
        let Some(Value::Vector(p)) = find(&props, "Location") else { continue };
        out.push(AmbientSoundInfo {
            pos: [p[0] as f64, p[1] as f64, p[2] as f64],
            sound: sound.clone(),
            radius: byte("SoundRadius", 32),
            volume: byte("SoundVolume", 128),
            pitch: byte("SoundPitch", 64),
        });
    }
    (out, song)
}
