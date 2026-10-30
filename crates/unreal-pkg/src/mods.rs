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
    let ok = |d: &Path| -> bool {
        find_rel(d, m.marker).is_some()
            && !(m.kind == ModKind::HdTextures && find_rel(d, "System/DeusEx.u").is_some())
    };
    if ok(dir) {
        return Some(dir.to_path_buf());
    }
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.is_dir() && ok(p))
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
