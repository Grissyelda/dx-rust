//! Grenades (ThrownProjectile: LAM, gas, EMP). Thrown ones fly under gravity, stop dead against walls
//! (Elasticity 0) and settle on the floor, and go off when the fuse (3 s) runs out. Ones placed on a wall (the player
//! using a grenade within a foot of a wall, or put there by the level designer) are proximity mines: armed, they
//! wait for a pawn of Mass >= 40 or the player within proxRadius, then beep and blow after skillTime (1 s untrained).
//! Using an armed mine before it goes off disarms it; using a disarmed one picks it up as the grenade weapon.
//! Explosions nearby set them off too.

use crate::actors::{Blockers, World as ActWorld};
use crate::ai::Npc;
use crate::explosion::{BlastsThisFrame, Boom, Explosions};
use crate::weapons::{spawn_mesh_parts, RuntimeAssets};
use crate::{LevelRes, Player};
use bevy::prelude::*;
use dx_level::actors::{light_part, rotator_to_bevy_mat3};
use dx_level::ue_to_bevy;
use glam::DVec3;
use imsim_core::movement::CollisionWorld;
use unreal_pkg::props::{find, Property, Value};

#[derive(Component)]
pub struct Grenade {
    pub class: String,
    pub pos: DVec3,
    pub vel: DVec3,
    pub falling: bool,
    pub stuck: bool,
    pub proximity: bool,
    pub armed: bool,
    pub disabled: bool,
    pub fuse: f32,
    pub time: f32,
    /// Countdown once something walked into range (0 = not triggered).
    pub skill_time: f32,
    pub prox_radius: f64,
    pub prox_check: f32,
    pub damage: f32,
    pub blast: f64,
    pub damage_type: &'static str,
    pub from: Option<usize>,
    /// Placed by the level (Owner == None): only the player sets it off.
    pub placed: bool,
    pub weapon: String,
    pub name: String,
    pub impact_snd: Option<String>,
    pub misc_snd: Option<String>,
    pub beep_phase: f32,
    /// ThrownProjectile.beepTime: the countdown beeper until the fuse runs out.
    pub beep_time: f32,
    /// Tumbling while it flies (bFixedRotationDir, RotationRate re-rolled each bounce), Unreal rotator units.
    pub rot: [f64; 3],
    pub rot_rate: [f64; 3],
    pub elasticity: f64,
}

/// Grenades to create: thrown (class, pos, dir, thrower) or placed on a wall (pos, normal).
#[derive(Resource, Default)]
pub struct GrenadeThrows(pub Vec<(String, DVec3, DVec3, Option<usize>, Option<DVec3>)>);

fn damage_type_of(n: &str) -> &'static str {
    match n.to_ascii_lowercase().as_str() {
        "teargas" => "TearGas",
        "emp" => "EMP",
        "poisongas" => "PoisonGas",
        "nanovirus" => "NanoVirus",
        _ => "Exploded",
    }
}

pub fn is_grenade_class(p: &[Property]) -> bool {
    find(p, "fuseLength").is_some() && find(p, "blastRadius").is_some()
}

fn make(
    class: &str,
    d: &[Property],
    pos: DVec3,
    vel: DVec3,
    from: Option<usize>,
    placed: bool,
    proximity: bool,
) -> Grenade {
    let f = |n: &str, dv: f32| match find(d, n) {
        Some(Value::Float(v)) => *v,
        Some(Value::Int(v)) => *v as f32,
        _ => dv,
    };
    let fuse = f("fuseLength", 3.0);
    Grenade {
        impact_snd: crate::sound::prop(d, "ImpactSound"),
        misc_snd: crate::sound::prop(d, "MiscSound"),
        beep_phase: 0.0,
        beep_time: 0.0,
        rot: [0.0; 3],
        rot_rate: [65536.0, 65536.0, 0.0],
        elasticity: f("elasticity", 0.5) as f64,
        class: class.to_string(),
        pos,
        vel,
        falling: !placed && !proximity,
        stuck: placed || proximity,
        proximity,
        armed: false,
        disabled: false,
        fuse,
        // Preplaced ones don't beep at the start of a level (time = fuseLength); they arm at once.
        time: if placed { fuse } else { 0.0 },
        skill_time: 0.0,
        prox_radius: f("proxRadius", 128.0) as f64,
        prox_check: 0.0,
        damage: f("Damage", 100.0),
        blast: f("blastRadius", 256.0) as f64,
        damage_type: match find(d, "DamageType") {
            Some(Value::Name(n)) => damage_type_of(n),
            _ => "Exploded",
        },
        from,
        placed,
        weapon: match find(d, "spawnWeaponClass") {
            Some(Value::Path(p)) => p.rsplit('.').next().unwrap_or("").to_string(),
            _ => String::new(),
        },
        name: match find(d, "ItemName") {
            Some(Value::Str(s)) => s.clone(),
            _ => class.to_string(),
        },
    }
}

/// Level-placed grenades become live Grenades (their mesh entities already exist).
pub fn attach_grenades(
    mut commands: Commands,
    level: Res<LevelRes>,
    q: Query<(Entity, &crate::actors::MeshActorRef), Without<Grenade>>,
    mut map_gen: Local<u64>,
    mut done: Local<bool>,
) {
    // A new map (loader.rs): start this system's per-map state over.
    if crate::loader::new_map(&mut map_gen) {
        *done = false;
    }
    if *done {
        return;
    }
    *done = true;
    for (e, r) in &q {
        let ma = &level.0.mesh_actors[r.0];
        let a = &level.0.actors[ma.actor];
        if !is_grenade_class(&a.props) {
            continue;
        }
        let l = a.location();
        let prox = matches!(
            find(&a.props, "bProximityTriggered"),
            Some(Value::Bool(true))
        );
        let mut g = make(
            &a.class,
            &a.props,
            DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64),
            DVec3::ZERO,
            None,
            true,
            prox,
        );
        g.armed = prox;
        // Mission00.FirstFrame, 00_TRAININGCOMBAT: "fool a lam into thinking that the player set it"
        // (SetOwner(Player)) - an owned mine is set off by heavy pawns, so the patrolling bot walks into it.
        if level.0.name.to_ascii_lowercase().starts_with("00_trainingcombat")
            && matches!(find(&a.props, "Tag"), Some(Value::Name(t)) if t.eq_ignore_ascii_case("Bot1LAM"))
        {
            g.placed = false;
        }
        commands.entity(e).insert(g);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn grenade_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    blockers: Res<Blockers>,
    mv: Res<crate::movers::Movers>,
    player: Res<Player>,
    mut throws: ResMut<GrenadeThrows>,
    mut booms: ResMut<Explosions>,
    blasts: Res<BlastsThisFrame>,
    (mut assets, mut images, mut meshes, mut materials): (
        ResMut<RuntimeAssets>,
        ResMut<Assets<Image>>,
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
    ),
    mut q: Query<(Entity, &mut Grenade, &mut Transform)>,
    npcs: Query<&Npc>,
    mut hud: ResMut<crate::movers::HudMessage>,
    mut gas: ResMut<crate::projectiles::GasClouds>,
) {
    let _prof = crate::prof::scope("grenades::grenade_system");
    let lvl = &level.0;
    // New grenades.
    for (class, pos, dir, from, wall) in throws.0.drain(..) {
        let mut d = assets.classes.defaults(&class);
        // Augmentique: the player's grenade skin (ApplyProjectileSkinFrom).
        if from.is_none() {
            crate::augmentique::apply_projectile_skin(&mut d, &format!("Weapon{class}"));
        }
        let speed = match find(&d, "speed") {
            Some(Value::Float(v)) => *v as f64,
            _ => 1000.0,
        };
        let placed_on_wall = wall.is_some();
        let mut g = make(&class, &d, pos, dir * speed, from, false, placed_on_wall);
        let rot = match wall {
            Some(n) => {
                let yaw = n.y.atan2(n.x);
                let pitch = n.z.atan2(n.truncate().length());
                let u = |r: f64| (r / std::f64::consts::TAU * 65536.0) as i32;
                [u(pitch), u(yaw), 0]
            }
            None => [0, 0, 0],
        };
        if placed_on_wall {
            // PlaceGrenade: stuck, proximity-triggered; it arms when its fuse timer first runs out.
            g.vel = DVec3::ZERO;
        }
        let m = rotator_to_bevy_mat3(rot);
        let root = commands
            .spawn((
                Transform::from_translation(Vec3::from(ue_to_bevy(pos.as_vec3().to_array())))
                    .with_rotation(Quat::from_mat3(&Mat3::from_cols(
                        Vec3::from(m[0]),
                        Vec3::from(m[1]),
                        Vec3::from(m[2]),
                    ))),
                Visibility::default(),
            ))
            .id();
        if let Some(Value::Path(mesh)) = find(&d, "Mesh") {
            let mesh = mesh.clone();
            let map = |p: [f32; 3]| ue_to_bevy(p);
            if let Some((_m, parts, handles)) = spawn_mesh_parts(
                &mesh,
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
                for (h, part) in handles.iter().zip(parts) {
                    if let Some(mut mm) = meshes.get_mut(h) {
                        mm.insert_attribute(
                            Mesh::ATTRIBUTE_COLOR,
                            light_part(&part, pos.as_vec3().to_array(), rot, 1.0, &lvl.lights),
                        );
                    }
                }
            }
        }
        commands.entity(root).insert(g);
    }
    let dt = time.delta_secs();
    for (e, mut g, mut tf) in &mut q {
        let g = &mut *g;
        g.time += dt;
        let mut explode = false;
        // Blown up by another explosion nearby (TakeDamage; gas doesn't set grenades off, EMP disables them).
        for (p, r, ty) in &blasts.0 {
            if p.distance(g.pos) < *r {
                match *ty {
                    "TearGas" | "PoisonGas" | "Radiation" | "NanoVirus" => {}
                    "EMP" => g.disabled = true,
                    _ => explode = true,
                }
            }
        }
        if g.disabled && !explode {
            continue;
        }
        // Flight: ThrownProjectile.HitWall reflects off whatever it hits with Elasticity, re-rolls its spin, clinks
        // (MiscSound) when it hits faster than 50, and settles once slower than 60 on a floor.
        if g.falling && !g.stuck {
            g.vel.z -= 950.0 * dt as f64;
            let mut left = dt as f64;
            for _ in 0..4 {
                if left <= 0.0 {
                    break;
                }
                let start = g.pos;
                let end = start + g.vel * left;
                let world = ActWorld {
                    level: lvl,
                    blockers: &blockers.list,
                    ignore: g.from,
                    pushable: None,
                    movers: Some(&mv.rt),
                };
                let h = world.trace(start, end, DVec3::splat(2.0));
                if h.fraction >= 1.0 {
                    g.pos = end;
                    break;
                }
                g.pos = start + (end - start) * (h.fraction - 0.01).max(0.0) + h.normal * 0.1;
                left *= 1.0 - h.fraction;
                let n = h.normal;
                g.vel = (g.vel.dot(n) * n * -2.0 + g.vel) * g.elasticity;
                let speed = g.vel.length();
                g.rot_rate = [
                    (crate::sound::frand() as f64 * 2.0 - 1.0) * 65536.0,
                    (crate::sound::frand() as f64 * 2.0 - 1.0) * 65536.0,
                    (crate::sound::frand() as f64 * 2.0 - 1.0) * 65536.0,
                ];
                if speed < 60.0 && n.z > 0.7 {
                    if let Some(snd) = &g.misc_snd {
                        crate::sound::play(
                            snd,
                            g.pos,
                            0.5 + crate::sound::frand() * 0.5,
                            512.0,
                            0.85 + crate::sound::frand() * 0.3,
                        );
                    }
                    // Settle: rotation = Rotator(HitNormal) with a random yaw.
                    g.vel = DVec3::ZERO;
                    g.stuck = true;
                    g.rot = [
                        (n.z.clamp(-1.0, 1.0).asin() / std::f64::consts::TAU * 65536.0),
                        crate::sound::frand() as f64 * 65536.0,
                        0.0,
                    ];
                    break;
                } else if speed > 50.0 {
                    if let Some(snd) = &g.misc_snd {
                        crate::sound::play(
                            snd,
                            g.pos,
                            0.5 + crate::sound::frand() * 0.5,
                            512.0,
                            0.85 + crate::sound::frand() * 0.3,
                        );
                    }
                }
            }
            if !g.stuck {
                for k in 0..3 {
                    g.rot[k] += g.rot_rate[k] * dt as f64;
                }
            }
            let r = [g.rot[0] as i32, g.rot[1] as i32, g.rot[2] as i32];
            let m = rotator_to_bevy_mat3(r);
            tf.rotation = Quat::from_mat3(&Mat3::from_cols(
                Vec3::from(m[0]),
                Vec3::from(m[1]),
                Vec3::from(m[2]),
            ));
            tf.translation = Vec3::from(ue_to_bevy(g.pos.as_vec3().to_array()));
        }
        // Beep faster as the fuse runs down: every 0.5 s, 0.3 s in the last half, 0.1 s in the last 0.75 s.
        if !g.disabled && g.time < g.fuse {
            g.beep_time += dt;
            let left = g.fuse - g.time;
            let rate = if left <= 0.75 {
                0.1
            } else if left <= g.fuse * 0.5 {
                0.3
            } else {
                0.5
            };
            if g.beep_time > rate {
                g.beep_time = 0.0;
                crate::sound::play("DeusExSounds.Generic.Beep4", g.pos, 1.0, 1280.0, 1.0);
            }
        }
        // Timer: a thrown grenade explodes when its fuse runs out; a proximity one arms.
        if g.time >= g.fuse && !g.armed {
            if g.proximity {
                g.armed = true;
            } else {
                explode = true;
            }
        }
        if std::env::var("DXVIEW_GRENLOG").is_ok() && (g.time * 2.0).floor() != ((g.time - dt) * 2.0).floor() {
            let near = npcs
                .iter()
                .filter(|n| n.blocker != usize::MAX)
                .map(|n| blockers.list[n.blocker].center.distance(g.pos))
                .fold(f64::MAX, f64::min);
            println!(
                "GREN {} t {:.1} armed {} prox {} placed {} disabled {} skill {:.2} nearest pawn {:.0} (radius {:.0})",
                g.class, g.time, g.armed, g.proximity, g.placed, g.disabled, g.skill_time, near, g.prox_radius
            );
        }
        if g.proximity && g.armed {
            g.prox_check += dt;
            if g.skill_time > 0.0 {
                // About to go off: Beep4 at pitch 2 every 0.3 s.
                g.beep_phase -= dt;
                if g.beep_phase <= 0.0 {
                    g.beep_phase = 0.3;
                    crate::sound::play("DeusExSounds.Generic.Beep4", g.pos, 1.0, 1280.0, 2.0);
                }
                g.skill_time -= dt;
                if g.skill_time <= 0.0 {
                    explode = true;
                }
            }
            if g.prox_check > 0.25 && g.skill_time == 0.0 {
                g.prox_check = 0.0;
                let mut tripped = false;
                if g.placed || g.from.is_none() {
                    // Owner None (placed): the player sets it off; skillTime = clamp(-20 x Demolition, 1, 10).
                    if g.placed && player.body.location.distance(g.pos) < g.prox_radius {
                        g.skill_time =
                            (-20.0 * crate::abilities::skill("SkillDemolition")).clamp(1.0, 10.0);
                    }
                }
                if !g.placed {
                    // The player's mines: pawns of Mass >= 40 set them off.
                    for n in &npcs {
                        if n.state == crate::ai::NpcState::Dead || n.blocker == usize::MAX {
                            continue;
                        }
                        let c = blockers.list[n.blocker].center;
                        if c.distance(g.pos) < g.prox_radius {
                            tripped = true;
                        }
                    }
                }
                if tripped {
                    g.skill_time = 1.0;
                }
            }
        }
        if explode {
            // ThrownProjectile.Explode: ImpactSound, volume 2, heard out to blastRadius x 16.
            if let Some(s) = &g.impact_snd {
                crate::sound::play(s, g.pos, 2.0, (g.blast * 16.0) as f32, 1.0);
            }
            booms.0.push(Boom {
                pos: g.pos,
                damage: g.damage,
                radius: g.blast,
                damage_type: g.damage_type,
                from: g.from,
                rings: true,
                fireball: matches!(g.damage_type, "Exploded" | "EMP"),
                fx: if g.damage_type == "EMP" { 3 } else { 2 },
            });
            if g.damage_type == "TearGas" {
                // SpawnTearGas: blastRadius / 36 floating clouds that linger 30-40 s.
                let n = (g.blast / 36.0) as usize;
                let mut seed = (g.pos.x as i64 as u64) ^ 0x7EA5_6A5;
                let mut rnd = || {
                    seed = seed
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    ((seed >> 33) as f64) / (1u64 << 31) as f64
                };
                let mut clouds = Vec::new();
                for _ in 0..n {
                    if rnd() < 0.9 {
                        let p = g.pos
                            + DVec3::new(
                                rnd() * g.blast - g.blast * 0.5,
                                rnd() * g.blast - g.blast * 0.5,
                                32.0,
                            );
                        clouds.push((
                            p,
                            rnd() as f32 * 0.5 + 2.0,
                            rnd() as f32 * 10.0 + 30.0,
                            g.from,
                        ));
                    }
                }
                gas.0.extend(clouds);
            }
            println!(
                "GRENADE {} exploded at ({:.0},{:.0},{:.0})",
                g.class, g.pos.x, g.pos.y, g.pos.z
            );
            commands.entity(e).despawn();
        }
    }
    let _ = &mut hud;
}

/// Using a grenade (frob): an armed mine about to go is disarmed; a disarmed (or placed and unarmed) one is taken
/// as its weapon. Returns a message.
pub fn frob_grenade(g: &mut Grenade) -> (Option<String>, bool) {
    if !g.disabled && g.proximity && g.armed {
        g.disabled = true;
        return (Some(format!("{} disarmed", g.name)), false);
    }
    (None, true)
}

/// Live thrown grenades (IsProjectileDangerous: ThrownProjectiles that aren't proximity mines): where, blast
/// radius, and who threw them (blocker; None = the player). Read by the AI (ReactToProjectiles).
pub static DANGER: std::sync::Mutex<Vec<(DVec3, f64, Option<usize>)>> =
    std::sync::Mutex::new(Vec::new());

pub fn danger_system(q: Query<&Grenade>) {
    let _prof = crate::prof::scope("grenades::danger_system");
    if let Ok(mut d) = DANGER.lock() {
        d.clear();
        d.extend(
            q.iter()
                .filter(|g| !g.proximity && !g.disabled && !g.placed)
                .map(|g| (g.pos, g.blast, g.from)),
        );
    }
}

/// A new map is going in (loader.rs): this module's state of the map being left.
pub fn reset_level() {
    if let Ok(mut v) = DANGER.lock() {
        v.clear();
    }
}
