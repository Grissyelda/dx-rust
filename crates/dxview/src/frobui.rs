//! The frob highlight (DeusExPlayer.HighlightCenterObject + FrobDisplayWindow): whatever you could use within
//! reach gets a pulsing cornered box around it and a small label with its name (doors show Locked/Unlocked
//! with lock and door strength).

use crate::actors::{Blockers, Deco};
use crate::ai::{Npc, NpcState};
use crate::carry::Carrying;
use crate::hud::ui_scale;
use crate::movers::Movers;
use crate::pickups::{ray_cylinder, PickupItem};
use crate::{FlyCam, LevelRes, SkyCam};
use bevy::prelude::*;
use dx_level::{bevy_to_ue, ue_to_bevy};
use glam::DVec3;
use unreal_pkg::props::{find, Property, Value};

/// MaxFrobDistance.
const FROB_DIST: f64 = 112.0;

#[derive(Clone, Debug)]
pub struct Highlight {
    pub center: DVec3,
    /// Box half extents (CollisionRadius, CollisionRadius, CollisionHeight; 16 for doors).
    pub ext: DVec3,
    pub mover: bool,
    pub text: String,
    /// Index of the door, when it is one.
    pub mover_idx: Option<usize>,
    /// The decoration or pawn's blocker, when it is one.
    pub blocker: Option<usize>,
}

impl Highlight {
    pub fn mover(&self) -> Option<usize> {
        self.mover_idx
    }
}

#[derive(Resource, Default)]
pub struct FrobHighlight {
    pub target: Option<Highlight>,
    timer: f32,
}

#[derive(Component)]
pub struct FrobCorner {
    index: usize,
    shadow: bool,
}
#[derive(Component)]
pub struct FrobLabel;
#[derive(Component)]
pub struct FrobLabelText;

fn str_prop(p: &[Property], n: &str) -> String {
    match find(p, n) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    }
}
fn f_prop(p: &[Property], n: &str, d: f64) -> f64 {
    match find(p, n) {
        Some(Value::Float(v)) => *v as f64,
        _ => d,
    }
}
/// bHighlight defaults to true on decorations and pawns.
fn highlighted(p: &[Property]) -> bool {
    !matches!(find(p, "bHighlight"), Some(Value::Bool(false)))
}

/// DeusExPlayer.GetDisplayName (nobody has been spoken to yet, so the unfamiliar name).
fn display_name(p: &[Property]) -> String {
    let u = str_prop(p, "UnfamiliarName");
    if !u.is_empty() {
        return u;
    }
    let i = str_prop(p, "ItemName");
    if !i.is_empty() {
        return i;
    }
    str_prop(p, "BindName")
}

pub fn spawn_frob_ui(commands: &mut Commands) {
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
                FrobCorner { index, shadow },
            ));
        }
    }
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                border: UiRect::all(Val::Px(1.0)),
                padding: UiRect::all(Val::Px(4.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
            BorderColor::all(Color::srgba(0.6, 0.6, 0.6, 0.8)),
            Visibility::Hidden,
            Pickable::IGNORE,
            FrobLabel,
        ))
        .with_children(|c| {
            c.spawn((
                Text::new(""),
                TextFont {
                    font_size: bevy::text::FontSize::Px(12.0),
                    ..default()
                },
                TextColor(Color::WHITE),
                Pickable::IGNORE,
                FrobLabelText,
            ));
        });
}

/// HighlightCenterObject, ten times a second: pawns take precedence, a door only if it's the first thing
/// hit, otherwise the smallest frobbable thing along the line.
#[allow(clippy::too_many_arguments)]
pub fn frob_target_system(
    time: Res<Time>,
    cam: Single<&Transform, (With<FlyCam>, Without<SkyCam>)>,
    level: Res<LevelRes>,
    blockers: Res<Blockers>,
    mv: Res<Movers>,
    carrying: Res<Carrying>,
    items: Query<(&PickupItem, Option<&Deco>, Option<&crate::destruct::Loose>)>,
    npcs: Query<(&Npc, &Deco)>,
    mut hl: ResMut<FrobHighlight>,
    hacking: Res<crate::devices::Hacking>,
    grenades: Query<&crate::grenades::Grenade>,
    inv: Res<crate::pickups::PlayerInventory>,
) {
    let _prof = crate::prof::scope("frobui::frob_target_system");
    // GMDX Tool Windows: Tool Display Style (setting ToolCounts): "owned/needed picks".
    let owned = |class: &str| inv.0.items.iter().filter(|i| i.def.class == class).map(|i| i.count).sum::<u32>();
    let tool_text = |needed: i32, class: &str, one: &str, many: &str| {
        if crate::settings::get("ToolCounts") > 0.5 {
            format!("{}/{} {}", owned(class), needed, many)
        } else {
            format!("{} {}", needed, if needed == 1 { one } else { many })
        }
    };
    hl.timer -= time.delta_secs();
    if hl.timer > 0.0 {
        return;
    }
    hl.timer = 0.1;
    let lvl = &level.0;
    let eye = bevy_to_ue(cam.translation.to_array());
    let eye = DVec3::new(eye[0] as f64, eye[1] as f64, eye[2] as f64);
    let f = cam.forward().as_vec3();
    let dir = DVec3::new(f.x as f64, f.z as f64, f.y as f64).normalize();
    // Third person: toward what the camera's centre line is on.
    let dir = crate::thirdperson::aim_dir(lvl, eye, dir);
    let end = eye + dir * FROB_DIST;
    let wall = lvl.collision.trace(eye, end, DVec3::splat(1.0)).fraction;

    // (fraction along the ray, radius for "smallest", is pawn, is mover, highlight)
    let mut hits: Vec<(f64, f64, bool, bool, Highlight)> = Vec::new();
    for (idx, (info, rt)) in lvl.movers.iter().zip(&mv.rt).enumerate() {
        let a = &lvl.actors[info.actor];
        if matches!(find(&a.props, "bHighlight"), Some(Value::Bool(false))) {
            continue;
        }
        if let Some(h) = crate::movers::trace_mover(info, rt, eye, end, DVec3::splat(1.0)) {
            if h.fraction <= wall + 0.01 {
                // Centre of the door's bounding box where it is now.
                let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
                for p in info.parts.iter().flat_map(|p| p.positions.iter()) {
                    for k in 0..3 {
                        lo[k] = lo[k].min(p[k]);
                        hi[k] = hi[k].max(p[k]);
                    }
                }
                let c = [
                    (lo[0] + hi[0]) * 0.5,
                    (lo[1] + hi[1]) * 0.5,
                    (lo[2] + hi[2]) * 0.5,
                ];
                let ax = dx_level::actors::rotator_axes(rt.rotation);
                let mut w = rt.location;
                for k in 0..3 {
                    w.x += (ax[k][0] * c[k]) as f64;
                    w.y += (ax[k][1] * c[k]) as f64;
                    w.z += (ax[k][2] * c[k]) as f64;
                }
                let text = if rt.locked {
                    let pick = rt.pickable;
                    let brk = matches!(find(&a.props, "bBreakable"), Some(Value::Bool(true)));
                    let pct = |v: f64| format!("{}%", ((v * 100.0) + 0.5) as i32);
                    format!(
                        "Locked\nLock Str: {}\nDoor Str: {}",
                        if pick {
                            // The number of lockpicks it takes at the current Lockpicking skill.
                            let n = ((rt.lock_strength / crate::movers::lockpick_skill()) + 0.99)
                                as i32;
                            format!("{}  ({})", pct(rt.lock_strength), tool_text(n, "Lockpick", "pick", "picks"))
                        } else {
                            "INF".into()
                        },
                        if brk {
                            pct(rt.door_strength)
                        } else {
                            "INF".into()
                        }
                    )
                } else {
                    "Unlocked".into()
                };
                hits.push((
                    h.fraction,
                    0.0,
                    false,
                    true,
                    Highlight {
                        center: w,
                        ext: DVec3::splat(16.0),
                        mover: true,
                        text,
                        mover_idx: Some(idx),
                        blocker: None,
                    },
                ));
            }
        }
    }
    // Decorations and pawns (live actor cylinders).
    let carried = carrying.held.as_ref().map(|h| h.blocker);
    for (&b, &e) in &blockers.deco {
        let c = blockers.list[b];
        if c.radius <= 0.0 || Some(b) == carried || items.get(e).is_ok() {
            continue;
        }
        let a = &lvl.actors[c.actor];
        if !highlighted(&a.props) || a.class.ends_with("Carcass") {
            continue;
        }
        let pawn = find(&a.props, "GroundSpeed").is_some();
        if pawn
            && npcs
                .get(e)
                .map(|(n, _)| n.state == NpcState::Dead)
                .unwrap_or(false)
        {
            continue;
        }
        if let Some(t) = ray_cylinder(eye, end, c.center, c.radius, c.height) {
            if t <= wall + 0.01 {
                let mut text = display_name(&a.props);
                // HackableDevices: "Bypass Str: NN%" and how many multitools it will take (untrained 0.1 each), or
                // "Bypassed" / "INF".
                if crate::devices::is_hackable(&a.props) {
                    let hackable = !matches!(find(&a.props, "bHackable"), Some(Value::Bool(false)));
                    let s = *hacking.strength.get(&b).unwrap_or(&match find(
                        &a.props,
                        "hackStrength",
                    ) {
                        Some(Value::Float(v)) => *v as f64,
                        _ => 0.2,
                    });
                    text = if !hackable {
                        format!("{text}\nBypass Str: INF")
                    } else if s <= 0.0 {
                        format!("{text}: Bypassed")
                    } else {
                        let tools = ((s / crate::devices::tech_skill()) + 0.99) as i32;
                        format!(
                            "{text}\nBypass Str: {}%  ({})",
                            (s * 100.0 + 0.5) as i32,
                            tool_text(tools, "Multitool", "tool", "tools")
                        )
                    };
                }
                hits.push((
                    t,
                    c.radius,
                    pawn,
                    false,
                    Highlight {
                        center: c.center,
                        ext: DVec3::new(c.radius, c.radius, c.height),
                        mover: false,
                        text,
                        mover_idx: None,
                        blocker: Some(b),
                    },
                ));
            }
        }
    }
    // GMDX Wireless Strength: nothing in reach, a hackable device further along the view while holding a multitool.
    if hits.is_empty() {
        if let Some((b, t)) = crate::devices::wireless_target(lvl, &blockers, &hacking, eye, dir) {
            let c = blockers.list[b];
            let a = &lvl.actors[c.actor];
            let s = *hacking.strength.get(&b).unwrap_or(&match find(&a.props, "hackStrength") {
                Some(Value::Float(v)) => *v as f64,
                _ => 0.2,
            });
            let tools = ((s / crate::devices::tech_skill()) + 0.99) as i32;
            let text = format!(
                "{}\nBypass Str: {}%  ({})",
                display_name(&a.props),
                (s * 100.0 + 0.5) as i32,
                tool_text(tools, "Multitool", "tool", "tools")
            );
            hits.push((
                t,
                c.radius,
                false,
                false,
                Highlight {
                    center: c.center,
                    ext: DVec3::new(c.radius, c.radius, c.height),
                    mover: false,
                    text,
                    mover_idx: None,
                    blocker: Some(b),
                },
            ));
        }
    }
    // Grenades stuck to walls (and lying about): the same box as the pickup (frobbing one disarms it).
    for g in &grenades {
        if let Some(t) = ray_cylinder(eye, end, g.pos, 8.0, 6.0) {
            if t <= wall + 0.01 {
                let name = match g.class.as_str() {
                    "WeaponLAM" => "Lightweight Attack Munitions (LAM)",
                    "WeaponGasGrenade" => "Gas Grenade",
                    "WeaponEMPGrenade" => "Electromagnetic Pulse (EMP) Grenade",
                    "WeaponNanoVirusGrenade" => "Scramble Grenade",
                    c => c,
                };
                hits.push((
                    t,
                    4.0,
                    false,
                    false,
                    Highlight {
                        center: g.pos,
                        ext: DVec3::new(6.0, 6.0, 6.0),
                        mover_idx: None,
                        blocker: None,
                        mover: false,
                        text: name.to_string(),
                    },
                ));
            }
        }
    }
    // Inventory items lying about.
    for (it, deco, loose) in &items {
        let center = match (deco, loose, it.actor) {
            (Some(d), _, _) => d.body.location,
            (_, Some(l), _) => l.pos,
            (_, _, Some(ai)) => {
                let l = lvl.actors[ai].location();
                DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64)
            }
            _ => continue,
        };
        let (r, h) = (
            f_prop(&it.props, "CollisionRadius", 8.0),
            f_prop(&it.props, "CollisionHeight", 8.0),
        );
        if let Some(t) = ray_cylinder(eye, end, center, r + 4.0, h + 4.0) {
            if t <= wall + 0.01 {
                let mut name = str_prop(&it.props, "ItemName");
                // GMDX Interaction: Show Item Count Labels (setting ItemCounts): "Medkit (3)" for a stack.
                if crate::settings::get("ItemCounts") > 0.5 {
                    if let Some(Value::Int(n)) = find(&it.props, "NumCopies") {
                        if *n > 1 {
                            name = format!("{name} ({n})");
                        }
                    }
                }
                // GMDX Tool Windows: Show Duplicate Keys (setting DuplicateKeys): a key already on the ring.
                if it.class == "NanoKey" && crate::settings::get("DuplicateKeys") > 0.5 {
                    let id = match find(&it.props, "KeyID") {
                        Some(Value::Name(n)) => n.clone(),
                        _ => String::new(),
                    };
                    if !id.is_empty() && crate::keys::list().iter().any(|(k, _)| k.eq_ignore_ascii_case(&id)) {
                        name = format!("{name} (Duplicate)");
                    }
                }
                hits.push((
                    t,
                    r,
                    false,
                    false,
                    Highlight {
                        center,
                        ext: DVec3::new(r, r, h),
                        mover_idx: None,
                        blocker: None,
                        mover: false,
                        text: if name.is_empty() {
                            it.class.clone()
                        } else {
                            name
                        },
                    },
                ));
            }
        }
    }
    // Bodies.
    for (n, d) in &npcs {
        if n.state != NpcState::Dead || n.carried {
            continue;
        }
        let r = n.params.radius * 2.0;
        let c = d.body.location - DVec3::new(0.0, 0.0, n.params.height - 10.0);
        if let Some(t) = ray_cylinder(eye, end, c, r, 14.0) {
            if t <= wall + 0.01 {
                let a = &lvl.actors[n.actor];
                let fam = str_prop(&a.props, "FamiliarName");
                let mut text = if n.knocked_out {
                    "Unconscious".to_string()
                } else if fam.is_empty() {
                    "Dead Body".into()
                } else {
                    format!("Dead Body ({fam})")
                };
                // GMDX Tool Windows: Show Carcass Searched State (setting SearchedText): "[Searched]" in front.
                if n.searched && crate::settings::get("SearchedText") > 0.5 {
                    text = format!("[Searched] {text}");
                }
                hits.push((
                    t,
                    r,
                    false,
                    false,
                    Highlight {
                        center: c,
                        ext: DVec3::new(r, r, 14.0),
                        mover: false,
                        text,
                        mover_idx: None,
                        blocker: None,
                    },
                ));
            }
        }
    }
    hits.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut best: Option<Highlight> = None;
    let mut min_size = f64::MAX;
    for (i, (_, r, pawn, mover, h)) in hits.into_iter().enumerate() {
        if pawn {
            best = Some(h);
            break;
        } else if mover {
            if i == 0 {
                best = Some(h);
                break;
            }
        } else if r < min_size {
            min_size = r;
            best = Some(h);
        }
    }
    HAS_TARGET.store(best.is_some(), std::sync::atomic::Ordering::Relaxed);
    hl.target = best;
}

/// Something is highlighted to frob (the pad's X: use it, else reload).
static HAS_TARGET: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn has_target() -> bool {
    HAS_TARGET.load(std::sync::atomic::Ordering::Relaxed) || LADDER_GRAB.load(std::sync::atomic::Ordering::Relaxed)
}

/// Enhancement "Grab Ladders": a ladder is in reach and not held yet, so the pad's X tap is Use (take hold).
pub static LADDER_GRAB: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// FrobDisplayWindow.DrawWindow: the cornered box (shrinking in and out every 0.3 s) and the name label.
#[allow(clippy::too_many_arguments)]
pub fn frob_draw_system(
    time: Res<Time>,
    windows: Query<&Window>,
    cam: Single<(&Camera, &GlobalTransform), (With<FlyCam>, Without<SkyCam>)>,
    hl: Res<FrobHighlight>,
    mut corners: Query<
        (
            &FrobCorner,
            &mut Node,
            &mut BackgroundColor,
            &mut Visibility,
        ),
        Without<FrobLabel>,
    >,
    mut label: Single<(&mut Node, &mut Visibility), (With<FrobLabel>, Without<FrobCorner>)>,
    mut text: Single<(&mut Text, &mut TextFont), With<FrobLabelText>>,
) {
    let _prof = crate::prof::scope("frobui::frob_draw_system");
    let Some(win) = windows.iter().next() else {
        return;
    };
    let s = ui_scale(win);
    let px = s / win.scale_factor();
    let (w, h) = (
        win.physical_width() as f32 / s,
        win.physical_height() as f32 / s,
    );
    let (camera, gt) = *cam;
    let project = |p: DVec3| -> Option<Vec2> {
        let b = Vec3::from(ue_to_bevy([p.x as f32, p.y as f32, p.z as f32]));
        camera.world_to_viewport(gt, b).ok().map(|v| v / px)
    };
    let hide = |corners: &mut Query<
        (
            &FrobCorner,
            &mut Node,
            &mut BackgroundColor,
            &mut Visibility,
        ),
        Without<FrobLabel>,
    >,
                label: &mut Visibility| {
        for (_, _, _, mut v) in corners.iter_mut() {
            *v = Visibility::Hidden;
        }
        *label = Visibility::Hidden;
    };
    let Some(t) = hl
        .target
        .as_ref()
        .filter(|_| !crate::consys::third_person())
    else {
        hide(&mut corners, &mut label.1);
        return;
    };
    let Some(c) = project(t.center) else {
        hide(&mut corners, &mut label.1);
        return;
    };
    let offset = 24.0 * (time.elapsed_secs() % 0.3);
    let (mut tlx, mut tly, mut brx, mut bry) = (c.x, c.y, c.x, c.y);
    for i in [-1.0, 1.0] {
        for j in [-1.0, 1.0] {
            for k in [-1.0, 1.0] {
                if let Some(q) =
                    project(t.center + DVec3::new(t.ext.x * i, t.ext.y * j, t.ext.z * k))
                {
                    tlx = tlx.min(q.x);
                    tly = tly.min(q.y);
                    brx = brx.max(q.x);
                    bry = bry.max(q.y);
                }
            }
        }
    }
    if !t.mover {
        tlx += (t.ext.x / 4.0) as f32;
        tly += (t.ext.z / 4.0) as f32;
        brx -= (t.ext.x / 4.0) as f32;
        bry -= (t.ext.z / 4.0) as f32;
    }
    let margin = 10.0;
    tlx = tlx.clamp(margin, w - margin);
    tly = tly.clamp(margin, h - margin);
    brx = brx.clamp(margin, w - margin);
    bry = bry.clamp(margin, h - margin);
    let corner = ((brx - tlx + bry - tly) * 0.1).clamp(4.0, 40.0);
    if brx - tlx < corner {
        tlx -= corner + 4.0;
        brx += corner + 4.0;
    }
    if bry - tly < corner {
        tly -= corner + 4.0;
        bry += corner + 4.0;
    }
    let o = offset;
    // (x, y, w, h) of the eight strokes.
    let boxes = [
        (tlx + o, tly + o, corner, 1.0),
        (tlx + o, tly + o, 1.0, corner),
        (brx - corner - o, tly + o, corner, 1.0),
        (brx - o, tly + o, 1.0, corner),
        (tlx + o, bry - o, corner, 1.0),
        (tlx + o, bry - corner - o, 1.0, corner),
        (brx - corner + 1.0 - o, bry - o, corner, 1.0),
        (brx - o, bry - corner - o, 1.0, corner),
    ];
    for (fc, mut node, mut bg, mut vis) in &mut corners {
        let (x, y, bw, bh) = boxes[fc.index];
        let d = if fc.shadow { 1.0 } else { 0.0 };
        node.left = Val::Px((x + d) * px);
        node.top = Val::Px((y + d) * px);
        node.width = Val::Px(bw * px);
        node.height = Val::Px(bh * px);
        if !fc.shadow {
            bg.0 = Color::WHITE;
        }
        *vis = Visibility::Inherited;
    }
    // Name label at the box's top-left corner, kept on screen.
    // Object Names (Game Options) off: the box only.
    if t.text.is_empty() || !crate::settings::object_names() {
        *label.1 = Visibility::Hidden;
        return;
    }
    text.0 .0 = t.text.clone();
    text.1.font_size = bevy::text::FontSize::Px(7.0 * px);
    label.0.left = Val::Px((tlx + 10.0).clamp(10.0, w - 60.0) * px);
    label.0.top = Val::Px((tly + 10.0).clamp(10.0, h - 30.0) * px);
    label.0.padding = UiRect::all(Val::Px(4.0 * px));
    *label.1 = Visibility::Inherited;
}
