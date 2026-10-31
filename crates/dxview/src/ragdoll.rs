//! Ragdolls (enhancement "Ragdolls", with Skeletal Characters): a person on a rebuilt body goes limp at once on
//! the killing (or knock-out) blow, carrying the hit's push and how they were moving. Avian rigid bodies (capsules)
//! stand for the hips, chest, head, upper and lower arms and legs, held together by spherical joints with swing /
//! twist limits; every other bone keeps its pose relative to the bone it hangs from (hands, feet, fingers, the
//! coat). The level is solid to them through static triangle colliders built from the drawn BSP, a cell at a time
//! around each body. Once a body has lain still for a moment (or after a while), its physics is removed and the
//! pose stays. Explosions still gib (ai.rs / effects.rs).

use avian3d::prelude::*;
use bevy::prelude::*;
use glam::DVec3;
use std::collections::HashMap;
use std::sync::Mutex;

use crate::skeletal::SkelBody;

pub fn enabled() -> bool {
    crate::settings::get("Ragdolls") > 0.5 && crate::skeletal::enabled()
}

/// A killing / knock-out blow (ai.rs), Unreal space.
pub struct Death {
    pub blocker: usize,
    pub hit: DVec3,
    pub from: DVec3,
    pub damage: f32,
    pub vel: DVec3,
}

static DEATHS: Mutex<Vec<Death>> = Mutex::new(Vec::new());

pub fn request(d: Death) {
    if let Ok(mut g) = DEATHS.lock() {
        g.push(d);
    }
}

/// Unreal gravity (950 units / s²) in Bevy metres.
const GRAVITY: f32 = 950.0 / 52.5;
/// World cells for the static colliders (metres).
const CELL: f32 = 8.0;
const LAYER_RAG: u32 = 1 << 1;
const LAYER_WORLD: u32 = 1 << 0;
/// Longest a ragdoll simulates (seconds), and how long it must lie still before it's frozen.
const MAX_LIFE: f32 = 12.0;
const STILL_FOR: f32 = 1.5;
/// Below this (m/s) a body counts as lying still.
const STILL_SPEED: f32 = 0.35;

pub struct RagdollPlugin;

impl Plugin for RagdollPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PhysicsPlugins::default())
            .insert_resource(Gravity(Vec3::NEG_Y * GRAVITY))
            .init_resource::<RagWorld>()
            .add_systems(Update, start_system.after(crate::skeletal::pose_system))
            .add_systems(PostUpdate, drive_system.before(bevy::transform::TransformSystems::Propagate));
    }
}

/// One limb or the trunk: the bone whose head it starts at and the bone whose head is its far end.
struct PartDef {
    bone: &'static str,
    tail: &'static str,
    /// Extend past the tail (the head's top, the hand, the foot) by this fraction of the length.
    extend: f32,
    radius: f32,
    parent: Option<usize>,
    /// Joint limits (radians): swing half-angle, twist either way.
    swing: f32,
    twist: f32,
}

const PARTS: &[PartDef] = &[
    PartDef { bone: "DEF-spine", tail: "DEF-spine.002", extend: 0.0, radius: 0.13, parent: None, swing: 0.0, twist: 0.0 },
    PartDef { bone: "DEF-spine.003", tail: "DEF-spine.005", extend: 0.0, radius: 0.15, parent: Some(0), swing: 0.5, twist: 0.4 },
    PartDef { bone: "DEF-spine.005", tail: "DEF-spine.006", extend: 1.6, radius: 0.1, parent: Some(1), swing: 0.7, twist: 0.7 },
    PartDef { bone: "DEF-upper_arm.L", tail: "DEF-forearm.L", extend: 0.0, radius: 0.05, parent: Some(1), swing: 1.6, twist: 0.9 },
    PartDef { bone: "DEF-forearm.L", tail: "DEF-hand.L", extend: 0.45, radius: 0.045, parent: Some(3), swing: 1.4, twist: 0.3 },
    PartDef { bone: "DEF-upper_arm.R", tail: "DEF-forearm.R", extend: 0.0, radius: 0.05, parent: Some(1), swing: 1.6, twist: 0.9 },
    PartDef { bone: "DEF-forearm.R", tail: "DEF-hand.R", extend: 0.45, radius: 0.045, parent: Some(5), swing: 1.4, twist: 0.3 },
    PartDef { bone: "DEF-thigh.L", tail: "DEF-shin.L", extend: 0.0, radius: 0.075, parent: Some(0), swing: 1.2, twist: 0.4 },
    PartDef { bone: "DEF-shin.L", tail: "DEF-foot.L", extend: 0.15, radius: 0.055, parent: Some(7), swing: 1.3, twist: 0.2 },
    PartDef { bone: "DEF-thigh.R", tail: "DEF-shin.R", extend: 0.0, radius: 0.075, parent: Some(0), swing: 1.2, twist: 0.4 },
    PartDef { bone: "DEF-shin.R", tail: "DEF-foot.R", extend: 0.15, radius: 0.055, parent: Some(9), swing: 1.3, twist: 0.2 },
];

/// Bones hung straight off the rig root (the export flattens Rigify's parenting) and the bone each rides with:
/// (bone prefix, side suffix or "", bone ridden).
const RIDERS: &[(&str, &str, &str)] = &[
    ("DEF-pelvis", "", "DEF-spine"),
    ("DEF-coat", "", "DEF-spine"),
    ("DEF-shoulder", "", "DEF-spine.003"),
    ("DEF-breast", "", "DEF-spine.003"),
    ("DEF-weapon", "", "DEF-spine.003"),
    ("DEF-f_", ".L", "DEF-hand.L"),
    ("DEF-thumb", ".L", "DEF-hand.L"),
    ("DEF-f_", ".R", "DEF-hand.R"),
    ("DEF-thumb", ".R", "DEF-hand.R"),
];

#[derive(Clone, Copy)]
enum Drive {
    /// Carried by a body: its world transform relative to the body.
    Part(usize, Mat4),
    /// Rides another bone: its world transform relative to that bone's.
    Rider(usize, Mat4),
}

#[derive(Component)]
pub struct Ragdoll {
    bodies: Vec<Entity>,
    joints: Vec<Entity>,
    /// Rig node -> (part, the bone's world transform relative to the part's body at the start).
    driven: Vec<Option<Drive>>,
    blocker: usize,
    /// Bones in the order to compute them (a rider and what hangs off it after the bone it rides).
    order: Vec<usize>,
    /// Joint anchors in each body's frame (parent part, child part, anchor in parent, anchor in child): DXVIEW_RAGLOG.
    anchors: Vec<(usize, usize, Vec3, Vec3)>,
    life: f32,
    still: f32,
}

/// Static colliders for the level, a cell at a time.
#[derive(Resource, Default)]
pub struct RagWorld {
    generation: u64,
    tris: HashMap<(i32, i32, i32), Vec<[Vec3; 3]>>,
    built: bool,
    spawned: HashMap<(i32, i32, i32), Entity>,
}

fn cell_of(p: Vec3) -> (i32, i32, i32) {
    ((p.x / CELL).floor() as i32, (p.y / CELL).floor() as i32, (p.z / CELL).floor() as i32)
}

impl RagWorld {
    fn build(&mut self, lvl: &dx_level::Level) {
        self.built = true;
        for b in lvl.batches.iter().filter(|b| {
            !b.sky && matches!(b.class, dx_level::SurfaceClass::Opaque | dx_level::SurfaceClass::Masked)
        }) {
            for t in b.indices.chunks_exact(3) {
                let p = [0, 1, 2].map(|k| Vec3::from(b.positions[t[k] as usize]));
                if (p[1] - p[0]).cross(p[2] - p[0]).length_squared() < 1e-10 {
                    continue;
                }
                // Into every cell its bounds touch.
                let lo = p[0].min(p[1]).min(p[2]);
                let hi = p[0].max(p[1]).max(p[2]);
                let (a, z) = (cell_of(lo), cell_of(hi));
                for x in a.0..=z.0 {
                    for y in a.1..=z.1 {
                        for w in a.2..=z.2 {
                            self.tris.entry((x, y, w)).or_default().push(p);
                        }
                    }
                }
            }
        }
    }

    /// Static colliders for every cell within `r` metres of `p`.
    fn ensure(&mut self, commands: &mut Commands, p: Vec3, r: f32) {
        let (a, z) = (cell_of(p - Vec3::splat(r)), cell_of(p + Vec3::splat(r)));
        for x in a.0..=z.0 {
            for y in a.1..=z.1 {
                for w in a.2..=z.2 {
                    let c = (x, y, w);
                    if self.spawned.contains_key(&c) {
                        continue;
                    }
                    let Some(t) = self.tris.get(&c) else { continue };
                    let verts: Vec<Vec3> = t.iter().flatten().copied().collect();
                    let idx: Vec<[u32; 3]> = (0..t.len() as u32).map(|i| [i * 3, i * 3 + 1, i * 3 + 2]).collect();
                    let e = commands
                        .spawn((
                            RigidBody::Static,
                            Collider::trimesh(verts, idx),
                            CollisionLayers::new(LAYER_WORLD, LAYER_RAG),
                            Friction::new(0.9),
                            Transform::default(),
                        ))
                        .id();
                    self.spawned.insert(c, e);
                }
            }
        }
    }
}

fn affine(m: &Mat4) -> Transform {
    Transform::from_matrix(*m)
}

/// World transforms of every rig node from the bones' current local transforms.
fn globals(s: &SkelBody, root: Mat4, local: impl Fn(usize) -> Mat4) -> Vec<Mat4> {
    let rig = s.rig;
    let mut g: Vec<Mat4> = Vec::with_capacity(rig.names.len());
    for i in 0..rig.names.len() {
        let parent = rig.parent[i].map_or(root, |p| g[p]);
        g.push(parent * local(i));
    }
    g
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn start_system(
    mut commands: Commands,
    level: Res<crate::LevelRes>,
    mut world: ResMut<RagWorld>,
    mut pawns: Query<(Entity, &crate::ai::Npc, &mut SkelBody), Without<Ragdoll>>,
    gts: Query<&GlobalTransform>,
    tfs: Query<&Transform>,
    mut map_gen: Local<u64>,
) {
    if crate::loader::new_map(&mut map_gen) {
        *world = RagWorld { generation: *map_gen, ..default() };
    }
    let deaths: Vec<Death> = DEATHS.lock().map(|mut g| std::mem::take(&mut *g)).unwrap_or_default();
    if deaths.is_empty() || !enabled() {
        return;
    }
    let _prof = crate::prof::scope("ragdoll::start_system");
    if !world.built {
        world.build(&level.0);
    }
    for d in deaths {
        let Some((_e, _, mut s)) = pawns.iter_mut().find(|(_, n, _)| n.blocker == d.blocker) else { continue };
        let e = _e;
        let Ok(root_g) = gts.get(s.root) else { continue };
        // The pose on screen, as world transforms.
        let root = root_g.to_matrix();
        let nodes = s.nodes.clone();
        let g = globals(&s, root, |i| tfs.get(nodes[i]).map_or(Mat4::IDENTITY, |t| t.to_matrix()));
        let idx = |name: &str| s.rig.names.iter().position(|n| n == name);
        // Bodies.
        let push_dir = {
            let v = d.hit - d.from;
            let v = if v.length_squared() > 1e-6 { v.normalize() } else { DVec3::X };
            Vec3::new(v.x as f32, v.z as f32, v.y as f32)
        };
        let hit = Vec3::from(dx_level::ue_to_bevy(d.hit.as_vec3().to_array()));
        let run = Vec3::new(d.vel.x as f32, d.vel.z as f32, d.vel.y as f32) * dx_level::METERS_PER_UNIT;
        let kick = (d.damage * 0.12).clamp(1.5, 7.0);
        let mut bodies: Vec<Entity> = Vec::new();
        let mut body_w: Vec<Mat4> = Vec::new();
        let mut ok = true;
        // Which part the shot hit: the one whose segment passes nearest.
        let mut segs: Vec<(Vec3, Vec3)> = Vec::new();
        for p in PARTS {
            let (Some(a), Some(b)) = (idx(p.bone), idx(p.tail)) else {
                ok = false;
                break;
            };
            let ha = g[a].w_axis.truncate();
            let mut hb = g[b].w_axis.truncate();
            hb += (hb - ha) * p.extend;
            segs.push((ha, hb));
        }
        if !ok {
            continue;
        }
        let near = segs
            .iter()
            .enumerate()
            .map(|(i, (a, b))| {
                let ab = *b - *a;
                let t = ((hit - *a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
                (i, (*a + ab * t).distance_squared(hit))
            })
            .min_by(|x, y| x.1.total_cmp(&y.1))
            .map_or(1, |x| x.0);
        world.ensure(&mut commands, segs[0].0, 3.0);
        for (i, p) in PARTS.iter().enumerate() {
            let (a, b) = segs[i];
            let len = a.distance(b).max(0.02);
            let axis = (b - a) / len;
            let rot = Quat::from_rotation_arc(Vec3::Y, axis);
            let mid = (a + b) * 0.5;
            let w = Mat4::from_rotation_translation(rot, mid);
            let mut v = run;
            if i == near {
                v += push_dir * kick;
            } else if PARTS[near].parent == Some(i) || p.parent == Some(near) {
                v += push_dir * kick * 0.4;
            }
            let body = commands
                .spawn((
                    RigidBody::Dynamic,
                    Collider::capsule(p.radius, (len - 2.0 * p.radius).max(0.01)),
                    CollisionLayers::new(LAYER_RAG, LAYER_WORLD),
                    Friction::new(0.8),
                    Restitution::new(0.0),
                    LinearDamping(0.1),
                    AngularDamping(1.2),
                    LinearVelocity(v),
                    TransformInterpolation,
                    Transform::from_translation(mid).with_rotation(rot),
                ))
                .id();
            bodies.push(body);
            body_w.push(w);
        }
        // Joints at each child part's start, the twist axis along the child.
        let mut joints = Vec::new();
        let mut anchors = Vec::new();
        for (i, p) in PARTS.iter().enumerate() {
            let Some(pi) = p.parent else { continue };
            let (a, b) = segs[i];
            anchors.push((pi, i, body_w[pi].inverse().transform_point3(a), body_w[i].inverse().transform_point3(a)));
            let rot = Quat::from_rotation_arc(Vec3::Y, (b - a).normalize_or(Vec3::Y));
            joints.push(
                commands
                    .spawn((
                        SphericalJoint::new(bodies[pi], bodies[i])
                            .with_anchor(a)
                            .with_basis(rot)
                            .with_swing_limits(-p.swing, p.swing)
                            .with_twist_limits(-p.twist, p.twist),
                        JointCollisionDisabled,
                    ))
                    .id(),
            );
        }
        // Which bones the bodies carry, and which ride other bones.
        let count = s.rig.names.len();
        let mut driven: Vec<Option<Drive>> = vec![None; count];
        for (pi, p) in PARTS.iter().enumerate() {
            if let Some(n) = idx(p.bone) {
                driven[n] = Some(Drive::Part(pi, body_w[pi].inverse() * g[n]));
            }
        }
        for (n, name) in s.rig.names.iter().enumerate() {
            if driven[n].is_some() || s.rig.parent[n].is_some_and(|p| p != 0) || s.rig.parent[n].is_none() {
                continue;
            }
            let side_ok = |sfx: &str| sfx.is_empty() || name.contains(sfx);
            if let Some(t) = RIDERS.iter().find(|(pre, sfx, _)| name.starts_with(pre) && side_ok(sfx)).and_then(|r| idx(r.2)) {
                driven[n] = Some(Drive::Rider(t, g[t].inverse() * g[n]));
            }
        }
        // Order: everything but the riders' subtrees, then those.
        let in_rider = |mut n: usize| loop {
            if matches!(driven[n], Some(Drive::Rider(..))) {
                break true;
            }
            match s.rig.parent[n] {
                Some(p) => n = p,
                None => break false,
            }
        };
        let mut order: Vec<usize> = (0..count).filter(|n| !in_rider(*n)).collect();
        order.extend((0..count).filter(|n| in_rider(*n)));
        s.ragdoll = true;
        if std::env::var_os("DXVIEW_RAGLOG").is_some() {
            println!("RAGDOLL start blocker {} hit part {near} kick {kick:.1} run {run:?} hips {:?}", d.blocker, segs[0].0);
        }
        commands.entity(e).insert(Ragdoll { bodies, joints, driven, blocker: d.blocker, order, anchors, life: 0.0, still: 0.0 });
    }
}

#[allow(clippy::type_complexity)]
pub fn drive_system(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &SkelBody, &mut Ragdoll)>,
    bodies: Query<(&Transform, &LinearVelocity)>,
    gts: Query<&GlobalTransform>,
    mut tfs: Query<&mut Transform, (Without<LinearVelocity>, Without<SkelBody>)>,
) {
    let dt = time.delta_secs();
    for (e, s, mut r) in &mut q {
        let Ok(root_g) = gts.get(s.root) else { continue };
        let root = root_g.to_matrix();
        let mut w: Vec<Mat4> = Vec::with_capacity(r.bodies.len());
        let mut fastest = 0.0f32;
        for b in &r.bodies {
            let Ok((t, v)) = bodies.get(*b) else {
                w.push(Mat4::IDENTITY);
                continue;
            };
            w.push(t.to_matrix());
            fastest = fastest.max(v.0.length());
        }
        // World transforms, top down: a carried bone from its body, the rest from its parent.
        let rig = s.rig;
        let mut g: Vec<Mat4> = vec![Mat4::IDENTITY; rig.names.len()];
        for &i in &r.order {
            let parent = rig.parent[i].map_or(root, |p| g[p]);
            let set = match r.driven[i] {
                Some(Drive::Part(pi, off)) => Some(w[pi] * off),
                Some(Drive::Rider(t, off)) => Some(g[t] * off),
                None => None,
            };
            g[i] = match set {
                Some(gi) => {
                    if let Ok(mut t) = tfs.get_mut(s.nodes[i]) {
                        *t = affine(&(parent.inverse() * gi));
                    }
                    gi
                }
                None => parent * tfs.get(s.nodes[i]).map_or(Mat4::IDENTITY, |t| t.to_matrix()),
            };
        }
        r.life += dt;
        r.still = if fastest < STILL_SPEED { r.still + dt } else { 0.0 };
        if std::env::var_os("DXVIEW_RAGLOG").is_some() && (r.life * 4.0).floor() != ((r.life - dt) * 4.0).floor() {
            let sep = r
                .anchors
                .iter()
                .map(|(a, b, la, lb)| (*b, w[*a].transform_point3(*la).distance(w[*b].transform_point3(*lb))))
                .fold((0, 0.0f32), |m, x| if x.1 > m.1 { x } else { m });
            println!("RAGDOLL t {:.2} fastest {fastest:.2} hips {:?} worst joint gap {sep:?}", r.life, w.first().map(|m| m.w_axis.truncate()));
        }
        if r.life > MAX_LIFE || r.still > STILL_FOR {
            // Lying still: the pose stays, the physics goes; the blood pools where the chest lies.
            if let Some(c) = w.get(1) {
                let p = dx_level::bevy_to_ue(c.w_axis.truncate().to_array());
                crate::gore::rested(r.blocker, DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64));
            }
            for b in r.bodies.drain(..) {
                commands.entity(b).despawn();
            }
            for j in r.joints.drain(..) {
                commands.entity(j).despawn();
            }
            commands.entity(e).remove::<Ragdoll>();
        }
    }
}
