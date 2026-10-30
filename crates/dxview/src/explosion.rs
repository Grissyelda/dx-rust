//! Explosions (DeusExProjectile state Exploding, Robot.Explode, ThrownProjectile): an animated fireball sprite
//! (ExplosionSmall/Medium/Large by blast radius), a white screen flash scaled by distance (ClientFlash), loud
//! noises for the AI, and damage dealt as five growing rings over a quarter second (DamageRing -> HurtRadius:
//! 2 x Damage / 5 per ring, scaled by 1 - (distance - CollisionRadius) / ring radius, only where there's a line of
//! sight). Killing a pawn with an explosion gibs it.

use crate::actors::{Blockers, World};
use crate::ai::{PlayerHealth, Target};
use crate::movers::Movers;
use crate::weapons::{image_for, RuntimeAssets};
use crate::{FlyCam, LevelRes, Player, SkyCam};
use bevy::prelude::*;
use dx_level::{bevy_to_ue, ue_to_bevy};
use glam::DVec3;
use imsim_core::movement::CollisionWorld;
use unreal_pkg::props::{find, Value};

#[derive(Clone, Debug)]
pub struct Boom {
    pub pos: DVec3,
    pub damage: f32,
    pub radius: f64,
    pub damage_type: &'static str,
    /// Who set it off (None = the player or the world).
    pub from: Option<usize>,
    /// DamageRing (five rings) or a single HurtRadius (robots blowing up).
    pub rings: bool,
    /// Draw the fireball (gas and EMP grenades don't).
    pub fireball: bool,
    /// Which DrawExplosionEffects: 0 plain, 1 projectile (fireball + three ShockRings), 2 thrown grenade (several
    /// fireballs across the blast), 3 EMP grenade (small fireball + SphereEffect).
    pub fx: u8,
}

/// ShockRing / SphereEffect: a translucent mesh that grows to `size` (x3 for the sphere) and fades over LifeSpan.
#[derive(Component)]
pub struct MeshFx {
    t: f32,
    life: f32,
    size: f32,
    mats: Vec<Handle<StandardMaterial>>,
    /// WaterRing: starts at this DrawScale and grows by the rate per second, glow fading linearly
    /// (instead of growing from nothing to `size`).
    pub grow: Option<(f32, f32)>,
}

/// A mesh effect that keeps one DrawScale for `life` seconds, fading out (Spark: FlatFX, 0.25 s).
#[allow(clippy::too_many_arguments)]
pub fn spawn_mesh_fx_timed(
    commands: &mut Commands,
    assets: &mut RuntimeAssets,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    class: &str,
    at: DVec3,
    rot: [i32; 3],
    scale: f32,
    life: f32,
) -> Option<Entity> {
    spawn_mesh_fx_ex(commands, assets, images, meshes, materials, class, at, rot, scale, Some((life, (scale, 0.0))))
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_mesh_fx(
    commands: &mut Commands,
    assets: &mut RuntimeAssets,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    class: &str,
    at: DVec3,
    rot: [i32; 3],
    size: f32,
) -> Option<Entity> {
    spawn_mesh_fx_ex(commands, assets, images, meshes, materials, class, at, rot, size, None)
}

#[allow(clippy::too_many_arguments)]
fn spawn_mesh_fx_ex(
    commands: &mut Commands,
    assets: &mut RuntimeAssets,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    class: &str,
    at: DVec3,
    rot: [i32; 3],
    size: f32,
    timed: Option<(f32, (f32, f32))>,
) -> Option<Entity> {
    let mut d = assets.classes.defaults(class);
    let Some(Value::Path(mesh)) = find(&d, "Mesh").cloned() else {
        return None;
    };
    // ShockRing.PostBeginPlay: big rings use FlatFXTex43.
    if class == "ShockRing" && size > 5.0 {
        d.retain(|p| p.name != "Skin");
        d.push(unreal_pkg::props::Property {
            name: "Skin".into(),
            index: 0,
            value: Value::Path("DeusExItems.Skins.FlatFXTex43".into()),
        });
    }
    let m = dx_level::actors::rotator_to_bevy_mat3(rot);
    let root = commands
        .spawn((
            Transform::from_translation(Vec3::from(ue_to_bevy(at.as_vec3().to_array())))
                .with_rotation(Quat::from_mat3(&Mat3::from_cols(
                    Vec3::from(m[0]),
                    Vec3::from(m[1]),
                    Vec3::from(m[2]),
                )))
                .with_scale(Vec3::splat(0.001)),
            Visibility::default(),
        ))
        .id();
    let map = |p: [f32; 3]| ue_to_bevy(p);
    let Some((_m, _parts, handles)) = crate::weapons::spawn_mesh_parts(
        &mesh, 1.0, &d, assets, images, meshes, materials, commands, root, &map, None,
    ) else {
        commands.entity(root).despawn();
        return None;
    };
    // Own additive materials so each effect fades on its own.
    let mut mats = Vec::new();
    for h in &handles {
        if let Some(mut mm) = meshes.get_mut(h) {
            let n = mm
                .attribute(Mesh::ATTRIBUTE_POSITION)
                .map(|a| a.len())
                .unwrap_or(0);
            mm.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0f32, 1.0, 1.0, 1.0]; n]);
        }
    }
    let tex = find(&d, "Skin").and_then(|v| {
        if let Value::Path(p) = v {
            Some(p.clone())
        } else {
            None
        }
    });
    let img = tex.as_deref().and_then(|p| image_for(assets, images, p));
    let mat = materials.add(StandardMaterial {
        base_color_texture: img,
        unlit: true,
        double_sided: true,
        cull_mode: None,
        alpha_mode: AlphaMode::Add,
        ..default()
    });
    mats.push(mat.clone());
    let life = match find(&d, "LifeSpan") {
        Some(Value::Float(l)) if *l > 0.0 && class == "WaterRing" => *l,
        _ => 0.5,
    };
    let (life, grow) = match timed {
        Some((l, g)) => (l, Some(g)),
        None => (life, None),
    };
    commands.entity(root).insert((
        MeshFx {
            t: 0.0,
            life,
            size,
            mats,
            grow,
        },
        FxMaterial(mat),
    ));
    Some(root)
}

/// Material to force onto an effect's mesh children (spawn_mesh_parts shares materials between users).
#[derive(Component)]
pub struct FxMaterial(Handle<StandardMaterial>);

/// Grow and fade ShockRings and SphereEffects.
pub fn mesh_fx_system(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(
        Entity,
        &mut MeshFx,
        &mut Transform,
        Option<&FxMaterial>,
        Option<&Children>,
    )>,
    mut kids: Query<&mut MeshMaterial3d<StandardMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let _prof = crate::prof::scope("explosion::mesh_fx_system");
    let dt = time.delta_secs();
    for (e, mut fx, mut tf, fm, ch) in &mut q {
        if let (Some(fm), Some(ch)) = (fm, ch) {
            for c in ch.iter() {
                if let Ok(mut m) = kids.get_mut(c) {
                    if m.0 != fm.0 {
                        m.0 = fm.0.clone();
                    }
                }
            }
        }
        fx.t += dt;
        let k = (fx.t / fx.life).min(1.0);
        if fx.t >= fx.life {
            commands.entity(e).despawn();
            continue;
        }
        let glow = match fx.grow {
            Some((start, rate)) => {
                tf.scale = Vec3::splat((start + rate * fx.t).max(0.001));
                1.0 - k
            }
            None => {
                tf.scale = Vec3::splat((fx.size * k).max(0.001));
                (1.0 - k) * 1.5
            }
        };
        for m in &fx.mats {
            if let Some(mut mm) = materials.get_mut(m) {
                mm.base_color = Color::linear_rgb(glow, glow, glow);
            }
        }
    }
}

#[derive(Resource, Default)]
pub struct Explosions(pub Vec<Boom>);

/// Places damaged by explosions this frame (grenades listen to chain-react).
#[derive(Resource, Default)]
pub struct BlastsThisFrame(pub Vec<(DVec3, f64, &'static str)>);

#[derive(Component)]
pub struct Exploding {
    boom: Boom,
    ring: u32,
    timer: f32,
}

#[derive(Component)]
pub struct Fireball {
    frames: Vec<Handle<StandardMaterial>>,
    speed: f32,
    t: f32,
    size: f32,
    sprite: Entity,
}

#[derive(Component)]
pub struct FlashOverlay;

/// Screen flash (DeusExPlayer.ClientFlash / ViewFlash): DesiredFlashScale and DesiredFlashFog decay at 2/s and
/// the shown values chase them at 10/s, so a flash swells in and fades out over about a second.
#[derive(Resource, Default)]
pub struct ScreenFlash {
    pub desired_scale: f32,
    pub desired_fog: [f32; 3],
    pub scale: f32,
    pub fog: [f32; 3],
}

impl ScreenFlash {
    /// ClientFlash(scale, fog) with fog already in 0..1 units (the script's vector x 0.001).
    pub fn add(&mut self, scale: f32, fog: [f32; 3]) {
        self.desired_scale += scale;
        for i in 0..3 {
            self.desired_fog[i] += fog[i];
        }
    }
}

pub fn spawn_flash_overlay(commands: &mut Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(Color::NONE),
        Pickable::IGNORE,
        GlobalZIndex(5),
        FlashOverlay,
    ));
}

fn frames_of(d: &[unreal_pkg::props::Property]) -> Vec<String> {
    let mut v: Vec<(u32, String)> = d
        .iter()
        .filter(|x| x.name == "frames")
        .filter_map(|x| match &x.value {
            Value::Path(s) => Some((x.index, s.clone())),
            _ => None,
        })
        .collect();
    v.sort_by_key(|x| x.0);
    v.into_iter().map(|x| x.1).collect()
}

/// Spawn a fireball sprite (AnimatedSprite: `frames` played at animSpeed, size = texture width x DrawScale).
pub fn spawn_fireball(
    commands: &mut Commands,
    assets: &mut RuntimeAssets,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    class: &str,
    at: DVec3,
) {
    let d = assets.classes.defaults(class);
    let mut paths = frames_of(&d);
    let n = match find(&d, "numFrames") {
        Some(Value::Int(v)) => *v as usize,
        _ => paths.len().max(1),
    };
    // Only the first frame is listed in the defaults; the rest follow it by number (FlatFXTex20, 21, ...).
    if paths.len() == 1 && n > 1 {
        let first = paths[0].clone();
        let digits: String = first
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if let Ok(base) = digits.parse::<u32>() {
            let stem = &first[..first.len() - digits.len()];
            paths = (0..n as u32)
                .map(|i| format!("{stem}{}", base + i))
                .collect();
        }
    }
    let speed = match find(&d, "animSpeed") {
        Some(Value::Float(v)) => *v,
        _ => 0.075,
    };
    let mut frames = Vec::new();
    let mut size = 128.0f32;
    for (i, p) in paths.iter().enumerate() {
        if i == 0 {
            size = assets
                .lib
                .width(p).map(|w| w as f32)
                .unwrap_or(128.0);
        }
        let img = image_for(assets, images, p);
        if img.is_none() {
            continue;
        }
        frames.push(materials.add(StandardMaterial {
            base_color_texture: img,
            unlit: true,
            double_sided: true,
            cull_mode: None,
            alpha_mode: AlphaMode::Add,
            ..default()
        }));
    }
    if frames.is_empty() {
        return;
    }
    let scale = match find(&d, "DrawScale") {
        Some(Value::Float(v)) => *v,
        _ => 1.0,
    };
    let quad = meshes.add(Rectangle::new(1.0, 1.0));
    let root = commands
        .spawn((
            Transform::from_translation(Vec3::from(ue_to_bevy(at.as_vec3().to_array()))),
            Visibility::default(),
        ))
        .id();
    let sprite = commands
        .spawn((
            Mesh3d(quad),
            MeshMaterial3d(frames[0].clone()),
            Transform::from_scale(Vec3::splat(size * scale * dx_level::METERS_PER_UNIT)),
        ))
        .id();
    commands.entity(root).add_child(sprite);
    commands.entity(root).insert(Fireball {
        frames,
        speed,
        t: 0.0,
        size: size * scale,
        sprite,
    });
}

#[allow(clippy::too_many_arguments)]
pub fn explosion_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    blockers: Res<Blockers>,
    mv: Res<Movers>,
    player: Res<Player>,
    mut ph: ResMut<PlayerHealth>,
    mut booms: ResMut<Explosions>,
    mut blasts: ResMut<BlastsThisFrame>,
    (mut pawn_hits, mut deco_hits, mut noises, mut flash): (
        ResMut<crate::damage::PawnHits>,
        ResMut<crate::destruct::DecoHits>,
        ResMut<crate::stealth::Noises>,
        ResMut<ScreenFlash>,
    ),
    (mut assets, mut images, mut meshes, mut materials): (
        ResMut<RuntimeAssets>,
        ResMut<Assets<Image>>,
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
    ),
    mut active: Query<(Entity, &mut Exploding)>,
    cam: Single<&Transform, (With<FlyCam>, Without<SkyCam>)>,
) {
    let _prof = crate::prof::scope("explosion::explosion_system");
    blasts.0.clear();
    let lvl = &level.0;
    for b in booms.0.drain(..) {
        if b.fireball {
            crate::fx::explosion(b.pos, b.radius as f32);
        }
        // The pad feels blasts near the player (ours): strength by size, falling off over four blast radii.
        if b.fireball || b.rings || b.damage_type == "EMP" {
            let d = (b.pos - player.body.location).length() as f32;
            let s = (b.radius as f32 / 256.0).clamp(0.3, 1.0) * (1.0 - d / (b.radius as f32 * 4.0).max(400.0)).max(0.0);
            if s > 0.05 {
                crate::dualsense::event(crate::dualsense::Haptic::Boom(s));
            }
        }
        // ExplosionLight: size 2 / 4 / 8 by blast radius (EMP: 8, bluish LE_Shell), shrinking a step every 0.1 s.
        if b.damage_type == "EMP" {
            crate::dynlight::explosion(b.pos, 8.0, 128, 96);
        } else if b.fireball || b.rings {
            let size = if b.radius < 128.0 {
                2.0
            } else if b.radius < 256.0 {
                4.0
            } else {
                8.0
            };
            crate::dynlight::explosion(b.pos, size, 16, 192);
        }
        // Fireball and flash.
        if b.fireball {
            let class = if b.radius < 128.0 {
                "ExplosionSmall"
            } else if b.radius < 256.0 {
                "ExplosionMedium"
            } else {
                "ExplosionLarge"
            };
            match b.fx {
                // ThrownProjectile: blastRadius / 36 fireballs (90% each), small or medium, spread over the blast.
                2 => {
                    let n = (b.radius / 36.0) as usize;
                    let mut seed = (b.pos.x.to_bits() ^ b.pos.y.to_bits()) | 1;
                    let mut rnd = || {
                        seed = seed
                            .wrapping_mul(6364136223846793005)
                            .wrapping_add(1442695040888963407);
                        ((seed >> 33) as f64) / (1u64 << 31) as f64
                    };
                    for _ in 0..n.max(1) {
                        if rnd() < 0.9 {
                            let at = b.pos
                                + DVec3::new(
                                    rnd() * b.radius - b.radius * 0.5,
                                    rnd() * b.radius - b.radius * 0.5,
                                    0.0,
                                );
                            let c = if rnd() < 0.5 {
                                "ExplosionSmall"
                            } else {
                                "ExplosionMedium"
                            };
                            spawn_fireball(
                                &mut commands,
                                &mut assets,
                                &mut images,
                                &mut meshes,
                                &mut materials,
                                c,
                                at,
                            );
                        }
                    }
                }
                3 => spawn_fireball(
                    &mut commands,
                    &mut assets,
                    &mut images,
                    &mut meshes,
                    &mut materials,
                    "ExplosionSmall",
                    b.pos,
                ),
                _ => spawn_fireball(
                    &mut commands,
                    &mut assets,
                    &mut images,
                    &mut meshes,
                    &mut materials,
                    class,
                    b.pos,
                ),
            }
        }
        // DeusExProjectile: three shock rings at right angles; EMPGrenade: the light sphere.
        if b.fx == 1 && b.fireball {
            for rot in [[16384, 0, 0], [0, 0, 0], [0, 16384, 0]] {
                spawn_mesh_fx(
                    &mut commands,
                    &mut assets,
                    &mut images,
                    &mut meshes,
                    &mut materials,
                    "ShockRing",
                    b.pos,
                    rot,
                    (b.radius / 32.0) as f32,
                );
            }
        }
        if b.fx == 3 {
            spawn_mesh_fx(
                &mut commands,
                &mut assets,
                &mut images,
                &mut meshes,
                &mut materials,
                "SphereEffect",
                b.pos,
                [0, 0, 0],
                3.0 * (b.radius / 32.0) as f32,
            );
        }
        let dist = player.body.location.distance(b.pos);
        let amount = if dist < 1.0 {
            10.0
        } else {
            2.0 * (b.radius / dist).clamp(0.0, 4.0)
        } as f32;
        // ThrownProjectile.Explode: ClientFlash(dist, fog) with fog EMP (0,200,1000), tear gas (0,1000,100),
        // everything else (1000,1000,900), all x 0.001.
        let fog = match b.damage_type {
            "EMP" => [0.0, 0.2, 1.0],
            "TearGas" => [0.0, 1.0, 0.1],
            _ => [1.0, 1.0, 0.9],
        };
        flash.add(amount, fog);
        // Loud noise (LoudNoise 2.0 out to blastRadius x 24, WeaponFire to x 5).
        let who = Some(b.from.map(Target::Pawn).unwrap_or(Target::Player));
        noises.0.push(crate::stealth::Noise {
            pos: b.pos,
            volume: 2.0,
            radius: (b.radius * 24.0) as f32,
            maker: who,
            instigator: who,
            kind: crate::stealth::NoiseKind::Loud,
        });
        noises.0.push(crate::stealth::Noise {
            pos: b.pos,
            volume: 2.0,
            radius: (b.radius * 5.0) as f32,
            maker: who,
            instigator: who,
            kind: crate::stealth::NoiseKind::WeaponFire,
        });
        commands.spawn(Exploding {
            boom: b,
            ring: 0,
            timer: 0.0,
        });
    }
    let dt = time.delta_secs();
    for (e, mut x) in &mut active {
        x.timer -= dt;
        if x.timer > 0.0 {
            continue;
        }
        x.timer = 0.05;
        x.ring += 1;
        let b = x.boom.clone();
        let (amount, radius) = if b.rings {
            (2.0 * b.damage / 5.0, b.radius * x.ring as f64 / 5.0)
        } else {
            (b.damage, b.radius)
        };
        blasts.0.push((b.pos, radius, b.damage_type));
        if amount <= 0.0 {
            // Effects only (a door bursting): nothing to hurt.
            commands.entity(e).despawn();
            continue;
        }
        crate::movers::blast_ring(b.pos, radius, amount, b.damage_type, if b.rings { b.damage } else { 0.0 });
        // Cameras, turrets and lasers: EMP confuses them; lasers (not blockers) take the blast here.
        crate::security::blast(b.pos, radius, amount, b.damage_type);
        // Birds aren't blockers: they take the ring too (EMP and gas don't hurt them).
        if !matches!(b.damage_type, "EMP" | "NanoVirus") {
            crate::critters::blast_birds(b.pos, radius, amount);
        }
        // HurtRadius reaches victims through VisibleCollidingActors, whose line check is FastTrace: world geometry
        // and movers only. Other actors (a stack of crates, a pawn in front) never shield anything from a blast.
        let world = World {
            level: lvl,
            blockers: &[],
            ignore: None,
            pushable: None,
            movers: Some(&mv.rt),
        };
        let sees =
            |to: DVec3, _me: Option<usize>| world.trace(b.pos, to, DVec3::ZERO).fraction >= 0.999;
        // HurtRadius on actors.
        for (i, c) in blockers.list.iter().enumerate() {
            if c.radius <= 0.0 {
                continue;
            }
            let d = c.center.distance(b.pos).max(1.0);
            if d > radius + c.radius || !sees(c.center, Some(i)) {
                continue;
            }
            let scale = (1.0 - ((d - c.radius) / radius).max(0.0)) as f32;
            if scale <= 0.0 {
                continue;
            }
            let dir = (c.center - b.pos) / d;
            let hit_loc = c.center - dir * (0.5 * (c.height + c.radius));
            let a = &lvl.actors[c.actor];
            if find(&a.props, "GroundSpeed").is_some() {
                pawn_hits.0.push(crate::damage::PawnHit {
                    blocker: i,
                    damage: amount * scale,
                    damage_type: b.damage_type,
                    hit_loc,
                    attacker: b.from,
                    attacker_pos: b.pos,
                });
            } else {
                deco_hits.0.push(crate::destruct::DecoHit {
                    blocker: i,
                    damage: amount * scale,
                    damage_type: b.damage_type,
                });
            }
        }
        // ...and the player.
        let pc = player.body.location;
        let d = pc.distance(b.pos).max(1.0);
        if !ph.dead && d < radius + player.params.radius && sees(pc, None) {
            let scale = (1.0 - ((d - player.params.radius) / radius).max(0.0)) as f32;
            let mut dmg = amount * scale;
            // GMDX Blast Padding: the player's own blast through active ballistic armour does PerkValue (25%) of
            // its damage, and the armour loses Damage x 4 x Enviro skill charge for it.
            if b.from.is_none() && ph.armor && b.damage_type == "Exploded" {
                if let Some(v) = crate::perks::value_if("PerkBlastPadding") {
                    crate::charged::drain("BallisticArmor", dmg * 4.0 * crate::abilities::skill("SkillEnviro"));
                    dmg *= v;
                }
            }
            let dir = (pc - b.pos) / d;
            let at = pc - dir * (0.5 * (player.params.height + player.params.radius));
            let f = cam.forward().as_vec3();
            let yaw = (f.z as f64).atan2(f.x as f64);
            ph.damage(dmg, b.damage_type, crate::ai::player_offset(pc, yaw, at));
        }
        if !b.rings || x.ring >= 5 {
            commands.entity(e).despawn();
        }
    }
}

/// Animate fireballs (camera-facing, one frame per animSpeed) and fade the screen flash.
#[allow(clippy::too_many_arguments)]
pub fn fireball_system(
    mut commands: Commands,
    time: Res<Time>,
    cam: Single<&Transform, (With<FlyCam>, Without<SkyCam>, Without<Fireball>)>,
    mut q: Query<(Entity, &mut Fireball)>,
    mut sprites: Query<
        (&mut Transform, &mut MeshMaterial3d<StandardMaterial>),
        (Without<FlyCam>, Without<Fireball>),
    >,
    mut flash: ResMut<ScreenFlash>,
    mut overlay: Single<&mut BackgroundColor, With<FlashOverlay>>,
) {
    let _prof = crate::prof::scope("explosion::fireball_system");
    let dt = time.delta_secs();
    for (e, mut f) in &mut q {
        f.t += dt;
        let i = (f.t / f.speed.max(0.01)) as usize;
        if i >= f.frames.len() {
            commands.entity(e).despawn();
            continue;
        }
        if let Ok((mut st, mut m)) = sprites.get_mut(f.sprite) {
            st.rotation = cam.rotation;
            st.scale = Vec3::splat(f.size * dx_level::METERS_PER_UNIT);
            if m.0 != f.frames[i] {
                m.0 = f.frames[i].clone();
            }
        }
    }
    // ViewFlash.
    let d = dt.min(0.1);
    let goal_scale = flash.desired_scale;
    let goal_fog = flash.desired_fog;
    let k = (1.0 - 2.0 * d).max(0.0);
    flash.desired_scale *= k;
    for i in 0..3 {
        flash.desired_fog[i] *= k;
    }
    flash.scale += (goal_scale - flash.scale) * (10.0 * d).min(1.0);
    for i in 0..3 {
        flash.fog[i] += (goal_fog[i] - flash.fog[i]) * (10.0 * d).min(1.0);
    }
    // The original brightens the scene (x (1 + scale)) and adds the fog colour. A see-through overlay stands in:
    // white flashes may blind, coloured ones (gas, EMP) only tint so the scene stays visible.
    let f = flash.fog;
    let fmax = f[0].max(f[1]).max(f[2]);
    let white = f[0] > 0.5 && f[1] > 0.5 && f[2] > 0.5;
    let strength = (fmax * 0.25 + flash.scale * 0.12).min(if white { 0.9 } else { 0.35 });
    if fmax > 0.01 {
        overlay.0 = Color::srgba(f[0] / fmax, f[1] / fmax, f[2] / fmax, strength);
    } else {
        overlay.0 = Color::NONE;
    }
    // FadeViewTrigger: InstantFlash / InstantFog take the screen to a colour.
    if let Some((c, a)) = crate::triggers::FADE.lock().ok().and_then(|f| *f) {
        if a > 0.0 {
            overlay.0 = Color::srgba(c[0], c[1], c[2], a);
        }
    }
    let _ = bevy_to_ue;
}
