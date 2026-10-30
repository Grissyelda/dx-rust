//! Enhancement: Glow And Bloom (setting Bloom, ours, off by default).
//!
//! The picture is rendered in high dynamic range and a bloom filter spreads only what is brighter than white, so
//! the ordinary scene is untouched and only glowing things bleed light into the dark around them: security camera
//! lenses and alarm lights, the augmentation canisters' nano glow, bioelectric cells, laser tripwire beams and
//! spots, JC's sunglasses (the lens highlights), keypad and computer lights. Those materials are pushed past white
//! by a strength per kind (the meshes are unlit, so their colour is what reaches the screen).

use bevy::camera::Hdr;
use bevy::post_process::bloom::{Bloom, BloomCompositeMode, BloomPrefilter};
use bevy::prelude::*;
use std::collections::HashMap;
use std::sync::Mutex;

pub fn on() -> bool {
    crate::settings::get("Bloom") > 0.5
}

/// Glowing textures by (part of their) name, and how far past white they go.
const GLOWS: &[(&str, f32)] = &[
    ("greenlighttex", 3.0),
    ("redlighttex", 3.0),
    ("yellowlighttex", 3.0),
    ("bluelighttex", 3.0),
    ("nano_sfx", 3.5),
    ("biocell_sfx", 3.0),
    ("laserbeam", 4.0),
    ("laserspot", 4.0),
    ("lensestex5", 6.0),
    ("flatfxtex34", 2.5),
    ("flatfxtex37", 2.5),
    ("securitycameratex", 1.0),
    ("keypadlight", 3.0),
];

fn boost(name: &str) -> Option<f32> {
    let n = name.to_ascii_lowercase();
    GLOWS.iter().find(|(k, _)| n.contains(k)).map(|g| g.1).filter(|b| *b > 1.0)
}

static GLOW: Mutex<Option<HashMap<AssetId<Image>, f32>>> = Mutex::new(None);

/// A texture image the game loaded by this name (level textures, runtime images): remembered when it should glow.
pub fn register(h: &Handle<Image>, name: &str) {
    if let Some(b) = boost(name) {
        if let Ok(mut g) = GLOW.lock() {
            g.get_or_insert_with(HashMap::new).insert(h.id(), b);
        }
    }
}

fn glow_of(id: AssetId<Image>) -> Option<f32> {
    GLOW.lock().ok()?.as_ref()?.get(&id).copied()
}

#[derive(Default)]
pub struct GlowState {
    on: bool,
    /// Materials pushed past white: their own colour to give back.
    applied: HashMap<AssetId<StandardMaterial>, Color>,
    scan: f32,
}

fn scale(c: Color, k: f32) -> Color {
    let l = c.to_linear();
    Color::LinearRgba(LinearRgba::new(l.red * k, l.green * k, l.blue * k, l.alpha))
}

/// The bloom on the cameras, and the glowing materials (looked for twice a second: the game swaps lens and screen
/// materials as things change).
pub fn glow_system(
    mut commands: Commands,
    time: Res<Time>,
    mut st: Local<GlowState>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    cams: Query<(Entity, Has<Hdr>, Has<Bloom>, Has<crate::weapons::ViewModelCam>), Or<(With<crate::FlyCam>, With<crate::SkyCam>, With<crate::weapons::ViewModelCam>, With<crate::augdisplay::HeatCam>)>>,
) {
    let want = on();
    // Every camera drawing into the window shares one HDR picture; the last (the view model camera, which also
    // does the brightness pass) blooms all of it.
    for (e, hdr, bloom, last) in &cams {
        if want && !hdr {
            commands.entity(e).insert(Hdr);
        }
        if want && last && !bloom {
            commands.entity(e).insert(Bloom {
                intensity: 0.22,
                low_frequency_boost: 0.6,
                low_frequency_boost_curvature: 0.9,
                high_pass_frequency: 0.9,
                prefilter: BloomPrefilter { threshold: 1.0, threshold_softness: 0.25 },
                composite_mode: BloomCompositeMode::Additive,
                ..default()
            });
        }
        if !want && bloom {
            commands.entity(e).remove::<Bloom>();
        }
        if !want && hdr {
            commands.entity(e).remove::<Hdr>();
        }
    }
    if !want {
        if st.on {
            for (id, c) in st.applied.drain() {
                if let Some(mut m) = materials.get_mut(id) {
                    m.base_color = c;
                }
            }
            st.on = false;
        }
        return;
    }
    st.on = true;
    st.scan -= time.delta_secs();
    if st.scan > 0.0 {
        return;
    }
    st.scan = 0.5;
    let found: Vec<(AssetId<StandardMaterial>, f32)> = materials
        .iter()
        .filter(|(id, _)| !st.applied.contains_key(id))
        .filter_map(|(id, m)| Some((id, glow_of(m.base_color_texture.as_ref()?.id())?)))
        .collect();
    for (id, k) in found {
        if let Some(mut m) = materials.get_mut(id) {
            let c = m.base_color;
            st.applied.insert(id, c);
            m.base_color = scale(c, k);
        }
    }
    // Forget materials that are gone.
    if st.applied.len() > 64 {
        st.applied.retain(|id, _| materials.get(*id).is_some());
    }
}
