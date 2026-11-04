//! The achievement test run: every one of the fifty is checked two ways.
//!
//! 1. The game's data can produce it: the conversations, choices, flags, map events, characters and items each
//!    achievement waits for exist in the player's install (DeusExConText.u and every map), and the flags our own
//!    mission scripts set are set somewhere. A name that is wrong on our side never unlocks; this finds it.
//! 2. The unlock logic fires: the hooks are fed what the game would feed them (flags, arrivals, events, choices,
//!    takedowns, augmentations, weapon mods...) for a throw-away character, and every achievement must end up
//!    unlocked - Majestic Revelations last, by itself.
//!
//! Run with the game folder: ACH_GAME=<Deus Ex folder> cargo test -p dxview achievement_run -- --nocapture
//! (writes a report to stdout; without ACH_GAME only part 2 runs).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use unreal_pkg::props::{read_properties, skip_state_frame, Value, RF_HAS_STACK};
use unreal_pkg::{Package, Reader};

/// What every map holds that the achievements care about.
#[derive(Default)]
struct MapFacts {
    /// Map file (lower case) -> mission number.
    mission: HashMap<String, i32>,
    /// Event and Tag names of every actor, by map.
    events: HashMap<String, HashSet<String>>,
    /// BindName of every pawn anywhere (lower case).
    binds: HashSet<String>,
    /// FlagTrigger FlagName anywhere.
    trigger_flags: HashSet<String>,
    /// Actor classes anywhere (lower case).
    classes: HashSet<String>,
    /// Actor classes by map (as written).
    map_classes: HashMap<String, HashSet<String>>,
    /// Maps (lower case) that mention a class name anywhere in their actors (placed, or carried).
    mentions: HashMap<String, HashSet<String>>,
    /// The pool table: (map, balls, cue balls).
    pool: Vec<(String, usize, usize)>,
}

fn scan_maps(root: &Path) -> MapFacts {
    let mut f = MapFacts::default();
    let Ok(rd) = std::fs::read_dir(root.join("Maps")) else { return f };
    let mut files: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dx"))).collect();
    files.sort();
    for path in files {
        let map = path.file_stem().unwrap().to_string_lossy().to_ascii_lowercase();
        let Ok(pkg) = Package::open(&path) else { continue };
        let (mut balls, mut cues) = (0, 0);
        for e in &pkg.exports {
            let cls = pkg.class_name(e.class_index).to_string();
            if matches!(cls.as_str(), "Model" | "Polys" | "Texture" | "Level" | "Brush" | "TextBuffer" | "Palette" | "ReachSpec") {
                continue;
            }
            let data = pkg.export_data(e);
            let mut r = Reader::new(data, 0);
            if e.flags & RF_HAS_STACK != 0 && skip_state_frame(&mut r).is_err() {
                continue;
            }
            let Ok(props) = read_properties(&pkg, &mut r) else { continue };
            f.classes.insert(cls.to_ascii_lowercase());
            f.map_classes.entry(map.clone()).or_default().insert(cls.clone());
            let name = |n: &str| {
                props.iter().find(|p| p.name.eq_ignore_ascii_case(n)).and_then(|p| match &p.value {
                    Value::Name(s) | Value::Str(s) => Some(s.clone()),
                    _ => None,
                })
            };
            for k in ["Event", "Tag"] {
                if let Some(v) = name(k) {
                    f.events.entry(map.clone()).or_default().insert(v.to_ascii_lowercase());
                }
            }
            if let Some(b) = name("BindName") {
                f.binds.insert(b.to_ascii_lowercase());
            }
            if cls == "FlagTrigger" {
                if let Some(n) = name("FlagName") {
                    f.trigger_flags.insert(n.to_ascii_lowercase());
                }
            }
            if cls.eq_ignore_ascii_case("DeusExLevelInfo") {
                if let Some(p) = props.iter().find(|p| p.name.eq_ignore_ascii_case("missionNumber")) {
                    if let Value::Int(n) = p.value {
                        f.mission.insert(map.clone(), n);
                    }
                }
            }
            if cls.eq_ignore_ascii_case("Poolball") {
                balls += 1;
                if props.iter().any(|p| p.name.eq_ignore_ascii_case("SkinColor") && matches!(p.value, Value::Byte(15))) {
                    cues += 1;
                }
            }
            // Carried items show up as class paths in InitialInventory and the like.
            let text = format!("{cls} {:?}", props.iter().map(|p| &p.value).collect::<Vec<_>>()).to_ascii_lowercase();
            for c in ["weaponassaultgun", "karkian", "greasel"] {
                if text.contains(c) {
                    f.mentions.entry(c.to_string()).or_default().insert(map.clone());
                }
            }
        }
        if balls > 0 {
            f.pool.push((map.clone(), balls, cues));
        }
        f.mission.entry(map).or_insert(-1);
    }
    f
}

/// Flags our own scripts set (missions.rs / mission.rs), as written there.
fn our_flags() -> HashSet<String> {
    let mut out = HashSet::new();
    for src in [include_str!("missions.rs"), include_str!("mission.rs"), include_str!("consys.rs"), include_str!("triggers.rs")] {
        for pat in ["c.set(\"", "set(fl, \"", "c.setd(\"", "fl.set(\""] {
            for (i, _) in src.match_indices(pat) {
                let rest = &src[i + pat.len()..];
                if let Some(end) = rest.find('"') {
                    out.insert(rest[..end].to_ascii_lowercase());
                }
            }
        }
    }
    out
}

/// One requirement of one achievement.
enum Need {
    /// A conversation by name (its _Played flag is the trigger).
    Con(&'static str),
    /// A conversation with a choice whose text contains this.
    ChoiceIn(&'static str, &'static str),
    /// A flag something in the game sets (a conversation, a FlagTrigger, our mission scripts) - or, for
    /// "<Bind>_Dead", a pawn with that BindName.
    Flag(&'static str),
    /// An actor Event / Tag in a map whose name contains the first string.
    Event(&'static str, &'static str),
    /// A pawn's BindName somewhere.
    Bind(&'static str),
    /// A map of this mission.
    Mission(i32),
    /// A map file.
    Map(&'static str),
    /// A class placed somewhere, or carried.
    Class(&'static str),
    /// Mentioned (placed or carried) in a map of this mission.
    InMission(&'static str, i32),
    /// The Underworld's pool table with its cue ball.
    Pool,
    /// Worked out at run time from the player's own state (augmentations, skills, mods): nothing to look up.
    Runtime,
}

fn needs(id: &str) -> Vec<Need> {
    use Need::*;
    match id {
        "majestic" => vec![Runtime],
        "lady" => vec![Flag("StatueMissionComplete")],
        "town" => vec![Mission(3)],
        "lebedev" => vec![Mission(4)],
        "choices" => vec![Mission(5)],
        "breakout" => vec![Mission(6)],
        "hongkong" => vec![Mission(8)],
        "tanker1" => vec![Mission(9)],
        "tanker2" => vec![Mission(10)],
        "paree" => vec![Mission(11)],
        "templars" => vec![Mission(12)],
        "vandenberg" => vec![Mission(14)],
        "sea" => vec![Mission(15)],
        "area51" | "darkage" => vec![Con("Endgame1")],
        "helios" => vec![Con("Endgame2")],
        "illuminati" => vec![Con("Endgame3")],
        "biomachina" | "slugger" | "sneak" | "section9" | "automoto" | "wardenclyffe" | "atomheart" | "bestoffense" | "specialeyes"
        | "veteran" | "lockload" | "upgrades" => vec![Runtime],
        "servant" => vec![Bind("SickMan")],
        "dunkin" => vec![Event("", "MadeBasket")],
        "escapa" => vec![Flag("MiguelFollowing"), Map("05_NYC_UNATCOHQ")],
        "sawnothing" => vec![Con("SimonsInterrogating3")],
        "imin" => vec![Event("", "alexcloset")],
        "pinball" => vec![Event("airfieldhelibase", "SecretDoor")],
        "wildlife" => vec![Class("karkian"), Class("greasel")],
        "dibbuk" => vec![Con("TalkAboutRay")],
        "sure" => vec![ChoiceIn("ClubMercedesConvo1", "Sure.")],
        "renton" => vec![Con("M12MeetSandraRenton")],
        "hero" => vec![Con("MeetSandraRenton"), Bind("PaulDenton"), Bind("SandraRenton"), Con("DL_JockDeath"), Mission(15)],
        "tail" => vec![Flag("GreenKnowsAboutDowd"), Flag("MS_ShadyGuyUnhidden")],
        "meanjoe" => vec![Flag("JoeGreene_Dead")],
        "based" => vec![Con("TerroristFollowup2")],
        "nicegun" => vec![InMission("weaponassaultgun", 1)],
        "nerves" => vec![Con("LebedevAnnaDead")],
        "tonjc" => vec![Bind("PaulDenton"), Mission(5)],
        "howdid" => vec![Con("AnnaExitKillphrase")],
        "notmachine" => vec![ChoiceIn("M11MeetGunther", "laputan machine")],
        "shame" => vec![Con("RentonLateComment")],
        "pool" => vec![Pool],
        _ => vec![],
    }
}

/// A class's default properties as text (ClassDb of the install in ACH_GAME).
fn defaults(class: &str) -> String {
    static DB: std::sync::OnceLock<std::sync::Mutex<Option<unreal_pkg::classes::ClassDb>>> = std::sync::OnceLock::new();
    let m = DB.get_or_init(|| {
        std::sync::Mutex::new(std::env::var_os("ACH_GAME").map(|r| unreal_pkg::classes::ClassDb::open(&Path::new(&r).join("System"))))
    });
    let mut g = m.lock().unwrap();
    g.as_mut().map(|db| format!("{:?}", db.defaults(class))).unwrap_or_default()
}

/// Checks one requirement; Err says what's missing.
fn check(n: &Need, db: &dx_level::conversations::ConDb, maps: &MapFacts, ours: &HashSet<String>, con_flags: &HashSet<String>) -> Result<String, String> {
    use Need::*;
    match n {
        Con(c) => db.find(c).map(|i| format!("conversation {c} (owner {})", db.cons[i].owner_name)).ok_or(format!("no conversation {c}")),
        ChoiceIn(c, t) => {
            let i = db.find(c).ok_or(format!("no conversation {c}"))?;
            let hit = db.cons[i].events.iter().any(|e| match &e.kind {
                dx_level::conversations::EventKind::Choice { choices, .. } => choices.iter().any(|ch| crate::util::contains_ci(&ch.text, t)),
                _ => false,
            });
            if hit { Ok(format!("{c} offers \"{t}\"")) } else { Err(format!("{c} has no choice with \"{t}\"")) }
        }
        Flag(f) => {
            let l = f.to_ascii_lowercase();
            if let Some(b) = l.strip_suffix("_dead") {
                return if maps.binds.contains(b) || maps.classes.contains(b) { Ok(format!("pawn {b} exists")) } else { Err(format!("no pawn bound {b}")) };
            }
            if let Some(c) = l.strip_suffix("_played") {
                return if db.find(c).is_some() { Ok(format!("conversation {c}")) } else { Err(format!("no conversation {c}")) };
            }
            let mut by = Vec::new();
            if con_flags.contains(&l) {
                by.push("a conversation");
            }
            if maps.trigger_flags.contains(&l) {
                by.push("a FlagTrigger");
            }
            if ours.contains(&l) {
                by.push("our mission script");
            }
            if by.is_empty() { Err(format!("nothing sets {f}")) } else { Ok(format!("{f} set by {}", by.join(", "))) }
        }
        Event(m, e) => {
            let e = e.to_ascii_lowercase();
            let hit: Vec<&String> = maps.events.iter().filter(|(k, v)| k.contains(m) && v.contains(&e)).map(|(k, _)| k).collect();
            if hit.is_empty() { Err(format!("no actor sends {e}")) } else { Ok(format!("{e} in {}", hit[0])) }
        }
        // (A pawn's BindName defaults to its class's: Paul is class PaulDenton, bound PaulDenton.)
        Bind(b) => {
            let l = b.to_ascii_lowercase();
            if maps.binds.contains(&l) || maps.classes.contains(&l) { Ok(format!("pawn {b}")) } else { Err(format!("no pawn bound {b}")) }
        }
        Mission(m) => {
            let hit: Vec<&String> = maps.mission.iter().filter(|(_, v)| **v == *m).map(|(k, _)| k).collect();
            if hit.is_empty() { Err(format!("no map of mission {m}")) } else { Ok(format!("mission {m}: {} maps", hit.len())) }
        }
        Map(m) => {
            if maps.mission.contains_key(&m.to_ascii_lowercase()) { Ok(format!("map {m}")) } else { Err(format!("no map {m}")) }
        }
        Class(c) => {
            let any = maps.classes.iter().any(|k| k.starts_with(c)) || maps.mentions.contains_key(*c);
            if any { Ok(format!("{c} in the maps")) } else { Err(format!("no {c} anywhere")) }
        }
        InMission(c, m) => {
            let hit = maps.mentions.get(*c).is_some_and(|ms| ms.iter().any(|map| maps.mission.get(map) == Some(m)));
            if hit {
                return Ok(format!("{c} in mission {m}"));
            }
            // Carried by default: a class placed in the mission whose defaults give it one (UNATCO troops).
            let carrier = maps.map_classes.iter().filter(|(map, _)| maps.mission.get(*map) == Some(m)).flat_map(|(_, cl)| cl.iter()).find(|cl| {
                defaults(cl).to_ascii_lowercase().contains(c)
            });
            match carrier {
                Some(cl) => Ok(format!("{c} carried by {cl} in mission {m}")),
                None => Err(format!("no {c} in mission {m}")),
            }
        }
        Pool => {
            let ok: Vec<_> = maps.pool.iter().filter(|p| p.0.contains("nyc_bar") && p.1 >= 3 && p.2 >= 1).collect();
            if ok.is_empty() { Err(format!("no pool table with a cue ball ({:?})", maps.pool)) } else { Ok(format!("{} balls, {} cue in {}", ok[0].1, ok[0].2, ok[0].0)) }
        }
        Runtime => Ok("from the player's own state".into()),
    }
}

/// Feeds every hook what the game would and checks all fifty unlock.
fn run_logic(choice_texts: &HashMap<&'static str, String>) -> Vec<(&'static str, bool)> {
    use crate::achievements::*;
    use imsim_core::flags::{FlagBase, FlagValue};
    let mut f = FlagBase::default();
    for n in [
        "StatueMissionComplete",
        "TerroristFollowup2_Played",
        "SimonsInterrogating3_Played",
        "TalkAboutRay_Played",
        "M12MeetSandraRenton_Played",
        "RentonLateComment_Played",
        "LebedevAnnaDead_Played",
        "AnnaExitKillphrase_Played",
        "GreenKnowsAboutDowd",
        "MS_ShadyGuyUnhidden",
        "JoeGreene_Dead",
        "Endgame1_Played",
        "Endgame2_Played",
        "Endgame3_Played",
        "MiguelFollowing",
        "MeetSandraRenton_Played",
    ] {
        f.set(n, FlagValue::Bool(true), 0);
    }
    check_flags(&f, 1, true);
    for (m, map) in [(2, "02_nyc_batterypark"), (3, "03_nyc_unatcoisland"), (4, "04_nyc_unatcoisland"), (5, "05_nyc_unatcohq"), (6, "06_hongkong_helibase"), (8, "08_nyc_street"), (9, "09_nyc_dockyard"), (10, "10_paris_catacombs"), (11, "11_paris_cathedral"), (12, "12_vandenberg_cmd"), (14, "14_vandenberg_sub"), (15, "15_area51_bunker")] {
        crate::achievements::arrival_for_test(m, map, &f);
    }
    event("02_NYC_Street", "MadeBasket");
    event("06_HongKong_AirfieldHeliBase", "SecretDoor");
    event("03_NYC_UNATCOHQ", "alexcloset");
    choice("ClubMercedesConvo1", choice_texts.get("sure").map_or("Sure.", |s| s.as_str()));
    choice("M11MeetGunther", choice_texts.get("notmachine").map_or("Laputan Machine", |s| s.as_str()));
    takedown_with(true, "SickMan", 1, false, false);
    for b in 2..5 {
        takedown_with(true, "Terrorist", b, false, true);
    }
    ads_detonated(9);
    takedown_with(false, "MJ12Troop", 9, true, false);
    hidden_in_view_with(false, true, false);
    hidden_in_view_with(true, false, true);
    let mut m = crate::weapons::ModState { accuracy: 0.1, clip: 0.1, range: 0.1, reload: -0.1, recoil: -0.1, scope: true, silencer: true, laser: true, ..Default::default() };
    assert!(all_mods_on(&[true; 8], false, false, false, &m), "every mod on");
    m.laser = false;
    assert!(!all_mods_on(&[true; 8], false, false, false, &m), "a missing laser stops Lock and Load");
    m.laser = true;
    if all_mods_on(&[true; 8], false, false, false, &m) {
        unlock("lockload");
    }
    // Skills and augmentations: two masters, one aug per body part, one at its top level, three boosted by the
    // Synthetic Heart, and three switched on while the Power Recirculator runs.
    let mut active = HashSet::new();
    let mut power = 0;
    let aug = |class: &'static str, location: usize, level: usize, boosted: bool, act: bool| AugView { class, location, level, max: 3, boosted, has: true, active: act };
    let base = vec![
        aug("AugCombat", 0, 3, false, false),
        aug("AugVision", 1, 1, true, false),
        aug("AugCloak", 2, 1, true, false),
        aug("AugStealth", 3, 1, true, false),
        aug("AugSpeed", 4, 1, false, false),
        aug("AugPower", 5, 1, false, true),
    ];
    check_abilities(&[3, 3, 0], &base, &mut active, &mut power);
    let mut on = base;
    for k in [1, 2, 3] {
        on[k].active = true;
        check_abilities(&[3, 3, 0], &on, &mut active, &mut power);
    }
    // Watched in play (ai / the watcher): sneaking past three guards, an alert seen through a wall, a body fed to
    // a Karkian, the pool table cleared - their unlock paths.
    for _ in 0..3 {
        add_progress("sneak", 1);
    }
    unlock("specialeyes");
    unlock("wildlife");
    unlock("pool");
    LIST.iter().map(|a| (a.0, state(a.0).0.is_some())).collect()
}

#[test]
fn achievement_run() {
    // A throw-away character in a scratch save folder.
    let dir = std::env::temp_dir().join(format!("dxach-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::env::set_var("DXVIEW_SAVE_DIR", &dir);
    crate::travel::set_session(crate::travel::NewGame { char_id: "AchTest-1".into(), name: "Ach Test".into(), ..Default::default() });
    crate::achievements::character_changed();
    assert_eq!(crate::achievements::unlocked_count(), 0, "a new character starts with no achievements");

    let mut report = String::new();
    let mut missing = Vec::new();
    let mut choice_texts: HashMap<&'static str, String> = HashMap::new();
    if let Some(root) = std::env::var_os("ACH_GAME") {
        let root = Path::new(&root);
        let db = dx_level::conversations::ConDb::load(&root.join("System")).expect("DeusExConText.u");
        let maps = scan_maps(root);
        let ours = our_flags();
        let mut con_flags = HashSet::new();
        for c in &db.cons {
            for e in &c.events {
                if let dx_level::conversations::EventKind::SetFlag(v) = &e.kind {
                    con_flags.extend(v.iter().filter(|r| r.value).map(|r| r.name.to_ascii_lowercase()));
                }
            }
        }
        // The real choice lines (so part 2 feeds the hooks what the conversation shows).
        for (id, con, needle) in [("sure", "ClubMercedesConvo1", "Sure."), ("notmachine", "M11MeetGunther", "laputan machine")] {
            if let Some(i) = db.find(con) {
                for e in &db.cons[i].events {
                    if let dx_level::conversations::EventKind::Choice { choices, .. } = &e.kind {
                        if let Some(c) = choices.iter().find(|c| crate::util::contains_ci(&c.text, needle)) {
                            choice_texts.insert(id, c.text.clone());
                        }
                    }
                }
            }
        }
        report.push_str(&format!("data: {} conversations, {} maps\n", db.cons.len(), maps.mission.len()));
        for (id, name, _) in crate::achievements::LIST.iter() {
            for n in needs(id) {
                match check(&n, &db, &maps, &ours, &con_flags) {
                    Ok(s) => report.push_str(&format!("  ok   {id:<13} {name}: {s}\n")),
                    Err(s) => {
                        report.push_str(&format!("  MISS {id:<13} {name}: {s}\n"));
                        missing.push(format!("{id}: {s}"));
                    }
                }
            }
        }
    } else {
        report.push_str("(ACH_GAME not set: the game-data checks are skipped)\n");
    }
    let got = run_logic(&choice_texts);
    for (id, ok) in &got {
        report.push_str(&format!("  {} {id}\n", if *ok { "unlocked" } else { "LOCKED  " }));
    }
    println!("ACHIEVEMENT RUN\n{report}");
    // A second character starts clean and the first keeps its unlocks.
    crate::travel::set_session(crate::travel::NewGame { char_id: "AchTest-2".into(), name: "Other".into(), ..Default::default() });
    // ... except the endings, which count for every character once one has reached them.
    assert_eq!(crate::achievements::unlocked_count(), crate::achievements::GLOBAL.len(), "achievements are per character (but the endings are shared)");
    crate::travel::set_session(crate::travel::NewGame { char_id: "AchTest-1".into(), name: "Ach Test".into(), ..Default::default() });
    assert_eq!(crate::achievements::unlocked_count(), crate::achievements::LIST.len());
    let _ = std::fs::remove_dir_all(&dir);
    assert!(missing.is_empty(), "game data the achievements wait for is missing: {missing:?}");
    let locked: Vec<_> = got.iter().filter(|g| !g.1).map(|g| g.0).collect();
    assert!(locked.is_empty(), "achievements that never unlocked: {locked:?}");
}
