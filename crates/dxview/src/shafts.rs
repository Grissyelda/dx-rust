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

/// Spotlights shown as beams at once (each one renders the scene into its own shadow map).
const SPOTS: usize = 3;

/// The shaft lights are made this many times weaker and the fog this many times more responsive to them: they
/// exist to light the air, and at full strength their specular highlights drew bright second pools on the floor
/// (the flashlight's "double" pool, spotlights' hot spots) even with affects_lightmapped_mesh_diffuse off.
const AIR_BOOST: f32 = 1000.0;

/// The haze the beams show in follows the eye: a box this big (metres) around it, soft at its edges.
const HAZE: Vec3 = Vec3::new(70.0, 36.0, 70.0);

/// The moon's shaft light: only while the eye is indoors (moonlight through windows); outdoors it would light the
/// whole haze into a grey veil over everything.
#[derive(Component)]
pub struct MoonShaft;

/// Marks the haze box (moved with the camera).
#[derive(Component)]
pub struct HazeBox;

/// A fog box whose density is recomputed a few times a second from wind-blown noise in world space (`roll`):
/// what makes the mist roll and billow instead of sitting there as a fixed pattern. `kind` 0 = mist over water
/// (`base` its surface height), 1 = smoke under a bar's ceiling, 2 = the light-shaft haze around the eye.
#[derive(Component)]
pub struct DriftFog {
    kind: u8,
    image: Handle<Image>,
    n: UVec3,
    base: f32,
}

/// Value noise (hash lattice), smooth, unbounded.
fn wnoise(p: Vec3) -> f32 {
    let h = |i: i32, j: i32, k: i32| {
        let n = (i.wrapping_mul(73_856_093)) ^ (j.wrapping_mul(19_349_663)) ^ (k.wrapping_mul(83_492_791));
        ((n as u32).wrapping_mul(2_654_435_761) >> 8) as f32 / (1u32 << 24) as f32
    };
    let (i, j, k) = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (fx, fy, fz) = (s(p.x - p.x.floor()), s(p.y - p.y.floor()), s(p.z - p.z.floor()));
    let l = |a: f32, b: f32, t: f32| a + (b - a) * t;
    l(
        l(l(h(i, j, k), h(i + 1, j, k), fx), l(h(i, j + 1, k), h(i + 1, j + 1, k), fx), fy),
        l(l(h(i, j, k + 1), h(i + 1, j, k + 1), fx), l(h(i, j + 1, k + 1), h(i + 1, j + 1, k + 1), fx), fy),
        fz,
    )
}

fn fbm(p: Vec3) -> f32 {
    wnoise(p) * 0.55 + wnoise(p * 2.03 + Vec3::splat(17.3)) * 0.3 + wnoise(p * 4.07 + Vec3::splat(41.9)) * 0.15
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The fog's density at world point `w` (box coordinates `u` 0..1) at time `t`.
fn roll(kind: u8, w: Vec3, u: Vec3, base: f32, t: f32) -> f32 {
    match kind {
        0 => {
            // Mist over still water: banks that drift with the air, their tops rising and sinking.
            let wind = Vec3::new(0.32, 0.0, 0.19) * t;
            let n = fbm((w - wind) * 0.24 + Vec3::new(0.0, t * 0.04, 0.0));
            let swell = fbm((w - wind * 0.6) * 0.06 + Vec3::new(5.0, t * 0.025, 9.0));
            let top = 0.35 + 1.9 * swell;
            let h = w.y - base;
            let layer = (1.0 - smooth(top * 0.3, top, h)) * smooth(-0.25, 0.1, h);
            edge(u.x) * edge(u.z) * layer * smooth(0.28, 0.72, n) * 1.8
        }
        1 => {
            // Smoke under a ceiling, curling as it drifts.
            let drift = Vec3::new(0.12, -0.05, 0.08) * t;
            let n = fbm((w - drift) * 0.45 + Vec3::new(0.0, t * 0.06, 0.0));
            edge(u.x) * edge(u.z) * edge(u.y) * (0.3 + 0.7 * u.y) * (0.2 + 1.6 * smooth(0.25, 0.8, n))
        }
        _ => {
            // The haze the beams show in: uneven, slowly moving, so a beam has texture.
            let wind = Vec3::new(0.25, 0.02, 0.15) * t;
            let n = fbm((w - wind) * 0.09);
            edge(u.x) * edge(u.y) * edge(u.z) * (0.35 + 1.3 * smooth(0.2, 0.8, n))
        }
    }
}

/// Recompute one fog box's density texture for time `t`.
fn refill(img: &mut Image, f: &DriftFog, tf: &Transform, t: f32) {
    let size = tf.scale;
    let min = tf.translation - size * 0.5;
    let n = f.n;
    let mut data = Vec::with_capacity((n.x * n.y * n.z * 4) as usize);
    for z in 0..n.z {
        for y in 0..n.y {
            for x in 0..n.x {
                let u = Vec3::new((x as f32 + 0.5) / n.x as f32, (y as f32 + 0.5) / n.y as f32, (z as f32 + 0.5) / n.z as f32);
                let w = min + u * size;
                data.extend_from_slice(&roll(f.kind, w, u, f.base, t).max(0.0).to_le_bytes());
            }
        }
    }
    img.data = Some(data);
}

/// A blank density texture for a rolling fog box (filled by `refill`), clamped (the noise is in world space).
fn fog_image(images: &mut Assets<Image>, n: UVec3) -> Handle<Image> {
    let mut img = density_image((n.x, n.y, n.z), |_, _, _| 0.0);
    img.sampler = bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
        address_mode_u: bevy::image::ImageAddressMode::ClampToEdge,
        address_mode_v: bevy::image::ImageAddressMode::ClampToEdge,
        address_mode_w: bevy::image::ImageAddressMode::ClampToEdge,
        mag_filter: bevy::image::ImageFilterMode::Linear,
        min_filter: bevy::image::ImageFilterMode::Linear,
        ..default()
    });
    images.add(img)
}

/// A small 3D density texture: `f(x, y, z)` over the unit cube (0..1 each), as single-channel floats.
fn density_image(n: (u32, u32, u32), f: impl Fn(f32, f32, f32) -> f32) -> Image {
    let mut data = Vec::with_capacity((n.0 * n.1 * n.2 * 4) as usize);
    for z in 0..n.2 {
        for y in 0..n.1 {
            for x in 0..n.0 {
                let v = f((x as f32 + 0.5) / n.0 as f32, (y as f32 + 0.5) / n.1 as f32, (z as f32 + 0.5) / n.2 as f32);
                data.extend_from_slice(&v.max(0.0).to_le_bytes());
            }
        }
    }
    let mut img = Image::new(
        bevy::render::render_resource::Extent3d { width: n.0, height: n.1, depth_or_array_layers: n.2 },
        bevy::render::render_resource::TextureDimension::D3,
        data,
        bevy::render::render_resource::TextureFormat::R32Float,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    img.sampler = bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
        address_mode_u: bevy::image::ImageAddressMode::Repeat,
        address_mode_v: bevy::image::ImageAddressMode::ClampToEdge,
        address_mode_w: bevy::image::ImageAddressMode::Repeat,
        mag_filter: bevy::image::ImageFilterMode::Linear,
        min_filter: bevy::image::ImageFilterMode::Linear,
        ..default()
    });
    img
}

/// 0 at the box's faces rising to 1 a quarter of the way in: no hard edge where a box meets the walls and floor.
fn edge(t: f32) -> f32 {
    let a = (t / 0.25).clamp(0.0, 1.0);
    let b = ((1.0 - t) / 0.25).clamp(0.0, 1.0);
    a * a * (3.0 - 2.0 * a) * b * b * (3.0 - 2.0 * b)
}


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
    indoor_check: f32,
    moon_on: bool,
    spots_lit: usize,
    roll_next: usize,
    roll_all: bool,
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
pub fn moon_dir(lvl: &dx_level::Level) -> Option<Vec3> {
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
    mut haze: Query<(&mut Transform, &mut Visibility), (With<HazeBox>, Without<FlyCam>, Without<ShaftSpot>, Without<ShaftBeam>, Without<MoonShaft>, Without<crate::dynlight::FlashCone>)>,
    drift: Query<(&DriftFog, &GlobalTransform)>,
    mut moon: Query<&mut Visibility, (With<MoonShaft>, Without<ShaftSpot>, Without<ShaftBeam>, Without<crate::dynlight::FlashCone>)>,
    mut images: ResMut<Assets<Image>>,
    mut spots: Query<(&ShaftSpot, &mut SpotLight, &mut Transform, &mut Visibility), (Without<FlyCam>, Without<ShaftBeam>, Without<crate::dynlight::FlashCone>)>,
    mut beam: Query<(&mut SpotLight, &mut Transform, &mut Visibility), (With<ShaftBeam>, Without<FlyCam>, Without<ShaftSpot>, Without<crate::dynlight::FlashCone>)>,
    mut st: Local<ShaftState>,
    mut map_gen: Local<u64>,
) {
    let lvl = &level.0;
    let fresh = crate::loader::new_map(&mut map_gen);
    let want = (shafts_on(), fog_on());
    let (cam_e, cam_tf, has_fog) = *cam;
    if fresh || want != st.built {
        st.built = want;
        st.roll_all = true;
        for e in &parts {
            commands.entity(e).despawn();
        }
        st.spots.clear();
        let any = want.0 || want.1;
        if any && !has_fog {
            commands.entity(cam_e).insert(VolumetricFog {
                ambient_color: Color::srgb(0.55, 0.6, 0.7),
                // Local fog glows dimly all over (the bars' and sewers' light, baked into the walls, isn't in the air).
                ambient_intensity: 0.0,
                jitter: 0.0,
                step_count: 24,
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
            // The haze the beams show in: around the eye only (beams farther off are faint anyway), so the march
            // is short and the box's soft edges are never close.
            let hn = UVec3::new(20, 10, 20);
            let tex = fog_image(&mut images, hn);
            commands.spawn((
                FogVolume {
                    density_factor: 0.006,
                    density_texture: Some(tex.clone()),
                    absorption: 0.01,
                    scattering: 0.6,
                    scattering_asymmetry: 0.6,
                    light_intensity: AIR_BOOST,
                    ..default()
                },
                Transform::from_translation(cam_tf.translation).with_scale(HAZE),
                HazeBox,
                DriftFog { kind: 2, image: tex, n: hn, base: 0.0 },
                ShaftPart,
            ));
            // The moon, in the air only.
            if let Some(d) = moon_dir(lvl).filter(|_| outdoors) {
                commands.spawn((
                    DirectionalLight {
                        color: Color::srgb(0.75, 0.82, 1.0),
                        illuminance: 0.35 / AIR_BOOST,
                        shadow_maps_enabled: true,
                        affects_lightmapped_mesh_diffuse: false,
                        ..default()
                    },
                    // One short cascade: the beams only matter near the eye (each camera that sees the world renders
                    // its own cascade).
                    bevy::light::CascadeShadowConfigBuilder { num_cascades: 1, maximum_distance: 45.0, ..default() }.build(),
                    Transform::default().looking_to(-d, Vec3::Y),
                    Visibility::Hidden,
                    MoonShaft,
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
        let n_fog = if want.1 { spawn_local_fog(&mut commands, &mut images, lvl) } else { 0 };
        if !want.0 && n_fog == 0 {
            // Nothing to march through on this map.
            commands.entity(cam_e).remove::<VolumetricFog>();
        }
        if n_fog > 0 {
            // Local fog glows dimly all over (the light baked into the walls isn't in the air).
            commands.entity(cam_e).insert(VolumetricFog {
                ambient_color: Color::srgb(0.55, 0.6, 0.7),
                // (0.02 laid a grey veil over everything near, people included.)
                ambient_intensity: 0.012,
                jitter: 0.0,
                step_count: 24,
            });
        }
        return;
    }
    // The fog rolls: one box's density is recomputed each frame, in turn (each about 10-15 times a second).
    let t = time.elapsed_secs();
    let boxes: Vec<(&DriftFog, Transform)> = drift.iter().map(|(f, g)| (f, g.compute_transform())).collect();
    if !boxes.is_empty() {
        st.roll_next = (st.roll_next + 1) % boxes.len();
        // Every box once straight away after a build (they start empty).
        let all = std::mem::take(&mut st.roll_all);
        for (k, (f, tf)) in boxes.iter().enumerate() {
            if all || k == st.roll_next {
                if let Some(mut img) = images.get_mut(&f.image) {
                    refill(&mut img, f, tf, t);
                }
            }
        }
    }
    if !want.0 {
        return;
    }
    for (mut tf, _) in &mut haze {
        // In 2 m steps (no shimmer of the soft edge as the box slides).
        tf.translation = (cam_tf.translation / 2.0).round() * 2.0;
    }
    st.indoor_check -= time.delta_secs();
    if st.indoor_check <= 0.0 {
        st.indoor_check = 0.25;
        let u = dx_level::bevy_to_ue(cam_tf.translation.to_array());
        let z = lvl.collision.zone_at(DVec3::new(u[0] as f64, u[1] as f64, u[2] as f64));
        let indoors = z > 0 && !lvl.zone_sky.get(z as usize).copied().unwrap_or(true);
        st.moon_on = indoors && !moon.is_empty();
        let want_v = if indoors { Visibility::Inherited } else { Visibility::Hidden };
        for mut v in &mut moon {
            if *v != want_v {
                *v = want_v;
            }
        }
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
            .filter(|(d, i)| *d < 25.0 && (st.spots[*i].pos - eye).dot(*cam_tf.forward()) > -st.spots[*i].range)
            .collect();
        order.sort_by(|a, b| a.0.total_cmp(&b.0));
        st.spots_lit = order.len().min(SPOTS);
        for (slot, mut sl, mut tf, mut vis) in &mut spots {
            match order.get(slot.0) {
                Some(&(_, i)) => {
                    let s = &st.spots[i];
                    sl.intensity = s.intensity / AIR_BOOST;
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
    // The flashlight's beam in the air: a light of its own for the haze (the Flashlight Cone's light grows with the
    // distance it reaches, which in the haze blew out the screen), modest. It shines from where the flashlight does
    // and the same way (it lights surfaces too: anywhere else it made a second pool), a little ahead so the air right
    // at the lens doesn't flare - but never past half the way to the wall in front (ahead of a wall it lit what was
    // behind it, and through the double-sided wall the far side's shadows showed).
    let aug = crate::abilities::aug_active("AugLight");
    let flash = crate::dynlight::flashlight();
    for (mut sl, mut tf, mut vis) in &mut beam {
        if let (true, Some((from, f, reach))) = (aug, flash) {
            let pos = from + f * (reach * 0.5).min(0.6);
            *tf = Transform::from_translation(pos).looking_to(f, Vec3::Y);
            sl.intensity = 14.0 / AIR_BOOST;
            sl.color = {
                let c = dx_level::lighting::hsv_white(32, 140);
                Color::srgb(c[0], c[1], c[2])
            };
            sl.range = 1024.0 * METERS_PER_UNIT;
            sl.outer_angle = 0.32;
            sl.inner_angle = 0.0;
            *vis = Visibility::Inherited;
        } else if *vis != Visibility::Hidden {
            sl.intensity = 0.0;
            *vis = Visibility::Hidden;
        }
    }
    // No beam anywhere near (no flashlight, spotlight or moon through a window): the haze isn't marched at all.
    let lit = aug || st.spots_lit > 0 || st.moon_on;
    let want_v = if lit { Visibility::Inherited } else { Visibility::Hidden };
    for (_, mut v) in &mut haze {
        if *v != want_v {
            *v = want_v;
        }
    }
}

/// Mist over sheltered water and haze in bars. Water is grouped by 6 m cells (only water whose face centres are in
/// zones without sky), each connected group gets its own box; every box has a soft-edged density texture (mist: dense
/// at the water, thinning upward, with wisps that drift) so it never shows a hard line where it meets things.
fn spawn_local_fog(commands: &mut Commands, images: &mut Assets<Image>, lvl: &dx_level::Level) -> usize {
    let sky_at = |p: Vec3| {
        let u = dx_level::bevy_to_ue(p.to_array());
        let z = lvl.collision.zone_at(DVec3::new(u[0] as f64, u[1] as f64, u[2] as f64));
        z <= 0 || lvl.zone_sky.get(z as usize).copied().unwrap_or(false)
    };
    const CELL: f32 = 6.0;
    // (cell x, cell z, height in cm) -> present
    let mut cells: std::collections::HashMap<(i32, i32, i32), ()> = Default::default();
    for b in lvl.batches.iter().filter(|b| b.class == dx_level::SurfaceClass::Water && !b.sky) {
        for t in b.indices.chunks_exact(3) {
            let p: Vec<Vec3> = t.iter().map(|&i| Vec3::from(b.positions[i as usize])).collect();
            let n = (p[1] - p[0]).cross(p[2] - p[0]);
            if n.length() < 1e-4 || n.normalize().y.abs() < 0.9 {
                continue;
            }
            let c = (p[0] + p[1] + p[2]) / 3.0;
            if sky_at(c + Vec3::Y * 0.5) {
                continue;
            }
            // Every cell the face covers (by its bounds).
            let lo = p[0].min(p[1]).min(p[2]);
            let hi = p[0].max(p[1]).max(p[2]);
            for cx in (lo.x / CELL).floor() as i32..=(hi.x / CELL).floor() as i32 {
                for cz in (lo.z / CELL).floor() as i32..=(hi.z / CELL).floor() as i32 {
                    cells.insert((cx, cz, (c.y * 100.0).round() as i32), ());
                }
            }
        }
    }
    // Connected groups (4-neighbours at the same height).
    let mut left: std::collections::HashSet<(i32, i32, i32)> = cells.keys().copied().collect();
    let mut groups: Vec<(IVec2, IVec2, f32)> = Vec::new();
    while let Some(&start) = left.iter().next() {
        left.remove(&start);
        let mut stack = vec![start];
        let (mut lo, mut hi) = (IVec2::new(start.0, start.1), IVec2::new(start.0, start.1));
        while let Some((x, z, h)) = stack.pop() {
            lo = lo.min(IVec2::new(x, z));
            hi = hi.max(IVec2::new(x, z));
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let k = (x + dx, z + dz, h);
                if left.remove(&k) {
                    stack.push(k);
                }
            }
        }
        groups.push((lo, hi, start.2 as f32 / 100.0));
    }
    let mut n = 0;
    for (lo, hi, y) in groups {
        let min = Vec2::new(lo.x as f32 * CELL, lo.y as f32 * CELL);
        let max = Vec2::new((hi.x + 1) as f32 * CELL, (hi.y + 1) as f32 * CELL);
        let size = Vec3::new(max.x - min.x + 2.0, 3.0, max.y - min.y + 2.0);
        let mid = Vec3::new((min.x + max.x) * 0.5, y - 0.3 + size.y * 0.5, (min.y + max.y) * 0.5);
        // About a texel every 1.2 m across (at most 40), 12 up.
        let res = UVec3::new(((size.x / 1.2) as u32).clamp(6, 40), 12, ((size.z / 1.2) as u32).clamp(6, 40));
        let tex = fog_image(images, res);
        if std::env::var_os("DXVIEW_SHAFTLOG").is_some() {
            println!("SHAFTS mist at {mid:?} size {size:?}");
        }
        commands.spawn((
            FogVolume {
                fog_color: Color::srgb(0.7, 0.76, 0.74),
                density_factor: 0.11,
                density_texture: Some(tex.clone()),
                absorption: 0.03,
                scattering: 0.5,
                scattering_asymmetry: 0.35,
                light_intensity: AIR_BOOST,
                ..default()
            },
            Transform::from_translation(mid).with_scale(size),
            DriftFog { kind: 0, image: tex, n: res, base: y },
            ShaftPart,
        ));
        n += 1;
    }
    for s in lvl.ambient_sounds.iter().filter(|s| s.sound.to_ascii_lowercase().ends_with(".bar")) {
        let p = Vec3::from(dx_level::ue_to_bevy([s.pos[0] as f32, s.pos[1] as f32, s.pos[2] as f32]));
        let r = (s.radius as f32 * 25.0 * METERS_PER_UNIT).clamp(8.0, 20.0);
        let res = UVec3::new(20, 10, 20);
        let tex = fog_image(images, res);
        commands.spawn((
            FogVolume {
                fog_color: Color::srgb(0.8, 0.76, 0.7),
                density_factor: 0.035,
                density_texture: Some(tex.clone()),
                absorption: 0.02,
                scattering: 0.6,
                scattering_asymmetry: 0.5,
                light_intensity: AIR_BOOST,
                ..default()
            },
            Transform::from_translation(p + Vec3::Y * 1.0).with_scale(Vec3::new(r * 2.0, 5.0, r * 2.0)),
            DriftFog { kind: 1, image: tex, n: res, base: 0.0 },
            ShaftPart,
        ));
        n += 1;
    }
    if std::env::var_os("DXVIEW_SHAFTLOG").is_some() {
        println!("SHAFTS local fog volumes {n}");
    }
    n
}
