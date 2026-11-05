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
    /// FrobDisplayWindow's coloured strength bars (a locked door, a hackable device).
    pub bars: Option<Bars>,
}

/// The strength bars FrobDisplayWindow draws right of the text, each with its tool count on it.
#[derive(Clone, Debug, PartialEq)]
pub enum Bars {
    /// A locked door: lockStrength on the second line (picks on it when bPickable), doorStrength on the third.
    Door { lock: f32, door: f32, picks: Option<String> },
    /// A hackable device: hackStrength in the lower half (tools on it when bHackable).
    Hack { strength: f32, tools: Option<String> },
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
#[derive(Component, Default)]
pub struct FrobLabel;

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
    // The label (FrobLabel) is a canvas image spawned by frob_draw_system when there is something to show.
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
        if setting!("ToolCounts") > 0.5 {
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
        // Out of reach (the frob ray is short): no brush trace.
        if rt.location.distance(eye) > info.reach + FROB_DIST + 2.0 {
            continue;
        }
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
                // FrobDisplayWindow: "Locked / Lock Str: NN% / Door Str: NN%" (INF when not pickable / breakable),
                // a colour bar for each strength and the number of picks it takes drawn on the lock's bar.
                let brk = matches!(find(&a.props, "bBreakable"), Some(Value::Bool(true)));
                let pct = |v: f64| format!("{}%", ((v * 100.0) + 0.5) as i32);
                let (text, bars) = if rt.locked {
                    let pick = rt.pickable;
                    let text = format!(
                        "Locked\nLock Str: {}\nDoor Str: {}",
                        if pick { pct(rt.lock_strength) } else { "INF".into() },
                        if brk { pct(rt.door_strength) } else { "INF".into() }
                    );
                    let picks = pick.then(|| {
                        let n = ((rt.lock_strength / crate::movers::lockpick_skill()) + 0.99) as i32;
                        tool_text(n, "Lockpick", "pick", "picks")
                    });
                    (text, Some(Bars::Door { lock: rt.lock_strength as f32, door: rt.door_strength as f32, picks }))
                } else {
                    ("Unlocked".to_string(), None)
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
                        bars,
                        blocker: None,
                    },
                ));
            }
        }
    }
    if env_on!("DXVIEW_FROBLOG") {
        let near = blockers.deco.keys().map(|&b| (b, blockers.list[b])).min_by(|a, b| a.1.center.distance(end).total_cmp(&b.1.center.distance(end)));
        if let Some((b, c)) = near {
            println!("FROBLOG eye {eye:.1} end {end:.1} wall {wall:.3} nearest {} at {:.1} r {} h {} ray {:?}", lvl.actors[c.actor].name, c.center, c.radius, c.height, ray_cylinder(eye, end, c.center, c.radius, c.height));
            let _ = b;
        }
    }
    // Decorations and pawns (live actor cylinders).
    let carried = carrying.held.as_ref().map(|h| h.blocker);
    for (&b, &e) in &blockers.deco {
        let c = blockers.list[b];
        if c.radius <= 0.0 || Some(b) == carried || items.get(e).is_ok() {
            continue;
        }
        // The ray first: it is short and rules out nearly every actor, and the property lookups below are what cost.
        if ray_cylinder(eye, end, c.center, c.radius, c.height).is_none_or(|t| t > wall + 0.01) {
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
                let mut bars = None;
                // HackableDevices: "Bypass Str: NN%" with a bar and how many multitools it will take (untrained 0.1
                // each) drawn on it, or "Bypassed" / "INF".
                if crate::devices::is_hackable(&a.props) {
                    let hackable = !matches!(find(&a.props, "bHackable"), Some(Value::Bool(false)));
                    let s = *hacking.strength.get(&b).unwrap_or(&match find(
                        &a.props,
                        "hackStrength",
                    ) {
                        Some(Value::Float(v)) => *v as f64,
                        _ => 0.2,
                    });
                    if s != 0.0 {
                        let tools = hackable.then(|| {
                            let n = ((s / crate::devices::tech_skill()) + 0.99) as i32;
                            tool_text(n, "Multitool", "tool", "tools")
                        });
                        bars = Some(Bars::Hack { strength: s as f32, tools });
                    }
                    text = if !hackable {
                        format!("{text}\nBypass Str: INF")
                    } else if s <= 0.0 {
                        format!("{text}: Bypassed")
                    } else {
                        format!("{text}\nBypass Str: {}%", (s * 100.0 + 0.5) as i32)
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
                        bars,
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
            let text = format!("{}\nBypass Str: {}%", display_name(&a.props), (s * 100.0 + 0.5) as i32);
            let bars = Some(Bars::Hack { strength: s as f32, tools: Some(tool_text(tools, "Multitool", "tool", "tools")) });
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
                    bars,
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
                        bars: None,
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
                if setting!("ItemCounts") > 0.5 {
                    if let Some(Value::Int(n)) = find(&it.props, "NumCopies") {
                        if *n > 1 {
                            name = format!("{name} ({n})");
                        }
                    }
                }
                // GMDX Tool Windows: Show Duplicate Keys (setting DuplicateKeys): a key already on the ring.
                if it.class == "NanoKey" && setting!("DuplicateKeys") > 0.5 {
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
                        bars: None,
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
                if n.searched && setting!("SearchedText") > 0.5 {
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
                        bars: None,
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
        } else {
            // Optional Fixes: the smallest by volume (CollisionRadius^2 x CollisionHeight), not radius alone.
            let size = if crate::optfixes::fixes() { h.ext.x * h.ext.x * h.ext.z } else { r };
            if size < min_size {
                min_size = size;
                best = Some(h);
            }
        }
    }
    // Grab Ladders: nothing else in reach and the line of sight meets a ladder face close by - highlight it with
    // the button to take hold (walking into a ladder doesn't climb it with the setting on, so say how).
    let mut ladder_aim = false;
    if best.is_none() && setting!("LadderGrab") > 0.5 && !LADDER_HELD.load(std::sync::atomic::Ordering::Relaxed) {
        let reach = (FROB_DIST * 0.85).min(wall * FROB_DIST + 2.0);
        let mut near: Option<(f64, DVec3)> = None;
        for l in &lvl.ladders {
            let n = DVec3::from_array(l.normal);
            let den = n.dot(dir);
            if den.abs() < 1e-4 {
                continue;
            }
            let t = (l.dist - n.dot(eye)) / den;
            if t <= 0.0 || t > reach || near.is_some_and(|(b, _)| t >= b) {
                continue;
            }
            let p = eye + dir * t;
            if l.touches(p.to_array(), 0.0, 0.0, 1.0) {
                near = Some((t, p));
            }
        }
        if let Some((_, p)) = near {
            ladder_aim = true;
            let key = if crate::gamepad::pad_in_use() {
                match crate::gamepad::glyph_kind() {
                    crate::gamepad::PadKind::PlayStation => "Square",
                    _ => "X",
                }
            } else {
                "Right Mouse"
            };
            best = Some(Highlight {
                center: p,
                ext: DVec3::new(10.0, 10.0, 20.0),
                mover: false,
                text: format!("Climb Ladder [{key}]"),
                mover_idx: None,
                bars: None,
                blocker: None,
            });
        }
    }
    LADDER_AIM.store(ladder_aim, std::sync::atomic::Ordering::Relaxed);
    HAS_TARGET.store(best.is_some(), std::sync::atomic::Ordering::Relaxed);
    // What a click can frob with a body in hand: a door, or a device that isn't a pawn (pickups::device_like).
    let body_ok = best.as_ref().is_some_and(|h| {
        h.mover_idx.is_some()
            || h.blocker.is_some_and(|b| {
                blockers.list.get(b).is_some_and(|c| {
                    !crate::consys::has_conversations(b) && crate::pickups::device_like(&lvl.actors[c.actor])
                })
            })
    });
    BODY_FROB.store(body_ok, std::sync::atomic::Ordering::Relaxed);
    hl.target = best;
}

/// The frob line meets a ladder in reach (Grab Ladders): Use takes hold of it from there.
pub static LADDER_AIM: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The player is on a ladder (no "Climb Ladder" prompt then).
pub static LADDER_HELD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The highlighted thing is a door or device, which a right click frobs even while carrying a body.
static BODY_FROB: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn body_frob() -> bool {
    BODY_FROB.load(std::sync::atomic::Ordering::Relaxed)
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
    mut label: Query<(Entity, &mut Node, &mut Visibility), (With<FrobLabel>, Without<FrobCorner>)>,
    mut commands: Commands,
    mut hud: ResMut<crate::classic_hud::ClassicHud>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut layers: ResMut<Assets<crate::classic_hud::HudLayer>>,
    mut label_state: Local<(Option<Handle<Image>>, u64)>,
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
                label: &mut Query<(Entity, &mut Node, &mut Visibility), (With<FrobLabel>, Without<FrobCorner>)>| {
        for (_, _, _, mut v) in corners.iter_mut() {
            v.set_if_neq(Visibility::Hidden);
        }
        for (_, _, mut v) in label.iter_mut() {
            v.set_if_neq(Visibility::Hidden);
        }
    };
    let Some(t) = hl
        .target
        .as_ref()
        // (InformationDevices.Frob hides the frob display while its text is up.)
        .filter(|_| !crate::consys::third_person() && !crate::info::reading())
    else {
        hide(&mut corners, &mut label);
        return;
    };
    let Some(c) = project(t.center) else {
        hide(&mut corners, &mut label);
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
        crate::uinode::set_rect(&mut node, (x + d) * px, (y + d) * px, bw * px, bh * px);
        if !fc.shadow && bg.0 != Color::WHITE {
            bg.0 = Color::WHITE;
        }
        vis.set_if_neq(Visibility::Inherited);
    }
    // The info window at the box's top-left corner + 10, kept on screen, drawn as FrobDisplayWindow does it with
    // FontMenuSmall_DS: darkened background, coloured strength bars, text, two translucent outline boxes and the
    // tool count on the bar. Object Names (Game Options) off: the box only - but doors and devices always show.
    if t.text.is_empty() || (!crate::settings::object_names() && t.bars.is_none() && !t.mover) {
        for (_, _, mut v) in label.iter_mut() {
            v.set_if_neq(Visibility::Hidden);
        }
        return;
    }
    let Some(font) = hud.font(&mut assets, "FontMenuSmall_DS") else { return };
    let lines: Vec<&str> = t.text.split('\n').collect();
    const BAR_LEN: i32 = 50;
    let mut iw = lines.iter().map(|l| font.width(l)).max().unwrap_or(0) + 8;
    let ih = font.height * lines.len() as i32 + 8;
    let has_bar = matches!(t.bars, Some(Bars::Door { .. }) | Some(Bars::Hack { .. }));
    if has_bar {
        iw += BAR_LEN + 2;
    }
    let ix = (tlx + 10.0).clamp(iw as f32 / 2.0 + 10.0, (w - 10.0 - iw as f32 / 2.0).max(iw as f32 / 2.0 + 10.0));
    let iy = (tly + 10.0).clamp(ih as f32 / 2.0 + 10.0, (h - 10.0 - ih as f32 / 2.0).max(ih as f32 / 2.0 + 10.0));
    let si = s.max(1.0) as i32;
    let k = si as f32 / win.scale_factor();
    let key = {
        use std::hash::{Hash, Hasher};
        let mut hs = std::collections::hash_map::DefaultHasher::new();
        t.text.hash(&mut hs);
        format!("{:?}", t.bars).hash(&mut hs);
        si.hash(&mut hs);
        (crate::menuui::h_col(0), crate::menuui::h_col(1), crate::menuui::h_col(7)).hash(&mut hs);
        hs.finish()
    };
    let pos = Vec2::new(ix.floor() * k, iy.floor() * k);
    if key != label_state.1 || label.is_empty() {
        label_state.1 = key;
        let c = draw_info(&font, &lines, iw, ih, t.bars.as_ref(), si);
        crate::prompts::present::<FrobLabel, _>(
            &mut commands, &mut images, &mut layers, &mut label, &mut label_state.0, Some(&c), (iw, ih), si, pos, k,
        );
    } else {
        for (_, mut nd, mut v) in label.iter_mut() {
            if nd.left != Val::Px(pos.x) || nd.top != Val::Px(pos.y) {
                nd.left = Val::Px(pos.x);
                nd.top = Val::Px(pos.y);
            }
            v.set_if_neq(Visibility::Inherited);
        }
    }
}

/// FrobDisplayWindow.DrawWindow's info box, in UI units (iw x ih).
fn draw_info(
    font: &crate::uicanvas::UiFont,
    lines: &[&str],
    iw: i32,
    ih: i32,
    bars: Option<&Bars>,
    s: i32,
) -> crate::uicanvas::Canvas {
    use crate::uicanvas::{color_scaled, Canvas, HAlign, Style, VAlign};
    const BAR_LEN: f32 = 50.0;
    let mut c = Canvas::new(iw, ih, s);
    // DSTY_Modulated black over ConWindowBackground: the world behind darkened.
    c.shade(0, 0, iw, ih, 0.75);
    let bx = iw - BAR_LEN as i32 - 4;
    let bar = |c: &mut Canvas, y: i32, h: i32, v: f32| {
        let bw = (BAR_LEN * v.clamp(0.0, 1.0)) as i32;
        if bw > 0 && h > 0 {
            c.fill(bx, y, bw, h, Style::Translucent, color_scaled(v));
        }
    };
    let third = (ih - 8) / 3;
    match bars {
        Some(Bars::Door { lock, door, .. }) => {
            bar(&mut c, 4 + third, third - 2, *lock);
            bar(&mut c, 4 + 2 * third, third - 2, *door);
        }
        Some(Bars::Hack { strength, .. }) => bar(&mut c, ih / 2, ih / 2 - 6, *strength),
        None => {}
    }
    let text = crate::menuui::h_col(7);
    for (i, l) in lines.iter().enumerate() {
        c.text(font, 4, 4 + i as i32 * font.height, iw - 8, font.height, HAlign::Left, VAlign::Top, text, l);
    }
    // The two highlight boxes (1-pixel outlines, translucent): borders, then background inside it.
    let outline = |c: &mut Canvas, x: i32, y: i32, w: i32, h: i32, col| {
        c.fill(x, y, w, 1, Style::Translucent, col);
        c.fill(x, y + h - 1, w, 1, Style::Translucent, col);
        c.fill(x, y + 1, 1, h - 2, Style::Translucent, col);
        c.fill(x + w - 1, y + 1, 1, h - 2, Style::Translucent, col);
    };
    outline(&mut c, 0, 0, iw, ih, crate::menuui::h_col(1));
    outline(&mut c, 1, 1, iw - 2, ih - 2, crate::menuui::h_col(0));
    // The number of picks / tools on top of the bar.
    let tx = iw - BAR_LEN as i32 - 2;
    match bars {
        Some(Bars::Door { picks: Some(p), .. }) => {
            c.text(font, tx, 4 + third, BAR_LEN as i32, third - 2, HAlign::Left, VAlign::Top, text, p);
        }
        Some(Bars::Hack { tools: Some(p), .. }) => {
            c.text(font, tx, ih / 2, BAR_LEN as i32, ih / 2 - 6, HAlign::Left, VAlign::Top, text, p);
        }
        _ => {}
    }
    c
}
