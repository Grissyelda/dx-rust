//! The Nameless Mod's in-game web (htmlWindow and its 147 htmlPageWindow classes: PlanetDeusEx, WorldCorp, ABI and
//! the rest). A page exists only as window-building code - InitWindow makes its text windows and hyperlink buttons,
//! DrawWindow places them and draws the pictures - so each page's own script is run once from the player's TNMGUI.u
//! as a data loader (uscript.rs with objects) against a recording window system, and what comes out is a flat list
//! of pictures, texts and links at their places. That list is what we draw and click; no script runs after that.
//! Text bodies come from TNMText by name (ProcessDeusExText). Interactive pages (the ABI accessory shop's buying,
//! the trivia quiz's answers) show, but only their links work.
// uc: htmlWindow.CreateWebPage, htmlWindow.ReplaceScreen, htmlWindow.goToPreviousPage, htmlWindow.goToNextPage,
// uc: htmlPageWindow.InitWindow, htmlPageWindow.ProcessDeusExText, TNMhyperlinkButton.HandlePress

use bevy::prelude::*;
use std::collections::{HashMap, HashSet};
use unreal_pkg::props::{find, Value};

use crate::menuui::{KitWid, Ui};
use crate::uicanvas::{HAlign, Rgb, Style, UiFont, VAlign};
use crate::uscript::V;
use crate::weapons::RuntimeAssets;

/// The scroll area's place in the screen and the page's width inside it.
const VIEW: (i32, i32, i32, i32) = (8, 21, 600, 400);
const PAGE_W: i32 = 585;

#[derive(Clone)]
enum Kind {
    /// DrawTexture (unscaled, wrapping from u, v) or DrawStretchedTexture (the region scaled to fit).
    Tex { name: String, uv: (i32, i32), region: Option<(i32, i32)>, masked: bool },
    /// Wrapped lines in a font.
    Text { font: String, lines: Vec<String>, color: Rgb, center: bool },
    /// Nothing to see: a link over something drawn.
    Area,
}

#[derive(Clone)]
struct Item {
    r: (i32, i32, i32, i32),
    kind: Kind,
    link: Option<String>,
}

#[derive(Clone, Default)]
pub struct Page {
    class: String,
    h: i32,
    bg: Option<String>,
    items: Vec<Item>,
    url: String,
}

#[derive(Default, Clone)]
struct Win {
    class: String,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    font: String,
    text: String,
    margins: (i32, i32),
    center: bool,
    color: Option<Rgb>,
    tex: Option<String>,
    link: Option<String>,
    hidden: bool,
    is_link: bool,
    is_text: bool,
}

const SELF: u32 = 0;
const PARENT: u32 = 1;
const GC: u32 = 2;
const PLAYER: u32 = 3;
const FLAGS: u32 = 4;
const INFO: u32 = 5;
const FIRST: u32 = 6;

struct Builder<'a> {
    assets: &'a mut RuntimeAssets,
    funcs: HashSet<String>,
    wins: Vec<Win>,
    members: HashMap<String, V>,
    draws: Vec<Item>,
    gc_font: String,
    gc_color: Rgb,
    gc_style: i64,
    fonts: HashMap<String, Option<UiFont>>,
    text_pkg: Option<unreal_pkg::Package>,
    flags: &'a dyn Fn(&str) -> bool,
    mission: i32,
    location: String,
    link_col: Rgb,
    hover_col: Rgb,
}

fn font_name(v: &V) -> String {
    let s = v.str();
    s.rsplit('.').next().unwrap_or(&s).to_string()
}

impl Builder<'_> {
    fn font(&mut self, name: &str) -> Option<UiFont> {
        if !self.fonts.contains_key(name) {
            let f = crate::classic_hud::load_font(self.assets, name);
            self.fonts.insert(name.to_string(), f);
        }
        self.fonts.get(name).cloned().flatten()
    }

    fn win(&mut self, id: u32) -> Option<&mut Win> {
        self.wins.get_mut(id as usize)
    }

    fn new_child(&mut self, class: &str) -> V {
        let l = class.rsplit('.').next().unwrap_or(class).to_ascii_lowercase();
        let is_link = l.contains("hyperlink") || l.contains("button");
        self.wins.push(Win {
            class: l.clone(),
            font: if is_link { "FontMenuSmall".into() } else { "FontMenuSmall".into() },
            is_link,
            is_text: l.contains("text"),
            ..Default::default()
        });
        V::Obj(self.wins.len() as u32 - 1)
    }

    fn color_of(&self, v: &V) -> Option<Rgb> {
        match v.str().as_str() {
            "#link" => Some(self.link_col),
            "#hover" => Some(self.hover_col),
            _ => None,
        }
    }

    /// ProcessDeusExText: the named text of TNMText, tags turned into plain paragraphs.
    fn text(&mut self, name: &str) -> String {
        if self.text_pkg.is_none() {
            let dir = self.assets.lib.system_dir().to_path_buf();
            self.text_pkg = unreal_pkg::Package::open(dir.join("TNMText.u")).ok();
        }
        let raw = self.text_pkg.as_ref().and_then(|p| p.text_named(name)).unwrap_or_default();
        let s = raw.replace(['\r', '\n', '\0'], "");
        let mut out = String::new();
        let mut rest = s.as_str();
        while let Some(lt) = rest.find('<') {
            out.push_str(&rest[..lt]);
            let Some(gt) = rest[lt..].find('>') else {
                rest = &rest[lt..];
                break;
            };
            let tag = rest[lt + 1..lt + gt].to_ascii_uppercase();
            if tag == "P" {
                out.push('\n');
            } else if tag == "PLAYERNAME" {
                out.push_str(&crate::travel::player_name());
            }
            rest = &rest[lt + gt + 1..];
        }
        out.push_str(rest);
        // ProcessBold: {bold} runs (the original shifts them into the font's bold half; we show them plain).
        out.replace(['{', '}'], "")
    }

    /// A window method (also the page's own unqualified calls, on window 0).
    fn win_method(&mut self, id: u32, name: &str, a: &[V]) -> Option<V> {
        let i = |k: usize| a.get(k).map_or(0, |v| v.int() as i32);
        match name {
            "setpos" => {
                let (x, y) = (i(0), i(1));
                let w = self.win(id)?;
                (w.x, w.y) = (x, y);
            }
            "setsize" => {
                let (ww, hh) = (i(0), i(1));
                let w = self.win(id)?;
                (w.w, w.h) = (ww, hh);
            }
            "setwidth" => self.win(id)?.w = i(0),
            "setheight" => self.win(id)?.h = i(0),
            "setfont" => self.win(id)?.font = font_name(a.first()?),
            "settext" | "setbuttontext" => self.win(id)?.text = a.first()?.str(),
            "appendtext" => {
                let t = a.first()?.str();
                self.win(id)?.text.push_str(&t);
            }
            "gettext" => return Some(V::Str(self.win(id)?.text.clone())),
            "settextmargins" => self.win(id)?.margins = (i(0), i(1)),
            "settextalignments" => self.win(id)?.center = i(0) == 1,
            "settextcolors" | "settextcolor" => {
                let c = self.color_of(a.first()?);
                self.win(id)?.color = c;
            }
            "setbuttontextures" | "setbackground" | "setclienttexture" => {
                let t = a.first().map(|v| v.str()).filter(|s| !s.is_empty());
                self.win(id)?.tex = t;
            }
            "show" => self.win(id)?.hidden = a.first().is_some_and(|v| !v.truthy() && !v.is_none()),
            "hide" => self.win(id)?.hidden = true,
            "newchild" => return Some(self.new_child(&a.first()?.str())),
            "setwordwrap" | "setwindowalignments" | "setbackgroundstyle" | "settitle" | "enablewordwrap" | "raise" | "lower"
            | "setsensitivity" | "setselectability" | "askparentforreconfigure" | "configurationchanged" | "setfocuswindow"
            | "setacceleratortext" | "setverticalspacing" | "setmaxlines" | "setlines" => {}
            _ => return None,
        }
        Some(V::None)
    }
}

fn style_of(n: i64) -> bool {
    // DSTY_Masked (2) and the blended styles keep the texture's holes.
    n >= 2
}

impl crate::uscript::Host for Builder<'_> {
    fn call(&mut self, name: &str, a: &[V]) -> Option<V> {
        // The text loader is ours whatever the script has for it (its own goes through the engine's text parser).
        if name == "processdeusextext" {
            let t = self.text(&a.first().map(|v| v.str()).unwrap_or_default());
            if let Some(V::Obj(id)) = a.get(1) {
                if let Some(w) = self.win(*id) {
                    w.text.push_str(&t);
                }
            }
            return Some(V::None);
        }
        if self.funcs.contains(name) {
            return None;
        }
        if let Some(v) = self.win_method(SELF, name, a) {
            return Some(v);
        }
        Some(match name {
            "getgc" => V::Obj(GC),
            "getrootwindow" | "getplayerpawn" => V::Obj(PLAYER),
            "addtimer" => V::Int(1),
            "rand" => V::Int(0),
            "frand" => V::Float(0.99),
            "len" => V::Int(a.first().map_or(0, |v| v.str().chars().count() as i64)),
            "min" => V::Int(a.iter().map(|v| v.int()).min().unwrap_or(0)),
            "max" => V::Int(a.iter().map(|v| v.int()).max().unwrap_or(0)),
            "fmin" => V::Float(a.iter().map(|v| v.float()).fold(f64::MAX, f64::min)),
            "fmax" => V::Float(a.iter().map(|v| v.float()).fold(f64::MIN, f64::max)),
            "cr" => V::Str("\n".into()),
            "caps" => V::Str(a.first().map(|v| v.str().to_uppercase()).unwrap_or_default()),
            "left" => V::Str(a.first().map(|v| v.str().chars().take(a.get(1).map_or(0, |n| n.int().max(0) as usize)).collect()).unwrap_or_default()),
            "right" => {
                let s: Vec<char> = a.first().map(|v| v.str().chars().collect()).unwrap_or_default();
                let n = a.get(1).map_or(0, |n| n.int().max(0) as usize).min(s.len());
                V::Str(s[s.len() - n..].iter().collect())
            }
            "instr" => {
                let (s, t) = (a.first().map(|v| v.str()).unwrap_or_default(), a.get(1).map(|v| v.str()).unwrap_or_default());
                V::Int(s.find(&t).map_or(-1, |i| s[..i].chars().count() as i64))
            }
            "sprintf" => {
                let mut s = a.first().map(|v| v.str()).unwrap_or_default();
                for v in a.iter().skip(1) {
                    for pat in ["%s", "%d", "%i", "%f"] {
                        if let Some(p) = s.find(pat) {
                            s.replace_range(p..p + 2, &v.str());
                            break;
                        }
                    }
                }
                V::Str(s)
            }
            "arraycount" => V::Int(64),
            // Anything else with one argument is a cast (TNMhyperlinkButton(x), Trestkon(Player)): the value itself.
            _ if a.len() == 1 => a[0].clone(),
            _ => V::None,
        })
    }

    fn get(&mut self, name: &str) -> V {
        match name {
            "winparent" => V::Obj(PARENT),
            "player" | "root" => V::Obj(PLAYER),
            "width" => V::Int(self.wins[0].w as i64),
            "height" => V::Int(self.wins[0].h as i64),
            "x" | "y" if !self.members.contains_key(name) => V::Int(0),
            "loadtimer" => V::Int(1000),
            "loadtimerid" => V::Int(1),
            "halign_left" | "valign_top" | "dsty_none" => V::Int(0),
            "halign_center" | "valign_center" | "dsty_normal" => V::Int(1),
            "halign_right" | "valign_bottom" | "dsty_masked" => V::Int(2),
            "halign_full" | "valign_full" | "dsty_translucent" => V::Int(3),
            "dsty_modulated" => V::Int(4),
            _ => self.members.get(name).cloned().unwrap_or(V::None),
        }
    }

    fn get_index(&mut self, name: &str, index: i64, _field: Option<&str>) -> V {
        self.members.get(&format!("{name}[{index}]")).cloned().unwrap_or(V::None)
    }

    fn set(&mut self, path: &str, v: &V) {
        if let Some((root, field)) = path.rsplit_once('.') {
            if let Some(V::Obj(id)) = self.members.get(root).cloned() {
                if field == "linkdestination" {
                    if let Some(w) = self.win(id) {
                        w.link = Some(v.str()).filter(|s| !s.is_empty());
                    }
                }
            }
            return;
        }
        self.members.insert(path.to_string(), v.clone());
    }

    fn method(&mut self, recv: &V, name: &str, a: &[V]) -> (V, Vec<(usize, V)>) {
        let i = |k: usize| a.get(k).map_or(0, |v| v.int() as i32);
        match recv {
            V::Obj(GC) => {
                match name {
                    "setfont" => self.gc_font = a.first().map(font_name).unwrap_or_default(),
                    "setstyle" => self.gc_style = a.first().map_or(1, |v| v.int()),
                    "settextcolor" => {
                        if let Some(c) = a.first().and_then(|v| self.color_of(v)) {
                            self.gc_color = c;
                        }
                    }
                    "gettextextent" => {
                        let t = a.get(3).map(|v| v.str()).unwrap_or_default();
                        let name = self.gc_font.clone();
                        let (w, h) = self.font(&name).map_or((t.len() as i32 * 6, 10), |f| (f.width(&t), f.height));
                        return (V::None, vec![(1, V::Float(w as f64)), (2, V::Float(h as f64))]);
                    }
                    "drawtexture" | "drawpattern" => {
                        let t = a.get(6).map(|v| v.str()).unwrap_or_default();
                        if !t.is_empty() {
                            self.draws.push(Item {
                                r: (i(0), i(1), i(2), i(3)),
                                kind: Kind::Tex { name: t, uv: (i(4), i(5)), region: None, masked: style_of(self.gc_style) },
                                link: None,
                            });
                        }
                    }
                    "drawstretchedtexture" => {
                        let t = a.get(8).map(|v| v.str()).unwrap_or_default();
                        if !t.is_empty() {
                            self.draws.push(Item {
                                r: (i(0), i(1), i(2), i(3)),
                                kind: Kind::Tex { name: t, uv: (i(4), i(5)), region: Some((i(6).max(1), i(7).max(1))), masked: style_of(self.gc_style) },
                                link: None,
                            });
                        }
                    }
                    "drawtext" => {
                        let t = a.get(4).map(|v| v.str()).unwrap_or_default();
                        self.draws.push(Item {
                            r: (i(0), i(1), i(2).max(1), i(3).max(1)),
                            kind: Kind::Text { font: self.gc_font.clone(), lines: vec![t], color: self.gc_color, center: false },
                            link: None,
                        });
                    }
                    _ => {}
                }
                (V::None, Vec::new())
            }
            V::Obj(FLAGS) => match name {
                "getbool" => (V::Bool((self.flags)(&a.first().map(|v| v.str()).unwrap_or_default())), Vec::new()),
                _ => (V::None, Vec::new()),
            },
            V::Obj(PLAYER) => match name {
                "getlevelinfo" => (V::Obj(INFO), Vec::new()),
                "getrootwindow" | "gettopwindow" => (V::Obj(PLAYER), Vec::new()),
                _ => (V::None, Vec::new()),
            },
            V::Obj(id) if *id >= FIRST || *id == SELF => (self.win_method(*id, name, a).unwrap_or(V::None), Vec::new()),
            _ => (V::None, Vec::new()),
        }
    }

    fn field(&mut self, recv: &V, name: &str) -> V {
        match recv {
            V::Obj(PARENT) => match name {
                "width" => V::Int(PAGE_W as i64),
                "height" => V::Int(VIEW.3 as i64),
                _ => V::None,
            },
            V::Obj(PLAYER) => match name {
                "flagbase" => V::Obj(FLAGS),
                "trueplayername" => V::Str(crate::travel::player_name()),
                _ => V::None,
            },
            V::Obj(INFO) => match name {
                "missionnumber" => V::Int(self.mission as i64),
                "missionlocation" => V::Str(self.location.clone()),
                _ => V::None,
            },
            V::Obj(id) if *id >= FIRST || *id == SELF => {
                let Some(w) = self.wins.get(*id as usize) else { return V::None };
                match name {
                    "x" => V::Int(w.x as i64),
                    "y" => V::Int(w.y as i64),
                    "width" => V::Int(w.w as i64),
                    "height" => V::Int(w.h as i64),
                    "hyperlinkcolor" => V::Str("#link".into()),
                    "hyperlinkhovercolor" => V::Str("#hover".into()),
                    "linkdestination" => w.link.clone().map_or(V::None, V::Str),
                    "clipwindow" => recv.clone(),
                    _ => V::None,
                }
            }
            _ => V::None,
        }
    }
}

fn rgb(d: &[unreal_pkg::props::Property], name: &str, dflt: Rgb) -> Rgb {
    match find(d, name) {
        Some(Value::Struct(_, b)) if b.len() >= 3 => [b[0], b[1], b[2]],
        _ => dflt,
    }
}

/// Run a page class's InitWindow and DrawWindow against the recording windows: the page as a display list.
pub fn load_page(assets: &mut RuntimeAssets, class: &str, flags: &dyn Fn(&str) -> bool, mission: i32, location: &str) -> Option<Page> {
    // The class and its parents down to htmlPageWindow, base first.
    let mut chain: Vec<String> = assets.classes.ancestry(class);
    let stop = chain.iter().position(|c| c.eq_ignore_ascii_case("htmlPageWindow"))?;
    chain.truncate(stop + 1);
    chain.reverse();
    let mut script = crate::uscript::Script::default();
    script.objects = true;
    let mut funcs = HashSet::new();
    for c in &chain {
        let src = assets.classes.script_text(c)?;
        script.add_class(&src);
    }
    for n in script.names() {
        funcs.insert(n);
    }
    let link_d = assets.classes.defaults("TNMhyperlinkButton");
    let d = assets.classes.defaults(class);
    let url = match find(&d, "fakeURL") {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    };
    let mut wins = vec![Win::default(); FIRST as usize];
    wins[0] = Win { class: class.to_ascii_lowercase(), w: PAGE_W, h: 800, font: "FontMenuSmall".into(), ..Default::default() };
    let mut b = Builder {
        assets,
        funcs,
        wins,
        members: HashMap::new(),
        draws: Vec::new(),
        gc_font: "FontMenuSmall".into(),
        gc_color: [255, 255, 255],
        gc_style: 1,
        fonts: HashMap::new(),
        text_pkg: None,
        flags,
        mission,
        location: location.to_string(),
        link_col: rgb(&link_d, "hyperlinkColor", [0, 128, 255]),
        hover_col: rgb(&link_d, "hyperlinkHoverColor", [255, 255, 255]),
    };
    script.call(&mut b, "initwindow", &[]);
    script.call(&mut b, "drawwindow", &[V::Obj(GC)]);
    let mut items = std::mem::take(&mut b.draws);
    let text_col = crate::menuui::m_col(8);
    let wins = b.wins.clone();
    for w in wins.iter().skip(FIRST as usize) {
        let mut w = w.clone();
        // A button with no size of its own is as big as its text (ButtonWindow's preferred size).
        if w.w <= 0 && w.h <= 0 && !w.text.is_empty() {
            if let Some(f) = b.font(&w.font) {
                (w.w, w.h) = (f.width(&w.text) + 8, f.height + 4);
            }
        }
        if w.hidden || (w.w <= 0 && w.h <= 0) {
            continue;
        }
        let w = &w;
        let r = (w.x, w.y, w.w, w.h);
        if let Some(t) = &w.tex {
            items.push(Item { r, kind: Kind::Tex { name: t.clone(), uv: (0, 0), region: None, masked: true }, link: w.link.clone() });
        }
        if !w.text.is_empty() {
            let Some(f) = b.font(&w.font) else { continue };
            let inner = (w.w - 2 * w.margins.0).max(8);
            let lines = if w.is_link { vec![w.text.clone()] } else { crate::uicanvas::wrap(&f, &w.text, inner) };
            let color = w.color.unwrap_or(if w.is_link && w.link.is_some() { b.link_col } else { text_col });
            let r = (w.x + w.margins.0, w.y + w.margins.1, inner, w.h);
            items.push(Item { r, kind: Kind::Text { font: w.font.clone(), lines, color, center: w.center }, link: w.link.clone() });
        } else if w.tex.is_none() && w.link.is_some() {
            items.push(Item { r, kind: Kind::Area, link: w.link.clone() });
        }
    }
    let h = b.wins[0].h.max(VIEW.3);
    let bg = b.wins[0].tex.clone();
    println!("TNM web page {class}: {} items, {h} high", items.len());
    Some(Page { class: class.to_string(), h, bg, items, url })
}

/// htmlWindow: the browser, its page and the way back and forward.
pub struct Browser {
    page: Page,
    previous: Option<String>,
    next: Option<String>,
    scroll: i32,
    fonts: HashMap<String, Option<UiFont>>,
    tex: HashMap<String, Option<String>>,
    hover: Option<String>,
    title: String,
    pub client: (i32, i32),
    /// A page asked for (a link, back or forward): loaded on the next tick.
    want: Option<(String, u8)>,
    drag: bool,
}

/// The page the terminal being opened starts on (its defaultWebPage; None = by where it stands).
static HOME: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub fn set_home(page: Option<String>) {
    if let Ok(mut h) = HOME.lock() {
        *h = page;
    }
}

/// CreateWebPage: the page a terminal opens on.
fn home(assets: &mut RuntimeAssets, mission: i32, location: &str) -> &'static str {
    let _ = assets;
    if mission == 24 {
        "GameSpyhtmlPageWindow"
    } else if mission == 23 {
        "ABIhtmlPageWindowSkins"
    } else if location.contains("_WC") {
        "WChtmlPageWindowNews"
    } else {
        "PDXhtmlPageWindowNews"
    }
}

fn game_flag(name: &str) -> bool {
    crate::tnmmode::flag_true(name)
}

fn cached_font(fonts: &mut HashMap<String, Option<UiFont>>, assets: &mut RuntimeAssets, name: &str) -> Option<UiFont> {
    if !fonts.contains_key(name) {
        let f = crate::classic_hud::load_font(assets, name);
        fonts.insert(name.to_string(), f);
    }
    fonts.get(name).cloned().flatten()
}

impl Browser {
    pub fn new(assets: &mut RuntimeAssets, start: Option<&str>) -> Browser {
        let (mission, location) = crate::tnmmode::level_place();
        let own = HOME.lock().ok().and_then(|h| h.clone());
        let class = start.map(str::to_string).or(own).unwrap_or_else(|| home(assets, mission, &location).to_string());
        let d = assets.classes.defaults("htmlWindow");
        let int = |k: &str, dflt: i32| match find(&d, k) {
            Some(Value::Int(v)) => *v,
            _ => dflt,
        };
        let title = match find(&d, "Title") {
            Some(Value::Str(s)) => s.clone(),
            _ => String::new(),
        };
        let page = load_page(assets, &class, &game_flag, mission, &location).unwrap_or_default();
        Browser {
            page,
            previous: None,
            next: None,
            scroll: 0,
            fonts: HashMap::new(),
            tex: HashMap::new(),
            hover: None,
            title,
            client: (int("ClientWidth", VIEW.0 * 2 + VIEW.2).max(VIEW.0 + VIEW.2 + 4), int("ClientHeight", VIEW.1 + VIEW.3 + 8).max(VIEW.1 + VIEW.3 + 4)),
            want: None,
            drag: false,
        }
    }

    pub fn title_suffix(&self) -> String {
        if self.page.url.is_empty() {
            String::new()
        } else {
            format!(" - Narcissus: {}", self.page.url)
        }
    }

    pub fn window_title(&self) -> String {
        self.title.clone()
    }

    fn max_scroll(&self) -> i32 {
        (self.page.h - VIEW.3).max(0)
    }

    fn go(&mut self, assets: &mut RuntimeAssets, class: &str, how: u8) {
        let (mission, location) = crate::tnmmode::level_place();
        let Some(page) = load_page(assets, class, &game_flag, mission, &location) else {
            println!("TNM web: no page {class}");
            return;
        };
        let old = std::mem::replace(&mut self.page, page).class;
        match how {
            // A link: the page left is the way back, and there is no way forward.
            0 => {
                self.previous = Some(old);
                self.next = None;
            }
            // Back: the page left is the way forward.
            1 => {
                self.next = Some(old);
                self.previous = None;
            }
            // Forward.
            _ => {
                self.previous = Some(old);
                self.next = None;
            }
        }
        self.scroll = 0;
    }

    pub fn tick(&mut self, assets: &mut RuntimeAssets, keys: &ButtonInput<KeyCode>, ptr: crate::tnmpc::Pointer) {
        if let Some((class, how)) = self.want.take() {
            self.go(assets, &class, how);
        }
        let step = |k: KeyCode, d: i32| if keys.just_pressed(k) { d } else { 0 };
        let mut s = self.scroll
            + step(KeyCode::ArrowDown, 32)
            + step(KeyCode::ArrowUp, -32)
            + step(KeyCode::PageDown, VIEW.3 - 40)
            + step(KeyCode::PageUp, -(VIEW.3 - 40))
            - (ptr.wheel * 48.0) as i32;
        if keys.just_pressed(KeyCode::Home) {
            s = 0;
        }
        if keys.just_pressed(KeyCode::End) {
            s = self.max_scroll();
        }
        self.hover = None;
        if !ptr.down {
            self.drag = false;
        }
        if let Some((mx, my)) = ptr.pos {
            let bar_x = VIEW.0 + PAGE_W;
            if (ptr.pressed && mx >= bar_x && mx < VIEW.0 + VIEW.2 && my >= VIEW.1 && my < VIEW.1 + VIEW.3) || self.drag {
                // The scroll bar: the thumb goes to the pointer.
                self.drag = ptr.down;
                s = ((my - VIEW.1) as f32 / VIEW.3 as f32 * self.max_scroll() as f32) as i32;
            } else if mx >= VIEW.0 && mx < bar_x && my >= VIEW.1 && my < VIEW.1 + VIEW.3 {
                let (px, py) = (mx - VIEW.0, my - VIEW.1 + self.scroll);
                let hit = self.page.items.iter().rev().find(|it| it.link.is_some() && px >= it.r.0 && px < it.r.0 + it.r.2.max(8) && py >= it.r.1 && py < it.r.1 + it.r.3.max(8));
                self.hover = hit.and_then(|it| it.link.clone());
                if ptr.pressed {
                    if let Some(l) = self.hover.clone() {
                        crate::sound::play_player("DeusExSounds.UserInterface.Menu_Activate", 0.25, 0.0);
                        self.want = Some((l, 0));
                    }
                }
            } else if ptr.pressed && my < VIEW.1 {
                // Back and Forward, on the bar over the page.
                if mx >= VIEW.0 && mx < VIEW.0 + 60 {
                    self.back();
                } else if mx >= VIEW.0 + 64 && mx < VIEW.0 + 140 {
                    self.forward();
                }
            }
        }
        if keys.just_pressed(KeyCode::Backspace) {
            self.back();
        }
        self.scroll = s.clamp(0, self.max_scroll());
    }

    fn back(&mut self) {
        if let Some(p) = self.previous.clone() {
            crate::sound::play_player("DeusExSounds.UserInterface.Menu_Slider", 0.25, 0.0);
            self.want = Some((p, 1));
        }
    }

    fn forward(&mut self) {
        if let Some(p) = self.next.clone() {
            crate::sound::play_player("DeusExSounds.UserInterface.Menu_Slider", 0.25, 0.0);
            self.want = Some((p, 2));
        }
    }

    /// A texture as the script names it (often bare: `Texture'PDX_Icon'`): where it lives.
    fn texture<W: KitWid>(&mut self, u: &mut Ui<W>, name: &str) -> Option<std::sync::Arc<crate::uicanvas::UiTex>> {
        if !self.tex.contains_key(name) {
            let found = if name.contains('.') {
                u.hud.tex(u.assets, name).map(|_| name.to_string())
            } else {
                ["TNMGUI", "TNM", "DeusExUI.UserInterface", "TNMItems", "DeusExUI"].iter().map(|p| format!("{p}.{name}")).find(|p| u.hud.tex(u.assets, p).is_some())
            };
            self.tex.insert(name.to_string(), found);
        }
        let path = self.tex.get(name)?.clone()?;
        u.hud.tex(u.assets, &path)
    }

    pub fn draw<W: KitWid>(&mut self, u: &mut Ui<W>, x: i32, y: i32) {
        let white = [255, 255, 255];
        let (vx, vy) = (x + VIEW.0, y + VIEW.1);
        // The bar over the page: Back, Forward.
        if let Some(f) = cached_font(&mut self.fonts, u.assets, "FontMenuSmall") {
            let dim = crate::menuui::m_col(11);
            let lit = crate::menuui::m_col(8);
            u.text(&f, x + VIEW.0, y + 4, 60, f.height, HAlign::Left, VAlign::Top, if self.previous.is_some() { lit } else { dim }, "< Back");
            u.text(&f, x + VIEW.0 + 64, y + 4, 76, f.height, HAlign::Left, VAlign::Top, if self.next.is_some() { lit } else { dim }, "Forward >");
        }
        u.c.set_clip(Some((vx, vy, PAGE_W, VIEW.3)));
        u.c.fill(vx, vy, PAGE_W, VIEW.3, Style::Normal, [0, 0, 0]);
        if let Some(bg) = self.page.bg.clone() {
            if let Some(t) = self.texture(u, &bg) {
                u.c.texture(vx, vy, PAGE_W, VIEW.3, 0, self.scroll, &t, Style::Normal, white);
            }
        }
        let items = self.page.items.clone();
        for it in &items {
            let (ix, iy) = (vx + it.r.0, vy + it.r.1 - self.scroll);
            match &it.kind {
                Kind::Tex { name, uv, region, masked } => {
                    if iy > vy + VIEW.3 || iy + it.r.3 < vy {
                        continue;
                    }
                    let Some(t) = self.texture(u, name) else { continue };
                    let style = if *masked { Style::Masked } else { Style::Normal };
                    let (w, h) = (if it.r.2 > 0 { it.r.2 } else { t.w as i32 }, if it.r.3 > 0 { it.r.3 } else { t.h as i32 });
                    match region {
                        Some((uw, vh)) => u.c.texture_stretched(ix, iy, w, h, (uv.0, uv.1, *uw, *vh), &t, style, white),
                        None => u.c.texture(ix, iy, w, h, uv.0, uv.1, &t, style, white),
                    }
                }
                Kind::Text { font, lines, color, center } => {
                    let Some(f) = cached_font(&mut self.fonts, u.assets, font) else { continue };
                    let lh = f.height;
                    let hot = it.link.is_some() && it.link == self.hover;
                    let col = if hot { white } else { *color };
                    let first = ((vy - iy) / lh.max(1)).max(0) as usize;
                    for (k, l) in lines.iter().enumerate().skip(first) {
                        let ly = iy + k as i32 * lh;
                        if ly > vy + VIEW.3 {
                            break;
                        }
                        u.c.text(&f, ix, ly, it.r.2.max(f.width(l)), lh, if *center { HAlign::Center } else { HAlign::Left }, VAlign::Top, col, l);
                    }
                }
                Kind::Area => {}
            }
        }
        u.c.set_clip(None);
        // The scroll bar.
        let (bx, bw) = (vx + PAGE_W, VIEW.2 - PAGE_W);
        u.c.fill(bx, vy, bw, VIEW.3, Style::Normal, [24, 24, 24]);
        let max = self.max_scroll();
        if max > 0 {
            let th = (VIEW.3 * VIEW.3 / self.page.h.max(1)).clamp(16, VIEW.3);
            let ty = vy + ((VIEW.3 - th) as i64 * self.scroll as i64 / max as i64) as i32;
            u.c.fill(bx + 2, ty, bw - 4, th, Style::Normal, crate::menuui::m_col(3));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With the game and the mod at hand (TNM_ROOT=<Deus Ex folder>, TNM_DIR=<the mod's folder>): every page class
    /// loads to something.
    /// WEB_DUMP=<class> prints one page's display list.
    #[test]
    fn pages_from_the_mod() {
        let Ok(root) = std::env::var("TNM_ROOT") else { return };
        let root = std::path::PathBuf::from(root);
        let tnm = std::env::var("TNM_DIR").map(std::path::PathBuf::from).ok().or_else(|| unreal_pkg::tnm::find(&root)).expect("the mod");
        unreal_pkg::paths::set(&root, unreal_pkg::tnm::overlays(&tnm));
        unreal_pkg::classes::set_extra_packages(unreal_pkg::tnm::PACKAGES);
        unreal_pkg::tnm::set_active(Some(tnm));
        let mut assets = RuntimeAssets {
            classes: unreal_pkg::classes::ClassDb::open(&root.join("System")),
            lib: dx_level::TexLib::new(&root),
            images: Default::default(),
            materials: Default::default(),
            fire: Vec::new(),
            sprites: Default::default(),
            shared_parts: Default::default(),
        };
        let none = |_: &str| false;
        let dump = std::env::var("WEB_DUMP").ok();
        let gui = unreal_pkg::Package::open(root.join("System").join("TNMGUI.u")).expect("TNMGUI.u");
        assert!(crate::classic_hud::load_font(&mut assets, "FontMenuSmall").is_some(), "the game's fonts");
        let all: Vec<String> = gui.exports.iter().filter(|e| e.class_index == 0).map(|e| e.name.clone()).collect();
        let mut names: Vec<String> = all
            .into_iter()
            .filter(|n| !n.eq_ignore_ascii_case("htmlPageWindow") && assets.classes.ancestry(n).iter().any(|a| a == "htmlpagewindow"))
            .collect();
        names.sort();
        assert!(names.len() > 100, "{} page classes", names.len());
        let (mut ok, mut empty, mut links, mut texts) = (0, Vec::new(), 0, 0);
        for n in &names {
            let Some(p) = load_page(&mut assets, n, &none, 20, "") else {
                empty.push(n.clone());
                continue;
            };
            if p.items.is_empty() {
                empty.push(n.clone());
                continue;
            }
            ok += 1;
            links += p.items.iter().filter(|i| i.link.is_some()).count();
            texts += p.items.iter().filter(|i| matches!(&i.kind, Kind::Text { lines, .. } if lines.len() > 3)).count();
            if dump.as_deref().is_some_and(|d| d.eq_ignore_ascii_case(n)) {
                println!("PAGE {n}: h {} bg {:?} url {}", p.h, p.bg, p.url);
                for i in &p.items {
                    let what = match &i.kind {
                        Kind::Tex { name, region, .. } => format!("tex {name} {region:?}"),
                        Kind::Text { font, lines, .. } => format!("text {font} {} lines: {:?}", lines.len(), lines.first().map(|l| l.chars().take(50).collect::<String>())),
                        Kind::Area => "area".into(),
                    };
                    println!("  {:?} {what} -> {:?}", i.r, i.link);
                }
            }
        }
        println!("{ok} of {} pages with content; {links} links, {texts} text bodies; empty: {empty:?}", names.len());
        assert!(ok * 10 >= names.len() * 9);
        assert!(links > 2000 && texts > 150);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// whoisWindow: the user database.

/// whoisWindow: every forum user with where they are and whether they're about, banned or dead - the people of the
/// story by their flags, the crowd at random.
// uc: whoisWindow.populateUserList, whoisWindow.BuildRowString, whoisWindow.GetUserStatus, whoisWindow.GetUserLocation
pub struct Whois {
    rows: Vec<[String; 3]>,
    heads: [String; 3],
    sort: (usize, bool),
    scroll: i32,
    font: Option<UiFont>,
    pub client: (i32, i32),
}

impl Whois {
    pub fn new(assets: &mut RuntimeAssets, seed: u64) -> Whois {
        let d = assets.classes.defaults("whoisWindow");
        let s = |k: &str| match find(&d, k) {
            Some(Value::Str(v)) => v.clone(),
            _ => String::new(),
        };
        let arr = |k: &str| -> Vec<String> {
            let mut v: Vec<(u32, String)> = d
                .iter()
                .filter(|p| p.name.eq_ignore_ascii_case(k))
                .filter_map(|p| match &p.value {
                    Value::Str(t) => Some((p.index, t.clone())),
                    _ => None,
                })
                .collect();
            v.sort();
            let n = v.last().map_or(0, |x| x.0 as usize + 1);
            let mut out = vec![String::new(); n];
            for (i, t) in v {
                out[i as usize] = t;
            }
            out
        };
        let int = |k: &str, dflt: i32| match find(&d, k) {
            Some(Value::Int(v)) => *v,
            _ => dflt,
        };
        let status = arr("strStatus");
        let locs = [arr("strLocationPDX"), arr("strLocationPWC"), arr("strLocationPCNC")];
        let dir = assets.lib.system_dir().to_path_buf();
        let pkg = unreal_pkg::Package::open(dir.join("TNMText.u")).ok();
        let mut seed = seed | 1;
        let mut frand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 40) as f32 / (1u64 << 24) as f32
        };
        let mut rows = Vec::new();
        let mut i = 0usize;
        for (text, realm, check) in [("TNMUsers", 0usize, true), ("TNMUsersGS", 0, true), ("TNMUsersPDX", 0, false), ("TNMUsersPWC", 1, false), ("TNMUsersPCNC", 2, false)] {
            let raw = pkg.as_ref().and_then(|p| p.text_named(text)).unwrap_or_default();
            for line in raw.split('\n') {
                let mut line = line.trim_matches(['\r', '\0', ' ']).to_string();
                while let (Some(a), Some(b)) = (line.find('<'), line.find('>')) {
                    if b < a {
                        break;
                    }
                    line.replace_range(a..=b, "");
                }
                if line.is_empty() {
                    continue;
                }
                let (name, fixed) = match line.split_once(';') {
                    Some((n, l)) => (n.to_string(), l.trim().parse::<usize>().ok()),
                    None => (line.clone(), None),
                };
                let list = &locs[realm];
                let loc_index = fixed.unwrap_or(match realm {
                    0 => i % 5,
                    _ => ((frand() * list.len() as f32) as usize).min(list.len().saturating_sub(1)),
                });
                let st = if check {
                    if crate::tnmmode::flag_true(&format!("{name}_Banned")) {
                        3
                    } else if crate::tnmmode::flag_true(&format!("{name}_Dead")) {
                        2
                    } else if frand() < 0.1 {
                        1
                    } else {
                        0
                    }
                } else if frand() < 0.05 {
                    3
                } else if frand() < 0.1 {
                    2
                } else if frand() < 0.1 {
                    1
                } else {
                    0
                };
                let loc = if st >= 2 { "Banspace".to_string() } else { list.get(loc_index).cloned().unwrap_or_default() };
                i += 1;
                if loc.is_empty() {
                    continue;
                }
                rows.push([name, loc, status.get(st).cloned().unwrap_or_default()]);
            }
        }
        rows.sort_by(|a, b| a[0].to_lowercase().cmp(&b[0].to_lowercase()));
        Whois {
            rows,
            heads: [s("HeaderUserLabel"), s("HeaderLocationLabel"), s("HeaderValueLabel")],
            sort: (0, false),
            scroll: 0,
            font: crate::classic_hud::load_font(assets, "FontMenuHeaders"),
            client: (int("ClientWidth", 600), int("ClientHeight", 376)),
        }
    }

    const COLS: [(i32, i32); 3] = [(7, 125), (132, 386), (518, 75)];

    fn row_h(&self) -> i32 {
        self.font.as_ref().map_or(12, |f| f.height + 1)
    }

    pub fn tick(&mut self, keys: &ButtonInput<KeyCode>, ptr: crate::tnmpc::Pointer) {
        let view = self.client.1 - 35;
        let max = (self.rows.len() as i32 * self.row_h() - view).max(0);
        let step = |k: KeyCode, d: i32| if keys.just_pressed(k) { d } else { 0 };
        self.scroll += step(KeyCode::ArrowDown, self.row_h() * 3) + step(KeyCode::ArrowUp, -self.row_h() * 3) + step(KeyCode::PageDown, view - 20) + step(KeyCode::PageUp, -(view - 20))
            - (ptr.wheel * 3.0 * self.row_h() as f32) as i32;
        if keys.just_pressed(KeyCode::Home) {
            self.scroll = 0;
        }
        if keys.just_pressed(KeyCode::End) {
            self.scroll = max;
        }
        self.scroll = self.scroll.clamp(0, max);
        if let (true, Some((mx, my))) = (ptr.pressed, ptr.pos) {
            if (7..27).contains(&my) {
                if let Some(c) = Self::COLS.iter().position(|(x, w)| mx >= *x && mx < x + w) {
                    crate::sound::play_player("DeusExSounds.UserInterface.Menu_Press", 0.25, 0.0);
                    let down = if self.sort.0 == c { !self.sort.1 } else { false };
                    self.sort = (c, down);
                    self.rows.sort_by(|a, b| {
                        let o = a[c].to_lowercase().cmp(&b[c].to_lowercase());
                        if down { o.reverse() } else { o }
                    });
                }
            }
        }
    }

    pub fn draw<W: KitWid>(&mut self, u: &mut Ui<W>, x: i32, y: i32) {
        let Some(f) = self.font.clone() else { return };
        let (head, text) = (crate::menuui::m_col(4), crate::menuui::m_col(8));
        for (k, (cx, cw)) in Self::COLS.iter().enumerate() {
            u.c.fill(x + cx, y + 7, cw - 2, 20, Style::Normal, crate::menuui::m_col(10));
            u.text(&f, x + cx + 4, y + 7, cw - 6, 20, HAlign::Left, VAlign::Center, head, &self.heads[k]);
        }
        let (vy, vh) = (y + 30, self.client.1 - 35);
        u.c.set_clip(Some((x + 7, vy, self.client.0 - 14, vh)));
        let rh = self.row_h();
        let first = (self.scroll / rh).max(0) as usize;
        for (r, row) in self.rows.iter().enumerate().skip(first) {
            let ry = vy + r as i32 * rh - self.scroll;
            if ry > vy + vh {
                break;
            }
            for (k, (cx, cw)) in Self::COLS.iter().enumerate() {
                u.c.text(&f, x + cx + 4, ry, cw - 6, rh, HAlign::Left, VAlign::Center, text, &row[k]);
            }
        }
        u.c.set_clip(None);
    }
}
