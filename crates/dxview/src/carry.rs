//! Picking up and throwing decorations with empty hands (DeusExPlayer.GrabDecoration / PutCarriedDecorationInHand /
//! DropDecoration): right-click a pushable object of Mass <= 50 to lift it; it floats translucent in front of you;
//! left- or right-click throws it (500 units/s along the view plus a 220 lift, divided by Mass/20).

use crate::actors::{Blockers, Deco, World};
use crate::movers::{HudMessage, Movers};
use crate::weapons::Weapons;
use crate::{FlyCam, LevelRes, Player, SkyCam};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};
use dx_level::{bevy_to_ue, ue_to_bevy};
use glam::DVec3;
use imsim_core::body::BodyPhysics;
use imsim_core::movement::CollisionWorld;

/// The decoration being carried, if any.
#[derive(Resource, Default)]
pub struct Carrying {
    pub held: Option<Held>,
    /// The use/fire input was taken by picking up or throwing this frame.
    pub consumed: bool,
}

pub struct Held {
    pub entity: Entity,
    pub blocker: usize,
    pub mass: f64,
    pub radius: f64,
    pub height: f64,
    /// Original materials of the mesh parts, restored when let go.
    materials: Vec<(Entity, Handle<StandardMaterial>)>,
}

/// CanBeLifted: 50, times (level + 2) with Microfibral Muscle on.
fn max_lift() -> f64 {
    50.0 * muscle_mult()
}

/// The Muscle aug's push/lift multiplier (level + 2 while active, else 1).
pub fn muscle_mult() -> f64 {
    crate::abilities::aug_level("AugMuscle").map_or(1.0, |l| l as f64 + 2.0)
}

#[allow(clippy::too_many_arguments)]
/// Something (a decoration or a body) is in our hands.
static CARRYING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Enhancement (Carried Objects As Cover): a crate or anything of 15 or more held in front of you stops enemy
/// bullets (and takes their damage). Ours; the original turns a carried decoration's collision off.
pub fn cover_on() -> bool {
    crate::settings::get("CarryCover") > 0.5
}

pub fn carrying() -> bool {
    CARRYING.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn carry_system(
    _keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: Single<&CursorOptions>,
    cam: Single<&Transform, (With<FlyCam>, Without<SkyCam>)>,
    player: Res<Player>,
    level: Res<LevelRes>,
    mut blockers: ResMut<Blockers>,
    mv: Res<Movers>,
    weapons: Res<Weapons>,
    frob_req: Res<crate::pickups::FrobRequest>,
    (mut hud, mut carrying_res): (ResMut<HudMessage>, ResMut<Carrying>),
    (mut decos, items, children, mut mats): (
        Query<(&mut Deco, &mut Transform), Without<FlyCam>>,
        Query<(), With<crate::pickups::PickupItem>>,
        Query<&Children>,
        Query<&mut MeshMaterial3d<StandardMaterial>>,
    ),
    (mut materials, mut meshes, mut seed): (
        ResMut<Assets<StandardMaterial>>,
        ResMut<Assets<Mesh>>,
        Local<u64>,
    ),
) {
    let _prof = crate::prof::scope("carry::carry_system");
    let carrying = &mut *carrying_res;
    carrying.consumed = false;
    CARRYING.store(carrying.held.is_some() || weapons.corpse.is_some(), std::sync::atomic::Ordering::Relaxed);
    let locked = cursor.grab_mode == CursorGrabMode::Locked;
    let use_pressed = mouse.just_pressed(MouseButton::Right) || frob_req.0;
    let fire_pressed = locked && mouse.just_pressed(MouseButton::Left);

    let eye = bevy_to_ue(cam.translation.to_array());
    let eye = DVec3::new(eye[0] as f64, eye[1] as f64, eye[2] as f64);
    let f = cam.forward().as_vec3();
    let view = DVec3::new(f.x as f64, f.z as f64, f.y as f64).normalize();
    let flat = DVec3::new(view.x, view.y, 0.0).normalize_or_zero();
    let pl = player.body.location;

    if let Some(h) = carrying.held.as_ref() {
        let Ok((mut d, mut tf)) = decos.get_mut(h.entity) else {
            carrying.held = None;
            return;
        };
        // AugMuscle.Deactivate: something too heavy to lift without it is put down.
        let too_heavy = h.mass > max_lift();
        if use_pressed || fire_pressed || too_heavy {
            carrying.consumed = use_pressed || fire_pressed;
            // DropDecoration: thrown from just in front of the player's eyes.
            if *seed == 0 {
                *seed = 0xC0FF_EE12_3456_789B;
            }
            let mut rnd = || {
                *seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                ((*seed >> 33) as f64) / (1u64 << 31) as f64 * 2.0 - 1.0
            };
            let jitter = DVec3::new(rnd(), rnd(), rnd()).normalize_or_zero() * 40.0;
            let velscale = (h.mass / 20.0).clamp(1.0, 40.0);
            // Throw speed x the Muscle aug's value.
            let mult = crate::abilities::aug("AugMuscle").unwrap_or(1.0) as f64;
            let mult = if too_heavy { 0.0 } else { mult };
            let vel = (view * mult * 500.0 + DVec3::new(0.0, 0.0, 220.0) + jitter) / velscale;
            let mut drop = pl + flat * (h.radius + player.params.radius + 4.0);
            drop.z += player.eye_height;
            let world = World {
                level: &level.0,
                blockers: &blockers.list,
                ignore: Some(h.blocker),
                pushable: None,
                movers: Some(&mv.rt),
            };
            let clear = world.trace(eye, drop, DVec3::splat(1.0)).fraction >= 1.0
                && world
                    .trace(
                        drop,
                        drop + DVec3::Z * 0.1,
                        DVec3::new(h.radius, h.radius, 1.0),
                    )
                    .fraction
                    >= 1.0;
            if !clear {
                hud.text = "Can't drop that here".into();
                hud.time_left = 2.0;
                return;
            }
            d.body.location = drop;
            d.body.velocity = vel;
            d.body.physics = BodyPhysics::Falling;
            d.thrown_by_player = true;
            blockers.list[h.blocker].center = drop;
            blockers.list[h.blocker].radius = h.radius;
            blockers.list[h.blocker].height = h.height;
            if d.pushable && h.mass <= 100.0 {
                blockers.pushable_mass.insert(h.blocker, h.mass);
            }
            d.last_lit = DVec3::splat(1.0e9); // relight on the next physics step
            let draw = drop + d.pre;
            tf.translation = Vec3::from(ue_to_bevy(draw.as_vec3().to_array()));
            for (e, m) in &h.materials {
                if let Ok(mut mm) = mats.get_mut(*e) {
                    mm.0 = m.clone();
                }
            }
            carrying.held = None;
            return;
        }
        // Keep it in front of us at chest height (PutCarriedDecorationInHand placement).
        let at = pl
            + DVec3::new(0.0, 0.0, player.params.height / 2.0)
            + flat * (0.5 * player.params.radius + h.radius);
        d.body.location = at;
        d.body.velocity = DVec3::ZERO;
        // Not a blocker for the player while held, but pawns walk around it (ai.rs reads this centre).
        blockers.list[h.blocker].center = at;
        let yaw = (flat.y.atan2(flat.x) / std::f64::consts::TAU * 65536.0) as i32;
        d.rotation[1] = yaw;
        d.spun = true;
        let draw = at + d.pre;
        tf.translation = Vec3::from(ue_to_bevy(draw.as_vec3().to_array()));
        return;
    }

    // Grab: use with empty hands on a pushable decoration within reach.
    if !use_pressed || weapons.held.is_some() {
        return;
    }
    let world = World {
        level: &level.0,
        blockers: &blockers.list,
        ignore: None,
        pushable: None,
        movers: Some(&mv.rt),
    };
    let hit = world.trace(eye, eye + view * 112.0, DVec3::splat(1.0));
    let Some(b) = hit.actor else { return };
    let Some(&e) = blockers.deco.get(&b) else {
        return;
    };
    if items.get(e).is_ok() {
        return; // inventory items are picked up, not carried
    }
    let Ok((mut d, _)) = decos.get_mut(e) else {
        return;
    };
    let a = &level.0.actors[blockers.list[b].actor];
    if unreal_pkg::props::find(&a.props, "GroundSpeed").is_some() {
        return;
    }
    // Buttons, switches and hackable devices are used, not lifted (pickups.rs handles them).
    if !d.pushable
        && (crate::devices::is_hackable(&a.props)
            || crate::computers::is_terminal(&a.class)
            || crate::fixtures::is_fixture(&a.class)
            || !crate::devices::name_prop(&a.props, "Event").is_empty()
            || !crate::devices::name_prop(&a.props, "textTag").is_empty())
    {
        return;
    }
    carrying.consumed = true;
    if !d.pushable || d.mass > max_lift() {
        hud.text = if d.pushable {
            "It's too heavy to lift".into()
        } else {
            "You can't lift that".into()
        };
        hud.time_left = 2.0;
        return;
    }
    // Translucent and unlit while carried.
    let mut saved = Vec::new();
    if let Ok(ch) = children.get(e) {
        for c in ch.iter() {
            if let Ok(mut mm) = mats.get_mut(c) {
                saved.push((c, mm.0.clone()));
                if let Some(orig) = materials.get(&mm.0).cloned() {
                    let mut m = orig;
                    m.alpha_mode = AlphaMode::Add;
                    mm.0 = materials.add(m);
                }
            }
        }
    }
    for (h, _) in &d.parts {
        if let Some(mut m) = meshes.get_mut(h) {
            let n = m
                .attribute(Mesh::ATTRIBUTE_POSITION)
                .map(|a| a.len())
                .unwrap_or(0);
            m.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.6f32, 0.6, 0.6, 1.0]; n]);
        }
    }
    d.body.physics = BodyPhysics::None;
    if d.floating {
        d.floating = false;
        d.rotation = d.orig_rot;
    }
    let (radius, height) = (blockers.list[b].radius, blockers.list[b].height);
    blockers.list[b].radius = 0.0;
    blockers.list[b].height = 0.0;
    blockers.pushable_mass.remove(&b);
    carrying.held = Some(Held {
        entity: e,
        blocker: b,
        mass: d.mass,
        radius,
        height,
        materials: saved,
    });
    let _ = &mut d;
}

/// A thrown body becomes a carcass again: shown, falling from where it left our hands.
pub fn corpse_drop_system(
    mut drops: ResMut<crate::weapons::CorpseDrops>,
    mut q: Query<(
        &mut crate::ai::Npc,
        &mut Deco,
        &mut Visibility,
        &mut Transform,
    )>,
) {
    let _prof = crate::prof::scope("carry::corpse_drop_system");
    for (e, pos, vel) in drops.0.drain(..) {
        let Ok((mut npc, mut d, mut vis, mut tf)) = q.get_mut(e) else {
            continue;
        };
        npc.carried = false;
        d.body.location = pos;
        d.body.velocity = vel;
        d.body.physics = BodyPhysics::Falling;
        d.thrown_body = true;
        d.thrown_by_player = true;
        d.last_lit = DVec3::splat(1.0e9);
        let draw = pos + d.pre;
        tf.translation = Vec3::from(ue_to_bevy(draw.as_vec3().to_array()));
        *vis = Visibility::Inherited;
    }
}
