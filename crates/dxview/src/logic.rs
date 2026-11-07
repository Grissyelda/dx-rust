//! Event-logic actors that aren't touch triggers (Engine / DeusEx `Triggers` subclasses):
//!
//! * Dispatcher: when triggered, fires OutEvents[0..8] in order, sleeping OutDelays[i] before each; it ignores
//!   further triggers until it has finished (state Dispatch: disable('Trigger')).
//! * SpecialEvent: the message to the instigator (ClientMessage) in every state; PlaySoundEffect /
//!   PlayersPlaySoundEffect also play Sound; DamageInstigator / KillInstigator hurt or kill the player.
//! * SequenceTrigger: ElevatorMover.SetSeq(SeqNum) on the movers tagged Event, when triggered (or touched, if it
//!   collides).
//! * DamageTrigger: on unless the map says bIsOn False (Trigger turns it on, UnTrigger off); while on, the player
//!   touching it takes DamageAmount of DamageType at the trigger's location every damageInterval (bConstantDamage
//!   repeats, else once per touch). The training's toxic pool is one: 10 'Radiation' a second while you're in it.

use crate::LevelRes;
use bevy::prelude::*;
use glam::DVec3;
use std::sync::Mutex;
use unreal_pkg::props::{find, Property, Value};

static INBOX: Mutex<Vec<(String, bool)>> = Mutex::new(Vec::new());

/// DamageTriggers' (bIsOn, bInitiallyActive) by list index, as they are now (saves) and as a save restores them.
static DAMAGE_STATE: Mutex<Vec<(usize, bool, bool)>> = Mutex::new(Vec::new());
static RESTORE: Mutex<Option<Vec<(usize, bool, bool)>>> = Mutex::new(None);

/// For saves: every DamageTrigger's (index, bIsOn, bInitiallyActive).
pub fn damage_state() -> Vec<(usize, bool, bool)> {
    DAMAGE_STATE.lock().map(|v| v.clone()).unwrap_or_default()
}

/// A save is loaded: its DamageTriggers' states apply once the list is built.
pub fn set_damage_state(v: &[(usize, bool, bool)]) {
    if let Ok(mut r) = RESTORE.lock() {
        *r = Some(v.to_vec());
    }
}

/// A new map: drop the old map's states.
pub fn reset_level() {
    if let Ok(mut v) = DAMAGE_STATE.lock() {
        v.clear();
    }
}

/// An event was raised this frame (devices.rs dispatch).
pub fn event(name: &str, on: bool) {
    if let Ok(mut q) = INBOX.lock() {
        q.push((name.to_string(), on));
    }
}

fn name(p: &[Property], n: &str) -> String {
    match find(p, n) {
        Some(Value::Name(v)) if v != "None" => v.clone(),
        Some(Value::Str(v)) => v.clone(),
        _ => String::new(),
    }
}
fn fl(p: &[Property], n: &str, d: f32) -> f32 {
    match find(p, n) {
        Some(Value::Float(v)) => *v,
        Some(Value::Int(v)) => *v as f32,
        _ => d,
    }
}
fn bl(p: &[Property], n: &str, d: bool) -> bool {
    match find(p, n) {
        Some(Value::Bool(v)) => *v,
        _ => d,
    }
}
fn indexed(p: &[Property], n: &str) -> Vec<(u32, Value)> {
    p.iter()
        .filter(|x| x.name.eq_ignore_ascii_case(n))
        .map(|x| (x.index, x.value.clone()))
        .collect()
}

enum Kind {
    Dispatcher {
        out: [(String, f32); 8],
        /// Running: (next OutEvents index, time left before it fires).
        run: Option<(usize, f32)>,
    },
    Special {
        message: String,
        state: String,
        sound: Option<String>,
        damage: f32,
        damage_type: String,
    },
    Sequence {
        seq: u8,
        collide: bool,
        touching: bool,
    },
    Damage {
        amount: f32,
        interval: f32,
        constant: bool,
        damage_type: String,
        /// bIsOn (Trigger turns it on, UnTrigger off).
        on: bool,
        /// Trigger.IsRelevant: bInitiallyActive, switched by the OtherTrigger* states, and a TriggerType the player
        /// counts for.
        active: bool,
        state: String,
        player_type: bool,
        touching: bool,
        /// Touch set `damagee` (the player).
        damagee: bool,
        /// The actor timer: (time left, rate, looping).
        timer: Option<(f32, f32, bool)>,
    },
}

struct Logic {
    group: String,
    tag: String,
    event: String,
    pos: DVec3,
    radius: f64,
    height: f64,
    kind: Kind,
}

#[derive(Resource, Default)]
pub struct LogicActors {
    list: Vec<Logic>,
    built: bool,
}

impl LogicActors {
}

fn build(level: &dx_level::Level) -> Vec<Logic> {
    let mut out = Vec::new();
    for a in &level.actors {
        let p = &a.props;
        let kind = match a.class.as_str() {
            "Dispatcher" => {
                let mut o: [(String, f32); 8] = Default::default();
                for (i, v) in indexed(p, "OutEvents") {
                    if let (Some(s), Value::Name(n)) = (o.get_mut(i as usize), v) {
                        if n != "None" {
                            s.0 = n;
                        }
                    }
                }
                for (i, v) in indexed(p, "OutDelays") {
                    if let (Some(s), Value::Float(d)) = (o.get_mut(i as usize), v) {
                        s.1 = d;
                    }
                }
                Kind::Dispatcher { out: o, run: None }
            }
            "SpecialEvent" => Kind::Special {
                message: name(p, "Message"),
                state: name(p, "InitialState"),
                sound: crate::sound::prop(p, "Sound"),
                damage: fl(p, "Damage", 0.0),
                damage_type: name(p, "DamageType"),
            },
            "SequenceTrigger" => Kind::Sequence {
                seq: fl(p, "SeqNum", 0.0).max(0.0) as u8,
                collide: bl(p, "bCollideActors", false),
                touching: false,
            },
            "DamageTrigger" => Kind::Damage {
                amount: fl(p, "DamageAmount", 5.0),
                interval: fl(p, "damageInterval", 0.5),
                constant: bl(p, "bConstantDamage", true),
                damage_type: name(p, "DamageType"),
                on: bl(p, "bIsOn", true),
                active: bl(p, "bInitiallyActive", true),
                state: name(p, "InitialState"),
                // TT_PlayerProximity, TT_PawnProximity, TT_AnyProximity take the player.
                player_type: matches!(find(p, "TriggerType"), None | Some(Value::Byte(0 | 1 | 3))),
                touching: false,
                damagee: false,
                timer: None,
            },
            _ => continue,
        };
        let l = a.location();
        out.push(Logic {
            group: crate::triggers::group_of(p),
            tag: name(p, "Tag"),
            event: name(p, "Event"),
            pos: DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64),
            radius: fl(p, "CollisionRadius", 40.0) as f64,
            height: fl(p, "CollisionHeight", 40.0) as f64,
            kind,
        });
    }
    out
}

fn fire(ev: &str, on: bool) {
    if ev.is_empty() {
        return;
    }
    if let Ok(mut q) = crate::movers::EVENTS_OUT.lock() {
        q.push((ev.to_string(), on));
    }
}

fn seq_movers(level: &dx_level::Level, movers: &mut crate::movers::Movers, event: &str, seq: u8) {
    if event.is_empty() {
        return;
    }
    for (j, info) in level.movers.iter().enumerate() {
        if info.tag.eq_ignore_ascii_case(event) && level.actors[info.actor].class == "ElevatorMover" {
            crate::movers::set_seq(info, &mut movers.rt[j], seq);
        }
    }
}

pub fn logic_system(
    time: Res<Time>,
    level: Res<LevelRes>,
    mut st: ResMut<LogicActors>,
    mut movers: ResMut<crate::movers::Movers>,
    mut ph: ResMut<crate::ai::PlayerHealth>,
    player: Res<crate::Player>,
    cam: Query<&Transform, With<crate::FlyCam>>,
) {
    // Multiplayer: the host's to run (a joiner sees the results through him).
    if crate::netplay::story_off() {
        return;
    }
    let lvl = &level.0;
    if !st.built {
        st.list = build(lvl);
        st.built = true;
    }
    if let Some(r) = RESTORE.lock().ok().and_then(|mut r| r.take()) {
        for (i, o, a) in r {
            if let Some(Kind::Damage { on, active, .. }) = st.list.get_mut(i).map(|l| &mut l.kind) {
                *on = o;
                *active = a;
            }
        }
    }
    let inbox: Vec<(String, bool)> = INBOX
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default();
    let dt = time.delta_secs().min(0.1);
    let pc = player.body.location;
    let (pr, phh) = (player.params.radius, player.params.height);
    for l in st.list.iter_mut() {
        let hit: Vec<bool> = inbox
            .iter()
            .filter(|(n, _)| !l.tag.is_empty() && n.eq_ignore_ascii_case(&l.tag))
            .map(|(_, on)| *on)
            .collect();
        let inside = (l.pos - pc).truncate().length() < l.radius + pr && (l.pos.z - pc.z).abs() < l.height + phh;
        match &mut l.kind {
            Kind::Dispatcher { out, run } => {
                if run.is_none() && hit.iter().any(|on| *on) {
                    *run = Some((0, out[0].1));
                }
                // Each OutEvents slot in turn (empty ones are skipped without sleeping).
                while let Some((i, left)) = run.as_mut() {
                    if *i >= out.len() {
                        *run = None;
                        break;
                    }
                    if out[*i].0.is_empty() {
                        *i += 1;
                        if *i < out.len() {
                            *left = out[*i].1;
                        }
                        continue;
                    }
                    *left -= dt;
                    if *left > 0.0 {
                        break;
                    }
                    crate::triggers::note_source(&out[*i].0, &l.group);
                    fire(&out[*i].0, true);
                    *i += 1;
                    if *i < out.len() {
                        *left = out[*i].1;
                    }
                }
            }
            Kind::Special { message, state, sound, damage, damage_type } => {
                for _ in hit.iter().filter(|on| **on) {
                    if !message.is_empty() {
                        crate::classic_hud::client_message(message);
                    }
                    match state.as_str() {
                        "PlaySoundEffect" | "PlayersPlaySoundEffect" => {
                            if let Some(s) = sound {
                                crate::sound::play_player(s, 1.0, 0.0);
                            }
                        }
                        "DamageInstigator" => {
                            let t = if damage_type.is_empty() { "None" } else { damage_type.as_str() };
                            ph.damage(*damage, t, DVec3::ZERO);
                        }
                        "KillInstigator" => {
                            ph.damage(10000.0, "None", DVec3::ZERO);
                        }
                        _ => {}
                    }
                }
            }
            Kind::Sequence { seq, collide, touching } => {
                let touched = *collide && inside && !*touching;
                *touching = *collide && inside;
                if hit.iter().any(|on| *on) || touched {
                    seq_movers(lvl, &mut movers, &l.event, *seq);
                }
            }
            Kind::Damage { amount, interval, constant, damage_type, on, active, state, player_type, touching, damagee, timer } => {
                // DamageTrigger.Touch: nothing while off or not relevant; else damagee = the toucher and the timer
                // runs at damageInterval (looping with bConstantDamage).
                let touch = |on: bool, active: bool, damagee: &mut bool, timer: &mut Option<(f32, f32, bool)>| {
                    if on && active && *player_type {
                        *damagee = true;
                        *timer = Some((*interval, *interval, *constant));
                    }
                };
                for &h in &hit {
                    if h {
                        // DamageTrigger.Trigger: bIsOn = True, then Trigger.Trigger in its state.
                        *on = true;
                        match state.as_str() {
                            "OtherTriggerToggles" => {
                                *active = !*active;
                                if *active && *touching {
                                    touch(*on, *active, damagee, timer); // CheckTouchList
                                }
                            }
                            "OtherTriggerTurnsOn" => {
                                let was = *active;
                                *active = true;
                                if !was && *touching {
                                    touch(*on, *active, damagee, timer);
                                }
                            }
                            "OtherTriggerTurnsOff" => *active = false,
                            _ => {}
                        }
                    } else {
                        *on = false; // UnTrigger
                    }
                }
                let (entered, left) = (inside && !*touching, !inside && *touching);
                *touching = inside;
                if entered {
                    touch(*on, *active, damagee, timer);
                } else if left && *on {
                    // UnTouch: damagee = None, SetTimer(0.1, False).
                    *damagee = false;
                    *timer = Some((0.1, 0.1, false));
                }
                if let Some((t, rate, looping)) = timer.as_mut() {
                    *t -= dt;
                    if *t <= 0.0 {
                        if !*on {
                            // Timer while off: SetTimer(0.1, False) and wait.
                            *timer = Some((0.1, 0.1, false));
                        } else {
                            if *damagee {
                                // TakeDamage(damageAmount, None, Location, ...): hit at the trigger's own location.
                                let ty = if damage_type.is_empty() { "None" } else { damage_type.as_str() };
                                let yaw = cam.single().map_or(0.0, |t| {
                                    let f = t.forward().as_vec3();
                                    (f.z as f64).atan2(f.x as f64)
                                });
                                ph.damage(*amount, ty, crate::ai::player_offset(pc, yaw, l.pos));
                                if env_on!("DXVIEW_ZONELOG") {
                                    println!("DMGTRIG {} {ty} hp {:.0}", *amount, ph.hp);
                                }
                            }
                            if *looping {
                                *t += *rate;
                            } else {
                                *timer = None;
                            }
                        }
                    }
                }
            }
        }
    }
    // Saves read the DamageTriggers' switches from here (rewritten only when one changed).
    let damage = st.list.iter().enumerate().filter_map(|(i, l)| match l.kind {
        Kind::Damage { on, active, .. } => Some((i, on, active)),
        _ => None,
    });
    if let Ok(mut v) = DAMAGE_STATE.lock() {
        if !damage.clone().eq(v.iter().copied()) {
            v.clear();
            v.extend(damage);
        }
    }
}
