//! Frobbing (the "use" key) and picking items up into the inventory, plus the item belt on the HUD.

use crate::actors::Deco;
use crate::movers::{self, HudMessage, Movers};
use crate::{FlyCam, LevelRes, Player, SkyCam};
use bevy::{
    asset::RenderAssetUsages,
    image::{ImageSampler, ImageSamplerDescriptor},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use dx_level::bevy_to_ue;
use glam::DVec3;
use imsim_core::inventory::{Inventory, ItemDef, ItemKind};
use std::collections::HashMap;
use std::path::PathBuf;
use unreal_pkg::props::{find, Property, Value};

#[derive(Resource)]
pub struct GameRoot(pub PathBuf);

/// Set when the player presses use (or a test asks for it).
#[derive(Resource, Default)]
pub struct FrobRequest(pub bool);

#[derive(Resource, Default)]
pub struct PlayerInventory(pub Inventory);

/// A placed inventory item that can be picked up.
#[derive(Component)]
pub struct PickupItem {
    /// The placed actor it came from (None for items spawned at runtime, e.g. crate contents).
    pub actor: Option<usize>,
    pub class: String,
    pub props: std::sync::Arc<Vec<Property>>,
}

fn s(p: &[Property], n: &str) -> String {
    match find(p, n) {
        Some(Value::Str(v)) => v.clone(),
        _ => String::new(),
    }
}
fn i(p: &[Property], n: &str) -> Option<i32> {
    match find(p, n) {
        Some(Value::Int(v)) => Some(*v),
        _ => None,
    }
}
fn path(p: &[Property], n: &str) -> Option<String> {
    match find(p, n) {
        Some(Value::Path(v)) if v != "None" => Some(v.clone()),
        _ => None,
    }
}

/// Describe a placed item from its effective properties.
pub fn item_def(class: &str, p: &[Property]) -> ItemDef {
    let kind = if let Some(n) = i(p, "numCredits") {
        ItemKind::Credits {
            amount: n.max(0) as u32,
        }
    } else if let Some(n) = i(p, "AmmoAmount") {
        ItemKind::Ammo {
            amount: n.max(0) as u32,
        }
    } else if find(p, "AmmoName").is_some() {
        ItemKind::Weapon
    } else {
        // bCanHaveMultipleCopies: stacks up to MaxCopies (0 = no limit); otherwise every one is its own item.
        ItemKind::Pickup {
            max_copies: if matches!(find(p, "bCanHaveMultipleCopies"), Some(Value::Bool(true))) {
                match i(p, "maxCopies") {
                    // GMDX Combat Medic's Bag: PerkValue (5) more medkits and bio cells.
                    Some(n) if n > 0 && matches!(class, "MedKit" | "BioelectricCell") => {
                        n as u32 + crate::perks::value_if("PerkCombatMedicsBag").unwrap_or(0.0) as u32
                    }
                    Some(n) if n > 0 => n as u32,
                    _ => u32::MAX,
                }
            } else {
                1
            },
        }
    };
    ItemDef {
        class: class.to_string(),
        name: s(p, "ItemName"),
        article: {
            let a = s(p, "ItemArticle");
            if a.is_empty() {
                "a".into()
            } else {
                a
            }
        },
        pickup_message: s(p, "PickupMessage"),
        belt_name: s(p, "beltDescription"),
        icon: path(p, "Icon"),
        kind,
        inv_size: (
            i(p, "invSlotsX").unwrap_or(1).max(1) as u32,
            i(p, "invSlotsY").unwrap_or(1).max(1) as u32,
        ),
        large_icon: path(p, "largeIcon"),
        large_size: (
            i(p, "largeIconWidth").unwrap_or(42).max(1) as u32,
            i(p, "largeIconHeight").unwrap_or(37).max(1) as u32,
        ),
        description: s(p, "Description"),
    }
}

/// New-game inventory (DeusExPlayer.ResetPlayer), described from the classes' own defaults.
pub fn starting_inventory(root: &std::path::Path) -> PlayerInventory {
    let mut db = unreal_pkg::classes::ClassDb::open(&root.join("System"));
    let mut inv = Inventory::default();
    // DXVIEW_GIVE="ClassA,ClassB" adds test items to the kit.
    let extra: Vec<String> = std::env::var("DXVIEW_GIVE")
        .map(|v| {
            v.split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let base = [
        "WeaponPistol",
        "WeaponProd",
        "MedKit",
    ];
    for class in base.iter().map(|s| s.to_string()).chain(extra) {
        let class = class.as_str();
        let d = db.defaults(class);
        if !d.is_empty() {
            let _ = inv.pick_up(&item_def(class, &d), 1);
            add_weapon_ammo(&mut inv, &d);
        }
    }
    // ResetPlayer: Credits = Default.Credits (JCDentonMale: 500).
    inv.credits = match unreal_pkg::props::find(&db.defaults("JCDentonMale"), "Credits") {
        Some(Value::Int(n)) => (*n).max(0) as u32,
        _ => 500,
    };
    PlayerInventory(inv)
}

/// A picked-up weapon brings its PickupAmmoCount rounds of its ammo type.
pub fn add_weapon_ammo(inv: &mut Inventory, p: &[Property]) {
    if let (Some(Value::Path(a)), Some(n)) = (find(p, "AmmoName"), i(p, "PickupAmmoCount")) {
        if a.ends_with("AmmoNone") || n <= 0 {
            return;
        }
        let class = a.rsplit('.').next().unwrap_or("").to_string();
        match inv.ammo.iter_mut().find(|x| x.0 == class) {
            Some(x) => x.1 += n as u32,
            None => inv.ammo.push((class, n as u32)),
        }
    }
}

/// Distance along a ray to a vertical cylinder, as a fraction of the ray, if it hits.
pub fn ray_cylinder(start: DVec3, end: DVec3, c: DVec3, r: f64, h: f64) -> Option<f64> {
    let d = end - start;
    let s = start - c;
    let mut best: Option<f64> = None;
    let a = d.x * d.x + d.y * d.y;
    if a > 1e-12 {
        let b = 2.0 * (s.x * d.x + s.y * d.y);
        let cc = s.x * s.x + s.y * s.y - r * r;
        let disc = b * b - 4.0 * a * cc;
        if disc >= 0.0 {
            let t = (-b - disc.sqrt()) / (2.0 * a);
            if (0.0..=1.0).contains(&t) && (s.z + d.z * t).abs() <= h {
                best = Some(t);
            }
        }
    }
    if d.z.abs() > 1e-12 {
        for plane in [h, -h] {
            let t = (plane - s.z) / d.z;
            if (0.0..=1.0).contains(&t) {
                let p = s + d * t;
                if p.x * p.x + p.y * p.y <= r * r && best.is_none_or(|b| t < b) {
                    best = Some(t);
                }
            }
        }
    }
    best
}

/// What a frob can land on (frob_system): the nearest along the eye's ray.
enum Target {
    Mover(usize),
    Item(Entity),
    Body(Entity),
    Device(usize),
    Grenade(Entity),
    Bot(Entity),
    /// A pawn or decoration with conversations (Frob -> StartConversation(IM_Frob)).
    Talk(usize),
}

/// The frob ray: from the eye along `dir`, MaxFrobDistance (112 units) to `end`; `wall` = the fraction where the
/// level stops it.
#[derive(Clone, Copy)]
struct FrobRay {
    eye: DVec3,
    dir: DVec3,
    end: DVec3,
    small: DVec3,
    wall: f64,
}

/// What frob_system's per-target steps read and write. Resources are reborrowed as `Mut`, so a step marks one
/// changed only where it writes it, as before (belt_ui watches the inventory).
struct FrobCx<'a> {
    level: &'a dx_level::Level,
    player: &'a Player,
    blockers: &'a crate::actors::Blockers,
    /// A left click with a lockpick, multitool or the key ring (ParseLeftClick: DoFrob(Self, inHand)).
    only_tool: bool,
    seed: &'a mut u64,
    hud: Mut<'a, HudMessage>,
    inv: Mut<'a, PlayerInventory>,
    mv: Mut<'a, Movers>,
    assets: Mut<'a, crate::weapons::RuntimeAssets>,
    weapons: Mut<'a, crate::weapons::Weapons>,
}

/// The queries frob_system's steps look through (pickups, pawns / bodies, grenades).
type FrobItems<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static PickupItem,
        Option<&'static Deco>,
        Option<&'static crate::destruct::Loose>,
    ),
>;
type FrobBodies<'w, 's> = Query<'w, 's, (Entity, &'static mut crate::ai::Npc, &'static Deco)>;
type FrobGrenades<'w, 's> = Query<'w, 's, (Entity, &'static mut crate::grenades::Grenade)>;

#[allow(clippy::too_many_arguments)]
pub fn frob_system(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut req: ResMut<FrobRequest>,
    cam: Single<&Transform, (With<FlyCam>, Without<SkyCam>)>,
    mut player: ResMut<Player>,
    level: Res<LevelRes>,
    mut mv: ResMut<Movers>,
    mut hud: ResMut<HudMessage>,
    mut inv: ResMut<PlayerInventory>,
    items: FrobItems,
    (mut bodies, mut assets, mut seed): (
        FrobBodies,
        ResMut<crate::weapons::RuntimeAssets>,
        Local<u64>,
    ),
    carrying: Res<crate::carry::Carrying>,
    (mut weapons, mut vis, mut picking): (
        ResMut<crate::weapons::Weapons>,
        Query<&mut Visibility, With<crate::ai::Npc>>,
        ResMut<movers::Picking>,
    ),
    (blockers, mut hacking, mut events, mut info, _root_dir, _ph): (
        Res<crate::actors::Blockers>,
        ResMut<crate::devices::Hacking>,
        ResMut<crate::devices::GameEvents>,
        ResMut<crate::info::InfoDisplay>,
        Res<GameRoot>,
        ResMut<crate::ai::PlayerHealth>,
    ),
    mut grenades: FrobGrenades,
) {
    let _prof = crate::prof::scope("pickups::frob_system");
    debug_unlock(&keys, &mut player, &mut hud);
    // ParseLeftClick: with a lockpick or the key ring in hand a left click frobs the door it points at, with a
    // multitool a hackable device (DoFrob(Self, inHand)); right-click frobs anything.
    let tool = weapons
        .held
        .as_ref()
        .and_then(|h| ["lockpick", "multitool", "nanokeyring"].into_iter().find(|t| h.stats.class.eq_ignore_ascii_case(t)));
    let left_tool = tool.is_some() && mouse.just_pressed(MouseButton::Left);
    let wants = mouse.just_pressed(MouseButton::Right) || req.0 || left_tool;
    let only_tool = left_tool && !mouse.just_pressed(MouseButton::Right) && !req.0;
    req.0 = false;
    // Carrying a body (POVCorpse in hand), ParseRightClick still frobs a door or a device it points at; anything
    // else and the click throws the body (weapons.rs, which asks frobui::body_frob() first).
    let with_body = weapons.corpse.is_some();
    if !wants || carrying.consumed || carrying.held.is_some() || (with_body && !crate::frobui::body_frob()) {
        return;
    }
    // Ray from the eye, MaxFrobDistance (112 units) long.
    let tf = *cam;
    let eye = bevy_to_ue(tf.translation.to_array());
    let eye = DVec3::new(eye[0] as f64, eye[1] as f64, eye[2] as f64);
    let f = tf.forward().as_vec3();
    let dir = DVec3::new(f.x as f64, f.z as f64, f.y as f64).normalize();
    let dir = crate::thirdperson::aim_dir(&level.0, eye, dir);
    let end = eye + dir * 112.0;
    let small = DVec3::splat(1.0);
    let level = &level.0;
    let wall = level.collision.trace(eye, end, small).fraction;
    let ray = FrobRay { eye, dir, end, small, wall };

    let best = nearest_target(level, &blockers, &mv, ray, &items, &bodies, &grenades);
    let best = narrow_target(best, level, &blockers, &hacking, ray, tool, only_tool, with_body);
    let mut cx = FrobCx {
        level,
        player: &player,
        blockers: &blockers,
        only_tool,
        seed: &mut seed,
        hud: hud.reborrow(),
        inv: inv.reborrow(),
        mv: mv.reborrow(),
        assets: assets.reborrow(),
        weapons: weapons.reborrow(),
    };
    futz_owned(&cx, &best, &items);
    match best {
        Some((Target::Talk(b), _)) => {
            // Multiplayer: conversations are the host's.
            if crate::netplay::conversations_blocked() {
                cx.hud.text = "Only the host can talk to people.".into();
                cx.hud.time_left = 2.0;
            } else {
                crate::consys::request_frob(b);
            }
        }
        Some((Target::Mover(idx), _)) => frob_mover(&mut cx, idx, &mut picking),
        Some((Target::Item(e), _)) => frob_item(&mut cx, e, &items, &mut commands),
        Some((Target::Body(e), _)) => frob_body(&mut cx, e, &mut bodies, &mut vis),
        Some((Target::Bot(e), _)) => frob_bot(&mut cx, e, &mut bodies),
        Some((Target::Grenade(e), _)) => frob_grenade(&mut cx, e, &mut grenades, &mut commands),
        Some((Target::Device(b), _)) => frob_device(&mut cx, b, &mut hacking, &mut events, &mut info),
        None => put_away(&mut cx.weapons),
    }
}

/// Debug Mode's PgUp: every locked door can be opened (off again outside Debug Mode).
fn debug_unlock(keys: &ButtonInput<KeyCode>, player: &mut ResMut<Player>, hud: &mut ResMut<HudMessage>) {
    if !crate::settings::debug_mode() && player.unlock_all {
        player.unlock_all = false;
    }
    if keys.just_pressed(KeyCode::PageUp) && crate::settings::debug_mode() {
        player.unlock_all = !player.unlock_all;
        hud.text = if player.unlock_all {
            "Debug: locked doors can be opened (PgUp to turn off)".into()
        } else {
            "Debug: door locks back on".into()
        };
        hud.time_left = 3.0;
    }
}

/// The nearest frobbable thing along the ray, short of the wall: grenades, usable decorations (talkers, devices),
/// movers, items, medical / repair bots, talking pawns, bodies.
fn nearest_target(
    level: &dx_level::Level,
    blockers: &crate::actors::Blockers,
    mv: &Movers,
    ray: FrobRay,
    items: &FrobItems,
    bodies: &FrobBodies,
    grenades: &FrobGrenades,
) -> Option<(Target, f64)> {
    let FrobRay { eye, end, small, wall, .. } = ray;
    let mut best: Option<(Target, f64)> = None;
    // Grenades (placed mines, thrown ones lying about).
    for (e, g) in grenades.iter() {
        if let Some(t) = ray_cylinder(eye, end, g.pos, 8.0, 6.0) {
            if t <= wall + 0.01 && best.as_ref().is_none_or(|(_, f)| t < *f) {
                best = Some((Target::Grenade(e), t));
            }
        }
    }
    // Usable decorations: buttons/switches with an Event, and hackable devices.
    for (&b, &de) in &blockers.deco {
        let c = blockers.list[b];
        if c.radius <= 0.0 || items.get(de).is_ok() {
            continue;
        }
        let p = &level.actors[c.actor].props;
        if find(p, "GroundSpeed").is_none() && crate::consys::has_conversations(b) {
            if let Some(t) = ray_cylinder(eye, end, c.center, c.radius + 2.0, c.height + 2.0) {
                if t <= wall + 0.01 && best.as_ref().is_none_or(|(_, f)| t < *f) {
                    best = Some((Target::Talk(b), t));
                }
            }
            continue;
        }
        if !device_like(&level.actors[c.actor]) {
            continue;
        }
        if let Some(t) = ray_cylinder(eye, end, c.center, c.radius + 2.0, c.height + 2.0) {
            if t <= wall + 0.01 && best.as_ref().is_none_or(|(_, f)| t < *f) {
                best = Some((Target::Device(b), t));
            }
        }
    }
    for (idx, (info, rt)) in level.movers.iter().zip(&mv.rt).enumerate() {
        if let Some(h) = movers::trace_mover(info, rt, eye, end, small) {
            if h.fraction <= wall + 0.01 && best.as_ref().is_none_or(|(_, f)| h.fraction < *f) {
                best = Some((Target::Mover(idx), h.fraction));
            }
        }
    }
    for (e, it, deco, loose) in items {
        if it.actor.is_some_and(crate::scriptworld::is_hidden) {
            continue;
        }
        let center = match (deco, loose, it.actor) {
            (Some(d), _, _) => d.body.location,
            (_, Some(l), _) => l.pos,
            (_, _, Some(ai)) => {
                let l = level.actors[ai].location();
                DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64)
            }
            _ => continue,
        };
        let g = |n: &str, d: f64| match find(&it.props, n) {
            Some(Value::Float(v)) => *v as f64,
            _ => d,
        };
        // A little generosity, as small items are hard to aim at.
        let (r, h) = (
            g("CollisionRadius", 8.0) + 4.0,
            g("CollisionHeight", 8.0) + 4.0,
        );
        if let Some(t) = ray_cylinder(eye, end, center, r, h) {
            if t <= wall + 0.01 && best.as_ref().is_none_or(|(_, f)| t < *f) {
                best = Some((Target::Item(e), t));
            }
        }
    }
    // Medical and repair bots (frobbable robots).
    for (e, npc, deco) in bodies {
        if npc.state == crate::ai::NpcState::Dead
            || !matches!(
                level.actors[npc.actor].class.as_str(),
                "MedicalBot" | "RepairBot"
            )
        {
            continue;
        }
        if let Some(t) = ray_cylinder(
            eye,
            end,
            deco.body.location,
            npc.params.radius,
            npc.params.height,
        ) {
            if t <= wall + 0.01 && best.as_ref().is_none_or(|(_, f)| t < *f) {
                best = Some((Target::Bot(e), t));
            }
        }
    }
    // Live pawns with something to say.
    for (_, npc, deco) in bodies {
        if npc.state == crate::ai::NpcState::Dead || !crate::consys::has_conversations(npc.blocker)
        {
            continue;
        }
        if let Some(t) = ray_cylinder(
            eye,
            end,
            deco.body.location,
            npc.params.radius,
            npc.params.height,
        ) {
            if t <= wall + 0.01 && best.as_ref().is_none_or(|(_, f)| t < *f) {
                best = Some((Target::Talk(npc.blocker), t));
            }
        }
    }
    // Bodies (dead or unconscious pawns) lying on the floor: a low, wide cylinder around where they fell.
    for (e, npc, deco) in bodies {
        if npc.state != crate::ai::NpcState::Dead || npc.carried || crate::scriptworld::is_hidden(npc.actor) {
            continue;
        }
        let r = npc.params.radius * 2.0;
        let c = deco.body.location - DVec3::new(0.0, 0.0, npc.params.height - 10.0);
        if let Some(t) = ray_cylinder(eye, end, c, r, 14.0) {
            if t <= wall + 0.01 && best.as_ref().is_none_or(|(_, f)| t < *f) {
                best = Some((Target::Body(e), t));
            }
        }
    }
    best
}

/// What's in hand narrows the target: GMDX Wireless Strength reaches a device further off with the multitool; a
/// tool's left click only works doors (lockpick, key ring) or hackable devices (multitool); with a body in hand only
/// doors and devices are frobbed.
#[allow(clippy::too_many_arguments)]
fn narrow_target(
    mut best: Option<(Target, f64)>,
    level: &dx_level::Level,
    blockers: &crate::actors::Blockers,
    hacking: &crate::devices::Hacking,
    ray: FrobRay,
    tool: Option<&str>,
    only_tool: bool,
    with_body: bool,
) -> Option<(Target, f64)> {
    // GMDX Wireless Strength: a hackable device out of reach but in line, with the multitool.
    if best.is_none() && tool == Some("multitool") {
        if let Some((b, t)) = crate::devices::wireless_target(level, blockers, hacking, ray.eye, ray.dir) {
            best = Some((Target::Device(b), t));
        }
    }
    if only_tool {
        let multitool = tool == Some("multitool");
        best = match best {
            Some((Target::Mover(i), t)) if !multitool => Some((Target::Mover(i), t)),
            Some((Target::Device(b), t))
                if multitool && crate::devices::is_hackable(&level.actors[blockers.list[b].actor].props) =>
            {
                Some((Target::Device(b), t))
            }
            _ => None,
        };
    }
    if with_body {
        best = match best {
            Some((Target::Mover(i), t)) => Some((Target::Mover(i), t)),
            Some((Target::Device(b), t)) => Some((Target::Device(b), t)),
            _ => None,
        };
    }
    best
}

/// DeusExPlayer.DoFrob: messing with something bOwned is a Futz for pawns who see it (reactions.rs).
fn futz_owned(cx: &FrobCx, best: &Option<(Target, f64)>, items: &FrobItems) {
    // DeusExPlayer.DoFrob: messing with something bOwned is a Futz for pawns who see it (reactions.rs).
    let (level, blockers, mv, player) = (cx.level, cx.blockers, &cx.mv, cx.player);
    let owned = |p: &[unreal_pkg::props::Property]| matches!(find(p, "bOwned"), Some(Value::Bool(true)));
    let at = match best {
        Some((Target::Mover(i), _)) => owned(&level.actors[level.movers[*i].actor].props).then(|| mv.rt[*i].location),
        Some((Target::Device(b), _)) => {
            let c = blockers.list[*b];
            owned(&level.actors[c.actor].props).then_some(c.center)
        }
        // GMDX Sleight of Hand: taking someone's things goes unnoticed.
        Some((Target::Item(_), _)) if crate::perks::has("PerkSleightOfHand") => None,
        Some((Target::Item(e), _)) => items.get(*e).ok().and_then(|(_, it, _, _)| {
            owned(&it.props).then(|| it.actor.map_or(player.body.location, |a| {
                let l = level.actors[a].location();
                glam::DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64)
            }))
        }),
        _ => None,
    };
    if let Some(at) = at {
        crate::reactions::futz(at);
    }
}

/// DeusExMover.Frob: the key ring toggles a lock it has the key for, a lockpick starts picking, else the door is
/// worked (movers::frob).
fn frob_mover(cx: &mut FrobCx, idx: usize, picking: &mut ResMut<movers::Picking>) {
    let (level, only_tool) = (cx.level, cx.only_tool);
    let (mv, hud, weapons) = (&mut cx.mv, &mut cx.hud, &cx.weapons);
    println!(
        "FROB mover {} ({})",
        level.movers[idx].name, level.movers[idx].tag
    );
    // DeusExMover.Frob with the NanoKey ring in hand: a door that takes a key it has toggles its lock (and
    // every mover with the same Tag).
    // ParseRightClick is DoFrob(Self, None): only a left click works the door with what's in hand.
    let with_ring = only_tool
        && weapons
            .held
            .as_ref()
            .is_some_and(|h| h.stats.class == "NanoKeyRing");
    let key_needed = match find(&level.actors[level.movers[idx].actor].props, "KeyIDNeeded") {
        Some(Value::Name(k)) if k != "None" => k.clone(),
        _ => String::new(),
    };
    if with_ring && !key_needed.is_empty() && mv.rt[idx].lock_strength > 0.0 {
        crate::sound::play_player("DeusExSounds.Generic.KeysRattling", 1.0, 0.0);
        crate::weapons::play_keyring_use();
        hud.text = if crate::keys::has(&key_needed) {
            let lock = !mv.rt[idx].locked;
            let tag = level.movers[idx].tag.clone();
            for (j, info) in level.movers.iter().enumerate() {
                if j == idx
                    || (!tag.is_empty()
                        && !tag.eq_ignore_ascii_case("DeusExMover")
                        && info.tag.eq_ignore_ascii_case(&tag))
                {
                    mv.rt[j].locked = lock;
                }
            }
            crate::netplay::set_lock(idx, lock);
            if lock {
                "Your NanoKey Ring locked it".into()
            } else {
                "Your NanoKey Ring unlocked it".into()
            }
        } else if mv.rt[idx].locked {
            "Your NanoKey Ring doesn't have the right code".into()
        } else {
            "It's already unlocked".into()
        };
        hud.time_left = 3.0;
        return;
    }
    // A lockpick in hand on a locked, pickable door starts picking it.
    let with_pick = only_tool
        && weapons
            .held
            .as_ref()
            .map(|h| h.stats.class == "Lockpick")
            .unwrap_or(false);
    if with_pick && mv.rt[idx].locked && mv.rt[idx].key_num == 0 {
        if picking.0.is_some() {
            hud.text = "Picking the lock...".into();
        } else if let Some(job) = movers::start_picking(level, mv, idx) {
            picking.0 = Some(job);
            hud.text = "Picking the lock...".into();
        } else {
            hud.text = "It's locked".into();
        }
        hud.time_left = 2.0;
        return;
    }
    if with_pick
        && !mv.rt[idx].locked
        && mv.rt[idx].key_num == 0
        && mv.rt[idx].pickable
        && mv.rt[idx].lock_strength <= 0.0
    {
        hud.text = "It's already unlocked".into();
        hud.time_left = 2.0;
        return;
    }
    // Optional (setting NoisyObjects): a door with sounds makes a noise when used.
    if unreal_pkg::props::find(&level.actors[level.movers[idx].actor].props, "OpeningSound").is_some() {
        crate::optfixes::object_noise(mv.rt[idx].location, 128.0);
    }
    // Multiplayer: the host's door is the one that counts (a joiner's opens here at once and his follows).
    crate::netplay::frob_mover(idx);
    if let Some(msg) =
        movers::frob(level, mv, idx, cx.player.body.location, cx.player.unlock_all)
    {
        println!("FROB message: {msg}");
        hud.text = msg;
        hud.time_left = 3.0;
    }
}

/// Picking an item up: augmentation canisters keep what they hold, NanoKeys go onto the ring, weapons bring their
/// ammo (give_weapon), anything else into the inventory (or, with no room, used on the spot: FloorConsumables).
fn frob_item(cx: &mut FrobCx, e: Entity, items: &FrobItems, commands: &mut Commands) {
    let Ok((_, it, _, _)) = items.get(e) else {
        return;
    };
    let (inv, hud) = (&mut cx.inv, &mut cx.hud);
    // AugmentationCannister: what it holds (AddAugs) is kept with it for the medical bot.
    if it.class == "AugmentationCannister" {
        let aug = |k: u32| {
            it.props
                .iter()
                .find(|p| p.name == "AddAugs" && p.index == k)
                .and_then(|p| match &p.value {
                    Value::Name(n) if n != "None" => Some(n.clone()),
                    _ => None,
                })
                .unwrap_or_default()
        };
        let def = item_def(&it.class, &it.props);
        match crate::augcan::add(inv, &def, [aug(0), aug(1)]) {
            Ok(m) => {
                hud.text = m;
                hud.time_left = 3.0;
                crate::classic_hud::received(&it.class);
                commands.entity(e).despawn();
            }
            Err(m) => {
                // No room: it stays where it is.
                hud.text = m;
                hud.time_left = 3.0;
            }
        }
        return;
    }
    // NanoKey.GiveTo: straight onto the ring.
    if it.class == "NanoKey" {
        let id = match find(&it.props, "KeyID") {
            Some(Value::Name(k)) => k.clone(),
            _ => String::new(),
        };
        let desc = match find(&it.props, "Description") {
            Some(Value::Str(d)) => d.clone(),
            _ => "NanoKey".into(),
        };
        hud.text = crate::keys::give(&id, &desc);
        hud.time_left = 3.0;
        commands.entity(e).despawn();
        return;
    }
    let def = item_def(&it.class, &it.props);
    let copies = i(&it.props, "NumCopies").unwrap_or(1).max(1) as u32;
    let before = inv.0.items.len()
        + inv.0.credits as usize
        + inv.0.ammo.iter().map(|x| x.1 as usize).sum::<usize>()
        + inv.0.items.iter().map(|x| x.count as usize).sum::<usize>();
    let rounds = i(&it.props, "PickupAmmoCount").unwrap_or(0).max(0) as u32;
    let got = if def.kind == ItemKind::Weapon {
        give_weapon(&mut inv.0, &it.class, &it.props, rounds, &mut cx.assets)
    } else {
        inv.0.pick_up(&def, copies)
    };
    if let Some(msg) = got {
        let after = inv.0.items.len()
            + inv.0.credits as usize
            + inv.0.ammo.iter().map(|x| x.1 as usize).sum::<usize>()
            + inv.0.items.iter().map(|x| x.count as usize).sum::<usize>();
        println!("PICKUP {} -> {msg}", it.class);
        // GMDX HUD: Pickup Message Shows Size (setting PickupSize): "... [2x3]" when it didn't fit.
        let msg = if setting!("PickupSize") > 0.5 && msg == imsim_core::inventory::no_room(&def) {
            format!("{msg} [{}x{}]", def.inv_size.0, def.inv_size.1)
        } else {
            msg
        };
        hud.text = msg;
        hud.time_left = 3.0;
        if after != before {
            commands.entity(e).despawn();
        } else if copies == 1
            && setting!("FloorConsumables") > 0.5
            && matches!(
                it.class.as_str(),
                "MedKit" | "BioelectricCell" | "Candybar" | "Sodacan" | "SoyFood" | "Liquor40oz" | "WineBottle" | "Cigarettes"
            )
        {
            // Ours: no room for it, so use it where it lies.
            cx.weapons.floor_use = Some(it.class.clone());
            commands.entity(e).despawn();
        }
    }
}

/// DeusExCarcass.Frob: a nanokey the pawn carried goes onto the ring; a body with nothing left on it is picked up
/// (POVCorpse) if our hands are empty; else it is searched (InitialInventory) once.
fn frob_body(
    cx: &mut FrobCx,
    e: Entity,
    bodies: &mut FrobBodies,
    vis: &mut Query<&mut Visibility, With<crate::ai::Npc>>,
) {
    let Ok((_, mut npc, _)) = bodies.get_mut(e) else {
        return;
    };
    let level = cx.level;
    let (inv, hud, assets, seed) = (&mut cx.inv, &mut cx.hud, &mut cx.assets, &mut *cx.seed);
    if *seed == 0 {
        *seed = 0x1234_5678_9ABC_DEF1;
    }
    let mut rnd = || {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((*seed >> 33) as u32) % 4
    };
    let mut msgs: Vec<String> = Vec::new();
    // A nanokey the pawn carried (PickupDistributor) goes onto the ring.
    let a = &level.actors[npc.actor];
    let key_msg = crate::keys::take_from(crate::ai::actor_tag(a)).or_else(|| {
        match find(&a.props, "BindName") {
            Some(Value::Str(b)) => crate::keys::take_from(b),
            _ => None,
        }
    });
    let got_key = key_msg.is_some();
    if got_key {
        // DeusExCarcass.Frob: PickupNanoKey + AddReceivedItem.
        crate::classic_hud::received("NanoKey");
    }
    msgs.extend(key_msg);
    // DeusExCarcass.Frob: a body with nothing left on it is picked up (POVCorpse) if our hands are empty.
    let has_loot = !npc.searched
        && level.actors[npc.actor].props.iter().any(|x| {
            x.name == "InitialInventory"
                && matches!(&x.value, Value::Item(p, _) if { let c = p.rsplit('.').next().unwrap_or(""); !c.is_empty() && c != "None" && !c.starts_with("Ammo") && !npc.dropped.iter().any(|d| d == c) })
        });
    let robot = npc.hp_body.robot;
    if !has_loot && !got_key && cx.weapons.held.is_none() && !robot {
        npc.searched = true;
        npc.carried = true;
        if let Ok(mut v) = vis.get_mut(e) {
            v.set_if_neq(Visibility::Hidden);
        }
        cx.weapons.want_corpse = Some(e);
        println!("CARRY body {}", level.actors[npc.actor].name);
        return;
    }
    if !npc.searched {
        npc.searched = true;
        let a = &level.actors[npc.actor];
        for x in a.props.iter().filter(|x| x.name == "InitialInventory") {
            let Value::Item(path, count) = &x.value else {
                continue;
            };
            let class = path.rsplit('.').next().unwrap_or("").to_string();
            if class.is_empty() || class == "None" || class.starts_with("Ammo") {
                continue; // DeusExCarcass.Frob destroys loose ammo
            }
            if npc.dropped.contains(&class) {
                continue; // dropped when an arm was shot off
            }
            let d = assets.classes.defaults(&class);
            if d.is_empty() {
                continue;
            }
            let def = item_def(&class, &d);
            let before =
                inv.0.items.iter().map(|x| x.count).sum::<u32>() + inv.0.items.len() as u32;
            let ammo_before: u32 = inv.0.ammo.iter().map(|x| x.1).sum();
            if def.kind == ItemKind::Weapon {
                // Weapons carry 1-4 random rounds (grenades 1); if we own the gun already we only take
                // the ammo.
                let grenade = matches!(
                    class.as_str(),
                    "WeaponLAM"
                        | "WeaponGasGrenade"
                        | "WeaponEMPGrenade"
                        | "WeaponNanoVirusGrenade"
                );
                let rounds = if grenade { 1 } else { rnd() + 1 };
                // Augmentique: the owner's weapon skin is found (GetSkinFromCarcass / UnlockSkin).
                let item_name = match find(&d, "ItemName") {
                    Some(Value::Str(n)) => n.clone(),
                    _ => String::new(),
                };
                if let Some(m) = crate::augmentique::take_weapon_from(&a.class, &class, &item_name) {
                    crate::classic_hud::client_message(&m);
                }
                let owned = inv.0.items.iter().any(|x| x.def.class == class);
                if let Some(m) = give_weapon(&mut inv.0, &class, &d, rounds, assets) {
                    msgs.push(m);
                }
                // AddReceivedItem: the weapon, or its ammo if we had the weapon already.
                let got_weapon = !owned && inv.0.items.iter().any(|x| x.def.class == class);
                if got_weapon {
                    crate::classic_hud::received(&class);
                } else if inv.0.ammo.iter().map(|x| x.1).sum::<u32>() > ammo_before {
                    if let Some(Value::Path(a)) = find(&d, "AmmoName") {
                        crate::classic_hud::received(a.rsplit('.').next().unwrap_or(""));
                    }
                }
            } else if let Some(m) = inv.0.pick_up(&def, (*count).max(1) as u32) {
                msgs.push(m);
                if inv.0.items.iter().map(|x| x.count).sum::<u32>()
                    + inv.0.items.len() as u32
                    > before
                    || def.kind != ItemKind::Weapon
                        && matches!(def.kind, ItemKind::Credits { .. })
                {
                    crate::classic_hud::received(&class);
                }
            }
        }
    }
    // DeusExCarcass.AddReceivedItem prints "You found:" once, then each item's pickup message follows.
    hud.text = if msgs.is_empty() {
        "You don't find anything".into()
    } else {
        format!("You found:\n{}", msgs.join("\n"))
    };
    hud.time_left = 3.0;
    println!(
        "SEARCH {} -> {}",
        level.actors[npc.actor].name,
        hud.text.replace('\n', " | ")
    );
}

/// MedicalBot / RepairBot.Frob: the bot stops and raises its arm, and its screen opens.
fn frob_bot(cx: &mut FrobCx, e: Entity, bodies: &mut FrobBodies) {
    // MedicalBot / RepairBot.Frob: the bot stops and raises its arm, and its screen opens
    // (HUDMedBotAddAugsScreen when an augmentation canister is carried, else HUDMedBotHealthScreen;
    // HUDRechargeWindow for a repair bot).
    let Ok((_, mut npc, _)) = bodies.get_mut(e) else {
        return;
    };
    let medic = cx.level.actors[npc.actor].class == "MedicalBot";
    // GMDX HUD: Streamlined Repair Bot Interface (setting StreamlinedRepairBot): a bot that is still
    // recharging just says so instead of opening its screen.
    if !medic && npc.service_cooldown > 0.0 && setting!("StreamlinedRepairBot") > 0.5 {
        cx.hud.text = format!("Recharging... {} seconds remaining", npc.service_cooldown.ceil() as i32);
        cx.hud.time_left = 2.0;
        return;
    }
    npc.service_pause = 1.0e9;
    npc.service_shown = false;
    crate::inventory_screen::open_bot(e, medic);
}

/// DeusExProjectile.Frob: disarming a mine (grenades::frob_grenade); one that's taken becomes its weapon.
fn frob_grenade(cx: &mut FrobCx, e: Entity, grenades: &mut FrobGrenades, commands: &mut Commands) {
    let Ok((_, mut g)) = grenades.get_mut(e) else {
        return;
    };
    let (msg, take) = crate::grenades::frob_grenade(&mut g);
    if let Some(m) = msg {
        cx.hud.text = m;
        cx.hud.time_left = 2.0;
    }
    if take && !g.weapon.is_empty() {
        // DeusExProjectile.Frob: becomes its weapon (one grenade).
        let class = g.weapon.clone();
        let d = cx.assets.classes.defaults(&class);
        if let Some(m) = give_weapon(&mut cx.inv.0, &class, &d, 1, &mut cx.assets) {
            cx.hud.text = m;
            cx.hud.time_left = 2.0;
        }
        commands.entity(e).despawn();
    }
}

/// Frobbing a device: Augmentique outfit boxes, computers and ATMs (log in), readables (InformationDevices.Frob),
/// fixtures, then buttons / keypads / hackables (devices::frob_device).
fn frob_device(
    cx: &mut FrobCx,
    b: usize,
    hacking: &mut ResMut<crate::devices::Hacking>,
    events: &mut ResMut<crate::devices::GameEvents>,
    info: &mut ResMut<crate::info::InfoDisplay>,
) {
    let blockers = cx.blockers;
    let a = &cx.level.actors[blockers.list[b].actor];
    let tool = cx.weapons.held.as_ref().map(|h| h.stats.class.clone());
    let tool = tool
        .as_deref()
        .filter(|t| cx.only_tool && (*t == "Multitool" || *t == "Lockpick"));
    println!("FROB device {} ({})", a.name, a.class);
    // Augmentique's outfit boxes (OutfitSpawner.Frob -> SpawnerPickup, then the box goes).
    if crate::augmentique::is_spawner(&a.class) {
        if let Some(msg) = crate::augmentique::spawner_pickup(&a.props) {
            crate::classic_hud::client_message(&msg);
        }
        let tag = crate::devices::name_prop(&a.props, "Tag");
        crate::scriptworld::push(crate::scriptworld::ActorReq::Destroy(crate::scriptworld::ActorSel::tag(&tag)));
        return;
    }
    // Computers and ATMs log you in (Computers.Frob / ATM.Frob).
    if crate::computers::is_terminal(&a.class) {
        crate::computers::frob(blockers.list[b].actor);
        return;
    }
    // Datacubes and other readables show their text (InformationDevices.Frob).
    if crate::classtree::is_a(&a.class, "InformationDevices") || find(&a.props, "textTag").is_some() {
        let assets = &mut cx.assets;
        let tag = crate::devices::name_prop(&a.props, "textTag");
        let str_p = |k: &str, d: &str| match find(&a.props, k) {
            Some(unreal_pkg::props::Value::Str(v)) => v.clone(),
            _ => d.to_string(),
        };
        let package = str_p("TextPackage", "DeusExText");
        let no_text = str_p("msgNoText", "There's nothing interesting to read");
        let image_label = str_p("ImageLabel", "[Image: %s]");
        let added = str_p("AddedToDatavaultLabel", "Image %s added to DataVault");
        let image_class = match find(&a.props, "imageClass") {
            Some(unreal_pkg::props::Value::Path(p)) => p.rsplit('.').next().map(str::to_string),
            _ => None,
        };
        let image = image_class.map(|c| {
            let d = assets.classes.defaults(&c);
            let desc = match find(&d, "imageDescription") {
                Some(unreal_pkg::props::Value::Str(v)) => v.clone(),
                _ => c.clone(),
            };
            (c, desc)
        });
        let vault = matches!(find(&a.props, "bAddToVault"), Some(unreal_pkg::props::Value::Bool(true)));
        let system = assets.lib.system_dir().to_path_buf();
        match info.frob(
            &system,
            &package,
            &tag,
            b,
            blockers.list[b].center,
            image.as_ref().map(|(c, d)| (c.as_str(), d.as_str())),
            vault,
            (&image_label, &added),
        ) {
            crate::info::Frobbed::Opened(Some(v)) => crate::consys::add_vault_note(&v),
            crate::info::Frobbed::NoText => crate::classic_hud::client_message(&no_text),
            _ => {}
        }
        return;
    }
    if crate::fixtures::is_fixture(&a.class) {
        crate::fixtures::frob(b);
        crate::netplay::frob_fixture(blockers.list[b].actor);
    }
    if let Some(msg) =
        crate::devices::frob_device(b, &a.props, &a.class, tool, hacking, events)
    {
        cx.hud.text = msg;
        cx.hud.time_left = 2.0;
    }
}

/// ParseRightClick with nothing to frob: put away what's in hand (PutInHand(None)).
fn put_away(weapons: &mut crate::weapons::Weapons) {
    // ParseRightClick with nothing to frob: put away what's in hand (PutInHand(None)).
    if let Some(h) = weapons.held.as_mut() {
        if matches!(
            h.state,
            crate::weapons::WState::Idle | crate::weapons::WState::Selecting
        ) {
            weapons.pending = None;
            h.state = crate::weapons::WState::Lowering;
            h.clip = crate::weapons::Clip::named(&h.mesh, "Down", false)
                .unwrap_or(h.clip.clone());
            h.clip.t = 0.0;
        }
    }
}

/// Take a weapon (with `rounds` of its ammo). If it is already owned, only the ammo is taken, as Deus Ex does.
pub fn give_weapon(
    inv: &mut Inventory,
    class: &str,
    p: &[Property],
    rounds: u32,
    assets: &mut crate::weapons::RuntimeAssets,
) -> Option<String> {
    let ammo = match find(p, "AmmoName") {
        Some(Value::Path(a)) if !a.ends_with("AmmoNone") => {
            Some(a.rsplit('.').next().unwrap_or("").to_string())
        }
        _ => None,
    };
    let owned = inv.items.iter().any(|i| i.def.class == class);
    if let Some(a) = &ammo {
        if rounds > 0 {
            match inv.ammo.iter_mut().find(|x| &x.0 == a) {
                Some(x) => x.1 += rounds,
                None => inv.ammo.push((a.clone(), rounds)),
            }
        }
    }
    if owned {
        let a = ammo?;
        let d = assets.classes.defaults(&a);
        let s = |n: &str| match find(&d, n) {
            Some(Value::Str(v)) => v.clone(),
            _ => String::new(),
        };
        return Some(format!(
            "{} {} {}",
            s("PickupMessage"),
            s("ItemArticle"),
            s("ItemName")
        ));
    }
    inv.pick_up(&item_def(class, p), 1)
}

#[derive(Component)]
#[allow(dead_code)]
pub struct BeltSlot(pub usize);
#[derive(Component)]
pub struct BeltIcon(pub usize);
#[derive(Component)]
pub struct BeltLabel(pub usize);
#[derive(Component)]
pub struct CreditsText;

pub fn spawn_belt_ui(commands: &mut Commands) {
    // Superseded by the original object belt (classic_hud.rs); kept hidden for the inventory sync.
    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(12.0),
                bottom: Val::Px(12.0),
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(4.0),
                ..default()
            },
            Visibility::Hidden,
        ))
        .id();
    for slot in [1usize, 2, 3, 4, 5, 6, 7, 8, 9, 0] {
        let cell = commands
            .spawn((
                Node {
                    width: Val::Px(54.0),
                    height: Val::Px(58.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.05, 0.06, 0.08, 0.6)),
                BorderColor::all(Color::srgba(0.6, 0.65, 0.7, 0.5)),
                BeltSlot(slot),
            ))
            .id();
        let num = commands
            .spawn((
                Text::new(format!("{slot}")),
                TextFont {
                    font_size: bevy::text::FontSize::Px(10.0),
                    ..default()
                },
                TextColor(Color::srgb(0.8, 0.8, 0.8)),
            ))
            .id();
        let icon = commands
            .spawn((
                Node {
                    width: Val::Px(42.0),
                    height: Val::Px(36.0),
                    ..default()
                },
                ImageNode::default(),
                Visibility::Hidden,
                BeltIcon(slot),
            ))
            .id();
        let label = commands
            .spawn((
                Text::new(""),
                TextFont {
                    font_size: bevy::text::FontSize::Px(8.0),
                    ..default()
                },
                TextColor(Color::srgb(0.85, 0.85, 0.85)),
                BeltLabel(slot),
            ))
            .id();
        commands.entity(cell).add_children(&[num, icon, label]);
        commands.entity(root).add_child(cell);
    }
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: bevy::text::FontSize::Px(14.0),
            ..default()
        },
        TextColor(Color::srgb(0.9, 0.9, 0.7)),
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(14.0),
            bottom: Val::Px(76.0),
            ..default()
        },
        CreditsText,
    ));
}

/// Keep the belt in sync with the inventory; icons come from the game's own UI package.
#[allow(clippy::too_many_arguments)]
pub fn belt_ui(
    inv: Res<PlayerInventory>,
    root: Res<GameRoot>,
    mut images: ResMut<Assets<Image>>,
    mut icon_cache: Local<HashMap<String, Option<Handle<Image>>>>,
    mut lib: Local<Option<dx_level::TexLib>>,
    mut icons: Query<(&BeltIcon, &mut ImageNode, &mut Visibility)>,
    mut labels: Query<(&BeltLabel, &mut Text), Without<CreditsText>>,
    mut credits: Single<&mut Text, With<CreditsText>>,
) {
    let _prof = crate::prof::scope("pickups::belt_ui");
    if !inv.is_changed() {
        return;
    }
    let lib = lib.get_or_insert_with(|| dx_level::TexLib::new(&root.0));
    for (slot, mut img, mut vis) in &mut icons {
        let item = inv.0.belt[slot.0].and_then(|i| inv.0.items.get(i));
        let handle = item.and_then(|it| it.def.icon.clone()).and_then(|p| {
            icon_cache
                .entry(p.clone())
                .or_insert_with(|| {
                    let t = lib.load_image_masked(&p)?;
                    let mut im = Image::new(
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
                    im.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
                    Some(images.add(im))
                })
                .clone()
        });
        match handle {
            Some(h) => {
                img.image = h;
                vis.set_if_neq(Visibility::Inherited);
            }
            None => *vis = Visibility::Hidden,
        }
    }
    for (slot, mut text) in &mut labels {
        let item = inv.0.belt[slot.0].and_then(|i| inv.0.items.get(i));
        text.0 = match item {
            Some(it) if it.count > 1 => format!("{} ({})", it.def.belt_name, it.count),
            Some(it) => it.def.belt_name.clone(),
            None => String::new(),
        };
    }
    // The classic HUD has no credits readout (they show in the inventory screen); the stand-in label stays empty.
    credits.0 = String::new();
}

/// A decoration that frobbing uses (not a pawn): a hackable device, a fixture, an outfit box, a terminal, or
/// something with an Event or text to show.
pub fn device_like(a: &dx_level::actors::ActorInfo) -> bool {
    let p = &a.props;
    find(p, "GroundSpeed").is_none()
        && (crate::devices::is_hackable(p)
            || crate::fixtures::is_fixture(&a.class)
            || crate::augmentique::is_spawner(&a.class)
            || crate::computers::is_terminal(&a.class)
            || !crate::devices::name_prop(p, "Event").is_empty()
            || !crate::devices::name_prop(p, "textTag").is_empty())
}
