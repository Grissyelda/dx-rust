//! The Nameless Mod's two arena powerups (the Mission 23 boss fight drops them in with RevealPowerups):
//! QuadDamage (a spinning logo) and RingofShadows (a spinning ring). Both bob on their pivot, and touching,
//! bumping or frobbing one hands the player its power for 30 seconds and removes it.
//!
//! Quad damage multiplies everything the player's weapons do by 4 (weapon damage, the projectiles they launch and
//! thrown grenades). The ring makes the player invisible to every pawn and device that looks for him. While either
//! runs the screen carries a faint tint (blue for the quad, grey for the ring), as the player's per-tick flash does.
//!
//! Deviation: the "enabled" line goes to the message log instead of fading phrases scattered over the screen.

use crate::actors::{Blockers, Deco};
use crate::LevelRes;
use bevy::prelude::*;
use std::sync::Mutex;
use unreal_pkg::props::{find, Value};

/// How long one powerup lasts, seconds.
const DURATION: f32 = 30.0;

#[derive(Default, Clone, Copy)]
struct Powers {
    /// Seconds of quad damage left (0 = off).
    quad: f32,
    /// Seconds of the ring left.
    shadow: f32,
}

static POWERS: Mutex<Powers> = Mutex::new(Powers { quad: 0.0, shadow: 0.0 });

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Quad,
    Shadow,
}

fn kind_of(class: &str) -> Option<Kind> {
    if unreal_pkg::tnm::active().is_none() {
        return None;
    }
    if class.eq_ignore_ascii_case("QuadDamage") || crate::classtree::is_a(class, "QuadDamage") {
        Some(Kind::Quad)
    } else if class.eq_ignore_ascii_case("RingofShadows") || crate::classtree::is_a(class, "RingofShadows") {
        Some(Kind::Shadow)
    } else {
        None
    }
}

/// A powerup decoration (frobbable, picked up on touch).
pub fn is_powerup(class: &str) -> bool {
    kind_of(class).is_some()
}

/// Damage multiplier for everything the player's weapons do.
pub fn damage_mult() -> f32 {
    if POWERS.lock().is_ok_and(|p| p.quad > 0.0) {
        4.0
    } else {
        1.0
    }
}

/// The ring is on: no pawn, camera or turret sees the player.
pub fn shadowed() -> bool {
    POWERS.lock().is_ok_and(|p| p.shadow > 0.0)
}

/// New game / load: both powers off (they are only ever picked up mid-fight).
pub fn clear() {
    if let Ok(mut p) = POWERS.lock() {
        *p = Powers::default();
    }
}

/// Picks up the powerup at level actor `actor`: its power starts (or starts over) and the decoration goes.
pub fn take(level: &dx_level::Level, actor: usize) {
    let Some(a) = level.actors.get(actor) else { return };
    let Some(k) = kind_of(&a.class) else { return };
    if crate::scriptworld::is_hidden(actor) {
        return;
    }
    if let Ok(mut p) = POWERS.lock() {
        match k {
            Kind::Quad => p.quad = DURATION,
            Kind::Shadow => p.shadow = DURATION,
        }
    }
    if let Some(Value::Str(s)) = find(&a.props, "strEnabled") {
        crate::classic_hud::client_message(s);
    }
    // Hidden at once so a touch and a frob in the same frame can't both take it.
    crate::scriptworld::mark_hidden(actor, true);
    crate::scriptworld::push(crate::scriptworld::ActorReq::Destroy(crate::scriptworld::ActorSel::tag(&format!(
        "#actor{actor}"
    ))));
    println!("TNM powerup {} taken", a.name);
}

/// Bobbing (PrePivot.Z = baseHeight + Amplitude x sin(2 x (time mod 360))), touch / bump pickup, the timers and
/// the screen tint.
#[allow(clippy::too_many_arguments)]
pub fn powerup_system(
    time: Res<Time>,
    level: Res<LevelRes>,
    blockers: Res<Blockers>,
    player: Res<crate::Player>,
    mut ph: ResMut<crate::ai::PlayerHealth>,
    mut decos: Query<(&Deco, &mut Transform)>,
    mut list: Local<(u64, Vec<(usize, f64, f64)>)>,
) {
    let lvl = &level.0;
    let dt = time.delta_secs();
    if let Ok(mut p) = POWERS.lock() {
        if p.quad > 0.0 || p.shadow > 0.0 {
            if ph.dead {
                *p = Powers::default();
            } else {
                // The original flashes every tick while a power runs: a faint held tint.
                if p.quad > 0.0 {
                    ph.flash = ph.flash.max(0.08);
                    ph.flash_color = [0.0, 0.0, 0.5];
                } else {
                    ph.flash = ph.flash.max(0.06);
                    ph.flash_color = [0.25, 0.25, 0.25];
                }
                p.quad = (p.quad - dt).max(0.0);
                p.shadow = (p.shadow - dt).max(0.0);
            }
        }
    }
    if crate::loader::new_map(&mut list.0) {
        list.1.clear();
        if unreal_pkg::tnm::active().is_some() {
            for (i, a) in lvl.actors.iter().enumerate() {
                if is_powerup(&a.class) {
                    let f = |n: &str, d: f64| match find(&a.props, n) {
                        Some(Value::Int(v)) => *v as f64,
                        Some(Value::Float(v)) => *v as f64,
                        _ => d,
                    };
                    list.1.push((i, f("baseHeight", 10.0), f("Amplitude", 27.0)));
                }
            }
        }
    }
    if list.1.is_empty() {
        return;
    }
    let t = time.elapsed_secs_f64();
    let bob = |base: f64, amp: f64| base + amp * ((t % 360.0) * 2.0).sin();
    // Bob the drawn mesh.
    for (d, mut tf) in &mut decos {
        let Some(ma) = lvl.mesh_actors.get(d.mesh_actor) else { continue };
        let Some(&(_, base, amp)) = list.1.iter().find(|(i, _, _)| *i == ma.actor) else { continue };
        let draw = d.body.location + d.pre + bevy::math::DVec3::new(0.0, 0.0, bob(base, amp));
        let v = Vec3::from(dx_level::ue_to_bevy([draw.x as f32, draw.y as f32, draw.z as f32]));
        if tf.translation != v {
            tf.translation = v;
        }
    }
    if ph.dead {
        return;
    }
    // Touch / Bump: the player's cylinder meeting the powerup's.
    let p = player.body.location;
    let (pr, phh) = (player.params.radius, player.params.height);
    let mut hit = Vec::new();
    for c in &blockers.list {
        if c.radius <= 0.0 || !list.1.iter().any(|(i, _, _)| *i == c.actor) {
            continue;
        }
        let d = (c.center - p).truncate().length();
        if d <= c.radius + pr + 2.0 && (c.center.z - p.z).abs() <= c.height + phh + 2.0 {
            hit.push(c.actor);
        }
    }
    for a in hit {
        take(lvl, a);
    }
}
