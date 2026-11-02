//! Ours: the controller layout picture - a gamepad drawn in the menu theme with every button's current binding
//! (gamepad.rs PAD_ACTIONS + padbinds.txt) labelled round it. Settings > Enhancements > Controller: Controller
//! Layout, and once on the first map (Liberty Island) for a player who is using a pad and hasn't seen it
//! (setting PadLayoutShown).

use crate::gamepad::{PadBtn, PadKind, PAD_ACTIONS};
use crate::menuui::{m_col, KitWid, Ui};
use crate::uicanvas::{Canvas, HAlign, Rgb, Style, VAlign};

/// The picture's size (the message box's client area), in UI units.
pub const W: i32 = 600;
pub const H: i32 = 300;

/// Where each button sits on the drawn pad (centre, UI units from the client's top-left).
fn anchor(b: PadBtn) -> (i32, i32) {
    match b {
        PadBtn::LT => (228, 70),
        PadBtn::LB => (240, 86),
        PadBtn::RT => (372, 70),
        PadBtn::RB => (360, 86),
        PadBtn::L3 => (242, 128),
        PadBtn::Up => (268, 150),
        PadBtn::Down => (268, 186),
        PadBtn::Left => (249, 168),
        PadBtn::Right => (287, 168),
        PadBtn::Select => (280, 118),
        PadBtn::Start => (320, 118),
        PadBtn::Touch => (300, 100),
        PadBtn::North => (360, 108),
        PadBtn::West => (340, 129),
        PadBtn::East => (380, 129),
        PadBtn::South => (360, 150),
        PadBtn::R3 => (330, 172),
    }
}

/// Inside the pad's outline: a body (an ellipse) with two grips.
fn inside(x: f32, y: f32) -> bool {
    let body = ((x - 300.0) / 112.0).powi(2) + ((y - 128.0) / 48.0).powi(2) <= 1.0;
    let grip = |gx: f32| ((x - gx) / 34.0).powi(2) + ((y - 178.0) / 40.0).powi(2) <= 1.0;
    let shoulders = y >= 74.0 && y <= 96.0 && ((x - 238.0).abs() < 26.0 || (x - 362.0).abs() < 26.0);
    body || grip(222.0) || grip(378.0) || shoulders
}

fn line(c: &mut Canvas, ox: i32, oy: i32, a: (i32, i32), b: (i32, i32), col: Rgb) {
    let (mut x0, mut y0) = a;
    let (x1, y1) = b;
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        c.fill(ox + x0, oy + y0, 1, 1, Style::Normal, col);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

/// What a button does now: the PAD_ACTIONS bound to it, joined.
fn actions_of(b: PadBtn, map: &[Option<PadBtn>]) -> String {
    let names: Vec<&str> = PAD_ACTIONS
        .iter()
        .zip(map)
        .filter(|(_, m)| **m == Some(b))
        .map(|(a, _)| a.0)
        .collect();
    names.join(" / ")
}

pub(crate) fn draw<W: KitWid>(u: &mut Ui<W>, cx: i32, cy: i32) {
    let kind = crate::gamepad::glyph_kind();
    let map = crate::gamepad::bindings();
    let font = u.f.small.clone();
    let text_col = m_col(8);
    let dim = m_col(1);
    let lit = m_col(1);
    // Backdrop.
    u.c.fill(cx, cy, W, H, Style::Normal, [8, 8, 10]);
    // The pad: filled, then its rim.
    for y in 50..230 {
        let mut run: Option<i32> = None;
        for x in 160..441 {
            let i = inside(x as f32, y as f32);
            match (i, run) {
                (true, None) => run = Some(x),
                (false, Some(s)) => {
                    u.c.fill(cx + s, cy + y, x - s, 1, Style::Normal, [28, 30, 34]);
                    run = None;
                }
                _ => {}
            }
            if i && (!inside(x as f32 - 1.0, y as f32) || !inside(x as f32 + 1.0, y as f32) || !inside(x as f32, y as f32 - 1.0) || !inside(x as f32, y as f32 + 1.0)) {
                u.c.fill(cx + x, cy + y, 1, 1, Style::Normal, dim);
            }
        }
        if let Some(s) = run {
            u.c.fill(cx + s, cy + y, 441 - s, 1, Style::Normal, [28, 30, 34]);
        }
    }
    // Sticks.
    for (sx, sy) in [anchor(PadBtn::L3), anchor(PadBtn::R3)] {
        for y in -11..=11 {
            let half = ((121 - y * y) as f32).sqrt() as i32;
            u.c.fill(cx + sx - half, cy + sy + y, half * 2 + 1, 1, Style::Normal, [50, 54, 60]);
        }
    }
    // The buttons themselves (their glyphs), on the pad.
    let shown: Vec<PadBtn> = crate::gamepad::ALL_BTNS
        .iter()
        .copied()
        .filter(|b| *b != PadBtn::Touch || kind == PadKind::PlayStation)
        .collect();
    for &b in &shown {
        let (ax, ay) = anchor(b);
        let w = crate::glyphs::width_in(kind, b);
        crate::glyphs::draw_in(&mut u.c, kind, b, cx + ax - w / 2, cy + ay - 7);
    }
    // Labels: the left half's buttons down the left side, the right half's down the right.
    let left = [PadBtn::LT, PadBtn::LB, PadBtn::L3, PadBtn::Up, PadBtn::Left, PadBtn::Right, PadBtn::Down, PadBtn::Select];
    let mut right = vec![PadBtn::RT, PadBtn::RB, PadBtn::North, PadBtn::West, PadBtn::East, PadBtn::South, PadBtn::R3, PadBtn::Start];
    if kind == PadKind::PlayStation {
        right.push(PadBtn::Touch);
    }
    let label_w = 150;
    for (side, list) in [(0, left.to_vec()), (1, right)] {
        let n = list.len() as i32;
        let step = ((H - 40) / n.max(1)).min(30);
        let top = (H - step * n) / 2 + 4;
        for (k, b) in list.iter().enumerate() {
            let ry = top + k as i32 * step + step / 2;
            let mut text = actions_of(*b, &map);
            match b {
                PadBtn::L3 => text = if text.is_empty() { "Move".into() } else { format!("Move | press: {text}") },
                PadBtn::R3 => text = if text.is_empty() { "Look".into() } else { format!("Look | press: {text}") },
                _ => {}
            }
            if text.is_empty() {
                text = "-".into();
            }
            let gw = crate::glyphs::width_in(kind, *b);
            let (ax, ay) = anchor(*b);
            if side == 0 {
                let gx = 8;
                crate::glyphs::draw_in(&mut u.c, kind, *b, cx + gx, cy + ry - 7);
                lines(u, &font, cx + gx + gw + 4, cy + ry, label_w - gw - 4, HAlign::Left, text_col, &text);
                line(&mut u.c, cx, cy, (label_w + 10, ry), (ax - crate::glyphs::width_in(kind, *b) / 2 - 2, ay), lit);
            } else {
                let gx = W - 8 - gw;
                crate::glyphs::draw_in(&mut u.c, kind, *b, cx + gx, cy + ry - 7);
                lines(u, &font, cx + W - 8 - label_w, cy + ry, label_w - gw - 4, HAlign::Right, text_col, &text);
                line(&mut u.c, cx, cy, (W - label_w - 10, ry), (ax + crate::glyphs::width_in(kind, *b) / 2 + 2, ay), lit);
            }
        }
    }
    // Footer.
    let foot = "Change these in Settings > Enhancements > Controller Bindings.";
    u.c.text(&font, cx, cy + H - 16, W, 14, HAlign::Center, VAlign::Center, dim, foot);
    let _ = (cx, cy);
}

/// Up to two lines of `s` in `w`, centred on `y` (the rest cut with an ellipsis).
#[allow(clippy::too_many_arguments)]
fn lines<W: KitWid>(u: &mut Ui<W>, font: &crate::uicanvas::UiFont, x: i32, y: i32, w: i32, ha: HAlign, col: Rgb, s: &str) {
    let mut ls = crate::uicanvas::wrap(font, s, w);
    if ls.len() > 2 {
        ls.truncate(2);
        if let Some(l) = ls.last_mut() {
            l.push_str("...");
        }
    }
    let top = y - (ls.len() as i32 * font.height) / 2;
    for (k, l) in ls.iter().enumerate() {
        u.c.text(font, x, top + k as i32 * font.height, w, font.height, ha, VAlign::Top, col, l);
    }
}

/// Once on the first map for a new player on a pad: the picture opens by itself (setting PadLayoutShown).
pub fn first_time_system(level: bevy::prelude::Res<crate::LevelRes>, mut waited: bevy::prelude::Local<f32>, time: bevy::prelude::Res<bevy::prelude::Time<bevy::prelude::Real>>) {
    if crate::settings::get("PadLayoutShown") > 0.5 || !crate::gamepad::pad_in_use() {
        return;
    }
    if !crate::util::contains_ci(&level.0.name, "01_NYC_UNATCOIsland") || crate::loader::settling() || crate::cinematic::active() || crate::consys::third_person() || crate::menus::open() {
        *waited = 0.0;
        return;
    }
    *waited += time.delta_secs();
    if *waited < 2.0 {
        return;
    }
    crate::settings::set("PadLayoutShown", 1.0);
    crate::settings::save();
    crate::menus::open_pad_map();
}
