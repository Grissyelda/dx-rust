//! The Nameless Mod's puzzle machines, written for DX RUST from how they behave in the player's own copy of the mod:
//! the generator control panel (tnmComputerAPGPuzzle -> NetworkTerminalAPGPuzzle -> APGPuzzleWindow, the space
//! station's APGs) and the AI packet consoles (tnmAIPuzzle -> tnmNetworkTerminalAIPuzzle -> AIPuzzleWindow, the
//! Citadel and the station's AI locks). Their pictures, sounds, strings and the panel's button table are the mod's
//! own, read from the player's packages at run time; the rules are written here.
// uc: tnmComputerAPGPuzzle.Trigger, tnmComputerAPGPuzzle.Frob, tnmAIPuzzle.Tick, tnmAIPuzzle.Frob

use crate::tnmpc::{Outcome, Pointer};
use crate::uicanvas::Style;
use crate::weapons::RuntimeAssets;
use bevy::prelude::*;
use std::collections::HashMap;
use std::sync::Mutex;
use unreal_pkg::props::{find, Property, Value};

/// What a puzzle machine remembers: switched on (APG panels wait for a Trigger), solved, its packet layout.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Dev {
    pub enabled: bool,
    pub solved: bool,
    pub layout: Option<i64>,
}

static STATE: Mutex<Option<HashMap<usize, Dev>>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut HashMap<usize, Dev>) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(HashMap::new))
}

pub fn reset_level() {
    with(|m| m.clear());
}

/// The console being used is on 22_Citadel (AIPuzzleWindow's AltTitle).
static ALT_TITLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_map(name: &str) {
    ALT_TITLE.store(name.trim_end_matches(".dx").eq_ignore_ascii_case("22_Citadel"), std::sync::atomic::Ordering::Relaxed);
}

pub fn alt_title() -> bool {
    ALT_TITLE.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn save() -> Vec<(usize, Dev)> {
    with(|m| m.iter().map(|(k, v)| (*k, *v)).collect())
}

pub fn restore(v: &[(usize, Dev)]) {
    with(|m| {
        m.clear();
        m.extend(v.iter().copied());
    });
}

/// Which of the two machines a class is (The Nameless Mod only).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Machine {
    Apg,
    Ai,
}

pub fn machine(class: &str) -> Option<Machine> {
    if !crate::tnmmode::active() {
        return None;
    }
    if crate::classtree::is_a(class, "tnmComputerAPGPuzzle") {
        Some(Machine::Apg)
    } else if crate::classtree::is_a(class, "tnmAIPuzzle") {
        Some(Machine::Ai)
    } else {
        None
    }
}

fn name_prop(p: &[Property], k: &str) -> String {
    match find(p, k) {
        Some(Value::Name(n)) if !n.eq_ignore_ascii_case("None") => n.clone(),
        _ => String::new(),
    }
}

fn str_prop(p: &[Property], d: &[Property], k: &str) -> String {
    match find(p, k).or_else(|| find(d, k)) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

/// tnmComputerAPGPuzzle.Trigger: an event naming the panel's Tag switches it on.
pub fn trigger_event(lvl: &dx_level::Level, tag: &str) {
    if !crate::tnmmode::active() {
        return;
    }
    for (i, a) in lvl.actors.iter().enumerate() {
        if crate::ai::actor_tag(a).eq_ignore_ascii_case(tag) && machine(&a.class) == Some(Machine::Apg) {
            with(|m| m.entry(i).or_default().enabled = true);
        }
    }
}

/// The machine's Frob before its terminal: a message instead when it's off, or already done (None: open it).
pub fn frob_gate(lvl: &dx_level::Level, actor: usize, flags: &imsim_core::flags::FlagBase, assets: &mut RuntimeAssets) -> Option<String> {
    let a = lvl.actors.get(actor)?;
    let m = machine(&a.class)?;
    let d = assets.classes.defaults(&a.class);
    let dev = with(|s| *s.entry(actor).or_default());
    match m {
        Machine::Apg => {
            if dev.solved {
                Some(str_prop(&a.props, &d, "strGeneratorsOnline"))
            } else if !dev.enabled {
                Some(str_prop(&a.props, &d, "strGeneratorsOffline"))
            } else {
                None
            }
        }
        Machine::Ai => {
            let flag = name_prop(&a.props, "EnabledFlag");
            let want = matches!(find(&a.props, "EnabledFlagDesiredValue").or_else(|| find(&d, "EnabledFlagDesiredValue")), Some(Value::Bool(true)));
            if !flag.is_empty() && flags.get_bool(&flag) != want {
                Some(str_prop(&a.props, &d, "DisabledMessage"))
            } else if dev.solved {
                Some(str_prop(&a.props, &d, "SolvedMessage"))
            } else {
                None
            }
        }
    }
}

/// tnmAIPuzzle.Tick (once): its layout, at random but never the one its LayoutCheckFlag names, kept in its
/// LayoutStoreFlag. Returns (layout, flag to store it in).
pub fn layout(lvl: &dx_level::Level, actor: usize, flags: &imsim_core::flags::FlagBase, count: i64, seed: u64) -> (i64, Option<String>) {
    if let Some(l) = with(|s| s.get(&actor).and_then(|d| d.layout)) {
        return (l, None);
    }
    let Some(a) = lvl.actors.get(actor) else { return (0, None) };
    let count = match find(&a.props, "LayoutCount") {
        Some(Value::Int(n)) if *n > 0 => *n as i64,
        _ => count.max(1),
    };
    let check = name_prop(&a.props, "LayoutCheckFlag");
    let bad = if check.is_empty() {
        -1
    } else {
        match flags.get(&check) {
            Some(imsim_core::flags::FlagValue::Int(n)) => *n as i64,
            _ => 0,
        }
    };
    let mut s = seed | 1;
    let mut pick = 0;
    for _ in 0..64 {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        pick = ((s >> 33) % count as u64) as i64;
        if pick != bad || count == 1 {
            break;
        }
    }
    with(|m| m.entry(actor).or_default().layout = Some(pick));
    let store = name_prop(&a.props, "LayoutStoreFlag");
    (pick, (!store.is_empty()).then_some(store))
}

/// The machine solved: what to trigger, and (for the AI consoles) the skin it shows from now on.
pub fn solved(lvl: &dx_level::Level, actor: usize, assets: &mut RuntimeAssets, push_count: u32, hacked: bool) -> Vec<String> {
    let Some(a) = lvl.actors.get(actor) else { return Vec::new() };
    with(|m| m.entry(actor).or_default().solved = true);
    let mut events = Vec::new();
    match machine(&a.class) {
        Some(Machine::Apg) => {
            // APGPuzzleWindow.SolvePuzzle: the solve event, then the perfect one (three presses) or the other.
            events.push(name_prop(&a.props, "solve_event"));
            let perfect = name_prop(&a.props, "perfect_solve_event");
            if push_count == 3 && !perfect.is_empty() {
                events.push(perfect);
            } else {
                events.push(name_prop(&a.props, "inefficient_solve_event"));
            }
        }
        Some(Machine::Ai) => {
            // tnmNetworkTerminalAIPuzzle.Tick: bFinished, the solved texture, SolveEvent (SolveEventHacked hacked).
            events.push(name_prop(&a.props, if hacked { "SolveEventHacked" } else { "SolveEvent" }));
            let d = assets.classes.defaults(&a.class);
            if let Some(Value::Path(t)) = find(&a.props, "SolvedTexture").or_else(|| find(&d, "SolvedTexture")) {
                let slot = match find(&a.props, "SolvedTextureMultiSkinIndex") {
                    Some(Value::Int(n)) => (*n).max(0) as u32,
                    _ => 0,
                };
                crate::reskin::request_actor(actor, vec![(slot, t.clone())]);
            }
        }
        None => {}
    }
    events.retain(|e| !e.is_empty());
    events
}

/// A solved AI console keeps its solved picture (the original's saved MultiSkins): put back on a load.
pub fn reapply_skins(lvl: &dx_level::Level, assets: &mut RuntimeAssets) {
    let done: Vec<usize> = with(|m| m.iter().filter(|(_, d)| d.solved).map(|(k, _)| *k).collect());
    for actor in done {
        let Some(a) = lvl.actors.get(actor) else { continue };
        if machine(&a.class) != Some(Machine::Ai) {
            continue;
        }
        let d = assets.classes.defaults(&a.class);
        if let Some(Value::Path(t)) = find(&a.props, "SolvedTexture").or_else(|| find(&d, "SolvedTexture")) {
            let slot = match find(&a.props, "SolvedTextureMultiSkinIndex") {
                Some(Value::Int(n)) => (*n).max(0) as u32,
                _ => 0,
            };
            crate::reskin::request_actor(actor, vec![(slot, t.clone())]);
        }
    }
}

// ------------------------------------------------------------------------------------------------ the APG panel

/// The panel's button table (ButtonToggle button_actions[6]: which of the five generators each button flips), from
/// the window's DefaultPuzzleSettings in the player's script.
struct TableHost([[bool; 5]; 6]);

impl crate::uscript::Host for TableHost {
    fn call(&mut self, _name: &str, _args: &[crate::uscript::V]) -> Option<crate::uscript::V> {
        None
    }
    fn get(&mut self, _name: &str) -> crate::uscript::V {
        crate::uscript::V::None
    }
    fn get_index(&mut self, _name: &str, _index: i64, _field: Option<&str>) -> crate::uscript::V {
        crate::uscript::V::None
    }
    fn set(&mut self, path: &str, v: &crate::uscript::V) {
        // button_actions[i].toggles[j]
        let low = path.to_ascii_lowercase();
        let num = |s: &str, key: &str| -> Option<usize> {
            let i = s.find(key)? + key.len();
            s[i..].split(']').next()?.trim().parse().ok()
        };
        if let (Some(i), Some(j)) = (num(&low, "button_actions["), num(&low, "toggles[")) {
            if i < 6 && j < 5 {
                self.0[i][j] = v.int() == 1;
            }
        }
    }
}

/// A timed flicker of one generator turning on (UpdateFlicker every 0.025 s for 0.1..0.2 s).
#[derive(Clone, Copy)]
struct Flicker {
    left: f32,
    tick: f32,
    phase: u32,
}

pub struct Apg {
    pub client: (i32, i32),
    table: [[bool; 5]; 6],
    gens: [bool; 5],
    flicker: [Option<Flicker>; 5],
    active: usize,
    power: i32,
    power_t: f32,
    pushes: u32,
    finishing: bool,
    finish_t: f32,
    solve_wait: Option<f32>,
    solvable: bool,
    lit: [f32; 6],
    solve_down: bool,
    bg: Vec<String>,
    bg_cols: usize,
    bar: (String, String),
    gen_tex: [(String, String); 5],
    small: [String; 2],
    big: [String; 3],
    seed: u64,
    /// Flags the panel wrote (APG_Solved, APG_ButtonCount): read by computers.rs.
    pub flags_out: Vec<(String, imsim_core::flags::FlagValue)>,
    pub pushes_done: u32,
}

fn tex_at(d: &[Property], k: &str, i: u32) -> String {
    d.iter()
        .find(|p| p.name.eq_ignore_ascii_case(k) && p.index == i)
        .and_then(|p| match &p.value {
            Value::Path(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

impl Apg {
    pub fn new(assets: &mut RuntimeAssets, seed: u64, solved_before: Option<u32>) -> Apg {
        let wd = assets.classes.defaults("APGPuzzleWindow");
        let sd = assets.classes.defaults("APGPuzzleButtonSmall");
        let bd = assets.classes.defaults("APGPuzzleButtonBig");
        let int = |k: &str, def: i32| match find(&wd, k) {
            Some(Value::Int(n)) => *n,
            _ => def,
        };
        let mut table = TableHost([[false; 5]; 6]);
        if let Some(src) = assets.classes.script_text("APGPuzzleWindow") {
            let mut script = crate::uscript::Script::default();
            script.add_source(&src);
            script.call(&mut table, "defaultpuzzlesettings", &[]);
        }
        let mut a = Apg {
            client: (int("ClientWidth", 640), int("ClientHeight", 480)),
            table: table.0,
            gens: [false; 5],
            flicker: [None; 5],
            active: 0,
            power: 0,
            power_t: 0.0,
            pushes: 0,
            finishing: false,
            finish_t: 0.0,
            solve_wait: None,
            solvable: false,
            lit: [0.0; 6],
            solve_down: false,
            bg: (0..6).map(|i| tex_at(&wd, "clientTextures", i)).filter(|s| !s.is_empty()).collect(),
            bg_cols: int("textureCols", 3).max(1) as usize,
            bar: (tex_at(&wd, "texProgressBarOn", 0), tex_at(&wd, "texProgressBarOff", 0)),
            gen_tex: std::array::from_fn(|i| (tex_at(&wd, "texGeneratorOn", i as u32), tex_at(&wd, "texGeneratorOff", i as u32))),
            small: [tex_at(&sd, "buttonTextures", 0), tex_at(&sd, "buttonTextures", 1)],
            big: [tex_at(&bd, "buttonTextures", 0), tex_at(&bd, "buttonTextures", 1), tex_at(&bd, "buttonTextures", 2)],
            seed,
            flags_out: Vec::new(),
            pushes_done: 0,
        };
        // CreateControls: solved before (APG_Solved), every generator is up and Solve can be pressed.
        if let Some(n) = solved_before {
            a.gens = [true; 5];
            a.active = 5;
            a.solvable = true;
            a.power = 10;
            a.pushes = n;
        }
        a
    }

    fn rand(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.seed >> 33) as f32) / (1u64 << 31) as f32
    }

    fn grid(n: f32) -> i32 {
        (n * 32.0) as i32
    }

    fn toggle_pos(i: usize) -> (i32, i32) {
        (10 + Self::grid(3.0 + (i / 3) as f32 * 8.0), 55 + Self::grid(7.0 + (i % 3) as f32 * 2.0))
    }

    fn solve_pos() -> (i32, i32) {
        (10 + Self::grid(2.0), 55 + Self::grid(4.0))
    }

    /// ToggleGenerator.
    fn toggle(&mut self, g: usize) {
        if !self.gens[g] {
            self.gens[g] = true;
            let wait = 0.1 + 0.1 * self.rand();
            self.flicker[g] = Some(Flicker { left: wait, tick: 0.0, phase: 0 });
            self.active += 1;
        } else {
            self.gens[g] = false;
            self.active = self.active.saturating_sub(1);
        }
    }

    pub fn tick(&mut self, dt: f32, ptr: Pointer) -> Outcome {
        for l in self.lit.iter_mut() {
            *l = (*l - dt).max(0.0);
        }
        for f in self.flicker.iter_mut() {
            if let Some(fl) = f.as_mut() {
                fl.left -= dt;
                fl.tick += dt;
                while fl.tick >= 0.025 {
                    fl.tick -= 0.025;
                    fl.phase += 1;
                }
                if fl.left <= 0.0 {
                    *f = None;
                }
            }
        }
        // UpdatePower every 0.05 s: the bar climbs or falls toward two cells per running generator.
        self.power_t += dt;
        while self.power_t >= 0.05 {
            self.power_t -= 0.05;
            if !self.finishing {
                let want = self.active as i32 * 2;
                self.power += (want - self.power).signum();
            }
        }
        if self.finishing {
            // GeneratorFinishSequence every 0.2 s: the bar fills, then SolvePuzzle a second later.
            if let Some(w) = self.solve_wait.as_mut() {
                *w -= dt;
                if *w <= 0.0 {
                    self.pushes_done = self.pushes;
                    return Outcome::Finished;
                }
            } else {
                self.finish_t += dt;
                while self.finish_t >= 0.2 && self.solve_wait.is_none() {
                    self.finish_t -= 0.2;
                    if self.power < 10 {
                        self.power += 1;
                    } else {
                        self.solve_wait = Some(1.0);
                    }
                }
            }
            return Outcome::Stay;
        }
        let Some(m) = ptr.pos else { return Outcome::Stay };
        let inside = |(x, y): (i32, i32), s: i32| m.0 >= x && m.1 >= y && m.0 < x + s && m.1 < y + s;
        self.solve_down = ptr.down && inside(Self::solve_pos(), 64);
        if !ptr.pressed {
            return Outcome::Stay;
        }
        if self.solvable && inside(Self::solve_pos(), 64) {
            self.power = 0;
            self.finishing = true;
            crate::sound::play_player("DeusExSounds.Generic.Beep2", 1.0, 0.0);
            self.pushes += 1;
            return Outcome::Stay;
        }
        let Some(i) = (0..6).find(|&i| inside(Self::toggle_pos(i), 32)) else { return Outcome::Stay };
        self.lit[i] = 0.3;
        crate::sound::play_player("DeusExSounds.Generic.Beep1", 1.0, 0.0);
        for g in 0..5 {
            if self.table[i][g] {
                self.toggle(g);
            }
        }
        use imsim_core::flags::FlagValue;
        if self.active == 5 {
            crate::sound::play_player("DeusExSounds.UserInterface.Menu_BuySkills", 1.0, 0.0);
            self.flags_out.push(("APG_Solved".into(), FlagValue::Bool(true)));
            self.flags_out.push(("APG_ButtonCount".into(), FlagValue::Int(self.pushes as i32)));
            self.solvable = true;
        } else {
            self.solvable = false;
            self.flags_out.push(("APG_Solved".into(), FlagValue::Bool(false)));
            self.flags_out.push(("APG_ButtonCount".into(), FlagValue::Int(0)));
        }
        Outcome::Stay
    }

    pub fn draw<W: crate::menuui::KitWid>(&mut self, u: &mut crate::menuui::Ui<W>, x: i32, y: i32) {
        // The client's six background tiles.
        let (mut tx, mut ty, mut row_h) = (x, y, 0);
        for (k, b) in self.bg.clone().iter().enumerate() {
            if k > 0 && k % self.bg_cols == 0 {
                tx = x;
                ty += row_h;
                row_h = 0;
            }
            if let Some(t) = u.hud.tex(u.assets, b) {
                u.c.texture(tx, ty, t.w as i32, t.h as i32, 0, 0, &t, Style::Normal, [255, 255, 255]);
                tx += t.w as i32;
                row_h = row_h.max(t.h as i32);
            }
        }
        // PostDrawWindow: the power bar (red while the finish runs), then the five generators.
        let tint = if self.finishing { [255, 0, 0] } else { [255, 255, 255] };
        for i in 0..10 {
            let t = if i + 1 <= self.power { &self.bar.0 } else { &self.bar.1 };
            let at = (x + 42 + 16 + Self::grid(5.0 + i as f32), y + 39 + Self::grid(5.0));
            crate::tnmpc::stretch(u, t, (at.0 as f32, at.1 as f32, 32.0, 32.0), None, Style::Normal, tint);
        }
        for i in 0..5 {
            let flick = self.flicker[i].map(|f| if f.phase % 2 == 0 { 1 } else { 2 }).unwrap_or(0);
            let on = (self.gens[i] || flick == 1) && flick != 2;
            let t = if on { &self.gen_tex[i].0 } else { &self.gen_tex[i].1 };
            let at = (x + 12 + Self::grid(3.0 + i as f32 * 3.0), y + 23 + Self::grid(2.0));
            crate::tnmpc::stretch(u, t, (at.0 as f32, at.1 as f32, 64.0, 64.0), None, Style::Masked, [255, 255, 255]);
        }
        for i in 0..6 {
            let p = Self::toggle_pos(i);
            let t = if self.lit[i] > 0.0 { &self.small[1] } else { &self.small[0] };
            crate::tnmpc::stretch(u, t, ((x + p.0) as f32, (y + p.1) as f32, 32.0, 32.0), None, Style::Normal, [255, 255, 255]);
        }
        let p = Self::solve_pos();
        let t = if self.solve_down && self.solvable {
            &self.big[0]
        } else if self.solvable {
            &self.big[2]
        } else {
            &self.big[1]
        };
        crate::tnmpc::stretch(u, t, ((x + p.0) as f32, (y + p.1) as f32, 64.0, 64.0), None, Style::Normal, [255, 255, 255]);
    }
}

#[cfg(test)]
mod tests {
    //! Run against the player's own copy of the mod: TNM_SYS=<folder with TNM.u and TNMGUI.u>.
    use super::*;

    #[test]
    fn panel_table_from_the_mod() {
        let Ok(dir) = std::env::var("TNM_SYS") else { return };
        unreal_pkg::classes::set_extra_packages(unreal_pkg::tnm::PACKAGES);
        let mut db = unreal_pkg::classes::ClassDb::open(std::path::Path::new(&dir));
        let Some(src) = db.script_text("APGPuzzleWindow") else { return };
        let mut t = TableHost([[false; 5]; 6]);
        let mut script = crate::uscript::Script::default();
        script.add_source(&src);
        script.call(&mut t, "defaultpuzzlesettings", &[]);
        let per: Vec<usize> = t.0.iter().map(|r| r.iter().filter(|b| **b).count()).collect();
        println!("APG buttons flip {per:?} generators");
        assert!(per.iter().all(|n| *n > 0), "every button flips something");
        // Some three presses turn all five on from cold (the perfect solve exists).
        let mut found = false;
        for a in 0..6 {
            for b in 0..6 {
                for c in 0..6 {
                    let mut g = [false; 5];
                    for i in [a, b, c] {
                        for (k, on) in t.0[i].iter().enumerate() {
                            if *on {
                                g[k] = !g[k];
                            }
                        }
                    }
                    found |= g.iter().all(|x| *x);
                }
            }
        }
        assert!(found, "a three-press solve exists");
    }
}
