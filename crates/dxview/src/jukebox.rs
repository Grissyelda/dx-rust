//! The Nameless Mod's music player: tnmPlayMusicWindow (every song: the PlayMusicWindow cheat) and tnmJukeboxWindow
//! (the songs unlocked, opened from the Jukebox accessory once the sword is unlocked), and the CollectableCD
//! decorations that unlock songs. The song table (names, CD names, intro and song files, musicians) is the window's
//! defaults in the player's TNM.u; its button captions and places are read from its CreateControls text there. The
//! window itself is drawn by menus.rs (a ToolWindow on the menu stack).
//!
//! Unlocked songs are Trestkon's globalconfig bUnlockedTrack<n>: settings "Tnm.bUnlockedTrack<n>", kept across
//! games like the original's ini.
// uc: tnmPlayMusicWindow.PopulateSongsList, tnmJukeboxWindow.PopulateSongsList, CollectableCD.Frob

use std::sync::Mutex;

use unreal_pkg::props::Value;

use crate::weapons::RuntimeAssets;

/// One row of the song table.
#[derive(Clone, Debug, Default)]
pub struct Song {
    pub index: usize,
    pub name: String,
    pub cd_name: String,
    pub intro: String,
    pub file: String,
    pub musician: String,
}

/// A tool button of the window: its variable (btnPlay ..), place, size and caption.
#[derive(Clone, Debug)]
pub struct ToolButton {
    pub var: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub caption: String,
}

/// The window's strings and controls, read once.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub title: String,
    pub buttons: Vec<ToolButton>,
    pub cd_caption: String,
    pub now_playing: String,
    /// tnmJukeboxWindow.strUnlocked ("%s" = unlocked / all).
    pub unlocked: String,
}

static OPEN: Mutex<Option<bool>> = Mutex::new(None);

/// Open the player: `unlocked_only` = the Jukebox (tnmJukeboxWindow), else every song (tnmPlayMusicWindow).
pub fn open(unlocked_only: bool) {
    if let Ok(mut g) = OPEN.lock() {
        *g = Some(unlocked_only);
    }
}

pub fn take_open() -> Option<bool> {
    OPEN.lock().ok().and_then(|mut g| g.take())
}

pub fn key(i: usize) -> String {
    format!("Tnm.bUnlockedTrack{i}")
}

pub fn unlocked(i: usize) -> bool {
    crate::settings::get(&key(i)) > 0.5
}

/// The table: tnmPlayMusicWindow's songNames / songNamesCD / songListIntros / songList / songMusician, rows with a
/// name only.
pub fn songs(assets: &mut RuntimeAssets) -> Vec<Song> {
    let d = assets.classes.defaults("tnmPlayMusicWindow");
    let get = |n: &str, i: usize| {
        d.iter().find(|p| p.name.eq_ignore_ascii_case(n) && p.index as usize == i).and_then(|p| match &p.value {
            Value::Str(s) => Some(s.clone()),
            _ => None,
        })
    };
    let count = d.iter().filter(|p| p.name.eq_ignore_ascii_case("songNames")).map(|p| p.index as usize + 1).max().unwrap_or(0);
    (0..count.max(124))
        .filter_map(|i| {
            let name = get("songNames", i).filter(|s| !s.is_empty())?;
            Some(Song {
                index: i,
                cd_name: get("songNamesCD", i).unwrap_or_else(|| name.clone()),
                name,
                intro: get("songListIntros", i).unwrap_or_default(),
                file: get("songList", i).unwrap_or_default(),
                musician: get("songMusician", i).unwrap_or_default(),
            })
        })
        .collect()
}

/// The number of song slots (arrayCount(songList)).
pub fn slots() -> usize {
    124
}

/// The string literal in `text` after `pat` (up to the next quote).
fn literal_after(text: &str, pat: &str) -> Option<String> {
    let i = text.find(pat)? + pat.len();
    let rest = &text[i..];
    let q = rest.find('"')?;
    let e = rest[q + 1..].find('"')?;
    Some(rest[q + 1..q + 1 + e].to_string())
}

/// tnmPlayMusicWindow.CreateControls / PlaySong: the buttons (CreateToolButton(x, y, caption) and the SetSize calls
/// after them), the CD names caption and "now playing" title, from the player's script text; tnmJukeboxWindow's
/// strUnlocked from its defaults.
pub fn layout(assets: &mut RuntimeAssets) -> Layout {
    let src = assets.classes.script_text("tnmPlayMusicWindow").unwrap_or_default();
    let mut out = Layout::default();
    out.title = literal_after(&src, "SetTitle(").unwrap_or_default();
    out.cd_caption = literal_after(&src, "btnCDNames.SetText(").unwrap_or_default();
    out.now_playing = literal_after(&src, "musicInfoTitle.SetText(").unwrap_or_default();
    for line in src.lines() {
        let t = line.trim();
        if t.starts_with("//") {
            continue;
        }
        if let Some(eq) = t.find("= CreateToolButton(") {
            let var = t[..eq].trim().to_string();
            let args = &t[eq + "= CreateToolButton(".len()..];
            let mut parts = args.splitn(3, ',');
            let (Some(x), Some(y), Some(rest)) = (parts.next(), parts.next(), parts.next()) else { continue };
            let caption = rest.trim().trim_start_matches('"');
            let caption = caption[..caption.rfind('"').unwrap_or(caption.len())].to_string();
            out.buttons.push(ToolButton { var, x: x.trim().parse().unwrap_or(0), y: y.trim().parse().unwrap_or(0), w: 75, h: 23, caption });
        } else if let Some(i) = t.find(".SetSize(") {
            let var = &t[..i];
            if let Some(b) = out.buttons.iter_mut().find(|b| b.var == var) {
                let args = t[i + 9..].trim_end_matches(';').trim_end_matches(')');
                let mut p = args.split(',');
                if let (Some(w), Some(h)) = (p.next(), p.next()) {
                    b.w = w.trim().parse().unwrap_or(b.w);
                    b.h = h.trim().parse().unwrap_or(b.h);
                }
            }
        }
    }
    let jd = assets.classes.defaults("tnmJukeboxWindow");
    out.unlocked = jd
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case("strUnlocked"))
        .and_then(|p| match &p.value {
            Value::Str(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_default();
    out
}

/// tnmPlayMusicWindow.BuildElapsedTimeString: minutes (two digits at least), a colon, then the seconds with up to
/// three more characters (two digits and the fraction's first: "01:05.2").
pub fn elapsed(seconds: f32) -> String {
    let mins = (seconds / 60.0).floor().max(0.0) as i32;
    let secs = seconds.max(0.0) % 60.0;
    let mut s = String::new();
    if mins < 10 {
        s.push('0');
    }
    s.push_str(&format!("{mins}:"));
    if secs < 10.0 {
        s.push('0');
    }
    // UnrealScript prints a float with six decimals; Mid(str, InStr(str, ":"), 3) keeps the colon and two more.
    s.push_str(&format!("{secs:.6}"));
    let colon = s.find(':').unwrap_or(0);
    let end = (colon + 3).min(s.len());
    s[..end].to_string()
}

/// CollectableCD.Frob: the songs a CD unlocks (UnlockTrack1..5, -1 = none). With the sword unlocked each new song
/// is announced (aquisitionmsg[2] with its name) or, all known, aquisitionmsg[3]; without, aquisitionmsg[1]. The
/// songs are unlocked either way and the CD is used up.
pub fn frob_cd(assets: &mut RuntimeAssets, props: &[unreal_pkg::props::Property]) {
    let tracks: Vec<usize> = (1..=5)
        .filter_map(|n| match unreal_pkg::props::find(props, &format!("UnlockTrack{n}")) {
            Some(Value::Int(t)) if *t >= 0 => Some(*t as usize),
            _ => None,
        })
        .collect();
    let d = assets.classes.defaults("CollectableCD");
    let msg = |i: u32| {
        d.iter().find(|p| p.name.eq_ignore_ascii_case("aquisitionmsg") && p.index == i).and_then(|p| match &p.value {
            Value::Str(s) => Some(s.clone()),
            _ => None,
        })
    };
    if crate::tnminvasion::unlocked() {
        let names = songs(assets);
        let mut all_known = true;
        for &t in &tracks {
            if !unlocked(t) {
                all_known = false;
                if let (Some(m), Some(s)) = (msg(2), names.iter().find(|s| s.index == t)) {
                    crate::classic_hud::client_message(&m.replacen("%s", &s.name, 1));
                }
            }
        }
        if all_known {
            if let Some(m) = msg(3) {
                crate::classic_hud::client_message(&m);
            }
        }
    } else if let Some(m) = msg(1) {
        crate::classic_hud::client_message(&m);
    }
    for t in tracks {
        crate::settings::set(&key(t), 1.0);
    }
    crate::settings::save();
}
