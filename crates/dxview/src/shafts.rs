//! Enhancements: Light Shafts and Local Fog (settings LightShafts / FogVolumes, ours, off by default). Both use
//! Bevy's volumetric fog on the world camera (ray-marched fog volumes lit by lights marked volumetric, with their
//! shadow maps, so walls and window frames cut the light into beams).
//!
//! - Light Shafts: a thin haze over the whole map, lit by the flashlight (its cone, or a beam-only light following
//!   it), by the map's Spotlight actors (the nearest few at a time, each a shadowed spot light that only shows in
//!   the air - the walls keep their baked light), and on open-air maps by the moon (a shadowed directional light from
//!   where the sky room's moon is, again only in the air): moonlight falling through windows and grates.
//! - Local Fog: low mist over water that isn't open to the sky (sewers, flooded tunnels, the ship's bilges) and a
//!   smoky haze where a bar's crowd sound plays (the Lucky Money, the Underworld, La Porte de l'Enfer), lit dimly
//!   all over and brightly by the lights above.

use crate::{FlyCam, LevelRes, SkyCam};
use bevy::light::{FogVolume, VolumetricFog, VolumetricLight};
use bevy::prelude::*;
use dx_level::METERS_PER_UNIT;
use glam::DVec3;
use unreal_pkg::props::{find, Value};

pub fn shafts_on() -> bool {
    crate::settings::get("LightShafts") > 0.5
}

pub fn fog_on() -> bool {
    crate::settings::get("FogVolumes") > 0.5
}

/// Entities this module made for the current map.
#[derive(Component)]
pub struct ShaftPart;

/// A pooled spot light standing in for one of the map's Spotlight actors.
#[derive(Component)]
pub struct ShaftSpot(usize);

/// The beam-only light following the flashlight when the Flashlight Cone enhancement is off.
#[derive(Component)]
pub struct ShaftBeam;

const SPOTS: usize = 6;

struct SpotDef {
    pos: Vec3,
    rot: Quat,
    color: Color,
    range: f32,
    angle: f32,
    intensity: f32,
}

#[derive(Default)]
pub struct ShaftState {
    built: (bool, bool),
    spots: Vec<SpotDef>,
    pick: f32,
}

fn byte(p: &[unreal_pkg::props::Property], name: &str, d: u8) -> u8 {
    match find(p, name) {
        Some(Value::Byte(b)) => *b,
        _ => d,
    }
}

fn spots_of(lvl: &dx_level::Level) -> Vec<SpotDef> {
    let mut out = Vec::new();
    for a in &lvl.actors {
        let effect = byte(&a.props, "LightEffect", 0);
        // LE_Searchlight, LE_StaticSpot, LE_Spotlight.
        if ![4u8, 8, 12].contains(&effect) || byte(&a.props, "LightType", 1) == 0 {
            continue;
        }
        let rot = match find(&a.props, "Rotation") {
            Some(Value::Rotator(r)) => *r,
            _ => [0; 3],
        };
        let m = dx_level::actors::rotator_to_bevy_mat3(rot);
        let fwd = Vec3::from(m[0]).normalize_or(Vec3::NEG_Y);
        let pos = Vec3::from(dx_level::ue_to_bevy(a.location()));
        let c = dx_level::lighting::hsv_white(byte(&a.props, "LightHue", 0), byte(&a.props, "LightSaturation", 255));
        let b = byte(&a.props, "LightBrightness", 64) as f32 / 255.0;
        let r_m = 25.0 * (byte(&a.props, "LightRadius", 64) as f32 + 1.0) * METERS_PER_UNIT;
        let cone = byte(&a.props, "LightCone", 128) as f32 / 256.0;
        let k = (1.5 * b).min(1.0).powf(2.2);
        out.push(SpotDef {
            pos,
            rot: Transform::from_translation(pos).looking_to(fwd, if fwd.y.abs() > 0.95 { Vec3::X } else { Vec3::Y }).rotation,
            color: Color::srgb(c[0], c[1], c[2]),
            range: r_m,
            angle: (cone * std::f32::consts::FRAC_PI_2).clamp(0.08, 1.2),
            intensity: std::f32::consts::PI.powi(2) * r_m * r_m * k * 0.6,
        });
    }
    out
}

/// The moon's direction from the sky room's viewpoint (Bevy space), when the sky shows one.
fn moon_dir(lvl: &dx_level::Level) -> Option<Vec3> {
    let o = Vec3::from(lvl.sky_origin?);
    let b = lvl.batches.iter().find(|b| {
        b.sky && b.texture.is_some_and(|t| lvl.textures[t].name.to_ascii_lowercase().contains("moon"))
    })?;
    let c = b.positions.iter().fold(Vec3::ZERO, |a, p| a + Vec3::from(*p)) / b.positions.len().max(1) as f32;
    (c - o).try_normalize()
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn shafts_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    cam: Single<(Entity, &Transform, Has<VolumetricFog>), (With<FlyCam>, Without<SkyCam>)>,
    parts: Query<Entity, With<ShaftPart>>,
    mut spots: Query<(&ShaftSpot, &mut SpotLight, &mut Transform, &mut Visibility), (Without<FlyCam>, Without<ShaftBeam>, Without<crate::dynlight::FlashCone>)>,
    mut beam: Query<(&mut SpotLight, &mut Transform, &mut Visibility), (With<ShaftBeam>, Without<FlyCam>, Without<ShaftSpot>, Without<crate::dynlight::FlashCone>)>,
    cone: Query<(Entity, &SpotLight, &Transform, &Visibility, Has<VolumetricLight>), (With<crate::dynlight::FlashCone>, Without<ShaftBeam>, Without<ShaftSpot>, Without<FlyCam>)>,
    mut st: Local<ShaftState>,
    mut map_gen: Local<u64>,
) {
    let lvl = &level.0;
    let fresh = crate::loader::new_map(&mut map_gen);
    let want = (shafts_on(), fog_on());
    let (cam_e, cam_tf, has_fog) = *cam;
    if fresh || want != st.built {
        st.built = want;
        for e in &parts {
            commands.entity(e).despawn();
        }
        st.spots.clear();
        let any = want.0 || want.1;
        if any && !has_fog {
            commands.entity(cam_e).insert(VolumetricFog {
                ambient_color: Color::srgb(0.55, 0.6, 0.7),
                // Local fog glows dimly all over (the bars' and sewers' light, baked into the walls, isn't in the air).
                ambient_intensity: if want.1 { 0.05 } else { 0.0 },
                jitter: 0.0,
                step_count: 48,
            });
        } else if !any && has_fog {
            commands.entity(cam_e).remove::<VolumetricFog>();
        }
        if !any {
            return;
        }
        let (bmin, bmax) = (Vec3::from(lvl.bounds_min), Vec3::from(lvl.bounds_max));
        let outdoors = lvl.zone_sky.iter().any(|s| *s);
        if want.0 && bmin.x < bmax.x {
            // The haze the beams show in.
            commands.spawn((
                FogVolume {
                    density_factor: 0.004,
                    absorption: 0.02,
                    scattering: 0.6,
                    scattering_asymmetry: 0.6,
                    ..default()
                },
                Transform::from_translation((bmin + bmax) * 0.5).with_scale((bmax - bmin).max(Vec3::splat(1.0))),
                ShaftPart,
            ));
            // The moon, in the air only.
            if let Some(d) = moon_dir(lvl).filter(|_| outdoors) {
                commands.spawn((
                    DirectionalLight {
                        color: Color::srgb(0.75, 0.82, 1.0),
                        illuminance: 0.35,
                        shadow_maps_enabled: true,
                        affects_lightmapped_mesh_diffuse: false,
                        ..default()
                    },
                    Transform::default().looking_to(-d, Vec3::Y),
                    VolumetricLight,
                    ShaftPart,
                ));
            }
            st.spots = spots_of(lvl);
            for i in 0..SPOTS.min(st.spots.len()) {
                commands.spawn((
                    SpotLight { intensity: 0.0, shadow_maps_enabled: true, affects_lightmapped_mesh_diffuse: false, ..default() },
                    Transform::default(),
                    Visibility::Hidden,
                    VolumetricLight,
                    ShaftSpot(i),
                    ShaftPart,
                ));
            }
            commands.spawn((
                SpotLight { intensity: 0.0, shadow_maps_enabled: true, affects_lightmapped_mesh_diffuse: false, ..default() },
                Transform::default(),
                Visibility::Hidden,
                VolumetricLight,
                ShaftBeam,
                ShaftPart,
            ));
        }
        if want.1 {
            spawn_local_fog(&mut commands, lvl);
        }
        return;
    }
    if !want.0 {
        return;
    }
    // The nearest spotlights in front, re-picked twice a second.
    st.pick -= time.delta_secs();
    if st.pick <= 0.0 && !st.spots.is_empty() {
        st.pick = 0.5;
        let eye = cam_tf.translation;
        let mut order: Vec<(f32, usize)> = st
            .spots
            .iter()
            .enumerate()
            .map(|(i, s)| (s.pos.distance(eye) - s.range, i))
            .filter(|(d, _)| *d < 40.0)
            .collect();
        order.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (slot, mut sl, mut tf, mut vis) in &mut spots {
            match order.get(slot.0) {
                Some(&(_, i)) => {
                    let s = &st.spots[i];
                    sl.intensity = s.intensity;
                    sl.color = s.color;
                    sl.range = s.range;
                    sl.outer_angle = s.angle;
                    sl.inner_angle = s.angle * 0.6;
                    *tf = Transform::from_translation(s.pos).with_rotation(s.rot);
                    *vis = Visibility::Inherited;
                }
                None => {
                    sl.intensity = 0.0;
                    *vis = Visibility::Hidden;
                }
            }
        }
    }
    // The flashlight: its cone carries the beam itself when it's on; otherwise a beam-only light follows the view.
    let mut cone_lit = false;
    for (e, sl, tf, vis, vol) in &cone {
        let lit = sl.intensity > 0.0 && *vis != Visibility::Hidden;
        cone_lit |= lit;
        if !vol {
            commands.entity(e).insert(VolumetricLight);
        }
        let _ = tf;
    }
    let aug = crate::abilities::aug_active("AugLight");
    for (mut sl, mut tf, mut vis) in &mut beam {
        if aug && !cone_lit {
            let f = cam_tf.forward().as_vec3();
            let pos = cam_tf.translation + cam_tf.right().as_vec3() * 0.19 - Vec3::Y * 0.11;
            *tf = Transform::from_translation(pos).looking_to(f, Vec3::Y);
            sl.intensity = 9000.0;
            sl.color = {
                let c = dx_level::lighting::hsv_white(32, 140);
                Color::srgb(c[0], c[1], c[2])
            };
            sl.range = 1024.0 * METERS_PER_UNIT;
            sl.outer_angle = 0.42;
            sl.inner_angle = 0.14;
            *vis = Visibility::Inherited;
        } else if *vis != Visibility::Hidden {
            sl.intensity = 0.0;
            *vis = Visibility::Hidden;
        }
    }
}

/// Mist over sheltered water and haze in bars.
fn spawn_local_fog(commands: &mut Commands, lvl: &dx_level::Level) {
    let sky_at = |p: Vec3| {
        let u = dx_level::bevy_to_ue(p.to_array());
        let z = lvl.collision.zone_at(DVec3::new(u[0] as f64, u[1] as f64, u[2] as f64));
        z > 0 && lvl.zone_sky.get(z as usize).copied().unwrap_or(false)
    };
    let mut n = 0;
    for b in lvl.batches.iter().filter(|b| b.class == dx_level::SurfaceClass::Water && !b.sky) {
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in &b.positions {
            lo = lo.min(Vec3::from(*p));
            hi = hi.max(Vec3::from(*p));
        }
        if hi.y - lo.y > 0.1 || lo.x > hi.x {
            continue;
        }
        let mid = (lo + hi) * 0.5 + Vec3::Y * 0.5;
        if sky_at(mid) {
            continue;
        }
        let size = Vec3::new(hi.x - lo.x + 3.0, 1.8, hi.z - lo.z + 3.0);
        commands.spawn((
            FogVolume {
                fog_color: Color::srgb(0.75, 0.8, 0.72),
                density_factor: 0.25,
                absorption: 0.05,
                scattering: 0.7,
                scattering_asymmetry: 0.3,
                light_intensity: 1.5,
                ..default()
            },
            Transform::from_translation(Vec3::new(mid.x, lo.y + 0.7, mid.z)).with_scale(size),
            ShaftPart,
        ));
        n += 1;
    }
    for s in lvl.ambient_sounds.iter().filter(|s| s.sound.to_ascii_lowercase().ends_with(".bar")) {
        let p = Vec3::from(dx_level::ue_to_bevy([s.pos[0] as f32, s.pos[1] as f32, s.pos[2] as f32]));
        let r = (s.radius as f32 * 25.0 * METERS_PER_UNIT).clamp(8.0, 22.0);
        commands.spawn((
            FogVolume {
                fog_color: Color::srgb(0.85, 0.8, 0.72),
                density_factor: 0.08,
                absorption: 0.03,
                scattering: 0.7,
                scattering_asymmetry: 0.5,
                light_intensity: 1.5,
                ..default()
            },
            Transform::from_translation(p + Vec3::Y * 1.0).with_scale(Vec3::new(r * 2.0, 5.0, r * 2.0)),
            ShaftPart,
        ));
        n += 1;
    }
    if std::env::var_os("DXVIEW_SHAFTLOG").is_some() {
        println!("SHAFTS local fog volumes {n}");
    }
}
