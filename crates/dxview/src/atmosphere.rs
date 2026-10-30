//! Enhancements (Atmospheric Fog and Light Halos, ROADMAP v2 #32). Ours, off by default.
//!
//! - Atmospheric Fog: a thin haze over distance in the world view, thicker in the rain, so far buildings and the
//!   skyline (skyline.rs) melt into the night. Bevy's DistanceFog on the world camera; its colour is the map's
//!   own night tint (dark blue-grey, warmer over the city in rain), underwater the zone's fog decides.
//! - Light Halos: the glow of air around the map's lamps - a soft volume of light about each lamp in sight,
//!   tinted like the lamp, sized by its radius and fading out as you get close (it never fills the screen). They
//!   fade in and out as the lamp comes into sight (a trace from the eye), and are drawn as one mesh of soft
//!   camera-facing discs pulled toward the camera so the wall a lamp sits on doesn't cut them.

use crate::{FlyCam, LevelRes, SkyCam};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use dx_level::ue_to_bevy;
use glam::DVec3;

pub fn fog_on() -> bool {
    crate::settings::get("AtmosphericFog") > 0.5
}

pub fn halos_on() -> bool {
    crate::settings::get("LightHalos") > 0.5
}

const MAX_HALOS: usize = 96;

/// Fog for the world camera: on/off and thicker in the rain.
pub fn fog_system(
    mut commands: Commands,
    level: Res<LevelRes>,
    cam: Single<(Entity, Option<&DistanceFog>), (With<FlyCam>, Without<SkyCam>)>,
    mut cur: Local<f32>,
) {
    let (e, fog) = *cam;
    if !fog_on() {
        if fog.is_some() {
            commands.entity(e).remove::<DistanceFog>();
        }
        return;
    }
    // Indoor maps (no sky anywhere) hardly need it; outdoor ones get a night haze.
    let outdoors = level.0.zone_sky.iter().any(|s| *s);
    let rain = crate::weather::density() / 14.0;
    let vis = if outdoors { 2600.0 - 1500.0 * rain } else { 4000.0 };
    let changed = (vis - *cur).abs() > 5.0 || fog.is_none();
    if !changed {
        return;
    }
    *cur = vis;
    let c = if rain > 0.1 { Color::linear_rgb(0.012, 0.012, 0.014) } else { Color::linear_rgb(0.006, 0.008, 0.013) };
    commands.entity(e).insert(DistanceFog {
        color: c,
        directional_light_color: Color::NONE,
        directional_light_exponent: 8.0,
        falloff: FogFalloff::from_visibility_squared(vis),
    });
}

#[derive(Default)]
pub struct HaloState {
    mesh: Option<Handle<Mesh>>,
    /// Per light: how much of it shows (eased), and the time of its last sight check.
    shown: Vec<f32>,
    seen: Vec<bool>,
    check: usize,
    idle: bool,
}

#[derive(Component)]
pub struct HaloMesh;

/// A soft round glow (white; brightness falls off as 1 / (1 + r^2) to the edge).
fn glow_image() -> Image {
    const N: u32 = 64;
    let mut px = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let dx = (x as f32 + 0.5) / N as f32 * 2.0 - 1.0;
            let dy = (y as f32 + 0.5) / N as f32 * 2.0 - 1.0;
            let r = (dx * dx + dy * dy).sqrt();
            let a = ((1.0 - r).max(0.0) / (1.0 + 12.0 * r * r)).powf(1.2);
            let v = (a * 255.0) as u8;
            px.extend([v, v, v, 255]);
        }
    }
    Image::new(
        Extent3d { width: N, height: N, depth_or_array_layers: 1 },
        TextureDimension::D2,
        px,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn pad(pos: &mut Vec<[f32; 3]>, uv: &mut Vec<[f32; 2]>, col: &mut Vec<[f32; 4]>, idx: &mut Vec<u32>, cap: usize) {
    pos.truncate(cap * 4);
    uv.truncate(cap * 4);
    col.truncate(cap * 4);
    idx.truncate(cap * 6);
    while pos.len() < cap * 4 {
        let k = pos.len() as u32;
        pos.extend([[0.0, -1.0e5, 0.0]; 4]);
        uv.extend([[0.0, 0.0]; 4]);
        col.extend([[0.0, 0.0, 0.0, 0.0]; 4]);
        idx.extend([k, k, k, k, k, k]);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn halo_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    cam: Single<&Transform, (With<FlyCam>, Without<SkyCam>, Without<HaloMesh>)>,
    mut st: Local<HaloState>,
    mut map_gen: Local<u64>,
    mut vis: Query<&mut Visibility, With<HaloMesh>>,
    (mut meshes, mut materials, mut images): (ResMut<Assets<Mesh>>, ResMut<Assets<StandardMaterial>>, ResMut<Assets<Image>>),
) {
    let lvl = &level.0;
    if crate::loader::new_map(&mut map_gen) {
        st.shown = vec![0.0; lvl.lights.len()];
        st.seen = vec![false; lvl.lights.len()];
        st.check = 0;
    }
    if !halos_on() {
        for mut v in &mut vis {
            *v = Visibility::Hidden;
        }
        return;
    }
    if st.mesh.is_none() {
        let img = images.add(glow_image());
        let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        let (mut p, mut u, mut c, mut i) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        pad(&mut p, &mut u, &mut c, &mut i, MAX_HALOS);
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, p);
        m.insert_attribute(Mesh::ATTRIBUTE_UV_0, u);
        m.insert_attribute(Mesh::ATTRIBUTE_COLOR, c);
        m.insert_indices(Indices::U32(i));
        let h = meshes.add(m);
        let mat = materials.add(StandardMaterial {
            base_color_texture: Some(img),
            unlit: true,
            double_sided: true,
            cull_mode: None,
            alpha_mode: AlphaMode::Add,
            fog_enabled: false,
            ..default()
        });
        commands.spawn((
            Mesh3d(h.clone()),
            MeshMaterial3d(mat),
            Transform::default(),
            Visibility::Visible,
            HaloMesh,
            bevy::camera::visibility::NoFrustumCulling,
            bevy::light::NotShadowCaster,
            bevy::light::NotShadowReceiver,
        ));
        st.mesh = Some(h);
    }
    for mut v in &mut vis {
        *v = Visibility::Visible;
    }
    let dt = time.delta_secs().min(0.1);
    let conv = |v: Vec3| DVec3::new(v.x as f64, v.z as f64, v.y as f64);
    let fw = conv(cam.forward().as_vec3());
    let right = conv(cam.right().as_vec3());
    let up = conv(cam.up().as_vec3());
    let eye = {
        let e = dx_level::bevy_to_ue(cam.translation.to_array());
        DVec3::new(e[0] as f64, e[1] as f64, e[2] as f64)
    };
    if st.shown.len() != lvl.lights.len() {
        st.shown = vec![0.0; lvl.lights.len()];
        st.seen = vec![false; lvl.lights.len()];
    }
    // The lamps that could show: in range, in front (with a margin), bright enough.
    let mut cand: Vec<(f64, usize)> = lvl
        .lights
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            let lp = DVec3::new(l.pos[0] as f64, l.pos[1] as f64, l.pos[2] as f64);
            let d = lp.distance(eye);
            let b = l.color[0].max(l.color[1]).max(l.color[2]);
            let size = l.radius as f64 * 0.35;
            (d < 4000.0 && b > 0.08 && (lp - eye).dot(fw) > -size).then_some((d, i))
        })
        .collect();
    cand.sort_by(|a, b| a.0.total_cmp(&b.0));
    cand.truncate(MAX_HALOS);
    // Sight checks: a few lamps a frame, round robin over the candidates.
    let n = cand.len();
    for k in 0..n.min(12) {
        let (_, i) = cand[(st.check + k) % n.max(1)];
        let l = &lvl.lights[i];
        let lp = DVec3::new(l.pos[0] as f64, l.pos[1] as f64, l.pos[2] as f64);
        // From the lamp a little toward the eye (lamps sit in fixtures and walls).
        let start = lp + (eye - lp).normalize_or_zero() * 12.0;
        st.seen[i] = lvl.collision.trace(start, eye, DVec3::ZERO).fraction >= 0.999;
    }
    st.check = st.check.wrapping_add(12);
    let mut pos = Vec::new();
    let mut uv = Vec::new();
    let mut col = Vec::new();
    let mut idx = Vec::new();
    let b = |p: DVec3| Vec3::from(ue_to_bevy(p.as_vec3().to_array())).to_array();
    let fog_thick = 1.0 + 1.5 * crate::weather::density() / 14.0;
    for (d, i) in cand {
        let target = if st.seen[i] { 1.0 } else { 0.0 };
        let s = &mut st.shown[i];
        *s += (target - *s).clamp(-dt * 3.0, dt * 3.0);
        if *s <= 0.0 {
            continue;
        }
        let l = &lvl.lights[i];
        let lp = DVec3::new(l.pos[0] as f64, l.pos[1] as f64, l.pos[2] as f64);
        let size = (l.radius as f64 * 0.22).clamp(20.0, 260.0);
        // Close up the glow thins out (you're inside it): nothing within half its size, full from twice it.
        let near = ((d - size * 0.5) / (size * 1.5)).clamp(0.0, 1.0) as f32;
        let bright = l.color[0].max(l.color[1]).max(l.color[2]);
        let k = *s * near * 0.03 * fog_thick as f32 * (bright * 1.5).min(1.0) / bright.max(1e-3);
        if k <= 0.001 {
            continue;
        }
        let c = [l.color[0] * k, l.color[1] * k, l.color[2] * k, 1.0];
        // Pulled toward the eye so a wall the lamp is on doesn't cut the disc in half.
        let at = lp + (eye - lp).normalize_or_zero() * size.min(d * 0.5);
        let (r, u) = (right * size, up * size);
        let q = pos.len() as u32;
        pos.extend([b(at - r - u), b(at + r - u), b(at + r + u), b(at - r + u)]);
        uv.extend([[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]);
        col.extend([c; 4]);
        idx.extend([q, q + 1, q + 2, q, q + 2, q + 3]);
    }
    if pos.is_empty() && st.idle {
        return;
    }
    st.idle = pos.is_empty();
    pad(&mut pos, &mut uv, &mut col, &mut idx, MAX_HALOS);
    if let Some(m) = st.mesh.as_ref().and_then(|h| meshes.get_mut(h)) {
        let mut m = m;
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
        m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
        m.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
        m.insert_indices(Indices::U32(idx));
    }
}
