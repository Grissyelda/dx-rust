//! Swimming extras around the movement code (main.rs `swim_state` + `Physics::Swimming`):
//! - breath and drowning (DeusExPlayer.PlayerSwimming swimTimer / Pawn.HeadZoneChange / Pawn.PainTimer),
//! - splashes going in and out (Pawn.FootZoneChange: zone EntrySound/ExitSound, WaterRing),
//! - stroke sounds (the Tread anim's PlayFootStep notify), air bubbles, the zone's underwater ambient sound,
//! - the zone's ViewFog on screen while the eye is under water.

use bevy::prelude::*;
use glam::DVec3;

use crate::actors::water_at;
use crate::ai::PlayerHealth;
use crate::ai::Target;
use crate::stealth::{Noise, NoiseKind, Noises};
use crate::weapons::RuntimeAssets;
use crate::{FlyCam, LevelRes, Player, SkyCam};

/// SkillSwimming's value (1.0 untrained .. 2.5 master).
fn swim_skill() -> f64 {
    crate::abilities::skill("SkillSwimming") as f64
}

/// UnderWaterTime: 20 s, or the Aqualung's LevelValue while it's on.
fn under_water_time() -> f64 {
    crate::abilities::aug("AugAqualung").map_or(UNDER_WATER_TIME, |v| v as f64)
}
/// JCDentonMale.UnderWaterTime.
const UNDER_WATER_TIME: f64 = 20.0;
/// WaterZone defaults (the Island's WaterZone keeps them).
const ENTRY_SOUND: &str = "DeusExSounds.Generic.SplashMedium";
const EXIT_SOUND: &str = "DeusExSounds.Generic.WaterOut";
const AMBIENT: &str = "Ambient.Ambient.Underwater";
const VIEW_FOG: [f32; 3] = [0.0, 0.05, 0.1];

#[derive(Resource, Default)]
pub struct Swim {
    /// swimTimer / swimDuration.
    pub timer: f64,
    pub duration: f64,
    /// Pawn.PainTime (counts down; PainTimer fires when it crosses zero).
    pain_time: f64,
    head_in: bool,
    foot_in: bool,
    started: bool,
    splash_time: f64,
    bubble_timer: f64,
    /// Tread loop clock (12 frames at 15 fps, PlayFootStep notify at 30%).
    tread: f64,
    /// Extra WaterRings due: (time, place).
    rings: Vec<(f64, DVec3)>,
}

#[derive(Component)]
pub struct Bubble {
    vel: DVec3,
    pos: DVec3,
    life: f32,
    size: f32,
}

fn frand() -> f64 {
    crate::sound::frand() as f64
}

#[allow(clippy::too_many_arguments)]
pub fn swim_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    mut player: ResMut<Player>,
    mut sw: ResMut<Swim>,
    mut ph: ResMut<PlayerHealth>,
    mut noises: ResMut<Noises>,
    cam: Single<&Transform, (With<FlyCam>, Without<SkyCam>, Without<Bubble>)>,
    mut post: Query<&mut crate::dynlight::Brightness>,
    (mut assets, mut images, mut meshes, mut materials): (
        ResMut<RuntimeAssets>,
        ResMut<Assets<Image>>,
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
    ),
) {
    let _prof = crate::prof::scope("swim::swim_system");
    let dt = time.delta_secs_f64().min(0.1);
    let now = time.elapsed_secs_f64();
    let lvl = &level.0;
    let active = !player.noclip && !ph.dead;
    let loc = player.body.location;
    let h = player.params.height;
    let head = active && water_at(lvl, loc + DVec3::Z * player.eye_height).is_some();
    let foot = active && water_at(lvl, loc - DVec3::Z * (h - 1.0)).is_some();
    ph.in_water = active && water_at(lvl, loc).is_some();
    if !sw.started {
        // No splash for starting out where we are.
        sw.started = true;
        sw.head_in = head;
        sw.foot_in = foot;
    }

    let mut ring_at = |sw: &mut Swim, at: DVec3, size: f32, commands: &mut Commands| {
        water_ring(
            commands,
            &mut assets,
            &mut images,
            &mut meshes,
            &mut materials,
            at,
            size,
        );
        // WaterRing.Tick: two more rings, 0.2 s apart.
        sw.rings.push((now + 0.2, at));
        sw.rings.push((now + 0.4, at));
    };

    // Pawn.FootZoneChange: splash in, drip out (at most every 0.25 s), and a noise.
    if foot != sw.foot_in && now - sw.splash_time > 0.25 {
        sw.splash_time = now;
        let falling = player.body.physics == imsim_core::movement::Physics::Falling;
        let feet = loc - DVec3::Z * h;
        noises.0.push(Noise {
            pos: feet,
            volume: if falling { 1.0 } else { 0.3 },
            radius: if falling { 800.0 } else { 240.0 },
            maker: Some(Target::Player),
            instigator: Some(Target::Player),
            kind: NoiseKind::Loud,
        });
        if foot {
            let vz = player.body.velocity.z;
            let size = (0.000025 * player.params.mass * (300.0 - 0.5 * vz.max(-500.0)))
                .clamp(1.0, 4.0) as f32;
            // Deep water below: a big splash; wading in: a step.
            let below = lvl
                .collision
                .trace(
                    loc - DVec3::Z * h * 0.8,
                    loc - DVec3::Z * (h + 40.0) * 0.8,
                    DVec3::ZERO,
                )
                .fraction
                < 1.0;
            if below {
                crate::sound::play_owned(
                    crate::sound::PLAYER,
                    crate::sound::slot::MISC,
                    "DeusExSounds.Player.WaterStep1",
                    None,
                    1.5 + 0.5 * size,
                    1600.0,
                    1.0,
                );
            } else {
                crate::sound::play_owned(
                    crate::sound::PLAYER,
                    crate::sound::slot::MISC,
                    ENTRY_SOUND,
                    None,
                    2.0 * size,
                    1600.0,
                    1.0,
                );
            }
            ring_at(&mut sw, feet, size, &mut commands);
        } else {
            crate::sound::play_owned(
                crate::sound::PLAYER,
                crate::sound::slot::INTERACT,
                EXIT_SOUND,
                None,
                1.0,
                1600.0,
                1.0,
            );
            ring_at(&mut sw, feet, 1.0, &mut commands);
        }
    }
    sw.foot_in = foot;
    let due: Vec<DVec3> = sw
        .rings
        .iter()
        .filter(|(t, _)| *t <= now)
        .map(|(_, p)| *p)
        .collect();
    sw.rings.retain(|(t, _)| *t > now);
    for at in due {
        water_ring(
            &mut commands,
            &mut assets,
            &mut images,
            &mut meshes,
            &mut materials,
            at,
            1.0,
        );
    }

    // Head going under / coming up (Pawn + DeusExPlayer.HeadZoneChange).
    if head != sw.head_in {
        if head {
            // Diving in kills your momentum and refills the breath timer.
            player.body.velocity = DVec3::ZERO;
            sw.duration = under_water_time() * swim_skill();
            sw.timer = sw.duration;
            sw.pain_time = under_water_time();
        } else {
            if sw.pain_time > 0.0 && sw.pain_time < 8.0 {
                crate::sound::play_owned(
                    crate::sound::PLAYER,
                    crate::sound::slot::PAIN,
                    &crate::travel::voice("DeusExSounds.Player.MaleGasp"),
                    None,
                    1.0,
                    0.0,
                    crate::sound::random_pitch(),
                );
            }
            sw.pain_time = -1.0;
        }
        sw.head_in = head;
    }

    if player.swimming && active {
        if sw.duration <= 0.0 {
            sw.duration = under_water_time() * swim_skill();
            sw.timer = sw.duration;
        }
        // AugAqualung on/off rescales the breath left (pct of the old duration).
        let want = under_water_time() * swim_skill();
        if (want - sw.duration).abs() > 1e-6 {
            let pct = sw.timer / sw.duration;
            sw.duration = want;
            sw.timer = want * pct;
        }
        sw.timer = (sw.timer - dt).max(0.0);
        if sw.timer > 0.0 {
            sw.pain_time = sw.timer;
        }
        ph.on_fire = None; // ExtinguishFire
        ph.breath = Some((sw.timer / sw.duration) as f32);
        // Tread loop: one stroke sound per cycle (Swimming or Treading, volume x0.5, speed = WaterSpeed / 180).
        let cycle = 12.0 / 15.0;
        let before = sw.tread;
        sw.tread = (sw.tread + dt) % cycle;
        let mark = 0.3 * cycle;
        if before < mark && (sw.tread >= mark || sw.tread < before) {
            let vol = (player.params.water_speed * swim_skill() / 180.0 + 0.2) as f32;
            let snd = if frand() < 0.5 {
                "DeusExSounds.Player.Swimming"
            } else {
                "DeusExSounds.Player.Treading"
            };
            let volume = (vol * 0.5).clamp(0.0, 1.0) * 0.5;
            let range = (375.0 * vol).clamp(0.01, 1500.0);
            crate::sound::play_owned(
                crate::sound::PLAYER,
                crate::sound::slot::INTERACT,
                snd,
                None,
                volume,
                range,
                (vol + 0.5).clamp(1.0, 1.5),
            );
            noises.0.push(Noise {
                pos: loc,
                volume,
                radius: range,
                maker: Some(Target::Player),
                instigator: Some(Target::Player),
                kind: NoiseKind::Loud,
            });
        }
        // An air bubble in front of the eyes now and then (only under water - they pop at the surface).
        sw.bubble_timer += dt;
        if sw.bubble_timer >= 0.2 {
            sw.bubble_timer = 0.0;
            if frand() < 0.4 {
                let f = cam.forward().as_vec3();
                let view = DVec3::new(f.x as f64, f.z as f64, f.y as f64);
                let rnd = DVec3::new(
                    frand() * 2.0 - 1.0,
                    frand() * 2.0 - 1.0,
                    frand() * 2.0 - 1.0,
                )
                .normalize_or_zero()
                    * 4.0;
                let at = loc + rnd + view * player.params.radius * 2.0 + DVec3::Z * h * 0.9;
                if water_at(lvl, at).is_some() {
                    spawn_bubble(
                        &mut commands,
                        &mut assets,
                        &mut images,
                        &mut meshes,
                        &mut materials,
                        at,
                    );
                }
            }
        }
    } else {
        ph.breath = None;
        sw.tread = 0.0;
    }

    // Pawn.PainTimer: out of breath under water, 5 points every 2 s.
    if sw.pain_time > 0.0 && active {
        sw.pain_time -= dt;
        if sw.pain_time <= 0.0 && head {
            ph.damage(5.0, "Drowned", DVec3::ZERO);
            sw.pain_time = if ph.dead { -1.0 } else { 2.0 };
        }
    }

    // The zone's AmbientSound plays while the eye is in it, and its ViewFog tints the view.
    if head {
        crate::sound::hold(
            crate::sound::key_of("underwater-ambient"),
            AMBIENT,
            None,
            128.0 / 255.0,
            0.0,
            1.0,
        );
    }
    let fog = if head { VIEW_FOG } else { [0.0; 3] };
    for mut b in &mut post {
        if b.fog_r != fog[0] || b.fog_g != fog[1] || b.fog_b != fog[2] {
            b.fog_r = fog[0];
            b.fog_g = fog[1];
            b.fog_b = fog[2];
        }
    }
}

pub fn water_ring(
    commands: &mut Commands,
    assets: &mut RuntimeAssets,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    at: DVec3,
    size: f32,
) {
    // The water surface ripples out from here too (water.rs).
    crate::water::ripple(at, (size * 0.6).clamp(0.3, 1.5));
    // WaterRing.PostBeginPlay: laid flat (Pitch 16384), random yaw; the first draw halves DrawScale, then it
    // grows 2 per second and fades over its 0.5 s LifeSpan.
    let yaw = (frand() * 65535.0) as i32;
    if let Some(e) = crate::explosion::spawn_mesh_fx(
        commands,
        assets,
        images,
        meshes,
        materials,
        "WaterRing",
        at,
        [16384, yaw, 0],
        size,
    ) {
        commands
            .entity(e)
            .entry::<crate::explosion::MeshFx>()
            .and_modify(move |mut fx| fx.grow = Some((size * 0.5, 2.0)));
    }
}

fn spawn_bubble(
    commands: &mut Commands,
    assets: &mut RuntimeAssets,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    at: DVec3,
) {
    const TEX: &str = "DeusExItems.Skins.FlatFXTex45";
    let img = crate::weapons::image_for(assets, images, TEX);
    let width = assets
        .lib
        .load_image(TEX)
        .map(|t| t.width as f32)
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
    let draw_scale = 0.05 + frand() as f32 * 0.1;
    commands.spawn((
        Mesh3d(quad),
        MeshMaterial3d(material),
        Transform::from_translation(Vec3::from(dx_level::ue_to_bevy(at.as_vec3().to_array())))
            .with_scale(Vec3::splat(width * draw_scale * dx_level::METERS_PER_UNIT)),
        Bubble {
            vel: DVec3::ZERO,
            pos: at,
            life: 10.0,
            size: width * draw_scale,
        },
    ));
}

/// AirBubble.Tick: wobble sideways, rise at RiseRate (50) x 0.9..1.1, gone at the surface or after 10 s.
pub fn bubble_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    cam: Single<&Transform, (With<FlyCam>, Without<SkyCam>, Without<Bubble>)>,
    mut q: Query<(Entity, &mut Bubble, &mut Transform)>,
) {
    let _prof = crate::prof::scope("swim::bubble_system");
    let dt = time.delta_secs().min(0.1);
    for (e, mut b, mut tf) in &mut q {
        b.life -= dt;
        b.vel = DVec3::new(
            8.0 - frand() * 17.0,
            8.0 - frand() * 17.0,
            50.0 * (frand() * 0.2 + 0.9),
        );
        let step = b.vel * dt as f64;
        b.pos += step;
        if b.life <= 0.0 || water_at(&level.0, b.pos).is_none() {
            commands.entity(e).despawn();
            continue;
        }
        tf.translation = Vec3::from(dx_level::ue_to_bevy(b.pos.as_vec3().to_array()));
        tf.rotation = cam.rotation;
        tf.scale = Vec3::splat(b.size * dx_level::METERS_PER_UNIT);
    }
}
