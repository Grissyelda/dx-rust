//! Fluid blood (enhancement "Fluid Blood"): blood that behaves like a liquid.
//! - Hits throw droplets out along the shot (and a little back); each flies under gravity, is traced against the
//!   level, and leaves a splat where it lands, sized by the drop.
//! - On walls and other steep faces a splat sends trickles down: thin streaks that run with a bead of blood at the
//!   tip, slow as they spend themselves, follow the wall (a trickle that reaches the wall's lower edge drips off
//!   it as a falling drop, one that reaches the floor ends in a small splat).
//! - Wounds on bodies (bodyfx.rs) keep bleeding: drops run out of them and fall (`drip`).
//! - Blood thrown at the eye from close by spatters the weapon in hand (viewblood.rs).
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
    setting!("FluidBlood") > 0.5
}

enum Req {
    /// A hit: where, the shot's direction, how much.
    Spray(DVec3, DVec3, f32),
    /// A body blown apart: centre, radius, height.
    Burst(DVec3, f64, f64),
    /// A drop leaving a wound: where, its velocity, its size.
    Drip(DVec3, DVec3, f32),
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

/// A drop of blood running out of a wound (bodyfx.rs): falls and splats where it lands.
pub fn drip(at: DVec3, vel: DVec3, size: f32) {
    if enabled() {
        if let Ok(mut g) = REQS.lock() {
            g.push(Req::Drip(at, vel, size));
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

/// A trickle running down a steep face from a splat: a streak that lengthens with a bead at its tip.
#[derive(Component)]
pub struct Run {
    /// Where it starts and which way is down the face (Unreal space), the face's normal.
    base: DVec3,
    down: DVec3,
    n: DVec3,
    /// Length so far and speed (units, units / s), how far it may go before it's spent.
    len: f64,
    speed: f64,
    max: f64,
    /// Length at the last check that the wall goes on under the tip.
    checked: f64,
    width: f32,
    bead: Option<Entity>,
    done: bool,
}

/// The bead at a trickle's tip.
#[derive(Component)]
pub struct Bead;

/// A trickle's streak and its bead (both counted against the decal cap).
#[allow(clippy::too_many_arguments)]
fn spawn_run(
    commands: &mut Commands,
    assets: &mut RuntimeAssets,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
    meshes: &mut Assets<Mesh>,
    st: &mut GoreState,
    mut run: Run,
) {
    let q = assets.sprites.quad(meshes);
    let streak = trickle_image(assets, images);
    let bead = bead_image(assets, images);
    let (sm, bm) = (assets.sprites.decal(materials, streak, false), assets.sprites.decal(materials, bead, false));
    let hidden = Transform::from_translation(bv(run.base)).with_scale(Vec3::splat(0.0001));
    let b = commands.spawn((Mesh3d(q.clone()), MeshMaterial3d(bm), hidden, bevy::light::NotShadowCaster, Bead)).id();
    run.bead = Some(b);
    let e = commands.spawn((Mesh3d(q), MeshMaterial3d(sm), hidden, bevy::light::NotShadowCaster)).id();
    commands.entity(e).insert(run);
    st.decals.push_back(e);
    st.decals.push_back(b);
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

/// A decal picture made here (modulated: white leaves the surface as it is, dark red stains it), cached by name.
/// `f(u, v)` = (how much blood, how dark it is) at that point, both 0..1.
pub fn synth_decal(assets: &mut RuntimeAssets, images: &mut Assets<Image>, key: &str, w: u32, h: u32, f: impl Fn(f32, f32) -> (f32, f32)) -> Handle<Image> {
    if let Some(Some(img)) = assets.images.get(key) {
        return img.clone();
    }
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let (a, dark) = f((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
            let a = a.clamp(0.0, 1.0);
            // Fresh blood (sRGB) toward a deep near-black red where it's thick.
            let blood = [0, 1, 2].map(|k| [118.0, 8.0, 8.0][k] * (1.0 - dark) + [34.0, 1.0, 2.0][k] * dark);
            for k in 0..3 {
                data.push((255.0 + (blood[k] - 255.0) * a).round() as u8);
            }
            data.push(255);
        }
    }
    let mut img = Image::new(
        bevy::render::render_resource::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        data,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.sampler = bevy::image::ImageSampler::linear();
    let hnd = images.add(img);
    assets.images.insert(key.to_string(), Some(hnd.clone()));
    hnd
}

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A trickle's streak: a thin line, thicker down the middle, faded in at the top where it leaves the splat.
fn trickle_image(assets: &mut RuntimeAssets, images: &mut Assets<Image>) -> Handle<Image> {
    synth_decal(assets, images, "Synth.BloodTrickle", 16, 64, |u, v| {
        let x = (u - 0.5).abs() * 2.0;
        let core = (1.0 - smooth(0.3, 0.95, x)) * smooth(0.0, 0.15, v);
        (core * 0.85, 0.1 + 0.3 * (1.0 - x))
    })
}

/// A bead of blood (the tip of a trickle, a drop on a weapon): round, darker in the middle, a light rim.
pub fn bead_image(assets: &mut RuntimeAssets, images: &mut Assets<Image>) -> Handle<Image> {
    synth_decal(assets, images, "Synth.BloodBead", 32, 32, |u, v| {
        let r = ((u - 0.5).powi(2) + (v - 0.55).powi(2)).sqrt() * 2.0;
        let a = 1.0 - smooth(0.7, 1.0, r);
        (a, 0.3 + 0.6 * (1.0 - r).max(0.0))
    })
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
        Mesh3d(assets.sprites.quad(meshes)),
        MeshMaterial3d(assets.sprites.decal(materials, img, false)),
        Transform::from_translation(bv(at + n * 0.3)).with_rotation(rot).with_scale(Vec3::splat(size * dx_level::METERS_PER_UNIT)),
        bevy::light::NotShadowCaster,
        crate::impacts::KeepDecal { tex: tex.to_string(), hole: false },
    ));
    if let Some(x) = extra {
        ec.insert(x);
    }
    let e = ec.id();
    st.decals.push_back(e);
    let cap = if setting!("PersistentDebris") > 0.5 { 3000 } else { 400 };
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
    mut runs: Query<(&mut Run, &mut Transform), (Without<Pool>, Without<Bead>)>,
    mut pools: Query<(&mut Pool, &mut Transform), (Without<Run>, Without<Bead>)>,
    mut beads: Query<&mut Transform, (With<Bead>, Without<Run>, Without<Pool>)>,
    mut bead_moves: Local<Vec<(Entity, Vec3, Quat, f32)>>,
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
    let mut reqs: Vec<Req> = REQS.lock().map(|mut g| std::mem::take(&mut *g)).unwrap_or_default();
    // Dev aid: DXVIEW_BLOODWALL=<game secs> throws a big spray where the view points at that time (trickles on a
    // wall, blood on the weapon).
    if let Some(at) = env_val!("DXVIEW_BLOODWALL").and_then(|v| v.parse::<f32>().ok()) {
        let t = time.elapsed_secs();
        if t >= at && t - time.delta_secs() < at {
            if let Some(c) = cam.iter().next() {
                let eye = DVec3::from_array(dx_level::bevy_to_ue(c.translation().to_array()).map(|v| v as f64));
                let f = c.translation() + c.forward().as_vec3();
                let fwd = (DVec3::from_array(dx_level::bevy_to_ue(f.to_array()).map(|v| v as f64)) - eye).normalize_or(DVec3::X);
                let h = level.0.collision.trace(eye, eye + fwd * 2000.0, DVec3::ZERO);
                let hit = eye + fwd * 2000.0 * h.fraction - fwd * 20.0;
                for _ in 0..4 {
                    reqs.push(Req::Spray(hit, fwd, 40.0));
                }
                reqs.push(Req::Spray(eye + fwd * 60.0, -fwd, 30.0));
                println!("BLOODWALL at {hit:?}");
            }
        }
    }
    let rested: Vec<(usize, DVec3)> = RESTED.lock().map(|mut g| std::mem::take(&mut *g)).unwrap_or_default();
    if !enabled() {
        st.drops.clear();
        return;
    }
    let _prof = crate::prof::scope("gore::gore_system");
    let dt = time.delta_secs().min(0.05);
    let lvl = &level.0;
    let st = &mut *st;
    let eye = cam.iter().next().map(|c| c.translation()).unwrap_or(Vec3::ZERO);
    // New blood.
    for r in reqs {
        match r {
            Req::Spray(at, dir, amount) => {
                let dir = dir.normalize_or(DVec3::Z);
                // Close to the eye (point blank, melee): some of it lands on the weapon in hand.
                let d = bv(at).distance(eye) / dx_level::METERS_PER_UNIT;
                if d < 150.0 {
                    crate::viewblood::splash(((1.0 - d / 150.0) * (2.0 + amount * 0.25)).clamp(1.0, 8.0) as u32);
                }
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
            Req::Drip(at, vel, size) => st.drops.push(Drop { pos: at, vel, size, life: 3.0 }),
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
        let _ = decal(&mut commands, &mut assets, &mut images, &mut materials, &mut meshes, st, p, n, size, None::<()>);
        if steep && size > 3.5 && n.z > -0.5 && rnd(&mut st.seed) < 0.75 {
            // Trickles from the splat: 1-3, from across its lower half, each its own speed, a little off straight
            // down (the face's own downhill direction, nudged sideways).
            let downhill = (DVec3::NEG_Z - n * DVec3::NEG_Z.dot(n)).normalize_or(DVec3::NEG_Z);
            let across = downhill.cross(n).normalize_or(DVec3::X);
            let k = 1 + (rnd(&mut st.seed) * 2.99) as usize;
            for _ in 0..k {
                let off = across * ((rnd(&mut st.seed) - 0.5) * size as f64 * 0.6) + downhill * (size as f64 * 0.2);
                let down = (downhill + across * ((rnd(&mut st.seed) - 0.5) * 0.25)).normalize_or(downhill);
                let max = 18.0 + rnd(&mut st.seed) * (size as f64 * 9.0).min(90.0);
                let run = Run {
                    base: p + off,
                    down,
                    n,
                    len: 0.0,
                    speed: 14.0 + rnd(&mut st.seed) * 22.0,
                    max,
                    checked: 0.0,
                    width: (0.5 + size * 0.06 + rnd(&mut st.seed) as f32 * 0.4).min(1.6),
                    bead: None,
                    done: false,
                };
                spawn_run(&mut commands, &mut assets, &mut images, &mut materials, &mut meshes, st, run);
            }
        }
    }
    // Trickles run: slowing as they spend themselves, following the face; one that runs off the wall's lower
    // edge drips off it, one that meets the floor ends in a small splat.
    let mut ends: Vec<(DVec3, DVec3, f32, bool)> = Vec::new();
    for (mut r, mut t) in &mut runs {
        if r.done {
            continue;
        }
        let step = r.speed * dt as f64;
        r.len += step;
        // Running dry: the speed falls off as the trickle uses itself up (a little faster near the end).
        let spent = (r.len / r.max).min(1.0);
        r.speed = (r.speed - dt as f64 * (6.0 + 30.0 * spent * spent)).max(0.0);
        let mut stop = r.speed <= 0.5 || r.len >= r.max;
        if r.len - r.checked > 3.0 {
            r.checked = r.len;
            let tip = r.base + r.down * r.len;
            let h = lvl.collision.trace(tip + r.n * 4.0, tip - r.n * 6.0, DVec3::ZERO);
            // How far in the face is at the tip (4 = where it was): much deeper = the face ended (an edge, a
            // recess under it) and the bead drops off; much shallower = something sticks out (the trickle stops).
            let depth = h.fraction * 10.0;
            if h.fraction >= 1.0 || depth > 5.5 {
                ends.push((tip + r.n * 1.5, DVec3::ZERO, r.width * 2.2, true));
                stop = true;
            } else if h.fraction <= 0.02 {
                // Under the floor (the probe starts in solid): it ran down onto it - a small splat there.
                ends.push((tip + r.n * 2.0 + DVec3::Z * 2.0, DVec3::Z, r.width * 3.0, false));
                stop = true;
            } else if depth < 2.5 && h.normal.z <= 0.6 {
                stop = true;
            } else if h.normal.z > 0.6 {
                // Down onto a floor.
                ends.push((tip, h.normal, r.width * 3.0, false));
                stop = true;
            } else if h.normal.dot(r.n) < 0.95 {
                // The face bends: keep to it.
                let n = h.normal;
                let down = (DVec3::NEG_Z - n * DVec3::NEG_Z.dot(n)).normalize_or(r.down);
                r.base = r.base + r.down * r.len - down * r.len;
                r.down = down;
                r.n = n;
            }
        }
        if stop {
            r.done = true;
        }
        let (base, down, nb) = (bv(r.base + r.n * 0.35), bv(r.down).normalize_or(Vec3::NEG_Y), bv(r.n).normalize_or(Vec3::X));
        let wm = r.width * dx_level::METERS_PER_UNIT;
        let len = (r.len as f32).max(0.5) * dx_level::METERS_PER_UNIT;
        // Quad Y up the trickle, Z out of the wall, X = Y x Z (a right-handed frame).
        let rot = Quat::from_mat3(&Mat3::from_cols((-down).cross(nb).normalize_or(Vec3::X), -down, nb));
        t.translation = base + down * (len * 0.5);
        t.rotation = rot;
        t.scale = Vec3::new(wm, len, 1.0);
        if let Some(b) = r.bead {
            bead_moves.push((b, base + down * len + nb * 0.0005, rot, wm * 1.7));
        }
    }
    for (b, at, rot, w) in bead_moves.drain(..) {
        if let Ok(mut t) = beads.get_mut(b) {
            t.translation = at;
            t.rotation = rot;
            t.scale = Vec3::new(w, w * 1.25, 1.0);
        }
    }
    for (p, n, size, falls) in ends {
        if falls {
            st.drops.push(Drop { pos: p, vel: DVec3::ZERO, size, life: 3.0 });
        } else {
            // Onto the floor under the tip.
            let h = lvl.collision.trace(p + DVec3::Z * 8.0, p - DVec3::Z * 16.0, DVec3::ZERO);
            let (p, n) = if h.fraction < 1.0 { (p + DVec3::Z * 8.0 - DVec3::Z * 24.0 * h.fraction, h.normal) } else { (p, n) };
            let _ = decal(&mut commands, &mut assets, &mut images, &mut materials, &mut meshes, st, p, n, size, None::<()>);
        }
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
