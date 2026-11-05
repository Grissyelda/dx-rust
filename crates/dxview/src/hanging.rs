//! HangingDecoration (lanterns, hanging pigs and chickens, shop and pool-table lights, chandeliers, traffic lights,
//! bird cages, clothes racks) and Cart, ported from their scripts:
//!
//! * HangingDecoration.Tick: the rotation is the original one plus wind (bRandomize: a new random gust every 2-6 s,
//!   eased in and out with Smerp), a fixed sway (bFixedSwaying: FixedSwayDirection x sin over FixedSwayPeriod) and
//!   a damped swing (bSwaying) started by CalculateHit when bumped, shot or blown at; the swing loses 0.5% a tick
//!   and stops below 0.001.
//! * Cart.StartRolling / Bump / Tick: a bump sets it rolling at 0.25 x the bumper's velocity x mass ratio, held for
//!   2 s (wheels: no friction), with the UtilityCart rattle pitched by speed.
//! * Mission09.ShipExplosionEffects swings every hanging decoration and rolls every cart (`ship_shake`).

use crate::actors::Deco;
use bevy::prelude::*;
use glam::DVec3;
use imsim_core::body::BodyPhysics;
use std::sync::Mutex;
use unreal_pkg::props::{find, Value};

#[derive(Component, Default)]
pub struct Hanging {
    fixed: bool,
    fixed_dir: DVec3,
    fixed_period: f64,
    randomize: bool,
    no_pitch: bool,
    no_roll: bool,
    fixed_timer: f64,
    /// (pitch, yaw, roll) in Unreal units.
    sway: DVec3,
    swaying: bool,
    damping: f64,
    period: f64,
    timer: f64,
    wind: DVec3,
    orig_wind: DVec3,
    wind_period: f64,
    wind_timer: f64,
    /// SetTimer(windPeriod): seconds to the next gust.
    next_gust: f64,
    orig_rot: [i32; 3],
}

#[derive(Component, Default)]
pub struct Cart {
    roll_timer: f64,
    push_timer: f64,
    push_vel: DVec3,
    just_pushed: bool,
}

/// Bumps this frame: (blocker, bumper location, bumper velocity, bumper mass).
static BUMPS: Mutex<Vec<(usize, DVec3, DVec3, f64)>> = Mutex::new(Vec::new());
/// Shots / blasts on decorations: (blocker, where from, momentum size).
static HITS: Mutex<Vec<(usize, DVec3, f64)>> = Mutex::new(Vec::new());
/// Mission09's ship shakes: (bobble, size).
static SHAKES: Mutex<Vec<(DVec3, f64)>> = Mutex::new(Vec::new());

/// Cart blockers on this map (DeusExDecoration's push rule skips them).
static CARTS: Mutex<Vec<usize>> = Mutex::new(Vec::new());

pub fn is_cart(blocker: usize) -> bool {
    CARTS.lock().is_ok_and(|c| c.contains(&blocker))
}

pub fn reset_level() {
    if let Ok(mut c) = CARTS.lock() {
        c.clear();
    }
}

pub fn bumped(list: &[usize], at: DVec3, vel: DVec3, mass: f64) {
    if let Ok(mut q) = BUMPS.lock() {
        q.extend(list.iter().map(|&b| (b, at, vel, mass)));
    }
}

/// HangingDecoration.TakeDamage ('Shot' / 'Exploded'): CalculateHit(HitLocation, Momentum).
pub fn hit(blocker: usize, from: DVec3, momentum: f64) {
    if let Ok(mut q) = HITS.lock() {
        q.push((blocker, from, momentum));
    }
}

pub fn ship_shake(bobble: DVec3, size: f64) {
    if let Ok(mut q) = SHAKES.lock() {
        q.push((bobble, size));
    }
}

fn smerp(a: f64, x: f64, y: f64) -> f64 {
    let a = a.clamp(0.0, 1.0);
    x + (y - x) * (3.0 * a * a - 2.0 * a * a * a)
}

impl Hanging {
    /// Timer: a new random gust.
    fn gust(&mut self, rnd: &mut impl FnMut() -> f64) {
        self.orig_wind = DVec3::new(4096.0 - rnd() * 8192.0, 4096.0 - rnd() * 8192.0, 4096.0 - rnd() * 8192.0);
        self.wind_timer = 0.0;
        self.wind_period = rnd() * 4.0 + 2.0;
        self.next_gust = self.wind_period;
    }

    /// CalculateHit: a swing away from `loc` (seen from the decoration at `me`), as strong as `vel` (capped).
    fn calculate_hit(&mut self, me: DVec3, loc: DVec3, vel: f64) {
        let d = loc - me;
        let yaw = (d.y.atan2(d.x) / std::f64::consts::TAU * 65536.0).round();
        let ang = std::f64::consts::TAU * (yaw / 65536.0);
        let mut np = (-ang.cos() * vel * 10.0).trunc();
        let mut nr = (ang.sin() * vel * 10.0).trunc();
        while np.abs() > 8192.0 {
            np = (np / 2.0).trunc();
            nr = (nr / 2.0).trunc();
        }
        while nr.abs() > 8192.0 {
            np = (np / 2.0).trunc();
            nr = (nr / 2.0).trunc();
        }
        if np > self.sway.x && self.sway.x >= 0.0 || np < self.sway.x && self.sway.x <= 0.0 {
            self.sway.x = np;
        }
        if nr > self.sway.z && self.sway.z >= 0.0 || nr < self.sway.z && self.sway.z <= 0.0 {
            self.sway.z = nr;
        }
        self.damping = 1.0;
        self.period = 2.0;
        if self.no_pitch {
            self.sway.x = 0.0;
        }
        if self.no_roll {
            self.sway.z = 0.0;
        }
    }
}

impl Cart {
    fn start_rolling(&mut self, d: &mut Deco, vel: DVec3) {
        if d.body.physics != BodyPhysics::Falling {
            d.body.physics = BodyPhysics::Rolling;
        }
        self.push_vel = DVec3::new(vel.x, vel.y, 0.0);
        d.body.velocity = self.push_vel;
        self.roll_timer = 2.0;
        self.just_pushed = true;
        self.push_timer = 0.5;
    }
}

#[allow(clippy::too_many_arguments)]
pub fn hanging_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<crate::LevelRes>,
    blockers: Res<crate::actors::Blockers>,
    mut fresh: Query<(Entity, &Deco), (Without<Hanging>, Without<Cart>, Added<Deco>)>,
    mut hanging: Query<(&mut Hanging, &mut Deco), Without<Cart>>,
    mut carts: Query<(&mut Cart, &mut Deco), Without<Hanging>>,
    mut seed: Local<u64>,
) {
    let _prof = crate::prof::scope("hanging::hanging_system");
    if *seed == 0 {
        *seed = 0x4841_4E47_494E_4721;
    }
    let mut rnd = || {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        (*seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let lvl = &level.0;
    for (e, d) in &mut fresh {
        let Some(ma) = lvl.mesh_actors.get(d.mesh_actor) else { continue };
        let a = &lvl.actors[ma.actor];
        if crate::classtree::is_a(&a.class, "HangingDecoration") {
            let p = &a.props;
            let b = |k: &str| matches!(find(p, k), Some(Value::Bool(true)));
            let rot = |k: &str| match find(p, k) {
                Some(Value::Rotator(r)) => DVec3::new(r[0] as f64, r[1] as f64, r[2] as f64),
                _ => DVec3::ZERO,
            };
            let mut h = Hanging {
                fixed: b("bFixedSwaying"),
                fixed_dir: rot("FixedSwayDirection"),
                fixed_period: match find(p, "FixedSwayPeriod") {
                    Some(Value::Float(v)) => *v as f64,
                    _ => 1.0,
                },
                randomize: b("bRandomize"),
                no_pitch: b("bNoPitch"),
                no_roll: b("bNoRoll"),
                orig_rot: d.rotation,
                period: 2.0,
                ..Default::default()
            };
            // BeginPlay: bRandomize starts the first gust.
            if h.randomize {
                h.gust(&mut rnd);
            }
            commands.entity(e).insert(h);
        } else if crate::classtree::is_a(&a.class, "Cart") {
            commands.entity(e).insert(Cart::default());
            if let (Some(b), Ok(mut c)) = (d.blocker, CARTS.lock()) {
                c.push(b);
            }
        }
    }
    let dt = time.delta_secs_f64().min(0.1);
    let bumps = BUMPS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    let hits = HITS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    let shakes = SHAKES.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    let of = |blk: Option<usize>, b: usize| blk == Some(b) && blockers.list.get(b).is_some_and(|c| c.radius > 0.0);

    for (mut h, mut d) in &mut hanging {
        let me = d.body.location;
        for &(b, at, vel, _) in &bumps {
            if of(d.blocker, b) {
                h.calculate_hit(me, at, vel.length());
                h.swaying = true;
            }
        }
        for &(b, from, m) in &hits {
            if of(d.blocker, b) {
                h.calculate_hit(me, from, m);
                h.swaying = true;
            }
        }
        for &(bobble, _) in &shakes {
            let v = DVec3::new(rnd() * 2.0 - 1.0, rnd() * 2.0 - 1.0, rnd() * 2.0 - 1.0).normalize_or_zero();
            let loc = me + v * (10.0 * rnd());
            h.calculate_hit(me, loc, (bobble * 0.5).length());
            h.swaying = true;
        }
        // Tick.
        let o = h.orig_rot;
        let mut rot = DVec3::new(o[0] as f64, o[1] as f64, o[2] as f64);
        let mut moving = false;
        if h.fixed || h.randomize {
            // Timer (bRandomize): the next gust.
            if h.randomize {
                h.next_gust -= dt;
                if h.next_gust <= 0.0 {
                    h.gust(&mut rnd);
                }
            }
            // UpdateWind: eased toward the gust over the first half, back over the second.
            let a = if h.wind_period > 0.0 { h.wind_timer / h.wind_period } else { 0.0 };
            let ow = h.orig_wind;
            h.wind = if h.wind_timer < h.wind_period / 2.0 {
                DVec3::new(smerp(a, 0.0, ow.x), smerp(a, 0.0, ow.y), smerp(a, 0.0, ow.z))
            } else {
                DVec3::new(smerp(a, ow.x, 0.0), smerp(a, ow.y, 0.0), smerp(a, ow.z, 0.0))
            };
            h.wind_timer += dt;
            let ang = if h.fixed_period != 0.0 { std::f64::consts::TAU * h.fixed_timer / h.fixed_period } else { 0.0 };
            rot += h.wind + h.fixed_dir * ang.sin();
            h.fixed_timer += dt;
            if h.fixed_timer > h.fixed_period {
                h.fixed_timer -= h.fixed_period;
            }
            moving = true;
        }
        if h.swaying {
            let ang = std::f64::consts::TAU * h.timer / h.period;
            rot += h.sway * ang.sin() * h.damping;
            // swayDamping *= 0.995 per tick (the original's per-frame loss, taken at 60 Hz).
            h.damping *= 0.995f64.powf(dt * 60.0);
            if h.damping < 0.001 {
                h.swaying = false;
                h.sway = DVec3::ZERO;
            }
            h.timer += dt;
            if h.timer > h.period {
                h.timer -= h.period;
            }
            moving = true;
        }
        if moving {
            let r = [rot.x.round() as i32, rot.y.round() as i32, rot.z.round() as i32];
            if r != d.rotation {
                d.rotation = r;
                d.spun = true;
            }
        }
    }

    for (mut c, mut d) in &mut carts {
        let d = &mut *d;
        for &(b, _, vel, mass) in &bumps {
            // Cart.Bump: not while it was just pushed, not falling, not when landed on.
            if of(d.blocker, b) && !c.just_pushed && d.body.physics != BodyPhysics::Falling && d.mass > 0.0 {
                c.start_rolling(d, vel * 0.25 * mass / d.mass);
            }
        }
        for &(_, size) in &shakes {
            let v = DVec3::new(rnd() * 2.0 - 1.0, rnd() * 2.0 - 1.0, rnd() * 2.0 - 1.0).normalize_or_zero();
            c.start_rolling(d, DVec3::new(100.0, 100.0, 0.0) + v * (200.0 * size));
        }
        if d.body.physics == BodyPhysics::Rolling && c.roll_timer > 0.0 {
            c.roll_timer -= dt;
            d.body.velocity = c.push_vel;
            if c.push_timer > 0.0 {
                c.push_timer -= dt;
            } else {
                c.just_pushed = false;
            }
        } else if c.push_timer <= 0.0 {
            c.just_pushed = false;
        }
        // The rattle, pitched by speed (SoundPitch 2 x speed clamped 32..64, 64 = normal).
        let speed = d.body.velocity.length();
        if speed > 1.0 {
            let pitch = (2.0 * speed).clamp(32.0, 64.0) / 64.0;
            crate::sound::hold(crate::sound::key_of(&d.name) ^ 0xCA27, "DeusExSounds.Generic.UtilityCart", Some(d.body.location), 1.0, 128.0, pitch as f32);
        }
    }
}
