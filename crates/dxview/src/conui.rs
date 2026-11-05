//! The conversation and infolink windows, drawn with the original art and fonts: HUDInfoLinkDisplay (infolinks),
//! HUDConWindowFirst (first-person conversations, above the belt) and ConWindowActive (the third-person
//! letterbox with the speech and the choices).

use crate::classic_hud::ClassicHud;
use crate::uicanvas::{wrap, Canvas, HAlign, Rgb, Style, UiFont, VAlign};
use crate::weapons::RuntimeAssets;

/// ConWindowSpeech / ConWindowActive colours.
const COL_SPEECH: Rgb = [0, 255, 255];
const COL_SPEECH_PLAYER: Rgb = [255, 255, 255];
const COL_NAME: Rgb = [255, 255, 255];
const COL_CHOICE: Rgb = [0, 0, 255];
const COL_FOCUS: Rgb = [255, 255, 0];
const COL_SKILL: Rgb = [255, 0, 0];

pub const INFOLINK_SIZE: (i32, i32) = (431, 114);

/// What the windows show this frame (copied out of the conversation state).
#[derive(Clone, Default)]
pub struct ConView {
    pub link: Option<LinkView>,
    pub play: Option<PlayView>,
    /// HUDBarkDisplay items: (speaker's UnfamiliarName, line).
    pub barks: Vec<(String, String)>,
}

#[derive(Clone, Default)]
pub struct LinkView {
    pub name: String,
    pub name_visible: bool,
    pub queued: Option<String>,
    pub portrait: Option<String>,
    pub show_icon: bool,
    pub show_portrait: bool,
    pub text: String,
    pub typed: usize,
    pub cursor: bool,
    pub time: f32,
}

#[derive(Clone, Default)]
pub struct PlayView {
    pub first_person: bool,
    pub name: String,
    pub text: String,
    pub player_line: bool,
    /// (text, skill choice)
    pub choices: Vec<(String, bool)>,
    pub focus: usize,
    pub win_pos: f32,
    /// bForcePlay: a cut scene's conversation (text centred).
    pub forced: bool,
}

pub fn view() -> ConView {
    let g = crate::consys::lock();
    let Some(s) = g.as_ref() else {
        return ConView::default();
    };
    ConView {
        barks: s
            .bark_lines
            .iter()
            .map(|b| (b.name.clone(), b.text.clone()))
            .collect(),
        link: s
            .link
            .as_ref()
            .filter(|l| l.is_visible())
            .map(|l| LinkView {
                name: l.name.clone(),
                name_visible: l.name_visible,
                queued: s.link_queued().then(|| s.queued_text.clone()),
                portrait: l.portrait.clone(),
                show_icon: l.show_icon,
                show_portrait: l.show_portrait,
                text: l.text.clone(),
                typed: l.typed as usize,
                cursor: l.cursor,
                time: s.time,
            }),
        play: s.play.as_ref().map(|p| PlayView {
            first_person: p.first_person,
            name: p.name.clone(),
            text: p.text.clone(),
            player_line: p.player_line,
            choices: p
                .choices
                .iter()
                .map(|(c, skill)| {
                    if *skill {
                        let (sk, lvl) = c.skill.clone().unwrap_or_default();
                        (format!("~  {}  ({}:{})", c.text, sk, lvl), true)
                    } else {
                        (format!("~ {}", c.text), false)
                    }
                })
                .collect(),
            focus: p.focus,
            win_pos: p.win_pos,
            forced: p.forced,
        }),
    }
}

fn nine(
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    kind: &str,
) -> [Option<std::sync::Arc<crate::uicanvas::UiTex>>; 9] {
    let names = [
        "TL", "TR", "BL", "BR", "Left", "Right", "Top", "Bottom", "Center",
    ];
    names.map(|n| hud.ui(assets, &format!("HUDWindow{kind}_{n}")))
}

/// HUDInfoLinkDisplay. `face`: the portrait and its interference too; without them (Lean Updates) they are a window
/// of their own over this one (`draw_infolink_face`, classic_hud.rs's Win::InfoLinkFace), redrawn every frame.
pub fn draw_infolink(
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    c: &mut Canvas,
    l: &LinkView,
    face: bool,
) {
    draw_infolink_frame(hud, assets, c);
    draw_infolink_contents(hud, assets, c, l, face);
}

/// The InfoLink's background and border, the part that doesn't change with the transmission: with Lean Updates the
/// HUD keeps it drawn and starts each redraw from a copy of it (classic_hud.rs).
pub fn draw_infolink_frame(hud: &mut ClassicHud, assets: &mut RuntimeAssets, c: &mut Canvas) {
    let h = c.h;
    for (i, (x, w)) in [(0, 256), (256, 175)].into_iter().enumerate() {
        if let Some(t) = hud.ui(assets, &format!("HUDInfolinkBackground_{}", i + 1)) {
            c.texture(x, 0, w, h, 0, 0, &t, crate::menuui::bg_style(), crate::menuui::h_col(0));
        }
        if let Some(t) = hud.ui(assets, &format!("HUDInfolinkBorder_{}", i + 1)) {
            c.texture(x, 0, w, h, 0, 0, &t, crate::menuui::border_style(), crate::menuui::border_col());
        }
    }
}

/// Whether the typing cursor shows at `l.time`: it blinks every half second.
pub fn infolink_cursor_shown(l: &LinkView) -> bool {
    l.cursor && ((l.time / 0.5) as i32) % 2 == 0
}

/// What the InfoLink shows over its frame: the portrait (with `face`) or the datalink icon, the speaker, the queue
/// note and the typed text.
pub fn draw_infolink_contents(
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    c: &mut Canvas,
    l: &LinkView,
    face: bool,
) {
    // Portrait, or the datalink icon while the transmission comes in.
    if l.show_portrait {
        if face {
            draw_infolink_face(hud, assets, c, l, 20, 25);
        }
    } else if l.show_icon {
        if let Some(t) = hud.ui(assets, "DataLinkIcon") {
            c.texture(20, 25, 64, 64, 0, 0, &t, Style::Masked, [255, 255, 255]);
        }
    }
    if let Some(f) = hud.font(assets, "FontMenuHeaders_DS") {
        if l.name_visible {
            c.text(
                &f,
                94,
                15,
                293,
                15,
                HAlign::Left,
                VAlign::Center,
                crate::menuui::h_col(7),
                &l.name,
            );
        }
        if let Some(q) = &l.queued {
            c.text(
                &f,
                187,
                15,
                200,
                15,
                HAlign::Right,
                VAlign::Center,
                crate::menuui::h_col(8),
                q,
            );
        }
    }
    c.fill(94, 32, 293, 1, Style::Normal, crate::menuui::h_col(8));
    // ComputerWindow: 42 x 6 characters of 7 x 10, typed out; older lines scroll off the top.
    let Some(f) = hud.font(assets, "FontFixedWidthSmall_DS") else {
        return;
    };
    let shown: String = l.text.chars().take(l.typed).collect();
    let lines = wrap_cols(&shown, 42);
    let start = lines.len().saturating_sub(6);
    let mut last = (0, 0);
    for (row, line) in lines[start..].iter().enumerate() {
        let y = 36 + row as i32 * 10;
        for (col, ch) in line.chars().enumerate() {
            let mut b = [0u8; 4];
            c.text(
                &f,
                94 + col as i32 * 7,
                y,
                7,
                10,
                HAlign::Left,
                VAlign::Top,
                [255, 255, 255],
                ch.encode_utf8(&mut b),
            );
        }
        last = (line.chars().count() as i32, row as i32);
    }
    if infolink_cursor_shown(l) {
        let (cx, cy) = if lines.is_empty() { (0, 0) } else { last };
        c.fill(
            94 + cx * 7,
            36 + cy * 10 + 8,
            7,
            2,
            Style::Normal,
            crate::menuui::h_col(7),
        );
    }
}

/// The InfoLink's portrait at (x, y), with the StaticWindow's interference scrolling over it every frame.
pub fn draw_infolink_face(hud: &mut ClassicHud, assets: &mut RuntimeAssets, c: &mut Canvas, l: &LinkView, x: i32, y: i32) {
    if let Some(t) = l.portrait.as_ref().and_then(|p| hud.tex(assets, p)) {
        c.texture(x, y, 64, 64, 0, 0, &t, Style::Normal, [255, 255, 255]);
    }
    let stat = hud
        .tex(assets, "DeusExUI.UserInterface.Static")
        .or_else(|| hud.tex(assets, "DeusExUI.Static"));
    if let Some(t) = stat {
        let r = (l.time * 977.0) as i32;
        c.texture(x, y, 64, 64, r % 128, (r / 7) % 128, &t, Style::Translucent, [255, 255, 255]);
    }
}

/// Word wrap to a fixed number of columns (ComputerWindow bWordWrap).
fn wrap_cols(text: &str, cols: usize) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut cur = String::new();
        for word in para.split(' ') {
            let n = cur.chars().count();
            let wn = word.chars().count();
            if n > 0 && n + 1 + wn > cols {
                out.push(std::mem::take(&mut cur));
            }
            if !cur.is_empty() {
                cur.push(' ');
            }
            let mut w = word.to_string();
            while w.chars().count() > cols {
                let head: String = w.chars().take(cols).collect();
                out.push(head);
                w = w.chars().skip(cols).collect();
            }
            cur.push_str(&w);
        }
        out.push(cur);
    }
    out
}

/// HUDConWindowFirst: the size it asks for at a given width.
pub fn first_height(hud: &mut ClassicHud, assets: &mut RuntimeAssets, p: &PlayView, w: i32) -> i32 {
    let (Some(nf), Some(tf)) = (
        hud.font(assets, "FontMenuHeaders_DS"),
        hud.font(assets, "FontMenuSmall_DS"),
    ) else {
        return 40;
    };
    let lines = if p.text.is_empty() {
        0
    } else {
        wrap(&tf, &p.text, w - 40 - 20).len() as i32
    };
    let text_h = if lines > 0 { lines * tf.height + 10 } else { 0 };
    (10 + nf.height + 4 + 1 + text_h + 10).max(40)
}

pub fn draw_first(hud: &mut ClassicHud, assets: &mut RuntimeAssets, c: &mut Canvas, p: &PlayView) {
    let (w, h) = (c.w, c.h);
    let bg = nine(hud, assets, "Background");
    let border = nine(hud, assets, "Border");
    c.borders(0, 0, w, h, &bg, crate::menuui::bg_style(), crate::menuui::h_col(0));
    c.borders(0, 0, w, h, &border, crate::menuui::border_style(), crate::menuui::border_col());
    let (Some(nf), Some(tf)) = (
        hud.font(assets, "FontMenuHeaders_DS"),
        hud.font(assets, "FontMenuSmall_DS"),
    ) else {
        return;
    };
    let mut y = 10;
    c.text(
        &nf,
        20,
        y + 2,
        w - 40,
        nf.height,
        HAlign::Left,
        VAlign::Center,
        crate::menuui::h_col(7),
        &p.name,
    );
    y += nf.height + 4;
    c.fill(20, y, w - 40, 1, Style::Normal, crate::menuui::h_col(7));
    y += 1;
    if !p.text.is_empty() {
        y += 5;
        for line in wrap(&tf, &p.text, w - 40 - 20) {
            c.text(
                &tf,
                30,
                y,
                w - 60,
                tf.height,
                HAlign::Left,
                VAlign::Top,
                crate::menuui::h_col(8),
                &line,
            );
            y += tf.height;
        }
    }
}

/// HUDBarkDisplayItem rows: the name ("Name: ", FontMenuHeaders_DS) then the word-wrapped line beside it.
fn bark_rows(
    nf: &UiFont,
    tf: &UiFont,
    barks: &[(String, String)],
    w: i32,
) -> Vec<(String, i32, Vec<String>, i32)> {
    barks
        .iter()
        .map(|(name, text)| {
            let label = format!("{name}: ");
            let nw = nf.width(&label);
            let lines = wrap(tf, text, (w - 40 - nw).max(20));
            let h = (lines.len() as i32 * tf.height + 1).max(nf.height);
            (label, nw, lines, h)
        })
        .collect()
}

/// HUDBarkDisplay.ParentRequestedPreferredSize: topMargin 13, rows 2 apart, bottomMargin 10, at least 40.
pub fn bark_height(hud: &mut ClassicHud, assets: &mut RuntimeAssets, barks: &[(String, String)], w: i32) -> i32 {
    let (Some(nf), Some(tf)) = (
        hud.font(assets, "FontMenuHeaders_DS"),
        hud.font(assets, "FontMenuSmall_DS"),
    ) else {
        return 40;
    };
    let rows = bark_rows(&nf, &tf, barks, w);
    let body: i32 = rows.iter().map(|r| r.3).sum::<i32>() + 2 * (rows.len() as i32 - 1).max(0);
    (13 + body + 10).max(40)
}

pub fn draw_barks(hud: &mut ClassicHud, assets: &mut RuntimeAssets, c: &mut Canvas, barks: &[(String, String)]) {
    let (w, h) = (c.w, c.h);
    let bg = nine(hud, assets, "Background");
    let border = nine(hud, assets, "Border");
    c.borders(0, 0, w, h, &bg, crate::menuui::bg_style(), crate::menuui::h_col(0));
    c.borders(0, 0, w, h, &border, crate::menuui::border_style(), crate::menuui::border_col());
    let (Some(nf), Some(tf)) = (
        hud.font(assets, "FontMenuHeaders_DS"),
        hud.font(assets, "FontMenuSmall_DS"),
    ) else {
        return;
    };
    let mut y = 13;
    for (label, nw, lines, rh) in bark_rows(&nf, &tf, barks, w) {
        c.text(&nf, 20, y, nw, nf.height, HAlign::Left, VAlign::Top, crate::menuui::h_col(7), &label);
        let mut ly = y + 1;
        for line in &lines {
            c.text(&tf, 20 + nw, ly, w - 40 - nw, tf.height, HAlign::Left, VAlign::Top, crate::menuui::h_col(8), line);
            ly += tf.height;
        }
        y += rh + 2;
    }
}

/// ConWindowActive letterbox heights for a screen height (upper, lower).
pub fn letterbox(sh: i32) -> (i32, i32) {
    ((sh as f32 * 0.104) as i32, (sh as f32 * 0.21) as i32)
}

/// The cut-scene letterbox (CinematicWindow / ConWindowActive with bForcePlay): the screen cut to 16:9, both bars
/// the same. Deviation: on screens already 16:9 or wider the original's bars vanish and with them the
/// subtitles' room, so the bars never go below the normal upper bar's height.
pub fn cinematic_letterbox(sw: i32, sh: i32) -> i32 {
    let cin_h = (sw as f32 * 0.5625) as i32;
    ((sh - cin_h) / 2).max((sh as f32 * 0.104) as i32)
}

pub fn draw_upper(hud: &mut ClassicHud, assets: &mut RuntimeAssets, c: &mut Canvas) {
    let (w, h) = (c.w, c.h);
    c.fill(0, 0, w, h, Style::Normal, [0, 0, 0]);
    if let Some(t) = hud.ui(assets, "ConWindowActiveBackground") {
        c.texture(0, 0, w, h, 0, 0, &t, Style::Normal, [255, 255, 255]);
    }
}

fn con_fonts(
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    sw: i32,
) -> Option<(std::sync::Arc<UiFont>, std::sync::Arc<UiFont>)> {
    // ConPlay.GetCurrentSpeechFont: the large fonts between 800 and 1280 wide.
    let large = sw > 800 && sw < 1280;
    let (s, n) = if large {
        ("FontConversationLarge", "FontConversationLargeBold")
    } else {
        ("FontConversation", "FontConversationBold")
    };
    Some((hud.font(assets, s)?, hud.font(assets, n)?))
}

/// Where the choices were drawn last frame, in screen UI units (top, bottom), for the mouse.
pub static CHOICE_ROWS: std::sync::Mutex<Vec<(i32, i32)>> = std::sync::Mutex::new(Vec::new());
/// The lower letterbox's top this frame (screen UI units).
pub static LOWER_Y: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// The lower letterbox: the speech window (name, then the text beside it), then the choices.
pub fn draw_lower(
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    c: &mut Canvas,
    p: &PlayView,
    sw: i32,
) {
    let (w, h) = (c.w, c.h);
    // TileWindow background: ConWindowActiveBackground, tile colour black.
    c.fill(0, 0, w, h, Style::Normal, [0, 0, 0]);
    let Some((sf, nf)) = con_fonts(hud, assets, sw) else {
        return;
    };
    let mut y = 2;
    // Every choice has to fit (deviation: the original clipped them at the bottom - Gunther's third choice after the
    // rescue). The choices' room first; the speech above them only if it fits too, else just its first line; rows
    // tighten as a last resort.
    let hotkeys = setting!("DialogueHotkeys") > 0.5;
    let mut pad = 4;
    let choice_room = |pad: i32| p.choices.len() as i32 * (sf.height + pad);
    while pad > 0 && choice_room(pad) + 2 > h {
        pad -= 1;
    }
    let text_room = (h - 2 - choice_room(pad)).max(0);
    if (!p.text.is_empty() || !p.name.is_empty()) && (p.choices.is_empty() || text_room >= sf.height.max(nf.height)) {
        let label = if p.name.is_empty() {
            String::new()
        } else {
            format!("{}: ", p.name)
        };
        let nw = nf.width(&label);
        if !label.is_empty() {
            c.text(
                &nf,
                2,
                y,
                nw,
                nf.height,
                HAlign::Left,
                VAlign::Top,
                COL_NAME,
                &label,
            );
        }
        let col = if p.player_line {
            COL_SPEECH_PLAYER
        } else {
            COL_SPEECH
        };
        let mut lines = wrap(&sf, &p.text, (w - 4 - nw).max(40));
        if !p.choices.is_empty() {
            let fit = (text_room / sf.height.max(1)).max(0) as usize;
            if lines.len() > fit {
                lines.truncate(fit);
                if let Some(l) = lines.last_mut() {
                    l.push_str("...");
                }
            }
        }
        let mut ty = y;
        for line in &lines {
            c.text(
                &sf,
                2 + nw,
                ty,
                w - 4 - nw,
                sf.height,
                // ConWindowSpeech.SetForcePlay: HALIGN_Center.
                if p.forced { HAlign::Center } else { HAlign::Left },
                VAlign::Top,
                col,
                line,
            );
            ty += sf.height;
        }
        y = (y + nf.height).max(ty);
    }
    let top = LOWER_Y.load(std::sync::atomic::Ordering::Relaxed);
    let mut rows = Vec::new();
    if !p.choices.is_empty() {
        y = y.min((h - choice_room(pad)).max(0));
    }
    // Enhancement "Dialogue Hotkeys": the first four choices are numbered.
    for (i, (text, skill)) in p.choices.iter().enumerate() {
        let bh = sf.height + pad;
        rows.push((top + y, top + y + bh));
        let focus = i == p.focus;
        if focus {
            c.fill(2, y, w - 4, bh, Style::Normal, COL_CHOICE);
        }
        let col = if focus {
            COL_FOCUS
        } else if *skill {
            COL_SKILL
        } else {
            COL_CHOICE
        };
        // Dialogue Hotkeys: "1." for the keys, or the pad's button (X / Y / LB / RB) when it was last used.
        let mut tx = 12;
        let shown = if hotkeys && i < 4 && crate::gamepad::pad_in_use() {
            let kind = crate::gamepad::glyph_kind();
            let b = crate::gamepad::DIALOGUE_HOTKEYS[i].0;
            crate::glyphs::draw_small(c, kind, b, tx, y + (bh - 11) / 2);
            tx += crate::glyphs::width_small(kind, b) + 4;
            text.clone()
        } else if hotkeys && i < 4 {
            format!("{}. {text}", i + 1)
        } else {
            text.clone()
        };
        c.text(&sf, tx, y, w - tx - 12, bh, HAlign::Left, VAlign::Center, col, &shown);
        y += bh;
    }
    if let Ok(mut r) = CHOICE_ROWS.lock() {
        *r = rows;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CONUI_PREVIEW=<out.rgba> HUD_GAME=<game root>: an infolink, a first-person window and the letterbox on a
    /// 960x540 UI at 2x (raw RGBA, 1920x1080).
    #[test]
    fn preview() {
        let (Ok(out), Ok(root)) = (std::env::var("CONUI_PREVIEW"), std::env::var("HUD_GAME"))
        else {
            return;
        };
        let root = std::path::PathBuf::from(root);
        let mut assets = RuntimeAssets {
            classes: unreal_pkg::classes::ClassDb::open(&root.join("System")),
            lib: dx_level::TexLib::new(&root),
            images: Default::default(),
            materials: Default::default(),
            fire: Vec::new(),
            sprites: Default::default(),
            shared_parts: Default::default(),
        };
        let mut hud = ClassicHud::default();
        let (sw, sh, s) = (960, 540, 2);
        let mut frame = vec![[0.15f32, 0.18, 0.22]; (sw * s * sh * s) as usize];
        let mut blit = |c: &Canvas, x: i32, y: i32| {
            for py in 0..c.h * s {
                for px in 0..c.w * s {
                    let (fx, fy) = (x * s + px, y * s + py);
                    if fx < 0 || fy < 0 || fx >= sw * s || fy >= sh * s {
                        continue;
                    }
                    let p = c.px[(py * c.w * s + px) as usize];
                    let f = &mut frame[(fy * sw * s + fx) as usize];
                    for k in 0..3 {
                        f[k] = (p[k] + f[k] * (1.0 - p[3])).min(1.0);
                    }
                }
            }
        };
        let link = LinkView {
            name: "Alex Jacobson".into(),
            name_visible: true,
            queued: Some("message waiting...".into()),
            portrait: Some("InfoPortraits.AlexJacobson".into()),
            show_icon: false,
            show_portrait: true,
            text: "Heads up, JC.  Your brother Paul is on his way to meet you at the dock.  The NSF is raiding the island, and shots have been fired.  I repeat: find Paul.".into(),
            typed: 120,
            cursor: true,
            time: 0.2,
        };
        let mut c = Canvas::new(INFOLINK_SIZE.0, INFOLINK_SIZE.1, s);
        draw_infolink(&mut hud, &mut assets, &mut c, &link, true);
        blit(&c, 104, 0);
        let fp = PlayView {
            first_person: true,
            name: "Harley Filben".into(),
            text: "Get away from me, you're gonna get me killed!".into(),
            ..Default::default()
        };
        let fw = (sw - 100).min(800);
        let fh = first_height(&mut hud, &mut assets, &fp, fw);
        let mut c = Canvas::new(fw, fh, s);
        draw_first(&mut hud, &mut assets, &mut c, &fp);
        blit(&c, sw / 2 - fw / 2, 140);
        let tp = PlayView {
            first_person: false,
            name: "Paul Denton".into(),
            text: "Remember that we're police.  Stick with the prod.  It will stun your opponents or knock them unconscious.".into(),
            choices: vec![
                ("~ I like to pick 'em off from a distance.  I'll take the rifle.".into(), false),
                ("~ Never know when I might come up against heavy armor.  Give me the GEP gun.".into(), false),
                ("~ The crossbow.  Sometimes you've got to make a silent takedown.".into(), false),
            ],
            focus: 1,
            win_pos: 1.0,
            ..Default::default()
        };
        let (uh, lh) = letterbox(sh);
        let mut c = Canvas::new(sw, uh, s);
        draw_upper(&mut hud, &mut assets, &mut c);
        blit(&c, 0, 0);
        let mut c = Canvas::new(sw, lh, s);
        draw_lower(&mut hud, &mut assets, &mut c, &tp, sw);
        blit(&c, 0, sh - lh);
        let bytes: Vec<u8> = frame
            .iter()
            .flat_map(|p| {
                [
                    (p[0] * 255.0) as u8,
                    (p[1] * 255.0) as u8,
                    (p[2] * 255.0) as u8,
                    255,
                ]
            })
            .collect();
        std::fs::write(out, bytes).unwrap();
    }

    /// HUD_GAME=<game root>: with Lean Updates the InfoLink's portrait is a window of its own (classic_hud.rs,
    /// Win::InfoLinkFace at (20, 25)); laid over the InfoLink drawn without it, it must give the whole InfoLink.
    #[test]
    fn infolink_split_draws_the_whole_infolink() {
        let Ok(root) = std::env::var("HUD_GAME") else {
            return;
        };
        let root = std::path::PathBuf::from(root);
        let mut assets = RuntimeAssets {
            classes: unreal_pkg::classes::ClassDb::open(&root.join("System")),
            lib: dx_level::TexLib::new(&root),
            images: Default::default(),
            materials: Default::default(),
            fire: Vec::new(),
            sprites: Default::default(),
            shared_parts: Default::default(),
        };
        let mut hud = ClassicHud::default();
        assert!(hud.tex(&mut assets, "InfoPortraits.AlexJacobson").is_some(), "no portrait in HUD_GAME");
        let s = 2;
        // As the HUD draws it: the frame once, then each InfoLink from a copy of it, without the portrait.
        let mut frame = Canvas::new(INFOLINK_SIZE.0, INFOLINK_SIZE.1, s);
        draw_infolink_frame(&mut hud, &mut assets, &mut frame);
        // The interference at four moments: the typing cursor shown (0.2, 0.37) and between blinks (0.6, 1.9).
        for time in [0.2f32, 0.37, 0.6, 1.9] {
            let link = LinkView {
                name: "Alex Jacobson".into(),
                name_visible: true,
                queued: Some("message waiting...".into()),
                portrait: Some("InfoPortraits.AlexJacobson".into()),
                show_icon: false,
                show_portrait: true,
                text: "Heads up, JC.  Your brother Paul is on his way to meet you at the dock.".into(),
                typed: 40,
                cursor: true,
                time,
            };
            let mut whole = Canvas::new(INFOLINK_SIZE.0, INFOLINK_SIZE.1, s);
            draw_infolink(&mut hud, &mut assets, &mut whole, &link, true);
            let mut split = frame.clone();
            draw_infolink_contents(&mut hud, &mut assets, &mut split, &link, false);
            let mut face = Canvas::new(64, 64, s);
            draw_infolink_face(&mut hud, &mut assets, &mut face, &link, 0, 0);
            // The face window over the InfoLink window, as the UI lays them (premultiplied: over).
            for py in 0..64 * s {
                for px in 0..64 * s {
                    let f = face.px[(py * 64 * s + px) as usize];
                    let b = &mut split.px[((25 * s + py) * split.w * s + 20 * s + px) as usize];
                    for k in 0..4 {
                        b[k] = f[k] + b[k] * (1.0 - f[3]);
                    }
                }
            }
            assert!(split.to_rgba8() == whole.to_rgba8(), "time {time}: the split InfoLink differs from the whole one");
        }
    }
}

/// A new map is going in (loader.rs): this module's state of the map being left.
pub fn reset_level() {
    if let Ok(mut v) = CHOICE_ROWS.lock() {
        v.clear();
    }
}
