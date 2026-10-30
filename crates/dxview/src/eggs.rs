//! The credits easter eggs' effects (CreditsWindow.ProcessEasterEgg, cheats only = Debug Mode):
//!
//! * THEREISNOSPOON = DeusExPlayer.Matrix: Sprite = Extras.Matrix_A00 and RMODE 6, so the world draws with the
//!   falling green code. Here every world and mesh material shows the Matrix_A00 animation (its AnimNext chain, 15
//!   frames a second) over its own lighting until the phrase is typed again (RMODE 5).
//! * IAMWARREN = DeusExPlayer.IAmWarren / UpdateWarrenEMPField: every 0.15 s one class in turn within 600 units -
//!   robots (EMP x2 their EMP points, or a hostile one half the time NanoVirus 100), alarm units, turrets, laser
//!   and beam triggers, cameras - takes 100 EMP if it was on screen in the last 2 s and isn't already confused, with
//!   SpawnEMPSparks (a 3 s spark generator, SparkFX1, gravity, Spark2). "Warren's EMP Field activated /
//!   deactivated".

use crate::gpuanim::AnimMaterial;
use bevy::prelude::*;
use glam::DVec3;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

static MATRIX: AtomicBool = AtomicBool::new(false);
static WARREN: AtomicBool = AtomicBool::new(false);

/// THEREISNOSPOON: the Matrix view on / off.
pub fn toggle_matrix() {
    MATRIX.fetch_xor(true, Ordering::Relaxed);
}

/// IAMWARREN: Warren's EMP field on / off.
pub fn toggle_warren() {
    let on = !WARREN.fetch_xor(true, Ordering::Relaxed);
    crate::classic_hud::client_message(if on { "Warren's EMP Field activated" } else { "Warren's EMP Field deactivated" });
}

pub fn warren_on() -> bool {
    WARREN.load(Ordering::Relaxed)
}

#[derive(Default)]
pub struct MatrixState {
    frames: Vec<Handle<Image>>,
    loaded: bool,
    frame: usize,
    t: f32,
    rate: f32,
    /// Materials changed, with their own texture to put back.
    std_orig: HashMap<AssetId<StandardMaterial>, Option<Handle<Image>>>,
    anim_orig: HashMap<AssetId<AnimMaterial>, Option<Handle<Image>>>,
    bsp_orig: HashMap<AssetId<crate::shadowlight::BspMaterial>, Option<Handle<Image>>>,
    shown: Option<usize>,
}

/// THEREISNOSPOON: swap every material's texture for the current Matrix frame (and back when it goes off).
pub fn matrix_system(
    time: Res<Time>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut std_mats: ResMut<Assets<StandardMaterial>>,
    mut anim_mats: ResMut<Assets<AnimMaterial>>,
    mut bsp_mats: ResMut<Assets<crate::shadowlight::BspMaterial>>,
    mut st: Local<MatrixState>,
) {
    let on = MATRIX.load(Ordering::Relaxed);
    if !on {
        if st.shown.is_some() {
            for (id, orig) in st.std_orig.drain() {
                if let Some(mut m) = std_mats.get_mut(id) {
                    m.base_color_texture = orig;
                }
            }
            for (id, orig) in st.anim_orig.drain() {
                if let Some(mut m) = anim_mats.get_mut(id) {
                    m.base.base_color_texture = orig;
                }
            }
            for (id, orig) in st.bsp_orig.drain() {
                if let Some(mut m) = bsp_mats.get_mut(id) {
                    m.base.base_color_texture = orig;
                }
            }
            st.shown = None;
        }
        return;
    }
    if !st.loaded {
        st.loaded = true;
        let (chain, rate) = match assets.lib.anim_chain("Extras.Matrix_A00") {
            Some((c, lo, hi)) => (c, hi.max(lo).max(1.0)),
            None => (vec!["Extras.Matrix_A00".to_string()], 15.0),
        };
        st.rate = rate;
        st.frames = chain.iter().filter_map(|p| crate::weapons::image_for(&mut assets, &mut images, p)).collect();
    }
    if st.frames.is_empty() {
        return;
    }
    st.t += time.delta_secs();
    let step = 1.0 / st.rate;
    while st.t >= step {
        st.t -= step;
        st.frame = (st.frame + 1) % st.frames.len();
    }
    let cur = st.frames[st.frame].clone();
    let changed = st.shown != Some(st.frame);
    st.shown = Some(st.frame);
    let frame_ids: Vec<AssetId<Image>> = st.frames.iter().map(|h| h.id()).collect();
    // New materials (a new map, a spawned pawn) are taken in as they appear; known ones only when the frame moves.
    let std_ids: Vec<AssetId<StandardMaterial>> = std_mats.ids().collect();
    for id in std_ids {
        let known = st.std_orig.contains_key(&id);
        if known && !changed {
            continue;
        }
        let Some(mut m) = std_mats.get_mut(id) else { continue };
        // Untextured materials (flat colours, the sky's backdrop fill) and additive effects keep their look.
        let Some(tex) = m.base_color_texture.clone() else { continue };
        if !known {
            if matches!(m.alpha_mode, AlphaMode::Add) || frame_ids.contains(&tex.id()) {
                continue;
            }
            st.std_orig.insert(id, Some(tex));
        }
        m.base_color_texture = Some(cur.clone());
    }
    let anim_ids: Vec<AssetId<AnimMaterial>> = anim_mats.ids().collect();
    for id in anim_ids {
        let known = st.anim_orig.contains_key(&id);
        if known && !changed {
            continue;
        }
        let Some(mut m) = anim_mats.get_mut(id) else { continue };
        let Some(tex) = m.base.base_color_texture.clone() else { continue };
        if !known {
            if frame_ids.contains(&tex.id()) {
                continue;
            }
            st.anim_orig.insert(id, Some(tex));
        }
        m.base.base_color_texture = Some(cur.clone());
    }
    // The world's BSP (shadowlight::BspMaterial).
    let bsp_ids: Vec<AssetId<crate::shadowlight::BspMaterial>> = bsp_mats.ids().collect();
    for id in bsp_ids {
        let known = st.bsp_orig.contains_key(&id);
        if known && !changed {
            continue;
        }
        let Some(mut m) = bsp_mats.get_mut(id) else { continue };
        let Some(tex) = m.base.base_color_texture.clone() else { continue };
        if !known {
            if matches!(m.base.alpha_mode, AlphaMode::Add) || frame_ids.contains(&tex.id()) {
                continue;
            }
            st.bsp_orig.insert(id, Some(tex));
        }
        m.base.base_color_texture = Some(cur.clone());
    }
}

#[derive(Default)]
pub struct WarrenState {
    timer: f32,
    slot: u8,
}

/// SpawnEMPSparks: a spark generator on the actor for 3 s.
fn sparks(pos: DVec3, follow: Option<usize>, centre: DVec3) {
    crate::particles::add_generator(crate::particles::GenSpec {
        pos,
        dir: DVec3::Z,
        tex: "Effects.Fire.SparkFX1".into(),
        draw_scale: 0.1,
        check: 0.1,
        frequency: 1.0,
        rise: 10.0,
        eject: 100.0,
        life: 4.0,
        random: true,
        follow: follow.map(|b| (b, pos - centre)),
        gravity: true,
        lifespan: Some(3.0),
        spawn_sound: Some("DeusExSounds.Generic.Spark2".into()),
    });
}

/// UpdateWarrenEMPField.
#[allow(clippy::too_many_arguments)]
pub fn warren_system(
    time: Res<Time>,
    player: Res<crate::Player>,
    blockers: Res<crate::actors::Blockers>,
    npcs: Query<&crate::ai::Npc>,
    mut hits: ResMut<crate::damage::PawnHits>,
    mut map_gen: Local<u64>,
    mut st: Local<WarrenState>,
) {
    if crate::loader::new_map(&mut map_gen) {
        *st = WarrenState::default();
    }
    if !warren_on() {
        return;
    }
    st.timer -= time.delta_secs();
    if st.timer > 0.0 {
        return;
    }
    st.timer = 0.15;
    let me = player.body.location;
    const RADIUS: f64 = 600.0;
    match st.slot {
        0 => {
            for n in &npcs {
                if !n.hp_body.robot || n.blocker == usize::MAX || n.state == crate::ai::NpcState::Dead {
                    continue;
                }
                let Some(c) = blockers.list.get(n.blocker) else { continue };
                // LastRendered() < 2.0: on screen in the last two seconds.
                if c.center.distance(me) > RADIUS || time.elapsed_secs() - n.last_rendered >= 2.0 || n.crazed > 0.0 || n.emp_hp <= 0.0 {
                    continue;
                }
                let nano = n.hates("Player") && crate::sound::frand() < 0.5;
                hits.0.push(crate::damage::PawnHit {
                    blocker: n.blocker,
                    damage: if nano { 100.0 } else { n.emp_hp * 2.0 },
                    damage_type: if nano { "NanoVirus" } else { "EMP" },
                    hit_loc: c.center,
                    attacker: None,
                    attacker_pos: me,
                });
                let dir = (me - c.center).normalize_or_zero();
                sparks(c.center + dir * c.radius, Some(n.blocker), c.center);
            }
        }
        // Alarm units, turrets, laser and beam triggers, cameras: security.rs applies it to the ones in range that
        // aren't confused yet (and sparks them).
        k => crate::security::warren(me, RADIUS, k),
    }
    st.slot = (st.slot + 1) % 6;
}

/// Sparks for a security device hit by the field (security.rs).
pub fn device_sparks(pos: DVec3) {
    sparks(pos, None, pos);
}

pub fn reset_level() {}
