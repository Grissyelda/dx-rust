//! Relief maps for the level's textures (remastered lighting): a normal map and, where there is one, a height
//! map, per texture.
//!
//! Where they come from, best first:
//! 1. Normal / height map packs laid out like the DX11 renderer's: `<Package>/<Group>/<Name>.bump.dds` (a tangent-
//!    space normal map) and `<Name>.height.dds` (a height map, white = high) - the Parallax Occlusion Mapping addon
//!    for New Vision ships them for ~1,100 textures. Only those two kinds of file are read from a pack: its other
//!    `.dds` files (colour replacements meant for the New Vision addon) are left alone.
//! 2. Generated from the texture's own picture for an allow list of surfaces (brick, stone, concrete, planks, tiles,
//!    grates, metal panels): height from luminance, a normal map from its slopes. Textures with painted-in light
//!    (signs, screens, lamps, windows, posters) and effects stay flat.
//!
//! The maps come out in the glTF / Bevy convention: tangent-space X right, Y up in the image. The addon's `.bump`
//! files point X the other way (checked on DrtyReddBrick_A against its height map: a protruding brick's left edge
//! faced right), so their red channel is flipped on load.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use crate::dds;
use crate::TextureImage;

/// One texture's maps, RGBA8, mip 0 first.
#[derive(Clone)]
pub struct Relief {
    pub width: u32,
    pub height: u32,
    /// Tangent-space normal map (X right, Y up, Z out), linear.
    pub normal: Vec<Vec<u8>>,
    /// Depth for parallax (Bevy's depth map: white = deep), same size as the normal map; None = flat.
    pub depth: Option<Vec<Vec<u8>>>,
    /// From a pack (true) or generated (false).
    pub packed: bool,
}

#[derive(Default)]
struct Index {
    /// "package/group/name" (lower case) -> (bump, height).
    files: HashMap<String, (Option<PathBuf>, Option<PathBuf>)>,
}

static INDEX: RwLock<Option<Index>> = RwLock::new(None);

/// Index the relief packs under these folders (Textures-style: package folders inside). A folder holding the
/// Parallax Occlusion addon's own variant folders ("Half Size-Full Depth"...) uses Half Size-Full Depth.
/// Returns how many textures have maps.
pub fn set_dirs(dirs: &[PathBuf]) -> usize {
    let mut idx = Index::default();
    fn walk(base: &Path, dir: &Path, depth: usize, out: &mut Index) {
        if depth > 5 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(base, &p, depth + 1, out);
                continue;
            }
            let Some(name) = p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()) else { continue };
            let (stem, bump) = if let Some(s) = name.strip_suffix(".bump.dds") {
                (s.to_string(), true)
            } else if let Some(s) = name.strip_suffix(".height.dds") {
                (s.to_string(), false)
            } else {
                continue;
            };
            let Ok(rel) = p.parent().unwrap_or(base).strip_prefix(base) else { continue };
            let mut key: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase()).collect();
            if key.is_empty() {
                continue;
            }
            key.push(stem);
            let e = out.files.entry(key.join("/")).or_default();
            if bump {
                e.0.get_or_insert(p);
            } else {
                e.1.get_or_insert(p);
            }
        }
    }
    for d in dirs {
        let d = pick_variant(d);
        walk(&d, &d, 0, &mut idx);
    }
    let n = idx.files.len();
    if let Ok(mut g) = INDEX.write() {
        *g = Some(idx);
    }
    n
}

/// The Parallax Occlusion addon's folder as downloaded holds four variants: take Half Size (its readme's advice:
/// quicker to load) with full depth.
fn pick_variant(d: &Path) -> PathBuf {
    for sub in ["0-Parallax Occlusion Mapping Addon/Half Size-Full Depth", "Half Size-Full Depth", "0-Parallax Occlusion Mapping Addon/Full Size-Full Depth", "Full Size-Full Depth"] {
        let p = d.join(sub);
        if p.is_dir() {
            return p;
        }
    }
    d.to_path_buf()
}

/// SecondRevision's height maps for New Vision's textures: `<Package>.heightmap`, a zip (stored) of one DDS (BC4) a
/// texture and `<Package>.heightmap.txt` naming them ("Package.Name<TAB>file.dds<TAB>depth"). AI-made from New
/// Vision's pictures, so (their terms, and the user's rule) only used while New Vision's textures are on - the caller
/// indexes them only then. Key: "package/name" (lower case).
struct Archived {
    file: PathBuf,
    local: u64,
    size: u64,
    depth: f32,
}

static ARCHIVED: RwLock<Option<HashMap<String, Archived>>> = RwLock::new(None);

fn le16(b: &[u8], o: usize) -> u64 {
    u16::from_le_bytes([b[o], b[o + 1]]) as u64
}
fn le32(b: &[u8], o: usize) -> u64 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) as u64
}

/// A stored zip's items: (name, local header offset, size).
fn zip_items(f: &mut std::fs::File) -> Option<Vec<(String, u64, u64)>> {
    use std::io::{Read, Seek, SeekFrom};
    let len = f.metadata().ok()?.len();
    let tail = len.min(66_000);
    f.seek(SeekFrom::Start(len - tail)).ok()?;
    let mut t = vec![0u8; tail as usize];
    f.read_exact(&mut t).ok()?;
    let at = (0..t.len().saturating_sub(21)).rev().find(|&i| le32(&t, i) == 0x0605_4b50)?;
    let (count, dir_size, dir_off) = (le16(&t, at + 10), le32(&t, at + 12), le32(&t, at + 16));
    f.seek(SeekFrom::Start(dir_off)).ok()?;
    let mut d = vec![0u8; dir_size as usize];
    f.read_exact(&mut d).ok()?;
    let mut out = Vec::new();
    let mut p = 0usize;
    for _ in 0..count {
        if p + 46 > d.len() || le32(&d, p) != 0x0201_4b50 {
            break;
        }
        let method = le16(&d, p + 10);
        let size = le32(&d, p + 24);
        let (nl, xl, cl) = (le16(&d, p + 28) as usize, le16(&d, p + 30) as usize, le16(&d, p + 32) as usize);
        let local = le32(&d, p + 42);
        let name = String::from_utf8_lossy(&d[p + 46..(p + 46 + nl).min(d.len())]).to_string();
        if method == 0 {
            out.push((name, local, size));
        }
        p += 46 + nl + xl + cl;
    }
    Some(out)
}

fn zip_read(file: &Path, local: u64, size: u64) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(file).ok()?;
    f.seek(SeekFrom::Start(local)).ok()?;
    let mut h = [0u8; 30];
    f.read_exact(&mut h).ok()?;
    if le32(&h, 0) != 0x0403_4b50 {
        return None;
    }
    f.seek(SeekFrom::Start(local + 30 + le16(&h, 26) + le16(&h, 28))).ok()?;
    let mut b = vec![0u8; size as usize];
    f.read_exact(&mut b).ok()?;
    Some(b)
}

/// Index every `*.heightmap` archive in these folders (see `Archived`); how many textures they cover.
pub fn set_heightmap_archives(dirs: &[PathBuf]) -> usize {
    let mut map: HashMap<String, Archived> = HashMap::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("heightmap")) {
                continue;
            }
            let Ok(mut f) = std::fs::File::open(&p) else { continue };
            let Some(items) = zip_items(&mut f) else { continue };
            let Some((_, l, sz)) = items.iter().find(|(n, ..)| n.to_ascii_lowercase().ends_with(".txt")) else { continue };
            let Some(txt) = zip_read(&p, *l, *sz) else { continue };
            for line in String::from_utf8_lossy(&txt).lines() {
                let mut it = line.split('\t');
                let (Some(key), Some(file), depth) = (it.next(), it.next(), it.next().and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(1.0)) else { continue };
                if depth <= 0.0 {
                    continue;
                }
                let Some((_, local, size)) = items.iter().find(|(n, ..)| n.eq_ignore_ascii_case(file.trim())) else { continue };
                map.entry(key.trim().to_ascii_lowercase().replace('.', "/")).or_insert(Archived { file: p.clone(), local: *local, size: *size, depth });
            }
        }
    }
    let n = map.len();
    if let Ok(mut g) = ARCHIVED.write() {
        *g = (n > 0).then_some(map);
    }
    n
}

/// A texture's relief from SecondRevision's archives: the height map (white = high) as Bevy's depth map, normals
/// from its slopes (steeper for the deeper ones).
fn archived_maps(name: &str) -> Option<Relief> {
    let parts: Vec<String> = name.to_ascii_lowercase().split('.').map(str::to_string).collect();
    let key = format!("{}/{}", parts.first()?, parts.last()?);
    let (file, local, size, depth) = {
        let g = ARCHIVED.read().ok()?;
        let a = g.as_ref()?.get(&key)?;
        (a.file.clone(), a.local, a.size, a.depth)
    };
    let bytes = zip_read(&file, local, size)?;
    let d = dds::parse(&bytes)?;
    let top = dds::decode_level(d.format, d.mips.first()?, d.width, d.height)?;
    let (w, h) = (d.width as usize, d.height as usize);
    let height: Vec<f32> = top.chunks_exact(4).map(|p| p[0] as f32 / 255.0).collect();
    let normal = build_mips(d.width, d.height, normals_from_height(w, h, &height, 1.6 * depth.clamp(0.25, 4.0)), true);
    let mut dm = top;
    for px in dm.chunks_exact_mut(4) {
        let v = 255 - px[0];
        px[0] = v;
        px[1] = v;
        px[2] = v;
        px[3] = 255;
    }
    Some(Relief { width: d.width, height: d.height, normal, depth: Some(build_mips(d.width, d.height, dm, false)), packed: true })
}

/// Packs indexed.
pub fn has_packs() -> bool {
    INDEX.read().map(|g| g.as_ref().is_some_and(|i| !i.files.is_empty())).unwrap_or(false)
        || ARCHIVED.read().map(|g| g.is_some()).unwrap_or(false)
}

fn read_dds(p: &Path) -> Option<(u32, u32, Vec<Vec<u8>>)> {
    let b = std::fs::read(p).ok()?;
    let d = dds::parse(&b)?;
    let mips = dds::decode_all(&d);
    (!mips.is_empty()).then_some((d.width, d.height, mips))
}

/// Box-filtered mips from mip 0 (RGBA8), down to 1 x 1; normal maps are renormalised per level.
fn build_mips(w: u32, h: u32, top: Vec<u8>, normal: bool) -> Vec<Vec<u8>> {
    let mut out = vec![top];
    let (mut w, mut h) = (w as usize, h as usize);
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let src = out.last().unwrap();
        let mut dst = vec![0u8; nw * nh * 4];
        for y in 0..nh {
            for x in 0..nw {
                let mut acc = [0u32; 4];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let sx = (x * 2 + dx).min(w - 1);
                    let sy = (y * 2 + dy).min(h - 1);
                    let o = (sy * w + sx) * 4;
                    for c in 0..4 {
                        acc[c] += src[o + c] as u32;
                    }
                }
                let o = (y * nw + x) * 4;
                if normal {
                    let v = [acc[0] as f32 / 4.0 / 127.5 - 1.0, acc[1] as f32 / 4.0 / 127.5 - 1.0, acc[2] as f32 / 4.0 / 127.5 - 1.0];
                    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-4);
                    for c in 0..3 {
                        dst[o + c] = ((v[c] / l * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8;
                    }
                    dst[o + 3] = 255;
                } else {
                    for c in 0..4 {
                        dst[o + c] = (acc[c] / 4) as u8;
                    }
                }
            }
        }
        out.push(dst);
        w = nw;
        h = nh;
    }
    out
}

/// Normal map (glTF convention) from a height field, `strength` = slope gain.
fn normals_from_height(w: usize, h: usize, height: &[f32], strength: f32) -> Vec<u8> {
    let at = |x: isize, y: isize| -> f32 {
        let x = x.rem_euclid(w as isize) as usize;
        let y = y.rem_euclid(h as isize) as usize;
        height[y * w + x]
    };
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h as isize {
        for x in 0..w as isize {
            // Sobel; image rows go down, so "up" slope is the negative row derivative.
            let dx = (at(x + 1, y - 1) + 2.0 * at(x + 1, y) + at(x + 1, y + 1)) - (at(x - 1, y - 1) + 2.0 * at(x - 1, y) + at(x - 1, y + 1));
            let dy_rows = (at(x - 1, y + 1) + 2.0 * at(x, y + 1) + at(x + 1, y + 1)) - (at(x - 1, y - 1) + 2.0 * at(x, y - 1) + at(x + 1, y - 1));
            let nx = -dx * strength;
            let ny = dy_rows * strength; // height rising downward in the image = the face tilts up
            let l = (nx * nx + ny * ny + 1.0).sqrt();
            let o = ((y as usize) * w + x as usize) * 4;
            out[o] = ((nx / l * 0.5 + 0.5) * 255.0).round() as u8;
            out[o + 1] = ((ny / l * 0.5 + 0.5) * 255.0).round() as u8;
            out[o + 2] = ((1.0 / l * 0.5 + 0.5) * 255.0).round() as u8;
            out[o + 3] = 255;
        }
    }
    out
}

/// The texture's picture, RGBA8 at full size.
fn texture_rgba(t: &TextureImage) -> Option<Vec<u8>> {
    let m = t.mips.first()?;
    if t.bc1 {
        dds::decode_level(dds::Format::Bc1, m, t.width, t.height)
    } else if t.bc3 {
        dds::decode_level(dds::Format::Bc3, m, t.width, t.height)
    } else if m.len() == (t.width * t.height * 4) as usize {
        Some(m.clone())
    } else {
        None
    }
}

/// Surfaces worth a generated relief map (by package / group / name words) and how strongly.
fn generated_strength(name: &str) -> Option<f32> {
    let n = name.to_ascii_lowercase();
    const NO: &[&str] = &[
        "sky", "water", "glass", "window", "wndow", "sign", "poster", "light", "lite", "lamp", "screen", "monitor",
        "comp", "neon", "glow", "fx", "effect", "decal", "foliage", "leaf", "tree", "grass", "flag", "logo", "mural",
        "paint", "marker", "mask", "cloud", "fire", "spark", "blood", "font", "deusexui", "extras", "pinkmask",
        "graymask", "blackmask", "text", "map", "news", "photo", "portrait", "face", "skin",
    ];
    if NO.iter().any(|w| n.contains(w)) {
        return None;
    }
    const YES: &[(&str, f32)] = &[
        ("brick", 2.6), ("stone", 2.2), ("rock", 2.4), ("cobble", 2.6), ("concrete", 1.6), ("cinder", 2.0),
        ("stucco", 1.6), ("plaster", 1.2), ("wood", 1.6), ("plank", 1.8), ("board", 1.4), ("tile", 1.8),
        ("ceramic", 1.4), ("grate", 2.6), ("grill", 2.4), ("vent", 2.0), ("metal", 1.4), ("panel", 1.4),
        ("plate", 1.6), ("floor", 1.6), ("wall", 1.4), ("earth", 1.8), ("dirt", 1.8), ("gravel", 2.4),
        ("asphalt", 1.4), ("road", 1.2), ("pipe", 1.4), ("rust", 1.8), ("crate", 1.6), ("pallet", 1.6),
        ("chainlink", 2.0), ("marble", 1.0), ("carpet", 1.2), ("fabric", 1.0),
    ];
    YES.iter().find(|(w, _)| n.contains(w)).map(|(_, s)| *s)
}

/// The relief maps for a level texture (its "Package.Group.Name" and its picture), if it gets any.
pub fn maps_for(t: &TextureImage, generate: bool) -> Option<Relief> {
    let key = t.name.to_ascii_lowercase().replace('.', "/");
    let found = INDEX.read().ok().and_then(|g| g.as_ref().and_then(|i| i.files.get(&key).cloned()));
    if let Some((bump, height)) = found {
        if let Some((w, h, mut nm)) = bump.as_deref().and_then(read_dds) {
            // Flip X (see the module notes); keep only mip 0 and rebuild the chain renormalised.
            let mut top = std::mem::take(&mut nm[0]);
            for px in top.chunks_exact_mut(4) {
                px[0] = 255 - px[0];
                px[3] = 255;
            }
            let normal = build_mips(w, h, top, true);
            let depth = height.as_deref().and_then(read_dds).and_then(|(hw, hh, hm)| {
                if (hw, hh) != (w, h) {
                    return None;
                }
                let mut d = hm.into_iter().next()?;
                for px in d.chunks_exact_mut(4) {
                    let v = 255 - px[0];
                    px[0] = v;
                    px[1] = v;
                    px[2] = v;
                    px[3] = 255;
                }
                Some(build_mips(w, h, d, false))
            });
            return Some(Relief { width: w, height: h, normal, depth, packed: true });
        }
    }
    if let Some(r) = archived_maps(&t.name) {
        return Some(r);
    }
    if !generate {
        return None;
    }
    let strength = generated_strength(&t.name)?;
    let rgba = texture_rgba(t)?;
    let (w, h) = (t.width as usize, t.height as usize);
    if w < 16 || h < 16 {
        return None;
    }
    // Height = luminance, softened a touch (texture noise is not relief), scaled to the texture's size so a slope
    // reads the same at any resolution.
    let lum: Vec<f32> = rgba.chunks_exact(4).map(|p| (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32) / 255.0).collect();
    let mut soft = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut s = 0.0;
            for (dx, dy, k) in [(0i32, 0i32, 4.0f32), (1, 0, 2.0), (-1, 0, 2.0), (0, 1, 2.0), (0, -1, 2.0), (1, 1, 1.0), (-1, 1, 1.0), (1, -1, 1.0), (-1, -1, 1.0)] {
                let xx = (x as i32 + dx).rem_euclid(w as i32) as usize;
                let yy = (y as i32 + dy).rem_euclid(h as i32) as usize;
                s += lum[yy * w + xx] * k;
            }
            soft[y * w + x] = s / 16.0;
        }
    }
    let k = strength * (w as f32 / 256.0).max(0.5);
    let top = normals_from_height(w, h, &soft, k);
    Some(Relief { width: w as u32, height: h as u32, normal: build_mips(w as u32, h as u32, top, true), depth: None, packed: false })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_normals_face_up_on_a_ridge() {
        // A bright horizontal band (raised) in the middle of a dark field: its top edge faces up (G > 128), the
        // bottom edge down.
        let (w, h) = (16usize, 16usize);
        let mut height = vec![0.0f32; w * h];
        for y in 6..10 {
            for x in 0..w {
                height[y * w + x] = 1.0;
            }
        }
        let n = normals_from_height(w, h, &height, 1.0);
        let g = |x: usize, y: usize| n[(y * w + x) * 4 + 1];
        assert!(g(8, 5) > 128 || g(8, 6) > 128, "top edge should face up");
        assert!(g(8, 9) < 128 || g(8, 10) < 128, "bottom edge should face down");
    }

    #[test]
    fn allow_list() {
        assert!(generated_strength("CoreTexBrick.Brick.DrtyReddBrick_A").is_some());
        assert!(generated_strength("CoreTexMisc.Glass.Window_A").is_none());
        assert!(generated_strength("UNATCO.Signs.Sign_A").is_none());
    }
}

#[cfg(test)]
mod archive_tests {
    use super::*;

    /// SecondRevision's New Vision height maps, when a copy is at SR_HEIGHTMAPS (skipped otherwise).
    #[test]
    fn reads_secondrevision_archives() {
        let Some(d) = std::env::var_os("SR_HEIGHTMAPS").map(PathBuf::from) else { return };
        let n = set_heightmap_archives(&[d]);
        assert!(n > 100, "{n}");
        let r = archived_maps("Airfield.Floors.747PlatSteel_A").expect("747PlatSteel_A");
        assert_eq!((r.width, r.height), (256, 256));
        let top = &r.depth.as_ref().unwrap()[0];
        let lo = top.chunks_exact(4).map(|p| p[0]).min().unwrap();
        let hi = top.chunks_exact(4).map(|p| p[0]).max().unwrap();
        assert!(hi > lo + 40, "{lo}..{hi}");
    }
}
