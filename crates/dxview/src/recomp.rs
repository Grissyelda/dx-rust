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
