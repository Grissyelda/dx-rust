//! Splashes when things other than the player go into the water (the player's own are in swim.rs).
//! Each class has its own ZoneChange rule, with the water zone's EntrySound (SplashMedium) and EntryActor
//! (WaterRing, which spawns two more rings 0.2 s apart):
//! * Decoration (crates, barrels, carcasses): only falling faster than 200; size = clamp(0.0001 x Mass x
//!   (250 - 0.5 x max(-600, Vz)), 1, 3) for the sound volume (SLOT_Interact) and the ring.
//! * Inventory lying in the world (dropped/thrown items): size = 0.000025 x Mass x (250 - 0.5 x Vz); ring 2 x size.
//! * Fragment (debris, shell casings) and ThrownProjectile (grenades): speed x 0.2 on entry, size =
//!   0.0005 x (250 - 0.5 x Vz), ring 4 x size; a grenade's tumble slows to 0.2 too.
//! * Pawn.FootZoneChange for NPCs, at most every 0.25 s: feet going in = splash of clamp(0.000025 x Mass x
//!   (300 - 0.5 x max(-500, Vz)), 1, 4) (sound x2), feet coming out = ExitSound (WaterOut) and a ring.
//! * DeusExCarcass.ZoneChange swaps to the floating carcass mesh (Mesh3): not done here (no mesh swap yet).

use crate::actors::{water_at, Deco};
use crate::ai::{Npc, NpcState};
use crate::destruct::{LandSound, Loose};
use crate::grenades::Grenade;
use crate::weapons::RuntimeAssets;
use crate::LevelRes;
use bevy::prelude::*;
use glam::DVec3;
use std::collections::HashMap;

const ENTRY_SOUND: &str = "DeusExSounds.Generic.SplashMedium";
const EXIT_SOUND: &str = "DeusExSounds.Generic.WaterOut";
/// Default PlaySound radius used for splashes elsewhere (swim.rs).
const RADIUS: f32 = 1600.0;
/// Inventory mass when we don't know the item's own (DeusExPickup default).
const ITEM_MASS: f64 = 10.0;

#[derive(Resource, Default)]
pub struct Splashes {
    /// Last known "in water" state per entity (for decorations, the feet for live NPCs).
    inside: HashMap<Entity, bool>,
    /// Last FootZoneChange splash per NPC (0.25 s gap).
    npc_time: HashMap<Entity, f64>,
    /// WaterRing.Tick's extra rings: (due, place, size).
    rings: Vec<(f64, DVec3, f32)>,
}

/// Where a body crossing the surface meets it: scan up from `p` to the top of the water (at most 64 units).
fn surface_near(level: &dx_level::Level, p: DVec3) -> DVec3 {
    let mut q = p;
    for _ in 0..16 {
        let up = q + DVec3::Z * 4.0;
        if water_at(level, up).is_none() {
            return q;
        }
        q = up;
    }
    p
}

#[allow(clippy::too_many_arguments)]
pub fn splash_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    mut st: ResMut<Splashes>,
    decos: Query<(Entity, &Deco, Option<&Npc>)>,
    mut loose: Query<(Entity, &mut Loose, Option<&LandSound>)>,
    mut grenades: Query<(Entity, &mut Grenade)>,
    (mut assets, mut images, mut meshes, mut materials): (
        ResMut<RuntimeAssets>,
        ResMut<Assets<Image>>,
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
    ),
) {
    let _prof = crate::prof::scope("splash::splash_system");
    let lvl = &level.0;
    let now = time.elapsed_secs_f64();
    let mut seen: Vec<Entity> = Vec::new();
    // (place, ring scale, sound volume, exit?)
    let mut fx: Vec<(DVec3, f32, f32, bool)> = Vec::new();

    let cross = |st: &mut Splashes, e: Entity, now_in: bool| -> Option<bool> {
        let was = st.inside.insert(e, now_in);
        match was {
            // First sight: no splash for starting out where it is.
            None => None,
            Some(w) if w != now_in => Some(now_in),
            _ => None,
        }
    };

    for (e, d, npc) in &decos {
        seen.push(e);
        match npc {
            Some(n) if n.state != NpcState::Dead => {
                // A live pawn: Pawn.FootZoneChange.
                let Some(b) = n.body.as_ref() else { continue };
                let feet = b.location - DVec3::Z * (n.params.height - 1.0);
                let now_in = water_at(lvl, feet).is_some();
                if let Some(entered) = cross(&mut st, e, now_in) {
                    let last = st.npc_time.get(&e).copied().unwrap_or(-1.0);
                    if now - last <= 0.25 {
                        continue;
                    }
                    st.npc_time.insert(e, now);
                    let at = surface_near(lvl, b.location - DVec3::Z * n.params.height);
                    if entered {
                        let size = (0.000025 * n.params.mass * (300.0 - 0.5 * b.velocity.z.max(-500.0)))
                            .clamp(1.0, 4.0) as f32;
                        fx.push((at, size, 2.0 * size, false));
                    } else {
                        fx.push((at, 1.0, 1.0, true));
                    }
                }
            }
            _ => {
                // Decorations and carcasses: Decoration.ZoneChange (the zone of the collision centre).
                let (loc, vel) = match npc.and_then(|n| n.body.as_ref()) {
                    Some(b) => (b.location, b.velocity),
                    None => (d.body.location, d.body.velocity),
                };
                let now_in = water_at(lvl, loc).is_some();
                if let Some(true) = cross(&mut st, e, now_in) {
                    if vel.z < -200.0 {
                        let size = (0.0001 * d.mass * (250.0 - 0.5 * vel.z.max(-600.0))).clamp(1.0, 3.0) as f32;
                        fx.push((surface_near(lvl, loc), size, size, false));
                    }
                }
            }
        }
    }

    for (e, mut l, item) in &mut loose {
        seen.push(e);
        let now_in = water_at(lvl, l.pos).is_some();
        if let Some(true) = cross(&mut st, e, now_in) {
            let at = surface_near(lvl, l.pos);
            if item.is_some() {
                // Inventory (Pickup state).
                let size = (0.000025 * ITEM_MASS * (250.0 - 0.5 * l.vel.z)) as f32;
                fx.push((at, 2.0 * size, size, false));
            } else {
                // Fragment / casing.
                l.vel *= 0.2;
                let size = (0.0005 * (250.0 - 0.5 * l.vel.z)) as f32;
                fx.push((at, 4.0 * size, size, false));
            }
        }
    }

    for (e, mut g) in &mut grenades {
        seen.push(e);
        let now_in = water_at(lvl, g.pos).is_some();
        if let Some(true) = cross(&mut st, e, now_in) {
            g.vel *= 0.2;
            let vz = g.vel.z;
            for r in g.rot_rate.iter_mut() {
                *r *= 0.2;
            }
            let size = (0.0005 * (250.0 - 0.5 * vz)) as f32;
            fx.push((surface_near(lvl, g.pos), 4.0 * size, size, false));
        }
    }

    // Forget what's gone.
    if st.inside.len() > seen.len() {
        let keep: std::collections::HashSet<Entity> = seen.into_iter().collect();
        st.inside.retain(|e, _| keep.contains(e));
        st.npc_time.retain(|e, _| keep.contains(e));
    }

    for (at, ring, vol, exit) in fx {
        let snd = if exit { EXIT_SOUND } else { ENTRY_SOUND };
        if vol > 0.01 {
            crate::sound::play(snd, at, vol.min(4.0), RADIUS, 1.0);
        }
        if ring > 0.01 {
            crate::swim::water_ring(&mut commands, &mut assets, &mut images, &mut meshes, &mut materials, at, ring);
            // The two extra rings are plain Spawn(class'WaterRing'): default DrawScale.
            st.rings.push((now + 0.2, at, 1.0));
            st.rings.push((now + 0.4, at, 1.0));
        }
    }
    let due: Vec<(DVec3, f32)> = st
        .rings
        .iter()
        .filter(|r| r.0 <= now)
        .map(|r| (r.1, r.2))
        .collect();
    st.rings.retain(|r| r.0 > now);
    for (at, size) in due {
        crate::swim::water_ring(&mut commands, &mut assets, &mut images, &mut meshes, &mut materials, at, size);
    }
}
