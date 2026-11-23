//! Events and devices: an actor's Event triggers every actor whose Tag matches (doors open/close, electricity
//! turns on/off); buttons and switches fire their Event when used (DeusExDecoration.Frob); hackable devices
//! (control panels, keypads) are bypassed with a multitool (HackableDevices.Frob/Timer: SkillTech value x 100
//! steps of 0.01 hack strength over hackTime) and then fire Event / UnTriggerEvent; a keypad also takes its code.

use crate::actors::{Blockers, Deco};
use crate::movers::{HudMessage, Movers};
use crate::LevelRes;
use bevy::prelude::*;
use std::collections::HashMap;
use unreal_pkg::props::{find, Property, Value};

/// Events raised this frame: (name, true = Trigger / false = UnTrigger).
#[derive(Resource, Default)]
pub struct GameEvents(pub Vec<(String, bool)>);

static STRENGTH_SETS: std::sync::Mutex<Vec<(String, f64)>> = std::sync::Mutex::new(Vec::new());

/// A mission script sets the hack strength left on the devices (keypads, boxes) with this Tag (0 = bypassed).
pub fn set_strength(tag: &str, strength: f64) {
    if let Ok(mut q) = STRENGTH_SETS.lock() {
        q.push((tag.to_string(), strength));
    }
}

/// SkillTech value (0.1 untrained .. 0.75 master).
pub fn tech_skill() -> f64 {
    crate::abilities::skill("SkillTech") as f64
}

#[derive(Resource, Default)]
pub struct Hacking {
    /// Remaining hack strength per device (blocker index).
    pub strength: HashMap<usize, f64>,
    pub job: Option<HackJob>,
    /// A keypad waiting for its code: (blocker, digits typed).
    pub keypad: Option<(usize, String)>,
}

pub struct HackJob {
    pub blocker: usize,
    pub left: i32,
    pub ticks: f64,
    pub per: f64,
}

/// GMDX Wireless Strength (DeusExPlayer.HighlightCenterObject): with a multitool in hand and nothing in frob reach,
/// the first hackable device (bHackable, strength left) within PerkValue (768) units along the view, in sight.
pub fn wireless_target(
    lvl: &dx_level::Level,
    blockers: &Blockers,
    hacking: &Hacking,
    eye: glam::DVec3,
    dir: glam::DVec3,
) -> Option<(usize, f64)> {
    // (Modern deathmatch: the Netrunner's Wireless Hacking reaches 12 metres.)
    let range = crate::perks::value_if("PerkWirelessStrength")
        .or_else(|| (crate::dmchar::class_perk() == "netrunner").then_some(630.0))? as f64;
    if !crate::perks::holding("Multitool") {
        return None;
    }
    let end = eye + dir * range;
    let wall = lvl.collision.trace(eye, end, glam::DVec3::splat(1.0)).fraction;
    let mut best: Option<(usize, f64)> = None;
    for (b, c) in blockers.list.iter().enumerate() {
        if c.radius <= 0.0 {
            continue;
        }
        let p = &lvl.actors[c.actor].props;
        if !is_hackable(p) || matches!(find(p, "bHackable"), Some(Value::Bool(false))) {
            continue;
        }
        let s = *hacking.strength.get(&b).unwrap_or(&crate::devices::initial_strength(p));
        if s <= 0.0 {
            continue;
        }
        if let Some(t) = crate::pickups::ray_cylinder(eye, end, c.center, c.radius, c.height) {
            if t <= wall + 0.01 && best.is_none_or(|(_, f)| t < f) {
                best = Some((b, t));
            }
        }
    }
    best
}

/// Decorations whose Event a script changed: (Tag, new Event). Cleared on a new map.
static EVENT_SET: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

/// Actor.Event = `event` for the decorations with this Tag (The Nameless Mod's TV switch).
pub fn set_event(tag: &str, event: &str) {
    if let Ok(mut q) = EVENT_SET.lock() {
        q.retain(|(t, _)| !t.eq_ignore_ascii_case(tag));
        q.push((tag.to_string(), event.to_string()));
    }
}

/// Keypads whose code a script set: (Tag, code) - The Nameless Mod's archive keypad picks one of five.
static CODE_SET: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

pub fn set_code(tag: &str, code: &str) {
    if let Ok(mut q) = CODE_SET.lock() {
        q.retain(|(t, _)| !t.eq_ignore_ascii_case(tag));
        q.push((tag.to_string(), code.to_string()));
    }
}

/// A keypad's validCode, as a script last set it.
pub fn code_of(p: &[Property]) -> String {
    let tag = name_prop(p, "Tag");
    if let Ok(q) = CODE_SET.lock() {
        if let Some((_, c)) = q.iter().find(|(t, _)| !tag.is_empty() && t.eq_ignore_ascii_case(&tag)) {
            return c.clone();
        }
    }
    match find(p, "validCode") {
        Some(Value::Str(v)) => v.clone(),
        _ => String::new(),
    }
}

pub fn reset_events() {
    if let Ok(mut q) = EVENT_SET.lock() {
        q.clear();
    }
    if let Ok(mut q) = CODE_SET.lock() {
        q.clear();
    }
    crate::ai::clear_player_hitter();
}

fn event_of(p: &[Property]) -> String {
    let tag = name_prop(p, "Tag");
    if !tag.is_empty() {
        if let Ok(q) = EVENT_SET.lock() {
            if let Some((_, e)) = q.iter().find(|(t, _)| t.eq_ignore_ascii_case(&tag)) {
                return e.clone();
            }
        }
    }
    name_prop(p, "Event")
}

pub fn name_prop(p: &[Property], n: &str) -> String {
    match find(p, n) {
        Some(Value::Name(s)) if s != "None" => s.clone(),
        _ => String::new(),
    }
}

fn names(p: &[Property], n: &str) -> Vec<String> {
    p.iter()
        .filter(|x| x.name == n)
        .filter_map(|x| match &x.value {
            Value::Name(s) if s != "None" && !s.is_empty() => Some(s.clone()),
            _ => None,
        })
        .collect()
}

/// HackableDevices.PostBeginPlay: hackStrength clamped to 0..1, and 1.0 when the device can't be hacked (the frob
/// box's INF then has a full green bar).
pub fn initial_strength(p: &[Property]) -> f64 {
    if matches!(find(p, "bHackable"), Some(Value::Bool(false))) {
        return 1.0;
    }
    match find(p, "hackStrength") {
        Some(Value::Float(v)) => (*v as f64).clamp(0.0, 1.0),
        _ => 0.2,
    }
}

pub fn is_hackable(p: &[Property]) -> bool {
    find(p, "hackStrength").is_some()
}

/// HackAction(bHacked = true) for the device's class: control panels untrigger UnTriggerEvent and trigger Event;
/// keypads (ToggleLocks/RunEvents/RunUntriggers) the same.
pub fn hack_action(p: &[Property], events: &mut GameEvents) {
    for u in names(p, "UnTriggerEvent") {
        crate::triggers::note_source(&u, &crate::triggers::group_of(p));
        events.0.push((u, false));
    }
    let e = name_prop(p, "Event");
    if !e.is_empty() {
        crate::triggers::note_source(&e, &crate::triggers::group_of(p));
        events.0.push((e, true));
    }
}

// ---- The Nameless Mod's CreditReader (a HackableDevices: pay its cost or bypass it; then it opens what OpenEvent
// names every time it is used). A paid / bypassed reader is kept as hack strength -1 (saved with the others).
// uc: CreditReader.HackAction, CreditReader.Bought, CreditReader.Hacked, Trestkon.tryBuy, Trestkon.purchaseitems

/// The reader waiting for the player's yes / no: (blocker, cost, [bought, declined, can't afford] messages).
static READER_ASK: std::sync::Mutex<Option<(usize, i32, [String; 3])>> = std::sync::Mutex::new(None);

pub fn is_reader(class: &str) -> bool {
    crate::tnmmode::active() && class.eq_ignore_ascii_case("CreditReader")
}

/// Trestkon's shop strings: [window title, question, bought, declined, can't afford].
pub fn frob_reader(blocker: usize, p: &[Property], texts: [String; 5], alarm_off: bool, hacking: &mut Hacking, events: &mut GameEvents) {
    let strength = *hacking.strength.entry(blocker).or_insert_with(|| initial_strength(p));
    let open = name_prop(p, "OpenEvent");
    let cost = match find(p, "cost") {
        Some(Value::Int(v)) => *v,
        _ => 0,
    };
    let bought = strength < -0.5 || matches!(find(p, "bBought"), Some(Value::Bool(true)));
    if bought || strength <= 0.0 {
        if !bought {
            // Hacked(): HackEvent (the shop's alarm, unless it was switched off), then Bought().
            let hack = name_prop(p, "HackEvent");
            if !hack.is_empty() && !alarm_off {
                events.0.push((hack, true));
            }
            hacking.strength.insert(blocker, -1.0);
        }
        if !open.is_empty() {
            events.0.push((open, true));
        }
        return;
    }
    let [title, question, a, b, c] = texts;
    crate::menus::open_yes_no(&title, &question.replacen("%d", &cost.to_string(), 1));
    if let Ok(mut q) = READER_ASK.lock() {
        *q = Some((blocker, cost, [a, b, c]));
    }
}

/// What using a device with the given tool does (called from the frob system). Returns a message.
pub fn frob_device(
    blocker: usize,
    p: &[Property],
    class: &str,
    tool: Option<&str>,
    hacking: &mut Hacking,
    events: &mut GameEvents,
) -> Option<String> {
    if is_hackable(p) {
        let strength =
            *hacking
                .strength
                .entry(blocker)
                .or_insert_with(|| crate::devices::initial_strength(p));
        let hackable = !matches!(find(p, "bHackable"), Some(Value::Bool(false)));
        if tool == Some("Multitool") && hackable {
            if hacking.job.is_some() {
                return Some("Bypassing the device...".into());
            }
            if strength > 0.0 {
                let hack_time = match find(p, "HackTime") {
                    Some(Value::Float(v)) => *v as f64,
                    _ => 4.0,
                };
                let n = (tech_skill() * 100.0).round() as i32;
                hacking.job = Some(HackJob {
                    blocker,
                    left: n,
                    ticks: 0.0,
                    per: hack_time * 10.0 / n.max(1) as f64,
                });
                return Some("Bypassing the device...".into());
            }
            hack_action(p, events);
            return Some("It's already bypassed".into());
        }
        if tool.is_none() {
            if class.starts_with("Keypad") {
                // Keypad.HackAction -> ActivateKeypadWindow: HUDKeypadWindow; a bypassed keypad grants access at once.
                let valid = code_of(p);
                crate::menus::open_keypad(blocker, valid, strength <= 0.0);
                return None;
            }
            if strength <= 0.0 {
                // HackAction(Frobber, hackStrength == 0): a bypassed camera or turret switches each time.
                hack_action(p, events);
                crate::security::hacked(blocker);
                return None;
            }
            return Some("It's secure".into());
        }
        return None;
    }
    // Plain decorations (buttons, switches): DeusExDecoration.Frob triggers Event (as a script last set it).
    let e = event_of(p);
    if !e.is_empty() {
        crate::triggers::note_source(&e, &crate::triggers::group_of(p));
        events.0.push((e, true));
    }
    None
}

/// Progress a multitool bypass, the keypad window, and dispatch events.
#[allow(clippy::too_many_arguments)]
pub fn device_system(
    time: Res<Time>,
    _keys: Res<ButtonInput<KeyCode>>,
    level: Res<LevelRes>,
    blockers: Res<Blockers>,
    mut hacking: ResMut<Hacking>,
    mut events: ResMut<GameEvents>,
    mut movers: ResMut<Movers>,
    mut hud: ResMut<HudMessage>,
    hl: Res<crate::frobui::FrobHighlight>,
    (mut weapons, mut inv): (
        ResMut<crate::weapons::Weapons>,
        ResMut<crate::pickups::PlayerInventory>,
    ),
    mut emitters: Query<&mut crate::electricity::Emitter>,
    mut decos: Query<&mut Deco>,
) {
    let _prof = crate::prof::scope("devices::device_system");
    let hacking = &mut *hacking;
    let lvl = &level.0;
    let props = |b: usize| &lvl.actors[blockers.list[b].actor].props;
    // Mission scripts: hackStrength set on the devices with a tag.
    for (tag, s) in STRENGTH_SETS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        for (b, c) in blockers.list.iter().enumerate() {
            if name_prop(&lvl.actors[c.actor].props, "Tag").eq_ignore_ascii_case(&tag) {
                hacking.strength.insert(b, s);
            }
        }
    }
    // Multitool bypass in progress.
    if let Some(job) = hacking.job.as_mut() {
        job.ticks += time.delta_secs_f64() * 10.0;
        let b = job.blocker;
        let mut s = *hacking.strength.get(&b).unwrap_or(&0.0);
        while job.ticks > job.per && job.left > 0 {
            job.left -= 1;
            s = (s - 0.01).clamp(0.0, 1.0);
            job.ticks -= job.per;
        }
        let left = job.left;
        hacking.strength.insert(b, s);
        let holding = weapons
            .held
            .as_ref()
            .map(|h| h.stats.class == "Multitool")
            .unwrap_or(false);
        let looking = hl.target.as_ref().and_then(|t| t.blocker) == Some(b);
        let mut stop = false;
        if s <= 1e-6 {
            hacking.strength.insert(b, 0.0);
            hud.text = "You bypassed the device".into();
            hud.time_left = 2.0;
            let p = props(b).clone();
            if is_reader(&lvl.actors[blockers.list[b].actor].class) {
                // CreditReader.HackAction(bHacked): Hacked() = HackEvent, then Bought() = OpenEvent.
                for n in ["HackEvent", "OpenEvent"] {
                    let e = name_prop(&p, n);
                    if !e.is_empty() {
                        events.0.push((e, true));
                    }
                }
                hacking.strength.insert(b, -1.0);
            }
            hack_action(&p, &mut events);
            crate::alarms::hacked(blockers.list[b].actor);
            crate::security::hacked(b);
            stop = true;
        } else if left <= 0 || !holding || !looking {
            stop = true;
        }
        if stop {
            hacking.job = None;
            // GMDX Cracked (HackableDevices.StopHacking): a finished hack of a device whose initial strength is at
            // most PerkValue (5%) leaves the multitool unused.
            let initial = match find(props(b), "hackStrength") {
                Some(Value::Float(v)) => *v,
                _ => 0.2,
            };
            let free = s <= 1e-6 && crate::perks::value_if("PerkCracked").is_some_and(|v| initial <= v + 1e-4);
            if !free && inv.0.use_one("Multitool") == 0 && holding {
                weapons.held = None;
            }
        }
    }
    // The Nameless Mod's CreditReader: the answer to its purchase question.
    if READER_ASK.lock().is_ok_and(|q| q.is_some()) {
        if let Some(yes) = crate::menus::take_box_answer() {
            if let Some((b, cost, [bought, declined, cant])) = READER_ASK.lock().ok().and_then(|mut q| q.take()) {
                let msg = if !yes {
                    declined
                } else if inv.0.credits as i64 >= cost as i64 {
                    inv.0.credits = (inv.0.credits as i64 - cost as i64).max(0) as _;
                    hacking.strength.insert(b, -1.0);
                    let open = name_prop(props(b), "OpenEvent");
                    if !open.is_empty() {
                        events.0.push((open, true));
                    }
                    bought.replacen("%d", &cost.to_string(), 1)
                } else {
                    cant.replacen("%d", &cost.to_string(), 1)
                };
                crate::classic_hud::client_message(&msg);
            }
        }
    }
    // HUDKeypadWindow.ValidateCode results: Event (or the locks of the doors it names, bToggleLock) and the
    // UnTriggerEvents on success, FailEvent on failure.
    for (b, ok) in crate::menus::take_keypad_results() {
        let p = props(b).clone();
        if ok {
            let ev = name_prop(&p, "Event");
            if !ev.is_empty() {
                if matches!(find(&p, "bToggleLock"), Some(Value::Bool(true))) {
                    // DeusExPlayer.KeypadToggleLocks: every mover tagged Event locks or unlocks.
                    for (j, info) in lvl.movers.iter().enumerate() {
                        if info.tag.eq_ignore_ascii_case(&ev) {
                            let rt = &mut movers.rt[j];
                            rt.locked = !rt.locked;
                        }
                    }
                } else {
                    crate::triggers::note_source(&ev, &crate::triggers::group_of(&p));
                    events.0.push((ev, true));
                }
            }
            for u in names(&p, "UnTriggerEvent") {
                events.0.push((u, false));
            }
            if let Some(s) = crate::sound::prop(&p, "successSound") {
                crate::sound::play_player(&s, 1.0, 0.0);
            }
        } else {
            let fe = name_prop(&p, "FailEvent");
            if !fe.is_empty() {
                events.0.push((fe, true));
            }
            if let Some(s) = crate::sound::prop(&p, "failureSound") {
                crate::sound::play_player(&s, 1.0, 0.0);
            }
        }
    }
    // Dispatch.
    events.0.extend(crate::movers::EVENTS_OUT.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default());
    // Multiplayer: a joiner's events are the host's to run (he echoes each to everyone, below); the host tells the
    // joiners about each one he runs.
    for (name, on) in crate::netplay::take_events() {
        // The joiner's side of an event the host ran: what shows and sounds here (lights, particles, sounds, the
        // pain zones this player walks in, electricity). Doors come with the host's mover states.
        crate::particles::event(&name, on);
        crate::dynlight::trigger_event(lvl, &name, on);
        crate::zones::event(lvl, &name, on);
        crate::sound::tag_state(&name, on);
        for mut e in &mut emitters {
            if e.tag.eq_ignore_ascii_case(&name) {
                e.set_on(on);
            }
        }
    }
    for (name, on) in events.0.drain(..) {
        if crate::netplay::dispatched(&name, on) {
            continue;
        }
        println!("EVENT {name} {}", if on { "trigger" } else { "untrigger" });
        if on {
            crate::achievements::event(&lvl.name, &name);
        }
        crate::triggers::dispatch(&name, on);
        crate::logic::event(&name, on);
        crate::particles::event(&name, on);
        crate::projgen::event(&name, on);
        if on {
            crate::destruct::trigger_event(&name);
        }
        crate::security::event(&name, on);
        crate::cinematic::event(&name, on);
        crate::dynlight::trigger_event(lvl, &name, on);
        crate::zones::event(lvl, &name, on);
        crate::sound::tag_state(&name, on);
        for (j, info) in lvl.movers.iter().enumerate() {
            // A CaroneElevator has no Trigger of its own (The Nameless Mod: tnmtriggers.rs answers its door
            // buttons).
            if on && info.tag.eq_ignore_ascii_case(&name) && lvl.actors[info.actor].class != "CaroneElevator" && !crate::staring::is_staring(j) {
                crate::movers::trigger(info, &mut movers.rt[j]);
            }
        }
        crate::tnmtriggers::event(&name, on);
        crate::staring::event(&name, on);
        if on {
            crate::warpzone::event(&name);
        }
        for mut e in &mut emitters {
            if e.tag.eq_ignore_ascii_case(&name) {
                e.set_on(on);
            }
        }
        // Switches flip their On/Off pose when used (Switch1.Frob) - not drawn yet.
        let _ = &mut decos;
    }
}
