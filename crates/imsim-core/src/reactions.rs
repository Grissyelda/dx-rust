//! What a character pays attention to and how scared it is (Deus Ex ScriptedPawn reaction callbacks:
//! SetReactions / BlockReactions / ResetReactions per state, IncreaseFear / UpdateFear / IsFearful).
//!
//! Each AI state switches whole kinds of stimuli on or off (a fleeing pawn ignores gunshots it hears, a searching
//! one ignores alarms and bodies); within a state the per-character bReact / bHate / bFear flags decide what a
//! stimulus does. Fear is a level 0..1 topped up by frightening things; it holds for FearSustainTime after the last
//! one, then drains at FearDecayRate per second. A character is fearful at 1.

/// SetReactions: the bLookingFor* switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LookingFor {
    pub enemy: bool,
    pub loud_noise: bool,
    pub alarm: bool,
    pub distress: bool,
    pub projectiles: bool,
    pub futz: bool,
    pub hacking: bool,
    pub shot: bool,
    pub weapon: bool,
    pub carcass: bool,
    pub injury: bool,
    pub indirect_injury: bool,
}

impl LookingFor {
    /// ResetReactions: everything (the orders states - Standing, Patrolling, Wandering, Sitting, ...).
    pub const ALL: LookingFor = LookingFor {
        enemy: true,
        loud_noise: true,
        alarm: true,
        distress: true,
        projectiles: true,
        futz: true,
        hacking: true,
        shot: true,
        weapon: true,
        carcass: true,
        injury: true,
        indirect_injury: true,
    };

    /// BlockReactions(bBlockInjury): nothing, injuries too only when `block_injury` (Attacking, Fleeing, Alerting,
    /// Burning, TakingHit, Stunned, RubbingEyes, BackingOff, OpeningDoor block all but injuries; Idle, Paralyzed,
    /// Conversation and Dying block injuries as well).
    pub fn blocked(block_injury: bool) -> LookingFor {
        LookingFor {
            enemy: false,
            loud_noise: false,
            alarm: false,
            distress: false,
            projectiles: false,
            futz: false,
            hacking: false,
            shot: false,
            weapon: false,
            carcass: false,
            injury: !block_injury,
            indirect_injury: !block_injury,
        }
    }

    /// Seeking: SetReactions(true, true, false, true, true, true, true, true, true, false, true, true) - no alarms,
    /// no bodies.
    pub const SEEKING: LookingFor = LookingFor { alarm: false, carcass: false, ..LookingFor::ALL };

    /// AvoidingProjectiles: everything but projectiles.
    pub const AVOIDING_PROJECTILES: LookingFor = LookingFor { projectiles: false, ..LookingFor::ALL };

    /// FirstPersonConversation: SetReactions(false, false, false, false, true, false, false, true, false, false,
    /// true, true) - only projectiles, shots and injuries break in.
    pub const FIRST_PERSON_CONVERSATION: LookingFor = LookingFor {
        enemy: false,
        loud_noise: false,
        alarm: false,
        distress: false,
        projectiles: true,
        futz: false,
        hacking: false,
        shot: true,
        weapon: false,
        carcass: false,
        injury: true,
        indirect_injury: true,
    };
}

/// FearLevel / FearTimer (IncreaseFear, UpdateFear, IsFearful).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Fear {
    pub level: f32,
    pub timer: f32,
    pub sustain_time: f32,
    pub decay_rate: f32,
}

impl Fear {
    pub fn new(sustain_time: f32, decay_rate: f32) -> Fear {
        Fear { level: 0.0, timer: 0.0, sustain_time, decay_rate }
    }

    /// IncreaseFear(instigator, added, newFearTimer): the timer is raised to FearSustainTime - newFearTimer, then
    /// the level grows (capped at 1) while the timer runs.
    pub fn increase(&mut self, added: f32, new_timer: f32) {
        if self.timer < self.sustain_time - new_timer {
            self.timer = self.sustain_time - new_timer;
        }
        if self.timer > 0.0 && added > 0.0 {
            self.level = (self.level + added).min(1.0);
        }
    }

    /// UpdateFear: the timer runs down; once it's out the level drains at FearDecayRate.
    pub fn tick(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        let decrement = if self.timer > 0.0 {
            if self.timer < dt {
                let mult = 1.0 - self.timer / dt;
                self.timer = 0.0;
                mult * self.decay_rate * dt
            } else {
                self.timer -= dt;
                0.0
            }
        } else {
            self.decay_rate * dt
        };
        if decrement > 0.0 && self.level > 0.0 {
            self.level = (self.level - decrement).max(0.0);
        }
    }

    /// IsFearful.
    pub fn fearful(&self) -> bool {
        self.level >= 1.0
    }
}

/// IsPrimaryDamageType: direct harm (bullets, blows, fire, stuns) as opposed to indirect (gas, explosions,
/// radiation, EMP, drowning) - bHateInjury / bFearInjury react to the first, bHate/FearIndirectInjury to the second.
pub fn is_primary_damage(damage_type: &str) -> bool {
    !matches!(
        damage_type,
        "Exploded" | "TearGas" | "HalonGas" | "PoisonGas" | "PoisonEffect" | "Radiation" | "EMP" | "Drowned" | "NanoVirus"
    )
}

/// ShouldReactToInjuryType.
pub fn reacts_to_injury(damage_type: &str, primary: bool, secondary: bool) -> bool {
    let p = is_primary_damage(damage_type);
    (primary && p) || (secondary && !p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_scare_is_enough_and_it_holds_then_drains() {
        let mut f = Fear::new(25.0, 0.5);
        assert!(!f.fearful());
        f.increase(1.0, 0.0);
        assert!(f.fearful());
        f.tick(24.0);
        assert!(f.fearful());
        // The timer runs out 1 s into this step: half a second's worth of decay.
        f.tick(2.0);
        assert!((f.level - 0.5).abs() < 1e-5);
        f.tick(1.0);
        assert_eq!(f.level, 0.0);
    }

    #[test]
    fn hacking_needs_two_looks() {
        // HandleHacking adds 0.51: twice is fearful.
        let mut f = Fear::new(25.0, 0.5);
        f.increase(0.51, 0.0);
        assert!(!f.fearful());
        f.increase(0.51, 0.0);
        assert!(f.fearful());
    }

    #[test]
    fn a_shortened_timer_does_not_lengthen() {
        // Distress from a fleeing friend: IncreaseFear(enemy, 1, friend's DistressTimer) holds only what's left.
        let mut f = Fear::new(25.0, 0.5);
        f.increase(1.0, 20.0);
        assert!((f.timer - 5.0).abs() < 1e-5);
    }

    #[test]
    fn state_tables() {
        assert!(!LookingFor::SEEKING.alarm && LookingFor::SEEKING.shot);
        let b = LookingFor::blocked(false);
        assert!(!b.shot && b.injury);
        assert!(!LookingFor::blocked(true).injury);
        assert!(is_primary_damage("Shot") && !is_primary_damage("TearGas"));
        assert!(reacts_to_injury("TearGas", false, true));
    }
}
