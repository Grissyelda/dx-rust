//! Enhancement "Grass" (setting Grass: Off / Low / High; Modern preset High): 3D grass blades over the map's grass
//! surfaces, grown on the GPU (imsim-plants, shared with the second game). Ours, after SecondRevision's Grass3D
//! (zlib; its approach, not its code: which textures grow grass, tiles near the eye, a hash per world cell, Bayer-like
//! thinning with distance, travelling wind gusts, blades pushed aside by what stands on them).
//!
//! At level load the faces drawn with one of the grass textures (`GRASS`) that face up are cut into tiles
//! (imsim_plants::grass::GrassField: plane, coverage mask, light-map coordinates). Each frame the nearest tiles are
//! packed into a data image; one mesh of blade slots draws them (shader imsim_plants::GRASS_WGSL), lit by the
//! ground's own light map at the root, so the grass is as bright as the lawn it stands on.

use crate::{FlyCam, LevelRes, Player, SkyCam};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use glam::Vec2;
use imsim_plants::grass::{GrassField, GroundTri, TEXELS};

/// The ground textures that grow grass (by name, the last part of the path) and their kind (the shader's colour).
pub const GRASS: &[(&str, u8)] = &[("ClenGrenGrass_A", 0), ("LiteGrenGrass_A", 1), ("DarkGrenGrass_A", 2), ("Ngrass", 3)];

const DATA_W: u32 = 1024;

pub fn level() -> u8 {
    crate::settings::get("Grass").round().clamp(0.0, 2.0) as u8
}

/// (tiles packed at most, blade slots per tile, full-density reach m, far edge m) by level.
fn budget(l: u8) -> (usize, u32, f32, f32) {
    match l {
        1 => (3000, 10, 6.0, 14.0),
        _ => (8192, 22, 9.0, 22.0),
    }
}

#[derive(Clone, Copy, Debug, Default, ShaderType, Reflect)]
pub struct GrassUniform {
    pub a: Vec4,
    pub wind: Vec4,
    pub eye: Vec4,
    pub b: Vec4,
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct GrassMaterial {
    #[texture(0, sample_type = "float", filterable = false)]
    pub data: Handle<Image>,
    #[texture(1)]
    #[sampler(2)]
    pub lightmap: Handle<Image>,
    #[uniform(3)]
    pub params: GrassUniform,
    #[uniform(4)]
    pub pushes: [Vec4; 16],
}

pub const GRASS_SHADER: Handle<Shader> = bevy::asset::uuid_handle!("1b7e4c90-2d6f-4a3b-8e51-7c0f9a2d6e14");

impl Material for GrassMaterial {
    fn vertex_shader() -> ShaderRef {
        GRASS_SHADER.into()
    }
    fn fragment_shader() -> ShaderRef {
        GRASS_SHADER.into()
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        // Blades are seen from both sides.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

pub struct GrassPlugin;

impl Plugin for GrassPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<GrassMaterial>::default())
            .add_systems(Startup, |mut shaders: ResMut<Assets<Shader>>| {
                let _ = shaders.insert(&GRASS_SHADER, Shader::from_wgsl(imsim_plants::GRASS_WGSL, "imsim_plants/grass.wgsl"));
            })
            .add_systems(Update, grass_system);
    }
}

/// The map's grass, built at level load (`build`).
#[derive(Resource, Default)]
pub struct Grass {
    field: GrassField,
    lightmap: Option<Handle<Image>>,
    /// (mesh entity, material, data image, level it was built for).
    drawn: Option<(Entity, Handle<GrassMaterial>, Handle<Image>, u8)>,
    last_eye: Vec3,
    last_t: f32,
    near: Vec<u32>,
    texels: Vec<[f32; 4]>,
}

/// Cuts the map's grass faces into tiles (called from the level setup; quick: the Island's lawns are ~50k tiles).
pub fn build(commands: &mut Commands, level: &dx_level::Level, lightmap: Option<Handle<Image>>) {
    let mut tris = Vec::new();
    for b in &level.batches {
        let Some(t) = b.texture else { continue };
        if b.sky || b.mirror.is_some() || b.uvs1.len() != b.positions.len() {
            continue;
        }
        let name = level.textures[t].name.rsplit('.').next().unwrap_or("");
        let Some(&(_, kind)) = GRASS.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) else { continue };
        for tri in b.indices.chunks_exact(3) {
            let i = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
            tris.push(GroundTri {
                p: i.map(|k| Vec3::from(b.positions[k])),
                lm: i.map(|k| Vec2::from(b.uvs1[k])),
                kind,
            });
        }
    }
    let t0 = std::time::Instant::now();
    let field = GrassField::build(&tris, 0.5);
    if !field.is_empty() {
        println!("grass: {} tiles from {} faces in {:.0} ms", field.tiles.len(), tris.len(), t0.elapsed().as_secs_f32() * 1000.0);
    }
    commands.insert_resource(Grass { field, lightmap, ..default() });
}

fn slot_mesh(slots: u32) -> Mesh {
    let mut pos = Vec::with_capacity(slots as usize * 7);
    for s in 0..slots {
        for c in 0..7u32 {
            pos.push([s as f32, c as f32, 0.0]);
        }
    }
    let mut idx = Vec::with_capacity(slots as usize * 15);
    for s in 0..slots {
        let b = s * 7;
        // Left / right pairs up the blade, then the tip.
        for t in [[0, 1, 2], [1, 3, 2], [2, 3, 4], [3, 5, 4], [4, 5, 6]] {
            idx.extend(t.iter().map(|k| b + k));
        }
    }
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_indices(Indices::U32(idx));
    m
}

#[allow(clippy::too_many_arguments)]
pub fn grass_system(
    mut commands: Commands,
    time: Res<Time>,
    grass: Option<ResMut<Grass>>,
    cam: Single<&Transform, (With<FlyCam>, Without<SkyCam>)>,
    player: Option<Res<Player>>,
    blockers: Option<Res<crate::actors::Blockers>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<GrassMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut vis: Query<&mut Visibility>,
    lres: Option<Res<LevelRes>>,
) {
    let Some(mut g) = grass else { return };
    let lv = level();
    if g.field.is_empty() || lv == 0 || lres.is_none() {
        if let Some((e, ..)) = g.drawn {
            if let Ok(mut v) = vis.get_mut(e) {
                *v = Visibility::Hidden;
            }
        }
        return;
    }
    let (cap, per, near, far) = budget(lv);
    // (Re)build the slot mesh for this level.
    if g.drawn.as_ref().is_none_or(|d| d.3 != lv || vis.get(d.0).is_err()) {
        if let Some((e, ..)) = g.drawn.take() {
            if let Ok(mut ec) = commands.get_entity(e) {
                ec.despawn();
            }
        }
        let Some(lm) = g.lightmap.clone() else { return };
        let rows = (cap as u32 * TEXELS).div_ceil(DATA_W);
        let mut img = Image::new_fill(
            bevy::render::render_resource::Extent3d { width: DATA_W, height: rows, depth_or_array_layers: 1 },
            bevy::render::render_resource::TextureDimension::D2,
            &[0u8; 16],
            bevy::render::render_resource::TextureFormat::Rgba32Float,
            RenderAssetUsages::default(),
        );
        img.sampler = bevy::image::ImageSampler::nearest();
        let data = images.add(img);
        let mat = mats.add(GrassMaterial { data: data.clone(), lightmap: lm, params: GrassUniform::default(), pushes: [Vec4::ZERO; 16] });
        let e = commands
            .spawn((
                Mesh3d(meshes.add(slot_mesh(cap as u32 * per))),
                MeshMaterial3d(mat.clone()),
                Transform::default(),
                Visibility::Inherited,
                bevy::camera::primitives::Aabb::from_min_max(Vec3::splat(-1.0e5), Vec3::splat(1.0e5)),
                bevy::camera::visibility::NoAutoAabb,
                bevy::light::NotShadowCaster,
                bevy::light::NotShadowReceiver,
            ))
            .id();
        g.drawn = Some((e, mat, data, lv));
        g.last_t = -1.0;
    }
    let Some((e, mat, data, _)) = g.drawn.clone() else { return };
    if let Ok(mut v) = vis.get_mut(e) {
        if *v != Visibility::Inherited {
            *v = Visibility::Inherited;
        }
    }
    let eye = cam.translation;
    let now = time.elapsed_secs();
    let mut repack = false;
    if g.last_t < 0.0 || eye.distance(g.last_eye) > 0.4 || now - g.last_t > 0.5 {
        g.last_eye = eye;
        g.last_t = now;
        let g = &mut *g;
        g.field.near(eye, far, cap, &mut g.near);
        g.texels.clear();
        g.texels.resize(g.near.len() * TEXELS as usize, [0.0; 4]);
        g.field.pack(&g.near, &mut g.texels);
        if let Some(mut img) = images.get_mut(&data) {
            if let Some(bytes) = img.data.as_mut() {
                for (k, t) in g.texels.iter().enumerate() {
                    let at = k * 16;
                    if at + 16 > bytes.len() {
                        break;
                    }
                    for (c, v) in t.iter().enumerate() {
                        bytes[at + c * 4..at + c * 4 + 4].copy_from_slice(&v.to_le_bytes());
                    }
                }
            }
        }
        repack = true;
    }
    // What stands on the grass: the player and the people and things near the eye (Bevy metres, radius).
    let mut pushes = [Vec4::ZERO; 16];
    let mut n = 0usize;
    let mut add = |c: glam::DVec3, r: f64, h: f64| {
        if n < 16 {
            let foot = c - glam::DVec3::new(0.0, 0.0, h);
            let b = dx_level::ue_to_bevy([foot.x as f32, foot.y as f32, foot.z as f32]);
            pushes[n] = Vec4::new(b[0], b[1], b[2], r as f32 * dx_level::METERS_PER_UNIT);
            n += 1;
        }
    };
    if let Some(p) = player.as_ref() {
        add(p.body.location, p.params.radius, p.params.height);
    }
    if let Some(bl) = blockers.as_ref() {
        let eye_ue = dx_level::bevy_to_ue(eye.to_array());
        let eye_ue = glam::DVec3::new(eye_ue[0] as f64, eye_ue[1] as f64, eye_ue[2] as f64);
        let mut near_b: Vec<(f64, &dx_level::collision::Cylinder)> = bl
            .list
            .iter()
            .filter(|c| c.radius > 4.0)
            .map(|c| (c.center.distance(eye_ue), c))
            .filter(|(d, _)| *d < 900.0)
            .collect();
        near_b.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, c) in near_b.into_iter().take(15) {
            add(c.center, c.radius, c.height);
        }
    }
    let Some(mut m) = mats.get_mut(&mat) else { return };
    let w = 0.35f32;
    m.params = GrassUniform {
        a: Vec4::new(g.near.len() as f32, per as f32, 0.0, lv as f32),
        // A light breeze off the sea with gusts (a direction and strength of its own; ours).
        wind: Vec4::new(0.8, 0.6, 0.05 * w, 0.35 * w),
        eye: eye.extend(near),
        b: Vec4::new(1.0, n as f32, 1.0, far),
    };
    m.pushes = pushes;
    let _ = repack;
}
