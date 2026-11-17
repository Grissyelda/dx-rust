//! Loadouts on a points budget (engine-neutral): what a player brings into a match - weapons, items, skill levels,
//! augmentations and weapon mods - priced from a data table. Unlocks widen the choice, never the budget: a build
//! that costs more than the budget isn't allowed (unless the host lifts the cap), and a locked entry can't be taken.
//!
//! Prices are game data; this module only adds them up and checks a build against them.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One player's loadout as chosen.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Build {
    /// The class template it started from (for its name and signature perk).
    pub class: String,
    pub weapons: Vec<String>,
    /// (item class, count).
    pub items: Vec<(String, u32)>,
    /// (skill class, levels bought above the match's starting level).
    pub skills: Vec<(String, u8)>,
    pub augs: Vec<String>,
    /// (weapon class, mod class) - a mod fitted to one of the build's weapons; the same pair may repeat (two
    /// accuracy mods).
    pub mods: Vec<(String, String)>,
}

/// The price table and limits.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Prices {
    pub budget: u32,
    pub weapons: HashMap<String, u32>,
    pub items: HashMap<String, u32>,
    /// Most of each item.
    pub item_max: HashMap<String, u32>,
    /// Cost of each level bought above the starting one, in turn (cumulative: [8, 10] = 8 for one level, 18 for
    /// two).
    pub skill_steps: Vec<u32>,
    pub augs: HashMap<String, u32>,
    pub mods: HashMap<String, u32>,
    pub max_weapons: usize,
    pub max_augs: usize,
    /// Mods on one weapon at most.
    pub max_mods_per_weapon: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// (cost, budget)
    OverBudget(u32, u32),
    /// Not in the price table.
    Unknown(String),
    /// Not unlocked yet.
    Locked(String),
    TooManyWeapons,
    TooManyAugs,
    TooManyOf(String),
    /// A mod for a weapon the build doesn't carry, or too many on one weapon.
    BadMod(String),
}

impl Prices {
    fn skill_cost(&self, levels: u8) -> Option<u32> {
        let n = levels as usize;
        if n > self.skill_steps.len() {
            return None;
        }
        Some(self.skill_steps[..n].iter().sum())
    }

    /// What a build costs (entries missing from the table count nothing; `check` reports them).
    pub fn cost(&self, b: &Build) -> u32 {
        let mut c = 0;
        for w in &b.weapons {
            c += self.weapons.get(w).copied().unwrap_or(0);
        }
        for (i, n) in &b.items {
            c += self.items.get(i).copied().unwrap_or(0) * n;
        }
        for (_, l) in &b.skills {
            c += self.skill_cost(*l).unwrap_or(0);
        }
        for a in &b.augs {
            c += self.augs.get(a).copied().unwrap_or(0);
        }
        for (_, m) in &b.mods {
            c += self.mods.get(m).copied().unwrap_or(0);
        }
        c
    }

    /// Everything wrong with a build: unknown or locked entries, limits, and the budget (`cap`: None = no budget,
    /// the host's Full setting). `unlocked(id)` says whether an entry may be taken ("WeaponGEPGun",
    /// "mod:WeaponRifle:WeaponModScope").
    pub fn check(&self, b: &Build, cap: Option<u32>, unlocked: impl Fn(&str) -> bool) -> Vec<Problem> {
        let mut out = Vec::new();
        if b.weapons.len() > self.max_weapons {
            out.push(Problem::TooManyWeapons);
        }
        if b.augs.len() > self.max_augs {
            out.push(Problem::TooManyAugs);
        }
        for w in &b.weapons {
            if !self.weapons.contains_key(w) {
                out.push(Problem::Unknown(w.clone()));
            } else if !unlocked(w) {
                out.push(Problem::Locked(w.clone()));
            }
        }
        for (i, n) in &b.items {
            if !self.items.contains_key(i) {
                out.push(Problem::Unknown(i.clone()));
            } else if !unlocked(i) {
                out.push(Problem::Locked(i.clone()));
            } else if *n > self.item_max.get(i).copied().unwrap_or(u32::MAX) {
                out.push(Problem::TooManyOf(i.clone()));
            }
        }
        for (s, l) in &b.skills {
            if self.skill_cost(*l).is_none() {
                out.push(Problem::TooManyOf(s.clone()));
            }
        }
        for a in &b.augs {
            if !self.augs.contains_key(a) {
                out.push(Problem::Unknown(a.clone()));
            } else if !unlocked(a) {
                out.push(Problem::Locked(a.clone()));
            }
        }
        let mut per: HashMap<&str, usize> = HashMap::new();
        for (w, m) in &b.mods {
            if !b.weapons.iter().any(|x| x == w) {
                out.push(Problem::BadMod(m.clone()));
                continue;
            }
            *per.entry(w.as_str()).or_default() += 1;
            if !self.mods.contains_key(m) {
                out.push(Problem::Unknown(m.clone()));
            } else if !unlocked(&format!("mod:{w}:{m}")) {
                out.push(Problem::Locked(format!("{m} on {w}")));
            }
        }
        if per.values().any(|&n| n > self.max_mods_per_weapon) {
            out.push(Problem::BadMod("too many mods on one weapon".into()));
        }
        if let Some(cap) = cap {
            let c = self.cost(b);
            if c > cap {
                out.push(Problem::OverBudget(c, cap));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prices() -> Prices {
        let mut p = Prices { budget: 100, skill_steps: vec![8, 10], max_weapons: 2, max_augs: 2, max_mods_per_weapon: 2, ..Default::default() };
        p.weapons.insert("WeaponPistol".into(), 0);
        p.weapons.insert("WeaponRifle".into(), 30);
        p.weapons.insert("WeaponGEPGun".into(), 40);
        p.items.insert("MedKit".into(), 4);
        p.item_max.insert("MedKit".into(), 3);
        p.augs.insert("AugCloak".into(), 20);
        p.mods.insert("WeaponModScope".into(), 5);
        p
    }

    #[test]
    fn cost_adds_everything() {
        let p = prices();
        let b = Build {
            weapons: vec!["WeaponPistol".into(), "WeaponRifle".into()],
            items: vec![("MedKit".into(), 2)],
            skills: vec![("SkillWeaponRifle".into(), 2)],
            augs: vec!["AugCloak".into()],
            mods: vec![("WeaponRifle".into(), "WeaponModScope".into())],
            ..Default::default()
        };
        assert_eq!(p.cost(&b), 30 + 8 + 18 + 20 + 5);
        assert!(p.check(&b, Some(100), |_| true).is_empty());
        assert_eq!(p.check(&b, Some(60), |_| true), vec![Problem::OverBudget(81, 60)]);
        assert!(p.check(&b, None, |_| true).is_empty(), "no cap with the host's Full setting");
    }

    #[test]
    fn locks_and_limits() {
        let p = prices();
        let b = Build {
            weapons: vec!["WeaponGEPGun".into(), "WeaponRifle".into(), "WeaponPistol".into()],
            items: vec![("MedKit".into(), 5)],
            skills: vec![("SkillWeaponRifle".into(), 3)],
            mods: vec![("WeaponPistol".into(), "WeaponModScope".into()), ("WeaponSword".into(), "WeaponModScope".into())],
            ..Default::default()
        };
        let probs = p.check(&b, None, |id| id != "WeaponGEPGun" && !id.starts_with("mod:WeaponPistol"));
        assert!(probs.contains(&Problem::TooManyWeapons));
        assert!(probs.contains(&Problem::Locked("WeaponGEPGun".into())));
        assert!(probs.contains(&Problem::TooManyOf("MedKit".into())));
        assert!(probs.contains(&Problem::TooManyOf("SkillWeaponRifle".into())));
        assert!(probs.contains(&Problem::Locked("WeaponModScope on WeaponPistol".into())));
        assert!(probs.contains(&Problem::BadMod("WeaponModScope".into())));
    }
}
