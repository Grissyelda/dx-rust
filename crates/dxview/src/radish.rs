//! The Nameless Mod's Radish Adventure: Trestkon's side-scrolling platform game (Trestkon.platform, RadishHUD,
//! Radish / SuperRadish, RadishRotatorTrigger, platformmover, tnmmissionbonus).
//!
//! - A map whose mission script is tnmmissionbonus turns the platformer on (DXVIEW_RADISH=1 forces it on any map, for
//!   tests). Going in strips the inventory to the bare fists (ammo too), sets every limb to 20, hides the normal HUD
//!   and shows the radish one (lives, two health radishes, the radish count, the breath gauge under water).
//! - The camera looks at Trestkon from the side: his facing turned by yawchange (49152 = three quarters of a turn),
//!   level, 560 units back. Forward jumps (held: again on landing); left / right walk that way on screen, turning
//!   him round; mouse look is off. He is held on one line: Y at yAxis (X at xAxis once a RadishRotatorTrigger with
//!   bYAxis turns the course round a corner, which also sets the facing, the camera yaw and bReverseControls).
//!   JumpZ 600, full air control, no fall damage.
//! - Damage takes one health radish and gives 3 s of flickering immunity; with none left (or 'Crushed') he dies.
//!   Dying costs a life: with lives left the level starts over after 2.5 s, with none it's back to the title after 10.
//!   Every 100 radishes is a life; a SuperRadish counts ten. Saving and quick loading are off in the platformer.
//! - platformmovers ride back and forth for ever (open, close, open...) at their own opentime / closetime.
//!
//! Lives, health and the count travel (and are saved) with Trestkon (trestkon.rs's save part).
//!
//! Deviations: the pawns' own plane lock (TNMPawn / TNMRobot xAxis) and the hovering, spinning weapon pickups
//! aren't ported; the radish HUD's Trestkon silhouette (a second draw of him over the scene) isn't drawn.

use crate::actors::{Blockers, Deco};
use crate::LevelRes;
use bevy::prelude::*;
use glam::DVec3;
use std::sync::Mutex;
use unreal_pkg::props::{find, Value};

/// RadishDelay: seconds of immunity after a hit.
const IMMUNITY: f32 = 3.0;
/// addzoom(560): how far back the camera sits.
const ZOOM: f64 = 560.0;
/// Trestkon's lives at the start (Trestkon.lives default); read from the mod when it's there.
const LIVES: i32 = 3;

struct State {
    /// bplatformer.
    on: bool,
    lives: i32,
    /// radishHealth (2, 1, 0 hits left before the next one kills).
    health: i32,
    /// radishCounter.
    radishes: i32,
    default_lives: Option<i32>,
    /// Seconds of immunity left (0 = none) and the flicker counter / state.
    immune: f32,
    flick: u32,
    hidden: bool,
    /// The plane he's held in: bYAxis = X is fixed at x_axis, else Y is fixed at y_axis.
    y_axis: bool,
    x_axis: f64,
    y_axis_v: f64,
    reverse: bool,
    /// yawchange in radians (Unreal yaw: clockwise from above).
    yaw_change: f64,
    /// bbackwards: facing the other way along the line.
    backwards: bool,
    /// Set the view's facing to this Unreal yaw (radians) next frame (RadishRotatorTrigger).
    set_facing: Option<f64>,
    /// Seconds since dying in the platformer.
    dead_t: Option<f32>,
    /// HUDRadishWeapon: (weapon name, rounds left or "INF", seconds shown).
    weapon_msg: Option<(String, String, f32)>,
    /// The Y the pawns are held at (Trestkon.platform(true) hands them his yAxis once, on the way in).
    pawn_y: Option<f64>,
}

static STATE: Mutex<State> = Mutex::new(State {
    on: false,
    lives: LIVES,
    health: 2,
    radishes: 0,
    default_lives: None,
    immune: 0.0,
    flick: 0,
    hidden: false,
    y_axis: false,
    x_axis: 0.0,
    y_axis_v: 0.0,
    reverse: false,
    yaw_change: 0.0,
    backwards: false,
    set_facing: None,
    dead_t: None,
    weapon_msg: None,
    pawn_y: None,
});

fn lock() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The platformer is on.
pub fn active() -> bool {
    lock().on
}

/// Immunity flicker: Trestkon's body is hidden this frame.
pub fn hidden() -> bool {
    let s = lock();
    s.on && s.hidden
}

/// ResetPlayer: lives back to the default, two health radishes, none picked up.
pub fn new_game() {
    let mut s = lock();
    s.lives = s.default_lives.unwrap_or(LIVES);
    s.health = 2;
    s.radishes = 0;
    s.on = false;
}

/// (lives, health, radishes) for the save.
pub fn save() -> (i32, i32, i32) {
    let s = lock();
    (s.lives, s.health, s.radishes)
}

pub fn restore(lives: i32, health: i32, radishes: i32) {
    let mut s = lock();
    // An old save without the radish part: the defaults.
    if lives == 0 && health == 0 && radishes == 0 {
        s.lives = s.default_lives.unwrap_or(LIVES);
        s.health = 2;
        s.radishes = 0;
    } else {
        s.lives = lives;
        s.health = health;
        s.radishes = radishes;
    }
}

/// setlives: a life count also refills the health radishes.
fn set_lives(s: &mut State, l: i32) {
    s.lives = l;
    s.health = 2;
}

/// PickedUpRadish.
fn picked_up(s: &mut State) {
    s.radishes += 1;
    if s.radishes % 100 == 0 {
        let l = s.lives + 1;
        set_lives(s, l);
    }
}

/// Trestkon.TakeDamage's platformer branch. None = not in the platformer (normal damage); Some(true) = the hit
/// kills; Some(false) = nothing more happens to the body.
pub fn intercept(damage_type: &str) -> Option<bool> {
    let mut s = lock();
    if !s.on {
        return None;
    }
    if damage_type.eq_ignore_ascii_case("fell") || s.immune > 0.0 {
        return Some(false);
    }
    s.health -= 1;
    if s.health < 0 || damage_type.eq_ignore_ascii_case("Crushed") {
        return Some(true);
    }
    s.immune = IMMUNITY;
    s.flick = 0;
    drop(s);
    crate::sound::play_player("DeusExSounds.Player.MalePainSmall", 1.0, 1600.0);
    Some(false)
}

/// What the platformer does with this frame's keys (Trestkon.PlayerWalking.ProcessMove): `screen` is left / right on
/// screen (-1..1), `up` forward. Returns (turn round, walk speed along the facing 0..1, jump).
pub fn steer(screen: f32, up: bool) -> (bool, f32, bool) {
    let mut s = lock();
    let mut x = screen;
    if s.reverse {
        x = -x;
    }
    // The facing is "right" on screen unless he's turned round.
    let along = if s.backwards { -x } else { x };
    let turn = along < -0.01;
    if turn {
        s.backwards = !s.backwards;
        s.yaw_change += std::f64::consts::PI;
    }
    (turn, x.abs().min(1.0), up)
}

/// The view facing a RadishRotatorTrigger asked for (Unreal yaw, radians), once.
pub fn take_facing() -> Option<f64> {
    lock().set_facing.take()
}

/// Holds a position (and its velocity) on the course's line.
pub fn lock_plane(loc: &mut DVec3, vel: &mut DVec3) {
    let s = lock();
    if !s.on {
        return;
    }
    if s.y_axis {
        loc.x = s.x_axis;
        vel.x = 0.0;
    } else {
        loc.y = s.y_axis_v;
        vel.y = 0.0;
    }
}

/// RadishHUD.ShowRadishWeaponMessage: the weapon fired, with what it has left.
pub fn weapon_message(name: &str, amount: String) {
    let mut s = lock();
    if s.on {
        s.weapon_msg = Some((name.to_string(), amount, 0.0));
    }
}

/// HUDRadishWeapon's text for the HUD while it's up (5 s): (weapon name, rounds left).
pub fn weapon_shown() -> Option<(String, String)> {
    let s = lock();
    if !s.on {
        return None;
    }
    s.weapon_msg.as_ref().filter(|m| m.2 <= 5.0).map(|m| (m.0.clone(), m.1.clone()))
}

/// tnmmissionbonus.FirstFrame on the map's own loose weapons (TNMWeapon) and ammo: no velocity, PHYS_Rotating with a
/// fixed half turn a second, unlit at full glow; the weapons also hover (TNMWeapon.Tick with bRadishHover:
/// PrePivot.Z = CollisionHeight + amplitude x sin(2 x (time mod 360)) while nobody owns them - radish_system bobs
/// them when the map gives an amplitude).
pub fn level_fixup(classes: &mut unreal_pkg::classes::ClassDb, actors: &mut Vec<dx_level::actors::ActorInfo>) {
    let bonus = env_on!("DXVIEW_RADISH")
        || actors.iter().any(|a| {
            a.class == "LevelInfo"
                && matches!(find(&a.props, "Script"), Some(Value::Path(p) | Value::Str(p) | Value::Name(p))
                    if p.rsplit('.').next().is_some_and(|n| n.eq_ignore_ascii_case("tnmmissionbonus")))
        });
    if !bonus {
        return;
    }
    let set = |props: &mut Vec<unreal_pkg::props::Property>, name: &str, v: Value| {
        props.retain(|p| !(p.name.eq_ignore_ascii_case(name) && p.index == 0));
        props.push(unreal_pkg::props::Property { name: name.into(), index: 0, value: v });
    };
    for a in actors.iter_mut() {
        let anc = classes.ancestry(&a.class);
        let weapon = anc.iter().any(|c| c == "tnmweapon");
        let ammo = anc.iter().any(|c| c == "ammo");
        if !(weapon || ammo) || find(&a.props, "Owner").is_some_and(|o| !matches!(o, Value::Object(0))) {
            continue;
        }
        set(&mut a.props, "Velocity", Value::Vector([0.0; 3]));
        set(&mut a.props, "Physics", Value::Byte(5));
        set(&mut a.props, "bFixedRotationDir", Value::Bool(true));
        let rate = match find(&a.props, "RotationRate") {
            Some(Value::Rotator(r)) => [r[0], 32768, r[2]],
            _ => [0, 32768, 0],
        };
        set(&mut a.props, "RotationRate", Value::Rotator(rate));
        set(&mut a.props, "bUnlit", Value::Bool(true));
        set(&mut a.props, "AmbientGlow", Value::Byte(255));
        if weapon {
            let ch = match find(&a.props, "CollisionHeight") {
                Some(Value::Float(v)) => *v,
                _ => 0.0,
            };
            let pre = match find(&a.props, "PrePivot") {
                Some(Value::Vector(v)) => *v,
                _ => [0.0; 3],
            };
            set(&mut a.props, "PrePivot", Value::Vector([pre[0], pre[1], ch]));
            set(&mut a.props, "bRadishHover", Value::Bool(true));
        }
    }
}

/// TNMPawn.Tick / TNMRobot.Tick with bPlatformer: Y held at the line platform(true) gave them (never X, even past
/// a corner). Every TNMPawn / TNMRobot but fish and bRadishCinematic ones.
pub fn pawn_lock(actor: &dx_level::actors::ActorInfo, loc: &mut DVec3) {
    let Some(y) = ({
        let s = lock();
        if s.on { s.pawn_y } else { None }
    }) else {
        return;
    };
    let tnm = crate::classtree::is_a(&actor.class, "TNMPawn") || crate::classtree::is_a(&actor.class, "TNMRobot");
    if !tnm || crate::classtree::is_a(&actor.class, "Fishes") || flag(&actor.props, "bRadishCinematic") {
        return;
    }
    loc.y = y;
}

/// The side camera for an eye at `eye` facing Unreal yaw `facing` (radians): (where it is, which way it looks).
pub fn camera(eye: DVec3, facing: f64) -> (DVec3, DVec3) {
    let yc = lock().yaw_change;
    let a = facing + yc;
    let dir = DVec3::new(a.cos(), a.sin(), 0.0);
    (eye - dir * ZOOM, dir)
}

/// The radish HUD's numbers: (lives, health, radishes, breath % with the head under water).
pub fn hud_state(breath: Option<f32>) -> Option<(i32, i32, i32, Option<i32>)> {
    let s = lock();
    s.on.then(|| (s.lives, s.health, s.radishes, breath.map(|b| (b * 100.0) as i32)))
}

/// The map's mission script is tnmmissionbonus (or DXVIEW_RADISH forces the platformer).
fn wants_platform(lvl: &dx_level::Level) -> bool {
    if env_on!("DXVIEW_RADISH") {
        return true;
    }
    if !crate::tnmmode::active() {
        return false;
    }
    match lvl.level_info().and_then(|a| find(&a.props, "Script")) {
        Some(Value::Path(p) | Value::Str(p) | Value::Name(p)) => {
            p.rsplit('.').next().is_some_and(|n| n.eq_ignore_ascii_case("tnmmissionbonus"))
        }
        _ => false,
    }
}

fn num(props: &[unreal_pkg::props::Property], name: &str, d: f64) -> f64 {
    match find(props, name) {
        Some(Value::Int(v)) => *v as f64,
        Some(Value::Float(v)) => *v as f64,
        Some(Value::Byte(v)) => *v as f64,
        _ => d,
    }
}

fn flag(props: &[unreal_pkg::props::Property], name: &str) -> bool {
    matches!(find(props, name), Some(Value::Bool(true)))
}

/// A radish decoration: (actor, radishes it's worth, baseHeight, Amplitude).
#[derive(Clone, Copy)]
struct Pick {
    actor: usize,
    worth: u32,
    base: f64,
    amp: f64,
}

/// A RadishRotatorTrigger: (actor, where, radius, height, once).
#[derive(Clone, Copy)]
struct Rotator {
    actor: usize,
    at: DVec3,
    radius: f64,
    height: f64,
    once: bool,
    used: bool,
    inside: bool,
}

#[derive(Default)]
pub struct Course {
    gen: u64,
    picks: Vec<Pick>,
    rotators: Vec<Rotator>,
    /// platformmovers: (mover index, opentime, closetime).
    platforms: Vec<(usize, f64, f64)>,
    /// What the Radish class says when one is picked up (pickup_messages[]).
    messages: Vec<String>,
    seed: u64,
    /// Hovering weapons with an amplitude of their own: (actor, amplitude).
    hovers: Vec<(usize, f64)>,
}

fn classify(class: &str) -> Option<u32> {
    if class.eq_ignore_ascii_case("SuperRadish") || crate::classtree::is_a(class, "SuperRadish") {
        Some(10)
    } else if class.eq_ignore_ascii_case("Radish") || crate::classtree::is_a(class, "Radish") {
        Some(1)
    } else {
        None
    }
}

/// Trestkon.platform(true): what going into the platformer does to him.
fn enter(inv: &mut imsim_core::inventory::Inventory, ph: &mut crate::ai::PlayerHealth, lvl: &dx_level::Level) {
    // Everything displayable but the fists goes, and every round of ammo.
    let mut i = 0;
    while i < inv.items.len() {
        if inv.items[i].def.class.eq_ignore_ascii_case("WeaponFists") {
            i += 1;
        } else {
            inv.remove(i);
        }
    }
    inv.ammo.clear();
    for p in ph.parts.iter_mut() {
        *p = 20.0;
    }
    ph.set_parts_total();
    // tnmmissionbonus.FirstFrame: the line runs through the PlayerStart.
    let start = lvl.actors.iter().find(|a| a.class == "PlayerStart").map(|a| a.location());
    let mut s = lock();
    if let Some(p) = start {
        s.x_axis = p[0] as f64;
        s.y_axis_v = p[1] as f64;
    }
    s.pawn_y = Some(s.y_axis_v);
    s.yaw_change = 49152.0 / 65536.0 * std::f64::consts::TAU;
    s.backwards = false;
    s.y_axis = false;
    s.reverse = false;
    s.immune = 0.0;
    s.hidden = false;
    println!("TNM radish adventure on (lives {}, radishes {})", s.lives, s.radishes);
}

/// The platformer's per-frame part: going in and out with the map, radishes (bob, touch), the rotator triggers,
/// the platforms, immunity and the lives when he dies.
#[allow(clippy::too_many_arguments)]
pub fn radish_system(
    time: Res<Time>,
    level: Res<LevelRes>,
    blockers: Res<Blockers>,
    player: Res<crate::Player>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut inv: ResMut<crate::pickups::PlayerInventory>,
    mut ph: ResMut<crate::ai::PlayerHealth>,
    mut movers: Option<ResMut<crate::movers::Movers>>,
    mut decos: Query<(&Deco, &mut Transform)>,
    mut course: Local<Course>,
) {
    let lvl = &level.0;
    let dt = time.delta_secs().min(0.1);
    if crate::loader::new_map(&mut course.gen) {
        course.picks.clear();
        course.rotators.clear();
        course.platforms.clear();
        course.hovers.clear();
        if lock().default_lives.is_none() && crate::tnmmode::active() {
            let d = assets.classes.defaults("Trestkon");
            let l = num(&d, "lives", LIVES as f64) as i32;
            lock().default_lives = Some(l);
        }
        let want = wants_platform(lvl);
        {
            let mut s = lock();
            s.dead_t = None;
            s.set_facing = None;
            // platform(true) from outside the platformer starts the count again.
            if want && !s.on {
                s.radishes = 0;
            }
            s.on = want;
        }
        if want {
            enter(&mut inv.0, &mut ph, lvl);
        }
        if crate::tnmmode::active() || want {
            for (i, a) in lvl.actors.iter().enumerate() {
                if flag(&a.props, "bRadishHover") {
                    let amp = num(&a.props, "amplitude", 0.0);
                    if amp != 0.0 {
                        course.hovers.push((i, amp));
                    }
                }
                if let Some(worth) = classify(&a.class) {
                    course.picks.push(Pick {
                        actor: i,
                        worth,
                        base: num(&a.props, "baseHeight", 0.0),
                        amp: num(&a.props, "Amplitude", 8.0),
                    });
                    if course.messages.is_empty() {
                        course.messages = a
                            .props
                            .iter()
                            .filter(|p| p.name.eq_ignore_ascii_case("pickup_messages"))
                            .filter_map(|p| match &p.value {
                                Value::Str(t) if !t.is_empty() => Some(t.clone()),
                                _ => None,
                            })
                            .collect();
                    }
                } else if a.class.eq_ignore_ascii_case("RadishRotatorTrigger") {
                    let l = a.location();
                    course.rotators.push(Rotator {
                        actor: i,
                        at: DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64),
                        radius: num(&a.props, "CollisionRadius", 40.0),
                        height: num(&a.props, "CollisionHeight", 40.0),
                        once: flag(&a.props, "bTriggerOnceOnly"),
                        used: false,
                        inside: false,
                    });
                }
            }
            for (m, info) in lvl.movers.iter().enumerate() {
                let Some(a) = lvl.actors.get(info.actor) else { continue };
                if a.class.eq_ignore_ascii_case("platformmover") || crate::classtree::is_a(&a.class, "platformmover") {
                    let open = num(&a.props, "opentime", -1.0);
                    let close = num(&a.props, "closetime", -1.0);
                    course.platforms.push((m, open, close));
                }
            }
            course.seed = lvl.actors.len() as u64 ^ 0x9e37_79b9;
        }
    }

    // platformmover: FinishedOpening goes straight to closing and FinishedClosing to opening again, each at its
    // own move time when one is set.
    if let Some(mv) = movers.as_deref_mut() {
        for &(m, open, close) in &course.platforms {
            let (Some(info), Some(rt)) = (lvl.movers.get(m), mv.rt.get_mut(m)) else { continue };
            if rt.interpolating || rt.destroyed {
                continue;
            }
            let base = info.move_time as f64;
            if rt.key_num == 0 {
                crate::movers::move_to(info, rt, 1, if open > 0.0 { open } else { base });
            } else {
                crate::movers::move_to(info, rt, 0, if close > 0.0 { close } else { base });
            }
        }
    }

    // Radishes hover: PrePivot.Z = baseHeight + Amplitude x sin(2 x (time mod 360)).
    if !course.picks.is_empty() || !course.hovers.is_empty() {
        let t = time.elapsed_secs_f64();
        for (d, mut tf) in &mut decos {
            let Some(ma) = lvl.mesh_actors.get(d.mesh_actor) else { continue };
            // TNMWeapon with bRadishHover: PrePivot.Z = CollisionHeight (already in d.pre) + amplitude x sin.
            let bob = if let Some(&(_, amp)) = course.hovers.iter().find(|h| h.0 == ma.actor) {
                amp * ((t % 360.0) * 2.0).sin()
            } else {
                let Some(p) = course.picks.iter().find(|p| p.actor == ma.actor) else { continue };
                p.base + p.amp * ((t % 360.0) * 2.0).sin()
            };
            let draw = d.body.location + d.pre + DVec3::new(0.0, 0.0, bob);
            let v = Vec3::from(dx_level::ue_to_bevy([draw.x as f32, draw.y as f32, draw.z as f32]));
            if tf.translation != v {
                tf.translation = v;
            }
        }
    }

    let on = active();
    if !on {
        return;
    }
    if let Some(m) = lock().weapon_msg.as_mut() {
        m.2 += dt;
    }
    // Dev aid: DXVIEW_RADISH_DIE=<secs> takes the last health radish that long into the run (death / lives tests).
    if let Some(at) = env_val!("DXVIEW_RADISH_DIE").and_then(|v| v.parse::<f32>().ok()) {
        if !ph.dead && time.elapsed_secs() > at && time.elapsed_secs() - time.delta_secs() <= at {
            {
                let mut s = lock();
                s.health = 0;
                s.immune = 0.0;
            }
            ph.damage(1.0, "Shot", DVec3::ZERO);
        }
    }

    // Dying: a life goes at once (state Dying's Begin); then the level again, or the title with none left.
    if ph.dead {
        let mut s = lock();
        match s.dead_t.as_mut() {
            None => {
                s.dead_t = Some(0.0);
                let l = s.lives - 1;
                set_lives(&mut s, l);
                s.immune = 0.0;
                s.hidden = false;
                println!("TNM radish adventure: died, lives {}", s.lives);
            }
            Some(t) => {
                *t += dt;
                let t = *t;
                if s.lives < 0 && t >= 10.0 {
                    s.dead_t = Some(f32::NEG_INFINITY);
                    s.on = false;
                    drop(s);
                    crate::loader::request("DX.dx", crate::loader::Goal::Title);
                } else if s.lives >= 0 && t >= 2.5 {
                    s.dead_t = Some(f32::NEG_INFINITY);
                    drop(s);
                    crate::loader::request(&lvl.name, crate::loader::Goal::Restart);
                }
            }
        }
        return;
    }

    // Immunity: the body flickers (hidden / shown every fourth tick) until it runs out.
    {
        let mut s = lock();
        if s.immune > 0.0 {
            s.flick += 1;
            if s.flick >= 4 {
                s.hidden = !s.hidden;
                s.flick = 0;
            }
            s.immune -= dt;
            if s.immune <= 0.0 {
                s.immune = 0.0;
                s.hidden = false;
                s.flick = 0;
            }
        }
    }

    let p = player.body.location;
    let (pr, phh) = (player.params.radius, player.params.height);

    // RadishRotatorTrigger.Touch: the facing, the camera yaw, the axis and the controls from the trigger.
    for r in course.rotators.iter_mut() {
        if r.used {
            continue;
        }
        let inside = (r.at - p).truncate().length() <= r.radius + pr && (r.at.z - p.z).abs() <= r.height + phh;
        if inside && !r.inside {
            let Some(a) = lvl.actors.get(r.actor) else { continue };
            let yaw = match find(&a.props, "Rotation") {
                Some(Value::Rotator(rot)) => rot[1] as f64,
                _ => 0.0,
            };
            let cam_yaw = num(&a.props, "desiredCamYaw", 49152.0);
            let mut s = lock();
            s.set_facing = Some(yaw / 65536.0 * std::f64::consts::TAU);
            s.yaw_change = cam_yaw / 65536.0 * std::f64::consts::TAU;
            s.y_axis = flag(&a.props, "bYAxis");
            s.reverse = flag(&a.props, "bReverseControls");
            s.x_axis = r.at.x;
            s.y_axis_v = r.at.y;
            s.backwards = false;
            if r.once {
                r.used = true;
            }
        }
        r.inside = inside;
    }

    // Radish.Touch: picked up (one, or ten for a SuperRadish), GrayAttack and one of its lines.
    let mut taken = Vec::new();
    for c in &blockers.list {
        let Some(pk) = course.picks.iter().find(|k| k.actor == c.actor) else { continue };
        if crate::scriptworld::is_hidden(c.actor) {
            continue;
        }
        let d = (c.center - p).truncate().length();
        if d <= c.radius.max(1.0) + pr + 2.0 && (c.center.z - p.z).abs() <= c.height.max(1.0) + phh + 2.0 {
            taken.push((c.actor, pk.worth));
        }
    }
    for (actor, worth) in taken {
        {
            let mut s = lock();
            for _ in 0..worth {
                picked_up(&mut s);
            }
        }
        crate::sound::play_player("DeusExSounds.Animal.GrayAttack", 1.0, 1600.0);
        if !course.messages.is_empty() {
            course.seed = course.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let i = (course.seed >> 33) as usize % course.messages.len();
            crate::classic_hud::client_message(&course.messages[i]);
        }
        crate::scriptworld::mark_hidden(actor, true);
        crate::scriptworld::push(crate::scriptworld::ActorReq::Destroy(crate::scriptworld::ActorSel::tag(&format!(
            "#actor{actor}"
        ))));
        course.picks.retain(|k| k.actor != actor);
    }
}

/// A see-through copy of one of Trestkon's drawn parts (RadishHUD.DrawWindow's second draw of him).
#[derive(Component)]
pub struct RadishGhost;

/// A part of Trestkon's body that already has its RadishGhost.
#[derive(Component)]
pub struct Ghosted;

/// RadishHUD.DrawWindow: while the platformer is on and Trestkon isn't hidden (the immunity flicker) but wasn't
/// drawn last frame - something stands between him and the camera - he is drawn again over everything (ClearZ),
/// translucent, every skin swapped for Engine.VisionBlue: a blue outline of him through the wall.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn silhouette_system(
    mut commands: Commands,
    level: Res<LevelRes>,
    player: Res<crate::Player>,
    body: Res<crate::mirror::MirrorBodyRes>,
    children: Query<&Children>,
    parts: Query<
        (&Mesh3d, Option<&bevy::mesh::skinning::SkinnedMesh>, Option<&bevy::mesh::morph::MeshMorphWeights>, Option<&bevy::camera::visibility::RenderLayers>),
        (Without<RadishGhost>, Without<Ghosted>),
    >,
    mut ghosts: Query<&mut Visibility, With<RadishGhost>>,
    cams: Query<&Transform, (With<crate::FlyCam>, Without<crate::SkyCam>)>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut mats: ResMut<Assets<crate::scope::GlassMaterial>>,
    mut mat: Local<Option<Handle<crate::scope::GlassMaterial>>>,
) {
    let on = active();
    if on {
        if let Some(b) = body.0.as_ref() {
            if mat.is_none() {
                let tex = crate::weapons::image_for(&mut assets, &mut images, "Engine.VisionBlue");
                *mat = Some(mats.add(crate::scope::GlassMaterial {
                    base: StandardMaterial {
                        base_color_texture: tex,
                        unlit: true,
                        alpha_mode: AlphaMode::Add,
                        cull_mode: None,
                        double_sided: true,
                        fog_enabled: false,
                        ..default()
                    },
                    extension: crate::scope::OnTop {},
                }));
            }
            let m = mat.clone().unwrap();
            for e in children.iter_descendants(b.root) {
                let Ok((mesh, skin, morph, layers)) = parts.get(e) else { continue };
                let mut g = commands.spawn((
                    RadishGhost,
                    Mesh3d(mesh.0.clone()),
                    MeshMaterial3d(m.clone()),
                    Transform::default(),
                    Visibility::Hidden,
                    bevy::light::NotShadowCaster,
                ));
                if let Some(s) = skin {
                    g.insert(s.clone());
                }
                if let Some(w) = morph {
                    g.insert(w.clone());
                }
                if let Some(l) = layers {
                    g.insert(l.clone());
                }
                let g = g.id();
                commands.entity(e).insert(Ghosted).add_child(g);
            }
        }
    }
    // Drawn last frame: the camera sees his middle or his eyes.
    let mut show = false;
    if on && !hidden() && !player.body.location.is_nan() {
        if let Some(c) = cams.iter().next() {
            let u = dx_level::bevy_to_ue(c.translation.to_array());
            let from = DVec3::new(u[0] as f64, u[1] as f64, u[2] as f64);
            let mid = player.body.location;
            let eye = mid + DVec3::new(0.0, 0.0, player.eye_height);
            let clear = |to: DVec3| level.0.collision.trace(from, to, DVec3::ZERO).fraction >= 1.0;
            show = !clear(mid) && !clear(eye);
        }
    }
    let want = if show { Visibility::Inherited } else { Visibility::Hidden };
    for mut v in &mut ghosts {
        v.set_if_neq(want);
    }
}
