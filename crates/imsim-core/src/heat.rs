//! Heat: how much a level's own security wants each player (engine-neutral).
//!
//! In a player-versus-player match on a living level the guards, cameras and turrets belong to the level and are
//! neutral to every player until one of them gives a reason. Each reason adds heat to that player only; past the
//! `wanted` mark the level's security hunts him (and nobody else), and he stays wanted while staff keep seeing
//! him. Out of sight for `hold_secs` the heat drains, and he is no longer wanted once it falls below `calm_below`
//! (the gap between the two marks keeps him from flickering in and out).
//!
//! Players are small numbers (the match's player ids). The tuning lives in data (the game passes it in).

use serde::{Deserialize, Serialize};

/// Heat levels, as shown to the player and used by the level's staff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum HeatLevel {
    /// Nobody cares.
    Calm,
    /// Staff are watching him (heat past `suspicious`).
    Suspicious,
    /// Staff attack him on sight.
    Wanted,
}

impl HeatLevel {
    pub fn code(self) -> u8 {
        match self {
            HeatLevel::Calm => 0,
            HeatLevel::Suspicious => 1,
            HeatLevel::Wanted => 2,
        }
    }
    pub fn from_code(c: u8) -> HeatLevel {
        match c {
            2 => HeatLevel::Wanted,
            1 => HeatLevel::Suspicious,
            _ => HeatLevel::Calm,
        }
    }
}

/// What makes heat and how fast it goes (all amounts are heat points; `*_rate` are per second of being seen).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HeatTuning {
    /// Heat at which staff start to watch.
    pub suspicious: f32,
    /// Heat at which staff attack.
    pub wanted: f32,
    /// A wanted player calms down only below this.
    pub calm_below: f32,
    /// The most heat a player can hold.
    pub max: f32,
    /// Seconds out of every staff member's sight before heat drains.
    pub hold_secs: f32,
    /// Heat lost per second once draining.
    pub decay: f32,
    /// An unsilenced shot heard by staff.
    pub shot_heard: f32,
    /// Seen by a camera with a weapon drawn.
    pub weapon_seen_rate: f32,
    /// Seen at all in a restricted room (a security room).
    pub restricted_rate: f32,
    /// Seen hacking or picking a lock.
    pub tampering_rate: f32,
    /// A player's registered weapon found lying about by staff (framing).
    pub weapon_found: f32,
    /// A disguised player's cover slipping, per second (seen running, armed close by, in a restricted room): the
    /// heat a blown uniform starts from is built up by this.
    pub disguise_rate: f32,
}

impl Default for HeatTuning {
    fn default() -> Self {
        HeatTuning {
            suspicious: 30.0,
            wanted: 100.0,
            calm_below: 40.0,
            max: 160.0,
            hold_secs: 12.0,
            decay: 9.0,
            shot_heard: 34.0,
            weapon_seen_rate: 40.0,
            restricted_rate: 70.0,
            tampering_rate: 90.0,
            weapon_found: 35.0,
            disguise_rate: 25.0,
        }
    }
}

/// Why heat went up (for messages and tuning).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeatCause {
    ShotHeard,
    WeaponSeen,
    Restricted,
    Tampering,
    /// Hurt staff, a camera or a turret: wanted at once.
    AttackedStaff,
    /// Staff found a body and blame him (its killer, or the owner of a weapon left beside it): wanted at once.
    BodyFound,
    /// Staff found his registered weapon lying about.
    WeaponFound,
    /// The Overwatch seat sent the guards after him: wanted at once.
    Dispatched,
    /// Disguised in a uniform, he gave himself away for `secs`.
    Disguise,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Entry {
    id: u8,
    value: f32,
    /// Seconds since staff last saw or heard him.
    quiet: f32,
    wanted: bool,
}

/// Every player's heat in one match.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Heat {
    entries: Vec<Entry>,
    pub tuning: HeatTuning,
}

impl Heat {
    pub fn new(tuning: HeatTuning) -> Heat {
        Heat { entries: Vec::new(), tuning }
    }

    fn entry(&mut self, id: u8) -> &mut Entry {
        if let Some(i) = self.entries.iter().position(|e| e.id == id) {
            return &mut self.entries[i];
        }
        self.entries.push(Entry { id, value: 0.0, quiet: f32::MAX, wanted: false });
        self.entries.last_mut().unwrap()
    }

    fn get(&self, id: u8) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Heat for a reason. Returns true when this made him wanted.
    pub fn add(&mut self, id: u8, cause: HeatCause, amount: f32) -> bool {
        let t = self.tuning.clone();
        let e = self.entry(id);
        e.quiet = 0.0;
        let was = e.wanted;
        if cause == HeatCause::AttackedStaff {
            e.value = e.value.max(t.wanted);
        } else {
            e.value = (e.value + amount.max(0.0)).min(t.max);
        }
        if e.value >= t.wanted {
            e.wanted = true;
        }
        e.wanted && !was
    }

    /// Heat for a reason at its tuned amount (`secs` of being seen for the per-second causes).
    pub fn raise(&mut self, id: u8, cause: HeatCause, secs: f32) -> bool {
        let t = &self.tuning;
        let amount = match cause {
            HeatCause::ShotHeard => t.shot_heard,
            HeatCause::WeaponSeen => t.weapon_seen_rate * secs,
            HeatCause::Restricted => t.restricted_rate * secs,
            HeatCause::Tampering => t.tampering_rate * secs,
            HeatCause::AttackedStaff | HeatCause::BodyFound | HeatCause::Dispatched => t.wanted,
            HeatCause::WeaponFound => t.weapon_found,
            HeatCause::Disguise => t.disguise_rate * secs,
        };
        self.add(id, cause, amount)
    }

    /// Staff see him now: a wanted player stays wanted, nobody's heat drains while seen.
    pub fn seen(&mut self, id: u8) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.quiet = 0.0;
        }
    }

    /// Time passes: heat drains for those out of sight long enough.
    pub fn tick(&mut self, dt: f32) {
        let t = &self.tuning;
        for e in &mut self.entries {
            e.quiet = if e.quiet == f32::MAX { f32::MAX } else { e.quiet + dt };
            // Only the time past the hold drains.
            let draining = (e.quiet - t.hold_secs).clamp(0.0, dt);
            if draining > 0.0 {
                e.value = (e.value - t.decay * draining).max(0.0);
            }
            if e.wanted && e.value < t.calm_below {
                e.wanted = false;
            }
        }
        self.entries.retain(|e| e.value > 0.0 || e.wanted);
    }

    pub fn level(&self, id: u8) -> HeatLevel {
        match self.get(id) {
            Some(e) if e.wanted => HeatLevel::Wanted,
            Some(e) if e.value >= self.tuning.suspicious => HeatLevel::Suspicious,
            _ => HeatLevel::Calm,
        }
    }

    pub fn wanted(&self, id: u8) -> bool {
        self.get(id).is_some_and(|e| e.wanted)
    }

    pub fn value(&self, id: u8) -> f32 {
        self.get(id).map_or(0.0, |e| e.value)
    }

    /// He died or left: a new life starts clean.
    pub fn clear(&mut self, id: u8) {
        self.entries.retain(|e| e.id != id);
    }

    /// Everyone with any heat: (id, level, value).
    pub fn all(&self) -> Vec<(u8, HeatLevel, f32)> {
        self.entries.iter().map(|e| (e.id, self.level(e.id), e.value)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_shots_make_wanted() {
        let mut h = Heat::default();
        assert!(!h.raise(1, HeatCause::ShotHeard, 0.0));
        assert_eq!(h.level(1), HeatLevel::Suspicious);
        assert!(!h.raise(1, HeatCause::ShotHeard, 0.0));
        assert!(h.raise(1, HeatCause::ShotHeard, 0.0));
        assert!(h.wanted(1));
        assert!(!h.wanted(2));
    }

    #[test]
    fn attacking_staff_is_wanted_at_once() {
        let mut h = Heat::default();
        assert!(h.raise(3, HeatCause::AttackedStaff, 0.0));
        assert_eq!(h.level(3), HeatLevel::Wanted);
    }

    #[test]
    fn heat_holds_while_seen_then_drains_with_hysteresis() {
        let mut h = Heat::default();
        h.raise(1, HeatCause::AttackedStaff, 0.0);
        // Seen every second for a while: no drain.
        for _ in 0..30 {
            h.seen(1);
            h.tick(1.0);
        }
        assert!(h.wanted(1));
        assert_eq!(h.value(1), 100.0);
        // Out of sight: holds for hold_secs, then drains; still wanted until below calm_below.
        h.tick(11.0);
        assert_eq!(h.value(1), 100.0);
        for _ in 0..5 {
            h.tick(1.0);
        }
        assert!(h.value(1) < 100.0);
        assert!(h.wanted(1), "wanted until below calm_below");
        for _ in 0..20 {
            h.tick(1.0);
        }
        assert!(!h.wanted(1));
        for _ in 0..60 {
            h.tick(1.0);
        }
        assert_eq!(h.level(1), HeatLevel::Calm);
        assert!(h.all().is_empty());
    }

    #[test]
    fn seen_rates_scale_with_time() {
        let mut h = Heat::default();
        h.raise(1, HeatCause::Restricted, 0.5);
        assert!((h.value(1) - 35.0).abs() < 1e-4);
        h.raise(1, HeatCause::Restricted, 1.0);
        assert!(h.wanted(1));
        h.clear(1);
        assert_eq!(h.value(1), 0.0);
    }

    #[test]
    fn heat_is_capped() {
        let mut h = Heat::default();
        for _ in 0..20 {
            h.raise(1, HeatCause::ShotHeard, 0.0);
        }
        assert_eq!(h.value(1), h.tuning.max);
    }
}
