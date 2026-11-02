//! A grass field over ground triangles. The ground is cut into square tiles (`TILE` metres); each tile keeps the
//! plane its ground lies in, a 4 x 4 mask of where inside it the grass faces really are, the light-map coordinate
//! at its middle and how that changes across it, and which kind of grass it holds. Each frame the tiles nearest the
//! eye are packed into texels for the shader, which grows a fixed number of blade slots per tile from a hash of
//! the tile's place in the world (so a blade stands in the same spot every frame) and thins them with distance.

use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

/// A tile's side (metres).
pub const TILE: f32 = 0.5;
/// Texels per tile in the packed data.
pub const TEXELS: u32 = 3;

/// A triangle of ground that grows grass: corners (Y-up), their light-map coordinates, and the kind of grass.
#[derive(Clone, Copy, Debug)]
pub struct GroundTri {
    pub p: [Vec3; 3],
    pub lm: [Vec2; 3],
    pub kind: u8,
}

/// One tile of the field.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Tile {
    /// The tile's corner (x, z) and the ground's height there.
    pub x: f32,
    pub z: f32,
    pub y: f32,
    /// The ground's slope: height per metre along x and along z.
    pub dydx: f32,
    pub dydz: f32,
    /// Where grass is inside the tile: bit (row * 4 + column), rows along z.
    pub mask: u16,
    pub kind: u8,
    /// Light map at the tile's middle, and per metre along x / along z.
    pub lm: [f32; 2],
    pub lm_dx: [f32; 2],
    pub lm_dz: [f32; 2],
}

/// The shader's per-frame values (std140-friendly: four vec4s).
#[derive(Clone, Copy, Debug, Default)]
pub struct GrassParams {
    /// x: tiles packed, y: blade slots per tile, z: the wind's time (seconds), w: grass level (1 low, 2 high).
    pub a: [f32; 4],
    /// Wind direction (x, z), strength, gusts.
    pub wind: [f32; 4],
    /// The eye (xyz), the reach of full density (metres).
    pub eye: [f32; 4],
    /// Light gain (x), how many pushes (y), blade height scale (z), outdoors 0..1 (w).
    pub b: [f32; 4],
}

/// The field: every tile, binned in square chunks for finding the near ones.
#[derive(Default)]
pub struct GrassField {
    pub tiles: Vec<Tile>,
    chunk: f32,
    bins: std::collections::HashMap<(i32, i32), Vec<u32>>,
}

fn bary(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> Option<[f32; 3]> {
    let v0 = b - a;
    let v1 = c - a;
    let v2 = p - a;
    let d = v0.x * v1.y - v1.x * v0.y;
    if d.abs() < 1e-9 {
        return None;
    }
    let v = (v2.x * v1.y - v1.x * v2.y) / d;
    let w = (v0.x * v2.y - v2.x * v0.y) / d;
    let u = 1.0 - v - w;
    (u >= -1e-4 && v >= -1e-4 && w >= -1e-4).then_some([u, v, w])
}

impl GrassField {
    /// Lays the tiles over the triangles (only those facing up at least `min_up`, the cosine of their slope).
    pub fn build(tris: &[GroundTri], min_up: f32) -> GrassField {
        use std::collections::HashMap;
        // Key: tile column, row, and a height band (two floors over one another stay apart).
        let mut map: HashMap<(i32, i32, i32), (Tile, u32)> = HashMap::new();
        for t in tris {
            let n = (t.p[1] - t.p[0]).cross(t.p[2] - t.p[0]).normalize_or_zero();
            let n = if n.y < 0.0 { -n } else { n };
            if n.y < min_up {
                continue;
            }
            let xz = |p: Vec3| Vec2::new(p.x, p.z);
            let (a, b, c) = (xz(t.p[0]), xz(t.p[1]), xz(t.p[2]));
            let lo = a.min(b).min(c);
            let hi = a.max(b).max(c);
            // The plane y = y0 + gx x + gz z, and the light map's own gradients.
            let solve = |va: f32, vb: f32, vc: f32| -> (f32, f32) {
                let (e1, e2) = (b - a, c - a);
                let (d1, d2) = (vb - va, vc - va);
                let det = e1.x * e2.y - e2.x * e1.y;
                if det.abs() < 1e-9 {
                    return (0.0, 0.0);
                }
                ((d1 * e2.y - d2 * e1.y) / det, (e1.x * d2 - e2.x * d1) / det)
            };
            let (gx, gz) = solve(t.p[0].y, t.p[1].y, t.p[2].y);
            let (lux, luz) = solve(t.lm[0].x, t.lm[1].x, t.lm[2].x);
            let (lvx, lvz) = solve(t.lm[0].y, t.lm[1].y, t.lm[2].y);
            let (i0, i1) = ((lo.x / TILE).floor() as i32, (hi.x / TILE).floor() as i32);
            let (j0, j1) = ((lo.y / TILE).floor() as i32, (hi.y / TILE).floor() as i32);
            for i in i0..=i1 {
                for j in j0..=j1 {
                    let corner = Vec2::new(i as f32 * TILE, j as f32 * TILE);
                    let mut mask = 0u16;
                    for r in 0..4 {
                        for c4 in 0..4 {
                            let p = corner + Vec2::new((c4 as f32 + 0.5) * TILE / 4.0, (r as f32 + 0.5) * TILE / 4.0);
                            if bary(p, a, b, c).is_some() {
                                mask |= 1 << (r * 4 + c4);
                            }
                        }
                    }
                    if mask == 0 {
                        continue;
                    }
                    let mid = corner + Vec2::splat(TILE * 0.5);
                    let y_at = |p: Vec2| t.p[0].y + gx * (p.x - a.x) + gz * (p.y - a.y);
                    let y_mid = y_at(mid);
                    let lm_mid = [
                        t.lm[0].x + lux * (mid.x - a.x) + luz * (mid.y - a.y),
                        t.lm[0].y + lvx * (mid.x - a.x) + lvz * (mid.y - a.y),
                    ];
                    let key = (i, j, (y_mid / 2.0).floor() as i32);
                    let tile = Tile {
                        x: corner.x,
                        z: corner.y,
                        y: y_at(corner),
                        dydx: gx,
                        dydz: gz,
                        mask,
                        kind: t.kind,
                        lm: lm_mid,
                        lm_dx: [lux, lvx],
                        lm_dz: [luz, lvz],
                    };
                    let count = mask.count_ones();
                    let e = map.entry(key).or_insert((tile, 0));
                    // Two triangles share the tile: the masks join, the plane is the one covering more of it.
                    let joined = e.0.mask | mask;
                    if count > e.1 {
                        *e = (tile, count);
                    }
                    e.0.mask = joined;
                }
            }
        }
        let mut f = GrassField { tiles: map.into_values().map(|(t, _)| t).collect(), chunk: 8.0, bins: Default::default() };
        f.tiles.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.z.total_cmp(&b.z)));
        for (k, t) in f.tiles.iter().enumerate() {
            let c = ((t.x / f.chunk).floor() as i32, (t.z / f.chunk).floor() as i32);
            f.bins.entry(c).or_default().push(k as u32);
        }
        f
    }

    /// The tiles within `radius` of `eye` (nearest first, at most `cap`), into `out`.
    pub fn near(&self, eye: Vec3, radius: f32, cap: usize, out: &mut Vec<u32>) {
        out.clear();
        let mut cand: Vec<(f32, u32)> = Vec::new();
        let r = (radius / self.chunk).ceil() as i32;
        let (ci, cj) = ((eye.x / self.chunk).floor() as i32, (eye.z / self.chunk).floor() as i32);
        for i in ci - r..=ci + r {
            for j in cj - r..=cj + r {
                let Some(list) = self.bins.get(&(i, j)) else { continue };
                for &k in list {
                    let t = &self.tiles[k as usize];
                    let mid = Vec3::new(t.x + TILE * 0.5, t.y, t.z + TILE * 0.5);
                    let d = mid.distance(eye);
                    if d < radius {
                        cand.push((d, k));
                    }
                }
            }
        }
        if cand.len() > cap {
            cand.select_nth_unstable_by(cap, |a, b| a.0.total_cmp(&b.0));
            cand.truncate(cap);
        }
        cand.sort_by(|a, b| a.0.total_cmp(&b.0));
        out.extend(cand.into_iter().map(|(_, k)| k));
    }

    /// Packs tiles as `TEXELS` RGBA32F texels each (the shader's layout).
    pub fn pack(&self, list: &[u32], out: &mut [[f32; 4]]) {
        for (n, &k) in list.iter().enumerate() {
            let t = &self.tiles[k as usize];
            let o = n * TEXELS as usize;
            if o + 2 >= out.len() {
                break;
            }
            out[o] = [t.x, t.z, t.y, t.mask as f32 + t.kind as f32 * 65536.0];
            out[o + 1] = [t.dydx, t.dydz, t.lm[0], t.lm[1]];
            out[o + 2] = [t.lm_dx[0], t.lm_dx[1], t.lm_dz[0], t.lm_dz[1]];
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_square_metre() {
        let p = |x: f32, z: f32| Vec3::new(x, 0.0, z);
        let l = Vec2::ZERO;
        let tris = [
            GroundTri { p: [p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0)], lm: [l; 3], kind: 0 },
            GroundTri { p: [p(0.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)], lm: [l; 3], kind: 0 },
        ];
        let f = GrassField::build(&tris, 0.5);
        assert_eq!(f.tiles.len(), 4);
        assert!(f.tiles.iter().all(|t| t.mask == 0xFFFF));
        let mut near = Vec::new();
        f.near(Vec3::new(0.25, 0.0, 0.25), 0.3, 10, &mut near);
        assert_eq!(near.len(), 1);
    }

    #[test]
    fn slope_and_light_map() {
        let tris = [GroundTri {
            p: [Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 1.0, 0.0), Vec3::new(0.0, 0.0, 2.0)],
            lm: [Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0)],
            kind: 2,
        }];
        let f = GrassField::build(&tris, 0.5);
        let t = f.tiles.iter().find(|t| t.x == 0.0 && t.z == 0.0).unwrap();
        assert!((t.dydx - 0.5).abs() < 1e-5 && t.dydz.abs() < 1e-5);
        assert!((t.lm_dx[0] - 0.5).abs() < 1e-5 && (t.lm_dz[1] - 0.5).abs() < 1e-5);
        assert_eq!(t.kind, 2);
    }
}
