//! FPS Counter (Settings > Enhancements > Performance, off by default): frames a second and the average frame time
//! in the top-right corner, over each half second of real time, in the game's own bitmap font (classic_hud.rs draws
//! it). Ours.

use bevy::prelude::*;

/// The counter's text while it's on.
static TEXT: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub fn text() -> Option<String> {
    TEXT.lock().ok().and_then(|g| g.clone())
}

/// Updates the text when half a second has passed (and clears it when the setting is off).
pub fn fps_system(time: Res<Time<Real>>, mut window: Local<(f32, u32)>) {
    let on = crate::perf::fps_counter();
    if !on {
        *window = (0.0, 0);
        if let Ok(mut g) = TEXT.lock() {
            if g.is_some() {
                *g = None;
            }
        }
        return;
    }
    window.0 += time.delta_secs();
    window.1 += 1;
    if window.0 >= 0.5 {
        let (secs, frames) = *window;
        if let Ok(mut g) = TEXT.lock() {
            *g = Some(format!("{:.0} fps  {:.1} ms", frames as f32 / secs, 1000.0 * secs / frames as f32));
        }
        *window = (0.0, 0);
    } else if let Ok(mut g) = TEXT.lock() {
        if g.is_none() {
            *g = Some(String::new());
        }
    }
}
