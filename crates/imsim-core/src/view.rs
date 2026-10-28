//! First-person view motion: walking head bob and smoothed eye height, ported from Unreal's
//! `PlayerPawn.CheckBob` / `UpdateEyeHeight` as modified by Deus Ex (no idle "breathing" bob).

use glam::DVec3;

#[derive(Debug, Clone)]
pub struct ViewBob {
    /// Bob strength (Deus Ex `Bob` default 0.016).
    pub bob: f64,
    pub bob_time: f64,
    pub applied_bob: f64,
    pub land_bob: f64,
    /// Current eye height above the collision center (smoothed toward `BaseEyeHeight`).
    pub eye_height: f64,
    /// Offset added to the camera position.
    pub walk_bob: DVec3,
    just_landed: bool,
}

impl ViewBob {
    pub fn new(bob: f64, base_eye_height: f64) -> ViewBob {
        ViewBob { bob, bob_time: 0.0, applied_bob: 0.0, land_bob: 0.0, eye_height: base_eye_height, walk_bob: DVec3::ZERO, just_landed: false }
    }

    /// Call when the character lands with vertical velocity `vz` (negative when falling).
    pub fn landed(&mut self, vz: f64) {
        // As in PlayerPawn.Landed; with a falling (negative) velocity this stays below the 0.01 threshold,
        // so the original never actually applied a landing dip. Kept for fidelity.
        self.land_bob = (0.055 * vz).min(50.0);
        self.just_landed = true;
    }

    /// Advance one frame. `right` is the view's right vector (Unreal space), `z_moved` is how far the body
    /// moved vertically this frame (smooths the eye over steps).
    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, dt: f64, walking: bool, velocity: DVec3, ground_speed: f64, right: DVec3, base_eye: f64, z_moved: f64) {
        // UpdateEyeHeight
        if walking && !self.just_landed {
            let smooth = (10.0 * dt).min(1.0);
            self.eye_height = (self.eye_height - z_moved) * (1.0 - smooth) + base_eye * smooth;
        } else {
            let smooth = (10.0 * dt).clamp(0.35, 1.0);
            self.just_landed = false;
            self.eye_height = self.eye_height * (1.0 - smooth) + base_eye * smooth;
        }

        // CheckBob (only while walking), else decay.
        if walking {
            let speed2d = velocity.truncate().length();
            if speed2d < 10.0 {
                self.bob_time += 0.2 * dt;
            } else {
                self.bob_time += dt * (0.3 + 0.7 * speed2d / ground_speed.max(1.0));
            }
            let side = right * (0.65 * self.bob * speed2d * (6.0 * self.bob_time).sin());
            let k = (16.0 * dt).min(1.0);
            self.applied_bob *= 1.0 - k;
            if self.land_bob > 0.01 {
                self.applied_bob += k * self.land_bob;
                self.land_bob *= 1.0 - 8.0 * dt;
            }
            let z = if speed2d < 10.0 { 0.0 } else { self.applied_bob + self.bob * speed2d * (12.0 * self.bob_time).sin() };
            self.walk_bob = DVec3::new(side.x, side.y, z);
        } else {
            self.bob_time = 0.0;
            self.walk_bob *= 1.0 - (8.0 * dt).min(1.0);
        }
    }

    /// PlayerSwimming.PlayerMove's bob, in place of CheckBob: WalkBob = Y * Bob * 0.5 * speed2D * sin(4t),
    /// Z = Bob * 1.5 * speed2D * sin(8t), with t the level time.
    pub fn swim_bob(&mut self, time_seconds: f64, velocity: DVec3, right: DVec3) {
        let speed2d = velocity.truncate().length();
        let side = right * (self.bob * 0.5 * speed2d * (4.0 * time_seconds).sin());
        self.walk_bob = DVec3::new(side.x, side.y, self.bob * 1.5 * speed2d * (8.0 * time_seconds).sin());
    }

    /// Camera offset from the character's collision center.
    pub fn camera_offset(&self) -> DVec3 {
        DVec3::new(0.0, 0.0, self.eye_height) + self.walk_bob
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bob_amplitude_matches_original_formula() {
        let mut v = ViewBob::new(0.016, 40.0);
        let mut max_z: f64 = 0.0;
        for _ in 0..600 {
            v.update(1.0 / 60.0, true, DVec3::new(320.0, 0.0, 0.0), 320.0, DVec3::Y, 40.0, 0.0);
            max_z = max_z.max(v.walk_bob.z);
        }
        // Bob * Speed2D = 0.016 * 320 = 5.12 units.
        assert!((max_z - 5.12).abs() < 0.1, "{max_z}");
    }
}
