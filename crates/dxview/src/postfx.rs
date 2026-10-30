//! Enhancements: Ambient Occlusion and Soft Shadows (settings AmbientOcclusion / SoftShadows, ours, off by default).
//!
//! - Ambient Occlusion: Bevy's screen-space ambient occlusion on the world camera (depth and normal prepasses);
//!   bsp_shadow.wgsl darkens the BSP's lightmap by it (Bevy only applies it to ambient light, and the world's light
//!   is all lightmap), so corners, grates and the ground under crates and furniture darken. It needs MSAA off on
//!   every camera drawing into the window; SMAA on the last one smooths the edges instead.
//! - Soft Shadows: the real-time shadow lights (shadowlight.rs), the flashlight's cone and the light-shaft spots get
//!   a light size, so their shadows are filtered with percentage-closer soft shadows: sharp where the caster meets
//!   the ground, blurring as the shadow stretches away (contact hardening).

use bevy::anti_alias::smaa::Smaa;
use bevy::pbr::ScreenSpaceAmbientOcclusion;
use bevy::prelude::*;
use bevy::render::view::Msaa;

pub fn ao_on() -> bool {
    crate::settings::get("AmbientOcclusion") > 0.5
}

pub fn soft_on() -> bool {
    crate::settings::get("SoftShadows") > 0.5
}

/// Size of the lights for soft shadows (metres): a lamp's bulb and shade, more or less.
const POINT_SIZE: f32 = 0.12;
const SPOT_SIZE: f32 = 0.08;

#[allow(clippy::type_complexity)]
pub fn postfx_system(
    mut commands: Commands,
    world: Query<(Entity, Has<ScreenSpaceAmbientOcclusion>), (With<crate::FlyCam>, Without<crate::SkyCam>)>,
    cams: Query<(Entity, &Msaa, Has<Smaa>, Has<crate::weapons::ViewModelCam>), Or<(With<crate::FlyCam>, With<crate::SkyCam>, With<crate::weapons::ViewModelCam>, With<crate::augdisplay::HeatCam>)>>,
    mut points: Query<&mut PointLight, With<crate::shadowlight::ShadowLight>>,
    mut spots: Query<&mut SpotLight>,
) {
    let ao = ao_on();
    for (e, has) in &world {
        if ao && !has {
            commands.entity(e).insert(ScreenSpaceAmbientOcclusion {
                quality_level: bevy::pbr::ScreenSpaceAmbientOcclusionQualityLevel::Medium,
                constant_object_thickness: 0.3,
            });
        } else if !ao && has {
            commands.entity(e).remove::<ScreenSpaceAmbientOcclusion>();
        }
    }
    let want_msaa = if ao { Msaa::Off } else { Msaa::Sample4 };
    for (e, msaa, smaa, last) in &cams {
        if *msaa != want_msaa {
            commands.entity(e).insert(want_msaa);
        }
        if last && ao && !smaa {
            commands.entity(e).insert(Smaa::default());
        } else if smaa && !ao {
            commands.entity(e).remove::<Smaa>();
        }
    }
    let soft = soft_on() && crate::shadowlight::level() > 0;
    for mut p in &mut points {
        if p.soft_shadows_enabled != soft {
            p.soft_shadows_enabled = soft;
            p.radius = if soft { POINT_SIZE } else { 0.0 };
        }
    }
    for mut s in &mut spots {
        let want = soft_on() && s.shadow_maps_enabled;
        if s.soft_shadows_enabled != want {
            s.soft_shadows_enabled = want;
            s.radius = if want { SPOT_SIZE } else { 0.0 };
        }
    }
}
