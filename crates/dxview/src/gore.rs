//! Fluid blood (enhancement "Fluid Blood"): blood that behaves like a liquid.
//! - Hits throw droplets out along the shot (and a little back); each flies under gravity, is traced against the
//!   level, and leaves a splat where it lands, sized by the drop.
//! - On walls and other steep faces a splat runs: it lengthens downward over a couple of seconds, as a trickle does.
//! - The dead bleed out: a pool spreads under the body (where a ragdoll came to rest) and slows as it grows.
//! - Explosions that blow a body apart throw a burst of droplets, big splats on everything near, and a wide pool.
//! Droplets are drawn as small camera-facing streaks in one mesh; splats, runs and pools are the original's blood
//! decals (FlatFXTex2/3/5/6, modulated). All of it shares one cap (more with "Casings And Blood Stay").

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use dx_level::ue_to_bevy;
use glam::DVec3;
use std::sync::Mutex;

use crate::weapons::RuntimeAssets;
use crate::LevelRes;

pub fn enabled() -> bool {
    crate::settings::get("FluidBlood") > 0.5
}

enum Req {
    /// A hit: where, the shot's direction, how much.
    Spray(DVec3, DVec3, f32),
    /// A body blown apart: centre, radius, height.
    Burst(DVec3, f64, f64),
}

static REQS: Mutex<Vec<Req>> = Mutex::new(Vec::new());

/// Blood thrown by a hit (Unreal space; `dir` along the shot).
pub fn spray(at: DVec3, dir: DVec3, amount: f32) {
    if enabled() {
        if let Ok(mut g) = REQS.lock() {
            g.push(Req::Spray(at, dir, amount));
        }
    }
}

/// A body blown apart.
pub fn burst(at: DVec3, r: f64, h: f64) {
    if enabled() {
        if let Ok(mut g) = REQS.lock() {
            g.push(Req::Burst(at, r, h));
        }
    }
}

/// Where a ragdoll came to rest (ragdoll.rs), by pawn blocker: the pool spreads there.
static RESTED: Mutex<Vec<(usize, DVec3)>> = Mutex::new(Vec::new());

pub fn rested(blocker: usize, at: DVec3) {
    if let Ok(mut g) = RESTED.lock() {
        g.push((blocker, at));
    }
}

const GRAVITY: f64 = 950.0;
const MAX_DROPS: usize = 400;
const SPLATS: [&str; 4] = [
    "DeusExItems.Skins.FlatFXTex2",
    "DeusExItems.Skins.FlatFXTex3",
    "DeusExItems.Skins.FlatFXTex5",
    "DeusExItems.Skins.FlatFXTex6",
];

struct Drop {
    pos: DVec3,
    vel: DVec3,
    /// Size (Unreal units across the splat it leaves).
    size: f32,
    life: f32,
}

/// A splat on a steep face, running down.
#[derive(Component)]
pub struct Run {
    t: f32,
    len: f32,
    base: Vec3,
    down: Vec3,
    width: f32,
}

/// A pool spreading under a body.
#[derive(Component)]
pub struct Pool {
    t: f32,
    max: f32,
}

#[derive(Default)]
pub struct GoreState {
    drops: Vec<Drop>,
    mesh: Option<Handle<Mesh>>,
    mat: Option<Handle<StandardMaterial>>,
    seed: u64,
    decals: std::collections::VecDeque<Entity>,
    /// Dead pawns seen (blocker) and how long ago, until their pool starts.
    dead: std::collections::HashMap<usize, f32>,
    pooled: std::collections::HashSet<usize>,
    drawn: bool,
}

fn rnd(seed: &mut u64) -> f64 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    ((*seed >> 33) as f64) / (1u64 << 31) as f64
}

fn vrand(seed: &mut u64) -> DVec3 {
    DVec3::new(rnd(seed) * 2.0 - 1.0, rnd(seed) * 2.0 - 1.0, rnd(seed) * 2.0 - 1.0).normalize_or(DVec3::Z)
}

fn bv(p: DVec3) -> Vec3 {
    Vec3::from(ue_to_bevy(p.as_vec3().to_array()))
}

#[allow(clippy::too_many_arguments)]
fn decal(
    commands: &mut Commands,
    assets: &mut RuntimeAssets,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
    meshes: &mut Assets<Mesh>,
    st: &mut GoreState,
    at: DVec3,
    n: DVec3,
    size: f32,
    extra: Option<impl Bundle>,
) -> Option<Entity> {
    let tex = SPLATS[((rnd(&mut st.seed) * 4.0) as usize).min(3)];
    let img = crate::weapons::modulated_image_for(assets, images, tex)?;
    let normal = bv(n).normalize_or(Vec3::Y);
    let rot = Quat::from_rotation_arc(Vec3::Z, normal) * Quat::from_rotation_z(rnd(&mut st.seed) as f32 * std::f32::consts::TAU);
    let mut ec = commands.spawn((
        Mesh3d(meshes.add(Rectangle::new(1.0, 1.0))),
        MeshMaterial3d(materials.add(crate::impacts::decal_material(img, false))),
        Transform::from_translation(bv(at + n * 0.3)).with_rotation(rot).with_scale(Vec3::splat(size * dx_level::METERS_PER_UNIT)),
        bevy::light::NotShadowCaster,
        crate::impacts::KeepDecal { tex: tex.to_string(), hole: false },
    ));
    if let Some(x) = extra {
        ec.insert(x);
    }
    let e = ec.id();
    st.decals.push_back(e);
    let cap = if crate::settings::get("PersistentDebris") > 0.5 { 3000 } else { 400 };
    while st.decals.len() > cap {
        if let Some(old) = st.decals.pop_front() {
            commands.entity(old).despawn();
        }
    }
    Some(e)
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn gore_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    cam: Query<&GlobalTransform, (With<crate::FlyCam>, Without<crate::SkyCam>)>,
    (mut assets, mut images, mut meshes, mut materials): (
        ResMut<RuntimeAssets>,
        ResMut<Assets<Image>>,
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
    ),
    npcs: Query<(&crate::ai::Npc, &crate::actors::Deco)>,
    mut runs: Query<(&mut Run, &mut Transform), Without<Pool>>,
    mut pools: Query<(&mut Pool, &mut Transform), Without<Run>>,
    mut st: Local<GoreState>,
    mut map_gen: Local<u64>,
) {
    if crate::loader::new_map(&mut map_gen) {
        let seed = st.seed;
        *st = GoreState { seed: seed ^ 0xB10D, ..default() };
    }
    if st.seed == 0 {
        st.seed = 0x6012_E000;
    }
    let reqs: Vec<Req> = REQS.lock().map(|mut g| std::mem::take(&mut *g)).unwrap_or_default();
    let rested: Vec<(usize, DVec3)> = RESTED.lock().map(|mut g| std::mem::take(&mut *g)).unwrap_or_default();
    if !enabled() {
        st.drops.clear();
        return;
    }
    let _prof = crate::prof::scope("gore::gore_system");
    let dt = time.delta_secs().min(0.05);
    let lvl = &level.0;
    let st = &mut *st;
    // New blood.
    for r in reqs {
        match r {
            Req::Spray(at, dir, amount) => {
                let dir = dir.normalize_or(DVec3::Z);
                let n = (4.0 + amount * 0.25).min(14.0) as usize;
                for i in 0..n {
                    // Most go on along the shot (the exit spray), some back toward the shooter.
                    let back = i % 4 == 3;
                    let base = if back { -dir * 0.6 } else { dir };
                    let v = (base + vrand(&mut st.seed) * 0.45).normalize_or(dir) * (120.0 + rnd(&mut st.seed) * 260.0);
                    let size = 3.0 + rnd(&mut st.seed) as f32 * 7.0;
                    st.drops.push(Drop { pos: at, vel: v, size, life: 3.0 });
                }
            }
            Req::Burst(at, r, h) => {
                for _ in 0..70 {
                    let v = (vrand(&mut st.seed) + DVec3::Z * 0.5).normalize_or(DVec3::Z) * (150.0 + rnd(&mut st.seed) * 450.0);
                    let off = DVec3::new((rnd(&mut st.seed) - 0.5) * r, (rnd(&mut st.seed) - 0.5) * r, (rnd(&mut st.seed) - 0.5) * h);
                    let size = 6.0 + rnd(&mut st.seed) as f32 * 16.0;
                    st.drops.push(Drop { pos: at + off, vel: v, size, life: 3.0 });
                }
                // A wide pool where it happened.
                let down = lvl.collision.trace(at, at - DVec3::Z * (h + 64.0), DVec3::ZERO);
                if down.fraction < 1.0 {
                    let p = at + (DVec3::NEG_Z * (h + 64.0)) * down.fraction;
                    let extra = Pool { t: 0.0, max: 150.0 };
                    let _ = decal(&mut commands, &mut assets, &mut images, &mut materials, &mut meshes, st, p, down.normal, 1.0, Some(extra));
                }
            }
        }
    }
    if st.drops.len() > MAX_DROPS {
        let k = st.drops.len() - MAX_DROPS;
        st.drops.drain(..k);
    }
    // Fly, and land.
    let mut landed: Vec<(DVec3, DVec3, f32)> = Vec::new();
    st.drops.retain_mut(|d| {
        d.life -= dt;
        let from = d.pos;
        d.vel.z -= GRAVITY * dt as f64;
        let to = from + d.vel * dt as f64;
        let h = lvl.collision.trace(from, to, DVec3::ZERO);
        if h.fraction < 1.0 {
            landed.push((from + (to - from) * h.fraction, h.normal, d.size));
            return false;
        }
        d.pos = to;
        d.life > 0.0
    });
    for (p, n, size) in landed {
        let steep = n.z.abs() < 0.6;
        if steep && size > 4.0 && rnd(&mut st.seed) < 0.6 {
            // A run down the wall: a splat that lengthens downward.
            let nb = bv(n).normalize_or(Vec3::X);
            let down = (Vec3::NEG_Y - nb * Vec3::NEG_Y.dot(nb)).normalize_or(Vec3::NEG_Y);
            let base = bv(p + n * 0.3);
            let width = size * 0.45 * dx_level::METERS_PER_UNIT;
            let run = Run { t: 0.0, len: (20.0 + rnd(&mut st.seed) as f32 * 50.0) * dx_level::METERS_PER_UNIT, base, down, width };
            if let Some(e) = decal(&mut commands, &mut assets, &mut images, &mut materials, &mut meshes, st, p, n, size, Some(run)) {
                // Oriented with the quad's Y along the run.
                let rot = Quat::from_mat3(&Mat3::from_cols(down.cross(nb).normalize_or(Vec3::X), -down, nb));
                commands.entity(e).insert(Transform::from_translation(base).with_rotation(rot).with_scale(Vec3::new(width, width, 1.0)));
            }
        } else {
            let _ = decal(&mut commands, &mut assets, &mut images, &mut materials, &mut meshes, st, p, n, size, None::<()>);
        }
    }
    // Runs lengthen, slowing as they go.
    for (mut r, mut t) in &mut runs {
        if r.t > 3.0 {
            continue;
        }
        r.t += dt;
        let k = 1.0 - (-r.t * 1.2).exp();
        let len = r.width + r.len * k;
        t.translation = r.base + r.down * (len * 0.5 - r.width * 0.5);
        t.scale = Vec3::new(r.width, len, 1.0);
    }
    // The dead bleed out: a pool spreads under each killed body (not robots, not the knocked out), where its
    // ragdoll came to rest or where it lies.
    let mut rest_at: std::collections::HashMap<usize, DVec3> = rested.into_iter().collect();
    for (n, d) in &npcs {
        if n.state != crate::ai::NpcState::Dead || n.knocked_out || n.hp_body.robot || n.carried || st.pooled.contains(&n.blocker) {
            continue;
        }
        let t = st.dead.entry(n.blocker).or_insert(0.0);
        *t += dt;
        let rested_here = rest_at.remove(&n.blocker);
        if *t < 2.5 && rested_here.is_none() {
            continue;
        }
        let at = rested_here.unwrap_or(d.body.location);
        let h = lvl.collision.trace(at + DVec3::Z * 16.0, at - DVec3::Z * 96.0, DVec3::ZERO);
        if h.fraction >= 1.0 || h.normal.z < 0.7 {
            continue;
        }
        st.pooled.insert(n.blocker);
        let p = at + DVec3::Z * 16.0 + (DVec3::NEG_Z * 112.0) * h.fraction;
        let max = if n.animal { 50.0 } else { 90.0 + rnd(&mut st.seed) as f32 * 40.0 };
        let _ = decal(&mut commands, &mut assets, &mut images, &mut materials, &mut meshes, st, p, h.normal, 1.0, Some(Pool { t: 0.0, max }));
    }
    for (mut p, mut t) in &mut pools {
        if p.t > 40.0 {
            continue;
        }
        p.t += dt;
        let size = p.max * (1.0 - (-p.t / 9.0).exp()).max(0.05);
        t.scale = Vec3::new(size, size, 1.0) * dx_level::METERS_PER_UNIT;
    }
    // Draw the drops in flight: short streaks along their motion, facing the camera.
    let eye = cam.iter().next().map(|c| c.translation()).unwrap_or(Vec3::ZERO);
    if st.mesh.is_none() {
        let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        let cap = MAX_DROPS;
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; cap * 4]);
        m.insert_indices(Indices::U32((0..cap as u32).flat_map(|q| [q * 4, q * 4 + 1, q * 4 + 2, q * 4, q * 4 + 2, q * 4 + 3]).collect()));
        let h = meshes.add(m);
        let mat = materials.add(StandardMaterial {
            base_color: Color::srgba(0.32, 0.0, 0.0, 0.9),
            unlit: true,
            double_sided: true,
            cull_mode: None,
            alpha_mode: AlphaMode::Blend,
            ..default()
        });
        commands.spawn((
            Mesh3d(h.clone()),
            MeshMaterial3d(mat.clone()),
            Transform::default(),
            bevy::camera::visibility::NoFrustumCulling,
            bevy::light::NotShadowCaster,
            bevy::light::NotShadowReceiver,
        ));
        st.mesh = Some(h);
        st.mat = Some(mat);
    }
    // Nothing flying now and nothing drawn last frame: leave the mesh alone (no upload).
    if st.drops.is_empty() && !st.drawn {
        return;
    }
    st.drawn = !st.drops.is_empty();
    if let Some(mut m) = st.mesh.as_ref().and_then(|h| meshes.get_mut(h)) {
        if let Some(bevy::mesh::VertexAttributeValues::Float32x3(pos)) = m.attribute_mut(Mesh::ATTRIBUTE_POSITION) {
            for (q, quad) in pos.chunks_exact_mut(4).enumerate() {
                let Some(d) = st.drops.get(q) else {
                    quad.fill([0.0; 3]);
                    continue;
                };
                let head = bv(d.pos);
                let dir = bv(d.vel).normalize_or(Vec3::NEG_Y);
                let len = (d.vel.length() as f32 * 0.02).clamp(2.0, 10.0) * dx_level::METERS_PER_UNIT;
                let tail = head - dir * len;
                let side = dir.cross((eye - head).normalize_or(Vec3::Z)).normalize_or(Vec3::X) * (d.size * 0.08).max(0.4) * dx_level::METERS_PER_UNIT;
                quad.copy_from_slice(&[(head - side).into(), (head + side).into(), (tail + side).into(), (tail - side).into()]);
            }
        }
    }
}
