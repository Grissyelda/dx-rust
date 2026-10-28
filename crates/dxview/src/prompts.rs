//! Controller button prompts (ours, ROADMAP Phase 0.2), shown only while a pad is the last thing used:
//! * a hint bar at the bottom of the free-cursor screens (menus, inventory / persona, computers, conversation
//!   choices): what A / X / B / the D-pad do there (the pad-as-a-mouse buttons, which aren't remappable);
//! * optionally (Controller Options > Button Prompts on Objects) the Use button's glyph beside the frob label.
//! Each is one small canvas in an image node (like the aug wheel), redrawn only when its content changes.

use crate::classic_hud::{ClassicHud, HudLayer};
use crate::gamepad::PadBtn;
use crate::uicanvas::{Canvas, HAlign, VAlign};
use bevy::prelude::*;
use std::sync::atomic::Ordering;

#[derive(Component)]
pub struct HintNode;
#[derive(Component)]
pub struct FrobGlyphNode;

fn hash(s: &str) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

/// Glyph + label pairs laid out left to right: total width in UI units.
fn bar_width(items: &[(PadBtn, &str)], font: &crate::uicanvas::UiFont) -> i32 {
    items.iter().map(|(b, t)| crate::glyphs::width(*b) + 3 + font.width(t) + 10).sum::<i32>() + 6
}

/// Show / place / (re)build one image node.
#[allow(clippy::too_many_arguments)]
fn present<T: Component + Default, F: bevy::ecs::query::QueryFilter>(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    layers: &mut Assets<HudLayer>,
    node: &mut Query<(Entity, &mut Node, &mut Visibility), F>,
    img: &mut Option<Handle<Image>>,
    c: Option<&Canvas>,
    size: (i32, i32),
    s: i32,
    pos: Vec2,
    k: f32,
) {
    let Some(c) = c else {
        for (_, _, mut v) in node.iter_mut() {
            if *v != Visibility::Hidden {
                *v = Visibility::Hidden;
            }
        }
        return;
    };
    let (pw, ph) = ((size.0 * s) as u32, (size.1 * s) as u32);
    let place = |nd: &mut Node| {
        nd.left = Val::Px(pos.x);
        nd.top = Val::Px(pos.y);
        nd.width = Val::Px(size.0 as f32 * k);
        nd.height = Val::Px(size.1 as f32 * k);
    };
    if let Some(h) = img.as_ref() {
        if let Some(mut im) = images.get_mut(h) {
            if im.width() == pw && im.height() == ph {
                im.data = Some(c.to_rgba8());
                for (_, mut nd, mut v) in node.iter_mut() {
                    place(&mut nd);
                    *v = Visibility::Inherited;
                }
                return;
            }
        }
    }
    let im = images.add(Image::new(
        bevy::render::render_resource::Extent3d { width: pw, height: ph, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        c.to_rgba8(),
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
    ));
    *img = Some(im.clone());
    for (e, ..) in node.iter() {
        commands.entity(e).despawn();
    }
    let mut nd = Node { position_type: PositionType::Absolute, ..default() };
    place(&mut nd);
    commands.spawn((T::default(), MaterialNode(layers.add(HudLayer::new(im))), nd, Visibility::Inherited, Pickable::IGNORE, GlobalZIndex(56)));
}

impl Default for HintNode {
    fn default() -> Self {
        HintNode
    }
}
impl Default for FrobGlyphNode {
    fn default() -> Self {
        FrobGlyphNode
    }
}

#[allow(clippy::too_many_arguments)]
pub fn prompts_system(
    mut commands: Commands,
    windows: Query<&Window>,
    mut hud: ResMut<ClassicHud>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut layers: ResMut<Assets<HudLayer>>,
    mut hint: Query<(Entity, &mut Node, &mut Visibility), (With<HintNode>, Without<FrobGlyphNode>)>,
    mut frob: Query<(Entity, &mut Node, &mut Visibility), (With<FrobGlyphNode>, Without<HintNode>)>,
    label: Query<(&Node, &Visibility), (With<crate::frobui::FrobLabel>, Without<HintNode>, Without<FrobGlyphNode>)>,
    mut state: Local<(Option<Handle<Image>>, Option<Handle<Image>>, u64, u64)>,
) {
    let Some(win) = windows.iter().next() else { return };
    let f = crate::gamepad::frame();
    let pad = f.connected && f.pad_last;
    let ui = crate::keybinds::UI_MODE.load(Ordering::Relaxed);
    let s = crate::hud::ui_scale(win).max(1.0) as i32;
    let k = s as f32 / win.scale_factor();
    let Some(font) = hud.font(&mut assets, "FontMenuSmall_DS") else { return };
    let kind = crate::gamepad::glyph_kind();
    let text_col = crate::menuui::h_col(7);
    let (hint_img, frob_img, hint_key, frob_key) = &mut *state;

    // ---- the hint bar
    let items: Vec<(PadBtn, &str)> = if !pad || !ui || crate::settings::get("PadHints") < 0.5 {
        Vec::new()
    } else if crate::consys::third_person() {
        vec![(PadBtn::Down, "Choose"), (PadBtn::South, "Select")]
    } else {
        let mut v = vec![(PadBtn::South, "Select"), (PadBtn::West, "Alternate"), (PadBtn::East, "Back"), (PadBtn::Down, "Move")];
        if crate::inventory_screen::screen_open() {
            v.push((PadBtn::Select, "Close"));
        }
        v
    };
    if items.is_empty() {
        present::<HintNode, _>(&mut commands, &mut images, &mut layers, &mut hint, hint_img, None, (1, 1), s, Vec2::ZERO, k);
        *hint_key = 0;
    } else {
        let w = bar_width(&items, &font);
        let h = 19;
        let pos = Vec2::new((win.width() - w as f32 * k) / 2.0, win.height() - (h as f32 + 6.0) * k);
        let key = hash(&format!("{items:?}{kind:?}{s}{w}{}", win.width()));
        if key != *hint_key || hint.is_empty() {
            *hint_key = key;
            let mut c = Canvas::new(w, h, s);
            // A dark translucent strip under the glyphs, like the HUD's own panels.
            c.fill(0, 0, w, h, crate::uicanvas::Style::Normal, [6, 6, 8]);
            let border = crate::menuui::h_col(1);
            c.fill(0, 0, w, 1, crate::uicanvas::Style::Normal, border);
            let mut x = 4;
            for (b, t) in &items {
                crate::glyphs::draw_in(&mut c, kind, *b, x, 2);
                x += crate::glyphs::width_in(kind, *b) + 3;
                c.text(&font, x, 2, font.width(t) + 2, 15, HAlign::Left, VAlign::Center, text_col, t);
                x += font.width(t) + 10;
            }
            present::<HintNode, _>(&mut commands, &mut images, &mut layers, &mut hint, hint_img, Some(&c), (w, h), s, pos, k);
        } else {
            for (_, mut nd, mut v) in hint.iter_mut() {
                nd.left = Val::Px(pos.x);
                nd.top = Val::Px(pos.y);
                *v = Visibility::Inherited;
            }
        }
    }

    // ---- the Use glyph beside the frob label
    let use_btn = crate::gamepad::button_for("Use / reload (tap), put away (hold)");
    let at = label.iter().next().and_then(|(n, v)| {
        (*v != Visibility::Hidden).then(|| match (n.left, n.top) {
            (Val::Px(x), Val::Px(y)) => Some(Vec2::new(x, y)),
            _ => None,
        })?
    });
    match (pad && !ui && crate::settings::get("PadFrobPrompt") > 0.5, use_btn, at) {
        (true, Some(b), Some(at)) => {
            let w = crate::glyphs::width_in(kind, b);
            let key = hash(&format!("{b:?}{kind:?}{s}"));
            let pos = Vec2::new(at.x - (w as f32 + 3.0) * k, at.y);
            if key != *frob_key || frob.is_empty() {
                *frob_key = key;
                let mut c = Canvas::new(w, crate::glyphs::SIZE, s);
                crate::glyphs::draw_in(&mut c, kind, b, 0, 0);
                present::<FrobGlyphNode, _>(&mut commands, &mut images, &mut layers, &mut frob, frob_img, Some(&c), (w, crate::glyphs::SIZE), s, pos, k);
            } else {
                for (_, mut nd, mut v) in frob.iter_mut() {
                    nd.left = Val::Px(pos.x);
                    nd.top = Val::Px(pos.y);
                    *v = Visibility::Inherited;
                }
            }
        }
        _ => {
            present::<FrobGlyphNode, _>(&mut commands, &mut images, &mut layers, &mut frob, frob_img, None, (1, 1), s, Vec2::ZERO, k);
            *frob_key = 0;
        }
    }
}
