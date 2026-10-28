//! The DX RUST launcher: point it at the Deus Ex folder (checked for System/DeusEx.u), choose the display mode and
//! resolution the game opens with, and start the game. Everything is kept next to the executables: the game folder
//! in launcher.txt (dxview reads it when it isn't given a folder), the display settings in settings.txt (the same
//! keys the game's Settings > Display menu writes).
//!
//! Once the folder is known the launcher draws itself as one of the original's menu screens - the Display
//! Settings window's art, title bar, action buttons, value boxes and bitmap fonts, in the menu colour theme the game
//! last saved - read from the player's own DeusExUI.u (nothing of the game ships with it) and drawn with the game's
//! own software UI canvas (dxview/src/uicanvas.rs). Before that it shows a plain folder picker.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[path = "../../dxview/src/uicanvas.rs"]
#[allow(dead_code)]
mod uicanvas;
mod art;

use art::Art;
use eframe::egui;
use std::path::{Path, PathBuf};
use uicanvas::{Canvas, HAlign, Rgb, Style, VAlign};

const TITLE: &str = "DX RUST Launcher";

/// The same list as dxview's Settings > Display (menus::RESOLUTIONS).
const RESOLUTIONS: &[(u32, u32)] = &[
    (640, 480), (800, 600), (1024, 768), (1152, 864), (1280, 720), (1280, 800), (1280, 960), (1280, 1024),
    (1366, 768), (1440, 900), (1600, 900), (1600, 1200), (1680, 1050), (1920, 1080), (1920, 1200), (2560, 1080),
    (2560, 1440), (3440, 1440), (3840, 2160),
];
/// settings.txt DisplayMode 0 / 1 / 2 (short enough for the original's 98-unit value box).
const MODES: [&str; 3] = ["Fullscreen", "Windowed", "Borderless"];

fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)).unwrap_or_else(|| ".".into())
}

fn valid_game(p: &Path) -> bool {
    unreal_pkg::gamedir::is_game_dir(p)
}

fn read_kv(path: &Path) -> Vec<(String, String)> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
        .collect()
}

fn write_kv(path: &Path, header: &str, kv: &[(String, String)]) -> std::io::Result<()> {
    let mut out = format!("# {header}\n");
    for (k, v) in kv {
        out.push_str(&format!("{k}={v}\n"));
    }
    std::fs::write(path, out)
}

// ------------------------------------------------------------------------------------------------ window layout
// MenuScreenDisplay's frame (UI units): client 391 x 408 on four 256 tiles, rows from y 27 every 36, value boxes
// at x 270 (98 wide), help line at 354. Five rows are used; the four spare choice wells are cut out (like dxview's
// Frame::cuts) and the window closes up.

const TITLE_H: i32 = 23;
const LEFT_BOTTOM: (i32, i32) = (10, 53);
const BAR_H: i32 = 21;
const CW: i32 = 391;
const CH_ART: i32 = 408;
const CUT: (i32, i32) = (27 + 36 * 5 - 9, 36 * 4);
const CH: i32 = CH_ART - CUT.1;
const HELP_Y: i32 = 354 - CUT.1;
const MARGIN: i32 = 8;
const ROW_X: i32 = 7;
const INFO_X: i32 = 270;
const INFO_W: i32 = 98;
const LABEL_W: i32 = 243;
/// Whole-number UI scale (the game's 2x at 1080p).
const SCALE: i32 = 2;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Wid {
    Folder,
    Path,
    Mode,
    Res,
    Mod,
    Play,
    Quit,
    Save,
}

type Rect = (i32, i32, i32, i32);

fn inside(r: Rect, p: (i32, i32)) -> bool {
    p.0 >= r.0 && p.1 >= r.1 && p.0 < r.0 + r.2 && p.1 < r.1 + r.3
}

struct Launcher {
    game: String,
    /// The Lay D Denton Project's folder (its System and Maps are laid over the game's; empty = none). Saved as
    /// LDDPPath; other mods get rows of their own later.
    lddp: String,
    mode: usize,
    res: usize,
    status: String,
    art: Option<Art>,
    theme: usize,
    translucent: bool,
    /// The folder's edit field: insertion point and whether it has the keyboard.
    cur: usize,
    editing: bool,
    hits: Vec<(Wid, Rect, bool)>,
    hover: Option<Wid>,
    pressed: Option<Wid>,
    tex: Option<egui::TextureHandle>,
    sized: bool,
}

impl Launcher {
    fn load() -> Launcher {
        let dir = exe_dir();
        let lk = read_kv(&dir.join("launcher.txt"));
        // DXL_GAME overrides the saved folder (for DXL_DUMP checks).
        let game = std::env::var("DXL_GAME").ok().unwrap_or_else(|| {
            lk.iter().find(|(k, _)| k == "GamePath").map(|(_, v)| v.clone()).unwrap_or_else(|| {
                // The usual Steam (every library, Steam Deck too), GOG and Heroic places.
                unreal_pkg::gamedir::detect().map(|p| p.display().to_string()).unwrap_or_default()
            })
        });
        let st = read_kv(&dir.join("settings.txt"));
        let get = |k: &str| st.iter().find(|(a, _)| a == k).and_then(|(_, v)| v.parse::<f32>().ok());
        let mode = get("DisplayMode").map(|v| v.round() as usize).unwrap_or(1).min(2);
        let res = get("Resolution").filter(|v| *v >= 0.0).map(|v| v as usize).unwrap_or(10).min(RESOLUTIONS.len() - 1);
        let lddp = lk.iter().find(|(k, _)| k == "LDDPPath").map(|(_, v)| v.clone()).unwrap_or_default();
        let mut l = Launcher {
            lddp,
            cur: game.chars().count(),
            game,
            mode,
            res,
            status: String::new(),
            art: None,
            theme: art::theme_from_settings(&st),
            translucent: art::translucent_from_settings(&st),
            editing: false,
            hits: Vec::new(),
            hover: None,
            pressed: None,
            tex: None,
            sized: false,
        };
        l.try_art();
        l
    }

    /// Load the menu art once the folder is a Deus Ex install.
    fn try_art(&mut self) {
        if self.art.is_none() && valid_game(Path::new(&self.game)) {
            self.art = Art::load(Path::new(&self.game), self.theme);
        }
    }

    fn set_game(&mut self, g: String) {
        self.game = g;
        self.cur = self.game.chars().count();
        self.status.clear();
        self.try_art();
    }

    fn save(&mut self) -> bool {
        let dir = exe_dir();
        let mut lk = vec![("GamePath".to_string(), self.game.clone())];
        if !self.lddp.is_empty() {
            lk.push(("LDDPPath".into(), self.lddp.clone()));
        }
        if let Err(e) = write_kv(&dir.join("launcher.txt"), "DX RUST launcher", &lk) {
            self.status = format!("Can't save launcher.txt: {e}");
            return false;
        }
        // Keep every other setting the game wrote; only the display ones change.
        let path = dir.join("settings.txt");
        let mut kv = read_kv(&path);
        for (k, v) in [("DisplayMode", self.mode.to_string()), ("Resolution", self.res.to_string())] {
            match kv.iter_mut().find(|(a, _)| a == k) {
                Some(x) => x.1 = v,
                None => kv.push((k.to_string(), v)),
            }
        }
        if let Err(e) = write_kv(&path, "DX RUST settings (written by the Settings menus)", &kv) {
            self.status = format!("Can't save settings.txt: {e}");
            return false;
        }
        true
    }

    fn play(&mut self) -> bool {
        if !valid_game(Path::new(&self.game)) {
            self.status = "That folder has no System\\DeusEx.u - choose the Deus Ex folder.".into();
            return false;
        }
        if !self.save() {
            return false;
        }
        let exe = exe_dir().join(if cfg!(windows) { "dxview.exe" } else { "dxview" });
        match std::process::Command::new(&exe).arg(&self.game).current_dir(exe_dir()).spawn() {
            Ok(_) => true,
            Err(e) => {
                self.status = format!("Can't start {}: {e}", exe.display());
                false
            }
        }
    }

    fn browse_lddp(&mut self) {
        let Some(p) = rfd::FileDialog::new().set_title("The Lay D Denton Project folder (with System and Maps)").pick_folder() else {
            return;
        };
        match find_lddp(&p) {
            Some(dir) => self.lddp = dir.display().to_string(),
            None => {
                rfd::MessageDialog::new()
                    .set_title("Not Lay D Denton")
                    .set_description(
                        "That folder doesn't hold the Lay D Denton Project (its System folder with FemJC.u). Choose the \
                         folder you unpacked the mod into, such as Lay_D_Denton_Project_1.1.",
                    )
                    .set_level(rfd::MessageLevel::Warning)
                    .show();
            }
        }
    }

    /// What the Lay D Denton row shows.
    fn lddp_label(&self) -> String {
        if self.lddp.is_empty() {
            "Off".into()
        } else if find_lddp(Path::new(&self.lddp)).is_some() {
            "On".into()
        } else {
            "Not Found".into()
        }
    }

    fn browse(&mut self) {
        if let Some(p) = rfd::FileDialog::new().set_title("The Deus Ex folder").pick_folder() {
            self.set_game(p.display().to_string());
        }
    }

    fn enabled(&self, w: Wid) -> bool {
        w != Wid::Play || valid_game(Path::new(&self.game))
    }

    fn help(&self) -> String {
        if !self.status.is_empty() {
            return self.status.clone();
        }
        match self.hover {
            Some(Wid::Folder) => "Choose the folder your own copy of Deus Ex is installed in",
            Some(Wid::Path) => "Type or paste the path of the Deus Ex folder",
            Some(Wid::Mode) => "Change how the game's window is shown",
            Some(Wid::Res) => "Change the video resolution",
            Some(Wid::Mod) => "Play with the Lay D Denton Project: choose its folder (right click to turn it off)",
            Some(Wid::Play) => "Save these settings and start the game",
            Some(Wid::Save) => "Keep these settings without starting the game",
            Some(Wid::Quit) => "Close the launcher",
            None if !valid_game(Path::new(&self.game)) => "There is no System\\DeusEx.u in that folder",
            None => "",
        }
        .to_string()
    }

    /// The canvas size in UI units.
    fn size() -> (i32, i32) {
        (MARGIN * 2 + LEFT_BOTTOM.0 + CW + 4, MARGIN * 2 + TITLE_H + CH + BAR_H)
    }

    // -------------------------------------------------------------------------------------------- drawing

    fn draw(&mut self, blink: bool) -> Option<Canvas> {
        let (w, h) = Self::size();
        let mut c = Canvas::new(w, h, SCALE);
        c.fill(0, 0, w, h, Style::Normal, [0, 0, 0]);
        self.hits.clear();
        let help = self.help();
        let found = valid_game(Path::new(&self.game));
        let state = |s: &Launcher, wd: Wid| -> usize {
            if !s.enabled(wd) {
                3
            } else if s.pressed == Some(wd) && s.hover == Some(wd) {
                2
            } else if s.hover == Some(wd) {
                1
            } else {
                0
            }
        };
        let st: Vec<(Wid, usize)> =
            [Wid::Folder, Wid::Path, Wid::Mode, Wid::Res, Wid::Mod, Wid::Play, Wid::Quit, Wid::Save].iter().map(|w| (*w, state(self, *w))).collect();
        let stof = |w: Wid| st.iter().find(|x| x.0 == w).map_or(0, |x| x.1);
        let (editing, cur, game) = (self.editing, self.cur, self.game.clone());
        let (mode, res, translucent) = (self.mode, self.res, self.translucent);
        let mod_label = self.lddp_label();
        let a = self.art.as_mut()?;
        let (ox, oy) = (MARGIN, MARGIN);
        let face = a.col(3);
        let title = a.title.clone();
        let small = a.small.clone();
        let tw = (31 + title.width(TITLE).min(400) + 45).max(200);
        // Right edge (under everything else).
        {
            let (rx, ry) = (ox + tw + 1, oy + TITLE_H - 4);
            let rw = CW - (tw - LEFT_BOTTOM.0) - 1 + 4;
            let rh = CH + 4 + 14;
            draw(&mut c, a, "MenuRightBorder_TopLeft", rx, ry, 2, 4, Style::Masked, face);
            draw(&mut c, a, "MenuRightBorder_Top", rx + 2, ry, rw - 2 - 5, 4, Style::Masked, face);
            draw(&mut c, a, "MenuRightBorder_TopRight", rx + rw - 5, ry, 5, 6, Style::Masked, face);
            draw(&mut c, a, "MenuRightBorder_Right", rx + rw - 4, ry + 6, 4, rh - 6 - 13, Style::Masked, face);
            draw(&mut c, a, "MenuRightBorder_BottomRight", rx + rw - 4, ry + rh - 13, 4, 13, Style::Masked, face);
        }
        // Title bar.
        for (set, col) in [("Bar", a.col(3)), ("Bubble", a.col(1))] {
            draw(&mut c, a, &format!("MenuTitle{set}_Left"), ox, oy, 87, TITLE_H, Style::Masked, col);
            if set == "Bar" {
                draw(&mut c, a, "MenuTitleBar_LeftBottom", ox, oy + TITLE_H - 1, LEFT_BOTTOM.0, LEFT_BOTTOM.1, Style::Masked, col);
            }
            draw(&mut c, a, &format!("MenuTitle{set}_Center"), ox + 87, oy, tw - 187, TITLE_H, Style::Masked, col);
            draw(&mut c, a, &format!("MenuTitle{set}_Right"), ox + tw - 100, oy, 100, TITLE_H, Style::Masked, col);
        }
        c.text(&title, ox + 31, oy, tw - 31, TITLE_H, HAlign::Left, VAlign::Center, a.col(2), TITLE);
        if let Some(t) = a.ui("MenuIcon_DeusEx") {
            c.texture(ox + 12, oy + 3, t.w as i32, t.h as i32, 0, 0, &t, Style::Masked, [255, 255, 255]);
        }
        // Client art: the band above the cut in place, the band below it moved up.
        let (cx, cy) = (ox + LEFT_BOTTOM.0, oy + TITLE_H);
        let style = if translucent { Style::Translucent } else { Style::Masked };
        for (by, bh, shift) in [(0, CUT.0, 0), (CUT.0, CH - CUT.0, CUT.1)] {
            c.set_clip(Some((cx, cy + by, CW, bh)));
            for (i, n) in ["MenuDisplayBackground_1", "MenuDisplayBackground_2", "MenuDisplayBackground_3", "MenuDisplayBackground_4"]
                .iter()
                .enumerate()
            {
                if let Some(t) = a.ui(n) {
                    let (col, row) = (i as i32 % 2, i as i32 / 2);
                    c.texture(cx + col * 256, cy + row * 256 - shift, t.w as i32, t.h as i32, 0, 0, &t, style, a.col(0));
                }
            }
        }
        c.set_clip(None);
        let mut hits = Vec::new();
        let row_y = |i: i32| cy + 27 + i * 36;
        // Row 0: the folder button and whether the game is there.
        action_button(&mut c, a, cx + ROW_X, row_y(0), LABEL_W, "Game Folder...", stof(Wid::Folder));
        hits.push((Wid::Folder, (cx + ROW_X, row_y(0), LABEL_W, 19), true));
        let (ftxt, fcol) = if found { ("Found", a.btn(0)) } else { ("Not Found", [255, 0, 0]) };
        info_button(&mut c, a, cx + ROW_X + INFO_X, row_y(0), INFO_W, ftxt, fcol);
        // Row 1: the folder path (an edit field across the row).
        {
            let (x, y, w) = (cx + ROW_X, row_y(1), INFO_X + INFO_W);
            // The box's middle is see-through and this row spans art that isn't a dark well: darken it first.
            c.fill(x + 3, y + 3, w - 8, 13, Style::Normal, [0, 0, 0]);
            info_button(&mut c, a, x, y, w, "", a.col(8));
            let (tx, ty, tww) = (x + 4, y + 4, w - 8);
            let chars: Vec<char> = game.chars().collect();
            let before: String = chars[..cur.min(chars.len())].iter().collect();
            let cur_x = small.width(&before);
            let scroll = (cur_x - (tww - 2)).max(0);
            let tcol = if stof(Wid::Path) >= 1 || editing { a.btn(1) } else { a.btn(0) };
            c.set_clip(Some((tx, ty - 1, tww, 13)));
            c.text(&small, tx - scroll, ty, tww + scroll, 11, HAlign::Left, VAlign::Center, tcol, &game);
            if editing && blink {
                c.fill(tx - scroll + cur_x, ty, 1, 11, Style::Normal, a.col(12));
            }
            c.set_clip(None);
            hits.push((Wid::Path, (x, y, w, 19), true));
        }
        // Rows 2 and 3: display mode and resolution (MenuUIChoiceEnum: click for the next value, right click back).
        for (i, wd, lbl, val) in [
            (2, Wid::Mode, "Display Mode", MODES[mode].to_string()),
            (3, Wid::Res, "Screen Resolution", format!("{}x{}", RESOLUTIONS[res].0, RESOLUTIONS[res].1)),
        ] {
            action_button(&mut c, a, cx + ROW_X, row_y(i), LABEL_W, lbl, stof(wd));
            let vcol = a.btn(if stof(wd) >= 1 { 1 } else { 0 });
            info_button(&mut c, a, cx + ROW_X + INFO_X, row_y(i), INFO_W, &val, vcol);
            hits.push((wd, (cx + ROW_X, row_y(i), INFO_X + INFO_W, 19), true));
        }
        // Row 4: the mod folder.
        {
            action_button(&mut c, a, cx + ROW_X, row_y(4), LABEL_W, "Lay D Denton...", stof(Wid::Mod));
            let vcol = a.btn(if stof(Wid::Mod) >= 1 { 1 } else { 0 });
            info_button(&mut c, a, cx + ROW_X + INFO_X, row_y(4), INFO_W, &mod_label, vcol);
            hits.push((Wid::Mod, (cx + ROW_X, row_y(4), INFO_X + INFO_W, 19), true));
        }
        // Action bar: Save on the left, Play and Quit on the right.
        {
            let (bx, by, bw) = (cx + 1, cy + CH + 1, CW - 1 - 2);
            let mut right = bw;
            for (wd, lbl) in [(Wid::Play, "Play"), (Wid::Quit, "Quit")] {
                let w = action_width(a, lbl);
                right -= w;
                action_button(&mut c, a, bx + right, by, w, lbl, stof(wd));
                hits.push((wd, (bx + right, by, w, 19), stof(wd) != 3));
                right -= 2;
            }
            let w = action_width(a, "Save");
            action_button(&mut c, a, bx, by, w, "Save", stof(Wid::Save));
            hits.push((Wid::Save, (bx, by, w, 19), true));
        }
        // Left edge.
        {
            let (ex, ey, eh) = (cx - 4, oy + TITLE_H + LEFT_BOTTOM.1, CH + BAR_H - LEFT_BOTTOM.1 - 1);
            draw(&mut c, a, "MenuLeftBorder_Top", ex, ey, 4, 2, Style::Masked, face);
            draw(&mut c, a, "MenuLeftBorder_Center", ex, ey + 2, 4, eh - 2 - 16, Style::Masked, face);
            draw(&mut c, a, "MenuLeftBorder_Bottom", ex, ey + eh - 16, 4, 16, Style::Masked, face);
        }
        // Help line (MenuUIHelpWindow: margins 10, FontMenuSmall).
        if !help.is_empty() {
            let (hx, hy, hw, hh) = (cx + 7, cy + HELP_Y, CW - 21, 27);
            c.set_clip(Some((hx, hy, hw, hh)));
            let lines = uicanvas::wrap(&small, &help, hw - 20);
            let lh = small.height;
            let mut ly = hy + (hh - lines.len() as i32 * lh) / 2;
            for l in lines {
                c.text(&small, hx + 10, ly, hw - 20, lh, HAlign::Left, VAlign::Top, a.col(7), &l);
                ly += lh;
            }
            c.set_clip(None);
        }
        self.hits = hits;
        Some(c)
    }

    fn hit_at(&self, p: (i32, i32)) -> Option<(Wid, bool)> {
        self.hits.iter().rev().find(|h| inside(h.1, p)).map(|h| (h.0, h.2))
    }

    fn activate(&mut self, w: Wid, back: bool, ctx: &egui::Context) {
        self.status.clear();
        match w {
            Wid::Folder => self.browse(),
            Wid::Path => {
                self.editing = true;
                self.cur = self.game.chars().count();
            }
            Wid::Mode => self.mode = (self.mode + if back { MODES.len() - 1 } else { 1 }) % MODES.len(),
            Wid::Res => self.res = (self.res + if back { RESOLUTIONS.len() - 1 } else { 1 }) % RESOLUTIONS.len(),
            Wid::Mod if back => self.lddp.clear(),
            Wid::Mod => self.browse_lddp(),
            Wid::Play => {
                if self.play() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Wid::Save => {
                if self.save() {
                    self.status = "Settings saved.".into();
                }
            }
            Wid::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    /// Keys for the folder's edit field (MenuUIEditWindow: arrows, Home / End, Backspace / Delete, typing, paste).
    fn edit_keys(&mut self, ctx: &egui::Context) {
        let events = ctx.input(|i| i.events.clone());
        let mut chars: Vec<char> = self.game.chars().collect();
        let mut changed = false;
        for e in events {
            match e {
                egui::Event::Text(t) | egui::Event::Paste(t) => {
                    for ch in t.chars().filter(|c| !c.is_control()) {
                        chars.insert(self.cur, ch);
                        self.cur += 1;
                        changed = true;
                    }
                }
                egui::Event::Key { key, pressed: true, .. } => match key {
                    egui::Key::ArrowLeft => self.cur = self.cur.saturating_sub(1),
                    egui::Key::ArrowRight => self.cur = (self.cur + 1).min(chars.len()),
                    egui::Key::Home => self.cur = 0,
                    egui::Key::End => self.cur = chars.len(),
                    egui::Key::Backspace if self.cur > 0 => {
                        chars.remove(self.cur - 1);
                        self.cur -= 1;
                        changed = true;
                    }
                    egui::Key::Delete if self.cur < chars.len() => {
                        chars.remove(self.cur);
                        changed = true;
                    }
                    egui::Key::Enter | egui::Key::Tab | egui::Key::Escape => self.editing = false,
                    _ => {}
                },
                _ => {}
            }
        }
        if changed {
            let cur = self.cur;
            self.set_game(chars.into_iter().collect());
            self.cur = cur;
        }
    }

    /// Before the folder is known there's no art to draw with: a plain picker.
    fn plain(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(TITLE);
            ui.add_space(8.0);
            ui.label("Choose the folder your own copy of Deus Ex is installed in:");
            ui.horizontal(|ui| {
                let mut g = self.game.clone();
                if ui.add(egui::TextEdit::singleline(&mut g).desired_width(300.0)).changed() {
                    self.set_game(g);
                }
                if ui.button("Browse...").clicked() {
                    self.browse();
                }
            });
            ui.label(if valid_game(Path::new(&self.game)) {
                "Found System\\DeusEx.u, but its DeusExUI.u couldn't be read."
            } else {
                "There is no System\\DeusEx.u in that folder yet."
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(valid_game(Path::new(&self.game)), egui::Button::new("Play")).clicked() && self.play() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if ui.button("Quit").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            if !self.status.is_empty() {
                ui.label(&self.status);
            }
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn draw(c: &mut Canvas, a: &mut Art, name: &str, x: i32, y: i32, w: i32, h: i32, style: Style, col: Rgb) {
    if w > 0 && h > 0 {
        if let Some(t) = a.ui(name) {
            c.texture(x, y, w, h, 0, 0, &t, style, col);
        }
    }
}

fn action_width(a: &Art, s: &str) -> i32 {
    (8 + a.title.width(s) + 11).max(83)
}

/// MenuUIActionButtonWindow / MenuUIChoiceButton (19 high, FontMenuTitle). State: normal, focus, pressed, disabled.
fn action_button(c: &mut Canvas, a: &mut Art, x: i32, y: i32, width: i32, s: &str, st: usize) {
    let set = if st == 2 { "Pressed" } else { "Normal" };
    let face = a.col(3);
    draw(c, a, &format!("MenuActionButton{set}_Left"), x, y, 8, 19, Style::Masked, face);
    draw(c, a, &format!("MenuActionButton{set}_Center"), x + 8, y, width - 8 - 11, 19, Style::Masked, face);
    draw(c, a, &format!("MenuActionButton{set}_Right"), x + width - 11, y, 11, 19, Style::Masked, face);
    let off = if st == 2 { 1 } else { 0 };
    let f = a.title.clone();
    c.text(&f, x + 8 + off, y + 1 + off, width, 18, HAlign::Left, VAlign::Center, a.btn(st), s);
}

/// MenuUIInfoButtonWindow: the box behind edit fields and enum values.
fn info_button(c: &mut Canvas, a: &mut Art, x: i32, y: i32, width: i32, s: &str, col: Rgb) {
    let face = a.col(3);
    draw(c, a, "MenuInfoButton_Left", x, y, 6, 19, Style::Masked, face);
    draw(c, a, "MenuInfoButton_Center", x + 6, y, width - 6 - 11, 19, Style::Masked, face);
    draw(c, a, "MenuInfoButton_Right", x + width - 11, y, 11, 19, Style::Masked, face);
    if !s.is_empty() {
        let f = a.title.clone();
        c.text(&f, x + 8, y + 1, width - 8, 18, HAlign::Left, VAlign::Center, col, s);
    }
}

impl eframe::App for Launcher {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.art.is_none() {
            self.plain(ctx);
            return;
        }
        // Pixel-exact: one egui point per physical pixel, the canvas carries its own whole-number scale.
        if ctx.pixels_per_point() != 1.0 {
            ctx.set_pixels_per_point(1.0);
        }
        let (w, h) = Self::size();
        let (pw, ph) = ((w * SCALE) as f32, (h * SCALE) as f32);
        if !self.sized {
            self.sized = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(pw, ph)));
        }
        let ui_pos = ctx.input(|i| i.pointer.latest_pos()).map(|p| ((p.x as i32) / SCALE, (p.y as i32) / SCALE));
        let (down_l, down_r, up_l, up_r) = ctx.input(|i| {
            (
                i.pointer.button_pressed(egui::PointerButton::Primary),
                i.pointer.button_pressed(egui::PointerButton::Secondary),
                i.pointer.button_released(egui::PointerButton::Primary),
                i.pointer.button_released(egui::PointerButton::Secondary),
            )
        });
        let under = ui_pos.and_then(|p| self.hit_at(p));
        self.hover = under.filter(|u| u.1).map(|u| u.0);
        if down_l || down_r {
            self.pressed = self.hover;
            if self.hover != Some(Wid::Path) {
                self.editing = false;
            }
        }
        if up_l || up_r {
            if let (Some(p), Some(h)) = (self.pressed, self.hover) {
                if p == h {
                    self.activate(p, up_r, ctx);
                }
            }
            self.pressed = None;
        }
        if self.editing {
            self.edit_keys(ctx);
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        } else {
            let (enter, esc) = ctx.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
            if enter {
                self.activate(Wid::Play, false, ctx);
            } else if esc {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        let blink = ctx.input(|i| i.time).rem_euclid(1.0) < 0.5;
        let Some(c) = self.draw(blink) else { return };
        let img = egui::ColorImage::from_rgba_premultiplied([(w * SCALE) as usize, (h * SCALE) as usize], &c.to_rgba8());
        match &mut self.tex {
            Some(t) => t.set(img, egui::TextureOptions::NEAREST),
            None => self.tex = Some(ctx.load_texture("launcher", img, egui::TextureOptions::NEAREST)),
        }
        let tex = self.tex.as_ref().unwrap().id();
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(egui::Color32::BLACK)).show(ctx, |ui| {
            let r = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(pw, ph));
            ui.painter().image(tex, r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        });
    }
}

/// The Lay D Denton Project's folder: `dir` itself or a folder just inside it (an unpacked zip often adds one) with
/// System/FemJC.u, any case.
fn find_lddp(dir: &Path) -> Option<std::path::PathBuf> {
    let has = |d: &Path| -> bool {
        let Ok(rd) = std::fs::read_dir(d) else { return false };
        rd.flatten().any(|e| {
            e.file_name().to_string_lossy().eq_ignore_ascii_case("System")
                && std::fs::read_dir(e.path())
                    .map(|r| r.flatten().any(|f| f.file_name().to_string_lossy().eq_ignore_ascii_case("FemJC.u")))
                    .unwrap_or(false)
        })
    };
    if has(dir) {
        return Some(dir.to_path_buf());
    }
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.is_dir() && has(p))
}

fn main() -> eframe::Result<()> {
    let mut app = Launcher::load();
    // DXL_DUMP=out.ppm: draw the window once (hovering DXL_HOVER = folder/path/mode/res/play/quit/save) and exit.
    if let Ok(out) = std::env::var("DXL_DUMP") {
        app.hover = match std::env::var("DXL_HOVER").unwrap_or_default().as_str() {
            "folder" => Some(Wid::Folder),
            "path" => Some(Wid::Path),
            "mode" => Some(Wid::Mode),
            "res" => Some(Wid::Res),
            "play" => Some(Wid::Play),
            "quit" => Some(Wid::Quit),
            "save" => Some(Wid::Save),
            _ => None,
        };
        app.editing = std::env::var("DXL_EDIT").is_ok();
        let Some(c) = app.draw(true) else {
            eprintln!("no art: {} isn't a readable Deus Ex folder", app.game);
            std::process::exit(1);
        };
        let (w, h) = (c.w * c.s, c.h * c.s);
        let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
        for p in c.to_rgba8().chunks_exact(4) {
            ppm.extend_from_slice(&p[..3]);
        }
        std::fs::write(out, ppm).expect("write dump");
        return Ok(());
    }
    let (w, h) = if app.art.is_some() {
        let (w, h) = Launcher::size();
        ((w * SCALE) as f32, (h * SCALE) as f32)
    } else {
        (460.0, 200.0)
    };
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([w, h])
            .with_resizable(false)
            .with_title(TITLE)
            .with_icon(std::sync::Arc::new(egui::IconData {
                rgba: include_bytes!("../../../assets/dxrust_48.rgba").to_vec(),
                width: 48,
                height: 48,
            })),
        ..Default::default()
    };
    eframe::run_native(TITLE, opts, Box::new(|_| Ok(Box::new(app))))
}
