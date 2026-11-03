//! Enhancement: diegetic screens (setting DiegeticScreens, Interface group, off by default).
//!
//! Terminals and keypads are used in the world instead of on a screen cut out of it (after Doom 3 and Prey).
//! - Computers (personal, public, security, ATM): the view eases in to the terminal's screen while the terminal
//!   plays its Activate animation (Computers state On; the security terminal unfolds), and the original computer
//!   UI - every window of it, rendered by computers.rs exactly as before - is drawn onto the monitor's face by a
//!   UI camera rendering into an image. The mouse works on the screen through a ray from the view; the cursor is
//!   drawn on the monitor. Move the mouse off the screen for a second (or Esc out) and the terminal closes, plays
//!   Deactivate (the security one folds up) and the view eases back.
//! - Keypads: the view eases in to the keypad and its own buttons are pressed with the mouse (the code also goes
//!   in from the number keys, as before); a small display above the pad shows the code and the result. The
//!   keypad window (menus.rs) runs the logic and sounds but isn't drawn.
//!
//! Each screen is a rectangle in the mesh's own space (bottom-left, bottom-right, top-left corners as the viewer
//! sees them), found from the meshes' faces (dx-level example screenfaces).

use crate::{FlyCam, LevelRes, SkyCam};
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use glam::DVec3;
use std::sync::Mutex;

pub fn enabled() -> bool {
    setting!("DiegeticScreens") > 0.5
}

/// A screen: three corners in the actor's space (Unreal units, DrawScale 1).
#[derive(Clone, Copy, Debug)]
struct ScreenDef {
    bl: [f64; 3],
    br: [f64; 3],
    tl: [f64; 3],
    /// The fourth corner when the screen isn't a rectangle (the ATM's display is a trapezoid painted on its panel).
    tr: Option<[f64; 3]>,
}

/// Keypad buttons: the 3 x 4 grid of button centres (columns, rows) in the face's (y, z), and the pitch.
#[derive(Clone, Copy, Debug)]
struct KeyGrid {
    cols: [f64; 3],
    rows: [f64; 4],
    pitch: f64,
    /// The pad face's x (front surface).
    x: f64,
    /// How far the mesh reaches above the face's top edge (the display goes above it).
    above: f64,
}

fn screen_def(class: &str) -> Option<ScreenDef> {
    Some(match class {
        // The middle of the three-panel monitor.
        "ComputerPersonal" => ScreenDef { bl: [17.55, 7.61, -0.14], br: [-17.55, 7.61, -0.14], tl: [17.55, 12.69, 24.18], tr: None },
        "ComputerPublic" => ScreenDef { bl: [3.0, 14.55, 4.29], br: [3.0, -14.55, 4.29], tl: [1.9, 14.55, 28.91], tr: None },
        // The screen inside the unfolded security terminal (Activate's last frame).
        "ComputerSecurity" => ScreenDef { bl: [-0.59, 8.95, -4.3], br: [-0.59, -8.96, -4.3], tl: [-0.59, 8.95, 4.39], tr: None },
        // The cyan "WELCOME" display painted on the recessed panel (ATMTex1 pixels (60,143) (112,143) (117,165)
        // (56,165), carried onto the panel's two faces through their texture mapping).
        // (Grown to the whole dark recess, as seen against the panel: a little taller at both edges, a little wider.)
        "ATM" => ScreenDef { bl: [0.82, 10.62, -7.44], br: [0.82, -4.01, -7.44], tl: [-0.99, 9.51, 3.43], tr: Some([-0.99, -2.93, 3.43]) },
        _ => return None,
    })
}

/// The terminal's own glowing monitor panel, measured from the mesh it is drawn with (an HDTP or other model
/// replaces the original's, whose numbers `screen_def` holds): the see-through (translucent / modulated) faces
/// turned like the original's screen, split into connected panels, and the panel nearest the original screen's
/// middle squared into a rectangle. Actor-local, DrawScale 1, like ScreenDef. None when nothing fits.
fn mesh_screen(ma: &dx_level::actors::MeshActor, scale: f64, def: &ScreenDef) -> Option<ScreenDef> {
    let v = |p: [f64; 3]| DVec3::from_array(p);
    let (bl, br, tl) = (v(def.bl), v(def.br), v(def.tl));
    let right = (br - bl).normalize_or_zero();
    let up0 = tl - bl;
    let up = (up0 - right * up0.dot(right)).normalize_or_zero();
    let n = up.cross(right).normalize_or_zero();
    let mid = bl + (br - bl) * 0.5 + up0 * 0.5;
    // Triangles (local, DrawScale undone) facing the way the screen faces.
    let mut tris: Vec<[DVec3; 3]> = Vec::new();
    for part in &ma.parts {
        if !matches!(part.class, dx_level::SurfaceClass::Translucent | dx_level::SurfaceClass::Modulated) {
            continue;
        }
        for t in part.indices.chunks_exact(3) {
            let p = |i: u32| {
                let q = part.positions[i as usize];
                // positions are Unreal-space, actor-local, rotated by the entity: undo nothing but DrawScale.
                DVec3::new(q[0] as f64, q[1] as f64, q[2] as f64) / scale.max(1e-3)
            };
            let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
            let tn = (b - a).cross(c - a);
            if tn.length() < 1e-6 || tn.normalize().dot(n).abs() < 0.85 {
                continue;
            }
            tris.push([a, b, c]);
        }
    }
    if tris.is_empty() {
        return None;
    }
    // Connected panels (shared corners, to 0.05 units).
    let key = |p: DVec3| ((p.x * 20.0).round() as i64, (p.y * 20.0).round() as i64, (p.z * 20.0).round() as i64);
    let mut owner: std::collections::HashMap<(i64, i64, i64), usize> = Default::default();
    let mut parent: Vec<usize> = (0..tris.len()).collect();
    fn root(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    for (ti, t) in tris.iter().enumerate() {
        for c in t {
            match owner.get(&key(*c)) {
                Some(&o) => {
                    let (ra, rb) = (root(&mut parent, ti), root(&mut parent, o));
                    parent[ra] = rb;
                }
                None => {
                    owner.insert(key(*c), ti);
                }
            }
        }
    }
    let mut groups: std::collections::HashMap<usize, Vec<usize>> = Default::default();
    for ti in 0..tris.len() {
        let r = root(&mut parent, ti);
        groups.entry(r).or_default().push(ti);
    }
    // The panel whose middle is nearest the original screen's middle (along the screen, not through it).
    let flat = |p: DVec3| p - n * (p - mid).dot(n);
    let best = groups.values().min_by(|a, b| {
        let c = |g: &Vec<usize>| {
            let s: DVec3 = g.iter().flat_map(|&i| tris[i].iter().copied()).sum::<DVec3>() / (g.len() * 3) as f64;
            flat(s).distance(mid)
        };
        c(a).total_cmp(&c(b))
    })?;
    let (mut u0, mut u1, mut v0, mut v1, mut d) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN, 0.0);
    for &i in best {
        for p in tris[i] {
            let q = p - bl;
            u0 = u0.min(q.dot(right));
            u1 = u1.max(q.dot(right));
            v0 = v0.min(q.dot(up));
            v1 = v1.max(q.dot(up));
            d += q.dot(n);
        }
    }
    d /= (best.len() * 3) as f64;
    if u1 - u0 < 4.0 || v1 - v0 < 3.0 {
        return None;
    }
    let at = |u: f64, vv: f64| (bl + right * u + up * vv + n * d).to_array();
    Some(ScreenDef { bl: at(u0, v0), br: at(u1, v0), tl: at(u0, v1), tr: None })
}

/// Texture pixel -> face (y, z) for a keypad mesh: u, v are 0..255 across the texture.
fn keypad_def(class: &str) -> Option<(ScreenDef, KeyGrid)> {
    // (u0 -> y0, u1 -> y1), (v0 -> z0, v1 -> z1), texture size in pixels, button centres in pixels.
    let (uy, vz, tw, th, cx, cy): ((f64, f64, f64, f64), (f64, f64, f64, f64), f64, f64, [f64; 3], [f64; 4]) = match class {
        "Keypad1" => ((83.0, 4.0, 255.0, -4.0), (42.0, 4.0, 253.0, -6.0), 64.0, 64.0, [43.2, 49.4, 55.4], [33.1, 38.8, 44.4, 50.6]),
        "Keypad2" => ((1.0, 4.45, 253.0, -4.45), (1.0, 7.41, 189.0, -7.41), 64.0, 128.0, [19.4, 33.5, 47.9], [20.0, 33.1, 46.2, 59.4]),
        "Keypad3" => ((1.0, 6.25, 253.0, -6.25), (1.0, 4.5, 208.0, -4.5), 64.0, 64.0, [32.9, 43.3, 53.0], [6.3, 14.4, 22.5, 30.4]),
        _ => return None,
    };
    let y_of = |px: f64| uy.1 + (px * 256.0 / tw - uy.0) / (uy.2 - uy.0) * (uy.3 - uy.1);
    let z_of = |py: f64| vz.1 + (py * 256.0 / th - vz.0) / (vz.2 - vz.0) * (vz.3 - vz.1);
    let cols = cx.map(y_of);
    let rows = cy.map(z_of);
    let pitch = ((cols[1] - cols[0]).abs()).min((rows[0] - rows[1]).abs());
    let x = 1.0;
    let above = if class == "Keypad1" { 2.0 } else { 0.0 };
    let (y0, y1, z0, z1) = (uy.1, uy.3, vz.1, vz.3);
    let def = ScreenDef { bl: [x, y0, z1], br: [x, y1, z1], tl: [x, y0, z0], tr: None };
    Some((def, KeyGrid { cols, rows, pitch, x, above }))
}

/// A screen placed in the world (Unreal space).
#[derive(Clone, Copy, Debug)]
struct Rect3 {
    c: DVec3,
    right: DVec3,
    up: DVec3,
    n: DVec3,
    w: f64,
    h: f64,
    bl: DVec3,
    /// Corners (bl, br, tr, tl) of a screen that isn't a rectangle: the picture is spread over them (bilinear) and
    /// the mouse is found back through the same mapping.
    quad: Option<[DVec3; 4]>,
    tl_raw: DVec3,
}

impl Rect3 {
    fn with_tr(mut self, tr: Option<DVec3>) -> Rect3 {
        if let Some(tr) = tr {
            let br = self.bl + self.right * self.w;
            // The real top-left (not squared to the bottom edge).
            let tl = self.tl_raw;
            self.quad = Some([self.bl, br, tr, tl]);
        }
        self
    }
    /// A point of the screen by (u, v): 0..1 left to right, bottom to top.
    fn at(&self, u: f64, v: f64) -> DVec3 {
        match self.quad {
            Some([a, b, c, d]) => a + (b - a) * u + (d - a) * v + (a - b + c - d) * (u * v),
            None => self.bl + self.right * (self.w * u) + self.up * (self.h * v),
        }
    }
    fn new(bl: DVec3, br: DVec3, tl: DVec3) -> Rect3 {
        let r = br - bl;
        let u = tl - bl;
        let right = r.normalize_or_zero();
        // The up edge made square to the bottom edge (the tilted panels are near enough to rectangles).
        let up = (u - right * u.dot(right)).normalize_or_zero();
        let n = up.cross(right).normalize_or_zero();
        let (w, h) = (r.length(), u.dot(up));
        Rect3 { c: bl + right * w * 0.5 + up * h * 0.5, right, up, n, w, h, bl, quad: None, tl_raw: tl }
    }
    /// Where a ray meets the screen's plane, as (u, v): 0..1 left to right, top to bottom.
    fn hit(&self, o: DVec3, d: DVec3) -> Option<(f64, f64)> {
        let den = d.dot(self.n);
        if den.abs() < 1e-6 {
            return None;
        }
        let t = (self.c - o).dot(self.n) / den;
        if t <= 0.0 {
            return None;
        }
        let p = o + d * t - self.bl;
        let (x, y) = (p.dot(self.right), p.dot(self.up));
        if let Some(q) = self.quad {
            // Inverse bilinear in the screen's plane (a few Newton steps from the middle).
            let to2 = |v: DVec3| bevy::math::DVec2::new((v - self.bl).dot(self.right), (v - self.bl).dot(self.up));
            let (a, b, c, dd) = (to2(q[0]), to2(q[1]), to2(q[2]), to2(q[3]));
            let target = bevy::math::DVec2::new(x, y);
            let (mut u, mut v) = (0.5f64, 0.5f64);
            for _ in 0..8 {
                let f = a + (b - a) * u + (dd - a) * v + (a - b + c - dd) * (u * v) - target;
                let du = (b - a) + (a - b + c - dd) * v;
                let dv = (dd - a) + (a - b + c - dd) * u;
                let det = du.x * dv.y - du.y * dv.x;
                if det.abs() < 1e-9 {
                    break;
                }
                u -= (f.x * dv.y - f.y * dv.x) / det;
                v -= (du.x * f.y - du.y * f.x) / det;
            }
            return Some((u, 1.0 - v));
        }
        Some((x / self.w, 1.0 - y / self.h))
    }
}

/// How far a terminal has moved from where the map put it (a computer saved a few units over its desk drops onto
/// it at the start: its screen has to go down with it).
fn shift_of(decos: &Query<&crate::actors::Deco>, mi: usize) -> DVec3 {
    decos.iter().find(|d| d.mesh_actor == mi).map_or(DVec3::ZERO, |d| d.body.location - d.start)
}

fn to_world(ma: &dx_level::actors::MeshActor, scale: f64, p: [f64; 3]) -> DVec3 {
    let ax = dx_level::actors::rotator_axes(ma.rotation);
    let v = [p[0] * scale, p[1] * scale, p[2] * scale];
    let mut o = [0.0f64; 3];
    for a in 0..3 {
        o[a] = ax[0][a] as f64 * v[0] + ax[1][a] as f64 * v[1] + ax[2][a] as f64 * v[2];
    }
    DVec3::new(ma.location[0] as f64 + o[0], ma.location[1] as f64 + o[1], ma.location[2] as f64 + o[2])
}

fn ue(v: Vec3) -> DVec3 {
    let e = dx_level::bevy_to_ue(v.to_array());
    DVec3::new(e[0] as f64, e[1] as f64, e[2] as f64)
}

fn bevy(p: DVec3) -> Vec3 {
    Vec3::from(dx_level::ue_to_bevy(p.as_vec3().to_array()))
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    Opening,
    Active,
    Closing,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Computer,
    Keypad,
}

struct Session {
    kind: Kind,
    actor: usize,
    mesh_actor: Option<usize>,
    rect: Rect3,
    grid: Option<(KeyGrid, f64)>,
    phase: Phase,
    t: f32,
    /// How long the Activate animation runs (the view waits for it).
    unfold: f32,
    /// Seconds the mouse has been off the screen.
    out: f32,
    mouse_uv: Option<(f64, f64)>,
    /// Computers: the terminal's own cursor (u, v: 0..1 left to right, top to bottom), moved by the mouse's motion.
    cursor: (f64, f64),
    /// Keypad button under the mouse, and one being shown pressed (index, seconds).
    hover: Option<usize>,
    flash: Option<(usize, f32)>,
    /// Desktop size in UI units and the scale it's drawn at (computers).
    desk: (i32, i32, i32),
    /// How far the terminal moved from its map place (shift_of).
    off: DVec3,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);
/// The computer UI's view for computers.rs while a terminal is diegetic.
static VIEW: Mutex<Option<CompView>> = Mutex::new(None);
static LEAVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The gamepad's cursor motion (logical pixels) while a terminal has its own cursor (keybinds.rs adds to it).
static PAD_DELTA: Mutex<Vec2> = Mutex::new(Vec2::ZERO);

/// A terminal is up with its own cursor: the pointer stays locked, the mouse's motion moves the cursor on the
/// monitor (keybinds.rs keeps the pad in its menu mode and hands its cursor motion here).
pub fn virtual_cursor() -> bool {
    SESSION.lock().ok().is_some_and(|s| s.as_ref().is_some_and(|s| s.kind == Kind::Computer && s.phase != Phase::Closing))
}

pub fn add_pad_delta(d: Vec2) {
    if let Ok(mut p) = PAD_DELTA.lock() {
        *p += d;
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CompView {
    /// Desktop size in UI units, and the whole-number scale drawn.
    pub sw: i32,
    pub sh: i32,
    pub s: i32,
    /// The mouse on the desktop (UI units), None while off the screen or not ready.
    pub mouse: Option<(i32, i32)>,
    /// The UI camera the computer's nodes render through.
    pub cam: Entity,
}

/// While a terminal is shown on its monitor: how computers.rs lays out and where its mouse is.
pub fn computer_view() -> Option<CompView> {
    VIEW.lock().ok().and_then(|v| *v)
}

/// The mouse left the screen for a second: the terminal closes (computers.rs).
pub fn take_leave() -> bool {
    LEAVE.swap(false, std::sync::atomic::Ordering::Relaxed)
}

/// A keypad is being used in the world: its window isn't drawn.
pub fn keypad_active() -> bool {
    SESSION.lock().ok().is_some_and(|s| s.as_ref().is_some_and(|s| s.kind == Kind::Keypad))
}

#[derive(Component)]
pub struct DgScreen;
#[derive(Component)]
pub struct DgCam;
#[derive(Component)]
pub struct DgKeyLcd;
#[derive(Component)]
pub struct DgKeyHover;

#[derive(Default)]
pub struct DgRes {
    screen: Option<(Entity, Handle<StandardMaterial>)>,
    cam: Option<(Entity, Handle<Image>, (i32, i32, i32))>,
    lcd: Option<(Entity, Handle<Image>, String)>,
    hover: Option<Entity>,
    gen: u64,
    /// Which screen shape the screen entity's mesh was built for.
    screen_key: u64,
}

fn quad_mesh(w: f32, h: f32) -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    let (x, y) = (w * 0.5, h * 0.5);
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[-x, -y, 0.0], [x, -y, 0.0], [x, y, 0.0], [-x, y, 0.0]]);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; 4]);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]);
    m.insert_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
    m
}

/// A screen that isn't a rectangle, as a mesh in the world (Bevy space): an 8 x 8 grid spread bilinearly over its
/// corners, `lift` units out along its normal, the picture's top-left at (0, 0).
fn screen_mesh(r: &Rect3, lift: f64) -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    const N: u32 = 8;
    let mut pos = Vec::new();
    let mut uv = Vec::new();
    for j in 0..=N {
        for i in 0..=N {
            let (u, v) = (i as f64 / N as f64, j as f64 / N as f64);
            pos.push(bevy(r.at(u, v) + r.n * lift).to_array());
            uv.push([u as f32, 1.0 - v as f32]);
        }
    }
    let n = (bevy(r.c + r.n) - bevy(r.c)).normalize_or_zero();
    let mut idx = Vec::new();
    for j in 0..N {
        for i in 0..N {
            let a = j * (N + 1) + i;
            idx.extend_from_slice(&[a, a + 1, a + N + 2, a, a + N + 2, a + N + 1]);
        }
    }
    let count = pos.len();
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![n.to_array(); count]);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    m.insert_indices(Indices::U32(idx));
    m
}

/// While a terminal is in use, its own see-through glow panels (ComputerPersonal's translucent holo screens) are
/// hidden: drawn over the live picture they washed it out into the room behind.
pub fn hide_glass_system(
    level: Res<LevelRes>,
    actors: Query<(&crate::actors::MeshActorRef, &crate::actors::Animated, &Children)>,
    mut parts: Query<(&Mesh3d, &mut Visibility), Without<DgScreen>>,
    mut hidden: Local<Vec<Entity>>,
) {
    let want = SESSION
        .lock()
        .ok()
        .and_then(|s| s.as_ref().filter(|s| s.kind == Kind::Computer && s.phase != Phase::Closing).and_then(|s| s.mesh_actor));
    if want.is_none() {
        for e in hidden.drain(..) {
            if let Ok((_, mut v)) = parts.get_mut(e) {
                v.set_if_neq(Visibility::Inherited);
            }
        }
        return;
    }
    if !hidden.is_empty() {
        return;
    }
    let lvl = &level.0;
    for (r, a, kids) in &actors {
        if Some(r.0) != want {
            continue;
        }
        let Some(ma) = lvl.mesh_actors.get(r.0) else { continue };
        for c in kids.iter() {
            let Ok((m3, mut v)) = parts.get_mut(c) else { continue };
            let Some(pi) = a.parts.iter().find(|(h, _)| *h == m3.0).map(|(_, i)| *i) else { continue };
            if ma.parts.get(pi).is_some_and(|p| matches!(p.class, dx_level::SurfaceClass::Translucent | dx_level::SurfaceClass::Modulated)) {
                v.set_if_neq(Visibility::Hidden);
                hidden.push(c);
            }
        }
    }
}

/// A transform putting a quad (facing +Z, X right, Y up) on a rectangle, `lift` units out of it.
fn quad_tf(r: &Rect3, at: DVec3, lift: f64) -> Transform {
    let p = bevy(at + r.n * lift);
    let x = (bevy(r.c + r.right) - bevy(r.c)).normalize();
    let y = (bevy(r.c + r.up) - bevy(r.c)).normalize();
    let z = x.cross(y).normalize();
    Transform::from_translation(p).with_rotation(Quat::from_mat3(&Mat3::from_cols(x, y, z)))
}

fn mesh_actor_of(lvl: &dx_level::Level, actor: usize) -> Option<usize> {
    lvl.mesh_actors.iter().position(|m| m.actor == actor)
}

fn draw_scale(a: &dx_level::actors::ActorInfo) -> f64 {
    match unreal_pkg::props::find(&a.props, "DrawScale") {
        Some(unreal_pkg::props::Value::Float(f)) => *f as f64,
        _ => 1.0,
    }
}

/// Plays a sequence on the terminal's animator; returns its length in seconds.
fn play(lvl: &dx_level::Level, anims: &mut Query<(&mut crate::actors::Animated, &crate::actors::MeshActorRef)>, mesh_actor: Option<usize>, seq: &str) -> f32 {
    let Some(mi) = mesh_actor else { return 0.0 };
    let Some(anim) = lvl.mesh_actors[mi].anim.as_ref() else { return 0.0 };
    let Some(clip) = crate::weapons::Clip::named(&anim.mesh, seq, false) else { return 0.0 };
    let len = clip.frames as f32 / clip.rate.max(0.1);
    for (mut an, r) in anims.iter_mut() {
        if r.0 == mi {
            an.clip = Some(clip);
            an.tween = None;
            break;
        }
    }
    len
}

const CAM_IN: f32 = 0.55;
const CAM_OUT: f32 = 0.45;
const ZOOM_FOV: f32 = 40.0;
/// How much of the view's height (and width) a computer's monitor fills while in use.
const FRAME_H: f64 = 0.62;
const FRAME_W: f64 = 0.8;

fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// How far in the view is (0 = the eyes, 1 = at the screen).
fn blend(s: &Session) -> f32 {
    match s.phase {
        Phase::Opening => smooth(s.t / CAM_IN),
        Phase::Active => 1.0,
        Phase::Closing => 1.0 - smooth(s.t / CAM_OUT),
    }
}

/// Starts, runs and ends a session; computes the mouse on the screen; keeps the screen quad, the computers' UI
/// camera and the keypad display.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn diegetic_system(
    mut commands: Commands,
    time: Res<Time<Real>>,
    level: Res<LevelRes>,
    blockers: Res<crate::actors::Blockers>,
    window: Single<&Window>,
    cam: Single<(&Camera, &GlobalTransform), (With<FlyCam>, Without<SkyCam>)>,
    mut anims: Query<(&mut crate::actors::Animated, &crate::actors::MeshActorRef)>,
    mut res: Local<DgRes>,
    (mut images, mut meshes, mut materials, mut hud, mut assets): (
        ResMut<Assets<Image>>,
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
        ResMut<crate::classic_hud::ClassicHud>,
        ResMut<crate::weapons::RuntimeAssets>,
    ),
    binds: Res<crate::keybinds::Bindings>,
    motion: Res<bevy::input::mouse::AccumulatedMouseMotion>,
    mut tfs: Query<(&mut Transform, &mut Visibility), Or<(With<DgScreen>, With<DgKeyLcd>, With<DgKeyHover>)>>,
    decos: Query<&crate::actors::Deco>,
) {
    let _prof = crate::prof::scope("diegetic::diegetic_system");
    let lvl = &level.0;
    let dt = time.delta_secs().min(0.1);
    if crate::loader::new_map(&mut res.gen) {
        if let Ok(mut s) = SESSION.lock() {
            *s = None;
        }
        if let Ok(mut v) = VIEW.lock() {
            *v = None;
        }
        for e in [res.screen.take().map(|s| s.0), res.cam.take().map(|c| c.0), res.lcd.take().map(|l| l.0), res.hover.take()].into_iter().flatten() {
            commands.entity(e).despawn();
        }
    }
    let mut guard = SESSION.lock().unwrap_or_else(|e| e.into_inner());
    // ---- start
    if guard.is_none() && enabled() {
        let comp = crate::computers::open_actor();
        let keypad = crate::menus::keypad_blocker();
        if let Some(actor) = comp {
            let a = &lvl.actors[actor];
            if let (Some(def), Some(mi)) = (screen_def(&a.class), mesh_actor_of(lvl, actor)) {
                let ma = &lvl.mesh_actors[mi];
                let sc = draw_scale(a);
                // The personal terminal's picture goes into its own glowing monitor panel as drawn (another model
                // than the original's put the screen where the original's panel was: floating off the monitor).
                let def = if a.class == "ComputerPersonal" { mesh_screen(ma, sc, &def).unwrap_or(def) } else { def };
                if env_on!("DXVIEW_DGLOG") {
                    println!("DG screen {} bl {:?} br {:?} tl {:?}", a.class, def.bl, def.br, def.tl);
                }
                let off = shift_of(&decos, mi);
                let rect = Rect3::new(to_world(ma, sc, def.bl) + off, to_world(ma, sc, def.br) + off, to_world(ma, sc, def.tl) + off)
                    .with_tr(def.tr.map(|t| to_world(ma, sc, t) + off));
                let unfold = play(lvl, &mut anims, Some(mi), "Activate");
                // The desktop: 640 x 480 stretched to the screen's shape; drawn at 2x for sharp text up close.
                let aspect = rect.w / rect.h.max(1e-3);
                let (sw, sh) = if aspect >= 4.0 / 3.0 { ((480.0 * aspect).round() as i32, 480) } else { (640, (640.0 / aspect).round() as i32) };
                *guard = Some(Session {
                    kind: Kind::Computer,
                    actor,
                    mesh_actor: Some(mi),
                    rect,
                    grid: None,
                    phase: Phase::Opening,
                    t: 0.0,
                    unfold,
                    out: 0.0,
                    mouse_uv: None,
                    cursor: (0.5, 0.45),
                    hover: None,
                    flash: None,
                    desk: (sw, sh, 2),
                    off,
                });
            }
        } else if let Some(b) = keypad {
            if let Some(actor) = blockers.list.get(b).map(|c| c.actor) {
                let a = &lvl.actors[actor];
                if let (Some((def, grid)), Some(mi)) = (keypad_def(&a.class), mesh_actor_of(lvl, actor)) {
                    let ma = &lvl.mesh_actors[mi];
                    let sc = draw_scale(a);
                    let off = shift_of(&decos, mi);
                    let rect = Rect3::new(to_world(ma, sc, def.bl) + off, to_world(ma, sc, def.br) + off, to_world(ma, sc, def.tl) + off);
                    *guard = Some(Session {
                        kind: Kind::Keypad,
                        actor,
                        mesh_actor: Some(mi),
                        rect,
                        grid: Some((grid, sc)),
                        phase: Phase::Opening,
                        t: 0.0,
                        unfold: 0.0,
                        out: 0.0,
                        mouse_uv: None,
                        cursor: (0.5, 0.5),
                        hover: None,
                        flash: None,
                        desk: (0, 0, 1),
                        off,
                    });
                }
            }
        }
    }
    // ---- run
    let mut view = None;
    let mut end = false;
    if let Some(s) = guard.as_mut() {
        s.t += dt;
        let ui_open = match s.kind {
            Kind::Computer => crate::computers::open_actor() == Some(s.actor),
            Kind::Keypad => crate::menus::keypad_blocker().is_some(),
        };
        if s.phase != Phase::Closing && !ui_open {
            s.phase = Phase::Closing;
            s.t = 0.0;
            if s.kind == Kind::Computer {
                play(lvl, &mut anims, s.mesh_actor, "Deactivate");
            }
        }
        if s.phase == Phase::Opening && s.t >= CAM_IN.max(s.unfold) {
            s.phase = Phase::Active;
            s.t = 0.0;
        }
        if s.phase == Phase::Closing && s.t >= CAM_OUT {
            end = true;
        }
        // The mouse on the screen.
        s.mouse_uv = None;
        let pad_d = PAD_DELTA.lock().map(|mut p| std::mem::take(&mut *p)).unwrap_or_default();
        if s.phase == Phase::Active && s.kind == Kind::Computer {
            // The terminal's own cursor: the mouse's motion over the screen's size as seen now, so it moves like a
            // pointer would across the monitor, and never leaves it.
            let (camera, gt) = *cam;
            let px = |u: f64, v: f64| camera.world_to_viewport(gt, bevy(s.rect.at(u, v))).ok();
            let (pw, ph) = match (px(0.0, 0.0), px(1.0, 0.0), px(0.0, 1.0)) {
                (Some(a), Some(b), Some(c)) => ((b - a).length().max(60.0), (c - a).length().max(45.0)),
                _ => (640.0, 480.0),
            };
            let sens = (crate::settings::mouse_sensitivity() / 3.0).clamp(0.4, 2.5);
            let d = motion.delta / window.scale_factor().max(0.1) * sens + pad_d;
            s.cursor.0 = (s.cursor.0 + d.x as f64 / pw as f64).clamp(0.0, 0.999);
            s.cursor.1 = (s.cursor.1 + d.y as f64 / ph as f64).clamp(0.0, 0.999);
            if let Some(uv) = env_val!("DXVIEW_DG_MOUSE").map(str::to_owned).and_then(|v| {
                let p: Vec<f64> = v.split(',').filter_map(|x| x.parse().ok()).collect();
                (p.len() == 2).then(|| (p[0], p[1]))
            }) {
                s.cursor = uv;
            }
            s.mouse_uv = Some(s.cursor);
        } else if s.phase == Phase::Active {
            let (camera, gt) = *cam;
            if let Some(ray) = window.cursor_position().and_then(|c| camera.viewport_to_world(gt, c).ok()) {
                let o = ue(ray.origin);
                let d = (ue(ray.origin + *ray.direction) - o).normalize_or_zero();
                if let Some((u, v)) = s.rect.hit(o, d) {
                    let inside = (-0.002..=1.002).contains(&u) && (-0.002..=1.002).contains(&v);
                    // Keypads: the pad's whole face and its display above it count.
                    let near = s.kind == Kind::Keypad && (-0.25..=1.25).contains(&u) && (-0.6..=1.2).contains(&v);
                    if inside || near {
                        s.mouse_uv = Some((u, v));
                    }
                }
            }
            // Dev aid: DXVIEW_DG_MOUSE="u,v" holds the mouse there on the screen.
            if let Some(uv) = env_val!("DXVIEW_DG_MOUSE").map(str::to_owned).and_then(|v| {
                let p: Vec<f64> = v.split(',').filter_map(|x| x.parse().ok()).collect();
                (p.len() == 2).then(|| (p[0], p[1]))
            }) {
                s.mouse_uv = Some(uv);
            }
            // Dev aid: DXVIEW_DG_LEAVE=<secs> takes the mouse off the screen after that long.
            if env_val!("DXVIEW_DG_LEAVE").map(str::to_owned).and_then(|v| v.parse::<f32>().ok()).is_some_and(|t| s.t > t) {
                s.mouse_uv = None;
            }
            if s.mouse_uv.is_some() || (window.cursor_position().is_none() && !env_on!("DXVIEW_DG_LEAVE")) {
                s.out = 0.0;
            } else {
                s.out += dt;
                if s.out >= 1.0 {
                    s.out = 0.0;
                    if env_on!("DXVIEW_DGLOG") {
                        println!("DG leave {:?}", s.kind);
                    }
                    // A keypad stays up until Escape (pad B / Circle) or the code is entered; a computer's cursor
                    // can't leave its monitor (step away with Escape, right click or a walk key: computers.rs).
                    if s.kind == Kind::Computer && env_on!("DXVIEW_DG_LEAVE") {
                        LEAVE.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }
        }
        // Keypad buttons.
        s.hover = None;
        if let (Kind::Keypad, Some((g, sc)), Some((u, v))) = (s.kind, s.grid, s.mouse_uv) {
            let p = s.rect.bl + s.rect.right * (u * s.rect.w) + s.rect.up * ((1.0 - v) * s.rect.h);
            // The point in the face's (y, z): back through the actor's axes.
            if let Some(ma) = s.mesh_actor.map(|i| &lvl.mesh_actors[i]) {
                let mut best = None;
                for (r, &z) in g.rows.iter().enumerate() {
                    for (c, &y) in g.cols.iter().enumerate() {
                        let at = to_world(ma, sc, [g.x, y, z]) + s.off;
                        let dd = at.distance(p);
                        if dd < g.pitch * sc * 0.6 && best.is_none_or(|(_, b)| dd < b) {
                            best = Some((r * 3 + c, dd));
                        }
                    }
                }
                s.hover = best.map(|b| b.0);
            }
            if binds.buttons.just_pressed(MouseButton::Left) {
                if let Some(i) = s.hover {
                    crate::menus::keypad_press_index(i);
                    s.flash = Some((i, 0.15));
                }
            }
        }
        if let Some((_, t)) = s.flash.as_mut() {
            *t -= dt;
        }
        if s.flash.is_some_and(|f| f.1 <= 0.0) {
            s.flash = None;
        }
        if s.kind == Kind::Computer && s.phase != Phase::Closing {
            let (sw, sh, sc) = s.desk;
            view = Some((sw, sh, sc, s.phase == Phase::Active, s.mouse_uv.map(|(u, v)| ((u * sw as f64).floor() as i32, (v * sh as f64).floor() as i32))));
        }
    }
    // ---- the computer UI camera and its image
    let want_cam = view.map(|v| (v.0, v.1, v.2));
    if res.cam.as_ref().map(|c| c.2) != want_cam {
        if let Some((e, img, _)) = res.cam.take() {
            commands.entity(e).despawn();
            images.remove(&img);
        }
        if let Some((sw, sh, sc)) = want_cam {
            let img = images.add(Image::new_target_texture((sw * sc) as u32, (sh * sc) as u32, TextureFormat::Bgra8UnormSrgb, None));
            let e = commands
                .spawn((
                    Camera2d,
                    Camera { order: -40, clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() },
                    RenderTarget::Image(img.clone().into()),
                    DgCam,
                ))
                .id();
            res.cam = Some((e, img, (sw, sh, sc)));
        }
    }
    if let Ok(mut v) = VIEW.lock() {
        *v = match (view, res.cam.as_ref()) {
            (Some((sw, sh, s, ready, mouse)), Some((cam_e, _, _))) => Some(CompView { sw, sh, s, mouse: if ready { mouse } else { None }, cam: *cam_e }),
            _ => None,
        };
    }
    // ---- the screen quad (computers: the UI image; shown once the terminal has opened)
    let show_screen = guard.as_ref().is_some_and(|s| s.kind == Kind::Computer && s.phase == Phase::Active);
    let cam_img = res.cam.as_ref().map(|c| c.1.clone());
    if let (Some(s), Some(img)) = (guard.as_ref(), cam_img.as_ref()) {
        if s.kind == Kind::Computer {
            // A little out from the monitor's surface (its own glow panels are hidden meanwhile: hide_glass_system).
            let lift = 0.4;
            let (tf, size) = match s.rect.quad {
                // A trapezoid: the mesh itself is the screen in the world (rebuilt for each terminal).
                Some(_) => (Transform::IDENTITY, (1.0, 1.0)),
                None => (quad_tf(&s.rect, s.rect.c, lift), (s.rect.w as f32 / 52.5, s.rect.h as f32 / 52.5)),
            };
            let key = s.rect.quad.map(|q| q[0].x.to_bits() ^ q[2].y.to_bits().rotate_left(7) ^ q[3].z.to_bits().rotate_left(13)).unwrap_or(1);
            if res.screen.is_some() && res.screen_key != key {
                if let Some((e, _)) = res.screen.as_ref() {
                    let mesh = match s.rect.quad {
                        Some(_) => screen_mesh(&s.rect, lift),
                        None => quad_mesh(1.0, 1.0),
                    };
                    commands.entity(*e).insert(Mesh3d(meshes.add(mesh)));
                }
                res.screen_key = key;
            }
            match res.screen.as_ref() {
                Some((e, mat)) => {
                    if let Some(mut m) = materials.get_mut(mat) {
                        if m.base_color_texture.as_ref() != Some(img) {
                            m.base_color_texture = Some(img.clone());
                        }
                    }
                    if let Ok((mut t, mut v)) = tfs.get_mut(*e) {
                        if *t != tf {
                            *t = tf;
                        }
                        v.set_if_neq(if show_screen { Visibility::Visible } else { Visibility::Hidden });
                    }
                }
                None => {
                    let mat = materials.add(StandardMaterial {
                        base_color_texture: Some(img.clone()),
                        unlit: true,
                        cull_mode: None,
                        double_sided: true,
                        ..default()
                    });
                    let mesh = match s.rect.quad {
                        Some(_) => screen_mesh(&s.rect, lift),
                        None => quad_mesh(1.0, 1.0),
                    };
                    res.screen_key = key;
                    let e = commands
                        .spawn((
                            Mesh3d(meshes.add(mesh)),
                            MeshMaterial3d(mat.clone()),
                            tf.with_scale(Vec3::new(size.0, size.1, 1.0)),
                            Visibility::Hidden,
                            DgScreen,
                            bevy::light::NotShadowCaster,
                            bevy::light::NotShadowReceiver,
                        ))
                        .id();
                    res.screen = Some((e, mat));
                }
            }
            if let Some((e, _)) = res.screen.as_ref() {
                if let Ok((mut t, _)) = tfs.get_mut(*e) {
                    t.scale = Vec3::new(size.0, size.1, 1.0);
                }
            }
        }
    } else if let Some((e, _)) = res.screen.as_ref() {
        if let Ok((_, mut v)) = tfs.get_mut(*e) {
            v.set_if_neq(Visibility::Hidden);
        }
    }
    // ---- keypads: the display above the pad and the button under the mouse
    let kp = guard.as_ref().filter(|s| s.kind == Kind::Keypad && s.phase != Phase::Closing).map(|s| (s.rect, s.hover, s.flash, s.grid, s.mesh_actor));
    if let Some((rect, hover, flash, grid, mi)) = kp {
        let (input, result, len) = crate::menus::keypad_state().unwrap_or_default();
        let label = match result {
            Some(true) => "GRANTED".to_string(),
            Some(false) => "DENIED".to_string(),
            // The digits typed, then a dash for each one still to come.
            None => input.chars().map(|c| c.to_string()).chain(std::iter::repeat_n("-".to_string(), len.saturating_sub(input.chars().count()))).collect::<Vec<_>>().join(" "),
        };
        let key = format!("{label}|{result:?}");
        // Display: 5 x 1.4 units, just above the face.
        let (lw, lh) = (rect.w.max(6.0) * 0.95, 1.6);
        let above = grid.map_or(0.0, |g| g.0.above * g.1);
        let at = rect.bl + rect.right * rect.w * 0.5 + rect.up * (rect.h + above + lh * 0.5 + 0.5);
        // (Out from the wall plate a keypad sits in.)
        let tf = quad_tf(&rect, at, 1.5).with_scale(Vec3::new(lw as f32 / 52.5, lh as f32 / 52.5, 1.0));
        if res.lcd.as_ref().map(|l| l.2.as_str()) != Some(key.as_str()) {
            let img = lcd_image(&mut hud, &mut assets, &label, result);
            match res.lcd.as_mut() {
                Some((e, h, k)) => {
                    if let Some(mut im) = images.get_mut(h) {
                        *im = img;
                    }
                    *k = key;
                    let _ = e;
                }
                None => {
                    let h = images.add(img);
                    let mat = materials.add(StandardMaterial { base_color_texture: Some(h.clone()), unlit: true, cull_mode: None, ..default() });
                    let e = commands
                        .spawn((Mesh3d(meshes.add(quad_mesh(1.0, 1.0))), MeshMaterial3d(mat), Transform::default(), Visibility::Hidden, DgKeyLcd, bevy::light::NotShadowCaster))
                        .id();
                    res.lcd = Some((e, h, key));
                }
            }
        }
        if let Some((e, _, _)) = res.lcd.as_ref() {
            if let Ok((mut t, mut v)) = tfs.get_mut(*e) {
                *t = tf;
                v.set_if_neq(Visibility::Visible);
            }
        }
        // The hovered (or just pressed) button lit.
        let shown = flash.map(|f| f.0).or(hover);
        if res.hover.is_none() {
            let mat = materials.add(StandardMaterial {
                base_color: Color::srgba(0.6, 0.9, 1.0, 0.35),
                unlit: true,
                alpha_mode: AlphaMode::Add,
                cull_mode: None,
                ..default()
            });
            let e = commands.spawn((Mesh3d(meshes.add(quad_mesh(1.0, 1.0))), MeshMaterial3d(mat), Transform::default(), Visibility::Hidden, DgKeyHover, bevy::light::NotShadowCaster)).id();
            res.hover = Some(e);
        }
        if let (Some(e), Some((g, sc)), Some(mi)) = (res.hover, grid, mi) {
            if let Ok((mut t, mut v)) = tfs.get_mut(e) {
                match shown {
                    Some(i) => {
                        let ma = &lvl.mesh_actors[mi];
                        let at = to_world(ma, sc, [g.x, g.cols[i % 3], g.rows[i / 3]]) + shift_of(&decos, mi);
                        let k = (g.pitch * sc * 0.85) as f32 / 52.5;
                        *t = quad_tf(&rect, at, 0.2).with_scale(Vec3::new(k, k, 1.0));
                        v.set_if_neq(Visibility::Visible);
                    }
                    None => *v = Visibility::Hidden,
                }
            }
        }
    } else {
        for e in [res.lcd.as_ref().map(|l| l.0), res.hover].into_iter().flatten() {
            if let Ok((_, mut v)) = tfs.get_mut(e) {
                v.set_if_neq(Visibility::Hidden);
            }
        }
    }
    if end {
        *guard = None;
    }
}

/// The keypad's display: the code typed (or the result) in green on black, 96 x 24 pixels.
fn lcd_image(hud: &mut crate::classic_hud::ClassicHud, assets: &mut crate::weapons::RuntimeAssets, text: &str, result: Option<bool>) -> Image {
    use crate::uicanvas::{Canvas, HAlign, Style, VAlign};
    let (w, h) = (96, 24);
    let mut c = Canvas::new(w, h, 1);
    // A brushed-metal bezel round a dark green glass.
    c.fill(0, 0, w, h, Style::Normal, [92, 100, 110]);
    c.fill(1, 1, w - 2, h - 2, Style::Normal, [40, 46, 54]);
    c.fill(3, 3, w - 6, h - 6, Style::Normal, [8, 22, 12]);
    let col = match result {
        Some(true) => [96, 255, 120],
        Some(false) => [255, 80, 64],
        None => [110, 240, 130],
    };
    if let Some(font) = hud.font(assets, "FontMenuHeaders") {
        c.text(&font, 0, 0, w, h, HAlign::Center, VAlign::Center, col, text);
    }
    let mut img = Image::new(
        bevy::render::render_resource::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        c.to_rgba8(),
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::default(),
    );
    img.sampler = bevy::image::ImageSampler::nearest();
    img
}

/// The view's place for drawing (PostUpdate, before transforms propagate): eased from the eyes to in front of the
/// screen; the eyes come back in First (game logic always sees them).
static EYE: Mutex<Option<(Transform, f32, Visibility)>> = Mutex::new(None);

pub fn place_system(
    window: Single<&Window>,
    mut cam: Single<(&mut Transform, &mut Projection), (With<FlyCam>, Without<SkyCam>)>,
    mut vm: Single<&mut Visibility, With<crate::weapons::ViewModelRoot>>,
) {
    let guard = SESSION.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = guard.as_ref() else { return };
    let k = blend(s);
    let (tf, proj) = &mut *cam;
    let Projection::Perspective(pp) = &mut **proj else { return };
    let eye_tf = **tf;
    let eye_fov = pp.fov;
    if let Ok(mut e) = EYE.lock() {
        *e = Some((eye_tf, eye_fov, **vm));
    }
    // Fit the screen. Computers keep the player's own field of view with the monitor filling about 60% of the
    // view's height, the room still round it (as a terminal in Prey / Human Revolution); keypads (with room for
    // the display above) at the zoom field of view.
    let fov = match s.kind {
        Kind::Computer => eye_fov,
        Kind::Keypad => ZOOM_FOV.to_radians(),
    };
    let aspect = (window.width() / window.height().max(1.0)) as f64;
    let r = &s.rect;
    let (h_fit, c) = match s.kind {
        Kind::Computer => ((r.h / FRAME_H).max(r.w / (aspect * FRAME_W)), r.c),
        Kind::Keypad => ((r.h + 5.0).max(r.w / aspect) * 1.3, r.c + r.up * 2.0),
    };
    let dist = (h_fit * 0.5) / (fov as f64 * 0.5).tan();
    let pos = bevy(c + r.n * dist);
    let up = (bevy(c + r.up) - bevy(c)).normalize();
    let target = Transform::from_translation(pos).looking_at(bevy(c), up);
    if env_on!("DXVIEW_DGLOG") {
        println!("DG {:?} {:?} rect c={:?} n={:?} up={:?} right={:?} w={:.1} h={:.1} dist={dist:.1} k={k:.2} eye={:?}", s.kind, s.phase, r.c, r.n, r.up, r.right, r.w, r.h, eye_tf.translation);
    }
    tf.translation = eye_tf.translation.lerp(target.translation, k);
    tf.rotation = eye_tf.rotation.slerp(target.rotation, k);
    pp.fov = eye_fov + (fov - eye_fov) * k;
    if k > 0.02 {
        vm.set_if_neq(Visibility::Hidden);
    }
}

pub fn restore_system(
    mut cam: Single<(&mut Transform, &mut Projection), (With<FlyCam>, Without<SkyCam>)>,
    mut vm: Single<&mut Visibility, With<crate::weapons::ViewModelRoot>>,
) {
    if let Some((t, fov, v)) = EYE.lock().ok().and_then(|mut e| e.take()) {
        let (tf, proj) = &mut *cam;
        **tf = t;
        if let Projection::Perspective(pp) = &mut **proj {
            pp.fov = fov;
        }
        **vm = v;
    }
}

/// An idle terminal's standby screen in the world (desktop.rs).
#[derive(Component)]
pub struct DgStandby;

struct StandbyScreen {
    e: Entity,
    img: Handle<Image>,
    base: Vec<[f32; 4]>,
    size: (i32, i32),
    tick: i64,
}

#[derive(Default)]
pub struct StandbyRes {
    gen: u64,
    /// Terminals with a screen: actor, class, where the screen is, node name, logo texture.
    list: Option<Vec<(usize, String, Rect3, String, String)>>,
    shown: std::collections::HashMap<usize, StandbyScreen>,
    /// When this map came in.
    t0: f32,
}

/// How near (Unreal units) an idle terminal shows its standby screen, and how many at once.
const STANDBY_NEAR: f64 = 1100.0;
const STANDBY_MAX: usize = 6;

/// Idle terminals near the player run: a standby screen (the node's logo, a login box with a blinking caret, the
/// clock) on the monitor, redrawn as the caret blinks. Not the security terminals (folded shut until used) and not
/// the one in use (its live desktop is there).
#[allow(clippy::too_many_arguments)]
pub fn standby_system(
    mut commands: Commands,
    vtime: Res<Time>,
    level: Res<LevelRes>,
    cam: Single<&GlobalTransform, (With<FlyCam>, Without<SkyCam>)>,
    (mut images, mut meshes, mut materials, mut hud, mut assets): (
        ResMut<Assets<Image>>,
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
        ResMut<crate::classic_hud::ClassicHud>,
        ResMut<crate::weapons::RuntimeAssets>,
    ),
    mut res: Local<StandbyRes>,
    decos: Query<&crate::actors::Deco>,
) {
    let _prof = crate::prof::scope("diegetic::standby_system");
    let lvl = &level.0;
    let new_map = crate::loader::new_map(&mut res.gen);
    if new_map {
        res.t0 = vtime.elapsed_secs();
    }
    if new_map || !enabled() {
        for (_, sc) in res.shown.drain() {
            commands.entity(sc.e).despawn();
            images.remove(&sc.img);
        }
        if new_map {
            res.list = None;
        }
        if !enabled() {
            return;
        }
    }
    // (Measured once the map's terminals have settled onto their desks.)
    if res.list.is_none() && vtime.elapsed_secs() - res.t0 < 2.0 {
        return;
    }
    if res.list.is_none() {
        let mut list = Vec::new();
        for (i, a) in lvl.actors.iter().enumerate() {
            if !matches!(a.class.as_str(), "ComputerPersonal" | "ComputerPublic" | "ATM") {
                continue;
            }
            let (Some(def), Some(mi)) = (screen_def(&a.class), mesh_actor_of(lvl, i)) else { continue };
            let ma = &lvl.mesh_actors[mi];
            let sc = draw_scale(a);
            let def = if a.class == "ComputerPersonal" { mesh_screen(ma, sc, &def).unwrap_or(def) } else { def };
            let off = shift_of(&decos, mi);
            let rect = Rect3::new(to_world(ma, sc, def.bl) + off, to_world(ma, sc, def.br) + off, to_world(ma, sc, def.tl) + off)
                .with_tr(def.tr.map(|t| to_world(ma, sc, t) + off));
            let (name, logo) = crate::computers::node_name_logo(lvl, i, &mut assets);
            list.push((i, a.class.clone(), rect, name, logo));
        }
        res.list = Some(list);
    }
    let eye = ue(cam.translation());
    let fwd = ue(cam.translation() + *cam.forward()) - eye;
    let busy = SESSION.lock().ok().and_then(|s| s.as_ref().filter(|s| s.kind == Kind::Computer && s.phase == Phase::Active).map(|s| s.actor));
    let mut want: Vec<(f64, usize)> = Vec::new();
    for (k, (actor, _, rect, _, _)) in res.list.as_ref().unwrap().iter().enumerate() {
        if Some(*actor) == busy || crate::scriptworld::is_hidden(*actor) {
            continue;
        }
        let to = rect.c - eye;
        let d = to.length();
        if d > STANDBY_NEAR || rect.n.dot(-to) <= 0.0 || (d > 150.0 && to.dot(fwd) < -0.2 * d) {
            continue;
        }
        want.push((d, k));
    }
    want.sort_by(|a, b| a.0.total_cmp(&b.0));
    want.truncate(STANDBY_MAX);
    let keep: Vec<usize> = want.iter().map(|w| res.list.as_ref().unwrap()[w.1].0).collect();
    let gone: Vec<usize> = res.shown.keys().copied().filter(|a| !keep.contains(a)).collect();
    for a in gone {
        if let Some(sc) = res.shown.remove(&a) {
            commands.entity(sc.e).despawn();
            images.remove(&sc.img);
        }
    }
    if want.is_empty() {
        return;
    }
    let secs = vtime.elapsed_secs_f64();
    let tick = (secs * 2.0).floor() as i64;
    let fonts = (hud.font(&mut assets, "FontMenuSmall"), hud.font(&mut assets, "FontMenuHeaders"), hud.font(&mut assets, "FontMenuTitle"));
    let (Some(small), Some(headers), Some(title)) = fonts else { return };
    for (_, k) in want {
        let (actor, class, rect, name, logo_name) = res.list.as_ref().unwrap()[k].clone();
        let logo = if logo_name.is_empty() { None } else { hud.tex(&mut assets, &logo_name) };
        let draw = |base: &[[f32; 4]], size: (i32, i32)| {
            let mut c = crate::uicanvas::Canvas::new(size.0, size.1, 1);
            c.px.copy_from_slice(base);
            crate::desktop::standby_over(&mut c, &class, &small, &headers, &title, logo.as_ref(), &name, secs);
            let mut img = Image::new(
                bevy::render::render_resource::Extent3d { width: size.0 as u32, height: size.1 as u32, depth_or_array_layers: 1 },
                bevy::render::render_resource::TextureDimension::D2,
                c.to_rgba8(),
                TextureFormat::Rgba8UnormSrgb,
                bevy::asset::RenderAssetUsages::default(),
            );
            img.sampler = bevy::image::ImageSampler::linear();
            img
        };
        match res.shown.get_mut(&actor) {
            Some(sc) => {
                if sc.tick != tick {
                    sc.tick = tick;
                    let img = draw(&sc.base, sc.size);
                    if let Some(mut im) = images.get_mut(&sc.img) {
                        *im = img;
                    }
                }
            }
            None => {
                let size = crate::desktop::standby_size(rect.w / rect.h.max(1e-3));
                let base = crate::desktop::standby_base(size.0, size.1, &name).px;
                let h = images.add(draw(&base, size));
                let mat = materials.add(StandardMaterial { base_color_texture: Some(h.clone()), unlit: true, cull_mode: None, double_sided: true, ..default() });
                let lift = 0.3;
                let (mesh, tf) = match rect.quad {
                    Some(_) => (screen_mesh(&rect, lift), Transform::IDENTITY),
                    None => (quad_mesh(1.0, 1.0), quad_tf(&rect, rect.c, lift).with_scale(Vec3::new(rect.w as f32 / 52.5, rect.h as f32 / 52.5, 1.0))),
                };
                let e = commands
                    .spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(mat), tf, Visibility::Visible, DgStandby, bevy::light::NotShadowCaster, bevy::light::NotShadowReceiver))
                    .id();
                if env_on!("DXVIEW_DGLOG") {
                    println!("DG standby {} {class} at {:?} n {:?} {:.1} x {:.1} tf {:?}", lvl.actors[actor].name, rect.c, rect.n, rect.w, rect.h, tf);
                }
                res.shown.insert(actor, StandbyScreen { e, img: h, base, size, tick });
            }
        }
    }
}

pub fn reset_level() {
    if let Ok(mut s) = SESSION.lock() {
        *s = None;
    }
    if let Ok(mut v) = VIEW.lock() {
        *v = None;
    }
    LEAVE.store(false, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn screen_ray_hits_its_middle() {
        let def = screen_def("ComputerPublic").unwrap();
        let r = Rect3::new(DVec3::from(def.bl), DVec3::from(def.br), DVec3::from(def.tl));
        // The public terminal faces +X.
        assert!(r.n.x > 0.9, "{:?}", r.n);
        let (u, v) = r.hit(r.c + r.n * 50.0, -r.n).unwrap();
        assert!((u - 0.5).abs() < 1e-6 && (v - 0.5).abs() < 1e-6);
        // Left on screen is +Y (the viewer looks along -X).
        let (u, _) = r.hit(r.c + r.n * 50.0 + DVec3::Y * 5.0, -r.n).unwrap();
        assert!(u < 0.5);
    }
    #[test]
    fn keypad_grid_is_on_the_face() {
        for c in ["Keypad1", "Keypad2", "Keypad3"] {
            let (def, g) = keypad_def(c).unwrap();
            let (ymin, ymax) = (def.br[1].min(def.bl[1]), def.br[1].max(def.bl[1]));
            for y in g.cols {
                assert!(y > ymin && y < ymax, "{c} {y}");
            }
            assert!(g.rows[0] > g.rows[3], "{c}: rows go down");
        }
    }
}
