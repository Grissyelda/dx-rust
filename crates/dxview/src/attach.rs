//! Weapon mods shown on the weapon (ours; MULTIPLAYER roadmap Phase 2, needed before weapon mastery ships): a
//! silencer on the muzzle, a laser sight under the barrel and a scope on top, in first person (the view model) and
//! in third person (the gun in a player's hands, for everyone else). The Visible Attachments mod has its own modelled
//! parts and takes over when it's installed; HDTP's view models aren't fitted (their barrels aren't where this looks).
//!
//! The parts are simple shapes made to fit each gun: the barrel's tip and line are found from the mesh (the
//! muzzle-flash section, else the front of the part reaching furthest forward), their size from the gun's length and
//! the barrel's thickness, the scope set on the gun's top and the laser under the barrel. The view model is a vertex
//! animation, so the parts follow it: three groups of the gun's own vertices (tip, a slice behind it, the top) are
//! tracked frame by frame and the parts moved with the frame they make.

use crate::weapons::{Held, ViewModelRoot, Weapons, VIEW_LAYER};
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use dx_level::actors::MeshPart;
use dx_level::SurfaceClass;

pub const SILENCER: u8 = 1;
pub const LASER: u8 = 2;
pub const SCOPE: u8 = 4;
pub const LASING: u8 = 8;

/// The mods that show, as bits (sent to the other players with the body's state).
pub fn bits(f: crate::visattach::Fitted) -> u8 {
    (f.silencer as u8 * SILENCER) | (f.laser as u8 * LASER) | (f.scope as u8 * SCOPE) | (f.lasing as u8 * LASING)
}

/// Where a gun's barrel is, in some rest space.
#[derive(Clone, Debug)]
pub struct Fit {
    pub tip: Vec3,
    pub axis: Vec3,
    pub up: Vec3,
    /// The gun's length along the axis, and the barrel's radius.
    pub len: f32,
    pub bore: f32,
    /// The gun's top / bottom above the barrel line where the scope / laser go (along `up`).
    pub top: f32,
    pub bottom: f32,
    /// The gun part and the vertex groups that make its frame (indices into that part's vertices).
    pub part: usize,
    pub groups: [Vec<usize>; 3],
}

fn centroid(pts: &[Vec3], ids: &[usize]) -> Vec3 {
    ids.iter().fold(Vec3::ZERO, |a, &i| a + pts[i]) / ids.len().max(1) as f32
}

/// Find the barrel. `parts`: each part's vertices in the rest space, with whether it's solid (not a translucent
/// effect); `flash`: the muzzle flash's centre when the gun has one; `fwd` / `up`: the rest space's forward and up.
pub fn fit(parts: &[(Vec<Vec3>, bool)], flash: Option<Vec3>, fwd: Vec3, up: Vec3) -> Option<Fit> {
    // The gun: the solid part reaching furthest forward.
    let (pi, (pts, _)) = parts
        .iter()
        .enumerate()
        .filter(|(_, p)| p.1 && p.0.len() >= 8)
        .max_by(|a, b| {
            let m = |p: &Vec<Vec3>| p.iter().map(|v| v.dot(fwd)).fold(f32::MIN, f32::max);
            m(&a.1 .0).total_cmp(&m(&b.1 .0))
        })?;
    // Its long axis (the barrel's line; a held gun is angled, so not just "forward"): the principal direction of
    // its vertices, by power iteration on their covariance, pointing forward.
    let mean = pts.iter().copied().sum::<Vec3>() / pts.len() as f32;
    let mut cov = Mat3::ZERO;
    for p in pts {
        let d = *p - mean;
        cov += Mat3::from_cols(d * d.x, d * d.y, d * d.z);
    }
    let mut axis = fwd;
    for _ in 0..32 {
        axis = (cov * axis).normalize_or_zero();
        if axis == Vec3::ZERO {
            axis = fwd;
            break;
        }
    }
    if axis.dot(fwd) < 0.0 {
        axis = -axis;
    }
    // A gun held more than 40 degrees off forward is something else (a sideways grip part): keep forward.
    if axis.dot(fwd) < 0.75 {
        axis = fwd;
    }
    let along: Vec<f32> = pts.iter().map(|v| v.dot(axis)).collect();
    let (lo, hi) = along.iter().fold((f32::MAX, f32::MIN), |a, &x| (a.0.min(x), a.1.max(x)));
    let len = hi - lo;
    if len <= 1e-5 {
        return None;
    }
    let cam_up = (up - axis * up.dot(axis)).normalize_or_zero();
    // The gun's own up (a held gun is rolled): toward its highest point near the front - the front sight, or the
    // top of the slide - away from the barrel's line.
    let front_hi = (0..pts.len())
        .filter(|&i| along[i] >= hi - 0.25 * len)
        .max_by(|&a, &b| pts[a].dot(cam_up).total_cmp(&pts[b].dot(cam_up)));
    let up = front_hi
        .map(|i| {
            let d = pts[i] - mean;
            (d - axis * d.dot(axis)).normalize_or_zero()
        })
        .filter(|u| u.dot(cam_up) > 0.5)
        .unwrap_or(cam_up);
    let side = axis.cross(up).normalize_or_zero();
    let lateral = |v: Vec3, c: Vec3| {
        let d = v - c;
        Vec2::new(d.dot(side), d.dot(up)).length()
    };
    let tip_ids: Vec<usize> = (0..pts.len()).filter(|&i| along[i] >= hi - 0.05 * len).collect();
    // On the bore's line (the flash is centred on it, when there is one), at the very front.
    // (The flash sprite's middle only when it lines up with the front face: some sit off to one side.)
    let face = centroid(pts, &tip_ids);
    let face = face + axis * (hi - face.dot(axis));
    let front = (tip_ids.iter().map(|&i| lateral(pts[i], face)).fold(0.0f32, f32::max)).clamp(0.012 * len, 0.06 * len);
    let tip = match flash.map(|f| f + axis * (hi - f.dot(axis))) {
        Some(f) if lateral(f, face) < front * 0.5 => f,
        _ => face,
    };
    // The barrel: about half the front face's reach round its middle.
    let bore = front * 0.55;
    // The gun's top and bottom over the scope's and laser's stretch (relative to the barrel line).
    let rel = |v: Vec3| (v - tip).dot(up);
    let back = |v: Vec3| (tip - v).dot(axis);
    let top = (0..pts.len()).filter(|&i| (0.3 * len..0.7 * len).contains(&back(pts[i]))).map(|i| rel(pts[i])).fold(bore, f32::max);
    let bottom = (0..pts.len()).filter(|&i| (0.06 * len..0.2 * len).contains(&back(pts[i]))).map(|i| rel(pts[i])).fold(-bore, f32::min);
    // The frame's vertex groups: the tip, the middle of the gun, its top.
    let mid: Vec<usize> = (0..pts.len()).filter(|&i| (along[i] - (lo + hi) / 2.0).abs() < 0.12 * len).collect();
    let mut by_top: Vec<usize> = (0..pts.len()).collect();
    by_top.sort_by(|&a, &b| rel(pts[b]).total_cmp(&rel(pts[a])));
    by_top.truncate((pts.len() / 12).max(3));
    if tip_ids.is_empty() || mid.is_empty() {
        return None;
    }
    Some(Fit { tip, axis, up, len, bore, top, bottom, part: pi, groups: [tip_ids, mid, by_top] })
}

/// The frame three vertex groups make (origin at the first): rotation columns and origin.
fn frame(pts: &[Vec3], groups: &[Vec<usize>; 3]) -> Option<(Mat3, Vec3)> {
    let (a, b, c) = (centroid(pts, &groups[0]), centroid(pts, &groups[1]), centroid(pts, &groups[2]));
    let x = (a - b).normalize_or_zero();
    let z = x.cross(c - b).normalize_or_zero();
    if x == Vec3::ZERO || z == Vec3::ZERO {
        return None;
    }
    let y = z.cross(x);
    Some((Mat3::from_cols(x, y, z), a))
}

// ---------------------------------------------------------------------------------------------------------------
// The parts' shapes.

#[derive(Default)]
struct Geo {
    pos: Vec<[f32; 3]>,
    col: Vec<[f32; 4]>,
    idx: Vec<u32>,
}

impl Geo {
    /// A light from above-front-right baked into the vertex colours (the material is unlit, tinted to the gun's
    /// light).
    fn shade(n: Vec3, up: Vec3, axis: Vec3, base: f32) -> [f32; 4] {
        let l = (up * 0.8 + axis * 0.3 + up.cross(axis) * 0.2).normalize();
        let k = base * (0.45 + 0.55 * n.dot(l).max(0.0));
        [k, k, k * 1.05, 1.0]
    }
    fn tube(&mut self, a: Vec3, b: Vec3, r: f32, up: Vec3, base: f32) {
        let axis = (b - a).normalize_or_zero();
        let side = axis.cross(up).normalize_or_zero();
        let up2 = side.cross(axis);
        const N: u32 = 12;
        let start = self.pos.len() as u32;
        for i in 0..N {
            let t = i as f32 / N as f32 * std::f32::consts::TAU;
            let n = side * t.cos() + up2 * t.sin();
            for end in [a, b] {
                self.pos.push((end + n * r).to_array());
                self.col.push(Self::shade(n, up, axis, base));
            }
        }
        for i in 0..N {
            let j = (i + 1) % N;
            let (p, q, r2, s) = (start + i * 2, start + i * 2 + 1, start + j * 2, start + j * 2 + 1);
            self.idx.extend([p, r2, q, q, r2, s]);
        }
        // End caps.
        for (end, flip) in [(a, true), (b, false)] {
            let c = self.pos.len() as u32;
            self.pos.push(end.to_array());
            self.col.push(Self::shade(if flip { -axis } else { axis }, up, axis, base * 0.7));
            for i in 0..N {
                let t = i as f32 / N as f32 * std::f32::consts::TAU;
                self.pos.push((end + (side * t.cos() + up2 * t.sin()) * r).to_array());
                self.col.push(Self::shade(if flip { -axis } else { axis }, up, axis, base * 0.7));
            }
            for i in 0..N {
                let (p, q) = (c + 1 + i, c + 1 + (i + 1) % N);
                if flip {
                    self.idx.extend([c, q, p]);
                } else {
                    self.idx.extend([c, p, q]);
                }
            }
        }
    }
    fn block(&mut self, centre: Vec3, axis: Vec3, up: Vec3, half: Vec3, base: f32) {
        let side = axis.cross(up).normalize_or_zero();
        let up = side.cross(axis);
        let dirs = [(axis, half.x, side, half.z, up, half.y), (side, half.z, up, half.y, axis, half.x), (up, half.y, axis, half.x, side, half.z)];
        for (n, d, u, hu, v, hv) in dirs {
            for sgn in [1.0f32, -1.0] {
                let n = n * sgn;
                let c = centre + n * d;
                let s = self.pos.len() as u32;
                for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                    self.pos.push((c + u * hu * a + v * hv * b).to_array());
                    self.col.push(Self::shade(n, up, axis, base));
                }
                self.idx.extend([s, s + 1, s + 2, s, s + 2, s + 3]);
            }
        }
    }
    fn mesh(self) -> Mesh {
        let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.pos);
        m.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.col);
        m.insert_indices(Indices::U32(self.idx));
        m
    }
}

/// The parts for these mods on a gun with this fit, in its rest space: the gun-metal mesh and the laser's lens.
fn build(f: &Fit, mods: u8) -> (Option<Mesh>, Option<Mesh>) {
    let (t, a, u, l) = (f.tip, f.axis, f.up, f.len);
    let mut g = Geo::default();
    if mods & SILENCER != 0 {
        let r = (f.bore * 1.4).max(0.018 * l);
        g.tube(t - a * 0.02 * l, t + a * 0.17 * l, r, u, 0.22);
    }
    if mods & SCOPE != 0 {
        let r = 0.024 * l;
        let h = f.top + r * 1.6;
        let (back, front) = (t - a * 0.62 * l + u * h, t - a * 0.36 * l + u * h);
        g.tube(back, front, r, u, 0.2);
        g.tube(front - a * 0.02 * l, front + a * 0.03 * l, r * 1.3, u, 0.24);
        g.tube(back - a * 0.03 * l, back + a * 0.02 * l, r * 1.2, u, 0.24);
        // Two rings down onto the gun.
        for k in [0.4, 0.62] {
            let c = t - a * k * l + u * (f.top + r * 0.5);
            g.block(c, a, u, Vec3::new(0.012 * l, r * 0.8, r * 0.55), 0.18);
        }
    }
    let mut lens = None;
    if mods & LASER != 0 {
        let r = 0.016 * l;
        let c = t - a * 0.12 * l + u * (f.bottom - r * 1.4);
        g.block(c, a, u, Vec3::new(0.05 * l, r, r), 0.26);
        // The emitter's lens on its front, bright red while the laser is on.
        let mut e = Geo::default();
        e.tube(c + a * 0.05 * l, c + a * 0.056 * l, r * 0.6, u, 1.0);
        lens = Some(e.mesh());
    }
    let metal = (!g.pos.is_empty()).then(|| g.mesh());
    (metal, lens)
}

// ---------------------------------------------------------------------------------------------------------------
// First person.

#[derive(Component)]
pub struct ViewAttach;

#[derive(Default)]
pub struct ViewState {
    /// What's built: (weapon class, view mesh, mods) and the fit in the camera rest space with its rest frame.
    key: Option<(String, String, u8)>,
    fit: Option<(Fit, Mat3, Vec3)>,
    parts: Vec<Entity>,
    metal: Option<Handle<StandardMaterial>>,
    lens: Option<Handle<StandardMaterial>>,
    tint: f32,
}

/// The rest pose of a view model in camera space, by part (Still / Idle1 / the first real sequence, frame 0).
fn rest_parts(h: &Held) -> Vec<(Vec<Vec3>, bool)> {
    let am = &h.mesh;
    let Some(seq) = ["Still", "Idle1", "Idle"]
        .iter()
        .find_map(|n| am.seqs.iter().find(|s| s.name.eq_ignore_ascii_case(n)))
        .or_else(|| am.seqs.iter().find(|s| !s.name.eq_ignore_ascii_case("All")))
    else {
        return Vec::new();
    };
    let (start, frames) = (seq.start_frame.max(0) as usize, seq.num_frames.max(1) as usize);
    let sc = h.stats.view_scale;
    h.parts
        .iter()
        .map(|p| {
            let pts = p
                .vert_ids
                .iter()
                .map(|&v| {
                    let q = am.sample(start, frames, 0.0, v as usize);
                    Vec3::from(crate::weapons::view_to_cam_pub([q[0] * sc, q[1] * sc, q[2] * sc]))
                })
                .collect();
            (pts, solid(p))
        })
        .collect()
}

fn solid(p: &MeshPart) -> bool {
    p.class != SurfaceClass::Translucent && !p.unlit
}

fn flash_centre(parts: &[(Vec<Vec3>, bool)], mesh_parts: &[MeshPart]) -> Option<Vec3> {
    let i = mesh_parts.iter().position(|p| p.class == SurfaceClass::Translucent && p.unlit)?;
    let pts = &parts.get(i)?.0;
    (!pts.is_empty()).then(|| pts.iter().copied().sum::<Vec3>() / pts.len() as f32)
}

/// Shows a mod's part on the weapon: not when Visible Attachments draws its own, not on HDTP's models.
fn draws_own(h: &Held) -> bool {
    crate::visattach::active() || crate::hdtp::is_view_mesh(&h.stats.class, &h.stats.view_mesh)
}

#[allow(clippy::too_many_arguments)]
pub fn view_attach_system(
    mut commands: Commands,
    weapons: Res<Weapons>,
    root: Query<Entity, With<ViewModelRoot>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut tfs: Query<&mut Transform, With<ViewAttach>>,
    mut st: Local<ViewState>,
) {
    let held = weapons.held.as_ref();
    let mods = held.filter(|h| !draws_own(h)).map_or(0, |h| bits(h.fitted()));
    let key = held.filter(|_| mods & (SILENCER | LASER | SCOPE) != 0).map(|h| (h.stats.class.clone(), h.stats.view_mesh.clone(), mods & (SILENCER | LASER | SCOPE)));
    if key != st.key && env_on!("DXVIEW_ATTACHLOG") {
        println!("ATTACH view {:?} own {:?} fitted {}", key, held.map(draws_own), held.map_or(0, |h| bits(h.fitted())));
    }
    if key != st.key {
        for e in st.parts.drain(..) {
            commands.entity(e).despawn();
        }
        st.fit = None;
        st.key = key.clone();
        let (Some(h), Some(_), Ok(root)) = (held, key, root.single()) else { return };
        let parts = rest_parts(h);
        let flash = flash_centre(&parts, &h.parts);
        let Some(f) = fit(&parts, flash, Vec3::NEG_Z, Vec3::Y) else { return };
        if let Some(path) = env_val!("DXVIEW_ATTACHDUMP") {
            let mut out = String::new();
            for (i, (pts, solid)) in parts.iter().enumerate() {
                for p in pts {
                    out += &format!("{i} {} {} {} {}\n", *solid as u8, p.x, p.y, p.z);
                }
            }
            if let Some(sh) = h.shown.get(f.part) {
                for p in sh {
                    out += &format!("S 0 {} {} {}\n", p[0], p[1], p[2]);
                }
            }
            let _ = std::fs::write(path, out);
        }
        if env_on!("DXVIEW_ATTACHLOG") {
            println!("ATTACH fit tip {:?} axis {:?} len {:.3} bore {:.4} top {:.3} bottom {:.3} part {} flash {:?}", f.tip, f.axis, f.len, f.bore, f.top, f.bottom, f.part, flash);
        }
        let Some((r0, o0)) = frame(&parts[f.part].0, &f.groups) else { return };
        if st.metal.is_none() {
            // (Both sides: the view model's root can be mirrored, which turns the faces' winding round.)
            st.metal = Some(materials.add(StandardMaterial { base_color: Color::WHITE, unlit: true, double_sided: true, cull_mode: None, ..default() }));
            st.lens = Some(materials.add(StandardMaterial { base_color: Color::srgb(1.0, 0.1, 0.08), unlit: true, double_sided: true, cull_mode: None, ..default() }));
        }
        let (metal, lens) = build(&f, mods);
        for (m, mat) in [(metal, st.metal.clone()), (lens, st.lens.clone())] {
            let (Some(m), Some(mat)) = (m, mat) else { continue };
            let e = commands
                .spawn((Mesh3d(meshes.add(m)), MeshMaterial3d(mat), Transform::default(), RenderLayers::layer(VIEW_LAYER), bevy::light::NotShadowCaster, ViewAttach, ChildOf(root)))
                .id();
            st.parts.push(e);
        }
        st.fit = Some((f, r0, o0));
    }
    let (Some(h), Some((f, r0, o0))) = (held, st.fit.as_ref()) else { return };
    // Follow the model's pose: the frame its tracked vertices make now against the rest frame.
    let Some(pts) = h.shown.get(f.part) else { return };
    let pts: Vec<Vec3> = pts.iter().map(|p| Vec3::from(*p)).collect();
    if f.groups.iter().flatten().any(|&i| i >= pts.len()) {
        return;
    }
    let Some((r1, o1)) = frame(&pts, &f.groups) else { return };
    let rot = r1 * r0.transpose();
    let t = Transform { translation: o1 - rot * *o0, rotation: Quat::from_mat3(&rot), scale: Vec3::ONE };
    if let Some(path) = env_val!("DXVIEW_ATTACHDUMP") {
        let out: String = pts.iter().map(|p| format!("L 0 {} {} {}\n", p.x, p.y, p.z)).collect();
        let _ = std::fs::write(format!("{path}.live"), format!("{out}T {:?} {:?} {:?}\n", t.translation, rot, *o0));
    }
    for e in &st.parts {
        if let Ok(mut tf) = tfs.get_mut(*e) {
            tf.set_if_neq(t);
        }
    }
    // Lit like the gun: its vertex colours' brightness (the view model is lit on the CPU).
    let tint = h
        .handles
        .get(f.part)
        .and_then(|hm| meshes.get(hm))
        .and_then(|m| match m.attribute(Mesh::ATTRIBUTE_COLOR) {
            Some(bevy::mesh::VertexAttributeValues::Float32x4(c)) if !c.is_empty() => {
                let step = (c.len() / 64).max(1);
                let (sum, n) = c.iter().step_by(step).fold((0.0, 0), |a, v| (a.0 + (v[0] + v[1] + v[2]) / 3.0, a.1 + 1));
                Some(sum / n as f32)
            }
            _ => None,
        })
        .unwrap_or(1.0)
        .clamp(0.15, 2.5);
    if (tint - st.tint).abs() > 0.04 {
        st.tint = tint;
        if let Some(mut m) = st.metal.as_ref().and_then(|m| materials.get_mut(m)) {
            m.base_color = Color::srgb(tint, tint, tint);
        }
        let on = h.lasing;
        if let Some(mut m) = st.lens.as_ref().and_then(|m| materials.get_mut(m)) {
            m.base_color = if on { Color::srgb(1.6, 0.12, 0.08) } else { Color::srgb(0.35 * tint, 0.05, 0.05) };
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Third person: another player's gun (ai::npc_weapon_models calls this when it builds the model).

/// Spawn the mods' parts under a third-person weapon model (`root`: the model's own space; `parts`: its mesh parts
/// in that space). Returns the entities.
pub fn third_person(commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>, root: Entity, parts: &[MeshPart], mods: u8) -> Vec<Entity> {
    if mods & (SILENCER | LASER | SCOPE) == 0 || crate::visattach::active() {
        return Vec::new();
    }
    let rest: Vec<(Vec<Vec3>, bool)> = parts.iter().map(|p| (p.positions.iter().map(|q| Vec3::from(*q)).collect(), solid(p))).collect();
    let flash = flash_centre(&rest, parts);
    // Third-person weapon meshes point down their own +X (Bevy +X after ue_to_bevy), up +Z (Bevy +Y).
    let Some(f) = fit(&rest, flash, Vec3::X, Vec3::Y) else { return Vec::new() };
    let (metal, lens) = build(&f, mods);
    let mut out = Vec::new();
    let grey = materials.add(StandardMaterial { base_color: Color::srgb(0.6, 0.6, 0.62), perceptual_roughness: 0.5, metallic: 0.4, double_sided: true, cull_mode: None, ..default() });
    let red = materials.add(StandardMaterial {
        base_color: Color::srgb(0.8, 0.05, 0.05),
        emissive: if mods & LASING != 0 { LinearRgba::rgb(4.0, 0.2, 0.1) } else { LinearRgba::BLACK },
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    for (m, mat) in [(metal, grey), (lens, red)] {
        let Some(m) = m else { continue };
        out.push(commands.spawn((Mesh3d(meshes.add(m)), MeshMaterial3d(mat), Transform::default(), ChildOf(root))).id());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box-shaped "gun" along -Z with a thin barrel at its front.
    fn gun() -> Vec<(Vec<Vec3>, bool)> {
        let mut pts = Vec::new();
        for z in 0..20 {
            let z = -(z as f32);
            for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                pts.push(Vec3::new(x, y, z));
            }
        }
        for z in 20..26 {
            let z = -(z as f32);
            for (x, y) in [(-0.3, 0.6), (0.3, 0.6), (0.3, 0.9), (-0.3, 0.9)] {
                pts.push(Vec3::new(x, y, z));
            }
        }
        vec![(pts, true)]
    }

    #[test]
    fn finds_the_barrel() {
        let f = fit(&gun(), None, Vec3::NEG_Z, Vec3::Y).unwrap();
        assert!((f.tip.z + 25.0).abs() < 0.05, "tip {:?}", f.tip);
        assert!((f.tip.y - 0.75).abs() < 0.05, "tip on the barrel line {:?}", f.tip);
        assert!(f.axis.dot(Vec3::NEG_Z) > 0.99);
        assert!((f.len - 25.0).abs() < 0.2);
        assert!(f.top > 0.2, "the body's top is above the barrel line");
        let (m, l) = build(&f, SILENCER | SCOPE | LASER);
        assert!(m.is_some() && l.is_some());
    }

    #[test]
    fn frame_follows_a_moved_gun() {
        let g = gun();
        let f = fit(&g, None, Vec3::NEG_Z, Vec3::Y).unwrap();
        let (r0, o0) = frame(&g[0].0, &f.groups).unwrap();
        let q = Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-0.2);
        let moved: Vec<Vec3> = g[0].0.iter().map(|p| q * *p + Vec3::new(1.0, 2.0, 3.0)).collect();
        let (r1, o1) = frame(&moved, &f.groups).unwrap();
        let rot = r1 * r0.transpose();
        let t = o1 - rot * o0;
        let p = f.tip;
        let want = q * p + Vec3::new(1.0, 2.0, 3.0);
        assert!((rot * p + t - want).length() < 1e-3);
    }
}
