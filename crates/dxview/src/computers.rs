//! Computers and ATMs: the NetworkTerminal windows (ComputerSecurity, ComputerPersonal, ComputerPublic, ATM).
//!
//! Ported from NetworkTerminal(Security/Personal/Public/ATM), Computers, ComputerSecurity, ComputerPublic, ATM,
//! ComputerUIWindow, ComputerScreenLogin / Hack / HackAccounts / Email / Bulletins / SpecialOptions / Security /
//! ATM / ATMWithdraw / ATMDisabled, ComputerSecurityCameraWindow and the ComputerSecurityChoice_* enums, and the
//! MenuUI widgets they're built from (title bar, client textures, action button bar, info / list header /
//! choice buttons, edit fields, lists with scroll bars, the security slider).
//!
//! Rules kept from the scripts:
//! * Frobbing a computer that's locked out says how long is left (lockoutDelay / SkillComputer value); otherwise
//!   the player is made bIgnore (pawns and turrets don't see him; cameras still do) and the terminal comes up.
//!   The game keeps running underneath (InvokeUIScreen with no pause); the HUD hides; a black backdrop covers the
//!   world.
//! * With Computer skill above Untrained the ICE Breaker window is up: hack time = 15 / (skill value x 1.5),
//!   detection = 15 x skill value + hack time; a computer hacked recently starts with only the time since then
//!   left. When detection runs out: "INTRUDER DETECTED" blinks for 3 s, then the player takes 200 - 50 x skill EMP
//!   damage, the terminal locks out and closes, and the computer sounds its alarm (Klaxon2, 30 s, heard by pawns).
//! * Logging in with a user name and password (any case) closes the hack window; a user's access level raises
//!   the terminal's skill level. A successful hack logs in as the first user and keeps the timer running.
//! * Security: three views (camera, turret, door by tag), live camera feeds (render-to-texture), camera on/off,
//!   door lock / open, turret Bypassed / Allies / Enemies / Everything (only at Advanced or better when hacked),
//!   pan / zoom (buttons, arrows, + / -, Shift for faster) at the Pan/Zoom Speed slider's rate, 1-3 pick a view.
//! * Personal: emails "<mission>_EmailMenu_<user>" from the text package, sortable by From / Subject; hacking a
//!   computer with several users opens the account list to read anyone's mail.
//! * Public: bulletins listed in bulletinTag's text.
//! * Special options trigger / untrigger their events once (bTriggerOnceOnly) and are offered first at login.
//! * ATM: account # / PIN (the script's quirk kept: an unknown account checks the PIN of account 0), withdraw
//!   from the balance (synced to the same account on every ATM in the map); hacked, it shows all accounts' sum x
//!   skill level x 0.5 and is sucked dry for good.
//!
//! Text model: each email / bulletin is a `Doc` (name, header fields, body, child docs). The original terminals
//! are flat lists, so `children` stays empty for the original maps; the tree is there so later content can nest.

use crate::actors::Blockers;
use crate::ai::{PlayerHealth, Target};
use crate::alarms::Alarms;
use crate::classic_hud::{ClassicHud, HudLayer};
use crate::devices::GameEvents;
use crate::inventory_screen::{blank, label};
use crate::keybinds::Bindings;
use crate::movers::Movers;
use crate::pickups::PlayerInventory;
use crate::security::{control, Security};
use crate::uicanvas::{wrap, Canvas, HAlign, Style, UiTex, VAlign};
use crate::weapons::RuntimeAssets;
use crate::menuui::*;
use crate::menuui::Rect;
use crate::{LevelRes, Player};
use bevy::camera::{visibility::RenderLayers, Exposure, RenderTarget};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::window::CursorOptions;
use glam::DVec3;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use unreal_pkg::props::{find, Property, Value};
use unreal_pkg::Reader;

const HACK_SIZE: (i32, i32) = (215, 112);
const ACCOUNTS_SIZE: (i32, i32) = (206, 232);
const FEED_SIZE: (i32, i32) = (200, 150);
/// ComputerSecurityCameraWindow positions inside the security client, and the viewport inside each.
const CAM_WINDOWS: [(i32, i32); 3] = [(1, 175), (208, 175), (415, 175)];

// ------------------------------------------------------------------------------------------------ globals

static OPEN: AtomicBool = AtomicBool::new(false);
static IGNORED: AtomicBool = AtomicBool::new(false);
static FROB: Mutex<Option<usize>> = Mutex::new(None);

/// A terminal window is up (the HUD hides, the game gets no input).
pub fn open() -> bool {
    OPEN.load(Ordering::Relaxed)
}

/// The player is logged in to a computer (Computers.ChangePlayerVisibility / MakePlayerIgnored): pawns and
/// turrets ignore him.
pub fn player_ignored() -> bool {
    // (Paralyzed in a cut scene: bDetectable False.)
    IGNORED.load(Ordering::Relaxed) || crate::cinematic::active()
}

pub fn is_terminal(class: &str) -> bool {
    matches!(class, "ComputerSecurity" | "ComputerPersonal" | "ComputerPublic" | "ATM")
}

/// Computers.Frob / ATM.Frob for a level actor (handled next frame).
pub fn frob(actor: usize) {
    if let Ok(mut f) = FROB.lock() {
        *f = Some(actor);
    }
}

/// Camera feed cameras (kept out of the world FOV logic).
#[derive(Component)]
pub struct FeedCam;

#[derive(Component)]
pub struct Backdrop;

// ------------------------------------------------------------------------------------------------ data

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Security,
    Personal,
    Public,
    Atm,
}

#[derive(Clone, Debug, Default)]
struct User {
    name: String,
    password: String,
    access: u8,
}

#[derive(Clone, Debug, Default)]
struct Account {
    number: String,
    pin: String,
    balance: i32,
}

#[derive(Clone, Debug, Default)]
struct View {
    title: String,
    camera: String,
    turret: String,
    door: String,
}

#[derive(Clone, Debug, Default)]
struct SpecialOption {
    text: String,
    trigger_text: String,
    user: String,
    trigger_event: String,
    untrigger_event: String,
    once: bool,
    triggered: bool,
}

#[derive(Clone, Debug, Default)]
struct Node {
    name: String,
    desc: String,
    address: String,
    texture: String,
}

/// One computer's or ATM's state (its properties and what's happened to it).
struct CompData {
    class: String,
    kind: Kind,
    pos: DVec3,
    users: Vec<User>,
    accounts: Vec<Account>,
    views: [View; 3],
    specials: Vec<SpecialOption>,
    node: Node,
    text_package: String,
    bulletin_tag: String,
    lockout_delay: f32,
    msg_locked_out: String,
    locked_out: bool,
    lockout_time: f64,
    last_hack_time: f64,
    /// Computers.BeginAlarm time (0 = quiet).
    last_alarm_time: f64,
    alarm_timeout: f64,
    alarm: Option<usize>,
    /// ATM.bSuckedDryByHack.
    sucked_dry: bool,
}

fn fstr(r: &mut Reader) -> String {
    r.fstring().unwrap_or_default()
}

fn name_at(r: &mut Reader, names: &[String]) -> String {
    let i = r.compact_index().unwrap_or(0);
    match names.get(i.max(0) as usize) {
        Some(n) if n != "None" => n.clone(),
        _ => String::new(),
    }
}

fn structs<'a>(p: &'a [Property], name: &str) -> Vec<(u32, &'a [u8])> {
    p.iter()
        .filter(|x| x.name.eq_ignore_ascii_case(name))
        .filter_map(|x| match &x.value {
            Value::Struct(_, b) => Some((x.index, b.as_slice())),
            _ => None,
        })
        .collect()
}

fn pstr(p: &[Property], n: &str, d: &str) -> String {
    match find(p, n) {
        Some(Value::Str(s)) => s.clone(),
        Some(Value::Name(s)) if s != "None" => s.clone(),
        _ => d.to_string(),
    }
}

fn pfloat(p: &[Property], n: &str, d: f32) -> f32 {
    match find(p, n) {
        Some(Value::Float(v)) => *v,
        Some(Value::Int(v)) => *v as f32,
        Some(Value::Byte(v)) => *v as f32,
        _ => d,
    }
}

impl CompData {
    fn load(lvl: &dx_level::Level, actor: usize, assets: &mut RuntimeAssets) -> CompData {
        let a = &lvl.actors[actor];
        let p = &a.props;
        let kind = match a.class.as_str() {
            "ComputerSecurity" => Kind::Security,
            "ComputerPersonal" => Kind::Personal,
            "ComputerPublic" => Kind::Public,
            _ => Kind::Atm,
        };
        let loc = a.location();
        let mut users = Vec::new();
        let mut accounts = Vec::new();
        // userList[8]: the list stops at the first empty name (NumUsers).
        let mut list = structs(p, "userList");
        list.sort_by_key(|x| x.0);
        for (_, b) in list {
            let mut r = Reader::new(b, 0);
            if kind == Kind::Atm {
                let number = fstr(&mut r);
                let pin = fstr(&mut r);
                let balance = r.i32().unwrap_or(0);
                if number.is_empty() {
                    break;
                }
                accounts.push(Account { number, pin, balance });
            } else {
                let name = fstr(&mut r);
                let password = fstr(&mut r);
                let access = r.u8().unwrap_or(0);
                if name.is_empty() {
                    break;
                }
                users.push(User { name, password, access });
            }
        }
        let mut views: [View; 3] = Default::default();
        for (i, b) in structs(p, "Views") {
            if let Some(v) = views.get_mut(i as usize) {
                let mut r = Reader::new(b, 0);
                v.title = fstr(&mut r);
                v.camera = name_at(&mut r, &lvl.names);
                v.turret = name_at(&mut r, &lvl.names);
                v.door = name_at(&mut r, &lvl.names);
            }
        }
        let mut specials: Vec<SpecialOption> = vec![SpecialOption::default(); 4];
        for (i, b) in structs(p, "specialOptions") {
            if let Some(s) = specials.get_mut(i as usize) {
                let mut r = Reader::new(b, 0);
                s.text = fstr(&mut r);
                s.trigger_text = fstr(&mut r);
                s.user = fstr(&mut r);
                s.trigger_event = name_at(&mut r, &lvl.names);
                s.untrigger_event = name_at(&mut r, &lvl.names);
                s.once = r.u8().unwrap_or(0) != 0;
                s.triggered = r.u8().unwrap_or(0) != 0;
            }
        }
        // NodeInfo[ComputerNode] from the Computers class defaults (texture refs resolve in DeusEx.u).
        let node_i = pfloat(p, "ComputerNode", 0.0) as u32;
        let mut node = Node::default();
        if kind != Kind::Atm {
            let d = assets.classes.defaults("Computers");
            if let Some((_, b)) = structs(&d, "NodeInfo").into_iter().find(|x| x.0 == node_i) {
                let mut r = Reader::new(b, 0);
                node.name = fstr(&mut r);
                node.desc = fstr(&mut r);
                node.address = fstr(&mut r);
                let t = r.compact_index().unwrap_or(0);
                if t != 0 {
                    if let Some(pkg) = assets.classes.packages.get("deusex") {
                        node.texture = pkg.object_path(t);
                    }
                }
            }
        }
        CompData {
            class: a.class.clone(),
            kind,
            pos: DVec3::new(loc[0] as f64, loc[1] as f64, loc[2] as f64),
            users,
            accounts,
            views,
            specials,
            node,
            text_package: pstr(p, "TextPackage", "DeusExText"),
            bulletin_tag: pstr(p, "bulletinTag", ""),
            lockout_delay: pfloat(p, "lockoutDelay", 30.0),
            msg_locked_out: pstr(p, "msgLockedOut", "Terminal is locked out for %d more seconds"),
            locked_out: false,
            lockout_time: 0.0,
            last_hack_time: pfloat(p, "lastHackTime", -9999.0) as f64,
            last_alarm_time: 0.0,
            alarm_timeout: pfloat(p, "alarmTimeout", 30.0) as f64,
            alarm: None,
            sucked_dry: false,
        }
    }

    /// Computers.NumUsers / ATM.NumUsers.
    fn num_users(&self) -> usize {
        if self.kind == Kind::Atm {
            self.accounts.len()
        } else {
            self.users.len()
        }
    }

    fn user_name(&self, i: i32) -> String {
        if i >= 0 {
            if let Some(u) = self.users.get(i as usize) {
                return u.name.clone();
            }
        }
        // Slots past the list are empty strings; out of range is "ERR".
        if (0..8).contains(&i) {
            String::new()
        } else {
            "ERR".into()
        }
    }

    /// ATM.GetBalance: one account, or (-1, hacked) every account's sum times the modifier.
    fn balance(&self, i: i32, modifier: f32) -> i32 {
        if i >= 0 {
            self.accounts.get(i as usize).map_or(0, |a| a.balance)
        } else {
            let sum: i32 = self.accounts.iter().map(|a| a.balance).sum();
            (sum as f32 * modifier) as i32
        }
    }
}

/// ATM.ModBalance with bSync: take credits from the account, and from the same account (number and PIN) on
/// every other ATM in the map; hacked (-1), every account is zeroed.
fn mod_balance(data: &mut HashMap<usize, CompData>, actor: usize, i: i32, n: i32) {
    let Some(d) = data.get_mut(&actor) else { return };
    if i < 0 {
        for a in &mut d.accounts {
            a.balance = 0;
        }
        return;
    }
    let Some(a) = d.accounts.get_mut(i as usize) else { return };
    a.balance -= n;
    let (num, pin) = (a.number.to_uppercase(), a.pin.to_uppercase());
    for (k, other) in data.iter_mut() {
        if *k == actor || other.kind != Kind::Atm {
            continue;
        }
        for acc in &mut other.accounts {
            if acc.number.to_uppercase() == num && acc.pin.to_uppercase() == pin {
                acc.balance -= n;
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------ text

/// A text document from the text package (DeusExTextParser): body text with paragraphs, and the email / file
/// entries it lists.
#[derive(Clone, Debug, Default, PartialEq)]
struct Doc {
    /// Text object name (an email's or file's own text).
    name: String,
    subject: String,
    from: String,
    to: String,
    cc: String,
    /// Nested documents (not used by the original content; kept so terminals can hold folder trees).
    children: Vec<Doc>,
}

#[derive(Debug, Default, PartialEq)]
struct Parsed {
    body: String,
    emails: Vec<Doc>,
    files: Vec<Doc>,
}

/// DeusExTextParser + ComputerUIWindow.ProcessDeusExTextTag: text and player names are kept, <P> starts a new
/// paragraph, <EMAIL=name,subject,from,to,cc> and <FILE=name,desc> are collected (at most 10 each), comments and
/// formatting tags are dropped; line breaks in the source don't count.
fn parse_text(raw: &str) -> Parsed {
    let s = raw.replace(['\r', '\n', '\0'], "");
    let mut out = Parsed::default();
    let mut rest = s.as_str();
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            out.body.push_str(rest);
            break;
        };
        out.body.push_str(&rest[..lt]);
        let after = &rest[lt + 1..];
        let Some(gt) = after.find('>') else {
            out.body.push_str(&rest[lt..]);
            break;
        };
        let tag = &after[..gt];
        rest = &after[gt + 1..];
        let upper = tag.to_ascii_uppercase();
        if upper == "COMMENT" {
            match rest.to_ascii_uppercase().find("</COMMENT>") {
                Some(e) => rest = &rest[e + "</COMMENT>".len()..],
                None => rest = "",
            }
        } else if upper == "P" {
            if !out.body.is_empty() {
                out.body.push('\n');
            }
        } else if upper == "PLAYERNAME" {
            out.body.push_str(&crate::travel::player_name());
        } else if upper == "PLAYERFIRSTNAME" {
            out.body.push_str(crate::travel::player_name().split(' ').next().unwrap_or(""));
        } else if let Some(v) = upper.strip_prefix("EMAIL=").map(|_| &tag[6..]) {
            if out.emails.len() < 10 {
                let f: Vec<&str> = v.splitn(5, ',').collect();
                let g = |i: usize| f.get(i).map(|s| s.trim().to_string()).unwrap_or_default();
                out.emails.push(Doc { name: g(0), subject: g(1), from: g(2), to: g(3), cc: g(4), children: vec![] });
            }
        } else if let Some(v) = upper.strip_prefix("FILE=").map(|_| &tag[5..]) {
            if out.files.len() < 10 {
                let (n, d) = v.split_once(',').unwrap_or((v, ""));
                out.files.push(Doc { name: n.trim().into(), subject: d.trim().into(), ..default() });
            }
        }
    }
    out
}

/// Text packages, opened as needed.
#[derive(Default)]
struct Texts {
    pkgs: HashMap<String, Option<unreal_pkg::Package>>,
}

impl Texts {
    fn get(&mut self, assets: &mut RuntimeAssets, pkg: &str, name: &str) -> Option<Parsed> {
        if name.is_empty() {
            return None;
        }
        let dir = assets.lib.system_dir().to_path_buf();
        let p = self
            .pkgs
            .entry(pkg.to_ascii_lowercase())
            .or_insert_with(|| unreal_pkg::Package::open(dir.join(format!("{pkg}.u"))).ok());
        // Lay D Denton's ComputerUIWindow.ProcessDeusExText: "FemJC" + the name first for her, else the original.
        let fem = crate::travel::female().then(|| p.as_ref().and_then(|p| p.text_named(&format!("FemJC{name}")))).flatten();
        fem.or_else(|| p.as_ref().and_then(|p| p.text_named(name))).map(|t| parse_text(&t))
    }
}

/// Unreal's string-to-int (atoi): leading digits, else 0.
fn atoi(s: &str) -> i32 {
    let t = s.trim_start();
    let (neg, t) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let mut v: i64 = 0;
    for c in t.chars() {
        match c.to_digit(10) {
            Some(d) => v = (v * 10 + d as i64).min(i32::MAX as i64),
            None => break,
        }
    }
    (if neg { -v } else { v }) as i32
}

impl KitWid for Wid {
    fn row(list: u8, i: usize) -> Self {
        Wid::Row(list, i)
    }
    fn scroll_up(id: u8) -> Self {
        Wid::ScrollUp(id)
    }
    fn scroll_down(id: u8) -> Self {
        Wid::ScrollDown(id)
    }
    fn scroll_track(id: u8) -> Self {
        Wid::ScrollTrack(id)
    }
}

// ------------------------------------------------------------------------------------------------ terminal

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Btn {
    Login,
    Cancel,
    Logout,
    Special,
    Return,
    Withdraw,
    Close,
}

/// Widgets the mouse can press.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Wid {
    CompWin,
    Title,
    Btn(Btn),
    Edit(u8),
    Choice(u8),
    ChoiceInfo(u8),
    Pan(u8),
    Slider,
    CamView(u8),
    Header(u8),
    /// (list, row in display order)
    Row(u8, usize),
    ScrollUp(u8),
    ScrollDown(u8),
    ScrollTrack(u8),
    SpecialOpt(usize),
    HackWin,
    HackBtn,
    AcctWin,
    AcctBtn,
}

/// Lists / scroll areas.
const L_LIST: u8 = 0;
const L_BODY: u8 = 1;
const L_ACCOUNTS: u8 = 2;

struct Sec {
    sel: usize,
    /// Choice values: camera (0 on, 1 off), door access (0 locked, 1 unlocked), door (0 open, 1 closed), turret.
    values: [usize; 4],
    fov: [f32; 3],
    /// Pan/Zoom Speed slider tick (0..8 over 1.0..5.0).
    tick: usize,
    turret_disabled: bool,
    special: bool,
}

struct Mail {
    docs: Vec<Doc>,
    /// Display order (indices into docs).
    order: Vec<usize>,
    sel: Option<usize>,
    /// Sort column (0 From, 1 Subject) and each column's reverse flag (bFromSortOrder / bSubjectSortOrder).
    sort: usize,
    rev: [bool; 2],
    body: String,
    center: bool,
    list_scroll: i32,
    body_scroll: i32,
    special: bool,
}

impl Mail {
    fn resort(&mut self) {
        let key = |d: &Doc, c: usize| if c == 0 { d.from.to_lowercase() } else { d.subject.to_lowercase() };
        let (c, rev) = (self.sort, self.rev[self.sort]);
        let docs = &self.docs;
        self.order.sort_by(|a, b| {
            let o = key(&docs[*a], c).cmp(&key(&docs[*b], c));
            if rev {
                o.reverse()
            } else {
                o
            }
        });
    }
}

struct Spec {
    /// (specialOptions index, still selectable)
    opts: Vec<(usize, bool)>,
    info: String,
    ret: Option<Btn>,
}

enum Screen {
    Login { user: Edit, pass: Edit, info: String },
    Security(Sec),
    Email(Mail),
    Bulletins(Mail),
    Special(Spec),
    Atm { acct: Edit, pin: Edit, err: String },
    Withdraw { amount: Edit, info: String, instr: String, modifier: f32, disabled_t: Option<f32> },
    Disabled,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Which {
    Login,
    Security,
    Email,
    Bulletins,
    Special,
    Atm,
    Withdraw,
    Disabled,
}

/// ComputerScreenHack: the ICE Breaker.
struct Hack {
    detection: f32,
    save_detection: f32,
    hack_t: f32,
    save_hack: f32,
    blink: f32,
    digit_t: f32,
    ticking: bool,
    hacking: bool,
    done: bool,
    detected: bool,
    notified: bool,
    msg: String,
    msg_shown: bool,
    red: bool,
    digits: [String; 4],
    btn_return: bool,
    btn_enabled: bool,
    seed: u64,
}

impl Hack {
    fn rand(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.seed >> 40) as f32) / (1u64 << 24) as f32
    }
}

struct Accounts {
    sel: usize,
    scroll: i32,
}

struct Term {
    actor: usize,
    kind: Kind,
    screen: Option<Screen>,
    hack: Option<Hack>,
    accounts: Option<Accounts>,
    hacked: bool,
    no_hack: bool,
    skill: i32,
    user: String,
    user_idx: i32,
    /// Window position once dragged by its title bar (bWindowDragged).
    win_pos: Option<(i32, i32)>,
    drag: Option<(i32, i32)>,
    pressed: Option<Wid>,
    pressed_right: Option<Wid>,
    focus: Option<Wid>,
    hits: Vec<(Wid, Rect, bool)>,
    repeat: f32,
    last_click: (f32, Option<Wid>),
    clock: f32,
    exit: bool,
    /// Where the computer window was drawn (screen UI units) and its size.
    comp_rect: Rect,
}

/// The world a terminal acts on.
struct Wx<'a> {
    data: &'a mut HashMap<usize, CompData>,
    sec: &'a mut Security,
    movers: &'a mut Movers,
    lvl: &'a dx_level::Level,
    inv: &'a mut PlayerInventory,
    ph: &'a mut PlayerHealth,
    events: &'a mut GameEvents,
    hud: &'a mut ClassicHud,
    assets: &'a mut RuntimeAssets,
    texts: &'a mut Texts,
    now: f64,
    mission: u8,
}

fn txt(hud: &mut ClassicHud, assets: &mut RuntimeAssets, class: &str, key: &str, d: &str) -> String {
    hud.str_default(assets, class, key, ).unwrap_or_else(|| d.to_string())
}

fn sound(name: &str) {
    crate::sound::play_player(&format!("DeusExSounds.Menu.{name}"), 0.25, 0.0);
}

impl Term {
    fn d<'b>(&self, wx: &'b mut Wx) -> &'b mut CompData {
        wx.data.get_mut(&self.actor).expect("computer data")
    }

    /// NetworkTerminal.AreSpecialOptionsAvailable.
    fn specials_available(&self, wx: &mut Wx, check_activated: bool) -> bool {
        let user = self.user.clone();
        self.d(wx).specials.iter().any(|s| {
            !s.text.is_empty()
                && (s.user.is_empty() || s.user.to_uppercase() == user)
                && !(check_activated && s.triggered)
        })
    }

    /// CloseHackWindow (the window's DestroyWindow: a detection not yet acted on still shocks).
    fn close_hack(&mut self, wx: &mut Wx) {
        if let Some(h) = self.hack.take() {
            if h.detected && !h.notified {
                self.hack_detected(wx, true);
            }
        }
    }

    /// NetworkTerminal.HackDetected.
    fn hack_detected(&mut self, wx: &mut Wx, damage_only: bool) {
        let now = wx.now;
        let d = self.d(wx);
        d.locked_out = true;
        d.lockout_time = now;
        wx.ph.damage((200 - 50 * self.skill) as f32, "EMP", DVec3::ZERO);
        crate::sound::play_player("DeusExSounds.Weapons.ProdFire", 1.0, 0.0);
        if !damage_only {
            self.close_screen("EXIT", wx);
        }
    }

    /// NetworkTerminal.ComputerHacked.
    fn computer_hacked(&mut self, wx: &mut Wx) {
        self.hacked = true;
        self.user_idx = 0;
        if self.kind != Kind::Atm {
            self.user = self.d(wx).user_name(0);
        }
        self.close_screen("LOGIN", wx);
    }

    /// NetworkTerminal.CloseScreen and the subclasses' versions.
    fn close_screen(&mut self, action: &str, wx: &mut Wx) {
        // ComputerScreenEmail.CloseScreen closes the account list first.
        if matches!(self.screen, Some(Screen::Email(_))) {
            self.accounts = None;
        }
        if self.screen.take().is_some() {
            sound("Menu_OK");
        }
        self.focus = None;
        self.pressed = None;
        if action == "EXIT" {
            self.exit = true;
            return;
        }
        if action == "LOGIN" && self.hack.is_some() && !self.hacked {
            self.close_hack(wx);
            self.no_hack = true;
        }
        let first = match self.kind {
            Kind::Atm => Which::Atm,
            Kind::Public => Which::Bulletins,
            _ => Which::Login,
        };
        let next = match (self.kind, action) {
            (_, "LOGOUT") if self.kind == Kind::Public => {
                self.exit = true;
                None
            }
            (_, "LOGOUT") => {
                if self.hacked {
                    self.exit = true;
                    None
                } else {
                    Some(first)
                }
            }
            (Kind::Security, "LOGIN") => Some(if self.specials_available(wx, true) { Which::Special } else { Which::Security }),
            (Kind::Security, "RETURN") => Some(Which::Security),
            (Kind::Personal, "LOGIN") => Some(if self.specials_available(wx, true) { Which::Special } else { Which::Email }),
            (Kind::Personal, "EMAIL") | (Kind::Personal, "RETURN") => Some(Which::Email),
            (Kind::Security | Kind::Personal, "SPECIAL") => Some(Which::Special),
            (Kind::Atm, "LOGIN") => Some(Which::Withdraw),
            (Kind::Atm, "ATMDISABLED") => Some(Which::Disabled),
            _ => None,
        };
        if let Some(w) = next {
            self.show(w, wx);
        }
    }

    /// NetworkTerminal.ShowScreen: create it (CreateControls), SetNetworkTerminal, SetCompOwner.
    fn show(&mut self, which: Which, wx: &mut Wx) {
        sound("Menu_Activate");
        self.focus = None;
        self.win_pos = None;
        let mut follow: Option<&str> = None;
        let screen = match which {
            Which::Login => {
                // SetHackButtonToReturn.
                if self.hacked {
                    if let Some(h) = self.hack.as_mut() {
                        h.btn_enabled = true;
                        h.btn_return = true;
                    }
                }
                self.focus = Some(Wid::Edit(0));
                let (mut user, mut pass) = (Edit::new(20), Edit::new(20));
                // Ours (Enhancement "Codes From Notes"): an account whose name and password are both in the notes
                // is filled in.
                if let Some(u) = self.d(wx).users.iter().find(|u| crate::menus::in_notes(&u.name) && crate::menus::in_notes(&u.password)) {
                    user.text = u.name.chars().collect();
                    user.cur = user.text.len();
                    user.anchor = user.cur;
                    pass.text = u.password.chars().collect();
                    pass.cur = pass.text.len();
                    pass.anchor = pass.cur;
                }
                Screen::Login { user, pass, info: self.d(wx).node.desc.clone() }
            }
            Which::Atm => {
                if self.hacked {
                    if let Some(h) = self.hack.as_mut() {
                        h.btn_enabled = true;
                        h.btn_return = true;
                    }
                }
                self.focus = Some(Wid::Edit(0));
                if self.d(wx).sucked_dry {
                    follow = Some("ATMDISABLED");
                }
                Screen::Atm { acct: Edit::new(24), pin: Edit::new(24), err: String::new() }
            }
            Which::Withdraw => {
                let modifier = self.skill as f32 * 0.5;
                let instr_t = txt(wx.hud, wx.assets, "ComputerScreenATMWithdraw", "InstructionText", "Account #: %d|nPlease enter the amount of|ncredits you wish to withdraw.");
                let arg = if self.hacked {
                    self.d(wx).sucked_dry = true;
                    txt(wx.hud, wx.assets, "ComputerScreenATMWithdraw", "HackedText", "TERMINAL HACKED")
                } else {
                    let i = self.user_idx;
                    self.d(wx).accounts.get(i.max(0) as usize).map(|a| a.number.to_uppercase()).unwrap_or_default()
                };
                self.focus = Some(Wid::Edit(0));
                Screen::Withdraw {
                    amount: Edit::new(10),
                    info: String::new(),
                    instr: instr_t.replacen("%d", &arg, 1).replacen("%s", &arg, 1),
                    modifier,
                    disabled_t: None,
                }
            }
            Which::Disabled => {
                self.close_hack(wx);
                Screen::Disabled
            }
            Which::Special => {
                let user = self.user.clone();
                let opts = self
                    .d(wx)
                    .specials
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| (s.user.is_empty() || s.user.to_uppercase() == user) && !s.text.is_empty())
                    .map(|(i, s)| (i, !s.triggered))
                    .collect();
                let ret = match self.kind {
                    Kind::Personal | Kind::Security => Some(Btn::Return),
                    _ => None,
                };
                Screen::Special(Spec { opts, info: String::new(), ret })
            }
            Which::Security => {
                let special = self.specials_available(wx, false);
                let turret_disabled = self.skill < 2 && self.hacked;
                let mut s = Sec { sel: 0, values: [0; 4], fov: [90.0; 3], tick: 0, turret_disabled, special };
                sync_choices(&mut s, wx.data.get(&self.actor).unwrap(), wx.sec, wx.lvl, wx.movers);
                Screen::Security(s)
            }
            Which::Email => {
                let special = self.specials_available(wx, false);
                // CreateHackAccountsWindow: hacked, and more than one account.
                if self.hacked && self.accounts.is_none() && self.d(wx).num_users() > 1 {
                    let cur = self.user.to_uppercase();
                    let sel = self.d(wx).users.iter().position(|u| u.name.to_uppercase() == cur).unwrap_or(0);
                    self.accounts = Some(Accounts { sel, scroll: 0 });
                }
                let name = format!("{:02}_EmailMenu_{}", wx.mission, self.user);
                let pkg = self.d(wx).text_package.clone();
                let docs = wx.texts.get(wx.assets, &pkg, &name).map(|p| p.emails).unwrap_or_default();
                let mut m = Mail {
                    order: (0..docs.len()).collect(),
                    docs,
                    sel: None,
                    sort: 0,
                    rev: [true, false],
                    body: String::new(),
                    center: false,
                    list_scroll: 0,
                    body_scroll: 0,
                    special,
                };
                if m.docs.is_empty() {
                    m.body = txt(wx.hud, wx.assets, "ComputerScreenEmail", "NoEmailTodayText", "No Email Today!");
                    m.center = true;
                } else {
                    m.resort();
                    select_doc(&mut m, 0, &pkg, wx);
                }
                self.focus = Some(Wid::Row(L_LIST, 0));
                Screen::Email(m)
            }
            Which::Bulletins => {
                let special = self.specials_available(wx, false);
                let (pkg, tag) = { let d = self.d(wx); (d.text_package.clone(), d.bulletin_tag.clone()) };
                let docs = wx.texts.get(wx.assets, &pkg, &tag).map(|p| p.files).unwrap_or_default();
                let mut m = Mail {
                    order: (0..docs.len()).collect(),
                    docs,
                    sel: None,
                    sort: 0,
                    rev: [false, false],
                    body: String::new(),
                    center: false,
                    list_scroll: 0,
                    body_scroll: 0,
                    special,
                };
                if tag.is_empty() {
                    m.body = txt(wx.hud, wx.assets, "ComputerScreenBulletins", "NoBulletinsTodayText", "No Bulletins Today!");
                    m.center = true;
                } else if !m.docs.is_empty() {
                    select_doc(&mut m, 0, &pkg, wx);
                }
                Screen::Bulletins(m)
            }
        };
        self.screen = Some(screen);
        if let Some(a) = follow {
            self.close_screen(a, wx);
        }
    }

    /// ProcessLogin (computers).
    fn login(&mut self, wx: &mut Wx) {
        let Some(Screen::Login { user, pass, .. }) = self.screen.as_ref() else { return };
        let (u, p) = (user.get().to_uppercase(), pass.get().to_uppercase());
        let d = self.d(wx);
        let idx = d.users.iter().position(|x| x.name.to_uppercase() == u);
        let ok = idx.is_some_and(|i| d.users[i].password.to_uppercase() == p);
        if ok {
            let i = idx.unwrap();
            let access = d.users[i].access as i32;
            self.user = d.users[i].name.to_uppercase();
            self.user_idx = i as i32;
            if self.skill < access {
                self.skill = access;
            }
            self.close_screen("LOGIN", wx);
        } else {
            let msg = txt(wx.hud, wx.assets, "ComputerScreenLogin", "InvalidLoginMessage", "LOGIN ERROR - ACCESS DENIED");
            if let Some(Screen::Login { user, pass, info }) = self.screen.as_mut() {
                *info = msg;
                user.clear();
                pass.clear();
            }
            self.focus = Some(Wid::Edit(0));
        }
    }

    /// ComputerScreenATM.ProcessLogin (userIndex starts at 0, so an unknown account checks account 0's PIN).
    fn atm_login(&mut self, wx: &mut Wx) {
        let Some(Screen::Atm { acct, pin, .. }) = self.screen.as_ref() else { return };
        let (a, p) = (acct.get().to_uppercase(), pin.get().to_uppercase());
        let d = self.d(wx);
        let idx = d.accounts.iter().position(|x| x.number.to_uppercase() == a).unwrap_or(0);
        let ok = d.accounts.get(idx).is_some_and(|x| x.pin.to_uppercase() == p);
        if ok {
            self.user = String::new();
            self.user_idx = idx as i32;
            self.close_screen("LOGIN", wx);
        } else {
            let msg = txt(wx.hud, wx.assets, "ComputerScreenATM", "InvalidLoginMessage", "LOGIN ERROR, ACCESS DENIED");
            if let Some(Screen::Atm { acct, pin, err }) = self.screen.as_mut() {
                *err = msg;
                acct.clear();
                pin.clear();
            }
            self.focus = Some(Wid::Edit(0));
        }
    }

    fn atm_balance(&self, wx: &mut Wx, modifier: f32) -> i32 {
        let i = if self.hacked { -1 } else { self.user_idx };
        self.d(wx).balance(i, modifier)
    }

    /// ComputerScreenATMWithdraw.WithdrawCredits.
    fn withdraw(&mut self, wx: &mut Wx) {
        let Some(Screen::Withdraw { amount, modifier, .. }) = self.screen.as_ref() else { return };
        let n = atoi(&amount.get());
        let modifier = *modifier;
        let msg;
        let mut dry = false;
        if n > 0 {
            let balance = self.atm_balance(wx, modifier);
            if balance >= n {
                let i = if self.hacked { -1 } else { self.user_idx };
                mod_balance(wx.data, self.actor, i, n);
                wx.inv.0.credits += n as u32;
                msg = format!("{} {}", n, txt(wx.hud, wx.assets, "ComputerScreenATMWithdraw", "CreditsWithdrawnLabel", "CREDITS WITHDRAWN"));
                if self.hacked && balance - n <= 0 {
                    dry = true;
                    self.d(wx).sucked_dry = true;
                }
            } else {
                msg = txt(wx.hud, wx.assets, "ComputerScreenATMWithdraw", "InsufficientCreditsLabel", "INSUFFICIENT CREDITS");
            }
        } else {
            msg = txt(wx.hud, wx.assets, "ComputerScreenATMWithdraw", "InvalidAmountLabel", "INVALID AMOUNT ENTERED");
        }
        let delay = wx.hud.defaults(wx.assets, "ComputerScreenATMWithdraw").iter().find(|p| p.name.eq_ignore_ascii_case("disabledDelay")).and_then(|p| match p.value { Value::Float(v) => Some(v), _ => None }).unwrap_or(5.0);
        if let Some(Screen::Withdraw { amount, info, disabled_t, .. }) = self.screen.as_mut() {
            *info = msg;
            amount.clear();
            if dry {
                *disabled_t = Some(delay);
            }
        }
        self.focus = Some(Wid::Edit(0));
    }

    /// ActivateSpecialOption.
    fn special(&mut self, k: usize, wx: &mut Wx) {
        let Some(Screen::Special(sp)) = self.screen.as_mut() else { return };
        let Some(&(si, _)) = sp.opts.get(k) else { return };
        sp.opts[k].1 = false;
        let s = wx.data.get_mut(&self.actor).unwrap().specials[si].clone();
        if s.triggered {
            return;
        }
        if !s.trigger_event.is_empty() {
            wx.events.0.push((s.trigger_event.clone(), true));
        }
        if !s.untrigger_event.is_empty() {
            wx.events.0.push((s.untrigger_event.clone(), false));
        }
        if s.once {
            wx.data.get_mut(&self.actor).unwrap().specials[si].triggered = true;
        }
        if let Some(Screen::Special(sp)) = self.screen.as_mut() {
            sp.info = s.trigger_text.clone();
        }
    }

    /// NetworkTerminal.ChangeAccount -> the email screen reloads.
    fn change_account(&mut self, i: usize, wx: &mut Wx) {
        self.user_idx = i as i32;
        self.user = self.d(wx).user_name(i as i32);
        if let Some(a) = self.accounts.as_mut() {
            a.sel = i;
        }
        if matches!(self.screen, Some(Screen::Email(_))) {
            // ChangeAccount -> CloseScreen("EMAIL"); the account list is recreated by the new screen.
            let keep = self.accounts.take();
            self.close_screen("EMAIL", wx);
            if self.accounts.is_none() {
                self.accounts = keep;
            }
            if let Some(a) = self.accounts.as_mut() {
                a.sel = i;
            }
        }
    }

    fn escape(&mut self, wx: &mut Wx) {
        let action = match &self.screen {
            Some(Screen::Email(_)) | Some(Screen::Security(_)) | Some(Screen::Special(_)) | Some(Screen::Withdraw { .. }) => "LOGOUT",
            _ => "EXIT",
        };
        if self.screen.is_none() {
            self.close_screen("EXIT", wx);
        } else {
            self.close_screen(action, wx);
        }
    }
}

fn select_doc(m: &mut Mail, row: usize, pkg: &str, wx: &mut Wx) {
    let Some(&i) = m.order.get(row) else { return };
    m.sel = Some(row);
    m.body = wx.texts.get(wx.assets, pkg, &m.docs[i].name).map(|p| p.body).unwrap_or_default();
    m.center = false;
    m.body_scroll = 0;
}

/// The mover indices with a tag.
fn doors<'a>(lvl: &'a dx_level::Level, tag: &'a str) -> impl Iterator<Item = usize> + 'a {
    lvl.movers.iter().enumerate().filter(move |(_, m)| !tag.is_empty() && m.tag.eq_ignore_ascii_case(tag)).map(|(i, _)| i)
}

/// SelectCamera -> UpdateActionButtons -> each choice's SetCameraView.
fn sync_choices(s: &mut Sec, d: &CompData, sec: &Security, lvl: &dx_level::Level, movers: &Movers) {
    let v = &d.views[s.sel];
    if let Some(on) = control::camera(sec, &v.camera) {
        s.values[0] = if on { 0 } else { 1 };
    }
    if let Some(rt) = doors(lvl, &v.door).next().and_then(|m| movers.rt.get(m)) {
        s.values[1] = if rt.locked { 0 } else { 1 };
        s.values[2] = if rt.key_num != 0 { 0 } else { 1 };
    }
    if let Some(t) = control::turret(sec, &v.turret) {
        s.values[3] = t as usize;
    }
}

// ------------------------------------------------------------------------------------------------ screens

/// Per-screen frame values (ComputerUIWindow defaults of each screen class).
struct Geo {
    class: &'static str,
    title: String,
    cw: i32,
    ch: i32,
    voff: i32,
    textures: Vec<&'static str>,
    cols: i32,
    /// ComputerUIScaleClientWindow: top, middle, bottom heights and middle count.
    scale: Option<(i32, i32, i32, i32)>,
    status: Option<(i32, i32, i32)>,
    buttons: Vec<(Btn, bool, bool)>,
    left_edge: bool,
}

fn geo(t: &Term, d: &CompData, hud: &mut ClassicHud, assets: &mut RuntimeAssets) -> Geo {
    let mut g = Geo {
        class: "ComputerScreenLogin",
        title: String::new(),
        cw: 343,
        ch: 151,
        voff: 0,
        textures: vec![],
        cols: 2,
        scale: None,
        status: None,
        buttons: vec![],
        left_edge: false,
    };
    match t.screen.as_ref() {
        Some(Screen::Login { user, pass, .. }) => {
            g.title = txt(hud, assets, "ComputerScreenLogin", "Title", "Welcome to %s").replacen("%s", &d.node.name, 1);
            g.textures = vec!["ComputerLogonBackground_1", "ComputerLogonBackground_2"];
            g.status = Some((131, 10, 21));
            g.buttons = vec![(Btn::Cancel, true, true), (Btn::Login, true, !user.text.is_empty() && !pass.text.is_empty())];
        }
        Some(Screen::Email(m)) | Some(Screen::Bulletins(m)) => {
            let email = matches!(t.screen, Some(Screen::Email(_)));
            g.class = if email { "ComputerScreenEmail" } else { "ComputerScreenBulletins" };
            g.title = txt(hud, assets, g.class, "Title", if email { "Email" } else { "Bulletins" });
            g.cw = 395;
            g.ch = 412;
            g.textures = if email {
                vec!["ComputerEmailBackground_1", "ComputerEmailBackground_2", "ComputerEmailBackground_3", "ComputerEmailBackground_4"]
            } else {
                vec!["ComputerBulletinBackground_1", "ComputerBulletinBackground_2", "ComputerBulletinBackground_3", "ComputerBulletinBackground_4"]
            };
            g.status = Some((383, 12, 21));
            g.buttons = vec![(Btn::Logout, true, true)];
            if m.special {
                g.buttons.push((Btn::Special, false, true));
                g.left_edge = true;
            }
        }
        Some(Screen::Special(sp)) => {
            g.class = "ComputerScreenSpecialOptions";
            g.title = txt(hud, assets, g.class, "Title", "Special Options");
            let n = sp.opts.len() as i32;
            g.cw = 331;
            g.ch = 12 + 30 * n + 75;
            g.textures = vec![
                "ComputerSpecialOptionsBackgroundTop_1",
                "ComputerSpecialOptionsBackgroundTop_2",
                "ComputerSpecialOptionsBackgroundMiddle_1",
                "ComputerSpecialOptionsBackgroundMiddle_2",
                "ComputerSpecialOptionsBackgroundBottom_1",
                "ComputerSpecialOptionsBackgroundBottom_2",
            ];
            g.scale = Some((12, 30, 75, n));
            g.status = Some((50 + 12 + 30 * n, 10, 21));
            g.buttons = vec![(Btn::Logout, true, true)];
            if sp.ret.is_some() {
                g.buttons.push((Btn::Return, false, true));
                g.left_edge = true;
            }
        }
        Some(Screen::Security(s)) => {
            g.class = "ComputerScreenSecurity";
            g.title = txt(hud, assets, g.class, "Title", "Surveillance");
            g.cw = 622;
            g.ch = 435;
            g.cols = 3;
            g.textures = (1..=6).map(|i| ["ComputerSecurityBackground_1", "ComputerSecurityBackground_2", "ComputerSecurityBackground_3", "ComputerSecurityBackground_4", "ComputerSecurityBackground_5", "ComputerSecurityBackground_6"][i - 1]).collect();
            g.buttons = vec![(Btn::Logout, true, true)];
            if s.special {
                g.buttons.push((Btn::Special, false, true));
                g.left_edge = true;
            }
        }
        Some(Screen::Atm { acct, pin, .. }) => {
            g.class = "ComputerScreenATM";
            g.title = txt(hud, assets, g.class, "Title", "PageNet Global Banking System");
            g.cw = 303;
            g.ch = 231;
            g.voff = 30;
            g.textures = vec!["ComputerGBSLogonBackground_1", "ComputerGBSLogonBackground_2"];
            g.status = Some((206, 10, 21));
            g.buttons = vec![(Btn::Cancel, true, true), (Btn::Login, true, !acct.text.is_empty() && !pin.text.is_empty())];
        }
        Some(Screen::Withdraw { amount, modifier, .. }) => {
            g.class = "ComputerScreenATMWithdraw";
            g.title = txt(hud, assets, g.class, "Title", "PageNet Global Banking System");
            g.cw = 403;
            g.ch = 211;
            g.voff = 30;
            g.textures = vec!["ComputerGBSWithdrawBackground_1", "ComputerGBSWithdrawBackground_2"];
            g.status = Some((186, 10, 21));
            let i = if t.hacked { -1 } else { t.user_idx };
            let bal = d.balance(i, *modifier);
            g.buttons = vec![(Btn::Withdraw, true, !amount.text.is_empty() && bal > 0), (Btn::Close, true, true)];
        }
        Some(Screen::Disabled) => {
            g.class = "ComputerScreenATMDisabled";
            g.title = txt(hud, assets, g.class, "Title", "PageNet Global Banking System");
            g.cw = 403;
            g.ch = 211;
            g.voff = 30;
            g.textures = vec!["ComputerGBSDisabledBackground_1", "ComputerGBSDisabledBackground_2"];
            g.status = Some((186, 10, 21));
            g.buttons = vec![(Btn::Close, true, true)];
        }
        None => {}
    }
    g
}

fn btn_label(hud: &mut ClassicHud, assets: &mut RuntimeAssets, class: &str, b: Btn, kind: Kind) -> String {
    match b {
        Btn::Login => txt(hud, assets, class, "ButtonLabelLogin", "|&Login"),
        Btn::Cancel => txt(hud, assets, class, "ButtonLabelCancel", "|&Cancel"),
        Btn::Logout => txt(hud, assets, class, "ButtonLabelLogout", "|&Logout"),
        Btn::Special => txt(hud, assets, class, "ButtonLabelSpecial", "|&Special Options"),
        Btn::Return => {
            if kind == Kind::Personal {
                txt(hud, assets, class, "EmailButtonLabel", "|&Email")
            } else {
                txt(hud, assets, class, "SecurityButtonLabel", "|&Security")
            }
        }
        Btn::Withdraw => txt(hud, assets, class, "ButtonLabelWithdraw", "|&Withdraw"),
        Btn::Close => txt(hud, assets, class, "ButtonLabelClose", "|&Close"),
    }
}

/// The computer window's size (ComputerUIWindow.ParentRequestedPreferredSize).
fn comp_size(g: &Geo, f: &Fonts) -> (i32, i32) {
    let tw = (31 + f.title.width(&g.title).min(400) + 45).max(200);
    ((g.cw + TITLE_LEFT_BOTTOM.0).max(tw), g.ch + TITLE_H + BAR_H + g.voff)
}

/// What drawing reads from the world.
struct Dv<'a> {
    data: &'a HashMap<usize, CompData>,
    sec: &'a Security,
    movers: &'a Movers,
    lvl: &'a dx_level::Level,
}

/// Draw the computer window (frame + screen) on its canvas.
fn draw_comp(u: &mut Ui<Wid>, t: &mut Term, wx: &Dv, g: &Geo) {
    let f = u.f;
    let (cw, ch, voff) = (g.cw, g.ch, g.voff);
    let (ww, wh) = comp_size(g, f);
    u.hit(Wid::CompWin, (0, 0, ww, wh), true);
    // ---- title (MenuUITitleWindow)
    let tw = (31 + f.title.width(&g.title).min(400) + 45).max(200);
    let y0 = voff;
    for (set, col) in [("Bar", m_col(3)), ("Bubble", m_col(1))] {
        u.draw(&format!("MenuTitle{set}_Left"), 0, y0, 87, TITLE_H, Style::Masked, col);
        if set == "Bar" {
            u.draw("MenuTitleBar_LeftBottom", 0, y0 + TITLE_H - 1, TITLE_LEFT_BOTTOM.0, TITLE_LEFT_BOTTOM.1, Style::Masked, col);
        }
        if let Some(tx) = u.tex(&format!("MenuTitle{set}_Center")) {
            u.c.texture(87, y0, tw - 187, TITLE_H, 0, 0, &tx, Style::Masked, col);
        }
        u.draw(&format!("MenuTitle{set}_Right"), tw - 100, y0, 100, TITLE_H, Style::Masked, col);
    }
    let ft = f.title.clone();
    u.text(&ft, 31, y0, tw - 31, TITLE_H, HAlign::Left, VAlign::Center, m_col(2), &g.title);
    u.icon("MenuIcon_DeusEx", 12, y0 + 3, Style::Masked, WHITE);
    u.hit(Wid::Title, (0, y0, tw, TITLE_H), true);
    // ---- client textures (MenuUIClientWindow / ComputerUIScaleClientWindow), translucent
    let (cx, cy) = (TITLE_LEFT_BOTTOM.0, TITLE_H + voff);
    u.c.set_clip(Some((cx, cy, cw, ch)));
    match g.scale {
        Some((top, mid, _bot, n)) => {
            let cols = g.cols as usize;
            for col in 0..cols {
                u.icon(g.textures[col], cx + col as i32 * 256, cy, Style::Translucent, m_col(0));
            }
            for k in 0..n {
                for col in 0..cols {
                    u.icon(g.textures[cols + col], cx + col as i32 * 256, cy + top + k * mid, Style::Translucent, m_col(0));
                }
            }
            for col in 0..cols {
                u.icon(g.textures[2 * cols + col], cx + col as i32 * 256, cy + top + n * mid, Style::Translucent, m_col(0));
            }
        }
        None => {
            for (i, name) in g.textures.iter().enumerate() {
                let (col, row) = (i as i32 % g.cols, i as i32 / g.cols);
                u.icon(name, cx + col * 256, cy + row * 256, Style::Translucent, m_col(0));
            }
        }
    }
    u.c.set_clip(None);
    // ---- the screen's own controls (client origin at cx, cy)
    draw_screen(u, t, wx, g, cx, cy);
    // ---- button bar
    let gap = if g.left_edge { 1 } else { 0 };
    let (bx, by, bw) = (cx + gap, TITLE_H + ch + voff, cw - 1);
    let mut left = 0;
    let mut right = bw;
    let kind = t.kind;
    for (b, right_align, enabled) in &g.buttons {
        let l = btn_label(u.hud, u.assets, g.class, *b, kind);
        let w = action_width(f, &l);
        let x = if *right_align {
            right -= w;
            let x = right;
            right -= 2;
            x
        } else {
            let x = left;
            left += w + 2;
            x
        };
        action_button(u, Wid::Btn(*b), bx + x, by + 1, w, &l, *enabled);
    }
    // ---- left edge (MenuUILeftEdgeWindow, with a left-aligned button)
    if g.left_edge {
        let (ex, ey, eh) = (cx - 4, TITLE_H + TITLE_LEFT_BOTTOM.1 + voff, ch + BAR_H - TITLE_LEFT_BOTTOM.1 - 1);
        u.draw("MenuLeftBorder_Top", ex, ey, 4, 2, Style::Masked, m_col(3));
        if let Some(tx) = u.tex("MenuLeftBorder_Center") {
            u.c.texture(ex, ey + 2, 4, eh - 2 - 16, 0, 0, &tx, Style::Masked, m_col(3));
        }
        u.draw("MenuLeftBorder_Bottom", ex, ey + eh - 16, 4, 16, Style::Masked, m_col(3));
    }
    // ---- status line (MenuUIHelpWindow)
    if let Some((sy, sl, sd)) = g.status {
        let d = wx.data.get(&t.actor).unwrap();
        let s = match t.screen {
            Some(Screen::Atm { .. }) | Some(Screen::Withdraw { .. }) | Some(Screen::Disabled) => {
                txt(u.hud, u.assets, g.class, "StatusText", "")
            }
            _ => {
                let func = txt(u.hud, u.assets, g.class, "ComputerNodeFunctionLabel", "");
                format!("Daedalus:GlobalNode:{}/{}", d.node.address, func)
            }
        };
        let fs = f.small.clone();
        u.c.set_clip(Some((cx + sl, TITLE_H + sy + voff, cw - sd, 13)));
        u.text(&fs, cx + sl, TITLE_H + sy + voff, cw - sd, 13, HAlign::Left, VAlign::Center, m_col(7), &s);
        u.c.set_clip(None);
    }
}

fn draw_screen(u: &mut Ui<Wid>, t: &mut Term, wx: &Dv, g: &Geo, dx: i32, dy: i32) {
    let f = u.f;
    let ft = f.title.clone();
    let fsm = f.small.clone();
    let fh = f.headers.clone();
    let focus = t.focus;
    let kind = t.kind;
    let actor = t.actor;
    match t.screen.as_mut() {
        Some(Screen::Login { user, pass, info }) => {
            let ul = txt(u.hud, u.assets, g.class, "UserNameLabel", "User Name");
            let pl = txt(u.hud, u.assets, g.class, "PasswordLabel", "Password");
            u.text(&ft, dx + 10, dy + 22, 90, ft.height, HAlign::Left, VAlign::Top, m_col(8), &ul);
            u.text(&ft, dx + 10, dy + 55, 90, ft.height, HAlign::Left, VAlign::Top, m_col(8), &pl);
            edit_field(u, Wid::Edit(0), dx + 105, dy + 20, 143, user, focus == Some(Wid::Edit(0)), true);
            edit_field(u, Wid::Edit(1), dx + 105, dy + 54, 143, pass, focus == Some(Wid::Edit(1)), true);
            let tex = wx.data.get(&actor).unwrap().node.texture.clone();
            if !tex.is_empty() {
                if let Some(tx) = u.hud.tex(u.assets, &tex) {
                    u.c.texture(dx + 276, dy + 5, 61, 61, 0, 0, &tx, Style::Masked, WHITE);
                }
            }
            u.wrapped(&fsm, dx + 10, dy + 97, 320, 25, HAlign::Left, VAlign::Center, m_col(8), info);
        }
        Some(Screen::Atm { acct, pin, err }) => {
            let al = txt(u.hud, u.assets, g.class, "AccountLabel", "Account #:");
            let pl = txt(u.hud, u.assets, g.class, "PinLabel", "PIN #:");
            let warn = txt(u.hud, u.assets, g.class, "WarningText", "");
            let info = txt(u.hud, u.assets, g.class, "LoginInfoText", "");
            u.text(&ft, dx + 10, dy + 111, 120, ft.height, HAlign::Left, VAlign::Top, m_col(8), &al);
            u.text(&ft, dx + 10, dy + 141, 120, ft.height, HAlign::Left, VAlign::Top, m_col(8), &pl);
            edit_field(u, Wid::Edit(0), dx + 131, dy + 109, 143, acct, focus == Some(Wid::Edit(0)), true);
            edit_field(u, Wid::Edit(1), dx + 131, dy + 139, 143, pin, focus == Some(Wid::Edit(1)), true);
            u.wrapped(&ft, dx + 8, dy + 10, 273, 50, HAlign::Center, VAlign::Center, m_col(8), &warn);
            u.wrapped(&ft, dx + 8, dy + 66, 273, 31, HAlign::Center, VAlign::Center, m_col(8), &info);
            u.wrapped(&fsm, dx + 80, dy + 172, 287, 25, HAlign::Left, VAlign::Center, m_col(8), err);
        }
        Some(Screen::Withdraw { amount, info, instr, modifier, .. }) => {
            let bl = txt(u.hud, u.assets, g.class, "BalanceLabel", "Current Balance:");
            let wl = txt(u.hud, u.assets, g.class, "WithdrawAmountLabel", "Amount to Withdraw:");
            u.text(&ft, dx + 20, dy + 91, 200, ft.height, HAlign::Left, VAlign::Top, m_col(8), &bl);
            u.text(&ft, dx + 20, dy + 121, 200, ft.height, HAlign::Left, VAlign::Top, m_col(8), &wl);
            let i = if t.hacked { -1 } else { t.user_idx };
            let bal = wx.data.get(&actor).unwrap().balance(i, *modifier);
            let mut be = Edit::new(10);
            be.text = bal.to_string().chars().collect();
            edit_field(u, Wid::Edit(9), dx + 231, dy + 89, 143, &mut be, false, false);
            edit_field(u, Wid::Edit(0), dx + 231, dy + 119, 143, amount, focus == Some(Wid::Edit(0)), true);
            u.wrapped(&ft, dx + 8, dy + 10, 381, 50, HAlign::Center, VAlign::Center, m_col(8), instr);
            u.wrapped(&fsm, dx + 8, dy + 152, 385, 25, HAlign::Center, VAlign::Center, m_col(8), info);
        }
        Some(Screen::Disabled) => {
            let info = txt(u.hud, u.assets, g.class, "LoginInfoText", "");
            u.wrapped(&ft, dx + 10, dy + 12, 377, 122, HAlign::Center, VAlign::Center, m_col(8), &info);
        }
        Some(Screen::Special(sp)) => {
            for (k, (si, enabled)) in sp.opts.iter().enumerate() {
                let label_s = wx.data.get(&actor).unwrap().specials[*si].text.clone();
                action_button(u, Wid::SpecialOpt(k), dx + 25, dy + 17 + 30 * k as i32, 273, &label_s, *enabled);
            }
            let n = sp.opts.len() as i32;
            u.wrapped(&fsm, dx + 10, dy + 16 + 12 + 30 * n, 315, 25, HAlign::Left, VAlign::Center, m_col(8), &sp.info);
        }
        Some(Screen::Email(m)) => {
            let hf = txt(u.hud, u.assets, g.class, "HeaderFromLabel", "From");
            let hs = txt(u.hud, u.assets, g.class, "HeaderSubjectLabel", "Subject");
            header_button(u, Wid::Header(0), dx + 10, dy + 3, 121, &hf);
            header_button(u, Wid::Header(1), dx + 134, dy + 3, 187, &hs);
            let rows: Vec<Vec<String>> = m.order.iter().map(|&i| vec![m.docs[i].from.clone(), m.docs[i].subject.clone()]).collect();
            list(u, L_LIST, (dx + 11, dy + 22, 373, 73), &[123, 250], &rows, m.sel, &mut m.list_scroll, false);
            let labels = [
                ("EmailFromHeader", "From:", 12, 104),
                ("EmailToHeader", "To:", 212, 104),
                ("EmailSubjectHeader", "Subj:", 12, 118),
            ];
            for (k, d, x, y) in labels {
                let l = txt(u.hud, u.assets, g.class, k, d);
                u.text(&ft, dx + x, dy + y, 60, ft.height, HAlign::Left, VAlign::Top, m_col(8), &l);
            }
            if let Some(doc) = m.sel.and_then(|r| m.order.get(r)).map(|&i| m.docs[i].clone()) {
                u.text(&fsm, dx + 50, dy + 105, 160, fsm.height, HAlign::Left, VAlign::Top, m_col(8), &doc.from);
                u.text(&fsm, dx + 240, dy + 105, 150, fsm.height, HAlign::Left, VAlign::Top, m_col(8), &doc.to);
                u.text(&fsm, dx + 50, dy + 119, 160, fsm.height, HAlign::Left, VAlign::Top, m_col(8), &doc.subject);
                if !doc.cc.is_empty() {
                    let l = txt(u.hud, u.assets, g.class, "EmailCarbonCopyHeader", "CC:");
                    u.text(&ft, dx + 212, dy + 118, 30, ft.height, HAlign::Left, VAlign::Top, m_col(8), &l);
                    u.text(&fsm, dx + 240, dy + 119, 150, fsm.height, HAlign::Left, VAlign::Top, m_col(8), &doc.cc);
                }
            }
            text_area(u, L_BODY, (dx + 11, dy + 136, 373, 239), &m.body, m.center, &mut m.body_scroll);
        }
        Some(Screen::Bulletins(m)) => {
            let h = txt(u.hud, u.assets, g.class, "BulletinsHeaderText", "Please choose a bulletin to view:");
            u.text(&fh, dx + 11, dy + 6, 300, 12, HAlign::Left, VAlign::Top, m_col(8), &h);
            let rows: Vec<Vec<String>> = m.order.iter().map(|&i| vec![m.docs[i].subject.clone()]).collect();
            list(u, L_LIST, (dx + 11, dy + 22, 373, 113), &[373], &rows, m.sel, &mut m.list_scroll, false);
            text_area(u, L_BODY, (dx + 11, dy + 143, 373, 232), &m.body, m.center, &mut m.body_scroll);
        }
        Some(Screen::Security(sc)) => {
            draw_security(u, sc, wx, actor, dx, dy, g, kind);
        }
        None => {}
    }
}

const CHOICE_KEYS: [(&str, &str, &str); 4] = [
    ("ComputerSecurityChoice_Camera", "|&Camera Status", ""),
    ("ComputerSecurityChoice_DoorAccess", "Door |&Access", ""),
    ("ComputerSecurityChoice_DoorOpen", "|&Door Status", ""),
    ("ComputerSecurityChoice_Turret", "|&Turret Status", ""),
];

fn enum_text(hud: &mut ClassicHud, assets: &mut RuntimeAssets, choice: usize, v: usize) -> String {
    const D: [[&str; 4]; 4] = [["On", "Off", "", ""], ["Locked", "Unlocked", "", ""], ["Open", "Closed", "", ""], ["Bypassed", "Allies", "Enemies", "Everything"]];
    hud.defaults(assets, CHOICE_KEYS[choice].0)
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case("enumText") && p.index as usize == v)
        .and_then(|p| match &p.value {
            Value::Str(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_else(|| D[choice][v.min(3)].to_string())
}

/// Which choices have something to control in the selected view.
fn choice_enabled(sc: &Sec, d: &CompData, sec: &Security, lvl: &dx_level::Level) -> [bool; 4] {
    let v = &d.views[sc.sel];
    let cam = control::camera(sec, &v.camera).is_some();
    let door = doors(lvl, &v.door).next().is_some();
    let turret = control::turret(sec, &v.turret).is_some();
    [cam, door, door, turret && !sc.turret_disabled]
}

#[allow(clippy::too_many_arguments)]
fn draw_security(u: &mut Ui<Wid>, sc: &mut Sec, wx: &Dv, actor: usize, dx: i32, dy: i32, g: &Geo, _kind: Kind) {
    let f = u.f;
    let ft = f.title.clone();
    let fh = f.headers.clone();
    let d = wx.data.get(&actor).unwrap();
    let en = choice_enabled(sc, d, wx.sec, wx.lvl);
    let has_turret = control::turret(wx.sec, &d.views[sc.sel].turret).is_some();
    for i in 0..4 {
        let action = txt(u.hud, u.assets, CHOICE_KEYS[i].0, "actionText", CHOICE_KEYS[i].1);
        let y = dy + 30 + 34 * i as i32;
        let present = if i == 3 { has_turret } else { en[i] };
        action_button(u, Wid::Choice(i as u8), dx + 13, y, 143, &action, en[i]);
        let value = if present { enum_text(u.hud, u.assets, i, sc.values[i]) } else { String::new() };
        info_button(u, dx + 13 + 154, y, 113, &value, if en[i] { m_btn(0) } else { m_btn(3) });
        u.hit(Wid::ChoiceInfo(i as u8), (dx + 167, y, 113, 19), en[i]);
    }
    let ah = txt(u.hud, u.assets, g.class, "ActiveWindowOptionsHeader", "Active Window Options");
    let ch = txt(u.hud, u.assets, g.class, "CameraOptionsHeader", "Camera Options");
    let ph = txt(u.hud, u.assets, g.class, "PanZoomSpeedHeader", "Pan/Zoom Speed");
    let click = txt(u.hud, u.assets, g.class, "ClickCameraWindowText", "Click on a camera view to select that camera.");
    u.text(&ft, dx + 12, dy + 9, 280, ft.height, HAlign::Left, VAlign::Top, m_col(8), &ah);
    u.text(&ft, dx + 298, dy + 9, 200, ft.height, HAlign::Left, VAlign::Top, m_col(8), &ch);
    u.text(&fh, dx + 302, dy + 127, 94, fh.height, HAlign::Center, VAlign::Top, m_col(8), &ph);
    // Pan / zoom buttons (MenuUISpecialButtonWindow, auto-repeat).
    let cam_ok = en[0];
    const PANS: [(&str, i32, i32); 6] = [
        ("PanUp", 335, 58),
        ("PanDown", 335, 98),
        ("PanLeft", 309, 78),
        ("PanRight", 361, 78),
        ("ZoomIn", 323, 31),
        ("ZoomOut", 349, 31),
    ];
    for (k, (n, x, y)) in PANS.iter().enumerate() {
        let w = Wid::Pan(k as u8);
        let set = if u.pressed == Some(w) && u.hover == Some(w) { "Pressed" } else { "Normal" };
        u.draw(&format!("SecurityButton{n}_{set}"), dx + x, dy + y, 25, 19, Style::Masked, m_col(3));
        u.hit(w, (dx + x, dy + y, 25, 19), cam_ok);
    }
    // Pan/Zoom Speed slider: SecuritySliderBar 77 x 21, MenuSlider thumb 9 x 15, ticks between 8 px margins.
    let (sx, sy) = (dx + 309, dy + 146);
    u.draw("SecuritySliderBar", sx, sy, 77, 21, Style::Masked, m_col(3));
    let tx = sx + 8 + (sc.tick as i32 * 61) / 8 - 4;
    u.draw("MenuSlider", tx, sy + 3, 9, 15, Style::Masked, m_col(3));
    u.hit(Wid::Slider, (sx, sy, 77, 21), true);
    u.wrapped(&f.small.clone(), dx + 9, dy + 398, 600, 25, HAlign::Center, VAlign::Center, m_col(8), &click);
    // Camera windows: title, status lines (the feed and the selection border are drawn by other layers).
    let cam_label = txt(u.hud, u.assets, "ComputerSecurityCameraWindow", "CameraLabel", "Camera");
    let no_signal = txt(u.hud, u.assets, "ComputerSecurityCameraWindow", "NoSignalLabel", "NO SIGNAL");
    let lab = |u: &mut Ui<Wid>, k: &str, dd: &str| txt(u.hud, u.assets, "ComputerSecurityCameraWindow", k, dd);
    for j in 0..3 {
        let (wx0, wy0) = (dx + CAM_WINDOWS[j].0, dy + CAM_WINDOWS[j].1);
        let v = d.views[j].clone();
        let cam = control::camera(wx.sec, &v.camera);
        let title = match cam {
            Some(_) => format!("{} {} : {}", cam_label, j + 1, v.title),
            None => no_signal.clone(),
        };
        u.text(&fh, wx0 + 6, wy0 + 2, 196, fh.height, HAlign::Left, VAlign::Top, m_col(8), &title);
        if let Some(on) = cam {
            let s = format!("{} {}", lab(u, "CameraStatusLabel", "Camera:"), if on { lab(u, "OnLabel", "On") } else { lab(u, "OffLabel", "Off") });
            u.text(&fh, wx0 + 6, wy0 + 171, 196, fh.height, HAlign::Left, VAlign::Top, m_col(8), &s);
            // Door status (refreshed every frame here; the script's 0.25 s timer).
            if let Some(rt) = doors(wx.lvl, &v.door).next().and_then(|m| wx.movers.rt.get(m)) {
                if !rt.destroyed {
                    let s = format!(
                        "{} {}, {}",
                        lab(u, "DoorStatusLabel", "Door:"),
                        if rt.key_num != 0 { lab(u, "OpenLabel", "Open") } else { lab(u, "ClosedLabel", "Closed") },
                        if rt.locked { lab(u, "LockedLabel", "Locked") } else { lab(u, "UnlockedLabel", "Unlocked") }
                    );
                    u.text(&fh, wx0 + 6, wy0 + 186, 196, fh.height, HAlign::Left, VAlign::Top, m_col(8), &s);
                }
            }
            if let Some(tm) = control::turret(wx.sec, &v.turret) {
                let k = ["DisabledLabel", "AttackingAlliesLabel", "AttackingEnemiesLabel", "AttackingEverythingLabel"][tm as usize];
                let dd = ["Bypassed", "Attacking Allies", "Attacking Enemies", "Attacking Everything"][tm as usize];
                let s = format!("{} {}", lab(u, "TurretStatusLabel", "Turret:"), lab(u, k, dd));
                u.text(&fh, wx0 + 6, wy0 + 201, 196, fh.height, HAlign::Left, VAlign::Top, m_col(8), &s);
            }
        }
        u.hit(Wid::CamView(j as u8), (wx0, wy0 + 16, 202, 152), true);
    }
}

/// The selection border over the selected camera view (ComputerSecurityCameraViewButton, drawn above the feed).
fn draw_selection(c: &mut Canvas, hud: &mut ClassicHud, assets: &mut RuntimeAssets, x: i32, y: i32) {
    let names = ["TL", "TR", "BL", "BR", "Left", "Right", "Top", "Bottom", "Center"];
    let t: [Option<Arc<UiTex>>; 9] = std::array::from_fn(|i| hud.ui(assets, &format!("PersonaItemHighlight_{}", names[i])));
    // The centre piece would cover the feed: borders only.
    let t2: [Option<Arc<UiTex>>; 9] = std::array::from_fn(|i| if i == 8 { None } else { t[i].clone() });
    c.borders(x, y, 202, 152, &t2, Style::Masked, WHITE);
}

/// ComputerScreenHack.
fn draw_hack(u: &mut Ui<Wid>, h: &Hack) {
    u.hit(Wid::HackWin, (0, 0, HACK_SIZE.0, HACK_SIZE.1), true);
    u.draw("ComputerHackBackground", 14, 13, 187, 94, Style::Translucent, h_col(0));
    u.draw("ComputerHackBorder", 0, 0, 221, 112, Style::Translucent, h_col(1));
    let fx = u.f.fixed.clone();
    let digits: String = h.digits.concat();
    u.c.set_clip(Some((22, 19, 168, 47)));
    let lines = wrap(&fx, &digits, 168);
    for (i, l) in lines.iter().enumerate() {
        u.c.text(&fx, 22, 19 + i as i32 * fx.height, 168, fx.height, HAlign::Left, VAlign::Top, HACK_DIGITS, l);
    }
    u.c.set_clip(None);
    if h.msg_shown {
        if let Some(t) = u.tex("HackInfoBackground") {
            u.c.texture(22, 19, 168, 47, 0, 0, &t, Style::Modulated, WHITE);
        }
        let fh = u.f.headers.clone();
        u.wrapped(&fh, 22, 19, 168, 47, HAlign::Center, VAlign::Center, if h.red { HACK_RED } else { h_col(7) }, &h.msg);
    }
    let pct = (h.detection / h.save_detection.max(1e-3)).clamp(0.0, 1.0);
    let w = (169.0 * pct) as i32;
    u.c.fill(22, 71, w, 12, Style::Normal, crate::uicanvas::color_scaled(pct));
    let label_s = if h.btn_return {
        txt(u.hud, u.assets, "ComputerScreenHack", "ReturnButtonLabel", "|&Return")
    } else {
        txt(u.hud, u.assets, "ComputerScreenHack", "HackButtonLabel", "|&Hack")
    };
    let fh = u.f.headers.clone();
    let bw = (4 + 6 + fh.width(&label(&label_s)) + 8).max(20);
    let st = u.state(Wid::HackBtn, h.btn_enabled);
    crate::inventory_screen::draw_button(&mut u.c, u.hud, u.assets, &fh, 20, 86, bw, &label_s, st, false);
    u.hit(Wid::HackBtn, (20, 86, bw, 16), h.btn_enabled);
}

/// ComputerScreenHackAccounts.
fn draw_accounts(u: &mut Ui<Wid>, a: &mut Accounts, users: &[String], current: &str) {
    u.hit(Wid::AcctWin, (0, 0, ACCOUNTS_SIZE.0, ACCOUNTS_SIZE.1), true);
    u.draw("ComputerHackAccountsBackground", 6, 9, 188, 181, Style::Translucent, h_col(0));
    u.draw("ComputerHackAccountsBorder", 0, 0, 206, 232, Style::Translucent, h_col(1));
    let fh = u.f.headers.clone();
    let cur_h = txt(u.hud, u.assets, "ComputerScreenHackAccounts", "CurrentAccountHeader", "Current User");
    let all_h = txt(u.hud, u.assets, "ComputerScreenHackAccounts", "AllAccountsHeader", "All User Accounts");
    u.text(&fh, 12, 12, 180, fh.height, HAlign::Left, VAlign::Top, m_col(8), &cur_h);
    u.text(&fh, 12, 53, 180, fh.height, HAlign::Left, VAlign::Top, m_col(8), &all_h);
    u.text(&fh, 16, 29, 170, 12, HAlign::Left, VAlign::Center, h_col(7), current);
    let rows: Vec<Vec<String>> = users.iter().map(|n| vec![n.to_uppercase()]).collect();
    list(u, L_ACCOUNTS, (14, 69, 170, 97), &[170], &rows, Some(a.sel), &mut a.scroll, true);
    let l = txt(u.hud, u.assets, "ComputerScreenHackAccounts", "ChangeAccountButtonLabel", "|&Change Account");
    let bw = (4 + 6 + fh.width(&label(&l)) + 8).max(20);
    let st = u.state(Wid::AcctBtn, true);
    crate::inventory_screen::draw_button(&mut u.c, u.hud, u.assets, &fh, 12, 169, bw, &l, st, false);
    u.hit(Wid::AcctBtn, (12, 169, bw, 16), true);
}

// ------------------------------------------------------------------------------------------------ hack tick

/// ComputerScreenHack.Tick (+ NetworkTerminal.UpdateHackDetectionTime before the hack starts).
fn tick_hack(t: &mut Term, wx: &mut Wx, dt: f32, fill: &str) {
    let last = wx.data.get(&t.actor).map_or(-9999.0, |d| d.last_hack_time);
    let hacked = t.hacked;
    let Some(h) = t.hack.as_mut() else { return };
    if !h.hacking && !hacked {
        let diff = (wx.now - last) as f32;
        if diff < h.save_detection {
            h.detection = diff + 0.5;
        }
    }
    if !h.ticking {
        return;
    }
    let mut finish = false;
    let mut notify = false;
    if h.hacking {
        h.hack_t -= dt;
        h.blink -= dt;
        h.digit_t -= dt;
        if h.blink < 0.0 {
            if h.msg.is_empty() {
                h.blink = 1.0;
                h.msg = txt(wx.hud, wx.assets, "ComputerScreenHack", "HackInitializingLabel", "Initializing ICE Breaker...");
                h.msg_shown = true;
            } else {
                h.blink = 1.0 / 3.0;
                h.msg.clear();
            }
        }
        if h.digit_t < 0.0 {
            h.digit_t = 0.05;
            for i in (1..4).rev() {
                h.digits[i] = h.digits[i - 1].clone();
            }
            let chars: Vec<char> = fill.chars().collect();
            let mut s = String::new();
            for _ in 0..23 {
                let space = (h.save_hack - h.hack_t) / h.save_hack > h.rand();
                if space || chars.is_empty() {
                    s.push(' ');
                } else {
                    // Mid(digitFillerChars, Rand(Len) + 1, 1): never the first character, sometimes nothing.
                    let k = (h.rand() * chars.len() as f32) as usize + 1;
                    if let Some(c) = chars.get(k) {
                        s.push(*c);
                    }
                }
            }
            h.digits[0] = s;
        }
        if h.hack_t < 0.0 {
            h.hacking = false;
            finish = true;
        }
    }
    if h.detected {
        h.detection -= dt;
        h.blink -= dt;
        if h.blink < 0.0 {
            if h.msg.is_empty() {
                h.blink = 1.0;
                h.msg = txt(wx.hud, wx.assets, "ComputerScreenHack", "HackDetectedLabel", "*** WARNING ***|nINTRUDER DETECTED!");
            } else {
                h.blink = 1.0 / 3.0;
                h.msg.clear();
            }
        }
        if h.detection < 0.0 && !h.notified {
            h.notified = true;
            notify = true;
        }
    } else {
        h.detection -= dt;
        if h.detection < 0.0 {
            // HackDetected.
            h.detection = 3.0; // hackDetectedDelay
            h.detected = true;
            h.blink = 1.0;
            h.msg = txt(wx.hud, wx.assets, "ComputerScreenHack", "HackDetectedLabel", "*** WARNING ***|nINTRUDER DETECTED!");
            h.msg_shown = true;
            h.red = true;
        }
    }
    if finish {
        // FinishHack.
        if let Some(h) = t.hack.as_mut() {
            h.done = true;
            h.msg = txt(wx.hud, wx.assets, "ComputerScreenHack", "HackSuccessfulLabel", "ICE Breaker Hack Successful...");
            h.msg_shown = true;
            h.digits = Default::default();
        }
        t.computer_hacked(wx);
    }
    if notify {
        t.hack_detected(wx, false);
    }
}

// ------------------------------------------------------------------------------------------------ input

#[allow(clippy::too_many_arguments)]
fn activate(t: &mut Term, w: Wid, right: bool, wx: &mut Wx) {
    match w {
        Wid::Btn(b) => {
            sound("Menu_Press");
            match b {
                Btn::Login => {
                    if t.kind == Kind::Atm {
                        t.atm_login(wx)
                    } else {
                        t.login(wx)
                    }
                }
                Btn::Cancel => t.close_screen("EXIT", wx),
                Btn::Logout => {
                    let a = match t.screen {
                        Some(Screen::Security(_)) | Some(Screen::Special(_)) => "LOGOUT",
                        _ => "EXIT",
                    };
                    t.close_screen(a, wx);
                }
                Btn::Special => t.close_screen("SPECIAL", wx),
                Btn::Return => t.close_screen("RETURN", wx),
                Btn::Withdraw => t.withdraw(wx),
                Btn::Close => {
                    let a = if matches!(t.screen, Some(Screen::Withdraw { .. })) { "LOGOUT" } else { "EXIT" };
                    t.close_screen(a, wx);
                }
            }
        }
        Wid::SpecialOpt(k) => {
            sound("Menu_Press");
            t.special(k, wx);
        }
        Wid::HackBtn => {
            sound("Menu_Press");
            let done = t.hack.as_ref().is_some_and(|h| h.done || t.hacked);
            if done {
                t.computer_hacked(wx);
            } else if let Some(h) = t.hack.as_mut() {
                // StartHack.
                h.hacking = true;
                h.ticking = true;
                h.msg = txt(wx.hud, wx.assets, "ComputerScreenHack", "HackInitializingLabel", "Initializing ICE Breaker...");
                h.msg_shown = true;
            }
            if let Some(h) = t.hack.as_mut() {
                h.btn_enabled = false;
            }
        }
        Wid::AcctBtn => {
            sound("Menu_Press");
            if let Some(a) = t.accounts.as_ref() {
                let i = a.sel;
                t.change_account(i, wx);
            }
        }
        Wid::Header(c) => {
            sound("Menu_Press");
            if let Some(Screen::Email(m)) = t.screen.as_mut() {
                let c = c as usize;
                m.rev[c] = !m.rev[c];
                m.sort = c;
                let cur = m.sel.and_then(|r| m.order.get(r).copied());
                m.resort();
                m.sel = cur.and_then(|i| m.order.iter().position(|&x| x == i));
            }
        }
        Wid::CamView(j) => {
            sound("Menu_Press");
            let actor = t.actor;
            if let Some(Screen::Security(s)) = t.screen.as_mut() {
                s.sel = j as usize;
                let d = wx.data.get(&actor).unwrap();
                sync_choices(s, d, wx.sec, wx.lvl, wx.movers);
            }
        }
        Wid::Choice(i) | Wid::ChoiceInfo(i) => {
            sound("Menu_Press");
            let actor = t.actor;
            let Some(Screen::Security(s)) = t.screen.as_mut() else { return };
            let i = i as usize;
            let n = if i == 3 { 4 } else { 2 };
            // ButtonActivated / ButtonActivatedRight both cycle to the next value first (the subclasses call
            // Super.ButtonActivated either way), then act.
            let _ = right;
            s.values[i] = (s.values[i] + 1) % n;
            let v = wx.data.get(&actor).unwrap().views[s.sel].clone();
            match i {
                0 => {
                    if let Some(on) = control::camera(wx.sec, &v.camera) {
                        control::set_camera(wx.sec, &v.camera, !on);
                    }
                }
                1 => {
                    for m in doors(wx.lvl, &v.door).collect::<Vec<_>>() {
                        wx.movers.rt[m].locked = !wx.movers.rt[m].locked;
                    }
                }
                2 => {
                    // TriggerDoor: Trigger every mover with the door's tag.
                    for m in doors(wx.lvl, &v.door).collect::<Vec<_>>() {
                        crate::movers::trigger(&wx.lvl.movers[m], &mut wx.movers.rt[m]);
                    }
                }
                _ => control::set_turret(wx.sec, &v.turret, s.values[3] as u8),
            }
            let d = wx.data.get(&actor).unwrap();
            sync_choices(s, d, wx.sec, wx.lvl, wx.movers);
            if i == 3 {
                // SetTurretState: the value the player picked stays shown.
            }
        }
        _ => {}
    }
}

/// PanCamera.
fn pan(t: &mut Term, wx: &mut Wx, k: u8, shift: bool) {
    let actor = t.actor;
    let Some(Screen::Security(s)) = t.screen.as_mut() else { return };
    let tag = wx.data.get(&actor).unwrap().views[s.sel].camera.clone();
    let Some(mut rot) = control::camera_desired(wx.sec, &tag) else { return };
    let mut m = 1.0 + s.tick as f32 * 0.5;
    if shift {
        m = (m * 2.0).max(5.0);
    }
    let fov = &mut s.fov[s.sel];
    let step = (m * 256.0 * (*fov / 90.0)) as f64;
    match k {
        0 => rot.0 += step,
        1 => rot.0 -= step,
        2 => rot.1 -= step,
        3 => rot.1 += step,
        4 => *fov -= m * 2.0,
        _ => *fov += m * 2.0,
    }
    control::set_camera_desired(wx.sec, &tag, rot);
    *fov = fov.clamp(5.0, 90.0);
}

// ------------------------------------------------------------------------------------------------ resource

struct Feed {
    img: Handle<Image>,
    cams: [Entity; 2],
    scale: i32,
}

struct Nodes {
    backdrop: Entity,
    comp: Layer,
    overlay: Layer,
    hack: Layer,
    accounts: Layer,
    cursor: Layer,
    feed_nodes: [Entity; 3],
    noise: Handle<Image>,
}

#[derive(Resource, Default)]
pub struct Computers {
    data: HashMap<usize, CompData>,
    term: Option<Term>,
    nodes: Option<Nodes>,
    feeds: [Option<Feed>; 3],
    texts: Texts,
    grabbed_before: bool,
    mission: Option<u8>,
    noise_t: f32,
    seed: u64,
    /// Saved state for terminals not opened yet since the load (applied when their data is first built).
    pending: HashMap<usize, CompSave>,
}

/// What a save keeps of a terminal that has been used (save.rs).
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct CompSave {
    pub actor: usize,
    /// ATM accounts: (number, PIN, balance).
    pub balances: Vec<(String, String, i32)>,
    /// SpecialOptions bAlreadyTriggered, in order.
    pub specials: Vec<bool>,
    pub locked_out: bool,
    pub sucked_dry: bool,
}

impl Computers {
    pub fn save(&self) -> Vec<CompSave> {
        let mut v: Vec<CompSave> = self
            .data
            .iter()
            .map(|(a, d)| CompSave {
                actor: *a,
                balances: d.accounts.iter().map(|x| (x.number.clone(), x.pin.clone(), x.balance)).collect(),
                specials: d.specials.iter().map(|x| x.triggered).collect(),
                // The lockout runs on the level clock, which a load restarts: a terminal locked out when saved
                // stays locked until its next unlock check.
                locked_out: d.locked_out,
                sucked_dry: d.sucked_dry,
            })
            .collect();
        // Terminals restored from the save but not opened since keep their saved state.
        for p in self.pending.values() {
            if !self.data.contains_key(&p.actor) {
                v.push(p.clone());
            }
        }
        v.sort_by_key(|c| c.actor);
        v
    }
    pub fn restore(&mut self, v: &[CompSave]) {
        self.data.clear();
        self.pending = v.iter().map(|c| (c.actor, c.clone())).collect();
    }
}

/// Build a terminal's data on first use: apply what a save kept, and (ATM.ModBalance bSync) take the balance of
/// an account that's already been drawn on at another ATM in the map.
fn first_load(st: &mut Computers, lvl: &dx_level::Level, actor: usize, assets: &mut RuntimeAssets) {
    if st.data.contains_key(&actor) {
        return;
    }
    let mut d = CompData::load(lvl, actor, assets);
    if let Some(p) = st.pending.remove(&actor) {
        for (acc, (_, _, bal)) in d.accounts.iter_mut().zip(&p.balances) {
            acc.balance = *bal;
        }
        for (sp, t) in d.specials.iter_mut().zip(&p.specials) {
            sp.triggered = *t;
        }
        d.locked_out = p.locked_out;
        d.sucked_dry = p.sucked_dry;
    }
    if d.kind == Kind::Atm {
        let known: Vec<(String, String, i32)> = st
            .data
            .values()
            .filter(|o| o.kind == Kind::Atm)
            .flat_map(|o| o.accounts.iter().map(|a| (a.number.to_uppercase(), a.pin.to_uppercase(), a.balance)))
            .chain(st.pending.values().flat_map(|p| {
                p.balances.iter().map(|(n, pin, b)| (n.to_uppercase(), pin.to_uppercase(), *b))
            }))
            .collect();
        for acc in &mut d.accounts {
            if let Some(k) = known.iter().find(|k| k.0 == acc.number.to_uppercase() && k.1 == acc.pin.to_uppercase()) {
                acc.balance = acc.balance.min(k.2);
            }
        }
    }
    st.data.insert(actor, d);
}

fn spawn_feed(commands: &mut Commands, images: &mut Assets<Image>, j: usize, s: i32) -> Feed {
    use bevy::render::render_resource::TextureFormat;
    let (w, h) = ((FEED_SIZE.0 * s) as u32, (FEED_SIZE.1 * s) as u32);
    let img = images.add(Image::new_target_texture(w, h, TextureFormat::Bgra8UnormSrgb, None));
    let proj = || {
        Projection::Perspective(PerspectiveProjection {
            fov: 2.0 * ((45f32.to_radians()).tan() / (FEED_SIZE.0 as f32 / FEED_SIZE.1 as f32)).atan(),
            // The camera's own mesh is all around the view point (and drawn double-sided): clip it.
            near: 24.0 * dx_level::METERS_PER_UNIT,
            far: 4000.0,
            ..default()
        })
    };
    let sky = commands
        .spawn((
            Camera3d::default(),
            Camera { order: -20 + 2 * j as isize, clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() },
            RenderTarget::Image(img.clone().into()),
            proj(),
            Transform::default(),
            RenderLayers::layer(1),
            bevy::light::cluster::ClusterConfig::Single,
            Tonemapping::None,
            Exposure { ev100: -0.263 },
            FeedCam,
        ))
        .id();
    let world = commands
        .spawn((
            Camera3d::default(),
            Camera { order: -19 + 2 * j as isize, clear_color: ClearColorConfig::None, ..default() },
            RenderTarget::Image(img.clone().into()),
            proj(),
            Transform::default(),
            Tonemapping::None,
            Exposure { ev100: -0.263 },
            FeedCam,
        ))
        .id();
    Feed { img, cams: [sky, world], scale: s }
}

fn despawn_feeds(st: &mut Computers, commands: &mut Commands) {
    for f in st.feeds.iter_mut() {
        if let Some(f) = f.take() {
            for c in f.cams {
                commands.entity(c).despawn();
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------ system

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn computer_system(
    mut commands: Commands,
    time: Res<Time>,
    window: Single<&Window>,
    mut cursor_opts: Single<&mut CursorOptions>,
    mut st: ResMut<Computers>,
    mut binds: ResMut<Bindings>,
    mut wheel: MessageReader<MouseWheel>,
    (mut hud, mut assets, mut images, mut layers): (
        ResMut<ClassicHud>,
        ResMut<RuntimeAssets>,
        ResMut<Assets<Image>>,
        ResMut<Assets<HudLayer>>,
    ),
    (level, mut sec, mut movers, mut alarms, mut events, blockers): (
        Res<LevelRes>,
        ResMut<Security>,
        ResMut<Movers>,
        ResMut<Alarms>,
        ResMut<GameEvents>,
        Res<Blockers>,
    ),
    (player, mut inv, mut ph): (Res<Player>, ResMut<PlayerInventory>, ResMut<PlayerHealth>),
    mut layer_nodes: Query<(&mut bevy::ui::Node, &mut Visibility, &MaterialNode<HudLayer>), (Without<ImageNode>, Without<Backdrop>)>,
    mut feed_nodes: Query<(&mut bevy::ui::Node, &mut Visibility, &mut ImageNode), (Without<MaterialNode<HudLayer>>, Without<Backdrop>)>,
    mut backdrop: Query<&mut Visibility, (With<Backdrop>, Without<ImageNode>, Without<MaterialNode<HudLayer>>)>,
    mut feed_cams: Query<(&mut Transform, &mut Projection), With<FeedCam>>,
) {
    let _prof = crate::prof::scope("computers::computer_system");
    let st = &mut *st;
    let lvl = &level.0;
    let dt = time.delta_secs();
    let now = time.elapsed_secs_f64();
    if st.seed == 0 {
        st.seed = 0x9E37_79B9_7F4A_7C15;
    }
    let mission = *st.mission.get_or_insert_with(|| {
        lvl.actors
            .iter()
            .find(|a| a.class == "DeusExLevelInfo")
            .map(|a| pfloat(&a.props, "MissionNumber", 0.0) as u8)
            .unwrap_or(0)
    });
    // ---- UI nodes
    if st.nodes.is_none() {
        let backdrop_e = commands
            .spawn((
                bevy::ui::Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    top: Val::Px(0.0),
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                BackgroundColor(Color::BLACK),
                Visibility::Hidden,
                Pickable::IGNORE,
                GlobalZIndex(48),
                Backdrop,
            ))
            .id();
        let comp = layer(&mut commands, &mut images, &mut layers, 49);
        let mut noise_img = blank(FEED_SIZE.0 as u32, FEED_SIZE.1 as u32);
        noise_img.sampler = bevy::image::ImageSampler::nearest();
        let noise = images.add(noise_img);
        let feed_nodes_e: [Entity; 3] = std::array::from_fn(|_| {
            commands
                .spawn((
                    ImageNode::new(noise.clone()),
                    bevy::ui::Node { position_type: PositionType::Absolute, ..default() },
                    Visibility::Hidden,
                    Pickable::IGNORE,
                    GlobalZIndex(50),
                ))
                .id()
        });
        let overlay = layer(&mut commands, &mut images, &mut layers, 51);
        let hack = layer(&mut commands, &mut images, &mut layers, 52);
        let accounts = layer(&mut commands, &mut images, &mut layers, 52);
        let cursor = layer(&mut commands, &mut images, &mut layers, 53);
        st.nodes = Some(Nodes { backdrop: backdrop_e, comp, overlay, hack, accounts, cursor, feed_nodes: feed_nodes_e, noise });
    }

    // ---- the computers' own ticking: alarms run out (Computers.Tick -> EndAlarm)
    for (actor, d) in st.data.iter_mut() {
        if d.last_alarm_time != 0.0 {
            if now - d.last_alarm_time >= d.alarm_timeout {
                d.last_alarm_time = 0.0;
                crate::sound::release_ambient(*actor);
                if let Some(a) = d.alarm {
                    alarms.set_external(a, false, d.pos, None);
                }
            } else {
                crate::sound::hold(0xC_6000_0000 | *actor as u64, "Ambient.Ambient.Klaxon2", Some(d.pos), 128.0 / 255.0, 25.0 * 65.0, 1.0);
            }
        }
    }

    // ---- frob
    let frobbed = FROB.lock().ok().and_then(|mut f| f.take());
    if let Some(actor) = frobbed {
        if st.term.is_none() && !ph.dead && !crate::inventory_screen::screen_open() && actor < lvl.actors.len() {
            first_load(st, lvl, actor, &mut assets);
            let d = st.data.get_mut(&actor).unwrap();
            let skill_value = crate::abilities::skill("SkillComputer").max(0.01);
            if d.locked_out {
                let delay = d.lockout_delay as f64 / skill_value as f64;
                let elapsed = now - d.lockout_time;
                if elapsed < delay {
                    let msg = d.msg_locked_out.replacen("%d", &((delay - elapsed) as i32).to_string(), 1);
                    crate::classic_hud::client_message(&msg);
                } else {
                    d.locked_out = false;
                }
            }
            if !d.locked_out {
                let kind = d.kind;
                let skill = crate::abilities::skill_level("SkillComputer") as i32;
                let uses_hack = kind != Kind::Public;
                let mut detection = 15.0f32;
                let mut hack = None;
                if skill > 0 && uses_hack {
                    let hack_time = detection / (skill_value * 1.5);
                    detection *= skill_value;
                    let total = detection + hack_time;
                    st.seed = st.seed.wrapping_add(actor as u64 * 7919 + 1);
                    hack = Some(Hack {
                        detection: total,
                        save_detection: total,
                        hack_t: hack_time,
                        save_hack: hack_time,
                        blink: 1.0,
                        digit_t: 0.05,
                        ticking: false,
                        hacking: false,
                        done: false,
                        detected: false,
                        notified: false,
                        msg: txt(&mut hud, &mut assets, "ComputerScreenHack", "HackReadyLabel", "Ice Breaker Ready..."),
                        msg_shown: true,
                        red: false,
                        digits: Default::default(),
                        btn_return: false,
                        btn_enabled: true,
                        seed: st.seed,
                    });
                }
                if kind != Kind::Atm {
                    IGNORED.store(true, Ordering::Relaxed);
                }
                let mut t = Term {
                    actor,
                    kind,
                    screen: None,
                    hack,
                    accounts: None,
                    hacked: false,
                    no_hack: false,
                    skill,
                    user: String::new(),
                    user_idx: 0,
                    win_pos: None,
                    drag: None,
                    pressed: None,
                    pressed_right: None,
                    focus: None,
                    hits: Vec::new(),
                    repeat: 0.0,
                    last_click: (0.0, None),
                    clock: 0.0,
                    exit: false,
                    comp_rect: (0, 0, 0, 0),
                };
                let mut wx = Wx {
                    data: &mut st.data,
                    sec: &mut sec,
                    movers: &mut movers,
                    lvl,
                    inv: &mut inv,
                    ph: &mut ph,
                    events: &mut events,
                    hud: &mut hud,
                    assets: &mut assets,
                    texts: &mut st.texts,
                    now,
                    mission,
                };
                let first = match kind {
                    Kind::Atm => Which::Atm,
                    Kind::Public => Which::Bulletins,
                    _ => Which::Login,
                };
                t.show(first, &mut wx);
                st.term = Some(t);
                st.grabbed_before = cursor_opts.grab_mode != bevy::window::CursorGrabMode::None;
                cursor_opts.grab_mode = bevy::window::CursorGrabMode::None;
                cursor_opts.visible = false;
                println!("COMPUTER open {} ({:?})", lvl.actors[actor].name, kind);
            }
        }
    }

    // ---- an open terminal
    let fill = txt(&mut hud, &mut assets, "ComputerScreenHack", "digitFillerChars", "01234567890ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz!@#$%^&*()_+-=][}{");
    let (pw, phh) = (window.physical_width() as i32, window.physical_height() as i32);
    // (s: canvases' whole-number scale; d: the scale shown, fractional with HUD Scale Large.)
    let (s, d) = crate::hud::screen_scales(&window, 640, 480);
    let (sw, sh) = ((pw as f32 / d) as i32, (phh as f32 / d) as i32);
    let sf = window.scale_factor();
    let mouse = window
        .cursor_position()
        .map(|c| ((c.x * sf / d).floor() as i32, (c.y * sf / d).floor() as i32))
        .unwrap_or((-100, -100));
    let mut close = false;
    if let Some(t) = st.term.as_mut() {
        let mut wx = Wx {
            data: &mut st.data,
            sec: &mut sec,
            movers: &mut movers,
            lvl,
            inv: &mut inv,
            ph: &mut ph,
            events: &mut events,
            hud: &mut hud,
            assets: &mut assets,
            texts: &mut st.texts,
            now,
            mission,
        };
        t.clock += dt;
        // The player died (NetworkTerminal.Tick).
        if wx.ph.dead {
            t.close_screen("EXIT", &mut wx);
        }
        // Walked away is impossible (no input reaches the game); the computer blown up closes it.
        let _ = &blockers;
        // ---- input (hits from the last frame)
        let hover = t.hits.iter().rev().find(|(_, r, _)| crate::inventory_screen::inside(mouse, *r)).map(|(w, _, e)| (*w, *e));
        let hover_w = hover.map(|h| h.0);
        let shift = binds.keys.pressed(KeyCode::ShiftLeft) || binds.keys.pressed(KeyCode::ShiftRight);
        let ctrl = binds.keys.pressed(KeyCode::ControlLeft) || binds.keys.pressed(KeyCode::ControlRight);
        if binds.buttons.just_pressed(MouseButton::Left) {
            t.pressed = None;
            if let Some((w, enabled)) = hover {
                if enabled {
                    match w {
                        Wid::Title => {
                            let (x, y, _, _) = t.comp_rect;
                            t.drag = Some((mouse.0 - x, mouse.1 - y));
                        }
                        Wid::Edit(n) => {
                            t.focus = Some(w);
                            let e = edit_mut(t, n);
                            if let Some(e) = e {
                                let font = wx.hud.font(wx.assets, "FontMenuSmall");
                                if let Some(font) = font {
                                    let rel = mouse.0 - e.org;
                                    let mut i = 0;
                                    while i < e.text.len() {
                                        let wch = font.width(&e.text[..=i].iter().collect::<String>());
                                        let wprev = font.width(&e.text[..i].iter().collect::<String>());
                                        if rel < (wch + wprev) / 2 {
                                            break;
                                        }
                                        i += 1;
                                    }
                                    e.cur = i;
                                    if !shift {
                                        e.anchor = i;
                                    }
                                }
                            }
                        }
                        Wid::Row(l, r) => {
                            let double = t.last_click.1 == Some(w) && t.clock - t.last_click.0 < 0.4;
                            t.last_click = (t.clock, Some(w));
                            sound("Menu_Press");
                            if l == L_ACCOUNTS {
                                if let Some(a) = t.accounts.as_mut() {
                                    a.sel = r;
                                }
                                if double {
                                    t.change_account(r, &mut wx);
                                }
                            } else {
                                let pkg = wx.data.get(&t.actor).unwrap().text_package.clone();
                                match t.screen.as_mut() {
                                    Some(Screen::Email(m)) | Some(Screen::Bulletins(m)) => {
                                        if m.sel != Some(r) {
                                            select_doc(m, r, &pkg, &mut wx);
                                        }
                                    }
                                    _ => {}
                                }
                                t.focus = Some(Wid::Row(L_LIST, r));
                            }
                        }
                        Wid::ScrollUp(_) | Wid::ScrollDown(_) => {
                            scroll(t, w, ROW_H);
                            t.repeat = 0.35;
                            sound("Menu_Press");
                        }
                        Wid::Pan(k) => {
                            pan(t, &mut wx, k, shift);
                            t.repeat = 0.35;
                            sound("Menu_Press");
                        }
                        Wid::Slider => sound("Menu_Press"),
                        Wid::CompWin | Wid::HackWin | Wid::AcctWin => {}
                        _ => {
                            if !matches!(w, Wid::ScrollTrack(_)) {
                                t.focus = Some(w);
                            }
                        }
                    }
                    t.pressed = Some(w);
                }
            }
        }
        if binds.buttons.pressed(MouseButton::Left) {
            if let Some((ox, oy)) = t.drag {
                t.win_pos = Some((mouse.0 - ox, mouse.1 - oy));
            }
            match t.pressed {
                Some(w @ (Wid::ScrollUp(_) | Wid::ScrollDown(_))) if hover_w == Some(w) => {
                    t.repeat -= dt;
                    if t.repeat <= 0.0 {
                        t.repeat = 0.05;
                        scroll(t, w, ROW_H);
                    }
                }
                Some(Wid::Pan(k)) if hover_w == Some(Wid::Pan(k)) => {
                    t.repeat -= dt;
                    if t.repeat <= 0.0 {
                        t.repeat = 0.05;
                        pan(t, &mut wx, k, shift);
                    }
                }
                Some(Wid::Slider) => {
                    if let Some((_, r, _)) = t.hits.iter().find(|h| h.0 == Wid::Slider) {
                        let rel = (mouse.0 - r.0 - 8) as f32 / 61.0;
                        if let Some(Screen::Security(sc)) = t.screen.as_mut() {
                            let tick = (rel * 8.0).round().clamp(0.0, 8.0) as usize;
                            if tick != sc.tick {
                                sc.tick = tick;
                                crate::sound::play_player("DeusExSounds.UserInterface.Menu_Slider", 0.25, 0.0);
                            }
                        }
                    }
                }
                Some(Wid::ScrollTrack(id)) if id < 100 => {
                    if let Some((_, r, _)) = t.hits.iter().find(|h| h.0 == Wid::ScrollTrack(id)) {
                        let frac = ((mouse.1 - r.1) as f32 / r.3.max(1) as f32).clamp(0.0, 1.0);
                        set_scroll_frac(t, id, frac);
                    }
                }
                _ => {}
            }
        }
        if binds.buttons.just_released(MouseButton::Left) {
            t.drag = None;
            if let Some(w) = t.pressed.take() {
                let on_release = matches!(
                    w,
                    Wid::Btn(_) | Wid::SpecialOpt(_) | Wid::HackBtn | Wid::AcctBtn | Wid::Header(_) | Wid::CamView(_) | Wid::Choice(_) | Wid::ChoiceInfo(_)
                );
                if on_release && hover_w == Some(w) {
                    activate(t, w, false, &mut wx);
                }
            }
        }
        // Right click on a choice cycles too (ButtonActivatedRight).
        if binds.buttons.just_pressed(MouseButton::Right) {
            t.pressed_right = hover.filter(|h| h.1 && matches!(h.0, Wid::Choice(_))).map(|h| h.0);
        }
        if binds.buttons.just_released(MouseButton::Right) {
            if let Some(w) = t.pressed_right.take() {
                if hover_w == Some(w) {
                    activate(t, w, true, &mut wx);
                }
            }
        }
        // Wheel: scroll what's under the mouse.
        let lines: f32 = wheel
            .read()
            .map(|w| if w.unit == bevy::input::mouse::MouseScrollUnit::Line { w.y } else { w.y / 40.0 })
            .sum();
        if lines != 0.0 {
            let id = match hover_w {
                Some(Wid::Row(l, _)) => Some(l),
                Some(Wid::ScrollTrack(i)) => Some(if i >= 100 { i - 100 } else { i }),
                Some(Wid::ScrollUp(i)) | Some(Wid::ScrollDown(i)) => Some(i),
                _ => None,
            };
            if let Some(id) = id {
                let px = (-lines * 3.0 * ROW_H as f32) as i32;
                add_scroll(t, id, px);
            }
        }
        // Keys.
        let typed = binds.typed.clone();
        let edit_focus = match t.focus {
            Some(Wid::Edit(n)) => Some(n),
            _ => None,
        };
        let mut handled_escape = false;
        for (k, text) in &typed {
            if t.screen.is_none() || t.exit {
                break;
            }
            match k {
                KeyCode::Escape => {
                    if !handled_escape {
                        handled_escape = true;
                        t.escape(&mut wx);
                    }
                }
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    // EditActivated: [Return] logs in / withdraws.
                    match t.screen {
                        Some(Screen::Login { .. }) | Some(Screen::Atm { .. }) => {
                            let ok = match &t.screen {
                                Some(Screen::Login { user, pass, .. }) => !user.text.is_empty() && !pass.text.is_empty(),
                                Some(Screen::Atm { acct, pin, .. }) => !acct.text.is_empty() && !pin.text.is_empty(),
                                _ => false,
                            };
                            if ok && edit_focus.is_some() {
                                activate(t, Wid::Btn(Btn::Login), false, &mut wx);
                            }
                        }
                        Some(Screen::Withdraw { .. }) if edit_focus.is_some() => t.withdraw(&mut wx),
                        _ => {}
                    }
                }
                KeyCode::Tab => {
                    if let Some(n) = edit_focus {
                        let other = if n == 0 { 1 } else { 0 };
                        if matches!(t.screen, Some(Screen::Login { .. }) | Some(Screen::Atm { .. })) {
                            t.focus = Some(Wid::Edit(other));
                        }
                    }
                }
                _ => {
                    if let Some(n) = edit_focus {
                        if let Some(e) = edit_mut(t, n) {
                            e.key(*k, text.as_deref(), shift, ctrl);
                        }
                        continue;
                    }
                    // Security keys (ComputerScreenSecurity.VirtualKeyPressed).
                    if matches!(t.screen, Some(Screen::Security(_))) {
                        let p = match k {
                            KeyCode::ArrowUp => Some(0),
                            KeyCode::ArrowDown => Some(1),
                            KeyCode::ArrowLeft => Some(2),
                            KeyCode::ArrowRight => Some(3),
                            KeyCode::NumpadAdd | KeyCode::Equal => Some(4),
                            KeyCode::NumpadSubtract | KeyCode::Minus => Some(5),
                            _ => None,
                        };
                        if let Some(p) = p {
                            let actor = t.actor;
                            let ok = matches!(&t.screen, Some(Screen::Security(sc)) if control::camera(wx.sec, &wx.data.get(&actor).unwrap().views[sc.sel].camera).is_some());
                            if ok {
                                pan(t, &mut wx, p, shift);
                            }
                        }
                        let v = match k {
                            KeyCode::Digit1 => Some(0),
                            KeyCode::Digit2 => Some(1),
                            KeyCode::Digit3 => Some(2),
                            _ => None,
                        };
                        if let Some(v) = v {
                            activate(t, Wid::CamView(v), false, &mut wx);
                        }
                    }
                    // Lists: up / down move the selection.
                    if matches!(k, KeyCode::ArrowUp | KeyCode::ArrowDown) {
                        let up = *k == KeyCode::ArrowUp;
                        let pkg = wx.data.get(&t.actor).unwrap().text_package.clone();
                        if let Some(Screen::Email(m)) | Some(Screen::Bulletins(m)) = t.screen.as_mut() {
                            if !m.order.is_empty() {
                                let cur = m.sel.unwrap_or(0);
                                let next = if up { cur.saturating_sub(1) } else { (cur + 1).min(m.order.len() - 1) };
                                if Some(next) != m.sel {
                                    select_doc(m, next, &pkg, &mut wx);
                                    let top = next as i32 * ROW_H;
                                    if top < m.list_scroll {
                                        m.list_scroll = top;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // ---- ticking
        tick_hack(t, &mut wx, dt, &fill);
        let mut go_disabled = false;
        if let Some(Screen::Withdraw { disabled_t: Some(dtl), .. }) = t.screen.as_mut() {
            *dtl -= dt;
            if *dtl <= 0.0 {
                go_disabled = true;
            }
        }
        if go_disabled {
            t.close_screen("ATMDISABLED", &mut wx);
        }
        if t.exit {
            close = true;
        }
    }

    // ---- close (NetworkTerminal.DestroyWindow, Computers state Off)
    if close {
        if let Some(mut t) = st.term.take() {
            let mut wx = Wx {
                data: &mut st.data,
                sec: &mut sec,
                movers: &mut movers,
                lvl,
                inv: &mut inv,
                ph: &mut ph,
                events: &mut events,
                hud: &mut hud,
                assets: &mut assets,
                texts: &mut st.texts,
                now,
                mission,
            };
            t.close_hack(&mut wx);
            let d = st.data.get_mut(&t.actor).unwrap();
            if t.hacked {
                d.last_hack_time = now;
            }
            if d.kind != Kind::Atm {
                IGNORED.store(false, Ordering::Relaxed);
                // State Off: a locked-out computer sounds its alarm.
                if d.locked_out {
                    d.last_alarm_time = now;
                    crate::sound::own_ambient(t.actor);
                    let a = *d.alarm.get_or_insert_with(|| alarms.add_external(t.actor, d.pos));
                    alarms.set_external(a, true, d.pos, Some(Target::Player));
                }
            }
            println!("COMPUTER closed (hacked {}, locked out {})", t.hacked, d.locked_out);
        }
        despawn_feeds(st, &mut commands);
        cursor_opts.visible = !st.grabbed_before;
        if st.grabbed_before {
            cursor_opts.grab_mode = bevy::window::CursorGrabMode::Locked;
        }
    }

    let is_open = st.term.is_some();
    OPEN.store(is_open, Ordering::Relaxed);
    if is_open {
        binds.capture = true;
    }
    let nodes = st.nodes.as_mut().unwrap();
    if let Ok(mut v) = backdrop.get_mut(nodes.backdrop) {
        *v = if is_open { Visibility::Inherited } else { Visibility::Hidden };
    }
    if !is_open {
        for e in [nodes.comp.e, nodes.overlay.e, nodes.hack.e, nodes.accounts.e, nodes.cursor.e] {
            if let Ok((_, mut v, _)) = layer_nodes.get_mut(e) {
                if *v != Visibility::Hidden {
                    *v = Visibility::Hidden;
                }
            }
        }
        for e in nodes.feed_nodes {
            if let Ok((_, mut v, _)) = feed_nodes.get_mut(e) {
                *v = Visibility::Hidden;
            }
        }
        if !close {
            wheel.clear();
        }
        return;
    }

    // ---- draw
    let fonts = {
        let mut get = |n: &str| hud.font(&mut assets, n);
        let (Some(title), Some(small), Some(headers), Some(fixed)) =
            (get("FontMenuTitle"), get("FontMenuSmall"), get("FontMenuHeaders"), get("FontFixedWidthSmall"))
        else {
            return;
        };
        Fonts { title, small, headers, fixed }
    };
    let t = st.term.as_mut().unwrap();
    let hover_now = t.hits.iter().rev().find(|(_, r, _)| crate::inventory_screen::inside(mouse, *r)).map(|(w, _, _)| *w);
    let mut hits: Vec<(Wid, Rect, bool)> = Vec::new();
    let g = geo(t, st.data.get(&t.actor).unwrap(), &mut hud, &mut assets);
    let (cw, ch) = comp_size(&g, &fonts);
    // NetworkTerminal.ConfigurationChanged: centred, or left of the hack window if it would overlap it.
    let hack_h = if t.hack.is_some() { HACK_SIZE.1 } else { 0 };
    let acct_h = if t.accounts.is_some() { ACCOUNTS_SIZE.1 } else { 0 };
    let always_center = matches!(
        t.screen,
        Some(Screen::Security(_)) | Some(Screen::Special(_)) | Some(Screen::Atm { .. }) | Some(Screen::Withdraw { .. }) | Some(Screen::Disabled)
    );
    let hack_w = if t.hack.is_some() { HACK_SIZE.0 } else { 0 };
    let pos = t.win_pos.unwrap_or_else(|| {
        let x = if hack_h + acct_h + 20 > sh / 2 - ch / 2 && !always_center { (sw - hack_w) / 2 - cw / 2 } else { sw / 2 - cw / 2 };
        (x, sh / 2 - ch / 2)
    });
    t.comp_rect = (pos.0, pos.1, cw, ch);
    let (pressed, focus) = (t.pressed, t.focus);
    let dv = Dv { data: &st.data, sec: &sec, movers: &movers, lvl };
    let nodes = st.nodes.as_mut().unwrap();
    // Computer window.
    let mut sec_sel: Option<(usize, [f32; 3])> = None;
    if t.screen.is_some() {
        let mut ui = Ui { c: Canvas::new(cw, ch, s), hud: &mut hud, assets: &mut assets, f: &fonts, o: pos, hits: &mut hits, pressed, hover: hover_now, focus };
        draw_comp(&mut ui, t, &dv, &g);
        let Ui { c, .. } = ui;
        place(&mut nodes.comp, &c, pos, d, sf, &mut images, &mut layers, &mut layer_nodes);
        if let Some(Screen::Security(sc)) = t.screen.as_ref() {
            sec_sel = Some((sc.sel, sc.fov));
        }
    } else if let Ok((_, mut v, _)) = layer_nodes.get_mut(nodes.comp.e) {
        *v = Visibility::Hidden;
    }
    // Camera feeds and the selection border above them.
    let client = (pos.0 + TITLE_LEFT_BOTTOM.0, pos.1 + TITLE_H + g.voff);
    match sec_sel {
        Some((sel, fov)) => {
            let mut oc = Canvas::new(cw, ch, s);
            let (bx, by) = (TITLE_LEFT_BOTTOM.0 + CAM_WINDOWS[sel].0, TITLE_H + g.voff + CAM_WINDOWS[sel].1 + 16);
            draw_selection(&mut oc, &mut hud, &mut assets, bx, by);
            place(&mut nodes.overlay, &oc, pos, d, sf, &mut images, &mut layers, &mut layer_nodes);
            st.noise_t -= dt;
            if st.noise_t <= 0.0 {
                st.noise_t = 0.05;
                if let Some(mut img) = images.get_mut(&nodes.noise) {
                    let n = (FEED_SIZE.0 * FEED_SIZE.1) as usize;
                    let mut data = Vec::with_capacity(n * 4);
                    for _ in 0..n {
                        st.seed = st.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                        let v = (st.seed >> 56) as u8;
                        data.extend_from_slice(&[v, v, v, 255]);
                    }
                    img.data = Some(data);
                }
            }
            let views = st.data.get(&t.actor).unwrap().views.clone();
            for j in 0..3 {
                let tag = &views[j].camera;
                let view = control::camera_view(&sec, tag);
                let img = match view {
                    Some((cp, pitch, yaw)) => {
                        if st.feeds[j].as_ref().is_some_and(|f| f.scale != s) {
                            if let Some(f) = st.feeds[j].take() {
                                for c in f.cams {
                                    commands.entity(c).despawn();
                                }
                            }
                        }
                        let f = st.feeds[j].get_or_insert_with(|| spawn_feed(&mut commands, &mut images, j, s));
                        let to_rad = |u: f64| (u / 65536.0 * std::f64::consts::TAU) as f32;
                        let rot = Quat::from_rotation_y(-to_rad(yaw) - std::f32::consts::FRAC_PI_2) * Quat::from_rotation_x(to_rad(pitch));
                        let eye = Vec3::from(dx_level::ue_to_bevy([cp.x as f32, cp.y as f32, cp.z as f32]));
                        let vfov = 2.0 * ((fov[j].to_radians() / 2.0).tan() / (FEED_SIZE.0 as f32 / FEED_SIZE.1 as f32)).atan();
                        for (k, c) in f.cams.iter().enumerate() {
                            if let Ok((mut tf, mut pr)) = feed_cams.get_mut(*c) {
                                tf.rotation = rot;
                                tf.translation = if k == 0 { Vec3::ZERO } else { eye };
                                if let Projection::Perspective(pp) = &mut *pr {
                                    pp.fov = vfov;
                                }
                            }
                        }
                        f.img.clone()
                    }
                    None => nodes.noise.clone(),
                };
                if let Ok((mut n, mut v, mut im)) = feed_nodes.get_mut(nodes.feed_nodes[j]) {
                    *v = Visibility::Inherited;
                    if im.image != img {
                        im.image = img;
                    }
                    let (x, y) = (client.0 + CAM_WINDOWS[j].0 + 1, client.1 + CAM_WINDOWS[j].1 + 17);
                    n.left = Val::Px(x as f32 * d / sf);
                    n.top = Val::Px(y as f32 * d / sf);
                    n.width = Val::Px(FEED_SIZE.0 as f32 * d / sf);
                    n.height = Val::Px(FEED_SIZE.1 as f32 * d / sf);
                }
            }
        }
        None => {
            if let Ok((_, mut v, _)) = layer_nodes.get_mut(nodes.overlay.e) {
                *v = Visibility::Hidden;
            }
            for e in nodes.feed_nodes {
                if let Ok((_, mut v, _)) = feed_nodes.get_mut(e) {
                    *v = Visibility::Hidden;
                }
            }
            if st.feeds.iter().any(|f| f.is_some()) {
                for f in st.feeds.iter_mut() {
                    if let Some(f) = f.take() {
                        for c in f.cams {
                            commands.entity(c).despawn();
                        }
                    }
                }
            }
        }
    }
    // ICE Breaker window (top right) and the account list under it.
    match t.hack.as_ref() {
        Some(h) => {
            let hp = (sw - HACK_SIZE.0, 0);
            let mut ui = Ui { c: Canvas::new(HACK_SIZE.0, HACK_SIZE.1, s), hud: &mut hud, assets: &mut assets, f: &fonts, o: hp, hits: &mut hits, pressed, hover: hover_now, focus };
            draw_hack(&mut ui, h);
            let Ui { c, .. } = ui;
            place(&mut nodes.hack, &c, hp, d, sf, &mut images, &mut layers, &mut layer_nodes);
        }
        None => {
            if let Ok((_, mut v, _)) = layer_nodes.get_mut(nodes.hack.e) {
                *v = Visibility::Hidden;
            }
        }
    }
    let users: Vec<String> = st.data.get(&t.actor).unwrap().users.iter().map(|u| u.name.clone()).collect();
    let current = t.user.clone();
    match t.accounts.as_mut() {
        Some(a) => {
            let ap = (sw - ACCOUNTS_SIZE.0, hack_h + 20);
            let mut ui = Ui { c: Canvas::new(ACCOUNTS_SIZE.0, ACCOUNTS_SIZE.1, s), hud: &mut hud, assets: &mut assets, f: &fonts, o: ap, hits: &mut hits, pressed, hover: hover_now, focus };
            draw_accounts(&mut ui, a, &users, &current);
            let Ui { c, .. } = ui;
            place(&mut nodes.accounts, &c, ap, d, sf, &mut images, &mut layers, &mut layer_nodes);
        }
        None => {
            if let Ok((_, mut v, _)) = layer_nodes.get_mut(nodes.accounts.e) {
                *v = Visibility::Hidden;
            }
        }
    }
    t.hits = hits;
    crate::snap::offer(
        crate::snap::LAYER_COMPUTER,
        crate::snap::Map { s: d, ox: 0, oy: 0, sf: window.scale_factor() },
        t.hits
            .iter()
            .filter(|h| h.2 && !matches!(h.0, Wid::CompWin | Wid::Title | Wid::HackWin | Wid::AcctWin | Wid::ScrollTrack(_)))
            .map(|h| h.1),
    );
    // The mouse cursor (ComputerUIWindow.StyleChanged: DeusExCursor1, hot spot 32, 32).
    let mut cc = Canvas::new(64, 64, s);
    if let Some(tx) = hud.ui(&mut assets, "DeusExCursor2") {
        cc.drop_shadow(0, 0, &tx, (2.0, 2.5), 2.5, 0.6);
        cc.texture(0, 0, 64, 64, 0, 0, &tx, Style::Masked, m_col(12));
    }
    let cp = (mouse.0 - 32, mouse.1 - 32);
    place(&mut nodes.cursor, &cc, cp, d, sf, &mut images, &mut layers, &mut layer_nodes);
    if window.cursor_position().is_none() {
        if let Ok((_, mut v, _)) = layer_nodes.get_mut(nodes.cursor.e) {
            *v = Visibility::Hidden;
        }
    }
    let _ = &player;
}

fn edit_mut(t: &mut Term, n: u8) -> Option<&mut Edit> {
    match (t.screen.as_mut(), n) {
        (Some(Screen::Login { user, .. }), 0) => Some(user),
        (Some(Screen::Login { pass, .. }), 1) => Some(pass),
        (Some(Screen::Atm { acct, .. }), 0) => Some(acct),
        (Some(Screen::Atm { pin, .. }), 1) => Some(pin),
        (Some(Screen::Withdraw { amount, .. }), 0) => Some(amount),
        _ => None,
    }
}

fn scroll_of(t: &mut Term, id: u8) -> Option<&mut i32> {
    if id == L_ACCOUNTS {
        return t.accounts.as_mut().map(|a| &mut a.scroll);
    }
    match t.screen.as_mut() {
        Some(Screen::Email(m)) | Some(Screen::Bulletins(m)) => Some(if id == L_LIST { &mut m.list_scroll } else { &mut m.body_scroll }),
        _ => None,
    }
}

fn add_scroll(t: &mut Term, id: u8, px: i32) {
    if let Some(s) = scroll_of(t, id) {
        *s = (*s + px).max(0); // clamped when drawn
    }
}

fn scroll(t: &mut Term, w: Wid, step: i32) {
    match w {
        Wid::ScrollUp(id) => add_scroll(t, id, -step),
        Wid::ScrollDown(id) => add_scroll(t, id, step),
        _ => {}
    }
}

fn set_scroll_frac(t: &mut Term, id: u8, frac: f32) {
    // The content height isn't known here; scroll a large range and let the draw clamp it.
    if let Some(s) = scroll_of(t, id) {
        *s = (frac * 4000.0) as i32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_menus_and_bodies_parse() {
        let menu = "<EMAIL=01_Email01,re: NOBODY CARES!,Anon,Free Enterprise Mailing List,>\r\n<EMAIL=01_Email07,Visitors,JReed,KLloyd,>\r\n";
        let p = parse_text(menu);
        assert_eq!(p.emails.len(), 2);
        assert_eq!(p.emails[1].name, "01_Email07");
        assert_eq!(p.emails[1].from, "JReed");
        assert_eq!(p.emails[1].to, "KLloyd");
        assert_eq!(p.emails[1].cc, "");
        let body = "<COMMENT>inbox</COMMENT>\r\n<DC=255,255,255>\r\n<P><B>From:</B> JReed\r\n<P>Hi <PLAYERNAME>.\r\n";
        assert_eq!(parse_text(body).body, "From: JReed\nHi JC Denton.");
        let files = parse_text("<FILE=02_Bulletin01,Weather>");
        assert_eq!(files.files[0].name, "02_Bulletin01");
        assert_eq!(files.files[0].subject, "Weather");
    }

    #[test]
    fn unreal_atoi() {
        assert_eq!(atoi("120"), 120);
        assert_eq!(atoi("12abc"), 12);
        assert_eq!(atoi("abc"), 0);
        assert_eq!(atoi(" -5"), -5);
    }

    #[test]
    fn edit_field_keys() {
        let mut e = Edit::new(5);
        for c in ["a", "b", "c"] {
            e.key(KeyCode::KeyA, Some(c), false, false);
        }
        assert_eq!(e.get(), "abc");
        e.key(KeyCode::ArrowLeft, None, true, false);
        e.key(KeyCode::Backspace, None, false, false);
        assert_eq!(e.get(), "ab");
        for c in ["x", "y", "z", "w"] {
            e.key(KeyCode::KeyX, Some(c), false, false);
        }
        assert_eq!(e.get(), "abxyz"); // max 5
    }

    #[test]
    fn atm_balances_sync_and_hack_sum() {
        let mk = |n: &str, b: i32| CompData {
            class: "ATM".into(),
            kind: Kind::Atm,
            pos: DVec3::ZERO,
            users: vec![],
            accounts: vec![Account { number: n.into(), pin: "1".into(), balance: b }, Account { number: "9".into(), pin: "9".into(), balance: 50 }],
            views: Default::default(),
            specials: vec![],
            node: Node::default(),
            text_package: String::new(),
            bulletin_tag: String::new(),
            lockout_delay: 60.0,
            msg_locked_out: String::new(),
            locked_out: false,
            lockout_time: 0.0,
            last_hack_time: -9999.0,
            last_alarm_time: 0.0,
            alarm_timeout: 30.0,
            alarm: None,
            sucked_dry: false,
        };
        let mut data = HashMap::new();
        data.insert(1, mk("230023", 200));
        data.insert(2, mk("230023", 200));
        mod_balance(&mut data, 1, 0, 120);
        assert_eq!(data[&1].accounts[0].balance, 80);
        assert_eq!(data[&2].accounts[0].balance, 80);
        // Hacked at Advanced (2 x 0.5 = 1.0): the sum of every account.
        assert_eq!(data[&2].balance(-1, 1.0), 130);
        assert_eq!(data[&2].balance(-1, 0.5), 65);
    }
    /// COMP_PREVIEW=<dir> HUD_GAME=<root> cargo test -p dxview preview_computers -- --nocapture
    /// writes <dir>/comp_<screen>.rgba (640 x 480).
    #[test]
    fn preview_computers() {
        let (Ok(out), Ok(root)) = (std::env::var("COMP_PREVIEW"), std::env::var("HUD_GAME")) else {
            return;
        };
        let root = std::path::PathBuf::from(root);
        let mut assets = RuntimeAssets {
            classes: unreal_pkg::classes::ClassDb::open(&root.join("System")),
            lib: dx_level::TexLib::new(&root),
            images: Default::default(),
            materials: Default::default(),
            fire: Vec::new(),
        };
        let lvl = dx_level::load_level(&root, "01_NYC_UNATCOIsland.dx").unwrap();
        let mut hud = ClassicHud::default();
        let get = |hud: &mut ClassicHud, a: &mut RuntimeAssets, n: &str| hud.font(a, n).unwrap();
        let fonts = Fonts {
            title: get(&mut hud, &mut assets, "FontMenuTitle"),
            small: get(&mut hud, &mut assets, "FontMenuSmall"),
            headers: get(&mut hud, &mut assets, "FontMenuHeaders"),
            fixed: get(&mut hud, &mut assets, "FontFixedWidthSmall"),
        };
        let mut sec = Security::default();
        let mut movers = Movers { rt: vec![], entities: vec![], parts: vec![] };
        let mut inv = PlayerInventory::default();
        let mut ph = PlayerHealth::default();
        let mut events = GameEvents::default();
        let mut texts = Texts::default();
        let mut data = HashMap::new();
        let find_actor = |n: &str| lvl.actors.iter().position(|a| a.name == n).unwrap();
        let shots: Vec<(&str, &str, Which, Option<&str>)> = vec![
            ("login", "ComputerSecurity1", Which::Login, None),
            ("security", "ComputerSecurity0", Which::Security, None),
            ("email", "ComputerPersonal0", Which::Email, Some("KLLOYD")),
            ("atm", "ATM0", Which::Atm, None),
            ("withdraw", "ATM0", Which::Withdraw, None),
            ("disabled", "ATM0", Which::Disabled, None),
        ];
        for (file, name, which, user) in shots {
            let actor = find_actor(name);
            data.entry(actor).or_insert_with(|| CompData::load(&lvl, actor, &mut assets));
            let kind = data[&actor].kind;
            let mut t = Term {
                actor,
                kind,
                screen: None,
                hack: Some(Hack {
                    detection: 18.0,
                    save_detection: 25.0,
                    hack_t: 5.0,
                    save_hack: 10.0,
                    blink: 1.0,
                    digit_t: 0.0,
                    ticking: true,
                    hacking: true,
                    done: false,
                    detected: false,
                    notified: false,
                    msg: "Initializing ICE Breaker...".into(),
                    msg_shown: true,
                    red: false,
                    digits: ["0A9 4f  x1 Z@ 9k  3 1a!".into(), " 7  q1 0  B3 ]{ 8 m 2 ".into(), "a 1 9 Z 0".into(), "  3 3 ".into()],
                    btn_return: false,
                    btn_enabled: false,
                    seed: 1,
                }),
                accounts: None,
                hacked: false,
                no_hack: false,
                skill: 1,
                user: user.unwrap_or("").into(),
                user_idx: 0,
                win_pos: None,
                drag: None,
                pressed: None,
                pressed_right: None,
                focus: None,
                hits: vec![],
                repeat: 0.0,
                last_click: (0.0, None),
                clock: 0.0,
                exit: false,
                comp_rect: (0, 0, 0, 0),
            };
            {
                let mut wx = Wx {
                    data: &mut data,
                    sec: &mut sec,
                    movers: &mut movers,
                    lvl: &lvl,
                    inv: &mut inv,
                    ph: &mut ph,
                    events: &mut events,
                    hud: &mut hud,
                    assets: &mut assets,
                    texts: &mut texts,
                    now: 10.0,
                    mission: 1,
                };
                t.show(which, &mut wx);
                if let Some(Screen::Login { user, .. }) = t.screen.as_mut() {
                    user.text = "NSF001".chars().collect();
                    user.cur = 6;
                    user.anchor = 6;
                    t.focus = Some(Wid::Edit(0));
                }
            }
            let (sw, sh) = (640, 480);
            let mut full = Canvas::new(sw, sh, 1);
            full.fill(0, 0, sw, sh, Style::Normal, BLACK);
            let mut hits = vec![];
            let g = geo(&t, data.get(&actor).unwrap(), &mut hud, &mut assets);
            let (cw, ch) = comp_size(&g, &fonts);
            let pos = (sw / 2 - cw / 2, sh / 2 - ch / 2);
            let dv = Dv { data: &data, sec: &sec, movers: &movers, lvl: &lvl };
            if t.screen.is_some() {
                let mut ui = Ui { c: Canvas::new(cw, ch, 1), hud: &mut hud, assets: &mut assets, f: &fonts, o: pos, hits: &mut hits, pressed: None, hover: None, focus: t.focus };
                draw_comp(&mut ui, &mut t, &dv, &g);
                let Ui { c, .. } = ui;
                blit(&mut full, &c, pos);
            }
            if let Some(h) = t.hack.as_ref() {
                let mut ui = Ui { c: Canvas::new(HACK_SIZE.0, HACK_SIZE.1, 1), hud: &mut hud, assets: &mut assets, f: &fonts, o: (0, 0), hits: &mut hits, pressed: None, hover: None, focus: None };
                draw_hack(&mut ui, h);
                let Ui { c, .. } = ui;
                blit(&mut full, &c, (sw - HACK_SIZE.0, 0));
            }
            std::fs::write(format!("{out}/comp_{file}.rgba"), full.to_rgba8()).unwrap();
            println!("wrote {file} ({} x {})", cw, ch);
        }
    }

    /// Composite a premultiplied canvas onto another.
    fn blit(dst: &mut Canvas, src: &Canvas, at: (i32, i32)) {
        for y in 0..src.h {
            for x in 0..src.w {
                let (dx, dy) = (x + at.0, y + at.1);
                if dx < 0 || dy < 0 || dx >= dst.w || dy >= dst.h {
                    continue;
                }
                let s = src.px[(y * src.w + x) as usize];
                let d = &mut dst.px[(dy * dst.w + dx) as usize];
                for k in 0..3 {
                    d[k] = (s[k] + d[k] * (1.0 - s[3])).min(1.0);
                }
            }
        }
    }
}

/// A new map is going in (loader.rs): this module's state of the map being left.
pub fn reset_level() {
    OPEN.store(false, std::sync::atomic::Ordering::Relaxed);
    IGNORED.store(false, std::sync::atomic::Ordering::Relaxed);
    if let Ok(mut v) = FROB.lock() {
        *v = None;
    }
}
