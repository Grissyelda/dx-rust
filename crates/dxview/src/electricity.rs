//! ElectricityEmitter: a crackling beam shot out along the emitter's rotation (jittered by up to randomAngle each
//! frame) that stops at the level or a door and shocks the first thing in it for DamageAmount every damageTime.
//! It flickers: every flickerTime there's an even chance the beam is hidden (and harmless) for that slice.

use crate::actors::{Blockers, World};
use crate::ai::PlayerHealth;
use crate::movers::Movers;
use crate::weapons::{image_for, RuntimeAssets};
use crate::{LevelRes, Player};
use bevy::{
    asset::RenderAssetUsages, mesh::Indices, prelude::*,
};
use dx_level::collision::{trace_cylinder, Cylinder};
use dx_level::ue_to_bevy;
use glam::DVec3;
use imsim_core::movement::CollisionWorld;
use unreal_pkg::props::{find, Value};

#[derive(Component)]
pub struct Emitter {
    pub tag: String,
    pos: DVec3,
    rot: [i32; 3],
    random_angle: f64,
    damage: f32,
    damage_time: f32,
    flicker: bool,
    flicker_time: f32,
    on: bool,
    hidden: bool,
    last_damage: f32,
    last_flicker: f32,
}

/// Bolt segments at most (a kink every ~48 units over 5000).
const SEGS: usize = 120;
/// Vertices of the fixed-size bolt mesh: two crossed quads per segment.
const BOLT_VERTS: usize = SEGS * 8;

/// The bolt mesh at its full capacity, positions all at the origin (written in place each crackle; normals, UVs
/// and indices never change, so the mesh never changes size and the GPU slab is reused).
fn beam_mesh() -> Mesh {
    let mut idx: Vec<u32> = Vec::with_capacity(SEGS * 12);
    for q in 0..SEGS * 2 {
        let b = q as u32 * 4;
        idx.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
    }
    let mut m = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; BOLT_VERTS]);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; BOLT_VERTS]);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32, 0.0]; BOLT_VERTS]);
    m.insert_indices(Indices::U32(idx));
    m
}

pub fn spawn_emitters(
    commands: &mut Commands,
    level: &dx_level::Level,
    assets: &mut RuntimeAssets,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    for a in level
        .actors
        .iter()
        .filter(|a| a.class == "ElectricityEmitter")
    {
        let p = &a.props;
        let f = |n: &str, d: f32| match find(p, n) {
            Some(Value::Float(v)) => *v,
            Some(Value::Int(v)) => *v as f32,
            _ => d,
        };
        let b = |n: &str, d: bool| match find(p, n) {
            Some(Value::Bool(v)) => *v,
            _ => d,
        };
        let tex = match find(p, "beamTexture") {
            Some(Value::Path(t)) => t.clone(),
            _ => "Effects.Electricity.Nano_SFX".into(),
        };
        // The original draws a thin, jagged bolt; a flat electric blue reads the same at this width.
        let _ = image_for(assets, images, &tex);
        let mat = materials.add(StandardMaterial {
            base_color: Color::LinearRgba(LinearRgba::rgb(0.5, 0.6, 2.5)),
            unlit: true,
            double_sided: true,
            cull_mode: None,
            alpha_mode: AlphaMode::Add,
            ..default()
        });
        let mesh = meshes.add(beam_mesh());
        let l = a.location();
        let rot = match find(p, "Rotation") {
            Some(Value::Rotator(r)) => *r,
            _ => [0; 3],
        };
        let at = Vec3::from(ue_to_bevy(l));
        commands.spawn((
            Mesh3d(mesh),
            // The bolt reaches at most 5000 units from the emitter in some direction: a fixed box that size (the
            // mesh is rewritten in place and never re-bounded).
            bevy::camera::primitives::Aabb::from_min_max(at - Vec3::splat(96.0), at + Vec3::splat(96.0)),
            bevy::camera::visibility::NoAutoAabb,
            MeshMaterial3d(mat),
            Transform::default(),
            Visibility::Hidden,
            Emitter {
                tag: crate::devices::name_prop(p, "Tag"),
                pos: DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64),
                rot,
                random_angle: f("randomAngle", 8192.0) as f64,
                damage: f("DamageAmount", 2.0),
                damage_time: f("damageTime", 0.2),
                flicker: b("bFlicker", true),
                flicker_time: f("flickerTime", 0.02),
                on: b("bInitiallyOn", true),
                hidden: false,
                last_damage: 0.0,
                last_flicker: 0.0,
            },
        ));
    }
}

#[allow(clippy::too_many_arguments)]
pub fn emitter_system(
    time: Res<Time>,
    level: Res<LevelRes>,
    blockers: Res<Blockers>,
    mv: Res<Movers>,
    player: Res<Player>,
    mut ph: ResMut<PlayerHealth>,
    mut q: Query<(&mut Emitter, &mut Transform, &mut Visibility, &Mesh3d, &ViewVisibility)>,
    mut meshes: ResMut<Assets<Mesh>>,
    (mut pawn_hits, mut deco_hits, mut seed): (
        ResMut<crate::damage::PawnHits>,
        ResMut<crate::destruct::DecoHits>,
        Local<u64>,
    ),
) {
    let _prof = crate::prof::scope("electricity::emitter_system");
    let dt = time.delta_secs();
    if *seed == 0 {
        *seed = 0xE1EC_7A1C;
    }
    let mut rnd = || {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((*seed >> 33) as f64) / (1u64 << 31) as f64
    };
    let lvl = &level.0;
    let pc = Cylinder {
        center: player.body.location,
        radius: player.params.radius,
        height: player.params.height,
        actor: usize::MAX,
    };
    let ear = player.body.location + DVec3::new(0.0, 0.0, player.eye_height);
    for (mut e, mut tf, mut vis, mesh3d, seen) in &mut q {
        if !e.on {
            vis.set_if_neq(Visibility::Hidden);
            continue;
        }
        if e.flicker {
            e.last_flicker += dt;
            if e.last_flicker >= e.flicker_time {
                e.last_flicker = 0.0;
                e.hidden = rnd() < 0.5;
            }
        }
        if e.hidden {
            vis.set_if_neq(Visibility::Hidden);
            continue;
        }
        // LaserEmitter.TurnOn: AmbientSound Electricity4 at SoundVolume 128 (0 while the flicker hides the beam),
        // SoundRadius 64. Held only when the listener can see it or is close: through the ground it would carry
        // 1600 units up into the open.
        {
            let d = ear.distance(e.pos);
            // Silent behind the logo on the title map.
            let heard = !crate::menus::intro_map()
                && d < 1625.0
                && (d < 150.0 || lvl.collision.trace(ear, e.pos, DVec3::ZERO).fraction > 0.9);
            if heard {
                let key = 0xE_0000_0000u64 | ((e.pos.x as i64 as u64 & 0xffff) << 16) | (e.pos.y as i64 as u64 & 0xffff);
                crate::sound::hold(key, "Ambient.Ambient.Electricity4", Some(e.pos), 0.7 * 2.0 * 128.0 / 255.0, 1625.0, 1.0);
            }
        }
        // CalcTrace: jitter the direction, trace 5000 units, stop at the level or a mover.
        let ra = e.random_angle;
        let r = [
            e.rot[0] + ((0.5 - rnd()) * ra) as i32,
            e.rot[1] + ((0.5 - rnd()) * ra) as i32,
            0,
        ];
        let unit = std::f64::consts::TAU / 65536.0;
        let (p, y) = (r[0] as f64 * unit, r[1] as f64 * unit);
        let dir = DVec3::new(p.cos() * y.cos(), p.cos() * y.sin(), p.sin());
        let start = e.pos;
        let end = start + dir * 5000.0;
        let world = World {
            level: lvl,
            blockers: &blockers.list,
            ignore: None,
            pushable: None,
            movers: Some(&mv.rt),
        };
        let h = world.trace(start, end, DVec3::ZERO);
        let mut frac = h.fraction;
        let mut hit_player = false;
        if let Some(t) = trace_cylinder(&pc, start, end, DVec3::ZERO) {
            if t.fraction < frac && !ph.dead {
                frac = t.fraction;
                hit_player = true;
            }
        }
        e.last_damage += dt;
        if e.last_damage >= e.damage_time && (hit_player || h.actor.is_some()) {
            e.last_damage = 0.0;
            let at = start + (end - start) * frac;
            if hit_player {
                ph.damage(e.damage, "Shocked", DVec3::ZERO);
            } else if let Some(b) = h.actor {
                let a = &lvl.actors[blockers.list[b].actor];
                if find(&a.props, "GroundSpeed").is_some() {
                    pawn_hits.0.push(crate::damage::PawnHit {
                        blocker: b,
                        damage: e.damage,
                        damage_type: "Shocked",
                        hit_loc: at,
                        attacker: None,
                        attacker_pos: start,
                    });
                } else {
                    deco_hits.0.push(crate::destruct::DecoHit {
                        blocker: b,
                        damage: e.damage,
                        damage_type: "Shocked",
                    });
                }
            }
        }
        // Draw the bolt: a thin zig-zag (a kink every ~48 units, a few units off the line), crossed quads per
        // segment, rewritten every frame so it crackles - only while it was on screen lately or is close.
        if tf.translation != Vec3::ZERO || tf.rotation != Quat::IDENTITY {
            *tf = Transform::IDENTITY;
        }
        vis.set_if_neq(Visibility::Inherited);
        if !seen.get() && ear.distance(e.pos) > 2000.0 {
            continue;
        }
        let len = 5000.0 * frac;
        let end_pt = start + dir * len;
        let side = dir.cross(DVec3::Z).normalize_or(DVec3::X);
        let up = side.cross(dir).normalize_or(DVec3::Z);
        let n = ((len / 48.0).ceil() as usize).clamp(1, SEGS);
        let mut pts = [Vec3::ZERO; SEGS + 1];
        for (i, pt) in pts.iter_mut().enumerate().take(n + 1) {
            let t = i as f64 / n as f64;
            let mut p = start + (end_pt - start) * t;
            if i != 0 && i != n {
                p += side * ((rnd() - 0.5) * 10.0) + up * ((rnd() - 0.5) * 10.0);
            }
            *pt = Vec3::from(ue_to_bevy(p.as_vec3().to_array()));
        }
        let w = 1.5 * dx_level::METERS_PER_UNIT;
        let (sv, uv_) = (
            Vec3::from(ue_to_bevy(side.as_vec3().to_array())) * w,
            Vec3::from(ue_to_bevy(up.as_vec3().to_array())) * w,
        );
        let Some(mut m) = meshes.get_mut(&mesh3d.0) else { continue };
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(pos)) = m.attribute_mut(Mesh::ATTRIBUTE_POSITION) else { continue };
        let mut v = 0;
        for k in 0..n {
            let (a, b) = (pts[k], pts[k + 1]);
            for off in [sv, uv_] {
                pos[v] = (a - off).to_array();
                pos[v + 1] = (b - off).to_array();
                pos[v + 2] = (b + off).to_array();
                pos[v + 3] = (a + off).to_array();
                v += 4;
            }
        }
        // The segments past the end collapse onto the bolt's tip (no triangle drawn).
        let tip = pts[n].to_array();
        for p in pos[v..].iter_mut() {
            *p = tip;
        }
    }
}

impl Emitter {
    /// Trigger turns it on, UnTrigger off.
    pub fn set_on(&mut self, on: bool) {
        self.on = on;
    }
}
