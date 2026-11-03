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
    /// Game time something drawing it was last on screen (a loop nobody sees doesn't step).
    seen: f32,
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
        if env_on!("DXVIEW_ANIMLOG") {
            println!("ANIMTEX {path}: {} frames, {:.3} s each", frames.len(), step);
        }
        a.push(Anim { handle: handle.clone(), frames, step, t: 0.0, at: 0, level, mats: None, seen: 0.0 });
    }
}

/// The images that loop (gpudrive.rs keeps the surfaces showing them CPU-culled).
pub fn images() -> Vec<AssetId<Image>> {
    ANIMS.lock().map(|l| l.iter().map(|a| a.handle.id()).collect()).unwrap_or_default()
}

#[allow(clippy::type_complexity)]
pub fn anim_tex_system(
    time: Res<Time>,
    mut images: ResMut<Assets<Image>>,
    mut bsp: ResMut<Assets<crate::shadowlight::BspMaterial>>,
    mut std_mats: ResMut<Assets<StandardMaterial>>,
    drawn: Query<(&ViewVisibility, Option<&MeshMaterial3d<StandardMaterial>>, Option<&MeshMaterial3d<crate::shadowlight::BspMaterial>>), With<Mesh3d>>,
    mut visible: Local<(std::collections::HashSet<AssetId<StandardMaterial>>, std::collections::HashSet<AssetId<crate::shadowlight::BspMaterial>>)>,
) {
    let _prof = crate::prof::scope("anim_tex::anim_tex_system");
    let dt = time.delta_secs().min(0.25);
    let now = time.elapsed_secs();
    let Ok(mut list) = ANIMS.lock() else { return };
    if list.is_empty() {
        return;
    }
    // The materials something on screen draws this frame.
    visible.0.clear();
    visible.1.clear();
    for (v, sm, bm) in &drawn {
        if !v.get() {
            continue;
        }
        if let Some(m) = sm {
            visible.0.insert(m.0.id());
        }
        if let Some(m) = bm {
            visible.1.insert(m.0.id());
        }
    }
    for a in list.iter_mut() {
        if a.mats.is_none() {
            let id = a.handle.id();
            let b: Vec<_> = bsp.iter().filter(|(_, m)| m.base.base_color_texture.as_ref().is_some_and(|h| h.id() == id)).map(|(i, _)| i).collect();
            let s: Vec<_> = std_mats.iter().filter(|(_, m)| m.base_color_texture.as_ref().is_some_and(|h| h.id() == id)).map(|(i, _)| i).collect();
            a.mats = Some((b, s));
            a.seen = now;
        }
        if let Some((b, s)) = &a.mats {
            if b.iter().any(|i| visible.1.contains(i)) || s.iter().any(|i| visible.0.contains(i)) {
                a.seen = now;
            }
        }
        if env_on!("DXVIEW_ANIMLOG") && (now * 2.0).floor() != ((now - dt) * 2.0).floor() {
            println!("ANIMTEX step {:?} at {} seen {:.1} s ago mats {:?}", a.handle.id(), a.at, now - a.seen, a.mats.as_ref().map(|m| (m.0.len(), m.1.len())));
        }
        // Out of sight for half a second: the loop pauses (nothing would show the new picture).
        if now - a.seen > 0.5 {
            continue;
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
            // A copy of the frame goes in: the image is a render-world asset, so the copy's pixels are taken
            // at upload (moving the frames themselves in, as before, left every frame empty after one loop and the
            // screens stood still).
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
