//! Multiplayer name tags: other players' names over their bodies, small, in FontMenuSmall_DS like the frob label.
//! Shown within TAG_RANGE (deathmatch: DM_RANGE) when the head can be seen; one image node per player, redrawn
//! only when the name or the UI scale changes.

use crate::classic_hud::HudLayer;
use crate::hud::ui_scale;
use crate::{FlyCam, LevelRes, SkyCam};
use bevy::prelude::*;
use dx_level::{bevy_to_ue, ue_to_bevy};
use glam::DVec3;

/// About 25 m in co-op, 8 m in deathmatch (Unreal units).
const TAG_RANGE: f64 = 1300.0;
const DM_RANGE: f64 = 420.0;

#[derive(Component)]
pub struct NameTag {
    name: String,
    scale: i32,
    img: Handle<Image>,
}

#[allow(clippy::too_many_arguments)]
pub fn nametag_system(
    mut commands: Commands,
    windows: Query<&Window>,
    cam: Single<(&Camera, &GlobalTransform), (With<FlyCam>, Without<SkyCam>)>,
    level: Res<LevelRes>,
    mut hud: ResMut<crate::classic_hud::ClassicHud>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut layers: ResMut<Assets<HudLayer>>,
    mut tags: Query<(Entity, &mut NameTag, &mut Node, &mut Visibility)>,
) {
    let targets = if crate::netplay::in_game() && !crate::loader::busy() && !crate::consys::third_person() {
        crate::netplay::tag_targets()
    } else {
        Vec::new()
    };
    if targets.is_empty() {
        for (_, _, _, mut v) in &mut tags {
            v.set_if_neq(Visibility::Hidden);
        }
        return;
    }
    let _prof = crate::prof::scope("nettags::nametag_system");
    let Some(win) = windows.iter().next() else { return };
    let s = ui_scale(win);
    let si = s.max(1.0) as i32;
    let px = s / win.scale_factor();
    let k = si as f32 / win.scale_factor();
    let (camera, gt) = *cam;
    let e = bevy_to_ue(gt.translation().to_array());
    let eye = DVec3::new(e[0] as f64, e[1] as f64, e[2] as f64);
    let lvl = &level.0;
    let Some(font) = hud.font(&mut assets, "FontMenuSmall_DS") else { return };
    // Which tags show, and where (UI units, centred above the head).
    let mut shown: Vec<(String, Vec2)> = Vec::new();
    for (name, head, coop) in targets {
        let range = if coop { TAG_RANGE } else { DM_RANGE };
        if head.distance(eye) > range || lvl.collision.trace(eye, head, DVec3::ZERO).fraction < 0.99 {
            continue;
        }
        let b = Vec3::from(ue_to_bevy([head.x as f32, head.y as f32, head.z as f32]));
        if let Ok(v) = camera.world_to_viewport(gt, b) {
            shown.push((name, v / px));
        }
    }
    let mut free: Vec<Entity> = Vec::new();
    for (ent, mut tag, mut node, mut vis) in &mut tags {
        let Some(i) = shown.iter().position(|(n, _)| *n == tag.name) else {
            free.push(ent);
            vis.set_if_neq(Visibility::Hidden);
            continue;
        };
        let (name, at) = shown.remove(i);
        let (w, h) = (font.width(&name) + 6, font.height + 4);
        if tag.scale != si {
            // Redraw at the new scale.
            let img = draw(&font, &name, w, h, si);
            if let Some(mut old) = images.get_mut(&tag.img) {
                *old = img;
            }
            tag.scale = si;
        }
        let (x, y) = ((at.x - w as f32 / 2.0).floor() * k, (at.y - h as f32).floor() * k);
        crate::uinode::set_rect(&mut node, x, y, w as f32 * k, h as f32 * k);
        vis.set_if_neq(Visibility::Inherited);
    }
    for e in free {
        // A tag whose player left or went out of sight for good is rebuilt when needed.
        if shown.is_empty() {
            commands.entity(e).despawn();
        }
    }
    for (name, at) in shown {
        let (w, h) = (font.width(&name) + 6, font.height + 4);
        let img = images.add(draw(&font, &name, w, h, si));
        let tex = img.clone();
        let (x, y) = ((at.x - w as f32 / 2.0).floor() * k, (at.y - h as f32).floor() * k);
        let nd = Node {
            position_type: PositionType::Absolute,
            left: Val::Px(x),
            top: Val::Px(y),
            width: Val::Px(w as f32 * k),
            height: Val::Px(h as f32 * k),
            ..default()
        };
        commands.spawn((
            NameTag { name, scale: si, img: tex },
            MaterialNode(layers.add(HudLayer::new(img))),
            nd,
            Visibility::Inherited,
            Pickable::IGNORE,
            GlobalZIndex(55),
        ));
    }
}

/// The tag picture: the name in the HUD's text colour over a light shade.
fn draw(font: &crate::uicanvas::UiFont, name: &str, w: i32, h: i32, s: i32) -> Image {
    use crate::uicanvas::{Canvas, HAlign, VAlign};
    let mut c = Canvas::new(w, h, s);
    c.shade(0, 0, w, h, 0.45);
    c.text(font, 3, 2, w - 6, font.height, HAlign::Left, VAlign::Top, crate::menuui::h_col(7), name);
    Image::new(
        bevy::render::render_resource::Extent3d { width: (w * s) as u32, height: (h * s) as u32, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        c.to_rgba8(),
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
    )
}
