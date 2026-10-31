//! Bodies, in more detail (enhancements on Skeletal Characters):
//! - "Wounds On Bodies": a wound mark where a shot went into a rebuilt body, riding the bone it hit.
//! - "Hurt Poses": hurt people move like it - a wounded arm hangs, a wounded leg stiffens into a limp, a wounded
//!   trunk hunches (NPCs, and JC's own body in third person / mirrors / first person).
//! - "First-Person Body": JC's body under the view (no head; arms only while a medkit is in hand), on its own
//!   layer that only the main camera draws.
//! - "Diegetic Healing": with a medkit in hand, look at the part of your body to treat - an arm, a leg or the
//!   chest - and use it there; used looking anywhere else, it treats the head. A label under the crosshair says
//!   which part and how it is.
//! - "Aug Glows": running augmentations glow on JC's body where they sit (eyes, arms, legs, chest).
//! - Healing shows: a soft pulse on the screen and a glow where the medicine went.

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use glam::DVec3;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use crate::skeletal::SkelBody;

pub const FP_LAYER: usize = 11;
/// How far behind the eye JC's first-person body stands (Unreal units).
const FP_BACK: f32 = 3.0;
/// And this far lower (Unreal units): the coat's tall collar stays under the eye.
const FP_DOWN: f32 = 3.0;

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

static WOUNDS: Mutex<Vec<(usize, DVec3, DVec3, &'static str)>> = Mutex::new(Vec::new());

/// A shot into pawn `blocker` at `at` (Unreal space) travelling along `dir`.
pub fn wound(blocker: usize, at: DVec3, dir: DVec3, kind: &'static str) {
    if on("BodyWounds") {
        if let Ok(mut g) = WOUNDS.lock() {
            g.push((blocker, at, dir, kind));
        }
    }
}

/// The wound marks on one body (oldest first).
#[derive(Component, Default)]
pub struct Wounds(VecDeque<Entity>);

/// Bones a wound can sit on (the ones that carry skin, not the coat or fingers).
const WOUND_BONES: &[&str] = &[
    "DEF-spine", "DEF-spine.001", "DEF-spine.002", "DEF-spine.003", "DEF-spine.004", "DEF-spine.005", "DEF-spine.006",
    "DEF-upper_arm.L", "DEF-upper_arm.L.001", "DEF-forearm.L", "DEF-forearm.L.001", "DEF-upper_arm.R",
    "DEF-upper_arm.R.001", "DEF-forearm.R", "DEF-forearm.R.001", "DEF-thigh.L", "DEF-thigh.L.001", "DEF-shin.L",
    "DEF-shin.L.001", "DEF-thigh.R", "DEF-thigh.R.001", "DEF-shin.R", "DEF-shin.R.001",
];

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn wounds_system(
    mut commands: Commands,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut bodies: Query<(Entity, &crate::ai::Npc, &SkelBody, Option<&mut Wounds>)>,
    gts: Query<&GlobalTransform>,
    mut mat_cache: Local<HashMap<&'static str, Handle<StandardMaterial>>>,
    mut quad: Local<Option<Handle<Mesh>>>,
    mut seed: Local<u64>,
) {
    let list: Vec<(usize, DVec3, DVec3, &'static str)> = WOUNDS.lock().map(|mut g| std::mem::take(&mut *g)).unwrap_or_default();
    if list.is_empty() {
        return;
    }
    if *seed == 0 {
        *seed = 0x3011D;
    }
    let q = quad.get_or_insert_with(|| meshes.add(Rectangle::new(1.0, 1.0))).clone();
    for (blocker, at, dir, kind) in list {
        let Some((e, _, s, wounds)) = bodies.iter_mut().find(|(_, n, ..)| n.blocker == blocker) else { continue };
        let hit = Vec3::from(dx_level::ue_to_bevy(at.as_vec3().to_array()));
        let d = Vec3::new(dir.x as f32, dir.z as f32, dir.y as f32).normalize_or(Vec3::NEG_Z);
        // The nearest skin bone (by its segment to its child's head when there is one).
        let mut best: Option<(f32, Entity, Mat4)> = None;
        for name in WOUND_BONES {
            let Some(i) = s.rig.names.iter().position(|n| n == name) else { continue };
            let Ok(g) = gts.get(s.nodes[i]) else { continue };
            let p = g.translation();
            let dist = p.distance_squared(hit);
            if best.is_none_or(|b| dist < b.0) {
                best = Some((dist, s.nodes[i], g.to_matrix()));
            }
        }
        let Some((dist, bone, g)) = best else { continue };
        if dist > 0.6 * 0.6 {
            continue;
        }
        let tex = match kind {
            "Burned" | "Flamed" | "Exploded" => "DeusExItems.Skins.FlatFXTex7",
            _ => ["DeusExItems.Skins.FlatFXTex2", "DeusExItems.Skins.FlatFXTex5"][(*seed as usize >> 7) & 1],
        };
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let mat = match mat_cache.get(tex) {
            Some(m) => m.clone(),
            None => {
                let Some(img) = crate::weapons::modulated_image_for(&mut assets, &mut images, tex) else { continue };
                let m = materials.add(crate::impacts::decal_material(img, true));
                mat_cache.insert(tex, m.clone());
                m
            }
        };
        // Facing back along the shot, a little out from the bone toward where it went in.
        let size = if kind == "Shot" || kind == "Sabot" { 7.0 } else { 14.0 } * dx_level::METERS_PER_UNIT;
        let world = Transform::from_translation(hit - d * 0.01)
            .looking_to(d, if d.y.abs() > 0.9 { Vec3::X } else { Vec3::Y })
            .with_scale(Vec3::splat(size));
        let local = Transform::from_matrix(g.inverse() * world.to_matrix());
        let w = commands
            .spawn((Mesh3d(q.clone()), MeshMaterial3d(mat), local, bevy::light::NotShadowCaster, RenderLayers::layer(0), ChildOf(bone)))
            .id();
        match wounds {
            Some(mut ws) => {
                ws.0.push_back(w);
                while ws.0.len() > 10 {
                    if let Some(old) = ws.0.pop_front() {
                        commands.entity(old).despawn();
                    }
                }
            }
            None => {
                commands.entity(e).insert(Wounds(VecDeque::from([w])));
            }
        }
    }
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
}

/// Which body part the medkit would treat (aim ray against the first-person body): None = the head.
static AIMED: Mutex<Option<usize>> = Mutex::new(None);

pub fn aimed_part() -> Option<usize> {
    AIMED.lock().ok().and_then(|g| *g)
}

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
                *v = Visibility::Hidden;
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
        let layer = if std::env::var_os("DXVIEW_FP_LAYER0").is_some() { 0 } else { FP_LAYER };
        let made = built.build_from_mirror(&mut commands, b, sb, r, RenderLayers::layer(layer));
        if std::env::var_os("DXVIEW_BODYLOG").is_some() {
            println!("FPBODY build {}", made.is_some());
        }
        if let Some(s) = made {
            commands.entity(r).insert(s);
            st.root = Some(r);
            st.gen = crate::augmentique::generation();
        } else {
            commands.entity(r).despawn();
        }
        return;
    };
    let Ok((_, mut s, mut tf)) = fp.get_mut(root) else { return };
    if let Ok(mut v) = vis.get_mut(root) {
        *v = Visibility::Inherited;
    }
    // Where the mirror body stands, pulled back a little so the view looks down over the chest, not into it.
    if let Ok(mg) = gts.get(b.root) {
        let mt = mg.compute_transform();
        let fwd = mt.rotation * Vec3::X;
        *tf = mt;
        if std::env::var_os("DXVIEW_BODYLOG").is_some() {
            println!("FPBODY at {:?} fwd {fwd:?} cam {:?}", mt.translation, cam.iter().next().map(|c| c.translation()));
        }
        // The eye sits where his (hidden) head would; nudged back a touch so the chest isn't in the near plane.
        tf.translation -= Vec3::new(fwd.x, 0.0, fwd.z).normalize_or_zero() * (FP_BACK * dx_level::METERS_PER_UNIT);
        tf.translation.y -= FP_DOWN * dx_level::METERS_PER_UNIT;
    }
    let medkit = weapons.held.as_ref().is_some_and(|h| h.stats.class == "MedKit");
    if let Some((seq, start, frames, f, looped)) = b.cur.clone() {
        let dt = time.delta_secs().min(0.1);
        crate::skeletal::advance_pub(&mut s, &seq, start, frames, f, looped, dt);
        if hurt_on() {
            crate::skeletal::apply_hurt(&mut s, &hurt_fracs(&ph.parts, &[100.0; 6]));
        }
        // No head under the view; the arms only with a medkit in hand (otherwise the view model's are the arms).
        // (The neck bone too: the coat's tall collar hangs off it and would wall in the view.)
        let hide: &[&str] = if medkit { &["DEF-spine.004", "DEF-spine.005", "DEF-spine.006"] } else { &["DEF-spine.004", "DEF-spine.005", "DEF-spine.006", "DEF-shoulder", "DEF-upper_arm", "DEF-f_", "DEF-thumb"] };
        crate::skeletal::write_bones_pub(&s, &mut tfs, hide);
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

#[derive(Component)]
pub struct HealLabel;

pub fn heal_label_system(
    mut commands: Commands,
    weapons: Res<crate::weapons::Weapons>,
    ph: Res<crate::ai::PlayerHealth>,
    mut q: Query<(&mut Text, &mut Visibility), With<HealLabel>>,
) {
    let medkit = weapons.held.as_ref().is_some_and(|h| h.stats.class == "MedKit");
    let show = medkit && diegetic_heal() && FP_SHOWN.load(std::sync::atomic::Ordering::Relaxed);
    if q.is_empty() {
        commands.spawn((
            Text::new(""),
            TextFont { font_size: bevy::text::FontSize::Px(15.0), ..default() },
            TextColor(Color::srgb(0.75, 1.0, 0.8)),
            Node { position_type: PositionType::Absolute, top: Val::Percent(56.0), width: Val::Percent(100.0), justify_content: JustifyContent::Center, ..default() },
            bevy::text::TextLayout::justify(bevy::text::Justify::Center),
            Visibility::Hidden,
            HealLabel,
            GlobalZIndex(20),
        ));
        return;
    }
    for (mut t, mut v) in &mut q {
        *v = if show { Visibility::Inherited } else { Visibility::Hidden };
        if !show {
            continue;
        }
        let part = aimed_part().unwrap_or(crate::ai::P_HEAD);
        let name = ["Head", "Torso", "Left Arm", "Right Arm", "Left Leg", "Right Leg"][part];
        let s = format!("Treat {name}  {}%", ph.parts[part].max(0.0).round() as i32);
        if t.0 != s {
            t.0 = s;
        }
    }
}

// ------------------------------------------------------------------------------------------------ glows

/// A glow on JC's body (aug or healing), riding a bone.
#[derive(Component)]
pub struct FpGlow {
    t: f32,
    life: Option<f32>,
    aug: Option<&'static str>,
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

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn glow_system(
    mut commands: Commands,
    time: Res<Time>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    bodies: Query<(&SkelBody, Has<FpBody>, Has<crate::skeletal::MirrorSkel>)>,
    mut glows: Query<(Entity, &mut FpGlow, &mut Transform, &MeshMaterial3d<StandardMaterial>)>,
    mut ph: ResMut<crate::ai::PlayerHealth>,
    mut assets: Local<Option<(Handle<Mesh>, Handle<StandardMaterial>, Handle<StandardMaterial>)>>,
    mut active: Local<HashMap<(Entity, &'static str, usize), Entity>>,
    gts: Query<&GlobalTransform>,
) {
    let dt = time.delta_secs();
    let (sphere, aug_mat, heal_mat) = assets
        .get_or_insert_with(|| {
            let add = |c: Color| StandardMaterial { base_color: c, unlit: true, alpha_mode: AlphaMode::Add, ..default() };
            (
                meshes.add(Sphere::new(1.0).mesh().ico(2).unwrap()),
                materials.add(add(Color::srgba(1.0, 0.55, 0.15, 1.0))),
                materials.add(add(Color::srgba(0.3, 1.0, 0.55, 1.0))),
            )
        })
        .clone();
    let healed: Vec<usize> = HEALED.lock().map(|mut g| std::mem::take(&mut *g)).unwrap_or_default();
    if !healed.is_empty() {
        // The screen: a soft green pulse.
        ph.flash = ph.flash.max(0.35);
        ph.flash_color = [0.25, 0.9, 0.5];
    }
    let glows_on = on("AugGlows");
    let mut want: Vec<((Entity, &'static str, usize), Entity, f32, Vec3)> = Vec::new();
    for (s, is_fp, is_mirror) in &bodies {
        if !is_fp && !is_mirror {
            continue;
        }
        let bone = |name: &str| s.rig.names.iter().position(|n| n == name).map(|i| s.nodes[i]);
        if glows_on {
            for (aug, bones) in AUG_GLOWS {
                if !crate::abilities::aug_active(aug) {
                    continue;
                }
                for (k, bn) in bones.iter().enumerate() {
                    let Some(b) = bone(bn) else { continue };
                    if *bn != "DEF-spine.006" {
                        want.push(((s.root, *aug, k), b, 0.06, Vec3::ZERO));
                        continue;
                    }
                    // The eyes: in front of the head, either side (the body's forward is glTF +Z).
                    let (Ok(rg), Ok(hg)) = (gts.get(s.root), gts.get(b)) else { continue };
                    let a = rg.affine();
                    let fwd = a.transform_vector3(Vec3::Z).normalize_or_zero();
                    let up = a.transform_vector3(Vec3::Y).normalize_or_zero();
                    let lat = a.transform_vector3(Vec3::X).normalize_or_zero();
                    for (j, side) in [-1.0f32, 1.0].iter().enumerate() {
                        let p = hg.translation() + fwd * 0.085 + up * 0.06 + lat * 0.032 * side;
                        let local = hg.affine().inverse().transform_point3(p);
                        want.push(((s.root, *aug, k * 2 + j), b, 0.018, local));
                    }
                }
            }
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
            if let Some(b) = bone(bn) {
                commands.spawn((
                    Mesh3d(sphere.clone()),
                    MeshMaterial3d(materials.add(StandardMaterial { base_color: Color::srgba(0.3, 1.0, 0.55, 1.0), unlit: true, alpha_mode: AlphaMode::Add, ..default() })),
                    Transform::from_scale(Vec3::splat(0.12)),
                    FpGlow { t: 0.0, life: Some(0.9), aug: None },
                    bevy::light::NotShadowCaster,
                    RenderLayers::from_layers(&[FP_LAYER, crate::mirror::BODY_LAYER]),
                    ChildOf(b),
                ));
            }
        }
    }
    // Aug glows: one per (body, aug, bone), kept while the aug runs.
    let keys: std::collections::HashSet<(Entity, &'static str, usize)> = want.iter().map(|w| w.0).collect();
    active.retain(|k, e| {
        let keep = keys.contains(k);
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
    for (k, bone, r, at) in want {
        if active.contains_key(&k) {
            continue;
        }
        let e = commands
            .spawn((
                Mesh3d(sphere.clone()),
                MeshMaterial3d(aug_mat.clone()),
                Transform::from_translation(at).with_scale(Vec3::splat(r)),
                FpGlow { t: 0.0, life: None, aug: Some(k.1) },
                bevy::light::NotShadowCaster,
                RenderLayers::from_layers(&[FP_LAYER, crate::mirror::BODY_LAYER]),
                ChildOf(bone),
            ))
            .id();
        active.insert(k, e);
    }
    let _ = heal_mat;
    for (e, mut g, mut t, m) in &mut glows {
        g.t += dt;
        match g.life {
            Some(life) => {
                let k = (g.t / life).min(1.0);
                // Swells and fades.
                t.scale = Vec3::splat(0.08 + 0.12 * k);
                if let Some(mut mat) = materials.get_mut(&m.0) {
                    mat.base_color = Color::srgba(0.3 * (1.0 - k), 1.0 * (1.0 - k), 0.55 * (1.0 - k), 1.0);
                }
                if g.t >= life {
                    commands.entity(e).despawn();
                }
            }
            None => {
                // A slow pulse (the parent bone's scale is ~1).
                let p = 0.85 + 0.15 * (g.t * 3.0).sin();
                let base = if g.aug.is_some_and(|a| matches!(a, "AugVision" | "AugTarget" | "AugLight" | "AugDrone")) { 0.018 } else { 0.06 };
                t.scale = Vec3::splat(base * p);
            }
        }
    }
}
