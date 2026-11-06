//! Sprint (setting Sprint, an enhancement - not in the original): the old run pace becomes the normal pace and
//! holding Walk (Shift) or clicking the left stick sprints faster, at a stamina cost. Stamina drains while sprinting,
//! comes back slowly after a short rest, and food tops it up. No sprinting crouched, swimming, on a ladder or in the
//! air; crouching keeps its old pace. Emptying it leaves you winded until it's back to WINDED_UNTIL.
//! The HUD shows it as a bar on the right of the body display (classic_hud draw_hit), like the bioenergy bar on the
//! left; the breath bar takes that place under water.

use std::sync::Mutex;

/// Speed while sprinting, times the normal (old run) pace.
pub const SPRINT_MULT: f64 = 1.45;
const MAX: f32 = 100.0;
/// Per second while sprinting (about 7 s from full).
const DRAIN: f32 = 14.0;
/// Per second once rested (about 25 s from empty).
const REGEN: f32 = 4.0;
/// Seconds without sprinting before it starts to come back.
const REST: f32 = 1.4;
/// After running dry, sprinting waits until stamina is back to this.
const WINDED_UNTIL: f32 = 25.0;

struct State {
    value: f32,
    since: f32,
    winded: bool,
    /// The pad's left-stick click: sprint until you stop, crouch or run dry.
    latched: bool,
    pad_prev: bool,
    sprinting: bool,
}

static ST: Mutex<State> = Mutex::new(State { value: MAX, since: 10.0, winded: false, latched: false, pad_prev: false, sprinting: false });

pub fn enabled() -> bool {
    setting!("Sprint") > 0.5
}

/// Stamina 0..1 for the HUD, or None when Sprint is off.
pub fn fraction() -> Option<f32> {
    enabled().then(|| ST.lock().map(|s| s.value / MAX).unwrap_or(1.0))
}

/// The pad's Walk button (left stick click) with Sprint on: a click starts or stops sprinting.
pub fn pad_button(down: bool) {
    if let Ok(mut s) = ST.lock() {
        if down && !s.pad_prev {
            s.latched = !s.latched;
        }
        s.pad_prev = down;
    }
}

/// Food eaten: `amount` of stamina back (before the Glutton perk's multiplier, applied here).
pub fn eat(amount: f32) {
    if !enabled() {
        return;
    }
    if let Ok(mut s) = ST.lock() {
        s.value = (s.value + crate::perks::food(amount)).min(MAX);
        if s.value >= WINDED_UNTIL {
            s.winded = false;
        }
    }
}

/// One frame of the player's movement: `want` = the sprint input held, `can` = moving on foot, standing, on the
/// ground. Returns whether this frame sprints.
pub fn tick(dt: f32, want: bool, can: bool) -> bool {
    let Ok(mut s) = ST.lock() else { return false };
    if !can {
        // Stopping, crouching or leaving the ground ends a stick-click sprint.
        s.latched = false;
    }
    let go = (want || s.latched) && can && !s.winded && s.value > 0.0;
    if go {
        s.value = (s.value - DRAIN * dt).max(0.0);
        s.since = 0.0;
        if s.value <= 0.0 {
            s.winded = true;
            s.latched = false;
        }
    } else {
        s.since += dt;
        if s.since >= REST {
            s.value = (s.value + REGEN * dt).min(MAX);
        }
        if s.winded && s.value >= WINDED_UNTIL {
            s.winded = false;
        }
    }
    s.sprinting = go;
    go
}

pub fn reset_level() {
    if let Ok(mut s) = ST.lock() {
        s.latched = false;
        s.sprinting = false;
    }
}

/// A new game or a load: full again.
pub fn refill() {
    if let Ok(mut s) = ST.lock() {
        s.value = MAX;
        s.winded = false;
        s.latched = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_wind_and_recover() {
        refill();
        // Sprinting drains it in about 7 s, then you're winded.
        let mut t = 0.0;
        while tick(0.1, true, true) {
            t += 0.1;
            assert!(t < 10.0);
        }
        assert!((6.0..8.0).contains(&t), "{t}");
        // Winded: no sprint until it's back to WINDED_UNTIL, which takes the rest plus the regen time.
        let mut t = 0.0;
        while !tick(0.1, true, true) {
            t += 0.1;
            assert!(t < 30.0);
        }
        assert!(t > REST + 5.0, "{t}");
        // Crouched / not moving: never.
        refill();
        assert!(!tick(0.1, true, false));
        // A stick click latches until you stop.
        pad_button(true);
        pad_button(false);
        assert!(tick(0.1, false, true));
        assert!(!tick(0.1, false, false));
        assert!(!tick(0.1, false, true));
    }
}
