//! Enhancement (Third Person View, ROADMAP v2 #31). Ours; first person stays the default.
//!
//! With the setting on, H switches between first person and an over-the-shoulder view; J swaps the shoulder.
//! The camera sits behind and beside JC's head and is pulled in by walls (traced as a small sphere), comes
//! closer while aiming (the scope key), and JC - the body the mirrors already show, holding what you hold - is
//! drawn instead of the first-person weapon. Shots still leave from JC's eyes, aimed at what the crosshair is on.
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
}

static STATE: Mutex<State> = Mutex::new(State {
    active: false,
    side: 1.0,
    side_want: 1.0,
    dist: 0.0,
    eye: None,
    cam: None,
});

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
    let Some(cam) = STATE.lock().ok().and_then(|s| s.cam) else { return fw };
    let h = level.collision.trace(cam, cam + fw * 8000.0, DVec3::ZERO);
    let target = cam + fw * 8000.0 * h.fraction.max(0.0);
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
    mut cam: Single<&mut Transform, (With<FlyCam>, Without<SkyCam>)>,
) {
    let Ok(mut st) = STATE.lock() else { return };
    st.cam = None;
    let on = enabled() && st.active && !crate::consys::third_person() && !crate::cinematic::active();
    let dt = time.delta_secs().min(0.1);
    if !on {
        st.dist = 0.0;
        return;
    }
    st.side += (st.side_want - st.side).clamp(-dt * 4.0, dt * 4.0);
    let aiming = keys.pressed(KeyCode::KeyZ) || keys.pressed(KeyCode::BracketLeft);
    let want = if aiming { 55.0 } else { 105.0 };
    st.dist = if st.dist <= 0.0 { want } else { st.dist + (want - st.dist) * (dt * 8.0).min(1.0) };
    let eye_t = **cam;
    let conv = |v: Vec3| DVec3::new(v.x as f64, v.z as f64, v.y as f64);
    let fw = conv(eye_t.forward().as_vec3());
    let rt = conv(eye_t.right().as_vec3());
    let e = bevy_to_ue(eye_t.translation.to_array());
    let eye = DVec3::new(e[0] as f64, e[1] as f64, e[2] as f64);
    // Over the shoulder: back, to the side and a little up; walls pull it in (a sphere of 8 units).
    let pivot = eye + rt * (st.side as f64 * 22.0) + DVec3::new(0.0, 0.0, 6.0);
    let side = level.0.collision.trace(eye, pivot, DVec3::splat(8.0));
    let pivot = eye + (pivot - eye) * side.fraction.max(0.0);
    let back = pivot - fw * st.dist as f64;
    let h = level.0.collision.trace(pivot, back, DVec3::splat(8.0));
    let at = pivot + (back - pivot) * (h.fraction.max(0.0) * 0.98);
    st.eye = Some(eye_t);
    st.cam = Some(at);
    cam.translation = Vec3::from(ue_to_bevy(at.as_vec3().to_array()));
}
