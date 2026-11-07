//! Touch controls (Android; DXVIEW_TOUCH=1 shows them on the desktop with the mouse as one finger, for testing).
//!
//! In play:
//! - the left half is a floating move stick: it appears where the thumb lands (a light push walks, a full push runs);
//! - the right half looks: drag to turn; a quick tap fires once, and a finger held still fires until it lifts (it
//!   can still drag to aim);
//! - buttons over the right thumb: Use (lit when something is in reach), Jump, Crouch (a toggle), Reload, Zoom; two
//!   Lean buttons on the left edge; Menu and Inventory at the top (plus Chat and Scores in multiplayer);
//! - the HUD is tappable: a belt slot selects it, an augmentation icon switches it, the health figure opens the
//!   Health screen, the compass the Goals screen, the ammo display changes ammo, the log the Logs screen.
//!
//! In menus, the inventory, computers and conversation choices a finger is the mouse: a tap clicks, a drag drags,
//! a long press is a right click and two fingers scroll. A keyboard button brings up the on-screen keyboard. In a
//! conversation a tap goes on to the next line, and in a cut scene it skips.
//!
//! The touch state is worked out in PreUpdate (`touch_system`, before keybinds::remap_system) and handed over as
//! inputs (the actions' bound keys and buttons, like the gamepad's), a move vector (gamepad::move_axes) and a look
//! delta (fly). `draw_system` draws the stick and the buttons as small UI images, redrawn only when they change.
use crate::keybinds::Input;
use crate::uicanvas::{Canvas, HAlign, VAlign};
use bevy::input::mouse::MouseButtonInput;
use bevy::input::touch::Touches;
use bevy::input::ButtonState;
use bevy::prelude::*;
use std::sync::Mutex;

/// Touch controls are on: Android, or the desktop test switch.
pub fn enabled() -> bool {
    crate::platform::android() || emulating()
}

/// The desktop test: the mouse plays one finger (DXVIEW_TOUCH=1).
pub fn emulating() -> bool {
    !crate::platform::android() && env_on!("DXVIEW_TOUCH")
}

/// A button on the screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Btn {
    Use,
    Jump,
    Crouch,
    Reload,
    Zoom,
    LeanL,
    LeanR,
    Menu,
    Inv,
    Keyboard,
    Chat,
    Scores,
}

impl Btn {
    fn label(self) -> &'static str {
        match self {
            Btn::Use => "USE",
            Btn::Jump => "JUMP",
            Btn::Crouch => "DUCK",
            Btn::Reload => "RELOAD",
            Btn::Zoom => "ZOOM",
            Btn::LeanL => "< LEAN",
            Btn::LeanR => "LEAN >",
            Btn::Menu => "MENU",
            Btn::Inv => "ITEMS",
            Btn::Keyboard => "KEYS",
            Btn::Chat => "CHAT",
            Btn::Scores => "SCORES",
        }
    }
    /// The action it presses (keybinds::ACTIONS name), and whether it's held (else a tap).
    fn action(self) -> Option<(&'static str, bool)> {
        Some(match self {
            Btn::Use => ("Use / pick up", true),
            Btn::Jump => ("Jump / swim up", true),
            Btn::Crouch => ("Crouch / swim down", true),
            Btn::Reload => ("Reload", false),
            Btn::Zoom => ("Scope", false),
            Btn::LeanL => ("Lean left", true),
            Btn::LeanR => ("Lean right", true),
            Btn::Inv => ("Inventory screen", false),
            Btn::Chat => ("Chat (multiplayer)", false),
            Btn::Scores => ("Classic / Modern (scores in multiplayer)", true),
            Btn::Menu | Btn::Keyboard => return None,
        })
    }
}

/// What a tap on the HUD does.
#[derive(Clone, Copy, PartialEq, Debug)]
enum HudTap {
    Belt(usize),
    Aug(KeyCode),
    Hit,
    Compass,
    Ammo,
    Log,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Role {
    Stick { origin: Vec2 },
    Look { firing: bool },
    Button(Btn),
    Hud(HudTap),
    /// A finger in a menu / screen: `pressed` once it's dragging (left held), `long` once it was a long press.
    Ui { pressed: bool, long: bool },
    /// A second finger in a menu: scrolls.
    Scroll,
}

#[derive(Clone, Copy, Debug)]
struct Finger {
    id: u64,
    start: Vec2,
    pos: Vec2,
    prev: Vec2,
    t0: f32,
    travel: f32,
    role: Role,
}

/// A button as laid out this frame: centre, radius (logical pixels), lit, held.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Placed {
    pub btn: Btn,
    pub at: Vec2,
    pub r: f32,
    pub lit: bool,
    pub down: bool,
}

#[derive(Default)]
struct State {
    fingers: Vec<Finger>,
    /// This frame's outputs.
    move_axes: Vec2,
    look: Vec2,
    held: Vec<Input>,
    taps: Vec<(Input, u8)>,
    cursor: Option<Vec2>,
    scroll: f32,
    /// The crouch toggle.
    crouch_on: bool,
    /// The layout drawn (draw_system) and used for hits (touch_system).
    buttons: Vec<Placed>,
    /// The stick's base and knob while a finger holds it.
    stick: Option<(Vec2, Vec2, f32)>,
    /// A screen to open (inventory_screen takes it).
    open_screen: Option<usize>,
    keyboard: bool,
    /// Last time a finger touched (the overlay fades out while a controller or keyboard is used instead).
    last_touch: f32,
    now: f32,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::default))
}

/// The HUD windows as they were last placed (classic_hud records them): window name, logical rect (x, y, w, h)
/// and logical pixels per HUD unit.
static HUD: Mutex<Vec<(&'static str, Rect, f32)>> = Mutex::new(Vec::new());
/// The augmentation icons in the active column (HUD units inside its window) and their hot keys.
static AUG_ICONS: Mutex<Vec<(Rect, KeyCode)>> = Mutex::new(Vec::new());

/// classic_hud: a window placed this frame (name as in classic_hud's Win, logical rect, logical px per unit).
pub fn note_hud(name: &'static str, rect: Option<Rect>, unit: f32) {
    if !enabled() {
        return;
    }
    if let Ok(mut h) = HUD.lock() {
        h.retain(|w| w.0 != name);
        if let Some(r) = rect {
            h.push((name, r, unit));
        }
    }
}

/// classic_hud: the augmentation icons drawn in the active column (HUD units, window-local) with their F-keys.
pub fn note_aug_icons(icons: Vec<(Rect, KeyCode)>) {
    if enabled() {
        if let Ok(mut a) = AUG_ICONS.lock() {
            *a = icons;
        }
    }
}

fn hud_rect(name: &str) -> Option<(Rect, f32)> {
    HUD.lock().ok()?.iter().find(|w| w.0 == name).map(|w| (w.1, w.2))
}

/// gamepad::move_axes: the stick's push (x right, y forward), 0..1.
pub fn move_axes() -> Option<Vec2> {
    if !enabled() {
        return None;
    }
    with(|s| (s.move_axes.length() > 0.0).then_some(s.move_axes))
}

/// fly(): the view turn this frame (yaw, pitch) in radians from dragging on the look side.
pub fn look_delta() -> Vec2 {
    if !enabled() {
        return Vec2::ZERO;
    }
    let sens = [0.5, 0.75, 1.0, 1.4, 2.0][(setting!("TouchLook") as usize).min(4)];
    let inv = if crate::settings::invert_mouse() { -1.0 } else { 1.0 };
    #[allow(unused_mut)]
    let mut turn = with(|s| {
        let d = std::mem::take(&mut s.look);
        Vec2::new(d.x * 0.0045 * sens, -inv * d.y * 0.0045 * sens)
    });
    // Gyro Aim: the phone's own turn, on while playing (not in a screen).
    #[cfg(target_os = "android")]
    {
        let ui = crate::keybinds::UI_MODE.load(std::sync::atomic::Ordering::Relaxed);
        let on = setting!("GyroAim") > 0.5 && !ui;
        crate::android::gyro_enable(on);
        if on {
            let g = [0.5, 0.75, 1.0, 1.4, 2.0][(setting!("GyroSensitivity") as usize).min(4)];
            let t = crate::android::gyro_take() * g;
            turn += Vec2::new(t.x, inv * t.y);
        }
    }
    turn
}

/// keybinds::remap_system: the inputs held by touch this frame (taps last two frames: a press, then a release),
/// the cursor (menus: a finger is the mouse) and scrolling.
pub fn outputs() -> (Vec<Input>, Option<Vec2>, f32) {
    if !enabled() {
        return (Vec::new(), None, 0.0);
    }
    with(|s| {
        let mut v = s.held.clone();
        s.taps.retain_mut(|t| {
            v.push(t.0);
            t.1 = t.1.saturating_sub(1);
            t.1 > 0
        });
        (v, s.cursor, std::mem::take(&mut s.scroll))
    })
}

/// inventory_screen: a persona screen a HUD tap asked for.
pub fn take_open_screen() -> Option<usize> {
    if !enabled() {
        return None;
    }
    with(|s| s.open_screen.take())
}

/// The overlay's opacity: full while fingers are used, faded while a controller or keyboard plays.
fn overlay_alpha(s: &State) -> f32 {
    let pad = crate::gamepad::pad_in_use();
    if pad && s.now - s.last_touch > 3.0 {
        0.0
    } else {
        [0.35, 0.5, 0.7, 0.9][(setting!("TouchOpacity") as usize).min(3)]
    }
}

fn bound(map: &[Input], action: &str) -> Option<Input> {
    crate::keybinds::ACTIONS.iter().position(|a| a.0 == action).and_then(|i| map.get(i).copied())
}

// ---------------------------------------------------------------------------------------------------------------
// Layout.

/// The buttons for this frame (play or a screen), placed round the HUD.
fn layout(w: f32, h: f32, ui: bool) -> Vec<Placed> {
    let size = [0.85, 1.0, 1.2][(setting!("TouchSize") as usize).min(2)];
    let b = h.min(w * 0.6) * 0.07 * size;
    let mut v = Vec::new();
    let mut put = |btn: Btn, at: Vec2, r: f32| v.push(Placed { btn, at, r, lit: false, down: false });
    if ui {
        // A screen: only the keyboard key, bottom-left.
        put(Btn::Keyboard, Vec2::new(1.2 * b, h - 1.2 * b), 0.8 * b);
        return v;
    }
    // The right thumb's cluster sits above the object belt (bottom right).
    let belt_top = hud_rect("Belt").map_or(h * 0.86, |(r, _)| r.min.y);
    let a = Vec2::new(w - 1.5 * b, belt_top - 1.4 * b);
    put(Btn::Jump, a, 1.1 * b);
    put(Btn::Use, a + Vec2::new(-2.6 * b, 0.25 * b), 1.0 * b);
    put(Btn::Crouch, a + Vec2::new(-0.2 * b, -2.5 * b), 0.85 * b);
    put(Btn::Reload, a + Vec2::new(-2.4 * b, -2.3 * b), 0.75 * b);
    put(Btn::Zoom, a + Vec2::new(-4.7 * b, 0.5 * b), 0.7 * b);
    // Lean on the left edge, under the health figure and the compass.
    let left_bottom = ["Hit", "Compass"].iter().filter_map(|n| hud_rect(n)).map(|(r, _)| r.max.y).fold(h * 0.3, f32::max);
    let ly = (left_bottom + 1.4 * b).min(h * 0.55);
    put(Btn::LeanL, Vec2::new(1.1 * b, ly), 0.7 * b);
    put(Btn::LeanR, Vec2::new(2.9 * b, ly), 0.7 * b);
    // Menu and the inventory at the top, left of the augmentation column.
    let right = hud_rect("Active").filter(|(r, _)| r.width() > 1.0).map_or(w, |(r, _)| r.min.x);
    let ty = 0.9 * b;
    put(Btn::Menu, Vec2::new(right - 1.0 * b, ty), 0.65 * b);
    put(Btn::Inv, Vec2::new(right - 2.6 * b, ty), 0.65 * b);
    if crate::netplay::active() {
        put(Btn::Chat, Vec2::new(right - 4.2 * b, ty), 0.65 * b);
        put(Btn::Scores, Vec2::new(right - 5.8 * b, ty), 0.65 * b);
    }
    v
}

fn button_at(buttons: &[Placed], p: Vec2) -> Option<Btn> {
    buttons.iter().find(|b| b.at.distance(p) <= b.r * 1.15).map(|b| b.btn)
}

/// A HUD tap target under `p`.
fn hud_at(p: Vec2) -> Option<HudTap> {
    let inside = |name: &str| hud_rect(name).filter(|(r, _)| r.contains(p));
    if let Some((r, unit)) = inside("Belt") {
        // HUDObjectBelt's slots (classic_hud slot_rect): 1-9 from the left 51 units apart from x 10, then 0.
        let x = (p.x - r.min.x) / unit.max(0.01);
        let slot = if x >= 10.0 + 9.0 * 51.0 {
            0
        } else {
            (((x - 10.0) / 51.0).floor() as i32 + 1).clamp(1, 9) as usize
        };
        return Some(HudTap::Belt(slot));
    }
    if let Some((r, unit)) = inside("Active") {
        let local = (p - r.min) / unit.max(0.01);
        if let Some(k) = AUG_ICONS.lock().ok().and_then(|a| a.iter().find(|i| i.0.contains(local)).map(|i| i.1)) {
            return Some(HudTap::Aug(k));
        }
        return None;
    }
    if inside("Hit").is_some() {
        return Some(HudTap::Hit);
    }
    if inside("Compass").is_some() {
        return Some(HudTap::Compass);
    }
    if inside("Ammo").is_some() {
        return Some(HudTap::Ammo);
    }
    if inside("Log").is_some() {
        return Some(HudTap::Log);
    }
    None
}

// ---------------------------------------------------------------------------------------------------------------
// Input.

const TAP_SECS: f32 = 0.2;
const TAP_TRAVEL: f32 = 14.0;
const LONG_SECS: f32 = 0.45;

/// The fingers this frame: (id, logical position, just started, just ended).
fn fingers(touches: &Touches, emu: &mut Local<(bool, bool)>, mouse_ev: &mut MessageReader<MouseButtonInput>, cursor: Option<Vec2>) -> Vec<(u64, Vec2, bool, bool)> {
    if emulating() {
        // The mouse as one finger (id 1): pressed while the left button is down.
        let (was, _) = **emu;
        let mut down = was;
        for e in mouse_ev.read() {
            if e.button == MouseButton::Left {
                down = e.state == ButtonState::Pressed;
            }
        }
        emu.0 = down;
        let Some(c) = cursor else { return Vec::new() };
        return match (was, down) {
            (false, true) => vec![(1, c, true, false)],
            (true, true) => vec![(1, c, false, false)],
            (true, false) => vec![(1, c, false, true)],
            _ => Vec::new(),
        };
    }
    let mut v: Vec<(u64, Vec2, bool, bool)> = touches.iter().map(|t| (t.id(), t.position(), touches.just_pressed(t.id()), false)).collect();
    for t in touches.iter_just_released().chain(touches.iter_just_canceled()) {
        v.push((t.id(), t.position(), false, true));
    }
    v
}

/// PreUpdate: fingers -> roles -> this frame's inputs, move vector, look delta and cursor.
pub fn touch_system(
    touches: Res<Touches>,
    windows: Query<&Window>,
    time: Res<Time<Real>>,
    binds: Res<crate::keybinds::Bindings>,
    mut mouse_ev: MessageReader<MouseButtonInput>,
    mut emu: Local<(bool, bool)>,
) {
    if !enabled() {
        return;
    }
    let Some(win) = windows.iter().next() else { return };
    let (w, h) = (win.width(), win.height());
    let now = time.elapsed_secs();
    let ui = crate::keybinds::UI_MODE.load(std::sync::atomic::Ordering::Relaxed);
    let talking = crate::consys::third_person() && !crate::consys::choosing();
    let cine = crate::cinematic::active();
    let list = fingers(&touches, &mut emu, &mut mouse_ev, win.cursor_position());
    let map = &binds.map;
    with(|s| {
        s.now = now;
        s.held.clear();
        s.move_axes = Vec2::ZERO;
        let mut buttons = layout(w, h, ui);
        if !list.is_empty() {
            s.last_touch = now;
        }
        // New fingers take a role by where they land.
        for &(id, p, started, _) in &list {
            if !started || s.fingers.iter().any(|f| f.id == id) {
                continue;
            }
            let role = if let Some(b) = button_at(&buttons, p) {
                Role::Button(b)
            } else if ui {
                if s.fingers.iter().any(|f| matches!(f.role, Role::Ui { .. })) {
                    Role::Scroll
                } else {
                    Role::Ui { pressed: false, long: false }
                }
            } else if let Some(t) = hud_at(p) {
                Role::Hud(t)
            } else if p.x < w * 0.5 && !s.fingers.iter().any(|f| matches!(f.role, Role::Stick { .. })) {
                Role::Stick { origin: p }
            } else {
                Role::Look { firing: false }
            };
            s.fingers.push(Finger { id, start: p, pos: p, prev: p, t0: now, travel: 0.0, role });
            if matches!(role, Role::Button(_)) && setting!("TouchVibrate") > 0.5 {
                #[cfg(target_os = "android")]
                crate::android::vibrate(12);
            }
        }
        // Moves and ends.
        let mut ended = Vec::new();
        for &(id, p, _, end) in &list {
            if let Some(f) = s.fingers.iter_mut().find(|f| f.id == id) {
                f.prev = f.pos;
                f.pos = p;
                f.travel = f.travel.max(f.start.distance(p));
                if end {
                    ended.push(*f);
                }
            }
        }
        s.fingers.retain(|f| !ended.iter().any(|e| e.id == f.id));
        // Releases.
        for f in &ended {
            let quick = now - f.t0 < TAP_SECS + 0.1 && f.travel < TAP_TRAVEL;
            match f.role {
                Role::Look { firing } => {
                    // A quick tap: one shot (in a conversation the next line, in a cut scene skip).
                    if quick && !firing {
                        if cine {
                            s.taps.push((Input::Key(KeyCode::Enter), 2));
                        } else if talking {
                            s.taps.push((Input::Key(KeyCode::Space), 2));
                        } else if let Some(i) = bound(map, "Fire / throw") {
                            s.taps.push((i, 2));
                        }
                    }
                }
                Role::Stick { .. } => {
                    if quick && (cine || talking) {
                        s.taps.push((Input::Key(if cine { KeyCode::Enter } else { KeyCode::Space }), 2));
                    }
                }
                Role::Button(b) => match b {
                    Btn::Menu => s.taps.push((Input::Key(KeyCode::Escape), 2)),
                    Btn::Keyboard => {
                        s.keyboard = !s.keyboard;
                        #[cfg(target_os = "android")]
                        crate::android::keyboard(s.keyboard);
                    }
                    Btn::Crouch => {
                        if !crate::settings::toggle_crouch() {
                            s.crouch_on = !s.crouch_on;
                        }
                    }
                    _ => {
                        if let Some((a, false)) = b.action() {
                            if let Some(i) = bound(map, a) {
                                s.taps.push((i, 2));
                            }
                        }
                    }
                },
                Role::Hud(t) if quick => match t {
                    HudTap::Belt(n) => {
                        let name = if n == 0 { "Belt 0 (keys)".to_string() } else { format!("Belt {n}") };
                        if let Some(i) = bound(map, &name) {
                            s.taps.push((i, 2));
                        }
                    }
                    HudTap::Aug(k) => s.taps.push((Input::Key(k), 2)),
                    HudTap::Hit => s.open_screen = Some(crate::persona::SCREEN_HEALTH),
                    HudTap::Compass => s.open_screen = Some(crate::persona::SCREEN_GOALS),
                    HudTap::Log => s.open_screen = Some(crate::persona::SCREEN_LOGS),
                    HudTap::Ammo => {
                        if let Some(i) = bound(map, "Change ammo") {
                            s.taps.push((i, 2));
                        }
                    }
                },
                Role::Ui { pressed, long } => {
                    // A tap that never dragged: a click now (press + release over two frames).
                    if !pressed && !long {
                        s.taps.push((Input::Mouse(MouseButton::Left), 2));
                    }
                }
                _ => {}
            }
        }
        // Held fingers.
        let mut cursor = None;
        let mut cursor_down = false;
        for i in 0..s.fingers.len() {
            let f = s.fingers[i];
            let d = f.pos - f.prev;
            match f.role {
                Role::Stick { origin } => {
                    // The stick (a HUD touch that drags becomes one too): push = offset / radius.
                    let r = h * 0.11;
                    let off = f.pos - origin;
                    let push = (off / r).clamp_length_max(1.0);
                    s.move_axes = Vec2::new(push.x, -push.y);
                    // The base follows a thumb that pushes past the rim.
                    if off.length() > r {
                        let o = f.pos - off.normalize() * r;
                        s.fingers[i].role = Role::Stick { origin: o };
                    }
                }
                Role::Look { firing } => {
                    s.look += d;
                    // Held still long enough: fire until it lifts.
                    let fire = firing || (now - f.t0 >= TAP_SECS && f.travel < TAP_TRAVEL && !talking && !cine);
                    if fire {
                        if let Some(i) = bound(map, "Fire / throw") {
                            s.held.push(i);
                        }
                    }
                    s.fingers[i].role = Role::Look { firing: fire };
                }
                Role::Hud(_) if f.travel >= TAP_TRAVEL => {
                    // Dragging off the HUD: it was meant to move or look.
                    s.fingers[i].role = if f.start.x < w * 0.5 { Role::Stick { origin: f.start } } else { Role::Look { firing: false } };
                }
                Role::Button(b) => {
                    if let Some((a, true)) = b.action() {
                        if b != Btn::Crouch || crate::settings::toggle_crouch() {
                            if let Some(i) = bound(map, a) {
                                s.held.push(i);
                            }
                        }
                    }
                }
                Role::Ui { pressed, long } => {
                    cursor = Some(f.pos);
                    if !pressed && !long && f.travel >= 8.0 {
                        // Dragging: the press lands where the finger first came down.
                        s.fingers[i].role = Role::Ui { pressed: true, long: false };
                        cursor = Some(f.start);
                        cursor_down = true;
                    } else if pressed {
                        cursor_down = true;
                    } else if !long && now - f.t0 >= LONG_SECS {
                        s.fingers[i].role = Role::Ui { pressed: false, long: true };
                        s.taps.push((Input::Mouse(MouseButton::Right), 2));
                    }
                }
                Role::Scroll => {
                    s.scroll += -d.y / 24.0;
                }
                _ => {}
            }
        }
        // The crouch toggle holds Crouch (unless the game's own Toggle Crouch does it).
        if s.crouch_on && !ui {
            if let Some(i) = bound(map, "Crouch / swim down") {
                s.held.push(i);
            }
        }
        if cursor_down {
            s.held.push(Input::Mouse(MouseButton::Left));
        }
        // The last spot a finger was stays the cursor (a click lands where it was tapped).
        if let Some(c) = cursor {
            s.cursor = Some(c);
        } else if !ui {
            s.cursor = None;
        } else if let Some(f) = ended.iter().find(|f| matches!(f.role, Role::Ui { .. })) {
            s.cursor = Some(f.pos);
        }
        // The stick shown while held.
        s.stick = s.fingers.iter().find_map(|f| match f.role {
            Role::Stick { origin } => Some((origin, f.pos, h * 0.11)),
            _ => None,
        });
        // Buttons lit / held.
        let use_lit = crate::frobui::has_target();
        for b in &mut buttons {
            b.down = s.fingers.iter().any(|f| f.role == Role::Button(b.btn)) || (b.btn == Btn::Crouch && s.crouch_on) || (b.btn == Btn::Keyboard && s.keyboard);
            b.lit = b.btn == Btn::Use && use_lit;
        }
        s.buttons = buttons;
        // Leaving a screen puts the keyboard away.
        if !ui && s.keyboard {
            s.keyboard = false;
            #[cfg(target_os = "android")]
            crate::android::keyboard(false);
        }
    });
}

// ---------------------------------------------------------------------------------------------------------------
// Drawing: each button and the stick are small images (prompts::present), redrawn when they change.

#[derive(Component, Default)]
pub struct TouchBtnNode;
#[derive(Component, Default)]
pub struct StickBaseNode;
#[derive(Component, Default)]
pub struct StickKnobNode;

/// One image per button slot (index into the layout), with the key it was drawn for.
#[derive(Default)]
pub struct DrawState {
    imgs: Vec<(Option<Handle<Image>>, u64, Entity)>,
    base: Option<Handle<Image>>,
    knob: Option<Handle<Image>>,
    stick_drawn: u64,
}

/// A soft ring (and fill) of `r` units in the middle of a (2r+2)^2 canvas.
fn disc(c: &mut Canvas, r: f32, ring: [f32; 3], fill: [f32; 3], fill_a: f32, ring_a: f32) {
    let s = c.s as f32;
    let (pw, ph) = ((c.w * c.s) as usize, (c.h * c.s) as usize);
    let (cx, cy) = (pw as f32 / 2.0, ph as f32 / 2.0);
    let rr = r * s;
    let thick = (1.6 * s).max(2.0);
    for y in 0..ph {
        for x in 0..pw {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let inside = (rr - d + 0.5).clamp(0.0, 1.0);
            let on_ring = (1.0 - ((d - (rr - thick * 0.5)).abs() - thick * 0.5).clamp(0.0, 1.0)) * inside.max((rr + 0.5 - d).clamp(0.0, 1.0));
            let fa = inside * fill_a;
            let ra = on_ring * ring_a;
            let a = ra + fa * (1.0 - ra);
            if a <= 0.0 {
                continue;
            }
            let p = &mut c.px[y * pw + x];
            for k in 0..3 {
                p[k] = ring[k] * ra + fill[k] * fa * (1.0 - ra);
            }
            p[3] = a;
        }
    }
}

fn draw_button(p: &Placed, unit: f32, alpha: f32, hud: &mut crate::classic_hud::ClassicHud, assets: &mut crate::weapons::RuntimeAssets) -> (Canvas, i32) {
    let r = (p.r / unit).round().max(6.0);
    let size = (r * 2.0 + 2.0) as i32;
    let mut c = Canvas::new(size, size, 2);
    let col = crate::menuui::h_col(7);
    let ring = [col[0] as f32 / 255.0, col[1] as f32 / 255.0, col[2] as f32 / 255.0];
    let (fill, fa) = if p.down {
        (ring, 0.45)
    } else if p.lit {
        ([0.95, 0.75, 0.3], 0.35)
    } else {
        ([0.02, 0.02, 0.03], 0.35)
    };
    disc(&mut c, r, if p.lit { [0.95, 0.75, 0.3] } else { ring }, fill, fa * alpha, 0.9 * alpha);
    if let Some(font) = hud.font(assets, "FontMenuSmall_DS") {
        let t = p.btn.label();
        let tc = if p.down { [10, 10, 12] } else { [240, 240, 240] };
        let tc = [(tc[0] as f32 * alpha) as u8, (tc[1] as f32 * alpha) as u8, (tc[2] as f32 * alpha) as u8];
        c.text(&font, 0, 0, size, size, HAlign::Center, VAlign::Center, tc, t);
    }
    (c, size)
}

pub fn draw_system(
    mut commands: Commands,
    windows: Query<&Window>,
    mut hud: ResMut<crate::classic_hud::ClassicHud>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut layers: ResMut<Assets<crate::classic_hud::HudLayer>>,
    mut btn_nodes: Query<(Entity, &mut Node, &mut Visibility), (With<TouchBtnNode>, Without<StickBaseNode>, Without<StickKnobNode>)>,
    mut base_nodes: Query<(Entity, &mut Node, &mut Visibility), (With<StickBaseNode>, Without<TouchBtnNode>, Without<StickKnobNode>)>,
    mut knob_nodes: Query<(Entity, &mut Node, &mut Visibility), (With<StickKnobNode>, Without<TouchBtnNode>, Without<StickBaseNode>)>,
    mut ds: Local<DrawState>,
) {
    if !enabled() {
        return;
    }
    let Some(win) = windows.iter().next() else { return };
    // Logical px per canvas unit (canvases are drawn at 2 physical pixels a unit).
    let unit = 2.0 / win.scale_factor();
    let (buttons, stick, alpha, hidden) = with(|s| {
        let a = overlay_alpha(s);
        let hidden = crate::cinematic::active() || crate::consys::third_person() || crate::inventory_screen::screen_open() && !s.buttons.iter().any(|b| b.btn == Btn::Keyboard);
        (s.buttons.clone(), s.stick, a, hidden)
    });
    let alpha = if hidden { 0.0 } else { alpha };
    // Buttons: one node each (despawned and rebuilt when the layout's count changes).
    if ds.imgs.len() != buttons.len() {
        for (_, _, e) in ds.imgs.drain(..) {
            commands.entity(e).despawn();
        }
        for _ in 0..buttons.len() {
            let e = commands
                .spawn((
                    TouchBtnNode,
                    Node { position_type: PositionType::Absolute, ..default() },
                    GlobalZIndex(60),
                    Visibility::Hidden,
                    Pickable::IGNORE,
                ))
                .id();
            ds.imgs.push((None, 0, e));
        }
        return;
    }
    for (i, p) in buttons.iter().enumerate() {
        let key = {
            use std::hash::{Hash, Hasher};
            let mut hs = std::collections::hash_map::DefaultHasher::new();
            (p.btn, p.lit, p.down, (p.r * 10.0) as i32, (alpha * 100.0) as i32, crate::menuui::h_col(7)).hash(&mut hs);
            hs.finish()
        };
        let e = ds.imgs[i].2;
        let Ok((_, mut nd, mut vis)) = btn_nodes.get_mut(e) else { continue };
        if alpha <= 0.0 {
            vis.set_if_neq(Visibility::Hidden);
            continue;
        }
        if key != ds.imgs[i].1 {
            let (c, size) = draw_button(p, unit, alpha, &mut hud, &mut assets);
            let px = (size * 2) as u32;
            let img = images.add(Image::new(
                bevy::render::render_resource::Extent3d { width: px, height: px, depth_or_array_layers: 1 },
                bevy::render::render_resource::TextureDimension::D2,
                c.to_rgba8(),
                bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
                bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
            ));
            if let Some(old) = ds.imgs[i].0.replace(img.clone()) {
                images.remove(&old);
            }
            ds.imgs[i].1 = key;
            commands.entity(e).insert(MaterialNode(layers.add(crate::classic_hud::HudLayer::new(img))));
            let side = size as f32 * unit;
            nd.width = Val::Px(side);
            nd.height = Val::Px(side);
        }
        let side = match nd.width {
            Val::Px(v) => v,
            _ => 0.0,
        };
        let (l, t) = (Val::Px(p.at.x - side / 2.0), Val::Px(p.at.y - side / 2.0));
        if nd.left != l || nd.top != t {
            nd.left = l;
            nd.top = t;
        }
        vis.set_if_neq(Visibility::Inherited);
    }
    // The stick: its base and knob drawn once, moved with the thumb.
    let r_units = stick.map_or(0.0, |s| s.2 / unit);
    let skey = (r_units as u64) << 8 | (alpha * 100.0) as u64;
    if let (Some((base, knob, r)), true) = (stick, alpha > 0.0) {
        if skey != ds.stick_drawn {
            ds.stick_drawn = skey;
            let ru = r / unit;
            let col = crate::menuui::h_col(7);
            let ring = [col[0] as f32 / 255.0, col[1] as f32 / 255.0, col[2] as f32 / 255.0];
            let bs = (ru * 2.0 + 2.0) as i32;
            let mut c = Canvas::new(bs, bs, 2);
            disc(&mut c, ru, ring, [0.02, 0.02, 0.03], 0.25 * alpha, 0.7 * alpha);
            crate::prompts::present::<StickBaseNode, _>(&mut commands, &mut images, &mut layers, &mut base_nodes, &mut ds.base, Some(&c), (bs, bs), 2, base - Vec2::splat(bs as f32 * unit / 2.0), unit);
            let ks = (ru * 0.9 + 2.0) as i32;
            let mut c = Canvas::new(ks, ks, 2);
            disc(&mut c, ru * 0.45, ring, ring, 0.55 * alpha, 0.9 * alpha);
            crate::prompts::present::<StickKnobNode, _>(&mut commands, &mut images, &mut layers, &mut knob_nodes, &mut ds.knob, Some(&c), (ks, ks), 2, knob - Vec2::splat(ks as f32 * unit / 2.0), unit);
        }
        let push = (knob - base).clamp_length_max(r);
        place_centred(&mut base_nodes, base);
        place_centred(&mut knob_nodes, base + push);
    } else {
        for (_, _, mut v) in base_nodes.iter_mut().chain(knob_nodes.iter_mut()) {
            v.set_if_neq(Visibility::Hidden);
        }
    }
}

/// Move a node (its own size kept) so its middle is at `at`.
fn place_centred<F: bevy::ecs::query::QueryFilter>(nodes: &mut Query<(Entity, &mut Node, &mut Visibility), F>, at: Vec2) {
    for (_, mut nd, mut v) in nodes.iter_mut() {
        let side = match nd.width {
            Val::Px(v) => v,
            _ => 0.0,
        };
        let (l, t) = (Val::Px(at.x - side / 2.0), Val::Px(at.y - side / 2.0));
        if nd.left != l || nd.top != t {
            nd.left = l;
            nd.top = t;
        }
        v.set_if_neq(Visibility::Inherited);
    }
}
