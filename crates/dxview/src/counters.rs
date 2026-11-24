//! The counter table (MULTIPLAYER roadmap Phase 3): every Modern power has an answer, kept in data
//! (data/deathmatch.ron `counters`) so it can be tuned after playtests without a rebuild of the rules around it.
//!
//! Each row is (power, answer, rule, value, note). The game's rules ask `value(power, answer, rule, default)` where
//! the answer applies: cover.rs (Vision and goggles against cloak), uniforms.rs (a close look, Vision), security.rs
//! (EMP against turrets), overwatch.rs (power cuts, EMP, exposure), lockdown.rs (multitool overrides, hazmat).
//! The rest (radar against cloak, eyes against radar transparency, evidence and hidden bodies against framing, a new
//! phrase each life) are how those modules already work; their rows document it, and the test below keeps every
//! power answered.

use serde::Deserialize;

#[derive(Deserialize, Clone, Debug)]
pub struct Counter {
    pub power: String,
    pub answer: String,
    pub rule: String,
    pub value: f32,
    #[serde(default)]
    pub note: String,
}

/// The powers a Modern match has; each must have at least one answer in the table.
pub const POWERS: [&str; 10] = ["AugCloak", "AugRadarTrans", "HackedTurret", "Uniform", "Overwatch", "Lockdown", "Framing", "Killphrase", "Downed", "AugBallistic"];

/// The rules the code reads.
pub const RULES: [&str; 14] = [
    "reveal_range",
    "radar",
    "seen_by_eyes",
    "disable_secs",
    "take_back",
    "unmask_secs",
    "unmask",
    "staff_suspicion",
    "out_secs",
    "cut_radius",
    "kick_on_hit",
    "override_secs",
    "hazard_mult",
    "pickup_clears",
];

/// More rules that only document behaviour.
pub const NOTES: [&str; 5] = ["carried_unseen", "new_phrase", "reach", "revive", "bypass"];

pub fn table() -> &'static [Counter] {
    &crate::deathmatch::data().counters
}

/// The value of an answer's rule, or `default` when the table doesn't have it.
pub fn value(power: &str, answer: &str, rule: &str, default: f32) -> f32 {
    table()
        .iter()
        .find(|c| c.power.eq_ignore_ascii_case(power) && c.answer.eq_ignore_ascii_case(answer) && c.rule.eq_ignore_ascii_case(rule))
        .map_or(default, |c| c.value)
}

/// The answers to a power (for the help screens).
pub fn answers(power: &str) -> Vec<&'static Counter> {
    table().iter().filter(|c| c.power.eq_ignore_ascii_case(power)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_power_has_an_answer() {
        for p in POWERS {
            assert!(!answers(p).is_empty(), "{p} has no answer in the counter table");
        }
        for c in table() {
            assert!(POWERS.contains(&c.power.as_str()), "unknown power {}", c.power);
            assert!(RULES.contains(&c.rule.as_str()) || NOTES.contains(&c.rule.as_str()), "unknown rule {}", c.rule);
            assert!(!c.note.is_empty(), "{} / {} has no note", c.power, c.answer);
        }
        assert_eq!(value("Overwatch", "PowerCut", "out_secs", 0.0), 45.0);
        assert_eq!(value("Nothing", "Nobody", "none", 7.0), 7.0);
    }
}
