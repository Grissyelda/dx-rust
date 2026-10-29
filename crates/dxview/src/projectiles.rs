//! Projectiles fired by weapons that aren't instant-hit (DeusExProjectile): darts for now. They fly straight
//! (PHYS_Projectile) at their `speed`, hit pawns, decorations or the player with `Damage` of their `DamageType`,
//! and darts stick in walls (bStickToWall) until their LifeSpan runs out.

use crate::actors::{Blockers, World};
use crate::ai::PlayerHealth;
use crate::movers::Movers;
use crate::weapons::{spawn_mesh_parts, RuntimeAssets};
use crate::{LevelRes, Player};
use bevy::prelude::*;
use dx_level::actors::{light_part, rotator_to_bevy_mat3};
use dx_level::collision::{trace_cylinder, Cylinder};
use dx_level::ue_to_bevy;
use glam::DVec3;
use imsim_core::movement::CollisionWorld;
use unreal_pkg::props::{find, Value};

/// A projectile to launch this frame.
pub struct Launch {
    pub class: String,
    pub pos: DVec3,
    pub dir: DVec3,
    /// Who fired it (None = the player): it can't hit its owner.
    pub from: Option<usize>,
    /// Damage multiplier (player skill: 1 - 2 * skill).
    pub mult: f32,
    /// Overrides for gas left by grenades: LifeSpan, a fixed DrawScale, and no initial speed (it drifts).
    pub life: Option<f32>,
    pub scale: Option<f32>,
    pub still: bool,
    /// Locked-on target (blocker) for tracking projectiles (GEP rockets).
    pub track: Option<usize>,
    /// Tracking toward a fixed spot instead (Mission06: the chopper's rockets at the blast doors).
    pub track_at: Option<DVec3>,
    /// Launch speed instead of the class's (ProjectileGenerator: EjectSpeed).
    pub speed: Option<f64>,
    /// Spawned with no owner (ProjectileGenerator): it can hurt the player.
    pub ownerless: bool,
}

/// Lingering gas clouds to spawn (SpawnTearGas): (where, DrawScale, LifeSpan).
#[derive(Resource, Default)]
pub struct GasClouds(pub Vec<(DVec3, f32, f32, Option<usize>)>);

#[derive(Resource, Default)]
pub struct Launches(pub Vec<Launch>);

#[derive(Component)]
pub struct Projectile {
    /// The projectile class (AugDefense ignores Tracer / GreaselSpit / GraySpit).
    pub class: String,
    /// A Tracer: flies until it touches anything, then just goes (DeusExProjectile: "Don't even attempt damage with
    /// a tracer").
    pub tracer: bool,
    pub pos: DVec3,
    pub vel: DVec3,
    pub damage: f32,
    pub damage_type: &'static str,
    pub from: Option<usize>,
    /// Spawned with no Owner (grenade gas, SpawnTearGas): it hurts the thrower too; `from` is only the Instigator.
    pub ownerless: bool,
    pub life: f32,
    pub stuck: bool,
    pub stick: bool,
    #[allow(dead_code)]
    pub parts: Vec<(Handle<Mesh>, dx_level::actors::MeshPart)>,
    /// Gas clouds (Cloud: TearGas, PoisonGas): a growing, fading sprite that hurts everything in `radius` it can see.
    pub cloud: Option<CloudFx>,
    /// bExplodes (rockets): blastRadius. Any impact (or running out of life) explodes instead of a direct hit.
    pub explodes: Option<f64>,
    /// bTracking with a Target: steer toward this blocker at RotationRate (rad/s) while within MaxRange.
    pub track: Option<usize>,
    pub track_at: Option<DVec3>,
    /// Sprite projectiles that aren't clouds (the flamethrower's Fireball): grows like a cloud, glow 1/(1+t)^3.
    pub sprite: Option<SpriteFx>,
    pub turn_rate: f64,
    pub max_range: f64,
    /// ImpactSound (played by Explode, 2.0 volume, radius max(blastRadius x 4, 1024)).
    pub impact_snd: Option<String>,
    /// AmbientSound while flying (the rocket's RocketLoop): sound, volume, radius, pitch.
    pub ambient_snd: Option<(String, f32, f32, f32)>,
    /// A projectile that gives off light (Fireball, PlasmaBolt, DartFlare...): its LightHue/Saturation/Brightness/Radius.
    pub light: Option<crate::dynlight::Spec>,
}

impl Projectile {
    fn impact(&self, at: DVec3) {
        if let Some(s) = &self.impact_snd {
            let r = (self.explodes.unwrap_or(0.0) * 4.0).max(1024.0) as f32;
            crate::sound::play(s, at, 2.0, r, 1.0);
        }
    }
}

pub struct SpriteFx {
    pub sprite: Entity,
    pub material: Handle<StandardMaterial>,
    pub min_scale: f32,
    pub max_scale: f32,
    pub tex_size: f32,
    pub age: f32,
}

pub struct CloudFx {
    /// Pawns this cloud has already reached (see the touch rule in projectile_system).
    pub touched: Vec<usize>,
    pub radius: f64,
    pub age: f32,
    pub min_scale: f32,
    pub max_scale: f32,
    pub interval: f32,
    pub next_damage: f32,
    /// Sprite size per unit of DrawScale (texture width in Unreal units).
    pub tex_size: f32,
    pub sprite: Entity,
    pub material: Handle<StandardMaterial>,
    /// A set DrawScale (grenade gas) instead of growing.
    pub fixed: Option<f32>,
}

fn damage_type_of(n: &str) -> &'static str {
    match n.to_ascii_lowercase().as_str() {
        "poison" => "Poison",
        "stunned" => "Stunned",
        "exploded" => "Exploded",
        "teargas" => "TearGas",
        "poisongas" => "PoisonGas",
        "burned" => "Burned",
        "flamed" => "Flamed",
        "emp" => "EMP",
        "sabot" => "Sabot",
        _ => "Shot",
    }
}

fn yaw_pitch(dir: DVec3) -> [i32; 3] {
    let yaw = dir.y.atan2(dir.x);
    let pitch = dir.z.atan2(dir.truncate().length());
    let u = |r: f64| (r / std::f64::consts::TAU * 65536.0) as i32;
    [u(pitch), u(yaw), 0]
}

/// Spawn the queued projectiles (class defaults: speed, Damage, DamageType, Mesh, DrawScale, LifeSpan).
#[allow(clippy::too_many_arguments)]
pub fn launch_system(
    mut commands: Commands,
    mut launches: ResMut<Launches>,
    mut clouds: ResMut<GasClouds>,
    mut assets: ResMut<RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    level: Res<LevelRes>,
) {
    let _prof = crate::prof::scope("projectiles::launch_system");
    let gas: Vec<Launch> = clouds
        .0
        .drain(..)
        .map(|(pos, scale, life, from)| Launch {
            class: "TearGas".into(),
            pos,
            dir: DVec3::X,
            from,
            mult: 1.0,
            life: Some(life),
            scale: Some(scale),
            still: true,
            track: None,
            track_at: None,
            speed: None,
            ownerless: false,
        })
        .collect();
    for l in launches.0.drain(..).chain(gas) {
        let mut d = assets.classes.defaults(&l.class);
        // Augmentique: rockets from the player's skinned GEP gun.
        if l.from.is_none() && l.class.starts_with("Rocket") {
            crate::augmentique::apply_projectile_skin(&mut d, "WeaponGEPGun");
        }
        let f = |n: &str, dflt: f32| match find(&d, n) {
            Some(Value::Float(v)) => *v,
            Some(Value::Int(v)) => *v as f32,
            Some(Value::Byte(v)) => *v as f32,
            _ => dflt,
        };
        let speed = if l.still {
            0.0
        } else {
            l.speed.unwrap_or(f("speed", 2000.0) as f64)
        };
        let damage = f("Damage", 10.0) * l.mult;
        let damage_type = match find(&d, "DamageType") {
            Some(Value::Name(n)) => damage_type_of(n),
            _ => "Shot",
        };
        let stick = !matches!(find(&d, "bStickToWall"), Some(Value::Bool(false)))
            && find(&d, "cloudRadius").is_none();
        let rot = yaw_pitch(l.dir);
        let m = rotator_to_bevy_mat3(rot);
        let root = commands
            .spawn((
                Transform::from_translation(Vec3::from(ue_to_bevy(l.pos.as_vec3().to_array())))
                    .with_rotation(Quat::from_mat3(&Mat3::from_cols(
                        Vec3::from(m[0]),
                        Vec3::from(m[1]),
                        Vec3::from(m[2]),
                    ))),
                Visibility::default(),
            ))
            .id();
        let mut parts = Vec::new();
        if let Some(Value::Path(mesh)) = find(&d, "Mesh") {
            let mesh = mesh.clone();
            let map = |p: [f32; 3]| ue_to_bevy(p);
            if let Some((_m, ps, handles)) = spawn_mesh_parts(
                &mesh,
                f("DrawScale", 1.0),
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
                let at = l.pos.as_vec3().to_array();
                let unlit = matches!(find(&d, "bUnlit"), Some(Value::Bool(true)));
                for (h, part) in handles.into_iter().zip(ps) {
                    if let Some(mut mm) = meshes.get_mut(&h) {
                        let colors = if unlit {
                            // bUnlit (Tracer): full bright.
                            let n = mm.attribute(Mesh::ATTRIBUTE_POSITION).map(|a| a.len()).unwrap_or(0);
                            vec![[1.0f32, 1.0, 1.0, 1.0]; n]
                        } else {
                            light_part(&part, at, [0, 0, 0], 1.0, &level.0.lights)
                        };
                        mm.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
                    }
                    parts.push((h, part));
                }
            }
        }
        // Sprite projectiles (gas clouds): a camera-facing quad with the Texture, additive.
        let mut cloud = None;
        let mut sprite_fx = None;
        if matches!(find(&d, "DrawType"), Some(Value::Byte(1))) {
            if let Some(Value::Path(tex)) = find(&d, "Texture") {
                let tex = tex.clone();
                // Fire-texture sprites (the flamethrower's flame_b) are drawn as a soft round fireball: the texture is
                // faded out towards its edges so the quad never reads as a square.
                let img = if find(&d, "cloudRadius").is_none()
                    && !tex.to_ascii_lowercase().ends_with("flame_b")
                {
                    round_fire_image(&mut assets, &mut images, &tex)
                } else {
                    None
                }
                .or_else(|| crate::weapons::image_for(&mut assets, &mut images, &tex));
                let tex_size = assets
                    .lib
                    .load_image(&tex)
                    .or_else(|| assets.lib.load_wet(&tex))
                    .map(|t| t.width as f32)
                    .or_else(|| assets.lib.load_fire(&tex).map(|f| f.width as f32))
                    .unwrap_or(64.0);
                let material = materials.add(StandardMaterial {
                    base_color_texture: img,
                    unlit: true,
                    double_sided: true,
                    cull_mode: None,
                    alpha_mode: AlphaMode::Add,
                    ..default()
                });
                let quad = meshes.add(Rectangle::new(1.0, 1.0));
                let sprite = commands
                    .spawn((
                        Mesh3d(quad),
                        MeshMaterial3d(material.clone()),
                        Transform::from_scale(Vec3::splat(0.001)),
                    ))
                    .id();
                commands.entity(root).add_child(sprite);
                if find(&d, "cloudRadius").is_some() {
                    let interval = f("damageInterval", 1.0);
                    cloud = Some(CloudFx {
                        touched: Vec::new(),
                        radius: f("cloudRadius", 128.0) as f64,
                        age: 0.0,
                        min_scale: f("MinDrawScale", 0.05),
                        max_scale: f("maxDrawScale", 2.0),
                        interval,
                        next_damage: interval,
                        tex_size,
                        sprite,
                        material,
                        fixed: l.scale,
                    });
                } else {
                    sprite_fx = Some(SpriteFx {
                        sprite,
                        material,
                        min_scale: f("MinDrawScale", 0.05),
                        max_scale: f("maxDrawScale", 2.5),
                        tex_size,
                        age: 0.0,
                    });
                }
            }
        }
        // ProjectileFly.BeginState: SpawnSound.
        if let Some(snd) = crate::sound::prop(&d, "SpawnSound") {
            crate::sound::play(&snd, l.pos, 1.0, 0.0, 1.0);
        }
        let ambient_snd = crate::sound::prop(&d, "AmbientSound").map(|snd| {
            (
                snd,
                1.4 * f("SoundVolume", 128.0) / 255.0,
                25.0 * (f("SoundRadius", 32.0) + 1.0),
                f("SoundPitch", 64.0) / 64.0,
            )
        });
        let byte = |n: &str, dv: u8| match find(&d, n) {
            Some(Value::Byte(b)) => *b,
            _ => dv,
        };
        let light = (byte("LightType", 0) != 0).then(|| {
            crate::dynlight::Spec::new(
                byte("LightHue", 0),
                byte("LightSaturation", 255),
                byte("LightBrightness", 64),
                byte("LightRadius", 64) as f32,
            )
        });
        let short = l.class.rsplit('.').next().unwrap_or(&l.class).to_string();
        let tracer = short == "Tracer" || short == "SniperTracer";
        commands.entity(root).insert(Projectile {
            tracer,
            class: short,
            light,
            impact_snd: crate::sound::prop(&d, "ImpactSound"),
            ambient_snd,
            cloud,
            pos: l.pos,
            vel: l.dir.normalize_or_zero() * speed,
            damage: if tracer { 0.0 } else { damage },
            damage_type,
            from: l.from,
            ownerless: l.still || l.ownerless,
            life: l.life.unwrap_or(f("LifeSpan", 60.0)).max(1.0),
            stuck: false,
            stick: stick
                && !tracer
                && matches!(find(&d, "bStickToWall"), Some(Value::Bool(true)))
                && !matches!(find(&d, "bExplodes"), Some(Value::Bool(true))),
            sprite: sprite_fx,
            parts,
            explodes: if matches!(find(&d, "bExplodes"), Some(Value::Bool(true))) {
                Some(f("blastRadius", 128.0) as f64)
            } else {
                None
            },
            track: l
                .track
                .filter(|_| matches!(find(&d, "bTracking"), Some(Value::Bool(true)))),
            track_at: l.track_at.filter(|_| matches!(find(&d, "bTracking"), Some(Value::Bool(true)))),
            turn_rate: match find(&d, "RotationRate") {
                Some(Value::Rotator(r)) => r[1].max(r[0]) as f64 / 65536.0 * std::f64::consts::TAU,
                _ => std::f64::consts::PI,
            },
            max_range: f("maxRange", 24000.0) as f64,
        });
    }
}

/// Fly, hit, stick.
#[allow(clippy::too_many_arguments)]
pub fn projectile_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    blockers: Res<Blockers>,
    mv: Res<Movers>,
    player: Res<Player>,
    mut ph: ResMut<PlayerHealth>,
    mut q: Query<(Entity, &mut Projectile, &mut Transform), Without<crate::FlyCam>>,
    (mut pawn_hits, mut deco_hits): (
        ResMut<crate::damage::PawnHits>,
        ResMut<crate::destruct::DecoHits>,
    ),
    (cam, mut sprites, mut materials): (
        Single<&Transform, (With<crate::FlyCam>, Without<crate::SkyCam>)>,
        Query<&mut Transform, (Without<Projectile>, Without<crate::FlyCam>)>,
        ResMut<Assets<StandardMaterial>>,
    ),
    mut booms: ResMut<crate::explosion::Explosions>,
) {
    let _prof = crate::prof::scope("projectiles::projectile_system");
    let dt = time.delta_secs().min(0.05);
    let lvl = &level.0;
    let player_cyl = Cylinder {
        center: player.body.location,
        radius: player.params.radius,
        height: player.params.height,
        actor: usize::MAX,
    };
    let player_yaw = {
        let f = cam.forward().as_vec3();
        (f.z as f64).atan2(f.x as f64)
    };
    for (e, mut p, mut tf) in &mut q {
        p.life -= dt;
        let p = &mut *p;
        let (life, pos, from, damage, damage_type) =
            (p.life, p.pos, p.from, p.damage, p.damage_type);
        if let Some(fx) = p.sprite.as_mut() {
            // Fireball.Tick: DrawScale = (-s/(1+t)^2 + s + 1) * MinDrawScale, ScaleGlow = 1 / (1+t)^3.
            fx.age += dt;
            let sm = if fx.min_scale > 0.0 {
                fx.max_scale / fx.min_scale
            } else {
                1.0
            };
            let v = 1.0 + fx.age;
            let scale = (-sm / (v * v) + sm + 1.0) * fx.min_scale;
            if let Ok(mut st) = sprites.get_mut(fx.sprite) {
                st.scale =
                    Vec3::splat((scale * fx.tex_size * dx_level::METERS_PER_UNIT).max(0.001));
                st.rotation = cam.rotation;
            }
            tf.rotation = Quat::IDENTITY;
            let glow = 1.0 / (v * v * v);
            if let Some(mut m) = materials.get_mut(&fx.material) {
                m.base_color = Color::linear_rgb(glow, glow, glow);
            }
        }
        if let Some(c) = p.cloud.as_mut() {
            // Cloud.Tick: grow (DrawScale = (-s/(1+t)^2 + s + 1) * MinDrawScale, s = Max/Min) and fade with the
            // time left (ScaleGlow = LifeSpan / 2); Cloud.Timer: every damageInterval, hurt what's in cloudRadius.
            c.age += dt;
            let sm = if c.min_scale > 0.0 {
                c.max_scale / c.min_scale
            } else {
                1.0
            };
            let v = 1.0 + c.age;
            let scale = c.fixed.unwrap_or((-sm / (v * v) + sm + 1.0) * c.min_scale);
            let size = scale * c.tex_size * dx_level::METERS_PER_UNIT;
            if let Ok(mut st) = sprites.get_mut(c.sprite) {
                st.scale = Vec3::splat(size.max(0.001));
                // Face the camera (the root has no rotation of its own for clouds).
                st.rotation = cam.rotation;
            }
            tf.rotation = Quat::IDENTITY;
            let glow = (life * 0.5).clamp(0.0, 1.0);
            if let Some(mut m) = materials.get_mut(&c.material) {
                m.base_color = Color::srgb(glow, glow, glow);
            }
            // Deviation: in the original the first Timer only comes as a pepper puff expires, 300 units out, so a
            // target right in front of the nozzle is never caught. A pawn the cloud reaches is gassed once at once.
            if from.is_none() || c.interval < 1.5 {
                for (b, cyl) in blockers.list.iter().enumerate() {
                    if cyl.radius <= 0.0
                        || (Some(b) == from && !p.ownerless)
                        || c.touched.contains(&b)
                        || cyl.center.distance(pos) >= c.radius * 0.5 + cyl.radius
                    {
                        continue;
                    }
                    if find(&lvl.actors[cyl.actor].props, "GroundSpeed").is_none()
                        || lvl.collision.trace(pos, cyl.center, DVec3::ZERO).fraction < 0.999
                    {
                        continue;
                    }
                    c.touched.push(b);
                    pawn_hits.0.push(crate::damage::PawnHit {
                        blocker: b,
                        damage,
                        damage_type,
                        hit_loc: cyl.center,
                        attacker: from,
                        attacker_pos: pos,
                    });
                }
            }
            c.next_damage -= dt;
            // The damage tick lands as the cloud expires too (LifeSpan == damageInterval for pepper spray).
            if c.next_damage <= 0.0 || life <= 0.0 {
                c.next_damage = c.interval;
                let center = pos;
                let r = c.radius;
                let sees =
                    |to: DVec3| lvl.collision.trace(center, to, DVec3::ZERO).fraction >= 0.999;
                for (b, cyl) in blockers.list.iter().enumerate() {
                    if cyl.radius <= 0.0
                        || (Some(b) == from && !p.ownerless)
                        || cyl.center.distance(center) >= r
                        || !sees(cyl.center)
                    {
                        continue;
                    }
                    let a = &lvl.actors[cyl.actor];
                    if find(&a.props, "GroundSpeed").is_some() {
                        pawn_hits.0.push(crate::damage::PawnHit {
                            blocker: b,
                            damage,
                            damage_type,
                            hit_loc: cyl.center,
                            attacker: from,
                            attacker_pos: center,
                        });
                    }
                }
                // Cloud.Timer: everything in reach but the cloud's Owner (pepper spray's sprayer; grenade gas has none).
                if (from.is_some() || p.ownerless)
                    && !ph.dead
                    && player.body.location.distance(center) < r
                    && sees(player.body.location)
                {
                    ph.gas(damage_type, damage);
                }
            }
        }
        if p.life <= 0.0 {
            if let Some(r) = p.explodes.filter(|_| true) {
                p.impact(p.pos);
                booms.0.push(crate::explosion::Boom {
                    pos: p.pos,
                    damage: p.damage,
                    radius: r,
                    damage_type: p.damage_type,
                    from: p.from,
                    rings: true,
                    fireball: true,
                    fx: 1,
                });
            }
            commands.entity(e).despawn();
            continue;
        }
        if p.stuck {
            continue;
        }
        if let Some(spec) = p.light {
            crate::dynlight::hold(e.to_bits() | (1 << 60), p.pos, spec, 0.15);
        }
        if let Some((snd, vol, radius, pitch)) = &p.ambient_snd {
            crate::sound::hold(
                e.to_bits() | (1 << 62),
                snd,
                Some(p.pos),
                *vol,
                *radius,
                *pitch,
            );
        }
        // DeusExProjectile.Tick, bTracking: turn toward the target (RotationRate) and fly the way we face; out of
        // MaxRange the lock is lost.
        let goal = match (p.track, p.track_at) {
            (Some(b), _) => blockers.list.get(b).filter(|c| c.radius > 0.0).map(|c| c.center),
            (None, Some(at)) => Some(at),
            _ => None,
        };
        if p.track.is_some() || p.track_at.is_some() {
            match goal {
                Some(center) if center.distance(p.pos) <= p.max_range => {
                    let speed = p.vel.length();
                    let cur = p.vel.normalize_or_zero();
                    let want = (center - p.pos).normalize_or_zero();
                    let ang = cur.dot(want).clamp(-1.0, 1.0).acos();
                    let max = p.turn_rate * dt as f64;
                    let dir = if ang <= max || ang < 1e-6 {
                        want
                    } else {
                        let axis = cur.cross(want).normalize_or_zero();
                        if axis.length_squared() < 0.5 {
                            cur
                        } else {
                            glam::DQuat::from_axis_angle(axis, max) * cur
                        }
                    };
                    p.vel = dir * speed;
                    tf.rotation = Quat::from_mat3(&{
                        let m = rotator_to_bevy_mat3(yaw_pitch(dir));
                        Mat3::from_cols(Vec3::from(m[0]), Vec3::from(m[1]), Vec3::from(m[2]))
                    });
                }
                _ => {
                    p.track = None;
                    p.track_at = None;
                }
            }
        }
        let start = p.pos;
        let end = start + p.vel * dt as f64;
        let world = World {
            level: lvl,
            blockers: &blockers.list,
            ignore: p.from,
            pushable: None,
            movers: Some(&mv.rt),
        };
        if p.cloud.is_some() {
            // Cloud.HitWall stops it; it touches nothing else.
            let h = lvl.collision.trace(start, end, DVec3::ZERO);
            if h.fraction < 1.0 {
                p.pos = start + (end - start) * (h.fraction - 0.01).max(0.0);
                p.vel = DVec3::ZERO;
            } else {
                p.pos = end;
            }
            tf.translation = Vec3::from(ue_to_bevy(p.pos.as_vec3().to_array()));
            continue;
        }
        if p.tracer {
            let h = world.trace(start, end, DVec3::ZERO);
            let hits_player = p.from.is_some()
                && trace_cylinder(&player_cyl, start, end, DVec3::ZERO).is_some_and(|x| x.fraction < h.fraction);
            if h.fraction < 1.0 || hits_player || start.distance(p.pos) > p.max_range {
                commands.entity(e).despawn();
                continue;
            }
            p.pos = end;
            tf.translation = Vec3::from(ue_to_bevy(p.pos.as_vec3().to_array()));
            continue;
        }
        let h = world.trace(start, end, DVec3::ZERO);
        let ph_hit = if p.from.is_some() && !ph.dead {
            trace_cylinder(&player_cyl, start, end, DVec3::ZERO)
        } else {
            None
        };
        if let Some(r) = p.explodes {
            // DeusExProjectile.Explode: whatever it touches, it blows up just short of it.
            let f = ph_hit.map(|x| x.fraction).unwrap_or(1.0).min(h.fraction);
            if f < 1.0 {
                let at = start + (end - start) * f - p.vel.normalize_or_zero() * 4.0;
                p.impact(at);
                booms.0.push(crate::explosion::Boom {
                    pos: at,
                    damage: p.damage,
                    radius: r,
                    damage_type: p.damage_type,
                    from: p.from,
                    rings: true,
                    fireball: true,
                    fx: 1,
                });
                commands.entity(e).despawn();
                continue;
            }
        }
        if let Some((bird, f)) = crate::critters::trace_birds(start, end) {
            if f < h.fraction && ph_hit.map_or(true, |x| f < x.fraction) && p.explodes.is_none() {
                crate::critters::hit_bird(bird, p.damage);
                p.impact(start + (end - start) * f);
                commands.entity(e).despawn();
                continue;
            }
        }
        if ph_hit.filter(|x| x.fraction < h.fraction).is_some() {
            // Hit the player.
            let at = start + (end - start) * ph_hit.map(|x| x.fraction).unwrap_or(0.0);
            let off = crate::ai::player_offset(player.body.location, player_yaw, at);
            ph.damage(p.damage, p.damage_type, off);
            p.impact(at);
            commands.entity(e).despawn();
            continue;
        }
        if h.fraction < 1.0 {
            let at = start + (end - start) * h.fraction;
            if let Some(b) = h.actor {
                let a = &lvl.actors[blockers.list[b].actor];
                if find(&a.props, "GroundSpeed").is_some() {
                    pawn_hits.0.push(crate::damage::PawnHit {
                        blocker: b,
                        damage: p.damage,
                        damage_type: p.damage_type,
                        hit_loc: at,
                        attacker: p.from,
                        attacker_pos: at - p.vel.normalize_or_zero() * 64.0,
                    });
                } else {
                    deco_hits.0.push(crate::destruct::DecoHit {
                        blocker: b,
                        damage: p.damage,
                        damage_type: p.damage_type,
                    });
                }
                p.impact(at);
                commands.entity(e).despawn();
                continue;
            }
            // A wall (DeusExProjectile.HitWall: a mover it hits takes the damage).
            crate::movers::hit_line(start, at, p.damage, p.damage_type);
            // A wall: darts stick in it.
            p.impact(at);
            p.pos = at - p.vel.normalize_or_zero() * 2.0;
            if p.stick {
                p.stuck = true;
            } else {
                commands.entity(e).despawn();
                continue;
            }
        } else {
            p.pos = end;
        }
        tf.translation = Vec3::from(ue_to_bevy(p.pos.as_vec3().to_array()));
    }
}

/// DeusExWeapon.ProjectileFire spread: the aim's yaw and pitch each move by accuracy * (Rand(1024) - 512).
pub fn projectile_dir(dir: DVec3, accuracy: f64, seed: &mut u64) -> DVec3 {
    let mut rnd = || {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((*seed >> 33) as f64) / (1u64 << 31) as f64
    };
    let unit = std::f64::consts::TAU / 65536.0;
    let yaw = dir.y.atan2(dir.x) + accuracy * (rnd() * 1024.0 - 512.0) * unit;
    let pitch = dir.z.atan2(dir.truncate().length()) + accuracy * (rnd() * 1024.0 - 512.0) * unit;
    DVec3::new(
        yaw.cos() * pitch.cos(),
        yaw.sin() * pitch.cos(),
        pitch.sin(),
    )
}

/// A still of a FireTexture with a radial fade (cached as "<path>#round"), or None if it isn't a fire texture.
fn round_fire_image(
    assets: &mut RuntimeAssets,
    images: &mut Assets<Image>,
    path: &str,
) -> Option<Handle<Image>> {
    let key = format!("{path}#round");
    if let Some(h) = assets.images.get(&key) {
        return h.clone();
    }
    let h = assets.lib.load_fire(path).map(|mut f| {
        for _ in 0..60 {
            f.step();
        }
        let mut data = Vec::new();
        f.rgba(&mut data);
        let (w, hgt) = (f.width, f.height);
        for y in 0..hgt {
            for x in 0..w {
                let dx = (x as f32 + 0.5) / w as f32 * 2.0 - 1.0;
                let dy = (y as f32 + 0.5) / hgt as f32 * 2.0 - 1.0;
                let r2 = (dx * dx + dy * dy).min(1.0);
                // Brighter in the middle, nothing at the rim.
                let k = (1.0 - r2).powi(2) * 1.6;
                let o = (y * w + x) * 4;
                for c in 0..3 {
                    data[o + c] = (data[o + c] as f32 * k).min(255.0) as u8;
                }
                data[o + 3] = 255;
            }
        }
        let mut img = Image::new(
            bevy::render::render_resource::Extent3d {
                width: w as u32,
                height: hgt as u32,
                depth_or_array_layers: 1,
            },
            bevy::render::render_resource::TextureDimension::D2,
            data,
            bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
            bevy::asset::RenderAssetUsages::RENDER_WORLD,
        );
        img.sampler = bevy::image::ImageSampler::linear();
        images.add(img)
    });
    assets.images.insert(key, h.clone());
    h
}
