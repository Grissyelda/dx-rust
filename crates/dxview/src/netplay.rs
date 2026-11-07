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
use imsim_net::game::{self, Endpoint, Event as NetEvent, HostAuth, JoinInfo, PeerId};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The game messages' version (Hello / Welcome).
pub const PROTO: u32 = 3;
/// Seconds before a dead player gets up again near the host (Easy / Medium).
pub const RESPAWN_SECS: f32 = 4.0;
/// Hard and Realistic co-op: the wait is longer, and if every player is down at once the level restarts for all.
pub const RESPAWN_HARD_SECS: f32 = 10.0;
/// Seconds between "everyone is down" and the host reloading the map.
const RESET_DELAY: f32 = 3.0;
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
    /// Deathmatch / Team Deathmatch: the match's rules (deathmatch.rs); None = co-op.
    pub dm: Option<crate::deathmatch::DmRules>,
}

impl Default for Rules {
    fn default() -> Self {
        Rules { coop: true, classic: false, tough: false, friendly_fire: false, max_players: 4, difficulty: 1.5, dm: None }
    }
}

/// How a player looks to the others.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Look {
    pub name: String,
    pub female: bool,
    pub skin: u8,
    /// The Augmentique outfit he wears, resolved (mesh, Texture, MultiSkins): his body is dressed in it.
    pub outfit: Option<(String, String, [String; 8])>,
    /// His weapon skins (class, Augmentique skin id): his body's weapon wears them.
    pub skins: Vec<(String, String)>,
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
    /// The flashlight augmentation is on.
    pub flash: bool,
    /// Augmentations running (shown as their glow on this player's body for the others).
    pub augs: Vec<String>,
    /// How much light falls on him (stealth::PlayerLight.brightness), and thermoptic camo: the host's pawns see
    /// his body by the same rules as the host's own JC.
    pub light: f32,
    pub camo: bool,
    /// The loader generation of his map: a new value is a new arrival (a reload of the same map counts).
    pub gen: u32,
}

/// The augmentations whose glow shows on a body.
const SHOWN_AUGS: [&str; 19] = [
    "AugVision", "AugTarget", "AugLight", "AugDrone", "AugSpeed", "AugStealth", "AugMuscle", "AugCombat", "AugBallistic",
    "AugEMP", "AugShield", "AugEnviro", "AugHealing", "AugDefense", "AugCloak", "AugRadarTrans", "AugPower", "AugAqualung",
    "AugHeartLung",
];

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
    /// Not drawn on the host (a body somebody carries, gibbed, swapped for its carcass).
    pub hidden: bool,
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
    /// `mods`: the mods this player runs (unreal_pkg::mods ids): a room's players all run the same ones.
    Hello { proto: u32, look: Look, version: String, mods: Vec<String> },
    /// Host -> a joiner who said hello: his player id.
    YouAre(u8),
    /// A joiner is ready (or not) to start.
    Ready(bool),
    /// A line typed in chat (host -> everyone: `from` = who said it).
    Chat { from: u8, text: String },
    Welcome { you: u8, map: String, rules: Rules },
    Refuse(String),
    /// Every player (the host is 0) and whether he's ready.
    Roster(Vec<(u8, Look, bool)>),
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
    /// `at`: where it hit, from the body's centre (None: unknown); `weapon`: the attacker's weapon class.
    Damage { damage: f32, kind: String, from: [f32; 3], by: Option<u8>, at: Option<[f32; 3]>, weapon: String },
    /// A player went down (`by`: the player who did it).
    Died { by: Option<u8>, kind: String },
    Travel(String),
    Speech { at: Option<[f32; 3]>, sound: String, name: String, text: String, radius: f32 },
    Noise { at: [f32; 3], radius: f32 },
    Rules(Rules),
    /// The host's goals (persona Goals screen), whenever they change.
    Goals(Vec<crate::consys::Goal>),
    /// A line for everyone's log ("X joined").
    Note(String),
    /// Host -> all: placed decorations that moved, and ones that broke, on map `map`.
    Decos { map: u32, moved: Vec<DecoNet>, gone: Vec<u32> },
    /// Joiner -> host: a decoration he pushed / carried / threw.
    DecoMove(u32, DecoNet),
    /// Joiner -> host: his shot / blow on a placed decoration.
    DecoHit { actor: u32, damage: f32, kind: String },
    /// Host -> all: an item dropped in the world (`id` > 0); joiner -> host: an item he dropped (id 0).
    ItemSpawn(u32, ItemNet),
    /// Joiner -> host: he picks the item up. Host -> the taker: it's his (the item as it was).
    ItemTake(u32),
    ItemGranted(ItemNet),
    /// Host -> all: the item is gone (somebody took it).
    ItemGone(u32),
    /// Joiner -> host: he searches the body of placed pawn `actor`. Host -> him: what he found.
    Search(u32),
    Loot { actor: u32, items: Vec<(String, i32)>, key: Option<(String, String)> },
    /// Host -> all: the laser tripwires / beams whose beam is on (placed actors), on map `map`.
    Lasers(u32, Vec<u32>),
    /// A projectile somebody launched (rocket, dart, plasma...): everyone else sees it fly (it does no harm there).
    Ghost(GhostNet),
    /// A player used one of his medkits on player `to`: `amount` health points, given on `to`'s side (the host
    /// passes it on). `by`: the healer's name.
    Heal { to: u8, amount: f32, by: String },
    // --- Protocol 2 (added at the end: nothing above shifts) ---
    /// Host -> all: placed pawn `actor` was just killed or knocked out by this hit (the joiners' copies fall the
    /// same way: ragdoll push, death side).
    PawnDied { actor: u32, hit: [f32; 3], from: [f32; 3], damage: f32, vel: [f32; 3] },
    /// Host -> all: placed pawn `actor` was blown apart (robots explode: `boom` = the blast radius, 0 = none).
    PawnGib { actor: u32, at: [f32; 3], r: f32, h: f32, robot: bool, boom: f32 },
    /// Host -> all: every player is down on Hard / Realistic - the map restarts (the host's Travel that follows
    /// reloads it; joiners come back with what their character had at its last save).
    Reset(String),
    /// Joiner -> host: he threw (or placed on a wall, `wall` = the wall's normal) a grenade: the host's to run.
    GrenThrow { class: String, pos: [f32; 3], dir: [f32; 3], wall: Option<[f32; 3]> },
    /// Host -> all: a grenade the host runs came into the world (thrown, placed on a wall).
    GrenSpawn(GrenNet),
    /// Host -> all: grenades whose state changed (where, armed, disarmed, about to go), on map `map`.
    GrenStates(u32, Vec<GrenState>),
    /// Host -> all: grenade `id` is gone - exploded (the joiners draw the blast) or picked up.
    GrenGone { map: u32, id: u32, exploded: bool },
    /// Joiner -> host: he used (disarmed / picked up) grenade `id`.
    GrenFrob(u32),
    /// Joiner -> host: a blast of his own (rocket, prod...): grenades, cameras, turrets and lasers on the host feel it.
    Blast { at: [f32; 3], radius: f32, damage: f32, kind: String },
    /// Host -> all: security cameras and turrets as the host runs them, on map `map`.
    Security(u32, SecNet),
    /// Joiner -> host: a security computer's setting (0 camera on/off: a = 1/0; 1 camera pan: a, b = pitch, yaw;
    /// 2 turret mode: a = mode), by device tag.
    SecControl { kind: u8, tag: String, a: f32, b: f32 },
    /// Joiner -> host: his shot / blow on the turret gun of placed AutoTurret `base`.
    TurretHit { base: u32, damage: f32, kind: String },
    /// Host -> all: the alarm units sounding (placed actors), on map `map`.
    Alarms(u32, Vec<u32>),
    /// Host -> all: hackable devices' strength left (placed actors), on map `map`.
    Hacks(u32, Vec<(u32, f32)>),
    /// Joiner -> host: his multitool work on placed device `actor`: strength left.
    HackSet(u32, f32),
    /// Joiner -> host: HackAction on placed device `actor` (0 camera / turret switch, 1 alarm unit silenced).
    HackAct(u32, u8),
    /// Joiner -> host: his look changed (a deathmatch character chosen after joining).
    LookIs(Look),
    /// Host -> all: the scoreboard / match state (deathmatch.rs Board).
    Board(crate::deathmatch::Board),
    /// Host -> a killer: his kill counted with this weapon class (skin unlocks, Reward).
    Frag(String),
    /// Joiner -> host: he takes a placed deathmatch pickup. Host -> him: it's his.
    PickupAsk(u32),
    PickupGrant(u32),
    /// Host -> all: the placed deathmatch pickups that are there (map hash, actors).
    Pickups(u32, Vec<u32>),
}

/// A grenade the host runs, as it comes into the world.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct GrenNet {
    pub map: u32,
    pub id: u32,
    pub class: String,
    pub pos: [f32; 3],
    pub vel: [f32; 3],
    pub wall: Option<[f32; 3]>,
    pub fuse: f32,
    pub blast: f32,
}

/// A grenade's state (GS_* flags).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct GrenState {
    pub id: u32,
    pub pos: [f32; 3],
    pub vel: [f32; 3],
    pub rot: [i32; 3],
    pub flags: u8,
}
pub const GS_STUCK: u8 = 1;
pub const GS_ARMED: u8 = 2;
pub const GS_DISABLED: u8 = 4;
pub const GS_TICKING: u8 = 8;
/// Ids of grenades made while playing start here (below: placed grenades, by actor index + 1).
pub const GREN_RUNTIME: u32 = 0x4000_0000;

/// The security devices as the host runs them.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct SecNet {
    pub cams: Vec<CamNet>,
    pub turrets: Vec<TurretNet>,
}

/// A security camera: rotation (pitch, yaw), lens (LENS_*), CAM_* flags.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CamNet {
    pub actor: u32,
    pub rot: [i32; 2],
    pub lens: u8,
    pub flags: u8,
}

/// A turret (by its placed AutoTurret base): gun rotation, lens, TUR_* flags, shots fired (wrapping).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TurretNet {
    pub actor: u32,
    pub rot: [i32; 2],
    pub lens: u8,
    pub flags: u8,
    pub shots: u8,
}
pub const LENS_NONE: u8 = 0;
pub const LENS_BLACK: u8 = 1;
pub const LENS_GREEN: u8 = 2;
pub const LENS_RED: u8 = 3;
pub const LENS_YELLOW: u8 = 4;
pub const CAM_GONE: u8 = 1;
pub const CAM_ACTIVE: u8 = 2;
pub const CAM_TRIGGERED: u8 = 4;
pub const CAM_CONFUSED: u8 = 8;
pub const CAM_HUM: u8 = 16;
pub const TUR_GONE: u8 = 1;
pub const TUR_ACTIVE: u8 = 2;
pub const TUR_DISABLED: u8 = 4;
pub const TUR_FIRING: u8 = 8;
pub const TUR_HUM: u8 = 16;
pub const TUR_PLAYERS: u8 = 32;
pub const TUR_PAWNS: u8 = 64;
pub const TUR_CONFUSED: u8 = 128;

/// What the host said about a placed pawn's end (joiner side, read by ai::puppet_step).
#[derive(Clone, Debug)]
pub enum PawnFx {
    Died { hit: DVec3, from: DVec3, damage: f32, vel: DVec3 },
    Gibbed { at: DVec3, r: f64, h: f64, robot: bool, boom: f64 },
}

/// A placed decoration's place (Unreal units) and rotation.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DecoNet {
    pub actor: u32,
    pub pos: [f32; 3],
    pub rot: [i32; 3],
}

/// A property value an item carries over its class defaults (a canister's AddAugs, a weapon's ammo...).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum NetVal {
    Int(i32),
    Float(f32),
    Bool(bool),
    Name(String),
    Str(String),
    Path(String),
}

/// An item dropped in the world: class, where it starts, how it's thrown, rounds for a weapon, overrides.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ItemNet {
    pub map: u32,
    pub class: String,
    pub pos: [f32; 3],
    pub vel: [f32; 3],
    pub ammo: Option<i32>,
    pub extra: Vec<(String, u32, NetVal)>,
}

/// A projectile's launch, for the others to see.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GhostNet {
    pub map: u32,
    pub class: String,
    pub pos: [f32; 3],
    pub dir: [f32; 3],
    pub speed: Option<f32>,
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
    /// Ready to start (the host's lobby).
    pub ready: bool,
    /// Said hello (his name and look are known).
    pub hello: bool,
    /// The look his body here was built with (a new look - another character - rebuilds it).
    body_look: Option<Look>,
}

#[derive(Default)]
struct Inbox {
    events: Vec<NetEvent>,
    rtt: Vec<(u8, f32)>,
    dead: Option<String>,
}

struct Net {
    role: Role,
    /// Host: the room's public key (direct codes carry its fingerprint), the direct code once our address is
    /// known, and what happened finding it (for the Host Room screen).
    public: Option<[u8; 32]>,
    direct: Option<String>,
    direct_note: String,
    /// Host: the game has started (players in the lobby are let in only then; later ones drop straight in).
    started: bool,
    /// Joiner: we said we're ready.
    ready: bool,
    /// The mods this player runs.
    mods: Vec<String>,
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
    damage_in: Vec<(f32, String, [f32; 3], Option<u8>, Option<[f32; 3]>, String)>,
    /// Medkits other players used on us: (points, who).
    heals_in: Vec<(f32, String)>,
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
    /// Hard / Realistic: everyone is down. Host: seconds until the map is reloaded (then f32::MAX while it loads);
    /// joiner: the host said so (the next Travel reloads the map; nobody gets up meanwhile).
    reset_t: Option<f32>,
    /// Joiner: placed pawns (actors) hidden because the host hides them.
    net_hidden: HashSet<usize>,
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
    // Deathmatch characters dress in their outfit and skins for everyone (they need Augmentique).
    let dm = crate::deathmatch::versus();
    Look {
        name: s.as_ref().map(|s| s.name.clone()).unwrap_or_else(|| "JC Denton".into()),
        female: s.as_ref().is_some_and(|s| s.female),
        skin: s.as_ref().map_or(0, |s| s.skin.min(4)),
        outfit: if dm { crate::augmentique::worn_look() } else { None },
        skins: if dm { crate::dmchar::current().map(|c| c.weapon_skins).unwrap_or_default() } else { Vec::new() },
    }
}

/// The host's rules with the deathmatch room's (deathmatch.rs holds what the Host Room chose).
fn rules_now(n: &Net) -> Rules {
    let mut r = n.rules.clone();
    r.dm = crate::deathmatch::rules();
    if r.dm.is_some() {
        r.coop = false;
        r.classic = false;
        r.friendly_fire = r.dm.as_ref().is_some_and(|d| d.friendly_fire);
    }
    r
}

/// A joiner got the room's rules: a deathmatch room plays his deathmatch character.
fn joiner_rules(n: &mut Net, r: &Rules) {
    let was = crate::deathmatch::versus();
    crate::deathmatch::set_rules(r.dm.clone());
    if r.dm.is_some() && !was {
        let c = crate::dmchar::last().unwrap_or_else(|| {
            let mut c = crate::dmchar::fresh();
            c.name = n.look.name.clone();
            crate::dmchar::save(&mut c);
            crate::dmchar::set_current(c.clone());
            c
        });
        crate::dmchar::apply(&c);
        let look = my_look();
        if look != n.look {
            n.look = look.clone();
            send_host(n, &Msg::LookIs(look), true);
        }
    }
}

fn start(role: Role, ep: Endpoint, rules: Rules, look: Look) {
    let (tx, rx) = channel();
    let inbox = Arc::new(Mutex::new(Inbox::default()));
    let ib = inbox.clone();
    let _ = std::thread::Builder::new().name("netplay".into()).spawn(move || net_thread(ep, rx, ib));
    let mut mods: Vec<String> = unreal_pkg::mods::active_ids().into_iter().map(str::to_string).collect();
    mods.sort();
    let net = Net {
        role,
        public: None,
        direct: None,
        direct_note: String::new(),
        started: false,
        ready: false,
        mods,
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
        heals_in: Vec::new(),
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
        reset_t: None,
        net_hidden: HashSet::new(),
        shots: 0,
        lost: None,
    };
    if let Ok(mut l) = LAST_END.lock() {
        l.clear();
    }
    if let Ok(mut h) = HOST_HEARD.lock() {
        *h = None;
    }
    *SYNC.lock().unwrap_or_else(|e| e.into_inner()) = None;
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
    with(|n| {
        n.public = Some(public);
        n.started = !crate::menus::intro_map();
        n.direct_note = "Finding this PC's internet address...".into();
    });
    find_direct_code(port, public);
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
/// A Classic room is played on the plain game: the player's mods are switched off while he's in it (back on when
/// he leaves); a map already up is rebuilt in place when that or the look's visual mod settings change.
/// Android joins Classic rooms only (no Modern features, no mods there).
const ANDROID_CLASSIC_ONLY: &str = "This room plays Modern. DX RUST on Android joins Classic rooms only.";

fn force_look(r: &Rules) {
    let modern = !r.classic;
    let off = r.classic && !unreal_pkg::mods::installed_ids().is_empty();
    let mut rebuild = unreal_pkg::mods::set_suspended(off);
    if rebuild && off {
        if let Some(note) = classic_mods_note() {
            feed(&note.replace("Classic: ", "Classic room: "), NEWS);
        }
    }
    if crate::remaster::is_modern() != modern {
        rebuild |= crate::remaster::switch(modern);
    }
    if rebuild {
        crate::remaster::request_reload();
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
        n.mods = unreal_pkg::mods::active_ids().into_iter().map(str::to_string).collect();
        n.mods.sort();
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
    let ep = Endpoint::join(JoinInfo { host: HostAuth::Key(key), targets, lobby: lobby_udp(), ticket: j.ticket, force_relay: env_on!("DXVIEW_FORCE_RELAY") })
        .map_err(|e| format!("Couldn't open a game socket ({e})."))?;
    let rules = Rules { coop: j.mode == imsim_net::Mode::Coop, classic: j.look == imsim_net::Look::Classic, tough: j.tough, max_players: j.max_players, ..Rules::default() };
    start(Role::Client, ep, rules, look);
    Ok(())
}

/// Leave the session (a host's room closes for everyone).
pub fn leave(why: &str) {
    let mut g = NET.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(n) = g.take() {
        // Out of the room: no match, no board.
        crate::deathmatch::host_end();
        crate::deathmatch::set_rules(None);
        crate::deathmatch::set_my_id(0);
        // The player's mods come back (the title map, or the next map, is built with them).
        let resume = unreal_pkg::mods::set_suspended(false);
        if resume && !(n.role == Role::Client && n.welcomed) {
            crate::remaster::request_reload();
        }
        if n.role == Role::Host {
            close_router_port();
        }
        if let Ok(mut c) = CHATBOX.lock() {
            c.open = false;
        }
        let _ = n.tx.send(Out::Stop);
        // Long enough for the link thread to say goodbye (the game may be quitting).
        std::thread::sleep(Duration::from_millis(30));
        // A joiner's world was the host's: without him it's back to the title (his character is saved).
        if n.role == Role::Client && n.welcomed {
            drop(g);
            crate::loader::request("DX.dx", crate::loader::Goal::Title);
            if !why.is_empty() {
                if let Ok(mut l) = LAST_END.lock() {
                    *l = why.to_string();
                }
            }
            return;
        }
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

/// Host: the next deathmatch on `map` (file stem) - everyone loads it fresh, as his deathmatch character.
pub fn start_match(map: &str) {
    with(|n| {
        if n.role == Role::Host {
            send_all(n, &Msg::Reset(map.to_string()), true, None);
        }
    });
    host_travel(map);
    let ng = crate::dmchar::current().map(|c| crate::dmchar::new_game(&c)).or_else(crate::travel::session).unwrap_or_default();
    crate::loader::request(&format!("{map}.dx"), crate::loader::Goal::NewGame(ng));
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
                    // Turrets and blasts hurt everyone; the host's own shots only with friendly fire.
                    let world_hit = matches!(h.damage_type, "AutoShot" | "Exploded" | "Burned" | "Flamed" | "Shocked" | "Radiation" | "PoisonGas" | "TearGas" | "HalonGas");
                    if h.attacker.is_some() || world_hit || players_hurt {
                        let by = if h.attacker.is_none() { Some(0) } else { None };
                        // Where on him (from his body's centre) and with what (the host's own weapon for his shots).
                        let local = n.remotes.iter().find(|r| r.id == p).map(|r| [at[0] - r.st.pos[0], at[1] - r.st.pos[1], at[2] - r.st.pos[2]]);
                        let weapon = if h.attacker.is_none() { MY_WEAPON.lock().map(|w| w.clone()).unwrap_or_default() } else { String::new() };
                        send_to(n, p, &Msg::Damage { damage: h.damage, kind: h.damage_type.to_string(), from, by, at: local, weapon }, true);
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
    // (Android is Classic only.)
    active() || crate::platform::android()
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
            n.reset_t = None;
            n.net_hidden.clear();
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
                heard_host();
            }
        }
    });
    // A joiner who hasn't heard from the host for HOST_SILENCE seconds has lost him.
    if role == Role::Client && with(|n| n.welcomed).unwrap_or(false) && host_silent() {
        with(|n| n.lost = Some("Lost connection to the host.".into()));
    }
    if let Some(why) = with(|n| n.lost.take()).flatten() {
        crate::classic_hud::client_message(&why);
        leave(&why);
        return;
    }
    // The playing map isn't ready (loading, title): wait.
    let playable = !crate::loader::busy() && !crate::menus::intro_map();
    // Joiner: travel with the host.
    if role == Role::Client {
        let (travel, reset) = with(|n| (n.travel_in.take(), n.reset_t.is_some())).unwrap_or((None, false));
        if let (Some(map), true) = (travel.as_ref(), reset) {
            // Everyone was down: the map again, with what this character had at its last save (not what he died with).
            let id = CHARACTER.lock().ok().and_then(|c| c.as_ref().map(|c| c.id.clone()));
            let carry = id.as_deref().and_then(crate::save::read_mp);
            if let Ok(mut c) = CHARACTER.lock() {
                if let Some(c) = c.as_mut() {
                    c.carry = carry;
                }
            }
            println!("NET reset: reloading {map} from the character's last save");
            ALLOW_TRAVEL.store(true, std::sync::atomic::Ordering::Relaxed);
            let ng = crate::travel::session().unwrap_or_default();
            crate::loader::request(map, crate::loader::Goal::NewGame(ng));
            ALLOW_TRAVEL.store(false, std::sync::atomic::Ordering::Relaxed);
            return;
        }
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
            let mut welcomed = Vec::new();
            if !n.started {
                return;
            }
            for r in n.remotes.iter_mut().filter(|r| r.waiting) {
                r.waiting = false;
                welcomed.push(r.id);
            }
            for id in welcomed {
                send_to(n, id, &Msg::Welcome { you: id, map: map.clone(), rules: rules_now(n) }, true);
                send_to(n, id, &Msg::Flags(flags.0.clone()), true);
                n.sent_movers.clear();
                n.goals_sig = 0;
            }
        });
    }
    // Deathmatch (and the co-op scoreboard): the host keeps everyone up to date.
    if role == Role::Host {
        with(|n| {
            let r = rules_now(n);
            if r != n.rules {
                n.rules = r.clone();
                send_all(n, &Msg::Rules(r), true, None);
            }
            if crate::deathmatch::board().is_none() && n.started {
                let mut players = vec![(0u8, n.look.name.clone())];
                players.extend(n.remotes.iter().filter(|r| r.hello).map(|r| (r.id, r.look.name.clone())));
                crate::deathmatch::host_begin(&players);
            }
            if let Some(b) = crate::deathmatch::take_board() {
                send_all(n, &Msg::Board(b), true, None);
            }
            for line in crate::deathmatch::take_news() {
                send_all(n, &Msg::Note(line), true, None);
            }
            if let Some(v) = crate::deathmatch::take_pickup_state() {
                send_all(n, &Msg::Pickups(my_map, v), true, None);
            }
        });
        if let Some((map, after)) = crate::deathmatch::take_travel() {
            match after {
                crate::deathmatch::After::Room => crate::deathmatch::await_room(),
                _ => start_match(&map),
            }
        }
    }
    // Joiner: deathmatch pickups he reached for.
    if role == Role::Client {
        for a in crate::deathmatch::take_asked() {
            with(|n| send_host(n, &Msg::PickupAsk(a), true));
        }
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
                    // The shooter's weapon, and where it hit (from the victim's centre).
                    let weapon = with(|n| n.remotes.iter().find(|r| r.id == from_peer).map(|r| r.st.weapon.clone())).flatten().unwrap_or_default();
                    if p == 0 {
                        with(|n| n.last_by = Some(from_peer));
                        let c = player.body.location;
                        let local = [at[0] - c.x as f32, at[1] - c.y as f32, at[2] - c.z as f32];
                        take_hit(&mut ph, &player, dmg, &kind, from, Some(from_peer), Some(local), &weapon);
                    } else {
                        with(|n| {
                            let local = n.remotes.iter().find(|r| r.id == p).map(|r| [at[0] - r.st.pos[0], at[1] - r.st.pos[1], at[2] - r.st.pos[2]]);
                            send_to(n, p, &Msg::Damage { damage: dmg, kind, from, by: Some(from_peer), at: local, weapon }, true)
                        });
                    }
                }
            }
        }
    }
    // Damage the host sent us (pawns' shots at our body there).
    let dmg = with(|n| std::mem::take(&mut n.damage_in)).unwrap_or_default();
    for (d, kind, from, by, at, weapon) in dmg {
        with(|n| n.last_by = by);
        take_hit(&mut ph, &player, d, &kind, from, by, at, &weapon);
    }
    // Medkits other players used on us.
    let heals = with(|n| std::mem::take(&mut n.heals_in)).unwrap_or_default();
    for (amount, by) in heals {
        if ph.dead {
            continue;
        }
        let got = ph.heal(amount);
        crate::bodyfx::healed(crate::ai::P_TORSO);
        crate::sound::play_player("DeusExSounds.Generic.MedicalHiss", 1.0, 256.0);
        crate::classic_hud::client_message(&format!("{by} healed you ({} points)", got.round() as i32));
    }
    // Lines spoken on the host.
    let sp = with(|n| std::mem::take(&mut n.speech_in)).unwrap_or_default();
    for (at, sound, name, text, radius) in sp {
        if env_on!("DXVIEW_NETLOG") {
            println!("NET speech {name}: {text} ({sound})");
        }
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
                if r.body_look.as_ref().is_some_and(|l| *l != r.look) {
                    // He's playing another character now (name, skin, Lay D): the body goes and comes back as it.
                    gone_bodies.push((*e, *b));
                    r.body = None;
                    r.blocker = None;
                    r.asked = false;
                    r.body_look = None;
                    continue;
                }
                r.body = Some(*e);
                r.blocker = Some(*b);
            }
            if r.body.is_none() && !r.asked && r.at.is_some() && r.st.map == my_map {
                r.asked = true;
                r.body_look = Some(r.look.clone());
                reqs.push((r.id, r.look.clone(), r.st.clone()));
            }
        }
        for (e, p, b) in &bodies {
            if !n.remotes.iter().any(|r| r.id == *p) && !gone_bodies.iter().any(|(g, _)| g == e) {
                gone_bodies.push((*e, *b));
            }
        }
        // The host plays the character he picked after opening the room (a save, or a new one): everyone hears.
        if n.role == Role::Host {
            let look = my_look();
            if look != n.look {
                n.look = look;
                send_roster(n);
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
    // Other players' flashlights: a light where each one looks.
    let lights: Vec<(u8, DVec3, f32, f32)> = with(|n| {
        n.remotes
            .iter()
            .filter(|r| r.at.is_some() && r.st.flash && r.st.map == my_map && !r.st.dead)
            .map(|r| (r.id, DVec3::new(r.st.pos[0] as f64, r.st.pos[1] as f64, r.st.pos[2] as f64), r.st.yaw, r.st.pitch))
            .collect()
    })
    .unwrap_or_default();
    // Other players' running augmentations: a soft light in each aug's colour on their bodies.
    let glows: Vec<(u8, DVec3, Vec<String>)> = with(|n| {
        n.remotes
            .iter()
            .filter(|r| r.at.is_some() && !r.st.augs.is_empty() && r.st.map == my_map && !r.st.dead)
            .map(|r| (r.id, DVec3::new(r.st.pos[0] as f64, r.st.pos[1] as f64, r.st.pos[2] as f64), r.st.augs.clone()))
            .collect()
    })
    .unwrap_or_default();
    for (id, at, augs) in glows {
        for (k, a) in augs.iter().take(4).enumerate() {
            let c = crate::bodyfx::aug_colour(a);
            let (h, sat) = hue_sat(c.x, c.y, c.z);
            let key = 0xA060_0000_0000 | (id as u64) << 8 | k as u64;
            crate::dynlight::hold(key, at + DVec3::new(0.0, 0.0, 10.0 + 8.0 * k as f64), crate::dynlight::Spec::new(h, sat, 90, 0.9), 0.0);
        }
    }
    for (id, at, yaw, pitch) in lights {
        let dir = DVec3::new((yaw.cos() * pitch.cos()) as f64, (yaw.sin() * pitch.cos()) as f64, pitch.sin() as f64);
        let eye = at + DVec3::Z * 30.0;
        let tr = lvl.collision.trace(eye, eye + dir * 900.0, DVec3::ZERO);
        let hit = eye + dir * (900.0 * tr.fraction - 48.0).max(16.0);
        crate::dynlight::hold(0xF00D_0000_0000 | id as u64, hit, crate::dynlight::Spec::new(32, 32, 210, 5.0), 0.0);
    }
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
        let mut net_hidden = with(|n| std::mem::take(&mut n.net_hidden)).unwrap_or_default();
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
            // A body carried off, blown apart or swapped for its carcass on the host: out of sight here too (and
            // back when the host puts it down). What this joiner hides himself (a body he carries) stays his.
            if p.dead != 0 && p.hidden && out.is_none() {
                if net_hidden.insert(npc.actor) {
                    vis.set_if_neq(Visibility::Hidden);
                    npc.carried = true;
                    npc.searched = true;
                }
            } else if !p.hidden && net_hidden.remove(&npc.actor) {
                vis.set_if_neq(Visibility::Inherited);
                npc.carried = false;
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
        with(|n| n.net_hidden = net_hidden);
        // Get in near the host once his body is here.
        let host_at = with(|n| {
            // (Deathmatch: everyone starts at a start of his own - deathmatch::match_system.)
            if n.placed_near_host || crate::deathmatch::playing() {
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

/// The weapon in this player's hands (deathmatch damage on the others' side).
static MY_WEAPON: Mutex<String> = Mutex::new(String::new());

/// A hit on this player from another machine: in a deathmatch through deathmatch.rs's table (by body region and
/// the attacker's weapon; teammates' share in Team Deathmatch), else as the game worked it out.
#[allow(clippy::too_many_arguments)]
fn take_hit(ph: &mut PlayerHealth, player: &crate::Player, d: f32, kind: &str, from: [f32; 3], by: Option<u8>, at: Option<[f32; 3]>, weapon: &str) {
    let from = DVec3::new(from[0] as f64, from[1] as f64, from[2] as f64);
    if crate::deathmatch::playing() {
        if crate::deathmatch::match_over() {
            return;
        }
        let me = crate::deathmatch::my_id();
        let mut mult = 1.0;
        if let Some(k) = by.filter(|k| *k != me) {
            if crate::deathmatch::allied(k, me) {
                mult = crate::deathmatch::friendly_mult();
            }
        }
        if mult <= 0.0 {
            return;
        }
        let local = at.map(|a| DVec3::new(a[0] as f64, a[1] as f64, a[2] as f64));
        let (dd, off) = crate::deathmatch::player_damage(weapon, kind, d, local, player.params.height, player.params.radius);
        ph.damage(dd * mult, kind, off);
        return;
    }
    let off = from - player.body.location;
    ph.damage(d, kind, off.normalize_or_zero() * player.params.radius);
}

enum Up {
    /// Get up again near `near` (None: where we fell); `hard`: with nothing.
    Respawn(Option<DVec3>, bool),
    /// Host: everyone stayed down - reload the map now.
    Reset,
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
        let hard = n.rules.coop && n.rules.difficulty >= 2.0;
        // Hard / Realistic: everyone down at once (each stays down RESPAWN_HARD_SECS, so the windows overlap) - the
        // level restarts for all. Players still loading or not in the game yet don't count.
        if n.role == Role::Host && hard && ph.dead && n.reset_t.is_none() {
            let all_down = n.remotes.iter().filter(|r| r.hello && !r.waiting && r.at.is_some() && r.st.map == my_map).all(|r| r.st.dead);
            if all_down {
                let line = "Everyone is down: the level restarts.".to_string();
                feed(&line, DEATH);
                send_all(n, &Msg::Note(line), true, None);
                send_all(n, &Msg::Reset(lvl.name.clone()), true, None);
                n.reset_t = Some(RESET_DELAY);
                n.respawn = None;
            }
        }
        if let Some(t) = n.reset_t.as_mut() {
            // Nobody gets up meanwhile (the host reloads, joiners wait for his Travel).
            if n.role == Role::Host && *t < f32::MAX {
                *t -= dt;
                if *t <= 0.0 {
                    *t = f32::MAX;
                    return Some(Up::Reset);
                }
            }
            return None;
        }
        if !ph.dead {
            n.respawn = None;
            return None;
        }
        if n.respawn.is_none() {
            // Just went down: who did it is everyone's news.
            let by = n.last_by.take();
            let kind = LAST_HURT.lock().map(|k| k.clone()).unwrap_or_default();
            if crate::deathmatch::playing() {
                crate::dmchar::note_death();
            }
            match n.role {
                Role::Host => died(n, 0, by, &kind),
                Role::Client => send_host(n, &Msg::Died { by, kind }, true),
                Role::Off => {}
            }
        }
        let t = n.respawn.get_or_insert(if hard { RESPAWN_HARD_SECS } else { RESPAWN_SECS });
        *t -= dt;
        if *t > 0.0 {
            return None;
        }
        n.respawn = None;
        if crate::deathmatch::playing() {
            crate::deathmatch::request_respawn();
            return None;
        }
        // Near the host (or, for the host, near any other living player); nobody: where we fell.
        let near = n
            .remotes
            .iter()
            .filter(|r| r.at.is_some() && !r.st.dead && r.st.map == my_map)
            .min_by_key(|r| r.id)
            .map(|r| DVec3::new(r.st.pos[0] as f64, r.st.pos[1] as f64, r.st.pos[2] as f64));
        // Deathmatch: somewhere away from everyone instead.
        let others: Vec<DVec3> = n.remotes.iter().filter(|r| r.at.is_some()).map(|r| DVec3::new(r.st.pos[0] as f64, r.st.pos[1] as f64, r.st.pos[2] as f64)).collect();
        Some(Up::Respawn(if n.rules.coop { near } else { dm_spot(lvl, &others) }, hard))
    })
    .flatten();
    let (near, hard) = match up {
        None => return,
        Some(Up::Reset) => {
            // The host's last save of this map (manual, quick or auto), else the map from its start. The loader's
            // travel brings every joiner along (Msg::Reset told them to reload it from their character's save).
            let path = crate::save::latest_for_map(&lvl.name);
            match path.filter(|p| crate::save::read(p).is_some()) {
                Some(p) => {
                    println!("NET reset: loading {}", p.display());
                    crate::save::load(std::path::Path::new(""), &p);
                }
                None => {
                    println!("NET reset: restarting {}", lvl.name);
                    crate::loader::request(&lvl.name, crate::loader::Goal::Restart);
                }
            }
            return;
        }
        Some(Up::Respawn(near, hard)) => (near, hard),
    };
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
fn died(n: &mut Net, who: u8, by: Option<u8>, kind: &str) {
    // The scoreboard (co-op counts deaths; deathmatch scores, says the line and credits the killer).
    let weapon = match by {
        Some(0) => MY_WEAPON.lock().map(|w| w.clone()).unwrap_or_default(),
        Some(k) => n.remotes.iter().find(|r| r.id == k).map(|r| r.st.weapon.clone()).unwrap_or_default(),
        None => String::new(),
    };
    let method = crate::deathmatch::method(&weapon, kind);
    let news = crate::deathmatch::host_died(who, by, &weapon, &method);
    if crate::deathmatch::playing() {
        if !news.line.is_empty() {
            feed(&news.line, DEATH);
            send_all(n, &Msg::Note(news.line.clone()), true, None);
        }
        if let Some((k, w)) = news.credit {
            if k == 0 {
                crate::dmchar::credit_kill(&w);
                crate::deathmatch::reward_kill();
            } else {
                send_to(n, k, &Msg::Frag(w), true);
            }
        }
        if let (Some(m), Some(k)) = (news.to_killer, by) {
            if k == 0 {
                crate::classic_hud::client_message(&m);
            } else {
                send_to(n, k, &Msg::Note(m), true);
            }
        }
        return;
    }
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
                format!("{} was {} by {}.", name(n, who), how_died(kind), name(n, k))
            } else {
                format!("{} killed {} ({} kills).", name(n, k), name(n, who), kills)
            }
        }
        _ => match kind {
            "Drowned" => format!("{} drowned.", name(n, who)),
            "Fell" => format!("{} fell to his death.", name(n, who)),
            "" => format!("{} went down.", name(n, who)),
            k => format!("{} was {}.", name(n, who), how_died(k)),
        },
    };
    if n.role == Role::Host {
        feed(&line, DEATH);
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
        // The male JC set in full (JCDouble's defaults with his face): on a Lay D Denton player's PC the JCDouble class
        // itself is dressed as her (travel's override layer), so nothing may be left to the class.
        props.push(Property { name: "Mesh".into(), index: 0, value: Value::Path("DeusExCharacters.GM_Trench".into()) });
        let t = |x: &str| format!("DeusExCharacters.Skins.{x}");
        let skins = [
            t(&format!("JCDentonTex{n}")),
            t("JCDentonTex2"),
            t("JCDentonTex3"),
            t("JCDentonTex0"),
            t("JCDentonTex1"),
            t("JCDentonTex2"),
            t("FramesTex4"),
            t("LensesTex5"),
        ];
        for (i, s) in skins.into_iter().enumerate() {
            props.push(Property { name: "MultiSkins".into(), index: i as u32, value: Value::Path(s) });
        }
    }
    // Deathmatch: his Augmentique outfit over the JC set, and his weapon skins on his body's weapon.
    if let Some((mesh, tex, skins)) = look.outfit.as_ref().filter(|_| crate::augmentique::active()) {
        props.retain(|p| !matches!(p.name.as_str(), "Mesh" | "MultiSkins" | "Texture" | "Skin"));
        props.push(Property { name: "Mesh".into(), index: 0, value: Value::Path(mesh.clone()) });
        props.push(Property { name: "Texture".into(), index: 0, value: Value::Path(tex.clone()) });
        props.push(Property { name: "Skin".into(), index: 0, value: Value::Path("None".into()) });
        for (i, s) in skins.iter().enumerate() {
            props.push(Property { name: "MultiSkins".into(), index: i as u32, value: Value::Path(s.clone()) });
        }
    }
    crate::deathmatch::set_remote_skins(id, look.skins.clone());
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
                    let mods = n.mods.clone();
                    send_host(n, &Msg::Hello { proto: PROTO, look, version: env!("CARGO_PKG_VERSION").into(), mods }, true);
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
                    let line = if why.contains("timed out") {
                        format!("{} lost connection.", r.look.name)
                    } else {
                        format!("{} left the game.", r.look.name)
                    };
                    sync(|s| {
                        s.synced.remove(&id);
                    });
                    if role == Role::Host {
                        crate::deathmatch::host_leave(id);
                    }
                    feed(&line, NEWS);
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
            if role == Role::Client && peer == 0 {
                heard_host();
            }
            let Some(m) = decode(&data) else { return };
            with(|n| on_msg(n, role, peer, m, lvl));
        }
    }
}

/// Names match as the room shows them: case, spaces and underscores aside.
fn name_key(s: &str) -> String {
    imsim_net::clean_name(s).to_ascii_lowercase()
}

/// Host: why `peer` can't play under `name` (the host or another player already does), else None.
fn name_refusal(n: &Net, peer: u8, name: &str) -> Option<String> {
    let k = name_key(name);
    let taken = name_key(&n.look.name) == k || n.remotes.iter().any(|r| r.hello && r.id != peer && name_key(&r.look.name) == k);
    taken.then(|| {
        format!(
            "Someone in this room already plays as {name}. Choose or make a character with another name, then join again."
        )
    })
}

/// Someone else in the session already plays under this name: his name as shown (menus refuse it for us).
pub fn name_taken(name: &str) -> Option<String> {
    let k = name_key(name);
    with(|n| n.remotes.iter().filter(|r| r.hello && r.id != n.my_id).find(|r| name_key(&r.look.name) == k).map(|r| r.look.name.clone()))
        .flatten()
}

fn send_roster(n: &Net) {
    let mut list = vec![(0u8, n.look.clone(), true)];
    list.extend(n.remotes.iter().filter(|r| r.hello).map(|r| (r.id, r.look.clone(), r.ready)));
    send_all(n, &Msg::Roster(list), true, None);
}

fn remote_mut(n: &mut Net, id: u8) -> &mut Remote {
    if let Some(i) = n.remotes.iter().position(|r| r.id == id) {
        return &mut n.remotes[i];
    }
    n.remotes.push(Remote {
        id,
        look: Look::default(),
        st: PState::default(),
        at: None,
        body: None,
        blocker: None,
        asked: false,
        waiting: false,
        ping: 0.0,
        ready: false,
        hello: false,
        body_look: None,
    });
    n.remotes.last_mut().unwrap()
}

fn on_msg(n: &mut Net, role: Role, peer: u8, m: Msg, _lvl: &dx_level::Level) {
    match (role, m) {
        // Host side.
        (Role::Host, Msg::Hello { proto, mut look, version, mut mods }) => {
            if proto != PROTO {
                send_to(n, peer, &Msg::Refuse(format!("The host plays version {} (you have {version}).", env!("CARGO_PKG_VERSION"))), true);
                let _ = n.tx.send(Out::Kick(peer, "version".into()));
                return;
            }
            // Mod parity: a Classic room is played without mods, a Modern one with the host's mods, every one.
            mods.sort();
            if let Some(why) = mods_mismatch(n.rules.classic, &n.mods, &mods) {
                send_to(n, peer, &Msg::Refuse(why), true);
                let _ = n.tx.send(Out::Kick(peer, "mods".into()));
                return;
            }
            look.name = imsim_net::show_name(&imsim_net::clean_name(&look.name));
            // No two players under one name (characters, feed lines and the scoreboard go by it).
            if let Some(why) = name_refusal(n, peer, &look.name) {
                send_to(n, peer, &Msg::Refuse(why), true);
                let _ = n.tx.send(Out::Kick(peer, "name".into()));
                return;
            }
            let line = format!("{} joined the game.", look.name);
            send_to(n, peer, &Msg::YouAre(peer), true);
            crate::deathmatch::host_join(peer, &look.name);
            let r = remote_mut(n, peer);
            r.look = look;
            r.waiting = true;
            r.hello = true;
            feed(&line, NEWS);
            send_all(n, &Msg::Note(line), true, Some(peer));
            send_roster(n);
        }
        (Role::Host, Msg::LookIs(mut look)) => {
            look.name = imsim_net::show_name(&imsim_net::clean_name(&look.name));
            if let Some(why) = name_refusal(n, peer, &look.name) {
                send_to(n, peer, &Msg::Refuse(why), true);
                let _ = n.tx.send(Out::Kick(peer, "name".into()));
                return;
            }
            crate::deathmatch::host_join(peer, &look.name);
            remote_mut(n, peer).look = look;
            send_roster(n);
        }
        (Role::Host, Msg::PickupAsk(a)) => {
            if crate::deathmatch::host_request(a) {
                send_to(n, peer, &Msg::PickupGrant(a), true);
            }
        }
        (Role::Host, Msg::Ready(on)) => {
            remote_mut(n, peer).ready = on;
            send_roster(n);
        }
        (Role::Host, Msg::Chat { text, .. }) => {
            let text = clean_chat(&text);
            if !text.is_empty() {
                let name = n.remotes.iter().find(|r| r.id == peer).map(|r| r.look.name.clone()).unwrap_or_default();
                feed(&format!("{name}: {text}"), CHAT);
                send_all(n, &Msg::Chat { from: peer, text }, true, Some(peer));
            }
        }
        (Role::Host, Msg::Me(st)) => {
            let r = remote_mut(n, peer);
            // A new map on his side (a reload of the same map too, after a reset): he gets the whole picture again.
            let arrived = r.at.is_some() && r.st.gen != st.gen;
            let first = r.at.is_none();
            r.st = st;
            r.at = Some(Instant::now());
            if arrived || first {
                send_to(n, peer, &Msg::Pickups(map_hash(&_lvl.name), crate::deathmatch::pickup_state()), true);
                if let Some(b) = crate::deathmatch::board() {
                    send_to(n, peer, &Msg::Board(b), true);
                }
            }
            if arrived {
                n.sent_movers.clear();
                n.flags_sig = 0;
                n.goals_sig = 0;
                sync(|s| {
                    s.synced.remove(&peer);
                });
            }
        }
        (Role::Host, Msg::Event(name, on)) => n.events_in.push((name, on)),
        (Role::Host, Msg::FrobMover(i)) => n.frob_movers.push((peer, i)),
        (Role::Host, Msg::FrobFixture(a)) => n.frob_fixtures.push(a),
        (Role::Host, Msg::Lock(i, l)) => n.locks.push((i, l)),
        (Role::Host, Msg::Hit { on, damage, kind, at, from }) => n.hits_in.push((peer, on, damage, kind, at, from)),
        (Role::Host, Msg::Heal { to, amount, by }) => {
            crate::deathmatch::host_heal(peer);
            if to == 0 {
                n.heals_in.push((amount, by));
            } else if to != peer {
                send_to(n, to, &Msg::Heal { to, amount, by }, true);
            }
        }
        (Role::Host, Msg::Noise { at, radius }) => n.noises_in.push((peer, at, radius)),
        (Role::Host, Msg::Died { by, kind }) => died(n, peer, by, &kind),
        (Role::Host, Msg::DecoMove(_, d)) => sync(|s| s.deco_moves.push(d)),
        (Role::Host, Msg::DecoHit { actor, damage, kind }) => sync(|s| s.deco_hits.push((actor, damage, kind))),
        (Role::Host, Msg::ItemSpawn(_, it)) => sync(|s| s.item_spawns.push((0, it))),
        (Role::Host, Msg::ItemTake(id)) => sync(|s| s.takes.push((peer, id))),
        (Role::Host, Msg::Search(actor)) => sync(|s| s.searches.push((peer, actor))),
        (Role::Host, Msg::Ghost(g)) => {
            send_all(n, &Msg::Ghost(g.clone()), false, Some(peer));
            sync(|s| s.ghosts.push(g));
        }
        (Role::Host, Msg::GrenThrow { class, pos, dir, wall }) => sync(|s| s.gren_throws.push((peer, class, d3(pos), d3(dir), wall.map(d3)))),
        (Role::Host, Msg::GrenFrob(id)) => sync(|s| s.gren_frobs.push(id)),
        (Role::Host, Msg::Blast { at, radius, damage, kind }) => sync(|s| {
            s.sec_blasts.push((d3(at), radius as f64, damage, kind.clone()));
            s.remote_blasts.push((d3(at), radius as f64, kind));
        }),
        (Role::Host, Msg::SecControl { kind, tag, a, b }) => sync(|s| s.sec_controls.push((kind, tag, a, b))),
        (Role::Host, Msg::TurretHit { base, damage, kind }) => sync(|s| s.turret_hits.push((base, damage, kind))),
        (Role::Host, Msg::HackSet(a, v)) => sync(|s| s.hacks_in.push((a, v))),
        (Role::Host, Msg::HackAct(a, k)) => sync(|s| s.hack_acts_in.push((a, k))),
        // Joiner side.
        (Role::Client, Msg::Welcome { you, map, rules }) if crate::platform::android() && !rules.classic => {
            let _ = (you, map);
            n.lost = Some(ANDROID_CLASSIC_ONLY.into());
        }
        (Role::Client, Msg::Rules(r)) if crate::platform::android() && !r.classic => {
            n.lost = Some(ANDROID_CLASSIC_ONLY.into());
        }
        (Role::Client, Msg::Welcome { you, map, rules }) => {
            force_look(&rules);
            // The host's difficulty is everyone's.
            if let Some(mut ng) = crate::travel::session() {
                if ng.difficulty != rules.difficulty {
                    ng.difficulty = rules.difficulty;
                    crate::travel::set_session(ng);
                }
            }
            n.remotes.retain(|r| r.id != you);
            n.my_id = you;
            crate::deathmatch::set_my_id(you);
            joiner_rules(n, &rules);
            n.rules = rules;
            n.welcomed = true;
            n.status = "In the host's game".into();
            n.host_map = Some(map.clone());
            n.travel_in = Some(map);
        }
        (Role::Client, Msg::Refuse(why)) => n.lost = Some(why),
        (Role::Client, Msg::YouAre(id)) => {
            n.my_id = id;
            crate::deathmatch::set_my_id(id);
            n.remotes.retain(|r| r.id != id);
            n.status = "In the host's lobby".into();
        }
        (Role::Client, Msg::Roster(list)) => {
            let me = n.my_id;
            n.remotes.retain(|r| list.iter().any(|(id, ..)| *id == r.id));
            for (id, look, ready) in list {
                if id != me {
                    let r = remote_mut(n, id);
                    r.look = look;
                    r.ready = ready;
                    r.hello = true;
                } else {
                    n.ready = ready;
                }
            }
        }
        (Role::Client, Msg::Chat { from, text }) => {
            let name = n.remotes.iter().find(|r| r.id == from).map(|r| r.look.name.clone()).unwrap_or_else(|| format!("Player {from}"));
            feed(&format!("{name}: {}", clean_chat(&text)), CHAT);
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
        (Role::Client, Msg::Damage { damage, kind, from, by, at, weapon }) => n.damage_in.push((damage, kind, from, by, at, weapon)),
        (Role::Client, Msg::Heal { amount, by, .. }) => n.heals_in.push((amount, by)),
        (Role::Client, Msg::Travel(map)) => {
            n.host_map = Some(map.clone());
            n.travel_in = Some(map);
        }
        (Role::Client, Msg::Speech { at, sound, name, text, radius }) => n.speech_in.push((at, sound, name, text, radius)),
        (Role::Client, Msg::Goals(g)) => n.goals_in = Some(g),
        (Role::Client, Msg::Rules(r)) => {
            force_look(&r);
            joiner_rules(n, &r);
            n.rules = r;
        }
        (Role::Client, Msg::Board(b)) => crate::deathmatch::set_board(b),
        (Role::Client, Msg::Frag(weapon)) => {
            crate::dmchar::credit_kill(&weapon);
            crate::deathmatch::reward_kill();
        }
        (Role::Client, Msg::PickupGrant(a)) => crate::deathmatch::grant_placed(a),
        (Role::Client, Msg::Pickups(map, there)) => {
            if map == map_hash(&_lvl.name) {
                crate::deathmatch::set_pickup_state(there);
            }
        }
        (Role::Client, Msg::Note(line)) => feed(&line, NEWS),
        (Role::Client, Msg::Decos { map, moved, gone }) => sync(|s| {
            if map == s.map {
                s.deco_moves.extend(moved);
                s.deco_gone.extend(gone);
            }
        }),
        (Role::Client, Msg::ItemSpawn(id, it)) => sync(|s| {
            if it.map == s.map {
                s.item_spawns.push((id, it));
            }
        }),
        (Role::Client, Msg::ItemGranted(it)) => sync(|s| s.granted.push(it)),
        (Role::Client, Msg::ItemGone(id)) => sync(|s| s.items_gone.push(id)),
        (Role::Client, Msg::Loot { actor, items, key }) => sync(|s| s.loot.push((actor, items, key))),
        (Role::Client, Msg::Lasers(map, on)) => sync(|s| {
            if map == s.map {
                s.lasers = Some(on);
            }
        }),
        (Role::Client, Msg::Ghost(g)) => sync(|s| s.ghosts.push(g)),
        (Role::Client, Msg::PawnDied { actor, hit, from, damage, vel }) => sync(|s| {
            s.pawn_fx.insert(actor, PawnFx::Died { hit: d3(hit), from: d3(from), damage, vel: d3(vel) });
        }),
        (Role::Client, Msg::PawnGib { actor, at, r, h, robot, boom }) => sync(|s| {
            s.pawn_fx.insert(actor, PawnFx::Gibbed { at: d3(at), r: r as f64, h: h as f64, robot, boom: boom as f64 });
        }),
        (Role::Client, Msg::Reset(_)) => {
            n.reset_t = Some(0.0);
            n.respawn = None;
        }
        (Role::Client, Msg::GrenSpawn(g)) => sync(|s| {
            if g.map == s.map {
                s.gren_spawns.push(g);
            }
        }),
        (Role::Client, Msg::GrenStates(map, v)) => sync(|s| {
            if map == s.map {
                s.gren_states_in.extend(v);
            }
        }),
        (Role::Client, Msg::GrenGone { map, id, exploded }) => sync(|s| {
            if map == s.map {
                s.gren_gone_in.push((id, exploded));
            }
        }),
        (Role::Client, Msg::Security(map, sec)) => sync(|s| {
            if map == s.map {
                s.sec_in = Some(sec);
            }
        }),
        (Role::Client, Msg::Alarms(map, v)) => sync(|s| {
            if map == s.map {
                s.alarms_in = Some(v);
            }
        }),
        (Role::Client, Msg::Hacks(map, v)) => sync(|s| {
            if map == s.map {
                s.hacks_in.extend(v);
            }
        }),
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
    plight: Res<crate::stealth::PlayerLight>,
    cam: Single<&Transform, (With<crate::FlyCam>, Without<crate::SkyCam>, Without<Npc>)>,
    npcs: Query<(&Npc, &crate::actors::Deco, &crate::actors::Animated, Option<&OutOfWorld>, &Visibility)>,
) {
    let Some(role) = with(|n| n.role) else { return };
    let _prof = crate::prof::scope("netplay::net_out_system");
    ME_DEAD.store(ph.dead, std::sync::atomic::Ordering::Relaxed);
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
            flash: crate::dynlight::flashlight().is_some(),
            augs: SHOWN_AUGS.iter().filter(|a| crate::abilities::aug_active(a)).map(|a| a.to_string()).collect(),
            light: plight.brightness,
            camo: ph.camo,
            gen: crate::loader::generation() as u32,
        };
        if let Ok(mut w) = MY_WEAPON.lock() {
            w.clone_from(&st.weapon);
        }
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
        for (npc, deco, an, out, vis) in &npcs {
            if npc.net.is_some() || npc.net_player.is_some() || npc.actor >= placed || npc.blocker == usize::MAX {
                continue;
            }
            alive.insert(npc.actor as u32);
            let (t, rate, looping) = an.clip.as_ref().map_or((an.time, 1.0, true), |c| (c.t, c.rate, c.looping));
            // The sequence actually playing: a clip set straight on the animation (the death fall, hit reactions)
            // doesn't always change `npc.anim` (a pawn killed mid-run kept "Run": the joiners' copy ran on).
            let anim = an.clip.as_ref().and_then(|c| clip_sequence(lvl, an.mesh_actor, c)).unwrap_or(npc.anim);
            let dead = if npc.state == NpcState::Dead { if npc.knocked_out { 2 } else { 1 } } else { 0 };
            let l = deco.body.location;
            list.push(PawnNet {
                actor: npc.actor as u32,
                pos: [l.x as f32, l.y as f32, l.z as f32],
                yaw: npc.yaw as f32,
                anim: anim.to_string(),
                t,
                rate,
                looping,
                dead,
                out_of_world: out.is_some(),
                gone: false,
                weapon: npc.weapon.as_ref().map(|w| w.class.clone()).unwrap_or_default(),
                shots: npc.net_shots,
                hidden: *vis == Visibility::Hidden,
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
                    hidden: true,
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

/// The name of the sequence a clip plays (the mesh's sequence starting at its first frame, that long).
fn clip_sequence(lvl: &dx_level::Level, mesh_actor: usize, c: &crate::weapons::Clip) -> Option<&'static str> {
    let m = lvl.mesh_actors.get(mesh_actor)?.anim.as_ref()?;
    let s = m
        .mesh
        .seqs
        .iter()
        .find(|s| s.start_frame.max(0) as usize == c.start && s.num_frames.max(1) as usize == c.frames && !s.name.eq_ignore_ascii_case("All"))?;
    Some(crate::util::intern(&s.name))
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
    // The host's direct code goes beside the room code (<file>.direct) once it's known.
    if let Some(path) = host.as_ref() {
        if *stage >= 2 && *stage != 20 {
            if let Some(code) = direct_code() {
                println!("NET dev direct code {code}");
                let _ = std::fs::write(format!("{path}.direct"), &code);
                *stage = if *stage == 9 { 20 } else { *stage };
            }
        }
    }
    match *stage {
        0 => {
            // Without online services (DXVIEW_MP_OFFLINE=1 for the host; a direct code in the joiner's file).
            if host.is_some() && env_on!("DXVIEW_MP_OFFLINE") {
                let rules = Rules { classic: !crate::remaster::is_modern(), ..Rules::default() };
                match open_host(rules) {
                    Ok((port, ..)) => println!("NET dev offline host on port {port}"),
                    Err(e) => println!("NET dev offline host failed: {e}"),
                }
                *stage = 9;
                return;
            }
            if let Some(code) = join.as_ref().map(|p| std::fs::read_to_string(p).unwrap_or_else(|_| p.clone())).map(|c| c.trim().to_string()) {
                if game::is_direct_code(&code) {
                    println!("NET dev joining direct {code}");
                    let r = join_code(&code);
                    println!("NET dev join: {r:?}");
                    *stage = 9;
                    return;
                }
                if host.is_none() && env_on!("DXVIEW_MP_OFFLINE") {
                    // Wait for the file to hold a direct code.
                    return;
                }
            }
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
    // (Deathmatch characters keep their own files: dmchar.rs.)
    if !is_client() || !in_game() || crate::deathmatch::versus() {
        *t = 0.0;
        return;
    }
    let dt = world.resource::<Time>().delta_secs();
    *t += dt;
    let now = SAVE_NOW.swap(false, std::sync::atomic::Ordering::Relaxed);
    if !(now || *t >= 30.0) || crate::loader::busy() || crate::save::restoring() || crate::menus::intro_map() {
        return;
    }
    // Not while down: the character's save is what he comes back with after a level restart.
    if world.get_resource::<PlayerHealth>().is_some_and(|p| p.dead) {
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

/// An RGB colour as Unreal's LightHue / LightSaturation bytes (saturation 0 = full colour, 255 = white).
fn hue_sat(r: f32, g: f32, b: f32) -> (u8, u8) {
    let max = r.max(g).max(b).max(1e-6);
    let min = r.min(g).min(b);
    let d = max - min;
    let s = d / max;
    let h = if d < 1e-6 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } / 6.0;
    ((h * 255.0) as u8, ((1.0 - s) * 255.0) as u8)
}

/// A player's body (host side): how much light falls on him (0..1), as his own game measured it.
pub fn body_light(id: u8) -> Option<f32> {
    with(|n| n.remotes.iter().find(|r| r.id == id && r.at.is_some()).map(|r| r.st.light)).flatten()
}

/// A player's body (host side): (crouched, ground speed, thermoptic camo on).
pub fn body_state(id: u8) -> Option<(bool, f32, bool)> {
    with(|n| {
        n.remotes.iter().find(|r| r.id == id && r.at.is_some()).map(|r| {
            let v = r.st.vel;
            (r.st.crouch, (v[0] * v[0] + v[1] * v[1]).sqrt(), r.st.camo)
        })
    })
    .flatten()
}

// ---------------------------------------------------------------------------------------------------------------
// Direct codes: joining without online services.

/// The router the host opened his port on (closed again when the room closes).
static GATEWAY: Mutex<Option<(imsim_net::upnp::Gateway, u16)>> = Mutex::new(None);

/// Host: work out the direct code on a thread - ask the router to open the game port (UPnP) and tell us its
/// internet address, else ask a public STUN server for the address (the port must then be forwarded by hand).
fn find_direct_code(port: u16, public: [u8; 32]) {
    let _ = std::thread::Builder::new().name("direct code".into()).spawn(move || {
        use imsim_net::upnp;
        let note: String;
        let mut ip = None;
        match upnp::find_gateway(Duration::from_millis(2500)) {
            Ok(g) => match g.add_port(upnp::Proto::Udp, port, 4 * 3600, "DX RUST game") {
                Ok(()) => {
                    ip = g.external_ip().ok().filter(|a| !a.is_private() && !a.is_unspecified());
                    note = format!("Your router opened port {port} for the game.");
                    if let Ok(mut gw) = GATEWAY.lock() {
                        *gw = Some((g, port));
                    }
                }
                Err(e) => note = format!("Couldn't open the port on the router ({e}): forward UDP port {port} to this PC."),
            },
            Err(_) => note = format!("No router answered (UPnP): forward UDP port {port} to this PC for players outside your network."),
        }
        if ip.is_none() {
            ip = upnp::public_ip().ok();
        }
        let code = ip.map(|ip| game::direct_code(ip, port, &public));
        with(|n| {
            if n.public == Some(public) {
                n.direct_note = match &code {
                    Some(_) => note,
                    None => "Couldn't find this PC's internet address: players on your network can still join with the code.".into(),
                };
                // Without an internet address the code still finds us on the LAN (it carries the key's fingerprint).
                n.direct = Some(code.unwrap_or_else(|| game::direct_code(std::net::Ipv4Addr::UNSPECIFIED, port, &public)));
            }
        });
    });
}

/// Host: the direct code (None while it's being worked out).
pub fn direct_code() -> Option<String> {
    with(|n| n.direct.clone()).flatten()
}

/// Host: what happened opening the router's port, for the Host Room screen.
pub fn direct_note() -> String {
    with(|n| n.direct_note.clone()).unwrap_or_default()
}

/// Whether `code` is a direct code (rather than a 4-character lobby code).
pub fn is_direct_code(code: &str) -> bool {
    game::is_direct_code(code)
}

/// Join with a direct code, as the character chosen (no online services needed).
pub fn join_code(code: &str) -> Result<(), String> {
    let (addr, fp) = game::parse_direct(code).ok_or("That isn't a room code.")?;
    if CHARACTER.lock().map(|c| c.is_none()).unwrap_or(true) {
        choose_character(crate::travel::session().unwrap_or_default(), None);
    }
    let look = my_look();
    leave("");
    let mut targets = Vec::new();
    if !addr.ip().is_unspecified() {
        targets.push(SocketAddr::V4(addr));
    }
    let ep = Endpoint::join(JoinInfo { host: HostAuth::Fingerprint(fp), targets, lobby: None, ticket: 0, force_relay: false })
        .map_err(|e| format!("Couldn't open a game socket ({e})."))?;
    start(Role::Client, ep, Rules::default(), look);
    Ok(())
}

/// Copy text (the direct code) to the system clipboard.
pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    crate::platform::clipboard_set(text)
}

fn close_router_port() {
    if let Some((g, port)) = GATEWAY.lock().ok().and_then(|mut g| g.take()) {
        let _ = std::thread::Builder::new().name("upnp close".into()).spawn(move || {
            let _ = g.remove_port(imsim_net::upnp::Proto::Udp, port);
        });
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Mods: everyone in a room runs the same ones.

fn mod_names(ids: &[String]) -> String {
    ids.iter().map(|i| unreal_pkg::mods::def(i).map_or(i.as_str(), |d| d.name)).collect::<Vec<_>>().join(", ")
}

// ---------------------------------------------------------------------------------------------------------------
// Healing each other: Use on another player's body with a medkit in the inventory.

/// Another player as the frob line sees him: (name, overall health percent, down).
pub fn mate(id: u8) -> Option<(String, u8, bool)> {
    with(|n| n.remotes.iter().find(|r| r.id == id).map(|r| (r.look.name.clone(), r.st.health, r.st.dead))).flatten()
}

/// The frob label over another player: his name, and what Use would do with the medkits we carry.
pub fn mate_label(id: u8, medkits: u32) -> String {
    let Some((name, health, dead)) = mate(id) else { return "Player".into() };
    if dead {
        format!("{name}\nDown")
    } else if health >= 100 {
        format!("{name}\nHealth {health}%")
    } else if medkits == 0 {
        format!("{name}\nHealth {health}% (no medkit)")
    } else {
        format!("{name}\nHealth {health}%: Use to heal ({medkits} medkit{})", if medkits == 1 { "" } else { "s" })
    }
}

/// Send `amount` health points to player `to` (one of our medkits, used up by the caller).
pub fn heal_player(to: u8, amount: f32) {
    with(|n| {
        let m = Msg::Heal { to, amount, by: n.look.name.clone() };
        if n.role == Role::Host {
            crate::deathmatch::host_heal(0);
            send_to(n, to, &m, true);
        } else {
            send_host(n, &m, true);
        }
    });
}

/// The note a host sees for his look: in a Classic room his mods are switched off while he plays (None = no mods).
pub fn classic_mods_note() -> Option<String> {
    let ids: Vec<String> = unreal_pkg::mods::installed_ids().into_iter().map(str::to_string).collect();
    (!ids.is_empty()).then(|| format!("Classic: {} switched off while you play.", mod_names(&ids)))
}

/// Why a joiner running `theirs` can't play in this room (None = he can).
fn mods_mismatch(classic: bool, host: &[String], theirs: &[String]) -> Option<String> {
    if classic {
        // A joiner's own mods are switched off when the room's rules reach him (force_look).
        return None;
    }
    if host == theirs {
        return None;
    }
    let missing: Vec<String> = host.iter().filter(|m| !theirs.contains(m)).cloned().collect();
    let extra: Vec<String> = theirs.iter().filter(|m| !host.contains(m)).cloned().collect();
    let mut why = "This Modern room's players all run the host's mods.".to_string();
    if !missing.is_empty() {
        why += &format!(" Install and turn on: {}.", mod_names(&missing));
    }
    if !extra.is_empty() {
        why += &format!(" Turn off: {}.", mod_names(&extra));
    }
    Some(why)
}

// ---------------------------------------------------------------------------------------------------------------
// The lobby before the game: who's in, who's ready.

/// A player as the lobby screens list him.
#[derive(Clone, Debug)]
pub struct LobbyPlayer {
    pub id: u8,
    pub name: String,
    pub ready: bool,
    pub ping: f32,
    pub host: bool,
    pub me: bool,
}

/// Everyone in the room, the host first.
pub fn lobby_players() -> Vec<LobbyPlayer> {
    with(|n| {
        let mut v = Vec::new();
        if n.role == Role::Host {
            v.push(LobbyPlayer { id: 0, name: n.look.name.clone(), ready: true, ping: 0.0, host: true, me: true });
        }
        for r in n.remotes.iter().filter(|r| r.hello) {
            v.push(LobbyPlayer { id: r.id, name: r.look.name.clone(), ready: r.ready || r.id == 0, ping: r.ping, host: r.id == 0, me: false });
        }
        if n.role == Role::Client && n.my_id != 255 {
            v.push(LobbyPlayer { id: n.my_id, name: n.look.name.clone(), ready: n.ready, ping: 0.0, host: false, me: true });
        }
        v.sort_by_key(|p| (!p.host, p.id));
        v
    })
    .unwrap_or_default()
}

/// Joiner: ready (or not) to start.
pub fn set_ready(on: bool) {
    with(|n| {
        if n.role == Role::Client {
            n.ready = on;
            send_host(n, &Msg::Ready(on), true);
        }
    });
}

pub fn is_ready() -> bool {
    with(|n| n.ready).unwrap_or(false)
}

/// Host: every player in the lobby is ready (true with nobody else in).
pub fn all_ready() -> bool {
    with(|n| n.remotes.iter().filter(|r| r.hello && r.waiting).all(|r| r.ready)).unwrap_or(true)
}

/// Host: the game starts - players in the lobby come in once a map is up (and anyone joining later drops in).
pub fn start_game() {
    with(|n| n.started = true);
}

pub fn started() -> bool {
    with(|n| n.started).unwrap_or(false)
}

/// Joiner: connected to the host and waiting in his lobby (not in his game yet).
pub fn in_lobby() -> bool {
    with(|n| n.role == Role::Client && n.my_id != 255 && !n.welcomed).unwrap_or(false)
}

/// Host: send a player away.
pub fn kick(id: u8) {
    with(|n| {
        if n.role == Role::Host {
            send_to(n, id, &Msg::Refuse("The host removed you from the game.".into()), true);
            let _ = n.tx.send(Out::Kick(id, "kicked".into()));
        }
    });
}

// ---------------------------------------------------------------------------------------------------------------
// News and chat at the top of the screen.

const NEWS: [u8; 3] = [200, 230, 255];
const CHAT: [u8; 3] = [255, 255, 255];
const DEATH: [u8; 3] = [255, 120, 100];
/// Seconds a line stays up.
const FEED_SECS: f32 = 7.0;
const FEED_LINES: usize = 5;

static FEED: Mutex<Vec<(Instant, String, [u8; 3])>> = Mutex::new(Vec::new());

struct Chat {
    open: bool,
    text: String,
    blink: Option<Instant>,
}
static CHATBOX: Mutex<Chat> = Mutex::new(Chat { open: false, text: String::new(), blink: None });

/// A line of news for the top of the screen (and the log).
pub fn feed(line: &str, colour: [u8; 3]) {
    if let Ok(mut f) = FEED.lock() {
        f.push((Instant::now(), line.to_string(), colour));
        if f.len() > 32 {
            f.remove(0);
        }
    }
}

/// Host only: a news line for everyone in the room (alarms and the like).
pub fn host_news(line: &str) {
    let sent = with(|n| {
        if n.role == Role::Host {
            send_all(n, &Msg::Note(line.to_string()), true, None);
            true
        } else {
            false
        }
    })
    .unwrap_or(false);
    if sent {
        feed(line, NEWS);
    }
}

fn clean_chat(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(120).collect::<String>().trim().to_string()
}

/// What the HUD's Net window shows: the latest news, and the chat line while typing.
pub fn hud_lines() -> Vec<(String, [u8; 3])> {
    if !active() {
        return Vec::new();
    }
    if list_held() {
        let mut out = vec![(format!("PLAYERS ({})", player_list().len()), [255, 242, 140])];
        for (name, ping, alive, host) in player_list() {
            let what = if alive { "" } else { "  - down" };
            let who = if host { "  (host)" } else { "" };
            let col = if alive { [220, 230, 240] } else { [255, 120, 100] };
            out.push((format!("{name}{who}   {ping:.0} ms{what}"), col));
        }
        return out;
    }
    let mut out: Vec<(String, [u8; 3])> = FEED
        .lock()
        .map(|f| {
            let n = f.len();
            f.iter()
                .skip(n.saturating_sub(FEED_LINES))
                .filter(|(t, ..)| t.elapsed().as_secs_f32() < FEED_SECS)
                .map(|(_, l, c)| (l.clone(), *c))
                .collect()
        })
        .unwrap_or_default();
    if let Ok(c) = CHATBOX.lock() {
        if c.open {
            let caret = c.blink.is_none_or(|t| t.elapsed().as_millis() % 1000 < 500);
            out.push((format!("Say: {}{}", c.text, if caret { "_" } else { " " }), [255, 242, 140]));
        }
    }
    out
}

/// The chat line is being typed (the game sees no keys meanwhile).
pub fn chat_open() -> bool {
    CHATBOX.lock().map(|c| c.open).unwrap_or(false)
}

/// Keys typed this frame while chatting (keybinds hands them over): letters go in, Enter sends, Escape cancels.
pub fn chat_keys(typed: &[(KeyCode, Option<String>)]) {
    let mut send = None;
    if let Ok(mut c) = CHATBOX.lock() {
        for (k, t) in typed {
            match k {
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    send = Some(std::mem::take(&mut c.text));
                    c.open = false;
                }
                KeyCode::Escape => {
                    c.text.clear();
                    c.open = false;
                }
                KeyCode::Backspace => {
                    c.text.pop();
                }
                _ => {
                    if let Some(t) = t {
                        for ch in t.chars().filter(|ch| !ch.is_control()) {
                            if c.text.chars().count() < 120 {
                                c.text.push(ch);
                            }
                        }
                    }
                }
            }
        }
        c.blink = Some(c.blink.unwrap_or_else(Instant::now));
    }
    if let Some(text) = send {
        say(&text);
    }
}

/// Say something to everyone.
pub fn say(text: &str) {
    let text = clean_chat(text);
    if text.is_empty() {
        return;
    }
    with(|n| {
        feed(&format!("{}: {text}", n.look.name), CHAT);
        match n.role {
            Role::Host => send_all(n, &Msg::Chat { from: 0, text: text.clone() }, true, None),
            Role::Client => send_host(n, &Msg::Chat { from: n.my_id, text: text.clone() }, true),
            Role::Off => {}
        }
    });
}

/// The chat key (Y by default, "Chat (multiplayer)") opens the chat line in play.
pub fn chat_system(keys: Res<ButtonInput<KeyCode>>) {
    if !active() || crate::keybinds::UI_MODE.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    if keys.just_pressed(KeyCode::KeyY) && !chat_open() {
        if let Ok(mut c) = CHATBOX.lock() {
            c.open = true;
            c.text.clear();
            c.blink = Some(Instant::now());
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The shared world: decorations, items dropped in the world, bodies' loot, laser beams, projectiles in flight.
// The host decides; joiners ask and show. (Placed pickups stay each player's own copy.)

#[derive(Default)]
struct Sync {
    /// The map this state is for (map_hash), and the loader generation it was made in.
    map: u32,
    gen: u64,
    // In (both sides): positions to put decorations at, decorations to break, items to spawn ((id, item); host:
    // id 0 = a joiner's drop to give an id), items gone, items granted to us, loot, laser states, projectiles.
    deco_moves: Vec<DecoNet>,
    deco_gone: Vec<u32>,
    item_spawns: Vec<(u32, ItemNet)>,
    items_gone: Vec<u32>,
    granted: Vec<ItemNet>,
    loot: Vec<(u32, Vec<(String, i32)>, Option<(String, String)>)>,
    lasers: Option<Vec<u32>>,
    ghosts: Vec<GhostNet>,
    // Host in: joiners' hits on decorations, take requests, searches.
    deco_hits: Vec<(u32, f32, String)>,
    takes: Vec<(u8, u32)>,
    searches: Vec<(u8, u32)>,
    // Host bookkeeping: what the joiners were last told, the items lying about, the broken decorations, which
    // joiner has had the whole picture for this map.
    sent_decos: HashMap<u32, DecoNet>,
    broken: Vec<u32>,
    new_broken: Vec<u32>,
    items: HashMap<u32, ItemNet>,
    next_item: u32,
    lasers_on: Vec<u32>,
    synced: HashMap<u8, u32>,
    // Joiner bookkeeping: where the host last put each decoration.
    applied: HashMap<u32, DecoNet>,
    deco_t: f32,
    /// Decorations this player pushed / carried / threw lately (blocker -> when): his to report (joiner), and the
    /// others' moves of them wait meanwhile (both sides).
    touched: HashMap<usize, Instant>,
    // Grenades. Host in: joiners' throws (who, class, pos, dir, wall), uses, blasts. Joiner in: spawns, states,
    // gone (id, exploded). Host bookkeeping: runtime grenades alive (for late joiners), states last sent, placed
    // grenades gone.
    gren_throws: Vec<(u8, String, DVec3, DVec3, Option<DVec3>)>,
    gren_frobs: Vec<u32>,
    remote_blasts: Vec<(DVec3, f64, String)>,
    gren_spawns: Vec<GrenNet>,
    gren_states_in: Vec<GrenState>,
    gren_gone_in: Vec<(u32, bool)>,
    gren_live: HashMap<u32, GrenNet>,
    gren_sent: HashMap<u32, GrenState>,
    gren_dead: Vec<u32>,
    gren_t: f32,
    next_gren: u32,
    // Security. Host in: joiners' blasts, computer settings, turret hits. Joiner in: the host's state. Host: last
    // state built / sent reliably, clocks.
    sec_blasts: Vec<(DVec3, f64, f32, String)>,
    sec_controls: Vec<(u8, String, f32, f32)>,
    turret_hits: Vec<(u32, f32, String)>,
    sec_in: Option<SecNet>,
    sec_last: Option<SecNet>,
    sec_rel: Option<SecNet>,
    sec_t: f32,
    sec_rel_t: f32,
    // Alarm units sounding: host (last sent), joiner (from the host).
    alarms_on: Vec<u32>,
    alarms_in: Option<Vec<u32>>,
    // Hacking: strengths in (joiner: the host's; host: joiners'), HackActions in (host), out (joiner: blocker /
    // actor, kind), the strength last known per placed device, clock.
    hacks_in: Vec<(u32, f32)>,
    hack_acts_in: Vec<(u32, u8)>,
    hack_out: Vec<(usize, u8)>,
    hack_known: HashMap<u32, f32>,
    hack_t: f32,
    /// Joiner: how placed pawns died on the host (by actor).
    pawn_fx: HashMap<u32, PawnFx>,
}

static SYNC: Mutex<Option<Sync>> = Mutex::new(None);
static HOST_HEARD: Mutex<Option<Instant>> = Mutex::new(None);
/// Seconds without a word from the host before a joiner gives up on him.
const HOST_SILENCE: f32 = 10.0;
const DECO_HZ: f32 = 10.0;
/// The kind of the last damage this player took (for the line when he goes down).
static LAST_HURT: Mutex<String> = Mutex::new(String::new());

/// Work on the shared-world state. Never call `with` (the NET lock) inside: NET may be held around this, not the
/// other way round.
fn sync<R>(f: impl FnOnce(&mut Sync) -> R) -> R {
    let mut g = SYNC.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Sync::default))
}

fn heard_host() {
    if let Ok(mut h) = HOST_HEARD.lock() {
        *h = Some(Instant::now());
    }
}

fn host_silent() -> bool {
    HOST_HEARD.lock().ok().and_then(|h| *h).is_some_and(|t| t.elapsed().as_secs_f32() > HOST_SILENCE)
}

/// The player took damage of this kind (PlayerHealth::damage): the line when he goes down says how.
pub fn note_hurt(kind: &str) {
    if active() {
        if let Ok(mut k) = LAST_HURT.lock() {
            k.clear();
            k.push_str(kind);
        }
    }
}

fn how_died(kind: &str) -> &'static str {
    match kind {
        "Shot" | "AutoShot" | "Sabot" => "shot",
        "Exploded" => "blown up",
        "Burned" | "Flamed" => "burned",
        "Shocked" | "Stunned" => "electrocuted",
        "PoisonGas" | "TearGas" | "Poison" | "PoisonEffect" | "HalonGas" => "gassed",
        "Radiation" => "irradiated",
        "Drowned" => "drowned",
        "Fell" => "killed by the fall",
        "KnockedOut" => "knocked out",
        _ => "killed",
    }
}

fn v3(v: DVec3) -> [f32; 3] {
    [v.x as f32, v.y as f32, v.z as f32]
}

fn d3(v: [f32; 3]) -> DVec3 {
    DVec3::new(v[0] as f64, v[1] as f64, v[2] as f64)
}

/// The map the shared-world state is for: a new map starts it over.
fn sync_map(lvl: &dx_level::Level) -> u32 {
    let m = map_hash(&lvl.name);
    let g = crate::loader::generation();
    sync(|s| {
        if s.gen != g || s.map != m {
            let next = s.next_item.max(1);
            let next_gren = s.next_gren;
            *s = Sync { map: m, gen: g, next_item: next, next_gren, ..Default::default() };
        }
    });
    m
}

/// Property overrides an item carries, for the wire.
pub fn net_props(extra: &[unreal_pkg::props::Property]) -> Vec<(String, u32, NetVal)> {
    use unreal_pkg::props::Value;
    extra
        .iter()
        .filter_map(|p| {
            let v = match &p.value {
                Value::Int(i) => NetVal::Int(*i),
                Value::Float(f) => NetVal::Float(*f),
                Value::Bool(b) => NetVal::Bool(*b),
                Value::Name(n) => NetVal::Name(n.clone()),
                Value::Str(t) => NetVal::Str(t.clone()),
                Value::Path(t) => NetVal::Path(t.clone()),
                _ => return None,
            };
            Some((p.name.clone(), p.index, v))
        })
        .collect()
}

/// The overrides back as properties.
pub fn game_props(extra: &[(String, u32, NetVal)]) -> Vec<unreal_pkg::props::Property> {
    use unreal_pkg::props::{Property, Value};
    extra
        .iter()
        .map(|(name, index, v)| Property {
            name: name.clone(),
            index: *index,
            value: match v {
                NetVal::Int(i) => Value::Int(*i),
                NetVal::Float(f) => Value::Float(*f),
                NetVal::Bool(b) => Value::Bool(*b),
                NetVal::Name(n) => Value::Name(n.clone()),
                NetVal::Str(t) => Value::Str(t.clone()),
                NetVal::Path(t) => Value::Path(t.clone()),
            },
        })
        .collect()
}

/// An item lying in the world that everybody shares (picked up once, by whoever is first).
#[derive(Component, Clone, Copy, Debug)]
pub struct NetItem(pub u32);

/// A joiner asked the host for this item; it's hidden until the host answers.
#[derive(Component)]
pub struct NetItemAsked;

// --- Hooks for destruct.rs / pickups.rs / security.rs / projectiles.rs ----------------------------------------

/// Joiner: his hit on placed decoration `actor` goes to the host. Returns true when the caller must not apply it.
pub fn deco_hit(actor: usize, damage: f32, kind: &str) -> bool {
    if !is_client() {
        return false;
    }
    with(|n| send_host(n, &Msg::DecoHit { actor: actor as u32, damage, kind: kind.to_string() }, true));
    true
}

/// Host: joiners' hits on decorations this frame (actor, damage, kind). Joiner: decorations the host broke
/// (actor, kind "NetBreak").
pub fn take_deco_hits() -> Vec<(usize, f32, String)> {
    sync(|s| {
        let mut v: Vec<(usize, f32, String)> = s.deco_hits.drain(..).map(|(a, d, k)| (a as usize, d, k)).collect();
        v.extend(s.deco_gone.drain(..).map(|a| (a as usize, 1.0e9, "NetBreak".to_string())));
        v
    })
}

/// Host: placed decoration `actor` broke.
pub fn deco_broken(actor: usize) {
    if is_host() {
        sync(|s| {
            s.broken.push(actor as u32);
            s.new_broken.push(actor as u32);
        });
    }
}

/// An item spawned in the world here. Host: it gets an id and every joiner sees it (returns the id). Joiner:
/// None (his drops go through `throw_item`).
pub fn item_spawned(class: &str, pos: DVec3, vel: DVec3, ammo: Option<i32>, extra: &[unreal_pkg::props::Property]) -> Option<u32> {
    if !is_host() {
        return None;
    }
    let it = ItemNet { map: sync(|s| s.map), class: class.to_string(), pos: v3(pos), vel: v3(vel), ammo, extra: net_props(extra) };
    let id = sync(|s| {
        s.next_item = s.next_item.max(1);
        let id = s.next_item;
        s.next_item += 1;
        s.items.insert(id, it.clone());
        id
    });
    with(|n| send_all(n, &Msg::ItemSpawn(id, it), true, None));
    Some(id)
}

/// Joiner: an item he drops is the host's to put in the world (and so everyone's). Returns true when the caller
/// must not spawn it here.
pub fn throw_item(class: &str, pos: DVec3, vel: DVec3, ammo: Option<i32>, extra: &[unreal_pkg::props::Property]) -> bool {
    if !is_client() {
        return false;
    }
    let it = ItemNet { map: sync(|s| s.map), class: class.to_string(), pos: v3(pos), vel: v3(vel), ammo, extra: net_props(extra) };
    with(|n| send_host(n, &Msg::ItemSpawn(0, it), true));
    true
}

/// Items to spawn here this frame: (id, item). Joiner: the host's (id > 0). Host: joiners' drops (id 0: give it
/// one with `item_spawned`).
pub fn take_item_spawns() -> Vec<(u32, ItemNet)> {
    sync(|s| std::mem::take(&mut s.item_spawns))
}

/// Items that are gone (taken) this frame.
pub fn take_items_gone() -> Vec<u32> {
    sync(|s| std::mem::take(&mut s.items_gone))
}

/// Host: somebody here (the host) took shared item `id`.
pub fn item_taken(id: u32) {
    if is_host() {
        sync(|s| {
            s.items.remove(&id);
        });
        with(|n| send_all(n, &Msg::ItemGone(id), true, None));
    }
}

/// Joiner: ask the host for shared item `id`.
pub fn ask_item(id: u32) {
    with(|n| send_host(n, &Msg::ItemTake(id), true));
}

/// Host: joiners asking for items this frame (who, id).
pub fn take_item_requests() -> Vec<(u8, u32)> {
    sync(|s| std::mem::take(&mut s.takes))
}

/// Host: give item `id` to joiner `peer` (it's gone for everyone). False when it was already gone.
pub fn grant_item(peer: u8, id: u32) -> bool {
    let it = sync(|s| s.items.remove(&id));
    with(|n| {
        send_all(n, &Msg::ItemGone(id), true, None);
        if let Some(it) = &it {
            send_to(n, peer, &Msg::ItemGranted(it.clone()), true);
        }
    });
    it.is_some()
}

/// Joiner: items the host gave us this frame.
pub fn take_granted() -> Vec<ItemNet> {
    sync(|s| std::mem::take(&mut s.granted))
}

/// Joiner: search the body of placed pawn `actor` (the host says what's on it).
pub fn search(actor: usize) {
    with(|n| send_host(n, &Msg::Search(actor as u32), true));
}

/// Host: joiners' searches this frame (who, actor).
pub fn take_searches() -> Vec<(u8, usize)> {
    sync(|s| s.searches.drain(..).map(|(p, a)| (p, a as usize)).collect())
}

/// Host: what joiner `peer` found on body `actor` (items: class, count; a nanokey: id, description).
pub fn send_loot(peer: u8, actor: usize, items: Vec<(String, i32)>, key: Option<(String, String)>) {
    with(|n| send_to(n, peer, &Msg::Loot { actor: actor as u32, items, key }, true));
}

/// Joiner: what we found on bodies (actor, items, key).
pub fn take_loot() -> Vec<(usize, Vec<(String, i32)>, Option<(String, String)>)> {
    sync(|s| s.loot.drain(..).map(|(a, i, k)| (a as usize, i, k)).collect())
}

/// Host: the lasers whose beam is on now (placed actors): the joiners are told when it changes.
pub fn lasers_on(mut on: Vec<u32>) {
    if !is_host() {
        return;
    }
    on.sort_unstable();
    let (changed, map) = sync(|s| {
        let c = s.lasers_on != on;
        if c {
            s.lasers_on = on.clone();
        }
        (c, s.map)
    });
    if changed {
        with(|n| send_all(n, &Msg::Lasers(map, on), true, None));
    }
}

/// Joiner: the host's laser beams, when they changed.
pub fn take_lasers() -> Option<Vec<u32>> {
    sync(|s| s.lasers.take())
}

/// A blocker that is another player's body here: whether he's hidden by camouflage. None = not a player.
/// Host: a pawn went down to a player (None = the host's own hand, else the attacker's blocker if it's a
/// player's body): the co-op scoreboard's count.
pub fn credit_pawn_kill(attacker: Option<usize>) {
    if !is_host() {
        return;
    }
    let who = match attacker {
        None => Some(0),
        Some(b) => with(|n| n.remotes.iter().find(|r| r.blocker == Some(b)).map(|r| r.id)).flatten(),
    };
    if let Some(id) = who {
        crate::deathmatch::host_npc_kill(id);
    }
}

pub fn player_blocker(b: usize) -> Option<bool> {
    with(|n| n.remotes.iter().find(|r| r.blocker == Some(b)).map(|r| r.st.camo)).flatten()
}

/// A projectile launched here that the others should see (rockets, darts, plasma...): host -> everyone,
/// joiner -> host (who passes it on).
pub fn ghost_out(class: &str, pos: DVec3, dir: DVec3, speed: Option<f64>) {
    if !in_game() {
        return;
    }
    let g = GhostNet { map: sync(|s| s.map), class: class.to_string(), pos: v3(pos), dir: v3(dir), speed: speed.map(|v| v as f32) };
    with(|n| match n.role {
        Role::Host => send_all(n, &Msg::Ghost(g), false, None),
        Role::Client => send_host(n, &Msg::Ghost(g), false),
        Role::Off => {}
    });
}

/// Projectiles the others launched, to fly here harmlessly: (class, pos, dir, speed).
pub fn take_ghosts() -> Vec<(String, DVec3, DVec3, Option<f64>)> {
    sync(|s| {
        let map = s.map;
        s.ghosts.drain(..).filter(|g| g.map == map).map(|g| (g.class, d3(g.pos), d3(g.dir), g.speed.map(|v| v as f64))).collect()
    })
}

// --- Pawns' ends, grenades, security devices, alarms, hacking: decided on the host, shown everywhere ------------

/// Host: placed pawn `actor` was killed or knocked out by this hit (ai.rs pawn_died).
pub fn pawn_died(actor: usize, hit: DVec3, from: DVec3, damage: f32, vel: DVec3) {
    with(|n| {
        if n.role == Role::Host && !n.remotes.is_empty() {
            send_all(n, &Msg::PawnDied { actor: actor as u32, hit: v3(hit), from: v3(from), damage, vel: v3(vel) }, true, None);
        }
    });
}

/// Host: placed pawn `actor` was blown apart (robots: `boom` = their explosion's radius).
pub fn pawn_gibbed(actor: usize, at: DVec3, r: f64, h: f64, robot: bool, boom: f64) {
    with(|n| {
        if n.role == Role::Host && !n.remotes.is_empty() {
            send_all(n, &Msg::PawnGib { actor: actor as u32, at: v3(at), r: r as f32, h: h as f32, robot, boom: boom as f32 }, true, None);
        }
    });
}

/// Joiner: what the host said about placed pawn `actor`'s end (once).
pub fn take_pawn_fx(actor: usize) -> Option<PawnFx> {
    sync(|s| if s.pawn_fx.is_empty() { None } else { s.pawn_fx.remove(&(actor as u32)) })
}

/// Hard / Realistic: everyone is down and the level is about to restart.
pub fn reset_pending() -> bool {
    with(|n| n.reset_t.is_some()).unwrap_or(false)
}

/// This player pushed, carried or threw the decoration with this blocker: for a while it's his to move (a
/// joiner reports it to the host; the others' word on it waits).
pub fn deco_touched(blocker: usize) {
    if active() {
        sync(|s| {
            s.touched.insert(blocker, Instant::now());
        });
    }
}

/// Seconds a decoration stays ours after we last touched it (a throw's flight, a push rolling out).
const TOUCH_SECS: f32 = 4.0;

fn touched_lately(s: &Sync, blocker: Option<usize>) -> bool {
    blocker.and_then(|b| s.touched.get(&b)).is_some_and(|t| t.elapsed().as_secs_f32() < TOUCH_SECS)
}

/// Joiner: a grenade he throws (or places on a wall) is the host's to run. True = don't spawn it here (it comes
/// back from the host for everyone).
pub fn grenade_throw_out(class: &str, pos: DVec3, dir: DVec3, wall: Option<DVec3>) -> bool {
    if !is_client() {
        return false;
    }
    with(|n| {
        if n.welcomed {
            send_host(n, &Msg::GrenThrow { class: class.to_string(), pos: v3(pos), dir: v3(dir), wall: wall.map(v3) }, true);
        }
    });
    true
}

/// Host: joiners' grenades to throw this frame: (class, pos, dir, thrower = his body's blocker, wall normal).
pub fn take_grenade_throws() -> Vec<(String, DVec3, DVec3, Option<usize>, Option<DVec3>)> {
    let list = sync(|s| std::mem::take(&mut s.gren_throws));
    if list.is_empty() {
        return Vec::new();
    }
    list.into_iter()
        .map(|(peer, class, pos, dir, wall)| {
            // (His body not here yet: the grenade counts as the host's.)
            let body = with(|n| n.remotes.iter().find(|r| r.id == peer).and_then(|r| r.blocker)).flatten();
            (class, pos, dir, body, wall)
        })
        .collect()
}

/// Host: a grenade came into the world here (thrown, placed): its id, and every joiner sees it. 0 = not in a game.
pub fn grenade_spawned(class: &str, pos: DVec3, vel: DVec3, wall: Option<DVec3>, fuse: f32, blast: f64) -> u32 {
    if !is_host() {
        return 0;
    }
    let g = sync(|s| {
        s.next_gren = s.next_gren.max(GREN_RUNTIME);
        let id = s.next_gren;
        s.next_gren += 1;
        let g = GrenNet { map: s.map, id, class: class.to_string(), pos: v3(pos), vel: v3(vel), wall: wall.map(v3), fuse, blast: blast as f32 };
        s.gren_live.insert(id, g.clone());
        g
    });
    let id = g.id;
    with(|n| send_all(n, &Msg::GrenSpawn(g), true, None));
    id
}

/// Joiner: grenades the host put in the world.
pub fn take_grenade_spawns() -> Vec<GrenNet> {
    sync(|s| std::mem::take(&mut s.gren_spawns))
}

/// Host: the grenades' states now (called every frame; sent ten times a second, what changed).
pub fn grenade_states_out(dt: f32, states: impl FnOnce() -> Vec<GrenState>) {
    if !is_host() {
        return;
    }
    let due = sync(|s| {
        s.gren_t += dt;
        if s.gren_t >= 0.1 {
            s.gren_t = 0.0;
            true
        } else {
            false
        }
    });
    if !due {
        return;
    }
    let now = states();
    let (map, changed) = sync(|s| {
        let mut out = Vec::new();
        for g in now {
            let differs = s.gren_sent.get(&g.id).is_none_or(|o| {
                o.flags != g.flags || o.rot != g.rot || (0..3).any(|k| (o.pos[k] - g.pos[k]).abs() > 2.0)
            });
            if differs {
                // Placed grenades are only news once they change from how the map placed them.
                if g.id >= GREN_RUNTIME || s.gren_sent.contains_key(&g.id) || g.flags & (GS_DISABLED | GS_TICKING) != 0 {
                    out.push(g.clone());
                }
                s.gren_sent.insert(g.id, g);
            }
        }
        (s.map, out)
    });
    if !changed.is_empty() {
        with(|n| send_all(n, &Msg::GrenStates(map, changed), true, None));
    }
}

/// Joiner: grenade states from the host.
pub fn take_grenade_states() -> Vec<GrenState> {
    sync(|s| std::mem::take(&mut s.gren_states_in))
}

/// Host: grenade `id` is gone (exploded, or picked up).
pub fn grenade_gone(id: u32, exploded: bool) {
    if id == 0 || !is_host() {
        return;
    }
    let map = sync(|s| {
        s.gren_live.remove(&id);
        s.gren_sent.remove(&id);
        if id < GREN_RUNTIME {
            s.gren_dead.push(id);
        }
        s.map
    });
    with(|n| send_all(n, &Msg::GrenGone { map, id, exploded }, true, None));
}

/// Joiner: grenades the host says are gone (id, exploded).
pub fn take_grenades_gone() -> Vec<(u32, bool)> {
    sync(|s| std::mem::take(&mut s.gren_gone_in))
}

/// Joiner: he used grenade `id` (the host disarms it or hands it over for everyone).
pub fn grenade_frob_out(id: u32) {
    with(|n| {
        if n.role == Role::Client && n.welcomed {
            send_host(n, &Msg::GrenFrob(id), true);
        }
    });
}

/// Host: grenades joiners used this frame.
pub fn take_grenade_frobs() -> Vec<u32> {
    sync(|s| std::mem::take(&mut s.gren_frobs))
}

/// Joiner: a blast of his own (security::blast) goes to the host - grenades, cameras, turrets and lasers there.
pub fn blast_out(at: DVec3, radius: f64, damage: f32, kind: &str) {
    with(|n| {
        if n.role == Role::Client && n.welcomed {
            send_host(n, &Msg::Blast { at: v3(at), radius: radius as f32, damage, kind: kind.to_string() }, true);
        }
    });
}

/// Host: joiners' blasts for the grenades (centre, radius, kind).
pub fn take_remote_blasts() -> Vec<(DVec3, f64, &'static str)> {
    sync(|s| s.remote_blasts.drain(..).map(|(p, r, k)| (p, r, crate::util::intern(&k))).collect())
}

/// Host: joiners' blasts for the security devices (centre, radius, damage, kind).
pub fn take_sec_blasts() -> Vec<(DVec3, f64, f32, &'static str)> {
    sync(|s| s.sec_blasts.drain(..).map(|(p, r, d, k)| (p, r, d, crate::util::intern(&k))).collect())
}

/// Host: the security devices' state (built at most ten times a second; sent when it changed, and reliably once a
/// second when it changed since the last reliable copy).
pub fn security_out(dt: f32, build: impl FnOnce() -> SecNet) {
    if !is_host() || !with(|n| n.remotes.iter().any(|r| r.hello && !r.waiting)).unwrap_or(false) {
        return;
    }
    let due = sync(|s| {
        s.sec_t += dt;
        s.sec_rel_t += dt;
        if s.sec_t >= 0.1 {
            s.sec_t = 0.0;
            true
        } else {
            false
        }
    });
    if !due {
        return;
    }
    let now = build();
    let (map, quick, sure) = sync(|s| {
        let quick = s.sec_last.as_ref() != Some(&now);
        let sure = s.sec_rel_t >= 1.0 && s.sec_rel.as_ref() != Some(&now);
        if sure {
            s.sec_rel_t = 0.0;
            s.sec_rel = Some(now.clone());
        }
        s.sec_last = Some(now.clone());
        (s.map, quick, sure)
    });
    if quick || sure {
        with(|n| send_all(n, &Msg::Security(map, now), sure, None));
    }
}

/// Joiner: the host's security devices, when they changed.
pub fn take_security() -> Option<SecNet> {
    sync(|s| s.sec_in.take())
}

/// Joiner: a security computer's setting goes to the host (security::control).
pub fn sec_control_out(kind: u8, tag: &str, a: f32, b: f32) {
    with(|n| {
        if n.role == Role::Client && n.welcomed {
            send_host(n, &Msg::SecControl { kind, tag: tag.to_string(), a, b }, kind != 1);
        }
    });
}

/// Host: joiners' security computer settings (kind, tag, a, b).
pub fn take_sec_controls() -> Vec<(u8, String, f32, f32)> {
    sync(|s| std::mem::take(&mut s.sec_controls))
}

/// Joiner: his hit on the gun of the turret whose placed base is `base`.
pub fn turret_hit_out(base: usize, damage: f32, kind: &str) {
    with(|n| {
        if n.role == Role::Client && n.welcomed {
            send_host(n, &Msg::TurretHit { base: base as u32, damage, kind: kind.to_string() }, true);
        }
    });
}

/// Host: joiners' hits on turret guns (base actor, damage, kind).
pub fn take_turret_hits() -> Vec<(usize, f32, &'static str)> {
    sync(|s| s.turret_hits.drain(..).map(|(a, d, k)| (a as usize, d, crate::util::intern(&k))).collect())
}

/// Host: the alarm units sounding now (placed actors): the joiners are told when it changes.
pub fn alarms_out(mut on: Vec<u32>) {
    if !is_host() {
        return;
    }
    on.sort_unstable();
    let (changed, map) = sync(|s| {
        let c = s.alarms_on != on;
        if c {
            s.alarms_on = on.clone();
        }
        (c, s.map)
    });
    if changed {
        with(|n| send_all(n, &Msg::Alarms(map, on), true, None));
    }
}

/// Joiner: the host's sounding alarm units, when they changed.
pub fn take_alarms() -> Option<Vec<u32>> {
    sync(|s| s.alarms_in.take())
}

/// Joiner: a HackAction on the device with this blocker (a camera / turret switched) is the host's to do.
pub fn hack_act_out(blocker: usize) {
    sync(|s| s.hack_out.push((blocker, 0)));
}

/// Joiner: an alarm unit (placed actor) silenced by his multitool is the host's to do.
pub fn alarm_hack_out(actor: usize) {
    sync(|s| s.hack_out.push((actor, 1)));
}

/// Hackable devices' strength left, both ways: the host's is everyone's (the frob label's "Bypassed"); a joiner's
/// multitool work goes to the host, with the HackActions it ends in.
pub fn device_sync_system(
    time: Res<Time>,
    level: Res<LevelRes>,
    blockers: Res<crate::actors::Blockers>,
    mut hacking: ResMut<crate::devices::Hacking>,
) {
    let role = role();
    if role == Role::Off || crate::loader::busy() || crate::menus::intro_map() || !in_game() {
        return;
    }
    let lvl = &level.0;
    let placed = crate::save::placed_count().min(lvl.actors.len());
    let blocker_of = |a: u32| blockers.list.iter().position(|c| c.actor == a as usize && c.radius > 0.0);
    let map = sync_map(lvl);
    let (ins, acts, outs, due) = sync(|s| {
        s.hack_t += time.delta_secs();
        let due = s.hack_t >= 0.25;
        if due {
            s.hack_t = 0.0;
        }
        (std::mem::take(&mut s.hacks_in), std::mem::take(&mut s.hack_acts_in), std::mem::take(&mut s.hack_out), due)
    });
    match role {
        Role::Host => {
            for (a, v) in ins {
                if let Some(b) = blocker_of(a) {
                    hacking.strength.insert(b, v as f64);
                }
            }
            for (a, k) in acts {
                match k {
                    0 => {
                        if let Some(b) = blocker_of(a) {
                            crate::security::hacked(b);
                        }
                    }
                    _ => crate::alarms::hacked(a as usize),
                }
            }
            if due {
                let changed: Vec<(u32, f32)> = sync(|s| {
                    let mut v = Vec::new();
                    for (b, st) in &hacking.strength {
                        let Some(c) = blockers.list.get(*b) else { continue };
                        if c.actor >= placed {
                            continue;
                        }
                        let a = c.actor as u32;
                        if s.hack_known.get(&a).is_none_or(|k| (*k - *st as f32).abs() > 1e-4) {
                            s.hack_known.insert(a, *st as f32);
                            v.push((a, *st as f32));
                        }
                    }
                    v
                });
                if !changed.is_empty() {
                    with(|n| send_all(n, &Msg::Hacks(map, changed), true, None));
                }
            }
        }
        Role::Client => {
            let busy = hacking.job.as_ref().map(|j| j.blocker);
            let mut known = Vec::new();
            for (a, v) in ins {
                if let Some(b) = blocker_of(a) {
                    if Some(b) != busy {
                        hacking.strength.insert(b, v as f64);
                    }
                    known.push((a, v));
                }
            }
            // What this joiner's multitool did since: to the host.
            let mut sets = Vec::new();
            sync(|s| {
                for (a, v) in known {
                    s.hack_known.insert(a, v);
                }
                if due || !outs.is_empty() {
                    for (b, st) in &hacking.strength {
                        let Some(c) = blockers.list.get(*b) else { continue };
                        if c.actor >= placed {
                            continue;
                        }
                        let a = c.actor as u32;
                        if s.hack_known.get(&a).is_none_or(|k| (*k - *st as f32).abs() > 1e-4) {
                            s.hack_known.insert(a, *st as f32);
                            sets.push((a, *st as f32));
                        }
                    }
                }
            });
            let acts: Vec<(u32, u8)> = outs
                .into_iter()
                .filter_map(|(x, k)| match k {
                    0 => blockers.list.get(x).filter(|c| c.actor < placed).map(|c| (c.actor as u32, 0)),
                    _ => (x < placed).then_some((x as u32, 1)),
                })
                .collect();
            if !sets.is_empty() || !acts.is_empty() {
                with(|n| {
                    for (a, v) in sets {
                        send_host(n, &Msg::HackSet(a, v), true);
                    }
                    for (a, k) in acts {
                        send_host(n, &Msg::HackAct(a, k), true);
                    }
                });
            }
        }
        Role::Off => {}
    }
}

/// Host: the whole picture of grenades, security devices, alarms and hacking for a joiner who just arrived.
fn send_devices_to(id: u8) {
    let (map, spawns, states, dead, sec, alarms, hacks) = sync(|s| {
        (
            s.map,
            s.gren_live.values().cloned().collect::<Vec<_>>(),
            s.gren_sent.values().filter(|g| g.id >= GREN_RUNTIME || g.flags & (GS_DISABLED | GS_TICKING) != 0).cloned().collect::<Vec<_>>(),
            s.gren_dead.clone(),
            s.sec_last.clone(),
            s.alarms_on.clone(),
            s.hack_known.iter().map(|(a, v)| (*a, *v)).collect::<Vec<_>>(),
        )
    });
    with(|n| {
        for g in spawns {
            send_to(n, id, &Msg::GrenSpawn(g), true);
        }
        if !states.is_empty() {
            send_to(n, id, &Msg::GrenStates(map, states), true);
        }
        for g in dead {
            send_to(n, id, &Msg::GrenGone { map, id: g, exploded: false }, true);
        }
        if let Some(sec) = sec {
            send_to(n, id, &Msg::Security(map, sec), true);
        }
        send_to(n, id, &Msg::Alarms(map, alarms), true);
        if !hacks.is_empty() {
            send_to(n, id, &Msg::Hacks(map, hacks), true);
        }
    });
}

// --- Decorations ------------------------------------------------------------------------------------------------

/// A decoration's place for the wire. A floating one sends its resting rotation: the rocking on the waves is each
/// machine's own (it runs on each one's clock), so it is never news.
fn deco_net(actor: usize, d: &crate::actors::Deco) -> DecoNet {
    let l = d.body.location;
    DecoNet { actor: actor as u32, pos: [l.x as f32, l.y as f32, l.z as f32], rot: if d.floating { d.orig_rot } else { d.rotation } }
}

/// Moved enough to tell the others. Something floating bobs up and down in the water by its own buoyancy on each
/// machine: only a move across the water (or a big drop) counts.
fn differs(a: &DecoNet, b: &DecoNet, floating: bool) -> bool {
    let flat = (a.pos[0] - b.pos[0]).abs() + (a.pos[1] - b.pos[1]).abs();
    let up = (a.pos[2] - b.pos[2]).abs();
    if floating {
        flat > 3.0 || up > 24.0 || a.rot[1] != b.rot[1]
    } else {
        flat + up > 0.75 || a.rot != b.rot
    }
}

fn put_deco(d: &mut crate::actors::Deco, tf: &mut Transform, blockers: &mut crate::actors::Blockers, to: &DecoNet) {
    let p = d3(to.pos);
    let far = p.distance(d.body.location) > 12.0;
    if d.floating {
        // Afloat: it keeps bobbing here; only where it floats and its heading come from the other side.
        let near = DVec3::new(p.x, p.y, d.body.location.z);
        let flat_far = (p - d.body.location).truncate().length() > 3.0;
        if flat_far || (p.z - d.body.location.z).abs() > 24.0 {
            d.body.location = if (p.z - d.body.location.z).abs() > 24.0 { p } else { near };
            d.body.velocity = DVec3::ZERO;
            if let Some(b) = d.blocker {
                if let Some(c) = blockers.list.get_mut(b) {
                    c.center = d.body.location;
                }
            }
        }
        d.orig_rot = to.rot;
        return;
    }
    d.body.location = p;
    d.body.velocity = DVec3::ZERO;
    if d.rotation != to.rot {
        d.rotation = to.rot;
        d.spun = true;
    }
    if let Some(b) = d.blocker {
        if let Some(c) = blockers.list.get_mut(b) {
            c.center = p;
        }
    }
    let draw = p + d.pre;
    tf.translation = Vec3::from(dx_level::ue_to_bevy([draw.x as f32, draw.y as f32, draw.z as f32]));
    if far {
        // Let its own physics settle and relight it where it now is.
        d.body.physics = imsim_core::body::BodyPhysics::Falling;
    }
}

/// The shared world each frame: decorations' places both ways, the host's broken-decoration and item news, and
/// the whole picture for a joiner who has just arrived on the host's map.
#[allow(clippy::too_many_arguments)]
pub fn world_sync_system(
    time: Res<Time>,
    level: Res<LevelRes>,
    player: Res<crate::Player>,
    mut blockers: ResMut<crate::actors::Blockers>,
    mut decos: Query<(&mut crate::actors::Deco, &mut Transform), Without<Npc>>,
) {
    let role = role();
    if role == Role::Off {
        return;
    }
    let _prof = crate::prof::scope("netplay::world_sync_system");
    let lvl = &level.0;
    if crate::loader::busy() || crate::menus::intro_map() {
        return;
    }
    let my_map = sync_map(lvl);
    let placed = crate::save::placed_count().min(lvl.actors.len());
    let actor_of = |d: &crate::actors::Deco, blockers: &crate::actors::Blockers| -> Option<usize> {
        let b = d.blocker?;
        let a = blockers.list.get(b)?.actor;
        (a < placed).then_some(a)
    };
    // Security cameras turn as the host's security devices say (Msg::Security), not as decorations.
    let camera = |a: usize| lvl.actors[a].class == "SecurityCamera";
    // Positions to apply (host: from joiners; joiner: from the host). What this player is moving himself (pushing,
    // carrying, a throw in flight) stays his for a moment: the other side's word on it is old news.
    let moves = sync(|s| std::mem::take(&mut s.deco_moves));
    if !moves.is_empty() {
        let want: HashMap<u32, &DecoNet> = moves.iter().map(|m| (m.actor, m)).collect();
        for (mut d, mut tf) in &mut decos {
            let Some(a) = actor_of(&d, &blockers) else { continue };
            if let Some(m) = want.get(&(a as u32)) {
                if camera(a) || sync(|s| touched_lately(s, d.blocker)) {
                    continue;
                }
                put_deco(&mut d, &mut tf, &mut blockers, m);
                if role == Role::Client {
                    // Where the host put it: not this player's own move (never sent back).
                    let now = deco_net(a, &d);
                    sync(|s| {
                        s.applied.insert(a as u32, now);
                    });
                }
            }
        }
    }
    let tick = sync(|s| {
        s.deco_t += time.delta_secs();
        if s.deco_t >= 1.0 / DECO_HZ {
            s.deco_t = 0.0;
            true
        } else {
            false
        }
    });
    if !tick {
        return;
    }
    let me = player.body.location;
    match role {
        Role::Host => {
            if !with(|n| n.remotes.iter().any(|r| !r.waiting && r.hello)).unwrap_or(false) {
                return;
            }
            // Moved since the joiners were last told (or since the map began).
            let mut moved = Vec::new();
            let mut all_moved = Vec::new();
            sync(|s| {
                for (d, _) in &decos {
                    let Some(a) = actor_of(&d, &blockers) else { continue };
                    if camera(a) {
                        continue;
                    }
                    let now = deco_net(a, &d);
                    let start = DecoNet { actor: a as u32, pos: v3(d.start), rot: d.orig_rot };
                    let last = s.sent_decos.get(&(a as u32)).unwrap_or(&start);
                    if differs(&now, last, d.floating) {
                        s.sent_decos.insert(a as u32, now.clone());
                        moved.push(now);
                    }
                }
                all_moved.extend(s.sent_decos.values().cloned());
            });
            let gone = sync(|s| std::mem::take(&mut s.new_broken));
            if !moved.is_empty() || !gone.is_empty() {
                if env_on!("DXVIEW_NETLOG") {
                    println!("NET decos out {:?} gone {gone:?}", moved.iter().map(|m| lvl.actors[m.actor as usize].name.as_str()).collect::<Vec<_>>());
                }
                with(|n| send_all(n, &Msg::Decos { map: my_map, moved, gone }, true, None));
            }
            // A joiner who has just arrived on this map gets the whole picture: moved and broken decorations, the
            // items lying about, the laser beams.
            let arrivals: Vec<u8> = with(|n| {
                n.remotes.iter().filter(|r| r.hello && !r.waiting && r.at.is_some() && r.st.map == my_map).map(|r| r.id).collect()
            })
            .unwrap_or_default();
            for id in arrivals {
                let fresh = sync(|s| {
                    if s.synced.get(&id) == Some(&my_map) {
                        None
                    } else {
                        s.synced.insert(id, my_map);
                        Some((s.broken.clone(), s.items.iter().map(|(k, v)| (*k, v.clone())).collect::<Vec<_>>(), s.lasers_on.clone()))
                    }
                });
                if let Some((broken, items, lasers)) = fresh {
                    with(|n| {
                        send_to(n, id, &Msg::Decos { map: my_map, moved: all_moved.clone(), gone: broken }, true);
                        for (k, it) in items {
                            send_to(n, id, &Msg::ItemSpawn(k, it), true);
                        }
                        send_to(n, id, &Msg::Lasers(my_map, lasers), true);
                    });
                    send_devices_to(id);
                }
            }
        }
        Role::Client => {
            if !with(|n| n.welcomed).unwrap_or(false) {
                return;
            }
            // What this player moved himself (pushed, carried, threw) goes to the host. Only what he touched lately:
            // everything else moves here only because the host says so (its own physics settling a crate the host
            // put somewhere, a barrel bobbing in the water, is not news - sent back it fought the host's).
            let mut out = Vec::new();
            sync(|s| {
                for (d, _) in &decos {
                    let Some(a) = actor_of(&d, &blockers) else { continue };
                    if d.body.location.distance(me) > 1200.0 || camera(a) || !touched_lately(s, d.blocker) {
                        continue;
                    }
                    let now = deco_net(a, &d);
                    let start = DecoNet { actor: a as u32, pos: v3(d.start), rot: d.orig_rot };
                    let last = s.applied.get(&(a as u32)).unwrap_or(&start);
                    if differs(&now, last, d.floating) {
                        s.applied.insert(a as u32, now.clone());
                        out.push(now);
                    }
                }
            });
            if !out.is_empty() {
                with(|n| {
                    for d in out {
                        if env_on!("DXVIEW_NETLOG") {
                            println!("NET decomove out {}", lvl.actors[d.actor as usize].name);
                        }
                        send_host(n, &Msg::DecoMove(my_map, d), false);
                    }
                });
            }
        }
        Role::Off => {}
    }
}

/// Items the others took, gone here too; the host hands out the items joiners asked for.
pub fn item_sync_system(mut commands: Commands, time: Res<Time>, items: Query<(Entity, &NetItem)>, added: Query<(&NetItem, &crate::pickups::PickupItem), Added<NetItem>>) {
    if role() == Role::Off {
        return;
    }
    let log = env_on!("DXVIEW_NETLOG");
    if log {
        for (ni, it) in &added {
            println!("NETITEM spawn {} {}", ni.0, it.class);
        }
    }
    let gone = take_items_gone();
    if !gone.is_empty() {
        for (e, ni) in &items {
            if gone.contains(&ni.0) {
                if log {
                    println!("NETITEM gone {}", ni.0);
                }
                commands.entity(e).despawn();
            }
        }
    }
    // Developer aid: DXVIEW_NETTAKE=<secs> - this player grabs every shared item lying about at that game time.
    if let Some(t) = env_val!("DXVIEW_NETTAKE").and_then(|v| v.parse::<f32>().ok()) {
        let now = time.elapsed_secs();
        if now >= t && now - time.delta_secs() < t {
            for (e, ni) in &items {
                println!("NETITEM take {}", ni.0);
                if is_client() {
                    ask_item(ni.0);
                    commands.entity(e).remove::<crate::pickups::PickupItem>().insert((NetItemAsked, Visibility::Hidden));
                } else {
                    item_taken(ni.0);
                    commands.entity(e).despawn();
                }
            }
        }
    }
    if is_host() {
        for (peer, id) in take_item_requests() {
            if grant_item(peer, id) {
                if let Some((e, _)) = items.iter().find(|(_, ni)| ni.0 == id) {
                    commands.entity(e).despawn();
                }
            }
        }
    }
}

// --- Name tags and the player list --------------------------------------------------------------------------------

/// Other players' bodies here that a name tag goes over: (name, head position).
pub fn tag_targets() -> Vec<(String, DVec3, bool)> {
    with(|n| {
        let coop = n.rules.coop;
        n.remotes
            .iter()
            .filter(|r| r.at.is_some() && r.body.is_some() && !r.st.dead && !r.st.camo)
            .map(|r| (r.look.name.clone(), d3(r.st.pos) + DVec3::Z * 50.0, coop))
            .collect()
    })
    .unwrap_or_default()
}

/// Everyone in the game for the player list (held key): name, ping (ms), alive, host.
pub fn player_list() -> Vec<(String, f32, bool, bool)> {
    with(|n| {
        let mut v = Vec::new();
        let me_dead = ME_DEAD.load(std::sync::atomic::Ordering::Relaxed);
        if n.role == Role::Host {
            v.push((n.look.name.clone(), 0.0, !me_dead, true));
        }
        for r in n.remotes.iter().filter(|r| r.hello) {
            v.push((r.look.name.clone(), r.ping, !r.st.dead, r.id == 0));
        }
        if n.role == Role::Client {
            let host_ping = n.remotes.iter().find(|r| r.id == 0).map(|r| r.ping).unwrap_or(0.0);
            v.push((n.look.name.clone(), host_ping, !me_dead, false));
        }
        v.sort_by_key(|p| !p.3);
        v
    })
    .unwrap_or_default()
}

static ME_DEAD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static LIST_HELD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The player list key ("Player list (multiplayer)", P by default) is held.
pub fn list_held() -> bool {
    LIST_HELD.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn player_list_key_system(keys: Res<ButtonInput<KeyCode>>) {
    let on = active() && !chat_open() && !crate::keybinds::UI_MODE.load(std::sync::atomic::Ordering::Relaxed) && keys.pressed(KeyCode::KeyP);
    LIST_HELD.store(on, std::sync::atomic::Ordering::Relaxed);
}
