//! The sky room drawn by the world camera, without a camera of its own.
//!
//! Unreal draws the sky zone from the SkyZoneInfo's point of view turned the way the player looks: a second scene
//! with rotation but no translation. dxview drew it with a second camera (SkyCam) before the world camera, and every
//! camera costs the render thread a full set of passes, bind groups and command buffers whatever it draws - about
//! 1.3 ms of the render thread's 13 ms for a few dozen sky faces.
//!
//! Here a copy of each sky face hangs off a root that follows the world camera's position and is scaled up by K
//! around it. Seen from its centre a scaled scene looks the same - only farther away - so with K large enough the sky
//! room lies beyond everything in the map (and the skyline and clouds), and the world draws over it as before. The
//! copies share the sky's meshes and materials (panning skies still pan), are on their own layer (only the world
//! camera sees them) and copy their originals' visibility (the skyline hides sky panels). The original faces stay at
//! the origin on layer 1 for the cameras that still draw the sky the old way: mirror reflections and security camera
//! feeds. DXVIEW_SKYCAM=1 brings the old sky camera back.

use bevy::camera::visibility::RenderLayers;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::pbr::Lightmap;
use bevy::prelude::*;

/// The world camera's layer for the sky copies.
pub const LAYER: usize = 14;

/// A sky face spawned by the level (layer 1, at the origin).
#[derive(Component)]
pub struct SkyBatch;

/// The root the copies hang from: follows the world camera, scaled by `k`.
#[derive(Component)]
pub struct SkyRoot {
    /// How far the farthest copied face can be (metres), for the camera's far plane.
    pub reach: f32,
}

/// A copy of a sky face, and the face it copies.
#[derive(Component)]
pub struct SkyCopy(pub Entity);

pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !env_on!("DXVIEW_SKYCAM"))
}

/// The scale and reach for a level: the nearest sky face ends up past the map, the skyline ring and the clouds.
fn scale_for(level: &dx_level::Level) -> Option<(f32, f32)> {
    let off = Vec3::from(level.sky_origin?);
    let mut near = f32::MAX;
    let mut far = 0.0f32;
    for b in level.batches.iter().filter(|b| b.sky) {
        for t in b.indices.chunks_exact(3) {
            let p = |i: u32| Vec3::from(b.positions[i as usize]) - off;
            let (a, bb, c) = (p(t[0]), p(t[1]), p(t[2]));
            let n = (bb - a).cross(c - a);
            if n.length_squared() > 1e-12 {
                // The plane's distance is never more than the face's: a safe nearest.
                near = near.min(n.normalize().dot(a).abs());
            }
            far = far.max(a.length()).max(bb.length()).max(c.length());
        }
    }
    if far <= 0.0 {
        return None;
    }
    let near = near.clamp(1.0, far);
    // The map's own size, the photo skyline (up to ~2.3 km out) and the cloud slab: the sky goes beyond all of it.
    let (lo, hi) = (Vec3::from(level.bounds_min), Vec3::from(level.bounds_max));
    let map = if lo.x <= hi.x { (hi - lo).length() } else { 1000.0 };
    let want = (map + 500.0).max(4500.0);
    let k = (want / near).max(1.0);
    Some((k, far * k * 1.1 + 100.0))
}

/// Once per map: the root and a copy of every sky face.
#[allow(clippy::type_complexity)]
pub fn init_system(
    mut commands: Commands,
    level: Res<crate::LevelRes>,
    faces: Query<
        (
            Entity,
            &Mesh3d,
            &Transform,
            Option<&MeshMaterial3d<crate::shadowlight::BspMaterial>>,
            Option<&MeshMaterial3d<StandardMaterial>>,
            Option<&Lightmap>,
            Option<&bevy::camera::primitives::Aabb>,
        ),
        With<SkyBatch>,
    >,
    mut gen: Local<u64>,
    mut pending: Local<bool>,
) {
    if !enabled() {
        return;
    }
    if crate::loader::new_map(&mut gen) {
        *pending = true;
    }
    if !*pending || faces.is_empty() {
        return;
    }
    *pending = false;
    let Some((k, reach)) = scale_for(&level.0) else { return };
    let root = commands
        .spawn((Transform::from_scale(Vec3::splat(k)), Visibility::default(), SkyRoot { reach }))
        .id();
    for (e, mesh, tf, bsp, std, lm, aabb) in &faces {
        let mut c = commands.spawn((
            mesh.clone(),
            *tf,
            Visibility::Inherited,
            RenderLayers::layer(LAYER),
            NotShadowCaster,
            NotShadowReceiver,
            SkyCopy(e),
            ChildOf(root),
        ));
        if let Some(m) = bsp {
            c.insert(m.clone());
        }
        if let Some(m) = std {
            c.insert(m.clone());
        }
        if let Some(l) = lm {
            c.insert(l.clone());
        }
        // Fixed bounds like the originals' (main.rs static_bounds): the sky never changes shape.
        if let Some(a) = aabb {
            c.insert((*a, bevy::camera::visibility::NoAutoAabb));
        }
    }
}

/// The root follows the world camera (after every system that places it: third person, terminals, mirrors), the
/// copies show when their faces do, and the old sky camera stays off - the world camera clears instead.
#[allow(clippy::type_complexity)]
pub fn follow_system(
    mut roots: Query<(&SkyRoot, &mut Transform), Without<crate::FlyCam>>,
    mut cam: Query<(&Transform, &mut Camera, &mut Projection), (With<crate::FlyCam>, Without<crate::SkyCam>, Without<SkyRoot>)>,
    mut sky_cam: Query<&mut Camera, (With<crate::SkyCam>, Without<crate::FlyCam>)>,
    mut copies: Query<(&SkyCopy, &mut Visibility)>,
    faces: Query<&Visibility, (With<SkyBatch>, Without<SkyCopy>)>,
) {
    if !enabled() {
        return;
    }
    let Some((root, mut rt)) = roots.iter_mut().next() else { return };
    let Ok((ct, mut camera, mut proj)) = cam.single_mut() else { return };
    if rt.translation != ct.translation {
        rt.translation = ct.translation;
    }
    if !matches!(camera.clear_color, ClearColorConfig::Default) {
        camera.clear_color = ClearColorConfig::Default;
    }
    if let Projection::Perspective(p) = &*proj {
        if p.far < root.reach {
            if let Projection::Perspective(p) = &mut *proj {
                p.far = root.reach;
            }
        }
    }
    for mut c in &mut sky_cam {
        if c.is_active {
            c.is_active = false;
        }
    }
    for (copy, mut v) in &mut copies {
        let want = faces.get(copy.0).map_or(Visibility::Hidden, |v| *v);
        if *v != want {
            *v = want;
        }
    }
}

/// The sky's materials, for the lightning flash (clouds.rs): the old sky camera's exposure did it before.
pub fn sky_materials(
    copies: &Query<&MeshMaterial3d<crate::shadowlight::BspMaterial>, With<SkyCopy>>,
) -> Vec<AssetId<crate::shadowlight::BspMaterial>> {
    let mut v: Vec<_> = copies.iter().map(|m| m.0.id()).collect();
    v.sort();
    v.dedup();
    v
}
