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
/// Each shadow light's own layer: a caster is on exactly one of them (one shadow per model, from the light that
/// lights it most), so a guard between three lamps doesn't trail three shadows.
pub const SLOT_LAYERS: [usize; SLOTS] = [SHADOW_LAYER, 20, 21, 22];
/// The shadow lights' own light is scaled down by this (so they add nothing visible); the shader scales it back.
const ENCODE: f32 = 1.0e-4;
/// Rank weights: the strongest chosen light's shadows at full strength, the others a little lighter.
const RANK_WEIGHT: [f32; SLOTS] = [1.0, 0.8, 0.65, 0.5];
/// A light already shown keeps its place unless another beats it by this much (no flicking between two alike).
const HOLD: f32 = 0.85;
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
    /// never recompiles every material). w: 1 = the dynamic lights as Unreal 1 drew them (`classic_lights`).
    pub q: Vec4,
    /// Remastered lighting (relight.rs): x 1 = on, y highlight strength, z roughness, w metalness.
    pub r: Vec4,
    /// x light gain, y added light (gamma), z emissive strength (named glowing textures), w glow of unlit surfaces.
    pub s: Vec4,
    /// xy: texture panning speed (PF_AutoUPan / VPan, UV per second), done in the shader on the GPU's clock so the
    /// material never changes after it's made.
    pub u: Vec4,
}

/// Bindless (like StandardMaterial and gpuanim's VertAnim): every BSP material lives in a few shared bind groups, so
/// Bevy draws the world's surfaces as a handful of GPU multi-draw calls instead of one draw call per texture - the
/// render thread records a fraction of the commands. Without binding arrays (old GPUs) it falls back to one bind
/// group per material, as before.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
#[bind_group_data(BspShadowKey)]
#[data(100, BspShadowParams, binding_array(111))]
#[bindless(index_table(range(100..106), binding(110)))]
pub struct BspShadow {
    pub params: BspShadowParams,
    /// Remastered lighting: how much of each surface's three strongest lights reaches each lightmap texel
    /// (dx-level lighting::Atlas::mask), and the lights themselves (2 texels each: position + radius, colour).
    #[texture(101)]
    #[sampler(102)]
    pub mask: Option<Handle<Image>>,
    #[texture(103, sample_type = "float", filterable = false)]
    pub lights: Option<Handle<Image>>,
    /// The surface's DetailTexture (params.q.y = its UV scale, q.z = Detail Textures on).
    #[texture(104)]
    #[sampler(105)]
    pub detail: Option<Handle<Image>>,
    /// The sky room's faces, drawn far past the map (skyfollow.rs): no dynamic light reaches them.
    pub sky: bool,
    /// A dynamic light here can be a spot light: only the flashlight with Flashlight Cone on (dynlight.rs).
    pub spot_lights: bool,
    /// The dynamic lights are in the lightmap (lmlights.rs): no walk for them at all.
    pub lm_lights: bool,
}

impl<'a> From<&'a BspShadow> for BspShadowParams {
    fn from(m: &'a BspShadow) -> Self {
        m.params
    }
}

/// What a BSP material's fragment shader compiles in: its spot shadows (`p.x` > 0) and its remastered lighting
/// (`r.x` on) only when it uses them, and its detail texture only when it has one with Detail Textures on (`q.y` > 0,
/// `q.z` on). The branches they replace were already skipped at run time, so the picture is the same, but the shader
/// carried their code and the registers it needs. `classic_lights` (`q.w`) replaces Bevy's lighting of the dynamic
/// lights.
///
/// The Classic lights' walk over each pixel's cluster (bsp_shadow.wgsl) only where a dynamic light can reach: not on
/// the sky room's faces, and its spot-light cone only with Flashlight Cone on. With the lightmap lights (lmlights.rs)
/// the dynamic lights come with the lightmap and no material walks.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BspShadowKey {
    spot_shadows: bool,
    relight: bool,
    classic_lights: bool,
    detail: bool,
    walk: bool,
    spot_lights: bool,
}

impl From<&BspShadow> for BspShadowKey {
    fn from(m: &BspShadow) -> Self {
        BspShadowKey {
            spot_shadows: m.params.p.x > 0.0,
            relight: m.params.r.x > 0.5,
            classic_lights: m.params.q.w > 0.5,
            detail: m.params.q.y > 0.0 && m.params.q.z > 0.5,
            walk: !m.sky && !m.lm_lights,
            spot_lights: m.spot_lights,
        }
    }
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
    fn specialize(
        _pipeline: &bevy::pbr::MaterialExtensionPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        key: bevy::pbr::MaterialExtensionKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        if let Some(fragment) = descriptor.fragment.as_mut() {
            if key.bind_group_data.spot_shadows {
                fragment.shader_defs.push("BSP_SPOT_SHADOWS".into());
            }
            if key.bind_group_data.relight {
                fragment.shader_defs.push("BSP_RELIGHT".into());
            }
            if key.bind_group_data.classic_lights {
                fragment.shader_defs.push("BSP_CLASSIC_LIGHTS".into());
            }
            if key.bind_group_data.detail {
                fragment.shader_defs.push("BSP_DETAIL".into());
            }
            if !key.bind_group_data.walk {
                fragment.shader_defs.push("BSP_NO_DYNLIGHTS".into());
            }
            if key.bind_group_data.spot_lights {
                fragment.shader_defs.push("BSP_SPOT_LIGHTS".into());
            }
        }
        Ok(())
    }
}

pub fn bsp_material(base: StandardMaterial) -> BspMaterial {
    BspMaterial {
        base,
        extension: BspShadow {
            params: BspShadowParams {
                p: Vec4::new(shadow_strength(), 1.0 / ENCODE, dx_level::lighting::LIGHT_GAIN, env_val!("DXVIEW_SHADOW_DEBUG").map(str::to_owned).and_then(|v| v.parse().ok()).unwrap_or(0.0)),
                q: Vec4::new(if crate::postfx::ao_on() { 0.85 } else { 0.0 }, 0.0, 0.0, if classic_lights() { 1.0 } else { 0.0 }),
                r: Vec4::ZERO,
                s: Vec4::new(1.0, 0.0, 0.0, 0.0),
                u: Vec4::ZERO,
            },
            mask: None,
            lights: None,
            detail: None,
            sky: false,
            spot_lights: crate::dynlight::flashlight_cone(),
            lm_lights: crate::lmlights::active(),
        },
    }
}

/// Lean Rendering lights the level's surfaces the way Unreal 1 did (bsp_shadow.wgsl, `BSP_CLASSIC_LIGHTS`): each
/// dynamic light (dynlight.rs) added to the lightmap as the bake added the static ones, diffuse only, instead of
/// Bevy's PBR lighting, whose light loop sets the BSP shader's register count.
pub fn classic_lights() -> bool {
    crate::perf::lean_rendering()
}

pub const SPOTS_SHADER: Handle<Shader> = bevy::asset::uuid_handle!("2c7d9a51-6e3b-4f08-8a1c-5b9e0d4f7a63");

pub fn setup_bsp_shader(mut shaders: ResMut<Assets<Shader>>) {
    let _ = shaders.insert(
        &SPOTS_SHADER,
        Shader::from_wgsl(include_str!("shaders/shadow_spots.wgsl"), "dxview/shadow_spots.wgsl"),
    );
    let _ = shaders.insert(
        &BSP_SHADER,
        Shader::from_wgsl(include_str!("shaders/bsp_shadow.wgsl"), "dxview/bsp_shadow.wgsl"),
    );
}

pub fn level() -> u8 {
    setting!("RealtimeShadows").round().clamp(0.0, 2.0) as u8
}

/// The BSP shader's shadow strength. With Lean Rendering and no shadow lights it's 0, which skips the shader's walk
/// over the lights in each fragment's cluster: with nothing casting, that walk takes nothing off the lightmap.
pub fn shadow_strength() -> f32 {
    if crate::perf::lean_rendering() && level() == 0 {
        0.0
    } else {
        STRENGTH
    }
}

/// Whether the mesh actors' fragment shaders (anim_frag.wgsl, actor_lit.wgsl) walk each fragment's cluster for shadow
/// lights: with the BSP's (`shadow_strength` above 0). With Lean Rendering and Realtime Shadows off no shadow light is
/// ever shown, so the walk found nothing in any fragment, and leaving it out (`ActorShadowKey`) keeps the picture.
pub fn actor_spot_shadows() -> bool {
    shadow_strength() > 0.0
}

/// A mesh actor material's pipeline key (gpuanim.rs `VertAnim`, skeletal.rs `ActorLit`): the shadow lights' walk
/// (`ACTOR_SPOT_SHADOWS`) compiled in or not. postfx.rs sets it again when Realtime Shadows or Lean Rendering change.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ActorShadowKey {
    pub spot_shadows: bool,
}

impl ActorShadowKey {
    pub fn specialize(self, descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor) {
        if let (true, Some(fragment)) = (self.spot_shadows, descriptor.fragment.as_mut()) {
            fragment.shader_defs.push("ACTOR_SPOT_SHADOWS".into());
        }
    }
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
    let mut l = vec![0];
    l.extend_from_slice(extra);
    RenderLayers::from_layers(&l)
}

/// A mesh part that casts (opaque or masked, not too big, not a lamp holding its light): `assign_system` puts it on
/// the layer of the one shadow light it casts from.
#[derive(Component)]
pub struct Casts;

/// A caster bigger than a person (a tree): its radius in Unreal units, for aiming its light's spot.
#[derive(Component)]
pub struct CastSize(pub f32);

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
    target: f32,
}

#[derive(Resource, Default)]
pub struct ShadowLights {
    slots: [Slot; SLOTS],
    chosen: Vec<usize>,
    /// Per chosen light: where its spot points (Bevy axes) and the cone's half angle.
    aim: HashMap<usize, (Vec3, f32)>,
    /// Caster root -> the light it casts from.
    assign: HashMap<Entity, usize>,
    repick: f32,
    /// Bumped whenever `assign` or a slot's light changes (assign_system re-layers every part only then).
    gen: u64,
}

/// The depth bias that marks shadowlight's spots for bsp_shadow.wgsl (other spot lights use their own).
pub const SHADOW_MARK: f32 = 0.0201;

pub fn setup_shadow_lights(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<StandardMaterial>>) {
    commands.insert_resource(ShadowLights::default());
    // DXVIEW_SHADOW_CUBE="x,y,z" (Bevy metres): a plain test cube that casts.
    if let Some(p) = env_val!("DXVIEW_SHADOW_CUBE").map(str::to_owned).map(|v| v.split(',').filter_map(|x| x.trim().parse::<f32>().ok()).collect::<Vec<_>>()) {
        if p.len() == 3 {
            let root = commands.spawn((Transform::from_xyz(p[0], p[1], p[2]), Visibility::default())).id();
            commands.spawn((
                Mesh3d(meshes.add(Cuboid::new(1.0, 1.0, 1.0))),
                MeshMaterial3d(mats.add(StandardMaterial { base_color: Color::srgb(0.8, 0.2, 0.2), unlit: true, ..default() })),
                Transform::default(),
                caster_layers(&[]),
                ShadowCaster,
                Casts,
                ChildOf(root),
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
    b * (1.0 - d as f32 / l.radius) * l.spot([p.x as f32, p.y as f32, p.z as f32])
}

/// Distance with height counted twice: lights and casters on your own floor matter most.
fn floor_dist(a: DVec3, b: DVec3) -> f64 {
    let d = a - b;
    (d.x * d.x + d.y * d.y + 4.0 * d.z * d.z).sqrt()
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn shadow_light_system(
    time: Res<Time>,
    level: Res<LevelRes>,
    player: Res<Player>,
    blockers: Res<Blockers>,
    mut st: ResMut<ShadowLights>,
    cam: Single<&Transform, (With<FlyCam>, Without<SkyCam>, Without<ShadowLight>)>,
    mut lights: Query<(&ShadowLight, &mut SpotLight, &mut Transform, &mut Visibility, &mut RenderLayers), Without<Casts>>,
    parts: Query<&ChildOf, With<Casts>>,
    gts: Query<&GlobalTransform>,
    sizes: Query<&CastSize>,
    mut map_gen: Local<u64>,
    mut was_dark: Local<bool>,
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
            let me = player.body.location;
            // Casters all round the player (not just in front: a shadow can fall into view from behind), nearest
            // first, height counted double.
            let mut roots: Vec<(f64, Entity, DVec3)> = Vec::new();
            let mut size_of: HashMap<Entity, f32> = HashMap::new();
            for c in &parts {
                let r = c.parent();
                if roots.iter().any(|(_, e, _)| *e == r) {
                    continue;
                }
                let Ok(gt) = gts.get(r) else { continue };
                let u = dx_level::bevy_to_ue(gt.translation().to_array());
                let size = sizes.get(r).map_or(60.0, |s| s.0);
                // About chest height of the actor's origin (pawns stand on theirs at the cylinder's middle); a
                // tree's crown.
                let p = DVec3::new(u[0] as f64, u[1] as f64, u[2] as f64 + if size > 60.0 { size as f64 } else { 20.0 });
                let d = floor_dist(p, me).min(floor_dist(p, eye));
                if d < 2400.0 {
                    roots.push((d, r, p));
                    size_of.insert(r, size);
                }
            }
            roots.sort_by(|a, b| a.0.total_cmp(&b.0));
            roots.truncate(48);
            if crate::thirdperson::active() {
                roots.push((0.0, Entity::PLACEHOLDER, me));
            }
            let held: Vec<usize> = st.slots.iter().filter(|s| s.weight > 0.0).filter_map(|s| s.light).collect();
            // Lamps inside something (a lamp's own shade, a barrel fire) would shut their own light in.
            let inside = |lp: DVec3| {
                blockers.list.iter().any(|c| {
                    let d = DVec3::new(lp.x - c.center.x, lp.y - c.center.y, 0.0).length();
                    d < c.radius && (lp.z - c.center.z).abs() < c.height
                })
            };
            let mut scored: Vec<(f32, usize)> = lvl
                .lights
                .iter()
                .enumerate()
                .filter(|(_, l)| l.radius > 160.0)
                .filter_map(|(i, l)| {
                    let lp = DVec3::new(l.pos[0] as f64, l.pos[1] as f64, l.pos[2] as f64);
                    if lp.distance(me).min(lp.distance(eye)) > 3500.0 + l.radius as f64 || inside(lp) {
                        return None;
                    }
                    let mut score = 0.0f32;
                    for (k, (d, _, p)) in roots.iter().enumerate().take(16) {
                        let r = reach(lvl, l, *p);
                        if r > 0.0 {
                            score += r / (1.0 + *d as f32 / 600.0) * if k == 0 { 1.2 } else { 1.0 };
                        }
                    }
                    if held.contains(&i) {
                        score /= HOLD;
                    }
                    (score > 0.05).then_some((score, i))
                })
                .collect();
            scored.sort_by(|a, b| b.0.total_cmp(&a.0));
            if env_on!("DXVIEW_SHADOWLOG") {
                eprintln!(
                    "SHADOW candidates {:?} casters {}",
                    scored.iter().take(6).map(|(s, i)| (*i, *s, lvl.lights[*i].pos, lvl.lights[*i].radius)).collect::<Vec<_>>(),
                    roots.len()
                );
            }
            st.chosen = scored.into_iter().take(n).map(|(_, i)| i).collect();
            // One shadow per model: each caster casts from the chosen light that lights it most (keeping the one it
            // had unless another beats it clearly).
            let old = std::mem::take(&mut st.assign);
            st.gen += 1;
            let mut per: HashMap<usize, Vec<(DVec3, f32)>> = HashMap::new();
            for (_, r, p) in &roots {
                let mut best: Option<(f32, usize)> = None;
                for &i in &st.chosen {
                    let mut v = reach(lvl, &lvl.lights[i], *p);
                    if old.get(r) == Some(&i) {
                        v /= HOLD;
                    }
                    if v > 0.0 && best.is_none_or(|b| v > b.0) {
                        best = Some((v, i));
                    }
                }
                if let Some((_, i)) = best {
                    st.assign.insert(*r, i);
                    per.entry(i).or_default().push((*p, size_of.get(r).copied().unwrap_or(60.0)));
                }
            }
            // Each spot: toward its casters' middle, wide enough for each (plus its size: ~60 units round).
            st.aim.clear();
            for (&i, casters) in &per {
                let l = &lvl.lights[i];
                let lp = DVec3::new(l.pos[0] as f64, l.pos[1] as f64, l.pos[2] as f64);
                let dirs: Vec<DVec3> = casters.iter().map(|(c, _)| (*c - lp).normalize_or_zero()).collect();
                let mid = dirs.iter().fold(DVec3::ZERO, |a, d| a + *d).normalize_or(DVec3::NEG_Z);
                let half = casters
                    .iter()
                    .zip(&dirs)
                    .map(|((c, size), d)| d.dot(mid).clamp(-1.0, 1.0).acos() + (*size as f64 / c.distance(lp).max(30.0)).atan())
                    .fold(0.0f64, f64::max)
                    .clamp(0.35, 1.1) as f32;
                st.aim.insert(i, (Vec3::new(mid.x as f32, mid.z as f32, mid.y as f32), half));
            }
            if env_on!("DXVIEW_SHADOWLOG") {
                eprintln!("SHADOW chosen {:?} assigned {:?}", st.chosen, per.iter().map(|(i, v)| (*i, v.len())).collect::<Vec<_>>());
            }
            // A chosen light nobody casts from isn't worth a shadow view.
            let aimed: Vec<usize> = st.chosen.iter().copied().filter(|i| st.aim.contains_key(i)).collect();
            st.chosen = aimed;
        }
    }
    // Slots: a chosen light keeps its slot; new ones take a free slot; each eases to its rank's weight, the rest fade
    // out (no popping).
    let chosen = st.chosen.clone();
    let mut bumps = 0;
    for s in st.slots.iter_mut() {
        s.target = match s.light.and_then(|l| chosen.iter().position(|c| *c == l)) {
            Some(rank) => RANK_WEIGHT[rank.min(SLOTS - 1)],
            None => 0.0,
        };
        let k = dt * 1.5;
        s.weight = if s.weight < s.target { (s.weight + k).min(s.target) } else { (s.weight - k).max(s.target) };
        if s.target == 0.0 && s.weight == 0.0 && s.light.is_some() {
            s.light = None;
            bumps += 1;
        }
    }
    for (rank, l) in chosen.iter().enumerate() {
        if st.slots.iter().any(|s| s.light == Some(*l)) {
            continue;
        }
        if let Some(s) = st.slots.iter_mut().find(|s| s.light.is_none()) {
            s.light = Some(*l);
            s.weight = 0.0;
            s.target = RANK_WEIGHT[rank.min(SLOTS - 1)];
            bumps += 1;
        }
    }
    st.gen += bumps;
    // Shadows off and every slot already dark and hidden: nothing to write.
    if n == 0 && st.slots.iter().all(|s| s.light.is_none()) && *was_dark {
        return;
    }
    *was_dark = n == 0 && st.slots.iter().all(|s| s.light.is_none());
    let third = crate::thirdperson::active();
    let gain = env_val!("DXVIEW_SHADOW_GAIN").map(str::to_owned).and_then(|v| v.parse().ok()).unwrap_or(1.0f32);
    for (sl, mut pl, mut tf, mut vis, mut layers) in &mut lights {
        let s = st.slots[sl.slot];
        // JC's own body in third person casts from the light that lights him most.
        let jc = third && s.light.is_some() && st.assign.get(&Entity::PLACEHOLDER) == s.light.as_ref();
        let want = if jc { RenderLayers::from_layers(&[SLOT_LAYERS[sl.slot], crate::mirror::BODY_LAYER]) } else { RenderLayers::layer(SLOT_LAYERS[sl.slot]) };
        if *layers != want {
            *layers = want;
        }
        let aim = s.light.and_then(|i| st.aim.get(&i).copied());
        let Some(info) = s.light.and_then(|i| lvl.lights.get(i)).filter(|_| s.weight > 0.0 && n > 0) else {
            if pl.intensity != 0.0 {
                pl.intensity = 0.0;
            }
            vis.set_if_neq(Visibility::Hidden);
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
        vis.set_if_neq(Visibility::Inherited);
    }
}

/// Puts each casting part on its light's layer (or none: then it casts nothing). The heat layer (vision aug) kept.
/// Every part is re-layered only when the assignments changed; otherwise just the parts that began casting.
pub fn assign_system(
    st: Res<ShadowLights>,
    mut parts: Query<(&ChildOf, &mut RenderLayers), With<Casts>>,
    fresh: Query<Entity, Added<Casts>>,
    mut seen: Local<(u64, bool)>,
) {
    let on = level() > 0;
    let all = seen.0 != st.gen || seen.1 != on;
    *seen = (st.gen, on);
    if !all && fresh.is_empty() {
        return;
    }
    let slot_of = |l: usize| st.slots.iter().position(|s| s.light == Some(l));
    let relayer = |c: &ChildOf, layers: &mut Mut<RenderLayers>| {
        let slot = if on { st.assign.get(&c.parent()).and_then(|&l| slot_of(l)) } else { None };
        let heat = layers.intersects(&RenderLayers::layer(crate::augdisplay::HEAT_LAYER));
        let world = layers.intersects(&RenderLayers::layer(crate::mirror::WORLD_LAYER));
        let mut want = [0usize; 4];
        let mut n = 1;
        if world {
            want[n] = crate::mirror::WORLD_LAYER;
            n += 1;
        }
        if let Some(k) = slot {
            want[n] = SLOT_LAYERS[k];
            n += 1;
        }
        if heat {
            want[n] = crate::augdisplay::HEAT_LAYER;
            n += 1;
        }
        let want = RenderLayers::from_layers(&want[..n]);
        if **layers != want {
            **layers = want;
        }
    };
    if all {
        for (c, mut layers) in &mut parts {
            relayer(c, &mut layers);
        }
    } else {
        for e in &fresh {
            if let Ok((c, mut layers)) = parts.get_mut(e) {
                relayer(c, &mut layers);
            }
        }
    }
}

/// Puts actor and mover (door) mesh parts (opaque or masked ones) on the shadow layer so the shadow lights see them.
#[allow(clippy::type_complexity)]
pub fn tag_casters_system(
    mut commands: Commands,
    movers: Option<Res<crate::movers::Movers>>,
    roots: Query<(), With<MeshActorRef>>,
    parts: Query<
        (Entity, &ChildOf, Option<&MeshMaterial3d<StandardMaterial>>, Option<&bevy::camera::primitives::Aabb>, &GlobalTransform, Option<&RenderLayers>),
        (With<Mesh3d>, Without<ShadowCaster>),
    >,
    lvl_res: Res<LevelRes>,
    anim_parts: Query<(), With<MeshMaterial3d<crate::gpuanim::AnimMaterial>>>,
    anim_mats: Res<Assets<crate::gpuanim::AnimMaterial>>,
    anim_handles: Query<&MeshMaterial3d<crate::gpuanim::AnimMaterial>>,
    mats: Res<Assets<StandardMaterial>>,
    mut mover_cache: Local<(u64, std::collections::HashSet<Entity>)>,
) {
    // The camera's layers (with SHADOW_LAYER while this is on) are set by thirdperson::toggle_system.
    let on = level() > 0;
    if !on {
        return;
    }
    if parts.is_empty() {
        return;
    }
    // The mover roots, gathered once per map.
    let gen = crate::loader::generation();
    if mover_cache.0 != gen || mover_cache.1.len() != movers.as_ref().map_or(0, |m| m.entities.len()) {
        mover_cache.0 = gen;
        mover_cache.1 = movers.as_ref().map(|m| m.entities.iter().copied().collect()).unwrap_or_default();
    }
    let mover_roots = &mover_cache.1;
    let world_pair = RenderLayers::from_layers(&[0, crate::mirror::WORLD_LAYER]);
    for (e, parent, mat, aabb, gt, layers) in &parts {
        // Parts with layers of their own are left alone, except the level's (movers: 0 + WORLD_LAYER); parts of
        // things that aren't actors or movers never cast. Both are marked checked so they aren't visited again.
        let p = parent.parent();
        let mover = mover_roots.contains(&p);
        if layers.is_some_and(|l| *l != world_pair) || (!roots.contains(p) && !mover) {
            commands.entity(e).insert(ShadowCaster);
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
        if env_on!("DXVIEW_SHADOWLOG") {
            eprintln!("SHADOW tag {e:?} alpha {alpha:?} size {size:.1} at {:?} casts {casts} anim {} aabb {:?}", gt.translation(), anim_parts.contains(e), aabb);
        }
        let mut ec = commands.entity(e);
        ec.insert(ShadowCaster);
        if casts {
            ec.insert((if layers.is_some() { world_pair.clone() } else { caster_layers(&[]) }, Casts));
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
