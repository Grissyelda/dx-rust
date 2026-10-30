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
    crate::weapons::spread_dir(base, right, up, accuracy * 0.04, range, seed)
}

/// A projectile's direction (mini-crossbow, rockets) the same way.
pub fn projectile_dir(forward: DVec3, right: DVec3, up: DVec3, accuracy: f64, seed: &mut u64) -> DVec3 {
    if !on() {
        return crate::projectiles::projectile_dir(forward, accuracy, seed);
    }
    let s = sway();
    let base = (forward * 1024.0 + right * (s.x as f64 * 1000.0) + up * (s.y as f64 * 1000.0)).normalize();
    crate::projectiles::projectile_dir(base, accuracy * 0.04, seed)
}

/// A weapon you can aim down the sights of: a gun or launcher (not melee, thrown, a tool or anything consumed).
const NO_SIGHTS: &[&str] = &[
    "WeaponProd",
    "WeaponGEPGun",
    "WeaponLAW",
    "WeaponShuriken",
    "WeaponLAM",
    "WeaponGasGrenade",
    "WeaponEMPGrenade",
    "WeaponNanoVirusGrenade",
];

pub fn sightable(h: &crate::weapons::Held) -> bool {
    let st = &h.stats;
    !st.hand_to_hand
        && !st.item
        && !st.has_scope
        && st.class.starts_with("Weapon")
        // No sights to look down: the riot prod, the shoulder-fired launchers, thrown weapons.
        && !NO_SIGHTS.iter().any(|c| st.class.eq_ignore_ascii_case(c))
        && st.ammo_class.as_deref().is_some_and(|a| !a.to_ascii_lowercase().contains("ammonone"))
}

/// How much the aim sways for a weapon skill level (Untrained .. Master): a master holds a rifle still.
fn skill_sway(level: usize) -> f32 {
    [1.0, 0.55, 0.25, 0.06][level.min(3)]
}

/// The view model's turn so the barrel points where the shot will go (Bevy camera space).
pub fn view_rotation() -> Quat {
    let s = sway();
    let k = 1000.0 / 1024.0;
    Quat::from_rotation_y(-(s.x * k).atan()) * Quat::from_rotation_x((s.y * k).atan())
}

/// Each frame: the point of aim drifts - a slow breath and a little hand tremor - by how well you know the
/// weapon (its skill level), more on the move, less crouched, standing still a while or down the sights; the
/// scope key held with a gun brings the sights up.
#[allow(clippy::too_many_arguments)]
pub fn aim_mode_system(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    aim: Res<crate::hud::Aim>,
    weapons: Res<crate::weapons::Weapons>,
    zoom: Res<crate::weapons::Zoom>,
    player: Res<crate::Player>,
    ph: Res<crate::ai::PlayerHealth>,
) {
    let dt = time.delta_secs().min(0.1);
    let Ok(mut st) = STATE.lock() else { return };
    if !on() {
        *st = State::default();
        return;
    }
    st.t += dt;
    let t = st.t;
    let held = weapons.held.as_ref().filter(|h| sightable(h));
    let want = held.is_some()
        && zoom.0.is_none()
        && (keys.pressed(KeyCode::KeyZ) || keys.pressed(KeyCode::BracketLeft) || std::env::var_os("DXVIEW_FORCE_ADS").is_some())
        && !crate::inventory_screen::screen_open()
        && !crate::consys::third_person();
    if held.is_none() {
        // Melee, tools and things you use never come up to the eye.
        st.ads = 0.0;
        st.sway = Vec2::ZERO;
        return;
    }
    let h = held.unwrap();
    let target = if want { 1.0 } else { 0.0 };
    // About a sixth of a second up or down.
    st.ads += (target - st.ads).clamp(-dt * 6.0, dt * 6.0);
    let level = h.stats.governing_skill.as_deref().map_or(0, crate::abilities::skill_level);
    let speed = player.body.velocity.truncate().length() as f32;
    let mut reach = 0.03 * skill_sway(level);
    reach *= 1.0 + (speed / 300.0).min(1.0);
    if player.crouched {
        reach *= 0.75;
    }
    reach *= 1.0 - (aim.standing / 4.0).clamp(0.0, 0.35);
    reach *= 1.0 - 0.5 * st.ads;
    // Wounded arms shake.
    reach += 0.02 * ph.accuracy_penalty_with(false).max(0.0);
    // A slow breath (rising and falling) with a lazy figure-eight, and a faint tremor on top.
    let wob = Vec2::new(
        (t * 0.47).sin() * 0.7 + (t * 1.13 + 1.3).sin() * 0.2 + (t * 3.1).sin() * 0.06,
        (t * 0.94 + 0.5).sin() * 0.35 + (t * 0.31).sin() * 0.5 + (t * 2.7 + 0.7).sin() * 0.06,
    );
    let goal = wob * reach;
    let k = 1.0 - (-dt * 6.0).exp();
    st.sway = st.sway + (goal - st.sway) * k;
}
