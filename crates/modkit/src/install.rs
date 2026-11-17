//! Installing a downloaded mod: unpack it into the game's own Mods folder (beside settings.txt), find the mods it
//! carries, and switch them on in launcher.txt - the same lines the launcher's Mods page writes, so both see it.

use crate::archive::{self, Kind};
use crate::catalog::{self, Entry};
use std::path::{Path, PathBuf};

/// Where installed mods go: `<data dir>/Mods/<id>` (beside the game's settings; the Steam Deck's own folder too).
pub fn mods_dir() -> PathBuf {
    unreal_pkg::paths::data_dir().join("Mods")
}

fn launcher_txt() -> PathBuf {
    unreal_pkg::paths::data_dir().join("launcher.txt")
}

/// A download found in the player's folders.
#[derive(Clone, Debug)]
pub struct Found {
    pub path: PathBuf,
    pub entry: &'static Entry,
    pub size: u64,
}

/// Where downloads are looked for: the player's Downloads folder, a Downloads folder beside the game, and a MOD
/// ASSETS folder near it.
pub fn download_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    for var in ["USERPROFILE", "HOME"] {
        if let Some(h) = std::env::var_os(var) {
            v.push(PathBuf::from(h).join("Downloads"));
        }
    }
    let data = unreal_pkg::paths::data_dir();
    v.push(data.join("Downloads"));
    if let Some(a) = unreal_pkg::mods::find_assets_near(&data) {
        v.push(a);
    }
    v.dedup();
    v
}

const ARCHIVE_EXT: &[&str] = &["zip", "7z", "exe", "rar"];

/// Every recognised mod download in the download folders (and one folder down), newest first.
pub fn find_downloads() -> Vec<Found> {
    let mut out: Vec<(Found, std::time::SystemTime)> = Vec::new();
    for dir in download_dirs() {
        let mut files = Vec::new();
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if let Ok(rd2) = std::fs::read_dir(&p) {
                    files.extend(rd2.flatten().map(|e| e.path()).filter(|p| p.is_file()));
                }
            } else {
                files.push(p);
            }
        }
        for p in files {
            let ext = p.extension().map(|x| x.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            if !ARCHIVE_EXT.contains(&ext.as_str()) {
                continue;
            }
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let Some(entry) = catalog::guess(&name) else { continue };
            let Ok(md) = std::fs::metadata(&p) else { continue };
            if out.iter().any(|(f, _)| f.path == p) {
                continue;
            }
            out.push((Found { path: p, entry, size: md.len() }, md.modified().unwrap_or(std::time::UNIX_EPOCH)));
        }
    }
    out.sort_by(|a, b| b.1.cmp(&a.1));
    out.into_iter().map(|(f, _)| f).collect()
}

/// The Nameless Mod's folder below `dir` (up to three levels): the one with System/TNM.u.
pub fn find_tnm(dir: &Path) -> Option<PathBuf> {
    fn walk(d: &Path, depth: usize) -> Option<PathBuf> {
        if unreal_pkg::mods::find_rel(d, "System/TNM.u").is_some() {
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

/// The supported mods inside an unpacked folder (up to three levels down), with where each one's folder is.
pub fn find_mods(dir: &Path) -> Vec<(&'static str, PathBuf)> {
    let mut out: Vec<(&'static str, PathBuf)> = Vec::new();
    if let Some(t) = find_tnm(dir) {
        out.push(("tnm", t));
    }
    fn walk(d: &Path, depth: usize, out: &mut Vec<(&'static str, PathBuf)>) {
        for m in unreal_pkg::mods::MODS {
            if out.iter().any(|(id, _)| *id == m.id) {
                continue;
            }
            // New Vision's marker is a game texture package and Helios's a bare Textures folder: only taken from a
            // folder named for them (as the launcher's scan does), never from inside another mod or a game copy.
            if matches!(m.kind, unreal_pkg::mods::ModKind::HdTextures | unreal_pkg::mods::ModKind::DdsTextures) {
                let n = d.to_string_lossy().to_ascii_lowercase();
                let named = match m.id {
                    "newvision" => n.contains("vision"),
                    "helios" => n.contains("helios") || n.contains("hto"),
                    _ => false,
                };
                if !named {
                    continue;
                }
            }
            if let Some(p) = unreal_pkg::mods::locate(m, d) {
                out.push((m.id, p));
            }
        }
        if depth == 0 {
            return;
        }
        let mut subs: Vec<PathBuf> =
            std::fs::read_dir(d).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default();
        subs.sort();
        for s in subs {
            walk(&s, depth - 1, out);
        }
    }
    walk(dir, 3, &mut out);
    out
}

/// launcher.txt: set these keys (in place where they are, else added at the end), leaving every other line alone.
pub fn set_launcher_lines(set: &[(String, String)], remove: &[String]) -> std::io::Result<()> {
    let path = launcher_txt();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let key_of = |l: &str| l.split_once('=').map(|(k, _)| k.trim().to_ascii_lowercase());
    lines.retain(|l| key_of(l).is_none_or(|k| !remove.iter().any(|r| r.eq_ignore_ascii_case(&k))));
    for (k, v) in set {
        match lines.iter_mut().find(|l| key_of(l).is_some_and(|lk| lk.eq_ignore_ascii_case(k))) {
            Some(l) => *l = format!("{k}={v}"),
            None => lines.push(format!("{k}={v}")),
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    std::fs::write(&path, out)
}

/// What an install made.
#[derive(Clone, Debug)]
pub struct Installed {
    pub folder: PathBuf,
    pub files: usize,
    /// The mods switched on (id, folder).
    pub mods: Vec<(&'static str, PathBuf)>,
}

/// The folder name for a download: the catalog id, else the file's name cleaned up.
fn folder_name(download: &Path, entry: Option<&Entry>) -> String {
    if let Some(e) = entry {
        return e.id.to_string();
    }
    let stem = download.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "mod".into());
    let clean: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    clean.trim_matches('_').chars().take(48).collect::<String>().to_ascii_lowercase()
}

/// Unpack a download into Mods/<id> (replacing an older copy only once the new one is complete) and switch on the
/// mods it carries. `progress(0..1, text)` is called as it goes.
pub fn install(download: &Path, progress: &mut dyn FnMut(f32, &str)) -> Result<Installed, String> {
    let kind = archive::sniff(download).map_err(|e| format!("can't read {}: {e}", download.display()))?;
    if !kind.can_unpack() {
        return Err(format!("This file is {}.", kind.describe()));
    }
    let name = download.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let entry = catalog::guess(&name);
    let root = mods_dir();
    std::fs::create_dir_all(&root).map_err(|e| format!("can't make {}: {e}", root.display()))?;
    let folder = folder_name(download, entry);
    let dest = root.join(&folder);
    let part = root.join(format!(".{folder}.partial"));
    let _ = std::fs::remove_dir_all(&part);
    progress(0.0, match kind {
        Kind::SevenZ { offset } if offset > 0 => "Opening the installer's archive",
        _ => "Unpacking",
    });
    let files = archive::extract(download, kind, &part, progress).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&part);
    })?;
    let found_in_part = find_mods(&part);
    if found_in_part.is_empty() {
        let _ = std::fs::remove_dir_all(&part);
        return Err(format!(
            "Unpacked {files} files, but none of them is a mod DX RUST supports. Is this the right download for {}?",
            entry.map_or("the mod", |e| e.name)
        ));
    }
    if dest.exists() {
        std::fs::remove_dir_all(&dest).map_err(|e| format!("can't replace the older copy in {}: {e}", dest.display()))?;
    }
    std::fs::rename(&part, &dest).map_err(|e| format!("can't move the unpacked files into {}: {e}", dest.display()))?;
    let mods: Vec<(&'static str, PathBuf)> =
        found_in_part.into_iter().map(|(id, p)| (id, dest.join(p.strip_prefix(&part).unwrap_or(Path::new(""))))).collect();
    let mut set = Vec::new();
    for (id, p) in &mods {
        if *id == "tnm" {
            set.push(("TNMPath".to_string(), p.display().to_string()));
        } else {
            set.push((format!("Mod.{id}"), p.display().to_string()));
            set.push((format!("ModOn.{id}"), "1".to_string()));
        }
    }
    set_launcher_lines(&set, &[]).map_err(|e| format!("installed, but launcher.txt couldn't be written: {e}"))?;
    progress(1.0, "Done");
    Ok(Installed { folder: dest, files, mods })
}

/// Switch an installed mod on or off.
pub fn set_on(id: &str, on: bool) -> std::io::Result<()> {
    set_launcher_lines(&[(format!("ModOn.{id}"), if on { "1" } else { "0" }.into())], &[])
}

/// Remove a mod: its launcher.txt lines go, and its files too when they are in our own Mods folder (never a folder
/// the player pointed the launcher at elsewhere).
pub fn remove(id: &str, folder: &Path) -> Result<(), String> {
    let keys: Vec<String> = if id == "tnm" { vec!["TNMPath".into()] } else { vec![format!("Mod.{id}"), format!("ModOn.{id}")] };
    set_launcher_lines(&[], &keys).map_err(|e| e.to_string())?;
    let root = mods_dir();
    if let (Ok(r), Ok(f)) = (root.canonicalize(), folder.canonicalize()) {
        // The whole Mods/<name> folder the install made (the mod's own folder may be inside it).
        if let Some(top) = f.strip_prefix(&r).ok().and_then(|rel| rel.components().next()) {
            let top = r.join(top);
            if top != r {
                std::fs::remove_dir_all(&top).map_err(|e| format!("can't delete {}: {e}", top.display()))?;
            }
        }
    }
    Ok(())
}

/// Where a catalog entry stands on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    NotInstalled,
    /// Installed (its folder), switched off.
    Off(PathBuf),
    /// Installed and on.
    On(PathBuf),
}

/// What launcher.txt says about a catalog entry: installed when any mod it provides has a folder that's found (on
/// when any of those is on). The Nameless Mod: TNMPath=.
pub fn state(e: &Entry) -> State {
    let lines = unreal_pkg::mods::read_launcher_txt(&launcher_txt());
    if e.id == "tnm" {
        return match lines.iter().find(|(k, _)| k.eq_ignore_ascii_case("TNMPath")).map(|(_, v)| PathBuf::from(v.trim())) {
            Some(p) if find_tnm(&p).is_some() => State::On(p),
            _ => State::NotInstalled,
        };
    }
    let mut set = unreal_pkg::mods::from_lines(&lines);
    unreal_pkg::mods::add_assets(&lines, &mut set);
    let mut off = None;
    for id in e.provides {
        let Some(s) = set.iter().find(|s| s.id == *id) else { continue };
        let Some(def) = unreal_pkg::mods::def(id) else { continue };
        let Some(p) = unreal_pkg::mods::locate(def, Path::new(&s.path)) else { continue };
        if s.on {
            return State::On(p);
        }
        off.get_or_insert(p);
    }
    off.map_or(State::NotInstalled, State::Off)
}

/// Switch every mod a catalog entry provides on or off.
pub fn set_entry_on(e: &Entry, on: bool) -> std::io::Result<()> {
    let set: Vec<(String, String)> =
        e.provides.iter().filter(|id| **id != "tnm").map(|id| (format!("ModOn.{id}"), if on { "1" } else { "0" }.into())).collect();
    set_launcher_lines(&set, &[])
}

/// Remove every mod a catalog entry provides (files only from our own Mods folder).
pub fn remove_entry(e: &Entry) -> Result<(), String> {
    let lines = unreal_pkg::mods::read_launcher_txt(&launcher_txt());
    let set = unreal_pkg::mods::from_lines(&lines);
    for id in e.provides {
        let folder = if *id == "tnm" {
            lines.iter().find(|(k, _)| k.eq_ignore_ascii_case("TNMPath")).map(|(_, v)| PathBuf::from(v.trim()))
        } else {
            set.iter().find(|s| s.id == *id).map(|s| PathBuf::from(&s.path))
        };
        remove(id, folder.as_deref().unwrap_or(Path::new("")))?;
    }
    Ok(())
}
