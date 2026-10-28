//! Ambient fish: FishGenerator / Fish2Generator (PawnGenerator) and Fishes (Fish, Fish2).
//!
//! * A generator keeps up to MaxCount fish alive while the player is within ActiveArea (1200), trying one every
//!   0.25 s at a random spot within Radius (70) of itself, in water, out of the player's sight. They are
//!   transient: out of ActiveArea and unseen for 5 s they go back into the pool. bGeneratorIsHome: the generator
//!   is every fish's home, HomeExtent = PawnHomeExtent (400).
//! * Flocking (PawnGenerator.UpdateSumVelocities, every 0.5 s): the generator sums its fish's velocities and
//!   averages their positions (FlockCenter). Every coast period (FishGenerator 3-6 s, Fish2Generator 3-8 s) it adds
//!   a random horizontal "turn" velocity (1.1 x the sum's size) for TurnPeriod (0.8 s), which swings the school.
//! * A fish (Fishes state Wandering) alternates: MoveTo a point 50 ahead along its velocity (a short dart), then for
//!   2-12 s (forwardTimer) keeps turning toward PickDirection: the school's summed velocity, pulled toward the flock
//!   centre when it's 1-4 x Radius away, back toward home when outside HomeExtent, with a +-4096 yaw / +-1500 pitch
//!   wobble while it's "following" (leaderTimer 5-15 s, then 2 s on its own). HitWall reflects it (once per
//!   0.5 s). Fish swim where they face, accelerating at AccelRate up to WaterSpeed (Fish 50, Fish2 30).
//!   (Deviation, noted: UE1 keeps the last MoveTo acceleration while TurnTo runs; here the swim direction follows
//!   the facing so the fish never slide sideways.)
//! * They ignore everything (BlockReactions). Shot or blown up, a fish (Health 10) just disappears: CarcassType
//!   is None and there's no death sound.

use crate::weapons::{spawn_mesh_parts, Clip, RuntimeAssets};
use crate::{FlyCam, LevelRes, Player, SkyCam};
use bevy::prelude::*;
use dx_level::actors::{anim_mesh, light_part, rotator_to_bevy_mat3, AnimMesh, MeshPart};
use dx_level::ue_to_bevy;
use glam::DVec3;
use std::f64::consts::TAU;
use std::sync::{Arc, Mutex};
use unreal_pkg::props::{find, Property, Value};

/// Fish collision cylinders this frame (entity, centre, radius, height), for weapon traces (critters::trace_birds).
pub static TARGETS: Mutex<Vec<(Entity, DVec3, f64, f64)>> = Mutex::new(Vec::new());
/// Damage dealt to fish since the last update.
pub static HITS: Mutex<Vec<(Entity, f32)>> = Mutex::new(Vec::new());

/// Is this entity one of the fish (critters::hit_bird routes hits here)?
pub fn is_fish(e: Entity) -> bool {
    TARGETS
        .lock()
        .map(|t| t.iter().any(|x| x.0 == e))
        .unwrap_or(false)
}

const UNR: f64 = TAU / 65536.0;

struct Gen {
    class: &'static str,
    /// FishGenerator: coast 3-6 s; Fish2Generator: 3-8 s.
    coast_extra: f64,
    pos: DVec3,
    yaw: f64,
    active_area: f64,
    radius: f64,
    home_extent: f64,
    turn_period: f64,
    focus: f64,
    max: usize,
    pool: usize,
    fish: Vec<Entity>,
    try_timer: f32,
    status_timer: f32,
    // UpdateSumVelocities
    velocity_timer: f64,
    coast_timer: f64,
    random_velocity: DVec3,
    sum_velocities: DVec3,
    flock_center: DVec3,
}

#[derive(Resource, Default)]
pub struct FishState {
    init: bool,
    gens: Vec<Gen>,
    seed: u64,
}

impl FishState {
    fn rand(&mut self) -> f64 {
        self.seed = self
            .seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.seed >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    /// Begin: MoveTo(Location + facing * (CollisionRadius + 5)).
    Start(DVec3),
    /// MoveTo(PickDirection(true)).
    Dart(DVec3),
    /// TurnTo(PickDirection(false)), re-picked when the turn completes.
    Turn(DVec3),
}

#[derive(Component)]
pub struct Fish {
    gen: usize,
    pos: DVec3,
    vel: DVec3,
    yaw: f64,
    pitch: f64,
    phase: Phase,
    leader_timer: f64,
    forward_timer: f64,
    bump_timer: f64,
    abort_timer: f64,
    unseen: f32,
    radius: f64,
    height: f64,
    water_speed: f64,
    accel_rate: f64,
    health: f32,
    mesh: Arc<AnimMesh>,
    parts: Vec<MeshPart>,
    handles: Vec<Handle<Mesh>>,
    clip: Clip,
    lit_at: DVec3,
}

fn fprop(p: &[Property], n: &str, d: f64) -> f64 {
    match find(p, n) {
        Some(Value::Float(v)) => *v as f64,
        Some(Value::Int(v)) => *v as f64,
        Some(Value::Byte(v)) => *v as f64,
        _ => d,
    }
}

fn bprop(p: &[Property], n: &str, d: bool) -> bool {
    match find(p, n) {
        Some(Value::Bool(v)) => *v,
        _ => d,
    }
}

fn fish_class(generator: &str) -> Option<(&'static str, f64)> {
    match generator {
        "FishGenerator" => Some(("Fish", 3.0)),
        "Fish2Generator" => Some(("Fish2", 5.0)),
        _ => None,
    }
}

fn ue(v: Vec3) -> DVec3 {
    DVec3::new(v.x as f64, v.z as f64, v.y as f64)
}

fn dir_of(yaw: f64, pitch: f64) -> DVec3 {
    DVec3::new(yaw.cos() * pitch.cos(), yaw.sin() * pitch.cos(), pitch.sin())
}

/// Rotator(v) as (yaw, pitch) in radians.
fn rot_of(v: DVec3) -> (f64, f64) {
    let flat = (v.x * v.x + v.y * v.y).sqrt();
    (v.y.atan2(v.x), v.z.atan2(flat))
}

fn wrap(a: f64) -> f64 {
    let mut a = a % TAU;
    if a > std::f64::consts::PI {
        a -= TAU;
    } else if a < -std::f64::consts::PI {
        a += TAU;
    }
    a
}

/// Swimmable: inside a water zone with a little room above (fish don't break the surface).
fn swimmable(level: &dx_level::Level, p: DVec3, height: f64) -> bool {
    crate::critters::in_water(level, p) && crate::critters::in_water(level, p + DVec3::new(0.0, 0.0, height + 2.0))
}

#[allow(clippy::too_many_arguments)]
pub fn fish_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    player: Res<Player>,
    cam: Single<&Transform, (With<FlyCam>, Without<SkyCam>, Without<Fish>)>,
    mut st: ResMut<FishState>,
    mut fishes: Query<(Entity, &mut Fish, &mut Transform), Without<FlyCam>>,
    (mut assets, mut images, mut meshes, mut materials): (
        ResMut<RuntimeAssets>,
        ResMut<Assets<Image>>,
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
    ),
) {
    let _prof = crate::prof::scope("fish::fish_system");
    let lvl = &level.0;
    let dt = time.delta_secs().min(0.1);
    let dtd = dt as f64;
    if !st.init {
        st.init = true;
        st.seed = 0xF15B_0A75;
        for a in &lvl.actors {
            let Some((class, coast_extra)) = fish_class(&a.class) else {
                continue;
            };
            let Some(Value::Vector(l)) = find(&a.props, "Location") else {
                continue;
            };
            let yaw = match find(&a.props, "Rotation") {
                Some(Value::Rotator(r)) => r[1] as f64 * UNR,
                _ => 0.0,
            };
            let pool = crate::critters::pool_count(&a.props);
            let max = (fprop(&a.props, "MaxCount", 0.0) as usize).min(32);
            let radius = fprop(&a.props, "Radius", 70.0);
            let phe = fprop(&a.props, "PawnHomeExtent", 0.0);
            let pos = DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64);
            let home_extent = if bprop(&a.props, "bGeneratorIsHome", false) {
                if phe > 0.0 {
                    phe
                } else {
                    radius
                }
            } else {
                0.0
            };
            let t = 0.25 + st.rand() as f32 * 0.75;
            st.gens.push(Gen {
                class,
                coast_extra,
                pos,
                yaw,
                active_area: fprop(&a.props, "ActiveArea", 1200.0),
                radius,
                home_extent,
                turn_period: fprop(&a.props, "TurnPeriod", 0.8),
                focus: fprop(&a.props, "Focus", 0.6),
                max: if max == 0 { pool } else { max.min(pool.max(1)) },
                pool,
                fish: Vec::new(),
                try_timer: t,
                status_timer: 0.0,
                velocity_timer: 0.0,
                coast_timer: 0.0,
                random_velocity: DVec3::ZERO,
                sum_velocities: DVec3::ZERO,
                flock_center: pos,
            });
        }
        println!("fish: {} fish generators", st.gens.len());
    }

    // Hits: Health 10, no carcass.
    for (e, dmg) in HITS
        .lock()
        .map(|mut h| std::mem::take(&mut *h))
        .unwrap_or_default()
    {
        if let Ok((_, mut f, _)) = fishes.get_mut(e) {
            f.health -= dmg;
            if f.health <= 0.0 {
                commands.entity(e).despawn();
                let gi = f.gen;
                if let Some(g) = st.gens.get_mut(gi) {
                    g.fish.retain(|x| *x != e);
                }
                println!("FISH killed");
            }
        }
    }

    let eye = ue(cam.translation) / dx_level::METERS_PER_UNIT as f64;
    let fwd = ue(cam.forward().as_vec3()).normalize_or_zero();
    let me = player.body.location;

    // Generators.
    let mut to_spawn: Vec<(usize, DVec3, f64, f64)> = Vec::new();
    for gi in 0..st.gens.len() {
        // CheckPawnStatus once a second: transient fish far away and unseen for 5 s go back to the pool.
        st.gens[gi].status_timer += dt;
        if st.gens[gi].status_timer > 1.0 {
            st.gens[gi].status_timer = 0.0;
            let area = st.gens[gi].active_area;
            let list = std::mem::take(&mut st.gens[gi].fish);
            let mut keep = Vec::new();
            for e in list {
                if let Ok((_, f, _)) = fishes.get(e) {
                    if f.pos.distance(me) >= area && f.unseen > 5.0 {
                        commands.entity(e).despawn();
                        st.gens[gi].pool += 1;
                    } else {
                        keep.push(e);
                    }
                }
            }
            st.gens[gi].fish = keep;
        }
        // UpdateSumVelocities every 0.5 s.
        if !st.gens[gi].fish.is_empty() {
            st.gens[gi].velocity_timer += dtd;
            if st.gens[gi].velocity_timer >= 0.5 {
                let mut sum = DVec3::ZERO;
                let mut center = DVec3::ZERO;
                let mut n = 0;
                for &e in &st.gens[gi].fish {
                    if let Ok((_, f, _)) = fishes.get(e) {
                        sum += f.vel;
                        center += f.pos;
                        n += 1;
                    }
                }
                let center = if n > 0 { center / n as f64 } else { st.gens[gi].pos };
                let vt = st.gens[gi].velocity_timer;
                let coast = st.gens[gi].coast_timer;
                let new_coast = coast - vt;
                if new_coast < -st.gens[gi].turn_period {
                    // GenerateCoastPeriod
                    let extra = st.gens[gi].coast_extra;
                    let c = st.rand() * extra + 3.0;
                    st.gens[gi].coast_timer = c;
                    st.gens[gi].random_velocity = DVec3::ZERO;
                } else {
                    if coast >= 0.0 && new_coast < 0.0 {
                        // GenerateRandomVelocity: a random yaw, 1.1 x the school's summed speed.
                        let mag = sum.length().max(0.01);
                        let a = (st.rand() * 32768.0).floor() * 2.0 * UNR;
                        st.gens[gi].random_velocity = DVec3::new(a.cos(), a.sin(), 0.0) * mag * 1.1;
                    }
                    st.gens[gi].coast_timer = new_coast;
                }
                let g = &mut st.gens[gi];
                g.sum_velocities = sum + g.random_velocity;
                g.flock_center = center;
                g.velocity_timer = 0.0;
            }
        }
        // GeneratePawn every 0.25 s while the generator matters.
        let g = &st.gens[gi];
        if g.pos.distance(me) >= g.active_area || g.fish.len() >= g.max || g.pool == 0 {
            continue;
        }
        st.gens[gi].try_timer -= dt;
        if st.gens[gi].try_timer >= 0.0 {
            continue;
        }
        st.gens[gi].try_timer = 0.25;
        let class = st.gens[gi].class;
        let d = assets.classes.defaults(class);
        let cr = fprop(&d, "CollisionRadius", 7.0);
        let ch = fprop(&d, "CollisionHeight", 4.0);
        let (r1, r2, r3) = (st.rand(), st.rand(), st.rand());
        let g = &st.gens[gi];
        let dist = (r1.sqrt() * g.radius).max(cr * 2.0 + 1.0);
        let a = r2 * TAU;
        let p = (r3 - 0.5) * 0.6;
        let want = g.pos + dir_of(a, p) * dist;
        let h = lvl.collision.trace(g.pos, want, DVec3::new(cr, cr, ch));
        let at = g.pos + (want - g.pos) * (h.fraction - 0.02).max(0.0);
        if !swimmable(lvl, at, ch)
            || at.distance(me) >= g.active_area
            || crate::critters::player_sees(lvl, eye, fwd, at)
        {
            continue;
        }
        // RandomBiasedRotation(Rotation.Yaw, Focus, Rotation.Pitch, Focus).
        let (r4, r5) = (st.rand(), st.rand());
        let g = &st.gens[gi];
        let yaw = g.yaw + (r4 * 2.0 - 1.0) * (1.0 - g.focus) * std::f64::consts::PI;
        let pitch = (r5 * 2.0 - 1.0) * (1.0 - g.focus) * 0.25;
        to_spawn.push((gi, at, yaw, pitch));
    }
    for (gi, at, yaw, pitch) in to_spawn {
        let class = st.gens[gi].class;
        let d = assets.classes.defaults(class);
        let Some(Value::Path(mesh_path)) = find(&d, "Mesh").cloned() else {
            continue;
        };
        let root = commands
            .spawn((
                Transform::from_translation(Vec3::from(ue_to_bevy(at.as_vec3().to_array()))),
                Visibility::default(),
            ))
            .id();
        let map = |p: [f32; 3]| ue_to_bevy(p);
        let Some((m, parts, handles)) = spawn_mesh_parts(
            &mesh_path,
            fprop(&d, "DrawScale", 1.0) as f32,
            &d,
            &mut assets,
            &mut images,
            &mut meshes,
            &mut materials,
            &mut commands,
            root,
            &map,
            None,
        ) else {
            commands.entity(root).despawn();
            continue;
        };
        let am = Arc::new(anim_mesh(&m));
        let mut clip = Clip::named(&am, "Swim", true).unwrap_or(Clip {
            start: 0,
            frames: 1,
            rate: 1.0,
            looping: true,
            t: 0.0,
        });
        clip.t = st.rand() as f32 * 2.0;
        let radius = fprop(&d, "CollisionRadius", 7.0);
        let leader = st.rand() * 10.0 + 5.0;
        let f = Fish {
            gen: gi,
            pos: at,
            vel: DVec3::ZERO,
            yaw,
            pitch,
            phase: Phase::Start(at + dir_of(yaw, pitch) * (radius + 5.0)),
            leader_timer: leader,
            forward_timer: -1.0,
            bump_timer: 0.0,
            abort_timer: 0.0,
            unseen: 0.0,
            radius,
            height: fprop(&d, "CollisionHeight", 4.0),
            water_speed: fprop(&d, "WaterSpeed", 50.0),
            accel_rate: fprop(&d, "AccelRate", 500.0),
            health: fprop(&d, "Health", 10.0) as f32,
            mesh: am,
            parts,
            handles,
            clip,
            lit_at: DVec3::splat(1.0e9),
        };
        commands.entity(root).insert(f);
        let g = &mut st.gens[gi];
        g.fish.push(root);
        g.pool -= 1;
    }

    // Per-generator flock data for PickDirection.
    let flock: Vec<(DVec3, DVec3, f64, DVec3, f64)> = st
        .gens
        .iter()
        .map(|g| (g.sum_velocities, g.flock_center, g.radius, g.pos, g.home_extent))
        .collect();

    // RotationRate (6000, 25000, 3072): pitch and yaw rates.
    let yaw_rate = 25000.0 * UNR;
    let pitch_rate = 6000.0 * UNR;
    for (_, mut f, mut tf) in &mut fishes {
        let f = &mut *f;
        let mut rnd = || st.rand();
        f.leader_timer -= dtd;
        f.forward_timer -= dtd;
        f.bump_timer = (f.bump_timer - dtd).max(0.0);
        if f.leader_timer < -2.0 {
            f.leader_timer = rnd() * 10.0 + 5.0;
        }
        let (sumv, center, gradius, home, hextent) = flock[f.gen];
        let near_home = hextent <= 0.0 || home.distance(f.pos) <= hextent;
        let pick = |f: &Fish, forward: bool, rnd: &mut dyn FnMut() -> f64| -> DVec3 {
            let mut cum = if forward || near_home {
                f.vel
            } else {
                (home - f.pos) * 20.0
            };
            if f.leader_timer > 0.0 && !forward {
                cum += sumv - f.vel;
                let cv = center - f.pos;
                let dist = cv.length();
                if dist > gradius && dist < gradius * 4.0 {
                    cum += cv * 2.0;
                }
            }
            if cum.length_squared() < 1e-6 {
                cum = dir_of(f.yaw, f.pitch);
            }
            let (mut y, mut p) = rot_of(cum);
            if !forward {
                if f.leader_timer > 1.2 {
                    y += ((rnd() * 8192.0).floor() - 4096.0) * UNR;
                    p += ((rnd() * 3000.0).floor() - 1500.0) * UNR;
                }
                f.pos + dir_of(y, p) * 200.0
            } else {
                f.pos + dir_of(y, p) * 50.0
            }
        };
        // The latent Moving loop.
        let target = match f.phase {
            Phase::Start(t) | Phase::Dart(t) | Phase::Turn(t) => t,
        };
        f.abort_timer += dtd;
        let to = target - f.pos;
        let (want_yaw, want_pitch) = rot_of(to);
        let want_pitch = want_pitch.clamp(-0.5, 0.5);
        let dy = wrap(want_yaw - f.yaw);
        let dp = want_pitch - f.pitch;
        f.yaw = wrap(f.yaw + dy.clamp(-yaw_rate * dtd, yaw_rate * dtd));
        f.pitch += dp.clamp(-pitch_rate * dtd, pitch_rate * dtd);
        let next_phase = match f.phase {
            Phase::Start(_) | Phase::Dart(_) => {
                if to.length() < f.radius.max(8.0) || f.abort_timer > 8.0 {
                    if matches!(f.phase, Phase::Dart(_)) {
                        // ResetForwardTimer
                        f.forward_timer = rnd() * 10.0 + 2.0;
                    }
                    None
                } else {
                    Some(f.phase)
                }
            }
            Phase::Turn(_) => {
                // TurnTo is done once facing the focus (within ~1/16 turn); Sleep(0) then pick again.
                if (dy.abs() < 0.05 && dp.abs() < 0.05) || f.abort_timer > 8.0 {
                    None
                } else {
                    Some(f.phase)
                }
            }
        };
        f.phase = match next_phase {
            Some(p) => p,
            None => {
                f.abort_timer = 0.0;
                if f.forward_timer < 0.0 {
                    Phase::Dart(pick(f, true, &mut rnd))
                } else {
                    Phase::Turn(pick(f, false, &mut rnd))
                }
            }
        };

        // PHYS_Swimming: accelerate the way it faces, capped at WaterSpeed, with a little fluid drag.
        let facing = dir_of(f.yaw, f.pitch);
        f.vel += facing * f.accel_rate * 0.3 * dtd;
        f.vel *= (1.0 - 1.0 * dtd).max(0.0);
        let speed = f.vel.length();
        if speed > f.water_speed {
            f.vel *= f.water_speed / speed;
        }
        let next = f.pos + f.vel * dtd;
        let h = lvl
            .collision
            .trace(f.pos, next, DVec3::new(f.radius, f.radius, f.height));
        let mut normal = None;
        if h.fraction < 1.0 {
            f.pos += (next - f.pos) * (h.fraction - 0.05).max(0.0);
            normal = Some(h.normal);
        } else if !swimmable(lvl, next, f.height) {
            // The water's edge counts as a wall (they never leave the water).
            let up = crate::critters::in_water(lvl, next);
            normal = Some(if up {
                DVec3::new(0.0, 0.0, -1.0)
            } else {
                -f.vel.normalize_or_zero()
            });
        } else {
            f.pos = next;
        }
        if let Some(n) = normal {
            if f.bump_timer <= 0.0 {
                // Wandering.HitWall (swimming): reflect, face the new way, dart again.
                f.bump_timer = 0.5;
                let v = f.vel.dot(n) * n * -2.0 + f.vel;
                let v = if v.length_squared() < 1e-6 { -facing * f.water_speed * 0.5 } else { v };
                f.vel = v;
                let (y, p) = rot_of(v);
                f.yaw = y;
                f.pitch = p.clamp(-0.5, 0.5);
                f.forward_timer = -1.0;
                f.abort_timer = 0.0;
                f.phase = Phase::Dart(pick(f, true, &mut rnd));
            }
        }

        // Seen within the last 5 s (LastRendered)?
        let seen = f.pos.distance(eye) < 3000.0 && crate::critters::player_sees(lvl, eye, fwd, f.pos);
        f.unseen = if seen { 0.0 } else { f.unseen + dt };

        let rot = [
            (f.pitch / UNR) as i32,
            (f.yaw / UNR) as i32,
            0,
        ];
        let m = rotator_to_bevy_mat3(rot);
        tf.translation = Vec3::from(ue_to_bevy(f.pos.as_vec3().to_array()));
        tf.rotation = Quat::from_mat3(&Mat3::from_cols(
            Vec3::from(m[0]),
            Vec3::from(m[1]),
            Vec3::from(m[2]),
        ));
        // LoopAnimPivot('Swim'); skip the vertex work for fish nobody can see.
        f.clip.t += dt;
        if f.unseen < 0.5 {
            let fr = f.clip.frame();
            for (pi, hm) in f.handles.iter().enumerate() {
                if let Some(mut mm) = meshes.get_mut(hm) {
                    let pos: Vec<[f32; 3]> = f.parts[pi]
                        .vert_ids
                        .iter()
                        .map(|&v| ue_to_bevy(f.mesh.sample(f.clip.start, f.clip.frames, fr, v as usize)))
                        .collect();
                    mm.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
                }
            }
            if f.pos.distance(f.lit_at) > 32.0 {
                f.lit_at = f.pos;
                for (pi, hm) in f.handles.iter().enumerate() {
                    if let Some(mut mm) = meshes.get_mut(hm) {
                        mm.insert_attribute(
                            Mesh::ATTRIBUTE_COLOR,
                            light_part(&f.parts[pi], f.pos.as_vec3().to_array(), rot, 1.0, &lvl.lights),
                        );
                    }
                }
            }
        }
    }
    if let Ok(mut t) = TARGETS.lock() {
        t.clear();
        for (e, f, _) in &fishes {
            t.push((e, f.pos, f.radius, f.height));
        }
    }
}

/// A new map is going in (loader.rs): this module's state of the map being left.
pub fn reset_level() {
    if let Ok(mut v) = TARGETS.lock() {
        v.clear();
    }
    if let Ok(mut v) = HITS.lock() {
        v.clear();
    }
}
