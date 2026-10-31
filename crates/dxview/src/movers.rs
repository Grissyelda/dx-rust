//! Doors and other movers at runtime: keyframe interpolation (Mover.InterpolateTo / PHYS_MovingBrush),
//! the trigger states used on Liberty Island (TriggerToggle, TriggerOpenTimed), and frobbing
//! (DeusExMover.Frob: locked doors refuse, open doors can always be closed, like-tagged doors move together).

use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};
use dx_level::actors::{light_part, rotator_axes, rotator_to_bevy_mat3};
use dx_level::movers::MoverInfo;
use dx_level::{ue_to_bevy, Level};
use glam::DVec3;
use imsim_core::movement::Hit;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct MoverRt {
    pub location: DVec3,
    pub rotation: [i32; 3],
    pub key_num: u8,
    pub prev_key_num: u8,
    pub interpolating: bool,
    pub alpha: f64,
    pub rate: f64,
    pub old_pos: DVec3,
    pub old_rot: [i32; 3],
    /// Remaining stay-open time for TriggerOpenTimed; closes when it runs out.
    pub close_timer: Option<f64>,
    pub opening: bool,
    pub locked: bool,
    pub moved_since_lit: bool,
    /// lockStrength 0..1 (lockpicks take it down 0.01 at a time) and bPickable.
    pub lock_strength: f64,
    pub pickable: bool,
    /// DeusExMover.doorStrength (0..1) and bDestroyed.
    pub door_strength: f64,
    pub destroyed: bool,
    /// How far a base this mover rides on (AttachTag) has carried it.
    pub carried: DVec3,
    /// Mission-script overrides of bBreakable / bFrobbable (None = the map's value).
    pub breakable: Option<bool>,
    pub frobbable: Option<bool>,
    /// Mover.WaitingPawn: the pawn (blocker) that frobbed it open (ScriptedPawn.FrobDoor); others leave it alone.
    pub waiting_pawn: Option<usize>,
}

#[derive(Resource)]
pub struct Movers {
    pub rt: Vec<MoverRt>,
    pub entities: Vec<Entity>,
    pub parts: Vec<Vec<(Handle<Mesh>, usize)>>,
}

#[derive(Resource, Default)]
pub struct HudMessage {
    pub text: String,
    pub time_left: f32,
}

fn to_dvec(v: [f32; 3]) -> DVec3 {
    DVec3::new(v[0] as f64, v[1] as f64, v[2] as f64)
}

pub fn bevy_rotation(rot: [i32; 3]) -> Quat {
    let m = rotator_to_bevy_mat3(rot);
    Quat::from_mat3(&Mat3::from_cols(
        Vec3::from(m[0]),
        Vec3::from(m[1]),
        Vec3::from(m[2]),
    ))
}

fn part_mesh(info: &MoverInfo, pi: usize, loc: [f32; 3], rot: [i32; 3], level: &Level) -> Mesh {
    let part = &info.parts[pi];
    let colors = light_part(part, loc, rot, 1.0, &level.lights);
    let pos: Vec<[f32; 3]> = part.positions.iter().map(|p| ue_to_bevy(*p)).collect();
    let nrm: Vec<[f32; 3]> = part.normals.iter().map(|n| [n[0], n[2], n[1]]).collect();
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, part.uvs.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(part.indices.clone()));
    mesh
}

pub fn spawn_movers(
    commands: &mut Commands,
    level: &Level,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    handles: &[Handle<Image>],
) -> Movers {
    let mut cache = HashMap::new();
    let mut out = Movers {
        rt: Vec::new(),
        entities: Vec::new(),
        parts: Vec::new(),
    };
    for info in &level.movers {
        let rt = MoverRt {
            location: to_dvec(info.location),
            rotation: info.rotation,
            key_num: info.key_num,
            prev_key_num: info.key_num,
            interpolating: false,
            alpha: 0.0,
            rate: 0.0,
            old_pos: to_dvec(info.location),
            old_rot: info.rotation,
            close_timer: None,
            waiting_pawn: None,
            opening: false,
            locked: info.locked,
            moved_since_lit: false,
            lock_strength: match unreal_pkg::props::find(
                &level.actors[info.actor].props,
                "lockStrength",
            ) {
                Some(unreal_pkg::props::Value::Float(v)) => *v as f64,
                _ => 0.2,
            },
            pickable: !matches!(
                unreal_pkg::props::find(&level.actors[info.actor].props, "bPickable"),
                Some(unreal_pkg::props::Value::Bool(false))
            ),
            door_strength: crate::sound::fprop(
                &level.actors[info.actor].props,
                "doorStrength",
                0.25,
            ) as f64,
            destroyed: false,
            carried: DVec3::ZERO,
            breakable: None,
            frobbable: None,
        };
        let parent = commands
            .spawn((
                Transform::from_translation(Vec3::from(ue_to_bevy(info.location)))
                    .with_rotation(bevy_rotation(info.rotation)),
                Visibility::default(),
            ))
            .id();
        let mut parts = Vec::new();
        for (pi, part) in info.parts.iter().enumerate() {
            if part.indices.is_empty() {
                continue;
            }
            let h = meshes.add(part_mesh(info, pi, info.location, info.rotation, level));
            let mat = crate::actors::material_for(
                materials,
                &mut cache,
                handles,
                part.texture,
                part.class,
            );
            let child = commands
                .spawn((Mesh3d(h.clone()), MeshMaterial3d(mat)))
                .id();
            commands.entity(parent).add_child(child);
            parts.push((h, pi));
        }
        out.rt.push(rt);
        out.entities.push(parent);
        out.parts.push(parts);
    }
    out
}

/// Sweep a box against one mover in its current placement.
pub fn trace_mover(
    info: &MoverInfo,
    rt: &MoverRt,
    start: DVec3,
    end: DVec3,
    extent: DVec3,
) -> Option<Hit> {
    let ax = rotator_axes(rt.rotation);
    let pre = to_dvec(info.pre_pivot);
    let to_local = |p: DVec3| {
        let d = p - rt.location;
        // Inverse rotation: project on the rotated axes.
        DVec3::new(
            d.x * ax[0][0] as f64 + d.y * ax[0][1] as f64 + d.z * ax[0][2] as f64,
            d.x * ax[1][0] as f64 + d.y * ax[1][1] as f64 + d.z * ax[1][2] as f64,
            d.x * ax[2][0] as f64 + d.y * ax[2][1] as f64 + d.z * ax[2][2] as f64,
        ) + pre
    };
    let h = info.collision.trace(to_local(start), to_local(end), extent);
    if h.fraction >= 1.0 {
        return None;
    }
    let n = h.normal;
    let wn = DVec3::new(
        ax[0][0] as f64 * n.x + ax[1][0] as f64 * n.y + ax[2][0] as f64 * n.z,
        ax[0][1] as f64 * n.x + ax[1][1] as f64 * n.y + ax[2][1] as f64 * n.z,
        ax[0][2] as f64 * n.x + ax[1][2] as f64 * n.y + ax[2][2] as f64 * n.z,
    );
    Some(Hit {
        fraction: h.fraction,
        normal: wn,
        actor: None,
    })
}

/// Mover.InterpolateTo
fn interpolate_to(info: &MoverInfo, rt: &mut MoverRt, new_key: u8, seconds: f64) {
    let new_key = new_key.min(7);
    if new_key == rt.prev_key_num && rt.key_num != rt.prev_key_num {
        // Reverse the movement smoothly.
        rt.alpha = 1.0 - rt.alpha;
        rt.old_pos = to_dvec(info.base_pos) + to_dvec(info.key_pos[rt.key_num as usize]);
        let kr = info.key_rot[rt.key_num as usize];
        rt.old_rot = [
            info.base_rot[0] + kr[0],
            info.base_rot[1] + kr[1],
            info.base_rot[2] + kr[2],
        ];
    } else {
        rt.old_pos = rt.location;
        rt.old_rot = rt.rotation;
        rt.alpha = 0.0;
    }
    rt.interpolating = true;
    rt.prev_key_num = rt.key_num;
    rt.key_num = new_key;
    rt.rate = 1.0 / seconds.max(0.005);
}

fn do_open(info: &MoverInfo, rt: &mut MoverRt) {
    rt.opening = true;
    interpolate_to(info, rt, 1, info.move_time as f64);
}

fn do_close(info: &MoverInfo, rt: &mut MoverRt) {
    rt.opening = false;
    let k = rt.key_num.saturating_sub(1);
    interpolate_to(info, rt, k, info.move_time as f64);
}

/// ElevatorMover.SetSeq (SequenceTrigger): go to keyframe `seq` unless already moving or there.
pub fn set_seq(info: &MoverInfo, rt: &mut MoverRt, seq: u8) {
    if rt.destroyed || rt.interpolating || rt.key_num == seq {
        return;
    }
    rt.opening = seq > rt.key_num;
    interpolate_to(info, rt, seq, info.move_time as f64);
}

/// Mover.Trigger for the supported states.
pub fn trigger(info: &MoverInfo, rt: &mut MoverRt) {
    if rt.destroyed {
        return;
    }
    match info.initial_state.as_str() {
        // ElevatorMover (and a plain Mover's BumpOpenTimed / StandOpenTimed) have no Trigger: a SequenceTrigger
        // moves an elevator (set_seq).
        "ElevatorMover" | "BumpOpenTimed" | "StandOpenTimed" | "BumpButton" => {}
        "TriggerOpenTimed" => {
            if !rt.interpolating && rt.key_num == 0 {
                do_open(info, rt);
                rt.close_timer = None;
            }
        }
        // TriggerToggle and anything not yet ported behave as a toggle.
        _ => {
            if rt.key_num == 0 || rt.key_num < rt.prev_key_num {
                do_open(info, rt);
            } else {
                do_close(info, rt);
            }
        }
    }
}

/// DeusExMover.Frob without inventory (no key ring or lockpicks yet). Returns the message to show.
pub fn frob(
    level: &Level,
    movers: &mut Movers,
    idx: usize,
    frobber: DVec3,
    ignore_locks: bool,
) -> Option<String> {
    let info = &level.movers[idx];
    if !movers.rt[idx].frobbable.unwrap_or(info.frobbable) {
        return None;
    }
    let rt = &movers.rt[idx];
    let mut open_it = false;
    let mut msg: Option<String> = Some("It's locked".to_string());
    let mut done = false;
    if rt.key_num != 0 || rt.prev_key_num != 0 {
        open_it = true;
        msg = None;
        done = true;
    }
    if !done && info.one_way {
        let ax = rotator_axes(rt.rotation);
        let x = DVec3::new(ax[0][0] as f64, ax[0][1] as f64, ax[0][2] as f64);
        if (rt.location - frobber).dot(x) > 0.0 {
            return Some("It won't open from this side".to_string());
        }
    }
    if !done && (!rt.locked || ignore_locks) {
        open_it = true;
        msg = None;
    }
    if open_it {
        // Smarter AI (ours): guards that see a door move go and look.
        crate::smartai::disturb(movers.rt[idx].location + DVec3::Z * 32.0, crate::smartai::Disturb::Door);
        let tag = info.tag.clone();
        trigger(info, &mut movers.rt[idx]);
        // Double doors: every mover with the same (non-default) tag.
        if !tag.is_empty() && tag != "DeusExMover" {
            for (j, other) in level.movers.iter().enumerate() {
                if j != idx && other.tag == tag {
                    trigger(other, &mut movers.rt[j]);
                }
            }
        }
    }
    msg
}

/// Doors pawns open (ScriptedPawn.FrobDoor): (mover, pawn blocker) frobbed, or a tag triggered by the door's button.
pub enum PawnDoor {
    Frob(usize, usize),
    Trigger(String, usize),
}
static PAWN_DOORS: std::sync::Mutex<Vec<PawnDoor>> = std::sync::Mutex::new(Vec::new());

pub fn pawn_door(req: PawnDoor) {
    if let Ok(mut q) = PAWN_DOORS.lock() {
        q.push(req);
    }
}

/// DeusExMover.Frob by a pawn that isn't the player: "Let any non-player pawn open any door for now" - locked or
/// not, from either side; this and every like-tagged mover are triggered.
fn frob_by_pawn(level: &Level, movers: &mut Movers, idx: usize) {
    let info = &level.movers[idx];
    if movers.rt[idx].destroyed || !movers.rt[idx].frobbable.unwrap_or(info.frobbable) {
        return;
    }
    let tag = info.tag.clone();
    trigger(info, &mut movers.rt[idx]);
    if !tag.is_empty() && tag != "DeusExMover" {
        for (j, other) in level.movers.iter().enumerate() {
            if j != idx && other.tag == tag {
                trigger(other, &mut movers.rt[j]);
            }
        }
    }
}

/// PHYS_MovingBrush plus the timed-close part of TriggerOpenTimed.
pub fn tick_movers(
    time: Res<Time>,
    walk_test: Option<Res<crate::WalkTest>>,
    level: Res<crate::LevelRes>,
    mut movers: ResMut<Movers>,
    mut tfs: Query<&mut Transform>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut map_gen: Local<u64>,
    mut was: Local<Vec<(bool, bool)>>,
    mut attach: Local<Option<Attachments>>,
) {
    // A new map (loader.rs): start this system's per-map state over.
    if crate::loader::new_map(&mut map_gen) {
        was.clear();
        *attach = None;
    }
    let _prof = crate::prof::scope("movers::tick_movers");
    let dt = if walk_test.is_some() {
        1.0 / 60.0
    } else {
        time.delta_secs_f64().min(0.1)
    };
    let level = &level.0;
    for req in PAWN_DOORS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        match req {
            PawnDoor::Frob(i, pawn) => {
                if i < movers.rt.len() {
                    frob_by_pawn(level, &mut movers, i);
                    movers.rt[i].waiting_pawn = Some(pawn);
                }
            }
            PawnDoor::Trigger(tag, pawn) => {
                for (j, info) in level.movers.iter().enumerate() {
                    if info.tag.eq_ignore_ascii_case(&tag) {
                        trigger(info, &mut movers.rt[j]);
                        movers.rt[j].waiting_pawn = Some(pawn);
                    }
                }
            }
        }
    }
    let at = attach.get_or_insert_with(|| Attachments::build(level));
    // MoverColliders ride their movers (MoverCollider.BeginPlay: SetBase(mover)); class-proximity triggers look for
    // them (triggers.rs) - the training ranges' targets open the blast doors as they come forward.
    if let Ok(mut c) = COLLIDERS.lock() {
        c.clear();
        for (k, (pos, m)) in at.colliders.iter().enumerate() {
            let d = m.map_or(DVec3::ZERO, |b| movers.rt[b].location - to_dvec(level.movers[b].base_pos));
            let now = *pos + d;
            // Touch is swept in the engine: a long step this frame still passes through a thin trigger, so the path
            // since last frame is sampled every 16 units.
            let prev = at.collider_prev.get(k).copied().flatten().unwrap_or(now);
            let n = ((now - prev).length() / 16.0).ceil().min(64.0) as usize;
            for s in 1..n {
                c.push(prev.lerp(now, s as f64 / n as f64));
            }
            c.push(now);
        }
        at.collider_prev = c_last(&at.colliders, &movers.rt, level);
    }
    for i in 0..movers.rt.len() {
        let info = &level.movers[i];
        let mut changed = false;
        {
            let rt = &mut movers.rt[i];
            if let Some(t) = rt.close_timer.as_mut() {
                *t -= dt;
                if *t <= 0.0 {
                    rt.close_timer = None;
                    do_close(info, rt);
                }
            }
            let mut left = dt;
            while rt.interpolating && left > 0.0 {
                let mut a = rt.alpha + rt.rate * left;
                if a > 1.0 {
                    left = (a - 1.0) / rt.rate;
                    a = 1.0;
                } else {
                    left = 0.0;
                }
                rt.alpha = a;
                let t = if info.glide == 1 {
                    a * a * (3.0 - 2.0 * a)
                } else {
                    a
                };
                let k = rt.key_num as usize;
                let target = to_dvec(info.base_pos) + to_dvec(info.key_pos[k]);
                rt.location = rt.old_pos + (target - rt.old_pos) * t;
                for c in 0..3 {
                    let tr = info.base_rot[c] + info.key_rot[k][c];
                    rt.rotation[c] = rt.old_rot[c] + ((tr - rt.old_rot[c]) as f64 * t) as i32;
                }
                changed = true;
                if a >= 1.0 {
                    // InterpolateEnd: chain through extra keyframes, else finish.
                    rt.interpolating = false;
                    let old = rt.prev_key_num;
                    rt.prev_key_num = rt.key_num;
                    rt.alpha = 0.0;
                    if rt.key_num > 0 && rt.key_num < old {
                        let k = rt.key_num - 1;
                        interpolate_to(info, rt, k, info.move_time as f64);
                    } else if rt.key_num + 1 < info.num_keys && rt.key_num > old {
                        let k = rt.key_num + 1;
                        interpolate_to(info, rt, k, info.move_time as f64);
                    } else if rt.opening && info.initial_state == "TriggerOpenTimed" {
                        rt.close_timer = Some(info.stay_open_time as f64);
                    }
                    // FinishNotify: the waiting pawn goes on (StopWaiting) and the door is free again.
                    if !rt.interpolating {
                        rt.waiting_pawn = None;
                    }
                }
            }
            if changed {
                rt.moved_since_lit = true;
            }
        }
        mover_sounds(level, i, &movers.rt[i], &mut was);
        if changed {
            let rt = &movers.rt[i];
            if let Ok(mut tf) = tfs.get_mut(movers.entities[i]) {
                let l = [
                    rt.location.x as f32,
                    rt.location.y as f32,
                    rt.location.z as f32,
                ];
                tf.translation = Vec3::from(ue_to_bevy(l));
                tf.rotation = bevy_rotation(rt.rotation);
            }
        }
        if let Ok(f) = std::env::var("DXVIEW_MOVERLOG") {
            if info.tag.eq_ignore_ascii_case(&f) || info.name.eq_ignore_ascii_case(&f) || level.actors[info.actor].class.eq_ignore_ascii_case(&f) {
                let l = movers.rt[i].location;
                println!("MOVER {} {:.0},{:.0},{:.0} key {} interp {}", info.name, l.x, l.y, l.z, movers.rt[i].key_num, movers.rt[i].interpolating);
            }
        }
        // Riding a moving base (AttachTag): carried along by how far the base has moved from where it started.
        if let Some(Some(b)) = attach.as_ref().map(|a| a.base[i]) {
            if !movers.rt[i].destroyed {
                let d = movers.rt[b].location - to_dvec(level.movers[b].base_pos);
                let ent = movers.entities[i];
                let rt = &mut movers.rt[i];
                // Its own place: just interpolated this frame (without the ride), or last frame's minus the ride.
                let own = if changed { rt.location } else { rt.location - rt.carried };
                if (own + d - rt.location).length_squared() > 1e-8 {
                    rt.location = own + d;
                    rt.carried = d;
                    rt.moved_since_lit = true;
                    if let Ok(mut tf) = tfs.get_mut(ent) {
                        tf.translation = Vec3::from(ue_to_bevy(rt.location.as_vec3().to_array()));
                    }
                }
            }
        }
        // Re-light once it stops.
        if !movers.rt[i].interpolating && movers.rt[i].moved_since_lit {
            movers.rt[i].moved_since_lit = false;
            let rt = movers.rt[i].clone();
            let l = [
                rt.location.x as f32,
                rt.location.y as f32,
                rt.location.z as f32,
            ];
            for (h, pi) in movers.parts[i].clone() {
                if let Some(mut m) = meshes.get_mut(&h) {
                    m.insert_attribute(
                        Mesh::ATTRIBUTE_COLOR,
                        light_part(&info.parts[pi], l, rt.rotation, 1.0, &level.lights),
                    );
                }
            }
        }
    }
}

/// Movers based on other movers (Actor.AttachTag: this actor rides the actor tagged with it - the training range's
/// glass targets on their sliding frames), and MoverCollider keypoints (moverTag).
pub struct Attachments {
    base: Vec<Option<usize>>,
    colliders: Vec<(DVec3, Option<usize>)>,
    collider_prev: Vec<Option<DVec3>>,
}

fn c_last(colliders: &[(DVec3, Option<usize>)], rts: &[MoverRt], level: &dx_level::Level) -> Vec<Option<DVec3>> {
    colliders
        .iter()
        .map(|(pos, m)| Some(*pos + m.map_or(DVec3::ZERO, |b| rts[b].location - to_dvec(level.movers[b].base_pos))))
        .collect()
}

impl Attachments {
    fn build(level: &dx_level::Level) -> Attachments {
        let name = |p: &[unreal_pkg::props::Property], n: &str| match unreal_pkg::props::find(p, n) {
            Some(unreal_pkg::props::Value::Name(v)) if v != "None" => v.clone(),
            _ => String::new(),
        };
        let by_tag = |tag: &str, me: usize| {
            (!tag.is_empty())
                .then(|| level.movers.iter().enumerate().find(|(j, m)| *j != me && m.tag.eq_ignore_ascii_case(tag)).map(|(j, _)| j))
                .flatten()
        };
        let base = level
            .movers
            .iter()
            .enumerate()
            .map(|(i, m)| by_tag(&name(&level.actors[m.actor].props, "AttachTag"), i))
            .collect();
        let colliders = level
            .actors
            .iter()
            .filter(|a| a.class == "MoverCollider")
            .map(|a| {
                let l = a.location();
                (DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64), by_tag(&name(&a.props, "moverTag"), usize::MAX))
            })
            .collect();
        Attachments { base, colliders, collider_prev: Vec::new() }
    }
}

/// Where the MoverColliders are this frame.
pub static COLLIDERS: std::sync::Mutex<Vec<DVec3>> = std::sync::Mutex::new(Vec::new());

/// A lock being picked (DeusExMover.Frob with a Lockpick in hand, then Timer): one lockpick is good for
/// SkillLockpicking value x 100 steps of 0.01 lock strength, a step every pickTime x 10 / steps tenths of a second.
#[derive(Resource, Default)]
pub struct Picking(pub Option<PickJob>);

pub struct PickJob {
    pub mover: usize,
    pub picks_left: i32,
    pub ticks: f64,
    pub per_pick: f64,
}

/// SkillLockpicking value (0.1 untrained .. 0.75 master); GMDX Locksport makes it 1 (one pick opens any lock).
pub fn lockpick_skill() -> f64 {
    if crate::perks::has("PerkLocksport") {
        return 1.0;
    }
    crate::abilities::skill("SkillLockpicking") as f64
}

pub fn start_picking(level: &Level, movers: &Movers, idx: usize) -> Option<PickJob> {
    let rt = &movers.rt[idx];
    if !rt.locked || !rt.pickable {
        return None;
    }
    let pick_time =
        match unreal_pkg::props::find(&level.actors[level.movers[idx].actor].props, "pickTime") {
            Some(unreal_pkg::props::Value::Float(v)) => *v as f64,
            _ => 4.0,
        };
    let num = (lockpick_skill() * 100.0).round() as i32;
    Some(PickJob {
        mover: idx,
        picks_left: num,
        ticks: 0.0,
        per_pick: pick_time * 10.0 / num.max(1) as f64,
    })
}

/// Advance a pick in progress; stops when the lock opens, the pick is used up, or we look away / put it down.
pub fn lockpick_system(
    time: Res<Time>,
    level: Res<crate::LevelRes>,
    mut movers: ResMut<Movers>,
    mut picking: ResMut<Picking>,
    hl: Res<crate::frobui::FrobHighlight>,
    mut weapons: ResMut<crate::weapons::Weapons>,
    mut inv: ResMut<crate::pickups::PlayerInventory>,
    mut hud: ResMut<HudMessage>,
) {
    let Some(job) = picking.0.as_mut() else {
        return;
    };
    let lvl = &level.0;
    job.ticks += time.delta_secs_f64() * 10.0;
    let mut strength = movers.rt[job.mover].lock_strength;
    while job.ticks > job.per_pick && job.picks_left > 0 {
        job.picks_left -= 1;
        strength = (strength - 0.01).clamp(0.0, 1.0);
        job.ticks -= job.per_pick;
    }
    let tag = lvl.movers[job.mover].tag.clone();
    let same = |j: usize| {
        j == job.mover || (!tag.is_empty() && tag != "DeusExMover" && lvl.movers[j].tag == tag)
    };
    for j in 0..movers.rt.len() {
        if same(j) {
            movers.rt[j].lock_strength = strength;
        }
    }
    let holding = weapons
        .held
        .as_ref()
        .map(|h| h.stats.class == "Lockpick")
        .unwrap_or(false);
    let looking = hl.target.as_ref().and_then(|t| t.mover()) == Some(job.mover);
    let mut stop = false;
    if strength <= 1e-6 {
        for j in 0..movers.rt.len() {
            if same(j) {
                movers.rt[j].locked = false;
                movers.rt[j].lock_strength = 0.0;
            }
        }
        hud.text = "You picked the lock".into();
        hud.time_left = 2.0;
        stop = true;
    } else if job.picks_left <= 0 || !holding || !looking {
        stop = true;
    }
    if stop {
        // StopPicking: the lockpick is spent whatever happened.
        picking.0 = None;
        if inv.0.use_one("Lockpick") == 0 && holding {
            weapons.held = None;
        }
    }
}

/// Events movers raise when they finish opening / start closing, for the event dispatch.
pub static EVENTS_OUT: std::sync::Mutex<Vec<(String, bool)>> = std::sync::Mutex::new(Vec::new());

/// Mover sounds (DEUS_EX CNN: they use SoundRadius and SoundVolume): OpeningSound/ClosingSound when a move starts,
/// OpenedSound/ClosedSound when it ends, MoveAmbientSound looping while it moves.
fn mover_sounds(level: &Level, i: usize, rt: &MoverRt, was: &mut Vec<(bool, bool)>) {
    if was.len() <= i {
        was.resize(i + 1, (false, false));
    }
    let (moving, opening) = was[i];
    was[i] = (rt.interpolating, rt.opening);
    let props = &level.actors[level.movers[i].actor].props;
    let byte = |n: &str, d: f32| crate::sound::fprop(props, n, d);
    let radius = 25.0 * (byte("SoundRadius", 32.0) + 1.0);
    let vol = byte("TransientSoundVolume", 1.0) * byte("SoundVolume", 128.0) / 255.0;
    let pos = rt.location;
    let play = |name: &str| {
        if let Some(snd) = crate::sound::prop(props, name) {
            crate::sound::play_owned(
                crate::sound::mover_owner(i),
                crate::sound::slot::NONE,
                &snd,
                Some(pos),
                vol,
                radius,
                1.0,
            );
        }
    };
    let event = &level.movers[i].event;
    let group = crate::triggers::group_of(&level.actors[level.movers[i].actor].props);
    let fire = |on: bool| {
        if !event.is_empty() && event != "None" {
            crate::triggers::note_source(event, &group);
            if let Ok(mut q) = EVENTS_OUT.lock() {
                q.push((event.clone(), on));
            }
        }
    };
    // Mover.FinishedOpening triggers Event; DoClose untriggers it.
    if rt.interpolating && !rt.opening && (!moving || opening) {
        fire(false);
    } else if !rt.interpolating && moving && opening {
        fire(true);
    }
    if rt.interpolating && (!moving || opening != rt.opening) {
        play(if rt.opening {
            "OpeningSound"
        } else {
            "ClosingSound"
        });
    } else if !rt.interpolating && moving {
        play(if opening {
            "OpenedSound"
        } else {
            "ClosedSound"
        });
    }
    if rt.interpolating {
        if let Some(snd) = crate::sound::prop(props, "MoveAmbientSound") {
            crate::sound::hold(
                crate::sound::mover_owner(i),
                &snd,
                Some(pos),
                1.4 * byte("SoundVolume", 128.0) / 255.0,
                radius,
                byte("SoundPitch", 64.0) / 64.0,
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Breakable doors (DeusExMover.TakeDamage / BlowItUp).
// ---------------------------------------------------------------------------------------------------------------

/// Something that does damage along a line hit world geometry here: (start, end, damage, damage type). If the
/// geometry was a mover, it takes the damage.
static LINE_HITS: std::sync::Mutex<Vec<(DVec3, DVec3, f32, &'static str)>> =
    std::sync::Mutex::new(Vec::new());
/// Blast rings (HurtRadius): (centre, radius, damage, damage type).
static BLASTS: std::sync::Mutex<Vec<(DVec3, f64, f32, &'static str, f32)>> =
    std::sync::Mutex::new(Vec::new());
/// Damage straight to one mover (stomped by a falling decoration): (mover, damage, type).
static DIRECT: std::sync::Mutex<Vec<(usize, f32, &'static str)>> =
    std::sync::Mutex::new(Vec::new());

/// A trace from `start` hit a wall at `end` (the hit point) doing `damage`.
pub fn hit_line(start: DVec3, end: DVec3, damage: f32, damage_type: &'static str) {
    if damage > 0.0 {
        if let Ok(mut q) = LINE_HITS.lock() {
            q.push((start, end, damage, damage_type));
        }
    }
}

pub fn blast(pos: DVec3, radius: f64, damage: f32, damage_type: &'static str) {
    blast_ring(pos, radius, damage, damage_type, 0.0);
}

/// One ring of a staggered blast whose whole Damage is `full` (GMDX's HurtRadiusGMDX mover fix uses it).
pub fn blast_ring(pos: DVec3, radius: f64, damage: f32, damage_type: &'static str, full: f32) {
    if let Ok(mut q) = BLASTS.lock() {
        q.push((pos, radius, damage, damage_type, full));
    }
}

pub fn damage_mover(i: usize, damage: f32, damage_type: &'static str) {
    if let Ok(mut q) = DIRECT.lock() {
        q.push((i, damage, damage_type));
    }
}

/// Which mover (if any) a segment hits first, and where along it.
pub fn mover_on_line(
    level: &Level,
    rts: &[MoverRt],
    start: DVec3,
    end: DVec3,
) -> Option<(usize, f64)> {
    let mut best: Option<(usize, f64)> = None;
    for (i, (info, rt)) in level.movers.iter().zip(rts).enumerate() {
        if rt.destroyed {
            continue;
        }
        if let Some(h) = trace_mover(info, rt, start, end, DVec3::ZERO) {
            if best.map_or(true, |b| h.fraction < b.1) {
                best = Some((i, h.fraction));
            }
        }
    }
    best
}

/// Apply queued damage to breakable movers and blow up the ones that give way.
#[allow(clippy::too_many_arguments)]
pub fn mover_damage_system(
    level: Res<crate::LevelRes>,
    mut movers: ResMut<Movers>,
    mut tfs: Query<&mut Transform>,
    mut events: ResMut<crate::devices::GameEvents>,
    mut booms: ResMut<crate::explosion::Explosions>,
    mut noises: ResMut<crate::stealth::Noises>,
) {
    let _prof = crate::prof::scope("movers::mover_damage_system");
    let lvl = &level.0;
    // (mover, damage, type, past the threshold anyway: GMDX's ring fix)
    let mut hits: Vec<(usize, f32, &'static str, bool)> = DIRECT
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
        .into_iter()
        .map(|(i, d, t)| (i, d, t, false))
        .collect();
    for (start, end, dmg, ty) in LINE_HITS
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
    {
        // Extend a little past the hit point so the trace reaches the surface it stopped at.
        let dir = (end - start).normalize_or_zero();
        if let Some((i, _)) = mover_on_line(lvl, &movers.rt, start, end + dir * 4.0) {
            hits.push((i, dmg, ty, false));
        }
    }
    let ring_fix = crate::settings::get("FixBlastDoors") > 0.5;
    for (pos, radius, dmg, ty, full) in BLASTS
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
    {
        // HurtRadius: victims by Location (a mover's is its pivot), CollisionRadius 22 (brush default), and a
        // clear line (FastTrace: level geometry only) from the blast.
        for (i, rt) in movers.rt.iter().enumerate() {
            if rt.destroyed {
                continue;
            }
            let d = rt.location.distance(pos).max(1.0);
            if d >= radius + 22.0 {
                continue;
            }
            if lvl.collision.trace(pos, rt.location, DVec3::ZERO).fraction < 0.999 {
                continue;
            }
            let scale = (1.0 - ((d - 22.0) / radius).max(0.0)) as f32;
            if scale > 0.0 {
                // GMDX fix (setting FixBlastDoors): the blast is dealt in five rings of 2 x Damage / 5, so a door
                // whose threshold the whole blast beats was never hurt by any one ring. A ring counts whenever the
                // full Damage at this distance would beat the threshold.
                let bypass = ring_fix
                    && full > 0.0
                    && full * scale >= crate::sound::fprop(&lvl.actors[lvl.movers[i].actor].props, "minDamageThreshold", 10.0);
                hits.push((i, dmg * scale, ty, bypass));
            }
        }
    }
    for (i, dmg, ty, bypass) in hits {
        let info = &lvl.movers[i];
        let p = &lvl.actors[info.actor].props;
        let ent = movers.entities[i];
        let rt = &mut movers.rt[i];
        if rt.destroyed
            || matches!(
                ty,
                "TearGas"
                    | "PoisonGas"
                    | "HalonGas"
                    | "Stunned"
                    | "Radiation"
                    | "EMP"
                    | "NanoVirus"
                    | "Shocked"
            )
        {
            continue;
        }
        let mut threshold = crate::sound::fprop(p, "minDamageThreshold", 10.0);
        if !rt.breakable.unwrap_or(matches!(
            unreal_pkg::props::find(p, "bBreakable"),
            Some(unreal_pkg::props::Value::Bool(true))
        )) {
            // GMDX Breakdown (OnMapLoadAndPurchase): pickable doors that aren't breakable and fire no Event give
            // way to a blast of PerkValue (160) or more.
            let no_event = !matches!(unreal_pkg::props::find(p, "Event"), Some(unreal_pkg::props::Value::Name(n)) if !n.is_empty() && !n.eq_ignore_ascii_case("None"));
            match crate::perks::value_if("PerkBreakdown") {
                Some(v) if rt.breakable.is_none() && rt.pickable && no_event => threshold = v,
                _ => continue,
            }
        }
        // GMDX Doorsman: every door's minDamageThreshold is PerkValue (5) lower, never below 1.
        if let Some(v) = crate::perks::value_if("PerkDoorsman") {
            threshold = (threshold - v).max(1.0);
        }
        // Damage is an int in the script: fractions drop.
        let dmg = dmg.floor();
        if dmg >= threshold || bypass {
            rt.door_strength = (rt.door_strength - dmg as f64 * 0.01).clamp(0.0, 1.0);
        }
        println!(
            "MOVER {} hit for {dmg} ({ty}) -> strength {:.2}",
            info.name, rt.door_strength
        );
        if rt.door_strength > 1e-4 {
            continue;
        }
        // BlowItUp.
        rt.destroyed = true;
        rt.interpolating = false;
        rt.close_timer = None;
        if !info.event.is_empty() && info.event != "None" {
            crate::triggers::note_source(&info.event, &crate::triggers::group_of(&level.0.actors[info.actor].props));
            events.0.push((info.event.clone(), true));
        }
        let ax = rotator_axes(rt.rotation);
        let pre = to_dvec(info.pre_pivot);
        let origin = rt.location
            - DVec3::new(
                ax[0][0] as f64 * pre.x + ax[1][0] as f64 * pre.y + ax[2][0] as f64 * pre.z,
                ax[0][1] as f64 * pre.x + ax[1][1] as f64 * pre.y + ax[2][1] as f64 * pre.z,
                ax[0][2] as f64 * pre.x + ax[1][2] as f64 * pre.y + ax[2][2] as f64 * pre.z,
            );
        let n = crate::sound::fprop(p, "NumFragments", 16.0) as usize;
        let spread = crate::sound::fprop(p, "FragmentSpread", 32.0) as f64;
        let class =
            crate::sound::prop(p, "FragmentClass").unwrap_or_else(|| "DeusEx.WoodFragment".into());
        crate::destruct::burst(crate::destruct::Burst {
            class: class
                .rsplit('.')
                .next()
                .unwrap_or("WoodFragment")
                .to_string(),
            pos: origin,
            count: n,
            spread,
            scale: crate::sound::fprop(p, "FragmentScale", 2.0),
            skin: crate::sound::prop(p, "FragmentTexture"),
        });
        if matches!(
            unreal_pkg::props::find(p, "bDrawExplosion"),
            Some(unreal_pkg::props::Value::Bool(true))
        ) {
            let r = if spread < 64.0 {
                64.0
            } else if spread < 128.0 {
                192.0
            } else {
                300.0
            };
            booms.0.push(crate::explosion::Boom {
                pos: origin,
                damage: 0.0,
                radius: r,
                damage_type: "Exploded",
                from: None,
                rings: false,
                fireball: true,
                fx: 0,
            });
        }
        let snd = if n <= 5 {
            "ExplodeSound1"
        } else {
            "ExplodeSound2"
        };
        if let Some(s) = crate::sound::prop(p, snd) {
            crate::sound::play(&s, origin, 2.0, (spread * 256.0) as f32, 1.0);
        }
        noises.0.push(crate::stealth::Noise {
            pos: origin,
            volume: 2.0,
            radius: (spread * 16.0) as f32,
            maker: None,
            instigator: Some(crate::ai::Target::Player),
            kind: crate::stealth::NoiseKind::Loud,
        });
        // Out of the way and out of collision.
        rt.location.z += 20000.0;
        if let Ok(mut tf) = tfs.get_mut(ent) {
            let l = [
                rt.location.x as f32,
                rt.location.y as f32,
                rt.location.z as f32,
            ];
            tf.translation = Vec3::from(ue_to_bevy(l));
        }
        println!("MOVER {} destroyed", info.name);
    }
}

/// A new map is going in (loader.rs): this module's state of the map being left.
pub fn reset_level() {
    if let Ok(mut v) = EVENTS_OUT.lock() {
        v.clear();
    }
    if let Ok(mut v) = PAWN_DOORS.lock() {
        v.clear();
    }
    if let Ok(mut v) = LINE_HITS.lock() {
        v.clear();
    }
    if let Ok(mut v) = BLASTS.lock() {
        v.clear();
    }
    if let Ok(mut v) = DIRECT.lock() {
        v.clear();
    }
}
