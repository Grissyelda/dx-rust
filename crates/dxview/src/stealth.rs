//! Stealth: how visible the player is (light where they stand, how fast they move) and what NPCs can hear.
//!
//! The scripts hand these to engine natives (Actor.AIVisibility, Pawn.AICanSee / AICanHear, AISendEvent) whose C++
//! is not available, so they are rebuilt here from what the scripts expect of them:
//! - visibility 0..1 from the light reaching the actor (lights in range with a clear line), plus a movement term;
//! - sight: within SightRadius, inside the horizontal field of view, a clear line to the head, centre or feet,
//!   and (for first sightings) the visibility scaled down with distance must beat VisibilityThreshold;
//! - hearing: an event of `volume` heard up to `radius`, fading with distance (halved through walls), must beat
//!   HearingThreshold.
//! The NPC side (EnemyReadiness, SightPercentage, Seeking) follows ScriptedPawn.CheckEnemyPresence / CheckCycle.

use crate::ai::Target;
use crate::{LevelRes, Player};
use bevy::prelude::*;
use dx_level::Level;
use glam::DVec3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NoiseKind {
    /// AISendEvent('LoudNoise'): footsteps, landings, things breaking.
    Loud,
    /// AISendEvent('WeaponFire'): gunshots (also counts as a loud noise).
    WeaponFire,
    /// AISendEvent('Distress'): a pawn crying out in pain or dying; `instigator` is whoever hurt it.
    Distress,
    /// Enhancement (thrown objects / found bodies): something worth a look; any armed pawn that hears it goes to
    /// see, whoever made it (a LoudNoise needs an enemy instigator).
    Alert,
}

/// Enhancement: thrown objects make noises guards investigate (setting ThrownNoise).
pub fn thrown_noise() -> bool {
    crate::settings::get("ThrownNoise") > 0.5
}

#[derive(Debug, Clone)]
pub struct Noise {
    pub pos: DVec3,
    pub volume: f32,
    pub radius: f32,
    /// Who made it (AISendEvent's sender, params.bestActor when it is a pawn or the player).
    pub maker: Option<Target>,
    /// Who is responsible (the Instigator), e.g. whoever threw the crate that landed.
    pub instigator: Option<Target>,
    pub kind: NoiseKind,
}

/// Noises made this frame, consumed by the NPCs.
#[derive(Resource, Default)]
pub struct Noises(pub Vec<Noise>);

/// The player's light level (updated a few times a second).
#[derive(Resource, Default)]
pub struct PlayerLight {
    pub brightness: f32,
    timer: f32,
}

/// Brightness (0..1) of the light reaching `pos` (AIVisibility's lighting): static and dynamic lights in range
/// with a clear line to them, each brightness x (1 - distance / radius). The engine's AIVisibility is native
/// and undocumented; this is calibrated by play-testing (env DXVIEW_VIS_GAIN scales it, default 1).
pub fn light_at(level: &Level, pos: DVec3) -> f32 {
    let gain = vis_gain();
    let dynamic = crate::dynlight::current();
    let mut sum = 0.0f32;
    for l in level.lights.iter().chain(dynamic.iter()) {
        let lp = DVec3::new(l.pos[0] as f64, l.pos[1] as f64, l.pos[2] as f64);
        let d = lp.distance(pos) as f32;
        if d >= l.radius {
            continue;
        }
        if level.collision.trace(pos, lp, DVec3::ZERO).fraction < 0.98 {
            continue;
        }
        let lum = (l.color[0] + l.color[1] + l.color[2]) / 3.0;
        sum += lum * (1.0 - d / l.radius) * gain;
    }
    sum.clamp(0.0, 1.0)
}

fn vis_gain() -> f32 {
    static G: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *G.get_or_init(|| {
        std::env::var("DXVIEW_VIS_GAIN")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0)
    })
}

pub fn player_light_system(
    time: Res<Time>,
    level: Res<LevelRes>,
    player: Res<Player>,
    mut pl: ResMut<PlayerLight>,
) {
    let _prof = crate::prof::scope("stealth::player_light_system");
    pl.timer -= time.delta_secs();
    if pl.timer > 0.0 {
        return;
    }
    pl.timer = 0.15;
    let b = &player.body;
    // Average of the light on the chest and the head.
    let head = b.location + DVec3::new(0.0, 0.0, player.params.height * 0.8);
    pl.brightness = (light_at(&level.0, b.location) + light_at(&level.0, head)) * 0.5;
}

/// AIVisibility(bIncludeVelocity=true) for the player: light plus a little for moving (running counts, creeping
/// hardly at all).
pub fn player_visibility(pl: &PlayerLight, velocity: DVec3) -> f32 {
    let speed = (DVec3::new(velocity.x, velocity.y, 0.0).length() as f32 / 320.0).min(1.5);
    (pl.brightness + 0.15 * speed * speed).clamp(0.0, 1.0)
}

/// Footsteps (DeusExPlayer.PlayFootStep): a LoudNoise every stride, louder and wider the faster you go.
pub fn footstep_system(
    player: Res<Player>,
    level: Res<LevelRes>,
    mut noises: ResMut<Noises>,
    mut last: Local<Option<DVec3>>,
    mut stride: Local<f64>,
) {
    let _prof = crate::prof::scope("stealth::footstep_system");
    let b = &player.body;
    let walking = matches!(
        b.physics,
        imsim_core::movement::Physics::Walking | imsim_core::movement::Physics::Climbing
    ) && !player.noclip;
    let prev = last.replace(b.location).unwrap_or(b.location);
    if !walking {
        return;
    }
    let step = if b.physics == imsim_core::movement::Physics::Climbing {
        (b.location - prev).length()
    } else {
        DVec3::new(b.location.x - prev.x, b.location.y - prev.y, 0.0).length()
    };
    *stride += step;
    // About one footfall per 90 units travelled.
    if *stride < 90.0 {
        return;
    }
    *stride = 0.0;
    // DeusExPlayer.PlayFootStep: the sound comes from the floor's texture group (GetFloorMaterial traces
    // CollisionHeight x 2 down); volume, range and pitch from speed and mass.
    let speed = b.velocity.length() as f32;
    let speed_factor = speed / 180.0;
    let mass_factor = 150.0 / 150.0;
    let vol = (speed_factor + 0.2) * mass_factor;
    let range = (375.0 * vol).clamp(0.01, 375.0 * 4.0);
    let pitch = (vol + 0.5).clamp(1.0, 1.5);
    let volume = vol.clamp(0.0, 1.0) * 0.5;
    // AugStealth (Run Silent): RunSilentValue scales the step's volume; at the top level it's silent.
    let silent = crate::abilities::aug("AugStealth").unwrap_or(1.0);
    if silent <= 0.0 {
        return;
    }
    let volume = volume * silent;
    let foot = b.location - DVec3::new(0.0, 0.0, player.params.height);
    let lvl = &level.0;
    let water = matches!(
        lvl.zone_water.get(
            lvl.collision
                .zone_at(foot + DVec3::new(0.0, 0.0, 2.0))
                .max(0) as usize
        ),
        Some(Some(_))
    );
    let group = if b.physics == imsim_core::movement::Physics::Climbing {
        Some("Ladder")
    } else {
        lvl.floor_group(b.location.to_array(), player.params.height * 2.0)
    };
    let (snd, mult) = crate::sound::footstep_sound(group, water);
    crate::dualsense::event(crate::dualsense::Haptic::Step(if water { Some("Water".into()) } else { group.map(|g| g.to_string()) }));
    crate::sound::play_owned(
        crate::sound::PLAYER,
        crate::sound::slot::INTERACT,
        &snd,
        None,
        volume,
        range,
        pitch,
    );
    // Climbing a ladder plays the step sound but makes no AI noise: Deus Ex's footsteps (and their LoudNoise) come
    // from the walk / run animations, which don't play on a ladder - so a climb up to waiting guards doesn't
    // bring them to the hatch.
    if b.physics == imsim_core::movement::Physics::Climbing {
        return;
    }
    noises.0.push(Noise {
        pos: b.location,
        volume: volume * mult,
        radius: range * mult,
        maker: Some(Target::Player),
        instigator: Some(Target::Player),
        kind: NoiseKind::Loud,
    });
}

/// AICanHear: how loud a noise is at the listener (0 if out of range).
pub fn hear(level: &Level, ear: DVec3, n: &Noise) -> f32 {
    let d = ear.distance(n.pos) as f32;
    if d >= n.radius || n.radius <= 0.0 {
        return 0.0;
    }
    let mut v = n.volume * (1.0 - d / n.radius);
    if level.collision.trace(ear, n.pos, DVec3::ZERO).fraction < 0.98 {
        v *= 0.5; // muffled by walls
    }
    v
}

/// AICanSee with bCheckVisibility: the target's visibility faded with distance, or 0 below the threshold.
pub fn sight_visibility(visibility: f32, dist: f64, sight_radius: f64, threshold: f32) -> f32 {
    if dist > sight_radius {
        return 0.0;
    }
    let v = visibility * (1.0 - (dist / sight_radius) as f32);
    if v < threshold {
        0.0
    } else {
        v
    }
}

/// DeusExWeapon.GetAIVolume: the noise a shot makes (none for silenced or melee weapons).
pub fn weapon_noise(w: &crate::weapons::WeaponStats, pos: DVec3, who: Target) -> Option<Noise> {
    if w.silenced || w.hand_to_hand || w.noise_level <= 0.0 {
        return None;
    }
    let volume = w.noise_level;
    Some(Noise {
        pos,
        volume,
        radius: volume * 800.0,
        maker: Some(who),
        instigator: Some(who),
        kind: NoiseKind::WeaponFire,
    })
}
