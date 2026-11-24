//! The Nameless Mod's new-game screen shows the look being chosen in 3D (tnmMenuScreenNewGame's winDummy, a
//! ViewportWindow on a TrestkonDummy actor) where the portrait sits: the look's own mesh and skins (tnmlook.rs,
//! from the player's TNMGUI script), built on a render layer of its own, turning slowly, drawn by a camera into an
//! image laid over the portrait's box. Nothing is drawn from our side: the model is the player's own.
// uc: tnmMenuScreenNewGame.CreateDummy, tnmMenuScreenNewGame.RefreshPlayerModel

use bevy::camera::{visibility::RenderLayers, Exposure, RenderTarget};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use std::sync::Mutex;
use unreal_pkg::props::{Property, Value};

/// The preview's render layer (no other camera draws it).
pub const LAYER: usize = 27;
const W: u32 = 228;
const H: u32 = 322;
/// Where the model stands (far from any level, so nothing else is near it).
const SPOT: Vec3 = Vec3::new(0.0, -2000.0, 0.0);

/// The portrait's box on the window in physical pixels, this frame.
static REQ: Mutex<Option<(f32, f32, f32, f32)>> = Mutex::new(None);
/// The box in the new-game window's canvas units (draw_new_game), and whether the screen is the top one.
static BOX: Mutex<Option<(i32, i32, i32, i32)>> = Mutex::new(None);

pub fn set_box(r: Option<(i32, i32, i32, i32)>) {
    if let Ok(mut b) = BOX.lock() {
        *b = r;
    }
}
pub fn box_rect() -> Option<(i32, i32, i32, i32)> {
    BOX.lock().ok().and_then(|b| *b)
}

/// menus.rs: the new-game window is up at `pos` (UI units), shown at scale `d`, window scale factor `sf`.
pub fn request(r: Option<(f32, f32, f32, f32)>) {
    if let Ok(mut g) = REQ.lock() {
        *g = r;
    }
}

#[derive(Component)]
pub struct PreviewCam;
#[derive(Component)]
pub struct PreviewRoot;

#[derive(Default)]
pub struct State {
    img: Option<Handle<Image>>,
    cam: Option<Entity>,
    node: Option<Entity>,
    root: Option<Entity>,
    built: Option<(String, [Option<String>; 8])>,
    yaw: f32,
}

#[allow(clippy::too_many_arguments)]
pub fn newgame_view_system(
    mut commands: Commands,
    mut st: Local<State>,
    time: Res<Time<Real>>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut cams: Query<&mut Camera, With<PreviewCam>>,
    mut roots: Query<&mut Transform, With<PreviewRoot>>,
    mut nodes: Query<(&mut Node, &mut Visibility), Without<PreviewRoot>>,
) {
    let req = REQ.lock().ok().and_then(|mut r| r.take()).filter(|_| crate::tnmmode::active());
    let Some((x, y, w, h)) = req else {
        if let Some(c) = st.cam {
            if let Ok(mut cam) = cams.get_mut(c) {
                cam.is_active = false;
            }
        }
        if let Some(n) = st.node {
            if let Ok((_, mut v)) = nodes.get_mut(n) {
                v.set_if_neq(Visibility::Hidden);
            }
        }
        return;
    };
    if st.img.is_none() {
        let img = images.add(Image::new_target_texture(W, H, bevy::render::render_resource::TextureFormat::Bgra8UnormSrgb, None));
        let m = dx_level::METERS_PER_UNIT;
        // ViewportWindow on the dummy: from in front, a little above the waist, looking at him.
        let eye = SPOT + Vec3::new(0.0, 8.0 * m, 130.0 * m);
        let cam = commands
            .spawn((
                Camera3d::default(),
                Camera { order: -45, clear_color: ClearColorConfig::Custom(Color::NONE), is_active: false, ..default() },
                RenderTarget::Image(img.clone().into()),
                Projection::Perspective(PerspectiveProjection { fov: 40f32.to_radians(), near: 4.0 * m, far: 400.0 * m, ..default() }),
                Transform::from_translation(eye).looking_at(SPOT + Vec3::new(0.0, 4.0 * m, 0.0), Vec3::Y),
                RenderLayers::layer(LAYER),
                Tonemapping::None,
                Exposure { ev100: -0.263 },
                Msaa::Off,
                PreviewCam,
            ))
            .id();
        let node = commands
            .spawn((
                ImageNode::new(img.clone()),
                Node { position_type: PositionType::Absolute, ..default() },
                Visibility::Hidden,
                Pickable::IGNORE,
                GlobalZIndex(69),
            ))
            .id();
        st.img = Some(img);
        st.cam = Some(cam);
        st.node = Some(node);
        return;
    }
    // The look being chosen: its mesh and skins (rebuilt when they change).
    let look = crate::tnmlook::pending();
    let dressed = crate::tnmlook::dress_look(&mut assets.classes, look);
    if dressed.is_some() && dressed != st.built {
        if let Some(r) = st.root.take() {
            commands.entity(r).despawn();
        }
        if let Some((mesh, skins)) = dressed.clone() {
            let mut d: Vec<Property> = assets.classes.defaults(crate::tnmmode::player_class()).to_vec();
            d.retain(|p| !p.name.eq_ignore_ascii_case("MultiSkins") && !p.name.eq_ignore_ascii_case("Mesh"));
            for (i, s) in skins.iter().enumerate() {
                if let Some(s) = s {
                    d.push(Property { name: "MultiSkins".into(), index: i as u32, value: Value::Path(s.clone()) });
                }
            }
            d.push(Property { name: "Mesh".into(), index: 0, value: Value::Path(mesh.clone()) });
            let root = commands.spawn((Transform::from_translation(SPOT), Visibility::Visible, PreviewRoot, RenderLayers::layer(LAYER))).id();
            let map = |p: [f32; 3]| dx_level::ue_to_bevy(p);
            let built = crate::weapons::spawn_mesh_parts(
                &mesh,
                1.0,
                &d,
                &mut assets,
                &mut images,
                &mut meshes,
                &mut materials,
                &mut commands,
                root,
                &map,
                Some(RenderLayers::layer(LAYER)),
            );
            if built.is_none() {
                println!("TNM new game: no preview mesh {mesh}");
            }
            st.root = Some(root);
        }
        st.built = dressed;
    }
    // Turning slowly, as the dummy's RotationRate.
    st.yaw = (st.yaw + time.delta_secs() * 0.6) % std::f32::consts::TAU;
    if let Some(r) = st.root {
        if let Ok(mut t) = roots.get_mut(r) {
            t.rotation = Quat::from_rotation_y(st.yaw);
        }
    }
    if let Some(c) = st.cam {
        if let Ok(mut cam) = cams.get_mut(c) {
            cam.is_active = true;
        }
    }
    if let Some(n) = st.node {
        if let Ok((mut node, mut v)) = nodes.get_mut(n) {
            v.set_if_neq(Visibility::Inherited);
            let (l, t, ww, hh) = (Val::Px(x), Val::Px(y), Val::Px(w), Val::Px(h));
            if node.left != l || node.top != t || node.width != ww || node.height != hh {
                node.left = l;
                node.top = t;
                node.width = ww;
                node.height = hh;
            }
        }
    }
}
