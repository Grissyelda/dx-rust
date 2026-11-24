//! Framing (MULTIPLAYER roadmap Phase 3, Modern; the room's "Framing" switch, needs the living level).
//!
//! Every player's weapons are registered to him: the ones he spawns with, and a weapon keeps its registration when it
//! is dropped and picked up by someone else (the item carries "RegisteredTo"). A player who dies drops the weapon in
//! his hands.
//!
//! The host's staff look about them (living.rs hands over what they see):
//! - a body of the staff (dead or knocked out) that a guard sees is found once: the level blames the owner of a
//!   registered weapon lying within `plant_radius` of it, else whoever took the guard down - wanted at once. Plant a
//!   rival's gun beside your victim and the level hunts him;
//! - a registered weapon lying about on its own, seen by a guard, makes its owner suspicious.
//!
//! Answers (counters.rs): pick the planted gun up before staff see it, carry or hide the body.

use bevy::prelude::*;
use glam::DVec3;
use imsim_core::heat::HeatCause;
use std::collections::HashMap;
use std::sync::Mutex;

/// Thrown weapons: not dropped on death.
const GRENADES: [&str; 4] = ["WeaponLAM", "WeaponGasGrenade", "WeaponEMPGrenade", "WeaponNanoVirusGrenade"];

/// The property a registered weapon carries in the world.
pub const PROP: &str = "RegisteredTo";

#[derive(Default)]
struct State {
    /// This player's weapons by class: who each is registered to (his own unless he picked up another's).
    mine: HashMap<String, u8>,
    /// Host: registered weapons lying in the world (entity bits, class, owner, where).
    lying: Vec<(u64, String, u8, DVec3)>,
    /// Host: bodies found (blocker) and loose weapons seen (entity bits).
    found: Vec<usize>,
    seen: Vec<u64>,
    look_t: f32,
    /// The weapon class in hand last frame (dropped on death).
    held: Option<String>,
    was_dead: bool,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::default))
}

pub fn enabled() -> bool {
    crate::mech::on(|r| r.framing) && crate::living::active()
}

/// Who a weapon of this class in this player's inventory is registered to.
pub fn owner_of(class: &str) -> u8 {
    with(|s| s.mine.get(class).copied()).unwrap_or_else(crate::deathmatch::my_id)
}

/// The registration a dropped weapon carries (empty when framing is off or it isn't a weapon).
pub fn drop_props(class: &str) -> Vec<unreal_pkg::props::Property> {
    if !enabled() || !class.starts_with("Weapon") {
        return Vec::new();
    }
    vec![unreal_pkg::props::Property { name: PROP.into(), index: 0, value: unreal_pkg::props::Value::Int(owner_of(class) as i32) }]
}

/// The owner a lying item is registered to.
pub fn registered(props: &[unreal_pkg::props::Property]) -> Option<u8> {
    match unreal_pkg::props::find(props, PROP) {
        Some(unreal_pkg::props::Value::Int(n)) if (0..255).contains(n) => Some(*n as u8),
        _ => None,
    }
}

/// This player picked a weapon up: it stays registered to whoever it was.
pub fn picked_up(class: &str, props: &[unreal_pkg::props::Property]) {
    if let Some(o) = registered(props) {
        with(|s| s.mine.insert(class.to_string(), o));
    }
}

pub fn new_life() {
    with(|s| {
        s.mine.clear();
        s.held = None;
    });
}

pub fn reset_level() {
    with(|s| *s = State::default());
}

/// Host, from living::observe: bodies and loose registered weapons the staff come across.
pub fn host_observe(lvl: &dx_level::Level, dt: f32, staff: &[crate::living::StaffEye], bodies: &[crate::living::StaffBody]) {
    if !enabled() || !crate::mech::host() {
        return;
    }
    let go = with(|s| {
        s.look_t -= dt;
        if s.look_t <= 0.0 {
            s.look_t = 0.25;
            true
        } else {
            false
        }
    });
    if !go || staff.is_empty() {
        return;
    }
    let t = crate::mech::tuning();
    let lying = with(|s| s.lying.clone());
    let sees = |at: DVec3, reach: f64| {
        let mut lit = None;
        staff.iter().any(|e| e.eye.distance(at) <= reach && crate::living::eye_sees(lvl, e, at, &mut lit))
    };
    for b in bodies.iter().filter(|b| !b.hidden) {
        if with(|s| s.found.contains(&b.blocker)) {
            continue;
        }
        let at = b.pos + DVec3::Z * 8.0;
        if !sees(at, t.body_sight as f64) {
            continue;
        }
        with(|s| s.found.push(b.blocker));
        // The gun beside it, else the one who did it.
        let planted = lying
            .iter()
            .filter(|w| w.3.distance(b.pos) <= t.plant_radius as f64)
            .min_by(|x, y| x.3.distance(b.pos).total_cmp(&y.3.distance(b.pos)))
            .map(|w| w.2);
        let blame = planted.or(b.killer).filter(|&id| id != crate::living::LEVEL);
        println!("FRAMING body {} found: blamed {:?} (killer {:?}, planted {:?})", b.blocker, blame, b.killer, planted);
        if let Some(id) = blame {
            crate::living::raise(id, HeatCause::BodyFound, 0.0, Some(b.pos));
            let line = if planted.is_some() && planted != b.killer { "Security found a body - and your gun beside it." } else { "Security found a body and knows it was you." };
            if id == crate::deathmatch::my_id() {
                crate::classic_hud::client_message(line);
            } else {
                crate::netplay::tell(id, line);
            }
        }
    }
    // Registered weapons lying about on their own.
    for (key, class, owner, at) in &lying {
        if with(|s| s.seen.contains(key)) || bodies.iter().any(|b| b.pos.distance(*at) <= t.plant_radius as f64) {
            continue;
        }
        if sees(*at + DVec3::Z * 6.0, t.weapon_sight as f64) {
            with(|s| s.seen.push(*key));
            println!("FRAMING {class} of {owner} seen lying about");
            crate::living::raise(*owner, HeatCause::WeaponFound, 0.0, Some(*at));
        }
    }
}

/// Host: where the registered weapons lie (a few times a second); every player: the weapon he dies holding drops.
#[allow(clippy::type_complexity)]
pub fn framing_system(
    time: Res<Time>,
    items: Query<(Entity, &crate::pickups::PickupItem, Option<&crate::actors::Deco>, Option<&crate::destruct::Loose>)>,
    weapons: Res<crate::weapons::Weapons>,
    ph: Res<crate::ai::PlayerHealth>,
    player: Res<crate::Player>,
    mut throws: ResMut<crate::destruct::ItemThrows>,
    mut t: Local<f32>,
) {
    if !enabled() {
        return;
    }
    // Dying drops the weapon in hand, registered to whoever it is.
    if let Some(h) = weapons.held.as_ref() {
        with(|s| s.held = Some(h.stats.class.clone()));
    }
    let edge = with(|s| {
        let e = ph.dead && !s.was_dead;
        s.was_dead = ph.dead;
        e
    });
    if edge {
        if let Some(class) = with(|s| s.held.take()).filter(|c| c.starts_with("Weapon") && !GRENADES.contains(&c.as_str())) {
            let props = drop_props(&class);
            let at = player.body.location + DVec3::Z * 20.0;
            let vel = player.body.velocity * 0.5 + DVec3::Z * 120.0;
            println!("FRAMING dropped {class} on death");
            throws.0.push((class, at, vel, Some(0), props));
        }
    }
    if !crate::mech::host() {
        return;
    }
    *t -= time.delta_secs();
    if *t > 0.0 {
        return;
    }
    *t = 0.25;
    let lying: Vec<(u64, String, u8, DVec3)> = items
        .iter()
        .filter_map(|(e, it, deco, loose)| {
            let owner = registered(&it.props)?;
            let at = match (deco, loose) {
                (Some(d), _) => d.body.location,
                (_, Some(l)) => l.pos,
                _ => return None,
            };
            Some((e.to_bits(), it.class.clone(), owner, at))
        })
        .collect();
    with(|s| s.lying = lying);
}
