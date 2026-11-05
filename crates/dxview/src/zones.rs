//! ZoneInfo rules in play: pain zones that switches turn on and off (ZoneInfo.Trigger toggles bPainZone when it has
//! DamagePerSec - the NSF HQ halon gas, the airfield's steam vent, the Area 51 fan), water currents (ZoneVelocity, through
//! actors::SwimWorld::zone_velocity) and pain for NPCs standing in a pain zone (ScriptedPawn.PainTimer).
//! The player's own pain timer is swim.rs (Pawn.PainTimer / FootZoneChange); it asks `pain()` here.

use bevy::prelude::*;
use glam::DVec3;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
struct State {
    level: String,
    /// bPainZone per BSP zone, as switched.
    pain: Vec<bool>,
    /// A zone was switched on: PainTimer runs at once for the player (ZoneInfo.Trigger -> EventInstigator.PainTimer).
    kick: bool,
    /// Zones switched since the map started (saved).
    toggled: Vec<usize>,
    /// Restored toggles waiting for the level.
    pending: Option<Vec<usize>>,
    /// Each NPC's PainTime (blocker -> seconds left).
    npc_timer: HashMap<usize, f64>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(lvl: &dx_level::Level, f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let s = g.get_or_insert_with(State::default);
    if s.level != lvl.name || s.pain.len() != lvl.zone_rules.len() {
        let pending = s.pending.take();
        *s = State {
            level: lvl.name.clone(),
            pain: lvl.zone_rules.iter().map(|r| r.pain).collect(),
            ..Default::default()
        };
        for z in pending.unwrap_or_default() {
            if let Some(p) = s.pain.get_mut(z) {
                *p = !*p;
                s.toggled.push(z);
            }
        }
    }
    f(s)
}

pub fn reset_level() {
    if let Ok(mut g) = STATE.lock() {
        let pending = g.as_mut().and_then(|s| s.pending.take());
        *g = Some(State { pending, ..Default::default() });
    }
}

/// The pain zone at BSP zone `z`, if it hurts now: (DamagePerSec, DamageType).
pub fn pain_zone(lvl: &dx_level::Level, z: i32) -> Option<(f32, String)> {
    let z = z.max(0) as usize;
    let r = lvl.zone_rules.get(z)?;
    if !with(lvl, |s| s.pain.get(z).copied().unwrap_or(false)) {
        return None;
    }
    Some((r.dps, r.damage_type.clone()))
}

/// The pain zone at `p`, if it hurts now.
pub fn pain(lvl: &dx_level::Level, p: DVec3) -> Option<(f32, String)> {
    pain_zone(lvl, lvl.collision.zone_at(p))
}

/// ZoneVelocity at `p`.
pub fn velocity(lvl: &dx_level::Level, p: DVec3) -> DVec3 {
    let z = lvl.collision.zone_at(p).max(0) as usize;
    lvl.zone_rules.get(z).map(|r| DVec3::new(r.velocity[0] as f64, r.velocity[1] as f64, r.velocity[2] as f64)).unwrap_or(DVec3::ZERO)
}

/// ZoneInfo.Trigger: zones tagged `name` with DamagePerSec toggle bPainZone.
pub fn event(lvl: &dx_level::Level, name: &str, on: bool) {
    if !on || name.is_empty() {
        return;
    }
    with(lvl, |s| {
        for (z, r) in lvl.zone_rules.iter().enumerate() {
            if r.dps != 0.0 && r.tag.eq_ignore_ascii_case(name) {
                s.pain[z] = !s.pain[z];
                if s.pain[z] {
                    s.kick = true;
                }
                if let Some(i) = s.toggled.iter().position(|&t| t == z) {
                    s.toggled.remove(i);
                } else {
                    s.toggled.push(z);
                }
                println!("ZONE {z} ({}) pain {}", r.tag, if s.pain[z] { "on" } else { "off" });
            }
        }
    });
}

/// A pain zone was just switched on (swim.rs restarts the player's PainTimer).
pub fn take_kick(lvl: &dx_level::Level) -> bool {
    with(lvl, |s| std::mem::take(&mut s.kick))
}

/// Zones switched from how the map left them (save.rs).
pub fn toggled() -> Vec<usize> {
    let g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let mut v = g.as_ref().map(|s| s.toggled.clone()).unwrap_or_default();
    v.sort();
    v
}

pub fn set_toggled(v: &[usize]) {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let s = g.get_or_insert_with(State::default);
    // Applied when the level is next looked at (it may not be built yet).
    s.level.clear();
    s.pending = Some(v.to_vec());
}

/// ScriptedPawn.PainTimer for NPCs: every second in a pain zone, DamagePerSec x depth (0.3 feet, 0.3 body, 0.4
/// head) of the most covering zone's type. The pawn's ReducedDamageType is filtered by damage.rs.
pub fn npc_pain_system(
    time: Res<Time>,
    level: Res<crate::LevelRes>,
    blockers: Res<crate::actors::Blockers>,
    npcs: Query<&crate::ai::Npc>,
    mut pawn_hits: ResMut<crate::damage::PawnHits>,
) {
    let _prof = crate::prof::scope("zones::npc_pain_system");
    let lvl = &level.0;
    if !lvl.zone_rules.iter().any(|r| r.dps != 0.0) {
        return;
    }
    let dt = time.delta_secs_f64().min(0.1);
    with(lvl, |s| {
        if !s.pain.iter().any(|&p| p) {
            s.npc_timer.clear();
            return;
        }
        for n in npcs.iter() {
            if n.state == crate::ai::NpcState::Dead {
                s.npc_timer.remove(&n.blocker);
                continue;
            }
            let Some(c) = blockers.list.get(n.blocker).filter(|c| c.radius > 0.0) else { continue };
            let zone_of = |p: DVec3| {
                let z = lvl.collision.zone_at(p).max(0) as usize;
                s.pain.get(z).copied().unwrap_or(false).then_some(z)
            };
            let feet = zone_of(c.center - DVec3::Z * (c.height - 1.0));
            let body = zone_of(c.center);
            let head = zone_of(c.center + DVec3::Z * c.height * 0.85);
            let Some(z) = head.or(body).or(feet) else {
                s.npc_timer.remove(&n.blocker);
                continue;
            };
            // FootZoneChange: entering starts the timer almost at once.
            let t = s.npc_timer.entry(n.blocker).or_insert(0.01);
            *t -= dt;
            if *t > 0.0 {
                continue;
            }
            *t = 1.0;
            let r = &lvl.zone_rules[z];
            let depth = 0.3 * feet.is_some() as u8 as f32 + 0.3 * body.is_some() as u8 as f32 + 0.4 * head.is_some() as u8 as f32;
            let dmg = (r.dps * depth).floor();
            if dmg > 0.0 {
                pawn_hits.0.push(crate::damage::PawnHit {
                    blocker: n.blocker,
                    damage: dmg,
                    damage_type: crate::util::intern(&r.damage_type),
                    hit_loc: c.center,
                    // TakeDamage(..., None, ...): no instigator. Its own blocker keeps it from blaming the player.
                    attacker: Some(n.blocker),
                    attacker_pos: c.center,
                });
            }
        }
    });
}
