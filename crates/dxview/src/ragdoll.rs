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

/// The blocker number a Death for JC himself carries.
pub const PLAYER: usize = usize::MAX;

/// JC's body when he dies on a rebuilt body (death.rs then hides its vertex-animated one).
#[derive(Component)]
pub struct PlayerBody;

static PLAYER_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// JC fell as a ragdoll (death.rs hides its own death body).
pub fn player_ragdoll() -> bool {
    PLAYER_ON.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn enabled() -> bool {
    setting!("Ragdolls") > 0.5 && crate::skeletal::enabled()
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

/// Ragdolls simulating at once, and how far from the player one may start (Unreal units, ~40 m).
const MAX_ACTIVE: usize = 6;
const FAR_UNITS: f64 = 2100.0;

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
            .add_systems(Update, (player_body_system.before(start_system), start_system.after(crate::skeletal::pose_system)))
            .add_systems(PostUpdate, drive_system.before(bevy::transform::TransformSystems::Propagate))
            .add_systems(Update, follow_movers_system)
            .add_systems(First, pause_system);
        // Avian puts dozens of small systems in FixedPostUpdate. Handed out one by one to worker threads they cost
        // close to a millisecond a frame in scheduling alone, idle or not; run in order on one thread they're
        // nearly free, and the solver's own parallel loops still spread over the workers while a body falls.
        app.edit_schedule(FixedPostUpdate, |s| {
            s.set_executor(bevy::ecs::schedule::SingleThreadedExecutor::new());
        });
    }
}

/// The physics clock runs only while a ragdoll is falling: with nothing to simulate Avian's step (broad phase,
/// solver, six substeps) still cost a couple of milliseconds a frame - most of the game, and all of it with
/// Ragdolls off. Paused, its schedule doesn't run at all.
fn pause_system(
    bodies: Query<&RigidBody>,
    mut time: ResMut<Time<Physics>>,
    mut config: ResMut<avian3d::physics_transform::PhysicsTransformConfig>,
) {
    let pending = DEATHS.lock().is_ok_and(|d| !d.is_empty());
    let live = pending || bodies.iter().any(|b| *b == RigidBody::Dynamic);
    if live && time.is_paused() {
        time.unpause();
    } else if !live && !time.is_paused() {
        time.pause();
    }
    // Avian propagates every transform in the world before each physics step, paused or not (half a millisecond of
    // the main thread for nothing while no ragdoll falls).
    if config.propagate_before_physics != live {
        config.propagate_before_physics = live;
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
    hinge: Option<(f32, f32)>,
}

/// Hinged parts (knees, elbows): `hinge` = Some(range of the bend in radians, about the body's left-to-right axis -
/// negative swings the part backward, as a knee bends; positive forward, as an elbow). Spherical parts use swing /
/// twist; hinges ignore them.
const PARTS: &[PartDef] = &[
    PartDef { bone: "DEF-spine", tail: "DEF-spine.002", extend: 0.0, radius: 0.13, parent: None, swing: 0.0, twist: 0.0, hinge: None },
    PartDef { bone: "DEF-spine.003", tail: "DEF-spine.005", extend: 0.0, radius: 0.15, parent: Some(0), swing: 0.45, twist: 0.3, hinge: None },
    PartDef { bone: "DEF-spine.005", tail: "DEF-spine.006", extend: 1.6, radius: 0.1, parent: Some(1), swing: 0.6, twist: 0.5, hinge: None },
    PartDef { bone: "DEF-upper_arm.L", tail: "DEF-forearm.L", extend: 0.0, radius: 0.05, parent: Some(1), swing: 1.4, twist: 0.5, hinge: None },
    PartDef { bone: "DEF-forearm.L", tail: "DEF-hand.L", extend: 0.45, radius: 0.045, parent: Some(3), swing: 0.0, twist: 0.0, hinge: Some((-0.05, 2.4)) },
    PartDef { bone: "DEF-upper_arm.R", tail: "DEF-forearm.R", extend: 0.0, radius: 0.05, parent: Some(1), swing: 1.4, twist: 0.5, hinge: None },
    PartDef { bone: "DEF-forearm.R", tail: "DEF-hand.R", extend: 0.45, radius: 0.045, parent: Some(5), swing: 0.0, twist: 0.0, hinge: Some((-0.05, 2.4)) },
    PartDef { bone: "DEF-thigh.L", tail: "DEF-shin.L", extend: 0.0, radius: 0.075, parent: Some(0), swing: 0.85, twist: 0.12, hinge: None },
    PartDef { bone: "DEF-shin.L", tail: "DEF-foot.L", extend: 0.15, radius: 0.055, parent: Some(7), swing: 0.0, twist: 0.0, hinge: Some((-2.3, 0.05)) },
    PartDef { bone: "DEF-thigh.R", tail: "DEF-shin.R", extend: 0.0, radius: 0.075, parent: Some(0), swing: 0.85, twist: 0.12, hinge: None },
    PartDef { bone: "DEF-shin.R", tail: "DEF-foot.R", extend: 0.15, radius: 0.055, parent: Some(9), swing: 0.0, twist: 0.0, hinge: Some((-2.3, 0.05)) },
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
    tris: HashMap<(i32, i32, i32), Vec<[Vec3; 3]>>,
    built: bool,
    spawned: HashMap<(i32, i32, i32), Entity>,
    /// Decorations (by blocker) and mover pieces (by mesh entity) given a collider already.
    extra: std::collections::HashSet<(u8, u64)>,
}

/// A collider that copies a mover piece's place each frame (doors swing; the ragdoll mustn't fall through one).
#[derive(Component)]
pub struct RagMover(pub Entity);

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
    mut pawns: Query<(Entity, Option<&crate::ai::Npc>, Option<&PlayerBody>, &mut SkelBody), Without<Ragdoll>>,
    gts: Query<&GlobalTransform>,
    tfs: Query<&Transform>,
    mut map_gen: Local<u64>,
    blockers: Res<crate::actors::Blockers>,
    movers: Option<Res<crate::movers::Movers>>,
    kids: Query<&Children>,
    mover_meshes: Query<(&Mesh3d, &GlobalTransform)>,
    mesh_assets: Res<Assets<Mesh>>,
    active: Query<(), With<Ragdoll>>,
    player: Option<Res<crate::Player>>,
) {
    if crate::loader::new_map(&mut map_gen) {
        *world = RagWorld::default();
    }
    let deaths: Vec<Death> = DEATHS.lock().map(|mut g| std::mem::take(&mut *g)).unwrap_or_default();
    if deaths.is_empty() || !enabled() {
        return;
    }
    let _prof = crate::prof::scope("ragdoll::start_system");
    if !world.built {
        world.build(&level.0);
    }
    // At most MAX_ACTIVE bodies simulate at once, and only near the player: the mission script killing every
    // terrorist on the island at once (Mission01 StatueMissionComplete) started ~28 ragdolls together and the game
    // ran at 4 fps until they settled. The rest play their death animation.
    let mut live = active.iter().count();
    let me = player.as_ref().map(|p| p.body.location);
    for d in deaths {
        if d.blocker != PLAYER {
            let near = match (me, blockers.list.get(d.blocker)) {
                (Some(p), Some(c)) => c.center.distance(p) < FAR_UNITS,
                _ => true,
            };
            if !near || live >= MAX_ACTIVE {
                continue;
            }
        }
        live += 1;
        let Some((_e, _, _, mut s)) = pawns
            .iter_mut()
            .find(|(_, n, pb, _)| n.is_some_and(|n| n.blocker == d.blocker) || (pb.is_some() && d.blocker == PLAYER))
        else {
            continue;
        };
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
        // The body's left-to-right axis (hips, else shoulders): the hinges' axis.
        let lr = |a: &str, b: &str| Some(g[idx(b)?].w_axis.truncate() - g[idx(a)?].w_axis.truncate()).filter(|v| v.length_squared() > 1e-6);
        let right = lr("DEF-thigh.L", "DEF-thigh.R").or_else(|| lr("DEF-upper_arm.L", "DEF-upper_arm.R")).unwrap_or(Vec3::X).normalize();
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
        world.ensure(&mut commands, segs[0].0, 6.0);
        // Things in the world besides the map: decorations (their collision cylinders) and movers (doors, lifts:
        // their pieces' meshes, following them as they move).
        let c0 = segs[0].0;
        for (bi, c) in blockers.list.iter().enumerate() {
            if c.radius <= 2.0 || !blockers.deco.contains_key(&bi) || world.extra.contains(&(0, bi as u64)) {
                continue;
            }
            let at = Vec3::from(dx_level::ue_to_bevy([c.center.x as f32, c.center.y as f32, c.center.z as f32]));
            if at.distance(c0) > 6.0 {
                continue;
            }
            world.extra.insert((0, bi as u64));
            let m = dx_level::METERS_PER_UNIT;
            commands.spawn((
                RigidBody::Static,
                Collider::cylinder(c.radius as f32 * m, c.height as f32 * 2.0 * m),
                CollisionLayers::new(LAYER_WORLD, LAYER_RAG),
                Friction::new(0.9),
                Transform::from_translation(at),
            ));
        }
        if let Some(mv) = movers.as_ref() {
            for root in &mv.entities {
                let Ok(ch) = kids.get(*root) else { continue };
                for c in ch.iter() {
                    let Ok((mh, gt)) = mover_meshes.get(c) else { continue };
                    if world.extra.contains(&(1, c.to_bits())) || gt.translation().distance(c0) > 12.0 {
                        continue;
                    }
                    let Some(col) = mesh_assets.get(&mh.0).and_then(mesh_trimesh) else { continue };
                    world.extra.insert((1, c.to_bits()));
                    commands.spawn((
                        RigidBody::Kinematic,
                        col,
                        CollisionLayers::new(LAYER_WORLD, LAYER_RAG),
                        Friction::new(0.9),
                        gt.compute_transform(),
                        RagMover(c),
                    ));
                }
            }
        }
        for (i, p) in PARTS.iter().enumerate() {
            let (a, b) = segs[i];
            let len = a.distance(b).max(0.02);
            let axis = (b - a) / len;
            let rot = Quat::from_rotation_arc(Vec3::Y, axis);
            let mid = (a + b) * 0.5;
            let w = Mat4::from_rotation_translation(rot, mid);
            // The shot carries the whole body a little, the part hit most and its neighbours more; a hit above the
            // hips tips the body over away from the shooter (the legs get a touch the other way), a hit in the legs
            // sweeps them out from under it.
            let mut v = run + push_dir * kick * 0.3;
            if i == near {
                v += push_dir * kick * 0.8;
            } else if PARTS[near].parent == Some(i) || p.parent == Some(near) {
                v += push_dir * kick * 0.45;
            }
            let upper_hit = near <= 6;
            let is_leg = i >= 7;
            if upper_hit && !is_leg && i != 0 {
                v += push_dir * kick * 0.25;
            } else if upper_hit && is_leg {
                v -= push_dir * kick * 0.12;
            } else if !upper_hit && !is_leg && i != 0 {
                v -= push_dir * kick * 0.1;
            }
            let body = commands
                .spawn((
                    RigidBody::Dynamic,
                    // A little fatter than the bones (the clothes and flesh round them): the drawn body stays out of
                    // walls and floors, not just its skeleton.
                    Collider::capsule(p.radius * 1.2, (len - 2.4 * p.radius).max(0.01)),
                    CollisionLayers::new(LAYER_RAG, LAYER_WORLD),
                    // Swept collision: a fast limb can't tunnel through a thin floor or wall between two steps.
                    SweptCcd::default(),
                    Friction::new(0.8),
                    Restitution::new(0.0),
                    LinearDamping(0.15),
                    AngularDamping(2.2),
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
            let child = (b - a).normalize_or(Vec3::NEG_Y);
            let rot = Quat::from_rotation_arc(Vec3::Y, child);
            let j = match p.hinge {
                Some((lo, hi)) => {
                    // A hinge about the body's left-to-right axis (the basis's Z), its limits measured from the bend
                    // the pose already has (the joint starts at angle 0).
                    let (pa, pb) = segs[pi];
                    let parent_dir = (pb - pa).normalize_or(Vec3::NEG_Y);
                    // The hinge's own axis: square to the upper bone. The body's left-to-right line was only right
                    // for a leg straight under the hip - a thigh turned out or forward hinged the knee across its
                    // own plane, so shins swung sideways and twisted. A bent limb's axis is the one its pose shows
                    // (signed to the body's right); a straight one, the body's right made square to the bone.
                    let axis = {
                        let bent = parent_dir.cross(child);
                        let side = (right - parent_dir * right.dot(parent_dir)).normalize_or(right);
                        if bent.length() > 0.17 {
                            // Signed like the body's right, so the range (knees back, elbows forward) means the same.
                            let b = bent.normalize();
                            if b.dot(side) < 0.0 { -b } else { b }
                        } else {
                            side
                        }
                    };
                    let bend = parent_dir.cross(child).dot(axis).atan2(parent_dir.dot(child));
                    let y = (child - axis * child.dot(axis)).normalize_or(Vec3::Y);
                    let basis = Quat::from_mat3(&Mat3::from_cols(y.cross(axis), y, axis));
                    let (min, max) = ((lo - bend).min(0.0), (hi - bend).max(0.0));
                    commands
                        .spawn((
                            RevoluteJoint::new(bodies[pi], bodies[i])
                                .with_anchor(a)
                                .with_basis(basis)
                                .with_angle_limits(min, max),
                            JointCollisionDisabled,
                        ))
                        .id()
                }
                None => commands
                    .spawn((
                        SphericalJoint::new(bodies[pi], bodies[i])
                            .with_anchor(a)
                            .with_basis(rot)
                            .with_swing_limits(-p.swing, p.swing)
                            .with_twist_limits(-p.twist, p.twist),
                        JointCollisionDisabled,
                    ))
                    .id(),
            };
            joints.push(j);
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
        if env_on!("DXVIEW_RAGLOG") {
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
        if env_on!("DXVIEW_RAGLOG") && (r.life * 4.0).floor() != ((r.life - dt) * 4.0).floor() {
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

/// JC dies on his rebuilt body (Skeletal Characters + Ragdolls, and a body for his mesh / outfit): a body with the
/// textures his mirror / third-person body wears, where he stood, in the pose he was in, then a ragdoll the next
/// frame (the bones exist by then), pushed back from the view as death.rs's DeathBack would fall.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn player_body_system(
    mut commands: Commands,
    time: Res<Time>,
    mut ph: ResMut<crate::ai::PlayerHealth>,
    player: Res<crate::Player>,
    level: Res<crate::LevelRes>,
    mirror: Res<crate::mirror::MirrorBodyRes>,
    mut bp: crate::skeletal::BuildParams,
    mut bodies: Query<&mut SkelBody, (With<PlayerBody>, Without<crate::skeletal::MirrorSkel>)>,
    live: Query<&SkelBody, (With<crate::skeletal::MirrorSkel>, Without<PlayerBody>)>,
    gts: Query<&GlobalTransform>,
    cam: Query<&GlobalTransform, (With<crate::FlyCam>, Without<crate::SkyCam>)>,
    mut st: Local<Option<(Entity, u32, Vec3)>>,
    mut map_gen: Local<u64>,
) {
    if crate::loader::new_map(&mut map_gen) {
        *st = None;
    }
    // Developer aid: DXVIEW_DIE_AT=<secs> kills JC that long into the run (ragdoll tests).
    if !ph.dead {
        if let Some(at) = env_val!("DXVIEW_DIE_AT").map(str::to_owned).and_then(|v| v.parse::<f32>().ok()) {
            if time.elapsed_secs() > at && time.elapsed_secs() - time.delta_secs() <= at {
                ph.damage(1000.0, "Shot", DVec3::ZERO);
            }
        }
    }
    if !ph.dead {
        if let Some((root, ..)) = st.take() {
            commands.entity(root).despawn();
        }
        PLAYER_ON.store(false, std::sync::atomic::Ordering::Relaxed);
        return;
    }
    if let Some((root, frames, fwd)) = st.as_mut() {
        *frames += 1;
        if *frames == 2 {
            let loc = player.body.location;
            let f = DVec3::new(fwd.x as f64, fwd.z as f64, fwd.y as f64);
            request(Death {
                blocker: PLAYER,
                hit: loc + DVec3::new(0.0, 0.0, 20.0),
                from: loc + DVec3::new(0.0, 0.0, 20.0) + f * 200.0,
                damage: 25.0,
                vel: player.body.velocity,
            });
        }
        if let (Ok(mut s), Ok(g)) = (bodies.get_mut(*root), gts.get(*root)) {
            let ue = dx_level::bevy_to_ue(g.translation().to_array());
            bp.relight(&mut s, &level.0, ue, time.delta_secs());
        }
        return;
    }
    if !enabled() {
        return;
    }
    let Some(b) = mirror.0.as_ref() else { return };
    let Some(body) = crate::skeletal::body_for(&crate::travel::jc_mesh()) else { return };
    let fwd = cam.iter().next().map_or(Vec3::NEG_Z, |t| t.forward().as_vec3());
    let flat = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or(Vec3::NEG_Z);
    // The mirror body stands where JC does; face it the way he looked.
    let loc = player.body.location;
    let yaw = (flat.z as f64).atan2(flat.x as f64);
    let m = dx_level::actors::rotator_to_bevy_mat3([0, (yaw / std::f64::consts::TAU * 65536.0) as i32, 0]);
    let root = commands
        .spawn((
            Transform::from_translation(Vec3::from(dx_level::ue_to_bevy(loc.as_vec3().to_array())))
                .with_rotation(Quat::from_mat3(&Mat3::from_cols(Vec3::from(m[0]), Vec3::from(m[1]), Vec3::from(m[2])))),
            Visibility::default(),
        ))
        .id();
    let Some(mut s) = bp.build_from_mirror_world(&mut commands, b, body, root) else {
        commands.entity(root).despawn();
        return;
    };
    // The pose he had (his live rebuilt body), else his idle.
    if let Some(l) = live.iter().next().filter(|l| l.pose.len() == s.pose.len()) {
        s.pose.clone_from(&l.pose);
    } else if let Some(q) = b.mesh.seqs.iter().find(|q| q.name.eq_ignore_ascii_case("BreatheLight")) {
        crate::skeletal::advance_pub(&mut s, "BreatheLight", q.start_frame.max(0) as usize, q.num_frames.max(1) as usize, 0.0, true, 0.0);
    }
    for (i, n) in s.nodes.iter().enumerate() {
        let p = s.pose[i];
        commands.entity(*n).insert(Transform { translation: p.t, rotation: p.r, scale: p.s });
    }
    commands.entity(root).insert((s, PlayerBody));
    *st = Some((root, 0, flat));
    PLAYER_ON.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Mover pieces' colliders follow the pieces (a door swinging shut pushes a body; one opening lets it through).
fn follow_movers_system(mut q: Query<(&RagMover, &mut Transform)>, gts: Query<&GlobalTransform, Without<RagMover>>) {
    for (m, mut tf) in &mut q {
        if let Ok(g) = gts.get(m.0) {
            let want = g.compute_transform();
            if *tf != want {
                *tf = want;
            }
        }
    }
}

/// A triangle-mesh collider from a mesh's positions and indices.
fn mesh_trimesh(m: &Mesh) -> Option<Collider> {
    let Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) = m.attribute(Mesh::ATTRIBUTE_POSITION) else { return None };
    let idx: Vec<u32> = match m.indices()? {
        bevy::mesh::Indices::U32(v) => v.clone(),
        bevy::mesh::Indices::U16(v) => v.iter().map(|&i| i as u32).collect(),
    };
    if idx.len() < 3 {
        return None;
    }
    let verts: Vec<Vec3> = p.iter().map(|v| Vec3::from(*v)).collect();
    let tris: Vec<[u32; 3]> = idx.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect();
    Some(Collider::trimesh(verts, tris))
}
