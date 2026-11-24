//! Light and sound as cover between players (MULTIPLAYER roadmap Phase 2, Modern combat).
//!
//! What a player sees of a rival in the dark is what the renderer shows him; the game adds nothing that gives the
//! rival away. A rival is "made out" by the same light rule the guards use (stealth::sight_visibility on the light
//! that falls on him, his movement, the deep-shadow reach of Smarter AI, crouching): until then his name tag stays
//! off and the crosshair doesn't turn red on him. Thermoptic camo hides him outright.
//!
//! Sound: with the room's Sound Radar on (DmRules.radar), rivals' gunfire and running footsteps show as short arcs
//! round the crosshair pointing where they came from, brighter when close. Crouching, walking, quiet feet
//! (Infiltrator) and silenced or melee weapons stay off it.

use crate::classic_hud::HudLayer;
use crate::netplay::Rival;
use crate::uicanvas::Canvas;
use crate::{FlyCam, SkyCam};
use bevy::prelude::*;
use dx_level::bevy_to_ue;
use glam::DVec3;
use std::sync::Mutex;

/// The rivals this player can't make out right now (by id), refreshed every frame.
static HIDDEN: Mutex<Vec<u8>> = Mutex::new(Vec::new());

/// Always seen this close, whatever the light (a body brushing past).
const NEAR: f64 = 250.0;
/// How far a lit rival can be made out (as a guard's SightRadius).
const SIGHT: f64 = 3000.0;
const THRESHOLD: f32 = 0.05;

/// Light and sound as cover apply: a Modern versus match.
pub fn on() -> bool {
    crate::living::modern_match()
}

/// Can this player make out the rival from `eye`?
pub fn made_out(r: &Rival, eye: DVec3) -> bool {
    made_out_with(r, eye, crate::abilities::aug_active("AugVision"), crate::charged::goggles_on())
}

/// made_out with this player's own Vision augmentation and tech goggles (the counter table's answers to cloak).
pub fn made_out_with(r: &Rival, eye: DVec3, vision: bool, goggles: bool) -> bool {
    if !r.rival {
        return true;
    }
    let dist = (r.pos - eye).length();
    let cloaked = r.camo || r.augs.iter().any(|a| a == "AugCloak");
    if cloaked {
        // Counters: Vision and thermal goggles see a cloaked rival within their reach.
        let reach = |answer: &str, dflt: f32| crate::counters::value("AugCloak", answer, "reveal_range", dflt) as f64;
        let seen = (vision && dist <= reach("AugVision", 1200.0)) || (goggles && dist <= reach("TechGoggles", 640.0));
        if !seen {
            return false;
        }
    }
    if dist < NEAR {
        return true;
    }
    let speed = (r.vel.x * r.vel.x + r.vel.y * r.vel.y).sqrt();
    let sp = (speed / 320.0).min(1.5) as f32;
    let mut v = (r.light + 0.15 * sp * sp).clamp(0.0, 1.0);
    // Deep shadow: only within a reach that shrinks with the dark (smartai::player_vis, always on here).
    let shadow = (1.0 - r.light / 0.12).clamp(0.0, 1.0);
    if shadow > 0.0 {
        let moving = (speed / 320.0).clamp(0.0, 1.0);
        let mut reach = 160.0 + 440.0 * (1.0 - shadow as f64) + 200.0 * moving;
        if r.crouch {
            reach *= 0.6;
        }
        if dist > reach {
            v *= (1.0 - (dist - reach) / (reach * 0.5)).clamp(0.0, 1.0) as f32;
        }
    }
    crate::stealth::sight_visibility(v, dist, SIGHT, THRESHOLD) > 0.0
}

/// A rival by id this player can't make out (the HUD's crosshair colour asks).
pub fn hidden(id: u8) -> bool {
    on() && HIDDEN.lock().map(|h| h.contains(&id)).unwrap_or(false)
}

/// A rival by name this player can't make out (name tags).
pub fn hidden_name(name: &str) -> bool {
    on() && NAMES.lock().map(|h| h.iter().any(|n| n == name)).unwrap_or(false)
}

static NAMES: Mutex<Vec<String>> = Mutex::new(Vec::new());

// ---------------------------------------------------------------------------------------------------------------
// The sound radar.

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Step,
    Shot,
}

struct Blip {
    id: u8,
    kind: Kind,
    at: DVec3,
    /// 0..1 by distance (1 = right beside).
    loud: f32,
    age: f32,
}

/// Weapons that make no shot sound worth hearing across a room.
fn quiet_weapon(w: &str) -> bool {
    matches!(
        w,
        "WeaponStealthPistol" | "WeaponMiniCrossbow" | "WeaponCombatKnife" | "WeaponCrowbar" | "WeaponBaton" | "WeaponSword" | "WeaponNanoSword" | "WeaponProd" | "WeaponShuriken" | "WeaponPepperGun" | "WeaponHideAGun"
    )
}

const STEP_RANGE: f64 = 900.0;
const SHOT_RANGE: f64 = 3000.0;
const QUIET_SHOT_RANGE: f64 = 500.0;
/// Running: faster than about a walk.
const RUN_SPEED: f64 = 230.0;
const STEP_EVERY: f32 = 0.42;
const RING: f32 = 64.0;

#[derive(Default)]
pub struct RadarState {
    shots: Vec<(u8, u8)>,
    steps: Vec<(u8, f32)>,
    blips: Vec<Blip>,
    img: Option<Handle<Image>>,
    key: u64,
    redraw: f32,
}

#[derive(Component, Default)]
pub struct RadarNode;

#[allow(clippy::too_many_arguments)]
pub fn cover_system(
    mut commands: Commands,
    time: Res<Time>,
    windows: Query<&Window>,
    cam: Single<&GlobalTransform, (With<FlyCam>, Without<SkyCam>)>,
    mut images: ResMut<Assets<Image>>,
    mut layers: ResMut<Assets<HudLayer>>,
    mut node: Query<(Entity, &mut Node, &mut Visibility), With<RadarNode>>,
    mut st: Local<RadarState>,
) {
    let active = on() && crate::netplay::in_game() && !crate::loader::busy();
    if !active {
        if let Ok(mut h) = HIDDEN.lock() {
            h.clear();
        }
        if let Ok(mut h) = NAMES.lock() {
            h.clear();
        }
        st.blips.clear();
        if st.key != 0 {
            let RadarState { img, .. } = &mut *st;
            crate::prompts::present::<RadarNode, _>(&mut commands, &mut images, &mut layers, &mut node, img, None, (1, 1), 1, Vec2::ZERO, 1.0);
            st.key = 0;
        }
        return;
    }
    let dt = time.delta_secs();
    let gt = *cam;
    let e = bevy_to_ue(gt.translation().to_array());
    let eye = DVec3::new(e[0] as f64, e[1] as f64, e[2] as f64);
    let rivals = crate::netplay::rivals();
    // Who can't be made out (or passes as a guard: uniforms.rs).
    let (mut ids, mut names) = (Vec::new(), Vec::new());
    for r in rivals.iter().filter(|r| !made_out(r, eye) || crate::uniforms::passes(r.id)) {
        ids.push(r.id);
        names.push(r.name.clone());
    }
    if let Ok(mut h) = HIDDEN.lock() {
        *h = ids;
    }
    if let Ok(mut h) = NAMES.lock() {
        *h = names;
    }
    let radar = crate::deathmatch::rules().is_some_and(|r| r.radar) && !crate::netplay::me_dead();
    // Hear: new shots and running steps.
    let RadarState { shots, steps, blips, img, key, redraw } = &mut *st;
    for b in blips.iter_mut() {
        b.age += dt;
    }
    blips.retain(|b| b.age < if b.kind == Kind::Shot { 2.2 } else { 1.2 });
    for r in rivals.iter().filter(|r| r.rival) {
        let d = (r.pos - eye).length();
        // Shots: a changed counter.
        let prev = shots.iter().position(|s| s.0 == r.id);
        let fired = match prev {
            Some(i) => {
                let f = shots[i].1 != r.shots;
                shots[i].1 = r.shots;
                f
            }
            None => {
                shots.push((r.id, r.shots));
                false
            }
        };
        let range = if quiet_weapon(&r.weapon) { QUIET_SHOT_RANGE } else { SHOT_RANGE };
        if fired && d < range && radar {
            blips.retain(|b| !(b.id == r.id && b.kind == Kind::Shot));
            blips.push(Blip { id: r.id, kind: Kind::Shot, at: r.pos, loud: (1.0 - d / range) as f32, age: 0.0 });
        }
        // Steps while running on the ground (or splashing in water).
        let speed = (r.vel.x * r.vel.x + r.vel.y * r.vel.y).sqrt();
        let running = speed > RUN_SPEED && !r.crouch && !r.quiet && (r.vel.z.abs() < 60.0 || r.swim);
        let t = match steps.iter().position(|s| s.0 == r.id) {
            Some(i) => &mut steps[i].1,
            None => {
                steps.push((r.id, 0.0));
                &mut steps.last_mut().unwrap().1
            }
        };
        *t -= dt;
        if running && d < STEP_RANGE && *t <= 0.0 && radar {
            *t = STEP_EVERY;
            blips.retain(|b| !(b.id == r.id && b.kind == Kind::Step));
            blips.push(Blip { id: r.id, kind: Kind::Step, at: r.pos, loud: (1.0 - d / STEP_RANGE) as f32, age: 0.0 });
        }
    }
    shots.retain(|s| rivals.iter().any(|r| r.id == s.0));
    steps.retain(|s| rivals.iter().any(|r| r.id == s.0));
    let Some(win) = windows.iter().next() else { return };
    if blips.is_empty() {
        if *key != 0 {
            crate::prompts::present::<RadarNode, _>(&mut commands, &mut images, &mut layers, &mut node, img, None, (1, 1), 1, Vec2::ZERO, 1.0);
            *key = 0;
        }
        return;
    }
    // Redrawn at 20 Hz while anything shows.
    *redraw -= dt;
    if *redraw > 0.0 && *key != 0 {
        return;
    }
    *redraw = 0.05;
    let s = crate::hud::ui_scale(win).max(1.0) as i32;
    let k = s as f32 / win.scale_factor();
    // The view's forward and right, flat (Unreal axes).
    let f = bevy_to_ue(gt.forward().as_vec3().to_array());
    let fwd = DVec3::new(f[0] as f64, f[1] as f64, 0.0).normalize_or_zero();
    let right = DVec3::new(-fwd.y, fwd.x, 0.0);
    // (Unreal is left-handed: +Y is to the right of +X seen from above.)
    let size = (RING as i32 + 10) * 2;
    let mut c = Canvas::new(size, size, s);
    let theme = crate::menuui::h_col(7);
    let marks: Vec<(f32, f32, [f32; 3], f32)> = blips
        .iter()
        .map(|b| {
            let to = (b.at - eye).with_z(0.0);
            let a = to.dot(right).atan2(to.dot(fwd)) as f32;
            let life = if b.kind == Kind::Shot { 2.2 } else { 1.2 };
            let fade = (1.0 - b.age / life).clamp(0.0, 1.0);
            let col = if b.kind == Kind::Shot { [1.0, 0.35, 0.2] } else { [theme[0] as f32 / 255.0, theme[1] as f32 / 255.0, theme[2] as f32 / 255.0] };
            let width = if b.kind == Kind::Shot { 0.32 } else { 0.2 };
            (a, width, col, fade * (0.35 + 0.65 * b.loud))
        })
        .collect();
    let half = size as f32 / 2.0;
    paint_marks(&mut c, half, &marks);
    *key = 1 + (*key % 1000);
    let pos = Vec2::new(win.width() / 2.0 - half * k, win.height() / 2.0 - half * k);
    crate::prompts::present::<RadarNode, _>(&mut commands, &mut images, &mut layers, &mut node, img, Some(&c), (size, size), s, pos, k);
}

/// The arcs: each mark centred on its angle (0 = straight ahead, up the screen), `width` radians each side,
/// a few units thick on the ring, faded at its ends.
fn paint_marks(c: &mut Canvas, half: f32, marks: &[(f32, f32, [f32; 3], f32)]) {
    let s = c.s;
    let stride = (c.w * s) as usize;
    let inv = 1.0 / s as f32;
    for py in 0..c.h * s {
        for px in 0..c.w * s {
            let (x, y) = ((px as f32 + 0.5) * inv - half, (py as f32 + 0.5) * inv - half);
            let r = (x * x + y * y).sqrt();
            let ring = 1.0 - ((r - RING).abs() / 3.0);
            if ring <= 0.0 {
                continue;
            }
            let ang = x.atan2(-y);
            let mut acc = [0.0f32; 3];
            let mut a = 0.0f32;
            for &(m, w, col, strength) in marks {
                let mut d = (ang - m).abs();
                if d > std::f32::consts::PI {
                    d = std::f32::consts::TAU - d;
                }
                let along = 1.0 - d / w;
                if along <= 0.0 {
                    continue;
                }
                let k = along.min(1.0).powf(0.7) * ring.min(1.0) * strength;
                for i in 0..3 {
                    acc[i] = acc[i].max(col[i] * k);
                }
                a = a.max(k);
            }
            if a <= 0.0 {
                continue;
            }
            let p = &mut c.px[py as usize * stride + px as usize];
            // Additive glow over transparent: premultiplied colour, alpha the coverage.
            *p = [acc[0], acc[1], acc[2], a * 0.85];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rival(light: f32, at: DVec3) -> Rival {
        Rival { id: 1, name: "R".into(), pos: at, vel: DVec3::ZERO, crouch: false, swim: false, shots: 0, weapon: String::new(), quiet: false, light, camo: false, down: false, rival: true, uniform: false, augs: Vec::new() }
    }

    #[test]
    fn darkness_hides_rivals() {
        let eye = DVec3::ZERO;
        assert!(made_out(&rival(0.4, DVec3::new(1500.0, 0.0, 0.0)), eye), "lit at 15 m");
        assert!(!made_out(&rival(0.01, DVec3::new(1500.0, 0.0, 0.0)), eye), "deep dark at 15 m");
        assert!(made_out(&rival(0.0, DVec3::new(200.0, 0.0, 0.0)), eye), "brushing past");
        let mut r = rival(0.5, DVec3::new(600.0, 0.0, 0.0));
        r.camo = true;
        assert!(!made_out_with(&r, eye, false, false));
        // Counters: Vision sees through the cloak within its reach, goggles closer in.
        assert!(made_out_with(&r, eye, true, false));
        assert!(made_out_with(&r, eye, false, true));
        r.pos = DVec3::new(900.0, 0.0, 0.0);
        assert!(!made_out_with(&r, eye, false, true));
        r.camo = false;
        r.rival = false;
        r.light = 0.0;
        r.pos = DVec3::new(2900.0, 0.0, 0.0);
        assert!(made_out(&r, eye), "teammates always");
    }

    #[test]
    fn marks_point_the_right_way() {
        // A mark straight ahead lights the top of the ring, not the bottom.
        let mut c = Canvas::new(148, 148, 1);
        paint_marks(&mut c, 74.0, &[(0.0, 0.3, [1.0, 0.0, 0.0], 1.0)]);
        let at = |x: i32, y: i32| c.px[(y * 148 + x) as usize][3];
        assert!(at(74, 74 - RING as i32) > 0.5);
        assert_eq!(at(74, 74 + RING as i32), 0.0);
        // To the right (+90 degrees) lights the right side.
        let mut c = Canvas::new(148, 148, 1);
        paint_marks(&mut c, 74.0, &[(std::f32::consts::FRAC_PI_2, 0.3, [1.0, 0.0, 0.0], 1.0)]);
        assert!(c.px[(74 * 148 + 74 + RING as i32) as usize][3] > 0.5);
    }
}
