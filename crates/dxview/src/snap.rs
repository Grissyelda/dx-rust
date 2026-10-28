//! Controller snap-to for the free-cursor screens (ours): each screen offers its clickable widgets every frame
//! (in its own UI units with its scale and offset); a D-pad press moves the cursor to the centre of the nearest
//! widget in that direction (hover then focuses it, A clicks it). The topmost layer that offered anything wins
//! (menus over computers over the inventory / persona screens). With nothing offered, the D-pad stays arrow keys
//! (conversation choices, the keypad's own keys).

use bevy::math::Vec2;
use std::cell::Cell;
use std::sync::Mutex;

pub const LAYER_PERSONA: u8 = 1;
pub const LAYER_COMPUTER: u8 = 2;
pub const LAYER_MENU: u8 = 3;

/// UI units -> logical window pixels: physical = unit * s + offset; logical = physical / scale factor.
#[derive(Clone, Copy)]
pub struct Map {
    /// The screen's shown scale (fractional with HUD Scale Large).
    pub s: f32,
    pub ox: i32,
    pub oy: i32,
    pub sf: f32,
}

/// Rects offered this frame (layer, logical x0 y0 x1 y1) and the ones from the last complete frame.
static NEXT: Mutex<Vec<(u8, [f32; 4])>> = Mutex::new(Vec::new());
static CUR: Mutex<Vec<[f32; 4]>> = Mutex::new(Vec::new());

thread_local! {
    /// The inventory screen's mapping while it calls the persona pages (same system, same thread).
    static PAGE_MAP: Cell<Option<Map>> = const { Cell::new(None) };
}

pub fn set_page_map(m: Option<Map>) {
    PAGE_MAP.with(|c| c.set(m));
}

pub fn page_map() -> Option<Map> {
    PAGE_MAP.with(|c| c.get())
}

/// Offer clickable rects (x, y, w, h in UI units).
pub fn offer(layer: u8, m: Map, rects: impl IntoIterator<Item = (i32, i32, i32, i32)>) {
    let mut g = NEXT.lock().unwrap();
    for (x, y, w, h) in rects {
        if w <= 0 || h <= 0 {
            continue;
        }
        let px = |u: i32, o: i32| (u as f32 * m.s + o as f32) / m.sf;
        g.push((layer, [px(x, m.ox), px(y, m.oy), px(x + w, m.ox), px(y + h, m.oy)]));
    }
}

/// PreUpdate (keybinds): last frame's offers become the targets; the topmost layer only.
pub fn swap_frame() -> bool {
    let mut n = NEXT.lock().unwrap();
    let top = n.iter().map(|r| r.0).max().unwrap_or(0);
    let cur: Vec<[f32; 4]> = n.iter().filter(|r| r.0 == top).map(|r| r.1).collect();
    n.clear();
    let any = !cur.is_empty();
    *CUR.lock().unwrap() = cur;
    any
}

/// The centre of the widget to snap to from `at` going `dir` (unit, screen y down).
pub fn pick(at: Vec2, dir: Vec2) -> Option<Vec2> {
    let rects = CUR.lock().unwrap().clone();
    pick_in(&rects, at, dir)
}

pub fn pick_in(rects: &[[f32; 4]], at: Vec2, dir: Vec2) -> Option<Vec2> {
    let centre = |r: &[f32; 4]| Vec2::new((r[0] + r[2]) * 0.5, (r[1] + r[3]) * 0.5);
    let inside = |r: &[f32; 4]| at.x >= r[0] && at.x < r[2] && at.y >= r[1] && at.y < r[3];
    // From inside a widget, measure from its centre (so a wide button doesn't skew the direction) and skip
    // widgets overlapping it along the move.
    let from = rects.iter().filter(|r| inside(r)).last();
    let origin = from.map(centre).unwrap_or(at);
    let mut best: Option<(f32, Vec2)> = None;
    for r in rects {
        if from.is_some_and(|f| f == r) {
            continue;
        }
        let c = centre(r);
        // Distance to the rect's nearest edge along the direction (rows of wide buttons: the next row, not
        // the far end of this one).
        let d = c - origin;
        let primary = d.dot(dir);
        if primary <= 2.0 {
            continue;
        }
        let perp = (d - dir * primary).length();
        // Mostly within a cone, then near; things far off to the side cost more.
        let score = primary + perp * 2.5;
        if best.is_none_or(|b| score < b.0) {
            best = Some((score, c));
        }
    }
    if best.is_none() && from.is_none() {
        // Not on a widget and nothing that way: the nearest one at all.
        best = rects.iter().map(|r| ((centre(r) - at).length(), centre(r))).min_by(|a, b| a.0.total_cmp(&b.0));
    }
    best.map(|b| b.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_by_direction() {
        // A column of three buttons and one off to the right of the middle.
        let r = [[0.0, 0.0, 100.0, 20.0], [0.0, 40.0, 100.0, 60.0], [0.0, 80.0, 100.0, 100.0], [200.0, 40.0, 260.0, 60.0]];
        let at = Vec2::new(50.0, 50.0);
        assert_eq!(pick_in(&r, at, Vec2::Y), Some(Vec2::new(50.0, 90.0)));
        assert_eq!(pick_in(&r, at, -Vec2::Y), Some(Vec2::new(50.0, 10.0)));
        assert_eq!(pick_in(&r, at, Vec2::X), Some(Vec2::new(230.0, 50.0)));
        assert_eq!(pick_in(&r, at, -Vec2::X), None);
        // Off every widget with nothing that way: the nearest.
        assert_eq!(pick_in(&r, Vec2::new(500.0, 500.0), Vec2::X), Some(Vec2::new(230.0, 50.0)));
    }
}
