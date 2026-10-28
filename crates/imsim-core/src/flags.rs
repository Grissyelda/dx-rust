//! The game's flag store (Deus Ex FlagBase): named values that record what happened (who died, who was spared,
//! what was said), read by conversations, triggers and mission scripts.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum FlagValue {
    Bool(bool),
    Int(i32),
    Name(String),
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct FlagBase {
    /// Lower-cased name -> (value, expiration mission; 0 = never).
    flags: HashMap<String, (FlagValue, i32)>,
    /// SetDefaultExpiration: what set_bool gives a flag (MissionScript sets missionNumber + 1 on each map).
    #[serde(default)]
    default_exp: i32,
}

impl FlagBase {
    pub fn set_bool(&mut self, name: &str, v: bool) {
        self.set(name, FlagValue::Bool(v), self.default_exp);
    }

    pub fn set_default_expiration(&mut self, mission: i32) {
        self.default_exp = mission;
    }

    pub fn delete(&mut self, name: &str) {
        self.flags.remove(&name.to_ascii_lowercase());
    }

    pub fn set(&mut self, name: &str, v: FlagValue, expires: i32) {
        self.flags.insert(name.to_ascii_lowercase(), (v, expires));
    }

    pub fn get_bool(&self, name: &str) -> bool {
        matches!(self.flags.get(&name.to_ascii_lowercase()), Some((FlagValue::Bool(true), _)))
    }

    pub fn get(&self, name: &str) -> Option<&FlagValue> {
        self.flags.get(&name.to_ascii_lowercase()).map(|(v, _)| v)
    }

    /// DeleteExpiredFlags: drop flags whose expiration mission has passed.
    pub fn expire(&mut self, mission: i32) {
        self.flags.retain(|_, (_, e)| *e == 0 || *e > mission);
    }

    /// DeleteAllFlags (MissionIntro / MissionEndgame: a new game starts clean).
    pub fn delete_all(&mut self) {
        self.flags.clear();
    }

    pub fn len(&self) -> usize {
        self.flags.len()
    }

    pub fn is_empty(&self) -> bool {
        self.flags.is_empty()
    }
}
