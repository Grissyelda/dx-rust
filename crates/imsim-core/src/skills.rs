//! Skills (Deus Ex Skill / SkillManager): four levels each, upgrades bought with skill points.
//!
//! A skill's effect is its `LevelValues[CurrentLevel]`; what the value means is up to the code that asks
//! (accuracy bonus for weapon skills, pick strength for lockpicking, heal multiplier for medicine...).

/// One skill's data (class defaults).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SkillDef {
    pub class: String,
    pub name: String,
    pub description: String,
    /// Icon texture path.
    pub icon: String,
    /// Points to go from level i to i + 1.
    pub cost: [i32; 3],
    pub values: [f32; 4],
    /// The tool the skill needs (lockpick, multitool), if any.
    pub item_needed: Option<String>,
    /// Upgraded automatically (swimming, environmental) rather than used.
    pub automatic: bool,
    /// Level a new character starts at (the pistol starts Trained in single player).
    pub start_level: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Skill {
    pub def: SkillDef,
    /// 0 untrained .. 3 master.
    pub level: usize,
}

pub const LEVEL_NAMES: [&str; 4] = ["UNTRAINED", "TRAINED", "ADVANCED", "MASTER"];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Skills {
    pub list: Vec<Skill>,
    pub points_avail: i32,
    pub points_total: i32,
}

impl Skills {
    pub fn new(defs: Vec<SkillDef>, points: i32) -> Skills {
        Skills {
            list: defs.into_iter().map(|d| Skill { level: d.start_level.min(3), def: d }).collect(),
            points_avail: points,
            points_total: points,
        }
    }

    pub fn find(&self, class: &str) -> Option<&Skill> {
        self.list.iter().find(|s| s.def.class.eq_ignore_ascii_case(class))
    }

    fn find_mut(&mut self, class: &str) -> Option<&mut Skill> {
        self.list.iter_mut().find(|s| s.def.class.eq_ignore_ascii_case(class))
    }

    /// GetSkillLevelValue: 0 for a skill the player doesn't have.
    pub fn value(&self, class: &str) -> f32 {
        self.find(class).map_or(0.0, |s| s.def.values[s.level])
    }

    /// GetSkillLevel.
    pub fn level(&self, class: &str) -> usize {
        self.find(class).map_or(0, |s| s.level)
    }

    /// GetCost: points for the next level (None at master).
    pub fn cost(&self, class: &str) -> Option<i32> {
        self.find(class).and_then(|s| s.def.cost.get(s.level).copied())
    }

    /// CanAffordToUpgrade.
    pub fn can_upgrade(&self, class: &str) -> bool {
        self.cost(class).is_some_and(|c| c <= self.points_avail)
    }

    /// IncLevel with the player: pays the cost.
    pub fn upgrade(&mut self, class: &str) -> bool {
        let avail = self.points_avail;
        let Some(s) = self.find_mut(class) else { return false };
        if s.level >= 3 || s.def.cost[s.level] > avail {
            return false;
        }
        let c = s.def.cost[s.level];
        s.level += 1;
        self.points_avail -= c;
        true
    }

    /// DecLevel(bGiveUserPoints): refunds the cost of the level dropped (character creation).
    pub fn downgrade(&mut self, class: &str, refund: bool) -> bool {
        let Some(s) = self.find_mut(class) else { return false };
        if s.level == 0 {
            return false;
        }
        s.level -= 1;
        let c = s.def.cost[s.level];
        if refund {
            self.points_avail += c;
        }
        true
    }

    /// Skill points awarded (SkillPointsAdd): both counters go up.
    pub fn award(&mut self, points: i32) {
        self.points_avail += points;
        self.points_total += points;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pistol() -> SkillDef {
        SkillDef {
            class: "SkillWeaponPistol".into(),
            cost: [1575, 3150, 5250],
            values: [0.0, -0.1, -0.25, -0.5],
            start_level: 1,
            ..Default::default()
        }
    }

    #[test]
    fn starts_trained_and_pays_for_upgrades() {
        let mut s = Skills::new(vec![pistol()], 5000);
        assert_eq!(s.level("SkillWeaponPistol"), 1);
        assert_eq!(s.value("SkillWeaponPistol"), -0.1);
        assert!(s.upgrade("SkillWeaponPistol"));
        assert_eq!(s.points_avail, 5000 - 3150);
        assert!(!s.upgrade("SkillWeaponPistol"), "5250 is more than what is left");
        assert!(s.downgrade("SkillWeaponPistol", true));
        assert_eq!(s.points_avail, 5000);
        assert_eq!(s.value("SkillMissing"), 0.0);
    }
}
