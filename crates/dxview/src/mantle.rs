//! Enhancement (Mantling, ROADMAP v2 #29). Ours, off by default.
//!
//! Jump (or hold Jump in the air) facing a ledge you can reach - higher than a step, up to about chest-and-arms
//! height above your feet - and you climb onto it: up first, then over the edge. A low obstacle you jump at is
//! vaulted onto the same way, and you carry on walking from its top. Nothing heavier than you can jump with in
//! your arms, not while crouched, swimming or on a ladder.

use glam::DVec3;
use imsim_core::movement::{CollisionWorld, MoveParams};
use std::sync::Mutex;

pub fn on() -> bool {
    crate::settings::get("Mantling") > 0.5
}

#[derive(Clone, Copy)]
struct Climb {
    from: DVec3,
    top: DVec3,
    to: DVec3,
    t: f64,
    dur: f64,
}

static CLIMB: Mutex<Option<Climb>> = Mutex::new(None);

/// Climbing right now.
pub fn active() -> bool {
    CLIMB.lock().map(|c| c.is_some()).unwrap_or(false)
}

pub fn cancel() {
    if let Ok(mut c) = CLIMB.lock() {
        *c = None;
    }
}

/// Highest ledge above the feet that can be climbed (units).
const REACH: f64 = 110.0;

/// Looks for a ledge in front (`fwd` flat, unit). Starts a climb and returns true when there is one.
pub fn try_start<W: CollisionWorld>(w: &W, loc: DVec3, p: &MoveParams, fwd: DVec3) -> bool {
    if !on() || active() || fwd.length_squared() < 0.5 {
        return false;
    }
    let r = p.radius;
    let h = p.height;
    let feet = loc.z - h;
    // Something in the way in front, above step height.
    let probe = loc + DVec3::new(0.0, 0.0, -h + p.max_step_height + 4.0);
    let block = w.trace(probe, probe + fwd * (r + 28.0), DVec3::ZERO);
    if block.fraction >= 1.0 {
        return false;
    }
    let wall_dist = (r + 28.0) * block.fraction;
    // The top of it: straight down from above, just past the face.
    let over = loc + fwd * (wall_dist + 10.0);
    let start = DVec3::new(over.x, over.y, feet + REACH + 4.0);
    let down = w.trace(start, DVec3::new(over.x, over.y, feet), DVec3::ZERO);
    if down.fraction <= 0.0 || down.fraction >= 1.0 || down.normal.z < 0.7 {
        return false;
    }
    let ledge = start.z + (feet - start.z) * down.fraction;
    let rise = ledge - feet;
    if rise <= p.max_step_height || rise > REACH {
        return false;
    }
    let ext = DVec3::new(r, r, h);
    // Room to rise in place, then to move over the edge and stand.
    let top = DVec3::new(loc.x, loc.y, ledge + h + 2.0);
    if w.trace(loc, top, ext).fraction < 1.0 {
        return false;
    }
    let to = DVec3::new(over.x, over.y, ledge + h + 2.0) + fwd * (r * 0.6);
    if w.trace(top, to, ext).fraction < 1.0 {
        return false;
    }
    let dur = 0.25 + 0.35 * (rise / REACH);
    if let Ok(mut c) = CLIMB.lock() {
        *c = Some(Climb { from: loc, top, to, t: 0.0, dur });
    }
    crate::sound::play_player("DeusExSounds.Player.MaleLand", 0.5, 400.0);
    true
}

/// Moves the climb on by `dt`: the new location, and whether it finished.
pub fn step(dt: f64) -> Option<(DVec3, bool)> {
    let Ok(mut c) = CLIMB.lock() else { return None };
    let cl = c.as_mut()?;
    cl.t = (cl.t + dt / cl.dur).min(1.0);
    let ease = |x: f64| x * x * (3.0 - 2.0 * x);
    // Two thirds of the time going up, then over.
    let pos = if cl.t < 0.66 {
        cl.from.lerp(cl.top, ease(cl.t / 0.66))
    } else {
        cl.top.lerp(cl.to, ease((cl.t - 0.66) / 0.34))
    };
    let done = cl.t >= 1.0;
    if done {
        *c = None;
    }
    Some((pos, done))
}
