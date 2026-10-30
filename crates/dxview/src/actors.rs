//! Mesh actors in the viewer: spawning, Unreal-style vertex lighting, and decoration physics
//! (falling at level start, pushing, sliding to a stop).

use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};
use dx_level::actors::{light_part, rotator_to_bevy_mat3, MeshActor};
use dx_level::collision::{trace_cylinder, Cylinder};
use dx_level::{ue_to_bevy, Level, SurfaceClass};
use glam::DVec3;
use imsim_core::body::{push_spin, push_velocity, Body, BodyPhysics};
use imsim_core::movement::{CollisionWorld, Hit};
use std::collections::HashMap;
use unreal_pkg::props::{find, Value};

/// Live blocking cylinders (decorations move, so this is the authority after load).
#[derive(Resource)]
pub struct Blockers {
    pub list: Vec<Cylinder>,
    /// Blocker index -> decoration entity.
    pub deco: HashMap<usize, Entity>,
    /// Blocker index -> mass, for pushable decorations light enough to move.
    pub pushable_mass: HashMap<usize, f64>,
}

/// Level BSP plus live actor cylinders, optionally ignoring one blocker (the mover itself).
pub struct World<'a> {
    pub level: &'a Level,
    pub blockers: &'a [Cylinder],
    pub ignore: Option<usize>,
    /// Masses of pushable blockers (by blocker index), for the player's push-through behavior.
    pub pushable: Option<&'a HashMap<usize, f64>>,
    /// Movers in their current placement.
    pub movers: Option<&'a [crate::movers::MoverRt]>,
}

impl CollisionWorld for World<'_> {
    fn trace(&self, start: DVec3, end: DVec3, extent: DVec3) -> Hit {
        let h = self.level.collision.trace(start, end, extent);
        let mut best = Hit {
            fraction: h.fraction,
            normal: h.normal,
            actor: None,
        };
        if let Some(rts) = self.movers {
            for (info, rt) in self.level.movers.iter().zip(rts) {
                if let Some(hm) = crate::movers::trace_mover(info, rt, start, end, extent) {
                    if hm.fraction < best.fraction {
                        best = hm;
                    }
                }
            }
        }
        for (i, c) in self.blockers.iter().enumerate() {
            // Destroyed decorations and gibbed pawns keep a zero-size cylinder: nothing to hit.
            if Some(i) == self.ignore || c.radius <= 0.0 || c.height <= 0.0 {
                continue;
            }
            // BlockPlayer stops only the player (whose world carries the push masses): shots, projectiles, sight
            // lines and other pawns pass (the training range's booth has one in the line of fire).
            if self.pushable.is_none() && self.level.players_only.contains(&c.actor) {
                continue;
            }
            if let Some(hc) = trace_cylinder(c, start, end, extent) {
                if hc.fraction < best.fraction {
                    best = Hit {
                        fraction: hc.fraction,
                        normal: hc.normal,
                        actor: Some(i),
                    };
                }
            }
        }
        best
    }

    fn pushable_mass(&self, actor: usize) -> Option<f64> {
        self.pushable.and_then(|m| m.get(&actor).copied())
    }
}

/// The player's world: like `World`, plus water zones (only the player swims so far).
pub struct SwimWorld<'a>(pub World<'a>);

impl CollisionWorld for SwimWorld<'_> {
    fn trace(&self, start: DVec3, end: DVec3, extent: DVec3) -> Hit {
        self.0.trace(start, end, extent)
    }
    fn pushable_mass(&self, actor: usize) -> Option<f64> {
        self.0.pushable_mass(actor)
    }
    fn water_at(&self, p: DVec3) -> Option<f64> {
        water_at(self.0.level, p)
    }
}

/// The fluid friction of the water zone at `p`, if it is one.
pub fn water_at(level: &Level, p: DVec3) -> Option<f64> {
    level
        .zone_water
        .get(level.collision.zone_at(p).max(0) as usize)
        .copied()
        .flatten()
        .map(|f| f as f64)
}

/// Vertex-animated actor (looping its current sequence).
#[derive(Component)]
pub struct Animated {
    pub mesh_actor: usize,
    pub parts: Vec<(Handle<Mesh>, usize)>,
    pub time: f32,
    /// Sequence playing instead of the idle loop (hit reactions, deaths).
    pub clip: Option<crate::weapons::Clip>,
    /// The pose drawn last frame (sequence start, frame count, frame) and the clip clock then, to notice changes.
    pub shown: Option<(usize, usize, f32)>,
    pub shown_t: f32,
    /// Blending from an earlier pose into the new sequence's first frame (UE1 TweenTime): (from pose, elapsed, length).
    pub tween: Option<((usize, usize, f32), f32, f32)>,
    /// Pawn.LipSynch's blend channels (mouth shapes, blinks), layered over the body pose.
    pub face: Face,
    pub face_shown: Option<FaceKey>,
}

/// The face channels as drawn: mouth tween (from frame, to frame, 0..1) and blink frame.
pub type FaceKey = (usize, usize, u16, Option<u16>);

/// Pawn.LipSynch: while the pawn speaks in a conversation its mouth tweens (0.1 s) to the Mouth<phoneme> blend
/// frame of each new phoneme, and closes (MouthClosed) when it stops; in a conversation it blinks, every 2 s with
/// a 40% chance (PlayBlendAnim 'Blink'). Blend frames are the Still pose with only the face changed, so they're
/// layered on the body pose as offsets from Still.
#[derive(Default, Clone)]
pub struct Face {
    /// Phoneme being spoken (b'A', b'E', b'F', b'M', b'O', b'T', b'U', b'X'); None = not speaking.
    pub phoneme: Option<u8>,
    /// In a conversation (LipSynch runs: blinks, gestures).
    pub talking: bool,
    /// LoopBaseConvoAnim's gesture clock (animTimer[2]).
    pub gesture_t: f32,
    mouth: Option<(usize, usize, f32)>,
    /// The mouth tween's length in seconds.
    mouth_len: f32,
    last: Option<u8>,
    was_speaking: bool,
    blink_timer: f32,
    blink: Option<f32>,
}

fn seq_start(mesh: &dx_level::actors::AnimMesh, name: &str) -> Option<(usize, usize, f32)> {
    mesh.seqs.iter().find(|s| s.name.eq_ignore_ascii_case(name)).map(|s| {
        (
            s.start_frame.max(0) as usize,
            s.num_frames.max(1) as usize,
            s.rate,
        )
    })
}

/// Advance LipSynch; returns what to layer this frame (None = nothing).
pub fn update_face(f: &mut Face, mesh: &dx_level::actors::AnimMesh, dt: f32) -> Option<FaceKey> {
    // Enhancement "Smooth Lip Sync" (after Joe Wintergreen's lipsync mod): phonemes blend over 0.36 s instead of
    // 0.1, the mouth eases shut over 0.3 s, blinks are checked every 0.5 s and play at a fifth of the speed.
    let smooth = crate::settings::get("SmoothLipSync") > 0.5;
    let (speak_t, close_t, blink_every, blink_rate) = if smooth { (0.36, 0.3, 0.5, 0.2) } else { (0.1, 0.1, 2.0, 1.0) };
    let closed = seq_start(mesh, "MouthClosed").map(|s| s.0);
    let cur = |f: &Face| f.mouth.map(|(a, b, k)| if k >= 0.5 { b } else { a });
    if let Some(p) = f.phoneme {
        f.was_speaking = true;
        if f.last != Some(p) {
            f.last = Some(p);
            let name = match p {
                b'A' => "MouthA",
                b'E' => "MouthE",
                b'F' => "MouthF",
                b'M' => "MouthM",
                b'O' => "MouthO",
                b'T' => "MouthT",
                b'U' => "MouthU",
                _ => "MouthClosed",
            };
            if let Some((to, _, _)) = seq_start(mesh, name) {
                let from = cur(f).or(closed).unwrap_or(to);
                f.mouth = Some((from, to, 0.0));
                f.mouth_len = speak_t;
            }
        }
    } else if f.was_speaking {
        f.was_speaking = false;
        f.last = None;
        if let (Some(c), Some(from)) = (closed, cur(f)) {
            f.mouth = Some((from, c, 0.0));
            f.mouth_len = close_t;
        }
    }
    if let Some(m) = f.mouth.as_mut() {
        m.2 = (m.2 + dt / f.mouth_len.max(0.01)).min(1.0);
        if m.2 >= 1.0 && Some(m.1) == closed && !f.was_speaking {
            f.mouth = None;
        }
    }
    if f.talking {
        f.blink_timer += dt;
        if f.blink_timer > blink_every {
            f.blink_timer = 0.0;
            if crate::sound::frand() < 0.4 {
                f.blink = Some(0.0);
            }
        }
    } else {
        f.blink_timer = 0.0;
    }
    let mut blink_key = None;
    if let Some(t) = f.blink.as_mut() {
        *t += dt;
        let (_, n, rate) = seq_start(mesh, "Blink").unwrap_or((0, 1, 10.0));
        let fr = *t * rate.max(1.0) * blink_rate;
        if fr >= n as f32 {
            f.blink = None;
        } else {
            blink_key = Some((fr * 64.0) as u16);
        }
    }
    let mouth = f.mouth.map(|(a, b, k)| (a, b, (k * 64.0) as u16));
    match (mouth, blink_key) {
        (None, None) => None,
        (m, b) => {
            let c = closed.unwrap_or(0);
            let (a, bb, k) = m.unwrap_or((c, c, 64));
            Some((a, bb, k, b))
        }
    }
}

/// The face offset for vertex `v` (mesh space), from the drawn [`FaceKey`].
pub fn face_offset(mesh: &dx_level::actors::AnimMesh, key: &FaceKey, still: usize, v: usize) -> [f32; 3] {
    let fv = mesh.frame_verts;
    let at = |fr: usize| mesh.verts.get(fr * fv + v).copied().unwrap_or([0.0; 3]);
    let s = at(still);
    let (a, b, k, blink) = *key;
    let k = k as f32 / 64.0;
    let (pa, pb) = (at(a), at(b));
    let mut o = [0.0f32; 3];
    for i in 0..3 {
        o[i] = (pa[i] + (pb[i] - pa[i]) * k) - s[i];
    }
    if let Some(bf) = blink {
        if let Some((bs, n, _)) = seq_start(mesh, "Blink") {
            let fr = (bf as f32 / 64.0).min(n as f32 - 1.0);
            let p = mesh.sample(bs, n, fr, v);
            for i in 0..3 {
                o[i] += p[i] - s[i];
            }
        }
    }
    o
}

/// Helios eyelids (dx_level::actors::LID_FIX): the part's texture coordinates with each blinking lid vertex slid up
/// toward the crease above it as far as the blink has closed it, or None when the part has no lid (or the fix is off).
/// Lid vertices = those the Blink sequence moves a little (the lid edges; the glasses' far-moving hidden bits are left
/// alone); the crease = the nearest still vertex above it in the picture in a face they share.
pub fn lid_uvs(mesh: &dx_level::actors::AnimMesh, part: &dx_level::actors::MeshPart, key: Option<&FaceKey>, still: usize) -> Option<Vec<[f32; 2]>> {
    if !dx_level::actors::LID_FIX.load(std::sync::atomic::Ordering::Relaxed) {
        return None;
    }
    let (bs, bn, _) = seq_start(mesh, "Blink")?;
    let fv = mesh.frame_verts;
    let at = |fr: usize, v: usize| mesh.verts.get(fr * fv + v).copied().unwrap_or([0.0; 3]);
    let dist = |a: [f32; 3], b: [f32; 3]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
    // Per part vertex: the lid's full travel (0 = not a lid vertex).
    let travel: Vec<f32> = part
        .vert_ids
        .iter()
        .map(|&v| {
            let d = (bs..bs + bn).map(|f| dist(at(f, v as usize), at(still, v as usize))).fold(0.0, f32::max);
            if d > 1.0 && d < 40.0 { d } else { 0.0 }
        })
        .collect();
    if travel.iter().all(|t| *t == 0.0) {
        return None;
    }
    // How closed it is now.
    let amount = |i: usize| -> f32 {
        let Some(bf) = key.and_then(|k| k.3) else { return 0.0 };
        let fr = (bf as f32 / 64.0).min(bn as f32 - 1.0);
        let v = part.vert_ids[i] as usize;
        (dist(mesh.sample(bs, bn, fr, v), at(still, v)) / travel[i]).clamp(0.0, 1.0)
    };
    let mut uv = part.uvs.clone();
    let mut target: Vec<Option<f32>> = vec![None; uv.len()];
    for tri in part.indices.chunks_exact(3) {
        for &a in tri {
            let a = a as usize;
            if travel[a] == 0.0 {
                continue;
            }
            for &b in tri {
                let b = b as usize;
                if travel[b] == 0.0 && part.uvs[b][1] < part.uvs[a][1] - 0.5 / 255.0 {
                    let t = target[a].get_or_insert(f32::MIN);
                    *t = t.max(part.uvs[b][1]);
                }
            }
        }
    }
    let mut any = false;
    for i in 0..uv.len() {
        if travel[i] == 0.0 {
            continue;
        }
        let k = amount(i);
        if k <= 0.0 {
            continue;
        }
        let to = target[i].filter(|t| *t > f32::MIN).unwrap_or(part.uvs[i][1] - 2.0 / 255.0);
        uv[i][1] = part.uvs[i][1] + (to - part.uvs[i][1]) * k;
        any = true;
    }
    let _ = any;
    Some(uv)
}

/// Which `Level::mesh_actors` entry an entity draws.
#[derive(Component)]
pub struct MeshActorRef(pub usize);

#[derive(Component)]
pub struct Deco {
    pub body: Body,
    pub blocker: Option<usize>,
    pub mass: f64,
    pub pushable: bool,
    /// Drawing offset (PrePivot) from the collision center, Unreal units.
    pub pre: DVec3,
    pub rotation: [i32; 3],
    pub scale_glow: f32,
    pub parts: Vec<(Handle<Mesh>, usize)>,
    pub mesh_actor: usize,
    pub last_lit: DVec3,
    /// Rotation changed since the transform was last updated.
    pub spun: bool,
    pub name: String,
    pub start: DVec3,
    /// Decoration.Buoyancy: floats in water when greater than Mass.
    pub buoyancy: f64,
    /// DeusExDecoration.bFloating: gently rocking on the water.
    pub floating: bool,
    pub orig_rot: [i32; 3],
    /// Thrown by the player and not yet landed (its landing noise is attributed to the player).
    pub thrown_by_player: bool,
    /// A body thrown from the player's hands (carry.rs corpse drops): thuds and slides when it lands.
    pub thrown_body: bool,
    /// DeusExDecoration.Bump: the PushSound plays while it's shoved and stops 0.2 s after the last push.
    pub push_snd: f32,
    /// Starts in PHYS_Falling (settles under gravity when restored from a save).
    pub falls: bool,
}

/// A PF_Environment mesh part: Unreal maps its texture from the surface normal as seen from the camera (a sphere
/// map), so the metal shines as it turns.
#[derive(Component)]
pub struct EnvMapped;

/// UV_0 = 0.5 + 0.5 * (view-space normal x, -y) for every environment-mapped part, each frame.
pub fn env_map_system(
    cams: Query<&GlobalTransform, (With<crate::FlyCam>, Without<crate::SkyCam>, Without<EnvMapped>)>,
    parts: Query<(Entity, &Mesh3d, &GlobalTransform, &ViewVisibility), With<EnvMapped>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut last: Local<HashMap<Entity, Quat>>,
) {
    let _prof = crate::prof::scope("actors::env_map_system");
    let Some(cam) = cams.iter().next() else { return };
    let cam_t = cam.compute_transform();
    let inv = cam_t.rotation.inverse();
    for (e, m, gt, vis) in &parts {
        // Only what is on screen and near enough for the shine to show (a sphere map follows the view, so every
        // env-mapped bottle and camera on the map was re-uploaded every frame), and only when the view turned.
        let t = gt.compute_transform();
        if !vis.get() || t.translation.distance(cam_t.translation) > 30.0 {
            continue;
        }
        let rot = inv * t.rotation;
        // A turn of under about a degree (two beyond 10 m) doesn't move the shine visibly.
        let far = t.translation.distance(cam_t.translation) > 10.0;
        let keep = if far { 0.99985 } else { 0.99996 };
        if last.get(&e).is_some_and(|q| q.dot(rot).abs() > keep) {
            continue;
        }
        last.insert(e, rot);
        let Some(mut mesh) = meshes.get_mut(&m.0) else { continue };
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(n)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else {
            continue;
        };
        let uvs: Vec<[f32; 2]> = n
            .iter()
            .map(|n| {
                let v = rot * Vec3::from(*n);
                [0.5 + 0.5 * v.x, 0.5 - 0.5 * v.y]
            })
            .collect();
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    }
}

pub fn material_for(
    materials: &mut Assets<StandardMaterial>,
    cache: &mut HashMap<(Option<usize>, SurfaceClass), Handle<StandardMaterial>>,
    handles: &[Handle<Image>],
    tex: Option<usize>,
    class: SurfaceClass,
) -> Handle<StandardMaterial> {
    cache
        .entry((tex, class))
        .or_insert_with(|| {
            materials.add(StandardMaterial {
                base_color_texture: tex.map(|i| handles[i].clone()),
                // Unreal modulation is 2x in gamma space (mid-gray leaves the background unchanged); in linear
                // terms that is a factor of 2^2.2 on the sampled texture. Deus Ex hides mesh sections this way.
                base_color: match (tex.is_some(), class) {
                    (true, SurfaceClass::Modulated) => {
                        Color::LinearRgba(LinearRgba::rgb(4.59, 4.59, 4.59))
                    }
                    (true, _) => Color::WHITE,
                    _ => Color::srgb(0.6, 0.6, 0.6),
                },
                unlit: true,
                double_sided: true,
                cull_mode: None,
                alpha_mode: match class {
                    SurfaceClass::Opaque => AlphaMode::Opaque,
                    SurfaceClass::Masked => AlphaMode::Mask(0.5),
                    SurfaceClass::Translucent => AlphaMode::Add,
                    SurfaceClass::Water => AlphaMode::Blend,
                    SurfaceClass::Modulated => AlphaMode::Multiply,
                },
                ..default()
            })
        })
        .clone()
}

fn bevy_rotation(rot: [i32; 3]) -> Quat {
    let m = rotator_to_bevy_mat3(rot);
    Quat::from_mat3(&Mat3::from_cols(
        Vec3::from(m[0]),
        Vec3::from(m[1]),
        Vec3::from(m[2]),
    ))
}

fn part_mesh(
    ma: &MeshActor,
    pi: usize,
    location: [f32; 3],
    level: &Level,
    scale_glow: f32,
) -> Mesh {
    let part = &ma.parts[pi];
    // Lit like Unreal lights actors: only by the lights that can see it (shadows darken it).
    let lights = crate::dynlight::lights_at(level, location, 32.0);
    let colors = light_part(part, location, ma.rotation, scale_glow, &lights);
    let pos: Vec<[f32; 3]> = part.positions.iter().map(|p| ue_to_bevy(*p)).collect();
    let nrm: Vec<[f32; 3]> = part.normals.iter().map(|n| [n[0], n[2], n[1]]).collect();
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, part.uvs.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(part.indices.clone()));
    mesh
}

pub fn spawn_mesh_actors(
    commands: &mut Commands,
    level: &Level,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    handles: &[Handle<Image>],
) -> Blockers {
    let mut cache = HashMap::new();
    let mut blockers = Blockers {
        list: level.blockers.clone(),
        deco: HashMap::new(),
        pushable_mass: HashMap::new(),
    };
    let actor_blocker: HashMap<usize, usize> = level
        .blockers
        .iter()
        .enumerate()
        .map(|(i, c)| (c.actor, i))
        .collect();

    for (mi, ma) in level.mesh_actors.iter().enumerate() {
        let info = &level.actors[ma.actor];
        let p = &info.props;
        let physics = match find(p, "Physics") {
            Some(Value::Byte(b)) => *b,
            _ => 0,
        };
        let is_deco = find(p, "bPushable").is_some();
        // Pawns are saved slightly above the floor and dropped by physics at level start.
        let is_pawn = find(p, "GroundSpeed").is_some();
        let pushable = matches!(find(p, "bPushable"), Some(Value::Bool(true)));
        let mass = match find(p, "Mass") {
            Some(Value::Float(v)) => *v as f64,
            _ => 100.0,
        };
        let f = |n: &str| match find(p, n) {
            Some(Value::Float(v)) => *v as f64,
            _ => 0.0,
        };

        let mut parts = Vec::new();
        let parent = commands
            .spawn((
                Transform::from_translation(Vec3::from(ue_to_bevy(ma.location)))
                    .with_rotation(bevy_rotation(ma.rotation)),
                if dx_level::actors::out_of_world(p) || matches!(find(p, "bHidden"), Some(Value::Bool(true))) {
                    crate::scriptworld::mark_hidden(ma.actor, true);
                    Visibility::Hidden
                } else {
                    Visibility::default()
                },
            ))
            .id();
        for (pi, part) in ma.parts.iter().enumerate() {
            if part.indices.is_empty() {
                continue;
            }
            let h = meshes.add(part_mesh(ma, pi, ma.location, level, ma.scale_glow));
            let mat = material_for(materials, &mut cache, handles, part.texture, part.class);
            let child = commands
                .spawn((Mesh3d(h.clone()), MeshMaterial3d(mat)))
                .id();
            if part.env {
                commands.entity(child).insert(EnvMapped);
            }
            commands.entity(parent).add_child(child);
            parts.push((h, pi));
        }

        commands.entity(parent).insert(MeshActorRef(mi));
        if find(p, "ItemName").is_some() && find(p, "PickupMessage").is_some() {
            commands.entity(parent).insert(crate::pickups::PickupItem {
                actor: Some(ma.actor),
                class: info.class.clone(),
                props: std::sync::Arc::new(info.props.clone()),
            });
        }
        if ma.anim.is_some() {
            commands.entity(parent).insert(Animated {
                mesh_actor: mi,
                parts: parts.clone(),
                time: 0.0,
                clip: None,
                shown: None,
                shown_t: 0.0,
                tween: None,
                face: Face::default(),
                face_shown: None,
            });
        }
        // Decorations (and anything set to fall) get live physics.
        if is_deco || physics == 2 || is_pawn {
            let loc = info.location();
            let center = DVec3::new(loc[0] as f64, loc[1] as f64, loc[2] as f64);
            let draw = DVec3::new(
                ma.location[0] as f64,
                ma.location[1] as f64,
                ma.location[2] as f64,
            );
            let blocker = actor_blocker.get(&ma.actor).copied();
            let body = Body {
                location: center,
                velocity: DVec3::ZERO,
                physics: if physics == 2 || (is_pawn && physics == 1) {
                    BodyPhysics::Falling
                } else {
                    BodyPhysics::None
                },
                radius: f("CollisionRadius").max(1.0),
                height: f("CollisionHeight").max(1.0),
                landed_vz: None,
            };
            if let Some(b) = blocker {
                blockers.deco.insert(b, parent);
                if pushable && mass <= 100.0 {
                    blockers.pushable_mass.insert(b, mass);
                }
            }
            commands.entity(parent).insert(Deco {
                body,
                blocker,
                mass,
                pushable,
                pre: draw - center,
                rotation: ma.rotation,
                scale_glow: ma.scale_glow,
                parts,
                mesh_actor: mi,
                last_lit: draw,
                spun: false,
                name: info.name.clone(),
                start: center,
                buoyancy: f("Buoyancy"),
                floating: matches!(find(p, "bFloating"), Some(Value::Bool(true))),
                orig_rot: ma.rotation,
                thrown_by_player: false,
                thrown_body: false,
                push_snd: 0.0,
                falls: physics == 2 || is_pawn,
            });
        }
    }
    blockers
}

/// Push decorations the player bumped into (DeusExDecoration.Bump).
pub fn apply_pushes(
    bumped: &[usize],
    pusher_loc: DVec3,
    pusher_height: f64,
    pusher_vel: DVec3,
    blockers: &Blockers,
    decos: &mut Query<&mut Deco>,
) {
    for b in bumped {
        let Some(&e) = blockers.deco.get(b) else {
            continue;
        };
        let Ok(mut d) = decos.get_mut(e) else {
            continue;
        };
        if !d.pushable {
            continue;
        }
        // Not when bumped from above or below.
        if (d.body.location.z - pusher_loc.z).abs() >= d.body.height + pusher_height - 1.0 {
            continue;
        }
        if let Some((v, scale)) = push_velocity(pusher_vel, d.mass, crate::carry::muscle_mult()) {
            if v.length_squared() > 0.0 {
                d.body.velocity = v;
                if !d.floating && d.mass > 15.0 {
                    d.push_snd = 0.2;
                }
                if d.body.physics != BodyPhysics::Falling {
                    d.body.physics = BodyPhysics::Rolling;
                }
                // Fake spin depending on where it was pushed from.
                let to = pusher_loc - d.body.location;
                let world_yaw = (to.y.atan2(to.x) / std::f64::consts::TAU * 65536.0) as i32;
                let spin = push_spin(world_yaw - d.rotation[1], scale);
                d.rotation[1] += spin;
                d.spun = true;
            }
        }
    }
}

/// How much of a body is under water (0..1): the share of points from its bottom to its top that lie in a water
/// zone, and that zone's fluid friction.
fn immersion(level: &dx_level::Level, c: DVec3, height: f64) -> (f64, f64) {
    let mut wet = 0;
    let mut fric = 0.0;
    const N: usize = 8;
    for i in 0..N {
        let z = c.z - height + 2.0 * height * (i as f64 + 0.5) / N as f64;
        let zone = level.collision.zone_at(DVec3::new(c.x, c.y, z));
        if let Some(Some(f)) = level.zone_water.get(zone.max(0) as usize) {
            wet += 1;
            fric = *f as f64;
        }
    }
    (wet as f64 / N as f64, fric)
}

/// Decoration physics: falling (with buoyancy and drag in water), rolling when pushed, bFloating rocking on
/// the waves, and the landing thud (a LoudNoise; falls faster than 500 hurt the object).
#[allow(clippy::too_many_arguments)]
/// The decoration whose path deco_physics prints (loader's throwbody test).
pub static BODY_LOG: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub fn deco_physics(
    time: Res<Time>,
    level: Res<crate::LevelRes>,
    mut blockers: ResMut<Blockers>,
    mut q: Query<(&mut Deco, &mut Transform)>,
    mut meshes: ResMut<Assets<Mesh>>,
    walk_test: Option<Res<crate::WalkTest>>,
    movers: Option<Res<crate::movers::Movers>>,
    (mut noises, mut deco_hits): (
        ResMut<crate::stealth::Noises>,
        ResMut<crate::destruct::DecoHits>,
    ),
) {
    let _prof = crate::prof::scope("actors::deco_physics");
    let dt = if walk_test.is_some() {
        1.0 / 60.0
    } else {
        time.delta_secs_f64().min(0.1)
    };
    if dt <= 0.0 {
        return;
    }
    let t = time.elapsed_secs_f64();
    let level = &level.0;
    let log_name = BODY_LOG.lock().ok().and_then(|g| g.clone());
    for (mut d, mut tf) in &mut q {
        let d = &mut *d;
        if log_name.as_deref() == Some(d.name.as_str()) && (t * 5.0).floor() != ((t - dt) * 5.0).floor() {
            println!(
                "BODY {} at {:.0?} vel {:.0?} {:?} size r{:.0} h{:.0} draw {:.0?}",
                d.name, d.body.location, d.body.velocity, d.body.physics, d.body.radius, d.body.height, tf.translation
            );
        }
        // DeusExDecoration.Tick: rock gently while floating (a 4 second cycle around the resting rotation).
        let mut bob = 0.0;
        if d.floating {
            let ang = std::f64::consts::TAU * t / 4.0;
            let o = d.orig_rot;
            // Plus riding the water's swell (water.rs): up and down with the surface and tilted with its slope.
            let (h, pitch, roll) = crate::water::bob(d.body.location, t);
            bob = h;
            d.rotation = [
                o[0] + (ang.sin() * 512.0) as i32 + pitch,
                o[1] + (ang.sin() * 256.0) as i32,
                o[2] + (ang.cos() * 512.0) as i32 + roll,
            ];
            d.spun = true;
        }
        if d.spun {
            d.spun = false;
            tf.rotation = bevy_rotation(d.rotation);
        }
        if d.push_snd > 0.0 {
            d.push_snd -= dt as f32;
            let props = &level.actors[level.mesh_actors[d.mesh_actor].actor].props;
            if let Some(snd) = crate::sound::prop(props, "PushSound") {
                crate::sound::hold(
                    crate::sound::key_of(&d.name),
                    &snd,
                    Some(d.body.location),
                    1.0,
                    128.0,
                    1.0,
                );
            }
        }
        if d.body.physics == BodyPhysics::None {
            if d.floating {
                let draw = d.body.location + d.pre + DVec3::new(0.0, 0.0, bob);
                tf.translation = Vec3::from(ue_to_bevy([draw.x as f32, draw.y as f32, draw.z as f32]));
            }
            continue;
        }
        let steps = (dt / (1.0 / 120.0)).ceil().max(1.0) as usize;
        let mut moved = false;
        let mut landed = None;
        let mut landed_flat = glam::DVec2::ZERO;
        let mut wet_now = 0.0;
        for _ in 0..steps {
            let h = dt / steps as f64;
            let world = World {
                level,
                blockers: &blockers.list,
                ignore: d.blocker,
                pushable: None,
                movers: movers.as_deref().map(|m| m.rt.as_slice()),
            };
            // In water: buoyancy (Buoyancy/Mass of the submerged share) against gravity, and fluid friction.
            let (wet, fric) = if d.body.physics == BodyPhysics::Falling {
                immersion(level, d.body.location, d.body.height)
            } else {
                (0.0, 0.0)
            };
            wet_now = wet;
            let mut g = -950.0;
            if wet > 0.0 {
                let net = if d.mass > 0.0 {
                    d.buoyancy / d.mass * wet
                } else {
                    0.0
                };
                g *= 1.0 - net;
                d.body.velocity *= (1.0 - fric * wet * h).max(0.0);
            }
            let before = d.body.velocity;
            let m = d.body.tick(&world, DVec3::new(0.0, 0.0, g), 8.0, h);
            moved |= m;
            if let Some(vz) = d.body.landed_vz {
                landed = Some(vz);
                landed_flat = before.truncate();
            }
        }
        // ZoneChange: buoyant things in water start floating; out of it they stop.
        if wet_now > 0.0 && !d.floating && d.buoyancy > d.mass {
            d.floating = true;
            d.orig_rot = d.rotation;
        } else if wet_now == 0.0 && d.floating && d.body.physics == BodyPhysics::Falling {
            d.floating = false;
            d.rotation = d.orig_rot;
            d.spun = true;
        }
        // A thrown body (ours, the user's ask: like a thrown crate): a thud, then it slides on along the floor.
        if let (Some(_), true) = (landed, d.thrown_body) {
            d.thrown_body = false;
            d.thrown_by_player = false;
            crate::sound::play("DeusExSounds.Generic.BodyThud", d.body.location, 1.0, 0.0, 1.0);
            noises.0.push(crate::stealth::Noise {
                pos: d.body.location,
                volume: 1.0,
                radius: 512.0,
                maker: None,
                instigator: Some(crate::ai::Target::Player),
                kind: crate::stealth::NoiseKind::Loud,
            });
            if landed_flat.length() > 40.0 {
                d.body.velocity = (landed_flat * 0.6).extend(0.0);
                d.body.physics = BodyPhysics::Rolling;
            }
            landed = None;
        }
        if let Some(vz) = landed {
            // DeusExMover.SupportActor: landing on a door or lift faster than 500 "stomps" it,
            // 1 - standingMass / Mass x Velocity.Z / 30 damage.
            if vz < -500.0 {
                if let Some(mv) = movers.as_ref() {
                    let c = d.body.location;
                    let foot = c - DVec3::new(0.0, 0.0, d.body.height + 8.0);
                    if let Some((i, _)) = crate::movers::mover_on_line(level, &mv.rt, c, foot) {
                        let base_mass = match find(&level.actors[level.movers[i].actor].props, "Mass") {
                            Some(Value::Float(m)) => *m as f64,
                            _ => 100.0,
                        }
                        .max(1.0);
                        let dmg = 1.0 - d.mass.max(1.0) / base_mass * vz / 30.0;
                        crate::movers::damage_mover(i, dmg as f32, "stomped");
                    }
                }
            }
            // Landed: lie flat; a thud loud enough for pawns to come looking, blamed on whoever threw it.
            d.rotation[0] = 0;
            d.rotation[2] = 0;
            d.spun = true;
            if vz <= -200.0 {
                // DeusExDecoration.Landed: a hit sound by FragType and Mass.
                let props = &level.actors[level.mesh_actors[d.mesh_actor].actor].props;
                let frag = match find(props, "fragType") {
                    Some(Value::Path(p)) => p.rsplit('.').next().unwrap_or("").to_string(),
                    _ => String::new(),
                };
                let base = match frag.as_str() {
                    "WoodFragment" => Some("WoodHit"),
                    "MetalFragment" => Some("MetalHit"),
                    "PlasticFragment" => Some("PlasticHit"),
                    "GlassFragment" => Some("GlassHit"),
                    _ => Some("PaperHit"),
                };
                if let Some(b) = base {
                    crate::sound::play(
                        &format!(
                            "DeusExSounds.Generic.{b}{}",
                            if d.mass <= 20.0 { 1 } else { 2 }
                        ),
                        d.body.location,
                        1.0,
                        0.0,
                        1.0,
                    );
                }
                let by = if d.thrown_by_player {
                    Some(crate::ai::Target::Player)
                } else {
                    None
                };
                noises.0.push(crate::stealth::Noise {
                    pos: d.body.location,
                    volume: 1.0,
                    radius: 768.0,
                    maker: None,
                    instigator: by,
                    kind: crate::stealth::NoiseKind::Loud,
                });
            }
            // Enhancement (Thrown Objects Make Noise): anything the player throws is heard landing, however softly,
            // further for heavy things, and every armed pawn in earshot goes to look.
            if d.thrown_by_player && crate::stealth::thrown_noise() && (vz <= -60.0 || landed_flat.length() > 60.0) {
                noises.0.push(crate::stealth::Noise {
                    pos: d.body.location,
                    volume: 1.0,
                    radius: (600.0 + d.mass as f32 * 6.0).clamp(600.0, 1400.0),
                    maker: None,
                    instigator: Some(crate::ai::Target::Player),
                    kind: crate::stealth::NoiseKind::Alert,
                });
            }
            // DeusExDecoration.Landed: explosives go off landing faster than 425 in any direction, others when
            // falling faster than 500.
            let explosive = matches!(
                find(
                    &level.actors[level.mesh_actors[d.mesh_actor].actor].props,
                    "bExplosive"
                ),
                Some(Value::Bool(true))
            );
            let land_speed = (vz * vz + d.body.velocity.truncate().length_squared()).sqrt();
            if vz < -500.0 || (explosive && land_speed > 425.0) {
                if let Some(b) = d.blocker {
                    deco_hits.0.push(crate::destruct::DecoHit {
                        blocker: b,
                        damage: (1.0 - vz / 30.0) as f32,
                        damage_type: "Fell",
                    });
                }
            }
            d.thrown_by_player = false;
        }
        if !moved && !d.floating {
            continue;
        }
        if let Some(b) = d.blocker {
            blockers.list[b].center = d.body.location;
        }
        let draw = d.body.location + d.pre + DVec3::new(0.0, 0.0, bob);
        tf.translation = Vec3::from(ue_to_bevy([draw.x as f32, draw.y as f32, draw.z as f32]));
        // Re-light when it has moved noticeably or come to rest.
        if draw.distance(d.last_lit) > 12.0 {
            let ma = &level.mesh_actors[d.mesh_actor];
            let loc = [draw.x as f32, draw.y as f32, draw.z as f32];
            // Walking into a shadow darkens a pawn (only the lights that can see it count).
            let c = d.body.location;
            let lit_by = crate::dynlight::lights_at(level, [c.x as f32, c.y as f32, c.z as f32], d.body.height as f32);
            for (h, pi) in &d.parts {
                if let Some(mut m) = meshes.get_mut(h) {
                    m.insert_attribute(
                        Mesh::ATTRIBUTE_COLOR,
                        light_part(
                            &ma.parts[*pi],
                            loc,
                            d.rotation,
                            d.scale_glow,
                            &lit_by,
                        ),
                    );
                }
            }
            d.last_lit = draw;
        }
    }
}

/// The sequence (start, frame count) and fractional frame an animated actor is showing.
pub fn current_frame(a: &Animated, anim: &dx_level::actors::ActorAnim) -> (usize, usize, f32) {
    match a.clip.as_ref() {
        Some(c) => {
            let fr = if c.looping {
                c.frame()
            } else {
                c.frame().min(c.frames as f32 - 1.001)
            };
            (c.start, c.frames, fr)
        }
        None => (
            anim.seq_start,
            anim.seq_frames,
            anim.start_frame + a.time * anim.rate,
        ),
    }
}

/// Play looping vertex animations: interpolate between keyframes and rewrite the mesh positions.
pub fn animate(
    time: Res<Time>,
    level: Res<crate::LevelRes>,
    mut q: Query<(Entity, &mut Animated, Option<&Children>, &GlobalTransform, Option<&crate::gpuanim::GpuAnimated>)>,
    seen: Query<&ViewVisibility>,
    cam: Query<&GlobalTransform, (With<crate::FlyCam>, Without<crate::SkyCam>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut tags: Query<&mut bevy::mesh::MeshTag>,
    mut map_gen: Local<u64>,
    mut cpu_blended: Local<std::collections::HashSet<Entity>>,
    mut tick: Local<u32>,
) {
    // A new map (loader.rs): start this system's per-map state over.
    if crate::loader::new_map(&mut map_gen) {
        cpu_blended.clear();
    }
    let _prof = crate::prof::scope("actors::animate");
    let dt = time.delta_secs().min(0.1);
    *tick = tick.wrapping_add(1);
    let eye = cam.iter().next().map(|t| t.translation());
    let (mut n_all, mut n_vis, mut n_up, mut n_verts, mut n_gpu) = (0, 0, 0, 0usize, 0);
    for (ent, mut a, kids, gt, on_gpu) in &mut q {
        n_all += 1;
        // Like Unreal, only meshes that were drawn last frame get their vertices posed (the clocks still run);
        // re-posing and re-uploading every pawn on the island every frame was the biggest per-frame cost.
        let visible = kids.is_none_or(|k| {
            k.iter()
                .any(|c| seen.get(c).map_or(true, |v| v.get()))
        });
        if !visible {
            a.time += dt.min(0.1);
            if let Some(c) = a.clip.as_mut() {
                c.t += dt;
            }
            if let Some(tw) = a.tween.as_mut() {
                tw.1 = tw.2;
            }
            a.shown = None;
            continue;
        }
        n_vis += 1;
        // Far away (over ~30 m) a pose every third frame is plenty; the clocks keep running in between.
        let far = eye.is_some_and(|e| e.distance_squared(gt.translation()) > 30.0 * 30.0);
        // (Posing on the GPU costs next to nothing, so those are posed every frame at any distance.)
        if on_gpu.is_none() && far && (tick.wrapping_add(ent.index_u32())) % 3 != 0 && a.shown.is_some() {
            a.time += dt;
            if let Some(c) = a.clip.as_mut() {
                c.t += dt;
            }
            continue;
        }
        a.time += dt;
        let a = &mut *a;
        let ma = &level.0.mesh_actors[a.mesh_actor];
        let Some(anim) = &ma.anim else { continue };
        // A new sequence (or the same one restarted) blends in from the pose on screen over 0.1 s, as PlayAnim's
        // TweenTime does, instead of snapping; its own clock waits for the blend.
        let clip_key = a.clip.as_ref().map(|c| (c.start, c.t));
        if let (Some((start, t)), Some(prev)) = (clip_key, a.shown) {
            let changed = start != prev.0 || t + 1e-4 < a.shown_t;
            if changed && t <= dt + 1e-4 {
                a.tween = Some((prev, 0.0, 0.1));
            }
        }
        let tweening = if let Some(tw) = a.tween.as_mut() {
            tw.1 += dt;
            if tw.1 >= tw.2 {
                a.tween = None;
                false
            } else {
                true
            }
        } else {
            false
        };
        // Either an override clip (played from its own clock) or the idle loop.
        if let Some(c) = a.clip.as_mut() {
            if tweening {
                c.t = 0.0;
            } else {
                c.t += dt;
            }
        }
        let (start, frames, f) = current_frame(a, anim);
        let f = if frames <= 1 { 0.0 } else { f };
        a.shown_t = a.clip.as_ref().map(|c| c.t).unwrap_or(0.0);
        let face = update_face(&mut a.face, &anim.mesh, dt);
        // A held pose (single-frame sequence, finished one-shot) only needs uploading once.
        // (A GPU-posed mesh that was blended on the CPU last frame gets its tag back even if the pose held.)
        if (a.tween.is_none() && a.shown == Some((start, frames, f)) && a.face_shown == face && !cpu_blended.contains(&ent))
            || crate::prof::NO_POSE.load(std::sync::atomic::Ordering::Relaxed)
        {
            continue;
        }
        a.face_shown = face;
        n_up += 1;
        let still = seq_start(&anim.mesh, "Still").map_or(0, |s| s.0);
        let blend = a
            .tween
            .map(|(from, e, len)| (from, (e / len).clamp(0.0, 1.0)));
        a.shown = Some(match blend {
            // Mid-blend the "shown" pose is the target; a further change re-blends from there (close enough).
            Some(_) => (start, frames, f),
            None => (start, frames, f),
        });
        if on_gpu.is_some() {
            // A plain frame pair is posed by the vertex shader from the part's MeshTag; a tween or face blend
            // is posed below on the CPU this frame (tag SENTINEL = use the mesh's positions).
            let (fa, fb, ft) = crate::gpuanim::frame_pair(start, frames, f);
            let tag = match (blend, &face) {
                (None, None) => crate::gpuanim::encode(fa, fb, ft),
                _ => None,
            };
            if let Some(kids) = kids {
                let want = tag.unwrap_or(crate::gpuanim::SENTINEL);
                for c in kids.iter() {
                    if let Ok(mut t) = tags.get_mut(c) {
                        if t.0 != want {
                            t.0 = want;
                        }
                    }
                }
            }
            if tag.is_some() {
                cpu_blended.remove(&ent);
                n_gpu += 1;
                continue;
            }
            cpu_blended.insert(ent);
        }
        for (h, pi) in &a.parts {
            let part = &ma.parts[*pi];
            let Some(mut m) = meshes.get_mut(h) else {
                continue;
            };
            let pos = part
                .vert_ids
                .iter()
                .map(|&v| {
                    let mut p = anim.mesh.sample(start, frames, f, v as usize);
                    if let Some(((s0, n0, f0), k)) = blend {
                        let q = anim.mesh.sample(s0, n0, f0, v as usize);
                        for i in 0..3 {
                            p[i] = q[i] + (p[i] - q[i]) * k;
                        }
                    }
                    if let Some(key) = &face {
                        let o = face_offset(&anim.mesh, key, still, v as usize);
                        for i in 0..3 {
                            p[i] += o[i];
                        }
                    }
                    ue_to_bevy([
                        p[0] * anim.draw_scale,
                        p[1] * anim.draw_scale,
                        p[2] * anim.draw_scale,
                    ])
                });
            n_verts += part.vert_ids.len();
            // Rewrite the positions in place (no new buffer per frame).
            match m.attribute_mut(Mesh::ATTRIBUTE_POSITION) {
                Some(bevy::mesh::VertexAttributeValues::Float32x3(dst)) if dst.len() == part.vert_ids.len() => {
                    for (d, p) in dst.iter_mut().zip(pos) {
                        *d = p;
                    }
                }
                _ => m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos.collect::<Vec<[f32; 3]>>()),
            }
            if let Some(uv) = lid_uvs(&anim.mesh, part, face.as_ref(), still) {
                m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
            }
        }
    }
    if crate::prof::on() {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 5 == 0 {
            println!("PROF animate: {n_all} animated, {n_vis} visible, {n_up} posed ({n_gpu} on the GPU), {n_verts} CPU verts");
        }
    }
}
