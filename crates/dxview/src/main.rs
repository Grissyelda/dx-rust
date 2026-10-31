//! dxview: free-fly viewer for Deus Ex levels, reading the user's own game install at runtime.
use bevy::{
    app::AppExit,
    asset::RenderAssetUsages,
    camera::{visibility::RenderLayers, Exposure},
    core_pipeline::tonemapping::Tonemapping,
    image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll},
    mesh::{Indices, PrimitiveTopology},
    pbr::Lightmap,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    render::view::screenshot::{save_to_disk, Screenshot},
    window::{CursorGrabMode, CursorOptions},
};
mod abilities;
mod weather;
mod shadowlight;
mod glow;
mod vision;
mod shafts;
mod postfx;
mod aimmode;
mod fx;
mod elements;
mod mantle;
mod thirdperson;
mod atmosphere;
mod skyline;
mod clouds;
mod audiofx;
mod anim_tex;
mod actors;
mod gpuanim;
mod loader;
mod cinematic;
mod ai;
mod alarms;
mod carry;
mod charged;
mod classic_hud;
mod consys;
mod keys;
mod logic;
mod classtree;
mod pawnclass;
mod scriptworld;
mod missions;
mod credits;
mod projgen;
mod fogball;
mod augdisplay;
mod shake;
mod particles;
mod eggs;
mod discord;
mod augcan;
mod shadows;
mod conui;
mod coronas;
mod critters;
mod fish;
mod splash;
mod security;
mod computers;
mod menuui;
mod settings;
mod interp;
mod travel;
mod save;
mod menus;
mod mirror;
mod reactions;
mod water;
mod damage;
mod death;
mod destruct;
mod devices;
mod drug;
mod dynlight;
mod effects;
mod impacts;
mod visattach;
mod hdtp;
mod uscript;
mod augmentique;
mod perks;
mod outfit_view;
mod electricity;
mod explosion;
mod fixtures;
mod frobui;
mod grenades;
mod hud;
mod info;
mod inventory_screen;
mod keybinds;
mod gamepad;
mod augwheel;
mod dualsense;
mod haptics;
mod snap;
mod prompts;
mod winicon;
mod glyphs;
mod mission;
mod movers;
mod persona;
mod pickups;
mod prof;
mod projectiles;
mod sound;
mod spawn;
mod spawnmenu;
mod stealth;
mod remaster;
mod swim;
mod trash;
mod triggers;
mod smartai;
mod achievements;
mod diegetic;
mod skeletal;
mod ragdoll;
mod gore;
mod bodyfx;
mod alertui;
mod uicanvas;
mod weapons;
mod wisp;

use actors::{Blockers, Deco, World};
use dx_level::{bevy_to_ue, ue_to_bevy, Level, SurfaceClass};
use glam::DVec3;
use imsim_core::movement::{Character, MoveParams, Physics};
use imsim_core::view::ViewBob;
use std::path::PathBuf;

#[derive(Resource)]
pub struct LevelRes(pub Level);

/// Materials whose UVs scroll over time (auto-panning surfaces, water).
#[derive(Resource, Default)]
struct UvScroll(Vec<(Handle<StandardMaterial>, Vec2, bool)>, Vec<(Handle<shadowlight::BspMaterial>, Vec2)>);

#[derive(Component)]
pub struct FlyCam {
    pub(crate) yaw: f32,
    pub(crate) pitch: f32,
    speed: f32,
}

#[derive(Component)]
struct Overlay;

#[derive(Component)]
struct HudText;

/// The player body: Unreal-style walking/falling against the level BSP.
#[derive(Resource)]
pub struct Player {
    pub body: Character,
    pub params: MoveParams,
    pub eye_height: f64,
    view: ViewBob,
    pub noclip: bool,
    /// Ducking (C held, or forced because there's no room to stand).
    pub crouched: bool,
    pub force_duck: bool,
    /// Toggle Crouch (DeusExPlayer.bCrouchOn / bWasCrouchOn / lastbDuck).
    pub crouch_on: bool,
    pub was_crouch_on: bool,
    /// bDuck as UnrealScript keeps it: an input button set by key press / release events only, so the 1 that
    /// HandleWalking writes while crouch is latched stays until the key's next release.
    pub duck_btn: bool,
    pub last_duck: bool,
    /// Standing collision height and BaseEyeHeight (JCDentonMale defaults).
    pub stand_height: f64,
    pub stand_eye: f64,
    /// curLeanDist: how far we're leaning sideways (+ right), up to 40 units.
    pub lean: f64,
    /// Enhancement (Smooth Leaning, peek assist): extra sideways camera offset past where the body could slide.
    pub peek: f64,
    /// Debug: PgUp lets frobbing open locked doors (until keys and lockpicks exist).
    pub unlock_all: bool,
    /// In the PlayerSwimming state (2 feet high, 3D swim controls). Physics can be Falling while bobbing at the surface.
    pub swimming: bool,
    /// PlayerSwimming's 0.7 s timer after surfacing over deep water; back to walking when it runs out.
    pub swim_out: Option<f64>,
}

/// Player tuning from the game's own class defaults (JCDentonMale), falling back to known values.
pub fn player_params(root: &std::path::Path) -> (MoveParams, f64, f64) {
    use unreal_pkg::props::{find, Value};
    let mut db = unreal_pkg::classes::ClassDb::open(&root.join("System"));
    let d = db.defaults("JCDentonMale");
    let z = db.defaults("ZoneInfo");
    let f = |list: &[unreal_pkg::props::Property], n: &str, dflt: f64| match find(list, n) {
        Some(Value::Float(v)) => *v as f64,
        _ => dflt,
    };
    let mut p = MoveParams::default();
    p.radius = f(&d, "CollisionRadius", p.radius);
    p.height = f(&d, "CollisionHeight", p.height);
    p.ground_speed = f(&d, "GroundSpeed", p.ground_speed);
    p.accel_rate = f(&d, "AccelRate", p.accel_rate);
    p.jump_z = f(&d, "JumpZ", p.jump_z);
    p.air_control = f(&d, "AirControl", p.air_control);
    p.max_step_height = f(&d, "MaxStepHeight", p.max_step_height);
    p.mass = f(&d, "Mass", p.mass);
    p.ground_friction = f(&z, "ZoneGroundFriction", p.ground_friction);
    p.terminal_velocity = f(&z, "ZoneTerminalVelocity", p.terminal_velocity);
    if let Some(Value::Vector(g)) = find(&z, "ZoneGravity") {
        p.gravity = DVec3::new(g[0] as f64, g[1] as f64, g[2] as f64);
    }
    // Lay D Denton's eye is 2 units lower (JCDentonMale.TravelPostAccept).
    let eye = f(&d, "BaseEyeHeight", 40.0) - if travel::female() { 2.0 } else { 0.0 };
    (p, eye, f(&d, "Bob", 0.016))
}

#[derive(Component)]
pub struct SkyCam;

/// Developer aid: `DXVIEW_SHOT="out.png;x;y;z;yaw_deg;pitch_deg"` renders one frame and exits.
#[derive(Resource, Clone)]
struct ShotCfg {
    /// (path, position, yaw, pitch)
    views: Vec<(String, Vec3, f32, f32)>,
}

fn shot_cfg() -> Option<ShotCfg> {
    let v = std::env::var("DXVIEW_SHOT").ok()?;
    let mut views = Vec::new();
    for one in v.split('|') {
        let p: Vec<&str> = one.split(';').collect();
        let f = |i: usize| p.get(i).and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
        views.push((
            p[0].to_string(),
            Vec3::new(f(1), f(2), f(3)),
            f(4).to_radians(),
            f(5).to_radians(),
        ));
    }
    Some(ShotCfg { views })
}

/// Developer aid: `DXVIEW_WALKTEST="ue_x;ue_y;ue_z;yaw_deg;seconds;out.png"` places the player (Unreal
/// coordinates, Unreal yaw), holds W for the given simulated time at a fixed 60 Hz, logs what moved,
/// saves a screenshot and exits. Used to test pushing and collision without a person at the keyboard.
#[derive(Resource)]
pub struct WalkTest {
    start: DVec3,
    yaw_deg: f32,
    secs: f64,
    out: String,
    t: f64,
    frame: u32,
    /// Frob (press E) when the walk ends, then let things run this many extra seconds before the screenshot.
    frob_then: f64,
    frobbed: bool,
    pitch_deg: f32,
    /// Hold crouch for the whole test (9th field "crouch").
    crouch: bool,
}

fn walk_test_cfg() -> Option<WalkTest> {
    let v = std::env::var("DXVIEW_WALKTEST").ok()?;
    let p: Vec<&str> = v.split(';').collect();
    let f = |i: usize| p.get(i).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
    Some(WalkTest {
        start: DVec3::new(f(0), f(1), f(2)),
        yaw_deg: f(3) as f32,
        secs: f(4),
        out: p.get(5).unwrap_or(&"walktest.png").to_string(),
        t: 0.0,
        frame: 0,
        frob_then: f(6),
        frobbed: false,
        pitch_deg: p.get(7).and_then(|x| x.parse().ok()).unwrap_or(-20.0),
        crouch: p.get(8) == Some(&"crouch"),
    })
}

#[allow(clippy::too_many_arguments)]
fn walk_test_finish(
    mut commands: Commands,
    wt: Option<ResMut<WalkTest>>,
    mut exit: MessageWriter<AppExit>,
    decos: Query<&Deco>,
    player: Res<Player>,
    inv: Res<pickups::PlayerInventory>,
    ph: Res<ai::PlayerHealth>,
    npcs: Query<&ai::Npc>,
    level: Res<LevelRes>,
) {
    let Some(mut wt) = wt else { return };
    if wt.t < wt.secs + wt.frob_then {
        return;
    }
    wt.frame += 1;
    if wt.frame == 1 {
        let b = &player.body;
        println!(
            "WALKTEST player end {:.1} {:.1} {:.1} vel {:.1} health {:.0}",
            b.location.x,
            b.location.y,
            b.location.z,
            b.velocity.length(),
            ph.hp
        );
        for n in &npcs {
            if n.state != ai::NpcState::Idle {
                println!(
                    "WALKTEST npc {} ({}) state {:?} target {:?} weapon {:?}",
                    level.0.actors[n.actor].name,
                    n.alliance,
                    n.state,
                    n.target,
                    n.weapon.as_ref().map(|w| (&w.class, &w.third_mesh))
                );
            }
        }
        println!(
            "WALKTEST inventory: {:?} credits {}",
            inv.0
                .items
                .iter()
                .map(|i| (i.def.name.clone(), i.count))
                .collect::<Vec<_>>(),
            inv.0.credits
        );
        for d in &decos {
            let moved = (d.body.location - d.start).length();
            if moved > 0.5 {
                println!(
                    "WALKTEST moved {:.1} units: {} now at {:.1} {:.1} {:.1} yaw {}",
                    moved,
                    d.name,
                    d.body.location.x,
                    d.body.location.y,
                    d.body.location.z,
                    d.rotation[1]
                );
            }
        }
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(wt.out.clone()));
    }
    if wt.frame == 30 {
        exit.write(AppExit::Success);
    }
}

/// Developer aid: `DXVIEW_DOORTEST="MoverName;ue_x;ue_y;ue_z;ue_yaw_deg;pitch_deg;prefix"` puts a fly camera at the
/// given spot, screenshots, triggers the mover (ignoring locks), waits for it to finish, screenshots again, exits.
#[derive(Resource)]
pub struct DoorTest {
    mover: String,
    cam: DVec3,
    yaw: f32,
    pitch: f32,
    prefix: String,
    frame: u32,
    after: u32,
}

fn door_test_cfg() -> Option<DoorTest> {
    let v = std::env::var("DXVIEW_DOORTEST").ok()?;
    let p: Vec<&str> = v.split(';').collect();
    let f = |i: usize| p.get(i).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
    Some(DoorTest {
        mover: p[0].to_string(),
        cam: DVec3::new(f(1), f(2), f(3)),
        yaw: f(4) as f32,
        pitch: f(5) as f32,
        prefix: p.get(6).unwrap_or(&"doortest").to_string(),
        frame: 0,
        after: 0,
    })
}

fn door_test(
    mut commands: Commands,
    dt: Option<ResMut<DoorTest>>,
    level: Res<LevelRes>,
    mut mv: ResMut<movers::Movers>,
    mut player: ResMut<Player>,
    mut cam: Single<(&mut Transform, &mut FlyCam), Without<SkyCam>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(mut t) = dt else { return };
    t.frame += 1;
    player.noclip = true;
    let c = ue_to_bevy([t.cam.x as f32, t.cam.y as f32, t.cam.z as f32]);
    cam.0.translation = Vec3::from(c);
    cam.1.yaw = (-t.yaw - 90.0).to_radians();
    cam.1.pitch = t.pitch.to_radians();
    let Some(i) = level.0.movers.iter().position(|m| m.name == t.mover) else {
        println!("DOORTEST no mover named {}", t.mover);
        exit.write(AppExit::Success);
        return;
    };
    if t.frame == 150 {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(format!("{}_before.png", t.prefix)));
    }
    if t.frame == 160 {
        let info = &level.0.movers[i];
        movers::trigger(info, &mut mv.rt[i]);
        let tag = info.tag.clone();
        if !tag.is_empty() && tag != "DeusExMover" {
            for (j, other) in level.0.movers.iter().enumerate() {
                if j != i && other.tag == tag {
                    movers::trigger(other, &mut mv.rt[j]);
                }
            }
        }
        println!(
            "DOORTEST triggered {} ({}), key {} -> moving",
            info.name, info.tag, mv.rt[i].key_num
        );
    }
    if t.frame > 160 && !mv.rt[i].interpolating && t.after == 0 {
        t.after = t.frame;
        let r = &mv.rt[i];
        println!(
            "DOORTEST finished at {:.1} {:.1} {:.1} rot {:?}",
            r.location.x, r.location.y, r.location.z, r.rotation
        );
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(format!("{}_after.png", t.prefix)));
    }
    if t.after > 0 && t.frame > t.after + 20 {
        exit.write(AppExit::Success);
    }
}

fn shot_system(
    mut commands: Commands,
    cfg: Option<Res<ShotCfg>>,
    mut frame: Local<u32>,
    mut exit: MessageWriter<AppExit>,
    mut cam: Single<(&mut Transform, &mut FlyCam), Without<SkyCam>>,
) {
    let Some(cfg) = cfg else { return };
    *frame += 1;
    let n = cfg.views.len() as u32;
    let slot = (*frame).saturating_sub(40) / 40;
    let phase = (*frame).saturating_sub(40) % 40;
    if *frame >= 40 && slot < n {
        let (path, pos, yaw, pitch) = &cfg.views[slot as usize];
        cam.0.translation = *pos;
        cam.1.yaw = *yaw;
        cam.1.pitch = *pitch;
        cam.0.rotation = Quat::from_rotation_y(*yaw) * Quat::from_rotation_x(*pitch);
        if phase == 30 {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path.clone()));
        }
    }
    if *frame > 40 + 40 * n + 20 {
        exit.write(AppExit::Success);
    }
}

/// Developer aid: `DXVIEW_SHOT_TIMES="dir;t1,t2,..."` saves a screenshot at each of those seconds after start
/// (game time) without touching the camera - for cut scenes - and exits after the last.
fn timed_shot_system(
    mut commands: Commands,
    time: Res<Time>,
    mut done: Local<usize>,
    mut cfg: Local<Option<Option<(String, Vec<f32>)>>>,
    mut exit: MessageWriter<AppExit>,
) {
    let cfg = cfg.get_or_insert_with(|| {
        let v = std::env::var("DXVIEW_SHOT_TIMES").ok()?;
        let (dir, ts) = v.split_once(';')?;
        Some((dir.to_string(), ts.split(',').filter_map(|t| t.trim().parse().ok()).collect()))
    });
    let Some((dir, ts)) = cfg.as_ref() else { return };
    let t = time.elapsed_secs();
    if let Some(&want) = ts.get(*done) {
        if t >= want {
            let path = format!("{dir}/t{:05.1}.png", want);
            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
            *done += 1;
        }
    } else if t > ts.last().copied().unwrap_or(0.0) + 1.0 {
        exit.write(AppExit::Success);
    }
}

/// A crash leaves its message and where it happened in crash.log beside the exe (and on stderr). Installed as a
/// startup system so it wraps the hook Bevy's plugins put in place.
fn install_crash_log() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = format!("dxview crashed: {info}\n{}\n", std::backtrace::Backtrace::force_capture());
        eprintln!("{msg}");
        if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
            let _ = std::fs::write(dir.join("crash.log"), &msg);
        }
        prev(info);
    }));
}

/// The player resource for a freshly loaded map: standing at its PlayerStart, settled onto the floor.
pub fn make_player(level: &Level, (params, eye_height, bob): (MoveParams, f64, f64)) -> Player {
    let start = level
        .player_start
        .as_ref()
        .map(|p| bevy_to_ue(p.position))
        .unwrap_or([0.0; 3]);
    let mut body = Character::new(DVec3::new(
        start[0] as f64,
        start[1] as f64,
        start[2] as f64,
    ));
    body.settle(
        &World {
            level,
            blockers: &level.blockers,
            ignore: None,
            pushable: None,
            movers: None,
        },
        &params,
    );
    println!("player: {:?} at {:?}", body.physics, body.location);
    Player {
        body,
        stand_height: params.height,
        stand_eye: eye_height,
        crouched: false,
        force_duck: false,
        crouch_on: false,
        was_crouch_on: false,
        duck_btn: false,
        last_duck: false,
        lean: 0.0,
        peek: 0.0,
        params,
        eye_height,
        view: ViewBob::new(bob, eye_height),
        noclip: shot_cfg().is_some(),
        unlock_all: false,
        swimming: false,
        swim_out: None,
    }
}

fn main() {
    prof::init();
    // DXT1 textures go to the GPU compressed (desktop GPUs and the Steam Deck take BC formats; checked again once
    // the render device exists). DXVIEW_NO_BC1 decodes them as before.
    dx_level::set_bc1_supported(std::env::var_os("DXVIEW_NO_BC1").is_none());
    let args: Vec<String> = std::env::args().collect();
    // The game folder: the first argument, else the launcher's choice (launcher.txt beside the exe), else the default.
    let root = args
        .get(1)
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .or_else(|| {
            let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
            std::fs::read_to_string(dir.join("launcher.txt"))
                .ok()?
                .lines()
                .find_map(|l| l.strip_prefix("GamePath=").map(|v| v.trim().to_string()))
                .filter(|v| !v.is_empty())
        })
        // Else the usual Steam / GOG / Heroic places (Steam Deck included).
        .or_else(|| unreal_pkg::gamedir::detect().map(|p| p.display().to_string()))
        .unwrap_or_else(|| r"C:\Games\Deus Ex".to_string());
    // Mods (unreal_pkg::mods): the launcher's Mods page writes Mod.<id>= / ModOn.<id>= lines in launcher.txt (Lay D
    // Denton's older LDDPPath= still counts). Overlays (Lay D Denton) are laid over the game folder, addons searched
    // after it, HD texture packs used texture by texture. DXVIEW_MOD (';'-separated folders) adds overlays for tests.
    let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())).unwrap_or_default();
    let mod_settings = unreal_pkg::mods::from_lines(&unreal_pkg::mods::read_launcher_txt(&exe_dir.join("launcher.txt")));
    let mut enabled = unreal_pkg::mods::enabled(&mod_settings);
    // DXVIEW_MODS="id=folder;id=folder": mods for test runs.
    if let Ok(v) = std::env::var("DXVIEW_MODS") {
        for part in v.split(';') {
            if let Some((id, dir)) = part.split_once('=') {
                if let Some(m) = unreal_pkg::mods::def(id.trim()) {
                    if let Some(p) = unreal_pkg::mods::locate(m, std::path::Path::new(dir.trim())) {
                        enabled.retain(|(e, _)| e.id != m.id);
                        enabled.push((m, p));
                    }
                }
            }
        }
    }
    let mut mods: Vec<PathBuf> = enabled
        .iter()
        .filter(|(m, _)| m.kind == unreal_pkg::mods::ModKind::Overlay)
        .map(|(_, p)| p.clone())
        .collect();
    if let Ok(v) = std::env::var("DXVIEW_MOD") {
        mods.extend(v.split(';').filter(|x| !x.trim().is_empty()).map(|x| PathBuf::from(x.trim())));
    }
    // DDS override packs (Helios) installed the way kentie's renderer reads them, in the game's own Textures
    // folder: used without setting anything up in the launcher.
    if !enabled.iter().any(|(m, _)| m.id == "helios") {
        if let Some(m) = unreal_pkg::mods::def("helios") {
            if let Some(p) = unreal_pkg::mods::locate(m, &PathBuf::from(&root).join("Textures")) {
                enabled.push((m, p));
            }
        }
    }
    if !enabled.is_empty() || !mods.is_empty() {
        println!("dxview: mods {:?}", enabled.iter().map(|(m, p)| format!("{}={}", m.id, p.display())).collect::<Vec<_>>());
    }
    unreal_pkg::paths::set(&PathBuf::from(&root), mods);
    unreal_pkg::mods::activate(&enabled);
    unreal_pkg::paths::set_hd_textures_on(settings::get("HDTextures") > 0.5);
    hdtp::apply_dds_filter();
    travel::init_lddp();
    hdtp::init(&PathBuf::from(&root));
    hdtp::build_layer();
    augmentique::init(&PathBuf::from(&root));
    perks::init(&PathBuf::from(&root));
    visattach::build_layer(&mut unreal_pkg::classes::ClassDb::open(&PathBuf::from(&root).join("System")));
    // `--load <save>`: the saved game's map, put back once it's built (save.rs).
    let loaded_map = save::startup(&args);
    let map = loaded_map.or_else(|| args.get(2).filter(|a| !a.starts_with("--")).cloned())
        .unwrap_or_else(|| {
            // No map named: boot to the logo map and its main menu, like the original (DXVIEW_SKIP_MENU=1 goes
            // straight to Liberty Island as before).
            if std::env::var("DXVIEW_SKIP_MENU").is_ok() || travel::session().is_some() {
                "01_NYC_UNATCOIsland.dx".to_string()
            } else {
                "DX.dx".to_string()
            }
        });
    // The chosen face on Paul and JC's stand-ins (DeusExPlayer.UpdatePlayerSkin).
    dx_level::actors::set_overrides(travel::skin_overrides(&map));
    hdtp::build_layer();
    visattach::apply();
    augmentique::build_layer();
    println!("dxview: loading {map} from {root} ...");
    let t0 = std::time::Instant::now();
    let level = match dx_level::load_level(&PathBuf::from(&root), &map) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("ERROR: {e}\nUsage: dxview.exe [\"C:\\Games\\Deus Ex\"] [map.dx]");
            eprintln!("Press Enter to close.");
            let _ = std::io::stdin().read_line(&mut String::new());
            std::process::exit(1);
        }
    };
    prof::hitch_note("first level load", t0.elapsed().as_secs_f64() * 1000.0);
    println!(
        "loaded in {:.1}s: {} triangles, {} textures ({} missing), {} backdrop / {} invisible faces skipped",
        t0.elapsed().as_secs_f32(),
        level.stats.triangles,
        level.stats.textures,
        level.stats.missing_textures,
        level.stats.skipped_backdrop,
        level.stats.skipped_invisible
    );
    App::new()
        .add_plugins(DefaultPlugins.set(bevy::log::LogPlugin {
            custom_layer: prof::span_layer,
            ..default()
        }).set(WindowPlugin {
            primary_window: Some(Window {
                title: "DX RUST".into(),
                // Settings > Display: the saved resolution and display mode (Windowed by default).
                resolution: menus::RESOLUTIONS
                    .get(settings::get("Resolution").max(-1.0) as isize as usize)
                    .copied()
                    .unwrap_or((1600, 900))
                    .into(),
                mode: menus::window_mode(settings::get("DisplayMode")),
                present_mode: menus::present_mode(settings::get("VSync")),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.05, 0.07, 0.12)))
        // A command aimed at an entity that went away the same frame (a pawn destroyed by the mission script,
        // an effect that despawned itself) is logged, never a crash.
        .insert_resource(bevy::ecs::error::FallbackErrorHandler(bevy::ecs::error::warn))
        .add_plugins(prof::render_plugins())
        .add_plugins(prof::ProfPhasePlugin)
        .insert_resource(make_player(&level, player_params(&PathBuf::from(&root))))
        .insert_resource(pickups::GameRoot(PathBuf::from(&root)))
        .insert_resource(LevelRes(level))
        .add_systems(PreStartup, |mut c: Commands| {
            if let Some(cfg) = shot_cfg() {
                c.insert_resource(cfg);
            }
            if let Some(wt) = walk_test_cfg() {
                c.insert_resource(wt);
            }
            if let Some(d) = door_test_cfg() {
                c.insert_resource(d);
            }
            if let Some(f) = weapons::fire_test_cfg() {
                c.insert_resource(f);
            }
            c.insert_resource(weapons::ScriptedInput::default());
        })
        .add_systems(loader::LevelSetup, setup)
        .add_systems(Startup, (detect_bc1, loader::first_level_setup).chain())
        .add_systems(First, loader::loader_system.before(save::save_first_system))
        .add_systems(Update, loader::loading_screen_system)
        .add_systems(
            Update,
            (
                cursor_grab,
                movers::tick_movers,
                fly.after(movers::tick_movers),
                actors::deco_physics.after(fly),
                actors::animate,
                shadows::shadow_system.after(ai::npc_system),
                uv_scroll,
                overlay_update,
                shot_system.after(fly).after(overlay_update),
                (timed_shot_system, weapons::prewarm_view_models_system.after(weapons::select_system)),
                walk_test_finish,
                hud_text,
                door_test.after(fly),
                pickups::frob_system.after(fly),
                pickups::belt_ui,
                weapons::select_system,
                weapons::visattach_refit_system.after(weapons::weapon_system),
                weapons::weapon_system.after(fly),
                weapons::fov_system,
                weapons::fire_test.before(weapons::select_system),
            ),
        )
        .add_systems(
            Update,
            (
                ai::attach_npcs,
                ai::npc_system.after(fly).after(ai::attach_npcs),
                ai::player_health_ui,
                ai::npc_weapon_models
                    .after(ai::npc_system)
                    .after(actors::animate),
                ai::carcass_swap_system.after(ai::npc_system),
                destruct::deco_damage_system
                    .after(weapons::weapon_system)
                    .after(ai::npc_system),
                destruct::loose_physics,
                hud::aim_system.before(weapons::weapon_system),
                hud::crosshair_system.after(fly),
                hud::scope_overlay_system.after(weapons::weapon_system),
                weapons::fire_texture_system,
                anim_tex::anim_tex_system,
                carry::carry_system
                    .after(fly)
                    .before(pickups::frob_system)
                    .before(weapons::weapon_system),
            ),
        )
        .add_systems(
            Update,
            (
                stealth::player_light_system.after(fly),
                stealth::footstep_system.after(fly).before(ai::npc_system),
                frobui::frob_target_system.after(carry::carry_system),
                frobui::frob_draw_system.after(frobui::frob_target_system),
                projectiles::launch_system
                    .after(ai::npc_system)
                    .after(weapons::weapon_system),
                projectiles::projectile_system
                    .after(projectiles::launch_system)
                    .before(destruct::deco_damage_system),
                weapons::muzzle_flash_system
                    .after(weapons::weapon_system)
                    .after(ai::npc_weapon_models),
                destruct::shell_system
                    .after(ai::npc_weapon_models)
                    .after(weapons::weapon_system),
                coronas::corona_system.after(fly),
                electricity::emitter_system
                    .before(destruct::deco_damage_system)
                    .before(ai::npc_system),
                effects::blood_system
                    .after(weapons::weapon_system)
                    .after(ai::npc_system),
                effects::cleaner_system.after(effects::blood_system),
                impacts::impacts_system
                    .after(weapons::weapon_system)
                    .after(ai::npc_system)
                    .before(projectiles::launch_system),
                actors::env_map_system.after(fly),
                mirror::mirror_body_system.after(fly).after(outfit_view::outfit_view_system),
                skeletal::mirror_skel_system.after(mirror::mirror_body_system),
                water::water_system,
                death::death_system.after(fly).after(ai::player_health_ui),
                carry::corpse_drop_system
                    .after(weapons::weapon_system)
                    .before(actors::deco_physics),
                movers::lockpick_system.after(frobui::frob_target_system),
            ),
        )
        .add_systems(
            Update,
            (
                grenades::attach_grenades,
                grenades::grenade_system
                    .after(weapons::weapon_system)
                    .after(explosion::explosion_system),
                explosion::explosion_system
                    .after(ai::npc_system)
                    .after(weapons::weapon_system),
                explosion::fireball_system.after(explosion::explosion_system),
                explosion::mesh_fx_system.after(explosion::explosion_system),
                effects::burn_fx_system.after(ai::npc_system),
                destruct::flare_system.after(weapons::weapon_system),
                effects::gib_system.after(ai::npc_system),
                destruct::weapon_drop_system.after(ai::npc_system),
                charged::charged_system.after(fly),
                spawnmenu::spawn_menu_system
                    .before(cursor_grab)
                    .before(destruct::weapon_drop_system),
            ),
        )
        .add_systems(Startup, sound::setup_sound)
        .init_resource::<keybinds::Bindings>()
        .add_systems(Startup, (gamepad::setup, || dualsense::setup(), || haptics::setup()))
        .add_systems(Update, (augwheel::augwheel_system, dualsense::dualsense_system, gamepad::rumble_system, prompts::prompts_system, winicon::window_icon_system))
        .init_resource::<inventory_screen::InvScreen>()
        .init_resource::<destruct::ItemThrows>()
        .add_systems(
            Update,
            inventory_screen::inventory_screen_system
                .after(keybinds::bind_menu_system)
                .after(classic_hud::classic_hud_system),
        )
        .add_systems(Update, outfit_view::outfit_view_system.after(inventory_screen::inventory_screen_system))
        .add_systems(
            Update,
            abilities::abilities_system.before(dynlight::aug_light_system),
        )
        .add_systems(
            Update,
            abilities::aug_defense_system.before(projectiles::projectile_system),
        )
        .add_systems(
            PreUpdate,
            (gamepad::poll_system, keybinds::remap_system).chain().after(bevy::input::InputSystems),
        )
        .add_systems(Update, keybinds::bind_menu_system.before(cursor_grab))
        .add_systems(Last, (prof::prof_report_system, prof::scene_stats_system, prof::prof_ab_system, prof::prof_features_system, prof::mesh_owner_system, prof::camera_report_system, prof::bounds_report_system))
        .init_resource::<consys::ConSetup>()
        .init_resource::<triggers::Triggers>()
        .init_resource::<logic::LogicActors>()
        .add_systems(Update, logic::logic_system.after(devices::device_system))
        .add_systems(Update, scriptworld::scriptworld_system.after(mission::mission_system))
        .add_systems(Update, missions::mission_fx_system.after(mission::mission_system))
        .add_systems(Update, credits::credits_system)
        .add_systems(Update, projgen::projgen_system)
        .add_systems(Update, fogball::fogball_system)
        // Shadow map sizes: the real-time shadows' cube maps (4 lights x 6 faces) and the spot / moon maps.
        .insert_resource(bevy::light::PointLightShadowMap { size: 512 })
        .insert_resource(bevy::light::DirectionalLightShadowMap { size: 1024 })
        .add_systems(Update, water::ocean_system)
        .add_systems(Update, glow::glow_system)
        .add_systems(Update, (shafts::shafts_system.after(dynlight::aug_light_system), postfx::postfx_system))
        .add_systems(Update, vision::vision_system.after(augdisplay::vision_system))
        .add_systems(Update, (augdisplay::augdisplay_system, augdisplay::vision_system, augdisplay::aug_text_system))
        .init_resource::<particles::Particles>()
        .add_systems(Update, particles::particle_system.after(devices::device_system).after(fixtures::fixture_system))
        .add_systems(Update, weather::weather_system.after(fly))
        .add_systems(Update, (shadowlight::shadow_light_system.after(fly), shadowlight::tag_casters_system, shadowlight::no_transparent_shadows_system))
        .add_systems(Update, fx::fx_system.after(fly))
        .add_systems(Update, thirdperson::toggle_system)
        .add_systems(Update, audiofx::soundscape_system)
        .add_systems(Update, (atmosphere::fog_system, atmosphere::halo_system.after(fly), skyline::skyline_system, clouds::clouds_system, clouds::lightning_sky_system.after(clouds::clouds_system), wisp::wisp_system.after(fly)))
        .add_systems(Update, alertui::alert_ui_system.after(ai::npc_system))
        .add_systems(Update, achievements::achievements_system.after(ai::npc_system))
        .add_systems(First, (diegetic::restore_system, thirdperson::restore_system).chain())
        .add_systems(PostUpdate, (thirdperson::place_system, diegetic::place_system).chain().before(bevy::transform::TransformSystems::Propagate).before(bevy::camera::CameraUpdateSystems))
        // Reflections from the camera as it is drawn: after the third-person / terminal camera moves (in Update
        // the reflection was made from the eyes while the view was from the orbit: it dragged across the water).
        .add_systems(PostUpdate, mirror::mirror_system.after(diegetic::place_system).before(bevy::transform::TransformSystems::Propagate).before(bevy::camera::CameraUpdateSystems))
        .add_systems(Update, diegetic::diegetic_system.after(computers::computer_system).after(menus::menu_system))
        .add_systems(Update, (elements::shock_system.before(ai::npc_system), elements::fire_system.after(ai::npc_system)))
        .add_systems(Update, aimmode::aim_mode_system.after(hud::aim_system).before(weapons::weapon_system).before(hud::crosshair_system))
        .add_systems(Update, (eggs::matrix_system, eggs::warren_system))
        .add_systems(
            PreUpdate,
            (
                consys::capture_input_system.after(keybinds::remap_system),
                cinematic::cinematic_input_system
                    .after(keybinds::remap_system)
                    .before(consys::capture_input_system),
            ),
        )
        .add_systems(
            Update,
            cinematic::cinematic_system
                .after(fly)
                .before(consys::con_view_system)
                .before(classic_hud::classic_hud_system),
        )
        .add_systems(
            Update,
            (
                triggers::trigger_system.before(consys::consys_system),
                consys::consys_system
                    .after(fly)
                    .after(pickups::frob_system)
                    .before(ai::npc_system)
                    .before(devices::device_system),
                mission::mission_system.before(consys::consys_system),
                ai::world_system
                    .before(ai::npc_system)
                    .after(ai::attach_npcs),
                consys::con_view_system
                    .after(fly)
                    .after(death::death_system)
                    .after(consys::consys_system)
                    .before(classic_hud::classic_hud_system),
            ),
        )
        .add_plugins(bevy::ui_render::UiMaterialPlugin::<classic_hud::HudLayer>::default())
        .add_plugins(remaster::RemasterPlugin)
        .add_plugins(ragdoll::RagdollPlugin)
        .add_systems(
            Update,
            (
                bodyfx::fp_body_system.after(skeletal::mirror_skel_system),
                bodyfx::glow_system.after(bodyfx::fp_body_system),
                bodyfx::heal_label_system,
                bodyfx::wounds_system.after(ai::npc_system),
            ),
        )
        // Vertex animation posed on the GPU (gpuanim.rs).
        .add_plugins(bevy::pbr::MaterialPlugin::<gpuanim::AnimMaterial>::default())
        .add_plugins(bevy::pbr::MaterialPlugin::<mirror::MirrorMaterial>::default())
        .add_plugins(bevy::pbr::MaterialPlugin::<water::WaterMaterial>::default())
        .add_plugins(bevy::pbr::MaterialPlugin::<clouds::CloudMaterial>::default())
        .add_plugins(bevy::pbr::MaterialPlugin::<shadowlight::BspMaterial>::default())
        .add_plugins(bevy::pbr::MaterialPlugin::<skeletal::ActorMaterial>::default())
        .add_plugins(bevy::pbr::MaterialPlugin::<weather::RainMaterial>::default())
        .init_resource::<water::WaterMats>()
        .init_resource::<gpuanim::GpuAnim>()
        .add_systems(Startup, (gpuanim::setup_anim_shader, mirror::setup_mirror_shader, water::setup_water_shader, clouds::setup_cloud_shader, shadowlight::setup_bsp_shader, skeletal::setup_shader, weather::setup_rain_shader, install_crash_log))
        .add_systems(
            Update,
            (
                gpuanim::gpuanim_setup.before(actors::animate),
                skeletal::spawn_system.after(gpuanim::gpuanim_setup).before(actors::animate),
                skeletal::pose_system.after(actors::animate).before(ai::npc_weapon_models),
                gore::gore_system.after(ai::npc_system),
            ),
        )
        .add_systems(Startup, classic_hud::setup_hud_shader)
        .add_systems(loader::LevelSetup, classic_hud::setup_classic_hud)
        .add_systems(
            Update,
            classic_hud::classic_hud_system
                .after(weapons::weapon_system)
                .after(ai::player_health_ui)
                .after(fly)
                .after(swim::swim_system)
                .before(hud_text),
        )
        .add_plugins(
            bevy::core_pipeline::fullscreen_material::FullscreenMaterialPlugin::<
                dynlight::Brightness,
            >::default(),
        )
        .add_systems(Startup, dynlight::setup_gamma_shader)
        .add_systems(loader::LevelSetup, (dynlight::setup_dynlights, shadowlight::setup_shadow_lights))
        .add_systems(
            Update,
            (
                dynlight::aug_light_system.after(fly),
                dynlight::brightness_key_system,
                dynlight::dynlight_system
                    .after(dynlight::aug_light_system)
                    .after(projectiles::projectile_system)
                    .after(explosion::explosion_system)
                    .after(destruct::flare_system)
                    .after(weapons::weapon_system)
                    .after(ai::npc_weapon_models),
                dynlight::relight_system
                    .after(dynlight::dynlight_system)
                    .after(actors::deco_physics),
            ),
        )
        .insert_resource(swim::Swim::default())
        .insert_resource(alarms::Alarms::default())
        .add_systems(
            Update,
            (
                alarms::alarm_system.after(ai::npc_system),
                grenades::danger_system.before(ai::npc_system),
            ),
        )
        .add_systems(
            Update,
            (
                swim::swim_system.after(fly).before(ai::npc_system),
                swim::bubble_system,
            ),
        )
        .insert_resource(critters::Critters::default())
        .insert_resource(spawn::Spawned::default())
        .add_systems(
            Update,
            movers::mover_damage_system
                .after(explosion::explosion_system)
                .after(weapons::weapon_system)
                .after(ai::npc_system)
                .after(projectiles::projectile_system)
                .before(devices::device_system)
                .before(destruct::deco_damage_system),
        )
        .insert_resource(fixtures::Fixtures::default())
        .add_systems(
            Update,
            fixtures::fixture_system
                .after(pickups::frob_system)
                .before(actors::animate),
        )
        .insert_resource(spawn::Generators::default())
        .add_systems(
            Update,
            spawn::generator_system
                .before(spawn::spawn_system)
                .after(fly),
        )
        .insert_resource(trash::TrashState::default())
        .add_systems(
            Update,
            trash::trash_system.after(destruct::deco_damage_system),
        )
        .add_systems(
            Update,
            spawn::spawn_system
                .before(ai::npc_system)
                .before(actors::animate),
        )
        .insert_resource(security::Security::default())
        .add_systems(
            Update,
            // Not after device_system or explosion_system: both run after npc_system, and cameras and turrets
            // must run before the AI. A hack or an EMP blast reaches them a frame later.
            security::security_system
                .before(ai::npc_system)
                .before(actors::deco_physics)
                .before(destruct::deco_damage_system),
        )
        .add_systems(Update, classic_hud::ui_gamma_system)
        .add_systems(First, save::save_first_system)
        .add_systems(Last, save::save_last_system)
        .init_resource::<menus::Menus>()
        .add_systems(
            Update,
            (
                menus::menu_system
                    .after(fly)
                    .after(computers::computer_system)
                    .after(inventory_screen::inventory_screen_system)
                    .after(keybinds::bind_menu_system),
                interp::rotating_system,
            ),
        )
        .init_resource::<computers::Computers>()
        .add_systems(
            Update,
            computers::computer_system
                .after(keybinds::bind_menu_system)
                .after(inventory_screen::inventory_screen_system)
                .after(security::security_system),
        )
        .insert_resource(splash::Splashes::default())
        .add_systems(
            Update,
            splash::splash_system
                .after(ai::npc_system)
                .after(grenades::grenade_system)
                .after(destruct::deco_damage_system),
        )
        .insert_resource(fish::FishState::default())
        .add_systems(
            Update,
            fish::fish_system
                .after(fly)
                .after(weapons::weapon_system)
                .after(explosion::explosion_system)
                .after(projectiles::projectile_system),
        )
        .add_systems(
            Update,
            critters::critter_system
                .after(fly)
                .after(weapons::weapon_system)
                .after(stealth::footstep_system)
                .before(ai::npc_system),
        )
        .add_systems(
            Update,
            (
                info::info_system.after(pickups::frob_system),
                sound::npc_footstep_system
                    .after(ai::npc_system)
                    .before(sound::sound_system),
                sound::sound_system
                    .after(fly)
                    .after(ai::npc_system)
                    .after(weapons::weapon_system)
                    .after(explosion::explosion_system)
                    .after(movers::tick_movers),
                drug::drug_system.before(fly),
                drug::scope_sway_system.before(fly).after(hud::aim_system),
                devices::device_system
                    .after(pickups::frob_system)
                    .after(frobui::frob_target_system),
            ),
        )
        .run();
}

/// The render device takes BC (DXT) compressed textures.
static GPU_BC: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

fn detect_bc1(dev: Option<Res<bevy::render::renderer::RenderDevice>>) {
    let ok = dev.is_none_or(|d| d.features().contains(bevy::render::render_resource::WgpuFeatures::TEXTURE_COMPRESSION_BC));
    GPU_BC.store(ok, std::sync::atomic::Ordering::Relaxed);
    if !ok {
        println!("dxview: no BC texture support on this GPU: DXT1 textures are decoded");
        dx_level::set_bc1_supported(false);
    }
}

/// A TextureImage's pixels for the GPU: (format, mips concatenated). BC1 blocks pass through unless the device
/// can't take them, then they decode to RGBA8.
pub fn image_data(t: &dx_level::TextureImage) -> (TextureFormat, Vec<u8>, u32) {
    let mut data = Vec::new();
    if t.bc3 {
        if GPU_BC.load(std::sync::atomic::Ordering::Relaxed) {
            for m in &t.mips {
                data.extend_from_slice(m);
            }
            return (TextureFormat::Bc3RgbaUnormSrgb, data, t.mips.len() as u32);
        }
        let mut n = 0;
        for (k, m) in t.mips.iter().enumerate() {
            let (w, h) = ((t.width >> k).max(1), (t.height >> k).max(1));
            let Some(px) = dx_level::dds::decode_level(dx_level::dds::Format::Bc3, m, w, h) else { break };
            data.extend_from_slice(&px);
            n += 1;
        }
        return (TextureFormat::Rgba8UnormSrgb, data, n.max(1));
    }
    if t.bc1 && GPU_BC.load(std::sync::atomic::Ordering::Relaxed) {
        for m in &t.mips {
            data.extend_from_slice(m);
        }
        return (TextureFormat::Bc1RgbaUnormSrgb, data, t.mips.len() as u32);
    }
    if t.bc1 {
        let mut n = 0;
        for (k, m) in t.mips.iter().enumerate() {
            let (w, h) = ((t.width >> k).max(1) as usize, (t.height >> k).max(1) as usize);
            let mut out = vec![0u8; w * h * 4];
            if unreal_pkg::objects::decode_dxt1(m, w, h, &mut out).is_none() {
                break;
            }
            data.extend_from_slice(&out);
            n += 1;
        }
        return (TextureFormat::Rgba8UnormSrgb, data, n.max(1));
    }
    for m in &t.mips {
        data.extend_from_slice(m);
    }
    (TextureFormat::Rgba8UnormSrgb, data, t.mips.len() as u32)
}

pub fn make_image(t: &dx_level::TextureImage) -> Image {
    let (format, data, levels) = image_data(t);
    let mut img = Image::new_uninit(
        Extent3d {
            width: t.width,
            height: t.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        format,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.texture_descriptor.mip_level_count = levels;
    img.data = Some(data);
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        address_mode_w: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 16,
        ..default()
    });
    img
}

fn setup(
    mut commands: Commands,
    level: Res<LevelRes>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    shot: Option<Res<ShotCfg>>,
    root_dir: Res<pickups::GameRoot>,
    mut mirror_mats: ResMut<Assets<mirror::MirrorMaterial>>,
    mut water_mats: ResMut<Assets<water::WaterMaterial>>,
    mut bsp_mats: ResMut<Assets<shadowlight::BspMaterial>>,
) {
    let mut water_list = water::WaterMats::default();
    let mut backing_mat: Option<Handle<StandardMaterial>> = None;
    // The tessellated water sheets' vertices (the open sea's first band meets them vertex to vertex).
    let mut sheet_pts: Vec<[f32; 3]> = Vec::new();
    let mut bsp_cache: std::collections::HashMap<AssetId<StandardMaterial>, Handle<shadowlight::BspMaterial>> = Default::default();
    let level = &level.0;
    let mut mirrors = mirror::Mirrors::default();
    let lightmap_handle: Option<Handle<Image>> = level.lightmap.as_ref().map(|a| {
        let mut img = Image::new(
            Extent3d {
                width: a.width,
                height: a.height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            a.rgba.clone(),
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            ..default()
        });
        images.add(img)
    });
    let handles: Vec<Handle<Image>> = level
        .textures
        .iter()
        .map(|t| images.add(make_image(t)))
        .collect();
    let mut cache: std::collections::HashMap<
        (Option<usize>, SurfaceClass, bool, [i32; 2], bool),
        Handle<StandardMaterial>,
    > = Default::default();
    let mut scrollers = UvScroll::default();

    for b in &level.batches {
        let mat = cache
            .entry((
                b.texture,
                b.class,
                b.sky,
                [(b.uv_scroll[0] * 1e4) as i32, (b.uv_scroll[1] * 1e4) as i32],
                b.one_sided,
            ))
            .or_insert_with(|| {
                let h = materials.add(StandardMaterial {
                    base_color_texture: b.texture.map(|i| handles[i].clone()),
                    base_color: match (b.texture.is_some(), b.class) {
                        (false, _) => Color::srgb(0.8, 0.2, 0.8),
                        (true, SurfaceClass::Water) => Color::srgba(0.75, 0.9, 1.0, 0.6),
                        _ => Color::WHITE,
                    },
                    lightmap_exposure: 1.0,
                    double_sided: !b.one_sided,
                    cull_mode: if b.one_sided { Some(bevy::render::render_resource::Face::Back) } else { None },
                    perceptual_roughness: 1.0,
                    reflectance: 0.05,
                    metallic: 0.0,
                    alpha_mode: match b.class {
                        SurfaceClass::Opaque => AlphaMode::Opaque,
                        SurfaceClass::Masked => AlphaMode::Mask(0.5),
                        SurfaceClass::Translucent => AlphaMode::Add,
                        SurfaceClass::Water => AlphaMode::Blend,
                        SurfaceClass::Modulated => AlphaMode::Multiply,
                    },
                    ..default()
                });
                let water = b.class == SurfaceClass::Water;
                if water || b.uv_scroll != [0.0, 0.0] {
                    scrollers
                        .0
                        .push((h.clone(), Vec2::from(b.uv_scroll), water));
                }
                h
            })
            .clone();
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        if b.class == SurfaceClass::Water && !b.sky {
            // Fine enough for the surface to move with the swell (Modern Water's waves).
            let (p, n, u, u1, i) = water::tessellate(&b.positions, &b.normals, &b.uvs, &b.uvs1, &b.indices, 2.0);
            sheet_pts.extend_from_slice(&p);
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, p);
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, n);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, u);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, u1);
            mesh.insert_indices(Indices::U32(i));
        } else {
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, b.positions.clone());
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, b.normals.clone());
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, b.uvs.clone());
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, b.uvs1.clone());
            mesh.insert_indices(Indices::U32(b.indices.clone()));
        }
        let mut e = match b.mirror {
            // Water: the WetTexture source picture rippled by the water shader (water.rs).
            _ if b.class == SurfaceClass::Water && !b.sky => {
                let base = materials.get(&mat).cloned().unwrap_or_default();
                let ph = mirror::placeholder(&mut mirrors, &mut images);
                let wm = water_mats.add(water::material(base, b.water_opaque, ph));
                water_list.mats.push(wm.clone());
                water_list.opaque.push(b.water_opaque);
                mirror::add_water(&mut mirrors, &wm, &b.positions);
                // Flat sheets get a dark backing under their moving surface (see water::WaterBacking).
                let (lo, hi) = b.positions.iter().fold((f32::MAX, f32::MIN), |a, p| (a.0.min(p[1]), a.1.max(p[1])));
                if hi - lo < 0.05 {
                    let back = backing_mat.get_or_insert_with(|| materials.add(water::backing_material())).clone();
                    commands.spawn((
                        Mesh3d(meshes.add(water::backing_mesh(&b.positions, &b.indices))),
                        MeshMaterial3d(back),
                        Visibility::Hidden,
                        water::WaterBacking { y: hi },
                        RenderLayers::layer(mirror::LAYER),
                        bevy::light::NotShadowCaster,
                        bevy::light::NotShadowReceiver,
                    ));
                }
                // Water isn't lightmapped: the shadow pairs (shadowlight.rs) would black it out under a shadow.
                // On the mirror faces' layer: only the world camera draws water (the reflection cameras would only clip
                // it away again).
                commands.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(wm), bevy::light::NotShadowReceiver, RenderLayers::layer(mirror::LAYER)))
            }
            // PF_Mirrored: the same surface with the reflection added (mirror.rs).
            Some(plane) => {
                let base = materials.get(&mat).cloned().unwrap_or_default();
                let mm = mirror::material(&mut mirrors, &mut images, &mut mirror_mats, base, plane, &b.positions, &b.indices);
                // On their own layer: the main camera draws them, the reflection camera must not (it would see
                // the mirror from behind).
                commands.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(mm), RenderLayers::layer(mirror::LAYER)))
            }
            // The lit world: StandardMaterial plus the real-time shadow pass over its lightmap (shadowlight.rs).
            None => {
                let bm = bsp_cache
                    .entry(mat.id())
                    .or_insert_with(|| {
                        let h = bsp_mats.add(shadowlight::bsp_material(materials.get(&mat).cloned().unwrap_or_default()));
                        if let Some((_, speed, _)) = scrollers.0.iter().find(|(m, _, w)| m.id() == mat.id() && !*w) {
                            scrollers.1.push((h.clone(), *speed));
                        }
                        h
                    })
                    .clone();
                commands.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(bm)))
            }
        };
        if !b.sky && b.texture.is_some_and(|t| level.textures[t].name.eq_ignore_ascii_case("CoreTexMisc.Marker_sky")) {
            // The map's edge walls (sky marker faces at the water line): hidden while the open sea runs past them.
            e.insert(water::EdgeWall);
        }
        if b.sky {
            let off = Vec3::from(level.sky_origin.unwrap_or([0.0; 3]));
            e.insert((Transform::from_translation(-off), RenderLayers::layer(1)));
            if b.class == SurfaceClass::Masked {
                e.insert(skyline::SkyPanel);
            }
        }
        if let Some(h) = lightmap_handle.as_ref().filter(|_| std::env::var_os("DXVIEW_NO_LIGHTMAP").is_none()) {
            e.insert(Lightmap {
                image: h.clone(),
                uv_rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                bicubic_sampling: false,
            });
        }
    }

    if let Some((mesh, mat)) = water::ocean_ring(&mirrors, &level.batches, &sheet_pts) {
        if let (Some(bevy::mesh::VertexAttributeValues::Float32x3(p)), Some(Indices::U32(i))) = (mesh.attribute(Mesh::ATTRIBUTE_POSITION), mesh.indices()) {
            let y = p.first().map_or(0.0, |v| v[1]);
            let back = backing_mat.get_or_insert_with(|| materials.add(water::backing_material())).clone();
            commands.spawn((
                Mesh3d(meshes.add(water::backing_mesh(p, i))),
                MeshMaterial3d(back),
                Visibility::Hidden,
                water::WaterBacking { y },
                RenderLayers::layer(mirror::LAYER),
                bevy::light::NotShadowCaster,
                bevy::light::NotShadowReceiver,
            ));
        }
        let mut e = commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(mat),
            Visibility::Hidden,
            water::OceanRing,
            RenderLayers::layer(mirror::LAYER),
            bevy::light::NotShadowReceiver,
            bevy::light::NotShadowCaster,
        ));
        if let Some(h) = lightmap_handle.as_ref().filter(|_| std::env::var_os("DXVIEW_NO_LIGHTMAP").is_none()) {
            e.insert(Lightmap { image: h.clone(), uv_rect: Rect::new(0.0, 0.0, 1.0, 1.0), bicubic_sampling: false });
        }
    }
    mirror::spawn_camera(&mut commands, &mirrors);
    commands.insert_resource(mirrors);
    commands.insert_resource(mirror::MirrorBodyRes::default());
    commands.insert_resource(water_list);
    // Mesh actors (decorations, pickups, characters) with live decoration physics.
    let blockers =
        actors::spawn_mesh_actors(&mut commands, level, &mut meshes, &mut materials, &handles);
    commands.insert_resource(blockers);
    let mv = movers::spawn_movers(&mut commands, level, &mut meshes, &mut materials, &handles);
    commands.insert_resource(mv);
    commands.insert_resource(movers::HudMessage::default());
    commands.insert_resource(pickups::FrobRequest::default());
    // DeusExPlayer.ResetPlayer: a new game starts with a pistol, a riot prod and a medkit on the belt.
    commands.insert_resource(pickups::starting_inventory(&root_dir.0));
    pickups::spawn_belt_ui(&mut commands);
    weapons::spawn_view_camera(&mut commands);
    commands.insert_resource(weapons::Weapons::default());
    commands.insert_resource(ai::WeaponDrops::default());
    commands.insert_resource(ai::FlareThrows::default());
    commands.insert_resource(charged::ActiveCharged::default());
    commands.insert_resource(spawnmenu::SpawnMenu::default());
    commands.insert_resource(effects::BurnFx::default());
    commands.insert_resource(effects::BloodDrips::default());
    commands.insert_resource(ai::GameFlags::default());
    commands.insert_resource(ai::Takedowns::default());
    commands.insert_resource(effects::Splats::default());
    commands.insert_resource(impacts::Decals::default());
    commands.insert_resource(weapons::Clips::default());
    commands.insert_resource(weapons::PawnHealth::default());
    commands.insert_resource(ai::PlayerHealth::default());
    commands.insert_resource(weapons::LastAttacker::default());
    commands.insert_resource(destruct::DecoHits::default());
    ai::spawn_health_ui(&mut commands);
    let mut runtime_assets = weapons::RuntimeAssets {
        classes: unreal_pkg::classes::ClassDb::open(&root_dir.0.join("System")),
        lib: dx_level::TexLib::new(&root_dir.0),
        images: Default::default(),
        materials: Default::default(),
        fire: Vec::new(),
    };
    hud::spawn_crosshair(&mut commands, &mut runtime_assets, &mut images);
    hud::spawn_scope_overlay(&mut commands, &mut runtime_assets, &mut images);
    coronas::spawn_coronas(&mut commands, level, &mut runtime_assets, &mut images);
    electricity::spawn_emitters(
        &mut commands,
        level,
        &mut runtime_assets,
        &mut images,
        &mut meshes,
        &mut materials,
    );
    drug::spawn_drug_overlay(&mut commands, &mut runtime_assets, &mut images);
    // AnimNext loops (screens, monitors, TV noise) on the level's surfaces and meshes.
    anim_tex::reset();
    for (i, t) in level.textures.iter().enumerate() {
        glow::register(&handles[i], &t.name);
        let masked = t.has_alpha_holes();
        anim_tex::register(&mut runtime_assets.lib, &t.name, &handles[i], masked, true);
    }
    // Procedural textures on the level's meshes and surfaces (the augmentation canister's blue Nano_SFX_A glow):
    // the level loader only has their static first frame, so they animate through the fire texture system.
    for (i, t) in level.textures.iter().enumerate() {
        if let Some(mut f) = runtime_assets.lib.load_fire(&t.name) {
            for _ in 0..40 {
                f.step();
            }
            let mut data = Vec::new();
            f.rgba(&mut data);
            let mut img = Image::new(
                Extent3d {
                    width: f.width as u32,
                    height: f.height as u32,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                data,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::default(),
            );
            img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: bevy::image::ImageAddressMode::Repeat,
                address_mode_v: bevy::image::ImageAddressMode::Repeat,
                mag_filter: ImageFilterMode::Linear,
                min_filter: ImageFilterMode::Linear,
                ..default()
            });
            if let Some(mut im) = images.get_mut(&handles[i]) {
                *im = img;
            }
            runtime_assets.fire.push((f, handles[i].clone()));
        }
    }
    commands.insert_resource(runtime_assets);
    commands.insert_resource(damage::PawnHits::default());
    commands.insert_resource(hud::Aim::default());
    commands.insert_resource(carry::Carrying::default());
    commands.insert_resource(stealth::Noises::default());
    commands.insert_resource(stealth::PlayerLight::default());
    commands.insert_resource(frobui::FrobHighlight::default());
    commands.insert_resource(projectiles::Launches::default());
    commands.insert_resource(destruct::Shells::default());
    commands.insert_resource(effects::BloodFx::default());
    commands.insert_resource(weapons::CorpseDrops::default());
    commands.insert_resource(weapons::Zoom::default());
    commands.insert_resource(movers::Picking::default());
    commands.insert_resource(devices::Hacking::default());
    commands.insert_resource(devices::GameEvents::default());
    frobui::spawn_frob_ui(&mut commands);
    info::spawn_info_panel(&mut commands);
    charged::spawn_goggles(&mut commands, &mut meshes, &mut materials);
    commands.insert_resource(info::InfoDisplay::default());
    explosion::spawn_flash_overlay(&mut commands);
    commands.insert_resource(explosion::Explosions::default());
    commands.insert_resource(explosion::BlastsThisFrame::default());
    commands.insert_resource(explosion::ScreenFlash::default());
    commands.insert_resource(grenades::GrenadeThrows::default());
    commands.insert_resource(projectiles::GasClouds::default());
    commands.insert_resource(effects::Gibs::default());
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: bevy::text::FontSize::Px(20.0),
            ..default()
        },
        TextColor(Color::srgb(1.0, 1.0, 1.0)),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(40.0),
            bottom: Val::Px(80.0),
            ..default()
        },
        HudText,
    ));
    commands.insert_resource(scrollers);

    // Lighting comes entirely from the rebuilt original lightmaps.
    commands.insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 0.0,
        ..default()
    });

    let (pos, yaw) = match &level.player_start {
        Some(p) => (Vec3::from(p.position) + Vec3::Y * 1.7, p.yaw),
        None => (Vec3::from(level.bounds_max) + Vec3::Y * 5.0, 0.0),
    };
    // Unreal yaw 0 looks along +X; Bevy cameras look along -Z by default.
    // Our fly yaw: 0 = looking along -Z. Unreal +X maps to Bevy +X, Unreal +Y to Bevy +Z.
    let cam_yaw = -yaw - std::f32::consts::FRAC_PI_2;
    // Sky room is drawn first by its own camera sitting at the origin (geometry is shifted by -sky_origin).
    commands.spawn((
        Camera3d::default(),
        Tonemapping::None,
        Exposure { ev100: -0.263 },
        Camera {
            order: 0,
            ..default()
        },
        Projection::Perspective(PerspectiveProjection {
            fov: 90f32.to_radians(),
            near: 0.05,
            far: 4000.0,
            ..default()
        }),
        Transform::default(),
        RenderLayers::layer(1),
        // Nothing in the sky box is lit by dynamic lights: one light cluster (GPU clustering breaks on ClusterConfig::None).
        bevy::light::cluster::ClusterConfig::Single,
        SkyCam,
    ));
    let (pos, cam_yaw, cam_pitch) = match &shot {
        Some(c) => (c.views[0].1, c.views[0].2, c.views[0].3),
        None => (pos, cam_yaw, 0.0),
    };
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: 1,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        Projection::Perspective(PerspectiveProjection {
            fov: 90f32.to_radians(),
            near: 0.05,
            far: 4000.0,
            ..default()
        }),
        Transform::from_translation(pos)
            .with_rotation(Quat::from_rotation_y(cam_yaw) * Quat::from_rotation_x(cam_pitch)),
        FlyCam {
            yaw: cam_yaw,
            pitch: cam_pitch,
            speed: 6.0,
        },
        Tonemapping::None,
        Exposure { ev100: -0.263 },
        // The world, plus mirror faces (mirror.rs).
        RenderLayers::from_layers(&[0, mirror::LAYER]),
    ));

    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: bevy::text::FontSize::Px(16.0),
            ..default()
        },
        TextColor(Color::srgb(0.9, 1.0, 0.9)),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(8.0),
            top: Val::Px(8.0),
            ..default()
        },
        Overlay,
    ));
}

fn cursor_grab(
    mut cursor: Single<&mut CursorOptions>,
    mouse: Res<ButtonInput<MouseButton>>,
    key: Res<ButtonInput<KeyCode>>,
    menu: Res<spawnmenu::SpawnMenu>,
    binds: Res<keybinds::Bindings>,
) {
    if binds.open {
        return;
    }
    if mouse.just_pressed(MouseButton::Left) && !menu.open && !consys::third_person() {
        cursor.visible = false;
        cursor.grab_mode = CursorGrabMode::Locked;
    }
    if key.just_pressed(KeyCode::Escape) && !menus::open() {
        cursor.visible = true;
        cursor.grab_mode = CursorGrabMode::None;
    }
}

fn fly(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    cursor: Single<&CursorOptions>,
    mut q: Single<(&mut Transform, &mut FlyCam), Without<SkyCam>>,
    mut sky: Single<&mut Transform, With<SkyCam>>,
    mut player: ResMut<Player>,
    level: Res<LevelRes>,
    blockers: Res<Blockers>,
    mut decos: Query<&mut Deco>,
    mut walk_test: Option<ResMut<WalkTest>>,
    movers_res: Res<movers::Movers>,
    mut frob_req: ResMut<pickups::FrobRequest>,
    (carrying, mut ph, weapons_res, mut scripted, mouse): (
        Res<carry::Carrying>,
        ResMut<ai::PlayerHealth>,
        Res<weapons::Weapons>,
        ResMut<weapons::ScriptedInput>,
        Res<ButtonInput<MouseButton>>,
    ),
) {
    let _prof = crate::prof::scope("main::fly");
    if ph.dead || cinematic::active() {
        return; // death.rs / cinematic.rs own the camera
    }
    let (tf, cam) = &mut *q;
    // Scripted walk test: fixed timestep, W held.
    let mut test_forward = false;
    let mut test_dt = None;
    let mut test_frob = false;
    let test_crouch = walk_test.as_ref().map_or(false, |w| w.crouch);
    if let Some(wt) = walk_test.as_deref_mut() {
        if wt.t == 0.0 {
            player.noclip = false;
            let params = player.params.clone();
            player.body = Character::new(wt.start);
            player.body.settle(
                &World {
                    level: &level.0,
                    blockers: &blockers.list,
                    ignore: None,
                    pushable: Some(&blockers.pushable_mass),
                    movers: Some(&movers_res.rt),
                },
                &params,
            );
            cam.yaw = (-wt.yaw_deg - 90.0).to_radians();
            cam.pitch = wt.pitch_deg.to_radians();
            // DXVIEW_WALKTEST_SELECT=<belt slot>: hold that belt item during the test.
            if let Some(slot) = std::env::var("DXVIEW_WALKTEST_SELECT").ok().and_then(|v| v.parse().ok()) {
                scripted.select = Some(slot);
            }
        }
        if wt.t < wt.secs {
            test_forward = true;
            test_dt = Some(1.0 / 60.0);
            wt.t += 1.0 / 60.0;
        } else if wt.t < wt.secs + wt.frob_then {
            test_dt = Some(1.0 / 60.0);
            wt.t += 1.0 / 60.0;
            if !wt.frobbed {
                wt.frobbed = true;
                test_frob = true;
            }
        }
    }
    // Third person's free camera: the look turns the camera round JC, not JC (thirdperson.rs).
    if let Some((y, p)) = thirdperson::sync(cam.yaw, cam.pitch) {
        cam.yaw = y;
        cam.pitch = p;
    }
    let free_cam = thirdperson::free();
    let mut looked = false;
    if cursor.grab_mode == CursorGrabMode::Locked {
        // MouseSensitivity (default 3) scales the look; Invert Mouse flips the pitch.
        let k = 0.0022 * crate::settings::mouse_sensitivity() / 3.0;
        let inv = if crate::settings::invert_mouse() { -1.0 } else { 1.0 };
        // The gamepad's right stick turns at a rate (gamepad.rs).
        let pad = gamepad::look_delta(time.delta_secs());
        let dyaw = motion.delta.x * k + pad.x;
        let dpitch = -inv * motion.delta.y * k + pad.y;
        looked = dyaw != 0.0 || dpitch != 0.0;
        if free_cam {
            thirdperson::orbit_look(dyaw, dpitch);
        } else {
            cam.yaw -= dyaw;
            cam.pitch = (cam.pitch + dpitch).clamp(-1.55, 1.55);
        }
    }
    thirdperson::note_input(
        looked
            || keys.get_pressed().next().is_some()
            || mouse.get_pressed().next().is_some()
            || gamepad::move_axes().is_some_and(|a| a.length() > 0.1),
        time.delta_secs(),
    );
    if scroll.delta.y != 0.0 && player.noclip {
        cam.speed = (cam.speed * (1.0 + 0.15 * scroll.delta.y.signum())).clamp(0.5, 200.0);
    }
    // Leaning rolls the view: ViewRotation.Roll = curLeanDist * 20 (Unreal units).
    // ViewShake adds its own roll and eye bob (PlayerPawn.ViewShake).
    let (shake_roll, shake_vert) = crate::shake::tick(time.delta_secs());
    let roll = (((player.lean + player.peek) * 20.0 + shake_roll) / 65536.0 * std::f64::consts::TAU) as f32;
    tf.rotation = Quat::from_rotation_y(cam.yaw)
        * Quat::from_rotation_x(cam.pitch)
        * Quat::from_rotation_z(-roll);
    if test_frob {
        frob_req.0 = true;
    }
    if keys.just_pressed(KeyCode::KeyV) && crate::settings::debug_mode() {
        player.noclip = !player.noclip;
        if !player.noclip {
            // Drop the body where the camera is.
            let ue = bevy_to_ue(tf.translation.to_array());
            let eye = player.eye_height;
            let params = player.params.clone();
            player.body =
                Character::new(DVec3::new(ue[0] as f64, ue[1] as f64, ue[2] as f64 - eye));
            player.body.settle(
                &World {
                    level: &level.0,
                    blockers: &blockers.list,
                    ignore: None,
                    pushable: Some(&blockers.pushable_mass),
                    movers: Some(&movers_res.rt),
                },
                &params,
            );
        }
    }
    if !player.noclip {
        // Walk: WASD on the ground plane in Unreal space, Space jumps, Shift walks slowly.
        // (Free third-person camera: relative to the camera.)
        let (f, r) = if free_cam {
            let (oy, op) = thirdperson::orbit();
            let q = Quat::from_rotation_y(oy) * Quat::from_rotation_x(op);
            (q * Vec3::NEG_Z, q * Vec3::X)
        } else {
            (tf.forward().as_vec3(), tf.right().as_vec3())
        };
        let (fu, ru) = (
            DVec3::new(f.x as f64, f.z as f64, 0.0),
            DVec3::new(r.x as f64, r.z as f64, 0.0),
        );
        let mut wish = DVec3::ZERO;
        if keys.pressed(KeyCode::KeyW) || test_forward {
            wish += fu;
        }
        if keys.pressed(KeyCode::KeyS) {
            wish -= fu;
        }
        if keys.pressed(KeyCode::KeyD) {
            wish += ru;
        }
        if keys.pressed(KeyCode::KeyA) {
            wish -= ru;
        }
        let mut wish = wish.normalize_or_zero();
        // The gamepad's left stick: analog direction (a light push also holds Walk, gamepad.rs).
        if let Some(a) = gamepad::move_axes() {
            if !crate::inventory_screen::screen_open() && !crate::consys::third_person() {
                wish = (fu * a.y as f64 + ru * a.x as f64).normalize_or_zero();
            }
        }
        // Free third-person camera: JC turns to face the way he moves, or where the camera looks while aiming or
        // shooting (quickly, so the shot leaves the way he faces); his view pitch is the camera's.
        if free_cam {
            let dt = time.delta_secs().min(0.1);
            let (oy, op) = thirdperson::orbit();
            let aim_keys = keys.pressed(KeyCode::KeyZ) || keys.pressed(KeyCode::BracketLeft) || crate::aimmode::ads() > 0.3;
            let fired = mouse.pressed(MouseButton::Left)
                && weapons_res.held.as_ref().is_some_and(|h| !h.stats.item);
            let carrying_now = carry::carrying();
            let face_cam = thirdperson::facing_camera(aim_keys, fired && !carrying_now, dt);
            let target = if face_cam {
                Some((oy, 22.0))
            } else if wish.length_squared() > 1e-6 {
                // FlyCam yaw: forward = (-sin yaw, -cos yaw) in Unreal x / y.
                Some(((-wish.x).atan2(-wish.y) as f32, 9.0))
            } else {
                None
            };
            if let Some((ty, rate)) = target {
                let d = (ty - cam.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
                cam.yaw += d * (dt * rate).min(1.0);
            }
            cam.pitch = op.clamp(-1.3, 1.2);
            tf.rotation = Quat::from_rotation_y(cam.yaw) * Quat::from_rotation_x(cam.pitch) * Quat::from_rotation_z(-roll);
        }
        // AugDrone: the keys fly the drone (forward / strafe along the view, Jump / Duck up and down) and the
        // player stands frozen (PlayerWalking.ProcessMove: Velocity = 0).
        let drone = augdisplay::drone_flying();
        if drone {
            let fw = DVec3::new(f.x as f64, f.z as f64, f.y as f64);
            let rt = DVec3::new(r.x as f64, r.z as f64, r.y as f64);
            let ax = |a: KeyCode, b: KeyCode| (keys.pressed(a) as i32 - keys.pressed(b) as i32) as f64;
            let v = fw * ax(KeyCode::KeyW, KeyCode::KeyS) + rt * ax(KeyCode::KeyD, KeyCode::KeyA)
                + DVec3::Z * ax(KeyCode::Space, KeyCode::KeyC);
            augdisplay::drone_input(v.normalize_or_zero());
            wish = DVec3::ZERO;
            player.body.velocity.x = 0.0;
            player.body.velocity.y = 0.0;
        }
        // PlayerSwimming.PlayerMove: aForward along the full view direction, strafe, and Jump/Duck as aUp, at
        // 0.2 / 0.1 / 0.1 of the raw axes (forward tops the 0.3 x AccelRate clamp, the others nearly reach it).
        let swim_up =
            (keys.pressed(KeyCode::Space) as i32 - keys.pressed(KeyCode::KeyC) as i32) as f64;
        if player.swimming && !drone {
            let x = DVec3::new(f.x as f64, f.z as f64, f.y as f64).normalize_or_zero();
            let mut fwd_in = 0.0;
            if keys.pressed(KeyCode::KeyW) || test_forward {
                fwd_in += 1.0;
            }
            if keys.pressed(KeyCode::KeyS) {
                fwd_in -= 1.0;
            }
            let mut side = 0.0;
            if keys.pressed(KeyCode::KeyD) {
                side += 1.0;
            }
            if keys.pressed(KeyCode::KeyA) {
                side -= 1.0;
            }
            let w3 = x * fwd_in + ru.normalize_or_zero() * side * 0.83 + DVec3::Z * swim_up * 0.83;
            wish = if w3.length() > 1.0 {
                w3.normalize()
            } else {
                w3
            };
        }
        // Ladders (faces in a "Ladder" texture group): touching one there's no gravity, and pushing into it climbs
        // - up, or down when looking down; walking away or off the side lets go.
        if !player.noclip && player.body.physics != Physics::Swimming {
            let c = player.body.location;
            let (r, hh) = (player.params.radius, player.params.height);
            let ladder = level
                .0
                .ladders
                .iter()
                .filter(|l| l.touches([c.x, c.y, c.z], r, hh, 4.0))
                .max_by(|a, b| {
                    let da =
                        (c.x * a.normal[0] + c.y * a.normal[1] + c.z * a.normal[2] - a.dist).abs();
                    let db =
                        (c.x * b.normal[0] + c.y * b.normal[1] + c.z * b.normal[2] - b.dist).abs();
                    db.partial_cmp(&da).unwrap()
                });
            if std::env::var("DXVIEW_LADDERLOG").is_ok() {
                println!(
                    "LADDER touch={} phys={:?} loc={:.1?} wish={:.2?}",
                    ladder.is_some(),
                    player.body.physics,
                    c,
                    wish
                );
            }
            match ladder {
                Some(l) => {
                    // The face's normal points out of the ladder; toward the player side.
                    let mut n = DVec3::new(l.normal[0], l.normal[1], 0.0).normalize_or_zero();
                    if (c.x * l.normal[0] + c.y * l.normal[1] + c.z * l.normal[2] - l.dist) < 0.0 {
                        n = -n;
                    }
                    let into = wish.dot(-n);
                    let climbing = player.body.physics == Physics::Climbing;
                    // Enhancement "Grab Ladders": Use takes hold, Jump lets go (pushing off), and on the ladder
                    // forward / back climb up / down; otherwise walking into one does nothing special.
                    let grab = settings::get("LadderGrab") > 0.5;
                    frobui::LADDER_GRAB.store(grab && !climbing, std::sync::atomic::Ordering::Relaxed);
                    if grab && !climbing {
                        if mouse.just_pressed(MouseButton::Right) {
                            player.body.physics = Physics::Climbing;
                            player.body.velocity = DVec3::ZERO;
                        }
                    } else if grab && keys.just_pressed(KeyCode::Space) {
                        player.body.physics = Physics::Falling;
                        player.body.velocity = n * 180.0 + DVec3::Z * 160.0;
                        wish = DVec3::ZERO;
                    } else if grab {
                        let looking_down = cam.pitch < -0.35;
                        let vertical = into * if looking_down { -1.0 } else { 1.0 };
                        // Keep some of the push into it so you step off at the top; pulling back doesn't leave.
                        let flat = wish - (-n) * if into > 0.0 { into * 0.7 } else { into };
                        wish = flat + DVec3::Z * vertical;
                    } else if into > 0.3 || (climbing && wish.length_squared() > 0.0) {
                        let looking_down = cam.pitch < -0.35;
                        let vertical = if into > 0.3 {
                            into * if looking_down { -1.0 } else { 1.0 }
                        } else {
                            0.0
                        };
                        // Sideways/back movement stays as it is; pushing into the ladder turns into climbing.
                        // Keep a little of the push toward the ladder so you step off onto the ledge at the top.
                        let flat = wish - (-n) * into.max(0.0) * 0.7;
                        wish = flat + DVec3::Z * vertical;
                        player.body.physics = Physics::Climbing;
                    } else if climbing && wish.length_squared() == 0.0 {
                        // Hold still on the ladder.
                    }
                }
                None => {
                    frobui::LADDER_GRAB.store(false, std::sync::atomic::Ordering::Relaxed);
                    if player.body.physics == Physics::Climbing {
                        player.body.physics = Physics::Falling;
                    }
                }
            }
        }
        let carried_mass = carrying.held.as_ref().map(|h| h.mass).unwrap_or(0.0);
        // Can't jump with anything heavier than 20 in your arms; carrying slows you by Mass * 2.
        // DoJump: nothing heavier than 20 in your arms. GMDX fix (setting FixCarryJump): the Muscle aug, which the
        // original read only after giving up, lets you jump with up to (CurrentLevel + 3) x 50 (150-300).
        let carry_limit = match crate::abilities::aug_level("AugMuscle") {
            Some(l) if crate::settings::get("FixCarryJump") > 0.5 && crate::abilities::aug_active("AugMuscle") => {
                (l as f64 + 3.0) * 50.0
            }
            _ => 20.0,
        };
        let jump = keys.just_pressed(KeyCode::Space) && carried_mass <= carry_limit && !player.swimming && !drone;
        let world = actors::SwimWorld(World {
            level: &level.0,
            blockers: &blockers.list,
            ignore: None,
            pushable: Some(&blockers.pushable_mass),
            movers: Some(&movers_res.rt),
        });
        let z_before = player.body.location.z;
        // Crouch (DeusExPlayer.HandleWalking / SetBasedPawnSize): hold C. Ducking shrinks the cylinder to 16 with the
        // feet kept on the floor, the eye drops with it, and you move at walking pace. You stay down while there
        // isn't room to stand (bForceDuck).
        let (leg_speed, legless) = ph.speed_factor();
        // PlayerSwimming.BeginState: SetBasedPawnSize(radius, 16) - swimming is the crouched size.
        let mut duck_key = (keys.pressed(KeyCode::KeyC) && !drone) || test_crouch;
        // DeusExPlayer.HandleWalking with bToggleCrouch: a press latches crouching on, the next press-and-release
        // turns it off. Swimming clears it (PlayerSwimming.BeginState).
        if player.swimming {
            player.crouch_on = false;
            player.was_crouch_on = false;
            player.last_duck = false;
        } else if crate::settings::toggle_crouch() {
            // Key events drive bDuck; between them it keeps its value (including the forced 1 below).
            if keys.just_pressed(KeyCode::KeyC) && !drone {
                player.duck_btn = true;
            }
            if keys.just_released(KeyCode::KeyC) {
                player.duck_btn = false;
            }
            let mut b_duck = player.duck_btn || test_crouch;
            if !player.crouch_on && !player.was_crouch_on && b_duck {
                player.crouch_on = true;
            } else if player.crouch_on && !player.was_crouch_on && !b_duck {
                player.was_crouch_on = true;
            } else if player.crouch_on && player.was_crouch_on && !b_duck && player.last_duck {
                player.crouch_on = false;
                player.was_crouch_on = false;
            }
            if player.crouch_on {
                b_duck = true;
                player.duck_btn = true;
            }
            player.last_duck = b_duck;
            duck_key = b_duck;
        } else {
            player.crouch_on = false;
            player.was_crouch_on = false;
        }
        let want_duck = duck_key || legless || player.swimming;
        let duck_h = 16.0;
        let (stand_h, stand_eye) = (player.stand_height, player.stand_eye);
        let delta_eye = stand_h - stand_eye;
        if (want_duck || player.force_duck) && !player.crouched {
            player.crouched = true;
            player.body.location.z -= stand_h - duck_h;
            player.params.height = duck_h;
            player.eye_height = duck_h - delta_eye;
        }
        if player.crouched {
            // Is there room to stand up? Trace up to twice the standing height from the feet.
            let loc = player.body.location;
            let top = DVec3::new(loc.x, loc.y, loc.z - duck_h + 2.0 * stand_h);
            let r = player.params.radius;
            player.force_duck = imsim_core::movement::CollisionWorld::trace(
                &world,
                loc,
                top,
                DVec3::new(r, r, 1.0),
            )
            .fraction
                < 1.0;
            if !want_duck && !player.force_duck {
                player.crouched = false;
                player.body.location.z += stand_h - duck_h;
                player.params.height = stand_h;
                player.eye_height = stand_eye;
            }
        }
        // Leaning (DeusExPlayer.PlayerWalking.ProcessMove): Q/E while standing still slides the body sideways up
        // to 40 units (if there's room and floor), lowers it a little, and rolls the view; letting go eases back.
        {
            let input =
                (keys.pressed(KeyCode::KeyE) as i32 - keys.pressed(KeyCode::KeyQ) as i32) as f64;
            let forward_held =
                keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::KeyS) || test_forward;
            let can_lean = player.body.velocity.truncate().length() < 10.0
                && !forward_held
                && player.body.physics == Physics::Walking;
            let right = DVec3::new(ru.x, ru.y, 0.0).normalize_or_zero();
            let dtl = test_dt.unwrap_or(time.delta_secs_f64().min(0.1));
            let r = player.params.radius;
            let h = player.params.height;
            let try_move = |from: DVec3, by: f64| -> Option<DVec3> {
                let to = from + right * by;
                let ext = DVec3::new(r, r, h);
                let blocked = imsim_core::movement::CollisionWorld::trace(&world, from, to, ext)
                    .fraction
                    < 1.0;
                let floor = imsim_core::movement::CollisionWorld::trace(
                    &world,
                    to,
                    to - DVec3::new(0.0, 0.0, h),
                    ext,
                )
                .fraction
                    < 1.0;
                (!blocked && floor).then_some(to)
            };
            if crate::settings::get("SmoothLean") > 0.5 {
                // Enhancement (Smooth Leaning): eases in and out instead of the fixed 80 units/s slide, works while
                // crouch-walking, and where the body can't slide any further (a wall at the corner) the eyes go on
                // as far as there's room for them, so you can peek past it.
                let crouch_ok = player.crouched
                    && player.body.velocity.truncate().length() < 130.0
                    && player.body.physics == Physics::Walking;
                let target = if can_lean || crouch_ok { input * 40.0 } else { 0.0 };
                let total = player.lean + player.peek;
                let mut want = total + (target - total) * (1.0 - (-9.0 * dtl).exp());
                if (target - want).abs() < 0.3 {
                    want = target;
                }
                let step = want.clamp(-40.0, 40.0) - player.lean;
                if step.abs() > 1e-6 {
                    let loc = player.body.location;
                    let moved = try_move(loc, step).map(|to| (to, step)).or_else(|| try_move(loc, step * 0.5).map(|to| (to, step * 0.5)));
                    if let Some((to, by)) = moved {
                        player.body.location = to;
                        player.lean += by;
                    }
                }
                let rest = want - player.lean;
                player.peek = if rest.abs() > 0.1 && target != 0.0 {
                    let eye = player.body.location + DVec3::new(0.0, 0.0, player.eye_height);
                    let f = imsim_core::movement::CollisionWorld::trace(&world, eye, eye + right * rest, DVec3::splat(5.0)).fraction;
                    rest * (f - 0.05).max(0.0)
                } else if target == 0.0 {
                    rest * (-9.0 * dtl).exp()
                } else {
                    0.0
                };
                if player.peek.abs() < 0.1 {
                    player.peek = 0.0;
                }
                if player.lean.abs() < 0.1 && target == 0.0 {
                    player.lean = 0.0;
                }
            } else if can_lean && input != 0.0 {
                player.peek = 0.0;
                let alpha = 40.0 * input * 2.0 * dtl;
                if (player.lean + alpha).abs() < 40.0 {
                    if let Some(to) = try_move(player.body.location, alpha) {
                        player.body.location = to;
                        player.lean += alpha;
                    }
                } else {
                    player.lean = input * 40.0;
                }
            } else if player.lean != 0.0 {
                let prev = player.lean;
                let a = (7.0 * dtl).clamp(0.001, 0.9);
                player.lean *= 1.0 - a;
                if player.lean.abs() < 1.0 {
                    player.lean = 0.0;
                }
                if let Some(to) = try_move(player.body.location, -(prev - player.lean)) {
                    player.body.location = to;
                }
            }
            // SetBasedPawnSize(radius, default height - |lean| / 3), feet kept on the floor.
            if !player.crouched {
                let want = stand_h - player.lean.abs() / 3.0;
                let d = player.params.height - want;
                if d.abs() > 1e-6 {
                    player.body.location.z -= d;
                    player.params.height = want;
                    player.eye_height = want - delta_eye;
                }
            }
        }
        let mut params = player.params.clone();
        // GMDX Sprinter (GetCurrentGroundSpeed): x PerkValue with empty hands and nothing carried, before AugSpeed.
        if let Some(v) = crate::perks::value_if("PerkSprinter") {
            if weapons_res.held.is_none() && carried_mass <= 0.0 && weapons_res.corpse.is_none() {
                params.ground_speed *= v as f64;
            }
        }
        // AugSpeed: GroundSpeed and JumpZ x LevelValue while active.
        if let Some(v) = crate::abilities::aug("AugSpeed") {
            params.ground_speed *= v as f64;
            params.jump_z *= v as f64;
        }
        params.ground_speed = (params.ground_speed - carried_mass * 2.0).max(40.0);
        // Carrying a body (POVCorpse, Mass 40) costs Mass * 3 of speed.
        if weapons_res.corpse.is_some() {
            params.ground_speed = (params.ground_speed - 40.0 * 3.0).max(100.0);
        }
        // Wounded legs and torso slow you down; with no legs you crawl.
        params.ground_speed *= leg_speed;
        // Modern Aiming: walking pace down the sights.
        params.ground_speed *= crate::aimmode::speed_mult();
        // Always Run: Shift walks; off, you walk unless Shift is held.
        let walk_key = keys.pressed(KeyCode::ShiftLeft) != !crate::settings::always_run();
        if walk_key || player.crouched {
            params.ground_speed *= 0.3;
        }
        // WaterSpeed = Default.WaterSpeed x SkillSwimming (untrained 1.0).
        params.water_speed =
            player.params.water_speed * crate::abilities::skill("SkillSwimming") as f64;
        player.body.bumped.clear();
        let dt = test_dt.unwrap_or((time.delta_secs_f64()).min(0.1));
        let steps = (dt / (1.0 / 120.0)).ceil().max(1.0) as usize;
        let mut landed = None;
        // A carried object can't be shoved into people: it's held in front at chest height, and we stop where it
        // would run into a pawn.
        let held = carrying.held.as_ref().map(|h| (h.radius, h.height));
        let pawns: Vec<dx_level::collision::Cylinder> = if held.is_some() {
            blockers
                .list
                .iter()
                .filter(|c| {
                    c.radius > 0.0
                        && unreal_pkg::props::find(&level.0.actors[c.actor].props, "GroundSpeed")
                            .is_some()
                })
                .copied()
                .collect()
        } else {
            Vec::new()
        };
        let was_walking = player.body.physics == Physics::Walking;
        // Enhancement (Mantling, mantle.rs): Jump at a ledge climbs it; while climbing nothing else moves you.
        let mut climbing = false;
        if crate::mantle::active() {
            if player.swimming || player.noclip {
                crate::mantle::cancel();
            } else if let Some((pos, done)) = crate::mantle::step(dt) {
                player.body.location = pos;
                player.body.velocity = DVec3::ZERO;
                player.body.physics = if done { Physics::Walking } else { Physics::Falling };
                climbing = true;
            }
        } else if crate::mantle::on()
            && !player.crouched
            && !player.swimming
            && !drone
            && carried_mass <= carry_limit
            && (jump
                || (keys.pressed(KeyCode::Space)
                    && player.body.physics == Physics::Falling
                    && player.body.velocity.z < 250.0))
        {
            let fwd = if wish.length_squared() > 0.1 { wish } else { fu.normalize_or_zero() };
            if crate::mantle::try_start(&world, player.body.location, &params, fwd) {
                climbing = true;
            }
        }
        let jump = jump && !climbing;
        for i in 0..if climbing { 0 } else { steps } {
            let vz = player.body.velocity.z;
            player
                .body
                .tick(&world, &params, wish, jump && i == 0, dt / steps as f64);
            if player.body.landed_speed.is_some() {
                landed = Some(vz);
            }
            if let Some((hr, hh)) = held {
                // Push back out of any pawn the carried object would be inside (walking, turning, or a pawn
                // stepping into it).
                let fwd = DVec3::new(fu.x, fu.y, 0.0).normalize_or_zero();
                let box_at = player.body.location
                    + DVec3::new(0.0, 0.0, params.height / 2.0)
                    + fwd * (0.5 * params.radius + hr);
                let mut push = DVec3::ZERO;
                for c in &pawns {
                    if (box_at.z - c.center.z).abs() >= c.height + hh {
                        continue;
                    }
                    let d = DVec3::new(box_at.x - c.center.x, box_at.y - c.center.y, 0.0);
                    let overlap = c.radius + hr - d.length();
                    if overlap > 0.0 {
                        let dir = if d.length_squared() > 1e-6 {
                            d.normalize()
                        } else {
                            -fwd
                        };
                        push += dir * (overlap + 0.5);
                    }
                }
                if push.length_squared() > 0.0 {
                    let loc = player.body.location;
                    let r = params.radius;
                    let h = imsim_core::movement::CollisionWorld::trace(
                        &world,
                        loc,
                        loc + push,
                        DVec3::new(r, r, params.height),
                    );
                    player.body.location = loc + push * h.fraction;
                    let into = player.body.velocity.dot(push.normalize());
                    if into < 0.0 {
                        player.body.velocity -= push.normalize() * into;
                    }
                }
            }
        }
        swim_state(&mut player, &world, &params, wish, cam.pitch, dt);
        // DeusExPlayer.DoJump: JumpSound (volume 1.5, bNoOverride, radius 1200, pitch 0.8..1).
        if jump && was_walking && player.body.physics == Physics::Falling && !player.noclip {
            sound::play_req(sound::Req {
                sound: travel::voice("DeusExSounds.Player.MaleJump"),
                pos: None,
                volume: 1.5,
                radius: 1200.0,
                pitch: 1.0 - 0.2 * sound::frand(),
                owner: sound::PLAYER,
                slot: sound::slot::MISC,
                no_override: true,
                min_gap: 0.0,
            });
        }
        if let Some(vz) = landed {
            player.view.landed(vz);
            // Human.PlayLanded plays a footstep.
            if !player.noclip && vz < -150.0 {
                dualsense::event(dualsense::Haptic::Land(((-vz as f32) - 150.0) / 600.0));
                let lvl = &level.0;
                let feet = player.body.location - DVec3::new(0.0, 0.0, params.height - 2.0);
                let water = matches!(
                    lvl.zone_water
                        .get(lvl.collision.zone_at(feet).max(0) as usize),
                    Some(Some(_))
                );
                let (snd, _) = sound::footstep_sound(
                    lvl.floor_group(player.body.location.to_array(), params.height * 2.0),
                    water,
                );
                let vol = ((-vz as f32) / 180.0 + 0.2).clamp(0.0, 1.0) * 0.5;
                sound::play_owned(
                    sound::PLAYER,
                    sound::slot::INTERACT,
                    &snd,
                    None,
                    vol,
                    1500.0,
                    1.0,
                );
            }
            // DeusExPlayer.Landed: falling faster than 700 hurts both legs (0.16 per unit/s over) and the torso (0.06).
            if vz < -700.0 && !player.noclip {
                // AugSpeed takes 15 x (level + 1) off each hit.
                let aug_reduce = crate::abilities::aug_level("AugSpeed")
                    .map_or(0.0, |l| 15.0 * (l as f64 + 1.0));
                let leg = (-0.16 * (vz + 700.0) - aug_reduce).max(0.0) as f32;
                let torso = (-0.06 * (vz + 700.0) - aug_reduce).max(0.0) as f32;
                ph.damage(leg, "fell", DVec3::new(0.0, -1.0, -1.0));
                ph.damage(leg, "fell", DVec3::new(0.0, 1.0, -1.0));
                ph.damage(torso, "fell", DVec3::new(0.0, 0.0, 1.0));
            }
        }
        if !player.body.bumped.is_empty() {
            let b = &player.body;
            actors::apply_pushes(
                &b.bumped,
                b.location,
                params.height,
                b.intended_velocity,
                &blockers,
                &mut decos,
            );
        }
        let z_moved = player.body.location.z - z_before;
        let walking = player.body.physics == Physics::Walking;
        let (vel, eye_h) = (player.body.velocity, player.eye_height + shake_vert);
        player.view.update(
            dt,
            walking,
            vel,
            params.ground_speed,
            ru.normalize_or_zero(),
            eye_h,
            z_moved,
        );
        if !crate::settings::head_bob() {
            player.view.walk_bob = DVec3::ZERO;
        }
        if player.swimming {
            player
                .view
                .swim_bob(time.elapsed_secs_f64(), vel, ru.normalize_or_zero());
        }
        let l = player.body.location
            + player.view.camera_offset()
            + DVec3::new(ru.x, ru.y, 0.0).normalize_or_zero() * player.peek;
        let eye = ue_to_bevy([l.x as f32, l.y as f32, l.z as f32]);
        tf.translation = Vec3::from(eye);
        sky.rotation = tf.rotation;
        return;
    }
    let fwd = tf.forward().as_vec3();
    let right = tf.right().as_vec3();
    let mut d = Vec3::ZERO;
    if keys.pressed(KeyCode::KeyW) {
        d += fwd;
    }
    if keys.pressed(KeyCode::KeyS) {
        d -= fwd;
    }
    if keys.pressed(KeyCode::KeyD) {
        d += right;
    }
    if keys.pressed(KeyCode::KeyA) {
        d -= right;
    }
    if keys.pressed(KeyCode::Space) {
        d += Vec3::Y;
    }
    if keys.pressed(KeyCode::ControlLeft) {
        d -= Vec3::Y;
    }
    let mult = if keys.pressed(KeyCode::ShiftLeft) {
        4.0
    } else {
        1.0
    };
    tf.translation += d.normalize_or_zero() * cam.speed * mult * time.delta_secs();
    sky.rotation = tf.rotation;
}

/// PlayerSwimming's way in and out: start when the physics starts swimming; on surfacing (ZoneChange to a dry
/// zone) jump out onto a ledge in front if pushing up-and-forward while looking up (CheckWaterJump), stand up if
/// the feet are out or something is underfoot, else wait 0.7 s over deep water.
fn swim_state(
    player: &mut Player,
    world: &actors::SwimWorld,
    params: &MoveParams,
    wish: DVec3,
    pitch: f32,
    dt: f64,
) {
    use imsim_core::movement::CollisionWorld;
    if player.noclip {
        player.swimming = false;
        player.swim_out = None;
        return;
    }
    if !player.swimming {
        if player.body.physics == Physics::Swimming {
            player.swimming = true;
            player.swim_out = None;
        }
        return;
    }
    if player.body.physics == Physics::Swimming {
        player.swim_out = None;
        return;
    }
    let b = player.body.clone();
    let (r, h) = (params.radius, params.height);
    let look = DVec3::new(wish.x, wish.y, 0.0);
    // bUpAndOut = (X . Acceleration) > 0 && Acceleration.Z > 0 && ViewRotation.Pitch > 4096 (22.5 degrees).
    let up_and_out =
        wish.z > 0.0 && (pitch as f64) > 22.5f64.to_radians() && look.length_squared() > 1e-6;
    // Pawn.CheckWaterJump: a wall (not a pawn) right in front at body height, and clear space 1.1 x MaxStepHeight up.
    let water_jump = || {
        let dir = look.normalize_or_zero();
        let loc = b.location;
        let hit = world.trace(loc, loc + dir * r, DVec3::new(r, r, h));
        let pawn = hit.actor.is_some_and(|a| {
            world.0.blockers.get(a).is_some_and(|c| {
                unreal_pkg::props::find(&world.0.level.actors[c.actor].props, "GroundSpeed")
                    .is_some()
            })
        });
        if hit.fraction >= 1.0 || pawn {
            return false;
        }
        let start = loc + DVec3::Z * 1.1 * params.max_step_height;
        world
            .trace(start, start + dir * 2.0 * r, DVec3::ZERO)
            .fraction
            >= 1.0
    };
    let center_dry = world.water_at(b.location).is_none();
    let jump_out = up_and_out && center_dry && water_jump();
    let leave = if b.physics == Physics::Climbing {
        true
    } else if jump_out {
        player.body.velocity.z = 330.0 + 2.0 * r;
        true
    } else {
        match player.swim_out {
            None => {
                // Just surfaced.
                let feet = b.location - DVec3::Z * (h - 2.0);
                if world.water_at(feet).is_none()
                    || b.velocity.z > 160.0
                    || b.physics == Physics::Walking
                {
                    true
                } else if world
                    .trace(b.location, b.location - DVec3::Z * (h + 6.0), DVec3::ZERO)
                    .fraction
                    < 1.0
                {
                    true
                } else {
                    player.swim_out = Some(0.7);
                    false
                }
            }
            Some(t) => {
                let t = t - dt;
                player.swim_out = Some(t);
                t <= 0.0 && center_dry
            }
        }
    };
    if leave {
        player.swimming = false;
        player.swim_out = None;
    }
}

fn overlay_update(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    level: Res<LevelRes>,
    cam: Single<(&Transform, &FlyCam), Without<SkyCam>>,
    player: Res<Player>,
    mut text: Single<(&mut Text, &mut Visibility), With<Overlay>>,
    mut show: Local<Option<bool>>,
) {
    let _prof = crate::prof::scope("main::overlay_update");
    let on = show.get_or_insert(true);
    if keys.just_pressed(KeyCode::PageDown) {
        *on = !*on;
    }
    // The corner readout is a Debug Mode feature (Settings > Enhancements).
    let debug = crate::settings::debug_mode() || std::env::var("DXVIEW_SHOT").is_ok();
    *text.1 = if *on && debug && !inventory_screen::screen_open() && !consys::third_person() && !cinematic::active() {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    let s = &level.0.stats;
    let p = cam.0.translation;
    let mode = if player.noclip {
        "FLY (V to walk) | WASD fly | Shift fast | Space/Ctrl up/down | Wheel speed".to_string()
    } else {
        let b = &player.body;
        let state = match b.physics {
            Physics::Walking => "walking",
            Physics::Falling => "falling",
            Physics::Noclip => "noclip",
            Physics::Climbing => "climbing",
            Physics::Swimming => "swimming",
        };
        format!(
            "WALK ({state}, {:.0} uu/s) (V to fly) | WASD move | Space jump | Shift walk slowly",
            b.velocity.truncate().length()
        )
    };
    text.0 .0 = format!(
        "{}\n{:.0} fps\npos {:.1} {:.1} {:.1} m\n{} tris, {} textures ({} missing)\n{}\nClick = capture mouse | Esc release | Right-click use | Q/E lean | C crouch | PgUp debug unlock{} | Ins spawn | Home keys | PgDn hide",
        level.0.name,
        1.0 / time.delta_secs().max(1e-4),
        p.x, p.y, p.z,
        s.triangles, s.textures, s.missing_textures,
        mode,
        if player.unlock_all { " ON" } else { "" }
    );
}

/// Scroll auto-panning surfaces and ripple the water (the original animates water procedurally).
fn uv_scroll(
    time: Res<Time>,
    scroll: Res<UvScroll>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut bsp_mats: ResMut<Assets<shadowlight::BspMaterial>>,
) {
    let _prof = crate::prof::scope("main::uv_scroll");
    let t = time.elapsed_secs();
    for (h, speed) in &scroll.1 {
        if let Some(mut m) = bsp_mats.get_mut(h) {
            let off = *speed * t;
            m.base.uv_transform = bevy::math::Affine2::from_translation(Vec2::new(off.x.fract(), off.y.fract()));
        }
    }
    for (h, speed, water) in &scroll.0 {
        if let Some(mut m) = materials.get_mut(h) {
            let mut off = *speed * t;
            if *water {
                off += Vec2::new(
                    0.015 * (t * 0.7).sin() + t * 0.01,
                    0.015 * (t * 0.53).cos() + t * 0.02,
                );
            }
            m.uv_transform =
                bevy::math::Affine2::from_translation(Vec2::new(off.x.fract(), off.y.fract()));
        }
    }
}

fn hud_text(
    time: Res<Time>,
    mut hud: ResMut<movers::HudMessage>,
    mut text: Single<&mut Text, With<HudText>>,
) {
    let _prof = crate::prof::scope("main::hud_text");
    if hud.time_left > 0.0 {
        hud.time_left -= time.delta_secs();
        if text.0 != hud.text {
            text.0 = hud.text.clone();
        }
    } else if !text.0.is_empty() {
        text.0.clear();
    }
}
