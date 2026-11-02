//! See-through level faces (masked: vent covers, chain-link fences, grates) for lines of sight: people see the
//! player through them (ScriptedPawn's sight traces pass masked surfaces in the original). Built per map from the
//! level's masked batches, binned by 128-unit cells.

use bevy::math::DVec3;
use std::collections::HashMap;
use std::sync::Mutex;

const CELL: f64 = 128.0;

struct Index {
    gen: u64,
    cells: HashMap<(i32, i32, i32), Vec<[DVec3; 3]>>,
}

static IDX: Mutex<Option<Index>> = Mutex::new(None);

fn cell(p: DVec3) -> (i32, i32, i32) {
    ((p.x / CELL).floor() as i32, (p.y / CELL).floor() as i32, (p.z / CELL).floor() as i32)
}

fn build(lvl: &dx_level::Level) -> HashMap<(i32, i32, i32), Vec<[DVec3; 3]>> {
    let mut cells: HashMap<(i32, i32, i32), Vec<[DVec3; 3]>> = HashMap::new();
    let ue = |p: [f32; 3]| {
        let u = dx_level::bevy_to_ue(p);
        DVec3::new(u[0] as f64, u[1] as f64, u[2] as f64)
    };
    for b in lvl.batches.iter().filter(|b| b.class == dx_level::SurfaceClass::Masked && !b.sky) {
        for t in b.indices.chunks_exact(3) {
            let tri = [ue(b.positions[t[0] as usize]), ue(b.positions[t[1] as usize]), ue(b.positions[t[2] as usize])];
            let lo = cell(tri[0].min(tri[1]).min(tri[2]) - DVec3::splat(4.0));
            let hi = cell(tri[0].max(tri[1]).max(tri[2]) + DVec3::splat(4.0));
            // (Huge faces - a whole fence - span many cells; cap the work.)
            if ((hi.0 - lo.0 + 1) * (hi.1 - lo.1 + 1) * (hi.2 - lo.2 + 1)) > 4096 {
                continue;
            }
            for x in lo.0..=hi.0 {
                for y in lo.1..=hi.1 {
                    for z in lo.2..=hi.2 {
                        cells.entry((x, y, z)).or_default().push(tri);
                    }
                }
            }
        }
    }
    cells
}

fn inside(t: &[DVec3; 3], p: DVec3) -> bool {
    let n = (t[1] - t[0]).cross(t[2] - t[0]);
    let l = n.length();
    if l < 1e-6 {
        return false;
    }
    let n = n / l;
    if (p - t[0]).dot(n).abs() > 3.0 {
        return false;
    }
    let q = p - n * (p - t[0]).dot(n);
    let side = |a: DVec3, b: DVec3| (b - a).cross(q - a).dot(n);
    let (s0, s1, s2) = (side(t[0], t[1]), side(t[1], t[2]), side(t[2], t[0]));
    (s0 >= -1.0 && s1 >= -1.0 && s2 >= -1.0) || (s0 <= 1.0 && s1 <= 1.0 && s2 <= 1.0)
}

/// Does this point (a trace's hit on the level) lie on a see-through face?
pub fn on_grate(lvl: &dx_level::Level, p: DVec3) -> bool {
    let gen = crate::loader::generation();
    let Ok(mut g) = IDX.lock() else { return false };
    if g.as_ref().is_none_or(|i| i.gen != gen) {
        *g = Some(Index { gen, cells: build(lvl) });
    }
    let Some(i) = g.as_ref() else { return false };
    i.cells.get(&cell(p)).is_some_and(|list| list.iter().any(|t| inside(t, p)))
}
