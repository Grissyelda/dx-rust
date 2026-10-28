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
//! Multiplayer (not planned), render device and colour depth (fixed by the renderer).

use crate::classic_hud::{ClassicHud, HudLayer};
use crate::inventory_screen::{inside, label};
use crate::keybinds::Bindings;
use crate::menuui::*;
use crate::menuui::Rect;
use crate::uicanvas::{wrap, Canvas, HAlign, Rgb, Style, VAlign};
use crate::weapons::RuntimeAssets;
use crate::{FlyCam, LevelRes, SkyCam};
use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::window::{CursorOptions, MonitorSelection, WindowMode};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use unreal_pkg::props::{find, Property, Value};
use unreal_pkg::Reader;

static OPEN: AtomicBool = AtomicBool::new(false);
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
    let v = crate::settings::get("HUDScale");
    Choice {
        key: "HUDScale".into(),
        label: "|&HUD Scale".into(),
        help: "Size of the HUD and in-game screens: Medium is the original's (the nearest whole multiple of 600 lines).".into(),
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
    let v = crate::settings::get("DisplayMode");
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
    c.value = crate::settings::get("VSync");
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

/// The Bevy window mode for a DisplayMode setting.
/// The window mode for a DisplayMode setting. The primary monitor, not "current": at startup there is no window
/// yet, so winit has no current monitor and Bevy panics ("Unable to get monitor").
pub fn window_mode(v: f32) -> WindowMode {
    match v.round() as i32 {
        0 => WindowMode::Fullscreen(MonitorSelection::Primary, bevy::window::VideoModeSelection::Current),
        2 => WindowMode::BorderlessFullscreen(MonitorSelection::Primary),
        _ => WindowMode::Windowed,
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
    /// Ours: starting a new character overwrites the shared autosave slots (st.box_new_game).
    AutosaveWarn,
    /// root.MessageBox asked for by another screen (the persona screen's Delete Note?): the answer goes back.
    External,
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
struct SaveLoad {
    saving: bool,
    games: Vec<crate::save::SaveInfo>,
    /// Selected row: on the save screen row 0 is "New Save Game", then the games.
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
            games: crate::save::list(),
            sel: None,
            scroll: 0,
            edit: Edit::new(64),
            confirm_delete: crate::settings::get("ConfirmSaveDeletes") < 0.5,
            snap: None,
        };
        s.sel = if saving || !s.games.is_empty() { Some(0) } else { None };
        s
    }
    /// The game a row stands for.
    fn game(&self, row: usize) -> Option<&crate::save::SaveInfo> {
        let i = if self.saving { row.checked_sub(1)? } else { row };
        self.games.get(i)
    }
    fn rows(&self) -> usize {
        self.games.len() + self.saving as usize
    }
}

struct Msg {
    text: String,
    yes_no: bool,
    purpose: MsgPurpose,
}

enum Body {
    Menu { buttons: Vec<MenuBtn>, x: i32, w: i32 },
    Choices { choices: Vec<Choice> },
    NewGame(NewGame),
    Keys(Keys),
    Msg(Msg),
    SaveLoad(SaveLoad),
    Keypad(KeypadWin),
    Enhance(Enhance),
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
}

/// A row of the Enhancements list: a group's header (click / Enter opens or closes it) or one setting.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Row {
    Group(usize),
    Item(usize),
}

/// The group of a "Group: Label" item, and the label without it.
fn split_group(label: &str) -> (&str, &str) {
    label.split_once(": ").unwrap_or(("Other", label))
}

impl Enhance {
    fn new() -> Enhance {
        let mut items = Vec::new();
        let tag = |mut c: Choice, group: &str| {
            c.label = format!("{group}: {}", label(&c.label));
            c
        };
        for c in enhancement_choices() {
            if c.key == "PadOptions" || c.key == "PadDualSense" || c.key == "DebugMode" {
                continue;
            }
            items.push(tag(c, "Controller"));
        }
        items.extend(pad_option_choices().into_iter().map(|c| tag(c, "Controller")));
        items.extend(dualsense_choices().into_iter().map(|c| tag(c, "DualSense")));
        for (key, group, lbl, help) in [
            ("NPCFiniteAmmo", "Gameplay", "NPC Ammo Runs Out", "Enemies use up their ammo, reload every clip and switch weapons (or to melee) when they run dry, as the original scripts say."),
            ("FloorConsumables", "Gameplay", "Use Consumables From The Floor", "With no room in your inventory, a medkit, bioelectric cell, food or drink you pick up is used on the spot."),
            ("LadderGrab", "Gameplay", "Grab Ladders", "Use takes hold of a ladder and Jump lets go; on it, forward and back climb up and down."),
            ("NotesAutofill", "Gameplay", "Codes From Notes", "Keypad codes and computer logins you've noted are offered: Tab types a keypad code, logins are filled in."),
            ("DialogueHotkeys", "Interface", "Dialogue Hotkeys", "Number the first four conversation choices; keys 1-4 pick them."),
            ("InventorySort", "Interface", "Inventory Auto-Sort", "A Sort button on the inventory screen packs the items neatly, weapons first."),
            ("PersistentDebris", "Visuals", "Casings And Blood Stay", "Shell casings never vanish and many more blood splats stay on the ground."),
            ("DiscordPresence", "Interface", "Discord Rich Presence", "Show your location, mission and difficulty in Discord."),
        ] {
            let mut c = debug_mode_choice();
            c.key = key.into();
            c.label = lbl.into();
            c.help = help.into();
            c.value = crate::settings::get(key);
            c.saved = c.value;
            items.push(tag(c, group));
        }
        let mut lip = debug_mode_choice();
        lip.key = "SmoothLipSync".into();
        lip.label = "Smooth |&Lip Sync".into();
        lip.help = "Mouths blend smoothly between sounds and close gently, and people blink visibly (after Joe Wintergreen's lipsync mod).".into();
        lip.value = crate::settings::get("SmoothLipSync");
        lip.saved = lip.value;
        items.push(tag(lip, "Visuals"));
        items.push(tag(debug_mode_choice(), "Developer"));
        Enhance { items, search: Edit::new(40), sel: Some(Row::Group(0)), scroll: 0, open: Default::default() }
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
        let mut rows = Vec::new();
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
    if code.is_empty() || crate::settings::get("NotesAutofill") < 0.5 {
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
                // UpdateButtonStatus: no saving or "back to game" on the logo map (or dead); no multiplayer here.
                let dead = DEAD.load(Ordering::Relaxed);
                for (i, b) in buttons.iter_mut().enumerate() {
                    if (i == 1 || i == 7) && (intro_map() || dead) {
                        b.enabled = false;
                    }
                    if i == 8 {
                        b.enabled = false;
                    }
                }
            }
            Body::Menu { buttons, x, w }
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
        e("PadLookSensitivity", "Controller |&Look Speed", "How fast the right stick turns the view.", &["Very Slow", "Slow", "Normal", "Fast", "Very Fast"]),
        e("PadInvertLook", "|&Invert Controller Look", "Push the right stick up to look down.", &["Disabled", "Enabled"]),
        e("PadDeadzone", "Stick |&Deadzone", "How far a stick moves before it counts.", &["Very Small", "Small", "Normal", "Large", "Very Large"]),
        e("PadAimSlowdown", "|&Aim Slowdown", "The right stick turns more slowly while the crosshair is on an enemy.", &["Disabled", "Enabled"]),
        e("PadAugPages", "Aug |&Hotkey Pages", "The D-pad toggles pages of three augmentations; tap the wheel button to turn the page, hold it for the wheel.", &["Disabled", "Enabled"]),
        e("PadHints", "Button |&Hints", "A bar at the bottom of menus and screens showing what the controller's buttons do there.", &["Disabled", "Enabled"]),
        e("PadFrobPrompt", "Button Prompts on |&Objects", "Show the Use button beside the name of what you're looking at (not in the original).", &["Disabled", "Enabled"]),
    ]
}

/// Ours: the DualSense screen (ROADMAP Phase 0.3), under Enhancements.
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
    let v = crate::settings::get("DebugMode");
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
    for (k, ch) in lines.chars().enumerate() {
        let one = ch.to_string();
        let gx = tx + font.width(&lines.chars().take(k).collect::<String>());
        if let Some((page, g)) = font.font.glyph(ch) {
            if let Some(tex) = font.pages.get(page) {
                let tex = tex.clone();
                u.c.texture(gx, ty, g.w, g.h, g.u, g.v, &tex, Style::Translucent, col);
            }
        }
        let _ = one;
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

#[allow(clippy::too_many_arguments)]
fn draw_win(u: &mut Ui<W>, win: &mut Win, fonts: &Fonts, help_text: &str, waiting_keys: bool) {
    let f = win.f.clone();
    let (so, _) = f.shadow.unwrap_or(((0, 0), (0, 0)));
    let (ox, oy) = (so.0, so.1);
    let (ww, wh) = win_size(&f, fonts);
    u.hit(W::Win, (0, 0, ww, wh), true);
    if let Body::Keypad(k) = &win.body {
        draw_keypad(u, k, fonts);
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
    // With a cut, the art above it is drawn in place and the art below it moved up by its height.
    // Cut bands are in the art's own coordinates; each later piece moves up by everything cut above it.
    let mut bands: Vec<(i32, i32, i32)> = Vec::new();
    {
        let (mut src, mut shift) = (0, 0);
        for &(y, h) in &f.cuts {
            if y > src {
                bands.push((src - shift, y - src, shift));
            }
            src = y + h;
            shift += h;
        }
        bands.push((src - shift, f.ch - (src - shift), shift));
    }
    for (by, bh, shift) in bands {
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
        u.wrapped(&fs, hx + 10, hy, hw - 20, hh, HAlign::Left, VAlign::Center, m_col(7), help_text);
        u.c.set_clip(None);
    }
}

fn action_enabled(win: &Win, action: u8, key: &str) -> bool {
    match &win.body {
        Body::NewGame(ng) if action == 5 && key == "START" => !ng.name.text.is_empty(),
        Body::SaveLoad(sl) if action == 5 && (key == "LOAD" || key == "DELETE") => sl.sel.and_then(|r| sl.game(r)).is_some(),
        Body::SaveLoad(sl) if action == 5 && key == "SAVE" => sl.sel.is_some() && !sl.edit.get().trim().is_empty() && crate::save::can_save(false),
        _ => true,
    }
}

fn draw_body(u: &mut Ui<W>, win: &mut Win, fonts: &Fonts, cx: i32, cy: i32, waiting_keys: bool) {
    let ft = fonts.title.clone();
    let fs = fonts.small.clone();
    let fh = fonts.headers.clone();
    let focus = win.focus;
    match &mut win.body {
        Body::Menu { buttons, x, w } => {
            let font = u.hud.font(u.assets, "DXFonts.MainMenuTrueType").unwrap_or_else(|| fonts.title.clone());
            for (i, b) in buttons.iter().enumerate() {
                menu_button(u, W::MenuBtn(i as u8), cx + *x, cy + b.y, *w, &b.label, b.enabled, &font);
            }
        }
        Body::Choices { choices } => {
            for (i, c) in choices.iter().enumerate() {
                let y = cy + 27 + i as i32 * 36 - c.voff;
                action_button(u, W::ChoiceBtn(i as u8), cx + 7, y + c.voff, 243, &c.label, true);
                match &c.kind {
                    ChoiceKind::Enum(_) => {
                        info_button(u, cx + 7 + c.info_x, y, c.info_w, &c.value_text(), m_btn(0));
                        if c.dropdown {
                            let st = if win.drop.map(|d| d.0) == Some(i as u8) { "Pressed" } else { "Normal" };
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
            // An open drop-down: the list under (or, near the bottom, over) its value box.
            if let Some((di, mut scroll)) = win.drop {
                if let Some(c) = choices.get(di as usize) {
                    if let ChoiceKind::Enum(items) = &c.kind {
                        let (bx, by) = (cx + 7 + c.info_x, cy + 27 + di as i32 * 36 - c.voff);
                        let rows: Vec<Vec<String>> = items.iter().map(|t| vec![t.clone()]).collect();
                        let h = (rows.len() as i32 * ROW_H).min(10 * ROW_H) + 2;
                        let bottom = cy + win.f.ch;
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
                win.drop = Some((di, scroll));
            }
            if win.f.class == "MenuScreenBrightness" {
                // Colour bars that should fade smoothly to black at the bottom.
                for (k, col) in [[255u8, 255, 255], [255, 0, 0], [0, 255, 0], [0, 0, 255]].iter().enumerate() {
                    u.draw("MenuBrightnessGradient", cx + 14 + k as i32 * 136, cy + 67, 130, 137, Style::Normal, *col);
                }
                u.c.fill(cx + 14, cy + 178, 520, 1, Style::Normal, m_col(3));
            }
        }
        Body::NewGame(ng) => {
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
        Body::Keys(k) if k.pad => {
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
        Body::Enhance(en) => {
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
        Body::Keys(k) => {
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
        Body::SaveLoad(sl) => {
            let d = u.assets.classes.defaults("MenuScreenLoadGame");
            let l = |k: &str| pstr(&d, k);
            header_button(u, W::Header(0), cx + 10, cy + 3, 238, &l("strHeaderNameLabel"));
            header_button(u, W::Header(1), cx + 251, cy + 3, 131, &l("strHeaderDateLabel"));
            let mut rows: Vec<Vec<String>> = Vec::new();
            if sl.saving {
                rows.push(vec![l("NewSaveGameButtonText"), String::new()]);
            }
            for g in &sl.games {
                rows.push(vec![g.description.clone(), crate::save::format_date(g.saved_unix)]);
            }
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
                    format!("File Size: {}KB", (g.bytes + 1023) / 1024)
                );
                u.wrapped(&fs, cx + 390, cy + 166, 155, 60, HAlign::Left, VAlign::Center, m_col(8), &info);
            }
            let tex = if sl.confirm_delete { "MenuCheckBox_On" } else { "MenuCheckBox_Off" };
            u.draw(tex, cx + 389, cy + 256, 13, 13, Style::Masked, m_col(3));
            u.text(&fs, cx + 406, cy + 256, 150, 13, HAlign::Left, VAlign::Center, m_col(8), &l("ConfirmDeleteLabel"));
            u.hit(W::Header(2), (cx + 389, cy + 255, 160, 15), true);
        }
        Body::Keypad(_) => {}
        Body::Msg(m) => {
            let fds = u.hud.font(u.assets, "FontMenuHeaders_DS").unwrap_or(fh.clone());
            let (tx, ty, tw, th) = (cx + 20, cy + 14, win.f.cw - 40, win.f.ch - 28);
            u.wrapped(&fds, tx, ty, tw, th, HAlign::Center, VAlign::Center, m_col(8), &m.text);
        }
    }
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
        u.text(&small, 0, 148, 103, 12, HAlign::Center, VAlign::Center, h_col(7), "Tab: code from notes");
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
    /// The new character waiting on the autosave warning.
    box_new_game: Option<crate::travel::NewGame>,
    applied_gen: u64,
    themes_applied: bool,
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

fn not_available(hud: &mut ClassicHud, assets: &mut RuntimeAssets, what: &str) -> Act {
    let _ = (hud, assets);
    Act::Msg(what.to_string(), format!("{what} isn't available yet in this version."), false, MsgPurpose::Info)
}

/// ProcessMenuAction for a main-menu style button.
fn menu_action(b: &MenuBtn, hud: &mut ClassicHud, assets: &mut RuntimeAssets, out: &mut Vec<Act>) {
    match b.action {
        0 | 1 => match b.invoke.as_str() {
            "MenuScreenSaveGame" | "MenuScreenLoadGame" => out.push(Act::Push(b.invoke.clone())),
            // ShowCredits(False): back to this menu afterwards.
            "CreditsWindow" => crate::credits::show_from_menu(),
            "menumpmain" | "MenuMPMain" => out.push(not_available(hud, assets, "Multiplayer")),
            "" => {}
            c => out.push(Act::Push(c.to_string())),
        },
        2 => out.push(Act::Pop),
        3 => {
            // StartNewGame: ask about the training mission once.
            if crate::settings::get("AskedToTrain") < 0.5 {
                crate::settings::set("AskedToTrain", 1.0);
                crate::settings::save();
                let d = assets.classes.defaults("MenuMain");
                out.push(Act::Msg(pstr(&d, "AskToTrainTitle"), pstr(&d, "AskToTrainMessage"), true, MsgPurpose::AskToTrain));
            } else {
                out.push(Act::Push("MenuSelectDifficulty".into()));
            }
        }
        4 => out.push(Act::Training),
        5 => out.push(Act::Intro),
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
        st.built = true;
        let mission = level
            .0
            .actors
            .iter()
            .find(|a| a.class == "DeusExLevelInfo")
            .map(|a| pint(&a.props, "missionNumber", 0))
            .unwrap_or(0);
        MISSION.store(mission, Ordering::Relaxed);
        if let Some(Value::Str(loc)) = level.0.actors.iter().find(|a| a.class == "DeusExLevelInfo").and_then(|a| find(&a.props, "MissionLocation")) {
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
            st.win_layers.push(layer(&mut commands, &mut images, &mut layers, 61 + z));
        }
        st.cursor = Some(layer(&mut commands, &mut images, &mut layers, 70));
        // DX.dx: the camera flies the IntroCam path from Position 1 (TravelPostAccept).
        if mission == -2 && std::env::var("DXVIEW_SHOT").is_err() {
            let pts = crate::interp::path(&level.0, "IntroCam");
            if let Some(first) = crate::interp::at_position(&pts, 1) {
                let it = crate::interp::Interp::start(&pts, first);
                st.intro = Some((pts, it));
            }
        }
    }
    if !st.themes_applied {
        st.themes_applied = true;
        apply_themes(&mut assets, crate::settings::get("MenuColor") as usize, crate::settings::get("HUDColor") as usize);
    }
    // Live settings: themes follow the setting.
    let gen = crate::settings::generation();
    if gen != st.applied_gen {
        st.applied_gen = gen;
        apply_themes(&mut assets, crate::settings::get("MenuColor") as usize, crate::settings::get("HUDColor") as usize);
    }

    // ---- the logo flyby (DeusExPlayer state Interpolating): the camera rides the path, Fire shows the menu
    if let Some((pts, it)) = st.intro.as_mut() {
        it.tick(pts, vtime.delta_secs_f64().max(dt as f64 * 0.0) + if vtime.is_paused() { 0.0 } else { 0.0 });
        let to_rad = |u: f64| (u / 65536.0 * std::f64::consts::TAU) as f32;
        let rot = Quat::from_rotation_y(-to_rad(it.rot[1]) - std::f32::consts::FRAC_PI_2)
            * Quat::from_rotation_x(to_rad(it.rot[0]))
            * Quat::from_rotation_z(-to_rad(it.rot[2]));
        // PlayerCalcView: the camera is the player's location + EyeHeight (JCDentonMale BaseEyeHeight 40; only
        // CameraPoint cut scenes zero it), so the logos sit lower in the frame than the path points alone.
        let eye = Vec3::from(dx_level::ue_to_bevy([it.loc.x as f32, it.loc.y as f32, it.loc.z as f32 + 40.0]));
        for mut t in &mut cam {
            t.translation = eye;
            t.rotation = rot;
        }
        for mut t in &mut sky {
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
                let w = open_window(&mut hud, &mut assets, "MenuMain");
                st.push(w);
            }
        }
    }

    // Developer aid: DXVIEW_MENU_OPEN="MenuMain,MenuSettings,MenuScreenDisplay" opens those windows in turn after
    // two seconds (screenshots of the menus).
    if let Ok(v) = std::env::var("DXVIEW_MENU_OPEN") {
        static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        st.dead_t += 0.0;
        if !DONE.load(Ordering::Relaxed) && vtime.elapsed_secs() > 2.0 {
            DONE.store(true, Ordering::Relaxed);
            st.intro = None;
            for c in v.split(',') {
                // "MenuScreenNewGame#3": the new-game screen on portrait 3.
                let (c, portrait) = c.split_once('#').map(|(a, b)| (a, b.parse().ok())).unwrap_or((c, None));
                let mut w = open_window(&mut hud, &mut assets, c.trim());
                if let (Some(Body::NewGame(ng)), Some(p)) = (w.as_mut().map(|w| &mut w.body), portrait) {
                    ng.portrait = p;
                }
                st.push(w);
            }
        }
    }
    // ---- Escape in game opens the main menu (ShowMainMenu), unless another screen has the keys
    let others = crate::inventory_screen::SCREEN_OPEN.load(Ordering::Relaxed)
        || crate::computers::open()
        || binds.open
        || crate::consys::third_person();
    // The Escape that closed another screen doesn't also open the menu.
    let others_before = std::mem::replace(&mut st.others_last, others);
    // Dead (DeusExPlayer state Dying): Escape or Fire brings up the menu to load a game.
    DEAD.store(ph.dead, Ordering::Relaxed);
    st.dead_t = if ph.dead { st.dead_t + dt } else { 0.0 };
    let dead_click = ph.dead && st.dead_t > 2.0 && binds.buttons.just_pressed(MouseButton::Left);
    if st.stack.is_empty() && st.intro.is_none() && !others && !others_before && (binds.keys.just_pressed(KeyCode::Escape) || dead_click) {
        let w = open_window(&mut hud, &mut assets, "MenuMain");
        st.push(w);
    }
    // A keypad was used (Keypad.ActivateKeypadWindow).
    if let Some((title, text)) = EXTERNAL_BOX.lock().ok().and_then(|mut b| b.take()) {
        let w = msg_box(&mut hud, &mut assets, &title, &text, true, MsgPurpose::External);
        st.push(w);
    }
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
    // DeusExPlayer.QuickSave / QuickLoad (ConfirmQuickLoad asks first).
    if st.stack.is_empty() && st.intro.is_none() && !others && !others_before {
        if binds.action_just_pressed("Quick Save") && crate::save::can_save(ph.dead) {
            let title = pstr(&assets.classes.defaults("DeusExPlayer"), "QuickSaveGameTitle");
            crate::save::request(crate::save::quick_path(), if title.is_empty() { "Quick Save" } else { &title });
        }
        if binds.action_just_pressed("Quick Load") && crate::save::quick_path().exists() {
            let d = assets.classes.defaults("DeusExRootWindow");
            let w = msg_box(&mut hud, &mut assets, &pstr(&d, "QuickLoadTitle"), &pstr(&d, "QuickLoadMessage"), true, MsgPurpose::QuickLoad);
            st.push(w);
        }
    }
    let was_open = OPEN.load(Ordering::Relaxed);
    let is_open = !st.stack.is_empty();
    // The key that opened the menu this frame (Escape) mustn't also close it.
    let opened_now = is_open && !was_open;

    // ---- pause / unpause (UIPauseGame, except on the logo map)
    if is_open && !was_open {
        if !intro_map() && !vtime.is_paused() {
            vtime.pause();
            st.paused_game = true;
        }
        st.grabbed_before = cursor_opts.grab_mode != bevy::window::CursorGrabMode::None;
        cursor_opts.grab_mode = bevy::window::CursorGrabMode::None;
        cursor_opts.visible = false;
    }
    if !is_open && was_open {
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
            bg.0 = Color::srgba(0.0, 0.0, 0.0, if black { 1.0 } else { 0.5 });
            *v = if is_open { Visibility::Inherited } else { Visibility::Hidden };
        }
    }
    if !is_open {
        for l in &st.win_layers {
            if let Ok((_, mut v, _)) = nodes.get_mut(l.e) {
                if *v != Visibility::Hidden {
                    *v = Visibility::Hidden;
                }
            }
        }
        if let Some(c) = &st.cursor {
            if let Ok((_, mut v, _)) = nodes.get_mut(c.e) {
                *v = Visibility::Hidden;
            }
        }
        if st.intro.is_none() {
            wheel.clear();
        }
        return;
    }
    binds.capture = true;

    // ---- layout
    let fonts = {
        let mut get = |n: &str| hud.font(&mut assets, n);
        let (Some(title), Some(small), Some(headers), Some(fixed)) =
            (get("FontMenuTitle"), get("FontMenuSmall"), get("FontMenuHeaders"), get("FontFixedWidthSmall"))
        else {
            return;
        };
        Fonts { title, small, headers, fixed }
    };
    let (pw, phh) = (window.physical_width() as i32, window.physical_height() as i32);
    let s = crate::hud::screen_scale(&window, 640, 480);
    let (sw, sh) = (pw / s, phh / s);
    let sf = window.scale_factor();
    let mouse = window
        .cursor_position()
        .map(|c| (((c.x * sf) as i32).div_euclid(s), ((c.y * sf) as i32).div_euclid(s)))
        .unwrap_or((-100, -100));
    let top = st.stack.len() - 1;
    let pos_of = |w: &Win| -> (i32, i32) {
        let (ww, wh) = win_size(&w.f, &fonts);
        w.pos.unwrap_or((sw / 2 - ww / 2, sh / 2 - wh / 2))
    };

    // ---- input
    let hover = st.hits.iter().rev().find(|(_, r, _)| inside(mouse, *r)).map(|h| (h.0, h.2));
    let hover_w = hover.map(|h| h.0);
    if std::env::var("DXVIEW_SNAPLOG").is_ok() {
        eprintln!("SNAP mouse {mouse:?} hover {hover:?} hits {:?}", st.hits.iter().map(|h| (h.0, h.1)).collect::<Vec<_>>());
    }
    let shift = binds.keys.pressed(KeyCode::ShiftLeft) || binds.keys.pressed(KeyCode::ShiftRight);
    let ctrl = binds.keys.pressed(KeyCode::ControlLeft) || binds.keys.pressed(KeyCode::ControlRight);
    let mut acts: Vec<Act> = Vec::new();
    if CLOSE_ALL.swap(false, Ordering::Relaxed) && st.waiting_key.is_none() {
        acts.push(Act::PopAll);
    }
    let click_focus = matches!(st.stack[top].body, Body::NewGame(_) | Body::Keypad(_));
    // Hover moves the focus (MFOCUS_EnterLeave), except on the new-game screen (MFOCUS_Click).
    if !click_focus {
        if let Some((w, true)) = hover {
            if matches!(w, W::MenuBtn(_) | W::Act(_) | W::ChoiceBtn(_)) && st.stack[top].focus != Some(w) {
                st.stack[top].focus = Some(w);
                crate::sound::play_player("DeusExSounds.UserInterface.Menu_Focus", 0.25, 0.0);
            }
        }
    }
    // Waiting for a key on the keyboard screen: the next key or button is the binding.
    if let Some(t) = st.waiting_key.as_mut() {
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
                                let _ = j;
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
    let waiting = st.waiting_key.is_some();
    // An open drop-down takes the click: a row picks it, the list's scroll bar scrolls it, anywhere else closes it.
    let mut drop_click = false;
    if !waiting && binds.buttons.just_pressed(MouseButton::Left) && st.stack[top].drop.is_some() {
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
    let mut sl_double: Option<&str> = None;
    if !waiting && !drop_click && binds.buttons.just_pressed(MouseButton::Left) {
        st.pressed = None;
        if let Some((w, enabled)) = hover {
            if enabled {
                let win = &mut st.stack[top];
                match w {
                    W::Title => {
                        let p = pos_of(win);
                        st.drag = Some((mouse.0 - p.0, mouse.1 - p.1));
                    }
                    W::Edit(n) => {
                        win.focus = Some(w);
                        let _ = n;
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
                                        en.step(1, &mut acts);
                                    }
                                    en.sel = Some(i);
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
                                if double {
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
                        if click_focus {
                            win.focus = Some(w);
                        }
                    }
                }
                st.pressed = Some(w);
            }
        }
    }
    if let Some(k) = sl_double {
        save_load_action(st, top, k, &mut hud, &mut assets, &mut acts);
    }
    if binds.buttons.pressed(MouseButton::Left) {
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
    if binds.buttons.just_released(MouseButton::Left) {
        st.drag = None;
        if let Some(w) = st.pressed.take() {
            if hover_w == Some(w) {
                activate(st, top, w, false, &mut hud, &mut assets, &mut acts);
            }
        }
    }
    if binds.buttons.just_pressed(MouseButton::Right) {
        if let Some((w @ (W::ChoiceBtn(_) | W::ChoiceVal(_)), true)) = hover {
            activate(st, top, w, true, &mut hud, &mut assets, &mut acts);
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
    // Keys.
    if !waiting && !opened_now {
        let typed = binds.typed.clone();
        for (k, text) in &typed {
            if acts.iter().any(|a| matches!(a, Act::Pop | Act::PopAll)) {
                break;
            }
            let win = &mut st.stack[top];
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
                    continue;
                }
                // Ours: Tab types the code found in the notes.
                if *k == KeyCode::Tab && kp.noted && kp.wait.is_none() {
                    kp.input.clear();
                    let code: Vec<char> = kp.valid.chars().collect();
                    for ch in code {
                        if let Some(i) = (0..12).find(|&i| key_char(i) == ch) {
                            keypad_press(kp, i);
                        }
                    }
                    continue;
                }
            }
            // Enhancements: arrows move / change, Enter changes or opens, everything else types into the search.
            if let Body::Enhance(en) = &mut win.body {
                match k {
                    KeyCode::ArrowUp => en.move_sel(-1),
                    KeyCode::ArrowDown => en.move_sel(1),
                    KeyCode::PageUp => en.move_sel(-20),
                    KeyCode::PageDown => en.move_sel(20),
                    KeyCode::ArrowLeft => en.step(-1, &mut acts),
                    KeyCode::ArrowRight | KeyCode::Enter | KeyCode::NumpadEnter if en.sel.is_some() => en.step(1, &mut acts),
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
                    continue;
                }
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
                    continue;
                }
            }
            // An edit with the focus takes typing (MenuUIEditWindow; the new-game name keeps to its filter).
            if let (Some(W::Edit(0)), Body::NewGame(ng)) = (win.focus, &mut win.body) {
                let filter = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz1234567890 ";
                let t = text.as_deref().map(|t| t.chars().filter(|c| filter.contains(*c)).collect::<String>());
                if !matches!(k, KeyCode::Escape | KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Tab) {
                    ng.name.key(*k, t.as_deref().filter(|t| !t.is_empty()), shift, ctrl);
                    continue;
                }
            }
            match k {
                KeyCode::Escape if win.drop.is_some() => {
                    win.drop = None;
                }
                KeyCode::Escape => {
                    if let Body::Msg(m) = &win.body {
                        // MB_YesNo: No; MB_OK: OK.
                        let yes = !m.yes_no;
                        box_answer(st, top, yes, &mut hud, &mut assets, &mut acts);
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
                        Body::Msg(_) => box_answer(st, top, true, &mut hud, &mut assets, &mut acts),
                        Body::Menu { .. } => {
                            if let Some(f) = win.focus {
                                activate(st, top, f, false, &mut hud, &mut assets, &mut acts);
                            }
                        }
                        Body::Keys(kk) if kk.sel.is_some() => {
                            if let Body::Keys(kk) = &mut win.body {
                                kk.waiting = true;
                            }
                            st.waiting_key = Some(0.0);
                        }
                        Body::NewGame(_) | Body::Keypad(_) => {}
                        Body::SaveLoad(sl) => {
                            let k = if sl.saving { "SAVE" } else { "LOAD" };
                            save_load_action(st, top, k, &mut hud, &mut assets, &mut acts);
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
                        box_answer(st, top, true, &mut hud, &mut assets, &mut acts);
                    }
                }
                KeyCode::KeyN => {
                    if matches!(&win.body, Body::Msg(m) if m.yes_no) {
                        box_answer(st, top, false, &mut hud, &mut assets, &mut acts);
                    }
                }
                KeyCode::ArrowUp | KeyCode::ArrowDown => {
                    let up = *k == KeyCode::ArrowUp;
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
                        Body::Choices { choices } => {
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
                KeyCode::ArrowLeft | KeyCode::ArrowRight => {
                    // MenuUIScreenWindow: left / right cycle the focused choice.
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
    }

    // ---- apply
    for a in acts {
        match a {
            Act::Push(c) => {
                let (class, arg) = c.split_once(':').map(|(a, b)| (a.to_string(), Some(b.to_string()))).unwrap_or((c.clone(), None));
                let mut w = open_window(&mut hud, &mut assets, &class);
                if let (Some(w), Some(arg)) = (w.as_mut(), arg) {
                    if let Body::NewGame(ng) = &mut w.body {
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
                }
                let tip = class == "MenuScreenNewGame" && crate::travel::lddp() && crate::settings::get("LDDPGaveNewGameTips") == 0.0;
                st.push(w);
                if tip {
                    // MenuScreenNewGame.GiveTip (once: bGaveNewGameTips).
                    crate::settings::set("LDDPGaveNewGameTips", 1.0);
                    crate::settings::save();
                    let w = msg_box(
                        &mut hud,
                        &mut assets,
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
                let w = msg_box(&mut hud, &mut assets, &title, &text, yes_no, purpose);
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
                window.resolution.set_physical_resolution(w, h);
            }
        }
    }
    if st.stack.is_empty() {
        return;
    }

    // ---- draw: the top window and the ones it doesn't hide
    let mut first = st.stack.len() - 1;
    while first > 0 && !st.stack[first].hides_below {
        first -= 1;
    }
    let help = {
        let top_w = &st.stack[st.stack.len() - 1];
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
                Some(Row::Group(g)) => format!("Show or hide the {} settings (Enter / click; Left closes). Type to search every setting.", en.groups().get(g).cloned().unwrap_or_default()),
                None => "Type to search the options.".into(),
            },
            (Body::Choices { .. }, _) if top_w.f.class == "MenuScreenBrightness" => {
                pstr(&assets.classes.defaults("MenuScreenBrightness"), "HelpMessage")
            }
            _ => String::new(),
        }
    };
    let mut hits: Vec<(W, Rect, bool)> = Vec::new();
    let n_layers = st.win_layers.len();
    let top_i = st.stack.len() - 1;
    let pressed = st.pressed;
    for (li, wi) in (first..st.stack.len()).enumerate() {
        if li >= n_layers {
            break;
        }
        let pos = pos_of(&st.stack[wi]);
        let (ww, wh) = win_size(&st.stack[wi].f, &fonts);
        let is_top = wi == top_i;
        let mut layer_hits: Vec<(W, Rect, bool)> = Vec::new();
        let focus = st.stack[wi].focus;
        let mut ui = Ui {
            c: Canvas::new(ww, wh, s),
            hud: &mut hud,
            assets: &mut assets,
            f: &fonts,
            o: pos,
            hits: &mut layer_hits,
            pressed: if is_top { pressed } else { None },
            hover: if is_top { hover_w } else { None },
            focus,
        };
        draw_win(&mut ui, &mut st.stack[wi], &fonts, if is_top { &help } else { "" }, waiting);
        let Ui { c, .. } = ui;
        place(&mut st.win_layers[li], &c, pos, s, sf, &mut images, &mut layers, &mut nodes);
        if is_top {
            hits = layer_hits;
        }
    }
    for li in (st.stack.len() - first)..n_layers {
        if let Ok((_, mut v, _)) = nodes.get_mut(st.win_layers[li].e) {
            *v = Visibility::Hidden;
        }
    }
    // A message box keeps the clicks to itself.
    st.hits = hits;
    // Controller snap-to targets: the widgets you click, not the frame, title bar or scroll tracks.
    crate::snap::offer(
        crate::snap::LAYER_MENU,
        crate::snap::Map { s, ox: 0, oy: 0, sf },
        st.hits.iter().filter(|h| h.2 && !matches!(h.0, W::Win | W::Title | W::ScrollTrack(_))).map(|h| h.1),
    );
    // Cursor: the same DeusExCursor2 as the in-game screens (ours: the original menus used Cursor1).
    if let Some(cl) = st.cursor.as_mut() {
        let mut cc = Canvas::new(64, 64, s);
        if let Some(tx) = hud.ui(&mut assets, "DeusExCursor2") {
            cc.drop_shadow(0, 0, &tx, (2.0, 2.5), 2.5, 0.6);
            cc.texture(0, 0, 64, 64, 0, 0, &tx, Style::Masked, m_col(12));
        }
        place(cl, &cc, (mouse.0 - 32, mouse.1 - 32), s, sf, &mut images, &mut layers, &mut nodes);
        if window.cursor_position().is_none() {
            if let Ok((_, mut v, _)) = nodes.get_mut(cl.e) {
                *v = Visibility::Hidden;
            }
        }
    }
    let _: Rgb = WHITE;
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
        Body::SaveLoad(sl) if id == 0 => sl.scroll = (sl.scroll + d).max(0),
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
        Body::SaveLoad(sl) if id == 0 => sl.scroll = v,
        Body::NewGame(ng) if id == 1 => ng.desc_scroll = v,
        _ => {}
    }
}

/// A message box answer (BoxOptionSelected in the window that opened it).
fn box_answer(st: &mut Menus, top: usize, yes: bool, hud: &mut ClassicHud, assets: &mut RuntimeAssets, acts: &mut Vec<Act>) {
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
        MsgPurpose::AutosaveWarn => {
            if let Some(ng) = st.box_new_game.take() {
                if yes {
                    acts.push(Act::NewGame(ng));
                }
            }
        }
        MsgPurpose::Overwrite => {
            if yes {
                if let Some((p, d)) = st.box_save.take() {
                    acts.push(Act::SaveGame(p, d));
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
        MsgPurpose::Info | MsgPurpose::NameBlank | MsgPurpose::Restart => {}
    }
}

/// The save / load screen under the message box lists the folder again (after a delete).
fn refresh_games(st: &mut Menus) {
    for w in st.stack.iter_mut() {
        if let Body::SaveLoad(sl) = &mut w.body {
            sl.games = crate::save::list();
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
                acts.push(Act::LoadGame(g.path));
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
                        if name.is_empty() {
                            let d = assets.classes.defaults("MenuScreenNewGame");
                            acts.push(Act::Msg(pstr(&d, "NameBlankTitle"), pstr(&d, "NameBlankPrompt"), false, MsgPurpose::NameBlank));
                        } else if key == "START" {
                            let lddp = crate::travel::lddp();
                            let female = lddp && ng.portrait % 2 == 1;
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
                            };
                            // Ours: a new character's autosaves go into the same five slots as everyone's.
                            if crate::save::autosaves_exist() {
                                st.box_new_game = Some(new_game);
                                acts.push(Act::Msg(
                                    "Autosaves".into(),
                                    format!(
                                        "This game will autosave into the same {} autosave slots, overwriting the oldest autosaves of earlier games. Your own saves are kept. Continue?",
                                        crate::save::AUTOSAVE_SLOTS
                                    ),
                                    true,
                                    MsgPurpose::AutosaveWarn,
                                ));
                            } else {
                                acts.push(Act::NewGame(new_game));
                            }
                        }
                    }
                }
                10 | 11 | 12 => box_answer(st, top, action != 10, hud, assets, acts),
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
                        "This version always renders with wgpu (Vulkan / DirectX 12).".into(),
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
        let mut assets = RuntimeAssets {
            classes: unreal_pkg::classes::ClassDb::open(&root.join("System")),
            lib: dx_level::TexLib::new(&root),
            images: Default::default(),
            materials: Default::default(),
            fire: Vec::new(),
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
            let (ww, wh) = win_size(&win.f, &fonts);
            let mut full = Canvas::new(640, 480, 1);
            full.fill(0, 0, 640, 480, Style::Normal, [20, 30, 40]);
            let mut hits = vec![];
            let pos = (320 - ww / 2, 240 - wh / 2);
            let help = match &win.body {
                Body::Choices { choices } => choices[0].help.clone(),
                _ => String::new(),
            };
            let mut ui = Ui { c: Canvas::new(ww, wh, 1), hud: &mut hud, assets: &mut assets, f: &fonts, o: pos, hits: &mut hits, pressed: None, hover: None, focus: win.focus };
            draw_win(&mut ui, &mut win, &fonts, &help, false);
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
            std::fs::write(format!("{out}/menu_{class}.rgba"), full.to_rgba8()).unwrap();
            println!("wrote {class} {ww}x{wh}");
        }
    }
}

/// A new map is going in (loader.rs): this module's state of the map being left.
pub fn reset_level() {
    OPEN.store(false, std::sync::atomic::Ordering::Relaxed);
    DEAD.store(false, std::sync::atomic::Ordering::Relaxed);
    if let Ok(mut v) = KEYPAD_OPEN.lock() {
        *v = None;
    }
    if let Ok(mut v) = KEYPAD_RESULTS.lock() {
        v.clear();
    }
}
