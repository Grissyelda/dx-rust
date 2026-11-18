//! The Nameless Mod's challenge maps: the combat course (95_Combat1..9) and the stealth course (95_Stealth1..9),
//! the maps whose mission script is tnmmissioncombat / tnmmissionstealth. Written for DX RUST from how these maps
//! behave in the player's own copy of the mod; the messages come from its packages at run time.
//!
//! On arrival a count-up timer starts with the map's par time (the player's best for the chosen difficulty when it
//! beats the stock par, else the stock one) and limit (70 / 90 / 150 / 150 / 120 / 60 / 120 / 200 / 300 s for
//! maps 1..9); the course's cameras and guards are tuned by difficulty (Easy: cameras slow to react and forgetful;
//! Hard: instant, sweeping twice as fast, guards twice as sharp-eyed); skill points are taken away and, on the combat
//! course, every skill set for the difficulty and the map's weapon handed over. The run is won when the last pawn in
//! the world falls (combat; a "targets left" line after each other one) or the map raises CompleteLevel; it's lost
//! when the player dies, the limit runs out (ResetLevelTime), the map raises ResetLevel, or - stealth on Normal and
//! Hard - someone spots him. The end freezes him in place (a cheer pose with a disco light, or rubbing his eyes);
//! three seconds after a win the next map of the course loads (the last goes back to the first; with
//! bLoopStealthUntilPB a run that missed the par plays again), two seconds after a loss the map restarts. Beating
//! par records the new best and says so. The level's EndMover sinks and spins faster once the run is won.
//!
//! The difficulty is chosen in the mod's challenge menu; here it is the setting TnmChallengeDifficulty (0..2).

use std::sync::Mutex;
use unreal_pkg::props::{find, Value};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Course {
    Combat,
    Stealth,
}

const LIMITS: [f32; 9] = [70.0, 90.0, 150.0, 150.0, 120.0, 60.0, 120.0, 200.0, 300.0];

struct Run {
    course: Course,
    map: usize,
    difficulty: usize,
    time: f32,
    par: f32,
    limit: f32,
    win_at: Option<f32>,
    fail_at: Option<f32>,
    /// The clock since arrival (Level.TimeSeconds).
    clock: f32,
    /// Living pawns in the world last frame (combat: counting down to the last).
    living: Option<usize>,
    kit_done: bool,
    beat_par: bool,
    /// HUDInvaderEnter: (message index, clock when shown).
    banner: Option<(usize, f32)>,
}

static RUN: Mutex<Option<Run>> = Mutex::new(None);
static DETECTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn best_path() -> std::path::PathBuf {
    unreal_pkg::paths::data_dir().join("TNMSave").join("challenges.txt")
}

fn bests() -> Vec<(String, f32)> {
    std::fs::read_to_string(best_path())
        .map(|t| {
            t.lines()
                .filter_map(|l| l.split_once('=').and_then(|(k, v)| v.trim().parse().ok().map(|v| (k.trim().to_string(), v))))
                .collect()
        })
        .unwrap_or_default()
}

fn set_best(key: &str, t: f32) {
    let mut all = bests();
    all.retain(|x| x.0 != key);
    all.push((key.to_string(), t));
    all.sort_by(|a, b| a.0.cmp(&b.0));
    let text: String = all.iter().map(|(k, v)| format!("{k}={v}\n")).collect();
    let p = best_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(p, text);
}

const DIFF: [&str; 3] = ["Easy", "Normal", "Hard"];

fn prefix(c: Course) -> &'static str {
    match c {
        Course::Combat => "combat",
        Course::Stealth => "stealth",
    }
}

/// The par-time property in Trestkon's config (combat3ParTimeHard ...).
fn par_key(c: Course, map: usize, diff: usize) -> String {
    format!("{}{}ParTime{}", prefix(c), map, DIFF[diff.min(2)])
}

/// Which course this map belongs to (its mission script), and its number.
pub fn course_of(lvl: &dx_level::Level) -> Option<(Course, usize)> {
    if !crate::tnmmode::active() {
        return None;
    }
    let script = match lvl.level_info().and_then(|a| find(&a.props, "Script")) {
        Some(Value::Path(p) | Value::Str(p) | Value::Name(p)) => p.rsplit('.').next().unwrap_or("").to_ascii_lowercase(),
        _ => return None,
    };
    let course = match script.as_str() {
        "tnmmissioncombat" => Course::Combat,
        "tnmmissionstealth" => Course::Stealth,
        _ => return None,
    };
    let n = lvl.name.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<String>();
    let n: usize = n.chars().rev().collect::<String>().parse().unwrap_or(9);
    Some((course, n.clamp(1, 9)))
}

pub fn active() -> bool {
    RUN.lock().is_ok_and(|r| r.is_some())
}

/// Frozen at the end of a run (StealthCinematic: no moving, no firing).
pub fn frozen() -> bool {
    RUN.lock().is_ok_and(|r| r.as_ref().is_some_and(|r| r.win_at.is_some() || r.fail_at.is_some()))
}

/// PlayerDetected (missions.rs): on the stealth course above Easy, a lost run.
pub fn detected() {
    DETECTED.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// HandleGameEvent: ResetLevel / ResetLevelTime / CompleteLevel raised by the map.
pub fn game_event(name: &str) -> bool {
    let known = ["ResetLevel", "ResetLevelTime", "CompleteLevel"].iter().any(|n| n.eq_ignore_ascii_case(name));
    if known && active() {
        if let Ok(mut q) = EVENTS.lock() {
            q.push(name.to_string());
        }
    }
    known && active()
}

pub fn reset_level() {
    if let Ok(mut r) = RUN.lock() {
        *r = None;
    }
    DETECTED.store(false, std::sync::atomic::Ordering::Relaxed);
    if let Ok(mut q) = EVENTS.lock() {
        q.clear();
    }
}

/// The HUD timer (TNMTimerDisplay): time, limit, par and whether each has turned red, and the flash once done.
pub fn timer() -> Option<(f32, f32, f32, bool)> {
    RUN.lock().ok().and_then(|r| r.as_ref().map(|r| (r.time, r.limit, r.par, r.win_at.is_some() || r.fail_at.is_some())))
}

/// Seconds since the run's map came in (for the timer's flashing).
pub fn clock() -> f32 {
    RUN.lock().ok().and_then(|r| r.as_ref().map(|r| r.clock)).unwrap_or(0.0)
}

/// HUDInvaderEnter's line (index into its InvaderEnter strings), shown five seconds.
pub fn banner() -> Option<usize> {
    RUN.lock().ok().and_then(|r| r.as_ref().and_then(|r| r.banner.filter(|b| r.clock < b.1 + 5.0).map(|b| b.0)))
}

/// The ConvertTime format: "1m, 5s, 250ms".
fn convert_time(t: f32) -> String {
    let mins = (t / 60.0) as i32;
    let secs = t % 60.0;
    let mut s = String::new();
    if mins > 0 {
        s.push_str(&format!("{mins}m, "));
    }
    if secs as i32 > 0 {
        s.push_str(&format!("{}s, ", secs as i32));
    }
    s.push_str(&format!("{}ms", ((secs % 1.0) * 1000.0) as i32));
    s
}

fn script_str(assets: &mut crate::weapons::RuntimeAssets, class: &str, key: &str) -> String {
    match find(&assets.classes.defaults(class), key) {
        Some(Value::Str(v)) => v.clone(),
        _ => String::new(),
    }
}

/// The combat course's weapon for each map: (weapon class, ammo class, rounds).
const KITS: [Option<(&str, &str, u32)>; 9] = [
    Some(("WeaponDoubleGuns", "Ammo45", 80)),
    Some(("WeaponTNMAssaultGun", "Ammo762mm", 240)),
    Some(("WeaponTNMRifle", "Ammo3006", 60)),
    Some(("WeaponTNMLAM", "AmmoLAM", 2)),
    Some(("WeaponTNMNanosword", "", 0)),
    Some(("WeaponTNMGEPGun", "AmmoRocket", 2)),
    Some(("WeaponTNMAssaultShotgun", "AmmoSabot", 24)),
    None,
    None,
];

const SKILLS: [&str; 11] = [
    "SkillWeaponRifle",
    "SkillWeaponPistol",
    "SkillWeaponHeavy",
    "SkillWeaponLowTech",
    "SkillWeaponFists",
    "SkillDemolition",
    "SkillComputer",
    "SkillLockpicking",
    "SkillTech",
    "SkillMedicine",
    "SkillEnviro",
];

#[allow(clippy::too_many_arguments)]
pub fn bonus_system(
    time: bevy::prelude::Res<bevy::prelude::Time>,
    level: bevy::prelude::Res<crate::LevelRes>,
    mut player: bevy::prelude::ResMut<crate::Player>,
    ph: bevy::prelude::Res<crate::ai::PlayerHealth>,
    mut inv: bevy::prelude::ResMut<crate::pickups::PlayerInventory>,
    mut weapons: bevy::prelude::ResMut<crate::weapons::Weapons>,
    mut assets: bevy::prelude::ResMut<crate::weapons::RuntimeAssets>,
    mut movers: bevy::prelude::ResMut<crate::movers::Movers>,
    mut npcs: bevy::prelude::Query<&mut crate::ai::Npc, bevy::prelude::Without<crate::ai::OutOfWorld>>,
    mut map_gen: bevy::prelude::Local<u64>,
) {
    if crate::loader::new_map(&mut map_gen) {
        reset_level();
        if let Some((course, map)) = course_of(&level.0) {
            let difficulty = (crate::settings::get("TnmChallengeDifficulty") as usize).min(2);
            let key = par_key(course, map, difficulty);
            let stock = match find(&assets.classes.defaults(crate::tnmmode::player_class()), &key) {
                Some(Value::Float(v)) => *v,
                Some(Value::Int(v)) => *v as f32,
                _ => 200.0,
            };
            let best = bests().into_iter().find(|b| b.0 == key).map(|b| b.1).unwrap_or(0.0);
            let par = if best != 0.0 && best < stock { best } else { stock };
            let limit = LIMITS[map - 1];
            println!("TNM challenge: {} {map}, par {par} limit {limit}", prefix(course));
            *RUN.lock().unwrap_or_else(|e| e.into_inner()) = Some(Run {
                course,
                map,
                difficulty,
                time: 0.0,
                par,
                limit,
                win_at: None,
                fail_at: None,
                clock: 0.0,
                living: None,
                kit_done: false,
                beat_par: false,
                banner: None,
            });
            crate::security::challenge_tuning(difficulty);
        }
    }
    let lvl = &level.0;
    let dt = time.delta_secs();
    let mut g = RUN.lock().unwrap_or_else(|e| e.into_inner());
    let Some(r) = g.as_mut() else { return };
    r.clock += dt;
    let done = r.win_at.is_some() || r.fail_at.is_some();
    // The kit (and the skill points taken away) once the player is set up.
    if !r.kit_done && r.clock > 0.5 {
        r.kit_done = true;
        crate::abilities::clear_points();
        // Hard: the course's guards see twice as well (VisibilityThreshold halved).
        if r.difficulty == 2 {
            for mut n in &mut npcs {
                if crate::classtree::is_a(&lvl.actors[n.actor].class, "TNMPawn") {
                    n.vis_threshold /= 2.0;
                }
            }
        }
        if r.course == Course::Combat {
            let level = [2, 3, 3][r.difficulty.min(2)];
            for s in SKILLS {
                crate::abilities::set_skill_level(s, level);
            }
            if let Some((w, ammo, n)) = KITS[r.map - 1] {
                if !ammo.is_empty() {
                    match inv.0.ammo.iter_mut().find(|x| x.0 == ammo) {
                        Some(x) => x.1 = n,
                        None => inv.0.ammo.push((ammo.to_string(), n)),
                    }
                }
                let d = assets.classes.defaults(w);
                if !d.is_empty() {
                    let _ = crate::pickups::give_weapon(&mut inv.0, w, &d, 0, &mut assets);
                    weapons.pending_class = Some(w.to_string());
                }
            }
            // Map 3: one-shot targets with no cloak, and on Hard the rifle skill drops to Trained.
            if r.map == 3 && r.difficulty == 2 {
                crate::abilities::set_skill_level("SkillWeaponRifle", 1);
            }
        }
    }
    if !done {
        r.time += dt;
    }
    let mut events: Vec<String> = EVENTS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    if !done && r.time >= r.limit {
        events.push("ResetLevelTime".into());
    }
    if !done && DETECTED.swap(false, std::sync::atomic::Ordering::Relaxed) && r.course == Course::Stealth && r.difficulty > 0 {
        r.banner = Some((4, r.clock));
        r.fail_at = Some(r.clock);
        println!("TNM challenge: spotted");
    }
    if !done && ph.dead && r.fail_at.is_none() {
        // PlayerKilled (a cinematic death in an 'Atmospheric' zone waits a little longer).
        r.fail_at = Some(r.clock);
    }
    // Combat: the run is won when the last pawn in the world falls.
    if r.course == Course::Combat && !done {
        let living = npcs.iter().filter(|n| n.state != crate::ai::NpcState::Dead && n.net.is_none()).count();
        if let Some(before) = r.living {
            if living < before {
                if living == 0 {
                    events.push("CompleteLevel".into());
                } else {
                    let t = script_str(&mut assets, "tnmmissioncombat", "remainingtargets");
                    if !t.is_empty() {
                        crate::classic_hud::client_message(&t.replacen("%d", &living.to_string(), 1).replacen("%s", &living.to_string(), 1));
                    }
                }
            }
        }
        r.living = Some(living);
    }
    for e in events {
        if r.win_at.is_some() || r.fail_at.is_some() {
            break;
        }
        if e.eq_ignore_ascii_case("CompleteLevel") {
            r.win_at = Some(r.clock);
            let class = if r.course == Course::Combat { "tnmmissioncombat" } else { "tnmmissionstealth" };
            if r.time < r.par {
                let t = script_str(&mut assets, class, "beatPar");
                if !t.is_empty() {
                    crate::classic_hud::client_message(&t);
                }
                crate::sound::play_player("TNMSounds.OneShot.tnmLogSkillPoints", 1.0, 0.0);
                r.beat_par = true;
                set_best(&par_key(r.course, r.map, r.difficulty), r.time);
            } else {
                let t = script_str(&mut assets, class, "overPar");
                if !t.is_empty() {
                    crate::classic_hud::client_message(&t.replacen("%s", &convert_time(r.time - r.par), 1));
                }
            }
            println!("TNM challenge: won in {:.2}s", r.time);
        } else {
            let k = if e.eq_ignore_ascii_case("ResetLevelTime") { 3 } else { 2 };
            r.banner = Some((k, r.clock));
            r.fail_at = Some(r.clock);
            println!("TNM challenge: failed ({e})");
        }
    }
    // The end: frozen; the EndMover sinks and spins up after a win.
    if r.win_at.is_some() || r.fail_at.is_some() {
        player.body.velocity = glam::DVec3::ZERO;
    }
    if r.win_at.is_some() {
        for (i, m) in lvl.movers.iter().enumerate() {
            if m.tag.eq_ignore_ascii_case("EndMover") {
                let rt = &mut movers.rt[i];
                rt.location.z -= 1.2 * dt as f64 * 60.0;
                rt.moved_since_lit = true;
                let rate = rt.rot_rate[1].max(4096);
                rt.rot_rate[1] = ((rate as f32) * 1.03f32.powf(dt * 60.0)) as i32;
                rt.desired_rot = Some([rt.rotation[0], (rt.rotation[1] + 32000).rem_euclid(65536), rt.rotation[2]]);
            }
        }
    }
    let (next, restart) = match (r.win_at, r.fail_at) {
        (Some(w), _) if r.clock >= w + 3.0 => {
            let loop_pb = matches!(find(&assets.classes.defaults(crate::tnmmode::player_class()), "bLoopStealthUntilPB"), Some(Value::Bool(true)));
            if loop_pb && !r.beat_par {
                (None, true)
            } else {
                let n = if r.map >= 8 && r.course == Course::Combat { 1 } else if r.map >= 9 { 1 } else { r.map + 1 };
                let name = match r.course {
                    Course::Combat => format!("95_Combat{n}"),
                    Course::Stealth => format!("95_Stealth{n}"),
                };
                (Some(name), false)
            }
        }
        (None, Some(f)) if r.clock >= f + 2.0 => (None, true),
        _ => (None, false),
    };
    if restart || next.is_some() {
        *g = None;
        drop(g);
        match next {
            Some(m) => crate::loader::request(&m, crate::loader::Goal::Travel(None)),
            None => crate::loader::request(&lvl.name, crate::loader::Goal::Restart),
        }
    }
}
