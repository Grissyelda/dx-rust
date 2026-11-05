//! Performance switches (Settings > Enhancements > Performance). Their defaults keep the PC's picture and
//! behaviour. Systems read them every frame, so they are cached here and read again only when the settings change
//! (`settings::generation`).

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering::Relaxed};

const MSAA: u8 = 1;
const HDR: u8 = 2;
const LEAN_RENDERING: u8 = 4;
const LEAN_UPDATES: u8 = 8;

/// The settings generation `FLAGS` was read at.
static READ_AT: AtomicU64 = AtomicU64::new(u64::MAX);
static FLAGS: AtomicU8 = AtomicU8::new(0);

fn flags() -> u8 {
    let generation = crate::settings::generation();
    if READ_AT.load(Relaxed) != generation {
        let on = |key: &str, bit: u8| if crate::settings::get(key) > 0.5 { bit } else { 0 };
        let flags = on("AntiAliasing", MSAA) | on("HDRRendering", HDR) | on("LeanRendering", LEAN_RENDERING)
            | on("LeanUpdates", LEAN_UPDATES);
        FLAGS.store(flags, Relaxed);
        READ_AT.store(generation, Relaxed);
    }
    FLAGS.load(Relaxed)
}

/// 4x MSAA on the cameras that draw into the window while SSAO and Upscaling are off (setting AntiAliasing). Off: no
/// MSAA.
pub fn msaa() -> bool {
    flags() & MSAA != 0
}

/// HDR (16-bit float) render targets (setting HDRRendering). Off: 8-bit targets, unless Bloom wants HDR.
pub fn hdr() -> bool {
    flags() & HDR != 0
}

/// Skip GPU work that adds no pixels (setting LeanRendering).
pub fn lean_rendering() -> bool {
    flags() & LEAN_RENDERING != 0
}

/// Skip CPU work that changes nothing (setting LeanUpdates).
pub fn lean_updates() -> bool {
    flags() & LEAN_UPDATES != 0
}
