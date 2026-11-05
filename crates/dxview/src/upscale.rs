//! Display setting "Upscaling" (ours): Off / Native AA / Quality / Balanced / Performance / Ultra Performance.
//!
//! - Off: the world is drawn at the window's size, as before (SMAA on the last camera when MSAA is off).
//! - Native AA: still the window's size, with temporal antialiasing on the world camera (Bevy's TAA: jittered
//!   frames blended through motion vectors) and a light contrast-adaptive sharpen after it.
//! - Quality / Balanced / Performance / Ultra Performance: the world camera draws into its own image 1/1.5, 1/1.7,
//!   1/2 or 1/3 the window's size on each side (AMD's FSR 1 ratios), antialiased there by TAA; a pass on an extra
//!   camera (`UpscaleCam`, between the world and the heat / view-model cameras) blows it up to the window with an
//!   edge-adaptive spatial upscaler (a WGSL port of the EASU half of AMD FidelityFX Super Resolution 1, MIT
//!   licence - see THIRD_PARTY.md), then RCAS sharpens it (Bevy's ContrastAdaptiveSharpening is RCAS). The view
//!   model, the heat view, the HUD and every screen stay at the window's own resolution.
//!
//! macOS has no TAA. Its motion-vector prepass makes a skeletal body with morph targets need more buffers in the vertex
//! stage than wgpu's Metal backend allows (16): the pipeline fails and Bevy quits the game. The prepass can't be taken
//! off a running camera safely, and Skeletal Characters can change in play (the Classic / Modern switch), so TAA stays
//! off there. Native AA is its sharpen alone, and the upscaled modes are EASU and RCAS without the temporal blend.
//!
//! The world camera keeps the window's logical size (its image's scale factor is divided by the same ratio), so
//! world_to_viewport / viewport_to_world on it still speak window coordinates.

use bevy::anti_alias::contrast_adaptive_sharpening::ContrastAdaptiveSharpening;
use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::camera::{Hdr, ImageRenderTarget, RenderTarget};
use bevy::render::camera::{MipBias, TemporalJitter};
use bevy::camera::visibility::RenderLayers;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::view::Msaa;
use bevy::window::PrimaryWindow;

/// The setting: 0 Off, 1 Native AA, 2 Quality, 3 Balanced, 4 Performance, 5 Ultra Performance.
pub fn mode() -> u32 {
    (setting!("Upscaling").round().max(0.0) as u32).min(5)
}

/// How many window pixels one rendered pixel covers on each side.
pub fn ratio(mode: u32) -> f32 {
    match mode {
        2 => 1.5,
        3 => 1.7,
        4 => 2.0,
        5 => 3.0,
        _ => 1.0,
    }
}

pub const NAMES: [&str; 6] = ["Off", "Native AA", "Quality", "Balanced", "Performance", "Ultra Performance"];

/// The camera that blows the world's picture up into the window.
#[derive(Component)]
pub struct UpscaleCam;

/// On the UpscaleCam: the low-resolution picture it upscales (extracted to the render world).
#[derive(Component, Clone, bevy::render::extract_component::ExtractComponent)]
pub struct UpscaleSource(pub Handle<Image>);

#[derive(Default)]
pub struct UpState {
    image: Option<(Handle<Image>, UVec2)>,
    applied: Option<(u32, UVec2, u32)>,
}

pub fn spawn_upscale_camera(commands: &mut Commands) {
    commands.spawn((
        Camera3d::default(),
        Camera {
            // After the world camera (-3) and before the heat (2) and view-model (3) cameras.
            order: 1,
            is_active: false,
            clear_color: ClearColorConfig::None,
            output_mode: bevy::camera::CameraOutputMode::Skip,
            ..default()
        },
        Tonemapping::None,
        bevy::camera::Exposure { ev100: -0.263 },
        Hdr,
        Msaa::Off,
        // It draws nothing itself: its only pass is the upscale.
        RenderLayers::none(),
        bevy::light::cluster::ClusterConfig::Single,
        Transform::default(),
        UpscaleCam,
    ));
}

fn same(a: &RenderTarget, b: &RenderTarget) -> bool {
    match (a, b) {
        (RenderTarget::Window(_), RenderTarget::Window(_)) => true,
        (RenderTarget::Image(x), RenderTarget::Image(y)) => x == y,
        _ => false,
    }
}

fn target_image(size: UVec2) -> Image {
    let mut image = Image::new_uninit(
        Extent3d { width: size.x, height: size.y, depth_or_array_layers: 1 },
        TextureDimension::D2,
        TextureFormat::Rgba16Float,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage |= TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::RENDER_ATTACHMENT;
    image
}

#[allow(clippy::type_complexity)]
pub fn upscale_system(
    mut commands: Commands,
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    mut images: ResMut<Assets<Image>>,
    mut world: Query<(Entity, &mut Camera, &mut RenderTarget, Has<TemporalAntiAliasing>, Option<&mut ContrastAdaptiveSharpening>), (With<crate::FlyCam>, Without<crate::SkyCam>, Without<UpscaleCam>)>,
    mut sky: Query<&mut RenderTarget, (With<crate::SkyCam>, Without<crate::FlyCam>, Without<UpscaleCam>)>,
    mut up: Query<(Entity, &mut Camera, Option<&mut ContrastAdaptiveSharpening>), (With<UpscaleCam>, Without<crate::FlyCam>)>,
    mut st: Local<UpState>,
) {
    if up.is_empty() {
        spawn_upscale_camera(&mut commands);
        return;
    }
    let Some(window) = window else { return };
    let m = mode();
    let phys = UVec2::new(window.physical_width().max(1), window.physical_height().max(1));
    let r = ratio(m);
    let size = if m >= 2 { (phys.as_vec2() / r).round().as_uvec2().max(UVec2::ONE) } else { phys };
    // The world camera can be rebuilt by a new map: check its make-up each frame (cheap), rebuild the image only
    // when the size changes.
    if m >= 2 && st.image.as_ref().is_none_or(|i| i.1 != size) {
        let h = images.add(target_image(size));
        st.image = Some((h, size));
    }
    let key = (m, size, (window.scale_factor() * 1000.0) as u32);
    let changed = st.applied != Some(key);
    st.applied = Some(key);
    let want_target = match (&st.image, m >= 2) {
        (Some((h, _)), true) => RenderTarget::Image(ImageRenderTarget {
            handle: h.clone(),
            // Logical size = the window's: physical / scale = (W / r) / (s / r).
            scale_factor: window.scale_factor() / r,
        }),
        _ => RenderTarget::default(),
    };
    for (e, mut cam, mut target, has_taa, cas) in &mut world {
        if changed || !same(&target, &want_target) {
            *target = want_target.clone();
        }
        let out = if m >= 2 { bevy::camera::CameraOutputMode::default() } else { bevy::camera::CameraOutputMode::Skip };
        if std::mem::discriminant(&cam.output_mode) != std::mem::discriminant(&out) {
            cam.output_mode = out;
        }
        let taa = m >= 1 && !cfg!(target_os = "macos");
        if taa && !has_taa {
            commands.entity(e).insert((TemporalAntiAliasing::default(), Msaa::Off));
        } else if !taa && has_taa {
            // The motion-vector and depth prepasses stay (part of every material's pipeline); the jitter must go.
            commands.entity(e).remove::<(TemporalAntiAliasing, TemporalJitter, MipBias)>();
        }
        // Native AA: a light sharpen after the temporal blend (the upscaled modes sharpen after upscaling instead).
        let want = (m == 1).then_some(0.35);
        match (want, cas) {
            (Some(s), None) => {
                commands.entity(e).insert(ContrastAdaptiveSharpening { enabled: true, sharpening_strength: s, denoise: false });
            }
            (None, Some(_)) => {
                commands.entity(e).remove::<ContrastAdaptiveSharpening>();
            }
            _ => {}
        }
    }
    for mut target in &mut sky {
        if !same(&target, &want_target) {
            *target = want_target.clone();
        }
    }
    for (e, mut cam, cas) in &mut up {
        let on = m >= 2;
        if cam.is_active != on {
            cam.is_active = on;
        }
        if on {
            if let Some((h, _)) = &st.image {
                commands.entity(e).insert(UpscaleSource(h.clone()));
            }
            // Stronger for the bigger stretches (FSR's own default is 0.2 stops of RCAS sharpness).
            let s = match m { 2 => 0.45, 3 => 0.5, 4 => 0.55, _ => 0.6 };
            match cas {
                Some(mut c) if c.sharpening_strength != s => c.sharpening_strength = s,
                Some(_) => {}
                None => {
                    commands.entity(e).insert(ContrastAdaptiveSharpening { enabled: true, sharpening_strength: s, denoise: false });
                }
            }
        } else if changed {
            commands.entity(e).remove::<UpscaleSource>();
        }
    }
}

pub const EASU_SHADER: Handle<Shader> = bevy::asset::uuid_handle!("3f0c5a52-7d8e-4a61-b1c2-9e4f6a0d2b77");

pub struct UpscalePlugin;

impl Plugin for UpscalePlugin {
    fn build(&self, app: &mut App) {
        use bevy::core_pipeline::{schedule::Core3d, Core3dSystems};
        use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
        app.add_systems(Startup, |mut shaders: ResMut<Assets<Shader>>| {
            let _ = shaders.insert(&EASU_SHADER, Shader::from_wgsl(include_str!("shaders/easu.wgsl"), "dxview/easu.wgsl"));
        });
        app.add_plugins(bevy::render::extract_component::ExtractComponentPlugin::<UpscaleSource>::default());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .add_systems(RenderStartup, render::init_pipeline)
            .add_systems(Render, render::prepare.in_set(RenderSystems::Prepare))
            .add_systems(Core3d, render::easu.in_set(Core3dSystems::EarlyPostProcess));
    }
}

mod render {
    use super::UpscaleSource;
    use bevy::core_pipeline::FullscreenShader;
    use bevy::prelude::*;
    use bevy::render::{
        render_asset::RenderAssets,
        render_resource::{
            binding_types::texture_2d, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries,
            CachedRenderPipelineId, ColorTargetState, ColorWrites, FragmentState, Operations, PipelineCache,
            RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor, ShaderStages,
            TextureFormat, TextureSampleType,
        },
        renderer::{RenderContext, ViewQuery},
        texture::GpuImage,
        view::{ExtractedView, ViewTarget},
    };
    use std::collections::HashMap;

    #[derive(Resource)]
    pub struct EasuPipeline {
        layout: BindGroupLayoutDescriptor,
        vertex: bevy::render::render_resource::VertexState,
        ids: HashMap<TextureFormat, CachedRenderPipelineId>,
    }

    #[derive(Component)]
    pub struct EasuPipelineId(CachedRenderPipelineId);

    pub fn init_pipeline(mut commands: Commands, fullscreen: Res<FullscreenShader>) {
        let layout = BindGroupLayoutDescriptor::new(
            "dxview_easu_layout",
            &BindGroupLayoutEntries::sequential(ShaderStages::FRAGMENT, (texture_2d(TextureSampleType::Float { filterable: true }),)),
        );
        commands.insert_resource(EasuPipeline { layout, vertex: fullscreen.to_vertex_state(), ids: HashMap::new() });
    }

    pub fn prepare(
        mut commands: Commands,
        cache: Res<PipelineCache>,
        mut pipe: ResMut<EasuPipeline>,
        views: Query<(Entity, &ExtractedView), With<UpscaleSource>>,
    ) {
        for (e, view) in &views {
            let fmt = view.target_format;
            let id = if let Some(id) = pipe.ids.get(&fmt) {
                *id
            } else {
                let desc = RenderPipelineDescriptor {
                    label: Some("dxview_easu".into()),
                    layout: vec![pipe.layout.clone()],
                    vertex: pipe.vertex.clone(),
                    fragment: Some(FragmentState {
                        shader: super::EASU_SHADER,
                        targets: vec![Some(ColorTargetState { format: fmt, blend: None, write_mask: ColorWrites::ALL })],
                        ..default()
                    }),
                    ..default()
                };
                let id = cache.queue_render_pipeline(desc);
                pipe.ids.insert(fmt, id);
                id
            };
            commands.entity(e).insert(EasuPipelineId(id));
        }
    }

    pub fn easu(
        view: ViewQuery<(&ViewTarget, &UpscaleSource, &EasuPipelineId)>,
        pipe: Res<EasuPipeline>,
        cache: Res<PipelineCache>,
        images: Res<RenderAssets<GpuImage>>,
        mut ctx: RenderContext,
    ) {
        let (target, src, id) = view.into_inner();
        let Some(pipeline) = cache.get_render_pipeline(id.0) else { return };
        let Some(img) = images.get(src.0.id()) else { return };
        let bind = ctx.render_device().create_bind_group(
            "dxview_easu_bind",
            &cache.get_bind_group_layout(&pipe.layout),
            &BindGroupEntries::sequential((&img.texture_view,)),
        );
        // Every pixel is written: straight into the current main texture (no ping-pong needed).
        let desc = RenderPassDescriptor {
            label: Some("dxview_easu"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: target.main_texture_view(),
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        };
        let mut pass = ctx.command_encoder().begin_render_pass(&desc);
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }
}
