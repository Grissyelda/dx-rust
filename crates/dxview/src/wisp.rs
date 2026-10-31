//! Barrel smoke (part of Enhanced Effects): after a shot a thin line of smoke pours from the muzzle for a moment,
//! rising and curling in the air and thinning out - the wisp you see off a hot barrel.
//!
//! First person: the points are born at the view model's muzzle (the centre of its muzzle-flash section), kept in
//! world space so the line trails behind as you move and turn, and drawn on the view model's layer in camera
//! space. NPCs and third person: drawn in the world from their muzzle. One ribbon mesh per layer, fixed size (a
//! mesh changing size every frame upsets Bevy's slab allocator), only rewritten while smoke is about.

use crate::{FlyCam, LevelRes, SkyCam};
use bevy::{
    asset::RenderAssetUsages,
    camera::visibility::RenderLayers,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use dx_level::{bevy_to_ue, ue_to_bevy};
use glam::DVec3;
use std::sync::Mutex;

/// Points per ribbon mesh.
const CAP: usize = 900;
/// How long a shot keeps the barrel smoking, and a point's life.
const EMIT: f32 = 1.3;
const LIFE: f32 = 1.8;
const STEP: f32 = 0.022;

#[derive(Clone, Copy)]
enum Req {
    /// The player fired (first person: from the view model's muzzle; else from `at`).
    Player { at: DVec3, dir: DVec3 },
    /// Someone else fired from `at` (Unreal units) along `dir`.
    World { at: DVec3, dir: DVec3 },
}

static REQS: Mutex<Vec<Req>> = Mutex::new(Vec::new());

pub fn player_shot(at: DVec3, dir: DVec3) {
    if crate::fx::on() {
        if let Ok(mut q) = REQS.lock() {
            q.push(Req::Player { at, dir });
        }
    }
}

pub fn world_shot(at: DVec3, dir: DVec3) {
    if crate::fx::on() {
        if let Ok(mut q) = REQS.lock() {
            if q.len() < 32 {
                q.push(Req::World { at, dir });
            }
        }
    }
}

struct Pt {
    /// Bevy world space.
    pos: Vec3,
    vel: Vec3,
    age: f32,
    /// How thick the smoke was when this bit left the barrel (the stream thins as the barrel cools).
    k: f32,
}

struct Wisp {
    pts: Vec<Pt>,
    /// Seconds of emitting left.
    emit: f32,
    acc: f32,
    /// First person (follows the view model's muzzle).
    view: bool,
    /// World wisps: where they pour from (Bevy), and the barrel's direction.
    at: Vec3,
    dir: Vec3,
    seed: f32,
    /// Lit by the place's light (0..1.1).
    lit: f32,
}

#[derive(Default)]
pub struct WispState {
    wisps: Vec<Wisp>,
    view_mesh: Option<Handle<Mesh>>,
    world_mesh: Option<Handle<Mesh>>,
    seed: f32,
    idle: bool,
}

#[derive(Component)]
pub struct WispMesh;

/// Soft across the line, faintly streaky along it.
fn profile_image() -> Image {
    const W: usize = 32;
    const H: usize = 64;
    let mut px = Vec::with_capacity(W * H * 4);
    for y in 0..H {
        for x in 0..W {
            let u = (x as f32 + 0.5) / W as f32;
            let v = (y as f32 + 0.5) / H as f32;
            let across = (u * std::f32::consts::PI).sin().powf(2.2);
            let streak = 0.75 + 0.25 * ((v * 6.283 * 3.0 + u * 9.0).sin() * 0.5 + 0.5);
            let a = (across * streak * 255.0) as u8;
            px.extend_from_slice(&[255, 255, 255, a]);
        }
    }
    Image::new(
        Extent3d { width: W as u32, height: H as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        px,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn ribbon_mesh() -> Mesh {
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32, -1.0e5, 0.0]; CAP * 2]);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32, 0.0]; CAP * 2]);
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; CAP * 2]);
    m.insert_indices(Indices::U32(vec![0; CAP * 6]));
    m
}

/// A smooth swirl (cheap curl-ish noise from a few sines).
fn swirl(p: Vec3, t: f32, s: f32) -> Vec3 {
    Vec3::new(
        (p.y * 24.0 + t * 2.7 + s).sin() + 0.6 * (p.z * 41.0 - t * 3.3 + s * 2.0).sin(),
        0.15 * (p.x * 30.0 + t * 2.0 + s).sin(),
        (p.x * 26.0 - t * 2.9 + s * 3.0).sin() + 0.6 * (p.y * 37.0 + t * 3.1 + s).sin(),
    )
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn wisp_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    cam: Single<&Transform, (With<FlyCam>, Without<SkyCam>, Without<WispMesh>)>,
    flashes: Query<(&Mesh3d, &GlobalTransform, Option<&RenderLayers>), With<crate::weapons::MuzzleFlashPart>>,
    mut st: Local<WispState>,
    mut map_gen: Local<u64>,
    (mut meshes, mut materials, mut images): (ResMut<Assets<Mesh>>, ResMut<Assets<StandardMaterial>>, ResMut<Assets<Image>>),
) {
    let _prof = crate::prof::scope("wisp::wisp_system");
    if crate::loader::new_map(&mut map_gen) {
        st.wisps.clear();
    }
    if st.view_mesh.is_none() {
        let img = images.add(profile_image());
        let mat = materials.add(StandardMaterial {
            base_color_texture: Some(img),
            unlit: true,
            double_sided: true,
            cull_mode: None,
            alpha_mode: AlphaMode::Blend,
            fog_enabled: false,
            ..default()
        });
        for view in [true, false] {
            let m = meshes.add(ribbon_mesh());
            let mut e = commands.spawn((
                Mesh3d(m.clone()),
                MeshMaterial3d(mat.clone()),
                Transform::default(),
                Visibility::Visible,
                WispMesh,
                bevy::camera::visibility::NoFrustumCulling,
                bevy::light::NotShadowCaster,
                bevy::light::NotShadowReceiver,
            ));
            if view {
                e.insert(RenderLayers::layer(crate::weapons::VIEW_LAYER));
                st.view_mesh = Some(m);
            } else {
                st.world_mesh = Some(m);
            }
        }
    }
    let reqs: Vec<Req> = REQS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    if reqs.is_empty() && st.wisps.is_empty() && st.idle {
        return;
    }
    let dt = time.delta_secs().min(0.1);
    let t = time.elapsed_secs();
    let cam_tf = **cam;
    let to_world = cam_tf.compute_affine();
    // The view model's muzzle (camera space): the centre of its muzzle-flash section.
    let view_muzzle = flashes
        .iter()
        .filter(|(_, _, l)| l.is_some_and(|l| l.intersects(&RenderLayers::layer(crate::weapons::VIEW_LAYER))))
        .find_map(|(m, gt, _)| {
            let mesh = meshes.get(&m.0)?;
            let Some(bevy::mesh::VertexAttributeValues::Float32x3(ps)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
                return None;
            };
            if ps.is_empty() {
                return None;
            }
            let c = ps.iter().fold(Vec3::ZERO, |a, p| a + Vec3::from(*p)) / ps.len() as f32;
            Some(gt.transform_point(c))
        });
    let third = crate::thirdperson::active();
    for r in reqs {
        st.seed += 1.37;
        let seed = st.seed;
        let (view, at_ue, dir_ue) = match r {
            Req::Player { at, dir } => (!third && view_muzzle.is_some(), at, dir),
            Req::World { at, dir } => (false, at, dir),
        };
        let lit = (0.25 + 1.4 * crate::stealth::light_at(&level.0, at_ue)).min(1.1);
        let at = Vec3::from(ue_to_bevy(at_ue.as_vec3().to_array()));
        let dir = Vec3::from(ue_to_bevy(dir_ue.as_vec3().to_array())).normalize_or_zero();
        // Another shot from the same barrel keeps its wisp pouring.
        if let Some(w) = st.wisps.iter_mut().find(|w| w.view == view && (view || w.at.distance(at) < 0.6)) {
            w.emit = EMIT;
            w.at = at;
            w.dir = dir;
            w.lit = lit;
            continue;
        }
        if st.wisps.len() >= 24 {
            st.wisps.remove(0);
        }
        st.wisps.push(Wisp { pts: Vec::new(), emit: EMIT, acc: 0.0, view, at, dir, seed, lit });
    }
    // Pour, rise, curl.
    for w in st.wisps.iter_mut() {
        if w.emit > 0.0 {
            w.emit -= dt;
            w.acc += dt;
            let src = if w.view {
                view_muzzle.map(|m| to_world.transform_point3(m))
            } else {
                Some(w.at)
            };
            // The barrel's heat: a stronger stream at first.
            let k = (w.emit / EMIT).clamp(0.0, 1.0).powf(0.7);
            while w.acc >= STEP {
                w.acc -= STEP;
                if let Some(p) = src {
                    // A little along the barrel at first, then straight up.
                    let vel = Vec3::Y * 0.12 + w.dir * 0.05 * k;
                    w.pts.push(Pt { pos: p, vel, age: 0.0, k });
                }
            }
        }
        for p in w.pts.iter_mut() {
            p.age += dt;
            // Buoyant: it speeds up as it rises, wanders more the older it is.
            let s = swirl(p.pos, t, w.seed);
            // A breeze of its own per wisp, so each leans its own way.
            let drift = Vec3::new(w.seed.sin(), 0.0, (w.seed * 1.7).cos()) * 0.05;
            p.vel += (Vec3::Y * 0.2 + drift + s * 0.32 * (0.25 + p.age)) * dt;
            p.vel *= 1.0 - 1.4 * dt;
            p.pos += p.vel * dt;
        }
        w.pts.retain(|p| p.age < LIFE);
    }
    st.wisps.retain(|w| w.emit > 0.0 || !w.pts.is_empty());
    let was_idle = st.idle;
    st.idle = st.wisps.is_empty();
    if st.idle && was_idle {
        return;
    }
    // Build the two ribbons.
    let to_cam = to_world.inverse();
    let eye_world = cam_tf.translation;
    for view in [true, false] {
        let mut pos: Vec<[f32; 3]> = Vec::with_capacity(CAP * 2);
        let mut uv: Vec<[f32; 2]> = Vec::with_capacity(CAP * 2);
        let mut col: Vec<[f32; 4]> = Vec::with_capacity(CAP * 2);
        let mut idx: Vec<u32> = Vec::with_capacity(CAP * 6);
        for w in st.wisps.iter().filter(|w| w.view == view) {
            if w.pts.len() < 2 || pos.len() / 2 + w.pts.len() > CAP {
                continue;
            }
            // Oldest first: the line runs from its far end down to the barrel.
            let n = w.pts.len();
            let place = |p: Vec3| if view { to_cam.transform_point3(p) } else { p };
            let eye = if view { Vec3::ZERO } else { eye_world };
            let first = pos.len() as u32;
            let mut along = 0.0f32;
            let mut last_side: Option<Vec3> = None;
            for i in 0..n {
                let p = place(w.pts[i].pos);
                let prev = place(w.pts[i.saturating_sub(1)].pos);
                // The direction over a few points either side (tightly packed points near the barrel twisted the
                // ribbon over on itself), and the side kept from flipping between neighbours.
                let a = place(w.pts[i.saturating_sub(3)].pos);
                let b = place(w.pts[(i + 3).min(n - 1)].pos);
                let tan = (b - a).normalize_or(Vec3::Y);
                let mut side = tan.cross((eye - p).normalize_or(Vec3::Z)).normalize_or(Vec3::X);
                if let Some(ls) = last_side {
                    if side.dot(ls) < 0.0 {
                        side = -side;
                    }
                    side = (side + ls * 0.5).normalize_or(side);
                }
                last_side = Some(side);
                let pt = &w.pts[i];
                let a01 = pt.age / LIFE;
                // Thin at the barrel, spreading as it rises.
                let width = if view { 0.004 + 0.03 * a01 } else { 0.008 + 0.09 * a01 };
                let fade_in = (pt.age / 0.08).min(1.0);
                let alpha = 0.16 * pt.k * fade_in * (1.0 - a01).powf(1.6);
                let g = 0.62 * w.lit;
                if i > 0 {
                    along += (p - prev).length();
                }
                pos.push((p - side * width * 0.5).to_array());
                pos.push((p + side * width * 0.5).to_array());
                let v = along * if view { 6.0 } else { 2.0 } - t * 0.4;
                uv.push([0.0, v]);
                uv.push([1.0, v]);
                col.push([g, g, g * 1.04, alpha]);
                col.push([g, g, g * 1.04, alpha]);
            }
            for i in 0..(n as u32 - 1) {
                let a = first + i * 2;
                idx.extend([a, a + 1, a + 3, a, a + 3, a + 2]);
            }
        }
        // Pad to the fixed size.
        while pos.len() < CAP * 2 {
            pos.push([0.0, -1.0e5, 0.0]);
            uv.push([0.0, 0.0]);
            col.push([0.0; 4]);
        }
        idx.resize(CAP * 6, 0);
        let h = if view { st.view_mesh.as_ref() } else { st.world_mesh.as_ref() };
        if let Some(m) = h.and_then(|h| meshes.get_mut(h)) {
            let mut m = m;
            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
            m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
            m.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
            m.insert_indices(Indices::U32(idx));
        }
    }
    let _ = bevy_to_ue;
}
