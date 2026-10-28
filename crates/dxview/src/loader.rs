//! Changing maps inside the running game (the original's level travel: one exe, a loading screen between maps).
//!
//! A request names the map and why we're going there (new game, saved game, travel, intro, restart). The map
//! is read on a background thread while the current one freezes behind the original's "LOADING" message
//! (Engine Console.LoadingMessage); then, in one frame at the start of the next update, the old map's entities
//! are despawned, every per-map resource and module state is reset, the new map is put in place and every
//! map-building system (the `LevelSetup` schedule, which ran at startup for the first map) runs again. What
//! travels with the player is captured before and put back after through the save code (save.rs), the same
//! path a loaded game takes.

use crate::travel::NewGame;
use bevy::ecs::schedule::ScheduleLabel;
use bevy::prelude::*;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Every system that builds the game from the loaded map (formerly Startup).
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct LevelSetup;

/// Why we're going to a map.
pub enum Goal {
    /// Start a new game there with this character.
    NewGame(NewGame),
    /// Put a saved game back.
    Load(Box<crate::save::SaveGame>),
    /// Leave this map for another, carrying the player (MapExit / Teleporter / mission scripts), arriving at
    /// the PlayerStart or at the actor tagged like the URL's "#tag" (Teleporter URLs).
    Travel(Option<String>),
    /// The intro cutscene; then a new game with this character on this map, or back to the title map.
    Intro(Option<(NewGame, String)>),
    /// The same map from scratch with the same character (Enter after dying with no save to go back to).
    Restart,
    /// The title map (DX.dx).
    Title,
}

struct Job {
    map: String,
    goal: Goal,
    rx: std::sync::mpsc::Receiver<Result<(dx_level::Level, (imsim_core::movement::MoveParams, f64, f64)), String>>,
    started: std::time::Instant,
}

static REQUEST: Mutex<Option<(String, Goal)>> = Mutex::new(None);
static JOB: Mutex<Option<Job>> = Mutex::new(None);
static KEEP: Mutex<Option<HashSet<Entity>>> = Mutex::new(None);
static GEN: AtomicU64 = AtomicU64::new(1);
/// After the intro: who to start the new game as (MissionIntro.PostIntro -> StartNewGame).
static AFTER_INTRO: Mutex<Option<(NewGame, String)>> = Mutex::new(None);

type Loaded = (dx_level::Level, (imsim_core::movement::MoveParams, f64, f64));

/// Hub streaming: the map behind the nearest MapExit / Teleporter is read in the background while the player
/// walks toward it, so stepping through is nearly instant (one map held at a time).
struct Prefetch {
    map: String,
    /// Couldn't be read: don't try again on this map.
    failed: bool,
    rx: Option<std::sync::mpsc::Receiver<Result<Loaded, String>>>,
    ready: Option<Loaded>,
}

static PREFETCH: Mutex<Option<Prefetch>> = Mutex::new(None);
/// This map's exits (generation, [(where, destination)]).
static EXITS: Mutex<(u64, Vec<(glam::DVec3, String)>)> = Mutex::new((0, Vec::new()));

fn norm(map: &str) -> String {
    let mut m = map.split('#').next().unwrap_or(map).trim().to_ascii_lowercase();
    if !m.ends_with(".dx") {
        m.push_str(".dx");
    }
    m
}

fn start_prefetch(root: std::path::PathBuf, map: &str) {
    let m = norm(map);
    let Ok(mut p) = PREFETCH.lock() else { return };
    if p.as_ref().is_some_and(|x| x.map == m) {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    // Load by the name as written (file names are case-sensitive off Windows).
    let mut mm = map.split('#').next().unwrap_or(map).trim().to_string();
    if !mm.to_ascii_lowercase().ends_with(".dx") {
        mm.push_str(".dx");
    }
    let _ = std::thread::Builder::new().name("level-prefetch".into()).spawn(move || {
        let r = dx_level::load_level(&root, &mm).map(|l| (l, crate::player_params(&root)));
        let _ = tx.send(r);
    });
    println!("loader: prefetching {m}");
    *p = Some(Prefetch { map: m, failed: false, rx: Some(rx), ready: None });
}

/// A prefetched copy of `map`, if it's ready (or nearly: waits for a load already under way).
fn take_prefetched(map: &str) -> Option<Loaded> {
    let m = norm(map);
    let mut g = PREFETCH.lock().ok()?;
    let p = g.as_mut().filter(|p| p.map == m && !p.failed)?;
    if p.ready.is_none() {
        let r = p.rx.take()?.recv().ok()?.ok()?;
        p.ready = Some(r);
    }
    g.take().and_then(|p| p.ready)
}

/// Each frame: keep the map behind the nearest exit within reach read ahead.
fn prefetch_tick(world: &mut World) {
    if let Ok(mut g) = PREFETCH.lock() {
        if let Some(p) = g.as_mut() {
            if let Some(rx) = p.rx.as_ref() {
                match rx.try_recv() {
                    Ok(Ok(l)) => {
                        p.ready = Some(l);
                        p.rx = None;
                    }
                    Ok(Err(_)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        p.rx = None;
                        p.failed = true;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                }
            }
        }
    }
    if std::env::var("DXVIEW_NO_PREFETCH").is_ok() || crate::menus::intro_map() {
        return;
    }
    let Some(level) = world.get_resource::<crate::LevelRes>() else { return };
    let Some(player) = world.get_resource::<crate::Player>() else { return };
    let pc = player.body.location;
    let Ok(mut ex) = EXITS.lock() else { return };
    if ex.0 != generation() {
        ex.0 = generation();
        ex.1 = level
            .0
            .actors
            .iter()
            .filter(|a| a.class == "MapExit" || a.class == "Teleporter")
            .filter_map(|a| {
                let key = if a.class == "Teleporter" { "URL" } else { "DestMap" };
                let dest = match unreal_pkg::props::find(&a.props, key) {
                    Some(unreal_pkg::props::Value::Str(v)) | Some(unreal_pkg::props::Value::Name(v)) => v.clone(),
                    _ => return None,
                };
                // A local teleporter's URL is a bare tag (no map).
                if dest.is_empty() || (a.class == "Teleporter" && !dest.contains('#') && !dest.contains('_')) {
                    return None;
                }
                let l = a.location();
                Some((glam::DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64), dest))
            })
            .collect();
    }
    let mut best: Option<(f64, String)> = None;
    for (at, dest) in &ex.1 {
        let d = pc.distance(*at);
        if d < 1500.0 && best.as_ref().is_none_or(|b| d < b.0) {
            best = Some((d, dest.clone()));
        }
    }
    drop(ex);
    if let Some((_, dest)) = best {
        let root = world.resource::<crate::pickups::GameRoot>().0.clone();
        start_prefetch(root, &dest);
    }
}

/// Go to `map` (file name; ".dx" added if missing).
pub fn request(map: &str, goal: Goal) {
    let mut m = map.to_string();
    if !m.to_ascii_lowercase().ends_with(".dx") {
        m.push_str(".dx");
    }
    println!("loader: going to {m}");
    if let Ok(mut r) = REQUEST.lock() {
        *r = Some((m, goal));
    }
}

/// A map is loading (the loading screen is up and the game is frozen).
pub fn busy() -> bool {
    REQUEST.lock().map(|r| r.is_some()).unwrap_or(false) || JOB.lock().map(|j| j.is_some()).unwrap_or(false)
}

/// Bumped every time a map is put in place; systems with per-map `Local` state compare it to start over.
pub fn generation() -> u64 {
    GEN.load(Ordering::Relaxed)
}

/// True once per map for this `Local` (and on the first call): reset the system's per-map state.
pub fn new_map(seen: &mut u64) -> bool {
    let g = generation();
    if *seen != g {
        *seen = g;
        true
    } else {
        false
    }
}

/// The character the intro will start a new game with, taken by MissionIntro.PostIntro.
pub fn take_after_intro() -> Option<(NewGame, String)> {
    AFTER_INTRO.lock().ok().and_then(|mut a| a.take())
}

/// Startup: remember what exists before the game builds anything (Bevy's own entities: window, monitors,
/// observers...), then build the first map.
pub fn first_level_setup(world: &mut World) {
    let keep: HashSet<Entity> = world.query::<Entity>().iter(world).collect();
    if let Ok(mut k) = KEEP.lock() {
        *k = Some(keep);
    }
    world.run_schedule(LevelSetup);
}

/// First schedule (before the save restore): start a requested load, and swap the map in once it's read.
pub fn loader_system(world: &mut World) {
    dev_goto(world);
    if !busy() {
        prefetch_tick(world);
    }
    let req = REQUEST.lock().ok().and_then(|mut r| r.take());
    if let Some((map, goal)) = req {
        let root = world.resource::<crate::pickups::GameRoot>().0.clone();
        // The player's face goes onto Paul, his carcass and JC's double as the map's actors are built.
        // (And Lay D Denton's choices: her looks, eye height and stand-ins are settled while the map is built.)
        let look = match &goal {
            Goal::NewGame(ng) | Goal::Intro(Some((ng, _))) => Some((ng.skin, ng.female, ng.retro_morpheus, ng.male_interactions)),
            Goal::Load(s) => Some((s.global.skin, s.global.female, s.global.retro_morpheus, s.global.male_interactions)),
            Goal::Intro(None) | Goal::Title => None,
            Goal::Travel(_) | Goal::Restart => crate::travel::session().map(|s| (s.skin, s.female, s.retro_morpheus, s.male_interactions)),
        };
        if let Some((sk, f, r, m)) = look {
            crate::travel::set_look(sk, f, r, m);
        }
        dx_level::actors::set_overrides(crate::travel::skin_overrides(&map));
        if matches!(goal, Goal::Travel(_)) {
            if let Some((level, params)) = take_prefetched(&map) {
                println!("loader: {map} was read ahead");
                swap(world, map, goal, level, params);
                return;
            }
        }
        if let Ok(mut p) = PREFETCH.lock() {
            *p = None;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let m = map.clone();
        let _ = std::thread::Builder::new().name("level-load".into()).spawn(move || {
            let r = dx_level::load_level(&root, &m).map(|l| (l, crate::player_params(&root)));
            // Developer aid: DXVIEW_LOAD_DELAY=<secs> holds the loading screen up longer (to look at it).
            if let Some(d) = std::env::var("DXVIEW_LOAD_DELAY").ok().and_then(|v| v.parse::<f32>().ok()) {
                std::thread::sleep(std::time::Duration::from_secs_f32(d));
            }
            let _ = tx.send(r);
        });
        if let Ok(mut j) = JOB.lock() {
            *j = Some(Job { map, goal, rx, started: std::time::Instant::now() });
        }
        // Frozen while it loads, like the original.
        world.resource_mut::<Time<Virtual>>().pause();
        return;
    }
    let done = {
        let Ok(mut j) = JOB.lock() else { return };
        let Some(job) = j.as_ref() else { return };
        match job.rx.try_recv() {
            Ok(r) => Some((j.take().unwrap(), r)),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some((j.take().unwrap(), Err("the loader stopped".into()))),
        }
    };
    let Some((job, result)) = done else { return };
    match result {
        Ok((level, params)) => {
            let t = job.started.elapsed().as_secs_f32();
            swap(world, job.map, job.goal, level, params);
            println!("loader: in place after {:.2}s", job.started.elapsed().as_secs_f32().max(t));
        }
        Err(e) => {
            eprintln!("loader: can't load {}: {e}", job.map);
            world.resource_mut::<Time<Virtual>>().unpause();
            let mut hud = world.resource_mut::<crate::movers::HudMessage>();
            hud.text = format!("Can't load {}", job.map);
            hud.time_left = 4.0;
        }
    }
}

/// Entities Bevy owns that must survive a map change.
fn protected(world: &World, e: Entity) -> bool {
    let Ok(r) = world.get_entity(e) else { return true };
    // (Resources are entities too in Bevy 0.19: they're replaced, not despawned.)
    r.contains::<bevy::ecs::resource::IsResource>()
        || r.contains::<Window>()
        || r.contains::<bevy::window::Monitor>()
        || r.contains::<bevy::picking::pointer::PointerId>()
}

fn swap(world: &mut World, map: String, goal: Goal, mut level: dx_level::Level, params: (imsim_core::movement::MoveParams, f64, f64)) {
    // What travels with the player, from the map being left.
    if matches!(goal, Goal::Travel(_)) {
        // MissionScript.PreTravel of the map being left.
        world.resource_scope(|world, mut flags: Mut<crate::ai::GameFlags>| {
            let lvl = &world.resource::<crate::LevelRes>().0;
            crate::missions::pre_travel_current(&mut flags.0, lvl);
        });
    }
    let carry = matches!(goal, Goal::Travel(_)).then(|| crate::save::capture_travel(world));
    // Arriving at a tagged actor (Teleporter URL "map#tag"): it stands in for the PlayerStart.
    if let Goal::Travel(Some(tag)) = &goal {
        let hit = level.actors.iter().find(|a| {
            matches!(unreal_pkg::props::find(&a.props, "Tag"), Some(unreal_pkg::props::Value::Name(t)) if t.eq_ignore_ascii_case(tag))
        });
        match hit {
            Some(a) => {
                let l = a.location();
                let yaw = match unreal_pkg::props::find(&a.props, "Rotation") {
                    Some(unreal_pkg::props::Value::Rotator(r)) => r[1] as f32 * std::f32::consts::PI / 32768.0,
                    _ => 0.0,
                };
                level.player_start = Some(dx_level::PlayerStart { position: dx_level::ue_to_bevy(l), yaw });
            }
            None => eprintln!("loader: no actor tagged {tag} on {map}; arriving at the PlayerStart"),
        }
    }

    // ---- the old map goes
    let keep = KEEP.lock().ok().and_then(|k| k.clone()).unwrap_or_default();
    // Roots only: despawning a parent takes its children with it.
    let all: Vec<Entity> = world.query_filtered::<Entity, Without<ChildOf>>().iter(world).collect();
    for e in all {
        if keep.contains(&e) || protected(world, e) {
            continue;
        }
        if let Ok(em) = world.get_entity_mut(e) {
            em.despawn();
        }
    }
    reset_modules();
    reset_resources(world);

    // ---- who the player is now
    let arrive = match goal {
        Goal::NewGame(ng) => {
            crate::save::new_game();
            crate::travel::set_session(ng);
            crate::save::Arrive::Fresh
        }
        Goal::Load(s) => {
            crate::save::prepare_load(&s);
            crate::save::Arrive::Load(s)
        }
        Goal::Travel(_) => match carry {
            Some(c) => crate::save::Arrive::Travel(Box::new(c)),
            None => crate::save::Arrive::Fresh,
        },
        Goal::Intro(after) => {
            if let Ok(mut a) = AFTER_INTRO.lock() {
                *a = after;
            }
            crate::save::Arrive::Fresh
        }
        Goal::Restart | Goal::Title => crate::save::Arrive::Fresh,
    };

    // ---- the new map
    let player = crate::make_player(&level, params);
    world.insert_resource(player);
    world.insert_resource(crate::LevelRes(level));
    crate::sound::relevel(world);
    GEN.fetch_add(1, Ordering::Relaxed);
    world.run_schedule(LevelSetup);
    crate::save::begin_level(arrive);
    world.resource_mut::<Time<Virtual>>().unpause();
    println!("loader: {map} built");
}

/// Module-level state of the map being left (queues, per-map tables, open windows). What travels with the player
/// is put back afterwards from the captured save.
fn reset_modules() {
    crate::abilities::reset_level();
    crate::shake::reset_level();
    crate::particles::reset_level();
    crate::projgen::reset_level();
    crate::missions::reset_level();
    crate::ai::reset_level();
    crate::reactions::reset_level();
    crate::alarms::reset_level();
    crate::classic_hud::reset_level();
    crate::computers::reset_level();
    crate::consys::reset_level();
    crate::conui::reset_level();
    crate::critters::reset_level();
    crate::destruct::reset_level();
    crate::dynlight::reset_level();
    crate::fish::reset_level();
    crate::fixtures::reset_level();
    crate::grenades::reset_level();
    crate::inventory_screen::reset_level();
    crate::keys::reset_level();
    crate::menus::reset_level();
    crate::movers::reset_level();
    crate::persona::reset_level();
    crate::security::reset_level();
    crate::spawn::reset_level();
    crate::trash::reset_level();
    crate::triggers::reset_level();
    crate::cinematic::reset_level();
    crate::scriptworld::reset_level();
    crate::impacts::reset_level();
}

/// Resources the app creates once with defaults (main.rs); the map-building systems re-insert the rest.
fn reset_resources(world: &mut World) {
    world.insert_resource(crate::inventory_screen::InvScreen::default());
    world.insert_resource(crate::destruct::ItemThrows::default());
    world.insert_resource(crate::consys::ConSetup::default());
    world.insert_resource(crate::triggers::Triggers::default());
    world.insert_resource(crate::logic::LogicActors::default());
    world.insert_resource(crate::particles::Particles::default());
    world.insert_resource(crate::gpuanim::GpuAnim::default());
    world.insert_resource(crate::swim::Swim::default());
    world.insert_resource(crate::alarms::Alarms::default());
    world.insert_resource(crate::critters::Critters::default());
    world.insert_resource(crate::spawn::Spawned::default());
    world.insert_resource(crate::fixtures::Fixtures::default());
    world.insert_resource(crate::spawn::Generators::default());
    world.insert_resource(crate::trash::TrashState::default());
    world.insert_resource(crate::security::Security::default());
    world.insert_resource(crate::menus::Menus::default());
    world.insert_resource(crate::computers::Computers::default());
    world.insert_resource(crate::splash::Splashes::default());
    world.insert_resource(crate::fish::FishState::default());
    world.insert_resource(crate::weapons::ScriptedInput::default());
    world.remove_resource::<crate::shadows::ShadowAssets>();
}

// ------------------------------------------------------------------------------------------------ loading screen

#[derive(Component)]
pub struct LoadingScreen;

/// The original's loading message: the last frame stays up with "LOADING" in the middle (Console.LoadingMessage,
/// Canvas.LargeFont, Style 3 = translucent, colour (0, 0, 255)).
#[allow(clippy::too_many_arguments)]
pub fn loading_screen_system(
    mut commands: Commands,
    q: Query<Entity, With<LoadingScreen>>,
    mut assets: Option<ResMut<crate::weapons::RuntimeAssets>>,
    mut images: ResMut<Assets<Image>>,
    mut layers: ResMut<Assets<crate::classic_hud::HudLayer>>,
    windows: Query<&Window>,
    mut shown: Local<bool>,
    mut font: Local<Option<Option<std::sync::Arc<crate::uicanvas::UiFont>>>>,
) {
    let want = busy();
    if want == *shown {
        return;
    }
    *shown = want;
    for e in &q {
        commands.entity(e).despawn();
    }
    if !want {
        return;
    }
    let Some(assets) = assets.as_mut() else { return };
    let Some(win) = windows.iter().next() else { return };
    if font.is_none() {
        let f = crate::classic_hud::load_font(assets, "Engine.LargeFont")
            .or_else(|| crate::classic_hud::load_font(assets, "FontMenuTitle"))
            .map(std::sync::Arc::new);
        *font = Some(f);
    }
    let Some(Some(f)) = font.as_ref() else { return };
    let msg = "LOADING";
    let s = crate::hud::ui_scale(win).max(1.0) as i32;
    let (tw, th) = (f.width(msg) + 2, f.height + 2);
    let mut c = crate::uicanvas::Canvas::new(tw, th, s);
    c.text_styled(f, 0, 0, tw, th, crate::uicanvas::HAlign::Center, crate::uicanvas::VAlign::Center, [0, 0, 255], msg, crate::uicanvas::Style::Translucent);
    let img = images.add(Image::new(
        bevy::render::render_resource::Extent3d { width: (tw * s) as u32, height: (th * s) as u32, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        c.to_rgba8(),
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
    ));
    let (lw, lh) = ((tw * s) as f32 / win.scale_factor(), (th * s) as f32 / win.scale_factor());
    commands.spawn((
        LoadingScreen,
        MaterialNode(layers.add(crate::classic_hud::HudLayer::new(img))),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(((win.width() - lw) / 2.0).round()),
            top: Val::Px(((win.height() - lh) / 2.0).round()),
            width: Val::Px(lw),
            height: Val::Px(lh),
            ..default()
        },
        GlobalZIndex(1000),
    ));
}

/// Developer aid: `DXVIEW_GOTO="secs;map;goal|secs;map;goal..."` changes map after that many seconds of real time
/// since the previous step (goal: new / intro / travel / restart / title), and `DXVIEW_GOTO_SHOT=<prefix>` writes
/// a screenshot <prefix><n>.png 3 s after each arrival.
fn dev_goto(world: &mut World) {
    use std::sync::atomic::AtomicUsize;
    // DXVIEW_TIMED_SHOTS="prefix;t1,t2,...": screenshots <prefix><t>.png at those real-time seconds.
    if let Ok(v) = std::env::var("DXVIEW_TIMED_SHOTS") {
        static T0: Mutex<Option<std::time::Instant>> = Mutex::new(None);
        static DONE: AtomicUsize = AtomicUsize::new(0);
        let t0 = *T0.lock().unwrap().get_or_insert_with(std::time::Instant::now);
        if let Some((prefix, list)) = v.split_once(';') {
            let times: Vec<f32> = list.split(',').filter_map(|x| x.parse().ok()).collect();
            let d = DONE.load(Ordering::Relaxed);
            if let Some(&t) = times.get(d) {
                if t0.elapsed().as_secs_f32() >= t {
                    DONE.store(d + 1, Ordering::Relaxed);
                    world
                        .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                        .observe(bevy::render::view::screenshot::save_to_disk(format!("{prefix}{t}.png")));
                }
            }
        }
    }
    static STEP: AtomicUsize = AtomicUsize::new(0);
    static SINCE: Mutex<Option<std::time::Instant>> = Mutex::new(None);
    static SHOT_AT: Mutex<Option<(std::time::Instant, usize)>> = Mutex::new(None);
    let Ok(v) = std::env::var("DXVIEW_GOTO") else { return };
    let now = std::time::Instant::now();
    let since = *SINCE.lock().unwrap().get_or_insert(now);
    if let Ok(prefix) = std::env::var("DXVIEW_GOTO_SHOT") {
        let due = SHOT_AT.lock().ok().and_then(|s| *s);
        if let Some((t, n)) = due {
            if now >= t {
                *SHOT_AT.lock().unwrap() = None;
                let path = format!("{prefix}{n}.png");
                world
                    .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                    .observe(bevy::render::view::screenshot::save_to_disk(path));
            }
        }
    }
    if busy() {
        return;
    }
    let steps: Vec<&str> = v.split('|').collect();
    let i = STEP.load(Ordering::Relaxed);
    let Some(step) = steps.get(i) else { return };
    let p: Vec<&str> = step.split(';').collect();
    let secs: f32 = p.first().and_then(|x| x.parse().ok()).unwrap_or(5.0);
    if now.duration_since(since).as_secs_f32() < secs {
        return;
    }
    STEP.store(i + 1, Ordering::Relaxed);
    *SINCE.lock().unwrap() = Some(now);
    let map = p.get(1).copied().unwrap_or("01_NYC_UNATCOIsland.dx");
    // "secs;<save file>;save" / "secs;<save file>;load": save or load a game there.
    match p.get(2).copied() {
        Some("event") => {
            world.resource_mut::<crate::devices::GameEvents>().0.push((map.to_string(), true));
            *SHOT_AT.lock().unwrap() = Some((now + std::time::Duration::from_secs_f32(
                std::env::var("DXVIEW_GOTO_SHOT_DELAY").ok().and_then(|d| d.parse().ok()).unwrap_or(6.0f32)), i + 1));
            return;
        }
        // "secs;;throwbody": the nearest living pawn dies on the spot and is thrown from the player's hands as a
        // carried body would be (body physics tests; DXVIEW_BODYLOG prints its path).
        Some("throwbody") => {
            let ploc = world.resource::<crate::Player>().body.location;
            let mut q = world.query::<(Entity, &crate::ai::Npc, &crate::actors::Deco)>();
            let best = q
                .iter(world)
                .filter(|(_, n, _)| n.state != crate::ai::NpcState::Dead)
                .min_by(|a, b| a.2.body.location.distance(ploc).total_cmp(&b.2.body.location.distance(ploc)))
                .map(|(e, n, d)| (e, n.blocker, d.name.clone()));
            if let Some((e, blocker, name)) = best {
                if let Some(mut n) = world.get_mut::<crate::ai::Npc>(e) {
                    n.state = crate::ai::NpcState::Dead;
                    n.carried = true;
                }
                if blocker != usize::MAX {
                    let mut b = world.resource_mut::<crate::actors::Blockers>();
                    b.list[blocker].radius = 0.0;
                    b.list[blocker].height = 0.0;
                }
                let vel = glam::DVec3::new(300.0, 0.0, 220.0);
                world.resource_mut::<crate::weapons::CorpseDrops>().0.push((e, ploc + glam::DVec3::new(15.0, 0.0, 40.0), vel));
                *crate::actors::BODY_LOG.lock().unwrap() = Some(name.clone());
                println!("GOTO threw {name}");
            }
            return;
        }
        // "secs;x,y,z,nx,ny,nz;place": the player sticks a LAM to the wall there (grenade tests).
        Some("place") => {
            let v: Vec<f64> = map.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if v.len() == 6 {
                let n = glam::DVec3::new(v[3], v[4], v[5]);
                world.resource_mut::<crate::grenades::GrenadeThrows>().0.push((
                    "LAM".into(),
                    glam::DVec3::new(v[0], v[1], v[2]),
                    -n,
                    None,
                    Some(n),
                ));
                println!("GOTO placed LAM at {v:?}");
            }
            return;
        }
        // "secs;<FlagName>;flag": set a game flag (mission script tests).
        Some("flag") => {
            world.resource_mut::<crate::ai::GameFlags>().0.set_bool(map, true);
            println!("GOTO flag {map}");
            return;
        }
        Some("save") => {
            crate::save::request(PathBuf::from(map), "Goto test");
            return;
        }
        Some("load") => {
            let root = world.resource::<crate::pickups::GameRoot>().0.clone();
            crate::save::load(&root, std::path::Path::new(map));
            *SHOT_AT.lock().unwrap() = Some((now + std::time::Duration::from_secs_f32(
                std::env::var("DXVIEW_GOTO_SHOT_DELAY").ok().and_then(|d| d.parse().ok()).unwrap_or(6.0f32)), i + 1));
            return;
        }
        _ => {}
    }
    let goal = match p.get(2).copied().unwrap_or("new") {
        "intro" => Goal::Intro(Some((NewGame::default(), "01_NYC_UNATCOIsland.dx".into()))),
        "travel" => Goal::Travel(None),
        "restart" => Goal::Restart,
        "title" => Goal::Title,
        _ => Goal::NewGame(NewGame::default()),
    };
    request(map, goal);
    let delay = std::env::var("DXVIEW_GOTO_SHOT_DELAY").ok().and_then(|d| d.parse().ok()).unwrap_or(6.0f32);
    *SHOT_AT.lock().unwrap() = Some((now + std::time::Duration::from_secs_f32(delay), i + 1));
}
