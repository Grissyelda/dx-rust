//! The menus: MenuMain and the windows it opens (MenuUIMenuWindow lists of big buttons, MenuUIScreenWindow
//! screens of choices, MenuScreenNewGame character creation, MenuScreenCustomizeKeys, MenuUIMessageBoxWindow),
//! the DeusExRootWindow window stack, and the DX.dx logo flyby they sit on when the game starts.
//!
//! Ported from MenuUIWindow (frame, action buttons, help line, ProcessMenuAction, message boxes),
//! MenuUIMenuWindow / MenuUIMenuButtonWindow, MenuUIScreenWindow + MenuUIChoice / Enum / Slider / Action and
//! every MenuChoice_* the Options / Display / Sound / Controls / Colors / Brightness screens use (labels, values,
//! help text and defaults come from the class defaults), MenuSelectDifficulty, MenuScreenNewGame,
//! MenuScreenCustomizeKeys, MenuUIMessageBoxWindow, DeusExRootWindow (InvokeMenu / InvokeMenuScreen hide the
//! menu below, message boxes don't; the game pauses under menus except on the logo map; MaskBackground) and
//! DeusExPlayer.TravelPostAccept's DX.dx IntroCam flyby.
//!
//! Settings are kept in settings.rs. Ours: HUD Scale (Small / Medium / Large) on the Display screen.
//! Not available yet (a message says so): Save / Load (point 3), Training, Play Intro, Credits (cutscenes),
//! render device and colour depth (fixed by the renderer). Ours: the multiplayer screens (Multiplayer, Host Room,
//! Join Room, Lobby Browser) over online services (online.rs).

use crate::classic_hud::{ClassicHud, HudLayer};
use crate::inventory_screen::{inside, label};
use crate::keybinds::Bindings;
use crate::menuui::*;
use crate::menuui::Rect;
use crate::uicanvas::{wrap, Canvas, HAlign, Rgb, Style, VAlign};
use crate::weapons::RuntimeAssets;
use crate::{FlyCam, LevelRes, SkyCam};
use bevy::input::mouse::MouseWheel;
use imsim_net::{Look, Mode, RoomInfo, RoomSettings};
use bevy::prelude::*;
use bevy::window::{CursorOptions, MonitorSelection, WindowMode};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::hash::{Hash, Hasher};
use unreal_pkg::props::{find, Property, Value};
use unreal_pkg::Reader;

static OPEN: AtomicBool = AtomicBool::new(false);
/// The menu was up when a map change cleared it (Load Game keeps its window until the loader resets this module), so
/// the next frame is back in the game as when it closes.
static CLOSED_BY_MAP: AtomicBool = AtomicBool::new(false);
/// DeusExLevelInfo.MissionNumber of this map (< 0: the logo / splash maps).
static MISSION: AtomicI32 = AtomicI32::new(0);
static DEAD: AtomicBool = AtomicBool::new(false);

static CLOSE_ALL: AtomicBool = AtomicBool::new(false);

/// The pad's Start on an in-game menu: close every menu window (not on the logo map or after death, where the
/// menu is the only thing there). True when it will.
pub fn request_close_all() -> bool {
    if intro_map() || DEAD.load(Ordering::Relaxed) {
        return false;
    }
    CLOSE_ALL.store(true, Ordering::Relaxed);
    true
}

/// A menu is up (the HUD hides, the game gets no input).
pub fn open() -> bool {
    OPEN.load(Ordering::Relaxed)
}

/// The map has a DeusExLevelInfo whose MissionNumber is 0 (the training maps, which don't store it).
static TRAINING: AtomicBool = AtomicBool::new(false);
pub fn training() -> bool {
    TRAINING.load(Ordering::Relaxed)
}

/// DeusExRootWindow.AtIntroMap: the logo map (no HUD, no saving).
pub fn intro_map() -> bool {
    MISSION.load(Ordering::Relaxed) < 0
}

#[derive(Component)]
pub struct MenuBackdrop;

// ------------------------------------------------------------------------------------------------ widgets

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum W {
    Win,
    Title,
    MenuBtn(u8),
    Act(u8),
    ChoiceBtn(u8),
    ChoiceVal(u8),
    Slider(u8),
    Edit(u8),
    Row(u8, usize),
    ScrollUp(u8),
    ScrollDown(u8),
    ScrollTrack(u8),
    Upgrade,
    Downgrade,
    PortraitPrev,
    PortraitNext,
    Portrait,
    Header(u8),
}

impl KitWid for W {
    fn row(list: u8, i: usize) -> Self {
        W::Row(list, i)
    }
    fn scroll_up(id: u8) -> Self {
        W::ScrollUp(id)
    }
    fn scroll_down(id: u8) -> Self {
        W::ScrollDown(id)
    }
    fn scroll_track(id: u8) -> Self {
        W::ScrollTrack(id)
    }
}

// ------------------------------------------------------------------------------------------------ class data

fn pstr(p: &[Property], n: &str) -> String {
    match find(p, n) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    }
}
fn pint(p: &[Property], n: &str, d: i32) -> i32 {
    match find(p, n) {
        Some(Value::Int(v)) => *v,
        Some(Value::Byte(v)) => *v as i32,
        Some(Value::Float(v)) => *v as i32,
        _ => d,
    }
}
fn pfl(p: &[Property], n: &str, d: f32) -> f32 {
    match find(p, n) {
        Some(Value::Float(v)) => *v,
        Some(Value::Int(v)) => *v as f32,
        Some(Value::Byte(v)) => *v as f32,
        _ => d,
    }
}
fn pbool(p: &[Property], n: &str) -> bool {
    matches!(find(p, n), Some(Value::Bool(true)))
}
fn ppath(p: &[Property], n: &str) -> String {
    match find(p, n) {
        Some(Value::Path(s)) if !s.ends_with(".None") => s.clone(),
        _ => String::new(),
    }
}
fn indexed<'a>(p: &'a [Property], n: &str) -> Vec<(u32, &'a Value)> {
    let mut v: Vec<(u32, &Value)> = p.iter().filter(|x| x.name.eq_ignore_ascii_case(n)).map(|x| (x.index, &x.value)).collect();
    v.sort_by_key(|x| x.0);
    v
}
fn short(path: &str) -> String {
    path.rsplit('.').next().unwrap_or(path).to_string()
}
fn texname(path: &str) -> String {
    path.trim_start_matches("DeusExUI.UserInterface.").to_string()
}

/// MenuUIWindow's frame values for a class.
#[derive(Clone, Debug, Default)]
struct Frame {
    class: String,
    title: String,
    cw: i32,
    ch: i32,
    voff: i32,
    textures: Vec<String>,
    cols: i32,
    left_edge: bool,
    right_edge: bool,
    bar: bool,
    help: bool,
    help_always: bool,
    help_y: i32,
    /// (align: 0 left / 2 right, AB_ action, label, key)
    actions: Vec<(u8, u8, String, String)>,
    /// Shadow: offset, size, textures.
    shadow: Option<((i32, i32), (i32, i32))>,
    escape_saves: bool,
    min_title: i32,
    /// Ours: a band of the client background left out (y, height) - the empty choice wells under the last row of a
    /// screen that offers fewer settings than the original's art has room for. The window is that much shorter.
    cuts: Vec<(i32, i32)>,
    /// Ours: a band of the client background drawn twice (y, height), the copy right under it - a row added to the
    /// art (the main menu's Achievements button). The window is that much taller.
    dups: Vec<(i32, i32)>,
}

fn frame(assets: &mut RuntimeAssets, class: &str) -> Frame {
    let d = assets.classes.defaults(class);
    let mut f = Frame {
        class: class.to_string(),
        title: pstr(&d, "Title"),
        cw: pint(&d, "ClientWidth", 300),
        ch: pint(&d, "ClientHeight", 200),
        voff: pint(&d, "verticalOffset", 0),
        cols: pint(&d, "textureCols", 1).max(1),
        left_edge: pbool(&d, "bLeftEdgeActive"),
        right_edge: pbool(&d, "bRightEdgeActive"),
        bar: pbool(&d, "bActionButtonBarActive"),
        help: pbool(&d, "bUsesHelpWindow"),
        help_always: pbool(&d, "bHelpAlwaysOn"),
        help_y: pint(&d, "helpPosY", 0),
        escape_saves: pbool(&d, "bEscapeSavesSettings"),
        min_title: 200,
        ..default()
    };
    for (_, v) in indexed(&d, "clientTextures") {
        if let Value::Path(p) = v {
            if !p.ends_with(".None") {
                f.textures.push(texname(p));
            }
        }
    }
    let label_of = |k: &str| pstr(&d, k);
    for (_, v) in indexed(&d, "actionButtons") {
        if let Value::Struct(_, b) = v {
            let mut r = Reader::new(b, 0);
            let align = r.u8().unwrap_or(0);
            let action = r.u8().unwrap_or(0);
            let text = r.fstring().unwrap_or_default();
            let key = r.fstring().unwrap_or_default();
            if action == 0 {
                break;
            }
            let lbl = match action {
                1 => label_of("btnLabelOK"),
                2 => label_of("btnLabelCancel"),
                3 => label_of("btnLabelResetToDefaults"),
                4 => label_of("btnLabelPrevious"),
                _ => text,
            };
            f.actions.push((align, action, lbl, key));
        }
    }
    let shadow = ppath(&d, "winShadowClass");
    if !shadow.is_empty() {
        let s = assets.classes.defaults(&short(&shadow));
        f.shadow = Some(((pint(&s, "shadowOffsetX", 0), pint(&s, "shadowOffsetY", 0)), (pint(&s, "shadowWidth", 0), pint(&s, "shadowHeight", 0))));
    }
    f
}

/// A MenuUIMenuWindow button.
#[derive(Clone, Debug)]
struct MenuBtn {
    label: String,
    y: i32,
    /// EMenuActions: 0 Menu, 1 MenuScreen, 2 Previous, 3 NewGame, 4 Training, 5 Intro, 6 Quit, 7 Custom.
    action: u8,
    invoke: String,
    key: String,
    enabled: bool,
}

fn menu_buttons(assets: &mut RuntimeAssets, class: &str) -> (Vec<MenuBtn>, i32, i32) {
    let d = assets.classes.defaults(class);
    let names: Vec<String> = indexed(&d, "ButtonNames")
        .into_iter()
        .filter_map(|(_, v)| match v {
            Value::Str(s) => Some(s.clone()),
            _ => None,
        })
        .collect();
    let defs = indexed(&d, "buttonDefaults");
    let mut out = Vec::new();
    for (i, n) in names.iter().enumerate() {
        if n.is_empty() {
            break;
        }
        let Some((_, Value::Struct(_, b))) = defs.iter().find(|x| x.0 as usize == i) else { continue };
        let mut r = Reader::new(b, 0);
        let y = r.i32().unwrap_or(0);
        let action = r.u8().unwrap_or(0);
        let invoke_i = r.compact_index().unwrap_or(0);
        let key = r.fstring().unwrap_or_default();
        let invoke = if invoke_i != 0 {
            assets.classes.packages.get("deusex").map(|p| short(&p.object_path(invoke_i))).unwrap_or_default()
        } else {
            String::new()
        };
        out.push(MenuBtn { label: n.clone(), y, action, invoke, key, enabled: true });
    }
    (out, pint(&d, "buttonXPos", 7), pint(&d, "buttonWidth", 245))
}

#[derive(Clone, Debug)]
enum ChoiceKind {
    /// enumText values.
    Enum(Vec<String>),
    /// ticks, start, end, tick labels.
    Slider { ticks: i32, start: f32, end: f32, labels: Vec<String> },
    /// MenuUIChoiceAction: open a screen (invoke) or a custom action.
    Action { action: u8, invoke: String },
}

#[derive(Clone, Debug)]
struct Choice {
    /// Settings key (the class name after "MenuChoice_").
    key: String,
    label: String,
    help: String,
    kind: ChoiceKind,
    value: f32,
    /// Value when the screen opened (CancelSetting restores it).
    saved: f32,
    /// MenuUIChoiceEnum.defaultInfoWidth / PosX, choiceControlPosX, buttonVerticalOffset.
    info_w: i32,
    info_x: i32,
    control_x: i32,
    voff: i32,
    /// Shown as a drop-down list (ours: screen resolution, display mode) instead of click-to-cycle.
    dropdown: bool,
}

pub const RESOLUTIONS: &[(u32, u32)] = &[
    (640, 480), (800, 600), (1024, 768), (1152, 864), (1280, 720), (1280, 800), (1280, 960), (1280, 1024),
    (1366, 768), (1440, 900), (1600, 900), (1600, 1200), (1680, 1050), (1920, 1080), (1920, 1200), (2560, 1080),
    (2560, 1440), (3440, 1440), (3840, 2160),
];

fn choice(assets: &mut RuntimeAssets, class: &str) -> Choice {
    let d = assets.classes.defaults(class);
    let key = class.trim_start_matches("MenuChoice_").to_string();
    let enums: Vec<String> = indexed(&d, "enumText")
        .into_iter()
        .filter_map(|(_, v)| match v {
            Value::Str(s) if !s.is_empty() => Some(s.clone()),
            _ => None,
        })
        .collect();
    let mut kind = if find(&d, "numTicks").is_some() {
        let ticks = pint(&d, "numTicks", 11).max(2);
        let start = pfl(&d, "startValue", 0.0);
        let end = pfl(&d, "endValue", 10.0);
        let labels: Vec<String> = match key.as_str() {
            "MusicVolume" | "SoundVolume" | "SpeechVolume" => {
                let mut v: Vec<String> = (0..11).map(|i| i.to_string()).collect();
                v[0] = pstr(&d, "VolumeOffText");
                v[10] = pstr(&d, "VolumeMaxText");
                v
            }
            "MouseSensitivity" => (1..11).map(|i| i.to_string()).collect(),
            "EffectsChannels" => (4..17).map(|i| i.to_string()).collect(),
            "LogTimeoutValue" => {
                let s = pstr(&d, "msgSecond");
                (0..19).map(|i| format!("{:.1}{s}", 1.0 + i as f32 * 0.5)).collect()
            }
            _ => Vec::new(),
        };
        ChoiceKind::Slider { ticks, start, end, labels }
    } else if find(&d, "Action").is_some() || find(&d, "Invoke").is_some() {
        ChoiceKind::Action { action: pint(&d, "Action", 0) as u8, invoke: short(&ppath(&d, "Invoke")) }
    } else {
        ChoiceKind::Enum(enums)
    };
    // Lists the menus fill in at run time.
    match key.as_str() {
        "MenuColor" => kind = ChoiceKind::Enum(crate::menuui::menu_themes().iter().map(|s| s.to_string()).collect()),
        "HUDColor" => kind = ChoiceKind::Enum(crate::menuui::hud_themes().iter().map(|s| s.to_string()).collect()),
        "Resolution" => kind = ChoiceKind::Enum(RESOLUTIONS.iter().map(|(w, h)| format!("{w}x{h}")).collect()),
        "SampleRate" | "SoundQuality" => {}
        _ => {}
    }
    let value = crate::settings::get(&key);
    Choice {
        label: pstr(&d, "actionText"),
        help: pstr(&d, "helpText"),
        kind,
        value,
        saved: value,
        info_w: pint(&d, "defaultInfoWidth", 77),
        info_x: pint(&d, "defaultInfoPosX", 270),
        control_x: pint(&d, "choiceControlPosX", 270),
        voff: pint(&d, "buttonVerticalOffset", 0),
        dropdown: key == "Resolution",
        key,
    }
}

/// Our own choice: HUD Scale on the Display screen.
fn hud_scale_choice() -> Choice {
    let v = setting!("HUDScale");
    Choice {
        key: "HUDScale".into(),
        label: "|&HUD Scale".into(),
        help: "Size of the HUD, menus, inventory and computers: Medium is the original's (whole multiples, crisp pixels); Large fills the screen (best on a Steam Deck); Small is a step smaller.".into(),
        kind: ChoiceKind::Enum(vec!["Small".into(), "Medium".into(), "Large".into()]),
        value: v,
        saved: v,
        info_w: 77,
        info_x: 270,
        control_x: 270,
        voff: 0,
        dropdown: false,
    }
}

/// Ours, in place of Toggle Full-Screen Mode: Full Screen (exclusive), Windowed, or Borderless (a screen-sized
/// window without borders, so alt-tab and clicking out behave).
fn display_mode_choice() -> Choice {
    let v = setting!("DisplayMode");
    Choice {
        key: "DisplayMode".into(),
        label: "|&Display Mode".into(),
        help: "Full Screen takes the monitor; Windowed is a normal window; Borderless fills the screen as a window, so alt-tab and clicking out work smoothly.".into(),
        kind: ChoiceKind::Enum(DISPLAY_MODES.iter().map(|s| s.to_string()).collect()),
        value: v,
        saved: v,
        info_w: 110,
        info_x: 270,
        control_x: 270,
        voff: 0,
        dropdown: true,
    }
}

pub const DISPLAY_MODES: [&str; 3] = ["Full Screen", "Windowed", "Borderless"];

/// Ours: Vertical Sync on the Display screen.
fn vsync_choice() -> Choice {
    let mut c = display_mode_choice();
    c.key = "VSync".into();
    c.label = "|&Vertical Sync".into();
    c.help = "Wait for the monitor's refresh before showing each frame: no tearing, frame rate capped at the refresh rate. Off: as many frames as the PC can draw.".into();
    c.kind = ChoiceKind::Enum(vec!["Disabled".into(), "Enabled".into()]);
    c.value = setting!("VSync");
    c.saved = c.value;
    c.dropdown = false;
    c
}

/// The Bevy present mode for a VSync setting.
pub fn present_mode(v: f32) -> bevy::window::PresentMode {
    if v > 0.5 {
        bevy::window::PresentMode::AutoVsync
    } else {
        bevy::window::PresentMode::AutoNoVsync
    }
}

/// Settings > Display resolution. macOS sizes windows in points, and the game opens with the saved size in points,
/// so a pick there is points too: physical pixels would make the window half the size on a Retina display.
fn set_window_size(window: &mut Window, w: u32, h: u32) {
    #[cfg(target_os = "macos")]
    window.resolution.set(w as f32, h as f32);
    #[cfg(not(target_os = "macos"))]
    window.resolution.set_physical_resolution(w, h);
}

/// The Bevy window mode for a DisplayMode setting.
/// The window mode for a DisplayMode setting. The primary monitor, not "current": at startup there is no window
/// yet, so winit has no current monitor and Bevy panics ("Unable to get monitor").
/// On Linux, Wayland has no primary monitor (and no exclusive fullscreen): Bevy's exclusive mode panicked there
/// ("Unable to get monitor", GitHub issue #4), so both full-screen modes are borderless on the first monitor.
/// The mode the window is created in. On Linux winit may not have listed any monitor yet when the first window is
/// made (Wayland, some X11 setups: GitHub issue #4, "Unable to get monitor" from Bevy as it looks the monitor up), so
/// a full-screen mode starts windowed and `fullscreen_when_ready_system` switches once a monitor is known.
pub fn start_window_mode(v: f32) -> WindowMode {
    #[cfg(target_os = "linux")]
    {
        let _ = v;
        WindowMode::Windowed
    }
    #[cfg(not(target_os = "linux"))]
    {
        window_mode(v)
    }
}

/// Where the window opens. Linux (X11): centred on the primary monitor - left to itself the window opened at 0,0,
/// on the leftmost monitor of a multi-monitor desktop (GitHub issue #50). Wayland ignores positions; the
/// compositor places it. Elsewhere the system's own placement (already the primary monitor).
pub fn start_window_position() -> bevy::window::WindowPosition {
    if cfg!(target_os = "linux") {
        bevy::window::WindowPosition::Centered(MonitorSelection::Primary)
    } else {
        bevy::window::WindowPosition::Automatic
    }
}

/// Linux: the saved full-screen mode, applied once Bevy knows of a monitor (see `start_window_mode`). If none ever
/// shows up the window stays windowed, maximized, instead of crashing.
pub fn fullscreen_when_ready_system(
    time: Res<Time<Real>>,
    monitors: Query<&bevy::window::Monitor>,
    mut windows: Query<&mut Window, With<bevy::window::PrimaryWindow>>,
    mut done: Local<bool>,
) {
    if *done || !cfg!(target_os = "linux") {
        return;
    }
    let want = setting!("DisplayMode").round() as i32;
    let Ok(mut w) = windows.single_mut() else { return };
    if want == 1 {
        *done = true;
        return;
    }
    if !monitors.is_empty() && time.elapsed_secs() > 0.2 {
        w.mode = window_mode(want as f32);
        *done = true;
    } else if time.elapsed_secs() > 5.0 {
        println!("dxview: no monitor reported - staying windowed");
        w.set_maximized(true);
        *done = true;
    }
}

pub fn window_mode(v: f32) -> WindowMode {
    #[cfg(target_os = "linux")]
    {
        match v.round() as i32 {
            // The monitor the window is on (it was opened centred on the primary one, `start_window_position`):
            // monitor 0 is the leftmost on a multi-monitor desktop (GitHub issue #50). Applied only once the window
            // exists, where an unknown current monitor means winit's own choice (Borderless(None)), never a panic.
            0 | 2 => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
            _ => WindowMode::Windowed,
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        match v.round() as i32 {
            0 => WindowMode::Fullscreen(MonitorSelection::Primary, bevy::window::VideoModeSelection::Current),
            2 => WindowMode::BorderlessFullscreen(MonitorSelection::Primary),
            _ => WindowMode::Windowed,
        }
    }
}

impl Choice {
    fn slider_tick(&self) -> i32 {
        match &self.kind {
            ChoiceKind::Slider { ticks, start, end, .. } => {
                let t = ((self.value - start) / (end - start).max(1e-6) * (*ticks - 1) as f32).round() as i32;
                t.clamp(0, ticks - 1)
            }
            _ => 0,
        }
    }
    fn set_tick(&mut self, t: i32) {
        if let ChoiceKind::Slider { ticks, start, end, .. } = &self.kind {
            let t = t.clamp(0, ticks - 1);
            self.value = start + (end - start) * t as f32 / (*ticks - 1) as f32;
        }
    }
    fn value_text(&self) -> String {
        match &self.kind {
            ChoiceKind::Enum(v) => v.get(self.value as usize).cloned().unwrap_or_default(),
            ChoiceKind::Slider { labels, .. } => {
                let t = self.slider_tick() as usize;
                labels.get(t).cloned().unwrap_or_else(|| format!("{:.2}", self.value))
            }
            ChoiceKind::Action { .. } => String::new(),
        }
    }
    /// CycleNextValue / CyclePreviousValue.
    fn cycle(&mut self, dir: i32) {
        match &self.kind {
            ChoiceKind::Enum(v) => {
                let n = v.len().max(1) as i32;
                self.value = (self.value as i32 + dir).rem_euclid(n) as f32;
            }
            ChoiceKind::Slider { ticks, .. } => {
                let t = (self.slider_tick() + dir).rem_euclid(*ticks);
                self.set_tick(t);
            }
            ChoiceKind::Action { .. } => {}
        }
    }
}

// ------------------------------------------------------------------------------------------------ windows

struct NewGame {
    name: Edit,
    /// texPortraits index: 0..4, or with Lay D Denton 0..9 alternating male / female (odd = female, skin = /2).
    portrait: usize,
    /// LDDP's checkboxes: "Original Morpheus?" and (female only) "Original Interactions?".
    retro_morpheus: bool,
    male_interactions: bool,
    skills: imsim_core::skills::Skills,
    /// Display order (sorted by name).
    order: Vec<usize>,
    sel: Option<usize>,
    difficulty: f32,
    desc_scroll: i32,
    /// Ours: a multiplayer character (Choose Character > New Character): START hands it to the game session.
    multiplayer: bool,
}

struct Keys {
    sel: Option<usize>,
    scroll: i32,
    waiting: bool,
    map: Vec<crate::keybinds::Input>,
    note: String,
    /// Ours: the Controller Bindings screen (gamepad.rs PAD_ACTIONS) on the same frame.
    pad: bool,
    pad_map: Vec<Option<crate::gamepad::PadBtn>>,
}

impl Keys {
    fn rows(&self) -> usize {
        if self.pad {
            crate::gamepad::PAD_ACTIONS.len()
        } else {
            crate::keybinds::ACTIONS.len()
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum MsgPurpose {
    Exit,
    AskToTrain,
    Info,
    NameBlank,
    Restart,
    /// Save over the selected game (st.box_save).
    Overwrite,
    /// Delete the selected game.
    Delete,
    /// DeusExRootWindow.ConfirmQuickLoad.
    QuickLoad,
    /// MenuUIWindow.ConfirmTraining / ConfirmIntro (MB_Training / MB_Intro).
    Training,
    Intro,
    /// root.MessageBox asked for by another screen (the persona screen's Delete Note?): the answer goes back.
    External,
    /// Ours: the controller layout picture (Settings > Enhancements > Controller Layout; once at the first map for
    /// a player on a pad).
    PadMap,
    /// Ours: "Connecting to online services..." (Cancel disconnects), "Looking for room...", and what a join found.
    Connecting,
    Joining,
    MpResult,
    /// "Leave the multiplayer game?" (the main menu's Leave Multiplayer).
    MpLeave,
    /// Ours: the Achievements screen's Reset All.
    AchReset,
}

/// Open the controller layout picture (paddiagram).
static PADMAP_OPEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn open_pad_map() {
    PADMAP_OPEN.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn pad_map_box(hud: &mut ClassicHud, assets: &mut RuntimeAssets) -> Option<Win> {
    let mut w = msg_box(hud, assets, "Controller Layout", "", false, MsgPurpose::PadMap)?;
    w.f.cw = crate::paddiagram::W;
    w.f.ch = crate::paddiagram::H;
    w.f.textures.clear();
    Some(w)
}

/// root.MessageBox(title, text, MB_YesNo) for another screen; its answer comes back through `take_box_answer`.
static EXTERNAL_BOX: std::sync::Mutex<Option<(String, String)>> = std::sync::Mutex::new(None);
static EXTERNAL_ANSWER: std::sync::Mutex<Option<bool>> = std::sync::Mutex::new(None);

pub fn open_yes_no(title: &str, text: &str) {
    if let Ok(mut b) = EXTERNAL_BOX.lock() {
        *b = Some((title.to_string(), text.to_string()));
    }
}

pub fn take_box_answer() -> Option<bool> {
    EXTERNAL_ANSWER.lock().ok().and_then(|mut a| a.take())
}

/// MenuScreenSaveGame / MenuScreenLoadGame.
/// A row of the save / load list.
#[derive(Clone, Copy, PartialEq, Debug)]
enum SlRow {
    /// "New Save Game" (save screen).
    New,
    /// A character's heading (load screen; index into `chars`), opened and closed like the Enhancements groups.
    Char(usize),
    /// A saved game (index into `games`).
    Game(usize),
}

struct SaveLoad {
    saving: bool,
    /// The save screen: only the character being played. The load screen: every character's.
    games: Vec<crate::save::SaveInfo>,
    /// Load screen: (character id, name, saves) in the order of each one's newest save.
    chars: Vec<(String, String, usize)>,
    /// The characters shown opened.
    open: std::collections::HashSet<String>,
    list: Vec<SlRow>,
    /// Selected row (into `list`).
    sel: Option<usize>,
    scroll: i32,
    /// The description being typed (save screen) for the selected row.
    edit: Edit,
    confirm_delete: bool,
    /// The selected game's snapshot (file, picture).
    snap: Option<(std::path::PathBuf, Option<crate::uicanvas::UiTex>)>,
}

impl SaveLoad {
    fn new(saving: bool) -> SaveLoad {
        let mut s = SaveLoad {
            saving,
            games: Vec::new(),
            chars: Vec::new(),
            open: Default::default(),
            list: Vec::new(),
            sel: None,
            scroll: 0,
            edit: Edit::new(64),
            confirm_delete: setting!("ConfirmSaveDeletes") < 0.5,
            snap: None,
        };
        s.reload();
        // The character being played (else the one saved last) starts opened, its newest save selected.
        if !saving {
            let me = crate::save::active_char();
            let first = s.chars.iter().find(|c| c.0 == me).or(s.chars.first()).map(|c| c.0.clone());
            if let Some(id) = first {
                s.open.insert(id.clone());
                s.build();
                s.sel = s.list.iter().position(|r| matches!(r, SlRow::Game(i) if s.games[*i].char_id == id));
            }
        }
        if s.sel.is_none() && !s.list.is_empty() {
            s.sel = Some(0);
        }
        s
    }
    /// Read the folder(s) again (after a delete) and rebuild the rows.
    fn reload(&mut self) {
        if self.saving {
            self.games = crate::save::list_char(&crate::save::active_char());
        } else {
            self.games = crate::save::list();
            self.chars.clear();
            for g in &self.games {
                match self.chars.iter_mut().find(|c| c.0 == g.char_id) {
                    Some(c) => c.2 += 1,
                    None => {
                        let name = if g.char_id.is_empty() {
                            format!("{} (earlier saves)", g.char_name)
                        } else {
                            g.char_name.clone()
                        };
                        self.chars.push((g.char_id.clone(), name, 1));
                    }
                }
            }
        }
        self.build();
    }
    fn build(&mut self) {
        self.list.clear();
        if self.saving {
            self.list.push(SlRow::New);
            self.list.extend((0..self.games.len()).map(SlRow::Game));
            return;
        }
        for (ci, c) in self.chars.iter().enumerate() {
            self.list.push(SlRow::Char(ci));
            if self.open.contains(&c.0) {
                self.list.extend((0..self.games.len()).filter(|&i| self.games[i].char_id == c.0).map(SlRow::Game));
            }
        }
    }
    /// The game a row stands for.
    fn game(&self, row: usize) -> Option<&crate::save::SaveInfo> {
        match self.list.get(row)? {
            SlRow::Game(i) => self.games.get(*i),
            _ => None,
        }
    }
    fn rows(&self) -> usize {
        self.list.len()
    }
    /// Open / close the character heading at `row` (load screen). True when the row was a heading.
    fn toggle(&mut self, row: usize) -> bool {
        let Some(SlRow::Char(c)) = self.list.get(row).copied() else { return false };
        let id = self.chars[c].0.clone();
        if !self.open.remove(&id) {
            self.open.insert(id);
        }
        self.build();
        self.sel = self.list.iter().position(|r| *r == SlRow::Char(c));
        crate::sound::play_player("DeusExSounds.UserInterface.Menu_Press", 0.25, 0.0);
        true
    }
    /// The text of a row: (description, date).
    fn row_text(&self, row: SlRow, new_label: &str) -> [String; 2] {
        match row {
            SlRow::New => [new_label.to_string(), String::new()],
            SlRow::Char(c) => {
                let (id, name, n) = &self.chars[c];
                let sign = if self.open.contains(id) { "-" } else { "+" };
                [format!("{sign} {name}"), format!("{n} save{}", if *n == 1 { "" } else { "s" })]
            }
            SlRow::Game(i) => {
                let g = &self.games[i];
                let d = if self.saving { g.description.clone() } else { format!("    {}", g.description) };
                [d, crate::save::format_date(g.saved_unix)]
            }
        }
    }
}

struct Msg {
    text: String,
    yes_no: bool,
    purpose: MsgPurpose,
}

enum Body {
    /// `notes`: ours, lines of text in wells at (y): the multiplayer screen's connection and server address.
    Menu { buttons: Vec<MenuBtn>, x: i32, w: i32, notes: Vec<(i32, String)> },
    Choices { choices: Vec<Choice> },
    NewGame(NewGame),
    Keys(Keys),
    Msg(Msg),
    SaveLoad(SaveLoad),
    Keypad(KeypadWin),
    Enhance(Enhance),
    /// Ours: the main menu's Achievements screen (every character's).
    Achieve(AchieveView),
    /// Ours: the welcome window (welcome.rs), its text scroll.
    Welcome(i32),
    /// Ours: the multiplayer screens - hosting a room, joining one by code, the public room list.
    Host(MpHost),
    Join(MpJoin),
    Browse(MpBrowse),
    /// Ours: Choose Character before joining a room (multiplayer characters, New Character).
    Chars(MpChars),
    /// Ours: Online Services - the lobby server's address.
    Addr(MpAddr),
    /// Ours: the waiting room (a joiner in the host's lobby).
    Wait(MpWait),
    /// Ours: the deathmatch character creator (dmchar.rs).
    DmChar(DmEdit),
}

struct AchieveView {
    scroll: i32,
    chars: Vec<crate::achievements::CharAch>,
    global: std::collections::HashMap<String, (u64, String)>,
}

/// Ours: Settings > Enhancements as one scrolling, searchable list of every option beyond the original (the
/// Controls screen's list art), so new v2 toggles never run out of rows. Labels carry their group ("Controller:",
/// "DualSense:") so a search finds a group. Click a selected row (or Enter, or Left / Right) to change it; action
/// rows open their screen. Applied on OK like the other settings screens.
struct Enhance {
    items: Vec<Choice>,
    search: Edit,
    /// Selected row.
    sel: Option<Row>,
    scroll: i32,
    /// Groups ("Controller", "DualSense"...) opened out; the rest show as one collapsed row each.
    open: std::collections::HashSet<String>,
    /// The Classic / Modern row at the top (remaster.rs): its state, whether it was flipped here, and the Modern
    /// values remembered when it went Classic (written on OK).
    modern: bool,
    mode_changed: bool,
    memory: Vec<(String, f32)>,
}

/// A row of the Enhancements list: a group's header (click / Enter opens or closes it) or one setting.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Row {
    /// Classic / Modern: every enhancement at once.
    Master,
    Group(usize),
    Item(usize),
}

/// The group of a "Group: Label" item, and the label without it.
fn split_group(label: &str) -> (&str, &str) {
    label.split_once(": ").unwrap_or(("Other", label))
}

/// A "Group: Label" item of the Enhancements list.
fn tag(mut c: Choice, group: &str) -> Choice {
    c.label = format!("{group}: {}", label(&c.label));
    c
}

impl Enhance {
    fn new() -> Enhance {
        let mut items = Vec::new();
        Self::controller_items(&mut items);
        Self::toggle_items(&mut items);
        Self::mod_items(&mut items);
        Self::option_items(&mut items);
        Self::performance_items(&mut items);
        Self::hud_items(&mut items);
        if crate::touch::enabled() {
            Self::touch_items(&mut items);
        }
        // Android is Classic only: no Modern feature to turn on one by one, no mods.
        if crate::platform::android() {
            items.retain(|c| !crate::remaster::modern_only(&c.key));
        }
        Enhance {
            items,
            search: Edit::new(40),
            sel: Some(if crate::platform::android() { Row::Group(0) } else { Row::Master }),
            scroll: 0,
            open: Default::default(),
            modern: crate::remaster::is_modern(),
            mode_changed: false,
            memory: Vec::new(),
        }
    }
    /// Controller and DualSense options (the Enhancements screen's old pad rows, Controller Options, DualSense Options).
    fn controller_items(items: &mut Vec<Choice>) {
        for c in enhancement_choices() {
            if c.key == "PadOptions" || c.key == "PadDualSense" || c.key == "DebugMode" {
                continue;
            }
            items.push(tag(c, "Controller"));
        }
        items.extend(pad_option_choices().into_iter().map(|c| tag(c, "Controller")));
        items.extend(dualsense_choices().into_iter().map(|c| tag(c, "DualSense")));
    }
    /// The on / off enhancements: gameplay, interface, AI, visuals, fixes, quality of life, sound.
    fn toggle_items(items: &mut Vec<Choice>) {
        for (key, group, lbl, help) in [
            ("NPCFiniteAmmo", "Gameplay", "NPC Ammo Runs Out", "Enemies use up their ammo and switch weapons (or to melee) when they run dry, as the original scripts say. They reload every clip either way."),
            ("FloorConsumables", "Gameplay", "Use Consumables From The Floor", "With no room in your inventory, a medkit, bioelectric cell, food or drink you pick up is used on the spot."),
            ("LadderGrab", "Gameplay", "Grab Ladders", "Use takes hold of a ladder and Jump lets go; on it, forward and back climb up and down."),
            ("NotesAutofill", "Gameplay", "Codes From Notes", "Keypad codes and computer logins you've noted are offered: Tab (pad: Y) types a keypad code or fills in a login."),
            ("DialogueHotkeys", "Interface", "Dialogue Hotkeys", "Number the first four conversation choices; keys 1-4 pick them."),
            ("InventorySort", "Interface", "Inventory Auto-Sort", "A Sort button on the inventory screen packs the items neatly, weapons first."),
            ("InventoryRotate", "Interface", "Item Rotation", "Items can lie turned in the inventory grid: R or right click while dragging (the pad's X) turns one, R turns the selected item in place. A Pack button turns and rearranges everything to free the most room, Shift-click puts an item on the belt, and pickups that only fit turned go in turned. Controller: Y uses / equips the item under the cursor, LT drops it, LB / RB change screens."),
            ("PersistentDebris", "Visuals", "Casings And Blood Stay", "Shell casings never vanish, many more blood splats stay, and blood, bullet holes and casings are still there when you come back to a map or load a save."),
            ("DiegeticScreens", "Interface", "Terminals In The World", "Computers, ATMs and keypads are used where they stand: the view moves in to the screen (the security terminal unfolds first), the original interface shows on the monitor and the mouse works on it; keypads are pressed button by button. Move the mouse off the screen for a second to step away."),
            ("DiscordPresence", "Interface", "Discord Rich Presence", "Show your location, mission and difficulty in Discord."),
            ("BodyAlarms", "AI", "Bodies Raise Alarms", "Guards who find a friend's body - dead or unconscious - run for the nearest alarm and then search the spot; with no alarm about they shout and others come to look."),
            ("SmarterAI", "AI", "Smarter AI", "GMDX's AI work plus our stealth: guards see better by difficulty and may open fire at once when they spot or hear you close by, wait a moment before shooting after losing you, lay down suppressive fire where you vanished, miss more against a moving target, strafe across your aim and pick new firing spots between shots, throw grenades when you're dug in, answer alarms from much farther away; cameras see farther and tell guards where you are. Ours: true darkness hides you beyond arm's reach, guards that searched in vain stay on edge, call each other over, and notice lights going out and doors opening."),
            ("HudFade", "Interface", "Combat HUD", "Out of combat the crosshair and the status displays (health, compass, ammo, belt, active augs) fade away; they come back at once in a fight, when you aim down the sights or at an enemy, fire, get hurt, are badly hurt or change weapons."),
            ("AlertIndicators", "AI", "Alert Indicators", "A small marker over a hostile guard's head: an eye filling as they grow suspicious, a question mark while they search, an exclamation mark in a fight."),
            ("StrongBodyThrow", "Gameplay", "Strong Body Throw", "A body you throw flies like a thrown crate and slides on when it lands. Off, it drops as in the original: half the throw speed, lying the way you face."),
            ("ThrownNoise", "AI", "Thrown Objects Make Noise", "Anything you throw is heard where it lands or bounces - further for heavy things - and armed guards in earshot go to look. Good for distractions."),
            ("SurfaceFootsteps", "AI", "Footsteps Louder On Hard Floors", "Carpet and grass muffle your steps, metal and water give you away; guards hear you by what you walk on."),
            ("WeaponInertia", "Visuals", "Weapon Inertia", "The weapon in your hands trails a little behind when you turn (GMDX's Viewmodel Inertia)."),
            ("ConversationLight", "Visuals", "Conversation Key Light", "A conversation held somewhere dark gets a soft light on the people in it, so you can see their faces. It lights only the people, fades in with the darkness and out when the talk ends, and guards don't see by it."),
            ("ModernGlass", "Visuals", "Modern Glass", "Window panes are frosted glass you can see, with their streaks catching the light, instead of a faint additive smear that leaves an empty-looking hole (the UNATCO HQ doors). Takes effect when the map loads."),
            ("MaterialSheen", "Visuals", "Material Properties", "Weapons and items in your hands catch the light by what they are made of: gun metal shines, plastic less, gloves and skin hardly at all."),
            ("FlashlightCone", "Visuals", "Flashlight Cone", "The flashlight augmentation casts a soft-edged cone with shadows from your shoulder instead of a pool of light where you look."),
            ("BulletPenetration", "Gameplay", "Bullets Through Thin Walls", "Bullets go on through thin wood (walls, doors) and breakable glass, doing less damage on the other side. Bulletproof glass stays bulletproof."),
            ("SmoothLean", "Gameplay", "Smooth Leaning", "Leaning eases in and out, works while crouch-walking, and at a corner your eyes lean on past where your body stops so you can peek."),
            ("MeleeArc", "Gameplay", "Melee Sweeps An Arc", "Knives, batons, prods and swords also hit whatever stands just to either side of your aim, at three quarters damage."),
            ("DTSEnergy", "Gameplay", "Dragon's Tooth Uses Bioenergy", "Each swing of the Dragon's Tooth sword costs 5 bioelectric energy; with none left the blade goes dark and won't swing (after GMDX)."),
            // GMDX bugfixes.
            ("FixSkillAwardOnce", "Fixes", "Skill Awards Pay Once", "A one-time skill point award set off by an event pays only once; in the original a repeated event paid again (GMDX / Shifter fix)."),
            ("FixTurretAnimals", "Fixes", "Turrets Ignore Animals", "Gun turrets don't waste their fire on rats, birds and dogs (GMDX fix)."),
            ("FixBlastDoors", "Fixes", "Explosions Break Strong Doors", "An explosion is dealt in five rings, so in the original no ring of even a big blast could beat a sturdy door's damage threshold. With this, a blast strong enough as a whole damages the door (GMDX fix; changes the balance)."),
            ("FixGibLoot", "Fixes", "Gibbed Bodies Drop Their Weapons", "Someone blown apart throws out the weapons they carried instead of losing them (GMDX / G-Flex fix; changes the balance)."),
            ("FixCarryJump", "Fixes", "Muscle Aug Jumps While Carrying", "With the Microfibral Muscle aug on you can jump while carrying heavier things, as the aug was meant to allow (GMDX fix; changes the balance)."),
            ("OptionalFixes", "Fixes", "Optional Fixes", "Dozens of the original's bugs fixed: misnamed mission flags (the Hong Kong ROM, the ship breach, the Vandenberg hazlab, Aimee's goal, Jock in Area 51), the ATM account-0 login, EMP'd robots taken as enemies, bot screens left open, the laser left on in conversations, the riot prod breaking crates, burning people left unconscious, barks under water, leather chair colours and more."),
            ("UnconsciousDeath", "Gameplay", "Unconscious Bodies Can Die", "Someone knocked out can still be killed: an explosion, fire or gas that reaches the body finishes them (changes the balance)."),
            ("NoisyObjects", "AI", "Using Things Makes Noise", "Doors, buttons, switches, lamps, taps and machines you use make a noise guards nearby hear."),
            ("WeaponPointing", "AI", "People Mind A Pointed Gun", "Friendly and neutral people you point a drawn weapon at, close up and in their view, complain."),
            ("RadarCloak", "Gameplay", "Thermoptic Camo Fools Machines", "Thermoptic camo also hides you from cameras, turrets, laser beams and mines, as Radar Transparency does; Cloak and Radar Transparency together get you through beams (changes the balance)."),
            ("NpcLimp", "AI", "Wounded Legs Slow People", "People with a crippled leg walk at 85% speed, with both legs crippled 70%."),
            ("BossResist", "Gameplay", "Stronger Gunther And Simons", "Gunther and Walton Simons shrug off tear gas and halon and take a third of explosive damage (changes the balance)."),
            ("LongFlares", "Gameplay", "Long-Burning Flares", "Flares burn for six minutes instead of thirty seconds."),
            // GMDX quality of life.
            ("ToolCounts", "Interface", "Tools Owned / Needed", "Locks and devices show how many lockpicks or multitools you have against how many they need, as 2/3 picks (GMDX)."),
            ("ItemCounts", "Interface", "Item Count Labels", "A stack of items lying about shows how many there are, as Medkit (3) (GMDX)."),
            ("DuplicateKeys", "Interface", "Show Duplicate Keys", "A nano key you already have on the ring is marked (Duplicate) (GMDX)."),
            ("SearchedText", "Interface", "Mark Searched Bodies", "Bodies you've already searched say [Searched] (GMDX)."),
            ("AmmoTotal", "Interface", "Ammo Display Shows Rounds", "The ammo display counts the rounds left instead of whole clips (GMDX's Accurate Ammo Display)."),
            ("AmmoMaxCyan", "Interface", "Show Full Ammo", "The ammo count turns cyan when you carry all you can of that ammo (GMDX)."),
            ("ShowMaxStacks", "Interface", "Show Stack Limits", "Inventory items show how many more you can carry, as Count: 3/15 (GMDX)."),
            ("PickupSize", "Interface", "No Room Message Shows Size", "When something won't fit, the message says how much room it needs, as [2x3] (GMDX)."),
            ("StreamlinedRepairBot", "Interface", "Streamlined Repair Bots", "A repair bot that is still recharging says how long it will be instead of opening its screen (GMDX)."),
            ("FasterInfolinks", "Interface", "Faster Infolinks", "Infolinks waiting their turn follow straight on, without the pause between them (GMDX)."),
            ("DropStacks", "Interface", "Drop Whole Stacks", "Shift + Drop on the inventory screen drops the whole stack at once (GMDX)."),
            ("SmartBinoculars", "Gameplay", "Scope Key Takes Out Binoculars", "With nothing in your hands, the scope key takes out the binoculars (GMDX's Smart Binocular Selection)."),
            ("HQAudio", "Sound", "Better Sound Playback", "Smoother playback of the game's sounds (finer resampling, even panning), and the rest of the sound steps back a little while someone is speaking."),
            ("SoundOcclusion", "Sound", "Sound Through Walls", "Sounds behind a wall are quieter and muffled; ones you can hear around a corner or through a doorway only a little."),
            ("RoomReverb", "Sound", "Room Reverb", "An echo shaped by where you are: long in big halls, short in corridors, faint outdoors, dull underwater."),
            ("Soundscape", "Sound", "Soundscape", "Far-off life around the open-air maps - dogs, gulls by the water, gusts of wind (more in the rain) - now and then, quietly, from the game's own sounds."),
            ("RestrictedCombatMusic", "Sound", "Combat Music Needs Two Attackers", "The combat music only starts when at least two enemies are fighting you, so one far-off guard doesn't set it off (GMDX)."),
            ("DiegeticScope", "Gameplay", "Diegetic Scope", "A zoomed rifle scope is a live lens in front of the eye instead of a black screen with a picture: the world stays in view around it, the reticle sits in the glass, and an untrained eye watches the picture wander under it."),
            ("ModernAim", "Gameplay", "Modern Aiming", "Shots go where your sights drift instead of scattering at random, and the crosshair follows that point; the accuracy brackets always show. Hold the scope key (Z or LT) with a weapon that has no scope to aim down the sights: steadier, a little zoom, walking pace."),
            ("SkeletalCharacters", "Visuals", "Skeletal Characters", "People get rebuilt bodies on a real skeleton (by the Machine God team): smooth joints and blending between moves, a face that lip-syncs, blinks and glances around. They keep the skins the original gives them. Characters the HD mods replace stay as they are."),
            ("Ragdolls", "Gameplay", "Ragdolls", "With Skeletal Characters on, people go limp the moment they're killed or knocked out, thrown by the hit and carried by how they were moving, and fall against whatever is around them. Explosions still blow them apart."),
            ("FluidBlood", "Visuals", "Fluid Blood", "Hits throw droplets that fly and splatter where they land; on walls the blood runs down. The dead bleed out into a spreading pool, and explosions throw blood over everything near."),
            ("BodyWounds", "Visuals", "Wounds On Bodies", "With Skeletal Characters on, shots leave wound marks on people where they went in, and the marks move with them."),
            ("HurtPoses", "Gameplay", "Hurt Poses", "With Skeletal Characters on, the wounded move like it: a badly hurt arm hangs, a hurt leg stiffens into a limp. JC's own body too."),
            ("FirstPersonBody", "Visuals", "First-Person Body", "With Skeletal Characters on, look down and see JC's body and legs under you; his arms show while a medkit is in hand."),
            ("DiegeticHealing", "Gameplay", "Diegetic Healing", "With First-Person Body on: hold a medkit, look at the arm, leg or chest you want to treat and use it there. Used looking anywhere else, it treats the head. A label under the crosshair says which part."),
            ("AugGlows", "Visuals", "Aug Glows", "Running augmentations glow on JC's body where they sit: eyes, arms, legs, chest (in mirrors, third person and the first-person body)."),
            ("CoatCloth", "Visuals", "Coat Cloth", "With Skeletal Characters on, long coats swing and trail with the body's motion and fall around the legs instead of moving as one stiff piece."),
            ("EnhancedFX", "Visuals", "Enhanced Effects", "More particles: sparks off metal, dust off stone, splinters off wood and splashes off water where bullets hit; spark showers and rolling smoke from explosions; smoke from gun barrels; a spray of blood on hits."),
            ("Elements", "Gameplay", "Elemental Reactions", "Electric shocks run through water to everyone standing in it (you too), people on fire set others they touch alight, and fire caught by someone in water goes out."),
            ("Sprint", "Gameplay", "Sprint and Stamina", "Your old running pace becomes your normal pace; hold Walk (Shift) or click the left stick to sprint faster. Sprinting uses stamina, shown on the right of the body display: it comes back slowly once you stop, and food tops it up. No sprinting while crouched, carrying, swimming or in the air."),
            ("Mantling", "Gameplay", "Mantling", "Jump at a ledge you can reach - up to about the height of your arms - and you climb onto it; jump at a low obstacle and you vault onto it. Not while crouched or carrying anything heavy."),
            ("CarryCover", "Gameplay", "Carried Objects As Cover", "A crate or anything else of some weight held in front of you stops the bullets meant for you, and takes the damage itself (it can break)."),
            ("ThirdPerson", "Gameplay", "Third Person View", "H switches between first person and a view of JC from behind (J swaps the side the camera sits on). The camera keeps out of walls and comes in closer while you aim with the scope key; first person is where you start. See Third Person Camera for how it turns."),
            ("ThirdPersonCover", "Gameplay", "Third-Person Cover", "In third person, leaning (Q / E) swaps the camera to that shoulder and peeks round the edge of the wall or corner beside you; your shots leave from where you peek. JC's body leans with it, crouched or standing."),
            ("MeleeBash", "Gameplay", "Melee Bash", "F swings the gun in your hands as a club: a short blow that can knock a guard out (heavier guns hit harder). Not while scoped, reloading or firing."),
            ("SmoothAnimation", "Visuals", "Smooth Animation", "People, JC's body and the weapon in your hands blend from one move to the next instead of snapping, and turn smoothly. Shots and blows still land on the original's frames."),
            ("PosableBodies", "Gameplay", "Posable Bodies", "With Ragdolls on, Use on a body's arm, leg or head takes hold of it and drags that limb along with your view; Use again lets go and the body settles in its new pose. Use on the chest or hips searches and picks up the body as before."),
            ("AtmosphericFog", "Visuals", "Atmospheric Fog", "A thin night haze over distance, thicker in the rain, so far-off buildings and the skyline melt into the dark."),
            ("ModernCoronas", "Visuals", "Modern Coronas", "Lamps and lights get a soft bloom in the light's colour round a small bright core, sized to the lamp so it shrinks with distance, fading in and out smoothly, instead of the original's corona pictures."),
            ("LightHalos", "Visuals", "Light Halos", "The air around lamps glows softly with their colour, fading as you get close; brighter in the rain."),
            ("Skyline", "Visuals", "3D Skyline", "The city beyond the map in 3D: where the sky has the original skyline photos (New York) they are cut into low-poly buildings at their own distances, so the picture is the same but shifts as you move; elsewhere procedural blocks - Hong Kong's dense towers, Paris's low roofs - with lit windows and roof lights."),
            ("Bloom", "Visuals", "Glow And Bloom", "Glowing things - camera lenses and alarm lights, laser tripwires, nano canisters, JC's sunglasses - are brighter than white and bleed a soft glow into the dark around them."),
            ("ModernVision", "Visuals", "Modern Vision Augmentation", "The Vision Enhancement aug and night-vision goggles amplify light like real night vision: green grain, bright lights flare and blow out, and at the aug's higher levels warm bodies glow through the dark."),
            ("AmbientOcclusion", "Visuals", "Ambient Occlusion", "Corners, crevices, grates and the ground under crates and furniture darken a little (screen-space ambient occlusion), so things sit in the world instead of floating on it. Turns off MSAA (SMAA smooths the edges instead)."),
            ("SoftShadows", "Visuals", "Soft Shadows", "Real-time shadows are sharp where an object meets the ground and soften as they stretch away from it (contact hardening), the way shadows from a lamp do. Needs Real-Time Shadows."),
            ("LightShafts", "Visuals", "Light Shafts", "Beams of light in the air: the flashlight's cone, spotlights and the moonlight through windows and grates show in the haze."),
            ("FogVolumes", "Visuals", "Local Fog", "Low mist lying over the water in sewers and other indoor waterways, and a smoky haze in the bars and clubs that catches the light."),
            ("Clouds", "Visuals", "Night Clouds", "Volumetric clouds drifting over the open-air maps, lit by the moon and the city; in the rain they close into a dark overcast with lightning and thunder."),
            ("ModernWater", "Visuals", "Modern Water", "Water with real reflections of what's around it, a dark see-through body, fine wind waves, rain rings and glints from the lights - like a modern shader pack. See Water Reflections for how the reflections are made."),
            ("OpenSea", "Visuals", "Open Sea", "Without Modern Water: the sea runs on past the edge of the map to the horizon, in the original water's look, instead of fading into black before the walls at the map's edge. Costs almost nothing. Takes effect on the next map."),
            ("NoStrobeLights", "Visuals", "No Flickering Lights", "Flickering and blinking lights burn steady (GMDX's Lighting Accessibility, for photosensitive players)."),
        ] {
            let mut c = debug_mode_choice();
            c.key = key.into();
            c.label = lbl.into();
            c.help = help.into();
            c.value = crate::settings::get(key);
            c.saved = c.value;
            items.push(tag(c, group));
        }
    }
    /// Mod switches (while the mod is on in the launcher), Augmentique's people / outfits, the Precipitation rain.
    fn mod_items(items: &mut Vec<Choice>) {
        // Mod switches, listed while the mod is on in the launcher.
        for (key, id, lbl, help) in [
            ("OutfitBoxes", "augmentique", "Outfit Boxes In The Original Maps", "Without GMDX's maps, Augmentique's outfits are hidden in boxes across the campaign - a few in a map, by a clothes rack, a couch or a chair - instead of all being open. Takes effect on the next start."),
            ("HDTextures", "newvision", "HD Textures (New Vision)", "New Vision's high-resolution textures on the world and on models. Changes take effect when the next map loads."),
            ("HDModels", "hdtp", "HD Items And Objects (HDTP)", "HDTP's detailed weapons, pickups and decorations. Changes take effect when the next map loads."),
            ("HDCharacters", "hdtp", "HD Characters (HDTP)", "HDTP's detailed people and their faces (not used while Helios dresses the people). Changes take effect when the next map loads."),
            ("DdsCharacters", "helios", "Helios Characters", "Helios's high-resolution textures on the original people, robots and animals, in place of HDTP's characters. Changes take effect when the next map loads."),
            ("DdsItems", "helios", "Helios Items And Decorations", "Helios's textures on items and decorations that keep their original models (HDTP's models keep their own). Changes take effect when the next map loads."),
            ("DdsWorld", "helios", "DDS World Textures", "Other DDS texture packs found beside Helios for walls, floors and the rest of the world (used in place of New Vision's where both have one). Changes take effect when the next map loads."),
            ("VisAttach", "visibleattachments", "Weapon Mods Shown (Visible Attachments)", "Scopes, silencers and lasers you fit show on the weapon. Changes take effect when the next map loads."),
        ] {
            if unreal_pkg::mods::active(id).is_none() {
                continue;
            }
            let mut c = debug_mode_choice();
            c.key = key.into();
            c.label = lbl.into();
            c.help = help.into();
            c.value = crate::settings::get(key);
            c.saved = c.value;
            items.push(tag(c, "Mods"));
        }
        if crate::augmentique::active() {
            let mut c = debug_mode_choice();
            c.key = "AugNPCs".into();
            c.label = "Outfits On People (Augmentique)".into();
            c.help = "People in the world wear random outfits: none, everyone but the main characters, or everyone. Takes effect when the next map loads.".into();
            c.kind = ChoiceKind::Enum(vec!["None".into(), "Generic People".into(), "Everyone".into()]);
            c.value = setting!("AugNPCs");
            c.saved = c.value;
            items.push(tag(c, "Mods"));
            let mut c = debug_mode_choice();
            c.key = "AugAllOutfits".into();
            c.label = "All Outfits Unlocked (Augmentique)".into();
            c.help = "Every outfit is in the Outfits screen from the start, instead of found in boxes in GMDX Augmented's maps. Takes effect on the next start.".into();
            c.value = setting!("AugAllOutfits");
            c.saved = c.value;
            items.push(tag(c, "Mods"));
        }
        if crate::weather::available() {
            let mut c = debug_mode_choice();
            c.key = "Weather".into();
            c.label = "Rain (Precipitation)".into();
            c.help = "Rain on the maps where it fits - New York, Hong Kong, the docks, a little over Paris - falling wherever the sky is open, splashing, with its sound. Dynamic lets it come and go.".into();
            c.kind = ChoiceKind::Enum(vec!["Off".into(), "Steady".into(), "Dynamic".into()]);
            c.value = setting!("Weather");
            c.saved = c.value;
            items.push(tag(c, "Visuals"));
        }
    }
    /// Settings with their own values: third person camera, FOV, frame cap, water reflections, worker threads,
    /// grass and trees, upscaling, shadows.
    fn option_items(items: &mut Vec<Choice>) {
        {
            let mut c = debug_mode_choice();
            c.key = "ThirdPersonCamera".into();
            c.label = "Third Person Camera".into();
            c.help = "Free: the camera turns around JC with the mouse or right stick, he runs the way you push and turns to face it, faces where the camera looks while you aim or shoot, and fidgets when left alone. Shoulder: a fixed view over his shoulder that turns with him.".into();
            c.kind = ChoiceKind::Enum(vec!["Free".into(), "Shoulder".into()]);
            c.value = setting!("ThirdPersonCamera");
            c.saved = c.value;
            items.push(tag(c, "Gameplay"));
        }
        {
            let mut c = debug_mode_choice();
            c.key = "FOV".into();
            c.label = "Field of View".into();
            c.help = "How wide you see, in degrees across the screen (the original's DefaultFOV; 75 as it shipped, made for 4:3). Wider suits widescreen and ultrawide monitors. The weapon in your hands keeps its own view, so it never stretches or shows its cut-off ends; scopes and binoculars zoom as before.".into();
            let vals: Vec<i32> = (60..=130).step_by(5).collect();
            c.kind = ChoiceKind::Slider {
                ticks: vals.len() as i32,
                start: 60.0,
                end: 130.0,
                labels: vals.iter().map(|v| if *v == 75 { "75 (original)".to_string() } else { v.to_string() }).collect(),
            };
            c.value = crate::weapons::base_fov();
            c.saved = c.value;
            items.push(tag(c, "Display"));
        }
        {
            let mut c = debug_mode_choice();
            c.key = "FPSLimit".into();
            c.label = "Frame Rate Cap".into();
            c.help = "The most frames a second the game draws. A cap keeps the frame rate steady and the graphics card cooler and quieter; Off lets it run as fast as it can (with Vertical Sync on, the monitor's refresh rate caps it anyway).".into();
            c.kind = ChoiceKind::Enum(crate::framecap::LABELS.iter().map(|s| s.to_string()).collect());
            c.value = setting!("FPSLimit");
            c.saved = c.value;
            items.push(tag(c, "Display"));
        }
        {
            let mut c = debug_mode_choice();
            c.key = "WaterReflections".into();
            c.label = "Water Reflections".into();
            c.help = "How Modern Water mirrors its surroundings. Screen-Space reflects what is on screen, worked out on the graphics card from the picture already drawn: nearly free for the processor. Planar draws the whole scene a second time from below the water for exact reflections of things off screen too - sharper, but it costs the processor a second view whenever water is in sight.".into();
            c.kind = ChoiceKind::Enum(vec!["Screen-Space".to_string(), "Planar".to_string()]);
            c.value = setting!("WaterReflections");
            c.saved = c.value;
            items.push(tag(c, "Visuals"));
        }
        {
            let mut c = debug_mode_choice();
            c.key = "WorkerThreads".into();
            c.label = "CPU Worker Threads".into();
            c.help = "How many threads the game spreads its parallel work over. Auto uses one per processor core (half the hardware threads) and stops at 8, leaving the main and render threads their own cores: more threads only add overhead. Takes effect on the next start.".into();
            c.kind = ChoiceKind::Enum(crate::threads::STEPS.iter().map(|&n| if n == 0 { "Auto".to_string() } else { n.to_string() }).collect());
            c.value = crate::threads::step_index(setting!("WorkerThreads"));
            c.saved = c.value;
            items.push(tag(c, "Display"));
        }
        {
            let mut c = debug_mode_choice();
            c.key = "Grass".into();
            items.push(tag({
                let mut t = debug_mode_choice();
                t.key = "Trees".into();
                t.label = "Grown Trees".into();
                t.help = "The parks' flat card trees and bushes are replaced with grown ones: real branches and bark, clusters of leaves, swaying in the wind and casting shadows. Same size and place as the originals.".into();
                t.value = setting!("Trees");
                t.saved = t.value;
                t
            }, "Visuals"));
            c.label = "3D Grass".into();
            c.help = "Blades of grass grow out of the lawns, lit like the ground they stand on, swaying in the wind and pushed aside by whoever walks through them. Low: fewer blades and a shorter reach.".into();
            c.kind = ChoiceKind::Enum(vec!["Off".into(), "Low".into(), "High".into()]);
            c.value = setting!("Grass");
            c.saved = c.value;
            items.push(tag(c, "Visuals"));
        }
        {
            let mut c = debug_mode_choice();
            c.key = "Upscaling".into();
            c.label = "Upscaling".into();
            c.help = "Native AA smooths edges with temporal antialiasing at full resolution. Quality, Balanced, Performance and Ultra Performance draw the world at 1/1.5, 1/1.7, 1/2 or 1/3 the window's size and upscale it with an edge-aware filter (AMD FSR 1) and sharpening - more frames on slower graphics cards. The weapon, HUD and menus always stay at full resolution.".into();
            if cfg!(target_os = "macos") {
                c.help.push_str(" On a Mac there is no temporal antialiasing: Native AA only sharpens.");
            }
            c.kind = ChoiceKind::Enum(crate::upscale::NAMES.iter().map(|s| s.to_string()).collect());
            c.value = setting!("Upscaling");
            c.saved = c.value;
            items.push(tag(c, "Display"));
        }
        {
            let mut c = debug_mode_choice();
            c.key = "RealtimeShadows".into();
            c.label = "Real-Time Shadows".into();
            c.help = "People and objects cast moving shadows from the lamps around them, as dark as the light they block. Low uses the two strongest lights near you, High four.".into();
            c.kind = ChoiceKind::Enum(vec!["Off".into(), "Low".into(), "High".into()]);
            c.value = setting!("RealtimeShadows");
            c.saved = c.value;
            items.push(tag(c, "Visuals"));
        }
    }
    /// Performance switches (perf.rs) and Remastered Lighting (relight.rs).
    fn performance_items(items: &mut Vec<Choice>) {
        // Performance (perf.rs): by default 4x MSAA and HDR are on and the lean switches off.
        for (key, lbl, help) in [
            ("AntiAliasing", "Anti-Aliasing (4x MSAA)", "Smooths the edges of the world and the weapon with 4x multisampling. Off saves a lot of GPU time and memory on slow GPUs. (With Ambient Occlusion on, SMAA smooths the edges instead; with Upscaling, its own antialiasing.)"),
            ("HDRRendering", "HDR Rendering", "Draws the scene in 16-bit colour before it reaches the screen. Off draws it in 8-bit, half the memory traffic; the picture is the same except for faint differences where bright things show through glass. Glow And Bloom turns it back on."),
            ("LeanRendering", "Lean Rendering", "Leaves out GPU work that adds nothing to the picture, for slow GPUs. Dynamic lights (muzzle flashes, fires, flares, the flashlight) light the walls and floors as in the original game, with its broad falloff and no highlight."),
            ("LeanUpdates", "Lean Updates", "Leaves out CPU work that changes nothing on screen, such as redrawing windows that haven't changed. For slow CPUs."),
            ("FPSCounter", "FPS Counter", "Shows frames per second and the average frame time in the top-right corner, over each half second."),
        ] {
            let mut c = debug_mode_choice();
            c.key = key.into();
            c.label = lbl.into();
            c.help = help.into();
            c.value = crate::settings::get(key);
            c.saved = c.value;
            items.push(tag(c, "Performance"));
        }
        // Remastered lighting (relight.rs): applied when the map next loads.
        for (key, label, help, values) in [
            ("RemasteredLighting", "Remastered Lighting", "Every wall and floor lit per pixel by the lamps that reach it, with real falloff and shape, and the original's shadows kept. Needed for Relief Maps and Material Highlights. Takes effect on the next map.", vec!["Off", "On"]),
            ("ReliefMaps", "Relief Maps", "With Remastered Lighting: bricks, planks, tiles and grates catch the light with normal maps - the Parallax Occlusion addon's where it has them (shipped with the game), the rest made from each texture. Parallax adds depth to them as you move. Takes effect on the next map.", vec!["Off", "Normal Maps", "Normal + Parallax"]),
            ("MaterialSpecular", "Material Highlights", "With Remastered Lighting: metal, glass, tiles and polished floors shine where the light catches them. Takes effect on the next map.", vec!["Off", "On"]),
            ("EmissiveSurfaces", "Glowing Surfaces", "Light fittings, screens and signs glow past white (with Bloom, a soft halo) and cast their own colour on the walls, floors and people around them. Takes effect on the next map.", vec!["Off", "On"]),
        ] {
            let mut c = debug_mode_choice();
            c.key = key.into();
            c.label = label.into();
            c.help = help.into();
            c.kind = ChoiceKind::Enum(values.iter().map(|v| v.to_string()).collect());
            c.value = crate::settings::get(key);
            c.saved = c.value;
            items.push(tag(c, "Visuals"));
        }
    }
    /// Touch controls and gyro aiming (Android).
    fn touch_items(items: &mut Vec<Choice>) {
        for (key, label, help, values) in [
            ("TouchLook", "Touch Look Speed", "How far the view turns for a drag on the right half of the screen.", vec!["Slowest", "Slow", "Normal", "Fast", "Fastest"]),
            ("TouchSize", "Touch Button Size", "The size of the on-screen buttons.", vec!["Small", "Normal", "Large"]),
            ("TouchOpacity", "Touch Button Opacity", "How solid the on-screen buttons and the move stick are drawn.", vec!["25%", "50%", "70%", "90%"]),
            ("TouchVibrate", "Vibrate On Button Press", "A short buzz when a finger lands on an on-screen button.", vec!["Off", "On"]),
            ("GyroAim", "Gyro Aim", "Tilt the phone to fine-tune your aim; dragging still turns the view.", vec!["Off", "On"]),
            ("GyroSensitivity", "Gyro Speed", "How far the view turns for a tilt of the phone.", vec!["Slowest", "Slow", "Normal", "Fast", "Fastest"]),
        ] {
            let mut c = debug_mode_choice();
            c.key = key.into();
            c.label = label.into();
            c.help = help.into();
            c.kind = ChoiceKind::Enum(values.iter().map(|v| v.to_string()).collect());
            c.value = crate::settings::get(key);
            c.saved = c.value;
            items.push(tag(c, "Touch"));
        }
    }
    /// HUD parts (hidden / size / opacity), Smooth Lip Sync, and Debug Mode last.
    fn hud_items(items: &mut Vec<Choice>) {
        // HUD customisation: each part shown or hidden, sized, faded.
        for (key, name) in crate::classic_hud::HUD_PARTS {
            let mut c = debug_mode_choice();
            c.key = format!("HUD_{key}_Hide");
            c.label = format!("{name}: Hidden");
            c.help = format!("Hide the {} on the HUD.", name.to_ascii_lowercase());
            c.value = crate::settings::get(&c.key);
            c.saved = c.value;
            items.push(tag(c, "HUD"));
            let mut c = debug_mode_choice();
            c.key = format!("HUD_{key}_Size");
            c.label = format!("{name}: Size");
            c.help = format!("The {}'s size on screen; it stays in its corner.", name.to_ascii_lowercase());
            c.kind = ChoiceKind::Enum(crate::classic_hud::HUD_SIZES.iter().map(|v| v.0.to_string()).collect());
            c.value = crate::settings::get(&c.key);
            c.saved = c.value;
            items.push(tag(c, "HUD"));
            let mut c = debug_mode_choice();
            c.key = format!("HUD_{key}_Fade");
            c.label = format!("{name}: Opacity");
            c.help = format!("How see-through the {} is.", name.to_ascii_lowercase());
            c.kind = ChoiceKind::Enum(crate::classic_hud::HUD_FADES.iter().map(|v| v.0.to_string()).collect());
            c.value = crate::settings::get(&c.key);
            c.saved = c.value;
            items.push(tag(c, "HUD"));
        }
        let mut lip = debug_mode_choice();
        lip.key = "SmoothLipSync".into();
        lip.label = "Smooth |&Lip Sync".into();
        lip.help = "Mouths blend smoothly between sounds and close gently, and people blink visibly (after Joe Wintergreen's lipsync mod).".into();
        lip.value = setting!("SmoothLipSync");
        lip.saved = lip.value;
        items.push(tag(lip, "Visuals"));
        items.push(tag(debug_mode_choice(), "Developer"));
    }
    /// The items the search keeps (every word in the label or help), in order.
    /// The groups in list order.
    fn groups(&self) -> Vec<String> {
        let mut g: Vec<String> = Vec::new();
        for c in &self.items {
            let n = split_group(&c.label).0;
            if !g.iter().any(|x| x == n) {
                g.push(n.to_string());
            }
        }
        g
    }
    /// The rows on show: with a search, every matching item (flat); without, each group's header and the items of
    /// the open groups.
    fn shown(&self) -> Vec<Row> {
        let q = self.search.get().to_ascii_lowercase();
        let words: Vec<&str> = q.split_whitespace().collect();
        if !words.is_empty() {
            return (0..self.items.len())
                .filter(|&i| {
                    let c = &self.items[i];
                    let hay = format!("{} {}", c.label, c.help).to_ascii_lowercase();
                    words.iter().all(|w| hay.contains(w))
                })
                .map(Row::Item)
                .collect();
        }
        // (Android is Classic only: no master switch.)
        let mut rows = if crate::platform::android() { Vec::new() } else { vec![Row::Master] };
        for (gi, g) in self.groups().iter().enumerate() {
            rows.push(Row::Group(gi));
            if self.open.contains(g) {
                rows.extend((0..self.items.len()).filter(|&i| split_group(&self.items[i].label).0 == g).map(Row::Item));
            }
        }
        rows
    }
    fn searching(&self) -> bool {
        !self.search.get().trim().is_empty()
    }
    /// The text of a row: (label, value).
    fn row_text(&self, r: Row) -> (String, String) {
        match r {
            Row::Master => ("CLASSIC / MODERN (Tab in game)".into(), if self.modern { "Modern".into() } else { "Classic".into() }),
            Row::Group(g) => {
                let name = self.groups().get(g).cloned().unwrap_or_default();
                let n = self.items.iter().filter(|c| split_group(&c.label).0 == name).count();
                let sign = if self.open.contains(&name) { "-" } else { "+" };
                (format!("{sign} {name}"), format!("{n} setting{}", if n == 1 { "" } else { "s" }))
            }
            Row::Item(i) => {
                let c = &self.items[i];
                let l = if self.searching() { c.label.clone() } else { format!("    {}", split_group(&c.label).1) };
                (l, Enhance::value_text(c))
            }
        }
    }
    fn value_text(c: &Choice) -> String {
        match &c.kind {
            ChoiceKind::Action { .. } => "...".into(),
            ChoiceKind::Slider { ticks, .. } if c.value_text().contains('.') => format!("{} / {}", c.slider_tick() + 1, ticks),
            _ => c.value_text(),
        }
    }
    /// Change the selected item a step (an action item opens its screen instead).
    fn step(&mut self, dir: i32, acts: &mut Vec<Act>) {
        let Some(row) = self.sel else { return };
        let i = match row {
            Row::Master => {
                self.flip_mode();
                return;
            }
            Row::Group(g) => {
                // Enter / click / Right opens a group, Left closes it.
                if let Some(name) = self.groups().get(g).cloned() {
                    if dir < 0 {
                        self.open.remove(&name);
                    } else if !self.open.remove(&name) {
                        self.open.insert(name);
                    }
                }
                return;
            }
            Row::Item(i) => i,
        };
        let c = &mut self.items[i];
        match &c.kind {
            ChoiceKind::Action { invoke, .. } if !invoke.is_empty() => acts.push(Act::Push(invoke.clone())),
            ChoiceKind::Action { .. } => {}
            _ => {
                c.cycle(dir);
                // The original screens' settings that take effect at once (volumes, brightness, themes, the
                // window).
                live_preview(c);
                if let Some(a) = choice_side_effect(c) {
                    acts.push(a);
                }
            }
        }
    }
    /// The Classic / Modern row: every covered row switches (applied with the rest on OK).
    fn flip_mode(&mut self) {
        let to_modern = !self.modern;
        let items = &self.items;
        let current = |k: &str| items.iter().find(|c| c.key == k).map_or_else(|| crate::settings::get(k), |c| c.value);
        let (mut vals, memory) = crate::remaster::switch_values(to_modern, current);
        if to_modern && !self.memory.is_empty() {
            // Back to Modern before OK: the values just put away.
            for (k, v) in vals.iter_mut() {
                if let Some(m) = self.memory.iter().find(|m| m.0 == format!("Modern.{k}")) {
                    *v = m.1;
                }
            }
        }
        if !to_modern {
            self.memory = memory;
        }
        for (k, v) in vals {
            if let Some(c) = self.items.iter_mut().find(|c| c.key == k) {
                c.value = v;
            } else {
                // Not in the list (a mod's setting that isn't shown): still switched.
                self.memory.push((k, v));
            }
        }
        self.modern = to_modern;
        self.mode_changed = true;
        crate::sound::play_player(
            if to_modern { "DeusExSounds.Augmentation.CloakUp" } else { "DeusExSounds.Augmentation.CloakDown" },
            0.7,
            0.0,
        );
    }
    /// Move the selection among the shown items, keeping it on screen.
    fn move_sel(&mut self, d: i32) {
        let shown = self.shown();
        if shown.is_empty() {
            return;
        }
        let at = self.sel.and_then(|s| shown.iter().position(|&i| i == s)).map_or(0, |p| (p as i32 + d).clamp(0, shown.len() as i32 - 1));
        let at = at.clamp(0, shown.len() as i32 - 1);
        self.sel = Some(shown[at as usize]);
        let y = at * ROW_H;
        if y < self.scroll {
            self.scroll = y;
        } else if y + ROW_H > self.scroll + 268 {
            self.scroll = y + ROW_H - 268;
        }
    }
}

/// HUDKeypadWindow (103 x 162): twelve HUDKeypadButtons (1-9, *, 0, #), the code display, ~~GRANTED~~ /
/// ~~DENIED~~ for a second.
struct KeypadWin {
    blocker: usize,
    valid: String,
    input: String,
    /// KeypadDelay: seconds left showing the result, and whether it was right.
    wait: Option<(f32, bool)>,
    /// bInstantSuccess (bypassed with a multitool): the code goes in by itself on the first frame.
    instant: bool,
    /// Ours (Enhancement "Codes From Notes"): the code is in one of the player's notes; Tab enters it.
    noted: bool,
}

/// Ours: a code or password appears in one of the player's notes as a whole word.
pub(crate) fn in_notes(code: &str) -> bool {
    if code.is_empty() || setting!("NotesAutofill") < 0.5 {
        return false;
    }
    let Some(st) = crate::consys::lock().as_ref().map(|s| s.notes.clone()) else { return false };
    let want = code.to_ascii_lowercase();
    st.iter().any(|n| {
        let n = n.to_ascii_lowercase();
        n.match_indices(&want).any(|(i, _)| {
            let before = n[..i].chars().next_back();
            let after = n[i + want.len()..].chars().next();
            !before.is_some_and(|c| c.is_ascii_alphanumeric()) && !after.is_some_and(|c| c.is_ascii_alphanumeric())
        })
    })
}

static KEYPAD_OPEN: std::sync::Mutex<Option<(usize, String, bool)>> = std::sync::Mutex::new(None);
static KEYPAD_RESULTS: std::sync::Mutex<Vec<(usize, bool)>> = std::sync::Mutex::new(Vec::new());

/// Keypad.ActivateKeypadWindow (devices.rs).
pub fn open_keypad(blocker: usize, valid: String, instant: bool) {
    if let Ok(mut k) = KEYPAD_OPEN.lock() {
        *k = Some((blocker, valid, instant));
    }
}

/// The keypad in use (its blocker), its code so far and the result showing (diegetic.rs).
static KEYPAD_NOW: std::sync::Mutex<Option<(usize, String, Option<bool>, usize)>> = std::sync::Mutex::new(None);
static KEYPAD_PRESSES: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());

/// The open keypad's code is in the notes (Codes From Notes): its "Tab / Y: code from notes" hint is wanted.
pub fn keypad_noted() -> bool {
    KEYPAD_NOTED.load(std::sync::atomic::Ordering::Relaxed)
}
static KEYPAD_NOTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn keypad_blocker() -> Option<usize> {
    KEYPAD_NOW.lock().ok().and_then(|k| k.as_ref().map(|k| k.0))
}

/// (code typed, result shown: Some(right?) during KeypadDelay, the code's length).
pub fn keypad_state() -> Option<(String, Option<bool>, usize)> {
    KEYPAD_NOW.lock().ok().and_then(|k| k.as_ref().map(|k| (k.1.clone(), k.2, k.3)))
}

/// A button pressed on the keypad in the world (0..8 = 1..9, 9 = *, 10 = 0, 11 = #).
pub fn keypad_press_index(i: usize) {
    if let Ok(mut q) = KEYPAD_PRESSES.lock() {
        q.push(i);
    }
}

/// Codes entered: (keypad blocker, right?), for devices.rs to run the keypad's events.
pub fn take_keypad_results() -> Vec<(usize, bool)> {
    KEYPAD_RESULTS.lock().map(|mut r| std::mem::take(&mut *r)).unwrap_or_default()
}

/// HUDKeypadWindow.IndexToString.
fn key_char(i: usize) -> char {
    match i {
        9 => '*',
        10 => '0',
        11 => '#',
        n => char::from(b'1' + n as u8),
    }
}

/// HUDKeypadWindow.PressButton: add the digit (Touchtone), check once the code is as long as the right one.
/// Ours: enter the code found in the notes.
fn keypad_from_notes(kp: &mut KeypadWin) {
    kp.input.clear();
    let code: Vec<char> = kp.valid.chars().collect();
    for ch in code {
        if let Some(i) = (0..12).find(|&i| key_char(i) == ch) {
            keypad_press(kp, i);
        }
    }
}

fn keypad_press(k: &mut KeypadWin, i: usize) {
    if k.wait.is_some() {
        return;
    }
    if k.input.len() < 16 {
        k.input.push(key_char(i));
        let tone = match i {
            10 => 0,
            n => n + 1,
        };
        crate::sound::play_player(&format!("DeusExSounds.Special.Touchtone{tone}"), 1.0, 0.0);
    }
    if k.input.len() == k.valid.len() {
        keypad_validate(k);
    }
}

fn keypad_validate(k: &mut KeypadWin) {
    let ok = k.input == k.valid;
    if let Ok(mut r) = KEYPAD_RESULTS.lock() {
        r.push((k.blocker, ok));
    }
    k.wait = Some((1.0, ok));
}

struct Win {
    f: Frame,
    body: Body,
    /// Dragged by the title bar (MenuUIWindow.MouseMoved).
    pos: Option<(i32, i32)>,
    focus: Option<W>,
    /// The window hides the one below it (menus and screens do; message boxes don't).
    hides_below: bool,
    /// An open drop-down: (choice, list scroll).
    drop: Option<(u8, i32)>,
}

fn open_window(hud: &mut ClassicHud, assets: &mut RuntimeAssets, class: &str) -> Option<Win> {
    if class == "PadLayout" {
        return pad_map_box(hud, assets);
    }
    let mut f = frame(assets, class);
    let body = match class {
        "MenuMain" | "MenuSelectDifficulty" | "MenuSettings" => {
            let (mut buttons, x, w) = menu_buttons(assets, class);
            if class == "MenuSettings" {
                // Ours: Enhancements, in the free slot above Previous Menu (optional features beyond the original).
                let at = buttons.iter().position(|b| b.action == 2).unwrap_or(buttons.len());
                let y = buttons.get(at.wrapping_sub(1)).map_or(13, |b| b.y + 36);
                buttons.insert(
                    at,
                    MenuBtn { label: "Enhancements".into(), y, action: 1, invoke: "MenuScreenEnhancements".into(), key: String::new(), enabled: true },
                );
            }
            if class == "MenuMain" {
                // Our title.
                f.title = "Welcome to DX RUST!".into();
                // UpdateButtonStatus: no saving or "back to game" on the logo map (or dead).
                let dead = DEAD.load(Ordering::Relaxed);
                for (i, b) in buttons.iter_mut().enumerate() {
                    if (i == 1 || i == 7) && (intro_map() || dead) {
                        b.enabled = false;
                    }
                    if i == 8 {
                        // Ours: the multiplayer slot connects to our online services (online.rs) and opens their
                        // screens; in a game session it leaves it.
                        mp_main_slot(b);
                    }
                }
                main_menu_achievements(&mut f, &mut buttons);
            }
            Body::Menu { buttons, x, w, notes: Vec::new() }
        }
        "MenuScreenMultiplayer" | "MenuScreenHostRoom" | "MenuScreenJoinRoom" | "MenuScreenLobbyBrowser" | "MenuScreenChooseCharacter"
        | "MenuScreenOnlineServices" | "MenuScreenWaitingRoom" | "MenuScreenDmCharacter" => {
            let (mf, body) = mp_window(assets, class);
            f = mf;
            body
        }
        "MenuScreenNewGame" => {
            let skills = fresh_skills();
            let mut order: Vec<usize> = (0..skills.list.len()).collect();
            order.sort_by_key(|&i| skills.list[i].def.name.to_lowercase());
            let mut name = Edit::new(32);
            name.text = "JC Denton".chars().collect();
            name.cur = name.text.len();
            name.anchor = 0;
            Body::NewGame(NewGame {
                name,
                portrait: 0,
                retro_morpheus: false,
                male_interactions: false,
                skills,
                order,
                sel: Some(0),
                difficulty: 1.0,
                desc_scroll: 0,
                multiplayer: false,
            })
        }
        "MenuScreenCustomizeKeys" => {
            let _ = hud;
            Body::Keys(Keys { sel: Some(0), scroll: 0, waiting: false, map: Vec::new(), note: String::new(), pad: false, pad_map: Vec::new() })
        }
        "MenuScreenPadBindings" => {
            // Ours: the keyboard screen's frame and list for the gamepad's buttons.
            f = frame(assets, "MenuScreenCustomizeKeys");
            f.class = class.to_string();
            f.title = "Controller Bindings".into();
            Body::Keys(Keys { sel: Some(0), scroll: 0, waiting: false, map: Vec::new(), note: String::new(), pad: true, pad_map: crate::gamepad::bindings() })
        }
        "MenuScreenSaveGame" | "MenuScreenLoadGame" => {
            let mut sl = SaveLoad::new(class == "MenuScreenSaveGame");
            if sl.saving {
                sl.edit.text = default_save_name().chars().collect();
                sl.edit.cur = sl.edit.text.len();
                sl.edit.anchor = 0;
            }
            Body::SaveLoad(sl)
        }
        "MenuScreenWelcome" => {
            f = frame(assets, "MenuScreenCustomizeKeys");
            f.class = class.to_string();
            f.title = crate::welcome::TITLE.into();
            let ok = f.actions.iter().find(|a| a.1 == 1).map(|a| a.2.clone()).unwrap_or_else(|| "OK".into());
            f.actions = vec![(0, 21, "Don't Show Again".into(), String::new()), (2, 20, ok, String::new())];
            Body::Welcome(0)
        }
        "MenuScreenAchievements" => {
            f = frame(assets, "MenuScreenCustomizeKeys");
            f.class = class.to_string();
            f.title = "Achievements".into();
            // One button: back to the main menu.
            let prev = f.actions.iter().find(|a| a.1 == 4).map(|a| a.2.clone()).unwrap_or_else(|| "Previous Menu".into());
            f.actions = vec![(0, 22, "Reset All".into(), String::new()), (2, 4, prev, String::new())];
            let (chars, global) = crate::achievements::all_characters();
            Body::Achieve(AchieveView { scroll: 0, chars, global })
        }
        "MenuScreenEnhancements" => {
            f = frame(assets, "MenuScreenCustomizeKeys");
            f.class = class.to_string();
            f.title = "Enhancements".into();
            let mut en = Enhance::new();
            // Ours: every original settings screen's choices on the same searchable list ("Display:", "Sound:"...).
            for (screen, group) in [
                ("MenuScreenDisplay", "Display"),
                ("MenuScreenSound", "Sound"),
                ("MenuScreenControls", "Controls"),
                ("MenuScreenOptions", "Game"),
                ("MenuScreenAdjustColors", "Colors"),
            ] {
                if let Some(Win { body: Body::Choices { choices }, .. }) = open_window(hud, assets, screen) {
                    for mut c in choices {
                        if en.items.iter().any(|o| o.key == c.key) {
                            continue;
                        }
                        c.label = format!("{group}: {}", label(&c.label));
                        en.items.push(c);
                    }
                }
            }
            Body::Enhance(en)
        }
        "MenuScreenMatchSettings" => {
            // Ours: the host's deathmatch match settings (deathmatch.rs reads them as the room's rules).
            f = frame(assets, "MenuScreenDisplay");
            f.class = class.to_string();
            f.title = "Match Settings".into();
            let choices = match_choices();
            let slots = art_rows(&f);
            cut_empty_rows(&mut f, choices.len(), slots);
            Body::Choices { choices }
        }
        "MenuScreenDualSense" | "MenuScreenPadOptions" => {
            // Ours, on the Controls screen's frame: features beyond the original (the v2.0 options go here).
            // (The Display screen's frame: room for nine rows.)
            f = frame(assets, "MenuScreenDisplay");
            f.class = class.to_string();
            let choices = if class == "MenuScreenDualSense" {
                f.title = "DualSense Options".into();
                dualsense_choices()
            } else if class == "MenuScreenPadOptions" {
                f.title = "Controller Options".into();
                pad_option_choices()
            } else {
                f.title = "Enhancements".into();
                enhancement_choices()
            };
            let slots = art_rows(&f);
            cut_empty_rows(&mut f, choices.len(), slots);
            Body::Choices { choices }
        }
        "MenuUIMessageBoxWindow" => {
            f.min_title = 250;
            Body::Msg(Msg { text: String::new(), yes_no: true, purpose: MsgPurpose::Info })
        }
        _ => {
            let d = assets.classes.defaults(class);
            let paths: Vec<String> = indexed(&d, "choices")
                .into_iter()
                .filter_map(|(_, v)| match v {
                    Value::Path(p) if !p.ends_with(".None") => Some(short(p)),
                    _ => None,
                })
                .collect();
            if paths.is_empty() {
                return None;
            }
            let mut choices: Vec<Choice> = paths.iter().map(|p| choice(assets, p)).collect();
            let original: Vec<String> = choices.iter().map(|c| c.key.clone()).collect();
            // Settings the renderer and mixer decide for themselves (always the best): not offered.
            // So do the old-hardware sound options (voice count, Dolby Surround, A3D/EAX hardware): the mixer is ours.
            const DROPPED: [&str; 11] = [
                "RenderDevice",
                "TextureColorBits",
                "WorldTextureDetail",
                "ObjectTextureDetail",
                "DetailTextures",
                "SampleRate",
                "SoundQuality",
                "FullScreen",
                "EffectsChannels",
                "SurroundSound",
                "Use3DHardware",
            ];
            let slots = art_rows(&f).max(original.len());
            choices.retain(|c| !DROPPED.contains(&c.key.as_str()));
            if class == "MenuScreenDisplay" {
                let at = choices.iter().position(|c| c.key == "Resolution").unwrap_or(choices.len());
                choices.insert(at, display_mode_choice());
                let at = choices.iter().position(|c| c.key == "Resolution").map_or(choices.len(), |i| i + 1);
                choices.insert(at, vsync_choice());
                choices.push(hud_scale_choice());
            }
            for c in choices.iter_mut().filter(|c| c.key == "Resolution") {
                c.info_w = 110;
            }
            if class == "MenuScreenDisplay" {
                // Ours are added in: close up below the last row.
                cut_empty_rows(&mut f, choices.len(), slots);
            } else {
                // Only dropped: each one's row goes (its well matches the kind of setting it held) and the spare
                // wells under the last.
                let gone: Vec<usize> = (0..slots)
                    .filter(|&r| original.get(r).is_none_or(|k| !choices.iter().any(|c| &c.key == k)))
                    .collect();
                cut_rows(&mut f, &gone);
            }
            Body::Choices { choices }
        }
    };
    let hides_below = class != "MenuUIMessageBoxWindow";
    Some(Win { f, body, pos: None, focus: None, hides_below, drop: None })
}

/// Ours: Achievements under Multiplayer. The art has no well for it (only a thin gap before Exit), so the
/// Multiplayer well (with the gap above it) is drawn twice and Exit moves down a row.
fn main_menu_achievements(f: &mut Frame, buttons: &mut Vec<MenuBtn>) {
    let Some(mp) = buttons.get(8).map(|b| b.y) else { return };
    const ROW: i32 = 36;
    // From the bottom of the button above (27 high, 36 apart) to the bottom of the Multiplayer button.
    f.dups.push((mp - (ROW - 27), ROW));
    f.ch += ROW;
    for b in buttons.iter_mut().skip(9) {
        b.y += ROW;
    }
    let at = 9.min(buttons.len());
    buttons.insert(
        at,
        MenuBtn { label: "Achievements".into(), y: mp + ROW, action: 1, invoke: "MenuScreenAchievements".into(), key: String::new(), enabled: true },
    );
}

/// Leave the art's unused choice wells out: rows are 36 apart from y 27, so the band from just above row `used`
/// to just above row `slots` goes, and the help line and buttons move up with the window's bottom.
fn cut_empty_rows(f: &mut Frame, used: usize, slots: usize) {
    let rows: Vec<usize> = (used..slots).collect();
    cut_rows(f, &rows);
}

/// How many choice rows the screen's art has wells for (rows 36 apart from y 27, above the help line).
fn art_rows(f: &Frame) -> usize {
    ((f.help_y - 27).max(0) / 36) as usize
}

/// Leave out the art of these rows (each 36 high, from 6 above the row), closing up what's below.
fn cut_rows(f: &mut Frame, rows: &[usize]) {
    let mut rows = rows.to_vec();
    rows.sort_unstable();
    rows.dedup();
    for r in rows.iter().rev() {
        f.cuts.push((27 + *r as i32 * 36 - 6, 36));
    }
    f.cuts.sort_unstable();
    let h = rows.len() as i32 * 36;
    f.ch -= h;
    f.help_y -= h;
}

/// Ours: the Enhancements screen - Controller Bindings, the gamepad options (ROADMAP Phase 0.1 / 0.2), Debug Mode.
fn enhancement_choices() -> Vec<Choice> {
    let _e = |key: &str, label: &str, help: &str, values: &[&str]| {
        let v = crate::settings::get(key);
        Choice {
            key: key.into(),
            label: label.into(),
            help: help.into(),
            kind: ChoiceKind::Enum(values.iter().map(|s| s.to_string()).collect()),
            value: v,
            saved: v,
            info_w: 98,
            info_x: 270,
            control_x: 270,
            voff: 0,
            dropdown: false,
        }
    };
    vec![
        Choice {
            key: "PadBindings".into(),
            label: "|&Controller Bindings".into(),
            help: "Assign the gamepad's buttons to actions.".into(),
            kind: ChoiceKind::Action { action: 1, invoke: "MenuScreenPadBindings".into() },
            value: 0.0,
            saved: 0.0,
            info_w: 98,
            info_x: 270,
            control_x: 270,
            voff: 0,
            dropdown: false,
        },
        Choice {
            key: "PadLayout".into(),
            label: "|&Controller Layout".into(),
            help: "A picture of the gamepad with what each button does (your bindings).".into(),
            kind: ChoiceKind::Action { action: 1, invoke: "PadLayout".into() },
            value: 0.0,
            saved: 0.0,
            info_w: 98,
            info_x: 270,
            control_x: 270,
            voff: 0,
            dropdown: false,
        },
        Choice {
            key: "PadDualSense".into(),
            label: "|&DualSense Options".into(),
            help: "The DualSense's light bar, player lights, adaptive triggers, haptics and speaker.".into(),
            kind: ChoiceKind::Action { action: 1, invoke: "MenuScreenDualSense".into() },
            value: 0.0,
            saved: 0.0,
            info_w: 98,
            info_x: 270,
            control_x: 270,
            voff: 0,
            dropdown: false,
        },
        Choice {
            key: "PadOptions".into(),
            label: "Controller |&Options".into(),
            help: "Button icons and hints, look speed, deadzone, aim slowdown and the aug hotkey pages.".into(),
            kind: ChoiceKind::Action { action: 1, invoke: "MenuScreenPadOptions".into() },
            value: 0.0,
            saved: 0.0,
            info_w: 98,
            info_x: 270,
            control_x: 270,
            voff: 0,
            dropdown: false,
        },
        debug_mode_choice(),
    ]
}

/// Ours: Controller Options (under Enhancements).
fn pad_option_choices() -> Vec<Choice> {
    let e = |key: &str, label: &str, help: &str, values: &[&str]| {
        let v = crate::settings::get(key);
        Choice {
            key: key.into(),
            label: label.into(),
            help: help.into(),
            kind: ChoiceKind::Enum(values.iter().map(|s| s.to_string()).collect()),
            value: v,
            saved: v,
            info_w: 98,
            info_x: 270,
            control_x: 270,
            voff: 0,
            dropdown: false,
        }
    };
    vec![
        e("PadGlyphs", "|&Button Icons", "Which controller's button icons the game shows: the connected one's, Xbox or PlayStation.", &["Automatic", "Xbox", "PlayStation"]),
        e("PadLookSensitivityH", "Controller |&Horizontal Sensitivity", "How fast the right stick turns the view left and right.", &["Very Slow", "Slow", "Normal", "Fast", "Very Fast"]),
        e("PadLookSensitivityV", "Controller |&Vertical Sensitivity", "How fast the right stick tilts the view up and down.", &["Very Slow", "Slow", "Normal", "Fast", "Very Fast"]),
        e("PadInvertLook", "|&Invert Controller Look", "Push the right stick up to look down.", &["Disabled", "Enabled"]),
        e("PadDeadzone", "Stick |&Deadzone", "How far a stick moves before it counts.", &["Very Small", "Small", "Normal", "Large", "Very Large"]),
        e("PadAimSlowdown", "|&Aim Slowdown", "The right stick turns more slowly while the crosshair is on an enemy.", &["Disabled", "Enabled"]),
        e("PadAugPages", "Aug |&Hotkey Pages", "The D-pad toggles pages of three augmentations; tap the wheel button to turn the page, hold it for the wheel.", &["Disabled", "Enabled"]),
        e("PadHints", "Button |&Hints", "A bar at the bottom of menus and screens showing what the controller's buttons do there.", &["Disabled", "Enabled"]),
        e("PadFrobPrompt", "Button Prompts on |&Objects", "Show the Use button beside the name of what you're looking at (not in the original).", &["Disabled", "Enabled"]),
    ]
}

/// Ours: the DualSense screen (ROADMAP Phase 0.3), under Enhancements.
/// Match Settings: map, frag limit, time limit, what comes after a match, team auto-balance.
fn match_choices() -> Vec<Choice> {
    let e = |key: &str, label: &str, help: &str, values: Vec<String>| {
        let v = crate::settings::get(key);
        Choice {
            key: key.into(),
            label: label.into(),
            help: help.into(),
            kind: ChoiceKind::Enum(values),
            value: v,
            saved: v,
            info_w: 140,
            info_x: 270,
            control_x: 270,
            voff: 0,
            dropdown: false,
        }
    };
    let n = |v: &[u32], unit: &str| v.iter().map(|x| if *x == 0 { "None".to_string() } else { format!("{x}{unit}") }).collect::<Vec<_>>();
    vec![
        e("DmMap", "|&Map", "The original's multiplayer map to play. Weapons are laid out so every part of it is worth fighting for: one GEP gun, one of each heavy weapon.", crate::deathmatch::maps().into_iter().map(|m| m.1).collect()),
        e("DmFragLimit", "|&Frag Limit", "Kills that win the match (the team's total in Team Deathmatch).", n(&crate::deathmatch::FRAG_LIMITS, " kills")),
        e("DmTimeLimit", "|&Time Limit", "Minutes until the match ends: the most kills wins.", n(&crate::deathmatch::TIME_LIMITS, " minutes")),
        e("DmAfter", "After the |&Match", "When a match ends and its results have shown: the next of the five maps, the same map again, or back to the room.", ["Next Map", "Same Map", "Back to Room"].iter().map(|s| s.to_string()).collect()),
        e("DmAutoBalance", "Team |&Auto-Balance", "Team Deathmatch: when one team is two players bigger, its next player to die joins the other.", ["Off", "On"].iter().map(|s| s.to_string()).collect()),
    ]
}

fn dualsense_choices() -> Vec<Choice> {
    let e = |key: &str, label: &str, help: &str, values: &[&str]| {
        let v = crate::settings::get(key);
        Choice {
            key: key.into(),
            label: label.into(),
            help: help.into(),
            kind: ChoiceKind::Enum(values.iter().map(|s| s.to_string()).collect()),
            value: v,
            saved: v,
            info_w: 98,
            info_x: 270,
            control_x: 270,
            voff: 0,
            dropdown: false,
        }
    };
    vec![
        e("PadLightbar", "|&Light Bar", "The light bar shows your health: green, through yellow, to red (it pulses when you're nearly dead).", &["Off", "Health"]),
        e("PadPlayerLeds", "|&Player Lights", "The five lights under the touchpad: your bioelectric energy, the usual player-one light, or off.", &["Bioenergy", "Player One", "Off"]),
        e("PadTriggers", "|&Adaptive Triggers", "Trigger resistance by weapon: a pistol's click, a rifle's heavy break, a launcher's wall; it buzzes in automatic fire and goes dead when the clip is empty.", &["Off", "Light", "Full"]),
        e("PadHaptics", "|&Haptics", "HD haptics (USB only) play footsteps by surface, shots, blasts and the side you're hit from; Classic Rumble uses the motors.", &["Classic Rumble", "HD (USB)"]),
        e("PadHapticSteps", "|&Footstep Haptics", "Feel your footsteps through the controller (HD haptics: by surface).", &["Disabled", "Enabled"]),
        e("PadRumble", "Rumble |&Strength", "How strongly the controller vibrates (every controller).", &["Off", "Low", "Normal", "Strong"]),
        e("PadSpeaker", "Infolinks Through |&Speaker", "Infolink transmissions play through the DualSense's speaker (USB only).", &["Disabled", "Enabled"]),
        e("PadSpeakerVolume", "Speaker |&Volume", "The DualSense speaker's volume.", &["Low", "Normal", "High"]),
    ]
}

/// Ours: Debug Mode on the Enhancements screen.
fn debug_mode_choice() -> Choice {
    let v = setting!("DebugMode");
    Choice {
        key: "DebugMode".into(),
        label: "|&Debug Mode".into(),
        help: "Developer cheats and readouts: the spawn menu (Insert), the debug unlock (Page Up), god mode and the info text in the corner.".into(),
        kind: ChoiceKind::Enum(vec!["Disabled".into(), "Enabled".into()]),
        value: v,
        saved: v,
        info_w: 77,
        info_x: 270,
        control_x: 270,
        voff: 0,
        dropdown: false,
    }
}

/// A new character's skills (Skill defaults, CurrentLevel), with the player's starting points.
fn fresh_skills() -> imsim_core::skills::Skills {
    let g = crate::abilities::lock();
    let Some(a) = g.as_ref() else { return imsim_core::skills::Skills::new(Vec::new(), 5000) };
    let defs = a.skills.list.iter().map(|s| s.def.clone()).collect();
    imsim_core::skills::Skills::new(defs, 5000)
}

fn msg_box(hud: &mut ClassicHud, assets: &mut RuntimeAssets, title: &str, text: &str, yes_no: bool, purpose: MsgPurpose) -> Option<Win> {
    let mut w = open_window(hud, assets, "MenuUIMessageBoxWindow")?;
    w.f.title = title.to_string();
    w.f.actions = if yes_no {
        let d = assets.classes.defaults("MenuUIMessageBoxWindow");
        vec![(2, 10, pstr(&d, "btnLabelNo"), "NO".into()), (2, 11, pstr(&d, "btnLabelYes"), "YES".into())]
    } else {
        let d = assets.classes.defaults("MenuUIMessageBoxWindow");
        vec![(2, 12, pstr(&d, "btnLabelOK"), "OK".into())]
    };
    w.body = Body::Msg(Msg { text: text.to_string(), yes_no, purpose });
    w.focus = Some(W::Act(if yes_no { 1 } else { 0 }));
    Some(w)
}

// ------------------------------------------------------------------------------------------------ drawing

/// The window's size (MenuUIWindow.ParentRequestedPreferredSize), including its shadow.
fn win_size(f: &Frame, fonts: &Fonts) -> (i32, i32) {
    if f.class == "HUDKeypadWindow" {
        return (103, 162);
    }
    let (so, ss) = f.shadow.unwrap_or(((0, 0), (0, 0)));
    let tw = title_width(f, fonts);
    let w = (f.cw + TITLE_LEFT_BOTTOM.0 + if f.right_edge { 4 } else { 0 }).max(tw) + so.0;
    let h = f.ch + TITLE_H + if f.bar { BAR_H } else { 0 } + f.voff + so.1;
    (w.max(ss.0), h.max(ss.1))
}

fn title_width(f: &Frame, fonts: &Fonts) -> i32 {
    (31 + fonts.title.width(&label(&f.title)).min(400) + 45).max(f.min_title)
}

/// MenuUIMenuButtonWindow: the big main-menu buttons with their light.
fn menu_button(u: &mut Ui<W>, w: W, x: i32, y: i32, width: i32, s: &str, enabled: bool, font: &crate::uicanvas::UiFont) {
    let st = u.state(w, enabled);
    let set = if st == 2 { "Pressed" } else { "Normal" };
    let face = m_col(3);
    let style = if crate::settings::menu_translucent() { Style::Translucent } else { Style::Masked };
    u.draw(&format!("MenuMainButton{set}_Left"), x + 9, y, 5, 27, style, face);
    if let Some(t) = u.tex(&format!("MenuMainButton{set}_Center")) {
        u.c.texture(x + 14, y, width - 14 - 20, 27, 0, 0, &t, style, face);
    }
    u.draw(&format!("MenuMainButton{set}_Right"), x + width - 20, y, 20, 27, style, face);
    let off = if st == 2 { 1 } else { 0 };
    // bTranslucentText: the text adds onto the button.
    let col = m_btn(st);
    let lines = label(s);
    let tw = font.width(&lines);
    let tx = x - 4 + off + (width - tw) / 2;
    let ty = y + 1 + off + (27 - font.height) / 2;
    for (bi, ch) in lines.char_indices() {
        let gx = tx + font.width(&lines[..bi]);
        if let Some((page, g)) = font.font.glyph(ch) {
            if let Some(tex) = font.pages.get(page) {
                let tex = tex.clone();
                u.c.texture(gx, ty, g.w, g.h, g.u, g.v, &tex, Style::Translucent, col);
            }
        }
    }
    let light = if st == 1 || st == 2 { "MenuMainLight_Focus" } else { "MenuMainLight_Normal" };
    u.draw(light, x, y, 7, 27, Style::Translucent, face);
    u.hit(w, (x, y, width, 27), enabled);
}

/// MenuUISliderButtonWindow at (x, y): bar, thumb at the tick, value box.
fn slider(u: &mut Ui<W>, w: W, x: i32, y: i32, c: &Choice, scale_w: i32, show_text: bool) {
    let face = m_col(3);
    u.draw("MenuSliderBar", x, y, scale_w, 21, Style::Masked, face);
    if let ChoiceKind::Slider { ticks, .. } = &c.kind {
        let span = scale_w - 16;
        let tx = x + 8 + (c.slider_tick() * span) / (ticks - 1).max(1) - 4;
        u.draw("MenuSlider", tx, y + 3, 9, 15, Style::Masked, face);
    }
    u.hit(w, (x, y, scale_w, 21), true);
    if show_text {
        info_button(u, x + 184, y + 1, 60, &c.value_text(), m_btn(0));
    }
}

/// Lean Updates (perf.rs): a window layer's last drawing.
#[derive(Default)]
struct Drawn {
    /// What its image was drawn from (`win_key` and the frame's inputs), when drawing again from the same would
    /// give the same picture and leave the window as it is; None draws it again.
    key: Option<u64>,
    /// The hit rectangles that drawing made.
    hits: Vec<(W, Rect, bool)>,
    /// Its canvas, kept for the next drawing.
    canvas: Option<Canvas>,
}

/// Feeds a value's Debug text to a hasher (for key parts without Hash).
fn hash_debug(h: &mut impl Hasher, v: &impl std::fmt::Debug) {
    struct Feed<'a, H>(&'a mut H);
    impl<H: Hasher> std::fmt::Write for Feed<'_, H> {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            self.0.write(s.as_bytes());
            Ok(())
        }
    }
    let _ = std::fmt::Write::write_fmt(&mut Feed(h), format_args!("{v:?}"));
}

fn edit_key(e: &Edit, h: &mut impl Hasher) {
    let Edit { text, cur, anchor, max, scroll, org } = e;
    (text, cur, anchor, max, scroll, org).hash(h);
}

fn choice_key(c: &Choice, h: &mut impl Hasher) {
    let Choice { key, label, help, kind, value, saved, info_w, info_x, control_x, voff, dropdown } = c;
    (key, label, help, value.to_bits(), saved.to_bits(), info_w, info_x, control_x, voff, dropdown).hash(h);
    match kind {
        ChoiceKind::Enum(v) => (0u8, v).hash(h),
        ChoiceKind::Slider { ticks, start, end, labels } => (1u8, ticks, start.to_bits(), end.to_bits(), labels).hash(h),
        ChoiceKind::Action { action, invoke } => (2u8, action, invoke).hash(h),
    }
}

/// Lean Updates: the window's part of what `draw_win` reads - all of the window (destructured, so a new field
/// can't be missed) and the outside state its body's drawing asks for.
fn win_key(win: &Win, h: &mut impl Hasher) {
    let Win { f, body, pos, focus, hides_below, drop } = win;
    let Frame {
        class, title, cw, ch, voff, textures, cols, left_edge, right_edge, bar, help, help_always, help_y, actions,
        shadow, escape_saves, min_title, cuts, dups,
    } = f;
    (class, title, cw, ch, voff, textures, cols, left_edge, right_edge, bar, help, help_always).hash(h);
    (help_y, actions, shadow, escape_saves, min_title, cuts, dups).hash(h);
    (pos, hides_below, drop).hash(h);
    hash_debug(h, focus);
    std::mem::discriminant(body).hash(h);
    match body {
        Body::Menu { buttons, x, w, notes } => {
            (x, w, buttons.len(), notes).hash(h);
            for MenuBtn { label, y, action, invoke, key, enabled } in buttons {
                (label, y, action, invoke, key, enabled).hash(h);
            }
        }
        Body::Choices { choices } => {
            choices.len().hash(h);
            for c in choices {
                choice_key(c, h);
            }
        }
        Body::NewGame(NewGame { name, portrait, retro_morpheus, male_interactions, skills, order, sel, difficulty, desc_scroll, multiplayer }) => {
            edit_key(name, h);
            (portrait, retro_morpheus, male_interactions, order, sel, difficulty.to_bits(), desc_scroll, multiplayer).hash(h);
            hash_debug(h, skills);
            crate::travel::lddp().hash(h);
        }
        Body::Keys(Keys { sel, scroll, waiting, map, note, pad, pad_map }) => {
            (sel, scroll, waiting, map, note, pad, pad_map).hash(h);
            hash_debug(h, &crate::gamepad::glyph_kind());
        }
        Body::Msg(Msg { text, yes_no, purpose }) => {
            (text, yes_no).hash(h);
            hash_debug(h, purpose);
        }
        Body::SaveLoad(SaveLoad { saving, games, chars, open, list, sel, scroll, edit, confirm_delete, snap }) => {
            (saving, sel, scroll, confirm_delete, chars).hash(h);
            let mut opened: Vec<&String> = open.iter().collect();
            opened.sort();
            opened.hash(h);
            hash_debug(h, games);
            hash_debug(h, list);
            edit_key(edit, h);
            // The picture is loaded into `snap` only when the selected file changes: its file stands for it.
            snap.as_ref().map(|(p, t)| (p, t.is_some())).hash(h);
            // The Save button, and the dates (the time zone).
            (crate::save::can_save(false), crate::save::format_date(0)).hash(h);
        }
        Body::Welcome(scroll) => scroll.hash(h),
        Body::Achieve(AchieveView { scroll, chars, global }) => {
            scroll.hash(h);
            for c in chars {
                let mut u: Vec<_> = c.unlocked.iter().collect();
                u.sort();
                (&c.name, u).hash(h);
            }
            let mut g: Vec<_> = global.iter().collect();
            g.sort();
            g.hash(h);
        }
        Body::Keypad(KeypadWin { blocker, valid, input, wait, instant, noted }) => {
            // The drawing reads whether a result shows and which, not the time left on it.
            (blocker, valid, input, wait.map(|w| w.1), instant, noted).hash(h);
            (crate::diegetic::keypad_active(), crate::gamepad::pad_in_use()).hash(h);
            hash_debug(h, &crate::gamepad::glyph_kind());
        }
        Body::Host(MpHost { choices, name, code, players, offline, open, direct, note, lobby, all_ready, copied }) => {
            choices.len().hash(h);
            for c in choices {
                choice_key(c, h);
            }
            (name, code, players, offline, open, direct, note, lobby, all_ready, copied.is_some()).hash(h);
        }
        Body::Wait(MpWait { rows, ready, connected, scroll }) => (rows, ready, connected, scroll).hash(h),
        Body::DmChar(d) => {
            edit_key(&d.name, h);
            hash_debug(h, &d.c);
            (d.sel, d.scroll, d.others.len(), &d.weapons).hash(h);
        }
        Body::Join(MpJoin { code }) => code.hash(h),
        Body::Addr(MpAddr { edit }) => edit_key(edit, h),
        Body::Chars(MpChars { list, rows, room, sel, scroll }) => (list, rows, room, sel, scroll).hash(h),
        Body::Browse(MpBrowse { rooms, state, list_ms, filters, shown, rows, sel, scroll }) => {
            hash_debug(h, rooms);
            hash_debug(h, state);
            (list_ms, filters, shown, rows, sel, scroll).hash(h);
        }
        Body::Enhance(Enhance { items, search, sel, scroll, open, modern, mode_changed, memory }) => {
            items.len().hash(h);
            for c in items {
                choice_key(c, h);
            }
            edit_key(search, h);
            hash_debug(h, sel);
            (scroll, modern, mode_changed, open.len(), memory.len()).hash(h);
            // (The same set iterates in the same order; another order only draws again.)
            for g in open {
                g.hash(h);
            }
            for (k, v) in memory {
                (k, v.to_bits()).hash(h);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
/// MenuScreenAdjustColorsExample.DrawWindow (207 x 135): background and borders in the HUD theme's colours and
/// translucency, then the title / header / normal text and the two button labels.
fn draw_colors_example(u: &mut Ui<W>, x: i32, y: i32) {
    let d = u.assets.classes.defaults("MenuScreenAdjustColorsExample");
    let bg = if crate::settings::hud_translucent() { Style::Translucent } else { Style::Masked };
    u.draw("MenuColorHUDBackground", x, y, 207, 135, bg, crate::menuui::h_col(0));
    if crate::settings::hud_borders() {
        let bs = if crate::settings::hud_border_translucent() { Style::Translucent } else { Style::Masked };
        u.draw("MenuColorHUDBorders", x, y, 207, 135, bs, crate::menuui::h_col(1));
    }
    let (headers, small) = (u.f.headers.clone(), u.f.small.clone());
    let rows = [
        (&headers, 23, 19, 90, 11, 2, "TitleBarLabel", "Title Bar"),
        (&headers, 27, 37, 90, 11, 7, "TextHeaderLabel", "Text Header"),
        (&small, 27, 56, 90, 11, 8, "TextNormalLabel", "Normal Text"),
        (&headers, 28, 110, 54, 10, 4, "ButtonPressedLabel", "Normal"),
        (&headers, 94, 110, 54, 10, 5, "ButtonNormalLabel", "Focus"),
    ];
    for (f, tx, ty, tw, th, col, key, def) in rows {
        let t = pstr(&d, key);
        let t = if t.is_empty() { def.to_string() } else { t };
        u.c.text(f, x + tx, y + ty, tw, th, HAlign::Left, VAlign::Top, crate::menuui::h_col(col), &t);
    }
}

fn draw_win(u: &mut Ui<W>, win: &mut Win, fonts: &Fonts, help_text: &str, waiting_keys: bool) {
    let f = win.f.clone();
    let (so, _) = f.shadow.unwrap_or(((0, 0), (0, 0)));
    let (ox, oy) = (so.0, so.1);
    let (ww, wh) = win_size(&f, fonts);
    u.hit(W::Win, (0, 0, ww, wh), true);
    if let Body::Keypad(k) = &win.body {
        // Diegetic keypads are used in the world: no window (diegetic.rs).
        if !crate::diegetic::keypad_active() {
            draw_keypad(u, k, fonts);
        }
        return;
    }
    // Shadow (MenuUIMessageBoxShadowWindow: two 256-wide halves, by button count).
    if f.shadow.is_some() {
        let n = match &win.body {
            Body::Msg(m) if m.yes_no => 2,
            _ => 1,
        };
        for k in 0..2 {
            if let Some(t) = u.tex(&format!("MenuMessageBoxShadow{n}_{}", k + 1)) {
                u.c.texture(k * 256, 0, t.w as i32, t.h as i32, 0, 0, &t, Style::Modulated, WHITE);
            }
        }
    }
    let translucent = crate::settings::menu_translucent();
    let tw = title_width(&f, fonts);
    // Right edge (drawn under everything else: Lower()).
    if f.right_edge {
        let (rx, ry) = (ox + tw + 1, oy + TITLE_H - 4);
        let rw = f.cw - (tw - TITLE_LEFT_BOTTOM.0) - 1 + 4;
        let rh = f.ch + 4 + 14;
        let face = m_col(3);
        u.draw("MenuRightBorder_TopLeft", rx, ry, 2, 4, Style::Masked, face);
        if let Some(t) = u.tex("MenuRightBorder_Top") {
            u.c.texture(rx + 2, ry, rw - 2 - 5, 4, 0, 0, &t, Style::Masked, face);
        }
        u.draw("MenuRightBorder_TopRight", rx + rw - 5, ry, 5, 6, Style::Masked, face);
        if let Some(t) = u.tex("MenuRightBorder_Right") {
            u.c.texture(rx + rw - 4, ry + 6, 4, rh - 6 - 13, 0, 0, &t, Style::Masked, face);
        }
        u.draw("MenuRightBorder_BottomRight", rx + rw - 4, ry + rh - 13, 4, 13, Style::Masked, face);
    }
    // Title bar.
    for (set, col) in [("Bar", m_col(3)), ("Bubble", m_col(1))] {
        u.draw(&format!("MenuTitle{set}_Left"), ox, oy, 87, TITLE_H, Style::Masked, col);
        if set == "Bar" {
            u.draw("MenuTitleBar_LeftBottom", ox, oy + TITLE_H - 1, TITLE_LEFT_BOTTOM.0, TITLE_LEFT_BOTTOM.1, Style::Masked, col);
        }
        if let Some(t) = u.tex(&format!("MenuTitle{set}_Center")) {
            u.c.texture(ox + 87, oy, tw - 187, TITLE_H, 0, 0, &t, Style::Masked, col);
        }
        u.draw(&format!("MenuTitle{set}_Right"), ox + tw - 100, oy, 100, TITLE_H, Style::Masked, col);
    }
    let ft = fonts.title.clone();
    u.text(&ft, ox + 31, oy, tw - 31, TITLE_H, HAlign::Left, VAlign::Center, m_col(2), &f.title);
    u.icon("MenuIcon_DeusEx", ox + 12, oy + 3, Style::Masked, WHITE);
    u.hit(W::Title, (ox, oy, tw, TITLE_H), true);
    // Client.
    let (cx, cy) = (ox + TITLE_LEFT_BOTTOM.0, oy + TITLE_H);
    u.c.set_clip(Some((cx, cy, f.cw, f.ch)));
    let style = if translucent { Style::Translucent } else { Style::Masked };
    for (by, bh, shift) in art_bands(&f) {
        u.c.set_clip(Some((cx, cy + by, f.cw, bh)));
        for (i, t) in f.textures.iter().enumerate() {
            let (col, row) = (i as i32 % f.cols, i as i32 / f.cols);
            u.icon(t, cx + col * 256, cy + row * 256 - shift, style, m_col(0));
        }
    }
    u.c.set_clip(None);
    draw_body(u, win, fonts, cx, cy, waiting_keys);
    // Action buttons.
    if f.bar {
        let gap = if f.left_edge { 1 } else { 0 };
        let rgap = if f.right_edge { 2 } else { 0 };
        let (bx, by, bw) = (cx + gap, cy + f.ch, f.cw - 1 - rgap);
        let (mut left, mut right) = (0, bw);
        for (i, (align, action, lbl, key)) in f.actions.iter().enumerate() {
            let w = action_width(fonts, lbl);
            let x = if *align == 2 {
                right -= w;
                let x = right;
                right -= 2;
                x
            } else {
                let x = left;
                left += w + 2;
                x
            };
            let enabled = action_enabled(win, *action, key);
            action_button(u, W::Act(i as u8), bx + x, by + 1, w, lbl, enabled);
        }
    }
    if f.left_edge {
        let face = m_col(3);
        let (ex, ey, eh) = (cx - 4, oy + TITLE_H + TITLE_LEFT_BOTTOM.1, f.ch + BAR_H - TITLE_LEFT_BOTTOM.1 - 1);
        u.draw("MenuLeftBorder_Top", ex, ey, 4, 2, Style::Masked, face);
        if let Some(t) = u.tex("MenuLeftBorder_Center") {
            u.c.texture(ex, ey + 2, 4, eh - 2 - 16, 0, 0, &t, Style::Masked, face);
        }
        u.draw("MenuLeftBorder_Bottom", ex, ey + eh - 16, 4, 16, Style::Masked, face);
    }
    // Help line (MenuUIHelpWindow: margins 10, FontMenuSmall).
    if f.help && (!help_text.is_empty() || f.help_always) {
        let fs = fonts.small.clone();
        let (hx, hy, hw, hh) = (cx + 7, cy + f.help_y, f.cw - 21, 27);
        u.c.set_clip(Some((hx, hy, hw, hh)));
        u.wrapped_scroll(&fs, hx + 10, hy, hw - 20, hh, m_col(7), help_text);
        u.c.set_clip(None);
    }
}

/// The client art's pieces as (y in the client, height, shift = art y - client y). With a cut, the art above it is
/// drawn in place and the art below it moved up by its height; a dup band is drawn again right under itself and
/// the art below it moves down. Cut and dup bands are in the art's own coordinates.
fn art_bands(f: &Frame) -> Vec<(i32, i32, i32)> {
    // (art y where it takes effect, height, dup?) in art order: a cut skips [y, y + h), a dup repeats [y - h, y).
    let mut ev: Vec<(i32, i32, bool)> = f.cuts.iter().map(|&(y, h)| (y, h, false)).collect();
    ev.extend(f.dups.iter().map(|&(y, h)| (y + h, h, true)));
    ev.sort_unstable();
    let cut: i32 = f.cuts.iter().map(|c| c.1).sum();
    let dup: i32 = f.dups.iter().map(|d| d.1).sum();
    let end = f.ch + cut - dup;
    let mut spans: Vec<(i32, i32)> = Vec::new();
    let mut src = 0;
    for (p, h, is_dup) in ev {
        if is_dup {
            spans.push((src, p));
            spans.push((p - h, p));
            src = p;
        } else {
            spans.push((src, p));
            src = p + h;
        }
    }
    spans.push((src, end));
    let mut bands = Vec::new();
    let mut dst = 0;
    for (a, b) in spans {
        if b > a {
            bands.push((dst, b - a, a - dst));
            dst += b - a;
        }
    }
    bands
}

fn action_enabled(win: &Win, action: u8, key: &str) -> bool {
    match &win.body {
        Body::NewGame(ng) if action == 5 && key == "START" => !ng.name.text.is_empty(),
        Body::SaveLoad(sl) if action == 5 && (key == "LOAD" || key == "DELETE") => sl.sel.and_then(|r| sl.game(r)).is_some(),
        Body::SaveLoad(sl) if action == 5 && key == "SAVE" => sl.sel.is_some() && !sl.edit.get().trim().is_empty() && crate::save::can_save(false),
        Body::Host(h) if action == ACT_START => h.open && h.all_ready,
        Body::Chars(c) if action == ACT_CHAR_GO => c.sel.is_some(),
        Body::Join(j) if action == ACT_JOIN_CODE => mp_join_act(&j.code).is_some(),
        Body::Browse(b) if action == ACT_JOIN_ROW => b.sel.is_some(),
        _ => true,
    }
}

/// The window's client area by body: menu buttons, choices, the new-game screen, the lists, message text.
fn draw_body(u: &mut Ui<W>, win: &mut Win, fonts: &Fonts, cx: i32, cy: i32, waiting_keys: bool) {
    let focus = win.focus;
    match &mut win.body {
        Body::Menu { buttons, x, w, notes } => {
            draw_menu_buttons(u, buttons, *x, *w, fonts, cx, cy);
            for (y, text) in notes.iter() {
                draw_menu_note(u, *x, *w, *y, text, cx, cy);
            }
        }
        Body::Host(h) => draw_mp_host(u, h, &win.f, &mut win.drop, cx, cy),
        Body::Join(j) => draw_mp_join(u, j, cx, cy),
        Body::Browse(b) => draw_mp_browse(u, b, cx, cy),
        Body::Chars(c) => draw_mp_chars(u, c, cx, cy),
        Body::Addr(a) => draw_mp_addr(u, a, cx, cy),
        Body::Wait(w) => draw_mp_wait(u, w, cx, cy),
        Body::DmChar(d) => draw_dm_char(u, d, cx, cy),
        Body::Choices { choices } => draw_choices(u, choices, &win.f, &mut win.drop, cx, cy),
        Body::NewGame(ng) => draw_new_game(u, ng, focus, fonts, cx, cy),
        Body::Keys(k) if k.pad => draw_pad_keys(u, k, cx, cy, waiting_keys),
        Body::Enhance(en) => draw_enhance(u, en, cx, cy),
        Body::Welcome(scroll) => {
            header_button(u, W::Header(0), cx + 10, cy + 3, 322, "What's new for Deus Ex players");
            text_area(u, 0, (cx + 11, cy + 23, 369, 268), crate::welcome::TEXT, false, scroll);
        }
        Body::Achieve(av) => draw_achieve(u, av, fonts, cx, cy),
        Body::Keys(k) => draw_keys(u, k, cx, cy, waiting_keys),
        Body::SaveLoad(sl) => draw_save_load(u, sl, fonts, cx, cy),
        Body::Keypad(_) => {}
        Body::Msg(m) if m.purpose == MsgPurpose::PadMap => {
            crate::paddiagram::draw(u, cx, cy);
        }
        Body::Msg(m) => {
            let fds = u.hud.font(u.assets, "FontMenuHeaders_DS").unwrap_or(fonts.headers.clone());
            let (tx, ty, tw, th) = (cx + 20, cy + 14, win.f.cw - 40, win.f.ch - 28);
            u.wrapped(&fds, tx, ty, tw, th, HAlign::Center, VAlign::Center, m_col(8), &m.text);
        }
    }
}

/// MenuUIMenuWindow: the big buttons in their column.
#[allow(clippy::too_many_arguments)]
fn draw_menu_buttons(u: &mut Ui<W>, buttons: &[MenuBtn], x: i32, w: i32, fonts: &Fonts, cx: i32, cy: i32) {
    let font = u.hud.font(u.assets, "DXFonts.MainMenuTrueType").unwrap_or_else(|| fonts.title.clone());
    for (i, b) in buttons.iter().enumerate() {
        menu_button(u, W::MenuBtn(i as u8), cx + x, cy + b.y, w, &b.label, b.enabled, &font);
    }
}

/// MenuUIScreenWindow: choice rows (enum value boxes, sliders), the colours example, an open drop-down, the
/// brightness bars.
fn draw_choices(u: &mut Ui<W>, choices: &[Choice], f: &Frame, open_drop: &mut Option<(u8, i32)>, cx: i32, cy: i32) {
    for (i, c) in choices.iter().enumerate() {
        let y = cy + 27 + i as i32 * 36 - c.voff;
        action_button(u, W::ChoiceBtn(i as u8), cx + 7, y + c.voff, 243, &c.label, true);
        match &c.kind {
            ChoiceKind::Enum(_) => {
                info_button(u, cx + 7 + c.info_x, y, c.info_w, &c.value_text(), m_btn(0));
                if c.dropdown {
                    let st = if open_drop.map(|d| d.0) == Some(i as u8) { "Pressed" } else { "Normal" };
                    u.draw(&format!("MenuVScrollDownButton_{st}"), cx + 7 + c.info_x + c.info_w - 17, y + 2, 15, 15, Style::Masked, m_col(3));
                }
                u.hit(W::ChoiceVal(i as u8), (cx + 7 + c.info_x, y, c.info_w, 19), true);
            }
            ChoiceKind::Slider { .. } => {
                let wide = c.key != "AdjustBrightness";
                slider(u, W::Slider(i as u8), cx + 7 + c.control_x, y, c, 177, wide);
            }
            ChoiceKind::Action { .. } => {}
        }
    }
    // MenuScreenAdjustColors: the MenuScreenAdjustColorsExample window at (395, 25) in the client, a
    // sample HUD window in the current HUD theme.
    if f.class == "MenuScreenAdjustColors" {
        draw_colors_example(u, cx + 395, cy + 25);
    }
    // An open drop-down: the list under (or, near the bottom, over) its value box.
    if let Some((di, mut scroll)) = *open_drop {
        if let Some(c) = choices.get(di as usize) {
            if let ChoiceKind::Enum(items) = &c.kind {
                let (bx, by) = (cx + 7 + c.info_x, cy + 27 + di as i32 * 36 - c.voff);
                let rows: Vec<Vec<String>> = items.iter().map(|t| vec![t.clone()]).collect();
                let h = (rows.len() as i32 * ROW_H).min(10 * ROW_H) + 2;
                let bottom = cy + f.ch;
                let ly = if by + 19 + h <= bottom { by + 19 } else { (by - h).max(cy) };
                u.c.fill(bx, ly, c.info_w, h, Style::Normal, BLACK);
                u.c.fill(bx, ly, c.info_w, 1, Style::Normal, m_col(3));
                u.c.fill(bx, ly + h - 1, c.info_w, 1, Style::Normal, m_col(3));
                u.c.fill(bx, ly, 1, h, Style::Normal, m_col(3));
                u.c.fill(bx + c.info_w - 1, ly, 1, h, Style::Normal, m_col(3));
                let sel = Some(c.value.max(0.0) as usize);
                u.hit(W::Header(50), (bx, ly, c.info_w, h), true);
                list(u, 50, (bx + 1, ly + 1, c.info_w - 2, h - 2), &[c.info_w - 2], &rows, sel, &mut scroll, false);
            }
        }
        *open_drop = Some((di, scroll));
    }
    if f.class == "MenuScreenBrightness" {
        // Colour bars that should fade smoothly to black at the bottom.
        for (k, col) in [[255u8, 255, 255], [255, 0, 0], [0, 255, 0], [0, 0, 255]].iter().enumerate() {
            u.draw("MenuBrightnessGradient", cx + 14 + k as i32 * 136, cy + 67, 130, 137, Style::Normal, *col);
        }
        u.c.fill(cx + 14, cy + 178, 520, 1, Style::Normal, m_col(3));
    }
}

/// MenuScreenNewGame: names, portrait (LDDP portraits and checkboxes), skills list and info, upgrade buttons.
fn draw_new_game(u: &mut Ui<W>, ng: &mut NewGame, focus: Option<W>, fonts: &Fonts, cx: i32, cy: i32) {
    let ft = fonts.title.clone();
    let fs = fonts.small.clone();
    let fh = fonts.headers.clone();
    let d = u.assets.classes.defaults("MenuScreenNewGame");
    let l = |k: &str| pstr(&d, k);
    u.text(&ft, cx + 21, cy + 17, 120, ft.height, HAlign::Left, VAlign::Top, m_col(8), &l("HeaderCodeNameLabel"));
    u.text(&ft, cx + 21, cy + 73, 120, ft.height, HAlign::Left, VAlign::Top, m_col(8), &l("HeaderNameLabel"));
    u.text(&ft, cx + 21, cy + 133, 120, ft.height, HAlign::Left, VAlign::Top, m_col(8), &l("HeaderAppearanceLabel"));
    u.text(&ft, cx + 172, cy + 17, 120, ft.height, HAlign::Left, VAlign::Top, m_col(8), &l("HeaderSkillsLabel"));
    u.text(&fs, cx + 430, cy + 18, 80, fs.height, HAlign::Left, VAlign::Top, m_col(8), &l("HeaderSkillLevelLabel"));
    u.text(&fs, cx + 505, cy + 18, 80, fs.height, HAlign::Left, VAlign::Top, m_col(8), &l("HeaderPointsNeededLabel"));
    u.text(&ft, cx + 409, cy + 344, 80, ft.height, HAlign::Left, VAlign::Top, m_col(8), &l("HeaderSkillPointsLabel"));
    // Code name (fixed) and real name.
    let mut code = Edit::new(32);
    code.text = "JC Denton".chars().collect();
    menu_edit(u, W::Edit(9), cx + 18, cy + 36, 113, &mut code, false, false);
    menu_edit(u, W::Edit(0), cx + 18, cy + 92, 113, &mut ng.name, focus == Some(W::Edit(0)), true);
    // Portrait and arrows.
    if crate::travel::lddp() {
        let sex = if ng.portrait % 2 == 1 { "Female" } else { "Male" };
        if let Some(t) = u.hud.tex(u.assets, &format!("FemJC.MenuPlayerSetupJCDenton{sex}_{}", ng.portrait / 2 + 1)) {
            u.c.texture(cx + 18, cy + 152, 114, 161, 0, 0, &t, Style::Masked, WHITE);
        }
        // MenuScreenNewGame.CreateLDDPCheckboxes (NewGameCheckBoxWindow at 3,342 and 3,362).
        let boxes = [
            (20u8, 342, "Original Morpheus?", ng.retro_morpheus, true),
            (21, 362, "Original Interactions?", ng.male_interactions, ng.portrait % 2 == 1),
        ];
        for (id, y, label, on, shown) in boxes {
            if !shown {
                continue;
            }
            let tex = if on { "MenuCheckBox_On" } else { "MenuCheckBox_Off" };
            u.draw(tex, cx + 3, cy + y, 13, 13, Style::Masked, m_col(3));
            u.text(&fs, cx + 20, cy + y, 135, 13, HAlign::Left, VAlign::Center, m_col(8), label);
            u.hit(W::Header(id), (cx + 3, cy + y - 1, 150, 15), true);
        }
    } else {
        u.draw(&format!("MenuNewGameJCDenton_{}", ng.portrait + 1), cx + 18, cy + 152, 114, 161, Style::Masked, WHITE);
    }
    u.hit(W::Portrait, (cx + 18, cy + 152, 114, 161), true);
    let lp = if u.pressed == Some(W::PortraitPrev) { "Pressed" } else { "Normal" };
    let rp = if u.pressed == Some(W::PortraitNext) { "Pressed" } else { "Normal" };
    u.draw(&format!("MenuLeftArrow_{lp}"), cx + 102, cy + 316, 14, 15, Style::Masked, m_col(3));
    u.draw(&format!("MenuRightArrow_{rp}"), cx + 117, cy + 316, 14, 15, Style::Masked, m_col(3));
    u.hit(W::PortraitPrev, (cx + 102, cy + 316, 14, 15), true);
    u.hit(W::PortraitNext, (cx + 117, cy + 316, 14, 15), true);
    // Skills list (3 columns, name in FontMenuHeaders, cost right-aligned).
    let rh = fh.height.max(fs.height) + 1;
    for (row, &i) in ng.order.iter().enumerate() {
        let s = &ng.skills.list[i];
        let y = cy + 41 + row as i32 * rh;
        if y + rh > cy + 41 + 150 {
            break;
        }
        let sel = ng.sel == Some(row);
        if sel {
            u.c.fill(cx + 172, y, 397, rh, Style::Normal, m_col(10));
        }
        let col = if sel { m_col(9) } else { m_col(8) };
        u.c.text(&fh, cx + 174, y, 262, rh, HAlign::Left, VAlign::Center, col, &s.def.name);
        u.c.text(&fs, cx + 172 + 262, y, 66, rh, HAlign::Left, VAlign::Center, col, imsim_core::skills::LEVEL_NAMES[s.level.min(3)]);
        let cost = if s.level >= 3 { "--".to_string() } else { s.def.cost[s.level].to_string() };
        u.c.text(&fs, cx + 172 + 328, y, 58, rh, HAlign::Right, VAlign::Center, col, &cost);
        u.hit(W::Row(0, row), (cx + 172, y, 397, rh), true);
    }
    // Skill info.
    if let Some(&i) = ng.sel.and_then(|r| ng.order.get(r)) {
        let s = ng.skills.list[i].def.clone();
        if !s.icon.is_empty() {
            if let Some(t) = u.hud.tex(u.assets, &s.icon) {
                u.c.texture(cx + 165 + 3, cy + 208 + 20, 24, 24, 0, 0, &t, Style::Masked, WHITE);
            }
        }
        u.c.text(&fh, cx + 165 + 39, cy + 208 + 2, 300, 12, HAlign::Left, VAlign::Top, m_col(3), &s.name);
        let (dx, dy, dw, dh) = (cx + 165 + 39, cy + 208 + 20, 362, 104);
        let lines = wrap(&fs, &label(&s.description), dw - 15);
        let content = lines.len() as i32 * fs.height;
        let inner = scrollbar(u, 1, dx, dy, dw, dh, content, &mut ng.desc_scroll);
        let lines = if inner == dw { wrap(&fs, &label(&s.description), dw) } else { lines };
        u.c.set_clip(Some((dx, dy, inner, dh)));
        for (k, l) in lines.iter().enumerate() {
            u.c.text(&fs, dx, dy + k as i32 * fs.height - ng.desc_scroll, inner, fs.height, HAlign::Left, VAlign::Top, m_col(3), l);
        }
        u.c.set_clip(None);
        u.hit(W::ScrollTrack(101), (dx, dy, inner, dh), true);
    }
    let cls = ng.sel.and_then(|r| ng.order.get(r)).map(|&i| ng.skills.list[i].def.class.clone());
    let can_up = cls.as_ref().is_some_and(|c| ng.skills.can_upgrade(c));
    let can_down = cls.as_ref().is_some_and(|c| ng.skills.level(c) > 0);
    action_button(u, W::Upgrade, cx + 164, cy + 341, 74, &l("ButtonUpgradeLabel"), can_up);
    action_button(u, W::Downgrade, cx + 241, cy + 341, 90, &l("ButtonDowngradeLabel"), can_down);
    info_button(u, cx + 487, cy + 341, 83, &ng.skills.points_avail.to_string(), m_btn(0));
}

/// Controller Bindings: actions and their pad buttons, with the button glyphs.
fn draw_pad_keys(u: &mut Ui<W>, k: &mut Keys, cx: i32, cy: i32, waiting_keys: bool) {
    let d = u.assets.classes.defaults("MenuScreenCustomizeKeys");
    header_button(u, W::Header(0), cx + 10, cy + 3, 162, &pstr(&d, "strHeaderActionLabel"));
    header_button(u, W::Header(1), cx + 175, cy + 3, 157, &pstr(&d, "strHeaderAssignedLabel"));
    let none = pstr(&d, "NoneText");
    let kind = crate::gamepad::glyph_kind();
    let rows: Vec<Vec<String>> = crate::gamepad::PAD_ACTIONS
        .iter()
        .enumerate()
        .map(|(i, (n, _, _))| {
            let b = k.pad_map.get(i).copied().flatten().map(|b| crate::gamepad::pad_name(kind, b)).unwrap_or_else(|| none.clone());
            let b = if waiting_keys && k.sel == Some(i) { "???".into() } else { b };
            vec![n.to_string(), b]
        })
        .collect();
    list(u, 0, (cx + 11, cy + 23, 369, 268), &[164, 205], &rows, k.sel, &mut k.scroll, false);
    // The button's glyph after its name (compact plates to fit the 12-unit rows).
    let f = u.f.small.clone();
    u.c.set_clip(Some((cx + 11, cy + 23, 369 - 16, 268)));
    for (i, row) in rows.iter().enumerate() {
        let Some(b) = k.pad_map.get(i).copied().flatten() else { continue };
        if waiting_keys && k.sel == Some(i) {
            continue;
        }
        let ry = cy + 23 + i as i32 * crate::menuui::ROW_H - k.scroll;
        if ry + crate::menuui::ROW_H <= cy + 23 || ry >= cy + 23 + 268 {
            continue;
        }
        let gx = cx + 11 + 164 + 2 + f.width(&row[1]) + 5;
        crate::glyphs::draw_small(&mut u.c, kind, b, gx, ry);
    }
    u.c.set_clip(None);
}

/// Settings > Enhancements: the search field and the grouped list.
fn draw_enhance(u: &mut Ui<W>, en: &mut Enhance, cx: i32, cy: i32) {
    header_button(u, W::Header(0), cx + 10, cy + 3, 62, "Search");
    // The field sits over the Customize Keys frame's two header wells: black out the art under it, or the
    // first well's bevelled end shows through midway along the field.
    u.c.fill(cx + 73, cy + 3, 262, 18, Style::Normal, BLACK);
    edit_field(u, W::Edit(2), cx + 75, cy + 3, 257, &mut en.search, true, true);
    let shown = en.shown();
    let rows: Vec<Vec<String>> = shown
        .iter()
        .map(|&r| {
            let (l, v) = en.row_text(r);
            vec![l, v]
        })
        .collect();
    let sel = en.sel.and_then(|s| shown.iter().position(|&i| i == s));
    if rows.is_empty() {
        let f = u.f.small.clone();
        u.c.text(&f, cx + 14, cy + 26, 360, ROW_H, HAlign::Left, VAlign::Center, m_col(8), "Nothing matches the search.");
    }
    list(u, 0, (cx + 11, cy + 23, 369, 268), &[240, 129], &rows, sel, &mut en.scroll, false);
}

/// The main menu's Achievements screen: every achievement with who unlocked it.
fn draw_achieve(u: &mut Ui<W>, av: &mut AchieveView, fonts: &Fonts, cx: i32, cy: i32) {
    let fh = fonts.headers.clone();
    let total = crate::achievements::LIST.len();
    let any = crate::achievements::LIST.iter().filter(|a| av.chars.iter().any(|c| c.unlocked.contains_key(a.0)) || av.global.contains_key(a.0)).count();
    header_button(u, W::Header(0), cx + 10, cy + 3, 222, "Achievement");
    header_button(u, W::Header(1), cx + 235, cy + 3, 97, &format!("{any} / {total}"));
    let (lx, ly, lw, lh) = (cx + 11, cy + 23, 369, 268);
    let content = total as i32 * crate::achievements::ROW;
    let inner = scrollbar(u, 0, lx, ly, lw, lh, content, &mut av.scroll);
    let head = fh.clone();
    let small = u.f.small.clone();
    crate::achievements::draw_all(&mut u.c, &head, &small, (lx, ly, inner, lh), av.scroll, &av.chars, &av.global);
    if av.chars.is_empty() {
        u.c.text(&small, lx + 4, ly + lh - 14, inner - 8, 12, HAlign::Center, VAlign::Center, m_col(8), "No character has unlocked any yet.");
    }
    // Rows for the mouse wheel and the pad's snap.
    for i in 0..total {
        let ry = ly + i as i32 * crate::achievements::ROW - av.scroll;
        let (top, bot) = (ry.max(ly), (ry + crate::achievements::ROW).min(ly + lh));
        if bot - top > 0 {
            u.hit(W::Row(0, i), (lx, top, inner, bot - top), true);
        }
    }
}

/// MenuScreenCustomizeKeys: actions and their keys.
fn draw_keys(u: &mut Ui<W>, k: &mut Keys, cx: i32, cy: i32, waiting_keys: bool) {
    let d = u.assets.classes.defaults("MenuScreenCustomizeKeys");
    header_button(u, W::Header(0), cx + 10, cy + 3, 162, &pstr(&d, "strHeaderActionLabel"));
    header_button(u, W::Header(1), cx + 175, cy + 3, 157, &pstr(&d, "strHeaderAssignedLabel"));
    let none = pstr(&d, "NoneText");
    let rows: Vec<Vec<String>> = crate::keybinds::ACTIONS
        .iter()
        .enumerate()
        .map(|(i, (n, _))| {
            let key = k.map.get(i).map(|x| crate::keybinds::input_name(*x)).unwrap_or_else(|| none.clone());
            let key = if waiting_keys && k.sel == Some(i) { "???".into() } else { key };
            vec![n.to_string(), key]
        })
        .collect();
    list(u, 0, (cx + 11, cy + 23, 369, 268), &[164, 205], &rows, k.sel, &mut k.scroll, false);
}

/// MenuScreenSaveGame / LoadGame: the games list (the row being named edits in place), snapshot, info, delete box.
fn draw_save_load(u: &mut Ui<W>, sl: &mut SaveLoad, fonts: &Fonts, cx: i32, cy: i32) {
    let fs = fonts.small.clone();
    let d = u.assets.classes.defaults("MenuScreenLoadGame");
    let l = |k: &str| pstr(&d, k);
    header_button(u, W::Header(0), cx + 10, cy + 3, 238, &l("strHeaderNameLabel"));
    header_button(u, W::Header(1), cx + 251, cy + 3, 131, &l("strHeaderDateLabel"));
    let new_label = l("NewSaveGameButtonText");
    let mut rows: Vec<Vec<String>> = sl.list.iter().map(|r| sl.row_text(*r, &new_label).to_vec()).collect();
    // The row being named shows the typed description.
    let editing = sl.saving.then_some(sl.sel).flatten();
    if let Some(r) = editing {
        if let Some(row) = rows.get_mut(r) {
            row[0] = String::new();
        }
    }
    let (lx, ly, lw, lh) = (cx + 11, cy + 22, 371, 270);
    list(u, 0, (lx, ly, lw, lh), &[240, 131], &rows, sl.sel, &mut sl.scroll, false);
    if let Some(r) = editing {
        let ey = ly + r as i32 * ROW_H - sl.scroll;
        if ey >= ly && ey + ROW_H <= ly + lh {
            u.c.set_clip(Some((lx, ly, lw, lh)));
            edit_field(u, W::Edit(1), lx + 1, ey - 3, 236, &mut sl.edit, true, true);
            u.c.set_clip(None);
        }
    }
    // Snapshot (not kept yet: a dark frame), save info, delete confirmation.
    u.c.fill(cx + 387, cy + 24, 160, 120, Style::Normal, BLACK);
    let sel_path = sl.sel.and_then(|r| sl.game(r)).map(|g| g.path.clone());
    if sl.snap.as_ref().map(|s| &s.0) != sel_path.as_ref() {
        sl.snap = sel_path.map(|p| {
            let t = crate::save::snapshot(&p).map(|b| {
                crate::uicanvas::UiTex::from_rgba(crate::save::SNAP_W as u32, crate::save::SNAP_H as u32, &b)
            });
            (p, t)
        });
    }
    if let Some((_, Some(t))) = &sl.snap {
        u.c.texture(cx + 387, cy + 24, 160, 120, 0, 0, t, Style::Normal, WHITE);
    }
    for (x, y, w, h) in [(387, 24, 160, 1), (387, 143, 160, 1), (387, 24, 1, 120), (546, 24, 1, 120)] {
        u.c.fill(cx + x, cy + y, w, h, Style::Normal, m_col(3));
    }
    let sel_game = sl.sel.and_then(|r| sl.game(r)).cloned();
    if let Some(g) = sel_game {
        let info = format!(
            "{}{}{}{}",
            l("LocationLabel").replace("%s", &g.location),
            l("SaveCountLabel").replace("%d", &g.save_count.to_string()),
            l("PlayTimeLabel").replace("%s", &crate::save::format_play_time(g.play_time)),
            format!("File Size: {}KB", g.bytes.div_ceil(1024))
        );
        u.wrapped(&fs, cx + 390, cy + 166, 155, 60, HAlign::Left, VAlign::Center, m_col(8), &info);
    }
    let tex = if sl.confirm_delete { "MenuCheckBox_On" } else { "MenuCheckBox_Off" };
    u.draw(tex, cx + 389, cy + 256, 13, 13, Style::Masked, m_col(3));
    u.text(&fs, cx + 406, cy + 256, 150, 13, HAlign::Left, VAlign::Center, m_col(8), &l("ConfirmDeleteLabel"));
    u.hit(W::Header(2), (cx + 389, cy + 255, 160, 15), true);
}

/// HUDKeypadWindow.DrawWindow + HUDKeypadButton.DrawWindow, in the HUD theme.
fn draw_keypad(u: &mut Ui<W>, k: &KeypadWin, fonts: &Fonts) {
    let bg_style = if crate::settings::hud_translucent() { Style::Translucent } else { Style::Masked };
    let border_style = if crate::settings::hud_border_translucent() { Style::Translucent } else { Style::Masked };
    u.draw("HUDKeypadBackground", 0, 0, 103, 162, bg_style, h_col(0));
    if crate::settings::hud_borders() {
        u.draw("HUDKeypadBorder", 0, 0, 103, 162, border_style, h_col(1));
    }
    // The code display: a ~ per digit of the right code; typed ones in the header colour, the rest dim (|p5).
    let small = fonts.small.clone();
    match k.wait {
        Some((_, ok)) => {
            let (txt, col) = if ok { ("~~GRANTED~~", [0, 255, 0]) } else { ("~~DENIED~~", [255, 0, 0]) };
            u.text(&small, 17, 21, 75, 11, HAlign::Center, VAlign::Center, col, txt);
        }
        None => {
            let n = k.valid.chars().count().max(1);
            let typed = k.input.chars().count().min(n);
            let cw = small.width("~").max(5);
            let x0 = 17 + (75 - cw * n as i32) / 2;
            for i in 0..n {
                let col = if i < typed { h_col(7) } else { [64, 64, 64] };
                u.text(&small, x0 + i as i32 * cw, 21, cw, 11, HAlign::Left, VAlign::Center, col, "~");
            }
        }
    }
    let big = u.hud.font(u.assets, "FontMenuExtraLarge").unwrap_or_else(|| fonts.title.clone());
    for i in 0..12 {
        let (x, y) = ((i % 3) as i32 * 26 + 16, (i / 3) as i32 * 28 + 35);
        let w = W::MenuBtn(i as u8);
        let down = u.pressed == Some(w) && u.hover == Some(w);
        let tex = if down { "HUDKeypadButton_Pressed" } else { "HUDKeypadButton_Normal" };
        u.draw(tex, x, y, 25, 27, bg_style, h_col(0));
        let (ox, oy) = if down { (1, 3) } else { (0, 2) };
        let col = if k.wait.is_some() { [64, 64, 64] } else { [255, 255, 255] };
        u.text(&big, x + ox, y + oy, 25, 27, HAlign::Center, VAlign::Center, col, &key_char(i).to_string());
        u.hit(w, (x, y, 25, 27), true);
    }
    if k.noted && k.wait.is_none() {
        let small = u.f.small.clone();
        if crate::gamepad::pad_in_use() {
            // The pad: Y types it (gamepad.rs sends Tab).
            let kind = crate::gamepad::glyph_kind();
            let gw = crate::glyphs::width_small(kind, crate::gamepad::PadBtn::North);
            let tw = small.width("code from notes");
            let x0 = (103 - gw - 4 - tw) / 2;
            crate::glyphs::draw_small(&mut u.c, kind, crate::gamepad::PadBtn::North, x0, 148);
            u.text(&small, x0 + gw + 4, 148, tw + 2, 12, HAlign::Left, VAlign::Center, h_col(7), "code from notes");
        } else {
            u.text(&small, 0, 148, 103, 12, HAlign::Center, VAlign::Center, h_col(7), "Tab: code from notes");
        }
    }
}

/// MenuUIWindow.CreateMenuEditWindow (the clip sits 5 down, not 4 as on computers).
#[allow(clippy::too_many_arguments)]
fn menu_edit(u: &mut Ui<W>, w: W, x: i32, y: i32, width: i32, e: &mut Edit, focused: bool, enabled: bool) {
    edit_field(u, w, x, y + 1, width, e, focused, enabled);
}

// ------------------------------------------------------------------------------------------------ resource

#[derive(Resource, Default)]
pub struct Menus {
    stack: Vec<Win>,
    built: bool,
    win_layers: Vec<Layer>,
    cursor: Option<Layer>,
    backdrop: Option<Entity>,
    hits: Vec<(W, Rect, bool)>,
    pressed: Option<W>,
    drag: Option<(i32, i32)>,
    last_click: (f32, Option<W>),
    clock: f32,
    paused_game: bool,
    grabbed_before: bool,
    /// The DX.dx flyby.
    intro: Option<(Vec<crate::interp::Point>, crate::interp::Interp)>,
    intro_menu_shown: bool,
    waiting_key: Option<f32>,
    others_last: bool,
    dead_t: f32,
    /// The game a confirmation box is about: (file, description to save).
    box_save: Option<(std::path::PathBuf, String)>,
    applied_gen: u64,
    themes_applied: bool,
    /// Lean Updates (perf.rs): each window layer's last drawing, and the (scale, colour) the cursor was drawn at.
    drawn: Vec<Drawn>,
    cursor_drawn: Option<(i32, Rgb)>,
    /// Ours: the online services the multiplayer screens use.
    mp: MpLink,
}

impl Menus {
    fn push(&mut self, w: Option<Win>) {
        if let Some(w) = w {
            crate::sound::play_player("DeusExSounds.UserInterface.Menu_Activate", 0.25, 0.0);
            self.stack.push(w);
            self.pressed = None;
        }
    }
    fn pop(&mut self) {
        if self.stack.pop().is_some() {
            self.pressed = None;
        }
    }
}

/// What a click or key asked for, applied after the input pass.
enum Act {
    Push(String),
    Pop,
    PopAll,
    Msg(String, String, bool, MsgPurpose),
    Quit,
    NewGame(crate::travel::NewGame),
    ToggleFullscreen,
    DisplayMode(f32),
    VSync(f32),
    Resolution(u32, u32),
    SaveGame(std::path::PathBuf, String),
    LoadGame(std::path::PathBuf),
    /// MenuMain Play Intro (ShowIntro): the intro cut scene, then back to the title map.
    Intro,
    /// StartTrainingMission: 00_Training with a fresh character.
    Training,
    /// Ours: Multiplayer - connect to online services (or go straight in when connected).
    Connect,
    /// Ours: look a room up by its code.
    Join(String),
    /// Ours: join the found room as a saved multiplayer character (its id) / a new one.
    MpLoadChar(String),
    MpNewChar(crate::travel::NewGame),
    /// Ours: the main menu's Leave Multiplayer.
    MpLeave,
    /// Ours: Online Services saved: the lobby's new address (as typed), then connect there.
    MpSetAddr(String),
    /// Ours: join with a direct code (Choose Character first).
    MpDirect(String),
    /// Ours: the connect-failed box's Play Offline.
    MpOffline,
}

/// A new save's description: where the player is (DeusExLevelInfo.MissionLocation), as the original's default.
fn default_save_name() -> String {
    let l = LOCATION.lock().map(|l| l.clone()).unwrap_or_default();
    if l.is_empty() {
        "Saved Game".into()
    } else {
        l
    }
}
static LOCATION: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// ProcessMenuAction for a main-menu style button.
fn menu_action(b: &MenuBtn, _hud: &mut ClassicHud, assets: &mut RuntimeAssets, out: &mut Vec<Act>) {
    match b.action {
        0 | 1 => match b.invoke.as_str() {
            "MenuScreenSaveGame" | "MenuScreenLoadGame" => out.push(Act::Push(b.invoke.clone())),
            // ShowCredits(False): back to this menu afterwards.
            "CreditsWindow" => crate::credits::show_from_menu(),
            "MenuScreenMultiplayer" => out.push(Act::Connect),
            "LeaveMultiplayer" => out.push(Act::Msg("Leave Multiplayer".into(), "Leave the multiplayer game?".into(), true, MsgPurpose::MpLeave)),
            "" => {}
            c => out.push(Act::Push(c.to_string())),
        },
        2 => out.push(Act::Pop),
        3 => {
            // StartNewGame: ask about the training mission once.
            if setting!("AskedToTrain") < 0.5 {
                crate::settings::set("AskedToTrain", 1.0);
                crate::settings::save();
                let d = assets.classes.defaults("MenuMain");
                out.push(Act::Msg(pstr(&d, "AskToTrainTitle"), pstr(&d, "AskToTrainMessage"), true, MsgPurpose::AskToTrain));
            } else {
                out.push(Act::Push("MenuSelectDifficulty".into()));
            }
        }
        // MenuUIWindow.ConfirmTraining / ConfirmIntro: with a game running (a map with a mission, the player not
        // dying or in a cut scene) ask first, as the game would be lost.
        4 | 5 => {
            let running = !intro_map() && !DEAD.load(Ordering::Relaxed) && !crate::cinematic::active();
            let training = b.action == 4;
            if running {
                let d = assets.classes.defaults("MenuMain");
                let key = if training { "TrainingWarningMessage" } else { "IntroWarningMessage" };
                let purpose = if training { MsgPurpose::Training } else { MsgPurpose::Intro };
                out.push(Act::Msg(pstr(&d, "MessageBoxTitle"), pstr(&d, key), true, purpose));
            } else {
                out.push(if training { Act::Training } else { Act::Intro });
            }
        }
        6 => {
            let d = assets.classes.defaults("MenuMain");
            out.push(Act::Msg(pstr(&d, "MessageBoxTitle"), pstr(&d, "ExitMessage"), true, MsgPurpose::Exit));
        }
        7 => {
            // MenuSelectDifficulty.ProcessCustomMenuButton.
            let diff = match b.key.as_str() {
                "EASY" => Some(1.0),
                "MEDIUM" => Some(1.5),
                "HARD" => Some(2.0),
                "REALISTIC" => Some(4.0),
                _ => None,
            };
            if let Some(dv) = diff {
                out.push(Act::Push(format!("MenuScreenNewGame:{dv}")));
            }
        }
        _ => {}
    }
}

/// What changing a choice does at once besides the setting itself.
fn choice_side_effect(c: &Choice) -> Option<Act> {
    match c.key.as_str() {
        "Resolution" => RESOLUTIONS.get(c.value.max(0.0) as usize).map(|&(w, h)| Act::Resolution(w, h)),
        "DisplayMode" => Some(Act::DisplayMode(c.value)),
        "VSync" => Some(Act::VSync(c.value)),
        _ => None,
    }
}

/// Save every choice of a screen (MenuUIScreenWindow.SaveSettings).
fn save_choices(win: &Win) {
    if let Body::Choices { choices } = &win.body {
        for c in choices {
            if !matches!(c.kind, ChoiceKind::Action { .. }) {
                crate::settings::set(&c.key, c.value);
            }
        }
        crate::settings::save();
    }
    if let Body::Enhance(en) = &win.body {
        if en.mode_changed {
            for (k, v) in &en.memory {
                crate::settings::set(k, *v);
            }
            crate::settings::set("ModernMode", if en.modern { 1.0 } else { 0.0 });
        }
        for c in &en.items {
            if !matches!(c.kind, ChoiceKind::Action { .. }) {
                crate::settings::set(&c.key, c.value);
            }
        }
        crate::settings::save();
    }
    if let Body::Keys(k) = &win.body {
        if k.pad {
            crate::gamepad::set_bindings(k.pad_map.clone());
        } else {
            crate::keybinds::set_map(&k.map);
        }
    }
}

/// CancelSetting: put live-previewed values back.
fn cancel_choices(win: &Win) {
    if let Body::Enhance(en) = &win.body {
        for c in en.items.iter().filter(|c| !matches!(c.kind, ChoiceKind::Action { .. })) {
            crate::settings::set(&c.key, c.saved);
        }
    }
    if let Body::Choices { choices } = &win.body {
        for c in choices {
            if !matches!(c.kind, ChoiceKind::Action { .. }) {
                crate::settings::set(&c.key, c.saved);
            }
        }
    }
}

/// Settings that preview live while the screen is up (volumes, brightness, themes, HUD scale). The sound and
/// speech sliders play a test sound at the new level (MenuChoice_SoundVolume / SpeechVolume.ScalePositionChanged:
/// Menu_SoundTest, and Menu_SpeechTest in the talk slot); music is heard as it plays.
fn live_preview(c: &Choice) {
    match c.key.as_str() {
        "SoundVolume" => {
            crate::settings::set(&c.key, c.value);
            crate::sound::play_player("DeusExSounds.UserInterface.Menu_SoundTest", 1.0, 0.0);
        }
        "SpeechVolume" => {
            crate::settings::set(&c.key, c.value);
            crate::sound::stop(crate::sound::PLAYER, crate::sound::slot::TALK);
            crate::sound::play_owned(crate::sound::PLAYER, crate::sound::slot::TALK, "DeusExSounds.UserInterface.Menu_SpeechTest", None, 1.0, 0.0, 1.0);
        }
        _ => {}
    }
    if matches!(
        c.key.as_str(),
        "MusicVolume" | "SoundVolume" | "SpeechVolume" | "AdjustBrightness" | "MenuColor" | "HUDColor" | "HUDScale" | "MenuTranslucency"
    ) {
        crate::settings::set(&c.key, c.value);
    }
}

// ------------------------------------------------------------------------------------------------ system

/// The frame's screen layout in menu units.
#[derive(Clone, Copy)]
struct View {
    /// Canvases' whole-number scale.
    s: i32,
    /// The scale shown, fractional with HUD Scale Large.
    d: f32,
    sf: f32,
    sw: i32,
    sh: i32,
    mouse: (i32, i32),
}

/// Where a window sits: dragged, or centred.
fn win_pos(w: &Win, fonts: &Fonts, sw: i32, sh: i32) -> (i32, i32) {
    let (ww, wh) = win_size(&w.f, fonts);
    w.pos.unwrap_or((sw / 2 - ww / 2, sh / 2 - wh / 2))
}

/// One frame's input pass: the borrowed state, what the pointer is over, and the actions asked for.
struct InCx<'a> {
    st: &'a mut Menus,
    binds: &'a mut Bindings,
    hud: &'a mut ClassicHud,
    assets: &'a mut RuntimeAssets,
    fonts: &'a Fonts,
    view: View,
    acts: Vec<Act>,
    top: usize,
    hover: Option<(W, bool)>,
    hover_w: Option<W>,
    click_focus: bool,
    shift: bool,
    ctrl: bool,
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn menu_system(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut vtime: ResMut<Time<Virtual>>,
    mut window: Single<&mut Window>,
    mut cursor_opts: Single<&mut CursorOptions>,
    mut st: ResMut<Menus>,
    mut binds: ResMut<Bindings>,
    mut wheel: MessageReader<MouseWheel>,
    (mut hud, mut assets, mut images, mut layers): (
        ResMut<ClassicHud>,
        ResMut<RuntimeAssets>,
        ResMut<Assets<Image>>,
        ResMut<Assets<HudLayer>>,
    ),
    (level, root, ph, mut player): (
        Res<LevelRes>,
        Res<crate::pickups::GameRoot>,
        Res<crate::ai::PlayerHealth>,
        ResMut<crate::Player>,
    ),
    mut nodes: Query<(&mut bevy::ui::Node, &mut Visibility, &MaterialNode<HudLayer>), Without<MenuBackdrop>>,
    mut backdrop: Query<(&mut Visibility, &mut BackgroundColor), (With<MenuBackdrop>, Without<MaterialNode<HudLayer>>)>,
    mut cam: Query<&mut Transform, (With<FlyCam>, Without<SkyCam>)>,
    mut sky: Query<&mut Transform, (With<SkyCam>, Without<FlyCam>)>,
    mut exit: MessageWriter<AppExit>,
) {
    // The credits scroll has the screen and the keys.
    if crate::credits::open() {
        return;
    }
    let _prof = crate::prof::scope("menus::menu_system");
    let st = &mut *st;
    let dt = time.delta_secs();
    st.clock += dt;
    if !st.built {
        build_once(st, &mut commands, &mut images, &mut layers, &level);
    }
    follow_themes(st, &mut assets);
    intro_flyby(st, &vtime, &mut cam, &mut sky, &mut player, &mut binds, &mut hud, &mut assets);
    dev_menu_open(st, &vtime, &mut hud, &mut assets);
    // ---- Escape in game opens the main menu (ShowMainMenu), unless another screen has the keys
    let others = crate::inventory_screen::SCREEN_OPEN.load(Ordering::Relaxed)
        || crate::computers::open()
        || binds.open
        || crate::consys::third_person();
    // The Escape that closed another screen doesn't also open the menu.
    let others_before = std::mem::replace(&mut st.others_last, others);
    open_from_game(st, &binds, &ph, dt, others || others_before, &mut hud, &mut assets);
    keypad_tick(st, dt);
    quick_save_load(st, &binds, &ph, others || others_before, &mut hud, &mut assets);
    mp_tick(st, dt, &mut hud, &mut assets);
    let was_open = OPEN.load(Ordering::Relaxed);
    let is_open = !st.stack.is_empty();
    // The key that opened the menu this frame (Escape) mustn't also close it.
    let opened_now = is_open && !was_open;
    pause_and_backdrop(st, &mut vtime, &mut cursor_opts, &mut backdrop, is_open, was_open);
    if !is_open {
        hide_closed(st, &mut nodes, &mut wheel);
        return;
    }
    binds.capture = true;

    // ---- layout
    let Some(fonts) = menu_fonts(&mut hud, &mut assets) else {
        return;
    };
    let (pw, phh) = (window.physical_width() as i32, window.physical_height() as i32);
    // (s: canvases' whole-number scale; d: the scale shown, fractional with HUD Scale Large.)
    let (s, d) = crate::hud::screen_scales(&window, 640, 480);
    let (sw, sh) = ((pw as f32 / d) as i32, (phh as f32 / d) as i32);
    let sf = window.scale_factor();
    let mouse = window
        .cursor_position()
        .map(|c| ((c.x * sf / d).floor() as i32, (c.y * sf / d).floor() as i32))
        .unwrap_or((-100, -100));
    let view = View { s, d, sf, sw, sh, mouse };
    let top = st.stack.len() - 1;

    // ---- input
    let hover = st.hits.iter().rev().find(|(_, r, _)| inside(mouse, *r)).map(|h| (h.0, h.2));
    let hover_w = hover.map(|h| h.0);
    if env_on!("DXVIEW_SNAPLOG") {
        eprintln!("SNAP mouse {mouse:?} hover {hover:?} hits {:?}", st.hits.iter().map(|h| (h.0, h.1)).collect::<Vec<_>>());
    }
    let shift = binds.keys.pressed(KeyCode::ShiftLeft) || binds.keys.pressed(KeyCode::ShiftRight);
    let ctrl = binds.keys.pressed(KeyCode::ControlLeft) || binds.keys.pressed(KeyCode::ControlRight);
    let mut acts: Vec<Act> = Vec::new();
    if CLOSE_ALL.swap(false, Ordering::Relaxed) && st.waiting_key.is_none() {
        acts.push(Act::PopAll);
    }
    let click_focus = matches!(st.stack[top].body, Body::NewGame(_) | Body::Keypad(_));
    let mut cx = InCx {
        st: &mut *st,
        binds: &mut binds,
        hud: &mut hud,
        assets: &mut assets,
        fonts: &fonts,
        view,
        acts,
        top,
        hover,
        hover_w,
        click_focus,
        shift,
        ctrl,
    };
    hover_focus(&mut cx);
    if cx.st.waiting_key.is_some() {
        capture_binding(&mut cx, dt);
    }
    let waiting = cx.st.waiting_key.is_some();
    let drop_click = !waiting && drop_down_click(&mut cx);
    if !waiting && !drop_click && cx.binds.buttons.just_pressed(MouseButton::Left) {
        if let Some(k) = left_press(&mut cx) {
            save_load_action(&mut *cx.st, top, k, &mut *cx.hud, &mut *cx.assets, &mut cx.acts);
        }
    }
    if cx.binds.buttons.pressed(MouseButton::Left) {
        left_held(&mut cx);
    }
    release_and_wheel(&mut cx, &mut wheel);
    // Keys.
    if !waiting && !opened_now {
        typed_keys(&mut cx);
    }
    let InCx { acts, .. } = cx;

    // ---- apply
    apply_acts(acts, st, &binds, &mut window, &root, &mut exit, &mut hud, &mut assets);
    if st.stack.is_empty() {
        return;
    }

    // ---- draw: the top window and the ones it doesn't hide
    let lean = crate::perf::lean_updates();
    draw_windows(st, &mut hud, &mut assets, &mut images, &mut layers, &mut nodes, &fonts, view, hover_w, waiting, lean);
    draw_cursor(st, &mut hud, &mut assets, &mut images, &mut layers, &mut nodes, &window, view, lean);
    let _: Rgb = WHITE;
}

/// The first frame on a map: mission number / location, the backdrop and window layers, the DX.dx flyby path.
fn build_once(
    st: &mut Menus,
    commands: &mut Commands,
    images: &mut ResMut<Assets<Image>>,
    layers: &mut ResMut<Assets<HudLayer>>,
    level: &LevelRes,
) {
    st.built = true;
    let mission = level.0.level_info()
        .map(|a| pint(&a.props, "missionNumber", 0))
        .unwrap_or(0);
    MISSION.store(mission, Ordering::Relaxed);
    TRAINING.store(level.0.level_info().is_some() && mission == 0, Ordering::Relaxed);
    if let Some(Value::Str(loc)) = level.0.level_info().and_then(|a| find(&a.props, "MissionLocation")) {
        if let Ok(mut l) = LOCATION.lock() {
            *l = loc.clone();
        }
    }
    let e = commands
        .spawn((
            bevy::ui::Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
            Visibility::Hidden,
            Pickable::IGNORE,
            GlobalZIndex(60),
            MenuBackdrop,
        ))
        .id();
    st.backdrop = Some(e);
    for z in 0..3 {
        st.win_layers.push(layer(commands, images, layers, 61 + z));
    }
    st.cursor = Some(layer(commands, images, layers, 70));
    // DX.dx: the camera flies the IntroCam path from Position 1 (TravelPostAccept).
    if mission == -2 && !env_on!("DXVIEW_SHOT") {
        let pts = crate::interp::path(&level.0, "IntroCam");
        if let Some(first) = crate::interp::at_position(&pts, 1) {
            let it = crate::interp::Interp::start(&pts, first);
            st.intro = Some((pts, it));
        }
    }
}

/// The menu / HUD colour themes follow the settings (once at first, then whenever they change).
fn follow_themes(st: &mut Menus, assets: &mut ResMut<RuntimeAssets>) {
    if !st.themes_applied {
        st.themes_applied = true;
        apply_themes(assets, setting!("MenuColor") as usize, setting!("HUDColor") as usize);
    }
    // Live settings: themes follow the setting.
    let gen = crate::settings::generation();
    if gen != st.applied_gen {
        st.applied_gen = gen;
        apply_themes(assets, setting!("MenuColor") as usize, setting!("HUDColor") as usize);
    }
}

/// The logo flyby (DeusExPlayer state Interpolating): the camera rides the path, Fire shows the menu.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn intro_flyby(
    st: &mut Menus,
    vtime: &ResMut<Time<Virtual>>,
    cam: &mut Query<&mut Transform, (With<FlyCam>, Without<SkyCam>)>,
    sky: &mut Query<&mut Transform, (With<SkyCam>, Without<FlyCam>)>,
    player: &mut ResMut<crate::Player>,
    binds: &mut ResMut<Bindings>,
    hud: &mut ResMut<ClassicHud>,
    assets: &mut ResMut<RuntimeAssets>,
) {
    let Some((pts, it)) = st.intro.as_mut() else { return };
    it.tick(pts, vtime.delta_secs_f64());
    let to_rad = |u: f64| (u / 65536.0 * std::f64::consts::TAU) as f32;
    let rot = Quat::from_rotation_y(-to_rad(it.rot[1]) - std::f32::consts::FRAC_PI_2)
        * Quat::from_rotation_x(to_rad(it.rot[0]))
        * Quat::from_rotation_z(-to_rad(it.rot[2]));
    // PlayerCalcView: the camera is the player's location + EyeHeight (JCDentonMale BaseEyeHeight 40; only
    // CameraPoint cut scenes zero it), so the logos sit lower in the frame than the path points alone.
    let eye = Vec3::from(dx_level::ue_to_bevy([it.loc.x as f32, it.loc.y as f32, it.loc.z as f32 + 40.0]));
    for mut t in cam.iter_mut() {
        t.translation = eye;
        t.rotation = rot;
    }
    for mut t in sky.iter_mut() {
        t.rotation = rot;
    }
    player.body.location = it.loc;
    player.body.velocity = glam::DVec3::ZERO;
    player.noclip = true;
    binds.capture = true;
    if st.stack.is_empty() {
        let at_rest = it.target.is_some_and(|t| pts[t].rate == 0.0 || pts[t].next.is_some_and(|n| pts[n].rate == 0.0)) && it.alpha > 0.9;
        let clicked = binds.buttons.just_pressed(MouseButton::Left)
            || binds.keys.just_pressed(KeyCode::Escape)
            || binds.keys.just_pressed(KeyCode::Enter)
            || binds.keys.just_pressed(KeyCode::Space);
        if clicked || (at_rest && !st.intro_menu_shown) {
            st.intro_menu_shown = true;
            let w = open_window(hud, assets, "MenuMain");
            st.push(w);
            // Ours: the welcome window over it, until "Don't Show Again".
            if crate::welcome::due() {
                let w = open_window(hud, assets, "MenuScreenWelcome");
                st.push(w);
            }
        }
    }
}

/// Developer aid: DXVIEW_MENU_OPEN="MenuMain,MenuSettings,MenuScreenDisplay" opens those windows in turn after
/// two seconds (screenshots of the menus).
fn dev_menu_open(st: &mut Menus, vtime: &ResMut<Time<Virtual>>, hud: &mut ResMut<ClassicHud>, assets: &mut ResMut<RuntimeAssets>) {
    if let Ok(v) = std::env::var("DXVIEW_MENU_OPEN") {
        static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !DONE.load(Ordering::Relaxed) && vtime.elapsed_secs() > 2.0 {
            DONE.store(true, Ordering::Relaxed);
            st.intro = None;
            for c in v.split(',') {
                // "MenuScreenNewGame#3": the new-game screen on portrait 3.
                let (c, portrait) = c.split_once('#').map(|(a, b)| (a, b.parse().ok())).unwrap_or((c, None));
                let mut w = open_window(hud, assets, c.trim());
                if let (Some(Body::NewGame(ng)), Some(p)) = (w.as_mut().map(|w| &mut w.body), portrait) {
                    ng.portrait = p;
                }
                st.push(w);
            }
        }
    }
}

/// Windows asked for from the game: the main menu on Escape (or Fire when dead), the controller layout, a keypad's
/// message box. `others`: another screen has (or just had) the keys.
fn open_from_game(
    st: &mut Menus,
    binds: &ResMut<Bindings>,
    ph: &crate::ai::PlayerHealth,
    dt: f32,
    others: bool,
    hud: &mut ResMut<ClassicHud>,
    assets: &mut ResMut<RuntimeAssets>,
) {
    // Dead (DeusExPlayer state Dying): Escape or Fire brings up the menu to load a game.
    DEAD.store(ph.dead, Ordering::Relaxed);
    st.dead_t = if ph.dead { st.dead_t + dt } else { 0.0 };
    let dead_click = ph.dead && st.dead_t > 2.0 && binds.buttons.just_pressed(MouseButton::Left);
    if st.stack.is_empty() && st.intro.is_none() && !others && (binds.keys.just_pressed(KeyCode::Escape) || dead_click) {
        let w = open_window(hud, assets, "MenuMain");
        st.push(w);
    }
    if PADMAP_OPEN.swap(false, Ordering::Relaxed) {
        let w = pad_map_box(hud, assets);
        st.push(w);
    }
    // Deathmatch, After the Match = Back to Room: the host chooses the next match.
    if crate::deathmatch::take_room_open() {
        let w = open_window(hud, assets, "MenuScreenMatchSettings");
        st.push(w);
    }
    // A keypad was used (Keypad.ActivateKeypadWindow).
    if let Some((title, text)) = EXTERNAL_BOX.lock().ok().and_then(|mut b| b.take()) {
        let w = msg_box(hud, assets, &title, &text, true, MsgPurpose::External);
        st.push(w);
    }
}

/// HUDKeypadWindow: open one asked for, the pad's notes code, buttons pressed in the world, the result delay, and
/// what diegetic.rs reads of it.
fn keypad_tick(st: &mut Menus, dt: f32) {
    if let Some((blocker, valid, instant)) = KEYPAD_OPEN.lock().ok().and_then(|mut k| k.take()) {
        let f = Frame { class: "HUDKeypadWindow".into(), cw: 103, ch: 162, ..default() };
        let noted = in_notes(&valid);
        let mut k = KeypadWin { blocker, valid, input: String::new(), wait: None, instant, noted };
        if k.instant {
            k.input = k.valid.clone();
            keypad_validate(&mut k);
        }
        st.stack.push(Win { f, body: Body::Keypad(k), pos: None, focus: None, hides_below: true, drop: None });
    }
    // The pad's Y on a keypad: the code from the notes (gamepad.rs).
    if crate::gamepad::PAD_NOTES.load(std::sync::atomic::Ordering::Relaxed) {
        if let Some(Body::Keypad(k)) = st.stack.last_mut().map(|w| &mut w.body) {
            if k.noted && k.wait.is_none() {
                keypad_from_notes(k);
            }
        }
    }
    // Buttons pressed on the keypad in the world (diegetic.rs).
    let presses: Vec<usize> = KEYPAD_PRESSES.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    if let Some(Body::Keypad(k)) = st.stack.last_mut().map(|w| &mut w.body) {
        for i in presses {
            keypad_press(k, i);
        }
    }
    // HUDKeypadWindow.KeypadDelay: a second on the result, then close (right) or start over (wrong).
    if let Some(Body::Keypad(k)) = st.stack.last_mut().map(|w| &mut w.body) {
        if let Some((t, ok)) = k.wait.as_mut() {
            *t -= dt;
            if *t <= 0.0 {
                let ok = *ok;
                k.wait = None;
                if ok {
                    st.stack.pop();
                } else {
                    k.input.clear();
                }
            }
        }
    }
    if let Ok(mut now) = KEYPAD_NOW.lock() {
        *now = match st.stack.last().map(|w| &w.body) {
            Some(Body::Keypad(k)) => Some((k.blocker, k.input.clone(), k.wait.map(|w| w.1), k.valid.chars().count())),
            _ => None,
        };
        let noted = matches!(st.stack.last().map(|w| &w.body), Some(Body::Keypad(k)) if k.noted && k.wait.is_none() && k.input != k.valid);
        KEYPAD_NOTED.store(noted, std::sync::atomic::Ordering::Relaxed);
    }
}

/// DeusExPlayer.QuickSave / QuickLoad (ConfirmQuickLoad asks first). `others`: another screen has (or just had)
/// the keys.
fn quick_save_load(
    st: &mut Menus,
    binds: &ResMut<Bindings>,
    ph: &crate::ai::PlayerHealth,
    others: bool,
    hud: &mut ResMut<ClassicHud>,
    assets: &mut ResMut<RuntimeAssets>,
) {
    if st.stack.is_empty() && st.intro.is_none() && !others {
        if binds.action_just_pressed("Quick Save") && crate::save::can_save(ph.dead) {
            let title = pstr(&assets.classes.defaults("DeusExPlayer"), "QuickSaveGameTitle");
            crate::save::request(crate::save::quick_path(), if title.is_empty() { "Quick Save" } else { &title });
        }
        if binds.action_just_pressed("Quick Load") && crate::save::quick_path().exists() {
            let d = assets.classes.defaults("DeusExRootWindow");
            let w = msg_box(hud, assets, &pstr(&d, "QuickLoadTitle"), &pstr(&d, "QuickLoadMessage"), true, MsgPurpose::QuickLoad);
            st.push(w);
        }
    }
}

/// Pause / unpause (UIPauseGame, except on the logo map), the pointer, and the backdrop behind the menus.
fn pause_and_backdrop(
    st: &mut Menus,
    vtime: &mut ResMut<Time<Virtual>>,
    cursor_opts: &mut Single<&mut CursorOptions>,
    backdrop: &mut Query<(&mut Visibility, &mut BackgroundColor), (With<MenuBackdrop>, Without<MaterialNode<HudLayer>>)>,
    is_open: bool,
    was_open: bool,
) {
    if is_open && !was_open {
        // (Never in multiplayer: the others play on.)
        if !intro_map() && !vtime.is_paused() && !crate::netplay::active() {
            vtime.pause();
            st.paused_game = true;
        }
        st.grabbed_before = cursor_opts.grab_mode != bevy::window::CursorGrabMode::None;
        cursor_opts.grab_mode = bevy::window::CursorGrabMode::None;
        cursor_opts.visible = false;
    }
    // A game session started while the menus were up (a room hosted from the game): the game runs again.
    if is_open && st.paused_game && crate::netplay::active() {
        vtime.unpause();
        st.paused_game = false;
    }
    if !is_open && (was_open || CLOSED_BY_MAP.swap(false, Ordering::Relaxed)) {
        if st.paused_game {
            vtime.unpause();
            st.paused_game = false;
        }
        // Back in the game: the mouse looks again.
        cursor_opts.visible = false;
        cursor_opts.grab_mode = bevy::window::CursorGrabMode::Locked;
    }
    OPEN.store(is_open, Ordering::Relaxed);
    if let Some(e) = st.backdrop {
        if let Ok((mut v, mut bg)) = backdrop.get_mut(e) {
            // MaskBackground (modulated MaskTexture) / UIBackground 2 = black.
            let black = crate::settings::ui_background() == 2 && !intro_map();
            let col = Color::srgba(0.0, 0.0, 0.0, if black { 1.0 } else { 0.5 });
            if bg.0 != col {
                bg.0 = col;
            }
            v.set_if_neq(if is_open { Visibility::Inherited } else { Visibility::Hidden });
        }
    }
}

/// No menu up: hide the window layers and the cursor.
fn hide_closed(st: &mut Menus, nodes: &mut Query<(&mut bevy::ui::Node, &mut Visibility, &MaterialNode<HudLayer>), Without<MenuBackdrop>>, wheel: &mut MessageReader<MouseWheel>) {
    for l in &st.win_layers {
        if let Ok((_, mut v, _)) = nodes.get_mut(l.e) {
            if *v != Visibility::Hidden {
                v.set_if_neq(Visibility::Hidden);
            }
        }
    }
    if let Some(c) = &st.cursor {
        if let Ok((_, mut v, _)) = nodes.get_mut(c.e) {
            v.set_if_neq(Visibility::Hidden);
        }
    }
    // (The kept canvases go: the next menu draws its windows afresh.)
    st.drawn.clear();
    if st.intro.is_none() {
        wheel.clear();
    }
}

/// The menu fonts; None (no menu this frame) until they all load.
fn menu_fonts(hud: &mut ClassicHud, assets: &mut RuntimeAssets) -> Option<Fonts> {
    let mut get = |n: &str| hud.font(assets, n);
    let (Some(title), Some(small), Some(headers), Some(fixed)) =
        (get("FontMenuTitle"), get("FontMenuSmall"), get("FontMenuHeaders"), get("FontFixedWidthSmall"))
    else {
        return None;
    };
    Some(Fonts { title, small, headers, fixed })
}

/// Hover moves the focus (MFOCUS_EnterLeave), except on the new-game screen (MFOCUS_Click).
fn hover_focus(cx: &mut InCx) {
    let top = cx.top;
    if !cx.click_focus {
        if let Some((w, true)) = cx.hover {
            if matches!(w, W::MenuBtn(_) | W::Act(_) | W::ChoiceBtn(_)) && cx.st.stack[top].focus != Some(w) {
                cx.st.stack[top].focus = Some(w);
                crate::sound::play_player("DeusExSounds.UserInterface.Menu_Focus", 0.25, 0.0);
            }
        }
    }
}

/// Waiting for a key on the keyboard screen: the next key or button is the binding (ProcessKeySelection); the
/// Controller Bindings screen waits for a pad button.
fn capture_binding(cx: &mut InCx, dt: f32) {
    let (st, binds, top) = (&mut *cx.st, &mut *cx.binds, cx.top);
    let Some(t) = st.waiting_key.as_mut() else { return };
    *t += dt;
    let ready = *t > 0.2;
    let mut got: Option<crate::keybinds::Input> = None;
    let mut cancel = false;
    // The Controller Bindings screen waits for a pad button instead (B and Start bind too; Escape cancels).
    let pad_screen = matches!(&st.stack[top].body, Body::Keys(k) if k.pad);
    if pad_screen {
        if ready {
            let pb = crate::gamepad::frame().just.first().copied();
            if binds.keys.just_pressed(KeyCode::Escape) && pb.is_none() {
                cancel = true;
            }
            if pb.is_some() || cancel {
                st.waiting_key = None;
                if let Body::Keys(k) = &mut st.stack[top].body {
                    k.waiting = false;
                    if let (Some(i), Some(b)) = (k.sel, pb) {
                        // The button leaves any other action it was on.
                        let mut note = String::new();
                        for (j, m) in k.pad_map.iter_mut().enumerate() {
                            if j != i && *m == Some(b) {
                                note = format!("'{}' reassigned from '{}'", crate::gamepad::pad_name(crate::gamepad::glyph_kind(), b), crate::gamepad::PAD_ACTIONS[j].0);
                                *m = None;
                            }
                        }
                        if let Some(m) = k.pad_map.get_mut(i) {
                            *m = Some(b);
                        }
                        k.note = note;
                    }
                }
            }
        }
    } else if ready {
        for k in binds.keys.get_just_pressed() {
            if *k == KeyCode::Escape {
                cancel = true;
            } else {
                got = Some(crate::keybinds::Input::Key(*k));
            }
        }
        for b in binds.buttons.get_just_pressed() {
            got = Some(crate::keybinds::Input::Mouse(*b));
        }
    }
    if cancel || got.is_some() {
        st.waiting_key = None;
        if let Body::Keys(k) = &mut st.stack[top].body {
            k.waiting = false;
            if let (Some(i), Some(inp)) = (k.sel, got) {
                // ProcessKeySelection: the key leaves any other function it was on.
                let mut note = String::new();
                for (j, m) in k.map.iter_mut().enumerate() {
                    if j != i && *m == inp {
                        note = format!("'{}' reassigned from '{}'", crate::keybinds::input_name(inp), crate::keybinds::ACTIONS[j].0);
                    }
                }
                if !note.is_empty() {
                    for (j, m) in k.map.iter_mut().enumerate() {
                        if j != i && *m == inp {
                            *m = crate::keybinds::ACTIONS[i].1;
                        }
                    }
                }
                if let Some(m) = k.map.get_mut(i) {
                    *m = inp;
                }
                k.note = note;
            }
        }
        crate::sound::play_player("DeusExSounds.UserInterface.Menu_Press", 0.25, 0.0);
    }
    binds.typed.clear();
}

/// An open drop-down takes the click: a row picks it, the list's scroll bar scrolls it, anywhere else closes it.
/// True when the click was the drop-down's.
fn drop_down_click(cx: &mut InCx) -> bool {
    let (st, acts, top, hover_w) = (&mut *cx.st, &mut cx.acts, cx.top, cx.hover_w);
    let mut drop_click = false;
    if cx.binds.buttons.just_pressed(MouseButton::Left) && st.stack[top].drop.is_some() {
        let in_list = matches!(hover_w, Some(W::Header(50) | W::ScrollUp(50) | W::ScrollDown(50) | W::ScrollTrack(50)));
        match hover_w {
            Some(W::Row(50, r)) => {
                let di = st.stack[top].drop.take().unwrap().0;
                if let Body::Choices { choices } = &mut st.stack[top].body {
                    if let Some(c) = choices.get_mut(di as usize) {
                        c.value = r as f32;
                        crate::sound::play_player("DeusExSounds.UserInterface.Menu_Press", 0.25, 0.0);
                        live_preview(c);
                        acts.extend(choice_side_effect(c));
                    }
                }
                drop_click = true;
            }
            _ if in_list => {}
            _ => {
                st.stack[top].drop = None;
                drop_click = true;
            }
        }
    }
    drop_click
}

/// A left click on the top window: title drag, edit focus, list rows (double click activates), scroll arrows.
/// Some(save / load key) when a save list row was double-clicked.
fn left_press(cx: &mut InCx) -> Option<&'static str> {
    let (st, acts, top, mouse) = (&mut *cx.st, &mut cx.acts, cx.top, cx.view.mouse);
    let mut sl_double: Option<&str> = None;
    st.pressed = None;
    if let Some((w, enabled)) = cx.hover {
        if enabled {
            let win = &mut st.stack[top];
            match w {
                W::Title => {
                    let p = win_pos(win, cx.fonts, cx.view.sw, cx.view.sh);
                    st.drag = Some((mouse.0 - p.0, mouse.1 - p.1));
                }
                W::Edit(_) => {
                    win.focus = Some(w);
                }
                W::Row(l, r) => {
                    let double = st.last_click.1 == Some(w) && st.clock - st.last_click.0 < 0.4;
                    st.last_click = (st.clock, Some(w));
                    crate::sound::play_player("DeusExSounds.UserInterface.Menu_Press", 0.25, 0.0);
                    match &mut win.body {
                        Body::NewGame(ng) if l == 0 => {
                            if ng.sel != Some(r) {
                                ng.desc_scroll = 0;
                            }
                            ng.sel = Some(r);
                            if double {
                                // ListRowActivated: upgrade.
                                if let Some(&i) = ng.order.get(r) {
                                    let c = ng.skills.list[i].def.class.clone();
                                    ng.skills.upgrade(&c);
                                }
                            }
                        }
                        Body::Keys(k) => {
                            k.sel = Some(r);
                            if double {
                                k.waiting = true;
                                st.waiting_key = Some(0.0);
                            }
                        }
                        Body::Enhance(en) if l == 0 => {
                            if let Some(&i) = en.shown().get(r) {
                                // A click on the selected row changes it; on another row it selects it.
                                // (A group header opens or closes at once.)
                                let header = matches!(i, Row::Group(_));
                                if en.sel == Some(i) || header {
                                    en.sel = Some(i);
                                    en.step(1, acts);
                                }
                                en.sel = Some(i);
                            }
                        }
                        Body::Browse(b) if l == 0 => {
                            b.sel = Some(r);
                            if double {
                                acts.extend(mp_browse_join(b));
                            }
                        }
                        Body::DmChar(d) if l == 0 => {
                            if d.sel == Some(r) {
                                if let Some(m) = d.step(1) {
                                    acts.push(Act::Msg("Deathmatch Character".into(), m, false, MsgPurpose::Info));
                                }
                            }
                            d.sel = Some(r);
                        }
                        Body::Chars(c) if l == 0 => {
                            c.sel = Some(r);
                            if double {
                                acts.extend(mp_chars_go(c));
                            }
                        }
                        Body::SaveLoad(sl) if l == 0 => {
                            if sl.sel != Some(r) {
                                sl.sel = Some(r);
                                if sl.saving {
                                    // Name the save: the old description, or where the player is for a new one.
                                    let t = sl.game(r).map(|g| g.description.clone()).unwrap_or_else(default_save_name);
                                    sl.edit.text = t.chars().collect();
                                    sl.edit.cur = sl.edit.text.len();
                                    sl.edit.anchor = 0;
                                }
                            }
                            if !sl.saving && sl.toggle(r) {
                                // A character's heading opens / closes (one click, like the Enhancements groups).
                            } else if double {
                                sl_double = Some(if sl.saving { "SAVE" } else { "LOAD" });
                            }
                        }
                        _ => {}
                    }
                }
                W::ScrollUp(id) | W::ScrollDown(id) => {
                    let d = if matches!(w, W::ScrollUp(_)) { -ROW_H } else { ROW_H };
                    scroll_by(win, id, d);
                }
                W::Win => {}
                _ => {
                    if cx.click_focus {
                        win.focus = Some(w);
                    }
                }
            }
            st.pressed = Some(w);
        }
    }
    sl_double
}

/// The left button held: drag the window, move a pressed slider, drag a scroll track.
fn left_held(cx: &mut InCx) {
    let (st, top, mouse) = (&mut *cx.st, cx.top, cx.view.mouse);
    if let Some((dx, dy)) = st.drag {
        st.stack[top].pos = Some((mouse.0 - dx, mouse.1 - dy));
    }
    if let Some(W::Slider(i)) = st.pressed {
        if let Some((_, r, _)) = st.hits.iter().find(|h| h.0 == W::Slider(i)) {
            let r = *r;
            if let Body::Choices { choices } = &mut st.stack[top].body {
                if let Some(c) = choices.get_mut(i as usize) {
                    if let ChoiceKind::Slider { ticks, .. } = c.kind {
                        let rel = (mouse.0 - r.0 - 8) as f32 / (r.2 - 16).max(1) as f32;
                        let t = (rel * (ticks - 1) as f32).round() as i32;
                        if t != c.slider_tick() {
                            c.set_tick(t);
                            crate::sound::play_player("DeusExSounds.UserInterface.Menu_Slider", 0.25, 0.0);
                            live_preview(c);
                        }
                    }
                }
            }
        }
    }
    if let Some(W::ScrollTrack(id)) = st.pressed {
        if id < 100 {
            if let Some((_, r, _)) = st.hits.iter().find(|h| h.0 == W::ScrollTrack(id)) {
                let frac = ((mouse.1 - r.1) as f32 / r.3.max(1) as f32).clamp(0.0, 1.0);
                set_scroll(&mut st.stack[top], id, (frac * 4000.0) as i32);
            }
        }
    }
}

/// Left release over the pressed widget activates it; right click steps a choice back; the wheel scrolls lists.
fn release_and_wheel(cx: &mut InCx, wheel: &mut MessageReader<MouseWheel>) {
    let (st, acts, top, hover_w) = (&mut *cx.st, &mut cx.acts, cx.top, cx.hover_w);
    if cx.binds.buttons.just_released(MouseButton::Left) {
        st.drag = None;
        if let Some(w) = st.pressed.take() {
            if hover_w == Some(w) {
                activate(st, top, w, false, &mut *cx.hud, &mut *cx.assets, acts);
            }
        }
    }
    if cx.binds.buttons.just_pressed(MouseButton::Right) {
        if let Some((w @ (W::ChoiceBtn(_) | W::ChoiceVal(_)), true)) = cx.hover {
            activate(st, top, w, true, &mut *cx.hud, &mut *cx.assets, acts);
        }
    }
    let lines: f32 = wheel.read().map(|w| if w.unit == bevy::input::mouse::MouseScrollUnit::Line { w.y } else { w.y / 40.0 }).sum();
    if lines != 0.0 {
        let px = (-lines * 3.0 * ROW_H as f32) as i32;
        let id = match hover_w {
            Some(W::Row(l, _)) => Some(l),
            Some(W::ScrollTrack(i)) => Some(i % 100),
            _ => None,
        };
        if let Some(id) = id {
            scroll_by(&mut st.stack[top], id, px);
        }
    }
}

/// The keys typed this frame, until one closes the window.
fn typed_keys(cx: &mut InCx) {
    let typed = cx.binds.typed.clone();
    for (k, text) in &typed {
        if cx.acts.iter().any(|a| matches!(a, Act::Pop | Act::PopAll)) {
            break;
        }
        if screen_typing(cx, k, text) {
            continue;
        }
        menu_key(cx, k);
    }
}

/// Screens that take a key themselves: keypad digits, the Enhancements list and search, the save description, the
/// new-game name. True when the key was theirs.
fn screen_typing(cx: &mut InCx, k: &KeyCode, text: &Option<String>) -> bool {
    let (shift, ctrl) = (cx.shift, cx.ctrl);
    let acts = &mut cx.acts;
    let win = &mut cx.st.stack[cx.top];
    // HUDKeypadWindow.VirtualKeyPressed: the number keys press the buttons.
    if let Body::Keypad(kp) = &mut win.body {
        let idx = match k {
            KeyCode::Digit0 | KeyCode::Numpad0 => Some(10),
            KeyCode::Digit1 | KeyCode::Numpad1 => Some(0),
            KeyCode::Digit2 | KeyCode::Numpad2 => Some(1),
            KeyCode::Digit3 | KeyCode::Numpad3 => Some(2),
            KeyCode::Digit4 | KeyCode::Numpad4 => Some(3),
            KeyCode::Digit5 | KeyCode::Numpad5 => Some(4),
            KeyCode::Digit6 | KeyCode::Numpad6 => Some(5),
            KeyCode::Digit7 | KeyCode::Numpad7 => Some(6),
            KeyCode::Digit8 | KeyCode::Numpad8 => Some(7),
            KeyCode::Digit9 | KeyCode::Numpad9 => Some(8),
            _ => None,
        };
        if let Some(i) = idx {
            keypad_press(kp, i);
            return true;
        }
        // Ours: Tab types the code found in the notes.
        if *k == KeyCode::Tab && kp.noted && kp.wait.is_none() {
            keypad_from_notes(kp);
            return true;
        }
    }
    // Enhancements: arrows move / change, Enter changes or opens, everything else types into the search.
    if let Body::Enhance(en) = &mut win.body {
        match k {
            KeyCode::ArrowUp => en.move_sel(-1),
            KeyCode::ArrowDown => en.move_sel(1),
            KeyCode::PageUp => en.move_sel(-20),
            KeyCode::PageDown => en.move_sel(20),
            KeyCode::ArrowLeft => en.step(-1, acts),
            KeyCode::ArrowRight | KeyCode::Enter | KeyCode::NumpadEnter if en.sel.is_some() => en.step(1, acts),
            KeyCode::Escape | KeyCode::Tab | KeyCode::Enter | KeyCode::NumpadEnter => {}
            _ => {
                en.search.key(*k, text.as_deref(), shift, ctrl);
                en.scroll = 0;
                // Keep a selection among what's left.
                let shown = en.shown();
                if !en.sel.is_some_and(|s| shown.contains(&s)) {
                    en.sel = shown.first().copied();
                }
            }
        }
        if !matches!(k, KeyCode::Escape | KeyCode::Tab | KeyCode::Enter | KeyCode::NumpadEnter) || en.sel.is_some() && matches!(k, KeyCode::Enter | KeyCode::NumpadEnter) {
            return true;
        }
    }
    // The deathmatch character creator: arrows work the list, everything else types the name.
    if let Body::DmChar(d) = &mut win.body {
        let msg = match k {
            KeyCode::ArrowUp => {
                d.move_sel(-1);
                None
            }
            KeyCode::ArrowDown => {
                d.move_sel(1);
                None
            }
            KeyCode::ArrowLeft => d.step(-1),
            KeyCode::ArrowRight | KeyCode::Enter | KeyCode::NumpadEnter => d.step(1),
            KeyCode::Escape | KeyCode::Tab => return false,
            _ => {
                d.name.key(*k, text.as_deref(), shift, ctrl);
                None
            }
        };
        if let Some(m) = msg {
            acts.push(Act::Msg("Deathmatch Character".into(), m, false, MsgPurpose::Info));
        }
        return true;
    }
    // The join screen's room code.
    if let Body::Join(j) = &mut win.body {
        return mp_join_typing(j, k, text, ctrl);
    }
    // The server address (host names, IPv4 / IPv6, a port).
    if let Body::Addr(a) = &mut win.body {
        if matches!(k, KeyCode::Escape | KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Tab | KeyCode::ArrowUp | KeyCode::ArrowDown) {
            return false;
        }
        let t = mp_addr_typed(text.as_deref());
        a.edit.key(*k, t.as_deref(), shift, ctrl);
        return true;
    }
    // The save screen's description takes typing (Up / Down / Enter / Escape still work the list).
    if let Body::SaveLoad(sl) = &mut win.body {
        if sl.saving
            && sl.sel.is_some()
            && !matches!(
                k,
                KeyCode::Escape | KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Tab | KeyCode::ArrowUp | KeyCode::ArrowDown
            )
        {
            sl.edit.key(*k, text.as_deref(), shift, ctrl);
            return true;
        }
    }
    // An edit with the focus takes typing (MenuUIEditWindow; the new-game name keeps to its filter).
    if let (Some(W::Edit(0)), Body::NewGame(ng)) = (win.focus, &mut win.body) {
        let filter = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz1234567890 ";
        let t = text.as_deref().map(|t| t.chars().filter(|c| filter.contains(*c)).collect::<String>());
        if !matches!(k, KeyCode::Escape | KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Tab) {
            ng.name.key(*k, t.as_deref().filter(|t| !t.is_empty()), shift, ctrl);
            return true;
        }
    }
    false
}

/// The window keys: Escape (cancel / OK / No), Enter (OK / activate), Y / N on yes-no boxes, arrows, Delete.
fn menu_key(cx: &mut InCx, k: &KeyCode) {
    let (st, top, hud, assets) = (&mut *cx.st, cx.top, &mut *cx.hud, &mut *cx.assets);
    let acts = &mut cx.acts;
    let win = &mut st.stack[top];
    match k {
        KeyCode::Escape if win.drop.is_some() => {
            win.drop = None;
        }
        KeyCode::Escape => {
            if let Body::Msg(m) = &win.body {
                // MB_YesNo: No; MB_OK: OK.
                let yes = !m.yes_no;
                box_answer(st, top, yes, hud, assets, acts);
            } else if win.f.escape_saves {
                save_choices(win);
                crate::sound::play_player("DeusExSounds.UserInterface.Menu_OK", 0.25, 0.0);
                acts.push(Act::Pop);
            } else {
                cancel_choices(win);
                crate::sound::play_player("DeusExSounds.UserInterface.Menu_Cancel", 0.25, 0.0);
                acts.push(Act::Pop);
            }
        }
        KeyCode::Enter | KeyCode::NumpadEnter => {
            match &win.body {
                Body::Msg(_) => box_answer(st, top, true, hud, assets, acts),
                Body::Menu { .. } => {
                    if let Some(f) = win.focus {
                        activate(st, top, f, false, hud, assets, acts);
                    }
                }
                Body::Keys(kk) if kk.sel.is_some() => {
                    if let Body::Keys(kk) = &mut win.body {
                        kk.waiting = true;
                    }
                    st.waiting_key = Some(0.0);
                }
                Body::NewGame(_) | Body::Keypad(_) => {}
                Body::Host(_) | Body::Join(_) | Body::Browse(_) | Body::Chars(_) | Body::Addr(_) | Body::Wait(_) => mp_enter(win, acts),
                Body::SaveLoad(sl) => {
                    let saving = sl.saving;
                    let heading = !saving
                        && match &mut st.stack[top].body {
                            Body::SaveLoad(sl) => sl.sel.is_some_and(|r| sl.toggle(r)),
                            _ => false,
                        };
                    if !heading {
                        let k = if saving { "SAVE" } else { "LOAD" };
                        save_load_action(st, top, k, hud, assets, acts);
                    }
                }
                _ => {
                    // Enter is OK.
                    save_choices(win);
                    crate::sound::play_player("DeusExSounds.UserInterface.Menu_OK", 0.25, 0.0);
                    acts.push(Act::Pop);
                }
            }
        }
        KeyCode::KeyY => {
            if matches!(&win.body, Body::Msg(m) if m.yes_no) {
                box_answer(st, top, true, hud, assets, acts);
            }
        }
        KeyCode::KeyN => {
            if matches!(&win.body, Body::Msg(m) if m.yes_no) {
                box_answer(st, top, false, hud, assets, acts);
            }
        }
        KeyCode::ArrowUp | KeyCode::ArrowDown => arrow_key(win, *k == KeyCode::ArrowUp),
        KeyCode::ArrowLeft | KeyCode::ArrowRight => {
            // MenuUIScreenWindow: left / right cycle the focused choice.
            if let (Some(W::ChoiceBtn(i)), Body::Host(h)) = (win.focus, &mut win.body) {
                if let Some(e) = mp_host_cycle(h, i, if *k == KeyCode::ArrowLeft { -1 } else { 1 }) {
                    acts.push(Act::Msg("Host Room".into(), e, false, MsgPurpose::Info));
                }
            }
            if let (Some(W::ChoiceBtn(i)), Body::Choices { choices }) = (win.focus, &mut win.body) {
                if let Some(c) = choices.get_mut(i as usize) {
                    c.cycle(if *k == KeyCode::ArrowLeft { -1 } else { 1 });
                    crate::sound::play_player("DeusExSounds.UserInterface.Menu_Press", 0.25, 0.0);
                    live_preview(c);
                }
            }
        }
        KeyCode::Delete => {
            if let Body::Keys(kk) = &mut win.body {
                if let Some(i) = kk.sel {
                    // ClearFunction: our actions always keep one input, so Delete puts the default back.
                    if kk.pad {
                        if let Some(m) = kk.pad_map.get_mut(i) {
                            *m = crate::gamepad::PAD_ACTIONS[i].2;
                        }
                    } else if let Some(m) = kk.map.get_mut(i) {
                        *m = crate::keybinds::ACTIONS[i].1;
                    }
                }
            }
        }
        _ => {}
    }
}

/// Up / Down: the next enabled menu button, choice, skill, save or key row (lists scroll to keep it in view).
fn arrow_key(win: &mut Win, up: bool) {
    match &mut win.body {
        Body::Menu { buttons, .. } => {
            let n = buttons.len();
            let cur = match win.focus {
                Some(W::MenuBtn(i)) => i as i32,
                _ => if up { n as i32 } else { -1 },
            };
            let mut i = cur;
            for _ in 0..n {
                i = (i + if up { -1 } else { 1 }).rem_euclid(n as i32);
                if buttons[i as usize].enabled {
                    break;
                }
            }
            win.focus = Some(W::MenuBtn(i as u8));
            crate::sound::play_player("DeusExSounds.UserInterface.Menu_Focus", 0.25, 0.0);
        }
        Body::Host(MpHost { choices, .. }) | Body::Choices { choices } => {
            let n = choices.len() as i32;
            let cur = match win.focus {
                Some(W::ChoiceBtn(i)) => i as i32,
                _ => if up { n } else { -1 },
            };
            win.focus = Some(W::ChoiceBtn((cur + if up { -1 } else { 1 }).rem_euclid(n) as u8));
        }
        Body::NewGame(ng) => {
            let n = ng.order.len();
            let cur = ng.sel.unwrap_or(0);
            ng.sel = Some(if up { cur.saturating_sub(1) } else { (cur + 1).min(n.saturating_sub(1)) });
            ng.desc_scroll = 0;
        }
        Body::SaveLoad(sl) => {
            let n = sl.rows();
            if n > 0 {
                let cur = sl.sel.unwrap_or(0);
                let next = if up { cur.saturating_sub(1) } else { (cur + 1).min(n - 1) };
                if Some(next) != sl.sel {
                    sl.sel = Some(next);
                    if sl.saving {
                        let t = sl.game(next).map(|g| g.description.clone()).unwrap_or_else(default_save_name);
                        sl.edit.text = t.chars().collect();
                        sl.edit.cur = sl.edit.text.len();
                        sl.edit.anchor = 0;
                    }
                }
                let y = next as i32 * ROW_H;
                if y < sl.scroll {
                    sl.scroll = y;
                }
                if y + ROW_H > sl.scroll + 270 {
                    sl.scroll = y + ROW_H - 270;
                }
            }
        }
        Body::Browse(b) => mp_browse_move(b, if up { -1 } else { 1 }),
        Body::Chars(c) => mp_chars_move(c, if up { -1 } else { 1 }),
        Body::Keys(kk) => {
            let n = kk.rows();
            let cur = kk.sel.unwrap_or(0);
            let next = if up { cur.saturating_sub(1) } else { (cur + 1).min(n - 1) };
            kk.sel = Some(next);
            let y = next as i32 * ROW_H;
            if y < kk.scroll {
                kk.scroll = y;
            }
            if y + ROW_H > kk.scroll + 268 {
                kk.scroll = y + ROW_H - 268;
            }
        }
        _ => {}
    }
}

/// Apply what the input pass asked for: open / close windows, start games, quit, window mode and size.
#[allow(clippy::too_many_arguments)]
fn apply_acts(
    acts: Vec<Act>,
    st: &mut Menus,
    binds: &Bindings,
    window: &mut Single<&mut Window>,
    root: &crate::pickups::GameRoot,
    exit: &mut MessageWriter<AppExit>,
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
) {
    for a in acts {
        match a {
            Act::Push(c) => {
                let (class, arg) = c.split_once(':').map(|(a, b)| (a.to_string(), Some(b.to_string()))).unwrap_or((c.clone(), None));
                let mut w = open_window(hud, assets, &class);
                if let (Some(w), Some(arg)) = (w.as_mut(), arg) {
                    if arg == "mp" {
                        mp_variant(w);
                    } else if let Body::NewGame(ng) = &mut w.body {
                        ng.difficulty = arg.parse().unwrap_or(1.5);
                        w.focus = Some(W::Edit(0));
                    }
                }
                if let Some(w) = w.as_mut() {
                    if let Body::Keys(k) = &mut w.body {
                        k.map = binds.map.clone();
                    }
                    if let Body::Choices { choices } = &mut w.body {
                        // Screen Resolution shows the window's size.
                        let (ww, wh) = (window.physical_width(), window.physical_height());
                        for c in choices.iter_mut().filter(|c| c.key == "Resolution") {
                            let i = RESOLUTIONS
                                .iter()
                                .enumerate()
                                .min_by_key(|(_, (rw, rh))| (*rw as i64 - ww as i64).abs() + (*rh as i64 - wh as i64).abs())
                                .map(|x| x.0)
                                .unwrap_or(0);
                            c.value = i as f32;
                            c.saved = c.value;
                        }
                    }
                    if class == "MenuSelectDifficulty" {
                        w.focus = Some(W::MenuBtn(1));
                    }
                    mp_opened(&mut st.mp, w);
                }
                let tip = class == "MenuScreenNewGame" && crate::travel::lddp() && setting!("LDDPGaveNewGameTips") == 0.0;
                st.push(w);
                if tip {
                    // MenuScreenNewGame.GiveTip (once: bGaveNewGameTips).
                    crate::settings::set("LDDPGaveNewGameTips", 1.0);
                    crate::settings::save();
                    let w = msg_box(
                        hud,
                        assets,
                        "LDDP 1.1 Checkbox Options",
                        "'Original Morpheus' reverts to his old voiceover. 'Original interactions' reverts to the old dialogue for Mercedes, Camille, Mamasan, and CanalWhore.",
                        false,
                        MsgPurpose::Info,
                    );
                    st.push(w);
                }
            }
            Act::Pop => st.pop(),
            Act::PopAll => st.stack.clear(),
            Act::Msg(title, text, yes_no, purpose) => {
                let w = msg_box(hud, assets, &title, &text, yes_no, purpose);
                st.push(w);
            }
            Act::Quit => {
                crate::settings::save();
                exit.write(AppExit::Success);
            }
            Act::NewGame(ng) => {
                crate::settings::save();
                let start = assets.classes.defaults("JCDentonMale");
                let map = match find(&start, "strStartMap") {
                    Some(Value::Str(s)) if !s.is_empty() => format!("{s}.dx"),
                    _ => "01_NYC_UNATCOIsland.dx".into(),
                };
                // MenuScreenNewGame: ShowIntro(True) - the intro cut scene, then the new game on strStartMap.
                if root.0.join("Maps").join("00_Intro.dx").exists() {
                    crate::loader::request("00_Intro.dx", crate::loader::Goal::Intro(Some((ng, map))));
                } else {
                    crate::loader::request(&map, crate::loader::Goal::NewGame(ng));
                }
                st.stack.clear();
            }
            Act::Intro => {
                crate::loader::request("00_Intro.dx", crate::loader::Goal::Intro(None));
                st.stack.clear();
            }
            Act::Training => {
                crate::settings::save();
                crate::loader::request("00_Training.dx", crate::loader::Goal::NewGame(crate::travel::NewGame::default()));
                st.stack.clear();
            }
            Act::ToggleFullscreen => {
                window.mode = match window.mode {
                    WindowMode::Windowed => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
                    _ => WindowMode::Windowed,
                };
            }
            Act::SaveGame(path, desc) => {
                crate::save::request(path, &desc);
                st.stack.clear();
            }
            Act::LoadGame(path) => {
                crate::settings::save();
                crate::save::load(&root.0, &path);
            }
            Act::DisplayMode(v) => {
                window.mode = window_mode(v);
            }
            Act::VSync(v) => {
                window.present_mode = present_mode(v);
            }
            Act::Resolution(w, h) => {
                set_window_size(window, w, h);
            }
            Act::Connect => mp_connect(st, hud, assets),
            Act::Join(code) => mp_join(st, hud, assets, code),
            Act::MpLoadChar(id) => {
                if crate::netplay::load_character(&id).is_some() {
                    mp_join_room(st, hud, assets);
                } else {
                    let w = msg_box(hud, assets, "Choose Character", "That character couldn't be loaded.", false, MsgPurpose::MpResult);
                    st.push(w);
                }
            }
            Act::MpNewChar(ng) => {
                crate::netplay::choose_character(ng, None);
                mp_join_room(st, hud, assets);
            }
            Act::MpLeave => mp_leave(st),
            Act::MpSetAddr(a) => mp_set_addr(st, hud, assets, &a),
            Act::MpDirect(code) => mp_direct(st, hud, assets, code),
            Act::MpOffline => mp_offline(st, hud, assets),
        }
    }
}

/// The help line of the top window: the focused choice's help, the keys screen's state, the Enhancements row.
fn help_text(top_w: &Win, assets: &mut RuntimeAssets) -> String {
    match (&top_w.body, top_w.focus) {
        (Body::Choices { choices }, Some(W::ChoiceBtn(i))) => choices.get(i as usize).map(|c| c.help.clone()).unwrap_or_default(),
        (Body::Keys(k), _) => {
            let d = assets.classes.defaults("MenuScreenCustomizeKeys");
            if k.pad && k.waiting {
                "Press the controller button to assign. [Escape] cancels.".to_string()
            } else if k.pad && k.note.is_empty() {
                "Select an action and press [Enter], Double-Click or the A / Cross button, then press a controller \
                 button. [Delete] restores the default."
                    .to_string()
            } else if k.waiting {
                pstr(&d, "InputHelpText")
            } else if !k.note.is_empty() {
                k.note.clone()
            } else {
                pstr(&d, "WaitingHelpText")
            }
        }
        (Body::Enhance(en), _) => match en.sel {
            Some(Row::Item(i)) => en.items[i].help.clone(),
            Some(Row::Master) => "Every enhancement at once: Classic is the original game, Modern your own mix (all of them the first time). In play, Tab switches with a wipe.".into(),
            Some(Row::Group(g)) => format!("Show or hide the {} settings (Enter / click; Left closes). Type to search every setting.", en.groups().get(g).cloned().unwrap_or_default()),
            None => "Type to search the options.".into(),
        },
        (Body::Host(h), focus) => mp_host_help(h, focus, &top_w.f),
        (Body::DmChar(d), _) => d.help(),
        (Body::Addr(_), _) => "The online services' host name or IP address, with :port when it isn't 27962 (ask whoever runs \
             the server). Empty: this PC. Save & Connect or [Enter].".into(),
        (Body::Join(_), _) => "Type the host's room code: four characters from online services, or the long direct code (Ctrl+V pastes it). Then Join Room or [Enter].".into(),
        (Body::Wait(w), _) if !w.connected => "Connecting to the host...".into(),
        (Body::Wait(w), _) if w.ready => "Ready. The game starts when the host starts it.".into(),
        (Body::Wait(_), _) => "Press Ready when you are; the host starts the game once everyone is.".into(),
        (Body::Choices { .. }, _) if top_w.f.class == "MenuScreenBrightness" => {
            pstr(&assets.classes.defaults("MenuScreenBrightness"), "HelpMessage")
        }
        _ => String::new(),
    }
}

/// Draw the top window and the ones it doesn't hide into the window layers (Lean Updates keeps unchanged ones),
/// keep the top window's hit rectangles, and offer them to the pad's snap-to.
#[allow(clippy::too_many_arguments)]
fn draw_windows(
    st: &mut Menus,
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    images: &mut Assets<Image>,
    layers: &mut Assets<HudLayer>,
    nodes: &mut Query<(&mut bevy::ui::Node, &mut Visibility, &MaterialNode<HudLayer>), Without<MenuBackdrop>>,
    fonts: &Fonts,
    view: View,
    hover_w: Option<W>,
    waiting: bool,
    lean: bool,
) {
    let View { s, d, sf, sw, sh, .. } = view;
    let mut first = st.stack.len() - 1;
    while first > 0 && !st.stack[first].hides_below {
        first -= 1;
    }
    let help = help_text(&st.stack[st.stack.len() - 1], assets);
    let mut hits: Vec<(W, Rect, bool)> = Vec::new();
    let n_layers = st.win_layers.len();
    let top_i = st.stack.len() - 1;
    let pressed = st.pressed;
    // Lean Updates (perf.rs): a window is drawn and uploaded again only when something its drawing reads has
    // changed - the window (win_key) or these.
    let frame_key = lean.then(|| {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (s, crate::settings::generation(), palette(), waiting).hash(&mut h);
        (std::sync::Arc::as_ptr(&fonts.title), std::sync::Arc::as_ptr(&fonts.small), std::sync::Arc::as_ptr(&fonts.headers)).hash(&mut h);
        h
    });
    if lean {
        st.drawn.resize_with(n_layers.max(st.drawn.len()), Drawn::default);
    } else {
        st.drawn.clear();
        st.cursor_drawn = None;
    }
    for (li, wi) in (first..st.stack.len()).enumerate() {
        if li >= n_layers {
            break;
        }
        let pos = win_pos(&st.stack[wi], fonts, sw, sh);
        let (ww, wh) = win_size(&st.stack[wi].f, fonts);
        let is_top = wi == top_i;
        let (w_pressed, w_hover) = if is_top { (pressed, hover_w) } else { (None, None) };
        let w_help = if is_top { help.as_str() } else { "" };
        let key = |win: &Win| {
            frame_key.clone().map(|mut h| {
                (pos, w_help).hash(&mut h);
                // The pointer only matters over the pressed widget (Ui::state, draw_keypad).
                hash_debug(&mut h, &(w_pressed, w_pressed.is_some() && w_pressed == w_hover));
                win_key(win, &mut h);
                h.finish()
            })
        };
        let before = key(&st.stack[wi]);
        if let Some(dr) = st.drawn.get(li).filter(|dr| before.is_some() && dr.key == before) {
            // Nothing it reads has changed: the layer keeps its image and the hit rectangles that drawing made.
            let l = &st.win_layers[li];
            crate::hud::set_filter(images, &l.img, s.max(1), d);
            show(l, pos, (ww, wh), d, sf, nodes);
            if is_top {
                hits = dr.hits.clone();
            }
            continue;
        }
        let mut layer_hits: Vec<(W, Rect, bool)> = Vec::new();
        let focus = st.stack[wi].focus;
        let canvas = match st.drawn.get_mut(li).and_then(|dr| dr.canvas.take()) {
            Some(mut c) => {
                c.reset(ww, wh, s);
                c
            }
            None => Canvas::new(ww, wh, s),
        };
        let mut ui = Ui {
            c: canvas,
            hud: &mut *hud,
            assets: &mut *assets,
            f: fonts,
            o: pos,
            hits: &mut layer_hits,
            pressed: w_pressed,
            hover: w_hover,
            focus,
        };
        draw_win(&mut ui, &mut st.stack[wi], fonts, w_help, waiting);
        let Ui { c, .. } = ui;
        place(&mut st.win_layers[li], &c, pos, d, sf, images, layers, nodes);
        if let Some(dr) = st.drawn.get_mut(li) {
            // A drawing that changed the window (a scroll clamped, a snapshot loaded) isn't kept: the next frame
            // draws again from what it left.
            dr.key = before.filter(|&k| key(&st.stack[wi]) == Some(k));
            dr.hits.clone_from(&layer_hits);
            dr.canvas = Some(c);
        }
        if is_top {
            hits = layer_hits;
        }
    }
    for li in (st.stack.len() - first)..n_layers {
        if let Ok((_, mut v, _)) = nodes.get_mut(st.win_layers[li].e) {
            v.set_if_neq(Visibility::Hidden);
        }
    }
    // A message box keeps the clicks to itself.
    st.hits = hits;
    // Controller snap-to targets: the widgets you click, not the frame, title bar or scroll tracks.
    crate::snap::offer(
        crate::snap::LAYER_MENU,
        crate::snap::Map { s: d, ox: 0, oy: 0, sf },
        st.hits.iter().filter(|h| h.2 && !matches!(h.0, W::Win | W::Title | W::ScrollTrack(_))).map(|h| h.1),
    );
}

/// Cursor: the same DeusExCursor2 as the in-game screens (ours: the original menus used Cursor1).
#[allow(clippy::too_many_arguments)]
fn draw_cursor(
    st: &mut Menus,
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    images: &mut Assets<Image>,
    layers: &mut Assets<HudLayer>,
    nodes: &mut Query<(&mut bevy::ui::Node, &mut Visibility, &MaterialNode<HudLayer>), Without<MenuBackdrop>>,
    window: &Window,
    view: View,
    lean: bool,
) {
    let View { s, d, sf, mouse, .. } = view;
    if let Some(cl) = st.cursor.as_mut() {
        let at = (mouse.0 - 32, mouse.1 - 32);
        let shown = window.cursor_position().is_some();
        // Lean Updates: its image (the blurred shadow too) is drawn once per scale and theme, then only moved; a
        // hidden cursor is left alone.
        let cursor_key = (s, m_col(12));
        if lean && !shown {
            if let Ok((_, mut v, _)) = nodes.get_mut(cl.e) {
                if *v != Visibility::Hidden {
                    *v = Visibility::Hidden;
                }
            }
        } else if lean && st.cursor_drawn == Some(cursor_key) {
            crate::hud::set_filter(images, &cl.img, s.max(1), d);
            show(cl, at, (64, 64), d, sf, nodes);
        } else {
            let mut cc = Canvas::new(64, 64, s);
            if let Some(tx) = hud.ui(assets, "DeusExCursor2") {
                cc.drop_shadow(0, 0, &tx, (2.0, 2.5), 2.5, 0.6);
                cc.texture(0, 0, 64, 64, 0, 0, &tx, Style::Masked, m_col(12));
            }
            place(cl, &cc, at, d, sf, images, layers, nodes);
            if lean {
                st.cursor_drawn = Some(cursor_key);
            }
            if !shown {
                if let Ok((_, mut v, _)) = nodes.get_mut(cl.e) {
                    *v = Visibility::Hidden;
                }
            }
        }
    }
}

fn scroll_by(win: &mut Win, id: u8, d: i32) {
    if id == 50 {
        if let Some((_, sc)) = win.drop.as_mut() {
            *sc = (*sc + d).max(0);
        }
        return;
    }
    match &mut win.body {
        Body::Keys(k) if id == 0 => k.scroll = (k.scroll + d).max(0),
        // The Enhancements list (list() clamps it to the end when it draws).
        Body::Enhance(en) if id == 0 => en.scroll = (en.scroll + d).max(0),
        Body::SaveLoad(sl) if id == 0 => sl.scroll = (sl.scroll + d).max(0),
        Body::Achieve(av) if id == 0 => av.scroll = (av.scroll + d).max(0),
        Body::Browse(b) if id == 0 => b.scroll = (b.scroll + d).max(0),
        Body::Chars(c) if id == 0 => c.scroll = (c.scroll + d).max(0),
        Body::Wait(w) if id == 0 => w.scroll = (w.scroll + d).max(0),
        Body::Welcome(sc) if id == 0 => *sc = (*sc + d).max(0),
        Body::NewGame(ng) if id == 1 => ng.desc_scroll = (ng.desc_scroll + d).max(0),
        _ => {}
    }
}

fn set_scroll(win: &mut Win, id: u8, v: i32) {
    if id == 50 {
        if let Some((_, sc)) = win.drop.as_mut() {
            *sc = v;
        }
        return;
    }
    match &mut win.body {
        Body::Keys(k) if id == 0 => k.scroll = v,
        Body::Enhance(en) if id == 0 => en.scroll = v,
        Body::SaveLoad(sl) if id == 0 => sl.scroll = v,
        Body::Achieve(av) if id == 0 => av.scroll = v,
        Body::Browse(b) if id == 0 => b.scroll = v,
        Body::Chars(c) if id == 0 => c.scroll = v,
        Body::Wait(w) if id == 0 => w.scroll = v,
        Body::Welcome(sc) if id == 0 => *sc = v,
        Body::NewGame(ng) if id == 1 => ng.desc_scroll = v,
        _ => {}
    }
}

/// A message box answer (BoxOptionSelected in the window that opened it).
fn box_answer(st: &mut Menus, top: usize, yes: bool, _hud: &mut ClassicHud, _assets: &mut RuntimeAssets, acts: &mut Vec<Act>) {
    let Body::Msg(m) = &st.stack[top].body else { return };
    let purpose = m.purpose;
    crate::sound::play_player("DeusExSounds.UserInterface.Menu_Press", 0.25, 0.0);
    acts.push(Act::Pop);
    match purpose {
        MsgPurpose::External => {
            if let Ok(mut a) = EXTERNAL_ANSWER.lock() {
                *a = Some(yes);
            }
        }
        MsgPurpose::Exit => {
            if yes {
                acts.push(Act::Quit);
            }
        }
        MsgPurpose::AskToTrain => {
            if yes {
                acts.push(Act::Training);
            } else {
                acts.push(Act::Push("MenuSelectDifficulty".into()));
            }
        }
        MsgPurpose::Overwrite => {
            if yes {
                if let Some((p, d)) = st.box_save.take() {
                    acts.push(Act::SaveGame(p, d));
                }
            }
        }
        MsgPurpose::AchReset => {
            if yes {
                crate::achievements::reset_all();
                // The screen under the box shows the empty list.
                if let Some(w) = st.stack.iter_mut().rev().find(|w| matches!(w.body, Body::Achieve(_))) {
                    let (chars, global) = crate::achievements::all_characters();
                    w.body = Body::Achieve(AchieveView { scroll: 0, chars, global });
                }
            }
        }
        MsgPurpose::Delete => {
            if yes {
                if let Some((p, _)) = st.box_save.take() {
                    crate::save::delete(&p);
                    refresh_games(st);
                }
            }
        }
        MsgPurpose::QuickLoad => {
            if yes {
                acts.push(Act::LoadGame(crate::save::quick_path()));
            }
        }
        MsgPurpose::Training => {
            if yes {
                acts.push(Act::Training);
            }
        }
        MsgPurpose::Intro => {
            if yes {
                acts.push(Act::Intro);
            }
        }
        MsgPurpose::MpLeave => {
            if yes {
                acts.push(Act::MpLeave);
            }
        }
        MsgPurpose::Connecting => {
            crate::online::disconnect();
            st.mp.online = false;
        }
        MsgPurpose::Info | MsgPurpose::NameBlank | MsgPurpose::Restart | MsgPurpose::PadMap | MsgPurpose::Joining | MsgPurpose::MpResult => {}
    }
}

/// The save / load screen under the message box lists the folder again (after a delete).
fn refresh_games(st: &mut Menus) {
    for w in st.stack.iter_mut() {
        if let Body::SaveLoad(sl) = &mut w.body {
            sl.reload();
            let n = sl.rows();
            sl.sel = sl.sel.map(|r| r.min(n.saturating_sub(1))).filter(|_| n > 0);
        }
    }
}

/// MenuScreenSaveGame / LoadGame buttons.
fn save_load_action(st: &mut Menus, top: usize, key: &str, hud: &mut ClassicHud, assets: &mut RuntimeAssets, acts: &mut Vec<Act>) {
    let d = assets.classes.defaults("MenuScreenLoadGame");
    let Body::SaveLoad(sl) = &st.stack[top].body else { return };
    let row = sl.sel;
    let game = row.and_then(|r| sl.game(r)).cloned();
    match key {
        "LOAD" => {
            if let Some(g) = game {
                // Hosting: not a character someone in the room already plays as.
                match crate::netplay::name_taken(&g.char_name).filter(|_| crate::netplay::is_host() && !g.char_name.trim().is_empty()) {
                    Some(who) => acts.push(Act::Msg("Name Taken".into(), name_taken_text(&who), false, MsgPurpose::MpResult)),
                    None => acts.push(Act::LoadGame(g.path)),
                }
            }
        }
        "SAVE" => {
            let desc = sl.edit.get().trim().to_string();
            if desc.is_empty() || row.is_none() {
                return;
            }
            match game {
                Some(g) => {
                    st.box_save = Some((g.path, desc));
                    acts.push(Act::Msg(pstr(&d, "OverwriteTitle"), pstr(&d, "OverwritePrompt"), true, MsgPurpose::Overwrite));
                }
                None => acts.push(Act::SaveGame(crate::save::new_slot(), desc)),
            }
        }
        "DELETE" => {
            if let Some(g) = game {
                if sl.confirm_delete {
                    st.box_save = Some((g.path, String::new()));
                    acts.push(Act::Msg(pstr(&d, "DeleteTitle"), pstr(&d, "DeletePrompt"), true, MsgPurpose::Delete));
                } else {
                    crate::save::delete(&g.path);
                    refresh_games(st);
                }
            }
        }
        _ => {}
    }
    let _ = hud;
}

#[allow(clippy::too_many_arguments)]
fn activate(st: &mut Menus, top: usize, w: W, right: bool, hud: &mut ClassicHud, assets: &mut RuntimeAssets, acts: &mut Vec<Act>) {
    let win = &mut st.stack[top];
    match (w, &mut win.body) {
        (W::MenuBtn(i), Body::Menu { buttons, .. }) => {
            crate::sound::play_player("DeusExSounds.UserInterface.Menu_Press", 0.25, 0.0);
            if let Some(b) = buttons.get(i as usize).cloned() {
                if b.enabled {
                    if win.f.class == "MenuMain" && i == 7 {
                        // Back to Game: close every menu.
                        acts.push(Act::PopAll);
                    } else {
                        menu_action(&b, hud, assets, acts);
                    }
                }
            }
        }
        (W::Act(i), _) => {
            crate::sound::play_player("DeusExSounds.UserInterface.Menu_Press", 0.25, 0.0);
            let Some((_, action, _, key)) = win.f.actions.get(i as usize).cloned() else { return };
            match action {
                1 | 4 => {
                    save_choices(win);
                    if win.f.class == "MenuScreenMatchSettings" {
                        crate::deathmatch::room_go();
                    }
                    crate::sound::play_player("DeusExSounds.UserInterface.Menu_OK", 0.25, 0.0);
                    acts.push(Act::Pop);
                }
                2 => {
                    cancel_choices(win);
                    crate::sound::play_player("DeusExSounds.UserInterface.Menu_Cancel", 0.25, 0.0);
                    acts.push(Act::Pop);
                }
                3 => reset_defaults(win),
                5 if matches!(win.body, Body::SaveLoad(_)) => save_load_action(st, top, &key, hud, assets, acts),
                5 => {
                    // MenuScreenNewGame START.
                    if let Body::NewGame(ng) = &win.body {
                        let name = ng.name.get().trim().to_string();
                        let taken = crate::netplay::name_taken(&name);
                        if name.is_empty() {
                            let d = assets.classes.defaults("MenuScreenNewGame");
                            acts.push(Act::Msg(pstr(&d, "NameBlankTitle"), pstr(&d, "NameBlankPrompt"), false, MsgPurpose::NameBlank));
                        } else if let Some(who) = taken.filter(|_| key == "START") {
                            acts.push(Act::Msg("Name Taken".into(), name_taken_text(&who), false, MsgPurpose::MpResult));
                        } else if key == "START" {
                            let lddp = crate::travel::lddp();
                            let female = lddp && ng.portrait % 2 == 1;
                            let name_for_id = name.clone();
                            let new_game = crate::travel::NewGame {
                                name,
                                skin: if lddp { (ng.portrait / 2) as u8 } else { ng.portrait as u8 },
                                female,
                                retro_morpheus: lddp && ng.retro_morpheus,
                                male_interactions: female && ng.male_interactions,
                                difficulty: ng.difficulty,
                                skills: ng.skills.list.iter().map(|s| (s.def.class.clone(), s.level)).collect(),
                                points_avail: ng.skills.points_avail,
                                points_total: ng.skills.points_total,
                                // Ours: a new character gets its own folder of saves, autosaves and achievements.
                                char_id: crate::save::new_char_id(&name_for_id),
                            };
                            // (Every character has its own five autosave slots: a new game overwrites none.)
                            if ng.multiplayer {
                                acts.push(Act::MpNewChar(new_game));
                            } else {
                                acts.push(Act::NewGame(new_game));
                            }
                        }
                    }
                }
                10..=12 => box_answer(st, top, action != 10, hud, assets, acts),
                ACT_START..=ACT_LAST => {
                    if mp_action(win, action, acts) {
                        st.mp.hosting = false;
                    }
                }
                // The Achievements screen's Reset All: asked first.
                22 => {
                    crate::sound::play_player("DeusExSounds.UserInterface.Menu_Press", 0.25, 0.0);
                    acts.push(Act::Msg(
                        "Reset Achievements".into(),
                        "Reset every achievement for every character? This can't be undone.".into(),
                        true,
                        MsgPurpose::AchReset,
                    ));
                }
                // The welcome window: OK, or Don't Show Again (welcome.rs).
                20 | 21 => {
                    if action == 21 {
                        crate::settings::set("WelcomeSeen", crate::welcome::VERSION);
                        crate::settings::save();
                    }
                    crate::sound::play_player("DeusExSounds.UserInterface.Menu_OK", 0.25, 0.0);
                    acts.push(Act::Pop);
                }
                _ => {}
            }
        }
        (W::ChoiceBtn(i) | W::ChoiceVal(i), Body::Choices { choices }) => {
            crate::sound::play_player("DeusExSounds.UserInterface.Menu_Press", 0.25, 0.0);
            let Some(c) = choices.get_mut(i as usize) else { return };
            match c.kind.clone() {
                ChoiceKind::Action { action, invoke } => match (c.key.as_str(), action) {
                    ("FullScreen", _) => acts.push(Act::ToggleFullscreen),
                    ("RenderDevice", _) => acts.push(Act::Msg(
                        "Rendering Device".into(),
                        "This version always renders with wgpu (Vulkan, DirectX 12 or Metal).".into(),
                        false,
                        MsgPurpose::Restart,
                    )),
                    (_, 0 | 1) if !invoke.is_empty() => acts.push(Act::Push(invoke)),
                    _ => {}
                },
                _ if c.dropdown => {
                    // Open the list, scrolled to the current value.
                    let n = match &c.kind {
                        ChoiceKind::Enum(v) => v.len() as i32,
                        _ => 0,
                    };
                    let shown = n.min(10);
                    let scroll = ((c.value.max(0.0) as i32 - shown / 2).clamp(0, (n - shown).max(0))) * ROW_H;
                    win.drop = if win.drop.map(|d| d.0) == Some(i) { None } else { Some((i, scroll)) };
                }
                _ => {
                    c.cycle(if right { -1 } else { 1 });
                    live_preview(c);
                    acts.extend(choice_side_effect(c));
                }
            }
        }
        (W::ChoiceBtn(i) | W::ChoiceVal(i), Body::Host(h)) => {
            crate::sound::play_player("DeusExSounds.UserInterface.Menu_Press", 0.25, 0.0);
            if let Some(e) = mp_host_cycle(h, i, if right { -1 } else { 1 }) {
                acts.push(Act::Msg("Host Room".into(), e, false, MsgPurpose::Info));
            }
        }
        (W::Header(k), Body::Host(h)) => {
            crate::sound::play_player("DeusExSounds.UserInterface.Menu_Press", 0.25, 0.0);
            mp_host_widget(h, k, acts);
        }
        (W::Header(k), Body::Browse(b)) => {
            crate::sound::play_player("DeusExSounds.UserInterface.Menu_Press", 0.25, 0.0);
            mp_browse_filter(b, k as usize);
        }
        (W::MenuBtn(i), Body::Keypad(k)) => keypad_press(k, i as usize),
        (W::Header(2), Body::SaveLoad(sl)) => {
            crate::sound::play_player("DeusExSounds.UserInterface.Menu_Press", 0.25, 0.0);
            sl.confirm_delete = !sl.confirm_delete;
            crate::settings::set("ConfirmSaveDeletes", if sl.confirm_delete { 0.0 } else { 1.0 });
            crate::settings::save();
        }
        (W::Upgrade | W::Downgrade, Body::NewGame(ng)) => {
            crate::sound::play_player("DeusExSounds.UserInterface.Menu_Press", 0.25, 0.0);
            if let Some(&i) = ng.sel.and_then(|r| ng.order.get(r)) {
                let c = ng.skills.list[i].def.class.clone();
                if w == W::Upgrade {
                    ng.skills.upgrade(&c);
                } else {
                    ng.skills.downgrade(&c, true);
                }
            }
        }
        (W::PortraitPrev, Body::NewGame(ng)) => {
            crate::sound::play_player("DeusExSounds.UserInterface.Menu_Press", 0.25, 0.0);
            let n = if crate::travel::lddp() { 10 } else { 5 };
            ng.portrait = (ng.portrait + n - 1) % n;
        }
        (W::PortraitNext | W::Portrait, Body::NewGame(ng)) => {
            crate::sound::play_player("DeusExSounds.UserInterface.Menu_Press", 0.25, 0.0);
            let n = if crate::travel::lddp() { 10 } else { 5 };
            ng.portrait = (ng.portrait + 1) % n;
        }
        (W::Header(20), Body::NewGame(ng)) => {
            crate::sound::play_player("DeusExSounds.UserInterface.Menu_Press", 0.25, 0.0);
            ng.retro_morpheus = !ng.retro_morpheus;
        }
        (W::Header(21), Body::NewGame(ng)) => {
            crate::sound::play_player("DeusExSounds.UserInterface.Menu_Press", 0.25, 0.0);
            ng.male_interactions = !ng.male_interactions;
        }
        _ => {}
    }
}

/// ResetToDefaults: every choice back to its default (the new-game screen: name, points, portrait, skills).
fn reset_defaults(win: &mut Win) {
    match &mut win.body {
        Body::Choices { choices } => {
            for c in choices.iter_mut() {
                if !matches!(c.kind, ChoiceKind::Action { .. }) {
                    c.value = match c.key.as_str() {
                        "MenuColor" => DEFAULT_MENU_THEME as f32,
                        "HUDColor" => DEFAULT_HUD_THEME as f32,
                        k => crate::settings::default_of(k),
                    };
                    live_preview(c);
                }
            }
        }
        Body::Enhance(en) => {
            for c in en.items.iter_mut() {
                if !matches!(c.kind, ChoiceKind::Action { .. }) {
                    c.value = crate::settings::default_of(&c.key);
                }
            }
        }
        Body::NewGame(ng) => {
            ng.name.text = "JC Denton".chars().collect();
            ng.name.cur = ng.name.text.len();
            ng.name.anchor = ng.name.cur;
            ng.portrait = 0;
            ng.retro_morpheus = false;
            ng.male_interactions = false;
            ng.skills = fresh_skills();
        }
        Body::Keys(k) => {
            if k.pad {
                k.pad_map = crate::gamepad::defaults();
            } else {
                k.map = crate::keybinds::ACTIONS.iter().map(|a| a.1).collect();
            }
        }
        _ => {}
    }
}

// ------------------------------------------------------------------------------------------------ multiplayer

// Ours: the multiplayer screens over online services (online.rs, imsim-net). Nothing connects before Multiplayer is
// pressed; the room closes with its screen and the connection with the last multiplayer window (mp_tick).

/// Action-bar actions of the multiplayer screens (past the original's AB_ values and ours above).
const ACT_START: u8 = 30;
const ACT_CLOSE_ROOM: u8 = 31;
const ACT_REFRESH: u8 = 40;
const ACT_JOIN_ROW: u8 = 41;
const ACT_JOIN_CODE: u8 = 42;
/// Choose Character: join with the selected character; Load Game from Start Game: New Game.
const ACT_CHAR_GO: u8 = 43;
const ACT_MP_NEWGAME: u8 = 44;
/// The connect-failed box: Change Address; Online Services: Save & Connect.
const ACT_ADDR: u8 = 45;
const ACT_ADDR_SAVE: u8 = 46;
/// The connect-failed box: Play Offline; the waiting room: Leave, Ready / Not Ready.
const ACT_OFFLINE: u8 = 47;
const ACT_LEAVE: u8 = 48;
const ACT_READY: u8 = 49;
/// The host's game screen: carry on with the game already up (as its character).
const ACT_MP_PLAYON: u8 = 50;
/// Host Room: Match Settings; the deathmatch character creator: Save Character, Delete; the multiplayer screen
/// and Host Room: Deathmatch Character.
const ACT_MATCH: u8 = 51;
const ACT_DMCHAR_SAVE: u8 = 52;
const ACT_DMCHAR_DEL: u8 = 53;
const ACT_DMCHAR: u8 = 54;
const ACT_LAST: u8 = ACT_DMCHAR;
/// Host Room widgets: Copy Code, and a Kick per player row (40 + row).
const HDR_COPY: u8 = 29;
const HDR_KICK: u8 = 40;
/// Player rows the Host Room's panel shows.
const HOST_ROWS: usize = 5;

/// How often the open multiplayer screen reads online services again.
const MP_POLL: f32 = 0.25;

/// What the menus keep of online services.
#[derive(Default)]
struct MpLink {
    /// Multiplayer connected (or is connecting): leaving its windows disconnects.
    online: bool,
    /// A room was asked for and its screen is up: leaving the screen closes the room.
    hosting: bool,
    poll: f32,
    /// The code being looked up (the answer box names it).
    join_code: String,
    /// The room found, kept while a character is chosen for it.
    joined: Option<imsim_net::client::Joined>,
    /// Ours: playing without online services (rooms by direct code only).
    offline: bool,
    /// The direct code being joined (instead of `joined`).
    direct: Option<String>,
    /// Hosting without online services failed: why (shown by mp_tick).
    host_error: Option<String>,
    /// The waiting room is (or was just) up: closing it leaves the host's lobby.
    waiting: bool,
}

/// MenuScreenWaitingRoom: a joiner in the host's lobby before the game starts.
struct MpWait {
    /// Player, status, ping.
    rows: Vec<Vec<String>>,
    ready: bool,
    /// In the host's lobby (else still connecting).
    connected: bool,
    scroll: i32,
}

/// Choose Character: the player's multiplayer characters (id, name, saved), newest first, after a "New Character"
/// row; the room being joined, in words.
struct MpChars {
    list: Vec<(String, String, u64)>,
    /// The list's rows (New Character, then name / date), made once.
    rows: Vec<Vec<String>>,
    room: String,
    /// Selected row: 0 New Character, then `list` from 1.
    sel: Option<usize>,
    scroll: i32,
}

/// MenuScreenHostRoom: the room's choices and what the lobby says of it (read by `mp_poll_host`).
struct MpHost {
    choices: Vec<Choice>,
    /// The host's name in the browser.
    name: String,
    /// The room's code once the lobby has handed one out.
    code: Option<String>,
    players: u8,
    /// Without online services: no lobby code, no browser.
    offline: bool,
    /// The game session is up (netplay host).
    open: bool,
    /// The long direct code (no lobby needed) and what opening the router's port said.
    direct: Option<String>,
    note: String,
    /// The room's players: (id, name, ready, ping ms, host, me).
    lobby: Vec<(u8, String, bool, u32, bool, bool)>,
    all_ready: bool,
    /// Copy Code was pressed (the button says Copied for a moment).
    copied: Option<std::time::Instant>,
}

/// Online Services: the lobby's address being typed.
struct MpAddr {
    edit: Edit,
}

/// MenuScreenJoinRoom: the code typed so far (upper case, CODE_ALPHABET only, at most 4).
struct MpJoin {
    code: String,
}

/// MenuScreenLobbyBrowser: the public rooms as the lobby last listed them, the header filters, the rows shown.
struct MpBrowse {
    rooms: Vec<RoomInfo>,
    state: crate::online::ListState,
    /// Our own round trip to the lobby when the list came.
    list_ms: u32,
    /// By header: Mode (All / Co-op / Deathmatch), Look (All / Classic / Modern), Players (All / Not Full),
    /// Ping (All / under 100 / under 200).
    filters: [u8; 4],
    /// The rooms that pass the filters (into `rooms`) and their list rows.
    shown: Vec<usize>,
    rows: Vec<Vec<String>>,
    /// Selected row (into `shown`).
    sel: Option<usize>,
    scroll: i32,
}

/// How many values each browser filter cycles through.
const FILTER_STEPS: [u8; 4] = [3, 3, 2, 3];
/// Browser columns: code, host, mode, look | players, tougher enemies, ping (the art's split sits at 240).
const BROWSE_COLS: [i32; 7] = [36, 82, 70, 52, 40, 18, 58];
/// The list's place in the Load Game frame (as the saves list).
const BROWSE_LIST: Rect = (11, 22, 371, 270);

/// A multiplayer screen's frame and body. Frames are the original's: big buttons on the Settings menu's, the room's
/// choices and the code on the Options screen's, the room list on the Load Game screen's.
fn mp_window(assets: &mut RuntimeAssets, class: &str) -> (Frame, Body) {
    match class {
        "MenuScreenMultiplayer" => {
            let mut f = frame(assets, "MenuSettings");
            f.class = class.to_string();
            f.title = "Multiplayer".into();
            let (_, x, w) = menu_buttons(assets, "MenuSettings");
            let b = |label: &str, y: i32, action: u8, invoke: &str| MenuBtn {
                label: label.into(),
                y,
                action,
                invoke: invoke.into(),
                key: String::new(),
                enabled: true,
            };
            // Wells 13..121 for the buttons, 157 for the connection line, 193 for the server's address, 229 for
            // the deathmatch character.
            const CUT: (i32, i32) = (224, 0);
            let mut dm = b("Deathmatch Character", 229, 1, "MenuScreenDmCharacter");
            dm.enabled = crate::dmchar::blocked().is_none();
            let buttons = vec![
                b("Host Room", 13, 1, "MenuScreenHostRoom"),
                b("Join Room", 49, 1, "MenuScreenJoinRoom"),
                b("Lobby Browser", 85, 1, "MenuScreenLobbyBrowser"),
                b("Online Services", 121, 1, "MenuScreenOnlineServices"),
                dm,
                b("Previous Menu", 266 - CUT.1, 2, ""),
            ];
            let addr = format!("Server: {}", crate::online::server_addr());
            (f, Body::Menu { buttons, x, w, notes: vec![(157, mp_status_text()), (193, addr)] })
        }
        "MenuScreenHostRoom" => {
            let mut f = frame(assets, "MenuScreenOptions");
            f.class = class.to_string();
            f.title = "Host Room".into();
            let choices = mp_host_choices();
            f.actions = vec![
                (0, ACT_START, "Start Game".into(), String::new()),
                (0, ACT_MATCH, "Match Settings".into(), String::new()),
                (0, ACT_DMCHAR, "Character".into(), String::new()),
                (2, ACT_CLOSE_ROOM, "Close Room".into(), String::new()),
            ];
            let name = crate::travel::with_session(|s| s.name.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| "Player".into());
            // All nine rows of art: the six choices, then the players panel over the last three.
            let host = MpHost {
                choices,
                name,
                code: None,
                players: 1,
                offline: false,
                open: false,
                direct: None,
                note: String::new(),
                lobby: Vec::new(),
                all_ready: true,
                copied: None,
            };
            (f, Body::Host(host))
        }
        "MenuScreenJoinRoom" => {
            let mut f = frame(assets, "MenuScreenOptions");
            f.class = class.to_string();
            f.title = "Join Room".into();
            let slots = art_rows(&f);
            cut_empty_rows(&mut f, 1, slots);
            let cancel = f.actions.iter().find(|a| a.1 == 2).map(|a| a.2.clone()).unwrap_or_else(|| "Cancel".into());
            f.actions = vec![(2, 2, cancel, String::new()), (2, ACT_JOIN_CODE, "Join Room".into(), String::new())];
            (f, Body::Join(MpJoin { code: String::new() }))
        }
        "MenuScreenOnlineServices" => {
            let mut f = frame(assets, "MenuScreenOptions");
            f.class = class.to_string();
            f.title = "Online Services".into();
            let slots = art_rows(&f);
            cut_empty_rows(&mut f, 1, slots);
            let cancel = f.actions.iter().find(|a| a.1 == 2).map(|a| a.2.clone()).unwrap_or_else(|| "Cancel".into());
            f.actions = vec![(2, 2, cancel, String::new()), (2, ACT_ADDR_SAVE, "Save & Connect".into(), String::new())];
            let mut edit = Edit::new(64);
            edit.text = crate::online::server_addr().chars().collect();
            edit.cur = edit.text.len();
            edit.anchor = 0;
            (f, Body::Addr(MpAddr { edit }))
        }
        "MenuScreenWaitingRoom" => {
            let mut f = frame(assets, "MenuScreenCustomizeKeys");
            f.class = class.to_string();
            f.title = "Waiting Room".into();
            f.actions = vec![(2, ACT_LEAVE, "Leave".into(), String::new()), (2, ACT_READY, "Ready".into(), String::new())];
            (f, Body::Wait(MpWait { rows: Vec::new(), ready: false, connected: false, scroll: 0 }))
        }
        "MenuScreenDmCharacter" => {
            let mut f = frame(assets, "MenuScreenCustomizeKeys");
            f.class = class.to_string();
            f.title = "Deathmatch Character".into();
            f.actions = vec![
                (2, 2, "Cancel".into(), String::new()),
                (2, ACT_DMCHAR_SAVE, "Save Character".into(), String::new()),
                (0, ACT_DMCHAR_DEL, "Delete".into(), String::new()),
            ];
            let c = crate::dmchar::last().unwrap_or_else(|| {
                let mut c = crate::dmchar::fresh();
                if let Some(n) = crate::travel::with_session(|s| s.name.clone()).filter(|n| !n.trim().is_empty()) {
                    c.name = n;
                }
                c
            });
            (f, Body::DmChar(DmEdit::new(c)))
        }
        "MenuScreenChooseCharacter" => {
            let mut f = frame(assets, "MenuScreenLoadGame");
            f.class = class.to_string();
            f.title = "Choose Character".into();
            f.actions = vec![(2, 2, "Cancel".into(), String::new()), (2, ACT_CHAR_GO, "Join Game".into(), String::new())];
            let mut list = crate::save::mp_characters();
            list.sort_by(|a, b| b.2.cmp(&a.2));
            // The newest character first (else New Character).
            let sel = Some(if list.is_empty() { 0 } else { 1 });
            let mut rows: Vec<Vec<String>> = Vec::with_capacity(list.len() + 1);
            rows.push(vec!["New Character".into(), String::new()]);
            rows.extend(list.iter().map(|(_, name, t)| vec![format!("    {name}"), crate::save::format_date(*t)]));
            (f, Body::Chars(MpChars { list, rows, room: String::new(), sel, scroll: 0 }))
        }
        _ => {
            let mut f = frame(assets, "MenuScreenLoadGame");
            f.class = class.to_string();
            f.title = "Lobby Browser".into();
            f.actions = vec![
                (2, 4, "Previous Menu".into(), String::new()),
                (2, ACT_JOIN_ROW, "Join Room".into(), String::new()),
                (0, ACT_REFRESH, "Refresh".into(), String::new()),
            ];
            let b = MpBrowse {
                rooms: Vec::new(),
                state: crate::online::ListState::Loading,
                list_ms: 0,
                filters: [0; 4],
                shown: Vec::new(),
                rows: Vec::new(),
                sel: None,
                scroll: 0,
            };
            (f, Body::Browse(b))
        }
    }
}

/// An Enum choice of the host's room.
fn mp_choice(key: &str, label: &str, help: &str, values: Vec<String>, value: usize) -> Choice {
    Choice {
        key: key.into(),
        label: label.into(),
        help: help.into(),
        kind: ChoiceKind::Enum(values),
        value: value as f32,
        saved: value as f32,
        info_w: 110,
        info_x: 270,
        control_x: 270,
        voff: 0,
        dropdown: false,
    }
}

/// Game Mode, Room, Players, Look, Tougher Enemies (RoomSettings' defaults; the look is the one being played),
/// Friendly Fire (off).
fn mp_host_choices() -> Vec<Choice> {
    let d = RoomSettings::default();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<String>>();
    let modern = crate::remaster::is_modern();
    vec![
        mp_choice(
            "MpMode",
            "|&Game Mode",
            "Co-op: play the story together. Deathmatch: everyone for themselves on the multiplayer maps. Team Deathmatch: UNATCO against the NSF, up to 8 a side. Match Settings sets the map and limits.",
            // (Android hosts co-op only: deathmatch needs Modern and Augmentique.)
            if crate::platform::android() { s(&["Co-op"]) } else { s(&["Co-op", "Deathmatch", "Team Deathmatch"]) },
            0,
        ),
        mp_choice(
            "MpRoom",
            "|&Room",
            "Public rooms are listed in the Lobby Browser. Private rooms are found by their code alone.",
            s(&["Public", "Private"]),
            if d.public { 0 } else { 1 },
        ),
        mp_choice(
            "MpPlayers",
            "|&Players",
            "Most players in the room. Fewer players are recommended for co-op.",
            (2..=imsim_net::MAX_PLAYERS).map(|n| n.to_string()).collect(),
            (d.max_players.clamp(2, imsim_net::MAX_PLAYERS) - 2) as usize,
        ),
        if crate::platform::android() {
            mp_choice("MpLook", "|&Look", "Android plays Classic: the original game for everyone in the room.", s(&["Classic"]), 0)
        } else {
            mp_choice("MpLook", "|&Look", &mp_look_help(!modern), s(&["Classic", "Modern"]), if modern { 1 } else { 0 })
        },
        mp_choice("MpTough", "|&Tougher Enemies", "Co-op: enemies get tougher with more players.", s(&["Off", "On"]), if d.tough { 1 } else { 0 }),
        mp_choice("MpFriendly", "|&Friendly Fire", "Players' shots and blasts hurt each other.", s(&["Off", "On"]), 0),
    ]
}

/// The room as the host's choices set it.
fn mp_host_settings(h: &MpHost) -> RoomSettings {
    let v = |k: &str| h.choices.iter().find(|c| c.key == k).map_or(0, |c| c.value.max(0.0) as u8);
    RoomSettings {
        mode: match v("MpMode") {
            1 => Mode::Deathmatch,
            2 => Mode::TeamDeathmatch,
            _ => Mode::Coop,
        },
        public: v("MpRoom") == 0,
        max_players: (2 + v("MpPlayers")).min(imsim_net::MAX_PLAYERS),
        look: if v("MpLook") == 0 { Look::Classic } else { Look::Modern },
        tough: v("MpTough") == 1,
        name: h.name.clone(),
        detail: mp_dm_detail(v("MpMode")),
    }
}

/// A deathmatch room's map and limits for the browser ("Cathedral/20_kills/10_min").
fn mp_dm_detail(mode: u8) -> String {
    if mode == 0 {
        return String::new();
    }
    let r = crate::deathmatch::rules_from_settings(crate::deathmatch::Mode::Deathmatch, false);
    let mut d = crate::deathmatch::map_title(&r.map).replace(' ', "_").replace('\'', "");
    if r.frag_limit > 0 {
        d.push_str(&format!("/{}_kills", r.frag_limit));
    }
    if r.time_limit > 0 {
        d.push_str(&format!("/{}_min", r.time_limit));
    }
    d
}

/// The Look choice's help: in Classic it says which of the host's mods are switched off while the room lasts.
fn mp_look_help(classic: bool) -> String {
    let base = "Everyone plays in this look. The Classic / Modern switch is off in multiplayer.";
    match (classic, crate::netplay::classic_mods_note()) {
        (true, Some(note)) => format!("{base} {note}"),
        _ => base.to_string(),
    }
}

/// A host choice stepped (click, Left / Right): the lobby (or, for Friendly Fire, the game session) hears of it at once.
/// Classic with mods installed is fine: the host's mods are switched off while the room lasts (netplay::force_look,
/// said in the Look choice's help). Returns a message for a box (none now).
fn mp_host_cycle(h: &mut MpHost, i: u8, dir: i32) -> Option<String> {
    let c = h.choices.get_mut(i as usize)?;
    let before = c.value;
    c.cycle(dir);
    // Deathmatch needs Augmentique (outfits, weapon skins) and is played in Modern.
    if c.key == "MpMode" && c.value > 0.5 {
        if let Some(why) = crate::dmchar::blocked() {
            c.value = 0.0;
            return Some(why.to_string());
        }
    }
    if c.key == "MpLook" && c.value < 0.5 && mp_host_versus(h) {
        let c = h.choices.get_mut(i as usize)?;
        c.value = 1.0;
        return Some("Deathmatch is played in the Modern look.".into());
    }
    let c = h.choices.get_mut(i as usize)?;
    if c.key == "MpMode" {
        let versus = c.value > 0.5;
        let changed = (before > 0.5) != versus || (before - c.value).abs() > 0.5;
        if let Some(l) = h.choices.iter_mut().find(|c| c.key == "MpLook") {
            if versus {
                l.value = 1.0;
            }
        }
        if changed {
            mp_apply_dm(h);
        }
    }
    let Some(c) = h.choices.get_mut(i as usize) else { return None };
    if c.key == "MpLook" {
        c.help = mp_look_help(c.value < 0.5);
    }
    if c.key == "MpFriendly" {
        crate::netplay::set_friendly_fire(c.value > 0.5);
    } else {
        crate::online::update(mp_host_settings(h));
    }
    mp_apply_dm(h);
    None
}

/// The room is a deathmatch (either kind).
fn mp_host_versus(h: &MpHost) -> bool {
    h.choices.iter().find(|c| c.key == "MpMode").is_some_and(|c| c.value > 0.5)
}

/// The host's deathmatch rules for his room (deathmatch.rs) from the mode, Friendly Fire and Match Settings.
fn mp_apply_dm(h: &MpHost) {
    let v = |k: &str| h.choices.iter().find(|c| c.key == k).map_or(0.0, |c| c.value);
    let mode = match v("MpMode") as u8 {
        1 => crate::deathmatch::Mode::Deathmatch,
        2 => crate::deathmatch::Mode::Team,
        _ => crate::deathmatch::Mode::Coop,
    };
    crate::deathmatch::set_rules(mode.versus().then(|| crate::deathmatch::rules_from_settings(mode, v("MpFriendly") > 0.5)));
}

/// The Host Room's own buttons: Copy Code, Kick on a player's row.
fn mp_host_widget(h: &mut MpHost, k: u8, acts: &mut Vec<Act>) {
    if k == HDR_COPY {
        let Some(code) = h.direct.as_deref() else { return };
        match crate::netplay::copy_to_clipboard(code) {
            Ok(()) => h.copied = Some(std::time::Instant::now()),
            Err(e) => acts.push(Act::Msg("Copy Code".into(), format!("The code couldn't be copied ({e}). It is {code}."), false, MsgPurpose::Info)),
        }
    } else if let Some(p) = k.checked_sub(HDR_KICK).and_then(|r| h.lobby.get(r as usize)) {
        if !p.4 {
            crate::netplay::kick(p.0);
        }
    }
}

fn mp_host_help(h: &MpHost, focus: Option<W>, f: &Frame) -> String {
    let action = match focus {
        Some(W::Act(i)) => f.actions.get(i as usize).map(|a| a.1),
        _ => None,
    };
    match (focus, action) {
        (Some(W::ChoiceBtn(i)), _) => h.choices.get(i as usize).map(|c| c.help.clone()).unwrap_or_default(),
        (_, Some(ACT_CLOSE_ROOM)) => "Close the room: everyone in it leaves.".into(),
        (Some(W::Header(HDR_COPY)), _) => "Copy the direct code to the clipboard: players can join with it without online services.".into(),
        (Some(W::Header(k)), _) if k >= HDR_KICK => "Send this player away.".into(),
        _ if !h.open => "Creating room...".into(),
        _ if !h.all_ready => "Waiting for every player to be ready.".into(),
        _ if intro_map() => "Start Game: load your game or start a new one; players who join are let in once you're on a playable map.".into(),
        _ => "Start Game: back to your game with the room open; players who join come into your map.".into(),
    }
}

/// The multiplayer screen's connection line.
fn mp_status_text() -> String {
    match crate::online::status() {
        crate::online::Status::Online { server, ms } => format!("Online: {server} ({ms} ms)"),
        crate::online::Status::Connecting => "Connecting...".into(),
        crate::online::Status::Failed(m) => m,
        crate::online::Status::Offline => "Offline".into(),
    }
}

/// Characters of a direct code (15, shown in three groups of five).
const DIRECT_LEN: usize = 15;

/// The join screen takes the code: letters typed in upper case, only CODE_ALPHABET (dashes and spaces skipped), a
/// lobby code (4) or a direct code (15); Ctrl+V pastes; Backspace / Delete take the last one off. True when the key
/// was the code's (Escape / Enter / Tab / arrows go on).
fn mp_join_typing(j: &mut MpJoin, k: &KeyCode, text: &Option<String>, ctrl: bool) -> bool {
    match k {
        KeyCode::Escape | KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Tab => false,
        KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::ArrowLeft | KeyCode::ArrowRight => false,
        KeyCode::Backspace | KeyCode::Delete => {
            j.code.pop();
            true
        }
        KeyCode::KeyV if ctrl => {
            let pasted = crate::platform::clipboard_get().unwrap_or_default();
            j.code.clear();
            mp_code_push(&mut j.code, &pasted);
            true
        }
        _ => {
            if !ctrl {
                mp_code_push(&mut j.code, text.as_deref().unwrap_or(""));
            }
            true
        }
    }
}

/// Code characters of `text` added to `code` (upper case, the alphabet only, at most a direct code's length).
fn mp_code_push(code: &mut String, text: &str) {
    for c in text.chars() {
        let u = c.to_ascii_uppercase();
        if code.len() < DIRECT_LEN && u.is_ascii() && imsim_net::CODE_ALPHABET.contains(&(u as u8)) {
            code.push(u);
        }
    }
}

/// A code as shown: a direct code's characters in groups of five.
fn mp_code_shown(code: &str) -> String {
    if code.len() <= 4 {
        return code.to_string();
    }
    let mut s = String::with_capacity(DIRECT_LEN + 2);
    for (i, c) in code.chars().enumerate() {
        if i > 0 && i % 5 == 0 {
            s.push('-');
        }
        s.push(c);
    }
    s
}

/// What Join Room does with the code typed: look a lobby code up, or join a direct code.
fn mp_join_act(code: &str) -> Option<Act> {
    if code.len() == 4 && imsim_net::valid_code(code) {
        return Some(Act::Join(code.to_string()));
    }
    let shown = mp_code_shown(code);
    (code.len() == DIRECT_LEN && crate::netplay::is_direct_code(&shown)).then_some(Act::MpDirect(shown))
}

/// Enter on a multiplayer screen: step the focused host choice, join the typed code or the selected room.
fn mp_enter(win: &mut Win, acts: &mut Vec<Act>) {
    match (&mut win.body, win.focus) {
        (Body::Host(h), Some(W::ChoiceBtn(i))) => {
            if let Some(e) = mp_host_cycle(h, i, 1) {
                acts.push(Act::Msg("Host Room".into(), e, false, MsgPurpose::Info));
            }
        }
        (Body::Join(j), _) => acts.extend(mp_join_act(&j.code)),
        (Body::Browse(b), _) => acts.extend(mp_browse_join(b)),
        (Body::Chars(c), _) => acts.extend(mp_chars_go(c)),
        (Body::Addr(a), _) => acts.push(Act::MpSetAddr(a.edit.get())),
        (Body::Wait(w), _) => {
            crate::netplay::set_ready(!w.ready);
            w.ready = !w.ready;
        }
        _ => {}
    }
}

/// What an address field takes of typed text: host-name, IPv4 / IPv6 and port characters (None: nothing left).
fn mp_addr_typed(text: Option<&str>) -> Option<String> {
    let t: String = text?.chars().filter(|c| c.is_ascii_alphanumeric() || ".-:[]_".contains(*c)).collect();
    (!t.is_empty()).then_some(t)
}

/// An address as typed, with the lobby's default port when none is given ("" stays "": this PC).
fn mp_addr_normalize(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() {
        return String::new();
    }
    let port = imsim_net::DEFAULT_PORT;
    if let Some(rest) = s.strip_prefix('[') {
        // [IPv6] or [IPv6]:port
        return match rest.split_once(']') {
            Some((ip, tail)) if tail.strip_prefix(':').is_some_and(|p| p.parse::<u16>().is_ok()) => format!("[{ip}]{tail}"),
            Some((ip, _)) => format!("[{ip}]:{port}"),
            None => format!("[{rest}]:{port}"),
        };
    }
    match s.matches(':').count() {
        0 => format!("{s}:{port}"),
        1 => match s.split_once(':') {
            Some((h, p)) if p.parse::<u16>().is_ok() && !h.is_empty() => s.to_string(),
            Some((h, _)) => format!("{h}:{port}"),
            None => s.to_string(),
        },
        // A bare IPv6 address.
        _ => format!("[{s}]:{port}"),
    }
}

/// Online Services saved: launcher.txt's LobbyServer= line, then a fresh connection there (the multiplayer windows
/// close; the connecting box takes over).
fn mp_set_addr(st: &mut Menus, hud: &mut ClassicHud, assets: &mut RuntimeAssets, typed: &str) {
    let addr = mp_addr_normalize(typed);
    if let Err(e) = crate::online::set_server_addr(&addr) {
        let w = msg_box(hud, assets, "Online Services", &format!("The address couldn't be saved ({e})."), false, MsgPurpose::Info);
        st.push(w);
        return;
    }
    if st.mp.hosting {
        crate::online::close_room();
    }
    crate::online::disconnect();
    st.stack.retain(|w| !mp_win(w) && !matches!(w.body, Body::Addr(_)));
    st.mp = MpLink::default();
    mp_connect(st, hud, assets);
}

/// A window opened with ":mp": the new-game screen makes a multiplayer character; the Load Game screen (Start Game
/// on the logo map) gains New Game.
fn mp_variant(w: &mut Win) {
    match &mut w.body {
        Body::NewGame(ng) => {
            ng.multiplayer = true;
            w.f.title = "New Multiplayer Character".into();
            w.focus = Some(W::Edit(0));
        }
        Body::SaveLoad(_) => {
            w.f.title = "Host: Choose Your Game".into();
            w.f.actions.push((0, ACT_MP_NEWGAME, "New Character".into(), String::new()));
            if !intro_map() {
                w.f.actions.push((0, ACT_MP_PLAYON, "Play On".into(), String::new()));
            }
        }
        _ => {}
    }
}

/// The main menu's slot 8: Multiplayer, or Leave Multiplayer while in a game session.
fn mp_main_slot(b: &mut MenuBtn) {
    let leave = crate::netplay::active();
    b.label = if leave { "Leave Multiplayer" } else { "Multiplayer" }.into();
    b.invoke = if leave { "LeaveMultiplayer" } else { "MenuScreenMultiplayer" }.into();
    b.action = 1;
    b.enabled = true;
}

/// Leave Multiplayer: the session, the room and the lobby; the main menu's slot says Multiplayer again.
fn mp_leave(st: &mut Menus) {
    crate::netplay::leave("");
    crate::online::close_room();
    crate::online::disconnect();
    st.mp = MpLink::default();
    for w in st.stack.iter_mut().filter(|w| w.f.class == "MenuMain") {
        if let Body::Menu { buttons, .. } = &mut w.body {
            if let Some(b) = buttons.get_mut(8) {
                mp_main_slot(b);
            }
        }
    }
}

/// Choose Character: the selected row's way in.
/// The box when a name is already played by someone in the room.
fn name_taken_text(who: &str) -> String {
    format!("{who} is already in this room. Every player needs their own name: pick another character or give this one a different name.")
}

fn mp_chars_go(c: &MpChars) -> Option<Act> {
    match c.sel? {
        0 => Some(Act::Push("MenuScreenNewGame:mp".into())),
        i => c.list.get(i - 1).map(|ch| Act::MpLoadChar(ch.0.clone())),
    }
}

fn mp_chars_move(c: &mut MpChars, d: i32) {
    let n = c.list.len() as i32 + 1;
    let next = (c.sel.map_or(-1, |s| s as i32) + d).clamp(0, n - 1);
    c.sel = Some(next as usize);
    let (y, h) = (next * ROW_H, BROWSE_LIST.3);
    if y < c.scroll {
        c.scroll = y;
    }
    if y + ROW_H > c.scroll + h {
        c.scroll = y + ROW_H - h;
    }
}

/// Into the found room with the chosen character (netplay.rs): the character screens close, a box says how it went.
fn mp_join_room(st: &mut Menus, hud: &mut ClassicHud, assets: &mut RuntimeAssets) {
    let r = match (&st.mp.direct, &st.mp.joined) {
        (Some(code), _) => Some(crate::netplay::join_code(code)),
        (None, Some(j)) => Some(crate::netplay::join_room(j)),
        _ => None,
    };
    let text = match r {
        Some(Ok(())) => {
            // Connected: the waiting room until the host starts.
            st.stack.retain(|w| !matches!(&w.body, Body::Chars(_)) && !matches!(&w.body, Body::NewGame(ng) if ng.multiplayer));
            st.mp.waiting = true;
            st.mp.poll = 0.0;
            let w = open_window(hud, assets, "MenuScreenWaitingRoom");
            st.push(w);
            return;
        }
        Some(Err(e)) => e,
        None => "The room was lost. Look it up again.".into(),
    };
    let w = msg_box(hud, assets, "Join Room", &text, false, MsgPurpose::MpResult);
    st.push(w);
}

/// The action bar of a multiplayer screen. True when the room was closed.
fn mp_action(win: &mut Win, action: u8, acts: &mut Vec<Act>) -> bool {
    match (action, &mut win.body) {
        (ACT_START, Body::Host(h)) if h.open && h.all_ready => {
            // The lobby's players come in once a map is up. On a playable map: back to the game, the room open. On
            // the logo map: the host's story first (a save, or New Game on the same screen).
            // The host picks his game every time: a save (any character's), a new character (New Game: the
            // character creator), or, with a game up, carrying on with it.
            if crate::deathmatch::versus() {
                // A deathmatch room: the host's story isn't used - the chosen map loads for everyone, each
                // player as his deathmatch character.
                match crate::dmchar::blocked() {
                    Some(why) => acts.push(Act::Msg("Host Room".into(), why.into(), false, MsgPurpose::Info)),
                    None => match crate::dmchar::last() {
                        None => {
                            acts.push(Act::Push("MenuScreenDmCharacter".into()));
                            acts.push(Act::Msg("Host Room".into(), "Make your deathmatch character first, then Start Game.".into(), false, MsgPurpose::Info));
                        }
                        Some(c) => {
                            mp_apply_dm(h);
                            let map = crate::deathmatch::rules().map(|r| r.map).unwrap_or_else(|| "DXMP_Cathedral".into());
                            let was_on = crate::netplay::started();
                            crate::dmchar::apply(&c);
                            crate::netplay::start_game();
                            let file = format!("{map}.dx");
                            if was_on && !intro_map() {
                                crate::netplay::host_travel(&map);
                            }
                            crate::loader::request(&file, crate::loader::Goal::NewGame(crate::dmchar::new_game(&c)));
                            acts.push(Act::PopAll);
                        }
                    },
                }
                return false;
            }
            crate::netplay::start_game();
            acts.push(Act::Push("MenuScreenLoadGame:mp".into()));
        }
        (ACT_MP_NEWGAME, _) => acts.push(Act::Push("MenuSelectDifficulty".into())),
        (ACT_MP_PLAYON, _) => acts.push(Act::PopAll),
        (ACT_MATCH, _) => acts.push(Act::Push("MenuScreenMatchSettings".into())),
        (ACT_DMCHAR, _) => match crate::dmchar::blocked() {
            Some(why) => acts.push(Act::Msg("Deathmatch Character".into(), why.into(), false, MsgPurpose::Info)),
            None => acts.push(Act::Push("MenuScreenDmCharacter".into())),
        },
        (ACT_DMCHAR_SAVE, Body::DmChar(d)) => match dm_char_save(d) {
            Ok(()) => {
                crate::sound::play_player("DeusExSounds.UserInterface.Menu_OK", 0.25, 0.0);
                acts.push(Act::Pop);
            }
            Err(m) => acts.push(Act::Msg("Deathmatch Character".into(), m, false, MsgPurpose::Info)),
        },
        (ACT_DMCHAR_DEL, Body::DmChar(d)) => {
            if !d.c.id.is_empty() {
                crate::dmchar::delete(&d.c.id);
            }
            let next = crate::dmchar::list().into_iter().next().unwrap_or_else(crate::dmchar::fresh);
            *d = DmEdit::new(next);
        }
        (ACT_ADDR, _) => {
            // From the connect-failed box: the box goes, the address screen comes.
            acts.push(Act::Pop);
            acts.push(Act::Push("MenuScreenOnlineServices".into()));
        }
        (ACT_ADDR_SAVE, Body::Addr(a)) => acts.push(Act::MpSetAddr(a.edit.get())),
        (ACT_CHAR_GO, Body::Chars(c)) => acts.extend(mp_chars_go(c)),
        (ACT_CLOSE_ROOM, Body::Host(_)) => {
            crate::online::close_room();
            acts.push(Act::Pop);
            return true;
        }
        (ACT_REFRESH, Body::Browse(b)) => {
            crate::online::refresh_list();
            b.rooms.clear();
            b.state = crate::online::ListState::Loading;
            b.scroll = 0;
            mp_browse_rebuild(b);
        }
        (ACT_JOIN_ROW, Body::Browse(b)) => acts.extend(mp_browse_join(b)),
        (ACT_JOIN_CODE, Body::Join(j)) => acts.extend(mp_join_act(&j.code)),
        (ACT_OFFLINE, _) => {
            acts.push(Act::Pop);
            acts.push(Act::MpOffline);
        }
        (ACT_READY, Body::Wait(w)) => {
            crate::netplay::set_ready(!w.ready);
            w.ready = !w.ready;
        }
        (ACT_LEAVE, Body::Wait(_)) => {
            crate::netplay::leave("");
            acts.push(Act::Pop);
        }
        _ => {}
    }
    false
}

/// Join with a direct code: choose a character first (Choose Character, then netplay::join_code).
fn mp_direct(st: &mut Menus, hud: &mut ClassicHud, assets: &mut RuntimeAssets, code: String) {
    if !crate::netplay::is_direct_code(&code) {
        let w = msg_box(hud, assets, "Join Room", "That isn't a room code.", false, MsgPurpose::MpResult);
        st.push(w);
        return;
    }
    let room = format!("Direct code {code}: the host's PC is reached without online services.");
    st.mp.direct = Some(code);
    st.mp.joined = None;
    let mut w = open_window(hud, assets, "MenuScreenChooseCharacter");
    if let Some(Body::Chars(c)) = w.as_mut().map(|w| &mut w.body) {
        c.room = room;
    }
    st.push(w);
}

/// Play Offline (online services couldn't be reached): the Multiplayer screen without the lobby - rooms are hosted
/// and joined by direct code.
fn mp_offline(st: &mut Menus, hud: &mut ClassicHud, assets: &mut RuntimeAssets) {
    crate::online::disconnect();
    st.mp = MpLink { online: true, offline: true, ..Default::default() };
    let mut w = open_window(hud, assets, "MenuScreenMultiplayer");
    if let Some(Body::Menu { buttons, notes, .. }) = w.as_mut().map(|w| &mut w.body) {
        for b in buttons.iter_mut().filter(|b| b.invoke == "MenuScreenLobbyBrowser") {
            b.enabled = false;
        }
        if let Some(n) = notes.first_mut() {
            n.1 = "Offline: rooms by direct code".into();
        }
    }
    st.push(w);
}

/// A connected joiner's waiting room (or why the host turned him away), a few times a second.
fn mp_wait_tick(st: &mut Menus, hud: &mut ClassicHud, assets: &mut RuntimeAssets) {
    let Some(Body::Wait(w)) = st.stack.last_mut().map(|w| &mut w.body) else { return };
    if crate::netplay::in_game() {
        // The host started: he travels us in.
        st.mp.waiting = false;
        st.stack.clear();
        return;
    }
    if !crate::netplay::active() {
        let why = crate::netplay::last_end();
        let why = if why.is_empty() { "The connection to the host was lost.".to_string() } else { why };
        st.mp.waiting = false;
        st.pop();
        let w = msg_box(hud, assets, "Join Room", &why, false, MsgPurpose::MpResult);
        st.push(w);
        return;
    }
    let lobby = crate::netplay::in_lobby();
    w.connected |= lobby;
    w.ready = crate::netplay::is_ready();
    let rows: Vec<Vec<String>> = crate::netplay::lobby_players()
        .into_iter()
        .map(|p| {
            let who = if p.host { format!("{} (host)", p.name) } else if p.me { format!("{} (you)", p.name) } else { p.name };
            let state = if p.ready || p.host { "Ready" } else { "Not ready" };
            let ping = if p.me { String::new() } else { format!("{} ms", p.ping.round() as u32) };
            vec![who, state.to_string(), ping]
        })
        .collect();
    if rows != w.rows {
        w.rows = rows;
    }
    // The Ready button says what it will do.
    let label = if w.ready { "Not Ready" } else { "Ready" };
    if let Some(top) = st.stack.last_mut() {
        if let Some(a) = top.f.actions.iter_mut().find(|a| a.1 == ACT_READY) {
            if a.2 != label {
                a.2 = label.into();
            }
        }
    }
}

/// Multiplayer pressed: in at once when connected, else connect behind "Connecting to online services...".
fn mp_connect(st: &mut Menus, hud: &mut ClassicHud, assets: &mut RuntimeAssets) {
    st.mp.online = true;
    if matches!(crate::online::status(), crate::online::Status::Online { .. }) {
        let w = open_window(hud, assets, "MenuScreenMultiplayer");
        st.push(w);
        return;
    }
    crate::online::connect();
    let w = mp_wait_box(hud, assets, "Multiplayer", "Connecting to online services...", MsgPurpose::Connecting);
    st.push(w);
}

/// Look a room up by code behind "Looking for room...".
fn mp_join(st: &mut Menus, hud: &mut ClassicHud, assets: &mut RuntimeAssets, code: String) {
    if st.mp.offline {
        let t = "Four-character room codes are looked up by online services. Without them, ask the host for the long direct code.";
        let w = msg_box(hud, assets, "Join Room", t, false, MsgPurpose::MpResult);
        st.push(w);
        return;
    }
    st.mp.direct = None;
    crate::online::join(&code);
    st.mp.join_code = code;
    let w = mp_wait_box(hud, assets, "Join Room", "Looking for room...", MsgPurpose::Joining);
    st.push(w);
}

/// A message box with one Cancel button (Escape too), answered by `mp_tick`.
fn mp_wait_box(hud: &mut ClassicHud, assets: &mut RuntimeAssets, title: &str, text: &str, purpose: MsgPurpose) -> Option<Win> {
    let mut w = msg_box(hud, assets, title, text, false, purpose)?;
    let cancel = pstr(&assets.classes.defaults("MenuMain"), "btnLabelCancel");
    if let Some(a) = w.f.actions.first_mut() {
        a.2 = if cancel.is_empty() { "Cancel".into() } else { cancel };
    }
    Some(w)
}

/// A window just opened (Act::Push): the host's screen asks for its room, the browser for the list.
fn mp_opened(link: &mut MpLink, w: &mut Win) {
    match &mut w.body {
        Body::Host(h) => {
            if link.offline {
                // Without online services: the game socket alone (direct code only), no lobby room.
                h.offline = true;
                if let Some(c) = h.choices.iter_mut().find(|c| c.key == "MpRoom") {
                    c.kind = ChoiceKind::Enum(vec!["Direct Code".into()]);
                    c.value = 0.0;
                    c.help = "Without online services the room isn't listed: players join with its direct code.".into();
                }
                if let Err(e) = crate::netplay::open_host(crate::online::rules_of(&mp_host_settings(h))) {
                    link.host_error = Some(e);
                }
            } else {
                crate::online::host(mp_host_settings(h));
            }
            crate::netplay::set_friendly_fire(h.choices.iter().any(|c| c.key == "MpFriendly" && c.value > 0.5));
            link.hosting = true;
            w.focus = Some(W::ChoiceBtn(0));
        }
        Body::Browse(_) => crate::online::refresh_list(),
        Body::Join(_) | Body::Addr(_) => w.focus = Some(W::ChoiceBtn(0)),
        Body::Wait(_) => link.waiting = true,
        _ => {}
    }
    link.poll = 0.0;
}

/// A window of the multiplayer screens (or one of their boxes).
fn mp_win(w: &Win) -> bool {
    matches!(w.body, Body::Host(_) | Body::Join(_) | Body::Browse(_) | Body::Chars(_) | Body::Wait(_))
        || matches!(&w.body, Body::NewGame(ng) if ng.multiplayer)
        || w.f.class == "MenuScreenMultiplayer"
        || matches!(&w.body, Body::Msg(m) if matches!(m.purpose, MsgPurpose::Connecting | MsgPurpose::Joining | MsgPurpose::MpResult))
}

/// The top window is a message box for this purpose.
fn top_box(st: &Menus, purpose: MsgPurpose) -> bool {
    matches!(st.stack.last().map(|w| &w.body), Some(Body::Msg(m)) if m.purpose == purpose)
}

/// Each frame: leaving (disconnect / close the room), the waiting boxes' answers, a lost connection, and a few
/// times a second what the top multiplayer screen shows.
fn mp_tick(st: &mut Menus, dt: f32, hud: &mut ClassicHud, assets: &mut RuntimeAssets) {
    if !st.mp.online {
        return;
    }
    // A game session (netplay.rs) going on owns the room and the connection once the menus are closed: back in
    // the game from the host's screen the room stays open, and the lobby stays connected.
    let session = crate::netplay::active();
    if st.mp.hosting && !st.stack.iter().any(|w| matches!(w.body, Body::Host(_))) {
        if !(st.stack.is_empty() && session) {
            crate::online::close_room();
        }
        st.mp.hosting = false;
    }
    // The waiting room closed (Leave, Escape) before the host started: out of his lobby.
    if st.mp.waiting && !st.stack.iter().any(|w| matches!(w.body, Body::Wait(_))) {
        st.mp.waiting = false;
        if crate::netplay::in_lobby() || (session && !crate::netplay::in_game() && !crate::netplay::is_host()) {
            crate::netplay::leave("");
        }
    }
    if !st.stack.iter().any(mp_win) {
        // (Asked again: closing the room above ends the host's session.)
        if !crate::netplay::active() {
            crate::online::disconnect();
        }
        st.mp = MpLink::default();
        return;
    }
    if top_box(st, MsgPurpose::Connecting) {
        mp_connecting(st, hud, assets);
        return;
    }
    if top_box(st, MsgPurpose::Joining) && !crate::online::joining() {
        mp_join_answer(st, hud, assets);
    }
    st.mp.poll -= dt;
    if st.mp.poll > 0.0 {
        return;
    }
    st.mp.poll = MP_POLL;
    if matches!(st.stack.last().map(|w| &w.body), Some(Body::Wait(_))) {
        mp_wait_tick(st, hud, assets);
        return;
    }
    if matches!(st.stack.last().map(|w| &w.body), Some(Body::Host(_))) {
        if let Some(e) = st.mp.host_error.take().or_else(crate::online::take_host_error) {
            // Hosting failed: back off the room's screen, with the reason.
            st.pop();
            st.mp.hosting = false;
            let w = msg_box(hud, assets, "Host Room", &e, false, MsgPurpose::Info);
            st.push(w);
            return;
        }
    }
    if let crate::online::Status::Failed(m) = crate::online::status() {
        // Lost: back to the menu under the multiplayer windows, with what went wrong.
        st.stack.retain(|w| !mp_win(w));
        st.mp.hosting = false;
        let w = msg_box(hud, assets, "Multiplayer", &m, false, MsgPurpose::Info);
        st.push(w);
        return;
    }
    let Some(top) = st.stack.last_mut() else { return };
    let multiplayer = top.f.class == "MenuScreenMultiplayer";
    match &mut top.body {
        Body::Menu { notes, .. } if multiplayer && !st.mp.offline => {
            if let Some((_, n)) = notes.first_mut() {
                let t = mp_status_text();
                if *n != t {
                    *n = t;
                }
            }
        }
        Body::Host(h) => mp_poll_host(h),
        Body::Browse(b) => mp_poll_browse(b),
        _ => {}
    }
}

/// "Connecting to online services...": in on Online, the reason on Failed.
fn mp_connecting(st: &mut Menus, hud: &mut ClassicHud, assets: &mut RuntimeAssets) {
    match crate::online::status() {
        crate::online::Status::Online { .. } => {
            st.pop();
            let w = open_window(hud, assets, "MenuScreenMultiplayer");
            st.push(w);
        }
        crate::online::Status::Failed(m) => {
            st.pop();
            let text = format!("{m}|nPlay Offline: rooms by direct code.");
            let mut w = msg_box(hud, assets, "Multiplayer", &text, false, MsgPurpose::Info);
            if let Some(w) = w.as_mut() {
                w.f.actions.push((2, ACT_OFFLINE, "Play Offline".into(), String::new()));
                w.f.actions.push((0, ACT_ADDR, "Server...".into(), String::new()));
            }
            st.push(w);
        }
        _ => {}
    }
}

/// "Looking for room..." answered: what the room is, or why not.
fn mp_join_answer(st: &mut Menus, hud: &mut ClassicHud, assets: &mut RuntimeAssets) {
    let Some(r) = crate::online::take_join() else { return };
    st.pop();
    match r {
        Ok(j) => {
            // Found: choose who to play (a multiplayer character, or a new one) before joining.
            let room = format!(
                "Room {}: {}, {} look, {}/{} players{}.",
                st.mp.join_code,
                mp_mode_name(j.mode),
                mp_look_name(j.look),
                j.players,
                j.max_players,
                if j.tough { ", tougher enemies" } else { "" },
            );
            let versus = j.mode != Mode::Coop;
            st.mp.joined = Some(j);
            if versus {
                // A deathmatch room: in as the deathmatch character (story characters stay out of it).
                mp_dm_join(st, hud, assets);
                return;
            }
            let mut w = open_window(hud, assets, "MenuScreenChooseCharacter");
            if let Some(Body::Chars(c)) = w.as_mut().map(|w| &mut w.body) {
                c.room = room;
            }
            st.push(w);
        }
        Err(m) => {
            let w = msg_box(hud, assets, "Join Room", &m, false, MsgPurpose::MpResult);
            st.push(w);
        }
    }
}

/// Joining a deathmatch room: Augmentique is needed; the last deathmatch character plays (a new one named after
/// the story character when there's none yet; Deathmatch Character edits it).
fn mp_dm_join(st: &mut Menus, hud: &mut ClassicHud, assets: &mut RuntimeAssets) {
    if let Some(why) = crate::dmchar::blocked() {
        let w = msg_box(hud, assets, "Join Room", why, false, MsgPurpose::MpResult);
        st.push(w);
        return;
    }
    let c = crate::dmchar::last().unwrap_or_else(|| {
        let mut c = crate::dmchar::fresh();
        if let Some(n) = crate::travel::with_session(|s| s.name.clone()).filter(|n| !n.trim().is_empty()) {
            c.name = n;
        }
        crate::dmchar::save(&mut c);
        crate::dmchar::set_current(c.clone());
        c
    });
    crate::dmchar::apply(&c);
    crate::netplay::choose_character(crate::dmchar::new_game(&c), None);
    mp_join_room(st, hud, assets);
}

fn mp_mode_name(m: Mode) -> &'static str {
    match m {
        Mode::Coop => "Co-op",
        Mode::Deathmatch => "Deathmatch",
        Mode::TeamDeathmatch => "Team DM",
    }
}

fn mp_look_name(l: Look) -> &'static str {
    match l {
        Look::Classic => "Classic",
        Look::Modern => "Modern",
    }
}

/// The host's screen: the room's code and players, who found it.
fn mp_poll_host(h: &mut MpHost) {
    // Match Settings may have changed under the room: the rules, and the browser's line.
    mp_apply_dm(h);
    static DETAIL: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
    let settings = mp_host_settings(h);
    if let Ok(mut d) = DETAIL.lock() {
        if *d != settings.detail {
            d.clone_from(&settings.detail);
            crate::online::update(settings);
        }
    }
    if let Some(hd) = crate::online::hosted() {
        if h.code.as_deref() != Some(hd.code.as_str()) {
            h.code = Some(hd.code);
        }
        h.players = hd.players;
    }
    h.open = crate::netplay::is_host();
    let direct = crate::netplay::direct_code();
    if direct != h.direct {
        h.direct = direct;
    }
    let note = crate::netplay::direct_note();
    if note != h.note {
        h.note = note;
    }
    let lobby: Vec<(u8, String, bool, u32, bool, bool)> =
        crate::netplay::lobby_players().into_iter().map(|p| (p.id, p.name, p.ready || p.host, p.ping.round().max(0.0) as u32, p.host, p.me)).collect();
    if lobby != h.lobby {
        h.lobby = lobby;
    }
    h.players = h.players.max(h.lobby.len() as u8);
    h.all_ready = crate::netplay::all_ready();
    if h.copied.is_some_and(|t| t.elapsed().as_secs_f32() > 2.0) {
        h.copied = None;
    }
}

/// The browser: the list as online services have it (rows rebuilt only when it changed).
fn mp_poll_browse(b: &mut MpBrowse) {
    let (rooms, state, ms) = crate::online::rooms();
    if rooms != b.rooms || state != b.state || ms != b.list_ms {
        b.rooms = rooms;
        b.state = state;
        b.list_ms = ms;
        mp_browse_rebuild(b);
    }
}

/// The browser's ping for a room: our trip to the lobby and the host's.
fn mp_ping(b: &MpBrowse, r: &RoomInfo) -> u32 {
    b.list_ms + r.host_ms
}

fn mp_passes(b: &MpBrowse, r: &RoomInfo) -> bool {
    let [mode, look, players, ping] = b.filters;
    let p = mp_ping(b, r);
    (mode == 0 || (mode == 1) == (r.mode == Mode::Coop))
        && (look == 0 || (look == 1) == (r.look == Look::Classic))
        && (players == 0 || r.players < r.max_players)
        && match ping {
            1 => p < 100,
            2 => p < 200,
            _ => true,
        }
}

/// The rooms that pass the filters and their rows; the selected room stays selected (by code).
fn mp_browse_rebuild(b: &mut MpBrowse) {
    let keep = b.sel.and_then(|s| b.shown.get(s)).and_then(|&i| b.rooms.get(i)).map(|r| r.code.clone());
    b.shown = (0..b.rooms.len()).filter(|&i| mp_passes(b, &b.rooms[i])).collect();
    b.rows = b
        .shown
        .iter()
        .map(|&i| {
            let r = &b.rooms[i];
            vec![
                r.code.clone(),
                r.name.clone(),
                mp_mode_name(r.mode).into(),
                // Deathmatch rooms are always Modern: their map instead.
                if r.mode == Mode::Coop || r.detail.is_empty() { mp_look_name(r.look).into() } else { r.detail.split('/').next().unwrap_or("").replace('_', " ") },
                format!("{}/{}", r.players, r.max_players),
                if r.tough { "+".into() } else { String::new() },
                format!("{} ms", mp_ping(b, r)),
            ]
        })
        .collect();
    b.sel = keep.and_then(|c| b.shown.iter().position(|&i| b.rooms[i].code == c)).or(if b.shown.is_empty() { None } else { Some(0) });
}

/// A header clicked: its filter steps on.
fn mp_browse_filter(b: &mut MpBrowse, k: usize) {
    if let Some(f) = b.filters.get_mut(k) {
        *f = (*f + 1) % FILTER_STEPS[k];
        mp_browse_rebuild(b);
    }
}

/// Up / Down in the room list (kept in view).
fn mp_browse_move(b: &mut MpBrowse, d: i32) {
    let n = b.shown.len() as i32;
    if n == 0 {
        return;
    }
    let next = (b.sel.map_or(-1, |s| s as i32) + d).clamp(0, n - 1);
    b.sel = Some(next as usize);
    let (y, h) = (next * ROW_H, BROWSE_LIST.3);
    if y < b.scroll {
        b.scroll = y;
    }
    if y + ROW_H > b.scroll + h {
        b.scroll = y + ROW_H - h;
    }
}

/// Join the selected room.
fn mp_browse_join(b: &MpBrowse) -> Option<Act> {
    let r = b.sel.and_then(|s| b.shown.get(s)).and_then(|&i| b.rooms.get(i))?;
    Some(Act::Join(r.code.clone()))
}

/// A header's text: the column, or the filter it holds.
fn mp_header_label(k: usize, v: u8) -> &'static str {
    match (k, v) {
        (0, 1) => "Co-op",
        (0, 2) => "Deathmatch",
        (0, _) => "Mode",
        (1, 1) => "Classic",
        (1, 2) => "Modern",
        (1, _) => "Look",
        (2, 1) => "Not Full",
        (2, _) => "Players",
        (3, 1) => "< 100 ms",
        (3, 2) => "< 200 ms",
        _ => "Ping",
    }
}

/// The multiplayer screen's connection line, in an info box in the well under the buttons.
fn draw_menu_note(u: &mut Ui<W>, x: i32, w: i32, y: i32, text: &str, cx: i32, cy: i32) {
    let (bx, by, bw) = (cx + x + 12, cy + y + 4, w - 24);
    info_button(u, bx, by, bw, "", m_btn(0));
    let f = u.f.small.clone();
    u.c.set_clip(Some((bx + 4, by, bw - 14, 19)));
    u.c.text(&f, bx + 2, by + 1, bw - 12, 18, HAlign::Center, VAlign::Center, m_col(8), text);
    u.c.set_clip(None);
}

/// MenuScreenHostRoom: the choices, the room's box on the right (lobby code, direct code + Copy Code, the router
/// note), and the players panel under them (ready, ping, Kick).
/// The deathmatch character creator: the character being edited, its name field, the list's selection.
struct DmEdit {
    c: crate::dmchar::DmChar,
    name: Edit,
    sel: Option<usize>,
    scroll: i32,
    /// The saved characters (Character row cycles through them and a new one).
    others: Vec<crate::dmchar::DmChar>,
    /// The weapon rows (classes with Augmentique skins).
    weapons: Vec<String>,
}

/// The creator's fixed rows before the weapons.
const DM_ROWS: usize = 5;

impl DmEdit {
    fn new(c: crate::dmchar::DmChar) -> DmEdit {
        let mut name = Edit::new(20);
        name.text = c.name.chars().collect();
        name.cur = name.text.len();
        name.anchor = 0;
        DmEdit { c, name, sel: Some(1), scroll: 0, others: crate::dmchar::list(), weapons: crate::dmchar::skin_weapons() }
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let c = &self.c;
        let mut v = vec![
            vec!["Character".into(), if c.id.is_empty() { "New character".into() } else { "Saved".into() }],
            vec!["Body".into(), if c.female { "Female (Lay D Denton)".into() } else { "Male".into() }],
            vec!["Face".into(), format!("{} of 5", c.skin.min(4) + 1)],
            vec!["Augmentique Outfit".into(), crate::dmchar::outfit_name(c)],
            vec!["Record".into(), format!("{} kills, {} deaths, {} wins", c.total_kills, c.deaths, c.wins)],
        ];
        for w in &self.weapons {
            let skins = crate::dmchar::skins_for(c, w);
            let cur = crate::dmchar::chosen_skin(c, w);
            let name = skins.iter().find(|s| s.0 == cur).map_or_else(|| cur.clone(), |s| s.1.clone());
            let locked = skins.iter().filter(|s| s.2 > 0).count();
            let val = if locked > 0 { format!("{name} ({locked} locked)") } else { name };
            v.push(vec![format!("{} skin", crate::dmchar::weapon_name(w)), val]);
        }
        v
    }
    /// Change the selected row (Enter / click / Right: next; Left: back). A message for a box.
    fn step(&mut self, dir: i32) -> Option<String> {
        let r = self.sel?;
        self.c.name = self.name.get().trim().to_string();
        match r {
            0 => {
                // Saved characters, then a new one.
                let n = self.others.len() as i32 + 1;
                let cur = self.others.iter().position(|o| o.id == self.c.id).map_or(n - 1, |i| i as i32);
                let next = (cur + dir).rem_euclid(n) as usize;
                let c = self.others.get(next).cloned().unwrap_or_else(|| crate::dmchar::DmChar { name: "Agent".into(), outfit: "default".into(), ..Default::default() });
                *self = DmEdit { sel: Some(0), ..DmEdit::new(c) };
            }
            1 => {
                if !self.c.female && !crate::dmchar::female_possible() {
                    return Some("The female body is Lay D Denton's: add that mod on the launcher's Mods page.".into());
                }
                self.c.female = !self.c.female;
                if !crate::augmentique::outfits_for(self.c.female).iter().any(|o| o.1 == self.c.outfit) {
                    self.c.outfit = "default".into();
                }
            }
            2 => self.c.skin = ((self.c.skin as i32 + dir).rem_euclid(5)) as u8,
            3 => crate::dmchar::cycle_outfit(&mut self.c, dir),
            4 => {}
            i => {
                if let Some(w) = self.weapons.get(i - DM_ROWS).cloned() {
                    crate::dmchar::cycle_skin(&mut self.c, &w, dir);
                }
            }
        }
        None
    }
    fn move_sel(&mut self, d: i32) {
        let n = (DM_ROWS + self.weapons.len()) as i32;
        let next = (self.sel.map_or(-1, |s| s as i32) + d).clamp(0, n - 1);
        self.sel = Some(next as usize);
        let (y, h) = (next * ROW_H, 268 - 22);
        if y < self.scroll {
            self.scroll = y;
        }
        if y + ROW_H > self.scroll + h {
            self.scroll = y + ROW_H - h;
        }
    }
    fn help(&self) -> String {
        let c = &self.c;
        match self.sel {
            Some(0) => "Your saved deathmatch characters (click to switch), or a new one. Kills and skins belong to each character.".into(),
            Some(1) => if crate::dmchar::female_possible() { "Male, or Lay D Denton's female JC.".into() } else { "Male. The female body needs the Lay D Denton mod.".into() },
            Some(2) => "Your face: the five new-game faces.".into(),
            Some(3) => "Every Augmentique outfit for this body. The other players see you in it.".into(),
            Some(4) => format!("{} matches played. Kills with each weapon earn its skins.", c.matches),
            Some(i) if i >= DM_ROWS => {
                let Some(w) = self.weapons.get(i - DM_ROWS) else { return String::new() };
                let kills = crate::dmchar::kills_with(c, w);
                let list: Vec<String> = crate::dmchar::skins_for(c, w)
                    .into_iter()
                    .skip(1)
                    .map(|(_, n, need)| if need == 0 { n } else { format!("{n} ({need} more)") })
                    .collect();
                format!("{} kills with the {}. Skins: {}. Others see your skin on your weapon.", kills, crate::dmchar::weapon_name(w), list.join(", "))
            }
            _ => "Name your deathmatch character above.".into(),
        }
    }
}

/// The creator: Name field over the list's headers, the rows.
fn draw_dm_char(u: &mut Ui<W>, d: &mut DmEdit, cx: i32, cy: i32) {
    header_button(u, W::Header(0), cx + 10, cy + 3, 62, "Name");
    u.c.fill(cx + 73, cy + 3, 262, 18, Style::Normal, BLACK);
    edit_field(u, W::Edit(5), cx + 75, cy + 3, 257, &mut d.name, true, true);
    let rows = d.rows();
    list(u, 0, (cx + 11, cy + 23, 369, 268), &[170, 199], &rows, d.sel, &mut d.scroll, false);
}

/// Save Character: named, kept, played from now on.
fn dm_char_save(d: &mut DmEdit) -> Result<(), String> {
    let name = d.name.get().trim().to_string();
    if name.is_empty() {
        return Err("Give your character a name.".into());
    }
    if let Some(who) = crate::netplay::name_taken(&name) {
        return Err(name_taken_text(&who));
    }
    d.c.name = name;
    crate::dmchar::save(&mut d.c);
    crate::dmchar::set_current(d.c.clone());
    Ok(())
}

fn draw_mp_host(u: &mut Ui<W>, h: &MpHost, f: &Frame, drop: &mut Option<(u8, i32)>, cx: i32, cy: i32) {
    draw_choices(u, &h.choices, f, drop, cx, cy);
    draw_mp_host_codes(u, h, cx, cy);
    draw_mp_host_players(u, h, cx, cy);
}

/// A black box with the menu's border colour (as the save screen's snapshot well).
fn mp_box(u: &mut Ui<W>, (bx, by, bw, bh): Rect) {
    u.c.fill(bx, by, bw, bh, Style::Normal, BLACK);
    for (x, y, w, hh) in [(0, 0, bw, 1), (0, bh - 1, bw, 1), (0, 0, 1, bh), (bw - 1, 0, 1, bh)] {
        u.c.fill(bx + x, by + y, w, hh, Style::Normal, m_col(3));
    }
}

/// The Host Room's codes box: the lobby code (with online services), the direct code and its Copy Code button, the
/// note about the router's port.
fn draw_mp_host_codes(u: &mut Ui<W>, h: &MpHost, cx: i32, cy: i32) {
    let (bx, by, bw, bh) = (cx + 394, cy + 22, 136, 210);
    mp_box(u, (bx, by, bw, bh));
    let (fh, fs) = (u.f.headers.clone(), u.f.small.clone());
    let big = u.hud.font(u.assets, "FontMenuExtraLarge").unwrap_or_else(|| u.f.title.clone());
    u.c.set_clip(Some((bx + 1, by + 1, bw - 2, bh - 2)));
    u.c.text(&fh, bx, by + 6, bw, fh.height, HAlign::Center, VAlign::Top, m_col(8), "Room Code:");
    let mut y = by + 20;
    match (&h.code, h.offline) {
        (Some(code), false) => u.c.text(&big, bx, y, bw, big.height, HAlign::Center, VAlign::Top, WHITE, code),
        (None, false) => u.c.text(&fs, bx, y + 6, bw, fs.height, HAlign::Center, VAlign::Top, m_col(8), "Creating room..."),
        (_, true) => u.c.text(&fs, bx, y + 6, bw, fs.height, HAlign::Center, VAlign::Top, m_col(8), "Offline: none"),
    }
    y += big.height + 6;
    u.c.text(&fh, bx, y, bw, fh.height, HAlign::Center, VAlign::Top, m_col(8), "Direct Code:");
    y += fh.height + 2;
    let direct = h.direct.as_deref().unwrap_or("Working it out...");
    u.c.text(&fs, bx, y, bw, fs.height, HAlign::Center, VAlign::Top, if h.direct.is_some() { WHITE } else { m_col(8) }, direct);
    y += fs.height + 4;
    u.c.set_clip(None);
    let copy = if h.copied.is_some() { "Copied!" } else { "Copy Code" };
    action_button(u, W::Header(HDR_COPY), bx + 14, y, bw - 28, copy, h.direct.is_some());
    y += 23;
    u.c.set_clip(Some((bx + 1, by + 1, bw - 2, bh - 2)));
    u.wrapped(&fs, bx + 6, y, bw - 12, by + bh - 4 - y, HAlign::Left, VAlign::Top, m_col(3), &h.note);
    u.c.set_clip(None);
}

/// The Host Room's players panel (over the art's last three rows): name, ready, ping, Kick for joiners.
fn draw_mp_host_players(u: &mut Ui<W>, h: &MpHost, cx: i32, cy: i32) {
    let (px, py, pw, ph) = (cx + 7, cy + 238, 523, 110);
    mp_box(u, (px, py, pw, ph));
    let max = mp_host_settings(h).max_players;
    let head = format!("Players {} / {max}", h.lobby.len().max(1));
    let cols = [(250, head.as_str()), (110, "Status"), (80, "Ping"), (73, "")];
    let mut hx = px + 3;
    for (k, (w, t)) in cols.iter().enumerate() {
        header_button(u, W::Header(60 + k as u8), hx, py + 3, *w, t);
        hx += w;
    }
    let fs = u.f.small.clone();
    let row_h = 15;
    for (r, (_, name, ready, ping, host, me)) in h.lobby.iter().take(HOST_ROWS).enumerate() {
        let ry = py + 21 + r as i32 * row_h;
        let name = if name.trim().is_empty() { "Player" } else { name.as_str() };
        let who = match (*host, *me) {
            (true, true) => format!("{name} (host, you)"),
            (true, false) => format!("{name} (host)"),
            (false, true) => format!("{name} (you)"),
            _ => name.to_string(),
        };
        let col = m_col(8);
        u.c.set_clip(Some((px + 6, ry, 244, row_h)));
        u.c.text(&fs, px + 8, ry, 242, row_h, HAlign::Left, VAlign::Center, col, &who);
        u.c.set_clip(None);
        let state = if *ready { "Ready" } else { "Not ready" };
        u.c.text(&fs, px + 3 + 250 + 5, ry, 105, row_h, HAlign::Left, VAlign::Center, col, state);
        if !*me {
            u.c.text(&fs, px + 3 + 360 + 5, ry, 75, row_h, HAlign::Left, VAlign::Center, col, &format!("{ping} ms"));
        }
        if !*host {
            header_button(u, W::Header(HDR_KICK + r as u8), px + 3 + 440 + 6, ry, 62, "Kick");
        }
    }
    let more = h.lobby.len().saturating_sub(HOST_ROWS);
    let note = if more > 0 {
        format!("and {more} more")
    } else if h.lobby.len() <= 1 {
        "Nobody has joined yet. Share a code: players wait here until you start.".to_string()
    } else {
        String::new()
    };
    if !note.is_empty() {
        let ry = py + 21 + h.lobby.len().min(HOST_ROWS) as i32 * row_h;
        u.c.text(&fs, px + 8, ry, pw - 16, row_h, HAlign::Left, VAlign::Center, m_col(3), &note);
    }
}

/// MenuScreenJoinRoom: "Room Code" and the code box (a lobby code's letters still to type shown as _; a direct code
/// in groups of five).
fn draw_mp_join(u: &mut Ui<W>, j: &MpJoin, cx: i32, cy: i32) {
    let y = cy + 27;
    action_button(u, W::ChoiceBtn(0), cx + 7, y, 243, "|&Room Code", true);
    let mut shown = mp_code_shown(&j.code);
    if j.code.len() < 4 {
        while shown.len() < 4 {
            shown.push('_');
        }
    } else if j.code.len() < DIRECT_LEN {
        shown.push('_');
    }
    info_button(u, cx + 7 + 270, y, 220, &shown, m_btn(1));
    u.hit(W::Edit(3), (cx + 7 + 270, y, 220, 19), true);
}

/// MenuScreenWaitingRoom: who's in the host's lobby and whether they're ready.
fn draw_mp_wait(u: &mut Ui<W>, w: &mut MpWait, cx: i32, cy: i32) {
    header_button(u, W::Header(0), cx + 10, cy + 3, 162, "Player");
    header_button(u, W::Header(1), cx + 175, cy + 3, 157, "Status");
    let (lx, ly, lw, lh) = (cx + 11, cy + 23, 369, 268);
    list(u, 0, (lx, ly, lw, lh), &[164, 110, 95], &w.rows, None, &mut w.scroll, false);
    if w.rows.is_empty() {
        let fs = u.f.small.clone();
        let t = if w.connected { "Waiting for the host..." } else { "Connecting to the host..." };
        u.wrapped(&fs, lx + 10, ly + 10, lw - 20, 40, HAlign::Center, VAlign::Top, m_col(8), t);
    }
}

/// MenuScreenLobbyBrowser: filter headers, the room list, the selected room's card, the list's state.
fn draw_mp_browse(u: &mut Ui<W>, b: &mut MpBrowse, cx: i32, cy: i32) {
    // Headers over the art's two wells: Room (code and host) | Mode | Look, and Players | Ping.
    let c = BROWSE_COLS;
    header_button(u, W::Header(10), cx + 10, cy + 3, c[0] + c[1], "Room");
    let mut hx = cx + 10 + c[0] + c[1];
    for (k, w) in [(0usize, c[2]), (1, c[3] - 2)] {
        header_button(u, W::Header(k as u8), hx, cy + 3, w, mp_header_label(k, b.filters[k]));
        hx += w;
    }
    header_button(u, W::Header(2), cx + 251, cy + 3, c[4] + c[5] + 6, mp_header_label(2, b.filters[2]));
    header_button(u, W::Header(3), cx + 251 + c[4] + c[5] + 6, cy + 3, 131 - (c[4] + c[5] + 6), mp_header_label(3, b.filters[3]));
    let (lx, ly, lw, lh) = BROWSE_LIST;
    list(u, 0, (cx + lx, cy + ly, lw, lh), &c, &b.rows, b.sel, &mut b.scroll, false);
    let fs = u.f.small.clone();
    if b.rows.is_empty() {
        let msg = match &b.state {
            crate::online::ListState::Loading => "Loading...".to_string(),
            crate::online::ListState::Failed(m) => m.clone(),
            _ if !b.rooms.is_empty() => "No public rooms match the filters.".into(),
            _ => "No public rooms".into(),
        };
        u.wrapped(&fs, cx + lx + 10, cy + ly + 10, lw - 20, 40, HAlign::Center, VAlign::Top, m_col(8), &msg);
    }
    // The selected room's card in the snapshot box.
    let (bx, by, bw, bh) = (cx + 387, cy + 24, 160, 120);
    u.c.fill(bx, by, bw, bh, Style::Normal, BLACK);
    for (x, y, w, h) in [(0, 0, bw, 1), (0, bh - 1, bw, 1), (0, 0, 1, bh), (bw - 1, 0, 1, bh)] {
        u.c.fill(bx + x, by + y, w, h, Style::Normal, m_col(3));
    }
    let room = b.sel.and_then(|s| b.shown.get(s)).and_then(|&i| b.rooms.get(i));
    if let Some(r) = room {
        let big = u.hud.font(u.assets, "FontMenuExtraLarge").unwrap_or_else(|| u.f.title.clone());
        let fh = u.f.headers.clone();
        u.c.text(&big, bx, by + 6, bw, big.height, HAlign::Center, VAlign::Top, WHITE, &r.code);
        let mut y = by + 10 + big.height;
        u.c.set_clip(Some((bx + 2, by + 1, bw - 4, bh - 2)));
        u.c.text(&fh, bx, y, bw, fh.height, HAlign::Center, VAlign::Top, m_col(8), &r.name);
        y += fh.height + 3;
        let lines = [
            format!("{}, {} look", mp_mode_name(r.mode), mp_look_name(r.look)),
            r.detail.replace('/', ", ").replace('_', " "),
            format!("{} / {} players", r.players, r.max_players),
            if r.tough { "Tougher enemies".to_string() } else { "Normal enemies".to_string() },
            format!("Ping about {} ms", mp_ping(b, r)),
        ];
        for l in &lines {
            u.c.text(&fs, bx, y, bw, fs.height, HAlign::Center, VAlign::Top, m_col(8), l);
            y += fs.height;
        }
        u.c.set_clip(None);
    }
    // The info well: how many rooms; the check box's well: how the headers work.
    let count = match &b.state {
        crate::online::ListState::Ready => format!("{} public room{}, {} shown.", b.rooms.len(), if b.rooms.len() == 1 { "" } else { "s" }, b.shown.len()),
        crate::online::ListState::Failed(_) => "The list couldn't be fetched.".into(),
        _ => "Asking the lobby for its rooms...".into(),
    };
    u.wrapped(&fs, cx + 390, cy + 166, 155, 60, HAlign::Left, VAlign::Center, m_col(8), &count);
    u.wrapped(&fs, cx + 390, cy + 250, 155, 26, HAlign::Left, VAlign::Center, m_col(8), "Click a header to filter the list.");
}

/// Online Services: "Server Address" and its field.
fn draw_mp_addr(u: &mut Ui<W>, a: &mut MpAddr, cx: i32, cy: i32) {
    let y = cy + 27;
    action_button(u, W::ChoiceBtn(0), cx + 7, y, 243, "|&Server Address", true);
    edit_field(u, W::Edit(4), cx + 7 + 270, y, 240, &mut a.edit, true, true);
}

/// Choose Character: the characters list (New Character first), the selected one's card, the room being joined.
fn draw_mp_chars(u: &mut Ui<W>, c: &mut MpChars, cx: i32, cy: i32) {
    header_button(u, W::Header(10), cx + 10, cy + 3, 238, "Character");
    header_button(u, W::Header(11), cx + 251, cy + 3, 131, "Last Played");
    let (lx, ly, lw, lh) = BROWSE_LIST;
    list(u, 0, (cx + lx, cy + ly, lw, lh), &[240, 131], &c.rows, c.sel, &mut c.scroll, false);
    let (bx, by, bw, bh) = (cx + 387, cy + 24, 160, 120);
    u.c.fill(bx, by, bw, bh, Style::Normal, BLACK);
    for (x, y, w, h) in [(0, 0, bw, 1), (0, bh - 1, bw, 1), (0, 0, 1, bh), (bw - 1, 0, 1, bh)] {
        u.c.fill(bx + x, by + y, w, h, Style::Normal, m_col(3));
    }
    let (fh, fs) = (u.f.headers.clone(), u.f.small.clone());
    let (title, about) = match c.sel {
        Some(0) => ("New Character".to_string(), "Make a new agent with the character creator. Multiplayer characters keep their own saves."),
        Some(i) => (
            c.list.get(i - 1).map(|ch| ch.1.clone()).unwrap_or_default(),
            "Plays on with this character's skills, augmentations and inventory.",
        ),
        None => (String::new(), ""),
    };
    u.c.set_clip(Some((bx + 2, by + 1, bw - 4, bh - 2)));
    u.c.text(&fh, bx, by + 10, bw, fh.height, HAlign::Center, VAlign::Top, WHITE, &title);
    u.wrapped(&fs, bx + 8, by + 18 + fh.height, bw - 16, bh - 26 - fh.height, HAlign::Center, VAlign::Top, m_col(8), about);
    u.c.set_clip(None);
    u.wrapped(&fs, cx + 390, cy + 166, 155, 60, HAlign::Left, VAlign::Center, m_col(8), &c.room);
    u.wrapped(&fs, cx + 390, cy + 250, 155, 26, HAlign::Left, VAlign::Center, m_col(8), "Double-click or Join Game to go in.");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MENU_PREVIEW=<dir> HUD_GAME=<root> cargo test -p dxview preview_menus -- --nocapture
    #[test]
    fn preview_menus() {
        let (Ok(out), Ok(root)) = (std::env::var("MENU_PREVIEW"), std::env::var("HUD_GAME")) else {
            return;
        };
        let root = std::path::PathBuf::from(root);
        // The load screen's character groups: two characters' saves and one from before characters had folders.
        let saves = std::env::temp_dir().join(format!("dxmenu-{}", std::process::id()));
        std::env::set_var("DXVIEW_SAVE_DIR", &saves);
        crate::save::write_for_test("JC_Denton-1", "JC Denton", "Liberty Island - the dock", 1);
        crate::save::write_for_test("JC_Denton-1", "JC Denton", "UNATCO HQ", 2);
        crate::save::write_for_test("Agent_Smith-2", "Agent Smith", "Battery Park", 3);
        crate::save::write_for_test("", "JC Denton", "Old save", 4);
        crate::travel::set_session(crate::travel::NewGame { char_id: "JC_Denton-1".into(), ..Default::default() });
        // Two characters' achievements (and an ending reached by one of them) for the Achievements screen.
        let _ = std::fs::create_dir_all(saves.join("Characters/JC_Denton-1"));
        let _ = std::fs::create_dir_all(saves.join("Characters/Agent_Smith-2"));
        let _ = std::fs::write(saves.join("Characters/JC_Denton-1/achievements.txt"), "format=2\nname=JC Denton\nlady=1793404800\ntown=1793404900\nlebedev=1793405000\nhelios=1793406000\n");
        let _ = std::fs::write(saves.join("Characters/Agent_Smith-2/achievements.txt"), "format=2\nname=Agent Smith\nlady=1793504800\ndunkin=1793504900\n");
        let _ = std::fs::write(saves.join("achievements_global.txt"), "helios=1793406000;JC Denton\n");
        let mut assets = RuntimeAssets {
            classes: unreal_pkg::classes::ClassDb::open(&root.join("System")),
            lib: dx_level::TexLib::new(&root),
            images: Default::default(),
            materials: Default::default(),
            fire: Vec::new(),
            sprites: Default::default(),
            shared_parts: Default::default(),
        };
        *crate::abilities::lock() = Some(crate::abilities::build(&mut assets.classes));
        apply_themes(&mut assets, DEFAULT_MENU_THEME, DEFAULT_HUD_THEME);
        let mut hud = ClassicHud::default();
        let get = |hud: &mut ClassicHud, a: &mut RuntimeAssets, n: &str| hud.font(a, n).unwrap();
        let fonts = Fonts {
            title: get(&mut hud, &mut assets, "FontMenuTitle"),
            small: get(&mut hud, &mut assets, "FontMenuSmall"),
            headers: get(&mut hud, &mut assets, "FontMenuHeaders"),
            fixed: get(&mut hud, &mut assets, "FontFixedWidthSmall"),
        };
        for class in [
            "MenuMain",
            "MenuSelectDifficulty",
            "MenuSettings",
            "MenuScreenNewGame",
            "MenuScreenOptions",
            "MenuScreenDisplay",
            "MenuScreenSound",
            "MenuScreenControls",
            "MenuScreenAdjustColors",
            "MenuScreenBrightness",
            "MenuScreenCustomizeKeys",
            "MenuScreenSaveGame",
            "MenuScreenLoadGame",
            "MenuScreenAchievements",
            "MenuScreenWelcome",
            "KEYPAD",
            "MSG",
        ] {
            let mut win = if class == "KEYPAD" {
                let f = Frame { class: "HUDKeypadWindow".into(), cw: 103, ch: 162, ..default() };
                Win { f, body: Body::Keypad(KeypadWin { blocker: 0, valid: "0451".into(), input: "04".into(), wait: None, instant: false, noted: false }), pos: None, focus: None, hides_below: true, drop: None }
            } else if class == "MSG" {
                let d = assets.classes.defaults("MenuMain");
                msg_box(&mut hud, &mut assets, &pstr(&d, "MessageBoxTitle"), &pstr(&d, "ExitMessage"), true, MsgPurpose::Exit).unwrap()
            } else {
                open_window(&mut hud, &mut assets, class).unwrap()
            };
            if let Body::Keys(k) = &mut win.body {
                k.map = crate::keybinds::ACTIONS.iter().map(|a| a.1).collect();
            }
            if let Body::Choices { .. } = win.body {
                win.focus = Some(W::ChoiceBtn(0));
            }
            if class == "MenuScreenDisplay" {
                win.drop = Some((1, 0));
            }
            if let Body::Menu { .. } = win.body {
                win.focus = Some(W::MenuBtn(1));
            }
            render(&mut hud, &mut assets, &fonts, &mut win, &format!("{out}/menu_{class}.rgba"));
        }
        preview_multiplayer(&out, &mut hud, &mut assets, &fonts);
    }

    /// One window centred on a 640 x 480 frame, written as raw RGBA.
    fn render(hud: &mut ClassicHud, assets: &mut RuntimeAssets, fonts: &Fonts, win: &mut Win, path: &str) {
        let (ww, wh) = win_size(&win.f, fonts);
        let mut full = Canvas::new(640, 480, 1);
        full.fill(0, 0, 640, 480, Style::Normal, [20, 30, 40]);
        let mut hits = vec![];
        let pos = (320 - ww / 2, 240 - wh / 2);
        let help = match (&win.body, win.focus) {
            (Body::Choices { choices }, _) => choices[0].help.clone(),
            _ => help_text(win, assets),
        };
        let mut ui = Ui { c: Canvas::new(ww, wh, 1), hud, assets, f: fonts, o: pos, hits: &mut hits, pressed: None, hover: None, focus: win.focus };
        draw_win(&mut ui, win, fonts, &help, false);
        let Ui { c, .. } = ui;
        for y in 0..c.h {
            for x in 0..c.w {
                let (dx, dy) = (x + pos.0, y + pos.1);
                if dx < 0 || dy < 0 || dx >= 640 || dy >= 480 {
                    continue;
                }
                let sp = c.px[(y * c.w + x) as usize];
                let d = &mut full.px[(dy * 640 + dx) as usize];
                for k in 0..3 {
                    d[k] = (sp[k] + d[k] * (1.0 - sp[3])).min(1.0);
                }
            }
        }
        std::fs::write(path, full.to_rgba8()).unwrap();
        println!("wrote {path} {ww}x{wh}");
    }

    /// The multiplayer screens against a real lobby on this PC (imsim-net's server): connect, host a room (a guest
    /// finds it), the browser with other hosts' rooms, the join screen, a join's answer, leaving.
    fn preview_multiplayer(out: &str, hud: &mut ClassicHud, assets: &mut RuntimeAssets, fonts: &Fonts) {
        use imsim_net::client::Client;
        use std::time::{Duration, Instant};
        let port = 41000 + (std::process::id() % 2000) as u16;
        std::thread::spawn(move || {
            let cfg = imsim_net::server::Config { port, admin_port: port + 1, name: "Local Test Lobby".into(), log: None, admin_key: "preview-key-0123456789".into() };
            let _ = imsim_net::server::run(cfg);
        });
        std::thread::sleep(Duration::from_millis(300));
        let addr = format!("127.0.0.1:{port}");
        std::env::set_var("DXVIEW_LOBBY", &addr);
        // Online Services writes this copy, never the player's launcher.txt.
        let launcher = std::env::temp_dir().join(format!("dxmenu-launcher-{}.txt", std::process::id()));
        std::fs::write(&launcher, "GamePath=C:\\Games\\Deus Ex\r\nLobbyServer=old.example:1\r\nModPath=D:\\Mods\r\n").unwrap();
        std::env::set_var("DXVIEW_LAUNCHER_TXT", &launcher);
        for (typed, want) in [
            ("lobby.example.net", "lobby.example.net:27962"),
            (" 10.0.0.5:4000 ", "10.0.0.5:4000"),
            ("10.0.0.5:", "10.0.0.5:27962"),
            ("::1", "[::1]:27962"),
            ("[::1]:5000", "[::1]:5000"),
            ("", ""),
        ] {
            assert_eq!(mp_addr_normalize(typed), want, "{typed:?}");
        }
        let mut st = Menus::default();
        let shot = |st: &mut Menus, hud: &mut ClassicHud, assets: &mut RuntimeAssets, name: &str| {
            let w = st.stack.last_mut().unwrap();
            render(hud, assets, fonts, w, &format!("{out}/menu_{name}.rgba"));
        };
        // Tick until `done` (or 8 s).
        let wait = |st: &mut Menus, hud: &mut ClassicHud, assets: &mut RuntimeAssets, what: &str, done: &dyn Fn(&Menus) -> bool| {
            let t = Instant::now();
            while !done(st) {
                assert!(t.elapsed() < Duration::from_secs(8), "multiplayer preview: timed out waiting for {what}");
                std::thread::sleep(Duration::from_millis(20));
                mp_tick(st, 0.02, hud, assets);
            }
        };
        let top_class = |st: &Menus| st.stack.last().map(|w| w.f.class.clone()).unwrap_or_default();
        // Main menu (Multiplayer + Achievements), then Multiplayer: the connecting box, the screen.
        st.stack.push(open_window(hud, assets, "MenuMain").unwrap());
        mp_connect(&mut st, hud, assets);
        shot(&mut st, hud, assets, "MP_Connecting");
        wait(&mut st, hud, assets, "online", &|st| top_class(st) == "MenuScreenMultiplayer");
        shot(&mut st, hud, assets, "MenuScreenMultiplayer");
        // Other players' rooms for the browser.
        let mut others = Vec::new();
        for (name, mode, look, max, tough, public, players) in [
            ("Paul Denton", Mode::Coop, Look::Classic, 4, true, true, 2),
            ("Gunther", Mode::Deathmatch, Look::Modern, 16, false, true, 16),
            ("Anna Navarre", Mode::Deathmatch, Look::Classic, 8, false, true, 3),
            ("Secret Agent", Mode::Coop, Look::Modern, 2, false, false, 1),
        ] {
            let mut c = Client::connect(&addr, Duration::from_secs(2)).unwrap();
            let s = RoomSettings { mode, public, max_players: max, look, tough, name: name.into(), detail: String::new() };
            let (code, token) = c.host(&s, 7790, &"ab".repeat(32), None).unwrap();
            c.update(&token, players, &s).unwrap();
            others.push((c, code));
        }
        // Host a room; a guest looks it up.
        let mut w = open_window(hud, assets, "MenuScreenHostRoom").unwrap();
        mp_opened(&mut st.mp, &mut w);
        st.push(Some(w));
        wait(&mut st, hud, assets, "room code", &|st| matches!(st.stack.last().map(|w| &w.body), Some(Body::Host(h)) if h.code.is_some()));
        let code = match &st.stack.last().unwrap().body {
            Body::Host(h) => h.code.clone().unwrap(),
            _ => unreachable!(),
        };
        assert!(imsim_net::valid_code(&code));
        let mut guest = Client::connect(&addr, Duration::from_secs(2)).unwrap();
        let j = guest.join(&code).unwrap();
        assert_eq!(j.max_players, RoomSettings::default().max_players);
        // Game Mode -> Deathmatch, Players 4 -> 6: the lobby hears of each.
        if let Some(Body::Host(h)) = st.stack.last_mut().map(|w| &mut w.body) {
            assert!(mp_host_cycle(h, 0, 1).is_none());
            assert!(mp_host_cycle(h, 2, 1).is_none());
            assert!(mp_host_cycle(h, 2, 1).is_none());
        }
        // The game session is up; the direct code (and its router note) can take a moment.
        wait(&mut st, hud, assets, "the room's session", &|st| matches!(st.stack.last().map(|w| &w.body), Some(Body::Host(h)) if h.open));
        mp_wait_direct(&mut st, hud, assets);
        shot(&mut st, hud, assets, "MenuScreenHostRoom");
        // The players panel with joiners (one not ready: Start Game waits) and Copy Code's feedback.
        if let Some(Body::Host(h)) = st.stack.last_mut().map(|w| &mut w.body) {
            h.lobby.push((1, "Paul Denton".into(), true, 42, false, false));
            h.lobby.push((2, "Anna Navarre".into(), false, 118, false, false));
            h.all_ready = false;
            h.copied = Some(Instant::now());
        }
        shot(&mut st, hud, assets, "MP_HostPlayers");
        {
            let w = st.stack.last().unwrap();
            assert!(!action_enabled(w, ACT_START, ""));
        }
        if let Some(Body::Host(h)) = st.stack.last_mut().map(|w| &mut w.body) {
            h.lobby.truncate(1);
            h.all_ready = true;
            h.copied = None;
        }
        // Start Game: the host's game screen (saves, New Character, and Play On with a game up).
        {
            let mut acts = Vec::new();
            let w = st.stack.last_mut().unwrap();
            mp_action(w, ACT_START, &mut acts);
            assert!(matches!(acts.as_slice(), [Act::Push(c)] if c == "MenuScreenLoadGame:mp"));
            let m = MISSION.swap(-2, Ordering::Relaxed);
            let mut acts = Vec::new();
            mp_action(st.stack.last_mut().unwrap(), ACT_START, &mut acts);
            MISSION.store(m, Ordering::Relaxed);
            assert!(matches!(acts.as_slice(), [Act::Push(c)] if c == "MenuScreenLoadGame:mp"));
            let mut w = open_window(hud, assets, "MenuScreenLoadGame").unwrap();
            mp_variant(&mut w);
            assert!(w.f.actions.iter().any(|a| a.1 == ACT_MP_NEWGAME));
            st.stack.push(w);
            shot(&mut st, hud, assets, "MP_StartLoadGame");
            st.stack.pop();
        }
        let rooms = Client::connect(&addr, Duration::from_secs(2)).unwrap().list().unwrap();
        let mine = rooms.iter().find(|r| r.code == code).expect("our room is listed");
        assert_eq!((mine.mode, mine.max_players), (Mode::Deathmatch, 6));
        // The browser (on top of the host's screen: the room stays open).
        let mut w = open_window(hud, assets, "MenuScreenLobbyBrowser").unwrap();
        mp_opened(&mut st.mp, &mut w);
        st.push(Some(w));
        wait(&mut st, hud, assets, "the room list", &|st| matches!(st.stack.last().map(|w| &w.body), Some(Body::Browse(b)) if b.state == crate::online::ListState::Ready));
        if let Some(Body::Browse(b)) = st.stack.last().map(|w| &w.body) {
            // Four public rooms (ours included), the private one left out.
            assert_eq!(b.rooms.len(), 4, "{:?}", b.rooms);
        }
        shot(&mut st, hud, assets, "MenuScreenLobbyBrowser");
        if let Some(Body::Browse(b)) = st.stack.last_mut().map(|w| &mut w.body) {
            mp_browse_filter(b, 0);
            mp_browse_filter(b, 2);
            assert!(b.shown.iter().all(|&i| b.rooms[i].mode == Mode::Coop && b.rooms[i].players < b.rooms[i].max_players));
            mp_browse_move(b, 1);
        }
        shot(&mut st, hud, assets, "MP_BrowserFiltered");
        // The join screen with a code half typed (lower case and look-alikes typed: kept upper case / dropped).
        st.stack.pop();
        let mut w = open_window(hud, assets, "MenuScreenJoinRoom").unwrap();
        mp_opened(&mut st.mp, &mut w);
        if let Body::Join(j) = &mut w.body {
            for t in ["d", "x", "0", "2"] {
                mp_join_typing(j, &KeyCode::KeyA, &Some(t.into()), false);
            }
            assert_eq!(j.code, "DX2");
            assert!(mp_join_act(&j.code).is_none());
            mp_join_typing(j, &KeyCode::KeyA, &Some("3".into()), false);
            assert!(matches!(mp_join_act(&j.code), Some(Act::Join(c)) if c == "DX23"));
        }
        st.push(Some(w));
        shot(&mut st, hud, assets, "MenuScreenJoinRoom");
        // Joining the private room by its code: Choose Character (one multiplayer character saved), New Character,
        // in; then a code that isn't a room.
        let _ = std::fs::create_dir_all(crate::save::mp_dir());
        let _ = std::fs::copy(crate::save::char_dir("JC_Denton-1").join("Save0002.ron"), crate::save::mp_path("JC_Denton-1"));
        let private = others[3].1.clone();
        mp_join(&mut st, hud, assets, private.clone());
        shot(&mut st, hud, assets, "MP_Joining");
        wait(&mut st, hud, assets, "the join answer", &|st| matches!(st.stack.last().map(|w| &w.body), Some(Body::Chars(_))));
        assert!(st.mp.joined.is_some());
        if let Some(Body::Chars(c)) = st.stack.last().map(|w| &w.body) {
            assert_eq!(c.list.len(), 1, "{:?}", c.list);
            assert!(matches!(mp_chars_go(c), Some(Act::MpLoadChar(id)) if id == "JC_Denton-1"));
        }
        shot(&mut st, hud, assets, "MenuScreenChooseCharacter");
        let mut w = open_window(hud, assets, "MenuScreenNewGame").unwrap();
        mp_variant(&mut w);
        st.stack.push(w);
        shot(&mut st, hud, assets, "MP_NewCharacter");
        mp_join_room(&mut st, hud, assets);
        assert!(!st.stack.iter().any(|w| matches!(&w.body, Body::Chars(_)) || matches!(&w.body, Body::NewGame(ng) if ng.multiplayer)));
        // Connected (to a host that never answers here): the waiting room, then as it looks in the host's lobby.
        assert!(matches!(st.stack.last().map(|w| &w.body), Some(Body::Wait(_))));
        assert!(st.mp.waiting);
        shot(&mut st, hud, assets, "MP_WaitingConnecting");
        if let Some(Body::Wait(w)) = st.stack.last_mut().map(|w| &mut w.body) {
            w.connected = true;
            w.rows = vec![
                vec!["Gunther (host)".into(), "Ready".into(), "38 ms".into()],
                vec!["Paul Denton".into(), "Ready".into(), "61 ms".into()],
                vec!["JC Denton (you)".into(), "Not ready".into(), String::new()],
            ];
        }
        shot(&mut st, hud, assets, "MenuScreenWaitingRoom");
        // In a session the main menu offers Leave Multiplayer.
        if crate::netplay::active() {
            let mut w = open_window(hud, assets, "MenuMain").unwrap();
            if let Body::Menu { buttons, .. } = &w.body {
                assert_eq!(buttons[8].label, "Leave Multiplayer");
            }
            w.focus = Some(W::MenuBtn(8));
            st.stack.push(w);
            shot(&mut st, hud, assets, "MP_MainLeave");
            st.stack.pop();
        }
        // Leave on the waiting room: out of the session.
        {
            let mut acts = Vec::new();
            mp_action(st.stack.last_mut().unwrap(), ACT_LEAVE, &mut acts);
            assert!(matches!(acts.as_slice(), [Act::Pop]));
            st.stack.pop();
            mp_tick(&mut st, 0.02, hud, assets);
            assert!(!st.mp.waiting && !crate::netplay::active());
        }
        st.stack.retain(|w| !matches!(&w.body, Body::Msg(_)));
        let bad = if private == "ZZZZ" { "YYYY" } else { "ZZZZ" };
        mp_join(&mut st, hud, assets, bad.into());
        wait(&mut st, hud, assets, "the failed join", &|st| top_box(st, MsgPurpose::MpResult));
        shot(&mut st, hud, assets, "MP_JoinFailed");
        // (The join started a game session toward the fake host: end it, it would keep the lobby connected.)
        crate::netplay::leave("");
        // Leaving: the room closes with its screen, the connection with the last multiplayer window.
        st.stack.retain(|w| !matches!(w.body, Body::Host(_) | Body::Join(_) | Body::Msg(_)));
        mp_tick(&mut st, 0.02, hud, assets);
        assert!(!st.mp.hosting);
        let t = Instant::now();
        while Client::connect(&addr, Duration::from_secs(2)).unwrap().join(&code).is_ok() {
            assert!(t.elapsed() < Duration::from_secs(4), "the room closes with its screen");
            std::thread::sleep(Duration::from_millis(50));
        }
        st.stack.retain(|w| w.f.class == "MenuMain");
        mp_tick(&mut st, 0.02, hud, assets);
        assert_eq!(crate::online::status(), crate::online::Status::Offline);
        assert!(!st.mp.online);
        // Connecting to nothing: the reason, then offline.
        std::env::set_var("DXVIEW_LOBBY", "127.0.0.1:1");
        mp_connect(&mut st, hud, assets);
        wait(&mut st, hud, assets, "the failure", &|st| top_box(st, MsgPurpose::Info));
        shot(&mut st, hud, assets, "MP_ConnectFailed");
        // Change Address on that box: the Online Services screen, then saving there rewrites only LobbyServer= and
        // connects again (still nothing there: the reason again).
        let (i, _) = st.stack.last().unwrap().f.actions.iter().enumerate().find(|(_, a)| a.1 == ACT_ADDR).expect("Server...");
        let mut acts = Vec::new();
        let top = st.stack.len() - 1;
        activate(&mut st, top, W::Act(i as u8), false, hud, assets, &mut acts);
        assert!(matches!(acts.as_slice(), [Act::Pop, Act::Push(c)] if c == "MenuScreenOnlineServices"));
        st.stack.pop();
        let mut w = open_window(hud, assets, "MenuScreenOnlineServices").unwrap();
        mp_opened(&mut st.mp, &mut w);
        if let Body::Addr(a) = &mut w.body {
            assert_eq!(a.edit.get(), "127.0.0.1:1");
            a.edit.clear();
            for t in ["1", "0", ".", "0", ".", "0", ".", "5", " ", "!"] {
                a.edit.key(KeyCode::KeyA, mp_addr_typed(Some(t)).as_deref(), false, false);
            }
            assert_eq!(a.edit.get(), "10.0.0.5");
        }
        st.stack.push(w);
        shot(&mut st, hud, assets, "MenuScreenOnlineServices");
        let typed = match &st.stack.last().unwrap().body {
            Body::Addr(a) => a.edit.get(),
            _ => unreachable!(),
        };
        mp_set_addr(&mut st, hud, assets, &typed);
        assert_eq!(std::fs::read_to_string(&launcher).unwrap(), "GamePath=C:\\Games\\Deus Ex\r\nLobbyServer=10.0.0.5:27962\r\nModPath=D:\\Mods\r\n");
        assert!(!st.stack.iter().any(|w| matches!(w.body, Body::Addr(_))));
        assert!(top_box(&st, MsgPurpose::Connecting));
        wait(&mut st, hud, assets, "the failure again", &|st| top_box(st, MsgPurpose::Info));
        st.stack.pop();
        mp_tick(&mut st, 0.02, hud, assets);
        assert!(!st.mp.online);
        // An empty address takes the line out (this PC); other lines stay.
        crate::online::set_server_addr("").unwrap();
        assert_eq!(std::fs::read_to_string(&launcher).unwrap(), "GamePath=C:\\Games\\Deus Ex\r\nModPath=D:\\Mods\r\n");
        let _ = std::fs::remove_file(&launcher);
        drop(others);
        preview_offline(out, &mut st, hud, assets, fonts);
    }

    /// Wait (at most 10 s, no failure) for the Host Room's direct code.
    fn mp_wait_direct(st: &mut Menus, hud: &mut ClassicHud, assets: &mut RuntimeAssets) {
        let t = std::time::Instant::now();
        while t.elapsed() < std::time::Duration::from_secs(10) {
            if matches!(st.stack.last().map(|w| &w.body), Some(Body::Host(h)) if h.direct.is_some()) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
            st.mp.poll = 0.0;
            mp_tick(st, 0.02, hud, assets);
        }
        println!("multiplayer preview: no direct code after 10 s");
    }

    /// Without online services: Play Offline on the connect-failed box, the Multiplayer screen without the browser,
    /// a Host Room with only the direct code, Join Room taking that code (and refusing a lobby code).
    fn preview_offline(out: &str, st: &mut Menus, hud: &mut ClassicHud, assets: &mut RuntimeAssets, fonts: &Fonts) {
        let shot = |st: &mut Menus, hud: &mut ClassicHud, assets: &mut RuntimeAssets, name: &str| {
            let w = st.stack.last_mut().unwrap();
            render(hud, assets, fonts, w, &format!("{out}/menu_{name}.rgba"));
        };
        std::env::set_var("DXVIEW_LOBBY", "127.0.0.1:1");
        mp_connect(st, hud, assets);
        let t = std::time::Instant::now();
        while !top_box(st, MsgPurpose::Info) {
            assert!(t.elapsed() < std::time::Duration::from_secs(8), "offline preview: no failure box");
            std::thread::sleep(std::time::Duration::from_millis(20));
            mp_tick(st, 0.02, hud, assets);
        }
        let (i, _) = st.stack.last().unwrap().f.actions.iter().enumerate().find(|(_, a)| a.1 == ACT_OFFLINE).expect("Play Offline");
        let mut acts = Vec::new();
        let top = st.stack.len() - 1;
        activate(st, top, W::Act(i as u8), false, hud, assets, &mut acts);
        assert!(matches!(acts.as_slice(), [Act::Pop, Act::MpOffline]));
        st.stack.pop();
        mp_offline(st, hud, assets);
        assert!(st.mp.offline);
        if let Some(Body::Menu { buttons, .. }) = st.stack.last().map(|w| &w.body) {
            assert!(buttons.iter().any(|b| b.invoke == "MenuScreenLobbyBrowser" && !b.enabled));
        }
        shot(st, hud, assets, "MP_Offline");
        // Host Room offline: the game socket alone.
        let mut w = open_window(hud, assets, "MenuScreenHostRoom").unwrap();
        mp_opened(&mut st.mp, &mut w);
        st.push(Some(w));
        assert!(st.mp.host_error.is_none(), "{:?}", st.mp.host_error);
        st.mp.poll = 0.0;
        mp_tick(st, 0.02, hud, assets);
        assert!(matches!(st.stack.last().map(|w| &w.body), Some(Body::Host(h)) if h.open && h.offline));
        mp_wait_direct(st, hud, assets);
        shot(st, hud, assets, "MP_HostOffline");
        let direct = match &st.stack.last().unwrap().body {
            Body::Host(h) => h.direct.clone(),
            _ => None,
        };
        // Join Room offline: the direct code goes to Choose Character, a lobby code is refused.
        let mut w = open_window(hud, assets, "MenuScreenJoinRoom").unwrap();
        mp_opened(&mut st.mp, &mut w);
        if let (Body::Join(j), Some(code)) = (&mut w.body, direct.as_ref()) {
            mp_join_typing(j, &KeyCode::KeyA, &Some(code.to_ascii_lowercase()), false);
            assert_eq!(mp_code_shown(&j.code), *code);
        }
        st.push(Some(w));
        shot(st, hud, assets, "MP_JoinDirect");
        if let Some(code) = direct {
            let act = match &st.stack.last().unwrap().body {
                Body::Join(j) => mp_join_act(&j.code),
                _ => None,
            };
            assert!(matches!(&act, Some(Act::MpDirect(c)) if *c == code));
            mp_direct(st, hud, assets, code.clone());
            assert!(matches!(st.stack.last().map(|w| &w.body), Some(Body::Chars(c)) if c.room.contains(&code)));
            assert_eq!(st.mp.direct.as_deref(), Some(code.as_str()));
            shot(st, hud, assets, "MP_ChooseDirect");
            st.stack.pop();
        }
        mp_join(st, hud, assets, "DX23".into());
        assert!(top_box(st, MsgPurpose::MpResult));
        shot(st, hud, assets, "MP_JoinOfflineCode");
        // Leaving: the room and the session close, nothing stays connected.
        st.stack.retain(|w| w.f.class == "MenuMain");
        mp_tick(st, 0.02, hud, assets);
        assert!(!crate::netplay::active());
        assert!(!st.mp.online && !st.mp.offline);
    }
}

/// A new map is going in (loader.rs): this module's state of the map being left.
pub fn reset_level() {
    if OPEN.swap(false, std::sync::atomic::Ordering::Relaxed) {
        CLOSED_BY_MAP.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    DEAD.store(false, std::sync::atomic::Ordering::Relaxed);
    if let Ok(mut v) = KEYPAD_OPEN.lock() {
        *v = None;
    }
    if let Ok(mut v) = KEYPAD_RESULTS.lock() {
        v.clear();
    }
}
