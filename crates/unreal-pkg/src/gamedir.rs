//! Finding the player's Deus Ex folder when none has been chosen: the usual Steam (every Steam library, Windows,
//! Linux / Steam Deck and macOS, the SD card too), GOG and Heroic install places.

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
        if cfg!(target_os = "macos") {
            steams.push(home.join("Library/Application Support/Steam"));
            // Steam for Mac has no Mac version of the game: `download_depot 6910 6911` in its console leaves the
            // Windows files here.
            v.push(home.join(
                "Library/Application Support/Steam/Steam.AppBundle/Steam/Contents/MacOS/steamapps/content/app_6910/depot_6911",
            ));
            v.push(home.join("Documents/Deus Ex"));
            v.push("/Applications/Deus Ex".into());
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

/// Deus Ex folders under `roots`, at most `depth` folders down (a folder holding System/DeusEx.u, any letter case),
/// nearest first. Used on Android, where the game is copied anywhere on the phone or its SD card. Hidden folders and
/// Android's own app folders are skipped.
pub fn scan(roots: &[PathBuf], depth: usize) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<(usize, PathBuf)>, level: usize) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut subs = Vec::new();
        for e in rd.flatten() {
            let Ok(t) = e.file_type() else { continue };
            if !t.is_dir() {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if name.eq_ignore_ascii_case("system") {
                if crate::paths::find_ci(&e.path(), "DeusEx.u").is_some() {
                    out.push((level, dir.to_path_buf()));
                    return;
                }
                continue;
            }
            if name.starts_with('.') || name.eq_ignore_ascii_case("Android") {
                continue;
            }
            subs.push(e.path());
        }
        if depth > 0 {
            for d in subs {
                walk(&d, depth - 1, out, level + 1);
            }
        }
    }
    let mut out = Vec::new();
    for r in roots {
        walk(r, depth, &mut out, 0);
    }
    out.sort_by_key(|(l, _)| *l);
    let mut seen = Vec::new();
    out.into_iter()
        .map(|(_, p)| p)
        .filter(|p| {
            let k = p.canonicalize().unwrap_or_else(|_| p.clone());
            let new = !seen.contains(&k);
            seen.push(k);
            new
        })
        .collect()
}

/// Android's storage roots: the phone's shared storage and every SD card or USB drive (/storage/XXXX-XXXX).
pub fn android_roots() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from("/storage/emulated/0")];
    if let Ok(rd) = std::fs::read_dir("/storage") {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n != "emulated" && n != "self" && e.path().is_dir() {
                v.push(e.path());
            }
        }
    }
    v
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn macos_candidates_cover_the_steam_library_and_the_depot_download() {
        let c = candidates();
        assert!(c.iter().any(|p| p.ends_with("Library/Application Support/Steam/steamapps/common/Deus Ex")));
        assert!(c.iter().any(|p| p.ends_with("steamapps/content/app_6910/depot_6911")));
    }
}

#[cfg(test)]
mod scan_tests {
    use super::*;

    #[test]
    fn finds_a_copied_game_folder_in_any_letter_case() {
        let root = std::env::temp_dir().join("dxrust_scan_test");
        let _ = std::fs::remove_dir_all(&root);
        let game = root.join("Download").join("Deus Ex GOTY");
        std::fs::create_dir_all(game.join("system")).unwrap();
        std::fs::write(game.join("system").join("deusex.u"), b"x").unwrap();
        std::fs::create_dir_all(root.join(".hidden").join("System")).unwrap();
        std::fs::write(root.join(".hidden").join("System").join("DeusEx.u"), b"x").unwrap();
        assert_eq!(scan(&[root.clone()], 4), vec![game]);
        assert!(scan(&[root.clone()], 0).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
