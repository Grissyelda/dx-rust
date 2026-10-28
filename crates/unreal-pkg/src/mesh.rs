//! Vertex-animated meshes (`Mesh` and `LodMesh`), Deus Ex flavor.
//!
//! Every animation frame stores a full copy of the vertex positions; animation sequences are frame
//! ranges. Deus Ex stores vertices as four i16 (x, y, z, pad) instead of Unreal's packed 11/11/10 bits.
//! LodMesh draws `faces` through `wedges` (vertex + 8-bit UV), with `special_verts` reserved slots at the
//! start of every frame. Layout follows SurrealEngine's UMesh/ULodMesh loaders (zlib license).

use crate::objects::open_object;
use crate::{Export, Package, PkgError, Reader, Result};

#[derive(Debug, Clone)]
pub struct AnimSeq {
    pub name: String,
    pub group: String,
    pub start_frame: i32,
    pub num_frames: i32,
    pub rate: f32,
    /// (time 0..1, function name)
    pub notifies: Vec<(f32, String)>,
}

#[derive(Debug, Clone, Copy)]
pub struct Wedge {
    pub vertex: u16,
    pub u: u8,
    pub v: u8,
}

#[derive(Debug, Clone, Copy)]
pub struct Face {
    pub wedges: [u16; 3],
    pub material: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct Material {
    pub poly_flags: u32,
    pub texture_index: i32,
}

#[derive(Debug, Clone, Default)]
pub struct LodMesh {
    pub name: String,
    /// All frames back to back: `frame_verts` vertices per frame (Unreal mesh space).
    pub verts: Vec<[f32; 3]>,
    pub frame_verts: usize,
    pub anim_frames: usize,
    pub anim_seqs: Vec<AnimSeq>,
    /// Object refs of the mesh's own textures (indexed by `Material::texture_index`).
    pub textures: Vec<i32>,
    pub scale: [f32; 3],
    pub origin: [f32; 3],
    /// Pitch, yaw, roll (65536 = full turn).
    pub rot_origin: [i32; 3],
    pub faces: Vec<Face>,
    pub wedges: Vec<Wedge>,
    pub materials: Vec<Material>,
    pub special_faces: Vec<Face>,
    pub special_verts: usize,
    /// Bounding box in mesh space (min, max).
    pub bbox: ([f32; 3], [f32; 3]),
}

fn skip_offset(r: &mut Reader) -> Result<u32> {
    r.u32()
}

fn v3(r: &mut Reader) -> Result<[f32; 3]> {
    Ok([r.f32()?, r.f32()?, r.f32()?])
}

/// Decode a `LodMesh` export (Deus Ex packages).
pub fn read_lod_mesh(pkg: &Package, e: &Export) -> Result<LodMesh> {
    let (mut r, _props) = open_object(pkg, e)?;
    let mut m = LodMesh { name: e.name.clone(), ..Default::default() };
    // UPrimitive bounds.
    let bmin = v3(&mut r)?;
    let bmax = v3(&mut r)?;
    r.u8()?;
    r.take(16)?;
    m.bbox = (bmin, bmax);

    // Vertices (i16 x, y, z, pad).
    // Lazy-array skip offsets are absolute file positions.
    let end = (skip_offset(&mut r)? as usize).wrapping_sub(e.offset);
    let n = r.compact_index()?.max(0) as usize;
    m.verts.reserve(n);
    for _ in 0..n {
        let b = r.take(8)?;
        let g = |i: usize| i16::from_le_bytes([b[i], b[i + 1]]) as f32;
        m.verts.push([g(0), g(2), g(4)]);
    }
    if r.pos() != end {
        return Err(PkgError::Unsupported(format!("mesh {}: vertex array ends at {} not {end}", e.name, r.pos())));
    }
    // Old-style triangles (FMeshTri: 3 vertex indices, 3 byte UVs, PolyFlags, TextureIndex). Empty for LodMesh;
    // a plain Mesh (the DX.dx logos) draws with them, so they become faces through one wedge per corner.
    let _end = skip_offset(&mut r)?;
    let n = r.compact_index()?.max(0) as usize;
    let mut tris = Vec::with_capacity(n);
    for _ in 0..n {
        let vi = [r.u16()?, r.u16()?, r.u16()?];
        let uv = [(r.u8()?, r.u8()?), (r.u8()?, r.u8()?), (r.u8()?, r.u8()?)];
        let flags = r.u32()?;
        let tex = r.i32()?;
        tris.push((vi, uv, flags, tex));
    }
    // Animation sequences.
    let n = r.compact_index()?.max(0) as usize;
    for _ in 0..n {
        let name = pkg.names.get(r.compact_index()? as usize).cloned().unwrap_or_default();
        let group = pkg.names.get(r.compact_index()? as usize).cloned().unwrap_or_default();
        let start_frame = r.i32()?;
        let num_frames = r.i32()?;
        let nn = r.compact_index()?.max(0) as usize;
        let mut notifies = Vec::with_capacity(nn);
        for _ in 0..nn {
            let t = r.f32()?;
            let f = pkg.names.get(r.compact_index()? as usize).cloned().unwrap_or_default();
            notifies.push((t, f));
        }
        let rate = r.f32()?;
        m.anim_seqs.push(AnimSeq { name, group, start_frame, num_frames, rate, notifies });
    }
    // Vertex connects.
    let _end = skip_offset(&mut r)?;
    let n = r.compact_index()?.max(0) as usize;
    r.take(n * 8)?;
    // Bounding box + sphere.
    r.take(25)?;
    r.take(16)?;
    // Vertex links.
    let _end = skip_offset(&mut r)?;
    let n = r.compact_index()?.max(0) as usize;
    r.take(n * 4)?;
    // Textures.
    let n = r.compact_index()?.max(0) as usize;
    for _ in 0..n {
        m.textures.push(r.compact_index()?);
    }
    // Per-frame bounds.
    let n = r.compact_index()?.max(0) as usize;
    r.take(n * 25)?;
    let n = r.compact_index()?.max(0) as usize;
    r.take(n * 16)?;
    m.frame_verts = r.u32()? as usize;
    m.anim_frames = r.u32()? as usize;
    r.u32()?; // and flags
    r.u32()?; // or flags
    m.scale = v3(&mut r)?;
    m.origin = v3(&mut r)?;
    m.rot_origin = [r.i32()?, r.i32()?, r.i32()?];
    r.u32()?; // cur poly
    r.u32()?; // cur vertex
    let n = r.compact_index()?.max(0) as usize;
    r.take(n * 4)?; // texture LOD

    if pkg.class_name(e.class_index) != "LodMesh" {
        for (vi, uv, flags, tex) in tris {
            let mi = match m.materials.iter().position(|x| x.poly_flags == flags && x.texture_index == tex) {
                Some(i) => i,
                None => {
                    m.materials.push(Material { poly_flags: flags, texture_index: tex });
                    m.materials.len() - 1
                }
            };
            let w0 = m.wedges.len() as u16;
            for k in 0..3 {
                m.wedges.push(Wedge { vertex: vi[k], u: uv[k].0, v: uv[k].1 });
            }
            m.faces.push(Face { wedges: [w0, w0 + 1, w0 + 2], material: mi as u16 });
        }
        return Ok(m);
    }
    let n = r.compact_index()?.max(0) as usize;
    r.take(n * 2)?; // collapse point thus
    let n = r.compact_index()?.max(0) as usize;
    r.take(n * 2)?; // face level
    let n = r.compact_index()?.max(0) as usize;
    for _ in 0..n {
        m.faces.push(Face { wedges: [r.u16()?, r.u16()?, r.u16()?], material: r.u16()? });
    }
    let n = r.compact_index()?.max(0) as usize;
    r.take(n * 2)?; // collapse wedge thus
    let n = r.compact_index()?.max(0) as usize;
    for _ in 0..n {
        m.wedges.push(Wedge { vertex: r.u16()?, u: r.u8()?, v: r.u8()? });
    }
    let n = r.compact_index()?.max(0) as usize;
    for _ in 0..n {
        m.materials.push(Material { poly_flags: r.u32()?, texture_index: r.i32()? });
    }
    let n = r.compact_index()?.max(0) as usize;
    for _ in 0..n {
        m.special_faces.push(Face { wedges: [r.u16()?, r.u16()?, r.u16()?], material: r.u16()? });
    }
    let _model_verts = r.u32()?;
    m.special_verts = r.u32()? as usize;
    Ok(m)
}

impl LodMesh {
    /// Vertex `wedge_vertex` of frame `frame` in mesh space.
    pub fn vertex(&self, frame: usize, wedge_vertex: usize) -> [f32; 3] {
        let i = frame.min(self.anim_frames.saturating_sub(1)) * self.frame_verts + self.special_verts + wedge_vertex;
        self.verts.get(i).copied().unwrap_or([0.0; 3])
    }

    pub fn seq(&self, name: &str) -> Option<&AnimSeq> {
        self.anim_seqs.iter().find(|s| s.name.eq_ignore_ascii_case(name))
    }
}
