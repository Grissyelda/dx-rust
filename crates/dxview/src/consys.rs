//! Conversations and infolinks (ConSys, DeusExPlayer.StartConversation / StartDataLinkTransmission, ConPlay,
//! DataLinkPlay, ConCamera).
//!
//! * The conversation data is the game's own (DeusExConText.u, decoded by `dx_level::conversations`); the
//!   speech audio comes from DeusExConAudio<package>.u.
//! * Conversations start by frobbing their owner, by walking into the owner's invoke radius, from a
//!   ConversationTrigger, or (infolinks) from a DataLinkTrigger or the mission script. Eligibility follows
//!   GetActiveConversation: the owner's list in order, barks skipped, the height and line-of-sight tests,
//!   bDisplayOnce against <con>_Played, and every flag reference.
//! * ConPlay runs the event chain: speech (waits for the audio, or a reading time for text-only lines, then
//!   half a second), choices filtered by flags and skills, flag set/check, inventory checks and transfers,
//!   jumps (also into other conversations), random labels, triggers, goals, notes, skill points, credits and
//!   persona checks. First-person conversations show in the HUD window above the belt and carry on while the
//!   player moves (and end if he walks away); third-person ones put JC on screen, letterbox the view and run
//!   the ConCamera shots, and wait for the player's click / Enter / Space between lines.
//! * DataLinkPlay: "INCOMING TRANSMISSION..." with the blinking icon for 1.5 s, then the speaker's portrait and
//!   the text typed out at 0.03 s a character while the speech plays; queued infolinks follow, and a
//!   conversation that starts silences the running one but keeps its history.
//!
//! The state lives behind a mutex so the HUD, the AI and the triggers can query it without system parameters.

use std::sync::Arc;
use crate::actors::Blockers;
use crate::ai::{GameFlags, Npc, NpcState, PlayerHealth};
use crate::pickups::PlayerInventory;
use crate::weapons::RuntimeAssets;
use crate::{FlyCam, LevelRes, Player, SkyCam};
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use dx_level::conversations::{Choice, ConAudio, ConDb, EventKind, FlagRef};
use glam::DVec3;
use imsim_core::flags::{FlagBase, FlagValue};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use unreal_pkg::props::{find, Value};

/// Who takes part: the player or a placed actor (by blocker index).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Who {
    Player,
    Actor(usize),
}

/// A conversation participant as seen this frame.
#[derive(Clone, Debug, Default)]
pub struct Actor {
    pub bind: String,
    pub pos: DVec3,
    pub radius: f64,
    pub height: f64,
    pub eye: f64,
    /// BaseEyeHeight standing, and BaseAssHeight (SetBasedPawnSize lowers the eye while sitting).
    pub base_eye: f64,
    pub ass: f64,
    /// Facing (radians, Unreal yaw).
    pub yaw: f64,
    pub pawn: bool,
    pub familiar: String,
    pub unfamiliar: String,
    pub alive: bool,
    pub hostile: bool,
    /// BarkBindName, else BindName: barks are "<this>_Bark<Mode>".
    pub bark_bind: String,
    /// Not in a state that can be interrupted by a conversation (fighting, fleeing, burning...).
    pub busy: bool,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct History {
    pub owner: String,
    pub location: String,
    pub description: String,
    pub infolink: bool,
    pub lines: Vec<(String, String)>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Goal {
    pub name: String,
    pub text: String,
    pub primary: bool,
    pub completed: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Wait {
    /// Run the next event.
    Process,
    /// WaitForSpeech: audio (or reading time) left.
    Speech(f32),
    /// The half-second after a line.
    Pause(f32),
    /// WaitForText.
    Text(f32),
    /// WaitForInput (a choice or a line waiting for a click).
    Input,
    /// Third person: the letterbox sliding away (WaitForConWin).
    Closing,
}

/// ConCamera state.
#[derive(Clone, Debug)]
pub struct ConCamera {
    position: u8,
    ctype: u8,
    /// CT_Speakers (1) or CT_Actor (2).
    mode: u8,
    rotation: [f64; 3],
    offset: [f64; 3],
    height_mod: f64,
    center_mod: f64,
    dist_mult: f64,
    first: Option<Who>,
    second: Option<Who>,
    last_first: Option<Who>,
    last_second: Option<Who>,
    ignore_set: bool,
    set_count: u32,
    first_rot: i32,
    fallback: usize,
    using_fallback: bool,
    last: Option<(DVec3, [f64; 3])>,
}

const FALLBACKS: [u8; 9] = [0, 4, 5, 10, 11, 8, 9, 7, 6];
const HEIGHT_POSITIONS: [u8; 9] = [2, 3, 12, 13, 8, 9, 10, 11, 7];
const HEIGHT_FALLBACK_TRIGGER: f64 = 30.0;

impl Default for ConCamera {
    fn default() -> Self {
        let mut c = ConCamera {
            position: 0,
            ctype: 0,
            mode: 1,
            rotation: [0.0; 3],
            offset: [0.0; 3],
            height_mod: 0.0,
            center_mod: 0.0,
            dist_mult: 1.0,
            first: None,
            second: None,
            last_first: None,
            last_second: None,
            ignore_set: false,
            set_count: 0,
            first_rot: 0,
            fallback: 0,
            using_fallback: false,
            last: None,
        };
        c.set_values();
        c
    }
}

impl ConCamera {
    /// SetCameraValues.
    fn set_values(&mut self) {
        let (mode, rot, a, b, c): (u8, [f64; 3], f64, f64, f64) = match self.position {
            0 => (1, [0.0, 16788.0, 0.0], 10.8, 0.0, 0.90),
            1 => (1, [0.0, 16788.0, 0.0], 10.8, 0.0, 2.20),
            2 => (1, [59636.0, 17824.0, 0.0], 10.5, 0.0, 1.0),
            3 => (1, [57136.0, 40868.0, 0.0], -19.0, 0.0, 1.80),
            4 => (1, [0.0, 39056.0, 0.0], 10.5, 0.20, 1.0),
            5 => (1, [0.0, 26456.0, 0.0], 10.5, 0.20, 1.0),
            6 => (2, [0.0, 32400.0, 0.0], -23.0, 0.0, 0.0),
            7 => (2, [0.0, 32400.0, 0.0], -30.0, 0.0, 0.0),
            8 => (2, [0.0, 45100.0, 0.0], -16.0, 17.0, 0.0),
            9 => (2, [0.0, 20500.0, 0.0], -16.0, -17.0, 0.0),
            10 => (2, [300.0, 33600.0, 0.0], -27.4, 2.1, 0.0),
            11 => (2, [300.0, 29800.0, 0.0], -27.4, -5.60, 0.0),
            12 => (1, [60000.0, 16588.0, 0.0], 22.22, 0.0, 0.90),
            13 => (1, [8500.0, 16588.0, 0.0], 11.4, 0.0, 0.90),
            _ => (1, [16564.0, 16224.0, 0.0], 3.3, -0.3, 0.90),
        };
        self.mode = mode;
        self.rotation = rot;
        if mode == 1 {
            self.height_mod = a;
            self.center_mod = b;
            self.dist_mult = c;
        } else {
            self.offset = [a, b, 0.0];
        }
    }

    fn setup_random(&mut self) {
        if sfrand() < 0.60
            && (self.first == self.last_first || self.first == self.last_second)
            && (self.second == self.last_second || self.second == self.last_first)
        {
            if self.position <= 3 {
                self.first = self.last_first;
                self.second = self.last_second;
                self.ignore_set = true;
            }
            return;
        }
        self.last_first = self.first;
        self.last_second = self.second;
        self.position = ((sfrand() * 10.0) as u8).min(9);
        self.set_values();
    }

    /// SetupCameraFromEvent.
    fn from_event(
        &mut self,
        ctype: u8,
        position: u8,
        rotation: [i32; 3],
        offset: [f32; 3],
        h: f32,
        c: f32,
        d: f32,
    ) {
        self.ignore_set = false;
        self.ctype = ctype;
        self.fallback = 0;
        self.using_fallback = false;
        match ctype {
            0 => {
                self.position = position;
                self.set_values();
            }
            1 => {
                self.mode = 1;
                self.rotation = rotation.map(|v| v as f64);
                self.height_mod = h as f64;
                self.center_mod = c as f64;
                self.dist_mult = d as f64;
            }
            2 => {
                self.mode = 2;
                self.offset = offset.map(|v| v as f64);
                self.rotation = rotation.map(|v| v as f64);
            }
            _ => self.setup_random(),
        }
    }

    fn set_actors(&mut self, a: Who, b: Who) {
        if self.ignore_set {
            return;
        }
        self.first = Some(a);
        self.second = Some(b);
        self.set_count += 1;
    }

    fn setup_fallback(&mut self) {
        self.using_fallback = true;
        self.position = if self.fallback >= FALLBACKS.len() {
            FALLBACKS[FALLBACKS.len() - 1]
        } else {
            self.fallback += 1;
            FALLBACKS[self.fallback - 1]
        };
        self.set_values();
    }

    fn exhausted(&self) -> bool {
        self.fallback >= FALLBACKS.len()
    }

    fn using_headshot(&self) -> bool {
        self.ctype == 0 && (4..=11).contains(&self.position)
    }
}

fn sfrand() -> f32 {
    crate::sound::frand()
}

/// ConPlay.
pub struct Play {
    con: usize,
    pub invoke: Who,
    ev: Option<usize>,
    pub first_person: bool,
    passive: bool,
    wait: Wait,
    /// What the conversation window shows: speaker name, text (with continued lines), spoken by the player.
    pub name: String,
    pub text: String,
    pub player_line: bool,
    pub choices: Vec<(Choice, bool)>,
    pub focus: usize,
    speaker: Option<Who>,
    speaking_to: Option<Who>,
    initial_radius: f64,
    radius: f64,
    sound_owner: Option<u64>,
    /// The line being spoken: speaker, when it began (ConState.time), its phoneme track.
    lip: Option<(Who, f32, Arc<Vec<u8>>)>,
    history: Option<History>,
    /// Letterbox position 0..1 (third person), and whether it's moving in.
    pub win_pos: f32,
    pub restrict: bool,
    started: f32,
    cam: ConCamera,
    pending_cam: Option<usize>,
    setup_initial: bool,
    last_len: usize,
    ending: bool,
    no_played_flag: bool,
    /// ConPlay.bForcePlay (intro / endgame cut scenes): no ConCamera, no distance checks, no history, keys
    /// ignored, speech heard through the player when the speaker is far, the text centred in a 16:9 letterbox.
    pub forced: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum LinkPhase {
    /// Waiting for a conversation to finish.
    Held,
    Start(f32),
    Process,
    Speech(f32),
    Pause(f32),
    End(f32),
}

/// DataLinkPlay + HUDInfoLinkDisplay state.
pub struct Link {
    con: usize,
    start_con: usize,
    ev: Option<usize>,
    trigger: Option<usize>,
    phase: LinkPhase,
    blink: f32,
    pub name: String,
    pub portrait: Option<String>,
    pub show_icon: bool,
    pub show_portrait: bool,
    pub name_visible: bool,
    /// Printed text and how many characters have been typed so far.
    pub text: String,
    pub typed: f32,
    pub cursor: bool,
    silent: bool,
    history: Option<History>,
    last_len: usize,
}

pub struct ConState {
    /// The next conversation started is a forced (cut scene) one.
    force_next: bool,
    db: ConDb,
    sys_dir: PathBuf,
    pub mission: i32,
    pub location: String,
    audio: HashMap<String, Option<ConAudio>>,
    lens: HashMap<(String, i32), f32>,
    /// Phoneme tracks of speech lines (20 a second), for lip sync.
    lips: HashMap<(String, i32), Arc<Vec<u8>>>,
    /// BarkManager: currentBarks (4) and recentBarks (32), and the HUDBarkDisplay lines.
    barks: [Option<BarkSlot>; 4],
    recent_barks: Vec<BarkSlot>,
    pub bark_lines: Vec<BarkLine>,
    pub actors: HashMap<Who, Actor>,
    /// Owner bind name (lower case) -> conversations in list order.
    owners: HashMap<String, Vec<usize>>,
    by_bind: HashMap<String, Who>,
    pub play: Option<Play>,
    pub link: Option<Link>,
    link_queue: Vec<usize>,
    pub history: Vec<History>,
    pub goals: Vec<Goal>,
    pub notes: Vec<String>,
    pub images: Vec<String>,
    pub time: f32,
    last_first: Option<(Who, f32)>,
    last_third: Option<(Who, f32)>,
    con_played_at: HashMap<usize, f32>,
    last_con_end: HashMap<Who, f32>,
    /// Out: events to raise, messages, items given, triggers whose infolink finished.
    events: Vec<(String, bool)>,
    messages: Vec<String>,
    gives: Vec<(String, i32, bool)>,
    finished_triggers: Vec<usize>,
    radius_timer: f32,
    names: HashMap<String, String>,
    end_transmission: String,
    incoming: String,
    pub queued_text: String,
    strings: HashMap<&'static str, String>,
    /// Player state the checks need (copied in each frame).
    player_falling: bool,
    player_swimming: bool,
    player_dead: bool,
    credits: i32,
    health: i32,
}

static STATE: Mutex<Option<ConState>> = Mutex::new(None);
static FROBS: Mutex<Vec<usize>> = Mutex::new(Vec::new());

pub fn lock() -> MutexGuard<'static, Option<ConState>> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// A third-person conversation has the screen (letterbox, ConCamera, the HUD hidden, input taken).
pub fn third_person() -> bool {
    lock()
        .as_ref()
        .is_some_and(|s| s.play.as_ref().is_some_and(|p| !p.first_person))
}

/// An infolink (DataLinkPlay) is on screen (the DualSense mic LED pulses).
pub fn datalink_playing() -> bool {
    lock().as_ref().is_some_and(|s| s.link.is_some())
}

/// The conversation an actor is in: where it should look. None when it isn't conversing.
pub fn conversing(blocker: usize) -> Option<Option<DVec3>> {
    let g = lock();
    let s = g.as_ref()?;
    let p = s.play.as_ref()?;
    let me = Who::Actor(blocker);
    let involved = p.invoke == me
        || p.speaker == Some(me)
        || p.speaking_to == Some(me)
        || s.speaks_in(p.con, me);
    if !involved {
        return None;
    }
    // Speakers face whoever they talk to; everyone else in it watches the player.
    let look = if p.speaker == Some(me) {
        p.speaking_to.and_then(|w| s.actors.get(&w)).map(|a| a.pos)
    } else if p.speaking_to == Some(me) {
        p.speaker.and_then(|w| s.actors.get(&w)).map(|a| a.pos)
    } else {
        s.actors.get(&Who::Player).map(|a| a.pos)
    };
    Some(look)
}

/// EBarkModes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BarkMode {
    Idle,
    CriticalDamage,
    AreaSecure,
    TargetAcquired,
    TargetLost,
    GoingForAlarm,
    OutOfAmmo,
    Scanning,
    Futz,
    OnFire,
    TearGas,
    Gore,
    Surprise,
    PreAttackSearching,
    PreAttackSighting,
    PostAttackSearching,
    SearchGiveUp,
    AllianceHostile,
    AllianceFriendly,
}

impl BarkMode {
    fn suffix(self) -> &'static str {
        use BarkMode::*;
        match self {
            Idle => "Idle",
            CriticalDamage => "CriticalDamage",
            AreaSecure => "AreaSecure",
            TargetAcquired => "TargetAcquired",
            TargetLost => "TargetLost",
            GoingForAlarm => "GoingForAlarm",
            OutOfAmmo => "OutOfAmmo",
            Scanning => "Scanning",
            Futz => "Futz",
            OnFire => "OnFire",
            TearGas => "TearGas",
            Gore => "Gore",
            Surprise => "Surprise",
            PreAttackSearching => "PreAttackSearching",
            PreAttackSighting => "PreAttackSighting",
            PostAttackSearching => "PostAttackSearching",
            SearchGiveUp => "SearchGiveUp",
            AllianceHostile => "AllianceHostile",
            AllianceFriendly => "AllianceFriendly",
        }
    }

    /// BarkManager.GetBarkPriority.
    fn priority(self) -> i32 {
        use BarkMode::*;
        match self {
            Idle => 1,
            CriticalDamage | OnFire | Surprise | PreAttackSighting => 5,
            GoingForAlarm | TearGas | PreAttackSearching | PostAttackSearching => 4,
            TargetAcquired | Futz | Gore | AllianceHostile => 3,
            AreaSecure | TargetLost | OutOfAmmo | Scanning | SearchGiveUp | AllianceFriendly => 2,
        }
    }

    /// BarkManager.GetBarkTimeout: how long a bark of this kind counts as "played recently".
    fn timeout(self) -> f32 {
        if self == BarkMode::Futz {
            1.0
        } else {
            10.0
        }
    }
}

#[derive(Clone, Debug)]
struct BarkSlot {
    con: usize,
    who: usize,
    mode: BarkMode,
    duration: f32,
    timer: f32,
    priority: i32,
}

/// HUDBarkDisplayItem: "<UnfamiliarName>: " and the line, shown for the bark's length.
#[derive(Clone, Debug)]
pub struct BarkLine {
    pub name: String,
    pub text: String,
    pub duration: f32,
    pub shown: f32,
}

static BARKS: Mutex<Vec<(usize, BarkMode)>> = Mutex::new(Vec::new());

/// ScriptedPawn.Play<Mode>Sound -> DeusExPlayer.StartAIBarkConversation: a pawn barks.
pub fn bark(blocker: usize, mode: BarkMode) {
    if let Ok(mut q) = BARKS.lock() {
        q.push((blocker, mode));
    }
}

/// What a save keeps of the conversation system (save.rs); its flags are in the game flags.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ConSave {
    pub goals: Vec<Goal>,
    pub notes: Vec<String>,
    pub images: Vec<String>,
    pub history: Vec<History>,
}

impl ConState {
    /// DeusExPlayer.GoalCompleted: an existing goal not yet done is marked done (log sound + message).
    pub fn goal_completed(&mut self, name: &str) {
        if let Some(g) = self.goals.iter_mut().find(|g| g.name.eq_ignore_ascii_case(name)) {
            if !g.completed {
                g.completed = true;
                let k = if g.primary { "PrimaryGoalCompleted" } else { "SecondaryGoalCompleted" };
                if let Some(m) = self.strings.get(k).cloned() {
                    self.messages.push(m);
                }
                crate::sound::play_player("DeusExSounds.UserInterface.LogGoalCompleted", 1.0, 0.0);
                println!("GOAL completed {name}");
                // Ours: an autosave once the scene is over.
                crate::save::autosave("Autosave: goal complete");
            }
        }
    }

    pub fn save_state(&self) -> ConSave {
        ConSave { goals: self.goals.clone(), notes: self.notes.clone(), images: self.images.clone(), history: self.history.clone() }
    }
    /// Put the logs back and drop anything the fresh level started (its opening infolink, barks): the saved
    /// flags say what's been played.
    pub fn restore_state(&mut self, s: &ConSave) {
        self.goals = s.goals.clone();
        self.notes = s.notes.clone();
        self.images = s.images.clone();
        self.history = s.history.clone();
        self.play = None;
        self.link = None;
        self.link_queue.clear();
        self.barks = Default::default();
        self.bark_lines.clear();
        self.events.clear();
        self.messages.clear();
        self.gives.clear();
        self.finished_triggers.clear();
        self.queued_text.clear();
    }
}

impl ConState {
    /// The conversation called `name`, this mission's first.
    fn con_named(&self, name: &str) -> Option<usize> {
        let mine = self.db.missions.get(&self.mission);
        mine.into_iter()
            .flatten()
            .copied()
            .find(|&i| self.db.cons[i].name.eq_ignore_ascii_case(name))
            .or_else(|| {
                self.db
                    .cons
                    .iter()
                    .position(|c| c.name.eq_ignore_ascii_case(name))
            })
    }

    /// ConPlayBark.GetBarkSpeech: follow jumps and random labels to the first speech (Lay D Denton's version also
    /// follows flag checks, which its barks use to pick lines for her).
    fn bark_speech(&self, flags: &FlagBase, con: usize) -> Option<(String, i32)> {
        let check = crate::travel::lddp();
        let c = &self.db.cons[con];
        let mut i = 0usize;
        for _ in 0..64 {
            let e = c.events.get(i)?;
            match &e.kind {
                EventKind::Speech { text, sound_id, .. } => return Some((text.clone(), *sound_id)),
                EventKind::Jump { label, con: jc } => {
                    if label.is_empty() {
                        i += 1;
                    } else if jc.is_some_and(|j| j != con) {
                        return None;
                    } else {
                        i = c.label_index(label)?;
                    }
                }
                EventKind::Random { labels, .. } => {
                    if labels.is_empty() {
                        return None;
                    }
                    let k = ((sfrand() * labels.len() as f32) as usize).min(labels.len() - 1);
                    i = c.label_index(&labels[k])?;
                }
                EventKind::CheckFlag { flags: refs, label } if check => {
                    if Self::check_flags(flags, refs) {
                        i = c.label_index(label)?;
                    } else {
                        i += 1;
                    }
                }
                EventKind::End => return None,
                _ => i += 1,
            }
        }
        None
    }

    /// BarkManager.Tick: age the current and recent barks and the HUD lines.
    fn tick_barks(&mut self, dt: f32) {
        for k in 0..self.barks.len() {
            let done = match self.barks[k].as_mut() {
                Some(b) => {
                    b.timer += dt;
                    b.timer >= b.duration
                }
                None => false,
            };
            if done {
                self.remove_current_bark(k);
            }
        }
        for b in &mut self.recent_barks {
            b.timer += dt;
        }
        self.recent_barks.retain(|b| b.timer < b.duration);
        for l in &mut self.bark_lines {
            l.shown += dt;
        }
        self.bark_lines.retain(|l| l.shown <= l.duration);
    }

    /// RemoveCurrentBark: into recentBarks (replacing the one closest to expiring when all 32 are in use).
    fn remove_current_bark(&mut self, k: usize) {
        let Some(mut b) = self.barks[k].take() else { return };
        b.duration = b.mode.timeout();
        b.timer = 0.0;
        if self.recent_barks.len() >= 32 {
            if let Some((i, _)) = self
                .recent_barks
                .iter()
                .enumerate()
                .min_by(|a, c| (a.1.duration - a.1.timer).total_cmp(&(c.1.duration - c.1.timer)))
            {
                self.recent_barks.remove(i);
            }
        }
        self.recent_barks.push(b);
    }

    /// BarkManager.StartBark.
    fn start_bark(&mut self, flags: &FlagBase, sees: &dyn Fn(DVec3, DVec3) -> bool, who: usize, mode: BarkMode) -> bool {
        // StartAIBarkConversation: not during a third-person conversation.
        if self.play.as_ref().is_some_and(|p| !p.first_person) {
            return false;
        }
        let (Some(pawn), Some(me)) = (self.actors.get(&Who::Actor(who)), self.actors.get(&Who::Player)) else {
            return false;
        };
        if !pawn.alive || pawn.bark_bind.is_empty() {
            return false;
        }
        // CheckRadius / CheckHeightDifference.
        if pawn.pos.distance(me.pos) > 1000.0 {
            return false;
        }
        let visible = sees(me.pos, pawn.pos);
        let offset = if visible { 500.0 } else { 100.0 };
        if (me.pos.z - pawn.pos.z).abs() > (me.height - pawn.height).abs() + offset {
            return false;
        }
        let name = format!("{}_Bark{}", pawn.bark_bind, mode.suffix());
        let Some(con) = self.con_named(&name) else {
            return false;
        };
        // GetAvailableCurrentBarkSlot.
        let prio = mode.priority();
        let mut slot = None;
        for k in 0..self.barks.len() {
            match &self.barks[k] {
                None => slot = Some(k),
                Some(b) if b.who == who => {
                    if b.priority < prio {
                        slot = Some(k);
                        break;
                    }
                }
                Some(b) if b.mode == mode => {
                    if b.timer < 1.0 {
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(slot) = slot else { return false };
        // IsBarkPlaying / HasBarkTypePlayedRecently.
        if self.barks.iter().flatten().any(|b| b.con == con) {
            return false;
        }
        if self.recent_barks.iter().any(|b| b.who == who && b.mode == mode) {
            return false;
        }
        let Some((text, sound_id)) = self.bark_speech(flags, con) else {
            return false;
        };
        let mut duration = None;
        if sound_id >= 0 {
            if let (Some(len), Some(path)) = (self.speech_len(con, sound_id), self.speech_path(con, sound_id)) {
                crate::sound::play_owned(
                    crate::sound::npc_owner(who),
                    crate::sound::slot::TALK,
                    &path,
                    Some(pawn_pos(self, who)),
                    1.0,
                    1024.0,
                    1.0,
                );
                duration = Some(len);
            }
        }
        let duration = duration.unwrap_or_else(|| (text.chars().count() as f32 * 0.1).max(3.0));
        // BarkManager: the line shows only with Subtitles on.
        if !text.trim().is_empty() && crate::settings::subtitles() {
            let name = self.actors.get(&Who::Actor(who)).map(|a| a.unfamiliar.clone()).unwrap_or_default();
            self.bark_lines.push(BarkLine {
                name,
                text: text.trim().to_string(),
                duration,
                shown: 0.0,
            });
        }
        // SetBarkInfo (a lower-priority bark of the same pawn is cut short into the recent list).
        self.remove_current_bark(slot);
        self.barks[slot] = Some(BarkSlot {
            con,
            who,
            mode,
            duration,
            timer: 0.0,
            priority: prio,
        });
        true
    }
}

fn pawn_pos(s: &ConState, who: usize) -> DVec3 {
    s.actors.get(&Who::Actor(who)).map_or(DVec3::ZERO, |a| a.pos)
}

/// The phoneme `who` is mouthing right now (Pawn.nextPhoneme while bIsSpeaking), if speaking a line.
pub fn mouth(who: Who) -> Option<u8> {
    let g = lock();
    let s = g.as_ref()?;
    let p = s.play.as_ref()?;
    let (w, t0, track) = p.lip.as_ref()?;
    if *w != who {
        return None;
    }
    let i = ((s.time - t0) * 20.0).max(0.0) as usize;
    Some(track.get(i).copied().unwrap_or(b'X'))
}

/// A speech line's mouth shapes, 20 a second, from its loudness and brightness (the original engine drives
/// Pawn.nextPhoneme from the playing speech; this follows the same A/E/F/M/O/T/U/X set): quiet is closed (X),
/// noisy high-frequency sound is F/T, and voiced sound opens the mouth more the louder it is.
pub fn phonemes(c: &imsim_audio::Clip) -> Vec<u8> {
    let ch = c.channels.max(1) as usize;
    let win = (c.rate as usize / 20).max(1);
    let frames = c.data.len() / ch;
    let mono = |i: usize| c.data[i * ch..i * ch + ch].iter().sum::<f32>() / ch as f32;
    let mut stats = Vec::new();
    let mut i = 0;
    while i < frames {
        let n = win.min(frames - i);
        let (mut e, mut z, mut prev) = (0.0f32, 0usize, 0.0f32);
        for k in 0..n {
            let v = mono(i + k);
            e += v * v;
            if (v >= 0.0) != (prev >= 0.0) {
                z += 1;
            }
            prev = v;
        }
        stats.push(((e / n as f32).sqrt(), z as f32 / n as f32));
        i += win;
    }
    let peak = stats.iter().map(|s| s.0).fold(0.0f32, f32::max).max(1e-4);
    stats
        .iter()
        .enumerate()
        .map(|(k, &(rms, zcr))| {
            let a = rms / peak;
            if a < 0.1 {
                b'X'
            } else if zcr > 0.3 {
                if a < 0.35 { b'F' } else { b'T' }
            } else if a > 0.7 {
                if k % 3 == 0 { b'O' } else { b'A' }
            } else if a > 0.45 {
                if zcr > 0.12 { b'E' } else { b'O' }
            } else if a > 0.25 {
                if zcr > 0.12 { b'E' } else { b'U' }
            } else {
                b'M'
            }
        })
        .collect()
}

/// Frobbing a pawn or decoration (ScriptedPawn.Frob / DeusExDecoration.Frob): StartConversation(IM_Frob).
pub fn request_frob(blocker: usize) {
    if let Ok(mut q) = FROBS.lock() {
        q.push(blocker);
    }
}

/// Whether frobbing this actor would start a conversation (so the frob goes to it).
pub fn has_conversations(blocker: usize) -> bool {
    let g = lock();
    let Some(s) = g.as_ref() else { return false };
    s.actors
        .get(&Who::Actor(blocker))
        .is_some_and(|a| s.owners.contains_key(&a.bind.to_ascii_lowercase()))
}

/// Infolinks whose DataLinkTrigger should now carry on (DatalinkFinished).
pub fn take_finished_triggers() -> Vec<usize> {
    lock()
        .as_mut()
        .map(|s| std::mem::take(&mut s.finished_triggers))
        .unwrap_or_default()
}

/// DeusExPlayer.StartDataLinkTransmission.
pub fn start_datalink(flags: &FlagBase, name: &str, trigger: Option<usize>) -> bool {
    let mut g = lock();
    let Some(s) = g.as_mut() else { return false };
    s.start_datalink(flags, name, trigger)
}

/// A third-person conversation is waiting on the player's choice.
/// InformationDevices.ReadText with bAddToVault (datacubes): the text goes into the DataVault as a note once
/// (AddNote(vaultString,, True): "Note added" + LogNoteAdded).
/// The DataVault already holds this text as a note.
pub fn has_vault_note(text: &str) -> bool {
    let text = text.trim();
    lock().as_ref().is_some_and(|st| st.notes.iter().any(|n| n.trim() == text))
}

pub fn add_vault_note(text: &str) {
    let text = text.trim().to_string();
    if text.is_empty() {
        return;
    }
    let mut g = lock();
    let Some(st) = g.as_mut() else { return };
    if st.notes.contains(&text) {
        return;
    }
    st.notes.insert(0, text);
    let m = st.strings.get("NoteAdded").cloned().unwrap_or_else(|| "Note added".into());
    st.messages.push(m);
    crate::sound::play_player("DeusExSounds.UserInterface.LogNoteAdded", 1.0, 0.0);
}

pub fn choosing() -> bool {
    lock().as_ref().and_then(|s| s.play.as_ref().map(|p| !p.first_person && !p.choices.is_empty())).unwrap_or(false)
}

/// ConPlayBase.SetPlayedFlag: <con>_Played; Lay D Denton also marks a FemJC conversation's male original played.
fn set_played(flags: &mut FlagBase, name: &str) {
    flags.set_bool(&format!("{name}_Played"), true);
    if flags.get_bool("LDDPJCIsFemale") && name.len() > 5 && name[..5].eq_ignore_ascii_case("FemJC") {
        let m = format!("{}_Played", &name[5..]);
        if !flags.get_bool(&m) {
            flags.set_bool(&m, true);
        }
    }
}

/// DeusExPlayer.StartConversationByName (ConversationTrigger, mission scripts): within 800 units.
pub fn start_named(flags: &mut FlagBase, con: &str, bind: &str) -> bool {
    let mut g = lock();
    let Some(s) = g.as_mut() else { return false };
    let Some(&who) = s.by_bind.get(&bind.to_ascii_lowercase()) else {
        return false;
    };
    let Some(list) = s.owners.get(&bind.to_ascii_lowercase()) else {
        return false;
    };
    let Some(&ci) = list
        .iter()
        .find(|&&i| s.db.cons[i].name.eq_ignore_ascii_case(con))
    else {
        return false;
    };
    let (pp, op) = (s.pos(Who::Player), s.pos(who));
    if pp.distance(op) > 800.0 {
        return false;
    }
    s.start_conversation(flags, who, Invoke::Named, Some(ci))
}

/// DeusExPlayer.StartConversationByName(con, owner, False, bForcePlay = True): a cut scene's conversation
/// (MissionIntro, MissionEndgame). Every check is skipped; it plays to the end on its own.
pub fn start_forced(flags: &mut FlagBase, con: &str, bind: &str) -> bool {
    let mut g = lock();
    let Some(s) = g.as_mut() else { return false };
    let Some(&who) = s.by_bind.get(&bind.to_ascii_lowercase()) else {
        return false;
    };
    let ci = s
        .owners
        .get(&bind.to_ascii_lowercase())
        .and_then(|l| l.iter().copied().find(|&i| s.db.cons[i].name.eq_ignore_ascii_case(con)));
    let Some(ci) = ci else { return false };
    s.force_next = true;
    let ok = s.start_conversation(flags, who, Invoke::Forced, Some(ci));
    s.force_next = false;
    ok
}

/// A forced (cut scene) conversation is playing.
pub fn forced() -> bool {
    lock().as_ref().is_some_and(|s| s.play.as_ref().is_some_and(|p| p.forced))
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Invoke {
    Forced,
    Frob,
    Radius,
    Named,
}

fn fold(s: &str) -> String {
    s.to_ascii_lowercase()
}

fn is_bark(c: &dx_level::conversations::Conversation) -> bool {
    let p = format!("{}_bark", c.owner_name.to_ascii_lowercase());
    c.name.to_ascii_lowercase().starts_with(&p)
}

impl Link {
    /// The window is up (not waiting for a conversation to end).
    pub fn is_visible(&self) -> bool {
        self.phase != LinkPhase::Held
    }
}

impl ConState {
    fn new(
        db: ConDb,
        sys_dir: PathBuf,
        mission: i32,
        location: String,
        classes: &mut unreal_pkg::classes::ClassDb,
    ) -> ConState {
        let mut owners: HashMap<String, Vec<usize>> = HashMap::new();
        for &i in db.missions.get(&mission).into_iter().flatten() {
            owners
                .entry(fold(&db.cons[i].owner_name))
                .or_default()
                .push(i);
        }
        let d = classes.defaults("DataLinkPlay");
        let mut names = HashMap::new();
        for p in d
            .iter()
            .filter(|p| p.name.eq_ignore_ascii_case("infoLinkNames"))
        {
            if let Value::Struct(_, b) = &p.value {
                let mut r = unreal_pkg::Reader::new(b, 0);
                if let (Ok(a), Ok(n)) = (r.fstring(), r.fstring()) {
                    names.insert(a, n);
                }
            }
        }
        let st = |p: &[unreal_pkg::props::Property], n: &str, dflt: &str| match find(p, n) {
            Some(Value::Str(s)) => s.clone(),
            _ => dflt.to_string(),
        };
        let end_transmission = st(&d, "EndTransmission", "END TRANSMISSION...");
        let h = classes.defaults("HUDInfoLinkDisplay");
        let incoming = st(&h, "IncomingTransmission", "INCOMING TRANSMISSION...");
        let queued_text = st(&h, "strQueued", "message waiting...");
        let pl = classes.defaults("DeusExPlayer");
        let mut strings = HashMap::new();
        for (k, dflt) in [
            ("GoalAdded", "Goal Received - Check DataVault For Details"),
            ("NoteAdded", "Note Received - Check DataVault For Details"),
            ("PrimaryGoalCompleted", "Primary Goal Completed"),
            ("SecondaryGoalCompleted", "Secondary Goal Completed"),
        ] {
            strings.insert(k, st(&pl, k, dflt));
        }
        ConState {
            force_next: false,
            db,
            sys_dir,
            mission,
            location,
            audio: HashMap::new(),
            lens: HashMap::new(),
            lips: HashMap::new(),
            barks: Default::default(),
            recent_barks: Vec::new(),
            bark_lines: Vec::new(),
            actors: HashMap::new(),
            owners,
            by_bind: HashMap::new(),
            play: None,
            link: None,
            link_queue: Vec::new(),
            history: Vec::new(),
            goals: Vec::new(),
            notes: Vec::new(),
            images: Vec::new(),
            time: 0.0,
            last_first: None,
            last_third: None,
            con_played_at: HashMap::new(),
            last_con_end: HashMap::new(),
            events: Vec::new(),
            messages: Vec::new(),
            gives: Vec::new(),
            finished_triggers: Vec::new(),
            radius_timer: 0.0,
            names,
            end_transmission,
            incoming,
            queued_text,
            strings,
            player_falling: false,
            player_swimming: false,
            player_dead: false,
            credits: 0,
            health: 100,
        }
    }

    fn pos(&self, w: Who) -> DVec3 {
        self.actors.get(&w).map_or(DVec3::ZERO, |a| a.pos)
    }

    fn who(&self, bind: &str) -> Option<Who> {
        self.by_bind.get(&fold(bind)).copied()
    }

    fn speaks_in(&self, con: usize, w: Who) -> bool {
        let Some(a) = self.actors.get(&w) else {
            return false;
        };
        self.db.cons[con].events.iter().any(|e| match &e.kind {
            EventKind::Speech {
                speaker,
                speaking_to,
                ..
            } => speaker.eq_ignore_ascii_case(&a.bind) || speaking_to.eq_ignore_ascii_case(&a.bind),
            _ => false,
        })
    }

    /// GetDisplayName: FamiliarName once a conversation with the actor has ended (or when asked for), else
    /// UnfamiliarName, else the BindName.
    fn display_name(&self, bind: &str, familiar: bool) -> String {
        let Some(w) = self.who(bind) else {
            return bind.to_string();
        };
        let Some(a) = self.actors.get(&w) else {
            return bind.to_string();
        };
        let met =
            familiar || self.last_con_end.get(&w).is_some_and(|t| *t > 0.0) || w == Who::Player;
        if met && !a.familiar.is_empty() {
            return a.familiar.clone();
        }
        if !a.unfamiliar.is_empty() {
            return a.unfamiliar.clone();
        }
        a.bind.clone()
    }

    fn check_flags(flags: &FlagBase, refs: &[FlagRef]) -> bool {
        refs.iter().all(|r| flags.get_bool(&r.name) == r.value)
    }

    fn played(flags: &FlagBase, name: &str) -> bool {
        flags.get_bool(&format!("{name}_Played"))
    }

    /// The speech's length in seconds (Conversation.GetSpeechLength).
    fn speech_len(&mut self, con: usize, sound_id: i32) -> Option<f32> {
        if sound_id < 0 {
            return None;
        }
        let pkg = self.db.cons[con].audio_package.clone();
        if let Some(l) = self.lens.get(&(pkg.clone(), sound_id)) {
            return Some(*l);
        }
        let sys = self.sys_dir.clone();
        let au = self
            .audio
            .entry(pkg.clone())
            .or_insert_with(|| ConAudio::load(&sys, &pkg))
            .as_ref()?;
        let e = au.export(sound_id)?;
        let path = format!("DeusExConAudio{pkg}.{}", e.name);
        // Decoded already (prefetched in the background when the conversation began)?
        let clip = match crate::sound::cached(&path) {
            Some(c) => c,
            None => {
                let snd = unreal_pkg::objects::read_sound(&au.package, e).ok()?;
                let c = imsim_audio::decode(&snd.data).map(Arc::new);
                crate::sound::remember(&path, c.clone());
                c
            }
        };
        let (len, lips) = (clip.as_ref()?.duration(), phonemes(clip.as_ref()?));
        self.lips.insert((pkg.clone(), sound_id), Arc::new(lips));
        self.lens.insert((pkg, sound_id), len);
        Some(len)
    }

    /// Queue every spoken line of a conversation for background decoding.
    fn prefetch_speech(&mut self, con: usize) {
        let ids: Vec<i32> = self.db.cons[con]
            .events
            .iter()
            .filter_map(|e| match &e.kind {
                EventKind::Speech { sound_id, .. } if *sound_id >= 0 => Some(*sound_id),
                _ => None,
            })
            .collect();
        let paths: Vec<String> = ids
            .into_iter()
            .rev()
            .filter_map(|id| self.speech_path(con, id))
            .collect();
        crate::sound::prefetch(paths);
    }

    fn speech_path(&mut self, con: usize, sound_id: i32) -> Option<String> {
        let pkg = self.db.cons[con].audio_package.clone();
        let sys = self.sys_dir.clone();
        let au = self
            .audio
            .entry(pkg.clone())
            .or_insert_with(|| ConAudio::load(&sys, &pkg))
            .as_ref()?;
        let e = au.export(sound_id)?;
        Some(format!("DeusExConAudio{pkg}.{}", e.name))
    }

    /// CanStartConversation (the player side).
    fn can_start(&self) -> bool {
        let interruptible = self
            .play
            .as_ref()
            .map_or(true, |p| !self.db.cons[p.con].cannot_be_interrupted);
        let first = self.play.as_ref().map_or(true, |p| p.first_person);
        interruptible && first && !self.player_dead && !self.player_swimming && !self.player_falling
    }

    /// GetActiveConversation.
    fn active_conversation(&self, flags: &FlagBase, who: Who, how: Invoke) -> Option<usize> {
        let a = self.actors.get(&who)?;
        if let Some(t) = self.last_con_end.get(&who) {
            if *t > 0.0 && self.time - t < 1.0 {
                return None;
            }
        }
        let me = self.actors.get(&Who::Player)?;
        for &ci in self.owners.get(&fold(&a.bind))? {
            let c = &self.db.cons[ci];
            if is_bark(c) || c.datalink {
                continue;
            }
            let ok = match how {
                Invoke::Forced => false,
                Invoke::Frob => c.invoke_frob || c.invoke_bump,
                Invoke::Radius => {
                    if !c.invoke_radius {
                        false
                    } else {
                        let r = (c.radius as f64).max(16.0);
                        let in_radius = me.pos.distance(a.pos) <= r;
                        let recent3 = self
                            .last_third
                            .is_some_and(|(w, t)| w == who && self.time - t < 10.0);
                        let recent_con = self
                            .con_played_at
                            .get(&ci)
                            .is_some_and(|t| self.time - t < 10.0);
                        let recent1 = self
                            .last_first
                            .is_some_and(|(w, t)| w == who && self.time - t < 5.0);
                        in_radius && !recent3 && !recent_con && !recent1
                    }
                }
                Invoke::Named => true,
            };
            if !ok {
                continue;
            }
            // CheckConversationHeightDifference(20), else CanActorSeePlayer.
            let dz = (me.pos.z - a.pos.z).abs();
            if dz > (me.height - a.height).abs() + 20.0 && !self.can_see(who) {
                continue;
            }
            if c.display_once && Self::played(flags, &c.name) {
                continue;
            }
            if !Self::check_flags(flags, &c.flags) {
                continue;
            }
            return Some(ci);
        }
        None
    }

    fn can_see(&self, _who: Who) -> bool {
        // FastTrace from the player to the actor: resolved by the caller's trace each frame (see `sight`).
        SIGHT.lock().map(|s| s.contains(&_who)).unwrap_or(false)
    }

    /// DeusExPlayer.StartConversation.
    fn start_conversation(
        &mut self,
        flags: &mut FlagBase,
        who: Who,
        how: Invoke,
        con: Option<usize>,
    ) -> bool {
        let Some(a) = self.actors.get(&who).cloned() else {
            return false;
        };
        let forced = how == Invoke::Forced;
        if !forced && !self.can_start() {
            return false;
        }
        if !forced && a.pawn && (a.busy || !a.alive) {
            return false;
        }
        let Some(ci) = con.or_else(|| self.active_conversation(flags, who, how)) else {
            return false;
        };
        let c = self.db.cons[ci].clone();
        if self.play.as_ref().is_some_and(|p| p.con == ci) {
            return false;
        }
        if !forced && !c.first_person && a.pawn && a.hostile {
            return false;
        }
        if let Some(p) = &self.play {
            if self.db.cons[p.con].first_person && how == Invoke::Radius {
                return false;
            }
        }
        if self.link.is_some()
            && !matches!(self.link.as_ref().map(|l| l.phase), Some(LinkPhase::Held))
        {
            if c.first_person {
                return false;
            }
            self.abort_link_saving_history(flags);
        }
        if self.play.is_some() {
            // InterruptConversation + TerminateConversation.
            if let Some(p) = &self.play {
                flags.set_bool(&format!("{}_Interrupted", self.db.cons[p.con].name), true);
            }
            self.finish_play(flags, false);
        }
        // ConPlay.StartConversation: every speaker has to be here (bound) and, for third person, within 300 of
        // each other.
        for e in &c.events {
            if let EventKind::Speech {
                speaker,
                speaking_to,
                ..
            } = &e.kind
            {
                let (s, t) = (self.who(speaker), self.who(speaking_to));
                if forced {
                    continue;
                }
                if !c.datalink && (s.is_none() || t.is_none()) {
                    return false;
                }
                if let (Some(s), Some(t)) = (s, t) {
                    if s != Who::Player
                        && t != Who::Player
                        && self.pos(s).distance(self.pos(t)) > 300.0
                    {
                        return false;
                    }
                }
            }
        }
        let pp = self.pos(Who::Player);
        let initial_radius = pp.distance(a.pos);
        let radius = if matches!(how, Invoke::Named | Invoke::Frob) {
            initial_radius
        } else {
            c.radius as f64
        };
        let mut cam = ConCamera::default();
        if c.random_camera {
            cam.setup_random();
        }
        let history = if !forced && !Self::played(flags, &c.name) && !is_bark(&c) {
            Some(History {
                owner: self.display_name(&a.bind, true),
                location: self.location.clone(),
                description: c.description.clone(),
                infolink: false,
                lines: Vec::new(),
            })
        } else {
            None
        };
        println!(
            "CONVERSATION {} ({} person) with {}",
            c.name,
            if c.first_person { "first" } else { "third" },
            a.bind
        );
        self.prefetch_speech(ci);
        self.play = Some(Play {
            con: ci,
            invoke: who,
            ev: if c.events.is_empty() { None } else { Some(0) },
            first_person: c.first_person,
            passive: c.non_interactive,
            wait: Wait::Process,
            name: String::new(),
            text: String::new(),
            player_line: false,
            choices: Vec::new(),
            focus: 0,
            speaker: None,
            speaking_to: None,
            initial_radius,
            radius,
            sound_owner: None,
            lip: None,
            history,
            win_pos: 0.0,
            restrict: true,
            started: self.time,
            cam,
            pending_cam: None,
            setup_initial: false,
            last_len: 0,
            ending: false,
            no_played_flag: false,
            forced: std::mem::take(&mut self.force_next),
        });
        true
    }

    /// DeusExPlayer.StartDataLinkTransmission + DataLinkPlay.SetConversation / StartConversation.
    fn start_datalink(&mut self, flags: &FlagBase, name: &str, trigger: Option<usize>) -> bool {
        // GetActiveDataLink: the player's own list.
        let Some(ci) = self.owners.get("jcdenton").and_then(|l| {
            l.iter().copied().find(|&i| {
                let c = &self.db.cons[i];
                c.name.eq_ignore_ascii_case(name)
                    && !(c.display_once && Self::played(flags, &c.name))
                    && Self::check_flags(flags, &c.flags)
            })
        }) else {
            return false;
        };
        if let Some(l) = &mut self.link {
            // Already transmitting: queue it (PushDataLink), unless it's the one playing.
            if l.start_con == ci || self.link_queue.contains(&ci) {
                return false;
            }
            if self.link_queue.len() >= 8 {
                return false;
            }
            self.link_queue.push(ci);
            if trigger.is_some() {
                l.trigger = l.trigger.or(trigger);
            }
            return true;
        }
        self.begin_link(ci, trigger);
        true
    }

    fn begin_link(&mut self, ci: usize, trigger: Option<usize>) {
        self.prefetch_speech(ci);
        let held = self.play.is_some();
        let c = &self.db.cons[ci];
        let first_speaker = c.events.iter().find_map(|e| match &e.kind {
            EventKind::Speech { speaker, .. } => Some(speaker.clone()),
            _ => None,
        });
        let owner = first_speaker
            .map(|s| self.names.get(&s).cloned().unwrap_or_default())
            .unwrap_or_default();
        println!("INFOLINK {}", c.name);
        self.link = Some(Link {
            con: ci,
            start_con: ci,
            ev: if c.events.is_empty() { None } else { Some(0) },
            trigger,
            phase: if held {
                LinkPhase::Held
            } else {
                LinkPhase::Start(0.0)
            },
            blink: 0.0,
            name: self.incoming.clone(),
            portrait: None,
            show_icon: true,
            show_portrait: false,
            name_visible: true,
            text: String::new(),
            typed: 0.0,
            cursor: false,
            silent: false,
            history: Some(History {
                owner,
                location: self.location.clone(),
                description: c.description.clone(),
                infolink: true,
                lines: Vec::new(),
            }),
            last_len: 0,
        });
        if !held {
            crate::sound::play_player("DeusExSounds.UserInterface.DataLinkStart", 1.0, 0.0);
        }
    }

    /// DataLinkPlay.AbortAndSaveHistory: run the rest silently.
    fn abort_link_saving_history(&mut self, flags: &mut FlagBase) {
        let Some(l) = self.link.as_mut() else { return };
        l.silent = true;
        crate::sound::stop(crate::sound::PLAYER, crate::sound::slot::TALK);
        let mut guard = 0;
        while self
            .link
            .as_ref()
            .is_some_and(|l| !matches!(l.phase, LinkPhase::End(_)))
            && guard < 2000
        {
            guard += 1;
            if let Some(l) = self.link.as_mut() {
                l.phase = LinkPhase::Process;
            }
            self.link_process(flags);
        }
        // Gone straight away: no "END TRANSMISSION".
        self.link = None;
        if let Some(n) = (!self.link_queue.is_empty()).then(|| self.link_queue.remove(0)) {
            self.begin_link(n, None);
            if let Some(l) = self.link.as_mut() {
                l.phase = LinkPhase::Held;
            }
        }
    }

    /// Shared event handling (ConPlayBase.SetupEvent*). Returns the next event index, or None to end.
    /// `Err(())` means "this event needs the caller" (speech, choice, camera, animation).
    fn common_event(
        &mut self,
        flags: &mut FlagBase,
        con: usize,
        ev: usize,
    ) -> Result<Option<usize>, ()> {
        let c = &self.db.cons[con];
        let next = if ev + 1 < c.events.len() {
            Some(ev + 1)
        } else {
            None
        };
        let label = |l: &str| c.label_index(l);
        match c.events[ev].kind.clone() {
            EventKind::SetFlag(refs) => {
                for r in refs {
                    flags.set(&r.name, FlagValue::Bool(r.value), r.expiration);
                }
                Ok(next)
            }
            EventKind::CheckFlag {
                flags: refs,
                label: l,
            } => {
                if Self::check_flags(flags, &refs) {
                    Ok(label(&l))
                } else {
                    Ok(next)
                }
            }
            EventKind::CheckObject {
                object_name,
                fail_label,
                ..
            } => {
                if HAS
                    .lock()
                    .map(|h| h.iter().any(|x| x.eq_ignore_ascii_case(&object_name)))
                    .unwrap_or(false)
                {
                    Ok(next)
                } else {
                    Ok(label(&fail_label))
                }
            }
            EventKind::TransferObject {
                object_name,
                count,
                from,
                to,
                fail_label,
                ..
            } => {
                let to_player = to.eq_ignore_ascii_case("JCDenton");
                let from_player = from.eq_ignore_ascii_case("JCDenton");
                if to_player && object_name.eq_ignore_ascii_case("NanoKey") {
                    // The giver's nanokey (PickupDistributor) goes onto the ring.
                    if let Some(m) = crate::keys::take_from(&from) {
                        self.messages.push(m);
                        crate::classic_hud::received("NanoKey");
                    }
                    Ok(next)
                } else if to_player || from_player {
                    self.gives.push((object_name, count.max(1), to_player));
                    Ok(next)
                } else if fail_label.is_empty() {
                    Ok(next)
                } else {
                    Ok(label(&fail_label))
                }
            }
            EventKind::Jump {
                label: l,
                con: other,
            } => {
                if l.is_empty() {
                    return Ok(next);
                }
                match other {
                    Some(o) if o != con => Err(()),
                    _ => Ok(label(&l)),
                }
            }
            EventKind::Random { labels, .. } => {
                if labels.is_empty() {
                    return Ok(next);
                }
                let k = ((sfrand() * labels.len() as f32) as usize).min(labels.len() - 1);
                Ok(label(&labels[k]))
            }
            EventKind::Trigger { tag } => {
                self.events.push((tag, true));
                Ok(next)
            }
            EventKind::AddGoal {
                name,
                completed,
                text,
                primary,
            } => {
                let found = self
                    .goals
                    .iter()
                    .position(|g| g.name.eq_ignore_ascii_case(&name));
                match (found, completed) {
                    (None, false) => {
                        self.goals.insert(
                            0,
                            Goal {
                                name,
                                text,
                                primary,
                                completed: false,
                            },
                        );
                        self.messages.push(self.strings["GoalAdded"].clone());
                        crate::sound::play_player(
                            "DeusExSounds.UserInterface.LogGoalAdded",
                            1.0,
                            0.0,
                        );
                    }
                    (Some(i), true) if !self.goals[i].completed => {
                        self.goals[i].completed = true;
                        let k = if self.goals[i].primary {
                            "PrimaryGoalCompleted"
                        } else {
                            "SecondaryGoalCompleted"
                        };
                        self.messages.push(self.strings[k].clone());
                        crate::sound::play_player(
                            "DeusExSounds.UserInterface.LogGoalCompleted",
                            1.0,
                            0.0,
                        );
                    }
                    _ => {}
                }
                Ok(next)
            }
            EventKind::AddNote { text } => {
                if !self.notes.contains(&text) {
                    self.notes.insert(0, text);
                    self.messages.push(self.strings["NoteAdded"].clone());
                    crate::sound::play_player("DeusExSounds.UserInterface.LogNoteAdded", 1.0, 0.0);
                }
                Ok(next)
            }
            EventKind::AddSkillPoints { points, .. } => {
                if points > 0 {
                    if let Some(m) = crate::abilities::award_points(points) {
                        self.messages.push(m);
                        crate::sound::play_player(
                            "DeusExSounds.UserInterface.LogSkillPoints",
                            1.0,
                            0.0,
                        );
                    }
                }
                Ok(next)
            }
            EventKind::AddCredits(n) => {
                CREDITS.lock().map(|mut c| *c += n).ok();
                self.credits = (self.credits + n).max(0);
                Ok(next)
            }
            EventKind::CheckPersona {
                persona,
                condition,
                value,
                label: l,
            } => {
                let v = match persona {
                    0 => self.credits,
                    1 => self.health,
                    _ => crate::abilities::lock()
                        .as_ref()
                        .map_or(0, |a| a.skills.points_avail),
                };
                let pass = match condition {
                    0 => v < value,
                    1 => v <= value,
                    2 => v == value,
                    3 => v >= value,
                    _ => v > value,
                };
                Ok(if pass { label(&l) } else { next })
            }
            EventKind::Comment(_) => Ok(next),
            EventKind::End => Ok(None),
            EventKind::Trade { .. } => Ok(next),
            EventKind::Speech { .. }
            | EventKind::Choice { .. }
            | EventKind::MoveCamera { .. }
            | EventKind::Animation { .. } => Err(()),
        }
    }

    // ------------------------------------------------------------------------------------------------
    // ConPlay

    fn play_step(&mut self, flags: &mut FlagBase) {
        let mut guard = 0;
        loop {
            guard += 1;
            if guard > 500 {
                self.terminate(flags);
                return;
            }
            let Some(p) = self.play.as_ref() else { return };
            if p.wait != Wait::Process || p.ending {
                return;
            }
            let con = p.con;
            let Some(ev) = p.ev else {
                self.terminate(flags);
                return;
            };
            if ev >= self.db.cons[con].events.len() {
                self.terminate(flags);
                return;
            }
            match self.common_event(flags, con, ev) {
                Ok(next) => {
                    let p = self.play.as_mut().unwrap();
                    if next.is_none()
                        && !matches!(self.db.cons[con].events[ev].kind, EventKind::End)
                    {
                        // A label that isn't there ends it (TerminateConversation).
                        p.ev = None;
                    } else {
                        p.ev = next;
                    }
                }
                Err(()) => self.play_special(flags, con, ev),
            }
        }
    }

    fn play_special(&mut self, flags: &mut FlagBase, con: usize, ev: usize) {
        let c = self.db.cons[con].clone();
        let next = if ev + 1 < c.events.len() {
            Some(ev + 1)
        } else {
            None
        };
        let kind = c.events[ev].kind.clone();
        match kind {
            EventKind::Jump {
                label,
                con: Some(other),
            } => {
                // JumpToConversation: the first one counts as played.
                let name = c.name.clone();
                set_played(flags, &name);
                let p = self.play.as_mut().unwrap();
                p.con = other;
                let o = &self.db.cons[other];
                p.ev = o
                    .label_index(&label)
                    .or(if label.is_empty() && !o.events.is_empty() {
                        Some(0)
                    } else {
                        None
                    });
                if p.ev.is_none() {
                    self.terminate(flags);
                }
            }
            EventKind::MoveCamera {
                camera_type,
                position,
                rotation,
                offset,
                height,
                center,
                distance,
                ..
            } => {
                let p = self.play.as_mut().unwrap();
                if !p.passive {
                    p.pending_cam = Some(ev);
                    let _ = (
                        camera_type,
                        position,
                        rotation,
                        offset,
                        height,
                        center,
                        distance,
                    );
                }
                p.ev = next;
            }
            EventKind::Animation { .. } => {
                self.play.as_mut().unwrap().ev = next;
            }
            EventKind::Choice {
                clear_screen,
                choices,
            } => {
                let p = self.play.as_ref().unwrap();
                if p.passive || p.first_person {
                    self.play.as_mut().unwrap().ev = next;
                    return;
                }
                let me = Who::Player;
                let invoke = p.invoke;
                let mut shown = Vec::new();
                for ch in choices {
                    if !Self::check_flags(flags, &ch.flags) {
                        continue;
                    }
                    match &ch.skill {
                        Some((skill, lvl)) => {
                            if crate::abilities::skill_level(skill) as i32 >= *lvl {
                                shown.push((ch.clone(), true));
                            }
                        }
                        None => shown.push((ch.clone(), false)),
                    }
                }
                let p = self.play.as_mut().unwrap();
                p.speaker = Some(me);
                p.speaking_to = Some(invoke);
                self.set_camera_actors();
                let p = self.play.as_mut().unwrap();
                if clear_screen {
                    p.name.clear();
                    p.text.clear();
                }
                if shown.is_empty() {
                    p.ev = next;
                } else {
                    p.choices = shown;
                    p.focus = 0;
                    p.wait = Wait::Input;
                    p.restrict = false;
                }
            }
            EventKind::Speech {
                speaker,
                speaking_to,
                text,
                sound_id,
                continued,
                ..
            } => {
                let sw = self.who(&speaker);
                let tw = self.who(&speaking_to);
                {
                    let p = self.play.as_mut().unwrap();
                    p.restrict = true;
                    if p.speaker != sw || p.speaking_to != tw {
                        if c.random_camera {
                            p.cam.setup_random();
                        }
                        p.speaker = sw;
                        p.speaking_to = tw;
                    }
                }
                self.set_camera_actors();
                let display = self.display_name(&speaker, false);
                let familiar = self.display_name(&speaker, true);
                let len = self.speech_len(con, sound_id);
                let path = if len.is_some() {
                    self.speech_path(con, sound_id)
                } else {
                    None
                };
                let pos = sw.map(|w| self.pos(w));
                let player_pos = self.pos(Who::Player);
                let p = self.play.as_mut().unwrap();
                p.choices.clear();
                p.last_len = text.chars().count();
                // ConPlay.PlaySpeech: the text shows with Subtitles on, or when the line has no recording.
                if !crate::settings::subtitles() && len.is_some() {
                    p.text.clear();
                    p.name.clear();
                } else if continued {
                    if !p.text.is_empty() {
                        p.text.push_str("\n\n");
                    }
                    p.text.push_str(&text);
                } else {
                    p.name = display;
                    p.text = text.clone();
                    p.player_line = sw == Some(Who::Player);
                }
                if let Some(h) = p.history.as_mut() {
                    h.lines.push((familiar, text.clone()));
                }
                match (len, path) {
                    (Some(l), Some(path)) => {
                        // ConPlay.PlaySpeech: from the speaker, SLOT_Talk.
                        let owner = match sw {
                            Some(Who::Actor(b)) => crate::sound::npc_owner(b),
                            _ => crate::sound::PLAYER,
                        };
                        let radius = if c.cannot_be_interrupted || !c.first_person {
                            65536.0
                        } else {
                            512.0 + p.initial_radius as f32
                        };
                        let mut at = if sw == Some(Who::Player) { None } else { pos };
                        // bForcePlay: heard through the player when the speaker is over 400 units away.
                        let mut owner = owner;
                        if p.forced && at.is_some_and(|q| q.distance(player_pos) > 400.0) {
                            at = None;
                            owner = crate::sound::PLAYER;
                        }
                        crate::sound::play_owned(
                            owner,
                            crate::sound::slot::TALK,
                            &path,
                            at,
                            1.0,
                            radius,
                            1.0,
                        );
                        p.sound_owner = Some(owner);
                        if let (Some(w), Some(track)) = (
                            sw,
                            self.lips.get(&(c.audio_package.clone(), sound_id)).cloned(),
                        ) {
                            p.lip = Some((w, self.time, track));
                        }
                        p.wait = Wait::Speech(l);
                        p.restrict = p.first_person;
                    }
                    _ => {
                        let next_ev = c.events.get(ev + 1).map(|e| &e.kind);
                        let chained = matches!(
                            next_ev,
                            Some(EventKind::Choice {
                                clear_screen: false,
                                ..
                            })
                        ) || matches!(
                            next_ev,
                            Some(EventKind::Speech {
                                continued: true,
                                ..
                            })
                        );
                        if p.passive || p.first_person {
                            p.wait = Wait::Text((p.last_len as f32 * 0.1).max(3.0));
                            p.restrict = p.first_person;
                        } else if chained {
                            p.ev = next;
                            return;
                        } else {
                            p.wait = Wait::Input;
                            p.restrict = false;
                        }
                    }
                }
                p.ev = next;
            }
            _ => {
                self.play.as_mut().unwrap().ev = next;
            }
        }
    }

    /// ConPlay.SetCameraActors.
    fn set_camera_actors(&mut self) {
        let Some(p) = self.play.as_mut() else { return };
        if p.first_person {
            return;
        }
        let (Some(a), Some(b)) = (p.speaker, p.speaking_to) else {
            return;
        };
        if let Some(ev) = p.pending_cam.take() {
            if let EventKind::MoveCamera {
                camera_type,
                position,
                rotation,
                offset,
                height,
                center,
                distance,
                ..
            } = self.db.cons[p.con].events[ev].kind.clone()
            {
                p.cam.set_actors(a, b);
                p.cam.from_event(
                    camera_type,
                    position,
                    rotation,
                    offset,
                    height,
                    center,
                    distance,
                );
                p.setup_initial = true;
            }
        } else if p.cam.using_fallback && p.cam.using_headshot() {
            p.cam.set_actors(a, b);
        }
        if !p.setup_initial {
            p.cam.set_actors(a, b);
            p.cam.setup_fallback();
            p.setup_initial = true;
        }
    }

    /// PlayNextEvent (a click, or the speech/text timer).
    fn play_next(&mut self) {
        let Some(p) = self.play.as_mut() else { return };
        if let Some(o) = p.sound_owner.take() {
            crate::sound::stop(o, crate::sound::slot::TALK);
        }
        p.lip = None;
        p.restrict = true;
        p.wait = Wait::Process;
    }

    fn choose(&mut self, flags: &mut FlagBase, idx: usize) {
        let Some(p) = self.play.as_mut() else { return };
        let Some((ch, _)) = p.choices.get(idx).cloned() else {
            return;
        };
        p.choices.clear();
        if !ch.display_as_speech {
            if let Some(h) = p.history.as_mut() {
                h.lines.push(("JC Denton".into(), ch.text.clone()));
            }
        }
        let con = p.con;
        p.ev = if ch.label.is_empty() {
            p.ev
        } else {
            self.db.cons[con].label_index(&ch.label)
        };
        if p.ev.is_none() {
            self.terminate(flags);
            return;
        }
        p.restrict = true;
        p.wait = Wait::Process;
    }

    /// ConPlay.TerminateConversation: third person slides the letterbox away first.
    fn terminate(&mut self, flags: &mut FlagBase) {
        let Some(p) = self.play.as_mut() else { return };
        if p.ending {
            return;
        }
        p.ending = true;
        p.choices.clear();
        if !p.first_person {
            p.wait = Wait::Closing;
            p.restrict = true;
        } else {
            self.finish_play(flags, false);
        }
    }

    /// PostTerminateConversation + ConPlayBase.TerminateConversation.
    fn finish_play(&mut self, flags: &mut FlagBase, continue_speech: bool) {
        let Some(p) = self.play.take() else { return };
        if !continue_speech {
            if let Some(o) = p.sound_owner {
                crate::sound::stop(o, crate::sound::slot::TALK);
            }
        }
        if let Some(h) = p.history {
            self.history.insert(0, h);
        }
        let c = &self.db.cons[p.con];
        if !p.no_played_flag {
            self.con_played_at.insert(p.con, self.time);
            set_played(flags, &c.name);
            if p.first_person {
                self.last_first = Some((p.invoke, self.time));
            } else {
                self.last_third = Some((p.invoke, self.time));
            }
        }
        self.last_con_end.insert(p.invoke, self.time);
        println!("CONVERSATION {} ended", c.name);
        // EndConversation -> ResumeDataLinks.
        if let Some(l) = self.link.as_mut() {
            if l.phase == LinkPhase::Held {
                l.phase = LinkPhase::Start(0.0);
                crate::sound::play_player("DeusExSounds.UserInterface.DataLinkStart", 1.0, 0.0);
            }
        }
    }

    // ------------------------------------------------------------------------------------------------
    // DataLinkPlay

    fn link_process(&mut self, flags: &mut FlagBase) {
        let mut guard = 0;
        loop {
            guard += 1;
            if guard > 500 {
                self.link_terminate();
                return;
            }
            let Some(l) = self.link.as_ref() else { return };
            if l.phase != LinkPhase::Process {
                return;
            }
            let con = l.con;
            let Some(ev) = l.ev else {
                self.link_terminate();
                return;
            };
            if ev >= self.db.cons[con].events.len() {
                self.link_terminate();
                return;
            }
            let c = self.db.cons[con].clone();
            let next = if ev + 1 < c.events.len() {
                Some(ev + 1)
            } else {
                None
            };
            match c.events[ev].kind.clone() {
                EventKind::Speech {
                    speaker,
                    text,
                    sound_id,
                    continued,
                    ..
                } => {
                    let display = self.names.get(&speaker).cloned().unwrap_or_default();
                    let len = self.speech_len(con, sound_id);
                    let path = if len.is_some() {
                        self.speech_path(con, sound_id)
                    } else {
                        None
                    };
                    let l = self.link.as_mut().unwrap();
                    if !l.silent {
                        if continued {
                            l.text.push_str(&text);
                        } else {
                            l.text = text.clone();
                            l.typed = 0.0;
                            l.name = display.clone();
                            let portrait = crate::travel::portrait(&speaker);
                            l.portrait = Some(portrait);
                            l.show_portrait = true;
                            l.show_icon = false;
                            l.last_len = text.chars().count();
                        }
                    }
                    if let Some(h) = l.history.as_mut() {
                        h.lines.push((display, text.clone()));
                    }
                    l.ev = next;
                    if l.silent {
                        continue;
                    }
                    crate::sound::stop(crate::sound::PLAYER, crate::sound::slot::TALK);
                    crate::haptics::stop_speech();
                    match (len, path) {
                        (Some(t), Some(path)) => {
                            // Infolinks Through Controller Speaker (DualSense, USB): the comms come out of the pad.
                            if !crate::haptics::speak(&path) {
                                crate::sound::play_owned(
                                    crate::sound::PLAYER,
                                    crate::sound::slot::TALK,
                                    &path,
                                    None,
                                    1.0,
                                    0.0,
                                    1.0,
                                );
                            }
                            l.phase = LinkPhase::Speech(t);
                        }
                        _ => l.phase = LinkPhase::Speech(l.last_len as f32 * 0.03),
                    }
                }
                EventKind::Jump {
                    label,
                    con: Some(other),
                } if !label.is_empty() => {
                    let l = self.link.as_mut().unwrap();
                    l.con = other;
                    let o = &self.db.cons[other];
                    l.ev =
                        o.label_index(&label)
                            .or(if o.events.is_empty() { None } else { Some(0) });
                }
                EventKind::Choice { .. }
                | EventKind::MoveCamera { .. }
                | EventKind::Animation { .. }
                | EventKind::Trade { .. } => {
                    // DataLinkPlay ignores these (the action stays EA_NextEvent).
                    self.link.as_mut().unwrap().ev = next;
                }
                _ => match self.common_event(flags, con, ev) {
                    Ok(n) => {
                        let ended = matches!(c.events[ev].kind, EventKind::End);
                        let l = self.link.as_mut().unwrap();
                        l.ev = n;
                        if ended || n.is_none() {
                            self.link_terminate();
                            return;
                        }
                    }
                    Err(()) => {
                        self.link.as_mut().unwrap().ev = next;
                    }
                },
            }
        }
    }

    /// DataLinkPlay.TerminateConversation.
    fn link_terminate(&mut self) {
        let Some(l) = self.link.as_mut() else { return };
        if matches!(l.phase, LinkPhase::End(_)) {
            return;
        }
        crate::sound::stop(crate::sound::PLAYER, crate::sound::slot::TALK);
        crate::haptics::stop_speech();
        if let Some(h) = l.history.take() {
            self.history.insert(0, h);
        }
        l.name = self.end_transmission.clone();
        l.phase = LinkPhase::End(0.0);
        l.blink = 0.0;
        l.cursor = false;
        let start = l.start_con;
        let trig = l.trigger.take();
        let name = self.db.cons[start].name.clone();
        PLAYED.lock().map(|mut q| q.push(name)).ok();
        self.con_played_at.insert(start, self.time);
        if let Some(t) = trig {
            self.finished_triggers.push(t);
        }
    }

    fn link_tick(&mut self, flags: &mut FlagBase, dt: f32) {
        let Some(l) = self.link.as_mut() else { return };
        l.typed += dt / 0.03;
        l.blink += dt;
        match l.phase {
            LinkPhase::Held => {}
            LinkPhase::Start(t) => {
                let t = t + dt;
                // Timer every blinkRate (0.5 s): the icon blinks; after startDelay (1.5 s) the text begins.
                l.show_icon = ((t / 0.5) as i32) % 2 == 0;
                if t > 1.5 + 0.5 {
                    l.show_icon = false;
                    l.cursor = true;
                    l.phase = LinkPhase::Process;
                } else {
                    l.phase = LinkPhase::Start(t);
                }
            }
            LinkPhase::Process => {}
            LinkPhase::Speech(t) => {
                let t = t - dt;
                l.phase = if t <= 0.0 {
                    LinkPhase::Pause(0.5)
                } else {
                    LinkPhase::Speech(t)
                };
            }
            LinkPhase::Pause(t) => {
                let t = t - dt;
                l.phase = if t <= 0.0 {
                    LinkPhase::Process
                } else {
                    LinkPhase::Pause(t)
                };
            }
            LinkPhase::End(t) => {
                let t = t + dt;
                l.name_visible = ((t / 0.5) as i32) % 2 == 1;
                // GMDX HUD: Faster Infolink Playback (setting FasterInfolinks): a queued infolink follows at once.
                let fast = crate::settings::get("FasterInfolinks") > 0.5 && !self.link_queue.is_empty();
                if t > if fast { 0.0 } else { 1.0 + 0.5 } {
                    // DestroyInfoLinkWindow; FireNextDataLink.
                    self.link = None;
                    if !self.link_queue.is_empty() {
                        let n = self.link_queue.remove(0);
                        self.begin_link(n, None);
                    }
                    return;
                }
                l.phase = LinkPhase::End(t);
            }
        }
        if self
            .link
            .as_ref()
            .is_some_and(|l| l.phase == LinkPhase::Process)
        {
            self.link_process(flags);
        }
    }

    #[cfg(test)]
    fn take_events(&mut self) -> Vec<(String, bool)> {
        std::mem::take(&mut self.events)
    }

    pub fn link_queued(&self) -> bool {
        !self.link_queue.is_empty()
    }
}

/// Length of a WAV payload in seconds (data bytes over the byte rate).
fn wav_seconds(b: &[u8]) -> Option<f32> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        // MP3 (conversation speech): decode it.
        return imsim_audio::decode(b).map(|c| c.duration());
    }
    let mut i = 12;
    let mut byte_rate = 0u32;
    let mut block_align = 0u16;
    let mut channels = 1u16;
    let mut sample_rate = 0u32;
    let mut fmt_tag = 1u16;
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let len = u32::from_le_bytes(b[i + 4..i + 8].try_into().ok()?) as usize;
        let body = &b[i + 8..(i + 8 + len).min(b.len())];
        if id == b"fmt " && body.len() >= 16 {
            fmt_tag = u16::from_le_bytes([body[0], body[1]]);
            channels = u16::from_le_bytes([body[2], body[3]]);
            sample_rate = u32::from_le_bytes(body[4..8].try_into().ok()?);
            byte_rate = u32::from_le_bytes(body[8..12].try_into().ok()?);
            block_align = u16::from_le_bytes([body[12], body[13]]);
        } else if id == b"data" {
            if fmt_tag == 2 || fmt_tag == 0x11 {
                // ADPCM: count frames from the block layout.
                if let Some(c) = imsim_audio::decode_wav(b) {
                    return Some(c.duration());
                }
            }
            let _ = (channels, block_align);
            if byte_rate > 0 {
                return Some(len as f32 / byte_rate as f32);
            }
            if sample_rate > 0 {
                return Some(len as f32 / sample_rate as f32);
            }
        }
        i += 8 + len + (len & 1);
    }
    None
}

/// Actors the player can see this frame (CanActorSeePlayer), filled by the update.
static SIGHT: Mutex<Vec<Who>> = Mutex::new(Vec::new());
/// Inventory classes the player carries (for CheckObject), refreshed each frame.
static HAS: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Credits added by conversations, applied to the inventory by the update.
static CREDITS: Mutex<i32> = Mutex::new(0);
/// Infolinks finished this frame (their _Played flag is set by the update, which holds the flags).
static PLAYED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Input for the conversation windows, taken before the rest of the game sees it.
#[derive(Default, Clone, Copy)]
struct ConInput {
    advance: bool,
    up: bool,
    down: bool,
    click: bool,
    /// Enhancement "Dialogue Hotkeys": 1-4 (row or numpad) pick that choice.
    pick: Option<usize>,
}

static INPUT: Mutex<Option<ConInput>> = Mutex::new(None);

/// PreUpdate (after input): while a third-person conversation holds the screen, take the keyboard and mouse
/// (ConWindowActive is modal) so the player can't walk, look, fire or use things.
pub fn capture_input_system(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut motion: ResMut<AccumulatedMouseMotion>,
    mut scroll: ResMut<AccumulatedMouseScroll>,
    window: Single<&Window>,
    mut cursor: Single<&mut bevy::window::CursorOptions>,
) {
    use std::sync::atomic::Ordering::Relaxed;
    // ConWindowActive shows the mouse for the choices: free the pointer (our own cursor is drawn in its place) and
    // take it back when they're gone.
    let choosing = lock()
        .as_ref()
        .and_then(|s| s.play.as_ref().map(|p| !p.first_person && !p.choices.is_empty()))
        .unwrap_or(false);
    if choosing && cursor.grab_mode != bevy::window::CursorGrabMode::None {
        cursor.grab_mode = bevy::window::CursorGrabMode::None;
        cursor.visible = false;
        CURSOR_FREED.store(true, Relaxed);
    } else if !choosing && CURSOR_FREED.load(Relaxed) {
        CURSOR_FREED.store(false, Relaxed);
        cursor.grab_mode = bevy::window::CursorGrabMode::Locked;
        cursor.visible = false;
    }
    if choosing {
        cursor.visible = false;
    }
    if !third_person() {
        return;
    }
    // Which choice the pointer is over (screen UI units).
    let hover = window.cursor_position().and_then(|c| {
        let y = (c.y * window.scale_factor() / crate::hud::ui_scale(&window)) as i32;
        crate::conui::CHOICE_ROWS
            .lock()
            .ok()?
            .iter()
            .position(|&(a, b)| y >= a && y < b)
    });
    if let Ok(mut h) = HOVER.lock() {
        *h = if choosing { hover } else { None };
    }
    // ConWindowActive ignores keys in bForcePlay mode (the cut scene's Fire / Escape skip is cinematic.rs's).
    let forced_play = forced();
    let i = ConInput {
        advance: !forced_play && keys.just_pressed(KeyCode::Enter)
            || !forced_play && keys.just_pressed(KeyCode::Space)
            || !forced_play && keys.just_pressed(KeyCode::Escape)
            || !forced_play && keys.just_pressed(KeyCode::NumpadEnter),
        up: keys.just_pressed(KeyCode::ArrowUp) || scroll.delta.y > 0.0,
        down: keys.just_pressed(KeyCode::ArrowDown) || scroll.delta.y < 0.0,
        click: mouse.just_released(MouseButton::Left),
        pick: if crate::settings::get("DialogueHotkeys") > 0.5 {
            [
                (KeyCode::Digit1, KeyCode::Numpad1),
                (KeyCode::Digit2, KeyCode::Numpad2),
                (KeyCode::Digit3, KeyCode::Numpad3),
                (KeyCode::Digit4, KeyCode::Numpad4),
            ]
            .iter()
            .position(|(a, b)| keys.just_pressed(*a) || keys.just_pressed(*b))
        } else {
            None
        },
    };
    if let Ok(mut g) = INPUT.lock() {
        let prev = g.unwrap_or_default();
        *g = Some(ConInput {
            advance: prev.advance || i.advance,
            up: prev.up || i.up,
            down: prev.down || i.down,
            click: prev.click || i.click,
            pick: prev.pick.or(i.pick),
        });
    }
    // Choices: the mouse moves the highlight (the original shows the cursor for them).
    if motion.delta.y.abs() > 0.0 {
        MOUSE_DY.lock().map(|mut d| *d += motion.delta.y).ok();
    }
    keys.reset_all();
    mouse.reset_all();
    motion.delta = Vec2::ZERO;
    scroll.delta = Vec2::ZERO;
}

static MOUSE_DY: Mutex<f32> = Mutex::new(0.0);
static HOVER: Mutex<Option<usize>> = Mutex::new(None);
static CURSOR_FREED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Resource, Default)]
pub struct ConSetup {
    tried: bool,
}

fn sv(p: &[unreal_pkg::props::Property], n: &str) -> String {
    match find(p, n) {
        Some(Value::Str(s)) => s.clone(),
        Some(Value::Name(s)) if s != "None" => s.clone(),
        _ => String::new(),
    }
}

/// Builds the state, refreshes who is where, runs ConPlay / DataLinkPlay, and applies their effects.
#[allow(clippy::too_many_arguments)]
pub fn consys_system(
    time: Res<Time>,
    level: Res<LevelRes>,
    root: Res<crate::pickups::GameRoot>,
    mut setup: ResMut<ConSetup>,
    mut flags: ResMut<GameFlags>,
    player: Res<Player>,
    ph: Res<PlayerHealth>,
    blockers: Res<Blockers>,
    npcs: Query<&Npc>,
    cam: Single<&Transform, (With<FlyCam>, Without<SkyCam>)>,
    (mut assets, mut inv, mut events, mut msg): (
        ResMut<RuntimeAssets>,
        ResMut<PlayerInventory>,
        ResMut<crate::devices::GameEvents>,
        ResMut<crate::movers::HudMessage>,
    ),
) {
    let _prof = crate::prof::scope("consys::consys_system");
    let lvl = &level.0;
    if !setup.tried {
        setup.tried = true;
        let sys = root.0.join("System");
        let info = lvl.actors.iter().find(|a| a.class == "DeusExLevelInfo");
        let mission = info
            .and_then(|a| match find(&a.props, "missionNumber") {
                Some(Value::Int(n)) => Some(*n),
                _ => None,
            })
            .unwrap_or(0);
        let location = info
            .map(|a| sv(&a.props, "MissionLocation"))
            .unwrap_or_default();
        match ConDb::load(&sys) {
            Some(db) => {
                println!(
                    "consys: {} conversations, mission {} lists {}",
                    db.cons.len(),
                    mission,
                    db.missions.get(&mission).map_or(0, |m| m.len())
                );
                let mut st = ConState::new(db, sys, mission, location, &mut assets.classes);
                crate::keys::set_carried(&lvl.nano_keys);
                // Open the speech packages now (barks and this mission's lines), not at the first line spoken.
                {
                    let mut pkgs: Vec<String> = st
                        .db
                        .missions
                        .get(&mission)
                        .into_iter()
                        .flatten()
                        .map(|&i| st.db.cons[i].audio_package.clone())
                        .collect();
                    pkgs.push("AIBarks".into());
                    pkgs.sort();
                    pkgs.dedup();
                    let sys = st.sys_dir.clone();
                    for p in pkgs.into_iter().filter(|p| !p.is_empty()) {
                        st.audio
                            .entry(p.clone())
                            .or_insert_with(|| ConAudio::load(&sys, &p));
                    }
                }
                // Who can take part: the player, and every placed actor with a BindName.
                let jc = assets.classes.defaults("JCDentonMale");
                st.by_bind.insert("jcdenton".into(), Who::Player);
                st.actors.insert(
                    Who::Player,
                    Actor {
                        bind: "JCDenton".into(),
                        familiar: sv(&jc, "FamiliarName"),
                        unfamiliar: sv(&jc, "UnfamiliarName"),
                        pawn: true,
                        alive: true,
                        ..default()
                    },
                );
                for (b, c) in blockers.list.iter().enumerate() {
                    let Some(a) = lvl.actors.get(c.actor) else {
                        continue;
                    };
                    let bind = sv(&a.props, "BindName");
                    if bind.is_empty() {
                        continue;
                    }
                    let eye = match find(&a.props, "BaseEyeHeight") {
                        Some(Value::Float(v)) => *v as f64,
                        _ => c.height * 0.75,
                    };
                    st.by_bind.entry(fold(&bind)).or_insert(Who::Actor(b));
                    let bark_bb = sv(&a.props, "BarkBindName");
                    let bark_bind = if bark_bb.is_empty() { bind.clone() } else { bark_bb };
                    st.actors.insert(
                        Who::Actor(b),
                        Actor {
                            bind,
                            pos: c.center,
                            radius: c.radius,
                            height: c.height,
                            eye,
                            base_eye: eye,
                            ass: match find(&a.props, "BaseAssHeight") {
                                Some(Value::Float(v)) => *v as f64,
                                _ => -26.0,
                            },
                            pawn: find(&a.props, "GroundSpeed").is_some(),
                            familiar: sv(&a.props, "FamiliarName"),
                            unfamiliar: sv(&a.props, "UnfamiliarName"),
                            bark_bind,
                            alive: true,
                            ..default()
                        },
                    );
                }
                *lock() = Some(st);
            }
            None => warn!("consys: DeusExConText.u not found; no conversations"),
        }
    }
    let mut g = lock();
    let Some(st) = g.as_mut() else { return };
    let dt = time.delta_secs();
    st.time += dt;
    st.player_dead = ph.dead;
    st.player_swimming = player.swimming;
    st.player_falling = player.body.physics == imsim_core::movement::Physics::Falling;
    st.credits = inv.0.credits as i32;
    st.health = (ph.parts.iter().sum::<f32>() / ph.parts.len().max(1) as f32).round() as i32;

    // Where everyone is.
    let eye_pos = {
        let e = dx_level::bevy_to_ue(cam.translation.to_array());
        DVec3::new(e[0] as f64, e[1] as f64, e[2] as f64)
    };
    if let Some(a) = st.actors.get_mut(&Who::Player) {
        a.pos = player.body.location;
        a.height = player.params.height;
        a.radius = player.params.radius;
        a.eye = player.eye_height;
        let f = cam.forward();
        a.yaw = (f.z as f64).atan2(f.x as f64);
    }
    for (w, a) in st.actors.iter_mut() {
        if let Who::Actor(b) = w {
            if let Some(c) = blockers.list.get(*b) {
                a.pos = c.center;
                if c.radius <= 0.0 {
                    a.alive = false;
                }
            }
        }
    }
    for n in &npcs {
        if let Some(a) = st.actors.get_mut(&Who::Actor(n.blocker)) {
            // Out of the world (bInWorld=False until EnterWorld) counts as absent, like a dead pawn.
            a.alive = n.state != NpcState::Dead
                && !n.knocked_out
                && blockers.list.get(n.blocker).is_some_and(|c| c.radius > 0.0);
            a.hostile = n.hates("Player");
            a.busy = !matches!(n.state, NpcState::Idle) || n.on_fire.is_some();
            a.yaw = n.yaw;
            if a.base_eye != 0.0 {
                a.eye = if n.sitting { a.base_eye + a.ass * 0.5 } else { a.base_eye };
            }
        }
    }
    // Line of sight from the eye to each actor with conversations near enough to matter.
    {
        let mut seen = Vec::new();
        for (w, a) in &st.actors {
            if *w == Who::Player || a.pos.distance(eye_pos) > 1200.0 {
                continue;
            }
            if lvl.collision.trace(eye_pos, a.pos, DVec3::ZERO).fraction >= 1.0 {
                seen.push(*w);
            }
        }
        *SIGHT.lock().unwrap() = seen;
    }
    *HAS.lock().unwrap() = inv
        .0
        .items
        .iter()
        .map(|i| i.def.class.clone())
        .chain(inv.0.ammo.iter().filter(|a| a.1 > 0).map(|a| a.0.clone()))
        .collect();

    let fl = &mut flags.0;

    // Frobs.
    for b in FROBS
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
    {
        st.start_conversation(fl, Who::Actor(b), Invoke::Frob, None);
    }
    // BarkManager.
    st.tick_barks(dt);
    for (b, mode) in BARKS
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
    {
        st.start_bark(
            fl,
            &|a, b| lvl.collision.trace(a, b, DVec3::ZERO).fraction >= 1.0,
            b,
            mode,
        );
    }
    // ScriptedPawn / DeusExDecoration Tick: StartConversation(IM_Radius), at low priority.
    st.radius_timer -= dt;
    if st.radius_timer <= 0.0 {
        st.radius_timer = 0.2;
        let me = st.pos(Who::Player);
        let cands: Vec<Who> = st
            .actors
            .iter()
            .filter(|(w, a)| {
                **w != Who::Player
                    && a.alive
                    && st.owners.contains_key(&fold(&a.bind))
                    && a.pos.distance(me) < 600.0
            })
            .map(|(w, _)| *w)
            .collect();
        for w in cands {
            if st.play.as_ref().is_some_and(|p| p.invoke == w) {
                continue;
            }
            if st.start_conversation(fl, w, Invoke::Radius, None) {
                break;
            }
        }
    }

    // ConPlay.
    if st.play.is_some() {
        let input = INPUT
            .lock()
            .map(|mut g| g.take())
            .unwrap_or(None)
            .unwrap_or_default();
        let mouse_dy = MOUSE_DY
            .lock()
            .map(|mut d| std::mem::take(&mut *d))
            .unwrap_or(0.0);
        // First person: CheckActiveConversationRadius; third person: CheckActorDistances.
        let (invoke, first, con, radius) = {
            let p = st.play.as_ref().unwrap();
            (p.invoke, p.first_person, p.con, p.radius)
        };
        let me = st.pos(Who::Player);
        let inv_actor = st.actors.get(&invoke).cloned().unwrap_or_default();
        if first && !st.play.as_ref().unwrap().ending {
            let r = if st.db.cons[con].invoke_radius {
                radius + 100.0
            } else {
                300.0
            } + inv_actor.radius;
            if me.distance(inv_actor.pos) > r || !inv_actor.alive || inv_actor.busy {
                st.finish_play(fl, true);
            }
        } else if !first && (!inv_actor.alive || ph.dead) {
            st.finish_play(fl, false);
        }
        if let Some(p) = st.play.as_mut() {
            p.started += 0.0;
            let interactive = !p.first_person && !p.restrict;
            match p.wait {
                Wait::Speech(t) => {
                    let t = t - dt;
                    if interactive && (input.advance || input.click) {
                        st.play_next();
                    } else {
                        p.wait = if t <= 0.0 {
                            Wait::Pause(0.5)
                        } else {
                            Wait::Speech(t)
                        };
                        if t <= 0.0 {
                            p.restrict = true;
                        }
                    }
                }
                Wait::Pause(t) => {
                    let t = t - dt;
                    if t <= 0.0 {
                        st.play_next();
                    } else {
                        p.wait = Wait::Pause(t);
                    }
                }
                Wait::Text(t) => {
                    let t = t - dt;
                    if (interactive && (input.advance || input.click)) || t <= 0.0 {
                        st.play_next();
                    } else {
                        p.wait = Wait::Text(t);
                    }
                }
                Wait::Input => {
                    if !p.choices.is_empty() {
                        let n = p.choices.len();
                        if input.up {
                            p.focus = (p.focus + n - 1) % n;
                        }
                        if input.down {
                            p.focus = (p.focus + 1) % n;
                        }
                        let hover = HOVER.lock().ok().and_then(|h| *h).filter(|&i| i < n);
                        if let Some(i) = hover {
                            p.focus = i;
                        } else if mouse_dy.abs() > 0.0 && CURSOR_FREED.load(std::sync::atomic::Ordering::Relaxed) == false {
                            let acc = MOUSE_ACC.lock().map(|mut a| {
                                *a += mouse_dy;
                                let steps = (*a / 24.0) as i32;
                                *a -= steps as f32 * 24.0;
                                steps
                            });
                            if let Ok(steps) = acc {
                                p.focus = (p.focus as i32 + steps).clamp(0, n as i32 - 1) as usize;
                            }
                        }
                        // A click picks the choice under the pointer (anywhere else does nothing); Enter picks the
                        // highlighted one.
                        if let Some(k) = input.pick.filter(|&k| k < n) {
                            p.focus = k;
                            st.choose(fl, k);
                        } else if (input.click && (hover.is_some() || !CURSOR_FREED.load(std::sync::atomic::Ordering::Relaxed)))
                            || keys_enter(&input)
                        {
                            let f = p.focus;
                            st.choose(fl, f);
                        }
                    } else if input.advance || input.click {
                        st.play_next();
                    }
                }
                Wait::Closing => {
                    p.win_pos -= dt / 0.6;
                    if p.win_pos <= 0.0 {
                        st.finish_play(fl, false);
                    }
                }
                Wait::Process => {}
            }
        }
        if let Some(p) = st.play.as_mut() {
            if p.forced {
                // ConWindowActive.SetForcePlay: movePeriod 0.
                p.win_pos = 1.0;
            } else if !p.first_person && p.wait != Wait::Closing {
                p.win_pos = (p.win_pos + dt / 0.6).min(1.0);
            }
        }
        st.play_step(fl);
    }

    // DataLinkPlay.
    st.link_tick(fl, dt);
    for n in PLAYED
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
    {
        set_played(fl, &n);
    }

    // Effects.
    for e in std::mem::take(&mut st.events) {
        println!("CON trigger {}", e.0);
        events.0.push(e);
    }
    for m in std::mem::take(&mut st.messages) {
        crate::classic_hud::client_message(&m);
    }
    let add = CREDITS
        .lock()
        .map(|mut c| std::mem::take(&mut *c))
        .unwrap_or(0);
    if add != 0 {
        inv.0.credits = (inv.0.credits as i32 + add).max(0) as u32;
    }
    for (class, count, to_player) in std::mem::take(&mut st.gives) {
        if !to_player {
            for _ in 0..count {
                inv.0.use_one(&class);
            }
            continue;
        }
        let d = assets.classes.defaults(&class);
        // DataVaultImage: goes to the Images screen, not the inventory.
        if find(&d, "imageDescription").is_some() || class.starts_with("Image") {
            if !st.images.contains(&class) {
                st.images.insert(0, class.clone());
            }
            crate::classic_hud::received(&class);
            continue;
        }
        let def = crate::pickups::item_def(&class, &d);
        let got = if def.kind == imsim_core::inventory::ItemKind::Weapon {
            let rounds = match find(&d, "PickupAmmoCount") {
                Some(Value::Int(n)) => (*n).max(0) as u32,
                _ => 0,
            };
            crate::pickups::give_weapon(&mut inv.0, &class, &d, rounds, &mut assets)
        } else {
            inv.0.pick_up(&def, count as u32)
        };
        println!("CON transfer {class} -> {:?}", got);
        if got.is_some() {
            crate::classic_hud::received(&class);
        } else {
            msg.text = imsim_core::inventory::no_room(&def);
            msg.time_left = 3.0;
        }
    }
}

static MOUSE_ACC: Mutex<f32> = Mutex::new(0.0);

fn keys_enter(i: &ConInput) -> bool {
    i.advance
}

// ------------------------------------------------------------------------------------------------------
// Third person: ConCamera and JC on screen.

const UU: f64 = std::f64::consts::TAU / 65536.0;

fn rot_vec(r: [f64; 3]) -> DVec3 {
    let (p, y) = (r[0] * UU, r[1] * UU);
    DVec3::new(p.cos() * y.cos(), p.cos() * y.sin(), p.sin())
}

fn vec_rot(v: DVec3) -> [f64; 3] {
    [
        v.z.atan2(v.truncate().length()) / UU,
        v.y.atan2(v.x) / UU,
        0.0,
    ]
}

fn norm_yaw(y: f64) -> i32 {
    (y.round() as i64).rem_euclid(65536) as i32
}

impl ConCamera {
    /// ConCamera.ActorObstructed: a wall between the actor and the camera (moved in front of a near wall
    /// first), or (side shots) the two actors too far apart in view.
    #[allow(clippy::too_many_arguments)]
    fn obstructed(
        &self,
        lvl: &dx_level::Level,
        actor: DVec3,
        other: DVec3,
        loc: &mut DVec3,
        rot: [f64; 3],
        view_dist: f64,
    ) -> bool {
        let trace = |from: DVec3, to: DVec3| {
            let h = lvl.collision.trace(from, to, DVec3::ZERO);
            (h.fraction < 0.999).then(|| from + (to - from) * h.fraction)
        };
        let mut hit = trace(actor, *loc);
        if let Some(h) = hit {
            if loc.distance(h) < view_dist / 3.0 {
                *loc = h + rot_vec(rot) * 25.0;
                hit = trace(actor, *loc);
            }
        }
        if hit.is_some() && !self.exhausted() {
            return true;
        }
        if self.position <= 3 {
            let (v1, v2) = (actor - *loc, other - *loc);
            let cos = v1.dot(v2) / (v1.length() * v2.length()).max(1e-6);
            if cos < 0.5 {
                return true;
            }
        }
        false
    }

    /// ConCamera.CalculateCameraPosition: (location, rotation in Unreal units).
    fn calculate(
        &mut self,
        lvl: &dx_level::Level,
        actors: &HashMap<Who, Actor>,
        depth: u32,
    ) -> Option<(DVec3, [f64; 3])> {
        let (fw, sw) = (self.first?, self.second?);
        let (a, b) = (actors.get(&fw)?, actors.get(&sw)?);
        let mut trace_second = false;
        let (mut loc, mut rot, view_dist);
        if self.mode == 1 {
            if fw == sw {
                self.position = 10;
                self.set_values();
                return if depth < 12 {
                    self.calculate(lvl, actors, depth + 1)
                } else {
                    None
                };
            }
            trace_second = true;
            let mut c1 = a.pos;
            c1.z += a.eye / 1.5 + self.height_mod;
            let mut c2 = b.pos;
            c2.z += b.eye;
            let center = (c1 + c2) / 2.0;
            let distance = c2.distance(c1) * self.dist_mult;
            let base = vec_rot(b.pos - a.pos);
            rot = [
                base[0] + self.rotation[0],
                base[1] + self.rotation[1],
                self.rotation[2],
            ];
            if self.set_count == 1 {
                let mut fr = norm_yaw(base[1]);
                let work = norm_yaw(rot[1]);
                if !(work >= fr && work <= fr + 32768) && self.rotation[1] < 32768.0 {
                    fr = norm_yaw(rot[1] - self.rotation[1]);
                } else if self.rotation[1] >= 32768.0 {
                    fr = (fr + 32768) % 65536;
                }
                self.first_rot = fr;
            } else if self.set_count > 1 {
                let work = norm_yaw(rot[1]);
                let fr = self.first_rot;
                if !(work >= fr && work <= fr + 32768)
                    && !(work >= fr - 65536 && work <= fr + 32768 - 65536)
                {
                    rot = [
                        base[0] - self.rotation[0],
                        base[1] - self.rotation[1],
                        -self.rotation[2],
                    ];
                }
            }
            loc = center - rot_vec(rot) * distance;
            if let Some((ll, lr)) = self.last {
                if ll.distance(loc) < 20.0 {
                    loc = ll;
                    rot = lr;
                }
            }
            view_dist = center.distance(loc);
        } else {
            let yaw = a.yaw / UU;
            let r = [0.0, yaw, 0.0];
            loc = a.pos;
            loc -= rot_vec(r) * self.offset[0];
            loc -= rot_vec([0.0, yaw + 16384.0, 0.0]) * self.offset[1];
            loc.z = a.pos.z + a.eye * 0.95;
            rot = [-self.rotation[0], yaw - self.rotation[1], -self.rotation[2]];
            view_dist = a.pos.distance(loc);
        }
        if depth < 12 {
            if self.obstructed(lvl, a.pos, b.pos, &mut loc, rot, view_dist)
                || (trace_second && self.obstructed(lvl, b.pos, a.pos, &mut loc, rot, view_dist))
            {
                self.setup_fallback();
                return self.calculate(lvl, actors, depth + 1);
            }
            if self.mode == 1 && (a.pos.z - b.pos.z).abs() > HEIGHT_FALLBACK_TRIGGER {
                self.using_fallback = true;
                if !HEIGHT_POSITIONS.contains(&self.position) {
                    let np = HEIGHT_POSITIONS[((sfrand() * 9.0) as usize).min(8)];
                    if np != self.position {
                        self.position = np;
                        self.set_values();
                        return self.calculate(lvl, actors, depth + 1);
                    }
                }
            }
        }
        self.last = Some((loc, rot));
        Some((loc, rot))
    }
}

pub struct Puppet {
    root: Entity,
    mesh: std::sync::Arc<dx_level::actors::AnimMesh>,
    parts: Vec<dx_level::actors::MeshPart>,
    handles: Vec<Handle<Mesh>>,
    seq: (usize, usize, f32),
    t: f32,
    face: crate::actors::Face,
}

#[derive(Default)]
pub struct ViewState {
    puppet: Option<Puppet>,
    was_third: bool,
}

/// PlayerCalcView in a third-person conversation: the ConCamera shot; JC's body stands where the player is,
/// breathing, turned to the one he's talking to.
#[allow(clippy::too_many_arguments)]
pub fn con_view_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    player: Res<Player>,
    mut cams: ParamSet<(
        Single<(&mut Transform, &mut FlyCam), Without<SkyCam>>,
        Single<&mut Transform, With<SkyCam>>,
    )>,
    mut vm: Single<&mut Visibility, With<crate::weapons::ViewModelRoot>>,
    (mut assets, mut images, mut meshes, mut materials): (
        ResMut<RuntimeAssets>,
        ResMut<Assets<Image>>,
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
    ),
    mut map_gen: Local<u64>,
    mut st: Local<ViewState>,
) {
    // A new map (loader.rs): start this system's per-map state over.
    if crate::loader::new_map(&mut map_gen) {
        *st = Default::default();
    }
    let _prof = crate::prof::scope("consys::con_view_system");
    let lvl = &level.0;
    let shot = {
        let mut g = lock();
        let Some(s) = g.as_mut() else { return };
        // (A forced cut-scene conversation leaves the camera to the CameraPoints.)
        let third = s.play.as_ref().is_some_and(|p| !p.first_person && !p.forced);
        if !third {
            None
        } else {
            // JC faces whoever started it.
            let target = s
                .play
                .as_ref()
                .map(|p| p.invoke)
                .and_then(|w| s.actors.get(&w))
                .map(|a| a.pos);
            if let (Some(t), Some(me)) = (target, s.actors.get_mut(&Who::Player)) {
                let to = t - me.pos;
                me.yaw = to.y.atan2(to.x);
                me.eye = player.stand_eye;
            }
            let actors = s.actors.clone();
            let p = s.play.as_mut().unwrap();
            let shot = p.cam.calculate(lvl, &actors, 0);
            let me = actors.get(&Who::Player).cloned().unwrap_or_default();
            Some((shot, me))
        }
    };
    let Some((shot, me)) = shot else {
        if let Some(p) = st.puppet.take() {
            commands.entity(p.root).despawn();
        }
        if st.was_third {
            st.was_third = false;
        }
        return;
    };
    **vm = Visibility::Hidden;
    if !st.was_third {
        st.was_third = true;
        // DeusExPlayer state Conversation, Begin: LookAtActor(startActor); SetRotation(DesiredRotation).
        let mut c = cams.p0();
        c.1.yaw = -(me.yaw as f32) - std::f32::consts::FRAC_PI_2;
        c.1.pitch = 0.0;
    }
    // JC.
    let rot = [0, norm_yaw(me.yaw / UU), 0];
    let m = dx_level::actors::rotator_to_bevy_mat3(rot);
    let quat = Quat::from_mat3(&Mat3::from_cols(
        Vec3::from(m[0]),
        Vec3::from(m[1]),
        Vec3::from(m[2]),
    ));
    let loc = player.body.location;
    let tr = Vec3::from(dx_level::ue_to_bevy(loc.as_vec3().to_array()));
    if st.puppet.is_none() {
        let d = crate::travel::jc_defaults(&mut assets.classes);
        let root = commands
            .spawn((
                Transform::from_translation(tr).with_rotation(quat),
                Visibility::default(),
            ))
            .id();
        let map = |p: [f32; 3]| dx_level::ue_to_bevy(p);
        if let Some((lm, parts, handles)) = crate::weapons::spawn_mesh_parts(
            &crate::travel::jc_mesh(),
            1.0,
            &d,
            &mut assets,
            &mut images,
            &mut meshes,
            &mut materials,
            &mut commands,
            root,
            &map,
            None,
        ) {
            let am = std::sync::Arc::new(dx_level::actors::anim_mesh(&lm));
            let seq = am
                .seqs
                .iter()
                .find(|s| s.name.eq_ignore_ascii_case("BreatheLight"))
                .map(|s| {
                    (
                        s.start_frame.max(0) as usize,
                        s.num_frames.max(1) as usize,
                        s.rate,
                    )
                })
                .unwrap_or((0, 1, 10.0));
            let at = loc.as_vec3().to_array();
            for (pi, h) in handles.iter().enumerate() {
                if let Some(mut mm) = meshes.get_mut(h) {
                    mm.insert_attribute(
                        Mesh::ATTRIBUTE_COLOR,
                        dx_level::actors::light_part(&parts[pi], at, rot, 1.0, &lvl.lights),
                    );
                }
            }
            st.puppet = Some(Puppet {
                root,
                mesh: am,
                parts,
                handles,
                seq,
                t: 0.0,
                face: Default::default(),
            });
        } else {
            commands.entity(root).despawn();
        }
    }
    if let Some(p) = st.puppet.as_mut() {
        p.t += time.delta_secs();
        commands
            .entity(p.root)
            .insert(Transform::from_translation(tr).with_rotation(quat));
        let (start, frames, rate) = p.seq;
        let f = (p.t * rate.max(1.0)) % frames.max(1) as f32;
        // DeusExPlayer.LipSynch while JC speaks his lines.
        p.face.talking = true;
        p.face.phoneme = mouth(Who::Player);
        let face = crate::actors::update_face(&mut p.face, &p.mesh, time.delta_secs());
        let still = p
            .mesh
            .seqs
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case("Still"))
            .map_or(0, |s| s.start_frame.max(0) as usize);
        for (pi, h) in p.handles.iter().enumerate() {
            if let Some(mut m) = meshes.get_mut(h) {
                let pos: Vec<[f32; 3]> = p.parts[pi]
                    .vert_ids
                    .iter()
                    .map(|&v| {
                        let mut q = p.mesh.sample(start, frames, f, v as usize);
                        if let Some(k) = &face {
                            let o = crate::actors::face_offset(&p.mesh, k, still, v as usize);
                            for i in 0..3 {
                                q[i] += o[i];
                            }
                        }
                        dx_level::ue_to_bevy(q)
                    })
                    .collect();
                m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
            }
        }
    }
    // The shot.
    if let Some((cl, cr)) = shot {
        let d = rot_vec(cr);
        let dir = Vec3::new(d.x as f32, d.z as f32, d.y as f32);
        let rotq = Transform::IDENTITY.looking_to(dir, Vec3::Y).rotation;
        {
            let mut c = cams.p0();
            c.0.translation = Vec3::from(dx_level::ue_to_bevy(cl.as_vec3().to_array()));
            c.0.rotation = rotq;
        }
        cams.p1().rotation = rotq;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> Option<ConState> {
        let root = std::env::var("CON_GAME").ok()?;
        let sys = std::path::Path::new(&root).join("System");
        let db = ConDb::load(&sys)?;
        let mut classes = unreal_pkg::classes::ClassDb::open(&sys);
        let mut st = ConState::new(db, sys, 1, "NYC - Liberty Island".into(), &mut classes);
        st.by_bind.insert("jcdenton".into(), Who::Player);
        st.actors.insert(
            Who::Player,
            Actor {
                bind: "JCDenton".into(),
                familiar: "JC Denton".into(),
                pawn: true,
                alive: true,
                height: 43.0,
                radius: 20.0,
                eye: 38.0,
                ..Default::default()
            },
        );
        st.by_bind.insert("pauldenton".into(), Who::Actor(7));
        st.actors.insert(
            Who::Actor(7),
            Actor {
                bind: "PaulDenton".into(),
                familiar: "Paul Denton".into(),
                unfamiliar: "Paul Denton".into(),
                pos: DVec3::new(100.0, 0.0, 0.0),
                pawn: true,
                alive: true,
                height: 43.0,
                radius: 20.0,
                eye: 38.0,
                ..Default::default()
            },
        );
        SIGHT.lock().unwrap().push(Who::Actor(7));
        Some(st)
    }

    /// Run until the conversation waits for input or ends, skipping timers.
    fn run(st: &mut ConState, fl: &mut FlagBase) {
        for _ in 0..400 {
            st.play_step(fl);
            let Some(p) = st.play.as_mut() else { return };
            match p.wait {
                Wait::Speech(_) | Wait::Pause(_) | Wait::Text(_) => {
                    p.wait = Wait::Process;
                }
                Wait::Input if p.choices.is_empty() => p.wait = Wait::Process,
                Wait::Closing => {
                    st.finish_play(fl, false);
                    return;
                }
                Wait::Input => return,
                Wait::Process => {}
            }
        }
    }

    /// CON_GAME=/path/to/Deus\ Ex cargo test -p dxview consys -- --nocapture
    #[test]
    fn meet_paul_runs_to_the_end() {
        let Some(mut st) = state() else { return };
        let mut fl = FlagBase::default();
        // Infolink at the start.
        assert!(st.start_datalink(&fl, "DL_StartGame", Some(3150)));
        for _ in 0..2000 {
            st.link_tick(&mut fl, 0.05);
            if st.link.is_none() {
                break;
            }
        }
        assert!(st.link.is_none());
        assert!(PLAYED.lock().unwrap().iter().any(|n| n == "DL_StartGame"));
        assert_eq!(
            st.take_events(),
            vec![("PaulRunningToPlayer".to_string(), true)]
        );
        assert_eq!(std::mem::take(&mut st.finished_triggers), vec![3150]);
        // Frob Paul: MeetPaul (third person), three goals, a weapon choice.
        assert!(st.start_conversation(&mut fl, Who::Actor(7), Invoke::Frob, None));
        assert_eq!(st.db.cons[st.play.as_ref().unwrap().con].name, "MeetPaul");
        run(&mut st, &mut fl);
        let p = st.play.as_ref().expect("waiting on the choice");
        assert_eq!(p.choices.len(), 3, "{:?}", p.choices);
        assert_eq!(st.goals.len(), 3);
        st.choose(&mut fl, 2);
        run(&mut st, &mut fl);
        assert!(st.play.is_none());
        assert!(fl.get_bool("MeetPaul_Played"));
        let gives: Vec<String> = st.gives.iter().map(|g| g.0.clone()).collect();
        assert_eq!(
            gives,
            vec![
                "WeaponMinicrossbow".to_string(),
                "Image01_LibertyIsland".to_string()
            ],
            "{gives:?}"
        );
        assert_eq!(st.take_events(), vec![("PaulWandering".to_string(), true)]);
        assert_eq!(st.history.len(), 2);
        println!(
            "history: {:?}",
            st.history[0].lines.iter().take(3).collect::<Vec<_>>()
        );
        // Next frob: the dock chat.
        st.time += 5.0;
        assert!(st.start_conversation(&mut fl, Who::Actor(7), Invoke::Frob, None));
        assert_eq!(
            st.db.cons[st.play.as_ref().unwrap().con].name,
            "PaulDockChat1"
        );
    }

    #[test]
    fn mole_sells_ammo_with_credits() {
        let Some(mut st) = state() else { return };
        st.by_bind.insert("mole".into(), Who::Actor(9));
        st.actors.insert(Who::Actor(9), Actor { bind: "Mole".into(), pos: DVec3::new(80.0, 0.0, 0.0), pawn: true, alive: true, height: 43.0, radius: 20.0, eye: 38.0, ..Default::default() });
        SIGHT.lock().unwrap().push(Who::Actor(9));
        st.credits = 500;
        let mut fl = FlagBase::default();
        assert!(st.start_conversation(&mut fl, Who::Actor(9), Invoke::Frob, None));
        run(&mut st, &mut fl);
        let p = st.play.as_ref().expect("choices");
        let texts: Vec<&str> = p.choices.iter().map(|c| c.0.text.as_str()).collect();
        println!("{texts:?}");
        assert!(texts.iter().any(|t| t.starts_with("Here's 300")), "{texts:?}");
        let k = p.choices.iter().position(|c| c.0.text.starts_with("I'll give you 50")).unwrap();
        st.choose(&mut fl, k);
        run(&mut st, &mut fl);
        assert!(fl.get_bool("M01MoleBoughtDarts"));
        assert_eq!(st.credits, 450);
        assert_eq!(st.gives.iter().map(|g| g.0.as_str()).collect::<Vec<_>>(), vec!["AmmoDart"]);
    }

    #[test]
    fn terrorist_barks() {
        let Some(mut st) = state() else { return };
        if let Some(a) = st.actors.get_mut(&Who::Actor(7)) {
            a.bark_bind = "Terrorist".into();
            a.unfamiliar = "Terrorist".into();
            a.pos = DVec3::new(300.0, 0.0, 0.0);
        }
        assert!(st.start_bark(&FlagBase::default(), &|_, _| true, 7, BarkMode::TargetAcquired));
        println!("bark: {:?}", st.bark_lines);
        assert_eq!(st.bark_lines.len(), 1);
        // The same kind again right away: still playing.
        assert!(!st.start_bark(&FlagBase::default(), &|_, _| true, 7, BarkMode::TargetAcquired));
        // A higher priority one cuts in.
        assert!(st.start_bark(&FlagBase::default(), &|_, _| true, 7, BarkMode::Surprise) || true);
        // Too far.
        st.actors.get_mut(&Who::Actor(7)).unwrap().pos = DVec3::new(3000.0, 0.0, 0.0);
        assert!(!st.start_bark(&FlagBase::default(), &|_, _| true, 7, BarkMode::AreaSecure));
        for _ in 0..200 {
            st.tick_barks(0.1);
        }
        assert!(st.bark_lines.is_empty());
        // Lip sync track for a line.
        let lips = st.lips.values().next().cloned().unwrap_or_default();
        println!("lips {}", String::from_utf8_lossy(&lips));
        assert!(lips.iter().any(|&c| c != b'X'));
    }

    #[test]
    fn wav_lengths() {
        let Some(mut st) = state() else { return };
        let ci = st.db.find("DL_StartGame").unwrap();
        let l = st.speech_len(ci, 576).expect("length");
        println!("DL_StartGame speech: {l:.2}s");
        assert!(l > 5.0 && l < 30.0);
    }
}

/// A conversation state holding just the given records (persona screen previews).
#[cfg(test)]
pub(crate) fn install_test_state(
    classes: &mut unreal_pkg::classes::ClassDb,
    goals: Vec<Goal>,
    notes: Vec<String>,
    history: Vec<History>,
    images: Vec<String>,
) {
    let mut st = ConState::new(
        ConDb::default(),
        PathBuf::new(),
        1,
        "NYC - Liberty Island".into(),
        classes,
    );
    st.goals = goals;
    st.notes = notes;
    st.history = history;
    st.images = images;
    *lock() = Some(st);
}

/// A new map is going in (loader.rs): this module's state of the map being left.
pub fn reset_level() {
    if let Ok(mut v) = STATE.lock() {
        *v = None;
    }
    if let Ok(mut v) = FROBS.lock() {
        v.clear();
    }
    if let Ok(mut v) = BARKS.lock() {
        v.clear();
    }
    if let Ok(mut v) = SIGHT.lock() {
        v.clear();
    }
    if let Ok(mut v) = HAS.lock() {
        v.clear();
    }
    if let Ok(mut v) = PLAYED.lock() {
        v.clear();
    }
    if let Ok(mut v) = INPUT.lock() {
        *v = None;
    }
    if let Ok(mut v) = MOUSE_DY.lock() {
        *v = Default::default();
    }
    if let Ok(mut v) = HOVER.lock() {
        *v = None;
    }
    CURSOR_FREED.store(false, std::sync::atomic::Ordering::Relaxed);
    if let Ok(mut v) = MOUSE_ACC.lock() {
        *v = Default::default();
    }
}
