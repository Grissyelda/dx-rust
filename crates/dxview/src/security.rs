//! Security devices: SecurityCamera, AutoTurret (+ its AutoTurretGun) and LaserTrigger (+ LaserEmitter).
//!
//! * SecurityCamera (HackableDevices): swings `swingAngle` either side every `swingPeriod` s; every 0.1 s looks
//!   for the player (within cameraRange, a clear line, within cameraFOV of where it points in both yaw and
//!   pitch; AugRadarTrans hides you). Seen: it follows you and blinks red with a beep for triggerDelay (4 s),
//!   then sounds the klaxon and raises the 'Alarm' AI event (heard out to 1625; pawns search where it last saw
//!   you). Out of sight for memoryTime (5 s) it calms down. bNoAlarm cameras only watch. Hacked or switched off
//!   (UnTrigger) it goes dark; frobbing a bypassed camera toggles it. EMP confuses it (twitching, yellow light).
//! * AutoTurret: a base (DeusExDecoration) carrying a gun (AutoTurretGun, HackableDevices) spawned on top of it -
//!   below it on the Island, where they hang from the ceiling. While active and not disabled it picks a target
//!   (bTrackPlayersOnly: you, or any pawn that isn't hostile to you; bTrackPawnsOnly: pawns hostile to you;
//!   neither: also decorations), turns the gun at 16384/s within pitchLimit, and fires every fireRate s once it's
//!   aimed: one gunAccuracy-spread trace of gunDamage 'AutoShot', PistolFire, a shell casing, a muzzle flash and
//!   a LoudNoise. Hearing an alarm switches it on until the alarm ends. Bypassing the gun disables / re-enables
//!   it. EMP confuses it.
//! * LaserTrigger: a LaserEmitter beam (5000 units along its rotation, up to two mirror bounces) shown as thin
//!   red segments with a spot where it lands. Breaking it (the player, or a decoration for TT_PlayerProximity;
//!   thermoptic camo doesn't) beeps and starts its klaxon + 'Alarm' event for alarmTimeout (30 s). Triggered /
//!   UnTriggered by events; EMP makes it flicker; 'Exploded' / 'Shot' damage past minDamageThreshold destroys it.
//! * Mission01 (statue cleared): the NSFTurret01/02 turrets and statue_lasertrap lasers go, and the NSFCam
//!   cameras stop raising alarms.

use crate::actors::{Blockers, Deco, MeshActorRef, World};
use crate::ai::{Npc, NpcState, PlayerHealth, Target};
use crate::alarms::Alarms;
use crate::weapons::{image_for, masked_image_for, spawn_mesh_parts, Clip, RuntimeAssets};
use crate::{FlyCam, LevelRes, Player, SkyCam};
use bevy::prelude::*;
use dx_level::actors::{anim_mesh, rotator_axes, AnimMesh, MeshActor};
use dx_level::SurfaceClass;
use dx_level::collision::{trace_cylinder, Cylinder};
use dx_level::ue_to_bevy;
use glam::DVec3;
use imsim_core::body::{Body, BodyPhysics};
use imsim_core::movement::CollisionWorld;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use unreal_pkg::props::{find, Property, Value};

const URU: f64 = 65536.0;

// ------------------------------------------------------------------------------------------------ queues

/// Devices bypassed (or frobbed once bypassed) this frame, by blocker: HackAction(bHacked = True).
static HACKED: Mutex<Vec<usize>> = Mutex::new(Vec::new());
/// Events dispatched this frame (tag, trigger?).
static EVENTS: Mutex<Vec<(String, bool)>> = Mutex::new(Vec::new());
/// Blasts this frame (centre, radius, damage, type), for EMP and lasers (which aren't blockers).
static BLASTS: Mutex<Vec<(DVec3, f64, f32, &'static str)>> = Mutex::new(Vec::new());
/// AlarmLight Trigger / UnTrigger: (actor, on) - the lens skin (MultiSkins[1]) swaps.
static ALARM_LIGHTS: Mutex<Vec<(usize, bool)>> = Mutex::new(Vec::new());

pub fn alarm_light(actor: usize, on: bool) {
    if let Ok(mut q) = ALARM_LIGHTS.lock() {
        q.push((actor, on));
    }
}

/// Mission01's "statue cleared" clean-up.
static MISSION: Mutex<bool> = Mutex::new(false);

pub fn hacked(blocker: usize) {
    if let Ok(mut q) = HACKED.lock() {
        q.push(blocker);
    }
}

pub fn event(tag: &str, on: bool) {
    if let Ok(mut q) = EVENTS.lock() {
        q.push((tag.to_string(), on));
    }
}

/// Warren's EMP field (eggs.rs), slot 2 turrets, 3 laser triggers, 4 beam triggers, 5 cameras (1 alarm units go to
/// alarms.rs): 100 EMP to the ones in range that aren't confused already, with sparks.
pub fn warren(center: DVec3, radius: f64, slot: u8) {
    if slot == 1 {
        crate::alarms::warren(center, radius);
        return;
    }
    let ty: &'static str = match slot {
        2 => "WarrenTurret",
        3 => "WarrenLaser",
        4 => "WarrenBeam",
        _ => "WarrenCamera",
    };
    blast(center, radius, 100.0, ty);
}

pub fn blast(center: DVec3, radius: f64, damage: f32, damage_type: &'static str) {
    if let Ok(mut q) = BLASTS.lock() {
        q.push((center, radius, damage, damage_type));
    }
}

/// Mission03: every SecurityCamera and AutoTurret UnTrigger(None, None).
pub fn untrigger_all() {
    event("*", false);
}

/// Mission01.Timer once StatueMissionComplete is set.
pub fn statue_cleared() {
    if let Ok(mut m) = MISSION.lock() {
        *m = true;
    }
}

/// Is this actor class one of the ElectronicDevices family (turrets never shoot those)?
fn electronic(class: &str) -> bool {
    matches!(
        class,
        "ATM" | "AcousticSensor" | "AlarmUnit" | "AutoTurretGun" | "AutoTurretGunSmall" | "CigaretteMachine"
            | "ComputerPersonal" | "ComputerPublic" | "ComputerSecurity" | "Computers" | "ControlPanel"
            | "HackableDevices" | "Keypad" | "Keypad1" | "Keypad2" | "Keypad3" | "Phone" | "Pinball"
            | "RetinalScanner" | "SecurityCamera" | "TAD" | "VendingMachine"
    )
}

// ------------------------------------------------------------------------------------------------ helpers

fn fv(p: &[Property], n: &str, d: f64) -> f64 {
    match find(p, n) {
        Some(Value::Float(v)) => *v as f64,
        Some(Value::Int(v)) => *v as f64,
        Some(Value::Byte(v)) => *v as f64,
        _ => d,
    }
}

fn bv(p: &[Property], n: &str, d: bool) -> bool {
    match find(p, n) {
        Some(Value::Bool(v)) => *v,
        _ => d,
    }
}

fn sv(p: &[Property], n: &str) -> String {
    match find(p, n) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

fn loc_of(p: &[Property]) -> DVec3 {
    match find(p, "Location") {
        Some(Value::Vector(v)) => DVec3::new(v[0] as f64, v[1] as f64, v[2] as f64),
        _ => DVec3::ZERO,
    }
}

fn rot_of(p: &[Property]) -> [i32; 3] {
    match find(p, "Rotation") {
        Some(Value::Rotator(r)) => *r,
        _ => [0, 0, 0],
    }
}

/// Rotator(v): (pitch, yaw) in Unreal units.
fn rotator(v: DVec3) -> (f64, f64) {
    let flat = (v.x * v.x + v.y * v.y).sqrt();
    (
        v.z.atan2(flat) / std::f64::consts::TAU * URU,
        v.y.atan2(v.x) / std::f64::consts::TAU * URU,
    )
}

/// Vector(Rotator(pitch, yaw)).
fn dir(pitch: f64, yaw: f64) -> DVec3 {
    let (p, y) = (pitch / URU * std::f64::consts::TAU, yaw / URU * std::f64::consts::TAU);
    DVec3::new(p.cos() * y.cos(), p.cos() * y.sin(), p.sin())
}

/// The shortest signed difference a - b on the 65536 circle.
fn wrap(d: f64) -> f64 {
    let mut d = d % URU;
    if d > URU / 2.0 {
        d -= URU;
    } else if d < -URU / 2.0 {
        d += URU;
    }
    d
}

/// bRotateToDesired at `rate` units/s per axis.
fn turn(cur: &mut f64, want: f64, rate: f64, dt: f64) {
    let d = wrap(want - *cur);
    let step = rate * dt;
    *cur += d.clamp(-step, step);
}

fn ue(v: Vec3) -> DVec3 {
    DVec3::new(v.x as f64, v.z as f64, v.y as f64)
}

/// A swappable skin slot on a mesh entity: the child drawing it, its own material, the texture shown now.
struct Skin {
    child: Entity,
    original: Handle<StandardMaterial>,
    class: SurfaceClass,
    shown: Option<&'static str>,
}

/// Find the child entity that draws texture slot `slot` of a mesh actor entity (MultiSkins[slot]).
fn find_skin(
    root: Entity,
    handles: &[Handle<Mesh>],
    parts: &[dx_level::actors::MeshPart],
    slot: u32,
    children: &Query<&Children>,
    mats: &Query<(&Mesh3d, &MeshMaterial3d<StandardMaterial>)>,
) -> Option<Skin> {
    let pi = parts.iter().position(|p| p.slots.contains(&slot))?;
    let want = handles.get(pi)?;
    let kids = children.get(root).ok()?;
    for c in kids.iter() {
        if let Ok((m, mat)) = mats.get(c) {
            if m.0 == *want {
                return Some(Skin { child: c, original: mat.0.clone(), class: parts[pi].class, shown: None });
            }
        }
    }
    None
}

/// MultiSkins[slot] = tex (None = the mesh's own texture).
fn set_skin(
    skin: &mut Skin,
    tex: Option<&'static str>,
    commands: &mut Commands,
    assets: &mut RuntimeAssets,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
) {
    if skin.shown == tex {
        return;
    }
    skin.shown = tex;
    let mat = match tex {
        None => skin.original.clone(),
        Some(path) => {
            let key = (Some(format!("skin:{path}")), skin.class);
            if let Some(h) = assets.materials.get(&key) {
                h.clone()
            } else {
                let img = if skin.class == SurfaceClass::Masked {
                    masked_image_for(assets, images, path)
                } else {
                    image_for(assets, images, path)
                };
                let h = materials.add(StandardMaterial {
                    base_color_texture: img,
                    unlit: true,
                    double_sided: true,
                    cull_mode: None,
                    alpha_mode: match skin.class {
                        SurfaceClass::Masked => AlphaMode::Mask(0.5),
                        SurfaceClass::Translucent => AlphaMode::Add,
                        SurfaceClass::Modulated => AlphaMode::Multiply,
                        _ => AlphaMode::Opaque,
                    },
                    ..default()
                });
                assets.materials.insert(key, h.clone());
                h
            }
        }
    };
    commands.entity(skin.child).insert(MeshMaterial3d(mat));
}

const GREEN: &str = "DeusExItems.Skins.GreenLightTex";
const RED: &str = "DeusExItems.Skins.RedLightTex";
const YELLOW: &str = "DeusExDeco.Skins.YellowLightTex";
const BLACK: &str = "DeusExItems.Skins.BlackMaskTex";
const FLASH: &str = "DeusExItems.Skins.FlatFXTex34";

// ------------------------------------------------------------------------------------------------ state

struct Cam {
    actor: usize,
    blocker: Option<usize>,
    entity: Option<Entity>,
    pos: DVec3,
    tag: String,
    orig: (f64, f64),
    rot: (f64, f64),
    want: (f64, f64),
    swing: bool,
    swing_angle: f64,
    swing_period: f64,
    fov: f64,
    range: f64,
    memory: f32,
    trigger_delay: f32,
    active: bool,
    no_alarm: bool,
    track: bool,
    seen: bool,
    triggered: bool,
    last_seen: f32,
    check_t: f32,
    swing_t: f32,
    trigger_t: f32,
    confused: bool,
    confusion_t: f32,
    confusion_dur: f32,
    player_loc: DVec3,
    lens: Option<Skin>,
    alarm: usize,
    /// The hum it's had since the level started (inactive placed cameras keep the class's CameraHum).
    hum: bool,
    seed: u64,
    gone: bool,
    msg_on: String,
    msg_off: String,
}

struct Turret {
    actor: usize,
    tag: String,
    base_blocker: Option<usize>,
    base_entity: Option<Entity>,
    gun_actor: usize,
    gun_blocker: usize,
    gun_entity: Entity,
    gun_pos: DVec3,
    orig: (f64, f64),
    rot: (f64, f64),
    want: (f64, f64),
    active: bool,
    pre_alarm: bool,
    heard: bool,
    disabled: bool,
    players_only: bool,
    pawns_only: bool,
    max_range: f64,
    fire_rate: f32,
    accuracy: f64,
    damage: f32,
    pitch_limit: f64,
    fire_t: f32,
    flash_t: f32,
    target: Option<TurretTarget>,
    last_target: Option<TurretTarget>,
    confused: bool,
    confusion_t: f32,
    confusion_dur: f32,
    hum: bool,
    mesh: Option<Arc<AnimMesh>>,
    parts: Vec<dx_level::actors::MeshPart>,
    handles: Vec<Handle<Mesh>>,
    clip: Option<Clip>,
    firing: bool,
    lens: Option<Skin>,
    muzzle: Option<Skin>,
    gone: bool,
    seed: u64,
}

#[derive(Clone, Copy, PartialEq)]
enum TurretTarget {
    Player,
    Pawn(usize),
    Deco(usize),
}

#[derive(Clone, Copy, PartialEq)]
enum Broke {
    Player,
    Blocker(usize),
}

struct Laser {
    actor: usize,
    tag: String,
    pos: DVec3,
    rot: [i32; 3],
    on: bool,
    no_alarm: bool,
    trigger_type: u8,
    confused: bool,
    confusion_t: f32,
    confusion_dur: f32,
    hp: f32,
    min_damage: f32,
    alarm_timeout: f32,
    alarm_t: Option<f32>,
    last_hit: Option<Broke>,
    seed: u64,
    beams: Vec<Entity>,
    spots: Vec<Entity>,
    emitter_entity: Option<Entity>,
    lens: Option<Skin>,
    alarm: usize,
    seek: DVec3,
    gone: bool,
    /// BeamTrigger: a blue beam that triggers Event (once with bTriggerOnceOnly) instead of an alarm.
    beam: Option<BeamEvent>,
}

struct BeamEvent {
    event: String,
    once: bool,
    done: bool,
}

#[derive(Resource, Default)]
pub struct Security {
    init: bool,
    cams: Vec<Cam>,
    turrets: Vec<Turret>,
    lasers: Vec<Laser>,
    beam_mesh: Option<Handle<Mesh>>,
    beam_mat: Option<Handle<StandardMaterial>>,
    spot_mesh: Option<Handle<Mesh>>,
    spot_mat: Option<Handle<StandardMaterial>>,
    /// LaserEmitter.SetBlueBeam (BeamTrigger): LaserBeam2 / LaserSpot2.
    beam2_mat: Option<Handle<StandardMaterial>>,
    spot2_mat: Option<Handle<StandardMaterial>>,
    seed: u64,
}

/// What a save keeps of the security devices (save.rs). Destroyed cameras and turrets come back destroyed by
/// themselves (their placed actors are gone); lasers by hp.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct SecuritySave {
    /// (actor, bActive, rotation pitch/yaw, desired pitch/yaw)
    pub cams: Vec<(usize, bool, (f64, f64), (f64, f64))>,
    /// (actor, bActive, bDisabled, rotation, desired)
    pub turrets: Vec<(usize, bool, bool, (f64, f64), (f64, f64))>,
    /// (actor, bIsOn, HitPoints)
    pub lasers: Vec<(usize, bool, f32)>,
}

impl Security {
    pub fn is_built(&self) -> bool {
        self.init
    }
    pub fn save(&self) -> SecuritySave {
        SecuritySave {
            cams: self.cams.iter().filter(|c| !c.gone).map(|c| (c.actor, c.active, c.rot, c.want)).collect(),
            turrets: self
                .turrets
                .iter()
                .filter(|t| !t.gone)
                .map(|t| (t.actor, t.active, t.disabled, t.rot, t.want))
                .collect(),
            lasers: self.lasers.iter().map(|l| (l.actor, l.on, if l.gone { 0.0 } else { l.hp })).collect(),
        }
    }
    pub fn restore(&mut self, s: &SecuritySave) {
        for (a, active, rot, want) in &s.cams {
            if let Some(c) = self.cams.iter_mut().find(|c| c.actor == *a) {
                c.active = *active;
                c.rot = *rot;
                c.want = *want;
                c.seen = false;
                c.triggered = false;
            }
        }
        for (a, active, disabled, rot, want) in &s.turrets {
            if let Some(t) = self.turrets.iter_mut().find(|t| t.actor == *a) {
                t.active = *active;
                t.disabled = *disabled;
                t.rot = *rot;
                t.want = *want;
                t.target = None;
            }
        }
        for (a, on, hp) in &s.lasers {
            if let Some(l) = self.lasers.iter_mut().find(|l| l.actor == *a) {
                l.on = *on;
                l.hp = *hp;
                l.alarm_t = None;
            }
        }
    }

    fn rand(&mut self) -> f64 {
        self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.seed >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn rnd(seed: &mut u64) -> f64 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    (*seed >> 11) as f64 / (1u64 << 53) as f64
}

/// Beam segments and spots: thin translucent red, LaserBeam1 / LaserSpot1 (Effects.Laser).
#[derive(Component)]
pub struct LaserFx;

// ------------------------------------------------------------------------------------------------ system

#[allow(clippy::too_many_arguments)]
pub fn security_system(
    mut commands: Commands,
    time: Res<Time>,
    mut level: ResMut<LevelRes>,
    mut blockers: ResMut<Blockers>,
    player: Res<Player>,
    mut ph: ResMut<PlayerHealth>,
    mut st: ResMut<Security>,
    mut alarms: ResMut<Alarms>,
    movers: Res<crate::movers::Movers>,
    cam: Single<&Transform, (With<FlyCam>, Without<SkyCam>, Without<LaserFx>, Without<Deco>)>,
    (mut decos, refs, children, part_mats, npcs): (
        Query<(&mut Deco, &mut Transform), Without<LaserFx>>,
        Query<(Entity, &MeshActorRef)>,
        Query<&Children>,
        Query<(&Mesh3d, &MeshMaterial3d<StandardMaterial>)>,
        Query<&Npc>,
    ),
    mut fx: Query<(&mut Transform, &mut Visibility), (With<LaserFx>, Without<Deco>, Without<FlyCam>)>,
    (mut assets, mut images, mut meshes, mut materials): (
        ResMut<RuntimeAssets>,
        ResMut<Assets<Image>>,
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
    ),
    (mut noises, mut pawn_hits, mut deco_hits, mut shells): (
        ResMut<crate::stealth::Noises>,
        ResMut<crate::damage::PawnHits>,
        ResMut<crate::destruct::DecoHits>,
        ResMut<crate::destruct::Shells>,
    ),
) {
    let _prof = crate::prof::scope("security::security_system");
    let dt = time.delta_secs().min(0.1);
    let dtd = dt as f64;
    let st = &mut *st;

    if !st.init {
        st.init = true;
        st.seed = 0x5EC0_1234;
        init(st, &mut commands, &mut level.0, &mut blockers, &mut alarms, &refs, &mut assets, &mut images, &mut meshes, &mut materials);
    }
    let lvl = &level.0;

    // ---------------------------------------------------------------- incoming
    let hacked: Vec<usize> = HACKED.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    let events: Vec<(String, bool)> = EVENTS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    let blasts: Vec<(DVec3, f64, f32, &'static str)> =
        BLASTS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    let mission = MISSION.lock().map(|mut m| std::mem::take(&mut *m)).unwrap_or(false);

    for b in hacked {
        if let Some(c) = st.cams.iter_mut().find(|c| c.blocker == Some(b) && !c.gone) {
            // SecurityCamera.HackAction: toggle (not while EMP'd).
            if !c.confused {
                if c.active {
                    cam_untrigger(c, true);
                } else {
                    cam_trigger(c, true);
                }
            }
        }
        if let Some(t) = st.turrets.iter_mut().find(|t| t.gun_blocker == b && !t.gone) {
            // AutoTurretGun.HackAction: disable, or bring back.
            if !t.disabled {
                turret_untrigger(t);
                t.disabled = true;
            } else {
                t.disabled = false;
                turret_trigger(t);
            }
            println!("SECURITY turret {} {}", lvl.actors[t.actor].name, if t.disabled { "disabled" } else { "enabled" });
        }
    }
    for (tag, on) in &events {
        for c in st.cams.iter_mut().filter(|c| !c.gone && (tag == "*" || c.tag.eq_ignore_ascii_case(tag))) {
            if *on {
                cam_trigger(c, false);
            } else {
                cam_untrigger(c, false);
            }
        }
        for t in st.turrets.iter_mut().filter(|t| !t.gone && (tag == "*" || t.tag.eq_ignore_ascii_case(tag))) {
            if *on {
                turret_trigger(t);
            } else {
                turret_untrigger(t);
            }
        }
        for l in st.lasers.iter_mut().filter(|l| !l.gone && l.tag.eq_ignore_ascii_case(tag)) {
            if !l.confused {
                if *on {
                    if !l.on {
                        l.on = true;
                        l.last_hit = None;
                    }
                } else if l.on {
                    l.on = false;
                    l.last_hit = None;
                    l.alarm_t = None;
                }
            }
        }
    }
    for (c, r, dmg, ty) in &blasts {
        if *ty == "EMP" {
            let extra = (*dmg - 15.0).max(0.0) / 5.0;
            for cm in st.cams.iter_mut().filter(|x| !x.gone && x.pos.distance(*c) < *r) {
                cm.confusion_dur += extra;
                cm.confusion_t = 0.0;
                if !cm.confused {
                    cm.confused = true;
                    crate::sound::play("DeusExSounds.Weapons.EMPZap", cm.pos, 1.0, 1280.0, 1.0);
                }
            }
            for t in st.turrets.iter_mut().filter(|x| !x.gone && x.gun_pos.distance(*c) < *r) {
                t.confusion_dur = (t.confusion_dur + extra).clamp(10.0, 30.0);
                t.confusion_t = 0.0;
                if !t.confused {
                    t.confused = true;
                    crate::sound::play("DeusExSounds.Weapons.EMPZap", t.gun_pos, 1.0, 1280.0, 1.0);
                }
            }
            for l in st.lasers.iter_mut().filter(|x| !x.gone && x.pos.distance(*c) < *r) {
                l.confusion_t = 0.0;
                if !l.confused {
                    l.confused = true;
                    crate::sound::play("DeusExSounds.Weapons.EMPZap", l.pos, 1.0, 1280.0, 1.0);
                }
            }
        } else if ty.starts_with("Warren") {
            // UpdateWarrenEMPField: TakeDamage(100, 'EMP') on each one in range not yet bConfused.
            let extra = (*dmg - 15.0).max(0.0) / 5.0;
            match *ty {
                "WarrenCamera" => {
                    for cm in st.cams.iter_mut().filter(|x| !x.gone && !x.confused && x.pos.distance(*c) < *r) {
                        cm.confusion_dur += extra;
                        cm.confusion_t = 0.0;
                        cm.confused = true;
                        crate::sound::play("DeusExSounds.Weapons.EMPZap", cm.pos, 1.0, 1280.0, 1.0);
                        crate::eggs::device_sparks(cm.pos);
                    }
                }
                "WarrenTurret" => {
                    for t in st.turrets.iter_mut().filter(|x| !x.gone && !x.confused && x.gun_pos.distance(*c) < *r) {
                        t.confusion_dur = (t.confusion_dur + extra).clamp(10.0, 30.0);
                        t.confusion_t = 0.0;
                        t.confused = true;
                        crate::sound::play("DeusExSounds.Weapons.EMPZap", t.gun_pos, 1.0, 1280.0, 1.0);
                        crate::eggs::device_sparks(t.gun_pos);
                    }
                }
                _ => {
                    let beam = *ty == "WarrenBeam";
                    for l in st.lasers.iter_mut().filter(|x| !x.gone && !x.confused && x.beam.is_some() == beam && x.pos.distance(*c) < *r) {
                        l.confusion_t = 0.0;
                        l.confused = true;
                        crate::sound::play("DeusExSounds.Weapons.EMPZap", l.pos, 1.0, 1280.0, 1.0);
                        crate::eggs::device_sparks(l.pos);
                    }
                }
            }
        } else if *ty == "Exploded" {
            for l in st.lasers.iter_mut().filter(|x| !x.gone && x.pos.distance(*c) < *r) {
                let d = l.pos.distance(*c);
                let amount = dmg * (1.0 - (d / r) as f32).max(0.0);
                if amount >= l.min_damage {
                    l.hp -= amount;
                }
            }
        }
    }
    if mission {
        // Mission01: the NSF turrets and the statue laser trap go; the NSF cameras stop raising alarms.
        for t in st.turrets.iter_mut() {
            if t.tag.eq_ignore_ascii_case("NSFTurret01") || t.tag.eq_ignore_ascii_case("NSFTurret02") {
                remove_turret(t, &mut commands, &mut blockers);
            }
        }
        for l in st.lasers.iter_mut().filter(|l| l.tag.eq_ignore_ascii_case("statue_lasertrap")) {
            l.hp = 0.0;
        }
        for c in st.cams.iter_mut() {
            if ["NSFCam01", "NSFCam02", "NSFCam03"].iter().any(|t| c.tag.eq_ignore_ascii_case(t)) {
                c.no_alarm = true;
            }
        }
        println!("SECURITY mission complete: NSF turrets and laser trap removed, NSF cameras quiet");
    }

    // AlarmLight: lit lens (SetLightColor's MultiSkins[1]) or BlackMaskTex.
    for (ai, on) in ALARM_LIGHTS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        let tex: &'static str = if on {
            match find(&level.0.actors[ai].props, "AlarmOnSkin") {
                Some(Value::Path(p)) => Box::leak(p.clone().into_boxed_str()),
                _ => continue,
            }
        } else {
            BLACK
        };
        for (e, r) in refs.iter() {
            if level.0.mesh_actors[r.0].actor != ai {
                continue;
            }
            if let Ok((d, _)) = decos.get(e) {
                let ma = &level.0.mesh_actors[d.mesh_actor];
                let hs: Vec<Handle<Mesh>> = ma_handles(&d.parts, ma.parts.len());
                if let Some(mut sk) = find_skin(e, &hs, &ma.parts, 1, &children, &part_mats) {
                    if !on {
                        sk.shown = None;
                    }
                    set_skin(&mut sk, Some(tex), &mut commands, &mut assets, &mut images, &mut materials);
                }
            }
        }
    }
    // ---------------------------------------------------------------- shared
    let eye = ue(cam.translation) / dx_level::METERS_PER_UNIT as f64;
    let me = player.body.location;
    let me_h = player.params.height;
    let detectable = !ph.dead && !player.noclip && !crate::consys::third_person();
    let radar_trans = crate::abilities::aug("AugRadarTrans").is_some();
    let player_cyl = Cylinder { center: me, radius: player.params.radius, height: me_h, actor: usize::MAX };
    let player_yaw = {
        let f = cam.forward().as_vec3();
        (f.z as f64).atan2(f.x as f64)
    };
    let mut world = World {
        level: lvl,
        blockers: &blockers.list,
        ignore: None,
        pushable: None,
        movers: Some(&movers.rt),
    };
    // Can `from` see the player (the first thing on the line is the player)?
    let sees_player = |world: &World, from: DVec3| -> bool {
        let Some(p) = trace_cylinder(&player_cyl, from, me, DVec3::ZERO) else { return false };
        world.trace(from, me, DVec3::ZERO).fraction > p.fraction
    };
    // Pawns: (blocker, centre, alive, hostile to the player, robot).
    let pawns: Vec<(usize, DVec3, bool)> = npcs
        .iter()
        .filter(|n| n.state != NpcState::Dead && n.body.is_some())
        .map(|n| (n.blocker, n.body.as_ref().unwrap().location, n.hates("Player")))
        .collect();

    // ---------------------------------------------------------------- cameras
    for i in 0..st.cams.len() {
        let c = &mut st.cams[i];
        if c.gone {
            continue;
        }
        // Destroyed (blown up)?
        let alive = c.blocker.map_or(true, |b| blockers.list[b].radius > 0.0)
            && c.entity.map_or(true, |e| decos.get(e).is_ok());
        if !alive {
            c.gone = true;
            alarms.set_external(c.alarm, false, c.pos, None);
            continue;
        }
        if c.lens.is_none() {
            if let Some(e) = c.entity {
                if let Ok((d, _)) = decos.get(e) {
                    let ma = &lvl.mesh_actors[d.mesh_actor];
                    let hs: Vec<Handle<Mesh>> = ma_handles(&d.parts, ma.parts.len());
                    c.lens = find_skin(e, &hs, &ma.parts, 2, &children, &part_mats);
                }
            }
        }
        world.ignore = c.blocker;
        let mut lens: &'static str = GREEN;
        let mut light: Option<u8> = None;
        let mut sound: Option<(&'static str, f32, f32, f32)> = None;
        if !c.active {
            lens = BLACK;
            if c.hum {
                sound = Some(("DeusExSounds.Generic.CameraHum", 192.0 / 255.0, 25.0 * 49.0, 1.0));
            }
        } else if c.confused {
            c.confusion_t += dt;
            if c.confusion_t % 0.25 > 0.2 {
                let a = c.swing_angle;
                c.want = (
                    c.orig.0 + 0.5 * a - rnd(&mut c.seed) * a,
                    c.orig.1 + 0.5 * a - rnd(&mut c.seed) * a,
                );
            }
            lens = YELLOW;
            light = Some(40);
            sound = Some(("DeusExSounds.Generic.CameraHum", 192.0 / 255.0, 25.0 * 49.0, 2.0));
            if c.confusion_t > c.confusion_dur {
                c.confused = false;
                c.confusion_t = 0.0;
                c.confusion_dur = 10.0;
                c.want = c.orig;
            }
        } else {
            light = Some(80);
            sound = Some(("DeusExSounds.Generic.CameraHum", 192.0 / 255.0, 25.0 * 49.0, 1.0));
            if !c.no_alarm {
                c.check_t += dt;
                if c.check_t > 0.1 {
                    c.check_t = 0.0;
                    // CheckPlayerVisibility.
                    let d = me.distance(c.pos);
                    if detectable && !radar_trans && d <= c.range && sees_player(&world, c.pos) {
                        let (rp, ry) = rotator(me - c.pos);
                        let yaw = wrap(c.rot.1 - ry).abs();
                        let pitch = wrap(c.rot.0 - rp).abs();
                        if yaw < c.fov && pitch < c.fov {
                            if c.track {
                                c.want = (rp, ry);
                            }
                            c.last_seen = 0.0;
                            c.seen = true;
                            c.track = true;
                            c.player_loc = me - DVec3::Z * (me_h - 5.0);
                            if !c.triggered && c.trigger_t >= c.trigger_delay {
                                cam_event(c, true);
                                println!("SECURITY camera {} raises the alarm", lvl.actors[c.actor].name);
                            }
                        }
                    }
                }
            }
            if c.seen {
                if !c.triggered {
                    c.trigger_t += dt;
                    if c.trigger_t % 0.5 > 0.4 {
                        lens = RED;
                        light = Some(0);
                        crate::sound::play_owned(
                            0xC_0000_0000 | c.actor as u64,
                            crate::sound::slot::MISC,
                            "DeusExSounds.Generic.Beep6",
                            Some(c.pos),
                            1.0,
                            1280.0,
                            1.0,
                        );
                    }
                } else {
                    lens = RED;
                    light = Some(0);
                }
                if c.last_seen < c.memory {
                    c.last_seen += dt;
                } else {
                    c.last_seen = 0.0;
                    c.seen = false;
                    cam_event(c, false);
                }
            } else {
                c.swing_t += dt;
                if c.swing && !c.track {
                    let ang = std::f64::consts::TAU * c.swing_t as f64 / c.swing_period;
                    c.want = (c.orig.0, c.orig.1 + ang.sin() * c.swing_angle);
                }
            }
            if c.triggered {
                sound = Some(("Ambient.Ambient.Klaxon2", 128.0 / 255.0, 25.0 * 65.0, 1.0));
            }
        }
        // Physics Rotating toward DesiredRotation (RotationRate 65535 both ways).
        turn(&mut c.rot.0, c.want.0, 65535.0, dtd);
        turn(&mut c.rot.1, c.want.1, 65535.0, dtd);
        if let Some(e) = c.entity {
            if let Ok((mut d, _)) = decos.get_mut(e) {
                let r = [c.rot.0.round() as i32, c.rot.1.round() as i32, 0];
                if d.rotation != r {
                    d.rotation = r;
                    d.spun = true;
                }
            }
        }
        if let Some(l) = c.lens.as_mut() {
            set_skin(l, Some(lens), &mut commands, &mut assets, &mut images, &mut materials);
        }
        if let Some(h) = light {
            if c.pos.distance(eye) < 4000.0 {
                crate::dynlight::hold(
                    0xC_0000_0000 | c.actor as u64,
                    c.pos,
                    crate::dynlight::Spec::new(h, 100, 120, 1.0),
                    0.0,
                );
            }
        }
        if let Some((snd, vol, radius, pitch)) = sound {
            if c.pos.distance(eye) < radius as f64 {
                crate::sound::hold(0xC_1000_0000 | c.actor as u64, snd, Some(c.pos), vol, radius, pitch);
            }
        }
        let seek = c.player_loc;
        alarms.set_external(c.alarm, c.triggered, seek, Some(Target::Player));
    }

    // ---------------------------------------------------------------- turrets
    let mut dead_turrets: Vec<usize> = Vec::new();
    for i in 0..st.turrets.len() {
        let t = &mut st.turrets[i];
        if t.gone {
            continue;
        }
        // The base and the gun go together (AutoTurret.Destroyed / AutoTurretGun.Destroyed).
        let base_ok = t.base_blocker.map_or(true, |b| blockers.list[b].radius > 0.0)
            && t.base_entity.map_or(true, |e| decos.get(e).is_ok());
        let gun_ok = blockers.list[t.gun_blocker].radius > 0.0 && decos.get(t.gun_entity).is_ok();
        if !base_ok || !gun_ok {
            dead_turrets.push(i);
            continue;
        }
        if t.lens.is_none() || t.muzzle.is_none() {
            t.lens = find_skin(t.gun_entity, &t.handles, &t.parts, 1, &children, &part_mats);
            t.muzzle = find_skin(t.gun_entity, &t.handles, &t.parts, 2, &children, &part_mats);
        }
        // AlarmHeard: an alarm going off nearby wakes it up until the alarm ends.
        let heard = alarms.heard_at(t.gun_pos);
        if heard && !t.heard && !t.active {
            t.pre_alarm = t.active;
            t.active = true;
            t.hum = true;
        } else if !heard && t.heard && t.active {
            t.active = t.pre_alarm;
        }
        t.heard = heard;
        if t.confused {
            t.confusion_t += dt;
            if t.confusion_t % 0.25 > 0.2 {
                t.want = (
                    t.orig.0 + (t.pitch_limit / 2.0 - rnd(&mut t.seed) * t.pitch_limit),
                    rnd(&mut t.seed) * URU,
                );
            }
            if t.confusion_t > t.confusion_dur {
                t.confused = false;
                t.confusion_t = 0.0;
                t.confusion_dur = 10.0;
            }
        }
        let on = t.active && !t.disabled;
        let mut dest = t.want;
        if on {
            t.target = None;
            if !t.confused {
                world.ignore = Some(t.gun_blocker);
                let clear = |to: DVec3| {
                    let w = World { level: lvl, blockers: &[], ignore: None, pushable: None, movers: Some(&movers.rt) };
                    w.trace(t.gun_pos, to, DVec3::ZERO).fraction >= 0.999
                };
                let in_range = |p: DVec3| p.distance(t.gun_pos) < t.max_range;
                // bTrackPlayersOnly (or neither): you, or a pawn that isn't hostile to you.
                if t.players_only || !t.pawns_only {
                    // A player at a computer is bIgnore (MakePlayerIgnored): turrets skip him.
                    if detectable && !radar_trans && !crate::computers::player_ignored() && in_range(me) && clear(me) {
                        t.target = Some(TurretTarget::Player);
                    } else if let Some(p) = pawns
                        .iter()
                        .filter(|p| !p.2 && in_range(p.1) && clear(p.1))
                        .min_by(|a, b| a.1.distance(t.gun_pos).total_cmp(&b.1.distance(t.gun_pos)))
                    {
                        t.target = Some(TurretTarget::Pawn(p.0));
                    }
                }
                if !t.players_only {
                    if !t.pawns_only {
                        // Decorations (not other electronics, not indestructible ones).
                        for (b, c) in blockers.list.iter().enumerate() {
                            if c.radius <= 0.0 || !in_range(c.center) || b == t.gun_blocker || Some(b) == t.base_blocker {
                                continue;
                            }
                            let a = &lvl.actors[c.actor];
                            if find(&a.props, "GroundSpeed").is_some()
                                || find(&a.props, "bPushable").is_none()
                                || electronic(&a.class)
                                || a.class.starts_with("AutoTurret")
                                || bv(&a.props, "bInvincible", false)
                                || !clear(c.center)
                            {
                                continue;
                            }
                            t.target = Some(TurretTarget::Deco(b));
                            break;
                        }
                    }
                    if let Some(p) = pawns
                        .iter()
                        .filter(|p| p.2 && in_range(p.1) && clear(p.1))
                        .min_by(|a, b| a.1.distance(t.gun_pos).total_cmp(&b.1.distance(t.gun_pos)))
                    {
                        t.target = Some(TurretTarget::Pawn(p.0));
                    }
                }
                let target_pos = |tt: TurretTarget| match tt {
                    TurretTarget::Player => me,
                    TurretTarget::Pawn(b) | TurretTarget::Deco(b) => blockers.list[b].center,
                };
                match t.target {
                    Some(tt) => {
                        let (p, y) = rotator(target_pos(tt) - t.gun_pos);
                        dest = (p, y);
                        let near = t.pitch_limit / 2.0;
                        t.want = (p.clamp(t.orig.0 - near, t.orig.0 + near), y);
                    }
                    None => t.want = t.orig,
                }
            }
        } else if !t.confused {
            t.want = t.orig;
        }
        // Physics Rotating (16384 / s).
        let before = t.rot;
        turn(&mut t.rot.0, t.want.0, 16384.0, dtd);
        turn(&mut t.rot.1, t.want.1, 16384.0, dtd);
        let near = wrap(t.rot.0 - t.want.0).abs() + wrap(t.rot.1 - t.want.1).abs();
        let mut lens: Option<&'static str> = None;
        if on {
            if t.target.is_some() && t.target != t.last_target {
                crate::sound::play("DeusExSounds.Generic.Beep6", t.gun_pos, 1.0, 1280.0, 1.0);
            }
            if t.target.is_some() {
                lens = Some(RED);
                if near < 4096.0 && wrap(t.rot.0 - dest.0).abs() < 8192.0 && t.fire_t > t.fire_rate {
                    fire(t, &world, lvl, &blockers, &player_cyl, player_yaw, &mut ph, &mut pawn_hits, &mut deco_hits, &mut shells, &mut noises);
                    t.fire_t = 0.0;
                }
            } else {
                t.firing = false;
                lens = Some(if t.confused { YELLOW } else { GREEN });
            }
            t.fire_t += dt;
            t.last_target = t.target;
        } else {
            t.firing = false;
            t.target = None;
        }
        // The gun: rotation, Fire loop while shooting, the lens and the muzzle flash skin.
        if let Ok((mut d, _)) = decos.get_mut(t.gun_entity) {
            let r = [t.rot.0.round() as i32, t.rot.1.round() as i32, 0];
            if d.rotation != r {
                d.rotation = r;
                d.spun = true;
            }
        }
        animate_gun(t, dt, &mut meshes);
        if let Some(l) = t.lens.as_mut() {
            set_skin(l, lens, &mut commands, &mut assets, &mut images, &mut materials);
        }
        t.flash_t -= dt;
        if let Some(m) = t.muzzle.as_mut() {
            set_skin(m, if t.flash_t > 0.0 { Some(FLASH) } else { None }, &mut commands, &mut assets, &mut images, &mut materials);
        }
        if t.flash_t > 0.0 {
            crate::dynlight::hold(0xD_1000_0000 | t.actor as u64, t.gun_pos, crate::dynlight::MUZZLE_FLASH, 0.0);
        }
        // AutoTurretHum on the base while it's on; AutoTurretMove while the gun turns.
        let base_pos = t.base_blocker.map(|b| blockers.list[b].center).unwrap_or(t.gun_pos);
        if t.hum && base_pos.distance(eye) < 1225.0 {
            crate::sound::hold(0xC_2000_0000 | t.actor as u64, "DeusExSounds.Generic.AutoTurretHum", Some(base_pos), 192.0 / 255.0, 1225.0, 1.0);
        }
        let moving = wrap(t.rot.0 - before.0).abs() + wrap(t.rot.1 - before.1).abs() > 0.5 && near > 64.0;
        if moving && t.gun_pos.distance(eye) < 625.0 {
            crate::sound::hold(
                0xC_3000_0000 | t.actor as u64,
                "DeusExSounds.Generic.AutoTurretMove",
                Some(t.gun_pos),
                0.5,
                625.0,
                if t.confused { 2.0 } else { 1.0 },
            );
        }
    }

    // ---------------------------------------------------------------- lasers
    let (beam_mesh, red_beam, spot_mesh, red_spot) = (
        st.beam_mesh.clone().unwrap(),
        st.beam_mat.clone().unwrap(),
        st.spot_mesh.clone().unwrap(),
        st.spot_mat.clone().unwrap(),
    );
    let (blue_beam, blue_spot) = (st.beam2_mat.clone().unwrap(), st.spot2_mat.clone().unwrap());
    for i in 0..st.lasers.len() {
        let l = &mut st.lasers[i];
        if l.gone {
            continue;
        }
        if l.hp <= 0.0 {
            // Destroyed (blown up, or removed by the mission script).
            l.gone = true;
            for e in l.beams.drain(..).chain(l.spots.drain(..)) {
                commands.entity(e).despawn();
            }
            if let Some(e) = l.emitter_entity.take() {
                commands.entity(e).despawn();
            }
            alarms.set_external(l.alarm, false, l.pos, None);
            continue;
        }
        if l.lens.is_none() {
            if let Some(e) = l.emitter_entity {
                let mi = refs.get(e).ok().map(|(_, r)| r.0);
                if let Some(mi) = mi {
                    let ma = &lvl.mesh_actors[mi];
                    let hs = child_handles(e, &children, &part_mats, ma.parts.len());
                    l.lens = find_skin_any(e, &hs, &ma.parts, 1, &children, &part_mats);
                }
            }
        }
        // Alarm timeout.
        if let Some(t0) = l.alarm_t.as_mut() {
            *t0 += dt;
            if *t0 >= l.alarm_timeout {
                l.alarm_t = None;
            }
        }
        // EMP: the beam flickers (on 5% of frames) until it wears off.
        let mut beam_on = l.on;
        if l.confused && l.on {
            l.confusion_t += dt;
            beam_on = rnd(&mut l.seed) > 0.95;
            if l.confusion_t > l.confusion_dur {
                l.confused = false;
                l.confusion_t = 0.0;
                beam_on = true;
            }
        }
        // bFrozen: not traced or drawn far from the player.
        let near = l.pos.distance(me) < 960.0 || l.pos.distance(eye) < 3000.0;
        let mut hit_now: Option<Broke> = None;
        let mut segs: Vec<(DVec3, DVec3)> = Vec::new();
        let mut spot_at: Vec<(DVec3, DVec3)> = Vec::new();
        if beam_on && near {
            // CalcTrace: the first thing in the way along the beam; mirrors (PF_Mirrored) bounce it twice.
            let ax = rotator_axes(l.rot);
            let mut start = l.pos;
            let mut d = DVec3::new(ax[0][0] as f64, ax[0][1] as f64, ax[0][2] as f64).normalize_or_zero();
            for _bounce in 0..2 {
                let end = start + d * 5000.0;
                let w = World { level: lvl, blockers: &blockers.list, ignore: None, pushable: None, movers: Some(&movers.rt) };
                let h = w.trace(start, end, DVec3::ZERO);
                let mut f = h.fraction;
                let mut normal = h.normal;
                let mut who = h.actor.map(Broke::Blocker);
                if let Some(p) = trace_cylinder(&player_cyl, start, end, DVec3::ZERO) {
                    if p.fraction < f && detectable {
                        f = p.fraction;
                        normal = p.normal;
                        who = Some(Broke::Player);
                    }
                }
                let at = start + (end - start) * f;
                segs.push((start, at));
                spot_at.push((at, normal));
                if who.is_some() {
                    hit_now = who;
                    break;
                }
                // PF_Mirrored surfaces would bounce it (up to twice); the level's surface flags aren't kept per
                // wall yet and the Island has no mirrors in a beam's path.
                let mirror = false;
                if !mirror {
                    break;
                }
                d = (d - 2.0 * d.dot(normal) * normal).normalize_or_zero();
                start = at + normal;
            }
            // Tick: a newly broken beam by something that counts sounds the alarm.
            if let (Some(be), Some(who)) = (l.beam.as_mut(), hit_now) {
                // BeamTrigger.Tick: a newly broken beam by something IsRelevant takes (adaptive armor hides the
                // player) beeps and triggers Event.
                if Some(who) != l.last_hit {
                    let relevant = match who {
                        Broke::Player => !ph.camo,
                        Broke::Blocker(b) => {
                            let a = &lvl.actors[blockers.list[b].actor];
                            let pawn = find(&a.props, "GroundSpeed").is_some();
                            match l.trigger_type {
                                1 => pawn,
                                3 => true,
                                _ => false,
                            }
                        }
                    };
                    if relevant {
                        crate::sound::play("DeusExSounds.Generic.Beep2", l.pos, 1.0, 1280.0, 3.0);
                        if !be.done {
                            be.done = be.once;
                            println!("SECURITY beam {} broken -> {}", lvl.actors[l.actor].name, be.event);
                            if !be.event.is_empty() {
                                if let Ok(mut q) = crate::movers::EVENTS_OUT.lock() {
                                    q.push((be.event.clone(), true));
                                }
                            }
                        }
                    }
                }
            } else if !l.no_alarm && hit_now.is_some() && hit_now != l.last_hit {
                let relevant = match hit_now.unwrap() {
                    Broke::Player => !ph.camo,
                    Broke::Blocker(b) => {
                        let a = &lvl.actors[blockers.list[b].actor];
                        let pawn = find(&a.props, "GroundSpeed").is_some();
                        // TT_PlayerProximity: decorations too; TT_PawnProximity / Any: pawns.
                        match l.trigger_type {
                            0 => !pawn,
                            1 => pawn,
                            3 => true,
                            _ => false,
                        }
                    }
                };
                if relevant {
                    if l.alarm_t.is_none() {
                        let (who_pos, who_h) = match hit_now.unwrap() {
                            Broke::Player => (me, me_h),
                            Broke::Blocker(b) => (blockers.list[b].center, blockers.list[b].height),
                        };
                        l.seek = who_pos - DVec3::Z * (who_h - 1.0);
                        l.alarm_t = Some(0.0);
                        println!("SECURITY laser {} tripped", lvl.actors[l.actor].name);
                    }
                    crate::sound::play("DeusExSounds.Generic.Beep2", l.pos, 1.0, 1280.0, 3.0);
                }
            }
            l.last_hit = hit_now;
        } else if !beam_on {
            l.last_hit = None;
        }
        // Beam and spot visuals.
        let (beam_mat, spot_mat) = if l.beam.is_some() { (&blue_beam, &blue_spot) } else { (&red_beam, &red_spot) };
        for k in 0..2 {
            if l.beams.len() <= k {
                let b = commands
                    .spawn((Mesh3d(beam_mesh.clone()), MeshMaterial3d(beam_mat.clone()), Transform::default(), Visibility::Hidden, LaserFx))
                    .id();
                let s = commands
                    .spawn((Mesh3d(spot_mesh.clone()), MeshMaterial3d(spot_mat.clone()), Transform::default(), Visibility::Hidden, LaserFx))
                    .id();
                l.beams.push(b);
                l.spots.push(s);
                continue;
            }
            let show = beam_on && near && k < segs.len();
            if let Ok((mut tf, mut vis)) = fx.get_mut(l.beams[k]) {
                if show {
                    let (a, b) = segs[k];
                    let mid = (a + b) * 0.5;
                    let len = a.distance(b).max(0.1);
                    let dv = Vec3::from(ue_to_bevy(((b - a) / len).as_vec3().to_array()));
                    *tf = Transform::from_translation(Vec3::from(ue_to_bevy(mid.as_vec3().to_array())))
                        .with_rotation(Quat::from_rotation_arc(Vec3::Y, dv.normalize_or(Vec3::Y)))
                        .with_scale(Vec3::new(0.65, len as f32, 0.65) * dx_level::METERS_PER_UNIT);
                    if *vis != Visibility::Inherited {
                        *vis = Visibility::Inherited;
                    }
                } else if *vis != Visibility::Hidden {
                    *vis = Visibility::Hidden;
                }
            }
            if let Ok((mut tf, mut vis)) = fx.get_mut(l.spots[k]) {
                if show {
                    let (at, n) = spot_at[k];
                    let nb = Vec3::from(ue_to_bevy(n.as_vec3().to_array())).normalize_or(Vec3::Y);
                    *tf = Transform::from_translation(Vec3::from(ue_to_bevy((at + n * 0.5).as_vec3().to_array())))
                        .with_rotation(Quat::from_rotation_arc(Vec3::Z, nb))
                        .with_scale(Vec3::splat(2.0 * dx_level::METERS_PER_UNIT));
                    if *vis != Visibility::Inherited {
                        *vis = Visibility::Inherited;
                    }
                } else if *vis != Visibility::Hidden {
                    *vis = Visibility::Hidden;
                }
            }
        }
        if let Some(s) = l.lens.as_mut() {
            set_skin(s, Some(if l.on { "Effects.Laser.LaserSpot1" } else { BLACK }), &mut commands, &mut assets, &mut images, &mut materials);
        }
        // The emitter's hum (Ambient.Laser, radius 25 x 17) while the beam is on; the klaxon while alarmed.
        if beam_on && l.pos.distance(eye) < 425.0 {
            crate::sound::hold(0xC_4000_0000 | l.actor as u64, "Ambient.Ambient.Laser", Some(l.pos), 0.5, 425.0, 1.0);
        }
        if l.alarm_t.is_some() && l.pos.distance(eye) < 1625.0 {
            crate::sound::hold(0xC_5000_0000 | l.actor as u64, "Ambient.Ambient.Klaxon2", Some(l.pos), 0.5, 1625.0, 1.0);
        }
        let instigator = if l.last_hit == Some(Broke::Player) || l.alarm_t.is_some() { Some(Target::Player) } else { None };
        alarms.set_external(l.alarm, l.alarm_t.is_some(), l.seek, instigator);
    }
    let _ = world;
    for i in dead_turrets {
        remove_turret(&mut st.turrets[i], &mut commands, &mut blockers);
    }
}

/// Deco.parts holds (mesh handle, part index): a handle per part index.
fn ma_handles(parts: &[(Handle<Mesh>, usize)], n: usize) -> Vec<Handle<Mesh>> {
    let mut v = vec![Handle::default(); n];
    for (h, i) in parts {
        if *i < n {
            v[*i] = h.clone();
        }
    }
    v
}

/// For mesh entities without a Deco: their children's mesh handles in part order (spawn_mesh_actors skips empty
/// parts, so this matches by order of the non-empty ones).
fn child_handles(
    root: Entity,
    children: &Query<&Children>,
    mats: &Query<(&Mesh3d, &MeshMaterial3d<StandardMaterial>)>,
    _n: usize,
) -> Vec<Handle<Mesh>> {
    let Ok(kids) = children.get(root) else { return Vec::new() };
    kids.iter().filter_map(|c| mats.get(c).ok().map(|(m, _)| m.0.clone())).collect()
}

/// find_skin where `handles` lists only the non-empty parts in order.
fn find_skin_any(
    root: Entity,
    handles: &[Handle<Mesh>],
    parts: &[dx_level::actors::MeshPart],
    slot: u32,
    children: &Query<&Children>,
    mats: &Query<(&Mesh3d, &MeshMaterial3d<StandardMaterial>)>,
) -> Option<Skin> {
    let nonempty: Vec<usize> = (0..parts.len()).filter(|&i| !parts[i].indices.is_empty()).collect();
    let k = nonempty.iter().position(|&i| parts[i].slots.contains(&slot))?;
    let want = handles.get(k)?;
    let kids = children.get(root).ok()?;
    for c in kids.iter() {
        if let Ok((m, mat)) = mats.get(c) {
            if m.0 == *want {
                return Some(Skin { child: c, original: mat.0.clone(), class: parts[nonempty[k]].class, shown: None });
            }
        }
    }
    None
}


fn cam_trigger(c: &mut Cam, by_player: bool) {
    if c.confused {
        return;
    }
    if !c.active {
        if by_player {
            crate::classic_hud::client_message(&c.msg_on);
        }
        c.active = true;
        c.hum = true;
    }
}

fn cam_untrigger(c: &mut Cam, by_player: bool) {
    if c.confused {
        return;
    }
    if c.active {
        if by_player {
            crate::classic_hud::client_message(&c.msg_off);
        }
        cam_event(c, false);
        c.active = false;
        c.hum = false;
        c.want = c.orig;
        c.seen = false;
        c.track = false;
    }
}

/// SecurityCamera.TriggerEvent.
fn cam_event(c: &mut Cam, on: bool) {
    c.triggered = on;
    c.track = on;
    c.trigger_t = 0.0;
}

fn turret_trigger(t: &mut Turret) {
    if t.confused || t.disabled {
        return;
    }
    if !t.active {
        t.active = true;
        t.hum = true;
    }
}

fn turret_untrigger(t: &mut Turret) {
    if t.confused || t.disabled {
        return;
    }
    if t.active {
        t.active = false;
        t.hum = false;
    }
}

fn remove_turret(t: &mut Turret, commands: &mut Commands, blockers: &mut Blockers) {
    if t.gone {
        return;
    }
    t.gone = true;
    for b in [Some(t.gun_blocker), t.base_blocker].into_iter().flatten() {
        if let Some(c) = blockers.list.get_mut(b) {
            c.radius = 0.0;
            c.height = 0.0;
        }
        if let Some(e) = blockers.deco.remove(&b) {
            commands.entity(e).despawn();
        }
    }
    if let Some(e) = t.base_entity.take() {
        commands.entity(e).despawn();
    }
    commands.entity(t.gun_entity).despawn();
}

/// AutoTurret.Fire: one trace from the gun along where it points, spread by gunAccuracy.
#[allow(clippy::too_many_arguments)]
fn fire(
    t: &mut Turret,
    world: &World,
    lvl: &dx_level::Level,
    blockers: &Blockers,
    player_cyl: &Cylinder,
    player_yaw: f64,
    ph: &mut PlayerHealth,
    pawn_hits: &mut crate::damage::PawnHits,
    deco_hits: &mut crate::destruct::DecoHits,
    shells: &mut crate::destruct::Shells,
    noises: &mut crate::stealth::Noises,
) {
    t.firing = true;
    let ax = rotator_axes([t.rot.0.round() as i32, t.rot.1.round() as i32, 0]);
    let x = DVec3::new(ax[0][0] as f64, ax[0][1] as f64, ax[0][2] as f64);
    let y = DVec3::new(ax[1][0] as f64, ax[1][1] as f64, ax[1][2] as f64);
    let z = DVec3::new(ax[2][0] as f64, ax[2][1] as f64, ax[2][2] as f64);
    let start = t.gun_pos;
    let end = start
        + y * (t.accuracy * (rnd(&mut t.seed) - 0.5) * 1000.0)
        + z * (t.accuracy * (rnd(&mut t.seed) - 0.5) * 1000.0)
        + x * 10000.0;
    let h = world.trace(start, end, DVec3::ZERO);
    let mut f = h.fraction;
    let mut player_hit = false;
    if let Some(p) = trace_cylinder(player_cyl, start, end, DVec3::ZERO) {
        if p.fraction < f && !ph.dead {
            f = p.fraction;
            player_hit = true;
        }
    }
    let at = start + (end - start) * f;
    // A shell casing out of the side, PistolFire, the muzzle flash (0.1 s), and noise.
    let side = rotator_axes([0, (t.rot.1 - 16384.0).round() as i32, 0]);
    shells.0.push((
        "ShellCasing",
        start,
        DVec3::new(side[0][0] as f64, side[0][1] as f64, side[0][2] as f64) * 100.0,
    ));
    crate::sound::play("DeusExSounds.Weapons.PistolFire", start, 1.0, 1600.0, 1.0);
    noises.0.push(crate::stealth::Noise {
        pos: start,
        volume: 1.0,
        radius: 1600.0,
        maker: None,
        instigator: None,
        kind: crate::stealth::NoiseKind::Loud,
    });
    t.flash_t = 0.1;
    if player_hit {
        ph.damage(t.damage, "AutoShot", crate::ai::player_offset(player_cyl.center, player_yaw, at));
        return;
    }
    if f >= 1.0 {
        return;
    }
    // A ricochet where it lands.
    let snd = match h.actor.map(|b| &lvl.actors[blockers.list[b].actor].props) {
        Some(p) if fv(p, "minDamageThreshold", 0.0) > 10.0 || find(p, "EMPHitPoints").is_some() => {
            "DeusExSounds.Generic.ArmorRicochet".to_string()
        }
        _ => format!("DeusExSounds.Generic.Ricochet{}", 1 + (rnd(&mut t.seed) * 4.0) as u32 % 4),
    };
    crate::sound::play(&snd, at, 1.0, 1024.0, (1.1 - 0.2 * rnd(&mut t.seed)) as f32);
    match h.actor {
        Some(b) => {
            let a = &lvl.actors[blockers.list[b].actor];
            if find(&a.props, "GroundSpeed").is_some() {
                // The pawn it meant to shoot blames the player (attacker = GetPlayerPawn()).
                pawn_hits.0.push(crate::damage::PawnHit {
                    blocker: b,
                    damage: t.damage,
                    damage_type: "AutoShot",
                    hit_loc: at,
                    attacker: None,
                    attacker_pos: start,
                });
            } else {
                deco_hits.0.push(crate::destruct::DecoHit { blocker: b, damage: t.damage, damage_type: "Shot" });
            }
        }
        None => crate::movers::hit_line(start, at, t.damage, "Shot"),
    }
}

/// The gun's Fire loop while it shoots, Still otherwise (vertex animation).
fn animate_gun(t: &mut Turret, dt: f32, meshes: &mut Assets<Mesh>) {
    let Some(am) = t.mesh.clone() else { return };
    let want = if t.firing { "Fire" } else { "Still" };
    let need_new = match &t.clip {
        None => true,
        Some(c) => (c.looping) != t.firing,
    };
    if need_new {
        t.clip = Clip::named(&am, want, t.firing);
        if t.clip.is_none() {
            return;
        }
    }
    let Some(c) = t.clip.as_mut() else { return };
    if !t.firing && c.t > 0.0 {
        return;
    }
    c.t += dt;
    let fr = c.frame();
    for (pi, hm) in t.handles.iter().enumerate() {
        if let Some(mut mm) = meshes.get_mut(hm) {
            let part = &t.parts[pi];
            if part.vert_ids.is_empty() {
                continue;
            }
            let pos: Vec<[f32; 3]> = part
                .vert_ids
                .iter()
                .map(|&v| ue_to_bevy(am.sample(c.start, c.frames, fr, v as usize)))
                .collect();
            mm.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn init(
    st: &mut Security,
    commands: &mut Commands,
    lvl: &mut dx_level::Level,
    blockers: &mut Blockers,
    alarms: &mut Alarms,
    refs: &Query<(Entity, &MeshActorRef)>,
    assets: &mut RuntimeAssets,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let by_actor: HashMap<usize, usize> = blockers.list.iter().enumerate().map(|(b, c)| (c.actor, b)).collect();
    let mesh_entity: HashMap<usize, Entity> =
        refs.iter().filter_map(|(e, r)| lvl.mesh_actors.get(r.0).map(|m| (m.actor, e))).collect();
    // Beam and spot assets.
    st.beam_mesh = Some(meshes.add(bevy::math::primitives::Cylinder::new(0.5, 1.0)));
    let beam_img = image_for(assets, images, "Effects.Laser.LaserBeam1");
    st.beam_mat = Some(materials.add(StandardMaterial {
        base_color: if beam_img.is_some() { Color::WHITE } else { Color::srgb(1.0, 0.1, 0.05) },
        base_color_texture: beam_img,
        unlit: true,
        double_sided: true,
        cull_mode: None,
        alpha_mode: AlphaMode::Add,
        ..default()
    }));
    let blue = |tex: &str, assets: &mut RuntimeAssets, images: &mut Assets<Image>| {
        let img = image_for(assets, images, tex);
        StandardMaterial {
            base_color: if img.is_some() { Color::WHITE } else { Color::srgb(0.1, 0.3, 1.0) },
            base_color_texture: img,
            unlit: true,
            double_sided: true,
            cull_mode: None,
            alpha_mode: AlphaMode::Add,
            ..default()
        }
    };
    st.beam2_mat = Some(materials.add(blue("Effects.Laser.LaserBeam2", assets, images)));
    st.spot2_mat = Some(materials.add(blue("Effects.Laser.LaserSpot2", assets, images)));
    st.spot_mesh = Some(meshes.add(Rectangle::new(1.0, 1.0)));
    let spot_img = image_for(assets, images, "Effects.Laser.LaserSpot1");
    st.spot_mat = Some(materials.add(StandardMaterial {
        base_color: if spot_img.is_some() { Color::WHITE } else { Color::srgb(1.0, 0.1, 0.05) },
        base_color_texture: spot_img,
        unlit: true,
        double_sided: true,
        cull_mode: None,
        alpha_mode: AlphaMode::Add,
        ..default()
    }));

    let n = lvl.actors.len();
    let mut new_guns: Vec<(usize, &'static str)> = Vec::new();
    for i in 0..n {
        let class = lvl.actors[i].class.clone();
        let p = lvl.actors[i].props.clone();
        match class.as_str() {
            "SecurityCamera" => {
                let r = rot_of(&p);
                let pos = loc_of(&p);
                let blocker = by_actor.get(&i).copied();
                let entity = blocker.and_then(|b| blockers.deco.get(&b).copied());
                crate::sound::own_ambient(i);
                crate::dynlight::suppress_actor_light(i);
                let alarm = alarms.add_external(i, pos);
                let d = assets.classes.defaults("SecurityCamera");
                st.cams.push(Cam {
                    actor: i,
                    blocker,
                    entity,
                    pos,
                    tag: crate::devices::name_prop(&p, "Tag"),
                    orig: (r[0] as f64, r[1] as f64),
                    rot: (r[0] as f64, r[1] as f64),
                    want: (r[0] as f64, r[1] as f64),
                    swing: bv(&p, "bSwing", false),
                    swing_angle: fv(&p, "swingAngle", 8192.0),
                    swing_period: fv(&p, "swingPeriod", 8.0).max(0.1),
                    fov: fv(&p, "cameraFOV", 4096.0),
                    range: fv(&p, "cameraRange", 1024.0),
                    memory: fv(&p, "memoryTime", 5.0) as f32,
                    trigger_delay: fv(&p, "triggerDelay", 4.0) as f32,
                    active: bv(&p, "bActive", true),
                    no_alarm: bv(&p, "bNoAlarm", false),
                    track: false,
                    seen: false,
                    triggered: false,
                    last_seen: 0.0,
                    check_t: 0.0,
                    swing_t: 0.0,
                    trigger_t: 0.0,
                    confused: false,
                    confusion_t: 0.0,
                    confusion_dur: fv(&p, "confusionDuration", 10.0) as f32,
                    player_loc: pos,
                    lens: None,
                    alarm,
                    hum: true,
                    seed: 0xCA3E_0000 ^ i as u64,
                    gone: false,
                    msg_on: { let m = sv(&d, "msgActivated"); if m.is_empty() { "Camera activated".into() } else { m } },
                    msg_off: { let m = sv(&d, "msgDeactivated"); if m.is_empty() { "Camera deactivated".into() } else { m } },
                });
            }
            "AutoTurret" => new_guns.push((i, "AutoTurretGun")),
            "AutoTurretSmall" => new_guns.push((i, "AutoTurretGunSmall")),
            "LaserTrigger" | "BeamTrigger" => {
                let pos = loc_of(&p);
                let beam = (class == "BeamTrigger").then(|| BeamEvent {
                    event: crate::devices::name_prop(&p, "Event"),
                    once: bv(&p, "bTriggerOnceOnly", false),
                    done: false,
                });
                let alarm = alarms.add_external(i, pos);
                st.lasers.push(Laser {
                    actor: i,
                    tag: crate::devices::name_prop(&p, "Tag"),
                    pos,
                    rot: rot_of(&p),
                    on: bv(&p, "bIsOn", true),
                    no_alarm: bv(&p, "bNoAlarm", false),
                    trigger_type: fv(&p, "TriggerType", 3.0) as u8,
                    confused: false,
                    confusion_t: 0.0,
                    confusion_dur: fv(&p, "confusionDuration", 10.0) as f32,
                    hp: fv(&p, "HitPoints", 50.0) as f32,
                    min_damage: fv(&p, "minDamageThreshold", 50.0) as f32,
                    alarm_timeout: fv(&p, "alarmTimeout", 30.0) as f32,
                    alarm_t: None,
                    last_hit: None,
                    seed: 0x1A5E_0000 ^ i as u64,
                    beams: Vec::new(),
                    spots: Vec::new(),
                    emitter_entity: mesh_entity.get(&i).copied(),
                    lens: None,
                    alarm,
                    seek: pos,
                    gone: false,
                    beam,
                });
            }
            _ => {}
        }
    }
    // AutoTurret.PreBeginPlay: spawn the gun on the base (CollisionHeight of both along the base's up axis),
    // facing the base's yaw.
    for (i, gun_class) in new_guns {
        let p = lvl.actors[i].props.clone();
        let base_loc = loc_of(&p);
        let base_rot = rot_of(&p);
        let base_h = fv(&p, "CollisionHeight", 20.2);
        let gd = assets.classes.defaults(gun_class);
        let gun_h = fv(&gd, "CollisionHeight", 9.1);
        let ax = rotator_axes(base_rot);
        let up = DVec3::new(ax[2][0] as f64, ax[2][1] as f64, ax[2][2] as f64);
        let gun_pos = base_loc + up * (base_h + gun_h);
        let rot = [0, base_rot[1], 0];
        let mut props = gd.clone();
        let set = |props: &mut Vec<Property>, n: &str, v: Value| {
            props.retain(|x| !(x.name.eq_ignore_ascii_case(n) && x.index == 0));
            props.push(Property { name: n.into(), index: 0, value: v });
        };
        set(&mut props, "Location", Value::Vector([gun_pos.x as f32, gun_pos.y as f32, gun_pos.z as f32]));
        set(&mut props, "Rotation", Value::Rotator(rot));
        let Some(Value::Path(mesh_path)) = find(&props, "Mesh").cloned() else { continue };
        let pre = match find(&props, "PrePivot") {
            Some(Value::Vector(v)) => DVec3::new(v[0] as f64, v[1] as f64, v[2] as f64),
            _ => DVec3::ZERO,
        };
        let draw = gun_pos + pre;
        let scale = fv(&props, "DrawScale", 1.0) as f32;
        let m = dx_level::actors::rotator_to_bevy_mat3(rot);
        let root = commands
            .spawn((
                Transform::from_translation(Vec3::from(ue_to_bevy(draw.as_vec3().to_array()))).with_rotation(
                    Quat::from_mat3(&Mat3::from_cols(Vec3::from(m[0]), Vec3::from(m[1]), Vec3::from(m[2]))),
                ),
                Visibility::default(),
            ))
            .id();
        let map = |q: [f32; 3]| ue_to_bevy(q);
        let Some((lm, parts, handles)) =
            spawn_mesh_parts(&mesh_path, scale, &props, assets, images, meshes, materials, commands, root, &map, None)
        else {
            commands.entity(root).despawn();
            continue;
        };
        let ai = lvl.actors.len();
        lvl.actors.push(dx_level::actors::ActorInfo {
            name: format!("{}{}", gun_class, i),
            class: gun_class.to_string(),
            export: -1,
            props: props.clone(),
        });
        let mi = lvl.mesh_actors.len();
        lvl.mesh_actors.push(MeshActor {
            actor: ai,
            parts: parts.clone(),
            location: draw.as_vec3().to_array(),
            rotation: rot,
            scale_glow: 1.0,
            anim: None,
        });
        let radius = fv(&props, "CollisionRadius", 22.5);
        let b = blockers.list.len();
        blockers.list.push(Cylinder { center: gun_pos, radius, height: gun_h, actor: ai });
        blockers.deco.insert(b, root);
        let part_refs: Vec<(Handle<Mesh>, usize)> = handles.iter().cloned().enumerate().map(|(k, h)| (h, k)).collect();
        commands.entity(root).insert((
            MeshActorRef(mi),
            Deco {
                body: Body {
                    location: gun_pos,
                    velocity: DVec3::ZERO,
                    physics: BodyPhysics::None,
                    radius,
                    height: gun_h,
                    landed_vz: None,
                },
                blocker: Some(b),
                mass: fv(&props, "Mass", 50.0),
                pushable: false,
                pre,
                rotation: rot,
                scale_glow: 1.0,
                parts: part_refs,
                mesh_actor: mi,
                last_lit: DVec3::splat(1e9),
                spun: true,
                name: format!("{}{}", gun_class, i),
                start: gun_pos,
                buoyancy: 0.0,
                floating: false,
                orig_rot: rot,
                thrown_by_player: false,
                thrown_body: false,
                push_snd: 0.0,
                falls: false,
            },
        ));
        let base_blocker = by_actor.get(&i).copied();
        crate::sound::own_ambient(i);
        let am = Arc::new(anim_mesh(&lm));
        let seed = 0x7A11_0000 ^ i as u64;
        st.turrets.push(Turret {
            actor: i,
            tag: crate::devices::name_prop(&p, "Tag"),
            base_blocker,
            base_entity: base_blocker.and_then(|bb| blockers.deco.get(&bb).copied()),
            gun_actor: ai,
            gun_blocker: b,
            gun_entity: root,
            gun_pos,
            orig: (0.0, base_rot[1] as f64),
            rot: (0.0, base_rot[1] as f64),
            want: (0.0, base_rot[1] as f64),
            active: bv(&p, "bActive", true),
            pre_alarm: bv(&p, "bActive", true),
            heard: false,
            disabled: false,
            players_only: bv(&p, "bTrackPlayersOnly", true),
            pawns_only: bv(&p, "bTrackPawnsOnly", false),
            max_range: fv(&p, "maxRange", 512.0),
            fire_rate: fv(&p, "fireRate", 0.25) as f32,
            accuracy: fv(&p, "gunAccuracy", 0.5),
            damage: fv(&p, "gunDamage", 5.0) as f32,
            pitch_limit: fv(&p, "pitchLimit", 11000.0),
            fire_t: 0.0,
            flash_t: 0.0,
            target: None,
            last_target: None,
            confused: false,
            confusion_t: 0.0,
            confusion_dur: fv(&p, "confusionDuration", 10.0) as f32,
            hum: true,
            mesh: Some(am),
            parts,
            handles,
            clip: None,
            firing: false,
            lens: None,
            muzzle: None,
            gone: false,
            seed,
        });
    }
    println!(
        "security: {} cameras, {} turrets, {} lasers",
        st.cams.len(),
        st.turrets.len(),
        st.lasers.len()
    );
}

/// What a security computer shows and controls (ComputerSecurityCameraWindow / ComputerScreenSecurity).
pub mod control {
    use super::*;

    /// A camera's state by tag: (exists, active).
    pub fn camera(st: &Security, tag: &str) -> Option<bool> {
        st.cams.iter().find(|c| !c.gone && c.tag.eq_ignore_ascii_case(tag)).map(|c| c.active)
    }

    /// Switch a camera by tag (DeusExPlayer.ToggleCameraState: Trigger / UnTrigger with the player as the
    /// instigator, so the "camera activated / deactivated" message shows).
    pub fn set_camera(st: &mut Security, tag: &str, on: bool) {
        for c in st.cams.iter_mut().filter(|c| !c.gone && c.tag.eq_ignore_ascii_case(tag)) {
            if on {
                cam_trigger(c, true);
            } else {
                cam_untrigger(c, true);
            }
        }
    }

    /// A camera's DesiredRotation (pitch, yaw): what the security screen pans (ReplicatedRotation).
    pub fn camera_desired(st: &Security, tag: &str) -> Option<(f64, f64)> {
        st.cams.iter().find(|c| !c.gone && c.tag.eq_ignore_ascii_case(tag)).map(|c| c.want)
    }

    /// DeusExPlayer.UpdateCameraRotation: set its DesiredRotation (a swinging camera keeps swinging over it).
    pub fn set_camera_desired(st: &mut Security, tag: &str, rot: (f64, f64)) {
        for c in st.cams.iter_mut().filter(|c| !c.gone && c.tag.eq_ignore_ascii_case(tag)) {
            c.want = rot;
        }
    }

    /// A camera's place and view (location, pitch, yaw), for the camera feed.
    pub fn camera_view(st: &Security, tag: &str) -> Option<(DVec3, f64, f64)> {
        st.cams.iter().find(|c| !c.gone && c.tag.eq_ignore_ascii_case(tag)).map(|c| (c.pos, c.rot.0, c.rot.1))
    }

    /// A turret's mode by tag: 0 disabled, 1 attack allies (players only), 2 attack enemies, 3 everything.
    pub fn turret(st: &Security, tag: &str) -> Option<u8> {
        st.turrets.iter().find(|t| !t.gone && t.tag.eq_ignore_ascii_case(tag)).map(|t| {
            if t.disabled {
                0
            } else if t.players_only {
                1
            } else if t.pawns_only {
                2
            } else {
                3
            }
        })
    }

    /// ComputerScreenSecurity.SetTurretState / SetTurretTrackMode.
    pub fn set_turret(st: &mut Security, tag: &str, mode: u8) {
        for t in st.turrets.iter_mut().filter(|t| !t.gone && t.tag.eq_ignore_ascii_case(tag)) {
            match mode {
                0 => {
                    // SetTurretState(False, True): off and disabled.
                    t.disabled = true;
                    t.active = false;
                    t.hum = false;
                }
                _ => {
                    t.disabled = false;
                    t.active = true;
                    t.hum = true;
                    t.players_only = mode == 1;
                    t.pawns_only = mode == 2;
                }
            }
        }
    }

    pub fn noop(_: &mut Security) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_takes_the_short_way_round() {
        assert_eq!(wrap(65000.0 - 100.0), 64900.0 - 65536.0);
        assert_eq!(wrap(100.0 - 65000.0), 636.0);
        let mut c = 65000.0;
        turn(&mut c, 500.0, 16384.0, 0.01);
        assert!((c - (65000.0 + 163.84)).abs() < 1e-6);
    }

    #[test]
    fn rotator_matches_unreal_axes() {
        let (p, y) = rotator(DVec3::new(0.0, 1.0, 0.0));
        assert!((p).abs() < 1e-9 && (y - 16384.0).abs() < 1e-6);
        let v = dir(0.0, 16384.0);
        assert!((v.y - 1.0).abs() < 1e-9);
    }
}

/// A new map is going in (loader.rs): this module's state of the map being left.
pub fn reset_level() {
    if let Ok(mut v) = HACKED.lock() {
        v.clear();
    }
    if let Ok(mut v) = EVENTS.lock() {
        v.clear();
    }
    if let Ok(mut v) = BLASTS.lock() {
        v.clear();
    }
    if let Ok(mut v) = MISSION.lock() {
        *v = Default::default();
    }
}
