//! Mod overlay folders (ours): folders laid out like the game folder (System, Maps, Textures, Sounds, Music) whose
//! files are used instead of the game folder's own, first match wins - a mod such as Lay D Denton, which ships as
//! "copy these over your install", runs without touching the player's install. Every package open goes through
//! `resolve`; directory searches use `find_ci`.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

static STATE: RwLock<Option<(PathBuf, Vec<PathBuf>)>> = RwLock::new(None);
/// Addon mod folders: searched after the game folder (they only add packages under new names).
static ADDONS: RwLock<Vec<PathBuf>> = RwLock::new(Vec::new());
/// High-resolution texture packs (mods::ModKind::HdTextures), used texture by texture while switched on.
static HD_TEX: RwLock<Vec<PathBuf>> = RwLock::new(Vec::new());
static HD_TEX_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// The game folder and the overlay folders, highest priority first.
pub fn set(game_root: &Path, overlays: Vec<PathBuf>) {
    if let Ok(mut g) = STATE.write() {
        *g = Some((game_root.to_path_buf(), overlays.into_iter().filter(|o| o.is_dir()).collect()));
    }
}

/// Addon folders (after the game folder) and HD texture packs.
pub fn set_addons(addons: Vec<PathBuf>, hd_textures: Vec<PathBuf>) {
    if let Ok(mut a) = ADDONS.write() {
        *a = addons.into_iter().filter(|o| o.is_dir()).collect();
    }
    if let Ok(mut h) = HD_TEX.write() {
        *h = hd_textures.into_iter().filter(|o| o.is_dir()).collect();
    }
}

/// The game folder, once set.
pub fn root() -> Option<PathBuf> {
    STATE.read().ok().and_then(|g| g.as_ref().map(|(r, _)| r.clone()))
}

pub fn addons() -> Vec<PathBuf> {
    ADDONS.read().map(|a| a.clone()).unwrap_or_default()
}

/// The HD texture packs' Textures folders, when HD textures are on.
pub fn hd_texture_dirs() -> Vec<PathBuf> {
    if !HD_TEX_ON.load(std::sync::atomic::Ordering::Relaxed) {
        return Vec::new();
    }
    HD_TEX.read().map(|h| h.iter().map(|d| crate::mods::find_rel(d, "Textures").unwrap_or_else(|| d.join("Textures"))).collect()).unwrap_or_default()
}

pub fn has_hd_textures() -> bool {
    HD_TEX.read().map(|h| !h.is_empty()).unwrap_or(false)
}

pub fn set_hd_textures_on(on: bool) {
    HD_TEX_ON.store(on, std::sync::atomic::Ordering::Relaxed);
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
        if path.exists() {
            return path.to_path_buf();
        }
        // Another case of the same name (Linux file systems are case-sensitive; scripts aren't).
        if let Some(p) = path.parent().zip(path.file_name()).and_then(|(d, n)| ci_in(d, &n.to_string_lossy())) {
            return p;
        }
        if let Ok(rel) = path.strip_prefix(root) {
            if let (Some(parent), Some(name)) = (rel.parent(), rel.file_name()) {
                if let Some(p) = in_addons(parent, &name.to_string_lossy()) {
                    return p;
                }
            }
        }
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
    if path.exists() {
        return path.to_path_buf();
    }
    if let Some(p) = ci_in(&root.join(parent), &name) {
        return p;
    }
    if let Some(p) = in_addons(parent, &name) {
        return p;
    }
    path.to_path_buf()
}

/// A file named `name` in an addon folder's `parent` (relative to the game folder), any case.
fn in_addons(parent: &Path, name: &str) -> Option<PathBuf> {
    let a = ADDONS.read().ok()?;
    a.iter().find_map(|d| ci_in(&d.join(parent), name))
}

/// A file named `name` (any case) in the game folder `dir`, looking in the overlays first.
pub fn find_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    let r = resolve(&dir.join(name));
    if r.is_file() {
        return Some(r);
    }
    ci_in(dir, name).or_else(|| {
        // An addon's package: the same folder name below an addon folder.
        let g = STATE.read().ok()?;
        let (root, _) = g.as_ref()?;
        let rel = dir.strip_prefix(root).ok()?;
        in_addons(rel, name)
    })
}

/// The file exists in an overlay or the game folder.
pub fn exists(path: &Path) -> bool {
    resolve(path).is_file() || path.is_file()
}
