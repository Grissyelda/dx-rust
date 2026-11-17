//! RecompLauncher (recomplauncher.com) support: the bits that apply only when the launcher starts the game.
//!
//! The launcher reads `recomplauncher.json` beside the executable, asks the player for their Deus Ex folder, and
//! starts `dxview --game <folder>`. It also sets RECOMPLAUNCHER=1, RECOMPLAUNCHER_MODE (desktop / bigpicture) and
//! RECOMPLAUNCHER_PAD (xbox / playstation). Without those, nothing here changes how the game starts.

use std::sync::OnceLock;

/// Pull `--game <path>` (or `--game=<path>`) out of the arguments: the game folder the launcher (or a script, or a
/// Steam shortcut) passes. The rest of the arguments keep their places, so `dxview --game X map.dx` still starts
/// map.dx.
pub fn take_game_arg(args: Vec<String>) -> (Vec<String>, Option<String>) {
    let mut out = Vec::with_capacity(args.len());
    let mut game = None;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        if a == "--game" {
            game = it.next();
        } else if let Some(v) = a.strip_prefix("--game=") {
            game = Some(v.to_string());
        } else {
            out.push(a);
        }
    }
    (out, game.filter(|g| !g.trim().is_empty()))
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Started by RecompLauncher.
pub fn launched() -> bool {
    static L: OnceLock<bool> = OnceLock::new();
    *L.get_or_init(|| env("RECOMPLAUNCHER").is_some_and(|v| v == "1"))
}

/// Play was pressed in the launcher's Big Picture mode (controller, couch, Steam Deck): open full screen.
pub fn big_picture() -> bool {
    static B: OnceLock<bool> = OnceLock::new();
    *B.get_or_init(|| env("RECOMPLAUNCHER_MODE").is_some_and(|v| v.eq_ignore_ascii_case("bigpicture")))
}

/// The display mode to open with: borderless full screen in Big Picture mode, else the player's own setting.
pub fn display_mode(saved: f32) -> f32 {
    if big_picture() { 2.0 } else { saved }
}

/// The controller the launcher saw plugged in (button icons before our own pad reading has seen one).
pub fn pad_hint() -> Option<crate::gamepad::PadKind> {
    match env("RECOMPLAUNCHER_PAD")?.to_ascii_lowercase().as_str() {
        "xbox" => Some(crate::gamepad::PadKind::Xbox),
        "playstation" => Some(crate::gamepad::PadKind::PlayStation),
        _ => None,
    }
}

/// Exit code for "no Deus Ex in the folder given" (recomplauncher.json's exit_codes explains it to the player).
pub const EXIT_NO_GAME: i32 = 2;

/// A folder that holds Deus Ex (the launcher's own check is System/DeusEx.u).
pub fn is_game_folder(root: &str) -> bool {
    // (resolve: the file's own letter case on Linux, where a Windows-made folder may say "system" or "DEUSEX.U".)
    unreal_pkg::paths::resolve(&std::path::Path::new(root).join("System").join("DeusEx.u")).exists()
}

/// `--prepare` (RecompLauncher's Level 3 setup step, run without a window after the player picks their copy):
/// checks the Deus Ex folder package by package, finds The Nameless Mod or mods unpacked beside the game, writes
/// launcher.txt's GamePath (so DX RUST's own launcher agrees) and prepared.txt. Prints `progress <0..1> <text>`
/// lines. Exit 0 = ready, 2 = no Deus Ex there, 3 = a package that won't open (a damaged or partial copy).
pub fn prepare(game: &str) -> i32 {
    use std::io::Write;
    let say = |p: f32, t: &str| {
        println!("progress {:.2} {t}", p.clamp(0.0, 1.0));
        let _ = std::io::stdout().flush();
    };
    let root = std::path::PathBuf::from(game);
    say(0.02, "Checking your Deus Ex folder");
    if !is_game_folder(game) {
        eprintln!("No System/DeusEx.u in {game}");
        return EXIT_NO_GAME;
    }
    unreal_pkg::paths::set(&root, Vec::new());
    // Every package the game reads at start, then every map: each must open and decode its tables.
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for (dir, ext) in [("System", "u"), ("Maps", "dx"), ("Textures", "utx"), ("Sounds", "uax"), ("Music", "umx")] {
        if let Ok(rd) = std::fs::read_dir(root.join(dir)) {
            let mut v: Vec<_> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext)))
                .collect();
            v.sort();
            files.extend(v);
        }
    }
    let maps = files.iter().filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dx"))).count();
    if maps == 0 {
        eprintln!("No maps in {game}/Maps");
        return EXIT_NO_GAME;
    }
    let mut bad = Vec::new();
    for (i, f) in files.iter().enumerate() {
        if i % 8 == 0 {
            say(0.05 + 0.6 * i as f32 / files.len() as f32, &format!("Checking {}", f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
        }
        if unreal_pkg::Package::open(f).is_err() {
            // Maps and code packages must open. Some texture / sound packages in every install are older-format
            // leftovers the game never reads (Render.utx, Palettes.utx are package version 61): only noted.
            let needed = f.parent().and_then(|d| d.file_name()).is_some_and(|d| d.eq_ignore_ascii_case("System") || d.eq_ignore_ascii_case("Maps"));
            if needed {
                bad.push(f.display().to_string());
            } else {
                println!("note: {} isn't read by DX RUST", f.display());
            }
        }
    }
    // Core packages that must be there for the game to start at all.
    for need in ["Core.u", "Engine.u", "DeusEx.u", "DeusExUI.u", "DeusExConText.u", "DeusExCharacters.u"] {
        if unreal_pkg::paths::find_ci(&root.join("System"), need).is_none() {
            bad.push(format!("System/{need} (missing)"));
        }
    }
    if !bad.is_empty() {
        for b in &bad {
            eprintln!("damaged or missing: {b}");
        }
        return 3;
    }
    say(0.7, "Reading the game's classes");
    let mut db = unreal_pkg::classes::ClassDb::open(&root.join("System"));
    let classes = db.classes_in("DeusEx").len();
    let _ = db.defaults("JCDentonMale");
    say(0.8, "Reading the conversations");
    let cons = dx_level::conversations::ConDb::load(&root.join("System")).map_or(0, |d| d.cons.len());
    say(0.9, "Looking for mods");
    let mut lines: Vec<(String, String)> = vec![("GamePath".into(), game.to_string())];
    // The Nameless Mod installed the way its readme asks (inside the game folder, Mods/TNM) or in DX RUST's Mods.
    let tnm_known = unreal_pkg::mods::read_launcher_txt(&unreal_pkg::paths::data_dir().join("launcher.txt"))
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("TNMPath") && unreal_pkg::tnm::find(std::path::Path::new(v)).is_some());
    if !tnm_known {
        for d in [root.join("Mods"), root.clone(), unreal_pkg::paths::data_dir().join("Mods")] {
            if let Some(t) = d.is_dir().then(|| unreal_pkg::tnm::find(&d)).flatten() {
                lines.push(("TNMPath".into(), t.display().to_string()));
                break;
            }
        }
    }
    if let Err(e) = modkit::install::set_launcher_lines(&lines, &[]) {
        eprintln!("launcher.txt: {e}");
    }
    let note = format!(
        "DX RUST prepared\ngame={game}\nmaps={maps}\npackages={}\nclasses={classes}\nconversations={cons}\n",
        files.len()
    );
    let _ = std::fs::write(unreal_pkg::paths::data_dir().join("prepared.txt"), note);
    say(1.0, &format!("Ready: {maps} maps, {cons} conversations"));
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn game_arg_is_taken_out() {
        let (a, g) = take_game_arg(v(&["dxview", "--game", "/games/Deus Ex", "01_NYC_UNATCOIsland.dx"]));
        assert_eq!(a, v(&["dxview", "01_NYC_UNATCOIsland.dx"]));
        assert_eq!(g.as_deref(), Some("/games/Deus Ex"));
        let (a, g) = take_game_arg(v(&["dxview", "--game=C:\\DX"]));
        assert_eq!(a, v(&["dxview"]));
        assert_eq!(g.as_deref(), Some("C:\\DX"));
        let (a, g) = take_game_arg(v(&["dxview", "C:\\DX", "map.dx"]));
        assert_eq!(a, v(&["dxview", "C:\\DX", "map.dx"]));
        assert!(g.is_none());
    }
}
