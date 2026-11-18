//! The Nameless Mod's achievements (Trestkon.SetAchievement, TNMAchievementManager, AchievementTrigger, the
//! TNM Achievements menu in TNMGUI).
//!
//! The list - nine categories of names, titles, descriptions and the hidden flags - is read at run time from the
//! player's TNM.u (TNMAchievementManager's defaults), never stored here. What's been earned is kept for every game
//! (the mod keeps it as global config): TNMSave/achievements.txt in the data folder. A new one is announced with
//! Trestkon's strAchievement and the title in the message log, when notifications are on (the menu's checkbox;
//! setting TnmAchNotify, default on).

use std::collections::HashMap;
use bevy::prelude::*;
use std::sync::Mutex;
use unreal_pkg::props::{Property, Value};

/// One achievement as the mod lists it.
#[derive(Clone, Debug, Default)]
pub struct Ach {
    /// TNMAchievementManager category (1..9).
    pub cat: usize,
    pub id: String,
    pub title: String,
    pub desc: String,
    pub secret: bool,
}

struct Store {
    list: Vec<Ach>,
    earned: HashMap<String, u64>,
    loaded: bool,
    /// Trestkon.strAchievement (the log line's start).
    prefix: String,
}

static STORE: Mutex<Store> = Mutex::new(Store { list: Vec::new(), earned: HashMap::new(), loaded: false, prefix: String::new() });

fn path() -> std::path::PathBuf {
    unreal_pkg::paths::data_dir().join("TNMSave").join("achievements.txt")
}

fn indexed<'a>(d: &'a [Property], name: &str) -> Vec<(u32, &'a Value)> {
    let mut v: Vec<(u32, &Value)> = d.iter().filter(|p| p.name.eq_ignore_ascii_case(name)).map(|p| (p.index, &p.value)).collect();
    v.sort_by_key(|x| x.0);
    v
}

/// Reads the list from the mod's class defaults (once per run) and the earned file.
pub fn load(classes: &mut unreal_pkg::classes::ClassDb) {
    let mut s = STORE.lock().unwrap_or_else(|e| e.into_inner());
    if s.loaded {
        return;
    }
    s.loaded = true;
    let d = classes.defaults("TNMAchievementManager");
    let str_at = |name: &str, i: u32| -> String {
        indexed(&d, name)
            .into_iter()
            .find(|(k, _)| *k == i)
            .and_then(|(_, v)| match v {
                Value::Str(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_default()
    };
    for cat in 1..=9usize {
        for (i, v) in indexed(&d, &format!("varAchievementsCAT{cat}")) {
            let Value::Str(id) = v else { continue };
            if id.is_empty() {
                continue;
            }
            let secret = indexed(&d, &format!("secretAchievementsCAT{cat}"))
                .into_iter()
                .any(|(k, v)| k == i && matches!(v, Value::Byte(b) if *b != 0));
            s.list.push(Ach {
                cat,
                id: id.clone(),
                title: str_at(&format!("strAchievementsTitlesCAT{cat}"), i),
                desc: str_at(&format!("strAchievementsDescCAT{cat}"), i),
                secret,
            });
        }
    }
    let t = classes.defaults(crate::tnmmode::player_class());
    s.prefix = match unreal_pkg::props::find(&t, "strAchievement") {
        Some(Value::Str(v)) => v.clone(),
        _ => String::new(),
    };
    if let Ok(text) = std::fs::read_to_string(path()) {
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                s.earned.insert(k.trim().to_string(), v.trim().parse().unwrap_or(0));
            }
        }
    }
    println!("TNM achievements: {} listed, {} earned", s.list.len(), s.earned.len());
}

fn save(s: &Store) {
    let mut out = String::new();
    let mut keys: Vec<_> = s.earned.iter().collect();
    keys.sort();
    for (k, v) in keys {
        out.push_str(&format!("{k}={v}\n"));
    }
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, out);
}

/// Trestkon.SetAchievement: earned for good; announced the first time (with notifications on).
pub fn unlock(id: &str) {
    if !crate::tnmmode::active() {
        return;
    }
    let mut s = STORE.lock().unwrap_or_else(|e| e.into_inner());
    if s.earned.keys().any(|k| k.eq_ignore_ascii_case(id)) {
        return;
    }
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    s.earned.insert(id.to_string(), t);
    save(&s);
    println!("TNM ACHIEVEMENT {id}");
    if crate::settings::get("TnmAchNotify") > 0.5 || !crate::settings::has("TnmAchNotify") {
        if let Some(a) = s.list.iter().find(|a| a.id.eq_ignore_ascii_case(id)) {
            let line = if s.prefix.is_empty() { a.title.clone() } else { format!("{} {}", s.prefix, a.title) };
            drop(s);
            crate::classic_hud::client_message(&line);
        }
    }
}

pub fn has(id: &str) -> bool {
    STORE.lock().map(|s| s.earned.keys().any(|k| k.eq_ignore_ascii_case(id))).unwrap_or(false)
}

/// The list (for the menu) and whether each is earned.
pub fn list() -> Vec<(Ach, bool)> {
    let s = STORE.lock().unwrap_or_else(|e| e.into_inner());
    s.list.iter().map(|a| (a.clone(), s.earned.keys().any(|k| k.eq_ignore_ascii_case(&a.id)))).collect()
}

/// The menu's Restore Defaults: nothing earned.
pub fn reset_all() {
    let mut s = STORE.lock().unwrap_or_else(|e| e.into_inner());
    s.earned.clear();
    save(&s);
}

/// The player's weapon in hand is (a subclass of) one of these.
fn in_hand_is(classes: &[&str]) -> bool {
    crate::perks::in_hand().is_some_and(|(c, _)| classes.iter().any(|b| crate::classtree::is_a(&c, b)))
}

/// What a pawn's death earns (TNMPawn.Died, the boss and creature classes' Died and SpawnCarcass): `by_player` = the
/// killer is the player, `stunned` = knocked out (TNMPawn sets bStunned from the damage type before Died),
/// `firewall` = the pawn's bFirewall, `gibbed` = no carcass was made (SpawnCarcass isn't called).
#[allow(clippy::too_many_arguments)]
pub fn pawn_died(class: &str, by_player: bool, stunned: bool, damage_type: &str, mission: i32, firewall: bool, gibbed: bool, flags: &imsim_core::flags::FlagBase) {
    if !crate::tnmmode::active() {
        return;
    }
    let is = |b: &str| crate::classtree::is_a(class, b);
    if firewall && damage_type.eq_ignore_ascii_case("EMP") {
        if by_player {
            unlock("achFirewall");
        }
        return;
    }
    if by_player {
        if in_hand_is(&["WeaponDaiKatana"]) {
            unlock("achDaikatana");
        } else if in_hand_is(&["WeaponHammer", "WeaponUniqueNVS", "WeaponScrewDriver", "WeaponWrench"]) {
            unlock("achToolKill");
        } else if in_hand_is(&["WeaponFists"]) {
            unlock("achFists");
        }
        let unique = [
            ("WeaponUniqueNVS", &["NVSHacker"][..]),
            ("WeaponUniqueKylie", &["Kylie"][..]),
            ("WeaponUniqueZP", &["ZeroPresence", "ZeroPresenceNormal"][..]),
            ("WeaponUniqueAO", &["AlphaOperator"][..]),
            ("WeaponKatanaSlicer", &["Slicer"][..]),
        ];
        if unique.iter().any(|(w, who)| in_hand_is(&[w]) && who.iter().any(|c| is(c))) {
            unlock("achUniqueKill");
        }
        // The classes' own Died.
        let own = [
            ("MelkyGreasel", "achGreasel"),
            ("RedGreasel", "achGreasel"),
            ("dd", "achDDKill"),
            ("Abomnation", "achAbomination"),
            ("DXIAvatar", "achAvatar"),
            ("Ryan", "achRyanKill"),
            ("DarkTemplar", "achDarkTemplar"),
            ("Beeblequix", "achBeebleKill"),
        ];
        for (c, a) in own {
            if is(c) {
                unlock(a);
            }
        }
        if (is("GenericLlama") && in_hand_is(&["WeaponFoon"])) || (is("GenericGoat") && in_hand_is(&["WeaponGoldenSpork", "WeaponSpork"])) {
            unlock("achCultWeapon");
        }
    }
    if gibbed {
        return;
    }
    // SpawnCarcass, whoever the killer.
    if is("Beefman") && !stunned {
        unlock("achBeefman");
    }
    let boss = is("ZeroPresence") || is("Kylie") || is("Slicer");
    if boss && mission == 20 {
        let side = if is("Slicer") { "Allied_PDX" } else { "Allied_WC" };
        if !flags.get_bool(side) && !stunned {
            unlock("achBossEarlyKill");
        }
    }
    let per_mission = [("ZeroPresence", "achZPDXIKnockout", "achZPDXIKill", "achZPABIKill"), ("Slicer", "achSlicerDXIKnockOut", "achSlicerDXIKill", "achSlicerABIKill")];
    for (c, ko, kill, abi) in per_mission {
        if is(c) {
            if mission == 21 {
                unlock(if stunned { ko } else { kill });
            } else if mission == 23 {
                unlock(abi);
            }
        }
    }
}

/// A projectile of the player's killed a pawn (TNMProjectile: a Foon dart on a llama, a spork on a goat).
pub fn projectile_kill(proj_class: &str, pawn_class: &str) {
    let foon = proj_class.eq_ignore_ascii_case("FoonProj") && crate::classtree::is_a(pawn_class, "GenericLlama");
    let spork = proj_class.eq_ignore_ascii_case("SporkProj") && crate::classtree::is_a(pawn_class, "GenericGoat");
    if foon || spork {
        unlock("achCultWeapon");
    }
}

static FIRESTAFF: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The fire staff was frobbed with no room for it.
pub fn firestaff_no_room() {
    if crate::tnmmode::active() {
        FIRESTAFF.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Bool flags set true from places without the flag base (item use), applied next frame.
static FLAG_SETS: Mutex<Vec<(&'static str, i32)>> = Mutex::new(Vec::new());

pub fn set_flag(name: &'static str, expiration: i32) {
    if let Ok(mut q) = FLAG_SETS.lock() {
        q.push((name, expiration));
    }
}

/// Weapons whose GiveTo earns something the first time the player is given one.
const GIVE: [(&str, &str); 14] = [
    ("WeaponDoubleGuns", "achPistols"),
    ("WeaponPlasmaNade", "achVortex"),
    ("WeaponMoonlightBlade", "achMoonlightBlade"),
    ("WeaponKatanaSlicer", "achKatana"),
    ("WeaponKatanaRonin", "achKatana"),
    ("WeaponDaiKatana", "achKatana"),
    ("WeaponKatana", "achKatana"),
    ("WeaponSporkGolden", "achSpork"),
    ("WeaponTemporal", "achTemporal"),
    ("WeaponBlowPipe", "achBlowpipe"),
    ("WeaponGibinator", "achGibinator"),
    ("WeaponFireBlade", "achFireblade"),
    ("WeaponNapalmLauncher", "achNapalm"),
    ("WeaponFireStaff", "achFirestaff"),
];

/// GiveTo of the weapons above (checked as items arrive: a new item of a class is a GiveTo).
pub fn inventory_system(inv: Res<crate::pickups::PlayerInventory>, mut flags: ResMut<crate::ai::GameFlags>, mut seen: Local<usize>) {
    for (name, exp) in FLAG_SETS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        flags.0.set(name, imsim_core::flags::FlagValue::Bool(true), exp);
    }
    // Trestkon.FrobTarget: no room for the fire staff before it has been decrypted.
    if FIRESTAFF.swap(false, std::sync::atomic::Ordering::Relaxed) && !flags.0.get_bool("Firestaff_Decrypted") {
        unlock("achFirestaffSpace");
    }
    if !crate::tnmmode::active() || !inv.is_changed() {
        return;
    }
    let n = inv.0.items.len();
    if n == *seen {
        return;
    }
    *seen = n;
    for it in &inv.0.items {
        let class = it.def.class.as_str();
        for (c, a) in GIVE {
            if crate::classtree::is_a(class, c) {
                unlock(a);
            }
        }
    }
}
