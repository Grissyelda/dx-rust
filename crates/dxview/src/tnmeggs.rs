//! The Nameless Mod's easter eggs (EasterEgg decorations): frobbing one records its number for good - across every
//! game, like the mod's own config - says whether it's a new find, hands out 50 skill points and goes.

use std::collections::BTreeSet;
use std::sync::Mutex;
use unreal_pkg::props::{find, Property, Value};

static FOUND: Mutex<Option<BTreeSet<i32>>> = Mutex::new(None);

fn file() -> std::path::PathBuf {
    unreal_pkg::paths::data_dir().join("TNMSave").join("eggs.txt")
}

fn with<R>(f: impl FnOnce(&mut BTreeSet<i32>) -> R) -> R {
    let mut g = FOUND.lock().unwrap_or_else(|e| e.into_inner());
    let set = g.get_or_insert_with(|| {
        std::fs::read_to_string(file())
            .map(|t| t.split_whitespace().filter_map(|n| n.parse().ok()).collect())
            .unwrap_or_default()
    });
    f(set)
}

/// An easter egg decoration (TNM only).
pub fn is_egg(class: &str) -> bool {
    unreal_pkg::tnm::active().is_some() && crate::classtree::is_a(class, "EasterEgg")
}

/// Trestkon.hasegg.
pub fn has(id: i32) -> bool {
    with(|s| s.contains(&id))
}

/// How many eggs have been found (Trestkon.counteggs).
pub fn count() -> usize {
    with(|s| s.len())
}

/// EasterEgg.Frob: the message to show. The caller removes the egg.
pub fn frob(props: &[Property]) -> String {
    let id = match find(props, "ID") {
        Some(Value::Int(n)) => *n,
        _ => 0,
    };
    let s = |k: &str, d: &str| match find(props, k) {
        Some(Value::Str(v)) => v.clone(),
        _ => d.to_string(),
    };
    let again = has(id);
    let msg = if again {
        s("message2", "Easter egg #%i (already found).")
    } else {
        with(|set| {
            set.insert(id);
            let text: Vec<String> = set.iter().map(|n| n.to_string()).collect();
            let _ = std::fs::create_dir_all(file().parent().unwrap());
            let _ = std::fs::write(file(), text.join("\n"));
        });
        s("Message", "Easter egg #%i found.")
    };
    // Trestkon.SkillPointsAdd(50).
    if let Some(m) = crate::abilities::award_points(50) {
        crate::classic_hud::client_message(&m);
    }
    println!("TNM easter egg {id} ({} found)", count());
    msg.replacen("%i", &id.to_string(), 1).replacen("%d", &id.to_string(), 1)
}
