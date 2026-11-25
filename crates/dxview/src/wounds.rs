//! Wounds On Bodies (setting BodyWounds, with Skeletal Characters): wounds drawn on the skin itself.
//!
//! A shot is followed into the body's skinned triangles as they stand this frame (the sections are skinned on the
//! CPU, once per hit): where the ray meets the skin is the entry wound, where it leaves on the far side the exit
//! wound. Each is kept in the body's rest space (the point on the triangle it hit, read back in the vertices' own
//! rest positions) and handed to the body's material, whose shader draws it on the skin (shaders/actor_lit.wgsl):
//! a dark opening, torn flesh, a stain soaking out, and blood running down from it in 1-3 rivulets that lengthen
//! for a few seconds, all glossy while wet. Since it lives in rest space it stays exactly on the skin in every pose,
//! ragdoll or not - nothing floats.
//!
//! Burns char the skin (embers for a moment), blows bruise it. Wounds keep bleeding drops that fall and splat
//! (Fluid Blood, gore.rs), from the point on the skin they were made at; the dead bleed harder for a while.
//! Arterial spurts (dismember.rs: stumps, a big wound in the neck) pulse with a slowing heartbeat.

use bevy::prelude::*;
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use glam::DVec3;
use std::collections::HashMap;
use std::sync::Mutex;

use crate::skeletal::{ActorMaterial, SkelBody, WOUNDS};

pub fn on() -> bool {
    setting!("BodyWounds") > 0.5 && crate::skeletal::enabled() && !crate::settings::low_gore()
}

/// What a wound looks like (shaders/actor_lit.wgsl reads the number).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Entry = 0,
    Exit = 1,
    Burn = 2,
    Blast = 3,
    Stump = 4,
    Bruise = 5,
}

struct Req {
    blocker: usize,
    at: DVec3,
    dir: DVec3,
    damage_type: &'static str,
    damage: f32,
}

static REQS: Mutex<Vec<Req>> = Mutex::new(Vec::new());

/// A hit on pawn `blocker` at `at` (Unreal space, on its collision cylinder) travelling along `dir`.
pub fn hit(blocker: usize, at: DVec3, dir: DVec3, damage_type: &'static str, damage: f32) {
    if let Ok(mut g) = REQS.lock() {
        g.push(Req { blocker, at, dir, damage_type, damage });
    }
}

/// Damage taken lately by each body (blocker -> (sum, game time of the last)): a shotgun's pellets add up
/// (dismember.rs reads it).
static RECENT: Mutex<Option<HashMap<usize, (f32, f32)>>> = Mutex::new(None);

/// The damage `blocker` took in the last fifth of a second.
pub fn recent_damage(blocker: usize, now: f32) -> f32 {
    RECENT
        .lock()
        .ok()
        .and_then(|g| g.as_ref().and_then(|m| m.get(&blocker).copied()))
        .map_or(0.0, |(d, t)| if now - t < 0.2 { d } else { 0.0 })
}

/// Where each body was last hit: the rig bone under the skin there and the shot (dismember.rs).
static LAST: Mutex<Option<HashMap<usize, LastHit>>> = Mutex::new(None);

#[derive(Clone)]
pub struct LastHit {
    pub bone: String,
    pub time: f32,
}

pub fn last_hit(blocker: usize) -> Option<LastHit> {
    LAST.lock().ok().and_then(|g| g.as_ref().and_then(|m| m.get(&blocker).cloned()))
}

pub fn reset_level() {
    if let Ok(mut g) = RECENT.lock() {
        *g = None;
    }
    if let Ok(mut g) = LAST.lock() {
        *g = None;
    }
}

// ------------------------------------------------------------------------------------------------ skin snapshot

/// One skinned section as it stands this frame.
pub(crate) struct SecSnap {
    pub mat: Handle<ActorMaterial>,
    pub world: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub rest: Vec<[f32; 4]>,
    pub uvs: Vec<[f32; 2]>,
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
    pub indices: Vec<u16>,
    /// The section's joint entities (SkinnedMesh.joints).
    pub joint_ents: Vec<Entity>,
}

pub(crate) type PartQuery<'w, 's> = Query<'w, 's, (&'static Mesh3d, &'static SkinnedMesh, &'static MeshMaterial3d<ActorMaterial>)>;

/// The body's solid sections skinned as they stand (Bevy world space).
pub(crate) fn snapshot(
    s: &SkelBody,
    meshes: &Assets<Mesh>,
    ibms: &Assets<SkinnedMeshInverseBindposes>,
    mats: &Assets<ActorMaterial>,
    parts: &PartQuery,
    gts: &Query<&GlobalTransform>,
) -> Vec<SecSnap> {
    let mut out = Vec::new();
    for &me in s.mesh_entities() {
        let Ok((m3, skin, mm)) = parts.get(me) else { continue };
        if !mats.get(&mm.0).is_some_and(|m| matches!(m.base.alpha_mode, AlphaMode::Opaque | AlphaMode::Mask(_))) {
            continue;
        }
        let Some(mesh) = meshes.get(&m3.0) else { continue };
        let Some(ibm) = ibms.get(&skin.inverse_bindposes) else { continue };
        use bevy::mesh::VertexAttributeValues as V;
        let (Some(V::Float32x3(pos)), Some(V::Float32x3(nrm)), Some(V::Uint16x4(jn)), Some(V::Float32x4(wt))) = (
            mesh.attribute(Mesh::ATTRIBUTE_POSITION),
            mesh.attribute(Mesh::ATTRIBUTE_NORMAL),
            mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX),
            mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT),
        ) else {
            continue;
        };
        let uvs = match mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
            Some(V::Float32x2(u)) => u.clone(),
            _ => vec![[0.0; 2]; pos.len()],
        };
        let rest = match mesh.attribute(Mesh::ATTRIBUTE_COLOR) {
            Some(V::Float32x4(c)) => c.clone(),
            _ => pos.iter().map(|p| [p[0], p[1], p[2], 1.0]).collect(),
        };
        let indices: Vec<u16> = match mesh.indices() {
            Some(bevy::mesh::Indices::U16(i)) => i.clone(),
            Some(bevy::mesh::Indices::U32(i)) => i.iter().map(|&x| x as u16).collect(),
            None => continue,
        };
        let skin_m: Vec<Mat4> = skin
            .joints
            .iter()
            .enumerate()
            .map(|(j, e)| gts.get(*e).map_or(Mat4::IDENTITY, |g| g.to_matrix()) * ibm.get(j).copied().unwrap_or(Mat4::IDENTITY))
            .collect();
        let mut world = Vec::with_capacity(pos.len());
        let mut normals = Vec::with_capacity(pos.len());
        for v in 0..pos.len() {
            let mut m = Mat4::ZERO;
            for k in 0..4 {
                let w = wt[v][k];
                if w > 0.0 {
                    if let Some(sm) = skin_m.get(jn[v][k] as usize) {
                        m += *sm * w;
                    }
                }
            }
            world.push(m.transform_point3(Vec3::from(pos[v])));
            normals.push(m.transform_vector3(Vec3::from(nrm[v])).normalize_or(Vec3::Y));
        }
        out.push(SecSnap {
            mat: mm.0.clone(),
            world,
            normals,
            rest,
            uvs,
            joints: jn.clone(),
            weights: wt.clone(),
            indices,
            joint_ents: skin.joints.clone(),
        });
    }
    out
}

/// Where a ray meets the skin: section, triangle (first index), barycentric weights, distance along the ray.
#[derive(Clone, Copy)]
pub(crate) struct SkinHit {
    pub sec: usize,
    pub tri: usize,
    pub bary: Vec3,
    pub t: f32,
}

impl SkinHit {
    pub fn world(&self, snap: &[SecSnap]) -> Vec3 {
        self.lerp(snap, |s, i| s.world[i])
    }
    pub fn rest(&self, snap: &[SecSnap]) -> Vec3 {
        self.lerp(snap, |s, i| Vec3::new(s.rest[i][0], s.rest[i][1], s.rest[i][2]))
    }
    pub fn normal(&self, snap: &[SecSnap]) -> Vec3 {
        let s = &snap[self.sec];
        let [a, b, c] = self.corners(snap);
        (s.world[b] - s.world[a]).cross(s.world[c] - s.world[a]).normalize_or(Vec3::Y)
    }
    fn corners(&self, snap: &[SecSnap]) -> [usize; 3] {
        let s = &snap[self.sec];
        [s.indices[self.tri] as usize, s.indices[self.tri + 1] as usize, s.indices[self.tri + 2] as usize]
    }
    fn lerp(&self, snap: &[SecSnap], f: impl Fn(&SecSnap, usize) -> Vec3) -> Vec3 {
        let s = &snap[self.sec];
        let [a, b, c] = self.corners(snap);
        f(s, a) * self.bary.x + f(s, b) * self.bary.y + f(s, c) * self.bary.z
    }
    /// The joint entity weighing most at the hit (its corner nearest the hit).
    pub fn joint(&self, snap: &[SecSnap]) -> Option<Entity> {
        let s = &snap[self.sec];
        let corners = self.corners(snap);
        let k = if self.bary.x >= self.bary.y && self.bary.x >= self.bary.z { 0 } else if self.bary.y >= self.bary.z { 1 } else { 2 };
        let v = corners[k];
        let (mut best, mut bw) = (0usize, -1.0f32);
        for j in 0..4 {
            if s.weights[v][j] > bw {
                bw = s.weights[v][j];
                best = j;
            }
        }
        s.joint_ents.get(s.joints[v][best] as usize).copied()
    }
}

/// Every place the ray (origin, unit direction, length) crosses the skin, nearest first.
pub(crate) fn cast(snap: &[SecSnap], o: Vec3, d: Vec3, len: f32) -> Vec<SkinHit> {
    let mut hits = Vec::new();
    for (si, s) in snap.iter().enumerate() {
        for t in (0..s.indices.len()).step_by(3) {
            if t + 2 >= s.indices.len() {
                break;
            }
            let (a, b, c) = (s.world[s.indices[t] as usize], s.world[s.indices[t + 1] as usize], s.world[s.indices[t + 2] as usize]);
            let (e1, e2) = (b - a, c - a);
            let p = d.cross(e2);
            let det = e1.dot(p);
            if det.abs() < 1e-9 {
                continue;
            }
            let inv = 1.0 / det;
            let tv = o - a;
            let u = tv.dot(p) * inv;
            if !(0.0..=1.0).contains(&u) {
                continue;
            }
            let q = tv.cross(e1);
            let v = d.dot(q) * inv;
            if v < 0.0 || u + v > 1.0 {
                continue;
            }
            let dist = e2.dot(q) * inv;
            if dist > 0.0 && dist < len {
                hits.push(SkinHit { sec: si, tri: t, bary: Vec3::new(1.0 - u - v, u, v), t: dist });
            }
        }
    }
    hits.sort_by(|x, y| x.t.total_cmp(&y.t));
    hits
}

/// The triangle passing nearest the ray, when the ray misses the skin (a graze, or the collision cylinder caught a
/// shot that passed between the legs): its middle, if within `reach` (Bevy metres).
pub(crate) fn nearest(snap: &[SecSnap], o: Vec3, d: Vec3, reach: f32) -> Option<SkinHit> {
    let mut best: Option<(f32, SkinHit)> = None;
    for (si, s) in snap.iter().enumerate() {
        for t in (0..s.indices.len()).step_by(3) {
            if t + 2 >= s.indices.len() {
                break;
            }
            let c = (s.world[s.indices[t] as usize] + s.world[s.indices[t + 1] as usize] + s.world[s.indices[t + 2] as usize]) / 3.0;
            let along = (c - o).dot(d);
            if along <= 0.0 {
                continue;
            }
            let off = (c - o - d * along).length();
            if off < reach && best.as_ref().is_none_or(|b| off < b.0) {
                best = Some((off, SkinHit { sec: si, tri: t, bary: Vec3::splat(1.0 / 3.0), t: along }));
            }
        }
    }
    best.map(|b| b.1)
}

// ------------------------------------------------------------------------------------------------ wounds kept

#[derive(Clone, Copy)]
pub(crate) struct Rec {
    pub rest: Vec3,
    pub r: f32,
    pub birth: f32,
    pub kind: Kind,
    pub run: f32,
    pub seed: f32,
}

/// The wounds on one body.
#[derive(Component, Default)]
pub struct SkinWounds {
    pub(crate) list: Vec<Rec>,
    /// When the body died (Time::elapsed_secs_wrapped; 0 = alive), and whether the material needs writing.
    dead: f32,
    dirty: bool,
    /// The first material last written (a rebuilt body gets new ones: write again).
    applied: Option<AssetId<ActorMaterial>>,
}

impl SkinWounds {
    pub(crate) fn add(&mut self, r: Rec) {
        if self.list.len() >= WOUNDS {
            // The oldest that isn't a stump gives way.
            if let Some(i) = self.list.iter().position(|w| w.kind != Kind::Stump) {
                self.list.remove(i);
            } else {
                self.list.remove(0);
            }
        }
        self.list.push(r);
        self.dirty = true;
    }
}

/// Write a body's wounds into its sections' materials (only what changed).
pub(crate) fn write_wounds(list: &[Rec], dead: f32, mats_list: &[Handle<ActorMaterial>], mats: &mut Assets<ActorMaterial>) {
    let mut wpos = [Vec4::ZERO; WOUNDS];
    let mut winfo = [Vec4::ZERO; WOUNDS];
    for (i, w) in list.iter().take(WOUNDS).enumerate() {
        wpos[i] = w.rest.extend(w.r);
        winfo[i] = Vec4::new(w.birth, w.kind as u8 as f32, w.run, w.seed);
    }
    let wmisc = Vec4::new(list.len().min(WOUNDS) as f32, dead, 0.0, 0.0);
    for h in mats_list {
        let Some(m) = mats.get(h) else { continue };
        let l = &m.extension.lights;
        if l.wpos == wpos && l.winfo == winfo && l.wmisc == wmisc {
            continue;
        }
        if let Some(mut m) = mats.get_mut(h) {
            m.extension.lights.wpos = wpos;
            m.extension.lights.winfo = winfo;
            m.extension.lights.wmisc = wmisc;
        }
    }
}

/// One wound alone on a material (a severed piece's cut end), the body dead since `dead`.
pub(crate) fn fill_one(l: &mut crate::skeletal::ActorLights, w: &Rec, dead: f32) {
    l.wpos = [Vec4::ZERO; WOUNDS];
    l.winfo = [Vec4::ZERO; WOUNDS];
    l.wpos[0] = w.rest.extend(w.r);
    l.winfo[0] = Vec4::new(w.birth, w.kind as u8 as f32, w.run, w.seed);
    l.wmisc = Vec4::new(1.0, dead.max(0.001), 0.0, 0.0);
}

/// A wound that keeps bleeding (Fluid Blood): drops leave its point on the skin and fall. A child of the bone under
/// it, placed on the skin; `n` = out of the skin (bone space).
#[derive(Component)]
pub struct Bleeder {
    owner: Entity,
    left: f32,
    acc: f32,
    dead_seen: bool,
    n: Vec3,
    rate: f32,
}

/// An artery pumping (dismember.rs stumps, wounds in the neck): pulses of drops with a slowing heartbeat.
#[derive(Component)]
pub struct Spurter {
    pub owner: Entity,
    /// Seconds left, of `total`; time to the next beat; out of the body (bone space); strength 0..1.
    pub left: f32,
    pub total: f32,
    pub beat: f32,
    pub n: Vec3,
    pub strength: f32,
}

fn rnd(seed: &mut u64) -> f32 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    ((*seed >> 40) as f32) / (1u64 << 24) as f32
}

/// The size of a wound in rest units (the bodies' rest space is about a metre to a metre).
fn wound_size(kind: Kind, damage: f32) -> f32 {
    match kind {
        Kind::Entry => (0.010 + damage * 0.0006).clamp(0.010, 0.028),
        Kind::Exit => (0.016 + damage * 0.0011).clamp(0.016, 0.05),
        Kind::Burn => (0.04 + damage * 0.001).clamp(0.045, 0.085),
        Kind::Blast => (0.03 + damage * 0.0008).clamp(0.03, 0.07),
        Kind::Stump => 0.045,
        Kind::Bruise => 0.04,
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn wounds_system(
    mut commands: Commands,
    time: Res<Time>,
    meshes: Res<Assets<Mesh>>,
    ibms: Res<Assets<SkinnedMeshInverseBindposes>>,
    mut mats: ResMut<Assets<ActorMaterial>>,
    mut bodies: Query<(Entity, &crate::ai::Npc, &SkelBody, Option<&mut SkinWounds>)>,
    parts: PartQuery,
    gts: Query<&GlobalTransform>,
    names: Query<&Name>,
    mut seed: Local<u64>,
) {
    let reqs: Vec<Req> = REQS.lock().map(|mut g| std::mem::take(&mut *g)).unwrap_or_default();
    let now = time.elapsed_secs_wrapped();
    let game_now = time.elapsed_secs();
    if *seed == 0 {
        *seed = 0x3011D_u64;
    }
    for r in &reqs {
        if let Ok(mut g) = RECENT.lock() {
            let m = g.get_or_insert_with(HashMap::new);
            let e = m.entry(r.blocker).or_insert((0.0, game_now));
            if game_now - e.1 > 0.2 {
                e.0 = 0.0;
            }
            e.0 += r.damage;
            e.1 = game_now;
        }
    }
    let wounds_on = on();
    let mut new_recs: Vec<(Entity, Rec)> = Vec::new();
    for r in reqs {
        let Some((e, _, s, _)) = bodies.iter().find(|(_, n, ..)| n.blocker == r.blocker) else { continue };
        let kind = match r.damage_type {
            "Shot" | "Sabot" | "Poison" => Kind::Entry,
            "Burned" | "Flamed" => Kind::Burn,
            "Exploded" => Kind::Blast,
            "KnockedOut" => Kind::Bruise,
            _ => continue,
        };
        let snap = snapshot(s, &meshes, &ibms, &mats, &parts, &gts);
        if snap.is_empty() {
            continue;
        }
        let hit = Vec3::from(dx_level::ue_to_bevy(r.at.as_vec3().to_array()));
        let d = Vec3::from(dx_level::ue_to_bevy(r.dir.as_vec3().to_array())).normalize_or(Vec3::NEG_Z);
        // From well outside the collision cylinder, along the shot and on through the body.
        let o = hit - d * 1.5;
        let hits = cast(&snap, o, d, 4.0);
        let entry = hits.first().copied().or_else(|| nearest(&snap, o, d, 0.12));
        let Some(entry) = entry else { continue };
        let world = entry.world(&snap);
        if let (Some(j), Ok(mut g)) = (entry.joint(&snap), LAST.lock()) {
            let bone = names.get(j).map(|n| n.as_str().to_string()).unwrap_or_default();
            g.get_or_insert_with(HashMap::new).insert(r.blocker, LastHit { bone, time: game_now });
        }
        if !wounds_on {
            continue;
        }
        let s_rnd = rnd(&mut seed);
        let run = match kind {
            Kind::Burn | Kind::Bruise => 0.0,
            Kind::Blast => 0.22 + s_rnd * 0.15,
            _ => 0.12 + s_rnd * 0.2 + r.damage * 0.004,
        };
        new_recs.push((e, Rec { rest: entry.rest(&snap), r: wound_size(kind, r.damage), birth: now, kind, run, seed: rnd(&mut seed) * 97.0 }));
        // A shot that goes on through: the exit wound, and what it carries out sprays on (gore.rs back-spatter).
        let mut exit_at = None;
        if kind == Kind::Entry && r.damage >= 18.0 {
            if let Some(x) = hits.iter().rev().find(|h| h.t > entry.t + 0.06).copied() {
                new_recs.push((e, Rec {
                    rest: x.rest(&snap),
                    r: wound_size(Kind::Exit, r.damage),
                    birth: now,
                    kind: Kind::Exit,
                    run: run * 1.4,
                    seed: rnd(&mut seed) * 97.0,
                }));
                exit_at = Some(x.world(&snap));
            }
        }
        if let Some(x) = exit_at {
            let ue = DVec3::from_array(dx_level::bevy_to_ue(x.to_array()).map(|v| v as f64));
            crate::gore::spatter(ue, r.dir.normalize_or(DVec3::X), r.damage);
        }
        // Bleeding from where it went in (Fluid Blood).
        if crate::gore::enabled() && !matches!(kind, Kind::Burn | Kind::Bruise) {
            if let (Some(j), Ok(gj)) = (entry.joint(&snap), entry.joint(&snap).map_or(Err(()), |j| gts.get(j).map_err(|_| ()))) {
                let inv = gj.to_matrix().inverse();
                let n = entry.normal(&snap);
                let n = if n.dot(d) > 0.0 { -n } else { n };
                let local = inv.transform_point3(world + n * 0.003);
                let ln = inv.transform_vector3(n).normalize_or(Vec3::Y);
                let left = if kind == Kind::Blast { 6.0 } else { 8.0 + rnd(&mut seed) * 8.0 };
                commands.spawn((
                    Transform::from_translation(local),
                    Visibility::Hidden,
                    ChildOf(j),
                    Bleeder { owner: e, left, acc: 0.0, dead_seen: false, n: ln, rate: (0.7 + r.damage / 30.0).min(2.0) },
                ));
                // A big hit in the neck opens an artery.
                let bone = names.get(j).map(|n| n.as_str()).unwrap_or("");
                if (bone == "DEF-spine.005" || bone == "DEF-spine.006") && r.damage >= 15.0 {
                    commands.spawn((
                        Transform::from_translation(local),
                        Visibility::Hidden,
                        ChildOf(j),
                        Spurter { owner: e, left: 5.0, total: 5.0, beat: 0.0, n: ln, strength: 0.6 },
                    ));
                }
            }
        }
    }
    // Bodies without wounds yet get the component with theirs (it arrives after this system: written next frame).
    let mut fresh: HashMap<Entity, SkinWounds> = HashMap::new();
    for (e, rec) in new_recs {
        match bodies.get_mut(e) {
            Ok((_, _, _, Some(mut w))) => w.add(rec),
            _ => fresh.entry(e).or_default().add(rec),
        }
    }
    for (e, w) in fresh {
        commands.entity(e).insert(w);
    }
    // Keep each body's materials in step: new wounds, a death (the stains spread faster), a rebuilt body.
    for (_, npc, s, w) in &mut bodies {
        let Some(mut w) = w else { continue };
        if npc.state == crate::ai::NpcState::Dead && w.dead == 0.0 {
            w.dead = now.max(0.001);
            w.dirty = true;
        }
        let first = s.materials().first().map(|h| h.id());
        if w.dirty || first != w.applied {
            w.dirty = false;
            w.applied = first;
            write_wounds(&w.list, w.dead, s.materials(), &mut mats);
        }
    }
}

/// Wounds bleed: drops leave their point on the skin, out and a little down (gore.rs flies them); harder for a
/// while once the heart's stopped and the body lies there. Arteries pump in beats.
pub fn bleed_system(
    mut commands: Commands,
    time: Res<Time>,
    mut bleeders: Query<(Entity, &mut Bleeder, &GlobalTransform)>,
    mut spurters: Query<(Entity, &mut Spurter, &GlobalTransform), Without<Bleeder>>,
    npcs: Query<&crate::ai::Npc>,
    mut seed: Local<u64>,
) {
    if !crate::gore::enabled() {
        return;
    }
    if *seed == 0 {
        *seed = 0xB1EED;
    }
    let dt = time.delta_secs().min(0.1);
    let ue = |p: Vec3| DVec3::from_array(dx_level::bevy_to_ue(p.to_array()).map(|v| v as f64));
    for (ent, mut b, g) in &mut bleeders {
        if b.left <= 0.0 {
            commands.entity(ent).despawn();
            continue;
        }
        let dead = npcs.get(b.owner).is_ok_and(|n| n.state == crate::ai::NpcState::Dead);
        if dead && !b.dead_seen {
            b.dead_seen = true;
            b.left = b.left.max(16.0);
        }
        b.left -= dt;
        let rate = if dead { 3.5 } else { 2.0 } * b.rate * (b.left / 12.0).clamp(0.25, 1.0);
        b.acc += rate * dt;
        while b.acc >= 1.0 {
            b.acc -= 1.0;
            let at = ue(g.translation());
            let n = g.affine().transform_vector3(b.n).normalize_or(Vec3::Y);
            let out = (ue(g.translation() + n) - at).normalize_or(DVec3::Z);
            let vel = out * (12.0 + rnd(&mut seed) as f64 * 24.0) + DVec3::new(rnd(&mut seed) as f64 - 0.5, rnd(&mut seed) as f64 - 0.5, 0.0) * 12.0;
            crate::gore::drip(at + out, vel, 1.6 + rnd(&mut seed) * 2.4);
        }
    }
    for (ent, mut sp, g) in &mut spurters {
        if sp.left <= 0.0 || npcs.get(sp.owner).is_err() {
            commands.entity(ent).despawn();
            continue;
        }
        sp.left -= dt;
        sp.beat -= dt;
        let fading = (sp.left / sp.total).clamp(0.0, 1.0);
        let at = ue(g.translation());
        let n = g.affine().transform_vector3(sp.n).normalize_or(Vec3::Y);
        let out = (ue(g.translation() + n) - at).normalize_or(DVec3::Z);
        if sp.beat <= 0.0 {
            // The heart slows as the blood runs out: beats further apart and weaker.
            sp.beat = 0.38 + (1.0 - fading) * 0.5;
            let k = (4.0 + 6.0 * fading * sp.strength) as usize;
            for _ in 0..k {
                let jit = DVec3::new(rnd(&mut seed) as f64 - 0.5, rnd(&mut seed) as f64 - 0.5, rnd(&mut seed) as f64 - 0.5) * 0.35;
                let speed = (90.0 + 260.0 * fading as f64 * sp.strength as f64) * (0.7 + rnd(&mut seed) as f64 * 0.6);
                crate::gore::drip(at + out, (out + jit).normalize_or(out) * speed, 2.0 + rnd(&mut seed) * 3.0);
            }
            crate::gore::mist(at + out * 2.0, out * 40.0 * fading as f64, 0.5 + fading * 0.5);
        } else if rnd(&mut seed) < dt * 6.0 {
            // Between beats it still runs.
            crate::gore::drip(at + out, out * 20.0, 1.8);
        }
    }
}
