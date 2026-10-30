//! Water surfaces. The original's water is a WetTexture: its source picture displaced every frame by a little
//! wave simulation (WaterTexture drops), drawn translucent on the zone portal. Here the source picture is drawn
//! by a shader that displaces it with an analytic swell (a few travelling sine waves) plus expanding rings
//! where something hit the water (splashes, the player wading or diving), shades it with the perturbed normal
//! (a sky glint, more opaque at grazing angles) - and floating things bob on the same swell (`height`), so what
//! floats moves with the surface you see. Deviation from the original: the swell is ours, not WaterTexture's
//! drop simulation, and there was no bobbing (DeusExDecoration only rocked floating things in place).

use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use glam::DVec3;
use std::sync::Mutex;

pub type WaterMaterial = ExtendedMaterial<StandardMaterial, WaterExt>;

pub const WATER_SHADER: Handle<Shader> = bevy::asset::uuid_handle!("8e2f4a17-5b3c-4d9e-a061-7c2b9f3e5d48");

/// Ripple rings at once (oldest replaced).
pub const RINGS: usize = 8;

#[derive(Clone, Copy, Debug, Default, ShaderType, Reflect)]
pub struct WaterParams {
    /// x: time (s), y: UV displacement strength, z: brightness of the (added) lit texture, w: glint strength.
    pub t: Vec4,
    /// x, y: distance (m) the water starts fading out and is gone.
    pub far: Vec4,
    /// Rings: Bevy x, z (meters), start time, strength (0 = unused).
    pub rings: [Vec4; RINGS],
    /// Enhancement "Modern Water": x 1 = on, y 1 = the reflection image holds this plane's reflection, z rain on the
    /// surface 0..1, w how strongly the waves bend the reflection.
    pub m: Vec4,
    /// Modern water's deep colour (rgb) and how see-through it is straight down (a).
    pub tint: Vec4,
    /// Direction to the moon (xyz, Bevy) and the glint's strength (w).
    pub moon: Vec4,
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct WaterExt {
    #[uniform(100)]
    pub params: WaterParams,
    /// The planar reflection of this water's plane (mirror.rs slots), when Modern Water is on.
    #[texture(101)]
    #[sampler(102)]
    pub reflection: Handle<Image>,
}

impl MaterialExtension for WaterExt {
    fn vertex_shader() -> ShaderRef {
        WATER_SHADER.into()
    }
    fn fragment_shader() -> ShaderRef {
        WATER_SHADER.into()
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
}

pub fn setup_water_shader(mut shaders: ResMut<Assets<Shader>>) {
    let _ = shaders.insert(
        &WATER_SHADER,
        Shader::from_wgsl(include_str!("shaders/water.wgsl"), "dxview/water.wgsl"),
    );
}

/// The swell: (direction angle, wavelength m, speed m/s, amplitude). Shared with the shader (keep in step).
const WAVES: [(f32, f32, f32, f32); 6] = [
    (0.35, 6.0, 1.1, 0.50),
    (0.90, 17.0, 2.1, 0.85),
    (1.95, 3.3, 0.85, 0.30),
    (4.10, 1.8, 0.65, 0.18),
    (2.75, 0.95, 0.45, 0.10),
    (5.30, 0.55, 0.38, 0.06),
];

/// Surface height (-1..1-ish) and its gradient (per meter) at a Bevy-space x, z, at time t: the same field the
/// shader draws (rings left out).
pub fn swell(x: f32, z: f32, t: f32) -> (f32, f32, f32) {
    let (mut h, mut gx, mut gz) = (0.0, 0.0, 0.0);
    for (ang, len, speed, amp) in WAVES {
        let (dx, dz) = (ang.cos(), ang.sin());
        let k = std::f32::consts::TAU / len;
        let ph = (dx * x + dz * z) * k - t * speed * k;
        h += amp * ph.sin();
        gx += amp * k * dx * ph.cos();
        gz += amp * k * dz * ph.cos();
    }
    (h, gx, gz)
}

/// How far a floating thing rises and falls on the swell, Unreal units (the original's gentle rocking; with Modern
/// Water the surface itself moves by WAVE_M metres per unit of swell and floating things ride it).
const BOB: f64 = 3.0;
pub const WAVE_M: f32 = 0.2;

fn bob_units() -> f64 {
    if modern() { WAVE_M as f64 / dx_level::METERS_PER_UNIT as f64 } else { BOB }
}

/// Vertical offset (Unreal units) and tilt (pitch, roll in Unreal rotator units) for something floating at an
/// Unreal-space spot.
pub fn bob(at: DVec3, t: f64) -> (f64, i32, i32) {
    let p = dx_level::ue_to_bevy([at.x as f32, at.y as f32, at.z as f32]);
    let (h, gx, gz) = swell(p[0], p[2], t as f32);
    // Slope in Unreal X (Bevy x) and Unreal Y (Bevy z), scaled down to a gentle tilt.
    let k = 600.0;
    (h as f64 * bob_units(), (gx * k) as i32, (gz * k) as i32)
}

static RINGS_Q: Mutex<Vec<(DVec3, f32)>> = Mutex::new(Vec::new());

/// Something hit the water at an Unreal-space spot: a ring spreads from it (strength ~0.5 wading .. 1.5 a dive).
pub fn ripple(at: DVec3, strength: f32) {
    if let Ok(mut q) = RINGS_Q.lock() {
        q.push((at, strength));
    }
}

#[derive(Resource, Default)]
pub struct WaterMats {
    pub mats: Vec<Handle<WaterMaterial>>,
    /// Drawn solid in the original (a portal sheet without PF_Translucent), by material.
    pub opaque: Vec<bool>,
    rings: [Vec4; RINGS],
    next: usize,
    modern: Option<bool>,
}

/// Enhancement "Modern Water" (setting ModernWater): reflections, a body colour and see-through depth, detailed
/// waves, rain rings - a shader-pack look instead of the original's translucent picture.
pub fn modern() -> bool {
    crate::settings::get("ModernWater") > 0.5
}

/// Each frame: time and the live rings into every water material.
pub fn water_system(
    time: Res<Time>,
    mut wm: ResMut<WaterMats>,
    mut mats: ResMut<Assets<WaterMaterial>>,
    level: Res<crate::LevelRes>,
    mut moon: Local<(u64, Vec3)>,
) {
    let t = time.elapsed_secs();
    if crate::loader::new_map(&mut moon.0) {
        moon.1 = crate::shafts::moon_dir(&level.0).filter(|d| d.y > 0.05).unwrap_or(Vec3::new(0.3, 0.8, 0.5).normalize());
    }
    let moon_dir = moon.1;
    let new: Vec<(DVec3, f32)> = RINGS_Q.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    for (at, s) in new {
        let p = dx_level::ue_to_bevy([at.x as f32, at.y as f32, at.z as f32]);
        let i = wm.next % RINGS;
        wm.rings[i] = Vec4::new(p[0], p[2], t, s);
        wm.next += 1;
    }
    for r in wm.rings.iter_mut() {
        if r.w > 0.0 && t - r.z > 4.0 {
            r.w = 0.0;
        }
    }
    let rings = wm.rings;
    let modern = modern();
    let switched = wm.modern != Some(modern);
    wm.modern = Some(modern);
    let rain = (crate::weather::density() / 14.0).clamp(0.0, 1.0);
    for (i, h) in wm.mats.iter().enumerate() {
        if let Some(mut m) = mats.get_mut(h) {
            m.extension.params.t.x = t;
            m.extension.params.rings = rings;
            m.extension.params.m.x = if modern { 1.0 } else { 0.0 };
            m.extension.params.m.z = rain;
            m.extension.params.moon = moon_dir.extend(1.0);
            m.extension.params.far.w = if modern { WAVE_M } else { 0.0 };
            if switched {
                let opaque = wm.opaque.get(i).copied().unwrap_or(false);
                m.base.alpha_mode = match (opaque, modern) {
                    (true, _) => AlphaMode::Opaque,
                    (false, true) => AlphaMode::Blend,
                    (false, false) => AlphaMode::Add,
                };
                m.base.perceptual_roughness = if modern { 0.07 } else { 1.0 };
                m.base.reflectance = if modern { 0.5 } else { 0.05 };
            }
        }
    }
}

/// The water material for a level batch (its source picture, translucent). `placeholder` stands in for the
/// reflection until a mirror slot holds this plane.
pub fn material(base: StandardMaterial, opaque: bool, placeholder: Handle<Image>) -> WaterMaterial {
    WaterMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            // PF_Translucent like the original's water faces: added onto what is beneath; a portal sheet without it
            // (Battery Park) is solid.
            alpha_mode: if opaque { AlphaMode::Opaque } else { AlphaMode::Add },
            ..base
        },
        extension: WaterExt {
            params: WaterParams {
                // The lit texture as is (PF_Translucent: texture x lightmap added on), so the lightmap's shadows and
                // lamp pools show; 2026-10-26 playtest: the old unlit sky term made the water too bright and even.
                t: Vec4::new(
                    0.0,
                    0.035,
                    std::env::var("DXVIEW_WATER_GAIN").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0),
                    0.03,
                ),
                far: Vec4::new(30.0, 120.0, std::env::var("DXVIEW_WATER_DEBUG").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0), 0.0),
                rings: [Vec4::ZERO; RINGS],
                m: Vec4::new(0.0, 0.0, 0.0, std::env::var("DXVIEW_WATER_BEND").ok().and_then(|v| v.parse().ok()).unwrap_or(0.03)),
                // Night harbour water: dark blue-green, a fair way see-through looking straight down.
                tint: Vec4::new(0.006, 0.026, 0.040, 0.45),
                moon: Vec4::new(0.3, 0.8, 0.5, 1.0),
            },
            reflection: placeholder,
        },
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn swell_gradient_matches_height() {
        let (h0, gx, gz) = super::swell(3.0, -2.0, 1.5);
        let e = 1e-3;
        let (hx, _, _) = super::swell(3.0 + e, -2.0, 1.5);
        let (hz, _, _) = super::swell(3.0, -2.0 + e, 1.5);
        assert!(((hx - h0) / e - gx).abs() < 0.05);
        assert!(((hz - h0) / e - gz).abs() < 0.05);
    }
}

/// Water sheets split until no edge is longer than `max` (Bevy metres), so the surface can move with the swell
/// (Modern Water). Positions, normals and both UV sets are interpolated.
#[allow(clippy::type_complexity)]
pub fn tessellate(
    pos: &[[f32; 3]],
    nrm: &[[f32; 3]],
    uv: &[[f32; 2]],
    uv1: &[[f32; 2]],
    idx: &[u32],
    max: f32,
) -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<[f32; 2]>, Vec<u32>) {
    let (mut p, mut n, mut u, mut u1) = (pos.to_vec(), nrm.to_vec(), uv.to_vec(), uv1.to_vec());
    let mut out = Vec::new();
    let mut mids: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
    let mut stack: Vec<[u32; 3]> = idx.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect();
    let budget = 400_000usize;
    while let Some(t) = stack.pop() {
        let d = |a: u32, b: u32, p: &Vec<[f32; 3]>| Vec3::from(p[a as usize]).distance(Vec3::from(p[b as usize]));
        let e = [d(t[0], t[1], &p), d(t[1], t[2], &p), d(t[2], t[0], &p)];
        let (k, &l) = e.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap();
        if l <= max || p.len() > budget {
            out.extend_from_slice(&t);
            continue;
        }
        let (a, b, c) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
        let key = (a.min(b), a.max(b));
        let m = *mids.entry(key).or_insert_with(|| {
            let mix3 = |x: [f32; 3], y: [f32; 3]| [(x[0] + y[0]) * 0.5, (x[1] + y[1]) * 0.5, (x[2] + y[2]) * 0.5];
            let mix2 = |x: [f32; 2], y: [f32; 2]| [(x[0] + y[0]) * 0.5, (x[1] + y[1]) * 0.5];
            p.push(mix3(p[a as usize], p[b as usize]));
            n.push(n[a as usize]);
            u.push(mix2(u[a as usize], u[b as usize]));
            if !u1.is_empty() {
                u1.push(mix2(u1[a as usize], u1[b as usize]));
            }
            (p.len() - 1) as u32
        });
        stack.push([a, m, c]);
        stack.push([m, b, c]);
    }
    let out = fix_tjunctions(&mut p, &mut n, &mut u, &mut u1, &out);
    (p, n, u, u1, out)
}

/// Where a vertex of one triangle lies on the middle of another's edge (the map's water faces meet that way, and the
/// splitting above makes many more), the moving surface would open a crack along that edge: such triangles become a
/// fan round their centre through every vertex on their edges, so both sides move together.
#[allow(clippy::type_complexity)]
fn fix_tjunctions(p: &mut Vec<[f32; 3]>, n: &mut Vec<[f32; 3]>, u: &mut Vec<[f32; 2]>, u1: &mut Vec<[f32; 2]>, idx: &[u32]) -> Vec<u32> {
    const CELL: f32 = 2.0;
    let key = |v: [f32; 3]| ((v[0] / CELL).floor() as i32, (v[2] / CELL).floor() as i32);
    // Distinct positions, bucketed (the first vertex at each position stands for it).
    let mut grid: std::collections::HashMap<(i32, i32), Vec<u32>> = std::collections::HashMap::new();
    let mut seen: std::collections::HashSet<(i32, i32, i32)> = std::collections::HashSet::new();
    for (i, v) in p.iter().enumerate() {
        let q = ((v[0] * 1000.0).round() as i32, (v[1] * 1000.0).round() as i32, (v[2] * 1000.0).round() as i32);
        if seen.insert(q) {
            grid.entry(key(*v)).or_default().push(i as u32);
        }
    }
    let mut out = Vec::with_capacity(idx.len());
    for t in idx.chunks_exact(3) {
        let c = [t[0], t[1], t[2]];
        // Vertices on each edge's inside, ordered along it.
        let mut ring: Vec<(u32, [f32; 2], [f32; 2])> = Vec::new();
        let mut extra = false;
        for e in 0..3 {
            let (a, b) = (c[e], c[(e + 1) % 3]);
            ring.push((a, u[a as usize], if u1.is_empty() { [0.0; 2] } else { u1[a as usize] }));
            let (pa, pb) = (Vec3::from(p[a as usize]), Vec3::from(p[b as usize]));
            let d = pb - pa;
            let len2 = d.length_squared();
            if len2 < 1e-8 {
                continue;
            }
            let (lo, hi) = (pa.min(pb), pa.max(pb));
            let (k0, k1) = (key(lo.to_array()), key(hi.to_array()));
            let mut on: Vec<(f32, u32)> = Vec::new();
            for gx in k0.0..=k1.0 {
                for gz in k0.1..=k1.1 {
                    let Some(list) = grid.get(&(gx, gz)) else { continue };
                    for &w in list {
                        let pw = Vec3::from(p[w as usize]);
                        let s = (pw - pa).dot(d) / len2;
                        if s <= 1e-4 || s >= 1.0 - 1e-4 {
                            continue;
                        }
                        if (pa + d * s).distance_squared(pw) < 1e-6 && pw.distance_squared(pa) > 1e-6 && pw.distance_squared(pb) > 1e-6 {
                            on.push((s, w));
                        }
                    }
                }
            }
            on.sort_by(|x, y| x.0.total_cmp(&y.0));
            for (s, w) in on {
                extra = true;
                let lerp2 = |x: [f32; 2], y: [f32; 2]| [x[0] + (y[0] - x[0]) * s, x[1] + (y[1] - x[1]) * s];
                let uv = lerp2(u[a as usize], u[b as usize]);
                let uv1 = if u1.is_empty() { [0.0; 2] } else { lerp2(u1[a as usize], u1[b as usize]) };
                ring.push((w, uv, uv1));
            }
        }
        if !extra {
            out.extend_from_slice(&c);
            continue;
        }
        // New corners (own copies: this face's texture coordinates there), the centre, and the fan.
        let mut ids = Vec::with_capacity(ring.len());
        for (k, (w, uv, uv1)) in ring.iter().enumerate() {
            if k == 0 || c.contains(w) {
                ids.push(*w);
                continue;
            }
            p.push(p[*w as usize]);
            n.push(n[c[0] as usize]);
            u.push(*uv);
            if !u1.is_empty() {
                u1.push(*uv1);
            }
            ids.push((p.len() - 1) as u32);
        }
        let avg3 = |f: &dyn Fn(u32) -> [f32; 3]| {
            let s = c.iter().fold([0.0f32; 3], |a, &i| {
                let v = f(i);
                [a[0] + v[0], a[1] + v[1], a[2] + v[2]]
            });
            [s[0] / 3.0, s[1] / 3.0, s[2] / 3.0]
        };
        let avg2 = |f: &dyn Fn(u32) -> [f32; 2]| {
            let s = c.iter().fold([0.0f32; 2], |a, &i| {
                let v = f(i);
                [a[0] + v[0], a[1] + v[1]]
            });
            [s[0] / 3.0, s[1] / 3.0]
        };
        let cp = avg3(&|i| p[i as usize]);
        let cu = avg2(&|i| u[i as usize]);
        let cu1 = if u1.is_empty() { [0.0; 2] } else { avg2(&|i| u1[i as usize]) };
        p.push(cp);
        n.push(n[c[0] as usize]);
        u.push(cu);
        if !u1.is_empty() {
            u1.push(cu1);
        }
        let ci = (p.len() - 1) as u32;
        for k in 0..ids.len() {
            out.extend_from_slice(&[ci, ids[k], ids[(k + 1) % ids.len()]]);
        }
    }
    out
}

/// The open sea past the map's edge (Modern Water): a ring of water at the height of the map's largest water sheet,
/// from that sheet's bounds out to the horizon, with the same material (reflection, tint, swell) - so the water
/// seems to go on for ever instead of stopping at the edge of the map. Shown only with Modern Water on.
#[derive(Component)]
pub struct OceanRing;

pub fn ocean_ring(
    mirrors: &crate::mirror::Mirrors,
    batches: &[dx_level::Batch],
    sheet_pts: &[[f32; 3]],
) -> Option<(Mesh, Handle<WaterMaterial>)> {
    // The biggest flat water plane, if it's sea-sized (indoor pools and fountains get none).
    let plane = mirrors
        .planes
        .iter()
        .filter(|p| p.water && !p.water_mats.is_empty())
        .max_by(|a, b| {
            let ar = |p: &crate::mirror::Plane| (p.max.x - p.min.x) * (p.max.z - p.min.z);
            ar(a).total_cmp(&ar(b))
        })?;
    let (w, l) = (plane.max.x - plane.min.x, plane.max.z - plane.min.z);
    if w.min(l) < 60.0 {
        return None;
    }
    let y = plane.d;
    // The sheet's texture mapping, continued: uv = A (x, z) + c from one of its triangles.
    let mut map: Option<[f32; 6]> = None;
    'find: for b in batches.iter().filter(|b| b.class == dx_level::SurfaceClass::Water && !b.sky) {
        for t in b.indices.chunks_exact(3) {
            let p: Vec<Vec3> = t.iter().map(|&i| Vec3::from(b.positions[i as usize])).collect();
            if p.iter().any(|q| (q.y - y).abs() > 0.05) {
                continue;
            }
            let uv: Vec<Vec2> = t.iter().map(|&i| Vec2::from(b.uvs[i as usize])).collect();
            let (e1, e2) = (Vec2::new(p[1].x - p[0].x, p[1].z - p[0].z), Vec2::new(p[2].x - p[0].x, p[2].z - p[0].z));
            let det = e1.x * e2.y - e1.y * e2.x;
            if det.abs() < 1e-3 {
                continue;
            }
            let (f1, f2) = (uv[1] - uv[0], uv[2] - uv[0]);
            // [f1 f2] = A [e1 e2]  ->  A = [f1 f2] [e1 e2]^-1
            let inv = [e2.y / det, -e2.x / det, -e1.y / det, e1.x / det];
            let a = [f1.x * inv[0] + f2.x * inv[2], f1.x * inv[1] + f2.x * inv[3], f1.y * inv[0] + f2.y * inv[2], f1.y * inv[1] + f2.y * inv[3]];
            let c = [uv[0].x - a[0] * p[0].x - a[1] * p[0].z, uv[0].y - a[2] * p[0].x - a[3] * p[0].z];
            map = Some([a[0], a[1], c[0], a[2], a[3], c[1]]);
            break 'find;
        }
    }
    let m = map.unwrap_or([0.01, 0.0, 0.0, 0.0, 0.01, 0.0]);
    let uv_at = |x: f32, z: f32| [m[0] * x + m[1] * z + m[2], m[3] * x + m[4] * z + m[5]];
    // Lit like the sheet's nearest edge: each ring vertex takes the lightmap spot of the closest sheet vertex, so the
    // moonlight on the water runs on past the edge without a seam.
    let sheet: Vec<(Vec2, [f32; 2])> = batches
        .iter()
        .filter(|b| b.class == dx_level::SurfaceClass::Water && !b.sky)
        .flat_map(|b| b.positions.iter().zip(b.uvs1.iter()))
        .filter(|(p, _)| (p[1] - y).abs() < 0.05)
        .map(|(p, l)| (Vec2::new(p[0], p[2]), *l))
        .collect();
    let nearest = |v: Vec2| -> [f32; 2] {
        sheet
            .iter()
            .min_by(|a, b| a.0.distance_squared(v).total_cmp(&b.0.distance_squared(v)))
            .map(|s| s.1)
            .unwrap_or([0.0, 0.0])
    };
    // Sampled every 2 m round the sheet's bounds (perimeter position -> lightmap spot), then looked up by where a
    // ring vertex's nearest point on the bounds lies.
    let (blo, bhi) = (Vec2::new(plane.min.x, plane.min.z), Vec2::new(plane.max.x, plane.max.z));
    let (bw, bl) = (bhi.x - blo.x, bhi.y - blo.y);
    let perim = 2.0 * (bw + bl);
    let along = |t: f32| -> Vec2 {
        let t = t.rem_euclid(perim);
        if t < bw {
            Vec2::new(blo.x + t, blo.y)
        } else if t < bw + bl {
            Vec2::new(bhi.x, blo.y + (t - bw))
        } else if t < 2.0 * bw + bl {
            Vec2::new(bhi.x - (t - bw - bl), bhi.y)
        } else {
            Vec2::new(blo.x, bhi.y - (t - 2.0 * bw - bl))
        }
    };
    let samples: Vec<[f32; 2]> = (0..(perim / 2.0).ceil() as usize).map(|k| nearest(along(k as f32 * 2.0))).collect();
    let edge_light = |v: Vec2| -> [f32; 2] {
        let c = v.clamp(blo, bhi);
        // Which side the clamped point is on, and how far along the perimeter.
        let t = if (c.y - blo.y).abs() < 1e-3 {
            c.x - blo.x
        } else if (c.x - bhi.x).abs() < 1e-3 {
            bw + (c.y - blo.y)
        } else if (c.y - bhi.y).abs() < 1e-3 {
            bw + bl + (bhi.x - c.x)
        } else {
            2.0 * bw + bl + (bhi.y - c.y)
        };
        samples.get(((t / 2.0).round() as usize).min(samples.len().saturating_sub(1))).copied().unwrap_or([0.0, 0.0])
    };
    // Bands out from the sheet's edge, wider and wider (fine near the edge so the swell can move it, coarse far out).
    let steps = [0.0f32, 3.0, 7.0, 13.0, 22.0, 36.0, 60.0, 100.0, 170.0, 300.0, 520.0, 900.0, 1600.0, 2800.0];
    let (lo, hi) = (Vec2::new(plane.min.x, plane.min.z), Vec2::new(plane.max.x, plane.max.z));
    let rect = |e: f32| (lo - Vec2::splat(e), hi + Vec2::splat(e));
    // Every band splits each side at the same places - where the (tessellated) sheet has vertices on that edge of its
    // bounds, plus enough more to keep pieces under 2.5 m - so neighbouring bands, and the sheet and the first band,
    // meet vertex to vertex: no T-junctions for the swell to open into cracks.
    let (w, l) = ((hi.x - lo.x).max(1e-3), (hi.y - lo.y).max(1e-3));
    let side_len = [w, l, w, l];
    let mut ts: [Vec<f32>; 4] = [vec![0.0, 1.0], vec![0.0, 1.0], vec![0.0, 1.0], vec![0.0, 1.0]];
    for p in sheet_pts.iter().filter(|p| (p[1] - y).abs() < 0.05) {
        let (x, z) = (p[0], p[2]);
        let on = |a: f32, b: f32| (a - b).abs() < 2e-3;
        if on(z, lo.y) && x >= lo.x && x <= hi.x {
            ts[0].push((x - lo.x) / w);
        }
        if on(x, hi.x) && z >= lo.y && z <= hi.y {
            ts[1].push((z - lo.y) / l);
        }
        if on(z, hi.y) && x >= lo.x && x <= hi.x {
            ts[2].push((hi.x - x) / w);
        }
        if on(x, lo.x) && z >= lo.y && z <= hi.y {
            ts[3].push((hi.y - z) / l);
        }
    }
    for (side, t) in ts.iter_mut().enumerate() {
        t.sort_by(|a, b| a.total_cmp(b));
        t.dedup_by(|a, b| (*a - *b).abs() < 1e-5);
        let mut filled = Vec::with_capacity(t.len() * 2);
        for k in 0..t.len() {
            filled.push(t[k]);
            if let Some(&next) = t.get(k + 1) {
                let gap = (next - t[k]) * side_len[side];
                let n = (gap / 2.5).ceil() as usize;
                for j in 1..n {
                    filled.push(t[k] + (next - t[k]) * j as f32 / n as f32);
                }
            }
        }
        *t = filled;
    }
    let (mut pos, mut nrm, mut uv, mut uv1, mut idx) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::<u32>::new());
    for s in steps.windows(2) {
        let (ia, ib) = (rect(s[0]), rect(s[1]));
        // Corners of the inner and outer rectangles, counter-clockwise: (x0,z0) (x1,z0) (x1,z1) (x0,z1).
        let corners = |r: (Vec2, Vec2)| [Vec2::new(r.0.x, r.0.y), Vec2::new(r.1.x, r.0.y), Vec2::new(r.1.x, r.1.y), Vec2::new(r.0.x, r.1.y)];
        let (ci, co) = (corners(ia), corners(ib));
        for side in 0..4 {
            let (a0, a1) = (ci[side], ci[(side + 1) % 4]);
            let (b0, b1) = (co[side], co[(side + 1) % 4]);
            let t = &ts[side];
            let row = pos.len() as u32;
            for &tk in t {
                for v in [a0.lerp(a1, tk), b0.lerp(b1, tk)] {
                    pos.push([v.x, y, v.y]);
                    nrm.push([0.0, 1.0, 0.0]);
                    uv.push(uv_at(v.x, v.y));
                    uv1.push(edge_light(v));
                }
            }
            for k in 0..t.len().saturating_sub(1) as u32 {
                let (i0, o0, i1, o1) = (row + 2 * k, row + 2 * k + 1, row + 2 * k + 2, row + 2 * k + 3);
                idx.extend([i0, i1, o1, i0, o1, o0]);
            }
        }
    }
    let mut mesh = Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv1);
    mesh.insert_indices(bevy::mesh::Indices::U32(idx));
    Some((mesh, plane.water_mats[0].clone()))
}

/// The map's edge walls (BSP faces with the sky marker texture outside the sky room: drawn black at the water's
/// edge). Hidden while the open sea shows, which would otherwise end at them.
#[derive(Component)]
pub struct EdgeWall;

/// A flat, dark copy of a water sheet (or of the open sea) half a metre under it (Modern Water): where the moving
/// surface's triangles part at a T-junction of the map's water faces, the gap shows dark water instead of the black
/// sky behind. Hidden with the eye under the surface. `y` = the water's height (Bevy).
#[derive(Component)]
pub struct WaterBacking {
    pub y: f32,
}

pub const BACKING_DROP: f32 = 0.5;

/// The backing's mesh: `positions` lowered, with normals; the material is dark and unlit.
pub fn backing_mesh(positions: &[[f32; 3]], indices: &[u32]) -> Mesh {
    let mut m = Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::default());
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions.iter().map(|p| [p[0], p[1] - BACKING_DROP, p[2]]).collect::<Vec<_>>());
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; positions.len()]);
    m.insert_indices(bevy::mesh::Indices::U32(indices.to_vec()));
    m
}

pub fn backing_material() -> StandardMaterial {
    StandardMaterial {
        base_color: Color::linear_rgb(0.03, 0.045, 0.052),
        unlit: true,
        double_sided: true,
        cull_mode: None,
        ..default()
    }
}

/// The open sea shows only with Modern Water (and the edge walls go).
#[allow(clippy::type_complexity)]
pub fn ocean_system(
    mut commands: Commands,
    mut q: Query<&mut Visibility, (With<OceanRing>, Without<EdgeWall>, Without<WaterBacking>)>,
    mut walls: Query<&mut Visibility, (With<EdgeWall>, Without<OceanRing>, Without<WaterBacking>)>,
    mut backs: Query<(&WaterBacking, &mut Visibility), (Without<OceanRing>, Without<EdgeWall>)>,
    cam: Query<(Entity, &Transform, Has<bevy::core_pipeline::prepass::DepthPrepass>), (With<crate::FlyCam>, Without<crate::SkyCam>)>,
) {
    let eye_y = cam.iter().next().map_or(f32::MAX, |t| t.1.translation.y);
    // Modern Water sees how deep the water is from the scene's depth (a depth prepass on the world camera): shallow
    // water shows what's under it, deep water turns to its own colour.
    for (e, _, has) in &cam {
        if modern() && !has {
            commands.entity(e).insert(bevy::core_pipeline::prepass::DepthPrepass);
        } else if !modern() && has && !crate::postfx::ao_on() {
            commands.entity(e).remove::<bevy::core_pipeline::prepass::DepthPrepass>();
        }
    }
    // The dark backings hid cracks in the moving surface; with the depth showing what's really under the water they'd
    // only be a false bottom half a metre down (DXVIEW_WATER_BACKING=1 brings them back).
    let backing = std::env::var_os("DXVIEW_WATER_BACKING").is_some();
    for (b, mut v) in &mut backs {
        let want = if backing && modern() && eye_y > b.y { Visibility::Inherited } else { Visibility::Hidden };
        if *v != want {
            *v = want;
        }
    }
    let on = modern() && !q.is_empty();
    let want = if on { Visibility::Inherited } else { Visibility::Hidden };
    for mut v in &mut q {
        if *v != want {
            *v = want;
        }
    }
    let wall = if on { Visibility::Hidden } else { Visibility::Inherited };
    for mut v in &mut walls {
        if *v != wall {
            *v = wall;
        }
    }
}
