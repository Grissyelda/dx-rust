//! Trash and flies.
//!
//! * TrashPaper / Tumbleweed (class Trash): rolls along at its wind direction x a speed factor re-rolled every
//!   second (0.8-1.0), tumbling about pitch/roll, playing Blowing. When it stops making progress (less than 0.1
//!   units a frame, i.e. 6 units/s at 60 Hz) or reaches water it settles flat at a random yaw on Still and is
//!   removed 20-30 s later (LifeSpan 30 caps it either way).
//! * Sources: a broken container with bGenerateTrash that sat on the ground (destruct.rs) throws up to four
//!   sheets (75% each) and maybe a rat; TrashGenerator actors drop a sheet with probability Frequency every 0.1 s.
//! * Flies: a decoration with bGenerateFlies gets a FlyGenerator 10% of the time (rolled once at level start).
//!   The generator keeps up to MaxCount (3) flies alive from a pool of 8 while the player is within ActiveArea
//!   (700). A fly (Fly.Wandering) keeps turning toward a jittered direction: its velocity plus a random push of
//!   2 x AirSpeed, or a strong pull back home when it strays past HomeExtent (the generator's Radius). It hums
//!   FlyBuzz.

use crate::weapons::{spawn_mesh_parts, Clip, RuntimeAssets};
use crate::{LevelRes, Player};
use bevy::prelude::*;
use dx_level::actors::{anim_mesh, light_part, rotator_to_bevy_mat3, AnimMesh, MeshPart};
use dx_level::ue_to_bevy;
use glam::DVec3;
use std::sync::{Arc, Mutex};
use unreal_pkg::props::{find, Property, Value};

/// A sheet of trash requested by another system: (class, position, wind direction x speed).
static QUEUE: Mutex<Vec<(&'static str, DVec3, DVec3)>> = Mutex::new(Vec::new());

static DESTROYED: Mutex<Vec<DVec3>> = Mutex::new(Vec::new());

/// A decoration at `pos` was destroyed: its fly swarm (if any) bursts out to full strength and stops generating.
pub fn deco_destroyed(pos: DVec3) {
    if let Ok(mut q) = DESTROYED.lock() {
        q.push(pos);
    }
}

pub fn spawn_trash(class: &'static str, pos: DVec3, dir: DVec3) {
    if let Ok(mut q) = QUEUE.lock() {
        q.push((class, pos, dir));
    }
}

#[derive(Component)]
pub struct Trash {
    pos: DVec3,
    vz: f64,
    dir: DVec3,
    /// Tumble rate (pitch, roll) in Unreal units per second, and the current rotation.
    rot_rate: [f64; 2],
    rot: [f64; 3],
    rnd: f64,
    time: f32,
    stopped: bool,
    life: f32,
    die_in: f32,
    height: f64,
    radius: f64,
    mesh: Arc<AnimMesh>,
    parts: Vec<MeshPart>,
    handles: Vec<Handle<Mesh>>,
    clip: Clip,
    lit_at: DVec3,
}

struct FlyGen {
    pos: DVec3,
    active_area: f64,
    radius: f64,
    max: usize,
    pool: usize,
    flies: Vec<Entity>,
    try_timer: f32,
    /// StopGenerator: no more new flies.
    stopped: bool,
    /// Burst: spawn this many at once.
    burst: usize,
}

#[derive(Component)]
pub struct Fly {
    gen: usize,
    pos: DVec3,
    vel: DVec3,
    air_speed: f64,
    accel: f64,
    key: u64,
    mesh: Arc<AnimMesh>,
    parts: Vec<MeshPart>,
    handles: Vec<Handle<Mesh>>,
    clip: Clip,
    lit_at: DVec3,
}

#[derive(Resource, Default)]
pub struct TrashState {
    init: bool,
    seed: u64,
    gens: Vec<(DVec3, f64, f64, f32, bool)>, // TrashGenerator: position, Frequency, WindSpeed, timer, tumbleweed
    yaw_dirs: Vec<DVec3>,
    flies: Vec<FlyGen>,
}

impl TrashState {
    fn rand(&mut self) -> f64 {
        self.seed = self
            .seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.seed >> 11) as f64 / (1u64 << 53) as f64
    }
    fn vrand(&mut self) -> DVec3 {
        loop {
            let v = DVec3::new(
                self.rand() * 2.0 - 1.0,
                self.rand() * 2.0 - 1.0,
                self.rand() * 2.0 - 1.0,
            );
            let l = v.length();
            if l > 0.01 && l <= 1.0 {
                return v / l;
            }
        }
    }
}

fn fprop(p: &[Property], n: &str, d: f64) -> f64 {
    match find(p, n) {
        Some(Value::Float(v)) => *v as f64,
        Some(Value::Int(v)) => *v as f64,
        Some(Value::Byte(v)) => *v as f64,
        _ => d,
    }
}

fn flag(p: &[Property], n: &str) -> bool {
    matches!(find(p, n), Some(Value::Bool(true)))
}

fn loc_of(p: &[Property]) -> Option<DVec3> {
    match find(p, "Location") {
        Some(Value::Vector(l)) => Some(DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64)),
        _ => None,
    }
}

fn to_rot(r: [f64; 3]) -> [i32; 3] {
    [r[0] as i32, r[1] as i32, r[2] as i32]
}

fn quat_of(rot: [i32; 3]) -> Quat {
    let m = rotator_to_bevy_mat3(rot);
    Quat::from_mat3(&Mat3::from_cols(
        Vec3::from(m[0]),
        Vec3::from(m[1]),
        Vec3::from(m[2]),
    ))
}

fn pose(
    meshes: &mut Assets<Mesh>,
    mesh: &AnimMesh,
    clip: &Clip,
    parts: &[MeshPart],
    handles: &[Handle<Mesh>],
) {
    let f = clip.frame();
    for (pi, hm) in handles.iter().enumerate() {
        if let Some(mut mm) = meshes.get_mut(hm) {
            let pos: Vec<[f32; 3]> = parts[pi]
                .vert_ids
                .iter()
                .map(|&v| ue_to_bevy(mesh.sample(clip.start, clip.frames, f, v as usize)))
                .collect();
            mm.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn trash_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    player: Res<Player>,
    mut st: ResMut<TrashState>,
    mut trash: Query<(Entity, &mut Trash, &mut Transform), Without<Fly>>,
    mut flies: Query<(Entity, &mut Fly, &mut Transform), Without<Trash>>,
    (mut assets, mut images, mut meshes, mut materials): (
        ResMut<RuntimeAssets>,
        ResMut<Assets<Image>>,
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
    ),
) {
    let _prof = crate::prof::scope("trash::trash_system");
    let lvl = &level.0;
    let dt = time.delta_secs().min(0.1);
    if !st.init {
        st.init = true;
        st.seed = 0x7A5C_0FF1;
        for a in &lvl.actors {
            let Some(pos) = loc_of(&a.props) else {
                continue;
            };
            if a.class == "TrashGenerator" {
                let yaw = match find(&a.props, "Rotation") {
                    Some(Value::Rotator(r)) => r[1] as f64 / 65536.0 * std::f64::consts::TAU,
                    _ => 0.0,
                };
                let tumble = matches!(find(&a.props, "TrashType"), Some(Value::Byte(1)));
                st.gens.push((
                    pos,
                    fprop(&a.props, "Frequency", 0.0),
                    fprop(&a.props, "WindSpeed", 0.0),
                    0.0,
                    tumble,
                ));
                st.yaw_dirs.push(DVec3::new(yaw.cos(), yaw.sin(), 0.0));
            }
            // DeusExDecoration.BeginPlay: 10% of fly-generating decorations get flies.
            if flag(&a.props, "bGenerateFlies") && a.class != "FlyGenerator" && st.rand() < 0.1 {
                let d = assets.classes.defaults("FlyGenerator");
                let pool = match find(&d, "PawnClasses") {
                    Some(Value::Struct(_, b)) if b.len() >= 4 => {
                        i32::from_le_bytes([b[0], b[1], b[2], b[3]]).max(0) as usize
                    }
                    _ => 8,
                };
                st.flies.push(FlyGen {
                    pos,
                    active_area: fprop(&d, "ActiveArea", 700.0),
                    radius: fprop(&d, "Radius", 50.0),
                    max: fprop(&d, "MaxCount", 3.0) as usize,
                    pool,
                    flies: Vec::new(),
                    try_timer: 0.0,
                    stopped: false,
                    burst: 0,
                });
            }
        }
        println!(
            "trash: {} trash generators, {} fly swarms",
            st.gens.len(),
            st.flies.len()
        );
    }

    // TrashGenerator.Tick: every 0.1 s, a sheet with probability Frequency.
    let mut new_trash: Vec<(&'static str, DVec3, DVec3)> = QUEUE
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default();
    for gi in 0..st.gens.len() {
        st.gens[gi].3 += dt;
        if st.gens[gi].3 <= 0.1 {
            continue;
        }
        st.gens[gi].3 = 0.0;
        let (pos, freq, wind, _, tumble) = st.gens[gi];
        if st.rand() < freq {
            let mut dir = st.yaw_dirs[gi] * wind;
            dir.x += 5.0 * st.rand();
            dir.y += 5.0 * st.rand();
            dir.z = -2.0 * st.rand();
            new_trash.push((if tumble { "Tumbleweed" } else { "TrashPaper" }, pos, dir));
        }
    }
    for (class, pos, dir) in new_trash {
        let d = assets.classes.defaults(class);
        let Some(Value::Path(mesh_path)) = find(&d, "Mesh").cloned() else {
            continue;
        };
        let root = commands
            .spawn((
                Transform::from_translation(Vec3::from(ue_to_bevy(pos.as_vec3().to_array()))),
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
        let clip = Clip::named(&am, "Blowing", true)
            .or_else(|| Clip::named(&am, "Still", true))
            .unwrap_or(Clip {
                start: 0,
                frames: 1,
                rate: 1.0,
                looping: true,
                t: 0.0,
            });
        // rot = RotRand(true) with Yaw 0: pitch and roll spin at up to a full turn a second.
        let rot_rate = [st.rand() * 65536.0, st.rand() * 65536.0];
        commands.entity(root).insert(Trash {
            pos,
            vz: 0.0,
            dir,
            rot_rate,
            rot: [0.0; 3],
            rnd: 1.0,
            time: 0.0,
            stopped: false,
            life: fprop(&d, "LifeSpan", 30.0) as f32,
            die_in: f32::MAX,
            height: fprop(&d, "CollisionHeight", 6.0),
            radius: fprop(&d, "CollisionRadius", 6.0),
            mesh: am,
            parts,
            handles,
            clip,
            lit_at: DVec3::splat(1e9),
        });
    }

    for (e, mut t, mut tf) in &mut trash {
        let t = &mut *t;
        t.life -= dt;
        t.die_in -= dt;
        if t.life <= 0.0 || t.die_in <= 0.0 {
            commands.entity(e).despawn();
            continue;
        }
        let ext = DVec3::new(
            t.radius.min(4.0),
            t.radius.min(4.0),
            if t.stopped { 0.1 } else { t.height.min(4.0) },
        );
        // Gravity (PHYS_Rolling/Falling keep it on the ground).
        t.vz = (t.vz - 950.0 * dt as f64).max(-2500.0);
        let down = t.pos + DVec3::new(0.0, 0.0, t.vz * dt as f64);
        let h = lvl.collision.trace(t.pos, down, ext);
        if h.fraction < 1.0 {
            t.pos += (down - t.pos) * (h.fraction - 0.01).max(0.0);
            t.vz = 0.0;
        } else {
            t.pos = down;
        }
        if !t.stopped {
            t.time += dt;
            if t.time >= 1.0 {
                t.rnd = st.rand() * 0.2 + 0.8;
                t.time = 0.0;
            }
            let vel = DVec3::new(t.dir.x, t.dir.y, 0.0) * t.rnd;
            let want = t.pos + vel * dt as f64;
            let h = lvl.collision.trace(t.pos, want, ext);
            let before = t.pos;
            t.pos += (want - t.pos) * (h.fraction - 0.01).max(0.0);
            for k in 0..2 {
                t.rot[k * 2] += t.rot_rate[k] * t.rnd * dt as f64;
            }
            // Stuck against something (0.1 unit a frame at the original's 60 Hz), or in water: settle.
            let water = matches!(
                lvl.zone_water
                    .get(lvl.collision.zone_at(t.pos).max(0) as usize),
                Some(Some(_))
            );
            if before.distance(t.pos) < 6.0 * dt as f64 || water {
                t.stopped = true;
                t.rot = [0.0, st.rand() * 65536.0, 0.0];
                if let Some(c) = Clip::named(&t.mesh, "Still", true) {
                    t.clip = c;
                }
                t.die_in = (10.0 * st.rand() + 20.0) as f32;
            }
            t.clip.t += dt;
        }
        pose(&mut meshes, &t.mesh, &t.clip, &t.parts, &t.handles);
        let rot = to_rot(t.rot);
        tf.translation = Vec3::from(ue_to_bevy(t.pos.as_vec3().to_array()));
        tf.rotation = quat_of(rot);
        if t.pos.distance(t.lit_at) > 24.0 || t.stopped && t.lit_at.x > 1e8 {
            t.lit_at = t.pos;
            for (pi, hm) in t.handles.iter().enumerate() {
                if let Some(mut mm) = meshes.get_mut(hm) {
                    mm.insert_attribute(
                        Mesh::ATTRIBUTE_COLOR,
                        light_part(
                            &t.parts[pi],
                            t.pos.as_vec3().to_array(),
                            rot,
                            1.0,
                            &lvl.lights,
                        ),
                    );
                }
            }
        }
    }

    // Fly swarms.
    for pos in DESTROYED
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
    {
        for g in st.flies.iter_mut() {
            if !g.stopped && g.pos.distance(pos) < 4.0 {
                g.burst = g.max.saturating_sub(g.flies.len()).min(g.pool);
                g.stopped = true;
            }
        }
    }
    let me = player.body.location;
    let mut spawn_at: Vec<(usize, DVec3)> = Vec::new();
    for gi in 0..st.flies.len() {
        let near = st.flies[gi].pos.distance(me) < st.flies[gi].active_area;
        if !near {
            // Transient: the swarm goes back into the pool out here.
            let list = std::mem::take(&mut st.flies[gi].flies);
            st.flies[gi].pool += list.len();
            for e in list {
                commands.entity(e).despawn();
            }
            continue;
        }
        if st.flies[gi].burst > 0 {
            let n = std::mem::take(&mut st.flies[gi].burst);
            for _ in 0..n {
                let v = st.vrand();
                let at = st.flies[gi].pos + v * st.flies[gi].radius * st.rand();
                spawn_at.push((gi, at));
            }
            continue;
        }
        st.flies[gi].try_timer -= dt;
        if st.flies[gi].stopped
            || st.flies[gi].try_timer > 0.0
            || st.flies[gi].flies.len() >= st.flies[gi].max
            || st.flies[gi].pool == 0
        {
            continue;
        }
        st.flies[gi].try_timer = 0.25;
        let v = st.vrand();
        let at = st.flies[gi].pos + v * st.flies[gi].radius * st.rand();
        spawn_at.push((gi, at));
    }
    for (gi, at) in spawn_at {
        let d = assets.classes.defaults("Fly");
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
        let clip = Clip::named(&am, "Still", true).unwrap_or(Clip {
            start: 0,
            frames: 1,
            rate: 1.0,
            looping: true,
            t: 0.0,
        });
        let air = fprop(&d, "AirSpeed", 100.0);
        let v0 = st.vrand() * air * 0.5;
        st.flies[gi].pool = st.flies[gi].pool.saturating_sub(1);
        st.flies[gi].flies.push(root);
        let key = crate::sound::key_of(&format!("fly{}", root.to_bits()));
        commands.entity(root).insert(Fly {
            gen: gi,
            pos: at,
            vel: v0,
            air_speed: air,
            accel: fprop(&d, "AccelRate", 500.0),
            key,
            mesh: am,
            parts,
            handles,
            clip,
            lit_at: DVec3::splat(1e9),
        });
    }
    for (_, mut f, mut tf) in &mut flies {
        let f = &mut *f;
        let Some(g) = st.flies.get(f.gen) else {
            continue;
        };
        let (home, extent) = (g.pos, g.radius);
        // PickDirection, every tick.
        let mut want = if f.pos.distance(home) > extent {
            (home - f.pos).normalize_or_zero() * f.air_speed * 4.0
        } else {
            f.vel
        };
        want += st.vrand() * f.air_speed * 2.0;
        let want = want.normalize_or_zero();
        f.vel += want * f.accel * dt as f64;
        let sp = f.vel.length();
        if sp > f.air_speed {
            f.vel *= f.air_speed / sp;
        }
        let next = f.pos + f.vel * dt as f64;
        let h = lvl.collision.trace(f.pos, next, DVec3::splat(1.0));
        if h.fraction < 1.0 {
            // HitWall: bounce with elasticity 0.3.
            f.pos += (next - f.pos) * (h.fraction - 0.05).max(0.0);
            f.vel = 0.3 * (f.vel - 2.0 * f.vel.dot(h.normal) * h.normal);
        } else {
            f.pos = next;
        }
        f.clip.t += dt;
        pose(&mut meshes, &f.mesh, &f.clip, &f.parts, &f.handles);
        let yaw = (f.vel.y.atan2(f.vel.x) / std::f64::consts::TAU * 65536.0) as i32;
        let rot = [0, yaw, 0];
        tf.translation = Vec3::from(ue_to_bevy(f.pos.as_vec3().to_array()));
        tf.rotation = quat_of(rot);
        if f.pos.distance(f.lit_at) > 32.0 {
            f.lit_at = f.pos;
            for (pi, hm) in f.handles.iter().enumerate() {
                if let Some(mut mm) = meshes.get_mut(hm) {
                    mm.insert_attribute(
                        Mesh::ATTRIBUTE_COLOR,
                        light_part(
                            &f.parts[pi],
                            f.pos.as_vec3().to_array(),
                            rot,
                            1.0,
                            &lvl.lights,
                        ),
                    );
                }
            }
        }
        // AmbientSound FlyBuzz: SoundRadius 6, SoundVolume 128.
        crate::sound::hold(
            f.key,
            "DeusExSounds.Animal.FlyBuzz",
            Some(f.pos),
            128.0 / 255.0,
            25.0 * 7.0,
            1.0,
        );
    }
}

/// A new map is going in (loader.rs): this module's state of the map being left.
pub fn reset_level() {
    if let Ok(mut v) = QUEUE.lock() {
        v.clear();
    }
    if let Ok(mut v) = DESTROYED.lock() {
        v.clear();
    }
}
