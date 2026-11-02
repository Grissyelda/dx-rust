//! Enhancement "Combat HUD" (setting HudFade, ours, in the Modern preset): out of combat the crosshair and the HUD's
//! status windows (health, compass, ammo, belt, active augs and items) fade away, leaving the view clear; they come
//! back at once when it matters - an enemy fighting you nearby (the combat-music test), aiming down the sights or
//! through a scope, the crosshair on an enemy, a shot fired, damage taken, low health, or a belt / weapon change -
//! and stay a few seconds after. The message log, damage arrows and every screen stay as they are.

use bevy::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering};

pub fn on() -> bool {
    crate::settings::get("HudFade") > 0.5
}

/// Enemies attacking the player within 1600 units (sound.rs's UpdateDynamicMusic count).
pub static ATTACKERS: AtomicU32 = AtomicU32::new(0);
/// The HUD's opacity now (f32 bits; 1 = fully shown).
static ALPHA: AtomicU32 = AtomicU32::new(0x3f80_0000);
/// Seconds since the last event that brings the HUD back (f32 bits).
static SINCE: AtomicU32 = AtomicU32::new(0);

/// How long the HUD stays after the last reason to show it, and how fast it goes / comes.
const HOLD: f32 = 4.0;
const FADE_OUT: f32 = 1.2;
const FADE_IN: f32 = 0.12;

/// Something that brings the HUD back: a shot, a hit taken, a belt or weapon change.
pub fn note() {
    SINCE.store(0f32.to_bits(), Ordering::Relaxed);
}

/// The status windows' and crosshair's opacity (1 when the enhancement is off).
pub fn alpha() -> f32 {
    if !on() {
        return 1.0;
    }
    f32::from_bits(ALPHA.load(Ordering::Relaxed))
}

pub fn hud_fade_system(
    time: Res<Time<Real>>,
    zoom: Res<crate::weapons::Zoom>,
    ph: Res<crate::ai::PlayerHealth>,
) {
    let dt = time.delta_secs().min(0.1);
    let mut since = f32::from_bits(SINCE.load(Ordering::Relaxed)) + dt;
    let health = ph.parts.iter().sum::<f32>() / (ph.parts.len().max(1) as f32 * 100.0);
    let urgent = ATTACKERS.load(Ordering::Relaxed) > 0
        || crate::aimmode::ads() > 0.0
        || zoom.0.is_some()
        || crate::hud::on_enemy()
        || health < 0.5
        || ph.dead;
    if urgent {
        since = 0.0;
    }
    SINCE.store(since.to_bits(), Ordering::Relaxed);
    let want = if since < HOLD { 1.0 } else { 0.0 };
    let mut a = f32::from_bits(ALPHA.load(Ordering::Relaxed));
    a = if want > a { (a + dt / FADE_IN).min(1.0) } else { (a - dt / FADE_OUT).max(0.0) };
    ALPHA.store(a.to_bits(), Ordering::Relaxed);
}
