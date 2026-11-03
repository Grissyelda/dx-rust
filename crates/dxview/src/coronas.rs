//! Light coronas: lights with bCorona draw their Skin texture as a glow over the scene at the light, when the
//! light is within 2000 units and nothing in the level blocks the line to it. Size is DrawScale x 0.2 x the
//! screen width; colour is the light's hue/saturation at full brightness x 2.5 (as SurrealEngine draws Deus Ex's).
//! The glow is additive in the original; here the texture is turned into an alpha-blended image with the same
//! look over dark backgrounds (alpha = brightest channel, colour divided by it).

use crate::weapons::RuntimeAssets;
use crate::{FlyCam, LevelRes, SkyCam};
use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use dx_level::{bevy_to_ue, ue_to_bevy};
use glam::DVec3;

#[derive(Component)]
pub struct CoronaSprite(usize);

/// Enhancement "Modern Coronas" (setting ModernCoronas, off by default): the lights' corona pictures replaced by a
/// drawn glow - a bright core, a wide soft halo, a faint horizontal streak and four thin diffraction spikes, in the
/// light's colour - that fades in and out over a moment instead of popping when something passes in front.
pub fn modern() -> bool {
    setting!("ModernCoronas") > 0.5
}

/// The modern glow picture for a light colour (sRGB, straight alpha).
fn modern_glow(color: [f32; 3]) -> Image {
    const N: usize = 256;
    let mut out = Vec::with_capacity(N * N * 4);
    let m = color[0].max(color[1]).max(color[2]).max(1e-3);
    let tint = [color[0] / m, color[1] / m, color[2] / m];
    for y in 0..N {
        for x in 0..N {
            let u = (x as f32 + 0.5) / N as f32 * 2.0 - 1.0;
            let v = (y as f32 + 0.5) / N as f32 * 2.0 - 1.0;
            let r2 = u * u + v * v;
            // A small hot core and a soft gaussian bloom (no streaks or spikes), gone at the picture's rim.
            let rim = (1.0 - r2).max(0.0);
            let core = (-r2 / 0.006).exp();
            let glow = ((-r2 / 0.035).exp() * 0.5 + (-r2 / 0.2).exp() * 0.2) * rim;
            let a = (core + glow).min(1.0);
            // The core whitens; the halo keeps the light's colour.
            let w = core.min(1.0);
            let rgb = [tint[0] + (1.0 - tint[0]) * w, tint[1] + (1.0 - tint[1]) * w, tint[2] + (1.0 - tint[2]) * w];
            out.extend_from_slice(&[(rgb[0] * 255.0) as u8, (rgb[1] * 255.0) as u8, (rgb[2] * 255.0) as u8, (a * 255.0) as u8]);
        }
    }
    Image::new(
        Extent3d { width: N as u32, height: N as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        out,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

pub fn spawn_coronas(
    commands: &mut Commands,
    level: &dx_level::Level,
    assets: &mut RuntimeAssets,
    images: &mut Assets<Image>,
) {
    let mut cache: std::collections::HashMap<(String, [u32; 3]), Handle<Image>> =
        std::collections::HashMap::new();
    for (i, c) in level.coronas.iter().enumerate() {
        if env_on!("DXVIEW_CORONALOG") {
            println!("CORONA {i} at bevy {:?} scale {} tex {}", ue_to_bevy(c.pos), c.draw_scale, c.texture);
        }
        let key = (c.texture.clone(), c.color.map(|v| (v * 1000.0) as u32));
        let modern = modern();
        let key = if modern { (String::from("#modern"), key.1) } else { key };
        let handle = match cache.get(&key) {
            Some(h) => h.clone(),
            None if modern => {
                let hd = images.add(modern_glow(c.color));
                cache.insert(key, hd.clone());
                hd
            }
            None => {
                let Some(t) = assets.lib.load_image(&c.texture) else {
                    continue;
                };
                let (w, h) = (t.width, t.height);
                let src = &t.mips[0];
                let mut out = Vec::with_capacity(src.len());
                for px in src.chunks(4) {
                    // sRGB-encoded texel tinted by the light colour x 2.5.
                    let mut rgb = [0f32; 3];
                    for k in 0..3 {
                        rgb[k] = (px[k] as f32 / 255.0 * c.color[k] * 2.5).min(1.0);
                    }
                    let a = rgb[0].max(rgb[1]).max(rgb[2]);
                    if a <= 1e-3 {
                        out.extend_from_slice(&[0, 0, 0, 0]);
                    } else {
                        out.extend_from_slice(&[
                            (rgb[0] / a * 255.0) as u8,
                            (rgb[1] / a * 255.0) as u8,
                            (rgb[2] / a * 255.0) as u8,
                            (a * 255.0) as u8,
                        ]);
                    }
                }
                let img = Image::new(
                    Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                    TextureDimension::D2,
                    out,
                    TextureFormat::Rgba8UnormSrgb,
                    RenderAssetUsages::RENDER_WORLD,
                );
                let hd = images.add(img);
                cache.insert(key, hd.clone());
                hd
            }
        };
        commands.spawn((
            ImageNode::new(handle),
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            Visibility::Hidden,
            Pickable::IGNORE,
            GlobalZIndex(-10),
            CoronaSprite(i),
        ));
    }
}

#[allow(clippy::too_many_arguments)]
pub fn corona_system(
    mut commands: Commands,
    windows: Query<&Window>,
    time: Res<Time>,
    cam: Single<(&Camera, &GlobalTransform, &Transform), (With<FlyCam>, Without<SkyCam>)>,
    level: Res<LevelRes>,
    mut q: Query<(Entity, &CoronaSprite, &mut Node, &mut Visibility, &mut ImageNode)>,
    mut fade: Local<std::collections::HashMap<usize, f32>>,
    mut built: Local<Option<bool>>,
    mut map_gen: Local<u64>,
    mut assets: ResMut<RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut core_state: Local<CoreState>,
    mut los: Local<Vec<(f32, bool)>>,
) {
    let _prof = crate::prof::scope("coronas::corona_system");
    if crate::loader::new_map(&mut map_gen) {
        *built = Some(modern());
        fade.clear();
    }
    // The setting changed: the sprites are rebuilt with the other pictures.
    if *built != Some(modern()) {
        *built = Some(modern());
        for (e, ..) in &q {
            commands.entity(e).despawn();
        }
        spawn_coronas(&mut commands, &level.0, &mut assets, &mut images);
        return;
    }
    let Some(win) = windows.iter().next() else {
        return;
    };
    let (camera, gt, tf) = *cam;
    let eye = bevy_to_ue(tf.translation.to_array());
    let eye = DVec3::new(eye[0] as f64, eye[1] as f64, eye[2] as f64);
    let width = win.width();
    let modern = modern();
    let dt = time.delta_secs().min(0.1);
    let mut cores: Vec<(Vec3, [f32; 3], f32, f32)> = Vec::new();
    let now = time.elapsed_secs();
    if los.len() != level.0.coronas.len() {
        los.clear();
        los.resize(level.0.coronas.len(), (-1.0, false));
    }
    for (_, c, mut node, mut vis, mut img) in &mut q {
        let co = &level.0.coronas[c.0];
        let p = DVec3::new(co.pos[0] as f64, co.pos[1] as f64, co.pos[2] as f64);
        let mut shown = false;
        let mut strength = 1.0f32;
        // The cheap tests first (range, in front of the camera); the line of sight is traced at 10 Hz per corona
        // and remembered (one trace per frame for every lamp in range was most of this system).
        let to = Vec3::from(ue_to_bevy(co.pos)) - tf.translation;
        let cosv = to.normalize_or_zero().dot(*tf.forward());
        let in_range = cosv > 0.0 && p.distance(eye) < 2000.0;
        let clear = in_range && {
            let l = &mut los[c.0];
            if now - l.0 >= 0.1 {
                l.0 = now + (c.0 % 6) as f32 * 0.016;
                l.1 = level.0.collision.trace(p, eye, DVec3::ZERO).fraction >= 1.0;
            }
            l.1
        };
        if clear {
            if let Ok(v) = camera.world_to_viewport(gt, Vec3::from(ue_to_bevy(co.pos))) {
                {
                    let mut size = co.draw_scale * width * 0.2;
                    if modern {
                        // Modern: a bloom the size of a real glow round the bulb (about two metres across at
                        // draw scale 1), so it shrinks with distance like the lamp; never below a few pixels.
                        let world_r = 60.0 * co.draw_scale.clamp(0.3, 2.0);
                        let right = *tf.right() * world_r * dx_level::METERS_PER_UNIT;
                        let edge = camera.world_to_viewport(gt, Vec3::from(ue_to_bevy(co.pos)) + right);
                        let r_px = edge.map_or(0.0, |e| (e - v).length());
                        size = (r_px * 2.0).clamp(width * 0.02, width * 0.12);
                        strength = 0.6 + 0.4 * cosv * cosv;
                    }
                    let (l, t, w) = (Val::Px(v.x - size * 0.5), Val::Px(v.y - size * 0.5), Val::Px(size));
                    if node.left != l || node.top != t || node.width != w {
                        node.left = l;
                        node.top = t;
                        node.width = w;
                        node.height = w;
                    }
                    shown = true;
                }
            }
        }
        if modern {
            // Ease in and out over about 0.15 s instead of popping.
            let f = fade.entry(c.0).or_insert(0.0);
            let want = if shown { strength } else { 0.0 };
            *f += (want - *f) * (dt * 8.0).min(1.0);
            let a = *f;
            let col = Color::srgba(1.0, 1.0, 1.0, a);
            if img.color != col {
                img.color = col;
            }
            if a > 0.01 {
                cores.push((Vec3::from(ue_to_bevy(co.pos)), co.color, a, co.draw_scale.clamp(0.3, 2.0)));
            }
            vis.set_if_neq(if a > 0.01 { Visibility::Inherited } else { Visibility::Hidden });
        } else {
            if shown {
                cores.push((Vec3::from(ue_to_bevy(co.pos)), co.color, 1.0, co.draw_scale.clamp(0.3, 2.0)));
            }
            if img.color != Color::WHITE {
                img.color = Color::WHITE;
            }
            vis.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
        }
    }
    write_cores(&mut commands, &mut core_state, &mut meshes, &mut mats, &mut images, tf, &cores);
}

/// Coronas into the bloom (with Glow And Bloom on): a small hot core of each visible corona drawn in the world
/// (HDR, additive), so the bloom pass spreads it round the bulb like the real glow; the 2D corona stays on top.
#[derive(Default)]
pub struct CoreState {
    mesh: Option<(Handle<Mesh>, Entity)>,
    gen: u64,
    /// Cores written last time (0 = the mesh holds collapsed quads).
    written: usize,
}

const CORE_CAP: usize = 64;

fn core_image() -> Image {
    let n = 32u32;
    let mut px = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let dx = (x as f32 + 0.5) / n as f32 * 2.0 - 1.0;
            let dy = (y as f32 + 0.5) / n as f32 * 2.0 - 1.0;
            let a = (-(dx * dx + dy * dy) * 6.0).exp();
            let v = (a * 255.0) as u8;
            px.extend_from_slice(&[v, v, v, 255]);
        }
    }
    Image::new(
        Extent3d { width: n, height: n, depth_or_array_layers: 1 },
        TextureDimension::D2,
        px,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn write_cores(
    commands: &mut Commands,
    st: &mut CoreState,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    tf: &Transform,
    cores: &[(Vec3, [f32; 3], f32, f32)],
) {
    let on = crate::glow::on();
    let gen = crate::loader::generation();
    if st.gen != gen {
        st.gen = gen;
        st.mesh = None;
    }
    if st.mesh.is_none() {
        if !on {
            return;
        }
        let mut m = Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; CORE_CAP * 4]);
        m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; CORE_CAP * 4]);
        m.insert_attribute(Mesh::ATTRIBUTE_UV_0, (0..CORE_CAP).flat_map(|_| [[0.0f32, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]).collect::<Vec<_>>());
        m.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; CORE_CAP * 4]);
        m.insert_indices(bevy::mesh::Indices::U32((0..CORE_CAP as u32).flat_map(|q| [q * 4, q * 4 + 1, q * 4 + 2, q * 4, q * 4 + 2, q * 4 + 3]).collect()));
        let h = meshes.add(m);
        let mat = mats.add(StandardMaterial {
            base_color: Color::LinearRgba(LinearRgba::rgb(6.0, 6.0, 6.0)),
            base_color_texture: Some(images.add(core_image())),
            unlit: true,
            alpha_mode: AlphaMode::Add,
            fog_enabled: false,
            ..default()
        });
        let e = commands
            .spawn((
                Mesh3d(h.clone()),
                MeshMaterial3d(mat),
                Transform::default(),
                bevy::camera::primitives::Aabb::from_min_max(Vec3::splat(-1.0e5), Vec3::splat(1.0e5)),
                bevy::camera::visibility::NoAutoAabb,
                bevy::light::NotShadowCaster,
            ))
            .id();
        st.mesh = Some((h, e));
    }
    let Some((h, _)) = &st.mesh else { return };
    // Nothing to show and nothing shown last time: the mesh already holds collapsed quads.
    let n = if on { cores.len().min(CORE_CAP) } else { 0 };
    if n == 0 && st.written == 0 {
        return;
    }
    let Some(mut m) = meshes.get_mut(h) else { return };
    let right = *tf.right();
    let up = *tf.up();
    if let Some(bevy::mesh::VertexAttributeValues::Float32x3(pos)) = m.attribute_mut(Mesh::ATTRIBUTE_POSITION) {
        for (i, p) in pos.iter_mut().enumerate() {
            let c = i / 4;
            *p = if c < n {
                let (at, _, _, scale) = cores[c];
                // About 9 units across at draw scale 1: the bulb itself.
                let r = 4.5 * scale * dx_level::METERS_PER_UNIT;
                let (sx, sy) = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)][i % 4];
                (at + right * r * sx + up * r * sy).to_array()
            } else {
                [0.0; 3]
            };
        }
    }
    if let Some(bevy::mesh::VertexAttributeValues::Float32x4(col)) = m.attribute_mut(Mesh::ATTRIBUTE_COLOR) {
        for (i, cl) in col.iter_mut().enumerate() {
            let c = i / 4;
            *cl = if c < n {
                let (_, cc, a, _) = cores[c];
                [cc[0] * a, cc[1] * a, cc[2] * a, 1.0]
            } else {
                [0.0; 4]
            };
        }
    }
    st.written = n;
}
