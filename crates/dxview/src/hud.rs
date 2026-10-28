//! Aiming: DeusExWeapon.CalculateAccuracy for the player (skill, standing-still bonus) and the HUD crosshair
//! with the accuracy brackets from AugmentationDisplayWindow (red on enemies, green on friends).

use crate::actors::{Blockers, World};
use crate::ai::Npc;
use crate::movers::Movers;
use crate::weapons::{RuntimeAssets, WState, Weapons};
use crate::{FlyCam, LevelRes, Player, SkyCam};
use bevy::{
    asset::RenderAssetUsages,
    image::{ImageSampler, ImageSamplerDescriptor},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use dx_level::bevy_to_ue;
use glam::DVec3;
use imsim_core::movement::CollisionWorld;
use unreal_pkg::props::find;

/// The player's current aim.
#[derive(Resource, Default)]
pub struct Aim {
    /// Seconds spent standing still (DeusExWeapon.standingTimer).
    pub standing: f32,
    /// 0 = dead on, 1 = way off (DeusExWeapon.currentAccuracy).
    pub accuracy: f32,
    /// currentAccuracy before the scope adjustment: how much a zoomed scope wanders.
    pub sway: f32,
    held_class: Option<String>,
}

#[derive(Component)]
pub struct CrossImage;
/// Crosshair texture size in UI units.
#[derive(Component)]
pub struct CrossSize(f32, f32);

/// Deus Ex UI scale for this window, in physical pixels per UI unit (the nearest whole multiple of 600 lines).
pub fn ui_scale(win: &Window) -> f32 {
    // DeusExRootWindow: whole multiples of 600 lines; the HUD Scale setting goes one step either way.
    ((win.physical_height() as f32 / 600.0).round() + crate::settings::hud_scale_step() as f32).max(1.0)
}
/// Scale for a full-screen window of w x h UI units (inventory, computers, menus): Medium is the largest whole
/// multiple that fits the window (as before); the HUD Scale setting steps it the same way as the HUD, though never
/// past what fits.
/// The inventory, computer and menu screens' scale: (the whole-number scale their canvas is drawn at, the scale
/// shown on screen). HUD Scale Small = one whole step down, Medium = the largest whole fit (crisp pixels, as
/// before), Large = as big as fits (fractional: the canvas is drawn at the next whole scale up and filtered down,
/// so a 640 x 480 screen fills a Steam Deck's 800 lines instead of sitting at 1x).
pub fn screen_scales(win: &Window, w: i32, h: i32) -> (i32, f32) {
    let (pw, ph) = (win.physical_width() as f32, win.physical_height() as f32);
    let fitf = (pw / w as f32).min(ph / h as f32).max(1.0);
    let fit = fitf.floor() as i32;
    match crate::settings::hud_scale_step() {
        s if s > 0 => (fitf.ceil() as i32, fitf),
        s => {
            let k = (fit + s).clamp(1, fit);
            (k, k as f32)
        }
    }
}

/// Linear filtering for a screen canvas shown at a fractional scale (nearest when whole).
pub fn set_filter(images: &mut Assets<Image>, img: &Handle<Image>, render: i32, display: f32) {
    use bevy::image::ImageSampler;
    let linear = (display - render as f32).abs() > 1e-3;
    static LAST: std::sync::Mutex<Vec<(bevy::asset::AssetId<Image>, bool)>> = std::sync::Mutex::new(Vec::new());
    let Ok(mut last) = LAST.lock() else { return };
    let id = img.id();
    if last.iter().any(|(i, l)| *i == id && *l == linear) {
        return;
    }
    if let Some(mut im) = images.get_mut(id) {
        im.sampler = if linear { ImageSampler::linear() } else { ImageSampler::nearest() };
        last.retain(|(i, _)| *i != id);
        last.push((id, linear));
    }
}

pub fn screen_scale(win: &Window, w: i32, h: i32) -> i32 {
    let (pw, ph) = (win.physical_width() as i32, win.physical_height() as i32);
    let fit = (pw / w).min(ph / h).max(1);
    (fit + crate::settings::hud_scale_step()).clamp(1, fit)
}

#[derive(Component)]
pub struct Bracket {
    index: usize,
    shadow: bool,
}

/// Bracket corner length in pixels (AugmentationDisplayWindow.corner).
const CORNER: f32 = 9.0;

/// Update the standing timer and accuracy every frame, like DeusExWeapon.Tick.
pub fn aim_system(
    time: Res<Time>,
    player: Res<Player>,
    weapons: Res<Weapons>,
    mut aim: ResMut<Aim>,
    ph: Res<crate::ai::PlayerHealth>,
    zoom: Res<crate::weapons::Zoom>,
) {
    let _prof = crate::prof::scope("hud::aim_system");
    let dt = time.delta_secs();
    let Some(h) = weapons.held.as_ref() else {
        aim.held_class = None;
        aim.standing = 0.0;
        return;
    };
    // BringUp resets the standing bonus.
    if aim.held_class.as_deref() != Some(h.stats.class.as_str()) || h.state == WState::Selecting {
        aim.held_class = Some(h.stats.class.clone());
        aim.standing = 0.0;
    }
    let speed = player.body.velocity.length() as f32;
    if speed < 10.0 {
        aim.standing += dt;
    } else {
        aim.standing = (aim.standing - 0.03 * dt * speed).max(0.0);
    }
    let skill = h.stats.skill;
    // Wounded arms and head make aiming worse.
    let mut acc = h.stats.base_accuracy + skill + ph.accuracy_penalty();
    let before = acc;
    if aim.standing > 0.0 {
        let div = (15.0 + 29.0 * skill).max(0.0);
        acc -= if div > 0.0 {
            (aim.standing / div).clamp(0.0, 0.6)
        } else {
            0.6
        };
        if acc < 0.1 && before > 0.1 {
            acc = 0.1;
        }
    }
    aim.sway = acc.max(0.0);
    // TraceFire: with the laser on the shot is dead on.
    if h.lasing {
        aim.accuracy = 0.0;
        return;
    }
    // TraceFire: an unused scope costs 0.2.
    if h.stats.has_scope && zoom.0.is_none() {
        acc += 0.2;
    } else if h.stats.has_scope {
        // Zoomed in: the shot goes where the (swaying) scope points.
        acc = 0.0;
    }
    aim.accuracy = acc.max(0.0);
}

pub fn spawn_crosshair(
    commands: &mut Commands,
    assets: &mut RuntimeAssets,
    images: &mut Assets<Image>,
) {
    let tex = assets
        .lib
        .load_image_masked("DeusExUI.UserInterface.CrossSquare")
        .map(|t| {
            let mut img = Image::new(
                Extent3d {
                    width: t.width,
                    height: t.height,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                t.mips[0].clone(),
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            );
            img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::nearest());
            (images.add(img), t.width as f32, t.height as f32)
        });
    if let Some((h, w, ht)) = tex {
        commands.spawn((
            ImageNode::new(h),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Px(w),
                height: Val::Px(ht),
                ..default()
            },
            Pickable::IGNORE,
            CrossImage,
            CrossSize(w, ht),
        ));
    }
    for shadow in [true, false] {
        for index in 0..8 {
            commands.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    ..default()
                },
                BackgroundColor(if shadow { Color::BLACK } else { Color::WHITE }),
                Visibility::Hidden,
                Pickable::IGNORE,
                Bracket { index, shadow },
            ));
        }
    }
}

/// Place the crosshair and the accuracy brackets; color them by what is under the crosshair.
#[allow(clippy::too_many_arguments)]
pub fn crosshair_system(
    windows: Query<&Window>,
    cam: Single<&Transform, (With<FlyCam>, Without<SkyCam>)>,
    level: Res<LevelRes>,
    blockers: Res<Blockers>,
    mv: Res<Movers>,
    weapons: Res<Weapons>,
    aim: Res<Aim>,
    npcs: Query<&Npc>,
    mut cross: Query<
        (&mut ImageNode, &mut Node, &CrossSize, &mut Visibility),
        (With<CrossImage>, Without<Bracket>),
    >,
    mut brackets: Query<
        (&Bracket, &mut Node, &mut BackgroundColor, &mut Visibility),
        Without<CrossImage>,
    >,
    zoom: Res<crate::weapons::Zoom>,
) {
    let _prof = crate::prof::scope("hud::crosshair_system");
    let Some(win) = windows.iter().next() else {
        return;
    };
    // Deus Ex draws its UI in scaled units: an integer scale for the screen (2x at 1080p).
    let s = ui_scale(win);
    let px = s / win.scale_factor();
    let (w, h) = (
        win.physical_width() as f32 / s,
        win.physical_height() as f32 / s,
    );
    let (x, y) = ((w * 0.5).floor() - 1.0, (h * 0.5).floor() - 1.0);

    // TraceLOS from the eye: what are we pointing at?
    let eye = bevy_to_ue(cam.translation.to_array());
    let eye = DVec3::new(eye[0] as f64, eye[1] as f64, eye[2] as f64);
    let f = cam.forward().as_vec3();
    let dir = DVec3::new(f.x as f64, f.z as f64, f.y as f64).normalize();
    let world = World {
        level: &level.0,
        blockers: &blockers.list,
        ignore: None,
        pushable: None,
        movers: Some(&mv.rt),
    };
    let hit = world.trace(eye, eye + dir * 8000.0, DVec3::splat(0.5));
    let mut color = Color::WHITE;
    let mut target_dist = None;
    ON_ENEMY.store(false, std::sync::atomic::Ordering::Relaxed);
    // TraceLOS also takes breakable DeusExMovers (the training range's glass targets): the nearest such mover
    // in front of whatever else the line hits.
    let mover_target = crate::movers::mover_on_line(&level.0, &mv.rt, eye, eye + dir * 8000.0).filter(|&(i, f)| {
        // The world trace stops its 0.5-unit box short of the mover's face; the mover line has no extent.
        f <= hit.fraction + 4.0 / 8000.0 && {
            let rt = &mv.rt[i];
            let p = &level.0.actors[level.0.movers[i].actor].props;
            rt.breakable.unwrap_or(matches!(find(p, "bBreakable"), Some(unreal_pkg::props::Value::Bool(true))))
        }
    });
    if std::env::var("DXVIEW_AIMLOG").is_ok() {
        let any = crate::movers::mover_on_line(&level.0, &mv.rt, eye, eye + dir * 8000.0);
        println!("AIM hit {:.1} actor {:?} mover {:?} target {:?}", hit.fraction * 8000.0, hit.actor, any.map(|(i, f)| (i, f * 8000.0)), mover_target.map(|m| m.0));
    }
    if let Some((_, f)) = mover_target {
        target_dist = Some(8000.0 * f);
    } else if let Some(b) = hit.actor {
        target_dist = Some(8000.0 * hit.fraction);
        let a = &level.0.actors[blockers.list[b].actor];
        if find(&a.props, "GroundSpeed").is_some() {
            let hostile = npcs
                .iter()
                .find(|n| n.blocker == b)
                .map(|n| n.hates("Player"))
                .unwrap_or(false);
            ON_ENEMY.store(hostile, std::sync::atomic::Ordering::Relaxed);
            color = if hostile {
                Color::srgb(1.0, 0.0, 0.0)
            } else {
                Color::srgb(0.0, 1.0, 0.0)
            };
        }
    }
    // The scope and binocular views hide the crosshair (DeusExScopeView.ActivateView).
    let zoomed = zoom.0.is_some();
    let menu = crate::inventory_screen::screen_open() || crate::consys::third_person() || crate::cinematic::active();
    for (mut img, mut node, size, mut vis) in &mut cross {
        *vis = if zoomed || menu || !crate::settings::crosshairs() {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        img.color = color;
        let (cw, ch) = (size.0 * px, size.1 * px);
        node.width = Val::Px(cw);
        node.height = Val::Px(ch);
        node.left = Val::Px((w * px - cw) * 0.5);
        node.top = Val::Px((h * px - ch) * 0.5);
    }

    // Brackets only for ranged weapons with a target in range. Looking through a rifle scope they always show,
    // sized by the accuracy that drives the scope's sway, so they close in as the aim steadies.
    let scoped = zoomed
        && weapons.held.as_ref().map_or(false, |h| {
            h.stats.has_scope && h.stats.class != "Binoculars"
        });
    let show = !menu && scoped
        || (!zoomed
            && !menu
            && match (weapons.held.as_ref(), target_dist) {
                (Some(hd), Some(d)) => {
                    !hd.stats.hand_to_hand && !hd.stats.item && hd.stats.max_range as f64 >= d
                }
                _ => false,
            });
    let acc = if scoped { aim.sway } else { aim.accuracy };
    let mut mult = (acc * 80.0 * (w / 640.0)).clamp(CORNER, 80.0);
    mult = mult.max(CORNER + 4.0);
    if acc == 0.0 {
        mult = CORNER;
    }
    let c = CORNER;
    let half = ((c - 1.0) / 2.0).floor();
    // (left, top, width, height) of the eight strokes, as AugmentationDisplayWindow draws them.
    let boxes = [
        (x, y - mult, 1.0, c),
        (x, y + mult - c, 1.0, c),
        (x - half, y - mult, c, 1.0),
        (x - half, y + mult, c, 1.0),
        (x - mult, y, c, 1.0),
        (x + mult - c, y, c, 1.0),
        (x - mult, y - half, 1.0, c),
        (x + mult, y - half, 1.0, c),
    ];
    for (br, mut node, mut bg, mut vis) in &mut brackets {
        *vis = if show {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if !show {
            continue;
        }
        let (l, t, bw, bh) = boxes[br.index];
        let o = if br.shadow { 1.0 } else { 0.0 };
        node.left = Val::Px((l + o) * px);
        node.top = Val::Px((t + o) * px);
        node.width = Val::Px(bw * px);
        node.height = Val::Px(bh * px);
        if !br.shadow {
            bg.0 = color;
        }
    }
}

/// Which picture a scope-overlay node shows.
#[derive(Component, Clone, Copy, PartialEq)]
pub enum ScopePart {
    /// One of the four black borders around the view.
    Border(u8),
    /// Binocular view/crosshair halves (0, 1) or the rifle scope view/crosshair.
    Binoc {
        cross: bool,
        half: u8,
    },
    Scope {
        cross: bool,
    },
}

fn ui_image(images: &mut Assets<Image>, w: u32, h: u32, data: Vec<u8>) -> Handle<Image> {
    let mut img = Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    images.add(img)
}

/// DeusExScopeView: black borders round a 256-high window (512 wide for binoculars), the view texture drawn
/// modulated (here: black with alpha = 1 - 2 x grey, since it only darkens) and the crosshair masked in the HUD
/// header colour (white in the default theme).
pub fn spawn_scope_overlay(
    commands: &mut Commands,
    assets: &mut RuntimeAssets,
    images: &mut Assets<Image>,
) {
    let mut modulated = |name: &str, images: &mut Assets<Image>| {
        assets.lib.load(name).map(|(t, mips)| {
            let m = &t.mips[0];
            let data: Vec<u8> = mips[0]
                .chunks_exact(4)
                .flat_map(|c| {
                    let g = (c[0] as f32 + c[1] as f32 + c[2] as f32) / (3.0 * 255.0);
                    [0u8, 0, 0, ((1.0 - 2.0 * g).clamp(0.0, 1.0) * 255.0) as u8]
                })
                .collect();
            ui_image(images, m.width, m.height, data)
        })
    };
    let views = [
        (
            ScopePart::Binoc {
                cross: false,
                half: 0,
            },
            modulated("DeusExUI.UserInterface.HUDBinocularView_1", images),
        ),
        (
            ScopePart::Binoc {
                cross: false,
                half: 1,
            },
            modulated("DeusExUI.UserInterface.HUDBinocularView_2", images),
        ),
        (
            ScopePart::Scope { cross: false },
            modulated("DeusExUI.UserInterface.HUDScopeView", images),
        ),
    ];
    let mut masked = |name: &str, images: &mut Assets<Image>| {
        assets
            .lib
            .load_image_masked(name)
            .map(|t| ui_image(images, t.width, t.height, t.mips[0].clone()))
    };
    let crosses = [
        (
            ScopePart::Binoc {
                cross: true,
                half: 0,
            },
            masked("DeusExUI.UserInterface.HUDBinocularCrosshair_1", images),
        ),
        (
            ScopePart::Binoc {
                cross: true,
                half: 1,
            },
            masked("DeusExUI.UserInterface.HUDBinocularCrosshair_2", images),
        ),
        (
            ScopePart::Scope { cross: true },
            masked("DeusExUI.UserInterface.HUDScopeCrosshair", images),
        ),
    ];
    for i in 0..4u8 {
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            BackgroundColor(Color::BLACK),
            Visibility::Hidden,
            Pickable::IGNORE,
            GlobalZIndex(-5),
            ScopePart::Border(i),
        ));
    }
    for (part, h) in views.into_iter().chain(crosses) {
        if let Some(h) = h {
            commands.spawn((
                ImageNode::new(h).with_mode(bevy::ui::widget::NodeImageMode::Stretch),
                Node {
                    position_type: PositionType::Absolute,
                    ..default()
                },
                Visibility::Hidden,
                Pickable::IGNORE,
                GlobalZIndex(-5),
                part,
            ));
        }
    }
}

/// Show the binocular view while zoomed with binoculars in hand, the rifle scope while a scoped weapon is zoomed.
pub fn scope_overlay_system(
    windows: Query<&Window>,
    zoom: Res<crate::weapons::Zoom>,
    weapons: Res<Weapons>,
    mut q: Query<(&ScopePart, &mut Node, &mut Visibility)>,
) {
    let _prof = crate::prof::scope("hud::scope_overlay_system");
    let Some(win) = windows.iter().next() else {
        return;
    };
    let held = weapons.held.as_ref();
    let binocs = zoom.0.is_some() && held.map_or(false, |h| h.stats.class == "Binoculars");
    let scope = zoom.0.is_some() && !binocs && held.map_or(false, |h| h.stats.has_scope);
    let s = ui_scale(win);
    let px = s / win.scale_factor();
    let (w, h) = (
        win.physical_width() as f32 / s,
        win.physical_height() as f32 / s,
    );
    let sw = if binocs { 512.0 } else { 256.0 };
    let sh = 256.0;
    let (fx, fy) = (((w - sw) / 2.0).floor(), ((h - sh) / 2.0).floor());
    for (part, mut node, mut vis) in &mut q {
        let (on, rect) = match *part {
            ScopePart::Border(i) => (
                binocs || scope,
                match i {
                    0 => (0.0, 0.0, w, fy),
                    1 => (0.0, fy + sh, w, h - fy - sh),
                    2 => (0.0, fy, fx, sh),
                    _ => (fx + sw, fy, w - fx - sw, sh),
                },
            ),
            ScopePart::Binoc { half, .. } => (binocs, (fx + 256.0 * half as f32, fy, 256.0, sh)),
            ScopePart::Scope { .. } => (scope, (fx, fy, 256.0, sh)),
        };
        *vis = if on {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if on {
            node.left = Val::Px(rect.0 * px);
            node.top = Val::Px(rect.1 * px);
            node.width = Val::Px(rect.2.max(0.0) * px);
            node.height = Val::Px(rect.3.max(0.0) * px);
        }
    }
}

/// The crosshair is on a hostile pawn (the red accuracy brackets): gamepad aim slowdown reads it.
static ON_ENEMY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn on_enemy() -> bool {
    ON_ENEMY.load(std::sync::atomic::Ordering::Relaxed)
}
