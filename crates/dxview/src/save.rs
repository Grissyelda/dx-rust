//! Saved games (DeusExPlayer.SaveGame / LoadGame).
//!
//! A save is the difference between the world now and the map as it loads: the map itself is read fresh from
//! the game files and the save puts back what changed. That keeps saves small (tens of KB as RON text), makes
//! them independent of how our runtime structures are laid out, and leaves nothing to drift: everything that
//! can change in play (flags, alliances, the player and inventory, every pawn, decoration, pickup, door,
//! trigger, camera, turret, laser, terminal, alarm, fixture, lamp, the conversation logs and nanokeys) is either
//! saved or rebuilt from saved values.
//!
//! The file is split in two like the original's save directory: `global` travels with the player (flags,
//! inventory, skills, augs, logs) and `level` is the state of the map the player is on. `visited` keeps other
//! maps' level states for when the in-process level loader (v1.0) lets the player go back to them.
//!
//! Loading starts a fresh copy of the game on the saved map with `--load <file>` (like travel.rs), puts the
//! global part back before the first frame, waits for every system to build its state from the map, then
//! puts the level part back (`restoring()` holds triggers and scripts off meanwhile). Not kept, as in the
//! original after a load the moment-to-moment reactions start over: an alarm already sounding, a pawn's
//! surprise / seek timers, projectiles in flight, a conversation in progress.

use crate::actors::{Animated, Blockers, Deco, MeshActorRef};
use crate::ai::{GameFlags, Npc, NpcState, OutOfWorld, PlayerHealth, Takedown, Takedowns};
use crate::pickups::{PickupItem, PlayerInventory};
use crate::weapons::{Clip, Clips, ModState, Weapons};
use crate::{FlyCam, LevelRes, Player};
use bevy::prelude::*;
use glam::DVec3;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;
use unreal_pkg::props::{find, Property, Value};

pub const VERSION: u32 = 1;

static RESTORING: AtomicBool = AtomicBool::new(false);
/// Placed actors in the map as it loaded (runtime spawns are appended after them).
static PLACED: AtomicUsize = AtomicUsize::new(usize::MAX);
static FRAME: AtomicU32 = AtomicU32::new(0);
/// The save this copy of the game was started from (taken by the restore).
static PENDING: Mutex<Option<SaveGame>> = Mutex::new(None);
/// Save requests from the menus / quick save: (slot file, description).
static REQUESTS: Mutex<Vec<(PathBuf, String)>> = Mutex::new(Vec::new());
/// Play time before this session (from the loaded save) and when the session started.
static PLAY_BASE: Mutex<f64> = Mutex::new(0.0);
static SESSION_START: Mutex<Option<std::time::Instant>> = Mutex::new(None);
static SAVE_COUNT: AtomicU32 = AtomicU32::new(0);
/// Result of the last save, for the HUD message.
static LAST: Mutex<Option<String>> = Mutex::new(None);

static LOADED_GAME: AtomicBool = AtomicBool::new(false);
/// The pending restore is a map change: only what travels with the player (and the map's own state if he's been
/// there before) goes back; he stays where the new map put him.
static TRAVEL: AtomicBool = AtomicBool::new(false);
/// Maps left during this game, by map file: their level state for coming back (hub maps), and saved with the game.
static VISITED: Mutex<BTreeMap<String, LevelSave>> = Mutex::new(BTreeMap::new());

/// How the player arrives on a freshly built map (loader.rs).
pub enum Arrive {
    /// New game, restart, intro, menu map: nothing to put back.
    Fresh,
    /// A saved game.
    Load(Box<SaveGame>),
    /// From another map: what travels with the player (`capture_travel`).
    Travel(Box<SaveGame>),
}

/// The map has just been rebuilt in place (loader.rs): start the restore cycle over for it.
pub fn begin_level(arrive: Arrive) {
    FRAME.store(0, Ordering::Relaxed);
    PLACED.store(usize::MAX, Ordering::Relaxed);
    if let Ok(mut q) = REQUESTS.lock() {
        q.clear();
    }
    let (pending, restoring, loaded, travel) = match arrive {
        Arrive::Fresh => (None, false, false, false),
        Arrive::Load(s) => {
            if let Ok(mut v) = VISITED.lock() {
                *v = s.visited.clone();
            }
            (Some(*s), true, true, false)
        }
        Arrive::Travel(s) => (Some(*s), true, false, true),
    };
    // Ours: autosave on arriving by travel, and on a new game's first map (character creation).
    let fresh_game = NEW_GAME_ARRIVAL.swap(false, Ordering::Relaxed);
    if let Ok(mut p) = AUTO_PENDING.lock() {
        *p = if travel {
            Some("Autosave: level start".into())
        } else if fresh_game && pending.is_none() {
            Some("Autosave: new game".into())
        } else {
            None
        };
    }
    if let Ok(mut p) = PENDING.lock() {
        *p = pending;
    }
    RESTORING.store(restoring, Ordering::Relaxed);
    LOADED_GAME.store(loaded, Ordering::Relaxed);
    TRAVEL.store(travel, Ordering::Relaxed);
}

// ------------------------------------------------------------------------------------------------ autosaves

/// Ours: five rotating autosave slots (Save/AutoSave1..5.ron), the oldest rewritten next.
pub const AUTOSAVE_SLOTS: usize = 5;

/// An autosave waiting for a safe moment: why (the save's description).
static AUTO_PENDING: Mutex<Option<String>> = Mutex::new(None);

fn autosave_path(i: usize) -> PathBuf {
    dir().join(format!("AutoSave{i}.ron"))
}

/// Any autosave slot is in use (the new-game warning).
pub fn autosaves_exist() -> bool {
    (1..=AUTOSAVE_SLOTS).any(|i| autosave_path(i).exists())
}

/// The slot to write: the first empty one, else the oldest.
fn next_autosave() -> PathBuf {
    (1..=AUTOSAVE_SLOTS)
        .map(|i| autosave_path(i))
        .min_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
        .unwrap_or_else(|| autosave_path(1))
}

/// Autosave as soon as it's safe (not on the menu maps, not dead, no conversation, the level restored).
pub fn autosave(reason: &str) {
    if let Ok(mut p) = AUTO_PENDING.lock() {
        *p = Some(reason.to_string());
    }
}

/// A new game: no maps visited, play time and save count from zero.
pub fn new_game() {
    if let Ok(mut v) = VISITED.lock() {
        v.clear();
    }
    if let Ok(mut b) = PLAY_BASE.lock() {
        *b = 0.0;
    }
    if let Ok(mut st) = SESSION_START.lock() {
        *st = Some(std::time::Instant::now());
    }
    SAVE_COUNT.store(0, Ordering::Relaxed);
    NEW_GAME_ARRIVAL.store(true, Ordering::Relaxed);
}

/// new_game() was called: the next fresh map is the new character's first.
static NEW_GAME_ARRIVAL: AtomicBool = AtomicBool::new(false);

/// Before loading a save's map: its character, play time and save count become this session's.
pub fn prepare_load(s: &SaveGame) {
    crate::travel::set_session(crate::travel::NewGame {
        name: s.global.name.clone(),
        skin: s.global.skin,
        difficulty: s.global.difficulty,
        skills: s.global.skills.clone(),
        points_avail: s.global.points.0,
        points_total: s.global.points.1,
        female: s.global.female,
        retro_morpheus: s.global.retro_morpheus,
        male_interactions: s.global.male_interactions,
    });
    if let Ok(mut b) = PLAY_BASE.lock() {
        *b = s.play_time;
    }
    if let Ok(mut st) = SESSION_START.lock() {
        *st = Some(std::time::Instant::now());
    }
    SAVE_COUNT.store(s.save_count, Ordering::Relaxed);
}

/// Leaving this map for another: what travels with the player, and this map's state kept for coming back.
pub fn capture_travel(world: &mut World) -> SaveGame {
    let s = capture_inner(world, "Travel", false);
    if let Ok(mut v) = VISITED.lock() {
        v.insert(s.map.to_ascii_lowercase(), s.level.clone());
    }
    s
}

/// This copy of the game was started from a save (MissionScript.FirstFrame doesn't run again on a load).
pub fn loaded_game() -> bool {
    LOADED_GAME.load(Ordering::Relaxed)
}

/// A saved game is being put back: triggers and scripts hold off until it's done.
pub fn restoring() -> bool {
    RESTORING.load(Ordering::Relaxed)
}

// ------------------------------------------------------------------------------------------------ data

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SaveGame {
    pub version: u32,
    pub description: String,
    pub map: String,
    /// DeusExLevelInfo.MissionLocation (shown in the load screen).
    pub location: String,
    pub mission: i32,
    pub saved_unix: u64,
    /// Seconds played (Play Time).
    pub play_time: f64,
    pub save_count: u32,
    pub global: Global,
    pub level: LevelSave,
    /// Other maps' states, by map file (filled once the in-process level loader exists).
    pub visited: BTreeMap<String, LevelSave>,
}

/// What goes with the player from map to map.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Global {
    pub name: String,
    pub skin: u8,
    pub difficulty: f32,
    /// Lay D Denton's choices (travel::NewGame).
    #[serde(default)]
    pub female: bool,
    #[serde(default)]
    pub retro_morpheus: bool,
    #[serde(default)]
    pub male_interactions: bool,
    pub player: PlayerSave,
    pub inventory: imsim_core::inventory::Inventory,
    pub clips: Vec<(String, u32)>,
    pub loaded_ammo: Vec<(String, String)>,
    pub mods: Vec<(String, ModState)>,
    /// The weapon or item in hand.
    pub held: Option<String>,
    /// (skill class, level), points available / total.
    pub skills: Vec<(String, usize)>,
    pub points: (i32, i32),
    pub augs: Vec<AugSave>,
    pub flags: imsim_core::flags::FlagBase,
    pub takedowns: Vec<TakedownSave>,
    pub keys: Vec<(String, String)>,
    pub con: crate::consys::ConSave,
    /// Augmentique: outfits found / worn, weapon skins.
    #[serde(default)]
    pub augmentique: crate::augmentique::AugSave,
    /// GMDX perks owned (class names).
    #[serde(default)]
    pub perks: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PlayerSave {
    pub location: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub crouched: bool,
    pub health: [f32; 6],
    pub hp: f32,
    pub energy: f32,
    pub poison: (u32, f32, f32),
    pub drug_timer: f32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AugSave {
    pub class: String,
    pub has: bool,
    pub active: bool,
    pub level: usize,
    pub boosted: bool,
    pub hotkey: i32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TakedownSave {
    pub name: String,
    pub alliance: String,
    pub class: String,
    pub lethal: bool,
    pub by_player: bool,
    pub damage_type: String,
}

/// The state of one map.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LevelSave {
    /// Placed actors in the map when saved: a check that the save matches the map file.
    pub placed: usize,
    /// Placed mesh actors still in the world (decorations, pickups, pawns...); the rest were destroyed or taken.
    pub present: Vec<usize>,
    /// Decorations moved from where the map put them: (actor, location, rotation).
    pub moved: Vec<(usize, [f64; 3], [i32; 3])>,
    pub npcs: Vec<NpcSave>,
    /// Items lying in the world that the map didn't place (dropped, spilled from crates, from bodies).
    pub loose: Vec<LooseSave>,
    pub movers: Vec<MoverSave>,
    pub triggers: Vec<crate::triggers::TrigSave>,
    pub security: crate::security::SecuritySave,
    pub computers: Vec<crate::computers::CompSave>,
    pub alarms_disabled: Vec<usize>,
    pub fixtures: Vec<(usize, bool, Option<i32>)>,
    /// Multitool bypass strength left, by blocker.
    pub hack_strength: Vec<(usize, f64)>,
    pub lamps: Vec<usize>,
    /// Nanokeys pawns still carry: (pawn tag, KeyID, description).
    pub keys_carried: Vec<(String, String, String)>,
    /// Actors a mission script has hidden (bHidden / out of the world).
    #[serde(default)]
    pub hidden_actors: Vec<usize>,
    /// Enhancement (Casings And Blood Stay): blood splats and bullet holes on the level, and spent casings.
    #[serde(default)]
    pub decals: Vec<crate::impacts::DecalSave>,
    #[serde(default)]
    pub casings: Vec<(String, [f64; 3])>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NpcSave {
    pub actor: usize,
    pub location: [f64; 3],
    pub yaw: f64,
    pub dead: bool,
    pub knocked_out: bool,
    pub hidden: bool,
    pub searched: bool,
    pub carried: bool,
    pub in_world: bool,
    pub parts: [f32; 6],
    pub health: f32,
    pub alliances: Option<imsim_core::alliance::AllianceTable>,
    pub alliance: String,
    pub orders: Option<crate::ai::Orders>,
    pub order_tag: String,
    pub home: [f64; 3],
    pub home_yaw: f64,
    pub use_home: bool,
    /// Weapons still carried (by class) and ammo left.
    pub arsenal: Vec<String>,
    pub ammo_left: Vec<(String, i32)>,
    /// The pose a body lies in.
    pub pose: Option<Clip>,
    pub killer_alliance: String,
    pub emit_carcass: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LooseSave {
    pub class: String,
    pub location: [f64; 3],
    /// Property overrides that matter (PickupAmmoCount, AddAugs...): (name, index, value).
    pub props: Vec<(String, u32, SimpleValue)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SimpleValue {
    Int(i32),
    Float(f32),
    Bool(bool),
    Name(String),
    Str(String),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MoverSave {
    pub index: usize,
    pub location: [f64; 3],
    pub rotation: [i32; 3],
    pub key_num: u8,
    pub prev_key_num: u8,
    pub opening: bool,
    pub locked: bool,
    pub lock_strength: f64,
    pub pickable: bool,
    pub door_strength: f64,
    pub destroyed: bool,
}

fn a3(v: DVec3) -> [f64; 3] {
    [v.x, v.y, v.z]
}
fn v3(a: [f64; 3]) -> DVec3 {
    DVec3::new(a[0], a[1], a[2])
}

/// Overrides kept for loose items.
const LOOSE_PROPS: [&str; 4] = ["PickupAmmoCount", "AddAugs", "NumCopies", "Charge"];

fn simple(v: &Value) -> Option<SimpleValue> {
    Some(match v {
        Value::Int(i) => SimpleValue::Int(*i),
        Value::Byte(b) => SimpleValue::Int(*b as i32),
        Value::Float(f) => SimpleValue::Float(*f),
        Value::Bool(b) => SimpleValue::Bool(*b),
        Value::Name(n) => SimpleValue::Name(n.clone()),
        Value::Str(s) => SimpleValue::Str(s.clone()),
        _ => return None,
    })
}
fn unsimple(v: &SimpleValue) -> Value {
    match v {
        SimpleValue::Int(i) => Value::Int(*i),
        SimpleValue::Float(f) => Value::Float(*f),
        SimpleValue::Bool(b) => Value::Bool(*b),
        SimpleValue::Name(n) => Value::Name(n.clone()),
        SimpleValue::Str(s) => Value::Str(s.clone()),
    }
}

// ------------------------------------------------------------------------------------------------ files

/// The save folder beside the exe.
pub fn dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|p| p.join("Save")))
        .unwrap_or_else(|| PathBuf::from("Save"))
}

pub fn quick_path() -> PathBuf {
    dir().join("QuickSave.ron")
}

/// A new slot file name.
pub fn new_slot() -> PathBuf {
    let d = dir();
    for i in 1..10000 {
        let p = d.join(format!("Save{i:04}.ron"));
        if !p.exists() {
            return p;
        }
    }
    d.join("Save9999.ron")
}

/// What the load / save screens list.
#[derive(Clone, Debug)]
pub struct SaveInfo {
    pub path: PathBuf,
    pub description: String,
    pub location: String,
    pub saved_unix: u64,
    pub play_time: f64,
    pub save_count: u32,
    pub bytes: u64,
}

/// Every save in the folder, newest first.
pub fn list() -> Vec<SaveInfo> {
    let mut v = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir()) else { return v };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("ron") {
            continue;
        }
        let Ok(t) = std::fs::read_to_string(&p) else { continue };
        // Only the header is needed; the whole file still parses quickly (tens of KB).
        let Ok(s) = ron::from_str::<SaveGame>(&t) else { continue };
        v.push(SaveInfo {
            bytes: t.len() as u64,
            path: p,
            description: s.description,
            location: s.location,
            saved_unix: s.saved_unix,
            play_time: s.play_time,
            save_count: s.save_count,
        });
    }
    v.sort_by(|a, b| b.saved_unix.cmp(&a.saved_unix));
    v
}

/// The most recently written save of any kind (by file time; cheaper than reading every save).
pub fn latest() -> Option<PathBuf> {
    let rd = std::fs::read_dir(dir()).ok()?;
    rd.flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("ron"))
        .filter_map(|p| Some((std::fs::metadata(&p).and_then(|m| m.modified()).ok()?, p)))
        .max_by_key(|x| x.0)
        .map(|x| x.1)
}

pub fn read(path: &Path) -> Option<SaveGame> {
    let t = std::fs::read_to_string(path).ok()?;
    match ron::from_str::<SaveGame>(&t) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("save: can't read {}: {e}", path.display());
            None
        }
    }
}

pub fn delete(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(snapshot_path(path));
}

pub const SNAP_W: usize = 160;
pub const SNAP_H: usize = 120;

/// The save's snapshot file (raw 160 x 120 RGB).
pub fn snapshot_path(save: &Path) -> PathBuf {
    save.with_extension("shot")
}

/// The snapshot as RGBA, if there is one.
pub fn snapshot(save: &Path) -> Option<Vec<u8>> {
    let b = std::fs::read(snapshot_path(save)).ok()?;
    if b.len() != SNAP_W * SNAP_H * 3 {
        return None;
    }
    Some(b.chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect())
}

/// Ask for a save at the end of this frame (the menus, quick save).
pub fn request(path: PathBuf, description: &str) {
    if let Ok(mut q) = REQUESTS.lock() {
        q.push((path, description.to_string()));
    }
}

/// The message from the last save, once.
pub fn take_message() -> Option<String> {
    LAST.lock().ok().and_then(|mut m| m.take())
}

/// Start the game again from a save (a fresh copy of the exe on the saved map).
/// Load a saved game: its map is built in place (loader.rs) and the save put back on it.
pub fn load(_root: &Path, path: &Path) {
    let Some(s) = read(path) else { return };
    if s.version > VERSION {
        eprintln!("save: {} is from a newer version", path.display());
        return;
    }
    println!("save: loading {}", path.display());
    let map = s.map.clone();
    crate::loader::request(&map, crate::loader::Goal::Load(Box::new(s)));
}

/// Called from main before the app is built: the save named by `--load`, if any. Returns its map.
pub fn startup(args: &[String]) -> Option<String> {
    *SESSION_START.lock().ok()? = Some(std::time::Instant::now());
    let i = args.iter().position(|a| a == "--load")?;
    let path = PathBuf::from(args.get(i + 1)?);
    let s = read(&path)?;
    if s.version > VERSION {
        eprintln!("save: {} is from a newer version", path.display());
        return None;
    }
    crate::travel::set_session(crate::travel::NewGame {
        name: s.global.name.clone(),
        skin: s.global.skin,
        difficulty: s.global.difficulty,
        skills: s.global.skills.clone(),
        points_avail: s.global.points.0,
        points_total: s.global.points.1,
        female: s.global.female,
        retro_morpheus: s.global.retro_morpheus,
        male_interactions: s.global.male_interactions,
    });
    *PLAY_BASE.lock().ok()? = s.play_time;
    SAVE_COUNT.store(s.save_count, Ordering::Relaxed);
    let map = s.map.clone();
    *PENDING.lock().ok()? = Some(s);
    RESTORING.store(true, Ordering::Relaxed);
    LOADED_GAME.store(true, Ordering::Relaxed);
    println!("save: loading {}", path.display());
    Some(map)
}

fn play_time() -> f64 {
    let base = PLAY_BASE.lock().map(|b| *b).unwrap_or(0.0);
    let start = SESSION_START.lock().ok().and_then(|s| *s);
    base + start.map_or(0.0, |s| s.elapsed().as_secs_f64())
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// "10/20/2026 3:46 pm" in local time (TZ_OFFSET_HOURS overrides the system's offset).
pub fn format_date(unix: u64) -> String {
    let off_min: i64 = std::env::var("TZ_OFFSET_HOURS")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .map(|h| (h * 60.0) as i64)
        .unwrap_or_else(local_offset_minutes);
    let t = unix as i64 + off_min * 60;
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    // Civil from days (Howard Hinnant).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let (h, min) = (secs / 3600, (secs % 3600) / 60);
    let (h12, ap) = match h {
        0 => (12, "am"),
        1..=11 => (h, "am"),
        12 => (12, "pm"),
        _ => (h - 12, "pm"),
    };
    format!("{m}/{d}/{y} {h12}:{min:02} {ap}")
}

/// The system's UTC offset in minutes (Windows: GetTimeZoneInformation; elsewhere UTC unless TZ_OFFSET_HOURS).
#[cfg(windows)]
fn local_offset_minutes() -> i64 {
    #[repr(C)]
    struct SystemTime([u16; 8]);
    #[repr(C)]
    struct TimeZoneInformation {
        bias: i32,
        standard_name: [u16; 32],
        standard_date: SystemTime,
        standard_bias: i32,
        daylight_name: [u16; 32],
        daylight_date: SystemTime,
        daylight_bias: i32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetTimeZoneInformation(tz: *mut TimeZoneInformation) -> u32;
    }
    let mut tz: TimeZoneInformation = unsafe { std::mem::zeroed() };
    let r = unsafe { GetTimeZoneInformation(&mut tz) };
    // 2 = TIME_ZONE_ID_DAYLIGHT; UTC = local + bias.
    let bias = tz.bias + if r == 2 { tz.daylight_bias } else { tz.standard_bias };
    -(bias as i64)
}
#[cfg(not(windows))]
fn local_offset_minutes() -> i64 {
    0
}

pub fn format_play_time(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

// ------------------------------------------------------------------------------------------------ capture

fn capture(world: &mut World, description: &str) -> SaveGame {
    let mut s = capture_inner(world, description, true);
    s.visited = VISITED.lock().map(|v| v.clone()).unwrap_or_default();
    s.visited.remove(&s.map.to_ascii_lowercase());
    s
}

fn capture_inner(world: &mut World, description: &str, count_it: bool) -> SaveGame {
    let placed = PLACED.load(Ordering::Relaxed);
    let (map, location, mission) = {
        let lvl = &world.resource::<LevelRes>().0;
        let info = lvl.actors.iter().find(|a| a.class == "DeusExLevelInfo");
        let s = |n: &str| match info.and_then(|a| find(&a.props, n)) {
            Some(Value::Str(v)) => v.clone(),
            _ => String::new(),
        };
        let mission = match info.and_then(|a| find(&a.props, "missionNumber")) {
            Some(Value::Int(n)) => *n,
            _ => 0,
        };
        (lvl.name.clone(), s("MissionLocation"), mission)
    };
    let mut g = Global::default();
    let session = crate::travel::session().unwrap_or_default();
    g.name = session.name;
    g.skin = session.skin;
    g.difficulty = session.difficulty;
    g.female = session.female;
    g.retro_morpheus = session.retro_morpheus;
    g.male_interactions = session.male_interactions;
    {
        let p = world.resource::<Player>();
        g.player.location = a3(p.body.location);
        g.player.crouched = p.crouched;
    }
    {
        let mut q = world.query::<&FlyCam>();
        if let Some(c) = q.iter(world).next() {
            g.player.yaw = c.yaw;
            g.player.pitch = c.pitch;
        }
    }
    {
        let ph = world.resource::<PlayerHealth>();
        g.player.health = ph.parts;
        g.player.hp = ph.hp;
        g.player.energy = ph.energy;
        g.player.poison = (ph.poison_counter, ph.poison_timer, ph.poison_damage);
        g.player.drug_timer = ph.drug_timer;
    }
    g.inventory = world.resource::<PlayerInventory>().0.clone();
    {
        let c = world.resource::<Clips>();
        g.clips = c.0.iter().map(|(k, v)| (k.clone(), *v)).collect();
        g.clips.sort();
        let w = world.resource::<Weapons>();
        g.loaded_ammo = w.loaded_ammo.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        g.loaded_ammo.sort();
        g.mods = w.mods.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        g.mods.sort_by(|a, b| a.0.cmp(&b.0));
        g.held = w.held.as_ref().map(|h| h.stats.class.clone()).filter(|c| !c.is_empty());
        g.augmentique = crate::augmentique::save();
        g.perks = crate::perks::save();
    }
    if let Some(a) = crate::abilities::lock().as_ref() {
        g.skills = a.skills.list.iter().map(|s| (s.def.class.clone(), s.level)).collect();
        g.points = (a.skills.points_avail, a.skills.points_total);
        g.augs = a
            .augs
            .list
            .iter()
            .map(|x| AugSave { class: x.def.class.clone(), has: x.has, active: x.active, level: x.level, boosted: x.boosted, hotkey: x.hotkey })
            .collect();
    }
    g.flags = world.resource::<GameFlags>().0.clone();
    g.takedowns = world
        .resource::<Takedowns>()
        .log
        .iter()
        .map(|t| TakedownSave {
            name: t.name.clone(),
            alliance: t.alliance.clone(),
            class: t.class.clone(),
            lethal: t.lethal,
            by_player: t.by_player,
            damage_type: t.damage_type.to_string(),
        })
        .collect();
    g.keys = crate::keys::list();
    if let Some(c) = crate::consys::lock().as_ref() {
        g.con = c.save_state();
    }

    // ---- the level
    let mut l = LevelSave { placed, ..default() };
    let mesh_owner: Vec<usize> = world.resource::<LevelRes>().0.mesh_actors.iter().map(|m| m.actor).collect();
    {
        let mut q = world.query::<&MeshActorRef>();
        let mut present: Vec<usize> =
            q.iter(world).filter_map(|m| mesh_owner.get(m.0).copied()).filter(|a| *a < placed).collect();
        present.sort();
        present.dedup();
        l.present = present;
    }
    {
        let mut q = world.query_filtered::<&Deco, Without<Npc>>();
        for d in q.iter(world) {
            let Some(&a) = mesh_owner.get(d.mesh_actor) else { continue };
            if a >= placed {
                continue;
            }
            if d.body.location.distance(d.start) > 1.0 || d.rotation != d.orig_rot {
                l.moved.push((a, a3(d.body.location), d.rotation));
            }
        }
        l.moved.sort_by_key(|m| m.0);
    }
    {
        let mut q = world.query::<(&Npc, &Deco, Option<&Animated>, Option<&OutOfWorld>, &Visibility)>();
        for (n, d, an, out, vis) in q.iter(world) {
            if n.actor >= placed {
                continue;
            }
            let dead = n.state == NpcState::Dead;
            l.npcs.push(NpcSave {
                actor: n.actor,
                location: a3(d.body.location),
                yaw: n.yaw,
                dead,
                knocked_out: n.knocked_out,
                hidden: *vis == Visibility::Hidden && out.is_none(),
                searched: n.searched,
                carried: n.carried,
                in_world: out.is_none(),
                parts: n.hp_body.parts,
                health: n.hp_body.health,
                alliances: Some(n.alliances.clone()),
                alliance: n.alliance.clone(),
                orders: Some(n.orders),
                order_tag: n.order_tag.clone(),
                home: a3(n.home),
                home_yaw: n.home_yaw,
                use_home: n.use_home,
                arsenal: n.arsenal.iter().map(|w| w.class.clone()).collect(),
                ammo_left: n.ammo_left.clone(),
                pose: if dead { an.and_then(|a| a.clip.clone()) } else { None },
                killer_alliance: n.killer_alliance.clone(),
                emit_carcass: n.emit_carcass,
            });
        }
        l.npcs.sort_by_key(|n| n.actor);
    }
    {
        let mut q = world.query::<(&PickupItem, &crate::destruct::Loose)>();
        for (p, lo) in q.iter(world) {
            if p.actor.is_some() {
                continue;
            }
            let props = p
                .props
                .iter()
                .filter(|x| LOOSE_PROPS.iter().any(|n| x.name.eq_ignore_ascii_case(n)))
                .filter_map(|x| simple(&x.value).map(|v| (x.name.clone(), x.index, v)))
                .collect();
            l.loose.push(LooseSave { class: p.class.clone(), location: a3(lo.pos), props });
        }
    }
    if crate::settings::get("PersistentDebris") > 0.5 {
        let mut q = world.query::<(&crate::impacts::KeepDecal, &Transform)>();
        for (k, t) in q.iter(world) {
            l.decals.push(crate::impacts::DecalSave {
                tex: k.tex.clone(),
                pos: t.translation.to_array(),
                rot: t.rotation.to_array(),
                size: t.scale.x,
                hole: k.hole,
            });
        }
        let mut q = world.query::<(&crate::destruct::Casing, &crate::destruct::Loose)>();
        for (c, lo) in q.iter(world) {
            l.casings.push((c.0.to_string(), a3(lo.pos)));
        }
    }
    {
        let m = world.resource::<crate::movers::Movers>();
        l.movers = m
            .rt
            .iter()
            .enumerate()
            .map(|(i, r)| MoverSave {
                index: i,
                // A door caught moving is saved where it's heading (its keyframe), as the original's InterpolateEnd
                // would leave it.
                location: a3(r.location),
                rotation: r.rotation,
                key_num: r.key_num,
                prev_key_num: if r.interpolating { r.key_num } else { r.prev_key_num },
                opening: r.opening,
                locked: r.locked,
                lock_strength: r.lock_strength,
                pickable: r.pickable,
                door_strength: r.door_strength,
                destroyed: r.destroyed,
            })
            .collect();
        let interp: Vec<bool> = m.rt.iter().map(|r| r.interpolating).collect();
        let lvl = &world.resource::<LevelRes>().0;
        for (i, ms) in l.movers.iter_mut().enumerate() {
            if interp[i] {
                if let Some(info) = lvl.movers.get(i) {
                    let k = ms.key_num as usize;
                    ms.location = [
                        info.base_pos[0] as f64 + info.key_pos[k][0] as f64,
                        info.base_pos[1] as f64 + info.key_pos[k][1] as f64,
                        info.base_pos[2] as f64 + info.key_pos[k][2] as f64,
                    ];
                    for c in 0..3 {
                        ms.rotation[c] = info.base_rot[c] + info.key_rot[k][c];
                    }
                }
            }
        }
    }
    l.triggers = world.resource::<crate::triggers::Triggers>().save();
    l.hidden_actors = crate::scriptworld::save();
    l.security = world.resource::<crate::security::Security>().save();
    l.computers = world.resource::<crate::computers::Computers>().save();
    l.alarms_disabled = world.resource::<crate::alarms::Alarms>().save();
    l.fixtures = world.resource::<crate::fixtures::Fixtures>().save();
    {
        let h = world.resource::<crate::devices::Hacking>();
        l.hack_strength = h.strength.iter().map(|(k, v)| (*k, *v)).collect();
        l.hack_strength.sort_by_key(|x| x.0);
    }
    l.lamps = crate::dynlight::toggled();
    l.keys_carried = crate::keys::carried();

    let count = if count_it {
        SAVE_COUNT.fetch_add(1, Ordering::Relaxed) + 1
    } else {
        SAVE_COUNT.load(Ordering::Relaxed)
    };
    SaveGame {
        version: VERSION,
        description: description.to_string(),
        map,
        location,
        mission,
        saved_unix: unix_now(),
        play_time: play_time(),
        save_count: count,
        global: g,
        level: l,
        visited: BTreeMap::new(),
    }
}

// ------------------------------------------------------------------------------------------------ restore

/// Before the first frame's game systems: the player and what travels with him.
fn restore_global(world: &mut World, s: &SaveGame) {
    let g = &s.global;
    let travel = TRAVEL.load(Ordering::Relaxed);
    if !travel {
        {
            let mut p = world.resource_mut::<Player>();
            p.body.location = v3(g.player.location);
            p.body.velocity = DVec3::ZERO;
            p.body.physics = imsim_core::movement::Physics::Falling;
            if g.player.crouched {
                p.force_duck = true;
            }
        }
        let mut q = world.query::<&mut FlyCam>();
        for mut c in q.iter_mut(world) {
            c.yaw = g.player.yaw;
            c.pitch = g.player.pitch;
        }
    }
    {
        let mut ph = world.resource_mut::<PlayerHealth>();
        ph.parts = g.player.health;
        ph.hp = g.player.hp;
        ph.energy = g.player.energy;
        (ph.poison_counter, ph.poison_timer, ph.poison_damage) = g.player.poison;
        ph.drug_timer = g.player.drug_timer;
    }
    world.resource_mut::<PlayerInventory>().0 = g.inventory.clone();
    world.resource_mut::<Clips>().0 = g.clips.iter().cloned().collect();
    {
        let mut w = world.resource_mut::<Weapons>();
        w.loaded_ammo = g.loaded_ammo.iter().cloned().collect();
        w.mods = g.mods.iter().cloned().collect();
        w.held = None;
        w.pending_class = g.held.clone();
    }
    world.resource_mut::<GameFlags>().0 = g.flags.clone();
    world.resource_mut::<Takedowns>().log = g
        .takedowns
        .iter()
        .map(|t| Takedown {
            name: t.name.clone(),
            alliance: t.alliance.clone(),
            class: t.class.clone(),
            lethal: t.lethal,
            by_player: t.by_player,
            damage_type: Box::leak(t.damage_type.clone().into_boxed_str()),
        })
        .collect();
    crate::keys::restore(&g.keys, &s.level.keys_carried);
}

/// Every system has built its state from the map.
fn all_built(world: &World) -> bool {
    crate::abilities::lock().is_some()
        && crate::consys::lock().is_some()
        && world.resource::<crate::triggers::Triggers>().is_built()
        && world.resource::<crate::security::Security>().is_built()
        && world.resource::<crate::alarms::Alarms>().is_built()
}

/// Once the map is fully built: the level state, and what lives in the systems' own state.
/// What travels with the player but needs the systems built first: skills, augs, the conversation logs, flags.
fn restore_travelling(world: &mut World, g: &Global) {
    if let Some(a) = crate::abilities::lock().as_mut() {
        for (class, lv) in &g.skills {
            if let Some(sk) = a.skills.list.iter_mut().find(|x| x.def.class.eq_ignore_ascii_case(class)) {
                sk.level = (*lv).min(3);
            }
        }
        a.skills.points_avail = g.points.0;
        a.skills.points_total = g.points.1;
        for sv in &g.augs {
            if let Some(x) = a.augs.list.iter_mut().find(|x| x.def.class.eq_ignore_ascii_case(&sv.class)) {
                x.has = sv.has;
                x.active = sv.active;
                x.level = sv.level;
                x.boosted = sv.boosted;
                x.hotkey = sv.hotkey;
            }
        }
        // Slot counts follow the installed augs.
        for loc in a.augs.locs.iter_mut() {
            loc.count = 0;
        }
        let counts: Vec<u8> = a.augs.list.iter().filter(|x| x.has && !x.def.always_active).map(|x| x.def.location).collect();
        for c in counts {
            if let Some(loc) = a.augs.locs.get_mut(c as usize) {
                loc.count += 1;
            }
        }
    }
    if let Some(c) = crate::consys::lock().as_mut() {
        c.restore_state(&g.con);
    }
    crate::sound::stop(crate::sound::PLAYER, crate::sound::slot::TALK);
    // The flags again: the first frames' scripts may have set some.
    world.resource_mut::<GameFlags>().0 = g.flags.clone();
}

fn restore_level(world: &mut World, s: &SaveGame) {
    let g = &s.global;
    restore_travelling(world, g);
    restore_level_part(world, s, &s.level);
    println!("save: restored \"{}\"", s.description);
}

/// The map's own state (a save's level part, or the state a visited map was left in).
fn restore_level_part(world: &mut World, s: &SaveGame, l: &LevelSave) {
    let g = &s.global;
    let placed = PLACED.load(Ordering::Relaxed);
    if l.placed != placed {
        eprintln!("save: the map has {placed} actors, the save expects {} (different map version?)", l.placed);
    }
    let mesh_owner: Vec<usize> = world.resource::<LevelRes>().0.mesh_actors.iter().map(|m| m.actor).collect();
    let present: std::collections::HashSet<usize> = l.present.iter().copied().collect();
    // ---- gone: destroyed decorations, taken pickups, bodies removed
    let mut gone: Vec<(Entity, Option<usize>)> = Vec::new();
    {
        let mut q = world.query::<(Entity, &MeshActorRef, Option<&Deco>)>();
        for (e, m, d) in q.iter(world) {
            let Some(&a) = mesh_owner.get(m.0) else { continue };
            if a < placed && !present.contains(&a) {
                gone.push((e, d.and_then(|d| d.blocker)));
            }
        }
    }
    for (e, b) in gone {
        if let Some(b) = b {
            let mut bl = world.resource_mut::<Blockers>();
            if let Some(c) = bl.list.get_mut(b) {
                c.radius = 0.0;
                c.height = 0.0;
            }
            bl.deco.remove(&b);
            bl.pushable_mass.remove(&b);
        }
        world.entity_mut(e).despawn();
    }
    // ---- moved decorations
    let moved: std::collections::HashMap<usize, ([f64; 3], [i32; 3])> = l.moved.iter().map(|(a, p, r)| (*a, (*p, *r))).collect();
    {
        let mut updates: Vec<(Option<usize>, DVec3)> = Vec::new();
        let mut q = world.query_filtered::<(&mut Deco, &mut Transform), Without<Npc>>();
        for (mut d, mut tf) in q.iter_mut(world) {
            let Some(&a) = mesh_owner.get(d.mesh_actor) else { continue };
            if let Some((p, r)) = moved.get(&a) {
                d.body.location = v3(*p);
                d.body.velocity = DVec3::ZERO;
                // Only what falls (PHYS_Falling decorations) settles again; a swivelling camera stays put.
                if d.falls {
                    d.body.physics = imsim_core::body::BodyPhysics::Falling;
                }
                d.rotation = *r;
                d.spun = true;
                // Drawn where it was saved now: something already at rest doesn't move again, so physics would
                // never update the transform (it stayed at the map's spot - floating chairs and computers).
                let draw = d.body.location + d.pre;
                tf.translation = Vec3::from(dx_level::ue_to_bevy([draw.x as f32, draw.y as f32, draw.z as f32]));
                updates.push((d.blocker, d.body.location));
            }
        }
        let mut bl = world.resource_mut::<Blockers>();
        for (b, p) in updates {
            if let Some(c) = b.and_then(|b| bl.list.get_mut(b)) {
                c.center = p;
            }
        }
    }
    // ---- pawns
    let by_actor: std::collections::HashMap<usize, &NpcSave> = l.npcs.iter().map(|n| (n.actor, n)).collect();
    {
        let mut blocker_updates: Vec<(usize, Option<DVec3>, bool)> = Vec::new();
        let mut enter: Vec<Entity> = Vec::new();
        let mut q = world.query::<(Entity, &mut Npc, &mut Deco, Option<&mut Animated>, &mut Visibility, Option<&OutOfWorld>)>();
        for (e, mut n, mut d, an, mut vis, out) in q.iter_mut(world) {
            let Some(sv) = by_actor.get(&n.actor) else { continue };
            d.body.location = v3(sv.location);
            d.body.velocity = DVec3::ZERO;
            n.yaw = sv.yaw;
            n.hp_body.parts = sv.parts;
            n.hp_body.health = sv.health;
            if let Some(t) = &sv.alliances {
                n.alliances = t.clone();
            }
            n.alliance = sv.alliance.clone();
            if let Some(o) = sv.orders {
                n.orders = o;
            }
            n.order_tag = sv.order_tag.clone();
            n.home = v3(sv.home);
            n.home_yaw = sv.home_yaw;
            n.use_home = sv.use_home;
            n.arsenal.retain(|w| sv.arsenal.iter().any(|c| c == &w.class));
            if n.weapon.as_ref().is_some_and(|w| !sv.arsenal.iter().any(|c| c == &w.class)) {
                n.weapon = n.arsenal.first().cloned();
            }
            n.ammo_left = sv.ammo_left.clone();
            n.searched = sv.searched;
            n.carried = sv.carried;
            n.killer_alliance = sv.killer_alliance.clone();
            n.emit_carcass = sv.emit_carcass;
            // Start over from orders: whatever it was reacting to when saved has to be seen again.
            n.target = None;
            n.seek = None;
            n.route = None;
            n.alert = None;
            n.readiness = 0.0;
            if sv.dead {
                n.state = NpcState::Dead;
                n.knocked_out = sv.knocked_out;
                n.weapon_drawn = false;
                d.body.physics = imsim_core::body::BodyPhysics::None;
                if let (Some(mut an), Some(pose)) = (an, sv.pose.clone()) {
                    an.clip = Some(pose);
                    an.tween = None;
                }
                blocker_updates.push((n.blocker, None, true));
            } else {
                if n.state == NpcState::Dead {
                    // Shouldn't happen (dead pawns don't come back), but don't leave it half dead.
                    n.state = NpcState::Idle;
                } else {
                    n.state = NpcState::Idle;
                }
                blocker_updates.push((n.blocker, Some(d.body.location), false));
            }
            if sv.hidden {
                *vis = Visibility::Hidden;
            }
            if sv.in_world && out.is_some() {
                enter.push(e);
            }
        }
        // Bodies keep the cylinder the death left (none); pawns in the world get theirs where they stand.
        let outs: Vec<(Entity, f64, f64, usize, DVec3)> = enter
            .iter()
            .filter_map(|&e| {
                let o = world.get::<OutOfWorld>(e)?;
                let n = world.get::<Npc>(e)?;
                let d = world.get::<Deco>(e)?;
                Some((e, o.radius, o.height, n.blocker, d.body.location))
            })
            .collect();
        {
            let mut bl = world.resource_mut::<Blockers>();
            for (b, p, dead) in blocker_updates {
                if let Some(c) = bl.list.get_mut(b) {
                    if dead {
                        c.radius = 0.0;
                        c.height = 0.0;
                    } else if let Some(p) = p {
                        c.center = p;
                    }
                }
            }
            for (_, r, h, b, p) in &outs {
                if let Some(c) = bl.list.get_mut(*b) {
                    c.radius = *r;
                    c.height = *h;
                    c.center = *p;
                }
            }
        }
        for (e, ..) in outs {
            world.entity_mut(e).remove::<OutOfWorld>();
            if let Some(mut v) = world.get_mut::<Visibility>(e) {
                *v = Visibility::Inherited;
            }
        }
    }
    // ---- loose items
    {
        let mut throws = world.resource_mut::<crate::destruct::ItemThrows>();
        for it in &l.loose {
            let ammo = it.props.iter().find(|p| p.0.eq_ignore_ascii_case("PickupAmmoCount")).and_then(|p| match p.2 {
                SimpleValue::Int(i) => Some(i),
                _ => None,
            });
            let extra: Vec<Property> = it
                .props
                .iter()
                .filter(|p| !p.0.eq_ignore_ascii_case("PickupAmmoCount"))
                .map(|(n, i, v)| Property { name: n.clone(), index: *i, value: unsimple(v) })
                .collect();
            throws.0.push((it.class.clone(), v3(it.location), DVec3::ZERO, ammo, extra));
        }
    }
    // ---- decals and casings (enhancement)
    crate::impacts::restore_decals(&l.decals);
    {
        let mut shells = world.resource_mut::<crate::destruct::Shells>();
        for (c, p) in &l.casings {
            if let Some(c) = crate::destruct::casing_class(c) {
                shells.0.push((c, v3(*p), DVec3::ZERO));
            }
        }
    }
    // ---- doors and lifts
    {
        let entities: Vec<Entity>;
        {
            let mut m = world.resource_mut::<crate::movers::Movers>();
            for ms in &l.movers {
                let Some(r) = m.rt.get_mut(ms.index) else { continue };
                r.location = v3(ms.location);
                r.rotation = ms.rotation;
                r.old_pos = r.location;
                r.old_rot = r.rotation;
                r.key_num = ms.key_num;
                r.prev_key_num = ms.prev_key_num;
                r.interpolating = false;
                r.alpha = 0.0;
                r.close_timer = None;
                r.opening = ms.opening;
                r.locked = ms.locked;
                r.lock_strength = ms.lock_strength;
                r.pickable = ms.pickable;
                r.door_strength = ms.door_strength;
                r.destroyed = ms.destroyed;
                r.moved_since_lit = true;
            }
            entities = m.entities.clone();
        }
        let rts: Vec<(DVec3, [i32; 3])> = world.resource::<crate::movers::Movers>().rt.iter().map(|r| (r.location, r.rotation)).collect();
        for (i, e) in entities.iter().enumerate() {
            let Some((loc, rot)) = rts.get(i) else { continue };
            if let Some(mut tf) = world.get_mut::<Transform>(*e) {
                tf.translation = Vec3::from(dx_level::ue_to_bevy([loc.x as f32, loc.y as f32, loc.z as f32]));
                tf.rotation = crate::movers::bevy_rotation(*rot);
            }
        }
    }
    world.resource_mut::<crate::triggers::Triggers>().restore(&l.triggers);
    crate::scriptworld::restore(&l.hidden_actors);
    world.resource_mut::<crate::security::Security>().restore(&l.security);
    world.resource_mut::<crate::computers::Computers>().restore(&l.computers);
    world.resource_mut::<crate::alarms::Alarms>().restore(&l.alarms_disabled);
    world.resource_mut::<crate::fixtures::Fixtures>().restore(&l.fixtures);
    world.resource_mut::<crate::devices::Hacking>().strength = l.hack_strength.iter().copied().collect();
    crate::dynlight::set_toggled(&l.lamps);
    {
        let mut hud = world.resource_mut::<crate::movers::HudMessage>();
        hud.text = String::new();
        hud.time_left = 0.0;
    }
    println!("save: restored \"{}\" ({} pawns, {} placed actors present, {} moved, {} loose items)", s.description, l.npcs.len(), l.present.len(), l.moved.len(), l.loose.len());
}

// ------------------------------------------------------------------------------------------------ systems

/// First schedule: count frames, record the map's placed actors, put the global part of a save back before the
/// game runs its first frame.
pub fn save_first_system(world: &mut World) {
    let f = FRAME.fetch_add(1, Ordering::Relaxed);
    if f == 0 {
        PLACED.store(world.resource::<LevelRes>().0.actors.len(), Ordering::Relaxed);
        let s = PENDING.lock().ok().and_then(|p| p.clone());
        if let Some(s) = s {
            restore_global(world, &s);
        }
    }
}

/// Last schedule: finish a load once every system is built; write requested saves.
pub fn save_last_system(world: &mut World) {
    let f = FRAME.load(Ordering::Relaxed);
    if restoring() && f >= 2 && (all_built(world) || f > 60) {
        let s = PENDING.lock().ok().and_then(|mut p| p.take());
        if let Some(s) = s {
            if TRAVEL.swap(false, Ordering::Relaxed) {
                restore_travelling(world, &s.global);
                // Back on a map left earlier: it is as the player left it.
                let map = world.resource::<LevelRes>().0.name.to_ascii_lowercase();
                let back = VISITED.lock().ok().and_then(|v| v.get(&map).cloned());
                if let Some(l) = back {
                    restore_level_part(world, &s, &l);
                    println!("save: {map} restored as it was left");
                }
            } else {
                restore_level(world, &s);
            }
        }
        RESTORING.store(false, Ordering::Relaxed);
    }
    // Developer aid: DXVIEW_SAVETEST=<seconds> saves to Save/SaveTest.ron after that long (with its snapshot) and
    // exits two seconds later.
    if let Some(secs) = std::env::var("DXVIEW_SAVETEST").ok().and_then(|v| v.parse::<f64>().ok()) {
        static ASKED: AtomicBool = AtomicBool::new(false);
        let elapsed = SESSION_START.lock().ok().and_then(|s| *s).map_or(0.0, |s| s.elapsed().as_secs_f64());
        if elapsed > secs && !restoring() && !ASKED.swap(true, Ordering::Relaxed) {
            request(dir().join("SaveTest.ron"), "Save test");
        }
        if elapsed > secs + 4.0 {
            println!("SAVETEST done");
            std::process::exit(0);
        }
    }
    // A pending autosave, once it's safe: the level restored and settled (a few frames in), a playable map, the
    // player alive, no conversation or infolink running, no menu up.
    if !restoring()
        && f > 10
        && !crate::menus::intro_map()
        && !crate::menus::open()
        && !crate::consys::third_person()
        && !crate::consys::datalink_playing()
        && !world.get_resource::<crate::ai::PlayerHealth>().is_some_and(|p| p.dead)
    {
        if let Some(desc) = AUTO_PENDING.lock().ok().and_then(|mut p| p.take()) {
            request(next_autosave(), &desc);
        }
    }
    let reqs: Vec<(PathBuf, String)> = REQUESTS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    for (path, desc) in reqs {
        if restoring() {
            continue;
        }
        let s = capture(world, &desc);
        let msg = match write(&path, &s) {
            Ok(()) => {
                println!("save: wrote {} ({})", path.display(), desc);
                // The snapshot for the load screen (160 x 120, raw RGB beside the save).
                let shot = snapshot_path(&path);
                world
                    .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                    .observe(move |c: On<bevy::render::view::screenshot::ScreenshotCaptured>| {
                        if let Ok(img) = c.image.clone().try_into_dynamic() {
                            let rgb = img.to_rgb8();
                            let (fw, h) = (rgb.width() as usize, rgb.height() as usize);
                            // The middle 4:3 of the screen.
                            let w = fw.min(h * 4 / 3);
                            let ox = (fw - w) / 2;
                            let raw = rgb.as_raw();
                            let mut out = Vec::with_capacity(SNAP_W * SNAP_H * 3);
                            for y in 0..SNAP_H {
                                for x in 0..SNAP_W {
                                    // Box filter over the source pixels this one covers.
                                    let (x0, x1) = (x * w / SNAP_W, ((x + 1) * w / SNAP_W).max(x * w / SNAP_W + 1));
                                    let (y0, y1) = (y * h / SNAP_H, ((y + 1) * h / SNAP_H).max(y * h / SNAP_H + 1));
                                    let mut acc = [0u32; 3];
                                    let mut n = 0u32;
                                    for sy in (y0..y1.min(h)).step_by(2) {
                                        for sx in (x0..x1.min(w)).step_by(2) {
                                            let i = (sy * fw + ox + sx) * 3;
                                            for k in 0..3 {
                                                acc[k] += raw[i + k] as u32;
                                            }
                                            n += 1;
                                        }
                                    }
                                    for k in 0..3 {
                                        out.push((acc[k] / n.max(1)) as u8);
                                    }
                                }
                            }
                            let _ = std::fs::write(&shot, out);
                        }
                    });
                format!("Game saved: {desc}")
            }
            Err(e) => {
                eprintln!("save: can't write {}: {e}", path.display());
                format!("Couldn't save the game: {e}")
            }
        };
        if let Ok(mut m) = LAST.lock() {
            *m = Some(msg.clone());
        }
        let mut hud = world.resource_mut::<crate::movers::HudMessage>();
        hud.text = msg;
        hud.time_left = 3.0;
    }
}

fn write(path: &Path, s: &SaveGame) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let text = ron::ser::to_string_pretty(s, ron::ser::PrettyConfig::default().depth_limit(4))
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    // Write beside and rename, so a crash mid-save never leaves half a file.
    let tmp = path.with_extension("ron.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// DeusExPlayer.QuickSave: not on the logo map, while dead, or during an infolink.
pub fn can_save(dead: bool) -> bool {
    !crate::menus::intro_map() && !dead && !restoring() && crate::consys::lock().as_ref().map_or(true, |c| c.link.is_none())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_format_like_the_load_screen() {
        // 2026-10-20 19:46 UTC
        std::env::set_var("TZ_OFFSET_HOURS", "-4");
        assert_eq!(format_date(1792525560), "10/20/2026 3:46 pm");
        assert_eq!(format_play_time(3725.0), "1:02:05");
    }

    #[test]
    fn save_round_trips_through_ron() {
        let mut s = SaveGame { version: VERSION, description: "Test".into(), map: "01_NYC_UNATCOIsland.dx".into(), ..default() };
        s.global.flags.set_bool("M01PlayerAggressive", true);
        s.global.inventory.credits = 123;
        s.level.npcs.push(NpcSave { actor: 7, dead: true, pose: Some(Clip { start: 3, frames: 9, rate: 15.0, looping: false, t: 2.0 }), ..default() });
        s.level.loose.push(LooseSave { class: "WeaponPistol".into(), location: [1.0, 2.0, 3.0], props: vec![("PickupAmmoCount".into(), 0, SimpleValue::Int(6))] });
        let t = ron::ser::to_string_pretty(&s, ron::ser::PrettyConfig::default()).unwrap();
        let b: SaveGame = ron::from_str(&t).unwrap();
        assert!(b.global.flags.get_bool("M01PlayerAggressive"));
        assert_eq!(b.global.inventory.credits, 123);
        assert_eq!(b.level.npcs[0].pose.as_ref().unwrap().frames, 9);
        assert_eq!(b.level.loose[0].class, "WeaponPistol");
    }
}
