//! Trees and bushes grown from a species description by space colonisation (Runions et al. 2007): attraction
//! points fill the crown's envelope, the branches grow toward the points near them and use them up, the radii
//! follow the pipe model (a parent as thick as its children together), the bark is a tapering tube along each
//! branch and the leaves are clusters of crossed cards at the twigs. Several variants of a kind come from different
//! seeds. Y-up, metres, the trunk's foot at the origin.
//!
//! Per vertex the meshes carry how far out along the plant it is (0 at the foot, 1 at the outermost twigs) and a
//! phase, for a shader that sways it in the wind ([`MeshData::sway`]).

use crate::rand;
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// A kind of tree or bush.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Species {
    pub name: String,
    /// Overall height (m).
    pub height: f32,
    /// Bare trunk before the crown starts, as a fraction of the height (0 for a bush: branches from the ground).
    pub trunk: f32,
    /// The crown's envelope: half widths (x z) and half height as fractions of the height; `cone` 0 = ellipsoid,
    /// 1 = a cone narrowing to the top (firs).
    pub crown: [f32; 2],
    pub cone: f32,
    /// Attraction points in the crown.
    pub points: usize,
    /// Growth step, influence and kill distances, as fractions of the height.
    pub step: f32,
    pub influence: f32,
    pub kill: f32,
    /// Twig radius (m) and the pipe model's exponent.
    pub twig: f32,
    pub pipe: f32,
    /// Upward pull on growth (0 none).
    pub tropism: f32,
    /// Leaf cards: size (m), how many clusters per twig, and on branches thinner than this (m).
    pub leaf: f32,
    pub leaves_per_twig: usize,
    pub leaf_radius: f32,
    /// Bark sides on the trunk (fewer on thin branches).
    pub sides: usize,
    /// The crown as clumps: this many ellipsoid lobes inside the envelope (0 = the whole envelope, one mass). Real
    /// crowns are a few clusters of foliage on the main limbs with gaps between, not a ball.
    #[serde(default)]
    pub lobes: usize,
    /// How much the trunk wanders as it climbs (0 straight, ~0.3 a clear bend or two).
    #[serde(default)]
    pub bend: f32,
    /// Root flare: the trunk's foot this much wider (0.6 = 60%), easing out over the first stretch.
    #[serde(default)]
    pub flare: f32,
    /// Bark cross-section irregularity on thick wood (0 round; ~0.12 lumpy, ridged).
    #[serde(default)]
    pub gnarl: f32,
}

impl Species {
    /// A broadleaf park tree (the plane trees and maples of a city park).
    pub fn park_tree() -> Species {
        Species {
            name: "park tree".into(),
            height: 7.5,
            trunk: 0.32,
            crown: [0.36, 0.34],
            cone: 0.0,
            points: 700,
            step: 0.035,
            influence: 0.26,
            kill: 0.05,
            twig: 0.02,
            pipe: 2.3,
            tropism: 0.15,
            leaf: 0.62,
            leaves_per_twig: 2,
            leaf_radius: 0.035,
            sides: 9,
            lobes: 5,
            bend: 0.32,
            flare: 0.7,
            gnarl: 0.13,
        }
    }

    /// A tall narrow tree (poplar / cypress).
    pub fn tall_tree() -> Species {
        Species { name: "tall tree".into(), height: 10.0, trunk: 0.2, crown: [0.17, 0.42], cone: 0.4, points: 650, step: 0.03, influence: 0.22, kill: 0.045, tropism: 0.35, leaf: 0.6, lobes: 6, bend: 0.18, ..Species::park_tree() }
    }

    /// A low round bush.
    pub fn bush() -> Species {
        Species {
            name: "bush".into(),
            height: 1.4,
            trunk: 0.0,
            crown: [0.55, 0.45],
            cone: 0.0,
            points: 260,
            step: 0.07,
            influence: 0.45,
            kill: 0.1,
            twig: 0.006,
            pipe: 2.2,
            tropism: 0.2,
            leaf: 0.36,
            leaves_per_twig: 2,
            leaf_radius: 0.02,
            sides: 5,
            lobes: 4,
            bend: 0.25,
            flare: 0.0,
            gnarl: 0.08,
        }
    }
}

/// A mesh: positions, normals, texture coordinates, per-vertex sway (how far out 0..1, phase 0..1), triangles.
#[derive(Clone, Debug, Default)]
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub sway: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

/// A grown plant: its bark and its leaves (alpha-tested cards).
#[derive(Clone, Debug, Default)]
pub struct Plant {
    pub bark: MeshData,
    pub leaves: MeshData,
    /// Bounds (min, max).
    pub bounds: [Vec3; 2],
}

struct Node {
    pos: Vec3,
    parent: Option<usize>,
    radius: f32,
    children: usize,
    /// Distance along the branches from the foot.
    dist: f32,
    /// Part of the trunk (the stem grown before the crown's colonisation).
    trunk: bool,
}

fn in_crown(s: &Species, p: Vec3, centre: Vec3, half: Vec3) -> bool {
    let mut q = (p - centre) / half;
    // A cone: narrower toward the top.
    let up = ((p.y - (centre.y - half.y)) / (2.0 * half.y)).clamp(0.0, 1.0);
    let w = 1.0 - s.cone * up;
    q.x /= w.max(0.05);
    q.z /= w.max(0.05);
    q.length_squared() <= 1.0
}

/// Grows one plant of the species from a seed.
pub fn grow(s: &Species, seed: u32) -> Plant {
    let mut rs = crate::hash(seed ^ 0xA511_E9B3);
    let h = s.height * (0.85 + 0.3 * rand(&mut rs));
    let half = Vec3::new(s.crown[0] * h, s.crown[1] * h, s.crown[0] * h * (0.8 + 0.4 * rand(&mut rs)));
    let base = s.trunk * h;
    let centre = Vec3::new(0.0, (base + half.y).min(h - half.y), 0.0);
    // The crown's lobes: clumps of foliage spread through the envelope (the top one highest, the rest round it).
    let mut lobes: Vec<(Vec3, Vec3)> = Vec::new();
    for k in 0..s.lobes {
        let a = k as f32 / s.lobes.max(1) as f32 * std::f32::consts::TAU + rand(&mut rs) * 1.1;
        let (rr, up) = if k == 0 { (0.15 * rand(&mut rs), 0.35 + 0.2 * rand(&mut rs)) } else { (0.45 + 0.25 * rand(&mut rs), -0.35 + 0.6 * rand(&mut rs)) };
        let c = centre + Vec3::new(a.cos() * rr * half.x, up * half.y, a.sin() * rr * half.z);
        let size = half * (0.38 + 0.2 * rand(&mut rs));
        lobes.push((c, Vec3::new(size.x, size.y * (0.7 + 0.25 * rand(&mut rs)), size.z)));
    }
    let in_lobes = |p: Vec3| lobes.is_empty() || lobes.iter().any(|(c, r)| ((p - *c) / *r).length_squared() <= 1.0);
    // Attraction points: rejection-sampled in the envelope's box, inside a lobe.
    let mut pts: Vec<Vec3> = Vec::with_capacity(s.points);
    let mut tries = 0;
    while pts.len() < s.points && tries < s.points * 60 {
        tries += 1;
        let p = centre + Vec3::new(rand(&mut rs) * 2.0 - 1.0, rand(&mut rs) * 2.0 - 1.0, rand(&mut rs) * 2.0 - 1.0) * half;
        if p.y > 0.05 * h && in_crown(s, p, centre, half) && in_lobes(p) {
            pts.push(p);
        }
    }
    let step = s.step * h;
    let influence = s.influence * h;
    let kill = s.kill * h;
    // The trunk, straight up to where the crown's points can reach it (a little lean).
    let lean = Vec3::new(rand(&mut rs) - 0.5, 0.0, rand(&mut rs) - 0.5) * 0.08;
    let mut nodes = vec![Node { pos: Vec3::ZERO, parent: None, radius: 0.0, children: 0, dist: 0.0, trunk: true }];
    // A bush: several stems from the ground.
    let stems = if s.trunk <= 0.0 { 3 + (rand(&mut rs) * 3.0) as usize } else { 1 };
    let mut tips: Vec<usize> = Vec::new();
    for k in 0..stems {
        let mut cur = 0usize;
        let a = k as f32 / stems as f32 * std::f32::consts::TAU + rand(&mut rs);
        let dir0 = if stems > 1 { (Vec3::Y + Vec3::new(a.cos(), 0.0, a.sin()) * 0.45).normalize() } else { (Vec3::Y + lean).normalize() };
        let target = if stems > 1 { step * 2.0 } else { base.max(step) };
        let mut y = 0.0;
        // The stem wanders: a slowly turning sideways drift (a bend or two, not a pole), kept from tipping over.
        let mut wander = Vec3::ZERO;
        let mut drift = Vec3::new(rand(&mut rs) - 0.5, 0.0, rand(&mut rs) - 0.5) * s.bend;
        while y < target {
            drift += Vec3::new(rand(&mut rs) - 0.5, 0.0, rand(&mut rs) - 0.5) * s.bend * 0.5;
            drift = drift.clamp_length_max(s.bend);
            wander = (wander + drift * 0.35).clamp_length_max(s.bend * 1.2);
            let dir = (dir0 + wander).normalize();
            let p = nodes[cur].pos + dir * step;
            let d = nodes[cur].dist + step;
            nodes.push(Node { pos: p, parent: Some(cur), radius: 0.0, children: 0, dist: d, trunk: true });
            nodes[cur].children += 1;
            cur = nodes.len() - 1;
            y += dir.y * step;
        }
        tips.push(cur);
    }
    // If no point is within reach of the trunk's top yet, keep climbing toward the crown.
    for _ in 0..60 {
        let top = nodes[*tips.last().unwrap()].pos;
        if pts.iter().any(|p| p.distance(top) < influence) {
            break;
        }
        let cur = *tips.last().unwrap();
        let p = top + (centre - top).normalize_or(Vec3::Y) * step;
        let d = nodes[cur].dist + step;
        nodes.push(Node { pos: p, parent: Some(cur), radius: 0.0, children: 0, dist: d, trunk: true });
        nodes[cur].children += 1;
        *tips.last_mut().unwrap() = nodes.len() - 1;
    }
    // Space colonisation.
    for _ in 0..220 {
        if pts.is_empty() {
            break;
        }
        let mut pull: Vec<(Vec3, u32)> = vec![(Vec3::ZERO, 0); nodes.len()];
        for p in &pts {
            let mut best: Option<(f32, usize)> = None;
            for (i, n) in nodes.iter().enumerate() {
                let d = n.pos.distance(*p);
                if d < influence && best.is_none_or(|b| d < b.0) {
                    best = Some((d, i));
                }
            }
            if let Some((_, i)) = best {
                pull[i].0 += (*p - nodes[i].pos).normalize_or_zero();
                pull[i].1 += 1;
            }
        }
        let mut grew = false;
        let count = nodes.len();
        for (i, (dir, n)) in pull.into_iter().enumerate().take(count) {
            if n == 0 {
                continue;
            }
            let dir = (dir.normalize_or_zero() + Vec3::Y * s.tropism).normalize_or(Vec3::Y);
            let p = nodes[i].pos + dir * step;
            if nodes.iter().any(|m| m.pos.distance(p) < step * 0.3) {
                continue;
            }
            let d = nodes[i].dist + step;
            nodes.push(Node { pos: p, parent: Some(i), radius: 0.0, children: 0, dist: d, trunk: false });
            nodes[i].children += 1;
            grew = true;
        }
        let ns = &nodes;
        pts.retain(|p| !ns.iter().any(|n| n.pos.distance(*p) < kill));
        if !grew {
            break;
        }
    }
    // Pipe model radii, from the twigs in.
    for i in (0..nodes.len()).rev() {
        if nodes[i].children == 0 {
            nodes[i].radius = s.twig;
        }
        if let Some(p) = nodes[i].parent {
            let r = nodes[i].radius.powf(s.pipe);
            nodes[p].radius = (nodes[p].radius.powf(s.pipe) + r).powf(1.0 / s.pipe);
        }
    }
    let max_dist = nodes.iter().map(|n| n.dist).fold(0.0f32, f32::max).max(1e-3);
    let mut plant = Plant::default();
    // Bark: a ring at every node, joined to its parent's ring (each branch a tube; a joint shares its parent's ring).
    // Each node's ring and its number of sides (a child of the same thickness class joins its parent's ring).
    let mut ring_of: Vec<(u32, usize)> = vec![(u32::MAX, 0); nodes.len()];
    // Lumps and ridges on thick wood: two slow lobed waves round the section that drift along the branch, so the
    // section changes shape as it climbs (and the trunk isn't a lathe-turned pole).
    let (g1, g2) = (rand(&mut rs) * 6.28, rand(&mut rs) * 6.28);
    let ring = |plant: &mut Plant, i: usize, dir: Vec3, sides: usize, phase: f32| -> u32 {
        let n = &nodes[i];
        let rot = Quat::from_rotation_arc(Vec3::Y, dir.normalize_or(Vec3::Y));
        let first = plant.bark.positions.len() as u32;
        // The root flare: a trunk's foot spreads out.
        let flare = if n.trunk { 1.0 + s.flare * (-n.pos.y / (0.045 * h).max(0.05)).exp() } else { 1.0 };
        let gn = s.gnarl * ((n.radius - 0.03) / 0.08).clamp(0.0, 1.0) * if n.trunk && n.pos.y < 0.1 * h { 1.6 } else { 1.0 };
        let (p1, p2) = (g1 + n.dist * 1.7, g2 - n.dist * 2.3);
        for k in 0..=sides {
            let a = k as f32 / sides as f32 * std::f32::consts::TAU;
            let lump = 1.0 + gn * (0.6 * (3.0 * a + p1).sin() + 0.4 * (5.0 * a + p2).sin());
            let out = rot * Vec3::new(a.cos(), 0.0, a.sin());
            plant.bark.positions.push((n.pos + out * n.radius * flare * lump).to_array());
            plant.bark.normals.push(out.to_array());
            plant.bark.uvs.push([k as f32 / sides as f32 * (n.radius * 40.0).clamp(1.0, 4.0).round(), n.dist / (2.0 * std::f32::consts::PI * 0.15)]);
            plant.bark.sway.push([n.dist / max_dist, phase]);
        }
        first
    };
    for i in 1..nodes.len() {
        let Some(p) = nodes[i].parent else { continue };
        let dir = nodes[i].pos - nodes[p].pos;
        // Thin branches get fewer sides; twigs thinner than a centimetre are left to the leaves.
        if nodes[i].radius < 0.004 {
            continue;
        }
        let sides = if nodes[i].radius > 0.05 { s.sides } else if nodes[i].radius > 0.015 { 5 } else { 3 };
        let phase = (crate::hash(i as u32 ^ seed) & 0xFFFF) as f32 / 65535.0;
        let a = if ring_of[p].0 != u32::MAX && ring_of[p].1 == sides {
            ring_of[p].0
        } else {
            ring(&mut plant, p, dir, sides, phase)
        };
        let b = ring(&mut plant, i, dir, sides, phase);
        ring_of[i] = (b, sides);
        for k in 0..sides as u32 {
            plant.bark.indices.extend([a + k, b + k, a + k + 1, a + k + 1, b + k, b + k + 1]);
        }
    }
    // Leaves: clusters of three crossed cards at the twigs and thin branches.
    for (i, n) in nodes.iter().enumerate() {
        if n.radius > s.leaf_radius || n.parent.is_none() || n.pos.y < s.trunk * h * 0.8 {
            continue;
        }
        let mut ls = crate::hash((i as u32).wrapping_mul(0x9E37_79B9) ^ seed);
        let count = if n.children == 0 { s.leaves_per_twig } else { 1 };
        for _ in 0..count {
            let size = s.leaf * (0.7 + 0.6 * rand(&mut ls));
            let mut at = n.pos + Vec3::new(rand(&mut ls) - 0.5, rand(&mut ls) - 0.5, rand(&mut ls) - 0.5) * size * 0.3;
            at.y = at.y.max(size * 0.35);
            // The clump it belongs to (the nearest lobe; else the whole crown).
            let mid = lobes
                .iter()
                .map(|(c, r)| (((at - *c) / *r).length_squared(), *c))
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map_or(centre, |l| l.1);
            // Leaves stay inside the clump's skin: pulled in where the card would stand out of it (no balloon).
            if let Some((c, r)) = lobes.iter().min_by(|a, b| ((at - a.0) / a.1).length_squared().total_cmp(&((at - b.0) / b.1).length_squared())) {
                let q = ((at - *c) / *r).length();
                if q > 0.85 {
                    at = *c + (at - *c) * (0.85 / q);
                }
            }
            let yaw = rand(&mut ls) * std::f32::consts::TAU;
            let tilt = (rand(&mut ls) - 0.5) * 0.9;
            let rot = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(tilt);
            let phase = rand(&mut ls);
            // Which quarter of the leaf atlas.
            let cell = (rand(&mut ls) * 4.0) as u32 % 4;
            let (u0, v0) = ((cell % 2) as f32 * 0.5, (cell / 2) as f32 * 0.5);
            for card in 0..3 {
                let r = rot * Quat::from_rotation_y(card as f32 * std::f32::consts::PI / 3.0);
                let (ax, ay) = (r * Vec3::X * size * 0.5, r * Vec3::Y * size * 0.5);
                let nrm = (r * Vec3::Z + Vec3::Y * 0.6).normalize();
                let first = plant.leaves.positions.len() as u32;
                for (cx, cy, u, v) in [(-1.0, -1.0, 0.0, 1.0), (1.0, -1.0, 1.0, 1.0), (1.0, 1.0, 1.0, 0.0), (-1.0, 1.0, 0.0, 0.0)] {
                    let p = at + ax * cx + ay * cy;
                    plant.leaves.positions.push(p.to_array());
                    // Normals bent outward from the middle of its clump: each clump shades like a mass of its own.
                    let out = ((p - mid).normalize_or_zero() * 0.7 + nrm * 0.3).normalize_or(Vec3::Y);
                    plant.leaves.normals.push(out.to_array());
                    plant.leaves.uvs.push([u0 + u * 0.5, v0 + v * 0.5]);
                    plant.leaves.sway.push([(n.dist / max_dist).max(0.6), phase]);
                }
                plant.leaves.indices.extend([first, first + 1, first + 2, first, first + 2, first + 3]);
            }
        }
    }
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for p in plant.bark.positions.iter().chain(&plant.leaves.positions) {
        lo = lo.min(Vec3::from(*p));
        hi = hi.max(Vec3::from(*p));
    }
    plant.bounds = [lo, hi];
    plant
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grows_a_tree_and_a_bush() {
        for s in [Species::park_tree(), Species::tall_tree(), Species::bush()] {
            let p = grow(&s, 7);
            assert!(p.bark.indices.len() > 100, "{} bark {}", s.name, p.bark.indices.len());
            assert!(p.leaves.indices.len() > 100, "{} leaves {}", s.name, p.leaves.indices.len());
            assert!(p.bounds[1].y > s.height * 0.5 && p.bounds[1].y < s.height * 1.6, "{} height {}", s.name, p.bounds[1].y);
            assert!(p.bark.indices.iter().all(|&i| (i as usize) < p.bark.positions.len()));
        }
    }

    #[test]
    fn same_seed_same_plant() {
        let a = grow(&Species::bush(), 3);
        let b = grow(&Species::bush(), 3);
        assert_eq!(a.bark.positions.len(), b.bark.positions.len());
        assert_eq!(a.leaves.positions, b.leaves.positions);
    }
}
