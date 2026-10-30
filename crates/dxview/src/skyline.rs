//! Enhancements (3D Skyline and Night Clouds, ROADMAP v2 #33). Ours, off by default.
//!
//! - 3D Skyline: on the city maps, procedural buildings stand beyond the edges of the map - New York's towers,
//!   Hong Kong's denser and taller ones, Paris's low blocks - in clusters that rise and fall around the horizon,
//!   dark against the sky with scattered lit windows and the odd red light on a roof, like the skyline photos in the
//!   original sky boxes. They are outside the level, so they only show through the open sky, never through walls.
//! - Night Clouds: a slow cloud layer high over the open-air maps, faintly lit from below by the city.
//!
//! Each map's buildings come from a seed of its name, so they're the same every visit.

use crate::LevelRes;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor, ImageFilterMode};

pub fn skyline_on() -> bool {
    crate::settings::get("Skyline") > 0.5
}

pub fn clouds_on() -> bool {
    crate::settings::get("Clouds") > 0.5
}

#[derive(Clone, Copy, PartialEq)]
enum City {
    NewYork,
    HongKong,
    Paris,
}

/// Which skyline a map gets (file name without .dx). Indoor maps get none (no sky, so nothing would show anyway).
fn city(map: &str) -> Option<City> {
    let m = map.to_ascii_lowercase();
    let m = m.trim_end_matches(".dx");
    if m.starts_with("00_") || m.contains("mj12lab") || m.starts_with("99_") {
        return None;
    }
    if ["01_", "02_", "03_", "04_", "05_", "08_", "09_"].iter().any(|p| m.starts_with(p)) {
        return Some(City::NewYork);
    }
    if m.starts_with("06_hongkong") {
        return Some(City::HongKong);
    }
    if m.starts_with("10_paris") || m.starts_with("11_paris") {
        return Some(City::Paris);
    }
    None
}

/// Clouds over every map open to the sky except the sea floor.
fn cloudy(map: &str) -> bool {
    let m = map.to_ascii_lowercase();
    !(m.starts_with("14_oceanlab") || m.starts_with("00_intro") || m == "dx" || m == "dx.dx")
}

#[derive(Component)]
pub struct SkylinePart;

#[derive(Component)]
pub struct CloudLayer;

fn rnd(seed: &mut u64) -> f32 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    ((*seed >> 40) as f32) / (1u64 << 24) as f32
}

fn hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

/// The window atlas: rows of windows on a dark facade, some lit (warm or cool), one image for all faces; each
/// building samples a random part of it, so no two look alike. Tiles across (repeat).
fn windows_image(c: City, seed: &mut u64) -> Image {
    const W: u32 = 256;
    const H: u32 = 256;
    let mut px = vec![0u8; (W * H * 4) as usize];
    let (lit_p, warm) = match c {
        City::NewYork => (0.22, [255.0, 200.0, 120.0]),
        City::HongKong => (0.32, [220.0, 235.0, 255.0]),
        City::Paris => (0.28, [255.0, 190.0, 110.0]),
    };
    // 4 x 8 texel cells: a 2 x 4 window in each.
    for cy in 0..(H / 8) {
        for cx in 0..(W / 4) {
            let lit = rnd(seed) < lit_p;
            let tone = if rnd(seed) < 0.25 { [180.0, 200.0, 230.0] } else { warm };
            let k = 0.45 + 0.55 * rnd(seed);
            for y in 0..8 {
                for x in 0..4 {
                    let o = (((cy * 8 + y) * W + cx * 4 + x) * 4) as usize;
                    let window = (1..3).contains(&x) && (2..6).contains(&y);
                    let v = if window && lit {
                        [tone[0] * k, tone[1] * k, tone[2] * k]
                    } else if window {
                        [10.0, 11.0, 14.0]
                    } else {
                        [4.0, 4.0, 5.0]
                    };
                    px[o] = v[0] as u8;
                    px[o + 1] = v[1] as u8;
                    px[o + 2] = v[2] as u8;
                    px[o + 3] = 255;
                }
            }
        }
    }
    let mut img = Image::new(
        Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        TextureDimension::D2,
        px,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..ImageSamplerDescriptor::linear()
    });
    img
}

/// A tiling cloud cover: fBm value noise, white with alpha.
fn cloud_image(seed: u64) -> Image {
    const N: usize = 256;
    let mut s = seed | 1;
    let grids: Vec<(usize, Vec<f32>)> = [8usize, 16, 32, 64]
        .iter()
        .map(|&g| (g, (0..g * g).map(|_| rnd(&mut s)).collect()))
        .collect();
    let sample = |g: usize, v: &[f32], x: f32, y: f32| {
        let (fx, fy) = (x * g as f32, y * g as f32);
        let (x0, y0) = (fx.floor() as usize % g, fy.floor() as usize % g);
        let (x1, y1) = ((x0 + 1) % g, (y0 + 1) % g);
        let (tx, ty) = (fx.fract(), fy.fract());
        let (tx, ty) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
        let a = v[y0 * g + x0] + (v[y0 * g + x1] - v[y0 * g + x0]) * tx;
        let b = v[y1 * g + x0] + (v[y1 * g + x1] - v[y1 * g + x0]) * tx;
        a + (b - a) * ty
    };
    let mut px = Vec::with_capacity(N * N * 4);
    for y in 0..N {
        for x in 0..N {
            let (u, v) = (x as f32 / N as f32, y as f32 / N as f32);
            let mut n = 0.0;
            let mut amp = 0.5;
            for (g, vals) in &grids {
                n += sample(*g, vals, u, v) * amp;
                amp *= 0.5;
            }
            let a = ((n - 0.42) * 3.2).clamp(0.0, 1.0);
            let shade = (190.0 + 65.0 * (n - 0.4).clamp(0.0, 1.0)) as u8;
            px.extend([shade, shade, shade, (a * 255.0) as u8]);
        }
    }
    let mut img = Image::new(
        Extent3d { width: N as u32, height: N as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        px,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        ..ImageSamplerDescriptor::linear()
    });
    img
}

/// One box building: four walls (windowed) and a dark roof. `uv0` picks its part of the window atlas.
fn add_box(
    pos: &mut Vec<[f32; 3]>,
    uv: &mut Vec<[f32; 2]>,
    col: &mut Vec<[f32; 4]>,
    idx: &mut Vec<u32>,
    c: Vec3,
    half: Vec2,
    yaw: f32,
    h: f32,
    uv0: Vec2,
    shade: f32,
) {
    let (s, co) = yaw.sin_cos();
    let rot = |x: f32, z: f32| Vec3::new(c.x + x * co - z * s, 0.0, c.z + x * s + z * co);
    let corners = [
        rot(-half.x, -half.y),
        rot(half.x, -half.y),
        rot(half.x, half.y),
        rot(-half.x, half.y),
    ];
    // Windows are 2 m wide and 3.5 m a floor: atlas texels at 4 per window column, 8 per floor.
    let per_u = 1.0 / (2.0 * 64.0);
    let per_v = 1.0 / (3.5 * 32.0);
    let mut u_run = 0.0;
    for k in 0..4 {
        let a = corners[k];
        let b = corners[(k + 1) % 4];
        let w = a.distance(b);
        let q = pos.len() as u32;
        let (u0, u1) = (uv0.x + u_run * per_u, uv0.x + (u_run + w) * per_u);
        let (v0, v1) = (uv0.y + h * per_v, uv0.y);
        pos.extend([
            [a.x, c.y, a.z],
            [b.x, c.y, b.z],
            [b.x, c.y + h, b.z],
            [a.x, c.y + h, a.z],
        ]);
        uv.extend([[u0, v0], [u1, v0], [u1, v1], [u0, v1]]);
        // A little darker on two sides (no lights to shade them otherwise).
        let sh = if k % 2 == 0 { shade } else { shade * 0.8 };
        col.extend([[sh, sh, sh, 1.0]; 4]);
        idx.extend([q, q + 1, q + 2, q, q + 2, q + 3]);
        u_run += w;
    }
    // Roof: a dark wall-free cell of the atlas.
    let q = pos.len() as u32;
    for p in corners {
        pos.push([p.x, c.y + h, p.z]);
        uv.push([0.0, 0.0]);
        col.push([0.15, 0.15, 0.17, 1.0]);
    }
    idx.extend([q, q + 2, q + 1, q, q + 3, q + 2]);
}

#[derive(Default)]
pub struct SkyState {
    built_for: u64,
    clouds: Option<Handle<StandardMaterial>>,
    beacons: Vec<(Vec3, Handle<StandardMaterial>)>,
}

#[allow(clippy::too_many_arguments)]
pub fn skyline_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    mut st: Local<SkyState>,
    mut map_gen: Local<u64>,
    parts: Query<Entity, Or<(With<SkylinePart>, With<CloudLayer>)>>,
    (mut meshes, mut materials, mut images): (ResMut<Assets<Mesh>>, ResMut<Assets<StandardMaterial>>, ResMut<Assets<Image>>),
    mut last: Local<(bool, bool)>,
) {
    let lvl = &level.0;
    let fresh = crate::loader::new_map(&mut map_gen);
    let want = (skyline_on(), clouds_on());
    if fresh || want != *last {
        *last = want;
        for e in &parts {
            commands.entity(e).despawn();
        }
        st.clouds = None;
        st.beacons.clear();
        st.built_for = 0;
        let outdoors = lvl.zone_sky.iter().any(|s| *s);
        if !outdoors {
            return;
        }
        let (bmin, bmax) = (Vec3::from(lvl.bounds_min), Vec3::from(lvl.bounds_max));
        if bmin.x > bmax.x {
            return;
        }
        let centre = (bmin + bmax) * 0.5;
        let half = ((bmax.x - bmin.x).max(bmax.z - bmin.z) * 0.5).max(60.0);
        let ground = bmin.y;
        let mut seed = hash(&lvl.name) | 1;
        if want.0 {
            if let Some(c) = city(&lvl.name) {
                build_skyline(&mut commands, &mut meshes, &mut materials, &mut images, &mut st, c, centre, half, ground, &mut seed);
            }
        }
        if want.1 && cloudy(&lvl.name) {
            let img = images.add(cloud_image(seed ^ 0xC10D));
            // Lit from below by the city: a dull orange-grey over New York and Hong Kong, grey elsewhere.
            let tint = match city(&lvl.name) {
                Some(City::NewYork) | Some(City::HongKong) => Color::srgba(0.16, 0.13, 0.12, 0.85),
                Some(City::Paris) => Color::srgba(0.13, 0.12, 0.12, 0.8),
                None => Color::srgba(0.08, 0.09, 0.11, 0.8),
            };
            let mat = materials.add(StandardMaterial {
                base_color: tint,
                base_color_texture: Some(img),
                unlit: true,
                double_sided: true,
                cull_mode: None,
                alpha_mode: AlphaMode::Blend,
                fog_enabled: false,
                ..default()
            });
            // A grid sheet high above, fading to nothing at its rim (vertex alpha).
            let size = (half * 30.0).clamp(3000.0, 12000.0);
            let n = 24;
            let mut pos = Vec::new();
            let mut uv = Vec::new();
            let mut col = Vec::new();
            let mut idx = Vec::new();
            for j in 0..=n {
                for i in 0..=n {
                    let (fx, fz) = (i as f32 / n as f32 * 2.0 - 1.0, j as f32 / n as f32 * 2.0 - 1.0);
                    let r = (fx * fx + fz * fz).sqrt();
                    let a = (1.0 - r).clamp(0.0, 1.0).powf(0.7);
                    pos.push([centre.x + fx * size, 0.0, centre.z + fz * size]);
                    uv.push([fx * 6.0, fz * 6.0]);
                    col.push([1.0, 1.0, 1.0, a]);
                }
            }
            for j in 0..n {
                for i in 0..n {
                    let k = (j * (n + 1) + i) as u32;
                    let w = (n + 1) as u32;
                    idx.extend([k, k + w, k + 1, k + 1, k + w, k + w + 1]);
                }
            }
            let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
            m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
            m.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
            m.insert_indices(Indices::U32(idx));
            let height = bmax.y + (half * 1.2).clamp(150.0, 600.0);
            commands.spawn((
                Mesh3d(meshes.add(m)),
                MeshMaterial3d(mat.clone()),
                Transform::from_xyz(0.0, height, 0.0),
                CloudLayer,
                bevy::light::NotShadowCaster,
                bevy::light::NotShadowReceiver,
            ));
            st.clouds = Some(mat);
        }
    }
    // Clouds drift; roof beacons blink.
    let t = time.elapsed_secs();
    if let Some(m) = st.clouds.as_ref().and_then(|h| materials.get_mut(h)) {
        let mut m = m;
        m.uv_transform = bevy::math::Affine2::from_translation(Vec2::new((t * 0.0015).fract(), (t * 0.0006).fract()));
    }
    for (k, (_, h)) in st.beacons.iter().enumerate() {
        let on = ((t * 0.7 + k as f32 * 0.37) % 1.0) < 0.5;
        if let Some(mut m) = materials.get_mut(h) {
            let want = if on { Color::srgb(1.0, 0.1, 0.05) } else { Color::srgb(0.12, 0.0, 0.0) };
            if m.base_color != want {
                m.base_color = want;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_skyline(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    st: &mut SkyState,
    c: City,
    centre: Vec3,
    half: f32,
    ground: f32,
    seed: &mut u64,
) {
    let img = images.add(windows_image(c, seed));
    let mat = materials.add(StandardMaterial {
        base_color_texture: Some(img),
        unlit: true,
        ..default()
    });
    // Ring: from just past the map to a long way out. (min distance beyond the edge, depth, floors min..max)
    let (gap, depth, lo, hi, count) = match c {
        City::NewYork => (350.0, 1600.0, 12.0, 120.0, 420),
        City::HongKong => (250.0, 1300.0, 18.0, 160.0, 520),
        City::Paris => (250.0, 1200.0, 6.0, 22.0, 520),
    };
    // Clusters: a few azimuths where the city rises (downtown), the rest lower.
    let peaks: Vec<(f32, f32)> = (0..4).map(|_| (rnd(seed) * std::f32::consts::TAU, 0.5 + rnd(seed))).collect();
    let mut pos = Vec::new();
    let mut uv = Vec::new();
    let mut col = Vec::new();
    let mut idx = Vec::new();
    let beacon_mesh = meshes.add(Sphere::new(1.6).mesh().ico(1).unwrap());
    for _ in 0..count {
        let az = rnd(seed) * std::f32::consts::TAU;
        let dist = half + gap + depth * rnd(seed).powf(0.7);
        let mut rise = 0.25f32;
        for (pa, pw) in &peaks {
            let d = ((az - pa + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI).abs();
            rise = rise.max(pw * (-d * d * 6.0).exp());
        }
        let h = (lo + (hi - lo) * rise * rnd(seed).powf(1.5)).max(lo) * if c == City::Paris { 1.0 } else { 1.0 + rise };
        let w = 12.0 + rnd(seed) * if c == City::Paris { 40.0 } else { 25.0 };
        let d = 12.0 + rnd(seed) * 25.0;
        let at = Vec3::new(centre.x + az.cos() * dist, ground - 5.0, centre.z + az.sin() * dist);
        let yaw = az + (rnd(seed) - 0.5) * 0.4;
        let uv0 = Vec2::new(rnd(seed), rnd(seed));
        add_box(&mut pos, &mut uv, &mut col, &mut idx, at, Vec2::new(w * 0.5, d * 0.5), yaw, h, uv0, 0.85 + rnd(seed) * 0.15);
        // A blinking red light on the tall ones.
        if h > hi * 0.7 && rnd(seed) < 0.5 && st.beacons.len() < 24 {
            let bm = materials.add(StandardMaterial { base_color: Color::srgb(1.0, 0.1, 0.05), unlit: true, ..default() });
            let p = Vec3::new(at.x, at.y + h + 2.0, at.z);
            commands.spawn((Mesh3d(beacon_mesh.clone()), MeshMaterial3d(bm.clone()), Transform::from_translation(p), SkylinePart, bevy::light::NotShadowCaster));
            st.beacons.push((p, bm));
        }
    }
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
    m.insert_indices(Indices::U32(idx));
    commands.spawn((
        Mesh3d(meshes.add(m)),
        MeshMaterial3d(mat),
        Transform::default(),
        SkylinePart,
        bevy::light::NotShadowCaster,
        bevy::light::NotShadowReceiver,
    ));
}
