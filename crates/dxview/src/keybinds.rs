//! Key bindings and a quick in-game editor (Home).
//!
//! The game's systems keep reading the default keys (KeyCode::KeyW, MouseButton::Left, ...). This module sits
//! between the hardware and them: it tracks the real keyboard and mouse itself, then each frame rebuilds
//! `ButtonInput<KeyCode>` / `ButtonInput<MouseButton>` so a press of the key bound to an action shows up as
//! that action's default input. Keys that aren't any action's default (Escape, Enter, Backspace...) pass
//! straight through. Bindings are saved to keybinds.txt next to the executable.

use bevy::input::keyboard::{KeyboardFocusLost, KeyboardInput};
use bevy::input::mouse::MouseButtonInput;
use bevy::input::ButtonState;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Input {
    Key(KeyCode),
    Mouse(MouseButton),
}

use Input::{Key as K, Mouse as M};

/// Every rebindable action and the input the game code reads for it.
pub const ACTIONS: &[(&str, Input)] = &[
    ("Move forward", K(KeyCode::KeyW)),
    ("Move back", K(KeyCode::KeyS)),
    ("Strafe left", K(KeyCode::KeyA)),
    ("Strafe right", K(KeyCode::KeyD)),
    ("Jump / swim up", K(KeyCode::Space)),
    ("Crouch / swim down", K(KeyCode::KeyC)),
    ("Walk (hold)", K(KeyCode::ShiftLeft)),
    ("Lean left", K(KeyCode::KeyQ)),
    ("Lean right", K(KeyCode::KeyE)),
    ("Fire / throw", M(MouseButton::Left)),
    ("Use / pick up", M(MouseButton::Right)),
    ("Reload", K(KeyCode::KeyR)),
    ("Scope", K(KeyCode::KeyZ)),
    ("Scope (alt)", K(KeyCode::BracketLeft)),
    ("Laser sight", K(KeyCode::BracketRight)),
    ("Change ammo", K(KeyCode::KeyT)),
    ("Change ammo (alt)", K(KeyCode::Quote)),
    ("Belt 1", K(KeyCode::Digit1)),
    ("Belt 2", K(KeyCode::Digit2)),
    ("Belt 3", K(KeyCode::Digit3)),
    ("Belt 4", K(KeyCode::Digit4)),
    ("Belt 5", K(KeyCode::Digit5)),
    ("Belt 6", K(KeyCode::Digit6)),
    ("Belt 7", K(KeyCode::Digit7)),
    ("Belt 8", K(KeyCode::Digit8)),
    ("Belt 9", K(KeyCode::Digit9)),
    ("Belt 0 (keys)", K(KeyCode::Digit0)),
    ("Augmentation F3", K(KeyCode::F3)),
    ("Augmentation F4", K(KeyCode::F4)),
    ("Augmentation F5", K(KeyCode::F5)),
    ("Augmentation F6", K(KeyCode::F6)),
    ("Augmentation F7", K(KeyCode::F7)),
    ("Augmentation F8", K(KeyCode::F8)),
    ("Augmentation F9", K(KeyCode::F9)),
    ("Augmentation F10", K(KeyCode::F10)),
    ("Augmentation F11", K(KeyCode::F11)),
    ("Augmentation F12 (light)", K(KeyCode::F12)),
    ("Inventory screen", K(KeyCode::F1)),
    ("Goals / notes screen", K(KeyCode::F2)),
    ("Quick Save", K(KeyCode::End)),
    ("Quick Load", K(KeyCode::Backspace)),
    ("Brightness cycle", K(KeyCode::Minus)),
    ("Fly mode (debug)", K(KeyCode::KeyV)),
    ("Fly down (debug)", K(KeyCode::ControlLeft)),
    ("Spawn menu (debug)", K(KeyCode::Insert)),
    ("Unlock all doors (debug)", K(KeyCode::PageUp)),
    ("Debug overlay", K(KeyCode::PageDown)),
    ("Third person view", K(KeyCode::KeyH)),
    ("Swap shoulder (third person)", K(KeyCode::KeyJ)),
];

/// Opens and closes the editor; not rebindable so it can't be lost (F10 is an augmentation key, as in the original).
pub const MENU_KEY: KeyCode = KeyCode::Home;

#[derive(Resource)]
pub struct Bindings {
    /// Physical input for each entry of ACTIONS.
    pub map: Vec<Input>,
    pub open: bool,
    /// Waiting for a key for this action; capture starts after a short delay so the click that chose it
    /// isn't taken as the new binding.
    waiting: Option<(usize, f32)>,
    built: bool,
    /// The real keyboard and mouse.
    pub keys: ButtonInput<KeyCode>,
    pub buttons: ButtonInput<MouseButton>,
    /// A menu (the inventory screen) has the input: the game sees none of it.
    pub capture: bool,
    /// This frame's key presses, repeats included, with the text each one types (text editing).
    pub typed: Vec<(KeyCode, Option<String>)>,
    /// The keyboard and mouse alone (`keys` / `buttons` are these merged with what the gamepad presses).
    kb_keys: ButtonInput<KeyCode>,
    kb_buttons: ButtonInput<MouseButton>,
}

impl Default for Bindings {
    fn default() -> Self {
        let mut b = Bindings {
            map: ACTIONS.iter().map(|a| a.1).collect(),
            open: false,
            waiting: None,
            built: false,
            keys: default(),
            buttons: default(),
            capture: false,
            typed: Vec::new(),
            kb_keys: default(),
            kb_buttons: default(),
        };
        b.load();
        b
    }
}

fn save_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("keybinds.txt")))
        .unwrap_or_else(|| PathBuf::from("keybinds.txt"))
}

/// Inputs that can be bound (and parsed back from the save file).
fn candidates() -> Vec<Input> {
    use KeyCode::*;
    let keys = [
        KeyA,
        KeyB,
        KeyC,
        KeyD,
        KeyE,
        KeyF,
        KeyG,
        KeyH,
        KeyI,
        KeyJ,
        KeyK,
        KeyL,
        KeyM,
        KeyN,
        KeyO,
        KeyP,
        KeyQ,
        KeyR,
        KeyS,
        KeyT,
        KeyU,
        KeyV,
        KeyW,
        KeyX,
        KeyY,
        KeyZ,
        Digit0,
        Digit1,
        Digit2,
        Digit3,
        Digit4,
        Digit5,
        Digit6,
        Digit7,
        Digit8,
        Digit9,
        F1,
        F2,
        F3,
        F4,
        F5,
        F6,
        F7,
        F8,
        F9,
        F11,
        F12,
        Space,
        Tab,
        CapsLock,
        ShiftLeft,
        ShiftRight,
        ControlLeft,
        ControlRight,
        AltLeft,
        AltRight,
        Backquote,
        Minus,
        Equal,
        BracketLeft,
        BracketRight,
        Backslash,
        Semicolon,
        Quote,
        Comma,
        Period,
        Slash,
        ArrowUp,
        ArrowDown,
        ArrowLeft,
        ArrowRight,
        Insert,
        Delete,
        Home,
        End,
        PageUp,
        PageDown,
        Numpad0,
        Numpad1,
        Numpad2,
        Numpad3,
        Numpad4,
        Numpad5,
        Numpad6,
        Numpad7,
        Numpad8,
        Numpad9,
        NumpadAdd,
        NumpadSubtract,
        NumpadMultiply,
        NumpadDivide,
        NumpadDecimal,
        NumpadEnter,
        Enter,
        Backspace,
    ];
    let mut v: Vec<Input> = keys.into_iter().map(Input::Key).collect();
    v.extend(
        [
            MouseButton::Left,
            MouseButton::Right,
            MouseButton::Middle,
            MouseButton::Back,
            MouseButton::Forward,
        ]
        .map(Input::Mouse),
    );
    v
}

impl Bindings {
    fn load(&mut self) {
        let Ok(text) = std::fs::read_to_string(save_path()) else {
            return;
        };
        let all = candidates();
        for line in text.lines() {
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            let (name, value) = (name.trim(), value.trim());
            let Some(i) = ACTIONS.iter().position(|a| a.0 == name) else {
                continue;
            };
            if let Some(inp) = all.iter().find(|c| format!("{c:?}") == value) {
                self.map[i] = *inp;
            }
        }
    }

    fn save(&self) {
        let text: String = ACTIONS
            .iter()
            .zip(&self.map)
            .map(|(a, b)| format!("{} = {b:?}\n", a.0))
            .collect();
        if let Err(e) = std::fs::write(save_path(), text) {
            warn!("could not save key bindings: {e}");
        }
    }

    /// The physical input bound to an action was pressed this frame (works while a menu has the input).
    pub fn action_just_pressed(&self, name: &str) -> bool {
        match ACTIONS
            .iter()
            .position(|a| a.0 == name)
            .map(|i| self.map[i])
        {
            Some(Input::Key(k)) => self.keys.just_pressed(k),
            Some(Input::Mouse(m)) => self.buttons.just_pressed(m),
            None => false,
        }
    }

    /// The game inputs a physical input produces.
    fn outputs(&self, phys: Input) -> Vec<Input> {
        let mut out: Vec<Input> = ACTIONS
            .iter()
            .zip(&self.map)
            .filter(|(_, b)| **b == phys)
            .map(|(a, _)| a.1)
            .collect();
        // Keys no action uses (Escape, Enter, Backspace...) pass through unchanged.
        if out.is_empty() && !ACTIONS.iter().any(|a| a.1 == phys) {
            out.push(phys);
        }
        out
    }
}

/// Replace every binding (the Keyboard/Mouse settings screen) and save them.
pub fn set_map(map: &[Input]) {
    if let Ok(mut p) = PENDING_MAP.lock() {
        *p = Some(map.to_vec());
    }
}
static PENDING_MAP: std::sync::Mutex<Option<Vec<Input>>> = std::sync::Mutex::new(None);

pub fn input_name(i: Input) -> String {
    match i {
        Input::Mouse(MouseButton::Left) => "Left Mouse".into(),
        Input::Mouse(MouseButton::Right) => "Right Mouse".into(),
        Input::Mouse(MouseButton::Middle) => "Middle Mouse".into(),
        Input::Mouse(b) => format!("Mouse {b:?}"),
        Input::Key(k) => {
            let s = format!("{k:?}");
            if let Some(r) = s.strip_prefix("Key") {
                r.to_string()
            } else if let Some(r) = s.strip_prefix("Digit") {
                r.to_string()
            } else {
                s
            }
        }
    }
}

/// PreUpdate, after Bevy's input systems: track the real devices (keyboard, mouse and the gamepad's presses, see
/// gamepad.rs), then rewrite the game's input state.
#[allow(clippy::too_many_arguments)]
/// The cursor is free this frame (menus, screens, conversation choices): the pad acts as a mouse.
pub static UI_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn remap_system(
    mut b: ResMut<Bindings>,
    mut kb: MessageReader<KeyboardInput>,
    mut focus: MessageReader<KeyboardFocusLost>,
    mut mouse_ev: MessageReader<MouseButtonInput>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
    time: Res<Time<Real>>,
    mut scroll: ResMut<bevy::input::mouse::AccumulatedMouseScroll>,
    mut windows: Query<(&mut Window, &CursorOptions)>,
    motion: Res<bevy::input::mouse::AccumulatedMouseMotion>,
    mut pad_cursor: Local<Option<Vec2>>,
) {
    let b = &mut *b;
    if let Some(m) = PENDING_MAP.lock().ok().and_then(|mut p| p.take()) {
        if m.len() == b.map.len() {
            b.map = m;
            b.save();
        }
    }
    b.kb_keys.clear();
    b.kb_buttons.clear();
    b.typed.clear();
    let mut kbm = false;
    for e in kb.read() {
        kbm = true;
        if e.state == ButtonState::Pressed {
            b.typed.push((e.key_code, e.text.as_ref().map(|t| t.to_string())));
        }
        match e.state {
            ButtonState::Pressed => b.kb_keys.press(e.key_code),
            ButtonState::Released => b.kb_keys.release(e.key_code),
        }
    }
    if focus.read().count() > 0 {
        b.kb_keys.release_all();
    }
    for e in mouse_ev.read() {
        kbm = true;
        match e.state {
            ButtonState::Pressed => b.kb_buttons.press(e.button),
            ButtonState::Released => b.kb_buttons.release(e.button),
        }
    }
    if kbm {
        crate::gamepad::note_kbm();
    }
    // The gamepad: in menus (cursor free) it's a mouse; in play its buttons press the actions' bound inputs.
    let ui = windows.iter().next().is_some_and(|(_, c)| c.grab_mode != CursorGrabMode::Locked) || b.open;
    UI_MODE.store(ui, std::sync::atomic::Ordering::Relaxed);
    let snap_ok = crate::snap::swap_frame();
    let pad = crate::gamepad::outputs(&b.map, time.delta_secs().min(0.1), ui, snap_ok && ui);
    if pad.scroll != 0.0 {
        scroll.delta.y += pad.scroll;
    }
    // The pad's cursor is kept here, not read back from the window: after a warp the OS still delivers move
    // events from before it, so reading the position back made the cursor hop between old and new spots. The
    // real mouse moving (raw motion, which warps don't make) hands the cursor back to it.
    if !ui || motion.delta != Vec2::ZERO {
        *pad_cursor = None;
    }
    if let Some((mut w, _)) = windows.iter_mut().next() {
        if ui && (pad.cursor != Vec2::ZERO || pad.snap.is_some()) {
            let (ww, wh) = (w.width(), w.height());
            let at = pad_cursor.or(w.cursor_position()).unwrap_or(Vec2::new(ww / 2.0, wh / 2.0));
            let k = 1.0 / w.scale_factor();
            let to = match pad.snap {
                Some(d) => crate::snap::pick(at, d).unwrap_or(at),
                None => at + pad.cursor * k,
            };
            *pad_cursor = Some(to.clamp(Vec2::ZERO, Vec2::new(ww - 1.0, wh - 1.0)));
        }
        if let Some(p) = *pad_cursor {
            if w.cursor_position().is_none_or(|c| c.distance(p) > 0.25) {
                w.set_cursor_position(Some(p));
            }
        }
    }
    // Merge: an input is down while the keyboard / mouse or the pad holds it.
    let pad_keys: Vec<KeyCode> = pad.inputs.iter().filter_map(|i| if let Input::Key(k) = i { Some(*k) } else { None }).collect();
    let pad_btns: Vec<MouseButton> = pad.inputs.iter().filter_map(|i| if let Input::Mouse(m) = i { Some(*m) } else { None }).collect();
    b.keys.clear();
    b.buttons.clear();
    let mut all_k: Vec<KeyCode> = b.kb_keys.get_pressed().chain(b.kb_keys.get_just_released()).chain(b.kb_keys.get_just_pressed()).copied().collect();
    all_k.extend(pad_keys.iter().copied());
    all_k.extend(b.keys.get_pressed().copied());
    all_k.sort_by_key(|k| format!("{k:?}"));
    all_k.dedup();
    for k in all_k {
        let kb_tap = b.kb_keys.just_pressed(k) && !b.kb_keys.pressed(k);
        let want = b.kb_keys.pressed(k) || pad_keys.contains(&k);
        let was = b.keys.pressed(k);
        if (want && !was) || kb_tap {
            b.keys.press(k);
        }
        if (!want && was) || (kb_tap && !want) {
            b.keys.release(k);
        }
    }
    let mut all_m: Vec<MouseButton> = b.kb_buttons.get_pressed().chain(b.kb_buttons.get_just_released()).chain(b.kb_buttons.get_just_pressed()).copied().collect();
    all_m.extend(pad_btns.iter().copied());
    all_m.extend(b.buttons.get_pressed().copied());
    all_m.sort_by_key(|m| format!("{m:?}"));
    all_m.dedup();
    for m in all_m {
        let kb_tap = b.kb_buttons.just_pressed(m) && !b.kb_buttons.pressed(m);
        let want = b.kb_buttons.pressed(m) || pad_btns.contains(&m);
        let was = b.buttons.pressed(m);
        if (want && !was) || kb_tap {
            b.buttons.press(m);
        }
        if (!want && was) || (kb_tap && !want) {
            b.buttons.release(m);
        }
    }
    keys.reset_all();
    buttons.reset_all();
    if b.open || b.capture {
        // The editor or a menu has the input; the game sees nothing.
        return;
    }
    // Held inputs first, then fresh presses and releases, so a game input pressed by two physical keys keeps
    // the strongest state.
    let mut phys: Vec<(Input, bool, bool, bool)> = Vec::new();
    for k in b.keys.get_pressed().chain(b.keys.get_just_released()) {
        phys.push((
            Input::Key(*k),
            b.keys.pressed(*k),
            b.keys.just_pressed(*k),
            b.keys.just_released(*k),
        ));
    }
    for m in b.buttons.get_pressed().chain(b.buttons.get_just_released()) {
        phys.push((
            Input::Mouse(*m),
            b.buttons.pressed(*m),
            b.buttons.just_pressed(*m),
            b.buttons.just_released(*m),
        ));
    }
    for (p, pressed, just_pressed, just_released) in phys {
        for out in b.outputs(p) {
            match out {
                Input::Key(k) => apply(&mut keys, k, pressed, just_pressed, just_released),
                Input::Mouse(m) => apply(&mut buttons, m, pressed, just_pressed, just_released),
            }
        }
    }
}

fn apply<T: Copy + Eq + std::hash::Hash + Send + Sync + 'static>(
    inp: &mut ButtonInput<T>,
    k: T,
    pressed: bool,
    just_pressed: bool,
    just_released: bool,
) {
    if pressed {
        let already = inp.pressed(k);
        inp.press(k);
        if !just_pressed && !already {
            inp.clear_just_pressed(k);
        }
    } else if just_released && !inp.pressed(k) {
        inp.press(k);
        inp.clear_just_pressed(k);
        inp.release(k);
    }
}

#[derive(Component)]
pub struct BindMenuRoot;
#[derive(Component)]
pub struct BindButton(usize);
#[derive(Component)]
pub struct BindLabel(usize);
#[derive(Component)]
pub struct ResetButton;
#[derive(Component)]
pub struct CloseButton;

const BG: Color = Color::srgba(0.06, 0.07, 0.09, 0.94);
const CELL: Color = Color::srgba(0.12, 0.14, 0.18, 0.95);
const CELL_HOVER: Color = Color::srgba(0.22, 0.26, 0.32, 0.95);
const CELL_WAIT: Color = Color::srgba(0.45, 0.38, 0.12, 0.95);

fn build(commands: &mut Commands) {
    let font = |size: f32| TextFont {
        font_size: bevy::text::FontSize::Px(size),
        ..default()
    };
    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(10.0),
                right: Val::Percent(10.0),
                top: Val::Percent(6.0),
                bottom: Val::Percent(6.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(14.0)),
                row_gap: Val::Px(8.0),
                ..default()
            },
            BackgroundColor(BG),
            Visibility::Hidden,
            GlobalZIndex(50),
            BindMenuRoot,
        ))
        .id();
    let title = commands
        .spawn((Text::new("KEY BINDINGS  -  click an action, then press a key or mouse button.  Esc cancels / closes, Home closes."), font(16.0), TextColor(Color::srgb(0.9, 0.92, 0.95))))
        .id();
    let grid = commands
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            flex_wrap: FlexWrap::Wrap,
            flex_grow: 1.0,
            column_gap: Val::Px(10.0),
            row_gap: Val::Px(4.0),
            align_content: AlignContent::FlexStart,
            ..default()
        })
        .id();
    commands.entity(root).add_children(&[title, grid]);
    for (i, (name, _)) in ACTIONS.iter().enumerate() {
        let row = commands
            .spawn((
                Button,
                Node {
                    width: Val::Px(330.0),
                    height: Val::Px(26.0),
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    padding: UiRect::horizontal(Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(CELL),
                BindButton(i),
            ))
            .id();
        let n = commands
            .spawn((
                Text::new(*name),
                font(14.0),
                TextColor(Color::srgb(0.8, 0.82, 0.85)),
                Pickable::IGNORE,
            ))
            .id();
        let k = commands
            .spawn((
                Text::new(""),
                font(14.0),
                TextColor(Color::srgb(0.55, 0.85, 1.0)),
                Pickable::IGNORE,
                BindLabel(i),
            ))
            .id();
        commands.entity(row).add_children(&[n, k]);
        commands.entity(grid).add_child(row);
    }
    let bar = commands
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(12.0),
            ..default()
        })
        .id();
    for (label, reset) in [("Reset to defaults", true), ("Close (Home)", false)] {
        let mut e = commands.spawn((
            Button,
            Node {
                padding: UiRect::axes(Val::Px(14.0), Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(CELL),
        ));
        if reset {
            e.insert(ResetButton);
        } else {
            e.insert(CloseButton);
        }
        let id = e.id();
        let t = commands
            .spawn((
                Text::new(label),
                font(14.0),
                TextColor(Color::WHITE),
                Pickable::IGNORE,
            ))
            .id();
        commands.entity(id).add_child(t);
        commands.entity(bar).add_child(id);
    }
    commands.entity(root).add_child(bar);
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn bind_menu_system(
    mut commands: Commands,
    time: Res<Time>,
    mut b: ResMut<Bindings>,
    mut cursor: Single<&mut CursorOptions>,
    mut root: Query<&mut Visibility, With<BindMenuRoot>>,
    mut rows: Query<(&Interaction, &BindButton, &mut BackgroundColor), Without<ResetButton>>,
    mut labels: Query<(&BindLabel, &mut Text, &mut TextColor)>,
    reset: Query<&Interaction, (Changed<Interaction>, With<ResetButton>)>,
    close: Query<&Interaction, (Changed<Interaction>, With<CloseButton>)>,
    mut hud: ResMut<crate::movers::HudMessage>,
) {
    let b = &mut *b;
    if !b.built {
        b.built = true;
        build(&mut commands);
    }
    let menu_key = b.keys.just_pressed(MENU_KEY);
    let close_clicked = b.open && close.iter().any(|i| *i == Interaction::Pressed);
    if menu_key || close_clicked {
        b.open = menu_key && !b.open;
        b.waiting = None;
        if b.open {
            cursor.visible = true;
            cursor.grab_mode = CursorGrabMode::None;
        }
    }
    for mut v in &mut root {
        let want = if b.open {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *v != want {
            *v = want;
        }
    }
    if !b.open {
        return;
    }
    if reset.iter().any(|i| *i == Interaction::Pressed) {
        b.map = ACTIONS.iter().map(|a| a.1).collect();
        b.waiting = None;
        b.save();
        hud.text = "Key bindings reset to defaults".into();
        hud.time_left = 2.0;
    }
    // Capture a new binding.
    if let Some((i, delay)) = b.waiting {
        if delay > 0.0 {
            b.waiting = Some((i, delay - time.delta_secs()));
        } else if b.keys.just_pressed(KeyCode::Escape) {
            b.waiting = None;
        } else {
            let all = candidates();
            let got = all.into_iter().find(|c| match c {
                Input::Key(k) => b.keys.just_pressed(*k),
                Input::Mouse(m) => b.buttons.just_pressed(*m),
            });
            if let Some(inp) = got {
                b.map[i] = inp;
                b.waiting = None;
                b.save();
            }
        }
    } else if b.keys.just_pressed(KeyCode::Escape) {
        b.open = false;
    }
    for (inter, row, mut bg) in &mut rows {
        if *inter == Interaction::Pressed && b.waiting.is_none() {
            b.waiting = Some((row.0, 0.15));
        }
        let c = if b.waiting.map(|w| w.0) == Some(row.0) {
            CELL_WAIT
        } else if *inter == Interaction::Hovered {
            CELL_HOVER
        } else {
            CELL
        };
        if bg.0 != c {
            bg.0 = c;
        }
    }
    for (l, mut t, mut col) in &mut labels {
        let s = if b.waiting.map(|w| w.0) == Some(l.0) {
            "press a key...".to_string()
        } else {
            input_name(b.map[l.0])
        };
        // Two actions on one input both fire: shown in orange.
        let clash = b.map.iter().filter(|m| **m == b.map[l.0]).count() > 1;
        let c = if clash {
            Color::srgb(1.0, 0.6, 0.2)
        } else {
            Color::srgb(0.55, 0.85, 1.0)
        };
        if t.0 != s {
            t.0 = s;
        }
        if col.0 != c {
            col.0 = c;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebound_key_drives_the_action_and_old_key_goes_quiet() {
        let mut b = Bindings {
            map: ACTIONS.iter().map(|a| a.1).collect(),
            open: false,
            waiting: None,
            built: true,
            keys: default(),
            buttons: default(),
            kb_keys: default(),
            kb_buttons: default(),
            capture: false,
            typed: Vec::new(),
        };
        let jump = ACTIONS
            .iter()
            .position(|a| a.0 == "Jump / swim up")
            .unwrap();
        b.map[jump] = Input::Key(KeyCode::KeyK);
        assert_eq!(
            b.outputs(Input::Key(KeyCode::KeyK)),
            vec![Input::Key(KeyCode::Space)]
        );
        assert!(b.outputs(Input::Key(KeyCode::Space)).is_empty());
        // Keys no action uses pass through.
        assert_eq!(
            b.outputs(Input::Key(KeyCode::Escape)),
            vec![Input::Key(KeyCode::Escape)]
        );
        // A mouse button can drive a keyboard action.
        let reload = ACTIONS.iter().position(|a| a.0 == "Reload").unwrap();
        b.map[reload] = Input::Mouse(MouseButton::Middle);
        assert_eq!(
            b.outputs(Input::Mouse(MouseButton::Middle)),
            vec![Input::Key(KeyCode::KeyR)]
        );
    }

    #[test]
    fn held_and_fresh_presses_rebuild_correctly() {
        let mut inp = ButtonInput::<KeyCode>::default();
        apply(&mut inp, KeyCode::KeyW, true, false, false);
        assert!(inp.pressed(KeyCode::KeyW) && !inp.just_pressed(KeyCode::KeyW));
        apply(&mut inp, KeyCode::KeyR, true, true, false);
        assert!(inp.just_pressed(KeyCode::KeyR));
        apply(&mut inp, KeyCode::KeyE, false, false, true);
        assert!(inp.just_released(KeyCode::KeyE) && !inp.pressed(KeyCode::KeyE));
    }
}
