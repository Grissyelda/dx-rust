//! The Nameless Mod's invasions, written for DX RUST from how they behave in the player's own copy of the mod: once
//! the game has been finished (an ending map reached: Trestkon's bUnlockedSword, kept across games) and the player
//! has the Invasions option on, InvasionTriggers placed in the maps may summon an Invader - a hostile "rogue entity"
//! that shadows the player with a random loadout. The Invader's names, skins, weapon sets and the warning line are
//! the mod's own, read from the player's packages at run time.
// uc: InvasionTrigger.Trigger, InvasionTrigger.Touch, InvasionTrigger.SummonInvader, Invader.PostPostBeginPlay,
// uc: Invader.SetSkin, Invader.InitializeInventory, Invader.Died, tnmMission25.FirstFrame

use bevy::prelude::*;
use glam::DVec3;
use std::sync::Mutex;
use unreal_pkg::props::{find, Property, Value};

fn unlock_path() -> std::path::PathBuf {
    unreal_pkg::paths::data_dir().join("TNMSave").join("unlocks.txt")
}

/// Trestkon.bUnlockedSword (a config value: every later game has it).
pub fn unlocked() -> bool {
    std::fs::read_to_string(unlock_path()).is_ok_and(|t| t.lines().any(|l| l.trim().eq_ignore_ascii_case("UnlockedSword=1")))
}

/// tnmMission25.FirstFrame on an ending map.
pub fn unlock() {
    if unlocked() {
        return;
    }
    let p = unlock_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(p, "UnlockedSword=1\n");
    println!("TNM: invasions unlocked");
}

/// The warning shown: (invader's name, real time it came).
static BANNER: Mutex<Option<(String, f64)>> = Mutex::new(None);

/// The warning's clock (real seconds since it was first asked).
pub fn now() -> f64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64()
}

/// HUDInvaderEnter: the name of the invader that just came, for five seconds.
pub fn banner(now: f64) -> Option<String> {
    BANNER.lock().ok().and_then(|b| b.as_ref().filter(|b| now < b.1 + 5.0).map(|b| b.0.clone()))
}

/// Events naming InvasionTriggers (InvasionTrigger.Trigger), drained by the system.
static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn event(tag: &str) {
    if crate::tnmmode::active() {
        if let Ok(mut q) = EVENTS.lock() {
            q.push(tag.to_string());
        }
    }
}

/// Invader.Died: 50 skill points to the player who takes one down.
pub fn note_kill(class: &str, by_player: bool) {
    if by_player && crate::tnmmode::active() && crate::classtree::is_a(class, "Invader") {
        if let Some(msg) = crate::abilities::award_points(50) {
            crate::classic_hud::client_message(&msg);
        }
    }
}

struct Trig {
    tag: String,
    at: DVec3,
    radius: f64,
    height: f64,
    spawn_tag: String,
    chance: f64,
    ignore_los: bool,
    class: String,
    once: bool,
    inside: bool,
    gone: bool,
}

#[derive(Default)]
pub struct State {
    gen: u64,
    trigs: Vec<Trig>,
    seed: u64,
    hunt_t: f32,
}

fn rand(seed: &mut u64) -> f64 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    ((*seed >> 33) as f64) / (1u64 << 31) as f64
}

/// Invader.InitializeInventory's weapon sets, by ClassStyle, from the player's script: each
/// `case Class_X:` block's InitialInventory[n].Inventory / .Count assignments.
fn loadouts(classes: &mut unreal_pkg::classes::ClassDb) -> Vec<Vec<(u32, String, Option<i32>)>> {
    let Some(src) = classes.script_text("Invader") else { return Vec::new() };
    let lo = src.to_ascii_lowercase();
    let Some(sw) = lo.find("switch(classstyle)").or_else(|| lo.find("switch (classstyle)")) else { return Vec::new() };
    let body = &src[sw..];
    let mut out = Vec::new();
    for case in body.split("case Class_").skip(1) {
        let mut items: Vec<(u32, String, Option<i32>)> = Vec::new();
        let block = case.split("break;").next().unwrap_or("");
        for st in block.split(';') {
            let st = st.trim();
            let Some(i) = st.find("InitialInventory[") else { continue };
            let rest = &st[i + 17..];
            let Some((n, tail)) = rest.split_once(']') else { continue };
            let Ok(n) = n.trim().parse::<u32>() else { continue };
            if tail.trim_start().starts_with(".Inventory") {
                // Class'Pkg.Name' or DynamicLoadObject("Pkg.Name", ...)
                let path = tail
                    .split_once("DynamicLoadObject(\"")
                    .map(|(_, r)| r.split('"').next().unwrap_or("").to_string())
                    .or_else(|| tail.split_once("Class'").map(|(_, r)| r.split('\'').next().unwrap_or("").to_string()))
                    .unwrap_or_default();
                if !path.is_empty() {
                    items.retain(|x| x.0 != n);
                    items.push((n, path, None));
                }
            } else if tail.trim_start().starts_with(".Count") {
                if let Some(c) = tail.split('=').nth(1).and_then(|v| v.trim().parse::<i32>().ok()) {
                    if let Some(x) = items.iter_mut().find(|x| x.0 == n) {
                        x.2 = Some(c);
                    }
                }
            }
        }
        out.push(items);
    }
    out
}

/// Invader.SetSkin's three looks (its MultiSkins blocks in order: the soldier, the player's trench coat, the player's
/// other mesh).
fn skin_sets(classes: &mut unreal_pkg::classes::ClassDb) -> Vec<Vec<(u32, String)>> {
    let Some(src) = classes.script_text("Invader") else { return Vec::new() };
    let lo = src.to_ascii_lowercase();
    let Some(at) = lo.find("function setskin") else { return Vec::new() };
    let body = &src[at..];
    let end = body[1..].find("\nfunction ").map(|e| e + 1).unwrap_or(body.len());
    let mut sets: Vec<Vec<(u32, String)>> = Vec::new();
    let mut cur: Vec<(u32, String)> = Vec::new();
    for line in body[..end].lines() {
        let l = line.trim();
        let ll = l.to_ascii_lowercase();
        if let Some(i) = ll.find("multiskins[") {
            let rest = &l[i + 11..];
            if let Some((n, tail)) = rest.split_once(']') {
                if let (Ok(n), Some(p)) = (n.trim().parse::<u32>(), tail.split_once("Texture'").map(|(_, r)| r.split('\'').next().unwrap_or("").to_string())) {
                    if cur.iter().any(|x| x.0 == n) {
                        sets.push(std::mem::take(&mut cur));
                    }
                    cur.push((n, p));
                }
            }
        } else if !cur.is_empty() && (ll.starts_with('}') || ll.starts_with("else")) {
            sets.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        sets.push(cur);
    }
    sets.retain(|s| !s.is_empty());
    sets
}

pub fn reset_level() {
    if let Ok(mut q) = EVENTS.lock() {
        q.clear();
    }
}

#[allow(clippy::too_many_arguments)]
pub fn invasion_system(
    time: Res<Time>,
    real: Res<Time<Real>>,
    level: Res<crate::LevelRes>,
    player: Res<crate::Player>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut st: Local<State>,
) {
    if !crate::tnmmode::active() {
        return;
    }
    let lvl = &level.0;
    if crate::loader::new_map(&mut st.gen) {
        st.trigs.clear();
        if st.seed == 0 {
            st.seed = 0x1A7A_DE12 ^ (real.elapsed_secs_f64() * 1e6) as u64;
        }
        let d = assets.classes.defaults("InvasionTrigger");
        for a in lvl.actors.iter() {
            if !crate::classtree::is_a(&a.class, "InvasionTrigger") {
                continue;
            }
            let get = |k: &str| find(&a.props, k).or_else(|| find(&d, k)).cloned();
            let f = |k: &str, def: f64| match get(k) {
                Some(Value::Float(v)) => v as f64,
                _ => def,
            };
            let l = a.location();
            st.trigs.push(Trig {
                tag: crate::ai::actor_tag(a).to_string(),
                at: DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64),
                radius: f("CollisionRadius", 40.0),
                height: f("CollisionHeight", 40.0),
                spawn_tag: match get("SpawnLocation") {
                    Some(Value::Name(n)) if !n.eq_ignore_ascii_case("None") => n,
                    _ => String::new(),
                },
                chance: f("SpawnChance", 0.35),
                ignore_los: matches!(get("bIgnoreLOS"), Some(Value::Bool(true))),
                class: match get("InvaderClass") {
                    Some(Value::Path(p)) => p.rsplit('.').next().unwrap_or("Invader").to_string(),
                    _ => "Invader".into(),
                },
                once: !matches!(get("bTriggerOnceOnly"), Some(Value::Bool(false))),
                inside: false,
                gone: false,
            });
        }
    }
    // Invaders shadow the player (Orders 'Shadowing'): an idle one goes looking where he is, every two seconds.
    st.hunt_t += time.delta_secs();
    if st.hunt_t >= 2.0 {
        st.hunt_t = 0.0;
        crate::ai::world_req(crate::ai::WorldReq::Hunt(crate::ai::Sel::tag("Invader"), player.body.location));
    }
    if st.trigs.is_empty() {
        return;
    }
    let events: Vec<String> = EVENTS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    let me = player.body.location;
    let (pr, ph) = (player.params.radius, player.params.height);
    let mut fire: Vec<usize> = Vec::new();
    for (k, t) in st.trigs.iter_mut().enumerate() {
        if t.gone {
            continue;
        }
        let d = DVec3::new(me.x - t.at.x, me.y - t.at.y, 0.0).length();
        let inside = d <= t.radius + pr && (me.z - t.at.z).abs() <= t.height + ph;
        if inside && !t.inside {
            fire.push(k);
        }
        t.inside = inside;
        if events.iter().any(|e| e.eq_ignore_ascii_case(&t.tag)) {
            fire.push(k);
        }
    }
    for k in fire {
        let unlocked = unlocked();
        let st = &mut *st;
        let t = &mut st.trigs[k];
        if t.gone {
            continue;
        }
        // Not unlocked: the trigger is gone for good; invasions off: it waits.
        if !unlocked {
            t.gone = true;
            continue;
        }
        if !crate::tnmmode::option("bInvasionsEnabled") {
            continue;
        }
        // bAttemptingSpawn: a roll that passed but found nowhere to summon keeps the trigger for another try.
        let mut attempting = false;
        if rand(&mut st.seed) <= t.chance {
            attempting = true;
            if summon(lvl, &mut assets, t, me, &mut st.seed, now()) {
                for s in ["FireStaffReload", "MadScream"] {
                    let path = assets.classes.resolve_ref("InvasionTrigger", s, "Sound").unwrap_or_else(|| s.to_string());
                    crate::sound::play_player(&path, 1.0, 0.0);
                }
                attempting = false;
            }
        }
        if !attempting && t.once {
            t.gone = true;
        }
    }
}

/// SummonInvader: at a navigation point with the trigger's SpawnLocation tag the player can't see, or else at a
/// random one within 8192 of the trigger (not an exit, item spot or teleporter, more than 512 away).
fn summon(lvl: &dx_level::Level, assets: &mut crate::weapons::RuntimeAssets, t: &Trig, me: DVec3, seed: &mut u64, now: f64) -> bool {
    let eye = me + DVec3::new(0.0, 0.0, 40.0);
    let seen = |p: DVec3| lvl.collision.trace(eye, p, DVec3::ZERO).fraction >= 1.0;
    let mut spot: Option<(DVec3, i32)> = None;
    for a in &lvl.actors {
        if !crate::classtree::is_a(&a.class, "NavigationPoint") {
            continue;
        }
        let l = a.location();
        let p = DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64);
        let yaw = match find(&a.props, "Rotation") {
            Some(Value::Rotator(r)) => r[1],
            _ => 0,
        };
        let ok = if !t.spawn_tag.is_empty() {
            crate::ai::actor_tag(a).eq_ignore_ascii_case(&t.spawn_tag)
        } else {
            let bad = ["MapExit", "InventorySpot", "Teleporter"].iter().any(|c| crate::classtree::is_a(&a.class, c));
            !bad && p.distance(t.at) <= 8192.0 && p.distance(t.at) > 512.0 && rand(seed) < 0.1
        };
        if ok && (t.ignore_los || !seen(p)) {
            spot = Some((p, yaw));
            break;
        }
    }
    let Some((at, yaw)) = spot else { return false };
    let d = assets.classes.defaults(&t.class);
    // A name of its own.
    let names: Vec<String> = d
        .iter()
        .filter(|p| p.name.eq_ignore_ascii_case("names"))
        .filter_map(|p| match &p.value {
            Value::Str(s) if !s.is_empty() => Some(s.clone()),
            _ => None,
        })
        .collect();
    let name = if names.is_empty() { String::new() } else { names[(rand(seed) * names.len() as f64) as usize % names.len()].clone() };
    let sv = |k: &str| match find(&d, k) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    };
    let mut props = vec![
        Property { name: "Tag".into(), index: 0, value: Value::Name("Invader".into()) },
        Property { name: "Orders".into(), index: 0, value: Value::Name("Wandering".into()) },
        Property { name: "MinHealth".into(), index: 0, value: Value::Float(0.0) },
        Property { name: "InitialAlliances".into(), index: 6, value: Value::Alliance("Player".into(), -1.0, true) },
        Property { name: "FamiliarName".into(), index: 0, value: Value::Str(format!("{} {name}", sv("FamiliarName"))) },
        Property { name: "UnfamiliarName".into(), index: 0, value: Value::Str(format!("{} {name}", sv("UnfamiliarName"))) },
    ];
    // InitializeInventory: one of its weapon sets at random (fists always in the last slot).
    let sets = loadouts(&mut assets.classes);
    let usable: Vec<&Vec<(u32, String, Option<i32>)>> = sets.iter().filter(|s| !s.is_empty()).collect();
    if !usable.is_empty() {
        let pick = usable[(rand(seed) * usable.len() as f64) as usize % usable.len()];
        for (n, class, count) in pick {
            props.push(Property { name: "InitialInventory".into(), index: *n, value: Value::Item(class.clone(), count.unwrap_or(1)) });
        }
    }
    // SetSkin: half of them dressed as the soldier, the rest as a dark copy of the player.
    let skins = skin_sets(&mut assets.classes);
    let player_mesh = crate::travel::jc_mesh();
    let set = if rand(seed) < 0.5 || skins.len() < 3 {
        skins.first()
    } else {
        props.push(Property { name: "Mesh".into(), index: 0, value: Value::Path(player_mesh.clone()) });
        if crate::util::contains_ci(&player_mesh, "Trenchkon") { skins.get(1) } else { skins.get(2) }
    };
    for (n, tex) in set.into_iter().flatten() {
        props.push(Property { name: "MultiSkins".into(), index: *n, value: Value::Path(tex.clone()) });
    }
    println!("TNM invasion: {} '{name}' at {at:?}", t.class);
    crate::missions::queue_pawn(&t.class, at, yaw, props);
    if let Ok(mut b) = BANNER.lock() {
        *b = Some((name, now));
    }
    true
}
