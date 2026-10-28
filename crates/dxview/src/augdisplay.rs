//! AugmentationDisplayWindow's single-player parts that weren't anywhere yet:
//!
//! * The spy drone (AugDrone / DeusExPlayer.CreateDrone / MoveDrone / DroneExplode): activating builds a SpyDrone
//!   in front of the eyes (Speed = MaxSpeed = 3 x LevelValue); while it's up the movement keys fly it along the view
//!   (Jump / Duck up and down), the player stands frozen with nothing in hand, the view still turns; the drone's view
//!   is a window at the left (width/4 x height/4 at width/8 + margin, half height) under "Remote SpyDrone Active",
//!   with "BioElectric energy low!" under 20% energy. Fire blows it up (EMP, 5 x LevelValue damage over
//!   8 x LevelValue); turning the aug off just ends it. Reconstruction takes reconstructTime (30 s).
//! * The targeting aug (DrawTargetAugmentation, bTargetActive): TraceLOS 8000 units through actors only (pawns,
//!   decorations - walls don't stop it), a 0.5 s hold on a lost target; the same left box with the target's name
//!   (BindName for pawns, ItemName for decorations, "(Disabled)" robots) and range in feet above it, health inside
//!   (level 1: High / Medium / Low; 2+: percent, and per part for people), the weapon at level 3+, and at level 4 a
//!   zoomed view of the target (else black with "Image Not Available"); "* No Target *" / "* Scanning *" above the
//!   crosshair while nothing is targeted.

use crate::classic_hud::HudLayer;
use crate::uicanvas::{Canvas, HAlign, Style, UiFont, VAlign};
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use glam::DVec3;
use std::sync::{Arc, Mutex};

#[derive(Default, Clone)]
struct DroneState {
    on: bool,
    level_value: f32,
    last: f64,
    input: DVec3,
    now: f64,
}

static DRONE: Mutex<Option<DroneState>> = Mutex::new(None);

fn drone() -> DroneState {
    DRONE.lock().ok().and_then(|d| d.clone()).unwrap_or_default()
}
fn with_drone(f: impl FnOnce(&mut DroneState)) {
    if let Ok(mut d) = DRONE.lock() {
        f(d.get_or_insert_with(DroneState::default));
    }
}

/// AugDrone state Active: Err(seconds left) while the last drone is still being rebuilt.
pub fn drone_activate(level_value: f32) -> Result<(), i32> {
    let d = drone();
    const RECONSTRUCT: f64 = 30.0;
    if d.last > 0.0 && d.now - d.last < RECONSTRUCT {
        return Err((RECONSTRUCT - (d.now - d.last)) as i32);
    }
    with_drone(|d| {
        d.on = true;
        d.level_value = level_value;
    });
    Ok(())
}

/// AugDrone.Deactivate.
pub fn drone_deactivate() {
    with_drone(|d| {
        if d.on {
            d.last = d.now;
        }
        d.on = false;
    });
}

/// The drone is up: the movement keys are its.
pub fn drone_flying() -> bool {
    drone().on
}

pub fn drone_input(v: DVec3) {
    with_drone(|d| d.input = v);
}

pub fn reset_level() {
    with_drone(|d| {
        d.on = false;
        d.input = DVec3::ZERO;
    });
}

#[derive(Component)]
pub struct AugPanel;
#[derive(Component)]
pub struct AugView;
#[derive(Component)]
pub struct AugViewCam;
#[derive(Component)]
pub struct DroneBody;

#[derive(Default)]
pub struct State {
    drone: Option<(Entity, DVec3, DVec3)>,
    target: Option<(Target, f64)>,
    image: Option<(Handle<Image>, UVec2)>,
    font: Option<Option<Arc<UiFont>>>,
    last_panel: u64,
    panel_img: Option<(Handle<Image>, (i32, i32, i32))>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Target {
    Pawn(usize),
    Deco(usize),
}

fn image(images: &mut Assets<Image>, size: UVec2) -> Handle<Image> {
    let mut im = Image::new_uninit(
        bevy::render::render_resource::Extent3d { width: size.x.max(1), height: size.y.max(1), depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        bevy::render::render_resource::TextureFormat::Bgra8UnormSrgb,
        bevy::asset::RenderAssetUsages::MAIN_WORLD | bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    im.texture_descriptor.usage |= bevy::render::render_resource::TextureUsages::TEXTURE_BINDING
        | bevy::render::render_resource::TextureUsages::COPY_DST
        | bevy::render::render_resource::TextureUsages::RENDER_ATTACHMENT;
    images.add(im)
}

fn ray_cyl(c: &dx_level::collision::Cylinder, o: DVec3, d: DVec3, max: f64) -> Option<f64> {
    dx_level::collision::trace_cylinder(c, o, o + d * max, DVec3::ZERO).map(|h| h.fraction * max)
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn augdisplay_system(
    mut commands: Commands,
    time: Res<Time>,
    (windows, level, player, ph): (Query<&Window>, Res<crate::LevelRes>, Res<crate::Player>, Res<crate::ai::PlayerHealth>),
    (blockers, carrying, binds): (Res<crate::actors::Blockers>, Res<crate::carry::Carrying>, Res<crate::keybinds::Bindings>),
    mut weapons: ResMut<crate::weapons::Weapons>,
    mut booms: ResMut<crate::explosion::Explosions>,
    npcs: Query<&crate::ai::Npc>,
    main: Query<(&Transform, &Projection), (With<crate::FlyCam>, Without<crate::SkyCam>, Without<AugViewCam>, Without<DroneBody>)>,
    mut view_cam: Query<(Entity, &mut Camera, &mut Transform, &mut RenderTarget), (With<AugViewCam>, Without<DroneBody>)>,
    mut body_tf: Query<&mut Transform, (With<DroneBody>, Without<AugViewCam>)>,
    mut nodes: Query<(Entity, &mut Node, &mut Visibility, Option<&mut ImageNode>), Or<(With<AugPanel>, With<AugView>)>>,
    (mut assets, mut images, mut meshes, mut materials, mut layers): (
        ResMut<crate::weapons::RuntimeAssets>,
        ResMut<Assets<Image>>,
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
        ResMut<Assets<HudLayer>>,
    ),
    mut map_gen: Local<u64>,
    mut st: Local<State>,
) {
    if crate::loader::new_map(&mut map_gen) {
        let font = st.font.take();
        *st = State::default();
        st.font = font;
        reset_level();
    }
    let now = time.elapsed_secs_f64();
    with_drone(|d| d.now = now);
    let Some(win) = windows.iter().next() else { return };
    let Some((cam_tf, cam_proj)) = main.iter().next() else { return };
    let lvl = &level.0;
    let eye_b = dx_level::bevy_to_ue(cam_tf.translation.to_array());
    let eye = DVec3::new(eye_b[0] as f64, eye_b[1] as f64, eye_b[2] as f64);
    let f = cam_tf.forward().as_vec3();
    let view = DVec3::new(f.x as f64, f.z as f64, f.y as f64).normalize_or_zero();
    let busy = ph.dead || crate::menus::open() || crate::inventory_screen::screen_open() || crate::consys::third_person();

    // ---- the spy drone
    let ds = drone();
    if ds.on && !ph.dead {
        if st.drone.is_none() {
            // CreateDrone: (2 + 13 + CollisionRadius) along the view, at eye height.
            let mut at = player.body.location + view * (2.0 + 13.0 + player.params.radius);
            at.z = player.body.location.z + (eye.z - player.body.location.z);
            let d = assets.classes.defaults("SpyDrone");
            let root = commands.spawn((Transform::from_translation(Vec3::from(dx_level::ue_to_bevy(at.as_vec3().to_array()))), Visibility::Inherited, DroneBody)).id();
            let map = |p: [f32; 3]| dx_level::ue_to_bevy(p);
            let _ = crate::weapons::spawn_mesh_parts("DeusExCharacters.SpyDrone", 1.0, &d, &mut assets, &mut images, &mut meshes, &mut materials, &mut commands, root, &map, None);
            st.drone = Some((root, at, DVec3::ZERO));
        }
        // Nothing in hand while it flies (PutInHand(None)).
        if weapons.held.is_some() {
            weapons.held = None;
            weapons.pending = None;
        }
        let max_speed = 3.0 * ds.level_value as f64;
        if let Some((e, pos, vel)) = st.drone.as_mut() {
            let dt = time.delta_secs_f64().min(0.1);
            if ds.input.length_squared() == 0.0 {
                *vel *= 0.9;
            } else {
                *vel += ds.input * (dt * max_speed);
            }
            vel.z += dt * (now * 2.0).sin();
            if vel.length() > max_speed {
                *vel = vel.normalize() * max_speed;
            }
            let ext = DVec3::new(13.0, 13.0, 2.76);
            let end = *pos + *vel * dt;
            let h = lvl.collision.trace(*pos, end, ext);
            if h.fraction < 1.0 {
                *pos += (end - *pos) * (h.fraction - 0.01).max(0.0);
                *vel -= h.normal * vel.dot(h.normal);
            } else {
                *pos = end;
            }
            if let Ok(mut t) = body_tf.get_mut(*e) {
                t.translation = Vec3::from(dx_level::ue_to_bevy(pos.as_vec3().to_array()));
                t.rotation = cam_tf.rotation;
            }
            crate::sound::hold(0x4452_4F4E_0000, "DeusExSounds.Augmentation.AugDroneLoop", Some(*pos), 192.0 / 255.0, 25.0 * 25.0, 1.0);
            // ParseLeftClick: fire detonates it.
            if !busy && binds.buttons.just_pressed(MouseButton::Left) {
                let lv = ds.level_value;
                booms.0.push(crate::explosion::Boom {
                    pos: *pos,
                    damage: 5.0 * lv,
                    radius: 8.0 * lv as f64,
                    damage_type: "EMP",
                    from: None,
                    rings: true,
                    fireball: true,
                    fx: 3,
                });
                crate::sound::play("DeusExSounds.Generic.SmallExplosion2", *pos, 2.0, (8.0 * lv * 4.0).max(1024.0), 1.0);
                crate::abilities::deactivate_aug("AugDrone");
                drone_deactivate();
            }
        }
    }
    if !drone().on {
        if let Some((e, ..)) = st.drone.take() {
            commands.entity(e).despawn();
        }
    }

    // ---- the targeting aug
    let target_level = crate::abilities::aug_level("AugTarget");
    let target_on = target_level.is_some() && !busy;
    let mut target: Option<Target> = None;
    if target_on {
        let carried = carrying.held.as_ref().map(|h| h.blocker);
        let mut best = 8000.0;
        for n in npcs.iter() {
            if n.state == crate::ai::NpcState::Dead {
                continue;
            }
            if let Some(c) = blockers.list.get(n.blocker).filter(|c| c.radius > 0.0) {
                if let Some(d) = ray_cyl(c, eye, view, best) {
                    best = d;
                    target = Some(Target::Pawn(n.blocker));
                }
            }
        }
        for (&b, _) in blockers.deco.iter() {
            if Some(b) == carried {
                continue;
            }
            if let Some(c) = blockers.list.get(b).filter(|c| c.radius > 0.0) {
                if npcs.iter().any(|n| n.blocker == b) {
                    continue;
                }
                if let Some(d) = ray_cyl(c, eye, view, best) {
                    best = d;
                    target = Some(Target::Deco(b));
                }
            }
        }
        match target {
            Some(t) => st.target = Some((t, now)),
            None => {
                if let Some((t, at)) = st.target {
                    if now - at < 0.5 {
                        target = Some(t);
                    } else {
                        st.target = None;
                    }
                }
            }
        }
    } else {
        st.target = None;
    }

    // ---- the view window (drone or zoom)
    let s = crate::hud::ui_scale(win).max(1.0) as i32;
    let (uw, uh) = ((win.physical_width() as i32) / s, (win.physical_height() as i32) / s);
    let margin = 4;
    let (bw, bh) = (uw / 4, uh / 4);
    let (bcx, bcy) = (uw / 8 + margin, uh / 2);
    let (bx, by) = (bcx - bw / 2, bcy - bh / 2);
    let drone_view = st.drone.as_ref().map(|d| d.1);
    let zoom_view = match (target_on, target_level, target) {
        (true, Some(l), Some(t)) if l >= 3 && drone_view.is_none() => {
            let c = match t {
                Target::Pawn(b) | Target::Deco(b) => blockers.list.get(b).copied(),
            };
            c.map(|c| {
                // SetViewportLocation(target - (radius + height) x 1.5 x dir), watching the target.
                let mult = c.radius + c.height;
                let dir = (c.center - eye).normalize_or_zero();
                (c.center - dir * mult * 1.5, c.center)
            })
        }
        _ => None,
    };
    let want_view = drone_view.is_some() || zoom_view.is_some();
    let size = UVec2::new((bw * s) as u32, (bh * s) as u32);
    if want_view && st.image.as_ref().is_none_or(|i| i.1 != size) {
        let img = image(&mut images, size);
        st.image = Some((img, size));
    }
    if view_cam.is_empty() {
        commands.spawn((
            Camera3d::default(),
            Camera { order: -30, is_active: false, clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() },
            RenderTarget::default(),
            Projection::Perspective(PerspectiveProjection::default()),
            Transform::default(),
            bevy::core_pipeline::tonemapping::Tonemapping::None,
            bevy::camera::Exposure { ev100: -0.263 },
            AugViewCam,
        ));
    }
    for (_, mut cam, mut tf, mut target_rt) in &mut view_cam {
        cam.is_active = want_view;
        if !want_view {
            continue;
        }
        if let Some((img, _)) = &st.image {
            if !matches!(&*target_rt, RenderTarget::Image(t) if t.handle == *img) {
                *target_rt = img.clone().into();
            }
        }
        if let Some(p) = drone_view {
            *tf = Transform::from_translation(Vec3::from(dx_level::ue_to_bevy(p.as_vec3().to_array()))).with_rotation(cam_tf.rotation);
        } else if let Some((from, at)) = zoom_view {
            let fb = Vec3::from(dx_level::ue_to_bevy(from.as_vec3().to_array()));
            let ab = Vec3::from(dx_level::ue_to_bevy(at.as_vec3().to_array()));
            *tf = Transform::from_translation(fb).looking_at(ab, Vec3::Y);
        }
        if let Projection::Perspective(mp) = cam_proj {
            let _ = mp;
        }
    }

    // ---- the panel (box, texts) as a HUD layer, redrawn when it changes
    if st.font.is_none() {
        st.font = Some(crate::classic_hud::load_font(&mut assets, "FontMenuSmall_DS").map(Arc::new));
    }
    let font = st.font.clone().flatten();
    let mut lines_above: Vec<(String, [u8; 3])> = Vec::new();
    let mut lines_in: Vec<(String, [u8; 3])> = Vec::new();
    let mut lines_bottom: Vec<(String, [u8; 3])> = Vec::new();
    let mut no_image = false;
    let mut scanning: Option<String> = None;
    let header = crate::menuui::h_col(7);
    let mut show_box = false;
    if drone_view.is_some() {
        show_box = true;
        lines_above.push(("Remote SpyDrone Active".into(), header));
        if ph.energy / 100.0 < 0.2 {
            lines_in.push(("BioElectric energy low!".into(), [255, 0, 0]));
        }
    } else if target_on {
        match target {
            Some(t) => {
                show_box = true;
                let lvl_n = target_level.unwrap_or(0);
                let (name, col, health, parts, weapon, disabled, center) = match t {
                    Target::Pawn(b) => {
                        let n = npcs.iter().find(|n| n.blocker == b);
                        let a = n.map(|n| &lvl.actors[n.actor]);
                        let hostile = n.is_some_and(|n| n.hates("Player"));
                        let bind = n.map(|n| n.bind_name.clone()).unwrap_or_default();
                        let hb = n.map(|n| n.hp_body.clone());
                        let robot = hb.as_ref().is_some_and(|h| h.robot);
                        let animal = n.is_some_and(|n| n.animal);
                        let mult = hb.as_ref().map_or(1.0, |h| h.health / h.max_health.max(1.0));
                        let parts = (!robot && !animal).then(|| hb.as_ref().map(|h| {
                            let mut p = [0f32; 6];
                            for i in 0..6 {
                                p[i] = h.parts[i] / h.defaults[i].max(1.0);
                            }
                            p
                        })).flatten();
                        let w = n.and_then(|n| n.weapon.as_ref().map(|w| w.name.clone()));
                        let dis = robot && n.is_some_and(|n| n.emp_hp <= 0.0);
                        let _ = a;
                        (bind, if hostile { [255, 0, 0] } else { [0, 255, 0] }, mult, parts, Some(w), dis, blockers.list[b].center)
                    }
                    Target::Deco(b) => {
                        let a = &lvl.actors[blockers.list[b].actor];
                        let name = match unreal_pkg::props::find(&a.props, "ItemName") {
                            Some(unreal_pkg::props::Value::Str(s)) => s.clone(),
                            _ => a.class.clone(),
                        };
                        (name, [255, 255, 255], 1.0, None, None, false, blockers.list[b].center)
                    }
                };
                let range = (center - player.body.location).length() / 16.0;
                lines_above.push((format!("{name}{}", if disabled { " (Disabled)" } else { "" }), col));
                lines_above.push((format!("Range {} ft", range as i32), col));
                if lvl_n == 0 {
                    let (s, m) = if health >= 0.66 { ("High", 1.0) } else if health >= 0.33 { ("Medium", 0.5) } else { ("Low", 0.05) };
                    lines_in.push((format!("{s} health"), crate::uicanvas::color_scaled(m)));
                } else {
                    let c = crate::uicanvas::color_scaled(health);
                    match parts {
                        Some(p) => {
                            lines_in.push((format!("{}% Overall", (health * 100.0) as i32), c));
                            for (i, n) in ["Head", "Torso", "L Arm", "R Arm", "L Leg", "R Leg"].iter().enumerate() {
                                lines_in.push((format!("{}% {n}", (p[i] * 100.0) as i32), c));
                            }
                        }
                        None => lines_in.push((format!("{}% health", (health * 100.0) as i32), c)),
                    }
                }
                if lvl_n > 1 {
                    if let Some(w) = weapon {
                        lines_bottom.push((format!("Weapon: {}", w.unwrap_or_else(|| "None".into())), header));
                    }
                }
                no_image = lvl_n < 3;
            }
            None => {
                scanning = Some(if now % 1.5 > 0.75 { "* No Target *".into() } else { "* Scanning *".into() });
            }
        }
    }
    // Panel layout in UI units: a window from the screen's left edge, around the box.
    let lh = font.as_ref().map_or(10, |f| f.height);
    let top = by - margin - lh * lines_above.len() as i32 - 2;
    let (pw, ph_) = (bx + bw + 4, (by + bh + 4) - top);
    let mut hash: u64 = 1469598103934665603;
    let mut feed = |x: &str| {
        for b in x.bytes() {
            hash ^= b as u64;
            hash = hash.wrapping_mul(1099511628211);
        }
    };
    for (l, c) in lines_above.iter().chain(&lines_in).chain(&lines_bottom) {
        feed(l);
        feed(&format!("{c:?}"));
    }
    feed(&format!("{show_box}{no_image}{scanning:?}{uw}x{uh}{s}"));
    let changed = hash != st.last_panel;
    st.last_panel = hash;
    let any = show_box || scanning.is_some();
    // Panel and scanning are drawn on one layer covering the left box and the centre text.
    let (lx, ly, lw, lh_total) = (0, top.min(uh / 2 - 40), uw / 2 + 120, (by + bh + 4 - top.min(uh / 2 - 40)).max(1));
    let _ = (pw, ph_);
    if changed && any {
        if let Some(f) = font.as_ref() {
            let mut c = Canvas::new(lw, lh_total, s);
            let oy = -ly;
            if show_box {
                // DrawDropShadowBox: shadow right and below, a border round the box.
                let (x, y, w, h) = (bx, by + oy, bw, bh);
                c.fill(x, y + h + 1, w + 2, 1, Style::Normal, [0, 0, 0]);
                c.fill(x + w + 1, y, 1, h + 2, Style::Normal, [0, 0, 0]);
                let b = crate::menuui::h_col(1);
                c.fill(x - 1, y - 1, w + 2, 1, Style::Normal, b);
                c.fill(x - 1, y + h, w + 2, 1, Style::Normal, b);
                c.fill(x - 1, y - 1, 1, h + 2, Style::Normal, b);
                c.fill(x + w, y - 1, 1, h + 2, Style::Normal, b);
                if no_image {
                    c.fill(x, y, w, h, Style::Normal, [0, 0, 0]);
                    c.text(f, x, y + h / 2 - f.height / 2, w, f.height, HAlign::Center, VAlign::Top, [255, 255, 255], "Image Not Available");
                }
                let ay = y - margin - f.height * lines_above.len() as i32;
                for (i, (l, col)) in lines_above.iter().enumerate() {
                    let align = if drone_view.is_some() { HAlign::Center } else { HAlign::Left };
                    c.text(f, x + margin, ay + i as i32 * f.height, w - margin, f.height, align, VAlign::Top, *col, l);
                }
                for (i, (l, col)) in lines_in.iter().enumerate() {
                    let align = if drone_view.is_some() { HAlign::Center } else { HAlign::Left };
                    c.text(f, x + margin, y + margin + i as i32 * f.height, w - margin, f.height, align, VAlign::Top, *col, l);
                }
                for (i, (l, col)) in lines_bottom.iter().rev().enumerate() {
                    c.text(f, x + margin, y + h - margin - (i as i32 + 1) * f.height, w - margin, f.height, HAlign::Left, VAlign::Top, *col, l);
                }
            }
            if let Some(t) = &scanning {
                let w = f.width(t);
                c.text(f, uw / 2 - w / 2 - lx, uh / 2 - f.height - 20 + oy, w + 2, f.height, HAlign::Left, VAlign::Top, header, t);
            }
            let bytes = c.to_rgba8();
            let px = ((lw * s) as u32, (lh_total * s) as u32);
            let k = 1.0 / win.scale_factor();
            let have_node = nodes.iter().any(|n| n.3.is_none());
            let same = st.panel_img.as_ref().is_some_and(|p| p.1 == (lw, lh_total, s)) && have_node;
            if same {
                // Same size: new pixels into the same image.
                if let Some((h, _)) = &st.panel_img {
                    if let Some(mut im) = images.get_mut(h) {
                        im.data = Some(bytes);
                    }
                }
                for (_, mut node, mut vis, im) in nodes.iter_mut() {
                    if im.is_none() {
                        *vis = Visibility::Inherited;
                        node.top = Val::Px((ly * s) as f32 * k);
                    }
                }
            } else {
                let img = images.add(Image::new(
                    bevy::render::render_resource::Extent3d { width: px.0, height: px.1, depth_or_array_layers: 1 },
                    bevy::render::render_resource::TextureDimension::D2,
                    bytes,
                    bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
                    bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
                ));
                if let Some((old, _)) = st.panel_img.replace((img.clone(), (lw, lh_total, s))) {
                    images.remove(old.id());
                }
                for (e, _, _, im) in nodes.iter() {
                    if im.is_none() {
                        commands.entity(e).despawn();
                    }
                }
                commands.spawn((
                    AugPanel,
                    MaterialNode(layers.add(HudLayer::new(img))),
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px((lx * s) as f32 * k),
                        top: Val::Px((ly * s) as f32 * k),
                        width: Val::Px(px.0 as f32 * k),
                        height: Val::Px(px.1 as f32 * k),
                        ..default()
                    },
                    Visibility::Inherited,
                    GlobalZIndex(-3),
                ));
            }
        }
    }
    if !any {
        for (_, _, mut vis, im) in nodes.iter_mut() {
            if im.is_none() {
                *vis = Visibility::Hidden;
            }
        }
        st.last_panel = 0;
    }
    // The view image node.
    let mut have_view = false;
    for (e, mut node, mut vis, im) in nodes.iter_mut() {
        let Some(mut im) = im else { continue };
        have_view = true;
        if !want_view {
            *vis = Visibility::Hidden;
            continue;
        }
        *vis = Visibility::Inherited;
        let k = 1.0 / win.scale_factor();
        node.left = Val::Px((bx * s) as f32 * k);
        node.top = Val::Px((by * s) as f32 * k);
        node.width = Val::Px((bw * s) as f32 * k);
        node.height = Val::Px((bh * s) as f32 * k);
        if let Some((img, _)) = &st.image {
            if im.image != *img {
                im.image = img.clone();
            }
        }
        let _ = e;
    }
    if !have_view && want_view {
        if let Some((img, _)) = &st.image {
            let k = 1.0 / win.scale_factor();
            commands.spawn((
                AugView,
                ImageNode::new(img.clone()),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px((bx * s) as f32 * k),
                    top: Val::Px((by * s) as f32 * k),
                    width: Val::Px((bw * s) as f32 * k),
                    height: Val::Px((bh * s) as f32 * k),
                    ..default()
                },
                Visibility::Inherited,
                GlobalZIndex(-4),
            ));
        }
    }
}

/// Render layer of heat sources seen by the vision aug through walls.
pub const HEAT_LAYER: usize = 7;

#[derive(Component)]
pub struct HeatCam;
#[derive(Component)]
pub struct IrTint;

/// DrawVisionAugmentation, level 2 and up (visionLevel >= 1): the screen outside the middle box goes dark red
/// ("IRAmp Active" over the box) and heat sources - pawns and bodies - within LevelValue (320 / 800 units; 0 at
/// level 2 itself) are drawn over everything, walls or not (a camera of their own after the world, depth
/// cleared). Level 1 is the light-amplification box (charged.rs).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn vision_system(
    mut commands: Commands,
    windows: Query<&Window>,
    ph: Res<crate::ai::PlayerHealth>,
    blockers: Res<crate::actors::Blockers>,
    main: Query<(&Transform, &Projection), (With<crate::FlyCam>, Without<crate::SkyCam>, Without<HeatCam>)>,
    mut heat: Query<(&mut Camera, &mut Transform, &mut Projection), With<HeatCam>>,
    npcs: Query<(Entity, &crate::ai::Npc, Option<&Children>)>,
    parts: Query<(), With<Mesh3d>>,
    mut tint: Query<(&mut Node, &mut Visibility), With<IrTint>>,
    mut tagged: Local<std::collections::HashSet<Entity>>,
    mut map_gen: Local<u64>,
) {
    if crate::loader::new_map(&mut map_gen) {
        tagged.clear();
    }
    let level = crate::abilities::aug_level("AugVision");
    let on = level.is_some_and(|l| l >= 1) && !ph.dead && !crate::menus::open() && !crate::consys::third_person();
    let range = crate::abilities::aug("AugVision").unwrap_or(0.0) as f64;
    if heat.is_empty() {
        commands.spawn((
            Camera3d::default(),
            Camera { order: 2, is_active: false, clear_color: ClearColorConfig::None, ..default() },
            Projection::Perspective(PerspectiveProjection::default()),
            Transform::default(),
            bevy::core_pipeline::tonemapping::Tonemapping::None,
            bevy::camera::Exposure { ev100: -0.263 },
            bevy::camera::visibility::RenderLayers::layer(HEAT_LAYER),
            HeatCam,
        ));
    }
    let Some((mtf, mproj)) = main.iter().next() else { return };
    for (mut cam, mut tf, mut proj) in &mut heat {
        cam.is_active = on && range > 0.0;
        *tf = *mtf;
        *proj = mproj.clone();
    }
    // Who glows: pawns (alive or bodies) within range of the eye.
    let eye_b = dx_level::bevy_to_ue(mtf.translation.to_array());
    let eye = DVec3::new(eye_b[0] as f64, eye_b[1] as f64, eye_b[2] as f64);
    let mut want: std::collections::HashSet<Entity> = std::collections::HashSet::new();
    if on && range > 0.0 {
        for (_, n, kids) in npcs.iter() {
            if n.carried {
                continue;
            }
            let Some(c) = blockers.list.get(n.blocker) else { continue };
            let at = if c.radius > 0.0 { c.center } else { n.body.as_ref().map_or(c.center, |b| b.location) };
            if at.distance(eye) > range {
                continue;
            }
            if let Some(k) = kids {
                for e in k.iter().filter(|e| parts.get(*e).is_ok()) {
                    want.insert(e);
                }
            }
        }
    }
    for e in tagged.iter().filter(|e| !want.contains(e)) {
        if let Ok(mut ec) = commands.get_entity(*e) {
            ec.remove::<bevy::camera::visibility::RenderLayers>();
        }
    }
    for e in want.iter().filter(|e| !tagged.contains(e)) {
        if let Ok(mut ec) = commands.get_entity(*e) {
            ec.insert(bevy::camera::visibility::RenderLayers::from_layers(&[0, HEAT_LAYER]));
        }
    }
    *tagged = want;
    // The dark red outside the middle box (four bands).
    if tint.is_empty() {
        for _ in 0..4 {
            commands.spawn((
                IrTint,
                Node { position_type: PositionType::Absolute, ..default() },
                BackgroundColor(Color::srgba(0.35, 0.0, 0.0, 0.55)),
                Visibility::Hidden,
                GlobalZIndex(-6),
            ));
        }
    }
    let Some(win) = windows.iter().next() else { return };
    let (w, h) = (win.width(), win.height());
    let rects = [
        (0.0, 0.0, w, h / 4.0),
        (0.0, h * 0.75, w, h / 4.0),
        (0.0, h / 4.0, w / 4.0, h / 2.0),
        (w * 0.75, h / 4.0, w / 4.0, h / 2.0),
    ];
    for (i, (mut node, mut vis)) in tint.iter_mut().enumerate() {
        *vis = if on { Visibility::Inherited } else { Visibility::Hidden };
        if let Some(r) = rects.get(i) {
            node.left = Val::Px(r.0);
            node.top = Val::Px(r.1);
            node.width = Val::Px(r.2);
            node.height = Val::Px(r.3);
        }
    }
}

#[derive(Component)]
pub struct AugText;

/// AugmentationDisplayWindow's remaining labels, in FontMenuSmall_DS on one 1x layer over the screen:
/// DrawVisionAugmentation's "LightAmp Active" inside the middle box (tech goggles or the vision aug) and, from vision
/// level 2 on, "IRAmp Active" above it; DrawDefenseAugmentation's "* ADS Tracking *" / "* ADS Detonating *" with the
/// range in feet (and "BEHIND" when it's out of view) in red, centred over the projectile the defense aug is
/// tracking, and while it detonates a dotted red line from the screen centre that crawls toward it (Interpolate).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn aug_text_system(
    mut commands: Commands,
    time: Res<Time>,
    windows: Query<&Window>,
    ph: Res<crate::ai::PlayerHealth>,
    active: Res<crate::charged::ActiveCharged>,
    player: Res<crate::Player>,
    cam: Query<(&Camera, &GlobalTransform), (With<crate::FlyCam>, Without<crate::SkyCam>)>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut layers: ResMut<Assets<HudLayer>>,
    mut node: Query<(&mut Visibility, &MaterialNode<HudLayer>), With<AugText>>,
    mut st: Local<(Option<Option<Arc<UiFont>>>, u64, (i32, i32), Option<Handle<Image>>)>,
) {
    let Some(win) = windows.iter().next() else { return };
    let busy = ph.dead || crate::menus::open() || crate::inventory_screen::screen_open() || crate::consys::third_person() || crate::credits::open();
    let light_amp = !busy && (active.has("TechGoggles") || crate::abilities::aug_active("AugVision"));
    let ir = light_amp && crate::abilities::aug_level("AugVision").is_some_and(|l| l >= 1);
    let defense = if busy { None } else { crate::abilities::defense_target() };
    let any = light_amp || defense.is_some();
    if st.0.is_none() {
        st.0 = Some(crate::classic_hud::load_font(&mut assets, "FontMenuSmall_DS").map(Arc::new));
    }
    let s = crate::hud::ui_scale(win).max(1.0) as i32;
    let root = ((win.physical_width() as i32 / s).max(1), (win.physical_height() as i32 / s).max(1));
    let (Some(font), Ok((camera, gt))) = (st.0.clone().flatten(), cam.single()) else { return };
    // The canvas and its node (rebuilt when the screen size changes).
    if st.2 != root || st.3.is_none() {
        let mut img = Image::new(
            bevy::render::render_resource::Extent3d { width: root.0 as u32, height: root.1 as u32, depth_or_array_layers: 1 },
            bevy::render::render_resource::TextureDimension::D2,
            vec![0; (root.0 * root.1 * 4) as usize],
            bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
            bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
        );
        img.sampler = bevy::image::ImageSampler::nearest();
        let h = images.add(img);
        if node.is_empty() {
            commands.spawn((
                AugText,
                MaterialNode(layers.add(HudLayer::new(h.clone()))),
                Node { position_type: PositionType::Absolute, left: Val::Px(0.0), top: Val::Px(0.0), width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
                Visibility::Hidden,
                Pickable::IGNORE,
                GlobalZIndex(-3),
            ));
        } else {
            for (_, m) in &node {
                if let Some(mut l) = layers.get_mut(&m.0) {
                    l.tex = h.clone();
                }
            }
        }
        if let Some(old) = st.3.replace(h) {
            images.remove(old.id());
        }
        st.2 = root;
        st.1 = 0;
    }
    for (mut vis, _) in &mut node {
        let want = if any { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
    }
    if !any {
        st.1 = 0;
        return;
    }
    let (w, h) = root;
    let margin = 4;
    let header = crate::menuui::h_col(7);
    // ConvertVectorToCoordinates: the screen point (UI units), and whether it's in front.
    let ads = defense.map(|(pos, detonating)| {
        let p = Vec3::from(dx_level::ue_to_bevy(pos.as_vec3().to_array()));
        let in_front = (p - gt.translation()).dot(*gt.forward()) > 0.0;
        let sp = camera.world_to_viewport(gt, p).ok().map(|v| v * win.scale_factor() / s as f32);
        let (cx, cy) = sp.map_or((w as f32 / 2.0, h as f32 / 2.0), |v| (v.x, v.y));
        let range = (pos - player.body.location).length() / 16.0;
        (cx, cy, in_front && sp.is_some(), range as i32, detonating)
    });
    let line_t = (time.elapsed_secs_f64() % 0.5) as f32;
    let mut hash: u64 = 1469598103934665603;
    for b in format!("{light_amp}{ir}{ads:?}{w}x{h}{header:?}{}", if ads.is_some_and(|a| a.4) { line_t } else { 0.0 }).bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    if hash == st.1 {
        return;
    }
    st.1 = hash;
    let mut c = Canvas::new(w, h, 1);
    if light_amp {
        // The middle box: width/2 x height/2 at the centre.
        let (bx, by) = (w / 4, h / 4);
        c.text(&font, bx + margin, by + margin, font.width("LightAmp Active") + 2, font.height, HAlign::Left, VAlign::Top, header, "LightAmp Active");
        if ir {
            c.text(&font, bx + margin, by - margin - font.height, font.width("IRAmp Active") + 2, font.height, HAlign::Left, VAlign::Top, header, "IRAmp Active");
        }
    }
    if let Some((cx, cy, seen, range, detonating)) = ads {
        let first = if detonating { "* ADS Detonating *" } else { "* ADS Tracking *" };
        let second = format!("Range {range} ft{}", if seen { "" } else { " BEHIND" });
        let tw = font.width(first).max(font.width(&second));
        let th = font.height * 2;
        let (x, y) = (cx as i32 - tw / 2, cy as i32 - th);
        c.text(&font, x, y, tw, font.height, HAlign::Center, VAlign::Top, [255, 0, 0], first);
        c.text(&font, x, y + font.height, tw, font.height, HAlign::Center, VAlign::Top, [255, 0, 0], &second);
        if detonating {
            // Interpolate(gc, width/2, height/2, boxCX, boxCY, 64): halve the step until it's under 16 units.
            let (fx, fy) = (w as f32 / 2.0, h as f32 / 2.0);
            let (mut dx, mut dy) = (cx - fx, cy - fy);
            let (mut points, mut power) = (1, 64);
            while power >= 0 && (dx.abs() >= 16.0 || dy.abs() >= 16.0) {
                dx *= 0.5;
                dy *= 0.5;
                points *= 2;
                power -= 1;
            }
            let (mut px, mut py) = (fx + line_t * dx * 2.0, fy + line_t * dy * 2.0);
            for _ in 0..points - 1 {
                px += dx;
                py += dy;
                c.fill(px as i32, py as i32, 2, 2, Style::Normal, [255, 0, 0]);
            }
        }
    }
    if let Some(hd) = &st.3 {
        if let Some(mut im) = images.get_mut(hd) {
            im.data = Some(c.to_rgba8());
        }
    }
}
