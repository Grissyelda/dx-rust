//! Class aliases: a mod's thin subclasses of original classes (a patrol point that only looks one way, a keypad
//! with another window class) answer to the original's name, so everything written for the original applies.
//! The table is the game's to set (dxview sets it while a mod that needs it is played); empty = no aliases.

use std::sync::RwLock;

static TABLE: RwLock<Vec<(String, String)>> = RwLock::new(Vec::new());

/// Replace the table: (class, the original class it stands for).
pub fn set(list: Vec<(String, String)>) {
    if let Ok(mut t) = TABLE.write() {
        *t = list;
    }
}

/// The class a class name stands for (itself when it has no alias).
pub fn of(class: &str) -> String {
    if let Ok(t) = TABLE.read() {
        if let Some((_, b)) = t.iter().find(|(a, _)| a.eq_ignore_ascii_case(class)) {
            return b.clone();
        }
    }
    class.to_string()
}

/// `class` is `base` or stands for it.
pub fn is(class: &str, base: &str) -> bool {
    class == base
        || TABLE.read().is_ok_and(|t| t.iter().any(|(a, b)| b == base && a.eq_ignore_ascii_case(class)))
}
