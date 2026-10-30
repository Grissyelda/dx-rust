//! Enhancements (3D Skyline and Night Clouds, ROADMAP v2 #33). Ours, off by default.
//!
//! - 3D Skyline: on the city maps, procedural buildings stand beyond the edges of the map - New York's towers,
//!   Hong Kong's denser and taller ones, Paris's low blocks - in clusters that rise and fall around the horizon,
//!   dark against the sky with scattered lit windows and the odd red light on a roof, like the skyline photos in the
//!   original sky boxes. They are outside the level, so they only show through the open sky, never through walls.
//! - Night Clouds: see clouds.rs (volumetric). `cloudy` picks the maps.
//!
//! Each map's buildings come from a seed of its name, so they're the same every visit.
//!
//! Where the map's sky room has masked skyline photos (the NYskybox panels of the New York maps), the skyline is
//! built from those instead: the photos are unrolled into one panorama as seen from the sky room's viewpoint, cut
//! into buildings by their silhouette, and each piece stands as a flat slab at its own distance from the map (side
//! walls joining slabs at different depths) - the original picture, seen from the centre of the map exactly as
//! before, but as low-poly 3D: it shifts as you move, and the sky room's flat panels are hidden.

use crate::LevelRes;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor, ImageFilterMode};

pub fn skyline_on() -> bool {
    crate::settings::get("Skyline") > 0.5
}

#[derive(Clone, Copy, PartialEq)]
pub enum City {
    NewYork,
    HongKong,
    Paris,
}

/// Which skyline a map gets (file name without .dx). Indoor maps get none (no sky, so nothing would show anyway).
pub fn city(map: &str) -> Option<City> {
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
pub fn cloudy(map: &str) -> bool {
    let m = map.to_ascii_lowercase();
    !(m.starts_with("14_oceanlab") || m.starts_with("00_intro") || m.starts_with("99_endgame") || m == "dx" || m == "dx.dx")
}

#[derive(Component)]
pub struct SkylinePart;


/// A masked skyline photo panel in the sky room (main.rs tags them): hidden while the photo skyline stands in.
#[derive(Component)]
pub struct SkyPanel;

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
    beacons: Vec<(Vec3, Handle<StandardMaterial>)>,
}

#[allow(clippy::too_many_arguments)]
pub fn skyline_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    mut st: Local<SkyState>,
    mut map_gen: Local<u64>,
    parts: Query<Entity, With<SkylinePart>>,
    mut panels: Query<&mut Visibility, With<SkyPanel>>,
    (mut meshes, mut materials, mut images): (ResMut<Assets<Mesh>>, ResMut<Assets<StandardMaterial>>, ResMut<Assets<Image>>),
    mut last: Local<(bool, bool)>,
) {
    let lvl = &level.0;
    let fresh = crate::loader::new_map(&mut map_gen);
    let want = (skyline_on(), false);
    if fresh || want != *last {
        *last = want;
        for e in &parts {
            commands.entity(e).despawn();
        }
        st.beacons.clear();
        st.built_for = 0;
        let outdoors = lvl.zone_sky.iter().any(|s| *s);
        // The intro and the endings: one map holding many scenes, whose sky-room backdrops stand in for walls and
        // distant streets; a skyline ring round the whole map showed through those "walls".
        let m = lvl.name.to_ascii_lowercase();
        if !outdoors || m.starts_with("00_intro") || m.starts_with("99_endgame") {
            for mut v in &mut panels {
                *v = Visibility::Inherited;
            }
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
        let mut photo = false;
        if want.0 {
            photo = build_photo_skyline(&mut commands, &mut meshes, &mut materials, &mut images, lvl, centre, half, &mut seed);
            if !photo {
                if let Some(c) = city(&lvl.name) {
                    build_skyline(&mut commands, &mut meshes, &mut materials, &mut images, &mut st, c, centre, half, ground, &mut seed);
                }
            }
        }
        for mut v in &mut panels {
            *v = if photo { Visibility::Hidden } else { Visibility::Inherited };
        }
    }
    // Roof beacons blink.
    let t = time.elapsed_secs();
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
        fog_enabled: false,
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

/// One opaque texel's colour from a level texture (mip 0, wrapping), or None where it's see-through.
fn texel(t: &dx_level::TextureImage, rgba: &[u8], u: f32, v: f32) -> [u8; 4] {
    // Bilinear (alpha-weighted, so masked holes don't bleed dark): the panorama samples the panels finer than their
    // texels.
    let (w, h) = (t.width as usize, t.height as usize);
    let fx = u.rem_euclid(1.0) * w as f32 - 0.5;
    let fy = v.rem_euclid(1.0) * h as f32 - 0.5;
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let at = |x: f32, y: f32| {
        let xi = (x as i64).rem_euclid(w as i64) as usize;
        let yi = (y as i64).rem_euclid(h as i64) as usize;
        let o = (yi * w + xi) * 4;
        [rgba[o] as f32, rgba[o + 1] as f32, rgba[o + 2] as f32, rgba[o + 3] as f32]
    };
    let mut c = [0.0f32; 3];
    let mut a = 0.0f32;
    for (x, y, k) in [(x0, y0, (1.0 - tx) * (1.0 - ty)), (x0 + 1.0, y0, tx * (1.0 - ty)), (x0, y0 + 1.0, (1.0 - tx) * ty), (x0 + 1.0, y0 + 1.0, tx * ty)] {
        let p = at(x, y);
        let wa = k * p[3] / 255.0;
        for i in 0..3 {
            c[i] += p[i] * wa;
        }
        a += wa;
    }
    if a <= 1e-4 {
        return [0, 0, 0, 0];
    }
    [(c[0] / a) as u8, (c[1] / a) as u8, (c[2] / a) as u8, (a * 255.0) as u8]
}

fn mip0_rgba(t: &dx_level::TextureImage) -> Option<Vec<u8>> {
    let m = t.mips.first()?;
    if t.bc1 {
        dx_level::dds::decode_level(dx_level::dds::Format::Bc1, m, t.width, t.height)
    } else if t.bc3 {
        dx_level::dds::decode_level(dx_level::dds::Format::Bc3, m, t.width, t.height)
    } else {
        Some(m.clone())
    }
}

/// The sky room's masked photo panels unrolled into a panorama (columns = azimuth all round, rows = elevation from
/// `e_hi` down to `e_lo`), as seen from the sky room's viewpoint.
pub struct Panorama {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
    pub e_lo: f32,
    pub e_hi: f32,
}

pub fn panorama(lvl: &dx_level::Level) -> Option<Panorama> {
    let origin = Vec3::from(lvl.sky_origin?);
    struct Tri {
        p: [Vec3; 3],
        uv: [Vec2; 3],
        tex: usize,
    }
    let mut tris = Vec::new();
    let mut decoded: std::collections::HashMap<usize, Vec<u8>> = Default::default();
    let (mut e_lo, mut e_hi) = (f32::MAX, f32::MIN);
    for b in lvl.batches.iter().filter(|b| b.sky && b.class == dx_level::SurfaceClass::Masked) {
        let Some(ti) = b.texture else { continue };
        if !decoded.contains_key(&ti) {
            let Some(px) = mip0_rgba(&lvl.textures[ti]) else { continue };
            decoded.insert(ti, px);
        }
        for t in b.indices.chunks_exact(3) {
            let p = [0, 1, 2].map(|k| Vec3::from(b.positions[t[k] as usize]) - origin);
            let uv = [0, 1, 2].map(|k| Vec2::from(b.uvs[t[k] as usize]));
            for q in p {
                let e = q.y.atan2(Vec2::new(q.x, q.z).length());
                e_lo = e_lo.min(e);
                e_hi = e_hi.max(e);
            }
            tris.push(Tri { p, uv, tex: ti });
        }
    }
    if tris.is_empty() || e_hi <= e_lo {
        return None;
    }
    // Twice the old 2048 x 192: the skyline is the backdrop of every outdoor view (and the clouds made its
    // blockiness plain). Rows are traced on all cores.
    let (w, h) = (4096usize, 384usize);
    let mut rgba = vec![0u8; w * h * 4];
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16);
    let rows_per = h.div_ceil(threads);
    std::thread::scope(|sc| {
        for (chunk_i, chunk) in rgba.chunks_mut(rows_per * w * 4).enumerate() {
            let (tris, decoded, lvl) = (&tris, &decoded, &lvl);
            sc.spawn(move || {
                let r0 = chunk_i * rows_per;
                for (ri, row) in chunk.chunks_mut(w * 4).enumerate() {
                    let r = r0 + ri;
                    let e = e_hi - (r as f32 + 0.5) / h as f32 * (e_hi - e_lo);
                    for c in 0..w {
                        let a = (c as f32 + 0.5) / w as f32 * std::f32::consts::TAU;
                        let d = Vec3::new(e.cos() * a.cos(), e.sin(), e.cos() * a.sin());
                        // Nearest opaque hit (a see-through texel lets a farther panel show).
                        let mut best: Option<(f32, [u8; 4])> = None;
                        for t in tris.iter() {
                            let (e1, e2) = (t.p[1] - t.p[0], t.p[2] - t.p[0]);
                            let pv = d.cross(e2);
                            let det = e1.dot(pv);
                            if det.abs() < 1e-8 {
                                continue;
                            }
                            let inv = 1.0 / det;
                            let tv = -t.p[0];
                            let u = tv.dot(pv) * inv;
                            if !(0.0..=1.0).contains(&u) {
                                continue;
                            }
                            let qv = tv.cross(e1);
                            let v = d.dot(qv) * inv;
                            if v < 0.0 || u + v > 1.0 {
                                continue;
                            }
                            let dist = e2.dot(qv) * inv;
                            if dist <= 0.0 || best.is_some_and(|b| b.0 <= dist) {
                                continue;
                            }
                            let uv = t.uv[0] * (1.0 - u - v) + t.uv[1] * u + t.uv[2] * v;
                            let px = texel(&lvl.textures[t.tex], &decoded[&t.tex], uv.x, uv.y);
                            if px[3] > 127 {
                                best = Some((dist, px));
                            }
                        }
                        if let Some((_, px)) = best {
                            row[c * 4..c * 4 + 4].copy_from_slice(&[px[0], px[1], px[2], 255]);
                        }
                    }
                }
            });
        }
    });
    Some(Panorama { w, h, rgba, e_lo, e_hi })
}

/// The photo skyline (see the module notes). False when the map's sky has no photo panels.
#[allow(clippy::too_many_arguments)]
fn build_photo_skyline(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    lvl: &dx_level::Level,
    centre: Vec3,
    half: f32,
    seed: &mut u64,
) -> bool {
    let Some(pano) = panorama(lvl) else { return false };
    let (w, h) = (pano.w, pano.h);
    let opaque = |c: usize, r: usize| pano.rgba[(r * w + c % w) * 4 + 3] > 0;
    // Each column's silhouette: (top row, bottom row) of its opaque run, or None.
    let cols: Vec<Option<(usize, usize)>> = (0..w)
        .map(|c| {
            let top = (0..h).find(|&r| opaque(c, r))?;
            let bot = (0..h).rev().find(|&r| opaque(c, r))?;
            Some((top, bot))
        })
        .collect();
    if cols.iter().all(|c| c.is_none()) {
        return false;
    }
    // Pieces: runs of columns cut where the roofline jumps a lot (a tower beside low blocks) or every 160 columns.
    // Every cut is a seam seen from off the middle, so they're kept few and wide.
    let mut pieces: Vec<(usize, usize)> = Vec::new();
    let mut c = 0;
    while c < w {
        if cols[c].is_none() {
            c += 1;
            continue;
        }
        let start = c;
        c += 1;
        while c < w && c - start < 320 {
            match (cols[c - 1], cols[c]) {
                (Some(a), Some(b)) if (a.0 as i32 - b.0 as i32).abs() <= 24 || c - start < 48 => c += 1,
                _ => break,
            }
        }
        // Keep narrow pieces with their neighbour.
        if c - start < 48 && c < w && cols[c].is_some() {
            while c < w && c - start < 48 && cols[c].is_some() {
                c += 1;
            }
        }
        pieces.push((start, c));
    }
    // The viewpoint the photo was taken from: the map's middle at the player's start height (or the bounds' middle).
    let eye_y = lvl.player_start.as_ref().map(|p| p.position[1] + 1.0).unwrap_or(centre.y);
    let base = (half + 1500.0).min(3300.0);
    let img = {
        let mut im = Image::new(
            Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
            TextureDimension::D2,
            pano.rgba.clone(),
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        im.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            ..default()
        });
        images.add(im)
    };
    let front_mat = materials.add(StandardMaterial {
        base_color_texture: Some(img.clone()),
        unlit: true,
        double_sided: true,
        cull_mode: None,
        alpha_mode: AlphaMode::Mask(0.5),
        // The photo already has the distance in it; fog over it greyed it out.
        fog_enabled: false,
        ..default()
    });
    // Side walls between pieces at different depths: the photo's edge column stretched along them, a little darker
    // (the side of the nearer building), so a seam seen from off the middle reads as a wall, not a black line.
    let wall_mat = materials.add(StandardMaterial {
        base_color_texture: Some(img.clone()),
        base_color: Color::srgb(0.7, 0.7, 0.7),
        unlit: true,
        double_sided: true,
        cull_mode: None,
        alpha_mode: AlphaMode::Mask(0.5),
        fog_enabled: false,
        ..default()
    });
    let elev = |r: f32| pano.e_hi - r / h as f32 * (pano.e_hi - pano.e_lo);
    let at = |col: f32, row: f32, rad: f32| -> [f32; 3] {
        let a = col / w as f32 * std::f32::consts::TAU;
        let e = elev(row);
        [centre.x + a.cos() * rad, eye_y + e.tan() * rad, centre.z + a.sin() * rad]
    };
    // Depth per piece: taller buildings tend to stand farther back (they show over the near ones), plus a little
    // noise, so neighbours sit at different depths.
    let depth: Vec<f32> = pieces
        .iter()
        .map(|&(a, b)| {
            let top = (a..b).filter_map(|c| cols[c].map(|x| x.0)).min().unwrap_or(h);
            let tall = 1.0 - top as f32 / h as f32;
            base * (0.86 + 0.24 * tall + 0.04 * rnd(seed))
        })
        .collect();
    let (mut fp, mut fu, mut fi) = (Vec::new(), Vec::new(), Vec::<u32>::new());
    let (mut wp, mut wu, mut wi) = (Vec::new(), Vec::new(), Vec::<u32>::new());
    for (k, &(a, b)) in pieces.iter().enumerate() {
        let rad = depth[k];
        let top = (a..b).filter_map(|c| cols[c].map(|x| x.0)).min().unwrap_or(0) as f32;
        let bot = (a..b).filter_map(|c| cols[c].map(|x| x.1)).max().unwrap_or(h - 1) as f32 + 1.0;
        // The front: one quad per few columns (keeps the chord close to the circle), photo-textured and masked.
        let n = (b - a).div_ceil(16);
        for s in 0..n {
            let c0 = a as f32 + (s * 16) as f32;
            let c1 = (a as f32 + ((s + 1) * 16) as f32).min(b as f32);
            let q = fp.len() as u32;
            fp.extend([at(c0, bot, rad), at(c1, bot, rad), at(c1, top, rad), at(c0, top, rad)]);
            let (u0, u1) = (c0 / w as f32, c1 / w as f32);
            let (v0, v1) = (top / h as f32, bot / h as f32);
            fu.extend([[u0, v1], [u1, v1], [u1, v0], [u0, v0]]);
            fi.extend([q, q + 1, q + 2, q, q + 2, q + 3]);
        }
        // A side wall to the next piece where they touch (no gap in the photo) and sit at different depths, up to
        // the lower of the two roofs at the seam.
        let next = (k + 1) % pieces.len();
        let (na, _) = pieces[next];
        if na == b % w && next != k {
            let (Some(l), Some(r)) = (cols[b - 1], cols[b % w]) else { continue };
            let seam_top = l.0.max(r.0) as f32;
            let seam_bot = l.1.max(r.1) as f32 + 1.0;
            let (r0, r1) = (rad, depth[next]);
            if (r0 - r1).abs() > 1.0 {
                let col = b as f32;
                let q = wp.len() as u32;
                wp.extend([at(col, seam_bot, r0), at(col, seam_bot, r1), at(col, seam_top, r1), at(col, seam_top, r0)]);
                // The nearer piece's edge column.
                let u = if r0 < r1 { (col - 0.5) / w as f32 } else { (col + 0.5) / w as f32 };
                let (v0, v1) = (seam_top / h as f32, seam_bot / h as f32);
                wu.extend([[u, v1], [u, v1], [u, v0], [u, v0]]);
                wi.extend([q, q + 1, q + 2, q, q + 2, q + 3]);
            }
        }
    }
    let mut fm = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    fm.insert_attribute(Mesh::ATTRIBUTE_POSITION, fp);
    fm.insert_attribute(Mesh::ATTRIBUTE_UV_0, fu);
    fm.insert_indices(Indices::U32(fi));
    commands.spawn((Mesh3d(meshes.add(fm)), MeshMaterial3d(front_mat), Transform::default(), SkylinePart, bevy::light::NotShadowCaster, bevy::light::NotShadowReceiver));
    if !wp.is_empty() {
        let mut wm = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        wm.insert_attribute(Mesh::ATTRIBUTE_POSITION, wp);
        wm.insert_attribute(Mesh::ATTRIBUTE_UV_0, wu);
        wm.insert_indices(Indices::U32(wi));
        commands.spawn((Mesh3d(meshes.add(wm)), MeshMaterial3d(wall_mat), Transform::default(), SkylinePart, bevy::light::NotShadowCaster, bevy::light::NotShadowReceiver));
    }
    true
}
