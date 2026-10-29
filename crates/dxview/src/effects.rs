//! Hit effects. DeusExWeapon.SpawnBlood: a penetrating weapon hitting a (non-robot) pawn makes a BloodSpurt
//! (the 'Spurt' mesh animation, half a second, facing along the shot) and one or two BloodDrops (fragments thrown
//! at VRand * 100 that fall and stay a while).

use crate::destruct::Loose;
use crate::weapons::{spawn_mesh_parts, RuntimeAssets};
use crate::LevelRes;
use bevy::prelude::*;
use dx_level::actors::{anim_mesh, rotator_to_bevy_mat3, AnimMesh, MeshPart};
use dx_level::ue_to_bevy;
use glam::DVec3;
use std::sync::Arc;

/// Blood to spawn: where, and the direction the shot travelled.
#[derive(Resource, Default)]
pub struct BloodFx(pub Vec<(DVec3, DVec3)>);

/// Drips from wounded pawns (ScriptedPawn bleeding: SpurtBlood drops a BloodDrop at mid-body).
#[derive(Resource, Default)]
pub struct BloodDrips(pub Vec<DVec3>);

/// A falling BloodDrop: where it lands it leaves a BloodSplat decal.
#[derive(Component)]
pub struct BloodDropMark;

/// BloodSplat decals on the ground, oldest first (capped so long fights don't pile up thousands).
#[derive(Resource, Default)]
pub struct Splats(pub std::collections::VecDeque<Entity>);

#[derive(Component)]
pub struct Spurt {
    mesh: Arc<AnimMesh>,
    parts: Vec<MeshPart>,
    handles: Vec<Handle<Mesh>>,
    start: usize,
    frames: usize,
    rate: f32,
    t: f32,
    scale: f32,
}

fn rot_of(dir: DVec3) -> [i32; 3] {
    let yaw = dir.y.atan2(dir.x);
    let pitch = dir.z.atan2(dir.truncate().length());
    let u = |r: f64| (r / std::f64::consts::TAU * 65536.0) as i32;
    [u(pitch), u(yaw), 0]
}

fn quat_of(rot: [i32; 3]) -> Quat {
    let m = rotator_to_bevy_mat3(rot);
    Quat::from_mat3(&Mat3::from_cols(
        Vec3::from(m[0]),
        Vec3::from(m[1]),
        Vec3::from(m[2]),
    ))
}

#[allow(clippy::too_many_arguments)]
pub fn blood_system(
    mut commands: Commands,
    time: Res<Time>,
    mut fx: ResMut<BloodFx>,
    mut assets: ResMut<RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    level: Res<LevelRes>,
    mut spurts: Query<(Entity, &mut Spurt)>,
    mut seed: Local<u64>,
    (mut drips, mut splats, landed): (
        ResMut<BloodDrips>,
        ResMut<Splats>,
        Query<(Entity, &Loose), With<BloodDropMark>>,
    ),
) {
    let _prof = crate::prof::scope("effects::blood_system");
    if *seed == 0 {
        *seed = 0xB100_D5EE;
    }
    let mut rnd = || {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((*seed >> 33) as f64) / (1u64 << 31) as f64
    };
    for (at, dir) in fx.0.drain(..) {
        // BloodSpurt: DrawScale 1 - FRand()*0.5, PlayAnim('Spurt'), LifeSpan 0.5.
        let sd = assets.classes.defaults("BloodSpurt");
        let scale = 1.0 - rnd() as f32 * 0.5;
        let root = commands
            .spawn((
                Transform::from_translation(Vec3::from(ue_to_bevy(at.as_vec3().to_array())))
                    .with_rotation(quat_of(rot_of(dir))),
                Visibility::default(),
            ))
            .id();
        let map = |p: [f32; 3]| ue_to_bevy(p);
        match spawn_mesh_parts(
            "DeusExItems.BloodSpurt",
            scale,
            &sd,
            &mut assets,
            &mut images,
            &mut meshes,
            &mut materials,
            &mut commands,
            root,
            &map,
            None,
        ) {
            Some((m, parts, handles)) => {
                let am = Arc::new(anim_mesh(&m));
                let seq = am
                    .seqs
                    .iter()
                    .find(|s| s.name.eq_ignore_ascii_case("Spurt"))
                    .map(|s| {
                        (
                            s.start_frame.max(0) as usize,
                            s.num_frames.max(1) as usize,
                            s.rate,
                        )
                    });
                let (start, frames, rate) = seq.unwrap_or((0, 1, 24.0));
                for h in &handles {
                    if let Some(mut mm) = meshes.get_mut(h) {
                        let n = mm
                            .attribute(Mesh::ATTRIBUTE_POSITION)
                            .map(|a| a.len())
                            .unwrap_or(0);
                        mm.insert_attribute(
                            Mesh::ATTRIBUTE_COLOR,
                            vec![[1.0f32, 1.0, 1.0, 1.0]; n],
                        );
                    }
                }
                commands.entity(root).insert(Spurt {
                    mesh: am,
                    parts,
                    handles,
                    start,
                    frames,
                    rate,
                    t: 0.0,
                    scale,
                });
            }
            None => commands.entity(root).despawn(),
        }
        // BloodDrop x1 (x2 half the time): thrown at VRand * 100, DrawScale 1..2.
        let drops = if rnd() < 0.5 { 2 } else { 1 };
        let dd = assets.classes.defaults("BloodDrop");
        for _ in 0..drops {
            let v = loop {
                let v = DVec3::new(rnd() * 2.0 - 1.0, rnd() * 2.0 - 1.0, rnd() * 2.0 - 1.0);
                let l = v.length_squared();
                if l > 1e-4 && l <= 1.0 {
                    break v / l.sqrt();
                }
            };
            let loose = Loose {
                pos: at,
                vel: v * 100.0,
                rot: [0.0, rnd() * 65536.0, 0.0],
                spin: [0.0; 3],
                elasticity: 0.0,
                extent: DVec3::splat(1.0),
                resting: false,
                life: Some(10.0),
                parts: Vec::new(),
                lit_at: DVec3::ZERO,
            };
            if let Some(e) = crate::destruct::spawn_loose(
                &mut commands,
                &mut assets,
                &mut images,
                &mut meshes,
                &mut materials,
                &level.0,
                "DeusExItems.BloodDrop",
                &dd,
                1.0 + rnd() as f32,
                loose,
            ) {
                commands.entity(e).insert(BloodDropMark);
            }
        }
    }
    // Drips from the wounded: a BloodDrop thrown at VRand * 100 from mid-body.
    let dd = assets.classes.defaults("BloodDrop");
    for at in std::mem::take(&mut drips.0) {
        let v =
            DVec3::new(rnd() * 2.0 - 1.0, rnd() * 2.0 - 1.0, rnd() * 2.0 - 1.0).normalize_or_zero();
        let loose = Loose {
            pos: at,
            vel: v * 100.0,
            rot: [0.0, rnd() * 65536.0, 0.0],
            spin: [0.0; 3],
            elasticity: 0.0,
            extent: DVec3::splat(1.0),
            resting: false,
            life: Some(10.0),
            parts: Vec::new(),
            lit_at: DVec3::ZERO,
        };
        if let Some(e) = crate::destruct::spawn_loose(
            &mut commands,
            &mut assets,
            &mut images,
            &mut meshes,
            &mut materials,
            &level.0,
            "DeusExItems.BloodDrop",
            &dd,
            1.0 + rnd() as f32,
            loose,
        ) {
            commands.entity(e).insert(BloodDropMark);
        }
    }
    // BloodDrop.HitWall / landing: a BloodSplat decal (FlatFXTex2/3/5/6, DrawScale 0.2-0.4, modulated) on the
    // surface it hit, and the drop is gone.
    for (e, l) in &landed {
        if !l.resting {
            continue;
        }
        commands.entity(e).despawn();
        let lvl = &level.0;
        let mut best: Option<(f64, DVec3)> = None;
        for d in [
            DVec3::NEG_Z,
            DVec3::X,
            DVec3::NEG_X,
            DVec3::Y,
            DVec3::NEG_Y,
            DVec3::Z,
        ] {
            let h = lvl.collision.trace(l.pos, l.pos + d * 6.0, DVec3::ZERO);
            if h.fraction < 1.0 && best.map_or(true, |(f, _)| h.fraction < f) {
                best = Some((h.fraction, h.normal));
            }
        }
        let Some((f, n)) = best else { continue };
        let tex = [
            "DeusExItems.Skins.FlatFXTex2",
            "DeusExItems.Skins.FlatFXTex3",
            "DeusExItems.Skins.FlatFXTex5",
            "DeusExItems.Skins.FlatFXTex6",
        ][((rnd() * 4.0) as usize).min(3)];
        let Some(img) = crate::weapons::modulated_image_for(&mut assets, &mut images, tex) else {
            continue;
        };
        let width = assets
            .lib
            .load_image(tex)
            .map(|t| t.width as f32)
            .unwrap_or(64.0);
        let size = width * (0.2 + rnd() as f32 * 0.2);
        let at = l.pos + DVec3::NEG_Z * 0.0 + (-n) * (f * 6.0) + n * 0.3;
        let normal = Vec3::from(ue_to_bevy(n.as_vec3().to_array())).normalize_or_zero();
        let mat = materials.add(crate::impacts::decal_material(img, false));
        let quad = meshes.add(Rectangle::new(1.0, 1.0));
        let rot = Quat::from_rotation_arc(Vec3::Z, normal)
            * Quat::from_rotation_z(rnd() as f32 * std::f32::consts::TAU);
        let splat = commands
            .spawn((
                Mesh3d(quad),
                MeshMaterial3d(mat),
                Transform::from_translation(Vec3::from(ue_to_bevy(at.as_vec3().to_array())))
                    .with_rotation(rot)
                    .with_scale(Vec3::splat(size * dx_level::METERS_PER_UNIT)),
                crate::impacts::KeepDecal { tex: tex.to_string(), hole: false },
            ))
            .id();
        splats.0.push_back(splat);
        // Enhancement "Casings And Blood Stay": many more before the oldest go.
        let cap = if crate::settings::get("PersistentDebris") > 0.5 { 3000 } else { 300 };
        while splats.0.len() > cap {
            if let Some(old) = splats.0.pop_front() {
                commands.entity(old).despawn();
            }
        }
    }
    let dt = time.delta_secs();
    for (e, mut s) in &mut spurts {
        s.t += dt;
        let f = s.t * s.rate;
        if f >= s.frames as f32 - 1.0 || s.t > 0.5 {
            commands.entity(e).despawn();
            continue;
        }
        let s = &*s;
        for (pi, h) in s.handles.iter().enumerate() {
            if let Some(mut m) = meshes.get_mut(h) {
                let pos: Vec<[f32; 3]> = s.parts[pi]
                    .vert_ids
                    .iter()
                    .map(|&v| {
                        let p = s.mesh.sample(s.start, s.frames, f, v as usize);
                        ue_to_bevy([p[0] * s.scale, p[1] * s.scale, p[2] * s.scale])
                    })
                    .collect();
                m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
            }
        }
    }
}

/// Pawns blown apart this frame: (centre, radius, height, robot).
#[derive(Resource, Default)]
pub struct Gibs(pub Vec<(DVec3, f64, f64, bool)>);

/// ScriptedPawn.SpawnCarcass when Health < -100: (radius + height) / 8 FleshFragments of DrawScale size / 25 thrown
/// about (robots throw MetalFragments). The body itself is hidden.
#[allow(clippy::too_many_arguments)]
pub fn gib_system(
    mut commands: Commands,
    mut gibs: ResMut<Gibs>,
    mut assets: ResMut<RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    level: Res<LevelRes>,
    mut npcs: Query<(&crate::ai::Npc, &crate::actors::Deco, &mut Visibility)>,
    mut seed: Local<u64>,
) {
    let _prof = crate::prof::scope("effects::gib_system");
    if gibs.0.is_empty() {
        return;
    }
    if *seed == 0 {
        *seed = 0x61B5;
    }
    let mut rnd = || {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((*seed >> 33) as f64) / (1u64 << 31) as f64
    };
    for (c, r, h, robot) in gibs.0.drain(..) {
        // Hide the body that was here.
        for (n, d, mut v) in &mut npcs {
            if n.carried
                && n.state == crate::ai::NpcState::Dead
                && d.body.location.distance(c) < 1.0
            {
                *v = Visibility::Hidden;
            }
        }
        let size = (r + h) / 2.0;
        let (class, mesh) = if robot {
            ("MetalFragment", "DeusExItems.MetalFragment1")
        } else {
            ("FleshFragment", "DeusExItems.FleshFragment1")
        };
        let d = assets.classes.defaults(class);
        let n = if robot {
            (size / 6.0).max(3.0) as usize
        } else {
            (size / 4.0) as usize
        };
        for i in 0..n {
            let off = DVec3::new(
                (1.0 - 2.0 * rnd()) * r,
                (1.0 - 2.0 * rnd()) * r,
                (1.0 - 2.0 * rnd()) * h,
            );
            let v = DVec3::new(rnd() * 2.0 - 1.0, rnd() * 2.0 - 1.0, rnd() * 0.8 + 0.4)
                .normalize_or_zero()
                * (200.0 + rnd() * 250.0);
            let loose = Loose {
                pos: c + off,
                vel: v,
                rot: [rnd() * 65536.0, rnd() * 65536.0, rnd() * 65536.0],
                spin: [
                    (rnd() - 0.5) * 80000.0,
                    (rnd() - 0.5) * 80000.0,
                    (rnd() - 0.5) * 80000.0,
                ],
                elasticity: 0.3,
                extent: DVec3::splat(2.0),
                resting: false,
                life: Some(10.0 + rnd() as f32 * 5.0),
                parts: Vec::new(),
                lit_at: DVec3::ZERO,
            };
            let mesh = if !robot && i % 3 == 1 {
                "DeusExItems.FleshFragment2"
            } else if !robot && i % 3 == 2 {
                "DeusExItems.FleshFragment3"
            } else {
                mesh
            };
            let scale = if robot {
                (size * 0.075 * rnd()).max(0.3) as f32
            } else {
                (size / 25.0) as f32
            };
            crate::destruct::spawn_loose(
                &mut commands,
                &mut assets,
                &mut images,
                &mut meshes,
                &mut materials,
                &level.0,
                mesh,
                &d,
                scale,
                loose,
            );
        }
        if !robot {
            // A burst of blood too.
            let mut fx = Vec::new();
            for _ in 0..4 {
                fx.push((
                    c + DVec3::new((rnd() - 0.5) * r, (rnd() - 0.5) * r, (rnd() - 0.5) * h),
                    DVec3::Z,
                ));
            }
            commands.queue(move |w: &mut bevy::ecs::world::World| {
                if let Some(mut b) = w.get_resource_mut::<BloodFx>() {
                    b.0.extend(fx);
                }
            });
        }
    }
}

/// The Fire actors CatchFire attaches: eight flame sprites (OnFire_J, animated) scattered over the body, DrawScale
/// 1-1.5; half of them also give off smoke (a ParticleGenerator of SmokePuff1: life 2 s, rising 90/s) and half
/// throw extra licks of flame (OneFlame_J: life 0.5 s, rising 30/s).
#[derive(Resource, Default)]
pub struct BurnFx(pub std::collections::HashMap<usize, Vec<FireSprite>>);

pub struct FireSprite {
    entity: Entity,
    off: Vec3,
    smoke: bool,
    flames: bool,
    smoke_t: f32,
    flame_t: f32,
}

/// One particle from a fire's ParticleGenerator: rises, spreads, grows (bScale) and fades over its life.
#[derive(Component)]
pub struct FireParticle {
    pos: DVec3,
    vel: DVec3,
    age: f32,
    life: f32,
    size: f32,
    grow: bool,
    mat: Handle<StandardMaterial>,
}

#[allow(clippy::too_many_arguments)]
pub fn burn_fx_system(
    mut commands: Commands,
    time: Res<Time>,
    mut fx: ResMut<BurnFx>,
    npcs: Query<&crate::ai::Npc>,
    ph: Res<crate::ai::PlayerHealth>,
    player: Res<crate::Player>,
    blockers: Res<crate::actors::Blockers>,
    cam: Single<&Transform, (With<crate::FlyCam>, Without<crate::SkyCam>)>,
    mut tfs: Query<
        &mut Transform,
        (
            Without<crate::FlyCam>,
            Without<crate::ai::Npc>,
            Without<FireParticle>,
        ),
    >,
    mut parts: Query<
        (Entity, &mut FireParticle, &mut Transform),
        (Without<crate::FlyCam>, Without<crate::ai::Npc>),
    >,
    (mut assets, mut images, mut meshes, mut materials, mut seed): (
        ResMut<RuntimeAssets>,
        ResMut<Assets<Image>>,
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
        Local<u64>,
    ),
) {
    let _prof = crate::prof::scope("effects::burn_fx_system");
    let dt = time.delta_secs().min(0.1);
    // Who is alight: pawns by blocker, the player as usize::MAX (with centre, radius, height).
    let mut burning: Vec<(usize, DVec3, f64, f64)> = npcs
        .iter()
        .filter(|n| {
            n.on_fire.is_some() && n.state != crate::ai::NpcState::Dead && n.blocker != usize::MAX
        })
        .map(|n| {
            let c = blockers.list[n.blocker];
            (n.blocker, c.center, n.params.radius, n.params.height)
        })
        .collect();
    if ph.on_fire.is_some() {
        burning.push((
            usize::MAX,
            player.body.location,
            player.params.radius,
            player.params.height,
        ));
    }
    // Put out fires that stopped (their particles live out their time).
    let keys: Vec<usize> = fx.0.keys().copied().collect();
    for k in keys {
        if !burning.iter().any(|b| b.0 == k) {
            for f in fx.0.remove(&k).unwrap_or_default() {
                commands.entity(f.entity).despawn();
            }
        }
    }
    if *seed == 0 {
        *seed = 0xF1AE;
    }
    let mut rnd = || {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((*seed >> 33) as f32) / (1u64 << 31) as f32
    };
    let quad = meshes.add(Rectangle::new(1.0, 1.0));
    let mut spawn_particle = |commands: &mut Commands,
                              assets: &mut RuntimeAssets,
                              images: &mut Assets<Image>,
                              materials: &mut Assets<StandardMaterial>,
                              tex: &str,
                              at: DVec3,
                              vel: DVec3,
                              life: f32,
                              draw_scale: f32,
                              grow: bool| {
        let img = crate::weapons::image_for(assets, images, tex);
        let width = assets
            .lib
            .load_fire(tex)
            .map(|f| f.width as f32)
            .unwrap_or(32.0);
        let mat = materials.add(StandardMaterial {
            base_color_texture: img,
            unlit: true,
            double_sided: true,
            cull_mode: None,
            alpha_mode: AlphaMode::Add,
            ..default()
        });
        commands.spawn((
            Mesh3d(quad.clone()),
            MeshMaterial3d(mat.clone()),
            Transform::from_translation(Vec3::from(ue_to_bevy(at.as_vec3().to_array())))
                .with_scale(Vec3::splat(width * draw_scale * dx_level::METERS_PER_UNIT)),
            FireParticle {
                pos: at,
                vel,
                age: 0.0,
                life,
                size: width * draw_scale,
                grow,
                mat,
            },
        ));
    };
    for (k, c, r, h) in burning {
        if !fx.0.contains_key(&k) {
            let tex = "Effects.Fire.OnFire_J";
            let img = crate::weapons::image_for(&mut assets, &mut images, tex);
            let size = assets
                .lib
                .load_fire(tex)
                .map(|f| f.width as f32)
                .unwrap_or(32.0);
            let mat = materials.add(StandardMaterial {
                base_color_texture: img,
                unlit: true,
                double_sided: true,
                cull_mode: None,
                alpha_mode: AlphaMode::Add,
                ..default()
            });
            let mut v = Vec::new();
            for _ in 0..8 {
                let off = Vec3::new(
                    0.5 * r as f32 * (1.0 - 2.0 * rnd()),
                    0.5 * r as f32 * (1.0 - 2.0 * rnd()),
                    0.6 * h as f32 * (1.0 - 2.0 * rnd()),
                );
                let scale = (0.5 * rnd() + 1.0) * size * dx_level::METERS_PER_UNIT;
                let e = commands
                    .spawn((
                        Mesh3d(quad.clone()),
                        MeshMaterial3d(mat.clone()),
                        Transform::from_scale(Vec3::splat(scale)),
                        Visibility::default(),
                    ))
                    .id();
                v.push(FireSprite {
                    entity: e,
                    off,
                    smoke: rnd() < 0.5,
                    flames: rnd() < 0.5,
                    smoke_t: rnd() * 0.15,
                    flame_t: rnd() * 0.075,
                });
            }
            fx.0.insert(k, v);
        }
        let mut emit: Vec<(bool, DVec3)> = Vec::new();
        for f in fx.0.get_mut(&k).unwrap() {
            let p = c + DVec3::new(f.off.x as f64, f.off.y as f64, f.off.z as f64);
            if let Ok(mut t) = tfs.get_mut(f.entity) {
                t.translation = Vec3::from(ue_to_bevy(p.as_vec3().to_array()));
                t.rotation = cam.rotation;
            }
            // ParticleGenerator: every checkTime, a 90% chance of a puff.
            if f.smoke {
                f.smoke_t += dt;
                if f.smoke_t >= 0.15 {
                    f.smoke_t = 0.0;
                    emit.push((true, p));
                }
            }
            if f.flames {
                f.flame_t += dt;
                if f.flame_t >= 0.075 {
                    f.flame_t = 0.0;
                    emit.push((false, p));
                }
            }
        }
        for (smoke, p) in emit {
            if rnd() > 0.9 {
                continue;
            }
            let dir = DVec3::new(
                rnd() as f64 * 2.0 - 1.0,
                rnd() as f64 * 2.0 - 1.0,
                rnd() as f64 * 2.0 - 1.0,
            )
            .normalize_or_zero();
            if smoke {
                spawn_particle(
                    &mut commands,
                    &mut assets,
                    &mut images,
                    &mut materials,
                    "Effects.Smoke.SmokePuff1",
                    p,
                    dir * 40.0 + DVec3::Z * 90.0,
                    2.0,
                    0.5,
                    true,
                );
            } else {
                spawn_particle(
                    &mut commands,
                    &mut assets,
                    &mut images,
                    &mut materials,
                    "Effects.Fire.OneFlame_J",
                    p,
                    dir * 20.0 + DVec3::Z * 30.0,
                    0.5,
                    0.7,
                    false,
                );
            }
        }
    }
    // Particles: rise, spread, grow (smoke), fade; gone at the end of their life.
    for (e, mut q, mut t) in &mut parts {
        q.age += dt;
        if q.age >= q.life {
            commands.entity(e).despawn();
            continue;
        }
        let k = q.age / q.life;
        let v = q.vel;
        q.pos += v * dt as f64;
        t.translation = Vec3::from(ue_to_bevy(q.pos.as_vec3().to_array()));
        t.rotation = cam.rotation;
        let grow = if q.grow { 1.0 + 2.0 * k } else { 1.0 };
        t.scale = Vec3::splat(q.size * grow * dx_level::METERS_PER_UNIT);
        if let Some(mut m) = materials.get_mut(&q.mat) {
            let g = 1.0 - k;
            m.base_color = Color::linear_rgb(g, g, g);
        }
    }
}

/// CleanerBot.Tick: every 0.3 s each cleaner destroys the decals it is driving over (within 1.2 x its radius
/// across, below its centre and no deeper than its height + 10).
pub fn cleaner_system(
    mut commands: Commands,
    time: Res<Time>,
    mut splats: ResMut<Splats>,
    tfs: Query<&Transform>,
    mut clock: Local<f32>,
) {
    *clock += time.delta_secs();
    if *clock <= 0.3 {
        return;
    }
    *clock = 0.0;
    let cleaners = crate::ai::CLEANERS.lock().map(|c| c.clone()).unwrap_or_default();
    if cleaners.is_empty() || splats.0.is_empty() {
        return;
    }
    splats.0.retain(|&e| {
        let Ok(tf) = tfs.get(e) else { return false };
        let at = dx_level::bevy_to_ue(tf.translation.to_array());
        let at = DVec3::new(at[0] as f64, at[1] as f64, at[2] as f64);
        let gone = cleaners.iter().any(|&(c, r, h)| {
            let dz = at.z - c.z;
            (at - c).truncate().length() <= r * 1.2 && dz < 0.0 && dz > -(h + 10.0)
        });
        if gone {
            commands.entity(e).despawn();
        }
        !gone
    });
}
