//! Enhancements (3D Skyline and Night Clouds, ROADMAP v2 #33). Ours, off by default.
//!
//! - 3D Skyline: where the map's sky room has masked skyline photos (the NYskybox panels of the New York maps), the
//!   photos are unrolled into one panorama as seen from the sky room's viewpoint, cut into buildings by their
//!   silhouette, and each piece stands as a flat slab at its own distance from the map (side walls joining slabs at
//!   different depths) - the original picture, seen from the centre of the map exactly as before, but as low-poly
//!   3D: it shifts as you move, and the sky room's flat panels are hidden. Maps without photo panels get no skyline.

use crate::LevelRes;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor, ImageFilterMode};

pub fn skyline_on() -> bool {
    setting!("Skyline") > 0.5
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


#[allow(clippy::too_many_arguments)]
pub fn skyline_system(
    mut commands: Commands,
    level: Res<LevelRes>,
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
        let outdoors = lvl.zone_sky.iter().any(|s| *s);
        // The intro and the endings: one map holding many scenes, whose sky-room backdrops stand in for walls and
        // distant streets; a skyline ring round the whole map showed through those "walls".
        let m = lvl.name.to_ascii_lowercase();
        if !outdoors || m.starts_with("00_intro") || m.starts_with("99_endgame") {
            for mut v in &mut panels {
                v.set_if_neq(Visibility::Inherited);
            }
            return;
        }
        let (bmin, bmax) = (Vec3::from(lvl.bounds_min), Vec3::from(lvl.bounds_max));
        if bmin.x > bmax.x {
            return;
        }
        let centre = (bmin + bmax) * 0.5;
        let half = ((bmax.x - bmin.x).max(bmax.z - bmin.z) * 0.5).max(60.0);
        let mut seed = hash(&lvl.name) | 1;
        let mut photo = false;
        if want.0 {
            photo = build_photo_skyline(&mut commands, &mut meshes, &mut materials, &mut images, lvl, centre, half, &mut seed);
        }
        for mut v in &mut panels {
            v.set_if_neq(if photo { Visibility::Hidden } else { Visibility::Inherited });
        }
    }
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
        if let std::collections::hash_map::Entry::Vacant(e) = decoded.entry(ti) {
            let Some(px) = mip0_rgba(&lvl.textures[ti]) else { continue };
            e.insert(px);
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
    clean_edges(&mut rgba, w, h);
    Some(Panorama { w, h, rgba, e_lo, e_hi })
}

/// The photo panels carry a dark matte line round their buildings, and see-through texels are black: under linear
/// filtering both showed as a dark outline against the clouds. Trim the edge texel (and the next one where it's
/// darker than the building inside it), then bleed the edge colour out into the see-through texels (alpha stays 0),
/// so filtering at the silhouette only mixes building colours.
fn clean_edges(rgba: &mut [u8], w: usize, h: usize) {
    let idx = |x: i64, y: i64| -> Option<usize> {
        if y < 0 || y >= h as i64 {
            return None;
        }
        Some((y as usize * w + x.rem_euclid(w as i64) as usize) * 4)
    };
    let luma = |p: &[u8]| p[0] as f32 * 0.3 + p[1] as f32 * 0.59 + p[2] as f32 * 0.11;
    const N8: [(i64, i64); 8] = [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)];
    for pass in 0..2 {
        let src = rgba.to_vec();
        for y in 0..h as i64 {
            for x in 0..w as i64 {
                let o = idx(x, y).unwrap();
                if src[o + 3] == 0 {
                    continue;
                }
                let edge = N8.iter().any(|(dx, dy)| idx(x + dx, y + dy).is_some_and(|n| src[n + 3] == 0));
                if !edge {
                    continue;
                }
                let cut = if pass == 0 {
                    true
                } else {
                    // Average of the opaque texels two steps in (away from the hole).
                    let (mut sum, mut n) = (0.0, 0);
                    for (dx, dy) in N8 {
                        if let Some(q) = idx(x + dx * 2, y + dy * 2) {
                            if src[q + 3] != 0 {
                                sum += luma(&src[q..q + 4]);
                                n += 1;
                            }
                        }
                    }
                    n > 0 && luma(&src[o..o + 4]) < 0.75 * sum / n as f32
                };
                if cut {
                    rgba[o..o + 4].copy_from_slice(&[0, 0, 0, 0]);
                }
            }
        }
    }
    // Bleed colour 4 texels out (alpha untouched).
    let mut filled: Vec<bool> = rgba.chunks_exact(4).map(|p| p[3] != 0).collect();
    for _ in 0..4 {
        let src = rgba.to_vec();
        let was = filled.clone();
        for y in 0..h as i64 {
            for x in 0..w as i64 {
                let i = y as usize * w + x as usize;
                if was[i] {
                    continue;
                }
                let (mut c, mut n) = ([0u32; 3], 0u32);
                for (dx, dy) in N8 {
                    if let Some(q) = idx(x + dx, y + dy) {
                        if was[q / 4] {
                            for k in 0..3 {
                                c[k] += src[q + k] as u32;
                            }
                            n += 1;
                        }
                    }
                }
                if n > 0 {
                    let o = i * 4;
                    for k in 0..3 {
                        rgba[o + k] = (c[k] / n) as u8;
                    }
                    filled[i] = true;
                }
            }
        }
    }
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
