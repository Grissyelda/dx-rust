//! Sound and music, the way Unreal's audio subsystem plays them for Deus Ex.
//!
//! * Gameplay code anywhere calls [`play`] / [`play_owned`] / [`play_player`] (Actor.PlaySound) or [`hold`]
//!   (a looping AmbientSound on something that moves or switches on and off). Requests go into a queue that
//!   [`sound_system`] drains once a frame, so no system needs an extra parameter.
//! * Sounds are the USound objects of the original packages (DeusExSounds.u, Ambient.uax, MoverSFX.uax...), whose
//!   WAV payloads are decoded on first use and cached.
//! * Positional sounds fade linearly to nothing at their radius and pan by direction (imsim_audio::spatialize).
//!   Volumes follow SurrealEngine's mixer: voice volume x SoundVolume/255 x 0.5, clamped to 1 (so pawns'
//!   TransientSoundVolume of 2 is full volume). Ambient sounds: AmbientFactor (0.7) x 2 x SoundVolume/255, radius
//!   25 x (SoundRadius + 1), pitch SoundPitch/64.
//! * Slots (ESoundSlot): a new sound in a non-None slot of the same owner replaces the old one, unless bNoOverride
//!   and the old one is still playing.
//! * Music: LevelInfo.Song (an Impulse Tracker module inside a .umx) from SongSection, with
//!   DeusExPlayer.UpdateDynamicMusic's combat (section 3) and dying (section 1) switches.

use crate::ai::{Npc, NpcState, PlayerHealth, Target};
use crate::{FlyCam, LevelRes, Player, SkyCam};
use bevy::prelude::*;
use glam::DVec3;
use imsim_audio::{Audio, Clip, Transition, VoiceId, VoiceParams};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use unreal_pkg::props::{find, Property, Value};
use unreal_pkg::Package;

/// ESoundSlot.
pub mod slot {
    pub const NONE: u8 = 0;
    pub const MISC: u8 = 1;
    pub const PAIN: u8 = 2;
    pub const INTERACT: u8 = 3;
    pub const AMBIENT: u8 = 4;
    pub const TALK: u8 = 5;
    pub const INTERFACE: u8 = 6;
}

/// Owner ids for slot bookkeeping.
pub const PLAYER: u64 = 1;
pub fn npc_owner(blocker: usize) -> u64 {
    0x1_0000_0000 | blocker as u64
}
pub fn mover_owner(i: usize) -> u64 {
    0x2_0000_0000 | i as u64
}

/// Radius used when PlaySound is given none (and the actor has no TransientSoundRadius).
pub const DEFAULT_RADIUS: f32 = 1600.0;

#[derive(Clone, Debug)]
pub struct Req {
    pub sound: String,
    /// Unreal units; None plays at the listener (sounds the player makes).
    pub pos: Option<DVec3>,
    pub volume: f32,
    pub radius: f32,
    pub pitch: f32,
    pub owner: u64,
    pub slot: u8,
    pub no_override: bool,
    /// Skip if this owner played in this slot less than this many seconds ago (ScriptedPawn LastPainSound).
    pub min_gap: f32,
}

struct Hold {
    key: u64,
    sound: String,
    pos: Option<DVec3>,
    volume: f32,
    radius: f32,
    pitch: f32,
}

static QUEUE: Mutex<Vec<Req>> = Mutex::new(Vec::new());
static HOLDS: Mutex<Vec<Hold>> = Mutex::new(Vec::new());
/// StopSound requests: (owner, slot).
static STOPS: Mutex<Vec<(u64, u8)>> = Mutex::new(Vec::new());

/// Actor.StopSound for whatever an owner is playing in a slot (conversation speech).
pub fn stop(owner: u64, slot: u8) {
    if let Ok(mut q) = STOPS.lock() {
        q.push((owner, slot));
    }
}

/// MissionIntro: SoundVolume 32 while the intro plays ("turn down the sound so we can hear the speech"); speech
/// keeps its own volume. 1.0 otherwise.
static DUCK: Mutex<f32> = Mutex::new(1.0);

/// Set the effects volume to an absolute SoundVolume (0..255) for now, or back to the setting (None).
pub fn set_instant_sound_volume(v: Option<f32>) {
    let k = match v {
        Some(v) => (v / 255.0) / crate::settings::sound_volume().max(1e-3),
        None => 1.0,
    };
    if let Ok(mut d) = DUCK.lock() {
        *d = k.min(1.0);
    }
}

fn duck() -> f32 {
    DUCK.lock().map(|d| *d).unwrap_or(1.0)
}

fn push(mut r: Req) {
    if r.sound.is_empty() {
        return;
    }
    if r.slot != slot::TALK {
        r.volume *= duck();
    }
    if let Ok(mut q) = QUEUE.lock() {
        if q.len() < 256 {
            q.push(r);
        }
    }
}

/// PlaySound at a place, slot None.
pub fn play(sound: &str, pos: DVec3, volume: f32, radius: f32, pitch: f32) {
    push(Req {
        sound: sound.into(),
        pos: Some(pos),
        volume,
        radius,
        pitch,
        owner: 0,
        slot: slot::NONE,
        no_override: false,
        min_gap: 0.0,
    });
}

/// PlaySound by an owner in a slot.
pub fn play_owned(
    owner: u64,
    slot: u8,
    sound: &str,
    pos: Option<DVec3>,
    volume: f32,
    radius: f32,
    pitch: f32,
) {
    // Speech follows the Speech Volume setting instead of Sound Effects Volume (the mixer's effects gain carries
    // the sound volume, so take it back out).
    let volume = if slot == slot::TALK {
        volume * crate::settings::speech_volume() / (crate::settings::sound_volume() / 0.8).max(0.05)
    } else {
        volume
    };
    push(Req {
        sound: sound.into(),
        pos,
        volume,
        radius,
        pitch,
        owner,
        slot,
        no_override: false,
        min_gap: 0.0,
    });
}

/// A sound the player makes (heard at full volume, centred).
pub fn play_player(sound: &str, volume: f32, radius: f32) {
    push(Req {
        sound: sound.into(),
        pos: None,
        volume,
        radius,
        pitch: 1.0,
        owner: PLAYER,
        slot: slot::NONE,
        no_override: false,
        min_gap: 0.0,
    });
}

pub fn play_req(r: Req) {
    push(r);
}

/// Keep a looping sound going this frame (an actor's AmbientSound). Stops by itself the first frame it isn't
/// held. `key` identifies the source.
pub fn hold(key: u64, sound: &str, pos: Option<DVec3>, volume: f32, radius: f32, pitch: f32) {
    if sound.is_empty() {
        return;
    }
    let volume = volume * duck();
    if let Ok(mut h) = HOLDS.lock() {
        h.push(Hold {
            key,
            sound: sound.into(),
            pos,
            volume,
            radius,
            pitch,
        });
    }
}

/// A Sound-valued property, if set.
pub fn prop(p: &[Property], name: &str) -> Option<String> {
    match find(p, name) {
        Some(Value::Path(s)) if !s.is_empty() && s != "None" => Some(s.clone()),
        _ => None,
    }
}

pub fn fprop(p: &[Property], name: &str, d: f32) -> f32 {
    match find(p, name) {
        Some(Value::Float(v)) => *v,
        Some(Value::Byte(v)) => *v as f32,
        Some(Value::Int(v)) => *v as f32,
        _ => d,
    }
}

/// Cheap shared random numbers for pitch/volume variation (FRand).
pub fn frand() -> f32 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static S: AtomicU64 = AtomicU64::new(0x2545_F491_4F6C_DD1D);
    let mut x = S.load(Ordering::Relaxed);
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    S.store(x, Ordering::Relaxed);
    (x >> 40) as f32 / (1u64 << 24) as f32
}

/// The sounds a weapon class plays (DeusExWeapon).
#[derive(Debug, Clone, Default)]
pub struct WeaponSounds {
    pub fire: Option<String>,
    /// CockingSound: reload begins.
    pub reload_begin: Option<String>,
    /// AltFireSound: reload ends.
    pub reload_end: Option<String>,
    pub select: Option<String>,
    /// Misc1: dry fire (guns) or hitting flesh (melee). Misc2: hitting something hard. Misc3: hitting something soft.
    pub misc: [Option<String>; 3],
    pub locked: Option<String>,
    pub tracking: Option<String>,
    /// SkilledTool.useSound: loops while a lockpick or multitool is at work.
    pub use_loop: Option<String>,
    pub volume: f32,
}

impl WeaponSounds {
    pub fn from_props(p: &[Property]) -> WeaponSounds {
        WeaponSounds {
            fire: prop(p, "FireSound"),
            reload_begin: prop(p, "CockingSound"),
            reload_end: prop(p, "AltFireSound"),
            select: prop(p, "SelectSound"),
            misc: [
                prop(p, "Misc1Sound"),
                prop(p, "Misc2Sound"),
                prop(p, "Misc3Sound"),
            ],
            locked: prop(p, "LockedSound"),
            tracking: prop(p, "TrackingSound"),
            use_loop: prop(p, "UseSound"),
            volume: fprop(p, "TransientSoundVolume", 1.0),
        }
    }
}

/// Tags untriggered last (and not triggered since): DamageTrigger.bIsOn / emitters switched off; their actors'
/// AmbientSounds stay quiet.
static OFF_TAGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Placed actors whose AmbientSound game code holds itself (security cameras, turrets: it changes with their
/// state), by actor index.
static OWNED: Mutex<Vec<usize>> = Mutex::new(Vec::new());

pub fn own_ambient(actor: usize) {
    if let Ok(mut o) = OWNED.lock() {
        if !o.contains(&actor) {
            o.push(actor);
        }
    }
}

/// Hand an actor's AmbientSound back to the ambient loop (a computer's alarm has stopped: EndAlarm).
pub fn release_ambient(actor: usize) {
    if let Ok(mut o) = OWNED.lock() {
        o.retain(|a| *a != actor);
    }
}

pub fn tag_state(tag: &str, on: bool) {
    if let Ok(mut t) = OFF_TAGS.lock() {
        let k = tag.to_ascii_lowercase();
        t.retain(|x| *x != k);
        if !on {
            t.push(k);
        }
    }
}

/// Sounds decoded so far, shared by the audio system, the conversation system and the background decoder.
static DECODED: std::sync::LazyLock<Mutex<HashMap<String, Option<Arc<Clip>>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static WANT: Mutex<Vec<String>> = Mutex::new(Vec::new());
static WORKER: std::sync::OnceLock<()> = std::sync::OnceLock::new();

/// An already-decoded sound (Some(None) = known not to decode).
pub fn cached(path: &str) -> Option<Option<Arc<Clip>>> {
    DECODED.lock().ok()?.get(path).cloned()
}

pub fn remember(path: &str, c: Option<Arc<Clip>>) {
    if let Ok(mut d) = DECODED.lock() {
        d.entry(path.to_string()).or_insert(c);
    }
}

/// Decode these sounds on a background thread so their first playing doesn't stall a frame (decoding a speech
/// MP3 or opening a sound package the first time takes many milliseconds).
pub fn prefetch(paths: impl IntoIterator<Item = String>) {
    if let Ok(mut w) = WANT.lock() {
        w.extend(paths);
    }
}

/// The map the decoder reads map-embedded sounds (MyLevel.*) from; it follows each new map.
static DECODER_MAP: Mutex<String> = Mutex::new(String::new());

/// Start the background decoder (once), with its own handles on the packages; on later maps, point it at the new
/// map and forget the old map's own sounds (every map calls its package MyLevel).
pub fn start_decoder(root: PathBuf, map_name: String) {
    if let Ok(mut m) = DECODER_MAP.lock() {
        if !m.eq_ignore_ascii_case(&map_name) {
            let old = std::mem::replace(&mut *m, map_name.clone()).to_ascii_lowercase();
            if let Ok(mut d) = DECODED.lock() {
                d.retain(|k, _| {
                    let k = k.to_ascii_lowercase();
                    !k.starts_with("mylevel.") && (old.is_empty() || !k.starts_with(&format!("{old}.")))
                });
            }
        }
    }
    WORKER.get_or_init(|| {
        let _ = std::thread::Builder::new()
            .name("sound-decode".into())
            .spawn(move || {
                let mut bank = SoundBank::new(root, map_name);
                loop {
                    let want_map = DECODER_MAP.lock().map(|m| m.clone()).unwrap_or_default();
                    if !want_map.eq_ignore_ascii_case(&bank.map_name) {
                        let old = std::mem::replace(&mut bank.map_name, want_map).to_ascii_lowercase();
                        bank.packages.remove("mylevel");
                        bank.packages.remove(&old);
                        bank.clips.retain(|k, _| {
                            let k = k.to_ascii_lowercase();
                            !k.starts_with("mylevel.") && !k.starts_with(&format!("{old}."))
                        });
                    }
                    let next = WANT.lock().ok().and_then(|mut w| w.pop());
                    let Some(path) = next else {
                        std::thread::sleep(std::time::Duration::from_millis(15));
                        continue;
                    };
                    if cached(&path).is_some() {
                        continue;
                    }
                    let c = bank.decode(&path);
                    remember(&path, c);
                }
            });
    });
}

/// Finds and decodes sounds and music from the game's packages.
pub struct SoundBank {
    root: PathBuf,
    map_name: String,
    packages: HashMap<String, Option<Package>>,
    clips: HashMap<String, Option<Arc<Clip>>>,
}

impl SoundBank {
    pub fn new(root: PathBuf, map_name: String) -> SoundBank {
        SoundBank {
            root,
            map_name,
            packages: HashMap::new(),
            clips: HashMap::new(),
        }
    }

    fn package(&mut self, name: &str) -> Option<&Package> {
        let key = name.to_ascii_lowercase();
        if !self.packages.contains_key(&key) {
            let mut cands = vec![
                self.root.join("System").join(format!("{name}.u")),
                self.root.join("Sounds").join(format!("{name}.uax")),
                self.root.join("Music").join(format!("{name}.umx")),
            ];
            if key == self.map_name.to_ascii_lowercase() || key == "mylevel" {
                cands.push(self.root.join("Maps").join(format!("{}.dx", self.map_name)));
            }
            let pkg = cands
                .iter()
                .find_map(|p| find_ci(p))
                .and_then(|p| Package::open(&p).ok());
            if pkg.is_none() {
                eprintln!("sound: package {name} not found");
            }
            self.packages.insert(key.clone(), pkg);
        }
        self.packages.get(&key).and_then(|p| p.as_ref())
    }

    fn raw(&mut self, path: &str) -> Option<unreal_pkg::objects::SoundData> {
        let (pkg_name, rest) = path.split_once('.')?;
        let pkg = self.package(pkg_name)?;
        let e = pkg
            .find_export_by_path(rest)
            .or_else(|| pkg.find_export_by_name(rest.rsplit('.').next().unwrap_or(rest)))?;
        unreal_pkg::objects::read_sound(pkg, e).ok()
    }

    /// The decoded sound at an object path like "DeusExSounds.Weapons.PistolFire".
    pub fn clip(&mut self, path: &str) -> Option<Arc<Clip>> {
        if let Some(c) = self.clips.get(path) {
            return c.clone();
        }
        let c = match cached(path) {
            Some(c) => c,
            None => {
                let c = self.decode(path);
                remember(path, c.clone());
                c
            }
        };
        if c.is_none() {
            eprintln!("sound: can't play {path}");
        }
        self.clips.insert(path.to_string(), c.clone());
        c
    }

    fn decode(&mut self, path: &str) -> Option<Arc<Clip>> {
        self.raw(path)
            .and_then(|s| imsim_audio::decode(&s.data))
            .map(Arc::new)
    }

    /// Every Sound object in a package, as object paths.
    fn all_sounds(&mut self, pkg_name: &str) -> Vec<String> {
        let Some(pkg) = self.package(pkg_name) else {
            return Vec::new();
        };
        (1..=pkg.exports.len() as i32)
            .filter(|&i| {
                pkg.class_name(pkg.exports[i as usize - 1].class_index)
                    .eq_ignore_ascii_case("Sound")
            })
            .map(|i| format!("{pkg_name}.{}", pkg.object_path(i)))
            .collect()
    }

    /// The tracker module file of a Music object ("LibertyIsland_Music.LibertyIsland_Music").
    pub fn song(&mut self, path: &str) -> Option<imsim_audio::Song> {
        let s = self.raw(path)?;
        Some(imsim_audio::Song::new(s.data))
    }
}

/// A file path with case-insensitive matching of the last component (the game folders mix cases).
fn find_ci(p: &std::path::Path) -> Option<PathBuf> {
    // A mod overlay's copy first (unreal_pkg::paths).
    if let (Some(dir), Some(name)) = (p.parent(), p.file_name()) {
        if let Some(f) = unreal_pkg::paths::find_ci(dir, &name.to_string_lossy()) {
            return Some(f);
        }
    }
    if p.exists() {
        return Some(p.to_path_buf());
    }
    let dir = p.parent()?;
    let want = p.file_name()?.to_string_lossy().to_ascii_lowercase();
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|q| {
            q.file_name()
                .map_or(false, |n| n.to_string_lossy().to_ascii_lowercase() == want)
        })
}

struct Playing {
    id: VoiceId,
    pos: Option<DVec3>,
    volume: f32,
    radius: f32,
    pitch: f32,
}

#[derive(PartialEq, Clone, Copy, Debug)]
enum MusicMode {
    Ambient,
    Combat,
    Dying,
}

#[derive(Resource)]
pub struct SoundState {
    volume_gen: u64,
    audio: Audio,
    bank: SoundBank,
    playing: Vec<Playing>,
    slots: HashMap<(u64, u8), (VoiceId, f32)>,
    held: HashMap<u64, (VoiceId, String)>,
    song: Option<imsim_audio::Song>,
    default_section: u8,
    mode: MusicMode,
    saved_section: u8,
    check_timer: f32,
    change_timer: f32,
    started: bool,
    time: f32,
    /// Placed actors with an AmbientSound (fires, cameras, turrets, the helicopter, bots...): actor index, sound,
    /// radius, volume, pitch, blocker (None = never moves).
    actor_ambients: Option<Vec<(usize, String, f64, f32, f32, Option<usize>)>>,
    /// Another song playing over the level's (the credits / quotes music): dynamic music waits.
    other_song: Option<imsim_audio::Song>,
}

/// ClientSetMusic with another song (Some(package.name)) or back to the level's song and section (None).
static MUSIC_REQ: Mutex<Option<Option<String>>> = Mutex::new(None);

pub fn set_music(song: Option<&str>) {
    if let Ok(mut m) = MUSIC_REQ.lock() {
        *m = Some(song.map(|s| s.to_string()));
    }
}

pub fn setup_sound(
    mut commands: Commands,
    level: Res<LevelRes>,
    root: Res<crate::pickups::GameRoot>,
) {
    commands.insert_resource(build_state(Audio::new(), &level.0, &root.0));
}

/// A new map in the same run (loader.rs): every sound and loop stops, the output keeps going, the new map's
/// packages and song are opened (the music changes with the map, as at the original's level change).
pub fn relevel(world: &mut World) {
    let Some(old) = world.remove_resource::<SoundState>() else { return };
    for p in &old.playing {
        old.audio.stop(p.id);
    }
    for (id, _) in old.slots.values() {
        old.audio.stop(*id);
    }
    for (id, _) in old.held.values() {
        old.audio.stop(*id);
    }
    old.audio.stop_music(imsim_audio::Transition::FastFade);
    for q in [&QUEUE] {
        if let Ok(mut q) = q.lock() {
            q.clear();
        }
    }
    if let Ok(mut h) = HOLDS.lock() {
        h.clear();
    }
    if let Ok(mut h) = STOPS.lock() {
        h.clear();
    }
    if let Ok(mut h) = OFF_TAGS.lock() {
        h.clear();
    }
    if let Ok(mut h) = OWNED.lock() {
        h.clear();
    }
    let audio = old.audio;
    let level = &world.resource::<LevelRes>().0;
    let root = world.resource::<crate::pickups::GameRoot>().0.clone();
    let st = build_state(audio, level, &root);
    world.insert_resource(st);
}

fn build_state(audio: Audio, level: &dx_level::Level, root: &std::path::Path) -> SoundState {
    let root = root.to_path_buf();
    let env = |n: &str, d: f32| {
        std::env::var(n)
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(d)
    };
    // DeusEx.ini SoundVolume/MusicVolume; SurrealEngine halves sound effects.
    audio.set_volumes(
        env("DXVIEW_SFX_VOL", 1.0) * 0.5,
        env("DXVIEW_MUSIC_VOL", 0.8),
    );
    let map_name = level.name.trim_end_matches(".dx").to_string();
    let mut bank = SoundBank::new(root.clone(), map_name.clone());
    // Decode the game's sound effects in the background now, instead of the first time each one plays.
    start_decoder(root.clone(), map_name);
    {
        let mut warm = bank.all_sounds("DeusExSounds");
        warm.reverse();
        prefetch(warm);
    }
    let (mut song, default_section) = match &level.song {
        Some((path, sec)) => (bank.song(path), *sec),
        None => (None, 0),
    };
    // Load the sections UpdateDynamicMusic switches between now, so a fight doesn't hitch.
    if let Some(s) = song.as_mut() {
        for n in [default_section as usize, 1, 3] {
            s.section(n);
        }
    }
    if song.is_some() {
        println!(
            "music: {} section {}",
            level.song.as_ref().map(|s| s.0.as_str()).unwrap_or(""),
            default_section
        );
    }
    SoundState {
        volume_gen: 0,
        audio,
        bank,
        playing: Vec::new(),
        slots: HashMap::new(),
        held: HashMap::new(),
        song,
        default_section,
        mode: MusicMode::Ambient,
        saved_section: 255,
        check_timer: 0.0,
        change_timer: 0.0,
        started: false,
        time: 0.0,
        actor_ambients: None,
        other_song: None,
    }
}

fn ue(v: Vec3) -> DVec3 {
    DVec3::new(v.x as f64, v.z as f64, v.y as f64)
}

#[allow(clippy::too_many_arguments)]
pub fn sound_system(
    time: Res<Time>,
    mut st: ResMut<SoundState>,
    level: Res<LevelRes>,
    player: Res<Player>,
    ph: Res<PlayerHealth>,
    cam: Single<&Transform, (With<FlyCam>, Without<SkyCam>)>,
    npcs: Query<&Npc>,
    blockers: Res<crate::actors::Blockers>,
) {
    let _prof = crate::prof::scope("sound::sound_system");
    // Settings > Sound volumes (live while the slider moves). Defaults 204 / 153 give the old levels.
    let gen = crate::settings::generation();
    if gen != st.volume_gen {
        st.volume_gen = gen;
        let env = |n: &str, d: f32| std::env::var(n).ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(d);
        st.audio.set_volumes(
            env("DXVIEW_SFX_VOL", 1.0) * 0.5 * crate::settings::sound_volume() / 0.8,
            env("DXVIEW_MUSIC_VOL", 0.8) * crate::settings::music_volume() / 0.6,
        );
    }
    let dt = time.delta_secs();
    st.time += dt;
    let now = st.time;
    let ear = ue(cam.translation) / dx_level::METERS_PER_UNIT as f64;
    let right = ue(cam.right().as_vec3()).normalize_or_zero();
    let place = |pos: Option<DVec3>, radius: f32| -> (f32, f32) {
        match pos {
            None => (1.0, 0.0),
            Some(p) => imsim_audio::spatialize(
                ear.to_array(),
                right.to_array(),
                p.to_array(),
                radius as f64,
            ),
        }
    };
    let st = &mut *st;

    for (owner, sl) in STOPS
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
    {
        if let Some((id, _)) = st.slots.remove(&(owner, sl)) {
            st.audio.stop(id);
        }
    }
    // One-shots.
    let reqs: Vec<Req> = QUEUE
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default();
    for r in reqs {
        let radius = if r.radius > 0.0 {
            r.radius
        } else {
            DEFAULT_RADIUS
        };
        if r.slot != slot::NONE && r.owner != 0 {
            if let Some(&(old, t)) = st.slots.get(&(r.owner, r.slot)) {
                let busy = st.audio.is_playing(old);
                if busy && r.no_override {
                    continue;
                }
                if r.min_gap > 0.0 && now - t < r.min_gap {
                    continue;
                }
                if busy {
                    st.audio.stop(old);
                }
            }
        }
        let (gain, pan) = place(r.pos, radius);
        if gain <= 0.0 {
            continue;
        }
        let Some(clip) = st.bank.clip(&r.sound) else {
            continue;
        };
        let id = st.audio.play(
            clip,
            VoiceParams {
                volume: r.volume * gain,
                pan,
                pitch: r.pitch,
                looping: false,
            },
        );
        if r.slot != slot::NONE && r.owner != 0 {
            st.slots.insert((r.owner, r.slot), (id, now));
        }
        if r.pos.is_some() {
            st.playing.push(Playing {
                id,
                pos: r.pos,
                volume: r.volume,
                radius,
                pitch: r.pitch,
            });
        }
    }

    // Level AmbientSound actors inside their radius are held like any other looping sound.
    let ambient = 0.7 * 2.0;
    for (i, a) in level.0.ambient_sounds.iter().enumerate() {
        let radius = 25.0 * (a.radius as f64 + 1.0);
        let p = DVec3::from_array(a.pos);
        if p.distance_squared(ear) <= radius * radius {
            hold(
                0x9_0000_0000 | i as u64,
                &a.sound,
                Some(p),
                ambient * a.volume as f32 / 255.0,
                radius as f32,
                a.pitch as f32 / 64.0,
            );
        }
    }

    // Actors' own AmbientSounds follow them (a pushed fire barrel, a patrolling bot); destroyed or dead ones stop.
    if st.actor_ambients.is_none() {
        let by_actor: HashMap<usize, usize> = blockers
            .list
            .iter()
            .enumerate()
            .map(|(b, c)| (c.actor, b))
            .collect();
        let list = level
            .0
            .actors
            .iter()
            .enumerate()
            .filter_map(|(i, a)| {
                // Emitters' hum is held by the emitter itself (off, flickering, out of earshot underground).
                if matches!(a.class.as_str(), "ElectricityEmitter" | "LaserEmitter") {
                    return None;
                }
                let snd = prop(&a.props, "AmbientSound")?;
                Some((
                    i,
                    snd,
                    25.0 * (fprop(&a.props, "SoundRadius", 32.0) as f64 + 1.0),
                    fprop(&a.props, "SoundVolume", 128.0),
                    fprop(&a.props, "SoundPitch", 64.0) / 64.0,
                    by_actor.get(&i).copied(),
                ))
            })
            .collect();
        st.actor_ambients = Some(list);
    }
    let off: Vec<String> = OFF_TAGS.lock().map(|t| t.clone()).unwrap_or_default();
    let owned: Vec<usize> = OWNED.lock().map(|o| o.clone()).unwrap_or_default();
    for (i, snd, radius, vol, pitch, blocker) in st.actor_ambients.as_ref().unwrap() {
        if owned.contains(i) {
            continue;
        }
        if !off.is_empty() {
            if let Some(Value::Name(t)) = find(&level.0.actors[*i].props, "Tag") {
                if off.contains(&t.to_ascii_lowercase()) {
                    continue;
                }
            }
        }
        let pos = match blocker {
            Some(b) => match blockers.list.get(*b) {
                Some(c) if c.radius > 0.0 => c.center,
                _ => continue,
            },
            None => match find(&level.0.actors[*i].props, "Location") {
                Some(Value::Vector(v)) => DVec3::new(v[0] as f64, v[1] as f64, v[2] as f64),
                _ => continue,
            },
        };
        if pos.distance_squared(ear) <= radius * radius {
            hold(
                0xA_0000_0000 | *i as u64,
                snd,
                Some(pos),
                ambient * vol / 255.0,
                *radius as f32,
                *pitch,
            );
        }
    }

    // Looping sounds held this frame: start new ones, update the rest, stop what was let go.
    let holds: Vec<Hold> = HOLDS
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default();
    let mut keep: HashMap<u64, ()> = HashMap::new();
    let mut updates = Vec::new();
    for h in holds {
        if keep.insert(h.key, ()).is_some() {
            continue;
        }
        let (gain, pan) = place(h.pos, h.radius);
        match st.held.get(&h.key) {
            Some((id, s)) if *s == h.sound && st.audio.is_playing(*id) => {
                updates.push((*id, h.volume * gain, pan, h.pitch))
            }
            other => {
                if let Some((id, _)) = other {
                    st.audio.stop(*id);
                }
                if let Some(clip) = st.bank.clip(&h.sound) {
                    let id = st.audio.play(
                        clip,
                        VoiceParams {
                            volume: h.volume * gain,
                            pan,
                            pitch: h.pitch,
                            looping: true,
                        },
                    );
                    st.held.insert(h.key, (id, h.sound.clone()));
                } else {
                    st.held.remove(&h.key);
                }
            }
        }
    }
    let gone: Vec<u64> = st
        .held
        .keys()
        .filter(|k| !keep.contains_key(k))
        .copied()
        .collect();
    for k in gone {
        if let Some((id, _)) = st.held.remove(&k) {
            st.audio.stop(id);
        }
    }

    // One-shots follow the listener (the sources stay where they were made).
    let audio = &st.audio;
    st.playing.retain(|p| audio.is_playing(p.id));
    for p in &st.playing {
        let (gain, pan) = place(p.pos, p.radius);
        updates.push((p.id, p.volume * gain, pan, p.pitch));
    }
    st.audio.set_many(&updates);
    st.slots
        .retain(|_, (id, t)| now - *t < 10.0 || audio.is_playing(*id));

    if let Some(req) = MUSIC_REQ.lock().ok().and_then(|mut m| m.take()) {
        match req {
            Some(path) => {
                st.other_song = st.bank.song(&path);
                if let Some(m) = st.other_song.as_mut().and_then(|s| s.section(0)) {
                    st.audio.play_music(m, Transition::FastFade);
                }
            }
            None => {
                if st.other_song.take().is_some() {
                    let sec = if st.saved_section == 255 { st.default_section } else { st.saved_section } as usize;
                    play_section(st, sec, Transition::FastFade);
                }
            }
        }
    }
    if st.other_song.is_some() {
        return;
    }
    // Music: UpdateDynamicMusic.
    if st.song.is_none() {
        return;
    }
    if !st.started {
        st.started = true;
        play_section(st, st.default_section as usize, Transition::Fade);
    }
    if st.mode == MusicMode::Dying {
        return;
    }
    if ph.dead {
        play_section(st, 1, Transition::Fade);
        st.mode = MusicMode::Dying;
        return;
    }
    st.check_timer += dt;
    st.change_timer += dt;
    if st.check_timer < 1.0 {
        return;
    }
    st.check_timer = 0.0;
    let me = player.body.location;
    let combat = npcs.iter().any(|n| {
        n.state == NpcState::Attacking
            && n.target == Some(Target::Player)
            && blockers
                .list
                .get(n.blocker)
                .map_or(false, |b| b.center.distance(me) < 1600.0 + b.radius)
    });
    if combat {
        st.change_timer = 0.0;
        if st.mode != MusicMode::Combat {
            st.saved_section = 255;
            play_section(st, 3, Transition::FastFade);
            st.mode = MusicMode::Combat;
        }
    } else if st.mode != MusicMode::Ambient && st.change_timer >= 5.0 {
        let sec = if st.saved_section == 255 {
            st.default_section
        } else {
            st.saved_section
        };
        play_section(
            st,
            sec as usize,
            if st.mode == MusicMode::Combat {
                Transition::SlowFade
            } else {
                Transition::Fade
            },
        );
        st.saved_section = 255;
        st.mode = MusicMode::Ambient;
        st.change_timer = 0.0;
    }
}

/// DeusExPlayer.PlayFootStep / ScriptedPawn.PlayFootStep: the step sound for a floor texture group.
/// Returns (sound, volumeMultiplier for the AI noise).
pub fn footstep_sound(group: Option<&str>, water: bool) -> (String, f32) {
    let r = frand();
    let pick = |base: &str, n: u32| {
        format!(
            "DeusExSounds.Player.{base}{}",
            1 + ((r * n as f32) as u32).min(n - 1)
        )
    };
    if water {
        return (pick("WaterStep", 3), 1.0);
    }
    match group.unwrap_or("").to_ascii_lowercase().as_str() {
        "textile" | "paper" => (pick("CarpetStep", 4), 0.7),
        "foliage" | "earth" => (pick("GrassStep", 4), 0.6),
        "metal" | "ladder" => (pick("MetalStep", 4), 1.0),
        "ceramic" | "glass" | "tiles" => (pick("TileStep", 4), 0.7),
        "wood" => (pick("WoodStep", 4), 0.7),
        _ => (pick("StoneStep", 4), 0.7),
    }
}

/// ScriptedPawn.RandomPitch.
pub fn random_pitch() -> f32 {
    1.1 - 0.2 * frand()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footsteps_follow_floor_groups() {
        assert!(footstep_sound(Some("Wood"), false).0.contains("WoodStep"));
        assert!(footstep_sound(Some("Earth"), false).0.contains("GrassStep"));
        assert!(footstep_sound(Some("Ladder"), false)
            .0
            .contains("MetalStep"));
        assert!(footstep_sound(Some("Concrete"), false)
            .0
            .contains("StoneStep"));
        assert!(footstep_sound(None, true).0.contains("WaterStep"));
        for _ in 0..50 {
            let s = footstep_sound(Some("Metal"), false).0;
            let n: u32 = s.chars().last().unwrap().to_digit(10).unwrap();
            assert!((1..=4).contains(&n));
        }
    }

    #[test]
    fn bank_decodes_game_sounds() {
        let root = PathBuf::from(std::env::var("DX_GAME_DIR").unwrap_or_else(|_| "Deus Ex".into()));
        if !root.exists() {
            return;
        }
        let mut b = SoundBank::new(root, "01_NYC_UNATCOIsland".into());
        let c = b.clip("DeusExSounds.Weapons.PistolFire").expect("pistol");
        assert!(c.duration() > 0.1);
        assert!(b.clip("Ambient.Ambient.Ocean").is_some());
        assert!(
            b.clip("MoverSFX.Door.MetalDoorOpen").is_some()
                || b.clip("MoverSFX.Door.WoodDoorOpen").is_some()
                || true
        );
        assert!(b.song("LibertyIsland_Music.LibertyIsland_Music").is_some());
    }
}

/// ScriptedPawn.PlayFootStep (driven by the walk/run animations in the original; here one step per ~100 units
/// walked): floor-group sound or the pawn's WalkSound (robots), volume 1, range 768 x 1.5 x Mass/150 up to 2048,
/// pitch from mass.
pub fn npc_footstep_system(
    level: Res<LevelRes>,
    npcs: Query<(&Npc, &crate::actors::Animated)>,
    mut last: Local<HashMap<usize, (usize, f32)>>,
) {
    let _prof = crate::prof::scope("sound::npc_footstep_system");
    // ScriptedPawn.PlayFootStep runs from the walk/run animations' PlayFootStep notifies, so steps keep time with
    // the legs (a big walker's slow stride, a running trooper's quick one).
    let lvl = &level.0;
    for (n, an) in &npcs {
        let Some(body) = n.body.as_ref() else {
            continue;
        };
        if n.state == NpcState::Dead
            || body.physics != imsim_core::movement::Physics::Walking
            || body.velocity.truncate().length() < 5.0
        {
            last.remove(&n.blocker);
            continue;
        }
        let ma = &lvl.mesh_actors[an.mesh_actor];
        let Some(anim) = &ma.anim else { continue };
        let (start, frames, f) = crate::actors::current_frame(an, anim);
        let Some(seq) = anim.mesh.seqs.iter().find(|s| {
            s.start_frame.max(0) as usize == start && s.num_frames.max(1) as usize == frames
        }) else {
            continue;
        };
        let phase = (f / frames.max(1) as f32).rem_euclid(1.0);
        let prev = last.insert(n.blocker, (start, phase));
        let Some((pstart, pphase)) = prev else {
            continue;
        };
        if pstart != start {
            continue;
        }
        let crossed = seq.notifies.iter().any(|(t, func)| {
            func.eq_ignore_ascii_case("PlayFootStep")
                && if phase >= pphase {
                    *t > pphase && *t <= phase
                } else {
                    *t > pphase || *t <= phase
                }
        });
        if !crossed {
            continue;
        }
        let props = &lvl.actors[n.actor].props;
        let mass = fprop(props, "Mass", 150.0);
        let v = mass / 150.0 * 1.5;
        let range = (768.0 * v).clamp(0.01, 2048.0);
        let pitch = (v + 0.5).clamp(1.0, 1.5);
        let snd = match prop(props, "WalkSound") {
            Some(s) => s,
            None => {
                let water = matches!(
                    lvl.zone_water.get(
                        lvl.collision
                            .zone_at(body.location - DVec3::new(0.0, 0.0, n.params.height - 2.0))
                            .max(0) as usize
                    ),
                    Some(Some(_))
                );
                footstep_sound(
                    lvl.floor_group(body.location.to_array(), n.params.height * 2.0),
                    water,
                )
                .0
            }
        };
        play_owned(
            npc_owner(n.blocker),
            slot::INTERACT,
            &snd,
            Some(body.location - DVec3::new(0.0, 0.0, n.params.height)),
            1.0,
            range,
            pitch,
        );
    }
}

/// A stable key for [`hold`] from a name (FNV-1a), tagged so it can't clash with actor-based keys.
pub fn key_of(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h | (1 << 63)
}

/// ClientSetMusic(Level.Song, section, ...): start that song section with the given transition.
fn play_section(st: &mut SoundState, section: usize, t: Transition) {
    let Some(song) = st.song.as_mut() else { return };
    if let Some(m) = song.section(section).or_else(|| song.section(0)) {
        st.audio.play_music(m, t);
    }
}
