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
                .or_insert_with(|| match find(p, "hackStrength") {
                    Some(Value::Float(v)) => *v as f64,
                    _ => 0.2,
                });
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
                let valid = match find(p, "validCode") {
                    Some(Value::Str(v)) => v.clone(),
                    _ => String::new(),
                };
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
    // Plain decorations (buttons, switches): DeusExDecoration.Frob triggers Event.
    let e = name_prop(p, "Event");
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
            hack_action(&p, &mut events);
            crate::alarms::hacked(blockers.list[b].actor);
            crate::security::hacked(b);
            stop = true;
        } else if left <= 0 || !holding || !looking {
            stop = true;
        }
        if stop {
            hacking.job = None;
            if inv.0.use_one("Multitool") == 0 && holding {
                weapons.held = None;
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
    for (name, on) in events.0.drain(..) {
        println!("EVENT {name} {}", if on { "trigger" } else { "untrigger" });
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
        crate::sound::tag_state(&name, on);
        for (j, info) in lvl.movers.iter().enumerate() {
            if on && info.tag.eq_ignore_ascii_case(&name) {
                crate::movers::trigger(info, &mut movers.rt[j]);
            }
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
