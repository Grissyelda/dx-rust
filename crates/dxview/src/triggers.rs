//! Triggers (Engine.Trigger and the Deus Ex subclasses placed in the maps): touched by the player (or a pawn,
//! by TriggerType) they fire their Event; triggered by an event they do their own thing.
//!
//! * Trigger: TT_PlayerProximity / TT_PawnProximity / TT_ClassProximity, bTriggerOnceOnly (collision off after
//!   firing), ReTriggerDelay, Message.
//! * DataLinkTrigger: starts its infolink (if CheckFlag allows, rechecking every second while touched) and
//!   fires its Event when the infolink is over.
//! * FlagTrigger: sets a flag, and/or passes the touch / trigger on while the flag has the wanted value.
//! * OrdersTrigger / AllianceTrigger: new orders or alliances for the pawns tagged Event.
//! * SkillAwardTrigger: skill points with a message. ConversationTrigger: starts a named conversation.
//!
//! Events go through `GameEvents`; anything an event reaches (by Tag) is triggered here, a frame later.

use crate::ai::{GameFlags, Npc};
use crate::LevelRes;
use bevy::prelude::*;
use glam::DVec3;
use imsim_core::flags::FlagValue;
use std::sync::Mutex;
use unreal_pkg::props::{find, Property, Value};

#[derive(Debug, Clone)]
enum Kind {
    Plain,
    /// LogicTrigger: inGroup1 / inGroup2, Op (AND, OR, XOR), Not, OneShot, in1, in2, outhit.
    Logic {
        g1: String,
        g2: String,
        op: u8,
        not: bool,
        one_shot: bool,
        in1: bool,
        in2: bool,
        hit: bool,
    },
    /// ShakeTrigger: shakeTime, shakeRollMagnitude, shakeVertMagnitude.
    Shake(f32, f32, f32),
    /// FadeViewTrigger: fadeColor, fadeTime, postFadeTime, bFadeDown; time left while fading.
    Fade {
        color: [f32; 3],
        fade: f32,
        post: f32,
        down: bool,
        time: Option<f32>,
    },
    DataLink {
        tag: String,
        check_flag: String,
        check_false: bool,
        timer: Option<f32>,
        started: bool,
    },
    Flag {
        name: String,
        value: bool,
        set: bool,
        trigger: bool,
        while_standing: bool,
        expiration: i32,
    },
    Orders {
        orders: String,
        orders_tag: String,
    },
    Alliance {
        alliance: String,
        list: Vec<(String, f32, bool)>,
    },
    SkillAward {
        points: i32,
        message: String,
    },
    Conversation {
        con: String,
        bind: String,
        check_flag: String,
        check_false: bool,
    },
}

#[derive(Debug, Clone)]
pub struct Trig {
    pub actor: usize,
    class: String,
    pos: DVec3,
    radius: f64,
    height: f64,
    tag: String,
    event: String,
    trigger_type: u8,
    class_prox: String,
    once: bool,
    /// bCollideActors (off once a bTriggerOnceOnly trigger has fired).
    collide: bool,
    active: bool,
    /// InitialState: NormalTrigger, OtherTriggerToggles, OtherTriggerTurnsOn, OtherTriggerTurnsOff.
    state: String,
    retrigger: f32,
    last_time: f32,
    message: String,
    touching: bool,
    destroyed: bool,
    kind: Kind,
    group: String,
    /// DirectionalTrigger: only touchers on the side its rotation faces count ((Location - Other.Location) . X <= 0).
    dir: Option<DVec3>,
}

#[derive(Resource, Default)]
pub struct Triggers {
    pub list: Vec<Trig>,
    built: bool,
    /// bWhileStandingOnly flags checked against where the player arrived (see trigger_system).
    standing_checked: bool,
    time: f32,
}

/// What a save keeps of each trigger (save.rs). `touching` is kept so a trigger the player stood in when saving
/// doesn't fire again on load (the original saves its Touching list).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TrigSave {
    pub actor: usize,
    pub active: bool,
    pub collide: bool,
    pub destroyed: bool,
    pub touching: bool,
    pub state: String,
}

impl Triggers {
    pub fn is_built(&self) -> bool {
        self.built
    }
    pub fn save(&self) -> Vec<TrigSave> {
        self.list
            .iter()
            .map(|t| TrigSave {
                actor: t.actor,
                active: t.active,
                collide: t.collide,
                destroyed: t.destroyed,
                touching: t.touching,
                state: t.state.clone(),
            })
            .collect()
    }
    pub fn restore(&mut self, v: &[TrigSave]) {
        for s in v {
            if let Some(t) = self.list.iter_mut().find(|t| t.actor == s.actor) {
                t.active = s.active;
                t.collide = s.collide;
                t.destroyed = s.destroyed;
                t.touching = s.touching;
                t.state = s.state.clone();
            }
        }
    }
}

/// Events dispatched this frame, for the triggers to look at next frame.
static INBOX: Mutex<Vec<(String, bool)>> = Mutex::new(Vec::new());

/// Called by the event dispatch for every event.
pub fn dispatch(name: &str, on: bool) {
    if let Ok(mut q) = INBOX.lock() {
        q.push((name.to_string(), on));
    }
}

/// Orders and alliance changes for tagged pawns (ScriptedPawn.SetOrders / SetAlliance / ChangeAlly).
#[derive(Debug, Clone)]
pub enum PawnOrder {
    Orders {
        tag: String,
        orders: String,
        orders_tag: String,
    },
    Alliance {
        tag: String,
        alliance: String,
        list: Vec<(String, f32, bool)>,
    },
    /// Mission scripts: SetOrders on the pawns a selection picks.
    Script {
        sel: crate::ai::Sel,
        orders: String,
        orders_tag: String,
    },
    /// ChangeAlly(alliance, level, permanent) on the selected pawns.
    Ally {
        sel: crate::ai::Sel,
        alliance: String,
        level: f32,
        permanent: bool,
    },
}

static ORDERS: Mutex<Vec<PawnOrder>> = Mutex::new(Vec::new());

static COLLIDE: Mutex<Vec<(String, bool)>> = Mutex::new(Vec::new());

/// foreach AllActors(class'Trigger', trig, tag) trig.SetCollision(on).
pub fn set_collision(tag: &str, on: bool) {
    if let Ok(mut q) = COLLIDE.lock() {
        q.push((tag.to_string(), on));
    }
}

pub fn push_order(o: PawnOrder) {
    if let Ok(mut q) = ORDERS.lock() {
        q.push(o);
    }
}

pub fn take_orders() -> Vec<PawnOrder> {
    ORDERS
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
}

/// Put back orders that couldn't be applied yet (the pawn is in a conversation).
pub fn defer_orders(v: Vec<PawnOrder>) {
    if let Ok(mut q) = ORDERS.lock() {
        let mut v = v;
        v.append(&mut q);
        *q = v;
    }
}

fn s(p: &[Property], n: &str) -> String {
    match find(p, n) {
        Some(Value::Str(v)) => v.clone(),
        Some(Value::Name(v)) if v != "None" => v.clone(),
        _ => String::new(),
    }
}
fn b(p: &[Property], n: &str, d: bool) -> bool {
    match find(p, n) {
        Some(Value::Bool(v)) => *v,
        _ => d,
    }
}
fn f(p: &[Property], n: &str, d: f64) -> f64 {
    match find(p, n) {
        Some(Value::Float(v)) => *v as f64,
        _ => d,
    }
}
fn i(p: &[Property], n: &str, d: i32) -> i32 {
    match find(p, n) {
        Some(Value::Int(v)) => *v,
        Some(Value::Byte(v)) => *v as i32,
        _ => d,
    }
}

const CLASSES: [&str; 12] = [
    "Trigger",
    "DirectionalTrigger",
    "DataLinkTrigger",
    "FlagTrigger",
    "OrdersTrigger",
    "AllianceTrigger",
    "SkillAwardTrigger",
    "ConversationTrigger",
    "GoalCompleteTrigger",
    "LogicTrigger",
    "ShakeTrigger",
    "FadeViewTrigger",
];

fn build(level: &dx_level::Level) -> Vec<Trig> {
    let mut out = Vec::new();
    for (ai, a) in level.actors.iter().enumerate() {
        if !CLASSES.contains(&a.class.as_str()) {
            continue;
        }
        let p = &a.props;
        let l = a.location();
        let kind = match a.class.as_str() {
            "DataLinkTrigger" => Kind::DataLink {
                tag: s(p, "datalinkTag"),
                check_flag: s(p, "checkFlag"),
                check_false: b(p, "bCheckFalse", false),
                timer: None,
                started: false,
            },
            "FlagTrigger" => Kind::Flag {
                name: s(p, "FlagName"),
                value: b(p, "flagValue", true),
                set: b(p, "bSetFlag", true),
                trigger: b(p, "bTrigger", false),
                while_standing: b(p, "bWhileStandingOnly", false),
                expiration: i(p, "flagExpiration", -1),
            },
            "OrdersTrigger" => Kind::Orders {
                orders: s(p, "Orders"),
                orders_tag: s(p, "ordersTag"),
            },
            "AllianceTrigger" => Kind::Alliance {
                alliance: s(p, "Alliance"),
                list: p
                    .iter()
                    .filter(|x| x.name.eq_ignore_ascii_case("Alliances"))
                    .filter_map(|x| match &x.value {
                        Value::Alliance(n, lv, perm) if !n.is_empty() && n != "None" => {
                            Some((n.clone(), *lv, *perm))
                        }
                        _ => None,
                    })
                    .collect(),
            },
            "SkillAwardTrigger" => Kind::SkillAward {
                points: i(p, "skillPointsAdded", 10),
                message: s(p, "awardMessage"),
            },
            "ConversationTrigger" => Kind::Conversation {
                con: s(p, "conversationTag"),
                bind: s(p, "BindName"),
                check_flag: s(p, "checkFlag"),
                check_false: b(p, "bCheckFalse", false),
            },
            "LogicTrigger" => Kind::Logic {
                g1: s(p, "inGroup1"),
                g2: s(p, "inGroup2"),
                op: i(p, "Op", 0) as u8,
                not: b(p, "Not", false),
                one_shot: b(p, "OneShot", false),
                in1: false,
                in2: false,
                hit: false,
            },
            "ShakeTrigger" => Kind::Shake(
                f(p, "shakeTime", 0.0) as f32,
                f(p, "shakeRollMagnitude", 0.0) as f32,
                f(p, "shakeVertMagnitude", 0.0) as f32,
            ),
            "FadeViewTrigger" => Kind::Fade {
                color: match find(p, "fadeColor") {
                    Some(Value::Struct(_, v)) if v.len() >= 3 => [v[0] as f32 / 255.0, v[1] as f32 / 255.0, v[2] as f32 / 255.0],
                    _ => [0.0; 3],
                },
                fade: f(p, "fadeTime", 0.0) as f32,
                post: f(p, "postFadeTime", 0.0) as f32,
                down: b(p, "bFadeDown", false),
                time: None,
            },
            _ => Kind::Plain,
        };
        out.push(Trig {
            actor: ai,
            class: a.class.clone(),
            pos: DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64),
            radius: f(p, "CollisionRadius", 40.0),
            height: f(p, "CollisionHeight", 40.0),
            tag: s(p, "Tag"),
            event: s(p, "Event"),
            trigger_type: i(p, "TriggerType", 0) as u8,
            class_prox: match find(p, "ClassProximityType") {
                Some(Value::Path(v)) => v.rsplit('.').next().unwrap_or("").to_string(),
                _ => String::new(),
            },
            once: b(p, "bTriggerOnceOnly", false),
            collide: b(p, "bCollideActors", true),
            active: b(p, "bInitiallyActive", true),
            state: s(p, "InitialState"),
            retrigger: f(p, "ReTriggerDelay", 0.0) as f32,
            last_time: -1000.0,
            message: s(p, "Message"),
            touching: false,
            destroyed: false,
            group: group_of(p),
            kind,
            dir: (a.class == "DirectionalTrigger").then(|| {
                let r = match find(p, "Rotation") {
                    Some(Value::Rotator(r)) => *r,
                    _ => [0; 3],
                };
                let x = dx_level::actors::rotator_axes(r)[0];
                DVec3::new(x[0] as f64, x[1] as f64, x[2] as f64)
            }),
        });
    }
    out
}

/// What a touch / trigger call asks the outside world to do.
#[derive(Default)]
struct Out {
    events: Vec<(String, bool)>,
    messages: Vec<String>,
}

impl Out {
    /// Fire an event as this actor (LogicTrigger reads the instigator's Group).
    fn emit(&mut self, event: &str, group: &str, on: bool) {
        note_source(event, group);
        self.events.push((event.to_string(), on));
    }
}

/// Who fired the next event of a tag: the source actor's Group (LogicTrigger's Other.Group).
static SOURCES: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

pub fn note_source(event: &str, group: &str) {
    if event.is_empty() {
        return;
    }
    if let Ok(mut q) = SOURCES.lock() {
        q.push((event.to_ascii_lowercase(), group.to_string()));
        let n = q.len();
        if n > 64 {
            q.drain(..n - 64);
        }
    }
}

/// The Group of whatever fired this event (None = an actor without a Group).
fn take_source(event: &str) -> String {
    let e = event.to_ascii_lowercase();
    SOURCES
        .lock()
        .ok()
        .and_then(|mut q| q.iter().position(|x| x.0 == e).map(|i| q.remove(i).1))
        .unwrap_or_default()
}

/// The Group property ('None' = empty).
pub fn group_of(p: &[Property]) -> String {
    match find(p, "Group") {
        Some(Value::Name(g)) if g != "None" => g.clone(),
        _ => String::new(),
    }
}

/// Trigger.Touch (the base class): IsRelevant was checked by the caller.
fn base_touch(t: &mut Trig, now: f32, out: &mut Out) {
    if !t.active {
        return;
    }
    if t.retrigger > 0.0 {
        if now - t.last_time < t.retrigger {
            return;
        }
        t.last_time = now;
    }
    if !t.event.is_empty() {
        out.emit(&t.event, &t.group, true);
    }
    if !t.message.is_empty() {
        out.messages.push(t.message.clone());
    }
    if t.once {
        t.collide = false;
    }
}

fn base_untouch(t: &Trig, out: &mut Out) {
    if t.active && !t.event.is_empty() {
        out.emit(&t.event, &t.group, false);
    }
}

fn flag_ok(flags: &imsim_core::flags::FlagBase, check: &str, check_false: bool) -> bool {
    if check.is_empty() {
        return true;
    }
    if flags.get_bool(check) {
        !check_false
    } else {
        check_false
    }
}

/// Lay D Denton's ConversationTrigger: the female JC gets "FemJC" + conversationTag.
fn fem_con(flags: &imsim_core::flags::FlagBase, con: &str) -> String {
    if flags.get_bool("LDDPJCIsFemale") {
        format!("FemJC{con}")
    } else {
        con.to_string()
    }
}

fn set_flag(flags: &mut imsim_core::flags::FlagBase, name: &str, v: bool, expiration: i32) {
    // flagExpiration -1: SetBool without an expiration = the flag base's default (the next mission).
    if expiration == -1 {
        flags.set_bool(name, v);
    } else {
        flags.set(name, FlagValue::Bool(v), expiration);
    }
}

/// A touch by the player (`by_player`) or a pawn of a class.
fn touch(
    t: &mut Trig,
    by_player: bool,
    flags: &mut imsim_core::flags::FlagBase,
    now: f32,
    out: &mut Out,
) {
    match t.kind.clone() {
        Kind::DataLink {
            tag,
            check_flag,
            check_false,
            ..
        } => {
            if !by_player {
                return;
            }
            if flag_ok(flags, &check_flag, check_false) {
                if crate::consys::start_datalink(flags, &tag, Some(t.actor)) {
                    if let Kind::DataLink { started, .. } = &mut t.kind {
                        *started = true;
                    }
                }
            } else if let Kind::DataLink { timer, .. } = &mut t.kind {
                // bCheckFlagTimer: try again in checkFlagTimer (1 s) while still touching.
                *timer = Some(1.0);
            }
        }
        Kind::Flag {
            name,
            value,
            set,
            trigger,
            ..
        } => {
            if !relevant(t, by_player) {
                return;
            }
            if set {
                set_flag(
                    flags,
                    &name,
                    value,
                    if let Kind::Flag { expiration, .. } = t.kind {
                        expiration
                    } else {
                        -1
                    },
                );
            }
            if trigger && flags.get_bool(&name) == value {
                base_touch(t, now, out);
            }
        }
        Kind::Orders { orders, orders_tag } => {
            if !relevant(t, by_player) {
                return;
            }
            push_order(PawnOrder::Orders {
                tag: t.event.clone(),
                orders,
                orders_tag,
            });
            if t.once {
                t.destroyed = true;
            }
        }
        Kind::Alliance { alliance, list } => {
            if !relevant(t, by_player) {
                return;
            }
            push_order(PawnOrder::Alliance {
                tag: t.event.clone(),
                alliance,
                list,
            });
            if t.once {
                t.destroyed = true;
            }
        }
        Kind::SkillAward { points, message } => {
            if !relevant(t, by_player) {
                return;
            }
            base_touch(t, now, out);
            if by_player {
                if let Some(m) = crate::abilities::award_points(points) {
                    // SkillPointsAdd's line, then the award message.
                    out.messages.push(m);
                    out.messages.push(message);
                    crate::sound::play_player(
                        "DeusExSounds.UserInterface.LogSkillPoints",
                        1.0,
                        0.0,
                    );
                }
            }
        }
        Kind::Conversation {
            con,
            bind,
            check_flag,
            check_false,
        } => {
            if !by_player || con.is_empty() || bind.is_empty() {
                return;
            }
            if flag_ok(flags, &check_flag, check_false)
                && crate::consys::start_named(flags, &fem_con(flags, &con), &bind)
            {
                base_touch(t, now, out);
            }
        }
        Kind::Plain | Kind::Logic { .. } => {
            if relevant(t, by_player) {
                base_touch(t, now, out);
            }
        }
        Kind::Shake(time, roll, vert) => {
            if relevant(t, by_player) {
                if by_player {
                    crate::shake::shake_view(time, roll, vert);
                }
                base_touch(t, now, out);
            }
        }
        Kind::Fade { .. } => {
            if relevant(t, by_player) {
                base_touch(t, now, out);
                start_fade(t);
            }
        }
    }
}

/// FadeViewTrigger.FadeView: fadeTime at least 0.1, postFadeTime at least 0.
fn start_fade(t: &mut Trig) {
    if let Kind::Fade { fade, post, time, .. } = &mut t.kind {
        *fade = fade.max(0.1);
        *post = post.max(0.0);
        *time = Some(*fade + *post);
    }
}

fn untouch(t: &mut Trig, by_player: bool, flags: &mut imsim_core::flags::FlagBase, out: &mut Out) {
    match t.kind.clone() {
        Kind::DataLink { .. } => {
            if let Kind::DataLink { timer, .. } = &mut t.kind {
                *timer = None;
            }
            if relevant(t, by_player) {
                base_untouch(t, out);
            }
        }
        Kind::Flag {
            name,
            value,
            set,
            trigger,
            while_standing,
            expiration,
        } => {
            if while_standing && relevant(t, by_player) {
                if trigger && flags.get_bool(&name) == value {
                    base_untouch(t, out);
                }
                if set {
                    set_flag(flags, &name, !value, expiration);
                }
            }
        }
        Kind::Plain | Kind::Logic { .. } | Kind::Shake(..) | Kind::Fade { .. } => {
            if relevant(t, by_player) {
                base_untouch(t, out);
            }
        }
        _ => {}
    }
}

/// ClassIsChildOf(Other.Class, ClassProximityType) for a pawn or its body: Carcass / DeusExCarcass take any body,
/// <Class>Carcass that pawn's body, Pawn / ScriptedPawn any living pawn, else the class itself.
fn class_prox_matches(prox: &str, cls: &str, dead: bool) -> bool {
    let p = prox.to_ascii_lowercase();
    if dead {
        p == "carcass" || p == "deusexcarcass" || p.eq_ignore_ascii_case(&format!("{cls}carcass"))
    } else {
        p == "pawn" || p == "scriptedpawn" || p.eq_ignore_ascii_case(cls)
    }
}

/// Trigger.IsRelevant for the player (or a pawn).
fn relevant(t: &Trig, by_player: bool) -> bool {
    if !t.active {
        return false;
    }
    match t.trigger_type {
        0 => by_player,
        1 | 3 => true,
        2 => !by_player || t.class_prox.to_ascii_lowercase().contains("player"),
        _ => false,
    }
}

/// Being triggered by an event (the class's Trigger function).
fn triggered(
    t: &mut Trig,
    on: bool,
    flags: &mut imsim_core::flags::FlagBase,
    now: f32,
    out: &mut Out,
) {
    let _ = now;
    if !on {
        if let Kind::Flag {
            name,
            value,
            set,
            trigger,
            while_standing,
            expiration,
        } = t.kind.clone()
        {
            if while_standing {
                if trigger && flags.get_bool(&name) == value && !t.event.is_empty() {
                    out.emit(&t.event, &t.group, false);
                }
                if set {
                    set_flag(flags, &name, !value, expiration);
                }
            }
        }
        return;
    }
    match t.kind.clone() {
        Kind::DataLink {
            tag,
            check_flag,
            check_false,
            ..
        } => {
            if flag_ok(flags, &check_flag, check_false) {
                if crate::consys::start_datalink(flags, &tag, Some(t.actor)) {
                    if let Kind::DataLink { started, .. } = &mut t.kind {
                        *started = true;
                    }
                }
            } else if !check_flag.is_empty() {
                if let Kind::DataLink { started, .. } = &mut t.kind {
                    *started = true;
                }
            }
        }
        Kind::Flag {
            name,
            value,
            set,
            trigger,
            expiration,
            ..
        } => {
            if set {
                set_flag(flags, &name, value, expiration);
            }
            if trigger && flags.get_bool(&name) == value && !t.event.is_empty() {
                out.emit(&t.event, &t.group, true);
            }
        }
        Kind::Orders { orders, orders_tag } => {
            push_order(PawnOrder::Orders {
                tag: t.event.clone(),
                orders,
                orders_tag,
            });
            if t.once {
                t.destroyed = true;
            }
        }
        Kind::Alliance { alliance, list } => {
            push_order(PawnOrder::Alliance {
                tag: t.event.clone(),
                alliance,
                list,
            });
            if t.once {
                t.destroyed = true;
            }
        }
        Kind::SkillAward { points, message } => {
            if let Some(m) = crate::abilities::award_points(points) {
                out.messages.push(m);
                out.messages.push(message);
                crate::sound::play_player("DeusExSounds.UserInterface.LogSkillPoints", 1.0, 0.0);
            }
            // GMDX fix (Shifter; setting FixSkillAwardOnce): a bTriggerOnceOnly award fired by an event pays once
            // (the original only switched its collision off, so a repeated event paid again).
            if t.once && crate::settings::get("FixSkillAwardOnce") > 0.5 {
                t.destroyed = true;
            }
        }
        Kind::Conversation {
            con,
            bind,
            check_flag,
            check_false,
        } => {
            if flag_ok(flags, &check_flag, check_false) && !con.is_empty() && !bind.is_empty() {
                crate::consys::start_named(flags, &fem_con(flags, &con), &bind);
            }
        }
        Kind::Plain => trigger_state(t),
        Kind::Logic { g1, g2, op, not, one_shot, mut in1, mut in2, mut hit } => {
            // Other.Group == inGroupN (an actor with no Group matches an unset group).
            let src = take_source(&t.tag);
            if src.eq_ignore_ascii_case(&g1) {
                in1 = !in1;
            }
            if src.eq_ignore_ascii_case(&g2) {
                in2 = !in2;
            }
            let mut o = match op {
                0 => in1 && in2,
                1 => in1 || in2,
                _ => in1 ^ in2,
            };
            if not {
                o = !o;
            }
            if o && !hit {
                if one_shot {
                    hit = true;
                }
                if !t.event.is_empty() {
                    out.emit(&t.event, &t.group, true);
                }
            }
            println!("LOGIC {} from group '{src}': {in1} {in2} -> {o}", t.tag);
            t.kind = Kind::Logic { g1, g2, op, not, one_shot, in1, in2, hit };
            trigger_state(t);
        }
        Kind::Shake(time, roll, vert) => {
            crate::shake::shake_view(time, roll, vert);
            trigger_state(t);
        }
        Kind::Fade { .. } => {
            trigger_state(t);
            start_fade(t);
        }
    }
}

/// Trigger.Trigger in the OtherTrigger* states.
fn trigger_state(t: &mut Trig) {
    match t.state.as_str() {
        "OtherTriggerToggles" => t.active = !t.active,
        "OtherTriggerTurnsOn" => t.active = true,
        "OtherTriggerTurnsOff" => t.active = false,
        _ => {}
    }
}

/// FadeViewTrigger.Tick: InstantFog / InstantFlash while it runs, then the trigger is destroyed.
pub static FADE: Mutex<Option<([f32; 3], f32)>> = Mutex::new(None);

fn fade_tick(t: &mut Trig, dt: f32) {
    if let Kind::Fade { color, fade, post, down, time } = &mut t.kind {
        let Some(tm) = time.as_mut() else { return };
        *tm = (*tm - dt).max(0.0);
        let alpha = (1.0 - ((*tm - *post) / *fade)).clamp(0.0, 1.0);
        // InstantFlash alpha with InstantFog alpha x colour (fading up), or the scene darkened by alpha toward the
        // colour (bFadeDown): either way the screen goes to the colour by alpha.
        let _ = down;
        if let Ok(mut f) = FADE.lock() {
            *f = Some((*color, alpha));
        }
        if *tm <= 0.0 {
            *time = None;
            t.destroyed = true;
            if let Ok(mut f) = FADE.lock() {
                *f = None;
            }
        }
    }
}

fn overlaps(t: &Trig, c: DVec3, r: f64, h: f64) -> bool {
    let d = (t.pos - c).truncate().length();
    d < t.radius + r
        && (t.pos.z - c.z).abs() < t.height + h
}

/// DirectionalTrigger.Touch: the toucher has to come in from the front ((Location - Other.Location) dot X <= 0).
fn from_front(t: &Trig, c: DVec3) -> bool {
    t.dir.is_none_or(|x| (t.pos - c).dot(x) <= 0.0)
}

#[allow(clippy::too_many_arguments)]
pub fn trigger_system(
    time: Res<Time>,
    level: Res<LevelRes>,
    mut trig: ResMut<Triggers>,
    mut flags: ResMut<GameFlags>,
    player: Res<crate::Player>,
    ph: Res<crate::ai::PlayerHealth>,
    npcs: Query<(&Npc, &crate::actors::Deco, Option<&Visibility>)>,
    blockers: Res<crate::actors::Blockers>,
    mut events: ResMut<crate::devices::GameEvents>,
) {
    let _prof = crate::prof::scope("triggers::trigger_system");
    let tr = &mut *trig;
    if !tr.built {
        tr.built = true;
        tr.list = build(&level.0);
        println!("triggers: {}", tr.list.len());
    }
    // Wait for the conversation system (infolinks) to be ready, and for a saved game to finish loading.
    if crate::consys::lock().is_none() || crate::save::restoring() {
        return;
    }
    for (tag, on) in COLLIDE.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        for t in tr.list.iter_mut().filter(|t| t.tag.eq_ignore_ascii_case(&tag)) {
            t.collide = on;
        }
    }
    let dt = time.delta_secs();
    tr.time += dt;
    let now = tr.time;
    let fl = &mut flags.0;
    // A bWhileStandingOnly FlagTrigger's flag is only true while the player stands in it, but leaving the map from
    // inside one never untouches it, so the flag would stay set on the next map (training: MunitionsTrooperReady
    // carried into the combat course had Mission00 strip the inventory every second). On arrival, a flag whose
    // standing triggers don't hold the player goes back to !value, as the UnTouch would have done.
    if !tr.standing_checked {
        tr.standing_checked = true;
        if !crate::save::loaded_game() {
            let pc = player.body.location;
            let (pr, phh) = (player.params.radius, player.params.height);
            let mut seen: Vec<(String, bool, bool)> = Vec::new();
            for t in tr.list.iter() {
                if let Kind::Flag { name, value, set: true, while_standing: true, .. } = &t.kind {
                    let inside = overlaps(t, pc, pr, phh);
                    match seen.iter_mut().find(|s| s.0.eq_ignore_ascii_case(name)) {
                        Some(s) => s.2 |= inside,
                        None => seen.push((name.clone(), *value, inside)),
                    }
                }
            }
            for (name, value, inside) in seen {
                if !inside && fl.get_bool(&name) == value {
                    fl.set_bool(&name, !value);
                    println!("TRIGGERS {name} reset to {} (not standing in its trigger)", !value);
                }
            }
        }
    }
    let mut out = Out::default();

    // Events from last frame reach the triggers tagged with them.
    for (name, on) in INBOX
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
    {
        for t in tr
            .list
            .iter_mut()
            .filter(|t| !t.destroyed && t.tag.eq_ignore_ascii_case(&name))
        {
            triggered(t, on, fl, now, &mut out);
        }
    }
    // Infolinks that finished: DatalinkFinished -> Trigger.Touch (fires Event, collision off).
    for a in crate::consys::take_finished_triggers() {
        if let Some(t) = tr.list.iter_mut().find(|t| t.actor == a) {
            if t.active {
                if !t.event.is_empty() {
                    out.emit(&t.event, &t.group, true);
                }
                if t.once {
                    t.collide = false;
                }
            }
        }
    }
    // Touches: the player, and pawns for the triggers that care about them.
    let pc = player.body.location;
    let (pr, phh) = (player.params.radius, player.params.height);
    let alive = !ph.dead;
    // (centre, radius, half height, class, dead): living pawns by their cylinders; bodies (Carcass) lying in the
    // world (not while carried) as a low cylinder where they lie.
    let pawns: Vec<(DVec3, f64, f64, String, bool)> = npcs
        .iter()
        .filter_map(|(n, d, vis)| {
            let cls = level.0.actors[n.actor].class.clone();
            if n.state == crate::ai::NpcState::Dead {
                if matches!(vis, Some(Visibility::Hidden)) {
                    return None;
                }
                return Some((d.body.location, 40.0, 10.0, cls, true));
            }
            let c = blockers.list.get(n.blocker)?;
            Some((c.center, c.radius, c.height, cls, false))
        })
        .chain(
            // Keypoints riding movers (MoverCollider) for class-proximity triggers.
            crate::movers::COLLIDERS
                .lock()
                .map(|c| c.clone())
                .unwrap_or_default()
                .into_iter()
                .map(|p| (p, 1.0, 1.0, "MoverCollider".to_string(), false)),
        )
        .collect();
    if std::env::var("DXVIEW_TRIGLOG").is_ok() {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 30 == 0 { println!("T{now:.1} "); println!("COLL {:?}", pawns.iter().filter(|p| p.3 == "MoverCollider").map(|p| p.0).collect::<Vec<_>>()); }
        for t in tr.list.iter().filter(|t| t.class == "DirectionalTrigger") {
            for (c, ..) in pawns.iter().filter(|p| p.3 == "MoverCollider") {
                if (t.pos - *c).length() < 150.0 {
                    println!("TRIG {} prox={} active={} collide={} d={:.0} dz={:.0} r={} h={} dir={:?}", t.event, t.class_prox, t.active, t.collide, (t.pos - *c).truncate().length(), t.pos.z - c.z, t.radius, t.height, t.dir);
                }
            }
        }
    }
    for t in tr.list.iter_mut() {
        fade_tick(t, dt);
        if t.destroyed || !t.collide {
            if t.touching {
                t.touching = false;
            }
            continue;
        }
        // Only touchers the trigger cares about count (a class-proximity trigger for bodies ignores the player
        // standing in it, so the body still touches it when he's there too).
        let player_in = alive
            && overlaps(t, pc, pr, phh)
            && (t.trigger_type != 2 || t.class_prox.to_ascii_lowercase().contains("player"));
        let pawn_hit = match t.trigger_type {
            1 | 3 => pawns.iter().find(|(c, r, h, _, dead)| !dead && overlaps(t, *c, *r, *h)),
            2 if !t.class_prox.is_empty() => pawns.iter().find(|(c, r, h, cls, dead)| {
                class_prox_matches(&t.class_prox, cls, *dead) && overlaps(t, *c, *r, *h)
            }),
            _ => None,
        };
        let pawn_in = pawn_hit.is_some();
        let inside = player_in || pawn_in;
        if inside && !t.touching {
            t.touching = true;
            let who = if player_in { pc } else { pawn_hit.map_or(pc, |p| p.0) };
            if from_front(t, who) {
                touch(t, player_in, fl, now, &mut out);
            }
        } else if !inside && t.touching {
            t.touching = false;
            untouch(t, player_in, fl, &mut out);
        } else if inside {
            // DataLinkTrigger.Timer: recheck the flag every second while touched.
            let retry = if let Kind::DataLink {
                timer: Some(tm), ..
            } = &mut t.kind
            {
                *tm -= dt;
                *tm <= 0.0
            } else {
                false
            };
            if retry {
                if let Kind::DataLink {
                    tag,
                    check_flag,
                    check_false,
                    timer,
                    ..
                } = &mut t.kind
                {
                    *timer = Some(1.0);
                    if flag_ok(fl, check_flag, *check_false) {
                        *timer = None;
                        let tag = tag.clone();
                        crate::consys::start_datalink(fl, &tag, Some(t.actor));
                    }
                }
            }
        }
        let _ = &t.class;
    }
    for e in out.events {
        events.0.push(e);
    }
    for m in out.messages {
        crate::classic_hud::client_message(&m);
    }
}

/// A new map is going in (loader.rs): this module's state of the map being left.
pub fn reset_level() {
    if let Ok(mut v) = INBOX.lock() {
        v.clear();
    }
    if let Ok(mut v) = ORDERS.lock() {
        v.clear();
    }
    if let Ok(mut v) = FADE.lock() {
        *v = None;
    }
    if let Ok(mut v) = SOURCES.lock() {
        v.clear();
    }
    if let Ok(mut v) = COLLIDE.lock() {
        v.clear();
    }
}
