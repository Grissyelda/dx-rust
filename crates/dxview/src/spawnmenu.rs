//! Developer item spawner (Insert): a grid of every inventory item the game defines, with its belt icon. Clicking one
//! drops it on the ground in front of the player, ready to pick up. Also: god mode, and installing any augmentation
//! (what a medical bot does with a canister). Esc or Insert closes it.

use crate::weapons::RuntimeAssets;
use crate::{FlyCam, LevelRes, Player, SkyCam};
use bevy::{
    asset::RenderAssetUsages,
    image::{ImageSampler, ImageSamplerDescriptor},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    window::{CursorGrabMode, CursorOptions},
};
use glam::DVec3;
use unreal_pkg::props::{find, Value};

/// Pawn classes offered by the spawner.
const PAWNS: &[&str] = &[
    "Rat",
    "Cat",
    "Mutt",
    "Doberman",
    "Greasel",
    "Karkian",
    "KarkianBaby",
    "Gray",
    "Terrorist",
    "UNATCOTroop",
    "MJ12Troop",
    "MJ12Commando",
    "SecurityBot2",
    "SecurityBot3",
    "MilitaryBot",
    "SpiderBot2",
    "MedicalBot",
    "RepairBot",
    "CleanerBot",
];

/// Every augmentation an AugmentationCannister can hold (the three you start with are already installed): a button
/// installs one the way a medical bot does, and each further click upgrades it a level.
const AUGS: &[&str] = &[
    "AugAqualung",
    "AugBallistic",
    "AugCloak",
    "AugCombat",
    "AugDefense",
    "AugDrone",
    "AugEMP",
    "AugEnviro",
    "AugHealing",
    "AugHeartLung",
    "AugMuscle",
    "AugPower",
    "AugRadarTrans",
    "AugShield",
    "AugSpeed",
    "AugStealth",
    "AugTarget",
    "AugVision",
];

#[derive(Resource, Default)]
pub struct SpawnMenu {
    pub open: bool,
    built: bool,
}

#[derive(Component)]
pub struct SpawnMenuRoot;
#[derive(Component)]
pub struct SpawnButton(String);

/// Every placeable inventory class in DeusEx.u: has a belt Icon and a real ItemName (skips the NPC-only weapons
/// and internal ammo types), grouped weapons, ammo, then everything else.
fn item_classes(assets: &mut RuntimeAssets) -> Vec<(String, String, String)> {
    let names: Vec<String> = match assets.classes.packages.get("deusex") {
        Some(p) => p
            .exports
            .iter()
            .filter(|e| p.class_name(e.class_index) == "Class")
            .map(|e| e.name.clone())
            .collect(),
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    for n in names {
        let d = assets.classes.defaults(&n);
        let Some(Value::Path(icon)) = find(&d, "Icon") else {
            continue;
        };
        let Some(Value::Str(name)) = find(&d, "ItemName") else {
            continue;
        };
        if icon.starts_with("Engine.") || name.contains("REPORT THIS") || find(&d, "Mesh").is_none()
        {
            continue;
        }
        out.push((n.clone(), name.clone(), icon.clone()));
    }
    let rank = |c: &str| {
        if c.starts_with("WeaponMod") {
            3
        } else if c.starts_with("Weapon") {
            0
        } else if c.starts_with("Ammo") {
            1
        } else {
            2
        }
    };
    out.sort_by(|a, b| rank(&a.0).cmp(&rank(&b.0)).then(a.1.cmp(&b.1)));
    out
}

fn build(commands: &mut Commands, assets: &mut RuntimeAssets, images: &mut Assets<Image>) {
    let items = item_classes(assets);
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(5.0),
                top: Val::Percent(6.0),
                width: Val::Percent(90.0),
                height: Val::Percent(84.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(12.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.03, 0.04, 0.06, 0.92)),
            BorderColor::all(Color::srgba(0.6, 0.65, 0.7, 0.9)),
            GlobalZIndex(50),
            Visibility::Hidden,
            SpawnMenuRoot,
        ))
        .with_children(|root| {
            root.spawn((
                Text::new(format!("SPAWN ITEM  -  click to drop it in front of you  ({} items, Insert / Esc to close)", items.len())),
                TextFont { font_size: bevy::text::FontSize::Px(16.0), ..default() },
                TextColor(Color::srgb(0.9, 0.92, 0.95)),
                Node { margin: UiRect::bottom(Val::Px(10.0)), ..default() },
            ));
            root.spawn(Node { flex_direction: FlexDirection::Row, flex_wrap: FlexWrap::Wrap, row_gap: Val::Px(6.0), column_gap: Val::Px(6.0), overflow: Overflow::scroll_y(), flex_grow: 1.0, ..default() }).with_children(|grid| {
                // Debug toggles and augmentations.
                let cans: Vec<(String, String, (f32, f32, f32))> = crate::augcan::VARIANTS
                    .iter()
                    .map(|[a, b]| (format!("CAN:{a},{b}"), format!("Aug canister: {} / {}", &a[3..], &b[3..]), (0.08, 0.14, 0.22)))
                    .collect();
                for (key, label, col) in [
                    ("GOD".to_string(), "God mode (no damage): toggle".to_string(), (0.12, 0.16, 0.12)),
                    ("SKILLPTS".to_string(), "Add 1000 skill points".to_string(), (0.12, 0.16, 0.12)),
                    ("QUOTES".to_string(), "Show the quotes window".to_string(), (0.12, 0.16, 0.12)),
                    // The original's cheats (DeusExPlayer exec functions).
                    ("CHEAT:AllHealth".to_string(), "AllHealth: heal fully".to_string(), (0.16, 0.12, 0.08)),
                    ("CHEAT:AllEnergy".to_string(), "AllEnergy: bioenergy full".to_string(), (0.16, 0.12, 0.08)),
                    ("CHEAT:AllAmmo".to_string(), "AllAmmo: ammo to max".to_string(), (0.16, 0.12, 0.08)),
                    ("CHEAT:AllWeapons".to_string(), "AllWeapons: every weapon".to_string(), (0.16, 0.12, 0.08)),
                    ("CHEAT:AllCredits".to_string(), "AllCredits: 100,000 credits".to_string(), (0.16, 0.12, 0.08)),
                    ("CHEAT:AllSkills".to_string(), "AllSkills: every skill Master".to_string(), (0.16, 0.12, 0.08)),
                    ("ALLAUGS".to_string(), "AllAugs: every augmentation".to_string(), (0.16, 0.12, 0.08)),
                    ("INVISIBLE".to_string(), "Invisible: AI can't see you (toggle)".to_string(), (0.16, 0.12, 0.08)),
                ]
                    .into_iter()
                    .chain(cans)
                    .chain(AUGS.iter().map(|a| (format!("AUG:{a}"), format!("Install / upgrade {}", &a[3..]), (0.10, 0.12, 0.20))))
                {
                    grid.spawn((
                        Button,
                        Node { width: Val::Px(112.0), height: Val::Px(40.0), align_items: AlignItems::Center, justify_content: JustifyContent::Center, border: UiRect::all(Val::Px(1.0)), ..default() },
                        BackgroundColor(Color::srgba(col.0, col.1, col.2, 0.9)),
                        BorderColor::all(Color::srgba(0.4, 0.45, 0.6, 1.0)),
                        SpawnButton(key),
                    ))
                    .with_children(|b| {
                        b.spawn((Text::new(label), TextFont { font_size: bevy::text::FontSize::Px(11.0), ..default() }, TextColor(Color::srgb(0.85, 0.9, 1.0)), TextLayout::justify(Justify::Center), Node { max_width: Val::Px(106.0), ..default() }, Pickable::IGNORE));
                    });
                }
                // Pawns: every animal (most never appear on the Island) and a few people and robots to fight.
                for class in PAWNS {
                    grid.spawn((
                        Button,
                        Node { width: Val::Px(112.0), height: Val::Px(40.0), align_items: AlignItems::Center, justify_content: JustifyContent::Center, border: UiRect::all(Val::Px(1.0)), ..default() },
                        BackgroundColor(Color::srgba(0.16, 0.12, 0.12, 0.9)),
                        BorderColor::all(Color::srgba(0.5, 0.35, 0.35, 1.0)),
                        SpawnButton(format!("PAWN:{class}")),
                    ))
                    .with_children(|b| {
                        b.spawn((Text::new(format!("{class}")), TextFont { font_size: bevy::text::FontSize::Px(11.0), ..default() }, TextColor(Color::srgb(0.95, 0.85, 0.85)), Pickable::IGNORE));
                    });
                }
                for (class, name, icon) in items {
                    let img = assets.lib.load_image_masked(&icon).map(|t| {
                        let mut im = Image::new(Extent3d { width: t.width, height: t.height, depth_or_array_layers: 1 }, TextureDimension::D2, t.mips[0].clone(), TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD);
                        im.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
                        images.add(im)
                    });
                    grid.spawn((
                        Button,
                        Node {
                            width: Val::Px(112.0),
                            height: Val::Px(84.0),
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::Center,
                            border: UiRect::all(Val::Px(1.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.12, 0.14, 0.18, 0.9)),
                        BorderColor::all(Color::srgba(0.35, 0.4, 0.45, 1.0)),
                        SpawnButton(class),
                    ))
                    .with_children(|b| {
                        if let Some(h) = img {
                            b.spawn((ImageNode::new(h), Node { width: Val::Px(42.0), height: Val::Px(37.0), ..default() }, Pickable::IGNORE));
                        }
                        b.spawn((
                            Text::new(name),
                            TextFont { font_size: bevy::text::FontSize::Px(11.0), ..default() },
                            TextColor(Color::srgb(0.85, 0.88, 0.9)),
                            TextLayout::justify(Justify::Center),
                            Node { max_width: Val::Px(106.0), margin: UiRect::top(Val::Px(3.0)), ..default() },
                            Pickable::IGNORE,
                        ));
                    });
                }
            });
        });
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_menu_system(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut menu: ResMut<SpawnMenu>,
    mut assets: ResMut<RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut cursor: Single<&mut CursorOptions>,
    mut root: Query<&mut Visibility, With<SpawnMenuRoot>>,
    mut buttons: Query<(&Interaction, &SpawnButton, &mut BackgroundColor), Changed<Interaction>>,
    (player, level, cam, mut drops, mut hud): (
        Res<Player>,
        Res<LevelRes>,
        Single<&Transform, (With<FlyCam>, Without<SkyCam>)>,
        ResMut<crate::ai::WeaponDrops>,
        ResMut<crate::movers::HudMessage>,
    ),
    mut inv: ResMut<crate::pickups::PlayerInventory>,
) {
    if !menu.built {
        menu.built = true;
        build(&mut commands, &mut assets, &mut images);
    }
    // Debug Mode (Settings > Enhancements) off: no spawn menu, and its god mode goes too.
    if !crate::settings::debug_mode() {
        if menu.open {
            menu.open = false;
        }
        crate::ai::GOD_MODE.store(false, std::sync::atomic::Ordering::Relaxed);
    }
    let toggle = (keys.just_pressed(KeyCode::Insert) && crate::settings::debug_mode())
        || (menu.open && keys.just_pressed(KeyCode::Escape));
    if toggle {
        menu.open = !menu.open;
        if menu.open {
            cursor.visible = true;
            cursor.grab_mode = CursorGrabMode::None;
        }
    }
    for mut v in &mut root {
        *v = if menu.open {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    if !menu.open {
        return;
    }
    for (i, b, mut bg) in &mut buttons {
        match i {
            Interaction::Pressed => {
                // Drop it a little in front of the player at chest height, short of any wall.
                let f = cam.forward().as_vec3();
                let fwd = DVec3::new(f.x as f64, f.z as f64, 0.0).normalize_or_zero();
                let from = player.body.location + DVec3::new(0.0, 0.0, 10.0);
                let want = from + fwd * (player.params.radius + 40.0);
                let h = level.0.collision.trace(from, want, DVec3::splat(6.0));
                let at = from + (want - from) * (h.fraction - 0.05).max(0.0);
                if b.0 == "GOD" {
                    let on = !crate::ai::GOD_MODE.load(std::sync::atomic::Ordering::Relaxed);
                    crate::ai::GOD_MODE.store(on, std::sync::atomic::Ordering::Relaxed);
                    hud.text = format!("God mode {}", if on { "ON" } else { "OFF" });
                    hud.time_left = 2.0;
                    *bg = BackgroundColor(Color::srgba(0.3, 0.45, 0.3, 0.95));
                    continue;
                }
                if let Some(c) = b.0.strip_prefix("CHEAT:") {
                    let c: &'static str = match c {
                        "AllHealth" => "AllHealth",
                        "AllEnergy" => "AllEnergy",
                        "AllAmmo" => "AllAmmo",
                        "AllWeapons" => "AllWeapons",
                        "AllCredits" => "AllCredits",
                        _ => "AllSkills",
                    };
                    crate::missions::player_req(crate::missions::PlayerReq::Cheat(c));
                    hud.text = c.to_string();
                    hud.time_left = 2.0;
                    continue;
                }
                if b.0 == "ALLAUGS" {
                    for aug in AUGS {
                        let _ = crate::abilities::give_aug(&mut assets.classes, aug, true);
                    }
                    hud.text = "AllAugs".into();
                    hud.time_left = 2.0;
                    continue;
                }
                if b.0 == "INVISIBLE" {
                    let on = !crate::abilities::CHEAT_INVISIBLE.fetch_xor(true, std::sync::atomic::Ordering::Relaxed);
                    hud.text = format!("Invisible {}", if on { "ON" } else { "OFF" });
                    hud.time_left = 2.0;
                    continue;
                }
                if b.0 == "QUOTES" {
                    // DeusExPlayer.ShowQuotesWindow (a cheat).
                    crate::credits::show_quotes();
                    continue;
                }
                if b.0 == "SKILLPTS" {
                    hud.text = crate::abilities::award_points(1000).unwrap_or_default();
                    hud.time_left = 2.5;
                    *bg = BackgroundColor(Color::srgba(0.3, 0.45, 0.3, 0.95));
                    continue;
                }
                if let Some(pair) = b.0.strip_prefix("CAN:") {
                    // An augmentation canister holding this pair, straight into the inventory.
                    let mut it = pair.split(',');
                    let augs = [it.next().unwrap_or("").to_string(), it.next().unwrap_or("").to_string()];
                    let d = assets.classes.defaults("AugmentationCannister");
                    let def = crate::pickups::item_def("AugmentationCannister", &d);
                    hud.text = crate::augcan::add(&mut *inv, &def, augs).unwrap_or_else(|e| e);
                    hud.time_left = 2.5;
                    *bg = BackgroundColor(Color::srgba(0.3, 0.45, 0.3, 0.95));
                    continue;
                }
                if let Some(aug) = b.0.strip_prefix("AUG:") {
                    let m = crate::abilities::give_aug(&mut assets.classes, aug, true).unwrap_or_default();
                    hud.text = if m.is_empty() { format!("Can't install {aug}") } else { m };
                    hud.time_left = 2.5;
                    *bg = BackgroundColor(Color::srgba(0.3, 0.45, 0.3, 0.95));
                    continue;
                }
                if let Some(class) = b.0.strip_prefix("PAWN:") {
                    // A pawn stands further out, facing the player.
                    let want = from + fwd * (player.params.radius + 140.0);
                    let h = level.0.collision.trace(from, want, DVec3::splat(20.0));
                    let at = from
                        + (want - from) * (h.fraction - 0.05).max(0.0)
                        + DVec3::new(0.0, 0.0, 20.0);
                    let yaw = ((-fwd.y).atan2(-fwd.x) / std::f64::consts::TAU * 65536.0) as i32;
                    crate::spawn::pawn_with(
                        class,
                        at,
                        yaw,
                        crate::spawn::hostile_animal_alliance(class)
                            .into_iter()
                            .collect(),
                    );
                } else {
                    drops.0.push((b.0.clone(), at));
                }
                hud.text = format!("Spawned {}", b.0);
                hud.time_left = 1.5;
                *bg = BackgroundColor(Color::srgba(0.3, 0.45, 0.3, 0.95));
            }
            Interaction::Hovered => *bg = BackgroundColor(Color::srgba(0.22, 0.26, 0.32, 0.95)),
            Interaction::None => *bg = BackgroundColor(Color::srgba(0.12, 0.14, 0.18, 0.9)),
        }
    }
}
