//! The Nameless Mod's text consoles on its terminals (TextConsole + its TextConsoleInterface subclasses):
//! the GSI command line with its little file system (dxigWindow), the two dispensing consoles (dxegWindow: the
//! editing archive that summons one item or bot; abigWindow: the ABI assembly line that makes one weapon), and the
//! two chat clients (tnmFakeIRCWindow: the scripted Narcissus chat; tnmIRCWindow: a real IRC client).
//!
//! Every word shown comes from the player's own TNMGUI.u / TNMText.u at run time: class defaults, the TNMText
//! notes, and - for the scripted chat, which exists only as code - tnmFakeIRCWindow.IRCUpdate run through the
//! uscript data loader (one call per timer tick, recording what it says).
// uc: TextConsole.InitWindow, TextConsole.EditActivated, TextConsoleOutput.DrawWindow, ConsoleInputWindow.VirtualKeyPressed
// uc: TextConsoleInterface.ParseRaw, TextConsoleInterface.Parse, TextConsoleIRC.InitWindow

use bevy::input::ButtonInput;
use bevy::prelude::KeyCode;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use unreal_pkg::props::Value;

use crate::menuui::{KitWid, Ui};
use crate::tnmpc::{fmtn, font, num, path, words, Pointer, Rng, Words};
use crate::uicanvas::{HAlign, Style, UiFont, VAlign};
use crate::weapons::RuntimeAssets;

// ------------------------------------------------------------------------------------------------ effects

/// What a console asks of the world; computers.rs applies these with the level, flags and events at hand.
#[derive(Clone, Debug)]
pub enum Effect {
    Flag(String, bool),
    /// A Name flag (Trestkon.ABISpawnItem rides one: the ABI script reads it).
    NameFlag(String, String),
    /// Trigger every actor with this Tag.
    Event(String),
    /// Spawn this class where the first actor with the Tag stands (dxeg: pawns stand, weapons come legendary).
    Summon { class: String, at_tag: String, legendary: bool },
    Achievement(String),
    /// Player.Credits changes by this much (the ABI shop).
    Credits(i64),
    /// What Trestkon wears now (the ABI shop: RefreshPlayerModel).
    Look(crate::tnmlook::Look),
}

static FX: Mutex<Vec<Effect>> = Mutex::new(Vec::new());

fn fx(e: Effect) {
    if let Ok(mut q) = FX.lock() {
        q.push(e);
    }
}

pub(crate) fn push_effect(e: Effect) {
    fx(e);
}

static CREDITS: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

/// The player's credits as the terminal saw them this frame.
pub fn credits_now() -> i64 {
    CREDITS.load(std::sync::atomic::Ordering::Relaxed)
}
pub fn note_credits(n: i64) {
    CREDITS.store(n, std::sync::atomic::Ordering::Relaxed);
}

pub fn take_effects() -> Vec<Effect> {
    FX.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}

/// What a console reads from the game when it starts and while it runs.
pub struct Ctx<'a> {
    pub user: String,
    pub flags: &'a imsim_core::flags::FlagBase,
    /// Trestkon.GetLevelName (the map's name) and the terminal actor's own name (the real chat's join line).
    pub level: String,
    pub terminal: String,
}

impl Ctx<'_> {
    fn b(&self, n: &str) -> bool {
        matches!(self.flags.get(n), Some(imsim_core::flags::FlagValue::Bool(true)))
    }
}

// ------------------------------------------------------------------------------------------------ the console

fn defaults(assets: &mut RuntimeAssets, class: &str) -> Words {
    Words { d: assets.classes.defaults(class).to_vec() }
}

/// Two fstrings of a struct (sValidClasses: NeatName, className).
fn struct_strings(b: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        // compact index
        let b0 = b[i];
        let mut n = (b0 & 0x3F) as i64;
        let neg = b0 & 0x80 != 0;
        i += 1;
        if b0 & 0x40 != 0 {
            let mut shift = 6;
            while i < b.len() {
                let c = b[i];
                i += 1;
                n |= ((c & 0x7F) as i64) << shift;
                shift += 7;
                if c & 0x80 == 0 {
                    break;
                }
            }
        }
        let n = if neg { -n } else { n };
        if n > 0 {
            let len = n as usize;
            let end = (i + len).min(b.len());
            let s: String = b[i..end].iter().take_while(|c| **c != 0).map(|c| *c as char).collect();
            out.push(s);
            i = end;
        } else if n < 0 {
            let len = (-n) as usize * 2;
            let end = (i + len).min(b.len());
            let s = String::from_utf16_lossy(&b[i..end].chunks(2).map(|c| u16::from_le_bytes([c[0], *c.get(1).unwrap_or(&0)])).take_while(|c| *c != 0).collect::<Vec<_>>());
            out.push(s);
            i = end;
        } else {
            out.push(String::new());
        }
    }
    out
}

/// Seconds since 1970 as day / month / clock (the console timestamps; tnmTime reads the system clock).
fn civil(secs: u64) -> (i64, u32, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, m, s) = ((rem / 3600) as u32, ((rem % 3600) / 60) as u32, (rem % 60) as u32);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if mo <= 2 { y + 1 } else { y };
    let wd = ((days + 4).rem_euclid(7)) as u32; // 0 = Sunday
    (y, mo, d, h, m, s, wd)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// "Wed Nov 23 14:05:09" (the chat's topic times).
fn stamp_long(secs: u64) -> String {
    let (_, mo, d, h, m, s, wd) = civil(secs);
    format!("{} {} {:02} {:02}:{:02}:{:02}", DAYS[wd as usize], MONTHS[(mo - 1) as usize], d, h, m, s)
}

/// A console: the window's pictures, the output above (newest at the bottom, word wrapped), the command line.
pub struct Console {
    lines: Vec<String>,
    input: String,
    history: Vec<String>,
    hist_at: Option<usize>,
    font: Option<Arc<UiFont>>,
    bg: Vec<String>,
    bg_cols: usize,
    pub client: (i32, i32),
    class: &'static str,
    blink: f32,
    /// ConsoleInputWindow.SetSensitivity (off while the thread delete runs / once the threads are closed).
    sensitive: bool,
    /// TextConsoleIRC's user list: "prefix name" and its priority (op 3, half-op 2, voice 1).
    users: Option<Vec<(String, u8)>>,
    scroll: usize,
    prog: Prog,
    /// Hides Return / Logout (dxigInfo.Finishing).
    pub busy: bool,
}

enum Prog {
    Dxig(Box<Dxig>),
    Dispense(Box<Dispenser>),
    Fake(Box<FakeIrc>),
    Real(Box<RealIrc>),
}

/// The consoles a tnmComputerScreenGames action opens.
pub fn is_console(action: &str) -> bool {
    matches!(action, "DXIGAME" | "DXEGAME" | "ABIGAME" | "FAKEIRC" | "REALIRC")
}

impl Console {
    pub fn new(action: &str, assets: &mut RuntimeAssets, seed: u64, cx: &Ctx) -> Option<Console> {
        let class: &'static str = match action {
            "DXIGAME" => "dxigWindow",
            "DXEGAME" => "dxegWindow",
            "ABIGAME" => "abigWindow",
            "FAKEIRC" => "tnmFakeIRCWindow",
            "REALIRC" => "tnmIRCWindow",
            _ => return None,
        };
        let wd = assets.classes.defaults(class).to_vec();
        let cd = assets.classes.defaults("TextConsole").to_vec();
        let fnt = font(assets, &path(&cd, "consoleFont")).or_else(|| font(assets, "DeusExUI.FontMenuSmall"));
        let bg: Vec<String> = (0..16)
            .map_while(|i| match wd.iter().find(|p| p.name.eq_ignore_ascii_case("clientTextures") && p.index == i).map(|p| &p.value) {
                Some(Value::Path(p)) => Some(p.clone()),
                _ => None,
            })
            .collect();
        let irc = matches!(action, "FAKEIRC" | "REALIRC");
        let mut c = Console {
            lines: Vec::new(),
            input: String::new(),
            history: Vec::new(),
            hist_at: None,
            font: fnt,
            bg,
            bg_cols: num(&wd, "textureCols", 3.0) as usize,
            client: (num(&wd, "ClientWidth", 600.0) as i32, num(&wd, "ClientHeight", 400.0) as i32),
            class,
            blink: 0.0,
            sensitive: true,
            users: irc.then(Vec::new),
            scroll: 0,
            prog: Prog::Dispense(Box::new(Dispenser::empty())),
            busy: false,
        };
        let prog = match action {
            "DXIGAME" => Prog::Dxig(Box::new(Dxig::new(assets, &mut c, cx))),
            "DXEGAME" => Prog::Dispense(Box::new(Dispenser::new(assets, &mut c, Kind::Dxe))),
            "ABIGAME" => Prog::Dispense(Box::new(Dispenser::new(assets, &mut c, Kind::Abi))),
            "FAKEIRC" => Prog::Fake(Box::new(FakeIrc::new(assets, &mut c, cx, seed))),
            _ => Prog::Real(Box::new(RealIrc::new(assets, &mut c, cx))),
        };
        c.prog = prog;
        Some(c)
    }

    pub fn class(&self) -> &'static str {
        self.class
    }

    /// TextConsoleOutput.PushLn.
    fn say(&mut self, s: &str) {
        self.lines.push(s.to_string());
        if self.lines.len() > 4000 {
            self.lines.drain(..1000);
        }
        self.scroll = 0;
    }
    /// TextConsoleOutput.Push: onto the last line.
    fn push(&mut self, s: &str) {
        match self.lines.last_mut() {
            Some(l) => l.push_str(s),
            None => self.lines.push(s.to_string()),
        }
    }

    pub fn tick(&mut self, dt: f32, keys: &ButtonInput<KeyCode>, typed: &[char], ptr: Pointer, cx: &Ctx) {
        self.blink = (self.blink + dt) % 1.0;
        if ptr.wheel != 0.0 {
            let up = ptr.wheel > 0.0;
            let n = (ptr.wheel.abs() * 3.0).round().max(1.0) as usize;
            self.scroll = if up { (self.scroll + n).min(self.lines.len().saturating_sub(1)) } else { self.scroll.saturating_sub(n) };
        }
        if self.sensitive {
            for &ch in typed {
                if !ch.is_control() && self.input.chars().count() < 256 {
                    self.input.push(ch);
                }
            }
            if keys.just_pressed(KeyCode::Backspace) {
                self.input.pop();
            }
            // ConsoleInputWindow: Up / Down walk the history.
            if keys.just_pressed(KeyCode::ArrowUp) && !self.history.is_empty() {
                let i = match self.hist_at {
                    None => Some(self.history.len() - 1),
                    Some(0) => None,
                    Some(i) => Some(i - 1),
                };
                self.hist_at = i;
                if let Some(i) = i {
                    self.input = self.history[i].clone();
                }
            }
            if keys.just_pressed(KeyCode::ArrowDown) {
                if let Some(i) = self.hist_at {
                    self.hist_at = (i + 1 < self.history.len()).then_some(i + 1);
                    self.input = self.hist_at.map(|i| self.history[i].clone()).unwrap_or_default();
                }
            }
            if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter) {
                let line = std::mem::take(&mut self.input);
                if !line.is_empty() {
                    self.history.push(line.clone());
                    self.hist_at = None;
                    self.parse_raw(&line, cx);
                }
            }
        }
        let mut prog = std::mem::replace(&mut self.prog, Prog::Dispense(Box::new(Dispenser::empty())));
        match &mut prog {
            Prog::Dxig(d) => d.tick(dt, self),
            Prog::Fake(f) => f.tick(dt, self),
            Prog::Real(r) => r.tick(self, cx),
            Prog::Dispense(_) => {}
        }
        self.prog = prog;
    }

    /// TextConsoleInterface.ParseRaw: the command prefix (the chat's "/"), then the command and its words.
    fn parse_raw(&mut self, raw: &str, cx: &Ctx) {
        let (prefix, default) = match &self.prog {
            Prog::Fake(f) => (f.w.s("command_prefix"), f.w.s("command_default")),
            Prog::Real(r) => (r.w.s("command_prefix"), r.w.s("command_default")),
            _ => (String::new(), String::new()),
        };
        let mut text = raw.to_string();
        if !prefix.is_empty() {
            if text.len() >= prefix.len() && text[..prefix.len()].eq_ignore_ascii_case(&prefix) {
                text = text[prefix.len()..].to_string();
            } else {
                text = format!("{default} {text}");
            }
        }
        let ws = words(&text);
        let cmd = ws.first().cloned().unwrap_or_default();
        let args: Vec<String> = ws.iter().skip(1).cloned().collect();
        // The rest after the command word and its spaces.
        let t = text.trim_start();
        let raw_args = match t.find(char::is_whitespace) {
            Some(i) => t[i..].trim_start().to_string(),
            None => String::new(),
        };
        let mut prog = std::mem::replace(&mut self.prog, Prog::Dispense(Box::new(Dispenser::empty())));
        let handled = match &mut prog {
            Prog::Dxig(d) => d.parse(self, &cmd, &args, raw, &raw_args),
            Prog::Dispense(d) => d.parse(self, &cmd, &raw_args, raw),
            Prog::Fake(f) => {
                let s = f.w.at("strError", 0);
                f.message(self, &s);
                true
            }
            Prog::Real(r) => r.parse(self, &cmd, &args, &raw_args, raw, cx),
        };
        let w = match &prog {
            Prog::Dxig(d) => &d.w,
            Prog::Dispense(d) => &d.w,
            Prog::Fake(f) => &f.w,
            Prog::Real(r) => &r.w,
        };
        let handled = handled || base_parse(self, w, &cmd, &args);
        if !handled {
            let s = fmtn(&w.at("strError", 0), &[&cmd]);
            self.say(&s);
        }
        self.prog = prog;
    }

    pub fn draw<W: KitWid>(&mut self, u: &mut Ui<W>, x: i32, y: i32) {
        // The client pictures, cols x rows.
        let (mut tx, mut ty, mut row_h) = (x, y, 0);
        for (k, b) in self.bg.clone().iter().enumerate() {
            if k > 0 && k % self.bg_cols.max(1) == 0 {
                tx = x;
                ty += row_h;
                row_h = 0;
            }
            if let Some(t) = u.hud.tex(u.assets, b) {
                u.c.texture(tx, ty, t.w as i32, t.h as i32, 0, 0, &t, Style::Normal, [255, 255, 255]);
                tx += t.w as i32;
                row_h = row_h.max(t.h as i32);
            }
        }
        let Some(f) = self.font.clone() else { return };
        // TextConsole: output 600-16 wide in the scroll area at (8, 7); the chat's is 100 narrower for the list.
        let irc = self.users.is_some();
        let lw = if irc { 600 - 16 - 80 - 15 - 5 } else { 600 - 16 };
        let (lx, top) = (x + 8, y + 7);
        let mut ly = y + 400 - 23 - 4;
        'up: for l in self.lines.iter().rev().skip(self.scroll) {
            let wrapped = crate::uicanvas::wrap(&f, l, lw);
            for wl in wrapped.iter().rev() {
                ly -= f.height;
                if ly < top {
                    break 'up;
                }
                u.c.text(&f, lx, ly, lw, f.height, HAlign::Left, VAlign::Top, [255, 255, 255], wl);
            }
        }
        // The user list (MenuUIListWindow, sorted by priority then name).
        if let Some(users) = &self.users {
            let (ux, uy) = (x + 600 - 80 - 16, y + 7);
            u.c.fill(ux, uy, 90, 400 - 23 - 11, Style::Translucent, [16, 16, 16]);
            let mut sorted = users.clone();
            sorted.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
            for (i, (n, _)) in sorted.iter().enumerate() {
                let yy = uy + 2 + i as i32 * f.height;
                if yy + f.height > uy + 400 - 23 - 11 {
                    break;
                }
                u.c.text(&f, ux + 3, yy, 86, f.height, HAlign::Left, VAlign::Top, [255, 255, 255], n);
            }
        }
        // The command line (an info button's well).
        let well = if self.sensitive { [24, 24, 24] } else { [8, 8, 8] };
        u.c.fill(x + 4, y + 400 - 23, 600 - 8, 21, Style::Translucent, well);
        if self.sensitive {
            let caret = if self.blink < 0.5 { "_" } else { "" };
            let shown = format!("{}{caret}", self.input);
            u.c.text(&f, x + 8, y + 400 - 23 + 4, 600 - 16, f.height, HAlign::Left, VAlign::Top, [255, 255, 255], &shown);
        }
    }

    /// The window closes (DestroyWindow): the dxig tree stays with the level; the chat quits.
    pub fn close(&mut self) {
        match &mut self.prog {
            Prog::Dxig(d) => d.store(),
            Prog::Real(r) => r.quit(),
            _ => {}
        }
    }
}

impl Drop for Console {
    fn drop(&mut self) {
        self.close();
    }
}

/// TextConsoleInterface.Parse: HELP (the command list, or one command's help line).
fn base_parse(c: &mut Console, w: &Words, cmd: &str, args: &[String]) -> bool {
    let help = w.s("strHelpCom");
    if help.is_empty() || !cmd.eq_ignore_ascii_case(&help) {
        return false;
    }
    let list = w.list("command_list");
    if let Some(a) = args.first() {
        if a.eq_ignore_ascii_case(&help) {
            c.say(&w.s("strHelpDef"));
            return true;
        }
        if let Some(i) = list.iter().position(|x| x.eq_ignore_ascii_case(a)) {
            let h = w.at("command_help", i as u32);
            if !h.is_empty() {
                c.say(&format!("{} - {h}", a.to_uppercase()));
                return true;
            }
        }
        c.say(&fmtn(&w.at("strError", 2), &[a]));
    } else {
        c.say(&w.s("strCommandList"));
        for x in list {
            c.say(&x.to_uppercase());
        }
    }
    true
}

// ------------------------------------------------------------------------------------------------ dxig

#[derive(Clone, Copy, PartialEq, Debug)]
enum Exe {
    Cd,
    Del,
    Dir,
    Read,
    Echo,
    Help,
    Threads { wc: bool },
}

#[derive(Clone, Debug)]
enum NodeKind {
    Folder { files: Vec<usize>, access: i32 },
    Txt(String),
    Exe(Exe),
    Sys,
}

#[derive(Clone, Debug)]
struct Node {
    name: String,
    parent: Option<usize>,
    kind: NodeKind,
    read_only: bool,
    in_use: bool,
}

/// The tree lives with the level (dxigInfo, one per map), so deleted files stay deleted until the next map.
#[derive(Clone)]
struct Tree {
    nodes: Vec<Node>,
    root: usize,
    cmds: usize,
}

static TREE: Mutex<Option<Tree>> = Mutex::new(None);

pub fn reset_level() {
    if let Ok(mut t) = TREE.lock() {
        *t = None;
    }
}

/// dxigWindow + dxigInterface: the GSI command line.
// uc: dxigInterface.Init, dxigInterface.Parse, dxigInterface.resolve_file, dxigInterface.get_file, dxigInterface.SetUser
// uc: dxigFile.Delete, dxigFolder.Delete, dxigFolder.PathString, dxigFolder.FindFile, dxigFolder.InUse, dxigFileSys.Delete
// uc: dxigExeCD.Execute, dxigExeDel.Execute, dxigExeDir.Execute, dxigExeRead.Execute, dxigExeEcho.Execute
// uc: dxigExeHelp.Execute, dxigExeThreads.Execute, dxigInfo.FinishGame, dxigTxtParsed.Setup
struct Dxig {
    w: Words,
    t: Tree,
    cur: usize,
    user: String,
    permissions: i32,
    /// Strings of each file class (dxigExeCD.strHelp, dxigFolder.strAccessDenied ...).
    cls: std::collections::HashMap<&'static str, Words>,
    /// dxigInfo's Finishing state: (wc, time to the next dot, dots left).
    finishing: Option<(bool, f32, i32)>,
}

impl Dxig {
    fn new(assets: &mut RuntimeAssets, c: &mut Console, cx: &Ctx) -> Dxig {
        let w = defaults(assets, "dxigInterface");
        let mut cls = std::collections::HashMap::new();
        for k in ["dxigFile", "dxigFolder", "dxigFileSys", "dxigExeCD", "dxigExeDel", "dxigExeDir", "dxigExeRead", "dxigExeEcho", "dxigExeHelp", "dxigExeThreads", "dxigInfo"] {
            cls.insert(k, defaults(assets, k));
        }
        // SetUser: the account logged in.
        let permissions = match cx.user.to_uppercase().as_str() {
            "FARGO" => 1,
            "SRVRADM" => 2,
            _ => 0,
        };
        c.say(&w.at("miscLines", 0));
        c.say(&w.at("miscLines", 1));
        let kept = TREE.lock().ok().and_then(|t| t.clone());
        let (t, fresh) = match kept {
            Some(t) => (t, false),
            None => (Self::build(assets, &w, cx), true),
        };
        let d = Dxig { w, cur: t.root, t, user: cx.user.clone(), permissions, cls, finishing: None };
        if fresh && cx.b("DXI_ThreadsClosed") {
            let s = d.w.at("miscLines", 2);
            c.say(&s);
            c.sensitive = false;
        } else {
            let p = format!("{}>", d.path_string(d.cur));
            c.say(&p);
        }
        d.store();
        d
    }

    fn build(assets: &mut RuntimeAssets, w: &Words, cx: &Ctx) -> Tree {
        let mut t = Tree { nodes: Vec::new(), root: 0, cmds: 0 };
        let add = |t: &mut Tree, name: String, kind: NodeKind, read_only: bool, parent: Option<usize>| -> usize {
            let i = t.nodes.len();
            t.nodes.push(Node { name, parent, kind, read_only, in_use: false });
            if let Some(p) = parent {
                if let NodeKind::Folder { files, .. } = &mut t.nodes[p].kind {
                    files.push(i);
                }
            }
            i
        };
        let folder = |access: i32| NodeKind::Folder { files: Vec::new(), access };
        let pkg = unreal_pkg::Package::open(assets.lib.system_dir().join("TNMText.u")).ok();
        // dxigTxtParsed: the named TNMText note, its text parts joined by "|n".
        let note = |name: &str| -> NodeKind {
            let raw = pkg.as_ref().and_then(|p| p.text_named(name)).unwrap_or_default();
            let parsed = crate::info::parse_tnm(&raw);
            NodeKind::Txt(parsed.paras.iter().map(|p| p.text.clone()).collect::<Vec<_>>().join("|n"))
        };
        let f = |i: u32| w.at("fileNames", i);
        let d = |i: u32| w.at("folderNames", i);
        let root = add(&mut t, "root".into(), folder(0), false, None);
        t.root = root;
        let a = add(&mut t, d(0), folder(0), false, Some(root));
        add(&mut t, f(0), note("dxig_TextFile11"), true, Some(a));
        add(&mut t, f(1), note("dxig_TextFile01"), true, Some(a));
        let doc = add(&mut t, d(1), folder(0), false, Some(root));
        let gw = add(&mut t, d(2), folder(0), false, Some(doc));
        for (i, n) in [(2, "07"), (3, "08"), (4, "10"), (5, "04"), (6, "05"), (7, "06")] {
            add(&mut t, f(i), note(&format!("dxig_TextFile{n}")), false, Some(gw));
        }
        let plan = add(&mut t, d(3), folder(0), false, Some(doc));
        add(&mut t, f(8), note("dxig_TextFile09"), false, Some(plan));
        let logs = add(&mut t, d(4), folder(1), false, Some(root));
        add(&mut t, f(9), note("dxig_TextFile03"), true, Some(logs));
        add(&mut t, f(10), note("dxig_TextFile02"), true, Some(logs));
        let cmds = add(&mut t, d(5), folder(2), true, Some(root));
        t.cmds = cmds;
        add(&mut t, f(11), NodeKind::Exe(Exe::Cd), true, Some(cmds));
        add(&mut t, f(12), NodeKind::Exe(Exe::Del), false, Some(cmds));
        add(&mut t, f(13), NodeKind::Exe(Exe::Dir), true, Some(cmds));
        add(&mut t, f(14), NodeKind::Exe(Exe::Read), true, Some(cmds));
        add(&mut t, f(15), NodeKind::Exe(Exe::Echo), false, Some(cmds));
        add(&mut t, f(16), NodeKind::Exe(Exe::Help), true, Some(cmds));
        for i in 17..=19 {
            add(&mut t, f(i), NodeKind::Sys, false, Some(cmds));
        }
        if cx.b("ALLIED_WC") {
            add(&mut t, f(20), NodeKind::Exe(Exe::Threads { wc: true }), true, Some(root));
        } else {
            add(&mut t, f(21), NodeKind::Exe(Exe::Threads { wc: false }), true, Some(root));
        }
        t
    }

    fn store(&self) {
        if let Ok(mut g) = TREE.lock() {
            *g = Some(self.t.clone());
        }
    }

    fn s(&self, class: &str, k: &str, i: u32) -> String {
        self.cls.get(class).map(|w| w.at(k, i)).unwrap_or_default()
    }

    fn files(&self, f: usize) -> &[usize] {
        match &self.t.nodes[f].kind {
            NodeKind::Folder { files, .. } => files,
            _ => &[],
        }
    }
    fn is_folder(&self, i: usize) -> bool {
        matches!(self.t.nodes[i].kind, NodeKind::Folder { .. })
    }
    fn find(&self, f: usize, name: &str) -> Option<usize> {
        self.files(f).iter().copied().find(|&i| self.t.nodes[i].name.eq_ignore_ascii_case(name))
    }
    /// dxigFolder.PathString (root's own is "root:\").
    fn path_string(&self, f: usize) -> String {
        let n = &self.t.nodes[f];
        if f == self.t.root {
            return format!("{}:\\", n.name);
        }
        let Some(mut p) = n.parent else { return "DELETED".into() };
        if !self.is_folder(f) {
            return format!("{}\\{}", self.path_string(p), n.name);
        }
        let mut path = n.name.clone();
        while p != self.t.root {
            path = format!("{}\\{path}", self.t.nodes[p].name);
            match self.t.nodes[p].parent {
                Some(q) => p = q,
                None => return "DELETED".into(),
            }
        }
        format!("{}:\\{path}", self.t.nodes[p].name)
    }
    /// get_file: ".." is the parent (at the root, itself), "." the folder, else a file in it.
    fn get_file(&self, f: usize, name: &str) -> Option<usize> {
        match name {
            ".." => Some(self.t.nodes[f].parent.unwrap_or(f)),
            "." => Some(f),
            _ => self.find(f, name),
        }
    }
    /// resolve_file: "root:" or the current folder, then each folder step; the last word looked up there.
    fn resolve(&self, s: &str) -> Option<usize> {
        let s = s.replace('/', "\\");
        let mut f = if s.len() >= 5 && s[..5].eq_ignore_ascii_case("root:") { self.t.root } else { self.cur };
        let mut name = s.clone();
        let mut folder = false;
        let mut first = false;
        let mut rest = s.as_str();
        while !rest.is_empty() {
            let end = rest.find('\\').unwrap_or(rest.len());
            if end == 0 {
                rest = &rest[1..];
                continue;
            }
            let seg = &rest[..end];
            let mut after = &rest[end..];
            let slashes = after.len() - after.trim_start_matches('\\').len();
            after = &after[slashes..];
            name = seg.to_string();
            if slashes == 0 {
                folder = false;
                break;
            }
            folder = true;
            rest = after;
            if !first && seg.eq_ignore_ascii_case("root:") {
                continue;
            }
            first = true;
            f = self.get_file(f, seg).filter(|&i| self.is_folder(i))?;
        }
        if name.eq_ignore_ascii_case("root:") {
            return Some(self.t.root);
        }
        let r = self.get_file(f, &name);
        if folder {
            r.filter(|&i| self.is_folder(i))
        } else {
            r
        }
    }
    fn in_use(&self, i: usize) -> bool {
        self.t.nodes[i].in_use || self.files(i).iter().any(|&k| self.in_use(k))
    }
    fn remove(&mut self, i: usize) {
        if let Some(p) = self.t.nodes[i].parent.take() {
            if let NodeKind::Folder { files, .. } = &mut self.t.nodes[p].kind {
                files.retain(|&k| k != i);
            }
        }
    }
    /// dxigFile / dxigFolder / dxigFileSys .Delete.
    fn delete(&mut self, c: &mut Console, i: usize) {
        let name = self.t.nodes[i].name.clone();
        match self.t.nodes[i].kind {
            NodeKind::Sys => {
                c.say(&fmtn(&self.s("dxigFileSys", "strAccessDenied", 0), &[&name.to_uppercase()]));
                c.say(&self.s("dxigFileSys", "strAccessDenied", 1));
            }
            NodeKind::Folder { .. } => {
                if self.in_use(i) {
                    c.say(&fmtn(&self.s("dxigFolder", "strAccessDenied", 0), &[&name]));
                } else if self.t.nodes[i].read_only {
                    c.say(&fmtn(&self.s("dxigFolder", "strAccessDenied", 1), &[&name]));
                } else {
                    self.remove(i);
                    c.say(&self.s("dxigFolder", "strAccessDenied", 2));
                }
            }
            _ => {
                if self.t.nodes[i].in_use {
                    c.say(&fmtn(&self.s("dxigFile", "strAccessDenied", 0), &[&name]));
                } else if self.t.nodes[i].read_only {
                    c.say(&fmtn(&self.s("dxigFile", "strAccessDenied", 1), &[&name]));
                } else {
                    self.remove(i);
                }
            }
        }
    }
    /// An exe by name in the current folder, then in sys (with .com / .exe / .bat).
    fn find_exe(&self, cmd: &str, in_cur: bool) -> Option<(usize, Exe)> {
        let exe = |i: usize| match self.t.nodes[i].kind {
            NodeKind::Exe(e) => Some((i, e)),
            _ => None,
        };
        let mut places = Vec::new();
        if in_cur {
            places.push(self.cur);
        }
        if self.t.nodes[self.t.cmds].parent.is_some() || self.t.cmds == self.cur {
            places.push(self.t.cmds);
        }
        for f in places {
            for ext in ["", ".com", ".exe", ".bat"] {
                // (a file that isn't an exe stops the search there: the cast fails)
                if let Some(i) = self.find(f, &format!("{cmd}{ext}")) {
                    if let Some(e) = exe(i) {
                        return Some(e);
                    }
                }
            }
        }
        None
    }

    fn parse(&mut self, c: &mut Console, cmd: &str, args: &[String], raw: &str, raw_args: &str) -> bool {
        let p = format!("{}>{raw}", self.path_string(self.cur));
        c.say(&p);
        let (mut cmd, mut args, mut raw_args) = (cmd.to_string(), args.to_vec(), raw_args.to_string());
        if raw.eq_ignore_ascii_case("cd..") {
            cmd = "cd".into();
            args = vec!["..".into()];
            raw_args = "..".into();
        }
        let Some((i, exe)) = self.find_exe(&cmd, true) else { return false };
        self.t.nodes[i].in_use = true;
        let a1 = args.first().cloned().unwrap_or_default();
        let help = |d: &Dxig, c: &mut Console, class: &str, n: u32| {
            for k in 0..n {
                c.say(&d.s(class, "strHelp", k));
            }
        };
        match exe {
            Exe::Cd => {
                if a1.is_empty() {
                    c.say(&self.path_string(self.cur));
                } else if a1 == "/?" {
                    help(self, c, "dxigExeCD", 3);
                } else {
                    match self.resolve(&a1).filter(|&f| self.is_folder(f)) {
                        None => c.say(&fmtn(&self.s("dxigExeCD", "strError", 0), &[&a1])),
                        Some(f) => {
                            let access = match self.t.nodes[f].kind {
                                NodeKind::Folder { access, .. } => access,
                                _ => 0,
                            };
                            if access > self.permissions {
                                let name = self.t.nodes[f].name.clone();
                                c.say(&fmtn(&self.s("dxigExeCD", "strError", 1), &[&name]));
                                c.say(&self.s("dxigExeCD", "strError", 2));
                                let (p, a) = (self.permissions.to_string(), access.to_string());
                                c.say(&fmtn(&self.s("dxigExeCD", "strError", 3), &[&self.user, &p, &a]));
                            } else {
                                self.cur = f;
                                c.say(&format!("{}>", self.path_string(f)));
                            }
                        }
                    }
                }
            }
            Exe::Del => {
                if a1 == "/?" {
                    help(self, c, "dxigExeDel", 2);
                } else {
                    for a in args.iter().filter(|a| !a.is_empty()) {
                        match self.find(self.cur, a) {
                            None => c.say(&fmtn(&self.s("dxigExeDel", "strError", 0), &[a])),
                            Some(f) => self.delete(c, f),
                        }
                    }
                }
            }
            Exe::Dir => {
                if a1 == "/?" {
                    help(self, c, "dxigExeDir", 2);
                } else {
                    let name = self.t.nodes[self.cur].name.clone();
                    c.say(&fmtn(&self.s("dxigExeDir", "strContents", 0), &[&name]));
                    for &k in self.files(self.cur).to_vec().iter() {
                        let pre = if self.is_folder(k) { "<DIR> " } else { "" };
                        c.say(&format!("{pre}{}", self.t.nodes[k].name));
                    }
                }
            }
            Exe::Read => {
                if a1 == "/?" {
                    help(self, c, "dxigExeRead", 2);
                } else {
                    match self.find(self.cur, &a1) {
                        None => c.say(&fmtn(&self.s("dxigExeRead", "strError", 0), &[&a1])),
                        Some(f) => match &self.t.nodes[f].kind {
                            NodeKind::Txt(t) => {
                                for l in t.clone().split("|n") {
                                    c.say(l);
                                }
                            }
                            _ => c.say(&fmtn(&self.s("dxigExeRead", "strError", 1), &[&a1])),
                        },
                    }
                }
            }
            Exe::Echo => {
                if a1 == "/?" {
                    help(self, c, "dxigExeEcho", 2);
                } else {
                    c.say(&raw_args);
                }
            }
            Exe::Help => {
                if a1.is_empty() {
                    for k in 0..6 {
                        c.say(&self.s("dxigExeHelp", "strCommands", k));
                    }
                } else if a1 == "/?" {
                    help(self, c, "dxigExeHelp", 2);
                } else {
                    // Only sys is searched (a command missing from it shows its "None").
                    let found = {
                        let saved = self.cur;
                        self.cur = self.t.cmds;
                        let r = self.find_exe(&a1, false);
                        self.cur = saved;
                        r
                    };
                    match found {
                        None => c.say(&fmtn(&self.s("dxigExeHelp", "strError", 0), &["None"])),
                        Some((_, e)) => {
                            let (class, n) = match e {
                                Exe::Cd => ("dxigExeCD", 3),
                                Exe::Del => ("dxigExeDel", 2),
                                Exe::Dir => ("dxigExeDir", 2),
                                Exe::Read => ("dxigExeRead", 2),
                                Exe::Echo => ("dxigExeEcho", 2),
                                Exe::Help => ("dxigExeHelp", 2),
                                Exe::Threads { .. } => ("", 0),
                            };
                            help(self, c, class, n);
                        }
                    }
                }
            }
            Exe::Threads { wc } => {
                if self.permissions < 2 {
                    if wc {
                        c.say(&self.s("dxigExeThreads", "strWCLines", 0));
                        c.say(&self.s("dxigExeThreads", "strWCLines", 1));
                    } else {
                        c.say(&self.s("dxigExeThreads", "strPDXLines", 0));
                    }
                } else {
                    // dxigInfo.FinishGame -> state Finishing.
                    c.sensitive = false;
                    c.busy = true;
                    let k = if wc { "strWCLines" } else { "strPDXLines" };
                    c.say(&self.s("dxigInfo", k, 0));
                    self.finishing = Some((wc, 0.25, 7));
                    self.t.nodes[i].read_only = false;
                }
            }
        }
        self.t.nodes[i].in_use = false;
        self.store();
        true
    }

    fn tick(&mut self, dt: f32, c: &mut Console) {
        let Some((wc, left, dots)) = self.finishing.as_mut() else { return };
        *left -= dt;
        if *left > 0.0 {
            return;
        }
        *left = 0.25;
        c.push(".");
        *dots -= 1;
        if *dots > 0 {
            return;
        }
        let wc = *wc;
        self.finishing = None;
        c.sensitive = true;
        c.busy = false;
        let k = if wc { "strWCLines" } else { "strPDXLines" };
        c.say(&self.s("dxigInfo", k, 1));
        fx(Effect::Event("MS_ThreadsHacked".into()));
    }
}

// ------------------------------------------------------------------------------------------------ dxeg / abig

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Dxe,
    Abi,
}

/// dxegInterface (one summons a day out of the editing archive) and abigInterface (one weapon off the assembly
/// line): "spawn <number or name>" and "list".
// uc: dxegInterface.Parse, dxegInterface.IsValidClass, dxegInterface.GetSpawnClass, dxegInterface.GetReturnString
// uc: dxegInterface.CheckString, dxegInterface.SummonActor, abigInterface.Parse, abigInterface.SummonActor
struct Dispenser {
    w: Words,
    kind: Kind,
    /// validClasses: (NeatName, className) by slot, empty slots kept so numbering skips them.
    valid: Vec<(String, String)>,
    spawn_tag: String,
    mover_tags: Vec<String>,
    /// tnmUtil.LookupClass finds the class in the player's packages.
    exists: Vec<bool>,
}

impl Dispenser {
    fn empty() -> Dispenser {
        Dispenser { w: Words { d: Vec::new() }, kind: Kind::Dxe, valid: Vec::new(), spawn_tag: String::new(), mover_tags: Vec::new(), exists: Vec::new() }
    }
    fn new(assets: &mut RuntimeAssets, c: &mut Console, kind: Kind) -> Dispenser {
        let class = if kind == Kind::Dxe { "dxegInterface" } else { "abigInterface" };
        let w = defaults(assets, class);
        let mut valid = vec![(String::new(), String::new()); 128];
        for p in w.d.iter().filter(|p| p.name.eq_ignore_ascii_case("validClasses")) {
            if let (Some(slot), Value::Struct(_, b)) = (valid.get_mut(p.index as usize), &p.value) {
                let s = struct_strings(b);
                *slot = (s.first().cloned().unwrap_or_default(), s.get(1).cloned().unwrap_or_default());
            }
        }
        let name = |k: &str| match unreal_pkg::props::find(&w.d, k) {
            Some(Value::Name(n)) | Some(Value::Str(n)) => n.clone(),
            _ => String::new(),
        };
        let spawn_tag = name("spawnLocationTag");
        let mover_tags = ["spawnMoverTag", "spawnMoverTag2"].iter().map(|k| name(k)).filter(|s| !s.is_empty()).collect();
        let exists = valid
            .iter()
            .map(|(_, cl)| {
                let short = cl.rsplit('.').next().unwrap_or(cl);
                !short.is_empty() && !assets.classes.defaults(short).is_empty()
            })
            .collect();
        c.say(&w.at("miscLines", 0));
        c.say(&w.at("miscLines", 1));
        c.say(" ");
        c.say(&w.at("miscLines", 2));
        c.say(" ");
        Dispenser { w, kind, valid, spawn_tag, mover_tags, exists }
    }
    /// CheckString: any letter of filterString in it (so "12" is a number and "Baton" a name).
    fn has_letters(&self, s: &str) -> bool {
        let f = self.w.s("filterString");
        s.chars().any(|c| f.contains(c))
    }
    /// GetSpawnClass / IsValidClass: the n-th listed item, or one by its name.
    fn pick(&self, raw: &str) -> Option<usize> {
        if raw.is_empty() {
            return None;
        }
        let n: i64 = raw.trim().parse().unwrap_or_else(|_| raw.trim().chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0));
        let mut j = 0;
        for (i, (neat, _)) in self.valid.iter().enumerate() {
            if neat.is_empty() {
                continue;
            }
            j += 1;
            if (j == n && !self.has_letters(raw)) || raw.eq_ignore_ascii_case(neat) {
                return Some(i);
            }
        }
        None
    }
    fn parse(&mut self, c: &mut Console, cmd: &str, raw_args: &str, raw: &str) -> bool {
        c.say(&format!(">{raw}"));
        let list = self.w.list("command_list");
        let up = cmd.to_uppercase();
        let is = |i: usize| list.get(i).is_some_and(|x| x.to_uppercase() == up);
        if is(0) {
            let flag = if self.kind == Kind::Dxe { "DXE_Game_Dispensed" } else { "ABI_Game_Dispensed" };
            if DISPENSED.lock().map(|d| d.contains(&flag.to_string())).unwrap_or(false) || crate::tnmconsole::flag_now(flag) {
                c.say(&self.w.at("strReturn", 0));
                return true;
            }
            match self.pick(raw_args) {
                Some(i) => {
                    let class = self.valid[i].1.clone();
                    if !spawn_point_exists(&self.spawn_tag) {
                        c.say(&self.w.at("strReturn", 6));
                        c.say(&self.w.at("strReturn", 4));
                        return true;
                    }
                    let short = class.rsplit('.').next().unwrap_or(&class).to_string();
                    if !self.exists.get(i).copied().unwrap_or(false) {
                        c.say(&self.w.at("strReturn", 5));
                        c.say(&self.w.at("strReturn", 4));
                        return true;
                    }
                    match self.kind {
                        Kind::Dxe => {
                            fx(Effect::Summon { class: short, at_tag: self.spawn_tag.clone(), legendary: true });
                            for t in &self.mover_tags {
                                fx(Effect::Event(t.clone()));
                            }
                            fx(Effect::Achievement("achApocalypse".into()));
                        }
                        Kind::Abi => {
                            fx(Effect::Flag("MS_SpawnABIGameItem".into(), true));
                            fx(Effect::NameFlag("ABISpawnItem".into(), class.clone()));
                        }
                    }
                    fx(Effect::Flag(flag.into(), true));
                    if let Ok(mut d) = DISPENSED.lock() {
                        d.push(flag.to_string());
                    }
                    let shown = if raw_args.trim().parse::<i64>().is_ok_and(|n| n != 0) && !self.has_letters(raw_args) {
                        format!("{raw_args}. {}", self.valid[i].0)
                    } else {
                        raw_args.to_string()
                    };
                    c.say(&fmtn(&self.w.at("strReturn", 2), &[&shown]));
                }
                None if raw_args.is_empty() => c.say(&self.w.at("strReturn", 1)),
                None => c.say(&fmtn(&self.w.at("strReturn", 3), &[raw_args])),
            }
            return true;
        }
        if is(1) {
            c.say(" ");
            c.say(&self.w.s("strInventory"));
            c.say(" ");
            let mut j = 0;
            for (neat, _) in self.valid.clone() {
                if !neat.is_empty() {
                    j += 1;
                    c.say(&format!("{j}. {neat}"));
                }
            }
            return true;
        }
        false
    }
}

/// Dispensed this map (the flag set this frame isn't in the flag base until computers.rs applies it).
static DISPENSED: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Flags as the last terminal frame saw them (for checks made inside parse).
static FLAGS_NOW: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// The level's actor tags (spawn points).
static TAGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn flag_now(n: &str) -> bool {
    FLAGS_NOW.lock().map(|f| f.iter().any(|x| x.eq_ignore_ascii_case(n))).unwrap_or(false)
}
fn spawn_point_exists(tag: &str) -> bool {
    TAGS.lock().map(|t| t.iter().any(|x| x.eq_ignore_ascii_case(tag))).unwrap_or(false)
}

/// computers.rs: the flags a console checks, and the level's tags, before a console runs this frame.
pub fn note_world(flags: &imsim_core::flags::FlagBase, lvl: &dx_level::Level) {
    if let Ok(mut f) = FLAGS_NOW.lock() {
        f.clear();
        for n in ["DXE_Game_Dispensed", "ABI_Game_Dispensed"] {
            if matches!(flags.get(n), Some(imsim_core::flags::FlagValue::Bool(true))) {
                f.push(n.to_string());
            }
        }
    }
    if let Ok(mut t) = TAGS.lock() {
        if t.is_empty() {
            for a in &lvl.actors {
                if let Some(Value::Name(n)) = unreal_pkg::props::find(&a.props, "Tag") {
                    t.push(n.clone());
                }
            }
            t.push(String::from("\u{0}")); // marks the list as built
        }
    }
}

pub fn reset_world() {
    if let Ok(mut t) = TAGS.lock() {
        t.clear();
    }
    if let Ok(mut d) = DISPENSED.lock() {
        d.clear();
    }
}

// ------------------------------------------------------------------------------------------------ chat (shared)

/// TextConsoleIRCInterface: the channel's users and the lines every client prints.
// uc: TextConsoleIRCInterface.AddUser, TextConsoleIRCInterface.RebuildUsers, TextConsoleIRCInterface.NewUser
// uc: TextConsoleIRCInterface.Join, TextConsoleIRCInterface.UserQuit, TextConsoleIRCInterface.Part, TextConsoleIRCInterface.Kick
// uc: TextConsoleIRCInterface.Connect, TextConsoleIRCInterface.Kicked, TextConsoleIRCInterface.SetMode
// uc: TextConsoleIRCInterface.SetTopic, TextConsoleIRCInterface.Talk, TextConsoleIRCInterface.PrivateText
// uc: TextConsoleIRCInterface.Action, TextConsoleIRCInterface.GetTimeStamp, TextConsoleIRCInterface.ChangedNick
// uc: TextConsoleIRCInterface.ChangeOp, TextConsoleIRCInterface.JoinedChannel, TextConsoleIRCInterface.Disconnected
#[derive(Default)]
struct Chat {
    nick: String,
    server: String,
    channel: String,
    /// (name, prefix)
    users: Vec<(String, String)>,
    in_channel: bool,
}

impl Chat {
    fn msg(&self, c: &mut Console, s: &str) {
        let (_, _, _, h, m, _, _) = civil(now_secs());
        c.say(&format!("[{h:02}:{m:02}] {s}"));
    }
    fn rebuild(&self, c: &mut Console) {
        let list = if self.in_channel {
            self.users
                .iter()
                .map(|(n, p)| {
                    let pr = match p.as_str() {
                        "@" => 3,
                        "%" => 2,
                        "+" => 1,
                        _ => 0,
                    };
                    (format!("{p}{n}"), pr)
                })
                .collect()
        } else {
            Vec::new()
        };
        c.users = Some(list);
    }
    fn new_user(&mut self, c: &mut Console, user: &str) {
        if self.users.iter().any(|u| u.0 == user) {
            return;
        }
        self.users.push((user.to_string(), String::new()));
        self.rebuild(c);
    }
    fn join(&mut self, c: &mut Console, user: &str) {
        if user != self.nick {
            self.msg(c, &format!("* {user} has joined {}", self.channel));
        } else {
            self.in_channel = true;
        }
        self.new_user(c, user);
    }
    fn remove(&mut self, user: &str) {
        if let Some(i) = self.users.iter().position(|u| u.0 == user) {
            self.users.remove(i);
        }
    }
    fn part(&mut self, c: &mut Console, user: &str) {
        if user == self.nick {
            self.msg(c, &format!("* You have left {}", self.channel));
        } else {
            self.msg(c, &format!("* {user} has left {}", self.channel));
        }
        self.remove(user);
        self.rebuild(c);
    }
    fn quit(&mut self, c: &mut Console, user: &str, reason: &str) {
        if user == self.nick {
            return;
        }
        self.msg(c, &format!("* {user} has quit irc (Reason: {reason})."));
        self.remove(user);
        self.rebuild(c);
    }
    /// Kick: true when it was us (the caller then tries to rejoin).
    fn kick(&mut self, c: &mut Console, user: &str, kicker: &str, reason: &str) -> bool {
        let reason = if reason.is_empty() { user } else { reason };
        if user == self.nick {
            self.msg(c, &format!("* You have been kicked by {kicker} (Reason: {reason})"));
        } else {
            self.msg(c, &format!("* {user} has been kicked by {kicker} (Reason: {reason})"));
        }
        self.remove(user);
        if user == self.nick {
            return true;
        }
        self.rebuild(c);
        false
    }
    fn set_mode(&mut self, c: &mut Console, user: &str, modes: &str) {
        self.msg(c, &format!("* {user} sets mode: {modes}"));
        self.rebuild(c);
    }
    fn set_topic(&mut self, c: &mut Console, user: &str, topic: &str) {
        self.msg(c, &format!("* {user} sets topic: '{topic}'"));
    }
    fn talk(&self, c: &mut Console, user: &str, text: &str) {
        self.msg(c, &format!("<{user}> {text}"));
    }
    fn private(&self, c: &mut Console, user: &str, text: &str) {
        self.msg(c, &format!("*{user}* {text}"));
    }
    fn action(&self, c: &mut Console, user: &str, text: &str) {
        self.msg(c, &format!("* {user} {text}"));
    }
    fn prefix(&mut self, c: &mut Console, user: &str, on: bool, p: &str) {
        if let Some(u) = self.users.iter_mut().find(|u| u.0 == user) {
            u.1 = if on { p.to_string() } else { String::new() };
        }
        self.rebuild(c);
    }
    fn changed_nick(&mut self, c: &mut Console, old: &str, new: &str) {
        if let Some(u) = self.users.iter_mut().find(|u| u.0 == old) {
            u.0 = new.to_string();
        }
        if old == self.nick {
            self.msg(c, &format!("* You are now known as {new}"));
            self.nick = new.to_string();
        } else {
            self.msg(c, &format!("* {old} is now known as {new}"));
        }
        self.rebuild(c);
    }
    fn joined_channel(&mut self, c: &mut Console, topic: &str, topic_user: &str, topic_time: &str, modes: &str) {
        let nick = self.nick.clone();
        self.join(c, &nick);
        self.msg(c, &format!("* Now talking in {}", self.channel));
        if !topic.is_empty() {
            self.msg(c, &format!("* Topic is: '{topic}'"));
            self.msg(c, &format!("* Set by: {topic_user} on {topic_time}"));
        }
        self.msg(c, &format!("* Channel modes: {modes}"));
    }
    fn user_set_topic(&self, c: &mut Console, user: &str, topic: &str) {
        if user == self.nick {
            self.msg(c, &format!("* You set the topic to: '{topic}'."));
        } else {
            self.msg(c, &format!("* {user} sets the topic to: '{topic}'."));
        }
    }
}

// ------------------------------------------------------------------------------------------------ the scripted chat

/// tnmFakeIRCWindow + FakeIRCInterface: Narcissus Relay Chat. The conversation is the window's IRCUpdate script
/// (run a step per timer tick through the uscript loader); the topic, its author and the modes are the
/// interface's own script functions.
// uc: tnmFakeIRCWindow.CreateControls, tnmFakeIRCWindow.IRCTimer, tnmFakeIRCWindow.IRCUpdate, tnmFakeIRCWindow.talk
// uc: tnmFakeIRCWindow.act, tnmFakeIRCWindow.kick, tnmFakeIRCWindow.mode, tnmFakeIRCWindow.topic, tnmFakeIRCWindow.join
// uc: tnmFakeIRCWindow.part, FakeIRCInterface.Parse, FakeIRCInterface.Init, FakeIRCInterface.GenerateUsers
// uc: FakeIRCInterface.AttemptConnect, FakeIRCInterface.JoinChannel, FakeIRCInterface.GetTopicTime, FakeIRCInterface.OpPhas
struct FakeIrc {
    w: Words,
    win: Words,
    chat: Chat,
    script: crate::uscript::Script,
    iface: crate::uscript::Script,
    irca: i64,
    timer: Option<f32>,
    banned: bool,
    no_retry: bool,
    rng: Rng,
}

/// The window script's host: records what one IRCUpdate step does.
struct FakeHost<'a> {
    irca: &'a mut i64,
    win: &'a Words,
    out: Vec<(String, Vec<String>)>,
    timer: Option<f32>,
    banned: Option<bool>,
}

fn v_str(v: &crate::uscript::V) -> String {
    match v {
        crate::uscript::V::Str(s) => s.clone(),
        crate::uscript::V::Int(i) => i.to_string(),
        crate::uscript::V::Float(f) => f.to_string(),
        crate::uscript::V::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}
fn v_f(v: &crate::uscript::V) -> f32 {
    match v {
        crate::uscript::V::Int(i) => *i as f32,
        crate::uscript::V::Float(f) => *f as f32,
        _ => 0.0,
    }
}

impl crate::uscript::Host for FakeHost<'_> {
    fn call(&mut self, name: &str, args: &[crate::uscript::V]) -> Option<crate::uscript::V> {
        let s = |i: usize| args.get(i).map(v_str).unwrap_or_default();
        let (n_text, n_time) = match name {
            "talk" | "act" => (2, Some(2)),
            "kick" => (3, Some(3)),
            "mode" | "topic" => (2, Some(2)),
            "join" | "part" => (1, Some(1)),
            "irctimer" => {
                self.timer = Some(args.first().map(v_f).unwrap_or(0.0));
                return Some(crate::uscript::V::None);
            }
            _ => return None,
        };
        self.out.push((name.to_string(), (0..n_text).map(s).collect()));
        if let Some(t) = n_time {
            self.timer = Some(args.get(t).map(v_f).unwrap_or(0.0));
        }
        Some(crate::uscript::V::None)
    }
    fn get(&mut self, name: &str) -> crate::uscript::V {
        match name {
            "irca" => crate::uscript::V::Int(*self.irca),
            "interface" => crate::uscript::V::Obj(1),
            _ => crate::uscript::V::None,
        }
    }
    fn get_index(&mut self, name: &str, index: i64, _field: Option<&str>) -> crate::uscript::V {
        crate::uscript::V::Str(self.win.at(name, index.max(0) as u32))
    }
    fn set(&mut self, path: &str, v: &crate::uscript::V) {
        match path {
            "irca" => *self.irca = match v {
                crate::uscript::V::Int(i) => *i,
                _ => *self.irca,
            },
            "interface.bbanned" => self.banned = Some(matches!(v, crate::uscript::V::Bool(true))),
            _ => {}
        }
    }
    fn method(&mut self, _recv: &crate::uscript::V, name: &str, args: &[crate::uscript::V]) -> (crate::uscript::V, Vec<(usize, crate::uscript::V)>) {
        let name = name.to_ascii_lowercase();
        self.out.push((format!("i:{name}"), args.iter().map(v_str).collect()));
        (crate::uscript::V::None, Vec::new())
    }
}

/// The interface script's host (GetTopic / GetTopicUser / GetChannelModes return literals).
struct NoHost;
impl crate::uscript::Host for NoHost {
    fn call(&mut self, _name: &str, _args: &[crate::uscript::V]) -> Option<crate::uscript::V> {
        None
    }
    fn get(&mut self, _name: &str) -> crate::uscript::V {
        crate::uscript::V::None
    }
    fn get_index(&mut self, _name: &str, _index: i64, _field: Option<&str>) -> crate::uscript::V {
        crate::uscript::V::None
    }
}

impl FakeIrc {
    fn new(assets: &mut RuntimeAssets, c: &mut Console, cx: &Ctx, seed: u64) -> FakeIrc {
        let w = defaults(assets, "FakeIRCInterface");
        let win = defaults(assets, "tnmFakeIRCWindow");
        let mut script = crate::uscript::Script::default();
        script.objects = true;
        script.add_source(&assets.classes.script_text("tnmFakeIRCWindow").unwrap_or_default());
        let mut iface = crate::uscript::Script::default();
        iface.add_source(&assets.classes.script_text("FakeIRCInterface").unwrap_or_default());
        let chat = Chat { nick: w.s("Nick"), server: w.s("Server"), channel: w.s("Channel"), users: Vec::new(), in_channel: false };
        let mut f = FakeIrc { w, win, chat, script, iface, irca: 0, timer: None, banned: false, no_retry: false, rng: Rng(seed | 1) };
        // GenerateUsers, then Connect.
        for i in 0..32 {
            let n = f.w.at("def_users", i);
            if n.is_empty() {
                break;
            }
            let p = f.w.at("def_prefixes", i);
            f.chat.users.push((n, p));
        }
        f.connect(c);
        // CreateControls: the first time the conversation plays (and NARC_IRC is set); after that it won't start.
        if !cx.b("NARC_IRC") {
            f.timer = Some(2.0);
            fx(Effect::Flag("NARC_IRC".into(), true));
        } else {
            let (a, b) = (f.win.at("strError", 0), f.win.at("strError", 1));
            f.chat.msg(c, &a);
            f.chat.msg(c, &b);
        }
        f
    }
    fn message(&self, c: &mut Console, s: &str) {
        self.chat.msg(c, s);
    }
    fn iface_str(&mut self, f: &str) -> String {
        v_str(&self.iface.call(&mut NoHost, f, &[]))
    }
    fn topic_time(&mut self) -> String {
        // GetTopicTime: a few minutes ago.
        let back = (self.rng.rand(4) + 1) as u64 * 60;
        let (_, mo, d, h, m, _, wd) = civil(now_secs().saturating_sub(back));
        let s = self.rng.rand(60);
        format!("{} {} {} {:02}:{:02}:{:02}", DAYS[wd as usize], MONTHS[(mo - 1) as usize], d, h, m, s)
    }
    /// TextConsoleIRCInterface.Connect with FakeIRCInterface's AttemptConnect / JoinChannel.
    fn connect(&mut self, c: &mut Console) {
        let s = format!("* Connecting to {}...", self.chat.server);
        self.chat.msg(c, &s);
        if self.banned {
            let s = self.w.at("strError", 1);
            self.chat.msg(c, &s);
            self.no_retry = true;
            return;
        }
        let s = format!("* Joining {}", self.chat.channel);
        self.chat.msg(c, &s);
        self.rejoin(c);
    }
    fn rejoin(&mut self, c: &mut Console) -> bool {
        if self.banned {
            let s = fmtn(&self.w.at("strError", 2), &[&self.chat.channel]);
            self.chat.msg(c, &s);
            return false;
        }
        self.chat.in_channel = true;
        self.chat.rebuild(c);
        let (t, u, m) = (self.iface_str("gettopic"), self.iface_str("gettopicuser"), self.iface_str("getchannelmodes"));
        let tt = self.topic_time();
        self.chat.joined_channel(c, &t, &u, &tt, &m);
        true
    }
    fn tick(&mut self, dt: f32, c: &mut Console) {
        let Some(t) = self.timer.as_mut() else { return };
        *t -= dt;
        if *t > 0.0 {
            return;
        }
        self.timer = None;
        let mut host = FakeHost { irca: &mut self.irca, win: &self.win, out: Vec::new(), timer: None, banned: None };
        self.script.call(&mut host, "ircupdate", &[]);
        let (out, timer, banned) = (std::mem::take(&mut host.out), host.timer, host.banned);
        if let Some(b) = banned {
            self.banned = b;
        }
        for (what, a) in out {
            let g = |i: usize| a.get(i).cloned().unwrap_or_default();
            match what.as_str() {
                "talk" => self.chat.talk(c, &g(0), &g(1)),
                "act" => self.chat.action(c, &g(0), &g(1)),
                "kick" => {
                    if self.chat.kick(c, &g(0), &g(1), &g(2)) {
                        // Kicked: try to rejoin.
                        let s = format!("* Attempting to rejoin {}", self.chat.channel);
                        self.chat.msg(c, &s);
                        self.chat.in_channel = false;
                        self.chat.rebuild(c);
                        self.rejoin(c);
                    }
                }
                "mode" => self.chat.set_mode(c, &g(0), &g(1)),
                "topic" => self.chat.set_topic(c, &g(0), &g(1)),
                "join" => self.chat.join(c, &g(0)),
                "part" => self.chat.part(c, &g(0)),
                "i:opphas" => {
                    // OpPhas: ChanServ gives the half-op mark.
                    if let Some(u) = self.chat.users.iter().position(|u| u.0.eq_ignore_ascii_case("Phasmatis")) {
                        let name = self.chat.users[u].0.clone();
                        self.chat.set_mode(c, "ChanServ", &format!("+h {name}"));
                        self.chat.users[u].1 = "%".into();
                    }
                    self.chat.rebuild(c);
                }
                "i:disconnected" => {
                    let s = format!("* Disconnected (Reason: {})", g(0));
                    self.chat.msg(c, &s);
                    self.chat.in_channel = false;
                    self.chat.msg(c, "* Attempting to reconnect.");
                    self.connect(c);
                }
                "i:consolemessage" => self.chat.msg(c, &g(0)),
                _ => {}
            }
        }
        if let Some(t) = timer {
            self.timer = Some(t.max(0.0));
        }
    }
}

// ------------------------------------------------------------------------------------------------ the real chat

enum NetIn {
    Opened,
    Line(String),
    Failed(String),
    Closed,
}

/// tnmIRCWindow + tnmIRCInterface + tnmIRCLink: an IRC client (default server and channel from the mod), on a
/// thread of its own. Nothing connects until the player names himself.
// uc: tnmIRCInterface.Parse, tnmIRCInterface.Init, tnmIRCInterface.PromptNick, tnmIRCInterface.Connect
// uc: tnmIRCInterface.ConnectFailed, tnmIRCInterface.Disconnected, tnmIRCInterface.SetTopicTime, tnmIRCInterface.ChangedNick
// uc: tnmIRCLink.Connect, tnmIRCLink.SendCommandText, tnmIRCLink.SendChannelText, tnmIRCLink.SendChannelAction
// uc: tnmIRCLink.SendPrivateText, tnmIRCLink.ProcessInput, tnmIRCLink.JoinChannel, tnmIRCLink.PartChannel
// uc: tnmIRCLink.DestroyLink, tnmIRCLink.ChopLeft, tnmIRCLink.Chop, tnmIRCLink.RemoveNickPrefix
struct RealIrc {
    w: Words,
    link: Words,
    chat: Chat,
    prompt_nick: bool,
    quitting: bool,
    no_retry: bool,
    logged_in: bool,
    rx: Option<Mutex<mpsc::Receiver<NetIn>>>,
    tx: Option<mpsc::Sender<String>>,
    topic: String,
    topic_user: String,
    topic_time: String,
    modes: String,
    link_nick: String,
    /// Trestkon.GetTNMVersion (its REVISION_VERSION constants).
    version: String,
}

/// Trestkon.GetTNMVersion: major, then minor and build when not zero (constants of the player's Trestkon script).
fn tnm_version(assets: &mut RuntimeAssets) -> String {
    let src = assets.classes.script_text("Trestkon").unwrap_or_default();
    let c = |n: &str| -> i64 {
        src.lines()
            .find_map(|l| {
                let t = l.trim();
                let rest = t.strip_prefix("CONST ").or_else(|| t.strip_prefix("const "))?;
                let (k, v) = rest.split_once('=')?;
                (k.trim().eq_ignore_ascii_case(n)).then(|| v.trim().trim_end_matches(';').trim().parse().unwrap_or(0))
            })
            .unwrap_or(0)
    };
    let mut s = c("REVISION_VERSION_MAJOR").to_string();
    for k in ["REVISION_VERSION_MINOR", "REVISION_VERSION_BUILD"] {
        let v = c(k);
        if v > 0 {
            s.push_str(&format!(".{v}"));
        }
    }
    s
}

fn parse_delimited(line: &str, delims: &str, n: usize, rest: bool) -> String {
    // ParseDelimited: the n-th (1-based) piece split on any delimiter char; with `rest`, everything from it on.
    let mut count = 1;
    let mut start = 0;
    for (i, ch) in line.char_indices() {
        if delims.contains(ch) {
            if count == n {
                return line[start..i].to_string();
            }
            count += 1;
            start = i + ch.len_utf8();
            if count == n && rest {
                return line[start..].to_string();
            }
        }
    }
    if count == n {
        line[start..].to_string()
    } else {
        String::new()
    }
}
fn chop_left(s: &str) -> String {
    s.trim_start_matches([':', ' ', '!']).to_string()
}
fn chop(s: &str) -> String {
    s.trim_matches([':', ' ', '!']).to_string()
}
fn no_prefix(s: &str) -> String {
    s.trim_start_matches([':', '@', '+', '%']).to_string()
}

impl RealIrc {
    fn new(assets: &mut RuntimeAssets, c: &mut Console, cx: &Ctx) -> RealIrc {
        let w = defaults(assets, "tnmIRCInterface");
        let link = defaults(assets, "tnmIRCLink");
        let chat = Chat { nick: String::new(), server: w.s("Server"), channel: w.s("Channel"), users: Vec::new(), in_channel: false };
        let r = RealIrc {
            w,
            link,
            chat,
            prompt_nick: true,
            quitting: false,
            no_retry: false,
            logged_in: false,
            rx: None,
            tx: None,
            topic: String::new(),
            topic_user: String::new(),
            topic_time: String::new(),
            modes: String::new(),
            link_nick: String::new(),
            version: tnm_version(assets),
        };
        // PromptNick, with the player's own name typed in when he chose one.
        let s = r.w.s("strNickname");
        r.chat.msg(c, &s);
        let name = crate::travel::player_name();
        if !name.is_empty() && !name.eq_ignore_ascii_case(crate::tnmmode::default_name()) {
            c.input = name;
        }
        let _ = cx;
        r
    }

    fn send(&self, line: &str) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(format!("{line}\r\n"));
        }
    }

    /// tnmIRCInterface.Connect -> tnmIRCLink.Connect (a thread resolves, opens, then reads lines).
    fn connect(&mut self, c: &mut Console) {
        let s = format!("* Connecting to {}...", self.chat.server);
        self.chat.msg(c, &s);
        crate::tnmach::unlock("achIRC");
        self.quit_link();
        self.link_nick = self.chat.nick.clone();
        let (host, port) = match self.chat.server.split_once(':') {
            Some((h, p)) => (h.to_string(), p.parse::<u16>().unwrap_or(6667)),
            None => (self.chat.server.clone(), 6667),
        };
        let (in_tx, in_rx) = mpsc::channel::<NetIn>();
        let (out_tx, out_rx) = mpsc::channel::<String>();
        let (timeout, resolve_failed, invalid) = (self.link.s("TimeOutError"), self.link.s("ResolveFailedText"), self.link.s("InvalidAddressText"));
        std::thread::Builder::new()
            .name("tnm-irc".into())
            .spawn(move || irc_thread(host, port, in_tx, out_rx, timeout, resolve_failed, invalid))
            .ok();
        self.rx = Some(Mutex::new(in_rx));
        self.tx = Some(out_tx);
        self.logged_in = false;
    }
    fn quit_link(&mut self) {
        self.tx = None;
        self.rx = None;
    }
    /// DestroyLink: QUIT with the mod's reason, then close.
    fn quit(&mut self) {
        if self.tx.is_some() {
            let r = self.link.s("DisconnectReason");
            self.send(&format!("QUIT :{r}"));
        }
        self.quit_link();
    }
    fn connect_failed(&mut self, c: &mut Console) {
        if self.no_retry {
            self.chat.msg(c, "* IRC shutting down...");
            return;
        }
        self.chat.msg(c, "* Attempting to reconnect.");
        self.connect(c);
    }
    fn disconnected(&mut self, c: &mut Console, reason: &str) {
        let s = format!("* Disconnected (Reason: {reason})");
        self.chat.msg(c, &s);
        if self.quitting {
            self.chat.msg(c, "* IRC shutting down...");
            self.quit_link();
            return;
        }
        self.chat.in_channel = false;
        self.chat.users.clear();
        self.chat.rebuild(c);
        self.chat.msg(c, "* Attempting to reconnect.");
        self.connect(c);
    }

    /// tnmIRCLink.SendCommandText.
    fn command(&mut self, c: &mut Console, text: &str) {
        let mut t = text.to_string();
        if t.len() >= 4 && t[..4].eq_ignore_ascii_case("MSG ") {
            t = format!("PRIVMSG {}", &t[4..]);
        }
        if t.len() >= 6 && t[..6].eq_ignore_ascii_case("LEAVE ") {
            t = format!("PART {}", &t[6..]);
        }
        let out = match t.find(' ') {
            Some(i) => {
                let cmd = t[..i].to_uppercase();
                let rest = chop_left(&t[i + 1..]);
                match cmd.as_str() {
                    "PRIVMSG" | "NOTICE" | "KILL" | "TOPIC" => match rest.find(' ') {
                        Some(j) => format!("{cmd} {} :{}", &rest[..j], chop_left(&rest[j + 1..])),
                        None => format!("{cmd}{}", &t[i..]),
                    },
                    "CTCP" => match rest.find(' ') {
                        Some(j) => format!("PRIVMSG {} :\u{1}{}\u{1}", &rest[..j], chop_left(&rest[j + 1..])),
                        None => format!("{cmd}{}", &t[i..]),
                    },
                    "KICK" => {
                        let mut out = format!("{cmd}{}", &t[i..]);
                        if let Some(j) = rest.find(' ') {
                            let r2 = chop_left(&rest[j + 1..]);
                            if let Some(k) = r2.find(' ') {
                                out = format!("{cmd} {} {} :{}", &rest[..j], &r2[..k], chop_left(&r2[k + 1..]));
                            }
                        }
                        out
                    }
                    "QUIT" => format!("{cmd} :{rest}"),
                    _ => format!("{cmd}{}", &t[i..]),
                }
            }
            None => t.to_uppercase(),
        };
        self.send(&out);
        // LoggingIn: a NICK changes ours at once.
        if !self.logged_in && out.len() > 5 && out[..5].eq_ignore_ascii_case("NICK ") {
            let new = chop(&parse_delimited(&out, " ", 2, false));
            let old = self.link_nick.clone();
            self.chat.changed_nick(c, &old, &new);
            self.link_nick = new;
        }
    }
    fn join_channel(&mut self, c: &mut Console, ch: &str) {
        self.chat.msg(c, &format!("* Joining {ch}"));
        if ch.starts_with('#') {
            self.send(&format!("JOIN {ch}"));
        }
    }
    fn part_channel(&mut self, c: &mut Console, ch: &str) {
        self.chat.msg(c, &format!("* Parting {ch}"));
        if ch.starts_with('#') {
            self.send(&format!("PART {ch}"));
        }
    }
    fn say_channel(&mut self, c: &mut Console, text: &str) {
        crate::tnmpc::sound("TNMGUI.Menu_OutGoing");
        let n = self.link_nick.clone();
        self.chat.talk(c, &n, text);
        let ch = self.chat.channel.clone();
        self.send(&format!("PRIVMSG {ch} :{text}"));
    }
    fn act_channel(&mut self, c: &mut Console, ch: &str, text: &str) {
        let n = self.link_nick.clone();
        self.chat.action(c, &n, text);
        self.send(&format!("PRIVMSG {ch} :\u{1}ACTION {text}\u{1}"));
    }

    fn parse(&mut self, c: &mut Console, cmd: &str, args: &[String], raw_args: &str, raw: &str, _cx: &Ctx) -> bool {
        if self.prompt_nick {
            self.chat.nick = raw.trim().to_string();
            self.prompt_nick = false;
            self.connect(c);
            return true;
        }
        let up = cmd.to_uppercase();
        let a0 = args.first().cloned().unwrap_or_default();
        if up == "NICK" {
            self.command(c, &format!("NICK {raw_args}"));
            return true;
        } else if up == "SERVER" {
            self.quitting = true;
            let r = self.link.s("DisconnectReason");
            self.send(&format!("QUIT :{r}"));
            self.quit_link();
            self.quitting = false;
            self.chat.server = a0.clone();
            self.chat.channel.clear();
            self.chat.in_channel = false;
            self.connect(c);
        } else if matches!(up.as_str(), "SWITCH" | "JOIN" | "J") {
            if self.chat.in_channel {
                let ch = self.chat.channel.clone();
                self.part_channel(c, &ch);
            }
            self.chat.channel = a0.clone();
            self.join_channel(c, &a0);
        }
        if !self.chat.in_channel {
            self.chat.msg(c, "Not in channel.");
            return false;
        }
        let mut handled = true;
        let mut cmd = cmd.to_string();
        match up.as_str() {
            "SAY" => self.say_channel(c, raw_args),
            "ME" => {
                let ch = self.chat.channel.clone();
                self.act_channel(c, &ch, raw_args);
            }
            "MSG" | "PRIVMSG" => {
                if args.len() < 2 {
                    let s = fmtn(&self.w.at("strError", 1), &[&cmd]);
                    c.say(&s);
                } else {
                    let rest = raw_args.split_once(' ').map(|x| x.1).unwrap_or("").to_string();
                    if a0.eq_ignore_ascii_case(&self.chat.channel) {
                        self.say_channel(c, &rest);
                    } else {
                        self.chat.private(c, &a0, &rest);
                        self.send(&format!("PRIVMSG {a0} :{rest}"));
                    }
                }
            }
            "QUIT" => {
                self.quitting = true;
                let r = if args.is_empty() { self.link.s("DisconnectReason") } else { raw_args.to_string() };
                self.command(c, &format!("QUIT {r}"));
            }
            "MJOIN" | "MULTIJOIN" => {
                self.chat.channel = a0.clone();
                cmd = "JOIN".into();
                handled = false;
            }
            "MSWITCH" | "MULTISWITCH" => self.chat.channel = a0.clone(),
            "HOP" => {
                let ch = self.chat.channel.clone();
                self.part_channel(c, &ch);
                self.join_channel(c, &ch);
            }
            _ => handled = false,
        }
        if !handled {
            self.command(c, &format!("{cmd} {raw_args}"));
        }
        // (an unknown command went to the server: the console's "not found" isn't shown)
        true
    }

    fn tick(&mut self, c: &mut Console, cx: &Ctx) {
        let Some(rx) = self.rx.as_ref() else { return };
        let mut got = Vec::new();
        if let Ok(rx) = rx.lock() {
            while let Ok(m) = rx.try_recv() {
                got.push(m);
            }
        }
        for m in got {
            match m {
                NetIn::Opened => {
                    let s = self.link.s("ConnectedText");
                    self.chat.msg(c, &s);
                    // LoggingIn: Begin.
                    let host = self.chat.server.split(':').next().unwrap_or("").to_string();
                    self.send(&format!("USER TNM2 localhost {host} :TNM 2.0 User"));
                    let n = self.link_nick.clone();
                    self.send(&format!("NICK {n}"));
                }
                NetIn::Failed(s) => {
                    self.chat.msg(c, &s);
                    self.rx = None;
                    self.tx = None;
                    self.connect_failed(c);
                    return;
                }
                NetIn::Closed => {
                    self.rx = None;
                    self.tx = None;
                    self.disconnected(c, "Connection terminated by user");
                    return;
                }
                NetIn::Line(l) => self.line(c, &l, cx),
            }
        }
    }

    fn line(&mut self, c: &mut Console, line: &str, cx: &Ctx) {
        if let Some(rest) = line.strip_prefix("PING ") {
            self.send(&format!("PONG {rest}"));
        }
        let command = parse_delimited(line, " ", 2, false);
        if parse_delimited(line, " ", 1, false) == "ERROR" {
            self.chat.msg(c, &chop_left(&parse_delimited(line, ":", 2, true)));
            return;
        }
        if !self.logged_in {
            // LoggingIn.ProcessInput
            if command == "433" {
                self.chat.msg(c, &chop_left(&parse_delimited(line, " ", 3, true)));
            } else if command.parse::<i64>().is_ok_and(|n| n != 0) {
                self.chat.msg(c, &chop_left(&parse_delimited(line, " ", 3, true)));
                self.logged_in = true;
                // LoggedIn: Begin
                let ch = self.chat.channel.clone();
                self.join_channel(c, &ch);
            }
            return;
        }
        let from = parse_delimited(line, ":!", 2, false);
        match command.as_str() {
            "JOIN" => {
                self.chat.join(c, &from);
                crate::tnmpc::sound(if from == self.link_nick { "TNMGUI.IRCConnect" } else { "TNMGUI.IRCConnectA" });
                if from == self.link_nick && crate::tnmmode::option("bIRCLocationInfo") {
                    let mut place = cx.level.replace('_', "-");
                    if place.to_uppercase().contains("GAMESPY") {
                        let mut r = Rng(now_secs() | 1);
                        let junk = |r: &mut Rng| (0..5).map(|_| (b'A' + r.rand(26) as u8) as char).collect::<String>();
                        place = format!("{}[MESSAGE INTERCEPTED]{}", junk(&mut r), junk(&mut r));
                    }
                    let text = if place.is_empty() {
                        "has joined from an unknown location?".to_string()
                    } else {
                        let digits: String = cx.terminal.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
                        let hex = format!("{:X}", digits.parse::<u64>().unwrap_or(0));
                        format!("has joined from {place} [{hex}]")
                    };
                    let ch = self.chat.channel.clone();
                    self.act_channel(c, &ch, &text);
                }
            }
            "PART" => {
                crate::tnmpc::sound("TNMGUI.IRCDisconnect");
                if from != self.chat.nick {
                    self.chat.part(c, &from);
                }
            }
            "NICK" => {
                let new = chop(&parse_delimited(line, " ", 3, false));
                self.chat.changed_nick(c, &from, &new);
                if self.link_nick == from {
                    self.link_nick = new;
                }
            }
            "QUIT" => {
                crate::tnmpc::sound("TNMGUI.IRCDisconnect");
                let r = chop_left(&parse_delimited(line, " ", 3, true));
                self.chat.quit(c, &from, &r);
            }
            "353" => {
                let mut names = parse_delimited(line, ":", 3, true);
                while !names.is_empty() {
                    let n = parse_delimited(&names, " ", 1, false);
                    let bare = no_prefix(&n);
                    self.chat.new_user(c, &bare);
                    match n.chars().next() {
                        Some('@') => self.chat.prefix(c, &bare, true, "@"),
                        Some('%') => self.chat.prefix(c, &bare, true, "%"),
                        Some('+') => self.chat.prefix(c, &bare, true, "+"),
                        _ => {}
                    }
                    names = parse_delimited(&names, " ", 2, true);
                }
            }
            "366" | "341" | "329" => {}
            "331" => {
                let ch = self.chat.channel.clone();
                self.send(&format!("MODE {ch}"));
            }
            "332" => self.topic = parse_delimited(line, ":", 3, true),
            "333" => {
                let who = parse_delimited(line, " ", 5, false);
                self.topic_user = match who.find('!') {
                    Some(i) => format!("{} ({})", &who[..i], &who[i + 1..]),
                    None => who,
                };
                let t: u64 = parse_delimited(line, " ", 6, true).trim().parse().unwrap_or(0);
                self.topic_time = stamp_long(t);
                let ch = self.chat.channel.clone();
                self.send(&format!("MODE {ch}"));
            }
            "324" => {
                self.modes = parse_delimited(line, " ", 5, true);
                let (t, u, tt, m) = (self.topic.clone(), self.topic_user.clone(), self.topic_time.clone(), self.modes.clone());
                self.chat.joined_channel(c, &t, &u, &tt, &m);
            }
            "MODE" => {
                let modes = chop(&parse_delimited(line, " ", 4, false));
                let mut i = 5;
                let mut add = true;
                for ch in modes.chars() {
                    match ch {
                        '+' => add = true,
                        '-' => add = false,
                        'o' | 'h' | 'v' => {
                            let who = chop(&parse_delimited(line, " ", i, false));
                            let p = match ch {
                                'o' => "@",
                                'h' => "%",
                                _ => "+",
                            };
                            self.chat.prefix(c, &who, add, p);
                            i += 1;
                        }
                        _ => {}
                    }
                }
                let m = chop_left(&parse_delimited(line, " ", 4, true));
                self.chat.set_mode(c, &from, &m);
            }
            "KICK" => {
                let who = chop(&parse_delimited(line, " ", 4, false));
                let why = chop_left(&parse_delimited(line, ":", 3, true));
                if self.chat.kick(c, &who, &from, &why) {
                    let s = format!("* Attempting to rejoin {}", self.chat.channel);
                    self.chat.msg(c, &s);
                    self.chat.in_channel = false;
                    self.chat.rebuild(c);
                }
            }
            "INVITE" => {
                let s = format!("{from} {} {}", self.link.s("InviteString"), parse_delimited(line, ":", 3, false));
                self.chat.msg(c, &s);
            }
            "TOPIC" => self.chat.user_set_topic(c, &from, &parse_delimited(line, ":", 3, true)),
            "PRIVMSG" => {
                let target = chop(&parse_delimited(line, " ", 3, false));
                let text = chop_left(&parse_delimited(line, " ", 4, true));
                let ctcp = text.len() >= 2 && text.starts_with('\u{1}') && text.ends_with('\u{1}');
                if ctcp && text[1..].starts_with("ACTION ") {
                    let body = &text[8..text.len() - 1];
                    self.chat.action(c, &from, body);
                } else if ctcp {
                    self.ctcp(&from, &text[1..text.len() - 1], cx);
                } else if target.starts_with(['&', '#', '@']) {
                    crate::tnmpc::sound("TNMGUI.Menu_Incoming");
                    self.chat.talk(c, &from, &text);
                } else {
                    crate::tnmpc::sound("TNMGUI.Menu_IncomingFriend");
                    self.chat.private(c, &from, &text);
                }
            }
            n if n.parse::<i64>().is_ok_and(|n| n != 0) => self.chat.msg(c, &chop_left(&parse_delimited(line, " ", 4, true))),
            _ => {}
        }
    }

    /// The CTCP questions the client answers (records, the time, the version, achievements and flags).
    fn ctcp(&mut self, from: &str, q: &str, cx: &Ctx) {
        let (word, arg) = match q.split_once(' ') {
            Some((a, b)) if a.eq_ignore_ascii_case("ACHIEVEMENT") || a.eq_ignore_ascii_case("FLAG") => (a.to_uppercase(), b.to_string()),
            _ => (q.to_string(), String::new()),
        };
        let reply = |body: String| format!("NOTICE {from} :\u{1}{body}\u{1}");
        let out = match word.as_str() {
            "BREAKOUT" => Some(reply(format!("BREAKOUT Breakout high score: {}", crate::tnmgames::high("Breakout")))),
            "TETRIS" => Some(reply(format!("TETRIS Tetris high score: {}", crate::tnmgames::high("Tetris")))),
            "DXPONG" => Some(reply(format!("DXPONG Pong high score: {}", crate::tnmgames::high("Pong")))),
            "LIGHTSOUT" => Some(reply(format!("LIGHTSOUT Lights out puzzles solved: {}", crate::tnmgames::high("LightsOut")))),
            "TIME" => {
                let t = now_secs();
                let (y, ..) = civil(t);
                Some(reply(format!("TIME {} {y}", stamp_long(t))))
            }
            "VERSION" => Some(reply(format!("VERSION {}", self.version))),
            "ACHIEVEMENT" => Some(reply(format!("{} has achievement {arg}? {}", self.link_nick, crate::tnmach::earned(&arg)))),
            "FLAG" => Some(reply(format!("{} has flag {arg} set? {}", self.link_nick, cx.b(&arg)))),
            _ => None,
        };
        if let Some(o) = out {
            self.send(&o);
        }
    }
}

/// The link's thread: resolve, open (20 s), then hand lines in and send lines out until either side goes.
fn irc_thread(host: String, port: u16, tx: mpsc::Sender<NetIn>, rx: mpsc::Receiver<String>, timeout: String, resolve_failed: String, invalid: String) {
    use std::io::{BufRead, BufReader, Write};
    use std::net::ToSocketAddrs;
    if host.is_empty() {
        let _ = tx.send(NetIn::Failed(invalid));
        return;
    }
    let addrs: Vec<std::net::SocketAddr> = match (host.as_str(), port).to_socket_addrs() {
        Ok(a) => a.collect(),
        Err(_) => {
            let _ = tx.send(NetIn::Failed(resolve_failed));
            return;
        }
    };
    let Some(stream) = addrs.iter().find_map(|a| std::net::TcpStream::connect_timeout(a, std::time::Duration::from_secs(20)).ok()) else {
        let _ = tx.send(NetIn::Failed(timeout));
        return;
    };
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(100)));
    let _ = tx.send(NetIn::Opened);
    let mut writer = match stream.try_clone() {
        Ok(w) => w,
        Err(_) => {
            let _ = tx.send(NetIn::Closed);
            return;
        }
    };
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    loop {
        loop {
            match rx.try_recv() {
                Ok(l) => {
                    if writer.write_all(l.as_bytes()).is_err() {
                        let _ = tx.send(NetIn::Closed);
                        return;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    // The window closed: the QUIT went out just before.
                    let _ = writer.flush();
                    return;
                }
            }
        }
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) => {
                let _ = tx.send(NetIn::Closed);
                return;
            }
            Ok(_) => {
                if buf.ends_with(b"\n") {
                    let l = String::from_utf8_lossy(&buf).trim_end_matches(['\r', '\n']).to_string();
                    buf.clear();
                    if tx.send(NetIn::Line(l)).is_err() {
                        return;
                    }
                }
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => {
                let _ = tx.send(NetIn::Closed);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delimited_and_chop() {
        let l = ":nick!user@host PRIVMSG #chan :hello there";
        assert_eq!(parse_delimited(l, " ", 2, false), "PRIVMSG");
        assert_eq!(parse_delimited(l, ":!", 2, false), "nick");
        assert_eq!(parse_delimited(l, ":", 3, true), "hello there");
        assert_eq!(chop("::a b!"), "a b");
        assert_eq!(no_prefix("@+bob"), "bob");
    }

    #[test]
    fn struct_fstrings() {
        let b = [4, b'L', b'A', b'M', 0, 3, b'A', b'B', 0];
        assert_eq!(struct_strings(&b), vec!["LAM".to_string(), "AB".to_string()]);
    }

    /// The scripted chat runs to its end through the loader: every step says something and sets the next timer,
    /// and the ban comes before the last line. TNM_SYS=<folder with TNMGUI.u>.
    #[test]
    fn fake_chat_script_runs() {
        let Ok(dir) = std::env::var("TNM_SYS") else { return };
        unreal_pkg::classes::set_extra_packages(unreal_pkg::tnm::PACKAGES);
        let mut db = unreal_pkg::classes::ClassDb::open(std::path::Path::new(&dir));
        let win = Words { d: db.defaults("tnmFakeIRCWindow").to_vec() };
        let mut script = crate::uscript::Script::default();
        script.objects = true;
        script.add_source(&db.script_text("tnmFakeIRCWindow").unwrap_or_default());
        let mut irca = 0i64;
        let (mut steps, mut said, mut banned_at) = (0, 0, None);
        loop {
            let mut host = FakeHost { irca: &mut irca, win: &win, out: Vec::new(), timer: None, banned: None };
            script.call(&mut host, "ircupdate", &[]);
            let (out, timer, banned) = (std::mem::take(&mut host.out), host.timer, host.banned);
            steps += 1;
            said += out.len();
            if banned == Some(true) {
                banned_at = Some(steps);
            }
            if timer.is_none() || steps > 200 {
                break;
            }
        }
        println!("fake chat: {steps} steps, {said} lines, banned at {banned_at:?}");
        assert!(steps > 40 && said >= steps && banned_at.is_some_and(|b| b < steps));
        let mut iface = crate::uscript::Script::default();
        iface.add_source(&db.script_text("FakeIRCInterface").unwrap_or_default());
        assert!(!v_str(&iface.call(&mut NoHost, "gettopic", &[])).is_empty());
        assert!(!v_str(&iface.call(&mut NoHost, "getchannelmodes", &[])).is_empty());
    }

    /// The command line and the dispenser run against the mod: commands answer, permissions hold, a summons is
    /// asked for once. TNM_ROOT + TNM_DIR as tnmweb's tests. Only counts are printed.
    #[test]
    fn consoles_run() {
        let Some(mut assets) = crate::tnmweb::tests::test_assets() else { return };
        reset_level();
        let flags = imsim_core::flags::FlagBase::default();
        let cx = Ctx { user: "CONTRACTOR".into(), flags: &flags, level: "21_DXI_MCR".into(), terminal: "tnmComputerPersonal3".into() };
        let mut c = Console::new("DXIGAME", &mut assets, 7, &cx).expect("dxig");
        let n0 = c.lines.len();
        for cmd in ["dir", "help", "cd logs", "cd sys", "cd ..", "del cd.com", "cd archived_emails", "dir"] {
            c.parse_raw(cmd, &cx);
        }
        let n1 = c.lines.len();
        println!("dxig: {n0} opening lines, {} after 8 commands", n1);
        assert!(n1 > n0 + 12);
        // As the admin the folder opens.
        let cx2 = Ctx { user: "SRVRADM".into(), flags: &flags, level: "21_DXI_MCR".into(), terminal: String::new() };
        let mut c2 = Console::new("DXIGAME", &mut assets, 7, &cx2).expect("dxig");
        let before = c2.lines.len();
        c2.parse_raw("cd logs", &cx2);
        assert!(c2.lines.last().is_some_and(|l| l.ends_with('>')), "admin reaches logs");
        assert!(c2.lines.len() > before);
        let _ = take_effects();
        TAGS.lock().unwrap().push("DXEDispatchLocation".into());
        let mut d = Console::new("DXEGAME", &mut assets, 7, &cx).expect("dxeg");
        // The mod's own command words.
        let (summon, list) = match &d.prog {
            Prog::Dispense(p) => (p.w.at("command_list", 0), p.w.at("command_list", 1)),
            _ => unreachable!(),
        };
        d.parse_raw(&list, &cx);
        let listed = d.lines.len();
        d.parse_raw(&format!("{summon} 1"), &cx);
        let fx1 = take_effects();
        d.parse_raw(&format!("{summon} 2"), &cx);
        let fx2 = take_effects();
        println!("dxeg: {listed} lines listed, {} effects, then {}", fx1.len(), fx2.len());
        assert!(fx1.iter().any(|e| matches!(e, Effect::Summon { .. })) && fx2.is_empty());
        reset_world();
    }

    #[test]
    fn civil_dates() {
        // 2026-11-23 is a Monday.
        let (y, mo, d, _, _, _, wd) = civil(1_795_392_000);
        assert_eq!((y, mo, d, wd), (2026, 11, 23, 1));
    }
}
