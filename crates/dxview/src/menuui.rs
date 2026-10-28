//! The MenuUI kit: the original menus' and computers' widgets (MenuUITitleWindow, MenuUIClientWindow,
//! MenuUIActionButtonWindow, MenuUIInfoButtonWindow, MenuUIListHeaderButtonWindow, MenuUIEditWindow,
//! MenuUIListWindow + MenuUIScrollAreaWindow, MenuUINormalLargeTextWindow) drawn on a `uicanvas::Canvas`, with
//! hit rectangles for a caller's own widget ids. Shared by computers.rs and menus.rs.

use crate::classic_hud::{ClassicHud, HudLayer};
use crate::inventory_screen::{blank, label, upload};
use crate::uicanvas::{wrap, Canvas, HAlign, Rgb, Style, UiFont, UiTex, VAlign};
use crate::weapons::RuntimeAssets;
use bevy::prelude::*;
use std::sync::Arc;

/// Widget ids a kit user supplies: the kit makes list rows and scroll bar parts itself.
pub(crate) trait KitWid: Copy + PartialEq {
    fn row(list: u8, i: usize) -> Self;
    fn scroll_up(id: u8) -> Self;
    fn scroll_down(id: u8) -> Self;
    fn scroll_track(id: u8) -> Self;
}

// ---------------------------------------------------------------- colour themes

/// DeusExPlayer.CreateColorThemeManager's order (ColorThemeMenu_<name>).
pub(crate) const MENU_THEMES: &[&str] = &[
    "Default", "BlueAndGold", "CoolGreen", "Cops", "Cyan", "DesertStorm", "DriedBlood", "Dusk", "Earth", "Green",
    "Grey", "IonStorm", "Lava", "NightVision", "Ninja", "Olive", "PaleGreen", "Pastel", "Plasma", "Primaries",
    "Purple", "Red", "Seawater", "SoylentGreen", "Starlight", "Steel", "SteelGreen", "Superhero", "Terminator",
    "Violet",
];
/// ColorThemeHUD_<name>.
pub(crate) const HUD_THEMES: &[&str] = &[
    "Default", "Amber", "Cops", "Cyan", "DarkBlue", "DesertStorm", "DriedBlood", "Dusk", "Grey", "IonStorm",
    "NightVision", "Ninja", "PaleGreen", "Pastel", "Plasma", "Primaries", "Purple", "Red", "SoylentGreen",
    "Starlight", "SteelGreen", "Superhero", "Terminator", "Violet",
];
/// The themes on offer: the game's, then Lay D Denton's own (ColorThemeMenu_LDDP / ColorThemeHUD_LDDP, which it adds
/// second; ours go last so saved theme numbers keep meaning the same theme).
pub(crate) fn menu_themes() -> Vec<&'static str> {
    let mut v = MENU_THEMES.to_vec();
    if crate::travel::lddp() {
        v.push("LDDP");
    }
    v
}
pub(crate) fn hud_themes() -> Vec<&'static str> {
    let mut v = HUD_THEMES.to_vec();
    if crate::travel::lddp() {
        v.push("LDDP");
    }
    v
}
/// MenuThemeName / HUDThemeName defaults ("Grey").
pub(crate) const DEFAULT_MENU_THEME: usize = 10;
pub(crate) const DEFAULT_HUD_THEME: usize = 8;

/// ColorThemeMenu_Grey: Background, TitleBackground, TitleText, ButtonFace, ButtonTextNormal, ButtonTextFocus,
/// ButtonTextDisabled, HelpText, ListText, ListTextHighlight, ListHighlight, ListFocus, Cursor.
const GREY_MENU: [Rgb; 13] = [
    [255, 255, 255],
    [0, 0, 255],
    [255, 255, 255],
    [255, 255, 255],
    [200, 200, 200],
    [255, 255, 255],
    [50, 50, 50],
    [200, 200, 200],
    [200, 200, 200],
    [0, 0, 0],
    [128, 128, 128],
    [64, 64, 64],
    [255, 255, 255],
];
/// ColorThemeHUD_Grey: Background, Borders, TitleText, ButtonFace, ButtonTextNormal, ButtonTextFocus,
/// ButtonTextDisabled, HeaderText, NormalText, ListText, ListTextHighlight, ListHighlight, ListFocus, Cursor.
const GREY_HUD: [Rgb; 14] = [
    [127, 127, 127],
    [255, 255, 255],
    [255, 255, 255],
    [127, 127, 127],
    [200, 200, 200],
    [255, 255, 255],
    [64, 64, 64],
    [255, 255, 255],
    [200, 200, 200],
    [200, 200, 200],
    [200, 200, 200],
    [128, 128, 128],
    [255, 255, 255],
    [255, 255, 255],
];
static MENU_COLORS: std::sync::RwLock<[Rgb; 13]> = std::sync::RwLock::new(GREY_MENU);
static HUD_COLORS: std::sync::RwLock<[Rgb; 14]> = std::sync::RwLock::new(GREY_HUD);

/// The current menu theme's colour (MenuColor_*), by index.
pub(crate) fn m_col(i: usize) -> Rgb {
    MENU_COLORS.read().map(|c| c[i]).unwrap_or([255, 255, 255])
}
/// Button text by state: normal, focus, pressed, disabled.
pub(crate) fn m_btn(state: usize) -> Rgb {
    m_col([4, 5, 5, 6][state.min(3)])
}
/// HUD button text by state: normal, focus, pressed, disabled.
pub(crate) fn h_btn(state: usize) -> Rgb {
    h_col([4, 5, 5, 6][state.min(3)])
}
/// Colors > HUD Background Translucency: backgrounds add (translucent) or cover (masked).
pub(crate) fn bg_style() -> Style {
    if crate::settings::hud_translucent() {
        Style::Translucent
    } else {
        Style::Masked
    }
}
/// Colors > HUD Borders Visible / HUD Borders Translucency (hidden: drawn adding black, i.e. not at all).
pub(crate) fn border_style() -> Style {
    if !crate::settings::hud_borders() || crate::settings::hud_border_translucent() {
        Style::Translucent
    } else {
        Style::Masked
    }
}
pub(crate) fn border_col() -> Rgb {
    if crate::settings::hud_borders() {
        h_col(1)
    } else {
        [0, 0, 0]
    }
}
/// The current HUD theme's colour (HUDColor_*), by index.
pub(crate) fn h_col(i: usize) -> Rgb {
    HUD_COLORS.read().map(|c| c[i]).unwrap_or([255, 255, 255])
}

fn theme_colors<const N: usize>(assets: &mut RuntimeAssets, class: &str) -> [Rgb; N] {
    let d = assets.classes.defaults(class);
    let mut out = [[0u8; 3]; N];
    for p in d.iter().filter(|p| p.name.eq_ignore_ascii_case("Colors")) {
        if let unreal_pkg::props::Value::Struct(_, b) = &p.value {
            if let Some(o) = out.get_mut(p.index as usize) {
                if b.len() >= 3 {
                    *o = [b[0], b[1], b[2]];
                }
            }
        }
    }
    out
}

/// ColorThemeManager.SetMenuThemeByName / SetHUDThemeByName.
/// The (menu, HUD) themes last applied, to tell which one the player just changed.
static LAST: std::sync::Mutex<Option<(usize, usize)>> = std::sync::Mutex::new(None);

/// The HUD palette for a menu theme without a ColorThemeHUD_ twin, from its MenuColor_* roles.
fn hud_from_menu(mc: &[Rgb; 13]) -> [Rgb; 14] {
    let half = |c: Rgb| [c[0] / 2, c[1] / 2, c[2] / 2];
    [
        half(mc[0]), // Background
        mc[0],       // Borders
        mc[2],       // TitleText
        half(mc[3]), // ButtonFace
        mc[4],
        mc[5],
        mc[6],
        mc[5],  // HeaderText
        mc[7],  // NormalText
        mc[8],  // ListText
        mc[9],  // ListTextHighlight
        mc[10], // ListHighlight
        mc[11], // ListFocus
        mc[12], // Cursor
    ]
}

pub(crate) fn apply_themes(assets: &mut RuntimeAssets, menu: usize, hud: usize) {
    let m = menu_themes().get(menu).copied().unwrap_or("Grey");
    let mc: [Rgb; 13] = theme_colors(assets, &format!("ColorThemeMenu_{m}"));
    // Deviation (user request 2026-10-21): the menu theme recolours the HUD and inventory screens too - with the
    // HUD theme of the same name when there is one, else with colours derived from the menu theme - until the
    // player picks a HUD theme of their own (HUDFollowsMenu 0); picking a menu theme again follows again.
    let mut hud = hud;
    let mut follow = crate::settings::get("HUDFollowsMenu") > 0.5;
    if let Some((lm, lh)) = LAST.lock().ok().and_then(|l| *l) {
        if menu != lm {
            follow = true;
        } else if hud != lh {
            follow = false;
        }
        crate::settings::set("HUDFollowsMenu", if follow { 1.0 } else { 0.0 });
    }
    let mut derived = false;
    if follow {
        match hud_themes().iter().position(|h| h.eq_ignore_ascii_case(m)) {
            Some(h) => {
                hud = h;
                if crate::settings::get("HUDColor") as usize != h {
                    crate::settings::set("HUDColor", h as f32);
                }
            }
            None => derived = true,
        }
    }
    if let Ok(mut l) = LAST.lock() {
        *l = Some((menu, hud));
    }
    let h = hud_themes().get(hud).copied().unwrap_or("Grey");
    let hc: [Rgb; 14] = if derived { hud_from_menu(&mc) } else { theme_colors(assets, &format!("ColorThemeHUD_{h}")) };
    if let Ok(mut w) = MENU_COLORS.write() {
        *w = mc;
    }
    if let Ok(mut w) = HUD_COLORS.write() {
        *w = hc;
    }
}
pub(crate) const HACK_DIGITS: Rgb = [0, 128, 0];
pub(crate) const HACK_RED: Rgb = [255, 0, 0];
pub(crate) const WHITE: Rgb = [255, 255, 255];
pub(crate) const BLACK: Rgb = [0, 0, 0];

pub(crate) type Rect = (i32, i32, i32, i32);

pub(crate) const ROW_H: i32 = 12;
pub(crate) const TITLE_H: i32 = 23;
pub(crate) const TITLE_LEFT_BOTTOM: (i32, i32) = (10, 53);
pub(crate) const BAR_H: i32 = 21;

// ------------------------------------------------------------------------------------------------ edit fields

/// MenuUIEditWindow: single line, insertion point, Shift selection, max length.
#[derive(Clone, Debug, Default)]
pub(crate) struct Edit {
    pub(crate) text: Vec<char>,
    pub(crate) cur: usize,
    pub(crate) anchor: usize,
    pub(crate) max: usize,
    /// Horizontal scroll (UI units) so the insertion point stays visible.
    pub(crate) scroll: i32,
    /// Where the text was drawn last (screen x), for clicks.
    pub(crate) org: i32,
}

impl Edit {
    pub(crate) fn new(max: usize) -> Edit {
        Edit { max, ..default() }
    }
    pub(crate) fn get(&self) -> String {
        self.text.iter().collect()
    }
    pub(crate) fn clear(&mut self) {
        self.text.clear();
        self.cur = 0;
        self.anchor = 0;
        self.scroll = 0;
    }
    pub(crate) fn sel(&self) -> (usize, usize) {
        (self.cur.min(self.anchor), self.cur.max(self.anchor))
    }
    pub(crate) fn delete_sel(&mut self) -> bool {
        let (a, b) = self.sel();
        if a == b {
            return false;
        }
        self.text.drain(a..b);
        self.cur = a;
        self.anchor = a;
        true
    }
    /// One key press (repeats included). Returns true when the text changed.
    pub(crate) fn key(&mut self, k: KeyCode, text: Option<&str>, shift: bool, ctrl: bool) -> bool {
        let n = self.text.len();
        let mv = |e: &mut Edit, to: usize| {
            e.cur = to;
            if !shift {
                e.anchor = to;
            }
        };
        match k {
            KeyCode::ArrowLeft => {
                let to = self.cur.saturating_sub(1);
                mv(self, to);
                false
            }
            KeyCode::ArrowRight => {
                let to = (self.cur + 1).min(n);
                mv(self, to);
                false
            }
            KeyCode::Home => {
                mv(self, 0);
                false
            }
            KeyCode::End => {
                mv(self, n);
                false
            }
            KeyCode::Backspace => {
                if self.delete_sel() {
                    return true;
                }
                if self.cur > 0 {
                    self.text.remove(self.cur - 1);
                    self.cur -= 1;
                    self.anchor = self.cur;
                    return true;
                }
                false
            }
            KeyCode::Delete => {
                if self.delete_sel() {
                    return true;
                }
                if self.cur < n {
                    self.text.remove(self.cur);
                    return true;
                }
                false
            }
            KeyCode::KeyA if ctrl => {
                self.anchor = 0;
                self.cur = n;
                false
            }
            _ => {
                let Some(t) = text else { return false };
                if ctrl {
                    return false;
                }
                let chars: Vec<char> = t.chars().filter(|c| !c.is_control()).collect();
                if chars.is_empty() {
                    return false;
                }
                let had = self.delete_sel();
                let mut changed = had;
                for c in chars {
                    if self.text.len() >= self.max {
                        break;
                    }
                    self.text.insert(self.cur, c);
                    self.cur += 1;
                    changed = true;
                }
                self.anchor = self.cur;
                changed
            }
        }
    }
}


// ------------------------------------------------------------------------------------------------ drawing kit

pub(crate) struct Fonts {
    pub(crate) title: Arc<UiFont>,
    pub(crate) small: Arc<UiFont>,
    pub(crate) headers: Arc<UiFont>,
    pub(crate) fixed: Arc<UiFont>,
}

/// One canvas being drawn, with its hit rectangles (screen coordinates).
pub(crate) struct Ui<'a, W: KitWid> {
    pub(crate) c: Canvas,
    pub(crate) hud: &'a mut ClassicHud,
    pub(crate) assets: &'a mut RuntimeAssets,
    pub(crate) f: &'a Fonts,
    pub(crate) o: (i32, i32),
    pub(crate) hits: &'a mut Vec<(W, Rect, bool)>,
    pub(crate) pressed: Option<W>,
    pub(crate) hover: Option<W>,
    pub(crate) focus: Option<W>,
}

impl<W: KitWid> Ui<'_, W> {
    pub(crate) fn hit(&mut self, w: W, r: Rect, enabled: bool) {
        self.hits.push((w, (r.0 + self.o.0, r.1 + self.o.1, r.2, r.3), enabled));
    }
    pub(crate) fn tex(&mut self, name: &str) -> Option<Arc<UiTex>> {
        self.hud.ui(self.assets, name)
    }
    pub(crate) fn draw(&mut self, name: &str, x: i32, y: i32, w: i32, h: i32, style: Style, col: Rgb) {
        if w <= 0 || h <= 0 {
            return;
        }
        if let Some(t) = self.tex(name) {
            self.c.texture(x, y, w, h, 0, 0, &t, style, col);
        }
    }
    /// DrawIcon: the texture at its own size.
    pub(crate) fn icon(&mut self, name: &str, x: i32, y: i32, style: Style, col: Rgb) {
        if let Some(t) = self.tex(name) {
            self.c.texture(x, y, t.w as i32, t.h as i32, 0, 0, &t, style, col);
        }
    }
    pub(crate) fn state(&self, w: W, enabled: bool) -> usize {
        if !enabled {
            3
        } else if self.pressed == Some(w) && self.hover == Some(w) {
            2
        } else if self.focus == Some(w) {
            1
        } else {
            0
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn text(&mut self, font: &UiFont, x: i32, y: i32, w: i32, h: i32, ha: HAlign, va: VAlign, col: Rgb, s: &str) {
        self.c.text(font, x, y, w, h, ha, va, col, &label(s));
    }
    /// A TextWindow with word wrap: lines, aligned in the box.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn wrapped(&mut self, font: &UiFont, x: i32, y: i32, w: i32, h: i32, ha: HAlign, va: VAlign, col: Rgb, s: &str) {
        let lines = wrap(font, &label(s), w);
        let lh = font.height;
        let total = lines.len() as i32 * lh;
        let mut ly = match va {
            VAlign::Top => y,
            VAlign::Center => y + (h - total) / 2,
            VAlign::Bottom => y + h - total,
        };
        for l in lines {
            self.c.text(font, x, ly, w, lh, ha, VAlign::Top, col, &l);
            ly += lh;
        }
    }
}

pub(crate) fn action_width(f: &Fonts, s: &str) -> i32 {
    (8 + f.title.width(&label(s)) + 11).max(83)
}

/// MenuUIActionButtonWindow / MenuUIChoiceButton (19 high, FontMenuTitle).
#[allow(clippy::too_many_arguments)]
pub(crate) fn action_button<W: KitWid>(u: &mut Ui<W>, w: W, x: i32, y: i32, width: i32, s: &str, enabled: bool) {
    let st = u.state(w, enabled);
    let set = if st == 2 { "Pressed" } else { "Normal" };
    let (lw, rw) = (8, 11);
    u.draw(&format!("MenuActionButton{set}_Left"), x, y, lw, 19, Style::Masked, m_col(3));
    if let Some(t) = u.tex(&format!("MenuActionButton{set}_Center")) {
        u.c.texture(x + lw, y, width - lw - rw, 19, 0, 0, &t, Style::Masked, m_col(3));
    }
    u.draw(&format!("MenuActionButton{set}_Right"), x + width - rw, y, rw, 19, Style::Masked, m_col(3));
    let off = if st == 2 { 1 } else { 0 };
    let f = u.f.title.clone();
    u.text(&f, x + 8 + off, y + 1 + off, width, 19 - 1, HAlign::Left, VAlign::Center, m_btn(st), s);
    u.hit(w, (x, y, width, 19), enabled);
}

/// MenuUIInfoButtonWindow: the box behind edit fields and enum values.
pub(crate) fn info_button<W: KitWid>(u: &mut Ui<W>, x: i32, y: i32, width: i32, s: &str, col: Rgb) {
    u.draw("MenuInfoButton_Left", x, y, 6, 19, Style::Masked, m_col(3));
    if let Some(t) = u.tex("MenuInfoButton_Center") {
        u.c.texture(x + 6, y, width - 6 - 11, 19, 0, 0, &t, Style::Masked, m_col(3));
    }
    u.draw("MenuInfoButton_Right", x + width - 11, y, 11, 19, Style::Masked, m_col(3));
    if !s.is_empty() {
        let f = u.f.title.clone();
        u.text(&f, x + 8, y + 1, width - 8, 18, HAlign::Left, VAlign::Center, col, s);
    }
}

/// MenuUIListHeaderButtonWindow (15 high, FontMenuHeaders).
pub(crate) fn header_button<W: KitWid>(u: &mut Ui<W>, w: W, x: i32, y: i32, width: i32, s: &str) {
    let st = u.state(w, true);
    let set = if st == 2 { "Pressed" } else { "Normal" };
    u.draw(&format!("MenuListHeaderButton{set}_Left"), x, y, 5, 15, Style::Masked, m_col(3));
    if let Some(t) = u.tex(&format!("MenuListHeaderButton{set}_Center")) {
        u.c.texture(x + 5, y, width - 5 - 13, 15, 0, 0, &t, Style::Masked, m_col(3));
    }
    u.draw(&format!("MenuListHeaderButton{set}_Right"), x + width - 13, y, 13, 15, Style::Masked, m_col(3));
    let off = if st == 2 { 1 } else { 0 };
    let f = u.f.headers.clone();
    u.text(&f, x + 8 + off, y + 1 + off, width, 14, HAlign::Left, VAlign::Center, m_btn(st.min(2)), s);
    u.hit(w, (x, y, width, 15), true);
}

/// CreateMenuEditWindow: an info button box with the edit text clipped inside it.
pub(crate) fn edit_field<W: KitWid>(u: &mut Ui<W>, w: W, x: i32, y: i32, width: i32, e: &mut Edit, focused: bool, enabled: bool) {
    info_button(u, x, y, width, "", m_col(8));
    let f = u.f.small.clone();
    let (tx, ty, tw, th) = (x + 4, y + 4, width - 8, 11);
    let cur_x = f.width(&e.text[..e.cur].iter().collect::<String>());
    if cur_x - e.scroll > tw - 2 {
        e.scroll = cur_x - tw + 2;
    }
    if cur_x < e.scroll {
        e.scroll = cur_x;
    }
    e.org = tx - e.scroll + u.o.0;
    u.c.set_clip(Some((tx, ty, tw, th)));
    let (a, b) = e.sel();
    let s: String = e.text.iter().collect();
    if focused && a != b {
        let sx = f.width(&e.text[..a].iter().collect::<String>());
        let sw = f.width(&e.text[a..b].iter().collect::<String>());
        u.c.fill(tx - e.scroll + sx, ty, sw, th, Style::Normal, m_col(8));
    }
    let mut cx = tx - e.scroll;
    for (i, ch) in s.chars().enumerate() {
        let col = if focused && i >= a && i < b { BLACK } else { m_col(8) };
        let one = ch.to_string();
        u.c.text(&f, cx, ty, 100, th, HAlign::Left, VAlign::Top, col, &one);
        cx += f.width(&one);
    }
    if focused && enabled {
        u.c.fill(tx - e.scroll + cur_x, ty, 1, th, Style::Normal, m_col(12));
    }
    u.c.set_clip(None);
    u.hit(w, (x, y, width, 19), enabled);
}

/// MenuUIScrollAreaWindow's vertical scroll bar (only when the content doesn't fit). Returns the width left
/// for the content.
#[allow(clippy::too_many_arguments)]
pub(crate) fn scrollbar<W: KitWid>(u: &mut Ui<W>, id: u8, x: i32, y: i32, w: i32, h: i32, content: i32, scroll: &mut i32) -> i32 {
    if content <= h {
        *scroll = 0;
        return w;
    }
    *scroll = (*scroll).clamp(0, content - h);
    let sx = x + w - 15;
    let up = if u.pressed == Some(W::scroll_up(id)) { "Pressed" } else { "Normal" };
    let down = if u.pressed == Some(W::scroll_down(id)) { "Pressed" } else { "Normal" };
    u.draw(&format!("MenuVScrollUpButton_{up}"), sx, y, 15, 15, Style::Masked, m_col(3));
    u.draw(&format!("MenuVScrollDownButton_{down}"), sx, y + h - 15, 15, 15, Style::Masked, m_col(3));
    let (ty, th) = (y + 15, h - 30);
    if let Some(t) = u.tex("MenuVScrollScale") {
        u.c.texture(sx, ty, 15, th, 0, 0, &t, Style::Masked, m_col(3));
    }
    let thumb = ((th * h) / content).clamp(14, th.max(14));
    let pos = ty + ((th - thumb) * *scroll) / (content - h).max(1);
    u.draw("MenuVScrollThumb_Top", sx + 3, pos, 9, 6, Style::Masked, m_col(3));
    if let Some(t) = u.tex("MenuVScrollThumb_Center") {
        u.c.texture(sx + 3, pos + 6, 9, thumb - 12, 0, 0, &t, Style::Masked, m_col(3));
    }
    u.draw("MenuVScrollThumb_Bottom", sx + 3, pos + thumb - 6, 9, 6, Style::Masked, m_col(3));
    u.hit(W::scroll_track(id), (sx, ty, 15, th), true);
    u.hit(W::scroll_up(id), (sx, y, 15, 15), true);
    u.hit(W::scroll_down(id), (sx, y + h - 15, 15, 15), true);
    w - 15
}

/// A ListWindow: rows of columns, the selected row highlighted.
#[allow(clippy::too_many_arguments)]
pub(crate) fn list<W: KitWid>(u: &mut Ui<W>, id: u8, r: Rect, cols: &[i32], rows: &[Vec<String>], sel: Option<usize>, scroll: &mut i32, hud_style: bool) {
    let (x, y, w, h) = r;
    let content = rows.len() as i32 * ROW_H;
    let inner = scrollbar(u, id, x, y, w, h, content, scroll);
    let f = u.f.small.clone();
    u.c.set_clip(Some((x, y, inner, h)));
    let (hi, hi_text, text) = if hud_style { (h_col(11), h_col(10), h_col(9)) } else { (m_col(10), m_col(9), m_col(8)) };
    for (i, row) in rows.iter().enumerate() {
        let ry = y + i as i32 * ROW_H - *scroll;
        if ry + ROW_H <= y || ry >= y + h {
            continue;
        }
        let selected = sel == Some(i);
        if selected {
            u.c.fill(x, ry, inner, ROW_H, Style::Normal, hi);
        }
        let mut cx = x;
        for (k, cw) in cols.iter().enumerate() {
            if let Some(s) = row.get(k) {
                u.c.set_clip(Some((cx.max(x), y.max(ry), (*cw).min(x + inner - cx), h)));
                u.c.text(&f, cx + 2, ry, *cw, ROW_H, HAlign::Left, VAlign::Center, if selected { hi_text } else { text }, s);
                u.c.set_clip(Some((x, y, inner, h)));
            }
            cx += cw;
        }
        let vy = ry.max(y);
        let vh = (ry + ROW_H).min(y + h) - vy;
        if vh > 0 {
            u.hit(W::row(id, i), (x, vy, inner, vh), true);
        }
    }
    u.c.set_clip(None);
}

/// A MenuUINormalLargeTextWindow in a scroll area (margins 4, 1; word wrap).
#[allow(clippy::too_many_arguments)]
pub(crate) fn text_area<W: KitWid>(u: &mut Ui<W>, id: u8, r: Rect, s: &str, center: bool, scroll: &mut i32) {
    let (x, y, w, h) = r;
    let f = u.f.small.clone();
    if center {
        u.wrapped(&f, x, y, w, h, HAlign::Center, VAlign::Center, m_col(8), s);
        return;
    }
    let mut lines = wrap(&f, &label(s), w - 8);
    let mut content = lines.len() as i32 * f.height + 2;
    let mut inner = w;
    if content > h {
        lines = wrap(&f, &label(s), w - 15 - 8);
        content = lines.len() as i32 * f.height + 2;
        inner = scrollbar(u, id, x, y, w, h, content, scroll);
    } else {
        *scroll = 0;
    }
    u.c.set_clip(Some((x, y, inner, h)));
    for (i, l) in lines.iter().enumerate() {
        let ly = y + 1 + i as i32 * f.height - *scroll;
        if ly + f.height < y || ly > y + h {
            continue;
        }
        u.c.text(&f, x + 4, ly, inner - 8, f.height, HAlign::Left, VAlign::Top, m_col(8), l);
    }
    u.c.set_clip(None);
    u.hit(W::scroll_track(id + 100), (x, y, inner, h), true);
}


// ---------------------------------------------------------------- canvas layers

pub(crate) struct Layer {
    pub(crate) img: Handle<Image>,
    pub(crate) e: Entity,
    pub(crate) last: Vec<u8>,
}


pub(crate) fn layer(commands: &mut Commands, images: &mut Assets<Image>, layers: &mut Assets<HudLayer>, z: i32) -> Layer {
    let img = images.add(blank(1, 1));
    let e = commands
        .spawn((
            MaterialNode(layers.add(HudLayer::new(img.clone()))),
            bevy::ui::Node { position_type: PositionType::Absolute, ..default() },
            Visibility::Hidden,
            Pickable::IGNORE,
            GlobalZIndex(z),
        ))
        .id();
    Layer { img, e, last: Vec::new() }
}

#[allow(clippy::type_complexity)]
pub(crate) fn place(
    l: &mut Layer,
    c: &Canvas,
    pos: (i32, i32),
    d: f32,
    sf: f32,
    images: &mut Assets<Image>,
    layers: &mut Assets<HudLayer>,
    nodes: &mut Query<(&mut bevy::ui::Node, &mut Visibility, &MaterialNode<HudLayer>), impl bevy::ecs::query::QueryFilter>,
) {
    // `d`: the shown scale (fractional with HUD Scale Large; the canvas is drawn at its own whole scale c.s).
    crate::hud::set_filter(images, &l.img, c.s, d);
    if upload(images, &l.img, &mut l.last, c) {
        if let Ok((_, _, m)) = nodes.get(l.e) {
            if let Some(m) = layers.get_mut(&m.0) {
                m.into_inner();
            }
        }
    }
    if let Ok((mut n, mut v, _)) = nodes.get_mut(l.e) {
        *v = Visibility::Inherited;
        n.left = Val::Px(pos.0 as f32 * d / sf);
        n.top = Val::Px(pos.1 as f32 * d / sf);
        n.width = Val::Px(c.w as f32 * d / sf);
        n.height = Val::Px(c.h as f32 * d / sf);
    }
}

