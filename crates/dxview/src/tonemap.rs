//! Tonemapping (setting Tonemapping: None / Filmic / AgX) and Eye Adaptation (setting EyeAdaptation), both done in
//! the last pass (shaders/gamma.wgsl, the Brightness material on the view-model camera).
//!
//! * None keeps the original's look: light past white clips.
//! * Filmic rolls bright light off smoothly (a fitted ACES curve, rescaled so the original's mid-tones stay where
//!   they were): glowing signs, lamps and bloom keep their colour instead of going flat white.
//! * AgX (Troy Sobotka's, in its common polynomial form) does the same with less hue shift in saturated light.
//!
//! Eye Adaptation: the eye opens slowly in the dark and closes quickly in bright light. It's measured on the CPU
//! from the level's lights (stealth::light_at, the reading the guards use) at the eye and at points ahead along the
//! view, so it costs nothing on the GPU and never reads the frame back.

use crate::{FlyCam, LevelRes, SkyCam};
use bevy::prelude::*;
use glam::DVec3;

pub fn mode() -> f32 {
    setting!("Tonemapping").round().clamp(0.0, 2.0)
}

fn adaptation_on() -> bool {
    setting!("EyeAdaptation") > 0.5
}

#[derive(Default)]
pub struct State {
    check: f32,
    /// Exposure the eye is heading for, and where it is (log2).
    want: f32,
    now: f32,
}

/// Exposure limits (log2): at most +0.75 stops in the dark (the night stays night), -0.5 in bright light.
const UP: f32 = 0.75;
const DOWN: f32 = -0.5;
/// The reading that needs no change (a lit room).
const MID: f32 = 0.32;

pub fn tonemap_system(
    time: Res<Time<Real>>,
    level: Option<Res<LevelRes>>,
    cam: Query<&Transform, (With<FlyCam>, Without<SkyCam>)>,
    mut q: Query<&mut crate::dynlight::Brightness>,
    mut st: Local<State>,
) {
    let dt = time.delta_secs().min(0.1);
    let tm = mode();
    if adaptation_on() {
        st.check -= dt;
        if st.check <= 0.0 {
            st.check = 0.2;
            if let (Some(level), Ok(tf)) = (level.as_ref(), cam.single()) {
                st.want = measure(&level.0, tf);
            }
        }
        // Opening up in the dark takes a few seconds, closing down in bright light under one.
        let rate = if st.want > st.now { 0.35 } else { 1.6 };
        let step = (st.want - st.now).clamp(-rate * dt, rate * dt);
        st.now += step;
    } else {
        st.now = 0.0;
        st.want = 0.0;
    }
    let exposure = st.now.exp2();
    for mut b in &mut q {
        if b.tm != tm || (b.exposure - exposure).abs() > 1e-4 {
            b.tm = tm;
            b.exposure = exposure;
        }
    }
}

fn measure(level: &dx_level::Level, tf: &Transform) -> f32 {
    let p = tf.translation;
    let eye = dx_level::bevy_to_ue([p.x, p.y, p.z]);
    let eye = DVec3::new(eye[0] as f64, eye[1] as f64, eye[2] as f64);
    let f = tf.forward();
    let fu = dx_level::bevy_to_ue([f.x, f.y, f.z]);
    let dir = DVec3::new(fu[0] as f64, fu[1] as f64, fu[2] as f64).normalize_or_zero();
    // What's in view: the point the view lands on (pulled back off the wall) and half way there.
    let end = eye + dir * 2000.0;
    let hit = level.collision.trace(eye, end, DVec3::ZERO);
    let far = eye + (end - eye) * (hit.fraction * 0.92);
    let mid = eye.lerp(far, 0.5);
    let r = crate::stealth::light_at(level, eye) * 0.4 + crate::stealth::light_at(level, mid) * 0.3 + crate::stealth::light_at(level, far) * 0.3;
    // Brighter than MID closes the eye down, darker opens it (log-scaled, gently: x^0.5).
    let stops = (MID / r.max(0.02)).log2() * 0.5;
    stops.clamp(DOWN, UP)
}
