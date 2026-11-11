//! Classic deathmatch: the original multiplayer patch's own numbers, read from the player's DeusEx.u.
//!
//! In a net game the original swaps many defaults for multiplayer ones in PreBeginPlay / PostBeginPlay /
//! BeginPlay: `if ( Level.NetMode != NM_StandAlone ) { HitDamage = mpHitDamage; ... }` in every weapon, skill
//! (cost[] / LevelValues[]), augmentation (LevelValues[3] = mpAugValue, EnergyRate = mpEnergyDrain), projectile
//! (blastRadius = mpBlastRadius), the turret (maxRange = mpTurretRange) and Human (GroundSpeed = mpGroundSpeed).
//! This module reads those blocks from each class's script text at run time and turns the plain assignments into
//! an override layer ("classicmp", unreal_pkg::classes) that is live only while a Classic deathmatch is played, so
//! every system that reads class defaults gets the multiplayer values without its own code knowing.
//!
//! Only straight assignments are taken: a number, True / False, another property of the class (`mpHitDamage`,
//! `Default.mpX`, `mpX * 3`). Nested ifs inside the block and anything computed through a call are left out (the
//! skills' start level, the turret's bDisabled = !bActive and the speed aug's anim rate are handled where they
//! matter). A block whose condition also names a class (`&& (Class == Class'Rocket')`) applies to that class only.
//!
//! Also here: DeusExWeapon.PostBeginPlay's bWeaponStay in multiplayer (true unless the class's own PostBeginPlay
//! clears it: LAM, gas / EMP grenades, LAW), the game types' defaults (DeathMatchGame, BasicTeamDMGame,
//! AdvTeamDMGame, TeamDMGame) and the AugPrefs grant order.

use std::collections::HashMap;
use std::sync::Mutex;
use unreal_pkg::classes::ClassDb;
use unreal_pkg::props::{self, Property, Value};

/// The layer's name in unreal_pkg::classes.
pub const LAYER: &str = "classicmp";

type Entry = (String, String, u32, Value);

static BUILT: Mutex<Option<Vec<Entry>>> = Mutex::new(None);
static STAY: Mutex<Option<HashMap<String, bool>>> = Mutex::new(None);
static ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The Classic multiplayer values are in force (a Classic deathmatch room).
pub fn active() -> bool {
    ON.load(std::sync::atomic::Ordering::Relaxed)
}

/// Switch the multiplayer values on (a Classic deathmatch room) or off. Built once from the game's code.
pub fn set_active(on: bool) {
    if active() == on {
        return;
    }
    ON.store(on, std::sync::atomic::Ordering::Relaxed);
    if !on {
        unreal_pkg::classes::set_override_layer(LAYER, Vec::new());
        return;
    }
    let list = {
        let mut g = BUILT.lock().unwrap_or_else(|e| e.into_inner());
        if g.is_none() {
            *g = Some(match open_db() {
                Some(mut db) => build_layer(&mut db),
                None => Vec::new(),
            });
        }
        g.clone().unwrap_or_default()
    };
    println!("CLASSICMP layer on: {} values", list.len());
    unreal_pkg::classes::set_override_layer(LAYER, list);
}

fn open_db() -> Option<ClassDb> {
    let root = unreal_pkg::paths::root()?;
    Some(ClassDb::open(&root.join("System")))
}

/// A class's base defaults, read once a class (the System folder's packages; mods never change these classes'
/// multiplayer values).
fn base(class: &str) -> Vec<props::Property> {
    static CACHE: Mutex<Option<(Option<std::path::PathBuf>, ClassDb, HashMap<String, Vec<props::Property>>)>> = Mutex::new(None);
    let root = unreal_pkg::paths::root();
    let mut g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if g.as_ref().is_none_or(|c| c.0 != root) {
        let Some(db) = open_db() else { return Vec::new() };
        *g = Some((root, db, HashMap::new()));
    }
    let Some((_, db, map)) = g.as_mut() else { return Vec::new() };
    map.entry(class.to_ascii_lowercase()).or_insert_with(|| db.base_defaults(class)).clone()
}

/// The layer for every DeusEx class: (class, property, index, value).
pub fn build_layer(db: &mut ClassDb) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut parsed: HashMap<String, Vec<Block>> = HashMap::new();
    let classes = db.classes_in("DeusEx");
    for class in &classes {
        // Farthest ancestor first: the nearest class's block runs last (Super.PreBeginPlay() comes first).
        let mut chain = db.ancestry(class);
        chain.reverse();
        let mut sets: Vec<(String, u32, Rhs)> = Vec::new();
        for anc in &chain {
            let blocks = match parsed.get(anc) {
                Some(b) => b.clone(),
                None => {
                    let b = db.script_text(anc).map(|t| mp_blocks(&t)).unwrap_or_default();
                    parsed.insert(anc.clone(), b.clone());
                    b
                }
            };
            for b in blocks {
                if b.only.as_ref().is_some_and(|o| !o.eq_ignore_ascii_case(class)) {
                    continue;
                }
                for (lhs, idx, rhs) in b.sets {
                    sets.retain(|(l, i, _)| !(l.eq_ignore_ascii_case(&lhs) && *i == idx));
                    sets.push((lhs, idx, rhs));
                }
            }
        }
        if sets.is_empty() {
            continue;
        }
        let Some(pkg) = db.package_of(class) else { continue };
        let declared = db.declared(&pkg, class);
        let defaults = db.base_defaults(class);
        for (lhs, idx, rhs) in sets {
            let Some(&ty) = declared.get(&lhs.to_ascii_lowercase()) else { continue };
            let Some(v) = rhs.resolve(&defaults, &declared) else { continue };
            let Some(v) = convert(v, ty) else { continue };
            out.push((class.clone(), lhs, idx, v));
        }
    }
    out
}

/// One `if ( Level.NetMode != NM_StandAlone )` block: its assignments, and the one class it is for (if any).
#[derive(Clone, Debug)]
struct Block {
    only: Option<String>,
    sets: Vec<(String, u32, Rhs)>,
}

#[derive(Clone, Debug, PartialEq)]
enum Rhs {
    Num(f64),
    Bool(bool),
    /// Another property (index), times a factor.
    Prop(String, u32, f64),
}

impl Rhs {
    fn resolve(&self, d: &[Property], declared: &HashMap<String, u8>) -> Option<f64> {
        if let Rhs::Prop(name, ..) = self {
            // An enum constant or a local, not a property of the class: left out.
            declared.get(&name.to_ascii_lowercase())?;
        }
        match self {
            Rhs::Num(n) => Some(*n),
            Rhs::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            Rhs::Prop(name, idx, k) => {
                let v = d.iter().find(|p| p.name.eq_ignore_ascii_case(name) && p.index == *idx).map(|p| &p.value);
                let x = match v {
                    Some(Value::Int(i)) => *i as f64,
                    Some(Value::Float(f)) => *f as f64,
                    Some(Value::Byte(b)) => *b as f64,
                    Some(Value::Bool(b)) => {
                        if *b {
                            1.0
                        } else {
                            0.0
                        }
                    }
                    // An unset int / float default is 0 (the property exists but was never given a value).
                    None => 0.0,
                    _ => return None,
                };
                Some(x * k)
            }
        }
    }
}

fn convert(v: f64, ty: u8) -> Option<Value> {
    Some(match ty {
        props::T_INT => Value::Int(v.round() as i32),
        props::T_FLOAT => Value::Float(v as f32),
        props::T_BYTE => Value::Byte(v.round().clamp(0.0, 255.0) as u8),
        props::T_BOOL => Value::Bool(v != 0.0),
        _ => return None,
    })
}

/// Strip `//` and `/* */` comments.
fn strip_comments(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    let mut in_str = false;
    while i < b.len() {
        let c = b[i];
        if in_str {
            out.push(c as char);
            if c == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == b'"' {
            in_str = true;
            out.push('"');
            i += 1;
        } else if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
        } else {
            out.push(c as char);
            i += 1;
        }
    }
    out
}

/// The index of the bracket matching the one at `open` (same kind), or None.
fn matching(b: &[u8], open: usize) -> Option<usize> {
    let (o, c) = match b[open] {
        b'{' => (b'{', b'}'),
        b'(' => (b'(', b')'),
        _ => return None,
    };
    let mut depth = 0i32;
    for (k, &ch) in b.iter().enumerate().skip(open) {
        if ch == o {
            depth += 1;
        } else if ch == c {
            depth -= 1;
            if depth == 0 {
                return Some(k);
            }
        }
    }
    None
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// The multiplayer blocks of the class's PreBeginPlay / PostBeginPlay / BeginPlay.
fn mp_blocks(src: &str) -> Vec<Block> {
    let text = strip_comments(src);
    let lower = text.to_ascii_lowercase();
    let lb = lower.as_bytes();
    let mut out = Vec::new();
    for func in ["prebeginplay", "postbeginplay", "beginplay"] {
        let mut from = 0;
        while let Some(p) = lower[from..].find(func) {
            let at = from + p;
            from = at + func.len();
            // A declaration: "function <name>(" with the name standing alone.
            if at > 0 && is_ident(lb[at - 1]) || lb.get(at + func.len()).is_some_and(|c| is_ident(*c)) {
                continue;
            }
            let before = lower[..at].trim_end();
            if !before.ends_with("function") {
                continue;
            }
            let Some(open) = lower[at..].find('{').map(|k| at + k) else { continue };
            let Some(close) = matching(lb, open) else { continue };
            let body = &text[open + 1..close];
            out.extend(blocks_in(body));
        }
    }
    out
}

/// `if ( ... NetMode != NM_Standalone ... )` blocks in a function body.
fn blocks_in(body: &str) -> Vec<Block> {
    let lower = body.to_ascii_lowercase();
    let b = lower.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(p) = lower[i..].find("if") {
        let at = i + p;
        i = at + 2;
        if (at > 0 && is_ident(b[at - 1])) || b.get(at + 2).is_some_and(|c| is_ident(*c)) {
            continue;
        }
        let Some(cond_open) = lower[at + 2..].find(|c: char| !c.is_whitespace()).map(|k| at + 2 + k) else { continue };
        if b[cond_open] != b'(' {
            continue;
        }
        let Some(cond_close) = matching(b, cond_open) else { continue };
        let cond: String = lower[cond_open..=cond_close].chars().filter(|c| !c.is_whitespace()).collect();
        if !cond.contains("netmode!=nm_standalone") {
            continue;
        }
        // Conditions with an || are not plain "in a net game".
        if cond.contains("||") {
            continue;
        }
        let only = cond.contains("class==class'").then(|| {
            let rest = &body[cond_open..=cond_close];
            let s = rest.to_ascii_lowercase().find("class'").map(|q| q + 6).unwrap_or(0);
            rest[s..].split('\'').next().unwrap_or("").to_string()
        });
        let Some(start) = lower[cond_close + 1..].find(|c: char| !c.is_whitespace()).map(|k| cond_close + 1 + k) else { continue };
        let block = if b[start] == b'{' {
            let Some(end) = matching(b, start) else { continue };
            i = end;
            &body[start + 1..end]
        } else {
            let end = lower[start..].find(';').map(|k| start + k + 1).unwrap_or(lower.len());
            i = end;
            &body[start..end]
        };
        let sets = statements(block).into_iter().filter_map(|s| assignment(&s)).collect::<Vec<_>>();
        if !sets.is_empty() {
            out.push(Block { only, sets });
        }
    }
    out
}

/// The block's own statements (nested blocks and the if / else headers in front of them left out).
fn statements(block: &str) -> Vec<String> {
    let b = block.as_bytes();
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut k = 0;
    while k < b.len() {
        match b[k] {
            b'{' => {
                // A nested block: skip it and whatever led to it (if / else / for).
                k = matching(b, k).map_or(b.len(), |e| e + 1);
                cur.clear();
                continue;
            }
            b';' => {
                let s = cur.trim().to_string();
                cur.clear();
                let l = s.to_ascii_lowercase();
                // `if (x) stmt;` and `else stmt;`: conditional, left out.
                if !(l.starts_with("if") || l.starts_with("else") || l.starts_with("for") || l.starts_with("while")) {
                    out.push(s);
                }
            }
            c => cur.push(c as char),
        }
        k += 1;
    }
    out
}

/// `Name = rhs` / `Name[3] = rhs`.
fn assignment(s: &str) -> Option<(String, u32, Rhs)> {
    let (l, r) = s.split_once('=')?;
    if r.starts_with('=') || l.ends_with(['!', '<', '>']) {
        return None;
    }
    let l = l.trim();
    let (name, idx) = match l.split_once('[') {
        Some((n, rest)) => (n.trim(), rest.trim_end_matches(']').trim().parse::<u32>().ok()?),
        None => (l, 0),
    };
    if name.is_empty() || !name.bytes().all(is_ident) || name.contains('.') {
        return None;
    }
    let r = r.trim();
    if r.contains('(') || r.contains('!') || r.contains('\'') {
        return None;
    }
    let rl = r.to_ascii_lowercase();
    let rhs = if rl == "true" {
        Rhs::Bool(true)
    } else if rl == "false" {
        Rhs::Bool(false)
    } else if let Ok(n) = r.parse::<f64>() {
        Rhs::Num(n)
    } else {
        let (base, k) = match r.split_once('*') {
            Some((a, b)) => (a.trim(), b.trim().parse::<f64>().ok()?),
            None => (r, 1.0),
        };
        let base = base.strip_prefix("Default.").or_else(|| base.strip_prefix("default.")).unwrap_or(base);
        let (pn, pi) = match base.split_once('[') {
            Some((n, rest)) => (n.trim(), rest.trim_end_matches(']').trim().parse::<u32>().ok()?),
            None => (base, 0),
        };
        if pn.is_empty() || !pn.bytes().all(is_ident) || pn.as_bytes()[0].is_ascii_digit() {
            return None;
        }
        Rhs::Prop(pn.to_string(), pi, k)
    };
    Some((name.to_string(), idx, rhs))
}

/// DeusExWeapon.PostBeginPlay in a net game: bWeaponStay = True, unless the class (or one between it and
/// DeusExWeapon) sets it back to False after (LAM, the gas and EMP grenades, the LAW).
pub fn weapon_stays(class: &str) -> bool {
    {
        let g = STAY.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(v) = g.as_ref().and_then(|m| m.get(&class.to_ascii_lowercase())) {
            return *v;
        }
    }
    let stays = match open_db() {
        Some(mut db) => {
            let chain = db.ancestry(class);
            if !chain.iter().any(|c| c == "deusexweapon") {
                false
            } else {
                !chain.iter().take_while(|c| *c != "deusexweapon").any(|c| {
                    db.script_text(c).is_some_and(|t| {
                        let t: String = strip_comments(&t).to_ascii_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
                        t.contains("bweaponstay=false;")
                    })
                })
            }
        }
        None => false,
    };
    STAY.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(class.to_ascii_lowercase(), stays);
    stays
}

/// The original's four game types (menuchoice_gametype: DeathMatchGame, BasicTeamDMGame, AdvTeamDMGame,
/// TeamDMGame = "Custom Team Deathmatch").
pub const GAME_CLASSES: [&str; 4] = ["DeathMatchGame", "BasicTeamDMGame", "AdvTeamDMGame", "TeamDMGame"];

/// A game type's own defaults (DeusExMPGame's globalconfig values as its class sets them):
/// (SkillsAvail, SkillsPerKill, InitialAugs, AugsPerKill, MPSkillStartLevel, bAugsAllowed, bCustomizable,
/// ScoreToWin, fFriendlyFireMult).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GameDefaults {
    pub skills_avail: i32,
    pub skills_per_kill: i32,
    pub initial_augs: i32,
    pub augs_per_kill: i32,
    pub skill_level: i32,
    pub augs_allowed: bool,
    pub customizable: bool,
    pub score_to_win: i32,
    pub friendly_fire: f32,
}

pub fn game_defaults(class: &str) -> GameDefaults {
    let d = base(class);
    let i = |n: &str, dflt: i32| match props::find(&d, n) {
        Some(Value::Int(v)) => *v,
        _ => dflt,
    };
    let b = |n: &str, dflt: bool| match props::find(&d, n) {
        Some(Value::Bool(v)) => *v,
        _ => dflt,
    };
    let f = |n: &str, dflt: f32| match props::find(&d, n) {
        Some(Value::Float(v)) => *v,
        _ => dflt,
    };
    GameDefaults {
        skills_avail: i("SkillsAvail", 2000),
        skills_per_kill: i("SkillsPerKill", 2000),
        initial_augs: i("InitialAugs", 2),
        augs_per_kill: i("AugsPerKill", 2),
        skill_level: i("MPSkillStartLevel", 1),
        augs_allowed: b("bAugsAllowed", true),
        customizable: b("bCustomizable", class == "DeathMatchGame" || class == "TeamDMGame"),
        score_to_win: i("ScoreToWin", 20),
        friendly_fire: f("fFriendlyFireMult", 0.5),
    }
}

/// DeusExPlayer.AugPrefs: the order augmentations are granted in (the player class's defaults).
pub fn aug_prefs() -> Vec<String> {
    static P: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        let d = base("JCDentonMale");
        let mut v: Vec<(u32, String)> = d
            .iter()
            .filter(|p| p.name == "AugPrefs")
            .filter_map(|p| match &p.value {
                Value::Name(n) if !n.is_empty() && n != "None" => Some((p.index, n.clone())),
                _ => None,
            })
            .collect();
        v.sort_by_key(|x| x.0);
        if v.is_empty() {
            return ["AugVision", "AugHealing", "AugSpeed", "AugDefense", "AugBallistic", "AugShield", "AugEMP", "AugStealth", "AugAqualung"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        }
        v.into_iter().map(|x| x.1).collect()
    })
    .clone()
}

/// A property of a class with the Classic values in force (or its plain default without them).
pub fn class_value(class: &str, name: &str) -> Option<Value> {
    unreal_pkg::classes::override_value(LAYER, class, name).filter(|_| active()).or_else(|| {
        let d = base(class);
        props::find(&d, name).cloned()
    })
}

/// A class's base defaults (the System folder's DeusEx.u and friends).
pub fn class_props(class: &str) -> Vec<props::Property> {
    base(class)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mp_blocks() {
        let src = r#"
class WeaponX extends DeusExWeapon;
simulated function PreBeginPlay()
{
	Super.PreBeginPlay();
	// If this is a netgame, then override defaults
	if ( Level.NetMode != NM_StandAlone )
	{
		HitDamage = mpHitDamage;
		BaseAccuracy = mpBaseAccuracy;
		cost[2] = mpCost3;
		LevelValues[3] = mpAugValue;
		recoilStrength = 0.75;
		if ( bSomething )
		{
			PickupAmmoCount = 9;
		}
		PickupAmmoCount = PickupAmmoCount * 3;
		AugmentationLocation = LOC_Torso;
		bDisabled = !bActive;
	}
}
function PostBeginPlay()
{
	if ( ( Level.NetMode != NM_Standalone ) && (Class == Class'Rocket') )
	{
		blastRadius = mpBlastRadius;
		speed = 2000.0000;
	}
}
"#;
        let b = mp_blocks(src);
        assert_eq!(b.len(), 2);
        let names: Vec<&str> = b[0].sets.iter().map(|s| s.0.as_str()).collect();
        assert_eq!(names, ["HitDamage", "BaseAccuracy", "cost", "LevelValues", "recoilStrength", "PickupAmmoCount", "AugmentationLocation"]);
        assert_eq!(b[0].sets[2].1, 2);
        assert_eq!(b[0].sets[4].2, Rhs::Num(0.75));
        assert_eq!(b[0].sets[5].2, Rhs::Prop("PickupAmmoCount".into(), 0, 3.0));
        assert_eq!(b[1].only.as_deref(), Some("Rocket"));
        assert_eq!(b[1].sets[1].2, Rhs::Num(2000.0));
    }

    /// With the game's code (CON_GAME=<root>): the pistol, a skill and an aug get their multiplayer numbers.
    #[test]
    fn layer_from_game() {
        let Ok(root) = std::env::var("CON_GAME") else { return };
        let mut db = ClassDb::open(&std::path::Path::new(&root).join("System"));
        let layer = build_layer(&mut db);
        let get = |c: &str, n: &str, i: u32| layer.iter().find(|e| e.0.eq_ignore_ascii_case(c) && e.1.eq_ignore_ascii_case(n) && e.2 == i).map(|e| e.3.clone());
        let mp = db.base_defaults("WeaponPistol");
        assert_eq!(get("WeaponPistol", "HitDamage", 0), props::find(&mp, "mpHitDamage").cloned());
        assert!(get("SkillWeaponPistol", "cost", 0).is_some());
        assert!(get("AugSpeed", "LevelValues", 3).is_some());
        assert!(get("JCDentonMale", "GroundSpeed", 0).is_some());
        println!("classicmp layer: {} values", layer.len());
    }
}
