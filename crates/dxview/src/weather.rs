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
    /// The top of the column it fell from: streaks fade in over the first stretch under it, so the column has no
    /// visible ceiling.
    top: f64,
    /// The height where it falls into water, found once when it's spawned (testing the zone under every drop
    /// every frame was most of the rain's CPU time).
    water_z: Option<f64>,
}

#[derive(Clone, Copy)]
struct Land {
    z: f64,
    n: DVec3,
    water: Option<f64>,
}

struct Splash {
    pos: DVec3,
    vel: DVec3,
    life: f32,
    max: f32,
    /// The surface it was thrown off (point, normal): a droplet falling back through it is gone (the dock roof is a
    /// thin sheet, and droplets falling back under it showed through from below).
    base: DVec3,
    n: DVec3,
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
    /// Per-quad data the rain shader builds the streaks from (two Rgba32Float texels a quad).
    data: Option<Handle<Image>>,
    splash_data: Option<Handle<Image>>,
    mats: Vec<Handle<RainMaterial>>,
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
    /// Where rain lands in each 16-unit cell (by the player's 256-unit height band): the surface under the cell's
    /// ceiling (collision or a drawn roof), its normal, and the water surface if it lands in water. One straight
    /// trace down per cell instead of one per drop.
    landing: std::collections::HashMap<(i32, i32, i32), Land>,
    /// The drawn level's roofs (built once per map).
    roofs: Option<Roofs>,
}

/// Every drawn BSP face that isn't a wall, by 128-unit cell, in Unreal units: rain lands on what's drawn, not only on
/// what collides - the dock's roof is a non-solid brush (the collision hull has nothing there), so the drops went
/// through it and fell under it.
#[derive(Default)]
struct Roofs {
    cells: std::collections::HashMap<(i32, i32), Vec<u32>>,
    tris: Vec<[DVec3; 3]>,
}

const ROOF_CELL: f64 = 128.0;

impl Roofs {
    fn build(lvl: &dx_level::Level) -> Roofs {
        let mut r = Roofs::default();
        for b in lvl.batches.iter().filter(|b| {
            !b.sky && matches!(b.class, dx_level::SurfaceClass::Opaque | dx_level::SurfaceClass::Masked)
        }) {
            for t in b.indices.chunks_exact(3) {
                let p = [0, 1, 2].map(|k| {
                    let u = dx_level::bevy_to_ue(b.positions[t[k] as usize]);
                    DVec3::new(u[0] as f64, u[1] as f64, u[2] as f64)
                });
                let n = (p[1] - p[0]).cross(p[2] - p[0]);
                let len = n.length();
                if len < 1e-6 || (n.z / len).abs() < 0.2 {
                    continue; // walls (and slivers) cover nothing
                }
                let i = r.tris.len() as u32;
                r.tris.push(p);
                let lo = p[0].min(p[1]).min(p[2]);
                let hi = p[0].max(p[1]).max(p[2]);
                for cx in (lo.x / ROOF_CELL).floor() as i32..=(hi.x / ROOF_CELL).floor() as i32 {
                    for cy in (lo.y / ROOF_CELL).floor() as i32..=(hi.y / ROOF_CELL).floor() as i32 {
                        r.cells.entry((cx, cy)).or_default().push(i);
                    }
                }
            }
        }
        r
    }

    /// The highest drawn surface straight below `p` (within `depth` units), as its height.
    fn below(&self, p: DVec3, depth: f64) -> Option<f64> {
        let key = ((p.x / ROOF_CELL).floor() as i32, (p.y / ROOF_CELL).floor() as i32);
        let mut best: Option<f64> = None;
        for &i in self.cells.get(&key)? {
            let [a, b, c] = self.tris[i as usize];
            // Barycentric in xy.
            let d = (b.y - c.y) * (a.x - c.x) + (c.x - b.x) * (a.y - c.y);
            if d.abs() < 1e-9 {
                continue;
            }
            let w0 = ((b.y - c.y) * (p.x - c.x) + (c.x - b.x) * (p.y - c.y)) / d;
            let w1 = ((c.y - a.y) * (p.x - c.x) + (a.x - c.x) * (p.y - c.y)) / d;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let z = w0 * a.z + w1 * b.z + w2 * c.z;
            if z <= p.z && z >= p.z - depth && best.is_none_or(|bz| z > bz) {
                best = Some(z);
            }
        }
        best
    }

    /// A drawn surface over `p` within `height` units (a roof overhead).
    fn covered(&self, p: DVec3, height: f64) -> bool {
        self.below(p + DVec3::new(0.0, 0.0, height), height - 8.0).is_some()
    }
}

#[derive(Component)]
pub struct RainMesh;

/// Raining here and now, and how hard (PrecipDensity 0..14).
static DENSITY: std::sync::Mutex<f32> = std::sync::Mutex::new(0.0);
static EXPOSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static EXPOSURE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// How open to the sky the player is while it rains (0 under a roof .. 1 out in it), eased; 0 when dry.
pub fn exposure() -> f32 {
    if density() <= 0.0 {
        return 0.0;
    }
    f32::from_bits(EXPOSURE.load(std::sync::atomic::Ordering::Relaxed))
}

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
        ResMut<Assets<RainMaterial>>,
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
    if st.roofs.is_none() && mode() > 0 && st.profile.is_some() {
        st.roofs = Some(Roofs::build(lvl));
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
        // Fixed quads that never change: POSITION carries (quad, corner); the shader (shaders/rain.wgsl) builds each
        // camera-facing streak from the quad's two data texels, written each frame (a tenth of the bytes the
        // finished quads were, and no mesh re-upload).
        let quads = |cap: usize| {
            let p: Vec<[f32; 3]> = (0..cap).flat_map(|q| (0..4).map(move |c| [q as f32, c as f32, 0.0])).collect();
            let u: Vec<[f32; 2]> = (0..cap).flat_map(|_| [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]).collect();
            let i: Vec<u32> = (0..cap as u32).flat_map(|q| {
                let k = q * 4;
                [k, k + 1, k + 2, k, k + 2, k + 3]
            }).collect();
            let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, p);
            m.insert_attribute(Mesh::ATTRIBUTE_UV_0, u);
            m.insert_indices(Indices::U32(i));
            m
        };
        let data_image = |images: &mut Assets<Image>, cap: usize| {
            let rows = (cap * 2).div_ceil(DATA_W) as u32;
            let mut img = Image::new_fill(
                bevy::render::render_resource::Extent3d { width: DATA_W as u32, height: rows, depth_or_array_layers: 1 },
                bevy::render::render_resource::TextureDimension::D2,
                &[0u8; 16],
                bevy::render::render_resource::TextureFormat::Rgba32Float,
                RenderAssetUsages::default(),
            );
            img.sampler = bevy::image::ImageSampler::nearest();
            images.add(img)
        };
        let drop_tex = image_for(&mut assets, &mut images, "Precipitation.Skins.Raindrop");
        // Splashlets: little streaks thrown up and out of the impact (the drop texture, stretched along the motion).
        let splash_tex = drop_tex.clone().or_else(|| image_for(&mut assets, &mut images, "Effects.Generated.WtrDrpSmall"));
        let m = meshes.add(quads(MAX_DROPS));
        let sm = meshes.add(quads(MAX_SPLASHES));
        let d = data_image(&mut images, MAX_DROPS);
        let sd = data_image(&mut images, MAX_SPLASHES);
        let mat = materials.add(RainMaterial { tex: drop_tex.unwrap_or_default(), data: d.clone() });
        let smat = materials.add(RainMaterial { tex: splash_tex.unwrap_or_default(), data: sd.clone() });
        st.mats = vec![mat.clone(), smat.clone()];
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
        st.data = Some(d);
        st.splash_data = Some(sd);
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
    let _p1 = crate::prof::scope("weather::spawn");
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
            // Up to the sky: a hit with open air above it (a roof - the dock's, a porch's) isn't the sky, so the
            // search goes on above it and the rain starts over the roof and lands on it (it used to start just
            // under the roof and fall inside). The map's sky ceiling has solid (no zone) above it.
            let mut base = DVec3::new((key.0 as f64 + 0.5) * 128.0, (key.1 as f64 + 0.5) * 128.0, me.z + 64.0);
            let up_z = base.z + RAIN_TOP + 200.0;
            let mut top = up_z - 24.0;
            for _ in 0..6 {
                if base.z >= up_z {
                    break;
                }
                let up = DVec3::new(base.x, base.y, up_z);
                let h = lvl.collision.trace(base, up, DVec3::ZERO);
                if h.fraction >= 1.0 {
                    break;
                }
                let hit = base + (up - base) * h.fraction;
                let above = hit + DVec3::new(0.0, 0.0, 24.0);
                if lvl.collision.zone_at(above) > 0 && lvl.collision.trace(hit + DVec3::new(0.0, 0.0, 1.0), above, DVec3::ZERO).fraction < 1.0 {
                    // Still solid 24 units up: look further for the roof's top.
                    base = hit + DVec3::new(0.0, 0.0, 48.0);
                    continue;
                }
                if lvl.collision.zone_at(above) > 0 {
                    base = above;
                    continue;
                }
                top = hit.z - 24.0;
                break;
            }
            top
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
        let lkey = ((pos.x / 16.0).floor() as i32, (pos.y / 16.0).floor() as i32, key.2);
        let ws = &mut *st;
        let land = *ws.landing.entry(lkey).or_insert_with(|| {
            // Straight down from the cell's ceiling (wind slant is small against the fall).
            let c = DVec3::new((lkey.0 as f64 + 0.5) * 16.0, (lkey.1 as f64 + 0.5) * 16.0, top);
            let low = DVec3::new(c.x, c.y, me.z - 2500.0);
            let h = lvl.collision.trace(c, low, DVec3::ZERO);
            let (mut z, mut n) = if h.fraction < 1.0 { (c.z + (low.z - c.z) * h.fraction, h.normal) } else { (f64::NEG_INFINITY, DVec3::Z) };
            // A drawn roof above where it would land (no collision there): it lands on the roof.
            if let Some(rz) = roofs_below(&ws.roofs, c, c.z - z.max(low.z)) {
                if rz > z + 1.0 {
                    z = rz;
                    n = DVec3::Z;
                }
            }
            // Water on the way down: if where it ends is in water, the surface is somewhere between (bisect).
            let at = DVec3::new(c.x, c.y, z.max(low.z) + 0.5);
            let water = crate::actors::water_at(lvl, at).map(|_| {
                let (mut a, mut b) = (c, at);
                if crate::actors::water_at(lvl, a).is_some() {
                    return a.z + 1.0;
                }
                for _ in 0..12 {
                    let m = (a + b) * 0.5;
                    if crate::actors::water_at(lvl, m).is_some() {
                        b = m;
                    } else {
                        a = m;
                    }
                }
                b.z
            });
            Land { z, n, water }
        });
        // Starting below where this cell's rain lands (inside, under the roof): it never fell from the sky.
        if land.z > pos.z - 1.0 {
            continue;
        }
        let hit = (land.z > end.z).then(|| {
            let t = ((pos.z - land.z) / (pos.z - end.z)).clamp(0.0, 1.0);
            (pos + (end - pos) * t, land.n)
        });
        let water_z = land.water;
        st.drops.push(Drop {
            pos,
            vel,
            hit,
            life,
            len: 8.0 + rnd(&mut seed) as f32 * 22.0,
            glow: 0.3 + rnd(&mut seed) as f32 * 0.6,
            top: top.min(me.z + RAIN_TOP + 80.0),
            water_z,
        });
    }
    drop(_p1);
    let _p2 = crate::prof::scope("weather::fall");
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
        if d.water_z.is_some_and(|wz| next.z <= wz) {
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
            st.splashes.push(Splash { pos: at + n * 0.5, vel: out + up, life, max: life, base: at, n });
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
        // Back down to the surface: gone.
        s.life > 0.0 && (s.pos - s.base).dot(s.n) > 0.4
    });
    st.seed = seed;
    drop(_p2);
    let _p3 = crate::prof::scope("weather::build");
    // Build the streaks: quads along the fall (slanted against the player's run, Raindrop.Tick), facing the camera.
    // Written in place into the fixed-size mesh, the drops split over all cores.
    let ambient = 0.55f32;
    let right = {
        let r = cam.right().as_vec3();
        DVec3::new(r.x as f64, r.z as f64, r.y as f64)
    };
    if std::env::var_os("DXVIEW_RAINLOG").is_some() {
        eprintln!("RAIN density {:.1} drops {} splashes {} height {:.0}", st.density, st.drops.len(), st.splashes.len(), st.height);
    }
    let drops = &st.drops;
    if let Some(mut img) = st.data.as_ref().and_then(|h| images.get_mut(h)) {
        write_data(&mut img, drops.len(), MAX_DROPS, |i| {
            let d = &drops[i];
            let dir = (d.vel - pvel * DVec3::new(0.2, 0.2, 0.0)).normalize_or_zero();
            // Far streaks get wider and dimmer by the same amount (the same light on screen) instead of thinning to
            // flickering sub-pixel lines.
            let spread = ((eye - d.pos).length() / 400.0).max(1.0);
            let tail = d.pos - dir * d.len as f64;
            // Fading in under the column's top (over 700 units) hides where the rain begins.
            let under = ((d.top - d.pos.z) / 700.0).clamp(0.0, 1.0) as f32;
            let g = d.glow * ambient / spread as f32 * under * under;
            let (h, t) = (bv(d.pos), bv(tail));
            [h[0], h[1], h[2], g, t[0], t[1], t[2], (0.35 * spread) as f32 * dx_level::METERS_PER_UNIT]
        });
    }
    // Splashlets: short streaks along each droplet's motion, facing the camera.
    let splashes = &st.splashes;
    if let Some(mut img) = st.splash_data.as_ref().and_then(|h| images.get_mut(h)) {
        write_data(&mut img, splashes.len(), MAX_SPLASHES, |i| {
            let s = &splashes[i];
            let speed = s.vel.length().max(1.0);
            let dir = s.vel / speed;
            let mut len = (speed * 0.025).clamp(1.0, 4.0);
            // The tail never reaches under the surface it was thrown off.
            let fall = -dir.dot(s.n);
            if fall < 0.0 {
                len = len.min(((s.pos - s.base).dot(s.n) - 0.3).max(0.0) / -fall);
            }
            let tail = s.pos - dir * len;
            let g = (s.life / s.max.max(0.01)).clamp(0.0, 1.0) * 0.5;
            let (h, t) = (bv(s.pos), bv(tail));
            [h[0], h[1], h[2], g, t[0], t[1], t[2], 0.35 * dx_level::METERS_PER_UNIT]
        });
    }
    let _ = right;
    // The materials' bind groups hold the data images' GPU textures: a re-uploaded image is a new texture, so
    // the materials are touched to rebuild them.
    for h in &st.mats {
        let _ = materials.get_mut(h);
    }
    drop(_p3);
    // Shelter: how much open sky is about the player (its own zone, and points around it in sight), eased.
    st.exposure_check -= dt;
    if st.exposure_check <= 0.0 {
        st.exposure_check = 0.25;
        let sky = |p: DVec3| {
            let z = lvl.collision.zone_at(p);
            z > 0 && lvl.zone_sky.get(z as usize).copied().unwrap_or(false)
        };
        // Under a drawn roof (the dock's) counts as indoors too.
        let roofed = st.roofs.as_ref().is_some_and(|r| r.covered(eye, 1200.0));
        let here = (sky(eye) && !roofed) as u8 as f32;
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
        EXPOSURE.store(st.exposure.to_bits(), std::sync::atomic::Ordering::Relaxed);
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

const MAX_DROPS: usize = 11000;
/// Half the side of the square the drops fall in (Unreal units).
const WIDE_RAD: f64 = 1500.0;
/// How high over the player rain starts, sky permitting (Unreal units).
const RAIN_TOP: f64 = 2600.0;
const MAX_SPLASHES: usize = 1500;


fn roofs_below(roofs: &Option<Roofs>, p: DVec3, depth: f64) -> Option<f64> {
    roofs.as_ref().and_then(|r| r.below(p, depth))
}

fn bv(p: DVec3) -> [f32; 3] {
    ue_to_bevy(p.as_vec3().to_array())
}

/// Fills the first `n` quads of a fixed-size quad mesh (positions and colours, 4 corners each) with `f`, in
/// parallel, and hides the rest.
/// Texels a row in the rain data images.
const DATA_W: usize = 1024;

/// Fill a rain data image: quad i = two texels (head xyz + brightness, tail xyz + half width; Bevy metres),
/// unused quads zero (brightness 0: the shader drops them). The quads are split over all cores.
fn write_data(img: &mut Image, n: usize, cap: usize, f: impl Fn(usize) -> [f32; 8] + Sync) {
    let n = n.min(cap);
    if std::env::var_os("DXVIEW_RAINLOG").is_some() {
        eprintln!("RAIN data {:?} {:?} n {n} first {:?}", img.data.as_ref().map(|d| d.len()), img.texture_descriptor.size, (0..n.min(3)).map(|i| f(i)).collect::<Vec<_>>());
    }
    let Some(bytes) = img.data.as_mut() else { return };
    const CHUNK: usize = 1024;
    let pool = bevy::tasks::ComputeTaskPool::get();
    let f = &f;
    let used = (n * 32).min(bytes.len());
    let (live, rest) = bytes.split_at_mut(used);
    pool.scope(|sc| {
        for (ci, chunk) in live.chunks_mut(CHUNK * 32).enumerate() {
            sc.spawn(async move {
                for (q, out) in chunk.chunks_exact_mut(32).enumerate() {
                    for (k, v) in f(ci * CHUNK + q).iter().enumerate() {
                        out[k * 4..k * 4 + 4].copy_from_slice(&v.to_le_bytes());
                    }
                }
            });
        }
    });
    rest.fill(0);
}

pub const RAIN_SHADER: Handle<Shader> = bevy::asset::uuid_handle!("8d4c2b1e-7f3a-4e95-a6d0-3b9e1c5f7a28");

/// Rain streaks and splashlets built on the GPU from a data image (shaders/rain.wgsl).
#[derive(Asset, bevy::render::render_resource::AsBindGroup, Reflect, Debug, Clone)]
pub struct RainMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub tex: Handle<Image>,
    #[texture(2, sample_type = "float", filterable = false)]
    pub data: Handle<Image>,
}

impl Material for RainMaterial {
    fn vertex_shader() -> bevy::shader::ShaderRef {
        RAIN_SHADER.into()
    }
    fn fragment_shader() -> bevy::shader::ShaderRef {
        RAIN_SHADER.into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    // The streaks face the camera whichever way they wind.
    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

pub fn setup_rain_shader(mut shaders: ResMut<Assets<Shader>>) {
    let _ = shaders.insert(&RAIN_SHADER, Shader::from_wgsl(include_str!("shaders/rain.wgsl"), "dxview/rain.wgsl"));
}
