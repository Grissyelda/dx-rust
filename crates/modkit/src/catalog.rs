//! The mods DX RUST knows how to use, where each one is published, and how to spot its download.
//!
//! DX RUST ships none of these files. The player downloads each mod from its own page (the authors keep their
//! downloads, their credit and their page views); the mod manager only unpacks the file the player chose, on the
//! player's own machine.

/// One download a player can get.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub id: &'static str,
    pub name: &'static str,
    pub authors: &'static str,
    /// Its page (ModDB unless the authors publish elsewhere): "Get It" opens this.
    pub page: &'static str,
    /// Words in the downloaded file's name that mark it as this mod's (lower case).
    pub file_hints: &'static [&'static str],
    /// The mods (unreal_pkg::mods ids, or "tnm") this download carries once unpacked.
    pub provides: &'static [&'static str],
    /// One line for the player.
    pub about: &'static str,
}

pub const CATALOG: &[Entry] = &[
    Entry {
        id: "tnm",
        name: "The Nameless Mod",
        authors: "Off Topic Productions",
        page: "https://www.moddb.com/mods/the-nameless-mod",
        file_hints: &["nameless", "tnm"],
        provides: &["tnm"],
        about: "A whole new game on the Deus Ex engine, set in a world built from an internet forum. Played from the launcher's Game row.",
    },
    Entry {
        id: "lddp",
        name: "Lay D Denton Project",
        authors: "The LDDP team",
        page: "https://www.moddb.com/mods/the-lay-d-denton-project",
        file_hints: &["lay_d", "lay d", "layd", "lddp"],
        provides: &["lddp"],
        about: "Play as the female JC Denton: her model, voice and conversations.",
    },
    Entry {
        id: "gmdx_ae",
        name: "GMDX: Augmented Edition",
        authors: "Sarge945 and the GMDX: Augmented Edition team",
        page: "https://www.moddb.com/mods/give-me-deus-ex-augmented-edition",
        file_hints: &["gmdx"],
        provides: &["gmdx", "augmentique", "visibleattachments", "precipitation"],
        about: "Perks, plus Augmentique outfits, Visible Attachments and Precipitation rain in one download.",
    },
    Entry {
        id: "hdtp",
        name: "HDTP",
        authors: "The Project HDTP team",
        page: "https://www.moddb.com/mods/project-hdtp",
        file_hints: &["hdtp"],
        provides: &["hdtp"],
        about: "Higher-detail models and textures for weapons, items and some characters.",
    },
    Entry {
        id: "newvision",
        name: "New Vision",
        authors: "David \"DaveW\" Watts",
        page: "https://www.moddb.com/mods/new-vision",
        file_hints: &["new vision", "newvision", "new_vision"],
        provides: &["newvision"],
        about: "High-resolution textures for the world.",
    },
    Entry {
        id: "augmentique",
        name: "Augmentique",
        authors: "Sarge945",
        page: "https://www.moddb.com/mods/augmentique",
        file_hints: &["augmentique"],
        provides: &["augmentique"],
        about: "Outfits for JC and the people of the world.",
    },
    Entry {
        id: "helios",
        name: "Helios Texture Overhaul",
        authors: "BerZerker",
        page: "https://www.moddb.com/mods/helios-texture-overhaul",
        file_hints: &["helios", "hto"],
        provides: &["helios"],
        about: "Repainted characters, items and world textures.",
    },
    Entry {
        id: "relief",
        name: "Parallax Occlusion Addon",
        authors: "BerZerker",
        page: "https://www.moddb.com/mods/helios-texture-overhaul/downloads/parallax-occlusion-addon",
        file_hints: &["parallax", "pom"],
        provides: &["relief"],
        about: "Relief for walls and floors under Remastered Lighting.",
    },
    Entry {
        id: "precipitation",
        name: "Precipitation",
        authors: "Smoke39",
        page: "https://www.moddb.com/mods/precipitation",
        file_hints: &["precipitation"],
        provides: &["precipitation"],
        about: "Rain and snow for the Weather setting.",
    },
];

pub fn entry(id: &str) -> Option<&'static Entry> {
    CATALOG.iter().find(|e| e.id == id)
}

/// The catalog entry a downloaded file's name points to (the longest hint wins: "gmdx" over "dx").
pub fn guess(file_name: &str) -> Option<&'static Entry> {
    let n = file_name.to_ascii_lowercase();
    CATALOG
        .iter()
        .filter_map(|e| e.file_hints.iter().filter(|h| n.contains(*h)).map(|h| h.len()).max().map(|l| (l, e)))
        .max_by_key(|(l, _)| *l)
        .map(|(_, e)| e)
}

/// Open a page in the player's web browser (the system's own opener; nothing is downloaded by the game).
pub fn open_page(url: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(url).spawn();
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(url).spawn();
    r.map(|_| ()).map_err(|e| format!("couldn't open a browser ({e}): {url}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloads_are_recognised() {
        assert_eq!(guess("The Nameless Mod Installer 2025-11-05 1723.exe").map(|e| e.id), Some("tnm"));
        assert_eq!(guess("GMDX_Augmented_Edition_v2.1.7z").map(|e| e.id), Some("gmdx_ae"));
        assert_eq!(guess("HDTP_Release_2.zip").map(|e| e.id), Some("hdtp"));
        assert_eq!(guess("Lay_D_Denton_Project_1.1.zip").map(|e| e.id), Some("lddp"));
        assert!(guess("holiday photos.zip").is_none());
        for e in CATALOG {
            assert!(e.page.starts_with("https://"), "{}", e.id);
        }
    }
}
