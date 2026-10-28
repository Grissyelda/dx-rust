//! Light coronas: lights with bCorona draw their Skin texture as a glow over the scene at the light, when the
//! light is within 2000 units and nothing in the level blocks the line to it. Size is DrawScale x 0.2 x the
//! screen width; colour is the light's hue/saturation at full brightness x 2.5 (as SurrealEngine draws Deus Ex's).
//! The glow is additive in the original; here the texture is turned into an alpha-blended image with the same
//! look over dark backgrounds (alpha = brightest channel, colour divided by it).

use crate::weapons::RuntimeAssets;
use crate::{FlyCam, LevelRes, SkyCam};
use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use dx_level::{bevy_to_ue, ue_to_bevy};
use glam::DVec3;

#[derive(Component)]
pub struct CoronaSprite(usize);

pub fn spawn_coronas(
    commands: &mut Commands,
    level: &dx_level::Level,
    assets: &mut RuntimeAssets,
    images: &mut Assets<Image>,
) {
    let mut cache: std::collections::HashMap<(String, [u32; 3]), Handle<Image>> =
        std::collections::HashMap::new();
    for (i, c) in level.coronas.iter().enumerate() {
        let key = (c.texture.clone(), c.color.map(|v| (v * 1000.0) as u32));
        let handle = match cache.get(&key) {
            Some(h) => h.clone(),
            None => {
                let Some(t) = assets.lib.load_image(&c.texture) else {
                    continue;
                };
                let (w, h) = (t.width, t.height);
                let src = &t.mips[0];
                let mut out = Vec::with_capacity(src.len());
                for px in src.chunks(4) {
                    // sRGB-encoded texel tinted by the light colour x 2.5.
                    let mut rgb = [0f32; 3];
                    for k in 0..3 {
                        rgb[k] = (px[k] as f32 / 255.0 * c.color[k] * 2.5).min(1.0);
                    }
                    let a = rgb[0].max(rgb[1]).max(rgb[2]);
                    if a <= 1e-3 {
                        out.extend_from_slice(&[0, 0, 0, 0]);
                    } else {
                        out.extend_from_slice(&[
                            (rgb[0] / a * 255.0) as u8,
                            (rgb[1] / a * 255.0) as u8,
                            (rgb[2] / a * 255.0) as u8,
                            (a * 255.0) as u8,
                        ]);
                    }
                }
                let img = Image::new(
                    Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                    TextureDimension::D2,
                    out,
                    TextureFormat::Rgba8UnormSrgb,
                    RenderAssetUsages::RENDER_WORLD,
                );
                let hd = images.add(img);
                cache.insert(key, hd.clone());
                hd
            }
        };
        commands.spawn((
            ImageNode::new(handle),
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            Visibility::Hidden,
            Pickable::IGNORE,
            GlobalZIndex(-10),
            CoronaSprite(i),
        ));
    }
}

pub fn corona_system(
    windows: Query<&Window>,
    cam: Single<(&Camera, &GlobalTransform, &Transform), (With<FlyCam>, Without<SkyCam>)>,
    level: Res<LevelRes>,
    mut q: Query<(&CoronaSprite, &mut Node, &mut Visibility)>,
) {
    let _prof = crate::prof::scope("coronas::corona_system");
    let Some(win) = windows.iter().next() else {
        return;
    };
    let (camera, gt, tf) = *cam;
    let eye = bevy_to_ue(tf.translation.to_array());
    let eye = DVec3::new(eye[0] as f64, eye[1] as f64, eye[2] as f64);
    let width = win.width();
    for (c, mut node, mut vis) in &mut q {
        let co = &level.0.coronas[c.0];
        let p = DVec3::new(co.pos[0] as f64, co.pos[1] as f64, co.pos[2] as f64);
        let mut shown = false;
        if p.distance(eye) < 2000.0 && level.0.collision.trace(p, eye, DVec3::ZERO).fraction >= 1.0
        {
            if let Ok(v) = camera.world_to_viewport(gt, Vec3::from(ue_to_bevy(co.pos))) {
                // In front of the camera only.
                let to = Vec3::from(ue_to_bevy(co.pos)) - tf.translation;
                if to.dot(*tf.forward()) > 0.0 {
                    let size = co.draw_scale * width * 0.2;
                    node.left = Val::Px(v.x - size * 0.5);
                    node.top = Val::Px(v.y - size * 0.5);
                    node.width = Val::Px(size);
                    node.height = Val::Px(size);
                    shown = true;
                }
            }
        }
        *vis = if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}
