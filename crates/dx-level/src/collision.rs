//! Box sweeps against the level BSP, the way Unreal 1 collides actors with world geometry.
//!
//! Unreal treats an actor's collision cylinder as an axis-aligned box (radius, radius, height) when it
//! moves through the BSP. Every BSP node that bounds solid space can carry a "leaf hull": the list of
//! planes enclosing a convex solid piece plus its bounding box. Sweeping a box against a convex hull is
//! a line-vs-convex clip with each plane pushed out by the box's support distance, plus bevel planes at
//! hull edges so corners don't let the box sink in.
//!
//! The structure follows SurrealEngine's `TraceAABBModel` (zlib license, (c) Magnus Norddahl and
//! contributors), rewritten in Rust; all coordinates here are Unreal units, Z up.

use glam::{DVec3, DVec4};
use unreal_pkg::objects::Model;

#[derive(Debug, Clone, Copy)]
pub struct Hit {
    /// 0..1 fraction of the requested move that is free.
    pub fraction: f64,
    pub normal: DVec3,
}

impl Hit {
    pub const NONE: Hit = Hit { fraction: 1.0, normal: DVec3::ZERO };
    pub fn blocked(&self) -> bool {
        self.fraction < 1.0
    }
}

#[derive(Clone)]
struct Node {
    plane: DVec4,
    front: i32,
    back: i32,
    collision_bound: i32,
    /// Index into `hulls` of this node's precomputed leaf hull (-1 = none or unreadable).
    hull: i32,
    zone: [i32; 2],
}

/// A leaf hull prepared once at load: its (shrunk) bounding box and every clip plane in the order the sweep
/// applies them: the six box planes, the hull's own planes, then the edge bevels. The bevels depend only on
/// the hull, so building them here instead of on every trace is where most of the speed-up comes from.
#[derive(Clone)]
struct Hull {
    bmin: DVec3,
    bmax: DVec3,
    first: u32,
    count: u32,
}

#[derive(Clone)]
pub struct BspCollision {
    nodes: Vec<Node>,
    leaf_hulls: Vec<i32>,
    hulls: Vec<Hull>,
    planes: Vec<DVec4>,
}

thread_local! {
    static STACK: std::cell::RefCell<Vec<i32>> = const { std::cell::RefCell::new(Vec::new()) };
}

impl std::fmt::Debug for BspCollision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BspCollision({} nodes)", self.nodes.len())
    }
}

impl BspCollision {
    pub fn new(model: &Model) -> BspCollision {
        let nodes = model
            .nodes
            .iter()
            .map(|n| Node {
                plane: DVec4::new(n.plane[0] as f64, n.plane[1] as f64, n.plane[2] as f64, n.plane[3] as f64),
                front: n.front,
                back: n.back,
                collision_bound: n.collision_bound,
                hull: -1,
                zone: n.zone,
            })
            .collect();
        let mut c = BspCollision { nodes, leaf_hulls: model.leaf_hulls.clone(), hulls: Vec::new(), planes: Vec::new() };
        let mut by_offset = std::collections::HashMap::new();
        for i in 0..c.nodes.len() {
            let cb = c.nodes[i].collision_bound;
            if cb < 0 {
                continue;
            }
            let id = *by_offset.entry(cb).or_insert_with(|| c.build_hull(cb as usize));
            c.nodes[i].hull = id;
        }
        c
    }

    /// Prepare one leaf hull (see `Hull`); -1 when its data is unreadable (such hulls never block).
    fn build_hull(&mut self, hull: usize) -> i32 {
        let Some(list) = self.leaf_hulls.get(hull..) else { return -1 };
        let Some(count) = list.iter().position(|&v| v < 0) else { return -1 };
        let Some(bb) = list.get(count + 1..count + 7) else { return -1 };
        let f = |i: usize| f32::from_bits(bb[i] as u32) as f64;
        let eps = 0.1;
        let (bmin, bmax) = (DVec3::new(f(0) + eps, f(1) + eps, f(2) + eps), DVec3::new(f(3) - eps, f(4) - eps, f(5) - eps));
        let mut planes = vec![
            DVec4::new(1.0, 0.0, 0.0, bmax.x),
            DVec4::new(-1.0, 0.0, 0.0, -bmin.x),
            DVec4::new(0.0, 1.0, 0.0, bmax.y),
            DVec4::new(0.0, -1.0, 0.0, -bmin.y),
            DVec4::new(0.0, 0.0, 1.0, bmax.z),
            DVec4::new(0.0, 0.0, -1.0, -bmin.z),
        ];
        let mut own = Vec::with_capacity(count);
        for &raw in &list[..count] {
            let flip = raw & 0x4000_0000 != 0;
            let idx = (raw & !0x4000_0000) as usize;
            let Some(n) = self.nodes.get(idx) else { return -1 };
            own.push(if flip { -n.plane } else { n.plane });
        }
        planes.extend_from_slice(&own);
        for i in 0..own.len() {
            for j in 0..i {
                let (a, b) = (own[i], own[j]);
                for (k, axis) in [DVec3::X, DVec3::Y, DVec3::Z].into_iter().enumerate() {
                    if (a[k] < 0.0 && b[k] > 0.0) || (a[k] > 0.0 && b[k] < 0.0) {
                        if let Some(p) = bevel_plane(a, b, axis) {
                            planes.push(p);
                        }
                    }
                }
            }
        }
        let first = self.planes.len() as u32;
        self.planes.extend_from_slice(&planes);
        self.hulls.push(Hull { bmin, bmax, first, count: planes.len() as u32 });
        self.hulls.len() as i32 - 1
    }

    /// The BSP zone a point is in (index into the model's zone list; 0 is "no zone").
    pub fn zone_at(&self, p: DVec3) -> i32 {
        if self.nodes.is_empty() {
            return 0;
        }
        let mut i = 0usize;
        loop {
            let n = &self.nodes[i];
            let d = n.plane.x * p.x + n.plane.y * p.y + n.plane.z * p.z - n.plane.w;
            let (child, side) = if d >= 0.0 { (n.front, 1) } else { (n.back, 0) };
            if child < 0 || child as usize >= self.nodes.len() {
                return n.zone[side];
            }
            i = child as usize;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Sweep a box with half-size `extent` from `start` to `end`. Returns the first blocking hit.
    pub fn trace(&self, start: DVec3, end: DVec3, extent: DVec3) -> Hit {
        if self.nodes.is_empty() {
            return Hit::NONE;
        }
        let delta = end - start;
        let len = delta.length();
        if len < 1e-6 {
            return Hit::NONE;
        }
        let mut best = Hit::NONE;
        // The swept box's bounds, grown by a unit: a hull wholly outside them can't produce a hit under 1.0.
        let mut lo = start.min(end) - extent - DVec3::ONE;
        let mut hi = start.max(end) + extent + DVec3::ONE;
        let pad = extent * 1.1;
        // Once something is hit, only the part of the sweep before it can hold a nearer hit: nodes and hulls
        // are tested against that shortened sweep (with a unit of slack), so the walk stops descending past it.
        let mut cut_end = end;
        STACK.with(|st| {
            // Iterative walk (no deep recursion on large trees), on a per-thread stack reused between traces.
            let mut stack = st.borrow_mut();
            stack.clear();
            stack.push(0);
            while let Some(ni) = stack.pop() {
                let Some(node) = self.nodes.get(ni as usize) else { continue };
                if node.hull >= 0 {
                    let h = &self.hulls[node.hull as usize];
                    if h.bmax.cmpge(lo).all() && h.bmin.cmple(hi).all() {
                        let mut c = Sweep::new(start, end, extent);
                        let planes = &self.planes[h.first as usize..(h.first + h.count) as usize];
                        if planes.iter().all(|&p| c.clip(p)) {
                            if let Some(h) = c.result() {
                                if h.fraction < best.fraction {
                                    best = h;
                                    let t = (best.fraction + 1.0 / len).min(1.0);
                                    cut_end = start + delta * t;
                                    lo = start.min(cut_end) - extent - DVec3::ONE;
                                    hi = start.max(cut_end) + extent + DVec3::ONE;
                                }
                            }
                        }
                    }
                }
                let s0 = side(node.plane, start, pad);
                let s1 = side(node.plane, cut_end, pad);
                if node.front >= 0 && (s0 <= 0 || s1 <= 0) {
                    stack.push(node.front);
                }
                if node.back >= 0 && (s0 >= 0 || s1 >= 0) {
                    stack.push(node.back);
                }
            }
        });
        best
    }

    /// The original, unoptimised sweep (hulls decoded and bevelled per call); kept to check `trace` against.
    #[doc(hidden)]
    pub fn trace_reference(&self, start: DVec3, end: DVec3, extent: DVec3) -> Hit {
        if self.nodes.is_empty() {
            return Hit::NONE;
        }
        let delta = end - start;
        if delta.length() < 1e-6 {
            return Hit::NONE;
        }
        let mut best = Hit::NONE;
        let mut stack = vec![0i32];
        let pad = extent * 1.1;
        while let Some(ni) = stack.pop() {
            let Some(node) = self.nodes.get(ni as usize) else { continue };
            if node.collision_bound >= 0 {
                if let Some(h) = self.sweep_hull(node.collision_bound as usize, start, end, extent) {
                    if h.fraction < best.fraction {
                        best = h;
                    }
                }
            }
            let s0 = side(node.plane, start, pad);
            let s1 = side(node.plane, end, pad);
            if node.front >= 0 && (s0 <= 0 || s1 <= 0) {
                stack.push(node.front);
            }
            if node.back >= 0 && (s0 >= 0 || s1 >= 0) {
                stack.push(node.back);
            }
        }
        best
    }

    /// True if a box at `pos` overlaps solid geometry.
    pub fn encroaches(&self, pos: DVec3, extent: DVec3) -> bool {
        // A tiny upward sweep: starting inside a hull reports a hit at fraction 0.
        let h = self.trace(pos, pos + DVec3::new(0.0, 0.0, 0.01), extent);
        h.fraction <= 0.0
    }

    fn sweep_hull(&self, hull: usize, start: DVec3, end: DVec3, extent: DVec3) -> Option<Hit> {
        let list = self.leaf_hulls.get(hull..)?;
        let count = list.iter().position(|&v| v < 0)?;
        let bb = list.get(count + 1..count + 7)?;
        let f = |i: usize| f32::from_bits(bb[i] as u32) as f64;
        let eps = 0.1;
        let (bmin, bmax) = (DVec3::new(f(0) + eps, f(1) + eps, f(2) + eps), DVec3::new(f(3) - eps, f(4) - eps, f(5) - eps));

        let mut c = Sweep::new(start, end, extent);
        let box_planes = [
            DVec4::new(1.0, 0.0, 0.0, bmax.x),
            DVec4::new(-1.0, 0.0, 0.0, -bmin.x),
            DVec4::new(0.0, 1.0, 0.0, bmax.y),
            DVec4::new(0.0, -1.0, 0.0, -bmin.y),
            DVec4::new(0.0, 0.0, 1.0, bmax.z),
            DVec4::new(0.0, 0.0, -1.0, -bmin.z),
        ];
        for p in box_planes {
            if !c.clip(p) {
                return None;
            }
        }
        let mut planes: Vec<DVec4> = Vec::with_capacity(count);
        for &raw in &list[..count] {
            let flip = raw & 0x4000_0000 != 0;
            let idx = (raw & !0x4000_0000) as usize;
            let p = self.nodes.get(idx)?.plane;
            planes.push(if flip { -p } else { p });
        }
        for &p in &planes {
            if !c.clip(p) {
                return None;
            }
        }
        for i in 0..planes.len() {
            for j in 0..i {
                let (a, b) = (planes[i], planes[j]);
                for (k, axis) in [DVec3::X, DVec3::Y, DVec3::Z].into_iter().enumerate() {
                    if (a[k] < 0.0 && b[k] > 0.0) || (a[k] > 0.0 && b[k] < 0.0) {
                        if !c.bevel(a, b, axis) {
                            return None;
                        }
                    }
                }
            }
        }
        c.result()
    }
}

/// -1 = box entirely in front of the plane, 1 = entirely behind, 0 = straddles.
fn side(plane: DVec4, center: DVec3, extent: DVec3) -> i32 {
    let e = extent.x * plane.x.abs() + extent.y * plane.y.abs() + extent.z * plane.z.abs();
    let s = plane.truncate().dot(center) - plane.w;
    if s - e > 0.0 {
        -1
    } else if s + e < 0.0 {
        1
    } else {
        0
    }
}

/// The bevel plane along `axis` at the edge where two hull planes meet (None when they don't form one).
fn bevel_plane(p0: DVec4, p1: DVec4, axis: DVec3) -> Option<DVec4> {
    let (n0, n1) = (p0.truncate(), p1.truncate());
    if axis.cross(n0).dot(axis.cross(n1)) <= 1e-5 {
        return None;
    }
    let line = n0.cross(n1);
    let l2 = line.dot(line);
    if l2 < 1e-6 {
        return None;
    }
    let point = (p0.w * n1.cross(line) + p1.w * line.cross(n0)) / l2;
    let mut normal = axis.cross(line.normalize()).normalize();
    if n0.dot(normal) < 0.0 {
        normal = -normal;
    }
    Some(normal.extend(point.dot(normal)))
}

struct Sweep {
    start: DVec3,
    end: DVec3,
    extent: DVec3,
    t_start: f64,
    t_end: f64,
    normal: DVec3,
    miss: bool,
}

impl Sweep {
    fn new(start: DVec3, end: DVec3, extent: DVec3) -> Sweep {
        Sweep { start, end, extent, t_start: -1.0, t_end: 2.0, normal: DVec3::ZERO, miss: false }
    }

    fn clip(&mut self, plane: DVec4) -> bool {
        let push = (plane.x * self.extent.x).abs() + (plane.y * self.extent.y).abs() + (plane.z * self.extent.z).abs();
        let n = plane.truncate();
        let d0 = n.dot(self.start) - plane.w;
        let d1 = n.dot(self.end) - plane.w;
        let facing = d0 - d1;
        let mut dist = d0 - push;
        if d0 > d1 && dist >= -push && dist < 0.0 {
            dist = 0.0;
        }
        let t = dist / (d0 - d1);
        if facing > 1e-5 {
            if t > self.t_start {
                self.t_start = t;
                self.normal = n;
            }
        } else if facing < -1e-5 {
            self.t_end = self.t_end.min(t);
        } else if d0 > push && d1 > push {
            self.miss = true;
            return false;
        }
        if self.t_start < self.t_end {
            true
        } else {
            self.miss = true;
            false
        }
    }

    fn bevel(&mut self, p0: DVec4, p1: DVec4, axis: DVec3) -> bool {
        match bevel_plane(p0, p1, axis) {
            Some(p) => self.clip(p),
            None => true,
        }
    }

    fn result(&self) -> Option<Hit> {
        if !self.miss && self.t_start > -1.0 && self.t_start < self.t_end && self.t_end > 0.0 {
            let len = (self.end - self.start).length().max(1e-9);
            // Stop 0.1 units short of the surface, like the original.
            let frac = ((self.t_start * len - 0.1) / len).max(0.0);
            Some(Hit { fraction: frac.min(1.0), normal: self.normal })
        } else {
            None
        }
    }
}

/// A blocking actor's collision cylinder (Unreal units; `center` is the actor location).
#[derive(Debug, Clone, Copy)]
pub struct Cylinder {
    pub center: DVec3,
    pub radius: f64,
    pub height: f64,
    /// Index into `Level::actors`.
    pub actor: usize,
}

/// Sweep a cylinder (radius `extent.x`, half-height `extent.z`) against another cylinder, as Unreal does
/// for actor-actor collision: the moving cylinder is shrunk to a point and the target grown by its size.
pub fn trace_cylinder(c: &Cylinder, start: DVec3, end: DVec3, extent: DVec3) -> Option<Hit> {
    let r = c.radius + extent.x;
    let h = c.height + extent.z;
    let d = end - start;
    let len = d.length();
    if len < 1e-9 {
        return None;
    }
    let s = start - c.center;
    let mut best: Option<(f64, DVec3)> = None;
    // Side wall: solve |s.xy + t d.xy| = r.
    let a = d.x * d.x + d.y * d.y;
    if a > 1e-12 {
        let b = 2.0 * (s.x * d.x + s.y * d.y);
        let cc = s.x * s.x + s.y * s.y - r * r;
        let disc = b * b - 4.0 * a * cc;
        if disc >= 0.0 {
            let t = (-b - disc.sqrt()) / (2.0 * a);
            if (0.0..=1.0).contains(&t) {
                let z = s.z + d.z * t;
                if z.abs() <= h {
                    let p = s + d * t;
                    best = Some((t, DVec3::new(p.x, p.y, 0.0).normalize_or_zero()));
                }
            }
        }
    }
    // Caps.
    if d.z.abs() > 1e-12 {
        for (plane, n) in [(h, 1.0), (-h, -1.0)] {
            // Only hit a cap from outside, moving toward it.
            if (s.z - plane) * n < 0.0 || d.z * n >= 0.0 {
                continue;
            }
            let t = (plane - s.z) / d.z;
            if (0.0..=1.0).contains(&t) {
                let p = s + d * t;
                if p.x * p.x + p.y * p.y <= r * r && best.map_or(true, |(bt, _)| t < bt) {
                    best = Some((t, DVec3::new(0.0, 0.0, n)));
                }
            }
        }
    }
    // Already overlapping and moving inward: block immediately.
    if best.is_none() && s.x * s.x + s.y * s.y < r * r && s.z.abs() < h {
        let out = DVec3::new(s.x, s.y, 0.0);
        if out.dot(d) < 0.0 {
            return Some(Hit { fraction: 0.0, normal: out.normalize_or_zero() });
        }
        return None;
    }
    best.map(|(t, n)| Hit { fraction: ((t * len - 0.1) / len).max(0.0), normal: n })
}
