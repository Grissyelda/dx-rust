//! Optional fixes. Bug fixes that keep the original's intent are switched together by the OptionalFixes setting
//! (on in the Modern preset, off in Classic); bigger changes to play each have their own setting, all off by
//! default:
//!
//! * UnconsciousDeath - unconscious bodies can still be killed by blasts, fire and gas.
//! * NoisyObjects - using doors, buttons, switches, lamps and fixtures makes a noise guards can hear.
//! * WeaponPointing - a friendly or neutral person you point a drawn weapon at (within 160 units, in view)
//!   complains (bAIReactToWeapon).
//! * RadarCloak - Thermoptic camo also counts as radar transparency (cameras, turrets, laser beams and placed
//!   mines miss you), and the Cloak and Radar Transparency augs together pass laser beams.
//! * NpcLimp - people with a crippled leg walk at 85% speed, both legs 70%.
//! * BossResist - Gunther and Walton Simons shrug off tear gas and halon, and take a third of explosions.
//! * LongFlares - flares burn for six minutes instead of thirty seconds.
//!
//! Every place that changes says "Optional" in a comment.


/// The bug fixes (OptionalFixes).
pub fn fixes() -> bool {
    setting!("OptionalFixes") > 0.5
}

pub fn unconscious_death() -> bool {
    setting!("UnconsciousDeath") > 0.5
}

pub fn noisy_objects() -> bool {
    setting!("NoisyObjects") > 0.5
}

pub fn weapon_pointing() -> bool {
    setting!("WeaponPointing") > 0.5
}

pub fn radar_cloak() -> bool {
    setting!("RadarCloak") > 0.5
}

pub fn npc_limp() -> bool {
    setting!("NpcLimp") > 0.5
}

pub fn boss_resist() -> bool {
    setting!("BossResist") > 0.5
}

pub fn long_flares() -> bool {
    setting!("LongFlares") > 0.5
}

/// Noisy object use: the player using something sends a LoudNoise of this radius from it.
pub fn object_noise(at: bevy::math::DVec3, radius: f64) {
    if noisy_objects() {
        crate::smartai::push_noise(crate::stealth::Noise {
            pos: at,
            volume: 1.0,
            radius: radius as f32,
            maker: Some(crate::ai::Target::Player),
            instigator: Some(crate::ai::Target::Player),
            kind: crate::stealth::NoiseKind::Loud,
        });
    }
}

/// Bosses whose weapon is never dropped (AnnaNavarre / GuntherHermann / WaltonSimons.ShouldDropWeapon False).
pub fn never_drops_weapon(class: &str) -> bool {
    fixes() && matches!(class, "AnnaNavarre" | "GuntherHermann" | "WaltonSimons")
}

/// Meshes without the *2H sequences although the pawn may carry a two-handed weapon (JaimeReyes / WIB
/// HasTwoHandedWeapon False).
pub fn no_two_handed(class: &str) -> bool {
    fixes() && matches!(class, "JaimeReyes" | "WIB")
}

/// The weapon-pointing reaction (setting WeaponPointing; AugmentationDisplayWindow with
/// bAIReactToWeapon): a pawn not hostile to the player, within 160 units under the crosshair, that can see the
/// player holding a drawn weapon, complains (FutzTimer 8, PlayFutzSound). Not while it's busy with orders that
/// take it somewhere, fleeing or fighting.
pub fn weapon_pointing_system(
    time: bevy::prelude::Res<bevy::prelude::Time>,
    weapons: bevy::prelude::Res<crate::weapons::Weapons>,
    level: bevy::prelude::Res<crate::LevelRes>,
    blockers: bevy::prelude::Res<crate::actors::Blockers>,
    cam: bevy::prelude::Single<&bevy::prelude::Transform, (bevy::prelude::With<crate::FlyCam>, bevy::prelude::Without<crate::SkyCam>)>,
    mut npcs: bevy::prelude::Query<&mut crate::ai::Npc>,
    mut t: bevy::prelude::Local<f32>,
) {
    use bevy::math::DVec3;
    if !weapon_pointing() {
        return;
    }
    *t -= time.delta_secs();
    if *t > 0.0 {
        return;
    }
    *t = 0.1;
    let Some(h) = weapons.held.as_ref() else { return };
    if h.stats.item || crate::consys::third_person() {
        return;
    }
    let eye = dx_level::bevy_to_ue(cam.translation.to_array());
    let eye = DVec3::new(eye[0] as f64, eye[1] as f64, eye[2] as f64);
    let f = cam.forward().as_vec3();
    let dir = DVec3::new(f.x as f64, f.z as f64, f.y as f64).normalize();
    let end = eye + dir * 160.0;
    let lvl = &level.0;
    for mut n in &mut npcs {
        if n.futz_timer > 0.0 || n.state != crate::ai::NpcState::Idle || n.hates("Player") || n.animal || n.hp_body.robot {
            continue;
        }
        let Some(c) = blockers.list.get(n.blocker).copied() else { continue };
        if c.radius <= 0.0 || crate::pickups::ray_cylinder(eye, end, c.center, c.radius, c.height).is_none() {
            continue;
        }
        // In its view (160 degree field) with nothing in between.
        let my_eye = c.center + DVec3::Z * c.height * 0.8;
        let to = (eye - my_eye).truncate().normalize_or_zero();
        let face = DVec3::new(n.yaw.cos(), n.yaw.sin(), 0.0).truncate();
        if to.dot(face) < (80f64).to_radians().cos() || lvl.collision.trace(my_eye, eye, DVec3::ZERO).fraction < 0.999 {
            continue;
        }
        n.futz_timer = 8.0;
        crate::consys::bark(n.blocker, crate::consys::BarkMode::Futz);
    }
}

/// Unconscious bodies (setting UnconsciousDeath; DeusExCarcass.TakeDamage while bNotDead): a blast,
/// fire or gas that reaches a knocked-out body kills it - it is then a dead body (killed, `<BindName>_Dead`, no
/// longer `_Unconscious`). (Bodies in water already never stay unconscious here.)
pub fn unconscious_death_system(
    blasts: bevy::prelude::Res<crate::explosion::BlastsThisFrame>,
    mut flags: bevy::prelude::ResMut<crate::ai::GameFlags>,
    mut npcs: bevy::prelude::Query<(&mut crate::ai::Npc, &crate::actors::Deco)>,
) {
    if !unconscious_death() || blasts.0.is_empty() {
        return;
    }
    for &(pos, radius, kind) in &blasts.0 {
        if !matches!(kind, "Exploded" | "Flamed" | "Burned" | "PoisonGas" | "HalonGas" | "Radiation") {
            continue;
        }
        for (mut n, d) in &mut npcs {
            if n.state != crate::ai::NpcState::Dead || !n.knocked_out || n.carried || d.body.location.distance(pos) > radius {
                continue;
            }
            n.knocked_out = false;
            n.emit_carcass = !n.hp_body.robot;
            if n.important && !n.bind_name.is_empty() {
                flags.0.set_bool(&format!("{}_Dead", n.bind_name), true);
                flags.0.set_bool(&format!("{}_Unconscious", n.bind_name), false);
            }
            println!("OPTFIX unconscious body {} killed by {kind}", n.bind_name);
        }
    }
}
