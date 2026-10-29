//! The player's death (DeusExPlayer state Dying): JC plays his death animation seen from 190 units behind,
//! then the body is left behind and the camera looks straight down at it, spiralling up while the screen fogs
//! to white over eight seconds, then fades to black over the next twelve. (The original then opens the main
//! menu; until there is one, Enter restarts.)

use crate::ai::{DamageFlash, PlayerHealth};
use crate::weapons::{spawn_mesh_parts, RuntimeAssets, ViewModelRoot};
use crate::{FlyCam, LevelRes, Player, SkyCam};
use bevy::prelude::*;
use dx_level::actors::{anim_mesh, light_part, rotator_to_bevy_mat3, AnimMesh, MeshPart};
use dx_level::{bevy_to_ue, ue_to_bevy};
use glam::DVec3;
use std::sync::Arc;

pub struct Body {
    root: Entity,
    mesh: Arc<AnimMesh>,
    parts: Vec<MeshPart>,
    handles: Vec<Handle<Mesh>>,
    start: usize,
    frames: usize,
    rate: f32,
    anim_len: f32,
    at: DVec3,
    yaw: f64,
}

#[derive(Default)]
pub struct DeathState {
    t: f32,
    body: Option<Body>,
}

#[allow(clippy::too_many_arguments)]
pub fn death_system(
    mut commands: Commands,
    time: Res<Time>,
    ph: Res<PlayerHealth>,
    player: Res<Player>,
    level: Res<LevelRes>,
    mut cams: ParamSet<(
        Single<&mut Transform, (With<FlyCam>, Without<SkyCam>)>,
        Single<&mut Transform, With<SkyCam>>,
    )>,
    mut vm: Single<&mut Visibility, With<ViewModelRoot>>,
    mut flash: Single<&mut BackgroundColor, With<DamageFlash>>,
    (mut assets, mut images, mut meshes, mut materials): (
        ResMut<RuntimeAssets>,
        ResMut<Assets<Image>>,
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
    ),
    mut map_gen: Local<u64>,
    mut st: Local<DeathState>,
) {
    // A new map (loader.rs): start this system's per-map state over.
    if crate::loader::new_map(&mut map_gen) {
        *st = Default::default();
    }
    let _prof = crate::prof::scope("death::death_system");
    if !ph.dead {
        if let Some(b) = st.body.take() {
            commands.entity(b.root).despawn();
        }
        st.t = 0.0;
        **vm = Visibility::Inherited;
        return;
    }
    **vm = Visibility::Hidden;
    let dt = time.delta_secs();
    let first = st.t == 0.0;
    st.t += dt;
    let lvl = &level.0;
    let eye_now = cams.p0().translation;
    if first {
        // Spawn JC (JCDentonMale: GM_Trench with his skins) where the player stood, facing the view direction.
        let d = crate::travel::jc_defaults(&mut assets.classes);
        let loc = player.body.location;
        let fwd = cams.p0().forward().as_vec3();
        let yaw = (fwd.z as f64).atan2(fwd.x as f64);
        let rot = [0, (yaw / std::f64::consts::TAU * 65536.0) as i32, 0];
        let m = rotator_to_bevy_mat3(rot);
        let root = commands
            .spawn((
                Transform::from_translation(Vec3::from(ue_to_bevy(loc.as_vec3().to_array())))
                    .with_rotation(Quat::from_mat3(&Mat3::from_cols(
                        Vec3::from(m[0]),
                        Vec3::from(m[1]),
                        Vec3::from(m[2]),
                    ))),
                Visibility::default(),
            ))
            .id();
        let map = |p: [f32; 3]| ue_to_bevy(p);
        if let Some((lm, parts, handles)) = spawn_mesh_parts(
            &crate::travel::jc_mesh(),
            1.0,
            &d,
            &mut assets,
            &mut images,
            &mut meshes,
            &mut materials,
            &mut commands,
            root,
            &map,
            None,
        ) {
            let am = Arc::new(anim_mesh(&lm));
            // Human.PlayDying: DeathBack when hit from the front (the usual case), DeathFront from behind.
            let seq = am
                .seqs
                .iter()
                .find(|s| s.name.eq_ignore_ascii_case("DeathBack"))
                .map(|s| {
                    (
                        s.start_frame.max(0) as usize,
                        s.num_frames.max(1) as usize,
                        s.rate,
                    )
                });
            let (start, frames, rate) = seq.unwrap_or((0, 1, 10.0));
            let at = loc.as_vec3().to_array();
            for (pi, h) in handles.iter().enumerate() {
                if let Some(mut mm) = meshes.get_mut(h) {
                    mm.insert_attribute(
                        Mesh::ATTRIBUTE_COLOR,
                        light_part(&parts[pi], at, rot, 1.0, &lvl.lights),
                    );
                }
            }
            let anim_len = (frames as f32 - 1.0).max(1.0) / rate.max(1.0);
            st.body = Some(Body {
                root,
                mesh: am,
                parts,
                handles,
                start,
                frames,
                rate,
                anim_len,
                at: loc,
                yaw,
            });
        } else {
            commands.entity(root).despawn();
        }
    }
    let t = st.t;
    let Some(b) = st.body.as_ref() else { return };
    // Animate the death (holds the last frame).
    let f = (t * b.rate).min(b.frames as f32 - 1.001);
    for (pi, h) in b.handles.iter().enumerate() {
        if let Some(mut m) = meshes.get_mut(h) {
            let pos: Vec<[f32; 3]> = b.parts[pi]
                .vert_ids
                .iter()
                .map(|&v| ue_to_bevy(b.mesh.sample(b.start, b.frames, f, v as usize)))
                .collect();
            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
        }
    }
    let (cam_pos, cam_rot, fog): (DVec3, Quat, f32) = if t < b.anim_len + 0.3 {
        // Third person, 190 units behind, looking at JC (stop short of walls).
        let back = DVec3::new(-b.yaw.cos(), -b.yaw.sin(), 0.0);
        let want = b.at + back * 190.0 + DVec3::new(0.0, 0.0, 30.0);
        let h = lvl.collision.trace(b.at, want, DVec3::new(12.0, 12.0, 2.0));
        let p = b.at + (want - b.at) * h.fraction;
        let look = (b.at - p).normalize_or_zero();
        let lb = Vec3::new(look.x as f32, look.z as f32, look.y as f32);
        (p, Transform::IDENTITY.looking_to(lb, Vec3::Y).rotation, 0.0)
    } else {
        // bHidden: spiral up over the body looking straight down.
        let tt = t - b.anim_len - 0.3;
        let dist = 32.0 + (tt.min(8.0) as f64) * 32.0;
        let up = b.at + DVec3::new(0.0, 0.0, dist);
        let h = lvl.collision.trace(b.at, up, DVec3::ZERO);
        let p = b.at + (up - b.at) * h.fraction;
        let yaw = (tt as f64 * 8192.0 / 65536.0 * std::f64::consts::TAU) % std::f64::consts::TAU;
        let rot = Quat::from_rotation_y(-(yaw as f32) - std::f32::consts::FRAC_PI_2)
            * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
        let fog = if tt < 8.0 {
            tt / 16.0
        } else {
            (0.5 - (tt - 8.0) / 8.0).max(-1.0)
        };
        (p, rot, fog)
    };
    let _ = eye_now;
    {
        let mut c = cams.p0();
        c.translation = Vec3::from(ue_to_bevy(cam_pos.as_vec3().to_array()));
        c.rotation = cam_rot;
    }
    cams.p1().rotation = cam_rot;
    // InstantFog: white while positive, darkening to black below zero.
    flash.0 = if fog >= 0.0 {
        Color::srgba(1.0, 1.0, 1.0, fog.min(0.5))
    } else {
        Color::srgba(0.0, 0.0, 0.0, (-fog).min(1.0))
    };
    let _ = bevy_to_ue;
}
