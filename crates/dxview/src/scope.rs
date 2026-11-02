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

#[derive(Component)]
pub struct ScopeCam;

#[derive(Component)]
pub struct ScopeGlass;

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
    mut glass: Query<(&mut Transform, &mut Visibility), (With<ScopeGlass>, Without<ScopeCam>, Without<crate::FlyCam>)>,
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
            layers.push(crate::shadowlight::SHADOW_LAYER);
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
            base_color_texture: Some(image),
            unlit: true,
            fog_enabled: false,
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
        for (mesh, mat, z) in [(meshes.add(disc(1.0)), lens, 0.0f32), (meshes.add(disc(RIM)), rim, 0.0005)] {
            commands.spawn((
                Mesh3d(mesh),
                MeshMaterial3d(mat),
                Transform::from_xyz(0.0, 0.0, -LENS_D + z),
                Visibility::Hidden,
                layer.clone(),
                ScopeGlass,
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
    for (mut g, mut v) in &mut glass {
        let want = if on { Visibility::Inherited } else { Visibility::Hidden };
        if *v != want {
            *v = want;
        }
        if on && (g.scale.x - r).abs() > 1e-6 {
            g.scale = Vec3::splat(r);
        }
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
