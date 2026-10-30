//! Bullet impacts and tracers.
//!
//! * DeusExWeapon.TraceFire: with 10mm, 30.06 or 7.62mm ammo, half of the single-slug shots that travel more than
//!   250 units (and never through a scope) spawn a Tracer 96 units along the shot; AutoTurret.Fire does the same for
//!   half of its shots. A Tracer is a DeusExProjectile that flies at 4000 and does no damage (projectiles.rs).
//! * DeusExWeapon.ProcessTraceHit -> SpawnEffects -> TraceHitSpawner (only when the shot hits the level or a mover):
//!   penetrating bullets throw a little smoke puff (half the time) and up to two Rockchips (not off movers), leave a
//!   BulletHole decal and a Spark; movers that can be broken by the shot get crack art instead (glass doors
//!   FlatFXTex29/30 at 0.1, others FlatFXTex7/8 at 0.4). Melee blows only mark breakable movers.
//! * AutoTurret.Fire: a Spark on whatever it hits; on the level or a mover the puff, chips and a hole (cracked glass
//!   on Glass walls).
//! * DeusExDecal: a decal lasts 18-23 s, then stays while it has been seen in the last second and goes once it isn't
//!   (Enhancement "Casings And Blood Stay" keeps them).

use crate::destruct::{Loose, LooseSound};
use crate::movers::Movers;
use crate::weapons::{modulated_image_for, RuntimeAssets};
use crate::LevelRes;
use bevy::prelude::*;
use dx_level::ue_to_bevy;
use glam::DVec3;
use std::sync::Mutex;
use unreal_pkg::props::{find, Value};

/// Who fired the shot that hit the wall.
#[derive(Clone, Copy, PartialEq)]
pub enum Source {
    /// A weapon (the player's or a pawn's): TraceHitSpawner with its bPenetrating / bHandToHand.
    Weapon { penetrating: bool, hand_to_hand: bool },
    /// An AutoTurret (its own SpawnEffects).
    Turret,
}

struct WallHit {
    at: DVec3,
    normal: DVec3,
    dir: DVec3,
    damage: f32,
    source: Source,
}

static WALL_HITS: Mutex<Vec<WallHit>> = Mutex::new(Vec::new());
/// Sparks on something other than the level (turret shots at pawns and decorations).
static SPARKS: Mutex<Vec<(DVec3, DVec3)>> = Mutex::new(Vec::new());
static TRACERS: Mutex<Vec<(DVec3, DVec3)>> = Mutex::new(Vec::new());

/// A shot hit the level or a mover at `at` (surface normal `normal`, travelling along `dir`).
pub fn wall_hit(at: DVec3, normal: DVec3, dir: DVec3, damage: f32, source: Source) {
    if let Ok(mut q) = WALL_HITS.lock() {
        q.push(WallHit { at, normal: normal.normalize_or(-dir), dir: dir.normalize_or_zero(), damage, source });
    }
}

/// A Spark where a turret's bullet hit an actor.
pub fn spark(at: DVec3, normal: DVec3) {
    if let Ok(mut q) = SPARKS.lock() {
        q.push((at, normal));
    }
}

/// The ammo DeusExWeapon.TraceFire draws tracers for.
pub fn tracer_ammo(ammo: Option<&str>) -> bool {
    matches!(
        ammo.map(|a| a.rsplit('.').next().unwrap_or(a)),
        Some("Ammo10mm") | Some("Ammo3006") | Some("Ammo762mm")
    )
}

/// TraceFire's tracer: when the shot went more than 250 units, a Tracer from 96 units along it, flying toward
/// `end` (the aim point, not where it hit). The caller rolls the 50% and checks the ammo / scope / slugs.
pub fn tracer(start: DVec3, end: DVec3, hit_at: DVec3) {
    if hit_at.distance(start) <= 250.0 {
        return;
    }
    let dir = (end - start).normalize_or_zero();
    if dir == DVec3::ZERO {
        return;
    }
    if let Ok(mut q) = TRACERS.lock() {
        q.push((start + dir * 96.0, dir));
    }
}

/// A bullet hole / crack decal, with the time it's had and where it is (for the seen-recently check).
#[derive(Component)]
pub struct BulletDecal {
    age: f32,
    life: f32,
    pos: DVec3,
}

/// SmokeTrail puff: a camera-facing SmokePuff1 sprite (translucent) that grows (DrawScale x (1 + age)), rises at
/// RiseRate x age and fades (ScaleGlow = LifeSpan left / LifeSpan).
#[derive(Component)]
pub struct Puff {
    age: f32,
    life: f32,
    scale: f32,
    tex_size: f32,
    pos: DVec3,
    vz: f64,
    mat: Handle<StandardMaterial>,
}

#[allow(clippy::too_many_arguments)]
fn spawn_puff(
    commands: &mut Commands,
    assets: &mut RuntimeAssets,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    at: DVec3,
    scale: f32,
    life: f32,
) {
    let tex = "Effects.Smoke.SmokePuff1";
    let Some(img) = crate::weapons::image_for(assets, images, tex) else { return };
    let tex_size = assets.lib.width(tex).map(|w| w as f32).unwrap_or(64.0);
    let mat = materials.add(StandardMaterial {
        base_color_texture: Some(img),
        unlit: true,
        double_sided: true,
        cull_mode: None,
        alpha_mode: AlphaMode::Add,
        ..default()
    });
    let quad = meshes.add(Rectangle::new(1.0, 1.0));
    commands.spawn((
        Mesh3d(quad),
        MeshMaterial3d(mat.clone()),
        Transform::from_translation(Vec3::from(ue_to_bevy(at.as_vec3().to_array())))
            .with_scale(Vec3::splat((scale * tex_size * dx_level::METERS_PER_UNIT).max(0.001))),
        Puff { age: 0.0, life, scale, tex_size, pos: at, vz: 0.0, mat },
    ));
}

/// A decal on the level (not on a mover) that the Casings And Blood Stay enhancement keeps with the level.
#[derive(Component)]
pub struct KeepDecal {
    pub tex: String,
    /// A bullet hole (depth-biased, exposure-compensated) rather than a blood splat.
    pub hole: bool,
}

/// A saved decal: texture, Bevy transform (translation, rotation, uniform scale).
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct DecalSave {
    pub tex: String,
    pub pos: [f32; 3],
    pub rot: [f32; 4],
    pub size: f32,
    pub hole: bool,
}

static RESTORE: std::sync::Mutex<Vec<DecalSave>> = std::sync::Mutex::new(Vec::new());

/// Put saved decals back (drawn by impacts_system on its next run).
pub fn restore_decals(d: &[DecalSave]) {
    if let Ok(mut r) = RESTORE.lock() {
        r.extend(d.iter().cloned());
    }
}

/// The decal material: modulated over the surface (holes exposure-compensated so mid-grey leaves the wall alone).
pub fn decal_material(img: Handle<Image>, hole: bool) -> StandardMaterial {
    StandardMaterial {
        base_color_texture: Some(img),
        base_color: if hole { Color::LinearRgba(LinearRgba::rgb(4.59, 4.59, 4.59)) } else { Color::WHITE },
        unlit: true,
        double_sided: true,
        cull_mode: None,
        alpha_mode: AlphaMode::Multiply,
        depth_bias: if hole { 40.0 } else { 0.0 },
        ..default()
    }
}

/// Every decal made here, oldest first (capped).
#[derive(Resource, Default)]
pub struct Decals(pub std::collections::VecDeque<Entity>);

fn rot_of(dir: DVec3) -> [i32; 3] {
    let yaw = dir.y.atan2(dir.x);
    let pitch = dir.z.atan2(dir.truncate().length());
    let u = |r: f64| (r / std::f64::consts::TAU * 65536.0) as i32;
    [u(pitch), u(yaw), 0]
}

#[allow(clippy::too_many_arguments)]
pub fn impacts_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    mv: Res<Movers>,
    mut assets: ResMut<RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut launches: ResMut<crate::projectiles::Launches>,
    mut decals: ResMut<Decals>,
    (cam, tfs, mut aged, mut puffs): (
        Single<&Transform, (With<crate::FlyCam>, Without<crate::SkyCam>)>,
        Query<&Transform, (Without<crate::FlyCam>, Without<BulletDecal>, Without<Puff>)>,
        Query<&mut BulletDecal>,
        Query<(Entity, &mut Puff, &mut Transform), (Without<crate::FlyCam>, Without<BulletDecal>)>,
    ),
    mut seed: Local<u64>,
    mut map_gen: Local<u64>,
    mut check_t: Local<f32>,
) {
    let _prof = crate::prof::scope("impacts::impacts_system");
    if crate::loader::new_map(&mut map_gen) {
        decals.0.clear();
        for q in [&WALL_HITS] {
            if let Ok(mut q) = q.lock() {
                q.clear();
            }
        }
    }
    // Decals put back from the level's saved state (Casings And Blood Stay).
    for d in RESTORE.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        let Some(img) = modulated_image_for(&mut assets, &mut images, &d.tex) else { continue };
        let mat = materials.add(decal_material(img, d.hole));
        let quad = meshes.add(Rectangle::new(1.0, 1.0));
        let tf = Transform::from_translation(Vec3::from(d.pos))
            .with_rotation(Quat::from_array(d.rot))
            .with_scale(Vec3::splat(d.size));
        let ue = dx_level::bevy_to_ue(d.pos);
        let pos = DVec3::new(ue[0] as f64, ue[1] as f64, ue[2] as f64);
        let mut e = commands.spawn((Mesh3d(quad), MeshMaterial3d(mat), tf, KeepDecal { tex: d.tex.clone(), hole: d.hole }));
        if d.hole {
            e.insert(BulletDecal { age: 0.0, life: 20.0, pos });
            let id = e.id();
            decals.0.push_back(id);
        }
    }
    if *seed == 0 {
        *seed = 0x1A9C_7E55_0BAD_F00D;
    }
    let mut rnd = || {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((*seed >> 33) as f64) / (1u64 << 31) as f64
    };
    let lvl = &level.0;

    // Tracers.
    for (pos, dir) in TRACERS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        launches.0.push(crate::projectiles::Launch {
            class: "Tracer".into(),
            pos,
            dir,
            from: None,
            mult: 0.0,
            life: None,
            scale: None,
            still: false,
            track: None,
            track_at: None,
            speed: None,
            ownerless: false,
        });
    }

    let mut spark_at = |commands: &mut Commands,
                        assets: &mut RuntimeAssets,
                        images: &mut Assets<Image>,
                        meshes: &mut Assets<Mesh>,
                        materials: &mut Assets<StandardMaterial>,
                        at: DVec3,
                        normal: DVec3,
                        roll: f64| {
        // Spark: FlatFX with SparkFX1, translucent, DrawScale 0.05, a random roll, LifeSpan 0.25.
        let mut r = rot_of(normal);
        r[2] = (roll * 65535.0) as i32;
        crate::explosion::spawn_mesh_fx_timed(commands, assets, images, meshes, materials, "Spark", at, r, 0.05, 0.25);
    };
    for (at, n) in SPARKS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        let roll = rnd();
        spark_at(&mut commands, &mut assets, &mut images, &mut meshes, &mut materials, at + n, n, roll);
    }

    let hits: Vec<WallHit> = WALL_HITS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    let persistent = crate::settings::get("PersistentDebris") > 0.5;
    let rock = assets.classes.defaults("Rockchip");
    let rock_meshes: Vec<String> = rock
        .iter()
        .filter(|p| p.name.eq_ignore_ascii_case("Fragments"))
        .filter_map(|p| match &p.value {
            Value::Path(s) if s != "None" => Some(s.clone()),
            _ => None,
        })
        .collect();
    for h in hits {
        let mover = crate::movers::mover_on_line(lvl, &mv.rt, h.at - h.dir * 8.0, h.at + h.dir * 8.0).map(|(i, _)| i);
        let (pen, h2h, turret) = match h.source {
            Source::Weapon { penetrating, hand_to_hand } => (penetrating, hand_to_hand, false),
            Source::Turret => (true, false, true),
        };
        let spot = h.at + h.normal;
        // Enhanced Effects: sparks / dust / splinters by what the wall is made of.
        if !h2h || mover.is_some() {
            crate::fx::impact(h.at, h.normal, h.dir, lvl.wall_group(h.at.to_array(), h.normal.to_array()));
        }
        // A puff of smoke (SmokeTrail: DrawScale x 0.3, 0.25 s) half the time.
        if pen && !h2h && rnd() < 0.5 {
            spawn_puff(&mut commands, &mut assets, &mut images, &mut meshes, &mut materials, spot, 0.03, 0.25);
        }
        // Rock chips, not off movers (the turret skips only BreakableGlass, a decoration).
        if pen && !h2h && (mover.is_none() || turret) && !rock_meshes.is_empty() {
            for _ in 0..2 {
                if rnd() >= 0.8 {
                    continue;
                }
                let mesh = rock_meshes[((rnd() * rock_meshes.len() as f64) as usize).min(rock_meshes.len() - 1)].clone();
                let v = loop {
                    let v = DVec3::new(rnd() * 2.0 - 1.0, rnd() * 2.0 - 1.0, rnd() * 2.0 - 1.0);
                    let l = v.length_squared();
                    if l > 1e-4 && l <= 1.0 {
                        break v / l.sqrt();
                    }
                };
                let loose = Loose {
                    pos: spot,
                    vel: v * 100.0,
                    rot: [rnd() * 65536.0, rnd() * 65536.0, rnd() * 65536.0],
                    spin: [(rnd() - 0.5) * 250000.0, (rnd() - 0.5) * 250000.0, (rnd() - 0.5) * 250000.0],
                    elasticity: 0.4,
                    extent: DVec3::splat(0.5),
                    resting: false,
                    life: Some(10.0),
                    parts: Vec::new(),
                    lit_at: DVec3::ZERO,
                };
                let scale = 0.1 + rnd() as f32 * 0.25;
                if let Some(e) = crate::destruct::spawn_loose(
                    &mut commands, &mut assets, &mut images, &mut meshes, &mut materials, lvl, &mesh, &rock, scale, loose,
                ) {
                    commands.entity(e).insert(LooseSound([
                        crate::sound::prop(&rock, "ImpactSound"),
                        crate::sound::prop(&rock, "MiscSound"),
                    ]));
                }
            }
        }
        // The hole (and the spark with it).
        let mut hole: Option<(&'static str, f32)> = None;
        if !h2h && pen {
            hole = Some(("DeusExItems.Skins.FlatFXTex9", 0.1));
            if !turret {
                let roll = rnd();
                spark_at(&mut commands, &mut assets, &mut images, &mut meshes, &mut materials, spot, h.normal, roll);
            }
        }
        if turret {
            let glass = mover.is_none()
                && lvl.wall_group(h.at.to_array(), h.normal.to_array()).is_some_and(|g| g.eq_ignore_ascii_case("Glass"));
            if glass {
                hole = Some((if rnd() < 0.5 { "DeusExItems.Skins.FlatFXTex29" } else { "DeusExItems.Skins.FlatFXTex30" }, 0.1));
            }
        }
        // Damage art on movers.
        if let Some(i) = mover.filter(|_| pen || h2h) {
            let rt = &mv.rt[i];
            let props = &lvl.actors[lvl.movers[i].actor].props;
            let breakable = rt.breakable.unwrap_or(matches!(find(props, "bBreakable"), Some(Value::Bool(true))));
            let threshold = match find(props, "minDamageThreshold") {
                Some(Value::Int(v)) => *v as f32,
                Some(Value::Float(v)) => *v,
                _ => 0.0,
            };
            if breakable && threshold <= h.damage {
                if rt.destroyed {
                    hole = None;
                } else {
                    let glass = crate::sound::prop(props, "FragmentClass")
                        .is_some_and(|f| f.to_ascii_lowercase().ends_with("glassfragment"));
                    hole = Some(if glass {
                        (if rnd() < 0.5 { "DeusExItems.Skins.FlatFXTex29" } else { "DeusExItems.Skins.FlatFXTex30" }, 0.1)
                    } else {
                        (if rnd() < 0.5 { "DeusExItems.Skins.FlatFXTex7" } else { "DeusExItems.Skins.FlatFXTex8" }, 0.4)
                    });
                }
            } else if !pen || h2h {
                hole = None;
            } else if hole.is_none() {
                hole = Some(("DeusExItems.Skins.FlatFXTex9", 0.1));
            }
        }
        let Some((tex, scale)) = hole else { continue };
        let Some(img) = modulated_image_for(&mut assets, &mut images, tex) else { continue };
        let width = assets.lib.width(tex).map(|w| w as f32).unwrap_or(64.0);
        let size = width * scale;
        let normal = Vec3::from(ue_to_bevy(h.normal.as_vec3().to_array())).normalize_or_zero();
        let mat = materials.add(decal_material(img, true));
        let quad = meshes.add(Rectangle::new(1.0, 1.0));
        let at = h.at + h.normal * 0.2;
        let world_pos = Vec3::from(ue_to_bevy(at.as_vec3().to_array()));
        let world_rot = Quat::from_rotation_arc(Vec3::Z, normal) * Quat::from_rotation_z(rnd() as f32 * std::f32::consts::TAU);
        let s = Vec3::splat(size * dx_level::METERS_PER_UNIT);
        // On a mover the hole rides along with it.
        let parent = mover.and_then(|i| mv.entities.get(i).copied()).and_then(|e| tfs.get(e).ok().map(|t| (e, *t)));
        let tf = match parent {
            Some((_, pt)) => {
                let inv = pt.rotation.inverse();
                Transform::from_translation(inv * (world_pos - pt.translation)).with_rotation(inv * world_rot).with_scale(s)
            }
            None => Transform::from_translation(world_pos).with_rotation(world_rot).with_scale(s),
        };
        let life = 18.0 + 5.0 * rnd() as f32;
        let e = commands.spawn((Mesh3d(quad), MeshMaterial3d(mat), tf, BulletDecal { age: 0.0, life, pos: at })).id();
        if parent.is_none() {
            commands.entity(e).insert(KeepDecal { tex: tex.to_string(), hole: true });
        }
        if let Some((pe, _)) = parent {
            commands.entity(pe).add_child(e);
        }
        decals.0.push_back(e);
        let cap = if persistent { 3000 } else { 400 };
        while decals.0.len() > cap {
            if let Some(old) = decals.0.pop_front() {
                commands.entity(old).try_despawn();
            }
        }
    }

    // DeusExDecal.Timer: past its life a decal goes once it hasn't been on screen for a second (checked every
    // second here; "on screen" = in front of the camera within the view cone and not too far away).
    let dt = time.delta_secs();
    *check_t -= dt;
    let check = *check_t <= 0.0;
    if check {
        *check_t = 1.0;
    }
    for (e, mut p, mut tf) in &mut puffs {
        p.age += dt;
        if p.age >= p.life {
            commands.entity(e).try_despawn();
            continue;
        }
        // SmokeTrail.Tick: Velocity.Z = RiseRate (2) x elapsed; DrawScale = OrigScale x (1 + elapsed); fade.
        p.vz = 2.0 * p.age as f64;
        p.pos.z += p.vz * dt as f64;
        tf.translation = Vec3::from(ue_to_bevy(p.pos.as_vec3().to_array()));
        tf.rotation = cam.rotation;
        tf.scale = Vec3::splat((p.scale * (1.0 + p.age) * p.tex_size * dx_level::METERS_PER_UNIT).max(0.001));
        let glow = 1.0 - p.age / p.life;
        if let Some(mut m) = materials.get_mut(&p.mat) {
            m.base_color = Color::linear_rgb(glow, glow, glow);
        }
    }
    let eye = {
        let e = dx_level::bevy_to_ue(cam.translation.to_array());
        DVec3::new(e[0] as f64, e[1] as f64, e[2] as f64)
    };
    let fwd = {
        let f = cam.forward().as_vec3();
        DVec3::new(f.x as f64, f.z as f64, f.y as f64).normalize_or_zero()
    };
    let mut gone: Vec<Entity> = Vec::new();
    for e in decals.0.iter().copied() {
        let Ok(mut d) = aged.get_mut(e) else { continue };
        d.age += dt;
        if !check || persistent || d.age < d.life {
            continue;
        }
        let to = d.pos - eye;
        let seen = to.length() < 3000.0 && to.normalize_or_zero().dot(fwd) > 0.5;
        if !seen {
            gone.push(e);
        }
    }
    if !gone.is_empty() {
        decals.0.retain(|e| !gone.contains(e));
        for e in gone {
            commands.entity(e).try_despawn();
        }
    }
}

pub fn reset_level() {
    for q in [&WALL_HITS] {
        if let Ok(mut q) = q.lock() {
            q.clear();
        }
    }
    if let Ok(mut q) = SPARKS.lock() {
        q.clear();
    }
    if let Ok(mut q) = TRACERS.lock() {
        q.clear();
    }
}
