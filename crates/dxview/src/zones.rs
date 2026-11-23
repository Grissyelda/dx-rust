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
    if let Ok(mut q) = VEL_SET.lock() {
        q.clear();
    }
    if let Ok(mut q) = OXY_SET.lock() {
        q.clear();
    }
    if let Ok(mut q) = OXY_ASK.lock() {
        q.clear();
    }
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
    let Some(r) = lvl.zone_rules.get(z) else { return DVec3::ZERO };
    if let Ok(v) = VEL_SET.lock() {
        if let Some((_, v)) = v.iter().find(|(t, _)| !r.tag.is_empty() && t.eq_ignore_ascii_case(&r.tag)) {
            return *v;
        }
    }
    DVec3::new(r.velocity[0] as f64, r.velocity[1] as f64, r.velocity[2] as f64)
}

/// Zones whose ZoneVelocity a script changed, by Tag (None = back to the map's value).
static VEL_SET: std::sync::Mutex<Vec<(String, DVec3)>> = std::sync::Mutex::new(Vec::new());

/// ZoneVelocity of the zones tagged `tag` (a script's sewer flush); None puts the map's value back.
pub fn set_velocity(tag: &str, v: Option<DVec3>) {
    if let Ok(mut q) = VEL_SET.lock() {
        q.retain(|(t, _)| !t.eq_ignore_ascii_case(tag));
        if let Some(v) = v {
            q.push((tag.to_string(), v));
        }
    }
}

/// Zones whose air a script changed: (zone index, no oxygen). An OxygenZoneTrigger toggles or sets the zones
/// tagged with its Event; the air pumps give every zone air.
static OXY_SET: std::sync::Mutex<Vec<(usize, bool)>> = std::sync::Mutex::new(Vec::new());

fn airless_zone(lvl: &dx_level::Level, z: usize) -> bool {
    let base = lvl.zone_rules.get(z).is_some_and(|r| r.no_oxygen);
    OXY_SET.lock().ok().and_then(|q| q.iter().rev().find(|(i, _)| *i == z).map(|(_, v)| *v)).unwrap_or(base)
}

/// No air to breathe at this point (The Nameless Mod's tnmzoneinfo bNoOxygen).
pub fn no_oxygen(lvl: &dx_level::Level, p: DVec3) -> bool {
    let z = lvl.collision.zone_at(p).max(0) as usize;
    airless_zone(lvl, z)
}

/// OxygenZoneTrigger.Trigger: the zones tagged `tag` lose or get air (toggled, or set to `value`).
pub fn set_oxygen(lvl: &dx_level::Level, tag: &str, toggle: bool, value: bool) {
    for (z, r) in lvl.zone_rules.iter().enumerate() {
        if !tag.is_empty() && r.tag.eq_ignore_ascii_case(tag) {
            let now = airless_zone(lvl, z);
            let v = if toggle { !now } else { value };
            if let Ok(mut q) = OXY_SET.lock() {
                q.retain(|(i, _)| *i != z);
                q.push((z, v));
            }
        }
    }
}

/// Some zone tagged `tag` has no air.
pub fn tag_airless(lvl: &dx_level::Level, tag: &str) -> bool {
    lvl.zone_rules.iter().enumerate().any(|(z, r)| r.tag.eq_ignore_ascii_case(tag) && airless_zone(lvl, z))
}

/// Air everywhere (The Nameless Mod's EnableAirPumps).
pub fn air_everywhere(lvl: &dx_level::Level) {
    if let Ok(mut q) = OXY_SET.lock() {
        q.clear();
        q.extend((0..lvl.zone_rules.len()).map(|z| (z, false)));
    }
}

/// Oxygen toggles requested by triggers (they don't hold the level): (tag, toggle, value).
static OXY_ASK: std::sync::Mutex<Vec<(String, bool, bool)>> = std::sync::Mutex::new(Vec::new());

pub fn ask_oxygen(tag: &str, toggle: bool, value: bool) {
    if let Ok(mut q) = OXY_ASK.lock() {
        q.push((tag.to_string(), toggle, value));
    }
}

/// Applies queued oxygen toggles (called from the swim system, which holds the level).
pub fn apply_oxygen_asks(lvl: &dx_level::Level) {
    let asks: Vec<(String, bool, bool)> = OXY_ASK.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    for (t, toggle, value) in asks {
        set_oxygen(lvl, &t, toggle, value);
    }
}

/// The Tag of the zone a point is in.
pub fn zone_tag(lvl: &dx_level::Level, p: DVec3) -> String {
    let z = lvl.collision.zone_at(p).max(0) as usize;
    lvl.zone_rules.get(z).map(|r| r.tag.clone()).unwrap_or_default()
}

/// ZoneInfo.Trigger: zones tagged `name` with DamagePerSec toggle bPainZone.
pub fn event(lvl: &dx_level::Level, name: &str, on: bool) {
    if crate::tnmmode::active() {
        tnm_event(lvl, name, on);
    }
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
                pawn_hits.0.push(crate::damage::PawnHit { world: false,
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

/// BarrelFire.DamageOther: bumping a burning barrel or standing on it burns the player, 5 'Burned' at most once a
/// second per barrel (ScriptedPawns aren't hurt).
pub fn barrel_fire_system(
    time: Res<Time>,
    level: Res<crate::LevelRes>,
    blockers: Res<crate::actors::Blockers>,
    player: Res<crate::Player>,
    mut ph: ResMut<crate::ai::PlayerHealth>,
    mut last: Local<(u64, Vec<(usize, f64)>, Vec<usize>)>,
) {
    let lvl = &level.0;
    if crate::loader::new_map(&mut last.0) {
        last.1.clear();
        last.2 = blockers
            .list
            .iter()
            .enumerate()
            .filter(|(_, c)| lvl.actors.get(c.actor).is_some_and(|a| a.class.eq_ignore_ascii_case("BarrelFire")))
            .map(|(i, _)| i)
            .collect();
    }
    if last.2.is_empty() || ph.dead {
        return;
    }
    let now = time.elapsed_secs_f64();
    let p = player.body.location;
    let (pr, phh) = (player.params.radius, player.params.height);
    let v = player.body.velocity;
    for k in 0..last.2.len() {
        let b = last.2[k];
        let Some(c) = blockers.list.get(b).filter(|c| c.radius > 0.0) else { continue };
        let d = (p - c.center).truncate();
        let gap = d.length() - (pr + c.radius);
        let dz = p.z - c.center.z;
        // SupportActor: standing on its top.
        let on_top = d.length() < c.radius + pr && (dz - (phh + c.height)).abs() < 4.0;
        // Bump: pressing into its side.
        let bump = gap < 1.5 && dz.abs() < phh + c.height && v.truncate().dot(-d.normalize_or_zero()) > 1.0;
        if !(on_top || bump) {
            continue;
        }
        let i = match last.1.iter().position(|(x, _)| *x == b) {
            Some(i) => i,
            None => {
                last.1.push((b, f64::NEG_INFINITY));
                last.1.len() - 1
            }
        };
        if now - last.1[i].1 >= 1.0 {
            last.1[i].1 = now;
            let off = c.center - p;
            ph.damage(5.0, "Burned", DVec3::new(0.0, 0.0, off.z));
        }
    }
}

// ---- The Nameless Mod's zones that change under the player: altergravzone (gravtrigger / Trigger step through its
// gravs[]), InertialMover's fan zone (ZoneGravity pushed each tick) and WaterToggleZone (Trigger floods or drains).
// uc: altergravzone.changegrav, gravtrigger.Trigger, WaterToggleZone.Trigger, InertialMover.Tick

#[derive(Default)]
struct Live {
    level: String,
    /// ZoneGravity per zone where it isn't the map's any more.
    gravity: HashMap<usize, DVec3>,
    /// altergravzone.gravnum per zone.
    gravnum: HashMap<usize, i32>,
    /// bWaterZone per zone where a WaterToggleZone switched it.
    water: HashMap<usize, bool>,
}

static LIVE: Mutex<Option<Live>> = Mutex::new(None);
/// Any override exists (the common case, none, costs one atomic read).
static LIVE_ANY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn live<R>(lvl: &dx_level::Level, f: impl FnOnce(&mut Live) -> R) -> R {
    let mut g = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    let l = g.get_or_insert_with(Live::default);
    if l.level != lvl.name {
        *l = Live { level: lvl.name.clone(), ..Default::default() };
        // altergravzone.prebeginplay: changegrav(gravnum).
        for (z, r) in lvl.zone_rules.iter().enumerate() {
            if r.class.eq_ignore_ascii_case("altergravzone") {
                let n = match unreal_pkg::props::find(&r.props, "gravnum") {
                    Some(unreal_pkg::props::Value::Int(v)) => *v,
                    _ => 0,
                };
                l.gravnum.insert(z, n);
                if let Some(g) = grav_of(r, n) {
                    l.gravity.insert(z, g);
                }
            }
        }
        LIVE_ANY.store(!l.gravity.is_empty(), std::sync::atomic::Ordering::Relaxed);
    }
    let r = f(l);
    LIVE_ANY.store(!l.gravity.is_empty() || !l.water.is_empty(), std::sync::atomic::Ordering::Relaxed);
    r
}

/// gravs[n].grav of an altergravzone (the struct is a vector then a bool).
fn grav_of(r: &dx_level::ZoneRule, n: i32) -> Option<DVec3> {
    r.props.iter().find(|p| p.name.eq_ignore_ascii_case("gravs") && p.index as i32 == n).and_then(|p| match &p.value {
        unreal_pkg::props::Value::Struct(_, b) if b.len() >= 12 => {
            let f = |k: usize| f32::from_le_bytes([b[k], b[k + 1], b[k + 2], b[k + 3]]) as f64;
            Some(DVec3::new(f(0), f(4), f(8)))
        }
        _ => None,
    })
}

/// Build the per-map state (altergravzones start on their gravnum).
pub fn live_init(lvl: &dx_level::Level) {
    live(lvl, |_| ());
}

/// ZoneGravity at `p` when a zone's gravity isn't the usual one.
pub fn gravity_at(lvl: &dx_level::Level, p: DVec3) -> Option<DVec3> {
    let z = lvl.collision.zone_at(p).max(0) as usize;
    if LIVE_ANY.load(std::sync::atomic::Ordering::Relaxed) {
        if let Some(g) = live(lvl, |l| l.gravity.get(&z).copied()) {
            return Some(g);
        }
    }
    let r = lvl.zone_rules.get(z)?;
    let g = r.gravity;
    (!r.class.is_empty() && g != [0.0, 0.0, 0.0] && g != [0.0, 0.0, -950.0]).then(|| DVec3::new(g[0] as f64, g[1] as f64, g[2] as f64))
}

/// A WaterToggleZone's present bWaterZone for the zone at `p` (None = as the map has it).
pub fn water_override(lvl: &dx_level::Level, zone: usize) -> Option<bool> {
    if !LIVE_ANY.load(std::sync::atomic::Ordering::Relaxed) {
        return None;
    }
    live(lvl, |l| l.water.get(&zone).copied())
}

/// gravtrigger: the altergravzones tagged `tag` go to gravs[num]. `step`: Trigger (+1) / UnTrigger (-1) instead.
pub fn change_grav(lvl: &dx_level::Level, tag: &str, num: Option<i32>, step: i32) {
    live(lvl, |l| {
        for (z, r) in lvl.zone_rules.iter().enumerate() {
            if !r.class.eq_ignore_ascii_case("altergravzone") || !r.tag.eq_ignore_ascii_case(tag) {
                continue;
            }
            let n = num.unwrap_or_else(|| l.gravnum.get(&z).copied().unwrap_or(0) + step).clamp(0, 9);
            l.gravnum.insert(z, n);
            if let Some(g) = grav_of(r, n) {
                l.gravity.insert(z, g);
            }
        }
    });
}

/// InertialMover.Tick on its fan zone: ZoneGravity += change, clamped; ZoneVelocity = it without Z.
pub fn push_gravity(lvl: &dx_level::Level, tag: &str, change: DVec3, min: DVec3, max: DVec3) {
    live(lvl, |l| {
        for (z, r) in lvl.zone_rules.iter().enumerate() {
            if r.tag.is_empty() || !r.tag.eq_ignore_ascii_case(tag) {
                continue;
            }
            let g0 = l.gravity.get(&z).copied().unwrap_or(DVec3::new(r.gravity[0] as f64, r.gravity[1] as f64, r.gravity[2] as f64));
            let g = (g0 + change).clamp(min, max);
            l.gravity.insert(z, g);
        }
    });
}

/// Events for the mod's zones: an altergravzone steps its gravity, a WaterToggleZone floods or drains.
pub fn tnm_event(lvl: &dx_level::Level, name: &str, on: bool) {
    if name.is_empty() || !lvl.zone_rules.iter().any(|r| !r.props.is_empty() && r.tag.eq_ignore_ascii_case(name)) {
        return;
    }
    change_grav(lvl, name, None, if on { 1 } else { -1 });
    if on {
        live(lvl, |l| {
            for (z, r) in lvl.zone_rules.iter().enumerate() {
                if r.class.eq_ignore_ascii_case("WaterToggleZone") && r.tag.eq_ignore_ascii_case(name) {
                    let was = l.water.get(&z).copied().unwrap_or(lvl.zone_water.get(z).is_some_and(|w| w.is_some()));
                    l.water.insert(z, !was);
                    println!("ZONE {z} ({}) water {}", r.tag, !was);
                }
            }
        });
    }
}
