//! Character movement modeled on Unreal 1's PHYS_Walking / PHYS_Falling / PHYS_Swimming.
//!
//! Walking uses the classic move / step-up / step-down loop so the character climbs stairs and curbs up
//! to `max_step_height`, slides along walls, and starts falling when the ground disappears. Falling
//! integrates gravity with very limited air control. Behavior follows the original engine as documented
//! by SurrealEngine's reimplementation (zlib license), written fresh here.

use glam::DVec3;

/// Result of sweeping a box through the world.
#[derive(Debug, Clone, Copy)]
pub struct Hit {
    /// Fraction of the move that is free (1.0 = nothing hit).
    pub fraction: f64,
    /// Surface normal at the hit (Z up).
    pub normal: DVec3,
    /// The blocking actor, if the hit was an actor rather than world geometry.
    pub actor: Option<usize>,
}

impl Hit {
    pub const NONE: Hit = Hit { fraction: 1.0, normal: DVec3::ZERO, actor: None };
}

/// Anything the character can collide with.
pub trait CollisionWorld {
    /// Sweep an axis-aligned box with half-size `extent` from `start` to `end`.
    fn trace(&self, start: DVec3, end: DVec3, extent: DVec3) -> Hit;

    /// Mass of a pushable actor, if `actor` can be pushed by walking into it.
    fn pushable_mass(&self, _actor: usize) -> Option<f64> {
        None
    }

    /// The fluid friction of the water zone containing `p`, or None when `p` isn't in water.
    fn water_at(&self, _p: DVec3) -> Option<f64> {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Physics {
    Walking,
    Falling,
    /// No collision, no gravity (debug fly mode).
    Noclip,
    /// On a ladder: no gravity, `wish` may point up or down; collides like walking. Landing on a floor while
    /// moving down returns to Walking; leaving the ladder is up to the caller (switch to Falling).
    Climbing,
    /// In a water zone (the centre of the cylinder is in water): `wish` is a full 3D direction, buoyancy against
    /// gravity, fluid friction. Leaving the water switches to Falling.
    Swimming,
}

/// Tuning for one character; defaults are Deus Ex's `JCDentonMale` + `ZoneInfo` values.
#[derive(Debug, Clone)]
pub struct MoveParams {
    pub radius: f64,
    pub height: f64,
    pub ground_speed: f64,
    pub accel_rate: f64,
    pub jump_z: f64,
    pub air_control: f64,
    pub max_step_height: f64,
    pub gravity: DVec3,
    pub ground_friction: f64,
    pub terminal_velocity: f64,
    pub mass: f64,
    /// Top swimming speed (Pawn.WaterSpeed).
    pub water_speed: f64,
    pub buoyancy: f64,
    /// AI walkers (bAvoidLedges / bCanWalkOffLedges false): stop at a drop instead of walking off it.
    pub stop_at_ledges: bool,
}

impl Default for MoveParams {
    fn default() -> Self {
        MoveParams {
            stop_at_ledges: false,
            radius: 20.0,
            height: 47.5,
            ground_speed: 320.0,
            accel_rate: 1000.0,
            jump_z: 300.0,
            air_control: 0.05,
            max_step_height: 25.0,
            gravity: DVec3::new(0.0, 0.0, -950.0),
            ground_friction: 8.0,
            terminal_velocity: 2500.0,
            mass: 150.0,
            water_speed: 300.0,
            buoyancy: 155.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Character {
    pub location: DVec3,
    pub velocity: DVec3,
    pub physics: Physics,
    /// Set for one tick when the character lands (with the impact speed), for footstep/damage hooks.
    pub landed_speed: Option<f64>,
    /// Actors this character ran into during the last tick (Unreal's `Bump`).
    pub bumped: Vec<usize>,
    /// Velocity the character was trying to move with this tick, before collisions cut it short. This is
    /// what Unreal's `Bump` sees as `Other.Velocity` (pushing a crate head-on would otherwise read zero).
    pub intended_velocity: DVec3,
}

/// Floors steeper than this (normal Z below it) are treated as walls.
const WALKABLE_Z: f64 = 0.7071;

impl Character {
    pub fn new(location: DVec3) -> Character {
        Character { location, velocity: DVec3::ZERO, physics: Physics::Falling, landed_speed: None, bumped: Vec::new(), intended_velocity: DVec3::ZERO }
    }

    fn extent(p: &MoveParams) -> DVec3 {
        DVec3::new(p.radius, p.radius, p.height)
    }

    /// Move by `delta`, stopping at the first blocking surface.
    fn try_move(&mut self, w: &dyn CollisionWorld, p: &MoveParams, delta: DVec3) -> Hit {
        if delta.length_squared() < 1e-8 {
            return Hit::NONE;
        }
        let hit = w.trace(self.location, self.location + delta, Self::extent(p));
        self.location += delta * hit.fraction;
        if let Some(a) = hit.actor {
            if !self.bumped.contains(&a) {
                self.bumped.push(a);
            }
        }
        hit
    }

    /// Nudge the character upward out of the floor if it starts embedded (spawn points sit low).
    pub fn settle(&mut self, w: &dyn CollisionWorld, p: &MoveParams) {
        let e = Self::extent(p);
        for _ in 0..16 {
            let h = w.trace(self.location, self.location + DVec3::new(0.0, 0.0, -0.5), e);
            if h.fraction > 0.0 {
                break;
            }
            self.location.z += 4.0;
        }
        let down = w.trace(self.location, self.location - DVec3::Z * 64.0, e);
        self.location.z -= 64.0 * down.fraction;
        self.physics = if down.fraction < 1.0 { Physics::Walking } else { Physics::Falling };
    }

    /// Advance one tick. `wish` is the desired move direction in world space (Z ignored when walking),
    /// length 0..1. `jump` starts a jump if standing.
    pub fn tick(&mut self, w: &dyn CollisionWorld, p: &MoveParams, wish: DVec3, jump: bool, dt: f64) {
        self.landed_speed = None;
        if dt <= 0.0 {
            return;
        }
        // ZoneChange into a water zone (the cylinder's centre is in water): start swimming.
        if matches!(self.physics, Physics::Walking | Physics::Falling | Physics::Climbing) && w.water_at(self.location).is_some() {
            self.physics = Physics::Swimming;
        }
        match self.physics {
            Physics::Noclip => {
                self.velocity = wish * p.ground_speed * 3.0;
                self.location += self.velocity * dt;
            }
            Physics::Walking => {
                if jump {
                    self.velocity.z = p.jump_z;
                    self.physics = Physics::Falling;
                    self.tick_falling(w, p, wish, dt);
                } else {
                    self.tick_walking(w, p, wish, dt);
                }
            }
            Physics::Falling => self.tick_falling(w, p, wish, dt),
            Physics::Swimming => self.tick_swimming(w, p, wish, dt),
            Physics::Climbing => {
                if jump {
                    // Jumping off: push away from the ladder (the caller passes the facing in wish).
                    self.velocity = DVec3::new(-wish.x, -wish.y, 0.0) * p.ground_speed * 0.5 + DVec3::Z * p.jump_z * 0.5;
                    self.physics = Physics::Falling;
                    self.tick_falling(w, p, DVec3::ZERO, dt);
                } else {
                    self.tick_climbing(w, p, wish, dt);
                }
            }
        }
    }

    fn tick_climbing(&mut self, w: &dyn CollisionWorld, p: &MoveParams, wish: DVec3, dt: f64) {
        let speed = p.ground_speed * 0.5;
        let accel = if wish.length_squared() > 1e-6 { wish.normalize() * p.accel_rate * wish.length().min(1.0) } else { DVec3::ZERO };
        self.apply_accel(dt, accel, p.accel_rate, 8.0, speed);
        self.intended_velocity = self.velocity;
        let start = self.location;
        let mut time_left = dt;
        for _ in 0..4 {
            if time_left <= 0.0 {
                break;
            }
            let delta = self.velocity * time_left;
            let hit = self.try_move(w, p, delta);
            time_left -= time_left * hit.fraction;
            if hit.fraction < 1.0 {
                if hit.normal.z >= WALKABLE_Z && self.velocity.z <= 0.0 {
                    self.physics = Physics::Walking;
                    self.velocity.z = 0.0;
                    return;
                }
                // Pressed flat against the rungs a sweep can stick at once: step back off the wall a hair.
                if hit.fraction < 0.01 && hit.normal.z.abs() < 0.7 {
                    self.try_move(w, p, hit.normal * 0.5);
                }
                let slide = (delta - hit.normal * delta.dot(hit.normal)) * (1.0 - hit.fraction);
                self.try_move(w, p, slide);
                time_left = 0.0;
            }
        }
        if dt > 0.0 {
            let moved = (self.location - start) / dt;
            if moved.length() < self.velocity.length() {
                self.velocity = moved;
            }
        }
    }

    fn apply_accel(&mut self, dt: f64, accel: DVec3, accel_rate: f64, friction: f64, max_speed: f64) {
        let mut accel = accel;
        if accel.length_squared() > 1e-4 {
            let a = accel.length();
            let dir = accel / a;
            if a > accel_rate {
                accel = dir * accel_rate;
            }
            let speed = self.velocity.length();
            self.velocity -= (self.velocity - dir * speed) * (friction * dt).min(1.0);
        } else {
            let speed = self.velocity.length();
            if speed > 0.0 {
                let ns = (speed - speed * friction * 2.0 * dt).max(0.0);
                self.velocity *= ns / speed;
            }
        }
        self.velocity += accel * dt;
        let speed = self.velocity.length();
        if speed > max_speed {
            self.velocity *= max_speed / speed;
        }
    }

    fn step_to_ground_ok_after_push(&mut self, w: &dyn CollisionWorld, p: &MoveParams, pushed: bool) -> bool {
        if !pushed {
            return true;
        }
        if !self.step_to_ground(w, p, -DVec3::Z * p.max_step_height) {
            self.physics = Physics::Falling;
            return false;
        }
        true
    }

    /// Step down onto ground within `down`; false if there is no walkable floor there.
    fn step_to_ground(&mut self, w: &dyn CollisionWorld, p: &MoveParams, down: DVec3) -> bool {
        let h = w.trace(self.location, self.location + down, Self::extent(p));
        if h.fraction >= 1.0 || h.normal.z < WALKABLE_Z {
            return false;
        }
        self.location += down * h.fraction;
        true
    }

    fn tick_walking(&mut self, w: &dyn CollisionWorld, p: &MoveParams, wish: DVec3, dt: f64) {
        let start = self.location;
        let flat = DVec3::new(wish.x, wish.y, 0.0);
        let accel = if flat.length_squared() > 1e-6 { flat.normalize() * p.accel_rate * flat.length().min(1.0) } else { DVec3::ZERO };
        self.velocity.z = 0.0;
        self.apply_accel(dt, accel, p.accel_rate, p.ground_friction, p.ground_speed);
        self.velocity.z = 0.0;
        self.intended_velocity = self.velocity;

        let up = DVec3::Z * p.max_step_height;
        let down = -DVec3::Z * p.max_step_height;
        let mut vel = self.velocity;
        let mut time_left = dt;
        let mut keep_velocity = false;

        if vel.length_squared() > 0.0 {
            for _ in 0..5 {
                if time_left <= 0.0 {
                    break;
                }
                let mut delta = vel * time_left;
                let mut hit = self.try_move(w, p, delta);
                time_left -= time_left * hit.fraction;
                delta = vel * time_left;

                // Walking into something pushable: keep moving, slowed by the mass ratio (Unreal sets
                // bJustTeleported here so the velocity is not recomputed from the blocked displacement).
                if let Some(m) = hit.actor.and_then(|a| w.pushable_mass(a)) {
                    if hit.normal.dot(vel.normalize_or_zero()) < -0.5 {
                        vel *= p.mass / (p.mass + m);
                        self.velocity = vel;
                        keep_velocity = true;
                        break;
                    }
                }

                if hit.fraction < 1.0 {
                    // Blocked: step up, try again, then step back down.
                    self.try_move(w, p, up);
                    hit = self.try_move(w, p, delta);
                    time_left -= time_left * hit.fraction;
                    self.try_move(w, p, down);
                }
                if hit.fraction < 1.0 && hit.normal.z.abs() < WALKABLE_Z {
                    // Wall: slide along it.
                    let slide = (delta - hit.normal * delta.dot(hit.normal)) * (1.0 - hit.fraction);
                    if delta.dot(slide) >= 0.0 {
                        let h2 = self.try_move(w, p, slide);
                        time_left -= time_left * h2.fraction;
                    } else {
                        time_left = 0.0;
                    }
                }
                if !self.step_to_ground(w, p, down) {
                    if p.stop_at_ledges {
                        // Back onto the ground we were on; stand at the edge.
                        self.location = start;
                        self.velocity = DVec3::ZERO;
                        let _ = self.step_to_ground(w, p, down);
                        return;
                    }
                    // Walked off a ledge.
                    self.physics = Physics::Falling;
                    self.velocity = (self.location - start) / dt;
                    self.velocity.z = 0.0;
                    return;
                }
            }
        } else if !self.step_to_ground(w, p, down) {
            self.physics = Physics::Falling;
            return;
        }
        if !self.step_to_ground_ok_after_push(w, p, keep_velocity) {
            return;
        }
        if keep_velocity {
            return;
        }
        // Velocity follows what actually happened (walls stop you).
        self.velocity = (self.location - start) / dt;
        self.velocity.z = 0.0;
    }

    fn tick_falling(&mut self, w: &dyn CollisionWorld, p: &MoveParams, wish: DVec3, dt: f64) {
        let flat = DVec3::new(wish.x, wish.y, 0.0);
        let max_accel = p.air_control * p.accel_rate;
        let accel = if flat.length_squared() > 1e-6 { flat.normalize() * max_accel * flat.length().min(1.0) } else { DVec3::ZERO };
        let old_xy = self.velocity.truncate();
        let mut v = self.velocity + (accel * 1.5 + p.gravity * 2.0) * 0.5 * dt;
        // Air control can steer but not exceed ground speed.
        let cur2 = old_xy.length_squared();
        let new_xy = v.truncate();
        if cur2 >= p.ground_speed * p.ground_speed && new_xy.length_squared() > cur2 {
            let s = cur2.sqrt();
            let n = new_xy.normalize() * s;
            v.x = n.x;
            v.y = n.y;
        }
        if v.length() > p.terminal_velocity {
            v = v.normalize() * p.terminal_velocity;
        }
        self.velocity = v;
        self.intended_velocity = v;

        let start = self.location;
        let mut time_left = dt;
        for _ in 0..5 {
            if time_left <= 0.0 {
                break;
            }
            let delta = self.velocity * time_left;
            let hit = self.try_move(w, p, delta);
            time_left -= time_left * hit.fraction;
            if hit.fraction < 1.0 {
                if hit.normal.z >= WALKABLE_Z {
                    self.land();
                    return;
                }
                // Slide along slopes and walls.
                let slide = (delta - hit.normal * delta.dot(hit.normal)) * (1.0 - hit.fraction);
                if delta.dot(slide) >= 0.0 {
                    let h2 = self.try_move(w, p, slide);
                    if h2.fraction < 1.0 && h2.normal.z >= WALKABLE_Z {
                        self.land();
                        return;
                    }
                }
                let vz = self.velocity.z;
                self.velocity = (self.location - start) / dt;
                // Keep falling speed when sliding down a wall instead of floating.
                self.velocity.z = self.velocity.z.min(vz.max(self.velocity.z));
                time_left = 0.0;
            }
        }
    }

    /// PHYS_Swimming (SurrealEngine's UActor::TickSwimming, written fresh): accelerate at AccelRate x 0.3 toward
    /// `wish` (3D, length 0..1) up to WaterSpeed against the zone's fluid friction, gravity scaled by
    /// (1 - Buoyancy / Mass) (JC floats up slowly), slide along walls, and step up out of the water at the edge.
    fn tick_swimming(&mut self, w: &dyn CollisionWorld, p: &MoveParams, wish: DVec3, dt: f64) {
        let Some(friction) = w.water_at(self.location) else {
            self.physics = Physics::Falling;
            self.tick_falling(w, p, wish, dt);
            return;
        };
        let rate = p.accel_rate * 0.3;
        let accel = if wish.length_squared() > 1e-6 { wish.normalize() * rate * wish.length().min(1.0) } else { DVec3::ZERO };
        self.apply_accel(dt, accel, rate, friction, p.water_speed);
        let mut vel = self.velocity;
        vel.z += p.gravity.z * (1.0 - p.buoyancy / p.mass) * dt;
        self.intended_velocity = vel;
        let start = self.location;
        let step_up = DVec3::Z * p.max_step_height;
        let mut time_left = dt;
        if vel.length_squared() > 0.0 {
            for _ in 0..5 {
                if time_left <= 0.0 {
                    break;
                }
                let delta = vel * time_left;
                let hit = self.try_move(w, p, delta);
                time_left -= time_left * hit.fraction;
                if hit.fraction >= 1.0 {
                    continue;
                }
                let delta = vel * time_left;
                if let Some(m) = hit.actor.and_then(|a| w.pushable_mass(a)) {
                    if hit.normal.dot(delta.normalize_or_zero()) < -0.9 {
                        self.velocity = self.velocity * p.mass / (p.mass + m);
                        return;
                    }
                }
                // A wall: try it a step higher; if that lifts us out of the water, climb out onto the edge.
                self.try_move(w, p, step_up);
                if w.water_at(self.location).is_none() {
                    // Climbed out onto the edge: keep the swimming velocity (bJustTeleported) - the step itself
                    // isn't speed, or the 25-unit hop would launch us at thousands of units a second.
                    self.try_move(w, p, delta);
                    self.physics = Physics::Falling;
                    self.velocity = DVec3::new(vel.x, vel.y, vel.z.max(0.0));
                    return;
                }
                self.try_move(w, p, -step_up);
                let slide = delta - hit.normal * delta.dot(hit.normal);
                if delta.dot(slide) >= 0.0 {
                    let h2 = self.try_move(w, p, slide);
                    time_left -= time_left * h2.fraction;
                } else {
                    time_left = 0.0;
                }
            }
        }
        self.velocity = (self.location - start) / dt;
        if w.water_at(self.location).is_none() {
            self.physics = Physics::Falling;
        }
    }

    fn land(&mut self) {
        self.landed_speed = Some(-self.velocity.z);
        self.velocity.z = 0.0;
        self.physics = Physics::Walking;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Infinite floor at z = 0 plus a 20-unit-high step at x >= 100 and a wall at x >= 300.
    struct Steps;
    impl CollisionWorld for Steps {
        fn trace(&self, s: DVec3, e: DVec3, ext: DVec3) -> Hit {
            let mut best = Hit::NONE;
            // Solids as boxes: (min, max).
            let solids = [
                (DVec3::new(-1e5, -1e5, -100.0), DVec3::new(1e5, 1e5, 0.0)),
                (DVec3::new(100.0, -1e5, -100.0), DVec3::new(1e5, 1e5, 20.0)),
                (DVec3::new(300.0, -1e5, -100.0), DVec3::new(1e5, 1e5, 1000.0)),
            ];
            for (mn, mx) in solids {
                let (mn, mx) = (mn - ext, mx + ext);
                let d = e - s;
                let (mut t0, mut t1, mut n) = (-1.0f64, 1.0f64, DVec3::ZERO);
                let mut ok = true;
                for a in 0..3 {
                    if d[a].abs() < 1e-12 {
                        if s[a] < mn[a] || s[a] > mx[a] {
                            ok = false;
                        }
                        continue;
                    }
                    let (mut ta, mut tb) = ((mn[a] - s[a]) / d[a], (mx[a] - s[a]) / d[a]);
                    let mut na = DVec3::ZERO;
                    na[a] = -d[a].signum();
                    if ta > tb {
                        std::mem::swap(&mut ta, &mut tb);
                    }
                    if ta > t0 {
                        t0 = ta;
                        n = na;
                    }
                    t1 = t1.min(tb);
                }
                if ok && t0 <= t1 && t0 >= -1e-9 && t0 < best.fraction {
                    let len = d.length();
                    best = Hit { fraction: ((t0 * len - 0.1) / len).max(0.0), normal: n, actor: None };
                }
            }
            best
        }
    }

    #[test]
    fn walks_up_step_and_stops_at_wall() {
        let p = MoveParams::default();
        let mut c = Character::new(DVec3::new(0.0, 0.0, 47.6));
        c.settle(&Steps, &p);
        assert_eq!(c.physics, Physics::Walking);
        for _ in 0..240 {
            c.tick(&Steps, &p, DVec3::X, false, 1.0 / 60.0);
        }
        // Climbed the 20-unit step and stopped before the wall (x = 300 - radius).
        assert!(c.location.z > 60.0, "z = {}", c.location.z);
        assert!(c.location.x < 281.0 && c.location.x > 270.0, "x = {}", c.location.x);
    }

    #[test]
    fn jump_goes_up_and_lands() {
        let p = MoveParams::default();
        let mut c = Character::new(DVec3::new(0.0, 0.0, 47.6));
        c.settle(&Steps, &p);
        c.tick(&Steps, &p, DVec3::ZERO, true, 1.0 / 60.0);
        assert_eq!(c.physics, Physics::Falling);
        let mut peak: f64 = 0.0;
        let mut landed = false;
        for _ in 0..120 {
            c.tick(&Steps, &p, DVec3::ZERO, false, 1.0 / 60.0);
            peak = peak.max(c.location.z);
            landed |= c.landed_speed.is_some();
        }
        assert!(landed);
        // v^2 / 2g = 300^2 / 1900 ≈ 47 units of rise.
        assert!((peak - 47.5 - 47.0).abs() < 6.0, "peak {}", peak);
    }

    /// The step world flooded below z = 150.
    struct Pool;
    impl CollisionWorld for Pool {
        fn trace(&self, s: DVec3, e: DVec3, ext: DVec3) -> Hit {
            Steps.trace(s, e, ext)
        }
        fn water_at(&self, p: DVec3) -> Option<f64> {
            (p.z < 150.0).then_some(1.2)
        }
    }

    #[test]
    fn floats_to_the_surface_and_swims() {
        let mut p = MoveParams::default();
        p.height = 16.0;
        let mut c = Character::new(DVec3::new(0.0, 0.0, 60.0));
        c.tick(&Pool, &p, DVec3::ZERO, false, 1.0 / 60.0);
        assert_eq!(c.physics, Physics::Swimming);
        for _ in 0..60 * 20 {
            c.tick(&Pool, &p, DVec3::ZERO, false, 1.0 / 60.0);
        }
        // Buoyancy 155 > Mass 150: bobbing at the surface.
        assert!((c.location.z - 150.0).abs() < 15.0, "z = {}", c.location.z);
        // Swimming forward tops out at WaterSpeed.
        let mut c = Character::new(DVec3::new(-200.0, 0.0, 60.0));
        c.physics = Physics::Swimming;
        for _ in 0..60 {
            c.tick(&Pool, &p, DVec3::X, false, 1.0 / 60.0);
        }
        let v = c.velocity.truncate().length();
        assert!(v > 200.0 && v <= 300.5, "v = {v}");
    }

    /// Water up to z = 30 against the 20-unit step at x = 100.
    struct Shallows;
    impl CollisionWorld for Shallows {
        fn trace(&self, s: DVec3, e: DVec3, ext: DVec3) -> Hit {
            Steps.trace(s, e, ext)
        }
        fn water_at(&self, p: DVec3) -> Option<f64> {
            (p.z < 30.0).then_some(1.2)
        }
    }

    #[test]
    fn climbing_out_at_the_edge_does_not_launch() {
        let mut p = MoveParams::default();
        p.height = 16.0;
        let mut c = Character::new(DVec3::new(60.0, 0.0, 17.0));
        c.physics = Physics::Swimming;
        let mut max_vz: f64 = 0.0;
        for _ in 0..240 {
            c.tick(&Shallows, &p, DVec3::X, false, 1.0 / 120.0);
            max_vz = max_vz.max(c.velocity.z);
        }
        assert!(max_vz < 400.0, "launched at {max_vz}");
        assert!(c.location.x > 100.0, "x = {}", c.location.x);
    }
}
