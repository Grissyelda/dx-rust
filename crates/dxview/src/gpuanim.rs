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
pub const ANIM_PREPASS_SHADER: Handle<Shader> = bevy::asset::uuid_handle!("6f0c2f7e-1a8b-4c9d-9e2f-0d5b7a3c1e45");

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
    fn prepass_vertex_shader() -> ShaderRef {
        ANIM_PREPASS_SHADER.into()
    }
    fn enable_shadows() -> bool {
        // Shadow maps (shadowlight.rs, the flashlight cone) draw animated meshes in their current pose.
        true
    }
}

pub fn setup_anim_shader(mut shaders: ResMut<Assets<Shader>>) {
    let _ = shaders.insert(
        &ANIM_SHADER,
        Shader::from_wgsl(include_str!("shaders/vert_anim.wgsl"), "dxview/vert_anim.wgsl"),
    );
    let _ = shaders.insert(
        &ANIM_PREPASS_SHADER,
        Shader::from_wgsl(include_str!("shaders/vert_anim_prepass.wgsl"), "dxview/vert_anim_prepass.wgsl"),
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

/// Texels per keyframe page: texel indices travel to the shader as f32 (UV_1), exact only below 2^24.
const PAGE_TEXELS: usize = 1 << 24;

/// One pair of keyframe textures (value + remainder). A page is filled once (the meshes that appear together,
/// a whole level at load) and uploaded once; meshes spawned later go into new small pages, so a late spawn never
/// re-uploads the level's keyframes (that was a multi-second stall with HDTP's big meshes).
#[derive(Default)]
struct Page {
    hi: Vec<u8>,
    lo: Vec<u8>,
    len: usize,
    rows: usize,
    images: Option<(Handle<Image>, Handle<Image>)>,
}

#[derive(Resource, Default)]
pub struct GpuAnim {
    pages: Vec<Page>,
    /// (anim mesh, draw scale bits) -> (page, first texel, vertices per frame, local bounds min, max). One block per
    /// mesh holds every mesh vertex of every frame; each part's vertices index into it by vertex id, so parts,
    /// wedges (UV seams) and actors sharing a mesh share its keyframes.
    keyed: HashMap<(usize, u32), (usize, u32, u32, Vec3, Vec3)>,
    /// (standard material, page) -> its animated twin.
    mats: HashMap<(AssetId<StandardMaterial>, usize), Handle<AnimMaterial>>,
}

impl GpuAnim {
    /// The mesh's keyframes, already in Bevy axes and scale (ue_to_bevy(p * DrawScale), exactly what the CPU
    /// path writes), added once per distinct (mesh, draw scale).
    fn keyframes(&mut self, mesh: &std::sync::Arc<dx_level::actors::AnimMesh>, draw_scale: f32) -> Option<(usize, u32, u32, Vec3, Vec3)> {
        let key = (std::sync::Arc::as_ptr(mesh) as usize, draw_scale.to_bits());
        if let Some(v) = self.keyed.get(&key) {
            return Some(*v);
        }
        let (fv, nf) = (mesh.frame_verts, mesh.frames.max(1));
        let need = fv * nf;
        if need == 0 || need > PAGE_TEXELS {
            return None;
        }
        if self.pages.last().is_none_or(|p| p.images.is_some() || p.len + need > PAGE_TEXELS) {
            self.pages.push(Page::default());
        }
        let pi = self.pages.len() - 1;
        let page = &mut self.pages[pi];
        let base = page.len as u32;
        page.hi.reserve(need * 8);
        page.lo.reserve(need * 8);
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for i in 0..need {
            let u = mesh.verts.get(i).copied().unwrap_or([0.0; 3]);
            let p = Vec3::from(dx_level::ue_to_bevy([u[0] * draw_scale, u[1] * draw_scale, u[2] * draw_scale]));
            lo = lo.min(p);
            hi = hi.max(p);
            for c in 0..4 {
                let v = if c < 3 { p[c] } else { 0.0 };
                let h = half::f16::from_f32(v);
                let l = half::f16::from_f32(v - h.to_f32());
                page.hi.extend_from_slice(&h.to_bits().to_le_bytes());
                page.lo.extend_from_slice(&l.to_bits().to_le_bytes());
            }
        }
        page.len += need;
        if lo.x > hi.x {
            (lo, hi) = (Vec3::ZERO, Vec3::ZERO);
        }
        let v = (pi, base, fv as u32, lo, hi);
        self.keyed.insert(key, v);
        if std::env::var_os("DXVIEW_GPUANIM_LOG").is_some() {
            let total: usize = self.pages.iter().map(|p| p.len).sum();
            println!("GPUANIM +{need} texels: {total} in {} page(s), {} meshes", self.pages.len(), self.keyed.len());
        }
        Some(v)
    }

    /// Upload the pages filled since the last call (each exactly once); returns them.
    fn upload(&mut self, images: &mut Assets<Image>) -> Vec<usize> {
        let mut done = Vec::new();
        for (pi, page) in self.pages.iter_mut().enumerate() {
            if page.images.is_some() || page.len == 0 {
                continue;
            }
            let rows = page.len.div_ceil(TEX_W).max(1);
            let bytes = |mut src: Vec<u8>| {
                src.resize(rows * TEX_W * 8, 0);
                src
            };
            let mk = |data: Vec<u8>| {
                Image::new(
                    Extent3d { width: TEX_W as u32, height: rows as u32, depth_or_array_layers: 1 },
                    TextureDimension::D2,
                    data,
                    TextureFormat::Rgba16Float,
                    RenderAssetUsages::RENDER_WORLD,
                )
            };
            page.rows = rows;
            let (hi, lo) = (std::mem::take(&mut page.hi), std::mem::take(&mut page.lo));
            page.images = Some((images.add(mk(bytes(hi))), images.add(mk(bytes(lo)))));
            done.push(pi);
        }
        done
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
        let Some((page, base, nv, lo, hi)) = ga.keyframes(&anim.mesh, anim.draw_scale) else {
            commands.entity(e).try_insert(GpuAnimSkip);
            continue;
        };
        let mut parts = Vec::new();
        for c in children.iter() {
            let Ok((m3, mm)) = kids.get(c) else { continue };
            let Some(&(_, pi)) = a.parts.iter().find(|(h, _)| h.id() == m3.0.id()) else { continue };
            let Some(part) = ma.parts.get(pi) else { continue };
            if let Some(mut m) = meshes.get_mut(&m3.0) {
                let uv1: Vec<[f32; 2]> = part.vert_ids.iter().map(|&v| [(base + v) as f32, nv as f32]).collect();
                m.insert_attribute(Mesh::ATTRIBUTE_UV_1, VertexAttributeValues::Float32x2(uv1));
            }
            parts.push((c, mm.0.clone(), lo, hi));
        }
        todo.push((e, page, parts));
    }
    let replaced = ga.upload(&mut images);
    for pi in replaced {
        let Some((hi_img, lo_img)) = ga.pages[pi].images.clone() else { continue };
        for ((_, p), h) in &ga.mats {
            if *p == pi {
                if let Some(mut m) = anim_mats.get_mut(h) {
                    m.extension = VertAnim { hi: hi_img.clone(), lo: lo_img.clone() };
                }
            }
        }
    }
    for (e, page, parts) in todo {
        let Some((hi_img, lo_img)) = ga.pages[page].images.clone() else { continue };
        for (c, std_mat, lo, hi) in parts {
            let mat = match ga.mats.get(&(std_mat.id(), page)) {
                Some(h) => h.clone(),
                None => {
                    let Some(base_mat) = std_mats.get(&std_mat).cloned() else { continue };
                    let h = anim_mats.add(AnimMaterial {
                        base: base_mat,
                        extension: VertAnim { hi: hi_img.clone(), lo: lo_img.clone() },
                    });
                    ga.mats.insert((std_mat.id(), page), h.clone());
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
