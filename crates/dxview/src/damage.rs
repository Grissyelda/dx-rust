//! Damage to pawns, ported from ScriptedPawn.TakeDamageBase / HandleDamage / GenerateTotalHealth and
//! Robot.TakeDamageBase: per-body-part health, hit locations from where the shot landed, the stun and
//! point-blank-from-behind multipliers, and the reaction (hit animation, stun, death) that follows.

use bevy::prelude::*;
use glam::DVec3;
use unreal_pkg::props::{find, Property, Value};

/// A hit on a pawn this frame.
#[derive(Debug, Clone)]
pub struct PawnHit {
    pub blocker: usize,
    pub damage: f32,
    pub damage_type: &'static str,
    /// Where the shot landed (Unreal units).
    pub hit_loc: DVec3,
    /// Who did it: None = the player, Some(blocker) = another pawn.
    pub attacker: Option<usize>,
    pub attacker_pos: DVec3,
}

#[derive(Resource, Default)]
pub struct PawnHits(pub Vec<PawnHit>);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HitLoc {
    None,
    HeadFront,
    HeadBack,
    TorsoFront,
    TorsoBack,
    LeftArmFront,
    LeftArmBack,
    RightArmFront,
    RightArmBack,
    LeftLeg,
    RightLeg,
}

impl HitLoc {
    /// ScriptedPawn.PlayTakingHit animation for this hit location.
    pub fn anim(self) -> Option<&'static str> {
        Some(match self {
            HitLoc::None => return None,
            HitLoc::HeadFront => "HitHead",
            HitLoc::TorsoFront => "HitTorso",
            HitLoc::LeftArmFront => "HitArmLeft",
            HitLoc::RightArmFront => "HitArmRight",
            HitLoc::HeadBack => "HitHeadBack",
            HitLoc::TorsoBack | HitLoc::LeftArmBack | HitLoc::RightArmBack => "HitTorsoBack",
            HitLoc::LeftLeg => "HitLegLeft",
            HitLoc::RightLeg => "HitLegRight",
        })
    }
}

/// Per-part health (head, torso, left arm, right arm, left leg, right leg) and the pawn's total.
#[derive(Debug, Clone)]
pub struct Body {
    pub parts: [f32; 6],
    pub defaults: [f32; 6],
    pub health: f32,
    pub max_health: f32,
    pub robot: bool,
    pub show_pain: bool,
    pub radius: f64,
    pub height: f64,
    /// ScriptedPawn.bInvincible: GenerateTotalHealth leaves Health alone, so it never dies (MIBs until Mission04
    /// lets them die, Gunther until the killphrase, scripted bosses).
    pub invincible: bool,
}

fn fv(p: &[Property], n: &str, d: f32) -> f32 {
    match find(p, n) {
        Some(Value::Float(v)) => *v,
        Some(Value::Int(v)) => *v as f32,
        Some(Value::Byte(v)) => *v as f32,
        _ => d,
    }
}

const HEAD: usize = 0;
const TORSO: usize = 1;
const ARM_L: usize = 2;
const ARM_R: usize = 3;
const LEG_L: usize = 4;
const LEG_R: usize = 5;

/// What happened to the pawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outcome {
    Ignored,
    Hurt {
        hit: HitLoc,
        pain: bool,
        stunned: bool,
    },
    Died {
        knocked_out: bool,
        fall_back: bool,
    },
}

impl Body {
    pub fn new(p: &[Property]) -> Body {
        let health = fv(p, "Health", 50.0);
        let part = |n: &str| fv(p, n, health);
        let defaults = [
            part("HealthHead"),
            part("HealthTorso"),
            part("HealthArmLeft"),
            part("HealthArmRight"),
            part("HealthLegLeft"),
            part("HealthLegRight"),
        ];
        Body {
            parts: defaults,
            defaults,
            health,
            max_health: health,
            robot: find(p, "EMPHitPoints").is_some(),
            show_pain: !matches!(find(p, "bShowPain"), Some(Value::Bool(false))),
            radius: fv(p, "CollisionRadius", 20.0) as f64,
            height: fv(p, "CollisionHeight", 47.5) as f64,
            invincible: matches!(find(p, "bInvincible"), Some(Value::Bool(true))),
        }
    }

    pub fn total(&mut self) {
        if self.invincible {
            return;
        }
        let frac = |i: usize| {
            if self.defaults[i] > 0.0 {
                (self.defaults[i] - self.parts[i]) / self.defaults[i]
            } else {
                0.0
            }
        };
        let limbs = (frac(ARM_L) + frac(ARM_R) + frac(LEG_L) + frac(LEG_R)) * 0.4;
        let damage = limbs + frac(HEAD) + frac(TORSO);
        self.health = (self.max_health - damage * self.max_health).clamp(0.0, self.max_health);
    }

    /// Apply one hit. `center`/`yaw` are the pawn's location and facing; `stunned` whether it is stunned now.
    pub fn take(
        &mut self,
        hit: &PawnHit,
        center: DVec3,
        yaw: f64,
        stunned: bool,
        pain_ready: bool,
    ) -> Outcome {
        let dt = hit.damage_type;
        if self.robot {
            // Robot.TakeDamageBase: no gas/poison/knockout; gunshots and flames 1/4, stun 1/2; never stunned.
            if matches!(
                dt,
                "TearGas"
                    | "HalonGas"
                    | "PoisonGas"
                    | "Radiation"
                    | "Poison"
                    | "PoisonEffect"
                    | "KnockedOut"
                    | "EMP"
                    | "NanoVirus"
            ) {
                return Outcome::Ignored;
            }
            let mut d = hit.damage
                * match dt {
                    "Shot" | "Flamed" | "Burned" => 0.25,
                    "Stunned" => 0.5,
                    _ => 1.0,
                };
            if d > 0.01 && d < 1.0 {
                d = 1.0;
            }
            if !self.invincible {
                self.health -= (d + 0.5).floor();
            }
            if self.health <= 0.0 {
                return Outcome::Died {
                    knocked_out: false,
                    fall_back: false,
                };
            }
            return Outcome::Hurt {
                hit: HitLoc::None,
                pain: false,
                stunned: false,
            };
        }

        // Hit location in object space: X front-to-back, Y side-to-side, Z up.
        let (s, c) = yaw.sin_cos();
        let o = hit.hit_loc - center;
        let off = DVec3::new(o.x * c + o.y * s, -o.x * s + o.y * c, o.z);

        // ModifyDamage: stunned pawns take 4x; point blank from behind is 10x.
        let mut dmg = hit.damage.floor();
        if stunned {
            dmg *= 4.0;
        } else if off.x < 0.0 && hit.attacker_pos.distance(center) < 64.0 {
            dmg *= 10.0;
        }
        if matches!(dt, "TearGas" | "EMP" | "NanoVirus") {
            dmg = 0.0;
        }

        // HandleDamage.
        let head_z = self.height * 0.7;
        let head_y = self.radius * 0.3;
        let arm = self.radius * 0.35;
        let stun_type = matches!(dt, "Stunned" | "KnockedOut");
        let mut loc = HitLoc::None;
        if dmg > 0.0 {
            if off.z > head_z {
                if off.x.abs() < head_y || off.y.abs() < head_y {
                    self.parts[HEAD] -= if stun_type { dmg } else { dmg * 8.0 };
                    loc = if off.x < 0.0 {
                        HitLoc::HeadBack
                    } else {
                        HitLoc::HeadFront
                    };
                } else {
                    self.parts[TORSO] -= dmg * 2.0;
                    loc = if off.x < 0.0 {
                        HitLoc::TorsoBack
                    } else {
                        HitLoc::TorsoFront
                    };
                }
            } else if off.z < 0.0 {
                if off.y > 0.0 {
                    self.parts[LEG_R] -= dmg * 2.0;
                    loc = HitLoc::RightLeg;
                } else {
                    self.parts[LEG_L] -= dmg * 2.0;
                    loc = HitLoc::LeftLeg;
                }
                // A dead leg passes damage to the other leg, then to the torso.
                if self.parts[LEG_R] < 0.0 && self.parts[LEG_L] > 0.0 {
                    self.parts[LEG_L] += self.parts[LEG_R];
                    self.parts[LEG_R] = 0.0;
                } else if self.parts[LEG_L] < 0.0 && self.parts[LEG_R] > 0.0 {
                    self.parts[LEG_R] += self.parts[LEG_L];
                    self.parts[LEG_L] = 0.0;
                }
                for i in [LEG_L, LEG_R] {
                    if self.parts[i] < 0.0 {
                        self.parts[TORSO] += self.parts[i];
                        self.parts[i] = 0.0;
                    }
                }
            } else {
                if off.y > arm {
                    self.parts[ARM_R] -= dmg * 2.0;
                    loc = if off.x < 0.0 {
                        HitLoc::RightArmBack
                    } else {
                        HitLoc::RightArmFront
                    };
                } else if off.y < -arm {
                    self.parts[ARM_L] -= dmg * 2.0;
                    loc = if off.x < 0.0 {
                        HitLoc::LeftArmBack
                    } else {
                        HitLoc::LeftArmFront
                    };
                } else {
                    self.parts[TORSO] -= dmg * 2.0;
                    loc = if off.x < 0.0 {
                        HitLoc::TorsoBack
                    } else {
                        HitLoc::TorsoFront
                    };
                }
                for i in [ARM_L, ARM_R] {
                    if self.parts[i] < 0.0 {
                        self.parts[TORSO] += self.parts[i];
                        self.parts[i] = 0.0;
                    }
                }
            }
        }
        self.total();

        if self.health <= 0.0 {
            // PlayDying: shot from the front falls back.
            let fwd = DVec3::new(c, s, 0.0);
            let fall_back = (center - hit.hit_loc).dot(fwd) < 0.0;
            return Outcome::Died {
                knocked_out: stun_type || matches!(dt, "Poison" | "PoisonEffect"),
                fall_back,
            };
        }
        let stunned_now = dt == "Stunned";
        Outcome::Hurt {
            hit: loc,
            pain: self.show_pain && pain_ready && loc != HitLoc::None && dmg > 0.0,
            stunned: stunned_now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trooper() -> Body {
        let p = vec![
            Property {
                name: "Health".into(),
                index: 0,
                value: Value::Int(100),
            },
            Property {
                name: "CollisionRadius".into(),
                index: 0,
                value: Value::Float(20.0),
            },
            Property {
                name: "CollisionHeight".into(),
                index: 0,
                value: Value::Float(47.5),
            },
        ];
        Body::new(&p)
    }
    fn hit(damage: f32, ty: &'static str, at: DVec3, from: DVec3) -> PawnHit {
        PawnHit {
            blocker: 0,
            damage,
            damage_type: ty,
            hit_loc: at,
            attacker: None,
            attacker_pos: from,
        }
    }

    #[test]
    fn prod_from_front_stuns_then_knocks_out() {
        let mut b = trooper();
        // Facing +X; prodded in the chest from 50 units in front.
        let from = DVec3::new(50.0, 0.0, 0.0);
        let o = b.take(
            &hit(15.0, "Stunned", DVec3::new(20.0, 0.0, 10.0), from),
            DVec3::ZERO,
            0.0,
            false,
            true,
        );
        assert!(matches!(o, Outcome::Hurt { stunned: true, .. }), "{o:?}");
        assert_eq!(b.health, 70.0);
        // While stunned damage is 4x: 60 * 2 to the torso finishes him.
        let o = b.take(
            &hit(15.0, "Stunned", DVec3::new(20.0, 0.0, 10.0), from),
            DVec3::ZERO,
            0.0,
            true,
            true,
        );
        assert!(
            matches!(
                o,
                Outcome::Died {
                    knocked_out: true,
                    ..
                }
            ),
            "{o:?}"
        );
    }

    #[test]
    fn head_shot_is_eight_times() {
        let mut b = trooper();
        let o = b.take(
            &hit(
                16.0,
                "Shot",
                DVec3::new(20.0, 0.0, 40.0),
                DVec3::new(500.0, 0.0, 40.0),
            ),
            DVec3::ZERO,
            0.0,
            false,
            true,
        );
        assert!(
            matches!(
                o,
                Outcome::Died {
                    knocked_out: false,
                    fall_back: true
                }
            ),
            "{o:?}"
        );
    }

    #[test]
    fn point_blank_from_behind_is_ten_times() {
        let mut b = trooper();
        let o = b.take(
            &hit(
                15.0,
                "Stunned",
                DVec3::new(-20.0, 0.0, 10.0),
                DVec3::new(-50.0, 0.0, 0.0),
            ),
            DVec3::ZERO,
            0.0,
            false,
            true,
        );
        assert!(
            matches!(
                o,
                Outcome::Died {
                    knocked_out: true,
                    ..
                }
            ),
            "{o:?}"
        );
    }
}
