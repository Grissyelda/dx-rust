//! Enhancement (Enhanced Effects, ROADMAP v2 #27): extra particles on top of the original's. Ours, off by default.
//!
//! - Bullet impacts by surface: sparks off metal, dust off stone and concrete, splinters off wood, earth off
//!   ground, glints off glass, little splashes off water.
//! - Explosions: a burst of sparks and embers, and dark smoke that rolls up and hangs about.
//! - Gunfire: a wisp of smoke from the muzzle.
//! - Hits on people: a fine spray of blood.
//! - Electricity (elements.rs): sparks crawling over water.
//!
//! One pool of CPU particles drawn as two meshes of camera-facing quads (glowing ones added, the rest blended),
//! each a fixed size so the render world never reallocates them. The sprites are drawn here (a soft disc), so
//! nothing depends on the game's textures.

use crate::{FlyCam, LevelRes, SkyCam};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use dx_level::ue_to_bevy;
use glam::DVec3;
use std::sync::Mutex;

const MAX_GLOW: usize = 1200;
const MAX_SOFT: usize = 900;

pub fn on() -> bool {
    crate::settings::get("EnhancedFX") > 0.5
}

#[derive(Clone, Copy)]
struct P {
    pos: DVec3,
    vel: DVec3,
    life: f32,
    max: f32,
    size: (f32, f32),
    col: [f32; 4],
    /// Fades from `col` to this.
    col_end: [f32; 4],
    gravity: f64,
    drag: f64,
    glow: bool,
    /// Drawn as a streak along its velocity (sparks).
    streak: bool,
    /// Bounces off the level (sparks, chips).
    bounce: bool,
}

enum Req {
    Impact { at: DVec3, normal: DVec3, dir: DVec3, group: Option<String> },
    Explosion { at: DVec3, radius: f32 },
    Muzzle { at: DVec3, dir: DVec3 },
    Blood { at: DVec3, dir: DVec3 },
    Sparks { at: DVec3, n: usize, speed: f64, col: [f32; 4] },
}

static REQS: Mutex<Vec<Req>> = Mutex::new(Vec::new());

fn push(r: Req) {
    if !on() {
        return;
    }
    if let Ok(mut q) = REQS.lock() {
        if q.len() < 256 {
            q.push(r);
        }
    }
}

/// A shot hit the level at `at` (surface normal, travelling along `dir`); `group` is the texture group.
pub fn impact(at: DVec3, normal: DVec3, dir: DVec3, group: Option<&str>) {
    push(Req::Impact { at, normal, dir, group: group.map(|g| g.to_ascii_lowercase()) });
}

pub fn explosion(at: DVec3, radius: f32) {
    push(Req::Explosion { at, radius });
}

pub fn muzzle(at: DVec3, dir: DVec3) {
    push(Req::Muzzle { at, dir });
}

pub fn blood(at: DVec3, dir: DVec3) {
    push(Req::Blood { at, dir });
}

/// Electric sparks (bluish white) at `at`.
pub fn sparks(at: DVec3, n: usize, speed: f64) {
    push(Req::Sparks { at, n, speed, col: [0.7, 0.85, 1.0, 1.0] });
}

#[derive(Default)]
pub struct FxState {
    ps: Vec<P>,
    seed: u64,
    glow_mesh: Option<Handle<Mesh>>,
    soft_mesh: Option<Handle<Mesh>>,
    /// The meshes were last built empty.
    idle: bool,
}

#[derive(Component)]
pub struct FxMesh;

fn rnd(seed: &mut u64) -> f64 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    ((*seed >> 11) as f64) / (1u64 << 53) as f64
}

fn rand_dir(seed: &mut u64) -> DVec3 {
    loop {
        let v = DVec3::new(rnd(seed) * 2.0 - 1.0, rnd(seed) * 2.0 - 1.0, rnd(seed) * 2.0 - 1.0);
        let l = v.length_squared();
        if l > 1e-4 && l <= 1.0 {
            return v / l.sqrt();
        }
    }
}

/// A direction in the hemisphere about `n`, leaning toward it by `k`.
fn cone(n: DVec3, k: f64, seed: &mut u64) -> DVec3 {
    let mut d = rand_dir(seed);
    if d.dot(n) < 0.0 {
        d = -d;
    }
    (d + n * k).normalize_or(n)
}

fn spawn(st: &mut FxState, r: Req, level: &dx_level::Level) {
    let s = &mut st.seed;
    let mut out: Vec<P> = Vec::new();
    let at_req = match &r {
        Req::Impact { at, .. } | Req::Explosion { at, .. } | Req::Muzzle { at, .. } | Req::Blood { at, .. } | Req::Sparks { at, .. } => *at,
    };
    match r {
        Req::Impact { at, normal, dir, group } => {
            let refl = (dir - normal * 2.0 * dir.dot(normal)).normalize_or(normal);
            let g = group.as_deref().unwrap_or("");
            let at = at + normal * 1.5;
            let metal = matches!(g, "metal" | "ladder");
            let wood = g == "wood";
            let glass = g == "glass";
            let earth = matches!(g, "earth" | "foliage");
            let water = g == "water";
            if metal || glass {
                for _ in 0..(6 + (rnd(s) * 8.0) as usize) {
                    let v = (cone(normal, 0.6, s) * 0.6 + refl * 0.4).normalize_or(normal);
                    out.push(P {
                        pos: at,
                        vel: v * (250.0 + rnd(s) * 350.0),
                        life: 0.0,
                        max: 0.25 + rnd(s) as f32 * 0.35,
                        size: (0.9, 0.5),
                        col: if glass { [0.8, 0.9, 1.0, 1.0] } else { [1.0, 0.75, 0.35, 1.0] },
                        col_end: if glass { [0.3, 0.4, 0.5, 0.0] } else { [0.8, 0.2, 0.0, 0.0] },
                        gravity: 500.0,
                        drag: 1.0,
                        glow: true,
                        streak: true,
                        bounce: true,
                    });
                }
            }
            // Dust (grey stone, brown earth, pale wood splinters), a little splash off water.
            let (dust_col, n_dust, chips) = if water {
                ([0.55, 0.65, 0.7, 0.55], 6, 0)
            } else if wood {
                ([0.42, 0.33, 0.24, 0.6], 3, 6)
            } else if earth {
                ([0.33, 0.27, 0.2, 0.65], 5, 3)
            } else if metal {
                ([0.35, 0.35, 0.35, 0.35], 2, 0)
            } else if glass {
                ([0.6, 0.65, 0.7, 0.25], 1, 4)
            } else {
                ([0.5, 0.48, 0.45, 0.6], 5, 4)
            };
            for _ in 0..n_dust {
                let v = cone(normal, 1.2, s);
                out.push(P {
                    pos: at,
                    vel: v * (30.0 + rnd(s) * 60.0) + if water { DVec3::new(0.0, 0.0, 120.0) } else { DVec3::ZERO },
                    life: 0.0,
                    max: 0.8 + rnd(s) as f32 * 1.0,
                    size: (3.0, 14.0 + rnd(s) as f32 * 8.0),
                    col: dust_col,
                    col_end: [dust_col[0], dust_col[1], dust_col[2], 0.0],
                    gravity: if water { 400.0 } else { -6.0 },
                    drag: 2.5,
                    glow: false,
                    streak: false,
                    bounce: false,
                });
            }
            for _ in 0..chips {
                let v = cone(normal, 0.8, s);
                let c = if wood { [0.55, 0.42, 0.28, 1.0] } else if glass { [0.75, 0.85, 0.95, 0.8] } else { [0.4, 0.38, 0.35, 1.0] };
                out.push(P {
                    pos: at,
                    vel: v * (120.0 + rnd(s) * 200.0),
                    life: 0.0,
                    max: 0.7 + rnd(s) as f32 * 0.6,
                    size: (0.8, 0.6),
                    col: c,
                    col_end: [c[0], c[1], c[2], 0.0],
                    gravity: 700.0,
                    drag: 0.5,
                    glow: false,
                    streak: wood,
                    bounce: true,
                });
            }
        }
        Req::Explosion { at, radius } => {
            let r = (radius as f64).clamp(64.0, 600.0);
            for _ in 0..(30 + (r / 8.0) as usize).min(110) {
                let v = rand_dir(s) + DVec3::new(0.0, 0.0, 0.4);
                out.push(P {
                    pos: at,
                    vel: v.normalize_or(DVec3::Z) * (r * 1.4 + rnd(s) * r * 2.2),
                    life: 0.0,
                    max: 0.5 + rnd(s) as f32 * 1.4,
                    size: (1.6, 0.8),
                    col: [1.0, 0.7, 0.3, 1.0],
                    col_end: [0.7, 0.15, 0.0, 0.0],
                    gravity: 450.0,
                    drag: 0.8,
                    glow: true,
                    streak: true,
                    bounce: true,
                });
            }
            for _ in 0..(10 + (r / 25.0) as usize).min(40) {
                let off = rand_dir(s) * r * 0.35 * rnd(s);
                out.push(P {
                    pos: at + off,
                    vel: off * 0.6 + DVec3::new(0.0, 0.0, 30.0 + rnd(s) * 50.0),
                    life: 0.0,
                    max: 3.0 + rnd(s) as f32 * 3.0,
                    size: (r as f32 * 0.25, r as f32 * 0.7),
                    col: [0.08, 0.07, 0.065, 0.75],
                    col_end: [0.12, 0.11, 0.1, 0.0],
                    gravity: -12.0,
                    drag: 0.9,
                    glow: false,
                    streak: false,
                    bounce: false,
                });
            }
        }
        Req::Muzzle { at, dir } => {
            for _ in 0..3 {
                out.push(P {
                    pos: at + dir * (rnd(s) * 6.0),
                    vel: dir * (20.0 + rnd(s) * 30.0) + rand_dir(s) * 6.0 + DVec3::new(0.0, 0.0, 10.0),
                    life: 0.0,
                    max: 0.6 + rnd(s) as f32 * 0.5,
                    size: (1.2, 6.0),
                    col: [0.6, 0.6, 0.62, 0.12],
                    col_end: [0.6, 0.6, 0.62, 0.0],
                    gravity: -15.0,
                    drag: 2.0,
                    glow: false,
                    streak: false,
                    bounce: false,
                });
            }
        }
        Req::Blood { at, dir } => {
            for _ in 0..10 {
                let v = (dir * 0.7 + rand_dir(s) * 0.6).normalize_or(dir);
                out.push(P {
                    pos: at,
                    vel: v * (60.0 + rnd(s) * 140.0),
                    life: 0.0,
                    max: 0.35 + rnd(s) as f32 * 0.35,
                    size: (1.2, 3.5),
                    col: [0.45, 0.02, 0.02, 0.9],
                    col_end: [0.3, 0.0, 0.0, 0.0],
                    gravity: 500.0,
                    drag: 1.5,
                    glow: false,
                    streak: false,
                    bounce: false,
                });
            }
        }
        Req::Sparks { at, n, speed, col } => {
            for _ in 0..n {
                let mut v = rand_dir(s);
                v.z = v.z.abs() * 0.6;
                out.push(P {
                    pos: at,
                    vel: v * speed * (0.5 + rnd(s)),
                    life: 0.0,
                    max: 0.15 + rnd(s) as f32 * 0.3,
                    size: (0.8, 0.3),
                    col,
                    col_end: [col[0] * 0.4, col[1] * 0.5, col[2], 0.0],
                    gravity: 300.0,
                    drag: 1.0,
                    glow: true,
                    streak: true,
                    bounce: true,
                });
            }
        }
    }
    // Smoke, dust and blood take the light where they are (the glowing ones make their own).
    let lit = (0.12 + 1.5 * crate::stealth::light_at(level, at_req)).min(1.1);
    for p in out.iter_mut().filter(|p| !p.glow) {
        for i in 0..3 {
            p.col[i] *= lit;
            p.col_end[i] *= lit;
        }
    }
    st.ps.extend(out);
}

/// A soft round sprite (white, alpha falling off to the edge).
fn disc_image() -> Image {
    const N: u32 = 32;
    let mut px = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let dx = (x as f32 + 0.5) / N as f32 * 2.0 - 1.0;
            let dy = (y as f32 + 0.5) / N as f32 * 2.0 - 1.0;
            let r = (dx * dx + dy * dy).sqrt();
            let a = (1.0 - r).clamp(0.0, 1.0).powf(1.6);
            px.extend([255, 255, 255, (a * 255.0) as u8]);
        }
    }
    Image::new(
        Extent3d { width: N, height: N, depth_or_array_layers: 1 },
        TextureDimension::D2,
        px,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn quads(cap: usize) -> Mesh {
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    let (mut p, mut u, mut c, mut i) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    pad(&mut p, &mut u, &mut c, &mut i, cap);
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, p);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, u);
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, c);
    m.insert_indices(Indices::U32(i));
    m
}

/// Fills up to `cap` quads with collapsed ones, so the mesh keeps one size.
fn pad(pos: &mut Vec<[f32; 3]>, uv: &mut Vec<[f32; 2]>, col: &mut Vec<[f32; 4]>, idx: &mut Vec<u32>, cap: usize) {
    pos.truncate(cap * 4);
    uv.truncate(cap * 4);
    col.truncate(cap * 4);
    idx.truncate(cap * 6);
    while pos.len() < cap * 4 {
        let k = pos.len() as u32;
        pos.extend([[0.0, -1.0e5, 0.0]; 4]);
        uv.extend([[0.0, 0.0]; 4]);
        col.extend([[0.0, 0.0, 0.0, 0.0]; 4]);
        idx.extend([k, k, k, k, k, k]);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn fx_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    cam: Single<&Transform, (With<FlyCam>, Without<SkyCam>, Without<FxMesh>)>,
    mut st: Local<FxState>,
    mut map_gen: Local<u64>,
    (mut meshes, mut materials, mut images): (ResMut<Assets<Mesh>>, ResMut<Assets<StandardMaterial>>, ResMut<Assets<Image>>),
) {
    let _prof = crate::prof::scope("fx::fx_system");
    if crate::loader::new_map(&mut map_gen) {
        let seed = st.seed;
        *st = FxState::default();
        st.seed = seed ^ 0xF00D_1234_5678;
    }
    if st.glow_mesh.is_none() {
        let img = images.add(disc_image());
        let gm = meshes.add(quads(MAX_GLOW));
        let sm = meshes.add(quads(MAX_SOFT));
        let mat = |materials: &mut Assets<StandardMaterial>, alpha: AlphaMode| {
            materials.add(StandardMaterial {
                base_color_texture: Some(img.clone()),
                unlit: true,
                double_sided: true,
                cull_mode: None,
                alpha_mode: alpha,
                ..default()
            })
        };
        let gmat = mat(&mut materials, AlphaMode::Add);
        let smat = mat(&mut materials, AlphaMode::Blend);
        for (m, mt) in [(gm.clone(), gmat), (sm.clone(), smat)] {
            commands.spawn((
                Mesh3d(m),
                MeshMaterial3d(mt),
                Transform::default(),
                Visibility::Visible,
                FxMesh,
                bevy::camera::visibility::NoFrustumCulling,
                bevy::light::NotShadowCaster,
                bevy::light::NotShadowReceiver,
            ));
        }
        st.glow_mesh = Some(gm);
        st.soft_mesh = Some(sm);
    }
    // DXVIEW_FXTEST=1: impacts on whatever the camera looks at, by surface, and an explosion now and then.
    if std::env::var_os("DXVIEW_FXTEST").is_some() && on() {
        let e = dx_level::bevy_to_ue(cam.translation.to_array());
        let e = DVec3::new(e[0] as f64, e[1] as f64, e[2] as f64);
        let f = cam.forward().as_vec3();
        let f = DVec3::new(f.x as f64, f.z as f64, f.y as f64);
        let h = level.0.collision.trace(e, e + f * 3000.0, DVec3::ZERO);
        if h.fraction < 1.0 {
            let at = e + f * 3000.0 * h.fraction;
            let k = (time.elapsed_secs() * 10.0) as u64;
            if k % 3 == 0 {
                for g in ["metal", "stone", "wood"] {
                    let side = DVec3::new(-f.y, f.x, 0.0).normalize_or_zero() * match g { "metal" => -60.0, "wood" => 60.0, _ => 0.0 };
                    impact(at + side, h.normal, f, Some(g));
                }
            }
            if k % 40 == 0 {
                explosion(at + h.normal * 40.0 + f * -200.0, 256.0);
            }
            if k % 5 == 0 {
                muzzle(e + f * 40.0, f);
                blood(at + h.normal * 30.0 - f * 120.0, f);
            }
        }
    }
    let reqs: Vec<Req> = REQS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    let had = !st.ps.is_empty() || !reqs.is_empty();
    for r in reqs {
        spawn(&mut st, r, &level.0);
    }
    let dt = time.delta_secs().min(0.1);
    let lvl = &level.0;
    let st = &mut *st;
    // Nothing to draw this frame or last: leave the (already empty) meshes alone.
    if !had && st.ps.is_empty() && st.idle {
        return;
    }
    st.idle = st.ps.is_empty();
    st.ps.retain_mut(|p| {
        p.life += dt;
        if p.life >= p.max {
            return false;
        }
        let d = dt as f64;
        p.vel.z -= p.gravity * d;
        p.vel *= (1.0 - p.drag * d).max(0.0);
        let next = p.pos + p.vel * d;
        if p.bounce && p.vel.length_squared() > 1.0 {
            let h = lvl.collision.trace(p.pos, next, DVec3::ZERO);
            if h.fraction < 1.0 {
                p.pos += (next - p.pos) * h.fraction + h.normal * 0.5;
                p.vel = (p.vel - h.normal * 2.0 * p.vel.dot(h.normal)) * 0.35;
                return true;
            }
        }
        p.pos = next;
        true
    });
    // Oldest go first when over the caps.
    let (glow_n, soft_n) = st.ps.iter().fold((0, 0), |(g, s), p| if p.glow { (g + 1, s) } else { (g, s + 1) });
    if glow_n > MAX_GLOW || soft_n > MAX_SOFT {
        let (mut g_over, mut s_over) = (glow_n.saturating_sub(MAX_GLOW), soft_n.saturating_sub(MAX_SOFT));
        st.ps.retain(|p| {
            if p.glow && g_over > 0 {
                g_over -= 1;
                false
            } else if !p.glow && s_over > 0 {
                s_over -= 1;
                false
            } else {
                true
            }
        });
    }
    let conv = |v: Vec3| DVec3::new(v.x as f64, v.z as f64, v.y as f64);
    let right = conv(cam.right().as_vec3());
    let up = conv(cam.up().as_vec3());
    let eye = {
        let e = dx_level::bevy_to_ue(cam.translation.to_array());
        DVec3::new(e[0] as f64, e[1] as f64, e[2] as f64)
    };
    let b = |p: DVec3| Vec3::from(ue_to_bevy(p.as_vec3().to_array())).to_array();
    for glow in [true, false] {
        let mut pos = Vec::new();
        let mut uv = Vec::new();
        let mut col = Vec::new();
        let mut idx = Vec::new();
        // Blended ones far to near, so nearer smoke covers farther.
        // (Each particle's distance is taken once, not twice per comparison.)
        let mut list: Vec<(f64, &P)> = st.ps.iter().filter(|p| p.glow == glow).map(|p| (if glow { 0.0 } else { p.pos.distance_squared(eye) }, p)).collect();
        if !glow {
            list.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
        }
        for (_, p) in list {
            let t = p.life / p.max;
            let size = (p.size.0 + (p.size.1 - p.size.0) * t) as f64;
            let mut c = [0.0f32; 4];
            for i in 0..4 {
                c[i] = p.col[i] + (p.col_end[i] - p.col[i]) * t;
            }
            // Smoke and dust fade in quickly as well as out.
            if !glow {
                c[3] *= (p.life / 0.08).min(1.0);
            }
            let k = pos.len() as u32;
            if p.streak && p.vel.length_squared() > 1.0 {
                let dir = p.vel.normalize();
                let to_eye = (eye - p.pos).normalize_or_zero();
                let side = dir.cross(to_eye).normalize_or_zero() * size * 0.5;
                let tail = p.pos - p.vel * 0.025;
                pos.extend([b(p.pos - side), b(p.pos + side), b(tail + side), b(tail - side)]);
            } else {
                let (r, u) = (right * size, up * size);
                pos.extend([b(p.pos - r - u), b(p.pos + r - u), b(p.pos + r + u), b(p.pos - r + u)]);
            }
            uv.extend([[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]);
            if glow {
                // Additive: alpha scales the colour.
                let a = c[3];
                col.extend([[c[0] * a * 2.0, c[1] * a * 2.0, c[2] * a * 2.0, 1.0]; 4]);
            } else {
                col.extend([c; 4]);
            }
            idx.extend([k, k + 1, k + 2, k, k + 2, k + 3]);
        }
        let cap = if glow { MAX_GLOW } else { MAX_SOFT };
        pad(&mut pos, &mut uv, &mut col, &mut idx, cap);
        let h = if glow { st.glow_mesh.as_ref() } else { st.soft_mesh.as_ref() };
        if let Some(m) = h.and_then(|h| meshes.get_mut(h)) {
            let mut m = m;
            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
            m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
            m.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
            m.insert_indices(Indices::U32(idx));
        }
    }
}
