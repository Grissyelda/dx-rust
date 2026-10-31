//! Enhancement: alert indicators (setting AlertIndicators, AI group, off by default).
//!
//! A small marker over the head of a guard hostile to the player, while it can be seen: an eye filling up as
//! the guard grows suspicious (its readiness to attack), a question mark while it searches, an exclamation mark
//! in a fight. Our own art in the original HUD's manner - thin outlined glyphs in the HUD theme's text colour,
//! amber for searching, red for a fight - drawn once per theme into small nearest-filtered images.

use crate::ai::{Npc, NpcState, Target};
use crate::{FlyCam, LevelRes};
use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use glam::DVec3;
use std::collections::HashMap;

/// Icon size in pixels (drawn at 1x; shown at the HUD's scale).
const S: usize = 16;
/// Gauge steps (0..=GAUGE) + question + exclamation.
const GAUGE: usize = 8;
const QUESTION: usize = GAUGE + 1;
const BANG: usize = GAUGE + 2;

#[derive(Component)]
pub struct AlertMark;

#[derive(Default)]
pub struct AlertUi {
    icons: Vec<Handle<Image>>,
    theme: [u8; 3],
    /// Pawn entity -> (marker node, icon shown).
    marks: HashMap<Entity, (Entity, usize)>,
    /// Pawn entity -> (seconds to the next line-of-sight check, last answer).
    los: HashMap<Entity, (f32, bool)>,
    /// Eased fill of each pawn's gauge (so it doesn't flicker as readiness ticks).
    fill: HashMap<Entity, f32>,
}

fn put(px: &mut [[u8; 4]], x: i32, y: i32, c: [u8; 4]) {
    if x >= 0 && y >= 0 && (x as usize) < S && (y as usize) < S {
        px[y as usize * S + x as usize] = c;
    }
}

/// Paints `mask` (1 = colour) with a one-pixel dark outline around it.
fn stamp(px: &mut [[u8; 4]], mask: &[[bool; S]; S], col: [u8; 4]) {
    for y in 0..S as i32 {
        for x in 0..S as i32 {
            if mask[y as usize][x as usize] {
                continue;
            }
            let near = (-1..=1).any(|dy| {
                (-1..=1).any(|dx| {
                    let (u, v) = (x + dx, y + dy);
                    u >= 0 && v >= 0 && (u as usize) < S && (v as usize) < S && mask[v as usize][u as usize]
                })
            });
            if near {
                put(px, x, y, [0, 0, 0, 200]);
            }
        }
    }
    for y in 0..S {
        for x in 0..S {
            if mask[y][x] {
                px[y * S + x] = col;
            }
        }
    }
}

/// The eye: a lens outline 14 wide, 8 high, with a pupil; `fill` 0..1 floods it from the bottom.
fn eye(level: f32, col: [u8; 3]) -> Vec<[u8; 4]> {
    let mut px = vec![[0u8; 4]; S * S];
    let (cx, cy) = (7.5f32, 7.5f32);
    let mut outline = [[false; S]; S];
    let mut inside = [[false; S]; S];
    for y in 0..S {
        for x in 0..S {
            let (dx, dy) = ((x as f32 + 0.5 - cx) / 7.0, (y as f32 + 0.5 - cy) / 4.2);
            // Lens: two arcs meeting at the corners.
            let lens = dy.abs() <= (1.0 - dx * dx).max(0.0);
            let inner = (dy.abs() * 4.2 + 1.0) / 4.2 <= (1.0 - (dx * 7.0 / 5.8).powi(2)).max(0.0);
            if lens && !inner {
                outline[y][x] = true;
            } else if inner {
                inside[y][x] = true;
            }
        }
    }
    // Fill rising from the bottom of the lens (the pupil always shows).
    let top = cy + 4.2 - level * 8.4;
    let warm = [
        (col[0] as f32 + (255.0 - col[0] as f32) * level) as u8,
        (col[1] as f32 * (1.0 - level * 0.35)) as u8,
        (col[2] as f32 * (1.0 - level)) as u8,
    ];
    let mut fillm = [[false; S]; S];
    for y in 0..S {
        for x in 0..S {
            let pupil = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt() <= 1.6;
            if inside[y][x] && (y as f32 + 0.5 >= top || pupil) {
                fillm[y][x] = true;
            }
        }
    }
    stamp(&mut px, &outline, [col[0], col[1], col[2], 255]);
    for y in 0..S {
        for x in 0..S {
            if fillm[y][x] {
                px[y * S + x] = [warm[0], warm[1], warm[2], 230];
            }
        }
    }
    px
}

/// 5 x 9 glyphs (? and !), stretched to 10 x 14 pixels.
const Q: [&str; 9] = [".###.", "#...#", "....#", "...#.", "..#..", "..#..", ".....", "..#..", "..#.."];
const B: [&str; 9] = ["..#..", "..#..", "..#..", "..#..", "..#..", "..#..", ".....", "..#..", "..#.."];

fn glyph(rows: &[&str; 9], col: [u8; 3]) -> Vec<[u8; 4]> {
    let mut px = vec![[0u8; 4]; S * S];
    let mut m = [[false; S]; S];
    for y in 0..14 {
        let row = rows[y * 9 / 14].as_bytes();
        for x in 0..10 {
            if row[x / 2] == b'#' {
                m[1 + y][3 + x] = true;
            }
        }
    }
    stamp(&mut px, &m, [col[0], col[1], col[2], 255]);
    px
}

fn image(px: Vec<[u8; 4]>) -> Image {
    let mut img = Image::new(
        Extent3d { width: S as u32, height: S as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        px.into_iter().flatten().collect(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::nearest());
    img
}

#[allow(clippy::too_many_arguments)]
pub fn alert_ui_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    npcs: Query<(Entity, &Npc)>,
    blockers: Res<crate::actors::Blockers>,
    cam: Query<(&Camera, &GlobalTransform), With<FlyCam>>,
    window: Single<&Window>,
    mut images: ResMut<Assets<Image>>,
    mut st: Local<AlertUi>,
    mut nodes: Query<(&mut Node, &mut Visibility, &mut ImageNode), With<AlertMark>>,
) {
    let _prof = crate::prof::scope("alertui::alert_ui_system");
    let on = crate::smartai::indicators()
        && !crate::consys::third_person()
        && !crate::menus::open()
        && !crate::computers::open()
        && !crate::inventory_screen::screen_open();
    // Pawns gone (or a new map): drop their markers.
    let alive: std::collections::HashSet<Entity> = npcs.iter().map(|(e, _)| e).collect();
    let gone: Vec<Entity> = st.marks.keys().filter(|e| !alive.contains(e)).copied().collect();
    for e in gone {
        if let Some((m, _)) = st.marks.remove(&e) {
            commands.entity(m).despawn();
        }
        st.los.remove(&e);
        st.fill.remove(&e);
    }
    let theme = crate::menuui::h_col(9);
    if st.icons.is_empty() || st.theme != theme {
        st.theme = theme;
        let amber = [255, 196, 64];
        let red = [255, 60, 48];
        let mut icons: Vec<Handle<Image>> = (0..=GAUGE).map(|k| images.add(image(eye(k as f32 / GAUGE as f32, theme)))).collect();
        icons.push(images.add(image(glyph(&Q, amber))));
        icons.push(images.add(image(glyph(&B, red))));
        st.icons = icons;
        for (_, (_, shown)) in st.marks.iter_mut() {
            *shown = usize::MAX;
        }
    }
    let Ok((camera, cam_gt)) = cam.single() else { return };
    let eye_ue = {
        let e = dx_level::bevy_to_ue(cam_gt.translation().to_array());
        DVec3::new(e[0] as f64, e[1] as f64, e[2] as f64)
    };
    let scale = crate::hud::ui_scale(&window) / window.scale_factor();
    let size = S as f32 * scale;
    let dt = time.delta_secs();
    let lvl = &level.0;
    for (e, npc) in &npcs {
        let c = blockers.list.get(npc.blocker).copied();
        let mut want: Option<usize> = None;
        if on && npc.state != NpcState::Dead && npc.hates("Player") {
            if let Some(c) = c.filter(|c| c.radius > 0.0) {
                let fill = st.fill.entry(e).or_insert(0.0);
                *fill += (npc.readiness.clamp(0.0, 1.0) - *fill) * (dt * 8.0).min(1.0);
                let fill = *fill;
                want = if matches!(npc.state, NpcState::Attacking | NpcState::Surprised(_) | NpcState::Alerting)
                    && npc.target == Some(Target::Player)
                {
                    Some(BANG)
                } else if npc.state == NpcState::Seeking {
                    Some(QUESTION)
                } else if fill > 0.04 {
                    Some(((fill * GAUGE as f32).ceil() as usize).clamp(1, GAUGE))
                } else {
                    None
                };
                let head = c.center + DVec3::new(0.0, 0.0, c.height + 16.0);
                if want.is_some() && head.distance(eye_ue) > 3000.0 {
                    want = None;
                }
                if want.is_some() {
                    // In sight (checked a few times a second).
                    let (t, seen) = st.los.entry(e).or_insert((0.0, false));
                    *t -= dt;
                    if *t <= 0.0 {
                        *t = 0.2;
                        *seen = lvl.collision.trace(eye_ue, head, DVec3::ZERO).fraction >= 0.999;
                    }
                    if !*seen {
                        want = None;
                    }
                }
                if let Some(k) = want {
                    let p = Vec3::from(dx_level::ue_to_bevy(head.as_vec3().to_array()));
                    match camera.world_to_viewport(cam_gt, p) {
                        Ok(v) => {
                            let icons = st.icons.clone();
                            let (m, shown) = *st.marks.entry(e).or_insert_with(|| {
                                let m = commands
                                    .spawn((
                                        AlertMark,
                                        ImageNode::new(icons[k].clone()),
                                        Node { position_type: PositionType::Absolute, ..default() },
                                        Visibility::Hidden,
                                        GlobalZIndex(8),
                                        Pickable::IGNORE,
                                    ))
                                    .id();
                                (m, usize::MAX)
                            });
                            if let Ok((mut n, mut vis, mut img)) = nodes.get_mut(m) {
                                n.left = Val::Px((v.x - size * 0.5).round());
                                n.top = Val::Px((v.y - size).round());
                                n.width = Val::Px(size);
                                n.height = Val::Px(size);
                                *vis = Visibility::Inherited;
                                if shown != k {
                                    img.image = icons[k].clone();
                                }
                            }
                            if let Some(entry) = st.marks.get_mut(&e) {
                                entry.1 = k;
                            }
                        }
                        Err(_) => want = None,
                    }
                }
            }
        }
        if want.is_none() {
            if let Some((m, _)) = st.marks.get(&e) {
                if let Ok((_, mut vis, _)) = nodes.get_mut(*m) {
                    *vis = Visibility::Hidden;
                }
            }
        }
    }
}
