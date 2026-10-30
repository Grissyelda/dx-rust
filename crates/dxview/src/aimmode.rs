//! Enhancement (Modern Aiming, ROADMAP v2 #25). Ours, off by default.
//!
//! - Sway instead of spread: the original throws each shot at a random point inside the accuracy cone
//!   (DeusExWeapon.TraceFire, `weapons::spread_dir`). Here the point of aim drifts smoothly about inside that same
//!   cone and the shot goes where it is, with only a little random scatter left; the crosshair sits on that point,
//!   so a steady hand (skill, standing still, the Targeting aug) is something you can see and wait for.
//! - Dynamic crosshair: the accuracy brackets always show with a ranged weapon out, opening and closing with it.
//! - Aim down sights: holding the scope key (Z / '[' / LT) with a weapon that has no scope raises it to the centre
//!   of the view, narrows the view a little, steadies the sway and slows you to a walk.

use bevy::prelude::*;
use glam::DVec3;
use std::sync::Mutex;

pub fn on() -> bool {
    crate::settings::get("ModernAim") > 0.5
}

#[derive(Clone, Copy, Default)]
struct State {
    /// The drifting point of aim, in TraceFire's spread units (x right, y up; 1 = the full accuracy-1 spread).
    sway: Vec2,
    /// 0 hip .. 1 down the sights.
    ads: f32,
    t: f32,
}

static STATE: Mutex<State> = Mutex::new(State { sway: Vec2::ZERO, ads: 0.0, t: 0.0 });

fn state() -> State {
    STATE.lock().map(|s| *s).unwrap_or_default()
}

/// How far down the sights (0..1).
pub fn ads() -> f32 {
    if on() {
        state().ads
    } else {
        0.0
    }
}

/// The point of aim's offset (TraceFire spread units), zero when off.
pub fn sway() -> Vec2 {
    if on() {
        state().sway
    } else {
        Vec2::ZERO
    }
}

/// The world view's field of view while aiming down the sights.
pub fn fov(fov: f32) -> f32 {
    fov * (1.0 - 0.18 * ads())
}

/// Ground speed while aiming down the sights.
pub fn speed_mult() -> f64 {
    1.0 - 0.45 * ads() as f64
}

/// Accuracy while aiming down the sights (TraceFire's currentAccuracy, lower is better).
pub fn accuracy(acc: f32) -> f32 {
    acc * (1.0 - 0.35 * ads())
}

/// Where a shot goes: the swaying point of aim plus a little scatter (the original: all scatter).
pub fn shot_dir(forward: DVec3, right: DVec3, up: DVec3, accuracy: f64, range: f64, seed: &mut u64) -> DVec3 {
    if !on() {
        return crate::weapons::spread_dir(forward, right, up, accuracy, range, seed);
    }
    let s = sway();
    let base = (forward * range.max(1024.0) + right * (s.x as f64 * 1000.0) + up * (s.y as f64 * 1000.0)).normalize();
    crate::weapons::spread_dir(base, right, up, accuracy * 0.15, range, seed)
}

/// A projectile's direction (mini-crossbow, rockets) the same way.
pub fn projectile_dir(forward: DVec3, right: DVec3, up: DVec3, accuracy: f64, seed: &mut u64) -> DVec3 {
    if !on() {
        return crate::projectiles::projectile_dir(forward, accuracy, seed);
    }
    let s = sway();
    let base = (forward * 1024.0 + right * (s.x as f64 * 1000.0) + up * (s.y as f64 * 1000.0)).normalize();
    crate::projectiles::projectile_dir(base, accuracy * 0.15, seed)
}

/// Each frame: the sway point drifts (two incommensurate wobbles per axis, a slow breath on top) within the
/// current accuracy; the scope key held with an unscoped ranged weapon brings the sights up.
pub fn aim_mode_system(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    aim: Res<crate::hud::Aim>,
    weapons: Res<crate::weapons::Weapons>,
    zoom: Res<crate::weapons::Zoom>,
) {
    let dt = time.delta_secs().min(0.1);
    let Ok(mut st) = STATE.lock() else { return };
    if !on() {
        *st = State::default();
        return;
    }
    st.t += dt;
    let t = st.t;
    let ranged = weapons
        .held
        .as_ref()
        .is_some_and(|h| !h.stats.hand_to_hand && !h.stats.item && !h.stats.has_scope && h.stats.class != "POVCorpse");
    let want = ranged
        && zoom.0.is_none()
        && (keys.pressed(KeyCode::KeyZ) || keys.pressed(KeyCode::BracketLeft) || std::env::var_os("DXVIEW_FORCE_ADS").is_some())
        && !crate::inventory_screen::screen_open()
        && !crate::consys::third_person();
    let target = if want { 1.0 } else { 0.0 };
    // About a sixth of a second up or down.
    st.ads += (target - st.ads).clamp(-dt * 6.0, dt * 6.0);
    // The cone the shot could go anywhere in is +-0.5 x accuracy; the drift keeps to most of it.
    let reach = aim.accuracy * 0.45;
    let wob = Vec2::new(
        (t * 0.83).sin() * 0.6 + (t * 1.97 + 1.3).sin() * 0.3 + (t * 0.31 + 0.4).sin() * 0.1,
        (t * 0.71 + 2.1).sin() * 0.55 + (t * 2.33 + 0.7).sin() * 0.25 + (t * 0.27).sin() * 0.2,
    );
    let goal = wob * reach;
    // Eased, so a steadier aim settles rather than snapping in.
    let k = (dt * 4.0).min(1.0);
    st.sway = st.sway + (goal - st.sway) * k;
}
