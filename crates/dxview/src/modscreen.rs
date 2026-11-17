//! Settings > Mods: the in-game mod manager (modkit). Lists the mods DX RUST supports with their authors and pages,
//! finds their downloads in the player's Downloads folder, and unpacks them - zip / 7z archives and the 7-Zip / zip
//! self-extracting installers - into the game's own Mods folder, on this machine (a Steam Deck too: no Windows PC
//! needed to run an installer). Nothing is downloaded or shipped by the game: "Get It" opens the mod's own page.

use modkit::catalog::{Entry, CATALOG};
use modkit::install::{self, Found, State};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Job {
    name: String,
    frac: f32,
    file: String,
    done: Option<Result<install::Installed, String>>,
}

pub struct ModsView {
    pub sel: Option<usize>,
    pub scroll: i32,
    states: Vec<State>,
    found: Vec<Found>,
    job: Option<Arc<Mutex<Job>>>,
    /// Something changed this run: the game uses its mods from the next start.
    pub changed: bool,
}

impl ModsView {
    pub fn new() -> ModsView {
        let mut v = ModsView { sel: Some(0), scroll: 0, states: Vec::new(), found: Vec::new(), job: None, changed: false };
        v.refresh();
        v
    }

    /// Read launcher.txt and the download folders again.
    pub fn refresh(&mut self) {
        self.states = CATALOG.iter().map(install::state).collect();
        self.found = install::find_downloads();
    }

    fn entry(&self) -> Option<&'static Entry> {
        CATALOG.get(self.sel?)
    }

    fn found_for(&self, e: &Entry) -> Option<&Found> {
        self.found.iter().find(|f| f.entry.id == e.id)
    }

    pub fn busy(&self) -> bool {
        self.job.as_ref().is_some_and(|j| j.lock().map(|j| j.done.is_none()).unwrap_or(false))
    }

    pub fn rows(&self) -> Vec<Vec<String>> {
        let job = self.job.as_ref().and_then(|j| j.lock().ok().map(|j| (j.name.clone(), j.frac, j.done.is_none())));
        CATALOG
            .iter()
            .zip(&self.states)
            .map(|(e, s)| {
                let status = match (&job, s) {
                    (Some((n, f, true)), _) if n == e.id => format!("Installing... {:.0}%", f * 100.0),
                    (_, State::On(_)) if e.id == "tnm" && crate::tnmmode::active() => "Playing now".to_string(),
                    (_, State::On(_)) if e.id == "tnm" => "Installed".to_string(),
                    (_, State::On(_)) => "Installed, on".to_string(),
                    (_, State::Off(_)) => "Installed, off".to_string(),
                    (_, State::NotInstalled) if self.found_for(e).is_some() => "Download found".to_string(),
                    _ => "Not installed".to_string(),
                };
                vec![e.name.to_string(), status]
            })
            .collect()
    }

    pub fn help(&self) -> String {
        let Some(e) = self.entry() else { return "Mods DX RUST can use. Select one.".into() };
        let i = self.sel.unwrap_or(0);
        let state = match &self.states[i] {
            State::On(p) | State::Off(p) => format!("Installed in {}.", p.display()),
            State::NotInstalled => match self.found_for(e) {
                Some(f) => format!("Found {} ({}). Install unpacks it here.", f.path.display(), size(f.size)),
                None => "Get It opens its page: download it into your Downloads folder, then Install.".into(),
            },
        };
        let restart = if self.changed { " Restart to use your changes." } else { "" };
        format!("{} By {}. {}{}", e.about, e.authors, state, restart)
    }

    pub fn key(&self, h: &mut impl Hasher) {
        (self.sel, self.scroll, self.changed, self.found.len()).hash(h);
        for s in &self.states {
            std::mem::discriminant(s).hash(h);
        }
        if let Some(j) = self.job.as_ref().and_then(|j| j.lock().ok().map(|j| ((j.frac * 100.0) as u32, j.done.is_some()))) {
            j.hash(h);
        }
    }

    pub fn move_sel(&mut self, d: i32) {
        let n = CATALOG.len() as i32;
        self.sel = Some((self.sel.map_or(0, |s| s as i32) + d).clamp(0, n - 1) as usize);
    }

    /// Install: the found download of the selected mod, on a thread (progress in its row).
    pub fn install(&mut self) -> Option<String> {
        if self.busy() {
            return Some("Wait for the install that's running to finish.".into());
        }
        let e = self.entry()?;
        let Some(f) = self.found_for(e).cloned() else {
            let dirs: Vec<String> = install::download_dirs().iter().map(|d| d.display().to_string()).collect();
            return Some(format!(
                "No download of {} found. Get It opens its page; save the file (the .exe installer, .7z or .zip) in one of: {}. Then Install.",
                e.name,
                dirs.join(", ")
            ));
        };
        let job = Arc::new(Mutex::new(Job { name: e.id.to_string(), ..Default::default() }));
        self.job = Some(job.clone());
        std::thread::spawn(move || {
            let mut last = std::time::Instant::now();
            let r = install::install(&f.path, &mut |frac, file| {
                if last.elapsed().as_millis() > 50 {
                    last = std::time::Instant::now();
                    if let Ok(mut j) = job.lock() {
                        j.frac = frac;
                        j.file = file.to_string();
                    }
                }
            });
            if let Ok(mut j) = job.lock() {
                j.done = Some(r);
            }
        });
        None
    }

    /// The main button: Install for a mod that isn't installed, else Turn On / Turn Off.
    pub fn main_label(&self) -> &'static str {
        match (self.entry(), self.sel.and_then(|i| self.states.get(i))) {
            (Some(e), Some(State::On(_))) if e.id != "tnm" => "Turn Off",
            (Some(e), Some(State::Off(_))) if e.id != "tnm" => "Turn On",
            (Some(e), Some(State::On(_))) if e.id == "tnm" && crate::tnmmode::active() => "Play Deus Ex",
            (Some(e), Some(State::On(_))) if e.id == "tnm" => "Play It",
            _ => "Install",
        }
    }

    /// The main button pressed: (message, ask to restart).
    pub fn main_action(&mut self) -> Option<(String, bool)> {
        match self.main_label() {
            "Turn Off" | "Turn On" => self.toggle(),
            "Play It" => {
                set_pending_game(Some("tnm"));
                Some(("Restart into The Nameless Mod now? A game in progress isn't saved.".into(), true))
            }
            "Play Deus Ex" => {
                set_pending_game(Some("dx"));
                Some(("Restart into Deus Ex now? A game in progress isn't saved.".into(), true))
            }
            _ => self.install().map(|t| (t, false)),
        }
    }

    /// Each frame while the screen is up: a finished install's message (and whether to offer a restart).
    pub fn poll(&mut self) -> Option<(String, bool)> {
        let done = self.job.as_ref()?.lock().ok()?.done.take()?;
        let name = self.job.take().and_then(|j| j.lock().ok().map(|j| j.name.clone())).unwrap_or_default();
        self.refresh();
        let title = modkit::catalog::entry(&name).map_or("The mod", |e| e.name);
        Some(match done {
            Ok(r) => {
                self.changed = true;
                let mods: Vec<&str> = r.mods.iter().map(|(id, _)| *id).collect();
                if mods.contains(&"tnm") {
                    set_pending_game(Some("tnm"));
                    (format!("{title} is installed ({} files). Restart into it now? (Later: Play It on this screen, or the launcher's Game row.)", r.files), true)
                } else {
                    (format!("{title} is installed and on ({} files). Restart the game now to use it?", r.files), true)
                }
            }
            Err(e) => (format!("{title} wasn't installed. {e}"), false),
        })
    }

    pub fn toggle(&mut self) -> Option<(String, bool)> {
        let e = self.entry()?;
        let i = self.sel?;
        let on = match &self.states[i] {
            State::NotInstalled => return Some((format!("{} isn't installed.", e.name), false)),
            _ if e.id == "tnm" => return None,
            State::On(_) => false,
            State::Off(_) => true,
        };
        if let Err(err) = install::set_entry_on(e, on) {
            return Some((format!("launcher.txt couldn't be written: {err}"), false));
        }
        self.changed = true;
        self.refresh();
        Some((format!("{} is now {}. Restart the game now to use your change?", e.name, if on { "on" } else { "off" }), true))
    }

    pub fn remove(&mut self) -> Option<String> {
        let e = self.entry()?;
        if self.busy() {
            return Some("Wait for the install that's running to finish.".into());
        }
        if matches!(self.states[self.sel?], State::NotInstalled) {
            return Some(format!("{} isn't installed.", e.name));
        }
        let r = install::remove_entry(e);
        self.changed = true;
        self.refresh();
        Some(match r {
            Ok(()) => format!("{} is removed (files the game unpacked are deleted; a folder of your own is left alone).", e.name),
            Err(err) => format!("{} couldn't be removed: {err}", e.name),
        })
    }

    pub fn page(&self) -> Option<String> {
        let e = self.entry()?;
        modkit::catalog::open_page(e.page).err()
    }
}

fn size(b: u64) -> String {
    if b >= 1 << 30 {
        format!("{:.1} GB", b as f64 / (1u64 << 30) as f64)
    } else {
        format!("{:.0} MB", b as f64 / (1u64 << 20) as f64)
    }
}

/// The game to start on the next restart (launcher.txt Game=), set when the player asks to switch.
static PENDING_GAME: std::sync::Mutex<Option<&'static str>> = std::sync::Mutex::new(None);

fn set_pending_game(g: Option<&'static str>) {
    if let Ok(mut p) = PENDING_GAME.lock() {
        *p = g;
    }
}

/// Start the game again (the mods are chosen at start-up), switching between Deus Ex and The Nameless Mod
/// when the player asked for that.
pub fn restart() {
    crate::settings::save();
    if let Some(g) = PENDING_GAME.lock().ok().and_then(|mut p| p.take()) {
        let _ = install::set_launcher_lines(&[("Game".into(), g.into())], &[]);
    }
    if let Ok(exe) = std::env::current_exe() {
        // `--tnm <folder>` and DXVIEW_TNM would override the Game line: left out of the new start.
        let mut args: Vec<String> = Vec::new();
        let mut it = std::env::args().skip(1);
        while let Some(a) = it.next() {
            if a == "--tnm" {
                it.next();
                continue;
            }
            args.push(a);
        }
        if std::process::Command::new(exe).args(args).env_remove("DXVIEW_TNM").spawn().is_ok() {
            std::process::exit(0);
        }
    }
}
