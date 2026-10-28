//! Button glyphs (ROADMAP Phase 0.2): Xbox and PlayStation sets drawn by us in Deus Ex's own HUD language, no
//! trademarked artwork. A 15 x 15 UI-unit plate: dark translucent fill, the HUD theme's border colour with a lighter
//! bevel on the top and left edges, and the label in the HUD header colour as 5 x 7 pixel lettering (the bitmap
//! fonts' look), so the glyphs follow the player's HUD theme and stay crisp at the whole-number UI scale.
//! Xbox: A B X Y, LB RB LT RT, View / Menu, L / R sticks. PlayStation: the four shapes drawn as lines, L1 R1 L2 R2,
//! Create / Options, L3 R3, the touchpad. The D-pad is a cross with the pressed arm lit.

use crate::gamepad::{PadBtn, PadKind};
use crate::uicanvas::{Canvas, Rgb, Style};

pub const SIZE: i32 = 15;

/// 5 x 7 pixel letters (rows top to bottom, bit 4 = left column).
fn glyph5x7(ch: char) -> Option<[u8; 7]> {
    Some(match ch {
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1E, 0x01, 0x01, 0x0E, 0x01, 0x01, 0x1E],
        _ => return None,
    })
}

/// Draw text in the 5 x 7 lettering (1 unit between letters), left-aligned at (x, y).
fn text(c: &mut Canvas, s: &str, x: i32, y: i32, col: Rgb) {
    let mut cx = x;
    for ch in s.chars() {
        if let Some(rows) = glyph5x7(ch) {
            for (ry, row) in rows.iter().enumerate() {
                for rx in 0..5 {
                    if row & (0x10 >> rx) != 0 {
                        c.fill(cx + rx, y + ry as i32, 1, 1, Style::Normal, col);
                    }
                }
            }
        }
        cx += 6;
    }
}

fn text_w(s: &str) -> i32 {
    s.chars().count() as i32 * 6 - 1
}

fn lighter(c: Rgb) -> Rgb {
    c.map(|v| (v as u16 + (255 - v as u16) / 2) as u8)
}

/// The plate every glyph sits on.
fn plate(c: &mut Canvas, x: i32, y: i32, w: i32, round: bool) {
    plate_h(c, x, y, w, SIZE, round)
}

fn plate_h(c: &mut Canvas, x: i32, y: i32, w: i32, size: i32, round: bool) {
    let border = crate::menuui::h_col(1);
    let hi = lighter(border);
    // Fill: the HUD background colour, dimmed, drawn additively like the HUD's own panels over a black base.
    c.fill(x + 1, y + 1, w - 2, size - 2, Style::Normal, [8, 8, 10]);
    c.fill(x + 1, y + 1, w - 2, size - 2, Style::Translucent, crate::menuui::h_col(0).map(|v| v / 4));
    let inset = if round { 1 } else { 0 };
    // Edges: lit top / left, plain bottom / right (the menus' bevel).
    c.fill(x + inset, y, w - 2 * inset, 1, Style::Normal, hi);
    c.fill(x, y + inset, 1, size - 2 * inset, Style::Normal, hi);
    c.fill(x + inset, y + size - 1, w - 2 * inset, 1, Style::Normal, border);
    c.fill(x + w - 1, y + inset, 1, size - 2 * inset, Style::Normal, border);
}

/// PlayStation face shapes as 1-unit lines in a 9 x 9 box at (x, y).
fn ps_shape(c: &mut Canvas, b: PadBtn, x: i32, y: i32, col: Rgb) {
    ps_shape_n(c, b, x, y, 9, col)
}

/// The shapes in an n x n box (n odd).
fn ps_shape_n(c: &mut Canvas, b: PadBtn, x: i32, y: i32, n: i32, col: Rgb) {
    let m = n - 1;
    match b {
        PadBtn::South => {
            for i in 0..n {
                c.fill(x + i, y + i, 1, 1, Style::Normal, col);
                c.fill(x + m - i, y + i, 1, 1, Style::Normal, col);
            }
        }
        PadBtn::East => {
            let r = m as f32 / 2.0 + 0.2;
            for i in 0..64 {
                let t = i as f32 / 64.0 * std::f32::consts::TAU;
                c.fill(x + m / 2 + (t.cos() * r).round() as i32, y + m / 2 + (t.sin() * r).round() as i32, 1, 1, Style::Normal, col);
            }
        }
        PadBtn::West => {
            c.fill(x, y, n, 1, Style::Normal, col);
            c.fill(x, y + m, n, 1, Style::Normal, col);
            c.fill(x, y, 1, n, Style::Normal, col);
            c.fill(x + m, y, 1, n, Style::Normal, col);
        }
        PadBtn::North => {
            c.fill(x, y + m, n, 1, Style::Normal, col);
            for i in 0..n {
                let yy = y + m - i;
                c.fill(x + i / 2, yy, 1, 1, Style::Normal, col);
                c.fill(x + m - i / 2, yy, 1, 1, Style::Normal, col);
            }
        }
        _ => {}
    }
}

/// The compact 11-unit glyph for list rows (12 high): same plates, the shapes in 7 x 7, the D-pad smaller.
pub const SMALL: i32 = 11;

pub fn draw_small(c: &mut Canvas, kind: PadKind, b: PadBtn, x: i32, y: i32) {
    let col = crate::menuui::h_col(7);
    let w = width_small(kind, b);
    match b {
        PadBtn::Up | PadBtn::Down | PadBtn::Left | PadBtn::Right => {
            let dim = crate::menuui::h_col(1);
            plate_h(c, x, y, SMALL, SMALL, true);
            let (cx, cy) = (x + 5, y + 5);
            c.fill(cx - 1, cy - 4, 3, 9, Style::Normal, dim);
            c.fill(cx - 4, cy - 1, 9, 3, Style::Normal, dim);
            let (ax, ay, aw, ah) = match b {
                PadBtn::Up => (cx - 1, cy - 4, 3, 3),
                PadBtn::Down => (cx - 1, cy + 2, 3, 3),
                PadBtn::Left => (cx - 4, cy - 1, 3, 3),
                _ => (cx + 2, cy - 1, 3, 3),
            };
            c.fill(ax, ay, aw, ah, Style::Normal, col);
        }
        PadBtn::South | PadBtn::East | PadBtn::West | PadBtn::North if kind == PadKind::PlayStation => {
            plate_h(c, x, y, SMALL, SMALL, true);
            ps_shape_n(c, b, x + 2, y + 2, 7, col);
        }
        PadBtn::Select | PadBtn::Start | PadBtn::Touch => {
            plate_h(c, x, y, w, SMALL, true);
            match b {
                PadBtn::Start => {
                    for dy in [3, 5, 7] {
                        c.fill(x + 4, y + dy, w - 8, 1, Style::Normal, col);
                    }
                }
                PadBtn::Select => {
                    c.fill(x + 4, y + 3, 5, 4, Style::Normal, crate::menuui::h_col(1));
                    c.fill(x + 6, y + 5, 5, 4, Style::Normal, col);
                }
                _ => c.fill(x + 3, y + 3, w - 6, 5, Style::Normal, col),
            }
        }
        _ => {
            plate_h(c, x, y, w, SMALL, matches!(b, PadBtn::South | PadBtn::East | PadBtn::West | PadBtn::North));
            let l = label(kind, b);
            text(c, l, x + (w - text_w(l)) / 2, y + 2, col);
        }
    }
}

pub fn width_small(kind: PadKind, b: PadBtn) -> i32 {
    let l = label(kind, b);
    if l.chars().count() >= 2 {
        text_w(l) + 4
    } else if matches!(b, PadBtn::Select | PadBtn::Start | PadBtn::Touch) {
        15
    } else {
        SMALL
    }
}

/// The label for a button in a set.
fn label(kind: PadKind, b: PadBtn) -> &'static str {
    match (kind, b) {
        (PadKind::Xbox, PadBtn::South) => "A",
        (PadKind::Xbox, PadBtn::East) => "B",
        (PadKind::Xbox, PadBtn::West) => "X",
        (PadKind::Xbox, PadBtn::North) => "Y",
        (PadKind::Xbox, PadBtn::LB) => "LB",
        (PadKind::Xbox, PadBtn::RB) => "RB",
        (PadKind::Xbox, PadBtn::LT) => "LT",
        (PadKind::Xbox, PadBtn::RT) => "RT",
        (PadKind::PlayStation, PadBtn::LB) => "L1",
        (PadKind::PlayStation, PadBtn::RB) => "R1",
        (PadKind::PlayStation, PadBtn::LT) => "L2",
        (PadKind::PlayStation, PadBtn::RT) => "R2",
        (PadKind::Xbox, PadBtn::L3) => "L",
        (PadKind::Xbox, PadBtn::R3) => "R",
        (PadKind::PlayStation, PadBtn::L3) => "L3",
        (PadKind::PlayStation, PadBtn::R3) => "R3",
        _ => "",
    }
}

/// The plate width for a button (two-letter labels get a wider plate).
pub fn width(b: PadBtn) -> i32 {
    width_in(crate::gamepad::glyph_kind(), b)
}

pub fn width_in(kind: PadKind, b: PadBtn) -> i32 {
    let l = label(kind, b);
    if l.chars().count() >= 2 {
        text_w(l) + 6
    } else if matches!(b, PadBtn::Select | PadBtn::Start | PadBtn::Touch) {
        19
    } else {
        SIZE
    }
}

/// Draw the glyph for `b` in the current set with its top-left at (x, y). (`_scale` is the canvas's own.)
pub fn draw(c: &mut Canvas, b: PadBtn, x: i32, y: i32, _scale: i32) {
    draw_in(c, crate::gamepad::glyph_kind(), b, x, y);
}

pub fn draw_in(c: &mut Canvas, kind: PadKind, b: PadBtn, x: i32, y: i32) {
    let col = crate::menuui::h_col(7);
    let w = width_in(kind, b);
    match b {
        PadBtn::Up | PadBtn::Down | PadBtn::Left | PadBtn::Right => {
            // The D-pad: a cross, the pressed arm lit.
            let dim = crate::menuui::h_col(1);
            plate(c, x, y, SIZE, true);
            let (cx, cy) = (x + 7, y + 7);
            c.fill(cx - 1, cy - 5, 3, 11, Style::Normal, dim);
            c.fill(cx - 5, cy - 1, 11, 3, Style::Normal, dim);
            let (ax, ay, aw, ah) = match b {
                PadBtn::Up => (cx - 1, cy - 5, 3, 4),
                PadBtn::Down => (cx - 1, cy + 2, 3, 4),
                PadBtn::Left => (cx - 5, cy - 1, 4, 3),
                _ => (cx + 2, cy - 1, 4, 3),
            };
            c.fill(ax, ay, aw, ah, Style::Normal, col);
        }
        PadBtn::South | PadBtn::East | PadBtn::West | PadBtn::North if kind == PadKind::PlayStation => {
            plate(c, x, y, SIZE, true);
            ps_shape(c, b, x + 3, y + 3, col);
        }
        PadBtn::Select | PadBtn::Start | PadBtn::Touch => {
            plate(c, x, y, w, true);
            match (kind, b) {
                // View: two overlapping windows; Create: a slanted line with dots; Menu / Options: three lines;
                // the touchpad: a wide bar.
                (PadKind::Xbox, PadBtn::Select) => {
                    for (ox, oy) in [(5, 4), (8, 7)] {
                        c.fill(x + ox, y + oy, 6, 1, Style::Normal, col);
                        c.fill(x + ox, y + oy + 4, 6, 1, Style::Normal, col);
                        c.fill(x + ox, y + oy, 1, 5, Style::Normal, col);
                        c.fill(x + ox + 5, y + oy, 1, 5, Style::Normal, col);
                    }
                }
                (PadKind::PlayStation, PadBtn::Select) => {
                    for i in 0..7 {
                        c.fill(x + 6 + i, y + 10 - i, 1, 1, Style::Normal, col);
                    }
                    c.fill(x + 5, y + 4, 1, 1, Style::Normal, col);
                    c.fill(x + 13, y + 10, 1, 1, Style::Normal, col);
                }
                (_, PadBtn::Start) => {
                    for dy in [4, 7, 10] {
                        c.fill(x + 5, y + dy, 9, 1, Style::Normal, col);
                    }
                }
                _ => {
                    c.fill(x + 4, y + 5, 11, 5, Style::Normal, col);
                }
            }
        }
        _ => {
            plate(c, x, y, w, matches!(b, PadBtn::South | PadBtn::East | PadBtn::West | PadBtn::North | PadBtn::L3 | PadBtn::R3));
            let l = label(kind, b);
            text(c, l, x + (w - text_w(l)) / 2, y + 4, col);
            if matches!(b, PadBtn::L3 | PadBtn::R3) && kind == PadKind::Xbox {
                // A stick: the letter over a small ring.
                c.fill(x + 4, y + 12, 7, 1, Style::Normal, crate::menuui::h_col(1));
            }
        }
    }
}
