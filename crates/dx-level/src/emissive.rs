//! Emissive Surfaces (dxview's setting EmissiveSurfaces; ROADMAP 52): glowing BSP faces light what's around them.
//!
//! Faces drawn full bright (PF_Unlit: light fittings, screens, signs) and textures named as glowing (lamp panels,
//! neon, monitors) become small area lights when the map is built. Each face is cut into pieces about SPACING
//! across, each piece an emitter in the texture's average colour that shines out of the face (Lambert, both ways for
//! a two-sided face) with Unreal's linear falloff. Their light is added into the baked lightmap, traced against the
//! BSP so walls keep it in, and one light per glowing surface joins the level's static lights for meshes (people,
//! decorations) as a `LightInfo::emitter` light. Nothing runs per frame: the cost is a little more bake at load.
//! With the setting off nothing here runs and the lightmap is the original's.

use crate::collision::BspCollision;
use crate::lighting::{Baked, LightInfo, LIGHT_GAIN};
use glam::DVec3;
use std::sync::RwLock;

static SWITCH: RwLock<Option<fn() -> bool>> = RwLock::new(None);

/// dxview hands over its setting (read at every map build).
pub fn set_switch(f: fn() -> bool) {
    if let Ok(mut g) = SWITCH.write() {
        *g = Some(f);
    }
}

/// Glowing surfaces light their surroundings in the next map built (DXL_EMIT=1 turns them on for the examples).
pub fn on() -> bool {
    SWITCH.read().ok().and_then(|g| *g).is_some_and(|f| f()) || std::env::var_os("DXL_EMIT").is_some_and(|v| v == "1")
}

/// Parts of texture names that glow (lower case): light fittings and panels, screens, monitors, neon, lit signs.
pub const GLOW_NAMES: &[&str] = &[
    "lightpanel", "lite_", "lights_", "lamp", "neon", "glow", "screen", "monitor", "lcd", "lightbox", "litesign",
    "exit_sign", "exitsign", "lightfix", "flourescent", "fluorescent",
];

/// A texture whose name says it glows (not the sky's, nor glass).
pub fn glow_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    !n.contains("sky") && !n.contains("glass") && GLOW_NAMES.iter().any(|w| n.contains(w))
}

/// Parts of a texture's own name that make a full-bright face a light source, and parts that make it a plain
/// surface the designers drew full bright (white paint, pipes, metal walls, ladders: lighting them up as lamps
/// flooded Hong Kong's market and Battery Park with white light).
const SOURCE_WORDS: &[&str] = &["light", "lite", "lght", "lamp", "lens", "sign", "screen", "monitor", "noise", "neon", "led_", "logo", "consol", "compu", "lcd", "glow", "chart"];
/// Lights that blink or flash (animated: a steady glow would be wrong) stay out.
const BLINK_WORDS: &[&str] = &["flash", "cycle", "blink", "strobe"];
const PLAIN_WORDS: &[&str] = &["paint", "wall", "drywa", "ladder", "pipe", "plain", "brick", "concrete", "stone", "wood", "floor", "metl", "baner", "banner", "heli_"];
/// Texture groups whose full-bright faces are light sources (lens panels, signs, screens).
const SOURCE_GROUPS: &[&str] = &["glass", "signs", "lights", "light", "misc", "screens", "monitors"];

/// Whether a face with this texture ("Package.Group.Name") glows and lights its surroundings: a full-bright one
/// (`unlit`) that looks like a light source, or any one named as glowing.
pub fn glows(path: &str, unlit: bool) -> bool {
    let lower = path.to_ascii_lowercase();
    if lower.contains("sky") || BLINK_WORDS.iter().any(|w| lower.contains(w)) {
        return false;
    }
    if !unlit {
        return glow_name(&lower);
    }
    let parts: Vec<&str> = lower.split('.').collect();
    let name = parts.last().copied().unwrap_or("");
    if SOURCE_WORDS.iter().any(|w| name.contains(w)) {
        return true;
    }
    if PLAIN_WORDS.iter().any(|w| name.contains(w)) {
        return false;
    }
    // "Package.Name" (no group) or a group of lights, glass panels, signs and screens.
    parts.len() < 3 || SOURCE_GROUPS.contains(&parts[parts.len() - 2])
}

/// How bright a glowing face's light is next to it (Unreal brightness, 0..1, x the texture's colour), and how far it
/// reaches per unit of the square root of its area (clamped to MIN_R..MAX_R).
const BRIGHT: f32 = 0.12;
/// Light leaving a face (and reaching a surface) at a grazing angle is wrapped by this much, so a sign flush with a
/// wall still spills a little light onto the wall round it, as a real one's box and glow would.
const WRAP: f64 = 0.4;
const REACH: f32 = 5.5;
const MIN_R: f32 = 128.0;
const MAX_R: f32 = 512.0;
/// The size of a face's pieces (Unreal units across).
const SPACING: f32 = 96.0;
/// At most this many pieces a surface (a huge face spreads its light over them).
const MAX_PIECES: usize = 24;
/// Faces darker than this (brightest channel of their average colour) don't light anything.
const MIN_COLOR: f32 = 0.1;

/// One face of a glowing surface, as found while building the level (Unreal units).
#[derive(Clone, Debug)]
pub struct GlowFace {
    pub surf: usize,
    pub points: Vec<[f32; 3]>,
    pub normal: [f32; 3],
    /// The texture's average colour (0..1), times how much of it isn't see-through.
    pub color: [f32; 3],
    pub two_sided: bool,
}

/// One piece of a glowing surface.
#[derive(Clone, Copy, Debug)]
pub struct Emitter {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub two_sided: bool,
    /// Colour x brightness (as LightInfo's colour).
    pub color: [f32; 3],
    pub radius: f32,
}

fn gain() -> f32 {
    std::env::var("DXL_EMIT_GAIN").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0)
}

fn v3(a: [f32; 3]) -> DVec3 {
    DVec3::new(a[0] as f64, a[1] as f64, a[2] as f64)
}

/// A glowing face with a placed light this close to it is a lamp the designers already lit: it adds only a little
/// light of its own, in that light's colour (their colour grading of the room stays: the training's blue medical
/// rooms went grey-white under their white ceiling panels).
const LAMP_NEAR: f64 = 160.0;
const LAMP_SHARE: f32 = 0.35;
/// Below this saturation a glowing face counts as a white lamp.
const LAMP_SAT: f32 = 0.35;

/// The glowing faces cut into emitters, and one light per glowing surface for meshes. `statics` = the level's
/// placed lights.
pub fn emitters(faces: &[GlowFace], statics: &[LightInfo]) -> (Vec<Emitter>, Vec<LightInfo>) {
    let k = gain();
    let mut by_surf: std::collections::BTreeMap<usize, Vec<&GlowFace>> = Default::default();
    for f in faces {
        by_surf.entry(f.surf).or_default().push(f);
    }
    let mut out = Vec::new();
    let mut lights = Vec::new();
    for list in by_surf.values() {
        // Small triangles (area, middle) covering the faces.
        let mut bits: Vec<(f64, DVec3)> = Vec::new();
        for f in list {
            let p: Vec<DVec3> = f.points.iter().map(|&q| v3(q)).collect();
            for i in 1..p.len().saturating_sub(1) {
                split(p[0], p[i], p[i + 1], &mut bits, 0);
            }
        }
        let area: f64 = bits.iter().map(|b| b.0).sum();
        if area < 16.0 {
            continue;
        }
        let f0 = list[0];
        // The surface's faces share a texture, so its colour.
        let mut color = f0.color;
        let level = color[0].max(color[1]).max(color[2]);
        if level < MIN_COLOR {
            continue;
        }
        let middle = bits.iter().fold(DVec3::ZERO, |m, b| m + b.1 * b.0) / area;
        let mut b = BRIGHT * k;
        // A white lamp already lit by a placed light: that light's colour, at a fraction of the strength. (A coloured
        // sign or screen keeps its own colour: a street lamp beside a green road sign isn't the sign's light.)
        let sat = (level - color[0].min(color[1]).min(color[2])) / level;
        if let Some(l) = statics
            .iter()
            .filter(|l| sat < LAMP_SAT && !l.emitter && l.color.iter().any(|&c| c > 0.0))
            .map(|l| (v3(l.pos).distance(middle), l))
            .filter(|(d, _)| *d < LAMP_NEAR)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, l)| l)
        {
            let m = l.color[0].max(l.color[1]).max(l.color[2]).max(1e-6);
            color = [l.color[0] / m * level, l.color[1] / m * level, l.color[2] / m * level];
            b *= LAMP_SHARE;
        }
        let n = v3(f0.normal).normalize_or_zero();
        let radius = (REACH * (area as f32).sqrt()).clamp(MIN_R, MAX_R);
        // Pieces: the triangles gathered into clusters about SPACING across, largest first.
        bits.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut cl: Vec<(f64, DVec3)> = Vec::new(); // (area, area x position)
        let s = SPACING as f64 * 0.75;
        for (a, c) in bits {
            match cl.iter_mut().find(|(ca, cp)| (*cp / *ca).distance(c) < s) {
                Some(e) => {
                    e.0 += a;
                    e.1 += c * a;
                }
                None => cl.push((a, c * a)),
            }
        }
        // Too many: the smallest go into their nearest neighbour.
        while cl.len() > MAX_PIECES {
            let (i, _) = cl.iter().enumerate().min_by(|a, b| a.1 .0.total_cmp(&b.1 .0)).unwrap();
            let (a, p) = cl.swap_remove(i);
            let c = p / a;
            let j = (0..cl.len()).min_by(|&x, &y| (cl[x].1 / cl[x].0).distance(c).total_cmp(&(cl[y].1 / cl[y].0).distance(c))).unwrap();
            cl[j].0 += a;
            cl[j].1 += p;
        }
        let mut mid = DVec3::ZERO;
        for (a, p) in &cl {
            let share = (*a / area) as f32;
            let c = *p / *a;
            mid += *p;
            out.push(Emitter {
                pos: c.as_vec3().to_array(),
                normal: n.as_vec3().to_array(),
                two_sided: f0.two_sided,
                color: [color[0] * b * share, color[1] * b * share, color[2] * b * share],
                radius,
            });
        }
        // For meshes: one light a little out from the surface's middle, shining out of it.
        let mid = mid / area + if f0.two_sided { DVec3::ZERO } else { n * 4.0 };
        lights.push(LightInfo {
            pos: mid.as_vec3().to_array(),
            color: [color[0] * b, color[1] * b, color[2] * b],
            radius,
            dir: if f0.two_sided { [0.0; 3] } else { n.as_vec3().to_array() },
            emitter: true,
            ..Default::default()
        });
    }
    (out, lights)
}

/// Cuts a triangle until its pieces are no bigger than SPACING squared.
fn split(a: DVec3, b: DVec3, c: DVec3, out: &mut Vec<(f64, DVec3)>, depth: u32) {
    let area = 0.5 * (b - a).cross(c - a).length();
    if area < 1e-3 {
        return;
    }
    let s = SPACING as f64;
    if area <= s * s * 0.5 || depth > 12 {
        out.push((area, (a + b + c) / 3.0));
        return;
    }
    // Across the longest edge.
    let (ab, bc, ca) = (a.distance(b), b.distance(c), c.distance(a));
    if ab >= bc && ab >= ca {
        let m = (a + b) * 0.5;
        split(a, m, c, out, depth + 1);
        split(m, b, c, out, depth + 1);
    } else if bc >= ca {
        let m = (b + c) * 0.5;
        split(a, b, m, out, depth + 1);
        split(a, m, c, out, depth + 1);
    } else {
        let m = (c + a) * 0.5;
        split(a, b, m, out, depth + 1);
        split(m, b, c, out, depth + 1);
    }
}

/// A texture's average colour (0..1) times how much of it is solid, from its smallest stored mip.
pub fn average_color(t: &crate::TextureImage) -> Option<[f32; 3]> {
    let (k, m) = t.mips.iter().enumerate().last()?;
    let w = (t.width >> k).max(1) as usize;
    let h = (t.height >> k).max(1) as usize;
    let rgba: Vec<u8> = if t.bc1 || t.bc3 {
        // BC3's colour block is the second half of each 16 bytes.
        let blocks: Vec<u8> = if t.bc3 { m.chunks_exact(16).flat_map(|b| b[8..16].to_vec()).collect() } else { m.clone() };
        let mut out = vec![0u8; w * h * 4];
        unreal_pkg::objects::decode_dxt1(&blocks, w, h, &mut out)?;
        out
    } else {
        m.clone()
    };
    // The colour of its bright parts (each texel weighted by its brightness squared: an EXIT sign's red letters,
    // not its dark plate), at the square root of its average brightness (a screen that's mostly dark still glows).
    let (mut hue, mut wsum, mut level, mut n) = ([0.0f64; 3], 0.0f64, 0.0f64, 0usize);
    for px in rgba.chunks_exact(4) {
        n += 1;
        if px[3] < 128 {
            continue;
        }
        let c = [px[0] as f64 / 255.0, px[1] as f64 / 255.0, px[2] as f64 / 255.0];
        let m = c[0].max(c[1]).max(c[2]);
        level += m;
        let w = m * m;
        wsum += w;
        for k in 0..3 {
            hue[k] += c[k] * w;
        }
    }
    if n == 0 || wsum <= 1e-9 {
        return None;
    }
    let peak = (hue[0].max(hue[1]).max(hue[2]) / wsum).max(1e-6);
    let level = (level / n as f64).sqrt();
    Some([0, 1, 2].map(|k| (hue[k] / wsum / peak * level) as f32))
}

/// Adds the emitters' light into the bake's lightmap. Returns the number of texels that got some.
pub fn add_to_bake(baked: &mut Baked, em: &[Emitter], col: &BspCollision) -> usize {
    if em.is_empty() {
        return 0;
    }
    let patches = &baked.patches;
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(1, 8);
    let chunk = patches.len().div_ceil(threads).max(1);
    // Per patch: the light to add to each of its texels (u_clamp x v_clamp), only for patches that got some.
    let results: Vec<(usize, Vec<[f32; 3]>)> = std::thread::scope(|sc| {
        let jobs: Vec<_> = patches
            .chunks(chunk)
            .enumerate()
            .map(|(ci, ch)| {
                sc.spawn(move || {
                    let mut out = Vec::new();
                    for (k, p) in ch.iter().enumerate() {
                        if let Some(add) = light_patch(p, em, col) {
                            out.push((ci * chunk + k, add));
                        }
                    }
                    out
                })
            })
            .collect();
        jobs.into_iter().flat_map(|j| j.join().unwrap_or_default()).collect()
    });
    let w = baked.width as usize;
    let mut texels = 0;
    for (pi, add) in results {
        let p = &baked.patches[pi];
        let [ax, ay, aw, ah] = p.rect;
        let (u, v) = ((aw - 2).max(1) as usize, (ah - 2).max(1) as usize);
        texels += add.iter().filter(|a| a.iter().any(|&c| c > 0.0)).count();
        // The block with its replicated border, as the bake wrote it.
        for yy in 0..ah as usize {
            for xx in 0..aw as usize {
                let sx = xx.saturating_sub(1).min(u - 1);
                let sy = yy.saturating_sub(1).min(v - 1);
                let a = add[sy * u + sx];
                let o = ((ay as usize + yy) * w + ax as usize + xx) * 4;
                for c in 0..3 {
                    let n = baked.rgba[o + c] as f32 + a[c] * LIGHT_GAIN * 255.0;
                    baked.rgba[o + c] = n.round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
    texels
}

/// The emitters' light on one surface's texels (None if none reaches it).
fn light_patch(p: &crate::lighting::Patch, em: &[Emitter], col: &BspCollision) -> Option<Vec<[f32; 3]>> {
    let pn = v3(p.normal);
    let near: Vec<&Emitter> = em
        .iter()
        .filter(|e| {
            // In reach of the texels' box, in front of the surface (either side of a two-sided one)
            let mut d2 = 0.0f32;
            for k in 0..3 {
                let c = e.pos[k].clamp(p.min[k], p.max[k]) - e.pos[k];
                d2 += c * c;
            }
            if d2 >= e.radius * e.radius {
                return false;
            }
            let pd = (v3(e.pos) - v3(p.origin)).dot(pn);
            // (a wall the glowing face is set into counts: it gets the wrapped spill).
            p.two_sided || pd > -1.0
        })
        .collect();
    if near.is_empty() {
        return None;
    }
    let [_, _, aw, ah] = p.rect;
    let (u, v) = ((aw - 2).max(1) as usize, (ah - 2).max(1) as usize);
    let (o, du, dv) = (v3(p.origin), v3(p.du), v3(p.dv));
    let at = |tx: usize, ty: usize| o + du * tx as f64 + dv * ty as f64;
    let mut add = vec![[0.0f32; 3]; u * v];
    let mut any = false;
    // Sight is traced on every other texel (grid point (i, j) = texel (2i, 2j), the last row and column clamped)
    // and blended between them: a quarter of the traces, and shadow edges as soft as the bake's tent filter.
    let (gw, gh) = (u.div_ceil(2) + 1, v.div_ceil(2) + 1);
    let mut vis: Vec<Option<f32>> = Vec::new();
    for e in &near {
        let ep = v3(e.pos);
        let en = v3(e.normal);
        vis.clear();
        vis.resize(gw * gh, None);
        // The light a texel at q gets before shadows (None: none).
        let term = |q: DVec3| -> Option<(f32, f64, f64)> {
            let to = ep - q;
            let d = to.length();
            if d >= e.radius as f64 || d < 1e-3 {
                return None;
            }
            let dir = to / d;
            let (raw_r, raw_e) = (dir.dot(pn), -dir.dot(en));
            let cos_r = if p.two_sided { raw_r.abs() } else { raw_r };
            let cos_e = if e.two_sided { raw_e.abs() } else { raw_e };
            let wrap = |c: f64| ((c + WRAP) / (1.0 + WRAP)).max(0.0);
            let f = (1.0 - d as f32 / e.radius) * (wrap(cos_r) * wrap(cos_e)) as f32;
            // Nothing behind the glowing face (one-sided), nor on the back of a one-sided surface.
            if f <= 0.0 || (!e.two_sided && (q - ep).dot(en) < -1.0) || (!p.two_sided && (ep - q).dot(pn) < -1.0) {
                return None;
            }
            Some((f, raw_r, raw_e))
        };
        let seen = |i: usize, j: usize, vis: &mut Vec<Option<f32>>| -> f32 {
            let k = j * gw + i;
            if let Some(s) = vis[k] {
                return s;
            }
            let q = at((2 * i).min(u - 1), (2 * j).min(v - 1));
            let s = match term(q) {
                // Out of the glowing face to just off this surface, on the sides that face each other.
                Some((_, raw_r, raw_e)) => {
                    let side_e = if e.two_sided && raw_e < 0.0 { -en } else { en };
                    let side_r = if p.two_sided && raw_r < 0.0 { -pn } else { pn };
                    if col.trace(ep + side_e * 1.5, q + side_r * 1.0, DVec3::ZERO).fraction >= 0.999 { 1.0 } else { 0.0 }
                }
                None => 0.0,
            };
            vis[k] = Some(s);
            s
        };
        let peak = e.color[0].max(e.color[1]).max(e.color[2]) * LIGHT_GAIN * 255.0;
        for ty in 0..v {
            for tx in 0..u {
                let Some((f, _, _)) = term(at(tx, ty)) else { continue };
                if f * peak < 0.5 {
                    continue;
                }
                let (i0, j0) = (tx / 2, ty / 2);
                let (fx, fy) = ((tx % 2) as f32 * 0.5, (ty % 2) as f32 * 0.5);
                let mut s = seen(i0, j0, &mut vis) * (1.0 - fx) * (1.0 - fy);
                if fx > 0.0 {
                    s += seen(i0 + 1, j0, &mut vis) * fx * (1.0 - fy);
                }
                if fy > 0.0 {
                    s += seen(i0, j0 + 1, &mut vis) * (1.0 - fx) * fy;
                }
                if fx > 0.0 && fy > 0.0 {
                    s += seen(i0 + 1, j0 + 1, &mut vis) * fx * fy;
                }
                if s <= 0.0 {
                    continue;
                }
                any = true;
                let a = &mut add[ty * u + tx];
                for c in 0..3 {
                    a[c] += e.color[c] * f * s;
                }
            }
        }
    }
    any.then_some(add)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn which_faces_glow() {
        // Full-bright light sources by name or group...
        assert!(glows("CoreTexMisc.Metal.Exitsign_a", true));
        assert!(glows("UNATCO.Glass.un_insetlight_b", true));
        assert!(glows("HK_Signs.Metal.HK_Sign_CNRD1", true));
        assert!(glows("CoreTexMisc.Glass.Med_Screen_A01", true));
        // ...not full-bright plain surfaces.
        assert!(!glows("CoreTexPaper.Paper.ClenWhitPaint_A", true));
        assert!(!glows("HK_Helibase.Metal.Metal_A", true));
        assert!(!glows("CoreTexMetal.Ladder.ladder_a", true));
        assert!(!glows("HK_Helibase.Pipe_B", true));
        // Lit faces only by a glowing name; never the sky.
        assert!(glows("UNATCO.Lights.LightPanel_A", false));
        assert!(!glows("CoreTexMisc.Glass.Med_Screen_A01", false));
        assert!(!glows("Sky.Lights.SkyLamp", true));
    }

    #[test]
    fn pieces_share_the_light() {
        // A 256 x 64 panel facing down: several pieces whose colours add up to the whole panel's light.
        let f = GlowFace {
            surf: 0,
            points: vec![[0.0, 0.0, 100.0], [256.0, 0.0, 100.0], [256.0, 64.0, 100.0], [0.0, 64.0, 100.0]],
            normal: [0.0, 0.0, -1.0],
            color: [1.0, 0.5, 0.25],
            two_sided: false,
        };
        let (em, lights) = emitters(&[f], &[]);
        assert!(em.len() >= 2 && em.len() <= MAX_PIECES);
        assert_eq!(lights.len(), 1);
        let sum: f32 = em.iter().map(|e| e.color[0]).sum();
        assert!((sum - lights[0].color[0]).abs() < 1e-4);
        assert!(lights[0].emitter && lights[0].dir == [0.0, 0.0, -1.0]);
        // The mesh light sits out in front of the panel, in its middle.
        assert!((lights[0].pos[0] - 128.0).abs() < 1.0 && lights[0].pos[2] < 100.0);
        assert!(em.iter().all(|e| (e.pos[2] - 100.0).abs() < 1e-3));
    }

    #[test]
    fn colour_of_the_bright_parts() {
        // Half black, half pure red: the light is red, at the square root of the average brightness.
        let mut px = Vec::new();
        for i in 0..16 {
            px.extend_from_slice(if i < 8 { &[0, 0, 0, 255] } else { &[255, 0, 0, 255] });
        }
        let t = crate::TextureImage { name: "x".into(), width: 4, height: 4, mips: vec![px], bc1: false, bc3: false };
        let c = average_color(&t).unwrap();
        assert!((c[0] - 0.5f32.sqrt()).abs() < 1e-3 && c[1] < 1e-6 && c[2] < 1e-6);
    }
}
