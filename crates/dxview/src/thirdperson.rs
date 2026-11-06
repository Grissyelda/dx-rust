//! Enhancement (Third Person View, ROADMAP v2 #31). Ours; first person stays the default.
//!
//! With the setting on, H switches between first person and an over-the-shoulder view; J swaps the shoulder.
//! The camera sits behind and beside JC's head and is pulled in by walls (traced as a small sphere), comes
//! closer while aiming (the scope key), and JC - the body the mirrors already show, holding what you hold - is
//! drawn instead of the first-person weapon. Shots still leave from JC's eyes, aimed at what the crosshair is on.
//!
//! Third Person Camera = Free (the default): a modern orbit camera. The mouse / right stick turn the camera round JC
//! (`orbit_*`), not JC; WASD / the left stick move him relative to the camera and he turns to face the way he runs
//! (`main.rs` fly turns FlyCam.yaw - his facing - toward the wish). While aiming (scope key, sights) or shooting he
//! faces where the camera looks. Left alone for a few seconds he plays Human's Idle1 now and then (mirror.rs).
//! Shots and frobbing go from his eyes toward what the camera's centre line is on (`aim_dir`).
//! Shoulder: the view turns with him (the first version).
//!
//! Cover leaning (setting ThirdPersonCover, ours, after cover shooters): Q / E in third person swaps the camera to
//! that shoulder, turns JC to face where the camera looks, bends his body round the corner (skeletal::apply_lean,
//! or a tilt of the vertex body) and takes the camera - and the shots, which leave from the peek point - out past
//! the edge: up to PEEK_REACH units to the side in all (less what the body itself slid, DeusExPlayer's 40-unit
//! lean), or, against a wall on that side, first forward along it to where the way out is clear (a corner ahead).
//! Crouched, the camera drops with him and stays just over his head.
//!
//! The game logic keeps reading the camera's transform as JC's eyes: the camera is moved back only for drawing
//! (`place_system`, in PostUpdate before transforms propagate) and put back at the eyes at the start of the
//! next frame (`restore_system`, in First).

use crate::{FlyCam, LevelRes, SkyCam};
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use dx_level::{bevy_to_ue, ue_to_bevy};
use glam::DVec3;
use imsim_core::movement::CollisionWorld;
use std::sync::Mutex;

pub fn enabled() -> bool {
    setting!("ThirdPerson") > 0.5
}

#[derive(Clone, Copy, Default)]
struct State {
    active: bool,
    /// -1 left shoulder, 1 right; eased.
    side: f32,
    side_want: f32,
    /// Camera distance (units), eased toward the wanted one.
    dist: f32,
    /// The eye transform to put back.
    eye: Option<Transform>,
    /// Where the camera was drawn this frame (Unreal units).
    cam: Option<DVec3>,
    /// Extra height over the eye while crouched (units), eased: the crouched eye is down at the hips.
    lift: f32,
    /// Free camera: the orbit's yaw / pitch (FlyCam's convention) and whether it has been set up.
    orbit_yaw: f32,
    orbit_pitch: f32,
    orbit_on: bool,
    /// The drawn camera's forward (Unreal space).
    cam_fw: Option<DVec3>,
    /// Seconds without any input (idle animations).
    idle: f32,
    /// Seconds left facing the camera after a shot.
    aim_hold: f32,
    /// loader::generation() the orbit was set up for (a new map starts it behind JC again).
    gen: u64,
    /// Cover leaning: the lean input (-1 left .. 1 right), eased; Q / E held now.
    lean_dir: f32,
    leaning: bool,
    /// The peek past the edge (units, eased): forward along the wall, then out to the side (signed: + right).
    peek_fwd: f64,
    peek_side: f64,
    /// Where the shots leave from while peeking (Unreal), and the extra body bend toward it (units).
    shot_from: Option<DVec3>,
    bend: f32,
}

static STATE: Mutex<State> = Mutex::new(State {
    active: false,
    side: 1.0,
    side_want: 1.0,
    dist: 0.0,
    eye: None,
    cam: None,
    lift: 0.0,
    orbit_yaw: 0.0,
    orbit_pitch: 0.0,
    orbit_on: false,
    cam_fw: None,
    idle: 0.0,
    aim_hold: 0.0,
    gen: 0,
    lean_dir: 0.0,
    leaning: false,
    peek_fwd: 0.0,
    peek_side: 0.0,
    shot_from: None,
    bend: 0.0,
});

/// Cover leaning in third person (see the module notes).
pub fn cover_on() -> bool {
    setting!("ThirdPersonCover") > 0.5
}

/// How far out to the side (units) a cover lean takes the camera and the shots in all, counting the body's own slide.
const PEEK_REACH: f64 = 44.0;
/// How far forward along a wall at that side the peek looks for the corner (units).
const PEEK_AHEAD: [f64; 4] = [12.0, 24.0, 36.0, 48.0];

/// The lean the rebuilt / vertex body shows: the body's slide (`lean`, Player::lean) plus, leaning from cover in third
/// person, the peek's bend toward the edge.
pub fn body_lean(lean: f32) -> f32 {
    if !active() {
        return lean;
    }
    lean + STATE.lock().map(|s| s.bend).unwrap_or(0.0)
}

/// Where a shot leaves from: the eyes, or leaning from cover in third person the peek point past the edge.
pub fn shot_origin(eye: DVec3) -> DVec3 {
    if !active() {
        return eye;
    }
    STATE.lock().ok().and_then(|s| s.shot_from).unwrap_or(eye)
}

/// The two legs of a peek from `from`: forward along `fwh`, then out along `rth` (signed), each traced (a sphere of
/// `r`, the level and movers - a closed door is a wall) and stopped short of what it meets.
fn peek_point(world: &crate::actors::World, from: DVec3, fwh: DVec3, rth: DVec3, fwd: f64, side: f64, r: f64) -> DVec3 {
    let leg = |a: DVec3, d: DVec3| {
        let len = d.length();
        if len < 0.01 {
            return a;
        }
        let h = world.trace(a, a + d, DVec3::splat(r));
        a + d * ((h.fraction.max(0.0) * len - 1.0).max(0.0) / len)
    };
    let q = leg(from, fwh * fwd);
    leg(q, rth * side)
}

/// The free orbit camera is in use (third person active with Third Person Camera = Free).
pub fn free() -> bool {
    active() && setting!("ThirdPersonCamera") < 0.5
}

/// Turn the orbit camera (radians, FlyCam's convention: yaw left positive, pitch up positive).
pub fn orbit_look(dyaw: f32, dpitch: f32) {
    if let Ok(mut s) = STATE.lock() {
        s.orbit_yaw -= dyaw;
        s.orbit_pitch = (s.orbit_pitch + dpitch).clamp(-1.3, 1.2);
    }
}

/// The orbit camera's yaw / pitch.
pub fn orbit() -> (f32, f32) {
    STATE.lock().map(|s| (s.orbit_yaw, s.orbit_pitch)).unwrap_or((0.0, 0.0))
}

/// Called by fly every frame with JC's view angles: the orbit starts behind him when the free camera comes on, and
/// when it goes off (back to first person) the view takes the camera's angles (returned).
pub fn sync(yaw: f32, pitch: f32) -> Option<(f32, f32)> {
    let on = free();
    let gen = crate::loader::generation();
    let Ok(mut s) = STATE.lock() else { return None };
    if on && (!s.orbit_on || s.gen != gen) {
        s.orbit_on = true;
        s.gen = gen;
        s.orbit_yaw = yaw;
        s.orbit_pitch = pitch.clamp(-1.3, 1.2);
        s.idle = 0.0;
        if env_on!("DXVIEW_BODYLOG") {
            println!("ORBIT start yaw {:.2} pitch {:.2} gen {gen}", yaw, pitch);
        }
        // DXVIEW_ORBIT="yaw_deg,pitch_deg": (test shots) the orbit turned that far from behind JC.
        if let Some((a, b)) = env_val!("DXVIEW_ORBIT").map(str::to_owned).and_then(|v| {
            let (a, b) = v.split_once(',')?;
            Some((a.trim().parse::<f32>().ok()?, b.trim().parse::<f32>().ok()?))
        }) {
            s.orbit_yaw += a.to_radians();
            s.orbit_pitch = (s.orbit_pitch + b.to_radians()).clamp(-1.3, 1.2);
        }
        None
    } else if !on && s.orbit_on {
        s.orbit_on = false;
        // Back to first person by H: look where the camera looked (not after conversations and cut scenes,
        // which hand the view back themselves).
        (enabled() && !s.active).then_some((s.orbit_yaw, s.orbit_pitch.clamp(-1.3, 1.2)))
    } else {
        None
    }
}

/// Input this frame (any key, button, mouse or stick): the idle timer starts over.
pub fn note_input(any: bool, dt: f32) {
    if let Ok(mut s) = STATE.lock() {
        s.idle = if any { 0.0 } else { s.idle + dt };
    }
}

/// Seconds since the last input (0 outside the free camera).
pub fn idle_secs() -> f32 {
    if !active() {
        return 0.0;
    }
    STATE.lock().map(|s| s.idle).unwrap_or(0.0)
}

/// Whether JC faces the camera: aiming now, or within a second of the last shot.
pub fn facing_camera(aiming: bool, fired: bool, dt: f32) -> bool {
    let Ok(mut s) = STATE.lock() else { return aiming };
    if fired {
        s.aim_hold = 1.2;
    } else {
        s.aim_hold = (s.aim_hold - dt).max(0.0);
    }
    // Leaning from cover he faces the camera too (the lean goes the camera's way).
    aiming || s.aim_hold > 0.0 || s.leaning
}

/// Third person right now (the setting on and switched to it, nothing else holding the camera).
pub fn active() -> bool {
    enabled()
        && STATE.lock().map(|s| s.active).unwrap_or(false)
        && !crate::consys::third_person()
        && !crate::cinematic::active()
}

/// The direction a shot from `eye` takes to land where the crosshair (on the drawn camera's centre line) points.
pub fn aim_dir(level: &dx_level::Level, eye: DVec3, fw: DVec3) -> DVec3 {
    if !active() {
        return fw;
    }
    let Some((cam, cfw)) = STATE.lock().ok().and_then(|s| Some((s.cam?, s.cam_fw.unwrap_or(fw)))) else { return fw };
    let h = level.collision.trace(cam, cam + cfw * 8000.0, DVec3::ZERO);
    let target = cam + cfw * 8000.0 * h.fraction.max(0.0);
    // (Shots pass shot_origin(eye) as `eye`: leaning from cover they leave from the peek point.)
    let from = eye;
    // Something between the camera and JC (he's in the way of the crosshair): just shoot straight.
    if (target - from).dot(fw) < 32.0 {
        return fw;
    }
    (target - from).normalize_or(fw)
}

/// The view's render layers: the world, mirror faces, shadow lights when on, and JC's body in third person.
pub fn camera_layers() -> RenderLayers {
    let mut l = vec![0, crate::mirror::LAYER, crate::weather::RAIN_LAYER, crate::skyfollow::LAYER];
    if crate::shadowlight::level() > 0 {
        l.extend_from_slice(&crate::shadowlight::SLOT_LAYERS);
    }
    // (A third-person conversation shows JC's rebuilt body too, instead of a vertex puppet with another face.)
    if active() || (crate::consys::third_person() && crate::skeletal::enabled() && crate::skeletal::body_for(&crate::travel::jc_mesh()).is_some()) {
        l.push(crate::mirror::BODY_LAYER);
    } else if crate::bodyfx::fp_on() {
        l.push(crate::bodyfx::FP_LAYER);
    }
    RenderLayers::from_layers(&l)
}

/// Keys and the view's layers (and hiding the first-person weapon).
pub fn toggle_system(
    keys: Res<ButtonInput<KeyCode>>,
    mut cam: Query<&mut RenderLayers, (With<FlyCam>, Without<SkyCam>)>,
    mut vm: Query<&mut RenderLayers, (With<crate::weapons::ViewModelCam>, Without<FlyCam>)>,
) {
    if let Ok(mut st) = STATE.lock() {
        let free = !crate::inventory_screen::screen_open() && !crate::menus::open();
        if enabled() && free {
            if keys.just_pressed(KeyCode::KeyH) {
                st.active = !st.active;
            }
            if keys.just_pressed(KeyCode::KeyJ) {
                st.side_want = -st.side_want;
            }
        }
        if !enabled() {
            st.active = false;
        } else if env_on!("DXVIEW_THIRD") && st.eye.is_none() && st.dist == 0.0 {
            // DXVIEW_THIRD=1: start in third person (screenshots).
            st.active = true;
        }
    }
    let want = camera_layers();
    for mut l in &mut cam {
        if *l != want {
            *l = want.clone();
        }
    }
    // The first-person weapon isn't drawn from behind; JC holds it. (That camera also draws the HUD and the
    // brightness pass, so it stays on and only stops seeing the weapon layer.)
    let vm_want = if active() { RenderLayers::layer(30) } else { RenderLayers::layer(crate::weapons::VIEW_LAYER) };
    for mut l in &mut vm {
        if *l != vm_want {
            *l = vm_want.clone();
        }
    }
}

/// First: the camera back at JC's eyes for this frame's logic.
pub fn restore_system(mut cam: Single<&mut Transform, (With<FlyCam>, Without<SkyCam>)>) {
    if let Ok(mut st) = STATE.lock() {
        if let Some(t) = st.eye.take() {
            **cam = t;
        }
    }
}

/// PostUpdate, before transforms propagate: the camera moved back over the shoulder for drawing.
pub fn place_system(
    time: Res<Time>,
    level: Res<LevelRes>,
    keys: Res<ButtonInput<KeyCode>>,
    player: Res<crate::Player>,
    movers: Option<Res<crate::movers::Movers>>,
    mut cam: Single<&mut Transform, (With<FlyCam>, Without<SkyCam>)>,
) {
    let free_cam = free();
    let Ok(mut st) = STATE.lock() else { return };
    st.cam = None;
    st.cam_fw = None;
    let on = enabled() && st.active && !crate::consys::third_person() && !crate::cinematic::active();
    let dt = time.delta_secs().min(0.1);
    if !on {
        st.dist = 0.0;
        st.lean_dir = 0.0;
        st.leaning = false;
        st.peek_fwd = 0.0;
        st.peek_side = 0.0;
        st.shot_from = None;
        st.bend = 0.0;
        return;
    }
    // Cover leaning: Q / E (the pad's bumpers press them too), standing or crouched and nearly still, on the ground.
    let cover = cover_on();
    let speed = player.body.velocity.truncate().length();
    let lean_in = if cover && !player.swimming && player.body.physics == imsim_core::movement::Physics::Walking && speed < 60.0 {
        (keys.pressed(KeyCode::KeyE) as i32 - keys.pressed(KeyCode::KeyQ) as i32) as f32
    } else {
        0.0
    };
    st.leaning = lean_in != 0.0;
    st.lean_dir += (lean_in - st.lean_dir) * (1.0 - (-8.0 * dt).exp());
    if st.lean_dir.abs() < 0.01 && lean_in == 0.0 {
        st.lean_dir = 0.0;
    }
    // Leaning swaps the camera to that shoulder for as long as the lean lasts.
    let side_to = if lean_in != 0.0 { lean_in } else { st.side_want };
    st.side += (side_to - st.side).clamp(-dt * 4.0, dt * 4.0);
    let aiming = keys.pressed(KeyCode::KeyZ) || keys.pressed(KeyCode::BracketLeft);
    let eye_t = **cam;
    let conv = |v: Vec3| DVec3::new(v.x as f64, v.z as f64, v.y as f64);
    // The free camera looks along its own orbit; the shoulder camera along JC's view.
    let rot = if free_cam {
        Quat::from_rotation_y(st.orbit_yaw) * Quat::from_rotation_x(st.orbit_pitch)
    } else if cover {
        // (Cover leaning: the view's lean roll is for first person; behind him the camera stays level.)
        let f = eye_t.rotation * Vec3::NEG_Z;
        Transform::IDENTITY.looking_to(f, Vec3::Y).rotation
    } else {
        eye_t.rotation
    };
    let fw = conv(rot * Vec3::NEG_Z);
    // Aiming pulls the camera in over the shoulder - less so looking down, where backing off along the view lifts it
    // straight over his head and he filled the screen.
    let down = (-fw.z).max(0.0) as f32;
    let want = match (free_cam, aiming) {
        (_, true) => 55.0 + 60.0 * down,
        (true, false) => 135.0,
        (false, false) => 105.0,
    };
    st.dist = if st.dist <= 0.0 { want } else { st.dist + (want - st.dist) * (dt * 8.0).min(1.0) };
    let rt = conv(rot * Vec3::X);
    let e = bevy_to_ue(eye_t.translation.to_array());
    let eye = DVec3::new(e[0] as f64, e[1] as f64, e[2] as f64);
    // Over the shoulder: back, to the side and a little up; walls pull it in (a sphere of 8 units).
    // (Crouched the eye drops to the hips: the camera stays a little over his head - lower than standing.)
    let lift_want = if player.crouched { 24.0 } else { 0.0 };
    st.lift += (lift_want - st.lift) * (dt * 6.0).min(1.0);
    // The free camera sits nearer the middle (a little to the side, so JC doesn't hide the crosshair) and higher.
    let (side_off, up_off) = if free_cam { (if aiming { 22.0 } else { 14.0 }, 10.0) } else { (22.0, 6.0) };
    let pivot = eye + rt * (st.side as f64 * side_off) + DVec3::new(0.0, 0.0, up_off + st.lift as f64);
    let side = level.0.collision.trace(eye, pivot, DVec3::splat(8.0));
    let mut pivot = eye + (pivot - eye) * side.fraction.max(0.0);
    // Cover leaning: out past the edge (see the module notes).
    let fwh = DVec3::new(fw.x, fw.y, 0.0).normalize_or_zero();
    let rth = DVec3::new(rt.x, rt.y, 0.0).normalize_or_zero();
    let (mut want_fwd, mut want_side) = (0.0f64, 0.0f64);
    let amt = st.lean_dir.abs() as f64;
    if amt > 0.01 && rth != DVec3::ZERO {
        let s = st.lean_dir.signum() as f64;
        // What the body's own slide (Player::lean, along the view's right) hasn't covered yet.
        let reach = (PEEK_REACH - player.lean.abs()).max(4.0);
        let world = crate::actors::World { level: &level.0, blockers: &[], ignore: None, pushable: None, movers: movers.as_ref().map(|m| m.rt.as_slice()) };
        let out = world.trace(pivot, pivot + rth * s * reach, DVec3::splat(8.0)).fraction.max(0.0);
        want_side = s * reach * out;
        if out < 0.6 {
            // A wall on that side: the corner is ahead. Along the wall to where the way out to the side is clear.
            for a in PEEK_AHEAD {
                let q = pivot + fwh * a;
                if world.trace(pivot, q, DVec3::splat(8.0)).fraction < 1.0 {
                    break;
                }
                let t = world.trace(q, q + rth * s * reach, DVec3::splat(8.0)).fraction;
                if t > 0.9 {
                    want_fwd = a;
                    want_side = s * reach * t;
                    break;
                }
            }
        }
        want_fwd *= amt;
        want_side *= amt;
    }
    let k = 1.0 - (-10.0 * dt as f64).exp();
    st.peek_fwd += (want_fwd - st.peek_fwd) * k;
    st.peek_side += (want_side - st.peek_side) * k;
    if st.peek_fwd.abs() + st.peek_side.abs() > 0.5 {
        let world = crate::actors::World { level: &level.0, blockers: &[], ignore: None, pushable: None, movers: movers.as_ref().map(|m| m.rt.as_slice()) };
        pivot = peek_point(&world, pivot, fwh, rth, st.peek_fwd, st.peek_side, 8.0);
        // The shots lean out with him: the same legs from his eyes (a thinner trace).
        st.shot_from = Some(peek_point(&world, eye, fwh, rth, st.peek_fwd, st.peek_side, 4.0));
        st.bend = st.peek_side as f32;
    } else {
        st.peek_fwd = 0.0;
        st.peek_side = 0.0;
        st.shot_from = None;
        st.bend = 0.0;
    }
    // DXVIEW_COVERLOG: the lean and the peek twice a second.
    if env_on!("DXVIEW_COVERLOG") {
        static T: Mutex<f32> = Mutex::new(0.0);
        if let Ok(mut t) = T.lock() {
            *t += dt;
            if *t > 0.5 {
                *t = 0.0;
                println!(
                    "COVER input {lean_in} lean_dir {:.2} body lean {:.1} peek fwd {:.1} side {:.1} shots from {:?}",
                    st.lean_dir, player.lean, st.peek_fwd, st.peek_side, st.shot_from.map(|p| (p - eye).length().round())
                );
            }
        }
    }
    let back = pivot - fw * st.dist as f64;
    let h = level.0.collision.trace(pivot, back, DVec3::splat(8.0));
    let at = pivot + (back - pivot) * (h.fraction.max(0.0) * 0.98);
    st.eye = Some(eye_t);
    st.cam = Some(at);
    st.cam_fw = Some(fw);
    cam.translation = Vec3::from(ue_to_bevy(at.as_vec3().to_array()));
    if free_cam || cover {
        cam.rotation = rot;
    }
}
