//! Remastered Lighting (enhancement, ROADMAP 49-52): the level lit per pixel instead of by its lightmap's texels.
//!
//! * Relit lightmaps (setting RemasteredLighting): each BSP surface's three strongest lights (dx-level's bake
//!   picks them and keeps how much of each reaches every lightmap texel) are worked out per pixel in
//!   shaders/bsp_shadow.wgsl over the surface's relief normal, and their flat baked term is taken back out of the
//!   lightmap: walls and floors gain shape and real falloff while every shadow the editor baked stays where it was
//!   (dropping the bake outright let lamps shine through walls - the maps were lit for Unreal's baker, which
//!   traced its shadows). A global gain and added light (RemasterGain / RemasterAmbient) tune the result.
//! * Relief maps (setting ReliefMaps: 0 off, 1 normal maps, 2 with parallax): dx_level::relief - the Parallax
//!   Occlusion addon's maps where it has them (shipped beside the game, the Parallax Maps mod folder, or the
//!   game's own Textures folder), else generated from the picture for brick, stone, wood, tiles, metal...
//! * Material table (setting MaterialSpecular): roughness, highlight strength and metalness by texture name, so
//!   metal, glass, tiles and wet stone catch the light.
//! * Emissive surfaces (setting EmissiveSurfaces): surfaces drawn full bright (PF_Unlit: lamp fittings, screens,
//!   signs) and named glowing textures go past white, so Bloom makes them glow.
//!
//! All of it is built into the level's materials when the map loads: changing these settings applies on the next
//! map (the Classic / Modern switch reloads the map in place for them).

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageSampler, ImageSamplerDescriptor, ImageFilterMode, ImageAddressMode};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

pub fn on() -> bool {
    crate::settings::get("RemasteredLighting") > 0.5
}

/// 0 off, 1 normal maps, 2 normal + parallax maps (only with remastered lighting: the lightmap alone has no
/// direction to light a normal map from).
pub fn relief_level() -> u8 {
    if !on() {
        return 0;
    }
    crate::settings::get("ReliefMaps").round().clamp(0.0, 2.0) as u8
}

fn specular_on() -> bool {
    on() && crate::settings::get("MaterialSpecular") > 0.5
}

pub fn emissive_on() -> bool {
    crate::settings::get("EmissiveSurfaces") > 0.5
}

/// The settings that are built into the level's materials (a change reloads the map when switched in play).
pub const RELOAD_KEYS: &[&str] = &["RemasteredLighting", "ReliefMaps", "MaterialSpecular", "EmissiveSurfaces", "ModernGlass"];

/// What a surface is made of, by its texture's name ("Package.Group.Name").
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Surface {
    pub roughness: f32,
    /// Highlight strength (0 = matte).
    pub spec: f32,
    pub metal: f32,
    /// Glow (named glowing textures: lamps, screens, signs); 0 = none.
    pub emissive: f32,
}

const MATTE: Surface = Surface { roughness: 0.9, spec: 0.0, metal: 0.0, emissive: 0.0 };

/// The material table.
pub fn surface(name: &str) -> Surface {
    let n = name.to_ascii_lowercase();
    let has = |w: &[&str]| w.iter().any(|x| n.contains(x));
    // Glowing first: light fittings and panels, screens, monitors, neon and lit signs.
    let emissive = if has(&["lightpanel", "lite_", "lights_", "lamp", "neon", "glow", "screen", "monitor", "lcd", "lightbox", "litesign", "exit_sign", "exitsign", "lightfix", "flourescent", "fluorescent"]) {
        1.4
    } else {
        0.0
    };
    let mut s = if has(&["glass", "window", "wndow", "mirror"]) {
        Surface { roughness: 0.08, spec: 1.6, metal: 0.0, emissive: 0.0 }
    } else if has(&["chrome", "steel", "alum", "brass", "copper", "polish"]) {
        Surface { roughness: 0.25, spec: 1.4, metal: 0.9, emissive: 0.0 }
    } else if has(&["metal", "pipe", "grate", "grill", "vent", "plate", "rail", "door", "panel", "rust"]) {
        Surface { roughness: if n.contains("rust") { 0.75 } else { 0.42 }, spec: 0.9, metal: 0.6, emissive: 0.0 }
    } else if has(&["marble", "ceramic", "tile", "porcelain", "toilet", "sink"]) {
        Surface { roughness: 0.22, spec: 1.0, metal: 0.0, emissive: 0.0 }
    } else if has(&["water", "wet", "puddle"]) {
        Surface { roughness: 0.1, spec: 1.4, metal: 0.0, emissive: 0.0 }
    } else if has(&["plastic", "computer", "comp_", "console", "keypad"]) {
        Surface { roughness: 0.35, spec: 0.6, metal: 0.0, emissive: 0.0 }
    } else if has(&["wood", "plank", "board", "floor", "parquet"]) {
        Surface { roughness: 0.6, spec: 0.35, metal: 0.0, emissive: 0.0 }
    } else if has(&["concrete", "asphalt", "road", "stone", "rock", "brick", "cinder", "stucco", "plaster", "earth", "dirt", "grass", "carpet", "fabric", "cloth"]) {
        Surface { roughness: 0.85, spec: 0.12, metal: 0.0, emissive: 0.0 }
    } else {
        Surface { roughness: 0.7, spec: 0.2, metal: 0.0, emissive: 0.0 }
    };
    if !has(&["sky"]) {
        s.emissive = emissive;
    } else {
        s = MATTE;
    }
    s
}

/// The light table (dx-level's `Atlas::lights`): 2 texels per light, Bevy-space position + radius (metres), then
/// Unreal colour x brightness.
pub fn light_image(lights: &[dx_level::lighting::LightInfo]) -> Image {
    let w = (lights.len() * 2).max(2) as u32;
    let mut data: Vec<f32> = Vec::with_capacity(w as usize * 4);
    for l in lights {
        let p = dx_level::ue_to_bevy(l.pos);
        data.extend_from_slice(&[p[0], p[1], p[2], l.radius * dx_level::METERS_PER_UNIT]);
        data.extend_from_slice(&[l.color[0], l.color[1], l.color[2], 0.0]);
    }
    while data.len() < w as usize * 4 {
        data.push(0.0);
    }
    let bytes: Vec<u8> = data.iter().flat_map(|f| f.to_le_bytes()).collect();
    Image::new(Extent3d { width: w, height: 1, depth_or_array_layers: 1 }, TextureDimension::D2, bytes, TextureFormat::Rgba32Float, RenderAssetUsages::RENDER_WORLD)
}

/// The visibility mask atlas (same layout as the lightmap).
pub fn mask_image(a: &dx_level::lighting::Atlas) -> Image {
    let mut img = Image::new(Extent3d { width: a.width, height: a.height, depth_or_array_layers: 1 }, TextureDimension::D2, a.mask.clone(), TextureFormat::Rgba8Unorm, RenderAssetUsages::RENDER_WORLD);
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor { mag_filter: ImageFilterMode::Linear, min_filter: ImageFilterMode::Linear, ..default() });
    img
}

fn mip_image(w: u32, h: u32, mips: &[Vec<u8>]) -> Image {
    let mut data = Vec::new();
    for m in mips {
        data.extend_from_slice(m);
    }
    let mut img = Image::new_uninit(Extent3d { width: w, height: h, depth_or_array_layers: 1 }, TextureDimension::D2, TextureFormat::Rgba8Unorm, RenderAssetUsages::RENDER_WORLD);
    img.texture_descriptor.mip_level_count = mips.len() as u32;
    img.data = Some(data);
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 8,
        ..default()
    });
    img
}

/// Put a level texture's relief maps on its material (normal map; with parallax, the depth map). Returns whether
/// it got a normal map (its meshes then need tangents).
/// How deep relief maps look (Unreal units).
const RELIEF_DEPTH: f32 = 2.5;

pub fn apply_relief(m: &mut StandardMaterial, t: &dx_level::TextureImage, images: &mut Assets<Image>) -> bool {
    if relief_level() == 0 {
        return false;
    }
    apply_relief_maps(m, t, dx_level::relief::maps_for(t, true), images)
}

/// The relief maps of many textures at once, on every core (reading / generating them one by one took seconds
/// of each map's load with HD textures). None when relief maps are off.
pub fn relief_maps_parallel(textures: &[dx_level::TextureImage], wanted: &[usize]) -> std::collections::HashMap<usize, dx_level::relief::Relief> {
    let mut out = std::collections::HashMap::new();
    if relief_level() == 0 || wanted.is_empty() {
        return out;
    }
    let n = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(2, 12);
    let chunk = wanted.len().div_ceil(n).max(1);
    std::thread::scope(|sc| {
        let jobs: Vec<_> = wanted
            .chunks(chunk)
            .map(|c| sc.spawn(move || c.iter().filter_map(|&i| dx_level::relief::maps_for(&textures[i], true).map(|r| (i, r))).collect::<Vec<_>>()))
            .collect();
        for j in jobs {
            out.extend(j.join().unwrap_or_default());
        }
    });
    out
}

/// `apply_relief` with the maps already made.
pub fn apply_relief_maps(m: &mut StandardMaterial, t: &dx_level::TextureImage, r: Option<dx_level::relief::Relief>, images: &mut Assets<Image>) -> bool {
    let lvl = relief_level();
    if lvl == 0 {
        return false;
    }
    let Some(r) = r else { return false };
    m.normal_map_texture = Some(images.add(mip_image(r.width, r.height, &r.normal)));
    if lvl >= 2 {
        if let Some(d) = r.depth.as_ref() {
            m.depth_map = Some(images.add(mip_image(r.width, r.height, d)));
            // Bevy's depth is in texture coordinates, and a BSP texture repeats once per its size in Unreal units
            // (a texel a unit): a fixed 0.035 was 9 units deep on a 256 wall and 36 on a 1024 one - the bricks slid
            // and smeared as you moved. RELIEF_DEPTH units deep on every texture, as the addon's renderer shows it.
            m.parallax_depth_scale = (RELIEF_DEPTH / t.width.max(16) as f32).clamp(0.001, 0.02);
            m.max_parallax_layer_count = 24.0;
            m.parallax_mapping_method = bevy::pbr::ParallaxMappingMethod::Relief { max_steps: 5 };
        }
    }
    true
}

/// The remastered-lighting parameters of a BSP material for its texture.
pub fn params(name: Option<&str>, p: &mut crate::shadowlight::BspShadowParams) {
    let s = name.map(surface).unwrap_or(MATTE);
    let gain = crate::settings::get("RemasterGain");
    let gain = if gain > 0.0 { gain } else { 1.0 };
    p.r = Vec4::new(if on() { 1.0 } else { 0.0 }, if specular_on() { s.spec } else { 0.0 }, s.roughness, s.metal);
    p.s = Vec4::new(
        if on() { gain } else { 1.0 },
        if on() { crate::settings::get("RemasterAmbient").max(0.0) } else { 0.0 },
        if emissive_on() { s.emissive } else { 0.0 },
        if emissive_on() { 1.2 } else { 0.0 },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table() {
        assert!(surface("CoreTexMetal.Metal.ClenBrwnRwire_A").metal > 0.0);
        assert!(surface("CoreTexGlass.Glass.ClearGlass").roughness < 0.2);
        assert_eq!(surface("CoreTexBrick.Brick.DrtyReddBrick_A").metal, 0.0);
        assert!(surface("UNATCO.Lights.LightPanel_A").emissive > 0.0);
    }
}
