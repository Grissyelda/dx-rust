//! The Overwatch seat (MULTIPLAYER roadmap Phase 3, Modern; the room's "Overwatch" switch, needs the living level).
//!
//! In a Modern match every security computer of the level is a seat. Use one (nobody else at it, its power on) and its
//! console fills the screen:
//! - every camera of the level live, four at a time (click one to switch it on as yours, or off);
//! - the turrets: switch one on (it hunts your rivals, as a hacked one) or off;
//! - the doors near the seat: lock or unlock them;
//! - the rivals the cameras see right now (the host works that out): send the guards after one (he becomes wanted,
//!   and every guard goes to where he was seen), then wait `dispatch_cooldown`;
//! - the seat's lockdown sectors (lockdown.rs) and an intercepted message, when the match has killphrases.
//! The seated player's body stays where it is, exposed: any hit throws him out of the seat (counter "Exposure").
//! Rivals cut the seat's power (counter "PowerCut"): a multitool used on the computer for `sabotage_secs`, or an EMP
//! blast within the counter's radius; the operator is thrown out and nobody sits there for `out_secs`.
//!
//! The host decides who sits where and whose power is out (Seats), and tells the seated player what the cameras see.

use crate::classic_hud::HudLayer;
use crate::mech::{self, MechMsg};
use crate::uicanvas::{Canvas, HAlign, Style, VAlign};
use bevy::prelude::*;
use bevy::window::CursorOptions;
use glam::DVec3;
use std::collections::HashMap;
use std::sync::Mutex;

const W: i32 = 640;
const H: i32 = 480;
const FEED: (i32, i32) = (200, 150);
/// Where the four feeds sit on the console.
const FEEDS: [(i32, i32); 4] = [(10, 30), (216, 30), (10, 206), (216, 206)];
const SEAT_REACH: f64 = 140.0;

#[derive(Default)]
struct State {
    /// (computer actor, player) - as the host last said.
    seats: Vec<(u32, u8)>,
    /// (computer actor, seconds of power cut left).
    power: Vec<(u32, f32)>,
    /// What the cameras see for our seat, and our dispatch cooldown.
    seen: Vec<(u8, DVec3)>,
    cool: f32,
    /// Host.
    dispatch_cool: HashMap<u8, f32>,
    send_t: f32,
    seen_t: f32,
    sent: (Vec<(u32, u8)>, Vec<u32>),
    /// Host: door toggles from seated players (mover, lock).
    doors: Vec<(u16, bool)>,
    /// Host: EMP blasts to look at (centre, radius).
    emps: Vec<(DVec3, f64)>,
    /// Our multitool cut under way: (computer actor, seconds so far).
    sabotage: Option<(u32, f32)>,
    /// A seat asked for (opened when the host says it's ours).
    asked: Option<u32>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::default))
}

static OPEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The console is up (the game gets no input, the cursor is free).
pub fn open() -> bool {
    OPEN.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn enabled() -> bool {
    mech::on(|r| r.overwatch) && crate::living::active()
}

/// A security computer is a seat.
pub fn is_seat(class: &str) -> bool {
    class.eq_ignore_ascii_case("ComputerSecurity")
}

/// Our seat (its computer actor).
pub fn my_seat() -> Option<u32> {
    let me = crate::deathmatch::my_id();
    with(|s| s.seats.iter().find(|x| x.1 == me).map(|x| x.0))
}

fn power_out(actor: u32) -> f32 {
    with(|s| s.power.iter().find(|p| p.0 == actor).map_or(0.0, |p| p.1))
}

fn holder(actor: u32) -> Option<u8> {
    with(|s| s.seats.iter().find(|x| x.0 == actor).map(|x| x.1))
}

fn name_of(id: u8) -> String {
    if id == crate::deathmatch::my_id() {
        crate::travel::player_name()
    } else {
        crate::netplay::players().into_iter().find(|p| p.0 == id).map_or_else(|| format!("Player {id}"), |p| p.1)
    }
}

/// pickups: Use on a terminal. True when Overwatch took it (the computer's own screens don't open).
pub fn intercept(actor: usize, class: &str, multitool: bool, hud: &mut crate::movers::HudMessage) -> bool {
    if !enabled() || !is_seat(class) {
        return false;
    }
    let a = actor as u32;
    let out = power_out(a);
    let taken = holder(a).filter(|&h| h != crate::deathmatch::my_id());
    if multitool && taken.is_some() {
        // Counter "PowerCut": cutting the console's power from under its operator.
        with(|s| s.sabotage = Some((a, 0.0)));
        hud.text = "Cutting the console's power...".into();
        hud.time_left = 1.0;
        return true;
    }
    hud.time_left = 2.5;
    if out > 0.0 {
        hud.text = format!("No power. Back in {} s.", out.ceil() as i32);
    } else if let Some(h) = taken {
        hud.text = format!("{} is at this console. A multitool could cut its power.", name_of(h));
    } else {
        with(|s| s.asked = Some(a));
        mech::to_host(MechMsg::SeatAsk(a));
        hud.text = "Taking the Overwatch seat...".into();
        hud.time_left = 1.0;
    }
    true
}

/// Get up (Escape, a hit, death, a new life).
pub fn leave_seat() {
    if my_seat().is_some() {
        mech::to_host(MechMsg::SeatLeave);
        let me = crate::deathmatch::my_id();
        with(|s| s.seats.retain(|x| x.1 != me));
    }
    with(|s| {
        s.asked = None;
        s.sabotage = None;
    });
}

// ------------------------------------------------------------------------------------------------ host

fn broadcast_seats() {
    let (seats, power) = with(|s| (s.seats.clone(), s.power.clone()));
    mech::to_joiners(MechMsg::Seats(seats, power));
}

/// Host: `from` sits at `actor` (None: gets up).
pub fn host_seat(from: u8, actor: Option<u32>) {
    if !mech::host() {
        return;
    }
    let ok = match actor {
        None => {
            with(|s| s.seats.retain(|x| x.1 != from));
            true
        }
        Some(a) => {
            if !enabled() {
                false
            } else if power_out(a) > 0.0 {
                crate::netplay::tell(from, "No power at that console.");
                false
            } else if holder(a).is_some_and(|h| h != from) {
                crate::netplay::tell(from, "Somebody beat you to that console.");
                false
            } else {
                with(|s| {
                    s.seats.retain(|x| x.1 != from);
                    s.seats.push((a, from));
                });
                println!("OVERWATCH {} sits at {a}", name_of(from));
                true
            }
        }
    };
    if ok || actor.is_none() {
        broadcast_seats();
    }
}

/// Host: the seated player `from` sends the guards after rival `id`.
pub fn host_dispatch(from: u8, id: u8) {
    if !mech::host() || my_seat_of(from).is_none() {
        return;
    }
    let cool = with(|s| s.dispatch_cool.get(&from).copied().unwrap_or(0.0));
    if cool > 0.0 {
        return;
    }
    let at = with(|s| s.seen.iter().find(|x| x.0 == id).map(|x| x.1)).or_else(|| crate::netplay::remote_bodies().into_iter().find(|b| b.0 == id).map(|b| b.1));
    let at = at.or_else(|| (id == crate::deathmatch::my_id()).then(|| MY_POS.lock().map(|p| *p).unwrap_or_default()));
    let Some(at) = at else { return };
    with(|s| s.dispatch_cool.insert(from, mech::tuning().dispatch_cooldown));
    crate::living::raise(id, imsim_core::heat::HeatCause::Dispatched, 0.0, Some(at));
    // Every guard of the level goes to look where he was seen.
    for n in 0..16 {
        crate::ai::world_req(crate::ai::WorldReq::Hunt(crate::ai::Sel::Pick { class: None, tag: Some(format!("LivingGuard{n}")), bind: None }, at));
    }
    let line = format!("Guards sent after {}.", name_of(id));
    if from == crate::deathmatch::my_id() {
        crate::classic_hud::client_message(&line);
    } else {
        crate::netplay::tell(from, &line);
    }
    println!("OVERWATCH {} dispatched guards after {}", name_of(from), name_of(id));
}

fn my_seat_of(id: u8) -> Option<u32> {
    with(|s| s.seats.iter().find(|x| x.1 == id).map(|x| x.0))
}

/// Host: `from` cut this seat's power with a multitool.
pub fn host_sabotage(from: u8, actor: u32) {
    if !mech::host() {
        return;
    }
    cut_power(actor, Some(from));
}

fn cut_power(actor: u32, by: Option<u8>) {
    let secs = crate::counters::value("Overwatch", "PowerCut", "out_secs", 45.0);
    let thrown = with(|s| {
        s.power.retain(|p| p.0 != actor);
        s.power.push((actor, secs));
        let h = s.seats.iter().find(|x| x.0 == actor).map(|x| x.1);
        s.seats.retain(|x| x.0 != actor);
        h
    });
    if let Some(h) = thrown {
        let line = match by {
            Some(b) => format!("{} cut the power: you're out of the seat.", name_of(b)),
            None => "An EMP blast cut the power: you're out of the seat.".into(),
        };
        if h == crate::deathmatch::my_id() {
            crate::classic_hud::client_message(&line);
        } else {
            crate::netplay::tell(h, &line);
        }
    }
    println!("OVERWATCH power cut at {actor} for {secs} s");
    broadcast_seats();
}

/// Host: the seated player `from` locks or unlocks a door.
pub fn host_door(from: u8, mover: u16, lock: bool) {
    if mech::host() && my_seat_of(from).is_some() {
        with(|s| s.doors.push((mover, lock)));
    }
}

/// Host (security.rs): an EMP blast - seats within the counter's radius lose power.
pub fn emp(center: DVec3, radius: f64) {
    if enabled() && mech::host() {
        with(|s| s.emps.push((center, radius)));
    }
}

/// The host's seats and power cuts.
pub fn set_seats(seats: Vec<(u32, u8)>, power: Vec<(u32, f32)>) {
    with(|s| {
        s.seats = seats;
        s.power = power;
    });
}

/// What the cameras see for our seat (from the host).
pub fn set_seen(v: Vec<(u8, [f32; 3])>, cool: f32) {
    with(|s| {
        s.seen = v.into_iter().map(|(i, p)| (i, DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64))).collect();
        s.cool = cool;
    });
}

pub fn reset_level() {
    with(|s| *s = State::default());
}

static MY_POS: Mutex<DVec3> = Mutex::new(DVec3::ZERO);

// ------------------------------------------------------------------------------------------------ console

#[derive(Component)]
pub struct OwNode(u8);
#[derive(Component)]
pub struct OwFeedNode(usize);
#[derive(Component)]
pub struct OwFeedCam(usize);

#[derive(Clone, Debug, PartialEq)]
enum Act {
    Prev,
    Next,
    Cam(String, bool),
    Turret(String, bool),
    Door(usize, bool),
    DoorPage(i32),
    Dispatch(u8),
    Lockdown(u8),
    Mail,
    Leave,
}

#[derive(Default)]
pub struct Console {
    seat: Option<u32>,
    page: usize,
    door_page: usize,
    hp: f32,
    grabbed_before: bool,
    layers: [Option<(Entity, Handle<Image>)>; 2],
    feeds: [Option<(Entity, Handle<Image>)>; 4],
    feed_nodes: [Option<Entity>; 4],
    key: u64,
    over_t: f32,
    doors: Vec<(usize, String, f64)>,
    mail: Option<String>,
    status: String,
}

fn upload(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    layers: &mut Assets<HudLayer>,
    slot: &mut Option<(Entity, Handle<Image>)>,
    c: &Canvas,
    which: u8,
) {
    let (pw, ph) = ((c.w * c.s) as u32, (c.h * c.s) as u32);
    if let Some((_, h)) = slot.as_ref() {
        if let Some(mut im) = images.get_mut(h) {
            if im.width() == pw && im.height() == ph {
                im.data = Some(c.to_rgba8());
                return;
            }
        }
    }
    if let Some((e, _)) = slot.take() {
        commands.entity(e).despawn();
    }
    let im = images.add(Image::new(
        bevy::render::render_resource::Extent3d { width: pw, height: ph, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        c.to_rgba8(),
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
    ));
    let e = commands
        .spawn((
            OwNode(which),
            MaterialNode(layers.add(HudLayer::new(im.clone()))),
            Node { position_type: PositionType::Absolute, ..default() },
            Visibility::Hidden,
            Pickable::IGNORE,
            GlobalZIndex(if which == 0 { 57 } else { 59 }),
        ))
        .id();
    *slot = Some((e, im));
}

fn close(ui: &mut Console, commands: &mut Commands, cursor: &mut CursorOptions) {
    for (e, _) in ui.layers.iter_mut().filter_map(|l| l.take()) {
        commands.entity(e).despawn();
    }
    for (c, _) in ui.feeds.iter_mut().filter_map(|f| f.take()) {
        commands.entity(c).despawn();
    }
    for e in ui.feed_nodes.iter_mut().filter_map(|f| f.take()) {
        commands.entity(e).despawn();
    }
    if ui.seat.take().is_some() {
        cursor.grab_mode = if ui.grabbed_before { bevy::window::CursorGrabMode::Locked } else { bevy::window::CursorGrabMode::None };
        cursor.visible = !ui.grabbed_before;
    }
    OPEN.store(false, std::sync::atomic::Ordering::Relaxed);
}

/// A door worth listing: one with a Tag of its own, locked at the start, needing a key, or a door by the map's word.
fn doors_near(lvl: &dx_level::Level, at: DVec3) -> Vec<(usize, String, f64)> {
    use unreal_pkg::props::{find, Value};
    let mut seen_tags: Vec<String> = Vec::new();
    let mut v: Vec<(usize, String, f64)> = Vec::new();
    for (i, m) in lvl.movers.iter().enumerate() {
        let a = &lvl.actors[m.actor];
        if a.class.eq_ignore_ascii_case("ElevatorMover") {
            continue;
        }
        let p = &a.props;
        let is_door = matches!(find(p, "bIsDoor"), Some(Value::Bool(true)))
            || matches!(find(p, "bLocked"), Some(Value::Bool(true)))
            || matches!(find(p, "KeyIDNeeded"), Some(Value::Name(n)) if !n.is_empty() && n != "None")
            || (!m.tag.is_empty() && !m.tag.eq_ignore_ascii_case("DeusExMover"));
        if !is_door || matches!(find(p, "bBreakable"), Some(Value::Bool(true))) && find(p, "bIsDoor").is_none() {
            continue;
        }
        // Double doors are one entry.
        if !m.tag.is_empty() && !m.tag.eq_ignore_ascii_case("DeusExMover") {
            if seen_tags.iter().any(|t| t.eq_ignore_ascii_case(&m.tag)) {
                continue;
            }
            seen_tags.push(m.tag.clone());
        }
        let l = DVec3::new(m.location[0] as f64, m.location[1] as f64, m.location[2] as f64);
        let name = if m.tag.is_empty() || m.tag.eq_ignore_ascii_case("DeusExMover") { a.name.clone() } else { m.tag.replace('_', " ") };
        v.push((i, name, l.distance(at)));
    }
    v.sort_by(|a, b| a.2.total_cmp(&b.2));
    v.truncate(18);
    v
}

/// Where a point shows in a camera's view (0..1 across, 0..1 down), if in front of it.
fn project(cam: (DVec3, f64, f64), at: DVec3) -> Option<(f32, f32)> {
    let to_rad = |u: f64| u / 65536.0 * std::f64::consts::TAU;
    let (yaw, pitch) = (to_rad(cam.2), to_rad(cam.1));
    let f = DVec3::new(pitch.cos() * yaw.cos(), pitch.cos() * yaw.sin(), pitch.sin());
    let r = DVec3::new(-yaw.sin(), yaw.cos(), 0.0);
    let u = DVec3::new(-pitch.sin() * yaw.cos(), -pitch.sin() * yaw.sin(), pitch.cos());
    let d = at - cam.0;
    let z = d.dot(f);
    if z < 16.0 {
        return None;
    }
    let tan_h = (45f64).to_radians().tan();
    let tan_v = tan_h * FEED.1 as f64 / FEED.0 as f64;
    let x = d.dot(r) / z / tan_h;
    let y = d.dot(u) / z / tan_v;
    (x.abs() <= 1.0 && y.abs() <= 1.0).then(|| (((x + 1.0) / 2.0) as f32, ((1.0 - y) / 2.0) as f32))
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn overwatch_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<crate::LevelRes>,
    windows: Query<&Window>,
    mut cursor: Single<&mut CursorOptions>,
    binds: Res<crate::keybinds::Bindings>,
    (mut hud, mut assets, mut images, mut layers): (ResMut<crate::classic_hud::ClassicHud>, ResMut<crate::weapons::RuntimeAssets>, ResMut<Assets<Image>>, ResMut<Assets<HudLayer>>),
    (mut sec, mut movers, ph, player, mut inv, mut msg): (
        ResMut<crate::security::Security>,
        ResMut<crate::movers::Movers>,
        Res<crate::ai::PlayerHealth>,
        Res<crate::Player>,
        ResMut<crate::pickups::PlayerInventory>,
        ResMut<crate::movers::HudMessage>,
    ),
    mut nodes: Query<(&OwNode, &mut Node, &mut Visibility), Without<OwFeedNode>>,
    mut feed_nodes: Query<(&OwFeedNode, &mut Node, &mut Visibility, &mut ImageNode), Without<OwNode>>,
    mut feed_cams: Query<(&OwFeedCam, &mut Transform, &mut Camera)>,
    mut ui: Local<Console>,
) {
    let dt = time.delta_secs();
    let lvl = &level.0;
    if let Ok(mut p) = MY_POS.lock() {
        *p = player.body.location;
    }
    if !enabled() {
        if ui.seat.is_some() {
            close(&mut ui, &mut commands, &mut cursor);
        }
        return;
    }
    // ---- host: power cuts, EMP blasts, door toggles, what each seat's cameras see.
    if mech::host() {
        host_tick(dt, lvl, &sec, &mut movers, &player);
    }
    // ---- our multitool cut.
    if let Some((a, t)) = with(|s| s.sabotage) {
        let at = lvl.actors.get(a as usize).map(|x| {
            let l = x.location();
            DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64)
        });
        let near = at.is_some_and(|p| p.distance(player.body.location) < SEAT_REACH) && !ph.dead;
        let need = mech::tuning().sabotage_secs;
        if !near || holder(a).is_none() {
            with(|s| s.sabotage = None);
            msg.text = "Cut broken off.".into();
            msg.time_left = 1.5;
        } else if t + dt >= need {
            with(|s| s.sabotage = None);
            inv.0.use_one("Multitool");
            mech::to_host(MechMsg::Sabotage(a));
            msg.text = "Power cut.".into();
            msg.time_left = 2.0;
            crate::sound::play_player("DeusExSounds.Generic.Spark1", 0.8, 0.0);
        } else {
            with(|s| s.sabotage = Some((a, t + dt)));
            msg.text = format!("Cutting the console's power... {}%", ((t + dt) / need * 100.0) as i32);
            msg.time_left = 0.3;
        }
    }
    // ---- the console.
    let seat = my_seat();
    let Some(win) = windows.iter().next() else { return };
    if seat.is_none() || ph.dead {
        if ui.seat.is_some() {
            close(&mut ui, &mut commands, &mut cursor);
            if ph.dead {
                leave_seat();
            }
        }
        return;
    }
    let seat = seat.unwrap();
    if ui.seat != Some(seat) {
        if ui.seat.is_some() {
            close(&mut ui, &mut commands, &mut cursor);
        }
        ui.seat = Some(seat);
        ui.hp = ph.hp;
        ui.page = 0;
        ui.door_page = 0;
        ui.grabbed_before = cursor.grab_mode != bevy::window::CursorGrabMode::None;
        cursor.grab_mode = bevy::window::CursorGrabMode::None;
        cursor.visible = true;
        let at = lvl.actors.get(seat as usize).map_or(player.body.location, |a| {
            let l = a.location();
            DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64)
        });
        ui.doors = doors_near(lvl, at);
        ui.mail = None;
        OPEN.store(true, std::sync::atomic::Ordering::Relaxed);
        crate::sound::play_player("DeusExSounds.UserInterface.Menu_Press", 0.5, 0.0);
        println!("OVERWATCH console open at {seat}");
    }
    // Counter "Exposure": the operator's body is in the room - a hit throws him out.
    let hit = ph.hp < ui.hp - 0.5 && crate::counters::value("Overwatch", "Exposure", "kick_on_hit", 1.0) > 0.0;
    ui.hp = ph.hp;
    let away = lvl.actors.get(seat as usize).is_some_and(|a| {
        let l = a.location();
        DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64).distance(player.body.location) > SEAT_REACH * 2.0
    });
    if hit || away || binds.keys.just_pressed(KeyCode::Escape) {
        if hit {
            crate::classic_hud::client_message("You were hit: out of the seat!");
        }
        leave_seat();
        close(&mut ui, &mut commands, &mut cursor);
        return;
    }

    // ---- layout and drawing.
    let (s, d) = crate::hud::screen_scales(win, W, H);
    let sf = win.scale_factor();
    let k = d / sf;
    let origin = Vec2::new((win.width() - W as f32 * k) / 2.0, (win.height() - H as f32 * k) / 2.0);
    let mouse = win.cursor_position().map(|p| (p - origin) / k).map(|p| (p.x as i32, p.y as i32));
    let cams = crate::security::control::cameras(&sec, lvl);
    let pages = cams.len().div_ceil(4).max(1);
    ui.page = ui.page.min(pages - 1);
    let turrets = crate::security::control::turrets(&sec);
    let (seen, cool) = with(|s| (s.seen.clone(), s.cool));
    let me = crate::deathmatch::my_id();
    let sectors = crate::lockdown::sectors_for(lvl, seat as usize);
    if ui.mail.is_none() {
        ui.mail = crate::killphrase::seat_mail(seat as usize, lvl);
    }
    let room = lvl.actors.get(seat as usize).map(|a| a.name.clone()).unwrap_or_default();

    // Widgets: (x, y, w, h, act, label, enabled).
    let mut wid: Vec<(i32, i32, i32, i32, Act, String, bool)> = Vec::new();
    wid.push((W - 120, 6, 110, 16, Act::Leave, "LEAVE SEAT [ESC]".into(), true));
    wid.push((10, 384, 110, 16, Act::Prev, "< CAMERAS".into(), ui.page > 0));
    wid.push((306, 384, 110, 16, Act::Next, "CAMERAS >".into(), ui.page + 1 < pages));
    for (j, &(fx, fy)) in FEEDS.iter().enumerate() {
        if let Some(c) = cams.get(ui.page * 4 + j) {
            let on = c.active && c.owner.is_none_or(|o| o == me);
            wid.push((fx, fy, FEED.0, FEED.1, Act::Cam(c.tag.clone(), !on), String::new(), true));
        }
    }
    let col_x = 428;
    let mut y = 30;
    let mut rows: Vec<(i32, String, [u8; 3])> = Vec::new();
    rows.push((y, "TURRETS".into(), [150, 190, 220]));
    y += 16;
    for (tag, _, on, owner) in turrets.iter().take(5) {
        let mine = owner.is_some_and(|o| o == me);
        let state = match (on, owner) {
            (false, _) => "OFF".to_string(),
            (true, Some(o)) if *o == me => "YOURS".into(),
            (true, Some(o)) => name_of(*o).to_uppercase(),
            (true, None) => "LEVEL".into(),
        };
        rows.push((y, format!("{}  {state}", tag.replace('_', " ")), [200, 215, 225]));
        wid.push((W - 62, y, 52, 14, Act::Turret(tag.clone(), !mine), if mine { "OFF".into() } else { "TAKE".into() }, true));
        y += 16;
    }
    y += 6;
    rows.push((y, "DOORS".into(), [150, 190, 220]));
    wid.push((W - 62, y, 24, 14, Act::DoorPage(-1), "<".into(), ui.door_page > 0));
    wid.push((W - 34, y, 24, 14, Act::DoorPage(1), ">".into(), (ui.door_page + 1) * 5 < ui.doors.len()));
    y += 16;
    for (i, name, _) in ui.doors.iter().skip(ui.door_page * 5).take(5) {
        let locked = movers.rt.get(*i).is_some_and(|r| r.locked);
        rows.push((y, format!("{}  {}", name, if locked { "LOCKED" } else { "OPEN" }), [200, 215, 225]));
        wid.push((W - 62, y, 52, 14, Act::Door(*i, !locked), if locked { "UNLOCK".into() } else { "LOCK".into() }, true));
        y += 16;
    }
    y += 6;
    rows.push((y, "SEEN BY CAMERAS".into(), [150, 190, 220]));
    y += 16;
    if seen.is_empty() {
        rows.push((y, "nobody".into(), [110, 120, 130]));
        y += 16;
    }
    for (id, _) in seen.iter().take(4) {
        rows.push((y, name_of(*id), [240, 120, 100]));
        let label = if cool > 0.0 { format!("{}s", cool.ceil() as i32) } else { "DISPATCH".into() };
        wid.push((W - 72, y, 62, 14, Act::Dispatch(*id), label, cool <= 0.0));
        y += 16;
    }
    if !sectors.is_empty() {
        y += 6;
        rows.push((y, "LOCKDOWN".into(), [150, 190, 220]));
        y += 16;
        for (i, name, active, cool) in &sectors {
            let (label, ok) = if *active > 0.0 {
                (format!("ON {}s", active.ceil() as i32), false)
            } else if *cool > 0.0 {
                (format!("{}s", cool.ceil() as i32), false)
            } else {
                ("SEAL".to_string(), true)
            };
            rows.push((y, name.clone(), [200, 215, 225]));
            wid.push((W - 62, y, 52, 14, Act::Lockdown(*i), label, ok));
            y += 16;
        }
    }
    if ui.mail.is_some() {
        y += 6;
        rows.push((y, "MAIL".into(), [150, 190, 220]));
        y += 16;
        rows.push((y, "1 intercepted message".into(), [200, 215, 225]));
        wid.push((W - 62, y, 52, 14, Act::Mail, "READ".into(), true));
    }
    let hover = mouse.and_then(|(mx, my)| wid.iter().position(|w| w.6 && mx >= w.0 && mx < w.0 + w.2 && my >= w.1 && my < w.1 + w.3));

    // Clicks.
    if binds.buttons.just_pressed(MouseButton::Left) {
        if let Some(i) = hover {
            let act = wid[i].4.clone();
            crate::sound::play_player("DeusExSounds.UserInterface.Menu_Press", 0.4, 0.0);
            match act {
                Act::Leave => {
                    leave_seat();
                    close(&mut ui, &mut commands, &mut cursor);
                    return;
                }
                Act::Prev => ui.page = ui.page.saturating_sub(1),
                Act::Next => ui.page = (ui.page + 1).min(pages - 1),
                Act::Cam(tag, on) => crate::security::control::set_camera(&mut sec, &tag, on),
                Act::Turret(tag, on) => crate::security::control::set_turret(&mut sec, &tag, if on { 1 } else { 0 }),
                Act::Door(i, lock) => {
                    if mech::host() {
                        with(|s| s.doors.push((i as u16, lock)));
                    } else {
                        mech::to_host(MechMsg::Door(i as u16, lock));
                    }
                    ui.status = format!("Door {}.", if lock { "locked" } else { "unlocked" });
                }
                Act::DoorPage(dp) => ui.door_page = (ui.door_page as i32 + dp).max(0) as usize,
                Act::Dispatch(id) => {
                    mech::to_host(MechMsg::Dispatch(id));
                    with(|s| s.cool = mech::tuning().dispatch_cooldown);
                }
                Act::Lockdown(i) => crate::lockdown::ask(i),
                Act::Mail => {
                    if let Some(text) = crate::killphrase::read_seat_mail(seat as usize, lvl) {
                        ui.status = text;
                    }
                    ui.mail = None;
                }
            }
        }
    }
    with(|s| s.cool = (s.cool - dt).max(0.0));

    // Base layer: frame, labels, lists, buttons (redrawn when what it shows changes).
    let font = hud.font(&mut assets, "FontMenuSmall_DS");
    let head = hud.font(&mut assets, "FontMenuHeaders").or_else(|| font.clone());
    let (Some(font), Some(head)) = (font, head) else { return };
    let key = crate::util::fnv1a(&format!(
        "{s}{d}{:?}{:?}{hover:?}{}{}{:?}{}",
        rows,
        wid.iter().map(|w| (&w.4, &w.5, w.6)).collect::<Vec<_>>(),
        ui.page,
        ui.status,
        cams.iter().map(|c| (c.active, c.owner)).collect::<Vec<_>>(),
        room
    ));
    if key != ui.key {
        ui.key = key;
        let mut c = Canvas::new(W, H, s);
        c.fill(0, 0, W, H, Style::Normal, [9, 11, 15]);
        let frame = [60, 92, 120];
        for (x, y, w, h) in [(0, 0, W, 1), (0, H - 1, W, 1), (0, 0, 1, H), (W - 1, 0, 1, H), (0, 24, W, 1), (420, 24, 1, H - 48), (0, H - 24, W, 1)] {
            c.fill(x, y, w, h, Style::Normal, frame);
        }
        c.text(&head, 10, 4, 400, 18, HAlign::Left, VAlign::Center, [170, 220, 255], &format!("OVERWATCH  -  {}", room.to_uppercase()));
        c.text(&font, 150, 384, 140, 16, HAlign::Center, VAlign::Center, [150, 170, 185], &format!("CAMERAS {} / {}", ui.page + 1, pages));
        for (j, &(fx, fy)) in FEEDS.iter().enumerate() {
            c.fill(fx - 1, fy - 1, FEED.0 + 2, FEED.1 + 2, Style::Normal, frame);
            c.fill(fx, fy, FEED.0, FEED.1, Style::Normal, [0, 0, 0]);
            let label = match cams.get(ui.page * 4 + j) {
                Some(cv) => {
                    let st = match (cv.active, cv.owner) {
                        (false, _) => "OFF",
                        (true, Some(o)) if o == me => "YOURS",
                        (true, Some(_)) => "A RIVAL'S",
                        (true, None) => "LEVEL",
                    };
                    format!("{}  [{st}]", cv.tag.replace('_', " "))
                }
                None => "-".into(),
            };
            c.text(&font, fx, fy + FEED.1 + 2, FEED.0, 14, HAlign::Left, VAlign::Center, [180, 200, 215], &label);
        }
        for (y, text, col) in &rows {
            c.text(&font, col_x, *y, 150, 14, HAlign::Left, VAlign::Center, *col, text);
        }
        for (i, (x, y, w, h, _, label, ok)) in wid.iter().enumerate() {
            if label.is_empty() {
                continue;
            }
            let hot = hover == Some(i);
            let bg = if !ok { [24, 28, 34] } else if hot { [70, 110, 150] } else { [34, 52, 70] };
            c.fill(*x, *y, *w, *h, Style::Normal, bg);
            c.text(&font, *x, *y, *w, *h, HAlign::Center, VAlign::Center, if *ok { [230, 240, 250] } else { [110, 120, 130] }, label);
        }
        let status = if ui.status.is_empty() { "Click a feed to switch its camera on as yours, or off. Your body is still in the room." } else { &ui.status };
        c.text(&font, 10, H - 22, W - 20, 20, HAlign::Left, VAlign::Center, [170, 185, 195], status);
        upload(&mut commands, &mut images, &mut layers, &mut ui.layers[0], &c, 0);
    }
    // Overlay: brackets round the rivals each feed sees, ten times a second.
    ui.over_t -= dt;
    if ui.over_t <= 0.0 {
        ui.over_t = 0.1;
        let mut o = Canvas::new(W, H, s);
        for (j, &(fx, fy)) in FEEDS.iter().enumerate() {
            let Some(cv) = cams.get(ui.page * 4 + j) else { continue };
            for (id, at) in &seen {
                if let Some((u, v)) = project((cv.pos, cv.pitch, cv.yaw), *at + DVec3::Z * 10.0) {
                    let (x, y) = (fx + (u * FEED.0 as f32) as i32, fy + (v * FEED.1 as f32) as i32);
                    let red = [255, 70, 50];
                    for (bx, by, bw, bh) in [(x - 10, y - 14, 20, 1), (x - 10, y + 14, 20, 1), (x - 10, y - 14, 1, 28), (x + 10, y - 14, 1, 29)] {
                        let (bx, by) = (bx.clamp(fx, fx + FEED.0 - 1), by.clamp(fy, fy + FEED.1 - 1));
                        o.fill(bx, by, bw.min(fx + FEED.0 - bx), bh.min(fy + FEED.1 - by), Style::Normal, red);
                    }
                    o.text(&font, (x - 40).max(fx), (y + 16).min(fy + FEED.1 - 12), 80, 12, HAlign::Center, VAlign::Center, red, &name_of(*id));
                }
            }
        }
        upload(&mut commands, &mut images, &mut layers, &mut ui.layers[1], &o, 1);
    }
    for (_, mut node, mut vis) in nodes.iter_mut() {
        node.left = Val::Px(origin.x);
        node.top = Val::Px(origin.y);
        node.width = Val::Px(W as f32 * k);
        node.height = Val::Px(H as f32 * k);
        vis.set_if_neq(Visibility::Inherited);
    }
    // Feeds: a render-to-texture camera per panel, aimed as its security camera.
    for j in 0..4 {
        let cam = cams.get(ui.page * 4 + j).filter(|c| c.active);
        match cam {
            Some(cv) => {
                if ui.feeds[j].is_none() {
                    use bevy::render::render_resource::TextureFormat;
                    let img = images.add(Image::new_target_texture(320, 240, TextureFormat::Bgra8UnormSrgb, None));
                    let cam_e = commands
                        .spawn((
                            Camera3d::default(),
                            Camera { order: -40 + j as isize, clear_color: ClearColorConfig::Custom(Color::srgb(0.02, 0.03, 0.05)), ..default() },
                            bevy::camera::RenderTarget::Image(img.clone().into()),
                            Projection::Perspective(PerspectiveProjection {
                                fov: 2.0 * ((45f32.to_radians()).tan() * FEED.1 as f32 / FEED.0 as f32).atan(),
                                near: 24.0 * dx_level::METERS_PER_UNIT,
                                far: 4000.0,
                                ..default()
                            }),
                            Transform::default(),
                            bevy::core_pipeline::tonemapping::Tonemapping::None,
                            bevy::camera::Exposure { ev100: -0.263 },
                            Msaa::Off,
                            crate::computers::FeedCam,
                            OwFeedCam(j),
                        ))
                        .id();
                    ui.feeds[j] = Some((cam_e, img.clone()));
                    let node_e = commands
                        .spawn((OwFeedNode(j), ImageNode::new(img), Node { position_type: PositionType::Absolute, ..default() }, Visibility::Hidden, Pickable::IGNORE, GlobalZIndex(58)))
                        .id();
                    ui.feed_nodes[j] = Some(node_e);
                }
                let to_rad = |u: f64| (u / 65536.0 * std::f64::consts::TAU) as f32;
                let rot = Quat::from_rotation_y(-to_rad(cv.yaw) - std::f32::consts::FRAC_PI_2) * Quat::from_rotation_x(to_rad(cv.pitch));
                let eye = Vec3::from(dx_level::ue_to_bevy([cv.pos.x as f32, cv.pos.y as f32, cv.pos.z as f32]));
                for (fc, mut tf, mut camera) in feed_cams.iter_mut() {
                    if fc.0 == j {
                        tf.set_if_neq(Transform { translation: eye, rotation: rot, ..*tf });
                        if !camera.is_active {
                            camera.is_active = true;
                        }
                    }
                }
            }
            None => {
                if let Some((c, _)) = ui.feeds[j].take() {
                    commands.entity(c).despawn();
                }
                if let Some(e) = ui.feed_nodes[j].take() {
                    commands.entity(e).despawn();
                }
            }
        }
    }
    for (fnode, mut node, mut vis, _) in feed_nodes.iter_mut() {
        let (fx, fy) = FEEDS[fnode.0];
        node.left = Val::Px(origin.x + fx as f32 * k);
        node.top = Val::Px(origin.y + fy as f32 * k);
        node.width = Val::Px(FEED.0 as f32 * k);
        node.height = Val::Px(FEED.1 as f32 * k);
        vis.set_if_neq(Visibility::Inherited);
    }
}

/// Host, each frame: power cuts run down, EMP blasts cut power, seated players' door toggles happen, and every 0.5 s
/// each seat is told which rivals its cameras see.
fn host_tick(dt: f32, lvl: &dx_level::Level, sec: &crate::security::Security, movers: &mut crate::movers::Movers, player: &crate::Player) {
    let emps = with(|s| std::mem::take(&mut s.emps));
    if !emps.is_empty() {
        let r = crate::counters::value("Overwatch", "EMP", "cut_radius", 320.0) as f64;
        let cut: Vec<u32> = lvl
            .actors
            .iter()
            .enumerate()
            .filter(|(_, a)| is_seat(&a.class))
            .filter(|(_, a)| {
                let l = a.location();
                let p = DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64);
                emps.iter().any(|(c, rad)| p.distance(*c) <= r.max(*rad * 0.5))
            })
            .map(|(i, _)| i as u32)
            .collect();
        for a in cut {
            cut_power(a, None);
        }
    }
    for (i, lock) in with(|s| std::mem::take(&mut s.doors)) {
        let i = i as usize;
        if i >= movers.rt.len() {
            continue;
        }
        let tag = lvl.movers[i].tag.clone();
        for (j, info) in lvl.movers.iter().enumerate() {
            if j == i || (!tag.is_empty() && !tag.eq_ignore_ascii_case("DeusExMover") && info.tag.eq_ignore_ascii_case(&tag)) {
                movers.rt[j].locked = lock;
            }
        }
        println!("OVERWATCH door {} {}", lvl.movers[i].tag, if lock { "locked" } else { "unlocked" });
    }
    let (changed, send) = with(|s| {
        for c in s.dispatch_cool.values_mut() {
            *c = (*c - dt).max(0.0);
        }
        for p in &mut s.power {
            p.1 -= dt;
        }
        s.power.retain(|p| p.1 > 0.0);
        let now = (s.seats.clone(), s.power.iter().map(|p| p.0).collect::<Vec<u32>>());
        let changed = now != s.sent;
        s.sent = now;
        s.send_t -= dt;
        let send = changed || s.send_t <= 0.0;
        if send {
            s.send_t = 2.0;
        }
        (changed, send)
    });
    if changed || send {
        broadcast_seats();
    }
    let go = with(|s| {
        s.seen_t -= dt;
        if s.seen_t <= 0.0 {
            s.seen_t = 0.5;
            true
        } else {
            false
        }
    });
    if !go {
        return;
    }
    let me = crate::deathmatch::my_id();
    let mut everyone: Vec<(u8, DVec3)> = crate::netplay::remote_bodies();
    everyone.push((me, player.body.location));
    for (_, sitter) in with(|s| s.seats.clone()) {
        let seen: Vec<(u8, DVec3)> = everyone
            .iter()
            .filter(|(id, _)| *id != sitter && !(crate::deathmatch::team_mode() && crate::deathmatch::allied(sitter, *id)))
            // Radar transparency blinds the cameras (counter: eyes still see him).
            .filter(|(id, _)| !(if *id == me { crate::abilities::aug_active("AugRadarTrans") } else { crate::netplay::remote_aug(*id, "AugRadarTrans") }))
            .filter(|(_, p)| crate::security::control::cameras_see(sec, lvl, *p + DVec3::Z * 20.0, sitter))
            .copied()
            .collect();
        let cool = with(|s| s.dispatch_cool.get(&sitter).copied().unwrap_or(0.0));
        if sitter == me {
            with(|s| {
                s.seen = seen.clone();
                s.cool = cool;
            });
        } else {
            mech::to_player(sitter, MechMsg::Seen(seen.iter().map(|(i, p)| (*i, [p.x as f32, p.y as f32, p.z as f32])).collect(), cool));
        }
    }
}
