//! The quest rig: quick ways to get through the whole game when testing.
//!
//! * In play (Debug Mode, the Insert menu): warp to any map with the earlier missions' story flags set, complete
//!   every goal in the log, set this mission's flags, and write a quest report (map, goals, flags) to
//!   quest_state.txt beside the game's settings.
//! * Unattended: DXVIEW_QUESTRUN=all (or "map,map,...") loads every map in mission order, lets each run for
//!   DXVIEW_QUESTRUN_SECS (10) seconds and writes questrun.log: the mission script and conversations each map got,
//!   the goals and flags it set by itself, its pawns, and anything that went wrong on the way (QuestRun notes: a
//!   portal tag that isn't there, a conversation owner missing...). With DXVIEW_QUESTRUN_FLAGS=1 each mission
//!   starts with the earlier missions' flags, as if they'd been played. God mode is on for the run.
//!
//! Story flags for a mission (`progress_flags`): every conversation of the earlier missions counts as played
//! (<Name>_Played) and its SetFlag events as done. Choices set both branches' flags, so it's an approximation.

use bevy::prelude::*;
use imsim_core::flags::FlagValue;
use std::collections::{BTreeSet, HashSet};
use std::sync::Mutex;
use std::time::Instant;

/// Something a module noticed that a quest run should report.
static NOTES: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn note(s: impl Into<String>) {
    if let Ok(mut n) = NOTES.lock() {
        if n.len() < 200 {
            n.push(s.into());
        }
    }
}

/// A warp the menu asked for: (map, set the story flags of the missions before it).
static WARP: Mutex<Option<(String, bool)>> = Mutex::new(None);
/// Menu actions: "goals", "flags", "report".
static ACTIONS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

pub fn warp(map: &str, flags: bool) {
    if let Ok(mut w) = WARP.lock() {
        *w = Some((map.to_string(), flags));
    }
}

pub fn action(a: &'static str) {
    if let Ok(mut q) = ACTIONS.lock() {
        q.push(a);
    }
}

/// Every map in the install, in mission order (file names sort that way), with its mission number.
pub fn all_maps(root: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(root.join("Maps"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            crate::util::ends_with_ci(&n, ".dx").then(|| n[..n.len() - 3].to_string())
        })
        .collect();
    v.sort_by_key(|a| a.to_ascii_lowercase());
    v
}

/// The mission a map belongs to, from its name ("05_NYC_UNATCOHQ" -> 5; DX / 00_ -> 0).
pub fn mission_of(map: &str) -> i32 {
    map.split('_').next().and_then(|n| n.parse().ok()).unwrap_or(0)
}

/// Set the flags the missions before `mission` would have left (see the module notes). Returns how many.
pub fn progress_flags(fl: &mut imsim_core::flags::FlagBase, sys: &std::path::Path, mission: i32) -> usize {
    let Some(db) = crate::consys::cached_db(sys) else { return 0 };
    let mut n = 0;
    let mut seen = HashSet::new();
    for (m, list) in &db.missions {
        if *m >= mission || *m >= 16 {
            continue;
        }
        for &ci in list {
            if !seen.insert(ci) {
                continue;
            }
            let c = &db.cons[ci];
            if c.name.contains("_Bark") {
                continue;
            }
            fl.set(&format!("{}_Played", c.name), FlagValue::Bool(true), 0);
            n += 1;
            for e in &c.events {
                if let dx_level::conversations::EventKind::SetFlag(refs) = &e.kind {
                    for r in refs {
                        // Flags that expired before this mission stay unset.
                        if r.expiration == 0 || r.expiration >= mission {
                            fl.set(&r.name, FlagValue::Bool(r.value), 0);
                            n += 1;
                        }
                    }
                }
            }
        }
    }
    n
}

fn flag_text(v: &FlagValue) -> String {
    match v {
        FlagValue::Bool(b) => b.to_string(),
        other => format!("{other:?}"),
    }
}

/// The quest state now: map, mission, goals, notes count, every flag.
fn report(lvl: &dx_level::Level, fl: &imsim_core::flags::FlagBase) -> String {
    let mut s = format!("map {} mission {}\n", lvl.name, lvl.mission_number());
    if let Some(cs) = crate::consys::lock().as_ref() {
        s.push_str(&format!("goals ({}):\n", cs.goals.len()));
        for g in &cs.goals {
            s.push_str(&format!("  [{}] {}{}: {}\n", if g.completed { "x" } else { " " }, g.name, if g.primary { "" } else { " (secondary)" }, g.text));
        }
        s.push_str(&format!("notes: {}\n", cs.notes.len()));
    }
    let mut f: Vec<(String, String, i32)> = fl.iter().map(|(k, v, e)| (k.to_string(), flag_text(v), e)).collect();
    f.sort();
    s.push_str(&format!("flags ({}):\n", f.len()));
    for (k, v, e) in f {
        s.push_str(&format!("  {k} = {v}{}\n", if e != 0 { format!(" (expires {e})") } else { String::new() }));
    }
    s
}

#[derive(Default)]
struct Run {
    maps: Vec<String>,
    idx: usize,
    secs: f64,
    with_flags: bool,
    /// The map asked for, and when it settled.
    asked: Option<(String, Instant)>,
    settled: Option<Instant>,
    flags_before: HashSet<String>,
    goals_before: usize,
    frames: u32,
    log: String,
    last_mission: i32,
}

static RUN: Mutex<Option<Run>> = Mutex::new(None);

pub fn active() -> bool {
    RUN.lock().is_ok_and(|r| r.is_some())
}

#[allow(clippy::too_many_arguments)]
pub fn quest_system(
    level: Res<crate::LevelRes>,
    root: Res<crate::pickups::GameRoot>,
    mut flags: ResMut<crate::ai::GameFlags>,
    mut hud: ResMut<crate::movers::HudMessage>,
    npcs: Query<&crate::ai::Npc>,
    mut exit: MessageWriter<AppExit>,
    mut started: Local<bool>,
) {
    let _prof = crate::prof::scope("questrun::quest_system");
    let lvl = &level.0;
    let sys = root.0.join("System");
    // Menu actions.
    for a in ACTIONS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        match a {
            "goals" => {
                if let Some(cs) = crate::consys::lock().as_mut() {
                    let names: Vec<String> = cs.goals.iter().filter(|g| !g.completed).map(|g| g.name.clone()).collect();
                    for n in &names {
                        cs.goal_completed(n);
                    }
                    hud.text = format!("{} goals completed", names.len());
                    hud.time_left = 2.5;
                }
            }
            "flags" => {
                let m = lvl.mission_number();
                let n = progress_flags(&mut flags.0, &sys, m + 1);
                hud.text = format!("{n} story flags set (missions up to {m})");
                hud.time_left = 2.5;
            }
            _ => {
                let path = unreal_pkg::paths::data_dir().join("quest_state.txt");
                let ok = std::fs::write(&path, report(lvl, &flags.0)).is_ok();
                hud.text = if ok { format!("Quest report written to {}", path.display()) } else { "Couldn't write quest_state.txt".into() };
                hud.time_left = 3.0;
            }
        }
    }
    if let Some((map, with)) = WARP.lock().ok().and_then(|mut w| w.take()) {
        if with {
            let n = progress_flags(&mut flags.0, &sys, mission_of(&map));
            println!("QUEST warp to {map}: {n} story flags");
        }
        crate::loader::request(&map, crate::loader::Goal::Travel(None));
    }

    // The unattended run.
    if !*started {
        *started = true;
        if let Some(v) = env_val!("DXVIEW_QUESTRUN") {
            let maps: Vec<String> = if v.eq_ignore_ascii_case("all") || v == "1" {
                // Not the title map, the intro or the endings (cut scenes that leave by themselves).
                all_maps(&root.0)
                    .into_iter()
                    .filter(|m| !m.eq_ignore_ascii_case("DX") && !crate::util::contains_ci(m, "Intro") && !crate::util::starts_with_ci(m, "99_"))
                    .collect()
            } else {
                v.split(',').map(|s| s.trim().trim_end_matches(".dx").to_string()).filter(|s| !s.is_empty()).collect()
            };
            let secs = env_val!("DXVIEW_QUESTRUN_SECS").and_then(|s| s.parse().ok()).unwrap_or(10.0);
            crate::ai::GOD_MODE.store(true, std::sync::atomic::Ordering::Relaxed);
            let mut log = format!("QUEST RUN: {} maps, {secs} s each{}\n\n", maps.len(), if env_on!("DXVIEW_QUESTRUN_FLAGS") { ", earlier missions' flags set" } else { "" });
            log.push_str("map | mission | script | conversations | pawns | load s | new goals | new flags | notes\n");
            *RUN.lock().unwrap() = Some(Run { maps, secs, with_flags: env_on!("DXVIEW_QUESTRUN_FLAGS"), log, last_mission: -1, ..Default::default() });
        }
    }
    let mut g = RUN.lock().unwrap();
    let Some(run) = g.as_mut() else { return };
    if crate::loader::busy() || crate::loader::settling() {
        return;
    }
    // Ask for the next map.
    let Some(want) = run.maps.get(run.idx).cloned() else {
        let path = unreal_pkg::paths::data_dir().join("questrun.log");
        let _ = std::fs::write(&path, &run.log);
        println!("{}\nQUEST RUN done: {}", run.log, path.display());
        *g = None;
        exit.write(AppExit::Success);
        return;
    };
    let here = lvl.name.eq_ignore_ascii_case(&want) || crate::util::ends_with_ci(&lvl.name, &format!("{want}.dx"));
    if run.asked.as_ref().map(|a| a.0.as_str()) != Some(want.as_str()) {
        // On to the next map: the earlier missions' flags (when asked), and what's there before it starts.
        let m = mission_of(&want);
        if run.with_flags && m != run.last_mission {
            progress_flags(&mut flags.0, &sys, m);
        }
        run.last_mission = m;
        if let Ok(mut n) = NOTES.lock() {
            n.clear();
        }
        run.flags_before = flags.0.iter().map(|(k, _, _)| k.to_string()).collect();
        run.goals_before = crate::consys::lock().as_ref().map_or(0, |c| c.goals.len());
        run.asked = Some((want.clone(), Instant::now()));
        run.settled = None;
        run.frames = 0;
        if !here {
            crate::loader::request(&want, crate::loader::Goal::Travel(None));
        }
        return;
    }
    if !here {
        // Never arrived (a load error): note it and go on.
        if run.asked.as_ref().is_some_and(|a| a.1.elapsed().as_secs_f64() > 60.0) {
            run.log.push_str(&format!("{want} | - | - | - | - | - | - | - | DID NOT LOAD\n"));
            run.idx += 1;
            run.asked = None;
        }
        return;
    }
    let t0 = *run.settled.get_or_insert_with(Instant::now);
    run.frames += 1;
    if t0.elapsed().as_secs_f64() < run.secs {
        return;
    }
    // Done here: what the map did by itself.
    let load = run.asked.as_ref().map(|a| t0.duration_since(a.1).as_secs_f64()).unwrap_or(0.0);
    let info = lvl.level_info();
    let script = info
        .and_then(|a| match unreal_pkg::props::find(&a.props, "Script") {
            Some(unreal_pkg::props::Value::Path(p)) => Some(p.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "-".into());
    let (cons, goals_now): (usize, Vec<String>) = match crate::consys::lock().as_ref() {
        Some(cs) => (cs.mission_conversations(), cs.goals.iter().skip(run.goals_before).map(|g| g.name.clone()).collect()),
        None => (0, Vec::new()),
    };
    let new_flags: BTreeSet<String> = flags.0.iter().map(|(k, _, _)| k.to_string()).filter(|k| !run.flags_before.contains(k)).collect();
    let pawns = npcs.iter().filter(|n| n.state != crate::ai::NpcState::Dead).count();
    let notes = NOTES.lock().map(|mut n| std::mem::take(&mut *n)).unwrap_or_default();
    run.log.push_str(&format!(
        "{} | {} | {} | {} | {} | {:.1} | {} | {} | {}\n",
        want,
        lvl.mission_number(),
        script,
        cons,
        pawns,
        load,
        if goals_now.is_empty() { "-".into() } else { goals_now.join(" ") },
        if new_flags.is_empty() { "-".into() } else { new_flags.into_iter().collect::<Vec<_>>().join(" ") },
        if notes.is_empty() { "-".into() } else { notes.join("; ") },
    ));
    println!("QUEST {want} done ({:.0} fps)", run.frames as f64 / run.secs.max(0.1));
    run.idx += 1;
    run.asked = None;
}
