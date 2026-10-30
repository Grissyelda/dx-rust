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

/// Enhancement "Modern Coronas" (setting ModernCoronas, off by default): the lights' corona pictures replaced by a
/// drawn glow - a bright core, a wide soft halo, a faint horizontal streak and four thin diffraction spikes, in the
/// light's colour - that fades in and out over a moment instead of popping when something passes in front.
pub fn modern() -> bool {
    crate::settings::get("ModernCoronas") > 0.5
}

/// The modern glow picture for a light colour (sRGB, straight alpha).
fn modern_glow(color: [f32; 3]) -> Image {
    const N: usize = 256;
    let mut out = Vec::with_capacity(N * N * 4);
    let m = color[0].max(color[1]).max(color[2]).max(1e-3);
    let tint = [color[0] / m, color[1] / m, color[2] / m];
    for y in 0..N {
        for x in 0..N {
            let u = (x as f32 + 0.5) / N as f32 * 2.0 - 1.0;
            let v = (y as f32 + 0.5) / N as f32 * 2.0 - 1.0;
            let r2 = u * u + v * v;
            let core = (-r2 / 0.006).exp();
            let halo = (-r2 / 0.16).exp() * 0.5 * (1.0 - r2.sqrt()).max(0.0);
            let streak = (-(v * v) / 0.0012).exp() * (-(u.abs()) * 2.6).exp() * 0.45;
            let spike = {
                let a = u.abs().min(v.abs());
                let b = ((u - v).abs() / 1.414).min((u + v).abs() / 1.414);
                let s1 = (-(a * a) / 0.00025).exp() * (-r2.sqrt() * 3.2).exp();
                let s2 = (-(b * b) / 0.00025).exp() * (-r2.sqrt() * 4.5).exp();
                (s1 * 0.5 + s2 * 0.25).min(1.0)
            };
            let glow = halo + streak + spike;
            let a = (core + glow).min(1.0);
            // The core whitens; the halo keeps the light's colour.
            let w = core.min(1.0);
            let rgb = [tint[0] + (1.0 - tint[0]) * w, tint[1] + (1.0 - tint[1]) * w, tint[2] + (1.0 - tint[2]) * w];
            out.extend_from_slice(&[(rgb[0] * 255.0) as u8, (rgb[1] * 255.0) as u8, (rgb[2] * 255.0) as u8, (a * 255.0) as u8]);
        }
    }
    Image::new(
        Extent3d { width: N as u32, height: N as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        out,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

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
        let modern = modern();
        let key = if modern { (String::from("#modern"), key.1) } else { key };
        let handle = match cache.get(&key) {
            Some(h) => h.clone(),
            None if modern => {
                let hd = images.add(modern_glow(c.color));
                cache.insert(key, hd.clone());
                hd
            }
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

#[allow(clippy::too_many_arguments)]
pub fn corona_system(
    mut commands: Commands,
    windows: Query<&Window>,
    time: Res<Time>,
    cam: Single<(&Camera, &GlobalTransform, &Transform), (With<FlyCam>, Without<SkyCam>)>,
    level: Res<LevelRes>,
    mut q: Query<(Entity, &CoronaSprite, &mut Node, &mut Visibility, &mut ImageNode)>,
    mut fade: Local<std::collections::HashMap<usize, f32>>,
    mut built: Local<Option<bool>>,
    mut map_gen: Local<u64>,
    mut assets: ResMut<RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
) {
    let _prof = crate::prof::scope("coronas::corona_system");
    if crate::loader::new_map(&mut map_gen) {
        *built = Some(modern());
        fade.clear();
    }
    // The setting changed: the sprites are rebuilt with the other pictures.
    if *built != Some(modern()) {
        *built = Some(modern());
        for (e, ..) in &q {
            commands.entity(e).despawn();
        }
        spawn_coronas(&mut commands, &level.0, &mut assets, &mut images);
        return;
    }
    let Some(win) = windows.iter().next() else {
        return;
    };
    let (camera, gt, tf) = *cam;
    let eye = bevy_to_ue(tf.translation.to_array());
    let eye = DVec3::new(eye[0] as f64, eye[1] as f64, eye[2] as f64);
    let width = win.width();
    let modern = modern();
    let dt = time.delta_secs().min(0.1);
    for (_, c, mut node, mut vis, mut img) in &mut q {
        let co = &level.0.coronas[c.0];
        let p = DVec3::new(co.pos[0] as f64, co.pos[1] as f64, co.pos[2] as f64);
        let mut shown = false;
        let mut strength = 1.0f32;
        if p.distance(eye) < 2000.0 && level.0.collision.trace(p, eye, DVec3::ZERO).fraction >= 1.0
        {
            if let Ok(v) = camera.world_to_viewport(gt, Vec3::from(ue_to_bevy(co.pos))) {
                // In front of the camera only.
                let to = Vec3::from(ue_to_bevy(co.pos)) - tf.translation;
                let cosv = to.normalize_or_zero().dot(*tf.forward());
                if cosv > 0.0 {
                    // Modern: a larger, softer glow that brightens as the view turns onto it.
                    let mut size = co.draw_scale * width * if modern { 0.3 } else { 0.2 };
                    if modern {
                        strength = 0.55 + 0.45 * cosv * cosv;
                        size *= 0.8 + 0.2 * cosv;
                    }
                    node.left = Val::Px(v.x - size * 0.5);
                    node.top = Val::Px(v.y - size * 0.5);
                    node.width = Val::Px(size);
                    node.height = Val::Px(size);
                    shown = true;
                }
            }
        }
        if modern {
            // Ease in and out over about 0.15 s instead of popping.
            let f = fade.entry(c.0).or_insert(0.0);
            let want = if shown { strength } else { 0.0 };
            *f += (want - *f) * (dt * 8.0).min(1.0);
            let a = *f;
            img.color = Color::srgba(1.0, 1.0, 1.0, a);
            *vis = if a > 0.01 { Visibility::Inherited } else { Visibility::Hidden };
        } else {
            if img.color != Color::WHITE {
                img.color = Color::WHITE;
            }
            *vis = if shown {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
        }
    }
}
