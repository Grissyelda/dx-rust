//! Achievements: the fifty of Deus Ex Remastered (names and goals as that release lists them), our own icons, a
//! persona screen (from the Logs screen's Achievements button) and an unlock notice in the HUD's manner.
//!
//! Unlocks are kept per character, not per save (achievements.txt in the character's save folder, save::active_dir:
//! `id=unix time` lines and `progress.id=n` for the counted ones; a new character starts with none). Each one is detected from the same things the game itself tracks:
//! mission arrivals, conversation flags and choices, events (MadeBasket, SecretDoor, alexcloset), takedowns,
//! augmentations, skills, weapon mods, and a few watched situations (sneaking past guards, a cloaked walk through
//! someone's view, a body near a Karkian, the Underworld's pool table).

use crate::ai::{Npc, NpcState, Target};
use crate::uicanvas::{Canvas, HAlign, Style, UiTex, VAlign};
use crate::LevelRes;
use bevy::prelude::*;
use glam::DVec3;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

/// (id, name, goal).
pub const LIST: [(&str, &str, &str); 50] = [
    ("majestic", "Majestic Revelations", "Unlock every other achievement."),
    ("lady", "Rescue the Lady", "Stop the NSF terrorists on Liberty Island."),
    ("town", "On the Town", "Free the hostages, discover the NSF generator, and take down the NSF's EMP field."),
    ("lebedev", "The Lebedev Sanction", "Locate the airfield where the NSF moved the Ambrosia, and hear some shocking details from Paul."),
    ("choices", "Inevitable Choices", "Question your alliances."),
    ("breakout", "Breakout", "Escape the Majestic 12 facility."),
    ("hongkong", "Hong Kong", "Find Tracer Tong, obtain the ROM encoding for the Dragon's Tooth, and ally the Triads."),
    ("tanker1", "The Supertanker Part 1", "Meet Stanton Dowd and discuss the supertanker."),
    ("tanker2", "The Supertanker Part 2", "Scuttle the supertanker and disable the EM disruption field before escaping the shipyard."),
    ("paree", "Grey Paree", "Meet with Nicolette DuClare and make contact with the Illuminati."),
    ("templars", "Treasures of the Templars", "Access the computer so that Morgan Everett can complete his work on the Gray Death cure."),
    ("vandenberg", "Vandenberg AFB", "Uplink Daedalus and gain Morgan Everett access to the Universal Constructor."),
    ("sea", "By the Sea", "Steal a submarine and retrieve the schematic for the Universal Constructor."),
    ("area51", "Area 51", "Decide the fate of mankind."),
    ("helios", "Second Coming", "Complete the game by merging with Helios."),
    ("darkage", "Cut the Cord", "Complete the game by enacting a new Dark Age."),
    ("illuminati", "Shaking the Invisible Hand", "Complete the game by joining the Illuminati."),
    ("biomachina", "Biomachina", "Install at least one augmentation into each body part."),
    ("slugger", "Slugger", "Take down 3 enemies with a melee attack while the Combat Strength augmentation is active."),
    ("sneak", "Solid Sneak", "Sneak past 3 enemies with the Run Silent augmentation active."),
    ("section9", "Section 9 Methods", "Travel through a humanoid enemy's line of sight while the Cloak augmentation is active."),
    ("automoto", "Automotonophobia", "Travel through the line of sight of a machine with the Radar Transparency augmentation active."),
    ("wardenclyffe", "Wardenclyffe Man", "Activate 3 other augmentations while the Power Recirculator augmentation is active."),
    ("atomheart", "Atom Heart Son", "Increase the level of 3 other augmentations by using the Synthetic Heart augmentation."),
    ("bestoffense", "The Best Offense Is...", "Kill an enemy with their own projectile via the Aggressive Defense System."),
    ("specialeyes", "Special Eyes", "Alert an enemy seen through a surface with the Vision Enhancement augmentation active."),
    ("servant", "Public Servant", "Grant the sick bum's request in Battery Park."),
    ("dunkin", "JC Dunkin'", "Score a basket in Hell's Kitchen."),
    ("escapa", "El Escapa", "Successfully break Miguel out of the MJ12 detention facility."),
    ("sawnothing", "You Saw Nothing", "Listen to Walton Simons' interrogation of the NSF soldiers."),
    ("imin", "I'm In", "Enter the secret compartment in Alex Jacobson's office."),
    ("pinball", "Pinball Wizard", "Reveal the secret room in the NSF Helibase."),
    ("wildlife", "Don't Feed the Wildlife", "Feed a body to a Karkian or Greasel."),
    ("dibbuk", "Dibbuk Defusal", "Something's odd about that odd mechanic. Maybe Everett would know about him."),
    ("sure", "Sure", "Pay Mercedes and Tessa's entry fee."),
    ("renton", "Renton, Far From Home", "Save Sandra from JoJo, mourn Mr. Renton's untimely death, and meet Sandra in California."),
    ("hero", "American Hero", "Despite everything, keep your allies and friends alive."),
    ("tail", "Lose the Tail", "If you mention Stanton Dowd to Joe Greene... watch your back."),
    ("meanjoe", "No Thanks, Mean Joe", "Ah, yes. A journalist. You can trust him..."),
    ("based", "Based", "Talk with Leo Gold further after detaining him."),
    ("nicegun", "Nice Gun, Mind If I Have It?", "Obtain an assault rifle on your first mission on Liberty Island."),
    ("nerves", "Nerves of Steel", "Stop Anna Navarre from killing Lebedev."),
    ("tonjc", "Thanks a 'Ton, JC", "Don't be afraid, I'm with you, Paul."),
    ("howdid", "How Did You...", "Two passwords for your life."),
    ("notmachine", "I Am Not a Machi...", "Having a man on the inside at UNATCO pays off."),
    ("shame", "What a Shame", "Comment on Mr. Renton's untimely passing."),
    ("pool", "Pool Shark", "Use the cue ball to sink all other balls on the Underworld's pool table."),
    ("veteran", "Veteran Agent", "Have two Master level skills at the same time."),
    ("lockload", "Lock and Load", "Install one of each compatible upgrade type onto a single weapon."),
    ("upgrades", "Upgrades, people. Upgrades", "Fully upgrade an augmentation."),
];

/// Counted achievements: (id, target).
const COUNTED: [(&str, u32); 4] = [("slugger", 3), ("sneak", 3), ("wardenclyffe", 3), ("atomheart", 3)];

/// Ours: the endings (and reaching them) count for every character once any character has done it, so all three
/// can be had over several playthroughs (Save/achievements_global.txt keeps who did it first).
pub const GLOBAL: [&str; 4] = ["area51", "helios", "darkage", "illuminati"];

/// Ours: achievements whose name or goal gives the story away: blurred with a lock over them until unlocked.
pub const SPOILER: [&str; 21] = [
    "lebedev", "choices", "breakout", "escapa", "sawnothing", "dibbuk", "renton", "hero", "tail", "meanjoe", "nerves",
    "tonjc", "notmachine", "shame", "templars", "vandenberg", "sea", "area51", "helios", "darkage", "illuminati",
];

pub fn is_global(id: &str) -> bool {
    GLOBAL.contains(&id)
}

pub fn is_spoiler(id: &str) -> bool {
    SPOILER.contains(&id)
}

#[derive(Default)]
struct Store {
    loaded: bool,
    /// The character whose file is loaded.
    char: String,
    /// This character's own unlocks.
    unlocked: HashMap<String, u64>,
    progress: HashMap<String, u32>,
    /// The GLOBAL ones any character has: (time, that character's name).
    global: HashMap<String, (u64, String)>,
    /// Unlocked this session, waiting to be shown.
    popups: Vec<String>,
}

impl Store {
    fn has(&self, id: &str) -> bool {
        self.unlocked.contains_key(id) || self.global.contains_key(id)
    }

    fn when(&self, id: &str) -> Option<u64> {
        self.unlocked.get(id).copied().or_else(|| self.global.get(id).map(|g| g.0))
    }

    /// Majestic Revelations: everything else (own or global).
    fn check_majestic(&mut self) {
        if !self.unlocked.contains_key("majestic") && LIST.iter().filter(|a| a.0 != "majestic").all(|a| self.has(a.0)) {
            self.unlocked.insert("majestic".into(), now_secs());
            self.popups.push("majestic".into());
        }
    }
}

static STORE: std::sync::LazyLock<Mutex<Store>> = std::sync::LazyLock::new(|| Mutex::new(Store::default()));

fn path() -> std::path::PathBuf {
    crate::save::active_dir().join("achievements.txt")
}

fn global_path() -> std::path::PathBuf {
    crate::save::dir().join("achievements_global.txt")
}

/// Settings > Achievements > Reset All: every character's unlocks and progress and the shared ones go (the files are
/// removed; the next unlock starts them again).
pub fn reset_all() {
    let _ = std::fs::remove_file(crate::save::dir().join("achievements.txt"));
    let _ = std::fs::remove_file(global_path());
    if let Ok(rd) = std::fs::read_dir(crate::save::dir().join("Characters")) {
        for e in rd.flatten() {
            let _ = std::fs::remove_file(e.path().join("achievements.txt"));
        }
    }
    if let Ok(mut s) = STORE.lock() {
        *s = Store::default();
    }
    println!("achievements: reset");
}

/// The character changed (a new game, a load): the next look reads that character's file.
pub fn character_changed() {
    if let Ok(mut s) = STORE.lock() {
        s.loaded = false;
    }
}

/// One achievements.txt: (the character's name if written, unlocks, progress, format 2?).
fn parse(text: &str) -> (Option<String>, HashMap<String, u64>, HashMap<String, u32>, bool) {
    let (mut name, mut un, mut pr, mut f2) = (None, HashMap::new(), HashMap::new(), false);
    for line in text.lines() {
        let l = line.trim();
        if l == "format=2" {
            f2 = true;
            continue;
        }
        if let Some(n) = l.strip_prefix("name=") {
            name = Some(n.to_string());
            continue;
        }
        let Some((k, v)) = l.split_once('=') else { continue };
        let v: u64 = v.trim().parse().unwrap_or(0);
        if let Some(id) = k.strip_prefix("progress.") {
            pr.insert(id.to_string(), v as u32);
        } else {
            un.insert(k.trim().to_string(), v);
        }
    }
    (name, un, pr, f2)
}

fn load_global() -> HashMap<String, (u64, String)> {
    let mut g = HashMap::new();
    let Ok(text) = std::fs::read_to_string(global_path()) else { return g };
    for line in text.lines() {
        let Some((k, v)) = line.trim().split_once('=') else { continue };
        let (t, who) = v.split_once(';').unwrap_or((v, ""));
        if is_global(k) {
            g.insert(k.to_string(), (t.trim().parse().unwrap_or(0), who.to_string()));
        }
    }
    g
}

fn save_global(g: &HashMap<String, (u64, String)>) {
    let mut ids: Vec<_> = g.iter().collect();
    ids.sort();
    let out: String = ids.iter().map(|(k, (t, w))| format!("{k}={t};{w}\n")).collect();
    let _ = std::fs::create_dir_all(crate::save::dir());
    let _ = std::fs::write(global_path(), out);
}

fn load(s: &mut Store) {
    let who = crate::save::active_char();
    if s.loaded && s.char == who {
        return;
    }
    s.loaded = true;
    s.char = who;
    s.unlocked.clear();
    s.progress.clear();
    s.popups.clear();
    s.global = load_global();
    let Ok(text) = std::fs::read_to_string(path()) else {
        s.check_majestic();
        return;
    };
    let (_, un, pr, f2) = parse(&text);
    s.unlocked = un;
    s.progress = pr;
    // Files from before format 2 may hold mission achievements the intro / endgame maps (missions 98 / 99) handed
    // out on arrival: those are dropped (arriving in a mission unlocks the ones behind it again).
    if !f2 {
        for id in ["lady", "town", "lebedev", "choices", "breakout", "hongkong", "tanker1", "tanker2", "paree", "templars", "vandenberg", "sea", "tonjc", "hero", "majestic"] {
            s.unlocked.remove(id);
        }
        save(s);
    }
    // An ending another character reached may complete the set.
    let before = s.unlocked.contains_key("majestic");
    s.check_majestic();
    if !before && s.unlocked.contains_key("majestic") {
        save(s);
    }
}

fn save(s: &Store) {
    let mut out = String::from("format=2\n");
    if let Some(n) = crate::travel::with_session(|g| g.name.clone()).filter(|n| !n.trim().is_empty()) {
        out.push_str(&format!("name={n}\n"));
    }
    let mut ids: Vec<_> = s.unlocked.iter().collect();
    ids.sort();
    for (k, v) in ids {
        out.push_str(&format!("{k}={v}\n"));
    }
    let mut pr: Vec<_> = s.progress.iter().collect();
    pr.sort();
    for (k, v) in pr {
        out.push_str(&format!("progress.{k}={v}\n"));
    }
    let _ = std::fs::create_dir_all(crate::save::active_dir());
    let _ = std::fs::write(path(), out);
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Achievements are for the story: none in a deathmatch room or on a deathmatch map (GitHub issue #57 - spawning
/// in, with the match's augmentations and skills handed out, unlocked a row of them).
fn earning() -> bool {
    !(crate::deathmatch::versus() || crate::deathmatch::playing() || DM_MAP.load(std::sync::atomic::Ordering::Relaxed))
}

static DM_MAP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The map just loaded (loader.rs): a deathmatch map earns nothing, whoever plays it.
pub fn set_map(name: &str) {
    DM_MAP.store(crate::deathmatch::is_dm_map(name), std::sync::atomic::Ordering::Relaxed);
}

/// Unlocks an achievement (once per character; GLOBAL ones also for every character).
pub fn unlock(id: &str) {
    if !earning() {
        return;
    }
    let Ok(mut s) = STORE.lock() else { return };
    load(&mut s);
    if s.unlocked.contains_key(id) || !LIST.iter().any(|a| a.0 == id) {
        return;
    }
    let t = now_secs();
    // A GLOBAL one another character already has still goes in this character's file (the menu's list of who
    // unlocked it) but isn't announced again.
    if !s.global.contains_key(id) {
        s.popups.push(id.to_string());
    }
    s.unlocked.insert(id.to_string(), t);
    if is_global(id) && !s.global.contains_key(id) {
        let who = crate::travel::with_session(|g| g.name.clone()).unwrap_or_default();
        s.global.insert(id.to_string(), (t, who));
        save_global(&s.global);
    }
    println!("ACHIEVEMENT {id}");
    s.check_majestic();
    save(&s);
}

/// Adds to a counted achievement's progress; unlocks it at the target.
pub fn add_progress(id: &str, n: u32) {
    if !earning() {
        return;
    }
    let target = COUNTED.iter().find(|c| c.0 == id).map_or(1, |c| c.1);
    let reached = {
        let Ok(mut s) = STORE.lock() else { return };
        load(&mut s);
        if s.unlocked.contains_key(id) {
            return;
        }
        let p = s.progress.entry(id.to_string()).or_insert(0);
        *p = (*p + n).min(target);
        let r = *p >= target;
        save(&s);
        r
    };
    if reached {
        unlock(id);
    }
}

/// (unlocked unix time, progress) for the persona screen (GLOBAL ones count from any character).
pub fn state(id: &str) -> (Option<u64>, u32) {
    let Ok(mut s) = STORE.lock() else { return (None, 0) };
    load(&mut s);
    (s.when(id), s.progress.get(id).copied().unwrap_or(0))
}

pub fn target(id: &str) -> u32 {
    COUNTED.iter().find(|c| c.0 == id).map_or(0, |c| c.1)
}

pub fn unlocked_count() -> usize {
    let Ok(mut s) = STORE.lock() else { return 0 };
    load(&mut s);
    LIST.iter().filter(|a| s.has(a.0)).count()
}

/// One character's achievements for the main menu's Achievements screen.
pub struct CharAch {
    pub name: String,
    pub unlocked: HashMap<String, u64>,
}

/// Every character's achievements (Save/achievements.txt for the saves from before characters, and each
/// Save/Characters/<id>/achievements.txt), with the GLOBAL ones' first finder.
pub fn all_characters() -> (Vec<CharAch>, HashMap<String, (u64, String)>) {
    let mut v = Vec::new();
    let mut read = |p: std::path::PathBuf, id: &str| {
        let Ok(text) = std::fs::read_to_string(&p) else { return };
        let (name, unlocked, _, _) = parse(&text);
        if unlocked.is_empty() {
            return;
        }
        let name = name.unwrap_or_else(|| char_label(id));
        v.push(CharAch { name, unlocked });
    };
    read(crate::save::dir().join("achievements.txt"), "");
    if let Ok(rd) = std::fs::read_dir(crate::save::dir().join("Characters")) {
        let mut dirs: Vec<_> = rd.flatten().filter(|e| e.path().is_dir()).collect();
        dirs.sort_by_key(|e| e.file_name());
        for e in dirs {
            let id = e.file_name().to_string_lossy().to_string();
            read(e.path().join("achievements.txt"), &id);
        }
    }
    (v, load_global())
}

/// A character id ("JC_Denton-1793404800") as a name ("JC Denton").
fn char_label(id: &str) -> String {
    if id.is_empty() {
        return "Earlier saves".into();
    }
    let mut stem = id;
    while let Some((a, b)) = stem.rsplit_once('-') {
        if b.chars().all(|c| c.is_ascii_digit()) {
            stem = a;
        } else {
            break;
        }
    }
    stem.replace('_', " ").trim().to_string()
}

// ------------------------------------------------------------------------------------------------ hooks

/// An event went out (devices.rs dispatch).
pub fn event(map: &str, name: &str) {
    let map = map.to_ascii_lowercase();
    match name.to_ascii_lowercase().as_str() {
        "madebasket" => unlock("dunkin"),
        "secretdoor" if map.contains("airfieldhelibase") => unlock("pinball"),
        "alexcloset" => unlock("imin"),
        _ => {}
    }
}

/// A conversation choice was made (consys.rs).
pub fn choice(con: &str, text: &str) {
    if con.eq_ignore_ascii_case("ClubMercedesConvo1") && text.trim().eq_ignore_ascii_case("Sure.") {
        unlock("sure");
    }
    if con.eq_ignore_ascii_case("M11MeetGunther") && crate::util::contains_ci(text, "laputan machine") {
        unlock("notmachine");
    }
}

/// A pawn was taken down (ai.rs).
pub fn takedown(by_player: bool, class: &str, bind: &str, blocker: usize, explosive: bool) {
    let _ = class;
    let melee_strength = by_player && crate::perks::in_hand_melee() && crate::abilities::aug_active("AugCombat");
    takedown_with(by_player, bind, blocker, explosive, melee_strength);
}

/// takedown with what it asks the world worked out: `melee_strength` = a melee weapon in hand and Combat Strength on.
pub fn takedown_with(by_player: bool, bind: &str, blocker: usize, explosive: bool, melee_strength: bool) {
    if !by_player {
        // Aggressive Defense: a pawn killed by the blast of its own projectile, which the ADS set off.
        if explosive && ads_owner_recent(blocker) {
            unlock("bestoffense");
        }
        return;
    }
    if explosive && ads_owner_recent(blocker) {
        unlock("bestoffense");
    }
    if bind.eq_ignore_ascii_case("SickMan") {
        unlock("servant");
    }
    if melee_strength {
        add_progress("slugger", 1);
    }
}

/// Aggressive Defense set off a projectile fired by this pawn (blocker).
static ADS_OWNERS: Mutex<Vec<(usize, std::time::Instant)>> = Mutex::new(Vec::new());

pub fn ads_detonated(owner: usize) {
    if let Ok(mut v) = ADS_OWNERS.lock() {
        v.retain(|(_, t)| t.elapsed().as_secs_f32() < 2.0);
        v.push((owner, std::time::Instant::now()));
    }
}

fn ads_owner_recent(blocker: usize) -> bool {
    ADS_OWNERS.lock().ok().is_some_and(|v| v.iter().any(|(b, t)| *b == blocker && t.elapsed().as_secs_f32() < 2.0))
}

/// A weapon mod went on (weapons::fit_mod): every upgrade the weapon can take is on it.
pub fn mods_fitted(w: &crate::weapons::WeaponStats, m: &crate::weapons::ModState) {
    if all_mods_on(&w.can_mod, w.has_scope, w.silenced, w.has_laser, m) {
        unlock("lockload");
    }
}

/// Every upgrade a weapon can take is on it (scope / silencer / laser count when the weapon came with one).
pub fn all_mods_on(c: &[bool; 8], has_scope: bool, silenced: bool, has_laser: bool, m: &crate::weapons::ModState) -> bool {
    let ok = (!c[0] || m.accuracy > 0.0)
        && (!c[1] || m.clip > 0.0)
        && (!c[2] || m.range > 0.0)
        && (!c[3] || m.reload < 0.0)
        && (!c[4] || m.recoil < 0.0)
        && (!c[5] || has_scope || m.scope)
        && (!c[6] || silenced || m.silencer)
        && (!c[7] || has_laser || m.laser);
    ok && c.iter().any(|x| *x)
}

/// While the cloak / radar transparency hides the player from a pawn that's looking right at him (ai.rs).
pub fn hidden_in_view(robot: bool) {
    hidden_in_view_with(robot, crate::abilities::aug_active("AugCloak"), crate::abilities::aug_active("AugRadarTrans"));
}

pub fn hidden_in_view_with(robot: bool, cloak: bool, radar: bool) {
    if robot {
        if radar {
            unlock("automoto");
        }
    } else if cloak {
        unlock("section9");
    }
}

// ------------------------------------------------------------------------------------------------ the watcher

#[derive(Default)]
pub struct Watch {
    gen: u64,
    mission: i32,
    map: String,
    tick: f32,
    /// Augs active last tick, and the count activated during this Power Recirculator run.
    active: HashSet<String>,
    power_count: u32,
    /// Pawns already counted as snuck past.
    passed: HashSet<Entity>,
    /// States last tick (Special Eyes: a pawn going on alert).
    states: HashMap<Entity, bool>,
    /// Bodies the player has carried (Don't Feed the Wildlife).
    carried: HashSet<Entity>,
    /// The pool balls' starting heights (by mesh actor), cue ball apart.
    pool: Option<Vec<(usize, f64, bool)>>,
    popup: Option<(String, f32)>,
    layer: Option<crate::menuui::Layer>,
}

fn mission_of(lvl: &dx_level::Level) -> i32 {
    lvl.level_info()
        .and_then(|a| match unreal_pkg::props::find(&a.props, "MissionNumber") {
            Some(unreal_pkg::props::Value::Int(n)) => Some(*n),
            Some(unreal_pkg::props::Value::Byte(n)) => Some(*n as i32),
            _ => None,
        })
        .unwrap_or(0)
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn achievements_system(
    mut commands: Commands,
    time: Res<Time<Real>>,
    level: Res<LevelRes>,
    flags: Res<crate::ai::GameFlags>,
    player: Res<crate::Player>,
    inv: Res<crate::pickups::PlayerInventory>,
    weapons: Res<crate::weapons::Weapons>,
    npcs: Query<(Entity, &Npc)>,
    blockers: Res<crate::actors::Blockers>,
    decos: Query<(&crate::actors::Deco, &crate::actors::MeshActorRef)>,
    window: Single<&Window>,
    mut st: Local<Watch>,
    (mut hud, mut assets, mut images, mut layers): (
        ResMut<crate::classic_hud::ClassicHud>,
        ResMut<crate::weapons::RuntimeAssets>,
        ResMut<Assets<Image>>,
        ResMut<Assets<crate::classic_hud::HudLayer>>,
    ),
    mut nodes: Query<(&mut bevy::ui::Node, &mut Visibility, &MaterialNode<crate::classic_hud::HudLayer>)>,
) {
    let _prof = crate::prof::scope("achievements::achievements_system");
    let lvl = &level.0;
    let dt = time.delta_secs().min(0.1);
    if crate::loader::new_map(&mut st.gen) {
        st.mission = mission_of(lvl);
        st.map = lvl.name.to_ascii_lowercase();
        st.passed.clear();
        st.states.clear();
        st.carried.clear();
        st.pool = None;
        // The notice's UI layer went with the old map's entities (it was made on the first map - the intro for a
        // new game - so no notice ever showed after it).
        st.layer = None;
        on_arrival(st.mission, &st.map, &flags.0);
    }
    // ---- the unlock notice
    draw_popup(&mut commands, &mut st, dt, &window, &mut hud, &mut assets, &mut images, &mut layers, &mut nodes);
    if crate::menus::intro_map() || crate::credits::open() {
        return;
    }
    // Carried bodies (POVCorpse) are remembered for a while after they're dropped.
    if let Some(e) = weapons.corpse {
        st.carried.insert(e);
    }
    st.tick += dt;
    if st.tick < 0.25 {
        return;
    }
    let period = std::mem::take(&mut st.tick);
    check_flags(&flags.0, st.mission, inv.0.items.iter().any(|i| i.def.class == "WeaponAssaultGun"));
    // ---- skills and augmentations
    {
        let ab = crate::abilities::lock();
        if let Some(a) = ab.as_ref() {
            let skills: Vec<usize> = a.skills.list.iter().map(|s| s.level).collect();
            let augs: Vec<AugView> = a
                .augs
                .list
                .iter()
                .map(|g| AugView { class: &g.def.class, location: g.def.location as usize, level: g.level, max: g.def.max_level, boosted: g.boosted, has: g.has, active: g.active })
                .collect();
            let w = &mut *st;
            check_abilities(&skills, &augs, &mut w.active, &mut w.power_count);
        }
    }
    // ---- pawns: sneaking past, going on alert behind a wall, eating bodies
    let me = player.body.location;
    let eye = me + DVec3::new(0.0, 0.0, player.params.height * 0.8);
    let run_silent = crate::abilities::aug_active("AugStealth");
    let vision = crate::abilities::aug_active("AugVision") && crate::abilities::aug_level("AugVision").unwrap_or(0) >= 2;
    // Living Karkians / Greasels near a point (only asked when a carried body is dropped).
    let animal_near = |at: DVec3| {
        npcs.iter()
            .filter(|(_, n)| n.state != NpcState::Dead && ["Karkian", "Greasel"].iter().any(|c| crate::classtree::is_a(&lvl.actors[n.actor].class, c) || lvl.actors[n.actor].class.starts_with(c)))
            .filter_map(|(_, n)| blockers.list.get(n.blocker).map(|c| c.center))
            .any(|a| a.distance(at) < 220.0)
    };
    for (e, n) in &npcs {
        let alerted = matches!(n.state, NpcState::Seeking | NpcState::Attacking | NpcState::Surprised(_)) && (n.target == Some(Target::Player) || n.state == NpcState::Seeking);
        let was = st.states.insert(e, alerted).unwrap_or(alerted);
        if n.state == NpcState::Dead {
            // A body the player carried, dropped by a Karkian or Greasel.
            if st.carried.contains(&e) && weapons.corpse != Some(e) {
                if let Some(at) = n.body.as_ref().map(|b| b.location) {
                    if animal_near(at) {
                        unlock("wildlife");
                    }
                }
            }
            continue;
        }
        let Some(c) = blockers.list.get(n.blocker).copied() else { continue };
        if c.radius <= 0.0 || !n.hates("Player") {
            continue;
        }
        if vision && alerted && !was && lvl.collision.trace(eye, c.center, DVec3::ZERO).fraction < 0.999 {
            unlock("specialeyes");
        }
        if run_silent && !st.passed.contains(&e) && n.state == NpcState::Idle && n.readiness < 0.3 && c.center.distance(me) < 220.0 {
            st.passed.insert(e);
            add_progress("sneak", 1);
        }
    }
    // ---- the Underworld's pool table (the NYC bar)
    if st.map.contains("nyc_bar") {
        if st.pool.is_none() {
            let mut v = Vec::new();
            for (i, ma) in lvl.mesh_actors.iter().enumerate() {
                let a = &lvl.actors[ma.actor];
                if a.class.eq_ignore_ascii_case("Poolball") {
                    let cue = matches!(unreal_pkg::props::find(&a.props, "SkinColor"), Some(unreal_pkg::props::Value::Byte(15)));
                    v.push((i, a.location()[2] as f64, cue));
                }
            }
            st.pool = Some(v);
        }
        let balls = st.pool.as_ref().unwrap();
        if balls.iter().filter(|b| !b.2).count() >= 2 {
            let sunk = |i: usize, z0: f64| decos.iter().find(|(_, r)| r.0 == i).is_none_or(|(d, _)| d.body.location.z < z0 - 12.0);
            let all = balls.iter().filter(|b| !b.2).all(|b| sunk(b.0, b.1));
            let cue_up = balls.iter().filter(|b| b.2).all(|b| !sunk(b.0, b.1));
            if all && cue_up {
                unlock("pool");
            }
        }
    }
    let _ = period;
}

/// The achievements that follow from the game's flags (polled four times a second), and Nice Gun.
pub fn check_flags(f: &imsim_core::flags::FlagBase, mission: i32, has_assault_gun: bool) {
    let fl = |n: &str| f.get_bool(n);
    // ---- flags
    // Rescue the Lady: as soon as Liberty Island's mission is done (the statue cleared), not only on reaching
    // mission 2 (the notice used to come on the next map's arrival, behind its loading screen).
    if fl("StatueMissionComplete") {
        unlock("lady");
    }
    if fl("TerroristFollowup2_Played") {
        unlock("based");
    }
    if fl("SimonsInterrogating3_Played") {
        unlock("sawnothing");
    }
    if fl("TalkAboutRay_Played") {
        unlock("dibbuk");
    }
    if fl("M12MeetSandraRenton_Played") {
        unlock("renton");
    }
    if fl("RentonLateComment_Played") {
        unlock("shame");
    }
    if fl("LebedevAnnaDead_Played") {
        unlock("nerves");
    }
    if fl("AnnaExitKillphrase_Played") {
        unlock("howdid");
    }
    if fl("GreenKnowsAboutDowd") && (fl("MS_ShadyGuyUnhidden") || fl("MS_UnhideCommandos")) {
        unlock("tail");
    }
    // (Joe Greene's body flag is JoeGreene_Dead - his class's BindName. Mission08's "JoeGreen_Dead" is a typo
    // in the original script and is never set, so the achievement waited on it forever.)
    if fl("JoeGreene_Dead") {
        unlock("meanjoe");
    }
    if fl("Endgame1_Played") {
        unlock("darkage");
        unlock("area51");
    }
    if fl("Endgame2_Played") {
        unlock("helios");
        unlock("area51");
    }
    if fl("Endgame3_Played") {
        unlock("illuminati");
        unlock("area51");
    }
    if mission == 1 && has_assault_gun {
        unlock("nicegun");
    }
}

/// One augmentation as the achievements see it.
pub struct AugView<'a> {
    pub class: &'a str,
    pub location: usize,
    pub level: usize,
    pub max: usize,
    pub boosted: bool,
    pub has: bool,
    pub active: bool,
}

/// Veteran Agent, Upgrades, Biomachina, Atom Heart Son and Wardenclyffe Man from the skills and augmentations.
/// `active` / `power_count`: the running augs last time and the activations counted during this Power Recirculator run.
pub fn check_abilities(skills: &[usize], augs: &[AugView], active: &mut HashSet<String>, power_count: &mut u32) {
    if skills.iter().filter(|l| **l >= 3).count() >= 2 {
        unlock("veteran");
    }
    let mut locs = [false; 6];
    for g in augs.iter().filter(|g| g.has) {
        if g.location < 6 {
            locs[g.location] = true;
        }
        if g.level >= g.max && !g.boosted && g.max > 0 {
            unlock("upgrades");
        }
    }
    if locs.iter().all(|x| *x) {
        unlock("biomachina");
    }
    if augs.iter().filter(|g| g.boosted).count() >= 3 {
        add_progress("atomheart", 3);
    }
    // The running augs, compared with last time's (rebuilt only when they change).
    let on = || augs.iter().filter(|g| g.active).map(|g| g.class);
    if on().any(|c| c == "AugPower") {
        // (The ones already on when it came on don't count; it's activations while it runs.)
        let newly = on().filter(|c| *c != "AugPower" && !active.contains(*c)).count() as u32;
        if active.contains("AugPower") {
            *power_count += newly;
        }
        if *power_count >= 3 {
            add_progress("wardenclyffe", 3);
        }
    } else {
        *power_count = 0;
    }
    if on().count() != active.len() || on().any(|c| !active.contains(c)) {
        active.clear();
        active.extend(on().map(str::to_owned));
    }
}

#[cfg(test)]
pub fn arrival_for_test(mission: i32, map: &str, f: &imsim_core::flags::FlagBase) {
    on_arrival(mission, map, f);
}

/// Mission arrivals: the mission before this one is done; and the story checks made on the way in.
fn on_arrival(mission: i32, map: &str, f: &imsim_core::flags::FlagBase) {
    // Only the story's missions: the intro and the endgame cut scenes are missions 98 / 99, the title -2.
    if !(1..=15).contains(&mission) {
        return;
    }
    let done = [(2, "lady"), (3, "town"), (4, "lebedev"), (5, "choices"), (6, "breakout"), (8, "hongkong"), (9, "tanker1"), (10, "tanker2"), (11, "paree"), (12, "templars"), (14, "vandenberg"), (15, "sea")];
    for (m, id) in done {
        if mission >= m {
            unlock(id);
        }
    }
    if mission == 5 && !f.get_bool("PaulDenton_Dead") {
        unlock("tonjc");
    }
    if map.starts_with("05_nyc_unatcohq") && f.get_bool("MiguelFollowing") && !f.get_bool("Miguel_Dead") {
        unlock("escapa");
    }
    if mission >= 15 && !f.get_bool("PaulDenton_Dead") && !f.get_bool("DL_JockDeath_Played") && !f.get_bool("SandraRenton_Dead") && f.get_bool("MeetSandraRenton_Played") {
        unlock("hero");
    }
}

// ------------------------------------------------------------------------------------------------ the notice

#[allow(clippy::too_many_arguments)]
fn draw_popup(
    commands: &mut Commands,
    st: &mut Watch,
    dt: f32,
    window: &Window,
    hud: &mut crate::classic_hud::ClassicHud,
    assets: &mut crate::weapons::RuntimeAssets,
    images: &mut Assets<Image>,
    layers: &mut Assets<crate::classic_hud::HudLayer>,
    nodes: &mut Query<(&mut bevy::ui::Node, &mut Visibility, &MaterialNode<crate::classic_hud::HudLayer>)>,
) {
    // Not behind a loading screen or a menu: the notice waits.
    if st.popup.is_none() && (crate::loader::settling() || crate::loader::busy() || crate::menus::open()) {
        return;
    }
    if st.popup.is_none() {
        let next = STORE.lock().ok().and_then(|mut s| (!s.popups.is_empty()).then(|| s.popups.remove(0)));
        if let Some(id) = next {
            crate::sound::play_player("DeusExSounds.UserInterface.LogGoalAdded", 1.0, 0.0);
            st.popup = Some((id, 0.0));
        }
    }
    let layer = st.layer.get_or_insert_with(|| crate::menuui::layer(commands, images, layers, 30));
    let Some((id, t)) = st.popup.as_mut() else {
        if let Ok((_, mut v, _)) = nodes.get_mut(layer.e) {
            v.set_if_neq(Visibility::Hidden);
        }
        return;
    };
    *t += dt;
    const LIFE: f32 = 5.0;
    if *t > LIFE {
        st.popup = None;
        if let Ok((_, mut v, _)) = nodes.get_mut(layer.e) {
            v.set_if_neq(Visibility::Hidden);
        }
        return;
    }
    let s = crate::hud::ui_scale(window) as i32;
    let (w, h) = (280, 46);
    let mut c = Canvas::new(w, h, s.max(1));
    let Some((name, _)) = LIST.iter().find(|a| a.0 == id.as_str()).map(|a| (a.1, a.2)) else { return };
    let (Some(head), Some(small)) = (hud.font(assets, "FontMenuHeaders"), hud.font(assets, "FontMenuSmall")) else { return };
    let col = crate::menuui::h_col(9);
    frame_box(&mut c, 0, 0, w, h, col);
    let icon = icon(LIST.iter().position(|a| a.0 == id.as_str()).unwrap_or(0), true, col);
    c.texture(5, 5, 36, 36, 0, 0, &icon, Style::Masked, [255, 255, 255]);
    c.text(&small, 48, 6, w - 54, small.height, HAlign::Left, VAlign::Top, col, "ACHIEVEMENT UNLOCKED");
    c.text(&head, 48, 22, w - 54, head.height, HAlign::Left, VAlign::Top, [255, 255, 255], name);
    // Slides down from the top, back up at the end.
    let k = (*t / 0.3).min(1.0).min(((LIFE - *t) / 0.3).max(0.0));
    let sf = window.scale_factor();
    let d = s as f32;
    let pw = window.physical_width() as f32 / d;
    let y = -(h as f32) + (h as f32 + 12.0) * k * (2.0 - k);
    crate::menuui::place(layer, &c, (((pw - w as f32) / 2.0) as i32, y as i32), d, sf, images, layers, nodes);
}

/// A window frame in the HUD's manner: dark glass, a light bevel top-left, shadow bottom-right.
fn frame_box(c: &mut Canvas, x: i32, y: i32, w: i32, h: i32, col: [u8; 3]) {
    c.fill(x, y, w, h, Style::Normal, [14, 16, 19]);
    let lite = col.map(|v| (v as u16 * 3 / 4) as u8);
    let dark = col.map(|v| v / 4);
    c.fill(x, y, w, 1, Style::Normal, lite);
    c.fill(x, y, 1, h, Style::Normal, lite);
    c.fill(x, y + h - 1, w, 1, Style::Normal, dark);
    c.fill(x + w - 1, y, 1, h, Style::Normal, dark);
    c.fill(x + 2, y + 2, w - 4, 1, Style::Normal, dark);
}

// ------------------------------------------------------------------------------------------------ icons

/// Shapes in a 0..1 square (y down).
#[derive(Clone, Copy)]
enum S {
    /// Disc (cx, cy, r).
    C(f32, f32, f32),
    /// Ring (cx, cy, r, width).
    O(f32, f32, f32, f32),
    /// Box (x0, y0, x1, y1).
    R(f32, f32, f32, f32),
    /// Thick segment (x0, y0, x1, y1, width).
    L(f32, f32, f32, f32, f32),
    /// Triangle.
    T(f32, f32, f32, f32, f32, f32),
    /// Cut a disc out of what's drawn so far.
    XC(f32, f32, f32),
    /// Cut a box.
    XR(f32, f32, f32, f32),
}

fn inside(s: S, x: f32, y: f32) -> bool {
    match s {
        S::C(cx, cy, r) | S::XC(cx, cy, r) => (x - cx).powi(2) + (y - cy).powi(2) <= r * r,
        S::O(cx, cy, r, w) => {
            let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
            d >= r - w * 0.5 && d <= r + w * 0.5
        }
        S::R(x0, y0, x1, y1) | S::XR(x0, y0, x1, y1) => x >= x0 && x <= x1 && y >= y0 && y <= y1,
        S::L(x0, y0, x1, y1, w) => {
            let (dx, dy) = (x1 - x0, y1 - y0);
            let t = (((x - x0) * dx + (y - y0) * dy) / (dx * dx + dy * dy).max(1e-6)).clamp(0.0, 1.0);
            let (px, py) = (x0 + dx * t, y0 + dy * t);
            (x - px).powi(2) + (y - py).powi(2) <= (w * 0.5).powi(2)
        }
        S::T(ax, ay, bx, by, cx, cy) => {
            let d = |px: f32, py: f32, qx: f32, qy: f32| (x - qx) * (py - qy) - (px - qx) * (y - qy);
            let (d1, d2, d3) = (d(ax, ay, bx, by), d(bx, by, cx, cy), d(cx, cy, ax, ay));
            let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
            let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
            !(neg && pos)
        }
    }
}

/// Each achievement's picture: a few shapes.
fn glyph(i: usize) -> Vec<S> {
    use S::*;
    match LIST[i].0 {
        // A laurel of stars round a pyramid.
        "majestic" => vec![T(0.5, 0.2, 0.2, 0.75, 0.8, 0.75), XR(0.2, 0.42, 0.8, 0.46), O(0.5, 0.5, 0.42, 0.05)],
        // The Statue's torch.
        "lady" => vec![R(0.44, 0.45, 0.56, 0.85), T(0.36, 0.45, 0.64, 0.45, 0.5, 0.58), C(0.5, 0.33, 0.1), T(0.42, 0.3, 0.58, 0.3, 0.5, 0.12)],
        // A generator's bolt.
        "town" => vec![T(0.55, 0.12, 0.3, 0.55, 0.52, 0.52), T(0.48, 0.48, 0.7, 0.45, 0.45, 0.88)],
        // A jet.
        "lebedev" => vec![L(0.5, 0.12, 0.5, 0.88, 0.1), T(0.5, 0.4, 0.12, 0.62, 0.88, 0.62), T(0.5, 0.75, 0.35, 0.88, 0.65, 0.88)],
        // A fork in the road.
        "choices" => vec![L(0.5, 0.88, 0.5, 0.5, 0.1), L(0.5, 0.52, 0.22, 0.15, 0.1), L(0.5, 0.52, 0.78, 0.15, 0.1)],
        // A broken chain.
        "breakout" => vec![O(0.3, 0.6, 0.15, 0.07), O(0.7, 0.4, 0.15, 0.07), XR(0.42, 0.3, 0.58, 0.7)],
        // The Dragon's Tooth.
        "hongkong" => vec![T(0.5, 0.08, 0.42, 0.62, 0.58, 0.62), R(0.3, 0.62, 0.7, 0.68), R(0.46, 0.68, 0.54, 0.9)],
        // A tanker.
        "tanker1" => vec![T(0.08, 0.55, 0.92, 0.55, 0.8, 0.72), R(0.2, 0.55, 0.8, 0.72), R(0.62, 0.35, 0.78, 0.55)],
        // A tanker going down.
        "tanker2" => vec![T(0.15, 0.4, 0.85, 0.62, 0.75, 0.75), L(0.08, 0.82, 0.92, 0.82, 0.06), L(0.18, 0.92, 0.82, 0.92, 0.04)],
        // The Eiffel tower.
        "paree" => vec![T(0.5, 0.08, 0.25, 0.9, 0.75, 0.9), XR(0.38, 0.62, 0.62, 0.92), XR(0.46, 0.25, 0.54, 0.6), R(0.3, 0.58, 0.7, 0.62)],
        // A cross pattee.
        "templars" => vec![T(0.5, 0.5, 0.32, 0.1, 0.68, 0.1), T(0.5, 0.5, 0.32, 0.9, 0.68, 0.9), T(0.5, 0.5, 0.1, 0.32, 0.1, 0.68), T(0.5, 0.5, 0.9, 0.32, 0.9, 0.68)],
        // A dish.
        "vandenberg" => vec![O(0.45, 0.45, 0.3, 0.08), XR(0.45, 0.0, 1.0, 1.0), L(0.45, 0.45, 0.75, 0.2, 0.06), L(0.4, 0.6, 0.6, 0.9, 0.1), R(0.3, 0.86, 0.75, 0.92)],
        // A submarine.
        "sea" => vec![C(0.3, 0.6, 0.15), C(0.7, 0.6, 0.15), R(0.3, 0.45, 0.7, 0.75), R(0.45, 0.3, 0.6, 0.45), L(0.55, 0.3, 0.55, 0.18, 0.04)],
        // The globe.
        "area51" => vec![O(0.5, 0.5, 0.36, 0.07), L(0.5, 0.14, 0.5, 0.86, 0.05), L(0.14, 0.5, 0.86, 0.5, 0.05), O(0.5, 0.5, 0.18, 0.05)],
        // Helios: a sun.
        "helios" => vec![C(0.5, 0.5, 0.2), L(0.5, 0.06, 0.5, 0.22, 0.06), L(0.5, 0.78, 0.5, 0.94, 0.06), L(0.06, 0.5, 0.22, 0.5, 0.06), L(0.78, 0.5, 0.94, 0.5, 0.06), L(0.19, 0.19, 0.3, 0.3, 0.06), L(0.7, 0.7, 0.81, 0.81, 0.06), L(0.81, 0.19, 0.7, 0.3, 0.06), L(0.3, 0.7, 0.19, 0.81, 0.06)],
        // A cut cable.
        "darkage" => vec![L(0.1, 0.5, 0.42, 0.5, 0.1), L(0.58, 0.5, 0.9, 0.5, 0.1), T(0.42, 0.38, 0.5, 0.5, 0.42, 0.62), L(0.58, 0.4, 0.66, 0.32, 0.04), L(0.6, 0.6, 0.68, 0.68, 0.04)],
        // The eye in the pyramid.
        "illuminati" => vec![T(0.5, 0.12, 0.12, 0.85, 0.88, 0.85), XC(0.5, 0.55, 0.16), C(0.5, 0.55, 0.07)],
        // A body with its augmentation points.
        "biomachina" => vec![C(0.5, 0.18, 0.09), L(0.5, 0.3, 0.5, 0.62, 0.12), L(0.5, 0.36, 0.25, 0.55, 0.07), L(0.5, 0.36, 0.75, 0.55, 0.07), L(0.5, 0.62, 0.35, 0.9, 0.08), L(0.5, 0.62, 0.65, 0.9, 0.08), XC(0.5, 0.45, 0.04)],
        // A fist.
        "slugger" => vec![R(0.28, 0.35, 0.72, 0.72), XR(0.38, 0.35, 0.39, 0.5), XR(0.5, 0.35, 0.51, 0.5), XR(0.61, 0.35, 0.62, 0.5), C(0.28, 0.6, 0.1), R(0.36, 0.72, 0.64, 0.88)],
        // Footprints.
        "sneak" => vec![C(0.35, 0.65, 0.1), C(0.35, 0.82, 0.06), C(0.65, 0.3, 0.1), C(0.65, 0.47, 0.06)],
        // A figure fading out.
        "section9" => vec![O(0.5, 0.22, 0.1, 0.04), L(0.5, 0.34, 0.5, 0.62, 0.04), L(0.5, 0.62, 0.35, 0.9, 0.04), L(0.5, 0.62, 0.65, 0.9, 0.04), L(0.5, 0.42, 0.28, 0.55, 0.04), L(0.5, 0.42, 0.72, 0.55, 0.04)],
        // A robot's head.
        "automoto" => vec![R(0.25, 0.3, 0.75, 0.75), XC(0.38, 0.48, 0.07), XC(0.62, 0.48, 0.07), XR(0.36, 0.62, 0.64, 0.66), L(0.5, 0.3, 0.5, 0.14, 0.04), C(0.5, 0.12, 0.05)],
        // Two coils and a spark.
        "wardenclyffe" => vec![C(0.2, 0.3, 0.1), C(0.8, 0.3, 0.1), R(0.17, 0.4, 0.23, 0.9), R(0.77, 0.4, 0.83, 0.9), L(0.3, 0.3, 0.45, 0.4, 0.04), L(0.45, 0.4, 0.55, 0.22, 0.04), L(0.55, 0.22, 0.7, 0.3, 0.04)],
        // A heart in an orbit.
        "atomheart" => vec![C(0.4, 0.42, 0.13), C(0.6, 0.42, 0.13), T(0.27, 0.47, 0.73, 0.47, 0.5, 0.78), O(0.5, 0.5, 0.42, 0.04)],
        // A rocket turned back.
        "bestoffense" => vec![L(0.75, 0.25, 0.35, 0.65, 0.12), T(0.25, 0.75, 0.25, 0.5, 0.5, 0.75), L(0.6, 0.2, 0.85, 0.2, 0.04), L(0.85, 0.2, 0.85, 0.45, 0.04)],
        // An eye through a wall.
        "specialeyes" => vec![R(0.45, 0.1, 0.55, 0.9), O(0.5, 0.5, 0.3, 0.06), C(0.5, 0.5, 0.11)],
        // A single round.
        "servant" => vec![R(0.42, 0.35, 0.58, 0.85), C(0.5, 0.35, 0.08), XR(0.42, 0.7, 0.58, 0.72)],
        // A basketball.
        "dunkin" => vec![C(0.5, 0.5, 0.36), XR(0.48, 0.14, 0.52, 0.86), XR(0.14, 0.48, 0.86, 0.52), XC(0.5, 0.5, 0.02)],
        // An open door.
        "escapa" => vec![R(0.25, 0.12, 0.75, 0.9), XR(0.32, 0.18, 0.68, 0.9), T(0.32, 0.18, 0.55, 0.28, 0.32, 0.9), T(0.55, 0.28, 0.55, 0.82, 0.32, 0.9)],
        // A closed eye.
        "sawnothing" => vec![O(0.5, 0.3, 0.36, 0.06), XR(0.0, 0.0, 1.0, 0.5), L(0.3, 0.68, 0.25, 0.78, 0.04), L(0.5, 0.72, 0.5, 0.84, 0.04), L(0.7, 0.68, 0.75, 0.78, 0.04)],
        // A key.
        "imin" => vec![O(0.3, 0.5, 0.14, 0.07), L(0.44, 0.5, 0.88, 0.5, 0.08), L(0.78, 0.5, 0.78, 0.65, 0.06), L(0.68, 0.5, 0.68, 0.62, 0.06)],
        // A ball and a flipper.
        "pinball" => vec![C(0.62, 0.3, 0.12), L(0.15, 0.78, 0.55, 0.65, 0.09), C(0.15, 0.78, 0.07)],
        // A paw.
        "wildlife" => vec![C(0.5, 0.62, 0.16), C(0.28, 0.4, 0.07), C(0.42, 0.28, 0.07), C(0.58, 0.28, 0.07), C(0.72, 0.4, 0.07)],
        // A ghost.
        "dibbuk" => vec![C(0.5, 0.4, 0.25), R(0.25, 0.4, 0.75, 0.82), XC(0.35, 0.85, 0.08), XC(0.65, 0.85, 0.08), XC(0.42, 0.38, 0.05), XC(0.58, 0.38, 0.05)],
        // A credit chit.
        "sure" => vec![R(0.18, 0.3, 0.82, 0.7), XR(0.24, 0.36, 0.76, 0.64), R(0.3, 0.45, 0.7, 0.48), R(0.3, 0.52, 0.55, 0.55)],
        // A suitcase.
        "renton" => vec![R(0.2, 0.38, 0.8, 0.8), O(0.5, 0.36, 0.1, 0.05), XR(0.0, 0.36, 1.0, 1.0), R(0.2, 0.38, 0.8, 0.8), XR(0.48, 0.5, 0.52, 0.68)],
        // A shield and star.
        "hero" => vec![R(0.25, 0.15, 0.75, 0.55), T(0.25, 0.55, 0.75, 0.55, 0.5, 0.9), XC(0.5, 0.45, 0.12)],
        // A magnifier.
        "tail" => vec![O(0.42, 0.42, 0.2, 0.07), L(0.56, 0.56, 0.85, 0.85, 0.09)],
        // A newspaper.
        "meanjoe" => vec![R(0.18, 0.2, 0.82, 0.8), XR(0.24, 0.26, 0.76, 0.74), R(0.28, 0.3, 0.72, 0.4), R(0.28, 0.46, 0.48, 0.7), R(0.52, 0.46, 0.72, 0.5), R(0.52, 0.56, 0.72, 0.6), R(0.52, 0.66, 0.72, 0.7)],
        // A speech balloon.
        "based" => vec![C(0.5, 0.42, 0.3), T(0.3, 0.6, 0.45, 0.68, 0.2, 0.88), XR(0.3, 0.38, 0.7, 0.42), XR(0.3, 0.48, 0.6, 0.52)],
        // A rifle.
        "nicegun" => vec![R(0.1, 0.42, 0.78, 0.52), R(0.78, 0.44, 0.92, 0.5), R(0.38, 0.52, 0.48, 0.7), T(0.1, 0.42, 0.1, 0.62, 0.25, 0.52), R(0.55, 0.36, 0.7, 0.42)],
        // Crosshairs.
        "nerves" => vec![O(0.5, 0.5, 0.3, 0.06), L(0.5, 0.08, 0.5, 0.36, 0.05), L(0.5, 0.64, 0.5, 0.92, 0.05), L(0.08, 0.5, 0.36, 0.5, 0.05), L(0.64, 0.5, 0.92, 0.5, 0.05)],
        // Two figures side by side.
        "tonjc" => vec![C(0.33, 0.3, 0.1), R(0.23, 0.42, 0.43, 0.85), C(0.67, 0.3, 0.1), R(0.57, 0.42, 0.77, 0.85)],
        // Two asterisks.
        "howdid" => vec![L(0.3, 0.3, 0.3, 0.7, 0.07), L(0.13, 0.4, 0.47, 0.6, 0.07), L(0.13, 0.6, 0.47, 0.4, 0.07), L(0.7, 0.3, 0.7, 0.7, 0.07), L(0.53, 0.4, 0.87, 0.6, 0.07), L(0.53, 0.6, 0.87, 0.4, 0.07)],
        // A broken gear.
        "notmachine" => vec![C(0.5, 0.5, 0.28), R(0.44, 0.1, 0.56, 0.9), R(0.1, 0.44, 0.9, 0.56), XC(0.5, 0.5, 0.12), XR(0.48, 0.5, 1.0, 0.54)],
        // A falling tear.
        "shame" => vec![C(0.5, 0.62, 0.2), T(0.5, 0.15, 0.32, 0.55, 0.68, 0.55)],
        // The eight ball.
        "pool" => vec![C(0.5, 0.5, 0.36), XC(0.5, 0.42, 0.14), C(0.5, 0.36, 0.045), C(0.5, 0.48, 0.055)],
        // Two chevrons.
        "veteran" => vec![L(0.2, 0.42, 0.5, 0.22, 0.09), L(0.5, 0.22, 0.8, 0.42, 0.09), L(0.2, 0.68, 0.5, 0.48, 0.09), L(0.5, 0.48, 0.8, 0.68, 0.09)],
        // A gun and a plus.
        "lockload" => vec![R(0.08, 0.55, 0.62, 0.64), R(0.25, 0.64, 0.33, 0.8), R(0.68, 0.18, 0.76, 0.48), R(0.57, 0.29, 0.87, 0.37)],
        // Three rising arrows.
        "upgrades" => vec![T(0.5, 0.1, 0.25, 0.32, 0.75, 0.32), T(0.5, 0.36, 0.25, 0.58, 0.75, 0.58), T(0.5, 0.62, 0.25, 0.84, 0.75, 0.84), XC(0.5, 0.36, 0.0)],
        _ => vec![C(0.5, 0.5, 0.3)],
    }
}

/// An achievement's icon, 36 x 36: the picture lit in the theme's colour on dark glass in a bevelled frame
/// (unlocked), or grey with a padlock (locked).
pub fn icon(i: usize, unlocked: bool, col: [u8; 3]) -> UiTex {
    const N: usize = 36;
    const SS: usize = 4;
    let shapes = glyph(i.min(LIST.len() - 1));
    let mut cov = vec![0f32; N * N];
    for py in 0..N {
        for px in 0..N {
            let mut hit = 0;
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32 - 4.0) / (N as f32 - 8.0);
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32 - 4.0) / (N as f32 - 8.0);
                    let mut on = false;
                    for s in &shapes {
                        match s {
                            S::XC(..) | S::XR(..) => {
                                if inside(*s, x, y) {
                                    on = false;
                                }
                            }
                            _ => {
                                if inside(*s, x, y) {
                                    on = true;
                                }
                            }
                        }
                    }
                    hit += on as u32;
                }
            }
            cov[py * N + px] = hit as f32 / (SS * SS) as f32;
        }
    }
    let base = if unlocked { col } else { [96, 100, 104] };
    let hi = if unlocked { [255, 255, 255] } else { [140, 144, 148] };
    let mut px = vec![[0u8; 4]; N * N];
    for y in 0..N {
        for x in 0..N {
            // Glass: dark, a little lighter at the top.
            let g = 18.0 + 14.0 * (1.0 - y as f32 / N as f32);
            let mut c = [g, g + 2.0, g + 5.0];
            // Frame: light top / left, dark bottom / right.
            let edge = x == 0 || y == 0 || x == N - 1 || y == N - 1;
            if edge {
                let v = if x == 0 || y == 0 { base.map(|b| b as f32 * 0.8) } else { base.map(|b| b as f32 * 0.3) };
                c = v;
            } else if x == 1 || y == 1 || x == N - 2 || y == N - 2 {
                c = [8.0, 9.0, 11.0];
            }
            // The picture: theme colour shading to white at the top, with a soft dark outline under it.
            let k = cov[y * N + x];
            let below = if y > 0 && x > 0 { cov[(y - 1) * N + (x - 1)] } else { 0.0 };
            if !edge {
                let shade = below * (1.0 - k) * 0.6;
                c = c.map(|v| v * (1.0 - shade));
                let t = 1.0 - y as f32 / N as f32;
                let fg = [0, 1, 2].map(|j| base[j] as f32 + (hi[j] as f32 - base[j] as f32) * t * 0.55);
                c = [0, 1, 2].map(|j| c[j] + (fg[j] - c[j]) * k);
            }
            px[y * N + x] = [c[0].clamp(0.0, 255.0) as u8, c[1].clamp(0.0, 255.0) as u8, c[2].clamp(0.0, 255.0) as u8, 255];
        }
    }
    if !unlocked {
        // A small padlock in the corner.
        let lock = [S::O(0.84, 0.74, 0.07, 0.04), S::R(0.74, 0.76, 0.94, 0.94)];
        for y in 0..N {
            for x in 0..N {
                let (fx, fy) = ((x as f32 + 0.5) / N as f32, (y as f32 + 0.5) / N as f32);
                if lock.iter().any(|s| inside(*s, fx, fy)) {
                    px[y * N + x] = [200, 170, 60, 255];
                }
            }
        }
    }
    UiTex { w: N as u32, h: N as u32, px }
}

/// The persona Achievements screen's list (inside the Logs screen's frame): returns the scroll range.
#[allow(clippy::too_many_arguments)]
pub fn draw_list(c: &mut Canvas, head: &crate::uicanvas::UiFont, small: &crate::uicanvas::UiFont, x: i32, y: i32, w: i32, h: i32, scroll: i32) -> i32 {
    let global = {
        let Ok(mut s) = STORE.lock() else { return 0 };
        load(&mut s);
        s.global.clone()
    };
    draw_rows(c, head, small, (x, y, w, h), scroll, &|id| {
        let (when, prog) = state(id);
        // A GLOBAL one another character reached: say who.
        let by = global.get(id).filter(|g| !g.1.is_empty() && crate::travel::with_session(|s| s.name != g.1).unwrap_or(true));
        (when, prog, by.map(|g| format!("All characters: reached by {}", g.1)))
    })
}

/// The main menu's Achievements screen: every character's. Returns the scroll range.
#[allow(clippy::too_many_arguments)]
pub fn draw_all(
    c: &mut Canvas,
    head: &crate::uicanvas::UiFont,
    small: &crate::uicanvas::UiFont,
    r: (i32, i32, i32, i32),
    scroll: i32,
    chars: &[CharAch],
    global: &HashMap<String, (u64, String)>,
) -> i32 {
    draw_rows(c, head, small, r, scroll, &|id| {
        let who: Vec<&str> = chars.iter().filter(|ch| ch.unlocked.contains_key(id)).map(|ch| ch.name.as_str()).collect();
        let first = chars.iter().filter_map(|ch| ch.unlocked.get(id).copied()).min().or_else(|| global.get(id).map(|g| g.0));
        let line = if who.is_empty() {
            None
        } else if is_global(id) {
            Some(format!("All characters (first: {})", global.get(id).map_or(who[0], |g| if g.1.is_empty() { who[0] } else { g.1.as_str() })))
        } else {
            Some(format!("Unlocked by: {}", who.join(", ")))
        };
        (first, 0, line)
    })
}

pub const ROW: i32 = 46;

/// The rows: icon, name, date / progress / Locked, the goal (and an extra line when given). Spoilers that are still
/// locked are drawn, then blurred and dimmed with a padlock over the middle.
#[allow(clippy::type_complexity)]
fn draw_rows(
    c: &mut Canvas,
    head: &crate::uicanvas::UiFont,
    small: &crate::uicanvas::UiFont,
    (x, y, w, h): (i32, i32, i32, i32),
    scroll: i32,
    info: &dyn Fn(&str) -> (Option<u64>, u32, Option<String>),
) -> i32 {
    let col = crate::menuui::h_col(9);
    c.set_clip(Some((x, y, w, h)));
    let lock = lock_tex(col);
    for (i, (id, name, goal)) in LIST.iter().enumerate() {
        let ry = y + i as i32 * ROW - scroll;
        if ry + ROW < y || ry > y + h {
            continue;
        }
        let (when, prog, extra) = info(id);
        let on = when.is_some();
        let ic = icon(i, on, col);
        c.texture(x + 4, ry + 5, 36, 36, 0, 0, &ic, Style::Masked, [255, 255, 255]);
        let title_col = if on { [255, 255, 255] } else { [150, 154, 158] };
        c.text(head, x + 46, ry + 4, w - 140, head.height, HAlign::Left, VAlign::Top, title_col, name);
        let status = match (when, target(id)) {
            (Some(t), _) => date(t),
            (None, n) if n > 0 => format!("{prog} / {n}"),
            _ => "Locked".to_string(),
        };
        c.text(small, x + w - 96, ry + 4, 90, small.height, HAlign::Right, VAlign::Top, if on { col } else { [120, 124, 128] }, &status);
        let lines = crate::uicanvas::wrap(small, goal, w - 54);
        let n = if extra.is_some() { 1 } else { 2 };
        let gy = ry + 4 + head.height + 2;
        for (k, l) in lines.iter().take(n).enumerate() {
            c.text(small, x + 46, gy + k as i32 * small.height, w - 54, small.height, HAlign::Left, VAlign::Top, if on { col } else { [128, 132, 136] }, l);
        }
        if let Some(e) = &extra {
            c.text(small, x + 46, gy + small.height, w - 54, small.height, HAlign::Left, VAlign::Top, [200, 200, 200], e);
        }
        if !on && is_spoiler(id) {
            // Everything but the status column, so "Locked" stays readable.
            let (bx, by) = (x + 2, ry.max(y));
            let bh = (ry + ROW - 1).min(y + h) - by;
            c.blur(bx, by, w - 100, bh, 3, 0.1);
            let (lx, ly) = (x + (w - 100) / 2 - lock.w as i32 / 2, ry + (ROW - lock.h as i32) / 2);
            c.texture(lx, ly, lock.w as i32, lock.h as i32, 0, 0, &lock, Style::Masked, [255, 255, 255]);
        }
        c.fill(x + 2, ry + ROW - 1, w - 4, 1, Style::Normal, [40, 44, 48]);
    }
    c.set_clip(None);
    (LIST.len() as i32 * ROW - h).max(0)
}

/// A padlock, 18 x 22, in the theme colour with a dark outline (over blurred spoilers).
fn lock_tex(col: [u8; 3]) -> UiTex {
    const W: usize = 18;
    const H: usize = 22;
    const SS: usize = 4;
    // Unit square shapes (y down): the shackle ring cut below its middle, the body, a keyhole.
    let shapes = [S::O(0.5, 0.36, 0.26, 0.13)];
    let body = S::R(0.12, 0.42, 0.88, 0.96);
    let hole = [S::C(0.5, 0.62, 0.08), S::R(0.46, 0.62, 0.54, 0.8)];
    let mut cov = vec![(0f32, 0f32); W * H];
    for py in 0..H {
        for px in 0..W {
            let (mut fill, mut cut) = (0, 0);
            for sy in 0..SS {
                for sx in 0..SS {
                    let fx = (px as f32 + (sx as f32 + 0.5) / SS as f32) / W as f32;
                    let fy = (py as f32 + (sy as f32 + 0.5) / SS as f32) / H as f32;
                    let ring = shapes.iter().any(|s| inside(*s, fx, fy)) && fy < 0.45;
                    let b = inside(body, fx, fy);
                    let k = hole.iter().any(|s| inside(*s, fx, fy));
                    if (ring || b) && !(b && k) {
                        fill += 1;
                    }
                    if b && k {
                        cut += 1;
                    }
                }
            }
            cov[py * W + px] = (fill as f32 / (SS * SS) as f32, cut as f32 / (SS * SS) as f32);
        }
    }
    let mut px = vec![[0u8; 4]; W * H];
    for y in 0..H {
        for x in 0..W {
            let (k, hole) = cov[y * W + x];
            // Outline: anything next to the lock.
            let near = (-1i32..=1).any(|dy| {
                (-1i32..=1).any(|dx| {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    nx >= 0 && ny >= 0 && (nx as usize) < W && (ny as usize) < H && cov[ny as usize * W + nx as usize].0 > 0.3
                })
            });
            let t = 1.0 - y as f32 / H as f32;
            let fg = [0, 1, 2].map(|j| col[j] as f32 + (255.0 - col[j] as f32) * t * 0.5);
            if k > 0.3 {
                px[y * W + x] = [fg[0] as u8, fg[1] as u8, fg[2] as u8, 255];
            } else if hole > 0.3 || near {
                px[y * W + x] = [10, 11, 13, 255];
            }
        }
    }
    UiTex { w: W as u32, h: H as u32, px }
}

/// A unix time as a calendar date (UTC), MM/DD/YYYY.
fn date(t: u64) -> String {
    let days = (t / 86400) as i64;
    // Civil-from-days (Howard Hinnant).
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
    format!("{m:02}/{d:02}/{y}")
}

#[cfg(test)]
mod tests {
    /// ACH_SHEET=out.rgba cargo test -p dxview icon_sheet: every icon, 10 x 5 (unlocked; locked in the second half).
    #[test]
    fn icon_sheet() {
        let Ok(out) = std::env::var("ACH_SHEET") else { return };
        let (cols, n) = (10usize, super::LIST.len());
        let rows = n.div_ceil(cols) * 2;
        let (w, h) = (cols * 40, rows * 40);
        let mut px = vec![0u8; w * h * 4];
        for i in 0..n * 2 {
            let t = super::icon(i % n, i < n, [200, 210, 225]);
            let (gx, gy) = ((i % n) % cols * 40 + 2, ((i % n) / cols + if i < n { 0 } else { rows / 2 }) * 40 + 2);
            for y in 0..36 {
                for x in 0..36 {
                    let p = t.px[y * 36 + x];
                    let o = ((gy + y) * w + gx + x) * 4;
                    px[o..o + 4].copy_from_slice(&p);
                }
            }
        }
        std::fs::write(&out, &px).unwrap();
        println!("{w}x{h}");
    }

    #[test]
    fn dates_and_icons() {
        assert_eq!(super::date(0), "01/01/1970");
        assert_eq!(super::date(1_793_404_800), "10/31/2026");
        for i in 0..super::LIST.len() {
            let t = super::icon(i, true, [200, 210, 220]);
            assert_eq!(t.px.len(), 36 * 36);
        }
        let ids: std::collections::HashSet<_> = super::LIST.iter().map(|a| a.0).collect();
        assert_eq!(ids.len(), 50);
    }
}
