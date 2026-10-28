//! Breakable decorations (DeusExDecoration.TakeDamage / Frag / Containers.Destroyed): damage darkens the
//! object, at zero hit points it bursts into bouncing fragments of its own texture and drops its contents.

use crate::actors::{Blockers, Deco};
use crate::pickups::PickupItem;
use crate::weapons::{spawn_mesh_parts, RuntimeAssets};
use crate::LevelRes;
use bevy::prelude::*;
use dx_level::actors::{light_part, MeshPart};
use dx_level::ue_to_bevy;
use glam::DVec3;
use std::sync::Arc;
use unreal_pkg::props::{find, Property, Value};

static TRIGGERED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// An event was triggered: explosive decorations with that Tag go off.
pub fn trigger_event(tag: &str) {
    if let Ok(mut q) = TRIGGERED.lock() {
        q.push(tag.to_string());
    }
}

/// Damage dealt to a non-pawn blocker this frame.
pub struct DecoHit {
    pub blocker: usize,
    pub damage: f32,
    /// Unreal damage type name ("Shot", "Stunned", "Sabot", "Exploded", ...).
    pub damage_type: &'static str,
}

#[derive(Resource, Default)]
pub struct DecoHits(pub Vec<DecoHit>);

/// Something thrown loose: a fragment or a dropped item. Bounces like Fragment.HitWall.
#[derive(Component)]
pub struct Loose {
    pub pos: DVec3,
    pub vel: DVec3,
    /// Rotator (pitch, yaw, roll) and spin per second, Unreal units.
    pub rot: [f64; 3],
    pub spin: [f64; 3],
    pub elasticity: f64,
    pub extent: DVec3,
    pub resting: bool,
    /// Seconds left before it disappears (fragments), or None to stay.
    pub life: Option<f32>,
    pub parts: Vec<(Handle<Mesh>, MeshPart)>,
    pub lit_at: DVec3,
}

impl Loose {
    /// An object let go at `pos` that falls and settles (a carcass, a dropped item).
    pub fn dropped(pos: DVec3, height: f64) -> Loose {
        let h = height.max(1.0);
        Loose {
            pos,
            vel: DVec3::ZERO,
            rot: [0.0; 3],
            spin: [0.0; 3],
            elasticity: 0.0,
            extent: DVec3::new(h, h, h),
            resting: false,
            life: None,
            parts: Vec::new(),
            lit_at: DVec3::ZERO,
        }
    }
}

/// A burst of fragments asked for by another system (a door blown apart).
pub struct Burst {
    pub class: String,
    pub pos: DVec3,
    pub count: usize,
    pub spread: f64,
    pub scale: f32,
    pub skin: Option<String>,
}

static BURSTS: std::sync::Mutex<Vec<Burst>> = std::sync::Mutex::new(Vec::new());

pub fn burst(b: Burst) {
    if let Ok(mut q) = BURSTS.lock() {
        q.push(b);
    }
}

/// DeusExFragment's bounce sounds (ImpactSound or MiscSound, half the time each).
#[derive(Component)]
pub struct LooseSound(pub [Option<String>; 2]);

/// Inventory.PlayLandingSound: a dropped item's LandSound when it hits the ground falling at 200+ (radius 768,
/// TransientSoundVolume), also a LoudNoise for the AI.
#[derive(Component)]
pub struct LandSound(pub String, pub f32);

fn fv(p: &[Property], n: &str, d: f32) -> f32 {
    match find(p, n) {
        Some(Value::Float(v)) => *v,
        Some(Value::Int(v)) => *v as f32,
        Some(Value::Byte(v)) => *v as f32,
        _ => d,
    }
}
/// The placed Location of an actor (fly swarms key on it).
fn start_of(p: &[Property]) -> Option<DVec3> {
    match find(p, "Location") {
        Some(Value::Vector(l)) => Some(DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64)),
        _ => None,
    }
}
fn flag(p: &[Property], n: &str) -> bool {
    matches!(find(p, n), Some(Value::Bool(true)))
}
fn path(p: &[Property], n: &str) -> Option<String> {
    p.iter()
        .find(|x| x.name.eq_ignore_ascii_case(n) && x.index == 0)
        .and_then(|x| match &x.value {
            Value::Path(s) if s != "None" => Some(s.clone()),
            _ => None,
        })
}

struct Rng<'a>(&'a mut u64);
impl Rng<'_> {
    fn f(&mut self) -> f64 {
        *self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((*self.0 >> 33) as f64) / (1u64 << 31) as f64
    }
    /// Uniform random unit vector (VRand).
    fn vrand(&mut self) -> DVec3 {
        loop {
            let v = DVec3::new(
                self.f() * 2.0 - 1.0,
                self.f() * 2.0 - 1.0,
                self.f() * 2.0 - 1.0,
            );
            let l = v.length_squared();
            if l > 1e-4 && l <= 1.0 {
                return v / l.sqrt();
            }
        }
    }
}

/// Texture a fragment takes from its decoration (DeusExDecoration.GetMeshTexture).
fn mesh_texture(classes: &unreal_pkg::classes::ClassDb, p: &[Property]) -> Option<String> {
    if let Some(s) = path(p, "Skin") {
        return Some(s);
    }
    if let Some(s) =
        p.iter().find_map(
            |x| match (&x.value, x.name.eq_ignore_ascii_case("MultiSkins")) {
                (Value::Path(s), true) if s != "None" => Some(s.clone()),
                _ => None,
            },
        )
    {
        return Some(s);
    }
    let mesh = path(p, "Mesh")?;
    let (m, pk) = dx_level::actors::load_mesh(classes, &mesh)?;
    let r = *m.textures.first()?;
    let pkg = classes.packages.get(&pk.to_ascii_lowercase())?;
    Some(if r > 0 {
        format!("{pk}.{}", pkg.object_path(r))
    } else {
        pkg.object_path(r)
    })
}

/// Spawn a loose mesh (fragment or dropped item) with its own root entity.
#[allow(clippy::too_many_arguments)]
pub fn spawn_loose(
    commands: &mut Commands,
    assets: &mut RuntimeAssets,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    level: &dx_level::Level,
    mesh_path: &str,
    props: &[Property],
    draw_scale: f32,
    loose: Loose,
) -> Option<Entity> {
    let root = commands
        .spawn((
            Transform::from_translation(Vec3::from(ue_to_bevy(loose.pos.as_vec3().to_array()))),
            Visibility::default(),
        ))
        .id();
    let map = |p: [f32; 3]| ue_to_bevy(p);
    let Some((_m, parts, handles)) = spawn_mesh_parts(
        mesh_path, draw_scale, props, assets, images, meshes, materials, commands, root, &map, None,
    ) else {
        commands.entity(root).despawn();
        return None;
    };
    let at = loose.pos.as_vec3().to_array();
    let mut loose = loose;
    for (h, part) in handles.iter().zip(parts) {
        if let Some(mut m) = meshes.get_mut(h) {
            m.insert_attribute(
                Mesh::ATTRIBUTE_COLOR,
                light_part(&part, at, [0, 0, 0], 1.0, &level.lights),
            );
        }
        loose.parts.push((h.clone(), part));
    }
    loose.lit_at = loose.pos;
    commands.entity(root).insert(loose);
    Some(root)
}

/// Apply queued damage to decorations; break the ones that run out of hit points.
#[allow(clippy::too_many_arguments)]
pub fn deco_damage_system(
    mut commands: Commands,
    mut hits: ResMut<DecoHits>,
    mut map_gen: Local<u64>,
    mut health: Local<std::collections::HashMap<usize, f32>>,
    mut seed: Local<u64>,
    level: Res<LevelRes>,
    mut blockers: ResMut<Blockers>,
    mut decos: Query<&mut Deco>,
    mut assets: ResMut<RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut booms: ResMut<crate::explosion::Explosions>,
    mut debug_frames: Local<u32>,
    player: Res<crate::Player>,
) {
    // A new map (loader.rs): start this system's per-map state over.
    if crate::loader::new_map(&mut map_gen) {
        health.clear();
    }
    let _prof = crate::prof::scope("destruct::deco_damage_system");
    if *seed == 0 {
        *seed = 0x51_7CC1_B727_220A;
    }
    let lvl = &level.0;
    // Fragment bursts (DeusExMover.BlowItUp): FragmentSpread x VRand() apart, each flying off at
    // Fragment.CalcVelocity (spread + 100..250); the last one just drops.
    for bu in BURSTS
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
    {
        let fd = assets.classes.defaults(&bu.class);
        let frag_meshes: Vec<String> = fd
            .iter()
            .filter(|x| x.name.eq_ignore_ascii_case("Fragments"))
            .filter_map(|x| match &x.value {
                Value::Path(s) if s != "None" => Some(s.clone()),
                _ => None,
            })
            .collect();
        if frag_meshes.is_empty() {
            continue;
        }
        let num_types = (fv(&fd, "numFragmentTypes", frag_meshes.len() as f32) as usize)
            .clamp(1, frag_meshes.len());
        let frag_props: Vec<Property> = bu
            .skin
            .iter()
            .map(|s| Property {
                name: "Skin".into(),
                index: 0,
                value: Value::Path(s.clone()),
            })
            .collect();
        let elasticity = fv(&fd, "Elasticity", 0.5) as f64;
        let life = fv(&fd, "LifeSpan", 10.0);
        let mut rng = Rng(&mut seed);
        for i in 0..bu.count {
            let mesh = &frag_meshes[((rng.f() * num_types as f64) as usize).min(num_types - 1)];
            let vel = if i + 1 == bu.count {
                DVec3::ZERO
            } else {
                rng.vrand() * (bu.spread + rng.f() * 150.0 + 100.0)
            };
            let loose = Loose {
                pos: bu.pos + rng.vrand() * bu.spread,
                vel,
                rot: [rng.f() * 65536.0, rng.f() * 65536.0, rng.f() * 65536.0],
                spin: [
                    (rng.f() - 0.5) * 250000.0,
                    (rng.f() - 0.5) * 250000.0,
                    (rng.f() - 0.5) * 250000.0,
                ],
                elasticity,
                extent: DVec3::new(2.0, 2.0, 1.0),
                resting: false,
                life: Some(life + rng.f() as f32 * 2.0),
                parts: Vec::new(),
                lit_at: DVec3::ZERO,
            };
            if let Some(fe) = spawn_loose(
                &mut commands,
                &mut assets,
                &mut images,
                &mut meshes,
                &mut materials,
                lvl,
                mesh,
                &frag_props,
                bu.scale,
                loose,
            ) {
                commands.entity(fe).insert(LooseSound([
                    crate::sound::prop(&fd, "ImpactSound"),
                    crate::sound::prop(&fd, "MiscSound"),
                ]));
            }
        }
    }
    // Developer aid: DXVIEW_DEBUG_DECOHIT="ActorName" hits that decoration for 100 after about two seconds.
    if let Ok(name) = std::env::var("DXVIEW_DEBUG_DECOHIT") {
        *debug_frames += 1;
        if *debug_frames == 120 {
            if let Some((b, _)) = blockers
                .list
                .iter()
                .enumerate()
                .find(|(_, c)| lvl.actors[c.actor].name == name)
            {
                hits.0.push(DecoHit {
                    blocker: b,
                    damage: 100.0,
                    damage_type: "Shot",
                });
            }
        }
    }
    // DeusExDecoration.Trigger: a triggered explosive decoration blows up (Explode), e.g. the crates the Battery
    // Park subway's beam triggers set off.
    for tag in TRIGGERED.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        for (b, c) in blockers.list.iter().enumerate() {
            let p = &lvl.actors[c.actor].props;
            if c.radius > 0.0
                && !hits.0.iter().any(|h| h.blocker == b && h.damage >= 10000.0)
                && flag(p, "bExplosive")
                && blockers.deco.contains_key(&b)
                && matches!(find(p, "Tag"), Some(Value::Name(t)) if t.eq_ignore_ascii_case(&tag))
            {
                hits.0.push(DecoHit { blocker: b, damage: 10000.0, damage_type: "Burned" });
            }
        }
    }
    for hit in std::mem::take(&mut hits.0) {
        let b = hit.blocker;
        let Some(&e) = blockers.deco.get(&b) else {
            continue;
        };
        let Ok(mut deco) = decos.get_mut(e) else {
            continue;
        };
        let a = &lvl.actors[blockers.list[b].actor];
        let p = &a.props;
        if find(p, "GroundSpeed").is_some()
            || flag(p, "bStatic")
            || flag(p, "bInvincible")
            || find(p, "HitPoints").is_none()
        {
            continue;
        }
        if matches!(
            hit.damage_type,
            "TearGas" | "PoisonGas" | "Radiation" | "EMP" | "NanoVirus" | "Shocked"
        ) {
            continue;
        }
        let max = fv(p, "HitPoints", 20.0).max(1.0);
        let hp = health.entry(b).or_insert(max);
        let threshold = fv(p, "minDamageThreshold", 0.0);
        let explosive = flag(p, "bExplosive");
        if explosive && matches!(hit.damage_type, "Burned" | "Flamed") {
            // Blow up if hit by fire.
            *hp = 0.0;
        } else if hit.damage >= threshold {
            *hp -= hit.damage;
        } else if hit.damage_type == "Sabot" {
            *hp -= hit.damage * 0.5;
        } else if hit.damage_type == "Exploded" {
            *hp -= hit.damage * 0.25;
        }
        println!("DECO {} hit for {} -> {:.0} hp", a.name, hit.damage, *hp);
        if a.class == "Barrel1" && hit.damage >= threshold {
            crate::projgen::barrel_hit(b, blockers.list[b].actor, *hp <= 0.0);
        }
        if *hp > 0.0 {
            // ResetScaleGlow: darken with damage, never fully black.
            deco.scale_glow = *hp / max * 0.9 + 0.1;
            let ma = &lvl.mesh_actors[deco.mesh_actor];
            let draw = deco.body.location + deco.pre;
            for (h, pi) in &deco.parts {
                if let Some(mut m) = meshes.get_mut(h) {
                    m.insert_attribute(
                        Mesh::ATTRIBUTE_COLOR,
                        light_part(
                            &ma.parts[*pi],
                            draw.as_vec3().to_array(),
                            deco.rotation,
                            deco.scale_glow,
                            &lvl.lights,
                        ),
                    );
                }
            }
            continue;
        }

        // Destroyed: stop blocking, burst into fragments, drop contents.
        let center = deco.body.location;
        // The pad feels it break when close (ours): bigger things harder, gone by 400 units.
        let near = 1.0 - (center.distance(player.body.location) / 400.0).min(1.0);
        if near > 0.0 {
            let size = (fv(p, "Mass", 20.0) / 60.0).clamp(0.3, 1.0) as f32;
            crate::dualsense::event(crate::dualsense::Haptic::Smash(size * near as f32));
        }
        blockers.list[b].radius = 0.0;
        blockers.list[b].height = 0.0;
        blockers.pushable_mass.remove(&b);
        commands.entity(e).despawn();
        let (radius, height) = (
            fv(p, "CollisionRadius", 20.0) as f64,
            fv(p, "CollisionHeight", 20.0) as f64,
        );
        crate::trash::deco_destroyed(start_of(p).unwrap_or(center));
        // Containers.Destroyed: a trash container sitting on the ground spills paper (four tries at 75%) and,
        // half the time, a rat.
        if flag(p, "bGenerateTrash")
            && lvl
                .collision
                .trace(
                    center,
                    center - DVec3::new(0.0, 0.0, height + 8.0),
                    DVec3::ZERO,
                )
                .fraction
                < 1.0
        {
            let mut rng = Rng(&mut seed);
            for _ in 0..4 {
                if rng.f() < 0.75 {
                    let at = center
                        + DVec3::new(
                            radius / 2.0 - rng.f() * radius,
                            radius / 2.0 - rng.f() * radius,
                            height / 2.0 - rng.f() * height,
                        );
                    let mut dir = rng.vrand() * 20.0 + DVec3::new(20.0, 20.0, 0.0);
                    dir.z = 0.0;
                    crate::trash::spawn_trash("TrashPaper", at, dir);
                }
            }
            if rng.f() < 0.5 {
                let rat_h = 3.5;
                crate::spawn::request(crate::spawn::SpawnReq {
                    class: "Rat".into(),
                    pos: center - DVec3::new(0.0, 0.0, height - rat_h - 1.0),
                    yaw: (rng.f() * 65536.0) as i32,
                    transient: true,
                    props: Vec::new(),
                    generator: None,
                });
            }
        }
        let avg = (radius + height) / 2.0;
        let frag_class = path(p, "FragType").unwrap_or_else(|| "DeusEx.MetalFragment".into());
        let frag_class = frag_class
            .rsplit('.')
            .next()
            .unwrap_or("MetalFragment")
            .to_string();
        let fd = assets.classes.defaults(&frag_class);
        let frag_meshes: Vec<String> = fd
            .iter()
            .filter(|x| x.name.eq_ignore_ascii_case("Fragments"))
            .filter_map(|x| match &x.value {
                Value::Path(s) if s != "None" => Some(s.clone()),
                _ => None,
            })
            .collect();
        let num_types = (fv(&fd, "numFragmentTypes", frag_meshes.len() as f32) as usize)
            .clamp(1, frag_meshes.len().max(1));
        let skin = mesh_texture(&assets.classes, p);
        let frag_props: Vec<Property> = skin
            .iter()
            .map(|s| Property {
                name: "Skin".into(),
                index: 0,
                value: Value::Path(s.clone()),
            })
            .collect();
        let elasticity = fv(&fd, "Elasticity", 0.5) as f64;
        let life = fv(&fd, "LifeSpan", 10.0);
        let n = (avg / 5.0 + 1.0) as usize;
        let dsize = avg / 20.0;
        if explosive {
            // DeusExDecoration.Explode: the bang (SmallExplosion1 up to radius 128, else LargeExplosion1, heard out
            // to radius x 16), an explosion and three shock rings, and the staggered HurtRadius (Exploding state:
            // five rings of 2 x damage / 5 out to explosionRadius) - the same DamageRing grenades use.
            let radius = fv(p, "explosionRadius", 128.0) as f64;
            let damage = fv(p, "explosionDamage", 100.0);
            let snd = if radius <= 128.0 {
                "DeusExSounds.Generic.SmallExplosion1"
            } else {
                "DeusExSounds.Generic.LargeExplosion1"
            };
            crate::sound::play(snd, center, 1.0, (radius * 16.0) as f32, 1.0);
            booms.0.push(crate::explosion::Boom {
                pos: center,
                damage,
                radius,
                damage_type: "Exploded",
                from: None,
                rings: true,
                fireball: true,
                fx: 1,
            });
            println!(
                "DECO {} exploded (damage {damage}, radius {radius})",
                a.name
            );
        }
        let frag_speed = if explosive { 4.0 } else { 1.0 };
        let mut rng = Rng(&mut seed);
        if !frag_meshes.is_empty() {
            for _ in 0..n {
                let mesh = &frag_meshes[((rng.f() * num_types as f64) as usize).min(num_types - 1)];
                let speed = (rng.f() * 150.0 + 100.0) * frag_speed;
                let dir = rng.vrand();
                let spin = [
                    (rng.f() - 0.5) * 250000.0,
                    (rng.f() - 0.5) * 250000.0,
                    (rng.f() - 0.5) * 250000.0,
                ];
                let scale = (dsize * 0.5 + 0.7 * dsize * rng.f()) as f32;
                let start = center
                    + DVec3::new(
                        (rng.f() - 0.5) * radius,
                        (rng.f() - 0.5) * radius,
                        (rng.f() - 0.5) * height,
                    );
                let loose = Loose {
                    pos: start,
                    vel: dir * speed,
                    rot: [rng.f() * 65536.0, rng.f() * 65536.0, rng.f() * 65536.0],
                    spin,
                    elasticity,
                    extent: DVec3::new(2.0, 2.0, 1.0),
                    resting: false,
                    life: Some(life + rng.f() as f32 * 2.0),
                    parts: Vec::new(),
                    lit_at: DVec3::ZERO,
                };
                if let Some(fe) = spawn_loose(
                    &mut commands,
                    &mut assets,
                    &mut images,
                    &mut meshes,
                    &mut materials,
                    lvl,
                    mesh,
                    &frag_props,
                    scale,
                    loose,
                ) {
                    commands.entity(fe).insert(LooseSound([
                        crate::sound::prop(&fd, "ImpactSound"),
                        crate::sound::prop(&fd, "MiscSound"),
                    ]));
                }
            }
        }
        // Containers.Destroyed: numThings of Contents (Content2/3 each with a 30% chance instead).
        if let Some(c1) = path(p, "Contents") {
            let mut class = c1;
            if let Some(c2) = path(p, "Content2") {
                if rng.f() < 0.3 {
                    class = c2;
                }
            }
            if let Some(c3) = path(p, "Content3") {
                if rng.f() < 0.3 {
                    class = c3;
                }
            }
            let short = class.rsplit('.').next().unwrap_or("").to_string();
            let d = assets.classes.defaults(&short);
            let count = fv(p, "numThings", 1.0).max(0.0) as usize;
            if let Some(mesh) = path(&d, "Mesh") {
                let props = Arc::new(d.clone());
                for _ in 0..count {
                    let mut off = rng.vrand() * radius;
                    off.z = 0.0;
                    let loose = Loose {
                        pos: center + off,
                        vel: rng.vrand() * 50.0,
                        rot: [0.0, rng.f() * 65535.0, 0.0],
                        spin: [0.0; 3],
                        elasticity: 0.0,
                        extent: DVec3::new(
                            fv(&d, "CollisionRadius", 8.0) as f64 * 0.5,
                            fv(&d, "CollisionRadius", 8.0) as f64 * 0.5,
                            fv(&d, "CollisionHeight", 4.0) as f64,
                        ),
                        resting: false,
                        life: None,
                        parts: Vec::new(),
                        lit_at: DVec3::ZERO,
                    };
                    if let Some(ent) = spawn_loose(
                        &mut commands,
                        &mut assets,
                        &mut images,
                        &mut meshes,
                        &mut materials,
                        lvl,
                        &mesh,
                        &d,
                        fv(&d, "DrawScale", 1.0),
                        loose,
                    ) {
                        commands.entity(ent).insert(PickupItem {
                            actor: None,
                            class: short.clone(),
                            props: props.clone(),
                        });
                    }
                }
            }
        }
    }
}

/// Bounce, spin, settle and expire loose objects (Fragment.HitWall / Dying).
pub fn loose_physics(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut q: Query<(Entity, &mut Loose, &mut Transform, Option<&LooseSound>, Option<&LandSound>)>,
    mut noises: ResMut<crate::stealth::Noises>,
) {
    let _prof = crate::prof::scope("destruct::loose_physics");
    let dt = time.delta_secs_f64().min(0.05);
    let lvl = &level.0;
    for (e, mut l, mut tf, lsnd, land) in &mut q {
        if let Some(t) = l.life.as_mut() {
            *t -= dt as f32;
            if *t <= 0.0 {
                commands.entity(e).despawn();
                continue;
            }
        }
        if !l.resting {
            l.vel.z -= 950.0 * dt;
            let mut remaining = dt;
            for _ in 0..3 {
                if remaining <= 0.0 {
                    break;
                }
                let delta = l.vel * remaining;
                let hit = lvl.collision.trace(l.pos, l.pos + delta, l.extent);
                if !hit.blocked() {
                    l.pos += delta;
                    break;
                }
                // Move up to the wall (backed off a hair), then reflect with damping.
                let f = hit.fraction.max(0.0);
                l.pos += delta * f + hit.normal * 0.1;
                remaining *= 1.0 - f;
                let n = hit.normal;
                if let Some(LandSound(snd, vol)) = land {
                    if n.z > 0.7 && l.vel.z <= -200.0 {
                        crate::sound::play(snd, l.pos, *vol, 768.0, 1.0);
                        noises.0.push(crate::stealth::Noise {
                            pos: l.pos,
                            volume: *vol,
                            radius: 768.0,
                            maker: None,
                            instigator: None,
                            kind: crate::stealth::NoiseKind::Loud,
                        });
                    }
                }
                if let Some(ls) = lsnd {
                    if l.vel.length() > 60.0 {
                        let k = if crate::sound::frand() < 0.5 { 0 } else { 1 };
                        if let Some(snd) = ls.0[k].as_ref().or(ls.0[1 - k].as_ref()) {
                            crate::sound::play(
                                snd,
                                l.pos,
                                0.5 + crate::sound::frand() * 0.5,
                                512.0,
                                0.85 + crate::sound::frand() * 0.3,
                            );
                        }
                    }
                }
                l.vel = (l.vel.dot(n) * n * -2.0 + l.vel) * l.elasticity;
                let speed = l.vel.length();
                for s in l.spin.iter_mut() {
                    *s *= 0.75;
                }
                if (speed < 60.0 && n.z > 0.7) || speed < 1.0 {
                    // Come to rest lying flat, keeping its heading.
                    l.resting = true;
                    l.vel = DVec3::ZERO;
                    l.rot[0] = 0.0;
                    l.rot[2] = 0.0;
                    break;
                }
            }
            if !l.resting {
                for k in 0..3 {
                    l.rot[k] += l.spin[k] * dt;
                }
            }
            tf.translation = Vec3::from(ue_to_bevy(l.pos.as_vec3().to_array()));
            let r = [l.rot[0] as i32, l.rot[1] as i32, l.rot[2] as i32];
            let m = dx_level::actors::rotator_to_bevy_mat3(r);
            tf.rotation = Quat::from_mat3(&Mat3::from_cols(
                Vec3::from(m[0]),
                Vec3::from(m[1]),
                Vec3::from(m[2]),
            ));
            if l.pos.distance(l.lit_at) > 24.0 || l.resting {
                l.lit_at = l.pos;
                let at = l.pos.as_vec3().to_array();
                for (h, part) in &l.parts {
                    if let Some(mut m) = meshes.get_mut(h) {
                        m.insert_attribute(
                            Mesh::ATTRIBUTE_COLOR,
                            light_part(part, at, r, 1.0, &lvl.lights),
                        );
                    }
                }
            }
        }
    }
}

/// Spent cases to eject this frame: (ShellCasing class, position, velocity).
#[derive(Resource, Default)]
pub struct Shells(pub Vec<(&'static str, DVec3, DVec3)>);

/// DeusExAmmo.UseAmmo for bullet ammo: a shell casing pops out sideways from the shooter at chest height.
pub fn shell_for(ammo: Option<&str>, silenced: bool) -> Option<&'static str> {
    match ammo? {
        "Ammo10mm" | "Ammo762mm" | "Ammo3006" => Some(if silenced {
            "ShellCasingSilent"
        } else {
            "ShellCasing"
        }),
        "AmmoShell" | "AmmoSabot" => Some("ShellCasing2"),
        _ => None,
    }
}

/// Where and how fast: offset = R*X + 0.3R*Y + 0.8H up; velocity = (90..110)*Y + (-10..10)*X, flat.
pub fn shell_launch(
    center: DVec3,
    radius: f64,
    height: f64,
    forward: DVec3,
    seed: &mut u64,
) -> (DVec3, DVec3) {
    let mut rng = Rng(seed);
    let x = DVec3::new(forward.x, forward.y, 0.0).normalize_or_zero();
    let y = DVec3::new(-x.y, x.x, 0.0);
    let pos = center + x * radius + y * (0.3 * radius) + DVec3::new(0.0, 0.0, 0.8 * height);
    let mut vel = y * (rng.f() * 20.0 + 90.0) + x * (10.0 - rng.f() * 20.0);
    vel.z = 0.0;
    (pos, vel)
}

pub fn shell_system(
    mut commands: Commands,
    mut shells: ResMut<Shells>,
    mut assets: ResMut<RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    level: Res<LevelRes>,
    mut seed: Local<u64>,
) {
    let _prof = crate::prof::scope("destruct::shell_system");
    if *seed == 0 {
        *seed = 0x5EED_CA5E;
    }
    for (class, pos, vel) in shells.0.drain(..) {
        let d = assets.classes.defaults(class);
        let Some(Value::Path(mesh)) = find(&d, "Mesh").cloned() else {
            continue;
        };
        let mut rng = Rng(&mut seed);
        let spin = [
            (rng.f() - 0.5) * 60000.0,
            (rng.f() - 0.5) * 60000.0,
            (rng.f() - 0.5) * 60000.0,
        ];
        let loose = Loose {
            pos,
            vel,
            rot: [0.0, rng.f() * 65536.0, 0.0],
            spin,
            elasticity: 0.4,
            extent: DVec3::new(
                fv(&d, "CollisionRadius", 0.6) as f64,
                fv(&d, "CollisionRadius", 0.6) as f64,
                fv(&d, "CollisionHeight", 0.3) as f64,
            ),
            resting: false,
            life: Some(fv(&d, "LifeSpan", 10.0)),
            parts: Vec::new(),
            lit_at: DVec3::ZERO,
        };
        spawn_loose(
            &mut commands,
            &mut assets,
            &mut images,
            &mut meshes,
            &mut materials,
            &level.0,
            &mesh,
            &d,
            fv(&d, "DrawScale", 1.0),
            loose,
        );
    }
}

/// Items the player drops from the inventory screen: (class, where, velocity, PickupAmmoCount override).
#[derive(Resource, Default)]
/// Items the player dropped: (class, where, velocity, PickupAmmoCount for weapons, property overrides such as a
/// canister's AddAugs).
pub struct ItemThrows(pub Vec<(String, DVec3, DVec3, Option<i32>, Vec<Property>)>);

/// ScriptedPawn.DropWeapon: dropped weapons fall where the pawn stood and can be picked up.
#[allow(clippy::too_many_arguments)]
pub fn weapon_drop_system(
    mut commands: Commands,
    mut drops: ResMut<crate::ai::WeaponDrops>,
    mut assets: ResMut<RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    level: Res<LevelRes>,
    mut throws: ResMut<ItemThrows>,
) {
    let _prof = crate::prof::scope("destruct::weapon_drop_system");
    let player_drops: Vec<(String, DVec3, DVec3, Option<i32>, Vec<Property>)> = throws.0.drain(..).collect();
    let all = drops
        .0
        .drain(..)
        .map(|(c, p)| (c, p, DVec3::new(0.0, 0.0, 50.0), None, Vec::new()))
        .chain(player_drops);
    for (class, pos, vel, ammo, extra) in all {
        let mut d = assets.classes.defaults(&class);
        for x in extra {
            d.retain(|p| !(p.name.eq_ignore_ascii_case(&x.name) && p.index == x.index));
            d.push(x);
        }
        if let Some(n) = ammo {
            // Weapon.DropFrom: a dropped gun carries no ammo (grenades carry all of theirs).
            d.retain(|p| !(p.name.eq_ignore_ascii_case("PickupAmmoCount") && p.index == 0));
            d.push(Property {
                name: "PickupAmmoCount".into(),
                index: 0,
                value: Value::Int(n),
            });
        }
        let Some(mesh) = path(&d, "Mesh") else {
            continue;
        };
        let r = fv(&d, "CollisionRadius", 8.0) as f64;
        let loose = Loose {
            pos,
            vel,
            rot: [0.0, 0.0, 0.0],
            spin: [0.0; 3],
            elasticity: 0.0,
            extent: DVec3::new(r * 0.5, r * 0.5, fv(&d, "CollisionHeight", 4.0) as f64),
            resting: false,
            life: None,
            parts: Vec::new(),
            lit_at: DVec3::ZERO,
        };
        let props = Arc::new(d.clone());
        if let Some(ent) = spawn_loose(
            &mut commands,
            &mut assets,
            &mut images,
            &mut meshes,
            &mut materials,
            &level.0,
            &mesh,
            &d,
            fv(&d, "DrawScale", 1.0),
            loose,
        ) {
            if let Some(Value::Path(snd)) = find(&d, "LandSound") {
                if !snd.ends_with(".None") {
                    let vol = fv(&d, "TransientSoundVolume", 1.0);
                    commands.entity(ent).insert(LandSound(snd.clone(), vol));
                }
            }
            commands.entity(ent).insert(PickupItem {
                actor: None,
                class: class.clone(),
                props,
            });
        }
    }
}

/// A lit flare's glow: an orange camera-facing corona that follows the flare until it burns out.
#[derive(Component)]
pub struct FlareGlow {
    flare: Entity,
    life: f32,
}

/// Flare.LightFlare: the flare tumbles away (random spin), burns for 30 s and can't be picked up again.
#[allow(clippy::too_many_arguments)]
pub fn flare_system(
    mut commands: Commands,
    time: Res<Time>,
    mut throws: ResMut<crate::ai::FlareThrows>,
    (mut assets, mut images, mut meshes, mut materials): (
        ResMut<RuntimeAssets>,
        ResMut<Assets<Image>>,
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
    ),
    level: Res<LevelRes>,
    cam: Single<
        &Transform,
        (
            With<crate::FlyCam>,
            Without<crate::SkyCam>,
            Without<FlareGlow>,
        ),
    >,
    loose: Query<&Loose>,
    mut glows: Query<(Entity, &mut FlareGlow, &mut Transform), Without<crate::FlyCam>>,
    mut seed: Local<u64>,
) {
    let _prof = crate::prof::scope("destruct::flare_system");
    if *seed == 0 {
        *seed = 0xF1A2E;
    }
    for (pos, vel) in throws.0.drain(..) {
        let d = assets.classes.defaults("Flare");
        let Some(mesh) = path(&d, "Mesh") else {
            continue;
        };
        let mut rng = Rng(&mut seed);
        let l = Loose {
            pos,
            vel,
            rot: [0.0, rng.f() * 65536.0, 0.0],
            spin: [
                (rng.f() - 0.5) * 60000.0,
                (rng.f() - 0.5) * 60000.0,
                (rng.f() - 0.5) * 60000.0,
            ],
            elasticity: 0.3,
            extent: DVec3::splat(3.0),
            resting: false,
            life: Some(30.0),
            parts: Vec::new(),
            lit_at: DVec3::ZERO,
        };
        let Some(ent) = spawn_loose(
            &mut commands,
            &mut assets,
            &mut images,
            &mut meshes,
            &mut materials,
            &level.0,
            &mesh,
            &d,
            fv(&d, "DrawScale", 1.0),
            l,
        ) else {
            continue;
        };
        let img = crate::weapons::image_for(&mut assets, &mut images, "Effects.Corona.Corona_G");
        // LightHue 16: orange.
        let mat = materials.add(StandardMaterial {
            base_color_texture: img,
            base_color: Color::linear_rgb(1.6, 0.7, 0.2),
            unlit: true,
            double_sided: true,
            cull_mode: None,
            alpha_mode: AlphaMode::Add,
            ..default()
        });
        let quad = meshes.add(Rectangle::new(1.0, 1.0));
        commands.spawn((
            Mesh3d(quad),
            MeshMaterial3d(mat),
            Transform::from_scale(Vec3::splat(1.6)),
            Visibility::default(),
            FlareGlow {
                flare: ent,
                life: 30.0,
            },
        ));
    }
    let dt = time.delta_secs();
    for (e, mut g, mut tf) in &mut glows {
        g.life -= dt;
        let Ok(l) = loose.get(g.flare) else {
            commands.entity(e).despawn();
            continue;
        };
        if g.life <= 0.0 {
            commands.entity(e).despawn();
            continue;
        }
        tf.translation = Vec3::from(ue_to_bevy(l.pos.as_vec3().to_array()));
        tf.rotation = cam.rotation;
        // The burning flare lights its surroundings (LE_TorchWaver flicker).
        crate::dynlight::hold(
            g.flare.to_bits() | (1 << 61),
            l.pos + DVec3::new(0.0, 0.0, 4.0),
            crate::dynlight::FLARE,
            0.25,
        );
        let flicker = 1.4 + 0.2 * ((g.life * 23.0).sin() * (g.life * 7.0).cos());
        tf.scale = Vec3::splat(flicker);
    }
}

/// A new map is going in (loader.rs): this module's state of the map being left.
pub fn reset_level() {
    if let Ok(mut v) = BURSTS.lock() {
        v.clear();
    }
}
