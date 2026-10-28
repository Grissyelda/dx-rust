//! Volumetric lights (Actor.VolumeRadius / VolumeBrightness / VolumeFog): a glowing ball of participating media
//! around the light - the blue glow behind the DX logo (the ElectricityEmitter under the globe). Drawn the way
//! SurrealEngine's FogmapBuilder lights it: colour hsbtorgb(LightHue, LightSaturation, LightBrightness), strength
//! LightBrightness/255 x VolumeBrightness/64 x 5, radius (VolumeRadius + 1) x 25, density integrated through a
//! sphere whose media falls off quadratically from the centre - which, seen from outside, is (1 - b^2)^1.5 across
//! the ball (b = distance from the centre line / radius). An additive camera-facing disc stands in for the per-pixel
//! integral (it's exact for views from outside the ball, where every one of these lights is seen).

use crate::LevelRes;
use bevy::prelude::*;
use dx_level::ue_to_bevy;
use unreal_pkg::props::{find, Value};

#[derive(Component)]
pub struct FogBall;

fn byte(p: &[unreal_pkg::props::Property], n: &str, d: u8) -> u8 {
    match find(p, n) {
        Some(Value::Byte(b)) => *b,
        _ => d,
    }
}

/// SurrealEngine's hsbtorgb (Math/hsb.h), v = 6.512735 x sqrt(brightness).
pub fn hsbtorgb(hue: u8, sat: u8, bright: u8) -> [f32; 3] {
    let v = 6.512735 * (bright as f32).sqrt();
    if sat >= 250 {
        return [v / 255.0; 3];
    }
    if bright == 0 {
        return [0.0; 3];
    }
    let mut s = sat as f32 / 2.5;
    if s > 32.0 {
        s += 2.0;
    }
    let pos = hue as f32 / 85.0;
    let frac = pos - pos.floor();
    let p = s * v / 104.0;
    let q = (1.0 - frac) * v + p * frac;
    let t = frac * v + p * (1.0 - frac);
    let (r, g, b) = if hue < 85 {
        (q, t, p)
    } else if hue < 170 {
        (p, q, t)
    } else {
        (t, p, q)
    };
    [r / 255.0, g / 255.0, b / 255.0]
}

fn ball_image() -> Image {
    let n = 128u32;
    let mut px = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let dx = (x as f32 + 0.5) / n as f32 * 2.0 - 1.0;
            let dy = (y as f32 + 0.5) / n as f32 * 2.0 - 1.0;
            let b2 = (dx * dx + dy * dy).min(1.0);
            let v = ((1.0 - b2).powf(1.5) * 255.0) as u8;
            px.extend_from_slice(&[v, v, v, 255]);
        }
    }
    Image::new(
        bevy::render::render_resource::Extent3d { width: n, height: n, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        px,
        bevy::render::render_resource::TextureFormat::Rgba8Unorm,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn fogball_system(
    mut commands: Commands,
    level: Res<LevelRes>,
    cam: Single<&Transform, (With<crate::FlyCam>, Without<crate::SkyCam>, Without<FogBall>)>,
    mut balls: Query<&mut Transform, With<FogBall>>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut map_gen: Local<u64>,
) {
    if crate::loader::new_map(&mut map_gen) {
        let mut img: Option<Handle<Image>> = None;
        let quad = meshes.add(Rectangle::new(1.0, 1.0));
        for a in &level.0.actors {
            let p = &a.props;
            let vr = byte(p, "VolumeRadius", 0);
            let vb = byte(p, "VolumeBrightness", 64);
            let lb = byte(p, "LightBrightness", 64);
            if vr == 0 || vb == 0 || lb == 0 {
                continue;
            }
            let c = hsbtorgb(byte(p, "LightHue", 0), byte(p, "LightSaturation", 255), lb);
            let k = lb as f32 / 255.0 * vb as f32 / 64.0 * 5.0;
            let radius = (vr as f32 + 1.0) * 25.0;
            let tex = img.get_or_insert_with(|| images.add(ball_image())).clone();
            let mat = materials.add(StandardMaterial {
                base_color: Color::linear_rgb(c[0] * k, c[1] * k, c[2] * k),
                base_color_texture: Some(tex),
                unlit: true,
                double_sided: true,
                cull_mode: None,
                alpha_mode: AlphaMode::Add,
                fog_enabled: false,
                ..default()
            });
            let l = a.location();
            commands.spawn((
                FogBall,
                Mesh3d(quad.clone()),
                MeshMaterial3d(mat),
                Transform::from_translation(Vec3::from(ue_to_bevy(l)))
                    .with_scale(Vec3::splat(2.0 * radius * dx_level::METERS_PER_UNIT)),
                bevy::light::NotShadowCaster,
            ));
        }
    }
    for mut t in &mut balls {
        t.rotation = cam.rotation;
    }
}
