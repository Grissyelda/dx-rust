//! The Nameless Mod's own trigger and elevator classes, written for DX RUST from how they behave in the player's copy
//! of the mod (nothing of it is shipped).
//!
//! * Triggers (`TnmKind`, run by triggers.rs like the Deus Ex ones): elevator sequence triggers, BindName changes,
//!   goals, limited and alliance-toggle (break and enter) triggers, destroy triggers, AI-event listeners, player
//!   visibility, orders, autosaves, robot disablers, the trigger-a-trigger / touch / frob relay, insanity, light
//!   switch flippers, its two conversation triggers, mover blow-up triggers and achievement triggers.
//! * CaroneElevator (`tnm_system`): an elevator that goes to a keyframe in the time its per-floor move times add up
//!   to, opening and closing its CEDoors (the movers tagged with that floor's event) around the trip, remembering a
//!   floor asked for while it moves; CESequenceDispatcher (floors on a timetable) and CEDoorButton (opens or shuts
//!   the doors where it stands).

use crate::ai::{Sel, WorldReq};
use bevy::prelude::*;
use glam::DVec3;
use imsim_core::flags::FlagBase;
use std::sync::Mutex;
use unreal_pkg::props::{find, Property, Value};

/// The trigger classes this module answers for (only while The Nameless Mod is played).
pub const CLASSES: [&str; 33] = [
    "CESequenceTrigger",
    "TNMBindNameTrigger",
    "GoalAddTrigger",
    "LimitedTrigger",
    "AllianceToggleTrigger",
    "DestroyTrigger",
    "AIEventTrigger",
    "PlayerVisibilityTrigger",
    "tnmOrdersTrigger",
    "AutosaveTrigger",
    "BotDisablerTrigger",
    "InvasionTrigger",
    "TriggerTrigger",
    "insanetrigger",
    "LightSwitchFlipper",
    "TNMConversationTrigger",
    "ConversationForceTrigger",
    "BlowUpMoverTrigger",
    "AchievementTrigger",
    "FrobableTrigger",
    "TakeDamageTrigger",
    "insanedeathtrigger",
    "SetGoalTrigger",
    "oxygenzonetrigger",
    "ShoutConversationTrigger",
    "LoudNoiseTrigger",
    "NoteAddTrigger",
    "CutsceneSoundTrigger",
    "CutsceneShakeTrigger",
    "WeaponSenseTrigger",
    "TurretDisableTurretTrigger",
    "TurretDisableStaringTrigger",
    "gravtrigger",
];

pub fn is_class(class: &str) -> bool {
    crate::tnmmode::active() && CLASSES.iter().any(|c| c.eq_ignore_ascii_case(class))
}

#[derive(Clone, Debug)]
pub enum TnmKind {
    /// SetSeq(SeqNum) on the elevators tagged Event.
    CeSeq(u8),
    BindName { new: String, check: Option<String> },
    GoalAdd { name: String, text: String, primary: bool, completed: bool },
    /// Triggers its Event this many more times.
    Limited(i32),
    /// The player is somewhere he shouldn't be (bBreakAndEnter) while touching it / after its Trigger.
    BreakEnter,
    Destroy,
    /// Fires its Event when this AI event reaches it.
    AiEvent(String),
    Visibility(bool),
    Orders { orders: String, tag: String },
    Autosave,
    BotDisabler,
    Invasion,
    /// MoreTriggers relay: 0 = trigger, 1 = touch, 2 = frob what Event names; uses so far and the limit.
    Relay { how: u8, used: i32, limit: i32 },
    Insane { start: bool },
    LightFlip(bool),
    Con { con: String, bind: String, check: String, check_false: bool, timer: bool, breathe: f32 },
    ConForce { con: String, bind: String, check: String, check_false: bool, parts: Vec<String>, used: i32, limit: i32 },
    BlowUp { used: i32, limit: i32 },
    /// AchievementTrigger: earns AchievementName (tnmach.rs).
    Achievement(String),
    /// OxygenZoneTrigger: the zones tagged Event lose or get air when triggered (touching does nothing).
    Oxygen { toggle: bool, value: bool },
    /// The Event's decorations solid again (Trigger) or not (UnTrigger): bodies of collision only; kept as a
    /// plain trigger.
    Inert,
    /// LoudNoiseTrigger: an AI LoudNoise at the trigger (volume, radius) when triggered.
    LoudNoise { pos: DVec3, volume: f32, radius: f32 },
    /// NoteAddTrigger: the text NoteTag of Textpackage goes into the DataVault.
    NoteAdd { tag: String, package: String },
    /// CutsceneSoundTrigger: the sound, heard everywhere.
    CutSound { sound: String, volume: f32 },
    /// CutsceneShakeTrigger: the cut-scene camera shakes from one magnitude to another over a time.
    CutShake { start: f32, end: f32, time: f32 },
    /// WeaponSenseTrigger: fires only when the player carries (or holds) a weapon of the kind; else failedEvent.
    WeaponSense { failed: String, in_hand: bool, cause: u8 },
    /// BSPTurretTriggers: the StaringMovers tagged Event stop firing (true) or stop following (false).
    TurretOff(bool),
    /// gravtrigger: its gravcommands (zone name, gravity number) as saved; triggered only.
    Grav(Vec<Vec<u8>>),
}

/// What a TNM trigger asks triggers.rs to do after it ran.
#[derive(Default)]
pub struct Act {
    /// Run the base class's Touch (event, message, once-only) / Trigger (OtherTrigger states).
    pub base: bool,
    /// Destroy the trigger.
    pub destroy: bool,
    /// Events to fire as the trigger.
    pub events: Vec<(String, bool)>,
    pub messages: Vec<String>,
}

fn s(p: &[Property], n: &str) -> String {
    match find(p, n) {
        Some(Value::Name(v) | Value::Str(v)) if v != "None" => v.clone(),
        _ => String::new(),
    }
}
fn b(p: &[Property], n: &str, d: bool) -> bool {
    match find(p, n) {
        Some(Value::Bool(v)) => *v,
        _ => d,
    }
}
fn i(p: &[Property], n: &str, d: i32) -> i32 {
    match find(p, n) {
        Some(Value::Int(v)) => *v,
        Some(Value::Byte(v)) => *v as i32,
        Some(Value::Float(v)) => *v as i32,
        _ => d,
    }
}

/// The trigger's own settings from its (effective) properties.
pub fn kind_for(class: &str, p: &[Property]) -> TnmKind {
    let lc = class.to_ascii_lowercase();
    match lc.as_str() {
        "cesequencetrigger" => TnmKind::CeSeq(i(p, "SeqNum", 0).clamp(0, 7) as u8),
        "tnmbindnametrigger" => TnmKind::BindName {
            new: s(p, "NewBindname"),
            check: b(p, "bCheck", false).then(|| s(p, "CheckBindName")),
        },
        "goaladdtrigger" | "setgoaltrigger" => TnmKind::GoalAdd {
            name: s(p, "goalName"),
            text: s(p, "goalText"),
            primary: b(p, "bPrimaryGoal", false),
            completed: b(p, "bGoalCompleted", false),
        },
        "limitedtrigger" => TnmKind::Limited(i(p, "NumberOfTriggers", 0)),
        "alliancetoggletrigger" => TnmKind::BreakEnter,
        "destroytrigger" => TnmKind::Destroy,
        "aieventtrigger" => TnmKind::AiEvent(s(p, "AIEvent").to_ascii_lowercase()),
        "playervisibilitytrigger" => TnmKind::Visibility(b(p, "HidePlayer", false)),
        "tnmorderstrigger" => TnmKind::Orders { orders: s(p, "Orders"), tag: s(p, "ordersTag") },
        "autosavetrigger" => TnmKind::Autosave,
        "botdisablertrigger" => TnmKind::BotDisabler,
        "invasiontrigger" => TnmKind::Invasion,
        "triggertrigger" => TnmKind::Relay {
            how: i(p, "TrigType", 0).clamp(0, 2) as u8,
            used: 0,
            limit: i(p, "DeactivationThreshold", 0),
        },
        "insanetrigger" => TnmKind::Insane { start: b(p, "bStartInsane", false) },
        "lightswitchflipper" => TnmKind::LightFlip(b(p, "SetOff", false)),
        "oxygenzonetrigger" => TnmKind::Oxygen { toggle: b(p, "bToggle", false), value: b(p, "value", false) },
        "tnmconversationtrigger" => TnmKind::Con {
            con: s(p, "conversationTag"),
            bind: s(p, "BindName"),
            check: s(p, "checkFlag"),
            check_false: b(p, "bCheckFalse", false),
            timer: b(p, "bCheckTimer", false),
            breathe: i(p, "conversationBreathePeriod", 0) as f32,
        },
        "conversationforcetrigger" => TnmKind::ConForce {
            con: s(p, "conversationTag"),
            bind: s(p, "BindName"),
            check: s(p, "checkFlag"),
            check_false: b(p, "bCheckFalse", false),
            parts: p
                .iter()
                .filter(|x| x.name.eq_ignore_ascii_case("ParticipantNames"))
                .filter_map(|x| match &x.value {
                    Value::Name(n) if n != "None" && !n.is_empty() => Some((x.index, n.clone())),
                    _ => None,
                })
                .collect::<std::collections::BTreeMap<_, _>>()
                .into_values()
                .collect(),
            used: 0,
            limit: i(p, "DeactivationThreshold", 0),
        },
        "blowupmovertrigger" => TnmKind::BlowUp { used: 0, limit: i(p, "DeactivationThreshold", 0) },
        "achievementtrigger" => TnmKind::Achievement(match find(p, "AchievementName") {
            Some(Value::Str(v)) | Some(Value::Name(v)) => v.clone(),
            _ => String::new(),
        }),
        "shoutconversationtrigger" => TnmKind::Con {
            con: s(p, "conversationTag"),
            bind: s(p, "BindName"),
            check: s(p, "checkFlag"),
            check_false: b(p, "bCheckFalse", false),
            timer: false,
            breathe: 0.0,
        },
        "loudnoisetrigger" => {
            let l = match find(p, "Location") {
                Some(Value::Vector(v)) => DVec3::new(v[0] as f64, v[1] as f64, v[2] as f64),
                _ => DVec3::ZERO,
            };
            TnmKind::LoudNoise {
                pos: l,
                volume: i(p, "SoundVolume", 128) as f32 / 255.0,
                radius: 25.0 * (i(p, "SoundRadius", 32) as f32 + 1.0),
            }
        }
        "noteaddtrigger" => TnmKind::NoteAdd { tag: s(p, "NoteTag"), package: s(p, "Textpackage") },
        "cutscenesoundtrigger" => TnmKind::CutSound {
            sound: match find(p, "Sound") {
                Some(Value::Path(v)) => v.clone(),
                _ => String::new(),
            },
            volume: match find(p, "volume") {
                Some(Value::Float(v)) => *v,
                _ => 1.0,
            },
        },
        "cutsceneshaketrigger" => {
            let f = |n: &str| match find(p, n) {
                Some(Value::Float(v)) => *v,
                Some(Value::Int(v)) => *v as f32,
                _ => 0.0,
            };
            TnmKind::CutShake { start: f("MagnitudeStart"), end: f("MagnitudeEnd"), time: f("TransitionTime") }
        }
        "weaponsensetrigger" => TnmKind::WeaponSense {
            failed: s(p, "failedEvent"),
            in_hand: b(p, "binhand", false),
            cause: i(p, "TriggerCause", 0).clamp(0, 3) as u8,
        },
        "gravtrigger" => TnmKind::Grav(
            p.iter()
                .filter(|x| x.name.eq_ignore_ascii_case("gravcommands"))
                .filter_map(|x| match &x.value {
                    Value::Struct(_, b) => Some(b.clone()),
                    _ => None,
                })
                .collect(),
        ),
        "turretdisableturrettrigger" => TnmKind::TurretOff(true),
        "turretdisablestaringtrigger" => TnmKind::TurretOff(false),
        _ => TnmKind::Inert,
    }
}

/// NoteAddTrigger.givenote.
fn note_add(tag: &str, package: &str) {
    if tag.is_empty() {
        return;
    }
    let Some(root) = unreal_pkg::paths::root() else { return };
    let pkg = if package.is_empty() { "DeusExText" } else { package };
    let Ok(p) = unreal_pkg::Package::open(root.join("System").join(format!("{pkg}.u"))) else { return };
    if let Some(t) = p.text_named(tag) {
        let v = crate::info::parse_tnm(&t).vault;
        if !crate::consys::has_vault_note(&v) {
            crate::consys::add_vault_note(&v);
        }
    }
}

/// WeaponSenseTrigger.Touch's test: a weapon in hand (bInHand), or any carried weapon that sets it off
/// (TriggerCause: 0 any, 1 Concealability Visual or None, 2 None, 3 a TNMWeapon with bNoMetal).
fn weapon_sensed(in_hand: bool, cause: u8) -> bool {
    if in_hand {
        return crate::perks::in_hand().is_some_and(|(c, _)| crate::classtree::is_a(&c, "DeusExWeapon"));
    }
    CARRIED.lock().is_ok_and(|w| {
        w.iter().any(|&(conc, no_metal)| match cause {
            0 => true,
            3 => no_metal,
            // EConcealability: CONC_None 0, CONC_Visual 1, CONC_Metal 2, CONC_All 3.
            1 => conc <= 1,
            _ => conc == 0,
        })
    })
}

/// The weapons the player carries, as (Concealability, bNoMetal), for WeaponSenseTriggers.
static CARRIED: Mutex<Vec<(u8, bool)>> = Mutex::new(Vec::new());

fn flag_ok(flags: &FlagBase, check: &str, check_false: bool) -> bool {
    check.is_empty() || flags.get_bool(check) != check_false
}

// ------------------------------------------------------------------------------------------------- shared state

/// Requests the elevator system takes up: (elevator tag, floor).
static CE_REQS: Mutex<Vec<(String, u8)>> = Mutex::new(Vec::new());
/// Events seen by the dispatch (dispatchers and door buttons listen).
static INBOX: Mutex<Vec<(String, bool)>> = Mutex::new(Vec::new());
/// Actor work for the system: (what, tag).
static WORK: Mutex<Vec<(Work, String)>> = Mutex::new(Vec::new());
/// AI events this frame: (kind, where, how far it carries).
static AI_EVENTS: Mutex<Vec<(&'static str, DVec3, f64)>> = Mutex::new(Vec::new());
/// Trigger tags whose AI event arrived (triggers.rs fires their Event).
static AI_FIRED: Mutex<Vec<usize>> = Mutex::new(Vec::new());
/// Triggers to touch as if the player walked in (the relay's touch mode), by Tag.
static TOUCHES: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[derive(Clone, Copy, PartialEq, Debug)]
enum Work {
    Destroy,
    Frob,
    BlowUp,
    LightOn,
    LightOff,
}

static BREAK_ENTER: Mutex<(bool, Vec<String>)> = Mutex::new((false, Vec::new()));
static PLAYER_HIDDEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static INSANE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn work(w: Work, tag: &str) {
    if tag.is_empty() {
        return;
    }
    if let Ok(mut q) = WORK.lock() {
        q.push((w, tag.to_string()));
    }
}

/// Every event the game dispatches (devices.rs).
pub fn event(name: &str, on: bool) {
    if !crate::tnmmode::active() {
        return;
    }
    if on {
        crate::texctl::event(name);
    }
    if let Ok(mut q) = INBOX.lock() {
        q.push((name.to_string(), on));
    }
}

/// An AI event (the alarm going off, a distress cry, a loud noise, gunfire) at `pos` heard out to `radius`.
pub fn ai_event(kind: &'static str, pos: DVec3, radius: f64) {
    if !crate::tnmmode::active() {
        return;
    }
    if let Ok(mut q) = AI_EVENTS.lock() {
        if q.len() < 256 {
            q.push((kind, pos, radius));
        }
    }
}

/// The player is in a place he shouldn't be, and these pawn tags hate it (TNMPawn.bHateBreakAndEnter).
pub fn break_enter_hated(tag: &str) -> bool {
    BREAK_ENTER
        .lock()
        .is_ok_and(|g| g.0 && g.1.iter().any(|t| t.eq_ignore_ascii_case(tag)))
}

/// PlayerVisibilityTrigger: the player's bHidden (nobody sees him).
pub fn player_hidden() -> bool {
    PLAYER_HIDDEN.load(std::sync::atomic::Ordering::Relaxed)
}

/// Trestkon.bInsane (the shadow code's voices).
pub fn insane() -> bool {
    INSANE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Triggers whose AI event arrived since the last call (indices into triggers.rs's list).
pub fn take_ai_fired() -> Vec<usize> {
    AI_FIRED.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}

/// Tags the relay asked to be touched.
pub fn take_touches() -> Vec<String> {
    TOUCHES.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}

/// A new map: nothing carried over.
pub fn reset_level() {
    for q in [&CE_REQS] {
        if let Ok(mut q) = q.lock() {
            q.clear();
        }
    }
    if let Ok(mut q) = INBOX.lock() {
        q.clear();
    }
    if let Ok(mut q) = WORK.lock() {
        q.clear();
    }
    if let Ok(mut q) = AI_EVENTS.lock() {
        q.clear();
    }
    if let Ok(mut q) = AI_FIRED.lock() {
        q.clear();
    }
    if let Ok(mut q) = TOUCHES.lock() {
        q.clear();
    }
    if let Ok(mut g) = BREAK_ENTER.lock() {
        *g = (false, Vec::new());
    }
    PLAYER_HIDDEN.store(false, std::sync::atomic::Ordering::Relaxed);
    if let Ok(mut l) = LISTENERS.lock() {
        l.clear();
    }
}

// --------------------------------------------------------------------------------------------- trigger behaviour

fn set_seq(event: &str, seq: u8) {
    if event.is_empty() {
        return;
    }
    if let Ok(mut q) = CE_REQS.lock() {
        q.push((event.to_string(), seq));
    }
}

fn goal(name: &str, text: &str, primary: bool, completed: bool) {
    if let Some(cs) = crate::consys::lock().as_mut() {
        cs.add_goal(name, text, primary);
        if completed {
            cs.goal_completed(name);
        }
    }
}

fn break_enter(event: &str, on: bool) {
    if let Ok(mut g) = BREAK_ENTER.lock() {
        g.0 = on;
        if !event.is_empty() && !g.1.iter().any(|t| t.eq_ignore_ascii_case(event)) {
            g.1.push(event.to_string());
        }
        if !on && !event.is_empty() {
            g.1.retain(|t| !t.eq_ignore_ascii_case(event));
        }
    }
}

fn relay(how: u8, event: &str, act: &mut Act) {
    match how {
        0 => act.events.push((event.to_string(), true)),
        1 => {
            if let Ok(mut q) = TOUCHES.lock() {
                q.push(event.to_string());
            }
        }
        _ => work(Work::Frob, event),
    }
}

/// MoreTriggers: count a use; spent after DeactivationThreshold uses (or at once when bTriggerOnceOnly).
fn spend(used: &mut i32, limit: i32, once: bool, act: &mut Act) {
    *used += 1;
    if once || (limit != 0 && *used >= limit) {
        act.destroy = true;
    }
}

/// ConversationForceTrigger: every named participant still up, the owner in the world.
fn con_force(flags: &mut FlagBase, con: &str, bind: &str, check: &str, check_false: bool, parts: &[String]) {
    if !flag_ok(flags, check, check_false) || con.is_empty() || bind.is_empty() {
        return;
    }
    if parts.iter().any(|p| flags.get_bool(&format!("{p}_Unconscious")) || flags.get_bool(&format!("{p}_Dead"))) {
        return;
    }
    crate::consys::start_forced(flags, con, bind);
}

fn tnm_con(flags: &mut FlagBase, con: &str, bind: &str, check: &str, check_false: bool, timer: bool, breathe: f32) -> bool {
    if !flag_ok(flags, check, check_false) || con.is_empty() || bind.is_empty() {
        return false;
    }
    if timer && crate::consys::since_third_person() < breathe {
        return false;
    }
    crate::consys::start_named(flags, con, bind)
}

/// Touch by the player (or a pawn; `relevant` = the trigger's IsRelevant for it).
pub fn on_touch(k: &mut TnmKind, event: &str, once: bool, by_player: bool, relevant: bool, flags: &mut FlagBase) -> Act {
    let mut act = Act::default();
    match k {
        TnmKind::CeSeq(seq) => {
            set_seq(event, *seq);
            act.base = relevant;
        }
        TnmKind::GoalAdd { name, text, primary, completed } => {
            if relevant {
                goal(name, text, *primary, *completed);
                act.events.push((event.to_string(), true));
                act.destroy = true;
            }
        }
        TnmKind::BreakEnter => {
            if relevant {
                break_enter(event, true);
                act.destroy = once;
            }
        }
        TnmKind::Destroy => {
            work(Work::Destroy, event);
            act.events.push((event.to_string(), true));
        }
        // These answer their Trigger only (their Touch does nothing).
        TnmKind::AiEvent(_) | TnmKind::Insane { .. } | TnmKind::Inert => {}
        TnmKind::Visibility(hide) => PLAYER_HIDDEN.store(*hide, std::sync::atomic::Ordering::Relaxed),
        TnmKind::Orders { orders, tag } => {
            if relevant {
                crate::triggers::push_order(crate::triggers::PawnOrder::Orders {
                    tag: event.to_string(),
                    orders: orders.clone(),
                    orders_tag: tag.clone(),
                });
                act.destroy = once;
            }
        }
        TnmKind::BotDisabler => {
            if relevant {
                crate::ai::world_req(WorldReq::Disable(Sel::tag(event)));
                act.destroy = once;
            }
        }
        // The swordsman's invaders only come after the sword is unlocked, which a story playthrough never does
        // before the end: the trigger is spent.
        TnmKind::Invasion => act.destroy = relevant,
        TnmKind::Relay { how, used, limit } => {
            if relevant {
                relay(*how, event, &mut act);
                spend(used, *limit, once, &mut act);
            }
        }
        TnmKind::LightFlip(off) => {
            work(if *off { Work::LightOff } else { Work::LightOn }, event);
            act.base = relevant;
        }
        TnmKind::Con { con, bind, check, check_false, timer, breathe } => {
            if by_player && tnm_con(flags, con, bind, check, *check_false, *timer, *breathe) {
                act.base = true;
            }
        }
        TnmKind::ConForce { con, bind, check, check_false, parts, used, limit } => {
            if relevant && by_player {
                con_force(flags, con, bind, check, *check_false, parts);
                spend(used, *limit, once, &mut act);
            }
        }
        TnmKind::BlowUp { used, limit } => {
            if relevant {
                work(Work::BlowUp, event);
                spend(used, *limit, once, &mut act);
            }
        }
        TnmKind::Achievement(name) => {
            if relevant {
                crate::tnmach::unlock(name);
                act.destroy = once;
            }
        }
        // OxygenZoneTrigger's Touch does nothing.
        TnmKind::Oxygen { .. } => {}
        TnmKind::BindName { .. } | TnmKind::Limited(_) | TnmKind::Autosave => act.base = relevant,
        // Trigger only.
        TnmKind::LoudNoise { .. } | TnmKind::CutSound { .. } | TnmKind::CutShake { .. } | TnmKind::Grav(_) => {}
        TnmKind::NoteAdd { tag, package } => {
            act.base = relevant;
            if relevant && by_player {
                note_add(tag, package);
            }
        }
        TnmKind::WeaponSense { failed, in_hand, cause } => {
            if relevant && by_player {
                if weapon_sensed(*in_hand, *cause) {
                    act.base = true;
                } else if !failed.is_empty() {
                    act.events.push((failed.clone(), true));
                }
            }
        }
        TnmKind::TurretOff(turret) => {
            if relevant {
                crate::staring::disable(event, *turret);
                act.destroy = once;
            }
        }
    }
    act
}

pub fn on_untouch(k: &mut TnmKind, event: &str, once: bool, relevant: bool) -> Act {
    let mut act = Act::default();
    match k {
        TnmKind::BreakEnter => {
            if relevant {
                break_enter(event, false);
                act.destroy = once;
            }
        }
        TnmKind::CeSeq(_) | TnmKind::BindName { .. } | TnmKind::Limited(_) | TnmKind::Autosave | TnmKind::LightFlip(_) => {
            act.base = relevant;
        }
        _ => {}
    }
    act
}

/// Being triggered (on) or untriggered (off) by an event.
pub fn on_trigger(k: &mut TnmKind, event: &str, once: bool, on: bool, flags: &mut FlagBase) -> Act {
    let mut act = Act::default();
    if !on {
        match k {
            TnmKind::BreakEnter => {
                break_enter(event, false);
                act.events.push((event.to_string(), true));
                act.destroy = once;
            }
            TnmKind::Inert => act.base = true,
            _ => {}
        }
        return act;
    }
    match k {
        TnmKind::CeSeq(seq) => {
            set_seq(event, *seq);
            act.base = true;
        }
        TnmKind::BindName { new, check } => {
            crate::ai::world_req(WorldReq::Rebind(Sel::tag(event), new.clone(), check.clone()));
            act.base = true;
        }
        TnmKind::GoalAdd { name, text, primary, completed } => {
            goal(name, text, *primary, *completed);
            act.base = true;
            act.destroy = true;
        }
        TnmKind::Limited(n) => {
            if *n > 0 {
                act.events.push((event.to_string(), true));
                *n -= 1;
            }
        }
        TnmKind::BreakEnter => {
            break_enter(event, true);
            act.events.push((event.to_string(), true));
            act.destroy = once;
        }
        TnmKind::Destroy => {
            work(Work::Destroy, event);
            act.base = true;
        }
        TnmKind::AiEvent(_) => {}
        TnmKind::Visibility(hide) => PLAYER_HIDDEN.store(*hide, std::sync::atomic::Ordering::Relaxed),
        TnmKind::Orders { orders, tag } => {
            crate::triggers::push_order(crate::triggers::PawnOrder::Orders {
                tag: event.to_string(),
                orders: orders.clone(),
                orders_tag: tag.clone(),
            });
            act.base = true;
            act.destroy = once;
        }
        TnmKind::Autosave => {
            crate::save::autosave("trigger");
            act.base = true;
            act.destroy = once;
        }
        TnmKind::BotDisabler => {
            crate::ai::world_req(WorldReq::Disable(Sel::tag(event)));
            act.base = true;
            act.destroy = once;
        }
        TnmKind::Invasion => act.destroy = true,
        TnmKind::Relay { how, used, limit } => {
            relay(*how, event, &mut act);
            spend(used, *limit, once, &mut act);
        }
        TnmKind::Insane { start } => {
            INSANE.store(*start, std::sync::atomic::Ordering::Relaxed);
            act.destroy = once;
        }
        TnmKind::LightFlip(off) => {
            work(if *off { Work::LightOff } else { Work::LightOn }, event);
            act.base = true;
        }
        TnmKind::Oxygen { toggle, value } => crate::zones::ask_oxygen(event, *toggle, *value),
        TnmKind::Con { con, bind, check, check_false, timer, breathe } => {
            if tnm_con(flags, con, bind, check, *check_false, *timer, *breathe) {
                act.base = true;
            }
        }
        TnmKind::ConForce { con, bind, check, check_false, parts, used, limit } => {
            con_force(flags, con, bind, check, *check_false, parts);
            spend(used, *limit, once, &mut act);
        }
        TnmKind::BlowUp { used, limit } => {
            work(Work::BlowUp, event);
            spend(used, *limit, once, &mut act);
        }
        TnmKind::Achievement(name) => {
            crate::tnmach::unlock(name);
            act.base = true;
            act.destroy = once;
        }
        TnmKind::Inert => act.base = true,
        TnmKind::LoudNoise { pos, volume, radius } => crate::smartai::push_noise(crate::stealth::Noise {
            pos: *pos,
            volume: *volume,
            radius: *radius,
            maker: Some(crate::ai::Target::Player),
            instigator: Some(crate::ai::Target::Player),
            kind: crate::stealth::NoiseKind::Loud,
        }),
        TnmKind::NoteAdd { tag, package } => {
            act.base = true;
            note_add(tag, package);
        }
        TnmKind::CutSound { sound, volume } => {
            act.base = true;
            if !sound.is_empty() {
                crate::sound::play_player(sound, *volume, 0.0);
            }
        }
        TnmKind::CutShake { start, end, time } => {
            // (tnmCameraPoint's ShakeMagnitude ramp: one shake of the mean strength for the time.)
            let m = (*start + *end) * 0.5;
            crate::shake::shake_view(time.max(0.1), m, m * 0.5);
            act.destroy = once;
        }
        TnmKind::WeaponSense { .. } => act.base = true,
        TnmKind::Grav(cmds) => {
            if let Ok(mut q) = GRAV.lock() {
                q.extend(cmds.iter().cloned());
            }
        }
        TnmKind::TurretOff(turret) => {
            crate::staring::disable(event, *turret);
            act.base = true;
            act.destroy = once;
        }
    }
    act
}

// -------------------------------------------------------------------------------------------------- AI listeners

/// AIEventTriggers of this map: (trigger index in triggers.rs, event kind, where, once).
static LISTENERS: Mutex<Vec<(usize, String, DVec3, bool, bool)>> = Mutex::new(Vec::new());

/// triggers.rs registers each AIEventTrigger when it builds its list.
pub fn listen(index: usize, kind: &str, pos: DVec3, once: bool) {
    if let Ok(mut l) = LISTENERS.lock() {
        l.push((index, kind.to_string(), pos, once, false));
    }
}

fn hear_ai_events() {
    let evs: Vec<_> = AI_EVENTS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    if evs.is_empty() {
        return;
    }
    let Ok(mut l) = LISTENERS.lock() else { return };
    let mut fired = Vec::new();
    for (idx, kind, pos, once, spent) in l.iter_mut() {
        if *spent {
            continue;
        }
        if evs.iter().any(|(k, at, r)| kind.as_str() == *k && at.distance(*pos) <= *r) {
            fired.push(*idx);
            if *once {
                *spent = true;
            }
        }
    }
    drop(l);
    if !fired.is_empty() {
        if let Ok(mut q) = AI_FIRED.lock() {
            q.extend(fired);
        }
    }
}

// ----------------------------------------------------------------------------------------------- the elevators

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    Idle,
    /// Closing the doors (and the slave) at the floor it's leaving.
    Closing,
    Moving,
    /// Doors open at a floor; closing them again after the open time when it runs the doors.
    Open(f32),
}

struct Ce {
    mover: usize,
    tag: String,
    times: [f32; 7],
    events: [String; 8],
    start_events: [String; 8],
    start_event: String,
    doors: bool,
    door_open: f32,
    realistic: bool,
    factor: f32,
    slave: String,
    slave_ctl: bool,
    phase: Phase,
    target: u8,
    request: Option<u8>,
    /// Waiting this long before going on to the floor asked for while it moved.
    request_wait: f32,
    old_key: u8,
    move_time: f32,
    /// A close was asked of the doors (waiting for them to shut).
    closing_sent: bool,
}

struct Dispatch {
    tag: String,
    event: String,
    steps: Vec<(f32, u8)>,
    run: Option<(usize, f32)>,
}

#[derive(Resource, Default)]
pub struct TnmMovers {
    built: bool,
    ces: Vec<Ce>,
    dispatchers: Vec<Dispatch>,
    /// TrackMovers (not the advanced kind): (mover, resetChance, randomized base and added move time).
    tracks: Vec<(usize, i32, Option<(i32, i32)>)>,
    seed: u64,
    inertial: Vec<Inertial>,
}

/// InertialMover: a fan that spins up and down and blows the air of its zone about.
struct Inertial {
    mover: usize,
    tag: String,
    on: bool,
    rate_on: [f64; 3],
    rate_off: [f64; 3],
    up: f64,
    down: f64,
    zone: String,
    /// RotationRate as the map saved it, taken up on the first tick.
    start: Option<[f64; 3]>,
    /// zoneGravity On / Off: change per tick, min, max.
    grav: [[DVec3; 3]; 2],
    kill: bool,
}

/// Movers the player's footing matters to: weakmovers (they break under him) and killing fans.
static FOOTING: Mutex<Vec<usize>> = Mutex::new(Vec::new());
static STOOD: Mutex<Vec<usize>> = Mutex::new(Vec::new());

/// lib.rs, once a frame while walking: the player stands on one of the movers that care (returns true when a fan
/// at speed kills him).
pub fn player_footing(level: &dx_level::Level, movers: &crate::movers::Movers, loc: DVec3, radius: f64, height: f64) {
    let Ok(list) = FOOTING.lock() else { return };
    if list.is_empty() {
        return;
    }
    if let Some(i) = crate::movers::standing_on(level, movers, loc, radius, height, &list) {
        if let Ok(mut q) = STOOD.lock() {
            q.push(i);
        }
    }
}

fn name_arr<const N: usize>(p: &[Property], n: &str) -> [String; N] {
    let mut out: [String; N] = std::array::from_fn(|_| String::new());
    for x in p.iter().filter(|x| x.name.eq_ignore_ascii_case(n)) {
        if let (Some(slot), Value::Name(v)) = (out.get_mut(x.index as usize), &x.value) {
            if v != "None" {
                *slot = v.clone();
            }
        }
    }
    out
}

fn build(lvl: &dx_level::Level, st: &mut TnmMovers) {
    st.ces.clear();
    st.dispatchers.clear();
    st.tracks.clear();
    st.inertial.clear();
    st.seed = 0x7472_6163_6B6D_6F76;
    let mut footing = Vec::new();
    for (mi, m) in lvl.movers.iter().enumerate() {
        let a = &lvl.actors[m.actor];
        if a.class.eq_ignore_ascii_case("weakmover") {
            footing.push(mi);
        }
        if a.class.eq_ignore_ascii_case("InertialMover") {
            let p = &a.props;
            let rot = |n: &str| match find(p, n) {
                Some(Value::Rotator(r)) => [r[0] as f64, r[1] as f64, r[2] as f64],
                _ => [0.0; 3],
            };
            let v = |n: &str| match find(p, n) {
                Some(Value::Vector(v)) => DVec3::new(v[0] as f64, v[1] as f64, v[2] as f64),
                _ => DVec3::ZERO,
            };
            let fl = |n: &str| match find(p, n) {
                Some(Value::Float(v)) => *v as f64,
                _ => 0.0,
            };
            let on = b(p, "bOn", false);
            let kill = b(p, "bKillSupportee", false);
            if kill {
                footing.push(mi);
            }
            st.inertial.push(Inertial {
                mover: mi,
                tag: m.tag.clone(),
                on,
                rate_on: rot("DesiredRotationRateOn"),
                rate_off: rot("DesiredRotationRateOff"),
                up: fl("RotationSpeedUpRate"),
                down: fl("RotationSlowDownRate"),
                zone: s(p, "FanZoneName"),
                start: Some(rot("RotationRate")),
                grav: [
                    [v("zoneGravityOnChange"), v("zoneGravityOnMin"), v("zoneGravityOnMax")],
                    [v("zoneGravityOffChange"), v("zoneGravityOffMin"), v("zoneGravityOffMax")],
                ],
                kill,
            });
            println!("TNM fan {} on {on} zone '{}'", a.name, s(p, "FanZoneName"));
        }
        if a.class.eq_ignore_ascii_case("TrackMover") && !b(&a.props, "bAdvancedTrackMover", false) {
            let p = &a.props;
            st.tracks.push((
                mi,
                i(p, "resetChance", 1).max(1),
                b(p, "bRandomizedMoveTime", false).then(|| (i(p, "baseMoveTime", 0), i(p, "addRandomMoveTime", 1).max(1))),
            ));
        }
        if !a.class.eq_ignore_ascii_case("CaroneElevator") {
            continue;
        }
        let p = &a.props;
        let mut times = [0.0f32; 7];
        for x in p.iter().filter(|x| x.name.eq_ignore_ascii_case("CEMoveTimes")) {
            if let (Some(t), Value::Float(v)) = (times.get_mut(x.index as usize), &x.value) {
                *t = *v;
            }
        }
        let f = |n: &str, d: f32| match find(p, n) {
            Some(Value::Float(v)) => *v,
            _ => d,
        };
        st.ces.push(Ce {
            mover: mi,
            tag: m.tag.clone(),
            times,
            events: name_arr(p, "CEEvents"),
            start_events: name_arr(p, "CEStartEvents"),
            start_event: s(p, "CEStartEvent"),
            doors: b(p, "bCEControlsDoors", false),
            door_open: f("CEDoorOpenTime", 0.0),
            realistic: b(p, "bRealisticMovetime", false),
            factor: f("RealisticEFactor", 0.0),
            slave: s(p, "CESlaveMover"),
            slave_ctl: b(p, "bCEControlsSlave", false),
            phase: Phase::Idle,
            target: 0,
            request: None,
            request_wait: 0.0,
            old_key: 0,
            move_time: 0.0,
            closing_sent: false,
        });
    }
    for a in &lvl.actors {
        if !a.class.eq_ignore_ascii_case("CESequenceDispatcher") {
            continue;
        }
        let p = &a.props;
        let mut seqs = [-1i32; 8];
        let mut delays = [0.0f32; 8];
        for x in &a.props {
            let k = x.index as usize;
            if k >= 8 {
                continue;
            }
            if x.name.eq_ignore_ascii_case("CESeqnums") {
                if let Value::Int(v) = x.value {
                    seqs[k] = v;
                }
            } else if x.name.eq_ignore_ascii_case("CEOutDelays") {
                if let Value::Float(v) = x.value {
                    delays[k] = v;
                }
            }
        }
        // The class default is 0 for every slot; only values set in the map (and in range) are used. A slot left
        // at 0 after a set one still runs, as the original's loop does for any value other than -1.
        let steps = (0..8).filter(|&k| seqs[k] >= 0 && seqs[k] < 8).map(|k| (delays[k], seqs[k] as u8)).collect();
        st.dispatchers.push(Dispatch { tag: s(p, "Tag"), event: s(p, "Event"), steps, run: None });
    }
    if let Ok(mut f) = FOOTING.lock() {
        *f = footing;
    }
    st.built = true;
}

/// Doors of a floor: (any open, any moving).
fn doors(lvl: &dx_level::Level, movers: &crate::movers::Movers, tag: &str) -> (bool, bool) {
    let (mut open, mut moving) = (false, false);
    if tag.is_empty() {
        return (false, false);
    }
    for (j, m) in lvl.movers.iter().enumerate() {
        if m.tag.eq_ignore_ascii_case(tag) {
            let rt = &movers.rt[j];
            moving |= rt.interpolating;
            open |= rt.key_num != 0 && !rt.interpolating;
        }
    }
    (open, moving)
}

fn fire(events: &mut Vec<(String, bool)>, e: &str) {
    if !e.is_empty() {
        events.push((e.to_string(), true));
    }
}

impl Ce {
    /// SetSeq(n).
    fn set_seq(&mut self, n: u8, key: u8) {
        if matches!(self.phase, Phase::Closing | Phase::Moving) {
            if n != self.target {
                self.request = Some(n);
            }
            return;
        }
        self.request = None;
        if n == key {
            // Already there: the doors open (and close again later when it runs them).
            self.phase = Phase::Open(-1.0);
            return;
        }
        let (a, b) = (key.min(n) as usize, key.max(n) as usize);
        let mut t: f32 = (a..b).map(|k| self.times.get(k).copied().unwrap_or(0.0)).sum();
        if self.realistic {
            t = (t * (1.0 - self.factor * (b - a) as f32)).max(0.0);
        }
        self.move_time = t;
        self.target = n;
        self.old_key = key;
        self.closing_sent = false;
        self.phase = Phase::Closing;
    }
}

pub fn tnm_system(
    time: Res<Time>,
    level: Res<crate::LevelRes>,
    mut st: ResMut<TnmMovers>,
    mut movers: ResMut<crate::movers::Movers>,
    mut events: ResMut<crate::devices::GameEvents>,
    blockers: Res<crate::actors::Blockers>,
    mut ph: ResMut<crate::ai::PlayerHealth>,
    inv: Res<crate::pickups::PlayerInventory>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut gen: Local<u64>,
) {
    if !crate::tnmmode::active() {
        return;
    }
    if inv.is_changed() {
        let list: Vec<(u8, bool)> = inv
            .0
            .items
            .iter()
            .filter(|i| crate::classtree::is_a(&i.def.class, "DeusExWeapon"))
            .map(|i| {
                let d = assets.classes.defaults(&i.def.class);
                let conc = match find(&d, "Concealability") {
                    Some(Value::Byte(b)) => *b,
                    _ => 0,
                };
                (conc, matches!(find(&d, "bNoMetal"), Some(Value::Bool(true))))
            })
            .collect();
        if let Ok(mut c) = CARRIED.lock() {
            *c = list;
        }
    }
    if crate::loader::new_map(&mut gen) {
        st.built = false;
    }
    let lvl = &level.0;
    if !st.built {
        build(lvl, &mut st);
        crate::zones::live_init(lvl);
    }
    for b in GRAV.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        let mut r = unreal_pkg::Reader::new(&b, 0);
        let zone = r.compact_index().ok().and_then(|i| lvl.names.get(i as usize).cloned()).unwrap_or_default();
        let num = r.i32().unwrap_or(0);
        if !zone.is_empty() && !zone.eq_ignore_ascii_case("None") {
            crate::zones::change_grav(lvl, &zone, Some(num), 0);
        }
    }
    hear_ai_events();
    let dt = time.delta_secs().min(0.1);
    let mut out: Vec<(String, bool)> = Vec::new();

    // Floors asked for.
    for (tag, n) in CE_REQS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        for ce in st.ces.iter_mut().filter(|c| c.tag.eq_ignore_ascii_case(&tag)) {
            let key = movers.rt[ce.mover].key_num;
            ce.set_seq(n, key);
        }
    }
    // Events: dispatchers start, door buttons (an event with an elevator's tag) open or shut its doors.
    for (name, on) in INBOX.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        if !on {
            continue;
        }
        // uc: InertialMover.Trigger
        for f in st.inertial.iter_mut().filter(|f| !f.tag.is_empty() && f.tag.eq_ignore_ascii_case(&name)) {
            f.on = !f.on;
        }
        for d in st.dispatchers.iter_mut().filter(|d| d.tag.eq_ignore_ascii_case(&name)) {
            if d.run.is_none() {
                d.run = Some((0, 0.0));
            }
        }
        for ce in st.ces.iter_mut().filter(|c| c.tag.eq_ignore_ascii_case(&name)) {
            if ce.phase == Phase::Idle || matches!(ce.phase, Phase::Open(_)) {
                let k = movers.rt[ce.mover].key_num as usize;
                fire(&mut out, &ce.events[k.min(7)]);
                if ce.slave_ctl {
                    fire(&mut out, &ce.slave);
                }
            }
        }
    }
    // Dispatchers on their timetable.
    let mut seqs: Vec<(String, u8)> = Vec::new();
    for d in st.dispatchers.iter_mut() {
        let Some((step, t)) = d.run.as_mut() else { continue };
        *t += dt;
        while let Some(&(delay, seq)) = d.steps.get(*step) {
            if *t < delay {
                break;
            }
            *t -= delay;
            seqs.push((d.event.clone(), seq));
            *step += 1;
        }
        if *step >= d.steps.len() {
            d.run = None;
        }
    }
    for (tag, n) in seqs {
        for ce in st.ces.iter_mut().filter(|c| c.tag.eq_ignore_ascii_case(&tag)) {
            let key = movers.rt[ce.mover].key_num;
            ce.set_seq(n, key);
        }
    }
    // Each elevator's trip.
    for ce in st.ces.iter_mut() {
        let rt_moving = movers.rt[ce.mover].interpolating;
        let key = movers.rt[ce.mover].key_num as usize;
        if env_on!("DXVIEW_CELOG") && ce.phase != Phase::Idle {
            println!("CE {} {:?} key {key} moving {rt_moving} target {} req {:?}", ce.tag, ce.phase, ce.target, ce.request);
        }
        match ce.phase {
            Phase::Idle => {
                if let Some(n) = ce.request {
                    ce.request_wait -= dt;
                    if ce.request_wait <= 0.0 {
                        ce.request = None;
                        ce.set_seq(n, key as u8);
                    }
                }
            }
            Phase::Closing => {
                let floor = ce.events[ce.old_key as usize].clone();
                let (open, moving) = if ce.doors { doors(lvl, &movers, &floor) } else { (false, false) };
                let (s_open, s_moving) = if ce.slave_ctl { doors(lvl, &movers, &ce.slave) } else { (false, false) };
                if open && !moving && !ce.closing_sent {
                    fire(&mut out, &floor);
                    ce.closing_sent = true;
                }
                if s_open && !s_moving && !ce.closing_sent {
                    fire(&mut out, &ce.slave);
                    ce.closing_sent = true;
                }
                if !open && !moving && !s_open && !s_moving {
                    // Off it goes.
                    fire(&mut out, &ce.start_event);
                    fire(&mut out, &ce.start_events[ce.old_key as usize]);
                    let info = &lvl.movers[ce.mover];
                    crate::movers::move_to(info, &mut movers.rt[ce.mover], ce.target, ce.move_time as f64);
                    ce.phase = Phase::Moving;
                    println!("ELEVATOR {} -> floor {} in {:.1} s", ce.tag, ce.target, ce.move_time);
                }
            }
            Phase::Moving => {
                if !rt_moving {
                    ce.old_key = key as u8;
                    if ce.doors {
                        ce.phase = Phase::Open(-1.0);
                    } else {
                        fire(&mut out, &ce.events[key.min(7)]);
                        ce.phase = Phase::Idle;
                        ce.request_wait = ce.door_open;
                    }
                }
            }
            Phase::Open(t) => {
                if t < 0.0 {
                    // Arrived (or asked for the floor it's at): open the doors where they're shut.
                    let floor = ce.events[key.min(7)].clone();
                    let (open, moving) = doors(lvl, &movers, &floor);
                    if moving {
                        continue;
                    }
                    if !ce.doors || !open {
                        fire(&mut out, &floor);
                        if ce.slave_ctl {
                            fire(&mut out, &ce.slave);
                        }
                    }
                    let door_time = lvl
                        .movers
                        .iter()
                        .find(|m| m.tag.eq_ignore_ascii_case(&floor))
                        .map_or(0.0, |m| m.move_time);
                    ce.phase = if ce.doors { Phase::Open(door_time + ce.door_open.max(0.01)) } else { Phase::Idle };
                    ce.request_wait = ce.door_open;
                } else {
                    let left = t - dt;
                    if left <= 0.0 || ce.request.is_some() {
                        // The doors shut again (CloseDoors), then any floor asked for meanwhile.
                        let floor = ce.events[key.min(7)].clone();
                        let (open, moving) = doors(lvl, &movers, &floor);
                        if open && !moving {
                            fire(&mut out, &floor);
                        }
                        ce.phase = Phase::Idle;
                        ce.request_wait = 0.0;
                    } else {
                        ce.phase = Phase::Open(left);
                    }
                }
            }
        }
    }
    // InertialMover.Tick: the spin eases toward the on / off rate (the original adds its rates once a tick; at its
    // usual 60 a second here), and the fan's zone gains or loses its wind.
    // uc: InertialMover.Tick, InertialMover.SupportActor, weakmover.SupportActor
    let stood = STOOD.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    let ticks = dt as f64 * 60.0;
    for f in st.inertial.iter_mut() {
        let rt = &mut movers.rt[f.mover];
        let dead = rt.destroyed;
        if let Some(r) = f.start.take() {
            rt.spin = r;
        }
        if !dead {
            let want = if f.on { f.rate_on } else { f.rate_off };
            for k in 0..3 {
                if rt.spin[k] < want[k] {
                    rt.spin[k] = (rt.spin[k] + f.up * ticks).min(want[k]);
                } else if rt.spin[k] > want[k] {
                    rt.spin[k] = (rt.spin[k] - f.down * ticks).max(want[k]);
                }
            }
        } else {
            rt.spin = [0.0; 3];
        }
        if !f.zone.is_empty() {
            let g = &f.grav[if !dead && f.on { 0 } else { 1 }];
            crate::zones::push_gravity(lvl, &f.zone, g[0] * ticks, g[1], g[2]);
        }
        if f.kill && !dead && stood.contains(&f.mover) && rt.spin.iter().map(|v| v.abs()).sum::<f64>() > 20000.0 && !ph.dead {
            crate::missions::player_req(crate::missions::PlayerReq::Kill("Exploded"));
        }
    }
    for &mi in &stood {
        if lvl.actors[lvl.movers[mi].actor].class.eq_ignore_ascii_case("weakmover") && !movers.rt[mi].destroyed {
            crate::movers::damage_mover(mi, 100000.0, "Exploded");
        }
    }
    // TrackMover.Tick: at rest on its last key it jumps back to the first (one chance in resetChance a tick, with a
    // new random MoveTime when asked); at rest on the first it sets off again.
    // uc: TrackMover.Tick
    if !st.tracks.is_empty() {
        let mut seed = st.seed;
        let mut rnd = |n: i32| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n.max(1) as u64) as i32
        };
        for &(mi, chance, random) in &st.tracks {
            let info = &lvl.movers[mi];
            let rt = &mut movers.rt[mi];
            if rt.interpolating || rt.destroyed {
                continue;
            }
            if rt.key_num + 1 == info.num_keys {
                if rnd(chance) == 0 {
                    crate::movers::move_to(info, rt, 0, 0.001);
                }
            } else if rt.key_num == 0 {
                match random {
                    Some((base, add)) if info.num_keys == 2 => {
                        crate::movers::move_to(info, rt, 1, (rnd(add) + base).max(0) as f64 + 0.001)
                    }
                    _ => crate::movers::trigger(info, rt),
                }
            }
        }
        st.seed = seed;
    }
    // Actor work asked for by triggers.
    for (w, tag) in WORK.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        match w {
            Work::Destroy => {
                crate::ai::world_req(WorldReq::Destroy(Sel::tag(&tag)));
                crate::scriptworld::push(crate::scriptworld::ActorReq::Destroy(crate::scriptworld::ActorSel::tag(&tag)));
            }
            Work::BlowUp => {
                for (j, m) in lvl.movers.iter().enumerate() {
                    if m.tag.eq_ignore_ascii_case(&tag) {
                        crate::movers::damage_mover(j, 100000.0, "Exploded");
                    }
                }
            }
            Work::Frob => {
                for (bi, c) in blockers.list.iter().enumerate() {
                    let a = &lvl.actors[c.actor];
                    if crate::devices::name_prop(&a.props, "Tag").eq_ignore_ascii_case(&tag) {
                        if crate::fixtures::is_fixture(&a.class) {
                            crate::fixtures::frob(bi);
                        }
                        let ev = crate::devices::name_prop(&a.props, "Event");
                        if !ev.is_empty() {
                            out.push((ev, true));
                        }
                    }
                }
            }
            Work::LightOn | Work::LightOff => {
                let want = w == Work::LightOn;
                for (bi, c) in blockers.list.iter().enumerate() {
                    let a = &lvl.actors[c.actor];
                    if crate::devices::name_prop(&a.props, "Tag").eq_ignore_ascii_case(&tag)
                        && crate::fixtures::is_fixture(&a.class)
                    {
                        crate::fixtures::set_switch(bi, want);
                    }
                }
            }
        }
    }
    // The shadow code's voices: drugged view while insane.
    if insane() && !ph.dead {
        ph.drug_timer = ph.drug_timer.max(2.0);
    }
    for e in FIRED.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        out.push((e, true));
    }
    if !out.is_empty() {
        events.0.extend(out);
    }
}

static FIRED: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// gravtrigger commands waiting for the level (the zone name is an index into the map's names).
static GRAV: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());

/// An event fired by code that doesn't hold the GameEvents (a knock on a KnockableMover).
pub fn fire_event(name: &str) {
    if let Ok(mut q) = FIRED.lock() {
        q.push(name.to_string());
    }
}
