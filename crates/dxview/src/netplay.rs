//! Co-op multiplayer: the game side of the link (imsim-net's encrypted UDP endpoint on a thread of its own).
//!
//! The host is the story: his map, his save, his conversations, his mission scripts, triggers, alarms and every
//! pawn's mind. Joiners load the same map and see the host's world through what he sends:
//! * pawns (placed actors, by actor index) 15 times a second: where, facing, the sequence playing, in or out of
//!   the world, dead or knocked out, the weapon in hand and a shot counter (their fire sounds play from it);
//! * movers whenever one changes (keyframe, interpolation, lock), flags whenever they change, every event the
//!   host's devices dispatch (lights, particles, sounds and zones follow it on the joiners' side), conversation
//!   lines and infolinks as they play, and map travel (everyone goes with the host).
//!
//! Every player sends his own body 30 times a second; on every other machine each player is a JCDouble pawn
//! (alliance Player) wearing that player's face, moved and animated from it. On the host those bodies are real
//! targets: pawns hostile to the Player alliance fight them as they fight any enemy, and hits on them are sent to
//! their owner as damage. A joiner's own shots at pawns go to the host as hits, from that joiner's body, so the
//! pawn turns on whoever shot it.
//!
//! What stays each player's own (by design): inventory and loot (every player finds his own copy of every item,
//! body search included), the HUD, augmentations (shown only for whoever runs them), decorations and their
//! physics. Joiners can use everything but conversations: doors, switches, keypads and computers go through the
//! host (frobs and dispatched events are sent to him and he does them).
//!
//! Rules from the host room: co-op or deathmatch, Classic or Modern for everyone (the switch is off in
//! multiplayer), tougher enemies with more players, friendly fire (off by default). A player who dies gets up
//! again near the host after RESPAWN_SECS - with his inventory on Easy / Medium, with nothing on Hard / Realistic.

use crate::ai::{Npc, NpcState, OutOfWorld, PlayerHealth};
use crate::LevelRes;
use bevy::prelude::*;
use glam::DVec3;
use imsim_net::game::{self, Endpoint, Event as NetEvent, JoinInfo, PeerId};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The game messages' version (Hello / Welcome).
pub const PROTO: u32 = 1;
/// Seconds before a dead player gets up again near the host.
pub const RESPAWN_SECS: f32 = 4.0;
const ME_HZ: f32 = 30.0;
const PAWN_HZ: f32 = 15.0;
const MOVER_HZ: f32 = 10.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Off,
    Host,
    Client,
}

/// The host room's rules, as every player gets them.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Rules {
    pub coop: bool,
    /// Classic look for everyone (else Modern).
    pub classic: bool,
    /// Enemies tougher with more players.
    pub tough: bool,
    pub friendly_fire: bool,
    pub max_players: u8,
    /// The host's CombatDifficulty (Hard and up: a death loses the inventory).
    pub difficulty: f32,
}

impl Default for Rules {
    fn default() -> Self {
        Rules { coop: true, classic: false, tough: false, friendly_fire: false, max_players: 4, difficulty: 1.5 }
    }
}

/// How a player looks to the others.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Look {
    pub name: String,
    pub female: bool,
    pub skin: u8,
}

/// One player's body, 30 times a second.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct PState {
    /// Collision centre (Unreal units).
    pub pos: [f32; 3],
    pub vel: [f32; 3],
    /// View yaw / pitch (radians, Unreal axes: yaw from +X toward +Y).
    pub yaw: f32,
    pub pitch: f32,
    pub crouch: bool,
    pub swim: bool,
    pub dead: bool,
    /// Overall health, percent.
    pub health: u8,
    /// Weapon class in hand ("" = none).
    pub weapon: String,
    /// Shots fired so far (wrapping): a new value plays the weapon's fire sound on the others' side.
    pub shots: u8,
    /// The map this state is on (hash of its name): states from another map are ignored.
    pub map: u32,
}

/// A pawn as the host sends it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PawnNet {
    pub actor: u32,
    pub pos: [f32; 3],
    pub yaw: f32,
    pub anim: String,
    pub t: f32,
    pub rate: f32,
    pub looping: bool,
    /// 0 alive, 1 dead, 2 knocked out.
    pub dead: u8,
    pub out_of_world: bool,
    /// Destroyed on the host (gibbed, Destroy()): gone for good.
    pub gone: bool,
    pub weapon: String,
    pub shots: u8,
}

/// A mover as the host sends it (when it changes).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MoverNet {
    pub idx: u16,
    pub loc: [f32; 3],
    pub rot: [i32; 3],
    pub key: u8,
    pub prev: u8,
    pub interp: bool,
    pub alpha: f32,
    pub rate: f32,
    pub old_pos: [f32; 3],
    pub old_rot: [i32; 3],
    pub opening: bool,
    pub locked: bool,
    pub destroyed: bool,
    /// TriggerOpenTimed's stay-open time left (negative = none).
    pub close: f32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum HitOn {
    /// A placed pawn (actor index).
    Pawn(u32),
    /// Another player's body.
    Player(u8),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
enum Msg {
    Hello { proto: u32, look: Look, version: String },
    Welcome { you: u8, map: String, rules: Rules },
    Refuse(String),
    /// Every player (the host is 0).
    Roster(Vec<(u8, Look)>),
    Me(PState),
    Players(Vec<(u8, PState)>),
    Pawns(Vec<PawnNet>),
    Movers(Vec<MoverNet>),
    Flags(imsim_core::flags::FlagBase),
    Event(String, bool),
    FrobMover(u16),
    FrobFixture(u32),
    Lock(u16, bool),
    Hit { on: HitOn, damage: f32, kind: String, at: [f32; 3], from: [f32; 3] },
    /// `by`: the player who did it (friendly fire / deathmatch), None = a pawn.
    Damage { damage: f32, kind: String, from: [f32; 3], by: Option<u8> },
    /// A player went down (`by`: the player who did it).
    Died { by: Option<u8> },
    Travel(String),
    Speech { at: Option<[f32; 3]>, sound: String, name: String, text: String, radius: f32 },
    Noise { at: [f32; 3], radius: f32 },
    Rules(Rules),
    /// The host's goals (persona Goals screen), whenever they change.
    Goals(Vec<crate::consys::Goal>),
    /// A line for everyone's log ("X joined").
    Note(String),
}

fn encode(m: &Msg) -> Vec<u8> {
    use bincode::Options;
    bincode::DefaultOptions::new().serialize(m).unwrap_or_default()
}

fn decode(b: &[u8]) -> Option<Msg> {
    use bincode::Options;
    bincode::DefaultOptions::new().with_limit(game::MAX_MESSAGE as u64).deserialize(b).ok()
}

enum Out {
    To(PeerId, Vec<u8>, bool),
    All(Vec<u8>, bool, Option<PeerId>),
    Register(SocketAddr, String),
    Kick(PeerId, String),
    Stop,
}

/// Another player, as this machine sees him.
#[derive(Clone, Debug)]
pub struct Remote {
    pub id: u8,
    pub look: Look,
    pub st: PState,
    pub at: Option<Instant>,
    /// His body here (a JCDouble pawn) once spawned.
    pub body: Option<Entity>,
    pub blocker: Option<usize>,
    asked: bool,
    /// Host: said hello, Welcome not sent yet (the host isn't in a game).
    waiting: bool,
    pub ping: f32,
}

#[derive(Default)]
struct Inbox {
    events: Vec<NetEvent>,
    rtt: Vec<(u8, f32)>,
    dead: Option<String>,
}

struct Net {
    role: Role,
    my_id: u8,
    rules: Rules,
    look: Look,
    remotes: Vec<Remote>,
    tx: Sender<Out>,
    inbox: Arc<Mutex<Inbox>>,
    status: String,
    /// Joiner: the host's map (from Welcome / Travel), and whether we're there.
    host_map: Option<String>,
    welcomed: bool,
    placed_near_host: bool,
    map_gen: u64,
    // Work for the game systems.
    frob_movers: Vec<(u8, u16)>,
    frob_fixtures: Vec<u32>,
    locks: Vec<(u16, bool)>,
    events_in: Vec<(String, bool)>,
    hits_in: Vec<(u8, HitOn, f32, String, [f32; 3], [f32; 3])>,
    damage_in: Vec<(f32, String, [f32; 3], Option<u8>)>,
    /// The player who last hurt us (deathmatch credit).
    last_by: Option<u8>,
    /// Deathmatch: (kills, deaths) by player id (host keeps them; everyone hears the lines).
    scores: HashMap<u8, (u32, u32)>,
    speech_in: Vec<(Option<[f32; 3]>, String, String, String, f32)>,
    noises_in: Vec<(u8, [f32; 3], f32)>,
    flags_in: Option<imsim_core::flags::FlagBase>,
    movers_in: Vec<MoverNet>,
    pawns_in: HashMap<u32, PawnNet>,
    travel_in: Option<String>,
    // Host bookkeeping.
    sent_movers: Vec<Option<MoverNet>>,
    flags_sig: u64,
    goals_sig: u64,
    goals_in: Option<Vec<crate::consys::Goal>>,
    pawns_alive: HashSet<u32>,
    // Clocks.
    me_t: f32,
    pawn_t: f32,
    mover_t: f32,
    flag_t: f32,
    respawn: Option<f32>,
    shots: u8,
    lost: Option<String>,
}

static NET: Mutex<Option<Net>> = Mutex::new(None);
/// The last reason a session ended (for the menus).
static LAST_END: Mutex<String> = Mutex::new(String::new());

fn with<R>(f: impl FnOnce(&mut Net) -> R) -> Option<R> {
    let mut g = NET.lock().unwrap_or_else(|e| e.into_inner());
    g.as_mut().map(f)
}

pub fn role() -> Role {
    with(|n| n.role).unwrap_or(Role::Off)
}
pub fn active() -> bool {
    role() != Role::Off
}
pub fn is_host() -> bool {
    role() == Role::Host
}
pub fn is_client() -> bool {
    role() == Role::Client
}
/// Joiner who is in the host's game (Welcome came).
pub fn in_game() -> bool {
    with(|n| n.role == Role::Host || n.welcomed).unwrap_or(false)
}

pub fn rules() -> Option<Rules> {
    with(|n| n.rules.clone())
}

pub fn status_text() -> String {
    with(|n| n.status.clone()).unwrap_or_else(|| LAST_END.lock().map(|s| s.clone()).unwrap_or_default())
}

/// Why the last session ended ("" = still on, or never started).
pub fn last_end() -> String {
    LAST_END.lock().map(|s| s.clone()).unwrap_or_default()
}

/// (id, name, ping ms) of every other player.
pub fn players() -> Vec<(u8, String, f32)> {
    with(|n| n.remotes.iter().map(|r| (r.id, r.look.name.clone(), r.ping)).collect()).unwrap_or_default()
}

/// How many players are in the session (this one included).
pub fn player_count() -> usize {
    with(|n| n.remotes.len() + 1).unwrap_or(1)
}

fn map_hash(name: &str) -> u32 {
    crate::util::fnv1a(&name.to_ascii_lowercase()) as u32
}

fn lobby_udp() -> Option<SocketAddr> {
    crate::online::server_addr().to_socket_addrs().ok()?.find(|a| a.is_ipv4())
}

fn my_look() -> Look {
    let s = crate::travel::session();
    Look {
        name: s.as_ref().map(|s| s.name.clone()).unwrap_or_else(|| "JC Denton".into()),
        female: s.as_ref().is_some_and(|s| s.female),
        skin: s.as_ref().map_or(0, |s| s.skin.min(4)),
    }
}

fn start(role: Role, ep: Endpoint, rules: Rules, look: Look) {
    let (tx, rx) = channel();
    let inbox = Arc::new(Mutex::new(Inbox::default()));
    let ib = inbox.clone();
    let _ = std::thread::Builder::new().name("netplay".into()).spawn(move || net_thread(ep, rx, ib));
    let net = Net {
        role,
        my_id: if role == Role::Host { 0 } else { 255 },
        rules,
        look,
        remotes: Vec::new(),
        tx,
        inbox,
        status: if role == Role::Host { "Hosting".into() } else { "Connecting to the host...".into() },
        host_map: None,
        welcomed: false,
        placed_near_host: false,
        map_gen: 0,
        frob_movers: Vec::new(),
        frob_fixtures: Vec::new(),
        locks: Vec::new(),
        events_in: Vec::new(),
        hits_in: Vec::new(),
        damage_in: Vec::new(),
        last_by: None,
        scores: HashMap::new(),
        speech_in: Vec::new(),
        noises_in: Vec::new(),
        flags_in: None,
        movers_in: Vec::new(),
        pawns_in: HashMap::new(),
        travel_in: None,
        sent_movers: Vec::new(),
        flags_sig: 0,
        goals_sig: 0,
        goals_in: None,
        pawns_alive: HashSet::new(),
        me_t: 0.0,
        pawn_t: 0.0,
        mover_t: 0.0,
        flag_t: 0.0,
        respawn: None,
        shots: 0,
        lost: None,
    };
    if let Ok(mut l) = LAST_END.lock() {
        l.clear();
    }
    *NET.lock().unwrap_or_else(|e| e.into_inner()) = Some(net);
}

fn net_thread(mut ep: Endpoint, rx: Receiver<Out>, inbox: Arc<Mutex<Inbox>>) {
    let mut last_rtt = Instant::now();
    loop {
        loop {
            match rx.try_recv() {
                Ok(Out::To(p, b, r)) => ep.send(p, &b, r),
                Ok(Out::All(b, r, except)) => ep.broadcast(&b, r, except),
                Ok(Out::Register(l, t)) => ep.register(l, t),
                Ok(Out::Kick(p, why)) => ep.kick(Some(p), &why),
                Ok(Out::Stop) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    ep.kick(None, "left");
                    let _ = ep.poll();
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
            }
        }
        let ev = ep.poll();
        if !ev.is_empty() || last_rtt.elapsed() > Duration::from_millis(500) {
            let rtt: Vec<(u8, f32)> = if last_rtt.elapsed() > Duration::from_millis(500) {
                last_rtt = Instant::now();
                ep.peers().into_iter().filter_map(|p| ep.rtt(p).map(|r| (p, r))).collect()
            } else {
                Vec::new()
            };
            if let Ok(mut ib) = inbox.lock() {
                ib.events.extend(ev);
                if !rtt.is_empty() {
                    ib.rtt = rtt;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn send_to(n: &Net, peer: u8, m: &Msg, reliable: bool) {
    let _ = n.tx.send(Out::To(peer, encode(m), reliable));
}

fn send_all(n: &Net, m: &Msg, reliable: bool, except: Option<u8>) {
    let _ = n.tx.send(Out::All(encode(m), reliable, except));
}

/// Joiner: to the host.
fn send_host(n: &Net, m: &Msg, reliable: bool) {
    send_to(n, 0, m, reliable);
}

// ---------------------------------------------------------------------------------------------------------------
// Starting and ending a session.

/// Host Room: open the game socket and the room's key. Returns (game port, public key hex, LAN address) for the
/// lobby's HOST line. Joiners are let in once the host is in a game.
pub fn open_host(rules: Rules) -> Result<(u16, String, Option<SocketAddr>), String> {
    leave("");
    let (private, public) = game::keypair();
    let ep = Endpoint::host(imsim_net::DEFAULT_PORT + 10, private, lobby_udp(), None).map_err(|e| format!("Couldn't open the game port ({e})."))?;
    let port = ep.local_port();
    let lan = game::lan_ip().map(|ip| SocketAddr::new(ip, port));
    force_look(&rules);
    let mut ep = ep;
    ep.max_peers = rules.max_players.saturating_sub(1).clamp(1, 15);
    start(Role::Host, ep, rules, my_look());
    Ok((port, game::hex(&public), lan))
}

/// The lobby gave our room its token: register our game socket with its UDP port (punching, the relay).
pub fn host_registered(token: &str) {
    if let Some(l) = lobby_udp() {
        with(|n| {
            let _ = n.tx.send(Out::Register(l, token.to_string()));
        });
    }
}

/// The room's look for everyone: Classic or Modern is set here (the in-game switch stays off while playing).
fn force_look(r: &Rules) {
    let modern = !r.classic;
    if crate::remaster::is_modern() != modern {
        let _ = crate::remaster::switch(modern);
    }
}

/// Friendly fire (the host's toggle; off unless he turns it on).
pub fn set_friendly_fire(on: bool) {
    with(|n| {
        n.rules.friendly_fire = on;
        if n.role == Role::Host {
            send_all(n, &Msg::Rules(n.rules.clone()), true, None);
        }
    });
}

/// The host changed the room's rules.
pub fn set_rules(r: Rules) {
    force_look(&r);
    with(|n| {
        n.rules = r.clone();
        if n.role == Role::Host {
            send_all(n, &Msg::Rules(r), true, None);
        }
    });
}

/// Join Room: connect to the host the lobby found. `look`: the character we play.
pub fn join(j: &imsim_net::client::Joined, look: Look) -> Result<(), String> {
    leave("");
    let key = game::unhex32(&j.key).ok_or("The room's key is missing (an older host?).")?;
    let mut targets = Vec::new();
    for a in [j.lan, j.udp, Some(j.addr)].into_iter().flatten() {
        if !targets.contains(&a) {
            targets.push(a);
        }
    }
    let ep = Endpoint::join(JoinInfo { host_key: key, targets, lobby: lobby_udp(), ticket: j.ticket, force_relay: env_on!("DXVIEW_FORCE_RELAY") })
        .map_err(|e| format!("Couldn't open a game socket ({e})."))?;
    let rules = Rules { coop: j.mode == imsim_net::Mode::Coop, classic: j.look == imsim_net::Look::Classic, tough: j.tough, max_players: j.max_players, ..Rules::default() };
    start(Role::Client, ep, rules, look);
    Ok(())
}

/// Leave the session (a host's room closes for everyone).
pub fn leave(why: &str) {
    let mut g = NET.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(n) = g.take() {
        let _ = n.tx.send(Out::Stop);
        // Long enough for the link thread to say goodbye (the game may be quitting).
        std::thread::sleep(Duration::from_millis(30));
        if !why.is_empty() {
            if let Ok(mut l) = LAST_END.lock() {
                *l = why.to_string();
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Hooks the game calls.

/// Devices dispatched an event: a joiner sends it to the host (who does it and echoes it to everyone); the host
/// tells the joiners. Returns true when the caller should not run it here.
pub fn dispatched(name: &str, on: bool) -> bool {
    with(|n| match n.role {
        Role::Client => {
            if n.welcomed {
                send_host(n, &Msg::Event(name.to_string(), on), true);
            }
            true
        }
        Role::Host => {
            send_all(n, &Msg::Event(name.to_string(), on), true, None);
            false
        }
        Role::Off => false,
    })
    .unwrap_or(false)
}

/// Events from the host for this joiner's own effects (lights, particles, sounds, zones).
pub fn take_events() -> Vec<(String, bool)> {
    with(|n| std::mem::take(&mut n.events_in)).unwrap_or_default()
}

/// A joiner frobbed a door: the host does it too (his door is the one that counts).
pub fn frob_mover(idx: usize) {
    with(|n| {
        if n.role == Role::Client && n.welcomed {
            send_host(n, &Msg::FrobMover(idx as u16), true);
        }
    });
}

/// A joiner used a fixture (switch, button, lamp): the host uses it too.
pub fn frob_fixture(actor: usize) {
    with(|n| {
        if n.role == Role::Client && n.welcomed {
            send_host(n, &Msg::FrobFixture(actor as u32), true);
        }
    });
}

/// A joiner's key ring locked or unlocked a door.
pub fn set_lock(idx: usize, locked: bool) {
    with(|n| {
        if n.role == Role::Client && n.welcomed {
            send_host(n, &Msg::Lock(idx as u16, locked), true);
        }
    });
}

/// Conversations belong to the host. True when this player may not start one (a joiner).
pub fn conversations_blocked() -> bool {
    is_client()
}

/// Joiners don't run mission scripts, triggers, alarms or anything else that drives the story.
pub fn story_off() -> bool {
    is_client()
}

/// The host is travelling: everyone comes along.
pub fn host_travel(map: &str) {
    with(|n| {
        if n.role == Role::Host {
            send_all(n, &Msg::Travel(map.to_string()), true, None);
        }
    });
}

/// Joiners only travel when the host does.
pub fn may_travel() -> bool {
    !is_client() || ALLOW_TRAVEL.load(std::sync::atomic::Ordering::Relaxed)
}
static ALLOW_TRAVEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A conversation line or infolink is playing on the host: everyone hears it (from the speaker when `at`).
pub fn speech(at: Option<DVec3>, sound: &str, name: &str, text: &str, radius: f32) {
    with(|n| {
        if n.role == Role::Host && !n.remotes.is_empty() {
            let at = at.map(|a| [a.x as f32, a.y as f32, a.z as f32]);
            send_all(n, &Msg::Speech { at, sound: sound.into(), name: name.into(), text: text.into(), radius }, true, None);
        }
    });
}

/// A loud noise this player made (gunfire): the host's pawns hear it.
pub fn noise(at: DVec3, radius: f32) {
    with(|n| {
        if n.role == Role::Client && n.welcomed {
            send_host(n, &Msg::Noise { at: [at.x as f32, at.y as f32, at.z as f32], radius }, false);
        }
    });
}

/// The local player fired: the others hear it.
pub fn shot() {
    with(|n| n.shots = n.shots.wrapping_add(1));
}

/// Hits on a puppet (a pawn the host runs, seen by a joiner, or a player's body): sent where they count.
/// `peer`: the body's player, if it is one.
pub fn puppet_hits(actor: usize, peer: Option<u8>, hits: &[crate::damage::PawnHit]) {
    with(|n| {
        for h in hits {
            let at = [h.hit_loc.x as f32, h.hit_loc.y as f32, h.hit_loc.z as f32];
            let from = [h.attacker_pos.x as f32, h.attacker_pos.y as f32, h.attacker_pos.z as f32];
            match (n.role, peer) {
                // A pawn's hit on a player's body (or our own with friendly fire): that player takes it.
                (Role::Host, Some(p)) => {
                    let players_hurt = n.rules.friendly_fire || !n.rules.coop;
                    if h.attacker.is_some() || players_hurt {
                        let by = if h.attacker.is_none() { Some(0) } else { None };
                        send_to(n, p, &Msg::Damage { damage: h.damage, kind: h.damage_type.to_string(), from, by }, true);
                    }
                }
                (Role::Client, Some(p)) => {
                    if h.attacker.is_none() && (n.rules.friendly_fire || !n.rules.coop) {
                        send_host(n, &Msg::Hit { on: HitOn::Player(p), damage: h.damage, kind: h.damage_type.to_string(), at, from }, true);
                    }
                }
                (Role::Client, None) => {
                    if h.attacker.is_none() {
                        send_host(n, &Msg::Hit { on: HitOn::Pawn(actor as u32), damage: h.damage, kind: h.damage_type.to_string(), at, from }, true);
                    }
                }
                _ => {}
            }
        }
    });
}

/// Players' bodies (centre, radius, half height) on this machine: triggers count them as the player, pawns as
/// enemies of the Player alliance.
pub fn remote_bodies() -> Vec<(u8, DVec3)> {
    with(|n| {
        n.remotes
            .iter()
            .filter(|r| r.at.is_some() && !r.st.dead)
            .map(|r| (r.id, DVec3::new(r.st.pos[0] as f64, r.st.pos[1] as f64, r.st.pos[2] as f64)))
            .collect()
    })
    .unwrap_or_default()
}

/// A player's body is within `radius` (pawns there keep thinking even unseen by the host).
pub fn remote_near(p: DVec3, radius: f64) -> bool {
    with(|n| {
        n.remotes.iter().any(|r| {
            r.at.is_some() && (DVec3::new(r.st.pos[0] as f64, r.st.pos[1] as f64, r.st.pos[2] as f64) - p).length_squared() < radius * radius
        })
    })
    .unwrap_or(false)
}

/// Tougher enemies (room rule): pawn health scales with the players in the game.
pub fn health_scale() -> f32 {
    with(|n| if n.rules.tough { 1.0 + 0.5 * n.remotes.len() as f32 } else { 1.0 }).unwrap_or(1.0)
}

/// The Classic / Modern switch is the room's.
pub fn look_locked() -> bool {
    active()
}

/// Seconds until this dead player gets up again (None = not waiting).
pub fn respawn_in() -> Option<f32> {
    with(|n| n.respawn).flatten()
}

// ---------------------------------------------------------------------------------------------------------------
// Puppets: what a pawn driven from the network does this frame (read by ai::npc_system).

/// The pose a puppet pawn takes this frame.
#[derive(Clone, Debug, Default)]
pub struct NetPose {
    pub pos: DVec3,
    pub yaw: f64,
    /// (sequence, time, rate, looping) for a pawn the host runs; None = a player's body (animated from motion).
    pub anim: Option<(String, f32, f32, bool)>,
    pub speed: f64,
    pub crouch: bool,
    pub swim: bool,
    pub dead: u8,
    pub weapon: String,
    pub shots: u8,
    pub shots_seen: u8,
    pub seen: bool,
}

// ---------------------------------------------------------------------------------------------------------------
// Systems.

/// Network in: messages applied to this machine's world (before the pawns think).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn net_in_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    mut movers: ResMut<crate::movers::Movers>,
    mut flags: ResMut<crate::ai::GameFlags>,
    mut ph: ResMut<PlayerHealth>,
    mut player: ResMut<crate::Player>,
    mut hits: ResMut<crate::damage::PawnHits>,
    mut events: ResMut<crate::devices::GameEvents>,
    mut noises: ResMut<crate::stealth::Noises>,
    mut blockers: ResMut<crate::actors::Blockers>,
    mut tfs: Query<&mut Transform, Without<Npc>>,
    mut npcs: Query<(Entity, &mut Npc, &crate::actors::Deco, &mut Visibility, Option<&OutOfWorld>)>,
    mut inv: ResMut<crate::pickups::PlayerInventory>,
    mut cams: Query<&mut crate::FlyCam>,
) {
    let Some(role) = with(|n| n.role) else { return };
    let _prof = crate::prof::scope("netplay::net_in_system");
    let dt = time.delta_secs();
    let lvl = &level.0;
    let placed = crate::save::placed_count().min(lvl.actors.len());
    let my_map = map_hash(&lvl.name);
    let new_map = with(|n| {
        let g = crate::loader::generation();
        let changed = n.map_gen != g;
        n.map_gen = g;
        if changed {
            n.placed_near_host = false;
            n.sent_movers.clear();
            n.pawns_alive.clear();
            n.pawns_in.clear();
            n.flags_sig = 0;
            for r in &mut n.remotes {
                r.body = None;
                r.blocker = None;
                r.asked = false;
            }
        }
        changed
    })
    .unwrap_or(false);
    let _ = new_map;
    // Messages.
    let (evs, rtt, dead) = with(|n| {
        let mut ib = n.inbox.lock().unwrap_or_else(|e| e.into_inner());
        (std::mem::take(&mut ib.events), std::mem::take(&mut ib.rtt), ib.dead.take())
    })
    .unwrap_or_default();
    if let Some(why) = dead {
        leave(&why);
        return;
    }
    for e in evs {
        handle(role, e, lvl);
    }
    with(|n| {
        for (p, r) in &rtt {
            if let Some(rm) = n.remotes.iter_mut().find(|x| x.id == *p) {
                rm.ping = *r;
            }
            if role == Role::Client && *p == 0 {
                n.status = format!("Connected ({:.0} ms)", r);
            }
        }
    });
    if let Some(why) = with(|n| n.lost.take()).flatten() {
        crate::classic_hud::client_message(&why);
        leave(&why);
        return;
    }
    // The playing map isn't ready (loading, title): wait.
    let playable = !crate::loader::busy() && !crate::menus::intro_map();
    // Joiner: travel with the host.
    if role == Role::Client {
        let travel = with(|n| n.travel_in.take()).flatten();
        if let Some(map) = travel {
            save_character_soon();
            if !map.eq_ignore_ascii_case(&lvl.name) || !playable {
                ALLOW_TRAVEL.store(true, std::sync::atomic::Ordering::Relaxed);
                let first = with(|n| !n.placed_near_host && n.map_gen <= 1).unwrap_or(false);
                let goal = if first || !playable {
                    let ng = crate::travel::session().unwrap_or_default();
                    crate::loader::Goal::NewGame(ng)
                } else {
                    crate::loader::Goal::Travel(None)
                };
                crate::loader::request(&map, goal);
                ALLOW_TRAVEL.store(false, std::sync::atomic::Ordering::Relaxed);
            }
            return;
        }
    }
    if !playable {
        return;
    }
    // Host: anyone still waiting for a game gets one now.
    if role == Role::Host {
        with(|n| {
            let map = lvl.name.clone();
            let rules = n.rules.clone();
            let mut welcomed = Vec::new();
            for r in n.remotes.iter_mut().filter(|r| r.waiting) {
                r.waiting = false;
                welcomed.push(r.id);
            }
            for id in welcomed {
                send_to(n, id, &Msg::Welcome { you: id, map: map.clone(), rules: rules.clone() }, true);
                send_to(n, id, &Msg::Flags(flags.0.clone()), true);
                n.sent_movers.clear();
                n.goals_sig = 0;
            }
        });
    }
    // Flags: the host's are everyone's.
    if role == Role::Client {
        if let Some(f) = with(|n| n.flags_in.take()).flatten() {
            flags.0 = f;
        }
        if let Some(g) = with(|n| n.goals_in.take()).flatten() {
            if let Some(st) = crate::consys::lock().as_mut() {
                st.goals = g;
            }
        }
    }
    // Movers.
    let mv_in = with(|n| std::mem::take(&mut n.movers_in)).unwrap_or_default();
    for m in mv_in {
        let i = m.idx as usize;
        let Some(rt) = movers.rt.get_mut(i) else { continue };
        rt.location = DVec3::new(m.loc[0] as f64, m.loc[1] as f64, m.loc[2] as f64);
        rt.rotation = m.rot;
        rt.key_num = m.key;
        rt.prev_key_num = m.prev;
        rt.interpolating = m.interp;
        rt.alpha = m.alpha as f64;
        rt.rate = m.rate as f64;
        rt.old_pos = DVec3::new(m.old_pos[0] as f64, m.old_pos[1] as f64, m.old_pos[2] as f64);
        rt.old_rot = m.old_rot;
        rt.opening = m.opening;
        rt.locked = m.locked;
        rt.destroyed = m.destroyed;
        rt.close_timer = (m.close >= 0.0).then_some(m.close as f64);
        rt.moved_since_lit = true;
        if let Some(e) = movers.entities.get(i) {
            if let Ok(mut tf) = tfs.get_mut(*e) {
                tf.translation = Vec3::from(dx_level::ue_to_bevy(m.loc));
                tf.rotation = crate::movers::bevy_rotation(m.rot);
            }
        }
    }
    // Host: a joiner's door, switch or lock.
    if role == Role::Host {
        let (fm, ff, lk) = with(|n| (std::mem::take(&mut n.frob_movers), std::mem::take(&mut n.frob_fixtures), std::mem::take(&mut n.locks))).unwrap_or_default();
        for (who, i) in fm {
            let i = i as usize;
            if i < movers.rt.len() {
                let at = with(|n| n.remotes.iter().find(|r| r.id == who).map(|r| DVec3::new(r.st.pos[0] as f64, r.st.pos[1] as f64, r.st.pos[2] as f64))).flatten().unwrap_or(player.body.location);
                let _ = crate::movers::frob(lvl, &mut movers, i, at, false);
            }
        }
        for a in ff {
            if let Some(b) = blockers.list.iter().position(|c| c.actor == a as usize) {
                crate::fixtures::frob(b);
            }
        }
        for (i, l) in lk {
            let i = i as usize;
            if i < movers.rt.len() {
                let tag = lvl.movers[i].tag.clone();
                for (j, info) in lvl.movers.iter().enumerate() {
                    if j == i || (!tag.is_empty() && !tag.eq_ignore_ascii_case("DeusExMover") && info.tag.eq_ignore_ascii_case(&tag)) {
                        movers.rt[j].locked = l;
                    }
                }
            }
        }
        // Joiners' dispatched events go through the host's devices (and so back to everyone).
        let ev = with(|n| std::mem::take(&mut n.events_in)).unwrap_or_default();
        events.0.extend(ev);
        let ns = with(|n| {
            std::mem::take(&mut n.noises_in)
                .into_iter()
                .map(|(p, at, r)| (n.remotes.iter().find(|x| x.id == p).and_then(|x| x.blocker), at, r))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
        for (body, at, r) in ns {
            let who = body.map(crate::ai::Target::Pawn);
            noises.0.push(crate::stealth::Noise {
                pos: DVec3::new(at[0] as f64, at[1] as f64, at[2] as f64),
                volume: 1.0,
                radius: r,
                maker: who,
                instigator: who,
                kind: crate::stealth::NoiseKind::WeaponFire,
            });
        }
    }
    // Hits a joiner's shots made.
    if role == Role::Host {
        let hs = with(|n| std::mem::take(&mut n.hits_in)).unwrap_or_default();
        for (from_peer, on, dmg, kind, at, from) in hs {
            let body = with(|n| n.remotes.iter().find(|r| r.id == from_peer).and_then(|r| r.blocker)).flatten();
            match on {
                HitOn::Pawn(a) => {
                    let Some(b) = npcs.iter().find(|(_, n, ..)| n.actor == a as usize && n.net.is_none()).map(|(_, n, ..)| n.blocker) else { continue };
                    hits.0.push(crate::damage::PawnHit {
                        blocker: b,
                        damage: dmg,
                        damage_type: crate::util::intern(&kind),
                        hit_loc: DVec3::new(at[0] as f64, at[1] as f64, at[2] as f64),
                        attacker: body,
                        attacker_pos: DVec3::new(from[0] as f64, from[1] as f64, from[2] as f64),
                    });
                }
                HitOn::Player(p) => {
                    let ff = with(|n| n.rules.friendly_fire || !n.rules.coop).unwrap_or(false);
                    if !ff {
                        continue;
                    }
                    if p == 0 {
                        with(|n| n.last_by = Some(from_peer));
                        ph.damage(dmg, &kind, DVec3::ZERO);
                    } else {
                        with(|n| send_to(n, p, &Msg::Damage { damage: dmg, kind, from, by: Some(from_peer) }, true));
                    }
                }
            }
        }
    }
    // Damage the host sent us (pawns' shots at our body there).
    let dmg = with(|n| std::mem::take(&mut n.damage_in)).unwrap_or_default();
    for (d, kind, from, by) in dmg {
        with(|n| n.last_by = by);
        let from = DVec3::new(from[0] as f64, from[1] as f64, from[2] as f64);
        let off = from - player.body.location;
        ph.damage(d, &kind, off.normalize_or_zero() * player.params.radius);
    }
    // Lines spoken on the host.
    let sp = with(|n| std::mem::take(&mut n.speech_in)).unwrap_or_default();
    for (at, sound, name, text, radius) in sp {
        if !sound.is_empty() {
            let at = at.map(|a| DVec3::new(a[0] as f64, a[1] as f64, a[2] as f64));
            match at {
                Some(a) => crate::sound::play(crate::util::intern(&sound), a, 1.0, radius, 1.0),
                None => crate::sound::play_player(crate::util::intern(&sound), 1.0, 0.0),
            }
        }
        if !text.is_empty() {
            let line = if name.is_empty() { text } else { format!("{name}: {text}") };
            crate::classic_hud::client_message(&line);
        }
    }

    // Players' bodies: spawned as JCDouble pawns, moved and posed from their states.
    let bodies: Vec<(Entity, u8, usize)> = npcs.iter().filter_map(|(e, n, ..)| n.net_player.map(|p| (e, p, n.blocker))).collect();
    let mut gone_bodies = Vec::new();
    let spawn_reqs = with(|n| {
        let mut reqs = Vec::new();
        for r in &mut n.remotes {
            if let Some((e, _, b)) = bodies.iter().find(|(_, p, _)| *p == r.id) {
                r.body = Some(*e);
                r.blocker = Some(*b);
            }
            if r.body.is_none() && !r.asked && r.at.is_some() && r.st.map == my_map {
                r.asked = true;
                reqs.push((r.id, r.look.clone(), r.st.clone()));
            }
        }
        for (e, p, b) in &bodies {
            if !n.remotes.iter().any(|r| r.id == *p) {
                gone_bodies.push((*e, *b));
            }
        }
        reqs
    })
    .unwrap_or_default();
    for (e, b) in gone_bodies {
        if let Some(c) = blockers.list.get_mut(b) {
            c.radius = 0.0;
            c.center = DVec3::splat(-1.0e6);
        }
        commands.entity(e).despawn();
    }
    for (id, look, st) in spawn_reqs {
        spawn_body(id, &look, &st);
    }
    let poses: Vec<(u8, NetPose)> = with(|n| {
        n.remotes
            .iter()
            .filter(|r| r.at.is_some())
            .map(|r| {
                let age = r.at.map_or(0.0, |t| t.elapsed().as_secs_f32()).min(0.25) as f64;
                let p = DVec3::new(r.st.pos[0] as f64, r.st.pos[1] as f64, r.st.pos[2] as f64);
                let v = DVec3::new(r.st.vel[0] as f64, r.st.vel[1] as f64, r.st.vel[2] as f64);
                (
                    r.id,
                    NetPose {
                        pos: p + v * age,
                        yaw: r.st.yaw as f64,
                        anim: None,
                        speed: v.truncate().length(),
                        crouch: r.st.crouch,
                        swim: r.st.swim,
                        dead: r.st.dead as u8,
                        weapon: r.st.weapon.clone(),
                        shots: r.st.shots,
                        shots_seen: 0,
                        seen: true,
                    },
                )
            })
            .collect()
    })
    .unwrap_or_default();
    for (_, mut npc, ..) in &mut npcs {
        if let Some(p) = npc.net_player {
            match poses.iter().find(|(id, _)| *id == p) {
                Some((_, pose)) => {
                    let seen = npc.net.as_ref().map_or(pose.shots, |o| o.shots_seen);
                    npc.net = Some(Box::new(NetPose { shots_seen: seen, ..pose.clone() }));
                }
                None => {
                    if let Some(np) = npc.net.as_mut() {
                        np.seen = false;
                    }
                }
            }
        }
    }

    // Joiner: every placed pawn is the host's.
    if role == Role::Client {
        let pin = with(|n| std::mem::take(&mut n.pawns_in)).unwrap_or_default();
        for (e, mut npc, deco, mut vis, out) in &mut npcs {
            // (Bodies the map placed have no blocker: they lie where they are, searchable, on every machine.)
            if npc.net_player.is_some() || npc.actor >= placed || npc.blocker >= blockers.list.len() {
                continue;
            }
            if npc.net.is_none() {
                npc.net = Some(Box::new(NetPose { pos: deco.body.location, yaw: npc.yaw, ..Default::default() }));
            }
            let Some(p) = pin.get(&(npc.actor as u32)) else { continue };
            if p.gone {
                if let Some(c) = blockers.list.get_mut(npc.blocker) {
                    c.radius = 0.0;
                }
                npc.state = NpcState::Dead;
                commands.entity(e).despawn();
                continue;
            }
            // In or out of the world as on the host (LeaveWorld / EnterWorld).
            match (p.out_of_world, out) {
                (true, None) => {
                    if let Some(c) = blockers.list.get_mut(npc.blocker) {
                        commands.entity(e).insert(OutOfWorld { radius: c.radius, height: c.height });
                        c.radius = 0.0;
                    }
                    vis.set_if_neq(Visibility::Hidden);
                }
                (false, Some(o)) => {
                    if let Some(c) = blockers.list.get_mut(npc.blocker) {
                        c.radius = o.radius;
                        c.height = o.height;
                    }
                    commands.entity(e).remove::<OutOfWorld>();
                    vis.set_if_neq(Visibility::Inherited);
                }
                _ => {}
            }
            let seen = npc.net.as_ref().map_or(p.shots, |o| o.shots_seen);
            npc.net = Some(Box::new(NetPose {
                pos: DVec3::new(p.pos[0] as f64, p.pos[1] as f64, p.pos[2] as f64),
                yaw: p.yaw as f64,
                anim: Some((p.anim.clone(), p.t, p.rate, p.looping)),
                speed: 0.0,
                crouch: false,
                swim: false,
                dead: p.dead,
                weapon: p.weapon.clone(),
                shots: p.shots,
                shots_seen: seen,
                seen: true,
            }));
        }
        // Get in near the host once his body is here.
        let host_at = with(|n| {
            if n.placed_near_host {
                return None;
            }
            n.remotes.iter().find(|r| r.id == 0 && r.at.is_some() && r.st.map == my_map).map(|r| r.st.pos)
        })
        .flatten();
        if let Some(h) = host_at {
            let h = DVec3::new(h[0] as f64, h[1] as f64, h[2] as f64);
            player.body.location = spot_near(lvl, &blockers, h, player.params.radius, player.params.height);
            player.body.velocity = DVec3::ZERO;
            // Facing the host.
            let d = h - player.body.location;
            for mut c in &mut cams {
                c.yaw = -(d.y.atan2(d.x) as f32) - std::f32::consts::FRAC_PI_2;
                c.pitch = 0.0;
            }
            with(|n| n.placed_near_host = true);
        }
    }

    // DXVIEW_NETLOG: what this side has, every 5 s.
    if env_on!("DXVIEW_NETLOG") {
        static T: Mutex<f32> = Mutex::new(0.0);
        let mut t = T.lock().unwrap_or_else(|e| e.into_inner());
        *t += dt;
        if *t >= 5.0 {
            *t = 0.0;
            let puppets = npcs.iter().filter(|(_, n, ..)| n.net.is_some() && n.net_player.is_none()).count();
            let bodies: Vec<String> = npcs
                .iter()
                .filter_map(|(_, n, d, ..)| n.net_player.map(|p| format!("{p}@{:.0},{:.0},{:.0} {}", d.body.location.x, d.body.location.y, d.body.location.z, n.anim)))
                .collect();
            let me = player.body.location;
            with(|n| {
                println!(
                    "NETLOG {:?} id {} welcomed {} remotes {:?} puppets {puppets} bodies {bodies:?} me {:.0},{:.0},{:.0} hp {:.0}",
                    n.role,
                    n.my_id,
                    n.welcomed,
                    n.remotes.iter().map(|r| (r.id, r.look.name.clone(), r.ping as i32)).collect::<Vec<_>>(),
                    me.x,
                    me.y,
                    me.z,
                    ph.hp
                )
            });
        }
    }
    // Death and getting up again near the host.
    respawn_tick(&mut ph, &mut player, &mut inv, lvl, &blockers, dt, my_map);
}

fn respawn_tick(
    ph: &mut PlayerHealth,
    player: &mut crate::Player,
    inv: &mut crate::pickups::PlayerInventory,
    lvl: &dx_level::Level,
    blockers: &crate::actors::Blockers,
    dt: f32,
    my_map: u32,
) {
    let up = with(|n| {
        if !ph.dead {
            n.respawn = None;
            return None;
        }
        if n.respawn.is_none() {
            // Just went down: who did it is everyone's news.
            let by = n.last_by.take();
            match n.role {
                Role::Host => died(n, 0, by),
                Role::Client => send_host(n, &Msg::Died { by }, true),
                Role::Off => {}
            }
        }
        let t = n.respawn.get_or_insert(RESPAWN_SECS);
        *t -= dt;
        if *t > 0.0 {
            return None;
        }
        n.respawn = None;
        // Near the host (or, for the host, near any other living player); nobody: where we fell.
        let near = n
            .remotes
            .iter()
            .filter(|r| r.at.is_some() && !r.st.dead && r.st.map == my_map)
            .min_by_key(|r| r.id)
            .map(|r| DVec3::new(r.st.pos[0] as f64, r.st.pos[1] as f64, r.st.pos[2] as f64));
        // Deathmatch: somewhere away from everyone instead.
        let others: Vec<DVec3> = n.remotes.iter().filter(|r| r.at.is_some()).map(|r| DVec3::new(r.st.pos[0] as f64, r.st.pos[1] as f64, r.st.pos[2] as f64)).collect();
        Some((if n.rules.coop { near } else { dm_spot(lvl, &others) }, n.rules.difficulty >= 2.0 && n.rules.coop))
    })
    .flatten();
    let Some((near, hard)) = up else { return };
    let at = near.unwrap_or(player.body.location);
    *ph = PlayerHealth::default();
    player.body = imsim_core::movement::Character::new(spot_near(lvl, blockers, at, player.params.radius, player.params.height));
    player.body.physics = imsim_core::movement::Physics::Falling;
    player.noclip = false;
    if hard {
        // Hard and Realistic: the dead lose what they carried.
        inv.0 = imsim_core::inventory::Inventory::default();
        crate::classic_hud::client_message("You got up again with nothing.");
    } else {
        crate::classic_hud::client_message("You got up again.");
    }
}

/// Deathmatch: a navigation point far from the other players (the best of a few tries).
fn dm_spot(lvl: &dx_level::Level, others: &[DVec3]) -> Option<DVec3> {
    let pts = &lvl.nav.points;
    if pts.is_empty() {
        return None;
    }
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos() as usize).unwrap_or(7);
    (0..24)
        .map(|k| {
            let p = pts[(seed.wrapping_mul(2654435761).wrapping_add(k * 7919)) % pts.len()].pos;
            DVec3::new(p[0], p[1], p[2])
        })
        .max_by(|a, b| {
            let d = |p: &DVec3| others.iter().map(|o| (*o - *p).length()).fold(f64::MAX, f64::min);
            d(a).total_cmp(&d(b))
        })
}

/// Host: a player went down - the line for everyone, and the deathmatch score.
fn died(n: &mut Net, who: u8, by: Option<u8>) {
    let name = |n: &Net, id: u8| -> String {
        if id == 0 && n.role == Role::Host {
            n.look.name.clone()
        } else {
            n.remotes.iter().find(|r| r.id == id).map(|r| r.look.name.clone()).unwrap_or_else(|| format!("Player {id}"))
        }
    };
    n.scores.entry(who).or_default().1 += 1;
    let line = match by {
        Some(k) if k != who => {
            n.scores.entry(k).or_default().0 += 1;
            let (kills, _) = n.scores[&k];
            if n.rules.coop {
                format!("{} was shot by {}.", name(n, who), name(n, k))
            } else {
                format!("{} killed {} ({} kills).", name(n, k), name(n, who), kills)
            }
        }
        _ => format!("{} went down.", name(n, who)),
    };
    if n.role == Role::Host {
        crate::classic_hud::client_message(&line);
        send_all(n, &Msg::Note(line), true, None);
    }
}

/// A free spot beside `at` for a body of this size (tries a ring, then above).
fn spot_near(lvl: &dx_level::Level, blockers: &crate::actors::Blockers, at: DVec3, r: f64, h: f64) -> DVec3 {
    let e = DVec3::new(r, r, h);
    let free = |p: DVec3| {
        lvl.collision.trace(p, p + DVec3::Z, e).fraction > 0.99
            && lvl.collision.trace(at, p, DVec3::splat(4.0)).fraction > 0.99
            && !blockers.list.iter().any(|c| c.radius > 0.0 && (c.center - p).truncate().length() < c.radius + r && (c.center.z - p.z).abs() < c.height + h)
    };
    for ring in [80.0, 130.0, 190.0] {
        for k in 0..12 {
            let a = k as f64 / 12.0 * std::f64::consts::TAU;
            let p = at + DVec3::new(a.cos() * ring, a.sin() * ring, 4.0);
            if free(p) {
                return p;
            }
        }
    }
    at + DVec3::Z * (h * 2.2)
}

/// A player's body here: JCDouble with his face (or Lay D Denton's body when he plays her and her files are here).
fn spawn_body(id: u8, look: &Look, st: &PState) {
    use unreal_pkg::props::{Property, Value};
    let n = match look.skin.min(4) {
        0 => 0,
        k => k as u32 + 3,
    };
    let mut props = vec![
        Property { name: "Alliance".into(), index: 0, value: Value::Name("Player".into()) },
        Property { name: "bInvincible".into(), index: 0, value: Value::Bool(true) },
        Property { name: "Orders".into(), index: 0, value: Value::Name("Standing".into()) },
        Property { name: "BindName".into(), index: 0, value: Value::Str(format!("NetPlayer{id}")) },
        Property { name: "FamiliarName".into(), index: 0, value: Value::Str(look.name.clone()) },
        Property { name: "UnfamiliarName".into(), index: 0, value: Value::Str(look.name.clone()) },
    ];
    if look.female && crate::travel::lddp() {
        props.push(Property { name: "Mesh".into(), index: 0, value: Value::Path("DeusExCharacters.GFM_Trench".into()) });
        let f = |t: &str| format!("FemJC.{t}");
        let skins = [
            f(&format!("JCDentonFemaleTex{n}")),
            f("JCDentonFemaleTex2"),
            f("JCDentonFemaleTex3"),
            f("JCDentonFemaleTex0"),
            f("JCDentonFemaleTex1"),
            f("JCDentonFemaleTex2"),
            "DeusExCharacters.Skins.FramesTex4".into(),
            "DeusExCharacters.Skins.LensesTex5".into(),
        ];
        for (i, s) in skins.into_iter().enumerate() {
            props.push(Property { name: "MultiSkins".into(), index: i as u32, value: Value::Path(s) });
        }
    } else {
        props.push(Property { name: "MultiSkins".into(), index: 0, value: Value::Path(format!("DeusExCharacters.Skins.JCDentonTex{n}")) });
    }
    let yaw = (st.yaw as f64 / std::f64::consts::TAU * 65536.0) as i32;
    crate::spawn::request(crate::spawn::SpawnReq {
        class: "JCDouble".into(),
        pos: DVec3::new(st.pos[0] as f64, st.pos[1] as f64, st.pos[2] as f64),
        yaw,
        transient: false,
        props,
        generator: None,
        net_player: Some(id),
    });
}

/// Turn one network event into work for the game.
fn handle(role: Role, e: NetEvent, lvl: &dx_level::Level) {
    match e {
        NetEvent::Joined(id, route) => {
            println!("NET joined {id} via {route:?}");
            with(|n| {
                if role == Role::Client {
                    n.status = "Connected. Waiting for the host's game...".into();
                    let look = n.look.clone();
                    send_host(n, &Msg::Hello { proto: PROTO, look, version: env!("CARGO_PKG_VERSION").into() }, true);
                }
            });
        }
        NetEvent::Left(id, why) => {
            println!("NET left {id}: {why}");
            with(|n| {
                if role == Role::Client && id == 0 {
                    n.lost = Some(if n.welcomed { format!("Lost the host ({why}).") } else { format!("Couldn't join: {why}") });
                    return;
                }
                if let Some(i) = n.remotes.iter().position(|r| r.id == id) {
                    let r = n.remotes.remove(i);
                    let line = format!("{} left the game.", r.look.name);
                    crate::classic_hud::client_message(&line);
                    if role == Role::Host {
                        send_all(n, &Msg::Note(line), true, None);
                        send_roster(n);
                    }
                }
            });
        }
        NetEvent::PublicAddr(a) => {
            static SEEN: Mutex<Option<SocketAddr>> = Mutex::new(None);
            if let Ok(mut s) = SEEN.lock() {
                if *s != Some(a) {
                    *s = Some(a);
                    println!("NET public address {a}");
                }
            }
        }
        NetEvent::Message { peer, data, .. } => {
            let Some(m) = decode(&data) else { return };
            with(|n| on_msg(n, role, peer, m, lvl));
        }
    }
}

fn send_roster(n: &Net) {
    let mut list = vec![(0u8, n.look.clone())];
    list.extend(n.remotes.iter().map(|r| (r.id, r.look.clone())));
    send_all(n, &Msg::Roster(list), true, None);
}

fn remote_mut(n: &mut Net, id: u8) -> &mut Remote {
    if let Some(i) = n.remotes.iter().position(|r| r.id == id) {
        return &mut n.remotes[i];
    }
    n.remotes.push(Remote { id, look: Look::default(), st: PState::default(), at: None, body: None, blocker: None, asked: false, waiting: false, ping: 0.0 });
    n.remotes.last_mut().unwrap()
}

fn on_msg(n: &mut Net, role: Role, peer: u8, m: Msg, _lvl: &dx_level::Level) {
    match (role, m) {
        // Host side.
        (Role::Host, Msg::Hello { proto, mut look, version }) => {
            if proto != PROTO {
                send_to(n, peer, &Msg::Refuse(format!("The host plays version {} (you have {version}).", env!("CARGO_PKG_VERSION"))), true);
                let _ = n.tx.send(Out::Kick(peer, "version".into()));
                return;
            }
            look.name = imsim_net::show_name(&imsim_net::clean_name(&look.name));
            let line = format!("{} joined the game.", look.name);
            let r = remote_mut(n, peer);
            r.look = look;
            r.waiting = true;
            crate::classic_hud::client_message(&line);
            send_all(n, &Msg::Note(line), true, Some(peer));
            send_roster(n);
        }
        (Role::Host, Msg::Me(st)) => {
            let r = remote_mut(n, peer);
            r.st = st;
            r.at = Some(Instant::now());
        }
        (Role::Host, Msg::Event(name, on)) => n.events_in.push((name, on)),
        (Role::Host, Msg::FrobMover(i)) => n.frob_movers.push((peer, i)),
        (Role::Host, Msg::FrobFixture(a)) => n.frob_fixtures.push(a),
        (Role::Host, Msg::Lock(i, l)) => n.locks.push((i, l)),
        (Role::Host, Msg::Hit { on, damage, kind, at, from }) => n.hits_in.push((peer, on, damage, kind, at, from)),
        (Role::Host, Msg::Noise { at, radius }) => n.noises_in.push((peer, at, radius)),
        (Role::Host, Msg::Died { by }) => died(n, peer, by),
        // Joiner side.
        (Role::Client, Msg::Welcome { you, map, rules }) => {
            force_look(&rules);
            n.remotes.retain(|r| r.id != you);
            n.my_id = you;
            n.rules = rules;
            n.welcomed = true;
            n.status = "In the host's game".into();
            n.host_map = Some(map.clone());
            n.travel_in = Some(map);
        }
        (Role::Client, Msg::Refuse(why)) => n.lost = Some(why),
        (Role::Client, Msg::Roster(list)) => {
            let me = n.my_id;
            n.remotes.retain(|r| list.iter().any(|(id, _)| *id == r.id));
            for (id, look) in list {
                if id != me {
                    remote_mut(n, id).look = look;
                }
            }
        }
        (Role::Client, Msg::Players(list)) => {
            let me = n.my_id;
            for (id, st) in list {
                if id != me {
                    let r = remote_mut(n, id);
                    r.st = st;
                    r.at = Some(Instant::now());
                }
            }
        }
        (Role::Client, Msg::Pawns(list)) => {
            for p in list {
                n.pawns_in.insert(p.actor, p);
            }
        }
        (Role::Client, Msg::Movers(list)) => n.movers_in.extend(list),
        (Role::Client, Msg::Flags(f)) => n.flags_in = Some(f),
        (Role::Client, Msg::Event(name, on)) => n.events_in.push((name, on)),
        (Role::Client, Msg::Damage { damage, kind, from, by }) => n.damage_in.push((damage, kind, from, by)),
        (Role::Client, Msg::Travel(map)) => {
            n.host_map = Some(map.clone());
            n.travel_in = Some(map);
        }
        (Role::Client, Msg::Speech { at, sound, name, text, radius }) => n.speech_in.push((at, sound, name, text, radius)),
        (Role::Client, Msg::Goals(g)) => n.goals_in = Some(g),
        (Role::Client, Msg::Rules(r)) => {
            force_look(&r);
            n.rules = r;
        }
        (Role::Client, Msg::Note(line)) => crate::classic_hud::client_message(&line),
        _ => {}
    }
}

/// Network out: this player's body, and the host's world for the joiners (after the pawns have moved).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn net_out_system(
    time: Res<Time>,
    level: Res<LevelRes>,
    player: Res<crate::Player>,
    ph: Res<PlayerHealth>,
    movers: Res<crate::movers::Movers>,
    flags: Res<crate::ai::GameFlags>,
    weapons: Res<crate::weapons::Weapons>,
    cam: Single<&Transform, (With<crate::FlyCam>, Without<crate::SkyCam>, Without<Npc>)>,
    npcs: Query<(&Npc, &crate::actors::Deco, &crate::actors::Animated, Option<&OutOfWorld>)>,
) {
    let Some(role) = with(|n| n.role) else { return };
    let _prof = crate::prof::scope("netplay::net_out_system");
    let dt = time.delta_secs();
    let lvl = &level.0;
    if crate::loader::busy() || !!crate::menus::intro_map() {
        return;
    }
    let ready = with(|n| role == Role::Host || n.welcomed).unwrap_or(false);
    if !ready {
        return;
    }
    let placed = crate::save::placed_count().min(lvl.actors.len());
    let my_map = map_hash(&lvl.name);
    // My body.
    let send_me = with(|n| {
        n.me_t += dt;
        if n.me_t >= 1.0 / ME_HZ {
            n.me_t = 0.0;
            true
        } else {
            false
        }
    })
    .unwrap_or(false);
    if send_me {
        let f = cam.forward().as_vec3();
        let yaw = (f.z).atan2(f.x);
        let pitch = f.y.clamp(-1.0, 1.0).asin();
        let b = &player.body;
        let st = PState {
            pos: [b.location.x as f32, b.location.y as f32, b.location.z as f32],
            vel: [b.velocity.x as f32, b.velocity.y as f32, b.velocity.z as f32],
            yaw,
            pitch,
            crouch: player.crouched,
            swim: player.swimming,
            dead: ph.dead,
            health: ph.hp.clamp(0.0, 100.0) as u8,
            weapon: weapons.held.as_ref().map(|h| h.stats.class.clone()).unwrap_or_default(),
            shots: with(|n| n.shots).unwrap_or(0),
            map: my_map,
        };
        with(|n| match role {
            Role::Client => send_host(n, &Msg::Me(st), false),
            Role::Host => {
                // Everyone's body to everyone (each joiner gets the others and the host).
                let mut all: Vec<(u8, PState)> = vec![(0, st)];
                all.extend(n.remotes.iter().filter(|r| r.at.is_some()).map(|r| (r.id, r.st.clone())));
                for r in n.remotes.iter().filter(|r| !r.waiting) {
                    let list: Vec<(u8, PState)> = all.iter().filter(|(id, _)| *id != r.id).cloned().collect();
                    send_to(n, r.id, &Msg::Players(list), false);
                }
            }
            Role::Off => {}
        });
    }
    if role != Role::Host || with(|n| n.remotes.iter().all(|r| r.waiting)).unwrap_or(true) {
        return;
    }
    // Pawns.
    let send_pawns = with(|n| {
        n.pawn_t += dt;
        if n.pawn_t >= 1.0 / PAWN_HZ {
            n.pawn_t = 0.0;
            true
        } else {
            false
        }
    })
    .unwrap_or(false);
    if send_pawns {
        let mut list = Vec::new();
        let mut alive = HashSet::new();
        for (npc, deco, an, out) in &npcs {
            if npc.net.is_some() || npc.net_player.is_some() || npc.actor >= placed || npc.blocker == usize::MAX {
                continue;
            }
            alive.insert(npc.actor as u32);
            let (t, rate, looping) = an.clip.as_ref().map_or((an.time, 1.0, true), |c| (c.t, c.rate, c.looping));
            let dead = if npc.state == NpcState::Dead { if npc.knocked_out { 2 } else { 1 } } else { 0 };
            let l = deco.body.location;
            list.push(PawnNet {
                actor: npc.actor as u32,
                pos: [l.x as f32, l.y as f32, l.z as f32],
                yaw: npc.yaw as f32,
                anim: npc.anim.to_string(),
                t,
                rate,
                looping,
                dead,
                out_of_world: out.is_some(),
                gone: false,
                weapon: npc.weapon.as_ref().map(|w| w.class.clone()).unwrap_or_default(),
                shots: npc.net_shots,
            });
        }
        with(|n| {
            for a in n.pawns_alive.difference(&alive) {
                list.push(PawnNet {
                    actor: *a,
                    pos: [0.0; 3],
                    yaw: 0.0,
                    anim: String::new(),
                    t: 0.0,
                    rate: 0.0,
                    looping: false,
                    dead: 1,
                    out_of_world: true,
                    gone: true,
                    weapon: String::new(),
                    shots: 0,
                });
            }
            n.pawns_alive = alive;
            for chunk in list.chunks(24) {
                send_all(n, &Msg::Pawns(chunk.to_vec()), false, None);
            }
        });
    }
    // (The conversation system's lock is never taken inside ours: it calls in here holding its own.)
    let mut goals = crate::consys::lock().as_ref().map(|st| st.goals.clone());
    // Movers, when they change.
    with(|n| {
        n.mover_t += dt;
        if n.mover_t < 1.0 / MOVER_HZ {
            return;
        }
        n.mover_t = 0.0;
        if n.sent_movers.len() != movers.rt.len() {
            n.sent_movers = vec![None; movers.rt.len()];
        }
        let mut changed = Vec::new();
        for (i, rt) in movers.rt.iter().enumerate() {
            let m = MoverNet {
                idx: i as u16,
                loc: [rt.location.x as f32, rt.location.y as f32, rt.location.z as f32],
                rot: rt.rotation,
                key: rt.key_num,
                prev: rt.prev_key_num,
                interp: rt.interpolating,
                alpha: rt.alpha as f32,
                rate: rt.rate as f32,
                old_pos: [rt.old_pos.x as f32, rt.old_pos.y as f32, rt.old_pos.z as f32],
                old_rot: rt.old_rot,
                opening: rt.opening,
                locked: rt.locked,
                destroyed: rt.destroyed,
                close: rt.close_timer.map_or(-1.0, |t| t as f32),
            };
            // Only the moments that matter: a new keyframe, lock or state (not every step of a door's swing).
            let differs = match &n.sent_movers[i] {
                // Sent once (when a joiner comes in): the doors as they are now.
                None => true,
                Some(o) => o.key != m.key || o.prev != m.prev || o.interp != m.interp || o.locked != m.locked || o.destroyed != m.destroyed || (!m.interp && o.loc != m.loc),
            };
            if differs {
                n.sent_movers[i] = Some(m.clone());
                changed.push(m);
            } else if n.sent_movers[i].is_none() {
                n.sent_movers[i] = Some(m);
            }
        }
        for chunk in changed.chunks(16) {
            send_all(n, &Msg::Movers(chunk.to_vec()), true, None);
        }
        // Flags, when they change (checked twice a second).
        n.flag_t += 1.0 / MOVER_HZ;
        if n.flag_t >= 0.5 {
            n.flag_t = 0.0;
            let sig = flags_sig(&flags.0);
            if sig != n.flags_sig {
                n.flags_sig = sig;
                send_all(n, &Msg::Flags(flags.0.clone()), true, None);
            }
            if let Some(goals) = goals.take() {
                let sig = crate::util::fnv1a(&format!("{goals:?}"));
                if sig != n.goals_sig {
                    n.goals_sig = sig;
                    send_all(n, &Msg::Goals(goals), true, None);
                }
            }
        }
    });
}

fn flags_sig(f: &imsim_core::flags::FlagBase) -> u64 {
    let mut v: Vec<(&str, String, i32)> = f.iter().map(|(k, v, e)| (k, format!("{v:?}"), e)).collect();
    v.sort();
    let mut h: u64 = 0xcbf29ce484222325;
    for (k, val, e) in v {
        for b in k.bytes().chain(val.bytes()).chain(e.to_le_bytes()) {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h
}

/// A new map is going in: nothing of the old map's pawns or doors carries over.
pub fn reset_level() {
    with(|n| {
        n.pawns_in.clear();
        n.movers_in.clear();
        n.frob_movers.clear();
        n.frob_fixtures.clear();
        n.hits_in.clear();
        n.events_in.clear();
    });
}

/// Developer aid (two copies of the game on one machine, no menus): `DXVIEW_MP_HOST=<code file>` connects to
/// online services, hosts a public co-op room and writes its code to the file; `DXVIEW_MP_JOIN=<code file>`
/// waits for that file and joins the room in it. (DXVIEW_LOBBY points both at a test lobby.)
pub fn dev_system(mut stage: Local<u8>, mut wait: Local<f32>, time: Res<Time>) {
    let host = env_val!("DXVIEW_MP_HOST").map(str::to_owned);
    let join = env_val!("DXVIEW_MP_JOIN").map(str::to_owned);
    if host.is_none() && join.is_none() {
        return;
    }
    if crate::loader::busy() || crate::menus::intro_map() {
        return;
    }
    *wait += time.delta_secs();
    match *stage {
        0 => {
            crate::online::connect();
            *stage = 1;
        }
        1 => {
            if let crate::online::Status::Online { .. } = crate::online::status() {
                if host.is_some() {
                    let look = if crate::remaster::is_modern() { imsim_net::Look::Modern } else { imsim_net::Look::Classic };
                    crate::online::host(imsim_net::RoomSettings { name: "Dev host".into(), look, ..Default::default() });
                    *stage = 2;
                } else {
                    *stage = 3;
                }
            }
        }
        2 => {
            if let (Some(h), Some(path)) = (crate::online::hosted(), host.as_ref()) {
                println!("NET dev room {}", h.code);
                let _ = std::fs::write(path, &h.code);
                *stage = 9;
            }
            if let Some(e) = crate::online::take_host_error() {
                println!("NET dev host failed: {e}");
                *stage = 9;
            }
        }
        3 => {
            if let Some(code) = join.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).map(|c| c.trim().to_string()).filter(|c| c.len() == 4) {
                println!("NET dev joining {code}");
                crate::online::join(&code);
                *stage = 4;
            }
        }
        4 => {
            if let Some(r) = crate::online::take_join() {
                match r {
                    Ok(j) => {
                        let r = join_room(&j);
                        println!("NET dev join: {r:?}");
                    }
                    Err(e) => println!("NET dev join failed: {e}"),
                }
                *stage = 9;
            }
        }
        _ => {}
    }
}

/// Join a room the lobby found, as the character chosen (choose_character; else a fresh JC of the session).
pub fn join_room(j: &imsim_net::client::Joined) -> Result<(), String> {
    if CHARACTER.lock().map(|c| c.is_none()).unwrap_or(true) {
        choose_character(crate::travel::session().unwrap_or_default(), None);
    }
    join(j, my_look())
}

// ---------------------------------------------------------------------------------------------------------------
// The joiner's own character (save.rs keeps it in Save/Multiplayer/<id>.ron).

struct Character {
    id: String,
    carry: Option<crate::save::SaveGame>,
}

static CHARACTER: Mutex<Option<Character>> = Mutex::new(None);
static SAVE_NOW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The character a joiner plays: a new one (from the character creator) or one saved before (`carry`: what he
/// had then). Set before joining.
pub fn choose_character(mut ng: crate::travel::NewGame, carry: Option<crate::save::SaveGame>) {
    if ng.char_id.is_empty() {
        ng.char_id = crate::save::new_char_id(&ng.name);
    }
    let id = ng.char_id.clone();
    crate::travel::set_session(ng);
    *CHARACTER.lock().unwrap_or_else(|e| e.into_inner()) = Some(Character { id, carry });
}

/// A saved multiplayer character, ready to play (None if the file is gone).
pub fn load_character(id: &str) -> Option<()> {
    let s = crate::save::read_mp(id)?;
    let g = &s.global;
    let ng = crate::travel::NewGame {
        name: g.name.clone(),
        skin: g.skin,
        difficulty: g.difficulty,
        skills: g.skills.clone(),
        points_avail: g.points.0,
        points_total: g.points.1,
        female: g.female,
        retro_morpheus: g.retro_morpheus,
        male_interactions: g.male_interactions,
        char_id: id.to_string(),
    };
    choose_character(ng, Some(s));
    Some(())
}

/// The loader's new game on the host's map: the joiner's character's carried things, once.
pub fn take_character_carry() -> Option<crate::save::SaveGame> {
    if !is_client() {
        return None;
    }
    CHARACTER.lock().ok()?.as_mut()?.carry.take()
}

/// Joiner: write the character now (before a map change).
pub fn save_character_soon() {
    SAVE_NOW.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Joiner: the character is written every 30 s in the host's game, before each map change and when asked.
pub fn char_save_system(world: &mut World, mut t: Local<f32>) {
    if !is_client() || !in_game() {
        *t = 0.0;
        return;
    }
    let dt = world.resource::<Time>().delta_secs();
    *t += dt;
    let now = SAVE_NOW.swap(false, std::sync::atomic::Ordering::Relaxed);
    if !(now || *t >= 30.0) || crate::loader::busy() || crate::save::restoring() || crate::menus::intro_map() {
        return;
    }
    *t = 0.0;
    let id = CHARACTER.lock().ok().and_then(|c| c.as_ref().map(|c| c.id.clone()));
    if let Some(id) = id {
        crate::save::write_mp(world, &id);
    }
}

/// Quitting the game says goodbye to the other players (the host's room closes for them at once).
pub fn exit_system(mut ev: MessageReader<AppExit>) {
    if ev.read().next().is_some() && active() {
        leave("");
    }
}
