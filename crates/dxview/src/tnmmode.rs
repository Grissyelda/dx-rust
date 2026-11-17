//! The Nameless Mod mode (the launcher's Game row): The Nameless Mod (Off Topic Productions, a whole game made on
//! Deus Ex) played from the player's own copy of its files - its ModDB download, unpacked by the launcher or the
//! in-game mod manager. Nothing of it ships with DX RUST.
//!
//! What this mode does: lays TNM2 and The Nameless Mod's folder over the game (unreal_pkg::tnm, its
//! TNMDefault.ini's search order), loads its code packages so its classes have their defaults and ancestry (its
//! characters, decorations and items act as the Deus Ex classes they extend), plays Trestkon from its start map,
//! reads its two halves of conversations and their speech, and plays its Ogg music (TNMOggMusicManager's tracks:
//! ambient, combat, conversation, death). Its own UnrealScript gameplay (TNM.u: the player class, mission scripts,
//! menus) isn't ported, so what goes beyond the Deus Ex classes it builds on doesn't happen yet.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The Nameless Mod's folder for this run: `--tnm <folder>`, DXVIEW_TNM, or launcher.txt Game=tnm with TNMPath=.
/// The `--tnm` pair is taken out of `args`.
pub fn pick(args: &mut Vec<String>) -> Option<PathBuf> {
    let mut chosen = None;
    if let Some(i) = args.iter().position(|a| a == "--tnm") {
        if i + 1 < args.len() {
            chosen = Some(PathBuf::from(args.remove(i + 1)));
        }
        args.remove(i);
    }
    if chosen.is_none() {
        chosen = std::env::var_os("DXVIEW_TNM").filter(|v| !v.is_empty()).map(PathBuf::from);
    }
    if chosen.is_none() {
        let lk = unreal_pkg::mods::read_launcher_txt(&unreal_pkg::paths::data_dir().join("launcher.txt"));
        let on = lk.iter().any(|(k, v)| k.eq_ignore_ascii_case("Game") && v.eq_ignore_ascii_case("tnm"));
        if on {
            chosen = lk.iter().find(|(k, _)| k.eq_ignore_ascii_case("TNMPath")).map(|(_, v)| PathBuf::from(v));
        }
    }
    let dir = unreal_pkg::tnm::find(&chosen?)?;
    println!("dxview: The Nameless Mod from {}", dir.display());
    Some(dir)
}

/// Set up the folders and packages (instead of the mods: The Nameless Mod is played without them).
pub fn setup(root: &Path, tnm: &Path) {
    unreal_pkg::paths::set(root, unreal_pkg::tnm::overlays(tnm));
    unreal_pkg::classes::set_extra_packages(unreal_pkg::tnm::PACKAGES);
    unreal_pkg::tnm::set_active(Some(tnm.to_path_buf()));
    unreal_pkg::mods::activate(&[]);
}

pub fn active() -> bool {
    unreal_pkg::tnm::active().is_some()
}

/// The player's class: Trestkon, or JC.
pub fn player_class() -> &'static str {
    if active() {
        "Trestkon"
    } else {
        "JCDentonMale"
    }
}

/// The player's name before New Game names him.
pub fn default_name() -> &'static str {
    if active() {
        "Trestkon"
    } else {
        "JC Denton"
    }
}

// ------------------------------------------------------------------------------------------------ Ogg music

/// Which of a map's tracks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Track {
    Ambient,
    Combat,
    Conversation,
    Death,
}

type Decoded = Arc<Mutex<Option<Option<Arc<imsim_audio::Clip>>>>>;

/// A map's TNMOggMusicManager (or DXOggMusicManager): its track files, decoded in the background as needed.
pub struct OggMusic {
    dir: PathBuf,
    files: Vec<(Track, String, String)>,
    clips: std::collections::HashMap<String, Decoded>,
}

impl OggMusic {
    /// The music of a map, when The Nameless Mod is played and the map has a music manager.
    pub fn from_level(level: &dx_level::Level) -> Option<OggMusic> {
        use unreal_pkg::props::{find, Value};
        let tnm = unreal_pkg::tnm::active()?;
        let dir = unreal_pkg::tnm::ogg_dir(&tnm)?;
        let a = level.actors.iter().find(|a| a.class.eq_ignore_ascii_case("TNMOggMusicManager") || a.class.eq_ignore_ascii_case("DXOggMusicManager"))?;
        let s = |n: &str| match find(&a.props, n) {
            Some(Value::Str(v)) if !v.trim().is_empty() => v.trim().to_string(),
            _ => String::new(),
        };
        // TNMOggMusicManager.init: Ambient1 (Ambient2 only after the story moves on; the alternative soundtrack is
        // a player option not offered here).
        let (ai, am) = match (s("Ambient1IntroOggFile"), s("Ambient1OggFile")) {
            (i, m) if !m.is_empty() => (i, m),
            _ => (s("AmbientIntroOggFile"), s("AmbientOggFile")),
        };
        let mut files = vec![(Track::Ambient, ai, am)];
        files.push((Track::Combat, s("CombatIntroOggFile"), s("CombatOggFile")));
        files.push((Track::Conversation, s("ConversationIntroOggFile"), s("ConversationOggFile")));
        files.push((Track::Death, String::new(), s("DeathOggFile")));
        files.retain(|f| !f.2.is_empty());
        if files.is_empty() {
            return None;
        }
        println!("music: The Nameless Mod's {:?}", files.iter().map(|f| f.2.as_str()).collect::<Vec<_>>());
        let mut m = OggMusic { dir, files, clips: Default::default() };
        // The ambient and combat tracks now, so the first fight doesn't wait for a decode.
        for t in [Track::Ambient, Track::Combat] {
            m.ready(t);
        }
        Some(m)
    }

    pub fn has(&self, t: Track) -> bool {
        self.files.iter().any(|f| f.0 == t)
    }

    fn decode(&mut self, name: &str) -> Option<Arc<imsim_audio::Clip>> {
        if name.is_empty() {
            return None;
        }
        let key = name.to_ascii_lowercase();
        let slot = self.clips.entry(key).or_insert_with(|| {
            let slot: Decoded = Arc::new(Mutex::new(None));
            let (s2, dir, name) = (slot.clone(), self.dir.clone(), name.replace('\\', "/"));
            std::thread::spawn(move || {
                let clip = unreal_pkg::paths::find_ci(&dir, &name)
                    .and_then(|p| std::fs::read(p).ok())
                    .and_then(|b| imsim_audio::decode_ogg(&b))
                    .map(Arc::new);
                match &clip {
                    None => eprintln!("music: {name} not found or not Ogg Vorbis in {}", dir.display()),
                    Some(c) => println!("music: {name} decoded ({:.0} s)", c.duration()),
                }
                if let Ok(mut s) = s2.lock() {
                    *s = Some(clip);
                }
            });
            slot
        });
        slot.lock().ok()?.clone().flatten()
    }

    fn decoding(&self, name: &str) -> bool {
        !name.is_empty()
            && self.clips.get(&name.to_ascii_lowercase()).is_some_and(|s| s.lock().map(|s| s.is_none()).unwrap_or(false))
    }

    /// The track's (intro, loop) once decoded (the decode starts on the first ask). None while it decodes, or
    /// when the map has no such track.
    pub fn ready(&mut self, t: Track) -> Option<(Option<Arc<imsim_audio::Clip>>, Arc<imsim_audio::Clip>)> {
        let (_, intro, main) = self.files.iter().find(|f| f.0 == t)?.clone();
        let m = self.decode(&main);
        let i = self.decode(&intro);
        if self.decoding(&intro) {
            return None;
        }
        Some((i, m?))
    }
}
