//! Lean Rendering's lightmap lights: the dynamic lights (dynlight.rs's point lights and the Flashlight Cone's spots)
//! added into the level's lightmap where they reach, every frame, the way dx-level's bake added the static ones and
//! Unreal 1 lit its lightmaps. The level's shader then reads them with the lightmap and needs no walk over the lights
//! in each pixel's cluster (bsp_shadow.wgsl's classic_lighting compiled with BSP_NO_DYNLIGHTS), a walk that cost
//! every pixel though a light reaches only a few of them.
//!
//! The lit lightmap is a float texture the size of the bake's atlas, holding linear values as the shader's sum did,
//! so a bright light still takes a surface past its texture's own colour. A map's first frame copies the bake into
//! it; after that only the blocks (each surface's part of the atlas, `dx_level::lighting::Patch`) whose lights
//! changed are drawn again, with their lights or, once a light has left, as baked. The texels get what the shader
//! gave the pixels: from_srgb(to_srgb(baked) + colour x brightness x LIGHT_GAIN x (1 - d / r) x N.L [x cone^2]), at
//! the lightmap's texels rather than per pixel (the bake's own resolution), a two-sided surface lit from either side.

use bevy::prelude::*;
use dx_level::lighting::Patch;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// The lit lightmap is in use: the level's shader leaves the dynamic lights to it (shadowlight.rs `BspShadowKey`).
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// The work for the render world, a frame's at a time (normally one).
static JOBS: Mutex<Vec<Job>> = Mutex::new(Vec::new());
/// The render world dropped work it couldn't draw: everything is sent again.
static RESYNC: AtomicBool = AtomicBool::new(false);
/// The current map's lit lightmap (work for another is dropped).
static TARGET: Mutex<Option<AssetId<Image>>> = Mutex::new(None);

/// Lights a frame at most (the 24 pool lights and the cone's two): one bit each in a block's mask.
const MAX_LIGHTS: usize = 32;
/// Bytes a light takes in the shader's uniform array, and a block in the instance buffer.
const LIGHT_BYTES: usize = 64;
const BLOCK_BYTES: usize = 96;
/// The block lookup's cell (Unreal units).
const CELL: f32 = 512.0;

pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// A map's lit lightmap, the blocks it can light and what was last drawn into it.
#[derive(Resource)]
pub struct LmLights {
    base: Handle<Image>,
    target: Handle<Image>,
    /// The atlas's width and height (texels).
    size: [u32; 2],
    patches: Vec<Patch>,
    grid: HashMap<(i32, i32, i32), Vec<u32>>,
    /// Block -> the signature of the lights it was drawn with.
    drawn: HashMap<u32, u64>,
    /// The bake still has to be copied in.
    full: bool,
    /// Per block: this frame's lights (bits), and when the lookup last met it.
    masks: Vec<u32>,
    met: Vec<u32>,
    stamp: u32,
}

/// A map without the lit lightmap: the last map's is let go.
pub fn clear(commands: &mut Commands) {
    ACTIVE.store(false, Ordering::Relaxed);
    commands.remove_resource::<LmLights>();
    *TARGET.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// At a map's setup (main.rs): the lit lightmap for the level's surfaces when Lean Rendering's Classic lights are
/// on, else None (the level draws with the bake, its dynamic lights walked per pixel). `base` is the bake's image.
pub fn setup(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    atlas: &dx_level::lighting::Atlas,
    base: &Handle<Image>,
) -> Option<Handle<Image>> {
    use bevy::asset::RenderAssetUsages;
    use bevy::image::{ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
    if !(crate::shadowlight::classic_lights() && !atlas.patches.is_empty()) {
        clear(commands);
        return None;
    }
    ACTIVE.store(true, Ordering::Relaxed);
    let mut img = Image::new_uninit(
        Extent3d { width: atlas.width, height: atlas.height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        TextureFormat::Rgba16Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.texture_descriptor.usage |= TextureUsages::RENDER_ATTACHMENT;
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    let target = images.add(img);
    *TARGET.lock().unwrap_or_else(|e| e.into_inner()) = Some(target.id());
    let mut grid: HashMap<(i32, i32, i32), Vec<u32>> = HashMap::new();
    for (i, p) in atlas.patches.iter().enumerate() {
        let (lo, hi) = (cell(p.min), cell(p.max));
        for x in lo.0..=hi.0 {
            for y in lo.1..=hi.1 {
                for z in lo.2..=hi.2 {
                    grid.entry((x, y, z)).or_default().push(i as u32);
                }
            }
        }
    }
    println!(
        "[DX RUST] lightmap lights: {} surface blocks in a {}x{} atlas ({:.1} MiB lit lightmap)",
        atlas.patches.len(),
        atlas.width,
        atlas.height,
        (atlas.width as f64 * atlas.height as f64 * 8.0) / (1024.0 * 1024.0)
    );
    let n = atlas.patches.len();
    commands.insert_resource(LmLights {
        base: base.clone(),
        target: target.clone(),
        size: [atlas.width, atlas.height],
        patches: atlas.patches.clone(),
        grid,
        drawn: HashMap::new(),
        full: true,
        masks: vec![0; n],
        met: vec![0; n],
        stamp: 0,
    });
    Some(target)
}

fn cell(p: [f32; 3]) -> (i32, i32, i32) {
    ((p[0] / CELL).floor() as i32, (p[1] / CELL).floor() as i32, (p[2] / CELL).floor() as i32)
}

/// A frame's work: the lights (the shader's uniform array) and the blocks to draw, the whole atlas's copy first
/// when `full`.
struct Job {
    base: AssetId<Image>,
    target: AssetId<Image>,
    full: bool,
    lights: Vec<u8>,
    blocks: Vec<u8>,
    count: u32,
}

/// One light as the shader takes it (Unreal units), and the signature of what it does.
struct Light {
    pos: [f32; 3],
    radius: f32,
    color: [f32; 3],
    /// A spot light: direction, cone scale and offset (Bevy's spot_light).
    spot: Option<([f32; 3], f32, f32)>,
}

impl Light {
    fn sig(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for v in self.pos.iter().chain([self.radius].iter()).chain(self.color.iter()) {
            v.to_bits().hash(&mut h);
        }
        if let Some((d, s, o)) = self.spot {
            for v in d.iter().chain([s, o].iter()) {
                v.to_bits().hash(&mut h);
            }
        }
        h.finish()
    }

    fn bytes(&self, out: &mut Vec<u8>) {
        let (dir, scale, offset) = self.spot.unwrap_or(([0.0; 3], 0.0, 0.0));
        let spot = if self.spot.is_some() { 1.0 } else { 0.0 };
        let vals = [
            self.pos[0], self.pos[1], self.pos[2], self.radius,
            self.color[0], self.color[1], self.color[2], spot,
            dir[0], dir[1], dir[2], scale,
            offset, 0.0, 0.0, 0.0,
        ];
        for v in vals {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
}

/// The walk's colour for a light (bsp_shadow.wgsl): Bevy's linear colour scaled to its brightest channel, back in
/// gamma space, times the Unreal brightness the light carries (negated) in its depth bias, times LIGHT_GAIN.
fn walk_color(c: Color, brightness: f32) -> [f32; 3] {
    let l = c.to_linear();
    let m = l.red.max(l.green).max(l.blue.max(1e-8));
    let k = brightness * dx_level::lighting::LIGHT_GAIN;
    [to_srgb(l.red / m) * k, to_srgb(l.green / m) * k, to_srgb(l.blue / m) * k]
}

fn to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.max(0.0).powf(1.0 / 2.4) - 0.055
    }
}

/// Bevy space to Unreal's: the axes swapped back (the scale is in `bevy_to_ue` for points).
fn dir_to_ue(d: Vec3) -> [f32; 3] {
    [d.x, d.z, d.y]
}

fn block_bytes(p: &Patch, mask: u32, out: &mut Vec<u8>) {
    let r = p.rect;
    for v in [r[0], r[1], r[2], r[3], r[2].saturating_sub(2).max(1), r[3].saturating_sub(2).max(1), mask, p.two_sided as u32] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for v3 in [p.origin, p.du, p.dv, p.normal] {
        for v in [v3[0], v3[1], v3[2], 0.0] {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
}

/// Every frame after dynlight.rs has placed its lights: the blocks whose lights changed, for the render world.
pub fn lmlights_system(
    lm: Option<ResMut<LmLights>>,
    slots: Query<(&PointLight, &Transform, &Visibility), With<crate::dynlight::DynLightSlot>>,
    cone: Query<(&SpotLight, &Transform, &Visibility), With<crate::dynlight::FlashCone>>,
) {
    let _prof = crate::prof::scope("lmlights::lmlights_system");
    let Some(mut lm) = lm else {
        ACTIVE.store(false, Ordering::Relaxed);
        return;
    };
    let lm = &mut *lm;
    let on = crate::shadowlight::classic_lights();
    ACTIVE.store(on, Ordering::Relaxed);
    if RESYNC.swap(false, Ordering::Relaxed) {
        lm.drawn.clear();
        lm.full = true;
    }
    let mut job = Job {
        base: lm.base.id(),
        target: lm.target.id(),
        full: std::mem::take(&mut lm.full),
        lights: Vec::with_capacity(MAX_LIGHTS * LIGHT_BYTES),
        blocks: Vec::new(),
        count: 0,
    };
    if job.full {
        // The whole atlas as baked, under everything else.
        job.count += 1;
        for v in [0u32, 0, lm.size[0], lm.size[1], 1, 1, 0, 0] {
            job.blocks.extend_from_slice(&v.to_le_bytes());
        }
        job.blocks.extend_from_slice(&[0u8; 64]);
    }
    let mut lights: Vec<Light> = Vec::new();
    if on {
        for (pl, tf, vis) in &slots {
            if *vis == Visibility::Hidden || pl.intensity <= 0.0 || pl.shadow_depth_bias >= 0.0 || !pl.affects_lightmapped_mesh_diffuse {
                continue;
            }
            lights.push(Light {
                pos: dx_level::bevy_to_ue(tf.translation.to_array()),
                radius: pl.range / dx_level::METERS_PER_UNIT,
                color: walk_color(pl.color, -pl.shadow_depth_bias),
                spot: None,
            });
        }
        for (sl, tf, vis) in &cone {
            if *vis == Visibility::Hidden || sl.intensity <= 0.0 || sl.shadow_depth_bias >= 0.0 || !sl.affects_lightmapped_mesh_diffuse {
                continue;
            }
            let cos_outer = sl.outer_angle.cos();
            let scale = 1.0 / (sl.inner_angle.cos() - cos_outer).max(1e-4);
            lights.push(Light {
                pos: dx_level::bevy_to_ue(tf.translation.to_array()),
                radius: sl.range / dx_level::METERS_PER_UNIT,
                color: walk_color(sl.color, -sl.shadow_depth_bias),
                spot: Some((dir_to_ue(tf.forward().as_vec3()), scale, -cos_outer * scale)),
            });
        }
        lights.truncate(MAX_LIGHTS);
    }
    let sigs: Vec<u64> = lights.iter().map(Light::sig).collect();
    // Which blocks each light reaches: in its sphere, in front of the surface (either side of a two-sided one).
    lm.stamp = lm.stamp.wrapping_add(1).max(1);
    let stamp = lm.stamp;
    let mut touched: Vec<u32> = Vec::new();
    for (li, l) in lights.iter().enumerate() {
        let r = l.radius;
        let (lo, hi) = (cell([l.pos[0] - r, l.pos[1] - r, l.pos[2] - r]), cell([l.pos[0] + r, l.pos[1] + r, l.pos[2] + r]));
        for x in lo.0..=hi.0 {
            for y in lo.1..=hi.1 {
                for z in lo.2..=hi.2 {
                    let Some(list) = lm.grid.get(&(x, y, z)) else { continue };
                    for &pi in list {
                        let i = pi as usize;
                        let bit = 1u32 << li;
                        if lm.met[i] == stamp && lm.masks[i] & bit != 0 {
                            continue;
                        }
                        let p = &lm.patches[i];
                        let mut d2 = 0.0f32;
                        for k in 0..3 {
                            let c = l.pos[k].clamp(p.min[k], p.max[k]) - l.pos[k];
                            d2 += c * c;
                        }
                        if d2 >= r * r {
                            continue;
                        }
                        let pd = (0..3).map(|k| p.normal[k] * (l.pos[k] - p.origin[k])).sum::<f32>();
                        if pd.abs() >= r || (!p.two_sided && pd <= 0.0) {
                            continue;
                        }
                        if lm.met[i] != stamp {
                            lm.met[i] = stamp;
                            lm.masks[i] = 0;
                            touched.push(pi);
                        }
                        lm.masks[i] |= bit;
                    }
                }
            }
        }
    }
    // Draw the blocks whose lights changed, and give back the bake to those a light has left.
    for &pi in &touched {
        let mask = lm.masks[pi as usize];
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let mut m = mask;
        while m != 0 {
            sigs[m.trailing_zeros() as usize].hash(&mut h);
            m &= m - 1;
        }
        let sig = h.finish();
        if lm.drawn.get(&pi) != Some(&sig) {
            lm.drawn.insert(pi, sig);
            block_bytes(&lm.patches[pi as usize], mask, &mut job.blocks);
            job.count += 1;
        }
    }
    let gone: Vec<u32> = lm.drawn.keys().copied().filter(|&pi| lm.met[pi as usize] != stamp).collect();
    for pi in gone {
        lm.drawn.remove(&pi);
        block_bytes(&lm.patches[pi as usize], 0, &mut job.blocks);
        job.count += 1;
    }
    if job.count == 0 {
        return;
    }
    for l in &lights {
        l.bytes(&mut job.lights);
    }
    job.lights.resize(MAX_LIGHTS * LIGHT_BYTES, 0);
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).push(job);
}

pub const LMLIGHTS_SHADER: Handle<Shader> = bevy::asset::uuid_handle!("dab45991-822b-401c-b42c-9dbb48e30a8b");

pub struct LmLightsPlugin;

impl Plugin for LmLightsPlugin {
    fn build(&self, app: &mut App) {
        use bevy::render::renderer::{RenderGraph, RenderGraphSystems};
        use bevy::render::{RenderApp, RenderStartup};
        app.add_systems(Startup, |mut shaders: ResMut<Assets<Shader>>| {
            let _ = shaders.insert(&LMLIGHTS_SHADER, Shader::from_wgsl(include_str!("shaders/lmlights.wgsl"), "dxview/lmlights.wgsl"));
        });
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .add_systems(RenderStartup, render::init_pipeline)
            .add_systems(RenderGraph, render::draw.in_set(RenderGraphSystems::Begin));
    }
}

mod render {
    use super::{Job, BLOCK_BYTES, JOBS, LIGHT_BYTES, LMLIGHTS_SHADER, MAX_LIGHTS, RESYNC, TARGET};
    use bevy::prelude::*;
    use bevy::render::{
        render_asset::RenderAssets,
        render_resource::{
            binding_types::{texture_2d, uniform_buffer_sized},
            BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, BufferInitDescriptor, BufferUsages,
            CachedRenderPipelineId, ColorTargetState, ColorWrites, FragmentState, LoadOp, Operations, PipelineCache,
            RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor, ShaderStages, StoreOp,
            TextureFormat, TextureSampleType, VertexAttribute, VertexFormat, VertexState, VertexStepMode,
        },
        renderer::RenderContext,
        texture::GpuImage,
    };
    use bevy::mesh::VertexBufferLayout;
    use std::sync::atomic::Ordering;

    #[derive(Resource)]
    pub struct LmPipeline {
        layout: BindGroupLayoutDescriptor,
        id: CachedRenderPipelineId,
    }

    pub fn init_pipeline(mut commands: Commands, cache: Res<PipelineCache>) {
        let layout = BindGroupLayoutDescriptor::new(
            "dxview_lmlights_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX_FRAGMENT,
                (
                    texture_2d(TextureSampleType::Float { filterable: false }),
                    uniform_buffer_sized(false, std::num::NonZero::new((MAX_LIGHTS * LIGHT_BYTES) as u64)),
                ),
            ),
        );
        let attributes = [
            (VertexFormat::Uint32x4, 0),
            (VertexFormat::Uint32x4, 16),
            (VertexFormat::Float32x4, 32),
            (VertexFormat::Float32x4, 48),
            (VertexFormat::Float32x4, 64),
            (VertexFormat::Float32x4, 80),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (format, offset))| VertexAttribute { format, offset, shader_location: i as u32 })
        .collect();
        let id = cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("dxview_lmlights".into()),
            layout: vec![layout.clone()],
            vertex: VertexState {
                shader: LMLIGHTS_SHADER,
                shader_defs: vec![],
                entry_point: Some("vertex".into()),
                buffers: vec![VertexBufferLayout {
                    array_stride: BLOCK_BYTES as u64,
                    step_mode: VertexStepMode::Instance,
                    attributes,
                }],
            },
            fragment: Some(FragmentState {
                shader: LMLIGHTS_SHADER,
                shader_defs: vec![],
                entry_point: Some("fragment".into()),
                targets: vec![Some(ColorTargetState { format: TextureFormat::Rgba16Float, blend: None, write_mask: ColorWrites::ALL })],
            }),
            ..default()
        });
        commands.insert_resource(LmPipeline { layout, id });
    }

    /// Before the cameras draw (the RenderGraph schedule's Begin set): the frame's blocks into the lit lightmap.
    pub fn draw(
        pipe: Option<Res<LmPipeline>>,
        cache: Res<PipelineCache>,
        images: Res<RenderAssets<GpuImage>>,
        mut ctx: RenderContext,
    ) {
        let mut jobs: Vec<Job> = std::mem::take(&mut *JOBS.lock().unwrap_or_else(|e| e.into_inner()));
        if jobs.is_empty() {
            return;
        }
        let current = *TARGET.lock().unwrap_or_else(|e| e.into_inner());
        jobs.retain(|j| Some(j.target) == current);
        let Some(first) = jobs.first() else { return };
        let ready = pipe.as_ref().and_then(|p| cache.get_render_pipeline(p.id)).is_some()
            && images.get(first.base).is_some()
            && images.get(first.target).is_some();
        if !ready {
            // Kept for the next frame; if it piles up, everything is sent again instead.
            if jobs.len() > 8 {
                RESYNC.store(true, Ordering::Relaxed);
            } else {
                let mut q = JOBS.lock().unwrap_or_else(|e| e.into_inner());
                let newer = std::mem::take(&mut *q);
                *q = jobs;
                q.extend(newer);
            }
            return;
        }
        let (Some(pipe), Some(base), Some(target)) = (pipe, images.get(first.base), images.get(first.target)) else { return };
        let Some(pipeline) = cache.get_render_pipeline(pipe.id) else { return };
        let layout = cache.get_bind_group_layout(&pipe.layout);
        for job in &jobs {
            let device = ctx.render_device().clone();
            let lights = device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("dxview_lmlights_lights"),
                contents: &job.lights,
                usage: BufferUsages::UNIFORM,
            });
            let blocks = device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("dxview_lmlights_blocks"),
                contents: &job.blocks,
                usage: BufferUsages::VERTEX,
            });
            let bind = device.create_bind_group(
                "dxview_lmlights_bind",
                &layout,
                &BindGroupEntries::sequential((&base.texture_view, lights.as_entire_binding())),
            );
            let load = if job.full { LoadOp::Clear(Default::default()) } else { LoadOp::Load };
            let desc = RenderPassDescriptor {
                label: Some("dxview_lmlights"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &target.texture_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations { load, store: StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            };
            let mut pass = ctx.command_encoder().begin_render_pass(&desc);
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind, &[]);
            // (Bevy's Buffer derefs to wgpu's, whose slice a raw render pass takes.)
            pass.set_vertex_buffer(0, (*blocks).slice(..));
            pass.draw(0..6, 0..job.count);
        }
    }
}
