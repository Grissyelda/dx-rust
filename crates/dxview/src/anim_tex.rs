//! Animated textures (Texture.AnimNext): a texture names the next frame of a loop, and the engine steps through
//! the chain at MaxFrameRate (UTexture::Tick; 0 = a new frame every rendered frame). The med lab's heart-rate
//! monitors, the news and observation screens, the TV noise are all such loops. Each animating texture's image
//! asset gets the next frame's pixels when its time comes, so every surface, mesh and mover using it follows.

use bevy::prelude::*;
use std::sync::Mutex;

struct Anim {
    handle: Handle<Image>,
    frames: Vec<Image>,
    /// Seconds per frame.
    step: f32,
    t: f32,
    at: usize,
    /// Belongs to the current level's textures (dropped with it); runtime-loaded images stay.
    level: bool,
    /// The materials drawing it (found once): touched with each new frame - a bindless material keeps the texture
    /// view it was prepared with, so with the level's surfaces on bindless BspMaterials the swapped picture never
    /// reached the screen (New Vision's monitors stood still).
    mats: Option<(Vec<AssetId<crate::shadowlight::BspMaterial>>, Vec<AssetId<StandardMaterial>>)>,
}

static ANIMS: Mutex<Vec<Anim>> = Mutex::new(Vec::new());

/// A new level: forget the old one's loops.
pub fn reset() {
    if let Ok(mut a) = ANIMS.lock() {
        a.retain(|x| !x.level);
    }
}

/// If the texture at `path` animates, load its other frames and drive `handle` through them. `masked`: decode
/// palette index 0 as see-through, like the first frame was.
pub fn register(lib: &mut dx_level::TexLib, path: &str, handle: &Handle<Image>, masked: bool, level: bool) {
    let Some((chain, _min, max)) = lib.anim_chain(path) else { return };
    let mut frames = Vec::new();
    for p in &chain {
        let img = if masked {
            lib.load_image_masked(p)
        } else {
            lib.load(p).map(|(t, mips)| dx_level::TextureImage { name: p.clone(), width: t.width(), height: t.height(), mips, bc1: false, bc3: false })
        };
        match img {
            Some(i) => frames.push(crate::make_image(&i)),
            None => return,
        }
    }
    let step = if max > 0.0 { 1.0 / max.clamp(0.01, 100.0) } else { 1.0 / 60.0 };
    if let Ok(mut a) = ANIMS.lock() {
        if a.iter().any(|x| x.handle == *handle) {
            return;
        }
        a.push(Anim { handle: handle.clone(), frames, step, t: 0.0, at: 0, level, mats: None });
    }
}

pub fn anim_tex_system(
    time: Res<Time>,
    mut images: ResMut<Assets<Image>>,
    mut bsp: ResMut<Assets<crate::shadowlight::BspMaterial>>,
    mut std_mats: ResMut<Assets<StandardMaterial>>,
) {
    let dt = time.delta_secs().min(0.25);
    let Ok(mut list) = ANIMS.lock() else { return };
    for a in list.iter_mut() {
        if a.mats.is_none() {
            let id = a.handle.id();
            let b: Vec<_> = bsp.iter().filter(|(_, m)| m.base.base_color_texture.as_ref().is_some_and(|h| h.id() == id)).map(|(i, _)| i).collect();
            let s: Vec<_> = std_mats.iter().filter(|(_, m)| m.base_color_texture.as_ref().is_some_and(|h| h.id() == id)).map(|(i, _)| i).collect();
            a.mats = Some((b, s));
        }
        a.t += dt;
        if a.t < a.step {
            continue;
        }
        while a.t >= a.step {
            a.t -= a.step;
            a.at = (a.at + 1) % a.frames.len();
        }
        let mut reshaped = false;
        if let Some(mut im) = images.get_mut(&a.handle) {
            let next = &a.frames[a.at];
            reshaped = im.texture_descriptor.size != next.texture_descriptor.size
                || im.texture_descriptor.format != next.texture_descriptor.format
                || im.texture_descriptor.mip_level_count != next.texture_descriptor.mip_level_count;
            *im = next.clone();
        }
        // Same size and format: Bevy writes the new pixels into the same texture. A different one (the first
        // frame of a New Vision texture is its HD picture, the loop's frames are the original's) makes a new texture,
        // and the materials have to pick it up.
        if !reshaped {
            continue;
        }
        if let Some((b, s)) = &a.mats {
            for id in b {
                let _ = bsp.get_mut(*id);
            }
            for id in s {
                let _ = std_mats.get_mut(*id);
            }
        }
    }
}
