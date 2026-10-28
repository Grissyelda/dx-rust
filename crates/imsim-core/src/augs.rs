//! Augmentations (Deus Ex Augmentation / AugmentationManager): body locations with a fixed number of slots, hotkeys
//! numbered per location, four tech levels, and bioelectric energy drained while active.
//!
//! This module keeps the state and the manager's rules; what an active augmentation does to the world is up to
//! the game, which asks `level_value` (GetAugLevelValue: `None` while the aug is missing or off).

/// AugmentationLocation.
pub const LOC_CRANIAL: u8 = 0;
pub const LOC_EYE: u8 = 1;
pub const LOC_TORSO: u8 = 2;
pub const LOC_ARM: u8 = 3;
pub const LOC_LEG: u8 = 4;
pub const LOC_SUBDERMAL: u8 = 5;
pub const LOC_DEFAULT: u8 = 6;

pub const LOC_NAMES: [&str; 7] = ["Cranial", "Eyes", "Torso", "Arms", "Legs", "Subdermal", "Default"];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AugDef {
    pub class: String,
    pub name: String,
    pub description: String,
    pub icon: String,
    pub small_icon: String,
    /// Energy units per minute at full use.
    pub energy_rate: f32,
    pub max_level: usize,
    pub values: [f32; 4],
    pub location: u8,
    /// Always on (IFF, infolink): no hotkey toggling, no energy.
    pub always_active: bool,
    /// Energy rate scales with the level value (Cloak, Radar Transparency: GetEnergyRate override).
    pub rate_scales: bool,
    /// Multiplies everyone's energy use while active (Power Recirculator).
    pub power: bool,
    pub activate_sound: String,
    pub deactivate_sound: String,
    pub loop_sound: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Aug {
    pub def: AugDef,
    pub has: bool,
    pub active: bool,
    pub level: usize,
    /// Raised a level by the Synthetic Heart.
    pub boosted: bool,
    /// HotKeyNum: F-key index = hotkey - 3 (F3 = 0 ... F12 = 9).
    pub hotkey: i32,
}

impl Aug {
    pub fn energy_rate(&self) -> f32 {
        if self.def.rate_scales {
            self.def.energy_rate * self.def.values[self.level.min(3)]
        } else {
            self.def.energy_rate
        }
    }
    pub fn value(&self) -> f32 {
        self.def.values[self.level.min(3)]
    }
    /// F-key slot 0..9 (F3..F12), if any.
    pub fn key(&self) -> Option<usize> {
        let k = self.hotkey - 3;
        (0..=9).contains(&k).then_some(k as usize)
    }
}

/// S_AugInfo: slots per location and the hotkey base.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LocInfo {
    pub slots: usize,
    pub count: usize,
    pub key_base: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Give {
    /// Newly installed at level 1.
    Added,
    /// Already installed: upgraded (IncLevel) if an upgrade was possible.
    Upgraded(bool),
    /// The location's slots are taken.
    SlotsFull,
    Unknown,
}

/// What a toggle did, so the game can play sounds and show the message.
#[derive(Debug, Clone, PartialEq)]
pub enum Toggle {
    Activated(usize),
    Deactivated(usize),
    /// No installed aug on that key.
    NoAug,
    /// Always-active augs can't be turned off; or energy is 0.
    Nothing,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Augs {
    pub list: Vec<Aug>,
    pub locs: [LocInfo; 7],
}

impl Augs {
    pub fn new(defs: Vec<AugDef>, locs: [LocInfo; 7]) -> Augs {
        Augs {
            list: defs
                .into_iter()
                .map(|d| Aug { def: d, has: false, active: false, level: 0, boosted: false, hotkey: 0 })
                .collect(),
            locs,
        }
    }

    pub fn index(&self, class: &str) -> Option<usize> {
        self.list.iter().position(|a| a.def.class.eq_ignore_ascii_case(class))
    }

    pub fn get(&self, class: &str) -> Option<&Aug> {
        self.index(class).map(|i| &self.list[i])
    }

    /// GetAugLevelValue: the active aug's value, None when it's missing or off.
    pub fn level_value(&self, class: &str) -> Option<f32> {
        self.get(class).filter(|a| a.has && a.active).map(|a| a.value())
    }

    /// GetClassLevel: the active aug's level (0..3), None when missing or off.
    pub fn class_level(&self, class: &str) -> Option<usize> {
        self.get(class).filter(|a| a.has && a.active).map(|a| a.level)
    }

    /// NumAugsActive (always-active ones don't count).
    pub fn num_active(&self) -> usize {
        self.list.iter().filter(|a| a.has && a.active && !a.def.always_active).count()
    }

    /// AreSlotsFull.
    pub fn slots_full(&self, i: usize) -> bool {
        let loc = self.list[i].def.location as usize;
        let n = self
            .list
            .iter()
            .enumerate()
            .filter(|(j, a)| *j != i && !a.def.name.is_empty() && a.def.location as usize == loc && a.has)
            .count();
        n >= self.locs.get(loc).map_or(0, |l| l.slots)
    }

    /// GivePlayerAugmentation. `can_upgrade` says whether an upgrade canister is at hand (IncLevel's check).
    pub fn give(&mut self, class: &str, can_upgrade: bool) -> Give {
        let Some(i) = self.index(class) else { return Give::Unknown };
        if self.list[i].has {
            return Give::Upgraded(self.upgrade(i, can_upgrade));
        }
        if self.slots_full(i) {
            return Give::SlotsFull;
        }
        let loc = self.list[i].def.location as usize;
        let a = &mut self.list[i];
        a.has = true;
        a.active = a.def.always_active;
        if let Some(l) = self.locs.get_mut(loc) {
            l.count += 1;
            a.hotkey = l.count as i32 + l.key_base;
        }
        Give::Added
    }

    /// Augmentation.IncLevel (deactivates first). Returns false at the maximum or without a canister.
    pub fn upgrade(&mut self, i: usize, can_upgrade: bool) -> bool {
        let a = &mut self.list[i];
        if a.level >= a.def.max_level || !can_upgrade {
            return false;
        }
        a.active = a.active && a.def.always_active;
        a.level += 1;
        true
    }

    /// ActivateAugByKey: toggles the installed aug on F-key slot `key` (0 = F3).
    pub fn toggle_key(&mut self, key: usize, energy: f32) -> Toggle {
        let Some(i) = self.list.iter().position(|a| a.has && a.key() == Some(key)) else { return Toggle::NoAug };
        if self.list[i].active {
            self.deactivate(i)
        } else {
            self.activate(i, energy)
        }
    }

    pub fn activate(&mut self, i: usize, energy: f32) -> Toggle {
        let a = &mut self.list[i];
        if !a.has || a.active || energy <= 0.0 {
            return Toggle::Nothing;
        }
        a.active = true;
        Toggle::Activated(i)
    }

    pub fn deactivate(&mut self, i: usize) -> Toggle {
        let a = &mut self.list[i];
        if !a.has || !a.active || a.def.always_active {
            return Toggle::Nothing;
        }
        a.active = false;
        Toggle::Deactivated(i)
    }

    /// DeactivateAll: the indices turned off.
    pub fn deactivate_all(&mut self) -> Vec<usize> {
        (0..self.list.len()).filter(|&i| matches!(self.deactivate(i), Toggle::Deactivated(_))).collect()
    }

    /// CalcEnergyUse: units used over `dt` seconds by everything active, times the Power Recirculator's value.
    pub fn energy_use(&self, dt: f32) -> f32 {
        let mut used = 0.0;
        let mut mult = 1.0;
        for a in self.list.iter().filter(|a| a.has && a.active) {
            used += a.energy_rate() / 60.0 * dt;
            if a.def.power {
                mult = a.value();
            }
        }
        used * mult
    }

    /// BoostAugs (Synthetic Heart): every other active aug below its maximum gains a level while the heart is on;
    /// turning it off takes the boost back.
    pub fn boost(&mut self, on: bool, booster: usize) {
        for (i, a) in self.list.iter_mut().enumerate() {
            if i == booster {
                continue;
            }
            if on {
                if a.active && !a.boosted && a.level < a.def.max_level {
                    a.level += 1;
                    a.boosted = true;
                }
            } else if a.boosted {
                a.level -= 1;
                a.boosted = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(class: &str, loc: u8, rate: f32) -> AugDef {
        AugDef {
            class: class.into(),
            name: class.into(),
            energy_rate: rate,
            max_level: 3,
            values: [1.2, 1.4, 1.6, 1.8],
            location: loc,
            ..Default::default()
        }
    }

    fn locs() -> [LocInfo; 7] {
        let s = |slots, key_base| LocInfo { slots, count: 0, key_base };
        [s(1, 4), s(1, 7), s(3, 8), s(1, 5), s(1, 6), s(2, 2), s(3, 11)]
    }

    #[test]
    fn hotkeys_follow_the_location_key_base() {
        let mut light = def("AugLight", LOC_DEFAULT, 10.0);
        light.max_level = 0;
        let mut a = Augs::new(vec![light, def("AugSpeed", LOC_LEG, 40.0), def("AugStealth", LOC_LEG, 40.0)], locs());
        assert_eq!(a.give("AugLight", false), Give::Added);
        assert_eq!(a.get("AugLight").unwrap().key(), Some(9), "F12");
        assert_eq!(a.give("AugSpeed", false), Give::Added);
        assert_eq!(a.get("AugSpeed").unwrap().key(), Some(4), "legs: F7");
        assert_eq!(a.give("AugStealth", false), Give::SlotsFull, "one leg slot");
        assert_eq!(a.toggle_key(4, 100.0), Toggle::Activated(1));
        assert_eq!(a.level_value("AugSpeed"), Some(1.2));
        assert_eq!(a.toggle_key(4, 100.0), Toggle::Deactivated(1));
        assert_eq!(a.level_value("AugSpeed"), None);
        assert_eq!(a.toggle_key(4, 0.0), Toggle::Nothing, "no energy");
    }

    #[test]
    fn energy_use_and_power_recirculator() {
        let mut power = def("AugPower", LOC_TORSO, 10.0);
        power.power = true;
        power.values = [0.9, 0.8, 0.6, 0.4];
        let mut a = Augs::new(vec![def("AugSpeed", LOC_LEG, 40.0), power], locs());
        a.give("AugSpeed", false);
        a.give("AugPower", false);
        a.activate(0, 100.0);
        assert!((a.energy_use(60.0) - 40.0).abs() < 1e-4);
        a.activate(1, 100.0);
        assert!((a.energy_use(60.0) - 50.0 * 0.9).abs() < 1e-4);
        assert_eq!(a.deactivate_all(), vec![0, 1]);
        assert_eq!(a.energy_use(60.0), 0.0);
    }

    #[test]
    fn heart_boosts_and_unboosts() {
        let mut a = Augs::new(vec![def("AugSpeed", LOC_LEG, 40.0), def("AugHeartLung", LOC_TORSO, 100.0)], locs());
        a.give("AugSpeed", false);
        a.give("AugHeartLung", false);
        a.activate(0, 100.0);
        a.boost(true, 1);
        assert_eq!(a.list[0].level, 1);
        a.boost(true, 1);
        assert_eq!(a.list[0].level, 1, "boosted once");
        a.boost(false, 1);
        assert_eq!(a.list[0].level, 0);
    }
}
