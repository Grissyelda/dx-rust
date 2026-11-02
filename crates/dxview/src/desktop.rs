//! Ours (Diegetic Screens): the terminals' own operating system look. A terminal in use draws its original windows
//! (computers.rs) over a desktop - a wallpaper in the network's colour with its emblem, a task bar with the node,
//! the user and a clock, and a status line - and idle terminals near the player show a live standby screen
//! (the node's logo, a login box with a blinking caret, the clock), so monitors read as running machines before
//! anyone uses them (after Prey 2017 / Deus Ex: Human Revolution).

use crate::uicanvas::{Canvas, HAlign, Rgb, Style, UiFont, UiTex, VAlign};
use std::sync::Arc;

/// The network's colour, by node name (UNATCO steel blue, MJ12 red, VersaLife green, NSF amber, else teal).
pub(crate) fn accent(node: &str) -> [f32; 3] {
    let n = node.to_ascii_lowercase();
    if n.contains("unatco") {
        [0.36, 0.62, 0.95]
    } else if n.contains("mj") || n.contains("majestic") || n.contains("page") {
        [0.95, 0.32, 0.28]
    } else if n.contains("versa") {
        [0.35, 0.9, 0.55]
    } else if n.contains("nsf") || n.contains("silhouette") {
        [0.98, 0.7, 0.3]
    } else if n.is_empty() {
        // ATMs: PageNet's cyan.
        [0.3, 0.85, 0.9]
    } else {
        [0.32, 0.82, 0.82]
    }
}

fn rgb(a: [f32; 3], k: f32) -> Rgb {
    [(a[0] * k * 255.0).clamp(0.0, 255.0) as u8, (a[1] * k * 255.0).clamp(0.0, 255.0) as u8, (a[2] * k * 255.0).clamp(0.0, 255.0) as u8]
}

/// The in-world clock: a night shift that runs with the level's time.
pub(crate) fn clock(secs: f64, with_secs: bool) -> String {
    let t = (78_420.0 + secs).rem_euclid(86_400.0) as u32;
    if with_secs {
        format!("{:02}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60)
    } else {
        format!("{:02}:{:02}", t / 3600, t / 60 % 60)
    }
}

/// The wallpaper, straight into the canvas's pixels: a dark gradient in the accent colour, a faint grid,
/// scan lines, a vignette and the network's ring emblem round (ex, ey) of radius r (UI units).
pub(crate) fn wallpaper(c: &mut Canvas, a: [f32; 3], ex: f32, ey: f32, r: f32) {
    let s = c.s.max(1);
    let (pw, ph) = (c.w * s, c.h * s);
    let (fw, fh) = (c.w as f32, c.h as f32);
    let grid = 32;
    for y in 0..ph {
        let uy = y as f32 / s as f32;
        let fy = uy / fh.max(1.0);
        let scan = if (y / s) % 2 == 0 { 1.0 } else { 0.9 };
        for x in 0..pw {
            let ux = x as f32 / s as f32;
            let fx = ux / fw.max(1.0);
            let v = 1.0 - 0.65 * (((fx - 0.5) * (fx - 0.5)) + ((fy - 0.48) * (fy - 0.48))) * 2.0;
            let mut k = (0.16 - 0.1 * fy) * v.max(0.2) * scan;
            if (x / s) % grid == 0 || (y / s) % grid == 0 {
                k += 0.025;
            }
            // The emblem: two rings, eight ticks and a cross.
            let (dx, dy) = (ux - ex, uy - ey);
            let d = (dx * dx + dy * dy).sqrt();
            let ring = (d - r).abs() < 1.6 || (d - r * 0.86).abs() < 0.8 || (d - r * 0.42).abs() < 1.0;
            let ang = dy.atan2(dx);
            let tick = d > r * 1.04 && d < r * 1.16 && ((ang / (std::f32::consts::PI / 4.0)).round() * (std::f32::consts::PI / 4.0) - ang).abs() * d < 1.5;
            let cross = d < r * 0.86 && (dx.abs() < 0.7 || dy.abs() < 0.7) && d > r * 0.42;
            if ring || tick || cross {
                k += 0.11 * (1.0 - fy * 0.3);
            }
            let i = (y * pw + x) as usize;
            c.px[i] = [a[0] * k, a[1] * k, a[2] * k, 1.0];
        }
    }
}

/// The task bar along the top and the status line along the bottom of a desktop `w` x `h`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn bars(c: &mut Canvas, small: &UiFont, headers: &UiFont, a: [f32; 3], node: &str, user: &str, secs: f64, hint: &str) {
    let (w, h) = (c.w, c.h);
    // Task bar.
    c.fill(0, 0, w, 20, Style::Normal, rgb(a, 0.2));
    c.fill(0, 20, w, 1, Style::Normal, rgb(a, 0.7));
    c.fill(6, 6, 8, 8, Style::Normal, rgb(a, 0.9));
    c.fill(8, 8, 4, 4, Style::Normal, rgb(a, 0.25));
    let title = if node.is_empty() { "TERMINAL".to_string() } else { node.to_ascii_uppercase() };
    c.text(headers, 20, 0, w / 2, 20, HAlign::Left, VAlign::Center, rgb(a, 1.1), &title);
    let who = if user.is_empty() { "NOT LOGGED IN".to_string() } else { format!("USER  {}", user.to_ascii_uppercase()) };
    c.text(small, 0, 0, w, 20, HAlign::Center, VAlign::Center, rgb(a, 0.95), &who);
    c.text(small, w - 120, 0, 112, 20, HAlign::Right, VAlign::Center, [225, 232, 240], &clock(secs, true));
    // Status line.
    c.fill(0, h - 18, w, 18, Style::Normal, rgb(a, 0.12));
    c.fill(0, h - 19, w, 1, Style::Normal, rgb(a, 0.45));
    let on = (secs * 1.6).fract() < 0.6;
    c.fill(8, h - 12, 6, 6, Style::Normal, if on { [90, 240, 120] } else { [30, 70, 40] });
    c.text(small, 20, h - 18, w / 2, 18, HAlign::Left, VAlign::Center, rgb(a, 0.85), "SECURE CHANNEL  //  LINK ACTIVE");
    c.text(small, w / 2, h - 18, w / 2 - 8, 18, HAlign::Right, VAlign::Center, rgb(a, 0.75), hint);
}

/// A terminal's desktop: wallpaper with the emblem centred below the windows, the logo, the bars.
#[allow(clippy::too_many_arguments)]
pub(crate) fn desk(c: &mut Canvas, small: &UiFont, headers: &UiFont, logo: Option<&Arc<UiTex>>, node: &str, user: &str, secs: f64, hint: &str) {
    let a = accent(node);
    let (w, h) = (c.w as f32, c.h as f32);
    wallpaper(c, a, w * 0.5, h * 0.52, h * 0.36);
    let _ = logo;
    bars(c, small, headers, a, node, user, secs, hint);
}

/// The step-away hint for the status line.
pub(crate) fn leave_hint() -> String {
    if crate::gamepad::pad_in_use() {
        "B  STEP AWAY".to_string()
    } else {
        "ESC / RIGHT CLICK  STEP AWAY".to_string()
    }
}

/// The size of an idle terminal's standby picture for a screen of aspect `aspect` (width / height).
pub(crate) fn standby_size(aspect: f64) -> (i32, i32) {
    let h = 224;
    (((h as f64 * aspect).round() as i32).clamp(160, 420), h)
}

/// The standby screen's wallpaper (drawn once per screen; the rest goes over a copy of it each tick).
pub(crate) fn standby_base(w: i32, h: i32, node: &str) -> Canvas {
    let mut c = Canvas::new(w, h, 1);
    wallpaper(&mut c, accent(node), w as f32 * 0.5, h as f32 * 0.5, h as f32 * 0.42);
    c
}

/// The standby screen over its wallpaper: the bar with the clock, the logo (or a big clock), the prompt box with
/// its blinking caret.
#[allow(clippy::too_many_arguments)]
pub(crate) fn standby_over(c: &mut Canvas, class: &str, small: &UiFont, headers: &UiFont, title: &UiFont, logo: Option<&Arc<UiTex>>, node: &str, secs: f64) {
    let (w, h) = (c.w, c.h);
    let a = accent(node);
    c.fill(0, 0, w, 16, Style::Normal, rgb(a, 0.22));
    c.fill(0, 16, w, 1, Style::Normal, rgb(a, 0.7));
    let head = match class {
        "ATM" => "PAGENET GLOBAL BANKING".to_string(),
        "ComputerPublic" => "PUBLIC ACCESS".to_string(),
        _ if node.is_empty() => "TERMINAL".to_string(),
        _ => node.to_ascii_uppercase(),
    };
    c.text(small, 6, 0, w - 60, 16, HAlign::Left, VAlign::Center, rgb(a, 1.1), &head);
    c.text(small, w - 60, 0, 54, 16, HAlign::Right, VAlign::Center, [220, 228, 236], &clock(secs, false));
    let mid = h / 2 - 6;
    match logo {
        Some(tx) => c.texture(w / 2 - 30, mid - 58, 61, 61, 0, 0, tx, Style::Masked, [255, 255, 255]),
        None => c.text(title, 0, mid - 52, w, 40, HAlign::Center, VAlign::Center, rgb(a, 1.1), &clock(secs, false)),
    }
    let label = match class {
        "ATM" => "INSERT CARD",
        "ComputerPublic" => "BULLETIN BOARD",
        _ => "SECURE LOGIN",
    };
    c.text(headers, 0, mid + 6, w, 16, HAlign::Center, VAlign::Center, rgb(a, 1.0), label);
    let (bx, by, bw, bh) = (w / 2 - 70, mid + 26, 140, 16);
    c.fill(bx, by, bw, bh, Style::Normal, rgb(a, 0.55));
    c.fill(bx + 1, by + 1, bw - 2, bh - 2, Style::Normal, [6, 8, 12]);
    if (secs * 2.0).fract() < 0.55 {
        c.fill(bx + 6, by + 4, 2, bh - 8, Style::Normal, [230, 236, 240]);
    }
    let use_key = if crate::gamepad::pad_in_use() { "X" } else { "RIGHT CLICK" };
    c.text(small, 0, h - 22, w, 14, HAlign::Center, VAlign::Center, rgb(a, 0.85), &format!("{use_key} TO CONNECT"));
}

#[cfg(test)]
mod tests {
    #[test]
    fn clock_runs_and_wraps() {
        assert_eq!(super::clock(0.0, true), "21:47:00");
        assert_eq!(super::clock(61.0, false), "21:48");
        assert_eq!(super::clock(86_400.0, true), "21:47:00");
    }
}
