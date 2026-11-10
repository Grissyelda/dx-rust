//! T-junctions in the level's BSP faces. BSP splits leave a vertex of one face in the middle of a neighbour's edge
//! (a riser's corner on a tread's long back edge, say). Unreal's span renderer never shows the seam, but a GPU
//! rasterises the neighbour's edge as one straight line that misses the extra vertex by a fraction of a pixel:
//! dashed cracks along such edges show whatever lies behind them - the outside of the level, the sky. Mirror
//! reflections look at stairs from below, along exactly these edges, and drew a dotted line of sky across every
//! step (#45). Each face's outline gets those vertices inserted, and a face that gained any is drawn as a fan around
//! its centre (a fan from a corner would still run a single long edge past them).

use std::collections::HashMap;
use unreal_pkg::objects::Model;

/// Grid cell size (Unreal units).
const CELL: f32 = 128.0;
/// How close to an edge a vertex has to be to lie on it (Unreal units; BSP points are stored as floats).
const TOL: f32 = 0.08;

pub struct TJunctions<'a> {
    points: &'a [[f32; 3]],
    grid: HashMap<[i32; 3], Vec<u32>>,
}

fn cell(p: [f32; 3]) -> [i32; 3] {
    [(p[0] / CELL).floor() as i32, (p[1] / CELL).floor() as i32, (p[2] / CELL).floor() as i32]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

impl<'a> TJunctions<'a> {
    /// Indexes every point used by a face's outline.
    pub fn new(model: &'a Model) -> Self {
        let mut grid: HashMap<[i32; 3], Vec<u32>> = HashMap::new();
        let mut seen = vec![false; model.points.len()];
        for node in &model.nodes {
            if node.num_vertices < 3 {
                continue;
            }
            for k in 0..node.num_vertices as usize {
                let Some(v) = model.vertices.get(node.vert_pool as usize + k) else { break };
                let i = v.vertex as usize;
                let Some(p) = model.points.get(i) else { break };
                if !seen[i] {
                    seen[i] = true;
                    grid.entry(cell(*p)).or_default().push(i as u32);
                }
            }
        }
        TJunctions { points: &model.points, grid }
    }

    /// The face outline (point indices) with every other face's vertex that lies on one of its edges inserted in
    /// order, or None when there's none.
    pub fn split(&self, outline: &[u32]) -> Option<Vec<u32>> {
        let n = outline.len();
        let mut out: Vec<u32> = Vec::with_capacity(n + 4);
        let mut any = false;
        let mut cells: Vec<[i32; 3]> = Vec::new();
        let mut found: Vec<(f32, u32)> = Vec::new();
        for k in 0..n {
            let (ia, ib) = (outline[k], outline[(k + 1) % n]);
            out.push(ia);
            let (Some(&a), Some(&b)) = (self.points.get(ia as usize), self.points.get(ib as usize)) else { continue };
            let d = sub(b, a);
            let len2 = dot(d, d);
            if len2 < 1.0 {
                continue;
            }
            let len = len2.sqrt();
            // The cells along the edge and their neighbours.
            cells.clear();
            let steps = (len / CELL).ceil() as i32 + 1;
            for s in 0..=steps {
                let t = (s as f32 / steps as f32).min(1.0);
                let c = cell([a[0] + d[0] * t, a[1] + d[1] * t, a[2] + d[2] * t]);
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        for dz in -1..=1 {
                            let q = [c[0] + dx, c[1] + dy, c[2] + dz];
                            if !cells.contains(&q) {
                                cells.push(q);
                            }
                        }
                    }
                }
            }
            found.clear();
            for c in &cells {
                let Some(list) = self.grid.get(c) else { continue };
                for &pi in list {
                    if pi == ia || pi == ib {
                        continue;
                    }
                    let p = self.points[pi as usize];
                    let ap = sub(p, a);
                    let t = dot(ap, d) / len2;
                    // Not at (or past) either end.
                    if t * len <= TOL || (1.0 - t) * len <= TOL {
                        continue;
                    }
                    let off = [ap[0] - d[0] * t, ap[1] - d[1] * t, ap[2] - d[2] * t];
                    if dot(off, off) > TOL * TOL {
                        continue;
                    }
                    // One vertex per spot (several points can share a position).
                    if found.iter().any(|(u, _)| ((u - t) * len).abs() <= TOL) {
                        continue;
                    }
                    found.push((t, pi));
                }
            }
            if !found.is_empty() {
                any = true;
                found.sort_by(|x, y| x.0.total_cmp(&y.0));
                out.extend(found.iter().map(|f| f.1));
            }
        }
        any.then_some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_vertex_on_edge() {
        let mut m = Model::default();
        // A square 0..100 and a neighbour whose corner sits at (50, 100, 0) on the square's top edge.
        m.points = vec![[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [100.0, 100.0, 0.0], [0.0, 100.0, 0.0], [50.0, 100.0, 0.0], [50.0, 200.0, 0.0]];
        let node = |pool: i32, nv: u8| unreal_pkg::objects::BspNode {
            plane: [0.0, 0.0, 1.0, 0.0],
            zone_mask: 0,
            flags: 0,
            vert_pool: pool,
            surf: 0,
            back: -1,
            front: -1,
            plane_idx: -1,
            zone: [0, 0],
            num_vertices: nv,
            collision_bound: -1,
        };
        m.vertices = [0, 1, 2, 3, 4, 5, 3].into_iter().map(|i| unreal_pkg::objects::BspVert { vertex: i, side: 0 }).collect();
        m.nodes = vec![node(0, 4), node(4, 3)];
        let tj = TJunctions::new(&m);
        assert_eq!(tj.split(&[0, 1, 2, 3]), Some(vec![0, 1, 2, 4, 3]));
        assert_eq!(tj.split(&[4, 5, 3]), None);
    }
}
