//! Blob shadows (Shadow decal): every pawn and the player cast FlatFXTex40 on the ground below them, traced 480
//! units down; it is (CollisionRadius / 15) - ((distance - CollisionHeight) / 130) times the texture's size (x the
//! pawn's ShadowScale), so it shrinks as the pawn rises off the ground and vanishes past about 2 metres.
//! Birds cast one too (bHasShadow, ShadowScale 0.5): it fades as they take off.
//!
//! All the blobs are one mesh, rewritten each frame (a fixed number of quads, unused ones collapsed): drawn one
//! per pawn they were fifty-odd separate transparent draw calls per camera.

use crate::actors::Deco;
use crate::ai::{Npc, NpcState};
use crate::weapons::{image_for, RuntimeAssets};
use crate::{LevelRes, Player};
use bevy::prelude::*;
use dx_level::ue_to_bevy;
use glam::DVec3;

#[derive(Component)]
pub struct ShadowOf(pub Option<Entity>);

/// The entity drawing the batch mesh.
#[derive(Component)]
pub struct ShadowBatch;

#[derive(Resource)]
pub struct ShadowAssets {
    /// The one mesh holding every blob.
    batch: Handle<Mesh>,
    /// Texture size in Unreal units at DrawScale 1.
    size: f32,
}

/// Quads in the batch mesh (more shadows than this are left out).
const CAP: usize = 192;

fn batch_mesh() -> Mesh {
    use bevy::mesh::{Indices, PrimitiveTopology};
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::default());
    let mut uvs = Vec::with_capacity(CAP * 4);
    let mut idx: Vec<u32> = Vec::with_capacity(CAP * 6);
    for q in 0..CAP as u32 {
        uvs.extend_from_slice(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
        let b = q * 4;
        idx.extend_from_slice(&[b, b + 2, b + 1, b, b + 3, b + 2]);
    }
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; CAP * 4]);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; CAP * 4]);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    m.insert_indices(Indices::U32(idx));
    m
}

#[allow(clippy::too_many_arguments)]
pub fn shadow_system(
    mut commands: Commands,
    level: Res<LevelRes>,
    player: Res<Player>,
    mut assets: ResMut<RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    sa: Option<Res<ShadowAssets>>,
    pawns: Query<(Entity, &Npc, &Deco)>,
    mut shadows: Query<(Entity, &ShadowOf, &mut Transform, &mut Visibility)>,
    mirrors: Option<Res<crate::mirror::Mirrors>>,
) {
    let _prof = crate::prof::scope("shadows::shadow_system");
    let Some(sa) = sa else {
        let img = image_for(&mut assets, &mut images, "DeusExItems.Skins.FlatFXTex40");
        let size = img
            .as_ref()
            .and_then(|h| images.get(h))
            .map_or(64.0, |i| i.width() as f32);
        let mat = materials.add(StandardMaterial {
            base_color_texture: img,
            // Modulated like the project's other STY_Modulated surfaces (exposure-compensated grey = no change).
            base_color: Color::LinearRgba(LinearRgba::rgb(4.59, 4.59, 4.59)),
            alpha_mode: AlphaMode::Multiply,
            unlit: true,
            double_sided: true,
            cull_mode: None,
            depth_bias: 50.0,
            ..default()
        });
        let batch = meshes.add(batch_mesh());
        // Bounds that hold any map (the mesh is rewritten every frame; Bevy would re-bound it each time).
        let aabb = bevy::camera::primitives::Aabb::from_min_max(Vec3::splat(-100_000.0), Vec3::splat(100_000.0));
        commands.spawn((Mesh3d(batch.clone()), MeshMaterial3d(mat), Transform::default(), Visibility::Visible, aabb, bevy::camera::visibility::NoAutoAabb, bevy::light::NotShadowCaster, ShadowBatch));
        commands.insert_resource(ShadowAssets { batch, size });
        return;
    };
    let lvl = &level.0;
    // One shadow per pawn, plus the player's (None).
    let have: Vec<Option<Entity>> = shadows.iter().map(|(_, s, _, _)| s.0).collect();
    for (e, _, _) in &pawns {
        if !have.contains(&Some(e)) {
            commands.spawn((
                Transform::default(),
                Visibility::Hidden,
                ShadowOf(Some(e)),
            ));
        }
    }
    // Birds (Pigeon / Seagull: bHasShadow, ShadowScale 0.5).
    let birds = crate::critters::shadow_casters();
    for (e, _, _, _) in &birds {
        if !have.contains(&Some(*e)) {
            commands.spawn((
                Transform::default(),
                Visibility::Hidden,
                ShadowOf(Some(*e)),
            ));
        }
    }
    if !have.contains(&None) {
        commands.spawn((
            Transform::default(),
            Visibility::Hidden,
            ShadowOf(None),
        ));
    }
    for (se, of, mut tf, mut vis) in &mut shadows {
        let (center, radius, height, scale) = match of.0 {
            // No body to shadow while a cut scene has the camera (the player isn't drawn: the intro, the endings).
            None if crate::cinematic::active() || crate::consys::third_person() => {
                *vis = Visibility::Hidden;
                continue;
            }
            None => (
                player.body.location,
                player.params.radius,
                player.params.height,
                1.0,
            ),
            Some(pe) => match pawns.get(pe) {
                Ok((_, n, d)) if n.state != NpcState::Dead && !n.carried => {
                    (d.body.location, n.params.radius, n.params.height, n.shadow_scale)
                }
                Ok(_) => {
                    *vis = Visibility::Hidden;
                    continue;
                }
                Err(_) => match birds.iter().find(|b| b.0 == pe) {
                    Some(&(_, c, r, h)) => (c, r, h, 0.5),
                    None => {
                        commands.entity(se).despawn();
                        continue;
                    }
                },
            },
        };
        let end = center - DVec3::new(0.0, 0.0, 480.0);
        let h = lvl.collision.trace(center, end, DVec3::ZERO);
        if h.fraction >= 1.0 {
            *vis = Visibility::Hidden;
            continue;
        }
        let hit = center + (end - center) * h.fraction;
        let dist = (hit - center).length();
        let ds = ((radius / 15.0) - ((dist - height) / 130.0)) as f32 * scale;
        if ds <= 0.0 {
            *vis = Visibility::Hidden;
            continue;
        }
        // Not on a mirrored floor: the reflection shows the pawn itself, and a dark blob over it reads as a smudge.
        if mirrors.as_ref().is_some_and(|m| m.on_mirror(Vec3::from(ue_to_bevy(hit.as_vec3().to_array())))) {
            *vis = Visibility::Hidden;
            continue;
        }
        let at = hit + h.normal * 0.5;
        let n = Vec3::from(ue_to_bevy([h.normal.x as f32, h.normal.y as f32, h.normal.z as f32])).normalize_or(Vec3::Y);
        let side = sa.size * ds * dx_level::METERS_PER_UNIT;
        *tf = Transform::from_translation(Vec3::from(ue_to_bevy(at.as_vec3().to_array())))
            .with_rotation(Quat::from_rotation_arc(Vec3::Y, n))
            .with_scale(Vec3::new(side, 1.0, side));
        if *vis != Visibility::Inherited {
            *vis = Visibility::Inherited;
        }
    }
    // Every shown blob into the batch mesh; the rest of its quads collapse to a point.
    let Some(mut mesh) = meshes.get_mut(&sa.batch) else { return };
    let mut pos: Vec<[f32; 3]> = Vec::with_capacity(CAP * 4);
    let mut nrm: Vec<[f32; 3]> = Vec::with_capacity(CAP * 4);
    for (_, _, tf, vis) in &shadows {
        if *vis == Visibility::Hidden || pos.len() >= CAP * 4 {
            continue;
        }
        let m = tf.to_matrix();
        for c in [Vec3::new(-0.5, 0.0, -0.5), Vec3::new(0.5, 0.0, -0.5), Vec3::new(0.5, 0.0, 0.5), Vec3::new(-0.5, 0.0, 0.5)] {
            pos.push(m.transform_point3(c).to_array());
        }
        let n = (tf.rotation * Vec3::Y).to_array();
        nrm.extend_from_slice(&[n; 4]);
    }
    pos.resize(CAP * 4, [0.0; 3]);
    nrm.resize(CAP * 4, [0.0, 1.0, 0.0]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
}
