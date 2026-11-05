//! Enhancements: Ambient Occlusion and Soft Shadows (settings AmbientOcclusion / SoftShadows, ours, off by default).
//!
//! - Ambient Occlusion: Bevy's screen-space ambient occlusion on the world camera (depth and normal prepasses);
//!   bsp_shadow.wgsl darkens the BSP's lightmap by it (Bevy only applies it to ambient light, and the world's light
//!   is all lightmap), so corners, grates and the ground under crates and furniture darken. It needs MSAA off on
//!   every camera drawing into the window; SMAA on the last one smooths the edges instead.
//! - Soft Shadows: the real-time shadow lights (shadowlight.rs) get
//!   a light size, so their shadows are filtered with percentage-closer soft shadows: sharp where the caster meets
//!   the ground, blurring as the shadow stretches away (contact hardening).

use bevy::anti_alias::smaa::Smaa;
use bevy::camera::MsaaWriteback;
use bevy::light::VolumetricFog;
use bevy::pbr::ScreenSpaceAmbientOcclusion;
use bevy::prelude::*;
use bevy::render::view::Msaa;

pub fn ao_on() -> bool {
    setting!("AmbientOcclusion") > 0.5
}

pub fn soft_on() -> bool {
    setting!("SoftShadows") > 0.5
}

/// Size of the lights for soft shadows (metres): a lamp's bulb and shade, more or less.
const POINT_SIZE: f32 = 0.12;

#[allow(clippy::type_complexity)]
pub fn postfx_system(
    mut commands: Commands,
    world: Query<(Entity, Has<ScreenSpaceAmbientOcclusion>), (With<crate::FlyCam>, Without<crate::SkyCam>)>,
    mut cams: Query<
        (Entity, &Msaa, Has<Smaa>, Has<crate::weapons::ViewModelCam>, &mut Camera, Has<VolumetricFog>),
        Or<(
            With<crate::FlyCam>,
            With<crate::SkyCam>,
            With<crate::weapons::ViewModelCam>,
            With<crate::augdisplay::HeatCam>,
            With<crate::upscale::UpscaleCam>,
        )>,
    >,
    mut points: Query<(&mut SpotLight, &crate::shadowlight::ShadowLight), Without<crate::dynlight::FlashCone>>,
    mut spots: Query<&mut SpotLight, (With<crate::dynlight::FlashCone>, Without<crate::shadowlight::ShadowLight>)>,
    mut bsp: ResMut<Assets<crate::shadowlight::BspMaterial>>,
    // The mesh actors, whose shadow lights' walk goes with the BSP's (shadowlight.rs `actor_spot_shadows`).
    (mut anim, mut actors): (
        ResMut<Assets<crate::gpuanim::AnimMaterial>>,
        ResMut<Assets<crate::skeletal::ActorMaterial>>,
    ),
    mut ao_gen: Local<Option<(bool, u32, bool)>>,
) {
    // The cameras' make-up never changes in play: the SSAO pass, the depth / normal prepasses, MSAA off with SMAA.
    // Each of those is part of every material's pipeline, so adding or removing one recompiled them all (a long
    // stall on the Classic / Modern switch) and, removed in the wrong order, crashed Bevy's SSAO (a camera with
    // SSAO but no depth prepass for a frame). The Ambient Occlusion setting is now just how strongly the world's
    // shader uses it.
    let ao = ao_on();
    // The BSP shader's shadow strength goes with it: 0 (Lean Rendering with no shadow lights) skips its walk over
    // the lights in each fragment's cluster.
    let strength = crate::shadowlight::shadow_strength();
    let detail = setting!("DetailTextures") > 0.5;
    if *ao_gen != Some((ao, strength.to_bits(), detail)) {
        *ao_gen = Some((ao, strength.to_bits(), detail));
        let ids: Vec<_> = bsp.ids().collect();
        for id in ids {
            if let Some(mut m) = bsp.get_mut(id) {
                m.extension.params.q.x = if ao { 0.85 } else { 0.0 };
                m.extension.params.p.x = strength;
                m.extension.params.q.z = if detail { 1.0 } else { 0.0 };
            }
        }
        // Only the materials whose walk changes: a material taken mutably is prepared again.
        let walk = crate::shadowlight::actor_spot_shadows();
        let ids: Vec<_> = anim.iter().filter(|(_, m)| m.extension.spot_shadows != walk).map(|(id, _)| id).collect();
        for id in ids {
            if let Some(mut m) = anim.get_mut(id) {
                m.extension.spot_shadows = walk;
            }
        }
        let ids: Vec<_> = actors.iter().filter(|(_, m)| m.extension.spot_shadows != walk).map(|(id, _)| id).collect();
        for id in ids {
            if let Some(mut m) = actors.get_mut(id) {
                m.extension.spot_shadows = walk;
            }
        }
    }
    // The SSAO pass costs GPU time, so it's only there when Ambient Occlusion is on in either look (now, or in
    // the Modern mix Classic / Modern remembers); once attached it stays for the session.
    static ATTACHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let wanted = ao || setting!("Modern.AmbientOcclusion") > 0.5;
    if wanted {
        ATTACHED.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    let ssao = ATTACHED.load(std::sync::atomic::Ordering::Relaxed) && !env_on!("DXVIEW_AO_NOSSAO");
    for (e, has) in &world {
        if ssao && !has {
            // Low: one slice of two samples, denoised. Medium was the single most expensive GPU pass of a Modern frame
            // (more than drawing the level), for a difference hard to see under Unreal's baked light.
            commands.entity(e).insert(ScreenSpaceAmbientOcclusion {
                quality_level: bevy::pbr::ScreenSpaceAmbientOcclusionQualityLevel::Low,
                constant_object_thickness: 0.3,
            });
        } else if !ssao && has {
            commands.entity(e).remove::<ScreenSpaceAmbientOcclusion>();
        }
    }
    // Anti-Aliasing off (perf.rs) leaves the edges alone; SSAO's SMAA still comes with Ambient Occlusion, and
    // Upscaling brings its own antialiasing (TAA).
    let aa_off = ssao || crate::upscale::mode() > 0;
    let want_msaa = if aa_off || !crate::perf::msaa() { Msaa::Off } else { Msaa::Sample4 };
    // Lean Rendering: the cameras drawing into the window share one multisampled picture. Nothing writes the
    // resolved picture between cameras except volumetric fog, so without it the multisampled one already holds
    // everything and the MSAA writebacks (a full-screen copy at the start of each camera after the first) are left
    // out.
    let writeback = if crate::perf::lean_rendering() && !cams.iter().any(|c| c.5) { MsaaWriteback::Off } else { MsaaWriteback::Auto };
    for (e, msaa, smaa, last, mut cam, _) in &mut cams {
        if *msaa != want_msaa {
            commands.entity(e).insert(want_msaa);
        }
        if last && aa_off && !smaa {
            // Medium preset: shorter edge searches than High, about two thirds of its cost.
            commands.entity(e).insert(Smaa { preset: bevy::anti_alias::smaa::SmaaPreset::Medium });
        } else if smaa && !aa_off {
            commands.entity(e).remove::<Smaa>();
        }
        if cam.msaa_writeback != writeback {
            cam.msaa_writeback = writeback;
        }
    }
    let soft = soft_on() && crate::shadowlight::level() > 0;
    // Soft shadows cost many shadow-map taps per pixel per light: the two strongest shadow lights get them.
    for (mut p, sl) in &mut points {
        let soft = soft && sl.slot() < 2;
        if p.soft_shadows_enabled != soft {
            p.soft_shadows_enabled = soft;
            p.radius = if soft { POINT_SIZE } else { 0.0 };
        }
    }
    // The flashlight's cone shines from next to the eye, where its shadows are thin edges: PCSS there only added
    // speckled noise on grazing walls. Its shadows stay hard (and the light-shaft spots only light the air).
    for mut s in &mut spots {
        let want = false;
        if s.soft_shadows_enabled != want {
            s.soft_shadows_enabled = want;
            s.radius = 0.0;
        }
    }
}
