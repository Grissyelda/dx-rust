//! The Nameless Mod (Off Topic Productions): a whole game made on Deus Ex. Its files stay in the player's own copy
//! (its ModDB download, unpacked by the launcher or the in-game mod manager); DX RUST reads them the way its own
//! TNMDefault.ini does - TNM2's System / Maps / Textures / Sounds / Music first, then its root folder, then the game.

use std::path::{Path, PathBuf};

/// Its code packages (TNMDefault.ini EditPackages / ServerPackages order, the ones its maps use).
pub const PACKAGES: &[&str] = &[
    "TNMNative", "MoreTriggers", "EFX", "HTK", "RF", "CaroneElevatorSet", "DXOgg", "TNMItems", "TNMDeco",
    "TNMCharacters", "TNMDataVault", "TNMGUI", "TNM", "TNMMP", "precipitation",
];

/// Its conversation packages: two halves of the game, text and speech kept apart.
pub const CON_TEXT: &[&str] = &["TNMConvos1Text", "TNMConvos2Text"];

/// The Nameless Mod's folder at `dir` or up to three folders below it: the one holding System/TNM.u and TNM2.
pub fn find(dir: &Path) -> Option<PathBuf> {
    fn walk(d: &Path, depth: usize) -> Option<PathBuf> {
        if crate::mods::find_rel(d, "System/TNM.u").is_some() {
            return Some(d.to_path_buf());
        }
        if depth == 0 {
            return None;
        }
        let mut subs: Vec<PathBuf> = std::fs::read_dir(d).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        subs.sort();
        subs.iter().find_map(|s| walk(s, depth - 1))
    }
    walk(dir, 3)
}

/// The folders laid over the game for it, first match wins (TNMDefault.ini's Paths order).
pub fn overlays(tnm: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(t2) = crate::mods::find_rel(tnm, "TNM2") {
        v.push(t2);
    }
    v.push(tnm.to_path_buf());
    v
}

/// Its Ogg music folder (TNMDefault.ini OggPath=..\TNM2\Music\Ogg\).
pub fn ogg_dir(tnm: &Path) -> Option<PathBuf> {
    crate::mods::find_rel(tnm, "TNM2/Music/Ogg").or_else(|| crate::mods::find_rel(tnm, "TNM2/Music"))
}

static ACTIVE: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);

/// This run plays The Nameless Mod from this folder.
pub fn set_active(dir: Option<PathBuf>) {
    if let Ok(mut a) = ACTIVE.write() {
        *a = dir;
    }
}

/// The Nameless Mod's folder when this run plays it.
pub fn active() -> Option<PathBuf> {
    ACTIVE.read().ok().and_then(|a| a.clone())
}
