//! Enhancement "Trees" (setting Trees; Modern preset on): the map's trees and bushes (Tree1-Tree4, Bushes1-Bushes3:
//! flat crossed-card meshes) are swapped for plants grown by imsim-plants (shared with the second game): branching
//! bark grown toward the crown by space colonisation, leaf clusters at the twigs, four variants of each kind per map,
//! sized to the original's height and spread. Ours, after SecondRevision's generated trees (zlib; the idea, not its
//! code). They keep the original's collision, are lit by the map's lights the way every mesh actor is (dx-level's
//! light_part rule, leaves lit through from behind a little), cast real-time shadows (shadowlight.rs, one light
//! each) and sway in the same breeze as the grass (shaders/plant_sway.wgsl). Bark and leaf pictures are made here.

use crate::actors::MeshActorRef;
use crate::LevelRes;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat};
use bevy::shader::ShaderRef;
use imsim_plants::tree::{grow, MeshData, Plant, Species};
use std::collections::HashMap;

pub type PlantMaterial = ExtendedMaterial<StandardMaterial, Sway>;

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
pub struct Sway {}

pub const SWAY_SHADER: Handle<Shader> = bevy::asset::uuid_handle!("5e2a9c71-4b3d-4f60-9a18-2d7c6e0f1b35");

impl MaterialExtension for Sway {
    fn vertex_shader() -> ShaderRef {
        SWAY_SHADER.into()
    }
    // Not in the depth prepass: there it would stand unswayed, and where the prepass's leaf and the drawn (swayed)
    // one differ the camera's clear colour showed through (blue leaf-shaped holes). Shadows use the still pose.
    fn enable_prepass() -> bool {
        false
    }
}

pub fn on() -> bool {
    setting!("Trees") > 0.5
}

/// The kind of plant each original class becomes.
fn species_for(class: &str) -> Option<(u32, Species)> {
    Some(match class.to_ascii_lowercase().as_str() {
        "tree1" | "tree2" => (1, Species::park_tree()),
        "tree3" => (3, Species::tall_tree()),
        "tree4" => (4, Species { crown: [0.42, 0.3], trunk: 0.38, ..Species::park_tree() }),
        "bushes1" | "bushes2" | "bushes3" => (7, Species::bush()),
        _ => return None,
    })
}

/// A plant part (bark or leaves) we added under an original tree.
#[derive(Component)]
pub struct PlantPart;

pub struct TreesPlugin;

impl Plugin for TreesPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<PlantMaterial>::default())
            .add_systems(Startup, |mut shaders: ResMut<Assets<Shader>>| {
                let _ = shaders.insert(&SWAY_SHADER, Shader::from_wgsl(include_str!("shaders/plant_sway.wgsl"), "dxview/plant_sway.wgsl"));
            })
            .add_systems(Update, trees_system);
    }
}

#[derive(Default)]
pub struct TreeState {
    gen: u64,
    on: bool,
    /// (kind, variant) -> grown plant.
    grown: HashMap<(u32, u32), Plant>,
    mats: Option<(Handle<PlantMaterial>, Handle<PlantMaterial>)>,
}

fn noise(x: u32, y: u32, s: u32) -> f32 {
    (imsim_plants::hash(x.wrapping_mul(0x27d4_eb2d) ^ y.wrapping_mul(0x1656_67b1) ^ s) & 0xFFFF) as f32 / 65535.0
}

/// Bark: grey-brown with vertical furrows and a little noise (256 x 256, tiles both ways).
fn bark_image() -> Image {
    let n = 256u32;
    let mut px = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let fx = x as f32 / n as f32;
            // Furrows: wavy dark lines down the trunk.
            let wave = (fx * 9.0 * std::f32::consts::TAU + (y as f32 / n as f32 * std::f32::consts::TAU * 2.0).sin() * 0.8).sin();
            let ridge = 0.62 + 0.38 * wave.abs().powf(0.35);
            let grain = 0.85 + 0.3 * noise(x / 2, y / 6, 7) - 0.15 * noise(x, y, 3);
            let v = (ridge * grain).clamp(0.0, 1.3);
            let (r, g, b) = (0.36 * v, 0.30 * v, 0.24 * v);
            px.extend_from_slice(&[(r.min(1.0) * 255.0) as u8, (g.min(1.0) * 255.0) as u8, (b.min(1.0) * 255.0) as u8, 255]);
        }
    }
    let mut img = Image::new(Extent3d { width: n, height: n, depth_or_array_layers: 1 }, TextureDimension::D2, px, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD);
    img.sampler = bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
        address_mode_u: bevy::image::ImageAddressMode::Repeat,
        address_mode_v: bevy::image::ImageAddressMode::Repeat,
        mag_filter: bevy::image::ImageFilterMode::Linear,
        min_filter: bevy::image::ImageFilterMode::Linear,
        ..default()
    });
    img
}

/// Leaves: four clusters in a 2 x 2 atlas, each ~45 small pointed leaves in greens on transparent (alpha tested).
fn leaf_image() -> Image {
    let n = 256u32;
    let mut px = vec![0u8; (n * n * 4) as usize];
    for cell in 0..4u32 {
        let (cx0, cy0) = ((cell % 2) * n / 2, (cell / 2) * n / 2);
        let half = n as f32 / 4.0;
        let mut s = imsim_plants::hash(cell * 977 + 13);
        for _ in 0..46 {
            // A leaf: an ellipse pointed at both ends, somewhere in the cluster's disc, turned any way.
            let a = imsim_plants::rand(&mut s) * std::f32::consts::TAU;
            let r = imsim_plants::rand(&mut s).sqrt() * half * 0.78;
            let (lx, ly) = (half + a.cos() * r, half + a.sin() * r);
            let turn = imsim_plants::rand(&mut s) * std::f32::consts::PI;
            let len = half * (0.16 + 0.1 * imsim_plants::rand(&mut s));
            let wid = len * 0.42;
            let shade = 0.65 + 0.5 * imsim_plants::rand(&mut s);
            let yellow = imsim_plants::rand(&mut s) < 0.1;
            let (c, sn) = (turn.cos(), turn.sin());
            let ext = len as i32 + 2;
            for dy in -ext..=ext {
                for dx in -ext..=ext {
                    let (px_, py_) = (lx + dx as f32, ly + dy as f32);
                    if px_ < 0.0 || py_ < 0.0 || px_ >= half * 2.0 || py_ >= half * 2.0 {
                        continue;
                    }
                    let u = (dx as f32 * c + dy as f32 * sn) / len;
                    let v = (-dx as f32 * sn + dy as f32 * c) / wid;
                    // Pointed ends: the width narrows toward both tips.
                    let w = (1.0 - u * u).max(0.0);
                    if u.abs() >= 1.0 || v.abs() >= w {
                        continue;
                    }
                    let vein = if v.abs() < 0.08 { 1.15 } else { 1.0 };
                    let edge = 0.8 + 0.2 * (1.0 - v.abs() / w.max(1e-3));
                    let k = shade * vein * edge;
                    let (r, g, b) = if yellow { (0.42 * k, 0.40 * k, 0.12 * k) } else { (0.16 * k, 0.30 * k, 0.08 * k) };
                    let i = (((cy0 + py_ as u32) * n + cx0 + px_ as u32) * 4) as usize;
                    px[i] = (r.min(1.0) * 255.0) as u8;
                    px[i + 1] = (g.min(1.0) * 255.0) as u8;
                    px[i + 2] = (b.min(1.0) * 255.0) as u8;
                    px[i + 3] = 255;
                }
            }
        }
    }
    Image::new(Extent3d { width: n, height: n, depth_or_array_layers: 1 }, TextureDimension::D2, px, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD)
}

/// The plant's mesh in the actor's own (Bevy) frame, scaled to the original, lit like a mesh actor at `location`
/// (Unreal) by `lights`; `leaves`: lit through from behind a little.
#[allow(clippy::too_many_arguments)]
fn plant_mesh(m: &MeshData, scale: Vec3, foot: Vec3, fit: &[f32; DIRS], root: &GlobalTransform, lights: &[dx_level::lighting::LightInfo], amb: [f32; 3], leaves: bool, glow: f32) -> Mesh {
    let pos: Vec<[f32; 3]> = m.positions.iter().map(|p| (squeeze(Vec3::from(*p) * scale, fit) + foot).to_array()).collect();
    let colors: Vec<[f32; 4]> = pos
        .iter()
        .zip(&m.normals)
        .map(|(p, n)| {
            let w = root.transform_point(Vec3::from(*p));
            let wn = root.rotation() * Vec3::from(*n);
            let pt = dx_level::bevy_to_ue(w.to_array());
            // Bevy (x, y, z) normal -> Unreal (x, z, y).
            let un = [wn.x, wn.z, wn.y];
            let mut c = amb;
            for l in lights {
                let to = [l.pos[0] - pt[0], l.pos[1] - pt[1], l.pos[2] - pt[2]];
                let d = (to[0] * to[0] + to[1] * to[1] + to[2] * to[2]).sqrt();
                if d >= l.radius {
                    continue;
                }
                let dn = (to[0] * un[0] + to[1] * un[1] + to[2] * un[2]) / d.max(1e-3);
                let ndl = if leaves { (dn * 0.65 + 0.35).max(0.0) } else { dn.max(0.0) };
                for ch in 0..3 {
                    c[ch] += l.color[ch] * dx_level::lighting::mesh_falloff(d, l.radius) * dx_level::lighting::MESH_GLOW * ndl;
                }
            }
            // Never darker than the original (unlit, at its ScaleGlow).
            let lin = |v: f32| (v * dx_level::lighting::LIGHT_GAIN).max(glow).clamp(0.0, 1.0).powf(2.2);
            [lin(c[0]), lin(c[1]), lin(c[2]), 1.0]
        })
        .collect();
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, m.normals.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, m.uvs.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, m.sway.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(m.indices.clone()));
    mesh
}

/// Directions round the trunk the free space is measured in.
const DIRS: usize = 16;

fn dir_of(k: usize) -> Vec2 {
    let a = k as f32 / DIRS as f32 * std::f32::consts::TAU;
    Vec2::new(a.cos(), a.sin())
}

/// The plant's horizontal squeeze toward the trunk by direction (1 = as grown), interpolated between the measured
/// directions.
fn squeeze(p: Vec3, fit: &[f32; DIRS]) -> Vec3 {
    let a = p.z.atan2(p.x).rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU * DIRS as f32;
    let i = a.floor() as usize % DIRS;
    let t = a - a.floor();
    let f = fit[i] * (1.0 - t) + fit[(i + 1) % DIRS] * t;
    Vec3::new(p.x * f, p.y, p.z * f)
}

/// How far the plant reaches out in each direction (actor-local Bevy metres, after `scale`), from its vertices
/// above `min_y`, spread a little to the neighbouring directions.
fn reach(plant: &Plant, scale: Vec3, min_y: f32) -> [f32; DIRS] {
    let mut r = [0.0f32; DIRS];
    for p in plant.bark.positions.iter().chain(&plant.leaves.positions) {
        let q = Vec3::from(*p) * scale;
        if q.y < min_y {
            continue;
        }
        let a = q.z.atan2(q.x).rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU * DIRS as f32;
        let k = a.round() as usize % DIRS;
        let d = Vec2::new(q.x, q.z).length();
        r[k] = r[k].max(d);
    }
    let mut out = r;
    for k in 0..DIRS {
        out[k] = r[k].max(r[(k + 1) % DIRS] * 0.85).max(r[(k + DIRS - 1) % DIRS] * 0.85);
    }
    out
}

/// The free space round a plant (actor-local Bevy metres) in each direction, at heights `ys` (actor-local Bevy
/// metres above the root): the nearest wall (BSP) or solid actor (a pole, a lamp, a crate - the tree's own
/// cylinder skipped) on a level line out from the trunk, less a margin.
fn room(lvl: &dx_level::Level, blockers: &[dx_level::collision::Cylinder], own: usize, gt: &GlobalTransform, ys: &[f32], max: f32) -> [f32; DIRS] {
    use bevy::math::DVec3;
    let k = gt.compute_transform().scale.x.max(1e-3);
    let to_ue = |p: Vec3| {
        let u = dx_level::bevy_to_ue(gt.transform_point(p).to_array());
        DVec3::new(u[0] as f64, u[1] as f64, u[2] as f64)
    };
    let base = to_ue(Vec3::ZERO);
    let mut out = [max; DIRS];
    for (i, o) in out.iter_mut().enumerate() {
        let d = dir_of(i);
        for &y in ys {
            let a = to_ue(Vec3::new(0.0, y, 0.0));
            let b = to_ue(Vec3::new(d.x * max, y, d.y * max));
            let len = (b - a).length().max(1e-3);
            let mut f = lvl.collision.trace(a, b, DVec3::ZERO).fraction;
            for c in blockers {
                if c.actor == own || c.radius < 3.0 || (c.center - base).truncate().length() < c.radius.max(12.0) {
                    continue;
                }
                if let Some(t) = crate::pickups::ray_cylinder(a, b, c.center, c.radius, c.height) {
                    f = f.min(t);
                }
            }
            // A margin of 12 units off what's there.
            let free = ((f * len - 12.0).max(0.0) as f32) * dx_level::METERS_PER_UNIT / k;
            *o = o.min(free);
        }
    }
    out
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn trees_system(
    mut commands: Commands,
    level: Option<Res<LevelRes>>,
    mut st: Local<TreeState>,
    roots: Query<(Entity, &MeshActorRef, &GlobalTransform, Option<&Children>)>,
    mut parts: Query<&mut Visibility, (With<Mesh3d>, Without<PlantPart>)>,
    ours: Query<Entity, With<PlantPart>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<PlantMaterial>>,
    mut images: ResMut<Assets<Image>>,
    blockers: Option<Res<crate::actors::Blockers>>,
) {
    let Some(level) = level else { return };
    if roots.is_empty() {
        return;
    }
    let gen = crate::loader::generation();
    let want = on();
    if st.gen == gen && st.on == want {
        return;
    }
    // Wait for the mesh actors' transforms (a frame after the map is built).
    if roots.iter().next().is_some_and(|(_, _, gt, _)| *gt == GlobalTransform::IDENTITY) {
        return;
    }
    let first_on_map = st.gen != gen;
    st.gen = gen;
    st.on = want;
    for e in &ours {
        commands.entity(e).despawn();
    }
    if first_on_map {
        st.grown.clear();
    }
    let lvl = &level.0;
    let map_seed = imsim_plants::hash(lvl.name.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32)));
    if st.mats.is_none() {
        let bark = images.add(bark_image());
        let leaf = images.add(leaf_image());
        let bark_m = mats.add(PlantMaterial {
            base: StandardMaterial { base_color_texture: Some(bark), unlit: true, perceptual_roughness: 1.0, ..default() },
            extension: Sway {},
        });
        let leaf_m = mats.add(PlantMaterial {
            base: StandardMaterial {
                base_color_texture: Some(leaf),
                unlit: true,
                alpha_mode: AlphaMode::Mask(0.5),
                double_sided: true,
                cull_mode: None,
                ..default()
            },
            extension: Sway {},
        });
        st.mats = Some((bark_m, leaf_m));
    }
    let Some((bark_m, leaf_m)) = st.mats.clone() else { return };
    let mut count = 0;
    for (root, r, gt, children) in &roots {
        let Some(ma) = lvl.mesh_actors.get(r.0) else { continue };
        let Some(info) = lvl.actors.get(ma.actor) else { continue };
        let Some((kind, species)) = species_for(&info.class) else { continue };
        // The original's own parts: hidden while ours stand in.
        if let Some(ch) = children {
            for c in ch.iter() {
                if let Ok(mut v) = parts.get_mut(c) {
                    let to = if want { Visibility::Hidden } else { Visibility::Inherited };
                    if *v != to {
                        *v = to;
                    }
                }
            }
        }
        if !want {
            continue;
        }
        // The original's size: its parts' bounds (Unreal units, actor-local).
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for p in ma.parts.iter().flat_map(|p| p.positions.iter()) {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        if lo[0] > hi[0] {
            continue;
        }
        let s = dx_level::METERS_PER_UNIT;
        let height = (hi[2] - lo[2]) * s;
        let spread = (hi[0] - lo[0]).max(hi[1] - lo[1]) * s;
        let variant = (imsim_plants::hash(ma.actor as u32 ^ map_seed) % 4) as u32;
        let plant = st.grown.entry((kind, variant)).or_insert_with(|| grow(&species, map_seed ^ (kind * 131 + variant * 7919)));
        let ph = (plant.bounds[1].y - plant.bounds[0].y).max(0.1);
        let pw = (plant.bounds[1].x - plant.bounds[0].x).max(plant.bounds[1].z - plant.bounds[0].z).max(0.1);
        let k = height / ph;
        // The crown no wider than the original's cards, and a little narrower than its height allows (the
        // original's cards spread wide and flat; a full crown that wide crowds what's round it).
        let kw = (spread / pw / k).clamp(0.6, 1.0) * k;
        let scale = Vec3::new(kw, k, kw);
        let foot = Vec3::from(dx_level::ue_to_bevy([0.0, 0.0, lo[2]]));
        // Fit it to its spot: in each direction the crown (and branches) are drawn in toward the trunk so they stop
        // short of the walls, poles and lamps round it (trees stood against buildings cut into them).
        let crown_y = species.trunk * height * 0.7;
        let top = height;
        let ys: Vec<f32> = (0..4).map(|i| foot.y + crown_y + (top - crown_y) * (0.15 + 0.25 * i as f32)).collect();
        let grown_reach = reach(plant, scale, crown_y * 0.5);
        let max_reach = grown_reach.iter().fold(0.0f32, |a, b| a.max(*b)) + 0.5;
        let free = room(lvl, blockers.as_ref().map_or(&[][..], |b| &b.list[..]), ma.actor, gt, &ys, max_reach);
        let mut fit = [1.0f32; DIRS];
        for i in 0..DIRS {
            if grown_reach[i] > free[i] {
                fit[i] = (free[i] / grown_reach[i].max(1e-3)).clamp(0.3, 1.0);
            }
        }
        // Smoothed round the trunk (no sudden notch).
        let raw = fit;
        for i in 0..DIRS {
            fit[i] = raw[i].min((raw[(i + 1) % DIRS] + raw[(i + DIRS - 1) % DIRS]) * 0.5 + 0.15);
        }
        if env_on!("DXVIEW_TREELOG") {
            let min = fit.iter().fold(1.0f32, |a, b| a.min(*b));
            println!("TREE {} {} h {height:.1} m reach {max_reach:.1} m fit min {min:.2} {:?}", info.class, ma.actor, fit.map(|f| (f * 100.0) as u8));
        }
        let center = dx_level::bevy_to_ue(gt.translation().to_array());
        let lights = crate::dynlight::lights_at(lvl, center, (hi[2] - lo[2]) * 0.5);
        let amb = dx_level::actors::zone_ambient_at(center);
        for (m, mat, leaves) in [(&plant.bark, &bark_m, false), (&plant.leaves, &leaf_m, true)] {
            if m.indices.is_empty() {
                continue;
            }
            let mesh = meshes.add(plant_mesh(m, scale, foot, &fit, gt, &lights, amb, leaves, if ma.parts.iter().any(|p| p.unlit) { ma.scale_glow * 0.8 } else { 0.0 }));
            commands.spawn((
                Mesh3d(mesh),
                MeshMaterial3d(mat.clone()),
                Transform::default(),
                Visibility::Inherited,
                PlantPart,
                crate::shadowlight::ShadowCaster,
                crate::shadowlight::Casts,
                crate::shadowlight::caster_layers(&[]),
                ChildOf(root),
            ));
        }
        commands.entity(root).insert(crate::shadowlight::CastSize(spread.max(height * 0.5) / s * 0.5));
        count += 1;
    }
    if count > 0 {
        println!("trees: {count} plants grown ({} variants)", st.grown.len());
    }
}
