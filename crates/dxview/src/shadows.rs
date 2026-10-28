//! Blob shadows (Shadow decal): every pawn and the player cast FlatFXTex40 on the ground below them, traced 480
//! units down; it is (CollisionRadius / 15) - ((distance - CollisionHeight) / 130) times the texture's size (x the
//! pawn's ShadowScale), so it shrinks as the pawn rises off the ground and vanishes past about 2 metres.
//! Birds cast one too (bHasShadow, ShadowScale 0.5): it fades as they take off.

use crate::actors::Deco;
use crate::ai::{Npc, NpcState};
use crate::weapons::{image_for, RuntimeAssets};
use crate::{LevelRes, Player};
use bevy::prelude::*;
use dx_level::ue_to_bevy;
use glam::DVec3;

#[derive(Component)]
pub struct ShadowOf(pub Option<Entity>);

#[derive(Resource)]
pub struct ShadowAssets {
    mesh: Handle<Mesh>,
    mat: Handle<StandardMaterial>,
    /// Texture size in Unreal units at DrawScale 1.
    size: f32,
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
        let mesh = meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(0.5)).mesh());
        commands.insert_resource(ShadowAssets { mesh, mat, size });
        return;
    };
    let lvl = &level.0;
    // One shadow per pawn, plus the player's (None).
    let have: Vec<Option<Entity>> = shadows.iter().map(|(_, s, _, _)| s.0).collect();
    for (e, _, _) in &pawns {
        if !have.contains(&Some(e)) {
            commands.spawn((
                Mesh3d(sa.mesh.clone()),
                MeshMaterial3d(sa.mat.clone()),
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
                Mesh3d(sa.mesh.clone()),
                MeshMaterial3d(sa.mat.clone()),
                Transform::default(),
                Visibility::Hidden,
                ShadowOf(Some(*e)),
            ));
        }
    }
    if !have.contains(&None) {
        commands.spawn((
            Mesh3d(sa.mesh.clone()),
            MeshMaterial3d(sa.mat.clone()),
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
}
