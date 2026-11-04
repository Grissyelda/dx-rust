//! Blood on the weapon (part of the Fluid Blood enhancement): blood thrown at the eye from close by - a point-blank
//! shot, a melee blow - spatters the first-person model in hand. Each spot sits on one of the model's vertices that
//! faces the eye and follows it through the animation (slides, reloads); a new weapon in hand starts clean, and the
//! spots dry off after a while.

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::weapons::{RuntimeAssets, ViewModelRoot, Weapons, VIEW_LAYER};

static PENDING: AtomicU32 = AtomicU32::new(0);

/// `n` spots of blood land on the weapon in hand (gore.rs).
pub fn splash(n: u32) {
    PENDING.fetch_add(n, Ordering::Relaxed);
}

/// Seconds a spot stays.
const LIFE: f32 = 90.0;
const CAP: usize = 28;

/// A spot sits on one triangle of the model (its three vertices and a point inside it), so it follows the pose.
struct Spot {
    e: Entity,
    part: usize,
    tri: [usize; 3],
    bary: (f32, f32),
    /// The triangle's normal is flipped to face the eye.
    flip: bool,
    spin: f32,
    size: f32,
    age: f32,
}

/// A triangle's point and its normal (model space) from the posed vertices.
fn tri_at(verts: &[[f32; 3]], tri: [usize; 3], bary: (f32, f32), flip: bool) -> Option<(Vec3, Vec3)> {
    let (a, b, c) = (Vec3::from(*verts.get(tri[0])?), Vec3::from(*verts.get(tri[1])?), Vec3::from(*verts.get(tri[2])?));
    let n = (b - a).cross(c - a).normalize_or_zero();
    if n == Vec3::ZERO {
        return None;
    }
    Some((a + (b - a) * bary.0 + (c - a) * bary.1, if flip { -n } else { n }))
}

#[derive(Default)]
pub struct State {
    spots: Vec<Spot>,
    /// What's in hand (class) the spots are on.
    on: Option<String>,
    seed: u64,
    mats: Vec<Handle<StandardMaterial>>,
    /// A spot too small to see, kept in front of the weapon camera so its pipeline is built before the first
    /// real one (else the first blood waits for a shader compile, or hitches the frame it appears).
    warm: Option<Entity>,
    tested: bool,
}

/// Spot shapes (white, the blood in alpha): a round drop, a spatter of droplets, a smear.
fn spot_images(images: &mut Assets<Image>) -> Vec<Handle<Image>> {
    const N: u32 = 32;
    let shapes: [&dyn Fn(f32, f32) -> f32; 3] = [
        &|x, y| {
            // A drop with a few arms thrown out of it.
            let r = (x * x + y * y).sqrt();
            let ang = y.atan2(x);
            let lobes = 0.18 + 0.07 * (ang * 5.0 + 0.7).sin().max(0.0) + 0.05 * (ang * 3.0).cos();
            1.0 - ((r - lobes) / 0.05).clamp(0.0, 1.0)
        },
        &|x, y| {
            let mut a: f32 = 0.0;
            for (cx, cy, r) in [(0.0, 0.0, 0.22), (0.25, 0.1, 0.1), (-0.2, 0.22, 0.08), (0.1, -0.28, 0.07), (-0.3, -0.12, 0.06)] {
                let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
                a = a.max(1.0 - ((d - r) / 0.04).clamp(0.0, 1.0));
            }
            a
        },
        &|x, y| {
            let d = (x * x * 0.35 + y * y * 3.0).sqrt();
            1.0 - ((d - 0.3) / 0.12).clamp(0.0, 1.0)
        },
    ];
    shapes
        .iter()
        .map(|f| {
            let mut data = Vec::with_capacity((N * N * 4) as usize);
            for py in 0..N {
                for px in 0..N {
                    let (x, y) = ((px as f32 + 0.5) / N as f32 - 0.5, (py as f32 + 0.5) / N as f32 - 0.5);
                    // A ragged edge (hash noise), so no spot is a clean disc.
                    let hsh = ((px.wrapping_mul(73856093) ^ py.wrapping_mul(19349663)) % 1000) as f32 / 1000.0;
                    let a = ((f(x, y) - 0.25 * hsh * (1.0 - f(x, y))) * 0.95).clamp(0.0, 1.0);
                    data.extend_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
                }
            }
            let mut img = Image::new(
                bevy::render::render_resource::Extent3d { width: N, height: N, depth_or_array_layers: 1 },
                bevy::render::render_resource::TextureDimension::D2,
                data,
                bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
                bevy::asset::RenderAssetUsages::RENDER_WORLD,
            );
            img.sampler = bevy::image::ImageSampler::linear();
            images.add(img)
        })
        .collect()
}

fn rnd(seed: &mut u64) -> f32 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    ((*seed >> 40) as f32) / (1u64 << 24) as f32
}

#[allow(clippy::too_many_arguments)]
pub fn view_blood_system(
    mut commands: Commands,
    time: Res<Time>,
    weapons: Res<Weapons>,
    root: Query<Entity, With<ViewModelRoot>>,
    (mut assets, mut images, mut meshes, mut materials): (
        ResMut<RuntimeAssets>,
        ResMut<Assets<Image>>,
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
    ),
    mut tf: Query<&mut Transform>,
    gts: Query<(&GlobalTransform, Option<&ViewVisibility>, Option<&InheritedVisibility>)>,
    vm_cam: Query<&Projection, With<crate::weapons::ViewModelCam>>,
    mut st: Local<State>,
) {
    let new = PENDING.swap(0, Ordering::Relaxed);
    let held = weapons.held.as_ref();
    let class = held.map(|h| h.stats.class.clone());
    // A different thing in hand (or none, or the blood setting off): clean.
    if class != st.on || !crate::gore::enabled() {
        for s in st.spots.drain(..) {
            commands.entity(s.e).despawn();
        }
        st.on = class;
    }
    let Some(h) = held else { return };
    if st.seed == 0 {
        st.seed = 0xB100D;
    }
    let dt = time.delta_secs();
    if st.mats.is_empty() {
        for (k, img) in spot_images(&mut images).into_iter().enumerate() {
            let shade = [0.18, 0.14, 0.16][k % 3];
            st.mats.push(materials.add(StandardMaterial {
                base_color_texture: Some(img),
                base_color: Color::srgba(shade, 0.012, 0.01, 1.0),
                unlit: true,
                double_sided: true,
                cull_mode: None,
                // Cut out, not blended: drawn with the solid gun in its own pass (crisp at this size).
                alpha_mode: AlphaMode::Mask(0.45),
                ..default()
            }));
        }
    }
    if crate::gore::enabled() && st.warm.is_none_or(|w| tf.get(w).is_err()) {
        if let Ok(root) = root.single() {
            let q = assets.sprites.quad(&mut meshes);
            st.warm = Some(
                commands
                    .spawn((
                        Mesh3d(q),
                        MeshMaterial3d(st.mats[0].clone()),
                        Transform::from_xyz(0.0, 0.0, -0.3).with_scale(Vec3::splat(1e-5)),
                        RenderLayers::layer(VIEW_LAYER),
                        bevy::light::NotShadowCaster,
                        ChildOf(root),
                    ))
                    .id(),
            );
        }
    }
    // Dev aid: DXVIEW_VBTEST=1 = a bright opaque test square and a blood one in the middle of the weapon view.
    if env_on!("DXVIEW_VBTEST") && !st.tested {
        if let Ok(root) = root.single() {
            st.tested = true;
            let q = assets.sprites.quad(&mut meshes);
            let green = materials.add(StandardMaterial { base_color: Color::srgb(0.0, 1.0, 0.0), unlit: true, ..default() });
            let rg = gts.get(root).map(|g| g.0.translation()).unwrap_or_default();
            for (m, x) in [(green, -0.04), (st.mats[0].clone(), -0.12)] {
                commands.spawn((
                    Mesh3d(q.clone()),
                    MeshMaterial3d(m),
                    Transform::from_translation(Vec3::new(x, -0.05, -0.5) - rg).with_scale(Vec3::splat(0.05)),
                    RenderLayers::layer(VIEW_LAYER),
                    ChildOf(root),
                ));
            }
            println!("VIEWBLOOD test squares under root at {rg:?}");
        }
    }
    // New spots: vertices facing the eye, spread over the model.
    if new > 0 && env_on!("DXVIEW_VBLOG") {
        println!("VIEWBLOOD {new} wanted, {} parts shown", h.shown.len());
    }
    if new > 0 && !h.shown.is_empty() && crate::gore::enabled() {
        let Ok(root) = root.single() else { return };
        let root_g = gts.get(root).map(|g| g.0.affine()).unwrap_or_default();
        // The weapon camera's tan(half fov) across and up.
        let cam_k = vm_cam
            .iter()
            .next()
            .map(|p| match p {
                Projection::Perspective(pp) => ((pp.fov * 0.5).tan() * pp.aspect_ratio, (pp.fov * 0.5).tan()),
                _ => (0.83, 0.47),
            })
            .unwrap_or((0.83, 0.47));
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in h.shown.iter().flatten() {
            lo = lo.min(Vec3::from(*p));
            hi = hi.max(Vec3::from(*p));
        }
        let ext = (hi - lo).length().max(1.0);
        let q = assets.sprites.quad(&mut meshes);
        // (Blended, not multiplied like the world's decals: guns are near black and a darkening stain vanished on
        // them. Wet dark red, in a few shapes.)
        // What the eye sees of the model: a coarse depth grid of its solid sections' triangles (in the weapon
        // camera's view), so blood lands on the top and back of the gun that face you, not on its hidden sides.
        let (kx, ky) = cam_k;
        const GW: usize = 48;
        const GH: usize = 32;
        let to_cell = |g: Vec3| -> Option<(usize, usize)> {
            if g.z > -0.01 {
                return None;
            }
            let (sx, sy) = (g.x / (-g.z * kx), g.y / (-g.z * ky));
            if sx.abs() >= 1.0 || sy.abs() >= 1.0 {
                return None;
            }
            Some((((sx + 1.0) * 0.5 * GW as f32) as usize, ((1.0 - (sy + 1.0) * 0.5) * GH as f32) as usize))
        };
        let solid = |pi: usize| h.parts.get(pi).is_some_and(|p| matches!(p.class, dx_level::SurfaceClass::Opaque | dx_level::SurfaceClass::Masked));
        let mut depth = vec![f32::MAX; GW * GH];
        for (pi, verts) in h.shown.iter().enumerate() {
            if !solid(pi) {
                continue;
            }
            let Some(part) = h.parts.get(pi) else { continue };
            for t in part.indices.chunks_exact(3) {
                let (Some(a), Some(b), Some(c)) = (verts.get(t[0] as usize), verts.get(t[1] as usize), verts.get(t[2] as usize)) else { continue };
                let (a, b, c) = (root_g.transform_point3(Vec3::from(*a)), root_g.transform_point3(Vec3::from(*b)), root_g.transform_point3(Vec3::from(*c)));
                for (u, v) in [(0.33, 0.33), (0.8, 0.1), (0.1, 0.8), (0.1, 0.1), (0.45, 0.45), (0.6, 0.2), (0.2, 0.6)] {
                    let p = a + (b - a) * u + (c - a) * v;
                    if let Some((x, y)) = to_cell(p) {
                        let d = &mut depth[y.min(GH - 1) * GW + x.min(GW - 1)];
                        *d = d.min(-p.z);
                    }
                }
            }
        }
        // Candidate triangles: solid, on screen (not near the edges - the grip and hand run off the bottom),
        // facing the eye and at the front of the depth grid (seen), weighted by their size on screen.
        let mut cands: Vec<(usize, [usize; 3], bool, f32, f32)> = Vec::new();
        for (pi, verts) in h.shown.iter().enumerate() {
            if !solid(pi) {
                continue;
            }
            let Some(part) = h.parts.get(pi) else { continue };
            for t in part.indices.chunks_exact(3) {
                let tri = [t[0] as usize, t[1] as usize, t[2] as usize];
                let Some((c, n)) = tri_at(verts, tri, (0.333, 0.333), false) else { continue };
                let g = root_g.transform_point3(c);
                let Some((cx, cy)) = to_cell(g) else { continue };
                let (sx, sy) = (g.x / (-g.z * kx), g.y / (-g.z * ky));
                if sx.abs() > 0.92 || sy < -0.85 || -g.z > depth[cy.min(GH - 1) * GW + cx.min(GW - 1)] + 0.008 {
                    continue;
                }
                let to_eye = -g.normalize_or(Vec3::Z);
                let toward = root_g.transform_vector3(n).normalize_or(Vec3::Z).dot(to_eye);
                if toward.abs() < 0.3 {
                    continue;
                }
                let (pa, pb, pc) = (Vec3::from(verts[tri[0]]), Vec3::from(verts[tri[1]]), Vec3::from(verts[tri[2]]));
                let area = (pb - pa).cross(pc - pa).length() * 0.5 * toward.abs() / (g.z * g.z).max(1e-4);
                cands.push((pi, tri, toward < 0.0, area, -g.z));
            }
        }
        let total: f32 = cands.iter().map(|c| c.3).sum();
        for _ in 0..new.min(8) {
            if cands.is_empty() || total <= 0.0 {
                break;
            }
            let mut r = rnd(&mut st.seed) * total;
            let Some(&(pi, tri, flip, _, dist)) = cands.iter().find(|c| {
                r -= c.3;
                r <= 0.0
            }) else {
                continue;
            };
            let (mut u, mut v) = (rnd(&mut st.seed), rnd(&mut st.seed));
            if u + v > 1.0 {
                (u, v) = (1.0 - u, 1.0 - v);
            }
            // A few millimetres to a centimetre and a half at arm's length, and not much past the triangle's own
            // size (a spot hanging off an edge floats).
            let span = h.shown.get(pi).map_or(1.0, |v| {
                let (a, b, c) = (Vec3::from(v[tri[0]]), Vec3::from(v[tri[1]]), Vec3::from(v[tri[2]]));
                a.distance(b).min(b.distance(c)).min(c.distance(a))
            });
            let size = (dist * (0.008 + rnd(&mut st.seed) * 0.02)).min(span * 1.5).max(dist * 0.005);
            let pick = (rnd(&mut st.seed) * st.mats.len() as f32) as usize % st.mats.len();
            let mat = st.mats[pick].clone();
            let e = commands
                .spawn((
                    Mesh3d(q.clone()),
                    MeshMaterial3d(mat),
                    Transform::from_scale(Vec3::splat(0.0001)),
                    RenderLayers::layer(VIEW_LAYER),
                    bevy::light::NotShadowCaster,
                    ChildOf(root),
                ))
                .id();
            let spin = rnd(&mut st.seed) * std::f32::consts::TAU;
            if env_on!("DXVIEW_VBLOG") {
                println!("VIEWBLOOD part {pi} tri {tri:?} size {size:.3} of {} candidates (ext {ext:.1})", cands.len());
            }
            st.spots.push(Spot { e, part: pi, tri, bary: (u, v), flip, spin, size, age: 0.0 });
        }
        while st.spots.len() > CAP {
            let s = st.spots.remove(0);
            commands.entity(s.e).despawn();
        }
    }
    // Follow the posed model; dry off.
    if env_on!("DXVIEW_VBLOG") && !st.spots.is_empty() && (time.elapsed_secs() * 2.0) as u32 != ((time.elapsed_secs() - dt) * 2.0) as u32 {
        let s0 = &st.spots[0];
        println!("VIEWBLOOD {} spots, first {:?} exists {}", st.spots.len(), tf.get(s0.e).map(|t| (t.translation, t.scale.x)).ok(), tf.get(s0.e).is_ok());
        let r = root.single().ok();
        println!(
            "VIEWBLOOD spot global {:?} root global {:?}",
            gts.get(s0.e).map(|g| (g.0.translation(), g.1.map(|v| v.get()), g.2.map(|v| v.get()))).ok(),
            r.and_then(|r| gts.get(r).ok()).map(|g| (g.0.translation(), g.0.rotation()))
        );
    }
    let mut gone = Vec::new();
    for (i, s) in st.spots.iter_mut().enumerate() {
        s.age += dt;
        if s.age > LIFE {
            gone.push(i);
            continue;
        }
        let Some((p, n)) = h.shown.get(s.part).and_then(|v| tri_at(v, s.tri, s.bary, s.flip)) else { continue };
        if let Ok(mut t) = tf.get_mut(s.e) {
            // Just off the face (the view model's units are metres: a millimetre and a half).
            let p = p + n * 0.0015;
            let rot = Quat::from_rotation_arc(Vec3::Z, n) * Quat::from_rotation_z(s.spin);
            // Shrinking a little as it dries in the last tenth of its life.
            let k = (1.0 - ((s.age - LIFE * 0.9) / (LIFE * 0.1)).max(0.0)).max(0.0);
            let nt = Transform::from_translation(p).with_rotation(rot).with_scale(Vec3::splat(s.size * k.max(0.001)));
            if *t != nt {
                *t = nt;
            }
        }
    }
    for i in gone.into_iter().rev() {
        let s = st.spots.remove(i);
        commands.entity(s.e).despawn();
    }
}
