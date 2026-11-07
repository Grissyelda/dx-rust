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
    let mut g = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
    let c = g.as_mut()?;
    c.total_kills += 1;
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
    let mut g = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = g.as_mut() {
        c.deaths += 1;
        save(c);
    }
}

pub fn note_match(won: bool) {
    let mut g = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = g.as_mut() {
        c.matches += 1;
        if won {
            c.wins += 1;
        }
        save(c);
    }
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
    use super::*;

    #[test]
    fn cycle_wraps_over_earned() {
        let mut c = fresh();
        // Without the mod's data there are no skins: only the default, and cycling stays on it.
        cycle_skin(&mut c, "WeaponPistol", 1);
        assert_eq!(chosen_skin(&c, "WeaponPistol"), "default");
    }
}
