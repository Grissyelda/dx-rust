//! Decoders for the object types needed to render a level: Palette, Texture, Polys and Model (BSP).
//!
//! Layouts follow Unreal Engine 1 package version 68 and were cross-checked against the
//! SurrealEngine loaders and against the real Deus Ex files.

use crate::props::{self, Property, Value, RF_HAS_STACK};
use crate::{Export, Package, PkgError, Reader, Result};

/// Polygon flags (PF_*) used when filtering and shading surfaces.
pub mod pf {
    pub const INVISIBLE: u32 = 0x0000_0001;
    pub const MASKED: u32 = 0x0000_0002;
    pub const TRANSLUCENT: u32 = 0x0000_0004;
    pub const NOT_SOLID: u32 = 0x0000_0008;
    pub const ENVIRONMENT: u32 = 0x0000_0010;
    pub const SEMISOLID: u32 = 0x0000_0020;
    pub const MODULATED: u32 = 0x0000_0040;
    pub const FAKE_BACKDROP: u32 = 0x0000_0080;
    pub const TWO_SIDED: u32 = 0x0000_0100;
    pub const UNLIT: u32 = 0x0040_0000;
    pub const PORTAL: u32 = 0x0400_0000;
}

/// Start a reader on an export's payload positioned at the property list, handling the
/// optional state-frame header, and decode the properties.
pub fn open_object<'a>(pkg: &'a Package, e: &Export) -> Result<(Reader<'a>, Vec<Property>)> {
    let mut r = Reader::new(pkg.export_data(e), 0);
    if e.flags & RF_HAS_STACK != 0 {
        props::skip_state_frame(&mut r)?;
    }
    let ps = props::read_properties(pkg, &mut r)?;
    Ok((r, ps))
}

fn vec3(r: &mut Reader) -> Result<[f32; 3]> {
    Ok([r.f32()?, r.f32()?, r.f32()?])
}

fn byte_prop(ps: &[Property], name: &str, default: u8) -> u8 {
    match props::find(ps, name) {
        Some(Value::Byte(b)) => *b,
        _ => default,
    }
}

fn bool_prop(ps: &[Property], name: &str) -> bool {
    matches!(props::find(ps, name), Some(Value::Bool(true)))
}

fn int_prop(ps: &[Property], name: &str, default: i32) -> i32 {
    match props::find(ps, name) {
        Some(Value::Int(i)) => *i,
        _ => default,
    }
}

// ---------------------------------------------------------------- Palette

#[derive(Debug, Clone)]
pub struct Palette {
    /// RGBA, 8 bits per channel.
    pub colors: Vec<[u8; 4]>,
}

pub fn read_palette(pkg: &Package, e: &Export) -> Result<Palette> {
    let (mut r, _ps) = open_object(pkg, e)?;
    let count = r.compact_index()?.max(0) as usize;
    let bytes = r.take(count * 4)?;
    let colors = bytes.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
    Ok(Palette { colors })
}

// ---------------------------------------------------------------- Texture

#[derive(Debug, Clone)]
pub struct Mip {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Original UE1 texture formats.
pub mod tex_format {
    pub const P8: u8 = 0;
    pub const RGBA7: u8 = 1;
    pub const RGB16: u8 = 2;
    pub const DXT1: u8 = 3;
    pub const RGB8: u8 = 4;
    pub const RGBA8: u8 = 5;
}

#[derive(Debug, Clone)]
pub struct Texture {
    pub name: String,
    pub format: u8,
    /// Object reference to the palette (valid in the texture's own package).
    pub palette_ref: i32,
    pub masked: bool,
    pub props: Vec<Property>,
    pub mips: Vec<Mip>,
    /// Engine class (Texture, FireTexture, WetTexture, ...).
    pub class: String,
    /// FireTexture spark definitions (type, heat, x, y, 4 argument bytes).
    pub sparks: Vec<[u8; 8]>,
}

pub fn read_texture(pkg: &Package, e: &Export) -> Result<Texture> {
    let (mut r, ps) = open_object(pkg, e)?;
    let has_comp = bool_prop(&ps, "bHasComp");

    let read_mips = |r: &mut Reader, width_offset: bool| -> Result<Vec<Mip>> {
        let count = r.u8()? as usize;
        let mut mips = Vec::with_capacity(count);
        for _ in 0..count {
            if width_offset {
                r.i32()?; // file offset of the next mip, unused when reading sequentially
            }
            let n = r.compact_index()?.max(0) as usize;
            let data = r.take(n)?.to_vec();
            let width = r.u32()?;
            let height = r.u32()?;
            r.u8()?; // UBits
            r.u8()?; // VBits
            mips.push(Mip { width, height, data });
        }
        Ok(mips)
    };

    let mut mips = read_mips(&mut r, pkg.version >= 63)?;
    let mut format = byte_prop(&ps, "Format", 0);
    if has_comp {
        let comp = read_mips(&mut r, pkg.version >= 68)?;
        if !comp.is_empty() {
            mips = comp;
            format = byte_prop(&ps, "CompFormat", format);
        }
    }

    let class = pkg.class_name(e.class_index).to_string();
    let mut sparks = Vec::new();
    if class == "FireTexture" {
        let n = r.compact_index()?.max(0) as usize;
        for _ in 0..n {
            let b = r.take(8)?;
            let mut a = [0u8; 8];
            a.copy_from_slice(b);
            sparks.push(a);
        }
    }
    let palette_ref = match props::find(&ps, "Palette") {
        Some(Value::Object(i)) => *i,
        _ => 0,
    };
    Ok(Texture {
        name: e.name.clone(),
        format,
        palette_ref,
        masked: bool_prop(&ps, "bMasked"),
        props: ps,
        mips,
        class,
        sparks,
    })
}

impl Texture {
    pub fn width(&self) -> u32 {
        self.mips.first().map(|m| m.width).unwrap_or(0)
    }
    pub fn height(&self) -> u32 {
        self.mips.first().map(|m| m.height).unwrap_or(0)
    }

    /// Decode mip 0 to RGBA8. `palette` is required for P8 textures.
    pub fn to_rgba8(&self, palette: Option<&Palette>) -> Option<Vec<u8>> {
        self.mip_to_rgba8(0, palette)
    }

    /// Decode one mip level to RGBA8. `palette` is required for P8 textures.
    pub fn mip_to_rgba8(&self, level: usize, palette: Option<&Palette>) -> Option<Vec<u8>> {
        let m = self.mips.get(level)?;
        let (w, h) = (m.width as usize, m.height as usize);
        if w == 0 || h == 0 || w > 4096 || h > 4096 {
            return None;
        }
        let mut out = vec![0u8; w * h * 4];
        match self.format {
            tex_format::P8 => {
                let pal = palette?;
                if m.data.len() < w * h {
                    return None;
                }
                for i in 0..w * h {
                    let idx = m.data[i] as usize;
                    let c = pal.colors.get(idx).copied().unwrap_or([0, 0, 0, 255]);
                    let a = if self.masked && idx == 0 { 0 } else { 255 };
                    out[i * 4..i * 4 + 4].copy_from_slice(&if a == 0 { [0, 0, 0, 0] } else { [c[0], c[1], c[2], a] });
                }
            }
            tex_format::RGBA8 => {
                if m.data.len() < w * h * 4 {
                    return None;
                }
                // Stored BGRA.
                for i in 0..w * h {
                    let s = &m.data[i * 4..i * 4 + 4];
                    out[i * 4..i * 4 + 4].copy_from_slice(&[s[2], s[1], s[0], s[3]]);
                }
            }
            tex_format::DXT1 => {
                decode_dxt1(&m.data, w, h, &mut out)?;
            }
            _ => return None,
        }
        Some(out)
    }
}

fn rgb565(c: u16) -> [u8; 3] {
    let r = ((c >> 11) & 0x1f) as u32;
    let g = ((c >> 5) & 0x3f) as u32;
    let b = (c & 0x1f) as u32;
    [((r * 255 + 15) / 31) as u8, ((g * 255 + 31) / 63) as u8, ((b * 255 + 15) / 31) as u8]
}

fn decode_dxt1(data: &[u8], w: usize, h: usize, out: &mut [u8]) -> Option<()> {
    let bw = (w + 3) / 4;
    let bh = (h + 3) / 4;
    if data.len() < bw * bh * 8 {
        return None;
    }
    for by in 0..bh {
        for bx in 0..bw {
            let b = &data[(by * bw + bx) * 8..(by * bw + bx) * 8 + 8];
            let c0 = u16::from_le_bytes([b[0], b[1]]);
            let c1 = u16::from_le_bytes([b[2], b[3]]);
            let (p0, p1) = (rgb565(c0), rgb565(c1));
            let mut pal = [[0u8; 4]; 4];
            pal[0] = [p0[0], p0[1], p0[2], 255];
            pal[1] = [p1[0], p1[1], p1[2], 255];
            if c0 > c1 {
                for k in 0..3 {
                    pal[2][k] = ((2 * p0[k] as u32 + p1[k] as u32) / 3) as u8;
                    pal[3][k] = ((p0[k] as u32 + 2 * p1[k] as u32) / 3) as u8;
                }
                pal[2][3] = 255;
                pal[3][3] = 255;
            } else {
                for k in 0..3 {
                    pal[2][k] = ((p0[k] as u32 + p1[k] as u32) / 2) as u8;
                }
                pal[2][3] = 255;
                pal[3] = [0, 0, 0, 0];
            }
            let bits = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
            for py in 0..4 {
                for px in 0..4 {
                    let (x, y) = (bx * 4 + px, by * 4 + py);
                    if x < w && y < h {
                        let idx = ((bits >> (2 * (py * 4 + px))) & 3) as usize;
                        out[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&pal[idx]);
                    }
                }
            }
        }
    }
    Some(())
}

// ---------------------------------------------------------------- Model (BSP)

#[derive(Debug, Clone)]
pub struct BspNode {
    pub plane: [f32; 4],
    pub zone_mask: u64,
    pub flags: u8,
    pub vert_pool: i32,
    pub surf: i32,
    pub back: i32,
    pub front: i32,
    pub plane_idx: i32,
    pub zone: [i32; 2],
    pub num_vertices: u8,
    /// Index into `Model::leaf_hulls` of this node's convex collision hull, or -1.
    pub collision_bound: i32,
}

#[derive(Debug, Clone)]
pub struct BspSurface {
    /// Object reference to the surface texture (an import in the map package).
    pub material: i32,
    pub poly_flags: u32,
    pub p_base: i32,
    pub v_normal: i32,
    pub v_texture_u: i32,
    pub v_texture_v: i32,
    pub light_map: i32,
    pub brush_poly: i32,
    pub pan_u: i16,
    pub pan_v: i16,
    pub brush_actor: i32,
}

#[derive(Debug, Clone)]
pub struct BspVert {
    pub vertex: i32,
    pub side: i32,
}

#[derive(Debug, Clone, Default)]
pub struct Model {
    pub name: String,
    pub vectors: Vec<[f32; 3]>,
    pub points: Vec<[f32; 3]>,
    pub nodes: Vec<BspNode>,
    pub surfaces: Vec<BspSurface>,
    pub vertices: Vec<BspVert>,
    pub num_shared_sides: i32,
    pub num_zones: i32,
    pub zone_actors: Vec<i32>,
    pub lightmaps: Vec<LightMapIndex>,
    /// Packed per-light shadow bitmaps (1 bit per lightmap texel).
    pub light_bits: Vec<u8>,
    /// Object refs of static lights; each surface's list starts at `LightMapIndex::light_actors`, ended by 0.
    pub lights: Vec<i32>,
    /// Collision hulls: per hull, node indices (bit 0x40000000 = flipped plane) ending in a negative
    /// value, then 6 floats (bounding box min, max) stored as raw bits.
    pub leaf_hulls: Vec<i32>,
    /// The Polys object this model was built from (the brush's editable polygons), 0 if none.
    pub polys: i32,
}

#[derive(Debug, Clone)]
pub struct LightMapIndex {
    pub data_offset: i32,
    pub pan: [f32; 3],
    pub u_clamp: i32,
    pub v_clamp: i32,
    pub u_scale: f32,
    pub v_scale: f32,
    pub light_actors: i32,
}

pub fn read_model(pkg: &Package, e: &Export) -> Result<Model> {
    let (mut r, _ps) = open_object(pkg, e)?;
    // UPrimitive: bounding box (6 floats + valid byte) and bounding sphere (4 floats).
    for _ in 0..6 {
        r.f32()?;
    }
    r.u8()?;
    for _ in 0..4 {
        r.f32()?;
    }

    let mut m = Model { name: e.name.clone(), ..Default::default() };

    let n = r.compact_index()?.max(0) as usize;
    m.vectors.reserve(n);
    for _ in 0..n {
        m.vectors.push(vec3(&mut r)?);
    }
    let n = r.compact_index()?.max(0) as usize;
    m.points.reserve(n);
    for _ in 0..n {
        m.points.push(vec3(&mut r)?);
    }

    let n = r.compact_index()?.max(0) as usize;
    m.nodes.reserve(n);
    for _ in 0..n {
        let plane = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
        let zone_mask = u64::from_le_bytes(r.take(8)?.try_into().unwrap());
        let flags = r.u8()?;
        let vert_pool = r.compact_index()?;
        let surf = r.compact_index()?;
        let back = r.compact_index()?;
        let front = r.compact_index()?;
        let plane_idx = r.compact_index()?;
        let collision_bound = r.compact_index()?;
        let _render_bound = r.compact_index()?;
        let zone0 = r.compact_index()?;
        let zone1 = r.compact_index()?;
        let num_vertices = r.u8()?;
        let _leaf0 = r.i32()?;
        let _leaf1 = r.i32()?;
        m.nodes.push(BspNode {
            plane,
            zone_mask,
            flags,
            vert_pool,
            surf,
            back,
            front,
            plane_idx,
            zone: [zone0, zone1],
            num_vertices,
            collision_bound,
        });
    }

    let n = r.compact_index()?.max(0) as usize;
    m.surfaces.reserve(n);
    for _ in 0..n {
        m.surfaces.push(BspSurface {
            material: r.compact_index()?,
            poly_flags: r.u32()?,
            p_base: r.compact_index()?,
            v_normal: r.compact_index()?,
            v_texture_u: r.compact_index()?,
            v_texture_v: r.compact_index()?,
            light_map: r.compact_index()?,
            brush_poly: r.compact_index()?,
            pan_u: r.u16()? as i16,
            pan_v: r.u16()? as i16,
            brush_actor: r.compact_index()?,
        });
    }

    let n = r.compact_index()?.max(0) as usize;
    m.vertices.reserve(n);
    for _ in 0..n {
        m.vertices.push(BspVert { vertex: r.compact_index()?, side: r.compact_index()? });
    }
    m.num_shared_sides = r.i32()?;
    m.num_zones = r.i32()?;
    // The tail (zones, lightmaps) is optional for geometry: ignore errors so brush models still load.
    let _ = read_model_tail(&mut r, &mut m);
    Ok(m)
}

fn read_model_tail(r: &mut Reader, m: &mut Model) -> Result<()> {
    // Zone table: actor ref, connectivity, visibility.
    for _ in 0..m.num_zones.max(0) {
        let actor = r.compact_index()?;
        let _conn = r.take(8)?;
        let _vis = r.take(8)?;
        m.zone_actors.push(actor);
    }
    m.polys = r.compact_index()?;
    let n = r.compact_index()?.max(0) as usize;
    for _ in 0..n {
        m.lightmaps.push(LightMapIndex {
            data_offset: r.i32()?,
            pan: [r.f32()?, r.f32()?, r.f32()?],
            u_clamp: r.compact_index()?,
            v_clamp: r.compact_index()?,
            u_scale: r.f32()?,
            v_scale: r.f32()?,
            light_actors: r.i32()?,
        });
    }
    let n = r.compact_index()?.max(0) as usize;
    m.light_bits = r.take(n)?.to_vec();
    let n = r.compact_index()?.max(0) as usize;
    r.take(n * 25)?; // bounds
    let n = r.compact_index()?.max(0) as usize;
    for _ in 0..n {
        m.leaf_hulls.push(r.i32()?);
    }
    let n = r.compact_index()?.max(0) as usize;
    for _ in 0..n {
        r.compact_index()?;
        r.compact_index()?;
        r.compact_index()?;
        r.take(8)?;
    }
    let n = r.compact_index()?.max(0) as usize;
    for _ in 0..n {
        m.lights.push(r.compact_index()?);
    }
    Ok(())
}

// ---------------------------------------------------------------- Polys (brush source polygons)

#[derive(Debug, Clone)]
pub struct Poly {
    pub base: [f32; 3],
    pub normal: [f32; 3],
    pub texture_u: [f32; 3],
    pub texture_v: [f32; 3],
    pub vertices: Vec<[f32; 3]>,
    pub poly_flags: u32,
    pub actor: i32,
    pub texture: i32,
    pub pan_u: i16,
    pub pan_v: i16,
}

pub fn read_polys(pkg: &Package, e: &Export) -> Result<Vec<Poly>> {
    let (mut r, _ps) = open_object(pkg, e)?;
    let count = r.i32()?.max(0) as usize;
    let _max = r.i32()?;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let nv = r.compact_index()?.max(0) as usize;
        let base = vec3(&mut r)?;
        let normal = vec3(&mut r)?;
        let texture_u = vec3(&mut r)?;
        let texture_v = vec3(&mut r)?;
        let mut vertices = Vec::with_capacity(nv);
        for _ in 0..nv {
            vertices.push(vec3(&mut r)?);
        }
        let poly_flags = r.u32()?;
        let actor = r.compact_index()?;
        let texture = r.compact_index()?;
        let _item_name = r.compact_index()?;
        let _link = r.compact_index()?;
        let _brush_poly = r.compact_index()?;
        let pan_u = r.u16()? as i16;
        let pan_v = r.u16()? as i16;
        out.push(Poly { base, normal, texture_u, texture_v, vertices, poly_flags, actor, texture, pan_u, pan_v });
    }
    Ok(out)
}

#[allow(dead_code)]
fn _unused(ps: &[Property]) -> i32 {
    int_prop(ps, "", 0)
}

/// One precomputed path between two navigation points (ULevel.ReachSpecs).
#[derive(Debug, Clone, Copy)]
pub struct ReachSpec {
    pub distance: i32,
    /// Object indices (exports) of the start and end NavigationPoints.
    pub start: i32,
    pub end: i32,
    pub collision_radius: i32,
    pub collision_height: i32,
    pub reach_flags: i32,
    pub pruned: bool,
}

/// Read the level's reach specs (the AI path network built by the editor).
pub fn read_reach_specs(pkg: &Package) -> Result<Vec<ReachSpec>> {
    let e = pkg
        .exports
        .iter()
        .find(|e| pkg.class_name(e.class_index) == "Level" && e.size > 0)
        .ok_or_else(|| PkgError::Unsupported("no Level export".into()))?;
    let (mut r, _props) = open_object(pkg, e)?;
    let count = r.i32()?.max(0) as usize;
    let _max = r.i32()?;
    for _ in 0..count {
        r.compact_index()?;
    }
    for _ in 0..4 {
        r.fstring()?; // protocol, host, map, portal
    }
    let opts = r.compact_index()?.max(0) as usize;
    for _ in 0..opts {
        r.fstring()?;
    }
    r.i32()?; // port
    r.u32()?;
    r.compact_index()?; // model
    let n = r.compact_index()?.max(0) as usize;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(ReachSpec {
            distance: r.i32()?,
            start: r.compact_index()?,
            end: r.compact_index()?,
            collision_radius: r.i32()?,
            collision_height: r.i32()?,
            reach_flags: r.i32()?,
            pruned: r.u8()? != 0,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------- Sound and Music

/// A USound / UMusic payload: the format name ("WAV", "it", "s3m", "xm", "umx"...) and the embedded file bytes.
#[derive(Debug, Clone)]
pub struct SoundData {
    pub format: String,
    pub data: Vec<u8>,
}

/// USound::Serialize (version 68): properties, FName FileType, then a lazy byte array (i32 offset of the end of the
/// array, compact count, bytes). UMusic is the same shape.
pub fn read_sound(pkg: &Package, e: &Export) -> Result<SoundData> {
    let (mut r, _ps) = open_object(pkg, e)?;
    let fmt = r.compact_index()?;
    let format = pkg.names.get(fmt.max(0) as usize).cloned().unwrap_or_default();
    if pkg.version >= 63 {
        let _end = r.i32()?;
    }
    let n = r.compact_index()?.max(0) as usize;
    let data = r.take(n)?.to_vec();
    Ok(SoundData { format, data })
}

// ---------------------------------------------------------------- Font

/// One glyph's rectangle on its page texture.
#[derive(Debug, Clone, Copy, Default)]
pub struct FontChar {
    pub u: i32,
    pub v: i32,
    pub w: i32,
    pub h: i32,
}

/// A bitmap font (v68): pages of {texture ref, glyph rects}, then CharactersPerPage.
#[derive(Debug, Clone)]
pub struct Font {
    /// Per page: texture object reference (in this package) and its glyphs.
    pub pages: Vec<(i32, Vec<FontChar>)>,
    pub chars_per_page: i32,
}

impl Font {
    /// The page and glyph for a character, if the font has it.
    pub fn glyph(&self, c: char) -> Option<(usize, FontChar)> {
        let code = c as u32 as usize;
        let per = self.chars_per_page.max(1) as usize;
        let (page, idx) = (code / per, code % per);
        let g = *self.pages.get(page)?.1.get(idx)?;
        (g.w > 0 && g.h > 0).then_some((page, g))
    }
}

pub fn read_font(pkg: &Package, e: &Export) -> Result<Font> {
    let (mut r, _ps) = open_object(pkg, e)?;
    let n = r.compact_index()?.max(0) as usize;
    let mut pages = Vec::with_capacity(n);
    for _ in 0..n {
        let tex = r.compact_index()?;
        let count = r.compact_index()?.max(0) as usize;
        let mut chars = Vec::with_capacity(count);
        for _ in 0..count {
            chars.push(FontChar { u: r.i32()?, v: r.i32()?, w: r.i32()?, h: r.i32()? });
        }
        pages.push((tex, chars));
    }
    let chars_per_page = r.i32()?;
    Ok(Font { pages, chars_per_page })
}
