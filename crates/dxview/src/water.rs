//! Water surfaces. The original's water is a WetTexture: its source picture displaced every frame by a little
//! wave simulation (WaterTexture drops), drawn translucent on the zone portal. Here the source picture is drawn
//! by a shader that displaces it with an analytic swell (a few travelling sine waves) plus expanding rings
//! where something hit the water (splashes, the player wading or diving), shades it with the perturbed normal
//! (a sky glint, more opaque at grazing angles) - and floating things bob on the same swell (`height`), so what
//! floats moves with the surface you see. Deviation from the original: the swell is ours, not WaterTexture's
//! drop simulation, and there was no bobbing (DeusExDecoration only rocked floating things in place).

use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use glam::DVec3;
use std::sync::Mutex;

pub type WaterMaterial = ExtendedMaterial<StandardMaterial, WaterExt>;

pub const WATER_SHADER: Handle<Shader> = bevy::asset::uuid_handle!("8e2f4a17-5b3c-4d9e-a061-7c2b9f3e5d48");

/// Ripple rings at once (oldest replaced).
pub const RINGS: usize = 8;

#[derive(Clone, Copy, Debug, Default, ShaderType, Reflect)]
pub struct WaterParams {
    /// x: time (s), y: UV displacement strength, z: brightness of the (added) lit texture, w: glint strength.
    pub t: Vec4,
    /// x, y: distance (m) the water starts fading out and is gone.
    pub far: Vec4,
    /// Rings: Bevy x, z (meters), start time, strength (0 = unused).
    pub rings: [Vec4; RINGS],
    /// Enhancement "Modern Water": x 1 = on, y 1 = the reflection image holds this plane's reflection, z rain on the
    /// surface 0..1, w how strongly the waves bend the reflection.
    pub m: Vec4,
    /// Modern water's deep colour (rgb) and how see-through it is straight down (a).
    pub tint: Vec4,
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct WaterExt {
    #[uniform(100)]
    pub params: WaterParams,
    /// The planar reflection of this water's plane (mirror.rs slots), when Modern Water is on.
    #[texture(101)]
    #[sampler(102)]
    pub reflection: Handle<Image>,
}

impl MaterialExtension for WaterExt {
    fn fragment_shader() -> ShaderRef {
        WATER_SHADER.into()
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
}

pub fn setup_water_shader(mut shaders: ResMut<Assets<Shader>>) {
    let _ = shaders.insert(
        &WATER_SHADER,
        Shader::from_wgsl(include_str!("shaders/water.wgsl"), "dxview/water.wgsl"),
    );
}

/// The swell: (direction angle, wavelength m, speed m/s, amplitude). Shared with the shader (keep in step).
const WAVES: [(f32, f32, f32, f32); 5] = [
    (0.35, 6.0, 1.1, 0.50),
    (1.95, 3.3, 0.85, 0.30),
    (4.10, 1.8, 0.65, 0.18),
    (2.75, 0.95, 0.45, 0.10),
    (5.30, 0.55, 0.38, 0.06),
];

/// Surface height (-1..1-ish) and its gradient (per meter) at a Bevy-space x, z, at time t: the same field the
/// shader draws (rings left out).
pub fn swell(x: f32, z: f32, t: f32) -> (f32, f32, f32) {
    let (mut h, mut gx, mut gz) = (0.0, 0.0, 0.0);
    for (ang, len, speed, amp) in WAVES {
        let (dx, dz) = (ang.cos(), ang.sin());
        let k = std::f32::consts::TAU / len;
        let ph = (dx * x + dz * z) * k - t * speed * k;
        h += amp * ph.sin();
        gx += amp * k * dx * ph.cos();
        gz += amp * k * dz * ph.cos();
    }
    (h, gx, gz)
}

/// How far a floating thing rises and falls on the swell, Unreal units.
const BOB: f64 = 3.0;

/// Vertical offset (Unreal units) and tilt (pitch, roll in Unreal rotator units) for something floating at an
/// Unreal-space spot.
pub fn bob(at: DVec3, t: f64) -> (f64, i32, i32) {
    let p = dx_level::ue_to_bevy([at.x as f32, at.y as f32, at.z as f32]);
    let (h, gx, gz) = swell(p[0], p[2], t as f32);
    // Slope in Unreal X (Bevy x) and Unreal Y (Bevy z), scaled down to a gentle tilt.
    let k = 600.0;
    (h as f64 * BOB, (gx * k) as i32, (gz * k) as i32)
}

static RINGS_Q: Mutex<Vec<(DVec3, f32)>> = Mutex::new(Vec::new());

/// Something hit the water at an Unreal-space spot: a ring spreads from it (strength ~0.5 wading .. 1.5 a dive).
pub fn ripple(at: DVec3, strength: f32) {
    if let Ok(mut q) = RINGS_Q.lock() {
        q.push((at, strength));
    }
}

#[derive(Resource, Default)]
pub struct WaterMats {
    pub mats: Vec<Handle<WaterMaterial>>,
    /// Drawn solid in the original (a portal sheet without PF_Translucent), by material.
    pub opaque: Vec<bool>,
    rings: [Vec4; RINGS],
    next: usize,
    modern: Option<bool>,
}

/// Enhancement "Modern Water" (setting ModernWater): reflections, a body colour and see-through depth, detailed
/// waves, rain rings - a shader-pack look instead of the original's translucent picture.
pub fn modern() -> bool {
    crate::settings::get("ModernWater") > 0.5
}

/// Each frame: time and the live rings into every water material.
pub fn water_system(time: Res<Time>, mut wm: ResMut<WaterMats>, mut mats: ResMut<Assets<WaterMaterial>>) {
    let t = time.elapsed_secs();
    let new: Vec<(DVec3, f32)> = RINGS_Q.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    for (at, s) in new {
        let p = dx_level::ue_to_bevy([at.x as f32, at.y as f32, at.z as f32]);
        let i = wm.next % RINGS;
        wm.rings[i] = Vec4::new(p[0], p[2], t, s);
        wm.next += 1;
    }
    for r in wm.rings.iter_mut() {
        if r.w > 0.0 && t - r.z > 4.0 {
            r.w = 0.0;
        }
    }
    let rings = wm.rings;
    let modern = modern();
    let switched = wm.modern != Some(modern);
    wm.modern = Some(modern);
    let rain = (crate::weather::density() / 14.0).clamp(0.0, 1.0);
    for (i, h) in wm.mats.iter().enumerate() {
        if let Some(mut m) = mats.get_mut(h) {
            m.extension.params.t.x = t;
            m.extension.params.rings = rings;
            m.extension.params.m.x = if modern { 1.0 } else { 0.0 };
            m.extension.params.m.z = rain;
            if switched {
                let opaque = wm.opaque.get(i).copied().unwrap_or(false);
                m.base.alpha_mode = match (opaque, modern) {
                    (true, _) => AlphaMode::Opaque,
                    (false, true) => AlphaMode::Blend,
                    (false, false) => AlphaMode::Add,
                };
                m.base.perceptual_roughness = if modern { 0.07 } else { 1.0 };
                m.base.reflectance = if modern { 0.5 } else { 0.05 };
            }
        }
    }
}

/// The water material for a level batch (its source picture, translucent). `placeholder` stands in for the
/// reflection until a mirror slot holds this plane.
pub fn material(base: StandardMaterial, opaque: bool, placeholder: Handle<Image>) -> WaterMaterial {
    WaterMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            // PF_Translucent like the original's water faces: added onto what is beneath; a portal sheet without it
            // (Battery Park) is solid.
            alpha_mode: if opaque { AlphaMode::Opaque } else { AlphaMode::Add },
            ..base
        },
        extension: WaterExt {
            params: WaterParams {
                // The lit texture as is (PF_Translucent: texture x lightmap added on), so the lightmap's shadows and
                // lamp pools show; 2026-10-26 playtest: the old unlit sky term made the water too bright and even.
                t: Vec4::new(
                    0.0,
                    0.035,
                    std::env::var("DXVIEW_WATER_GAIN").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0),
                    0.03,
                ),
                far: Vec4::new(30.0, 120.0, std::env::var("DXVIEW_WATER_DEBUG").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0), 0.0),
                rings: [Vec4::ZERO; RINGS],
                m: Vec4::new(0.0, 0.0, 0.0, std::env::var("DXVIEW_WATER_BEND").ok().and_then(|v| v.parse().ok()).unwrap_or(0.03)),
                // Night harbour water: dark blue-green, a fair way see-through looking straight down.
                tint: Vec4::new(0.010, 0.030, 0.034, 0.45),
            },
            reflection: placeholder,
        },
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn swell_gradient_matches_height() {
        let (h0, gx, gz) = super::swell(3.0, -2.0, 1.5);
        let e = 1e-3;
        let (hx, _, _) = super::swell(3.0 + e, -2.0, 1.5);
        let (hz, _, _) = super::swell(3.0, -2.0 + e, 1.5);
        assert!(((hx - h0) / e - gx).abs() < 0.05);
        assert!(((hz - h0) / e - gz).abs() < 0.05);
    }
}
