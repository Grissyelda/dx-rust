//! Frame Rate Cap (setting FPSLimit, Settings > Enhancements > Display): at the end of each frame the main thread
//! waits out what's left of the frame's time at the chosen rate. Bevy's renderer runs a frame behind on its own
//! thread, so holding the main loop back holds the whole pipeline to the cap. Sleeps until a millisecond and a half
//! before the deadline, then spins the rest (OS sleeps overshoot by about a millisecond).

use bevy::prelude::*;
use std::time::{Duration, Instant};

/// The choices in the menu, by setting value.
pub const LABELS: [&str; 7] = ["Off", "30", "60", "120", "144", "200", "240"];
const RATES: [f64; 7] = [0.0, 30.0, 60.0, 120.0, 144.0, 200.0, 240.0];

/// The cap in frames a second, or None.
pub fn rate() -> Option<f64> {
    let i = setting!("FPSLimit").round().clamp(0.0, (RATES.len() - 1) as f32) as usize;
    (RATES[i] > 0.0).then_some(RATES[i])
}

/// Last: wait for the frame's slot. `next` = when the next frame may start.
pub fn frame_cap_system(mut next: Local<Option<Instant>>) {
    let Some(fps) = rate() else {
        *next = None;
        return;
    };
    let period = Duration::from_secs_f64(1.0 / fps);
    let now = Instant::now();
    let due = match *next {
        // Far behind (a load, a hitch): start counting again from now instead of racing to catch up.
        Some(t) if t + period * 2 > now => t,
        _ => now,
    };
    if due > now {
        let left = due - now;
        if left > Duration::from_micros(1500) {
            std::thread::sleep(left - Duration::from_micros(1500));
        }
        while Instant::now() < due {
            std::hint::spin_loop();
        }
    }
    *next = Some(due.max(now) + period);
}
