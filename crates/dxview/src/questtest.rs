//! The quest audit: a whole-game check of what the player's install asks of the game against what our code does,
//! so a mission can't quietly be unwinnable. Static - nothing is played - so it runs in seconds:
//!
//! 1. Mission scripts: every name / string literal in the original MissionScript subclasses (flags, tags, pawn
//!    bind names, goals, map names) that our port (missions.rs, mission.rs, ...) never mentions.
//! 2. Flags any original class sets (`flags.SetBool('X', ...)`) that nothing of ours sets or reads.
//! 3. Flags that conversations, triggers or scripts wait for and that nothing in the game sets (dead content in
//!    the original, or a name we'd have to supply).
//! 4. Map travel: every MapExit / Teleporter / script destination exists (and its #tag lands somewhere), and every
//!    mission map can be reached from Liberty Island.
//! 5. Events each map sends that nothing receives, and conversation Trigger events with no receiver in their
//!    mission's maps.
//! 6. Placed classes with their own script (functions / states) that our code never names - behaviour that may be
//!    missing.
//! 7. Locked doors with no way through: no key anywhere in the mission, not pickable, not breakable, no event opens
//!    them.
//! 8. Hazards: pain / kill / current zones (handled by zones.rs).
//! 9. Conversations whose owner isn't in any map of their mission.
//!
//! Run with the game folder:
//!   QUEST_GAME=<Deus Ex folder> cargo test -p dxview quest_audit -- --nocapture
//! The report goes to stdout and to QUEST_OUT (default quest_report.md in the system temp folder). It fails only on
//! problems on our side (1, 2 and broken travel); the rest is information about the original's content.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use unreal_pkg::props::{find, Property, Value};
use unreal_pkg::Package;

/// Our sources, lower case, with their file names (crates/dxview, dx-level, imsim-core).
fn our_sources() -> Vec<(String, String)> {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    for d in [base.join("src"), base.join("../dx-level/src"), base.join("../imsim-core/src")] {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "rs") && !p.ends_with("questtest.rs") {
                if let Ok(s) = std::fs::read_to_string(&p) {
                    out.push((p.file_name().unwrap().to_string_lossy().to_string(), s.to_ascii_lowercase()));
                }
            }
        }
    }
    out
}

/// UnrealScript without comments and without its defaultproperties.
fn code_of(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let b = src.as_bytes();
    let mut i = 0;
    let mut in_str: Option<u8> = None;
    while i < b.len() {
        let c = b[i];
        if let Some(q) = in_str {
            out.push(c as char);
            if c == b'\\' && i + 1 < b.len() {
                out.push(b[i + 1] as char);
                i += 2;
                continue;
            }
            if c == q || c == b'\n' {
                in_str = None;
            }
            i += 1;
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        if c == b'"' || c == b'\'' {
            in_str = Some(c);
        }
        out.push(c as char);
        i += 1;
    }
    match out.to_ascii_lowercase().find("defaultproperties") {
        Some(k) => out[..k].to_string(),
        None => out,
    }
}

/// Name ('X') and string ("X") literals, with the text just before each (to tell class'X' and SetBool('X' apart).
fn literals(code: &str) -> Vec<(String, String, bool)> {
    let b = code.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'"' || c == b'\'' {
            let start = i + 1;
            let mut j = start;
            while j < b.len() && b[j] != c && b[j] != b'\n' {
                if b[j] == b'\\' {
                    j += 1;
                }
                j += 1;
            }
            if j < b.len() && b[j] == c {
                let lit = &code[start..j];
                let before = code[..i].trim_end();
                let ctx = before[before.len().saturating_sub(24)..].to_string();
                out.push((lit.to_string(), ctx, c == b'\''));
            }
            i = j + 1;
            continue;
        }
        i += 1;
    }
    out
}

#[derive(Default)]
struct MapInfo {
    mission: i32,
    script: String,
    tags: HashSet<String>,
    /// (class, actor name, event)
    sends: Vec<(String, String, String)>,
    classes: BTreeMap<String, usize>,
    /// (class, actor name, tag) of every actor with a Tag.
    tagged: Vec<(String, String, String)>,
    /// (actor, class, destination as written)
    exits: Vec<(String, String, String)>,
    /// (mover name, KeyIDNeeded, pickable, breakable, tag, frobbable)
    locked: Vec<(String, String, bool, bool, String, bool)>,
    keys: HashSet<String>,
    binds: HashSet<String>,
    flag_sets: HashSet<String>,
    flag_checks: HashSet<String>,
    /// (zone actor, what)
    hazards: Vec<(String, String)>,
}

fn name_of(ps: &[Property], k: &str) -> String {
    match find(ps, k) {
        Some(Value::Name(n)) | Some(Value::Str(n)) if !n.eq_ignore_ascii_case("None") => n.clone(),
        _ => String::new(),
    }
}

fn names_of(ps: &[Property], k: &str) -> Vec<String> {
    ps.iter()
        .filter(|p| p.name.eq_ignore_ascii_case(k))
        .filter_map(|p| match &p.value {
            Value::Name(n) | Value::Str(n) if !n.eq_ignore_ascii_case("None") && !n.is_empty() => Some(n.clone()),
            _ => None,
        })
        .collect()
}

fn bool_of(ps: &[Property], k: &str) -> Option<bool> {
    match find(ps, k) {
        Some(Value::Bool(b)) => Some(*b),
        _ => None,
    }
}

const SKIP: &[&str] = &["Model", "Polys", "Texture", "Level", "Brush", "TextBuffer", "Palette", "ReachSpec", "Sound", "Music", "LevelSummary", "Camera"];

fn scan_map(path: &Path, db: &mut unreal_pkg::classes::ClassDb, anc: &mut HashMap<String, Vec<String>>) -> Option<MapInfo> {
    let pkg = Package::open(path).ok()?;
    let mut m = MapInfo { mission: -1, ..Default::default() };
    for e in &pkg.exports {
        let cls = pkg.class_name(e.class_index).to_string();
        if e.class_index == 0 || SKIP.contains(&cls.as_str()) {
            continue;
        }
        let a = anc.entry(cls.to_ascii_lowercase()).or_insert_with(|| db.ancestry(&cls)).clone();
        if !a.iter().any(|c| c == "actor") {
            continue;
        }
        let Some(ps) = db.actor_properties(&pkg, e) else { continue };
        *m.classes.entry(cls.clone()).or_default() += 1;
        let is = |c: &str| a.iter().any(|x| x == c);
        let tag = name_of(&ps, "Tag");
        if !tag.is_empty() {
            m.tags.insert(tag.to_ascii_lowercase());
            m.tagged.push((cls.clone(), e.name.clone(), tag.clone()));
        }
        let ev = name_of(&ps, "Event");
        if !ev.is_empty() {
            m.sends.push((cls.clone(), e.name.clone(), ev));
        }
        for k in ["OutEvents", "UnTriggerEvent", "FailEvent", "ZonePlayerEvent", "Events", "AttachTag"] {
            for v in names_of(&ps, k) {
                if k == "AttachTag" {
                    continue;
                }
                m.sends.push((cls.clone(), e.name.clone(), v));
            }
        }
        if is("deusexlevelinfo") {
            // (The training maps don't store it: the default, 0.)
            m.mission = match find(&ps, "missionNumber") {
                Some(Value::Int(n)) => *n,
                _ => 0,
            };
            if let Some(Value::Path(p)) | Some(Value::Name(p)) = find(&ps, "Script") {
                m.script = p.clone();
            }
        }
        let b = name_of(&ps, "BindName");
        if !b.is_empty() {
            m.binds.insert(b.to_ascii_lowercase());
        }
        if is("scriptedpawn") {
            m.binds.insert(cls.to_ascii_lowercase());
        }
        if is("mapexit") {
            m.exits.push((e.name.clone(), cls.clone(), name_of(&ps, "DestMap")));
        } else if is("teleporter") {
            let u = name_of(&ps, "URL");
            if !u.is_empty() {
                m.exits.push((e.name.clone(), cls.clone(), u));
            }
        }
        if is("deusexmover") && bool_of(&ps, "bLocked") == Some(true) {
            m.locked.push((
                e.name.clone(),
                name_of(&ps, "KeyIDNeeded"),
                bool_of(&ps, "bPickable").unwrap_or(true),
                bool_of(&ps, "bBreakable").unwrap_or(false),
                tag.clone(),
                bool_of(&ps, "bFrobbable").unwrap_or(true),
            ));
        }
        if is("nanokey") {
            let k = name_of(&ps, "KeyID");
            if !k.is_empty() {
                m.keys.insert(k.to_ascii_lowercase());
            }
        }
        if is("pickupdistributor") {
            // NanoKeyData structs: their KeyID is a name inside.
            let text = format!("{:?}", ps);
            for w in text.split(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
                if !w.is_empty() {
                    m.keys.insert(w.to_ascii_lowercase());
                }
            }
        }
        if is("flagtrigger") {
            let f = name_of(&ps, "FlagName");
            if !f.is_empty() {
                if bool_of(&ps, "bSetFlag").unwrap_or(true) {
                    m.flag_sets.insert(f.to_ascii_lowercase());
                } else {
                    m.flag_checks.insert(f.to_ascii_lowercase());
                }
            }
            // bTrigger flag triggers check the flag before firing their event.
            if bool_of(&ps, "bTrigger") == Some(true) && !f.is_empty() {
                m.flag_checks.insert(f.to_ascii_lowercase());
            }
        }
        for k in ["checkFlag", "FlagName", "flagName", "RequiredFlag"] {
            if !is("flagtrigger") {
                let f = name_of(&ps, k);
                if !f.is_empty() && is("trigger") {
                    m.flag_checks.insert(f.to_ascii_lowercase());
                }
            }
        }
        if is("zoneinfo") && !is("levelinfo") {
            let mut what = Vec::new();
            if bool_of(&ps, "bPainZone") == Some(true) || matches!(find(&ps, "DamagePerSec"), Some(Value::Int(v)) if *v != 0) {
                what.push(format!(
                    "pain {} {} ({}){}",
                    match find(&ps, "DamagePerSec") {
                        Some(Value::Int(v)) => *v,
                        _ => 0,
                    },
                    name_of(&ps, "DamageType"),
                    if bool_of(&ps, "bPainZone") == Some(true) { "on" } else { "off" },
                    if tag.is_empty() || tag.eq_ignore_ascii_case(&cls) { String::new() } else { format!(", switched by '{tag}'") }
                ));
            }
            if bool_of(&ps, "bKillZone") == Some(true) {
                what.push("kill zone".into());
            }
            if let Some(Value::Vector(v)) = find(&ps, "ZoneVelocity") {
                if v.iter().any(|x| *x != 0.0) {
                    what.push(format!("current {:?}", v));
                }
            }
            for k in ["bNeutralZone", "bDestructive", "bNoInventory"] {
                if bool_of(&ps, k) == Some(true) {
                    what.push(k.to_string());
                }
            }
            if !name_of(&ps, "ZonePlayerEvent").is_empty() {
                what.push(format!("player event {}", name_of(&ps, "ZonePlayerEvent")));
            }
            if !what.is_empty() {
                m.hazards.push((e.name.clone(), what.join(", ")));
            }
        }
    }
    Some(m)
}

/// Does our code mention this literal? (Quoted first; bare words for names built by format!.)
fn ours_has(ours: &[(String, String)], lit: &str) -> bool {
    let l = lit.to_ascii_lowercase();
    let q = format!("\"{l}\"");
    if ours.iter().any(|(_, s)| s.contains(&q)) {
        return true;
    }
    let word = |s: &str| {
        s.match_indices(&l).any(|(i, _)| {
            let pre = s[..i].chars().last().is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_');
            let post = s[i + l.len()..].chars().next().is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_');
            pre && post
        })
    };
    if ours.iter().any(|(_, s)| word(s)) {
        return true;
    }
    // A conversation's _Played flag (or a pawn's _Dead) built from its name.
    for suf in ["_played", "_dead", "_unconscious"] {
        if let Some(base) = l.strip_suffix(suf) {
            if !base.is_empty() && ours_has(ours, base) {
                return true;
            }
        }
    }
    // Names built with format!: "MS_Dish{i}Rotated" covers MS_Dish1Rotated.
    let b = l.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() {
            let mut j = i;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let (pre, post) = (&l[..i], &l[j..]);
            let hit = ours.iter().any(|(_, s)| {
                s.match_indices(&format!("{pre}{{")).any(|(k, _)| {
                    let rest = &s[k + pre.len()..];
                    rest.find('}').is_some_and(|e| e < 16 && rest[e + 1..].starts_with(post))
                })
            });
            if hit {
                return true;
            }
            i = j;
        } else {
            i += 1;
        }
    }
    false
}

/// The functions / events / states a class's own code declares.
fn declared(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    for kw in ["function ", "event ", "state "] {
        for (i, _) in code.match_indices(kw) {
            let rest = &code[i + kw.len()..];
            let head: String = rest.chars().take_while(|c| *c != '(' && *c != '{' && *c != '\n' && *c != ';').collect();
            if let Some(name) = head.split_whitespace().last() {
                out.push(format!("{}{}", if kw == "state " { "state " } else { "" }, name.to_ascii_lowercase()));
            }
        }
    }
    out
}

/// Script that only picks a skin (dx_level::skincolor runs those switches) or only sets multiplayer values.
fn trivial_script(code: &str) -> bool {
    let d = declared(code);
    let c = code.to_ascii_lowercase();
    let skin = c.contains("skin =") || c.contains("skin=") || c.contains("multiskins[");
    d.iter().all(|f| match f.as_str() {
        "beginplay" | "postbeginplay" | "travelpostaccept" | "setskin" => skin,
        "prebeginplay" => c.contains("netmode") || skin,
        _ => false,
    })
}

/// Flags set by script calls: flags.SetBool('X' / SetInt / SetName / SetFloat / SetByte / SetVector / SetRotator.
fn flag_calls(code: &str) -> (Vec<String>, Vec<String>) {
    let (mut sets, mut gets) = (Vec::new(), Vec::new());
    for (lit, ctx, name) in literals(code) {
        if !name {
            continue;
        }
        let c = ctx.to_ascii_lowercase();
        let call = c.rsplit(|ch: char| !ch.is_ascii_alphanumeric() && ch != '(' && ch != '.').next().unwrap_or("");
        if !c.ends_with('(') {
            continue;
        }
        if ["setbool(", "setint(", "setname(", "setfloat(", "setbyte(", "setvector(", "setrotator("].iter().any(|k| call.ends_with(k)) {
            sets.push(lit);
        } else if ["getbool(", "getint(", "getname(", "getfloat(", "getbyte(", "checkflag("].iter().any(|k| call.ends_with(k)) {
            gets.push(lit);
        }
    }
    (sets, gets)
}

fn map_key(s: &str) -> String {
    s.split(['#', '?']).next().unwrap_or("").trim().to_ascii_lowercase().trim_end_matches(".dx").to_string()
}

#[test]
fn quest_audit() {
    let Some(root) = std::env::var_os("QUEST_GAME") else {
        println!("QUEST_GAME not set: the quest audit needs the game folder");
        return;
    };
    let root = PathBuf::from(root);
    let system = root.join("System");
    let mut db = unreal_pkg::classes::ClassDb::open(&system);
    let cons = dx_level::conversations::ConDb::load(&system).expect("DeusExConText.u");
    let ours = our_sources();
    let mut r = String::new();
    let mut ours_missing: Vec<String> = Vec::new();

    // ---- The maps.
    let mut files: Vec<PathBuf> = std::fs::read_dir(root.join("Maps")).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dx"))).collect();
    files.sort();
    let mut anc = HashMap::new();
    let mut maps: BTreeMap<String, MapInfo> = BTreeMap::new();
    let mut map_names: HashMap<String, String> = HashMap::new();
    for p in &files {
        let stem = p.file_stem().unwrap().to_string_lossy().to_string();
        if let Some(m) = scan_map(p, &mut db, &mut anc) {
            map_names.insert(stem.to_ascii_lowercase(), stem.clone());
            maps.insert(stem, m);
        }
    }
    let by_mission = |n: i32| -> Vec<String> { maps.iter().filter(|(_, m)| m.mission == n).map(|(k, _)| k.clone()).collect() };
    writeln!(r, "# Quest audit\n\n{} maps, {} conversations, {} of our source files.\n", maps.len(), cons.cons.len(), ours.len()).ok();

    // ---- The original's scripts.
    let dx = Package::open(system.join("DeusEx.u")).expect("DeusEx.u");
    let texts: Vec<(String, String)> = dx.script_texts().into_iter().map(|t| (t.owner, code_of(&t.text))).collect();
    let mut script_sets: HashMap<String, Vec<String>> = HashMap::new();
    let mut script_gets: HashMap<String, Vec<String>> = HashMap::new();
    for (owner, code) in &texts {
        let (s, g) = flag_calls(code);
        for f in s {
            script_sets.entry(f.to_ascii_lowercase()).or_default().push(owner.clone());
        }
        for f in g {
            script_gets.entry(f.to_ascii_lowercase()).or_default().push(owner.clone());
        }
    }

    // 1. Mission script literals our port never mentions.
    writeln!(r, "## 1. Mission script names and strings our port doesn't mention\n").ok();
    let mut mission_classes: Vec<&(String, String)> = texts.iter().filter(|(o, _)| o.starts_with("Mission")).collect();
    mission_classes.sort_by(|a, b| a.0.cmp(&b.0));
    let generic: HashSet<&str> = ["none", "", " ", "player", "dead", "_dead", "_unconscious", "_played"].into_iter().collect();
    for (owner, code) in mission_classes {
        let mut miss = BTreeSet::new();
        for (lit, ctx, _) in literals(code) {
            let l = lit.trim();
            if generic.contains(l.to_ascii_lowercase().as_str()) || l.len() < 3 || l.contains(' ') && !l.contains('_') {
                continue;
            }
            // Log text and debug strings.
            if ctx.to_ascii_lowercase().ends_with("log(") || ctx.to_ascii_lowercase().ends_with("broadcastmessage(") {
                continue;
            }
            if !ours_has(&ours, l) {
                miss.insert(l.to_string());
            }
        }
        if miss.is_empty() {
            writeln!(r, "- {owner}: all covered").ok();
        } else {
            writeln!(r, "- **{owner}**: {}", miss.iter().cloned().collect::<Vec<_>>().join(", ")).ok();
            ours_missing.extend(miss.iter().map(|m| format!("{owner}: {m}")));
        }
    }

    // 2. Flags any other original class sets that we never mention.
    writeln!(r, "\n## 2. Flags original classes set that our code never mentions\n").ok();
    let mut n2 = 0;
    let mut sets: Vec<_> = script_sets.iter().collect();
    sets.sort();
    for (f, owners) in sets {
        if owners.iter().all(|o| o.starts_with("Mission")) {
            continue;
        }
        if !ours_has(&ours, f) {
            let mut o = owners.clone();
            o.sort();
            o.dedup();
            writeln!(r, "- {f} (set by {})", o.join(", ")).ok();
            ours_missing.push(format!("flag {f} ({})", o.join(", ")));
            n2 += 1;
        }
    }
    if n2 == 0 {
        writeln!(r, "- none").ok();
    }

    // 3. Flags waited for that nothing sets.
    writeln!(r, "\n## 3. Flags something waits for that nothing sets\n").ok();
    let mut set_by: HashSet<String> = script_sets.keys().cloned().collect();
    let mut wait: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for c in &cons.cons {
        for f in &c.flags {
            wait.entry(f.name.to_ascii_lowercase()).or_default().insert(format!("con {}", c.name));
        }
        for e in &c.events {
            use dx_level::conversations::EventKind::*;
            match &e.kind {
                SetFlag(v) => set_by.extend(v.iter().map(|f| f.name.to_ascii_lowercase())),
                CheckFlag { flags, .. } => {
                    for f in flags {
                        wait.entry(f.name.to_ascii_lowercase()).or_default().insert(format!("con {}", c.name));
                    }
                }
                _ => {}
            }
        }
    }
    for (k, m) in &maps {
        set_by.extend(m.flag_sets.iter().cloned());
        for f in &m.flag_checks {
            wait.entry(f.clone()).or_default().insert(format!("map {k}"));
        }
    }
    for (f, owners) in &script_gets {
        for o in owners {
            wait.entry(f.clone()).or_default().insert(format!("script {o}"));
        }
    }
    let auto = |f: &str| {
        f.ends_with("_dead") || f.ends_with("_unconscious") || f.ends_with("_played") || f.ends_with("_takendamage")
    };
    let mut n3 = 0;
    for (f, who) in &wait {
        if set_by.contains(f) || auto(f) {
            continue;
        }
        // The ones we set ourselves (Lay D Denton's, ours).
        let we = ours_has(&ours, f);
        writeln!(r, "- {f}: waited for by {}{}", who.iter().take(4).cloned().collect::<Vec<_>>().join(", "), if we { " (we set or read it)" } else { "" }).ok();
        n3 += 1;
    }
    if n3 == 0 {
        writeln!(r, "- none").ok();
    }
    // _Dead flags waited for whose pawn doesn't exist anywhere.
    let all_binds: HashSet<String> = maps.values().flat_map(|m| m.binds.iter().cloned()).collect();
    let dead_waits: Vec<&String> = wait.keys().filter(|f| f.ends_with("_dead") && !all_binds.contains(f.trim_end_matches("_dead"))).collect();
    if !dead_waits.is_empty() {
        writeln!(r, "\n_Dead flags for pawns no map places (spawned by a script, or dead content): {}", dead_waits.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")).ok();
    }

    // 4. Travel.
    writeln!(r, "\n## 4. Map travel\n").ok();
    let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut broken = Vec::new();
    for (k, m) in &maps {
        for (actor, cls, dest) in &m.exits {
            let d = map_key(dest);
            if d.is_empty() {
                // A MapExit with no DestMap (or one only a script sets) goes nowhere.
                writeln!(r, "- {k}: {cls} {actor} has no destination").ok();
                continue;
            }
            match map_names.get(&d) {
                Some(dn) => {
                    edges.entry(k.clone()).or_default().insert(dn.clone());
                    if let Some(tag) = dest.split('#').nth(1).map(|t| t.split('?').next().unwrap_or("").to_ascii_lowercase()).filter(|t| !t.is_empty()) {
                        if !maps[dn].tags.contains(&tag) {
                            writeln!(r, "- {k}: {cls} {actor} -> {dest}: no '{tag}' in {dn} (the player starts at the PlayerStart)").ok();
                        }
                    }
                }
                None => broken.push(format!("{k}: {cls} {actor} -> {dest} (no such map)")),
            }
        }
    }
    // Script travel: map names in a mission's script go from that mission's maps.
    for (owner, code) in &texts {
        let Some(n) = owner.strip_prefix("Mission").and_then(|n| n.parse::<i32>().ok()).or(if owner == "MissionEndgame" { Some(99) } else { None }) else { continue };
        for (lit, _, _) in literals(code) {
            if let Some(dn) = map_names.get(&map_key(&lit)) {
                for src in by_mission(n) {
                    if src != *dn {
                        edges.entry(src).or_default().insert(dn.clone());
                    }
                }
            }
        }
    }
    for b in &broken {
        writeln!(r, "- **broken**: {b}").ok();
    }
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut stack = vec!["01_NYC_UNATCOIsland".to_string()];
    while let Some(m) = stack.pop() {
        if seen.insert(m.clone()) {
            stack.extend(edges.get(&m).into_iter().flatten().cloned());
        }
    }
    let unreached: Vec<&String> = maps.iter().filter(|(k, m)| !seen.contains(*k) && m.mission >= 1 && m.mission <= 15).map(|(k, _)| k).collect();
    writeln!(r, "\nReachable from Liberty Island: {} maps. Mission maps not reached by exits or scripts: {}", seen.len(), if unreached.is_empty() { "none".to_string() } else { unreached.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ") }).ok();

    // 5. Events nobody receives.
    writeln!(r, "\n## 5. Events nothing receives\n").ok();
    let con_triggers: Vec<(usize, String)> = cons
        .cons
        .iter()
        .enumerate()
        .flat_map(|(i, c)| c.events.iter().filter_map(move |e| match &e.kind {
            dx_level::conversations::EventKind::Trigger { tag } if !tag.is_empty() => Some((i, tag.clone())),
            _ => None,
        }))
        .collect();
    for (k, m) in &maps {
        let mut lost: BTreeSet<String> = BTreeSet::new();
        for (cls, actor, ev) in &m.sends {
            let l = ev.to_ascii_lowercase();
            if !m.tags.contains(&l) && !ours_has(&ours, ev) {
                lost.insert(format!("{ev} ({cls} {actor})"));
            }
        }
        if !lost.is_empty() {
            writeln!(r, "- {k}: {}", lost.into_iter().collect::<Vec<_>>().join(", ")).ok();
        }
    }
    let mut con_lost = BTreeSet::new();
    for (ci, tag) in &con_triggers {
        let c = &cons.cons[*ci];
        // (A conversation can be in several lists: Hong Kong's are listed for missions 6 and 7.)
        let lists: Vec<i32> = cons.missions.iter().filter(|(_, v)| v.contains(ci)).map(|(m, _)| *m).collect();
        let mission = lists.iter().copied().find(|m| !by_mission(*m).is_empty());
        let pool: Vec<String> = if lists.is_empty() { maps.keys().cloned().collect() } else { lists.iter().flat_map(|m| by_mission(*m)).collect() };
        let l = tag.to_ascii_lowercase();
        if !pool.iter().any(|p| maps[p].tags.contains(&l)) && !ours_has(&ours, tag) {
            con_lost.insert(format!("{tag} (con {}, mission {:?})", c.name, mission));
        }
    }
    writeln!(r, "\nConversation Trigger events with no receiver in their mission: {}", if con_lost.is_empty() { "none".into() } else { con_lost.into_iter().collect::<Vec<_>>().join(", ") }).ok();

    // 6. Classes with their own behaviour that we never name.
    writeln!(r, "\n## 6. Placed classes with their own script that our code never names\n").ok();
    let mut placed: BTreeMap<String, (usize, BTreeSet<String>)> = BTreeMap::new();
    for (k, m) in &maps {
        for (c, n) in &m.classes {
            let e = placed.entry(c.clone()).or_default();
            e.0 += n;
            e.1.insert(k.clone());
        }
    }
    let own: HashMap<String, String> = texts.iter().map(|(o, c)| (o.to_ascii_lowercase(), c.to_ascii_lowercase())).collect();
    let has_behaviour = |c: &str| own.get(c).is_some_and(|t| !declared(t).is_empty() && !trivial_script(t));
    let mut rows = Vec::new();
    for (c, (n, ms)) in &placed {
        let lc = c.to_ascii_lowercase();
        if !has_behaviour(&lc) || ours_has(&ours, c) {
            continue;
        }
        let a = anc.get(&lc).cloned().unwrap_or_default();
        let parent = a.iter().skip(1).find(|p| ours_has(&ours, p)).cloned().unwrap_or_default();
        let mut fns = declared(&own[&lc]);
        fns.sort();
        fns.dedup();
        rows.push((*n, format!("- {c} x{n} (handled as {parent}; declares {}) in {}", fns.join(" "), ms.iter().take(3).cloned().collect::<Vec<_>>().join(", "))));
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, s) in &rows {
        writeln!(r, "{s}").ok();
    }
    if rows.is_empty() {
        writeln!(r, "- none").ok();
    }

    // Every placed class, by its family, for reading the rest.
    writeln!(r, "\n### Placed decals and effects\n").ok();
    for (c, (n, ms)) in &placed {
        let a = anc.get(&c.to_ascii_lowercase()).cloned().unwrap_or_default();
        if a.iter().any(|x| x == "decal" || x == "effects") {
            writeln!(r, "- {c} x{n} ({}) in {}", a.get(1).cloned().unwrap_or_default(), ms.iter().take(4).cloned().collect::<Vec<_>>().join(", ")).ok();
        }
    }

    // 6b. Actors something triggers whose class answers Trigger in its own script, and that we never name.
    writeln!(r, "\n## 6b. Triggered actors whose class has its own Trigger and that our code never names\n").ok();
    let mut n6 = BTreeMap::new();
    for (k, m) in &maps {
        let sent: HashSet<String> = m
            .sends
            .iter()
            .map(|s| s.2.to_ascii_lowercase())
            .chain(con_triggers.iter().map(|t| t.1.to_ascii_lowercase()))
            .collect();
        for (cls, actor, tag) in &m.tagged {
            if !sent.contains(&tag.to_ascii_lowercase()) {
                continue;
            }
            let a = anc.get(&cls.to_ascii_lowercase()).cloned().unwrap_or_default();
            // The nearest class whose own code has a Trigger function.
            let Some(owner) = a.iter().find(|c| own.get(*c).is_some_and(|t| declared(t).iter().any(|f| f == "trigger" || f == "untrigger"))) else { continue };
            if ["actor", "trigger", "mover", "deusexmover", "dispatcher", "scriptedpawn", "pawn"].contains(&owner.as_str()) || ours_has(&ours, owner) || ours_has(&ours, cls) {
                continue;
            }
            n6.entry(owner.clone()).or_insert_with(Vec::new).push(format!("{k} {actor} ('{tag}')"));
        }
    }
    for (owner, list) in &n6 {
        writeln!(r, "- {owner}: {}", list.join(", ")).ok();
    }
    if n6.is_empty() {
        writeln!(r, "- none").ok();
    }

    // 7. Locked doors with no way through.
    writeln!(r, "\n## 7. Locked doors with no key, lock to pick, way to break or event to open them\n").ok();
    let con_objects: HashSet<String> = cons
        .cons
        .iter()
        .flat_map(|c| c.events.iter())
        .filter_map(|e| match &e.kind {
            dx_level::conversations::EventKind::TransferObject { object_name, .. } => Some(object_name.to_ascii_lowercase()),
            _ => None,
        })
        .collect();
    let mut n7 = 0;
    for (k, m) in &maps {
        let mission_keys: HashSet<String> = by_mission(m.mission).iter().flat_map(|mm| maps[mm].keys.iter().cloned()).collect();
        let opened: HashSet<String> = m.sends.iter().map(|s| s.2.to_ascii_lowercase()).chain(con_triggers.iter().map(|t| t.1.to_ascii_lowercase())).collect();
        for (name, key, pick, brk, tag, frob) in &m.locked {
            let kl = key.to_ascii_lowercase();
            let has_key = !kl.is_empty() && (mission_keys.contains(&kl) || con_objects.iter().any(|o| o.contains(&kl)) || ours_has(&ours, key));
            let by_event = !tag.is_empty() && opened.contains(&tag.to_ascii_lowercase());
            if !has_key && !pick && !brk && !by_event {
                writeln!(r, "- {k}: {name} (key '{key}', tag '{tag}'{})", if *frob { "" } else { ", not frobbable" }).ok();
                n7 += 1;
            }
        }
    }
    if n7 == 0 {
        writeln!(r, "- none").ok();
    }

    // 8. Hazards.
    writeln!(r, "\n## 8. Zone hazards (zones.rs / swim.rs)\n").ok();
    for (k, m) in &maps {
        for (z, w) in &m.hazards {
            writeln!(r, "- {k} {z}: {w}").ok();
        }
    }

    // 9. Conversations whose owner isn't in their mission.
    writeln!(r, "\n## 9. Conversations whose owner no map of their mission places\n").ok();
    let mut n9 = BTreeSet::new();
    for (mission, list) in &cons.missions {
        let mm = by_mission(*mission);
        // A list for a mission without maps (7, 16) is a copy of another.
        if mm.is_empty() {
            continue;
        }
        let binds: HashSet<String> = mm.iter().flat_map(|m| maps[m].binds.iter().cloned()).collect();
        for &ci in list {
            let c = &cons.cons[ci];
            let o = c.owner_name.to_ascii_lowercase();
            if c.datalink || o.is_empty() || o == "jcdenton" || binds.contains(&o) || crate::util::contains_ci(&c.name, "_Bark") {
                continue;
            }
            n9.insert(format!("{} (owner {}, mission {mission})", c.name, c.owner_name));
        }
    }
    writeln!(r, "{}", if n9.is_empty() { "- none".to_string() } else { n9.iter().map(|s| format!("- {s}")).collect::<Vec<_>>().join("\n") }).ok();

    // Mission scripts each map runs.
    writeln!(r, "\n## Mission script per map\n").ok();
    for (k, m) in &maps {
        writeln!(r, "- {k}: mission {} script {}", m.mission, if m.script.is_empty() { "-" } else { &m.script }).ok();
    }

    let out = std::env::var_os("QUEST_OUT").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("quest_report.md"));
    std::fs::write(&out, &r).ok();
    println!("{r}\n(report written to {})", out.display());
    assert!(broken.is_empty(), "map exits to maps that don't exist: {broken:?}");
    if std::env::var_os("QUEST_STRICT").is_some() {
        assert!(ours_missing.is_empty(), "original script names our port doesn't handle: {ours_missing:?}");
    }
}
