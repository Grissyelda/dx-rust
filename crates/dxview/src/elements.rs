//! Enhancement (Elemental Reactions, ROADMAP v2 #28). Ours, off by default.
//!
//! - Electricity through water: a shock ('Stunned' from the riot prod, 'Shocked') to someone standing in water runs
//!   through it to everyone else in that water within 600 units (you included), weaker with distance, with sparks.
//! - Fire spreads by touch: someone burning sets alight whoever they bump into (half the time each second).
//! - Water puts fire out: whoever is burning and wades into water stops burning.

use crate::actors::Blockers;
use crate::ai::{Npc, PlayerHealth};
use crate::damage::{PawnHit, PawnHits};
use crate::{LevelRes, Player};
use bevy::prelude::*;
use glam::DVec3;

pub fn on() -> bool {
    crate::settings::get("Elements") > 0.5
}

const SHOCK_RANGE: f64 = 600.0;

fn electric(t: &str) -> bool {
    matches!(t, "Stunned" | "Shocked")
}

/// Ankle-deep counts: a point just above the bottom of the cylinder is under water.
fn wading(level: &dx_level::Level, center: DVec3, half_height: f64) -> bool {
    crate::actors::water_at(level, center - DVec3::new(0.0, 0.0, half_height - 6.0)).is_some()
}

/// Before the AI takes this frame's hits: shocks in water spread.
pub fn shock_system(
    level: Res<LevelRes>,
    blockers: Res<Blockers>,
    player: Res<Player>,
    mut hits: ResMut<PawnHits>,
    mut ph: ResMut<PlayerHealth>,
    npcs: Query<&Npc>,
) {
    if !on() || hits.0.is_empty() {
        return;
    }
    let lvl = &level.0;
    let mut extra: Vec<PawnHit> = Vec::new();
    let mut player_dmg = 0.0f32;
    for h in hits.0.iter().filter(|h| electric(h.damage_type)) {
        let Some(c) = blockers.list.get(h.blocker) else { continue };
        if !wading(lvl, c.center, c.height) {
            continue;
        }
        crate::fx::sparks(c.center - DVec3::new(0.0, 0.0, c.height - 4.0), 14, 220.0);
        for n in &npcs {
            if n.blocker == h.blocker || extra.iter().any(|e| e.blocker == n.blocker) {
                continue;
            }
            let Some(o) = blockers.list.get(n.blocker) else { continue };
            let d = o.center.distance(c.center);
            if d > SHOCK_RANGE || !wading(lvl, o.center, o.height) {
                continue;
            }
            let k = (1.0 - d / SHOCK_RANGE) as f32 * 0.7 + 0.3;
            crate::fx::sparks(o.center - DVec3::new(0.0, 0.0, o.height - 4.0), 10, 180.0);
            extra.push(PawnHit {
                blocker: n.blocker,
                damage: h.damage * k,
                damage_type: h.damage_type,
                hit_loc: o.center,
                attacker: h.attacker,
                attacker_pos: h.attacker_pos,
            });
        }
        let pc = player.body.location;
        let d = pc.distance(c.center);
        if d <= SHOCK_RANGE && wading(lvl, pc, player.params.height) && h.attacker.is_some() {
            player_dmg = player_dmg.max(h.damage * ((1.0 - d / SHOCK_RANGE) as f32 * 0.7 + 0.3));
        }
    }
    hits.0.extend(extra);
    if player_dmg > 0.0 {
        crate::fx::sparks(player.body.location - DVec3::new(0.0, 0.0, player.params.height - 4.0), 14, 200.0);
        ph.damage(player_dmg, "Shocked", DVec3::ZERO);
    }
}

/// After the AI: fire spreads by touch and goes out in water.
pub fn fire_system(
    time: Res<Time>,
    level: Res<LevelRes>,
    blockers: Res<Blockers>,
    player: Res<Player>,
    mut hits: ResMut<PawnHits>,
    mut ph: ResMut<PlayerHealth>,
    mut npcs: Query<&mut Npc>,
    mut seed: Local<u64>,
    mut tick: Local<f32>,
) {
    if !on() {
        return;
    }
    let lvl = &level.0;
    // Water puts out whoever burns in it.
    for mut n in &mut npcs {
        if n.on_fire.is_some() {
            if let Some(c) = blockers.list.get(n.blocker) {
                if wading(lvl, c.center, c.height) {
                    n.on_fire = None;
                }
            }
        }
    }
    if ph.on_fire.is_some() && wading(lvl, player.body.location, player.params.height) {
        ph.on_fire = None;
    }
    *tick += time.delta_secs();
    if *tick < 1.0 {
        return;
    }
    *tick = 0.0;
    if *seed == 0 {
        *seed = 0xF1AE_5EED;
    }
    let mut rnd = || {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((*seed >> 11) as f64) / (1u64 << 53) as f64
    };
    // Who is burning, and who is touching whom.
    let mut burning: Vec<(Option<usize>, DVec3, f64)> = Vec::new();
    for n in &npcs {
        if n.on_fire.is_some() {
            if let Some(c) = blockers.list.get(n.blocker) {
                burning.push((Some(n.blocker), c.center, c.radius));
            }
        }
    }
    if ph.on_fire.is_some() {
        burning.push((None, player.body.location, player.params.radius));
    }
    for (src, at, r) in burning {
        for n in &npcs {
            if Some(n.blocker) == src || n.on_fire.is_some() {
                continue;
            }
            let Some(c) = blockers.list.get(n.blocker) else { continue };
            if c.center.distance(at) <= r + c.radius + 10.0 && rnd() < 0.5 {
                hits.0.push(PawnHit {
                    blocker: n.blocker,
                    damage: 2.0,
                    damage_type: "Flamed",
                    hit_loc: c.center,
                    attacker: src,
                    attacker_pos: at,
                });
            }
        }
        if src.is_some()
            && ph.on_fire.is_none()
            && player.body.location.distance(at) <= r + player.params.radius + 10.0
            && rnd() < 0.5
        {
            ph.damage(2.0, "Flamed", DVec3::ZERO);
        }
    }
}
