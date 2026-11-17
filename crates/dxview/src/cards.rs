//! Modern multiplayer cosmetics (MULTIPLAYER roadmap Phase 2): name-tag colours and calling cards.
//!
//! Both are earned, never bought: each has a milestone on the deathmatch character's record (kills, takedowns, wins,
//! matches played, teammates healed, credits ever earned) in data/deathmatch.ron `cosmetics`; the first ones are open
//! from the start. A player's tag colour is his name tag's colour for everyone (Team Deathmatch keeps the team's
//! colour); his calling card is shown to the player he takes down ("TAKEN DOWN BY") and on the results screen when he
//! wins. Cards are our own designs: a background pattern and an emblem drawn from a few shapes in the card's colour.

use crate::classic_hud::HudLayer;
use crate::dmchar::DmChar;
use crate::uicanvas::{Canvas, HAlign, Style, VAlign};
use bevy::prelude::*;
use serde::Deserialize;
use std::sync::Mutex;

/// A milestone on the character's record: `what` is kills / takedowns / wins / matches / heals / credits.
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Need {
    pub what: String,
    pub n: u32,
}

#[derive(Deserialize, Clone, Debug)]
pub struct TagColor {
    pub id: String,
    pub name: String,
    pub rgb: (u8, u8, u8),
    #[serde(default)]
    pub need: Option<Need>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Card {
    pub id: String,
    pub name: String,
    /// The line over the player's name.
    pub title: String,
    /// plain / stripes / grid / dots / chevrons / hex / circuit / rings / rays / waves / plates.
    pub pattern: String,
    /// ring / eye / triangle / crescent / crosshair / cross / diamond / star / crown / shield / bolt / orbit.
    pub emblem: String,
    pub accent: (u8, u8, u8),
    #[serde(default)]
    pub need: Option<Need>,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Cosmetics {
    pub tag_colors: Vec<TagColor>,
    pub cards: Vec<Card>,
}

fn data() -> &'static Cosmetics {
    &crate::deathmatch::data().cosmetics
}

/// How far a character has come toward a milestone kind.
pub fn progress(c: &DmChar, what: &str) -> u32 {
    match what {
        "kills" => c.total_kills,
        "takedowns" => c.takedowns,
        "wins" => c.wins,
        "matches" => c.matches,
        "heals" => c.heals,
        "credits" => c.credits_total,
        _ => 0,
    }
}

pub fn earned(c: &DmChar, need: &Option<Need>) -> bool {
    need.as_ref().is_none_or(|n| progress(c, &n.what) >= n.n)
}

/// "25 kills (12 more)" for a locked one; "" when earned.
pub fn need_text(c: &DmChar, need: &Option<Need>) -> String {
    let Some(n) = need else { return String::new() };
    let have = progress(c, &n.what);
    if have >= n.n {
        return String::new();
    }
    let what = match n.what.as_str() {
        "credits" => "credits earned",
        "heals" => "teammates healed",
        "matches" => "matches played",
        w => w,
    };
    format!("{} {what} ({} more)", n.n, n.n - have)
}

pub fn tag(id: &str) -> Option<&'static TagColor> {
    data().tag_colors.iter().find(|t| t.id == id)
}

pub fn card(id: &str) -> Option<&'static Card> {
    data().cards.iter().find(|t| t.id == id)
}

/// The character's name-tag colour (the first in the list when none or unknown).
pub fn tag_rgb(c: &DmChar) -> [u8; 3] {
    let t = tag(&c.tag_color).filter(|t| earned(c, &t.need)).or_else(|| data().tag_colors.first());
    t.map_or([230, 230, 230], |t| [t.rgb.0, t.rgb.1, t.rgb.2])
}

/// The character's calling card id (the first one when none, unknown or not earned).
pub fn card_id(c: &DmChar) -> String {
    card(&c.card).filter(|k| earned(c, &k.need)).or_else(|| data().cards.first()).map_or_else(String::new, |k| k.id.clone())
}

/// The next tag colour / card in the list (locked ones included: the creator shows what they need).
pub fn cycle_tag(c: &mut DmChar, dir: i32) {
    let all = &data().tag_colors;
    if all.is_empty() {
        return;
    }
    let i = all.iter().position(|t| t.id == c.tag_color).unwrap_or(0) as i32;
    c.tag_color = all[((i + dir).rem_euclid(all.len() as i32)) as usize].id.clone();
}

pub fn cycle_card(c: &mut DmChar, dir: i32) {
    let all = &data().cards;
    if all.is_empty() {
        return;
    }
    let i = all.iter().position(|t| t.id == c.card).unwrap_or(0) as i32;
    c.card = all[((i + dir).rem_euclid(all.len() as i32)) as usize].id.clone();
}

/// The creator's row values: "Name" or "Name (locked: 25 kills (12 more))".
pub fn tag_label(c: &DmChar) -> String {
    let Some(t) = tag(&c.tag_color).or_else(|| data().tag_colors.first()) else { return "White".into() };
    let need = need_text(c, &t.need);
    if need.is_empty() { t.name.clone() } else { format!("{} (locked: {need})", t.name) }
}

pub fn card_label(c: &DmChar) -> String {
    let Some(k) = card(&c.card).or_else(|| data().cards.first()) else { return "Operative".into() };
    let need = need_text(c, &k.need);
    if need.is_empty() { k.name.clone() } else { format!("{} (locked: {need})", k.name) }
}

/// Everything earned beyond the open ones (for the "unlocked" notices).
pub fn earned_ids(c: &DmChar) -> Vec<String> {
    let d = data();
    d.tag_colors
        .iter()
        .filter(|t| t.need.is_some() && earned(c, &t.need))
        .map(|t| format!("tag:{}", t.id))
        .chain(d.cards.iter().filter(|k| k.need.is_some() && earned(c, &k.need)).map(|k| format!("card:{}", k.id)))
        .collect()
}

/// After the record changed in a match: announce what it just earned (and remember it was announced).
pub fn announce_new(c: &mut DmChar) {
    if !c.cosmetics_seen.iter().any(|s| s == "init") {
        // A character from before cosmetics: what his record already earned is his without a fanfare.
        c.cosmetics_seen = earned_ids(c);
        c.cosmetics_seen.push("init".into());
        return;
    }
    for id in earned_ids(c) {
        if c.cosmetics_seen.contains(&id) {
            continue;
        }
        c.cosmetics_seen.push(id.clone());
        let msg = match id.split_once(':') {
            Some(("tag", t)) => tag(t).map(|t| format!("Name-tag colour unlocked: {}", t.name)),
            Some(("card", k)) => card(k).map(|k| format!("Calling card unlocked: {}", k.name)),
            _ => None,
        };
        if let Some(m) = msg {
            if crate::deathmatch::playing() {
                crate::classic_hud::client_message(&m);
                crate::sound::play_player("DeusExSounds.UserInterface.LogGoalAdded", 0.8, 0.0);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Drawing.

/// Card size in UI units.
pub const CARD_W: i32 = 220;
pub const CARD_H: i32 = 48;

fn mixf(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Paint a box pixel by pixel at full resolution: `f(u, v)` in UI units from the box's corner gives (colour 0..1,
/// coverage 0..1) laid over what's there.
fn paint(c: &mut Canvas, x: i32, y: i32, w: i32, h: i32, f: impl Fn(f32, f32) -> Option<([f32; 3], f32)>) {
    let s = c.s;
    let stride = (c.w * s) as usize;
    let inv = 1.0 / s as f32;
    for py in (y * s).max(0)..((y + h) * s).min(c.h * s) {
        for px in (x * s).max(0)..((x + w) * s).min(c.w * s) {
            let (u, v) = ((px as f32 + 0.5) * inv - x as f32, (py as f32 + 0.5) * inv - y as f32);
            let Some((rgb, a)) = f(u, v) else { continue };
            let a = a.clamp(0.0, 1.0);
            if a <= 0.0 {
                continue;
            }
            let p = &mut c.px[py as usize * stride + px as usize];
            for k in 0..3 {
                p[k] = p[k] * (1.0 - a) + rgb[k] * a;
            }
            p[3] = a + p[3] * (1.0 - a);
        }
    }
}

/// 0..1 coverage of a soft edge at signed distance `d` (negative inside), in UI units.
fn edge(d: f32) -> f32 {
    (0.5 - d * 1.5).clamp(0.0, 1.0)
}

/// The pattern's strength at (u, v) on a w x h card (0..1).
fn pattern(kind: &str, u: f32, v: f32, w: f32, h: f32) -> f32 {
    let fr = |x: f32| x - x.floor();
    match kind {
        "stripes" => (fr((u + v) / 9.0) < 0.35) as u8 as f32,
        "grid" => ((fr(u / 8.0) < 0.12) || (fr(v / 8.0) < 0.12)) as u8 as f32,
        "dots" => {
            let (a, b) = (fr(u / 7.0) - 0.5, fr(v / 7.0) - 0.5);
            edge((a * a + b * b).sqrt() * 7.0 - 1.2)
        }
        "chevrons" => {
            let t = fr((u + (v - h / 2.0).abs()) / 12.0);
            (t < 0.3) as u8 as f32
        }
        "hex" => {
            // Three line families at 60 degrees make a hex-ish lattice.
            let l = |a: f32| fr((u * a.cos() + v * a.sin()) / 9.0) < 0.1;
            (l(0.0) || l(1.047) || l(2.094)) as u8 as f32 * 0.8
        }
        "circuit" => {
            let (cx, cy) = ((u / 12.0).floor(), (v / 12.0).floor());
            let hsh = ((cx * 12.9898 + cy * 78.233).sin() * 43758.545).fract().abs();
            let (a, b) = (fr(u / 12.0), fr(v / 12.0));
            let trace = if hsh < 0.5 { (b - 0.5).abs() < 0.08 } else { (a - 0.5).abs() < 0.08 };
            let pad = ((a - 0.5).powi(2) + (b - 0.5).powi(2)).sqrt() < 0.16 && hsh > 0.7;
            (trace || pad) as u8 as f32
        }
        "rings" => {
            let d = ((u - w * 0.15).powi(2) + (v - h * 0.5).powi(2)).sqrt();
            (fr(d / 7.0) < 0.22) as u8 as f32
        }
        "rays" => {
            let a = (v - h * 0.5).atan2(u - w * 0.15);
            (fr(a * 12.0 / std::f32::consts::TAU) < 0.4) as u8 as f32 * (u / w).min(1.0)
        }
        "waves" => {
            let y = v + (u / 10.0).sin() * 3.0;
            (fr(y / 8.0) < 0.25) as u8 as f32
        }
        "plates" => {
            let (a, b) = (fr(u / 22.0), fr(v / 16.0));
            let seam = a < 0.04 || b < 0.06;
            let rivet = ((a - 0.12).powi(2) * 484.0 + (b - 0.2).powi(2) * 256.0).sqrt() < 1.2;
            (seam || rivet) as u8 as f32
        }
        _ => 0.0,
    }
}

/// The emblem's signed distance at (x, y) from its centre, radius r (negative inside).
fn emblem(kind: &str, x: f32, y: f32, r: f32) -> f32 {
    let len = (x * x + y * y).sqrt();
    let ring = |rad: f32, t: f32| (len - rad).abs() - t;
    let bx = |cx: f32, cy: f32, hw: f32, hh: f32| ((x - cx).abs() - hw).max((y - cy).abs() - hh);
    let tri = |s: f32, flip: f32| {
        // Equilateral triangle pointing up (flip -1: down).
        let yy = y * flip;
        let k = 3f32.sqrt();
        let (px, py) = (x.abs(), yy + s * 0.35);
        (px * k / 2.0 + py * 0.5 - s * 0.5).max(-py)
    };
    match kind {
        "ring" => ring(r * 0.7, r * 0.13).min(len - r * 0.2),
        "eye" => {
            let lens = ((x / (r * 0.95)).powi(2) + (y / (r * 0.45)).powi(2)).sqrt() - 1.0;
            let iris = len - r * 0.28;
            (lens * r * 0.4).abs().max(-1.0).min(iris) - r * 0.06
        }
        "triangle" => {
            let outer = tri(r * 1.6, 1.0);
            let inner = tri(r * 0.9, 1.0) + r * 0.05;
            outer.max(-inner)
        }
        "crescent" => (len - r * 0.8).max(-(((x - r * 0.35).powi(2) + (y + r * 0.1).powi(2)).sqrt() - r * 0.7)),
        "crosshair" => ring(r * 0.62, r * 0.08).min(bx(0.0, 0.0, r * 0.06, r * 0.95)).min(bx(0.0, 0.0, r * 0.95, r * 0.06)).max(-(len - r * 0.15)),
        "cross" => bx(0.0, 0.0, r * 0.25, r * 0.8).min(bx(0.0, 0.0, r * 0.8, r * 0.25)),
        "diamond" => {
            let d = (x.abs() + y.abs()) * 0.7071 - r * 0.6;
            d.max(-((x.abs() + y.abs()) * 0.7071 - r * 0.3))
        }
        "star" => {
            // Five points: the distance to a star outline in polar form.
            let a = y.atan2(x) + std::f32::consts::FRAC_PI_2;
            let k = (a * 5.0 / std::f32::consts::TAU).fract().abs();
            let spike = ((k - 0.5).abs() * 2.0).powf(2.2);
            len - r * (0.36 + 0.6 * spike)
        }
        "crown" => {
            let base = bx(0.0, r * 0.45, r * 0.8, r * 0.15);
            // Three teeth pointing up from the band (apex at the top, wider toward the band).
            let tooth = |cx: f32, apex: f32| {
                let (top, bottom, half) = (apex, r * 0.35, r * 0.3);
                let k = ((y - top) / (bottom - top)).clamp(0.0, 1.0);
                ((x - cx).abs() - half * k).max(top - y).max(y - bottom)
            };
            base.min(tooth(-r * 0.55, -r * 0.45)).min(tooth(0.0, -r * 0.75)).min(tooth(r * 0.55, -r * 0.45))
        }
        "shield" => {
            let top = bx(0.0, -r * 0.25, r * 0.7, r * 0.55);
            let bottom = (((x / (r * 0.7)).powi(2) + ((y - r * 0.25) / (r * 0.75)).powi(2)).sqrt() - 1.0) * r * 0.7;
            let body = top.min(bottom.max(-(y - r * 0.25)).max(y - r * 1.0));
            body.max(-(top + r * 0.18).min(bottom + r * 0.18).max(-(y + r * 0.62)))
        }
        "bolt" => {
            let seg = |ax: f32, ay: f32, bx2: f32, by: f32, t: f32| {
                let (px, py, dx, dy) = (x - ax, y - ay, bx2 - ax, by - ay);
                let h = ((px * dx + py * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
                ((px - dx * h).powi(2) + (py - dy * h).powi(2)).sqrt() - t
            };
            let t = r * 0.13;
            seg(r * 0.3, -r * 0.9, -r * 0.25, r * 0.05, t).min(seg(-r * 0.25, r * 0.05, r * 0.25, -r * 0.05, t)).min(seg(r * 0.25, -r * 0.05, -r * 0.3, r * 0.9, t))
        }
        "orbit" => {
            let e = |a: f32| {
                let (c, s) = (a.cos(), a.sin());
                let (xx, yy) = (x * c + y * s, -x * s + y * c);
                ((((xx / (r * 0.9)).powi(2) + (yy / (r * 0.32)).powi(2)).sqrt() - 1.0) * r * 0.32).abs() - r * 0.05
            };
            e(0.5).min(e(-0.5)).min(len - r * 0.2)
        }
        _ => len - r * 0.5,
    }
}

/// Draw a calling card at (x, y): background, pattern and emblem in the card's colour, the title over the player's
/// name, a thin border.
pub fn draw_card(c: &mut Canvas, x: i32, y: i32, card: &Card, name: &str, small: &crate::uicanvas::UiFont, big: &crate::uicanvas::UiFont) {
    let acc = [card.accent.0 as f32 / 255.0, card.accent.1 as f32 / 255.0, card.accent.2 as f32 / 255.0];
    let (w, h) = (CARD_W as f32, CARD_H as f32);
    let dark = [0.02, 0.02, 0.03];
    let pat = card.pattern.clone();
    paint(c, x, y, CARD_W, CARD_H, |u, v| {
        // Dark at the text side, the colour coming up behind the emblem.
        let t = (1.0 - u / w).powf(1.6) * 0.55 + 0.08;
        let base = mixf(dark, acc, t);
        let p = pattern(&pat, u, v, w, h) * (0.10 + 0.12 * (1.0 - u / w));
        Some((mixf(base, acc, p), 1.0))
    });
    // Emblem in a soft disc on the left.
    let (ex, ey, r) = (24.0, h * 0.5, 17.0);
    let emb = card.emblem.clone();
    let glow = mixf(acc, [1.0, 1.0, 1.0], 0.35);
    paint(c, x, y, 48, CARD_H, |u, v| {
        let (dx, dy) = (u - ex, v - ey);
        let halo = (1.0 - ((dx * dx + dy * dy).sqrt() / (r * 1.4))).clamp(0.0, 1.0).powi(2) * 0.25;
        let d = emblem(&emb, dx, dy, r);
        let a = edge(d);
        if a <= 0.0 && halo <= 0.0 {
            return None;
        }
        Some((if a > 0.0 { glow } else { acc }, a.max(halo)))
    });
    // Border: the colour, brighter at the top.
    let b = [card.accent.0, card.accent.1, card.accent.2];
    let hi = [b[0].saturating_add(60), b[1].saturating_add(60), b[2].saturating_add(60)];
    c.fill(x, y, CARD_W, 1, Style::Normal, hi);
    c.fill(x, y + CARD_H - 1, CARD_W, 1, Style::Normal, b);
    c.fill(x, y, 1, CARD_H, Style::Normal, b);
    c.fill(x + CARD_W - 1, y, 1, CARD_H, Style::Normal, b);
    let tx = x + 52;
    let tw = CARD_W - 58;
    let title_col = [((b[0] as u16 + 255) / 2) as u8, ((b[1] as u16 + 255) / 2) as u8, ((b[2] as u16 + 255) / 2) as u8];
    c.text(small, tx, y + 7, tw, small.height, HAlign::Left, VAlign::Top, title_col, &card.title);
    c.set_clip(Some((tx, y, tw, CARD_H)));
    c.text(big, tx, y + 9 + small.height, tw, big.height, HAlign::Left, VAlign::Top, [255, 255, 255], name);
    c.set_clip(None);
}

// ---------------------------------------------------------------------------------------------------------------
// "TAKEN DOWN BY": the killer's card, on the victim's screen for a few seconds after he goes down (Modern).

struct Shown {
    name: String,
    card: String,
    t: f32,
}

static KILLER: Mutex<Option<Shown>> = Mutex::new(None);
const SHOW_SECS: f32 = 5.0;

/// The player went down to `name` (whose card is `card`).
pub fn show_killer(name: &str, card: &str) {
    if card.is_empty() {
        return;
    }
    *KILLER.lock().unwrap_or_else(|e| e.into_inner()) = Some(Shown { name: name.to_string(), card: card.to_string(), t: 0.0 });
}

pub fn reset_level() {
    *KILLER.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

#[derive(Component, Default)]
pub struct KillCardNode;

#[allow(clippy::too_many_arguments)]
pub fn killcard_system(
    mut commands: Commands,
    time: Res<Time>,
    windows: Query<&Window>,
    mut hud: ResMut<crate::classic_hud::ClassicHud>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut layers: ResMut<Assets<HudLayer>>,
    mut node: Query<(Entity, &mut Node, &mut Visibility), With<KillCardNode>>,
    mut state: Local<(Option<Handle<Image>>, u64)>,
) {
    let (img, key) = &mut *state;
    let mut g = KILLER.lock().unwrap_or_else(|e| e.into_inner());
    let live = match g.as_mut() {
        Some(s) => {
            s.t += time.delta_secs();
            s.t < SHOW_SECS
        }
        None => false,
    };
    if !live {
        *g = None;
        if *key != 0 {
            crate::prompts::present::<KillCardNode, _>(&mut commands, &mut images, &mut layers, &mut node, img, None, (1, 1), 1, Vec2::ZERO, 1.0);
            *key = 0;
        }
        return;
    }
    let Some(sh) = g.as_ref() else { return };
    let Some(win) = windows.iter().next() else { return };
    let s = crate::hud::ui_scale(win).max(1.0) as i32;
    let k = s as f32 / win.scale_factor();
    let h = crate::util::fnv1a(&format!("{}|{}|{s}|{}", sh.name, sh.card, win.width()));
    let (w, hh) = (CARD_W, CARD_H + 14);
    let pos = Vec2::new((win.width() - w as f32 * k) / 2.0, win.height() * 0.62);
    if h == *key && !node.is_empty() {
        return;
    }
    *key = h;
    let (Some(small), Some(title)) = (hud.font(&mut assets, "FontMenuSmall_DS"), hud.font(&mut assets, "FontMenuTitle")) else { return };
    let Some(cd) = card(&sh.card) else { return };
    let mut c = Canvas::new(w, hh, s);
    c.text(&small, 0, 0, w, small.height, HAlign::Center, VAlign::Top, crate::menuui::h_col(7), "TAKEN DOWN BY");
    draw_card(&mut c, 0, 14, cd, &sh.name, &small, &title);
    crate::prompts::present::<KillCardNode, _>(&mut commands, &mut images, &mut layers, &mut node, img, Some(&c), (w, hh), s, pos, k);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn milestones() {
        let c = DmChar { total_kills: 12, wins: 1, ..Default::default() };
        assert!(earned(&c, &None));
        assert!(earned(&c, &Some(Need { what: "kills".into(), n: 10 })));
        assert!(!earned(&c, &Some(Need { what: "kills".into(), n: 25 })));
        assert_eq!(need_text(&c, &Some(Need { what: "kills".into(), n: 25 })), "25 kills (13 more)");
        assert_eq!(need_text(&c, &Some(Need { what: "wins".into(), n: 1 })), "");
    }

    /// CARD_PREVIEW=<file.rgba>: every card on one sheet (no lettering: test font), 2x.
    #[test]
    fn card_sheet() {
        let Ok(out) = std::env::var("CARD_PREVIEW") else { return };
        let d = crate::deathmatch::data();
        let font = crate::uicanvas::UiFont::new(
            unreal_pkg::objects::Font { pages: vec![(0, Vec::new())], chars_per_page: 256 },
            vec![std::sync::Arc::new(crate::uicanvas::UiTex { w: 1, h: 1, px: vec![[0; 4]] })],
        );
        let n = d.cosmetics.cards.len() as i32;
        let mut c = Canvas::new(CARD_W * 2 + 12, (CARD_H + 6) * ((n + 1) / 2), 2);
        for (i, k) in d.cosmetics.cards.iter().enumerate() {
            let i = i as i32;
            draw_card(&mut c, (i % 2) * (CARD_W + 12), (i / 2) * (CARD_H + 6), k, "Agent", &font, &font);
        }
        println!("CARD sheet {} x {}", c.w * 2, c.h * 2);
        std::fs::write(out, c.to_rgba8()).unwrap();
    }

    #[test]
    fn every_card_draws() {
        let d = crate::deathmatch::data();
        assert!(d.cosmetics.cards.len() >= 8 && d.cosmetics.tag_colors.len() >= 6);
        let font = crate::uicanvas::UiFont::new(
            unreal_pkg::objects::Font { pages: vec![(0, Vec::new())], chars_per_page: 256 },
            vec![std::sync::Arc::new(crate::uicanvas::UiTex { w: 1, h: 1, px: vec![[0; 4]] })],
        );
        for k in &d.cosmetics.cards {
            let mut c = Canvas::new(CARD_W, CARD_H, 2);
            draw_card(&mut c, 0, 0, k, "Agent", &font, &font);
            // Something of the emblem is lit (brighter than the background at the emblem's centre).
            let s = 2usize;
            let at = |x: usize, y: usize| c.px[(y * s) * (CARD_W as usize * s) + x * s];
            let centre = at(24, (CARD_H / 2) as usize);
            let far = at(200, 4);
            let lum = |p: [f32; 4]| p[0] + p[1] + p[2];
            assert!(lum(centre) > lum(far) || k.emblem == "ring" || k.emblem == "crescent", "{} emblem {}", k.id, k.emblem);
        }
    }
}
