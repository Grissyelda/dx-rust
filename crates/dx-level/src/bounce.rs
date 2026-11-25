//! Bounce Light (dxview's setting BounceLight): one bounce of the level's own light, baked when a map is built.
//!
//! The original lightmaps hold only direct light: Unreal's 1998 baker traced shadows but never let light bounce, so a
//! red glass panel never reddens the floor in front of it and a lamp-lit wall never lifts the shadow beside it. Modern
//! renderers get that from real-time global illumination (UE5's Lumen in Surreal 98); here it's baked once, so it
//! costs nothing while playing:
//!
//! * every lightmap texel of the surfaces drawn lit sends `RAYS` cosine-weighted rays into the level (the BSP's
//!   collision hull, traced on all cores, every other texel and blended between them);
//! * a ray that lands on another lit surface picks up that texel's light (the original bake plus the glowing
//!   surfaces' light from emissive.rs) times that surface's colour (its texture's average) - colour bleeding;
//! * the mean is added to the texel, in linear light (the atlas is uploaded as sRGB).
//!
//! Moving things (people, decorations) take the same light from probes: points over the floors and on the
//! navigation network gather it from all round, and `ambient_at` (through actors::zone_ambient_at) adds the nearest
//! ones in the same zone to their vertex light.
//!
//! The result is cached on disk (set_cache_dir, keyed by the lightmap, the surfaces' colours and BAKE_VERSION), so a
//! map bakes once. With the setting off nothing here runs.

use crate::collision::{BspCollision, ZoneTree};
use crate::lighting::{Baked, Patch};
use glam::DVec3;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

/// Rays per gathered texel and per probe.
pub const RAYS: usize = 40;
const PROBE_RAYS: usize = 64;
/// How far a ray looks for light (Unreal units).
const REACH: f64 = 4096.0;
/// Probe grid cell (Unreal units) for the lookup, and how far a probe still counts.
const CELL: f32 = 128.0;
const PROBE_REACH: f32 = 220.0;
/// Bump when the bake changes, so old cached bakes aren't used.
pub const BAKE_VERSION: u32 = 1;

static SWITCH: RwLock<Option<fn() -> f32>> = RwLock::new(None);
static CACHE_DIR: RwLock<Option<PathBuf>> = RwLock::new(None);

/// dxview hands over its setting (the bounce strength: 0 = off). DXL_BOUNCE=<strength> for the examples.
pub fn set_switch(f: fn() -> f32) {
    if let Ok(mut s) = SWITCH.write() {
        *s = Some(f);
    }
}

/// The bounce's strength in the next map built (0 = off).
pub fn strength() -> f32 {
    if let Some(v) = std::env::var("DXL_BOUNCE").ok().and_then(|s| s.parse::<f32>().ok()) {
        return v.max(0.0);
    }
    SWITCH.read().ok().and_then(|s| *s).map_or(0.0, |f| f().max(0.0))
}

/// Where baked bounces are kept between runs (None: bake every time).
pub fn set_cache_dir(dir: Option<PathBuf>) {
    if let Ok(mut d) = CACHE_DIR.write() {
        *d = dir;
    }
}

fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}
fn linear_to_srgb(v: f32) -> f32 {
    let v = v.max(0.0);
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}
fn v3(a: [f32; 3]) -> DVec3 {
    DVec3::new(a[0] as f64, a[1] as f64, a[2] as f64)
}

/// The surfaces' texel lookup: which lit surface (and which of its texels) a point lies on.
struct Surfaces<'a> {
    patches: &'a [Patch],
    /// Linear colour of each patch's surface (its texture's average).
    albedo: &'a [[f32; 3]],
    rgba: &'a [u8],
    width: usize,
    cells: HashMap<(i32, i32, i32), Vec<u32>>,
}

const LOOK_CELL: f32 = 256.0;

impl<'a> Surfaces<'a> {
    fn new(patches: &'a [Patch], albedo: &'a [[f32; 3]], rgba: &'a [u8], width: usize) -> Self {
        let mut cells: HashMap<(i32, i32, i32), Vec<u32>> = HashMap::new();
        for (i, p) in patches.iter().enumerate() {
            let lo = p.min.map(|v| ((v - 4.0) / LOOK_CELL).floor() as i32);
            let hi = p.max.map(|v| ((v + 4.0) / LOOK_CELL).floor() as i32);
            for x in lo[0]..=hi[0] {
                for y in lo[1]..=hi[1] {
                    for z in lo[2]..=hi[2] {
                        cells.entry((x, y, z)).or_default().push(i as u32);
                    }
                }
            }
        }
        Surfaces { patches, albedo, rgba, width, cells }
    }

    /// The light leaving point `q` toward a ray that arrived going `dir` (linear): the texel's light times the
    /// surface's colour, or None when no lit surface is there (sky, a full-bright face, an unlit hull).
    fn radiance(&self, q: DVec3, dir: DVec3) -> Option<[f32; 3]> {
        let key = ((q.x as f32 / LOOK_CELL).floor() as i32, (q.y as f32 / LOOK_CELL).floor() as i32, (q.z as f32 / LOOK_CELL).floor() as i32);
        let list = self.cells.get(&key)?;
        for &i in list {
            let p = &self.patches[i as usize];
            let n = v3(p.normal);
            let r = q - v3(p.origin);
            if r.dot(n).abs() > 2.5 {
                continue;
            }
            // Seen from its lit side (either side of a two-sided one).
            if !p.two_sided && dir.dot(n) > 0.0 {
                continue;
            }
            let (du, dv) = (v3(p.du), v3(p.dv));
            let (a11, a12, a22) = (du.dot(du), du.dot(dv), dv.dot(dv));
            let det = a11 * a22 - a12 * a12;
            if det.abs() < 1e-9 {
                continue;
            }
            let (b1, b2) = (du.dot(r), dv.dot(r));
            let tx = (a22 * b1 - a12 * b2) / det;
            let ty = (a11 * b2 - a12 * b1) / det;
            let [ax, ay, aw, ah] = p.rect;
            let (u, v) = ((aw - 2).max(1) as f64, (ah - 2).max(1) as f64);
            if tx < -0.75 || ty < -0.75 || tx > u - 0.25 || ty > v - 0.25 {
                continue;
            }
            let x = ax as usize + 1 + (tx.round().clamp(0.0, u - 1.0) as usize);
            let y = ay as usize + 1 + (ty.round().clamp(0.0, v - 1.0) as usize);
            let o = (y * self.width + x) * 4;
            let al = self.albedo[i as usize];
            return Some([0, 1, 2].map(|c| srgb_to_linear(self.rgba[o + c] as f32 / 255.0) * al[c]));
        }
        None
    }
}

/// A point's light from all round (sphere) or over a hemisphere about `n` (cosine-weighted), linear.
fn gather(s: &Surfaces, col: &BspCollision, at: DVec3, n: Option<DVec3>, rays: usize, seed: u32) -> [f32; 3] {
    let mut sum = [0.0f32; 3];
    // A per-point turn of the ray set (no banding between neighbouring texels).
    let turn = (hash(seed) as f64 / u32::MAX as f64) * std::f64::consts::TAU;
    let (t, b) = match n {
        Some(n) => basis(n),
        None => (DVec3::X, DVec3::Y),
    };
    for i in 0..rays {
        let u = (i as f64 + 0.5) / rays as f64;
        let v = (radical_inverse(i as u32) + turn / std::f64::consts::TAU).fract();
        let phi = v * std::f64::consts::TAU;
        let dir = match n {
            // Cosine-weighted: the mean of the light is the irradiance (over pi, which the reflectance cancels).
            Some(n) => {
                let r = u.sqrt();
                (t * (r * phi.cos()) + b * (r * phi.sin()) + n * (1.0 - u).max(0.0).sqrt()).normalize()
            }
            None => {
                let z = 1.0 - 2.0 * u;
                let r = (1.0 - z * z).max(0.0).sqrt();
                DVec3::new(r * phi.cos(), r * phi.sin(), z)
            }
        };
        let start = at;
        let end = start + dir * REACH;
        let hit = col.trace(start, end, DVec3::ZERO);
        if hit.fraction >= 0.999_9 {
            continue;
        }
        let q = start + (end - start) * hit.fraction;
        if let Some(l) = s.radiance(q, dir) {
            for c in 0..3 {
                sum[c] += l[c];
            }
        }
    }
    sum.map(|c| c / rays as f32)
}

fn basis(n: DVec3) -> (DVec3, DVec3) {
    let a = if n.z.abs() < 0.9 { DVec3::Z } else { DVec3::X };
    let t = a.cross(n).normalize();
    (t, n.cross(t))
}

fn radical_inverse(mut b: u32) -> f64 {
    b = b.rotate_right(16);
    b = ((b & 0x5555_5555) << 1) | ((b & 0xAAAA_AAAA) >> 1);
    b = ((b & 0x3333_3333) << 2) | ((b & 0xCCCC_CCCC) >> 2);
    b = ((b & 0x0F0F_0F0F) << 4) | ((b & 0xF0F0_F0F0) >> 4);
    b = ((b & 0x00FF_00FF) << 8) | ((b & 0xFF00_FF00) >> 8);
    b as f64 / 4_294_967_296.0
}

fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

/// The bounce for every lit texel: (patch index, linear light per texel) - gathered on every other texel and
/// blended, then smoothed over the patch (the ray noise).
fn bake_patches(s: &Surfaces, col: &BspCollision, only: &[usize]) -> Vec<(usize, Vec<[f32; 3]>)> {
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(1, 16);
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|sc| {
        let jobs: Vec<_> = (0..threads)
            .map(|_| {
                sc.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(&pi) = only.get(k) else { break };
                        out.push((pi, bake_one(s, col, pi)));
                    }
                    out
                })
            })
            .collect();
        jobs.into_iter().flat_map(|j| j.join().unwrap_or_default()).collect()
    })
}

fn bake_one(s: &Surfaces, col: &BspCollision, pi: usize) -> Vec<[f32; 3]> {
    let p = &s.patches[pi];
    let [_, _, aw, ah] = p.rect;
    let (u, v) = ((aw - 2).max(1) as usize, (ah - 2).max(1) as usize);
    let (o, du, dv, n) = (v3(p.origin), v3(p.du), v3(p.dv), v3(p.normal));
    // Grid point (i, j) = texel (2i, 2j), last row / column clamped.
    let (gw, gh) = (u.div_ceil(2) + 1, v.div_ceil(2) + 1);
    let mut grid = vec![[0.0f32; 3]; gw * gh];
    for j in 0..gh {
        for i in 0..gw {
            let (tx, ty) = ((2 * i).min(u - 1), (2 * j).min(v - 1));
            let at = o + du * tx as f64 + dv * ty as f64 + n * 1.5;
            let seed = (pi as u32).wrapping_mul(7919) ^ ((tx as u32) << 12) ^ (ty as u32);
            let mut g = gather(s, col, at, Some(n), RAYS, seed);
            if p.two_sided {
                let back = gather(s, col, at - n * 3.0, Some(-n), RAYS / 2, seed ^ 0x9e37);
                for c in 0..3 {
                    g[c] = g[c].max(back[c]);
                }
            }
            grid[j * gw + i] = g;
        }
    }
    // Blend to every texel, then a 3x3 box over the patch (it hides the ray noise; the light is soft anyway).
    let mut tex = vec![[0.0f32; 3]; u * v];
    for ty in 0..v {
        for tx in 0..u {
            let (i0, j0) = (tx / 2, ty / 2);
            let (fx, fy) = ((tx % 2) as f32 * 0.5, (ty % 2) as f32 * 0.5);
            let at = |i: usize, j: usize| grid[j.min(gh - 1) * gw + i.min(gw - 1)];
            let mut c = [0.0f32; 3];
            for (w, g) in [((1.0 - fx) * (1.0 - fy), at(i0, j0)), (fx * (1.0 - fy), at(i0 + 1, j0)), ((1.0 - fx) * fy, at(i0, j0 + 1)), (fx * fy, at(i0 + 1, j0 + 1))] {
                for k in 0..3 {
                    c[k] += g[k] * w;
                }
            }
            tex[ty * u + tx] = c;
        }
    }
    let mut out = tex.clone();
    for ty in 0..v {
        for tx in 0..u {
            let mut c = [0.0f32; 3];
            let mut w = 0.0f32;
            for y in ty.saturating_sub(1)..=(ty + 1).min(v - 1) {
                for x in tx.saturating_sub(1)..=(tx + 1).min(u - 1) {
                    let t = tex[y * u + x];
                    for k in 0..3 {
                        c[k] += t[k];
                    }
                    w += 1.0;
                }
            }
            out[ty * u + tx] = c.map(|x| x / w);
        }
    }
    out
}

/// The probes for moving things, by cell: (position, zone, linear light).
#[derive(Clone, Default)]
pub struct Probes {
    cells: HashMap<(i32, i32, i32), Vec<([f32; 3], i32, [f32; 3])>>,
    tree: Option<ZoneTree>,
}

impl std::fmt::Debug for Probes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Probes({} cells)", self.cells.len())
    }
}

impl Probes {
    fn insert(&mut self, at: [f32; 3], zone: i32, light: [f32; 3]) {
        let key = at.map(|v| (v / CELL).floor() as i32);
        self.cells.entry((key[0], key[1], key[2])).or_default().push((at, zone, light));
    }

    /// The bounce light at a point (linear): the probes within PROBE_REACH in the same zone, nearer ones weighing
    /// more. Zero where none is.
    pub fn at(&self, p: [f32; 3]) -> [f32; 3] {
        if self.cells.is_empty() {
            return [0.0; 3];
        }
        let zone = self.tree.as_ref().map_or(-1, |t| t.zone_at(v3(p)));
        let key = p.map(|v| (v / CELL).floor() as i32);
        let (mut sum, mut wsum) = ([0.0f32; 3], 0.0f32);
        for x in key[0] - 2..=key[0] + 2 {
            for y in key[1] - 2..=key[1] + 2 {
                for z in key[2] - 2..=key[2] + 2 {
                    let Some(list) = self.cells.get(&(x, y, z)) else { continue };
                    for (q, qz, l) in list {
                        if zone >= 0 && *qz >= 0 && *qz != zone {
                            continue;
                        }
                        let d = ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + (q[2] - p[2]).powi(2)).sqrt();
                        if d > PROBE_REACH {
                            continue;
                        }
                        let w = 1.0 / (d + 24.0).powi(2);
                        wsum += w;
                        for c in 0..3 {
                            sum[c] += l[c] * w;
                        }
                    }
                }
            }
        }
        if wsum <= 0.0 { [0.0; 3] } else { sum.map(|c| c / wsum) }
    }
}

/// Where probes go: over the floors (every PROBE_STEP, at knee and chest height) and at the given points (the
/// navigation network), each pulled out of walls by tracing.
const PROBE_STEP: f32 = 288.0;

fn probe_points(patches: &[Patch], only: &[usize], extra: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let mut seen: std::collections::HashSet<(i32, i32, i32)> = Default::default();
    let mut out = Vec::new();
    let mut add = |p: [f32; 3], out: &mut Vec<[f32; 3]>| {
        let k = p.map(|v| (v / 128.0).round() as i32);
        if seen.insert((k[0], k[1], k[2])) {
            out.push(p);
        }
    };
    for &pi in only {
        let p = &patches[pi];
        if p.normal[2] < 0.7 {
            continue;
        }
        let (o, du, dv) = (v3(p.origin), v3(p.du), v3(p.dv));
        let [_, _, aw, ah] = p.rect;
        let (u, v) = ((aw - 2).max(1) as f64, (ah - 2).max(1) as f64);
        let (lu, lv) = (du.length() * u, dv.length() * v);
        let (nu, nv) = ((lu / PROBE_STEP as f64).ceil().max(1.0) as usize, (lv / PROBE_STEP as f64).ceil().max(1.0) as usize);
        for i in 0..nu {
            for j in 0..nv {
                let q = o + du * ((i as f64 + 0.5) / nu as f64 * u) + dv * ((j as f64 + 0.5) / nv as f64 * v);
                for h in [64.0] {
                    add([q.x as f32, q.y as f32, q.z as f32 + h], &mut out);
                }
            }
        }
    }
    for e in extra {
        add(*e, &mut out);
    }
    out
}

/// Bakes the bounce into `baked` (only the patches in `only`: those drawn lit) and returns the probes for actors.
/// `albedo_of`: each patch's surface colour (linear), by patch index. `extra_probes`: more probe points (nav).
pub fn add_to_bake(baked: &mut Baked, only: &[usize], albedo_of: &[[f32; 3]], col: &BspCollision, extra_probes: &[[f32; 3]], key_name: &str) -> (usize, Probes) {
    let k = strength();
    let t0 = std::time::Instant::now();
    if k <= 0.0 || only.is_empty() {
        return (0, Probes::default());
    }
    let key = cache_key(baked, albedo_of, only);
    let cached = load_cache(key_name, key);
    let from_cache = cached.is_some();
    let (results, probes_raw) = match cached {
        Some(c) => c,
        None => {
            let s = Surfaces::new(&baked.patches, albedo_of, &baked.rgba, baked.width as usize);
            let results = bake_patches(&s, col, only);
            let pts = probe_points(&baked.patches, only, extra_probes);
            let tree = col.zone_tree();
            let probes_raw: Vec<([f32; 3], i32, [f32; 3])> = probe_values(&s, col, &pts)
                .into_iter()
                .map(|(p, l)| (p, tree.zone_at(v3(p)), l))
                .collect();
            save_cache(key_name, key, &results, &probes_raw);
            (results, probes_raw)
        }
    };
    let w = baked.width as usize;
    let mut texels = 0;
    for (pi, add) in &results {
        let Some(p) = baked.patches.get(*pi) else { continue };
        let [ax, ay, aw, ah] = p.rect;
        let (u, v) = ((aw - 2).max(1) as usize, (ah - 2).max(1) as usize);
        if add.len() != u * v {
            continue;
        }
        texels += u * v;
        for yy in 0..ah as usize {
            for xx in 0..aw as usize {
                let sx = xx.saturating_sub(1).min(u - 1);
                let sy = yy.saturating_sub(1).min(v - 1);
                let a = add[sy * u + sx];
                let o = ((ay as usize + yy) * w + ax as usize + xx) * 4;
                for c in 0..3 {
                    let lin = srgb_to_linear(baked.rgba[o + c] as f32 / 255.0) + a[c] * k;
                    baked.rgba[o + c] = (linear_to_srgb(lin).min(1.0) * 255.0).round() as u8;
                }
            }
        }
    }
    let mut probes = Probes { tree: Some(col.zone_tree()), ..Default::default() };
    for (p, z, l) in &probes_raw {
        probes.insert(*p, *z, l.map(|c| c * k));
    }
    println!(
        "level: bounce light on {} texels, {} probes, {} ms{}",
        texels,
        probes_raw.len(),
        t0.elapsed().as_millis(),
        if from_cache { " (cached)" } else { "" }
    );
    (texels, probes)
}

fn probe_values(s: &Surfaces, col: &BspCollision, pts: &[[f32; 3]]) -> Vec<([f32; 3], [f32; 3])> {
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(1, 16);
    let chunk = pts.len().div_ceil(threads).max(1);
    std::thread::scope(|sc| {
        let jobs: Vec<_> = pts
            .chunks(chunk)
            .enumerate()
            .map(|(ci, ch)| {
                sc.spawn(move || {
                    ch.iter()
                        .enumerate()
                        .filter_map(|(i, p)| {
                            let at = v3(*p);
                            // Inside a wall (a probe point in solid): skip it.
                            if col.trace(at + DVec3::Z * 2.0, at - DVec3::Z * 2.0, DVec3::ZERO).fraction < 1e-3 {
                                return None;
                            }
                            Some((*p, gather(s, col, at, None, PROBE_RAYS, (ci * chunk + i) as u32 ^ 0xb5)))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        jobs.into_iter().flat_map(|j| j.join().unwrap_or_default()).collect()
    })
}

/// A texture's plain average colour in linear light (the colour it reflects), from its smallest mip; see-through
/// texels left out. Kept below 0.85 so nothing bounces back more light than it took.
pub fn average_albedo(t: &crate::TextureImage) -> Option<[f32; 3]> {
    let (k, m) = t.mips.iter().enumerate().last()?;
    let w = (t.width >> k).max(1) as usize;
    let h = (t.height >> k).max(1) as usize;
    let rgba: Vec<u8> = if t.bc1 || t.bc3 {
        let blocks: Vec<u8> = if t.bc3 { m.chunks_exact(16).flat_map(|b| b[8..16].to_vec()).collect() } else { m.clone() };
        let mut out = vec![0u8; w * h * 4];
        unreal_pkg::objects::decode_dxt1(&blocks, w, h, &mut out)?;
        out
    } else {
        m.clone()
    };
    let (mut sum, mut n) = ([0.0f64; 3], 0usize);
    for px in rgba.chunks_exact(4) {
        if px[3] < 128 {
            continue;
        }
        n += 1;
        for c in 0..3 {
            sum[c] += srgb_to_linear(px[c] as f32 / 255.0) as f64;
        }
    }
    if n == 0 {
        return None;
    }
    Some(sum.map(|c| ((c / n as f64) as f32).min(0.85)))
}

// ---- probes for actors in play (actors::zone_ambient_at adds them)

static PLAY: RwLock<Option<std::sync::Arc<Probes>>> = RwLock::new(None);
thread_local! {
    static BUILDING: std::cell::RefCell<Option<std::sync::Arc<Probes>>> = const { std::cell::RefCell::new(None) };
}

/// The probes of the level in play (dxview, when a level is put in place).
pub fn set_play(p: Option<std::sync::Arc<Probes>>) {
    if let Ok(mut g) = PLAY.write() {
        *g = p.filter(|p| !p.cells.is_empty());
    }
}

/// The probes of the level this thread is building.
pub fn set_building(p: Option<std::sync::Arc<Probes>>) {
    BUILDING.with(|b| *b.borrow_mut() = p.filter(|p| !p.cells.is_empty()));
}

/// Bounce light at a point for actors (linear, before the light gain, like the zone ambient).
pub fn ambient_at(p: [f32; 3]) -> [f32; 3] {
    let enc = |l: [f32; 3]| l.map(|c| linear_to_srgb(c) / crate::lighting::LIGHT_GAIN);
    if let Some(l) = BUILDING.with(|b| b.borrow().as_ref().map(|pr| pr.at(p))) {
        return enc(l);
    }
    let Ok(g) = PLAY.read() else { return [0.0; 3] };
    g.as_ref().map(|pr| enc(pr.at(p))).unwrap_or([0.0; 3])
}

// ---- cache

fn cache_key(baked: &Baked, albedo_of: &[[f32; 3]], only: &[usize]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ BAKE_VERSION as u64;
    let mut eat = |b: &[u8]| {
        for &x in b {
            h ^= x as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    eat(&baked.rgba);
    eat(&(baked.patches.len() as u64).to_le_bytes());
    for a in albedo_of {
        for c in a {
            eat(&((c * 255.0) as u8).to_le_bytes());
        }
    }
    for &i in only {
        eat(&(i as u32).to_le_bytes());
    }
    h
}

fn cache_path(name: &str, key: u64) -> Option<PathBuf> {
    let dir = CACHE_DIR.read().ok()?.clone()?;
    let stem: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
    Some(dir.join(format!("{stem}-{key:016x}.bounce")))
}

type Cached = (Vec<(usize, Vec<[f32; 3]>)>, Vec<([f32; 3], i32, [f32; 3])>);

fn load_cache(name: &str, key: u64) -> Option<Cached> {
    let bytes = std::fs::read(cache_path(name, key)?).ok()?;
    let mut r = Reader { b: &bytes, at: 0 };
    if r.u32()? != 0x4243_4e42 || r.u32()? != BAKE_VERSION {
        return None;
    }
    let np = r.u32()? as usize;
    let mut patches = Vec::with_capacity(np);
    for _ in 0..np {
        let pi = r.u32()? as usize;
        let n = r.u32()? as usize;
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push([r.f16()?, r.f16()?, r.f16()?]);
        }
        patches.push((pi, v));
    }
    let nq = r.u32()? as usize;
    let mut probes = Vec::with_capacity(nq);
    for _ in 0..nq {
        let p = [r.f32()?, r.f32()?, r.f32()?];
        let z = r.u32()? as i32;
        probes.push((p, z, [r.f16()?, r.f16()?, r.f16()?]));
    }
    Some((patches, probes))
}

fn save_cache(name: &str, key: u64, patches: &[(usize, Vec<[f32; 3]>)], probes: &[([f32; 3], i32, [f32; 3])]) {
    let Some(path) = cache_path(name, key) else { return };
    let mut b: Vec<u8> = Vec::new();
    let u32b = |b: &mut Vec<u8>, v: u32| b.extend_from_slice(&v.to_le_bytes());
    u32b(&mut b, 0x4243_4e42);
    u32b(&mut b, BAKE_VERSION);
    u32b(&mut b, patches.len() as u32);
    for (pi, v) in patches {
        u32b(&mut b, *pi as u32);
        u32b(&mut b, v.len() as u32);
        for c in v {
            for x in c {
                b.extend_from_slice(&f16_bits(*x).to_le_bytes());
            }
        }
    }
    u32b(&mut b, probes.len() as u32);
    for (p, z, l) in probes {
        for x in p {
            b.extend_from_slice(&x.to_le_bytes());
        }
        u32b(&mut b, *z as u32);
        for x in l {
            b.extend_from_slice(&f16_bits(*x).to_le_bytes());
        }
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
        // Older bakes of the same map go (only ours: same stem, .bounce).
        if let (Ok(rd), Some(stem)) = (std::fs::read_dir(dir), path.file_name().and_then(|f| f.to_str()).and_then(|f| f.rsplit_once('-')).map(|(s, _)| s.to_string())) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if n.ends_with(".bounce") && n.rsplit_once('-').is_some_and(|(s, _)| s == stem) && e.path() != path {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, &b).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}
impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let s = self.b.get(self.at..self.at + n)?;
        self.at += n;
        Some(s)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn f16(&mut self) -> Option<f32> {
        Some(f16_value(u16::from_le_bytes(self.take(2)?.try_into().ok()?)))
    }
}

fn f16_bits(v: f32) -> u16 {
    let x = v.max(0.0).min(65000.0).to_bits();
    let exp = ((x >> 23) & 0xff) as i32 - 127 + 15;
    if exp <= 0 {
        return 0;
    }
    if exp >= 31 {
        return 0x7bff;
    }
    ((exp as u16) << 10) | ((x >> 13) & 0x3ff) as u16
}

fn f16_value(h: u16) -> f32 {
    let exp = ((h >> 10) & 0x1f) as u32;
    if exp == 0 {
        return 0.0;
    }
    f32::from_bits(((exp + 127 - 15) << 23) | (((h & 0x3ff) as u32) << 13))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_floats_round_trip() {
        for v in [0.0f32, 0.001, 0.25, 0.5, 1.0, 3.7] {
            let back = f16_value(f16_bits(v));
            assert!((back - v).abs() <= v * 0.002 + 1e-4, "{v} -> {back}");
        }
    }

    #[test]
    fn colour_space_round_trip() {
        for v in [0.0f32, 0.01, 0.2, 0.5, 0.9, 1.0] {
            assert!((linear_to_srgb(srgb_to_linear(v)) - v).abs() < 1e-4);
        }
    }

    #[test]
    fn cosine_rays_face_the_normal() {
        let n = DVec3::new(0.3, -0.2, 0.9).normalize();
        let (t, b) = basis(n);
        assert!(t.dot(n).abs() < 1e-9 && b.dot(n).abs() < 1e-9 && t.dot(b).abs() < 1e-9);
        assert!(radical_inverse(1) > 0.49 && radical_inverse(1) < 0.51);
    }
}
