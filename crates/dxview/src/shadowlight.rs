//! Enhancement (Real-Time Shadows, ROADMAP v2 #26): people and decorations cast moving shadows from the map's own
//! lights, and so do doors and other movers. Ours, not in the original (which only had the blob shadow under pawns).
//!
//! The world is lit by its lightmaps, so a shadow has to take light away rather than add it, and exactly as much as
//! that lamp put into the lightmap there. The strongest static lights near the player each get a shadow-casting Bevy
//! point light (`ShadowLight`) that adds no light of its own worth seeing (intensity x `ENCODE`): its colour,
//! brightness and radius only carry the Unreal light's values to the BSP's fragment shader (shaders/bsp_shadow.wgsl,
//! `BspMaterial`). There, for each such light whose shadow map says the texel is blocked, the light's own Unreal
//! term (dx-level's bake: colour x (1 - d/r) x N.L x LIGHT_GAIN, summed in gamma space) comes off the lightmap value.
//! So a guard's shadow is as dark as the light that guard really stands in, on any surface, and never darker than
//! black. The BSP itself never casts (the lightmaps already hold its shadows): the lights live on `SHADOW_LAYER`,
//! and only actor and mover meshes join it (`tag_casters_system`). Setting "RealtimeShadows": 0 off, 1 two lights, 2 four.

use crate::actors::{Blockers, MeshActorRef};
use crate::{FlyCam, LevelRes, Player, SkyCam};
use bevy::camera::visibility::RenderLayers;
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use dx_level::ue_to_bevy;
use glam::DVec3;
use std::collections::HashMap;

pub const SHADOW_LAYER: usize = 8;
const SLOTS: usize = 4;
/// The shadow lights' own light is scaled down by this (so they add nothing visible); the shader scales it back.
const ENCODE: f32 = 1.0e-4;
/// How much of the blocked light a shadow takes away (the rest stands for light bounced about).
const STRENGTH: f32 = 0.85;

pub const BSP_SHADER: Handle<Shader> = bevy::asset::uuid_handle!("2c7d9a51-6e3b-4f08-8a1c-5b9e0d4f7a62");

/// The lit BSP: StandardMaterial with the shadow pass over its lightmap.
pub type BspMaterial = ExtendedMaterial<StandardMaterial, BspShadow>;

#[derive(Clone, Copy, Debug, Default, ShaderType, Reflect)]
pub struct BspShadowParams {
    /// x: strength (0 = no shadows), y: 1 / ENCODE, z: dx-level's LIGHT_GAIN.
    pub p: Vec4,
    /// x: Ambient Occlusion's strength on the lightmap (0 = off; the SSAO pass itself always runs, so switching it
    /// never recompiles every material).
    pub q: Vec4,
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct BspShadow {
    #[uniform(100)]
    pub params: BspShadowParams,
}

impl MaterialExtension for BspShadow {
    fn fragment_shader() -> ShaderRef {
        BSP_SHADER.into()
    }
    // In the prepass: with Ambient Occlusion (or Modern Water's depth) the camera has a depth / normal prepass, and
    // without the world in it SSAO had no walls to work from, the water no seabed, and Bevy's forward pass (which
    // loads opaque normals from the normal prepass when MSAA is off) lit the BSP with garbage normals: dark wedges
    // and a dead flashlight.
    fn enable_prepass() -> bool {
        true
    }
}

pub fn bsp_material(base: StandardMaterial) -> BspMaterial {
    BspMaterial {
        base,
        extension: BspShadow {
            params: BspShadowParams {
                p: Vec4::new(STRENGTH, 1.0 / ENCODE, dx_level::lighting::LIGHT_GAIN, std::env::var("DXVIEW_SHADOW_DEBUG").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0)),
                q: Vec4::new(if crate::postfx::ao_on() { 0.85 } else { 0.0 }, 0.0, 0.0, 0.0),
            },
        },
    }
}

pub fn setup_bsp_shader(mut shaders: ResMut<Assets<Shader>>) {
    let _ = shaders.insert(
        &BSP_SHADER,
        Shader::from_wgsl(include_str!("shaders/bsp_shadow.wgsl"), "dxview/bsp_shadow.wgsl"),
    );
}

pub fn level() -> u8 {
    crate::settings::get("RealtimeShadows").round().clamp(0.0, 2.0) as u8
}

fn wanted() -> usize {
    match level() {
        0 => 0,
        1 => 2,
        _ => 4,
    }
}

/// Layers for a casting mesh part, plus `extra` (the vision aug's heat layer).
pub fn caster_layers(extra: &[usize]) -> RenderLayers {
    let mut l = vec![0, SHADOW_LAYER];
    l.extend_from_slice(extra);
    RenderLayers::from_layers(&l)
}

#[derive(Component)]
pub struct ShadowLight {
    slot: usize,
}

impl ShadowLight {
    pub fn slot(&self) -> usize {
        self.slot
    }
}

/// A mesh part already looked at by `tag_casters_system`.
#[derive(Component)]
pub struct ShadowCaster;

#[derive(Clone, Copy, Default)]
struct Slot {
    light: Option<usize>,
    weight: f32,
}

#[derive(Resource, Default)]
pub struct ShadowLights {
    slots: [Slot; SLOTS],
    chosen: Vec<usize>,
    /// Per chosen light: where its spot points (Bevy axes) and the cone's half angle.
    aim: HashMap<usize, (Vec3, f32)>,
    repick: f32,
}

/// The depth bias that marks shadowlight's spots for bsp_shadow.wgsl (other spot lights use their own).
pub const SHADOW_MARK: f32 = 0.0201;

pub fn setup_shadow_lights(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<StandardMaterial>>) {
    commands.insert_resource(ShadowLights::default());
    // DXVIEW_SHADOW_CUBE="x,y,z" (Bevy metres): a plain test cube that casts.
    if let Some(p) = std::env::var("DXVIEW_SHADOW_CUBE").ok().map(|v| v.split(',').filter_map(|x| x.trim().parse::<f32>().ok()).collect::<Vec<_>>()) {
        if p.len() == 3 {
            commands.spawn((
                Mesh3d(meshes.add(Cuboid::new(1.0, 1.0, 1.0))),
                MeshMaterial3d(mats.add(StandardMaterial { base_color: Color::srgb(0.8, 0.2, 0.2), unlit: true, ..default() })),
                Transform::from_xyz(p[0], p[1], p[2]),
                caster_layers(&[]),
                ShadowCaster,
            ));
        }
    }
    // Spot lights aimed at what casts: one shadow view each, where a point light renders six cube faces (each a
    // whole extra pass on the render thread - 24 of them were most of a 30 ms frame).
    for slot in 0..SLOTS {
        commands.spawn((
            SpotLight {
                intensity: 0.0,
                range: 1.0,
                shadow_maps_enabled: true,
                affects_lightmapped_mesh_diffuse: false,
                shadow_depth_bias: SHADOW_MARK,
                outer_angle: 1.0,
                inner_angle: 0.9,
                ..default()
            },
            Transform::default(),
            Visibility::Hidden,
            RenderLayers::layer(SHADOW_LAYER),
            ShadowLight { slot },
        ));
    }
}

/// How much of light `l` reaches `p` (0 out of range or out of sight).
fn reach(level: &dx_level::Level, l: &dx_level::lighting::LightInfo, p: DVec3) -> f32 {
    let lp = DVec3::new(l.pos[0] as f64, l.pos[1] as f64, l.pos[2] as f64);
    let d = lp.distance(p);
    if d >= l.radius as f64 {
        return 0.0;
    }
    let h = level.collision.trace(lp, p, DVec3::ZERO);
    if h.fraction < 0.98 {
        return 0.0;
    }
    let b = l.color[0].max(l.color[1]).max(l.color[2]);
    b * (1.0 - d as f32 / l.radius)
}

#[allow(clippy::too_many_arguments)]
pub fn shadow_light_system(
    time: Res<Time>,
    level: Res<LevelRes>,
    player: Res<Player>,
    blockers: Res<Blockers>,
    mut st: ResMut<ShadowLights>,
    cam: Single<&Transform, (With<FlyCam>, Without<SkyCam>, Without<ShadowLight>)>,
    mut lights: Query<(&ShadowLight, &mut SpotLight, &mut Transform, &mut Visibility)>,
    mut map_gen: Local<u64>,
) {
    let _prof = crate::prof::scope("shadowlight::shadow_light_system");
    let lvl = &level.0;
    if crate::loader::new_map(&mut map_gen) {
        *st = ShadowLights::default();
    }
    let dt = time.delta_secs().min(0.1);
    let n = wanted();
    st.repick -= dt;
    if st.repick <= 0.0 {
        st.repick = 0.5;
        st.chosen.clear();
        if n > 0 {
            let eye = {
                let e = dx_level::bevy_to_ue(cam.translation.to_array());
                DVec3::new(e[0] as f64, e[1] as f64, e[2] as f64)
            };
            let fwd = {
                let f = cam.forward().as_vec3();
                DVec3::new(f.x as f64, f.z as f64, f.y as f64)
            };
            // Where shadows would be seen: the player, the camera, and things in front of it not far off.
            let mut points = vec![player.body.location, eye - DVec3::new(0.0, 0.0, 30.0)];
            let mut near: Vec<(f64, DVec3)> = blockers
                .list
                .iter()
                .filter(|c| c.radius > 4.0 && c.height > 8.0)
                .map(|c| (c.center.distance(eye), c.center))
                .filter(|(d, c)| *d < 1800.0 && (*c - eye).dot(fwd) > 0.0)
                .collect();
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            points.extend(near.into_iter().take(10).map(|(_, c)| c));
            let third = crate::thirdperson::active();
            let mut aims: Vec<(usize, Vec3, f32)> = Vec::new();
            let held: Vec<usize> = st.slots.iter().filter(|s| s.weight > 0.0).filter_map(|s| s.light).collect();
            let mut scored: Vec<(f32, usize)> = lvl
                .lights
                .iter()
                .enumerate()
                .filter(|(_, l)| l.radius > 160.0)
                .filter(|(_, l)| {
                    let lp = DVec3::new(l.pos[0] as f64, l.pos[1] as f64, l.pos[2] as f64);
                    lp.distance(eye) < 3500.0 + l.radius as f64
                })
                .filter_map(|(i, l)| {
                    let mut best = 0.0f32;
                    // A shadow map is six extra views of the scene: only worth it when something that casts
                    // (a pawn or decoration in front, or JC's body in third person) is in the light.
                    let lp = DVec3::new(l.pos[0] as f64, l.pos[1] as f64, l.pos[2] as f64);
                    let mut casters: Vec<DVec3> = Vec::new();
                    for (k, p) in points.iter().enumerate() {
                        let w = if k == 1 { 1.0 } else { 1.0 / (1.0 + p.distance(eye) as f32 / 1200.0) };
                        let r = reach(lvl, l, *p);
                        if r > 0.0 && (k >= 2 || (k == 0 && third)) {
                            casters.push(*p);
                        }
                        best = best.max(r * w);
                    }
                    if casters.is_empty() {
                        return None;
                    }
                    // The spot: toward the casters' middle, wide enough for each (plus its size: ~60 units round).
                    let dirs: Vec<DVec3> = casters.iter().map(|c| (*c - lp).normalize_or_zero()).collect();
                    let mid = dirs.iter().fold(DVec3::ZERO, |a, d| a + *d).normalize_or(DVec3::NEG_Z);
                    let half = casters
                        .iter()
                        .zip(&dirs)
                        .map(|(c, d)| d.dot(mid).clamp(-1.0, 1.0).acos() + (60.0 / c.distance(lp).max(30.0)).atan())
                        .fold(0.0f64, f64::max)
                        .clamp(0.35, 1.35) as f32;
                    aims.push((i, Vec3::new(mid.x as f32, mid.z as f32, mid.y as f32), half));
                    // Hold on to the lights already shown (no flicking between two alike).
                    if held.contains(&i) {
                        best *= 1.3;
                    }
                    (best > 0.05).then_some((best, i))
                })
                .collect();
            scored.sort_by(|a, b| b.0.total_cmp(&a.0));
            if std::env::var_os("DXVIEW_SHADOWLOG").is_some() {
                eprintln!(
                    "SHADOW candidates {:?}",
                    scored.iter().take(6).map(|(s, i)| (*i, *s, lvl.lights[*i].pos, lvl.lights[*i].radius)).collect::<Vec<_>>()
                );
            }
            st.chosen = scored.into_iter().take(n).map(|(_, i)| i).collect();
            for (i, d, h) in aims {
                if st.chosen.contains(&i) {
                    st.aim.insert(i, (d, h));
                }
            }
        }
    }
    // Slots: a chosen light keeps its slot; new ones take a free slot; the rest fade out.
    let chosen = st.chosen.clone();
    for s in st.slots.iter_mut() {
        if s.light.is_some_and(|l| chosen.contains(&l)) {
            s.weight = (s.weight + dt * 2.0).min(1.0);
        } else {
            s.weight = (s.weight - dt * 2.0).max(0.0);
            if s.weight == 0.0 {
                s.light = None;
            }
        }
    }
    for l in chosen {
        if st.slots.iter().any(|s| s.light == Some(l)) {
            continue;
        }
        if let Some(s) = st.slots.iter_mut().find(|s| s.light.is_none()) {
            s.light = Some(l);
            s.weight = 0.0;
        }
    }
    let gain = std::env::var("DXVIEW_SHADOW_GAIN").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0f32);
    for (sl, mut pl, mut tf, mut vis) in &mut lights {
        let s = st.slots[sl.slot];
        let aim = s.light.and_then(|i| st.aim.get(&i).copied());
        let Some(info) = s.light.and_then(|i| lvl.lights.get(i)).filter(|_| s.weight > 0.0 && n > 0) else {
            pl.intensity = 0.0;
            *vis = Visibility::Hidden;
            continue;
        };
        // The shader reads colour x intensity / (4 pi) back as the Unreal light's colour (brightness included).
        let b = info.color[0].max(info.color[1]).max(info.color[2]).max(1e-4);
        pl.color = Color::linear_rgb(info.color[0] / b, info.color[1] / b, info.color[2] / b);
        pl.intensity = 4.0 * std::f32::consts::PI * b * ENCODE * gain * s.weight;
        pl.range = info.radius * dx_level::METERS_PER_UNIT;
        tf.translation = Vec3::from(ue_to_bevy(info.pos));
        if let Some((d, half)) = aim {
            // Ease toward the new aim (casters move between repicks).
            let want = d.normalize_or(Vec3::NEG_Y);
            let cur = tf.forward().as_vec3();
            let k = (dt * 4.0).min(1.0);
            let dir = cur.lerp(want, k).normalize_or(want);
            let up = if dir.y.abs() > 0.95 { Vec3::X } else { Vec3::Y };
            tf.look_to(dir, up);
            pl.outer_angle += (half - pl.outer_angle) * k;
            pl.inner_angle = pl.outer_angle * 0.9;
        }
        *vis = Visibility::Inherited;
    }
}

/// Puts actor and mover (door) mesh parts (opaque or masked ones) on the shadow layer so the shadow lights see them.
#[allow(clippy::type_complexity)]
pub fn tag_casters_system(
    mut commands: Commands,
    movers: Option<Res<crate::movers::Movers>>,
    roots: Query<(), With<MeshActorRef>>,
    parts: Query<
        (Entity, &ChildOf, Option<&MeshMaterial3d<StandardMaterial>>, Option<&bevy::camera::primitives::Aabb>, &GlobalTransform),
        (With<Mesh3d>, Without<RenderLayers>, Without<ShadowCaster>),
    >,
    lvl_res: Res<LevelRes>,
    anim_parts: Query<(), With<MeshMaterial3d<crate::gpuanim::AnimMaterial>>>,
    anim_mats: Res<Assets<crate::gpuanim::AnimMaterial>>,
    anim_handles: Query<&MeshMaterial3d<crate::gpuanim::AnimMaterial>>,
    mats: Res<Assets<StandardMaterial>>,
) {
    // The camera's layers (with SHADOW_LAYER while this is on) are set by thirdperson::toggle_system.
    let on = level() > 0;
    if !on {
        return;
    }
    let mover_roots: std::collections::HashSet<Entity> =
        movers.as_ref().map(|m| m.entities.iter().copied().collect()).unwrap_or_default();
    for (e, parent, mat, aabb, gt) in &parts {
        let p = parent.parent();
        let mover = mover_roots.contains(&p);
        if !roots.contains(p) && !mover {
            continue;
        }
        // Bounds come a frame after the mesh; wait for them.
        let Some(aabb) = aabb else { continue };
        // Big things (ships, buildings, the statue) would throw a lamp's light off whole areas the map lit by
        // hand; shadows are for people, crates, furniture and doors.
        let size = (Vec3::from(aabb.half_extents) * 2.0 * gt.compute_transform().scale).max_element();
        let too_big = size > if mover { 6.0 } else { 3.5 };
        // Lamps: a fixture holding one of the map's lights would shut its own light in (the lightmaps never let
        // meshes cast), so a part whose box holds a light doesn't cast.
        let holds_light = !mover && {
            let inv = gt.affine().inverse();
            let pad = Vec3::splat(24.0 * dx_level::METERS_PER_UNIT);
            let (lo, hi) = (Vec3::from(aabb.min()) - pad, Vec3::from(aabb.max()) + pad);
            lvl_res.0.lights.iter().any(|l| {
                let q = inv.transform_point3(Vec3::from(ue_to_bevy(l.pos)));
                q.cmpge(lo).all() && q.cmple(hi).all()
            })
        };
        let alpha = if let Some(m) = mat.and_then(|m| mats.get(&m.0)) {
            Some(m.alpha_mode)
        } else if anim_parts.contains(e) {
            anim_handles.get(e).ok().and_then(|h| anim_mats.get(&h.0)).map(|m| m.base.alpha_mode)
        } else {
            None
        };
        let casts = matches!(alpha, Some(AlphaMode::Opaque) | Some(AlphaMode::Mask(_))) && !holds_light && !too_big;
        if std::env::var_os("DXVIEW_SHADOWLOG").is_some() {
            eprintln!("SHADOW tag {e:?} alpha {alpha:?} size {size:.1} at {:?} casts {casts} anim {} aabb {:?}", gt.translation(), anim_parts.contains(e), aabb);
        }
        let mut ec = commands.entity(e);
        ec.insert(ShadowCaster);
        if casts {
            ec.insert(caster_layers(&[]));
        }
    }
}


/// Transparent meshes cast no shadows. Bevy's shadow pass draws every caster whatever its alpha mode, so decals
/// (bullet holes, blood splats, blob shadows lying on the floor), effect sprites and the like dropped dark squares
/// into the flashlight cone and the shadow lights. Anything new with a blended / additive / modulated material is
/// marked NotShadowCaster.
#[allow(clippy::type_complexity)]
pub fn no_transparent_shadows_system(
    mut commands: Commands,
    q: Query<
        (Entity, &MeshMaterial3d<StandardMaterial>),
        (Without<bevy::light::NotShadowCaster>, Or<(Added<MeshMaterial3d<StandardMaterial>>, Changed<MeshMaterial3d<StandardMaterial>>)>),
    >,
    mats: Res<Assets<StandardMaterial>>,
) {
    for (e, h) in &q {
        let Some(m) = mats.get(&h.0) else { continue };
        if matches!(m.alpha_mode, AlphaMode::Blend | AlphaMode::Add | AlphaMode::Multiply | AlphaMode::Premultiplied | AlphaMode::AlphaToCoverage) {
            commands.entity(e).insert(bevy::light::NotShadowCaster);
        }
    }
}
