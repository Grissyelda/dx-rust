//! Enhancement: Night Clouds (setting Clouds, ours, off by default) - a volumetric cloud layer over the open-air
//! maps.
//!
//! One big sheet sits at the bottom of a cloud slab high over the map; its shader (shaders/clouds.wgsl) marches
//! each pixel's view ray up through the slab, reading a tiling 3D noise (puffy cells eroded by finer noise, made
//! here once) that the wind carries along and that slowly churns, so the clouds drift, change shape and thin out
//! into gaps. They are lit from below by the city (orange over New York and Hong Kong) and from above by the moon,
//! each step's moon light dimmed by the cloud between it and the moon, so thick clouds have dark bellies and bright
//! edges.

use crate::LevelRes;
use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat};
use bevy::shader::ShaderRef;

pub const CLOUD_SHADER: Handle<Shader> = bevy::asset::uuid_handle!("3c7e1d52-9a4b-4f60-8d2e-b51a07c4e9f3");

pub fn on() -> bool {
    crate::settings::get("Clouds") > 0.5
}

#[derive(Clone, Copy, Debug, Default, ShaderType, Reflect)]
pub struct CloudParams {
    /// x: slab bottom (Bevy y, m), y: thickness (m), z: noise tiles per metre, w: coverage 0..1.
    pub slab: Vec4,
    /// x, z: wind (noise tiles per second), y: churn speed, w: density.
    pub wind: Vec4,
    /// The city's glow from below (rgb) and its strength.
    pub glow: Vec4,
    /// Direction to the moon (xyz) and its strength.
    pub moon: Vec4,
    /// x, y: the sheet's centre (Bevy x, z), z: radius where it starts fading, w: where it's gone.
    pub fade: Vec4,
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct CloudMaterial {
    #[uniform(0)]
    pub params: CloudParams,
    #[texture(1, dimension = "3d")]
    #[sampler(2)]
    pub noise: Handle<Image>,
}

impl Material for CloudMaterial {
    fn fragment_shader() -> ShaderRef {
        CLOUD_SHADER.into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
}

pub fn setup_cloud_shader(mut shaders: ResMut<Assets<Shader>>) {
    let _ = shaders.insert(&CLOUD_SHADER, Shader::from_wgsl(include_str!("shaders/clouds.wgsl"), "dxview/clouds.wgsl"));
}

#[derive(Component)]
pub struct CloudSheet;

const N: usize = 64;

fn hash3(x: i32, y: i32, z: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6b343) ^ (y as u32).wrapping_mul(0xd8163841) ^ (z as u32).wrapping_mul(0xcb1ab31f) ^ seed;
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1e995);
    h ^= h >> 15;
    (h & 0xffff) as f32 / 65535.0
}

/// Tiling value noise with `cells` lattice cells across the texture.
fn value(p: [f32; 3], cells: i32, seed: u32) -> f32 {
    let f = [p[0] * cells as f32, p[1] * cells as f32, p[2] * cells as f32];
    let i = [f[0].floor() as i32, f[1].floor() as i32, f[2].floor() as i32];
    let t = [f[0] - i[0] as f32, f[1] - i[1] as f32, f[2] - i[2] as f32];
    let s = |x: f32| x * x * (3.0 - 2.0 * x);
    let (u, v, w) = (s(t[0]), s(t[1]), s(t[2]));
    let at = |dx: i32, dy: i32, dz: i32| {
        hash3((i[0] + dx).rem_euclid(cells), (i[1] + dy).rem_euclid(cells), (i[2] + dz).rem_euclid(cells), seed)
    };
    let l = |a: f32, b: f32, k: f32| a + (b - a) * k;
    l(
        l(l(at(0, 0, 0), at(1, 0, 0), u), l(at(0, 1, 0), at(1, 1, 0), u), v),
        l(l(at(0, 0, 1), at(1, 0, 1), u), l(at(0, 1, 1), at(1, 1, 1), u), v),
        w,
    )
}

/// Tiling cellular noise: 1 at a cell's point, falling off to 0 between points (puffs).
fn worley(p: [f32; 3], cells: i32, seed: u32) -> f32 {
    let f = [p[0] * cells as f32, p[1] * cells as f32, p[2] * cells as f32];
    let i = [f[0].floor() as i32, f[1].floor() as i32, f[2].floor() as i32];
    let mut best = 9.0f32;
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let c = [i[0] + dx, i[1] + dy, i[2] + dz];
                let w = [c[0].rem_euclid(cells), c[1].rem_euclid(cells), c[2].rem_euclid(cells)];
                let pt = [
                    c[0] as f32 + hash3(w[0], w[1], w[2], seed),
                    c[1] as f32 + hash3(w[0], w[1], w[2], seed ^ 0x51),
                    c[2] as f32 + hash3(w[0], w[1], w[2], seed ^ 0xa3),
                ];
                let d = (pt[0] - f[0]).powi(2) + (pt[1] - f[1]).powi(2) + (pt[2] - f[2]).powi(2);
                best = best.min(d);
            }
        }
    }
    (1.0 - best.sqrt()).clamp(0.0, 1.0)
}

/// The cloud noise: puffy cells (Worley) with value-noise fBm through them, tiling in all three directions.
pub fn noise_image() -> Image {
    let mut v = vec![0.0f32; N * N * N];
    for z in 0..N {
        for y in 0..N {
            for x in 0..N {
                let p = [x as f32 / N as f32, y as f32 / N as f32, z as f32 / N as f32];
                let fbm = value(p, 4, 1) * 0.5 + value(p, 8, 2) * 0.3 + value(p, 16, 3) * 0.2;
                let cell = worley(p, 4, 7) * 0.6 + worley(p, 8, 9) * 0.4;
                v[(z * N + y) * N + x] = fbm * 0.45 + cell * 0.55;
            }
        }
    }
    let (lo, hi) = v.iter().fold((f32::MAX, f32::MIN), |a, x| (a.0.min(*x), a.1.max(*x)));
    let data: Vec<u8> = v.iter().map(|x| (((x - lo) / (hi - lo).max(1e-5)) * 255.0) as u8).collect();
    let mut im = Image::new(
        Extent3d { width: N as u32, height: N as u32, depth_or_array_layers: N as u32 },
        TextureDimension::D3,
        data,
        TextureFormat::R8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    im.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        address_mode_w: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    im
}

#[derive(Default)]
pub struct CloudState {
    noise: Option<Handle<Image>>,
    last: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn clouds_system(
    mut commands: Commands,
    level: Res<LevelRes>,
    mut st: Local<CloudState>,
    mut map_gen: Local<u64>,
    sheets: Query<Entity, With<CloudSheet>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<CloudMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let lvl = &level.0;
    let fresh = crate::loader::new_map(&mut map_gen);
    let want = on();
    if !fresh && want == st.last {
        return;
    }
    st.last = want;
    for e in &sheets {
        commands.entity(e).despawn();
    }
    if !want || !crate::skyline::cloudy(&lvl.name) || !lvl.zone_sky.iter().any(|s| *s) {
        return;
    }
    let (bmin, bmax) = (Vec3::from(lvl.bounds_min), Vec3::from(lvl.bounds_max));
    if bmin.x > bmax.x {
        return;
    }
    let centre = (bmin + bmax) * 0.5;
    let half = ((bmax.x - bmin.x).max(bmax.z - bmin.z) * 0.5).max(60.0);
    let noise = st.noise.get_or_insert_with(|| images.add(noise_image())).clone();
    let glow = match crate::skyline::city(&lvl.name) {
        Some(crate::skyline::City::NewYork) | Some(crate::skyline::City::HongKong) => Vec4::new(0.30, 0.20, 0.13, 0.2),
        Some(crate::skyline::City::Paris) => Vec4::new(0.24, 0.19, 0.15, 0.24),
        None => Vec4::new(0.12, 0.13, 0.16, 0.2),
    };
    let moon = crate::shafts::moon_dir(lvl).filter(|d| d.y > 0.05).unwrap_or(Vec3::new(0.3, 0.8, 0.5).normalize());
    let base = bmax.y + (half * 1.2).clamp(150.0, 600.0);
    let thick = 160.0;
    let size = (half * 30.0).clamp(3000.0, 12000.0);
    let mat = mats.add(CloudMaterial {
        params: CloudParams {
            slab: Vec4::new(base, thick, 1.0 / 1800.0, 0.45),
            wind: Vec4::new(0.0035, 0.0012, 0.0014, 1.0),
            glow,
            moon: moon.extend(0.12),
            fade: Vec4::new(centre.x, centre.z, size * 0.35, size * 0.95),
        },
        noise,
    });
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    m.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [centre.x - size, base, centre.z - size],
            [centre.x + size, base, centre.z - size],
            [centre.x + size, base, centre.z + size],
            [centre.x - size, base, centre.z + size],
        ],
    );
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, -1.0, 0.0]; 4]);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; 4]);
    m.insert_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3, 0, 2, 1, 0, 3, 2]));
    commands.spawn((
        Mesh3d(meshes.add(m)),
        MeshMaterial3d(mat),
        Transform::default(),
        CloudSheet,
        bevy::camera::visibility::NoFrustumCulling,
        bevy::light::NotShadowCaster,
        bevy::light::NotShadowReceiver,
    ));
}
