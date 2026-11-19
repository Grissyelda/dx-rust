//! The Nameless Mod mode (the launcher's Game row): The Nameless Mod (Off Topic Productions, a whole game made on
//! Deus Ex) played from the player's own copy of its files - its ModDB download, unpacked by the launcher or the
//! in-game mod manager. Nothing of it ships with DX RUST.
//!
//! What this mode does: lays TNM2 and The Nameless Mod's folder over the game (unreal_pkg::tnm, its
//! TNMDefault.ini's search order), loads its code packages so its classes have their defaults and ancestry (its
//! characters, decorations and items act as the Deus Ex classes they extend), plays Trestkon from its start map,
//! reads its two halves of conversations and their speech, and plays its Ogg music (TNMOggMusicManager's tracks:
//! ambient, combat, conversation, death). Its gameplay is rewritten in Rust piece by piece: the difficulty filter
//! (`before_load`), out-of-world bodies and datacubes (scriptworld.rs), MSCallbackTrigger (triggers.rs) and the mission
//! scripts (missions/tnm.rs). What isn't rewritten yet plays with the Deus Ex classes it builds on.

use bevy::prelude::*;
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

/// The Nameless Mod's slow motion (Trestkon.slomoandpitch: GameSpeed / TimeDilation and the sound pitch with it).
static SPEED: Mutex<f32> = Mutex::new(1.0);

/// Set the game's speed (1 = normal); sound effects follow at 0.25 + 0.75 x speed.
pub fn set_speed(speed: f32) {
    if let Ok(mut s) = SPEED.lock() {
        *s = speed.clamp(0.05, 4.0);
    }
}

/// Applies the speed to virtual time and the mixer's pitch when it changes.
pub fn speed_system(mut time: ResMut<Time<Virtual>>, mut last: Local<f32>) {
    let s = SPEED.lock().map(|s| *s).unwrap_or(1.0);
    if *last == 0.0 {
        *last = 1.0;
    }
    if (s - *last).abs() > 1e-4 {
        *last = s;
        time.set_relative_speed(s);
        imsim_audio::set_master_pitch(s * 0.75 + 0.25);
    }
}

/// Before a map is built: The Nameless Mod's enemy / item appearance level for this playthrough, from the
/// difficulty picked (its Easy / Medium / Hard set both to 0 / 1 / 2; Realistic counts as Hard). Off outside TNM.
pub fn before_load() {
    // A new map plays at normal speed (the slow-motion scenes end on their own map).
    set_speed(1.0);
    // Trestkon's PawnDifficulty / ItemDifficulty: chosen with the difficulty (games from before the mod's menus
    // were ours: from the combat difficulty).
    let f = active().then(|| match difficulty() {
        Some(d) => (d.enemies.clamp(0, 3) as u8, d.items.clamp(0, 3) as u8),
        None => {
            let d = crate::travel::combat_difficulty();
            let n = if d <= 1.0 { 0 } else if d <= 1.5 { 1 } else { 2 };
            (n, n)
        }
    });
    dx_level::actors::set_difficulty_filter(f);
    let radish = active() || env_on!("DXVIEW_RADISH");
    dx_level::actors::set_level_fixups(radish.then(|| Box::new(crate::radish::level_fixup) as dx_level::actors::LevelFixups));
}

/// The player's class: Trestkon, or JC.
/// The intro cut scene's map: Deus Ex's 00_Intro, or The Nameless Mod's TNMIntro.
pub fn intro_map() -> &'static str {
    if active() {
        "TNMIntro.dx"
    } else {
        "00_Intro.dx"
    }
}

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

pub type Decoded = Arc<Mutex<Option<Option<Arc<imsim_audio::Clip>>>>>;

/// One Ogg file from The Nameless Mod's music folder, decoded on a thread (the credits' own track). The slot holds
/// Some(result) once the decode is done.
pub fn decode_ogg_file(name: &str) -> Option<Decoded> {
    let tnm = unreal_pkg::tnm::active()?;
    let dir = unreal_pkg::tnm::ogg_dir(&tnm)?;
    let slot: Decoded = Arc::new(Mutex::new(None));
    let (s2, name) = (slot.clone(), name.replace('\\', "/"));
    std::thread::spawn(move || {
        let clip = unreal_pkg::paths::find_ci(&dir, &name).and_then(|p| std::fs::read(p).ok()).and_then(|b| imsim_audio::decode_ogg(&b)).map(Arc::new);
        if clip.is_none() {
            eprintln!("music: {name} not found or not Ogg Vorbis in {}", dir.display());
        }
        if let Ok(mut s) = s2.lock() {
            *s = Some(clip);
        }
    });
    Some(slot)
}

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

// ------------------------------------------------------------------------------------------------ difficulty

/// The Nameless Mod's difficulty (tnmMenuSelectDifficulty / tnmMenuScreenCustomDifficulty -> tnmMenuScreenNewGame ->
/// Trestkon's travel values): combat difficulty, PawnHealth (the player's damage to pawns is multiplied by it), which of
/// a map's pawns and items appear (PawnDifficulty / ItemDifficulty, 0..3), Legendary and Nightmare.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Difficulty {
    pub combat: f32,
    pub pawn_health: f32,
    pub enemies: i32,
    pub items: i32,
    pub legendary: bool,
    pub nightmare: bool,
}

impl Default for Difficulty {
    fn default() -> Self {
        Difficulty { combat: 1.5, pawn_health: 1.0, enemies: 1, items: 1, legendary: false, nightmare: false }
    }
}

/// (chosen in the menus for the next new game, the game being played)
static DIFFICULTY: Mutex<(Option<Difficulty>, Option<Difficulty>)> = Mutex::new((None, None));

/// The menus' choice for the new game about to start (None: the extras, which use none).
pub fn set_pending_difficulty(d: Option<Difficulty>) {
    if let Ok(mut g) = DIFFICULTY.lock() {
        g.0 = d;
    }
}

/// A new game starts with the menus' choice.
pub fn start_difficulty() {
    if let Ok(mut g) = DIFFICULTY.lock() {
        g.1 = g.0.take();
    }
}

/// The game's difficulty values (None in games started without them).
pub fn difficulty() -> Option<Difficulty> {
    DIFFICULTY.lock().ok().and_then(|g| g.1)
}

/// A loaded game's values.
pub fn restore_difficulty(d: Option<Difficulty>) {
    if let Ok(mut g) = DIFFICULTY.lock() {
        g.1 = d;
    }
}

/// TNMGameInfo.ReduceDamage: the player's damage to a pawn times PawnHealth.
pub fn player_damage_mult() -> f32 {
    if !active() {
        return 1.0;
    }
    difficulty().map_or(1.0, |d| d.pawn_health)
}
