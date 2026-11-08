//! Conversation Key Light (setting ConversationLight, Modern preset): a third-person conversation held somewhere
//! dark gets a soft light on the people in it, as a film crew would set one, so faces read. It lights the people
//! only (meshes: dynlight's CPU light list), never the walls, and is marked a glow (LightInfo::emitter) so stealth,
//! halos and shadows ignore it. It fades in with the darkness of the spot (measured like the player's visibility,
//! without itself) and out when the conversation ends; it sits on the camera's side of the two speakers, above and
//! a little to the side (a three-quarter key), and follows the camera's cuts smoothly.

use bevy::math::DVec3;
use bevy::prelude::*;

use crate::{FlyCam, LevelRes, SkyCam};

/// Brightness (0..1, the Unreal byte / 255) at full darkness.
const MAX_BRIGHT: f32 = 0.38;
/// The visibility reading (stealth::light_at) at and below which the light is fully on, and above which it is off.
const DARK: f32 = 0.12;
const BRIGHT: f32 = 0.4;
/// Warm white: LightHue / LightSaturation (255 = white).
const HUE: u8 = 28;
const SAT: u8 = 215;

#[derive(Default)]
pub struct State {
    /// 0..1 how much of the light is on (eased).
    level: f32,
    /// The darkness reading's wanted level (0..1), refreshed every 0.25 s.
    want: f32,
    check: f32,
    pos: Option<DVec3>,
}

pub fn enabled() -> bool {
    setting!("ConversationLight") > 0.5
}

pub fn con_light_system(
    time: Res<Time<Real>>,
    level: Res<LevelRes>,
    cam: Query<&Transform, (With<FlyCam>, Without<SkyCam>)>,
    mut st: Local<State>,
) {
    let dt = time.delta_secs().min(0.1);
    let subjects = if enabled() { crate::consys::shot_subjects() } else { None };
    let Some((a, b)) = subjects else {
        // Fade out (the last spot kept), then off.
        if st.level > 0.0 {
            st.level = (st.level - dt * 2.0).max(0.0);
            if let Some(p) = st.pos {
                crate::dynlight::set_fill(Some(light(p, st.level, 400.0)));
            }
        }
        if st.level <= 0.0 {
            if st.pos.take().is_some() {
                crate::dynlight::set_fill(None);
            }
            st.want = 0.0;
            st.check = 0.0;
        }
        return;
    };
    let lvl = &level.0;
    // How dark it is where they stand: the player's visibility reading at their faces (our light doesn't count -
    // it's a glow), darkest of the two.
    st.check -= dt;
    if st.check <= 0.0 {
        st.check = 0.25;
        let la = crate::stealth::light_at(lvl, a);
        let lb = b.map_or(la, |b| crate::stealth::light_at(lvl, b));
        let lit = la.min(lb);
        // Full below 0.12 (the Liberty Island dock at night reads about 0.2), none above 0.4 (a lit room).
        let t = ((lit - DARK) / (BRIGHT - DARK)).clamp(0.0, 1.0);
        st.want = 1.0 - t * t * (3.0 - 2.0 * t);
        if env_on!("DXVIEW_CONLIGHT_FORCE") {
            st.want = 1.0;
        }
        if env_on!("DXVIEW_CONLIGHTLOG") {
            println!("CONLIGHT lit {la:.3} / {lb:.3} -> want {:.2}, level {:.2}, at {:?}", st.want, st.level, st.pos);
        }
    }
    let rate = if st.want > st.level { 1.5 } else { 1.0 };
    st.level += (st.want - st.level).clamp(-rate * dt, rate * dt);
    let Some(cam) = cam.iter().next() else { return };
    let c = cam_ue(cam.translation);
    let mid = b.map_or(a, |b| (a + b) * 0.5);
    // Toward the camera, flattened, then up and to the camera's right.
    let mut v = c - mid;
    v.z = 0.0;
    let v = v.try_normalize().unwrap_or(DVec3::X);
    let right = v.cross(DVec3::Z);
    let reach = (c - mid).length().clamp(60.0, 140.0);
    let want = mid + v * reach + DVec3::Z * 48.0 + right * 45.0;
    // Never through a wall: stop short of whatever is between them and the spot.
    let h = lvl.collision.trace(mid, want, DVec3::ZERO);
    let want = if h.fraction < 1.0 { mid + (want - mid) * (h.fraction * 0.9).max(0.2) } else { want };
    let pos = match st.pos {
        Some(p) => p + (want - p) * (1.0 - (-dt as f64 * 5.0).exp()),
        None => want,
    };
    st.pos = Some(pos);
    let radius = pos.distance(a).max(b.map_or(0.0, |b| pos.distance(b))) as f32 + 160.0;
    if st.level > 0.002 {
        crate::dynlight::set_fill(Some(light(pos, st.level, radius)));
    } else {
        crate::dynlight::set_fill(None);
    }
}

fn light(pos: DVec3, level: f32, radius: f32) -> dx_level::lighting::LightInfo {
    let c = dx_level::lighting::hsv_white(HUE, SAT);
    let b = MAX_BRIGHT * level;
    let mut l = dx_level::lighting::LightInfo::omni(pos.as_vec3().to_array(), [c[0] * b, c[1] * b, c[2] * b], radius);
    l.emitter = true;
    l
}

fn cam_ue(t: Vec3) -> DVec3 {
    let u = dx_level::bevy_to_ue(t.to_array());
    DVec3::new(u[0] as f64, u[1] as f64, u[2] as f64)
}
