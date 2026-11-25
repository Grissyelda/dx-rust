//! Bodies, in more detail (enhancements on Skeletal Characters):
//! - "Wounds On Bodies": see wounds.rs (drawn on the skin in the body's rest space).
//! - "Hurt Poses": hurt people move like it - a wounded arm hangs, a wounded leg stiffens into a limp, a wounded
//!   trunk hunches (NPCs, and JC's own body in third person / mirrors / first person).
//! - "First-Person Body": JC's body under the view (no head or arms: the view model has the arms), on its own
//!   layer that only the main camera draws.
//! - "Diegetic Healing": with a medkit in hand, look at the part of your body to treat - an arm, a leg or the
//!   chest - and use it there; used looking anywhere else, it treats the head. A label under the crosshair says
//!   which part and how it is.
//! - "Aug Glows": running augmentations glow on JC's body where they sit (eyes, arms, legs, chest).
//! - Healing shows: a soft pulse on the screen and a glow where the medicine went.

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use glam::DVec3;
use std::collections::HashMap;
use std::sync::Mutex;

use crate::skeletal::SkelBody;

pub const FP_LAYER: usize = 11;
/// How far behind the eye JC's first-person body stands (Unreal units): the cut under the collar stays behind and
/// below the view even looking straight down, while the chest, belt, legs and feet show (as Half-Life MMod shows
/// the body). More when crouched (the crouch pose leans the chest forward into the eye).
const FP_BACK: f32 = 12.0;
const FP_BACK_CROUCH: f32 = 18.0;
/// And this far lower (Unreal units; negative = higher): raised so the cut sits close under the eye.
const FP_DOWN: f32 = -3.0;
/// The first-person body keeps what lies below this fraction of its standing height (chest down: just under the
/// collar). Its faces are drawn from the front only, so whatever of the opening is seen shows no inside of the coat.
const FP_KEEP: f32 = 0.82;
/// The bones whose triangles the first-person body leaves out: head, neck and the arms (the view model's arms are
/// the arms).
const FP_DROP: &[&str] = &["DEF-spine.005", "DEF-spine.006", "DEF-upper_arm", "DEF-forearm", "DEF-hand", "DEF-f_", "DEF-thumb"];

/// A bone whose triangles the first-person body leaves out (FP_DROP names are prefixes).
fn fp_dropped(bone: &str) -> bool {
    FP_DROP.iter().any(|p| bone.starts_with(p))
}

fn on(key: &str) -> bool {
    crate::settings::get(key) > 0.5 && crate::skeletal::enabled()
}

static FP_SHOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// JC's first-person body is wanted now (mirror.rs keeps his pose running for it).
pub fn fp_on() -> bool {
    on("FirstPersonBody")
        && !crate::thirdperson::active()
        && !crate::consys::third_person()
        && !crate::cinematic::active()
}

// ------------------------------------------------------------------------------------------------ wounds

/// A shot into pawn `blocker` at `at` (Unreal space) travelling along `dir` (wounds.rs draws it on the skin).
pub fn wound(blocker: usize, at: DVec3, dir: DVec3, kind: &'static str, damage: f32) {
    crate::wounds::hit(blocker, at, dir, kind, damage);
}

// ------------------------------------------------------------------------------------------------ hurt poses

/// The parts' health as fractions of full (head, torso, left arm, right arm, left leg, right leg).
pub fn hurt_fracs(parts: &[f32; 6], full: &[f32; 6]) -> [f32; 6] {
    let mut f = [1.0f32; 6];
    for i in 0..6 {
        f[i] = (parts[i] / full[i].max(1.0)).clamp(0.0, 1.0);
    }
    f
}

pub fn hurt_on() -> bool {
    on("HurtPoses")
}

// ------------------------------------------------------------------------------------------------ first-person body

#[derive(Component)]
pub struct FpBody;

#[derive(Default)]
pub struct FpState {
    root: Option<Entity>,
    gen: u64,
    /// How much further back than FP_BACK the body stands this frame (m): enough to keep the cut chest out of view
    /// whatever the pose (walking and aiming lean it forward), eased back when the pose allows.
    extra: f32,
    /// Frames to wait after the third-person camera before measuring again (its last place lingers a frame).
    skip: u8,
}

/// The bones round the cut (chest, shoulders): kept behind and below the view.
const FP_CHEST: &[&str] = &["DEF-spine.003", "DEF-spine.004", "DEF-shoulder.L", "DEF-shoulder.R", "DEF-breast.L", "DEF-breast.R"];
/// How far the coat's front stands ahead of those bones (Unreal units), and the clearance kept on top.
const FP_CHEST_FRONT: f32 = 8.0;
const FP_CHEST_CLEAR: f32 = 2.0;
/// tan of half the vertical field of view (75 degrees across a 16:9 view): looking straight down, the view reaches
/// this far behind the eye per unit below it.
const FP_VIEW_BACK: f32 = 0.45;

/// Which body part the medkit would treat (aim ray against the first-person body): None = the head.
static AIMED: Mutex<Option<usize>> = Mutex::new(None);

/// Diegetic Healing is on and a medkit is in hand.
pub fn diegetic_heal() -> bool {
    on("DiegeticHealing")
}

/// Bones that make up each part for aiming: (part, bone a, bone b, radius m).
const AIM_PARTS: &[(usize, &str, &str, f32)] = &[
    (crate::ai::P_TORSO, "DEF-spine", "DEF-spine.004", 0.18),
    (crate::ai::P_ARM_L, "DEF-upper_arm.L", "DEF-hand.L", 0.07),
    (crate::ai::P_ARM_R, "DEF-upper_arm.R", "DEF-hand.R", 0.07),
    (crate::ai::P_LEG_L, "DEF-thigh.L", "DEF-foot.L", 0.09),
    (crate::ai::P_LEG_R, "DEF-thigh.R", "DEF-foot.R", 0.09),
];

fn seg_ray(o: Vec3, d: Vec3, a: Vec3, b: Vec3) -> f32 {
    // Closest distance between the ray and segment ab.
    let u = b - a;
    let w = o - a;
    let (aa, bb, cc, dd, ee) = (u.dot(u), u.dot(d), d.dot(d), u.dot(w), d.dot(w));
    let den = (aa * cc - bb * bb).max(1e-8);
    let s = ((bb * ee - cc * dd) / den).clamp(0.0, 1.0);
    let t = ((aa * ee - bb * dd) / den).max(0.0);
    ((a + u * s) - (o + d * t)).length()
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn fp_body_system(
    mut commands: Commands,
    body: Res<crate::mirror::MirrorBodyRes>,
    weapons: Res<crate::weapons::Weapons>,
    ph: Res<crate::ai::PlayerHealth>,
    cam: Query<&GlobalTransform, (With<crate::FlyCam>, Without<crate::SkyCam>)>,
    mut fp: Query<(Entity, &mut SkelBody, &mut Transform), With<FpBody>>,
    mut tfs: Query<&mut Transform, (Without<SkelBody>, Without<FpBody>, Without<FpGlow>)>,
    gts: Query<&GlobalTransform>,
    mut vis: Query<&mut Visibility, With<FpBody>>,
    mut st: Local<FpState>,
    mut built: crate::skeletal::BuildParams,
    time: Res<Time>,
    level: Res<crate::LevelRes>,
    mirror: Query<&SkelBody, (With<crate::skeletal::MirrorSkel>, Without<FpBody>)>,
) {
    let want = fp_on() && !ph.dead;
    FP_SHOWN.store(want, std::sync::atomic::Ordering::Relaxed);
    let Some(b) = body.0.as_ref() else {
        if let Ok(mut g) = AIMED.lock() {
            *g = None;
        }
        return;
    };
    // A rebuilt mirror body (new outfit, new map) means a new first-person one too.
    if st.root.is_some() && st.gen != crate::augmentique::generation() {
        if let Some(r) = st.root.take() {
            commands.entity(r).despawn();
        }
    }
    if st.root.is_some_and(|r| fp.get(r).is_err()) && st.root.is_some_and(|r| commands.get_entity(r).is_err()) {
        st.root = None;
    }
    if !want {
        if let Some(r) = st.root {
            if let Ok(mut v) = vis.get_mut(r) {
                v.set_if_neq(Visibility::Hidden);
            }
        }
        if let Ok(mut g) = AIMED.lock() {
            *g = None;
        }
        return;
    }
    let jc = crate::travel::jc_mesh();
    let Some(sb) = crate::skeletal::body_for(&jc) else { return };
    let Some(root) = st.root else {
        // Built from the mirror body's textures once it has them.
        let r = commands.spawn((Transform::default(), Visibility::Inherited, FpBody)).id();
        let layer = if env_on!("DXVIEW_FP_LAYER0") { 0 } else { FP_LAYER };
        let made = built.build_from_mirror_without(&mut commands, b, sb, r, RenderLayers::layer(layer), FP_DROP, FP_KEEP);
        if env_on!("DXVIEW_BODYLOG") {
            println!("FPBODY build {}", made.is_some());
        }
        if let Some(s) = made {
            built.cull_back(&s);
            commands.entity(r).insert(s);
            st.root = Some(r);
            st.gen = crate::augmentique::generation();
        } else {
            commands.entity(r).despawn();
        }
        return;
    };
    let Ok((_, mut s, mut tf)) = fp.get_mut(root) else { return };
    // Crouched, the crouch pose folds the knees up round the eye: the body is left out until he stands.
    let crouching = b.cur.as_ref().is_some_and(|c| crate::util::contains_ci(&c.0, "crouch"));
    if let Ok(mut v) = vis.get_mut(root) {
        let want_v = if crouching { Visibility::Hidden } else { Visibility::Inherited };
        if *v != want_v {
            *v = want_v;
        }
    }
    // Where the mirror body stands, pulled back a little so the view looks down over the chest, not into it.
    if let Ok(mg) = gts.get(b.root) {
        let mt = mg.compute_transform();
        let fwd = mt.rotation * Vec3::X;
        *tf = mt;
        if env_on!("DXVIEW_BODYLOG") {
            println!("FPBODY at {:?} fwd {fwd:?} cam {:?}", mt.translation, cam.iter().next().map(|c| c.translation()));
        }
        let crouched = b.cur.as_ref().is_some_and(|c| crate::util::contains_ci(&c.0, "crouch"));
        let back = if crouched { FP_BACK_CROUCH } else { FP_BACK };
        let flat = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or_zero();
        tf.translation -= flat * (back * dx_level::METERS_PER_UNIT);
        tf.translation.y -= FP_DOWN * dx_level::METERS_PER_UNIT;
        // The pose (last frame's bones, in the body's own space) decides how much further back: the coat's front
        // at the chest must stay behind the view even looking straight down (walking, running and the aiming poses
        // lean the chest forward into the eye).
        // (Not while the camera is back over the shoulder: measured from there the chest was metres "ahead of the
        // eye", and back in first person the body then eased in from far behind - out of sight for seconds.)
        if crate::thirdperson::active() || crate::consys::third_person() || crate::cinematic::active() {
            st.extra = 0.0;
            st.skip = 3;
        } else if st.skip > 0 {
            st.skip -= 1;
        } else if let (Some(c), Ok(rg)) = (cam.iter().next(), gts.get(root)) {
            let eye = c.translation();
            // Along the view (the body can turn away from it in the aiming poses).
            let cf = c.forward().as_vec3();
            let flat = Vec3::new(cf.x, 0.0, cf.z).try_normalize().unwrap_or(flat);
            let inv = rg.affine().inverse();
            let m = dx_level::METERS_PER_UNIT;
            let mut need = 0.0f32;
            // A wider Field of View sees further back and down past the eye.
            let k = ((crate::weapons::base_fov().to_radians() * 0.5).tan() / 37.5f32.to_radians().tan()).max(1.0);
            for name in FP_CHEST {
                let Some(i) = s.rig.names.iter().position(|n| n == name) else { continue };
                let Ok(bg) = gts.get(s.nodes[i]) else { continue };
                let p = tf.transform_point(inv.transform_point3(bg.translation())) + flat * (FP_CHEST_FRONT * m);
                let ahead = (p - eye).dot(flat);
                let below = (eye.y - p.y).max(0.0);
                need = need.max(ahead + below * FP_VIEW_BACK * k + FP_CHEST_CLEAR * m);
            }
            let dt = time.delta_secs().min(0.1);
            let need = need.min(0.6 * k);
            st.extra = if need > st.extra { need } else { (st.extra - dt * 0.4).max(need) };
            tf.translation -= flat * st.extra.max(0.0);
        }
    }
    let medkit = weapons.held.as_ref().is_some_and(|h| crate::tnmitems::is_medkit(&h.stats.class));
    if let Some((seq, start, frames, f, looped)) = b.cur.clone() {
        let dt = time.delta_secs().min(0.1);
        // The mirror body (posed just before this, same rig, same move) already has this frame's pose: copied, not
        // sampled and blended a second time.
        match mirror.get(b.root).ok().filter(|m| m.pose.len() == s.pose.len()) {
            Some(m) => s.pose.clone_from(&m.pose),
            None => {
                crate::skeletal::advance_pub(&mut s, &seq, start, frames, f, looped, dt);
                if hurt_on() {
                    crate::skeletal::apply_hurt(&mut s, &hurt_fracs(&ph.parts, &[100.0; 6]));
                }
            }
        }
        // No head or arms under the view (the view model's are the arms).
        // (The head and arms were left out of this body when it was built: FP_DROP.)
        crate::skeletal::write_bones_pub(&s, &mut tfs, &[]);
        let ue = dx_level::bevy_to_ue(tf.translation.to_array());
        built.relight(&mut s, &level.0, ue, dt);
    }
    // Aiming for the medkit.
    let mut aimed = None;
    if medkit && diegetic_heal() {
        if let Some(c) = cam.iter().next() {
            let o = c.translation();
            let d = c.forward().as_vec3();
            let mut best = f32::MAX;
            for (part, a, bname, r) in AIM_PARTS {
                let (Some(ia), Some(ib)) = (s.rig.names.iter().position(|n| n == a), s.rig.names.iter().position(|n| n == bname)) else { continue };
                let (Ok(ga), Ok(gb)) = (gts.get(s.nodes[ia]), gts.get(s.nodes[ib])) else { continue };
                let dist = seg_ray(o, d, ga.translation(), gb.translation());
                if dist < *r && dist < best {
                    best = dist;
                    aimed = Some(*part);
                }
            }
        }
    }
    if let Ok(mut g) = AIMED.lock() {
        *g = aimed;
    }
}

// ------------------------------------------------------------------------------------------------ heal label

// ------------------------------------------------------------------------------------------------ glows

/// A glow on JC's body (aug or healing), riding a bone.
#[derive(Component)]
pub struct FpGlow {
    t: f32,
    life: Option<f32>,
    aug: Option<&'static str>,
    /// Its size before the pulse.
    base: f32,
}

static HEALED: Mutex<Vec<usize>> = Mutex::new(Vec::new());

/// Medicine went into body part `part`: the glow there and a pulse on the screen.
pub fn healed(part: usize) {
    if let Ok(mut g) = HEALED.lock() {
        g.push(part);
    }
}

/// Where each running aug glows: (aug class, bones).
const AUG_GLOWS: &[(&str, &[&str])] = &[
    ("AugVision", &["DEF-spine.006"]),
    ("AugTarget", &["DEF-spine.006"]),
    ("AugLight", &["DEF-spine.006"]),
    ("AugSpeed", &["DEF-shin.L", "DEF-shin.R"]),
    ("AugStealth", &["DEF-shin.L", "DEF-shin.R"]),
    ("AugMuscle", &["DEF-forearm.L", "DEF-forearm.R"]),
    ("AugCombat", &["DEF-forearm.L", "DEF-forearm.R"]),
    ("AugBallistic", &["DEF-spine.003"]),
    ("AugEMP", &["DEF-spine.003"]),
    ("AugShield", &["DEF-spine.003"]),
    ("AugEnviro", &["DEF-spine.003"]),
    ("AugHealing", &["DEF-spine.003"]),
    ("AugDefense", &["DEF-spine.004"]),
    ("AugCloak", &["DEF-spine.002"]),
    ("AugRadarTrans", &["DEF-spine.002"]),
    ("AugPower", &["DEF-spine.003"]),
    ("AugAqualung", &["DEF-spine.004"]),
    ("AugHeartLung", &["DEF-spine.003"]),
    ("AugDrone", &["DEF-spine.006"]),
];

/// Each aug's own glow colour (linear, past white so the bloom catches it): the eye augs tint the eyes and the light
/// streaming from them, the others glow where they sit.
pub(crate) fn aug_colour(aug: &str) -> Vec3 {
    match aug {
        "AugVision" => Vec3::new(0.35, 1.6, 0.45),
        "AugTarget" => Vec3::new(1.8, 0.25, 0.2),
        "AugLight" => Vec3::new(1.9, 1.6, 1.05),
        "AugDrone" => Vec3::new(0.3, 1.3, 1.9),
        "AugSpeed" => Vec3::new(0.4, 1.0, 2.0),
        "AugStealth" => Vec3::new(0.55, 0.45, 1.6),
        "AugMuscle" => Vec3::new(2.0, 0.85, 0.2),
        "AugCombat" => Vec3::new(2.0, 0.35, 0.15),
        "AugBallistic" => Vec3::new(1.3, 1.3, 1.4),
        "AugEMP" => Vec3::new(0.5, 0.8, 2.0),
        "AugShield" => Vec3::new(2.0, 0.6, 0.15),
        "AugEnviro" => Vec3::new(1.6, 1.8, 0.3),
        "AugHealing" => Vec3::new(0.3, 2.0, 0.5),
        "AugDefense" => Vec3::new(1.9, 1.3, 0.3),
        "AugCloak" => Vec3::new(0.6, 0.9, 1.3),
        "AugRadarTrans" => Vec3::new(1.2, 0.5, 1.8),
        "AugPower" => Vec3::new(1.9, 1.7, 0.4),
        "AugAqualung" => Vec3::new(0.25, 1.1, 2.0),
        "AugHeartLung" => Vec3::new(2.0, 0.3, 0.45),
        _ => Vec3::new(1.4, 0.9, 0.4),
    }
}

fn eye_aug(aug: &str) -> bool {
    matches!(aug, "AugVision" | "AugTarget" | "AugLight" | "AugDrone")
}

/// A soft round glow (white, alpha from the centre out) for the aug glows and the eye light.
fn soft_image() -> Image {
    let n = 64u32;
    let mut px = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let (dx, dy) = ((x as f32 + 0.5) / n as f32 * 2.0 - 1.0, (y as f32 + 0.5) / n as f32 * 2.0 - 1.0);
            let a = (-(dx * dx + dy * dy) * 4.0).exp() * (1.0 - (dx * dx + dy * dy).sqrt()).max(0.0);
            let v = (a.min(1.0) * 255.0) as u8;
            px.extend_from_slice(&[v, v, v, 255]);
        }
    }
    Image::new(
        bevy::render::render_resource::Extent3d { width: n, height: n, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        px,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    )
}

/// A leaf (pointed oval with a midrib, alpha cut) for the healing effect.
fn leaf_image() -> Image {
    let n = 48u32;
    let mut px = vec![0u8; (n * n * 4) as usize];
    for y in 0..n {
        for x in 0..n {
            let u = (y as f32 + 0.5) / n as f32 * 2.0 - 1.0;
            let v = (x as f32 + 0.5) / n as f32 * 2.0 - 1.0;
            let w = (1.0 - u * u).max(0.0) * 0.55;
            if v.abs() >= w {
                continue;
            }
            let rib = if v.abs() < 0.05 { 1.25 } else { 1.0 };
            let k = (0.75 + 0.25 * (1.0 - v.abs() / w.max(1e-3))) * rib;
            let i = ((y * n + x) * 4) as usize;
            px[i] = (0.35 * k * 255.0).min(255.0) as u8;
            px[i + 1] = (0.95 * k * 255.0).min(255.0) as u8;
            px[i + 2] = (0.4 * k * 255.0).min(255.0) as u8;
            px[i + 3] = 255;
        }
    }
    Image::new(
        bevy::render::render_resource::Extent3d { width: n, height: n, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        px,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    )
}

/// A healing leaf rising from where the medicine went: drifting up and out, turning, fading.
#[derive(Component)]
pub struct HealLeaf {
    vel: Vec3,
    spin: Vec3,
    t: f32,
    life: f32,
    size: f32,
}

#[derive(Default)]
pub struct GlowAssets {
    quad: Handle<Mesh>,
    leaf: Handle<StandardMaterial>,
    soft: Handle<Image>,
    /// One material per aug colour (soft glow), and a faint one for the halo round the glasses.
    by_aug: HashMap<&'static str, (Handle<StandardMaterial>, Handle<StandardMaterial>)>,
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn glow_system(
    mut commands: Commands,
    time: Res<Time>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    bodies: Query<(&SkelBody, Has<FpBody>, Has<crate::skeletal::MirrorSkel>)>,
    mut glows: Query<(Entity, &mut FpGlow, &mut Transform, &ChildOf), Without<HealLeaf>>,
    mut leaves: Query<(Entity, &mut HealLeaf, &mut Transform, &MeshMaterial3d<StandardMaterial>), Without<FpGlow>>,
    mut ph: ResMut<crate::ai::PlayerHealth>,
    mut assets: Local<Option<GlowAssets>>,
    mut active: Local<HashMap<(Entity, &'static str, usize), Entity>>,
    gts: Query<&GlobalTransform>,
    cam: Single<&GlobalTransform, (With<crate::FlyCam>, Without<crate::SkyCam>)>,
    mut seed: Local<u32>,
) {
    let dt = time.delta_secs();
    let ga = assets.get_or_insert_with(|| {
        let soft = images.add(soft_image());
        let leaf_tex = images.add(leaf_image());
        GlowAssets {
            quad: meshes.add(Rectangle::new(1.0, 1.0)),
            leaf: materials.add(StandardMaterial {
                base_color: Color::WHITE,
                base_color_texture: Some(leaf_tex),
                unlit: true,
                alpha_mode: AlphaMode::Mask(0.5),
                double_sided: true,
                cull_mode: None,
                ..default()
            }),
            soft,
            by_aug: HashMap::new(),
        }
    });
    let healed: Vec<usize> = HEALED.lock().map(|mut g| std::mem::take(&mut *g)).unwrap_or_default();
    if !healed.is_empty() {
        // The screen: a soft green pulse.
        ph.flash = ph.flash.max(0.35);
        ph.flash_color = [0.25, 0.9, 0.5];
    }
    let glows_on = on("AugGlows");
    let third = crate::thirdperson::active();
    let cam_rot = cam.compute_transform().rotation;
    let mut rnd = || {
        *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (*seed >> 8) as f32 / 16_777_216.0
    };
    // (key, bone, scale, offset, rotation, layer, eye light?)
    let mut want: Vec<((Entity, &'static str, usize), Entity, Vec3, Vec3, Quat, usize, bool)> = Vec::new();
    for (s, is_fp, is_mirror) in &bodies {
        if !is_fp && !is_mirror {
            continue;
        }
        let bone = |name: &str| s.rig.names.iter().position(|n| n == name).map(|i| s.nodes[i]);
        let layer = if is_fp { FP_LAYER } else { crate::mirror::BODY_LAYER };
        if glows_on {
            for (aug, bones) in AUG_GLOWS {
                if !crate::abilities::aug_active(aug) {
                    continue;
                }
                for (k, bn) in bones.iter().enumerate() {
                    // The first-person body has no head, neck or arms: a glow there would sit on the camera.
                    if is_fp && fp_dropped(bn) {
                        continue;
                    }
                    let Some(b) = bone(bn) else { continue };
                    if *bn != "DEF-spine.006" {
                        want.push(((s.root, *aug, k), b, Vec3::splat(0.07), Vec3::ZERO, Quat::IDENTITY, layer, false));
                        continue;
                    }
                    // The eyes: the aug glows through JC's glasses - a small bright spot on each lens and a faint
                    // soft halo round the pair (no beams: the light itself is the flashlight's own).
                    let (Ok(rg), Ok(hg)) = (gts.get(s.root), gts.get(b)) else { continue };
                    let a = rg.affine();
                    let fwd = a.transform_vector3(Vec3::Z).normalize_or_zero();
                    let up = a.transform_vector3(Vec3::Y).normalize_or_zero();
                    let lat = a.transform_vector3(Vec3::X).normalize_or_zero();
                    let inv = hg.affine().inverse();
                    // On the lenses: a touch in front of the eyes, where the glasses sit.
                    let lens = |side: f32| hg.translation() + fwd * 0.094 + up * 0.06 + lat * 0.032 * side;
                    let bright = if *aug == "AugLight" { 1.0 } else { 0.8 };
                    for (j, side) in [-1.0f32, 1.0].iter().enumerate() {
                        want.push(((s.root, *aug, k * 4 + j), b, Vec3::splat(0.026 * bright), inv.transform_point3(lens(*side)), Quat::IDENTITY, layer, false));
                    }
                    let mid = (lens(-1.0) + lens(1.0)) * 0.5 + fwd * 0.004;
                    want.push(((s.root, *aug, k * 4 + 2), b, Vec3::splat(0.11 * bright), inv.transform_point3(mid), Quat::IDENTITY, layer, true));
                }
            }
        }
        // Healing: leaves rising from the part treated - on the body you see (third person: JC's; first person:
        // the first-person body, not its missing head).
        let shown = if third { is_mirror } else { is_fp };
        if !shown {
            continue;
        }
        for part in &healed {
            let bn = match *part {
                crate::ai::P_HEAD => "DEF-spine.006",
                crate::ai::P_TORSO => "DEF-spine.003",
                crate::ai::P_ARM_L => "DEF-forearm.L",
                crate::ai::P_ARM_R => "DEF-forearm.R",
                crate::ai::P_LEG_L => "DEF-shin.L",
                _ => "DEF-shin.R",
            };
            if is_fp && bn == "DEF-spine.006" {
                continue;
            }
            let Some(at) = bone(bn).and_then(|b| gts.get(b).ok()).map(|g| g.translation()) else { continue };
            for _ in 0..14 {
                let off = Vec3::new(rnd() - 0.5, rnd() * 0.5 - 0.15, rnd() - 0.5) * 0.25;
                let out = Vec3::new(off.x, 0.0, off.z).normalize_or(Vec3::X);
                commands.spawn((
                    Mesh3d(ga.quad.clone()),
                    MeshMaterial3d(ga.leaf.clone()),
                    Transform::from_translation(at + off).with_rotation(cam_rot * Quat::from_rotation_z(rnd() * std::f32::consts::TAU)).with_scale(Vec3::splat(0.0)),
                    HealLeaf {
                        vel: Vec3::Y * (0.35 + rnd() * 0.35) + out * (0.05 + rnd() * 0.12),
                        spin: Vec3::new(rnd() - 0.5, rnd() - 0.5, rnd() - 0.5) * 6.0,
                        t: -rnd() * 0.25,
                        life: 1.1 + rnd() * 0.6,
                        size: 0.035 + rnd() * 0.025,
                    },
                    bevy::light::NotShadowCaster,
                    bevy::light::NotShadowReceiver,
                ));
            }
        }
    }
    // Aug glows: one per (body, aug, piece), kept while the aug runs.
    let keys: std::collections::HashSet<(Entity, &'static str, usize)> = want.iter().map(|w| w.0).collect();
    active.retain(|k, e| {
        let keep = keys.contains(k);
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
    for (k, bone, scale, at, rot, layer, beam) in want {
        if let Some(e) = active.get(&k) {
            // Keep the eye pieces where the eyes are (the head turns under them).
            if let Ok((_, _, mut t, _)) = glows.get_mut(*e) {
                t.translation = at;
            }
            continue;
        }
        let soft = ga.soft.clone();
        let (glow_mat, beam_mat) = ga
            .by_aug
            .entry(k.1)
            .or_insert_with(|| {
                let c = aug_colour(k.1);
                let col = Color::LinearRgba(LinearRgba::rgb(c.x, c.y, c.z));
                // The halo round the glasses: the same colour, faint.
                let beam_c = c * 0.22;
                (
                    materials.add(StandardMaterial { base_color: col, base_color_texture: Some(soft), unlit: true, alpha_mode: AlphaMode::Add, ..default() }),
                    materials.add(StandardMaterial {
                        base_color: Color::LinearRgba(LinearRgba::rgb(beam_c.x, beam_c.y, beam_c.z)),
                        base_color_texture: Some(ga.soft.clone()),
                        unlit: true,
                        alpha_mode: AlphaMode::Add,
                        ..default()
                    }),
                )
            })
            .clone();
        let (mesh, mat) = if beam { (ga.quad.clone(), beam_mat) } else { (ga.quad.clone(), glow_mat) };
        let e = commands
            .spawn((
                Mesh3d(mesh),
                MeshMaterial3d(mat),
                Transform::from_translation(at).with_rotation(rot).with_scale(scale),
                FpGlow { t: 0.0, life: None, aug: Some(k.1), base: scale.x },
                bevy::light::NotShadowCaster,
                RenderLayers::layer(layer),
                ChildOf(bone),
            ))
            .id();
        active.insert(k, e);
    }
    for (_e, mut g, mut t, parent) in &mut glows {
        g.t += dt;
        if g.life.is_none() {
            // The round glows: a slow pulse (the eye glows a soft shimmer), facing the camera.
            if let Ok(pg) = gts.get(parent.parent()) {
                t.rotation = pg.compute_transform().rotation.inverse() * cam_rot;
            }
            let p = if g.aug.is_some_and(eye_aug) { 0.92 + 0.05 * (g.t * 2.1).sin() + 0.03 * (g.t * 7.3).sin() } else { 0.85 + 0.15 * (g.t * 3.0).sin() };
            t.scale = Vec3::splat(g.base * p);
        }
    }
    // Leaves: rise, spread, turn, grow in and shrink away.
    for (e, mut l, mut t, _m) in &mut leaves {
        l.t += dt;
        if l.t < 0.0 {
            continue;
        }
        let k = l.t / l.life;
        if k >= 1.0 {
            commands.entity(e).despawn();
            continue;
        }
        l.vel *= 1.0 - 0.6 * dt;
        t.translation += l.vel * dt;
        let spin = l.spin * dt;
        t.rotation *= Quat::from_euler(EulerRot::XYZ, spin.x, spin.y, spin.z);
        let grow = (k / 0.15).min(1.0) * (1.0 - ((k - 0.6) / 0.4).max(0.0));
        t.scale = Vec3::splat(l.size * grow.max(0.0));
    }
}
