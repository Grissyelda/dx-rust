//! Dismemberment (setting Dismemberment, with Skeletal Characters; never on Low Gore).
//!
//! A killing shot heavy enough takes off what it hit: a head blown apart (or knocked clean off by a big enough
//! round), an arm at the shoulder or the elbow, a leg at the hip or the knee. A shotgun's pellets count together.
//! What comes off is cut from the body's own skinned triangles as they stand that instant (the triangles weighted
//! mostly to the severed bones), and flies as a piece of its own: it tumbles, bounces off the level, leaves a trail
//! of drops and slaps down in a splat, then bleeds into a small pool. On the body the severed bones shrink to the
//! joint (so the limb is gone, ragdoll or not), the stump is a wound with bone in it (wounds.rs, drawn on the skin),
//! an artery pumps from it with a slowing heartbeat, and the pool under the body spreads wider.

use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
use bevy::prelude::*;
use glam::DVec3;
use std::sync::Mutex;

use crate::skeletal::{ActorMaterial, SkelBody};
use crate::wounds::{Kind, Rec, SkinWounds};

pub fn on() -> bool {
    setting!("Dismemberment") > 0.5 && crate::skeletal::enabled() && !crate::settings::low_gore()
}

struct Blow {
    blocker: usize,
    at: DVec3,
    dir: DVec3,
    damage: f32,
    damage_type: &'static str,
}

static BLOWS: Mutex<Vec<Blow>> = Mutex::new(Vec::new());

/// The blow that killed pawn `blocker` (ai.rs, not knocked out): maybe it takes something off.
pub fn death_blow(blocker: usize, at: DVec3, dir: DVec3, damage: f32, damage_type: &'static str) {
    if on() {
        if let Ok(mut g) = BLOWS.lock() {
            g.push(Blow { blocker, at, dir, damage, damage_type });
        }
    }
}

/// The body last cut (test aid: GOTO "secs;dx,dy,dz;gore" looks at it).
pub(crate) static LAST: Mutex<Option<Entity>> = Mutex::new(None);

pub fn reset_level() {
    if let Ok(mut g) = BLOWS.lock() {
        g.clear();
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Region {
    Head,
    UpperArm(bool),
    Forearm(bool),
    Thigh(bool),
    Shin(bool),
}

/// The part a bone belongs to (right = true).
fn region_of(bone: &str) -> Option<Region> {
    let right = bone.ends_with(".R") || bone.contains(".R.");
    Some(match bone {
        "DEF-spine.005" | "DEF-spine.006" => Region::Head,
        b if b.starts_with("DEF-upper_arm") || b.starts_with("DEF-shoulder") => Region::UpperArm(right),
        b if b.starts_with("DEF-forearm") || b.starts_with("DEF-hand") || b.starts_with("DEF-f_") || b.starts_with("DEF-thumb") => Region::Forearm(right),
        b if b.starts_with("DEF-thigh") => Region::Thigh(right),
        b if b.starts_with("DEF-shin") || b.starts_with("DEF-foot") || b.starts_with("DEF-toe") => Region::Shin(right),
        _ => return None,
    })
}

fn side(right: bool) -> &'static str {
    if right { "R" } else { "L" }
}

/// The bone the part comes off at, and the bone the stump stays on (its artery rides that one).
fn joints(r: Region) -> (String, String) {
    match r {
        Region::Head => ("DEF-spine.006".into(), "DEF-spine.005".into()),
        Region::UpperArm(s) => (format!("DEF-upper_arm.{}", side(s)), format!("DEF-shoulder.{}", side(s))),
        Region::Forearm(s) => (format!("DEF-forearm.{}", side(s)), format!("DEF-upper_arm.{}.001", side(s))),
        Region::Thigh(s) => (format!("DEF-thigh.{}", side(s)), format!("DEF-pelvis.{}", side(s))),
        Region::Shin(s) => (format!("DEF-shin.{}", side(s)), format!("DEF-thigh.{}.001", side(s))),
    }
}

/// How much damage (one blow, or pellets landing together) takes the part off.
fn threshold(r: Region) -> f32 {
    match r {
        Region::Head => 18.0,
        Region::Forearm(_) | Region::Shin(_) => 22.0,
        Region::UpperArm(_) | Region::Thigh(_) => 32.0,
    }
}

/// The stump's size in rest units.
fn stump_size(r: Region) -> f32 {
    match r {
        Region::Head => 0.045,
        Region::UpperArm(_) => 0.045,
        Region::Forearm(_) => 0.035,
        Region::Thigh(_) => 0.06,
        Region::Shin(_) => 0.042,
    }
}

/// The rig nodes that go with the part: its bone and all under it, and (for arms) the fingers, which hang off the
/// rig's root in these rigs.
fn region_nodes(s: &SkelBody, r: Region) -> Vec<usize> {
    let (root, _) = joints(r);
    let names = &s.rig.names;
    let Some(ri) = names.iter().position(|n| *n == root) else { return Vec::new() };
    let mut out = vec![ri];
    let mut i = 0;
    while i < out.len() {
        let p = out[i];
        for (c, par) in s.rig.parent.iter().enumerate() {
            if *par == Some(p) && !out.contains(&c) {
                out.push(c);
            }
        }
        i += 1;
    }
    if let Region::UpperArm(sd) | Region::Forearm(sd) = r {
        let suf = format!(".{}", side(sd));
        for (c, n) in names.iter().enumerate() {
            if (n.starts_with("DEF-f_") || n.starts_with("DEF-thumb")) && n.ends_with(&suf) && !out.contains(&c) {
                out.push(c);
            }
        }
    }
    out
}

/// The bones kept shrunk on a body (their parts are gone).
#[derive(Component, Default)]
pub struct Severed {
    nodes: Vec<Entity>,
}

/// A severed part flying, then lying.
#[derive(Component)]
pub struct Piece {
    /// Centre (Unreal space), velocity, spin (Bevy axes, radians a second).
    pos: DVec3,
    vel: DVec3,
    spin: Vec3,
    rot: Quat,
    resting: bool,
    landed: bool,
    trail: f32,
    life: f32,
    bleed: f32,
}

fn rnd(seed: &mut u64) -> f32 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    ((*seed >> 40) as f32) / (1u64 << 24) as f32
}

fn ue(p: Vec3) -> DVec3 {
    DVec3::from_array(dx_level::bevy_to_ue(p.to_array()).map(|v| v as f64))
}

fn bv(p: DVec3) -> Vec3 {
    Vec3::from(dx_level::ue_to_bevy(p.as_vec3().to_array()))
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn dismember_system(
    mut commands: Commands,
    time: Res<Time>,
    mut meshes: ResMut<Assets<Mesh>>,
    ibms: Res<Assets<SkinnedMeshInverseBindposes>>,
    mut mats: ResMut<Assets<ActorMaterial>>,
    mut bodies: Query<(Entity, &crate::ai::Npc, &SkelBody, Option<&mut Severed>, Option<&mut SkinWounds>)>,
    parts: crate::wounds::PartQuery,
    gts: Query<&GlobalTransform>,
    mut seed: Local<u64>,
) {
    let blows: Vec<Blow> = BLOWS.lock().map(|mut g| std::mem::take(&mut *g)).unwrap_or_default();
    if blows.is_empty() {
        return;
    }
    if *seed == 0 {
        *seed = 0xD15_3E3;
    }
    let now = time.elapsed_secs();
    let now_w = time.elapsed_secs_wrapped();
    for b in blows {
        let Some((e, npc, s, ..)) = bodies.iter().find(|(_, n, ..)| n.blocker == b.blocker) else { continue };
        if npc.hp_body.robot || npc.knocked_out {
            continue;
        }
        let total = b.damage.max(crate::wounds::recent_damage(b.blocker, now));
        let blast = b.damage_type == "Exploded";
        if !matches!(b.damage_type, "Shot" | "Sabot" | "Exploded") {
            continue;
        }
        // What the blow hit (wounds.rs found the bone under the skin there).
        let region = match crate::wounds::last_hit(b.blocker) {
            Some(h) if now - h.time < 0.6 => region_of(&h.bone),
            _ => None,
        };
        let mut cut: Vec<Region> = Vec::new();
        // Test aid: DXVIEW_DISMEMBER_FORCE=head|pop|arm|forearm|leg|shin cuts that off every killed body.
        let forced = env_val!("DXVIEW_DISMEMBER_FORCE").map(str::to_owned);
        let total = if forced.as_deref() == Some("pop") { 20.0 } else if forced.is_some() { 60.0 } else { total };
        if let Some(f) = forced.as_deref() {
            cut.push(match f {
                "arm" => Region::UpperArm(false),
                "forearm" => Region::Forearm(true),
                "leg" => Region::Thigh(false),
                "shin" => Region::Shin(true),
                _ => Region::Head,
            });
        } else if blast {
            // A blast that kills without blowing the body apart: a limb or two may go.
            if total >= 30.0 {
                let all = [Region::Forearm(false), Region::Forearm(true), Region::Shin(false), Region::Shin(true), Region::UpperArm(false), Region::Thigh(true)];
                let k = 1 + (rnd(&mut seed) * 1.6) as usize;
                for _ in 0..k {
                    let r = all[(rnd(&mut seed) * all.len() as f32) as usize % all.len()];
                    if !cut.contains(&r) {
                        cut.push(r);
                    }
                }
            }
        } else if let Some(r) = region {
            let k = if b.damage_type == "Sabot" { 1.3 } else { 1.0 };
            if total * k >= threshold(r) && rnd(&mut seed) < 0.85 {
                cut.push(r);
            }
        }
        if cut.is_empty() {
            continue;
        }
        let snap = crate::wounds::snapshot(s, &meshes, &ibms, &mats, &parts, &gts);
        if snap.is_empty() {
            continue;
        }
        let mut new_nodes: Vec<Entity> = Vec::new();
        let mut new_wounds: Vec<Rec> = Vec::new();
        let dir = Vec3::from(dx_level::ue_to_bevy(b.dir.as_vec3().to_array())).normalize_or(Vec3::NEG_Z);
        for r in cut {
            let (root, stay) = joints(r);
            let nodes = region_nodes(s, r);
            if nodes.is_empty() {
                continue;
            }
            let ents: Vec<Entity> = nodes.iter().map(|&i| s.nodes[i]).collect();
            let Some(ri) = s.rig.names.iter().position(|n| *n == root) else { continue };
            let Ok(joint_g) = gts.get(s.nodes[ri]) else { continue };
            let joint = joint_g.translation();
            // Each section's vertices: how much of each lies on the severed bones.
            let cutw: Vec<Vec<f32>> = snap
                .iter()
                .map(|sec| {
                    (0..sec.world.len())
                        .map(|v| (0..4).filter(|&k| sec.joint_ents.get(sec.joints[v][k] as usize).is_some_and(|j| ents.contains(j))).map(|k| sec.weights[v][k]).sum())
                        .collect()
                })
                .collect();
            // The stump: the point of the body left nearest the joint, in rest space.
            let mut best: Option<(f32, Vec3)> = None;
            for (si, sec) in snap.iter().enumerate() {
                for v in 0..sec.world.len() {
                    if cutw[si][v] >= 0.5 {
                        continue;
                    }
                    let d = sec.world[v].distance_squared(joint);
                    if best.is_none_or(|b| d < b.0) {
                        best = Some((d, Vec3::new(sec.rest[v][0], sec.rest[v][1], sec.rest[v][2])));
                    }
                }
            }
            let Some((_, stump_rest)) = best else { continue };
            let stump = Rec { rest: stump_rest, r: stump_size(r), birth: now_w, kind: Kind::Stump, run: 0.35, seed: rnd(&mut seed) * 97.0 };
            new_wounds.push(stump);
            new_nodes.extend(ents.iter().copied());
            // The head: a heavy round blows it apart; otherwise it comes off whole.
            let pop = r == Region::Head && !blast && total < 40.0 && (forced.as_deref() == Some("pop") || rnd(&mut seed) < 0.7);
            let joint_ue = ue(joint);
            if pop {
                crate::gore::chunks(joint_ue + b.dir.normalize_or(DVec3::X) * 4.0, b.dir, 7, 320.0);
                crate::gore::mist(joint_ue, b.dir * 60.0, 2.0);
                for _ in 0..3 {
                    crate::gore::spray(joint_ue, b.dir, 30.0);
                }
            } else {
                // The piece: its triangles cut out of the skinned sections as they stand.
                let mut pts: Vec<Vec3> = Vec::new();
                let mut built: Vec<(Mesh, Handle<ActorMaterial>)> = Vec::new();
                for (si, sec) in snap.iter().enumerate() {
                    let mut remap = vec![u32::MAX; sec.world.len()];
                    let (mut pos, mut nrm, mut uv, mut col, mut idx) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::<u32>::new());
                    for t in sec.indices.chunks_exact(3) {
                        let w = (cutw[si][t[0] as usize] + cutw[si][t[1] as usize] + cutw[si][t[2] as usize]) / 3.0;
                        if w < 0.5 {
                            continue;
                        }
                        for &v in t {
                            let v = v as usize;
                            if remap[v] == u32::MAX {
                                remap[v] = pos.len() as u32;
                                pos.push(sec.world[v]);
                                nrm.push(sec.normals[v].to_array());
                                uv.push(sec.uvs[v]);
                                col.push(sec.rest[v]);
                            }
                            idx.push(remap[v]);
                        }
                    }
                    if idx.is_empty() {
                        continue;
                    }
                    pts.extend(pos.iter().copied());
                    let mut m = Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::default());
                    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos.iter().map(|p| p.to_array()).collect::<Vec<_>>());
                    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
                    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
                    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
                    m.insert_indices(bevy::mesh::Indices::U32(idx));
                    built.push((m, sec.mat.clone()));
                }
                if !pts.is_empty() {
                    let centre = pts.iter().copied().sum::<Vec3>() / pts.len() as f32;
                    let root_e = commands
                        .spawn((
                            Transform::from_translation(centre),
                            Visibility::Inherited,
                            Piece {
                                pos: ue(centre),
                                vel: {
                                    let push = (total as f64 * 6.0).clamp(160.0, 460.0);
                                    let away = if blast { (ue(centre) - b.at).normalize_or(b.dir) } else { b.dir.normalize_or(DVec3::X) };
                                    away * push + DVec3::Z * (140.0 + rnd(&mut seed) as f64 * 120.0)
                                        + DVec3::new(rnd(&mut seed) as f64 - 0.5, rnd(&mut seed) as f64 - 0.5, 0.0) * 120.0
                                },
                                spin: Vec3::new(rnd(&mut seed) - 0.5, rnd(&mut seed) - 0.5, rnd(&mut seed) - 0.5) * 22.0,
                                rot: Quat::IDENTITY,
                                resting: false,
                                landed: false,
                                trail: 0.0,
                                life: 0.0,
                                bleed: 6.0,
                            },
                        ))
                        .id();
                    for (mut m, mat) in built {
                        if let Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) = m.attribute_mut(Mesh::ATTRIBUTE_POSITION) {
                            for q in p.iter_mut() {
                                *q = (Vec3::from(*q) - centre).to_array();
                            }
                        }
                        // Its own material: the section's, with the cut end drawn as a stump.
                        let Some(mut pm) = mats.get(&mat).cloned() else { continue };
                        crate::wounds::fill_one(&mut pm.extension.lights, &stump, now_w);
                        pm.base.cull_mode = None;
                        let mh = mats.add(pm);
                        commands.spawn((
                            Mesh3d(meshes.add(m)),
                            MeshMaterial3d(mh),
                            Transform::default(),
                            Visibility::Inherited,
                            bevy::camera::visibility::RenderLayers::from_layers(&[0, crate::mirror::WORLD_LAYER]),
                            bevy::light::NotShadowCaster,
                            ChildOf(root_e),
                        ));
                    }
                }
                crate::gore::spray(joint_ue, b.dir, 30.0);
                crate::gore::mist(joint_ue, b.dir * 40.0, 1.2);
            }
            crate::sound::play("DeusExSounds.Weapons.BatonHitFlesh", joint_ue, 1.0, 1200.0, 0.75 + rnd(&mut seed) * 0.15);
            // The artery: on the bone the stump stays on, pumping out along the severed bone's line.
            if let Some(si) = s.rig.names.iter().position(|n| *n == stay) {
                if let Ok(pg) = gts.get(s.nodes[si]) {
                    let inv = pg.to_matrix().inverse();
                    let out = (joint - pg.translation()).normalize_or(dir);
                    commands.spawn((
                        Transform::from_translation(inv.transform_point3(joint)),
                        Visibility::Hidden,
                        ChildOf(s.nodes[si]),
                        crate::wounds::Spurter {
                            owner: e,
                            left: 7.0,
                            total: 7.0,
                            beat: 0.0,
                            n: inv.transform_vector3(out).normalize_or(Vec3::Y),
                            strength: 1.0,
                        },
                    ));
                }
            }
            crate::gore::pool_bonus(b.blocker, 45.0);
            println!("DISMEMBER {:?} on pawn {} ({} damage, {})", r, b.blocker, total as i32, if pop { "burst" } else { "piece" });
        }
        if let Ok(mut g) = LAST.lock() {
            *g = Some(e);
        }
        if let Ok((_, _, _, sev, wounds)) = bodies.get_mut(e) {
            match sev {
                Some(mut sv) => sv.nodes.extend(new_nodes),
                None => {
                    commands.entity(e).insert(Severed { nodes: new_nodes });
                }
            }
            match wounds {
                Some(mut w) => {
                    for r in new_wounds {
                        w.add(r);
                    }
                }
                None => {
                    let mut w = SkinWounds::default();
                    for r in new_wounds {
                        w.add(r);
                    }
                    commands.entity(e).insert(w);
                }
            }
        }
    }
}

/// Severed bones stay shrunk to their joint (after the pose and the ragdoll have written them).
pub fn shrink_system(bodies: Query<&Severed>, mut tfs: Query<&mut Transform, Without<Piece>>) {
    for s in &bodies {
        for &n in &s.nodes {
            if let Ok(mut t) = tfs.get_mut(n) {
                if t.scale != Vec3::splat(1e-4) {
                    t.scale = Vec3::splat(1e-4);
                }
            }
        }
    }
}

/// Severed parts fly, tumble and bounce off the level, trailing drops; slap down in a splat and bleed a little
/// pool where they come to rest.
pub fn pieces_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<crate::LevelRes>,
    mut pieces: Query<(Entity, &mut Piece, &mut Transform)>,
    mut seed: Local<u64>,
) {
    if *seed == 0 {
        *seed = 0x9_1ECE;
    }
    let dt = time.delta_secs().min(0.05);
    let lvl = &level.0;
    let keep = setting!("PersistentDebris") > 0.5;
    for (e, mut p, mut t) in &mut pieces {
        p.life += dt;
        if !keep && p.life > 90.0 {
            commands.entity(e).despawn();
            continue;
        }
        if p.resting {
            if p.bleed > 0.0 {
                p.bleed -= dt;
                if rnd(&mut seed) < dt * 3.0 {
                    crate::gore::drip(p.pos + DVec3::Z * 2.0, DVec3::new(rnd(&mut seed) as f64 - 0.5, rnd(&mut seed) as f64 - 0.5, 0.0) * 20.0, 2.0);
                }
            }
            continue;
        }
        p.vel.z -= 950.0 * dt as f64;
        let from = p.pos;
        let to = from + p.vel * dt as f64;
        let h = lvl.collision.trace(from, to, DVec3::splat(3.0));
        if h.fraction < 1.0 {
            p.pos = from + (to - from) * h.fraction + h.normal * 0.5;
            let speed = p.vel.length();
            if !p.landed || speed > 150.0 {
                crate::gore::splat(p.pos - h.normal * 2.0, h.normal, if p.landed { 10.0 } else { 26.0 });
                if speed > 120.0 {
                    crate::sound::play("DeusExSounds.Generic.BodyThud", p.pos, 0.6, 800.0, 1.1 + rnd(&mut seed) * 0.2);
                }
            }
            p.landed = true;
            // Bounce a little, lose most of the speed along the surface, and the spin.
            let vn = h.normal * p.vel.dot(h.normal);
            let vt = p.vel - vn;
            p.vel = vt * 0.55 - vn * 0.25;
            p.spin *= 0.6;
            if h.normal.z > 0.6 && p.vel.length() < 50.0 {
                p.resting = true;
                p.vel = DVec3::ZERO;
                if env_on!("DXVIEW_GORELOG") {
                    println!("PIECE rests at {:?} after {:.1}s", p.pos, p.life);
                }
                crate::gore::pool(p.pos, 34.0 + rnd(&mut seed) * 16.0);
            }
        } else {
            p.pos = to;
            p.trail += dt;
            if p.trail > 0.035 && p.life < 3.0 {
                p.trail = 0.0;
                crate::gore::drip(p.pos, p.vel * 0.1, 2.0 + rnd(&mut seed) * 2.0);
            }
        }
        let spin = p.spin;
        p.rot = (Quat::from_scaled_axis(spin * dt) * p.rot).normalize();
        t.translation = bv(p.pos);
        t.rotation = p.rot;
    }
}
