//! Modern Glass (setting ModernGlass, in the Modern preset): window glass (translucent faces textured from a
//! "Glass" texture group: CoreTexGlass.Glass.*, NYCBar.Glass.*) drawn as frosted glass instead of Unreal's pure
//! additive blend. Unreal adds the streak picture to what's behind, so a dark streak texture (WindOpacStrek_A on the
//! UNATCO HQ doors) all but vanishes and the window reads as an empty hole. Here the same picture becomes a blended
//! layer: a faint pale-blue frosting everywhere (so the pane is there) with the streaks brighter and more opaque.
//! Built once per map load (relight::RELOAD_KEYS has the key, so the Classic / Modern switch reloads).

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::collections::HashMap;

pub fn enabled() -> bool {
    setting!("ModernGlass") > 0.5
}

/// A window-glass texture by its "Package.Group.Name".
pub fn is_glass(name: &str) -> bool {
    crate::util::contains_ci(name, ".Glass.")
}

/// The frosted copies of the level's glass textures (texture index -> image), for the textures in `wanted`.
pub fn images(level: &dx_level::Level, wanted: impl Iterator<Item = usize>, images: &mut Assets<Image>, handles: &[Handle<Image>]) -> HashMap<usize, Handle<Image>> {
    let mut out = HashMap::new();
    if !enabled() {
        return out;
    }
    for i in wanted {
        if out.contains_key(&i) || !level.textures.get(i).is_some_and(|t| is_glass(&t.name)) {
            continue;
        }
        let Some(src) = handles.get(i).and_then(|h| images.get(h)) else { continue };
        if let Some(img) = frost(src) {
            out.insert(i, images.add(img));
        }
    }
    out
}

/// Mip 0 of an RGBA8 image made into frosted glass, with its own mip chain.
fn frost(src: &Image) -> Option<Image> {
    if !matches!(src.texture_descriptor.format, TextureFormat::Rgba8UnormSrgb | TextureFormat::Rgba8Unorm) {
        return None;
    }
    let (w, h) = (src.texture_descriptor.size.width as usize, src.texture_descriptor.size.height as usize);
    let data = src.data.as_ref()?;
    if data.len() < w * h * 4 || w == 0 || h == 0 {
        return None;
    }
    let mut top = Vec::with_capacity(w * h * 4);
    for p in data[..w * h * 4].chunks_exact(4) {
        let (r, g, b) = (p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0);
        let lum = 0.299 * r + 0.587 * g + 0.114 * b;
        // Frost: a pale blue-grey veil; streaks: the picture itself, lifted.
        let k = (lum * 1.6).min(1.0);
        let mix = |c: f32, tint: f32| ((tint * (1.0 - k) + (c * 1.3 + 0.15).min(1.0) * k) * 255.0).round() as u8;
        let a = (0.22 + 0.6 * k).min(0.85);
        top.extend_from_slice(&[mix(r, 0.74), mix(g, 0.82), mix(b, 0.88), (a * 255.0).round() as u8]);
    }
    // Box-filtered mips (sRGB treated as linear: close enough for a soft veil).
    let mut all = top.clone();
    let (mut cw, mut ch, mut cur) = (w, h, top);
    let mut levels = 1;
    while cw > 1 || ch > 1 {
        let (nw, nh) = ((cw / 2).max(1), (ch / 2).max(1));
        let mut next = vec![0u8; nw * nh * 4];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let mut s = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let (sx, sy) = ((x * 2 + dx).min(cw - 1), (y * 2 + dy).min(ch - 1));
                        s += cur[(sy * cw + sx) * 4 + c] as u32;
                    }
                    next[(y * nw + x) * 4 + c] = (s / 4) as u8;
                }
            }
        }
        all.extend_from_slice(&next);
        cur = next;
        cw = nw;
        ch = nh;
        levels += 1;
    }
    let mut img = Image::new(
        Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        all,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.texture_descriptor.mip_level_count = levels;
    img.sampler = ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
        address_mode_u: bevy::image::ImageAddressMode::Repeat,
        address_mode_v: bevy::image::ImageAddressMode::Repeat,
        mag_filter: bevy::image::ImageFilterMode::Linear,
        min_filter: bevy::image::ImageFilterMode::Linear,
        mipmap_filter: bevy::image::ImageFilterMode::Linear,
        ..default()
    });
    Some(img)
}

/// The material of a frosted pane: blended, seen from both sides, unlit (Unreal's glass is unlit too).
pub fn material(materials: &mut Assets<StandardMaterial>, image: Handle<Image>) -> Handle<StandardMaterial> {
    materials.add(StandardMaterial {
        base_color_texture: Some(image),
        base_color: Color::WHITE,
        unlit: true,
        double_sided: true,
        cull_mode: None,
        alpha_mode: AlphaMode::Blend,
        ..default()
    })
}
