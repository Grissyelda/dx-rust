//! The outfits screen's ViewportWindow (Augmentique's PersonaScreenOutfits): JC seen from in front, rendered into
//! an image shown in the screen's 400 x 400 box, with the widgets that sit on it (the custom outfit's arrows, the
//! description) on a layer above. The view point is PersonaScreenOutfits.GetViewportLocation: 65 units ahead of
//! him along his facing, at eye height - 10 (Change Camera: - 55; crouched + 15 / - 15), looking back at him,
//! FOV 55. JC himself is the mirror body (mirror.rs), shown while this is open.

use crate::inventory_screen::{blank, upload};
use crate::uicanvas::Canvas;
use crate::classic_hud::HudLayer;
use bevy::camera::{visibility::RenderLayers, Exposure, RenderTarget};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use std::sync::Mutex;

/// What the screen wants this frame.
pub struct Request {
    /// The view's box in screen canvas units, the canvas origin / scale on the window, the window scale factor.
    pub rect: (i32, i32, i32, i32),
    pub ox: i32,
    pub oy: i32,
    pub d: f32,
    pub sf: f32,
    pub low_camera: bool,
    pub overlay: Option<Canvas>,
}

static REQ: Mutex<Option<Request>> = Mutex::new(None);
static OPEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The outfits screen is showing JC (mirror.rs shows his body).
pub fn open() -> bool {
    OPEN.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn request(r: Option<Request>) {
    if let Ok(mut g) = REQ.lock() {
        *g = r;
    }
}

#[derive(Component)]
pub struct OutfitCam;

#[derive(Default)]
pub struct State {
    img: Option<Handle<Image>>,
    cams: Vec<Entity>,
    node: Option<Entity>,
    overlay: Option<(Handle<Image>, Entity, Vec<u8>, Handle<HudLayer>)>,
}

const SIZE: u32 = 400;

#[allow(clippy::too_many_arguments)]
pub fn outfit_view_system(
    mut commands: Commands,
    mut st: Local<State>,
    mut images: ResMut<Assets<Image>>,
    mut layers: ResMut<Assets<HudLayer>>,
    player: Res<crate::Player>,
    cams: Query<&crate::FlyCam, Without<OutfitCam>>,
    mut q: Query<(&mut Transform, &mut Camera), With<OutfitCam>>,
    mut nodes: Query<(&mut Node, &mut Visibility)>,
) {
    let req = REQ.lock().ok().and_then(|mut r| r.take());
    let on = req.is_some();
    OPEN.store(on, std::sync::atomic::Ordering::Relaxed);
    let Some(req) = req else {
        for &c in &st.cams {
            if let Ok((_, mut cam)) = q.get_mut(c) {
                cam.is_active = false;
            }
        }
        for e in st.node.iter().copied().chain(st.overlay.as_ref().map(|o| o.1)).collect::<Vec<_>>() {
            if let Ok((_, mut v)) = nodes.get_mut(e) {
                *v = Visibility::Hidden;
            }
        }
        return;
    };
    // Built once: the image, a sky-room camera and a world + JC camera drawing into it, the image node and the
    // overlay layer.
    if st.img.is_none() {
        let img = images.add(Image::new_target_texture(SIZE, SIZE, bevy::render::render_resource::TextureFormat::Bgra8UnormSrgb, None));
        let proj = || {
            Projection::Perspective(PerspectiveProjection { fov: 55f32.to_radians(), near: 4.0 * dx_level::METERS_PER_UNIT, far: 4000.0, ..default() })
        };
        let sky = commands
            .spawn((
                Camera3d::default(),
                Camera { order: -40, clear_color: ClearColorConfig::Custom(Color::BLACK), is_active: false, ..default() },
                RenderTarget::Image(img.clone().into()),
                proj(),
                Transform::default(),
                RenderLayers::layer(1),
                Tonemapping::None,
                Exposure { ev100: -0.263 },
                OutfitCam,
            ))
            .id();
        let world = commands
            .spawn((
                Camera3d::default(),
                Camera { order: -39, clear_color: ClearColorConfig::None, is_active: false, ..default() },
                RenderTarget::Image(img.clone().into()),
                proj(),
                Transform::default(),
                RenderLayers::from_layers(&[0, crate::mirror::BODY_LAYER]),
                Tonemapping::None,
                Exposure { ev100: -0.263 },
                OutfitCam,
            ))
            .id();
        let node = commands
            .spawn((
                ImageNode::new(img.clone()),
                Node { position_type: PositionType::Absolute, ..default() },
                Visibility::Hidden,
                Pickable::IGNORE,
                GlobalZIndex(41),
            ))
            .id();
        let oimg = images.add(blank(1, 1));
        let omat = layers.add(HudLayer::new(oimg.clone()));
        let onode = commands
            .spawn((
                MaterialNode(omat.clone()),
                Node { position_type: PositionType::Absolute, ..default() },
                Visibility::Hidden,
                Pickable::IGNORE,
                GlobalZIndex(42),
            ))
            .id();
        st.img = Some(img);
        st.cams = vec![sky, world];
        st.node = Some(node);
        st.overlay = Some((oimg, onode, Vec::new(), omat));
        return;
    }
    // GetViewportLocation / GetViewportRotation.
    let yaw = cams.iter().next().map_or(0.0, |c| c.yaw);
    let ue_yaw = -(yaw as f64) - std::f64::consts::FRAC_PI_2;
    let dir = glam::DVec3::new(ue_yaw.cos(), ue_yaw.sin(), 0.0);
    let crouched = player.crouched;
    let mut loc = player.body.location + dir * 65.0;
    loc.z = player.body.location.z
        + match (crouched, req.low_camera) {
            (true, true) => player.eye_height - 15.0,
            (true, false) => player.eye_height + 15.0,
            (false, true) => player.eye_height - 55.0,
            (false, false) => player.eye_height - 10.0,
        };
    let eye = Vec3::from(dx_level::ue_to_bevy(loc.as_vec3().to_array()));
    let back = Vec3::from(dx_level::ue_to_bevy((-dir).as_vec3().to_array()));
    let tf = Transform::from_translation(eye).looking_to(back, Vec3::Y);
    for &c in &st.cams {
        if let Ok((mut t, mut cam)) = q.get_mut(c) {
            *t = tf;
            cam.is_active = true;
        }
    }
    let (x, y, w, h) = req.rect;
    if let Some(n) = st.node {
        if let Ok((mut node, mut v)) = nodes.get_mut(n) {
            *v = Visibility::Inherited;
            node.left = Val::Px((req.ox as f32 + x as f32 * req.d) / req.sf);
            node.top = Val::Px((req.oy as f32 + y as f32 * req.d) / req.sf);
            node.width = Val::Px(w as f32 * req.d / req.sf);
            node.height = Val::Px(h as f32 * req.d / req.sf);
        }
    }
    if let (Some((oimg, onode, last, omat)), Some(c)) = (st.overlay.as_mut(), req.overlay.as_ref()) {
        crate::hud::set_filter(&mut images, oimg, c.s, req.d);
        if upload(&mut images, oimg, last, c) {
            // A resized image: the material re-reads it.
            if let Some(m) = layers.get_mut(&*omat) {
                m.into_inner();
            }
        }
        if let Ok((mut node, mut v)) = nodes.get_mut(*onode) {
            *v = Visibility::Inherited;
            node.left = Val::Px(req.ox as f32 / req.sf);
            node.top = Val::Px(req.oy as f32 / req.sf);
            node.width = Val::Px(c.w as f32 * req.d / req.sf);
            node.height = Val::Px(c.h as f32 * req.d / req.sf);
        }
    }
}
