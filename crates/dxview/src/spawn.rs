//! Spawning pawns at run time (Actor.Spawn for ScriptedPawn classes).
//!
//! A spawned pawn becomes a full map actor: a new `Level::actors` entry (class defaults + location/rotation), a
//! `Level::mesh_actors` entry with its animation player, a blocking cylinder, and the same Deco/Animated/Npc
//! components the map's own pawns get, so the AI, damage, carcasses and searching all treat it like any other.
//! Requests go through a global queue (like `sound::play`) so any system can ask for a pawn.

use crate::actors::{Animated, Blockers, Deco, MeshActorRef};
use crate::weapons::{spawn_mesh_parts, RuntimeAssets};
use crate::LevelRes;
use bevy::prelude::*;
use dx_level::actors::{anim_mesh, rotator_to_bevy_mat3, ActorAnim, ActorInfo, MeshActor};
use dx_level::collision::Cylinder;
use dx_level::ue_to_bevy;
use glam::DVec3;
use imsim_core::body::{Body, BodyPhysics};
use std::sync::Mutex;
use unreal_pkg::props::{find, Property, Value};

pub struct SpawnReq {
    pub class: String,
    /// Collision centre, Unreal units.
    pub pos: DVec3,
    /// Unreal yaw units.
    pub yaw: i32,
    /// bTransient: generator pawns that may be removed again when nobody is around.
    pub transient: bool,
    /// Property overrides on top of the class defaults (Orders, Alliance, ...).
    pub props: Vec<Property>,
    /// Index of the ground generator this pawn belongs to.
    pub generator: Option<usize>,
}

static QUEUE: Mutex<Vec<SpawnReq>> = Mutex::new(Vec::new());

/// Ask for a pawn of `class` standing at `pos` (collision centre).
pub fn pawn(class: &str, pos: DVec3, yaw: i32) {
    pawn_with(class, pos, yaw, Vec::new());
}

/// As `pawn`, with property overrides on top of the class defaults.
pub fn pawn_with(class: &str, pos: DVec3, yaw: i32, props: Vec<Property>) {
    request(SpawnReq {
        class: class.to_string(),
        pos,
        yaw,
        transient: false,
        props,
        generator: None,
    });
}

/// The hostile animals. Their class defaults leave them neutral to the player (they only turn on whoever hurts
/// them, bHateInjury); every map that places one sets InitialAlliances Player -1 on the instance (Dobermans in
/// the MJ12 lab, Karkians/Greasels in the Ocean Lab and Area 51). The spawner gives them the same.
pub fn hostile_animal_alliance(class: &str) -> Option<Property> {
    matches!(
        class,
        "Doberman" | "Greasel" | "Karkian" | "KarkianBaby" | "Gray"
    )
    .then(|| Property {
        name: "InitialAlliances".into(),
        index: 1,
        value: Value::Alliance("Player".into(), -1.0, true),
    })
}

pub fn request(r: SpawnReq) {
    if let Ok(mut q) = QUEUE.lock() {
        q.push(r);
    }
}

/// Pawns spawned this session, by entity (for generators that keep count).
#[derive(Resource, Default)]
pub struct Spawned {
    pub count: usize,
}

/// Marks pawns created at run time.
#[derive(Component)]
pub struct SpawnedPawn {
    pub transient: bool,
}

fn fv(p: &[Property], n: &str, d: f64) -> f64 {
    match find(p, n) {
        Some(Value::Float(v)) => *v as f64,
        Some(Value::Int(v)) => *v as f64,
        Some(Value::Byte(v)) => *v as f64,
        _ => d,
    }
}

fn set(props: &mut Vec<Property>, name: &str, value: Value) {
    props.retain(|p| !(p.name.eq_ignore_ascii_case(name) && p.index == 0));
    props.push(Property {
        name: name.into(),
        index: 0,
        value,
    });
}

fn bevy_rotation(rot: [i32; 3]) -> Quat {
    let m = rotator_to_bevy_mat3(rot);
    Quat::from_mat3(&Mat3::from_cols(
        Vec3::from(m[0]),
        Vec3::from(m[1]),
        Vec3::from(m[2]),
    ))
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_system(
    mut commands: Commands,
    mut level: ResMut<LevelRes>,
    mut blockers: ResMut<Blockers>,
    mut spawned: ResMut<Spawned>,
    (mut assets, mut images, mut meshes, mut materials): (
        ResMut<RuntimeAssets>,
        ResMut<Assets<Image>>,
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
    ),
) {
    let _prof = crate::prof::scope("spawn::spawn_system");
    let reqs: Vec<SpawnReq> = match QUEUE.lock() {
        Ok(mut q) if !q.is_empty() => std::mem::take(&mut *q),
        _ => return,
    };
    for r in reqs {
        let mut props = assets.classes.defaults(&r.class);
        if find(&props, "GroundSpeed").is_none() {
            println!("SPAWN {}: not a pawn class", r.class);
            continue;
        }
        for p in r.props {
            // Overrides replace the same array slot (InitialAlliances[1] etc.).
            props.retain(|q| !(q.name.eq_ignore_ascii_case(&p.name) && q.index == p.index));
            props.push(p);
        }
        let rot = [0, r.yaw, 0];
        set(
            &mut props,
            "Location",
            Value::Vector([r.pos.x as f32, r.pos.y as f32, r.pos.z as f32]),
        );
        set(&mut props, "Rotation", Value::Rotator(rot));
        if r.transient {
            set(&mut props, "bTransient", Value::Bool(true));
        }
        let Some(Value::Path(mesh_path)) = find(&props, "Mesh").cloned() else {
            println!("SPAWN {}: no mesh", r.class);
            continue;
        };
        let pre = match find(&props, "PrePivot") {
            Some(Value::Vector(v)) => DVec3::new(v[0] as f64, v[1] as f64, v[2] as f64),
            _ => DVec3::ZERO,
        };
        let draw = r.pos + pre;
        let draw_scale = fv(&props, "DrawScale", 1.0) as f32;
        let root = commands
            .spawn((
                Transform::from_translation(Vec3::from(ue_to_bevy(draw.as_vec3().to_array())))
                    .with_rotation(bevy_rotation(rot)),
                Visibility::default(),
            ))
            .id();
        let map = |p: [f32; 3]| ue_to_bevy(p);
        let Some((m, parts, handles)) = spawn_mesh_parts(
            &mesh_path,
            draw_scale,
            &props,
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
            println!("SPAWN {}: mesh {} failed", r.class, mesh_path);
            continue;
        };
        let lvl = &mut level.0;
        spawned.count += 1;
        let name = format!("{}{}", r.class, 1000 + spawned.count);
        let ai = lvl.actors.len();
        let seq = m
            .seq("BreatheLight")
            .or_else(|| m.seq("Still"))
            .or_else(|| m.seq("Idle1"));
        let anim = seq.map(|s| ActorAnim {
            mesh: std::sync::Arc::new(anim_mesh(&m)),
            draw_scale,
            seq_start: s.start_frame.max(0) as usize,
            seq_frames: s.num_frames as usize,
            rate: s.rate,
            start_frame: 0.0,
        });
        let radius = fv(&props, "CollisionRadius", 20.0);
        let height = fv(&props, "CollisionHeight", 20.0);
        let mass = fv(&props, "Mass", 100.0);
        let buoyancy = fv(&props, "Buoyancy", 0.0);
        lvl.actors.push(ActorInfo {
            name: name.clone(),
            class: r.class.clone(),
            export: -1,
            props,
        });
        let mi = lvl.mesh_actors.len();
        lvl.mesh_actors.push(MeshActor {
            actor: ai,
            parts,
            location: draw.as_vec3().to_array(),
            rotation: rot,
            scale_glow: 1.0,
            anim,
        });
        let b = blockers.list.len();
        blockers.list.push(Cylinder {
            center: r.pos,
            radius,
            height,
            actor: ai,
        });
        blockers.deco.insert(b, root);
        let part_refs: Vec<(Handle<Mesh>, usize)> = handles
            .into_iter()
            .enumerate()
            .map(|(i, h)| (h, i))
            .collect();
        let body = Body {
            location: r.pos,
            velocity: DVec3::ZERO,
            physics: BodyPhysics::Falling,
            radius: radius.max(1.0),
            height: height.max(1.0),
            landed_vz: None,
        };
        commands.entity(root).insert((
            MeshActorRef(mi),
            Animated {
                mesh_actor: mi,
                parts: part_refs.clone(),
                time: 0.0,
                clip: None,
                shown: None,
                shown_t: 0.0,
                tween: None,
                face: crate::actors::Face::default(),
                face_shown: None,
            },
            Deco {
                body,
                blocker: Some(b),
                mass,
                pushable: false,
                pre,
                rotation: rot,
                scale_glow: 1.0,
                parts: part_refs,
                mesh_actor: mi,
                last_lit: DVec3::splat(1e9),
                spun: true,
                name: name.clone(),
                start: r.pos,
                buoyancy,
                floating: false,
                orig_rot: rot,
                thrown_by_player: false,
                thrown_body: false,
                push_snd: 0.0,
                falls: true,
            },
            crate::ai::Npc::new(ai, b, &lvl.actors[ai].props, r.yaw, r.pos),
            SpawnedPawn {
                transient: r.transient,
            },
        ));
        if let Some(g) = r.generator {
            commands.entity(root).insert(FromGenerator(g));
        }
        println!(
            "SPAWN {} as {} at {:.0} {:.0} {:.0}",
            r.class, name, r.pos.x, r.pos.y, r.pos.z
        );
    }
}

/// Ground PawnGenerators (RatGenerator): like the bird generators in critters.rs, but the pawns are full
/// ScriptedPawns. Up to MaxCount alive from the pool while the player is within ActiveArea, one try every 0.25 s
/// at a hidden floor spot within Radius; transient pawns out of the area and out of sight go back to the pool.
struct GroundGen {
    class: &'static str,
    pos: DVec3,
    active_area: f64,
    radius: f64,
    max: usize,
    pool: usize,
    alive: Vec<Entity>,
    pending: usize,
    try_timer: f32,
    status_timer: f32,
    /// PawnGenerator.bActive (bTriggered ones start off until Trigger).
    active: bool,
    tag: String,
    gen_class: String,
}

static TRIGGERED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// foreach AllActors(class'<generator>', gen) gen.Trigger(...): PawnGenerator.Trigger sets bActive.
pub fn trigger_generators(class: &str) {
    if let Ok(mut q) = TRIGGERED.lock() {
        q.push(class.to_string());
    }
}

#[derive(Resource, Default)]
pub struct Generators {
    init: bool,
    gens: Vec<GroundGen>,
    seed: u64,
}

impl Generators {
    fn rand(&mut self) -> f64 {
        self.seed = self
            .seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.seed >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn ground_class(generator: &str) -> Option<&'static str> {
    match generator {
        "RatGenerator" => Some("Rat"),
        _ => None,
    }
}

/// Generator pawns spawned this frame, tagged with their generator (filled in by spawn_system).
#[derive(Component)]
pub struct FromGenerator(pub usize);

#[allow(clippy::too_many_arguments)]
pub fn generator_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    player: Res<crate::Player>,
    cam: Single<&Transform, (With<crate::FlyCam>, Without<crate::SkyCam>)>,
    mut blockers: ResMut<Blockers>,
    mut st: ResMut<Generators>,
    mut assets: ResMut<RuntimeAssets>,
    pawns: Query<(Entity, &Deco, &crate::ai::Npc, Option<&FromGenerator>), With<SpawnedPawn>>,
) {
    let _prof = crate::prof::scope("spawn::generator_system");
    let lvl = &level.0;
    let dt = time.delta_secs().min(0.1);
    if !st.init {
        st.init = true;
        st.seed = 0xBADC_0FFE;
        for a in &lvl.actors {
            let Some(class) = ground_class(&a.class) else {
                continue;
            };
            let Some(Value::Vector(l)) = find(&a.props, "Location") else {
                continue;
            };
            let pos = DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64);
            let pool = crate::critters::pool_count(&a.props);
            let max = fv(&a.props, "MaxCount", 0.0) as usize;
            st.gens.push(GroundGen {
                class,
                pos,
                active_area: fv(&a.props, "ActiveArea", 2000.0),
                radius: fv(&a.props, "Radius", 600.0),
                max: if max == 0 { pool } else { max.min(pool.max(1)) },
                pool,
                alive: Vec::new(),
                pending: 0,
                try_timer: 0.25,
                status_timer: 0.0,
                active: !matches!(find(&a.props, "bTriggered"), Some(Value::Bool(true))),
                tag: match find(&a.props, "Tag") {
                    Some(Value::Name(t)) => t.clone(),
                    _ => String::new(),
                },
                gen_class: a.class.clone(),
            });
        }
        println!("spawn: {} ground pawn generators", st.gens.len());
    }
    for c in TRIGGERED.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        for g in st.gens.iter_mut() {
            if g.gen_class.eq_ignore_ascii_case(&c) || g.tag.eq_ignore_ascii_case(&c) {
                g.active = true;
            }
        }
    }
    // Adopt pawns spawned for a generator last frame.
    for (e, _, _, g) in &pawns {
        if let Some(FromGenerator(gi)) = g {
            if let Some(gen) = st.gens.get_mut(*gi) {
                if !gen.alive.contains(&e) {
                    gen.alive.push(e);
                    gen.pending = gen.pending.saturating_sub(1);
                }
            }
        }
    }
    let eye = {
        let u = dx_level::bevy_to_ue(cam.translation.to_array());
        DVec3::new(u[0] as f64, u[1] as f64, u[2] as f64)
    };
    let fwd = {
        let f = cam.forward().as_vec3();
        DVec3::new(f.x as f64, f.z as f64, f.y as f64).normalize_or_zero()
    };
    let me = player.body.location;
    for gi in 0..st.gens.len() {
        st.gens[gi].status_timer += dt;
        if st.gens[gi].status_timer > 1.0 {
            st.gens[gi].status_timer = 0.0;
            let area = st.gens[gi].active_area;
            let list = std::mem::take(&mut st.gens[gi].alive);
            let mut keep = Vec::new();
            for e in list {
                let Ok((_, d, n, _)) = pawns.get(e) else {
                    continue;
                };
                // Dead ones stay as carcasses but no longer count.
                if n.state == crate::ai::NpcState::Dead {
                    continue;
                }
                if d.body.location.distance(me) >= area
                    && !crate::critters::player_sees(lvl, eye, fwd, d.body.location)
                {
                    if let Some(b) = d.blocker {
                        blockers.list[b].radius = 0.0;
                        blockers.list[b].height = 0.0;
                        blockers.deco.remove(&b);
                    }
                    commands.entity(e).despawn();
                    st.gens[gi].pool += 1;
                } else {
                    keep.push(e);
                }
            }
            st.gens[gi].alive = keep;
        }
        let g = &st.gens[gi];
        if !g.active || g.pos.distance(me) >= g.active_area || g.alive.len() + g.pending >= g.max || g.pool == 0
        {
            continue;
        }
        st.gens[gi].try_timer -= dt;
        if st.gens[gi].try_timer >= 0.0 {
            continue;
        }
        st.gens[gi].try_timer = 0.25;
        let r = st.rand().sqrt();
        let a = st.rand() * std::f64::consts::TAU;
        let g = &st.gens[gi];
        let start = g.pos + DVec3::new(0.0, 0.0, 8.0);
        let want = start + DVec3::new(a.cos(), a.sin(), 0.0) * r * g.radius;
        let h = lvl.collision.trace(start, want, DVec3::new(8.0, 8.0, 3.0));
        let at = start + (want - start) * (h.fraction - 0.02).max(0.0);
        let Some((floor, _)) = crate::critters::ground_below(lvl, at, 160.0) else {
            continue;
        };
        if crate::critters::in_water(lvl, floor + DVec3::new(0.0, 0.0, 4.0))
            || crate::critters::player_sees(lvl, eye, fwd, floor + DVec3::new(0.0, 0.0, 4.0))
        {
            continue;
        }
        let class = g.class;
        let h = fv(&assets.classes.defaults(class), "CollisionHeight", 4.0);
        let yaw = (st.rand() * 65536.0) as i32;
        st.gens[gi].pool -= 1;
        st.gens[gi].pending += 1;
        request(SpawnReq {
            class: class.into(),
            pos: floor + DVec3::new(0.0, 0.0, h + 1.0),
            yaw,
            transient: true,
            props: Vec::new(),
            generator: Some(gi),
        });
    }
}

/// A new map is going in (loader.rs): this module's state of the map being left.
pub fn reset_level() {
    if let Ok(mut v) = QUEUE.lock() {
        v.clear();
    }
}
