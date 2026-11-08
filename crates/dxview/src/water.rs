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
    /// The wave grid (see `grid_mesh`): x 0 = none, 1 = a sheet / the open sea (gives way to the grid inside its
    /// square), 2 = the grid itself; y the grid's half size (m), z its step (m).
    pub grid: Vec4,
    /// The water mask: its corner (Bevy x, z), texel size (m), and its size in texels (w = width).
    pub mask: Vec4,
    /// Water heights (Bevy y) by the mask's plane index (16).
    pub heights: [Vec4; 4],
    /// Screen-space reflections (Water Reflections = Screen-Space): x 1 = on (the material is drawn opaque in the
    /// transmissive pass, where the scene drawn so far is on hand as a texture: the shader reflects it and blends
    /// the water onto it itself, refracted by the waves), y march steps.
    pub ssr: Vec4,
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct WaterExt {
    #[uniform(100)]
    pub params: WaterParams,
    /// The planar reflection of this water's plane (mirror.rs slots), when Modern Water is on.
    #[texture(101)]
    #[sampler(102)]
    pub reflection: Handle<Image>,
    /// The water mask (R8Uint): bit 7 = water here, bits 0-4 = which height (`heights`); dry texels next to the
    /// water carry its height too so the grid's edge triangles stay level.
    #[texture(103, sample_type = "u_int")]
    pub mask: Handle<Image>,
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
    // Only waves the 2 m mesh can carry smoothly move the surface (and what floats on it): the shorter ones are
    // shading only (they made facets at three vertices per wavelength).
    let storm = storm();
    let waves = WAVES.iter().copied().filter(|w| w.1 >= DISPLACE_MIN_LEN);
    let storm_w = (STORM_WAVE.0, STORM_WAVE.1, STORM_WAVE.2, STORM_WAVE.3 * storm);
    for (ang, len, speed, amp) in waves.chain(std::iter::once(storm_w)) {
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
pub const WAVE_M: f32 = 0.16;
/// A storm's long swell (as WAVES; its amplitude x storm()). Keep in step with water.wgsl storm_wave.
const STORM_WAVE: (f32, f32, f32, f32) = (0.55, 31.0, 3.4, 1.1);

/// How stormy it is, 0..1: the rain (weather.rs), in the steps the water materials are updated in.
pub fn storm() -> f32 {
    if let Some(v) = env_val!("DXVIEW_STORM").and_then(|v| v.parse::<f32>().ok()) {
        return v.clamp(0.0, 1.0);
    }
    ((crate::weather::density() / 14.0).clamp(0.0, 1.0) * 50.0).round() / 50.0
}

/// Metres the surface moves per unit of swell: the calm harbour's WAVE_M, up to 3.5 times that in a storm.
pub fn wave_m() -> f32 {
    WAVE_M * (1.0 + 2.5 * storm())
}
/// Waves shorter than this (m) don't displace vertices (see `swell` / water.wgsl swell_h).
pub const DISPLACE_MIN_LEN: f32 = 12.0;

fn bob_units() -> f64 {
    if modern() { wave_m() as f64 / dx_level::METERS_PER_UNIT as f64 } else { BOB }
}

/// Vertical offset (Unreal units) and tilt (pitch, roll in Unreal rotator units) for something floating at an
/// Unreal-space spot.
pub fn bob(at: DVec3, t: f64) -> (f64, i32, i32) {
    // Classic water has no swell of ours: only DeusExDecoration's own rocking moves floating things.
    if !modern() {
        return (0.0, 0, 0);
    }
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
    /// The global parameters last written (modern, rain, moon, open sea).
    last: Option<[f32; 6]>,
}

/// Enhancement "Modern Water" (setting ModernWater): reflections, a body colour and see-through depth, detailed
/// waves, rain rings - a shader-pack look instead of the original's translucent picture.
pub fn modern() -> bool {
    setting!("ModernWater") > 0.5
}

/// Modern Water's reflections drawn by a second camera under the water (Water Reflections = Planar).
pub fn planar() -> bool {
    modern() && setting!("WaterReflections") > 0.5
}

/// Modern Water's reflections worked out on the GPU from the picture already drawn (Water Reflections =
/// Screen-Space, the default): no second view for the CPU to prepare and record.
pub fn ssr() -> bool {
    modern() && setting!("WaterReflections") < 0.5
}

/// Enhancement "Open Sea" (setting OpenSea, ours; off by default): without Modern Water, the open sea (`ocean_ring`)
/// still runs past the map's edge to the horizon, drawn the Classic way, the edge walls go, and the
/// map's water no longer fades out with distance. Classic alone ends the water 30-120 m from the eye in black.
pub fn open_sea() -> bool {
    crate::settings::get("OpenSea") > 0.5
}

/// Each frame: time and the live rings into every water material.
pub fn water_system(
    time: Res<Time>,
    mut wm: ResMut<WaterMats>,
    mut mats: ResMut<Assets<WaterMaterial>>,
    level: Res<crate::LevelRes>,
    mut moon: Local<(u64, Vec3)>,
) {
    // The waves run on the GPU's own clock (Bevy's globals.time, which wraps every hour like this one): the
    // materials only change when a ring starts or ends or a setting moves, not every frame (each change re-prepares
    // every water material on the render thread).
    let t = time.elapsed_secs_wrapped();
    let mut changed = false;
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
        changed = true;
    }
    for r in wm.rings.iter_mut() {
        if r.w > 0.0 && (t - r.z > 4.0 || t < r.z) {
            r.w = 0.0;
            changed = true;
        }
    }
    let rings = wm.rings;
    let modern = modern();
    let switched = wm.modern != Some(modern);
    wm.modern = Some(modern);
    let open_sea = open_sea();
    let ssr = ssr();
    let key = [
        if modern { 1.0 } else { 0.0 } + if ssr { 2.0 } else { 0.0 },
        storm(),
        moon_dir.x,
        moon_dir.y,
        moon_dir.z,
        if open_sea { 1.0 } else { 0.0 },
    ];
    let switched = switched || wm.last.is_some_and(|l| l[0] != key[0]);
    if wm.last != Some(key) {
        wm.last = Some(key);
        changed = true;
    }
    if !changed && !switched {
        return;
    }
    let rain = key[1];
    for (i, h) in wm.mats.iter().enumerate() {
        if let Some(mut m) = mats.get_mut(h) {
            m.extension.params.t.x = t;
            m.extension.params.rings = rings;
            m.extension.params.m.x = if modern { 1.0 } else { 0.0 };
            m.extension.params.m.z = rain;
            m.extension.params.moon = moon_dir.extend(1.0);
            m.extension.params.far.w = if modern { wave_m() } else { 0.0 };
            // Classic water fades out between far.x and far.y metres from the eye; the open sea goes to the horizon.
            let (near, far) = if open_sea { (1.0e6, 2.0e6) } else { (30.0, 120.0) };
            if m.extension.params.far.x != near {
                m.extension.params.far.x = near;
                m.extension.params.far.y = far;
            }
            if switched {
                let opaque = wm.opaque.get(i).copied().unwrap_or(false);
                m.base.alpha_mode = match (opaque, modern) {
                    (true, _) => AlphaMode::Opaque,
                    // Screen-space: drawn opaque in the transmissive pass, blending onto its own copy of the scene
                    // (Bevy only puts opaque transmissive materials there).
                    (false, true) if ssr => AlphaMode::Opaque,
                    (false, true) => AlphaMode::Blend,
                    (false, false) => AlphaMode::Add,
                };
                m.base.perceptual_roughness = if modern { 0.07 } else { 1.0 };
                m.base.reflectance = if modern { 0.5 } else { 0.05 };
                // Screen-space reflections read the scene drawn so far (Bevy's transmission texture): a material
                // with any specular transmission is drawn in the transmissive pass, after the opaque world is
                // copied (the shader turns the transmission itself off again).
                m.base.specular_transmission = if ssr && !opaque { 1e-3 } else { 0.0 };
                m.extension.params.ssr = Vec4::new(if ssr && !opaque { 1.0 } else { 0.0 }, 28.0, 0.0, 0.0);
            }
        }
    }
}

/// The water material for a level batch (its source picture, translucent). `placeholder` stands in for the
/// reflection until a mirror slot holds this plane.
pub fn material(base: StandardMaterial, opaque: bool, placeholder: Handle<Image>, mask: Handle<Image>) -> WaterMaterial {
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
                    env_val!("DXVIEW_WATER_GAIN").map(str::to_owned).and_then(|v| v.parse().ok()).unwrap_or(1.0),
                    0.03,
                ),
                far: Vec4::new(30.0, 120.0, env_val!("DXVIEW_WATER_DEBUG").map(str::to_owned).and_then(|v| v.parse().ok()).unwrap_or(0.0), 0.0),
                rings: [Vec4::ZERO; RINGS],
                m: Vec4::new(0.0, 0.0, 0.0, env_val!("DXVIEW_WATER_BEND").map(str::to_owned).and_then(|v| v.parse().ok()).unwrap_or(0.03)),
                // Night harbour water: dark blue-green, a fair way see-through looking straight down.
                tint: Vec4::new(0.006, 0.026, 0.040, 0.45),
                moon: Vec4::new(0.3, 0.8, 0.5, 1.0),
                grid: Vec4::ZERO,
                mask: Vec4::ZERO,
                heights: [Vec4::ZERO; 4],
                ssr: Vec4::ZERO,
            },
            reflection: placeholder,
            mask,
        },
    }
}

// ------------------------------------------------------------------------------------------------ the wave grid
//
// Modern Water's waves no longer move the map's own water faces: those are a few huge triangles (split finely
// they were still too coarse in places, and every seam between differently split sheets showed as triangles at the
// wave crests, however the T-junctions were mended). Instead one even grid follows the eye - GRID_R round it, a
// vertex every GRID_STEP - and carries the swell; a mask of where the map has water (its water faces and the open
// sea, rasterised once per map) cuts the grid to the water's shape and gives each vertex its water's height. The
// map's water faces and the open sea stay flat and give way to the grid inside its square, beyond which the swell
// has died away. One level, one step: nothing to stitch, so nothing to crack.

pub const GRID_R: f32 = 150.0;
pub const GRID_STEP: f32 = 1.5;
/// Mask texels (m).
const MASK_TEXEL: f32 = 0.25;

#[derive(Component)]
pub struct WaterGrid;

/// The grid mesh, centred on (0, 0) (the vertex shader moves it to the eye, snapped to GRID_STEP).
pub fn grid_mesh() -> Mesh {
    let n = (2.0 * GRID_R / GRID_STEP).round() as u32;
    let mut pos = Vec::with_capacity(((n + 1) * (n + 1)) as usize);
    for j in 0..=n {
        for i in 0..=n {
            pos.push([-GRID_R + i as f32 * GRID_STEP, 0.0, -GRID_R + j as f32 * GRID_STEP]);
        }
    }
    let count = pos.len();
    let mut idx = Vec::with_capacity((n * n * 6) as usize);
    for j in 0..n {
        for i in 0..n {
            let a = j * (n + 1) + i;
            idx.extend_from_slice(&[a, a + n + 1, a + 1, a + 1, a + n + 1, a + n + 2]);
        }
    }
    let mut m = Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::RENDER_WORLD);
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; count]);
    m.insert_indices(bevy::mesh::Indices::U32(idx));
    m
}

/// A 1 x 1 empty mask (no grid).
pub fn mask_placeholder(images: &mut Assets<Image>) -> Handle<Image> {
    images.add(Image::new(
        bevy::render::render_resource::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        vec![0u8],
        bevy::render::render_resource::TextureFormat::R8Uint,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    ))
}

/// The water mask over the map's flat water faces and the open sea's triangles (Bevy space): the image, its
/// corner + texel + width (WaterParams.mask) and the heights. None when the map has no flat water.
/// Texels: bit 7 = water, bit 6 = the face was solid in the original (Batch::water_opaque: the Battery Park
/// harbour - the wave grid over it is never see-through, or the bare harbour bed under it shows), bits 0-4 = the
/// height's index + 1.
pub fn build_mask(sheets: &[(&[[f32; 3]], &[u32], bool)], ring: Option<(&[[f32; 3]], &[u32])>) -> Option<(Image, Vec4, [Vec4; 4])> {
    let mut heights: Vec<f32> = Vec::new();
    let mut lo = Vec2::splat(f32::MAX);
    let mut hi = Vec2::splat(f32::MIN);
    let mut tris: Vec<([Vec2; 3], u8)> = Vec::new();
    for (pos, idx, solid) in sheets {
        let solid = if *solid { 0x40u8 } else { 0 };
        for t in idx.chunks_exact(3) {
            let p = [pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]];
            // Flat faces only (the grid is level water).
            if (p[0][1] - p[1][1]).abs() > 0.05 || (p[0][1] - p[2][1]).abs() > 0.05 {
                continue;
            }
            let y = p[0][1];
            let k = match heights.iter().position(|h| (h - y).abs() < 0.1) {
                Some(k) => k,
                None if heights.len() < 16 => {
                    heights.push(y);
                    heights.len() - 1
                }
                None => continue,
            };
            let v = p.map(|q| Vec2::new(q[0], q[2]));
            for q in v {
                lo = lo.min(q);
                hi = hi.max(q);
            }
            tris.push((v, (k as u8 + 1) | solid));
        }
    }
    if tris.is_empty() {
        return None;
    }
    // The open sea runs out for kilometres: the mask needs only what the grid can reach from anywhere the player
    // can stand - the map's own water plus a grid's width round it.
    if let Some((pos, idx)) = ring {
        let (rlo, rhi) = (lo - Vec2::splat(GRID_R + 8.0), hi + Vec2::splat(GRID_R + 8.0));
        for t in idx.chunks_exact(3) {
            let p = [pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]];
            let y = p[0][1];
            let Some(k) = heights.iter().position(|h| (h - y).abs() < 0.1) else { continue };
            let v = p.map(|q| Vec2::new(q[0], q[2]));
            if v.iter().all(|q| q.x < rlo.x) || v.iter().all(|q| q.x > rhi.x) || v.iter().all(|q| q.y < rlo.y) || v.iter().all(|q| q.y > rhi.y) {
                continue;
            }
            tris.push((v, k as u8 + 1));
        }
        lo = rlo;
        hi = rhi;
    }
    let size = hi - lo;
    // At most 4096 texels a side (coarser on a huge map).
    let texel = MASK_TEXEL.max(size.x.max(size.y) / 4096.0);
    let (w, h) = (((size.x / texel).ceil() as usize + 2).max(1), ((size.y / texel).ceil() as usize + 2).max(1));
    let origin = lo - Vec2::splat(texel);
    let mut m = vec![0u8; w * h];
    // Scanline fill: each row's span of the triangle at the texel centres.
    for (v, k) in &tris {
        let q = v.map(|p| (p - origin) / texel);
        let ymin = q[0].y.min(q[1].y).min(q[2].y);
        let ymax = q[0].y.max(q[1].y).max(q[2].y);
        if ymax < 0.0 || ymin > h as f32 {
            continue;
        }
        let (r0, r1) = ((ymin - 0.5).ceil().max(0.0) as usize, ((ymax - 0.5).floor().max(-1.0) as i64).min(h as i64 - 1));
        if r1 < 0 {
            continue;
        }
        for y in r0..=(r1 as usize) {
            let cy = y as f32 + 0.5;
            let (mut xa, mut xb) = (f32::MAX, f32::MIN);
            for e in 0..3 {
                let (a, b) = (q[e], q[(e + 1) % 3]);
                if (a.y - cy) * (b.y - cy) > 0.0 || (a.y - b.y).abs() < 1e-9 {
                    if (a.y - cy).abs() < 1e-9 {
                        xa = xa.min(a.x);
                        xb = xb.max(a.x);
                    }
                    continue;
                }
                let t = (cy - a.y) / (b.y - a.y);
                let x = a.x + (b.x - a.x) * t;
                xa = xa.min(x);
                xb = xb.max(x);
            }
            if xa > xb {
                continue;
            }
            let (c0, c1) = ((xa - 0.5).ceil().max(0.0) as usize, ((xb - 0.5).floor() as i64).min(w as i64 - 1));
            if c1 < c0 as i64 {
                continue;
            }
            for x in c0..=(c1 as usize) {
                m[y * w + x] = 0x80 | k;
            }
        }
    }
    // Dry texels next to the water take its height (8 texels out = 2 m, more than a grid step), so a grid triangle
    // across the shore line stays level instead of dropping to the floor of the world.
    // Grown ring by ring from the shore (a frontier, not whole-image passes: the mask can be 4096 square).
    let steps = ((GRID_STEP * 1.5 / texel).ceil() as usize).max(2);
    let mut front: Vec<u32> = Vec::new();
    for y in 0..h {
        for x in 0..w {
            if m[y * w + x] == 0 {
                continue;
            }
            let edge = (x > 0 && m[y * w + x - 1] == 0)
                || (x + 1 < w && m[y * w + x + 1] == 0)
                || (y > 0 && m[(y - 1) * w + x] == 0)
                || (y + 1 < h && m[(y + 1) * w + x] == 0);
            if edge {
                front.push((y * w + x) as u32);
            }
        }
    }
    let mut next: Vec<u32> = Vec::new();
    for _ in 0..steps {
        next.clear();
        for &i in &front {
            let i = i as usize;
            let (x, y) = (i % w, i / w);
            let v = m[i] & 0x1F;
            let mut try_set = |j: usize, m: &mut Vec<u8>| {
                if m[j] == 0 {
                    m[j] = v;
                    next.push(j as u32);
                }
            };
            if x > 0 {
                try_set(i - 1, &mut m);
            }
            if x + 1 < w {
                try_set(i + 1, &mut m);
            }
            if y > 0 {
                try_set(i - w, &mut m);
            }
            if y + 1 < h {
                try_set(i + w, &mut m);
            }
        }
        std::mem::swap(&mut front, &mut next);
        if front.is_empty() {
            break;
        }
    }
    let mut hs = [Vec4::ZERO; 4];
    for (k, y) in heights.iter().enumerate() {
        hs[(k + 1) / 4][(k + 1) % 4] = *y;
    }
    let img = Image::new(
        bevy::render::render_resource::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        m,
        bevy::render::render_resource::TextureFormat::R8Uint,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    Some((img, Vec4::new(origin.x, origin.y, texel, w as f32), hs))
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

/// The open sea past the map's edge (Modern Water): a ring of water at the height of the map's largest water sheet,
/// from that sheet's bounds out to the horizon, with the same material (reflection, tint, swell) - so the water
/// seems to go on for ever instead of stopping at the edge of the map. Shown only with Modern Water on.
#[derive(Component)]
pub struct OceanRing;

pub fn ocean_ring(
    mirrors: &crate::mirror::Mirrors,
    batches: &[dx_level::Batch],
    sheet_pts: &[[f32; 3]],
    edge_walls: &[[Vec3; 3]],
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
    // Which sides of the sheet run out to the map's edge: the edge walls (sky-marker faces outside the sky room)
    // standing in the water along that side. Only those sides get open sea; a map with none (indoor pools, harbours
    // walled in by land) gets no ring at all.
    let open = open_sides(plane.min, plane.max, y, edge_walls);
    if !open.iter().any(|o| *o) {
        return None;
    }
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
                // (Flat now - the wave grid carries the swell - so long pieces are fine.)
                let n = (gap / 40.0).ceil() as usize;
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
        for side in (0..4).filter(|&k| open[k]) {
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

/// The sides of a water sheet's bounds (0: -z, 1: +x, 2: +z, 3: -x; as `ocean_ring` walks them) that end at the map's
/// edge walls: a wall triangle reaching the water's height within 30 m of the side, lying along it.
fn open_sides(min: Vec3, max: Vec3, y: f32, walls: &[[Vec3; 3]]) -> [bool; 4] {
    const NEAR: f32 = 30.0;
    let mut open = [false; 4];
    for t in walls {
        let (lo_y, hi_y) = (t[0].y.min(t[1].y).min(t[2].y), t[0].y.max(t[1].y).max(t[2].y));
        if lo_y > y + 1.0 || hi_y < y - 1.0 {
            continue;
        }
        let c = (t[0] + t[1] + t[2]) / 3.0;
        let along_x = c.x > min.x - NEAR && c.x < max.x + NEAR;
        let along_z = c.z > min.z - NEAR && c.z < max.z + NEAR;
        if along_x && (c.z - min.z).abs() < NEAR {
            open[0] = true;
        }
        if along_z && (c.x - max.x).abs() < NEAR {
            open[1] = true;
        }
        if along_x && (c.z - max.z).abs() < NEAR {
            open[2] = true;
        }
        if along_z && (c.x - min.x).abs() < NEAR {
            open[3] = true;
        }
    }
    open
}

/// The map's edge walls (BSP faces with the sky marker texture outside the sky room: drawn black at the water's
/// edge). Hidden while the open sea shows, which would otherwise end at them.
#[derive(Component)]
pub struct EdgeWall;

/// A flat, dark copy of a water sheet (or of the open sea) BACKING_DROP under it (below a storm's troughs) (Modern Water): where the moving
/// surface's triangles part at a T-junction of the map's water faces, the gap shows dark water instead of the black
/// sky behind. Hidden with the eye under the surface. `y` = the water's height (Bevy).
#[derive(Component)]
pub struct WaterBacking {
    pub y: f32,
}

pub const BACKING_DROP: f32 = 1.4;

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
        base_color: if env_on!("DXVIEW_BACKING_DEBUG") { Color::linear_rgb(1.0, 0.0, 1.0) } else { Color::linear_rgb(0.004, 0.012, 0.018) },
        unlit: true,
        double_sided: true,
        cull_mode: None,
        ..default()
    }
}

/// The open sea shows only with Modern Water or Open Sea (and the edge walls go).
#[allow(clippy::type_complexity)]
pub fn ocean_system(
    mut commands: Commands,
    mut q: Query<&mut Visibility, (With<OceanRing>, Without<EdgeWall>, Without<WaterBacking>, Without<WaterGrid>)>,
    mut walls: Query<&mut Visibility, (With<EdgeWall>, Without<OceanRing>, Without<WaterBacking>, Without<WaterGrid>)>,
    mut backs: Query<(&WaterBacking, &mut Visibility), (Without<OceanRing>, Without<EdgeWall>, Without<WaterGrid>)>,
    mut grids: Query<&mut Visibility, (With<WaterGrid>, Without<OceanRing>, Without<EdgeWall>, Without<WaterBacking>)>,
    cam: Query<(Entity, &Transform, Has<bevy::core_pipeline::prepass::DepthPrepass>), (With<crate::FlyCam>, Without<crate::SkyCam>)>,
) {
    let eye_y = cam.iter().next().map_or(f32::MAX, |t| t.1.translation.y);
    // Modern Water sees how deep the water is from the scene's depth (a depth prepass on the world camera): shallow
    // water shows what's under it, deep water turns to its own colour.
    // Attached once Modern Water is on now or in the Modern mix Classic / Modern remembers, then kept for the session
    // (see postfx.rs: taking it off while SSAO was on crashed Bevy, and adding it recompiles every material - the
    // switch's wipe waits for that). Lean Rendering wants it too, for what it saves: with the depth laid down first,
    // the opaque pass shades only what ends up in front.
    static WANTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if modern() || setting!("Modern.ModernWater") > 0.5 || crate::perf::lean_rendering() {
        WANTED.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    for (e, _, has) in &cam {
        if !has && WANTED.load(std::sync::atomic::Ordering::Relaxed) {
            commands.entity(e).insert(bevy::core_pipeline::prepass::DepthPrepass);
        }
    }
    // The dark backings catch cracks left where the moving surface's pieces part (DXVIEW_WATER_BACKING=0 hides them).
    let backing = env_val!("DXVIEW_WATER_BACKING") != Some("0");
    for (b, mut v) in &mut backs {
        let want = if backing && modern() && eye_y > b.y { Visibility::Inherited } else { Visibility::Hidden };
        if *v != want {
            *v = want;
        }
    }
    let gwant = if modern() { Visibility::Inherited } else { Visibility::Hidden };
    for mut v in &mut grids {
        if *v != gwant {
            *v = gwant;
        }
    }
    let on = (modern() || open_sea()) && !q.is_empty();
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
