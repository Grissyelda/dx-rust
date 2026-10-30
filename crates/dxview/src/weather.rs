//! Rain, from the Precipitation mod (Smoke39; the version GMDX: Augmented Edition ships): Precipitator spawns
//! Raindrops (the Tracer mesh with Precipitation.Skins.Raindrop, translucent) in a radius around the player,
//! PrecipitationInfo.PrecipRad 650 / PrecipFreq 0.01 / PrecipDensity drops a batch, 600 units up; each falls at
//! 900..1700 plus the wind, splashes two WtrDrpSmall droplets where it lands (0.4 s, gravity, eject 35) and a
//! RaindropRing on water; RainCity loops around the player (two FollowingSounds) and RainInside under shelter;
//! footsteps outside get a WaterStep splash (RainStep). GMDX's SetRainLevel makes it dynamic: the density drifts
//! towards a new goal picked every 150-400 s.
//!
//! Ours: the mod places PrecipitationInfo / shelter actors by hand in GMDX's own maps. The original maps have none,
//! so rain falls in every BSP zone that is open to the sky (`Level::zone_sky`), on the maps where it fits (`profile`):
//! New York, Hong Kong and the docks; light over Paris; dry in the deserts, the labs and the intro. The drops are
//! one streak mesh rebuilt each frame; shelter is how much of the sky is in sight around the player. A setting
//! (Weather: Off / Steady / Dynamic), used while the Precipitation package is in a mod folder.

use crate::weapons::{image_for, RuntimeAssets};
use crate::{FlyCam, LevelRes, Player, SkyCam};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use dx_level::ue_to_bevy;
use glam::DVec3;

/// How it rains on a map: the most it gets (GMDX precipMaxDensity is 14) and the wind.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Profile {
    pub max: f32,
    pub wind: DVec3,
}

/// The maps it rains on (by file name, without .dx). Everything else stays dry.
pub fn profile(map: &str) -> Option<Profile> {
    let m = map.to_ascii_lowercase();
    let m = m.trim_end_matches(".dx");
    let p = |max: f32, wx: f64, wy: f64| Some(Profile { max, wind: DVec3::new(wx, wy, 0.0) });
    // The intro, training, title, endings, the deserts, the labs, the sea: dry.
    if m.starts_with("00_")
        || m.starts_with("12_")
        || m.starts_with("14_")
        || m.starts_with("15_")
        || m.starts_with("99_")
        || m == "dx"
        || m == "dxonly"
        || m == "entry"
        || m.contains("mj12lab")
    {
        return None;
    }
    if m.starts_with("09_nyc_dockyard") || m.starts_with("09_nyc_ship") {
        return p(14.0, 120.0, 60.0);
    }
    if m.starts_with("01_") || m.starts_with("02_") || m.starts_with("03_") || m.starts_with("04_") || m.starts_with("05_")
        || m.starts_with("08_") || m.starts_with("09_")
    {
        return p(12.0, 40.0, 20.0);
    }
    if m.starts_with("06_hongkong") {
        return p(14.0, 20.0, -30.0);
    }
    if m.starts_with("10_paris") || m.starts_with("11_paris") {
        return p(5.0, 0.0, 30.0);
    }
    None
}

/// Setting "Weather": 0 off, 1 steady (GMDX iWeatherControl 1: density 7), 2 dynamic (the default).
fn mode() -> u8 {
    crate::settings::get("Weather").round() as u8
}

/// The Precipitation package is there to draw and hear the rain with.
pub fn available() -> bool {
    static FOUND: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FOUND.get_or_init(|| {
        unreal_pkg::paths::root().is_some_and(|r| unreal_pkg::paths::exists(&r.join("System").join("Precipitation.u")))
    })
}

struct Drop {
    pos: DVec3,
    vel: DVec3,
    /// Where it lands (BSP), if within its life; and whether it falls into water first.
    hit: Option<(DVec3, DVec3)>,
    life: f32,
    len: f32,
    glow: f32,
}

struct Splash {
    pos: DVec3,
    vel: DVec3,
    life: f32,
    max: f32,
}

/// The rain's own render layer: the world view draws it, mirror / water reflection cameras don't (a reflected drop
/// close to the water reads as a streak floating under the surface).
pub const RAIN_LAYER: usize = 9;

#[derive(Default)]
pub struct WeatherState {
    drops: Vec<Drop>,
    splashes: Vec<Splash>,
    mesh: Option<Handle<Mesh>>,
    splash_mesh: Option<Handle<Mesh>>,
    spawned: bool,
    profile: Option<Profile>,
    density: f32,
    desired: f32,
    next_change: f32,
    tick: f32,
    acc: f32,
    height: f32,
    seed: u64,
    /// How open to the sky the player is (0 under a roof .. 1 out in it), eased.
    exposure: f32,
    exposure_check: f32,
    sky_near: f32,
    /// How high rain can start over each 128-unit cell (the sky zone's ceiling above, or RAIN_TOP), by the player's
    /// height band.
    ceiling: std::collections::HashMap<(i32, i32, i32), f64>,
}

#[derive(Component)]
pub struct RainMesh;

/// Raining here and now, and how hard (PrecipDensity 0..14).
static DENSITY: std::sync::Mutex<f32> = std::sync::Mutex::new(0.0);
static EXPOSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn density() -> f32 {
    DENSITY.lock().map(|d| *d).unwrap_or(0.0)
}

/// PrecipitationInfo.RainStep: a footstep out in the rain (bSplashyFeet: density 4 and up) also splashes. Returns the
/// volume scale for the normal step sound (carpet 0.75, grass 0.5).
pub fn rain_step(group: Option<&str>, volume: f32, range: f32, pitch: f32) -> f32 {
    if density() < 4.0 || !EXPOSED.load(std::sync::atomic::Ordering::Relaxed) {
        return 1.0;
    }
    let n = (crate::sound::frand() * 3.0) as u32 % 3 + 1;
    let (splash, step) = match group.map(|g| g.to_ascii_lowercase()) {
        Some(g) if g == "textile" => (1.0, 0.75),
        Some(g) if g == "foliage" => (1.0, 0.5),
        _ => (0.75, 1.0),
    };
    crate::sound::play_player(&format!("DeusExSounds.Generic.WaterStep{n}"), splash * volume, range);
    let _ = pitch;
    step
}

fn rnd(seed: &mut u64) -> f64 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    ((*seed >> 11) as f64) / (1u64 << 53) as f64
}

#[allow(clippy::too_many_arguments)]
pub fn weather_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    player: Res<Player>,
    cam: Single<&Transform, (With<FlyCam>, Without<SkyCam>, Without<RainMesh>)>,
    mut st: Local<WeatherState>,
    mut map_gen: Local<u64>,
    (mut assets, mut images, mut meshes, mut materials): (
        ResMut<RuntimeAssets>,
        ResMut<Assets<Image>>,
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
    ),
    mut vis: Query<&mut Visibility, With<RainMesh>>,
) {
    let _prof = crate::prof::scope("weather::weather_system");
    let lvl = &level.0;
    if crate::loader::new_map(&mut map_gen) {
        let seed = st.seed;
        *st = WeatherState::default();
        st.seed = seed ^ 0x5EED_F00D_1234;
        st.profile = profile(&lvl.name);
    }
    let dt = time.delta_secs().min(0.1);
    let on = mode() > 0 && st.profile.is_some() && available() && !crate::credits::open();
    if !on {
        if let Ok(mut d) = DENSITY.lock() {
            *d = 0.0;
        }
        EXPOSED.store(false, std::sync::atomic::Ordering::Relaxed);
        for mut v in &mut vis {
            *v = Visibility::Hidden;
        }
        st.drops.clear();
        st.splashes.clear();
        return;
    }
    let prof = st.profile.unwrap();
    let mut seed = st.seed;
    // MissionScript.SetRainLevel, once a second.
    st.tick += dt;
    while st.tick >= 1.0 {
        st.tick -= 1.0;
        if mode() >= 2 {
            if st.density <= 0.0 && st.next_change == 0.0 {
                st.density = 5.0 + rnd(&mut seed) as f32 * 2.0;
            }
            if st.next_change <= 0.0 {
                st.desired = rnd(&mut seed) as f32 * prof.max;
                st.next_change = 150.0 + (rnd(&mut seed) * 250.0) as f32;
            } else if (st.density - st.desired).abs() < 1.0 {
                st.density = st.desired;
            } else if st.density < st.desired {
                st.density += rnd(&mut seed) as f32 * 0.4;
            } else {
                st.density -= rnd(&mut seed) as f32 * 0.4;
            }
            st.next_change -= 1.0;
        } else {
            st.density = 7.0f32.min(prof.max);
        }
        st.density = st.density.clamp(0.0, prof.max);
    }
    if let Ok(mut d) = DENSITY.lock() {
        *d = st.density;
    }
    // The streak mesh (and the splash mesh), once a map.
    if !st.spawned {
        st.spawned = true;
        let empty = |cap: usize| {
            let (mut p, mut u, mut c, mut i) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            pad(&mut p, &mut u, &mut c, &mut i, cap);
            let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, p);
            m.insert_attribute(Mesh::ATTRIBUTE_UV_0, u);
            m.insert_attribute(Mesh::ATTRIBUTE_COLOR, c);
            m.insert_indices(Indices::U32(i));
            m
        };
        let add_mat = |materials: &mut Assets<StandardMaterial>, tex: Option<Handle<Image>>| {
            materials.add(StandardMaterial {
                base_color_texture: tex,
                unlit: true,
                double_sided: true,
                cull_mode: None,
                alpha_mode: AlphaMode::Add,
                ..default()
            })
        };
        let drop_tex = image_for(&mut assets, &mut images, "Precipitation.Skins.Raindrop");
        // Splashlets: little streaks thrown up and out of the impact (the drop texture, stretched along the motion).
        let splash_tex = drop_tex.clone().or_else(|| image_for(&mut assets, &mut images, "Effects.Generated.WtrDrpSmall"));
        let m = meshes.add(empty(MAX_DROPS));
        let sm = meshes.add(empty(MAX_SPLASHES));
        let mat = add_mat(&mut materials, drop_tex);
        let smat = add_mat(&mut materials, splash_tex);
        for (mesh, mat) in [(m.clone(), mat), (sm.clone(), smat)] {
            commands.spawn((
                Mesh3d(mesh),
                MeshMaterial3d(mat),
                Transform::default(),
                Visibility::Visible,
                RainMesh,
                bevy::camera::visibility::RenderLayers::layer(RAIN_LAYER),
                bevy::camera::visibility::NoFrustumCulling,
                bevy::light::NotShadowCaster,
                bevy::light::NotShadowReceiver,
            ));
        }
        st.mesh = Some(m);
        st.splash_mesh = Some(sm);
        st.height = 300.0;
    }
    for mut v in &mut vis {
        *v = Visibility::Visible;
    }
    let me = player.body.location;
    let pvel = player.body.velocity;
    // PrecipRad 650 made the rain a visible column round JC: ours falls over a far wider square with the same
    // drops per area (the mod's rate scaled by the area), so its edge is out where the streaks are too thin to see.
    let rad = 650.0f64;
    let wide = WIDE_RAD;
    let area_k = ((wide / rad) * (wide / rad)) as f32;
    // Precipitator.Timer: the batch height grows from 300 so the first drops reach the ground quickly - ours on up
    // to RAIN_TOP (the mod's 600 had them start in plain sight a few metres overhead), capped by the sky above.
    st.height = (st.height + dt * 60000.0 * 0.01).min(RAIN_TOP as f32);
    let eye = {
        let e = dx_level::bevy_to_ue(cam.translation.to_array());
        DVec3::new(e[0] as f64, e[1] as f64, e[2] as f64)
    };
    let fwd = {
        let f = cam.forward().as_vec3();
        DVec3::new(f.x as f64, f.z as f64, f.y as f64).normalize_or_zero()
    };
    let yaw_fwd = DVec3::new(fwd.x, fwd.y, 0.0).normalize_or_zero();
    // PrecipDensity drops every PrecipFreq (0.01 s).
    st.acc += dt * st.density / 0.01 * area_k;
    let mut n = st.acc.floor() as usize;
    st.acc -= n as f32;
    n = n.min(2500);
    for _ in 0..n {
        if st.drops.len() >= MAX_DROPS {
            break;
        }
        // Precipitator.RandomSpawn.
        let mut pos = me + pvel * rad / 1200.0 + yaw_fwd * (rad / 4.0) - prof.wind
            + DVec3::new(
                (rnd(&mut seed) - 0.5) * wide * 2.0,
                (rnd(&mut seed) - 0.5) * wide * 2.0,
                0.0,
            );
        let key = ((pos.x / 128.0).floor() as i32, (pos.y / 128.0).floor() as i32, (me.z / 256.0).floor() as i32);
        let top = *st.ceiling.entry(key).or_insert_with(|| {
            let base = DVec3::new((key.0 as f64 + 0.5) * 128.0, (key.1 as f64 + 0.5) * 128.0, me.z + 64.0);
            let up = base + DVec3::new(0.0, 0.0, RAIN_TOP + 200.0);
            let h = lvl.collision.trace(base, up, DVec3::ZERO);
            base.z + (RAIN_TOP + 200.0) * h.fraction - 24.0
        });
        let want = me.z + st.height as f64 + (rnd(&mut seed) - 0.5) * 150.0;
        pos.z = want.min(top - rnd(&mut seed) * 60.0);
        if pos.z < me.z + 40.0 {
            continue;
        }
        let z = lvl.collision.zone_at(pos);
        if z <= 0 || !lvl.zone_sky.get(z as usize).copied().unwrap_or(false) {
            continue;
        }
        // Raindrop.Randomize.
        let vel = DVec3::new(0.0, 0.0, -900.0 - rnd(&mut seed) * 800.0) + prof.wind;
        // Long enough to reach the ground from the top (it ends where it lands).
        let life = ((pos.z - me.z + 600.0) / -vel.z).clamp(1.0, 4.0) as f32;
        let end = pos + vel * life as f64;
        let h = lvl.collision.trace(pos, end, DVec3::ZERO);
        let hit = (h.fraction < 1.0).then(|| (pos + (end - pos) * h.fraction, h.normal));
        st.drops.push(Drop {
            pos,
            vel,
            hit,
            life,
            len: 8.0 + rnd(&mut seed) as f32 * 22.0,
            glow: 0.3 + rnd(&mut seed) as f32 * 0.6,
        });
    }
    // Fall; land; splash.
    let mut splashes = Vec::new();
    let mut rings = Vec::new();
    st.drops.retain_mut(|d| {
        d.life -= dt;
        let next = d.pos + d.vel * dt as f64;
        if let Some((at, n)) = d.hit {
            if (next - d.pos).dot(at - d.pos) >= (at - d.pos).length_squared() {
                // Landed: Precipitator.Impact (only where it's seen).
                if (at - eye).dot(fwd) > 0.0 && at.distance(eye) < 1500.0 {
                    splashes.push((at, n));
                }
                return false;
            }
        }
        // Into water: a RaindropRing.
        if crate::actors::water_at(lvl, next).is_some() {
            if next.distance(eye) < 1500.0 {
                rings.push(next);
            }
            return false;
        }
        d.pos = next;
        d.life > 0.0
    });
    for (at, n) in splashes {
        // A crown: 3-5 droplets thrown out round the impact, mostly up off the surface.
        let t1 = if n.z.abs() < 0.9 { n.cross(DVec3::Z) } else { n.cross(DVec3::X) }.normalize_or_zero();
        let t2 = n.cross(t1);
        let count = 3 + (rnd(&mut seed) * 3.0) as usize;
        let a0 = rnd(&mut seed) as f64 * std::f64::consts::TAU;
        for k in 0..count {
            let a = a0 + k as f64 / count as f64 * std::f64::consts::TAU + (rnd(&mut seed) as f64 - 0.5) * 0.8;
            let out = (t1 * a.cos() + t2 * a.sin()) * (25.0 + rnd(&mut seed) as f64 * 45.0);
            let up = n * (45.0 + rnd(&mut seed) as f64 * 55.0);
            let life = (0.18 + rnd(&mut seed) * 0.17) as f32;
            st.splashes.push(Splash { pos: at + n * 0.5, vel: out + up, life, max: life });
        }
    }
    for at in rings.into_iter().take(12) {
        crate::water::ripple(at, 0.15);
    }
    if st.splashes.len() > MAX_SPLASHES {
        let k = st.splashes.len() - MAX_SPLASHES;
        st.splashes.drain(..k);
    }
    st.splashes.retain_mut(|s| {
        s.life -= dt;
        s.vel.z -= 950.0 * dt as f64;
        s.pos += s.vel * dt as f64;
        s.life > 0.0
    });
    st.seed = seed;
    // Build the streaks: quads along the fall (slanted against the player's run, Raindrop.Tick), facing the camera.
    let ambient = 0.55f32;
    let mut pos = Vec::with_capacity(st.drops.len() * 4);
    let mut uv = Vec::with_capacity(st.drops.len() * 4);
    let mut col = Vec::with_capacity(st.drops.len() * 4);
    let mut idx = Vec::with_capacity(st.drops.len() * 6);
    for d in &st.drops {
        let dir = (d.vel - pvel * DVec3::new(0.2, 0.2, 0.0)).normalize_or_zero();
        let to_eye = eye - d.pos;
        // Far streaks get wider and dimmer by the same amount (the same light on screen) instead of thinning to
        // flickering sub-pixel lines.
        let spread = (to_eye.length() / 400.0).max(1.0);
        let side = dir.cross(to_eye.normalize_or_zero()).normalize_or_zero() * 0.35 * spread;
        let tail = d.pos - dir * d.len as f64;
        let b = |p: DVec3| Vec3::from(ue_to_bevy(p.as_vec3().to_array())).to_array();
        let k = pos.len() as u32;
        pos.extend([b(d.pos - side), b(d.pos + side), b(tail + side), b(tail - side)]);
        uv.extend([[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]);
        let g = d.glow * ambient / spread as f32;
        col.extend([[g, g, g, 1.0]; 4]);
        idx.extend([k, k + 1, k + 2, k, k + 2, k + 3]);
    }
    pad(&mut pos, &mut uv, &mut col, &mut idx, MAX_DROPS);
    if let Some(m) = st.mesh.as_ref().and_then(|h| meshes.get_mut(h)) {
        let mut m = m;
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
        m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
        m.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
        m.insert_indices(Indices::U32(idx));
    }
    // Splashlets: short streaks along each droplet's motion, facing the camera.
    let right = {
        let r = cam.right().as_vec3();
        DVec3::new(r.x as f64, r.z as f64, r.y as f64)
    };
    let mut pos = Vec::with_capacity(st.splashes.len() * 4);
    let mut uv = Vec::with_capacity(st.splashes.len() * 4);
    let mut col = Vec::with_capacity(st.splashes.len() * 4);
    let mut idx = Vec::with_capacity(st.splashes.len() * 6);
    for s in &st.splashes {
        // A short streak along the droplet's motion, facing the camera.
        let k = pos.len() as u32;
        let b = |p: DVec3| Vec3::from(ue_to_bevy(p.as_vec3().to_array())).to_array();
        let speed = s.vel.length().max(1.0);
        let dir = s.vel / speed;
        let to_eye = (eye - s.pos).normalize_or_zero();
        let side = dir.cross(to_eye).normalize_or(right) * 0.35;
        let len = (speed * 0.025).clamp(1.0, 4.0);
        let tail = s.pos - dir * len;
        pos.extend([b(s.pos - side), b(s.pos + side), b(tail + side), b(tail - side)]);
        uv.extend([[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]);
        let g = (s.life / s.max.max(0.01)).clamp(0.0, 1.0) * 0.5;
        col.extend([[g, g, g, 1.0]; 4]);
        idx.extend([k, k + 1, k + 2, k, k + 2, k + 3]);
    }
    pad(&mut pos, &mut uv, &mut col, &mut idx, MAX_SPLASHES);
    if let Some(m) = st.splash_mesh.as_ref().and_then(|h| meshes.get_mut(h)) {
        let mut m = m;
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
        m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
        m.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
        m.insert_indices(Indices::U32(idx));
    }
    // Shelter: how much open sky is about the player (its own zone, and points around it in sight), eased.
    st.exposure_check -= dt;
    if st.exposure_check <= 0.0 {
        st.exposure_check = 0.25;
        let sky = |p: DVec3| {
            let z = lvl.collision.zone_at(p);
            z > 0 && lvl.zone_sky.get(z as usize).copied().unwrap_or(false)
        };
        let here = sky(eye) as u8 as f32;
        let mut seen = 0;
        let mut total = 0;
        for k in 0..12 {
            let a = k as f64 / 12.0 * std::f64::consts::TAU;
            for r in [192.0, 448.0] {
                let p = eye + DVec3::new(a.cos() * r, a.sin() * r, 0.0);
                total += 1;
                let h = lvl.collision.trace(eye, p, DVec3::ZERO);
                let q = eye + (p - eye) * h.fraction.max(0.0) - (p - eye).normalize_or_zero() * 4.0;
                if sky(q) && sky(q + DVec3::new(0.0, 0.0, 64.0)) {
                    seen += 1;
                }
            }
        }
        st.sky_near = seen as f32 / total as f32;
        EXPOSED.store(here > 0.5, std::sync::atomic::Ordering::Relaxed);
        let want = (0.6 * here + 0.4 * st.sky_near).max(here * 0.8);
        st.exposure += (want - st.exposure) * 0.5;
    }
    // FollowingSound x NoiseMult (2) at NoiseVolume = 255 / 20 x density; RainInside under shelter near the open.
    let loud = (st.density / 20.0).clamp(0.0, 1.0);
    let out_vol = loud * st.exposure * 2.0 * 0.5;
    let in_vol = loud * (1.0 - st.exposure) * (st.sky_near * 2.0).min(1.0) * 0.6;
    if out_vol > 0.01 {
        crate::sound::hold(0xEA7E_0001, "Precipitation.Ambient.RainCity", None, out_vol, 0.0, 1.0);
    }
    if in_vol > 0.01 {
        crate::sound::hold(0xEA7E_0002, "Precipitation.Ambient.RainInside", None, in_vol, 0.0, 1.0);
    }
}

/// A new map is going in: nothing to keep (the state is a Local reset by loader::new_map).
pub fn reset_level() {
    if let Ok(mut d) = DENSITY.lock() {
        *d = 0.0;
    }
    EXPOSED.store(false, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rainy_maps() {
        assert!(profile("01_NYC_UNATCOIsland").is_some());
        assert!(profile("06_HongKong_WanChai_Market.dx").is_some());
        assert!(profile("10_Paris_Chateau").is_some_and(|p| p.max <= 5.0));
        assert!(profile("12_Vandenberg_Cmd").is_none());
        assert!(profile("15_Area51_Bunker").is_none());
        assert!(profile("00_Intro").is_none());
        assert!(profile("05_NYC_UNATCOMJ12lab").is_none());
    }
}

const MAX_DROPS: usize = 9000;
/// Half the side of the square the drops fall in (Unreal units).
const WIDE_RAD: f64 = 1500.0;
/// How high over the player rain starts, sky permitting (Unreal units).
const RAIN_TOP: f64 = 1800.0;
const MAX_SPLASHES: usize = 1500;

/// Fills the quad buffers up to `cap` quads with collapsed (zero-area, black) ones: the meshes keep one size, so the
/// render world never reallocates them (a mesh changing size every frame, or going empty, upset the slab allocator).
fn pad(pos: &mut Vec<[f32; 3]>, uv: &mut Vec<[f32; 2]>, col: &mut Vec<[f32; 4]>, idx: &mut Vec<u32>, cap: usize) {
    pos.truncate(cap * 4);
    uv.truncate(cap * 4);
    col.truncate(cap * 4);
    idx.truncate(cap * 6);
    while pos.len() < cap * 4 {
        let k = pos.len() as u32;
        pos.extend([[0.0, -1.0e5, 0.0]; 4]);
        uv.extend([[0.0, 0.0]; 4]);
        col.extend([[0.0, 0.0, 0.0, 0.0]; 4]);
        idx.extend([k, k, k, k, k, k]);
    }
}
