//! Diegetic Healing (setting DiegeticHealing, ours): with a medkit in hand, hold Alt (pad: LT) and the body from the
//! persona Health screen fades in over the view. A small medkit stands for the cursor (mouse, or either stick); the
//! part under it lights up with its health, and a click (pad: A or RT) treats that part with one medkit - the
//! inventory never opens and the game goes on. Let go and it fades away. Without Alt the medkit is used the
//! original's way (HealPlayer: the points spread from the head down).

use crate::classic_hud::{ClassicHud, HudLayer};
use crate::uicanvas::{Canvas, HAlign, Style, VAlign};
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Up (Alt held with a medkit) - mouse look and firing pause while it is.
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// A click asked for a treatment: the part (persona order index + 1; 0 none), taken by the weapon system.
static USE: AtomicUsize = AtomicUsize::new(0);

pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// The part a click chose (crate::ai::P_*), kept for the medkit's use once the weapon system takes the click.
static CHOSEN: AtomicUsize = AtomicUsize::new(0);

/// A click on a part asks for a medkit use: true once (the part waits for take_part()).
pub fn take_use() -> bool {
    let v = USE.swap(0, Ordering::Relaxed);
    CHOSEN.store(v, Ordering::Relaxed);
    v > 0
}

/// The part this medkit use treats (None = the original's HealPlayer spread).
pub fn take_part() -> Option<usize> {
    let v = CHOSEN.swap(0, Ordering::Relaxed);
    (v > 0).then(|| crate::persona::PARTS[v - 1])
}

const W: i32 = 263;
const H: i32 = 412;
/// The body picture's place in the overlay (it is drawn 219 x 357 like the Health screen's).
const BX: i32 = 22;
const BY: i32 = 16;

#[derive(Component)]
pub struct HealBody;
#[derive(Component)]
pub struct HealCursor;

#[derive(Default)]
pub struct HealUi {
    img: Option<Handle<Image>>,
    layer: Option<Handle<HudLayer>>,
    key: u64,
    alpha: f32,
    /// The cursor in overlay units.
    cur: Vec2,
    shown: bool,
}

fn part_at(p: Vec2) -> Option<usize> {
    crate::persona::PART_RECTS.iter().position(|r| {
        let (x, y) = (BX - 24 + r.0, BY - 36 + r.1);
        p.x >= x as f32 && p.x < (x + r.2) as f32 && p.y >= y as f32 && p.y < (y + r.3) as f32
    })
}

#[allow(clippy::too_many_arguments)]
pub fn heal_ui_system(
    mut commands: Commands,
    time: Res<Time<Real>>,
    windows: Query<&Window>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    weapons: Res<crate::weapons::Weapons>,
    ph: Res<crate::ai::PlayerHealth>,
    mut hud: ResMut<ClassicHud>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut layers: ResMut<Assets<HudLayer>>,
    mut body: Query<(Entity, &mut Node, &mut Visibility), (With<HealBody>, Without<HealCursor>)>,
    mut cursor: Query<(Entity, &mut Node, &mut Visibility, &mut ImageNode), (With<HealCursor>, Without<HealBody>)>,
    mut st: Local<HealUi>,
) {
    let Some(win) = windows.iter().next() else { return };
    let dt = time.delta_secs().min(0.1);
    let pad = crate::gamepad::frame();
    let medkit = weapons.held.as_ref().is_some_and(|h| h.stats.class == "MedKit");
    // DXVIEW_HEALUI=1 holds it up (screenshots).
    let held = keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight) || (pad.connected && pad.lt > 0.5) || env_on!("DXVIEW_HEALUI");
    let blocked = crate::menus::open() || crate::inventory_screen::screen_open() || crate::consys::third_person() || ph.dead;
    let want = crate::bodyfx::diegetic_heal() && medkit && held && !blocked;
    ACTIVE.store(want, Ordering::Relaxed);
    if want && !st.shown {
        // Opens on the chest.
        st.cur = Vec2::new(BX as f32 + 115.0, BY as f32 + 105.0);
    }
    st.shown = want;
    st.alpha = if want { (st.alpha + dt / 0.15).min(1.0) } else { (st.alpha - dt / 0.2).max(0.0) };
    let s = crate::hud::ui_scale(win).max(1.0) as i32;
    let k = s as f32 / win.scale_factor();
    if st.alpha <= 0.0 {
        for (_, _, mut v) in &mut body {
            *v = Visibility::Hidden;
        }
        for (_, _, mut v, _) in &mut cursor {
            *v = Visibility::Hidden;
        }
        return;
    }
    // The cursor: the mouse, or the right stick (the left one still walks).
    if want {
        let mut d = motion.delta / (k * win.scale_factor()).max(0.1);
        if pad.connected {
            d += Vec2::new(pad.rs.x, -pad.rs.y) * 260.0 * dt;
        }
        st.cur = (st.cur + d).clamp(Vec2::ZERO, Vec2::new(W as f32, H as f32));
    }
    let hover = if want { part_at(st.cur) } else { None };
    if want && hover.is_some() && (mouse.just_pressed(MouseButton::Left) || pad.just.contains(&crate::gamepad::PadBtn::South) || pad.just.contains(&crate::gamepad::PadBtn::RT)) {
        USE.store(hover.unwrap() + 1, Ordering::Relaxed);
    }
    let pos = Vec2::new((win.width() - W as f32 * k) * 0.5, (win.height() - H as f32 * k) * 0.42);
    // The body picture: redrawn when the part under the cursor or the health changes.
    let hp: Vec<i32> = crate::persona::PARTS.iter().map(|&p| ph.parts[p].max(0.0).round() as i32).collect();
    let key = {
        let mut h: u64 = 1469598103934665603;
        for v in hp.iter().map(|v| *v as u64).chain([hover.map_or(99, |h| h as u64), s as u64, crate::gamepad::pad_in_use() as u64]) {
            h ^= v;
            h = h.wrapping_mul(1099511628211);
        }
        h
    };
    if key != st.key || body.is_empty() {
        st.key = key;
        let mut c = Canvas::new(W, H, s);
        // A dark panel behind the body so it reads over any scene.
        c.fill(4, 4, W - 8, H - 8, Style::Normal, [4, 6, 8]);
        for (k2, (y, h)) in [(0, 256), (256, 101)].iter().enumerate() {
            if let Some(t) = hud.ui(&mut assets, &format!("HealthBody_{}", k2 + 1)) {
                c.texture(BX, BY + y, 219, *h, 0, 0, &t, Style::Masked, [255, 255, 255]);
            }
        }
        for (k2, (y, h)) in [(0, 256), (256, 101)].iter().enumerate() {
            if let Some(t) = hud.ui(&mut assets, &format!("HealthOverlays_{}", k2 + 1)) {
                c.texture(BX, BY + y, 219, *h, 0, 0, &t, Style::Translucent, crate::menuui::h_col(0));
            }
        }
        let font = hud.font(&mut assets, "FontMenuSmall_DS");
        if let Some(i) = hover {
            let r = crate::persona::PART_RECTS[i];
            let borders = crate::inventory_screen::item_borders(&mut hud, &mut assets);
            c.borders(BX - 24 + r.0, BY - 36 + r.1, r.2, r.3, &borders, Style::Masked, [255, 255, 255]);
        }
        if let Some(f) = font.as_ref() {
            let line = match hover {
                Some(i) => format!("{}   {} / 100", crate::persona::PART_TITLES[i].1, hp[i]),
                None => "Point at a part to treat it".to_string(),
            };
            c.text(f, 0, BY + 360, W, 14, HAlign::Center, VAlign::Center, crate::menuui::h_col(7), &line);
            let how = if crate::gamepad::pad_in_use() { "A: treat it - one medkit" } else { "Click: treat it - one medkit" };
            c.text(f, 0, BY + 376, W, 14, HAlign::Center, VAlign::Center, crate::menuui::h_col(8), how);
        }
        let (pw, ph_) = ((W * s) as u32, (H * s) as u32);
        let fits = st.img.as_ref().and_then(|h| images.get(h)).is_some_and(|im| im.width() == pw && im.height() == ph_);
        if fits {
            if let Some(mut im) = st.img.as_ref().and_then(|h| images.get_mut(h)) {
                im.data = Some(c.to_rgba8());
            }
        } else {
            {
                let im = images.add(Image::new(
                    bevy::render::render_resource::Extent3d { width: pw, height: ph_, depth_or_array_layers: 1 },
                    bevy::render::render_resource::TextureDimension::D2,
                    c.to_rgba8(),
                    bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
                    bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
                ));
                st.img = Some(im.clone());
                for (e, ..) in &body {
                    commands.entity(e).despawn();
                }
                let layer = layers.add(HudLayer::new(im));
                st.layer = Some(layer.clone());
                commands.spawn((HealBody, MaterialNode(layer), Node { position_type: PositionType::Absolute, ..default() }, Visibility::Hidden, Pickable::IGNORE, GlobalZIndex(57)));
            }
        }
    }
    if let Some(mut l) = st.layer.as_ref().and_then(|h| layers.get_mut(h)) {
        if (l.gamma.y - st.alpha).abs() > 1e-3 {
            l.gamma.y = st.alpha;
        }
    }
    for (_, mut nd, mut v) in &mut body {
        nd.left = Val::Px(pos.x);
        nd.top = Val::Px(pos.y);
        nd.width = Val::Px(W as f32 * k);
        nd.height = Val::Px(H as f32 * k);
        *v = Visibility::Inherited;
    }
    // The medkit cursor (its belt icon at half size, the hot spot at its centre).
    if cursor.is_empty() {
        if let Some(t) = hud.tex(&mut assets, "DeusExUI.Icons.BeltIconMedKit") {
            let px: Vec<u8> = t.px.iter().flatten().copied().collect();
            let im = images.add(Image::new(
                bevy::render::render_resource::Extent3d { width: t.w, height: t.h, depth_or_array_layers: 1 },
                bevy::render::render_resource::TextureDimension::D2,
                px,
                bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
                bevy::asset::RenderAssetUsages::RENDER_WORLD,
            ));
            commands.spawn((HealCursor, ImageNode::new(im), Node { position_type: PositionType::Absolute, ..default() }, Visibility::Hidden, Pickable::IGNORE, GlobalZIndex(58)));
        }
        return;
    }
    for (_, mut nd, mut v, mut img) in &mut cursor {
        let size = 32.0 * k;
        nd.left = Val::Px(pos.x + st.cur.x * k - size * 0.5);
        nd.top = Val::Px(pos.y + st.cur.y * k - size * 0.5);
        nd.width = Val::Px(size);
        nd.height = Val::Px(size);
        img.color = Color::srgba(1.0, 1.0, 1.0, st.alpha);
        *v = if want { Visibility::Inherited } else { Visibility::Hidden };
    }
}
