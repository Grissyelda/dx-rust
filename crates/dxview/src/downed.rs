//! Downed state, revive and interrogation (MULTIPLAYER roadmap Phase 3, Modern; the room's "Downed State" switch).
//!
//! A blow from a non-lethal weapon (prod, baton, pepper spray, gas, tranquilizer darts) that would kill a player
//! leaves him down instead: on the floor, weapon away, unable to move, for DOWN_SECS. Then he gets up by himself with
//! a little health. Meanwhile:
//! - a teammate (Team Deathmatch) who uses a medkit on him revives him at once with REVIVE_HEALTH;
//! - any further damage kills him (a rival "finishes" him; the finisher gets the kill);
//! - a rival who uses a multitool on him for INTERROGATE_SECS learns where every other rival is (pinged on his
//!   screen for PING_SECS) and spends one multitool.
//!
//! Downing a rival is a takedown (credits and the character's takedown count), not a kill: the kill (score) goes to
//! whoever finishes him. Each player's own game decides his state (as with death); the others see it in his body's
//! state (PState.down) and his body lies on the floor.

use crate::ai::{PlayerHealth, P_HEAD, P_TORSO};
use bevy::prelude::*;
use glam::DVec3;
use std::sync::Mutex;

pub const DOWN_SECS: f32 = 30.0;
/// Health of every part when getting up alone / revived.
const UP_HEALTH: f32 = 25.0;
const REVIVE_HEALTH: f32 = 40.0;
pub const INTERROGATE_SECS: f32 = 4.0;
const PING_SECS: f32 = 15.0;
/// Interrogating from further than this (or the body getting up) breaks it off.
const INTERROGATE_REACH: f64 = 140.0;

#[derive(Default)]
struct State {
    down: bool,
    left: f32,
    /// Went down since netplay last asked (it tells the host who did it).
    just: bool,
    /// Revived by a teammate (shown once).
    revived_by: Option<String>,
    /// Our interrogation of a downed rival: (his id, seconds so far, where we started).
    interrogate: Option<(u8, f32, DVec3)>,
}

static STATE: Mutex<State> = Mutex::new(State { down: false, left: 0.0, just: false, revived_by: None, interrogate: None });

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    f(&mut STATE.lock().unwrap_or_else(|e| e.into_inner()))
}

/// The room plays with the downed state (Modern versus, the host's switch).
pub fn enabled() -> bool {
    crate::living::modern_match() && crate::deathmatch::rules().is_some_and(|r| r.downed)
}

/// Damage types that down instead of kill.
pub fn knocks_down(kind: &str) -> bool {
    matches!(kind, "KnockedOut" | "Stunned" | "TearGas" | "PoisonGas" | "Poison" | "PoisonEffect")
}

pub fn is_down() -> bool {
    with(|s| s.down)
}

/// PlayerHealth::damage, before the health is added up: a non-lethal blow that would kill downs instead (the head
/// and torso kept above nothing); a blow taken while down kills.
pub fn intercept(parts: &mut [f32; 6], kind: &str) {
    if !enabled() {
        return;
    }
    let down = is_down();
    if down {
        // Finished.
        parts[P_HEAD] = 0.0;
        with(|s| s.down = false);
        return;
    }
    if !knocks_down(kind) {
        return;
    }
    let p = &*parts;
    let crit = p[P_HEAD] <= 0.0 || p[P_TORSO] <= 0.0;
    let ave = (p[0] + p[1] + p[2] + p[3] + p[4] + p[5] - p[P_HEAD] - p[P_TORSO]) / 4.0;
    if !(crit || (ave + (p[P_HEAD] + p[P_TORSO]) / 2.0) / 2.0 <= 0.0) {
        return;
    }
    parts[P_HEAD] = parts[P_HEAD].max(5.0);
    parts[P_TORSO] = parts[P_TORSO].max(5.0);
    with(|s| {
        s.down = true;
        s.left = DOWN_SECS;
        s.just = true;
        s.interrogate = None;
    });
    if env_on!("DXVIEW_NETLOG") {
        println!("DOWNED: down by {kind}");
    }
    crate::classic_hud::client_message("You're down. A teammate with a medkit can revive you; anyone can finish you.");
}

/// This player just went down (netplay tells the host, with the last attacker).
pub fn take_just() -> bool {
    with(|s| std::mem::take(&mut s.just))
}

/// A teammate's medkit (netplay Msg::Revive): up at once.
pub fn revive(by: &str) {
    with(|s| {
        if s.down {
            s.down = false;
            s.left = 0.0;
            s.revived_by = Some(by.to_string());
        }
    });
}

/// A new life, a new map: not down.
pub fn reset() {
    with(|s| *s = State::default());
}

pub fn reset_level() {
    reset();
}

/// Start interrogating downed rival `id` (pickups: Use with a multitool in hand).
pub fn begin_interrogation(id: u8, at: DVec3) {
    with(|s| {
        if s.interrogate.is_none_or(|i| i.0 != id) {
            s.interrogate = Some((id, 0.0, at));
        }
    });
}

#[allow(clippy::too_many_arguments)]
pub fn downed_system(
    time: Res<Time>,
    mut ph: ResMut<PlayerHealth>,
    player: Res<crate::Player>,
    mut hud: ResMut<crate::movers::HudMessage>,
    mut inv: ResMut<crate::pickups::PlayerInventory>,
    mut weapons: ResMut<crate::weapons::Weapons>,
    mut root: Query<&mut Visibility, With<crate::weapons::ViewModelRoot>>,
    mut was: Local<bool>,
) {
    let dt = time.delta_secs();
    // Dev aid: DXVIEW_DOWN_AT=<secs> - a knock-out blow at that game time (test runs).
    // (Real seconds since start: on a slow machine game time falls behind.)
    if let Some(at) = env_val!("DXVIEW_DOWN_AT").and_then(|v| v.parse::<f32>().ok()) {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let t = START.get_or_init(std::time::Instant::now).elapsed().as_secs_f32();
        if t >= at && !DONE.swap(true, std::sync::atomic::Ordering::Relaxed) {
            println!("DOWNED: test blow (enabled {})", enabled());
            ph.damage(500.0, "KnockedOut", DVec3::new(0.0, 0.0, 10.0));
        }
    }
    if !enabled() {
        if *was || is_down() {
            reset();
            *was = false;
        }
        return;
    }
    if ph.dead {
        with(|s| s.down = false);
    }
    let (down, left, revived) = with(|s| {
        if s.down {
            s.left -= dt;
        }
        (s.down, s.left, s.revived_by.take())
    });
    if down && left <= 0.0 {
        // Up again by himself.
        with(|s| s.down = false);
        for p in ph.parts.iter_mut() {
            *p = p.max(UP_HEALTH);
        }
        ph.recompute();
        crate::classic_hud::client_message("You came round.");
        if env_on!("DXVIEW_NETLOG") {
            println!("DOWNED: up again, health {:.0}", ph.hp);
        }
    }
    if let Some(by) = revived {
        for p in ph.parts.iter_mut() {
            *p = p.max(REVIVE_HEALTH);
        }
        ph.recompute();
        crate::bodyfx::healed(P_TORSO);
        crate::sound::play_player("DeusExSounds.Generic.MedicalHiss", 1.0, 256.0);
        crate::classic_hud::client_message(&format!("{by} got you back on your feet."));
    }
    let down = is_down();
    if down {
        // Weapon away, the view on the floor (lib.rs keeps him crouched and still), the countdown in the middle.
        weapons.held = None;
        if let Ok(mut v) = root.single_mut() {
            v.set_if_neq(Visibility::Hidden);
        }
        hud.text = format!("DOWN - {} s. A teammate's medkit gets you up; any hit finishes you.", with(|s| s.left).ceil().max(0.0) as i32);
        hud.time_left = 101.0;
    } else if *was {
        if let Ok(mut v) = root.single_mut() {
            v.set_if_neq(Visibility::Inherited);
        }
        if hud.time_left > 100.0 && hud.text.starts_with("DOWN") {
            hud.text.clear();
            hud.time_left = 0.0;
        }
    }
    *was = down;
    // Our interrogation of a rival: kept up while near him and he stays down; done after INTERROGATE_SECS.
    let me = player.body.location;
    let step = with(|s| {
        let (id, t, _) = s.interrogate.as_mut()?;
        *t += dt;
        Some((*id, *t))
    });
    let Some((id, t)) = step else { return };
    let target = crate::netplay::rivals().into_iter().find(|r| r.id == id);
    let ok = target.as_ref().is_some_and(|r| r.down && (r.pos - me).length() < INTERROGATE_REACH) && !down && !ph.dead;
    if !ok {
        with(|s| s.interrogate = None);
        hud.text = "Interrogation broken off.".into();
        hud.time_left = 2.0;
        return;
    }
    if t < INTERROGATE_SECS {
        hud.text = format!("Interrogating... {}%", (t / INTERROGATE_SECS * 100.0) as i32);
        hud.time_left = 0.5;
        return;
    }
    with(|s| s.interrogate = None);
    inv.0.use_one("Multitool");
    let me_id = crate::deathmatch::my_id();
    let others: Vec<crate::netplay::Rival> = crate::netplay::rivals().into_iter().filter(|r| r.rival && r.id != id).collect();
    for r in &others {
        crate::living::add_ping(r.pos + DVec3::Z * 40.0, r.id, PING_SECS, crate::living::Audience::Player(me_id).code());
    }
    let name = target.map(|r| r.name).unwrap_or_default();
    crate::netplay::tell(id, &format!("{} interrogated you.", crate::travel::player_name()));
    hud.text = if others.is_empty() { format!("{name} gave nothing away: nobody else is out there.") } else { format!("{name} talked: {} rival{} marked.", others.len(), if others.len() == 1 { "" } else { "s" }) };
    hud.time_left = 4.0;
    crate::sound::play_player("DeusExSounds.UserInterface.LogGoalAdded", 0.8, 0.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_lethal_kinds() {
        assert!(knocks_down("KnockedOut") && knocks_down("Stunned") && knocks_down("TearGas"));
        assert!(!knocks_down("Shot") && !knocks_down("Exploded") && !knocks_down("Burned"));
    }
}
