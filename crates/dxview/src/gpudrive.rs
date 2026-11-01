//! Work handed to the GPU that the CPU used to do every frame.
//!
//! Frustum culling of the level's own surfaces: Bevy tests every mesh against every camera's frustum on the main
//! thread each frame, and every surface that comes into or goes out of view is re-filed into the render thread's
//! draw lists (bins) - turning the camera re-files hundreds of them. The level's surfaces never move, so they opt
//! out of CPU culling (NoCpuCulling): they stay filed for good and the GPU culls them itself in its pre-pass
//! (Bevy's GPU preprocessing does a frustum test per instance before drawing). Surfaces showing a procedural fire
//! texture keep CPU culling: the fire texture system only animates what's on screen (it reads their visibility).
//! DXVIEW_CPU_CULL=1 keeps the old behaviour.

use bevy::camera::visibility::NoCpuCulling;
use bevy::prelude::*;
use std::collections::HashSet;

fn enabled() -> bool {
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
    let fire: HashSet<AssetId<Image>> = assets.fire.iter().map(|(_, h)| h.id()).collect();
    let mut n = 0;
    for (e, m) in &surfaces {
        let shows_fire = mats.get(&m.0).is_some_and(|m| {
            [&m.base.base_color_texture, &m.base.emissive_texture].into_iter().flatten().any(|h| fire.contains(&h.id()))
        });
        if !shows_fire {
            commands.entity(e).insert(NoCpuCulling);
            n += 1;
        }
    }
    if crate::prof::on() {
        println!("gpudrive: {n} level surfaces culled on the GPU");
    }
}
