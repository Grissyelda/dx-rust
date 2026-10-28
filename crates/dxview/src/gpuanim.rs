//! Vertex animation on the GPU. Unreal 1 meshes animate by keyframed vertex positions; posing them on the CPU
//! and re-uploading every posed mesh each frame cost ~1 ms of game time plus ~1 ms of render time (Bevy
//! re-extracts and re-uploads the whole mesh). Instead every animated part's keyframes go to the GPU once, in
//! two textures (the f16 value and its f16 remainder, so the sum is f32-accurate), and each frame only the
//! part's MeshTag changes: it packs the frame pair and the blend (frame a | frame b << 11 | t * 1023 << 22).
//! Each vertex finds its keyframes through UV_1 = (its texel index in frame 0, vertices per frame).
//!
//! The extension is bindless like StandardMaterial (textures only), so animated parts still batch with
//! everything else; an earlier version with storage buffers made every animated material its own bind group
//! and cost more in draw preparation than it saved.
//!
//! Poses that need more than one frame pair (the 0.1 s TweenTime blend after a sequence change, LipSynch mouth
//! and blink offsets) are posed on the CPU as before for those frames, with the tag set to SENTINEL so the
//! shader uses the mesh's own positions. Meshes with a single frame are never touched.

use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoAutoAabb;
use bevy::mesh::{MeshTag, VertexAttributeValues};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat};
use bevy::shader::ShaderRef;
use std::collections::HashMap;

pub type AnimMaterial = ExtendedMaterial<StandardMaterial, VertAnim>;

pub const ANIM_SHADER: Handle<Shader> = bevy::asset::uuid_handle!("6f0c2f7e-1a8b-4c9d-9e2f-0d5b7a3c1e44");

/// MeshTag meaning "draw the mesh's own positions" (CPU-posed this frame).
pub const SENTINEL: u32 = u32::MAX;
const TEX_W: usize = 4096;
const MAX_FRAME: usize = 2046;

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
#[bindless(index_table(range(101..103), binding(100)))]
pub struct VertAnim {
    #[texture(101)]
    pub hi: Handle<Image>,
    #[texture(102)]
    pub lo: Handle<Image>,
}

impl MaterialExtension for VertAnim {
    fn vertex_shader() -> ShaderRef {
        ANIM_SHADER.into()
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
}

pub fn setup_anim_shader(mut shaders: ResMut<Assets<Shader>>) {
    let _ = shaders.insert(
        &ANIM_SHADER,
        Shader::from_wgsl(include_str!("shaders/vert_anim.wgsl"), "dxview/vert_anim.wgsl"),
    );
}

/// The tag for a single frame pair: frame a | frame b << 11 | round(t * 1023) << 22.
pub fn encode(a: usize, b: usize, t: f32) -> Option<u32> {
    if a > MAX_FRAME || b > MAX_FRAME {
        return None;
    }
    let q = (t.clamp(0.0, 1.0) * 1023.0).round() as u32;
    Some(a as u32 | (b as u32) << 11 | q << 22)
}

/// The frame pair and blend of a pose, the way `AnimMesh::sample` picks them.
pub fn frame_pair(start: usize, count: usize, f: f32) -> (usize, usize, f32) {
    let count = count.max(1);
    let f = f.rem_euclid(count as f32);
    let i0 = f.floor() as usize % count;
    let i1 = (i0 + 1) % count;
    (start + i0, start + i1, f - f.floor())
}

/// On the parent of an animated mesh whose parts can be posed on the GPU.
#[derive(Component)]
pub struct GpuAnimated;

/// Looked at and not eligible (a single-frame mesh): left on the CPU path.
#[derive(Component)]
pub struct GpuAnimSkip;

#[derive(Resource, Default)]
pub struct GpuAnim {
    frames: Vec<[f32; 3]>,
    uploaded: usize,
    /// (anim mesh, vertex ids, draw scale bits) -> (first texel, vertex count, local bounds min, max).
    keyed: HashMap<(usize, u64, u32), (u32, u32, Vec3, Vec3)>,
    images: Option<(Handle<Image>, Handle<Image>)>,
    rows: usize,
    mats: HashMap<AssetId<StandardMaterial>, Handle<AnimMaterial>>,
}

impl GpuAnim {
    /// The part's keyframes in its own vertex order, already in Bevy axes and scale (ue_to_bevy(p * DrawScale),
    /// exactly what the CPU path writes), added once per distinct (mesh, vertex list, draw scale).
    fn keyframes(&mut self, mesh: &std::sync::Arc<dx_level::actors::AnimMesh>, vert_ids: &[u32], draw_scale: f32) -> (u32, u32, Vec3, Vec3) {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        vert_ids.hash(&mut h);
        let key = (std::sync::Arc::as_ptr(mesh) as usize, h.finish(), draw_scale.to_bits());
        if let Some(v) = self.keyed.get(&key) {
            return *v;
        }
        let base = self.frames.len() as u32;
        let (fv, nf) = (mesh.frame_verts, mesh.frames.max(1));
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for fr in 0..nf {
            for &v in vert_ids {
                let u = mesh.verts.get(fr * fv + v as usize).copied().unwrap_or([0.0; 3]);
                let p = dx_level::ue_to_bevy([u[0] * draw_scale, u[1] * draw_scale, u[2] * draw_scale]);
                lo = lo.min(Vec3::from(p));
                hi = hi.max(Vec3::from(p));
                self.frames.push(p);
            }
        }
        if lo.x > hi.x {
            (lo, hi) = (Vec3::ZERO, Vec3::ZERO);
        }
        let v = (base, vert_ids.len() as u32, lo, hi);
        self.keyed.insert(key, v);
        v
    }

    /// (Re)build the two keyframe textures when keyframes were added; true if the images were replaced.
    fn upload(&mut self, images: &mut Assets<Image>) -> bool {
        if self.frames.len() == self.uploaded && self.images.is_some() {
            return false;
        }
        self.uploaded = self.frames.len();
        // Room for meshes spawned later without growing again.
        let rows = (self.frames.len() + self.frames.len() / 4 + TEX_W).div_ceil(TEX_W).max(1);
        let rows = rows.max(self.rows);
        let mut hi = vec![0u8; rows * TEX_W * 8];
        let mut lo = vec![0u8; rows * TEX_W * 8];
        for (i, p) in self.frames.iter().enumerate() {
            for c in 0..3 {
                let h = half::f16::from_f32(p[c]);
                let l = half::f16::from_f32(p[c] - h.to_f32());
                hi[i * 8 + c * 2..i * 8 + c * 2 + 2].copy_from_slice(&h.to_bits().to_le_bytes());
                lo[i * 8 + c * 2..i * 8 + c * 2 + 2].copy_from_slice(&l.to_bits().to_le_bytes());
            }
        }
        let mk = |data: Vec<u8>| {
            Image::new(
                Extent3d { width: TEX_W as u32, height: rows as u32, depth_or_array_layers: 1 },
                TextureDimension::D2,
                data,
                TextureFormat::Rgba16Float,
                RenderAssetUsages::RENDER_WORLD,
            )
        };
        if let (Some((h, l)), true) = (&self.images, rows == self.rows) {
            if let Some(mut img) = images.get_mut(h) {
                img.data = Some(hi);
            }
            if let Some(mut img) = images.get_mut(l) {
                img.data = Some(lo);
            }
            return false;
        }
        self.rows = rows;
        self.images = Some((images.add(mk(hi)), images.add(mk(lo))));
        true
    }
}

/// Move newly spawned animated meshes (more than one frame) onto the GPU path: keyframes added to the
/// textures, UV_1 pointing each vertex at its keyframes, the extended material, and bounds covering every
/// frame (the vertices move on the GPU, so Bevy's own bounds would be those of the rest pose).
pub fn gpuanim_setup(
    mut commands: Commands,
    level: Res<crate::LevelRes>,
    mut ga: ResMut<GpuAnim>,
    new: Query<(Entity, &crate::actors::Animated, Option<&Children>), (Without<GpuAnimated>, Without<GpuAnimSkip>)>,
    kids: Query<(&Mesh3d, &MeshMaterial3d<StandardMaterial>)>,
    std_mats: Res<Assets<StandardMaterial>>,
    mut anim_mats: ResMut<Assets<AnimMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
) {
    let _prof = crate::prof::scope("gpuanim::setup");
    if new.is_empty() {
        return;
    }
    let cpu_only = std::env::var("DXVIEW_CPU_ANIM").is_ok();
    let mut todo = Vec::new();
    for (e, a, children) in &new {
        let ma = &level.0.mesh_actors[a.mesh_actor];
        let anim = ma.anim.as_ref().filter(|an| an.mesh.frames > 1 && !cpu_only);
        let (Some(anim), Some(children)) = (anim, children) else {
            commands.entity(e).try_insert(GpuAnimSkip);
            continue;
        };
        let mut parts = Vec::new();
        for c in children.iter() {
            let Ok((m3, mm)) = kids.get(c) else { continue };
            let Some(&(_, pi)) = a.parts.iter().find(|(h, _)| h.id() == m3.0.id()) else { continue };
            let Some(part) = ma.parts.get(pi) else { continue };
            let (base, nv, lo, hi) = ga.keyframes(&anim.mesh, &part.vert_ids, anim.draw_scale);
            if let Some(mut m) = meshes.get_mut(&m3.0) {
                let uv1: Vec<[f32; 2]> = (0..nv).map(|i| [(base + i) as f32, nv as f32]).collect();
                m.insert_attribute(Mesh::ATTRIBUTE_UV_1, VertexAttributeValues::Float32x2(uv1));
            }
            parts.push((c, mm.0.clone(), lo, hi));
        }
        todo.push((e, parts));
    }
    let replaced = ga.upload(&mut images);
    let (hi_img, lo_img) = ga.images.clone().unwrap();
    if replaced {
        for h in ga.mats.values() {
            if let Some(mut m) = anim_mats.get_mut(h) {
                m.extension = VertAnim { hi: hi_img.clone(), lo: lo_img.clone() };
            }
        }
    }
    for (e, parts) in todo {
        for (c, std_mat, lo, hi) in parts {
            let mat = match ga.mats.get(&std_mat.id()) {
                Some(h) => h.clone(),
                None => {
                    let Some(base_mat) = std_mats.get(&std_mat).cloned() else { continue };
                    let h = anim_mats.add(AnimMaterial {
                        base: base_mat,
                        extension: VertAnim { hi: hi_img.clone(), lo: lo_img.clone() },
                    });
                    ga.mats.insert(std_mat.id(), h.clone());
                    h
                }
            };
            // Bounds of every frame.
            let (a0, a1) = (lo, hi);
            commands.entity(c).try_remove::<MeshMaterial3d<StandardMaterial>>().try_insert((
                MeshMaterial3d(mat),
                MeshTag(SENTINEL),
                Aabb::from_min_max(a0.min(a1), a0.max(a1)),
                NoAutoAabb,
            ));
        }
        commands.entity(e).try_insert(GpuAnimated);
    }
}
