//! Per-character alliance tables (Deus Ex ScriptedPawn.AlliancesEx, GetAllianceType, ChangeAlly,
//! AgitateAlliance, UpdateAgitation), engine-neutral.
//!
//! Each character keeps its own feelings toward other alliances: a level in -1..1 (negative = hostile,
//! positive = friendly) plus an agitation 0..1 built up by provocations (being hurt by members of that
//! alliance). Permanent entries can't be agitated or changed by triggers that honour permanence. Members of
//! the character's own alliance are always friendly and permanent, so friendly fire never turns allies on
//! each other.

/// How one character regards an alliance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attitude {
    Hostile,
    Neutral,
    Friendly,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AllyEntry {
    pub name: String,
    pub level: f32,
    pub agitation: f32,
    pub permanent: bool,
}

/// Deus Ex keeps 16 entries; a 17th pushes the oldest out.
const MAX_ENTRIES: usize = 16;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AllianceTable {
    /// The character's own alliance (ScriptedPawn.Alliance).
    pub own: String,
    pub entries: Vec<AllyEntry>,
    /// AgitationTimer: provocations hold for AgitationSustainTime before they start to decay.
    sustain_timer: f32,
    /// AgitationCheckTimer: at most one provocation counts every 1.5 s.
    check_timer: f32,
    pub sustain_time: f32,
    pub decay_rate: f32,
    pub max_provocations: i32,
}

impl AllianceTable {
    /// InitializeAlliances: the placed InitialAlliances (name, level, permanent), plus the own alliance.
    pub fn new(own: &str, initial: impl IntoIterator<Item = (String, f32, bool)>) -> AllianceTable {
        let mut t = AllianceTable {
            own: own.to_string(),
            entries: Vec::new(),
            sustain_timer: 0.0,
            check_timer: 0.0,
            sustain_time: 30.0,
            decay_rate: 0.05,
            max_provocations: 1,
        };
        for (n, l, p) in initial {
            if !n.is_empty() && !n.eq_ignore_ascii_case("None") {
                t.change_ally(&n, l, p, false);
            }
        }
        if !own.is_empty() {
            t.change_ally(own, 1.0, true, false);
        }
        t
    }

    fn find(&self, name: &str) -> Option<&AllyEntry> {
        self.entries.iter().find(|e| e.name.eq_ignore_ascii_case(name))
    }

    /// ChangeAlly (AllianceTrigger, scripted events). The own alliance stays friendly and permanent.
    pub fn change_ally(&mut self, name: &str, level: f32, permanent: bool, honor_permanence: bool) {
        let (level, permanent) = if name.eq_ignore_ascii_case(&self.own) { (1.0, true) } else { (level.clamp(-1.0, 1.0), permanent) };
        if honor_permanence && self.find(name).is_some_and(|e| e.permanent) {
            return;
        }
        let entry = AllyEntry { name: name.to_string(), level, agitation: 0.0, permanent };
        match self.entries.iter_mut().find(|e| e.name.eq_ignore_ascii_case(name)) {
            Some(e) => *e = entry,
            None => {
                if self.entries.len() >= MAX_ENTRIES {
                    self.entries.pop();
                }
                self.entries.insert(0, entry);
            }
        }
    }

    /// GetAllianceType.
    pub fn attitude(&self, name: &str) -> Attitude {
        if name.is_empty() {
            return Attitude::Neutral;
        }
        match self.find(name) {
            Some(e) if e.level < 0.0 || e.agitation >= 1.0 => Attitude::Hostile,
            Some(e) if e.level > 0.0 => Attitude::Friendly,
            _ => Attitude::Neutral,
        }
    }

    pub fn hostile_to(&self, name: &str) -> bool {
        self.attitude(name) == Attitude::Hostile
    }

    pub fn friendly_to(&self, name: &str) -> bool {
        self.attitude(name) == Attitude::Friendly
    }

    /// The raw level (0 when unknown).
    pub fn level(&self, name: &str) -> f32 {
        self.find(name).map(|e| e.level).unwrap_or(0.0)
    }

    /// Anyone this character could fight (CheckEnemyPresence's bCheck: a negative level or full agitation).
    pub fn has_enemies(&self) -> bool {
        self.entries.iter().any(|e| e.level < 0.0 || e.agitation >= 1.0)
    }

    /// IncreaseAgitation + AgitateAlliance: a provocation by a member of `name` (hurting this character).
    /// `amount` None uses 1 / (MaxProvocations + 1). Returns true if the alliance is now hostile.
    pub fn provoke(&mut self, name: &str, amount: Option<f32>) -> bool {
        if name.is_empty() || name.eq_ignore_ascii_case(&self.own) {
            return false;
        }
        self.sustain_timer = self.sustain_time;
        if self.check_timer > 0.0 {
            return self.hostile_to(name);
        }
        self.check_timer = 1.5;
        let amount = amount.unwrap_or(1.0 / (self.max_provocations.max(0) + 1) as f32);
        self.agitate(name, amount)
    }

    /// AgitateAlliance: raise the agitation toward `name` (not the own alliance, not permanent entries).
    /// Returns true if the alliance is now hostile.
    pub fn agitate(&mut self, name: &str, amount: f32) -> bool {
        if name.is_empty() || name.eq_ignore_ascii_case(&self.own) || amount <= 0.0 {
            return self.hostile_to(name);
        }
        match self.entries.iter_mut().find(|e| e.name.eq_ignore_ascii_case(name)) {
            Some(e) if e.permanent => {}
            Some(e) => e.agitation = (e.agitation + amount).min(1.0),
            None => {
                if self.entries.len() >= MAX_ENTRIES {
                    self.entries.pop();
                }
                self.entries.insert(0, AllyEntry { name: name.to_string(), level: 0.0, agitation: amount.min(1.0), permanent: false });
            }
        }
        self.hostile_to(name)
    }

    /// UpdateAgitation: after AgitationSustainTime without provocation, agitation drains at AgitationDecayRate.
    pub fn tick(&mut self, dt: f32) {
        self.check_timer = (self.check_timer - dt).max(0.0);
        let decrement = if self.sustain_timer > 0.0 {
            if self.sustain_timer < dt {
                let mult = 1.0 - self.sustain_timer / dt;
                self.sustain_timer = 0.0;
                mult * self.decay_rate * dt
            } else {
                self.sustain_timer -= dt;
                0.0
            }
        } else {
            self.decay_rate * dt
        };
        if decrement > 0.0 {
            for e in self.entries.iter_mut().filter(|e| !e.permanent) {
                e.agitation = (e.agitation - decrement).max(0.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nsf() -> AllianceTable {
        AllianceTable::new("NSF", [("UNATCO".to_string(), -1.0, true), ("Player".to_string(), -1.0, true), ("paul".to_string(), 1.0, true)])
    }

    #[test]
    fn own_alliance_never_turns_hostile() {
        let mut t = nsf();
        for _ in 0..10 {
            t.provoke("NSF", None);
            t.tick(2.0);
        }
        assert_eq!(t.attitude("NSF"), Attitude::Friendly);
    }

    #[test]
    fn two_provocations_make_a_neutral_hostile_then_it_fades() {
        let mut t = AllianceTable::new("UNATCO", [("Player".to_string(), 1.0, false)]);
        assert!(!t.provoke("Player", None));
        // A second hit inside 1.5 s doesn't count.
        t.tick(1.0);
        assert!(!t.provoke("Player", None));
        t.tick(1.0);
        assert!(t.provoke("Player", None));
        assert_eq!(t.attitude("Player"), Attitude::Hostile);
        // 30 s sustain, then 0.05 / s decay.
        t.tick(31.0);
        t.tick(1.0);
        assert_eq!(t.attitude("Player"), Attitude::Friendly);
    }

    #[test]
    fn permanent_entries_ignore_provocation() {
        let mut t = nsf();
        assert!(!t.provoke("paul", None));
        t.tick(2.0);
        assert!(!t.provoke("paul", None));
        assert_eq!(t.attitude("paul"), Attitude::Friendly);
    }
}
