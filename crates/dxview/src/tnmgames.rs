//! The Nameless Mod's computer-game records (Trestkon's GameHighScore* and PCGameData timeplayed, kept by the mod as
//! global config): the high scores of Tetris, Pong and Breakout, the number of Lights Out puzzles solved and the
//! time spent on each game. Kept for every game in TNMSave/games.txt in the data folder.

use std::collections::BTreeMap;
use std::sync::Mutex;

static STORE: Mutex<Option<BTreeMap<String, f64>>> = Mutex::new(None);

fn path() -> std::path::PathBuf {
    unreal_pkg::paths::data_dir().join("TNMSave").join("games.txt")
}

fn with<R>(f: impl FnOnce(&mut BTreeMap<String, f64>) -> R) -> R {
    let mut g = STORE.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(|| {
        let mut m = BTreeMap::new();
        if let Ok(t) = std::fs::read_to_string(path()) {
            for l in t.lines() {
                if let Some((k, v)) = l.split_once('=') {
                    if let Ok(v) = v.trim().parse() {
                        m.insert(k.trim().to_string(), v);
                    }
                }
            }
        }
        m
    });
    f(m)
}

fn save(m: &BTreeMap<String, f64>) {
    let p = path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let out: String = m.iter().map(|(k, v)| format!("{k}={v}\n")).collect();
    let _ = std::fs::write(p, out);
}

/// GameHighScore<Game> (Tetris, Pong, Breakout, LightsOut).
pub fn high(game: &str) -> i64 {
    with(|m| m.get(&format!("high.{game}")).copied().unwrap_or(0.0) as i64)
}

/// A new score: kept if it beats the record. True when it did.
pub fn offer(game: &str, score: i64) -> bool {
    with(|m| {
        let k = format!("high.{game}");
        if score as f64 > m.get(&k).copied().unwrap_or(0.0) {
            m.insert(k, score as f64);
            save(m);
            true
        } else {
            false
        }
    })
}

/// GameHighScoreLightsOut++ (the number of puzzles solved).
pub fn add(game: &str, n: i64) {
    with(|m| {
        *m.entry(format!("high.{game}")).or_insert(0.0) += n as f64;
        save(m);
    })
}

/// AddGameTimePlayed (saved once a whole second has gathered).
pub fn played(game: &str, secs: f32) {
    with(|m| {
        let k = format!("time.{game}");
        let before = m.get(&k).copied().unwrap_or(0.0);
        let after = before + secs as f64;
        m.insert(k, after);
        if after.floor() != before.floor() {
            save(m);
        }
    })
}
