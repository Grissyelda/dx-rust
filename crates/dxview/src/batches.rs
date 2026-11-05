//! Lean Rendering: still mesh-actor parts drawn together.
//!
//! Every part of a level mesh actor is a mesh of its own, because its lighting is baked into its vertex colours
//! where it stands (actors.rs), so Bevy can't instance them: Liberty Island's busy views drew about 550 of them, each
//! a draw of its own on the render thread. Parts that share a
//! material and a cell of the map, and haven't moved since the map settled, are copied into one mesh in world space
//! and drawn once. The parts stay (physics, frobbing, saves and every other system use them as before) but leave
//! every camera's layers. A part that moves, is re-lit, gets another mesh or material, is hidden or goes leaves its
//! batch and is drawn on its own again, and its batch is rebuilt without it. It may join again once it has been left
//! alone for 10 s (20, 40, then 80 s for one that keeps changing: a decoration by a flickering fire).
//!
//! The parts are looked at again every two seconds, so what comes into view later joins too. An animated actor's
//! parts join only while it holds a single-frame pose it has been drawn in (its idle loop is one frame and nothing
//! plays), and leave when it plays something.
//!
//! The cell is 256 m, so most maps are a few cells. On Liberty Island's busy dock view: 32 m cells took 42 of 813
//! draws, 128 m 72, one cell for the whole map 87.

use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::{Aabb, MeshAabb};
use bevy::camera::visibility::{NoAutoAabb, RenderLayers};
use bevy::math::Vec3A;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::actors::{Animated, EnvMapped, MeshActorRef};

/// The layer a batched part is moved to: no camera draws it.
const AWAY: usize = 31;

/// How long a new map runs, with nothing loading or restoring, before its parts are batched (things dropped at the
/// map's start have landed by then; anything that moves later leaves its batch): seconds of game time and frames.
const SETTLE_SECS: f32 = 2.0;
const SETTLE_FRAMES: u32 = 10;

/// Seconds between looks for parts that can join.
const SCAN_SECS: f32 = 2.0;

/// The cell parts are grouped by (metres).
const CELL: f32 = 256.0;

fn enabled() -> bool {
    crate::perf::lean_rendering()
}

/// One mesh drawing many parts, in world space.
#[derive(Component)]
pub struct StaticBatch {
    pub members: Vec<Entity>,
}

/// On a part a batch draws: which batch, and what it was when copied.
#[derive(Component)]
pub struct Batched {
    batch: Entity,
    at: GlobalTransform,
    mesh: AssetId<Mesh>,
    material: AssetId<StandardMaterial>,
}

/// What a batch holds: one material, one cell.
type Key = (AssetId<StandardMaterial>, IVec3);

/// A mesh-actor part as the scan sees it.
type PartItem<'a> = (
    Entity,
    &'a ChildOf,
    &'a Mesh3d,
    &'a MeshMaterial3d<StandardMaterial>,
    &'a GlobalTransform,
    &'a InheritedVisibility,
    Has<RenderLayers>,
    Has<EnvMapped>,
);
type PartFilter = (Without<Batched>, Without<StaticBatch>);

#[derive(Default)]
pub struct BatchState {
    map_gen: u64,
    settled: u32,
    settled_secs: f32,
    /// Seconds to the next scan; None until the map has settled.
    scan_in: Option<f32>,
    /// A member's mesh, to notice it re-lit or re-made.
    by_mesh: HashMap<AssetId<Mesh>, Entity>,
    /// A member's batch (a despawned member can't be asked).
    batch_of: HashMap<Entity, Entity>,
    by_key: HashMap<Key, Entity>,
    key_of: HashMap<Entity, Key>,
    /// Members that belong to animated actors, by actor: they leave when the actor plays something.
    animated: HashMap<Entity, Vec<Entity>>,
    /// Parts that left a batch, when they may join again (game seconds on `clock`) and how many times they left.
    out: HashMap<Entity, (f32, u32)>,
    clock: f32,
    left: u32,
    rebuilt: u32,
    /// Scans logged so far (the first three are).
    logged: u32,
}

static LEFT: AtomicU32 = AtomicU32::new(0);
static REBUILT: AtomicU32 = AtomicU32::new(0);

/// For the PROF line: parts that left their batch and batches rebuilt, since this map was batched.
pub fn churn() -> (u32, u32) {
    (LEFT.load(Ordering::Relaxed), REBUILT.load(Ordering::Relaxed))
}

/// The mesh layout `actors::part_mesh` builds: positions, normals, UVs and colours, indexed triangles.
fn mergeable(m: &Mesh) -> bool {
    let Ok(attrs) = m.try_attributes() else { return false };
    m.primitive_topology() == PrimitiveTopology::TriangleList
        && attrs.count() == 4
        && matches!(m.try_attribute(Mesh::ATTRIBUTE_POSITION), Ok(VertexAttributeValues::Float32x3(_)))
        && matches!(m.try_attribute(Mesh::ATTRIBUTE_NORMAL), Ok(VertexAttributeValues::Float32x3(_)))
        && matches!(m.try_attribute(Mesh::ATTRIBUTE_UV_0), Ok(VertexAttributeValues::Float32x2(_)))
        && matches!(m.try_attribute(Mesh::ATTRIBUTE_COLOR), Ok(VertexAttributeValues::Float32x4(_)))
        && m.indices().is_some()
}

/// The members' meshes in world space, as one mesh, and its bounds.
fn merge(meshes: &Assets<Mesh>, members: &[(AssetId<Mesh>, GlobalTransform)]) -> Option<(Mesh, Aabb)> {
    let (mut pos, mut nrm, mut uv, mut col, mut idx) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (id, gt) in members {
        let m = meshes.get(*id)?;
        let (
            Ok(VertexAttributeValues::Float32x3(p)),
            Ok(VertexAttributeValues::Float32x3(n)),
            Ok(VertexAttributeValues::Float32x2(u)),
            Ok(VertexAttributeValues::Float32x4(c)),
        ) = (
            m.try_attribute(Mesh::ATTRIBUTE_POSITION),
            m.try_attribute(Mesh::ATTRIBUTE_NORMAL),
            m.try_attribute(Mesh::ATTRIBUTE_UV_0),
            m.try_attribute(Mesh::ATTRIBUTE_COLOR),
        )
        else {
            return None;
        };
        let base = pos.len() as u32;
        let a = gt.affine();
        let normal_matrix = a.matrix3.inverse().transpose();
        pos.extend(p.iter().map(|v| a.transform_point3(Vec3::from(*v)).to_array()));
        nrm.extend(n.iter().map(|v| Vec3::from(normal_matrix * Vec3A::from(*v)).normalize_or_zero().to_array()));
        uv.extend_from_slice(u);
        col.extend_from_slice(c);
        match m.indices()? {
            Indices::U32(i) => idx.extend(i.iter().map(|x| x + base)),
            Indices::U16(i) => idx.extend(i.iter().map(|x| u32::from(*x) + base)),
        }
    }
    if pos.is_empty() {
        return None;
    }
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for p in &pos {
        lo = lo.min(Vec3::from(*p));
        hi = hi.max(Vec3::from(*p));
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
    mesh.insert_indices(Indices::U32(idx));
    Some((mesh, Aabb::from_min_max(lo, hi)))
}

fn same_place(a: &GlobalTransform, b: &GlobalTransform) -> bool {
    let (a, b) = (a.affine(), b.affine());
    a.translation.distance_squared(b.translation) < 1.0e-8
        && a.matrix3.x_axis.distance_squared(b.matrix3.x_axis) < 1.0e-8
        && a.matrix3.y_axis.distance_squared(b.matrix3.y_axis) < 1.0e-8
        && a.matrix3.z_axis.distance_squared(b.matrix3.z_axis) < 1.0e-8
}

/// An animated actor holds still: its idle loop is a single frame it has been drawn in, and nothing else plays.
fn holds_still(a: &Animated) -> bool {
    a.clip.is_none() && a.tween.is_none() && !a.face.talking && matches!(a.shown, Some((_, frames, _)) if frames <= 1)
}

/// Builds the batches once a map has settled and adds to them every two seconds, takes out parts that change, and
/// undoes it all when Lean Rendering is switched off.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn batch_system(
    mut commands: Commands,
    mut local: Local<BatchState>,
    time: Res<Time>,
    parts: Query<PartItem, PartFilter>,
    actors: Query<Option<&Animated>, With<MeshActorRef>>,
    changed: Query<
        (Entity, &Batched, &GlobalTransform, &Mesh3d, &MeshMaterial3d<StandardMaterial>, &InheritedVisibility),
        Or<(
            Changed<GlobalTransform>,
            Changed<Mesh3d>,
            Changed<MeshMaterial3d<StandardMaterial>>,
            Changed<InheritedVisibility>,
        )>,
    >,
    members: Query<&Batched>,
    mut batches: Query<(Entity, &mut StaticBatch, &Mesh3d, &mut Aabb)>,
    mut mesh_events: MessageReader<AssetEvent<Mesh>>,
    mut gone: RemovedComponents<Batched>,
    mut unmaterialed: RemovedComponents<MeshMaterial3d<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    materials: Res<Assets<StandardMaterial>>,
) {
    let st = &mut *local;
    if crate::loader::new_map(&mut st.map_gen) {
        // The old map's batches and parts went with it (loader.rs despawns every root).
        let map_gen = st.map_gen;
        *st = BatchState { map_gen, ..default() };
        LEFT.store(0, Ordering::Relaxed);
        REBUILT.store(0, Ordering::Relaxed);
    }
    if !enabled() {
        if st.scan_in.is_some() {
            // Lean Rendering switched off: every part draws itself again.
            for (b, batch, _, _) in &batches {
                for m in &batch.members {
                    if let Ok(mut ec) = commands.get_entity(*m) {
                        ec.remove::<(Batched, RenderLayers)>();
                    }
                }
                commands.entity(b).despawn();
            }
            let map_gen = st.map_gen;
            *st = BatchState { map_gen, ..default() };
        }
        st.settled = 0;
        st.settled_secs = 0.0;
        mesh_events.clear();
        gone.clear();
        unmaterialed.clear();
        return;
    }
    let Some(scan_in) = st.scan_in else {
        mesh_events.clear();
        gone.clear();
        unmaterialed.clear();
        if crate::loader::busy() || crate::save::restoring() {
            st.settled = 0;
            st.settled_secs = 0.0;
            return;
        }
        st.settled += 1;
        st.settled_secs += time.delta_secs();
        if st.settled >= SETTLE_FRAMES && st.settled_secs >= SETTLE_SECS {
            st.scan_in = Some(0.0);
        }
        return;
    };

    st.clock += time.delta_secs();

    // Parts that changed leave their batch.
    let mut leaving: HashSet<Entity> = HashSet::new();
    for (e, b, gt, mesh, mat, vis) in &changed {
        if !vis.get() || mesh.0.id() != b.mesh || mat.0.id() != b.material || !same_place(gt, &b.at) {
            leaving.insert(e);
        }
    }
    for ev in mesh_events.read() {
        if let AssetEvent::Modified { id } | AssetEvent::Removed { id } = ev {
            if let Some(&e) = st.by_mesh.get(id) {
                leaving.insert(e);
            }
        }
    }
    for e in unmaterialed.read() {
        if st.batch_of.contains_key(&e) {
            leaving.insert(e);
        }
    }
    for (actor, list) in &st.animated {
        // (Batched, all its parts out of every view, the actor isn't posed: its clocks run, so a sequence that
        // starts shows up as a clip.)
        if actors.get(*actor).ok().flatten().is_none_or(|a| a.clip.is_some() || a.tween.is_some() || a.face.talking) {
            leaving.extend(list.iter().copied());
        }
    }
    let mut dirty: HashSet<Entity> = HashSet::new();
    for e in gone.read() {
        // Despawned (or its marker taken off by someone else): its batch must stop drawing it.
        if let Some(b) = st.batch_of.remove(&e) {
            dirty.insert(b);
            st.by_mesh.retain(|_, m| *m != e);
            st.left += 1;
        }
    }
    for e in leaving {
        let Ok(b) = members.get(e) else { continue };
        if st.batch_of.remove(&e).is_none() {
            continue;
        }
        dirty.insert(b.batch);
        st.by_mesh.remove(&b.mesh);
        let times = st.out.get(&e).map_or(0, |x| x.1) + 1;
        st.out.insert(e, (st.clock + 10.0 * 2f32.powi(times.min(4) as i32 - 1), times));
        st.left += 1;
        commands.entity(e).remove::<(Batched, RenderLayers)>();
    }
    for b in dirty {
        let Ok((_, mut batch, mesh, mut aabb)) = batches.get_mut(b) else { continue };
        batch.members.retain(|m| st.batch_of.get(m) == Some(&b));
        let keep: Vec<(AssetId<Mesh>, GlobalTransform)> =
            batch.members.iter().filter_map(|m| members.get(*m).ok().map(|x| (x.mesh, x.at))).collect();
        match (keep.len() >= 2).then(|| merge(&meshes, &keep)).flatten() {
            Some((m, bounds)) => {
                let _ = meshes.insert(mesh.0.id(), m);
                *aabb = bounds;
                st.rebuilt += 1;
            }
            None => {
                // Too few left to be worth a batch: they draw themselves again (and may join another later).
                for m in batch.members.drain(..) {
                    st.batch_of.remove(&m);
                    if let Ok(x) = members.get(m) {
                        st.by_mesh.remove(&x.mesh);
                    }
                    if let Ok(mut ec) = commands.get_entity(m) {
                        ec.remove::<(Batched, RenderLayers)>();
                    }
                }
                if let Some(k) = st.key_of.remove(&b) {
                    st.by_key.remove(&k);
                }
                commands.entity(b).despawn();
            }
        }
    }
    let batch_of = &st.batch_of;
    let mut animated = std::mem::take(&mut st.animated);
    animated.retain(|_, list| {
        list.retain(|p| batch_of.contains_key(p));
        !list.is_empty()
    });
    st.animated = animated;

    // Every two seconds: parts that can join.
    let left = scan_in - time.delta_secs();
    if left > 0.0 {
        st.scan_in = Some(left);
    } else {
        st.scan_in = Some(SCAN_SECS);
        scan(&mut commands, st, &parts, &actors, &members, &mut batches, &mut meshes, &materials);
    }
    LEFT.store(st.left, Ordering::Relaxed);
    REBUILT.store(st.rebuilt, Ordering::Relaxed);
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn scan(
    commands: &mut Commands,
    st: &mut BatchState,
    parts: &Query<PartItem, PartFilter>,
    actors: &Query<Option<&Animated>, With<MeshActorRef>>,
    members: &Query<&Batched>,
    batches: &mut Query<(Entity, &mut StaticBatch, &Mesh3d, &mut Aabb)>,
    meshes: &mut Assets<Mesh>,
    materials: &Assets<StandardMaterial>,
) {
    let t0 = std::time::Instant::now();
    // (part, mesh, where, its actor if animated)
    type Joiner = (Entity, AssetId<Mesh>, GlobalTransform, Option<Entity>);
    let mut groups: HashMap<Key, (Handle<StandardMaterial>, Vec<Joiner>)> = HashMap::new();
    // Why parts stay out, for the log lines of the first scans.
    let (mut hidden, mut layered, mut see_through, mut other) = (0, 0, 0, 0);
    let (mut unseen, mut looping, mut playing) = (0, 0, 0);
    for (e, parent, mesh, mat, gt, vis, has_layers, env) in parts {
        if st.out.get(&e).is_some_and(|x| x.0 > st.clock) {
            continue;
        }
        let Ok(anim) = actors.get(parent.parent()) else { continue };
        if let Some(a) = anim.filter(|a| !holds_still(a)) {
            if a.clip.is_some() || a.tween.is_some() || a.face.talking {
                playing += 1;
            } else if a.shown.is_none() {
                unseen += 1;
            } else {
                looping += 1;
            }
            continue;
        }
        if !vis.get() {
            hidden += 1;
            continue;
        }
        if has_layers || env {
            // Layers of their own (shadow casting, vision) or a view-dependent environment map.
            layered += 1;
            continue;
        }
        let Some(m) = materials.get(&mat.0) else { continue };
        if !matches!(m.alpha_mode, AlphaMode::Opaque | AlphaMode::Mask(_)) {
            see_through += 1;
            continue;
        }
        let Some(aabb) = meshes.get(&mesh.0).filter(|mm| mergeable(mm)).and_then(|mm| mm.compute_aabb()) else {
            other += 1;
            continue;
        };
        let center = gt.transform_point(Vec3::from(aabb.center));
        let key = (mat.0.id(), (center / CELL).floor().as_ivec3());
        let actor = anim.is_some().then(|| parent.parent());
        groups.entry(key).or_insert_with(|| (mat.0.clone(), Vec::new())).1.push((e, mesh.0.id(), *gt, actor));
    }
    let (mut joined, mut alone) = (0usize, 0usize);
    for (key, (handle, list)) in groups {
        let existing = st.by_key.get(&key).copied().filter(|b| batches.contains(*b));
        let batch = if let Some(b) = existing {
            // More parts for a batch this cell already has: it is rebuilt with them.
            let Ok((_, mut batch, mesh, mut aabb)) = batches.get_mut(b) else { continue };
            let mut keep: Vec<(AssetId<Mesh>, GlobalTransform)> =
                batch.members.iter().filter_map(|m| members.get(*m).ok().map(|x| (x.mesh, x.at))).collect();
            keep.extend(list.iter().map(|x| (x.1, x.2)));
            let Some((m, bounds)) = merge(meshes, &keep) else { continue };
            let _ = meshes.insert(mesh.0.id(), m);
            *aabb = bounds;
            batch.members.extend(list.iter().map(|x| x.0));
            st.rebuilt += 1;
            b
        } else if list.len() >= 2 {
            let keep: Vec<(AssetId<Mesh>, GlobalTransform)> = list.iter().map(|x| (x.1, x.2)).collect();
            let Some((mesh, bounds)) = merge(meshes, &keep) else { continue };
            let b = commands
                .spawn((
                    Mesh3d(meshes.add(mesh)),
                    MeshMaterial3d(handle),
                    Transform::IDENTITY,
                    Visibility::default(),
                    bounds,
                    NoAutoAabb,
                    StaticBatch { members: list.iter().map(|x| x.0).collect() },
                ))
                .id();
            st.by_key.insert(key, b);
            st.key_of.insert(b, key);
            b
        } else {
            alone += list.len();
            continue;
        };
        for (e, id, gt, actor) in &list {
            commands.entity(*e).insert((
                Batched { batch, at: *gt, mesh: *id, material: key.0 },
                RenderLayers::layer(AWAY),
            ));
            st.by_mesh.insert(*id, *e);
            st.batch_of.insert(*e, batch);
            if let Some(a) = actor {
                st.animated.entry(*a).or_default().push(*e);
            }
        }
        joined += list.len();
    }
    if st.logged < 3 {
        st.logged += 1;
        println!(
            "[Batches] scan {}: {joined} still mesh-actor parts joined, {} parts in {} batches ({CELL} m cells), {:.1} ms; left out: {alone} alone in their cell, {hidden} hidden, {unseen} animated actors' not drawn yet, {looping} looping, {playing} playing, {layered} on layers of their own or environment-mapped, {see_through} see-through, {other} of another mesh layout",
            st.logged,
            st.batch_of.len(),
            st.by_key.len(),
            t0.elapsed().as_secs_f32() * 1000.0
        );
    }
}
