//! Enhancements to the sound (ROADMAP v2 #34 / #35). Ours, all off by default.
//!
//! - Better Sound Playback (HQAudio): smoother resampling of the game's 11 / 22 kHz sounds (4-point Hermite rather
//!   than straight lines), constant-power panning, and while someone is speaking the rest of the sound steps back a
//!   little so the words come through.
//! - Sound Through Walls (SoundOcclusion): a sound with a wall between it and you is quieter and muffled; one you can
//!   reach around a corner or through a doorway nearby only a little (a few side paths are traced).
//! - Room Reverb (RoomReverb): an echo shaped by the space around you - long in halls and the statue's base, short
//!   in corridors, faint outdoors, dull underwater - estimated by tracing out from your head every half second.
//! - Soundscape (Soundscape): far-off life around the open-air maps, from the game's own sounds - dogs, gulls by the
//!   water, gusts of wind (more in the rain) - placed at random far away in the open and played quietly now and then.

use crate::LevelRes;
use glam::DVec3;
use std::collections::HashMap;
use std::sync::Mutex;

pub fn hq_on() -> bool {
    crate::settings::get("HQAudio") > 0.5
}

pub fn occlusion_on() -> bool {
    crate::settings::get("SoundOcclusion") > 0.5
}

pub fn reverb_on() -> bool {
    crate::settings::get("RoomReverb") > 0.5
}

pub fn soundscape_on() -> bool {
    crate::settings::get("Soundscape") > 0.5
}

/// Occlusion results by source cell: (gain, filter, time checked).
#[derive(Default)]
pub struct Occluder {
    cache: HashMap<(i32, i32, i32), (f32, f32, f32)>,
    /// The listener's cell when the cache was filled (moving clears it).
    ear_cell: (i32, i32, i32),
}

fn cell(p: DVec3, size: f64) -> (i32, i32, i32) {
    ((p.x / size).floor() as i32, (p.y / size).floor() as i32, (p.z / size).floor() as i32)
}

impl Occluder {
    /// (gain multiplier, filter 0..1) for a sound at `src` heard at `ear`.
    pub fn at(&mut self, level: &dx_level::Level, ear: DVec3, src: DVec3, now: f32) -> (f32, f32) {
        let ec = cell(ear, 48.0);
        if ec != self.ear_cell {
            self.ear_cell = ec;
            self.cache.clear();
        }
        let key = cell(src, 32.0);
        if let Some(&(g, f, t)) = self.cache.get(&key) {
            if now - t < 0.25 {
                return (g, f);
            }
        }
        let clear = |a: DVec3, b: DVec3| level.collision.trace(a, b, DVec3::ZERO).fraction >= 0.999;
        // Start a little off the source toward the ear (sounds sit on walls and floors).
        let s = src + (ear - src).normalize_or_zero() * 8.0 + DVec3::new(0.0, 0.0, 4.0);
        let (g, f) = if clear(ear, s) {
            (1.0, 1.0)
        } else {
            // Around a corner or through an opening: a point off the line that both ends can see.
            let dir = (s - ear).normalize_or_zero();
            let side = dir.cross(DVec3::Z).normalize_or(DVec3::X);
            let mid = ear + (s - ear) * 0.5;
            let d = ear.distance(s).min(600.0);
            let mut around = false;
            for o in [side * d * 0.35, -side * d * 0.35, DVec3::Z * 96.0, side * d * 0.7, -side * d * 0.7] {
                let p = mid + o;
                if clear(ear, p) && clear(p, s) {
                    around = true;
                    break;
                }
            }
            if around { (0.8, 0.5) } else { (0.5, 0.12) }
        };
        self.cache.insert(key, (g, f, now));
        (g, f)
    }
}

/// The room around the listener: (room size, damping, wet), from traces out from `ear`.
pub fn room(level: &dx_level::Level, ear: DVec3) -> (f32, f32, f32) {
    let mut sum = 0.0;
    let mut open = 0;
    let dirs: Vec<DVec3> = (0..8)
        .map(|k| {
            let a = k as f64 / 8.0 * std::f64::consts::TAU;
            DVec3::new(a.cos(), a.sin(), 0.15)
        })
        .chain([DVec3::Z, DVec3::new(0.3, 0.2, -0.9)])
        .collect();
    let reach = 3000.0;
    for d in &dirs {
        let d = d.normalize();
        let h = level.collision.trace(ear, ear + d * reach, DVec3::ZERO);
        if h.fraction >= 0.999 {
            open += 1;
        }
        sum += h.fraction * reach;
    }
    let avg = sum / dirs.len() as f64;
    let zone = level.collision.zone_at(ear);
    let sky = zone > 0 && level.zone_sky.get(zone as usize).copied().unwrap_or(false);
    let water = crate::actors::water_at(level, ear).is_some();
    if water {
        return (0.3, 1.0, 0.25);
    }
    let size = (avg / 1600.0).clamp(0.0, 1.0) as f32;
    if sky || open >= 5 {
        // Outdoors: a faint, short slap back off nearby walls.
        return (0.2 + 0.2 * size, 0.7, 0.05);
    }
    // Indoors: bigger rooms ring longer; small ones are short and a little brighter.
    (0.3 + 0.65 * size, 0.35 + 0.3 * (1.0 - size), 0.12 + 0.2 * size)
}

/// Ducking under speech: the gain for everything but speech (eased).
static DUCK: Mutex<f32> = Mutex::new(1.0);

pub fn update_duck(talking: bool, dt: f32) -> f32 {
    let want = if hq_on() && talking { 0.6 } else { 1.0 };
    let Ok(mut d) = DUCK.lock() else { return 1.0 };
    *d += (want - *d).clamp(-dt * 3.0, dt * 1.5);
    *d
}

pub fn duck() -> f32 {
    DUCK.lock().map(|d| *d).unwrap_or(1.0)
}

#[derive(Default)]
pub struct Scape {
    next: f32,
    seed: u64,
}

fn rnd(seed: &mut u64) -> f32 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    ((*seed >> 40) as f32) / (1u64 << 24) as f32
}

/// Far-off sounds around the open-air maps, now and then.
pub fn soundscape_system(
    time: bevy::prelude::Res<bevy::prelude::Time>,
    level: bevy::prelude::Res<LevelRes>,
    player: bevy::prelude::Res<crate::Player>,
    mut st: bevy::prelude::Local<Scape>,
) {
    if !soundscape_on() {
        return;
    }
    let lvl = &level.0;
    st.next -= time.delta_secs();
    if st.next > 0.0 {
        return;
    }
    if st.seed == 0 {
        st.seed = 0x5C4F_E5EE_D123;
    }
    let mut seed = st.seed;
    let rain = crate::weather::density() / 14.0;
    st.next = 5.0 + rnd(&mut seed) * 14.0 - 4.0 * rain;
    let m = lvl.name.to_ascii_lowercase();
    let me = player.body.location;
    let zone = lvl.collision.zone_at(me);
    let outside = zone > 0 && lvl.zone_sky.get(zone as usize).copied().unwrap_or(false);
    if !outside {
        st.seed = seed;
        return;
    }
    let near_water = (0..6).any(|k| {
        let a = k as f64 / 6.0 * std::f64::consts::TAU;
        let p = me + DVec3::new(a.cos() * 900.0, a.sin() * 900.0, -200.0);
        crate::actors::water_at(lvl, p).is_some()
    });
    let city = ["01_", "02_", "03_", "04_", "05_", "06_", "08_", "09_", "10_", "11_"].iter().any(|p| m.starts_with(p));
    let mut pool: Vec<(&str, f32)> = vec![("DeusExSounds.Generic.WindGust1", 0.5 + rain), ("DeusExSounds.Generic.WindGust2", 0.5 + rain)];
    if city {
        pool.extend([
            ("DeusExSounds.Animal.DogLargeBark1", 0.6),
            ("DeusExSounds.Animal.DogLargeBark2", 0.6),
            ("DeusExSounds.Animal.DogSmallBark1", 0.5),
            ("DeusExSounds.Animal.DogSmallBark3", 0.5),
            ("DeusExSounds.Animal.PigeonCoo", 0.4 * (1.0 - rain)),
        ]);
    }
    if near_water {
        pool.extend([("DeusExSounds.Animal.SeagullCry", 1.4 * (1.0 - rain * 0.7))]);
    }
    let total: f32 = pool.iter().map(|p| p.1).sum();
    let mut pick = rnd(&mut seed) * total;
    let mut snd = pool[0].0;
    for (s, w) in &pool {
        if pick < *w {
            snd = s;
            break;
        }
        pick -= w;
    }
    // Somewhere far off in the open.
    let a = rnd(&mut seed) as f64 * std::f64::consts::TAU;
    let d = 900.0 + rnd(&mut seed) as f64 * 1300.0;
    let at = me + DVec3::new(a.cos() * d, a.sin() * d, 150.0 + rnd(&mut seed) as f64 * 300.0);
    let wind = snd.contains("Wind");
    let vol = if wind { 0.35 + 0.4 * rain } else { 0.45 };
    let pitch = 0.9 + rnd(&mut seed) * 0.2;
    // Heard over a wide radius (they are far away), so they stay quiet and spread.
    crate::sound::play(snd, at, vol, (d as f32) * 1.6, pitch);
    st.seed = seed;
}
