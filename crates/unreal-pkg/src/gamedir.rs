//! Finding the player's Deus Ex folder when none has been chosen: the usual Steam (every Steam library, Windows and
//! Linux / Steam Deck, the SD card too), GOG and Heroic install places.

use std::path::{Path, PathBuf};

/// A Deus Ex folder: it has System/DeusEx.u (any case).
pub fn is_game_dir(p: &Path) -> bool {
    crate::paths::find_ci(&p.join("System"), "DeusEx.u").is_some()
        || p.join("System").join("DeusEx.u").exists()
}

/// Steam library folders from libraryfolders.vdf ("path" lines), plus the Steam folder itself.
fn steam_libraries(steam: &Path) -> Vec<PathBuf> {
    let mut v = vec![steam.to_path_buf()];
    for vdf in [steam.join("steamapps").join("libraryfolders.vdf"), steam.join("config").join("libraryfolders.vdf")] {
        let Ok(t) = std::fs::read_to_string(&vdf) else { continue };
        for line in t.lines() {
            let l = line.trim();
            if let Some(rest) = l.strip_prefix("\"path\"") {
                let p = rest.trim().trim_matches('"').replace("\\\\", "\\");
                if !p.is_empty() {
                    v.push(PathBuf::from(p));
                }
            }
        }
    }
    v
}

/// Every place worth looking, most likely first.
pub fn candidates() -> Vec<PathBuf> {
    let mut steams: Vec<PathBuf> = Vec::new();
    let mut v: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for d in [r"C:\Program Files (x86)\Steam", r"C:\Program Files\Steam"] {
            steams.push(d.into());
        }
        for d in [
            r"C:\Games\Deus Ex",
            r"C:\GOG Games\Deus Ex GOTY",
            r"C:\GOG Games\Deus Ex",
            r"C:\Program Files (x86)\GOG Galaxy\Games\Deus Ex GOTY",
            r"C:\Program Files (x86)\GOG Galaxy\Games\Deus Ex",
        ] {
            v.push(d.into());
        }
        for drive in ['D', 'E', 'F'] {
            v.push(format!(r"{drive}:\SteamLibrary\steamapps\common\Deus Ex").into());
            v.push(format!(r"{drive}:\Games\Deus Ex").into());
        }
    } else if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for d in [".local/share/Steam", ".steam/steam", ".steam/root", ".var/app/com.valvesoftware.Steam/.local/share/Steam"] {
            steams.push(home.join(d));
        }
        for d in ["Games/Heroic/Deus Ex GOTY", "Games/Heroic/Deus Ex", "GOG Games/Deus Ex GOTY", "Games/Deus Ex"] {
            v.push(home.join(d));
        }
        // Steam Deck SD cards and other removable drives.
        if let Ok(rd) = std::fs::read_dir("/run/media") {
            for e in rd.flatten() {
                v.push(e.path().join("steamapps/common/Deus Ex"));
                v.push(e.path().join("SteamLibrary/steamapps/common/Deus Ex"));
                if let Ok(rd2) = std::fs::read_dir(e.path()) {
                    for e2 in rd2.flatten() {
                        v.push(e2.path().join("steamapps/common/Deus Ex"));
                        v.push(e2.path().join("SteamLibrary/steamapps/common/Deus Ex"));
                    }
                }
            }
        }
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for s in &steams {
        for lib in steam_libraries(s) {
            out.push(lib.join("steamapps").join("common").join("Deus Ex"));
        }
    }
    out.extend(v);
    out
}

/// The first Deus Ex folder found, if any.
pub fn detect() -> Option<PathBuf> {
    candidates().into_iter().find(|p| is_game_dir(p))
}
