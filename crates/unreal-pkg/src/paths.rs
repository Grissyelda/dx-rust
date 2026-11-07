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

/// Whether `exe_dir` is the executable folder of a macOS app bundle (`Name.app/Contents/MacOS`).
fn in_app_bundle(exe_dir: &Path) -> bool {
    exe_dir.ends_with("Contents/MacOS")
        && exe_dir.parent().and_then(Path::parent).and_then(Path::extension).is_some_and(|e| e == "app")
}

/// The folder for what the game writes (settings, saves, key bindings, logs): beside the executable, so a release
/// stays one portable folder. A macOS app bundle is signed and can be read-only, so there it's the player's
/// Application Support folder.
pub fn data_dir() -> PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf));
        let support = exe_dir
            .as_deref()
            .filter(|d| cfg!(target_os = "macos") && in_app_bundle(d))
            .and(std::env::var_os("HOME"))
            .map(|home| PathBuf::from(home).join("Library/Application Support/DX RUST"));
        if let Some(dir) = support {
            let _ = std::fs::create_dir_all(&dir);
            return dir;
        }
        exe_dir.unwrap_or_else(|| ".".into())
    })
    .clone()
}

/// The game folder and the overlay folders, highest priority first.
pub fn set(game_root: &Path, overlays: Vec<PathBuf>) {
    if let Ok(mut g) = STATE.write() {
        *g = Some((game_root.to_path_buf(), overlays.into_iter().filter(|o| o.is_dir()).collect()));
    }
}

/// Addon folders (after the game folder) and HD texture packs.
pub fn set_addons(addons: Vec<PathBuf>, hd_textures: Vec<PathBuf>) {
    if let Ok(mut l) = LOOSE.write() {
        *l = addons.iter().filter(|o| o.is_file()).cloned().collect();
    }
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
    if crate::mods::suspended() {
        return Vec::new();
    }
    ADDONS.read().map(|a| a.clone()).unwrap_or_default()
}

/// The HD texture packs' Textures folders, when HD textures are on.
pub fn hd_texture_dirs() -> Vec<PathBuf> {
    if !HD_TEX_ON.load(std::sync::atomic::Ordering::Relaxed) || crate::mods::suspended() {
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

/// The overlay folders (none while mods are suspended).
pub fn overlays() -> Vec<PathBuf> {
    if crate::mods::suspended() {
        return Vec::new();
    }
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
    let off = crate::mods::suspended();
    if ovs.is_empty() || off {
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
    if crate::mods::suspended() {
        return None;
    }
    let a = ADDONS.read().ok()?;
    if let Some(p) = a.iter().find_map(|d| ci_in(&d.join(parent), name)) {
        return Some(p);
    }
    drop(a);
    // An addon given as its one package file (a loose Precipitation.u in a mods folder) stands for System/<name>.
    let is_system = parent.as_os_str().to_string_lossy().eq_ignore_ascii_case("System");
    let l = LOOSE.read().ok()?;
    l.iter()
        .find(|f| is_system && f.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name)))
        .cloned()
}

/// Addon packages given as single files (mods::locate returned the file itself).
static LOOSE: RwLock<Vec<PathBuf>> = RwLock::new(Vec::new());

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

// ---- DDS texture overrides (kentie's Direct3D 10 renderer layout; Helios and other packs) ----
//
// A pack is a folder (usually the game's own Textures folder) holding `<Package>/<Group>/<Name>.dds` for textures it
// replaces; `<Name>.bump.dds` / `.height.dds` / `.detail.dds` are extra maps for the same texture and are not
// replacements. Which packages are taken is up to the game (`set_dds_filter`).

static DDS: RwLock<Option<DdsIndex>> = RwLock::new(None);

struct DdsIndex {
    /// "package/group/name" (lower case) -> file.
    files: std::collections::HashMap<String, PathBuf>,
    /// Package names (lower case) whose overrides are used.
    allow: Option<Vec<String>>,
    /// Package names never used.
    deny: Vec<String>,
}

/// Index the DDS packs under these folders (each one a Textures-style folder). Returns how many files were found.
pub fn set_dds(dirs: &[PathBuf]) -> usize {
    let mut files = std::collections::HashMap::new();
    fn walk(base: &Path, dir: &Path, depth: usize, out: &mut std::collections::HashMap<String, PathBuf>) {
        if depth > 4 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(base, &p, depth + 1, out);
                continue;
            }
            let Some(name) = p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()) else { continue };
            let Some(stem) = name.strip_suffix(".dds") else { continue };
            if stem.ends_with(".bump") || stem.ends_with(".height") || stem.ends_with(".detail") {
                continue;
            }
            let Ok(rel) = p.parent().unwrap_or(base).strip_prefix(base) else { continue };
            let mut key: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase()).collect();
            if key.is_empty() {
                continue;
            }
            key.push(stem.to_string());
            out.entry(key.join("/")).or_insert(p);
        }
    }
    for d in dirs {
        walk(d, d, 0, &mut files);
    }
    let n = files.len();
    if let Ok(mut g) = DDS.write() {
        *g = if n > 0 { Some(DdsIndex { files, allow: None, deny: Vec::new() }) } else { None };
    }
    n
}

/// Which packages' overrides are used: `allow` = only these (None = all), `deny` = never these.
pub fn set_dds_filter(allow: Option<Vec<String>>, deny: Vec<String>) {
    if let Ok(mut g) = DDS.write() {
        if let Some(i) = g.as_mut() {
            i.allow = allow.map(|v| v.iter().map(|s| s.to_ascii_lowercase()).collect());
            i.deny = deny.iter().map(|s| s.to_ascii_lowercase()).collect();
        }
    }
}

/// Any DDS overrides indexed.
pub fn has_dds() -> bool {
    if crate::mods::suspended() {
        return false;
    }
    DDS.read().map(|g| g.is_some()).unwrap_or(false)
}

/// The override for a texture path ("Package.Group.Name" or "Package.Name"), if a pack has one and its package
/// is allowed.
pub fn dds_for(path: &str) -> Option<PathBuf> {
    if crate::mods::suspended() {
        return None;
    }
    let g = DDS.read().ok()?;
    let idx = g.as_ref()?;
    let key = path.to_ascii_lowercase().replace('.', "/");
    let pkg = key.split('/').next()?;
    if idx.deny.iter().any(|d| d == pkg) {
        return None;
    }
    if let Some(a) = &idx.allow {
        if !a.iter().any(|x| x == pkg) {
            return None;
        }
    }
    idx.files.get(&key).cloned()
}

#[cfg(test)]
mod data_dir_tests {
    use super::in_app_bundle;
    use std::path::Path;

    #[test]
    fn only_an_app_bundle_counts() {
        assert!(in_app_bundle(Path::new("/Applications/DX RUST.app/Contents/MacOS")));
        assert!(!in_app_bundle(Path::new("/Users/jc/Games/dxrust")));
        assert!(!in_app_bundle(Path::new("/Users/jc/Contents/MacOS")));
    }
}
