//! The AI path network: the level's NavigationPoints (PathNode, PatrolPoint, ...) and the editor-built
//! ReachSpecs between them, with A* search (Unreal's FindPathToward) and patrol chains.

use unreal_pkg::classes::ClassDb;
use unreal_pkg::props::{find, Value};
use unreal_pkg::Package;
use std::collections::{BinaryHeap, HashMap};

/// ReachSpec flags (UnPath.h).
pub const R_WALK: i32 = 1;
pub const R_FLY: i32 = 2;
pub const R_SWIM: i32 = 4;
pub const R_JUMP: i32 = 8;
pub const R_DOOR: i32 = 16;
pub const R_PLAYERONLY: i32 = 64;

#[derive(Debug, Clone)]
pub struct NavPoint {
    pub name: String,
    pub class: String,
    pub tag: String,
    pub pos: [f64; 3],
    /// Unreal yaw (65536 = full turn).
    pub yaw: i32,
    /// PatrolPoint: how long to wait here, and the next point in the chain.
    pub pause_time: f32,
    pub next_patrol: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
pub struct NavEdge {
    pub to: usize,
    pub dist: f32,
    pub radius: f32,
    pub height: f32,
    pub flags: i32,
}

#[derive(Debug, Clone, Default)]
pub struct NavGraph {
    pub points: Vec<NavPoint>,
    pub edges: Vec<Vec<NavEdge>>,
}

#[derive(PartialEq)]
struct Open(f64, usize);
impl Eq for Open {}
impl PartialOrd for Open {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Open {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        o.0.partial_cmp(&self.0).unwrap_or(std::cmp::Ordering::Equal)
    }
}

impl NavGraph {
    pub fn build(pkg: &Package, classes: &mut ClassDb) -> NavGraph {
        let specs = unreal_pkg::objects::read_reach_specs(pkg).unwrap_or_default();
        let mut exports: Vec<i32> = Vec::new();
        for s in &specs {
            exports.push(s.start);
            exports.push(s.end);
        }
        for (i, e) in pkg.exports.iter().enumerate() {
            if pkg.class_name(e.class_index) == "PatrolPoint" && e.size > 0 {
                exports.push(i as i32 + 1);
            }
        }
        exports.retain(|&x| x > 0);
        exports.sort_unstable();
        exports.dedup();

        let mut g = NavGraph::default();
        let mut index: HashMap<i32, usize> = HashMap::new();
        let mut next_names: Vec<String> = Vec::new();
        for &x in &exports {
            let e = &pkg.exports[(x - 1) as usize];
            let Some(p) = classes.actor_properties(pkg, e) else { continue };
            let Some(Value::Vector(l)) = find(&p, "Location") else { continue };
            let name_of = |n: &str| match find(&p, n) {
                Some(Value::Name(s)) if s != "None" => s.clone(),
                _ => String::new(),
            };
            index.insert(x, g.points.len());
            g.points.push(NavPoint {
                name: e.name.clone(),
                class: pkg.class_name(e.class_index).to_string(),
                tag: name_of("Tag"),
                pos: [l[0] as f64, l[1] as f64, l[2] as f64],
                yaw: match find(&p, "Rotation") {
                    Some(Value::Rotator(r)) => r[1],
                    _ => 0,
                },
                pause_time: match find(&p, "pausetime") {
                    Some(Value::Float(v)) => *v,
                    _ => 0.0,
                },
                next_patrol: None,
            });
            next_names.push(name_of("Nextpatrol"));
            g.edges.push(Vec::new());
        }
        // PatrolPoint.PreBeginPlay: the next point is the first other PatrolPoint whose Tag matches Nextpatrol.
        for i in 0..g.points.len() {
            let want = &next_names[i];
            if want.is_empty() || g.points[i].class != "PatrolPoint" {
                continue;
            }
            g.points[i].next_patrol = (0..g.points.len()).find(|&j| j != i && g.points[j].class == "PatrolPoint" && g.points[j].tag.eq_ignore_ascii_case(want));
        }
        for s in &specs {
            let (Some(&a), Some(&b)) = (index.get(&s.start), index.get(&s.end)) else { continue };
            g.edges[a].push(NavEdge { to: b, dist: s.distance as f32, radius: s.collision_radius as f32, height: s.collision_height as f32, flags: s.reach_flags });
        }
        g
    }

    fn usable(e: &NavEdge, radius: f64, height: f64) -> bool {
        e.flags & (R_PLAYERONLY | R_FLY) == 0 && e.radius as f64 >= radius && e.height as f64 >= height
    }

    /// Shortest path of point indices from `from` to `to` (both included) for a pawn of the given size.
    pub fn find_path(&self, from: usize, to: usize, radius: f64, height: f64) -> Option<Vec<usize>> {
        if from == to {
            return Some(vec![to]);
        }
        let n = self.points.len();
        let mut cost = vec![f64::INFINITY; n];
        let mut prev = vec![usize::MAX; n];
        let goal = self.points[to].pos;
        let h = |i: usize| {
            let p = self.points[i].pos;
            ((p[0] - goal[0]).powi(2) + (p[1] - goal[1]).powi(2) + (p[2] - goal[2]).powi(2)).sqrt()
        };
        let mut open = BinaryHeap::new();
        cost[from] = 0.0;
        open.push(Open(h(from), from));
        while let Some(Open(_, i)) = open.pop() {
            if i == to {
                let mut path = vec![to];
                let mut c = to;
                while prev[c] != usize::MAX {
                    c = prev[c];
                    path.push(c);
                }
                path.reverse();
                return Some(path);
            }
            for e in &self.edges[i] {
                if !Self::usable(e, radius, height) {
                    continue;
                }
                let c = cost[i] + e.dist.max(1.0) as f64;
                if c < cost[e.to] {
                    cost[e.to] = c;
                    prev[e.to] = i;
                    open.push(Open(c + h(e.to), e.to));
                }
            }
        }
        None
    }

    /// Point indices sorted by distance from `pos`, nearest first, within `max_dist`.
    pub fn nearby(&self, pos: [f64; 3], max_dist: f64) -> Vec<usize> {
        let mut v: Vec<(f64, usize)> = self
            .points
            .iter()
            .enumerate()
            .map(|(i, p)| ((p.pos[0] - pos[0]).powi(2) + (p.pos[1] - pos[1]).powi(2) + ((p.pos[2] - pos[2]) * 2.0).powi(2), i))
            .filter(|(d, _)| *d <= max_dist * max_dist)
            .collect();
        v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        v.into_iter().map(|x| x.1).collect()
    }

    /// The patrol chain a pawn with `order_tag` follows: the closest point of the chain (PickStartPoint).
    pub fn patrol_start(&self, order_tag: &str, pos: [f64; 3]) -> Option<usize> {
        let mut best: Option<(f64, usize)> = None;
        let mut seen = vec![false; self.points.len()];
        for (i, p) in self.points.iter().enumerate() {
            if p.class != "PatrolPoint" || !p.tag.eq_ignore_ascii_case(order_tag) {
                continue;
            }
            let mut c = Some(i);
            while let Some(k) = c {
                if seen[k] {
                    break;
                }
                seen[k] = true;
                let q = self.points[k].pos;
                let d = ((q[0] - pos[0]).powi(2) + (q[1] - pos[1]).powi(2) + (q[2] - pos[2]).powi(2)).sqrt();
                if best.map_or(true, |(bd, _)| d < bd) {
                    best = Some((d, k));
                }
                c = self.points[k].next_patrol;
            }
        }
        best.map(|b| b.1)
    }
}
