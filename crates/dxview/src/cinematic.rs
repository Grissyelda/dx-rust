//! Cut scenes: the CameraPoint camera script (Engine CameraPoint, "DEUS_EX CNN"), the player held Paralyzed in
//! the letterbox (DeusExPlayer state Paralyzed, label Letterbox + CinematicWindow), and the intro's mission
//! script (MissionIntro: the forced 'Intro' conversation, then PostIntro).
//!
//! CameraPoints are placed in the editor and numbered (sequenceNum). The one numbered 0 starts the sequence when
//! the map begins: it takes the player out of the world and every point runs in turn, each evaluating one command
//! on the view (cut / smooth move to the point, push along the view, FOV, tilt, pan, roll, wait for a time or an
//! event, trigger an event) over timeSmooth with the 3a^2 - 2a^3 ease, then waiting timeWaitPost. A bParallel
//! point hands its change to the next point, which performs both at once; bRandom points pick among themselves.
//! The view is the player's location and ViewRotation (BaseEyeHeight is set to 0) and FOVAngle.

use crate::ai::GameFlags;
use crate::{FlyCam, LevelRes, Player, SkyCam};
use bevy::prelude::*;
use glam::DVec3;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use unreal_pkg::props::{find, Value};

const UU: f64 = std::f64::consts::PI / 32768.0;

static ACTIVE: AtomicBool = AtomicBool::new(false);
/// PHYS_Interpolating along a MapExit's camera path to the next map (state Interpolating: HUD hidden).
static TRANSIT: Mutex<Option<Transit>> = Mutex::new(None);
static TRANSIT_ON: AtomicBool = AtomicBool::new(false);
/// MapExit / Teleporter actors of this map.
static EXITS: Mutex<Vec<Exit>> = Mutex::new(Vec::new());
/// InterpolateTriggers of this map, and the actors they've sent along a path.
static ITRIGGERS: Mutex<Vec<ITrigger>> = Mutex::new(Vec::new());
static RIDERS: Mutex<Vec<Rider>> = Mutex::new(Vec::new());

#[derive(Clone, Debug)]
struct ITrigger {
    tag: String,
    event: String,
    touch: bool,
    once: bool,
    gone: bool,
    loc: DVec3,
    radius: f64,
    height: f64,
}

struct Rider {
    /// Reached a bEndOfPath point (InterpolationPoint.InterpolateEnd): stopped where it is (Vehicles are destroyed).
    done: bool,
    actor: usize,
    pts: Vec<crate::interp::Point>,
    it: crate::interp::Interp,
}

/// Events for MapExits this frame (devices.rs dispatch).
static EXIT_EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Transit {
    pts: Vec<crate::interp::Point>,
    it: crate::interp::Interp,
    next_map: String,
}

#[derive(Clone, Debug)]
struct Exit {
    tag: String,
    /// MapExit.DestMap, or a Teleporter's URL ("map#tag", or a local teleporter's tag).
    dest: String,
    teleporter: bool,
    transition: bool,
    path_tag: String,
    touch: bool,
    /// Touchable again: the player arrived standing in it (a "map#tag" arrival at a teleporter) and must step
    /// out first, or he'd bounce straight back.
    armed: bool,
    loc: DVec3,
    radius: f64,
    height: f64,
}

/// A level event (devices.rs dispatch): MapExits tagged with it send the player on (MapExit.Trigger).
pub fn event(name: &str, on: bool) {
    if on {
        if let Ok(mut q) = EXIT_EVENTS.lock() {
            q.push(name.to_string());
        }
    }
}

/// The player rides a map exit's camera path (DeusExPlayer state Interpolating).
pub fn transit() -> bool {
    TRANSIT_ON.load(Ordering::Relaxed)
}

/// A CameraPoint cut scene's letterbox is up.
pub fn letterbox() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}
static STATE: Mutex<Option<Cine>> = Mutex::new(None);
/// Fire or Escape during the cut scene (DeusExPlayer.Paralyzed.Fire -> ShowMainMenu -> PostIntro).
static SKIP: AtomicBool = AtomicBool::new(false);
/// The intro has played: waiting on its last frame for the player to go on.
static INTRO_WAIT: AtomicBool = AtomicBool::new(false);

/// A cut scene holds the screen (player paralyzed in the letterbox, or riding a map exit's path): HUD hidden,
/// input off, AI blind to him.
pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed) || TRANSIT_ON.load(Ordering::Relaxed)
}

/// The cut scene's FOVAngle while one runs.
pub fn fov() -> Option<f32> {
    if !letterbox() {
        return None;
    }
    STATE.lock().ok().and_then(|s| s.as_ref().map(|c| c.fov as f32))
}

pub fn reset_level() {
    // MissionIntro.PreTravel: the sound volume back.
    crate::sound::set_instant_sound_volume(None);
    ACTIVE.store(false, Ordering::Relaxed);
    TRANSIT_ON.store(false, Ordering::Relaxed);
    SKIP.store(false, Ordering::Relaxed);
    INTRO_WAIT.store(false, Ordering::Relaxed);
    for q in [&EXIT_EVENTS] {
        if let Ok(mut q) = q.lock() {
            q.clear();
        }
    }
    if let Ok(mut t) = TRANSIT.lock() {
        *t = None;
    }
    if let Ok(mut e) = EXITS.lock() {
        e.clear();
    }
    if let Ok(mut e) = ITRIGGERS.lock() {
        e.clear();
    }
    if let Ok(mut e) = RIDERS.lock() {
        e.clear();
    }
    if let Ok(mut s) = STATE.lock() {
        *s = None;
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Cmd {
    Move,
    Push,
    Fov,
    Tilt,
    Pan,
    Roll,
    Wait,
    Trigger,
}

#[derive(Clone, Debug)]
struct Point {
    seq: i32,
    cmd: Cmd,
    value: f64,
    event: String,
    smooth: f64,
    wait_post: f64,
    parallel: bool,
    random: bool,
    random_count: i32,
    post_random: i32,
    loc: DVec3,
    rot: [f64; 3],
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    /// Executing the command (bTickReady), timer in seconds.
    Run,
    /// The Next label: sleeping timeWaitPost (a parallel point keeps executing meanwhile).
    Post,
    /// CAMCMD_WAIT for an event (Tag = eventName, asleep until triggered).
    WaitEvent,
}

#[derive(Clone, Debug)]
struct Running {
    i: usize,
    phase: Phase,
    tick_ready: bool,
    cur: f64,
    post_left: f64,
    start: (DVec3, [f64; 3], f64),
    end: (DVec3, [f64; 3], f64),
    random_remain: i32,
    continue_at: Option<usize>,
}

struct Cine {
    points: Vec<Point>,
    running: Vec<Running>,
    loc: DVec3,
    rot: [f64; 3],
    fov: f64,
    /// MissionIntro: has the Intro conversation started.
    intro_started: bool,
    intro: bool,
    timer: f32,
}

fn f(p: &[unreal_pkg::props::Property], n: &str) -> f64 {
    match find(p, n) {
        Some(Value::Float(v)) => *v as f64,
        Some(Value::Int(v)) => *v as f64,
        _ => 0.0,
    }
}

fn load_points(level: &dx_level::Level) -> Vec<Point> {
    let mut v: Vec<Point> = level
        .actors
        .iter()
        .filter(|a| a.class == "CameraPoint")
        .map(|a| {
            let p = &a.props;
            let cmd = match find(p, "Cmd").or_else(|| find(p, "cmd")) {
                Some(Value::Byte(b)) => *b,
                _ => 0,
            };
            let b = |n: &str| matches!(find(p, n), Some(Value::Bool(true)));
            let event = match find(p, "eventName") {
                Some(Value::Name(n)) | Some(Value::Str(n)) => n.clone(),
                _ => String::new(),
            };
            let l = a.location();
            let rot = match find(p, "Rotation") {
                Some(Value::Rotator(r)) => [r[0] as f64, r[1] as f64, r[2] as f64],
                _ => [0.0; 3],
            };
            Point {
                seq: match find(p, "sequenceNum") {
                    Some(Value::Int(n)) => *n,
                    _ => 0,
                },
                cmd: match cmd {
                    1 => Cmd::Push,
                    2 => Cmd::Fov,
                    3 => Cmd::Tilt,
                    4 => Cmd::Pan,
                    5 => Cmd::Roll,
                    6 => Cmd::Wait,
                    7 => Cmd::Trigger,
                    _ => Cmd::Move,
                },
                value: f(p, "Value").max(f(p, "value")),
                event: if event.eq_ignore_ascii_case("None") { String::new() } else { event },
                smooth: f(p, "timeSmooth"),
                wait_post: f(p, "timeWaitPost"),
                parallel: b("bParallel"),
                random: b("bRandom"),
                random_count: match find(p, "randomCount") {
                    Some(Value::Int(n)) => *n,
                    _ => 0,
                },
                post_random: match find(p, "postRandomNum") {
                    Some(Value::Int(n)) => *n,
                    _ => 0,
                },
                loc: DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64),
                rot,
            }
        })
        .collect();
    // InitAllPoints links each point to the next higher sequence number.
    v.sort_by_key(|p| p.seq);
    v
}

/// Shortest way round for a rotator component difference (CameraPoint.Running.Tick).
fn wrap(d: f64) -> f64 {
    if d >= 32768.0 {
        d - 65536.0
    } else if d <= -32768.0 {
        d + 65536.0
    } else {
        d
    }
}

fn forward(rot: [f64; 3]) -> DVec3 {
    let (p, y) = (rot[0] * UU, rot[1] * UU);
    DVec3::new(p.cos() * y.cos(), p.cos() * y.sin(), p.sin())
}

impl Cine {
    /// WakeUp: the point starts its command from the view as it is now.
    fn wake(&mut self, i: usize, prev: Option<Running>, ev: &mut Vec<(String, bool)>) {
        if i >= self.points.len() {
            return;
        }
        self.running.retain(|r| r.i != i);
        let pt = self.points[i].clone();
        let start = (self.loc, self.rot, self.fov);
        if std::env::var("DXVIEW_CAMLOG").is_ok() {
            println!("CAMLOG wake seq {} cmd {:?} value {} smooth {} loc {:.0?} rot {:.0?} fov {:.1}", pt.seq, pt.cmd, pt.value, pt.smooth, self.loc, self.rot, self.fov);
        }
        let mut end = start;
        let mut phase = Phase::Run;
        match pt.cmd {
            Cmd::Move => {
                end.0 = pt.loc;
                end.1 = pt.rot;
            }
            Cmd::Push => end.0 += forward(self.rot) * pt.value,
            Cmd::Fov => end.2 += pt.value,
            Cmd::Tilt => end.1[0] += pt.value,
            Cmd::Pan => end.1[1] += pt.value,
            Cmd::Roll => end.1[2] += pt.value,
            Cmd::Wait => {
                if !pt.event.is_empty() {
                    phase = Phase::WaitEvent;
                }
            }
            Cmd::Trigger => {
                if !pt.event.is_empty() {
                    println!("CAMERA trigger {}", pt.event);
                    ev.push((pt.event.clone(), true));
                }
            }
        }
        // Running in parallel with the previous point: its change is added to ours.
        if let Some(pi) = prev.as_ref() {
            end.0 += pi.end.0 - pi.start.0;
            for k in 0..3 {
                end.1[k] += pi.end.1[k] - pi.start.1[k];
            }
            end.2 += pi.end.2 - pi.start.2;
        }
        let (random_remain, continue_at) = prev.as_ref().map_or((-1, None), |r| (r.random_remain, r.continue_at));
        self.running.push(Running {
            i,
            phase,
            tick_ready: phase == Phase::Run,
            cur: 0.0,
            post_left: 0.0,
            start,
            end,
            random_remain,
            continue_at,
        });
        if pt.parallel {
            // Begin: bParallel -> nextPoint.prevPoint = Self; Next(). With no post delay the Next label wakes the
            // next point at once, in the same frame, before this point's Tick has moved the view: the next point
            // starts from the same view and carries this one's change (waking it a frame later, after an instant
            // change had been applied, counted the change twice - the intro ran at FOV 149 from the hooker scene on).
            self.next(i);
            if pt.wait_post <= 0.0 {
                self.finish_post(i, ev);
            }
        }
    }

    /// Next(): to the Next label (post delay, then the next point).
    fn next(&mut self, i: usize) {
        if let Some(r) = self.running.iter_mut().find(|r| r.i == i) {
            if !self.points[i].parallel {
                r.tick_ready = false;
            }
            r.phase = Phase::Post;
            r.post_left = self.points[i].wait_post;
        }
    }

    /// After the post delay: wake the next point (or a random one); this one goes back to sleep (Idle) - a
    /// parallel point's own change stops here, the next point carries it.
    fn finish_post(&mut self, i: usize, ev: &mut Vec<(String, bool)>) {
        let pt = self.points[i].clone();
        let Some(k) = self.running.iter().position(|r| r.i == i) else { return };
        let me = self.running.remove(k);
        if pt.random && !pt.parallel {
            let (mut remain, mut cont) = (me.random_remain, me.continue_at);
            if remain == -1 {
                remain = pt.random_count;
                cont = self.points.iter().position(|p| p.seq == pt.post_random);
            }
            if remain == 0 {
                if let Some(c) = cont {
                    self.wake(c, None, ev);
                }
            } else {
                let randoms: Vec<usize> = (0..self.points.len()).filter(|&k| self.points[k].random).collect();
                if !randoms.is_empty() {
                    let k = randoms[(crate::sound::frand() * randoms.len() as f32) as usize % randoms.len()];
                    self.wake(k, None, ev);
                    if let Some(rr) = self.running.iter_mut().find(|x| x.i == k) {
                        rr.random_remain = remain - 1;
                        rr.continue_at = cont;
                    }
                }
            }
        } else if i + 1 < self.points.len() {
            let prev = pt.parallel.then_some(me);
            self.wake(i + 1, prev, ev);
        }
    }

    fn tick(&mut self, dt: f64, ev: &mut Vec<(String, bool)>) {
        let ids: Vec<usize> = self.running.iter().map(|r| r.i).collect();
        for i in ids {
            let Some(r) = self.running.iter().find(|r| r.i == i).cloned() else { continue };
            let pt = &self.points[i];
            if r.tick_ready {
                let cur = r.cur + dt;
                if cur >= pt.smooth {
                    (self.loc, self.rot, self.fov) = r.end;
                    if let Some(x) = self.running.iter_mut().find(|x| x.i == i) {
                        x.cur = cur;
                        if pt.parallel {
                            x.tick_ready = false;
                        }
                    }
                    if r.phase == Phase::Run {
                        self.next(i);
                    }
                } else {
                    let a = cur / pt.smooth.max(1e-9);
                    let beta = 3.0 * a * a - 2.0 * a * a * a;
                    let mut rot = r.start.1;
                    for k in 0..3 {
                        rot[k] = r.start.1[k] + beta * wrap(r.end.1[k] - r.start.1[k]);
                    }
                    self.rot = rot;
                    self.loc = r.start.0 + (r.end.0 - r.start.0) * beta;
                    self.fov = r.start.2 + beta * (r.end.2 - r.start.2);
                    if let Some(x) = self.running.iter_mut().find(|x| x.i == i) {
                        x.cur = cur;
                    }
                }
            }
            let Some(r) = self.running.iter().find(|r| r.i == i).cloned() else { continue };
            if r.phase == Phase::Post {
                let left = r.post_left - dt;
                if let Some(x) = self.running.iter_mut().find(|x| x.i == i) {
                    x.post_left = left;
                }
                if left <= 0.0 {
                    self.finish_post(i, ev);
                }
            }
        }
    }

    /// A WAIT point asleep on this event (its Tag) is triggered.
    fn trigger(&mut self, name: &str) {
        let hits: Vec<usize> = self
            .running
            .iter()
            .filter(|r| r.phase == Phase::WaitEvent && self.points[r.i].event.eq_ignore_ascii_case(name))
            .map(|r| r.i)
            .collect();
        for i in hits {
            self.next(i);
        }
    }
}

/// PreUpdate: Fire or Escape skips the cut scene (and isn't seen by the menus).
pub fn cinematic_input_system(mut keys: ResMut<ButtonInput<KeyCode>>, mut mouse: ResMut<ButtonInput<MouseButton>>) {
    if !active() {
        return;
    }
    let go_on = INTRO_WAIT.load(Ordering::Relaxed) && (keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space) || keys.just_pressed(KeyCode::NumpadEnter));
    if go_on || keys.just_pressed(KeyCode::Escape) || mouse.just_pressed(MouseButton::Left) {
        SKIP.store(true, Ordering::Relaxed);
        for k in [KeyCode::Escape, KeyCode::Enter, KeyCode::Space, KeyCode::NumpadEnter] {
            keys.clear_just_pressed(k);
        }
        mouse.clear_just_pressed(MouseButton::Left);
    }
}

/// MissionIntro.PostIntro: the new game the intro was started for, or back to the title.
fn post_intro() {
    match crate::loader::take_after_intro() {
        Some((ng, map)) => crate::loader::request(&map, crate::loader::Goal::NewGame(ng)),
        None => crate::loader::request("DX.dx", crate::loader::Goal::Title),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn cinematic_system(
    time: Res<Time<Real>>,
    level: Res<LevelRes>,
    mut player: ResMut<Player>,
    mut flags: ResMut<GameFlags>,
    mut events: ResMut<crate::devices::GameEvents>,
    mut cams: ParamSet<(
        Single<(&mut Transform, &mut FlyCam), Without<SkyCam>>,
        Single<&mut Transform, With<SkyCam>>,
        Query<
            (&mut crate::actors::Deco, &mut Transform, &crate::actors::MeshActorRef, &mut Visibility),
            Without<crate::weapons::ViewModelRoot>,
        >,
    )>,
    mut vm: Single<&mut Visibility, With<crate::weapons::ViewModelRoot>>,
    mut blockers: ResMut<crate::actors::Blockers>,
    mut map_gen: Local<u64>,
) {
    let _prof = crate::prof::scope("cinematic::cinematic_system");
    if crate::loader::new_map(&mut map_gen) {
        let points = load_points(&level.0);
        let mission = level
            .0
            .actors
            .iter()
            .find(|a| a.class == "DeusExLevelInfo")
            .and_then(|a| match find(&a.props, "missionNumber") {
                Some(Value::Int(n)) => Some(*n),
                _ => None,
            })
            .unwrap_or(0);
        let has_start = points.iter().any(|p| p.seq == 0);
        if let Ok(mut e) = EXITS.lock() {
            *e = load_exits(&level.0);
        }
        if let Ok(mut e) = ITRIGGERS.lock() {
            *e = load_itriggers(&level.0);
        }
        if has_start {
            println!("cinematic: {} camera points", points.len());
        }
        let eye = player.body.location + DVec3::new(0.0, 0.0, player.eye_height);
        let c = Cine {
            points,
            running: Vec::new(),
            loc: eye,
            rot: [0.0; 3],
            fov: crate::weapons::DEFAULT_FOV as f64,
            intro_started: false,
            intro: mission == 98,
            timer: 0.0,
        };
        let mut c = c;
        let mut ev = Vec::new();
        if has_start {
            // CameraPoint 0, Idle.Begin: the player goes Paralyzed (Letterbox), hidden, out of collision, eye 0.
            ACTIVE.store(true, Ordering::Relaxed);
            c.wake(0, None, &mut ev);
        } else {
            ACTIVE.store(false, Ordering::Relaxed);
        }
        events.0.extend(ev);
        if let Ok(mut s) = STATE.lock() {
            *s = Some(c);
        }
    }
    let evs: Vec<String> = EXIT_EVENTS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    // ---- InterpolateTrigger.SendActorOnPath: actors tagged with its Event ride the path named by their own Event
    {
        let pl = player.body.location;
        let (pr, phh) = (player.params.radius, player.params.height);
        let mut fire: Vec<String> = Vec::new();
        if let Ok(mut trs) = ITRIGGERS.lock() {
            for t in trs.iter_mut().filter(|t| !t.gone) {
                let by_event = evs.iter().any(|e| e.eq_ignore_ascii_case(&t.tag));
                let by_touch = t.touch
                    && !active()
                    && DVec3::new(pl.x - t.loc.x, pl.y - t.loc.y, 0.0).length() < t.radius + pr
                    && (pl.z - t.loc.z).abs() < t.height + phh;
                if by_event || by_touch {
                    fire.push(t.event.clone());
                    if t.once {
                        t.gone = true;
                    }
                }
            }
        }
        for ev in fire {
            let lvl = &level.0;
            for (ai, a) in lvl.actors.iter().enumerate() {
                if a.class == "InterpolationPoint" || !matches!(find(&a.props, "Tag"), Some(Value::Name(t)) if t.eq_ignore_ascii_case(&ev)) {
                    continue;
                }
                let path = match find(&a.props, "Event") {
                    Some(Value::Name(n)) => n.clone(),
                    _ => continue,
                };
                let pts = crate::interp::path(lvl, &path);
                if let Some(first) = crate::interp::at_position(&pts, 1) {
                    println!("cinematic: {} rides {path}", a.name);
                    let it = crate::interp::Interp::start(&pts, first);
                    if let Ok(mut r) = RIDERS.lock() {
                        r.retain(|x| x.actor != ai);
                        r.push(Rider { done: false, actor: ai, pts, it });
                    }
                }
            }
        }
        // PHYS_Interpolating riders: out of collision, placed along the path every frame.
        if let Ok(mut riders) = RIDERS.lock() {
            if !riders.is_empty() {
                let dt = time.delta_secs_f64().min(0.1);
                let owners: Vec<usize> = level.0.mesh_actors.iter().map(|m| m.actor).collect();
                let mut q = cams.p2();
                for r in riders.iter_mut() {
                    if r.done {
                        continue;
                    }
                    let ended = r.it.tick(&r.pts, dt);
                    // InterpolationPoint.InterpolateEnd: a bEndOfPath point ends the ride (the path's points loop
                    // back to Position 0 otherwise, and the helicopter took off again and again). Vehicles.
                    // Interpolating destroys the vehicle there; anything else drops (PHYS_Falling).
                    let end = ended.iter().any(|&i| r.pts.get(i).is_some_and(|p| p.end_of_path));
                    let vehicle = matches!(
                        level.0.actors[r.actor].class.as_str(),
                        "AttackHelicopter" | "BlackHelicopter" | "MiniSub" | "NYPoliceBoat" | "Van"
                    );
                    for (mut d, mut tf, mr, mut vis) in q.iter_mut() {
                        if owners.get(mr.0) != Some(&r.actor) {
                            continue;
                        }
                        if end {
                            r.done = true;
                            if vehicle {
                                *vis = Visibility::Hidden;
                                crate::sound::own_ambient(r.actor);
                            } else {
                                d.body.physics = imsim_core::body::BodyPhysics::Falling;
                            }
                            continue;
                        }
                        d.body.physics = imsim_core::body::BodyPhysics::None;
                        d.body.velocity = DVec3::ZERO;
                        d.floating = false;
                        d.body.location = r.it.loc - d.pre;
                        d.rotation = [r.it.rot[0] as i32, r.it.rot[1] as i32, r.it.rot[2] as i32];
                        d.spun = true;
                        tf.translation = Vec3::from(dx_level::ue_to_bevy(r.it.loc.as_vec3().to_array()));
                        if let Some(b) = d.blocker.and_then(|b| blockers.list.get_mut(b)) {
                            b.radius = 0.0;
                            b.height = 0.0;
                        }
                    }
                }
            }
        }
    }
    // ---- map exits: touched, or triggered by an event (MapExit.Touch / Trigger, Teleporter.Touch)
    if !crate::loader::busy() && !transit() {
        let pl = player.body.location;
        let (pr, ph) = (player.params.radius, player.params.height);
        let inside = |x: &Exit| {
            (DVec3::new(pl.x - x.loc.x, pl.y - x.loc.y, 0.0)).length() < x.radius + pr && (pl.z - x.loc.z).abs() < x.height + ph
        };
        let exits = {
            let mut e = EXITS.lock().unwrap_or_else(|e| e.into_inner());
            for x in e.iter_mut() {
                if !x.armed && !inside(x) {
                    x.armed = true;
                }
            }
            e.clone()
        };
        let hit = exits.iter().find(|x| {
            evs.iter().any(|e| !x.tag.is_empty() && e.eq_ignore_ascii_case(&x.tag))
                || (x.touch && x.armed && !letterbox() && inside(x))
        });
        if let Some(x) = hit.cloned() {
            load_map(&level.0, &x, &mut player);
        }
    }
    // ---- riding the camera path to the next map
    {
        let mut tg = TRANSIT.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = tg.as_mut() {
            let ended = t.it.tick(&t.pts, time.delta_secs_f64().min(0.1));
            let rot = t.it.rot;
            let d = forward(rot);
            let dir = Vec3::new(d.x as f32, d.z as f32, d.y as f32);
            let rotq = Transform::IDENTITY.looking_to(dir, Vec3::Y).rotation;
            {
                let mut cam = cams.p0();
                cam.0.translation = Vec3::from(dx_level::ue_to_bevy(t.it.loc.as_vec3().to_array()));
                cam.0.rotation = rotq;
            }
            cams.p1().rotation = rotq;
            player.body.location = t.it.loc;
            player.body.velocity = DVec3::ZERO;
            **vm = Visibility::Hidden;
            // InterpolateEnd at a bEndOfPath point: SendPlayer(NextMap). (Or the path ran out.)
            let done = ended.iter().any(|&i| t.pts[i].end_of_path) || t.it.target.is_none();
            if done && !crate::loader::busy() {
                let m = std::mem::take(&mut t.next_map);
                if !m.is_empty() {
                    crate::loader::request(&m, crate::loader::Goal::Travel(None));
                }
            }
        }
    }

    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(c) = g.as_mut() else { return };

    // MissionIntro: FirstFrame starts the Intro conversation (forced); Timer sends the player on once it's played.
    if c.intro {
        if !c.intro_started {
            if !c.intro_started && crate::consys::lock().is_some() {
                // InitStateMachine / FirstFrame: every flag deleted, PlayerTraveling set.
                flags.0 = Default::default();
                flags.0.set("PlayerTraveling", imsim_core::flags::FlagValue::Bool(true), 0);
                drop(g);
                let ok = crate::consys::start_forced(&mut flags.0, "Intro", "BobPage");
                // "turn down the sound so we can hear the speech" (put back in PreTravel).
                crate::sound::set_instant_sound_volume(Some(32.0));
                g = STATE.lock().unwrap_or_else(|e| e.into_inner());
                let Some(c2) = g.as_mut() else { return };
                c2.intro_started = true;
                if !ok {
                    eprintln!("cinematic: couldn't start the Intro conversation");
                }
            }
        }
        let Some(c) = g.as_mut() else { return };
        c.timer += time.delta_secs();
        if c.timer >= 1.0 {
            c.timer = 0.0;
            if flags.0.get_bool("Intro_Played") {
                // Ours: the intro holds on its last (black) frame with "Press Enter to continue" until the player
                // goes on (the original went straight to the new game's load).
                flags.0.set_bool("Intro_Played", false);
                INTRO_WAIT.store(true, Ordering::Relaxed);
                let line = if crate::gamepad::pad_in_use() { "Press A to continue" } else { "Press Enter to continue" };
                crate::classic_hud::quote_text(&[line.to_string()], 1.0e6);
            }
        }
    }
    let Some(c) = g.as_mut() else { return };
    if SKIP.swap(false, Ordering::Relaxed) && active() {
        if c.intro && !crate::loader::busy() {
            if INTRO_WAIT.swap(false, Ordering::Relaxed) {
                crate::classic_hud::quote_text(&[], 0.0);
            }
            drop(g);
            crate::sound::stop(crate::sound::PLAYER, crate::sound::slot::TALK);
            post_intro();
            return;
        }
    }
    if !letterbox() {
        return;
    }
    // Events the camera waits for.
    let pending: Vec<(String, bool)> = events.0.clone();
    for (n, on) in &pending {
        if *on {
            c.trigger(n);
        }
    }
    let mut ev = Vec::new();
    c.tick(time.delta_secs_f64().min(0.1), &mut ev);
    events.0.extend(ev);

    // The view: Player.SetLocation / ViewRotation (eye height 0). The body follows the camera, out of the world.
    player.body.location = c.loc;
    player.body.velocity = DVec3::ZERO;
    player.eye_height = 0.0;
    **vm = Visibility::Hidden;
    let d = forward(c.rot);
    let dir = Vec3::new(d.x as f32, d.z as f32, d.y as f32);
    let mut rotq = Transform::IDENTITY.looking_to(dir, Vec3::Y).rotation;
    if c.rot[2] != 0.0 {
        rotq = rotq * Quat::from_rotation_z(-(c.rot[2] * UU) as f32);
    }
    {
        let mut cam = cams.p0();
        cam.0.translation = Vec3::from(dx_level::ue_to_bevy(c.loc.as_vec3().to_array()));
        cam.0.rotation = rotq;
    }
    cams.p1().rotation = rotq;
}

fn load_exits(level: &dx_level::Level) -> Vec<Exit> {
    let s = |p: &[unreal_pkg::props::Property], n: &str| match find(p, n) {
        Some(Value::Str(v)) | Some(Value::Name(v)) => v.clone(),
        _ => String::new(),
    };
    level
        .actors
        .iter()
        .filter(|a| a.class == "MapExit" || a.class == "Teleporter")
        .filter_map(|a| {
            let p = &a.props;
            let teleporter = a.class == "Teleporter";
            let dest = if teleporter { s(p, "URL") } else { s(p, "DestMap") };
            if dest.is_empty() || matches!(find(p, "bEnabled"), Some(Value::Bool(false))) {
                return None;
            }
            let touch = !matches!(find(p, "bCollideActors"), Some(Value::Bool(false)));
            let l = a.location();
            let (r_d, h_d) = if teleporter { (18.0, 40.0) } else { (12.0, 15.0) };
            let num = |n: &str, d: f64| match find(p, n) {
                Some(Value::Float(v)) => *v as f64,
                _ => d,
            };
            Some(Exit {
                tag: s(p, "Tag"),
                dest,
                teleporter,
                transition: matches!(find(p, "bPlayTransition"), Some(Value::Bool(true))),
                path_tag: s(p, "cameraPathTag"),
                touch,
                armed: false,
                loc: DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64),
                radius: num("CollisionRadius", r_d),
                height: num("CollisionHeight", h_d),
            })
        })
        .collect()
}

/// MapExit.LoadMap / Teleporter.Touch.
fn load_map(level: &dx_level::Level, x: &Exit, player: &mut Player) {
    println!("cinematic: exit to {} ({})", x.dest, if x.teleporter { "teleporter" } else { "map exit" });
    if x.teleporter {
        if x.dest.contains('#') || x.dest.contains('/') {
            let (m, tag) = match x.dest.split_once('#') {
                Some((m, t)) => (m.to_string(), Some(t.to_string()).filter(|t| !t.is_empty())),
                None => (x.dest.clone(), None),
            };
            crate::loader::request(&m, crate::loader::Goal::Travel(tag));
        } else if let Some(d) = level.actors.iter().find(|a| {
            a.class == "Teleporter"
                && matches!(find(&a.props, "Tag"), Some(Value::Name(t)) if t.eq_ignore_ascii_case(&x.dest))
        }) {
            // A teleporter in this map: Accept puts the player there.
            let l = d.location();
            player.body.location = DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64);
            player.body.velocity = DVec3::ZERO;
        }
        return;
    }
    if x.transition {
        let pts = crate::interp::path(level, &x.path_tag);
        if let Some(first) = crate::interp::at_position(&pts, 1) {
            let it = crate::interp::Interp::start(&pts, first);
            if let Ok(mut t) = TRANSIT.lock() {
                *t = Some(Transit { pts, it, next_map: x.dest.clone() });
            }
            TRANSIT_ON.store(true, Ordering::Relaxed);
            return;
        }
    }
    crate::loader::request(&x.dest, crate::loader::Goal::Travel(None));
}

fn load_itriggers(level: &dx_level::Level) -> Vec<ITrigger> {
    level
        .actors
        .iter()
        .filter(|a| a.class == "InterpolateTrigger")
        .map(|a| {
            let p = &a.props;
            let nm = |n: &str| match find(p, n) {
                Some(Value::Name(v)) | Some(Value::Str(v)) => v.clone(),
                _ => String::new(),
            };
            let num = |n: &str, d: f64| match find(p, n) {
                Some(Value::Float(v)) => *v as f64,
                _ => d,
            };
            let l = a.location();
            let tag = nm("Tag");
            ITrigger {
                tag: if tag.is_empty() { "InterpolateTrigger".into() } else { tag },
                event: nm("Event"),
                touch: !matches!(find(p, "bCollideActors"), Some(Value::Bool(false))),
                once: matches!(find(p, "bTriggerOnceOnly"), Some(Value::Bool(true))),
                gone: false,
                loc: DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64),
                radius: num("CollisionRadius", 40.0),
                height: num("CollisionHeight", 40.0),
            }
        })
        .collect()
}
