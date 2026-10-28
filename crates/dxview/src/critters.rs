//! Ambient birds: PawnGenerator (PigeonGenerator, SeagullGenerator) and Bird (Pigeon, Seagull).
//!
//! * A generator keeps up to MaxCount birds alive while the player is within ActiveArea, adding one every 0.25 s at
//!   a random spot within Radius of the ground below it, only where the player can't see it appear. Birds are
//!   transient: once out of ActiveArea and out of sight they are removed and go back into the pool.
//! * On the ground a bird wanders (Animal/ScriptedPawn state Wandering): it walks ~100-130 units at WalkingSpeed,
//!   stops for 6 x (1 - 0.9 x Restlessness) s on Idle1 or Idle2, and every half second has a 10% chance to coo
//!   (PigeonCoo / SeagullCry).
//! * Every half second it looks for a pawn within 500 units heading its way (Animal.ShouldBeStartled: moving at 20
//!   or more and due to pass within 0.6 x its speed in the next 2 s) and listens for LoudNoise. Either one frightens
//!   it (fright = Cowardice x 99 + 1 s) and it takes off (state Flying, PigeonFly / SeagullFly sound).
//! * Flying: climb out at ~55 degrees for ~300 units, then fly ~1200-unit legs at AirSpeed in random reachable
//!   directions, bouncing off walls, until it's no longer frightened and a LikesFlying roll says land; it then
//!   picks ground within 100 units, glides down and starts wandering again.

use crate::ai::Npc;
use crate::weapons::{spawn_mesh_parts, Clip, RuntimeAssets};
use crate::{FlyCam, LevelRes, Player, SkyCam};
use bevy::prelude::*;
use dx_level::actors::{anim_mesh, light_part, rotator_to_bevy_mat3, AnimMesh, MeshPart};
use dx_level::ue_to_bevy;
use glam::DVec3;
use std::sync::Arc;
use unreal_pkg::props::{find, Property, Value};

/// Birds that cast a blob shadow (bHasShadow, ShadowScale 0.5): entity, centre, CollisionRadius, CollisionHeight.
pub fn shadow_casters() -> Vec<(Entity, DVec3, f64, f64)> {
    TARGETS.lock().map(|t| t.clone()).unwrap_or_default()
}

/// Bird collision cylinders this frame (entity, centre, radius, height), for weapon traces.
static TARGETS: std::sync::Mutex<Vec<(Entity, DVec3, f64, f64)>> =
    std::sync::Mutex::new(Vec::new());
/// Damage dealt to birds since the last critter update (entity, damage).
static HITS: std::sync::Mutex<Vec<(Entity, f32)>> = std::sync::Mutex::new(Vec::new());

/// The nearest bird on the line from `start` to `end`: (entity, fraction along the line).
pub fn trace_birds(start: DVec3, end: DVec3) -> Option<(Entity, f64)> {
    // Birds and fish (fish::TARGETS) share the small-animal trace.
    let mut list = TARGETS.lock().ok()?.clone();
    if let Ok(f) = crate::fish::TARGETS.lock() {
        list.extend(f.iter().copied());
    }
    let mut best: Option<(Entity, f64)> = None;
    for &(e, c, r, h) in list.iter() {
        // A few units of slack: birds are small and fast, and the original traces against their mesh.
        let cyl = dx_level::collision::Cylinder {
            center: c,
            radius: r + 4.0,
            height: h + 4.0,
            actor: usize::MAX,
        };
        if let Some(hit) = dx_level::collision::trace_cylinder(&cyl, start, end, DVec3::splat(0.5))
        {
            if best.map_or(true, |b| hit.fraction < b.1) {
                best = Some((e, hit.fraction));
            }
        }
    }
    best
}

/// Audible radius of a coo / cry. PlaySound's engine default (~1500) made every gull on the island audible from the
/// dock and, on top of the level's own gull AmbientSounds, they sounded constant; a pawn's WorldSoundRadius-scale
/// radius keeps them local, as they sound in the original.
const CRY_RADIUS: f32 = 640.0;

/// HurtRadius: every bird inside the blast takes damage falling off with distance.
pub fn blast_birds(center: DVec3, radius: f64, damage: f32) {
    let mut list = TARGETS.lock().map(|t| t.clone()).unwrap_or_default();
    if let Ok(f) = crate::fish::TARGETS.lock() {
        list.extend(f.iter().copied());
    }
    for (e, c, _, _) in list {
        let d = c.distance(center);
        if d < radius {
            hit_bird(e, damage * (1.0 - (d / radius) as f32).max(0.0) + 1.0);
        }
    }
}

/// A bird was hit (Pawn.TakeDamage on a Bird); handled at the next critter update.
pub fn hit_bird(e: Entity, damage: f32) {
    if crate::fish::is_fish(e) {
        if let Ok(mut h) = crate::fish::HITS.lock() {
            h.push((e, damage));
        }
        return;
    }
    if let Ok(mut h) = HITS.lock() {
        h.push((e, damage));
    }
}

struct Generator {
    class: String,
    pos: DVec3,
    ground: DVec3,
    active_area: f64,
    radius: f64,
    max: usize,
    pool: usize,
    birds: Vec<Entity>,
    try_timer: f32,
    status_timer: f32,
}

#[derive(Resource, Default)]
pub struct Critters {
    init: bool,
    gens: Vec<Generator>,
    seed: u64,
}

impl Critters {
    fn rand(&mut self) -> f64 {
        self.seed = self
            .seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.seed >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Walking,
    Pausing(f32),
    /// Climbing away from the ground (PickInitialDestination).
    TakeOff,
    Flying,
    /// Heading for the landing spot, then dropping onto it.
    Landing,
    Falling,
}

#[derive(Component)]
pub struct Bird {
    gen: usize,
    pigeon: bool,
    pos: DVec3,
    vel: DVec3,
    yaw: f64,
    pitch: f64,
    dest: DVec3,
    mode: Mode,
    fright: f32,
    check_timer: f32,
    coo_timer: f32,
    /// Seconds since the player last had this bird in view (UE1 bStasis: pawns nobody has rendered for a while
    /// stop ticking, so off-screen birds stay quiet).
    unseen: f32,
    hit_timer: f32,
    stuck: f32,
    height: f64,
    walk_speed: f64,
    air_speed: f64,
    likes_flying: f64,
    cowardice: f32,
    restlessness: f32,
    wanderlust: f64,
    mesh: Arc<AnimMesh>,
    parts: Vec<MeshPart>,
    handles: Vec<Handle<Mesh>>,
    clip: Clip,
    clip_name: &'static str,
    anim_rate: f32,
    lit_at: DVec3,
    radius: f64,
    health: f32,
    class: &'static str,
}

fn fprop(p: &[Property], n: &str, d: f64) -> f64 {
    match find(p, n) {
        Some(Value::Float(v)) => *v as f64,
        Some(Value::Int(v)) => *v as f64,
        Some(Value::Byte(v)) => *v as f64,
        _ => d,
    }
}

/// The generator's first PawnTypes entry: Count is its first int.
pub fn pool_count(p: &[Property]) -> usize {
    match find(p, "PawnClasses") {
        Some(Value::Struct(_, b)) if b.len() >= 4 => {
            i32::from_le_bytes([b[0], b[1], b[2], b[3]]).max(0) as usize
        }
        _ => 0,
    }
}

fn pawn_class_for(generator: &str) -> Option<&'static str> {
    match generator {
        "PigeonGenerator" => Some("Pigeon"),
        "SeagullGenerator" => Some("Seagull"),
        _ => None,
    }
}

fn ue(v: Vec3) -> DVec3 {
    DVec3::new(v.x as f64, v.z as f64, v.y as f64)
}

/// Can the player see this point (in front of the camera and not behind a wall)?
pub fn player_sees(level: &dx_level::Level, eye: DVec3, fwd: DVec3, p: DVec3) -> bool {
    let d = p - eye;
    let len = d.length();
    if len < 1.0 {
        return true;
    }
    if d.dot(fwd) / len < 0.5 {
        return false;
    }
    level.collision.trace(eye, p, DVec3::ZERO).fraction >= 0.999
}

/// Ground below a point (Unreal units), if any within `down`.
pub fn ground_below(level: &dx_level::Level, p: DVec3, down: f64) -> Option<(DVec3, DVec3)> {
    let h = level
        .collision
        .trace(p, p - DVec3::new(0.0, 0.0, down), DVec3::splat(1.0));
    (h.fraction < 1.0 && h.normal.z > 0.6)
        .then(|| (p - DVec3::new(0.0, 0.0, down * h.fraction), h.normal))
}

pub fn in_water(level: &dx_level::Level, p: DVec3) -> bool {
    matches!(
        level
            .zone_water
            .get(level.collision.zone_at(p).max(0) as usize),
        Some(Some(_))
    )
}

#[allow(clippy::too_many_arguments)]
pub fn critter_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    player: Res<Player>,
    cam: Single<&Transform, (With<FlyCam>, Without<SkyCam>, Without<Bird>)>,
    npcs: Query<&Npc>,
    noises: Res<crate::stealth::Noises>,
    mut st: ResMut<Critters>,
    mut birds: Query<(Entity, &mut Bird, &mut Transform), Without<FlyCam>>,
    (mut assets, mut images, mut meshes, mut materials): (
        ResMut<RuntimeAssets>,
        ResMut<Assets<Image>>,
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
    ),
) {
    let _prof = crate::prof::scope("critters::critter_system");
    let lvl = &level.0;
    let dt = time.delta_secs().min(0.1);
    if !st.init {
        st.init = true;
        st.seed = 0x5EED_B1D5;
        for a in &lvl.actors {
            if pawn_class_for(&a.class).is_none() {
                continue;
            }
            let Some(Value::Vector(l)) = find(&a.props, "Location") else {
                continue;
            };
            let pos = DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64);
            let ground = ground_below(lvl, pos, 200.0).map(|g| g.0).unwrap_or(pos);
            let pool = pool_count(&a.props);
            let max = (fprop(&a.props, "MaxCount", 0.0) as usize).min(32);
            st.gens.push(Generator {
                class: a.class.clone(),
                pos,
                ground,
                active_area: fprop(&a.props, "ActiveArea", 2000.0),
                radius: fprop(&a.props, "Radius", 600.0),
                max: if max == 0 { pool } else { max },
                pool,
                birds: Vec::new(),
                try_timer: 0.25,
                status_timer: 0.0,
            });
        }
        println!("critters: {} bird generators", st.gens.len());
    }
    // Shot birds: no death animation (bPlayDying False), the carcass drops where it was (PigeonCarcass /
    // SeagullCarcass) with the Die sound.
    for (e, dmg) in HITS
        .lock()
        .map(|mut h| std::mem::take(&mut *h))
        .unwrap_or_default()
    {
        let Ok((_, mut b, _)) = birds.get_mut(e) else {
            continue;
        };
        b.health -= dmg;
        b.fright = b.fright.max(b.cowardice * 99.0 + 1.0);
        if b.health > 0.0 {
            continue;
        }
        let (pos, class) = (b.pos, b.class);
        commands.entity(e).despawn();
        let d = assets.classes.defaults(class);
        if let Some(snd) = crate::sound::prop(&d, "Die") {
            crate::sound::play(&snd, pos, 1.0, 1024.0, 1.0);
        }
        let cd = assets.classes.defaults(&format!("{class}Carcass"));
        if let Some(Value::Path(mesh)) = find(&cd, "Mesh").cloned() {
            let loose = crate::destruct::Loose::dropped(pos, fprop(&cd, "CollisionHeight", 3.0));
            crate::destruct::spawn_loose(
                &mut commands,
                &mut assets,
                &mut images,
                &mut meshes,
                &mut materials,
                lvl,
                &mesh,
                &cd,
                fprop(&cd, "DrawScale", 1.0) as f32,
                loose,
            );
        }
        println!("BIRD {class} killed");
    }
    let eye = ue(cam.translation) / dx_level::METERS_PER_UNIT as f64;
    let fwd = ue(cam.forward().as_vec3()).normalize_or_zero();
    let me = player.body.location;

    // Generators: transient clean-up once a second, a new bird every 0.25 s while there's room.
    let mut to_spawn: Vec<(usize, DVec3)> = Vec::new();
    for gi in 0..st.gens.len() {
        st.gens[gi].status_timer += dt;
        if st.gens[gi].status_timer > 1.0 {
            st.gens[gi].status_timer = 0.0;
            let area = st.gens[gi].active_area;
            let list = std::mem::take(&mut st.gens[gi].birds);
            let mut keep = Vec::new();
            for e in list {
                match birds.get(e) {
                    Ok((_, b, _)) => {
                        if b.pos.distance(me) >= area && !player_sees(lvl, eye, fwd, b.pos) {
                            commands.entity(e).despawn();
                            st.gens[gi].pool += 1;
                        } else {
                            keep.push(e);
                        }
                    }
                    // Killed: it doesn't go back into the pool.
                    Err(_) => {}
                }
            }
            st.gens[gi].birds = keep;
        }
        let g = &st.gens[gi];
        if g.pos.distance(me) >= g.active_area || g.birds.len() >= g.max || g.pool == 0 {
            continue;
        }
        st.gens[gi].try_timer -= dt;
        if st.gens[gi].try_timer >= 0.0 {
            continue;
        }
        st.gens[gi].try_timer = 0.25;
        // GeneratePawn: a random reachable spot within Radius (sqrt for an even spread), hidden from the player.
        let r = st.rand();
        let a = st.rand() * std::f64::consts::TAU;
        let g = &st.gens[gi];
        let dist = r.sqrt() * g.radius;
        let start = g.ground + DVec3::new(0.0, 0.0, 20.0);
        let want = start + DVec3::new(a.cos(), a.sin(), 0.0) * dist;
        let h = lvl.collision.trace(start, want, DVec3::new(8.0, 8.0, 4.0));
        let at = start + (want - start) * (h.fraction - 0.02).max(0.0);
        let Some((floor, _)) = ground_below(lvl, at, 120.0) else {
            continue;
        };
        if in_water(lvl, floor + DVec3::new(0.0, 0.0, 4.0))
            || floor.distance(me) >= g.active_area
            || player_sees(lvl, eye, fwd, floor + DVec3::new(0.0, 0.0, 6.0))
        {
            continue;
        }
        to_spawn.push((gi, floor));
    }
    for (gi, floor) in to_spawn {
        let class = pawn_class_for(&st.gens[gi].class).unwrap_or("Pigeon");
        let d = assets.classes.defaults(class);
        let Some(Value::Path(mesh_path)) = find(&d, "Mesh") else {
            continue;
        };
        let mesh_path = mesh_path.clone();
        let height = fprop(&d, "CollisionHeight", 4.0);
        let pos = floor + DVec3::new(0.0, 0.0, height);
        let yaw = st.rand() * std::f64::consts::TAU;
        let root = commands
            .spawn((
                Transform::from_translation(Vec3::from(ue_to_bevy(pos.as_vec3().to_array()))),
                Visibility::default(),
            ))
            .id();
        let map = |p: [f32; 3]| ue_to_bevy(p);
        let Some((m, parts, handles)) = spawn_mesh_parts(
            &mesh_path,
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
        ) else {
            commands.entity(root).despawn();
            continue;
        };
        let am = Arc::new(anim_mesh(&m));
        let clip = Clip::named(&am, "Idle1", true).unwrap_or(Clip {
            start: 0,
            frames: 1,
            rate: 1.0,
            looping: true,
            t: 0.0,
        });
        let ground_speed = fprop(&d, "GroundSpeed", 24.0);
        let b = Bird {
            gen: gi,
            pigeon: class == "Pigeon",
            pos,
            vel: DVec3::ZERO,
            yaw,
            pitch: 0.0,
            dest: pos,
            mode: Mode::Pausing(0.5),
            fright: 0.0,
            check_timer: st.rand() as f32 * 0.5,
            coo_timer: st.rand() as f32 * 0.5,
            unseen: 0.0,
            hit_timer: 0.0,
            stuck: 0.0,
            height,
            walk_speed: ground_speed * fprop(&d, "WalkingSpeed", 0.4),
            air_speed: fprop(&d, "AirSpeed", 150.0),
            likes_flying: fprop(&d, "LikesFlying", 0.25),
            cowardice: fprop(&d, "Cowardice", 0.2) as f32,
            restlessness: fprop(&d, "Restlessness", 1.0) as f32,
            wanderlust: fprop(&d, "Wanderlust", 0.05),
            mesh: am,
            parts,
            handles,
            clip,
            clip_name: "Idle1",
            anim_rate: 1.0,
            lit_at: DVec3::splat(1.0e9),
            radius: fprop(&d, "CollisionRadius", 10.0),
            health: fprop(&d, "Health", 20.0) as f32,
            class: if class == "Pigeon" {
                "Pigeon"
            } else {
                "Seagull"
            },
        };
        commands.entity(root).insert(b);
        let g = &mut st.gens[gi];
        g.birds.push(root);
        g.pool -= 1;
    }

    // Who could startle a bird: the player and every live NPC (position, velocity).
    let mut movers: Vec<(DVec3, DVec3)> = vec![(me, player.body.velocity)];
    for n in &npcs {
        if let Some(b) = n.body.as_ref() {
            if n.state != crate::ai::NpcState::Dead {
                movers.push((b.location, b.velocity));
            }
        }
    }
    let loud: Vec<&crate::stealth::Noise> = noises
        .0
        .iter()
        .filter(|n| {
            matches!(
                n.kind,
                crate::stealth::NoiseKind::Loud | crate::stealth::NoiseKind::WeaponFire
            )
        })
        .collect();

    for (e, mut b, mut tf) in &mut birds {
        let b = &mut *b;
        let mut rnd = || st.rand();
        b.fright = (b.fright - dt).max(0.0);
        // Startle checks (Animal.Tick in Wandering, Bird.ReadyToLand in flight).
        b.check_timer += dt;
        let mut startled = false;
        if b.check_timer > 0.5 {
            b.check_timer = 0.0;
            for &(p, v) in &movers {
                let speed = v.length();
                if speed < 20.0 || p.distance(b.pos) > 500.0 {
                    continue;
                }
                let t = p.distance(b.pos) / speed;
                if t <= 2.0 && b.pos.distance(p + v * t) < speed * 0.6 {
                    startled = true;
                    break;
                }
            }
        }
        if loud
            .iter()
            .any(|n| crate::stealth::hear(&level.0, b.pos, n) >= 0.15)
        {
            startled = true;
        }
        let grounded = matches!(b.mode, Mode::Walking | Mode::Pausing(_));
        if startled {
            b.fright = b.cowardice * 99.0 + 1.0;
            if grounded {
                // Flying.BeginState: the flutter of wings, then PickInitialDestination.
                let snd = if b.pigeon {
                    "DeusExSounds.Animal.PigeonFly"
                } else {
                    "DeusExSounds.Animal.SeagullFly"
                };
                crate::sound::play_owned(
                    e.to_bits(),
                    crate::sound::slot::MISC,
                    snd,
                    Some(b.pos),
                    2.0,
                    0.0,
                    1.0,
                );
                let pitch = (10000.0 + (rnd() * 6000.0 - 3000.0)) / 65536.0 * std::f64::consts::TAU;
                let dir = DVec3::new(
                    b.yaw.cos() * pitch.cos(),
                    b.yaw.sin() * pitch.cos(),
                    pitch.sin(),
                );
                let mag = 300.0 + rnd() * 100.0 - 50.0;
                let h = lvl
                    .collision
                    .trace(b.pos, b.pos + dir * mag, DVec3::splat(4.0));
                b.dest = if h.fraction > 0.1 {
                    b.pos + dir * mag * (h.fraction - 0.05).max(0.0)
                } else {
                    b.pos + DVec3::new(0.0, 0.0, 100.0)
                };
                b.mode = Mode::TakeOff;
                b.stuck = 0.0;
            }
        }

        match b.mode {
            Mode::Pausing(t) => {
                b.vel = DVec3::ZERO;
                let t = t - dt;
                if t <= 0.0 {
                    // PickDestination: (wanderlust x 300 + 100) +-10% in a random direction, if the way is clear.
                    let mag = (b.wanderlust * 300.0 + 100.0) * (rnd() * 0.2 + 0.9);
                    let a = rnd() * std::f64::consts::TAU;
                    let want = b.pos + DVec3::new(a.cos(), a.sin(), 0.0) * mag;
                    let h = lvl.collision.trace(b.pos, want, DVec3::new(4.0, 4.0, 2.0));
                    if h.fraction > 0.3 {
                        b.dest = b.pos + (want - b.pos) * (h.fraction - 0.05);
                        b.mode = Mode::Walking;
                    } else {
                        b.mode = Mode::Pausing(0.5);
                    }
                } else {
                    b.mode = Mode::Pausing(t);
                }
                // Wandering.Tick: every half second, a 10% chance of a coo (or a gull's cry).
                b.coo_timer += dt;
                if b.coo_timer > 0.5 {
                    b.coo_timer = 0.0;
                    b.unseen = if player_sees(lvl, eye, fwd, b.pos) {
                        0.0
                    } else {
                        b.unseen + 0.5
                    };
                    if b.unseen < 5.0 && rnd() < 0.1 {
                        let snd = if b.pigeon {
                            "DeusExSounds.Animal.PigeonCoo"
                        } else {
                            "DeusExSounds.Animal.SeagullCry"
                        };
                        crate::sound::play_owned(
                            e.to_bits(),
                            crate::sound::slot::MISC,
                            snd,
                            Some(b.pos),
                            2.0,
                            CRY_RADIUS,
                            1.0,
                        );
                    }
                }
            }
            Mode::Walking => {
                let to = DVec3::new(b.dest.x - b.pos.x, b.dest.y - b.pos.y, 0.0);
                let step = b.walk_speed * dt as f64;
                if to.length() <= step.max(1.0) {
                    // Pausing: Sleep(6 x (1 - 0.9 x Restlessness)) on an idle loop.
                    b.mode = Mode::Pausing(
                        6.0 * (1.0 - 0.9 * b.restlessness).max(0.1) + rnd() as f32 * 0.5,
                    );
                } else {
                    let dir = to.normalize();
                    b.yaw = dir.y.atan2(dir.x);
                    let next = b.pos + dir * step;
                    match ground_below(lvl, next + DVec3::new(0.0, 0.0, 8.0), b.height + 24.0) {
                        Some((floor, _))
                            if (floor.z + b.height - b.pos.z).abs() < 12.0
                                && !in_water(lvl, floor + DVec3::new(0.0, 0.0, 2.0)) =>
                        {
                            b.pos = floor + DVec3::new(0.0, 0.0, b.height);
                            b.vel = dir * b.walk_speed;
                        }
                        _ => b.mode = Mode::Pausing(0.6),
                    }
                }
                b.coo_timer += dt;
                if b.coo_timer > 0.5 {
                    b.coo_timer = 0.0;
                    b.unseen = if player_sees(lvl, eye, fwd, b.pos) {
                        0.0
                    } else {
                        b.unseen + 0.5
                    };
                    if b.unseen < 5.0 && rnd() < 0.1 {
                        let snd = if b.pigeon {
                            "DeusExSounds.Animal.PigeonCoo"
                        } else {
                            "DeusExSounds.Animal.SeagullCry"
                        };
                        crate::sound::play_owned(
                            e.to_bits(),
                            crate::sound::slot::MISC,
                            snd,
                            Some(b.pos),
                            2.0,
                            CRY_RADIUS,
                            1.0,
                        );
                    }
                }
            }
            Mode::TakeOff | Mode::Flying | Mode::Landing => {
                b.stuck += dt;
                b.hit_timer = (b.hit_timer - dt).max(0.0);
                let to = b.dest - b.pos;
                let arrived = to.length() < 12.0;
                if arrived || (b.stuck > 10.0 && b.mode != Mode::Landing) {
                    match b.mode {
                        Mode::Landing => {
                            b.mode = Mode::Falling;
                        }
                        _ => {
                            // Fly: ReadyToLand? (not frightened, nobody close, and a LikesFlying roll), else another leg.
                            if b.fright <= 0.0 && rnd() > b.likes_flying {
                                if let Some(spot) = pick_landing(lvl, b.pos, &mut rnd) {
                                    b.dest = spot;
                                    b.mode = Mode::Landing;
                                } else {
                                    b.dest = pick_leg(lvl, b.pos, b.vel, b.stuck, &mut rnd);
                                    b.mode = Mode::Flying;
                                }
                            } else {
                                b.dest = pick_leg(lvl, b.pos, b.vel, b.stuck, &mut rnd);
                                b.mode = Mode::Flying;
                                b.stuck = 0.0;
                            }
                        }
                    }
                } else {
                    // MoveTo at AirSpeed, turning at RotationRate (yaw 50000/s, pitch 6000/s).
                    let want = to.normalize();
                    let cur = if b.vel.length_squared() > 1.0 {
                        b.vel.normalize()
                    } else {
                        want
                    };
                    let max_turn = 50000.0 / 65536.0 * std::f64::consts::TAU * dt as f64;
                    let ang = cur.dot(want).clamp(-1.0, 1.0).acos();
                    let dir = if ang <= max_turn || ang < 1e-6 {
                        want
                    } else {
                        let axis = cur.cross(want);
                        if axis.length_squared() < 1e-9 {
                            want
                        } else {
                            glam::DQuat::from_axis_angle(axis.normalize(), max_turn) * cur
                        }
                    };
                    b.vel = dir * b.air_speed;
                    let next = b.pos + b.vel * dt as f64;
                    let h = lvl.collision.trace(b.pos, next, DVec3::splat(3.0));
                    if h.fraction < 1.0 {
                        b.pos += (next - b.pos) * (h.fraction - 0.05).max(0.0);
                        if b.hit_timer <= 0.0 {
                            // Flying.HitWall: reflect, and head 80 units the new way.
                            b.hit_timer = 0.5;
                            let n = h.normal;
                            let v = b.vel.dot(n) * n * -2.0 + b.vel;
                            b.vel = if v.length() < 0.01 {
                                DVec3::new(b.yaw.cos(), b.yaw.sin(), 0.0) * b.air_speed
                            } else {
                                v
                            };
                            if b.mode == Mode::Landing {
                                b.mode = Mode::Falling;
                            } else {
                                b.dest = b.pos + b.vel.normalize() * 80.0;
                                b.mode = Mode::Flying;
                            }
                        }
                    } else {
                        b.pos = next;
                    }
                    b.yaw = b.vel.y.atan2(b.vel.x);
                    let flat = DVec3::new(b.vel.x, b.vel.y, 0.0).length();
                    let target_pitch = b.vel.z.atan2(flat).clamp(-0.6, 0.6);
                    let pmax = 6000.0 / 65536.0 * std::f64::consts::TAU * dt as f64;
                    b.pitch += (target_pitch - b.pitch).clamp(-pmax, pmax);
                    // Flap faster climbing, slower diving.
                    b.anim_rate = ((b.vel.z / b.air_speed * 250.0 + 250.0).clamp(0.0, 500.0)
                        / 500.0
                        + 0.5) as f32;
                }
            }
            Mode::Falling => {
                b.vel.x *= (1.0 - 2.0 * dt as f64).max(0.0);
                b.vel.y *= (1.0 - 2.0 * dt as f64).max(0.0);
                b.vel.z = (b.vel.z - 300.0 * dt as f64).max(-200.0);
                let next = b.pos + b.vel * dt as f64;
                let h = lvl
                    .collision
                    .trace(b.pos, next, DVec3::new(3.0, 3.0, b.height));
                if h.fraction < 1.0 {
                    b.pos += (next - b.pos) * (h.fraction - 0.02).max(0.0);
                    if h.normal.z > 0.6 {
                        b.vel = DVec3::ZERO;
                        b.pitch = 0.0;
                        b.mode = Mode::Pausing(0.5);
                    } else {
                        b.vel = DVec3::new(0.0, 0.0, b.vel.z);
                    }
                } else {
                    b.pos = next;
                }
                b.pitch *= 0.9;
                b.anim_rate = 0.1;
            }
        }

        // Animation: Walk while walking, an idle loop while pausing, Fly in the air.
        let want_anim: &'static str = match b.mode {
            Mode::Walking => "Walk",
            Mode::Pausing(_) => {
                if matches!(b.clip_name, "Idle1" | "Idle2") {
                    b.clip_name
                } else if rnd() < 0.5 {
                    "Idle1"
                } else {
                    "Idle2"
                }
            }
            _ => "Fly",
        };
        if want_anim != b.clip_name {
            if let Some(c) = Clip::named(&b.mesh, want_anim, true) {
                b.clip = c;
                b.clip_name = want_anim;
            }
        }
        let rate = if matches!(b.mode, Mode::Walking | Mode::Pausing(_)) {
            1.0
        } else {
            b.anim_rate
        };
        b.clip.t += dt * rate;
        let f = b.clip.frame();
        for (pi, hm) in b.handles.iter().enumerate() {
            if let Some(mut mm) = meshes.get_mut(hm) {
                let pos: Vec<[f32; 3]> = b.parts[pi]
                    .vert_ids
                    .iter()
                    .map(|&v| ue_to_bevy(b.mesh.sample(b.clip.start, b.clip.frames, f, v as usize)))
                    .collect();
                mm.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
            }
        }
        let rot = [
            (b.pitch / std::f64::consts::TAU * 65536.0) as i32,
            (b.yaw / std::f64::consts::TAU * 65536.0) as i32,
            0,
        ];
        let m = rotator_to_bevy_mat3(rot);
        tf.translation = Vec3::from(ue_to_bevy(b.pos.as_vec3().to_array()));
        tf.rotation = Quat::from_mat3(&Mat3::from_cols(
            Vec3::from(m[0]),
            Vec3::from(m[1]),
            Vec3::from(m[2]),
        ));
        if b.pos.distance(b.lit_at) > 32.0 {
            b.lit_at = b.pos;
            for (pi, hm) in b.handles.iter().enumerate() {
                if let Some(mut mm) = meshes.get_mut(hm) {
                    mm.insert_attribute(
                        Mesh::ATTRIBUTE_COLOR,
                        light_part(
                            &b.parts[pi],
                            b.pos.as_vec3().to_array(),
                            rot,
                            1.0,
                            &lvl.lights,
                        ),
                    );
                }
            }
        }
        let _ = b.gen;
    }
    if let Ok(mut t) = TARGETS.lock() {
        t.clear();
        for (e, b, _) in &birds {
            t.push((e, b.pos, b.radius, b.height));
        }
    }
}

/// Flying.PickDestination: ~1200 units in a random direction that's clear (halving down to 100), trying four
/// directions (and their mirror below/above); failing that, keep going the way it's flying.
fn pick_leg(
    level: &dx_level::Level,
    pos: DVec3,
    vel: DVec3,
    stuck: f32,
    rnd: &mut impl FnMut() -> f64,
) -> DVec3 {
    for _ in 0..4 {
        let mut dir =
            DVec3::new(rnd() * 2.0 - 1.0, rnd() * 2.0 - 1.0, rnd() * 2.0 - 1.0).normalize_or_zero();
        // Keep it mostly level: birds don't need to dive into the ground or climb into the sky box.
        dir.z *= 0.35;
        let dir = dir.normalize_or_zero();
        for flip in [1.0, -1.0] {
            let d = DVec3::new(dir.x, dir.y, dir.z * flip);
            let mut mag = 1200.0 + rnd() * 200.0 - 100.0;
            while mag > 100.0 {
                if level
                    .collision
                    .trace(pos, pos + d * mag, DVec3::splat(4.0))
                    .fraction
                    >= 1.0
                {
                    return pos + d * mag;
                }
                mag *= 0.5;
            }
        }
    }
    let ahead = if vel.length() > 0.001 {
        vel.normalize() * 40.0
    } else {
        DVec3::new(0.0, 0.0, 40.0)
    };
    let wobble = if stuck > 5.0 {
        DVec3::new(rnd() - 0.5, rnd() - 0.5, rnd() - 0.5) * 2.0 * ((stuck - 5.0) as f64 * 3.0)
    } else {
        DVec3::ZERO
    };
    pos + ahead + wobble
}

/// Flying.PickFinalDestination: up to three tries at the ground below a point within 100 units, reachable in a
/// straight line and not in water.
fn pick_landing(
    level: &dx_level::Level,
    pos: DVec3,
    rnd: &mut impl FnMut() -> f64,
) -> Option<DVec3> {
    for _ in 0..3 {
        let start = pos + DVec3::new(rnd() * 200.0 - 100.0, rnd() * 200.0 - 100.0, 0.0);
        let Some((floor, _)) = ground_below(level, start, 1000.0) else {
            continue;
        };
        let spot = floor + DVec3::new(0.0, 0.0, 10.0);
        if in_water(level, floor + DVec3::new(0.0, 0.0, 2.0)) {
            continue;
        }
        if level.collision.trace(pos, spot, DVec3::splat(3.0)).fraction >= 1.0 {
            return Some(spot);
        }
    }
    None
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
