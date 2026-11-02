//! The mods DX RUST supports (ours). Each lives in its own folder on the player's PC, laid out like the game folder;
//! nothing is copied into the install. The launcher's Mods page keeps one folder per mod in launcher.txt
//! (`Mod.<id>=<folder>`, `ModOn.<id>=0|1`; Lay D Denton's older `LDDPPath=` still counts) and the game reads the same
//! lines. How a mod's files are used depends on its kind:
//!
//! * `Overlay`: its files replace the game's (maps, conversations, packages) - Lay D Denton.
//! * `Addon`: it only adds packages under new names (meshes, textures, code we port) - its own copies of game
//!   packages (HDTP's DeusEx.u, GMDX's DeusEx.u) are never used in place of the game's.
//! * `HdTextures`: texture packages with the game's names whose textures carry high-resolution compressed mips -
//!   used texture by texture, falling back to the game's, and only while HD textures are switched on.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModKind {
    Overlay,
    Addon,
    HdTextures,
    /// DDS override files laid out `<Package>/<Group>/<Name>.dds` (kentie's Direct3D 10 renderer packs: Helios):
    /// the folder is the one holding the package folders (usually the game's own Textures folder).
    DdsTextures,
    /// Normal / height maps laid out `<Package>/<Group>/<Name>.bump.dds` and `.height.dds` (the DX11 renderer's;
    /// the Parallax Occlusion Mapping addon): read by dx-level's relief module for remastered lighting, never as
    /// colour textures.
    ReliefMaps,
}


#[derive(Clone, Copy, Debug)]
pub struct ModDef {
    pub id: &'static str,
    /// The launcher's button text.
    pub name: &'static str,
    pub kind: ModKind,
    /// A file (relative, any case) that only this mod's folder has.
    pub marker: &'static str,
    /// The launcher's help line.
    pub help: &'static str,
}

pub const MODS: &[ModDef] = &[
    ModDef {
        id: "lddp",
        name: "Lay D Denton",
        kind: ModKind::Overlay,
        marker: "System/FemJC.u",
        help: "Play as the female JC (the Lay D Denton Project): choose its folder",
    },
    ModDef {
        id: "visibleattachments",
        name: "Visible Attachments",
        kind: ModKind::Addon,
        marker: "System/VisibleAttachments.u",
        help: "Weapon mods (scope, laser, silencer) shown on the weapons: choose its folder",
    },
    ModDef {
        id: "hdtp",
        name: "HDTP",
        kind: ModKind::Addon,
        marker: "System/HDTPCharacters.u",
        help: "High-definition models (the HD Texture Project): choose its folder; switch them in Settings",
    },
    ModDef {
        id: "newvision",
        name: "New Vision",
        kind: ModKind::HdTextures,
        marker: "Textures/CoreTexMetal.utx",
        help: "High-resolution world textures (New Vision): choose its folder; switch them in Settings",
    },
    ModDef {
        id: "helios",
        name: "Helios (DDS)",
        kind: ModKind::DdsTextures,
        marker: "DeusExCharacters/Skins",
        help: "Helios Texture Overhaul and other DDS texture packs for the Direct3D 10 renderer: choose the Textures folder holding them",
    },
    ModDef {
        id: "augmentique",
        name: "Augmentique",
        kind: ModKind::Addon,
        marker: "System/Augmentique.u",
        help: "Outfits for JC and the people of the world (Augmentique): choose its folder",
    },
    ModDef {
        id: "precipitation",
        name: "Precipitation",
        kind: ModKind::Addon,
        marker: "System/Precipitation.u",
        help: "Rain (Smoke39's Precipitation, also in GMDX Augmented's System folder): choose its folder",
    },
    ModDef {
        id: "relief",
        name: "Parallax Maps",
        kind: ModKind::ReliefMaps,
        marker: "",
        help: "Normal and height maps (the Parallax Occlusion Mapping addon): choose its folder; used by Remastered Lighting",
    },
    ModDef {
        id: "gmdx",
        name: "GMDX Perks",
        kind: ModKind::Addon,
        marker: "System/GMDXUI.u",
        help: "Perks bought with skill points, from GMDX Augmented Edition: choose its folder",
    },
];

pub fn def(id: &str) -> Option<&'static ModDef> {
    MODS.iter().find(|m| m.id == id)
}

/// A file at a relative path below `dir`, any case per component.
pub fn find_rel(dir: &Path, rel: &str) -> Option<PathBuf> {
    let mut cur = dir.to_path_buf();
    for part in rel.split('/') {
        let exact = cur.join(part);
        if exact.exists() {
            cur = exact;
            continue;
        }
        cur = std::fs::read_dir(&cur)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(part)))?;
    }
    Some(cur)
}

/// The mod's folder: `dir` itself or a folder just inside it (an unpacked zip often adds one) holding the marker.
/// New Vision's marker is also in the game's own folder, so a folder with System/DeusEx.u isn't New Vision.
pub fn locate(m: &ModDef, dir: &Path) -> Option<PathBuf> {
    if m.kind == ModKind::ReliefMaps {
        // Any folder with .bump.dds files a few levels down (the addon's variants are picked when indexing).
        fn has_bump(d: &Path, depth: usize) -> bool {
            let Ok(rd) = std::fs::read_dir(d) else { return false };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    if depth < 5 && has_bump(&p, depth + 1) {
                        return true;
                    }
                } else if p.file_name().is_some_and(|n| n.to_string_lossy().to_ascii_lowercase().ends_with(".bump.dds")) {
                    return true;
                }
            }
            false
        }
        return has_bump(dir, 0).then(|| dir.to_path_buf());
    }
    if m.kind == ModKind::DdsTextures {
        // The folder holding the package folders: this one, its Textures folder, or one just inside.
        let has = |d: &Path| find_rel(d, m.marker).is_some_and(|p| dir_has_dds(&p));
        if has(dir) {
            return Some(dir.to_path_buf());
        }
        if let Some(t) = find_rel(dir, "Textures").filter(|t| has(t)) {
            return Some(t);
        }
        return std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.is_dir() && has(p));
    }
    // An addon that is one package file: the file itself, chosen directly or lying loose in the folder.
    if let Some(file) = loose_marker(m) {
        if dir.is_file() {
            return dir.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(file)).then(|| dir.to_path_buf());
        }
    }
    let ok = |d: &Path| -> bool {
        find_rel(d, m.marker).is_some()
            && !(m.kind == ModKind::HdTextures && find_rel(d, "System/DeusEx.u").is_some())
    };
    if ok(dir) {
        return Some(dir.to_path_buf());
    }
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.is_dir() && ok(p))
}

/// Folder names that hint at each mod (lower case), for `scan`: a folder named like the mod is tried first.
fn hints(id: &str) -> &'static [&'static str] {
    match id {
        "lddp" => &["lay_d", "lay d", "lddp", "femjc"],
        "visibleattachments" => &["visible"],
        "hdtp" => &["hdtp"],
        "newvision" => &["new vision", "newvision", "new_vision"],
        "helios" => &["textures", "helios"],
        "augmentique" => &["augmentique"],
        "precipitation" => &["precipitation", "gmdx"],
        "relief" => &["parallax", "relief", "pom"],
        "gmdx" => &["gmdx"],
        _ => &[],
    }
}

/// One folder holding many mods (each in its own folder inside, as unpacked): where each supported mod is.
/// Folders named like the mod are tried first, then every other folder; a folder already taken by one mod is
/// still offered to the others (GMDX Augmented's System folder also carries Precipitation).
pub fn scan(dir: &Path) -> Vec<(&'static ModDef, PathBuf)> {
    let mut subs: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect())
        .unwrap_or_default();
    subs.sort();
    let mut out = Vec::new();
    for m in MODS {
        let name_of = |p: &Path| p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        let (mut named, rest): (Vec<&PathBuf>, Vec<&PathBuf>) =
            subs.iter().partition(|p| hints(m.id).iter().any(|h| name_of(p).contains(h)));
        named.sort_by_key(|p| hints(m.id).iter().position(|h| name_of(p).contains(h)).unwrap_or(99));
        // The Helios pack is a bare Textures folder: never search the other mods' folders for it (New Vision has
        // a Textures folder of packages, not of DDS files, but HDTP or others might ship DDS files).
        // New Vision's marker is a game texture package, so only a folder named like it counts.
        let rest = if matches!(m.kind, ModKind::DdsTextures | ModKind::ReliefMaps | ModKind::HdTextures) { Vec::new() } else { rest };
        // A loose package in the folder itself (Precipitation.u dropped beside the mod folders) is taken before
        // other mods' copies (GMDX: Augmented Edition ships one in its System folder).
        let first_hint = hints(m.id).first().copied().unwrap_or("");
        let own_folder = named.iter().copied().find(|p| name_of(p).contains(first_hint)).and_then(|p| locate(m, p));
        let loose = loose_marker(m).and_then(|f| find_rel(dir, f)).filter(|p| p.is_file());
        if let Some(p) = own_folder.or(loose).or_else(|| named.into_iter().chain(rest).find_map(|p| locate(m, p))) {
            out.push((m, p));
        }
    }
    out
}

/// The single package an addon may be installed as on its own ("System/Precipitation.u" -> "Precipitation.u"):
/// only for mods that are one package (Precipitation).
fn loose_marker(m: &ModDef) -> Option<&'static str> {
    (m.id == "precipitation").then(|| m.marker.rsplit('/').next().unwrap_or(m.marker))
}

fn dir_has_dds(d: &Path) -> bool {
    std::fs::read_dir(d).ok().is_some_and(|rd| {
        rd.flatten().any(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("dds")))
    })
}

/// One mod as launcher.txt has it.
#[derive(Clone, Debug, PartialEq)]
pub struct ModSetting {
    pub id: &'static str,
    pub path: String,
    pub on: bool,
}

/// The mod settings in launcher.txt's lines (`Mod.<id>=`, `ModOn.<id>=`; `LDDPPath=` for Lay D Denton).
pub fn from_lines(lines: &[(String, String)]) -> Vec<ModSetting> {
    let mut out = Vec::new();
    for m in MODS {
        let mut path = lines.iter().find(|(k, _)| k.eq_ignore_ascii_case(&format!("Mod.{}", m.id))).map(|(_, v)| v.clone());
        if path.is_none() && m.id == "lddp" {
            path = lines.iter().find(|(k, _)| k == "LDDPPath" || k == "ModPath").map(|(_, v)| v.clone());
        }
        let Some(path) = path.filter(|p| !p.trim().is_empty()) else { continue };
        let on = lines
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(&format!("ModOn.{}", m.id)))
            .map(|(_, v)| v.trim() != "0")
            .unwrap_or(true);
        out.push(ModSetting { id: m.id, path, on });
    }
    out
}

/// launcher.txt's `ModAssets=` folder (one folder holding every mod, each in its own folder): the mods found in
/// it that have no line of their own are added, on.
pub fn add_assets(lines: &[(String, String)], mods: &mut Vec<ModSetting>) {
    let Some(dir) = lines.iter().find(|(k, _)| k.eq_ignore_ascii_case("ModAssets")).map(|(_, v)| v.clone()) else { return };
    if dir.trim().is_empty() {
        return;
    }
    for (m, p) in scan(Path::new(dir.trim())) {
        if !mods.iter().any(|s| s.id == m.id) {
            mods.push(ModSetting { id: m.id, path: p.display().to_string(), on: true });
        }
    }
}

/// A "MOD ASSETS" folder (any case, spaces or underscores) beside `dir` or up to three folders above it.
pub fn find_assets_near(dir: &Path) -> Option<PathBuf> {
    let mut d = Some(dir);
    for _ in 0..4 {
        let cur = d?;
        if let Ok(rd) = std::fs::read_dir(cur) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_ascii_lowercase().replace(['_', '-'], " ");
                if e.path().is_dir() && (n == "mod assets" || n == "modassets") {
                    return Some(e.path());
                }
            }
        }
        d = cur.parent();
    }
    None
}

/// launcher.txt lines for these settings.
pub fn to_lines(mods: &[ModSetting]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for s in mods {
        out.push((format!("Mod.{}", s.id), s.path.clone()));
        out.push((format!("ModOn.{}", s.id), if s.on { "1" } else { "0" }.into()));
    }
    out
}

/// The enabled mods whose folders are found, with their kinds, in MODS order.
pub fn enabled(mods: &[ModSetting]) -> Vec<(&'static ModDef, PathBuf)> {
    MODS.iter()
        .filter_map(|m| {
            let s = mods.iter().find(|s| s.id == m.id && s.on)?;
            Some((m, locate(m, Path::new(&s.path))?))
        })
        .collect()
}

/// Read launcher.txt (key=value lines).
pub fn read_launcher_txt(path: &Path) -> Vec<(String, String)> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
        .collect()
}

static ACTIVE: std::sync::RwLock<Vec<(&'static str, PathBuf)>> = std::sync::RwLock::new(Vec::new());

/// The game's mod list for this run (set once at start-up, after `paths::set`): overlays, addons and HD texture
/// packs go to `paths`; `active` answers "is this mod on, and where".
pub fn activate(list: &[(&'static ModDef, PathBuf)]) {
    if let Ok(mut a) = ACTIVE.write() {
        *a = list.iter().map(|(m, p)| (m.id, p.clone())).collect();
    }
    let addons = list.iter().filter(|(m, _)| m.kind == ModKind::Addon).map(|(_, p)| p.clone()).collect();
    let hd = list.iter().filter(|(m, _)| m.kind == ModKind::HdTextures).map(|(_, p)| p.clone()).collect();
    crate::paths::set_addons(addons, hd);
    let dds: Vec<PathBuf> = list.iter().filter(|(m, _)| m.kind == ModKind::DdsTextures).map(|(_, p)| p.clone()).collect();
    if !dds.is_empty() {
        let n = crate::paths::set_dds(&dds);
        println!("mods: {n} DDS texture overrides");
    }
}

/// The folder of an enabled mod.
pub fn active(id: &str) -> Option<PathBuf> {
    ACTIVE.read().ok()?.iter().find(|(i, _)| *i == id).map(|(_, p)| p.clone())
}

#[cfg(test)]
mod tests {
    /// MOD_ASSETS=<folder> cargo test -p unreal-pkg scan_assets -- --nocapture
    #[test]
    fn scan_assets() {
        let Ok(dir) = std::env::var("MOD_ASSETS") else { return };
        for (m, p) in super::scan(std::path::Path::new(&dir)) {
            println!("{} = {}", m.id, p.display());
        }
    }
}
