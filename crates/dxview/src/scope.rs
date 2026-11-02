//! Enhancement "Diegetic Scope" (setting DiegeticScope, ours, in the Modern preset): a scoped weapon zoomed doesn't
//! black out the screen behind the original's ScopeView picture. The scope is a live lens in front of the eye: a
//! second camera renders the world at the scope's field of view into an image shown on a glass disc, ringed by the
//! scope's tube and carrying a reticle, while the rest of the view stays at (nearly) the normal field of view. The
//! original's scope sway (drug::scope_sway_system: random drift by the weapon skill's accuracy) still moves the view,
//! magnified in the glass, so an untrained eye watches the picture wander under the reticle and a master's stays
//! put. Binoculars keep the original overlay.

use bevy::camera::visibility::RenderLayers;
use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};

/// The lens picture (square, pixels).
const RES: u32 = 768;
/// How far in front of the eye the lens sits (m): nearer than any view model.
const LENS_D: f32 = 0.06;
/// The lens radius as a fraction of the screen's half height.
const LENS_FRAC: f32 = 0.62;
/// The tube round the lens: its outer radius over the lens radius.
const RIM: f32 = 1.22;

pub fn setting_on() -> bool {
    crate::settings::get("DiegeticScope") > 0.5
}

/// A scoped weapon (not binoculars) is zoomed with the setting on.
pub fn active(zoom: &crate::weapons::Zoom, weapons: &crate::weapons::Weapons) -> bool {
    setting_on()
        && zoom.0.is_some()
        && weapons.held.as_ref().is_some_and(|h| h.stats.has_scope && h.stats.class != "Binoculars")
}

static ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Looking through the diegetic scope now (set each frame by `scope_system`).
pub fn diegetic() -> bool {
    ACTIVE.load(std::sync::atomic::Ordering::Relaxed)
}

/// The rifle's eyepiece in the view model camera's space (centre, rotation, radius in metres), set by the weapon
/// system while the lens sits in it; None = the lens floats on its own in front of the eye.
static LENS: std::sync::Mutex<Option<(Vec3, Quat, f32)>> = std::sync::Mutex::new(None);

pub fn set_lens(l: Option<(Vec3, Quat, f32)>) {
    if let Ok(mut g) = LENS.lock() {
        *g = l;
    }
}

#[derive(Component)]
pub struct ScopeCam;

/// `.0` = the disc's place in the stack: 0 the picture, 1 the reticle and tube, 2 the glass's sheen.
#[derive(Component)]
pub struct ScopeGlass(u8);

#[derive(Default)]
pub struct ScopeState {
    built: bool,
}

/// The reticle and tube picture over the lens: transparent glass with a fine cross and heavier posts, darkening
/// towards the glass's edge, then the black tube out to the rim (fading at its outer edge).
fn rim_image() -> Image {
    const N: usize = 512;
    let mut px = vec![0u8; N * N * 4];
    let c = (N as f32 - 1.0) * 0.5;
    let r_glass = c / RIM;
    for y in 0..N {
        for x in 0..N {
            let (dx, dy) = (x as f32 - c, y as f32 - c);
            let r = (dx * dx + dy * dy).sqrt();
            let mut a: f32;
            if r <= r_glass {
                // Vignette inside the glass.
                let t = r / r_glass;
                a = (t - 0.72).max(0.0) / 0.28;
                a = a * a * 0.85;
                // The reticle: a fine cross, heavy posts from the edge to a third of the way in.
                let (ax, ay) = (dx.abs(), dy.abs());
                let fine = 0.9;
                let post = 3.2;
                let on_h = ay < fine || (ay < post && ax > r_glass * 0.33);
                let on_v = ax < fine || (ax < post && ay > r_glass * 0.33);
                if (on_h || on_v) && r < r_glass * 0.98 {
                    a = 1.0;
                }
            } else {
                // The tube: black, soft at the outside.
                let t = (r - r_glass) / (c - r_glass);
                a = 1.0 - ((t - 0.7).max(0.0) / 0.3).min(1.0);
            }
            let i = (y * N + x) * 4;
            px[i + 3] = (a.clamp(0.0, 1.0) * 255.0) as u8;
        }
    }
    Image::new(
        Extent3d { width: N as u32, height: N as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        px,
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    )
}

/// The glass itself, drawn additively over the picture: a soft reflection of the sky arcing across the upper left,
/// a faint cool tint thickening toward the edge with a thin warm / cool colour fringe at the rim (chromatic
/// aberration), so the lens reads as glass in front of the eye rather than a hole.
fn sheen_image() -> Image {
    const N: usize = 256;
    let mut px = vec![0u8; N * N * 4];
    let c = (N as f32 - 1.0) * 0.5;
    for y in 0..N {
        for x in 0..N {
            let (dx, dy) = ((x as f32 - c) / c, (y as f32 - c) / c);
            let r = (dx * dx + dy * dy).sqrt();
            if r > 1.0 {
                continue;
            }
            // The reflection: a broad crescent band toward the upper left, brightest along an arc.
            let (ox, oy) = (dx + 0.55, dy + 0.55);
            let arc = ((ox * ox + oy * oy).sqrt() - 0.75).abs();
            let band = (1.0 - arc / 0.22).max(0.0).powi(2) * (1.0 - r).clamp(0.0, 1.0).sqrt();
            // A small hot glint near the top left.
            let (gx, gy) = (dx + 0.42, dy + 0.48);
            let glint = (1.0 - (gx * gx + gy * gy).sqrt() / 0.12).max(0.0).powi(2);
            // Tint and fringe at the edge.
            let edge = ((r - 0.7) / 0.3).clamp(0.0, 1.0).powi(2);
            let fringe_w = (1.0 - ((r - 0.93) / 0.05).abs()).max(0.0);
            let fringe_c = (1.0 - ((r - 0.985) / 0.03).abs()).max(0.0);
            let rr = band * 0.10 + glint * 0.35 + edge * 0.015 + fringe_w * 0.06;
            let gg = band * 0.11 + glint * 0.35 + edge * 0.03 + fringe_c * 0.04;
            let bb = band * 0.13 + glint * 0.35 + edge * 0.05 + fringe_c * 0.09;
            let i = (y * N + x) * 4;
            px[i] = (rr.clamp(0.0, 1.0) * 255.0) as u8;
            px[i + 1] = (gg.clamp(0.0, 1.0) * 255.0) as u8;
            px[i + 2] = (bb.clamp(0.0, 1.0) * 255.0) as u8;
            px[i + 3] = 255;
        }
    }
    Image::new(
        Extent3d { width: N as u32, height: N as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        px,
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    )
}

/// A disc of `r` (m), UVs across its square, facing +Z (toward the view camera, which looks down -Z).
fn disc(r: f32) -> Mesh {
    const SEG: usize = 64;
    let mut pos = vec![[0.0f32, 0.0, 0.0]];
    let mut uv = vec![[0.5f32, 0.5]];
    for k in 0..=SEG {
        let a = k as f32 / SEG as f32 * std::f32::consts::TAU;
        let (s, c) = a.sin_cos();
        pos.push([c * r, s * r, 0.0]);
        uv.push([0.5 + c * 0.5, 0.5 - s * 0.5]);
    }
    let mut idx = Vec::new();
    for k in 1..=SEG as u32 {
        idx.extend_from_slice(&[0, k, k + 1]);
    }
    let n = pos.len();
    let mut m = Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::default());
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 0.0, 1.0]; n]);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    m.insert_indices(bevy::mesh::Indices::U32(idx));
    m
}

/// Builds the scope camera and the glass once, then each frame turns them on with the scope, keeps the camera on
/// the eye at the scope's field of view and sizes the glass to the screen.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn scope_system(
    mut commands: Commands,
    zoom: Res<crate::weapons::Zoom>,
    weapons: Res<crate::weapons::Weapons>,
    mut st: Local<ScopeState>,
    eye: Query<(&Transform, &Projection), (With<crate::FlyCam>, Without<crate::SkyCam>, Without<ScopeCam>)>,
    vm: Query<&Projection, (With<crate::weapons::ViewModelCam>, Without<ScopeCam>)>,
    mut cam: Query<(&mut Camera, &mut Transform, &mut Projection), (With<ScopeCam>, Without<crate::FlyCam>, Without<ScopeGlass>)>,
    mut glass: Query<(&ScopeGlass, &mut Transform, &mut Visibility), (Without<ScopeCam>, Without<crate::FlyCam>)>,
    (mut meshes, mut materials, mut images): (ResMut<Assets<Mesh>>, ResMut<Assets<StandardMaterial>>, ResMut<Assets<Image>>),
) {
    let on = active(&zoom, &weapons) && !crate::thirdperson::active();
    ACTIVE.store(on, std::sync::atomic::Ordering::Relaxed);
    // A new map clears the world (the camera and glass with it): built again when next wanted.
    if st.built && cam.is_empty() {
        st.built = false;
    }
    if !st.built {
        if !on {
            return;
        }
        st.built = true;
        let mut image = Image::new_uninit(
            Extent3d { width: RES, height: RES, depth_or_array_layers: 1 },
            TextureDimension::D2,
            TextureFormat::Bgra8UnormSrgb,
            bevy::asset::RenderAssetUsages::MAIN_WORLD | bevy::asset::RenderAssetUsages::RENDER_WORLD,
        );
        image.texture_descriptor.usage |= TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::RENDER_ATTACHMENT;
        let image = images.add(image);
        let mut layers = vec![0, crate::mirror::LAYER, crate::weather::RAIN_LAYER, crate::skyfollow::LAYER];
        if crate::shadowlight::level() > 0 {
            layers.extend_from_slice(&crate::shadowlight::SLOT_LAYERS);
        }
        commands.spawn((
            Camera3d::default(),
            Camera {
                // Before the world view (and after the reflections, which it may show on water).
                order: -2,
                is_active: false,
                clear_color: ClearColorConfig::Custom(Color::BLACK),
                ..default()
            },
            RenderTarget::Image(image.clone().into()),
            Projection::Perspective(PerspectiveProjection::default()),
            Transform::default(),
            Tonemapping::None,
            bevy::camera::Exposure { ev100: -0.263 },
            bevy::render::view::Msaa::Off,
            RenderLayers::from_layers(&layers),
            ScopeCam,
        ));
        let lens = materials.add(StandardMaterial {
            // A faint cool cast through the glass.
            base_color: Color::srgb(0.93, 0.97, 1.0),
            base_color_texture: Some(image),
            unlit: true,
            fog_enabled: false,
            ..default()
        });
        let sheen = materials.add(StandardMaterial {
            base_color_texture: Some(images.add(sheen_image())),
            unlit: true,
            fog_enabled: false,
            alpha_mode: AlphaMode::Add,
            ..default()
        });
        let rim = materials.add(StandardMaterial {
            base_color: Color::BLACK,
            base_color_texture: Some(images.add(rim_image())),
            unlit: true,
            fog_enabled: false,
            alpha_mode: AlphaMode::Blend,
            ..default()
        });
        let layer = RenderLayers::layer(crate::weapons::VIEW_LAYER);
        let unit = meshes.add(disc(1.0));
        for (k, (mesh, mat)) in [(unit.clone(), lens), (meshes.add(disc(RIM)), rim), (unit, sheen)].into_iter().enumerate() {
            commands.spawn((
                Mesh3d(mesh),
                MeshMaterial3d(mat),
                Transform::from_xyz(0.0, 0.0, -LENS_D),
                Visibility::Hidden,
                layer.clone(),
                ScopeGlass(k as u8),
                bevy::light::NotShadowCaster,
                bevy::light::NotShadowReceiver,
                bevy::camera::visibility::NoFrustumCulling,
            ));
        }
        return;
    }
    let Ok((mut c, mut tf, mut proj)) = cam.single_mut() else { return };
    if c.is_active != on {
        c.is_active = on;
    }
    // The lens's size: LENS_FRAC of the view model camera's half height at LENS_D (both drawn by that camera).
    let half_h = match vm.iter().next() {
        Some(Projection::Perspective(p)) => (p.fov * 0.5).tan() * LENS_D,
        _ => 0.03,
    };
    let r = half_h * LENS_FRAC;
    let lens = LENS.lock().ok().and_then(|g| *g);
    for (k, mut g, mut v) in &mut glass {
        let want = if on { Visibility::Inherited } else { Visibility::Hidden };
        if *v != want {
            *v = want;
        }
        if !on {
            continue;
        }
        // Each disc a hair nearer the eye than the one before (picture, reticle, sheen).
        let lift = k.0 as f32 * 0.0004;
        *g = match lens {
            // In the rifle's eyepiece: just in front of its rear cap (the original's eyepiece is a closed face, which
            // hid a lens set inside the tube), a little smaller than the tube.
            Some((c, rot, rad)) => Transform {
                translation: c + rot * Vec3::new(0.0, 0.0, 0.003 + lift),
                rotation: rot,
                scale: Vec3::splat(rad * 0.94),
            },
            None => Transform::from_xyz(0.0, 0.0, -LENS_D + lift).with_scale(Vec3::splat(r)),
        };
    }
    if !on {
        return;
    }
    let mut far = 1000.0;
    if let Some((e, p)) = eye.iter().next() {
        *tf = *e;
        if let Projection::Perspective(p) = p {
            far = p.far;
        }
    }
    // The scope's field of view (Zoom holds it horizontally, in degrees) across the square picture.
    let fov = zoom.0.unwrap_or(20.0).to_radians();
    if let Projection::Perspective(p) = &mut *proj {
        if (p.fov - fov).abs() > 1e-5 || (p.aspect_ratio - 1.0).abs() > 1e-5 || (p.far - far).abs() > 0.5 {
            p.fov = fov;
            p.aspect_ratio = 1.0;
            p.near = 0.1;
            p.far = far;
        }
    }
}
