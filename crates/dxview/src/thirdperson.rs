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
//! The game logic keeps reading the camera's transform as JC's eyes: the camera is moved back only for drawing
//! (`place_system`, in PostUpdate before transforms propagate) and put back at the eyes at the start of the
//! next frame (`restore_system`, in First).

use crate::{FlyCam, LevelRes, SkyCam};
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use dx_level::{bevy_to_ue, ue_to_bevy};
use glam::DVec3;
use std::sync::Mutex;

pub fn enabled() -> bool {
    crate::settings::get("ThirdPerson") > 0.5
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
});

/// The free orbit camera is in use (third person active with Third Person Camera = Free).
pub fn free() -> bool {
    active() && crate::settings::get("ThirdPersonCamera") < 0.5
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
        if std::env::var_os("DXVIEW_BODYLOG").is_some() {
            println!("ORBIT start yaw {:.2} pitch {:.2} gen {gen}", yaw, pitch);
        }
        // DXVIEW_ORBIT="yaw_deg,pitch_deg": (test shots) the orbit turned that far from behind JC.
        if let Some((a, b)) = std::env::var("DXVIEW_ORBIT").ok().and_then(|v| {
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
    aiming || s.aim_hold > 0.0
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
    // Something between the camera and JC (he's in the way of the crosshair): just shoot straight.
    if (target - eye).dot(fw) < 32.0 {
        return fw;
    }
    (target - eye).normalize_or(fw)
}

/// The view's render layers: the world, mirror faces, shadow lights when on, and JC's body in third person.
pub fn camera_layers() -> RenderLayers {
    let mut l = vec![0, crate::mirror::LAYER, crate::weather::RAIN_LAYER];
    if crate::shadowlight::level() > 0 {
        l.push(crate::shadowlight::SHADOW_LAYER);
    }
    if active() {
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
        } else if std::env::var_os("DXVIEW_THIRD").is_some() && st.eye.is_none() && st.dist == 0.0 {
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
        return;
    }
    st.side += (st.side_want - st.side).clamp(-dt * 4.0, dt * 4.0);
    let aiming = keys.pressed(KeyCode::KeyZ) || keys.pressed(KeyCode::BracketLeft);
    let want = match (free_cam, aiming) {
        (_, true) => 55.0,
        (true, false) => 135.0,
        (false, false) => 105.0,
    };
    st.dist = if st.dist <= 0.0 { want } else { st.dist + (want - st.dist) * (dt * 8.0).min(1.0) };
    let eye_t = **cam;
    let conv = |v: Vec3| DVec3::new(v.x as f64, v.z as f64, v.y as f64);
    // The free camera looks along its own orbit; the shoulder camera along JC's view.
    let rot = if free_cam {
        Quat::from_rotation_y(st.orbit_yaw) * Quat::from_rotation_x(st.orbit_pitch)
    } else {
        eye_t.rotation
    };
    let fw = conv(rot * Vec3::NEG_Z);
    let rt = conv(rot * Vec3::X);
    let e = bevy_to_ue(eye_t.translation.to_array());
    let eye = DVec3::new(e[0] as f64, e[1] as f64, e[2] as f64);
    // Over the shoulder: back, to the side and a little up; walls pull it in (a sphere of 8 units).
    let lift_want = if player.crouched { 24.0 } else { 0.0 };
    st.lift += (lift_want - st.lift) * (dt * 6.0).min(1.0);
    // The free camera sits nearer the middle (a little to the side, so JC doesn't hide the crosshair) and higher.
    let (side_off, up_off) = if free_cam { (if aiming { 22.0 } else { 14.0 }, 10.0) } else { (22.0, 6.0) };
    let pivot = eye + rt * (st.side as f64 * side_off) + DVec3::new(0.0, 0.0, up_off + st.lift as f64);
    let side = level.0.collision.trace(eye, pivot, DVec3::splat(8.0));
    let pivot = eye + (pivot - eye) * side.fraction.max(0.0);
    let back = pivot - fw * st.dist as f64;
    let h = level.0.collision.trace(pivot, back, DVec3::splat(8.0));
    let at = pivot + (back - pivot) * (h.fraction.max(0.0) * 0.98);
    st.eye = Some(eye_t);
    st.cam = Some(at);
    st.cam_fw = Some(fw);
    cam.translation = Vec3::from(ue_to_bevy(at.as_vec3().to_array()));
    if free_cam {
        cam.rotation = rot;
    }
}
