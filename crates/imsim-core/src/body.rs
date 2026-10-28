//! Non-pawn physics for movable objects (crates, barrels): Unreal's PHYS_Falling and PHYS_Rolling for
//! decorations. Falling objects drop until they land on a walkable floor and then rest; rolling objects
//! slide with ground friction (used when pushed), stepping down small ledges and falling off big ones.

use crate::movement::{CollisionWorld, Hit};
use glam::DVec3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyPhysics {
    None,
    Falling,
    Rolling,
}

#[derive(Debug, Clone)]
pub struct Body {
    pub location: DVec3,
    pub velocity: DVec3,
    pub physics: BodyPhysics,
    pub radius: f64,
    pub height: f64,
    /// Set for one tick when the body lands on a floor: the vertical speed it hit with.
    pub landed_vz: Option<f64>,
}

const WALKABLE_Z: f64 = 0.7071;

impl Body {
    fn extent(&self) -> DVec3 {
        DVec3::new(self.radius, self.radius, self.height)
    }

    fn try_move(&mut self, w: &dyn CollisionWorld, delta: DVec3) -> Hit {
        if delta.length_squared() < 1e-8 {
            return Hit::NONE;
        }
        let h = w.trace(self.location, self.location + delta, self.extent());
        self.location += delta * h.fraction;
        h
    }

    /// Advance one tick. Returns true if the body moved.
    pub fn tick(&mut self, w: &dyn CollisionWorld, gravity: DVec3, ground_friction: f64, dt: f64) -> bool {
        let start = self.location;
        self.landed_vz = None;
        match self.physics {
            BodyPhysics::None => return false,
            BodyPhysics::Falling => {
                self.velocity += gravity * dt;
                let delta = self.velocity * dt;
                let h = self.try_move(w, delta);
                if h.fraction < 1.0 {
                    if h.normal.z >= WALKABLE_Z {
                        // Landed: rest on the floor (Decoration.Landed lays it flat; rotation untouched here).
                        self.landed_vz = Some(self.velocity.z);
                        self.velocity = DVec3::ZERO;
                        self.physics = BodyPhysics::None;
                    } else {
                        let slide = (delta - h.normal * delta.dot(h.normal)) * (1.0 - h.fraction);
                        if delta.dot(slide) >= 0.0 {
                            self.try_move(w, slide);
                        }
                        let vz = self.velocity.z;
                        self.velocity = (self.location - start) / dt;
                        self.velocity.z = self.velocity.z.min(vz);
                    }
                }
            }
            BodyPhysics::Rolling => {
                let k = (ground_friction * dt).min(1.0);
                self.velocity.x *= 1.0 - k;
                self.velocity.y *= 1.0 - k;
                self.velocity.z = 0.0;
                if self.velocity.length_squared() < 1.0 {
                    self.velocity = DVec3::ZERO;
                    self.physics = BodyPhysics::None;
                    return false;
                }
                let delta = self.velocity * dt;
                let h = self.try_move(w, delta);
                if h.fraction < 1.0 && h.normal.z < WALKABLE_Z {
                    let slide = (delta - h.normal * delta.dot(h.normal)) * (1.0 - h.fraction);
                    if delta.dot(slide) >= 0.0 {
                        self.try_move(w, slide);
                    }
                }
                // Stay on the ground; fall off ledges taller than a step for this size.
                let down = DVec3::new(0.0, 0.0, -(25.0 / 47.5) * self.height);
                let g = w.trace(self.location, self.location + down, self.extent());
                if g.fraction >= 1.0 || g.normal.z < WALKABLE_Z {
                    self.physics = BodyPhysics::Falling;
                } else {
                    self.location += down * g.fraction;
                }
                self.velocity = (self.location - start) / dt;
                self.velocity.z = 0.0;
            }
        }
        (self.location - start).length_squared() > 1e-6
    }
}

/// Deus Ex `DeusExDecoration.Bump` push rule: the speed a pushed object takes from the pusher.
/// Returns `None` if the object is too heavy to move, else (new velocity, velocity scale).
/// `aug_mult` is 1 without the Muscle augmentation.
pub fn push_velocity(pusher_velocity: DVec3, mass: f64, aug_mult: f64) -> Option<(DVec3, f64)> {
    let max_push = 100.0 * aug_mult;
    if mass > max_push {
        return None;
    }
    let mut scale = ((50.0 * aug_mult) / mass).clamp(0.0, 1.0);
    if scale < 0.25 {
        scale = 0.0;
    }
    let mut v = pusher_velocity * scale;
    v.z = 0.0;
    Some((v, scale))
}

/// The fake spin a pushed decoration gets (DeusExDecoration.Bump): yaw change in Unreal units given the
/// pusher's direction relative to the object's facing (`local_yaw`, Unreal units) and the push scale.
pub fn push_spin(local_yaw: i32, scale: f64) -> i32 {
    let mut y = local_yaw.rem_euclid(65536);
    if y >= 32768 {
        y -= 65536;
    }
    // Ignore which side we're pushing from.
    if y >= 24576 {
        y -= 32768;
    } else if y >= 8192 {
        y -= 16384;
    } else if y <= -24576 {
        y += 32768;
    } else if y <= -8192 {
        y += 16384;
    }
    (y as f64 * scale * 0.025) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_rule_matches_script() {
        let v = DVec3::new(320.0, 0.0, 0.0);
        assert_eq!(push_velocity(v, 40.0, 1.0).unwrap().0.x, 320.0); // light crate: full speed
        assert_eq!(push_velocity(v, 100.0, 1.0).unwrap().0.x, 160.0); // 50/100
        assert_eq!(push_velocity(v, 250.0, 1.0), None); // too heavy without Muscle aug
        assert_eq!(push_velocity(v, 250.0, 3.0).unwrap().0.x, 192.0); // Muscle level 1: 150/250 = 0.6
    }
}
