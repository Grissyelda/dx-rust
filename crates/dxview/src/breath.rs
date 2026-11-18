//! The Nameless Mod's visible breath (TNMPawn / Trestkon spawnbreathgen / updatebreathgen): in the cold ABI maps
//! (23_ABIExterior, its 2009 copy, 23_ABIApproach, and the stealth bonus map 95_Stealth7 for pawns) - or wherever a
//! map sets bVisibleBreath - a small smoke-puff generator at the mouth (half the radius ahead, 0.91 of the eye height
//! up; women, animals, wolves and the Despot by their own offsets; crouched further forward and lower) breathes out
//! over the second half of each breath cycle (breathtime, the class default), the eject speed swelling and fading
//! with it; speaking keeps it going. None under water, dying or stunned. Trestkon's own shows only outside his
//! walking state (in third-person conversations), and not while hidden or cloaked.

use bevy::prelude::*;
use glam::DVec3;
use std::collections::HashMap;
use unreal_pkg::props::{find, Value};

use crate::actors::Blockers;
use crate::ai::{Npc, NpcState};

const PAWN_MAPS: [&str; 4] = ["23_ABIEXTERIOR", "23_ABIEXTERIOR_2009", "23_ABIAPPROACH", "95_STEALTH7"];
const PLAYER_MAPS: [&str; 3] = ["23_ABIEXTERIOR", "23_ABIEXTERIOR_2009", "23_ABIAPPROACH"];

#[derive(Default)]
pub struct BreathState {
    gen: u64,
    map: String,
    /// Per pawn (blocker; usize::MAX = the player): breath timer, spewing.
    timers: HashMap<usize, (f32, bool)>,
    /// Class defaults: (breathtime, breathSize, DrawScale, BaseEyeHeight).
    defaults: HashMap<String, (f32, f32, f32, f64)>,
    seed: u64,
}

fn num(p: &[unreal_pkg::props::Property], k: &str) -> Option<f32> {
    match find(p, k) {
        Some(Value::Float(v)) => Some(*v),
        Some(Value::Int(v)) => Some(*v as f32),
        _ => None,
    }
}

/// updatebreathgen's cycle: (spewing, eject speed) after `dt`.
fn cycle(t: &mut f32, spewing: bool, bt: f32, dt: f32, speaking: bool) -> (bool, f64) {
    *t += dt;
    if *t > bt {
        *t = 0.0;
    }
    if (*t < bt * 0.5 || *t > bt * 0.9) && spewing && !speaking {
        return (false, 0.0);
    }
    let m = if *t < bt * 0.75 { (*t - bt * 0.5) / (bt * 0.25) } else { (bt - *t) / (bt * 0.15) };
    if m < 0.0 && !speaking {
        (false, 0.0)
    } else {
        (true, 12.5 * m.abs() as f64)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn breath_system(
    time: Res<Time>,
    level: Res<crate::LevelRes>,
    blockers: Res<Blockers>,
    player: Res<crate::Player>,
    ph: Res<crate::ai::PlayerHealth>,
    cams: Query<&crate::FlyCam, Without<crate::SkyCam>>,
    npcs: Query<&Npc>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut st: Local<BreathState>,
) {
    if !crate::tnmmode::active() {
        return;
    }
    let lvl = &level.0;
    let dt = time.delta_secs().min(0.1);
    if crate::loader::new_map(&mut st.gen) {
        st.timers.clear();
        st.map = lvl.name.trim_end_matches(".dx").to_uppercase();
    }
    let pawn_map = PAWN_MAPS.contains(&st.map.as_str());
    let player_map = PLAYER_MAPS.contains(&st.map.as_str());
    let speaking = crate::consys::speaking();
    let st = &mut *st;
    for n in &npcs {
        if n.net.is_some() {
            continue;
        }
        let Some(a) = lvl.actors.get(n.actor) else { continue };
        let Some(c) = blockers.list.get(n.blocker) else { continue };
        if c.radius <= 0.0 || n.state == NpcState::Dead {
            st.timers.remove(&n.blocker);
            continue;
        }
        if !crate::classtree::is_a(&a.class, "TNMPawn") {
            continue;
        }
        let own = matches!(find(&a.props, "bVisibleBreath"), Some(Value::Bool(true)));
        if !(own || pawn_map) {
            continue;
        }
        let (bt, size, scale, eye) = *st.defaults.entry(a.class.clone()).or_insert_with(|| {
            let d = assets.classes.defaults(&a.class);
            (
                num(&d, "breathtime").unwrap_or(0.0),
                num(&d, "breathSize").unwrap_or(1.0),
                num(&d, "DrawScale").unwrap_or(1.0),
                num(&d, "BaseEyeHeight").unwrap_or(c.height as f32 * 0.8) as f64,
            )
        });
        let bt = num(&a.props, "breathtime").unwrap_or(bt);
        if bt <= 0.0 {
            continue;
        }
        let size = num(&a.props, "breathSize").unwrap_or(size);
        let scale = num(&a.props, "DrawScale").unwrap_or(scale);
        let female = matches!(find(&a.props, "bIsFemale"), Some(Value::Bool(true)));
        let is = |k: &str| crate::classtree::is_a(&a.class, k);
        let (fx, fz) = if female {
            (0.15, 0.9)
        } else if is("SnowWolf") || is("Wolfy") {
            (0.9, 0.91)
        } else if is("TNMAnimal") || is("SD") {
            (1.0, 0.91)
        } else if is("Despot") {
            (0.95, 0.91)
        } else {
            (0.5, 0.91)
        };
        let mut off = DVec3::new(c.radius * fx, 0.0, eye * fz);
        if n.crouching {
            off += DVec3::new(c.radius * 0.5, 0.0, -c.height * 0.15);
        }
        let (sy, cy) = n.yaw.sin_cos();
        let fwd = DVec3::new(cy, sy, 0.0);
        let pos = c.center + DVec3::new(off.x * cy - off.y * sy, off.x * sy + off.y * cy, off.z);
        if crate::actors::water_at(lvl, pos).is_some() {
            st.timers.remove(&n.blocker);
            continue;
        }
        let seed = &mut st.seed;
        let e = st.timers.entry(n.blocker).or_insert_with(|| {
            *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((*seed >> 40) as f32 / (1u64 << 24) as f32 * bt, false)
        });
        let stunned = matches!(n.state, NpcState::Stunned(_));
        let talking = speaking == Some(Some(n.blocker));
        let (on, eject) = if stunned { (false, 0.0) } else { cycle(&mut e.0, e.1, bt, dt, talking) };
        e.1 = on;
        crate::particles::hold(
            format!("held:breath:{}", n.blocker),
            crate::particles::Held {
                pos,
                dir: fwd,
                spewing: on,
                eject,
                draw_scale: 0.1 * scale * size,
                tex: "Effects.Smoke.SmokePuff1",
                check: 0.1,
                rise: 3.0,
                life: 0.5,
            },
        );
    }
    // Trestkon: only out of his walking state (a third-person conversation), not hidden or cloaked.
    if player_map || matches!(find(&assets.classes.defaults(crate::tnmmode::player_class()), "bVisibleBreath"), Some(Value::Bool(true))) {
        let shown = crate::consys::third_person()
            && !crate::cinematic::active()
            && !crate::abilities::aug_active("AugCloak")
            && !crate::tnmtriggers::player_hidden()
            && !ph.camo;
        let d = assets.classes.defaults(crate::tnmmode::player_class());
        let bt = num(&d, "breathtime").unwrap_or(0.0);
        if shown && bt > 0.0 {
            let size = num(&d, "breathSize").unwrap_or(1.0);
            let yaw = cams.iter().next().map_or(0.0, |c| -(c.yaw as f64) - std::f64::consts::FRAC_PI_2);
            let (sy, cy) = yaw.sin_cos();
            let mut off = DVec3::new(player.params.radius * 0.5, 0.0, player.eye_height * 0.91);
            if player.crouched {
                off += DVec3::new(player.params.radius * 0.5, 0.0, -player.params.height * 0.15);
            }
            let pos = player.body.location + DVec3::new(off.x * cy, off.x * sy, off.z);
            let e = st.timers.entry(usize::MAX).or_insert((0.0, false));
            let (on, eject) = cycle(&mut e.0, e.1, bt, dt, false);
            e.1 = on;
            crate::particles::hold(
                "held:breath:player".into(),
                crate::particles::Held {
                    pos,
                    dir: DVec3::new(cy, sy, 0.0),
                    spewing: on,
                    eject,
                    draw_scale: 0.1 * size,
                    tex: "Effects.Smoke.SmokePuff1",
                    check: 0.1,
                    rise: 3.0,
                    life: 0.5,
                },
            );
        }
    }
}
