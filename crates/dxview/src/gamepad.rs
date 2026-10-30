//! Controller support (ROADMAP Phase 0.1). Gamepads come in through gilrs (XInput / Windows.Gaming.Input on Windows,
//! evdev on Linux / Steam Deck): Xbox pads, DualSense, DualShock 4 and anything with an SDL mapping.
//!
//! The pad drives the same layer the keyboard does (keybinds.rs). Each pad button is bound to a pad action. A plain
//! action presses whatever physical key or mouse button that action is bound to on the keyboard screen, so keyboard
//! rebinding and every system that reads keys keep working unchanged. On top of that:
//! * the left stick gives analog movement (`move_axes`; a light push walks, like holding Walk) and the right stick
//!   turns the view (`look_delta`: rate with a response curve, sensitivity and invert from the settings);
//! * when the mouse cursor is free (menus, the inventory, computers, conversation choices) the pad becomes a mouse:
//!   the left stick moves the cursor, A clicks, X right-clicks, B / Start are Escape, the D-pad gives arrow keys,
//!   the right stick scrolls;
//! * specials: Use / reload (tap uses, hold reloads), Inventory / goals (tap F1, hold F2), next / previous belt item
//!   (the mouse wheel), the augmentation wheel (augwheel.rs) and D-pad hotkey pages.
//!
//! Pad bindings live in padbinds.txt beside the exe (Settings > Controls > Controller Bindings edits them).

use crate::keybinds::{Input, ACTIONS};
use bevy::prelude::*;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PadBtn {
    /// A / Cross.
    South,
    /// B / Circle.
    East,
    /// X / Square.
    West,
    /// Y / Triangle.
    North,
    LB,
    RB,
    LT,
    RT,
    /// View / Share / Create.
    Select,
    /// Menu / Options.
    Start,
    L3,
    R3,
    Up,
    Down,
    Left,
    Right,
    /// The touchpad click (DualSense / DualShock 4).
    Touch,
}

pub const ALL_BTNS: [PadBtn; 17] = [
    PadBtn::South,
    PadBtn::East,
    PadBtn::West,
    PadBtn::North,
    PadBtn::LB,
    PadBtn::RB,
    PadBtn::LT,
    PadBtn::RT,
    PadBtn::Select,
    PadBtn::Start,
    PadBtn::L3,
    PadBtn::R3,
    PadBtn::Up,
    PadBtn::Down,
    PadBtn::Left,
    PadBtn::Right,
    PadBtn::Touch,
];

/// Which glyph family fits the connected pad.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PadKind {
    #[default]
    Xbox,
    PlayStation,
}

/// What a pad button does.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PadAction {
    /// One of keybinds::ACTIONS (by name): presses its bound key / button.
    Game(&'static str),
    /// Tap: Use / pick up; hold (0.35 s): Reload.
    UseReload,
    /// Tap: inventory (F1); hold: goals / notes (F2).
    InvGoals,
    NextBelt,
    PrevBelt,
    /// Tap: third person view (when that Enhancement is on, else Change ammo); hold: Change ammo.
    ViewAmmo,
    /// Hold for the augmentation wheel.
    AugWheel,
    /// The pause menu (Escape).
    Menu,
    None,
}

/// Every pad action with its default button (the modern shooter layout, user decision 2026-10-27).
pub const PAD_ACTIONS: &[(&str, PadAction, Option<PadBtn>)] = &[
    ("Fire / throw", PadAction::Game("Fire / throw"), Some(PadBtn::RT)),
    ("Aim / scope", PadAction::Game("Scope"), Some(PadBtn::LT)),
    ("Jump / swim up", PadAction::Game("Jump / swim up"), Some(PadBtn::South)),
    ("Crouch / swim down", PadAction::Game("Crouch / swim down"), Some(PadBtn::East)),
    ("Use / reload (tap), put away (hold)", PadAction::UseReload, Some(PadBtn::West)),
    ("Next weapon / item", PadAction::NextBelt, Some(PadBtn::North)),
    ("Lean left", PadAction::Game("Lean left"), Some(PadBtn::LB)),
    ("Lean right", PadAction::Game("Lean right"), Some(PadBtn::RB)),
    ("Walk (hold)", PadAction::Game("Walk (hold)"), Some(PadBtn::L3)),
    ("Third person (tap) / change ammo (hold)", PadAction::ViewAmmo, Some(PadBtn::R3)),
    ("Change ammo", PadAction::Game("Change ammo"), None),
    ("Augmentation wheel (hold)", PadAction::AugWheel, Some(PadBtn::Up)),
    ("Flashlight", PadAction::Game("Augmentation F12 (light)"), Some(PadBtn::Down)),
    ("Previous belt item", PadAction::PrevBelt, Some(PadBtn::Left)),
    ("Next belt item", PadAction::NextBelt, Some(PadBtn::Right)),
    ("Inventory (tap) / goals (hold)", PadAction::InvGoals, Some(PadBtn::Select)),
    ("Menu", PadAction::Menu, Some(PadBtn::Start)),
    ("Reload", PadAction::Game("Reload"), None),
    ("Use / pick up", PadAction::Game("Use / pick up"), None),
    ("Laser sight", PadAction::Game("Laser sight"), None),
    ("Inventory screen", PadAction::Game("Inventory screen"), None),
    ("Goals / notes screen", PadAction::Game("Goals / notes screen"), Some(PadBtn::Touch)),
    ("Quick Save", PadAction::Game("Quick Save"), None),
    ("Quick Load", PadAction::Game("Quick Load"), None),
];

/// Analog state and what the pad wants this frame.
#[derive(Default, Clone)]
pub struct PadFrame {
    pub connected: bool,
    pub kind: PadKind,
    pub name: String,
    /// Sticks (x right, y up) and triggers (0..1), after deadzones.
    pub ls: Vec2,
    pub rs: Vec2,
    pub lt: f32,
    pub rt: f32,
    pub held: Vec<PadBtn>,
    pub just: Vec<PadBtn>,
    /// The last input came from the pad (glyph auto-select, cursor hiding).
    pub pad_last: bool,
}

static FRAME: Mutex<Option<PadFrame>> = Mutex::new(None);

pub fn frame() -> PadFrame {
    FRAME.lock().ok().and_then(|f| f.clone()).unwrap_or_default()
}

pub fn connected() -> bool {
    frame().connected
}

/// The connected pad's glyph family, or the one picked in Enhancements.
pub fn glyph_kind() -> PadKind {
    match crate::settings::get("PadGlyphs") as i32 {
        1 => PadKind::Xbox,
        2 => PadKind::PlayStation,
        _ => frame().kind,
    }
}

pub struct GilrsRes(pub gilrs::Gilrs);

pub fn setup(world: &mut World) {
    match gilrs::Gilrs::new() {
        Ok(g) => {
            for (_, p) in g.gamepads() {
                println!("PAD {} connected ({:?})", p.name(), p.power_info());
            }
            world.insert_non_send(GilrsRes(g));
        }
        Err(e) => warn!("no gamepad support: {e}"),
    }
}

fn btn_of(b: gilrs::Button) -> Option<PadBtn> {
    use gilrs::Button as B;
    Some(match b {
        B::South => PadBtn::South,
        B::East => PadBtn::East,
        B::West => PadBtn::West,
        B::North => PadBtn::North,
        B::LeftTrigger => PadBtn::LB,
        B::RightTrigger => PadBtn::RB,
        B::LeftTrigger2 => PadBtn::LT,
        B::RightTrigger2 => PadBtn::RT,
        B::Select => PadBtn::Select,
        B::Start => PadBtn::Start,
        B::LeftThumb => PadBtn::L3,
        B::RightThumb => PadBtn::R3,
        B::DPadUp => PadBtn::Up,
        B::DPadDown => PadBtn::Down,
        B::DPadLeft => PadBtn::Left,
        B::DPadRight => PadBtn::Right,
        B::C | B::Z | B::Mode => return None,
        _ => return None,
    })
}

fn kind_of(name: &str, vendor: Option<u16>) -> PadKind {
    let n = name.to_ascii_lowercase();
    if vendor == Some(0x054c) || n.contains("dualsense") || n.contains("dualshock") || n.contains("playstation") || n.contains("ps5") || n.contains("ps4") || n.contains("wireless controller") {
        PadKind::PlayStation
    } else {
        PadKind::Xbox
    }
}

/// Radial deadzone with the rest rescaled to 0..1.
fn dead(v: Vec2, dz: f32) -> Vec2 {
    let m = v.length();
    if m <= dz {
        Vec2::ZERO
    } else {
        v / m * ((m - dz) / (1.0 - dz)).min(1.0)
    }
}

/// PreUpdate, before keybinds::remap_system: poll the pads.
pub fn poll_system(gilrs: Option<NonSendMut<GilrsRes>>, mut prev: Local<Vec<PadBtn>>) {
    let mut f = PadFrame::default();
    let mut any_event = false;
    if let Some(g) = gilrs {
        read_pad(g, &mut f, &mut any_event);
    }
    finish_frame(f, any_event, &mut prev);
}

fn read_pad(mut g: NonSendMut<GilrsRes>, f: &mut PadFrame, any_event: &mut bool) {
    while let Some(ev) = g.0.next_event() {
        *any_event = true;
        match ev.event {
            gilrs::EventType::Connected => {
                let p = g.0.gamepad(ev.id);
                println!("PAD {} connected", p.name());
            }
            gilrs::EventType::Disconnected => println!("PAD disconnected"),
            _ => {}
        }
    }
    // A DualSense on HID (dualsense.rs) is the pad: its own report has every control and the touchpad.
    if let Some(ds) = crate::dualsense::input() {
        f.connected = true;
        f.name = "DualSense".into();
        f.kind = PadKind::PlayStation;
        let dz = [0.06, 0.1, 0.15, 0.22, 0.3][(crate::settings::get("PadDeadzone") as usize).min(4)];
        f.ls = dead(ds.ls, dz);
        f.rs = dead(ds.rs, dz);
        f.lt = ds.lt;
        f.rt = ds.rt;
        f.held = ds.held;
        if f.lt > 0.33 {
            f.held.push(PadBtn::LT);
        }
        if f.rt > 0.33 {
            f.held.push(PadBtn::RT);
        }
        if crate::dualsense::touch_click() {
            f.held.push(PadBtn::Touch);
        }
        *ACTIVE.lock().unwrap() = None;
        return;
    }
    // The active pad: the first connected one.
    let pad = g.0.gamepads().find(|(_, p)| p.is_connected()).map(|(id, _)| id);
    *ACTIVE.lock().unwrap() = pad;
    if let Some(id) = pad {
        let p = g.0.gamepad(id);
        f.connected = true;
        f.name = p.name().to_string();
        f.kind = kind_of(p.name(), p.vendor_id());
        let dz = [0.06, 0.1, 0.15, 0.22, 0.3][(crate::settings::get("PadDeadzone") as usize).min(4)];
        f.ls = dead(Vec2::new(p.value(gilrs::Axis::LeftStickX), p.value(gilrs::Axis::LeftStickY)), dz);
        f.rs = dead(Vec2::new(p.value(gilrs::Axis::RightStickX), p.value(gilrs::Axis::RightStickY)), dz);
        let trig = |b: gilrs::Button| p.button_data(b).map_or(0.0, |d| d.value());
        f.lt = trig(gilrs::Button::LeftTrigger2);
        f.rt = trig(gilrs::Button::RightTrigger2);
        for b in [
            gilrs::Button::South,
            gilrs::Button::East,
            gilrs::Button::West,
            gilrs::Button::North,
            gilrs::Button::LeftTrigger,
            gilrs::Button::RightTrigger,
            gilrs::Button::Select,
            gilrs::Button::Start,
            gilrs::Button::LeftThumb,
            gilrs::Button::RightThumb,
            gilrs::Button::DPadUp,
            gilrs::Button::DPadDown,
            gilrs::Button::DPadLeft,
            gilrs::Button::DPadRight,
        ] {
            if p.is_pressed(b) {
                if let Some(pb) = btn_of(b) {
                    f.held.push(pb);
                }
            }
        }
        // Triggers as buttons past a third of their travel.
        if f.lt > 0.33 {
            f.held.push(PadBtn::LT);
        }
        if f.rt > 0.33 {
            f.held.push(PadBtn::RT);
        }
        if crate::dualsense::touch_click() {
            f.held.push(PadBtn::Touch);
        }
    }
}

fn finish_frame(mut f: PadFrame, any_event: bool, prev: &mut Vec<PadBtn>) {
    // Developer aid: DXVIEW_FAKEPAD="ps|xbox;Btn,Btn;rx,ry" pretends a pad is connected with those buttons held and
    // the right stick there (screenshots of the aug wheel and glyphs without a pad).
    if let Ok(v) = std::env::var("DXVIEW_FAKEPAD") {
        let p: Vec<&str> = v.split(';').collect();
        f.connected = true;
        f.kind = if p.first() == Some(&"ps") { PadKind::PlayStation } else { PadKind::Xbox };
        f.name = "Fake pad".into();
        if let Some(bs) = p.get(1) {
            for b in bs.split(',') {
                if let Some(pb) = ALL_BTNS.iter().find(|x| format!("{x:?}").eq_ignore_ascii_case(b.trim())) {
                    f.held.push(*pb);
                }
            }
        }
        if let Some(r) = p.get(2) {
            let v: Vec<f32> = r.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if v.len() == 2 {
                f.rs = Vec2::new(v[0], v[1]);
            }
        }
        f.pad_last = true;
    }
    f.just = f.held.iter().copied().filter(|b| !prev.contains(b)).collect();
    let was_pad = FRAME.lock().ok().and_then(|x| x.as_ref().map(|x| x.pad_last)).unwrap_or(false);
    f.pad_last = if !f.just.is_empty() || f.ls.length() > 0.3 || f.rs.length() > 0.3 { true } else { was_pad && !any_kbm() };
    let _ = any_event;
    *prev = f.held.clone();
    if let Ok(mut x) = FRAME.lock() {
        *x = Some(f);
    }
}

/// Keyboard / mouse activity this frame (set by keybinds) hands the glyphs back to the keyboard.
static KBM: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn note_kbm() {
    KBM.store(true, std::sync::atomic::Ordering::Relaxed);
}
fn any_kbm() -> bool {
    KBM.swap(false, std::sync::atomic::Ordering::Relaxed)
}

// ------------------------------------------------------------------------------------------------ bindings

pub struct PadBindings {
    /// The button for each PAD_ACTIONS entry.
    pub map: Vec<Option<PadBtn>>,
}

fn save_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("padbinds.txt")))
        .unwrap_or_else(|| PathBuf::from("padbinds.txt"))
}

static BINDS: Mutex<Option<Vec<Option<PadBtn>>>> = Mutex::new(None);

pub fn defaults() -> Vec<Option<PadBtn>> {
    PAD_ACTIONS.iter().map(|a| a.2).collect()
}

pub fn bindings() -> Vec<Option<PadBtn>> {
    let mut g = BINDS.lock().unwrap();
    if g.is_none() {
        let mut map = defaults();
        if let Ok(t) = std::fs::read_to_string(save_path()) {
            for line in t.lines() {
                let Some((name, v)) = line.split_once('=') else { continue };
                let (name, v) = (name.trim(), v.trim());
                let Some(i) = PAD_ACTIONS.iter().position(|a| a.0 == name) else { continue };
                map[i] = if v == "None" { None } else { ALL_BTNS.iter().copied().find(|b| format!("{b:?}") == v) };
            }
        }
        *g = Some(map);
    }
    g.clone().unwrap()
}

pub fn set_bindings(map: Vec<Option<PadBtn>>) {
    let text: String = PAD_ACTIONS
        .iter()
        .zip(&map)
        .map(|(a, b)| format!("{} = {}\n", a.0, b.map_or("None".to_string(), |b| format!("{b:?}"))))
        .collect();
    if let Err(e) = std::fs::write(save_path(), text) {
        warn!("could not save pad bindings: {e}");
    }
    *BINDS.lock().unwrap() = Some(map);
}

/// The pad action bound to a button (the first one).
pub fn action_of(b: PadBtn) -> PadAction {
    let map = bindings();
    PAD_ACTIONS.iter().zip(&map).find(|(_, m)| **m == Some(b)).map(|(a, _)| a.1.clone()).unwrap_or(PadAction::None)
}

/// The button bound to a pad action (for prompts), by action name.
pub fn button_for(name: &str) -> Option<PadBtn> {
    let map = bindings();
    PAD_ACTIONS.iter().position(|a| a.0 == name).and_then(|i| map[i])
}

// ------------------------------------------------------------------------------------------------ to the game

/// What the pad presses this frame, as physical keyboard / mouse inputs (keybinds merges them with the real
/// devices), plus mouse-wheel steps. `ui`: the cursor is free (menus), so the pad acts as a mouse.
pub struct PadOut {
    pub inputs: Vec<Input>,
    pub scroll: f32,
    /// Cursor movement in physical pixels (UI mode).
    pub cursor: Vec2,
    /// A D-pad snap this frame (UI mode, widgets offered): the direction, screen y down.
    pub snap: Option<Vec2>,
}

#[derive(Default)]
pub struct HoldState {
    /// How long each special button has been held.
    use_t: f32,
    use_fired: bool,
    inv_t: f32,
    inv_fired: bool,
    view_t: f32,
    view_fired: bool,
    /// One-frame taps to send (input, frames left).
    taps: Vec<(Input, u8)>,
    scroll_acc: f32,
    /// D-pad held in snap mode: for how long, and when it next repeats.
    dpad_t: f32,
    dpad_next: f32,
    /// Buttons ignored until released (Start after it closed the menu, so it doesn't reopen it).
    suppress: Vec<PadBtn>,
}

static HOLD: Mutex<Option<HoldState>> = Mutex::new(None);

/// The physical keyboard / mouse input an action is bound to.
fn bound(map: &[Input], action: &str) -> Option<Input> {
    ACTIONS.iter().position(|a| a.0 == action).map(|i| map[i])
}

/// The pad buttons for conversation choices 1-4 with Dialogue Hotkeys on.
pub const DIALOGUE_HOTKEYS: [(PadBtn, KeyCode); 4] =
    [(PadBtn::West, KeyCode::Digit1), (PadBtn::North, KeyCode::Digit2), (PadBtn::LB, KeyCode::Digit3), (PadBtn::RB, KeyCode::Digit4)];

/// Y in a menu: type the keypad code found in the notes (menus.rs takes it).
pub static PAD_NOTES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The pad was the last thing used (glyphs instead of keys in prompts).
pub fn pad_in_use() -> bool {
    let f = frame();
    f.connected && f.pad_last
}

pub fn outputs(key_map: &[Input], dt: f32, ui: bool, snap_ok: bool) -> PadOut {
    let f = frame();
    let mut out = PadOut { inputs: Vec::new(), scroll: 0.0, cursor: Vec2::ZERO, snap: None };
    if !f.connected {
        return out;
    }
    let mut g = HOLD.lock().unwrap();
    let h = g.get_or_insert_with(HoldState::default);
    // Taps from last frame's releases are held one frame so the game sees a press and a release.
    h.taps.retain_mut(|t| {
        out.inputs.push(t.0);
        t.1 = t.1.saturating_sub(1);
        t.1 > 0
    });
    h.suppress.retain(|b| f.held.contains(b));
    let suppressed = h.suppress.clone();
    let held = |b: PadBtn| f.held.contains(&b) && !suppressed.contains(&b);
    let just = |b: PadBtn| f.just.contains(&b) && !suppressed.contains(&b);
    if ui {
        // The pad as a mouse: left stick cursor (with acceleration), A click, X right click, B / Start Escape,
        // D-pad arrows, right stick scroll.
        let speed = 900.0 * (0.4 + 0.6 * f.ls.length());
        out.cursor = Vec2::new(f.ls.x, -f.ls.y) * speed * dt;
        if held(PadBtn::South) {
            out.inputs.push(Input::Mouse(MouseButton::Left));
        }
        // Ours: with Dialogue Hotkeys on, conversation choices 1-4 are X / Y / LB / RB (A still takes the
        // highlighted one); elsewhere Y is Tab (a keypad code from the notes).
        let hotkeys = crate::consys::choosing() && crate::settings::get("DialogueHotkeys") > 0.5;
        if hotkeys {
            for (b, k) in DIALOGUE_HOTKEYS {
                if just(b) {
                    h.taps.push((Input::Key(k), 2));
                }
            }
        } else {
            if held(PadBtn::West) {
                out.inputs.push(Input::Mouse(MouseButton::Right));
            }
            if just(PadBtn::North) {
                PAD_NOTES.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        // Start on the menus closes them all (back to the game); elsewhere it's Escape like B.
        if just(PadBtn::Start) && crate::menus::open() && crate::menus::request_close_all() {
            h.suppress.push(PadBtn::Start);
        } else if held(PadBtn::East) || held(PadBtn::Start) {
            out.inputs.push(Input::Key(KeyCode::Escape));
        }
        let dirs = [
            (PadBtn::Up, KeyCode::ArrowUp, Vec2::new(0.0, -1.0)),
            (PadBtn::Down, KeyCode::ArrowDown, Vec2::new(0.0, 1.0)),
            (PadBtn::Left, KeyCode::ArrowLeft, Vec2::new(-1.0, 0.0)),
            (PadBtn::Right, KeyCode::ArrowRight, Vec2::new(1.0, 0.0)),
        ];
        if snap_ok {
            // Snap-to: a press jumps to the next widget that way; held, it repeats after 0.4 s every 0.12 s.
            let dir = dirs.iter().find(|d| held(d.0)).map(|d| d.2);
            match dir {
                Some(d) => {
                    let fresh = dirs.iter().any(|x| x.2 == d && just(x.0));
                    if fresh {
                        h.dpad_t = 0.0;
                        h.dpad_next = 0.4;
                        out.snap = Some(d);
                    } else {
                        h.dpad_t += dt;
                        if h.dpad_t >= h.dpad_next {
                            h.dpad_next += 0.12;
                            out.snap = Some(d);
                        }
                    }
                }
                None => h.dpad_t = 0.0,
            }
        } else {
            for (b, k, _) in dirs {
                if held(b) {
                    out.inputs.push(Input::Key(k));
                }
            }
        }
        // Select still closes the inventory / goals screens.
        if just(PadBtn::Select) {
            if let Some(k) = bound(key_map, "Inventory screen") {
                h.taps.push((k, 2));
            }
        }
        if held(PadBtn::Touch) {
            out.inputs.push(Input::Mouse(MouseButton::Left));
        }
        // The DualSense touchpad as a laptop trackpad: one finger moves the cursor, two scroll, a flick is an
        // arrow key (choice pages, list pages).
        let t = crate::dualsense::take_touch();
        out.cursor += t.drag * 0.9;
        h.scroll_acc += -t.scroll / 90.0;
        match t.swipe {
            -1 => h.taps.push((Input::Key(KeyCode::ArrowLeft), 2)),
            1 => h.taps.push((Input::Key(KeyCode::ArrowRight), 2)),
            _ => {}
        }
        h.scroll_acc += f.rs.y * dt * 8.0;
        while h.scroll_acc.abs() >= 1.0 {
            out.scroll += h.scroll_acc.signum();
            h.scroll_acc -= h.scroll_acc.signum();
        }
        return out;
    }
    h.scroll_acc = 0.0;
    // In play a touchpad flick turns the belt (like the mouse wheel).
    let t = crate::dualsense::take_touch();
    out.scroll += -(t.swipe as f32);
    let map = bindings();
    for (i, (_, act, _)) in PAD_ACTIONS.iter().enumerate() {
        let Some(b) = map[i] else { continue };
        match act {
            PadAction::Game(name) => {
                if held(b) {
                    if let Some(k) = bound(key_map, name) {
                        out.inputs.push(k);
                    }
                }
            }
            PadAction::UseReload => {
                // Tap: use what's highlighted (or drop / throw what's carried), else reload. Hold: Use with nothing
                // highlighted, i.e. put the weapon away (the original's right click on nothing).
                if held(b) {
                    h.use_t += dt;
                    if h.use_t >= 0.35 && !h.use_fired {
                        h.use_fired = true;
                        if let Some(k) = bound(key_map, "Use / pick up") {
                            h.taps.push((k, 2));
                        }
                    }
                } else if h.use_t > 0.0 {
                    if !h.use_fired {
                        let act = if crate::frobui::has_target() || crate::carry::carrying() { "Use / pick up" } else { "Reload" };
                        if let Some(k) = bound(key_map, act) {
                            h.taps.push((k, 2));
                        }
                    }
                    h.use_t = 0.0;
                    h.use_fired = false;
                }
            }
            PadAction::InvGoals => {
                if held(b) {
                    h.inv_t += dt;
                    if h.inv_t >= 0.4 && !h.inv_fired {
                        h.inv_fired = true;
                        if let Some(k) = bound(key_map, "Goals / notes screen") {
                            h.taps.push((k, 2));
                        }
                    }
                } else if h.inv_t > 0.0 {
                    if !h.inv_fired {
                        if let Some(k) = bound(key_map, "Inventory screen") {
                            h.taps.push((k, 2));
                        }
                    }
                    h.inv_t = 0.0;
                    h.inv_fired = false;
                }
            }
            PadAction::ViewAmmo => {
                if crate::augwheel::takes(b) {
                    h.view_t = 0.0;
                    h.view_fired = true;
                } else if held(b) {
                    h.view_t += dt;
                    if h.view_t >= 0.35 && !h.view_fired {
                        h.view_fired = true;
                        if let Some(k) = bound(key_map, "Change ammo") {
                            h.taps.push((k, 2));
                        }
                    }
                } else if h.view_t > 0.0 {
                    if !h.view_fired {
                        let act = if crate::thirdperson::enabled() { "Third person view" } else { "Change ammo" };
                        if let Some(k) = bound(key_map, act) {
                            h.taps.push((k, 2));
                        }
                    }
                    h.view_t = 0.0;
                    h.view_fired = false;
                } else {
                    h.view_fired = false;
                }
            }
            PadAction::NextBelt => {
                if just(b) && !crate::augwheel::takes(b) {
                    out.scroll -= 1.0;
                }
            }
            PadAction::PrevBelt => {
                if just(b) && !crate::augwheel::takes(b) {
                    out.scroll += 1.0;
                }
            }
            PadAction::Menu => {
                if held(b) {
                    out.inputs.push(Input::Key(KeyCode::Escape));
                }
            }
            PadAction::AugWheel | PadAction::None => {}
        }
    }
    // Hotkey pages (augwheel.rs) use the D-pad when that mode is on.
    for k in crate::augwheel::hotkey_presses(&f) {
        if let Some(inp) = bound(key_map, k) {
            h.taps.push((inp, 2));
        }
    }
    // A light push of the left stick walks (Walk held), as an analog stick's half tilt does in console games.
    let m = f.ls.length();
    if m > 0.0 && m < 0.5 {
        if let Some(k) = bound(key_map, "Walk (hold)") {
            out.inputs.push(k);
        }
    }
    // The stick also presses the movement keys (swimming, the spy drone, ladders read them).
    for (cond, action) in [
        (f.ls.y > 0.35, "Move forward"),
        (f.ls.y < -0.35, "Move back"),
        (f.ls.x > 0.35, "Strafe right"),
        (f.ls.x < -0.35, "Strafe left"),
    ] {
        if cond {
            if let Some(k) = bound(key_map, action) {
                out.inputs.push(k);
            }
        }
    }
    out
}

/// Analog movement (forward, right) for walking, or None when the stick is centred.
pub fn move_axes() -> Option<Vec2> {
    let f = frame();
    (f.connected && f.ls.length() > 0.0).then_some(f.ls)
}

/// View turn this frame (yaw, pitch) in radians from the right stick: an exponent-2 curve, up to 220 deg/s
/// horizontally and 150 vertically at default sensitivity, scaled by Pad Look Sensitivity and inverted with
/// Invert Pad Look.
pub fn look_delta(dt: f32) -> Vec2 {
    let f = frame();
    // The aug wheel has the right stick while it's up.
    if !f.connected || crate::augwheel::open() {
        return Vec2::ZERO;
    }
    let m = f.rs.length();
    if m <= 0.0 {
        return Vec2::ZERO;
    }
    let curved = f.rs / m * m.powf(2.0);
    let sens = [0.5, 0.75, 1.0, 1.4, 1.9][(crate::settings::get("PadLookSensitivity") as usize).min(4)];
    let inv = if crate::settings::get("PadInvertLook") > 0.5 { -1.0 } else { 1.0 };
    // Aim slowdown (Enhancement): 40% slower while the crosshair is on an enemy.
    let slow = if crate::settings::get("PadAimSlowdown") > 0.5 && crate::hud::on_enemy() { 0.6 } else { 1.0 };
    Vec2::new(
        curved.x * 220f32.to_radians() * sens * slow * dt,
        curved.y * 150f32.to_radians() * sens * slow * inv * dt,
    )
}

/// A button's name in a glyph set (the Controller Bindings list, prompts without room for a glyph).
pub fn pad_name(kind: PadKind, b: PadBtn) -> String {
    let ps = kind == PadKind::PlayStation;
    match b {
        PadBtn::South => if ps { "Cross" } else { "A" },
        PadBtn::East => if ps { "Circle" } else { "B" },
        PadBtn::West => if ps { "Square" } else { "X" },
        PadBtn::North => if ps { "Triangle" } else { "Y" },
        PadBtn::LB => if ps { "L1" } else { "LB" },
        PadBtn::RB => if ps { "R1" } else { "RB" },
        PadBtn::LT => if ps { "L2" } else { "LT" },
        PadBtn::RT => if ps { "R2" } else { "RT" },
        PadBtn::Select => if ps { "Create" } else { "View" },
        PadBtn::Start => if ps { "Options" } else { "Menu" },
        PadBtn::L3 => if ps { "L3" } else { "Left Stick" },
        PadBtn::R3 => if ps { "R3" } else { "Right Stick" },
        PadBtn::Up => "D-Pad Up",
        PadBtn::Down => "D-Pad Down",
        PadBtn::Left => "D-Pad Left",
        PadBtn::Right => "D-Pad Right",
        PadBtn::Touch => "Touchpad",
    }
    .to_string()
}

// ------------------------------------------------------------------------------------------------ rumble

/// The gilrs pad in use (None when the DualSense on HID is the pad, or nothing is connected).
static ACTIVE: Mutex<Option<gilrs::GamepadId>> = Mutex::new(None);

/// Rumbles running: (low / big motor, high / small motor, seconds left, length).
static RUMBLE: Mutex<Vec<(f32, f32, f32, f32)>> = Mutex::new(Vec::new());

/// Classic rumble: both motors 0..1 for `secs`, fading out over the last half. Overlapping rumbles add up.
pub fn rumble(low: f32, high: f32, secs: f32) {
    if !connected() || secs <= 0.0 {
        return;
    }
    let mut g = RUMBLE.lock().unwrap();
    if g.len() < 16 {
        g.push((low.clamp(0.0, 1.0), high.clamp(0.0, 1.0), secs, secs));
    }
}

/// The motors' strength now.
pub fn rumble_now() -> (f32, f32) {
    let g = RUMBLE.lock().unwrap();
    let (mut lo, mut hi) = (0.0f32, 0.0f32);
    for &(l, h, left, len) in g.iter() {
        let k = (left / (len * 0.5)).min(1.0);
        lo += l * k;
        hi += h * k;
    }
    (lo.min(1.0), hi.min(1.0))
}

/// Update: age the rumbles; drive a gilrs pad's motors (force feedback) when it isn't the HID DualSense.
pub fn rumble_system(
    time: Res<Time<Real>>,
    gilrs: Option<NonSendMut<GilrsRes>>,
    mut effect: Local<Option<(gilrs::ff::Effect, u16, u16)>>,
) {
    let dt = time.delta_secs();
    {
        let mut g = RUMBLE.lock().unwrap();
        for r in g.iter_mut() {
            r.2 -= dt;
        }
        g.retain(|r| r.2 > 0.0);
    }
    // Paused on the menus: no rumble.
    let (lo, hi) = if crate::menus::open() { (0.0, 0.0) } else { rumble_now() };
    let Some(mut g) = gilrs else { return };
    let Some(id) = *ACTIVE.lock().unwrap() else {
        *effect = None;
        return;
    };
    let (ml, mh) = ((lo * 65535.0) as u16, (hi * 65535.0) as u16);
    // Rebuild the effect only when the strength moves noticeably (each build talks to the driver).
    let same = effect.as_ref().is_some_and(|(_, a, b)| a.abs_diff(ml) < 2000 && b.abs_diff(mh) < 2000);
    if same {
        return;
    }
    *effect = None;
    if ml == 0 && mh == 0 {
        return;
    }
    use gilrs::ff::{BaseEffect, BaseEffectType, EffectBuilder, Replay, Ticks};
    let replay = Replay { after: Ticks::from_ms(0), play_for: Ticks::from_ms(2000), with_delay: Ticks::from_ms(0) };
    let built = EffectBuilder::new()
        .add_effect(BaseEffect { kind: BaseEffectType::Strong { magnitude: ml }, scheduling: replay, ..Default::default() })
        .add_effect(BaseEffect { kind: BaseEffectType::Weak { magnitude: mh }, scheduling: replay, ..Default::default() })
        .gamepads(&[id])
        .finish(&mut g.0);
    if let Ok(e) = built {
        let _ = e.play();
        *effect = Some((e, ml, mh));
    }
}
