//! The gamepad augmentation wheel and D-pad hotkey pages (ROADMAP Phase 0.1, the fix for the clunky aug management).
//!
//! Wheel (the default): hold the wheel button (D-pad Up). Every installed aug that can be switched on and off sits
//! on a ring in hotkey order (F3 first, clockwise from the top), drawn like the HUD's aug column (HUDIconsBackground,
//! the small icon yellow while on and grey while off, its F-key). The right stick picks one; its name, state and
//! energy rate show in the middle; releasing the button toggles it (the same path as its F-key). The view doesn't
//! turn while the wheel is up.
//!
//! Hotkey pages (Settings > Enhancements > Pad Aug Hotkey Pages): the augs are split into pages of three in hotkey
//! order. D-pad Left / Down / Right toggle the current page's three, a tap of the wheel button turns the page, and
//! holding it still opens the wheel. A small strip over the belt shows the page.

use crate::classic_hud::{ClassicHud, HudLayer};
use crate::gamepad::{PadBtn, PadFrame};
use crate::uicanvas::{Canvas, HAlign, Style, VAlign};
use bevy::prelude::*;
use std::sync::Mutex;

#[derive(Default)]
struct WheelState {
    open: bool,
    held_t: f32,
    sel: Option<usize>,
    page: usize,
    /// Aug F-key actions to tap (drained by gamepad::outputs).
    taps: Vec<&'static str>,
}

static STATE: Mutex<Option<WheelState>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut WheelState) -> R) -> R {
    let mut g = STATE.lock().unwrap();
    f(g.get_or_insert_with(WheelState::default))
}

pub fn open() -> bool {
    with(|s| s.open)
}

fn pages_mode() -> bool {
    crate::settings::get("PadAugPages") > 0.5
}

/// D-pad buttons the hotkey pages use (so they don't also step the belt).
pub fn takes(b: PadBtn) -> bool {
    pages_mode() && matches!(b, PadBtn::Left | PadBtn::Down | PadBtn::Right)
}

/// The F-key action for an aug hotkey (3..12).
fn action_for(hotkey: i32) -> Option<&'static str> {
    Some(match hotkey {
        3 => "Augmentation F3",
        4 => "Augmentation F4",
        5 => "Augmentation F5",
        6 => "Augmentation F6",
        7 => "Augmentation F7",
        8 => "Augmentation F8",
        9 => "Augmentation F9",
        10 => "Augmentation F10",
        11 => "Augmentation F11",
        12 => "Augmentation F12 (light)",
        _ => return None,
    })
}

/// Switchable installed augs: (hotkey, name, small icon, on, energy rate per minute).
fn augs() -> Vec<(i32, String, String, bool, f32)> {
    let g = crate::abilities::lock();
    let mut v: Vec<(i32, String, String, bool, f32)> = g
        .as_ref()
        .map(|a| {
            a.augs
                .list
                .iter()
                .filter(|x| x.has && !x.def.always_active && x.key().is_some())
                .map(|x| (x.hotkey, x.def.name.clone(), x.def.small_icon.clone(), x.active, x.energy_rate()))
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_key(|x| x.0);
    v
}

/// gamepad::outputs: this frame's aug toggles (hotkey pages and the wheel), as keyboard action names.
pub fn hotkey_presses(f: &PadFrame) -> Vec<&'static str> {
    let mut out = with(|s| std::mem::take(&mut s.taps));
    if pages_mode() && !open() {
        let list = augs();
        let page = with(|s| s.page);
        for (slot, b) in [PadBtn::Left, PadBtn::Down, PadBtn::Right].iter().enumerate() {
            if f.just.contains(b) {
                if let Some(a) = list.get(page * 3 + slot).and_then(|a| action_for(a.0)) {
                    out.push(a);
                }
            }
        }
    }
    out
}

#[derive(Component)]
pub struct WheelNode;

const R: f32 = 78.0;
const SIZE: i32 = 240;

#[allow(clippy::too_many_arguments)]
pub fn augwheel_system(
    mut commands: Commands,
    time: Res<Time<Real>>,
    windows: Query<&Window>,
    mut hud: ResMut<ClassicHud>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut layers: ResMut<Assets<HudLayer>>,
    mut node: Query<(Entity, &mut Node, &mut Visibility), With<WheelNode>>,
    mut img: Local<Option<Handle<Image>>>,
    mut last: Local<u64>,
) {
    let f = crate::gamepad::frame();
    let dt = time.delta_secs().min(0.1);
    let wheel_btn = crate::gamepad::button_for("Augmentation wheel (hold)");
    let busy = crate::inventory_screen::screen_open() || crate::consys::third_person() || crate::cinematic::active();
    let list = augs();
    let held = wheel_btn.is_some_and(|b| f.held.contains(&b)) && !busy && f.connected;
    let pages = pages_mode();
    let npages = list.len().div_ceil(3).max(1);
    with(|s| {
        if held {
            s.held_t += dt;
            // In pages mode a tap turns the page, so the wheel waits a moment.
            if !s.open && (!pages || s.held_t > 0.25) && !list.is_empty() {
                s.open = true;
                s.sel = None;
            }
        } else {
            if s.open {
                if let Some(a) = s.sel.and_then(|i| list.get(i)).and_then(|a| action_for(a.0)) {
                    s.taps.push(a);
                }
            } else if pages && s.held_t > 0.0 && s.held_t <= 0.25 {
                s.page = (s.page + 1) % npages;
            }
            s.open = false;
            s.held_t = 0.0;
        }
        if s.page >= npages {
            s.page = 0;
        }
        if s.open && f.rs.length() > 0.5 && !list.is_empty() {
            // Clockwise from the top.
            let ang = f.rs.x.atan2(f.rs.y).rem_euclid(std::f32::consts::TAU);
            let n = list.len();
            s.sel = Some(((ang / std::f32::consts::TAU * n as f32 + 0.5) as usize) % n);
        }
    });
    let (open, sel, page) = with(|s| (s.open, s.sel, s.page));
    let show_strip = pages && f.connected && !busy && !list.is_empty() && !open;
    let Some(win) = windows.iter().next() else { return };
    let s = crate::hud::ui_scale(win).max(1.0) as i32;
    let k = s as f32 / win.scale_factor();
    // Redraw when what's shown changes.
    let key = {
        let mut h: u64 = 1469598103934665603;
        for b in format!("{open}{sel:?}{page}{show_strip}{s}{list:?}{:?}", crate::gamepad::glyph_kind()).bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(1099511628211);
        }
        h
    };
    let visible = open || show_strip;
    for (_, _, mut v) in &mut node {
        let want = if visible { Visibility::Inherited } else { Visibility::Hidden };
        if *v != want {
            *v = want;
        }
    }
    if !visible || key == *last {
        if !visible {
            *last = 0;
        }
        return;
    }
    *last = key;
    let mut c = Canvas::new(SIZE, SIZE, s);
    let bg = hud.ui(&mut assets, "HUDIconsBackground");
    let head = hud.font(&mut assets, "FontMenuHeaders_DS");
    let small = hud.font(&mut assets, "FontMenuSmall_DS");
    let border = crate::menuui::h_col(1);
    let header = crate::menuui::h_col(7);
    let slot = |c: &mut Canvas, hud: &mut ClassicHud, assets: &mut crate::weapons::RuntimeAssets, x: i32, y: i32, a: &(i32, String, String, bool, f32), hi: bool| {
        if let Some(t) = &bg {
            c.texture(x, y, 36, 36, 0, 0, t, Style::Translucent, crate::menuui::h_col(0));
        }
        if let Some(t) = hud.tex(assets, &a.2) {
            c.texture(x + 2, y + 2, 32, 32, 0, 0, &t, Style::Translucent, if a.3 { [255, 255, 0] } else { [100, 100, 100] });
        }
        if hi {
            // The selection: a bright frame in the HUD border colour, doubled.
            for d in [0, 1] {
                c.fill(x - 2 + d, y - 2 + d, 40 - 2 * d, 1, Style::Normal, header);
                c.fill(x - 2 + d, y + 37 - d, 40 - 2 * d, 1, Style::Normal, header);
                c.fill(x - 2 + d, y - 2 + d, 1, 40 - 2 * d, Style::Normal, header);
                c.fill(x + 37 - d, y - 2 + d, 1, 40 - 2 * d, Style::Normal, header);
            }
        }
    };
    let n = list.len();
    if open {
        // A thin ring and the slots on it.
        let ctr = SIZE as f32 / 2.0;
        for i in 0..180 {
            let t = i as f32 / 180.0 * std::f32::consts::TAU;
            c.fill((ctr + t.sin() * R) as i32, (ctr - t.cos() * R) as i32, 1, 1, Style::Translucent, border);
        }
        for (i, a) in list.iter().enumerate() {
            let t = i as f32 / n as f32 * std::f32::consts::TAU;
            let (x, y) = ((ctr + t.sin() * R) as i32 - 18, (ctr - t.cos() * R) as i32 - 18);
            slot(&mut c, &mut hud, &mut assets, x, y, a, sel == Some(i));
        }
        // The middle: name, on / off and energy use of the picked aug, else a hint.
        if let (Some(h), Some(sm)) = (&head, &small) {
            let (name, l2) = match sel.and_then(|i| list.get(i)) {
                Some(a) => (a.1.clone(), format!("{} - {:.0} / min", if a.3 { "Active" } else { "Inactive" }, a.4)),
                None => ("Augmentations".to_string(), "Right stick to pick".to_string()),
            };
            let w = 130;
            for (i, line) in crate::uicanvas::wrap(h, &name, w).iter().take(2).enumerate() {
                c.text(h, SIZE / 2 - w / 2, SIZE / 2 - 18 + i as i32 * h.height, w, h.height, HAlign::Center, VAlign::Top, header, line);
            }
            if sel.is_some() {
                c.text(sm, SIZE / 2 - w / 2, SIZE / 2 + 8, w, sm.height, HAlign::Center, VAlign::Top, crate::menuui::h_col(8), &l2);
            } else {
                // Nothing picked yet: [right stick] Pick, then "release [wheel button]" as glyphs.
                let kind = crate::gamepad::glyph_kind();
                let col = crate::menuui::h_col(8);
                let line = |c: &mut Canvas, y: i32, items: &[(Option<PadBtn>, &str)]| {
                    let tw: i32 = items.iter().map(|(b, t)| b.map_or(0, |b| crate::glyphs::width_in(kind, b) + 3) + sm.width(t) + 4).sum::<i32>() - 4;
                    let mut x = SIZE / 2 - tw / 2;
                    for (b, t) in items {
                        if let Some(b) = b {
                            crate::glyphs::draw_in(c, kind, *b, x, y);
                            x += crate::glyphs::width_in(kind, *b) + 3;
                        }
                        c.text(sm, x, y, sm.width(t) + 2, crate::glyphs::SIZE, HAlign::Left, VAlign::Center, col, t);
                        x += sm.width(t) + 4;
                    }
                };
                line(&mut c, SIZE / 2 + 4, &[(Some(PadBtn::R3), "Pick")]);
                if let Some(b) = wheel_btn {
                    line(&mut c, SIZE / 2 + 21, &[(None, "Release"), (Some(b), "to toggle")]);
                }
            }
        }
    } else if show_strip {
        // Hotkey page: three slots with their D-pad direction, at the bottom of the canvas.
        let y = SIZE - 52;
        for (slot_i, (b, x)) in [(PadBtn::Left, SIZE / 2 - 66), (PadBtn::Down, SIZE / 2 - 18), (PadBtn::Right, SIZE / 2 + 30)].iter().enumerate() {
            if let Some(a) = list.get(page * 3 + slot_i) {
                slot(&mut c, &mut hud, &mut assets, *x, y, a, false);
                crate::glyphs::draw(&mut c, *b, *x + 11, y + 38, s);
            }
        }
        if let Some(sm) = &small {
            let t = format!("Page {}/{}", page + 1, npages);
            c.text(sm, SIZE / 2 - 60, y - 12, 120, sm.height, HAlign::Center, VAlign::Top, header, &t);
        }
    }
    let bytes = c.to_rgba8();
    let px = (SIZE * s) as u32;
    if let Some(h) = img.as_ref() {
        if let Some(mut im) = images.get_mut(h) {
            if im.width() == px {
                im.data = Some(bytes);
                for (_, mut nd, _) in &mut node {
                    nd.left = Val::Px((win.width() - SIZE as f32 * k) / 2.0);
                    nd.top = Val::Px(if open { (win.height() - SIZE as f32 * k) / 2.0 } else { win.height() - (SIZE as f32 + 70.0) * k });
                }
                return;
            }
        }
    }
    let im = images.add(Image::new(
        bevy::render::render_resource::Extent3d { width: px, height: px, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        bytes,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
    ));
    *img = Some(im.clone());
    // A new size (the UI scale changed): replace the node.
    for (e, ..) in &node {
        commands.entity(e).despawn();
    }
    commands.spawn((
        WheelNode,
        MaterialNode(layers.add(HudLayer::new(im))),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px((win.width() - SIZE as f32 * k) / 2.0),
            top: Val::Px(if open { (win.height() - SIZE as f32 * k) / 2.0 } else { win.height() - (SIZE as f32 + 70.0) * k }),
            width: Val::Px(SIZE as f32 * k),
            height: Val::Px(SIZE as f32 * k),
            ..default()
        },
        Visibility::Inherited,
        Pickable::IGNORE,
        GlobalZIndex(-2),
    ));
}
