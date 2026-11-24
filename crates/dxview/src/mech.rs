//! Modern multiplayer, Phase 3 (MULTIPLAYER_ROADMAP.md "Modern mechanics"): the shared parts of uniforms.rs,
//! overwatch.rs, lockdown.rs, framing.rs and killphrase.rs - their tuning (data/deathmatch.ron `mechanics`), the one
//! network message they all travel in, and the queue that hands each message to its module.
//!
//! The host decides everything that more than one player could fight over (whose uniform, whose seat, which sector,
//! whose phrase); each player's own game decides what happens to him (as with damage and death). Messages from the
//! network are queued here (netplay holds its own lock while it reads them) and handed out by mech_system once a frame.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct MechTuning {
    pub uniform_secs: f32,
    pub uniform_secs_infiltrator: f32,
    pub close_look: f32,
    pub close_look_secs: f32,
    pub run_speed: f32,
    pub armed_near: f32,
    pub witness_secs: f32,
    pub body_sight: f32,
    pub weapon_sight: f32,
    pub plant_radius: f32,
    pub dispatch_cooldown: f32,
    pub sabotage_secs: f32,
    pub lockdown_secs: f32,
    pub lockdown_cooldown: f32,
    pub hazard_dps: HashMap<String, f32>,
    pub kp_cubes: usize,
    pub kp_reach: f32,
    pub kp_words: Vec<String>,
}

impl Default for MechTuning {
    fn default() -> Self {
        MechTuning {
            uniform_secs: 150.0,
            uniform_secs_infiltrator: 240.0,
            close_look: 420.0,
            close_look_secs: 1.25,
            run_speed: 230.0,
            armed_near: 320.0,
            witness_secs: 0.4,
            body_sight: 1200.0,
            weapon_sight: 700.0,
            plant_radius: 200.0,
            dispatch_cooldown: 30.0,
            sabotage_secs: 3.5,
            lockdown_secs: 30.0,
            lockdown_cooldown: 90.0,
            hazard_dps: HashMap::new(),
            kp_cubes: 6,
            kp_reach: 220.0,
            kp_words: Vec::new(),
        }
    }
}

pub fn tuning() -> &'static MechTuning {
    &crate::deathmatch::data().mechanics
}

/// A Modern versus match is being played with this switch of the room on.
pub fn on(f: impl Fn(&crate::deathmatch::DmRules) -> bool) -> bool {
    crate::living::modern_match() && crate::deathmatch::rules().is_some_and(|r| f(&r))
}

/// Everything the Phase 3 modules send each other.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum MechMsg {
    // ---- uniforms.rs
    /// Joiner -> host: take the uniform off this body (placed actor index).
    UniformAsk(u32),
    /// Host -> the taker: it's yours (the guard's class), or why not.
    UniformGive(String),
    UniformNo(String),
    /// Host -> the wearer: staff saw through it.
    UniformBlown,
    // ---- overwatch.rs
    /// Joiner -> host: sit at this security computer (actor), or get up.
    SeatAsk(u32),
    SeatLeave,
    /// Host -> all: who sits where (computer actor, player), and seats without power (actor, seconds left).
    Seats(Vec<(u32, u8)>, Vec<(u32, f32)>),
    /// Joiner (seated) -> host: send the guards after this rival.
    Dispatch(u8),
    /// Host -> the seated player: rivals the level's cameras see now (id, where), and his dispatch cooldown.
    Seen(Vec<(u8, [f32; 3])>, f32),
    /// Joiner -> host: his multitool cut this seat's power.
    Sabotage(u32),
    /// Joiner (seated) -> host: lock or unlock this door (mover index) from the console.
    Door(u16, bool),
    // ---- lockdown.rs
    /// Joiner -> host: lock down this sector (index into the map's lockdowns).
    LockdownAsk(u8),
    /// Host -> all: each sector's (seconds running, seconds of cooldown left, who sealed it).
    Lockdowns(Vec<(f32, f32, u8)>),
    /// Joiner -> host: his multitool forced this sealed door (mover index).
    Override(u16),
    // ---- killphrase.rs
    /// Host -> all: every player's phrase, and which fragment each datacube and seat mail holds ((spot, player,
    /// word); spots 0.. are cubes, 100.. seats).
    Phrases(Vec<(u8, [String; 3])>, Vec<(u16, u8, u8)>),
    /// Host -> one player: a fragment learnt (interrogation): whose, which word.
    Fragment(u8, u8),
    /// Joiner -> host: he says the phrase to this rival's face.
    Speak(u8),
    /// Host -> the victim: the phrase was spoken by this player.
    Spoken(u8),
}

/// (from player, message) waiting for mech_system.
static INBOX: Mutex<Vec<(u8, MechMsg)>> = Mutex::new(Vec::new());

/// netplay: a message from the network (netplay holds its lock: nothing else may be called from here).
pub fn incoming(from: u8, m: MechMsg) {
    if let Ok(mut q) = INBOX.lock() {
        q.push((from, m));
    }
}

/// To the host (a joiner), or straight to this machine's own handling (the host, or a game without a network).
pub fn to_host(m: MechMsg) {
    if crate::netplay::is_client() {
        crate::netplay::mech_out(None, m);
    } else {
        incoming(crate::deathmatch::my_id(), m);
    }
}

/// Host: to one player (the host's own player too).
pub fn to_player(id: u8, m: MechMsg) {
    if id == crate::deathmatch::my_id() {
        incoming(id, m);
    } else {
        crate::netplay::mech_out(Some(id), m);
    }
}

/// Host: to every joiner (not to himself).
pub fn to_joiners(m: MechMsg) {
    if crate::netplay::is_host() {
        crate::netplay::mech_out_all(m);
    }
}

/// This machine runs the match's referee (the host, or no network at all).
pub fn host() -> bool {
    !crate::netplay::is_client()
}

/// Hands each queued message to its module (in First, before the game systems).
pub fn mech_system(level: Option<Res<crate::LevelRes>>) {
    let lvl = level.as_deref().map(|l| &l.0);
    let msgs: Vec<(u8, MechMsg)> = INBOX.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    for (from, m) in msgs {
        if env_on!("DXVIEW_MECHLOG") {
            println!("MECH from {from}: {m:?}");
        }
        match m {
            MechMsg::UniformAsk(a) => crate::uniforms::host_ask(from, a, lvl),
            MechMsg::UniformGive(c) => crate::uniforms::given(&c),
            MechMsg::UniformNo(why) => crate::classic_hud::client_message(&why),
            MechMsg::UniformBlown => crate::uniforms::blow(),
            MechMsg::SeatAsk(a) => crate::overwatch::host_seat(from, Some(a)),
            MechMsg::SeatLeave => crate::overwatch::host_seat(from, None),
            MechMsg::Seats(s, p) => crate::overwatch::set_seats(s, p),
            MechMsg::Dispatch(id) => crate::overwatch::host_dispatch(from, id),
            MechMsg::Seen(v, cool) => crate::overwatch::set_seen(v, cool),
            MechMsg::Sabotage(a) => crate::overwatch::host_sabotage(from, a),
            MechMsg::Door(i, l) => crate::overwatch::host_door(from, i, l),
            MechMsg::LockdownAsk(i) => crate::lockdown::host_ask(from, i),
            MechMsg::Lockdowns(v) => crate::lockdown::set_state(v),
            MechMsg::Override(i) => crate::lockdown::host_override(from, i),
            MechMsg::Phrases(p, spots) => crate::killphrase::set_phrases(p, spots),
            MechMsg::Fragment(of, w) => {
                crate::killphrase::learn(of, w, "interrogation");
            }
            MechMsg::Speak(id) => crate::killphrase::host_speak(from, id),
            MechMsg::Spoken(by) => crate::killphrase::spoken(by),
        }
    }
}

/// A new map, a new match: every module's state goes.
pub fn reset_level() {
    if let Ok(mut q) = INBOX.lock() {
        q.clear();
    }
    crate::uniforms::reset_level();
    crate::overwatch::reset_level();
    crate::lockdown::reset_level();
    crate::framing::reset_level();
    crate::killphrase::reset_level();
}

/// A new life (respawn): what belonged to the old one goes.
pub fn new_life() {
    crate::uniforms::reset();
    crate::overwatch::leave_seat();
    crate::framing::new_life();
}
