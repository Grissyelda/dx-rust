//! Rebuilds the original Unreal lightmaps.
//!
//! The map stores, for every lit surface, a small grid of texels (`LightMapIndex`) and, for each static
//! light that can reach it, a 1-bit shadow mask (`Model::light_bits`). The engine multiplied those masks by
//! each light's color and distance falloff at load time. We do the same here and pack the results into one
//! atlas so a renderer can sample it with a second UV set.

use std::collections::HashMap;
use unreal_pkg::objects::Model;
use unreal_pkg::props::{self, Value};
use unreal_pkg::classes::ClassDb;
use unreal_pkg::Package;

/// Overall scale from summed light intensity to the 0..1 lightmap value (the original used 2x overbright).
pub const LIGHT_GAIN: f32 = 4.0;

#[derive(Clone)]
pub struct Atlas {
    pub width: u32,
    pub height: u32,
    /// RGBA8, color channels sRGB-encoded.
    pub rgba: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub struct LightInfo {
    /// Unreal units.
    pub pos: [f32; 3],
    /// Color times brightness (0..1 per channel).
    pub color: [f32; 3],
    /// Unreal units.
    pub radius: f32,
}

struct Rect {
    x: u32,
    y: u32,
}

pub struct Baked {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    rects: HashMap<usize, Rect>, // surface index -> atlas rect (includes 1px border offset)
}

fn byte(ps: &[props::Property], name: &str) -> Option<u8> {
    match props::find(ps, name) {
        Some(Value::Byte(b)) => Some(*b),
        _ => None,
    }
}

pub fn hsv_white(hue: u8, whiteness: u8) -> [f32; 3] {
    // Unreal's LightSaturation is inverted: 255 means white, 0 means a fully saturated color.
    let h = hue as f32 / 255.0 * 6.0;
    let s = 1.0 - whiteness as f32 / 255.0;
    let f = h - h.floor();
    let (p, q, t) = (1.0 - s, 1.0 - s * f, 1.0 - s * (1.0 - f));
    match h.floor() as i32 % 6 {
        0 => [1.0, t, p],
        1 => [q, 1.0, p],
        2 => [p, 1.0, t],
        3 => [p, q, 1.0],
        4 => [t, p, 1.0],
        _ => [1.0, p, q],
    }
}

fn load_light(pkg: &Package, idx: i32, classes: &mut ClassDb) -> Option<LightInfo> {
    let e = pkg.exports.get(idx as usize - 1)?;
    // TriggerLights switch on and off in play (and most start off): not baked, dxview's dynlight drives them.
    if pkg.class_name(e.class_index) == "TriggerLight" {
        return None;
    }
    // Class defaults (Light: Steady, brightness 64, radius 64, saturation 255) overlaid with the placed values.
    let ps = classes.actor_properties(pkg, e)?;
    let Some(Value::Vector(pos)) = props::find(&ps, "Location") else { return None };
    let ltype = byte(&ps, "LightType").unwrap_or(0);
    if ltype == 0 {
        return None;
    }
    let brightness = byte(&ps, "LightBrightness").unwrap_or(64) as f32 / 255.0;
    let radius = 25.0 * (byte(&ps, "LightRadius").unwrap_or(64) as f32 + 1.0);
    let c = hsv_white(byte(&ps, "LightHue").unwrap_or(0), byte(&ps, "LightSaturation").unwrap_or(255));
    Some(LightInfo { pos: *pos, color: [c[0] * brightness, c[1] * brightness, c[2] * brightness], radius })
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn linear_to_srgb_gamma(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

pub fn bake(pkg: &Package, model: &Model, classes: &mut ClassDb) -> Baked {
    // Pack rectangles on shelves, tallest first.
    let mut order: Vec<usize> = (0..model.surfaces.len())
        .filter(|&i| {
            let l = model.surfaces[i].light_map;
            l >= 0 && (l as usize) < model.lightmaps.len()
        })
        .collect();
    order.sort_by_key(|&i| std::cmp::Reverse(model.lightmaps[model.surfaces[i].light_map as usize].v_clamp));
    let atlas_w = 4096u32;
    let (mut x, mut y, mut shelf_h) = (4u32, 0u32, 2u32); // top-left 4x2 block reserved as full-bright
    let mut rects: HashMap<usize, Rect> = HashMap::new();
    let mut placed: Vec<(usize, u32, u32, u32, u32)> = Vec::new();
    for &si in &order {
        let lm = &model.lightmaps[model.surfaces[si].light_map as usize];
        let (w, h) = (lm.u_clamp.max(1) as u32 + 2, lm.v_clamp.max(1) as u32 + 2);
        if x + w > atlas_w {
            y += shelf_h;
            x = 0;
            shelf_h = 0;
        }
        if x == 4 && y == 0 {
            // first shelf shares the row with the reserved block
        }
        rects.insert(si, Rect { x: x + 1, y: y + 1 });
        placed.push((si, x, y, w, h));
        x += w;
        shelf_h = shelf_h.max(h);
    }
    let height = (y + shelf_h).max(2).next_power_of_two().max(64);
    let mut rgba = vec![0u8; (atlas_w * height * 4) as usize];
    // Reserved full-bright block.
    for yy in 0..2 {
        for xx in 0..4 {
            let o = ((yy * atlas_w + xx) * 4) as usize;
            rgba[o..o + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
    }

    let mut lights: HashMap<i32, Option<LightInfo>> = HashMap::new();
    for &(si, ax, ay, aw, ah) in &placed {
        let surf = &model.surfaces[si];
        let lm = &model.lightmaps[surf.light_map as usize];
        let (u_clamp, v_clamp) = (lm.u_clamp.max(1) as usize, lm.v_clamp.max(1) as usize);
        let (Some(base), Some(tu), Some(tv), Some(nrm)) = (
            model.points.get(surf.p_base as usize).copied(),
            model.vectors.get(surf.v_texture_u as usize).copied(),
            model.vectors.get(surf.v_texture_v as usize).copied(),
            model.vectors.get(surf.v_normal as usize).copied(),
        ) else {
            continue;
        };
        let nl = dot(nrm, nrm).sqrt().max(1e-9);
        let n = scale(nrm, 1.0 / nl);
        let det = dot(tu, cross(tv, n));
        if det.abs() < 1e-9 {
            continue;
        }
        let (e1, e2) = (scale(cross(tv, n), 1.0 / det), scale(cross(n, tu), 1.0 / det));

        // Light list for this surface (terminated by a null reference).
        let mut infos: Vec<Option<LightInfo>> = Vec::new();
        if lm.light_actors >= 0 {
            let mut i = lm.light_actors as usize;
            while i < model.lights.len() && model.lights[i] != 0 {
                let r = model.lights[i];
                infos.push(*lights.entry(r).or_insert_with(|| load_light(pkg, r, classes)));
                i += 1;
            }
        }
        let stride = (u_clamp + 7) / 8;
        let block = stride * v_clamp;

        let mut tex = vec![[0u8; 4]; u_clamp * v_clamp];
        for ty in 0..v_clamp {
            for tx in 0..u_clamp {
                let a = tx as f32 * lm.u_scale + lm.pan[0];
                let b = ty as f32 * lm.v_scale + lm.pan[1];
                let d = [e1[0] * a + e2[0] * b, e1[1] * a + e2[1] * b, e1[2] * a + e2[2] * b];
                let p = [base[0] + d[0], base[1] + d[1], base[2] + d[2]];
                let mut sum = [0.0f32; 3];
                for (k, info) in infos.iter().enumerate() {
                    let Some(l) = info else { continue };
                    // The engine filtered each 1-bit shadow mask with a 3x3 tent before lighting, which softens
                    // shadow edges by about a texel (without it every shadow is a hard staircase).
                    let bit = |x: usize, y: usize| -> f32 {
                        let off = lm.data_offset as usize + k * block + y * stride + (x >> 3);
                        model.light_bits.get(off).map(|v| ((v >> (x & 7)) & 1) as f32).unwrap_or(0.0)
                    };
                    let mut vis = 0.0f32;
                    for (dy, wy) in [(-1i32, 1.0f32), (0, 2.0), (1, 1.0)] {
                        for (dx, wx) in [(-1i32, 1.0f32), (0, 2.0), (1, 1.0)] {
                            let x = (tx as i32 + dx).clamp(0, u_clamp as i32 - 1) as usize;
                            let y = (ty as i32 + dy).clamp(0, v_clamp as i32 - 1) as usize;
                            vis += bit(x, y) * wx * wy;
                        }
                    }
                    let vis = vis / 16.0;
                    if vis <= 0.0 {
                        continue;
                    }
                    let to = sub(l.pos, p);
                    let dist = dot(to, to).sqrt();
                    if dist >= l.radius {
                        continue;
                    }
                    let ndl = (dot(to, n) / dist.max(1e-6)).abs();
                    let f = (1.0 - dist / l.radius) * ndl * vis;
                    for c in 0..3 {
                        sum[c] += l.color[c] * f;
                    }
                }
                tex[ty * u_clamp + tx] = [
                    linear_to_srgb_gamma(sum[0] * LIGHT_GAIN),
                    linear_to_srgb_gamma(sum[1] * LIGHT_GAIN),
                    linear_to_srgb_gamma(sum[2] * LIGHT_GAIN),
                    255,
                ];
            }
        }
        // Copy with a replicated 1-texel border.
        for yy in 0..ah as usize {
            for xx in 0..aw as usize {
                let sx = xx.saturating_sub(1).min(u_clamp - 1);
                let sy = yy.saturating_sub(1).min(v_clamp - 1);
                let o = (((ay as usize + yy) * atlas_w as usize) + ax as usize + xx) * 4;
                rgba[o..o + 4].copy_from_slice(&tex[sy * u_clamp + sx]);
            }
        }
    }
    Baked { width: atlas_w, height, rgba, rects }
}

/// All static lights referenced by the level BSP.
pub fn collect_lights(pkg: &Package, model: &Model, classes: &mut ClassDb) -> Vec<LightInfo> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for &r in &model.lights {
        if r > 0 && seen.insert(r) {
            if let Some(l) = load_light(pkg, r, classes) {
                out.push(l);
            }
        }
    }
    out
}

impl std::fmt::Debug for Atlas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Atlas({}x{})", self.width, self.height)
    }
}

impl Baked {
    /// Atlas UV of point `p` (Unreal units) on surface `si`.
    pub fn uv1(&self, model: &Model, si: usize, p: [f32; 3]) -> [f32; 2] {
        let full_bright = [2.0 / self.width as f32, 1.0 / self.height as f32];
        let Some(r) = self.rects.get(&si) else { return full_bright };
        let surf = &model.surfaces[si];
        if surf.poly_flags & 0x0040_0000 != 0 {
            return full_bright; // PF_Unlit: drawn at full brightness
        }
        let lm = &model.lightmaps[surf.light_map as usize];
        let (Some(base), Some(tu), Some(tv)) = (
            model.points.get(surf.p_base as usize).copied(),
            model.vectors.get(surf.v_texture_u as usize).copied(),
            model.vectors.get(surf.v_texture_v as usize).copied(),
        ) else {
            return full_bright;
        };
        let d = sub(p, base);
        let u = (dot(tu, d) - lm.pan[0]) / lm.u_scale + 0.5;
        let v = (dot(tv, d) - lm.pan[1]) / lm.v_scale + 0.5;
        let u = u.clamp(0.0, lm.u_clamp as f32);
        let v = v.clamp(0.0, lm.v_clamp as f32);
        [(r.x as f32 + u) / self.width as f32, (r.y as f32 + v) / self.height as f32]
    }

    pub fn into_atlas(self) -> Atlas {
        Atlas { width: self.width, height: self.height, rgba: self.rgba }
    }
}
