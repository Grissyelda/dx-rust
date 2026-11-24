//! Uniforms (MULTIPLAYER roadmap Phase 3, Modern; the room's "Uniforms" switch, needs the living level).
//!
//! Use on a knocked-out guard of the level takes his uniform (once per body: the host keeps who stripped which). Its
//! wearer's body is dressed as that guard for everyone, and the level's staff take him for one of their own:
//! - a drawn weapon, a lockpick or a restricted room no longer make them hostile on sight (living.rs);
//! - but where they can see him, running, a weapon within `armed_near` of them, a restricted room or a tool at work
//!   make them wonder (heat, cause Disguise): first suspicious, then the cover is blown - the uniform comes off and
//!   the alarm goes up;
//! - anything that makes him wanted (an unsilenced shot near them, hurting staff, a body pinned on him) blows it too.
//! It lasts `uniform_secs` (the Infiltrator's longer).
//!
//! Rival players see a guard: no name tag, no red crosshair (cover.rs). Looking straight at him from within
//! `close_look` for the counter table's seconds unmasks him for that viewer, and so does the Vision augmentation.

use crate::mech::{self, MechMsg};
use bevy::prelude::*;
use std::sync::Mutex;

struct Worn {
    class: String,
    name: String,
    left: f32,
}

#[derive(Default)]
struct State {
    worn: Option<Worn>,
    /// Host: bodies already stripped (placed actor index).
    stripped: Vec<u32>,
    /// Joiner: bodies we asked about (a second Use searches them as usual).
    asked: Vec<u32>,
    /// Rivals this player has unmasked, with the uniform they wore then.
    revealed: Vec<(u8, String)>,
    /// The rival looked at and for how long.
    look: (u8, f32),
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::default))
}

/// The room plays with uniforms (and the level has staff to fool).
pub fn enabled() -> bool {
    mech::on(|r| r.uniforms) && crate::living::active()
}

/// The guard's class whose uniform this player wears.
pub fn worn_class() -> Option<String> {
    with(|s| s.worn.as_ref().map(|w| w.class.clone()))
}

/// (the guard's name, seconds left) for the HUD.
pub fn worn_status() -> Option<(String, f32)> {
    with(|s| s.worn.as_ref().map(|w| (w.name.clone(), w.left)))
}

/// A rival who passes as a guard to this player (in a uniform he hasn't been unmasked in).
pub fn passes(id: u8) -> bool {
    if !crate::living::modern_match() {
        return false;
    }
    let Some(class) = crate::netplay::uniform_of(id) else { return false };
    !with(|s| s.revealed.iter().any(|r| r.0 == id && r.1 == class))
}

/// A class of guard whose uniform can be worn: a person with an outfit of its own.
pub fn wearable(class: &str) -> bool {
    crate::classtree::is_a(class, "HumanMilitary") || crate::classtree::is_a(class, "HumanCivilian") || class.eq_ignore_ascii_case(&crate::living::tuning().guard)
}

fn familiar(class: &str) -> String {
    match crate::classicmp::class_value(class, "FamiliarName") {
        Some(unreal_pkg::props::Value::Str(s)) if !s.is_empty() => s,
        _ => class.to_string(),
    }
}

/// Use on a body: a knocked-out guard's uniform is taken (here, or asked of the host). True when that was the Use;
/// false lets the body be searched as usual.
pub fn try_take(actor: usize, class: &str, knocked_out: bool, hud: &mut crate::movers::HudMessage) -> bool {
    if !enabled() || !knocked_out || !wearable(class) {
        return false;
    }
    let a = actor as u32;
    if mech::host() {
        if with(|s| s.stripped.contains(&a)) {
            return false;
        }
        with(|s| s.stripped.push(a));
        wear(class);
        return true;
    }
    if with(|s| s.asked.contains(&a)) {
        return false;
    }
    with(|s| s.asked.push(a));
    mech::to_host(MechMsg::UniformAsk(a));
    hud.text = "Taking his uniform...".into();
    hud.time_left = 1.5;
    true
}

/// Host: a joiner asks for the uniform off this body.
pub fn host_ask(from: u8, actor: u32, lvl: Option<&dx_level::Level>) {
    let class = lvl.and_then(|l| l.actors.get(actor as usize)).map(|a| a.class.clone()).unwrap_or_default();
    let why = if !enabled() {
        Some("Uniforms are off in this room.")
    } else if class.is_empty() || !wearable(&class) {
        Some("Nothing you could pass in.")
    } else if with(|s| s.stripped.contains(&actor)) {
        Some("Somebody already took his uniform.")
    } else {
        None
    };
    match why {
        Some(w) => mech::to_player(from, MechMsg::UniformNo(w.into())),
        None => {
            with(|s| s.stripped.push(actor));
            mech::to_player(from, MechMsg::UniformGive(class));
        }
    }
}

/// The host gave us this uniform.
pub fn given(class: &str) {
    wear(class);
}

fn wear(class: &str) {
    let t = mech::tuning();
    let secs = if crate::dmchar::class_perk() == "infiltrator" { t.uniform_secs_infiltrator } else { t.uniform_secs };
    let name = familiar(class);
    with(|s| s.worn = Some(Worn { class: class.to_string(), name: name.clone(), left: secs }));
    crate::classic_hud::client_message(&format!(
        "You're in a {name}'s uniform: staff take you for one of them. Don't run, don't show a weapon near them, keep out of restricted rooms."
    ));
    crate::sound::play_player("DeusExSounds.UserInterface.LogGoalAdded", 0.8, 0.0);
    println!("UNIFORM on: {class} for {secs} s");
}

/// Staff saw through it (or it wore out, or a new life): off.
pub fn blow() {
    if with(|s| s.worn.take()).is_some() {
        crate::classic_hud::client_message("Your cover is blown!");
        crate::sound::play_player("DeusExSounds.Generic.Beep6", 0.8, 0.0);
        println!("UNIFORM blown");
    }
}

/// Host: player `id`'s cover is blown.
pub fn host_blown(id: u8) {
    mech::to_player(id, MechMsg::UniformBlown);
}

/// A new life: no uniform.
pub fn reset() {
    with(|s| s.worn = None);
}

pub fn reset_level() {
    with(|s| *s = State::default());
}

/// The uniform's clock, and unmasking rivals: a close, steady look or the Vision augmentation.
pub fn uniform_system(time: Res<Time>, ph: Res<crate::ai::PlayerHealth>, player: Res<crate::Player>) {
    let dt = time.delta_secs();
    if ph.dead && worn_class().is_some() {
        reset();
    }
    let expired = with(|s| {
        let w = s.worn.as_mut()?;
        w.left -= dt;
        (w.left <= 0.0).then(|| s.worn.take()).flatten()
    });
    if expired.is_some() {
        crate::classic_hud::client_message("Your uniform won't pass any more: you took it off.");
    }
    if !crate::living::modern_match() {
        return;
    }
    let me = player.body.location;
    let t = mech::tuning();
    let rivals = crate::netplay::rivals();
    // Masks that no longer apply (a new uniform, or none) are forgotten.
    with(|s| s.revealed.retain(|(id, c)| rivals.iter().any(|r| r.id == *id && crate::netplay::uniform_of(*id).as_deref() == Some(c.as_str()))));
    let vision = crate::abilities::aug_active("AugVision") && crate::counters::value("Uniform", "AugVision", "unmask", 1.0) > 0.0;
    let aimed = crate::hud::aimed_pawn().and_then(crate::netplay::peer_of_blocker);
    let need = crate::counters::value("Uniform", "CloseLook", "unmask_secs", t.close_look_secs);
    for r in rivals.iter().filter(|r| r.rival && r.uniform && passes(r.id)) {
        let d = r.pos.distance(me);
        let by_vision = vision && d < 1200.0;
        let looked = aimed == Some(r.id) && d <= t.close_look as f64 && {
            let held = with(|s| {
                s.look = if s.look.0 == r.id { (r.id, s.look.1 + dt) } else { (r.id, dt) };
                s.look.1
            });
            held >= need
        };
        if by_vision || looked {
            let class = crate::netplay::uniform_of(r.id).unwrap_or_default();
            with(|s| s.revealed.push((r.id, class)));
            crate::classic_hud::client_message(&format!("That guard is {}!", r.name));
            crate::sound::play_player("DeusExSounds.Generic.Beep6", 0.5, 0.0);
        }
    }
    if aimed.is_none() {
        with(|s| s.look = (u8::MAX, 0.0));
    }
}
