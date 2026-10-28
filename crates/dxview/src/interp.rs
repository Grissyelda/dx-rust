//! InterpolationPoint paths (PHYS_Interpolating) and PHYS_Rotating decorations.
//!
//! Points sharing a Tag link by Position (InterpolationPoint.BeginPlay: Prev = Position - 1, Next = Position + 1,
//! else the point at Position 0). An interpolating actor moves from Target to Target.Next as PhysAlpha goes 0..1
//! at PhysRate x RateModifier (both lerped between the two points); location and rotation follow the 4-point
//! blend SurrealEngine uses (weights w(t) = t^4/16 - t^2/2 + 1 at t+1, t, t-1, t-2, normalised; straight lerp at
//! the ends of a path). Reaching a point calls InterpolateEnd; bSkipNextPath points are passed over; a rate of
//! 0 holds. FovModifier / GameSpeedModifier lerp the same way.
//!
//! Used by the DX.dx logo flyby behind the main menu (IntroCam) and later by cutscene cameras.

use crate::actors::Deco;
use bevy::prelude::*;
use glam::DVec3;
use unreal_pkg::props::{find, Property, Value};

#[derive(Clone, Debug)]
pub struct Point {
    pub actor: usize,
    pub position: i32,
    pub loc: DVec3,
    /// Pitch, yaw, roll (Unreal units, not wrapped: the original blends the raw numbers).
    pub rot: [f64; 3],
    pub rate: f64,
    pub fov: f64,
    pub game_speed: f64,
    pub end_of_path: bool,
    pub skip_next: bool,
    pub prev: Option<usize>,
    pub next: Option<usize>,
}

fn f(p: &[Property], n: &str, d: f64) -> f64 {
    match find(p, n) {
        Some(Value::Float(v)) => *v as f64,
        Some(Value::Int(v)) => *v as f64,
        _ => d,
    }
}

/// Every InterpolationPoint with this tag, linked.
pub fn path(lvl: &dx_level::Level, tag: &str) -> Vec<Point> {
    let mut pts: Vec<Point> = lvl
        .actors
        .iter()
        .enumerate()
        .filter(|(_, a)| a.class == "InterpolationPoint" && crate::devices::name_prop(&a.props, "Tag").eq_ignore_ascii_case(tag))
        .map(|(i, a)| {
            let p = &a.props;
            let l = a.location();
            let r = match find(p, "Rotation") {
                Some(Value::Rotator(r)) => [r[0] as f64, r[1] as f64, r[2] as f64],
                _ => [0.0; 3],
            };
            Point {
                actor: i,
                position: f(p, "Position", 0.0) as i32,
                loc: DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64),
                rot: r,
                rate: f(p, "RateModifier", 1.0),
                fov: f(p, "FovModifier", 1.0),
                game_speed: f(p, "GameSpeedModifier", 1.0),
                end_of_path: matches!(find(p, "bEndOfPath"), Some(Value::Bool(true))),
                skip_next: matches!(find(p, "bSkipNextPath"), Some(Value::Bool(true))),
                prev: None,
                next: None,
            }
        })
        .collect();
    let n = pts.len();
    for i in 0..n {
        let pos = pts[i].position;
        let prev = (0..n).find(|&j| pts[j].position == pos - 1);
        let next = (0..n).find(|&j| pts[j].position == pos + 1).or_else(|| (0..n).find(|&j| pts[j].position == 0));
        pts[i].prev = prev;
        pts[i].next = next;
    }
    pts
}

/// The index of the point at Position `p`.
pub fn at_position(pts: &[Point], p: i32) -> Option<usize> {
    pts.iter().position(|x| x.position == p)
}

/// An actor in PHYS_Interpolating.
#[derive(Clone, Debug)]
pub struct Interp {
    pub target: Option<usize>,
    pub alpha: f64,
    pub rate: f64,
    pub loc: DVec3,
    pub rot: [f64; 3],
    pub fov_mod: f64,
}

fn w(t: f64) -> f64 {
    let s = t * t;
    s * s / 16.0 - s * 0.5 + 1.0
}

fn spline<T>(a: T, b: T, c: T, d: T, t: f64, mul: impl Fn(T, f64) -> T, add: impl Fn(T, T) -> T) -> T
where
    T: Copy,
{
    let (w0, w1, w2, w3) = (w(t + 1.0), w(t), w(t - 1.0), w(t - 2.0));
    let inv = 1.0 / (w0 + w1 + w2 + w3);
    let s = add(add(mul(a, w0), mul(b, w1)), add(mul(c, w2), mul(d, w3)));
    mul(s, inv)
}

fn rot_mul(r: [f64; 3], k: f64) -> [f64; 3] {
    [r[0] * k, r[1] * k, r[2] * k]
}
fn rot_add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

impl Interp {
    pub fn start(pts: &[Point], first: usize) -> Interp {
        Interp { target: Some(first), alpha: 0.0, rate: 1.0, loc: pts[first].loc, rot: pts[first].rot, fov_mod: pts[first].fov }
    }

    /// TickInterpolating: advance, returning the points whose InterpolateEnd fired.
    pub fn tick(&mut self, pts: &[Point], dt: f64) -> Vec<usize> {
        let mut ended = Vec::new();
        let mut left = dt;
        let mut guard = 0;
        while left > 0.0 && guard < 64 {
            guard += 1;
            let Some(ti) = self.target else { break };
            let t = &pts[ti];
            let Some(ni) = t.next else { break };
            let nx = &pts[ni];
            let a = self.alpha;
            self.fov_mod = t.fov + (nx.fov - t.fov) * a;
            let rate = self.rate * (t.rate + (nx.rate - t.rate) * a);
            if rate == 0.0 || self.rate == 0.0 {
                break;
            }
            let mut alpha = a + rate * left;
            let (mut start, mut end) = (false, false);
            if rate < 0.0 && alpha < 0.0 {
                left = alpha / rate;
                alpha = 0.0;
                start = true;
            } else if rate > 0.0 && alpha > 1.0 {
                left = (alpha - 1.0) / rate;
                alpha = 1.0;
                end = true;
            } else {
                left = 0.0;
            }
            match (t.prev, nx.next) {
                (Some(pi), Some(nni)) => {
                    let (p, nn) = (&pts[pi], &pts[nni]);
                    self.loc = spline(p.loc, t.loc, nx.loc, nn.loc, alpha, |v, k| v * k, |a, b| a + b);
                    self.rot = spline(p.rot, t.rot, nx.rot, nn.rot, alpha, rot_mul, rot_add);
                }
                _ => {
                    self.loc = t.loc + (nx.loc - t.loc) * alpha;
                    self.rot = rot_add(t.rot, rot_mul(rot_add(nx.rot, rot_mul(t.rot, -1.0)), alpha));
                }
            }
            self.alpha = alpha;
            if start {
                ended.push(ti);
                let mut nt = t.prev;
                while let Some(k) = nt {
                    if !pts[k].skip_next {
                        break;
                    }
                    nt = pts[k].prev;
                }
                self.target = nt;
                self.alpha = 1.0;
            } else if end {
                ended.push(ti);
                let mut nt = t.next;
                while let Some(k) = nt {
                    if !pts[k].skip_next {
                        break;
                    }
                    nt = pts[k].next;
                }
                self.target = nt;
                self.alpha = 0.0;
            }
        }
        ended
    }
}

/// PHYS_Rotating decorations with bFixedRotationDir spin at their RotationRate (the DX logo).
#[derive(Component)]
pub struct Rotating {
    pub rate: [f64; 3],
    pub rot: [f64; 3],
}

pub fn rotating_system(
    time: Res<Time>,
    level: Res<crate::LevelRes>,
    mut commands: Commands,
    mut map_gen: Local<u64>,
    mut done: Local<bool>,
    mut decos: Query<(Entity, &mut Deco, Option<&mut Rotating>)>,
) {
    // A new map (loader.rs): start this system's per-map state over.
    if crate::loader::new_map(&mut map_gen) {
        *done = false;
    }
    let dt = time.delta_secs_f64();
    if !*done {
        *done = true;
        for (e, d, _) in &decos {
            let Some(m) = level.0.mesh_actors.get(d.mesh_actor) else { continue };
            let Some(a) = level.0.actors.get(m.actor) else { continue };
            let p = &a.props;
            let phys = match find(p, "Physics") {
                Some(Value::Byte(b)) => *b,
                _ => 0,
            };
            if phys == 5 && matches!(find(p, "bFixedRotationDir"), Some(Value::Bool(true))) {
                if let Some(Value::Rotator(r)) = find(p, "RotationRate") {
                    commands.entity(e).insert(Rotating {
                        rate: [r[0] as f64, r[1] as f64, r[2] as f64],
                        rot: [d.rotation[0] as f64, d.rotation[1] as f64, d.rotation[2] as f64],
                    });
                }
            }
        }
        return;
    }
    for (_, mut d, r) in &mut decos {
        let Some(mut r) = r else { continue };
        for k in 0..3 {
            r.rot[k] = (r.rot[k] + r.rate[k] * dt) % 65536.0;
        }
        d.rotation = [r.rot[0] as i32, r.rot[1] as i32, r.rot[2] as i32];
        d.spun = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(position: i32, x: f64, rate: f64) -> Point {
        Point {
            actor: 0,
            position,
            loc: DVec3::new(x, 0.0, 0.0),
            rot: [0.0; 3],
            rate,
            fov: 1.0,
            game_speed: 1.0,
            end_of_path: false,
            skip_next: false,
            prev: None,
            next: None,
        }
    }

    #[test]
    fn moves_along_and_holds_at_zero_rate() {
        let mut pts = vec![pt(0, 0.0, 1.0), pt(1, 100.0, 1.0), pt(2, 200.0, 0.0)];
        for i in 0..3 {
            pts[i].prev = if i > 0 { Some(i - 1) } else { None };
            pts[i].next = if i < 2 { Some(i + 1) } else { Some(0) };
        }
        let mut it = Interp::start(&pts, 0);
        let ended = it.tick(&pts, 0.5);
        assert!(ended.is_empty());
        assert!(it.loc.x > 0.0 && it.loc.x < 100.0);
        let ended = it.tick(&pts, 0.75);
        assert_eq!(ended, vec![0]);
        assert_eq!(it.target, Some(1));
        // From point 1 the rate falls to 0 towards point 2: it slows and never quite arrives.
        for _ in 0..200 {
            it.tick(&pts, 0.05);
        }
        // Unreal's 4-point spline doesn't pass through the points: with the path looping back to x = 0 after
        // point 2, it settles near x = 120.
        assert!(it.loc.x > 100.0 && it.loc.x <= 200.0);
    }
}
