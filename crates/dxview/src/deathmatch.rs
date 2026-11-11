//! Deathmatch and Team Deathmatch on the original's five multiplayer maps (DXMP_*.dx), after DeusExMPGame /
//! DeathMatchGame / TeamDMGame. The host is the referee: he keeps the scoreboard, ends the match, chooses the next
//! map and decides who gets a pickup; everyone else is told (netplay.rs carries it).
//!
//! Ported rules:
//! * FindPlayerStart / EvaluatePlayerStart: Deathmatch skips bTeamOnlyStart points and weights every start by
//!   -10000 / distance to each other player (+ a small random factor); Team Deathmatch skips bNonTeamOnlyStart
//!   points, costs 300 / 250 / 175 / 100 / 25 by distance bands, x5 for enemies and as a bonus for friends; a
//!   start within 100 of anyone is never used. Ours on top: a team start (TeamNumber) of your own team scores +150,
//!   so teams start on their sides.
//! * Killed: a death resets the streak; dying by your own hand or the world costs a point (never below 0); a kill
//!   is a point and a streak step; killing a teammate costs a point. CheckVictoryConditions: frag limit (the first
//!   player, or team, to reach it) and time limit (the most points when the clock runs out; ties go on deaths, then
//!   streak - GetWinningPlayer); a draw is possible. GameOver: the results for NewMapDelay seconds, then the next
//!   map.
//! * GiveInitialInventory (pistol + 50 rounds, medkit, lockpick, multitool), RestartPlayer (everything reset,
//!   skills at MPSkillStartLevel, InitialAugs augmentations at full level in AugPrefs order) and Reward
//!   (AugsPerKill more for each kill; Team Deathmatch none).
//! * TeamDMGame.GetAutoTeam (the smaller team; UNATCO on a tie), ArePlayersAllied, fFriendlyFireMult.
//!
//! Ours: the damage table (data/deathmatch.ron: what a hit does to a player by weapon and body region), the
//! per-map pickup lists (one GEP gun per map, one of each heavy weapon, commons spread, respawn timers by tier),
//! auto-balance, the match's time limit together with its frag limit, a map rotation.

use bevy::prelude::*;
use glam::DVec3;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

// ---------------------------------------------------------------------------------------------------------------
// Rules.

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub enum Mode {
    #[default]
    Coop,
    Deathmatch,
    Team,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Coop => "Co-op",
            Mode::Deathmatch => "Deathmatch",
            Mode::Team => "Team Deathmatch",
        }
    }
    pub fn versus(self) -> bool {
        self != Mode::Coop
    }
}

/// What happens when a match ends (after the results).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum After {
    /// The next map of the five (DXMapList.GetNextMap).
    #[default]
    NextMap,
    SameMap,
    /// Back to the host's room screen (everyone waits there).
    Room,
}

/// Which deathmatch: the original multiplayer patch as it was (Classic: no mods, its own rules, maps and HUD), or
/// ours (Modern: the Modern look, Augmentique characters, balanced pickups and the damage table).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub enum Family {
    Classic,
    #[default]
    Modern,
}

/// The original's game types (menuchoice_gametype).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub enum ClassicGame {
    /// DeathMatchGame (customizable).
    #[default]
    Deathmatch,
    /// BasicTeamDMGame: every augmentation and every skill at Master from the start.
    BasicTeam,
    /// AdvTeamDMGame: a few skill points and augmentations at the start, more for each kill.
    AdvancedTeam,
    /// TeamDMGame: "Custom Team Deathmatch" (customizable).
    CustomTeam,
}

impl ClassicGame {
    pub const ALL: [ClassicGame; 4] = [ClassicGame::Deathmatch, ClassicGame::BasicTeam, ClassicGame::AdvancedTeam, ClassicGame::CustomTeam];
    pub fn class(self) -> &'static str {
        crate::classicmp::GAME_CLASSES[self as usize]
    }
    /// menuchoice_gametype.gameNames.
    pub fn name(self) -> &'static str {
        ["Deathmatch", "Basic Team Deathmatch", "Advanced Team Deathmatch", "Custom Team Deathmatch"][self as usize]
    }
    pub fn team(self) -> bool {
        self != ClassicGame::Deathmatch
    }
    /// bCustomizable: the host's settings count (else the game type's own numbers).
    pub fn customizable(self) -> bool {
        matches!(self, ClassicGame::Deathmatch | ClassicGame::CustomTeam)
    }
}

/// DeusExMPGame.VictoryCondition.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub enum Victory {
    #[default]
    Frags,
    Time,
}

/// A Classic match's DeusExMPGame settings.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ClassicRules {
    pub game: ClassicGame,
    pub skills_avail: i32,
    pub skills_per_kill: i32,
    pub initial_augs: i32,
    pub augs_per_kill: i32,
    /// MPSkillStartLevel (1 Trained .. 3 Master).
    pub skill_level: i32,
    pub augs_allowed: bool,
    pub victory: Victory,
    /// ScoreToWin: kills, or minutes with a time limit.
    pub score_to_win: u32,
    /// fFriendlyFireMult (0..1).
    pub friendly_fire: f32,
}

impl Default for ClassicRules {
    fn default() -> Self {
        ClassicRules::of(ClassicGame::Deathmatch)
    }
}

impl ClassicRules {
    /// A game type with its own numbers (the class's defaults, read from the game).
    pub fn of(game: ClassicGame) -> Self {
        let d = crate::classicmp::game_defaults(game.class());
        ClassicRules {
            game,
            skills_avail: d.skills_avail,
            skills_per_kill: d.skills_per_kill,
            initial_augs: d.initial_augs,
            augs_per_kill: d.augs_per_kill,
            skill_level: d.skill_level.clamp(0, 3),
            augs_allowed: d.augs_allowed,
            victory: Victory::Frags,
            score_to_win: d.score_to_win.max(1) as u32,
            friendly_fire: d.friendly_fire,
        }
    }
}

/// The host room's match settings, as every player gets them.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DmRules {
    pub mode: Mode,
    /// Kills to win (0 = no frag limit).
    pub frag_limit: u32,
    /// Minutes (0 = none).
    pub time_limit: u32,
    /// The map file's stem ("DXMP_Cathedral").
    pub map: String,
    pub after: After,
    /// Team Deathmatch: a dead player of the bigger team (by two or more) joins the smaller one.
    pub auto_balance: bool,
    /// Team Deathmatch: teammates' shots hurt (x friendly_fire_mult).
    pub friendly_fire: bool,
    /// Classic (the original patch) or Modern (ours).
    pub family: Family,
    /// Classic: the game type and its settings.
    pub classic: ClassicRules,
}

impl Default for DmRules {
    fn default() -> Self {
        DmRules {
            mode: Mode::Coop,
            frag_limit: 20,
            time_limit: 10,
            map: "DXMP_Cathedral".into(),
            after: After::NextMap,
            auto_balance: true,
            friendly_fire: false,
            family: Family::Modern,
            classic: ClassicRules { game: ClassicGame::Deathmatch, skills_avail: 2000, skills_per_kill: 2000, initial_augs: 2, augs_per_kill: 2, skill_level: 1, augs_allowed: true, victory: Victory::Frags, score_to_win: 20, friendly_fire: 0.5 },
        }
    }
}

impl DmRules {
    pub fn classic(&self) -> bool {
        self.family == Family::Classic
    }
    /// The key map data names this match by (MapEntry parts' `modes`).
    pub fn mode_key(&self) -> &'static str {
        match (self.family, self.classic.game, self.mode) {
            (Family::Classic, ClassicGame::Deathmatch, _) => "cdm",
            (Family::Classic, ClassicGame::BasicTeam, _) => "cbasic",
            (Family::Classic, ClassicGame::AdvancedTeam, _) => "cadv",
            (Family::Classic, ClassicGame::CustomTeam, _) => "ctdm",
            (Family::Modern, _, Mode::Team) => "tdm",
            _ => "dm",
        }
    }
    /// The match's name ("Advanced Team Deathmatch").
    pub fn title(&self) -> &'static str {
        if self.classic() {
            self.classic.game.name()
        } else {
            self.mode.name()
        }
    }
}

/// Match Settings' choices (settings DmFragLimit / DmTimeLimit, as indices).
pub const FRAG_LIMITS: [u32; 8] = [0, 5, 10, 15, 20, 25, 30, 50];
pub const TIME_LIMITS: [u32; 8] = [0, 5, 10, 15, 20, 30, 45, 60];

/// The host's rules for a room of `mode`, from his Match Settings.
pub fn rules_from_settings(mode: Mode, friendly_fire: bool) -> DmRules {
    let idx = |k: &str, n: usize| (crate::settings::get(k).max(0.0) as usize).min(n - 1);
    let maps = &data().maps;
    DmRules {
        mode,
        frag_limit: FRAG_LIMITS[idx("DmFragLimit", FRAG_LIMITS.len())],
        time_limit: TIME_LIMITS[idx("DmTimeLimit", TIME_LIMITS.len())],
        map: maps[idx("DmMap", maps.len())].file.clone(),
        after: match idx("DmAfter", 3) {
            1 => After::SameMap,
            2 => After::Room,
            _ => After::NextMap,
        },
        auto_balance: crate::settings::get("DmAutoBalance") > 0.5,
        friendly_fire,
        family: Family::Modern,
        classic: ClassicRules::default(),
    }
}

/// The host's Classic rules for a game type, from his Classic Match Settings (settings Cmp*; the game types that
/// aren't customizable keep their own numbers, as ResetNonCustomizableOptions does).
pub fn classic_rules_from_settings(game: ClassicGame) -> DmRules {
    let mut c = ClassicRules::of(game);
    let set = |k: &str| crate::settings::has(k).then(|| crate::settings::get(k));
    // (Stored as the Match Settings choice's index: menus::classic_match_choices.)
    if game.customizable() {
        if let Some(v) = set("CmpSkillsAvail") {
            c.skills_avail = (v.round() as i32).clamp(0, 20) * 500;
        }
        if let Some(v) = set("CmpSkillsPerKill") {
            c.skills_per_kill = (v.round() as i32).clamp(0, 20) * 500;
        }
        if let Some(v) = set("CmpInitialAugs") {
            c.initial_augs = (v.round() as i32).clamp(0, 9);
        }
        if let Some(v) = set("CmpAugsPerKill") {
            c.augs_per_kill = (v.round() as i32).clamp(0, 9);
        }
        if let Some(v) = set("CmpSkillLevel") {
            c.skill_level = (v.round() as i32 + 1).clamp(1, 3);
        }
        if game.team() {
            if let Some(v) = set("CmpFriendly") {
                c.friendly_fire = (v.round() / 10.0).clamp(0.0, 1.0);
            }
        }
    }
    if let Some(v) = set("CmpVictory") {
        c.victory = if v > 0.5 { Victory::Time } else { Victory::Frags };
    }
    if let Some(v) = set("CmpScore") {
        c.score_to_win = (v.round() as u32 + 1).clamp(1, 40);
    }
    let idx = |k: &str, n: usize| (crate::settings::get(k).max(0.0) as usize).min(n - 1);
    let maps = &data().maps;
    let (frag_limit, time_limit) = match c.victory {
        Victory::Frags => (c.score_to_win, 0),
        Victory::Time => (0, c.score_to_win),
    };
    DmRules {
        mode: if game.team() { Mode::Team } else { Mode::Deathmatch },
        frag_limit,
        time_limit,
        map: maps[idx("DmMap", maps.len())].file.clone(),
        // DeusExMPGame.bChangeLevels: the next map of the list.
        after: After::NextMap,
        auto_balance: false,
        friendly_fire: game.team() && c.friendly_fire > 0.0,
        family: Family::Classic,
        classic: c,
    }
}

static RULES: Mutex<Option<DmRules>> = Mutex::new(None);

/// The room's match rules (None = not in a room, or co-op).
pub fn rules() -> Option<DmRules> {
    let g = RULES.lock().unwrap_or_else(|e| e.into_inner());
    g.clone().or_else(env_rules)
}

/// DXVIEW_DM=dm|tdm|cdm|cbasic|cadv|ctdm[,map][,frags][,minutes]: deathmatch rules without the menus (test runs;
/// the c* ones are Classic game types).
fn env_rules() -> Option<DmRules> {
    let v = env_val!("DXVIEW_DM")?;
    let mut it = v.split(',');
    let classic = |g: ClassicGame| {
        let mut r = DmRules { mode: if g.team() { Mode::Team } else { Mode::Deathmatch }, family: Family::Classic, classic: ClassicRules::of(g), ..DmRules::default() };
        r.frag_limit = r.classic.score_to_win;
        r.time_limit = 0;
        r
    };
    let mut r = match it.next()? {
        "tdm" | "team" => DmRules { mode: Mode::Team, ..DmRules::default() },
        "dm" => DmRules { mode: Mode::Deathmatch, ..DmRules::default() },
        "cdm" => classic(ClassicGame::Deathmatch),
        "cbasic" => classic(ClassicGame::BasicTeam),
        "cadv" => classic(ClassicGame::AdvancedTeam),
        "ctdm" => classic(ClassicGame::CustomTeam),
        _ => return None,
    };
    crate::classicmp::set_active(r.classic());
    if let Some(m) = it.next().filter(|m| !m.is_empty()) {
        r.map = m.trim_end_matches(".dx").to_string();
    }
    if let Some(f) = it.next().and_then(|f| f.parse().ok()) {
        r.frag_limit = f;
    }
    if let Some(t) = it.next().and_then(|t| t.parse().ok()) {
        r.time_limit = t;
    }
    Some(r)
}

/// The room's rules changed (the host's Host Room, or the host's Rules for a joiner). Co-op clears them.
pub fn set_rules(r: Option<DmRules>) {
    let r = r.filter(|r| r.mode.versus());
    // A Classic room plays with the original multiplayer values (classicmp.rs).
    crate::classicmp::set_active(r.as_ref().is_some_and(|r| r.classic()));
    *RULES.lock().unwrap_or_else(|e| e.into_inner()) = r;
}

/// A Classic deathmatch room (the original patch's rules).
pub fn classic() -> bool {
    rules().is_some_and(|r| r.classic())
}

/// The player's body takes hits as the original's net game does (three regions): a Classic match being played.
pub fn mp_body() -> bool {
    playing() && classic()
}

pub fn mode() -> Mode {
    rules().map_or(Mode::Coop, |r| r.mode)
}

/// A versus room (deathmatch or team deathmatch) - whether or not its map is up yet.
pub fn versus() -> bool {
    mode().versus()
}

pub fn team_mode() -> bool {
    mode() == Mode::Team
}

static ON_MAP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A deathmatch is being played here: a versus room on one of its maps.
pub fn playing() -> bool {
    ON_MAP.load(std::sync::atomic::Ordering::Relaxed) && versus()
}

/// The map is one of the multiplayer maps.
pub fn is_dm_map(name: &str) -> bool {
    let stem = name.trim_end_matches(".dx");
    data().maps.iter().any(|m| m.file.eq_ignore_ascii_case(stem))
}

// ---------------------------------------------------------------------------------------------------------------
// Data (data/deathmatch.ron).

#[derive(Deserialize, Clone, Copy, Debug)]
pub struct Hit {
    pub body: f32,
    pub head: f32,
    pub legs: f32,
}

/// One multiplayer map's data (data/deathmatch.ron `maps`). Everything after `keep` is per mode: a part applies when
/// its `modes` list is empty or holds the match's mode_key() ("dm", "tdm", "cdm", "cbasic", "cadv", "ctdm"; later
/// modes add theirs) or its family ("classic" / "modern").
#[derive(Deserialize, Clone, Debug)]
pub struct MapEntry {
    pub file: String,
    pub title: String,
    /// Modern deathmatch: the placed pickups kept, by actor name (Classic keeps every one).
    pub keep: Vec<String>,
    /// Spawn sets that replace the map's PlayerStarts.
    #[serde(default)]
    pub spawns: Vec<SpawnSet>,
    /// Play-area blockers: cylinders that stop players (cut a map down for a small mode).
    #[serde(default)]
    pub blockers: Vec<AreaBlock>,
    /// Objective spots (flags, control points, uplinks, extraction zones, data drops).
    #[serde(default)]
    pub objectives: Vec<ObjectiveSpot>,
    /// Loot tables: at each match, `picks` of the `spots` get an item drawn from `items` by weight.
    #[serde(default)]
    pub loot: Vec<LootTable>,
    /// Codes made fresh each match for keypads / computers (written in datacubes and notes by the modes).
    #[serde(default)]
    pub codes: Vec<CodeSpot>,
}

/// A position in map units.
pub type Pos = (f32, f32, f32);

#[derive(Deserialize, Clone, Debug)]
pub struct SpawnSet {
    pub name: String,
    #[serde(default)]
    pub modes: Vec<String>,
    pub points: Vec<SpawnSpot>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct SpawnSpot {
    pub at: Pos,
    /// Unreal yaw (65536 a turn).
    #[serde(default)]
    pub yaw: i32,
    /// Only this team spawns here (team games).
    #[serde(default)]
    pub team: Option<u8>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct AreaBlock {
    pub name: String,
    #[serde(default)]
    pub modes: Vec<String>,
    pub at: Pos,
    pub radius: f32,
    pub height: f32,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ObjectiveSpot {
    pub name: String,
    /// "flag", "point", "uplink", "extract", "data" (the modes that use them say).
    pub kind: String,
    #[serde(default)]
    pub modes: Vec<String>,
    pub at: Pos,
    #[serde(default = "default_objective_radius")]
    pub radius: f32,
    #[serde(default)]
    pub team: Option<u8>,
}

fn default_objective_radius() -> f32 {
    160.0
}

#[derive(Deserialize, Clone, Debug)]
pub struct LootTable {
    pub name: String,
    #[serde(default)]
    pub modes: Vec<String>,
    /// Where the items may go: placed actors by name (their location) or points.
    #[serde(default)]
    pub actors: Vec<String>,
    #[serde(default)]
    pub points: Vec<Pos>,
    /// (class, weight).
    pub items: Vec<(String, u32)>,
    pub picks: u32,
}

#[derive(Deserialize, Clone, Debug)]
pub struct CodeSpot {
    pub name: String,
    #[serde(default)]
    pub modes: Vec<String>,
    /// Keypads / computers (actor names) that take the code.
    pub devices: Vec<String>,
    #[serde(default = "default_digits")]
    pub digits: u8,
}

fn default_digits() -> u8 {
    4
}

/// Every mode key a map part may name.
pub const MODE_KEYS: [&str; 8] = ["dm", "tdm", "cdm", "cbasic", "cadv", "ctdm", "classic", "modern"];

/// A part with this `modes` list is in this match.
pub fn part_applies(modes: &[String], key: &str, family: Family) -> bool {
    let fam = if family == Family::Classic { "classic" } else { "modern" };
    modes.is_empty() || modes.iter().any(|m| m.eq_ignore_ascii_case(key) || m.eq_ignore_ascii_case(fam))
}

/// The map's data, by file (with or without ".dx").
pub fn map_entry(file: &str) -> Option<&'static MapEntry> {
    let stem = file.trim_end_matches(".dx");
    data().maps.iter().find(|m| m.file.eq_ignore_ascii_case(stem))
}

/// The match's spawn set on this map, as starts (None: the map's own PlayerStarts).
pub fn match_starts(file: &str) -> Option<Vec<Start>> {
    let r = rules()?;
    let key = r.mode_key();
    let set = map_entry(file)?.spawns.iter().find(|s| part_applies(&s.modes, key, r.family))?;
    Some(
        set.points
            .iter()
            .map(|p| Start {
                pos: DVec3::new(p.at.0 as f64, p.at.1 as f64, p.at.2 as f64),
                yaw: p.yaw,
                team_only: p.team.is_some(),
                non_team_only: false,
                team: p.team.unwrap_or(0),
            })
            .collect(),
    )
}

/// The match's blockers, objectives and loot tables on this map.
pub fn match_blockers(file: &str) -> Vec<&'static AreaBlock> {
    let Some(r) = rules() else { return Vec::new() };
    map_entry(file).map(|m| m.blockers.iter().filter(|b| part_applies(&b.modes, r.mode_key(), r.family)).collect()).unwrap_or_default()
}

pub fn match_objectives(file: &str) -> Vec<&'static ObjectiveSpot> {
    let Some(r) = rules() else { return Vec::new() };
    map_entry(file).map(|m| m.objectives.iter().filter(|b| part_applies(&b.modes, r.mode_key(), r.family)).collect()).unwrap_or_default()
}

pub fn match_loot(file: &str) -> Vec<&'static LootTable> {
    let Some(r) = rules() else { return Vec::new() };
    map_entry(file).map(|m| m.loot.iter().filter(|b| part_applies(&b.modes, r.mode_key(), r.family)).collect()).unwrap_or_default()
}

/// The match's codes on this map, made from `seed` (the host picks the seed and sends it): (code spot, digits).
pub fn match_codes(file: &str, seed: u64) -> Vec<(String, String)> {
    let Some(r) = rules() else { return Vec::new() };
    let Some(m) = map_entry(file) else { return Vec::new() };
    let mut x = seed ^ 0x9E37_79B9_7F4A_7C15;
    m.codes
        .iter()
        .filter(|c| part_applies(&c.modes, r.mode_key(), r.family))
        .map(|c| {
            let code: String = (0..c.digits.clamp(1, 8))
                .map(|_| {
                    // splitmix64
                    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
                    let mut z = x;
                    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
                    char::from(b'0' + ((z ^ (z >> 31)) % 10) as u8)
                })
                .collect();
            (c.name.clone(), code)
        })
        .collect()
}

/// The match's loot: (table, spot index into actors then points, class), drawn from `seed`.
pub fn roll_loot(t: &LootTable, seed: u64) -> Vec<(usize, String)> {
    let spots = t.actors.len() + t.points.len();
    let total: u64 = t.items.iter().map(|i| i.1 as u64).sum();
    if spots == 0 || total == 0 {
        return Vec::new();
    }
    let mut x = seed | 1;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let mut order: Vec<usize> = (0..spots).collect();
    for i in (1..order.len()).rev() {
        order.swap(i, (next() % (i as u64 + 1)) as usize);
    }
    order
        .into_iter()
        .take(t.picks as usize)
        .map(|spot| {
            let mut w = next() % total;
            let class = t.items.iter().find(|i| {
                if w < i.1 as u64 {
                    true
                } else {
                    w -= i.1 as u64;
                    false
                }
            });
            (spot, class.map_or_else(String::new, |c| c.0.clone()))
        })
        .collect()
}

#[derive(Deserialize, Debug)]
pub struct Data {
    pub weapons: HashMap<String, Hit>,
    pub traced: Vec<String>,
    pub kinds: HashMap<String, f32>,
    pub friendly_fire_mult: f32,
    pub respawn: HashMap<String, f32>,
    pub tiers: HashMap<String, String>,
    pub start_kit: Vec<(String, u32)>,
    pub dm_augs: (u32, u32),
    pub team_augs: (u32, u32),
    pub aug_prefs: Vec<String>,
    pub skill_level: usize,
    pub skin_unlocks: Vec<u32>,
    pub new_map_delay: f32,
    pub maps: Vec<MapEntry>,
    /// Operative profile (dmchar.rs): credits and weapon mastery.
    #[serde(default)]
    pub profile: crate::dmchar::ProfileTuning,
}

pub fn data() -> &'static Data {
    static D: OnceLock<Data> = OnceLock::new();
    D.get_or_init(|| ron::from_str(include_str!("data/deathmatch.ron")).expect("data/deathmatch.ron"))
}

/// The maps (file stem, title), in rotation order.
pub fn maps() -> Vec<(String, String)> {
    data().maps.iter().map(|m| (m.file.clone(), m.title.clone())).collect()
}

pub fn map_title(file: &str) -> String {
    let stem = file.trim_end_matches(".dx");
    data().maps.iter().find(|m| m.file.eq_ignore_ascii_case(stem)).map_or_else(|| stem.to_string(), |m| m.title.clone())
}

/// DXMapList.GetNextMap: the map after `file` in the list.
pub fn next_map(file: &str) -> String {
    let m = &data().maps;
    let stem = file.trim_end_matches(".dx");
    let i = m.iter().position(|e| e.file.eq_ignore_ascii_case(stem)).map_or(0, |i| (i + 1) % m.len());
    m[i].file.clone()
}

/// The respawn time of a pickup class (None: not a deathmatch pickup).
pub fn respawn_secs(class: &str) -> Option<f32> {
    let d = data();
    let tier = d.tiers.get(class)?;
    d.respawn.get(tier).copied()
}

// ---------------------------------------------------------------------------------------------------------------
// Damage: what a hit does to a deathmatch player.

/// A player's body regions in multiplayer (DeusExPlayer.GetMPHitLocation: head, torso + arms, legs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    Head,
    Body,
    Legs,
}

/// The region of a hit at `local` (the hit point minus the body's collision centre; z up) on a body of half
/// height `h` and radius `r`.
pub fn region(local: DVec3, h: f64, r: f64) -> Region {
    if local.z > h * 0.78 && local.truncate().length() < r * 0.5 {
        Region::Head
    } else if local.z < 0.0 {
        Region::Legs
    } else {
        Region::Body
    }
}

/// A hit on this player in a deathmatch: (damage, offset) to hand to PlayerHealth::damage, whose part rules
/// (head and torso x2, legs x1 spilling into the torso, Shot x CombatDifficulty) give back the table's points.
/// `weapon`: the attacker's weapon class ("" unknown), `raw`: the damage his game worked out, `local`: the hit
/// point relative to this body's centre (None: unknown - a blast, a burn: taken on the body).
pub fn player_damage(weapon: &str, kind: &str, raw: f32, local: Option<DVec3>, h: f64, r: f64) -> (f32, DVec3) {
    let d = data();
    let reg = local.map_or(Region::Body, |l| region(l, h, r));
    let at = match reg {
        Region::Head => DVec3::new(0.0, 0.0, h * 0.9),
        Region::Body => DVec3::new(0.0, 0.0, h * 0.4),
        Region::Legs => DVec3::new(0.0, if local.is_some_and(|l| l.y > 0.0) { 1.0 } else { -1.0 }, -h * 0.5),
    };
    let traced = d.traced.iter().any(|k| k == kind);
    let points = match d.weapons.get(weapon).filter(|_| traced) {
        Some(t) => match reg {
            Region::Head => t.head,
            Region::Body => t.body,
            Region::Legs => t.legs,
        },
        None => {
            // Blasts and the rest: the game's own damage, scaled; on the torso that is doubled like a body shot.
            let k = d.kinds.get(kind).copied().unwrap_or(1.0);
            raw * k * if reg == Region::Legs { 1.0 } else { 2.0 }
        }
    };
    // PlayerHealth doubles head and torso hits and multiplies shots by CombatDifficulty: undo both.
    let mut feed = if reg == Region::Legs { points } else { points / 2.0 };
    if matches!(kind, "Shot" | "AutoShot") {
        feed /= crate::travel::combat_difficulty().max(0.01);
    }
    (feed.max(if raw > 0.0 { 1.0 } else { 0.0 }), at)
}

/// Team Deathmatch: the share of a teammate's hit that lands (0 without friendly fire).
pub fn friendly_mult() -> f32 {
    match rules() {
        Some(r) if r.classic() => {
            if r.mode == Mode::Team {
                r.classic.friendly_fire
            } else {
                1.0
            }
        }
        Some(r) if r.mode == Mode::Team && r.friendly_fire => data().friendly_fire_mult,
        Some(r) if r.mode == Mode::Team => 0.0,
        _ => 1.0,
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Spawning.

/// A PlayerStart of the map: (centre, yaw in Unreal units, team-only, non-team-only, team number).
#[derive(Clone, Debug)]
pub struct Start {
    pub pos: DVec3,
    pub yaw: i32,
    pub team_only: bool,
    pub non_team_only: bool,
    pub team: u8,
}

pub fn starts(lvl: &dx_level::Level) -> Vec<Start> {
    use unreal_pkg::props::{find, Value};
    lvl.actors
        .iter()
        .filter(|a| a.class == "PlayerStart")
        .map(|a| {
            let l = a.location();
            let b = |n: &str| matches!(find(&a.props, n), Some(Value::Bool(true)));
            let yaw = match find(&a.props, "Rotation") {
                Some(Value::Rotator(r)) => r[1],
                _ => 0,
            };
            let team = match find(&a.props, "TeamNumber") {
                Some(Value::Byte(t)) => *t,
                _ => 0,
            };
            Start { pos: DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64), yaw, team_only: b("bTeamOnlyStart"), non_team_only: b("bNonTeamOnlyStart"), team }
        })
        .collect()
}

fn frand() -> f64 {
    crate::sound::frand() as f64
}

/// FindPlayerStart: the best start for a player of `team` (Team Deathmatch) with the other players at `others`
/// (position, team). (centre, yaw radians on the Unreal circle).
pub fn spawn_point(lvl: &dx_level::Level, team: Option<u8>, others: &[(DVec3, Option<u8>)]) -> Option<(DVec3, f64)> {
    let team_game = team_mode();
    let all = match_starts(&lvl.name).filter(|v| !v.is_empty()).unwrap_or_else(|| starts(lvl));
    let usable: Vec<&Start> = all.iter().filter(|s| if team_game { !s.non_team_only } else { !s.team_only }).collect();
    let pool = if usable.is_empty() { all.iter().collect() } else { usable };
    let mut best: Option<(&Start, f64)> = None;
    for s in pool {
        let mut w = frand();
        let mut too_close = false;
        for (p, t) in others {
            let dist = (*p - s.pos).length();
            if dist < 100.0 {
                too_close = true;
            }
            if team_game {
                let cost = match dist {
                    d if d < 200.0 => 300.0,
                    d if d < 400.0 => 250.0,
                    d if d < 800.0 => 175.0,
                    d if d < 1600.0 => 100.0,
                    d if d < 3200.0 => 25.0,
                    _ => 0.0,
                };
                if t.is_some() && *t == team {
                    w += cost;
                } else {
                    w -= 5.0 * cost;
                }
            } else {
                w -= 10000.0 / (dist + 0.1);
            }
        }
        if team_game && team == Some(s.team) {
            w += 150.0;
        }
        if too_close {
            w = -100000.0;
        }
        if best.is_none_or(|b| w > b.1) {
            best = Some((s, w));
        }
    }
    best.map(|(s, _)| (s.pos, s.yaw as f64 / 65536.0 * std::f64::consts::TAU))
}

/// The view yaw (FlyCam) that faces along an Unreal yaw.
pub fn cam_yaw(unreal_yaw: f64) -> f32 {
    -(unreal_yaw as f32) - std::f32::consts::FRAC_PI_2
}

/// RestartPlayer + GiveInitialInventory: a fresh inventory (the start kit), skills at the MP start level and the
/// mode's starting augmentations.
pub fn reset_player(inv: &mut imsim_core::inventory::Inventory, classes: &mut unreal_pkg::classes::ClassDb) {
    let d = data();
    *inv = imsim_core::inventory::Inventory::default();
    for (class, n) in &d.start_kit {
        let p = classes.defaults(class);
        if p.is_empty() {
            continue;
        }
        let def = crate::pickups::item_def(class, &p);
        if def.kind == imsim_core::inventory::ItemKind::Weapon {
            let _ = inv.pick_up(&def, 1);
            if let Some(unreal_pkg::props::Value::Path(a)) = unreal_pkg::props::find(&p, "AmmoName") {
                let ammo = a.rsplit('.').next().unwrap_or("").to_string();
                if !ammo.is_empty() && !ammo.eq_ignore_ascii_case("AmmoNone") {
                    inv.ammo.retain(|x| x.0 != ammo);
                    inv.ammo.push((ammo, *n));
                }
            }
        } else {
            let _ = inv.pick_up(&def, (*n).max(1));
        }
    }
    let mut g = crate::abilities::lock();
    if let Some(a) = g.as_mut() {
        for s in a.skills.list.iter_mut() {
            s.level = d.skill_level.min(3);
        }
        let _ = a.augs.deactivate_all();
        // ResetAugmentations: everything but the default ones (light, IFF, infolink) comes out.
        for aug in a.augs.list.iter_mut().filter(|x| x.def.location != imsim_core::augs::LOC_DEFAULT) {
            aug.has = false;
            aug.active = false;
            aug.level = 0;
            aug.boosted = false;
        }
        for (k, l) in a.augs.locs.iter_mut().enumerate() {
            if k != imsim_core::augs::LOC_DEFAULT as usize {
                l.count = 0;
            }
        }
    }
    drop(g);
    let (start, _) = if team_mode() { d.team_augs } else { d.dm_augs };
    grant_augs(start);
}

/// AmmoCrate.RestockWeapon for every weapon carried: its first ammo type up to the weapon's PickupAmmoCount
/// (mpPickupAmmoCount when the weapon has one). The message (AmmoReceived).
pub fn restock(inv: &mut imsim_core::inventory::Inventory, classes: &mut unreal_pkg::classes::ClassDb) -> String {
    use unreal_pkg::props::{find, Value};
    let weapons: Vec<String> = inv.items.iter().filter(|i| i.def.kind == imsim_core::inventory::ItemKind::Weapon).map(|i| i.def.class.clone()).collect();
    for w in weapons {
        let d = classes.defaults(&w);
        let ammo = match find(&d, "AmmoNames").or(find(&d, "AmmoName")) {
            Some(Value::Path(a)) if !a.ends_with("AmmoNone") && a != "None" => a.rsplit('.').next().unwrap_or("").to_string(),
            _ => continue,
        };
        let want = match find(&d, "mpPickupAmmoCount").or(find(&d, "PickupAmmoCount")) {
            Some(Value::Int(n)) => (*n).max(0) as u32,
            _ => continue,
        };
        match inv.ammo.iter_mut().find(|x| x.0 == ammo) {
            Some(x) => x.1 = x.1.max(want),
            None => inv.ammo.push((ammo, want)),
        }
    }
    let msg = match find(&classes.defaults("ammocrate"), "AmmoReceived") {
        Some(Value::Str(s)) if !s.is_empty() => s.clone(),
        _ => "Ammo restocked".into(),
    };
    msg
}

/// DeusExPlayer.GrantAugs: the next `n` augmentations of AugPrefs not yet installed, at their highest level.
pub fn grant_augs(n: u32) -> Vec<String> {
    let mut out = Vec::new();
    let mut left = n;
    let mut g = crate::abilities::lock();
    let Some(a) = g.as_mut() else { return out };
    for pref in &data().aug_prefs {
        if left == 0 {
            break;
        }
        let Some(i) = a.augs.index(pref) else { continue };
        if a.augs.list[i].has {
            continue;
        }
        if a.augs.give(pref, false) == imsim_core::augs::Give::Added {
            let aug = &mut a.augs.list[i];
            aug.level = aug.def.max_level;
            out.push(aug.def.name.clone());
            left -= 1;
        }
    }
    out
}

/// Reward: a kill in Deathmatch brings AugsPerKill more augmentations.
pub fn reward_kill() {
    let (_, per) = if team_mode() { data().team_augs } else { data().dm_augs };
    let got = grant_augs(per);
    if !got.is_empty() {
        crate::classic_hud::client_message(&format!("Augmentation{} granted: {}", if got.len() > 1 { "s" } else { "" }, got.join(", ")));
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The scoreboard.

/// TEAM_UNATCO / TEAM_NSF.
pub const TEAM_UNATCO: u8 = 0;
pub const TEAM_NSF: u8 = 1;

pub fn team_name(t: u8) -> &'static str {
    if t == TEAM_NSF {
        "Team NSF"
    } else {
        "Team UNATCO"
    }
}

/// One player's line on the scoreboard (PlayerReplicationInfo: Score, Deaths, Streak, Team; ours: heals given,
/// pawns killed in co-op).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ScoreRow {
    pub id: u8,
    pub name: String,
    pub team: u8,
    pub score: i32,
    pub deaths: u32,
    pub streak: u32,
    pub heals: u32,
    pub npc_kills: u32,
}

/// The end of a match, for the results screen.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Ending {
    /// "X has won the match!" / "Team NSF has won the match!" / "The match is a draw!".
    pub headline: String,
    /// "The match ended with X taking out Y with a pistol!" (frag limit only).
    pub last_kill: String,
    pub winner_id: Option<u8>,
    pub winner_team: Option<u8>,
    pub next_map: String,
    pub after: After,
}

/// What the host sends everyone: the rows, the clock, the end.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Board {
    pub rows: Vec<ScoreRow>,
    /// Seconds of the time limit left (None: no time limit).
    pub time_left: Option<f32>,
    pub frag_limit: u32,
    pub ended: Option<Ending>,
    /// Seconds until the next map once ended.
    pub next_in: f32,
}

struct HostMatch {
    board: Board,
    /// Seconds played.
    clock: f32,
    notified_minute: bool,
    notified_frags: u32,
    /// Board changed since netplay last took it.
    dirty: bool,
    travel: Option<String>,
}

static HOST: Mutex<Option<HostMatch>> = Mutex::new(None);
static MY_ID: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// This player's id in the session (the host is 0; netplay sets a joiner's).
pub fn my_id() -> u8 {
    MY_ID.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_my_id(id: u8) {
    MY_ID.store(id, std::sync::atomic::Ordering::Relaxed);
}
static SEEN: Mutex<Option<Board>> = Mutex::new(None);

/// The board as this machine knows it (host: his own; joiner: the host's last).
pub fn board() -> Option<Board> {
    if let Some(h) = HOST.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        return Some(h.board.clone());
    }
    SEEN.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// A joiner: the host's board came.
pub fn set_board(b: Board) {
    let ended_now = b.ended.is_some() && SEEN.lock().map(|s| s.as_ref().is_none_or(|o| o.ended.is_none())).unwrap_or(false);
    *SEEN.lock().unwrap_or_else(|e| e.into_inner()) = Some(b);
    if ended_now {
        on_match_end();
    }
}

/// The host: the board when it changed since last asked (to send to everyone).
pub fn take_board() -> Option<Board> {
    let mut g = HOST.lock().unwrap_or_else(|e| e.into_inner());
    let h = g.as_mut()?;
    if !h.dirty {
        return None;
    }
    h.dirty = false;
    Some(h.board.clone())
}

/// The host: a new match on a freshly loaded map (scores to zero, teams kept).
pub fn host_begin(players: &[(u8, String)]) {
    let r = rules().unwrap_or_default();
    let mut g = HOST.lock().unwrap_or_else(|e| e.into_inner());
    let old_teams: Vec<(u8, u8)> = g.as_ref().map(|h| h.board.rows.iter().map(|r| (r.id, r.team)).collect()).unwrap_or_default();
    let mut board = Board { frag_limit: r.frag_limit, time_left: (r.time_limit > 0).then_some(r.time_limit as f32 * 60.0), ..Default::default() };
    for (id, name) in players {
        let team = old_teams.iter().find(|t| t.0 == *id).map(|t| t.1).unwrap_or_else(|| auto_team(&board.rows));
        board.rows.push(ScoreRow { id: *id, name: name.clone(), team, ..Default::default() });
    }
    *g = Some(HostMatch { board, clock: 0.0, notified_minute: false, notified_frags: 0, dirty: true, travel: None });
    drop(g);
    *SEEN.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The host is no longer refereeing (left the room, back to co-op).
pub fn host_end() {
    *HOST.lock().unwrap_or_else(|e| e.into_inner()) = None;
    *SEEN.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// GetAutoTeam: the team with fewer players (UNATCO on a tie).
fn auto_team(rows: &[ScoreRow]) -> u8 {
    let u = rows.iter().filter(|r| r.team == TEAM_UNATCO).count();
    let n = rows.iter().filter(|r| r.team == TEAM_NSF).count();
    if u > n {
        TEAM_NSF
    } else {
        TEAM_UNATCO
    }
}

fn with_host<R>(f: impl FnOnce(&mut HostMatch) -> R) -> Option<R> {
    let mut g = HOST.lock().unwrap_or_else(|e| e.into_inner());
    g.as_mut().map(|h| {
        let r = f(h);
        h.dirty = true;
        r
    })
}

/// The host: a player came in (or changed his name).
pub fn host_join(id: u8, name: &str) {
    with_host(|h| match h.board.rows.iter_mut().find(|r| r.id == id) {
        Some(r) => r.name = name.to_string(),
        None => {
            let team = auto_team(&h.board.rows);
            h.board.rows.push(ScoreRow { id, name: name.to_string(), team, ..Default::default() });
        }
    });
}

pub fn host_leave(id: u8) {
    with_host(|h| h.board.rows.retain(|r| r.id != id));
}

/// A player's team (host or joiner, from the board).
pub fn team_of(id: u8) -> Option<u8> {
    if !team_mode() {
        return None;
    }
    board()?.rows.iter().find(|r| r.id == id).map(|r| r.team)
}

/// Team Deathmatch: the team of the player with this name (name tags).
pub fn team_of_name(name: &str) -> Option<u8> {
    if !team_mode() {
        return None;
    }
    board()?.rows.iter().find(|r| r.name == name).map(|r| r.team)
}

/// Team Deathmatch: both players are on the same team.
pub fn allied(a: u8, b: u8) -> bool {
    team_mode() && team_of(a).is_some() && team_of(a) == team_of(b)
}

/// The host: `healer` used a medkit on another player.
pub fn host_heal(healer: u8) {
    with_host(|h| {
        if let Some(r) = h.board.rows.iter_mut().find(|r| r.id == healer) {
            r.heals += 1;
        }
    });
}

/// The host: `who` killed a pawn (co-op's scoreboard).
pub fn host_npc_kill(who: u8) {
    with_host(|h| {
        if let Some(r) = h.board.rows.iter_mut().find(|r| r.id == who) {
            r.npc_kills += 1;
        }
    });
}

/// What a kill meant, for netplay to pass on.
#[derive(Clone, Debug, Default)]
pub struct KillNews {
    /// The line for everyone's log.
    pub line: String,
    /// The killer scored with this weapon class (unlocks, Reward): (killer, weapon).
    pub credit: Option<(u8, String)>,
    /// A message for the killer alone ("You killed a teammate!").
    pub to_killer: Option<String>,
    /// The match just ended.
    pub ended: bool,
}

/// The weapon's name as the kill line says it: " with a pistol!" (CreateKillerProfile: WithString + itemArticle +
/// itemName; poison / burning have their own).
pub fn method(weapon: &str, kind: &str) -> String {
    match kind {
        "Poison" | "PoisonEffect" => return " with deadly poison!".into(),
        "Burned" | "Flamed" => return " with excessive burning!".into(),
        _ => {}
    }
    if weapon.is_empty() {
        return ".".into();
    }
    let d = class_defaults(weapon);
    let s = |n: &str| match unreal_pkg::props::find(&d, n) {
        Some(unreal_pkg::props::Value::Str(v)) => v.clone(),
        _ => String::new(),
    };
    let name = s("ItemName");
    if name.is_empty() {
        return ".".into();
    }
    let art = s("ItemArticle");
    if art.is_empty() || art == "-" {
        format!(" with {name}!")
    } else {
        format!(" with {art} {name}!")
    }
}

/// A class's defaults from a ClassDb of the game's System folder (opened once, for netplay's lines).
fn class_defaults(class: &str) -> Vec<unreal_pkg::props::Property> {
    static DB: Mutex<Option<unreal_pkg::classes::ClassDb>> = Mutex::new(None);
    let mut g = DB.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        let Some(root) = unreal_pkg::paths::root() else { return Vec::new() };
        *g = Some(unreal_pkg::classes::ClassDb::open(&root.join("System")));
    }
    g.as_mut().map(|db| db.defaults(class)).unwrap_or_default()
}

/// The host: player `victim` died, by `killer`'s hand (None: the world) with `weapon` (DeusExMPGame.Killed and
/// the victory check). `method`: method() for the line.
pub fn host_died(victim: u8, killer: Option<u8>, weapon: &str, method: &str) -> KillNews {
    let mut news = KillNews::default();
    let r = rules().unwrap_or_default();
    let mut g = HOST.lock().unwrap_or_else(|e| e.into_inner());
    let Some(h) = g.as_mut() else { return news };
    if h.board.ended.is_some() {
        return news;
    }
    h.dirty = true;
    let name = |h: &HostMatch, id: u8| h.board.rows.iter().find(|r| r.id == id).map_or_else(|| format!("Player {id}"), |r| r.name.clone());
    let vname = name(h, victim);
    if let Some(v) = h.board.rows.iter_mut().find(|x| x.id == victim) {
        v.deaths += 1;
        v.streak = 0;
        if killer.is_none_or(|k| k == victim) && v.score > 0 {
            v.score -= 1;
        }
    }
    if r.mode == Mode::Coop {
        // Co-op keeps deaths only (netplay says the line).
        return news;
    }
    match killer {
        Some(k) if k != victim => {
            let kname = name(h, k);
            news.line = format!("{kname} killed {vname}{method}");
            let mates = r.mode == Mode::Team && team_of_rows(&h.board.rows, k) == team_of_rows(&h.board.rows, victim);
            if mates {
                if let Some(kr) = h.board.rows.iter_mut().find(|x| x.id == k) {
                    kr.score = (kr.score - 1).max(0);
                }
                news.to_killer = Some("You killed a teammate!".into());
            } else {
                if let Some(kr) = h.board.rows.iter_mut().find(|x| x.id == k) {
                    kr.score += 1;
                    kr.streak += 1;
                }
                news.credit = Some((k, weapon.to_string()));
                if let Some(end) = check_victory(h, &r, Some((&kname, &vname, method))) {
                    h.board.ended = Some(end);
                    h.board.next_in = data().new_map_delay;
                    news.ended = true;
                }
            }
        }
        _ => {
            news.line = if method.len() > 1 { format!("{vname} killed themselves{method}") } else { format!("{vname} died.") };
        }
    }
    // Auto-balance: the dead player of a team two bigger than the other changes sides.
    if r.mode == Mode::Team && r.auto_balance {
        let u = h.board.rows.iter().filter(|x| x.team == TEAM_UNATCO).count() as i32;
        let n = h.board.rows.iter().filter(|x| x.team == TEAM_NSF).count() as i32;
        if let Some(v) = h.board.rows.iter_mut().find(|x| x.id == victim) {
            let (mine, other) = if v.team == TEAM_UNATCO { (u, n) } else { (n, u) };
            if mine - other >= 2 {
                v.team = 1 - v.team;
                news.line.push_str(&format!(" {} moves to {} to even the teams.", v.name, team_name(v.team)));
            }
        }
    }
    let ended = news.ended;
    drop(g);
    if ended {
        on_match_end();
    }
    news
}

fn team_of_rows(rows: &[ScoreRow], id: u8) -> Option<u8> {
    rows.iter().find(|r| r.id == id).map(|r| r.team)
}

/// GetWinningPlayer: most points, then fewest deaths, then longest streak; a full tie is no winner.
fn winner(rows: &[ScoreRow]) -> Option<&ScoreRow> {
    let key = |r: &ScoreRow| (r.score, -(r.deaths as i64), r.streak);
    let best = rows.iter().max_by_key(|r| key(r))?;
    (rows.iter().filter(|r| key(r) == key(best)).count() == 1).then_some(best)
}

fn team_scores(rows: &[ScoreRow]) -> [i32; 2] {
    let mut s = [0, 0];
    for r in rows {
        s[(r.team as usize).min(1)] += r.score;
    }
    s
}

/// CheckVictoryConditions. `last`: the kill that may have ended it (killer, victim, method).
fn check_victory(h: &mut HostMatch, r: &DmRules, last: Option<(&str, &str, &str)>) -> Option<Ending> {
    let timed_out = h.board.time_left.is_some_and(|t| t <= 0.0);
    let after = r.after;
    let next = match after {
        After::NextMap => next_map(&r.map),
        After::SameMap | After::Room => r.map.clone(),
    };
    let last_kill = last.map(|(k, v, m)| format!("The match ended with {k} taking out {v}{m}")).unwrap_or_default();
    if r.mode == Mode::Team {
        let s = team_scores(&h.board.rows);
        let won = |t: u8| Ending { headline: format!("{} has won the match!", team_name(t)), last_kill: last_kill.clone(), winner_team: Some(t), next_map: next.clone(), after, ..Default::default() };
        if r.frag_limit > 0 {
            for t in [TEAM_UNATCO, TEAM_NSF] {
                if s[t as usize] >= r.frag_limit as i32 {
                    return Some(won(t));
                }
            }
        }
        if timed_out {
            return Some(match s[0].cmp(&s[1]) {
                std::cmp::Ordering::Greater => won(TEAM_UNATCO),
                std::cmp::Ordering::Less => won(TEAM_NSF),
                std::cmp::Ordering::Equal => Ending { headline: "The match is a draw!".into(), next_map: next, after, ..Default::default() },
            });
        }
        return None;
    }
    let w = winner(&h.board.rows).cloned();
    let reached = r.frag_limit > 0 && w.as_ref().is_some_and(|w| w.score >= r.frag_limit as i32);
    if reached || timed_out {
        return Some(match w {
            Some(w) => Ending { headline: format!("{} has won the match!", w.name), last_kill: if reached { last_kill } else { String::new() }, winner_id: Some(w.id), next_map: next, after, ..Default::default() },
            None => Ending { headline: "The match is a draw!".into(), next_map: next, after, ..Default::default() },
        });
    }
    None
}

/// The host, every frame on a deathmatch map: the clock, the notices, the time limit, the next map. Returns lines
/// for everyone (NotifyGameStatus).
pub fn host_tick(dt: f32) -> Vec<String> {
    let mut out = Vec::new();
    let Some(r) = rules() else { return out };
    let mut g = HOST.lock().unwrap_or_else(|e| e.into_inner());
    let Some(h) = g.as_mut() else { return out };
    h.clock += dt;
    if h.board.ended.is_some() {
        let before = h.board.next_in;
        h.board.next_in -= dt;
        if before.ceil() != h.board.next_in.ceil() {
            h.dirty = true;
        }
        if h.board.next_in <= 0.0 && h.travel.is_none() {
            h.travel = h.board.ended.as_ref().map(|e| e.next_map.clone());
        }
        return out;
    }
    if let Some(t) = h.board.time_left.as_mut() {
        let before = *t;
        *t -= dt;
        if before.ceil() != t.ceil() {
            h.dirty = true;
        }
        // One minute left (NotifyMinutes), when the limit is over two minutes.
        if r.classic() && *t <= 60.0 && !h.notified_minute && r.time_limit >= 2 {
            // DeusExMPGame.NotifyGameStatus: "Less than a minute to go, and X leads the match!" / tied.
            h.notified_minute = true;
            let lead = if r.mode == Mode::Team {
                let s = team_scores(&h.board.rows);
                match s[0].cmp(&s[1]) {
                    std::cmp::Ordering::Greater => team_name(TEAM_UNATCO).to_string(),
                    std::cmp::Ordering::Less => team_name(TEAM_NSF).to_string(),
                    _ => "Tied".into(),
                }
            } else {
                winner(&h.board.rows).map_or_else(|| "Tied".to_string(), |w| w.name.clone())
            };
            out.push(crate::mphud::note_code("time", 1, &lead));
        } else if *t <= 60.0 && !h.notified_minute && r.time_limit >= 2 {
            h.notified_minute = true;
            let lead = if r.mode == Mode::Team {
                let s = team_scores(&h.board.rows);
                match s[0].cmp(&s[1]) {
                    std::cmp::Ordering::Greater => team_name(TEAM_UNATCO).to_string(),
                    std::cmp::Ordering::Less => team_name(TEAM_NSF).to_string(),
                    _ => "Nobody".into(),
                }
            } else {
                winner(&h.board.rows).map_or_else(|| "Nobody".to_string(), |w| w.name.clone())
            };
            out.push(format!("One minute left! {lead} is in the lead."));
        }
    }
    // Frag notices: a fifth of the limit to go, and one to go (NotifyGameStatus).
    if r.frag_limit >= 2 {
        let best = if r.mode == Mode::Team {
            team_scores(&h.board.rows).into_iter().max().unwrap_or(0)
        } else {
            h.board.rows.iter().map(|x| x.score).max().unwrap_or(0)
        };
        let left = r.frag_limit as i32 - best;
        let fifth = if crate::matchmode::current().fifth_notice(r.frag_limit) { (r.frag_limit / 5) as i32 } else { 0 };
        for mark in [fifth, 1] {
            if mark > 0 && left == mark && h.notified_frags != mark as u32 {
                h.notified_frags = mark as u32;
                if r.classic() {
                    // MPMSG_CloseKills: "Only N more kills, and X will win the match!".
                    let lead = if r.mode == Mode::Team {
                        let s = team_scores(&h.board.rows);
                        match s[0].cmp(&s[1]) {
                            std::cmp::Ordering::Greater => team_name(TEAM_UNATCO).to_string(),
                            std::cmp::Ordering::Less => team_name(TEAM_NSF).to_string(),
                            _ => "Tied".into(),
                        }
                    } else {
                        winner(&h.board.rows).map_or_else(|| "Tied".to_string(), |w| w.name.clone())
                    };
                    out.push(crate::mphud::note_code("close", mark as u32, &lead));
                } else {
                    out.push(format!("{mark} kill{} to win!", if mark == 1 { "" } else { "s" }));
                }
            }
        }
    }
    let ended = check_victory(h, &r, None);
    if let Some(e) = ended {
        h.board.ended = Some(e);
        h.board.next_in = data().new_map_delay;
        h.dirty = true;
        drop(g);
        on_match_end();
    }
    out
}

/// The host: the match is over and its results have shown: the map to load now (once).
pub fn host_take_travel() -> Option<String> {
    let mut g = HOST.lock().unwrap_or_else(|e| e.into_inner());
    g.as_mut()?.travel.take()
}

/// The match ended here (host or joiner): play stops, the results show.
fn on_match_end() {
    crate::sound::play_player("DeusExSounds.UserInterface.Menu_OK", 0.6, 0.0);
    // The character's record: a match, and a win when it was ours (or our team's).
    if let Some(b) = board() {
        if let Some(e) = &b.ended {
            let me = my_id();
            let mine = b.rows.iter().find(|r| r.id == me).map(|r| r.team);
            let won = e.winner_id == Some(me) || (e.winner_team.is_some() && e.winner_team == mine);
            crate::dmchar::note_match(won);
        }
    }
}

/// The match is over (the results screen is up, nobody fights).
pub fn match_over() -> bool {
    playing() && board().is_some_and(|b| b.ended.is_some())
}

// ---------------------------------------------------------------------------------------------------------------
// Pickups: the map's list, taken / respawning (the host decides; joiners mirror him).

/// A placed pickup this map keeps.
struct Spot {
    actor: usize,
    /// A weapon that stays where it lies (Classic: bWeaponStay); everyone takes a copy.
    stays: bool,
    respawn: f32,
    /// Seconds until it's back (0 = there).
    left: f32,
}

#[derive(Default)]
struct Pickups {
    map_gen: u64,
    spots: Vec<Spot>,
    /// Changed since netplay last took the list.
    dirty: bool,
    /// Joiner: the host's list of placed pickups that are there.
    shown: Option<Vec<u32>>,
    /// Joiner: pickups he asked the host for (to send), and asked but not answered yet (kept hidden).
    asked: Vec<u32>,
    pending: Vec<u32>,
}

static PICKUPS: Mutex<Option<Pickups>> = Mutex::new(None);

fn with_pickups<R>(f: impl FnOnce(&mut Pickups) -> R) -> R {
    let mut g = PICKUPS.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Pickups::default))
}

/// A pickup taken out of the map (it waits here to come back).
#[derive(Component)]
pub struct DmTaken {
    pub item: crate::pickups::PickupItem,
}

/// A placed pickup the deathmatch rules take out of the map for good.
#[derive(Component)]
pub struct DmRemoved;

/// What happens when this player frobs a placed pickup in a deathmatch.
pub enum Frob {
    /// Not a deathmatch pickup: the usual.
    Normal,
    /// Take it (the host, or a pickup the host has granted).
    Take,
    /// A joiner: asked the host; it's hidden until he says.
    Asked,
    /// A weapon that stays (Classic): take a copy, leave it there.
    Stay,
}

/// The pickup's properties as this mode hands it out (Classic: mpPickupAmmoCount x 3 rounds with a weapon).
pub fn pickup_props(class: &str, props: &[unreal_pkg::props::Property]) -> Vec<unreal_pkg::props::Property> {
    use unreal_pkg::props::{Property, Value};
    let mut v = props.to_vec();
    if playing() {
        if let Some(n) = crate::matchmode::current().pickup_ammo(class) {
            v.retain(|p| !(p.name.eq_ignore_ascii_case("PickupAmmoCount") && p.index == 0));
            v.push(Property { name: "PickupAmmoCount".into(), index: 0, value: Value::Int(n as i32) });
        }
    }
    v
}

/// pickups::frob_item: a placed pickup is about to be taken.
pub fn frob_placed(actor: usize) -> Frob {
    if !playing() {
        return Frob::Normal;
    }
    let spot = with_pickups(|p| p.spots.iter().find(|s| s.actor == actor).map(|s| s.stays));
    let Some(stays) = spot else {
        return Frob::Normal;
    };
    if stays {
        return Frob::Stay;
    }
    if crate::netplay::is_client() {
        with_pickups(|p| {
            if !p.asked.contains(&(actor as u32)) {
                p.asked.push(actor as u32);
            }
            if !p.pending.contains(&(actor as u32)) {
                p.pending.push(actor as u32);
            }
        });
        return Frob::Asked;
    }
    Frob::Take
}

/// The pickup was taken here (the host's own, or granted): its respawn clock starts.
pub fn taken(actor: usize) {
    with_pickups(|p| {
        if let Some(s) = p.spots.iter_mut().find(|s| s.actor == actor) {
            s.left = s.respawn.max(1.0);
            p.dirty = true;
        }
    });
}

/// Joiner: pickups asked for since last asked (netplay sends them).
pub fn take_asked() -> Vec<u32> {
    with_pickups(|p| std::mem::take(&mut p.asked))
}

/// Host: a joiner asks for a placed pickup; true when it's his (it was there: it goes now).
pub fn host_request(actor: u32) -> bool {
    with_pickups(|p| match p.spots.iter_mut().find(|s| s.actor == actor as usize) {
        Some(s) if s.left <= 0.0 => {
            s.left = s.respawn.max(1.0);
            p.dirty = true;
            true
        }
        _ => false,
    })
}

/// Host: the placed pickups that are there now, when that changed (netplay sends it to everyone).
pub fn take_pickup_state() -> Option<Vec<u32>> {
    with_pickups(|p| {
        if !p.dirty {
            return None;
        }
        p.dirty = false;
        Some(p.spots.iter().filter(|s| s.left <= 0.0).map(|s| s.actor as u32).collect())
    })
}

/// Host: the full list again (a player came in).
pub fn pickup_state() -> Vec<u32> {
    with_pickups(|p| p.spots.iter().filter(|s| s.left <= 0.0).map(|s| s.actor as u32).collect())
}

/// Joiner: the host's list came.
pub fn set_pickup_state(there: Vec<u32>) {
    with_pickups(|p| {
        p.pending.clear();
        p.shown = Some(there);
    });
}

static GRANTED: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// Joiner: the host gave us a placed pickup we asked for.
pub fn grant_placed(actor: u32) {
    GRANTED.lock().unwrap_or_else(|e| e.into_inner()).push(actor);
}

pub fn take_granted_placed() -> Vec<u32> {
    std::mem::take(&mut *GRANTED.lock().unwrap_or_else(|e| e.into_inner()))
}

/// A granted pickup's class (the host said it's ours): (class, props) from the map.
pub fn placed_item(lvl: &dx_level::Level, actor: usize) -> Option<(String, Vec<unreal_pkg::props::Property>)> {
    let a = lvl.actors.get(actor)?;
    Some((a.class.clone(), a.props.clone()))
}

/// Builds the map's list on arrival, takes out what the rules leave out, runs the respawn clocks (host) or
/// mirrors the host's list (joiner).
#[allow(clippy::type_complexity)]
pub fn pickups_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<crate::LevelRes>,
    items: Query<(Entity, &crate::pickups::PickupItem, &mut Visibility), (Without<DmTaken>, Without<DmRemoved>)>,
    mut taken_q: Query<(Entity, &DmTaken, &mut Visibility), Without<crate::pickups::PickupItem>>,
) {
    let lvl = &level.0;
    let gen = crate::loader::generation();
    let new_map = with_pickups(|p| p.map_gen != gen);
    if new_map {
        if crate::loader::busy() {
            return;
        }
        let on = versus() && is_dm_map(&lvl.name);
        if on && items.is_empty() {
            // (The map's pickups aren't spawned yet.)
            return;
        }
        ON_MAP.store(on, std::sync::atomic::Ordering::Relaxed);
        let mut spots = Vec::new();
        if on {
            let stem = lvl.name.trim_end_matches(".dx").to_string();
            let entry = data().maps.iter().find(|m| m.file.eq_ignore_ascii_case(&stem));
            let mode = crate::matchmode::current();
            for (e, it, _) in &items {
                let Some(a) = it.actor else { continue };
                let Some(info) = lvl.actors.get(a) else { continue };
                let Some(secs) = mode.respawn_secs(&info.class) else { continue };
                // Classic: every pickup the map places (Inventory.RespawnTime); Modern: the map list's.
                let keep = mode.keeps_all_pickups() || entry.is_some_and(|m| m.keep.iter().any(|k| k.eq_ignore_ascii_case(&info.name)));
                if keep {
                    spots.push(Spot { actor: a, stays: mode.weapon_stays(&info.class), respawn: secs, left: 0.0 });
                } else {
                    commands.entity(e).remove::<crate::pickups::PickupItem>().insert((DmRemoved, Visibility::Hidden));
                }
            }
            println!("DM map {} : {} pickups kept", lvl.name, spots.len());
        }
        with_pickups(|p| {
            *p = Pickups { map_gen: gen, spots, dirty: true, ..Default::default() };
        });
        return;
    }
    if !playing() {
        return;
    }
    let dt = time.delta_secs();
    let client = crate::netplay::is_client();
    // Which actors should be there now.
    let there: Vec<usize> = with_pickups(|p| {
        if client {
            let mut v: Vec<usize> = p.shown.as_ref().map(|v| v.iter().map(|&a| a as usize).collect()).unwrap_or_else(|| p.spots.iter().map(|s| s.actor).collect());
            v.retain(|a| !p.pending.contains(&(*a as u32)));
            return v;
        }
        let mut back = false;
        for s in p.spots.iter_mut().filter(|s| s.left > 0.0) {
            s.left -= dt;
            if s.left <= 0.0 {
                s.left = 0.0;
                back = true;
            }
        }
        if back {
            p.dirty = true;
        }
        p.spots.iter().filter(|s| s.left <= 0.0).map(|s| s.actor).collect()
    });
    // Respawned ones come back.
    for (e, t, mut vis) in &mut taken_q {
        let Some(a) = t.item.actor else { continue };
        if there.contains(&a) {
            commands.entity(e).remove::<DmTaken>().insert(crate::pickups::PickupItem { actor: t.item.actor, class: t.item.class.clone(), props: t.item.props.clone() });
            vis.set_if_neq(Visibility::Inherited);
            if let Some(info) = lvl.actors.get(a) {
                let l = info.location();
                crate::sound::play("DeusExSounds.Generic.WeaponPickup", DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64), 0.6, 512.0, 1.0);
            }
        }
    }
    // Ones somebody else took go (the host: a joiner took it; a joiner: anyone did).
    {
        for (e, it, _) in &items {
            let Some(a) = it.actor else { continue };
            if with_pickups(|p| p.spots.iter().any(|s| s.actor == a)) && !there.contains(&a) {
                hide(&mut commands, e, it);
            }
        }
    }
}

/// Takes a pickup out of the world until it respawns.
pub fn hide(commands: &mut Commands, e: Entity, it: &crate::pickups::PickupItem) {
    commands
        .entity(e)
        .remove::<crate::pickups::PickupItem>()
        .insert((DmTaken { item: crate::pickups::PickupItem { actor: it.actor, class: it.class.clone(), props: it.props.clone() } }, Visibility::Hidden));
}

// ---------------------------------------------------------------------------------------------------------------
// Arriving on a deathmatch map, the host's referee clock.

static SPAWNED_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Where the other players are (and their teams), for a spawn point.
fn others() -> Vec<(DVec3, Option<u8>)> {
    crate::netplay::remote_bodies().into_iter().map(|(id, p)| (p, team_of(id))).collect()
}

/// Put this player at a fresh start with the start kit (arrival, and every respawn).
pub fn respawn_here(
    lvl: &dx_level::Level,
    player: &mut crate::Player,
    ph: &mut crate::ai::PlayerHealth,
    inv: &mut imsim_core::inventory::Inventory,
    classes: &mut unreal_pkg::classes::ClassDb,
) -> Option<f32> {
    let (at, yaw) = spawn_point(lvl, team_of(my_id()), &others())?;
    *ph = crate::ai::PlayerHealth::default();
    player.body = imsim_core::movement::Character::new(at);
    player.body.physics = imsim_core::movement::Physics::Falling;
    player.noclip = false;
    let mode = crate::matchmode::current();
    mode.restart_player(inv, classes);
    crate::mphud::start_shield(mode.spawn_shield());
    Some(cam_yaw(yaw))
}

/// On a deathmatch map's first frames: everyone starts at a start of his own with the start kit; the host starts
/// the match. Every frame after: the host's clock, notices and the next map.
#[allow(clippy::too_many_arguments)]
pub fn match_system(
    time: Res<Time>,
    level: Res<crate::LevelRes>,
    mut player: ResMut<crate::Player>,
    mut ph: ResMut<crate::ai::PlayerHealth>,
    mut inv: ResMut<crate::pickups::PlayerInventory>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut cams: Query<&mut crate::FlyCam>,
) {
    if !playing() || crate::loader::busy() || crate::loader::settling() {
        return;
    }
    let gen = crate::loader::generation();
    if SPAWNED_GEN.swap(gen, std::sync::atomic::Ordering::Relaxed) != gen {
        if crate::netplay::is_host() || !crate::netplay::active() {
            let mut players = vec![(0u8, crate::travel::player_name())];
            players.extend(crate::netplay::players().into_iter().map(|p| (p.0, p.1)));
            host_begin(&players);
        }
        if let Some(yaw) = respawn_here(&level.0, &mut player, &mut ph, &mut inv.0, &mut assets.classes) {
            for mut c in &mut cams {
                c.yaw = yaw;
                c.pitch = 0.0;
            }
        }
        let r = rules().unwrap_or_default();
        let lim = match (r.frag_limit, r.time_limit) {
            (0, 0) => String::new(),
            (f, 0) => format!(" First to {f} kills wins."),
            (0, t) => format!(" {t} minutes."),
            (f, t) => format!(" First to {f} kills, {t} minutes."),
        };
        crate::classic_hud::client_message(&format!("{} on {}.{lim}", r.mode.name(), map_title(&level.0.name)));
        return;
    }
    if RESPAWN.swap(false, std::sync::atomic::Ordering::Relaxed) && !match_over() {
        if let Some(yaw) = respawn_here(&level.0, &mut player, &mut ph, &mut inv.0, &mut assets.classes) {
            for mut c in &mut cams {
                c.yaw = yaw;
                c.pitch = 0.0;
            }
            if !classic() {
                crate::classic_hud::client_message("You're back in the fight.");
            }
        }
    }
    if crate::netplay::is_host() || !crate::netplay::active() {
        for line in host_tick(time.delta_secs()) {
            if !crate::mphud::from_note(&line) {
                crate::classic_hud::client_message(&line);
            }
            NEWS.lock().unwrap_or_else(|e| e.into_inner()).push(line);
        }
        if let Some(map) = host_take_travel() {
            let after = rules().map(|r| r.after).unwrap_or_default();
            TRAVEL.lock().unwrap_or_else(|e| e.into_inner()).replace((map, after));
        }
    }
}

static NEWS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static RESPAWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The original multiplayer player classes (menuchoice_class), in Player Setup order.
pub const CLASSIC_CLASSES: [(&str, &str); 4] = [("JCDentonMale", "JC Denton"), ("MPNSF", "NSF Terrorist"), ("MPUnatco", "UNATCO Trooper"), ("MPMJ12", "Majestic-12 Agent")];

/// Classic: the class this player plays (team games: his team's - TeamDMGame's DefaultPlayerClass per team; else
/// his Player Setup pick, setting MpClass). None: JC Denton.
pub fn classic_class() -> Option<String> {
    let r = rules().filter(|r| r.classic())?;
    if r.mode == Mode::Team {
        let t = team_of(my_id())?;
        return Some(if t == TEAM_NSF { "MPNSF" } else { "MPUnatco" }.to_string());
    }
    let k = (setting!("MpClass") as usize).min(CLASSIC_CLASSES.len() - 1);
    (k > 0).then(|| CLASSIC_CLASSES[k].0.to_string())
}

/// A class's look (Mesh, Texture, Skin, MultiSkins) from the player's own DeusEx.u; empty when its mesh package
/// isn't there.
pub fn class_dress(class: &str) -> Vec<unreal_pkg::props::Property> {
    use unreal_pkg::props::{Property, Value};
    let Some(Value::Path(mesh)) = crate::classicmp::class_value(class, "Mesh") else { return Vec::new() };
    let pkg = mesh.split('.').next().unwrap_or_default();
    let found = unreal_pkg::paths::root().is_some_and(|r| {
        let dir = r.join("System");
        std::fs::read_dir(&dir).map(|d| d.flatten().any(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(&format!("{pkg}.u")))).unwrap_or(false)
    });
    if !found {
        return Vec::new();
    }
    let mut v = vec![Property { name: "Mesh".into(), index: 0, value: Value::Path(mesh) }];
    for name in ["Texture", "Skin"] {
        if let Some(x) = crate::classicmp::class_value(class, name) {
            v.push(Property { name: name.into(), index: 0, value: x });
        }
    }
    let d = crate::classicmp::class_props(class);
    for p in d.into_iter().filter(|p| p.name.eq_ignore_ascii_case("MultiSkins")) {
        v.push(p);
    }
    v
}

/// netplay: this dead player gets up again now (match_system puts him at a start with the kit).
pub fn request_respawn() {
    RESPAWN.store(true, std::sync::atomic::Ordering::Relaxed);
}
static TRAVEL: Mutex<Option<(String, After)>> = Mutex::new(None);

static AWAIT_ROOM: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static ROOM_OPEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Host, After the Match = Back to Room: Match Settings opens for the next match (menus.rs).
pub fn await_room() {
    AWAIT_ROOM.store(true, std::sync::atomic::Ordering::Relaxed);
    ROOM_OPEN.store(true, std::sync::atomic::Ordering::Relaxed);
    crate::classic_hud::client_message("The match is over: choose the next one in Match Settings.");
}

/// menus.rs: open Match Settings now (once).
pub fn take_room_open() -> bool {
    ROOM_OPEN.swap(false, std::sync::atomic::Ordering::Relaxed)
}

/// menus.rs: Match Settings was saved: when the room waits for it, the next match starts on its map.
/// Match Settings were saved: the room's rules again from them (same family, game and mode).
pub fn reapply_settings() {
    let Some(r) = rules() else { return };
    let nr = if r.classic() { classic_rules_from_settings(r.classic.game) } else { rules_from_settings(r.mode, r.friendly_fire) };
    set_rules(Some(nr));
}

pub fn room_go() {
    if !AWAIT_ROOM.swap(false, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let Some(old) = rules() else { return };
    let r = if old.classic() { classic_rules_from_settings(old.classic.game) } else { rules_from_settings(old.mode, old.friendly_fire) };
    let map = r.map.clone();
    set_rules(Some(r));
    TRAVEL.lock().unwrap_or_else(|e| e.into_inner()).replace((map, After::NextMap));
}

/// Host: the referee's lines for everyone's log (netplay sends them).
pub fn take_news() -> Vec<String> {
    std::mem::take(&mut *NEWS.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Host: the match's results have shown: (map, what next). netplay takes everyone there.
pub fn take_travel() -> Option<(String, After)> {
    TRAVEL.lock().unwrap_or_else(|e| e.into_inner()).take()
}

static REMOTE_SKINS: Mutex<Vec<(u8, Vec<(String, String)>)>> = Mutex::new(Vec::new());

/// Another player's weapon skins (from his look): his body's weapon wears them.
pub fn set_remote_skins(id: u8, skins: Vec<(String, String)>) {
    let mut g = REMOTE_SKINS.lock().unwrap_or_else(|e| e.into_inner());
    g.retain(|r| r.0 != id);
    g.push((id, skins));
}

pub fn remote_skin(id: u8, class: &str) -> Option<String> {
    let g = REMOTE_SKINS.lock().unwrap_or_else(|e| e.into_inner());
    g.iter().find(|r| r.0 == id)?.1.iter().find(|s| s.0 == class).map(|s| s.1.clone())
}

pub fn reset_level() {
    ON_MAP.store(false, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The map data's per-mode parts: names unique, modes known, codes and loot sensible; with CON_GAME the
    /// actors they name are in the map files.
    #[test]
    fn map_data_is_consistent() {
        let d = data();
        let game = std::env::var("CON_GAME").ok();
        for m in &d.maps {
            let mut names = std::collections::HashSet::new();
            let modes = m.spawns.iter().map(|x| (&x.name, &x.modes))
                .chain(m.blockers.iter().map(|x| (&x.name, &x.modes)))
                .chain(m.objectives.iter().map(|x| (&x.name, &x.modes)))
                .chain(m.loot.iter().map(|x| (&x.name, &x.modes)))
                .chain(m.codes.iter().map(|x| (&x.name, &x.modes)));
            for (name, list) in modes {
                assert!(names.insert(name.clone()), "{}: {name} twice", m.file);
                for k in list {
                    assert!(MODE_KEYS.contains(&k.to_ascii_lowercase().as_str()), "{}: {name}: unknown mode {k}", m.file);
                }
            }
            for c in &m.codes {
                assert!((1..=8).contains(&c.digits), "{}: {} digits", m.file, c.name);
                assert!(!c.devices.is_empty(), "{}: {} has no devices", m.file, c.name);
            }
            for t in &m.loot {
                assert!(t.items.iter().all(|i| i.1 > 0), "{}: {} zero weight", m.file, t.name);
                assert!(t.picks as usize <= t.actors.len() + t.points.len(), "{}: {} picks more than its spots", m.file, t.name);
            }
            for s in &m.spawns {
                assert!(!s.points.is_empty(), "{}: {} has no points", m.file, s.name);
            }
            let Some(root) = game.as_deref() else { continue };
            let Ok(pkg) = unreal_pkg::Package::open(std::path::Path::new(root).join("Maps").join(format!("{}.dx", m.file))) else { continue };
            let have: std::collections::HashSet<String> = pkg.exports.iter().map(|e| e.name.to_ascii_lowercase()).collect();
            let named = m.keep.iter().chain(m.loot.iter().flat_map(|t| t.actors.iter())).chain(m.codes.iter().flat_map(|c| c.devices.iter()));
            for a in named {
                assert!(have.contains(&a.to_ascii_lowercase()), "{}: no actor {a}", m.file);
            }
        }
    }

    #[test]
    fn codes_and_loot_are_seeded() {
        let t = LootTable { name: "t".into(), modes: Vec::new(), actors: Vec::new(), points: vec![(0.0, 0.0, 0.0); 6], items: vec![("MedKit".into(), 1), ("Lockpick".into(), 3)], picks: 4 };
        let a = roll_loot(&t, 42);
        assert_eq!(a, roll_loot(&t, 42));
        assert_eq!(a.len(), 4);
        let spots: std::collections::HashSet<usize> = a.iter().map(|x| x.0).collect();
        assert_eq!(spots.len(), 4);
        assert!(a.iter().all(|x| x.1 == "MedKit" || x.1 == "Lockpick"));
    }

    #[test]
    fn data_parses() {
        let d = data();
        assert_eq!(d.maps.len(), 5);
        for m in &d.maps {
            let geps = m.keep.iter().filter(|k| k.starts_with("WeaponGEPGun")).count();
            assert_eq!(geps, 1, "{}", m.file);
            for heavy in ["WeaponLAW", "WeaponPlasmaRifle", "WeaponFlamethrower", "WeaponRifle", "WeaponNanoSword"] {
                // (WeaponRifle also prefixes nothing else.)
                assert!(m.keep.iter().filter(|k| k.strip_prefix(heavy).is_some_and(|n| n.chars().all(|c| c.is_ascii_digit()))).count() <= 1, "{} {heavy}", m.file);
            }
        }
        assert_eq!(respawn_secs("WeaponGEPGun"), Some(120.0));
        assert_eq!(respawn_secs("WeaponPistol"), Some(30.0));
        assert_eq!(next_map("DXMP_Smuggler"), d.maps[0].file);
    }

    #[test]
    fn lethality() {
        let (h, r) = (47.5, 20.0);
        let head = DVec3::new(0.0, 0.0, 45.0);
        let body = DVec3::new(0.0, 5.0, 20.0);
        // A pistol head shot kills (100 of 100, fed as 50 that the head doubles).
        let (d, at) = player_damage("WeaponPistol", "Shot", 14.0, Some(head), h, r);
        assert_eq!(region(at, h, r), Region::Head);
        assert!((d * 2.0 * crate::travel::combat_difficulty() - 100.0).abs() < 0.5);
        // Three pistol body shots kill: 34 each.
        let (d, _) = player_damage("WeaponPistol", "Shot", 14.0, Some(body), h, r);
        assert!((d * 2.0 * crate::travel::combat_difficulty() - 34.0).abs() < 0.5);
        // A blast: the game's damage x0.6, on the body.
        let (d, at) = player_damage("WeaponGEPGun", "Exploded", 100.0, None, h, r);
        assert_eq!(region(at, h, r), Region::Body);
        assert!((d - 60.0).abs() < 0.01);
    }

    #[test]
    fn scoring_and_victory() {
        set_rules(Some(DmRules { mode: Mode::Deathmatch, frag_limit: 2, time_limit: 0, ..DmRules::default() }));
        host_begin(&[(0, "A".into()), (1, "B".into())]);
        let n = host_died(1, Some(0), "WeaponPistol", " with a pistol!");
        assert_eq!(n.line, "A killed B with a pistol!");
        assert!(!n.ended);
        // A suicide costs a point, not below zero.
        host_died(1, None, "", ".");
        let n = host_died(1, Some(0), "WeaponPistol", "!");
        assert!(n.ended);
        let b = board().unwrap();
        assert_eq!(b.ended.as_ref().unwrap().winner_id, Some(0));
        assert_eq!(b.rows.iter().find(|r| r.id == 1).unwrap().deaths, 3);
        host_end();
        set_rules(None);
    }
}
