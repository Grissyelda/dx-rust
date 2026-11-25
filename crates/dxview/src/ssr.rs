//! Screen-Space Reflections (setting Reflections: Off / Subtle / Glossy).
//!
//! Polished floors, tiles, metal, and wet ground reflect what's on screen: a full-screen pass (shaders/ssr.wgsl) on
//! the world camera after its main passes marches each reflecting pixel's mirrored ray through the depth prepass.
//! Which pixels reflect comes from the BSP shader itself: it writes the surface's reflectivity (relight::params,
//! from the material table) into the picture's alpha, so no extra target or G-buffer is needed and people,
//! decorations and the sky never turn into mirrors.
//!
//! Bevy's own SSR needs deferred rendering; our world is drawn forward, hence this pass. It needs the world camera's
//! depth and normal prepasses, which are added the first time reflections are on and then kept (they're part of
//! every material's pipeline: taking them off again would recompile everything).

use bevy::prelude::*;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::render_resource::ShaderType;

pub const SSR_SHADER: Handle<Shader> = bevy::asset::uuid_handle!("8a3e51c2-4b7d-4f19-9d6e-1c0b7a2e5f84");

/// Off / Subtle / Glossy.
pub fn level() -> u32 {
    (setting!("Reflections").round().max(0.0) as u32).min(2)
}

/// How reflective a BSP surface is drawn (0..1) for the material table's numbers: polished, smooth surfaces most.
/// The level setting scales it; 0 when reflections are off.
pub fn reflectivity(roughness: f32, spec: f32, metal: f32, name: &str) -> f32 {
    let lv = level();
    if lv == 0 || crate::platform::classic_only() {
        return 0.0;
    }
    let n = name.to_ascii_lowercase();
    // Windows are drawn by Modern Glass; skies, mirrors (their own planar reflection) and water (Modern Water's own)
    // never take this pass.
    if ["sky", "mirror", "water", "glass", "window", "wndow"].iter().any(|w| n.contains(w)) {
        return 0.0;
    }
    // Test aid: every solid surface a mirror-like floor.
    if env_on!("DXVIEW_SSR_ALL") {
        return 0.9;
    }
    let smooth = (1.0 - (roughness - 0.1) / 0.6).clamp(0.0, 1.0);
    let base = smooth * (0.45 + 0.35 * spec.min(1.4) / 1.4 + 0.2 * metal);
    // Floors meant to shine read as wet / polished in Glossy.
    let r = match lv {
        1 => base * 0.55,
        _ => (base * 1.1 + if smooth > 0.2 { 0.08 } else { 0.0 }).min(1.0),
    };
    if r < 0.04 { 0.0 } else { r }
}

#[derive(Component, Clone, Copy, ExtractComponent, ShaderType, Default)]
pub struct SsrSettings {
    /// x strength, y steps, z max distance (m), w debug.
    pub a: Vec4,
}

#[derive(Default)]
pub struct State {
    prepasses: bool,
}

/// Keeps the world camera's SsrSettings in step with the setting (and adds the prepasses once).
pub fn ssr_system(
    mut commands: Commands,
    mut cams: Query<(Entity, Option<&mut SsrSettings>), (With<crate::FlyCam>, Without<crate::SkyCam>)>,
    mut st: Local<State>,
) {
    let lv = level();
    let on = lv > 0 && !crate::platform::classic_only();
    for (e, s) in &mut cams {
        if on && !st.prepasses {
            st.prepasses = true;
            commands.entity(e).insert((bevy::core_pipeline::prepass::DepthPrepass, bevy::core_pipeline::prepass::NormalPrepass));
        }
        if on {
            let want = SsrSettings {
                a: Vec4::new(
                    if lv == 2 { 1.0 } else { 0.75 },
                    if lv == 2 { 28.0 } else { 20.0 },
                    if lv == 2 { 40.0 } else { 25.0 },
                    if env_on!("DXVIEW_SSR_DEBUG") { 1.0 } else { 0.0 },
                ),
            };
            match s {
                Some(mut s) => {
                    if s.a != want.a {
                        *s = want;
                    }
                }
                None => {
                    commands.entity(e).insert(want);
                }
            }
        } else if s.is_some() {
            commands.entity(e).remove::<SsrSettings>();
        }
    }
}

pub struct SsrPlugin;

impl Plugin for SsrPlugin {
    fn build(&self, app: &mut App) {
        use bevy::core_pipeline::{schedule::Core3d, tonemapping::tonemapping, Core3dSystems};
        use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
        app.add_systems(Startup, |mut shaders: ResMut<Assets<Shader>>| {
            let _ = shaders.insert(&SSR_SHADER, Shader::from_wgsl(include_str!("shaders/ssr.wgsl"), "dxview/ssr.wgsl"));
        });
        app.add_plugins((
            bevy::render::extract_component::ExtractComponentPlugin::<SsrSettings>::default(),
            bevy::render::extract_component::UniformComponentPlugin::<SsrSettings>::default(),
        ));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .add_systems(RenderStartup, render::init_pipeline)
            .add_systems(Render, render::prepare.in_set(RenderSystems::Prepare))
            .add_systems(Core3d, render::ssr.in_set(Core3dSystems::PostProcess).before(tonemapping));
    }
}

mod render {
    use super::SsrSettings;
    use bevy::core_pipeline::prepass::ViewPrepassTextures;
    use bevy::core_pipeline::FullscreenShader;
    use bevy::prelude::*;
    use bevy::render::{
        extract_component::{ComponentUniforms, DynamicUniformIndex},
        render_resource::{
            binding_types::{sampler, texture_2d, texture_2d_multisampled, texture_depth_2d, texture_depth_2d_multisampled, uniform_buffer},
            BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, CachedRenderPipelineId, ColorTargetState,
            ColorWrites, FragmentState, Operations, PipelineCache, RenderPassColorAttachment, RenderPassDescriptor,
            RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages, TextureFormat,
            TextureSampleType,
        },
        renderer::{RenderContext, RenderDevice, ViewQuery},
        view::{ExtractedView, Msaa, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms},
    };
    use std::collections::HashMap;

    static LOGGED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    #[derive(Resource)]
    pub struct SsrPipeline {
        /// Single-sampled and MSAA prepass textures.
        layouts: [BindGroupLayoutDescriptor; 2],
        vertex: bevy::render::render_resource::VertexState,
        sampler: Sampler,
        ids: HashMap<(TextureFormat, bool), CachedRenderPipelineId>,
    }

    #[derive(Component)]
    pub struct SsrPipelineId(CachedRenderPipelineId, bool);

    pub fn init_pipeline(mut commands: Commands, fullscreen: Res<FullscreenShader>, device: Res<RenderDevice>) {
        let single = BindGroupLayoutDescriptor::new(
            "dxview_ssr_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    texture_depth_2d(),
                    texture_2d(TextureSampleType::Float { filterable: false }),
                    uniform_buffer::<ViewUniform>(true),
                    uniform_buffer::<SsrSettings>(true),
                ),
            ),
        );
        // With MSAA the prepass textures are multisampled (the picture itself is resolved by then).
        let multi = BindGroupLayoutDescriptor::new(
            "dxview_ssr_layout_msaa",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    texture_depth_2d_multisampled(),
                    texture_2d_multisampled(TextureSampleType::Float { filterable: false }),
                    uniform_buffer::<ViewUniform>(true),
                    uniform_buffer::<SsrSettings>(true),
                ),
            ),
        );
        let sampler = device.create_sampler(&SamplerDescriptor {
            mag_filter: bevy::render::render_resource::FilterMode::Linear,
            min_filter: bevy::render::render_resource::FilterMode::Linear,
            ..default()
        });
        commands.insert_resource(SsrPipeline { layouts: [single, multi], vertex: fullscreen.to_vertex_state(), sampler, ids: HashMap::new() });
    }

    pub fn prepare(
        mut commands: Commands,
        cache: Res<PipelineCache>,
        mut pipe: ResMut<SsrPipeline>,
        views: Query<(Entity, &ExtractedView, Option<&Msaa>), With<SsrSettings>>,
    ) {
        for (e, view, msaa) in &views {
            let fmt = view.target_format;
            let ms = msaa.is_some_and(|m| m.samples() > 1);
            if env_on!("DXVIEW_SSRLOG") && LOGGED.load(std::sync::atomic::Ordering::Relaxed) < 3 {
                println!("SSR prepare: view {e:?} format {fmt:?}");
            }
            let id = if let Some(id) = pipe.ids.get(&(fmt, ms)) {
                *id
            } else {
                let desc = RenderPipelineDescriptor {
                    label: Some("dxview_ssr".into()),
                    layout: vec![pipe.layouts[ms as usize].clone()],
                    vertex: pipe.vertex.clone(),
                    fragment: Some(FragmentState {
                        shader: super::SSR_SHADER,
                        shader_defs: if ms { vec!["MULTISAMPLED".into()] } else { Vec::new() },
                        targets: vec![Some(ColorTargetState { format: fmt, blend: None, write_mask: ColorWrites::ALL })],
                        ..default()
                    }),
                    ..default()
                };
                let id = cache.queue_render_pipeline(desc);
                pipe.ids.insert((fmt, ms), id);
                id
            };
            commands.entity(e).insert(SsrPipelineId(id, ms));
        }
    }

    #[allow(clippy::type_complexity)]
    pub fn ssr(
        view: ViewQuery<(&ViewTarget, &SsrPipelineId, &ViewPrepassTextures, &ViewUniformOffset, &DynamicUniformIndex<SsrSettings>, Option<&Msaa>)>,
        pipe: Res<SsrPipeline>,
        cache: Res<PipelineCache>,
        view_uniforms: Res<ViewUniforms>,
        settings: Res<ComponentUniforms<SsrSettings>>,
        mut ctx: RenderContext,
    ) {
        let (target, id, prepass, view_offset, settings_index, msaa) = view.into_inner();
        let log = env_on!("DXVIEW_SSRLOG") && LOGGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 20;
        if log {
            println!(
                "SSR pass: msaa {:?} pipeline {} depth {} normal {}",
                msaa.map(|m| m.samples()),
                cache.get_render_pipeline(id.0).is_some(),
                prepass.depth_view().is_some(),
                prepass.normal_view().is_some()
            );
        }
        // The pipeline was made for the other sample count (MSAA switched this frame): next frame.
        if msaa.is_some_and(|m| m.samples() > 1) != id.1 {
            return;
        }
        let Some(pipeline) = cache.get_render_pipeline(id.0) else { return };
        let (Some(depth), Some(normal)) = (prepass.depth_view(), prepass.normal_view()) else { return };
        let (Some(view_binding), Some(settings_binding)) = (view_uniforms.uniforms.binding(), settings.uniforms().binding()) else { return };
        let post = target.post_process_write();
        let bind = ctx.render_device().create_bind_group(
            "dxview_ssr_bind",
            &cache.get_bind_group_layout(&pipe.layouts[id.1 as usize]),
            &BindGroupEntries::sequential((post.source, &pipe.sampler, depth, normal, view_binding, settings_binding)),
        );
        let desc = RenderPassDescriptor {
            label: Some("dxview_ssr"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: post.destination,
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
        pass.set_bind_group(0, &bind, &[view_offset.offset, settings_index.index()]);
        pass.draw(0..3, 0..1);
    }
}
