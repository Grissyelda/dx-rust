//! Enhancement: Modern Vision Augmentation (setting ModernVision, ours, off by default).
//!
//! The Vision Enhancement aug and the tech goggles normally show the original's light-amplification box (a green
//! tint over the middle of the screen, charged.rs) and, from the aug's second level, a dark red surround with heat
//! sources drawn through walls (augdisplay.rs). With this on, the whole view goes through an image intensifier
//! instead (shaders/gamma.wgsl `night_vision`): light amplified many times, bright lights flaring and blowing out,
//! green phosphor and grain, a round tube edge. Where the original shows heat (the aug's IR levels, or the goggles
//! with GMDX's Thermal Imaging perk), the people it would show glow white-hot - in plain sight in the dark as well
//! as through walls (the augdisplay heat camera still draws them over everything).

use crate::gpuanim::AnimMaterial;
use bevy::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

pub fn enabled() -> bool {
    crate::settings::get("ModernVision") > 0.5
}

/// Is the modern light amplification showing now (then the original's box, tint and frame stay hidden)?
pub fn active() -> bool {
    STATE.lock().map(|s| s.on).unwrap_or(false)
}

#[derive(Default)]
struct VisionState {
    on: bool,
    /// Pawn mesh parts the heat view shows (augdisplay::vision_system fills it).
    heat: HashSet<Entity>,
}

static STATE: std::sync::LazyLock<Mutex<VisionState>> = std::sync::LazyLock::new(|| Mutex::new(VisionState::default()));

/// augdisplay::vision_system: the parts that glow this frame (empty when the heat view is off).
pub fn set_heat(parts: &HashSet<Entity>) {
    if let Ok(mut s) = STATE.lock() {
        s.heat.clone_from(parts);
    }
}

/// Parts wearing a white-hot material, and what they wore before.
#[derive(Component)]
pub struct HotStd(Handle<StandardMaterial>);
#[derive(Component)]
pub struct HotAnim(Handle<AnimMaterial>);

#[derive(Default)]
pub struct HotCache {
    std: HashMap<AssetId<StandardMaterial>, Handle<StandardMaterial>>,
    anim: HashMap<AssetId<AnimMaterial>, Handle<AnimMaterial>>,
}

const HOT: f32 = 5.0;

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn vision_system(
    mut commands: Commands,
    time: Res<Time>,
    ph: Res<crate::ai::PlayerHealth>,
    active_c: Res<crate::charged::ActiveCharged>,
    mut post: Query<&mut crate::dynlight::Brightness>,
    std_parts: Query<(Entity, &MeshMaterial3d<StandardMaterial>, Option<&HotStd>)>,
    anim_parts: Query<(Entity, &MeshMaterial3d<AnimMaterial>, Option<&HotAnim>)>,
    hot_std: Query<(Entity, &HotStd)>,
    hot_anim: Query<(Entity, &HotAnim)>,
    mut std_mats: ResMut<Assets<StandardMaterial>>,
    mut anim_mats: ResMut<Assets<AnimMaterial>>,
    mut cache: Local<HotCache>,
    mut map_gen: Local<u64>,
) {
    if crate::loader::new_map(&mut map_gen) {
        cache.std.clear();
        cache.anim.clear();
    }
    let lit = (active_c.has("TechGoggles") || crate::abilities::aug_active("AugVision"))
        && !ph.dead
        && !crate::menus::open()
        && !crate::consys::third_person();
    let on = enabled() && lit;
    let heat: HashSet<Entity> = {
        let Ok(mut s) = STATE.lock() else { return };
        s.on = on;
        if on { s.heat.clone() } else { HashSet::new() }
    };
    // Nothing glowing and nothing left to give back: don't walk every mesh part.
    if heat.is_empty() && hot_std.is_empty() && hot_anim.is_empty() {
        for mut b in &mut post {
            if b.nv != 0.0 || on {
                b.nv = if on { 1.0 } else { 0.0 };
                b.nv_t = time.elapsed_secs() % 100.0;
                b.nv_ir = 0.0;
                b.nv_box = 0.0;
            }
        }
        return;
    }
    let t = time.elapsed_secs();
    for mut b in &mut post {
        let nv = if on { 1.0 } else { 0.0 };
        if b.nv != nv || on {
            b.nv = nv;
            b.nv_t = t % 100.0;
            b.nv_ir = if heat.is_empty() { 0.0 } else { 1.0 };
            b.nv_box = 0.0;
        }
    }
    // Warm bodies white-hot.
    for (e, m, hot) in &std_parts {
        if heat.contains(&e) && hot.is_none() {
            let id = m.0.id();
            let h = cache
                .std
                .entry(id)
                .or_insert_with(|| {
                    let mut c = std_mats.get(id).cloned().unwrap_or_default();
                    c.base_color = Color::LinearRgba(LinearRgba::new(HOT, HOT * 0.95, HOT * 0.9, 1.0));
                    std_mats.add(c)
                })
                .clone();
            commands.entity(e).insert((HotStd(m.0.clone()), MeshMaterial3d(h)));
        }
    }
    for (e, m, hot) in &anim_parts {
        if heat.contains(&e) && hot.is_none() {
            let id = m.0.id();
            if !cache.anim.contains_key(&id) {
                let Some(mut c) = anim_mats.get(id).cloned() else { continue };
                c.base.base_color = Color::LinearRgba(LinearRgba::new(HOT, HOT * 0.95, HOT * 0.9, 1.0));
                cache.anim.insert(id, anim_mats.add(c));
            }
            let h = cache.anim[&id].clone();
            commands.entity(e).insert((HotAnim(m.0.clone()), MeshMaterial3d(h)));
        }
    }
    for (e, orig) in &hot_std {
        if !heat.contains(&e) {
            commands.entity(e).insert(MeshMaterial3d(orig.0.clone())).remove::<HotStd>();
        }
    }
    for (e, orig) in &hot_anim {
        if !heat.contains(&e) {
            commands.entity(e).insert(MeshMaterial3d(orig.0.clone())).remove::<HotAnim>();
        }
    }
}
