//! Movers (doors, lifts, hatches): their brush geometry, collision and keyframes, from `Mover`/`DeusExMover`.
//!
//! A mover's `Brush` is its own small BSP `Model` in brush-local space. World placement follows Unreal's
//! brush transform: world = Location + Rotation * (local - PrePivot) (scales are 1 on every Island mover).
//! Keyframes are offsets from `BasePos`/`BaseRot`.

use crate::actors::{ActorInfo, MeshPart};
use crate::collision::BspCollision;
use crate::SurfaceClass;
use unreal_pkg::objects::{self, pf};
use unreal_pkg::props::{find, Value};
use unreal_pkg::Package;

#[derive(Debug, Clone)]
pub struct MoverInfo {
    /// Index into `Level::actors`.
    pub actor: usize,
    pub name: String,
    pub tag: String,
    pub event: String,
    /// Geometry relative to the mover's Location (i.e. already minus PrePivot), Unreal space.
    pub parts: Vec<MeshPart>,
    /// Collision in raw brush-local space (add PrePivot after un-rotating a world point).
    pub collision: BspCollision,
    /// How far any part of the brush reaches from the mover's Location (whatever its rotation): traces that pass
    /// farther away than this (plus their extent) can't touch it.
    pub reach: f64,
    pub pre_pivot: [f32; 3],
    pub base_pos: [f32; 3],
    pub base_rot: [i32; 3],
    pub key_pos: [[f32; 3]; 8],
    pub key_rot: [[i32; 3]; 8],
    pub num_keys: u8,
    /// Starting keyframe and current placement (as saved in the map).
    pub key_num: u8,
    pub location: [f32; 3],
    pub rotation: [i32; 3],
    pub move_time: f32,
    pub stay_open_time: f32,
    pub delay_time: f32,
    /// UnrealScript state name: TriggerToggle, TriggerOpenTimed, BumpOpenTimed, StandOpenTimed, ...
    pub initial_state: String,
    /// 0 = MV_MoveByTime (linear), 1 = MV_GlideByTime (ease in/out).
    pub glide: u8,
    pub locked: bool,
    pub frobbable: bool,
    pub one_way: bool,
    pub trigger_once: bool,
    pub is_door: bool,
}

fn fv(p: &[unreal_pkg::props::Property], n: &str, d: f32) -> f32 {
    match find(p, n) {
        Some(Value::Float(v)) => *v,
        _ => d,
    }
}
fn bv(p: &[unreal_pkg::props::Property], n: &str) -> bool {
    matches!(find(p, n), Some(Value::Bool(true)))
}
fn byv(p: &[unreal_pkg::props::Property], n: &str, d: u8) -> u8 {
    match find(p, n) {
        Some(Value::Byte(v)) => *v,
        _ => d,
    }
}
fn vecv(p: &[unreal_pkg::props::Property], n: &str, idx: u32) -> Option<[f32; 3]> {
    p.iter().find(|x| x.index == idx && x.name.eq_ignore_ascii_case(n)).and_then(|x| match &x.value {
        Value::Vector(v) => Some(*v),
        _ => None,
    })
}
fn rotv(p: &[unreal_pkg::props::Property], n: &str, idx: u32) -> Option<[i32; 3]> {
    p.iter().find(|x| x.index == idx && x.name.eq_ignore_ascii_case(n)).and_then(|x| match &x.value {
        Value::Rotator(v) => Some(*v),
        _ => None,
    })
}
fn namev(p: &[unreal_pkg::props::Property], n: &str) -> String {
    match find(p, n) {
        Some(Value::Name(s)) => s.clone(),
        _ => String::new(),
    }
}

/// Build every mover. `tex` resolves a material object ref (and whether the face is drawn PF_Masked) to (texture
/// index, masked, width, height).
pub fn build(pkg: &Package, actors: &[ActorInfo], tex: &mut dyn FnMut(i32, bool) -> Option<(usize, bool, f32, f32)>) -> Vec<MoverInfo> {
    let mut out = Vec::new();
    for (ai, a) in actors.iter().enumerate() {
        let p = &a.props;
        if find(p, "KeyPos").is_none() && find(p, "MoveTime").is_none() {
            continue; // not a mover
        }
        let brush = p.iter().find(|x| x.name == "Brush").and_then(|x| match &x.value {
            Value::Path(s) => Some(s.clone()),
            _ => None,
        });
        let Some(brush) = brush else { continue };
        // Path is "<map>.<ModelName>": find the export by its last component.
        let mname = brush.rsplit('.').next().unwrap_or("").to_string();
        let Some(me) = pkg.exports.iter().find(|e| e.name == mname && pkg.class_name(e.class_index) == "Model") else { continue };
        let Ok(model) = objects::read_model(pkg, me) else { continue };
        let pre = match find(p, "PrePivot") {
            Some(Value::Vector(v)) => *v,
            _ => [0.0; 3],
        };

        // Geometry (same face/UV rules as the level BSP), grouped by texture and blend class.
        let mut parts: Vec<MeshPart> = Vec::new();
        for node in &model.nodes {
            if node.num_vertices < 3 {
                continue;
            }
            let Some(surf) = model.surfaces.get(node.surf as usize) else { continue };
            if surf.poly_flags & (pf::INVISIBLE | pf::PORTAL) != 0 {
                continue;
            }
            let t = tex(surf.material, surf.poly_flags & pf::MASKED != 0);
            let (tidx, masked, tw, th) = match t {
                Some((i, m, w, h)) => (Some(i), m, w, h),
                None => (None, false, 256.0, 256.0),
            };
            let class = if surf.poly_flags & pf::MODULATED != 0 {
                SurfaceClass::Modulated
            } else if surf.poly_flags & pf::TRANSLUCENT != 0 {
                SurfaceClass::Translucent
            } else if surf.poly_flags & pf::MASKED != 0 || masked {
                SurfaceClass::Masked
            } else {
                SurfaceClass::Opaque
            };
            let unlit = surf.poly_flags & 0x0040_0000 != 0;
            let two_sided = surf.poly_flags & 0x100 != 0;
            let pi = match parts.iter().position(|q| q.texture == tidx && q.class == class && q.unlit == unlit && q.two_sided == two_sided) {
                Some(i) => i,
                None => {
                    parts.push(MeshPart { texture: tidx, class, unlit, env: false, two_sided, positions: vec![], normals: vec![], uvs: vec![], indices: vec![], vert_ids: vec![], slots: vec![] });
                    parts.len() - 1
                }
            };
            let (Some(base), Some(tu), Some(tv)) = (
                model.points.get(surf.p_base as usize).copied(),
                model.vectors.get(surf.v_texture_u as usize).copied(),
                model.vectors.get(surf.v_texture_v as usize).copied(),
            ) else {
                continue;
            };
            let n = {
                let v = [node.plane[0], node.plane[1], node.plane[2]];
                let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
                [v[0] / l, v[1] / l, v[2] / l]
            };
            let part = &mut parts[pi];
            let first = part.positions.len() as u32;
            let mut ok = true;
            for k in 0..node.num_vertices as usize {
                let Some(pt) = model.vertices.get(node.vert_pool as usize + k).and_then(|v| model.points.get(v.vertex as usize)).copied() else {
                    ok = false;
                    break;
                };
                let d = [pt[0] - base[0], pt[1] - base[1], pt[2] - base[2]];
                let u = (d[0] * tu[0] + d[1] * tu[1] + d[2] * tu[2] + surf.pan_u as f32) / tw;
                let v = (d[0] * tv[0] + d[1] * tv[1] + d[2] * tv[2] + surf.pan_v as f32) / th;
                part.positions.push([pt[0] - pre[0], pt[1] - pre[1], pt[2] - pre[2]]);
                part.normals.push(n);
                part.uvs.push([u, v]);
            }
            if !ok {
                part.positions.truncate(first as usize);
                part.normals.truncate(first as usize);
                part.uvs.truncate(first as usize);
                continue;
            }
            // Bevy front faces are counter-clockwise around the normal; Unreal's polys mostly run the other way.
        let flip = {
            // Newell's normal over the whole polygon (BSP splits leave collinear vertices).
            let cnt = node.num_vertices as u32;
            let mut w = [0.0f32; 3];
            for k in 0..cnt {
                let a = part.positions[(first + k) as usize];
                let c = part.positions[(first + (k + 1) % cnt) as usize];
                w[0] += (a[1] - c[1]) * (a[2] + c[2]);
                w[1] += (a[2] - c[2]) * (a[0] + c[0]);
                w[2] += (a[0] - c[0]) * (a[1] + c[1]);
            }
            w[0] * n[0] + w[1] * n[1] + w[2] * n[2] < 0.0
        };
        for k in 1..node.num_vertices as u32 - 1 {
                part.indices.extend_from_slice(&if flip { [first, first + k + 1, first + k] } else { [first, first + k, first + k + 1] });
            }
        }

        // Faces the brush's BSP left out: non-solid ones (PF_NotSolid / PF_Semisolid, like a vent's grate) never get
        // nodes, but the brush's Polys still have them and the engine draws them - add those from the Polys.
        let in_bsp: std::collections::HashSet<(i32, u32)> = model
            .nodes
            .iter()
            .filter_map(|n| model.surfaces.get(n.surf as usize))
            .map(|sf| (sf.material, sf.poly_flags & !(0x0080_0000 | 0x0000_8000)))
            .collect();
        let polys = if model.polys > 0 {
            pkg.exports.get(model.polys as usize - 1).and_then(|e| objects::read_polys(pkg, e).ok()).unwrap_or_default()
        } else {
            Vec::new()
        };
        for poly in &polys {
            let pflags = poly.poly_flags;
            if poly.vertices.len() < 3 || pflags & (pf::INVISIBLE | pf::PORTAL) != 0 {
                continue;
            }
            if pflags & (pf::NOT_SOLID | 0x20) == 0 || in_bsp.contains(&(poly.texture, pflags & !(0x0080_0000 | 0x0000_8000))) {
                continue;
            }
            let (tidx, masked, tw, th) = match tex(poly.texture, pflags & pf::MASKED != 0) {
                Some((i, m, w, h)) => (Some(i), m, w, h),
                None => (None, false, 256.0, 256.0),
            };
            let class = if pflags & pf::MODULATED != 0 {
                SurfaceClass::Modulated
            } else if pflags & pf::TRANSLUCENT != 0 {
                SurfaceClass::Translucent
            } else if pflags & pf::MASKED != 0 || masked {
                SurfaceClass::Masked
            } else {
                SurfaceClass::Opaque
            };
            let unlit = pflags & 0x0040_0000 != 0;
            let two_sided = pflags & 0x100 != 0;
            let pi = match parts.iter().position(|q| q.texture == tidx && q.class == class && q.unlit == unlit && q.two_sided == two_sided) {
                Some(i) => i,
                None => {
                    parts.push(MeshPart { texture: tidx, class, unlit, env: false, two_sided, positions: vec![], normals: vec![], uvs: vec![], indices: vec![], vert_ids: vec![], slots: vec![] });
                    parts.len() - 1
                }
            };
            let part = &mut parts[pi];
            let first = part.positions.len() as u32;
            let (base, tu, tv, n) = (poly.base, poly.texture_u, poly.texture_v, poly.normal);
            for pt in &poly.vertices {
                let d = [pt[0] - base[0], pt[1] - base[1], pt[2] - base[2]];
                let u = (d[0] * tu[0] + d[1] * tu[1] + d[2] * tu[2] + poly.pan_u as f32) / tw;
                let v = (d[0] * tv[0] + d[1] * tv[1] + d[2] * tv[2] + poly.pan_v as f32) / th;
                part.positions.push([pt[0] - pre[0], pt[1] - pre[1], pt[2] - pre[2]]);
                part.normals.push(n);
                part.uvs.push([u, v]);
            }
            let cnt = poly.vertices.len() as u32;
            let mut w = [0.0f32; 3];
            for k in 0..cnt {
                let a = part.positions[(first + k) as usize];
                let c = part.positions[(first + (k + 1) % cnt) as usize];
                w[0] += (a[1] - c[1]) * (a[2] + c[2]);
                w[1] += (a[2] - c[2]) * (a[0] + c[0]);
                w[2] += (a[0] - c[0]) * (a[1] + c[1]);
            }
            let flip = w[0] * n[0] + w[1] * n[1] + w[2] * n[2] < 0.0;
            for k in 1..cnt - 1 {
                part.indices.extend_from_slice(&if flip { [first, first + k + 1, first + k] } else { [first, first + k, first + k + 1] });
            }
        }

        // A window pane coplanar with a solid face of the same door: the brush has the hole (its inner faces face
        // into it) and the pane, but its BSP kept a solid face over the pane too (the UNATCO HQ doors). The solid
        // face would hide the glass: drop solid triangles lying on a translucent pane.
        drop_covered_by_panes(&mut parts);

        let mut key_pos = [[0.0f32; 3]; 8];
        let mut key_rot = [[0i32; 3]; 8];
        for i in 0..8u32 {
            key_pos[i as usize] = vecv(p, "KeyPos", i).unwrap_or([0.0; 3]);
            key_rot[i as usize] = rotv(p, "KeyRot", i).unwrap_or([0; 3]);
        }
        let location = a.location();
        let reach = model
            .points
            .iter()
            .map(|q| {
                let d = [q[0] - pre[0], q[1] - pre[1], q[2] - pre[2]];
                ((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]) as f64).sqrt()
            })
            .fold(0.0f64, f64::max)
            + 2.0;
        let reach = if model.points.is_empty() { f64::INFINITY } else { reach };
        out.push(MoverInfo {
            actor: ai,
            name: a.name.clone(),
            tag: namev(p, "Tag"),
            event: namev(p, "Event"),
            parts,
            collision: BspCollision::new(&model),
            reach,
            pre_pivot: pre,
            base_pos: vecv(p, "BasePos", 0).unwrap_or(location),
            base_rot: rotv(p, "BaseRot", 0).unwrap_or([0; 3]),
            key_pos,
            key_rot,
            num_keys: byv(p, "NumKeys", 2),
            key_num: byv(p, "KeyNum", 0),
            location,
            rotation: rotv(p, "Rotation", 0).unwrap_or([0; 3]),
            move_time: fv(p, "MoveTime", 1.0),
            stay_open_time: fv(p, "StayOpenTime", 4.0),
            delay_time: fv(p, "DelayTime", 0.0),
            initial_state: namev(p, "InitialState"),
            glide: byv(p, "MoverGlideType", 1),
            locked: bv(p, "bLocked"),
            frobbable: bv(p, "bFrobbable"),
            one_way: bv(p, "bOneWay"),
            trigger_once: bv(p, "bTriggerOnceOnly"),
            is_door: bv(p, "bIsDoor"),
        });
    }
    out
}

/// See the call in `load_movers`.
fn drop_covered_by_panes(parts: &mut [MeshPart]) {
    let panes: Vec<([[f32; 3]; 3], [f32; 3])> = parts
        .iter()
        // (Masked panes too: vent grates' brushes keep a solid face on each grate pane - the vents drew shut.)
        .filter(|p| matches!(p.class, SurfaceClass::Translucent | SurfaceClass::Masked))
        .flat_map(|p| {
            p.indices.chunks_exact(3).map(move |t| {
                let v = [p.positions[t[0] as usize], p.positions[t[1] as usize], p.positions[t[2] as usize]];
                (v, p.normals[t[0] as usize])
            })
        })
        .collect();
    if panes.is_empty() {
        return;
    }
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let inside = |q: [f32; 3], t: &[[f32; 3]; 3], n: [f32; 3]| {
        (0..3).all(|k| dot(cross(sub(t[(k + 1) % 3], t[k]), sub(q, t[k])), n) >= -0.05)
            || (0..3).all(|k| dot(cross(sub(t[(k + 1) % 3], t[k]), sub(q, t[k])), n) <= 0.05)
    };
    for p in parts.iter_mut().filter(|p| p.class == SurfaceClass::Opaque) {
        let before = p.indices.len();
        let kept: Vec<u32> = p
            .indices
            .chunks_exact(3)
            .filter(|t| {
                let v = [p.positions[t[0] as usize], p.positions[t[1] as usize], p.positions[t[2] as usize]];
                let c = [(v[0][0] + v[1][0] + v[2][0]) / 3.0, (v[0][1] + v[1][1] + v[2][1]) / 3.0, (v[0][2] + v[1][2] + v[2][2]) / 3.0];
                let n = p.normals[t[0] as usize];
                !panes.iter().any(|(pt, pn)| {
                    dot(n, *pn).abs() > 0.99 && dot(sub(c, pt[0]), *pn).abs() < 0.5 && inside(c, pt, *pn)
                })
            })
            .flatten()
            .copied()
            .collect();
        if kept.len() != before {
            p.indices = kept;
        }
    }
}
