//! PlayerPawn.ShakeView / ClientShake / ViewShake: the view rolls back and forth and the eye bobs for a while
//! (explosions, set pieces, hard landings, being hit, weapon kick, ShakeTrigger, the Karkian's ram).

use std::sync::Mutex;

#[derive(Default)]
struct Shake {
    shakemag: f32,
    shaketimer: f32,
    maxshake: f32,
    verttimer: f32,
    shake_vert: f32,
    /// ViewRotation.Roll's shake part (0..65535).
    roll: i32,
    dir: bool,
    seed: u64,
}

static STATE: Mutex<Option<Shake>> = Mutex::new(None);

/// ShakeView(shaketime, RollMag, vertmag) -> ClientShake(X = roll, Y = 100 time, Z = 100 vert).
pub fn shake_view(time: f32, roll: f32, vert: f32) {
    if let Ok(mut s) = STATE.lock() {
        let s = s.get_or_insert_with(Shake::default);
        if s.shakemag < roll || s.shaketimer <= time {
            s.shakemag = roll;
            s.shaketimer = time;
            s.maxshake = vert;
            s.verttimer = 0.0;
            s.shake_vert = -1.1 * s.maxshake;
        }
    }
}

pub fn reset_level() {
    if let Ok(mut s) = STATE.lock() {
        *s = None;
    }
}

fn frand(seed: &mut u64) -> f32 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    ((*seed >> 40) as f32) / (1u64 << 24) as f32
}

/// ViewShake(DeltaTime): returns (roll in Unreal units, signed, ShakeVert for the eye height).
pub fn tick(dt: f32) -> (f64, f64) {
    let Ok(mut g) = STATE.lock() else { return (0.0, 0.0) };
    let s = g.get_or_insert_with(Shake::default);
    let d = dt.min(0.1);
    let mut r = s.roll & 65535;
    if s.shaketimer > 0.0 {
        s.shaketimer -= dt;
        if s.verttimer == 0.0 {
            s.verttimer = 0.1;
            s.shake_vert = -1.1 * s.maxshake;
        } else {
            s.verttimer -= dt;
            if s.verttimer < 0.0 {
                s.verttimer = 0.2 * frand(&mut s.seed);
                s.shake_vert = (2.0 * frand(&mut s.seed) - 1.0) * s.maxshake;
            }
        }
        let m = s.shakemag;
        if s.dir {
            r += (10.0 * m * d) as i32;
            s.dir = r > 32768 || (r as f32) < (0.5 + frand(&mut s.seed)) * m;
            if r < 32768 && r as f32 > 1.3 * m {
                r = (1.3 * m) as i32;
                s.dir = false;
            } else if frand(&mut s.seed) < 3.0 * dt {
                s.dir = !s.dir;
            }
        } else {
            r -= (10.0 * m * d) as i32;
            r &= 65535;
            s.dir = r > 32768 && (r as f32) < 65535.0 - (0.5 + frand(&mut s.seed)) * m;
            if r > 32768 && (r as f32) < 65535.0 - 1.3 * m {
                r = (65535.0 - 1.3 * m) as i32;
                s.dir = true;
            } else if frand(&mut s.seed) < 3.0 * dt {
                s.dir = !s.dir;
            }
        }
    } else {
        s.shake_vert = 0.0;
        if r < 32768 {
            if r > 0 {
                r = (r - (r.max(500) as f32 * 10.0 * d) as i32).max(0);
            }
        } else {
            r += ((65536 - r.max(500)) as f32 * 10.0 * d) as i32;
            if r > 65534 {
                r = 0;
            }
        }
    }
    s.roll = r & 65535;
    let signed = if s.roll >= 32768 { s.roll - 65536 } else { s.roll };
    (signed as f64, s.shake_vert as f64)
}
