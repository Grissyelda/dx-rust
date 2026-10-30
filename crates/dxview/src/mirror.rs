//! Mirrored surfaces (BSP PF_Mirrored): Unreal draws the scene reflected in the surface's plane, then the surface's
//! own (translucent) texture over it - the polished marble floors of the intro and the training course, some
//! windows. Here a second camera renders the reflected view into an image each frame (the mirror of the main
//! camera across the nearest mirror plane in front of it, with an oblique near plane on the mirror so nothing
//! behind it shows), and the mirror faces sample that image at their screen position and add their own lit
//! texture. Up to SLOTS planes reflect at once (the nearest ones within 40 m, each with its own camera pair and
//! image); farther mirror faces show just their texture. Each reflection has the sky too: a second camera renders
//! the sky room with the reflected view's rotation into the same image first.

use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::camera::visibility::RenderLayers;
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::shader::ShaderRef;

/// Render layer of mirror faces: seen by the main camera only.
pub const LAYER: usize = 5;
/// Render layer of JC's own body: seen by the reflection camera only (first person has no body to see).
pub const BODY_LAYER: usize = 6;

pub type MirrorMaterial = ExtendedMaterial<StandardMaterial, MirrorExt>;

pub const MIRROR_SHADER: Handle<Shader> = bevy::asset::uuid_handle!("3b9d1e52-7c4a-4f0e-a1d6-58c2e9f7b013");

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct MirrorExt {
    #[texture(100)]
    #[sampler(101)]
    pub reflection: Handle<Image>,
    /// x: 1 = this face's plane is the one reflecting now; y: how much of the face's own texture is added.
    #[uniform(102)]
    pub params: Vec4,
}

impl MaterialExtension for MirrorExt {
    fn fragment_shader() -> ShaderRef {
        MIRROR_SHADER.into()
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
}

pub fn setup_mirror_shader(mut shaders: ResMut<Assets<Shader>>) {
    let _ = shaders.insert(
        &MIRROR_SHADER,
        Shader::from_wgsl(include_str!("shaders/mirror.wgsl"), "dxview/mirror.wgsl"),
    );
}

/// One mirror plane: Bevy-space unit normal and distance, the bounds of its faces, its materials.
pub struct Plane {
    pub n: Vec3,
    pub d: f32,
    pub min: Vec3,
    pub max: Vec3,
    pub mats: Vec<Handle<MirrorMaterial>>,
    /// The mirrored faces themselves (Bevy space).
    pub tris: Vec<[Vec3; 3]>,
    /// A water surface (reflects only with Modern Water on), and its water materials.
    pub water: bool,
    pub water_mats: Vec<Handle<crate::water::WaterMaterial>>,
}

impl Mirrors {
    /// Is this Bevy-space point on a mirrored face (within 2 cm of its plane)?
    pub fn on_mirror(&self, p: Vec3) -> bool {
        self.planes.iter().any(|pl| {
            (pl.n.dot(p) - pl.d).abs() < 0.02
                && p.cmpge(pl.min - Vec3::splat(0.02)).all()
                && p.cmple(pl.max + Vec3::splat(0.02)).all()
                && pl.tris.iter().any(|t| {
                    // Inside the triangle: the point is on the same side of all three edges.
                    let c0 = (t[1] - t[0]).cross(p - t[0]).dot(pl.n);
                    let c1 = (t[2] - t[1]).cross(p - t[1]).dot(pl.n);
                    let c2 = (t[0] - t[2]).cross(p - t[2]).dot(pl.n);
                    (c0 >= -1e-5 && c1 >= -1e-5 && c2 >= -1e-5) || (c0 <= 1e-5 && c1 <= 1e-5 && c2 <= 1e-5)
                })
        })
    }
}

/// How many mirror planes can reflect at the same time.
pub const SLOTS: usize = 3;

/// How far past the mirror plane the reflection camera's oblique near plane sits (metres; about one unit).
const MIRROR_CLIP_EPS: f32 = 0.02;

/// A reflection in progress: its image (the window's size) and the plane it shows.
#[derive(Default)]
pub struct Slot {
    pub image: Option<Handle<Image>>,
    pub size: UVec2,
    pub plane: Option<usize>,
}

#[derive(Resource, Default)]
pub struct Mirrors {
    pub planes: Vec<Plane>,
    /// The placeholder image materials start with.
    pub image: Option<Handle<Image>>,
    pub slots: Vec<Slot>,
    /// Some plane is reflecting (JC's body is drawn for the reflections then).
    pub active: Option<usize>,
}

/// A reflection camera (world, or the sky room with `sky`) of slot `0`.
#[derive(Component)]
pub struct MirrorCam(pub usize, pub bool);

/// The render target the reflection is drawn into (the window's size).
pub fn make_image(images: &mut Assets<Image>, size: UVec2) -> Handle<Image> {
    let mut image = Image::new_uninit(
        Extent3d {
            width: size.x.max(1),
            height: size.y.max(1),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Bgra8UnormSrgb,
        bevy::asset::RenderAssetUsages::MAIN_WORLD | bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage |=
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::RENDER_ATTACHMENT;
    images.add(image)
}

/// The mirror material for a BSP batch on a mirror plane (called while the level's batches are spawned).
pub fn material(
    mirrors: &mut Mirrors,
    images: &mut Assets<Image>,
    mats: &mut Assets<MirrorMaterial>,
    base: StandardMaterial,
    plane: [f32; 4],
    positions: &[[f32; 3]],
    indices: &[u32],
) -> Handle<MirrorMaterial> {
    let image = mirrors
        .image
        .get_or_insert_with(|| make_image(images, UVec2::new(16, 16)))
        .clone();
    let translucent = !matches!(base.alpha_mode, AlphaMode::Opaque | AlphaMode::Mask(_));
    let h = mats.add(MirrorMaterial {
        base: StandardMaterial {
            alpha_mode: AlphaMode::Opaque,
            // Mirrored faces can lie over opaque floor faces in the same plane (the intro's globe-room walkway):
            // Unreal draws the mirror's view over what's there; here they z-fought in shifting stripes as the camera
            // moved. A small bias puts the mirror in front.
            depth_bias: 64.0,
            ..base
        },
        extension: MirrorExt {
            reflection: image,
            params: Vec4::new(
                0.0,
                if translucent && std::env::var("DXVIEW_MIRROR_ONLY").is_err() { 1.0 } else { 0.0 },
                0.0,
                0.0,
            ),
        },
    });
    let n = Vec3::new(plane[0], plane[1], plane[2]);
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for p in positions {
        lo = lo.min(Vec3::from(*p));
        hi = hi.max(Vec3::from(*p));
    }
    let tris: Vec<[Vec3; 3]> = indices
        .chunks_exact(3)
        .filter_map(|t| {
            Some([
                Vec3::from(*positions.get(t[0] as usize)?),
                Vec3::from(*positions.get(t[1] as usize)?),
                Vec3::from(*positions.get(t[2] as usize)?),
            ])
        })
        .collect();
    let same = mirrors
        .planes
        .iter_mut()
        .find(|p| p.n.dot(n) > 0.999 && (p.d - plane[3]).abs() < 0.02);
    match same {
        Some(p) => {
            p.min = p.min.min(lo);
            p.max = p.max.max(hi);
            p.mats.push(h.clone());
            p.tris.extend(tris);
        }
        None => mirrors.planes.push(Plane {
            n,
            d: plane[3],
            min: lo,
            max: hi,
            mats: vec![h.clone()],
            tris,
            water: false,
            water_mats: Vec::new(),
        }),
    }
    h
}

/// The placeholder reflection image (made once).
pub fn placeholder(mirrors: &mut Mirrors, images: &mut Assets<Image>) -> Handle<Image> {
    mirrors.image.get_or_insert_with(|| make_image(images, UVec2::new(16, 16))).clone()
}

/// A water batch (horizontal faces): its plane reflects like a mirror while Modern Water is on (water.rs). Not
/// counted as a mirror face (`on_mirror`: blob shadows still fall on water).
pub fn add_water(mirrors: &mut Mirrors, h: &Handle<crate::water::WaterMaterial>, positions: &[[f32; 3]]) {
    if positions.is_empty() {
        return;
    }
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for p in positions {
        lo = lo.min(Vec3::from(*p));
        hi = hi.max(Vec3::from(*p));
    }
    // Only flat sheets (a water face that isn't level - a waterfall - doesn't reflect).
    if hi.y - lo.y > 0.05 {
        return;
    }
    let d = (lo.y + hi.y) * 0.5;
    match mirrors.planes.iter_mut().find(|p| p.water && (p.d - d).abs() < 0.03) {
        Some(p) => {
            p.min = p.min.min(lo);
            p.max = p.max.max(hi);
            p.water_mats.push(h.clone());
        }
        None => mirrors.planes.push(Plane {
            n: Vec3::Y,
            d,
            min: lo,
            max: hi,
            mats: Vec::new(),
            tris: Vec::new(),
            water: true,
            water_mats: vec![h.clone()],
        }),
    }
}

/// Spawns this map's reflection cameras (once its mirrors are known): a sky-room and a world camera per slot.
pub fn spawn_camera(commands: &mut Commands, mirrors: &Mirrors) {
    let Some(image) = mirrors.image.clone() else { return };
    for i in 0..SLOTS {
        for sky in [true, false] {
            let mut e = commands.spawn((
                Camera3d::default(),
                Camera {
                    order: -20 + 2 * i as isize + if sky { 0 } else { 1 },
                    is_active: false,
                    clear_color: if sky { ClearColorConfig::Custom(Color::BLACK) } else { ClearColorConfig::None },
                    ..default()
                },
                RenderTarget::Image(image.clone().into()),
                Projection::Perspective(PerspectiveProjection::default()),
                Transform::default(),
                Tonemapping::None,
                bevy::camera::Exposure { ev100: -0.263 },
                MirrorCam(i, sky),
            ));
            if sky {
                e.insert((RenderLayers::layer(1), bevy::light::cluster::ClusterConfig::Single));
            } else {
                e.insert(RenderLayers::from_layers(&[0, BODY_LAYER]));
            }
        }
    }
}

/// Each frame: pick the planes to reflect, place their cameras, keep their images the window's size and tell the
/// materials which plane shows which image.
#[allow(clippy::type_complexity)]
pub fn mirror_system(
    mut mirrors: ResMut<Mirrors>,
    windows: Query<&Window>,
    mut images: ResMut<Assets<Image>>,
    mut mats: ResMut<Assets<MirrorMaterial>>,
    main: Query<(&Transform, &Projection), (With<crate::FlyCam>, Without<crate::SkyCam>, Without<MirrorCam>)>,
    sky_main: Query<&Projection, (With<crate::SkyCam>, Without<MirrorCam>)>,
    mut cams: Query<(&MirrorCam, &mut Camera, &mut Transform, &mut Projection, &mut RenderTarget)>,
    mut water_mats: ResMut<Assets<crate::water::WaterMaterial>>,
) {
    if mirrors.planes.is_empty() {
        return;
    }
    if crate::credits::open() {
        for (_, mut cam, ..) in &mut cams {
            cam.is_active = false;
        }
        return;
    }
    let Some((main_tf, Projection::Perspective(main_proj))) = main.iter().next() else { return };
    let sky_proj = match sky_main.iter().next() {
        Some(Projection::Perspective(p)) => p.clone(),
        _ => main_proj.clone(),
    };
    if mirrors.slots.len() < SLOTS {
        mirrors.slots.resize_with(SLOTS, Slot::default);
    }
    // This frame's camera (its Transform: the GlobalTransform is last frame's until PostUpdate, and a reflection a
    // frame behind slides about on the floor when the camera moves fast - the intro's opening pan).
    let eye = main_tf.translation;
    // The nearest planes the camera isn't edge-on to, close enough to matter (within 40 m of their faces).
    // Only planes the view can see: some corner of the faces' bounds in front of the camera, inside a cone a bit
    // wider than the field of view (each reflection is a whole extra render).
    let fwd = main_tf.forward().as_vec3();
    let half = main_proj.fov * 0.5 * main_proj.aspect_ratio.max(1.0) + 0.35;
    let cone = half.min(3.0).cos();
    let visible = |p: &Plane| {
        // Water goes on past its sheet (the open sea shares its material): reflect it whenever the eye is above it,
        // or the reflection pops in and out with the view's pitch.
        if p.water {
            return eye.y > p.d;
        }
        let (lo, hi) = (p.min - Vec3::splat(0.5), p.max + Vec3::splat(0.5));
        let inside = eye.cmpge(lo).all() && eye.cmple(hi).all();
        // Looking straight at a big surface (down at a mirror floor) can leave every corner of its bounds outside the
        // cone: so also when the view ray goes through the bounds, or their nearest point is in the cone.
        let ray_hits = {
            let inv = fwd.recip();
            let t0 = (lo - eye) * inv;
            let t1 = (hi - eye) * inv;
            let tmin = t0.min(t1).max_element();
            let tmax = t0.max(t1).min_element();
            tmax >= tmin.max(0.0)
        };
        let nearest_in = (eye.clamp(p.min, p.max) - eye).normalize_or_zero().dot(fwd) > cone;
        inside
            || ray_hits
            || nearest_in
            || (0..8).any(|i| {
                let c = Vec3::new(
                    if i & 1 == 0 { p.min.x } else { p.max.x },
                    if i & 2 == 0 { p.min.y } else { p.max.y },
                    if i & 4 == 0 { p.min.z } else { p.max.z },
                );
                let d = (c - eye).normalize_or_zero();
                d.dot(fwd) > cone
            })
    };
    let mut near: Vec<(usize, f32)> = mirrors
        .planes
        .iter()
        .enumerate()
        .filter(|(_, p)| (!p.water || crate::water::modern()) && (p.n.dot(eye) - p.d).abs() >= 0.01 && visible(p))
        .map(|(i, p)| (i, eye.clamp(p.min, p.max).distance(eye)))
        .filter(|(_, d)| *d <= 40.0)
        .collect();
    near.sort_by(|a, b| a.1.total_cmp(&b.1));
    // One water plane reflects at a time (the nearest): each slot is two more renders of the scene.
    let mut seen_water = false;
    near.retain(|(i, _)| {
        if !mirrors.planes[*i].water {
            return true;
        }
        let keep = !seen_water;
        seen_water = true;
        keep
    });
    // DXVIEW_MIRROR_SLOTS=n: at most n reflections (performance comparisons).
    let max = std::env::var("DXVIEW_MIRROR_SLOTS").ok().and_then(|v| v.parse().ok()).unwrap_or(SLOTS).min(SLOTS);
    near.truncate(max);
    let want: Vec<usize> = near.iter().map(|x| x.0).collect();
    // Keep planes in the slot they had; free the rest; give the newcomers free slots.
    let mut changed: Vec<usize> = Vec::new();
    for s in mirrors.slots.iter_mut() {
        if let Some(p) = s.plane {
            if !want.contains(&p) {
                s.plane = None;
                changed.push(p);
            }
        }
    }
    for p in &want {
        if mirrors.slots.iter().any(|s| s.plane == Some(*p)) {
            continue;
        }
        if let Some(s) = mirrors.slots.iter_mut().find(|s| s.plane.is_none()) {
            s.plane = Some(*p);
            changed.push(*p);
        }
    }
    mirrors.active = want.first().copied();
    // Images the window's size.
    if let Some(w) = windows.iter().next() {
        let full = UVec2::new(w.physical_width().max(1), w.physical_height().max(1));
        let nearest = want.first().copied();
        for k in 0..SLOTS {
            // The nearest reflection at full resolution, the others at half (they're farther and smaller on screen).
            // Water's ripples hide resolution: always half.
            let water = mirrors.slots[k].plane.is_some_and(|p| mirrors.planes[p].water);
            let size = if mirrors.slots[k].plane.is_some() && mirrors.slots[k].plane == nearest && !water { full } else { (full / 2).max(UVec2::ONE) };
            if mirrors.slots[k].plane.is_some() && mirrors.slots[k].size != size {
                let img = make_image(&mut images, size);
                if let Some(old) = mirrors.slots[k].image.replace(img) {
                    images.remove(old.id());
                }
                mirrors.slots[k].size = size;
                if let Some(p) = mirrors.slots[k].plane {
                    if !changed.contains(&p) {
                        changed.push(p);
                    }
                }
            }
        }
    }
    // Materials of planes that changed slot (or lost / got one).
    for pi in changed {
        let slot = mirrors.slots.iter().find(|s| s.plane == Some(pi));
        let img = slot.and_then(|s| s.image.clone());
        for h in &mirrors.planes[pi].mats {
            if let Some(mut m) = mats.get_mut(h) {
                match &img {
                    Some(i) => {
                        m.extension.reflection = i.clone();
                        m.extension.params.x = 1.0;
                    }
                    None => m.extension.params.x = 0.0,
                }
            }
        }
        for h in &mirrors.planes[pi].water_mats {
            if let Some(mut m) = water_mats.get_mut(h) {
                match &img {
                    Some(i) => {
                        m.extension.reflection = i.clone();
                        m.extension.params.m.y = 1.0;
                    }
                    None => m.extension.params.m.y = 0.0,
                }
            }
        }
    }
    for (mc, mut cam, mut tf, mut proj, mut target) in &mut cams {
        let slot = &mirrors.slots[mc.0.min(SLOTS - 1)];
        let (Some(pi), Some(img)) = (slot.plane, slot.image.clone()) else {
            cam.is_active = false;
            continue;
        };
        cam.is_active = true;
        if !matches!(&*target, RenderTarget::Image(t) if t.handle == img) {
            *target = img.into();
        }
        let p = &mirrors.planes[pi];
        // Flip the plane so the camera is on its positive side.
        let (n, d) = if p.n.dot(eye) - p.d >= 0.0 { (p.n, p.d) } else { (-p.n, -p.d) };
        // The reflected camera: the eye mirrored across n.x = d, looking along the mirrored forward with the
        // mirrored up. A rotation can't mirror, so this view is the reflection flipped left to right; the shader
        // samples it with u flipped. (A negative-scale camera transform breaks Bevy's frustum culling.)
        let mirror_v = |v: Vec3| v - 2.0 * v.dot(n) * n;
        let dist = n.dot(eye) - d;
        let pos = eye - 2.0 * dist * n;
        let fwd = mirror_v(main_tf.forward().as_vec3());
        let up = mirror_v(main_tf.up().as_vec3());
        let look = Transform::from_translation(pos).looking_to(fwd, up);
        if mc.1 {
            // The sky room: same turn, at its own origin.
            *tf = Transform::from_rotation(look.rotation);
            *proj = Projection::Perspective(PerspectiveProjection { fov: main_proj.fov, aspect_ratio: main_proj.aspect_ratio, ..sky_proj.clone() });
            continue;
        }
        *tf = look;
        // Oblique near plane on the mirror: keep only what is on the viewer's side of it (n.x - d >= 0), written in
        // this camera's view space: (view-space n, -distance from the plane).
        let nv = (tf.rotation.inverse() * n).normalize();
        *proj = Projection::Perspective(PerspectiveProjection {
            near_clip_plane: if std::env::var("DXVIEW_MIRROR_NOCLIP").is_ok() {
                main_proj.near_clip_plane
            } else {
                // A unit (2 cm) past the mirror, so faces lying in its plane (the floor around a mirrored patch,
                // the intro's globe-room walkway) are clipped away for good instead of flickering in and out in
                // stripes along the clip plane.
                nv.extend(-(dist + MIRROR_CLIP_EPS))
            },
            ..main_proj.clone()
        });
    }
}

pub fn reset_level() {}

/// JC as the mirrors show him: GM_Trench with his skins (JCDentonMale), standing where the player is, facing the
/// view yaw, playing BreatheLight / Walk / Run / Crouch / CrouchWalk by what the player is doing. Only the
/// reflection camera draws it (BODY_LAYER), and only while a mirror is live.
pub struct MirrorBody {
    root: Entity,
    mesh: std::sync::Arc<dx_level::actors::AnimMesh>,
    parts: Vec<dx_level::actors::MeshPart>,
    handles: Vec<Handle<Mesh>>,
    seq: String,
    t: f32,
    lit_at: f32,
    /// The weapon in JC's hand (its ThirdPersonMesh on the hand's attachment triangle): class, root, scale.
    held: Option<(String, Entity, f32, Vec<dx_level::actors::MeshPart>, Vec<Handle<Mesh>>)>,
    /// augmentique::generation() when built (a new outfit rebuilds him).
    gen: u64,
}

#[derive(Resource, Default)]
pub struct MirrorBodyRes(pub Option<MirrorBody>);

/// Third person, left alone (thirdperson::idle_secs): after IDLE_AFTER seconds JC plays Human's idle fidget
/// (Idle1 / Idle12H, the one ScriptedPawn.PlayIdle uses) once, breathes for a while, and again.
const IDLE_AFTER: f32 = 5.0;
const IDLE_GAP: f32 = 6.0;

fn idle_fidget(mesh: &dx_level::actors::AnimMesh, two: bool) -> Option<(&'static str, bool, f32)> {
    let idle = crate::thirdperson::idle_secs();
    if idle < IDLE_AFTER {
        return None;
    }
    let name: &'static str = if two { "Idle12H" } else { "Idle1" };
    let s = mesh.seqs.iter().find(|s| s.name.eq_ignore_ascii_case(name))?;
    let len = s.num_frames.max(1) as f32 / s.rate.max(1.0);
    let phase = (idle - IDLE_AFTER) % (len + IDLE_GAP);
    (phase < len).then_some((name, false, 1.0))
}

#[allow(clippy::too_many_arguments)]
pub fn mirror_body_system(
    mut commands: Commands,
    time: Res<Time>,
    mirrors: Res<Mirrors>,
    level: Res<crate::LevelRes>,
    player: Res<crate::Player>,
    mut body: ResMut<MirrorBodyRes>,
    cams: Query<(&crate::FlyCam, &Transform), Without<crate::SkyCam>>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    (mut images, mut meshes, mut materials): (ResMut<Assets<Image>>, ResMut<Assets<Mesh>>, ResMut<Assets<StandardMaterial>>),
    mut vis: Query<&mut Visibility>,
    weapons: Res<crate::weapons::Weapons>,
    keys: Res<ButtonInput<KeyCode>>,
    ph: Res<crate::ai::PlayerHealth>,
) {
    // Something else shows JC (conversation puppet, death), or no mirror reflecting: hide.
    // DXVIEW_MIRROR_BODY=1: show him under a flying camera too (screenshot tests).
    let test = std::env::var("DXVIEW_MIRROR_BODY").is_ok();
    // Augmentique's outfits screen shows him too (outfit_view.rs).
    let outfits = crate::outfit_view::open();
    // Third person (thirdperson.rs): the main camera draws him too.
    let show = (outfits || crate::thirdperson::active() || mirrors.active.is_some() && (test || !player.noclip)) && !crate::consys::third_person() && !crate::cinematic::active() && !ph.dead;
    if let Some(b) = body.0.as_ref() {
        if b.gen != crate::augmentique::generation() {
            commands.entity(b.root).despawn();
            body.0 = None;
        }
    }
    if !show {
        if let Some(b) = body.0.as_ref() {
            if let Ok(mut v) = vis.get_mut(b.root) {
                *v = Visibility::Hidden;
            }
        }
        return;
    }
    let lvl = &level.0;
    if body.0.is_none() {
        let d = crate::travel::jc_defaults(&mut assets.classes);
        let root = commands.spawn((Transform::default(), Visibility::default())).id();
        let map = |p: [f32; 3]| dx_level::ue_to_bevy(p);
        match crate::weapons::spawn_mesh_parts(
            &crate::travel::jc_mesh(),
            1.0,
            &d,
            &mut assets,
            &mut images,
            &mut meshes,
            &mut materials,
            &mut commands,
            root,
            &map,
            Some(RenderLayers::layer(BODY_LAYER)),
        ) {
            Some((lm, parts, handles)) => {
                body.0 = Some(MirrorBody {
                    root,
                    mesh: std::sync::Arc::new({
                        let mut am = dx_level::actors::anim_mesh(&lm);
                        dx_level::actors::synth_crouch_walk(&mut am);
                        am
                    }),
                    parts,
                    handles,
                    seq: String::new(),
                    t: 0.0,
                    lit_at: -1.0,
                    held: None,
                    gen: crate::augmentique::generation(),
                });
            }
            None => {
                if std::env::var("DXVIEW_BODYLOG").is_ok() {
                    println!("mirror body: no mesh {}", crate::travel::jc_mesh());
                }
                commands.entity(root).despawn();
                return;
            }
        }
        if std::env::var("DXVIEW_BODYLOG").is_ok() {
            println!("mirror body: built {}", crate::travel::jc_mesh());
        }
    }
    let Some(b) = body.0.as_mut() else { return };
    if let Ok(mut v) = vis.get_mut(b.root) {
        *v = Visibility::Inherited;
    }
    let yaw = cams.iter().next().map_or(0.0, |c| c.0.yaw);
    // FlyCam yaw 0 looks along Bevy -Z (Unreal -Y): JC's Unreal yaw is the view's.
    let mut ue_yaw = -(yaw as f64) - std::f64::consts::FRAC_PI_2;
    // DXVIEW_BODY_AHEAD="units,degrees": (test shots) the body that far in front of the fly camera, turned by that.
    let ahead = std::env::var("DXVIEW_BODY_AHEAD").ok().and_then(|v| {
        let (a, b) = v.split_once(',')?;
        Some((a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?))
    });
    let look_yaw = ue_yaw;
    if let Some((_, deg)) = ahead {
        ue_yaw += deg.to_radians();
    }
    let rot = [0, (ue_yaw / std::f64::consts::TAU * 65536.0) as i32, 0];
    let m = dx_level::actors::rotator_to_bevy_mat3(rot);
    let quat = Quat::from_mat3(&Mat3::from_cols(Vec3::from(m[0]), Vec3::from(m[1]), Vec3::from(m[2])));
    // The mesh is built around the standing cylinder's centre: keep the feet where the (crouched) body's are.
    let mut loc = player.body.location;
    loc.z += player.stand_height - player.params.height;
    if test && player.noclip {
        if let Some((_, tf)) = cams.iter().next() {
            let e = dx_level::bevy_to_ue(tf.translation.to_array());
            loc = glam::DVec3::new(e[0] as f64, e[1] as f64, e[2] as f64 - player.eye_height);
            if let Some((d, _)) = ahead {
                loc += glam::DVec3::new(look_yaw.cos(), look_yaw.sin(), 0.0) * d;
            }
        }
    }
    commands
        .entity(b.root)
        .insert(Transform::from_translation(Vec3::from(dx_level::ue_to_bevy(loc.as_vec3().to_array()))).with_rotation(quat));
    let speed = player.body.velocity.truncate().length();
    let moving = speed > 10.0;
    let running = speed > player.params.ground_speed * 0.6;
    // The weapon in hand decides the pose (the way ScriptedPawn picks its *2H / Shoot sets): a two-handed one
    // (Mass >= 30) uses the 2H set; a ranged one is held up aimed; firing plays Shoot (melee: Attack); reloading
    // plays the reload; in the air Jump.
    let held = weapons.held.as_ref().filter(|h| !h.stats.item && h.stats.class != "POVCorpse");
    let two = held.is_some_and(|h| h.stats.mass >= 30.0);
    let ranged = held.is_some_and(|h| !h.stats.hand_to_hand);
    let state = held.map(|h| h.state);
    let firing = state == Some(crate::weapons::WState::Firing);
    let reloading = state == Some(crate::weapons::WState::Reloading);
    let falling = player.body.physics == imsim_core::movement::Physics::Falling && !player.swimming;
    // Holding the gun up to aim (the scope key in third person, or Modern Aiming's sights): the aimed poses even on the
    // move. Otherwise Human.uc's choices: Walk / Run (RunShoot only while firing; two-handed guns run with
    // RunShoot2H), CrouchWalk / CrouchShoot, BreatheLight between shots.
    let aiming = ranged && !reloading && (keys.pressed(KeyCode::KeyZ) || keys.pressed(KeyCode::BracketLeft) || crate::aimmode::ads() > 0.3);
    let shooting = firing || aiming;
    // (sequence, loops: else held on its last frame / frame 0 for aim poses, rate x)
    let (want, looped, rate_k): (&str, bool, f32) = if player.swimming {
        (if firing { "TreadShoot" } else { "Tread" }, true, 1.0)
    } else if falling {
        ("Jump", false, 3.0)
    } else if player.crouched {
        if firing && ranged {
            // PlayCrawling while firing: CrouchShoot, once per shot.
            ("CrouchShoot", true, 1.0)
        } else if aiming {
            // Aiming crouched (moving or not): the gun held up - CrouchShoot's first frame.
            ("CrouchShoot", false, 1.0)
        } else if moving {
            // PlayCrawling: LoopAnim('CrouchWalk'), its legs in step with the ground covered. Meshes without it
            // (the women's GFM_Trench: Lay D Denton) get one made up (dx_level::actors::synth_crouch_walk).
            ("CrouchWalk", true, (speed as f32 / (player.params.ground_speed as f32 * 0.3).max(1.0)).clamp(0.6, 2.0))
        } else {
            ("Crouch", false, 1.0)
        }
    } else if reloading && ranged && !moving {
        ("Reload", true, 1.0)
    } else if firing && !ranged {
        ("Attack", true, 1.0)
    } else if moving && shooting && ranged && {
        // Aiming while moving sideways to where he faces (the free third-person camera lets him): Human's
        // Strafe / Strafe2H.
        let (c, s) = (ue_yaw.cos(), ue_yaw.sin());
        let v = player.body.velocity;
        let fwd = v.x * c + v.y * s;
        let side = -v.x * s + v.y * c;
        side.abs() > fwd.abs() * 1.2
    } {
        let pace = (speed as f32 / player.params.ground_speed as f32).clamp(0.35, 1.3);
        (if two { "Strafe2H" } else { "Strafe" }, true, pace)
    } else if moving {
        // Legs keep pace with the ground (Run's cycle covers about the full run speed).
        let pace = (speed as f32 / player.params.ground_speed as f32).clamp(0.35, 1.3);
        match (shooting && ranged, two, running) {
            // Aiming or firing on the move: the aimed run, slowed to a walk's pace when walking.
            (true, true, _) => ("RunShoot2H", true, pace),
            (true, false, _) => ("RunShoot", true, pace),
            (false, true, true) => ("RunShoot2H", true, 1.0),
            (false, true, false) => ("Walk2H", true, 1.0),
            (false, false, true) => ("Run", true, 1.0),
            (false, false, false) => ("Walk", true, 1.0),
        }
    } else if ranged && shooting {
        // Firing: the shot (once per trigger pull, its own rate); aiming between shots: the aim pose (Shoot frame 0).
        (if two { "Shoot2H" } else { "Shoot" }, firing, 1.0)
    } else if let Some(idle) = idle_fidget(&b.mesh, two) {
        idle
    } else if two {
        ("BreatheLight2H", true, 1.0)
    } else {
        ("BreatheLight", true, 1.0)
    };
    // DXVIEW_BODY_SEQ=<sequence>: loop that one (screenshots of the third-person body).
    let forced = std::env::var("DXVIEW_BODY_SEQ").ok();
    let (want, looped, rate_k) = match forced.as_deref() {
        Some(f) => (f, true, 1.0),
        None => (want, looped, rate_k),
    };
    // Aim pose: frame 0 of Shoot / CrouchShoot held while aiming and not firing.
    let aim_pose = aiming && !firing && (!moving || player.crouched) && !falling && !player.swimming;
    if std::env::var_os("DXVIEW_BODYLOG").is_some() {
        static T: std::sync::Mutex<f32> = std::sync::Mutex::new(0.0);
        if let Ok(mut t) = T.lock() {
            *t += time.delta_secs();
            if *t > 0.5 {
                *t = 0.0;
                println!("BODY {want} speed {speed:.0} crouched {} aiming {aiming} firing {firing} pose {aim_pose}", player.crouched);
            }
        }
    }
    let key = format!("{want}{}", if aim_pose { "#aim" } else { "" });
    if b.seq != key {
        b.seq = key;
        b.t = 0.0;
    }
    let dt = time.delta_secs();
    b.t += dt;
    let (start, frames, rate) = b
        .mesh
        .seqs
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(want))
        .or_else(|| b.mesh.seqs.iter().find(|s| s.name.eq_ignore_ascii_case("BreatheLight")))
        .map(|s| (s.start_frame.max(0) as usize, s.num_frames.max(1) as usize, s.rate))
        .unwrap_or((0, 1, 10.0));
    let rate = rate * rate_k;
    let f = if aim_pose {
        0.0
    } else if looped {
        (b.t * rate.max(1.0)) % frames.max(1) as f32
    } else {
        (b.t * rate.max(1.0)).min(frames.saturating_sub(1) as f32)
    };
    let relight = time.elapsed_secs() - b.lit_at > 0.5;
    if relight {
        b.lit_at = time.elapsed_secs();
    }
    // The weapon in hand (a thrown or put-away one leaves the hand empty).
    // Keyed by class and (Visible Attachments) what's fitted, so a new mod shows in the reflection too.
    let want_w = weapons.held.as_ref().and_then(|h| {
        let fitted = h.fitted();
        h.stats.third_mesh.clone().map(|m| (format!("{}#{:?}#{}", h.stats.class, h.vis.as_ref().map(|_| fitted), crate::augmentique::player_skin(&h.stats.class)), m, h.stats.third_scale, h.stats.class.clone(), fitted))
    });
    if b.held.as_ref().map(|h| &h.0) != want_w.as_ref().map(|w| &w.0) {
        if let Some((_, e, ..)) = b.held.take() {
            commands.entity(e).despawn();
        }
        if let Some((key, mesh, scale, class, fitted)) = want_w {
            let mut d = assets.classes.defaults(&class);
            let st = crate::hdtp::WeaponState { scope: fitted.scope, silencer: fitted.silencer, laser: fitted.laser, lasing: fitted.lasing, ammo: Vec::new(), loaded: true };
            if let Some(skins) = crate::hdtp::world_skins(&class, &st) {
                crate::visattach::apply_skins(&mut d, &skins);
            } else if let Some(skins) = crate::visattach::skins_for(&mesh, fitted) {
                d.retain(|p| !p.name.eq_ignore_ascii_case("MultiSkins"));
                crate::visattach::apply_skins(&mut d, &skins);
            }
            // Augmentique: his weapon skin (fills the slots still empty).
            crate::augmentique::apply_weapon_skin(&mut d, &class, &crate::augmentique::player_skin(&class), false);
            let class = key;
            let root = commands.spawn((Transform::default(), Visibility::Inherited)).id();
            commands.entity(b.root).add_child(root);
            let map = |p: [f32; 3]| dx_level::ue_to_bevy(p);
            match crate::weapons::spawn_mesh_parts(&mesh, 1.0, &d, &mut assets, &mut images, &mut meshes, &mut materials, &mut commands, root, &map, Some(RenderLayers::layer(BODY_LAYER))) {
                Some((_m, parts, handles)) => b.held = Some((class, root, scale, parts, handles)),
                None => commands.entity(root).despawn(),
            }
        }
    }
    if let Some((_, root, scale, parts, handles)) = b.held.as_ref() {
        if let Some([o, x, y, z]) = b.mesh.weapon_coords(start, frames, f) {
            // Body-local Unreal: p' = O + [X Y Z] x ThirdPersonScale x p, then into Bevy axes.
            let sw = Mat3::from_cols(Vec3::X, Vec3::Z, Vec3::Y);
            let lin = sw * (Mat3::from_cols(Vec3::from(x), Vec3::from(y), Vec3::from(z)) * *scale) * sw;
            let t = Vec3::from(dx_level::ue_to_bevy(o));
            commands.entity(*root).insert(Transform::from_matrix(Mat4::from_cols(
                lin.x_axis.extend(0.0),
                lin.y_axis.extend(0.0),
                lin.z_axis.extend(0.0),
                t.extend(1.0),
            )));
            if relight {
                for (pi, h) in handles.iter().enumerate() {
                    if let Some(mut mm) = meshes.get_mut(h) {
                        mm.insert_attribute(Mesh::ATTRIBUTE_COLOR, dx_level::actors::light_part(&parts[pi], loc.as_vec3().to_array(), [0, 0, 0], 1.0, &lvl.lights));
                    }
                }
            }
        }
    }
    let at = loc.as_vec3().to_array();
    for (pi, h) in b.handles.iter().enumerate() {
        if let Some(mut mm) = meshes.get_mut(h) {
            let pos: Vec<[f32; 3]> = b.parts[pi]
                .vert_ids
                .iter()
                .map(|&v| dx_level::ue_to_bevy(b.mesh.sample(start, frames, f, v as usize)))
                .collect();
            mm.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
            if relight {
                mm.insert_attribute(
                    Mesh::ATTRIBUTE_COLOR,
                    dx_level::actors::light_part(&b.parts[pi], at, rot, 1.0, &lvl.lights),
                );
            }
        }
    }
}

