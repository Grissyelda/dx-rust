//! Deathmatch characters (ours): a name, a body (male, or Lay D Denton's female JC when that mod is installed),
//! one of the five faces, an Augmentique outfit, a skin per weapon, and the kills that earn those skins. Kept apart
//! from story characters, one file each: Save/Multiplayer/Deathmatch/<id>.ron.
//!
//! Weapon skins are Augmentique's (WeaponSkinManager's list, in its order): in deathmatch the n-th skin of a weapon
//! is earned by data/deathmatch.ron's skin_unlocks[n] kills with that weapon (10, 25, 50...). The skins chosen
//! here are the ones in your hands and on your body's weapon for everyone else.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct DmChar {
    pub id: String,
    pub name: String,
    /// Lay D Denton's body (only with that mod installed).
    pub female: bool,
    /// Face 0..4 (the new-game portraits).
    pub skin: u8,
    /// Augmentique outfit id ("default" = JC's own).
    pub outfit: String,
    /// Weapon class -> chosen skin id ("default" = none).
    pub weapon_skins: Vec<(String, String)>,
    /// Weapon class -> kills with it (in deathmatch).
    pub kills: Vec<(String, u32)>,
    pub total_kills: u32,
    pub deaths: u32,
    pub matches: u32,
    pub wins: u32,
    pub created: u64,
    pub saved: u64,
    // ---- Operative profile v1 (Modern multiplayer, MULTIPLAYER roadmap Phase 0): what carries between matches.
    /// Credits to spend, and all ever earned.
    pub credits: u32,
    pub credits_total: u32,
    /// Teammates healed.
    pub heals: u32,
    /// Bought unlocks by id ("perk.sprinter", "mod.pistol.scope" ...; the store comes with Phase 2).
    pub unlocks: Vec<String>,
    /// Loadouts, and the one in use.
    pub loadouts: Vec<Loadout>,
    pub loadout: usize,
    /// Rivals taken down with non-lethal weapons.
    pub takedowns: u32,
    /// Cosmetics (cards.rs): the name-tag colour and calling card chosen, and the earned ones already announced.
    pub tag_color: String,
    pub card: String,
    pub cosmetics_seen: Vec<String>,
}

/// A loadout: what a Modern match starts with (MULTIPLAYER Phase 2: weapons, items, skill levels above Trained,
/// augmentations, mods), on the points budget (imsim_core::loadout).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Loadout {
    pub name: String,
    pub build: imsim_core::loadout::Build,
    // (Phase 0's first draft; not used.)
    pub primary: String,
    pub secondary: String,
    pub items: Vec<String>,
}

/// What earns credits (data/deathmatch.ron `profile`).
#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct ProfileTuning {
    pub per_kill: u32,
    /// A rival downed with a non-lethal weapon (prod, baton, pepper spray, gas).
    pub per_takedown: u32,
    /// Damage on a rival someone else then killed (within 10 s).
    pub per_assist: u32,
    /// A camera, turret or security computer hacked.
    pub per_hack: u32,
    pub per_heal: u32,
    pub per_objective: u32,
    pub per_match: u32,
    pub per_win: u32,
    /// Kills with a weapon for each mastery level.
    pub mastery: Vec<u32>,
    pub max_loadouts: usize,
}

impl Default for ProfileTuning {
    fn default() -> Self {
        ProfileTuning {
            per_kill: 10,
            per_takedown: 15,
            per_assist: 5,
            per_hack: 8,
            per_heal: 5,
            per_objective: 20,
            per_match: 25,
            per_win: 75,
            mastery: vec![10, 25, 50, 100, 200, 350, 500],
            max_loadouts: 5,
        }
    }
}

fn tuning() -> &'static ProfileTuning {
    &crate::deathmatch::data().profile
}

/// Weapon mastery: (level, kills for the next level or None at the top) from the kills with it.
pub fn mastery(c: &DmChar, weapon: &str) -> (usize, Option<u32>) {
    let kills = c.kills.iter().find(|k| k.0 == weapon).map_or(0, |k| k.1);
    let steps = &tuning().mastery;
    let level = steps.iter().filter(|&&n| kills >= n).count();
    (level, steps.get(level).copied())
}

/// Credits earned now (a Modern match only: Classic is the original's, nothing carries).
fn earn(c: &mut DmChar, n: u32) {
    c.credits = c.credits.saturating_add(n);
    c.credits_total = c.credits_total.saturating_add(n);
}

/// Spend credits on an unlock; Err when it's already had or too dear.
pub fn buy(c: &mut DmChar, id: &str, cost: u32) -> Result<(), &'static str> {
    if c.unlocks.iter().any(|u| u == id) {
        return Err("Already unlocked.");
    }
    if c.credits < cost {
        return Err("Not enough credits.");
    }
    c.credits -= cost;
    c.unlocks.push(id.to_string());
    Ok(())
}

/// Add or replace a loadout (by name), up to the tuning's limit.
pub fn set_loadout(c: &mut DmChar, l: Loadout) -> Result<usize, &'static str> {
    if let Some(i) = c.loadouts.iter().position(|x| x.name == l.name) {
        c.loadouts[i] = l;
        return Ok(i);
    }
    if c.loadouts.len() >= tuning().max_loadouts {
        return Err("No room for another loadout.");
    }
    c.loadouts.push(l);
    Ok(c.loadouts.len() - 1)
}

/// A teammate healed (Modern co-op of the match): counted, credits.
pub fn note_heal() {
    if crate::deathmatch::classic() {
        return;
    }
    let mut g = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = g.as_mut() {
        c.heals += 1;
        earn(c, tuning().per_heal);
        save(c);
    }
    drop(g);
    ledger_add("Heals", tuning().per_heal);
}

// ---------------------------------------------------------------------------------------------------------------
// The match's credits, by what earned them (shown with the results).

static LEDGER: Mutex<Vec<(&'static str, u32, u32)>> = Mutex::new(Vec::new());

/// A new match: the breakdown starts empty.
pub fn begin_match() {
    LEDGER.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

fn ledger_add(what: &'static str, credits: u32) {
    let mut l = LEDGER.lock().unwrap_or_else(|e| e.into_inner());
    match l.iter_mut().find(|x| x.0 == what) {
        Some(x) => {
            x.1 += 1;
            x.2 += credits;
        }
        None => l.push((what, 1, credits)),
    }
}

/// This match's credits: (what, how many, credits).
pub fn ledger() -> Vec<(&'static str, u32, u32)> {
    LEDGER.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Credits for a thing done (Modern only): the character's purse and the match's breakdown.
fn reward(what: &'static str, credits: u32) {
    if crate::deathmatch::classic() || !crate::deathmatch::playing() {
        return;
    }
    let mut g = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = g.as_mut() {
        earn(c, credits);
        save(c);
    }
    drop(g);
    ledger_add(what, credits);
}

/// A camera, turret or security computer hacked in a Modern match (living.rs).
pub fn note_hack() {
    reward("Hacks", tuning().per_hack);
}

/// A rival downed (downed.rs; the host saw it): a takedown.
pub fn note_takedown() {
    if !crate::deathmatch::classic() {
        let mut g = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = g.as_mut() {
            c.takedowns += 1;
        }
    }
    reward("Takedowns", tuning().per_takedown);
}

/// Damage on a rival someone else killed.
pub fn note_assist() {
    reward("Assists", tuning().per_assist);
}

/// The weapons whose kills are takedowns (non-lethal).
pub fn non_lethal(weapon: &str) -> bool {
    matches!(weapon, "WeaponProd" | "WeaponBaton" | "WeaponPepperGun" | "WeaponGasGrenade")
}

// ---------------------------------------------------------------------------------------------------------------
// Loadouts and unlocks.

/// The credits that unlock an entry (None: open from the start).
pub fn unlock_cost(id: &str) -> Option<u32> {
    crate::deathmatch::data().loadouts.unlock.get(id).copied()
}

/// An entry this character may take: open, bought, or part of the build's class template. Mods ("mod:W:M") need
/// the weapon's mastery level (or come with the class).
pub fn unlocked(c: &DmChar, class: &str, id: &str) -> bool {
    let tmpl = crate::deathmatch::class_template(class).map(|t| &t.build);
    if let Some(rest) = id.strip_prefix("mod:") {
        let (w, m) = rest.split_once(':').unwrap_or((rest, ""));
        if tmpl.is_some_and(|b| b.mods.iter().any(|x| x.0 == w && x.1 == m)) {
            return true;
        }
        let need = crate::deathmatch::data().loadouts.mod_mastery.get(m).copied().unwrap_or(0);
        return mastery(c, w).0 >= need;
    }
    if tmpl.is_some_and(|b| b.weapons.iter().any(|x| x == id) || b.augs.iter().any(|x| x == id) || b.items.iter().any(|x| x.0 == id)) {
        return true;
    }
    unlock_cost(id).is_none() || c.unlocks.iter().any(|u| u == id)
}

/// What's wrong with a build for this character and the room's rules (empty: fine).
pub fn problems(c: &DmChar, b: &imsim_core::loadout::Build) -> Vec<imsim_core::loadout::Problem> {
    let p = &crate::deathmatch::data().loadouts.prices;
    let full = crate::deathmatch::rules().is_some_and(|r| r.full_loadouts);
    p.check(b, if full { None } else { Some(p.budget) }, |id| unlocked(c, &b.class, id))
}

/// The build this player brings into a Modern match: his character's chosen loadout when it passes, else None
/// (the plain start kit).
pub fn active_build() -> Option<imsim_core::loadout::Build> {
    let c = last()?;
    let l = c.loadouts.get(c.loadout)?;
    if l.build.weapons.is_empty() && l.build.items.is_empty() {
        return None;
    }
    let probs = problems(&c, &l.build);
    if !probs.is_empty() {
        println!("LOADOUT {} not used: {probs:?}", l.name);
        return None;
    }
    println!("LOADOUT {} ({}): {:?} {:?} augs {:?} mods {:?}", l.name, l.build.class, l.build.weapons, l.build.items, l.build.augs, l.build.mods);
    Some(l.build.clone())
}

/// The class of the loadout in use (its signature perk), in a Modern match.
pub fn class_perk() -> String {
    if !crate::deathmatch::playing() || crate::deathmatch::classic() {
        return String::new();
    }
    static CACHE: Mutex<(u64, String)> = Mutex::new((u64::MAX, String::new()));
    let gen = crate::loader::generation();
    let mut g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if g.0 != gen {
        *g = (gen, active_build().map(|b| b.class).unwrap_or_default());
    }
    g.1.clone()
}

/// Spend credits to unlock an entry for the current character (and keep it). Err with why not.
pub fn buy_unlock(id: &str) -> Result<(), String> {
    let cost = unlock_cost(id).ok_or("That's open already.")?;
    let mut g = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
    let c = g.as_mut().ok_or("Make a deathmatch character first.")?;
    buy(c, id, cost).map_err(|e| e.to_string())?;
    save(c);
    Ok(())
}

/// Replace the current character (after editing loadouts on the screen) and keep him.
pub fn store(mut c: DmChar) {
    save(&mut c);
    set_current(c);
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn dir() -> PathBuf {
    crate::save::mp_dir().join("Deathmatch")
}

fn path(id: &str) -> PathBuf {
    dir().join(format!("{id}.ron"))
}

/// Every deathmatch character, last played first.
pub fn list() -> Vec<DmChar> {
    let mut v: Vec<DmChar> = std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "ron"))
        .filter_map(|e| ron::from_str::<DmChar>(&std::fs::read_to_string(e.path()).ok()?).ok())
        .collect();
    v.sort_by(|a, b| b.saved.cmp(&a.saved));
    v
}

pub fn load(id: &str) -> Option<DmChar> {
    ron::from_str(&std::fs::read_to_string(path(id)).ok()?).ok()
}

pub fn save(c: &mut DmChar) {
    if c.id.is_empty() {
        c.id = new_id(&c.name);
    }
    c.saved = now();
    // Cosmetics this record just earned are announced once.
    crate::cards::announce_new(c);
    if c.created == 0 {
        c.created = c.saved;
    }
    let _ = std::fs::create_dir_all(dir());
    if let Ok(t) = ron::ser::to_string_pretty(c, ron::ser::PrettyConfig::default()) {
        let _ = std::fs::write(path(&c.id), t);
    }
}

pub fn delete(id: &str) {
    let _ = std::fs::remove_file(path(id));
    let mut g = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
    if g.as_ref().is_some_and(|c| c.id == id) {
        *g = None;
    }
}

fn new_id(name: &str) -> String {
    let base: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).take(16).collect();
    format!("{}-{}", if base.is_empty() { "Player" } else { &base }, now())
}

/// A fresh character (the creator's starting point).
pub fn fresh() -> DmChar {
    DmChar { name: "Agent".into(), outfit: "default".into(), ..Default::default() }
}

static CURRENT: Mutex<Option<DmChar>> = Mutex::new(None);

/// The deathmatch character being played (None: none chosen yet).
pub fn current() -> Option<DmChar> {
    CURRENT.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Play as this character from now on (the last one played is remembered: setting DmCharacter).
pub fn set_current(c: DmChar) {
    if let Ok(mut f) = std::fs::File::create(dir().join("last.txt")) {
        use std::io::Write;
        let _ = write!(f, "{}", c.id);
    }
    *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(c);
}

/// The last character played, if there is one (else the newest).
pub fn last() -> Option<DmChar> {
    if let Some(c) = current() {
        return Some(c);
    }
    let id = std::fs::read_to_string(dir().join("last.txt")).unwrap_or_default();
    let c = load(id.trim()).or_else(|| list().into_iter().next())?;
    *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(c.clone());
    Some(c)
}

/// Deathmatch is possible here: Modern rooms with Augmentique installed (why not, otherwise).
pub fn blocked() -> Option<&'static str> {
    if env_on!("DXVIEW_DM_NO_MOD_CHECK") {
        return None;
    }
    if !unreal_pkg::mods::installed_ids().contains(&"augmentique") {
        return Some("Deathmatch needs the Augmentique mod (outfits and weapon skins): add it on the launcher's Mods page.");
    }
    None
}

/// Lay D Denton's female body is possible.
pub fn female_possible() -> bool {
    crate::travel::lddp()
}

/// The weapon classes a character's skins screen lists: the deathmatch weapons with Augmentique skins.
pub fn skin_weapons() -> Vec<String> {
    let d = crate::deathmatch::data();
    let mut v: Vec<String> = d.tiers.keys().filter(|c| c.starts_with("Weapon") && !crate::augmentique::weapon_skins(c).is_empty()).cloned().collect();
    v.sort();
    v
}

pub fn kills_with(c: &DmChar, class: &str) -> u32 {
    c.kills.iter().find(|k| k.0 == class).map_or(0, |k| k.1)
}

/// A weapon's skins for this character: (id, name, kills still needed - 0 = earned), "default" first.
pub fn skins_for(c: &DmChar, class: &str) -> Vec<(String, String, u32)> {
    let need = &crate::deathmatch::data().skin_unlocks;
    let have = kills_with(c, class);
    let mut v = vec![("default".to_string(), crate::augmentique::weapon_skin_name(class, "default"), 0)];
    for (i, (id, name)) in crate::augmentique::weapon_skins(class).into_iter().enumerate() {
        let n = need.get(i).or(need.last()).copied().unwrap_or(10);
        v.push((id, name, n.saturating_sub(have)));
    }
    v
}

pub fn chosen_skin(c: &DmChar, class: &str) -> String {
    c.weapon_skins.iter().find(|s| s.0 == class).map_or_else(|| "default".into(), |s| s.1.clone())
}

/// The next earned skin of a weapon (dir +1 / -1), for the creator.
pub fn cycle_skin(c: &mut DmChar, class: &str, dir: i32) {
    let earned: Vec<String> = skins_for(c, class).into_iter().filter(|s| s.2 == 0).map(|s| s.0).collect();
    let cur = chosen_skin(c, class);
    let i = earned.iter().position(|x| *x == cur).unwrap_or(0) as i32;
    let next = earned[((i + dir).rem_euclid(earned.len() as i32)) as usize].clone();
    c.weapon_skins.retain(|s| s.0 != class);
    c.weapon_skins.push((class.to_string(), next));
}

/// The outfit's place in Augmentique's list for this character's body (None: the mod's default).
pub fn outfit_index(c: &DmChar) -> Option<usize> {
    crate::augmentique::outfits_for(c.female).into_iter().find(|o| o.1 == c.outfit).map(|o| o.0)
}

/// The next outfit for the character's body.
pub fn cycle_outfit(c: &mut DmChar, dir: i32) {
    let all = crate::augmentique::outfits_for(c.female);
    if all.is_empty() {
        return;
    }
    let i = all.iter().position(|o| o.1 == c.outfit).unwrap_or(0) as i32;
    c.outfit = all[((i + dir).rem_euclid(all.len() as i32)) as usize].1.clone();
}

pub fn outfit_name(c: &DmChar) -> String {
    crate::augmentique::outfits_for(c.female).into_iter().find(|o| o.1 == c.outfit).map_or_else(|| "Default".into(), |o| o.2)
}

/// Become this character for the match: name, body and face for the map's build (travel session), the outfit and
/// weapon skins on (Augmentique's state).
pub fn apply(c: &DmChar) {
    let female = c.female && female_possible();
    let mut ng = crate::travel::session().unwrap_or_default();
    ng.name = c.name.clone();
    ng.skin = c.skin.min(4);
    ng.female = female;
    crate::travel::set_session(ng);
    crate::travel::set_look(c.skin.min(4), female, false, false);
    crate::augmentique::wear(outfit_index(c), c.weapon_skins.clone());
}

/// The session's character, as a new game for the loader (deathmatch maps start fresh every time).
pub fn new_game(c: &DmChar) -> crate::travel::NewGame {
    crate::travel::NewGame {
        name: c.name.clone(),
        skin: c.skin.min(4),
        female: c.female && female_possible(),
        difficulty: 1.5,
        char_id: String::new(),
        ..Default::default()
    }
}

/// A kill in deathmatch with `weapon`: counted, and a skin it earns is announced. Returns the notice.
pub fn credit_kill(weapon: &str) -> Option<String> {
    if crate::deathmatch::classic() {
        return None;
    }
    let mut g = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
    let c = g.as_mut()?;
    c.total_kills += 1;
    if non_lethal(weapon) {
        c.takedowns += 1;
    }
    let (what, n) = if non_lethal(weapon) { ("Takedowns", tuning().per_takedown) } else { ("Kills", tuning().per_kill) };
    earn(c, n);
    ledger_add(what, n);
    if weapon.is_empty() {
        save(c);
        return None;
    }
    let before: Vec<String> = skins_for(c, weapon).into_iter().filter(|s| s.2 == 0).map(|s| s.0).collect();
    match c.kills.iter_mut().find(|k| k.0 == weapon) {
        Some(k) => k.1 += 1,
        None => c.kills.push((weapon.to_string(), 1)),
    }
    let fresh: Vec<(String, String, u32)> = skins_for(c, weapon).into_iter().filter(|s| s.2 == 0 && !before.contains(&s.0)).collect();
    save(c);
    drop(g);
    let (_, name, _) = fresh.into_iter().next()?;
    let wname = weapon_name(weapon);
    let msg = format!("{wname} skin unlocked: {name}");
    crate::classic_hud::client_message(&msg);
    crate::sound::play_player("DeusExSounds.UserInterface.LogGoalAdded", 0.8, 0.0);
    Some(msg)
}

/// The match is over: a death, a match, a win counted.
pub fn note_death() {
    if crate::deathmatch::classic() {
        return;
    }
    let mut g = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = g.as_mut() {
        c.deaths += 1;
        save(c);
    }
}

pub fn note_match(won: bool) {
    if crate::deathmatch::classic() {
        return;
    }
    let mut g = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = g.as_mut() {
        c.matches += 1;
        earn(c, tuning().per_match);
        ledger_add("Match played", tuning().per_match);
        if won {
            c.wins += 1;
            earn(c, tuning().per_win);
            ledger_add("Win", tuning().per_win);
        }
        save(c);
    }
}

/// A class's default properties (cached; the game's System folder).
fn class_defaults(class: &str) -> Vec<unreal_pkg::props::Property> {
    type Cache = Option<(unreal_pkg::classes::ClassDb, std::collections::HashMap<String, Vec<unreal_pkg::props::Property>>)>;
    static DB: Mutex<Cache> = Mutex::new(None);
    let mut g = DB.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        let root = unreal_pkg::paths::root();
        let Some(root) = root else { return Vec::new() };
        *g = Some((unreal_pkg::classes::ClassDb::open(&root.join("System")), Default::default()));
    }
    let (db, cache) = g.as_mut().unwrap();
    cache.entry(class.to_string()).or_insert_with(|| db.defaults(class)).clone()
}

/// A class's name for the screens: ItemName (weapons, items), SkillName, AugmentationName, else the class.
pub fn label(class: &str) -> String {
    let d = class_defaults(class);
    for p in ["ItemName", "SkillName", "AugmentationName"] {
        if let Some(unreal_pkg::props::Value::Str(s)) = unreal_pkg::props::find(&d, p) {
            if !s.is_empty() {
                return s.clone();
            }
        }
    }
    class.trim_start_matches("Weapon").trim_start_matches("Skill").trim_start_matches("Aug").to_string()
}

/// A class's description for the screens (Description / description).
pub fn description(class: &str) -> String {
    let d = class_defaults(class);
    for p in ["Description", "description"] {
        if let Some(unreal_pkg::props::Value::Str(s)) = unreal_pkg::props::find(&d, p) {
            if !s.is_empty() {
                return s.replace("|n", " ").replace("|N", " ");
            }
        }
    }
    String::new()
}

/// The weapon takes this mod (its bCanHave* default, as WeaponMod.CanUpgradeWeapon reads it).
pub fn can_have(weapon: &str, m: &str) -> bool {
    let prop = match m {
        "WeaponModAccuracy" => "bCanHaveModBaseAccuracy",
        "WeaponModClip" => "bCanHaveModReloadCount",
        "WeaponModRange" => "bCanHaveModAccurateRange",
        "WeaponModReload" => "bCanHaveModReloadTime",
        "WeaponModRecoil" => "bCanHaveModRecoilStrength",
        "WeaponModScope" => "bCanHaveScope",
        "WeaponModSilencer" => "bCanHaveSilencer",
        "WeaponModLaser" => "bCanHaveLaser",
        _ => return false,
    };
    matches!(unreal_pkg::props::find(&class_defaults(weapon), prop), Some(unreal_pkg::props::Value::Bool(true)))
}

/// "Pistol" for WeaponPistol (the class's ItemName when it can be read).
pub fn weapon_name(class: &str) -> String {
    static NAMES: Mutex<Option<std::collections::HashMap<String, String>>> = Mutex::new(None);
    let mut g = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(Default::default);
    if let Some(n) = m.get(class) {
        return n.clone();
    }
    let n = unreal_pkg::paths::root()
        .map(|root| {
            let mut db = unreal_pkg::classes::ClassDb::open(&root.join("System"));
            match unreal_pkg::props::find(&db.defaults(class), "ItemName") {
                Some(unreal_pkg::props::Value::Str(s)) if !s.is_empty() => s.clone(),
                _ => class.trim_start_matches("Weapon").to_string(),
            }
        })
        .unwrap_or_else(|| class.trim_start_matches("Weapon").to_string());
    m.insert(class.to_string(), n.clone());
    n
}

#[cfg(test)]
mod tests {
    #[test]
    fn profile_mastery_and_store() {
        let mut c = super::fresh();
        c.kills.push(("WeaponPistol".into(), 26));
        assert_eq!(super::mastery(&c, "WeaponPistol"), (2, Some(50)));
        assert_eq!(super::mastery(&c, "WeaponRifle"), (0, Some(10)));
        super::earn(&mut c, 30);
        assert_eq!(super::buy(&mut c, "perk.sprinter", 50), Err("Not enough credits."));
        assert_eq!(super::buy(&mut c, "perk.sprinter", 20), Ok(()));
        assert_eq!(c.credits, 10);
        assert_eq!(c.credits_total, 30);
        assert!(super::buy(&mut c, "perk.sprinter", 0).is_err());
        let l = super::Loadout { name: "Stealth".into(), primary: "WeaponMiniCrossbow".into(), ..Default::default() };
        assert_eq!(super::set_loadout(&mut c, l.clone()), Ok(0));
        assert_eq!(super::set_loadout(&mut c, l), Ok(0));
    }

    use super::*;

    #[test]
    fn cycle_wraps_over_earned() {
        let mut c = fresh();
        // Without the mod's data there are no skins: only the default, and cycling stays on it.
        cycle_skin(&mut c, "WeaponPistol", 1);
        assert_eq!(chosen_skin(&c, "WeaponPistol"), "default");
    }
}
