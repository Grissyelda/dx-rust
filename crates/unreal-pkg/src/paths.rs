//! Mod overlay folders (ours): folders laid out like the game folder (System, Maps, Textures, Sounds, Music) whose
//! files are used instead of the game folder's own, first match wins - a mod such as Lay D Denton, which ships as
//! "copy these over your install", runs without touching the player's install. Every package open goes through
//! `resolve`; directory searches use `find_ci`.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

static STATE: RwLock<Option<(PathBuf, Vec<PathBuf>)>> = RwLock::new(None);

/// The game folder and the overlay folders, highest priority first.
pub fn set(game_root: &Path, overlays: Vec<PathBuf>) {
    if let Ok(mut g) = STATE.write() {
        *g = Some((game_root.to_path_buf(), overlays.into_iter().filter(|o| o.is_dir()).collect()));
    }
}

pub fn overlays() -> Vec<PathBuf> {
    STATE.read().ok().and_then(|g| g.as_ref().map(|s| s.1.clone())).unwrap_or_default()
}

/// A file in `dir` named `name`, ignoring case (the game folders mix cases).
fn ci_in(dir: &Path, name: &str) -> Option<PathBuf> {
    let exact = dir.join(name);
    if exact.is_file() {
        return Some(exact);
    }
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|q| q.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name)))
}

/// Where the file at `path` (a path inside the game folder) really is: the first overlay that has it, else `path`.
pub fn resolve(path: &Path) -> PathBuf {
    let g = STATE.read();
    let Some((root, ovs)) = g.as_ref().ok().and_then(|g| g.as_ref()) else { return path.to_path_buf() };
    if ovs.is_empty() {
        return path.to_path_buf();
    }
    let Ok(rel) = path.strip_prefix(root) else { return path.to_path_buf() };
    let (Some(parent), Some(name)) = (rel.parent(), rel.file_name()) else { return path.to_path_buf() };
    let name = name.to_string_lossy();
    for o in ovs {
        if let Some(p) = ci_in(&o.join(parent), &name) {
            return p;
        }
    }
    path.to_path_buf()
}

/// A file named `name` (any case) in the game folder `dir`, looking in the overlays first.
pub fn find_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    let r = resolve(&dir.join(name));
    if r.is_file() {
        return Some(r);
    }
    ci_in(dir, name)
}

/// The file exists in an overlay or the game folder.
pub fn exists(path: &Path) -> bool {
    resolve(path).is_file() || path.is_file()
}
