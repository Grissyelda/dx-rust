//! Work handed to the GPU that the CPU used to do every frame.
//!
//! Frustum culling of the level's own surfaces: Bevy tests every mesh against every camera's frustum on the main
//! thread each frame, and every surface that comes into or goes out of view is re-filed into the render thread's
//! draw lists (bins) - turning the camera re-files hundreds of them. The level's surfaces never move, so they opt
//! out of CPU culling (NoCpuCulling): they stay filed for good and the GPU culls them itself in its pre-pass
//! (Bevy's GPU preprocessing does a frustum test per instance before drawing). Surfaces showing a procedural fire
//! texture keep CPU culling: the fire texture system only animates what's on screen (it reads their visibility).
//! DXVIEW_CPU_CULL=1 keeps the old behaviour.
//!
//! A catch in Bevy 0.19: a GPU-culled mesh is filed into a view's list only in the frame it becomes GPU-culled. A
//! camera switched off drops its lists, and when it comes back (a mirror or water plane taking a reflection slot,
//! a security feed, a camera spawned later) it never gets the level's surfaces again - reflections showed the trees
//! and people but not the walls between them. So whenever a 3D camera comes on, the surfaces go back to CPU culling
//! for one frame (every view finds them that way, the new one too) and are GPU-culled again the frame after, which
//! files them into every live view afresh (`refile_system`).

use bevy::camera::visibility::NoCpuCulling;
use bevy::prelude::*;
use std::collections::HashSet;

/// The level's surfaces are culled on the GPU (not with DXVIEW_CPU_CULL, the Switch's path).
pub(crate) fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !env_on!("DXVIEW_CPU_CULL"))
}

/// Once per map: the level's BSP surfaces are culled on the GPU.
pub fn gpu_cull_system(
    mut commands: Commands,
    assets: Option<Res<crate::weapons::RuntimeAssets>>,
    mats: Res<Assets<crate::shadowlight::BspMaterial>>,
    surfaces: Query<(Entity, &MeshMaterial3d<crate::shadowlight::BspMaterial>), (Without<NoCpuCulling>, Without<crate::skyfollow::SkyCopy>)>,
    mut gen: Local<u64>,
    mut pending: Local<u32>,
) {
    if !enabled() {
        return;
    }
    if crate::loader::new_map(&mut gen) {
        // A few frames after the map is built (its surfaces, materials and fire textures all exist by then).
        *pending = 3;
    }
    if *pending == 0 {
        return;
    }
    *pending -= 1;
    if *pending > 0 {
        return;
    }
    let Some(assets) = assets else { return };
    // (Fire textures and the Texture.AnimNext loops step only while something on screen shows them, which they
    // learn from ViewVisibility - set by CPU culling alone.)
    let mut fire: HashSet<AssetId<Image>> = assets.fire.iter().map(|(_, h)| h.id()).collect();
    fire.extend(crate::anim_tex::images());
    let mut n = 0;
    for (e, m) in &surfaces {
        let shows_fire = mats.get(&m.0).is_some_and(|m| {
            [&m.base.base_color_texture, &m.base.emissive_texture].into_iter().flatten().any(|h| fire.contains(&h.id()))
        });
        if !shows_fire {
            commands.entity(e).insert((NoCpuCulling, GpuCulled));
            n += 1;
        }
    }
    if crate::prof::on() {
        println!("gpudrive: {n} level surfaces culled on the GPU");
    }
}

/// A level surface this module put on GPU culling.
#[derive(Component)]
pub struct GpuCulled;

/// See the module notes: re-files the GPU-culled surfaces into every view when a 3D camera comes on. Runs before
/// the visibility check, so the frame on CPU culling still finds them in every view.
pub fn refile_system(
    mut commands: Commands,
    cams: Query<(Entity, &Camera), With<Camera3d>>,
    surfaces: Query<(Entity, Has<NoCpuCulling>), With<GpuCulled>>,
    mut was_on: Local<std::collections::HashMap<Entity, bool>>,
    mut phase: Local<u8>,
) {
    if *phase == 1 {
        // Second frame: back on the GPU, filed into every live view.
        for (e, has) in &surfaces {
            if !has {
                commands.entity(e).insert(NoCpuCulling);
            }
        }
        *phase = 0;
    }
    let mut came_on = false;
    was_on.retain(|e, _| cams.contains(*e));
    for (e, cam) in &cams {
        let before = was_on.insert(e, cam.is_active);
        if cam.is_active && before != Some(true) {
            came_on = true;
        }
    }
    if came_on && *phase == 0 && !surfaces.is_empty() {
        for (e, has) in &surfaces {
            if has {
                commands.entity(e).remove::<NoCpuCulling>();
            }
        }
        *phase = 1;
        if crate::cinematic::active() || crate::consys::third_person() {
            crate::prof::hitch_line("a camera came on during a scene: level surfaces re-filed");
        }
        if env_on!("DXVIEW_REFILELOG") {
            println!("gpudrive: a camera came on, level surfaces re-filed");
        }
    }
}
