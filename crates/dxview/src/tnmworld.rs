//! The Nameless Mod's world actors that only dress a map: decalspawner (a decal put on the nearest surface along
//! its spawndirection when the map starts).
// uc: decalspawner.postBeginPlay

use bevy::prelude::*;
use glam::DVec3;
use unreal_pkg::props::{find, Value};

/// Each map once: every decalspawner traces 320 units against its spawndirection (then along it) and leaves its
/// decal there at its scale, for good.
pub fn decal_spawner_system(
    level: Res<crate::LevelRes>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut map: Local<u64>,
) {
    if !crate::loader::new_map(&mut map) {
        return;
    }
    let lvl = &level.0;
    let world = crate::actors::World { level: lvl, blockers: &[], ignore: None, pushable: None, movers: None };
    for a in lvl.actors.iter().filter(|a| a.class.eq_ignore_ascii_case("decalspawner")) {
        let p = &a.props;
        let Some(Value::Path(class)) = find(p, "decal") else { continue };
        let class = class.rsplit('.').next().unwrap_or(class).to_string();
        let d = assets.classes.defaults(&class);
        let Some(Value::Path(tex)) = find(&d, "Texture") else { continue };
        let scale = match find(p, "scale") {
            Some(Value::Float(v)) => *v,
            _ => match find(&d, "DrawScale") {
                Some(Value::Float(v)) => *v,
                _ => 1.0,
            },
        };
        let dir = match find(p, "spawndirection") {
            Some(Value::Rotator(r)) => crate::staring::dir_of(*r),
            _ => DVec3::X,
        };
        let l = a.location();
        let at = DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64);
        for end in [at - dir * 320.0, at + dir * 320.0] {
            let h = imsim_core::movement::CollisionWorld::trace(&world, at, end, DVec3::ZERO);
            if h.fraction < 1.0 {
                crate::impacts::mark_forever(at + (end - at) * h.fraction, h.normal, tex, scale);
                break;
            }
        }
    }
}

/// TripProj.Tick's damage: 5 'Poison' to whoever stands in a trip mine's beam (security.rs draws and traces it).
// uc: TripProj.Tick
pub fn trip_system(
    blockers: Res<crate::actors::Blockers>,
    mut ph: ResMut<crate::ai::PlayerHealth>,
    mut hits: ResMut<crate::damage::PawnHits>,
) {
    let list = crate::security::TRIP_HITS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    for who in list {
        match who {
            None => ph.damage(5.0, "Poison", DVec3::ZERO),
            Some(b) => {
                let Some(c) = blockers.list.get(b) else { continue };
                hits.0.push(crate::damage::PawnHit {
                    blocker: b,
                    damage: 5.0,
                    damage_type: "Poison",
                    hit_loc: c.center,
                    attacker: None,
                    attacker_pos: c.center,
                    world: true,
                });
            }
        }
    }
}
