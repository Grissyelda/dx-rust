//! Dynamic lights: muzzle flashes, explosions, burning projectiles, flares, fires and lamps that aren't baked into
//! the lightmaps, and the flashlight augmentation (AugLight). Plus the renderer's brightness (a gamma curve on the
//! final image, like the original's brightness slider).
//!
//! Gameplay code calls [`flash`] (a light for a fixed time), [`explosion`] (ExplosionLight: radius `size`, shrinking by
//! one every 0.1 s) or [`hold`] (a light that exists this frame). [`dynlight_system`] turns the active lights into
//! Bevy point lights for the BSP (which is lit by the GPU on top of its lightmap) and publishes them so vertex-lit
//! meshes include them ([`lights`]); [`relight_system`] re-lights decorations and pawns near them.
//!
//! Unreal light values: LightRadius r covers 25 x (r + 1) units; LightHue/LightSaturation as in the lightmaps
//! (saturation inverted); LightBrightness / 255. Falloff on meshes is the lightmap's (1 - d / radius).

use crate::actors::Deco;
use crate::{FlyCam, LevelRes, Player, SkyCam};
use bevy::prelude::*;
use dx_level::lighting::{hsv_white, LightInfo};
use dx_level::ue_to_bevy;
use glam::DVec3;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use unreal_pkg::props::{find, Value};

/// A light as Unreal describes it.
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub hue: u8,
    pub saturation: u8,
    pub brightness: u8,
    /// LightRadius (Unreal byte; may be fractional for shrinking lights).
    pub radius: f32,
}

impl Spec {
    pub const fn new(hue: u8, saturation: u8, brightness: u8, radius: f32) -> Spec {
        Spec {
            hue,
            saturation,
            brightness,
            radius,
        }
    }
    fn info(&self, pos: DVec3, scale: f32) -> LightInfo {
        let c = hsv_white(self.hue, self.saturation);
        let b = self.brightness as f32 / 255.0 * scale;
        LightInfo {
            pos: pos.as_vec3().to_array(),
            color: [c[0] * b, c[1] * b, c[2] * b],
            radius: 25.0 * (self.radius + 1.0),
        }
    }
}

/// DeusExWeapon.MuzzleFlashLight: MuzzleFlash (hue 28, saturation 160, brightness 255, radius 3, 0.1 s).
pub const MUZZLE_FLASH: Spec = Spec::new(28, 160, 255, 3.0);
/// Fireball (the flamethrower's flame): hue 16, saturation 32, brightness 200, radius 2.
pub const FIREBALL: Spec = Spec::new(16, 32, 200, 2.0);
/// A lit flare: hue 16, saturation 96, brightness 255, radius 8 (LE_TorchWaver).
pub const FLARE: Spec = Spec::new(16, 96, 255, 8.0);

enum Kind {
    /// Steady for its lifetime.
    Timed(f32),
    /// ExplosionLight.Timer: every 0.1 s the radius drops by one; gone below 1.
    Shrink(f32),
}

struct Transient {
    pos: DVec3,
    spec: Spec,
    kind: Kind,
}

static PENDING: Mutex<Vec<Transient>> = Mutex::new(Vec::new());
static HELD: Mutex<Vec<(u64, DVec3, Spec, f32)>> = Mutex::new(Vec::new());
/// The dynamic lights active this frame (for vertex lighting).
static CURRENT: Mutex<Vec<LightInfo>> = Mutex::new(Vec::new());
/// Placed actor lights switched from how the map left them (Lamp.Frob), by actor index.
static TOGGLED: Mutex<Option<HashSet<usize>>> = Mutex::new(None);

/// Placed actors whose light game code holds itself (security cameras change colour), by actor index.
static SUPPRESSED: Mutex<Option<HashSet<usize>>> = Mutex::new(None);

pub fn suppress_actor_light(actor: usize) {
    if let Ok(mut t) = SUPPRESSED.lock() {
        t.get_or_insert_with(HashSet::new).insert(actor);
    }
}

/// Flip a placed actor's light on/off (Lamp: LightType LT_Steady <-> LT_None).
pub fn toggle_actor_light(actor: usize) {
    if let Ok(mut t) = TOGGLED.lock() {
        let set = t.get_or_insert_with(HashSet::new);
        if !set.remove(&actor) {
            set.insert(actor);
        }
    }
}

/// An event reaches the TriggerLights tagged with it (TriggerLight states TriggerToggle / TriggerTurnsOn /
/// TriggerTurnsOff / TriggerControl; ChangeTime fades are instant here).
pub fn trigger_event(level: &dx_level::Level, tag: &str, on: bool) {
    for (i, a) in level.actors.iter().enumerate() {
        if a.class != "TriggerLight" {
            continue;
        }
        let t = match find(&a.props, "Tag") {
            Some(Value::Name(t)) => t,
            _ => continue,
        };
        if !t.eq_ignore_ascii_case(tag) {
            continue;
        }
        let initially = matches!(find(&a.props, "bInitiallyOn"), Some(Value::Bool(true)));
        let state = match find(&a.props, "InitialState") {
            Some(Value::Name(s)) => s.to_ascii_lowercase(),
            _ => "triggertoggle".into(),
        };
        let lit_now = initially != TOGGLED.lock().ok().and_then(|t| t.as_ref().map(|s| s.contains(&i))).unwrap_or(false);
        let want = match state.as_str() {
            "triggerturnson" if on => Some(true),
            "triggerturnsoff" if on => Some(false),
            "triggercontrol" => Some(if on { !initially } else { initially }),
            "triggertoggle" if on => Some(!lit_now),
            _ => None,
        };
        if want.is_some_and(|w| w != lit_now) {
            toggle_actor_light(i);
        }
    }
}

/// Lamps switched from how the map left them (save.rs).
pub fn toggled() -> Vec<usize> {
    let mut v: Vec<usize> = TOGGLED.lock().ok().and_then(|t| t.clone()).unwrap_or_default().into_iter().collect();
    v.sort();
    v
}
pub fn set_toggled(v: &[usize]) {
    if let Ok(mut t) = TOGGLED.lock() {
        *t = Some(v.iter().copied().collect());
    }
}

/// A light that lasts `seconds`.
pub fn flash(pos: DVec3, spec: Spec, seconds: f32) {
    if let Ok(mut p) = PENDING.lock() {
        p.push(Transient {
            pos,
            spec,
            kind: Kind::Timed(seconds),
        });
    }
}

/// ExplosionLight with this `size` (2 small, 4 medium, 8 large) and colour.
pub fn explosion(pos: DVec3, size: f32, hue: u8, saturation: u8) {
    if let Ok(mut p) = PENDING.lock() {
        p.push(Transient {
            pos,
            spec: Spec::new(hue, saturation, 255, size.clamp(1.0, 16.0)),
            kind: Kind::Shrink(0.1),
        });
    }
}

/// A light that shines this frame. `flicker` 0..1 dims it at random (fire).
pub fn hold(key: u64, pos: DVec3, spec: Spec, flicker: f32) {
    // GMDX Lighting Accessibility (setting NoStrobeLights): no flickering lights.
    let flicker = if crate::settings::get("NoStrobeLights") > 0.5 { 0.0 } else { flicker };
    if let Ok(mut h) = HELD.lock() {
        h.push((key, pos, spec, flicker));
    }
}

/// The static lights plus whatever dynamic lights are on right now, for vertex lighting.
pub fn lights(statics: &[LightInfo]) -> Cow<'_, [LightInfo]> {
    let cur = CURRENT.lock().map(|c| c.clone()).unwrap_or_default();
    if cur.is_empty() {
        Cow::Borrowed(statics)
    } else {
        let mut v = statics.to_vec();
        v.extend(cur);
        Cow::Owned(v)
    }
}

/// Shadowed static lights by 32-unit cell (static lights never move, so the traces are done once per spot).
static SHADOW_CACHE: Mutex<Option<HashMap<(i32, i32, i32, i32), std::sync::Arc<Vec<LightInfo>>>>> = Mutex::new(None);

/// The lights that reach an actor at `center` (half height `half_h`), as Unreal lights actors: static lights that
/// can see it (cached per spot), plus the dynamic lights on now that can see it.
pub fn lights_at(level: &dx_level::Level, center: [f32; 3], half_h: f32) -> Vec<LightInfo> {
    let key = (
        (center[0] / 32.0).floor() as i32,
        (center[1] / 32.0).floor() as i32,
        (center[2] / 32.0).floor() as i32,
        (half_h / 16.0).round() as i32,
    );
    let statics = {
        let mut g = SHADOW_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        let map = g.get_or_insert_with(HashMap::new);
        if map.len() > 200_000 {
            map.clear();
        }
        map.entry(key)
            .or_insert_with(|| {
                // The middle of the cell, so every actor in it gets the same answer.
                let c = [(key.0 as f32 + 0.5) * 32.0, (key.1 as f32 + 0.5) * 32.0, (key.2 as f32 + 0.5) * 32.0];
                std::sync::Arc::new(dx_level::actors::shadowed_lights(&level.collision, &level.lights, c, half_h))
            })
            .clone()
    };
    let cur = current();
    let mut v: Vec<LightInfo> = statics.as_ref().clone();
    if !cur.is_empty() {
        v.extend(dx_level::actors::shadowed_lights(&level.collision, &cur, center, half_h));
    }
    if let Some(l) = flash_spot_on(level, center) {
        v.push(l);
    }
    v
}

/// The Flashlight Cone's light on a vertex-lit mesh: the cone from the eye, as a light just in front of the mesh
/// on the eye's side - only inside the cone and when the eye can see the mesh (the old beam light sat near where
/// the view hit and lit whatever was round it, behind walls and poles too).
#[derive(Clone, Copy)]
struct FlashSpot {
    eye: DVec3,
    dir: DVec3,
    reach: f64,
    spec: Spec,
}

static FLASH_SPOT: Mutex<Option<FlashSpot>> = Mutex::new(None);

fn flash_spot_on(level: &dx_level::Level, center: [f32; 3]) -> Option<LightInfo> {
    let f = (*FLASH_SPOT.lock().ok()?)?;
    let c = DVec3::new(center[0] as f64, center[1] as f64, center[2] as f64);
    let to = c - f.eye;
    let d = to.length();
    if d < 1.0 || d > f.reach + 400.0 {
        return None;
    }
    let cos = to.dot(f.dir) / d;
    // The cone's soft edge (outer 0.48 rad, full inside 0.04 rad, as the GPU spot).
    let (c_out, c_in) = (0.48f64.cos(), 0.04f64.cos());
    let k = ((cos - c_out) / (c_in - c_out)).clamp(0.0, 1.0);
    if k <= 0.0 {
        return None;
    }
    let h = level.collision.trace(f.eye, c - to / d * 24.0, DVec3::ZERO);
    if h.fraction < 0.999 {
        return None;
    }
    // Strength falls off with distance like the cone's pool (full to 256 units, then dimming).
    let fall = (1.0 - ((d - 256.0) / (f.reach + 400.0 - 256.0)).max(0.0)).clamp(0.0, 1.0);
    let k = (k * k * (3.0 - 2.0 * k)) as f32 * fall as f32;
    let mut l = f.spec.info(c - to / d * 64.0, k);
    l.radius = 160.0;
    Some(l)
}

/// The flashlight cone (enhancement): a spot light that follows the view.
#[derive(Component)]
pub struct FlashCone;

/// Enhancement: a flashlight cone with soft spill instead of the beam light (setting FlashlightCone).
pub fn flashlight_cone() -> bool {
    crate::settings::get("FlashlightCone") > 0.5
}

/// The flashlight's light this frame (with the Flashlight Cone): where it shines from, its direction and how far
/// it reaches before a wall (metres). Light Shafts' beam follows it so the two make one pool.
static FLASH: Mutex<Option<(Vec3, Vec3, f32)>> = Mutex::new(None);
pub fn flashlight() -> Option<(Vec3, Vec3, f32)> {
    FLASH.lock().ok().and_then(|f| *f)
}

/// Lights for the vertex-lit meshes only (the flashlight beam while the cone lights the world).
static CPU_ONLY: Mutex<Vec<LightInfo>> = Mutex::new(Vec::new());

/// The dynamic lights on right now.
pub fn current() -> Vec<LightInfo> {
    CURRENT.lock().map(|c| c.clone()).unwrap_or_default()
}

#[derive(Component)]
pub struct DynLightSlot;

#[derive(Resource)]
pub struct DynLights {
    transients: Vec<Transient>,
    pool: Vec<Entity>,
    gain: f32,
    seed: u32,
    flicker: HashMap<u64, (f32, f32)>,
    /// Placed actors that give off light but aren't baked (fires, lamps, cameras): actor index, spec, flicker.
    actor_lights: Option<Vec<(usize, Spec, f32, Option<usize>, bool)>>,
}

const POOL: usize = 24;

pub fn setup_dynlights(mut commands: Commands) {
    let mut pool = Vec::new();
    for _ in 0..POOL {
        pool.push(
            commands
                .spawn((
                    PointLight {
                        intensity: 0.0,
                        range: 1.0,
                        shadow_maps_enabled: false,
                        ..default()
                    },
                    Transform::default(),
                    Visibility::Hidden,
                    DynLightSlot,
                ))
                .id(),
        );
    }
    let gain = std::env::var("DXVIEW_DYNLIGHT_GAIN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0);
    commands.insert_resource(DynLights {
        transients: Vec::new(),
        pool,
        gain,
        seed: 0x1234_5678,
        flicker: HashMap::new(),
        actor_lights: None,
    });
}

fn rand(seed: &mut u32) -> f32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    (*seed >> 8) as f32 / (1u32 << 24) as f32
}

#[allow(clippy::too_many_arguments)]
pub fn dynlight_system(
    time: Res<Time>,
    level: Res<LevelRes>,
    mut st: ResMut<DynLights>,
    blockers: Res<crate::actors::Blockers>,
    cam: Single<&Transform, (With<FlyCam>, Without<SkyCam>, Without<DynLightSlot>)>,
    mut slots: Query<(&mut PointLight, &mut Transform, &mut Visibility), With<DynLightSlot>>,
) {
    let _prof = crate::prof::scope("dynlight::dynlight_system");
    let dt = time.delta_secs();
    let st = &mut *st;
    if let Ok(mut p) = PENDING.lock() {
        st.transients.extend(p.drain(..));
    }
    // Age the transient lights.
    st.transients.retain_mut(|t| match &mut t.kind {
        Kind::Timed(left) => {
            *left -= dt;
            *left > 0.0
        }
        Kind::Shrink(timer) => {
            *timer -= dt;
            while *timer <= 0.0 {
                *timer += 0.1;
                t.spec.radius -= 1.0;
            }
            t.spec.radius >= 1.0
        }
    });

    // Placed light-giving actors that the lightmaps don't include (BarrelFire, Lamp3, security cameras...).
    if st.actor_lights.is_none() {
        let by_actor: HashMap<usize, usize> = blockers
            .list
            .iter()
            .enumerate()
            .map(|(b, c)| (c.actor, b))
            .collect();
        let baked: Vec<[f32; 3]> = level.0.lights.iter().map(|l| l.pos).collect();
        let mut list = Vec::new();
        for (i, a) in level.0.actors.iter().enumerate() {
            let byte = |n: &str| match find(&a.props, n) {
                Some(Value::Byte(b)) => Some(*b),
                _ => None,
            };
            // Lamps can be switched on even when placed dark.
            let lamp = matches!(a.class.as_str(), "Lamp1" | "Lamp2" | "Lamp3");
            // TriggerLight: on if bInitiallyOn (default off); its trigger state switches it (trigger_event).
            let trigger_light = a.class == "TriggerLight";
            let initially_on = if trigger_light {
                matches!(find(&a.props, "bInitiallyOn"), Some(Value::Bool(true)))
            } else {
                byte("LightType").unwrap_or(0) != 0
            };
            let lamp = lamp || trigger_light;
            if !initially_on && !lamp {
                continue;
            }
            let Some(Value::Vector(p)) = find(&a.props, "Location") else {
                continue;
            };
            if baked.iter().any(|b| {
                (b[0] - p[0]).abs() < 1.0 && (b[1] - p[1]).abs() < 1.0 && (b[2] - p[2]).abs() < 1.0
            }) {
                continue;
            }
            let spec = Spec::new(
                byte("LightHue").unwrap_or(0),
                byte("LightSaturation").unwrap_or(255),
                byte("LightBrightness").unwrap_or(64),
                byte("LightRadius").unwrap_or(64) as f32,
            );
            // LE_TorchWaver / LE_FireWaver flicker.
            let flicker = if matches!(byte("LightEffect"), Some(1) | Some(2)) {
                0.25
            } else {
                0.0
            };
            list.push((i, spec, flicker, by_actor.get(&i).copied(), initially_on));
        }
        println!("dynlight: {} unbaked actor lights", list.len());
        st.actor_lights = Some(list);
    }
    let eye = DVec3::new(
        cam.translation.x as f64,
        cam.translation.z as f64,
        cam.translation.y as f64,
    ) / dx_level::METERS_PER_UNIT as f64;
    if let Some(list) = st.actor_lights.as_ref() {
        let toggled = TOGGLED
            .lock()
            .ok()
            .and_then(|t| t.clone())
            .unwrap_or_default();
        let suppressed = SUPPRESSED
            .lock()
            .ok()
            .and_then(|t| t.clone())
            .unwrap_or_default();
        for (i, spec, flicker, blocker, initially_on) in list {
            if *initially_on == toggled.contains(i) || suppressed.contains(i) {
                continue;
            }
            let pos = match blocker {
                Some(b) => match blockers.list.get(*b) {
                    Some(c) if c.radius > 0.0 => c.center,
                    _ => continue,
                },
                None => match find(&level.0.actors[*i].props, "Location") {
                    Some(Value::Vector(v)) => DVec3::new(v[0] as f64, v[1] as f64, v[2] as f64),
                    _ => continue,
                },
            };
            let r = 25.0 * (spec.radius as f64 + 1.0);
            if pos.distance(eye) < r + 4000.0 {
                hold(0xD_0000_0000 | *i as u64, pos, *spec, *flicker);
            }
        }
    }

    // Everything on this frame.
    let mut active: Vec<(LightInfo, Spec)> = st
        .transients
        .iter()
        .map(|t| (t.spec.info(t.pos, 1.0), t.spec))
        .collect();
    let held: Vec<(u64, DVec3, Spec, f32)> = HELD
        .lock()
        .map(|mut h| std::mem::take(&mut *h))
        .unwrap_or_default();
    let mut seen = HashSet::new();
    for (key, pos, spec, flicker) in held {
        if !seen.insert(key) {
            continue;
        }
        let mut scale = 1.0;
        if flicker > 0.0 {
            // A new random level about ten times a second, eased toward.
            let e = st.flicker.entry(key).or_insert((1.0, 0.0));
            e.1 -= dt;
            if e.1 <= 0.0 {
                e.1 = 0.1;
                let r = rand(&mut st.seed);
                e.0 = 1.0 - flicker * r;
            }
            scale = st.flicker[&key].0;
        }
        active.push((spec.info(pos, scale), spec));
    }
    if let Ok(mut c) = CURRENT.lock() {
        *c = active.iter().map(|(l, _)| *l).collect();
        if let Ok(x) = CPU_ONLY.lock() {
            c.extend(x.iter().copied());
        }
    }

    // The nearest ones become GPU point lights for the BSP. Calibrated so that at half the radius the light adds what
    // the lightmaps give there: ((1 - 0.5) x brightness x 3) in gamma space, then linear.
    active.sort_by(|a, b| {
        let da = DVec3::from_array(a.0.pos.map(|v| v as f64)).distance(eye);
        let db = DVec3::from_array(b.0.pos.map(|v| v as f64)).distance(eye);
        da.total_cmp(&db)
    });
    let mut it = active.iter();
    for (mut pl, mut tf, mut vis) in &mut slots {
        match it.next() {
            Some((l, spec)) => {
                let r_m = l.radius * dx_level::METERS_PER_UNIT;
                let c = hsv_white(spec.hue, spec.saturation);
                let b = (l.color[0].max(l.color[1]).max(l.color[2])).max(1e-4);
                let k = (1.5 * b).min(1.0).powf(2.2);
                pl.color = Color::srgb(c[0], c[1], c[2]);
                pl.intensity = std::f32::consts::PI.powi(2) * r_m * r_m * k * st.gain;
                pl.range = r_m;
                tf.translation = Vec3::from(ue_to_bevy(l.pos));
                *vis = Visibility::Inherited;
            }
            None => {
                pl.intensity = 0.0;
                *vis = Visibility::Hidden;
            }
        }
    }
}

/// Re-light decorations and pawns that a dynamic light reaches (and once more when it's gone), up to 30 times a
/// second.
/// A hash of the lights reaching something and where it is (positions to a unit, colours in 1/64 steps): equal
/// signatures light it the same, so its mesh needn't be relit and re-uploaded.
pub fn light_sig(lights: &[LightInfo], at: [f32; 3], extra: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for l in lights {
        for k in 0..3 {
            (l.pos[k].round() as i32).hash(&mut h);
            ((l.color[k] * 64.0).round() as i32).hash(&mut h);
        }
        (l.radius.round() as i32).hash(&mut h);
    }
    for k in 0..3 {
        (at[k].round() as i32).hash(&mut h);
    }
    extra.hash(&mut h);
    h.finish()
}

pub fn relight_system(
    time: Res<Time>,
    level: Res<LevelRes>,
    blockers: Res<crate::actors::Blockers>,
    decos: Query<(Entity, &Deco)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut map_gen: Local<u64>,
    mut lit: Local<HashMap<Entity, u64>>,
    mut timer: Local<f32>,
) {
    // A new map (loader.rs): start this system's per-map state over.
    if crate::loader::new_map(&mut map_gen) {
        lit.clear();
    }
    let _prof = crate::prof::scope("dynlight::relight_system");
    *timer -= time.delta_secs();
    if *timer > 0.0 {
        return;
    }
    *timer = 1.0 / 30.0;
    let dyn_lights = current();
    if dyn_lights.is_empty() && lit.is_empty() {
        return;
    }
    let lvl = &level.0;
    let mut now = HashMap::new();
    for (e, d) in &decos {
        let draw = d.body.location + d.pre;
        let p = draw.as_vec3().to_array();
        let near = dyn_lights.iter().any(|l| {
            let dd =
                ((l.pos[0] - p[0]).powi(2) + (l.pos[1] - p[1]).powi(2) + (l.pos[2] - p[2]).powi(2))
                    .sqrt();
            dd < l.radius + 96.0
        });
        let was = lit.get(&e).copied();
        if !near && was.is_none() {
            continue;
        }
        // The lights that reach it as the spawn lighting sees them (static lights it is in sight of, plus the
        // dynamic ones in sight), so a deco lit by a passing flash goes back to exactly how it was.
        let half_h = d.blocker.and_then(|b| blockers.list.get(b)).map_or(16.0, |c| c.height as f32);
        let lights = lights_at(lvl, p, half_h);
        // Only when what lights it changed (a flickering fire's brightness in 1/64 steps, a light moving more than
        // a unit, the deco itself moving or turning): HDTP's dense meshes made relighting every deco near a lamp
        // 30 times a second the biggest per-frame mesh upload.
        let sig = light_sig(
            &lights,
            p,
            ((d.rotation[0] as u64 & 0xffff) << 32)
                ^ ((d.rotation[1] as u64 & 0xffff) << 16)
                ^ (d.rotation[2] as u64 & 0xffff)
                ^ (((d.scale_glow * 64.0).round() as u64) << 48),
        );
        if near {
            now.insert(e, sig);
        }
        if was == Some(sig) {
            continue;
        }
        let ma = &lvl.mesh_actors[d.mesh_actor];
        for (h, pi) in &d.parts {
            if let Some(mut m) = meshes.get_mut(h) {
                m.insert_attribute(
                    Mesh::ATTRIBUTE_COLOR,
                    dx_level::actors::light_part(&ma.parts[*pi], p, d.rotation, d.scale_glow, &lights),
                );
            }
        }
    }
    *lit = now;
}

/// AugLight, the flashlight augmentation JC starts with (F12 through `abilities`, which also drains its 10 energy
/// units a minute). Two Beam lights: one 64 units short of where the view meets something (up to 1024 away), its
/// radius growing with distance (4 + 5.12 x dist / 1024); one small glow at the player's shoulders.
pub fn aug_light_system(
    mut commands: Commands,
    level: Res<LevelRes>,
    player: Res<Player>,
    cam: Single<&Transform, (With<FlyCam>, Without<SkyCam>, Without<FlashCone>)>,
    mut cone: Query<(&mut SpotLight, &mut Transform, &mut Visibility), With<FlashCone>>,
    st: Option<Res<DynLights>>,
) {
    let _prof = crate::prof::scope("dynlight::aug_light_system");
    // Off while a conversation has the camera (the beam would sweep the scene from JC's eyes).
    let on = crate::abilities::aug_active("AugLight") && !crate::consys::third_person();
    let cone_on = on && flashlight_cone();
    if !cone_on {
        for (mut sl, _, mut vis) in &mut cone {
            sl.intensity = 0.0;
            *vis = Visibility::Hidden;
        }
    }
    if let Ok(mut c) = CPU_ONLY.lock() {
        c.clear();
    }
    if let Ok(mut f) = FLASH.lock() {
        *f = None;
    }
    if let Ok(mut f) = FLASH_SPOT.lock() {
        *f = None;
    }
    if !on {
        return;
    }
    let eye = DVec3::new(
        cam.translation.x as f64,
        cam.translation.z as f64,
        cam.translation.y as f64,
    ) / dx_level::METERS_PER_UNIT as f64;
    let f = cam.forward().as_vec3();
    let fwd = DVec3::new(f.x as f64, f.z as f64, f.y as f64).normalize_or_zero();
    let range = 1024.0;
    let h = level.0.collision.trace(eye, eye + fwd * range, DVec3::ZERO);
    let dist = range * h.fraction;
    let size = (dist / range).clamp(0.0, 1.0) as f32;
    let beam = Spec::new(32, 140, 192, size * 5.12 + 4.0);
    let beam_at = eye + fwd * dist - fwd * 64.0;
    if cone_on {
        // Enhancement (Flashlight Cone): a real spot light from the shoulder with a soft edge and shadows lights the
        // world; the meshes are still vertex-lit by the original beam light (CPU only, so the BSP isn't lit twice).
        // Its strength follows the beam's: at the spot it looks at, about what the beam light gives 64 units off.
        if let Ok(mut c) = FLASH_SPOT.lock() {
            *c = Some(FlashSpot { eye, dir: fwd, reach: dist, spec: Spec::new(32, 140, 192, 4.0) });
        }
        let r_m = beam.info(beam_at, 1.0).radius * dx_level::METERS_PER_UNIT;
        let k = (1.5f32 * 192.0 / 255.0).min(1.0).powf(2.2);
        let gain = st.as_ref().map_or(1.0, |s| s.gain);
        // (capped: aimed at the sky, the light needed to reach "the spot" 1024 units off blew out everything near)
        let d = (dist.clamp(64.0, 256.0) / 64.0) as f32;
        let intensity = std::f32::consts::PI.powi(2) * r_m * r_m * k * gain * d * d * 5.0 * std::env::var("DXVIEW_CONE_GAIN").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0);
        let c = hsv_white(32, 140);
        let right = cam.right().as_vec3();
        // Just off the eye (a little right and down, so shadows show as thin edges): from the shoulder, the pool's
        // parallax against where the view points was obvious up close.
        let pos = cam.translation + right * 4.0 * dx_level::METERS_PER_UNIT - Vec3::Y * 3.0 * dx_level::METERS_PER_UNIT;
        let look = cam.translation + cam.forward().as_vec3() * (dist as f32 * dx_level::METERS_PER_UNIT).max(1.0);
        let tf = Transform::from_translation(pos).looking_at(look, Vec3::Y);
        if let Ok(mut f) = FLASH.lock() {
            *f = Some((pos, (look - pos).normalize_or(Vec3::NEG_Z), dist as f32 * dx_level::METERS_PER_UNIT));
        }
        match cone.iter_mut().next() {
            Some((mut sl, mut t, mut vis)) => {
                if std::env::var_os("DXVIEW_CONELOG").is_some() {
                    println!("CONE dist {dist:.0} intensity {intensity:.0} at {:?}", tf.translation);
                }
                sl.intensity = intensity;
                sl.color = Color::srgb(c[0], c[1], c[2]);
                *t = tf;
                *vis = Visibility::Inherited;
            }
            None => {
                commands.spawn((
                    SpotLight {
                        intensity,
                        color: Color::srgb(c[0], c[1], c[2]),
                        range: 2048.0 * dx_level::METERS_PER_UNIT,
                        inner_angle: 0.04,
                        outer_angle: 0.48,
                        shadow_maps_enabled: std::env::var_os("DXVIEW_CONE_NOSHADOW").is_none(),
                        // Grazing walls lit from almost the eye: the default biases left acne (black lines, a
                        // dark wedge, speckles) on them.
                        shadow_depth_bias: 0.12,
                        shadow_normal_bias: 3.0,
                        ..default()
                    },
                    tf,
                    FlashCone,
                ));
            }
        }
    } else {
        hold(crate::sound::key_of("aug-light-beam"), beam_at, beam, 0.0);
        if let Ok(mut f) = FLASH.lock() {
            *f = Some((cam.translation, cam.forward().as_vec3(), dist as f32 * dx_level::METERS_PER_UNIT));
        }
    }
    let flat = DVec3::new(fwd.x, fwd.y, 0.0).normalize_or_zero();
    let glow_at = player.body.location
        + DVec3::new(0.0, 0.0, player.eye_height)
        + flat * player.params.radius * 1.5;
    hold(
        crate::sound::key_of("aug-light-glow"),
        glow_at,
        Spec::new(32, 140, 220, 4.0),
        0.0,
    );
}

/// The renderer's brightness: a gamma curve on the final picture (the original's brightness slider). the - key steps
/// through settings; DXVIEW_GAMMA sets the starting value.
#[derive(
    Component,
    bevy::render::extract_component::ExtractComponent,
    Clone,
    Copy,
    bevy::render::render_resource::ShaderType,
    Default,
)]
pub struct Brightness {
    pub gamma: f32,
    /// Constant view fog added after the gamma curve (ViewFog of the zone the eye is in: underwater blue).
    pub fog_r: f32,
    pub fog_g: f32,
    pub fog_b: f32,
    /// Modern Vision Augmentation (vision.rs): light amplification strength (0 = off), time (grain), infrared level
    /// (0 none, 1 heat shows), and the light-amp box (0 = the whole screen, 1 = the original's middle half).
    pub nv: f32,
    pub nv_t: f32,
    pub nv_ir: f32,
    pub nv_box: f32,
}

pub const GAMMA_SHADER: Handle<Shader> =
    bevy::asset::uuid_handle!("6d1f2b0e-8c3a-4c9e-9a51-2f6a1d7e4b10");

impl bevy::core_pipeline::fullscreen_material::FullscreenMaterial for Brightness {
    fn fragment_shader() -> bevy::shader::ShaderRef {
        GAMMA_SHADER.into()
    }
}

pub fn default_gamma() -> f32 {
    std::env::var("DXVIEW_GAMMA")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.15)
}

pub fn setup_gamma_shader(mut shaders: ResMut<Assets<Shader>>) {
    let _ = shaders.insert(
        &GAMMA_SHADER,
        Shader::from_wgsl(include_str!("shaders/gamma.wgsl"), "dxview/gamma.wgsl"),
    );
}

pub fn brightness_key_system(
    keys: Res<ButtonInput<KeyCode>>,
    mut q: Query<&mut Brightness>,
    mut hud: ResMut<crate::movers::HudMessage>,
    mut gen: Local<u64>,
) {
    // The Brightness slider (Settings > Display > Adjust Brightness) sets the gamma; DXVIEW_GAMMA overrides it.
    let g = crate::settings::generation();
    if g != *gen {
        *gen = g;
        if std::env::var("DXVIEW_GAMMA").is_err() {
            for mut b in &mut q {
                b.gamma = crate::settings::gamma();
            }
        }
    }
    if !keys.just_pressed(KeyCode::Minus) {
        return;
    }
    const STEPS: [f32; 6] = [1.0, 1.15, 1.35, 1.55, 1.75, 2.0];
    for mut b in &mut q {
        let i = STEPS
            .iter()
            .position(|s| (*s - b.gamma).abs() < 0.01)
            .map(|i| (i + 1) % STEPS.len())
            .unwrap_or(0);
        b.gamma = STEPS[i];
        hud.text = format!("Brightness (gamma) {:.2}  (- to change)", b.gamma);
        hud.time_left = 2.0;
    }
}

/// A new map is going in (loader.rs): this module's state of the map being left.
pub fn reset_level() {
    if let Ok(mut v) = PENDING.lock() {
        v.clear();
    }
    if let Ok(mut v) = HELD.lock() {
        v.clear();
    }
    if let Ok(mut v) = CURRENT.lock() {
        v.clear();
    }
    if let Ok(mut v) = TOGGLED.lock() {
        *v = None;
    }
    if let Ok(mut v) = SUPPRESSED.lock() {
        *v = None;
    }
    if let Ok(mut v) = SHADOW_CACHE.lock() {
        *v = None;
    }
}
