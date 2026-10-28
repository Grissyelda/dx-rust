//! Class ancestry for IsA / ClassIsChildOf checks (mission scripts' `foreach AllActors(class'Terrorist', ...)` also
//! finds TerroristCommander, `class'ScriptedPawn'` every pawn). Filled from the game's code packages for every class
//! a level uses (`register`), then read anywhere without the ClassDb.

use std::collections::HashMap;
use std::sync::RwLock;

static TREE: RwLock<Option<HashMap<String, Vec<String>>>> = RwLock::new(None);

/// Look up and remember `class`'s ancestry (no-op when known).
pub fn register(classes: &mut unreal_pkg::classes::ClassDb, class: &str) {
    let key = class.to_ascii_lowercase();
    if TREE.read().ok().and_then(|t| t.as_ref().map(|m| m.contains_key(&key))).unwrap_or(false) {
        return;
    }
    let chain = classes.ancestry(class);
    if let Ok(mut t) = TREE.write() {
        t.get_or_insert_with(HashMap::new).insert(key, chain);
    }
}

/// Every class a level's actors use.
pub fn register_level(classes: &mut unreal_pkg::classes::ClassDb, level: &dx_level::Level) {
    let mut seen = std::collections::HashSet::new();
    for a in &level.actors {
        if seen.insert(a.class.to_ascii_lowercase()) {
            register(classes, &a.class);
        }
    }
}

/// IsA: `class` is `base` or derives from it. Unknown classes only match themselves.
pub fn is_a(class: &str, base: &str) -> bool {
    if class.eq_ignore_ascii_case(base) {
        return true;
    }
    let b = base.to_ascii_lowercase();
    TREE.read()
        .ok()
        .and_then(|t| t.as_ref().and_then(|m| m.get(&class.to_ascii_lowercase()).map(|c| c.iter().any(|x| *x == b))))
        .unwrap_or(false)
}
