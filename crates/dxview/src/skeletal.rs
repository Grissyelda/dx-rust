//! Skeletal characters (enhancement "Skeletal Characters"): the Machine God team's rebuilt human bodies on their
//! Rigify rig (126 deforming bones, face shape keys), in place of the original vertex-animated meshes. Shipped
//! with the game with the team's blessing (credit: THIRD_PARTY.md, README, credits); the bodies carry the
//! original's MultiSkins slot layout, so every character keeps the skins the map gives it.
//!
//! Data (tools/rig: bake_humans.py in Blender, then pack_body.py / pack_anims.py): one `.dxanim` per rig (node tree,
//! rest pose, 151 clips at the original's 10 frames a second and frame counts) and one `.dxbody` per body (skinned
//! sections named "NNN_STYLE_..." for MultiSkins slot NNN and its Unreal style, inverse bind poses, morph targets).
//!
//! Each frame the pawn's `Animated` state (sequence, frame, override clip; the same clock the vertex meshes use)
//! picks a clip and a time; the pose is sampled on the CPU, cross-faded from the pose on screen when the sequence
//! changes, and written to the bone entities (Bevy skins on the GPU). Lip sync, blinks, brows and glances drive
//! the face's morph targets. The original vertex parts stay (hidden) so everything that reads them still works.
//! The weapon follows the body's own weapon triangle (000_WEAPONTRI), skinned like the original's special face.
//!
//! Lighting: `ActorLit` (shaders/actor_lit.wgsl) lights each pixel from up to eight of the Unreal lights that reach
//! the pawn, the way dx-level's light_part lights vertices: colour x (1 - d / r) x max(N.L, 0), x LIGHT_GAIN x
//! ScaleGlow, clamped, to linear.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::mesh::morph::{MeshMorphWeights, MorphAttributes, MorphWeights};
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::actors::{Animated, MeshActorRef};
use crate::LevelRes;

// ------------------------------------------------------------------------------------------------ data

const BODIES: &[(&str, &[u8])] = &[
    ("GM_Trench", include_bytes!("../../../assets/rig/GM_Trench.dxbody")),
    ("GM_Suit", include_bytes!("../../../assets/rig/GM_Suit.dxbody")),
    ("GM_DressShirt", include_bytes!("../../../assets/rig/GM_DressShirt.dxbody")),
    ("GM_DressShirt_B", include_bytes!("../../../assets/rig/GM_DressShirt_B.dxbody")),
    ("GM_Jumpsuit", include_bytes!("../../../assets/rig/GM_Jumpsuit.dxbody")),
    ("GMK_DressShirt", include_bytes!("../../../assets/rig/GMK_DressShirt.dxbody")),
    ("GFM_Trench", include_bytes!("../../../assets/rig/GFM_Trench.dxbody")),
    ("GFM_Dress", include_bytes!("../../../assets/rig/GFM_Dress.dxbody")),
    ("GFM_SuitSkirt", include_bytes!("../../../assets/rig/GFM_SuitSkirt.dxbody")),
    ("GFM_TShirtPants", include_bytes!("../../../assets/rig/GFM_TShirtPants.dxbody")),
    ("GFMK_Dress", include_bytes!("../../../assets/rig/GFMK_Dress.dxbody")),
    ("GFM_Jumpsuit", include_bytes!("../../../assets/rig/GFM_Jumpsuit.dxbody")),
];
const ANIM_MALE: &[u8] = include_bytes!("../../../assets/rig/anims_male.dxanim");
const ANIM_FEMALE: &[u8] = include_bytes!("../../../assets/rig/anims_female.dxanim");

/// Blender units to Unreal units (the rebuilt bodies stand at the original meshes' size).
const UE_PER_BLENDER: f32 = 53.0;
/// Frames per second of the baked clips (the original sequences' frames, one per tenth of a second).
const CLIP_FPS: f32 = 10.0;
/// Cross-fade between sequences, seconds.
const FADE: f32 = 0.25;

struct Reader<'a> {
    b: &'a [u8],
    o: usize,
}

impl<'a> Reader<'a> {
    fn u8(&mut self) -> u8 {
        let v = self.b[self.o];
        self.o += 1;
        v
    }
    fn u16(&mut self) -> u16 {
        let v = u16::from_le_bytes([self.b[self.o], self.b[self.o + 1]]);
        self.o += 2;
        v
    }
    fn u32(&mut self) -> u32 {
        let v = u32::from_le_bytes(self.b[self.o..self.o + 4].try_into().unwrap());
        self.o += 4;
        v
    }
    fn f32(&mut self) -> f32 {
        f32::from_bits(self.u32())
    }
    fn i16(&mut self) -> i16 {
        self.u16() as i16
    }
    fn str8(&mut self) -> String {
        let n = self.u8() as usize;
        let s = String::from_utf8_lossy(&self.b[self.o..self.o + n]).into_owned();
        self.o += n;
        s
    }
    fn f32s(&mut self, n: usize) -> Vec<f32> {
        (0..n).map(|_| self.f32()).collect()
    }
}

#[derive(Clone, Copy)]
pub struct Trs {
    pub t: Vec3,
    pub r: Quat,
    pub s: Vec3,
}

impl Trs {
    fn lerp(&self, o: &Trs, k: f32) -> Trs {
        Trs { t: self.t.lerp(o.t, k), r: self.r.slerp(o.r, k), s: self.s.lerp(o.s, k) }
    }
    fn mat(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.s, self.r, self.t)
    }
}

struct Chan {
    node: usize,
    path: u8,
    times: usize,
    /// 3 floats per key (T / S) or 4 (R, normalised).
    data: Vec<f32>,
}

pub struct Clip {
    times: Vec<Vec<f32>>,
    chans: Vec<Chan>,
}

pub struct Rig {
    pub names: Vec<String>,
    pub parent: Vec<Option<usize>>,
    pub rest: Vec<Trs>,
    pub clips: HashMap<String, Clip>,
}

fn parse_rig(b: &[u8]) -> Rig {
    let mut r = Reader { b, o: 8 };
    let n = r.u16() as usize;
    let (mut names, mut parent, mut rest) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..n {
        let p = r.u16();
        parent.push((p != 0xFFFF).then_some(p as usize));
        names.push(r.str8());
        let t = Vec3::new(r.f32(), r.f32(), r.f32());
        let q = Quat::from_xyzw(r.f32(), r.f32(), r.f32(), r.f32());
        let s = Vec3::new(r.f32(), r.f32(), r.f32());
        rest.push(Trs { t, r: q, s });
    }
    let nc = r.u16() as usize;
    let mut clips = HashMap::new();
    for _ in 0..nc {
        let name = r.str8();
        let _dur = r.f32();
        let nt = r.u8() as usize;
        let times: Vec<Vec<f32>> = (0..nt)
            .map(|_| {
                let k = r.u16() as usize;
                r.f32s(k)
            })
            .collect();
        let nch = r.u16() as usize;
        let mut chans = Vec::with_capacity(nch);
        for _ in 0..nch {
            let node = r.u16() as usize;
            let path = r.u8();
            let ts = r.u8() as usize;
            let keys = times[ts].len();
            let data = if path == 1 {
                (0..keys * 4).map(|_| r.i16() as f32 / 32767.0).collect()
            } else {
                r.f32s(keys * 3)
            };
            chans.push(Chan { node, path, times: ts, data });
        }
        clips.insert(name.to_ascii_lowercase(), Clip { times, chans });
    }
    Rig { names, parent, rest, clips }
}

pub struct Section {
    pub material: String,
    pub slot: usize,
    pub style: String,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
    pub indices: Vec<u16>,
    /// Per morph target: position deltas (None = this target doesn't move the section).
    pub morphs: Vec<Option<Vec<[f32; 3]>>>,
}

pub struct Body {
    pub joint_names: Vec<String>,
    pub ibm: Vec<Mat4>,
    pub morphs: Vec<String>,
    pub sections: Vec<Section>,
    pub female: bool,
}

fn parse_body(name: &str, b: &[u8]) -> Body {
    let mut r = Reader { b, o: 8 };
    let nj = r.u16() as usize;
    let (mut joint_names, mut ibm) = (Vec::new(), Vec::new());
    for _ in 0..nj {
        joint_names.push(r.str8());
        let m = r.f32s(16);
        ibm.push(Mat4::from_cols_slice(&m));
    }
    let nm = r.u8() as usize;
    let morphs: Vec<String> = (0..nm).map(|_| r.str8()).collect();
    let ns = r.u8() as usize;
    let mut sections = Vec::new();
    for _ in 0..ns {
        let material = r.str8();
        let nv = r.u32() as usize;
        let v3 = |r: &mut Reader| -> Vec<[f32; 3]> { (0..nv).map(|_| [r.f32(), r.f32(), r.f32()]).collect() };
        let positions = v3(&mut r);
        let normals = v3(&mut r);
        let uvs: Vec<[f32; 2]> = (0..nv).map(|_| [r.f32(), r.f32()]).collect();
        let joints: Vec<[u16; 4]> =
            (0..nv).map(|_| [r.u8() as u16, r.u8() as u16, r.u8() as u16, r.u8() as u16]).collect();
        let weights: Vec<[f32; 4]> = (0..nv)
            .map(|_| {
                let w = [r.u16(), r.u16(), r.u16(), r.u16()];
                [w[0] as f32 / 65535.0, w[1] as f32 / 65535.0, w[2] as f32 / 65535.0, w[3] as f32 / 65535.0]
            })
            .collect();
        let ni = r.u32() as usize;
        let indices: Vec<u16> = (0..ni).map(|_| r.u16()).collect();
        let morph_d: Vec<Option<Vec<[f32; 3]>>> = (0..nm).map(|_| (r.u8() == 1).then(|| v3(&mut r))).collect();
        let mut it = material.splitn(3, '_');
        let slot = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let style = it.next().unwrap_or("NORMAL").to_string();
        sections.push(Section { material, slot, style, positions, normals, uvs, joints, weights, indices, morphs: morph_d });
    }
    Body { joint_names, ibm, morphs, sections, female: name.starts_with("GF") }
}

fn rigs() -> &'static [Rig; 2] {
    static R: OnceLock<[Rig; 2]> = OnceLock::new();
    R.get_or_init(|| [parse_rig(ANIM_MALE), parse_rig(ANIM_FEMALE)])
}

fn bodies() -> &'static HashMap<String, Body> {
    static B: OnceLock<HashMap<String, Body>> = OnceLock::new();
    B.get_or_init(|| BODIES.iter().map(|(n, b)| (n.to_ascii_lowercase(), parse_body(n, b))).collect())
}

/// The rebuilt body for an original mesh ("DeusExCharacters.GM_Trench" or "GM_Trench"), if there is one.
pub fn body_for(mesh: &str) -> Option<&'static Body> {
    let short = mesh.rsplit('.').next().unwrap_or(mesh).to_ascii_lowercase();
    if let Some(b) = bodies().get(&short) {
        return Some(b);
    }
    // Carcass meshes (GM_Trench_Carcass...) are a lying pose of their own, not the standing body: mapped onto the
    // rig they stood in its rest pose (a T-pose), unlit, where the map places a body.
    if short.contains("carcass") {
        return None;
    }
    // Augmentique's outfit meshes (AMTGM_Trench, AMTGFM_SuitSkirt_F...) are the
    // vanilla bodies under another name; variants (_F, _S, _B without a body of their own) fall back to the base.
    let mut n = short.as_str();
    for p in ["amt"] {
        if let Some(r) = n.strip_prefix(p) {
            n = r;
        }
    }
    let mut n = n.to_string();
    loop {
        if let Some(b) = bodies().get(&n) {
            return Some(b);
        }
        match n.rfind('_') {
            Some(i) if n[..i].contains('_') => n.truncate(i),
            _ => return None,
        }
    }
}

/// The textures Deus Ex hides a mesh section with.
fn is_mask(name: &str) -> bool {
    let n = name.rsplit('.').next().unwrap_or(name).to_ascii_lowercase();
    matches!(n.as_str(), "graymasktex" | "blackmasktex" | "pinkmasktex")
}

fn log_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| env_on!("DXVIEW_SKELLOG"))
}

pub fn enabled() -> bool {
    crate::settings::get("SkeletalCharacters") > 0.5
}

// ------------------------------------------------------------------------------------------------ sampling

impl Clip {
    fn frames(&self) -> usize {
        self.times.iter().map(|t| t.len()).max().unwrap_or(1).max(1)
    }
}

/// Pose `out` (one entry per rig node, starting from rest) at clip time `t`; `looped` wraps the last key back to
/// the first over one frame.
fn sample(rig: &Rig, clip: &Clip, t: f32, looped: bool, out: &mut [Trs]) {
    out.copy_from_slice(&rig.rest);
    for ch in &clip.chans {
        let times = &clip.times[ch.times];
        let n = times.len();
        if n == 0 {
            continue;
        }
        let last = times[n - 1];
        let (i0, i1, k) = if n == 1 || t <= times[0] {
            (0, 0, 0.0)
        } else if t >= last {
            if looped {
                let k = ((t - last) * CLIP_FPS).clamp(0.0, 1.0);
                (n - 1, 0, k)
            } else {
                (n - 1, n - 1, 0.0)
            }
        } else {
            let i = times.partition_point(|&x| x <= t).max(1) - 1;
            let span = (times[i + 1] - times[i]).max(1e-6);
            (i, i + 1, (t - times[i]) / span)
        };
        let o = &mut out[ch.node];
        match ch.path {
            1 => {
                let q = |i: usize| Quat::from_xyzw(ch.data[i * 4], ch.data[i * 4 + 1], ch.data[i * 4 + 2], ch.data[i * 4 + 3]).normalize();
                o.r = q(i0).slerp(q(i1), k);
            }
            p => {
                let v = |i: usize| Vec3::new(ch.data[i * 3], ch.data[i * 3 + 1], ch.data[i * 3 + 2]);
                let x = v(i0).lerp(v(i1), k);
                if p == 0 {
                    o.t = x;
                } else {
                    o.s = x;
                }
            }
        }
    }
}

/// The baked clip standing in for an original sequence name.
fn clip_for<'a>(rig: &'a Rig, seq: &str, female: bool) -> Option<&'a Clip> {
    let s = seq.to_ascii_lowercase();
    let g = if female { "feminine" } else { "masculine" };
    let mapped: Option<String> = match s.as_str() {
        "strafe" => Some("strafeleft".into()),
        "strafe2h" => Some("strafeleft2h".into()),
        "breathelight" | "walk" | "pickup" | "dance" => Some(format!("{s}{g}")),
        _ if s.starts_with("gesture") => Some(format!("{s}{g}")),
        _ => None,
    };
    mapped.and_then(|m| rig.clips.get(&m)).or_else(|| rig.clips.get(&s)).or_else(|| rig.clips.get(&format!("{s}{g}")))
}

// ------------------------------------------------------------------------------------------------ material

pub const ACTOR_SHADER: Handle<Shader> = bevy::asset::uuid_handle!("6b1e4f0a-93d2-4c75-8e1f-2a7c5d9b3e41");
pub type ActorMaterial = ExtendedMaterial<StandardMaterial, ActorLit>;

#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType, Reflect)]
pub struct ActorLights {
    /// xyz: light position (Bevy world), w: radius (Bevy units).
    pub pos: [Vec4; 16],
    /// rgb: Unreal light colour (brightness included).
    pub col: [Vec4; 16],
    /// x: lights in use, y: LIGHT_GAIN x ScaleGlow, z: 1 = unlit, w: 1 = lit from both sides.
    pub misc: Vec4,
    /// x: how far the eyelids are closed (0..1: the lid vertices' picture slides to UV_1, see lid_uvs), y: unused,
    /// z: world height above which nothing is drawn, w: 1 = that cut is on.
    pub wet: Vec4,
    /// rgb: the lights past the sixteen strongest, summed as an even fill (the vertex-lit meshes add up every
    /// light; leaving them out drew the rebuilt people darker).
    pub amb: Vec4,
}

/// Bindless like StandardMaterial: all bodies' sections share a few bind groups and draw as GPU multi-draws.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
#[data(100, ActorLights, binding_array(111))]
#[bindless(index_table(range(100..101), binding(110)))]
pub struct ActorLit {
    pub lights: ActorLights,
}

impl<'a> From<&'a ActorLit> for ActorLights {
    fn from(m: &'a ActorLit) -> Self {
        m.lights
    }
}

impl MaterialExtension for ActorLit {
    fn fragment_shader() -> ShaderRef {
        ACTOR_SHADER.into()
    }
    fn enable_prepass() -> bool {
        true
    }
    fn enable_shadows() -> bool {
        true
    }
}

pub fn setup_shader(mut shaders: ResMut<Assets<Shader>>) {
    let _ = shaders.insert(&ACTOR_SHADER, Shader::from_wgsl(include_str!("shaders/actor_lit.wgsl"), "dxview/actor_lit.wgsl"));
}

// ------------------------------------------------------------------------------------------------ runtime

/// On a pawn drawn with a rebuilt body.
#[derive(Component)]
pub struct SkelBody {
    body: &'static Body,
    pub(crate) rig: &'static Rig,
    mesh_actor: usize,
    /// The body's root (holds the alignment to the original mesh) and one entity per rig node.
    pub(crate) root: Entity,
    pub(crate) nodes: Vec<Entity>,
    meshes: Vec<Entity>,
    mats: Vec<Handle<ActorMaterial>>,
    /// Pose on screen, and the pose a cross-fade started from.
    pub(crate) pose: Vec<Trs>,
    /// Frames left in which the section meshes' Transforms are touched, and whether any was on screen last frame:
    /// Bevy 0.19 keeps a mesh instance's morph descriptor index from when the instance was last re-extracted, and a
    /// skinned section never moves (only its joints do), so one first seen before its weights were extracted kept
    /// "no morphs" for good - every standing person's face was frozen (JC's body moves, so his worked).
    morph_kick: u8,
    seen_last: bool,
    from: Vec<Trs>,
    scratch: Vec<Trs>,
    posed: bool,
    skip_dt: f32,
    fade: f32,
    shown_seq: Option<(usize, bool)>,
    /// The clip for `shown_seq` (looked up by name only when the sequence changes).
    clip: Option<&'static Clip>,
    /// Posed this frame: on screen last frame, near the eye or never posed (pose_system).
    pose_now: bool,
    /// Scratch for node globals (weapon frame, cloth) and the body joint -> rig node map (made on first use).
    gmat: Vec<Mat4>,
    joint_node: Vec<Option<usize>>,
    /// Glancing and blinking on their own (outside conversations too).
    blink_in: f32,
    blink_t: Option<f32>,
    glance_in: f32,
    glance: f32,
    glance_to: f32,
    light_in: f32,
    light_sig: u64,
    /// The weapon frame (origin, X, Y, Z) in the original mesh's space, as `AnimMesh::weapon_coords` gives it.
    pub weapon: Option<[[f32; 3]; 4]>,
    /// The turn that takes this body's weapon frame onto the original mesh's (worked out once, see weapon_fix).
    weapon_turn: Option<Mat3>,
    /// A ragdoll (ragdoll.rs) has the bones: no more posing.
    pub ragdoll: bool,
    /// The sections (by `mats` index) whose eyelids blink by sliding their picture (lid_uvs), and the blink last
    /// written to their materials.
    lid_mats: Vec<usize>,
    blink_shown: f32,
    /// The rig node -> pawn transform (the root's local Transform), and the coat's simulated bone tails (world).
    root_local: Mat4,
    cloth_tail: Vec<Vec3>,
    cloth_prev: Vec<Vec3>,
    /// The coat's heads and animated tails last frame (world; the substeps move between those and this frame's),
    /// the time not yet simulated and the cloth's own clock (wind gusts).
    cloth_head: Vec<Vec3>,
    cloth_goal: Vec<Vec3>,
    cloth_acc: f32,
    cloth_clock: f32,
    /// From glTF space (the rig's) to the actor's local Unreal space (before DrawScale).
    to_ue: Mat4,
}

/// A vertex part hidden while its pawn wears a rebuilt body.
#[derive(Component)]
pub struct HiddenPart;

/// Weapon frame for a pawn with a rebuilt body (pawn-local Unreal space, before DrawScale).
pub fn weapon_coords(q: &Query<&SkelBody>, e: Entity) -> Option<Option<[[f32; 3]; 4]>> {
    q.get(e).ok().map(|s| s.weapon)
}

fn mesh_name(level: &dx_level::Level, ma: usize) -> Option<String> {
    let m = level.mesh_actors.get(ma)?;
    let a = level.actors.get(m.actor)?;
    match unreal_pkg::props::find(&a.props, "Mesh") {
        Some(unreal_pkg::props::Value::Path(p)) => Some(p.clone()),
        _ => None,
    }
}

fn build_mesh(sec: &Section, nmorph: usize) -> Mesh {
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, sec.positions.clone());
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, sec.normals.clone());
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, sec.uvs.clone());
    m.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(sec.joints.clone()));
    m.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, sec.weights.clone());
    m.insert_indices(Indices::U16(sec.indices.clone()));
    if nmorph > 0 {
        let nv = sec.positions.len();
        let mut attrs = Vec::with_capacity(nv * nmorph);
        for k in 0..nmorph {
            match sec.morphs.get(k).and_then(|d| d.as_ref()) {
                Some(d) => attrs.extend(d.iter().map(|p| MorphAttributes::new(Vec3::from(*p), Vec3::ZERO, Vec3::ZERO))),
                None => attrs.extend(std::iter::repeat_n(MorphAttributes::new(Vec3::ZERO, Vec3::ZERO, Vec3::ZERO), nv)),
            }
        }
        m.set_morph_targets(attrs);
    }
    let _ = m.generate_skinned_mesh_bounds();
    m
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn spawn_system(
    mut commands: Commands,
    level: Res<LevelRes>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<ActorMaterial>>,
    std_mats: Res<Assets<StandardMaterial>>,
    anim_mats: Res<Assets<crate::gpuanim::AnimMaterial>>,
    mut ibms: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    pawns: Query<(Entity, &Animated, &Children), (With<MeshActorRef>, Without<SkelBody>)>,
    parts: Query<(
        &Mesh3d,
        Option<&MeshMaterial3d<StandardMaterial>>,
        Option<&MeshMaterial3d<crate::gpuanim::AnimMaterial>>,
    )>,
    skels: Query<(Entity, &SkelBody, &Animated)>,
    hidden_parts: Query<(Entity, &ChildOf), With<HiddenPart>>,
    mut vis: Query<&mut Visibility>,
    mut map_gen: Local<u64>,
    mut checked: Local<std::collections::HashSet<(Entity, usize)>>,
) {
    if crate::loader::new_map(&mut map_gen) {
        checked.clear();
    }
    let on = enabled();
    // Bodies whose pawn changed mesh (a carcass) or with the setting off: back to the vertex mesh.
    for (e, s, a) in &skels {
        if !on || (a.mesh_actor != s.mesh_actor && !s.ragdoll) {
            commands.entity(s.root).despawn();
            for m in &s.meshes {
                commands.entity(*m).despawn();
            }
            commands.entity(e).remove::<SkelBody>();
            checked.remove(&(e, s.mesh_actor));
            // The vertex parts come back.
            for (p, c) in &hidden_parts {
                if c.parent() == e {
                    if let Ok(mut v) = vis.get_mut(p) {
                        *v = Visibility::Inherited;
                    }
                    commands.entity(p).remove::<HiddenPart>();
                }
            }
        }
    }
    if !on {
        return;
    }
    let lvl = &level.0;
    for (e, a, kids) in &pawns {
        if !checked.insert((e, a.mesh_actor)) {
            continue;
        }
        let Some(name) = mesh_name(lvl, a.mesh_actor) else { continue };
        let Some(body) = body_for(&name) else { continue };
        let ma = &lvl.mesh_actors[a.mesh_actor];
        let Some(anim) = &ma.anim else { continue };
        let _rig = &rigs()[body.female as usize];
        // Textures by MultiSkins slot, from the vertex parts' materials.
        let mut slot_tex: HashMap<u32, Option<Handle<Image>>> = HashMap::new();
        for c in kids.iter() {
            let Ok((m3, sm, am)) = parts.get(c) else { continue };
            let Some(pi) = a.parts.iter().find(|(h, _)| *h == m3.0).map(|(_, i)| *i) else { continue };
            let tex = sm
                .and_then(|h| std_mats.get(&h.0))
                .map(|m| m.base_color_texture.clone())
                .or_else(|| am.and_then(|h| anim_mats.get(&h.0)).map(|m| m.base.base_color_texture.clone()));
            let Some(tex) = tex else { continue };
            // GrayMaskTex / BlackMaskTex / PinkMaskTex: the slot is hidden (Paul's empty glasses slots).
            let masked = ma.parts[pi].texture.and_then(|t| lvl.textures.get(t)).is_some_and(|t| is_mask(&t.name));
            if !masked {
                for s in &ma.parts[pi].slots {
                    slot_tex.entry(*s).or_insert(tex.clone());
                }
            }
            if let Ok(mut v) = vis.get_mut(c) {
                *v = Visibility::Hidden;
            }
            commands.entity(c).insert(HiddenPart);
        }
        let sb = build(&mut commands, &mut meshes, &mut mats, &mut ibms, body, e, &slot_tex, &anim.mesh, anim.draw_scale, a.mesh_actor, None, &[], None);
        commands.entity(e).insert(sb);
    }
}

/// Build a rebuilt body under `e` (the entity drawing the original mesh, actor-local space): its rig, its skinned
/// sections textured by `slot_tex` (MultiSkins slot -> texture; slots missing are left out) and the morph weights
/// on `e`. `layers`: the render layers (None = the world and the shadow lights).
#[allow(clippy::too_many_arguments)]
fn build(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<ActorMaterial>,
    ibms: &mut Assets<SkinnedMeshInverseBindposes>,
    body: &'static Body,
    e: Entity,
    slot_tex: &HashMap<u32, Option<Handle<Image>>>,
    mesh: &dx_level::actors::AnimMesh,
    ds: f32,
    mesh_actor: usize,
    layers: Option<RenderLayers>,
    drop: &[&str],
    cut: Option<f32>,
) -> SkelBody {
    let rig = &rigs()[body.female as usize];
    {
        // The original Still frame's lowest point is where the rebuilt body's feet (z = 0) stand.
        let still = mesh.seqs.iter().find(|s| s.name.eq_ignore_ascii_case("Still")).map_or(0, |s| s.start_frame.max(0) as usize);
        let fv = mesh.frame_verts;
        let zmin = (0..fv)
            .filter_map(|v| mesh.verts.get(still * fv + v))
            .map(|p| p[2])
            .fold(f32::MAX, f32::min);
        let zoff = if zmin == f32::MAX { -39.0 } else { zmin };
        // glTF (x, y, z) -> Unreal (53 z, -53 x, 53 y + zoff); in Bevy axes that's a quarter turn about Y.
        let to_ue = Mat4::from_cols(
            Vec4::new(0.0, -UE_PER_BLENDER, 0.0, 0.0),
            Vec4::new(0.0, 0.0, UE_PER_BLENDER, 0.0),
            Vec4::new(UE_PER_BLENDER, 0.0, 0.0, 0.0),
            Vec4::new(0.0, 0.0, zoff, 1.0),
        );
        let k = UE_PER_BLENDER * dx_level::METERS_PER_UNIT * ds;
        let root_tf = Transform {
            translation: Vec3::new(0.0, zoff * ds * dx_level::METERS_PER_UNIT, 0.0),
            rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            scale: Vec3::splat(k),
        };
        let root = commands.spawn((root_tf, Visibility::Inherited, ChildOf(e))).id();
        let mut nodes: Vec<Entity> = Vec::with_capacity(rig.names.len());
        for (i, n) in rig.names.iter().enumerate() {
            let r = rig.rest[i];
            let parent = rig.parent[i].map_or(root, |p| nodes[p]);
            nodes.push(commands.spawn((Transform { translation: r.t, rotation: r.r, scale: r.s }, Visibility::Inherited, Name::new(n.clone()), ChildOf(parent))).id());
        }
        let joints: Vec<Entity> = body
            .joint_names
            .iter()
            .map(|j| rig.names.iter().position(|n| n == j).map_or(root, |i| nodes[i]))
            .collect();
        let ibm = ibms.add(SkinnedMeshInverseBindposes::from(body.ibm.clone()));
        let weights_ent = e;
        let nm = body.morphs.len();
        let mut mesh_ents = Vec::new();
        let mut mat_handles = Vec::new();
        let mut lid_mats = Vec::new();
        let mut first_mesh: Option<Handle<Mesh>> = None;
        for sec in &body.sections {
            if sec.style == "WEAPONTRI" {
                continue;
            }
            // A slot the pawn hides (Unreal's PinkMaskTex / GrayMaskTex) has no part drawing it: leave it out.
            if !slot_tex.contains_key(&(sec.slot as u32)) {
                continue;
            }
            let tex = slot_tex.get(&(sec.slot as u32)).cloned().flatten();
            // Only sections the face shapes move (the head) carry morph targets: each morphing mesh needs a bind
            // group of its own on the GPU and can't be drawn in a multi-draw with the rest.
            let morphs = nm > 0 && sec.morphs.iter().any(|d| d.as_ref().is_some_and(|d| d.iter().any(|p| p[0] != 0.0 || p[1] != 0.0 || p[2] != 0.0)));
            let mut mesh = build_mesh(sec, if morphs { nm } else { 0 });
            // Left out (the first-person body's head and arms): triangles with a corner mostly on those bones,
            // cut away rather than shrunk (shrinking the bones dragged the shoulders and collar into wings).
            // `cut`: also every triangle with a corner above that fraction of the body's height as it stands in its
            // bind pose (the first-person body keeps the waist and legs; a cut in the shader left the depth prepass
            // drawing the rest, black).
            let top = cut.map(|f| {
                let h = body.sections.iter().flat_map(|q| q.positions.iter()).fold(0.0f32, |m, p| m.max(p[1]));
                h * f
            });
            if !drop.is_empty() || top.is_some() {
                let gone: Vec<bool> = (0..sec.positions.len())
                    .map(|v| {
                        if top.is_some_and(|t| sec.positions[v][1] > t) {
                            return true;
                        }
                        let w: f32 = (0..4)
                            .filter(|&k| {
                                body.joint_names
                                    .get(sec.joints[v][k] as usize)
                                    .is_some_and(|n| drop.iter().any(|d| n.starts_with(d)))
                            })
                            .map(|k| sec.weights[v][k])
                            .sum();
                        w > 0.4
                    })
                    .collect();
                let kept: Vec<u16> = sec
                    .indices
                    .chunks(3)
                    .filter(|t| t.len() == 3 && !t.iter().any(|&i| gone[i as usize]))
                    .flatten()
                    .copied()
                    .collect();
                if kept.is_empty() {
                    continue;
                }
                mesh.insert_indices(bevy::mesh::Indices::U16(kept));
            }
            let lids = morphs && lid_uvs(&mut mesh, sec, body);
            let mh = meshes.add(mesh);
            if morphs {
                first_mesh.get_or_insert(mh.clone());
            }
            let alpha = match sec.style.as_str() {
                "MASKED" => AlphaMode::Mask(0.5),
                "TRANSLUCENT" => AlphaMode::Add,
                "MODULATED" => AlphaMode::Multiply,
                _ => AlphaMode::Opaque,
            };
            let mat = mats.add(ActorMaterial {
                base: StandardMaterial {
                    base_color_texture: tex,
                    base_color: if sec.style == "MODULATED" { Color::LinearRgba(LinearRgba::rgb(4.59, 4.59, 4.59)) } else { Color::WHITE },
                    unlit: true,
                    double_sided: true,
                    cull_mode: None,
                    alpha_mode: alpha,
                    ..default()
                },
                // Modulated sections (glasses frames, masks) take no light in Unreal: lit, a GrayMaskTex slot (meant
                // to vanish) drew as black glasses.
                extension: ActorLit {
                    lights: ActorLights {
                        misc: Vec4::new(0.0, dx_level::lighting::LIGHT_GAIN, if sec.style == "MODULATED" { 1.0 } else { 0.0 }, 0.0),
                        // Lenses and masks never get wet: a sheen on an additive section draws it.
                        wet: Vec4::new(0.0, if matches!(sec.style.as_str(), "MODULATED" | "TRANSLUCENT") { 1.0 } else { 0.0 }, 0.0, 0.0),
                        ..default()
                    },
                },
            });
            let layer = layers.clone().unwrap_or_else(|| {
                if matches!(alpha, AlphaMode::Opaque | AlphaMode::Mask(_)) {
                    crate::shadowlight::caster_layers(&[])
                } else {
                    RenderLayers::layer(0)
                }
            });
            let me = commands
                .spawn((
                    Mesh3d(mh),
                    MeshMaterial3d(mat.clone()),
                    SkinnedMesh { inverse_bindposes: ibm.clone(), joints: joints.clone() },
                    bevy::camera::visibility::DynamicSkinnedMeshBounds,
                    Transform::default(),
                    Visibility::Inherited,
                    layer,
                    crate::shadowlight::ShadowCaster,
                    ChildOf(e),
                ))
                .id();
            if layers.is_none() && matches!(alpha, AlphaMode::Opaque | AlphaMode::Mask(_)) {
                commands.entity(me).insert(crate::shadowlight::Casts);
            }
            if morphs {
                commands.entity(me).insert(MeshMorphWeights::Reference(weights_ent));
            }
            mesh_ents.push(me);
            if lids {
                lid_mats.push(mat_handles.len());
            }
            mat_handles.push(mat);
        }
        if nm > 0 && first_mesh.is_some() {
            if let Ok(w) = MorphWeights::new(vec![0.0; nm], first_mesh) {
                commands.entity(e).insert(w);
            }
        }
        let rest = rig.rest.clone();
        SkelBody {
            body,
            rig,
            mesh_actor,
            root,
            nodes,
            meshes: mesh_ents,
            mats: mat_handles,
            pose: rest.clone(),
            morph_kick: 30,
            seen_last: false,
            from: rest,
            scratch: Vec::new(),
            posed: false,
            skip_dt: 0.0,
            fade: 0.0,
            shown_seq: None,
            clip: None,
            pose_now: true,
            gmat: Vec::new(),
            joint_node: Vec::new(),
            blink_in: 1.0 + crate::sound::frand() * 4.0,
            blink_t: None,
            glance_in: 2.0 + crate::sound::frand() * 4.0,
            glance: 0.0,
            glance_to: 0.0,
            light_in: crate::sound::frand() * 0.2,
            light_sig: 0,
            weapon: None,
            ragdoll: false,
            lid_mats,
            blink_shown: 0.0,
            weapon_turn: None,
            root_local: root_tf.to_matrix(),
            cloth_tail: Vec::new(),
            cloth_prev: Vec::new(),
            cloth_head: Vec::new(),
            cloth_goal: Vec::new(),
            cloth_acc: 0.0,
            cloth_clock: 0.0,
            to_ue,
        }
    }
}

/// Pose `s` for original sequence `seq` (starting at mesh frame `start`, `frames` long) at frame `f`, cross-fading
/// from the pose on screen when the sequence changes; then the weapon frame.
fn advance(s: &mut SkelBody, seq: &str, start: usize, frames: usize, f: f32, looped: bool, dt: f32) {
    let key = (start, looped);
    if s.shown_seq != Some(key) {
        if s.shown_seq.is_some() {
            s.from.clone_from(&s.pose);
            s.fade = FADE;
        }
        s.shown_seq = Some(key);
        let rig: &'static Rig = s.rig;
        s.clip = clip_for(rig, seq, s.body.female).or_else(|| clip_for(rig, "BreatheLight", s.body.female));
    }
    let clip = s.clip;
    let mut pose = std::mem::take(&mut s.scratch);
    pose.resize(s.rig.rest.len(), s.rig.rest[0]);
    if let Some(c) = clip {
        // The callers' frame keeps counting past the end of a looping sequence (Unreal wraps it when drawing): wrap
        // it here, or every looping move froze on its first frame after one pass (people slid about standing still).
        let f = if looped && frames > 1 { f.rem_euclid(frames as f32) } else { f };
        // Same frame count: frame f is clip time f / 10; otherwise stretch the clip over the sequence.
        let n = c.frames() as f32;
        let fr = if (n - frames as f32).abs() < 0.5 || frames <= 1 { f } else { f * n / frames as f32 };
        sample(s.rig, c, fr / CLIP_FPS, looped && frames > 1, &mut pose);
    } else {
        pose.copy_from_slice(&s.rig.rest);
    }
    if s.fade > 0.0 {
        s.fade = (s.fade - dt).max(0.0);
        let k = 1.0 - s.fade / FADE;
        let k = k * k * (3.0 - 2.0 * k);
        for (p, f0) in pose.iter_mut().zip(&s.from) {
            *p = f0.lerp(p, k);
        }
    }
    s.scratch = std::mem::replace(&mut s.pose, pose);
    s.posed = true;
    // The weapon triangle, skinned by hand from the pose (glTF space -> the original mesh's space).
    s.weapon = weapon_frame(s);
}

fn write_bones(s: &SkelBody, tfs: &mut Query<&mut Transform, Without<SkelBody>>) {
    for (i, n) in s.nodes.iter().enumerate() {
        if let Ok(mut t) = tfs.get_mut(*n) {
            let p = s.pose[i];
            t.translation = p.t;
            t.rotation = p.r;
            t.scale = p.s;
        }
    }
}

/// The sequence a frame belongs to: the one starting there, else the shortest holding it. (Deus Ex meshes list an
/// "All" sequence over every frame first; taking the first match named every pose and mouth shape "All", so no
/// person walked, gestured or moved their lips.)
fn seq_name(mesh: &dx_level::actors::AnimMesh, frame: usize) -> Option<&str> {
    let holds = |s: &&unreal_pkg::mesh::AnimSeq| frame >= s.start_frame.max(0) as usize && frame < (s.start_frame.max(0) + s.num_frames.max(1)) as usize;
    mesh.seqs
        .iter()
        .filter(holds)
        .filter(|s| !s.name.eq_ignore_ascii_case("All"))
        .min_by_key(|s| (s.start_frame.max(0) as usize != frame, s.num_frames))
        .or_else(|| mesh.seqs.iter().find(holds))
        .map(|s| s.name.as_str())
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn pose_system(
    time: Res<Time>,
    level: Res<LevelRes>,
    mut q: Query<(Entity, &mut SkelBody, &mut Animated, &GlobalTransform, Option<&mut MorphWeights>, Option<&crate::ai::Npc>)>,
    kids: Query<&Children>,
    mut tfs: Query<&mut Transform, Without<SkelBody>>,
    mut hidden: Query<&mut Visibility, With<HiddenPart>>,
    seen: Query<&ViewVisibility>,
    cam: Query<&GlobalTransform, (With<crate::FlyCam>, Without<crate::SkyCam>)>,
    mut mats: ResMut<Assets<ActorMaterial>>,
    mut tick: Local<u32>,
) {
    let _prof = crate::prof::scope("skeletal::pose_system");
    let dt = time.delta_secs().min(0.1);
    let lvl = &level.0;
    *tick = tick.wrapping_add(1);
    let tick = *tick;
    let eye = cam.iter().next().map(|t| t.translation());
    // Which bodies to pose: any of their sections on screen last frame, or near the eye (the skinned bounds lag a
    // frame behind the pose and a body culled on stale bounds must not freeze for good), or never posed yet.
    for (_, mut s, _, gt, ..) in &mut q {
        let want = s.shown_seq.is_none()
            || eye.is_none_or(|c| c.distance_squared(gt.translation()) < 45.0 * 45.0)
            || s.meshes.iter().any(|m| seen.get(*m).is_ok_and(|v| v.get()));
        if s.pose_now != want {
            s.pose_now = want;
        }
    }
    if log_on() && tick % 120 == 0 {
        for (e, s, a, gt, ..) in q.iter() {
            let seen_any = s.meshes.iter().any(|m| seen.get(*m).is_ok_and(|v| v.get()));
            let d = eye.map_or(-1.0, |c| c.distance(gt.translation()));
            bevy::log::info!(
                "SKEL {:?} actor {} seq {:?} t {:.2} seen {} near {:.1} m posed {} ragdoll {} lights {:.2}",
                e, s.mesh_actor, s.shown_seq, a.time, seen_any, d, s.pose_now, s.ragdoll, s.light_in
            );
        }
    }
    // Poses, faces and weapon frames, in parallel.
    let hurt = crate::bodyfx::hurt_on();
    let cloth_wanted = cloth_on();
    q.par_iter_mut().for_each(|(e, mut s, mut a, gt, weights, npc)| {
        let s = &mut *s;
        s.posed = false;
        if s.ragdoll {
            return;
        }
        let Some(ma) = lvl.mesh_actors.get(s.mesh_actor) else { return };
        let Some(anim) = &ma.anim else { return };
        // The vertex parts are hidden, so `actors::animate` only advances the clocks; the face runs here.
        let face = crate::actors::update_face(&mut a.face, &anim.mesh, dt);
        if !s.pose_now {
            return;
        }
        // Far off (over ~30 m), a new pose every third frame.
        s.skip_dt += dt;
        let far = eye.is_some_and(|c| c.distance_squared(gt.translation()) > 30.0 * 30.0);
        if far && s.shown_seq.is_some() && (tick.wrapping_add(e.index_u32())) % 3 != 0 {
            return;
        }
        let dt = std::mem::take(&mut s.skip_dt);
        let (start, frames, f) = crate::actors::current_frame(&a, anim);
        let seq = seq_name(&anim.mesh, start).unwrap_or("BreatheLight");
        advance(s, seq, start, frames, f, a.clip.as_ref().is_none_or(|c| c.looping), dt);
        weapon_fix(s, &anim.mesh, start, frames, f);
        if hurt {
            if let Some(n) = npc.filter(|n| n.state != crate::ai::NpcState::Dead) {
                apply_hurt(s, &crate::bodyfx::hurt_fracs(&n.hp_body.parts, &n.hp_body.defaults));
            }
        }
        if cloth_wanted {
            // Not while seated: the coat pressed into the chair jittered against it.
            if seq.starts_with("Sit") || npc.is_some_and(|n| n.seat.is_some()) {
                cloth_rest(s);
            } else {
                cloth(s, gt.to_matrix(), dt);
            }
        }
        // Face: lip sync (the mouth shapes), blinks, brows with speech, glances.
        let has_w = weights.is_some();
        let mut wmax = 0.0f32;
        if let Some(mut w) = weights {
            face_weights(s, &anim.mesh, face.as_ref(), a.face.talking, dt, w.weights_mut());
            wmax = w.weights().iter().fold(0.0f32, |m, x| m.max(x.abs()));
        }
        if log_on() && tick % 10 == 0 && eye.is_some_and(|c| c.distance(gt.translation()) < 12.0) {
            let hips: f32 = s.pose.iter().map(|p| p.r.x.abs() + p.r.y.abs() + p.t.y).sum();
            bevy::log::info!(
                "SKELPOSE actor {} seq {seq} start {start} frames {frames} f {f:.2} clip {} face {:?} talking {} weights {} max {wmax:.2} posesum {:.4}",
                s.mesh_actor, a.clip.is_some(), face, a.face.talking, has_w, hips
            );
        }
    });
    // Bones, hidden parts and lights.
    for (e, mut s, _a, gt, _w, _n) in &mut q {
        if let Ok(ch) = kids.get(e) {
            for c in ch.iter() {
                if let Ok(mut v) = hidden.get_mut(c) {
                    if *v != Visibility::Hidden {
                        *v = Visibility::Hidden;
                    }
                }
            }
        }
        if s.posed {
            write_bones(&s, &mut tfs);
        }
        if !s.lid_mats.is_empty() {
            let blink = s
                .body
                .morphs
                .iter()
                .position(|m| m.eq_ignore_ascii_case("Blink"))
                .and_then(|i| _w.as_ref().and_then(|w| w.weights().get(i).copied()))
                .unwrap_or(0.0);
            set_blink(&mut s, blink, &mut mats);
        }
        // Re-extract the section meshes (their morph descriptors) for a while after the body appears and whenever
        // it comes back on screen.
        let seen_now = s.meshes.iter().any(|m| seen.get(*m).is_ok_and(|v| v.get()));
        if seen_now && !s.seen_last {
            s.morph_kick = s.morph_kick.max(4);
        }
        s.seen_last = seen_now;
        if s.morph_kick > 0 {
            s.morph_kick -= 1;
            for m in &s.meshes {
                if let Ok(mut t) = tfs.get_mut(*m) {
                    t.set_changed();
                }
            }
        }
        let Some(ma) = lvl.mesh_actors.get(s.mesh_actor) else { continue };
        s.light_in -= dt;
        if s.light_in > 0.0 {
            continue;
        }
        // Off screen too (slower): a body walking into view is never drawn unlit.
        s.light_in = if s.pose_now { 0.15 } else { 1.0 };
        let ue = dx_level::bevy_to_ue(gt.translation().to_array());
        relight(&mut s, lvl, ue, ma.scale_glow, &mut mats);
    }
}

/// Node globals (glTF space) from the local pose.
fn globals(rig: &Rig, pose: &[Trs], g: &mut Vec<Mat4>) {
    g.clear();
    for (i, p) in pose.iter().enumerate() {
        let m = p.mat();
        let v = match rig.parent[i] {
            Some(pi) => g[pi] * m,
            None => m,
        };
        g.push(v);
    }
}

fn weapon_frame(s: &mut SkelBody) -> Option<[[f32; 3]; 4]> {
    let body: &'static Body = s.body;
    let sec = body.sections.iter().find(|x| x.style == "WEAPONTRI")?;
    if sec.indices.len() < 3 {
        return None;
    }
    if s.joint_node.len() != body.joint_names.len() {
        s.joint_node = body.joint_names.iter().map(|j| s.rig.names.iter().position(|n| n == j)).collect();
    }
    let mut g = std::mem::take(&mut s.gmat);
    globals(s.rig, &s.pose, &mut g);
    let joint_node = &s.joint_node;
    let skin = |v: usize| -> Vec3 {
        let p = Vec3::from(sec.positions[v]).extend(1.0);
        let mut o = Vec4::ZERO;
        for k in 0..4 {
            let w = sec.weights[v][k];
            if w <= 0.0 {
                continue;
            }
            let j = sec.joints[v][k] as usize;
            if let (Some(Some(n)), Some(ibm)) = (joint_node.get(j), s.body.ibm.get(j)) {
                o += (g[*n] * *ibm * p) * w;
            }
        }
        s.to_ue.transform_point3(o.truncate())
    };
    let (v0, v1, v2) = (skin(sec.indices[0] as usize), skin(sec.indices[1] as usize), skin(sec.indices[2] as usize));
    s.gmat = g;
    // The same frame AnimMesh::weapon_coords builds from the original's special face.
    let x = (v1 - v0).normalize_or_zero();
    let y = (v0 - v2).cross(x).normalize_or_zero();
    let z = x.cross(y);
    let o = (v0 + v2) * 0.5;
    Some([o.to_array(), x.to_array(), y.to_array(), z.to_array()])
}

/// Per rebuilt body (by its address): the weapon frame's turn once some pawn measured it.
static BODY_WEAPON_TURN: std::sync::Mutex<Option<HashMap<usize, Mat3>>> = std::sync::Mutex::new(None);

/// The weapon frame from the rebuilt body's weapon triangle doesn't always sit like the original mesh's special face
/// (the remodelled triangle can face the other way: JC and some NPCs held their guns upside down). The first time a
/// body is posed, its frame is compared with the original mesh's at the same frame; the nearest whole-axis turn
/// between them (identity when they already agree) is kept and applied to every frame after.
fn weapon_fix(s: &mut SkelBody, mesh: &dx_level::actors::AnimMesh, start: usize, frames: usize, f: f32) {
    let Some(w) = s.weapon else { return };
    let axes = |w: &[[f32; 3]; 4]| Mat3::from_cols(Vec3::from(w[1]), Vec3::from(w[2]), Vec3::from(w[3]));
    if s.weapon_turn.is_none() {
        let key = s.body as *const Body as usize;
        let Some(v) = mesh.weapon_coords(start, frames, f) else { return };
        let r = axes(&v) * axes(&w).transpose();
        // Snap to a signed permutation (the triangle's axes swapped / flipped).
        let snap = |x: f32| if x > 0.5 { 1.0 } else if x < -0.5 { -1.0 } else { 0.0 };
        let q = Mat3::from_cols(
            Vec3::new(snap(r.x_axis.x), snap(r.x_axis.y), snap(r.x_axis.z)),
            Vec3::new(snap(r.y_axis.x), snap(r.y_axis.y), snap(r.y_axis.z)),
            Vec3::new(snap(r.z_axis.x), snap(r.z_axis.y), snap(r.z_axis.z)),
        );
        let ok = (q.determinant() - 1.0).abs() < 1e-3
            && (q * q.transpose()).abs_diff_eq(Mat3::IDENTITY, 1e-3)
            // Close to it (within ~30 degrees): the two poses agree well enough to tell.
            && r.abs_diff_eq(q, 0.45);
        // The turn belongs to the rebuilt body, not the pawn: a pawn caught in a pose that differs too much from the
        // original's takes what another pawn on the same body measured (or tries again next frame).
        let mut guard = BODY_WEAPON_TURN.lock().ok();
        let known = guard.as_mut().map(|g| g.get_or_insert_with(HashMap::new));
        if ok {
            s.weapon_turn = Some(q);
            if let Some(k) = known {
                k.entry(key).or_insert(q);
            }
        } else if let Some(t) = known.and_then(|k| k.get(&key).copied()) {
            s.weapon_turn = Some(t);
        }
        if log_on() {
            bevy::log::info!("SKELWEAPON actor {} turn {:?} (measured {ok}, raw {:?})", s.mesh_actor, s.weapon_turn, r);
        }
    }
    if let Some(t) = s.weapon_turn.filter(|t| *t != Mat3::IDENTITY) {
        let n = t * axes(&w);
        s.weapon = Some([w[0], n.x_axis.to_array(), n.y_axis.to_array(), n.z_axis.to_array()]);
    }
}

/// Blinking: the Blink shape pulls the upper lid down over the eye, stretching whatever the face picture holds
/// between the brow and the lid's edge - on some faces (Paul's, Helios's larger painted eyes) that strip is part of
/// the eye, so a closing lid showed a stretched eyeball. Every vertex the Blink shape moves gets a second set of picture
/// coordinates (UV_1) = those of the nearest still vertex above it (the skin of the crease), and actor_lit.wgsl slides
/// the lid's picture toward them as the blink closes (ActorLights.wet.x): the lids close as skin and the open eye is
/// untouched. False when the section has no lids.
fn lid_uvs(mesh: &mut Mesh, sec: &Section, body: &Body) -> bool {
    let Some(k) = body.morphs.iter().position(|m| m.eq_ignore_ascii_case("Blink")) else { return false };
    let Some(Some(d)) = sec.morphs.get(k) else { return false };
    let moving: Vec<bool> = d.iter().map(|v| v[0].abs() + v[1].abs() + v[2].abs() > 1e-4).collect();
    if !moving.iter().any(|m| *m) {
        return false;
    }
    let mut uvs = sec.uvs.clone();
    for (v, &m) in moving.iter().enumerate() {
        if !m {
            continue;
        }
        let p = Vec3::from(sec.positions[v]);
        let best = (0..sec.positions.len())
            .filter(|&u| !moving[u] && sec.positions[u][1] > p.y)
            .min_by(|&a, &b| Vec3::from(sec.positions[a]).distance_squared(p).total_cmp(&Vec3::from(sec.positions[b]).distance_squared(p)));
        if let Some(u) = best {
            uvs[v] = sec.uvs[u];
        }
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, uvs);
    true
}

/// The blink (the Blink shape's weight) on the lid sections' materials, when it changed enough to show.
fn set_blink(s: &mut SkelBody, blink: f32, mats: &mut Assets<ActorMaterial>) {
    let blink = blink.clamp(0.0, 1.0);
    if (blink - s.blink_shown).abs() < 0.02 && !(blink == 0.0 && s.blink_shown != 0.0) {
        return;
    }
    s.blink_shown = blink;
    for &i in &s.lid_mats {
        if let Some(mut m) = s.mats.get(i).and_then(|h| mats.get_mut(h)) {
            m.extension.lights.wet.x = blink;
        }
    }
}

/// DXVIEW_FACETEST="MouthA:1,Blink:0.5": hold these face shapes on every rebuilt body (screenshots).
fn face_test() -> &'static [(String, f32)] {
    static T: OnceLock<Vec<(String, f32)>> = OnceLock::new();
    T.get_or_init(|| {
        std::env::var("DXVIEW_FACETEST")
            .ok()
            .map(|v| v.split(',').filter_map(|p| p.split_once(':').and_then(|(n, x)| Some((n.trim().to_string(), x.trim().parse().ok()?)))).collect())
            .unwrap_or_default()
    })
}

fn face_weights(s: &mut SkelBody, mesh: &dx_level::actors::AnimMesh, face: Option<&crate::actors::FaceKey>, talking: bool, dt: f32, w: &mut [f32]) {
    let idx = |name: &str| s.body.morphs.iter().position(|m| m.eq_ignore_ascii_case(name));
    for x in w.iter_mut() {
        *x = 0.0;
    }
    if !face_test().is_empty() {
        for (n, v) in face_test() {
            if let Some(i) = idx(n) {
                if let Some(x) = w.get_mut(i) {
                    *x = *v;
                }
            }
        }
        return;
    }
    let set = |name: &str, v: f32, w: &mut [f32]| {
        if let Some(i) = idx(name) {
            if let Some(x) = w.get_mut(i) {
                *x = (*x + v).clamp(-1.0, 1.0);
            }
        }
    };
    let mut speaking = false;
    if let Some((a, b, k, blink)) = face {
        let k = *k as f32 / 64.0;
        let shape = |fr: usize| seq_name(mesh, fr).map(|n| n.to_string());
        if let Some(n) = shape(*a) {
            if !n.eq_ignore_ascii_case("MouthClosed") {
                set(&n, 1.0 - k, w);
                speaking = true;
            }
        }
        if let Some(n) = shape(*b) {
            if !n.eq_ignore_ascii_case("MouthClosed") {
                set(&n, k, w);
                speaking = true;
            }
        }
        if let Some(bf) = blink {
            if let Some(sq) = mesh.seqs.iter().find(|q| q.name.eq_ignore_ascii_case("Blink")) {
                let n = sq.num_frames.max(1) as f32;
                let t = (*bf as f32 / 64.0 / n).clamp(0.0, 1.0);
                set("Blink", (t * std::f32::consts::PI).sin(), w);
            }
        }
    }
    // Blinks of their own every few seconds (a quick close and open, ~0.18 s).
    s.blink_in -= dt;
    if s.blink_in <= 0.0 && s.blink_t.is_none() {
        s.blink_t = Some(0.0);
        s.blink_in = 2.0 + crate::sound::frand() * 4.5;
    }
    if let Some(t) = s.blink_t.as_mut() {
        *t += dt;
        let k = *t / 0.18;
        if k >= 1.0 {
            s.blink_t = None;
        } else {
            set("Blink", (k * std::f32::consts::PI).sin(), w);
        }
    }
    // Glances: the eyes move now and then, more often in a conversation.
    s.glance_in -= dt;
    if s.glance_in <= 0.0 {
        s.glance_to = if crate::sound::frand() < 0.5 { 0.0 } else { (crate::sound::frand() * 2.0 - 1.0) * 0.6 };
        s.glance_in = if talking { 0.8 + crate::sound::frand() * 2.0 } else { 2.0 + crate::sound::frand() * 5.0 };
    }
    s.glance += (s.glance_to - s.glance) * (dt * 12.0).min(1.0);
    if s.glance > 0.0 {
        set("EyesRight", s.glance, w);
    } else {
        set("EyesLeft", -s.glance, w);
    }
    // Brows follow the speech a little; a hint of a smile between lines in a conversation.
    if speaking {
        set("BrowsUp", 0.25, w);
    } else if talking {
        set("MouthSmile", 0.15, w);
    }
}

// ------------------------------------------------------------------------------------------------ JC's own body

/// On JC's mirror / third-person body root (mirror.rs) while it wears a rebuilt body.
#[derive(Component)]
pub struct MirrorSkel;

static MIRROR_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static MIRROR_WEAPON: std::sync::Mutex<Option<[[f32; 3]; 4]>> = std::sync::Mutex::new(None);

/// JC's mirror / third-person body is a rebuilt one (mirror.rs then skips posing the vertex mesh).
pub fn mirror_skel_on() -> bool {
    MIRROR_ON.load(std::sync::atomic::Ordering::Relaxed)
}

/// The weapon frame of JC's rebuilt body (body-local Unreal space), as `AnimMesh::weapon_coords` gives it.
pub fn mirror_weapon() -> Option<[[f32; 3]; 4]> {
    MIRROR_WEAPON.lock().ok().and_then(|g| *g)
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn mirror_skel_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    body: Res<crate::mirror::MirrorBodyRes>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<ActorMaterial>>,
    std_mats: Res<Assets<StandardMaterial>>,
    mut ibms: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    kids: Query<&Children>,
    parts: Query<(&Mesh3d, &MeshMaterial3d<StandardMaterial>)>,
    mut skels: Query<(Entity, &mut SkelBody, &GlobalTransform, Option<&mut MorphWeights>), With<MirrorSkel>>,
    ph: Res<crate::ai::PlayerHealth>,
    mut tfs: Query<&mut Transform, Without<SkelBody>>,
    mut vis: Query<&mut Visibility, Without<SkelBody>>,
    hidden: Query<(), With<HiddenPart>>,
) {
    let on = enabled();
    let Some(b) = body.0.as_ref() else {
        MIRROR_ON.store(false, std::sync::atomic::Ordering::Relaxed);
        return;
    };
    let jc = crate::travel::jc_mesh();
    let wanted = on && body_for(&jc).is_some();
    // Off (or a body that has none): back to the vertex mesh.
    for (e, s, ..) in &skels {
        if e != b.root || !wanted {
            commands.entity(s.root).despawn();
            for m in &s.meshes {
                commands.entity(*m).despawn();
            }
            commands.entity(e).remove::<(SkelBody, MirrorSkel)>();
            if let Ok(ch) = kids.get(e) {
                for c in ch.iter() {
                    if hidden.contains(c) {
                        if let Ok(mut v) = vis.get_mut(c) {
                            *v = Visibility::Inherited;
                        }
                        commands.entity(c).remove::<HiddenPart>();
                    }
                }
            }
        }
    }
    if !wanted {
        MIRROR_ON.store(false, std::sync::atomic::Ordering::Relaxed);
        return;
    }
    let dt = time.delta_secs().min(0.1);
    if let Ok((_, mut s, gt, w)) = skels.get_mut(b.root) {
        MIRROR_ON.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some((seq, start, frames, f, looped)) = b.cur.clone() {
            advance(&mut s, &seq, start, frames, f, looped, dt);
            weapon_fix(&mut s, &b.mesh, start, frames, f);
            if crate::bodyfx::hurt_on() {
                apply_hurt(&mut s, &crate::bodyfx::hurt_fracs(&ph.parts, &[100.0; 6]));
            }
            if cloth_on() {
                if seq.starts_with("Sit") {
                    cloth_rest(&mut s);
                } else {
                    cloth(&mut s, gt.to_matrix(), dt);
                }
            }
            write_bones(&s, &mut tfs);
            if let Ok(mut g) = MIRROR_WEAPON.lock() {
                *g = s.weapon;
            }
            if let Some(mut w) = w {
                face_weights(&mut s, &b.mesh, None, false, dt, w.weights_mut());
            }
        }
        // Keep the vertex parts hidden.
        if let Ok(ch) = kids.get(b.root) {
            for c in ch.iter() {
                if hidden.contains(c) {
                    if let Ok(mut v) = vis.get_mut(c) {
                        if *v != Visibility::Hidden {
                            *v = Visibility::Hidden;
                        }
                    }
                }
            }
        }
        s.light_in -= dt;
        if s.light_in <= 0.0 {
            s.light_in = 0.25;
            let ue = dx_level::bevy_to_ue(gt.translation().to_array());
            relight(&mut s, &level.0, ue, 1.0, &mut mats);
        }
        return;
    }
    // Build it once the vertex parts are there: their textures by slot.
    let Some(sb) = body_for(&jc) else { return };
    let Ok(ch) = kids.get(b.root) else { return };
    let mut slot_tex: HashMap<u32, Option<Handle<Image>>> = HashMap::new();
    let mut found = false;
    for c in ch.iter() {
        let Ok((m3, mm)) = parts.get(c) else { continue };
        let Some(pi) = b.handles.iter().position(|h| *h == m3.0) else { continue };
        found = true;
        let tex = std_mats.get(&mm.0).and_then(|m| m.base_color_texture.clone());
        for sl in &b.parts[pi].slots {
            slot_tex.entry(*sl).or_insert(tex.clone());
        }
        if let Ok(mut v) = vis.get_mut(c) {
            *v = Visibility::Hidden;
        }
        commands.entity(c).insert(HiddenPart);
    }
    if !found {
        return;
    }
    let s = build(
        &mut commands,
        &mut meshes,
        &mut mats,
        &mut ibms,
        sb,
        b.root,
        &slot_tex,
        &b.mesh,
        1.0,
        usize::MAX,
        Some(RenderLayers::layer(crate::mirror::BODY_LAYER)),
        &[],
        None,
    );
    commands.entity(b.root).insert((s, MirrorSkel));
}

/// Light a body from the sixteen strongest Unreal lights that reach `ue` (when what reaches it changed).
fn relight(s: &mut SkelBody, lvl: &dx_level::Level, ue: [f32; 3], scale_glow: f32, mats: &mut Assets<ActorMaterial>) {
    crate::dynlight::with_lights_at(lvl, ue, 40.0, |lights| relight_from(s, lights, ue, scale_glow, mats));
}

fn relight_from(s: &mut SkelBody, lights: &[dx_level::lighting::LightInfo], ue: [f32; 3], scale_glow: f32, mats: &mut Assets<ActorMaterial>) {
    let sig = crate::dynlight::light_sig(lights, [0.0; 3], 0) ^ ((scale_glow * 64.0) as u64);
    if sig == s.light_sig {
        return;
    }
    s.light_sig = sig;
    let mut best: Vec<(f32, &dx_level::lighting::LightInfo)> = lights
        .iter()
        .filter_map(|l| {
            let d = Vec3::from(l.pos).distance(Vec3::from(ue));
            let w = (1.0 - d / l.radius.max(1.0)) * l.color[0].max(l.color[1]).max(l.color[2]);
            (d < l.radius + 48.0).then_some((w, l))
        })
        .collect();
    best.sort_by(|x, y| y.0.total_cmp(&x.0));
    // The rest: an even fill at half strength (a sphere facing a light averages about half of N.L's peak).
    let mut amb = Vec3::ZERO;
    for (_, l) in best.iter().skip(16) {
        let d = Vec3::from(l.pos).distance(Vec3::from(ue));
        if d < l.radius {
            amb += Vec3::from(l.color) * (1.0 - d / l.radius) * 0.5;
        }
    }
    best.truncate(16);
    // The zone's ambient light, as every actor's light starts from it.
    amb += Vec3::from(dx_level::actors::zone_ambient_at(ue));
    let mut lt = ActorLights { misc: Vec4::new(best.len() as f32, dx_level::lighting::LIGHT_GAIN * scale_glow, 0.0, 0.0), ..default() };
    for (i, (_, l)) in best.iter().enumerate() {
        let bp = Vec3::from(dx_level::ue_to_bevy(l.pos));
        lt.pos[i] = bp.extend(l.radius * dx_level::METERS_PER_UNIT);
        lt.col[i] = Vec3::from(l.color).extend(0.0);
    }
    lt.amb = amb.extend(0.0);
    for h in &s.mats {
        let Some(cur) = mats.get(h) else { continue };
        let old = cur.extension.lights;
        let mut new = lt;
        // Per-material: unlit, the blink, the height cut.
        new.misc.z = old.misc.z;
        new.wet.x = old.wet.x;
        new.wet.y = old.wet.y;
        new.wet.z = old.wet.z;
        new.wet.w = old.wet.w;
        // Unchanged light (standing in the same lamps' reach): leave the material alone - a changed one is
        // re-prepared on the render thread.
        if new == old {
            continue;
        }
        if let Some(mut m) = mats.get_mut(h) {
            m.extension.lights = new;
        }
    }
}

// ------------------------------------------------------------------------------------------------ for bodyfx.rs

/// What building a body for JC from his mirror body needs.
#[derive(bevy::ecs::system::SystemParam)]
pub struct BuildParams<'w, 's> {
    meshes: ResMut<'w, Assets<Mesh>>,
    mats: ResMut<'w, Assets<ActorMaterial>>,
    ibms: ResMut<'w, Assets<SkinnedMeshInverseBindposes>>,
    std_mats: Res<'w, Assets<StandardMaterial>>,
    kids: Query<'w, 's, &'static Children>,
    parts: Query<'w, 's, (&'static Mesh3d, &'static MeshMaterial3d<StandardMaterial>)>,
}

impl BuildParams<'_, '_> {
    /// Light a body from the Unreal lights that reach `ue` (see `relight`), at most every 0.25 s.
    pub fn relight(&mut self, s: &mut SkelBody, lvl: &dx_level::Level, ue: [f32; 3], dt: f32) {
        s.light_in -= dt;
        if s.light_in <= 0.0 {
            s.light_in = 0.25;
            relight(s, lvl, ue, 1.0, &mut self.mats);
        }
    }

    /// `build_from_mirror` drawn in the world (and shadows), like the NPCs' bodies.
    pub fn build_from_mirror_world(&mut self, commands: &mut Commands, b: &crate::mirror::MirrorBody, body: &'static Body, parent: Entity) -> Option<SkelBody> {
        self.build_mirror(commands, b, body, parent, None, &[], None)
    }

    /// The first-person body: like `build_from_mirror` without the triangles on the `drop` bones or above `cut` of
    /// the body's standing height.
    pub fn build_from_mirror_without(&mut self, commands: &mut Commands, b: &crate::mirror::MirrorBody, body: &'static Body, parent: Entity, layers: RenderLayers, drop: &[&str], cut: f32) -> Option<SkelBody> {
        self.build_mirror(commands, b, body, parent, Some(layers), drop, Some(cut))
    }

    /// Draw this body's faces from the front only (the first-person body: from above, the inside of the coat at
    /// the cut would show; culled, the view goes through the opening to the legs and floor).
    pub fn cull_back(&mut self, s: &SkelBody) {
        for h in &s.mats {
            if let Some(mut m) = self.mats.get_mut(h) {
                // Their outside is the front face: culling Front (as before) drew only the inside, so looking down
                // showed the black inside of the coat.
                m.base.cull_mode = Some(bevy::render::render_resource::Face::Back);
                m.base.double_sided = false;
            }
        }
    }

    /// Nothing of this body drawn above world height `y` (None = all of it).
    pub fn set_cut(&mut self, s: &SkelBody, y: Option<f32>) {
        for h in &s.mats {
            if let Some(mut m) = self.mats.get_mut(h) {
                let (z, w) = y.map_or((0.0, 0.0), |y| (y, 1.0));
                if m.extension.lights.wet.z != z || m.extension.lights.wet.w != w {
                    m.extension.lights.wet.z = z;
                    m.extension.lights.wet.w = w;
                }
            }
        }
    }

    /// A rebuilt body under `parent` with the textures JC's mirror body wears (None until its parts exist).
    pub fn build_from_mirror(&mut self, commands: &mut Commands, b: &crate::mirror::MirrorBody, body: &'static Body, parent: Entity, layers: RenderLayers) -> Option<SkelBody> {
        self.build_mirror(commands, b, body, parent, Some(layers), &[], None)
    }

    fn build_mirror(&mut self, commands: &mut Commands, b: &crate::mirror::MirrorBody, body: &'static Body, parent: Entity, layers: Option<RenderLayers>, drop: &[&str], cut: Option<f32>) -> Option<SkelBody> {
        let ch = self.kids.get(b.root).ok()?;
        let mut slot_tex: HashMap<u32, Option<Handle<Image>>> = HashMap::new();
        for c in ch.iter() {
            let Ok((m3, mm)) = self.parts.get(c) else { continue };
            let Some(pi) = b.handles.iter().position(|h| *h == m3.0) else { continue };
            let tex = self.std_mats.get(&mm.0).and_then(|m| m.base_color_texture.clone());
            for sl in &b.parts[pi].slots {
                slot_tex.entry(*sl).or_insert(tex.clone());
            }
        }
        if slot_tex.is_empty() {
            return None;
        }
        Some(build(commands, &mut self.meshes, &mut self.mats, &mut self.ibms, body, parent, &slot_tex, &b.mesh, 1.0, usize::MAX, layers, drop, cut))
    }
}

pub fn advance_pub(s: &mut SkelBody, seq: &str, start: usize, frames: usize, f: f32, looped: bool, dt: f32) {
    advance(s, seq, start, frames, f, looped, dt);
}

/// Write the pose to the bones; bones whose names start with one of `hide` shrink to nothing (and so does what
/// hangs off them).
pub fn write_bones_pub<F: bevy::ecs::query::QueryFilter>(s: &SkelBody, tfs: &mut Query<&mut Transform, F>, hide: &[&str]) {
    for (i, n) in s.nodes.iter().enumerate() {
        if let Ok(mut t) = tfs.get_mut(*n) {
            let p = s.pose[i];
            t.translation = p.t;
            t.rotation = p.r;
            t.scale = if hide.iter().any(|h| s.rig.names[i].starts_with(h)) { Vec3::splat(1e-4) } else { p.s };
        }
    }
}

/// The relaxed standing pose (BreatheLight's first frame) of each rig: what a hurt limb sags toward.
fn relaxed(rig: &Rig, female: bool) -> &'static [Trs] {
    static R: OnceLock<[Vec<Trs>; 2]> = OnceLock::new();
    let both = R.get_or_init(|| {
        [false, true].map(|fem| {
            let r = &rigs()[fem as usize];
            let mut out = r.rest.clone();
            if let Some(c) = clip_for(r, "BreatheLight", fem) {
                sample(r, c, 0.0, false, &mut out);
            }
            out
        })
    });
    let _ = rig;
    &both[female as usize]
}

/// Hurt Poses: `frac` = parts' health 0..1 (head, torso, left arm, right arm, left leg, right leg). A badly hurt
/// arm hangs (its bones sag toward the relaxed pose); a hurt leg stiffens (the knee barely bends: a limp).
pub fn apply_hurt(s: &mut SkelBody, frac: &[f32; 6]) {
    let rel = relaxed(s.rig, s.body.female);
    let names = &s.rig.names;
    let blend = |bones: &[&str], w: f32, pose: &mut Vec<Trs>| {
        if w <= 0.0 {
            return;
        }
        for (i, n) in names.iter().enumerate() {
            if bones.iter().any(|b| n == b) {
                pose[i].r = pose[i].r.slerp(rel[i].r, w);
            }
        }
    };
    let arm = |f: f32| ((0.5 - f) / 0.5).clamp(0.0, 1.0) * 0.75;
    let leg = |f: f32| ((0.6 - f) / 0.6).clamp(0.0, 1.0) * 0.65;
    let mut pose = std::mem::take(&mut s.pose);
    blend(&["DEF-upper_arm.L", "DEF-upper_arm.L.001", "DEF-forearm.L", "DEF-forearm.L.001"], arm(frac[2]), &mut pose);
    blend(&["DEF-upper_arm.R", "DEF-upper_arm.R.001", "DEF-forearm.R", "DEF-forearm.R.001"], arm(frac[3]), &mut pose);
    blend(&["DEF-shin.L", "DEF-shin.L.001"], leg(frac[4]), &mut pose);
    blend(&["DEF-shin.R", "DEF-shin.R.001"], leg(frac[5]), &mut pose);
    s.pose = pose;
}

// ------------------------------------------------------------------------------------------------ coat cloth

/// The coat's bones per rig, in hierarchy order: (node, length in rig units).
fn coat_bones(rig: &Rig) -> &'static [(usize, f32)] {
    static C: OnceLock<[Vec<(usize, f32)>; 2]> = OnceLock::new();
    let both = C.get_or_init(|| {
        [false, true].map(|fem| {
            let r = &rigs()[fem as usize];
            let mut g: Vec<Mat4> = Vec::with_capacity(r.rest.len());
            for (i, t) in r.rest.iter().enumerate() {
                let m = t.mat();
                g.push(r.parent[i].map_or(m, |p| g[p] * m));
            }
            let mut out: Vec<(usize, f32)> = Vec::new();
            for (i, n) in r.names.iter().enumerate() {
                if !n.starts_with("DEF-coat") {
                    continue;
                }
                let child = (0..r.names.len()).find(|&c| r.parent[c] == Some(i));
                let len = match child {
                    Some(c) => g[c].w_axis.truncate().distance(g[i].w_axis.truncate()),
                    None => r.parent[i].and_then(|p| out.iter().find(|(n, _)| *n == p).map(|x| x.1)).unwrap_or(0.1),
                };
                out.push((i, len.max(0.02)));
            }
            out
        })
    });
    let _ = rig;
    &both[std::ptr::eq(rig, &rigs()[1]) as usize]
}

/// Cloth (enhancement "Coat Cloth"): each coat bone's tail is a point in world space on a damped spring toward
/// where the animation puts it, so the coat trails the body's motion and settles back; held to the bone's length,
/// pushed out of the legs, softly limited in how far it swings, with a light gust of wind; the bone turns to point at
/// it. Simulated in fixed 1/120 s substeps with the animated targets moving smoothly between frames (a body posed
/// every third frame, or a frame hitch, no longer jolts it). `pawn` = the pawn entity's world transform.
fn cloth(s: &mut SkelBody, pawn: Mat4, dt: f32) {
    const STEP: f32 = 1.0 / 120.0;
    /// Spring toward the animated pose (rad/s natural frequency ~ 2 pi x 2.6 Hz) and its damping per second.
    const K: f32 = 270.0;
    const DAMP: f32 = 7.0;
    /// Past SOFT the swing is pulled back harder; HARD it never passes (radians off the animated direction).
    const SOFT: f32 = 0.3;
    const HARD: f32 = 0.75;
    let bones = coat_bones(s.rig);
    if bones.is_empty() || dt <= 0.0 {
        return;
    }
    let rig = s.rig;
    let world = pawn * s.root_local;
    let inv = world.inverse();
    let mut g = std::mem::take(&mut s.gmat);
    globals(rig, &s.pose, &mut g);
    // The legs, as capsules in world space (bone indices looked up once per rig).
    let at = |i: Option<usize>| i.map(|i| world.transform_point3(g[i].w_axis.truncate()));
    let legs: [Option<(Vec3, Vec3)>; 4] = leg_bones(rig).map(|(a, b)| Some((at(a)?, at(b)?)));
    let leg_r = 0.1 * world.x_axis.truncate().length();
    let fresh = s.cloth_tail.len() != bones.len();
    if fresh {
        s.cloth_tail = vec![Vec3::ZERO; bones.len()];
        s.cloth_prev = vec![Vec3::ZERO; bones.len()];
        s.cloth_head = vec![Vec3::ZERO; bones.len()];
        s.cloth_goal = vec![Vec3::ZERO; bones.len()];
        s.cloth_acc = 0.0;
    }
    let scale = world.x_axis.truncate().length();
    let gravity = Vec3::NEG_Y * (950.0 / 52.5) * 0.5;
    // Substeps this frame (the rest carries over).
    s.cloth_acc = (s.cloth_acc + dt.min(0.1)).min(0.1);
    let n = (s.cloth_acc / STEP).floor() as usize;
    s.cloth_acc -= n as f32 * STEP;
    s.cloth_clock += dt;
    let clock = s.cloth_clock;
    let decay = (1.0 - DAMP * STEP).max(0.0);
    let spring = K * STEP * STEP;
    for (k, &(i, len)) in bones.iter().enumerate() {
        let parent = rig.parent[i].map_or(Mat4::IDENTITY, |p| g[p]);
        let anim = parent * s.pose[i].mat();
        let head_r = anim.w_axis.truncate();
        let dir_r = anim.y_axis.truncate().normalize_or(Vec3::NEG_Y);
        let head_w = world.transform_point3(head_r);
        let target_w = world.transform_point3(head_r + dir_r * len);
        let len_w = len * scale;
        let jumped = s.cloth_tail[k].distance(target_w) > len_w * 4.0 + 1.0;
        if fresh || jumped {
            // First frame, or a jump (teleport, respawn): start where the animation is.
            s.cloth_tail[k] = target_w;
            s.cloth_prev[k] = target_w;
            s.cloth_head[k] = head_w;
            s.cloth_goal[k] = target_w;
            g[i] = anim;
            continue;
        }
        let (head0, goal0) = (s.cloth_head[k], s.cloth_goal[k]);
        // A gentle gust across the coat (each bone a little out of step), mostly sideways to the body.
        let side = world.z_axis.truncate().normalize_or(Vec3::X);
        let ph = k as f32 * 0.7;
        let gust = (clock * 1.3 + ph).sin() * 0.5 + (clock * 2.9 + ph * 1.7).sin() * 0.25;
        let wind = side * gust * 2.5 + Vec3::Y * (clock * 2.1 + ph).sin() * 0.6;
        let mut tail = s.cloth_tail[k];
        let mut prev = s.cloth_prev[k];
        for j in 0..n {
            let u = (j + 1) as f32 / n as f32;
            let head = head0.lerp(head_w, u);
            let goal = goal0.lerp(target_w, u);
            let vel = (tail - prev) * decay;
            let mut next = tail + vel + (goal - tail) * spring + (gravity + wind) * STEP * STEP;
            // Hold the length.
            next = head + (next - head).normalize_or(goal - head) * len_w;
            // Out of the legs.
            for (a, b) in legs.iter().flatten() {
                let ab = *b - *a;
                let t = ((next - *a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
                let c = *a + ab * t;
                let d = next - c;
                if d.length() < leg_r {
                    next = c + d.normalize_or(Vec3::Z) * leg_r;
                }
            }
            // A soft limit on the swing: past SOFT half the excess is taken back each step, never past HARD.
            let a_dir = (goal - head).normalize_or(Vec3::NEG_Y);
            let n_dir = (next - head).normalize_or(a_dir);
            let ang = a_dir.angle_between(n_dir);
            if ang > SOFT {
                let keep = (SOFT + (ang - SOFT) * 0.5).min(HARD);
                let axis = a_dir.cross(n_dir).normalize_or(Vec3::X);
                next = head + Quat::from_axis_angle(axis, keep) * a_dir * len_w;
            }
            prev = tail;
            tail = next;
        }
        s.cloth_tail[k] = tail;
        s.cloth_prev[k] = prev;
        s.cloth_head[k] = head_w;
        s.cloth_goal[k] = target_w;
        // Turn the bone toward its tail (in rig space).
        let want = (inv.transform_point3(tail) - head_r).normalize_or(dir_r);
        let (sc, rot, tr) = anim.to_scale_rotation_translation();
        let rot = Quat::from_rotation_arc(dir_r, want) * rot;
        g[i] = Mat4::from_scale_rotation_translation(sc, rot, tr);
        let local = parent.inverse() * g[i];
        let (ls, lr, lt) = local.to_scale_rotation_translation();
        s.pose[i] = Trs { t: lt, r: lr, s: ls };
    }
    s.gmat = g;
}

/// The coat hangs as animated (sitting: the chair would fight it) and starts afresh when it moves again.
fn cloth_rest(s: &mut SkelBody) {
    s.cloth_tail.clear();
}

/// The leg segments the coat is kept out of (thigh -> shin, shin -> foot, both sides), as rig node indices.
fn leg_bones(rig: &Rig) -> [(Option<usize>, Option<usize>); 4] {
    static C: OnceLock<[[(Option<usize>, Option<usize>); 4]; 2]> = OnceLock::new();
    let both = C.get_or_init(|| {
        rigs().each_ref().map(|r| {
            let at = |name: &str| r.names.iter().position(|n| n == name);
            [("DEF-thigh.L", "DEF-shin.L"), ("DEF-shin.L", "DEF-foot.L"), ("DEF-thigh.R", "DEF-shin.R"), ("DEF-shin.R", "DEF-foot.R")]
                .map(|(a, b)| (at(a), at(b)))
        })
    });
    if std::ptr::eq(rig, &rigs()[1]) { both[1] } else { both[0] }
}

pub fn cloth_on() -> bool {
    crate::settings::get("CoatCloth") > 0.5
}

