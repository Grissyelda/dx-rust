//! The Nameless Mod's staring movers and projectile spawn points (StaringMover, ProjectileSpawnPoint): brush movers
//! that turn to follow a target at their RotationRate, and the turrets among them that fire from the spawn points
//! they own - the harpoon and Alpha turrets in the city, the ABI exterior's Cerberus guns and SAM site.
//!
//! A staring mover looks for a target when it has none (the player, unless he runs Radar Transparency; or, with
//! bTargetNPCsOnly, a pawn of one of its enemy alliances), turns toward it while it is within min..maxViewDistance
//! (yaw only unless bAllowPitchRotation, pitch capped at maxPitch), drives its slave mover (SlaveMover) the same way,
//! and - as a turret with its turret part on - fires every fireAttemptTime seconds once the target is within
//! TurretFOV of where it faces (all its points at once, or one after another with SalvoFireDelay in the SalvoFire
//! state). Triggering it toggles staring and / or firing by its state (TriggerToggle / TriggerTurretOnly /
//! TriggerStaringOnly / SalvoFire); bAlarmActivated ones come on while an alarm sounds nearby (unless disableFlag is
//! set); EMP confuses an EMP-vulnerable one (it twitches about for 3..30 s).
//!
//! A spawn point rides the mover that owns it (ownerTurret, or the turret that assimilated it within
//! AssimilationRange) and fires its ProjectileClass along the turret's aim, with its speed and damage multipliers,
//! fire sound and cooldown (fireTime, burstAmount). Not here: point-defence lasers (bPointDefense, bHasLaser).

use bevy::prelude::*;
use glam::DVec3;
use std::sync::Mutex;
use unreal_pkg::props::{find, Property, Value};

use crate::ai::{Npc, NpcState};

fn fp(p: &[Property], n: &str, d: f64) -> f64 {
    match find(p, n) {
        Some(Value::Float(v)) => *v as f64,
        Some(Value::Int(v)) => *v as f64,
        Some(Value::Byte(v)) => *v as f64,
        _ => d,
    }
}
fn bp(p: &[Property], n: &str, d: bool) -> bool {
    match find(p, n) {
        Some(Value::Bool(v)) => *v,
        _ => d,
    }
}
fn np(p: &[Property], n: &str) -> String {
    match find(p, n) {
        Some(Value::Name(v)) if !v.eq_ignore_ascii_case("None") => v.clone(),
        Some(Value::Path(v)) => v.rsplit('.').next().unwrap_or("").to_string(),
        Some(Value::Str(v)) => v.clone(),
        _ => String::new(),
    }
}
fn rp(p: &[Property], n: &str) -> Option<[i32; 3]> {
    match find(p, n) {
        Some(Value::Rotator(r)) => Some(*r),
        _ => None,
    }
}

const U2R: f64 = std::f64::consts::PI / 32768.0;

/// Rotator(v): (pitch, yaw, 0).
pub fn rotator(v: DVec3) -> [i32; 3] {
    let yaw = v.y.atan2(v.x) / U2R;
    let pitch = v.z.atan2((v.x * v.x + v.y * v.y).sqrt()) / U2R;
    [(pitch.round() as i32).rem_euclid(65536), (yaw.round() as i32).rem_euclid(65536), 0]
}

/// Vector(r).
pub fn dir_of(r: [i32; 3]) -> DVec3 {
    let (p, y) = (r[0] as f64 * U2R, r[1] as f64 * U2R);
    DVec3::new(p.cos() * y.cos(), p.cos() * y.sin(), p.sin())
}

/// v >> r (Unreal's rotation of a vector by a rotator).
pub fn rotate(v: DVec3, r: [i32; 3]) -> DVec3 {
    let (p, y, rl) = (r[0] as f64 * U2R, r[1] as f64 * U2R, r[2] as f64 * U2R);
    let (sp, cp, sy, cy, sr, cr) = (p.sin(), p.cos(), y.sin(), y.cos(), rl.sin(), rl.cos());
    let x = DVec3::new(cp * cy, cp * sy, sp);
    let yv = DVec3::new(sr * sp * cy - cr * sy, sr * sp * sy + cr * cy, -sr * cp);
    let z = DVec3::new(-(cr * sp * cy + sr * sy), -(cr * sp * sy - sr * cy), cr * cp);
    x * v.x + yv * v.y + z * v.z
}

fn sub(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// |a - b| as Unreal's turret check works it out: (Abs(a - b) % 65536) folded into -32768..32767.
fn ang_diff(a: i32, b: i32) -> i32 {
    let d = (a - b).abs() % 65536;
    if d > 32767 {
        d - 65536
    } else {
        d
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Target {
    None,
    Player,
    /// A pawn, by blocker.
    Pawn(usize),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Toggle,
    TurretOnly,
    StaringOnly,
    Salvo,
    Other,
}

struct Turret {
    mover: usize,
    name: String,
    min_d: f64,
    max_d: f64,
    is_turret: bool,
    staring: bool,
    turret_on: bool,
    fire_time: f32,
    stop_when_disabled: bool,
    pitch_ok: bool,
    head: bool,
    fov: i32,
    child: Option<usize>,
    parent: Option<usize>,
    slave: bool,
    emp_vulnerable: bool,
    confused: bool,
    conf_t: f32,
    conf_dur: f32,
    salvo_delay: [f32; 16],
    /// SalvoFiring: the next point to fire and the wait before it.
    salvo: Option<(usize, f32)>,
    mode: Mode,
    target_disabled_robots: bool,
    rot_override: bool,
    npcs_only: bool,
    enemies: Vec<String>,
    target_class: String,
    require_zone: bool,
    zones: [String; 2],
    target_aim: [i32; 3],
    dynamic_aim: bool,
    fire_sound: Option<String>,
    alarm_activated: bool,
    alarm_was: bool,
    disable_flag: String,
    max_pitch: i32,
    fire_timer: f32,
    view: Target,
    spots: Vec<usize>,
}

struct Spot {
    tag: String,
    pos: DVec3,
    rot: [i32; 3],
    owner: Option<usize>,
    attach_rot: [i32; 3],
    base_rot: [i32; 3],
    attach_off: DVec3,
    class: String,
    speed_mult: f64,
    damage_mult: f32,
    homing: bool,
    sound: Option<String>,
    radius: f32,
    volume: f32,
    fire_time: f32,
    burst: i32,
    require_los: bool,
    fire_timer: f32,
    burst_count: i32,
}

#[derive(Default)]
struct State {
    built: bool,
    turrets: Vec<Turret>,
    spots: Vec<Spot>,
    /// Mover index -> turret.
    by_mover: std::collections::HashMap<usize, usize>,
    rand: u64,
}

impl State {
    fn frand(&mut self) -> f32 {
        self.rand = self.rand.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.rand >> 40) as f32) / (1u64 << 24) as f32
    }
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static EVENTS: Mutex<Vec<(String, bool)>> = Mutex::new(Vec::new());
static EMPS: Mutex<Vec<(usize, f32)>> = Mutex::new(Vec::new());
/// Script requests: fire every spawn point with this tag (ProjectileSpawnPoint.FireProjectile).
static FIRE_TAGS: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Script requests: a staring mover's DesiredRotation (by tag).
static TURN: Mutex<Vec<(String, [i32; 3])>> = Mutex::new(Vec::new());
/// Script requests by mover: a DesiredRotation for it and (Some) for its slave.
static TURN_M: Mutex<Vec<(usize, [i32; 3], Option<[i32; 3]>)>> = Mutex::new(Vec::new());
/// AmbientSound set by mission scripts, by mover (SoundRadius / SoundVolume of the mover).
static AMBIENT: Mutex<Vec<(usize, String, Option<f64>)>> = Mutex::new(Vec::new());
/// Last frame's staring movers: (tag, mover, rotation, at rest = Rotation == DesiredRotation).
static SNAP: Mutex<Vec<(String, usize, [i32; 3], bool)>> = Mutex::new(Vec::new());

/// The staring movers with this tag as of last frame: (mover, rotation, turned to its DesiredRotation).
pub fn with_tag(tag: &str) -> Vec<(usize, [i32; 3], bool)> {
    SNAP.lock()
        .map(|s| s.iter().filter(|x| x.0.eq_ignore_ascii_case(tag)).map(|x| (x.1, x.2, x.3)).collect())
        .unwrap_or_default()
}

/// Mission scripts: a DesiredRotation for a staring mover and, when given, for its slave.
pub fn turn_mover(mover: usize, rot: [i32; 3], child: Option<[i32; 3]>) {
    if let Ok(mut q) = TURN_M.lock() {
        q.push((mover, rot, child));
    }
}

/// Mission scripts: a staring mover's AmbientSound (None = quiet), with a SoundRadius set at the same time.
pub fn set_ambient(mover: usize, sound: Option<&str>, sound_radius: Option<f64>) {
    if let Ok(mut a) = AMBIENT.lock() {
        let keep = a.iter().find(|x| x.0 == mover).and_then(|x| x.2);
        a.retain(|x| x.0 != mover);
        if let Some(s) = sound {
            a.push((mover, s.to_string(), sound_radius.or(keep)));
        }
    }
}

/// Movers that are staring movers (for devices: their Trigger isn't a door's).
static STARING: Mutex<Vec<usize>> = Mutex::new(Vec::new());

pub fn reset_level() {
    if let Ok(mut s) = STATE.lock() {
        *s = None;
    }
    for q in [&EVENTS] {
        if let Ok(mut q) = q.lock() {
            q.clear();
        }
    }
    if let Ok(mut q) = STARING.lock() {
        q.clear();
    }
    if let Ok(mut q) = FIRE_TAGS.lock() {
        q.clear();
    }
    if let Ok(mut q) = TURN.lock() {
        q.clear();
    }
    if let Ok(mut q) = EMPS.lock() {
        q.clear();
    }
    if let Ok(mut q) = TURN_M.lock() {
        q.clear();
    }
    if let Ok(mut q) = AMBIENT.lock() {
        q.clear();
    }
    if let Ok(mut q) = SNAP.lock() {
        q.clear();
    }
}

pub fn is_staring(mover: usize) -> bool {
    STARING.lock().is_ok_and(|s| s.contains(&mover))
}

pub fn event(tag: &str, on: bool) {
    if let Ok(mut q) = EVENTS.lock() {
        if q.len() < 256 {
            q.push((tag.to_string(), on));
        }
    }
}

/// An EMP hit on mover `i`: true when a staring mover takes it as confusion (it isn't hurt).
pub fn emp(i: usize, damage: f32) -> bool {
    if !is_staring(i) {
        return false;
    }
    let vulnerable = STATE.lock().ok().and_then(|s| s.as_ref().and_then(|s| s.by_mover.get(&i).map(|&t| s.turrets[t].emp_vulnerable))).unwrap_or(false);
    if vulnerable {
        if let Ok(mut q) = EMPS.lock() {
            q.push((i, damage));
        }
    }
    vulnerable
}

/// Mission scripts: fire the spawn points with this tag.
pub fn fire_tag(tag: &str) {
    if let Ok(mut q) = FIRE_TAGS.lock() {
        q.push(tag.to_string());
    }
}

/// Mission scripts: set the DesiredRotation of the staring movers with this tag.
pub fn turn_tag(tag: &str, rot: [i32; 3]) {
    if let Ok(mut q) = TURN.lock() {
        q.push((tag.to_string(), rot));
    }
}

fn build(lvl: &dx_level::Level) -> State {
    let mut st = State { built: true, rand: 0x5eed_1234, ..Default::default() };
    for (mi, m) in lvl.movers.iter().enumerate() {
        let a = &lvl.actors[m.actor];
        if !crate::classtree::is_a(&a.class, "StaringMover") {
            continue;
        }
        let p = &a.props;
        let mut salvo_delay = [0.1f32; 16];
        for pr in p.iter().filter(|x| x.name.eq_ignore_ascii_case("SalvoFireDelay")) {
            if let (Some(slot), Value::Float(v)) = (salvo_delay.get_mut(pr.index as usize), &pr.value) {
                *slot = *v;
            }
        }
        let mode = match np(p, "InitialState").to_ascii_lowercase().as_str() {
            "triggertoggle" => Mode::Toggle,
            "triggerturretonly" => Mode::TurretOnly,
            "triggerstaringonly" => Mode::StaringOnly,
            "salvofire" => Mode::Salvo,
            _ => Mode::Other,
        };
        let snd = crate::sound::prop(p, "fireAttemptSound");
        let enemies = (1..=3).map(|k| np(p, &format!("EnemyAlliance{k}"))).filter(|s| !s.is_empty()).collect();
        st.by_mover.insert(mi, st.turrets.len());
        st.turrets.push(Turret {
            mover: mi,
            name: a.name.clone(),
            min_d: fp(p, "minViewDistance", 128.0),
            max_d: fp(p, "maxViewDistance", 1_000_000.0),
            is_turret: bp(p, "bIsTurret", true),
            staring: bp(p, "bStaringActive", true),
            turret_on: bp(p, "bTurretActive", false),
            fire_time: fp(p, "fireAttemptTime", 1.0) as f32,
            stop_when_disabled: bp(p, "bStopStaringWhenDisabled", true),
            pitch_ok: bp(p, "bAllowPitchRotation", false),
            head: bp(p, "bAimForHead", false),
            fov: fp(p, "TurretFOV", 4096.0) as i32,
            child: None,
            parent: None,
            slave: false,
            emp_vulnerable: bp(p, "bEMPVulnerable", false),
            confused: false,
            conf_t: 0.0,
            conf_dur: fp(p, "confusionDuration", 0.0) as f32,
            salvo_delay,
            salvo: None,
            mode,
            target_disabled_robots: bp(p, "bTargetDisabledRobots", false),
            rot_override: bp(p, "bRotationOverride", false),
            npcs_only: bp(p, "bTargetNPCsOnly", false),
            enemies,
            target_class: np(p, "targetClass"),
            require_zone: bp(p, "bTargetRequireZone", false),
            zones: [np(p, "TargetRequiredZone1"), np(p, "TargetRequiredZone2")],
            target_aim: m.rotation,
            dynamic_aim: bp(p, "bDynamicAim", true),
            fire_sound: snd,
            alarm_activated: bp(p, "bAlarmActivated", false),
            alarm_was: false,
            disable_flag: np(p, "disableFlag"),
            max_pitch: fp(p, "maxPitch", 0.0) as i32,
            fire_timer: 0.0,
            view: Target::None,
            spots: Vec::new(),
        });
    }
    // PreBeginPlay: a master takes its slave (SlaveMover, by actor name); the slave stops staring of its own.
    for t in 0..st.turrets.len() {
        let slave = np(&lvl.actors[lvl.movers[st.turrets[t].mover].actor].props, "SlaveMover");
        if slave.is_empty() {
            continue;
        }
        if let Some(c) = st.turrets.iter().position(|x| x.name.eq_ignore_ascii_case(&slave)) {
            st.turrets[t].child = Some(c);
            st.turrets[c].staring = false;
            st.turrets[c].slave = true;
            st.turrets[c].parent = Some(t);
        }
    }
    for a in lvl.actors.iter() {
        if !crate::classtree::is_a(&a.class, "ProjectileSpawnPoint") {
            continue;
        }
        let p = &a.props;
        let l = a.location();
        let pos = DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64);
        let class = match find(p, "ProjectileClass") {
            Some(Value::Path(v)) => v.rsplit('.').next().unwrap_or("Rocket").to_string(),
            _ => "Rocket".to_string(),
        };
        let owner_name = np(p, "ownerTurret");
        let spot = Spot {
            tag: crate::ai::actor_tag(a).to_string(),
            pos,
            rot: rp(p, "Rotation").unwrap_or([0; 3]),
            owner: (!owner_name.is_empty()).then(|| st.turrets.iter().position(|t| t.name.eq_ignore_ascii_case(&owner_name))).flatten(),
            attach_rot: [0; 3],
            base_rot: [0; 3],
            attach_off: DVec3::ZERO,
            class,
            speed_mult: fp(p, "ProjectileSpeedMult", 1.0),
            damage_mult: fp(p, "ProjectileDamageMult", 1.0) as f32,
            homing: bp(p, "bProjectileHoming", false),
            sound: crate::sound::prop(p, "ProjectileFireSound"),
            radius: fp(p, "ProjectileFireSoundRadius", 1024.0) as f32,
            volume: fp(p, "ProjectileFireSoundVolume", 3.0) as f32,
            fire_time: fp(p, "fireTime", 0.0) as f32,
            burst: fp(p, "burstAmount", 0.0) as i32,
            require_los: bp(p, "bRequireLOS", false),
            fire_timer: 0.0,
            burst_count: 0,
        };
        st.spots.push(spot);
    }
    // PostPostBeginPlay: each turret takes the points that name it, then (bAssimilateNearbys) the free-standing
    // ones within AssimilationRange. A point rides the mover that took it last.
    let independent: Vec<bool> = lvl
        .actors
        .iter()
        .filter(|a| crate::classtree::is_a(&a.class, "ProjectileSpawnPoint"))
        .map(|a| bp(&a.props, "bIndependent", false))
        .collect();
    for t in 0..st.turrets.len() {
        let m = &lvl.movers[st.turrets[t].mover];
        let mloc = DVec3::new(m.location[0] as f64, m.location[1] as f64, m.location[2] as f64);
        let p = &lvl.actors[m.actor].props;
        let assimilate = bp(p, "bAssimilateNearbys", true);
        let range = fp(p, "AssimilationRange", 256.0);
        for s in 0..st.spots.len() {
            if independent[s] || st.turrets[t].spots.contains(&s) {
                continue;
            }
            let named = st.spots[s].owner == Some(t);
            if named || (assimilate && st.spots[s].pos.distance(mloc) <= range) {
                st.turrets[t].spots.push(s);
                if st.turrets[t].spots.len() > 16 {
                    st.turrets[t].spots.pop();
                    continue;
                }
                let sp = &mut st.spots[s];
                sp.owner = Some(t);
                sp.attach_rot = sub(sp.rot, m.rotation);
                sp.base_rot = sp.rot;
                sp.attach_off = sp.pos - mloc;
            }
        }
    }
    if let Ok(mut q) = STARING.lock() {
        *q = st.turrets.iter().map(|t| t.mover).collect();
    }
    if !st.turrets.is_empty() || !st.spots.is_empty() {
        println!("TNM staring movers: {} ({} spawn points)", st.turrets.len(), st.spots.len());
    }
    st
}

/// ProjectileSpawnPoint.FireProjectile.
#[allow(clippy::too_many_arguments)]
fn fire_spot(
    st: &mut State,
    s: usize,
    lvl: &dx_level::Level,
    movers: &crate::movers::Movers,
    targets: &dyn Fn(Target) -> Option<(DVec3, f64, Option<usize>)>,
    launches: &mut crate::projectiles::Launches,
    speed_of: &mut dyn FnMut(&str) -> f64,
) {
    let Some(t) = st.spots[s].owner else { return };
    if movers.rt.get(st.turrets[t].mover).is_none_or(|r| r.destroyed) {
        return;
    }
    if st.spots[s].fire_timer < st.spots[s].fire_time {
        return;
    }
    let tgt = targets(st.turrets[t].view);
    let sp = &st.spots[s];
    if sp.require_los {
        if let Some((loc, eye, _)) = tgt {
            let at = if st.turrets[t].head { loc + DVec3::new(0.0, 0.0, eye) } else { loc };
            if lvl.collision.trace(sp.pos, at, DVec3::ZERO).fraction < 1.0 {
                return;
            }
        }
    }
    let aim = st.turrets[t].target_aim;
    let class = sp.class.clone();
    let speed = speed_of(&class) * sp.speed_mult;
    launches.0.push(crate::projectiles::Launch {
        class,
        pos: sp.pos,
        dir: dir_of(aim),
        from: None,
        mult: sp.damage_mult,
        life: None,
        scale: None,
        still: false,
        track: if sp.homing { tgt.and_then(|t| t.2) } else { None },
        track_at: None,
        speed: Some(speed),
        ownerless: true,
    });
    if let Some(snd) = &sp.sound {
        crate::sound::play(snd, sp.pos, sp.volume, sp.radius, 1.0);
    }
    let sp = &mut st.spots[s];
    sp.burst_count += 1;
    if sp.burst_count >= sp.burst {
        sp.fire_timer = 0.0;
        sp.burst_count = 0;
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn staring_system(
    time: Res<Time>,
    level: Res<crate::LevelRes>,
    mut movers: ResMut<crate::movers::Movers>,
    blockers: Res<crate::actors::Blockers>,
    player: Res<crate::Player>,
    flags: Res<crate::ai::GameFlags>,
    alarms: Res<crate::alarms::Alarms>,
    mut launches: ResMut<crate::projectiles::Launches>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    npcs: Query<&Npc>,
    mut map_gen: Local<u64>,
) {
    if crate::loader::new_map(&mut map_gen) {
        reset_level();
    }
    let lvl = &level.0;
    let dt = time.delta_secs().min(0.1);
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(build(lvl));
    }
    let st = g.as_mut().unwrap();
    if st.turrets.is_empty() && st.spots.is_empty() {
        EVENTS.lock().map(|mut q| q.clear()).ok();
        return;
    }
    // Pawns by blocker: location, eye height, alliance, a disabled robot, dead.
    struct P {
        loc: DVec3,
        eye: f64,
        alliance: String,
        disabled: bool,
        class: String,
    }
    let mut pawns: std::collections::HashMap<usize, P> = std::collections::HashMap::new();
    for n in &npcs {
        if n.state == NpcState::Dead || n.net.is_some() {
            continue;
        }
        let Some(c) = blockers.list.get(n.blocker) else { continue };
        if c.radius <= 0.0 {
            continue;
        }
        let a = &lvl.actors[n.actor];
        pawns.insert(
            n.blocker,
            P {
                loc: c.center,
                eye: fp(&a.props, "BaseEyeHeight", 38.0),
                alliance: n.alliance.clone(),
                disabled: n.hp_body.robot && n.emp_hp <= 0.0,
                class: a.class.clone(),
            },
        );
    }
    let me = player.body.location;
    let my_eye = player.eye_height;
    let radar = crate::abilities::aug_active("AugRadarTrans");
    let my_zone = crate::zones::zone_tag(lvl, me);
    let targets = |t: Target| -> Option<(DVec3, f64, Option<usize>)> {
        match t {
            Target::None => None,
            Target::Player => Some((me, my_eye, None)),
            Target::Pawn(b) => pawns.get(&b).map(|p| (p.loc, p.eye, Some(b))),
        }
    };
    // Events: each state's Trigger.
    for (tag, on) in EVENTS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        if !on {
            continue;
        }
        for t in st.turrets.iter_mut() {
            let info = &lvl.movers[t.mover];
            if !info.tag.eq_ignore_ascii_case(&tag) || movers.rt[t.mover].destroyed {
                continue;
            }
            match t.mode {
                Mode::Toggle | Mode::Salvo => {
                    if t.stop_when_disabled {
                        t.staring = !t.staring;
                    }
                    if t.is_turret && !t.stop_when_disabled {
                        t.turret_on = !t.turret_on;
                    } else if t.is_turret {
                        t.turret_on = t.staring;
                    }
                }
                Mode::TurretOnly => {
                    if t.is_turret {
                        t.turret_on = !t.turret_on;
                    }
                }
                Mode::StaringOnly => {
                    if t.stop_when_disabled {
                        t.staring = !t.staring;
                    }
                }
                Mode::Other => {}
            }
        }
    }
    for (tag, rot) in TURN.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        for t in st.turrets.iter() {
            if lvl.movers[t.mover].tag.eq_ignore_ascii_case(&tag) {
                let rt = &mut movers.rt[t.mover];
                rt.desired_rot = Some(rot);
                rt.rot_rate = rp(&lvl.actors[lvl.movers[t.mover].actor].props, "RotationRate").unwrap_or([1250, 12500, 0]);
            }
        }
    }
    for (mi, rot, child) in TURN_M.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        let Some(&t) = st.by_mover.get(&mi) else { continue };
        let rate = |m: usize| rp(&lvl.actors[lvl.movers[m].actor].props, "RotationRate").unwrap_or([1250, 12500, 0]);
        let rt = &mut movers.rt[mi];
        rt.desired_rot = Some([rot[0].rem_euclid(65536), rot[1].rem_euclid(65536), rot[2].rem_euclid(65536)]);
        rt.rot_rate = rate(mi);
        if let (Some(cr), Some(c)) = (child, st.turrets[t].child) {
            let cm = st.turrets[c].mover;
            let r = rate(cm);
            let rt = &mut movers.rt[cm];
            rt.desired_rot = Some([cr[0].rem_euclid(65536), cr[1].rem_euclid(65536), cr[2].rem_euclid(65536)]);
            rt.rot_rate = r;
        }
    }
    for (mi, dmg) in EMPS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        let Some(&t) = st.by_mover.get(&mi) else { continue };
        let t = if st.turrets[t].slave { st.turrets[t].parent.unwrap_or(t) } else { t };
        let tu = &mut st.turrets[t];
        tu.conf_dur = (tu.conf_dur + (dmg - 15.0).max(0.0) / 5.0).clamp(3.0, 30.0);
        tu.conf_t = 0.0;
        if !tu.confused {
            tu.confused = true;
            let l = movers.rt[tu.mover].location;
            crate::sound::play("DeusExSounds.Generic.EMPZap", l, 1.0, 1280.0, 1.0);
        }
    }
    for s in st.spots.iter_mut() {
        s.fire_timer += dt;
    }
    let mut speed_cache: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut speed_of = |class: &str| -> f64 {
        *speed_cache.entry(class.to_string()).or_insert_with(|| fp(&assets.classes.defaults(class), "speed", 1000.0))
    };
    for t in 0..st.turrets.len() {
        let mi = st.turrets[t].mover;
        if movers.rt[mi].destroyed {
            continue;
        }
        let props = &lvl.actors[lvl.movers[mi].actor].props;
        let rate = rp(props, "RotationRate").unwrap_or([1250, 12500, 0]);
        let loc = movers.rt[mi].location;
        let rotation = movers.rt[mi].rotation;
        // AlarmHeard.
        if st.turrets[t].alarm_activated {
            let heard = alarms.heard_at(loc);
            let tu = &mut st.turrets[t];
            if heard && !tu.alarm_was && !flags.0.get_bool(&tu.disable_flag) {
                tu.staring = true;
                if tu.is_turret {
                    tu.turret_on = true;
                }
            } else if !heard && tu.alarm_was {
                tu.staring = false;
                if tu.is_turret {
                    tu.turret_on = false;
                }
            }
            tu.alarm_was = heard;
        }
        st.turrets[t].target_aim = rotation;
        // SalvoFiring: the points one after another.
        if let Some((k, wait)) = st.turrets[t].salvo {
            let w = wait - dt;
            if w <= 0.0 {
                let spot = st.turrets[t].spots.get(k).copied();
                match spot {
                    Some(s) => {
                        fire_spot(st, s, lvl, &movers, &targets, &mut launches, &mut speed_of);
                        st.turrets[t].salvo = Some((k + 1, st.turrets[t].salvo_delay[k.min(15)]));
                    }
                    None => st.turrets[t].salvo = None,
                }
            } else {
                st.turrets[t].salvo = Some((k, w));
            }
        }
        let mut want: Option<[i32; 3]> = None;
        let mut child_want: Option<[i32; 3]> = None;
        let mut fire = false;
        if st.turrets[t].confused {
            let tu_conf_t = st.turrets[t].conf_t + dt;
            st.turrets[t].conf_t = tu_conf_t;
            if tu_conf_t % 0.25 > 0.2 {
                let (r1, r2) = (st.frand(), st.frand());
                let tu = &st.turrets[t];
                let mut d = movers.rt[mi].desired_rot.unwrap_or(rotation);
                if tu.pitch_ok {
                    d[0] = rotation[0] + 4096 - (r1 * 8192.0) as i32;
                }
                d[1] = rotation[1] + 4096 - (r2 * 8192.0) as i32;
                if tu.max_pitch != 0 && d[0] > tu.max_pitch {
                    d[0] = tu.max_pitch;
                }
                want = Some(d);
                child_want = Some(d);
                st.turrets[t].target_aim = d;
            }
            if st.turrets[t].conf_t > st.turrets[t].conf_dur {
                let tu = &mut st.turrets[t];
                tu.confused = false;
                tu.conf_t = 0.0;
                tu.conf_dur = 0.0;
            }
        } else {
            let tu = &mut st.turrets[t];
            // A target it can no longer have.
            match tu.view {
                Target::Pawn(b) => match pawns.get(&b) {
                    None => tu.view = Target::None,
                    Some(p) if p.disabled && !tu.target_disabled_robots => tu.view = Target::None,
                    _ => {}
                },
                Target::Player if radar => tu.view = Target::None,
                _ => {}
            }
            if tu.require_zone {
                let z = match tu.view {
                    Target::Player => Some(my_zone.clone()),
                    Target::Pawn(b) => pawns.get(&b).map(|p| crate::zones::zone_tag(lvl, p.loc)),
                    Target::None => None,
                };
                if let Some(z) = z {
                    if !tu.zones.iter().any(|r| r.eq_ignore_ascii_case(&z)) {
                        tu.view = Target::None;
                    }
                }
            }
            let tgt = targets(tu.view);
            if let (Some((tl, eye, _)), true) = (tgt, tu.staring) {
                let dist = tl.distance(loc);
                if dist >= tu.min_d && dist <= tu.max_d {
                    let nl = if tu.head { tl + DVec3::new(0.0, 0.0, eye) } else { tl };
                    let mut nr = rotator(nl - loc);
                    let mut aim = nr;
                    aim[1] = rotation[1];
                    if !tu.dynamic_aim && !tu.pitch_ok {
                        aim[0] = 0;
                    }
                    tu.target_aim = aim;
                    if !tu.pitch_ok {
                        nr[0] = 0;
                    }
                    if tu.max_pitch != 0 && nr[0] > tu.max_pitch && nr[0] < 32768 {
                        nr[0] = tu.max_pitch;
                    }
                    if !tu.rot_override {
                        want = Some(nr);
                    }
                    if let Some(c) = tu.child {
                        let cm = st.turrets[c].mover;
                        if !st.turrets[c].rot_override && !movers.rt[cm].destroyed {
                            let mut cr = rotator(nl - movers.rt[cm].location);
                            if !st.turrets[c].pitch_ok {
                                cr[0] = 0;
                            }
                            cr[1] = if !st.turrets[t].rot_override { want.map_or(rotation[1], |w| w[1]) } else { rotation[1] };
                            child_want = Some(cr);
                        }
                    }
                    let tu = &mut st.turrets[t];
                    if tu.is_turret && tu.turret_on {
                        tu.fire_timer += dt;
                        let r = rotator(tl - loc);
                        if ang_diff(rotation[1], r[1]).abs() <= tu.fov && ang_diff(rotation[0], r[0]).abs() <= tu.fov && tu.fire_timer >= tu.fire_time {
                            fire = true;
                            tu.fire_timer = 0.0;
                            if let Some(s) = &tu.fire_sound {
                                crate::sound::play(s, loc, 1.0, 1280.0, 1.0);
                            }
                        }
                    }
                }
            } else if let (Some((tl, _, _)), true, Some(par)) = (tgt, tu.slave && tu.is_turret && tu.turret_on, tu.parent) {
                if !st.turrets[par].confused {
                    let ploc = movers.rt[st.turrets[par].mover].location;
                    let dist = tl.distance(ploc);
                    let tu = &mut st.turrets[t];
                    if dist >= tu.min_d && dist <= tu.max_d {
                        tu.fire_timer += dt;
                        let r = rotator(tl - loc);
                        if ang_diff(rotation[1], r[1]).abs() < tu.fov && ang_diff(rotation[0], r[0]).abs() < tu.fov && tu.fire_timer >= tu.fire_time {
                            fire = true;
                            tu.fire_timer = 0.0;
                        }
                    }
                }
            } else if tgt.is_none() {
                let tu = &mut st.turrets[t];
                if !tu.npcs_only {
                    let zone_ok = !tu.require_zone || tu.zones.iter().any(|z| z.eq_ignore_ascii_case(&my_zone));
                    if zone_ok && !radar {
                        tu.view = Target::Player;
                    }
                } else {
                    // The last pawn in the list that qualifies.
                    let mut pick = None;
                    for (b, p) in pawns.iter() {
                        let d = p.loc.distance(loc);
                        if d < tu.min_d || d > tu.max_d {
                            continue;
                        }
                        if !tu.target_class.is_empty() && !crate::classtree::is_a(&p.class, &tu.target_class) {
                            continue;
                        }
                        if !tu.enemies.iter().any(|e| e.eq_ignore_ascii_case(&p.alliance)) {
                            continue;
                        }
                        if tu.require_zone && !tu.zones.iter().any(|z| z.eq_ignore_ascii_case(&crate::zones::zone_tag(lvl, p.loc))) {
                            continue;
                        }
                        if p.disabled && !tu.target_disabled_robots {
                            continue;
                        }
                        pick = Some(*b);
                    }
                    if let Some(b) = pick {
                        tu.view = Target::Pawn(b);
                    }
                }
            }
        }
        if let Some(w) = want {
            let rt = &mut movers.rt[mi];
            rt.desired_rot = Some([w[0].rem_euclid(65536), w[1].rem_euclid(65536), 0]);
            rt.rot_rate = rate;
        }
        if let (Some(w), Some(c)) = (child_want, st.turrets[t].child) {
            let cm = st.turrets[c].mover;
            let crate_rate = rp(&lvl.actors[lvl.movers[cm].actor].props, "RotationRate").unwrap_or([1250, 12500, 0]);
            let rt = &mut movers.rt[cm];
            rt.desired_rot = Some([w[0].rem_euclid(65536), w[1].rem_euclid(65536), 0]);
            rt.rot_rate = crate_rate;
        }
        if fire {
            if st.turrets[t].mode == Mode::Salvo {
                if st.turrets[t].salvo.is_none() {
                    st.turrets[t].salvo = Some((0, 0.0));
                }
            } else {
                let spots = st.turrets[t].spots.clone();
                for s in spots {
                    fire_spot(st, s, lvl, &movers, &targets, &mut launches, &mut speed_of);
                }
            }
        }
    }
    // The spawn points ride their movers (UpdateProjectileSpawnLocation).
    for s in st.spots.iter_mut() {
        let Some(t) = s.owner else { continue };
        let rt = &movers.rt[st.turrets[t].mover];
        let rot = add(rt.rotation, s.attach_rot);
        let trot = sub(rot, s.base_rot);
        s.rot = rot;
        s.pos = rt.location + rotate(s.attach_off, trot);
    }
    // Scripted ambient sounds ride their movers.
    if let Ok(a) = AMBIENT.lock() {
        for (mi, snd, sr) in a.iter() {
            let info = &lvl.movers[*mi];
            let p = &lvl.actors[info.actor].props;
            let radius = 25.0 * (sr.unwrap_or_else(|| fp(p, "SoundRadius", 32.0)) + 1.0);
            let vol = (fp(p, "SoundVolume", 128.0) / 255.0) as f32;
            crate::sound::hold(crate::sound::key_of(&info.name) ^ 0x57a1, snd, Some(movers.rt[*mi].location), vol, radius as f32, 1.0);
        }
    }
    if let Ok(mut snap) = SNAP.lock() {
        snap.clear();
        for t in &st.turrets {
            let rt = &movers.rt[t.mover];
            snap.push((lvl.movers[t.mover].tag.clone(), t.mover, rt.rotation, rt.desired_rot.is_none()));
        }
    }
    // Mission scripts: fire spawn points by tag (they need an owner turret, as FireProjectile does).
    for tag in FIRE_TAGS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        for s in 0..st.spots.len() {
            if st.spots[s].tag.eq_ignore_ascii_case(&tag) {
                fire_spot(st, s, lvl, &movers, &targets, &mut launches, &mut speed_of);
            }
        }
    }
}
