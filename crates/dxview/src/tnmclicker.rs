//! The Nameless Mod's idle game on its terminals (clickerWindow, "The Idle Mod"): hit the target (or let the
//! weapons do it on a timer), collect what drops, sell it, buy weapons and their mods, open loot with lockpicks
//! and passwords, follow the quests to new places, and meet the bosses.
//!
//! The rules are ported here: the damage loop, the maximum health curve, the credit bonus, the loot rolls over
//! the class's loot tables, selling, buying, mods, items, the boss checks and the log. What exists in the mod only
//! as switch tables in code - the target's size and graphic, a place's background, difficulty, enemies and boss,
//! which loot table a kill or a box uses, the music per place, which quests / weapons / places a state unlocks,
//! the quest rewards and the quest list text - is read from the player's own TNMGUI script through the uscript
//! data loader. Progress is `config` in the mod (kept across games): data_dir/TNMSave/clicker.ron.
//!
//! Layout deviation: the original spreads nine windows over the whole screen; here they share one terminal window
//! (the play area as the original's 600 x 400 client, the panels as tabs beside it).
// uc: clickerWindow.InitWindow, clickerWindow.ConfigurationChanged, clickerWindow.calculateMaxHealth
// uc: clickerWindow.GetCreditsBonus, clickerWindow.GetIdleAttack, clickerWindow.GetClickAttack
// uc: clickerWindow.BuildNumberSeparator, clickerWindow.GetTimerRate, clickerWindow.DoTimedAttack
// uc: clickerWindow.TargetTakeDamage, clickerWindow.DoLowHealthStuff, clickerWindow.SetLocation, clickerWindow.CheckBoss
// uc: clickerWindow.ShouldBeBoss, clickerWindow.AddEnergy, clickerWindow.AddMaxEnergy, clickerWindow.AddCredits
// uc: clickerWindow.isSafeAdd, clickerWindow.AddQuest, clickerWindow.CompleteQuest, clickerWindow.awardLootbox
// uc: clickerWindow.awardKillLoot, clickerWindow.AddLogMessage, clickerWindow.ActivateItem
// uc: clickerWindowInventoryNodal.SellItem, clickerWindowInventoryNodal.SellAllItem, clickerWindowToolsNodal.UseItem
// uc: clickerWindowToolsNodal.UseAllItem, clickerWindowWeaponsNodal.BuyItem, clickerWindowWeaponsNodal.BuyAccuracyMod
// uc: clickerWindowWeaponsNodal.GetAccuracyModCost, clickerWindowMap.PopulateList, clickerWindowMap.ListSelectionChanged
// uc: clickerWindowLog.PopulateLog, clickerWindowStats.Refresh

use bevy::input::ButtonInput;
use bevy::prelude::KeyCode;
use std::collections::HashMap;
use std::sync::Arc;
use unreal_pkg::props::Value;

use crate::menuui::{KitWid, Ui};
use crate::tnmpc::{fmtn, font, Pointer, Rng};
use crate::uicanvas::{HAlign, Style, UiFont, VAlign};
use crate::uscript::{Host, Script, V};
use crate::weapons::RuntimeAssets;

const MAX_INT: i64 = i32::MAX as i64;

/// The `config` variables (the mod keeps them in its ini, so they last across games), with their array sizes.
const CONFIG: [(&str, usize); 20] = [
    ("enemieskilledtype", 63),
    ("enemieskilledlocation", 63),
    ("itemquantity", 63),
    ("itemdiscovered", 63),
    ("weaponquantity", 55),
    ("weapondiscovered", 55),
    ("weaponmodsappliedaccuracy", 55),
    ("weaponmodsappliedrange", 55),
    ("weaponmodsappliedreload", 55),
    ("weaponmodsappliedclip", 55),
    ("credits", 0),
    ("megacredits", 0),
    ("energy", 0),
    ("maxenergy", 0),
    ("passwords", 63),
    ("queststarted", 25),
    ("questcomplete", 25),
    ("bossdefeated", 63),
    ("locationunlocked", 63),
    ("locationvisited", 63),
];

fn key(name: &str, i: Option<i64>) -> String {
    match i {
        Some(i) => format!("{name}[{i}]"),
        None => name.to_string(),
    }
}

fn to_v(v: &Value) -> V {
    match v {
        Value::Int(i) => V::Int(*i as i64),
        Value::Byte(b) => V::Int(*b as i64),
        Value::Float(f) => V::Float(*f as f64),
        Value::Bool(b) => V::Bool(*b),
        Value::Str(s) | Value::Name(s) | Value::Path(s) => V::Str(s.clone()),
        _ => V::None,
    }
}

fn v_int(v: &V) -> i64 {
    v.int()
}
fn v_str(v: &V) -> String {
    match v {
        V::Str(s) => s.clone(),
        V::Int(i) => i.to_string(),
        V::Float(f) => format!("{f:.6}"),
        V::Bool(b) => if *b { "True".into() } else { "False".into() },
        _ => String::new(),
    }
}

/// BuildNumberSeparator: thousands with commas.
fn sep(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 {
        format!("-{out}")
    } else {
        out
    }
}

/// A `const` of a class script (timerReductionPerMod, MaxMods).
fn script_const(src: &str, name: &str) -> Option<f64> {
    src.lines().find_map(|l| {
        let t = l.trim();
        let rest = t.strip_prefix("const ").or_else(|| t.strip_prefix("CONST "))?;
        let (k, v) = rest.split_once('=')?;
        if !k.trim().eq_ignore_ascii_case(name) {
            return None;
        }
        let v = v.split(';').next()?.trim();
        v.parse().ok()
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Map,
    Weapons,
    Inventory,
    Tools,
    Quests,
    Stats,
    Log,
    Active,
}

const TABS: [Tab; 8] = [Tab::Map, Tab::Weapons, Tab::Inventory, Tab::Tools, Tab::Quests, Tab::Stats, Tab::Log, Tab::Active];

/// The game's state: the class defaults (tables), the variables (by name, arrays as "name[i]"), the log.
struct State {
    defs: HashMap<String, V>,
    vals: HashMap<String, V>,
    log: Vec<String>,
    rng: Rng,
    /// What the script asked for this call.
    completes: Vec<i64>,
    text: String,
    music: Option<String>,
    unlocks_due: bool,
}

impl State {
    fn get(&self, name: &str, i: Option<i64>) -> V {
        let k = key(name, i);
        if let Some(v) = self.vals.get(&k) {
            return v.clone();
        }
        if let Some(v) = self.defs.get(&k) {
            return v.clone();
        }
        // An unset element of a default array (UnrealScript: zero / empty).
        V::None
    }
    fn i(&self, name: &str, i: Option<i64>) -> i64 {
        v_int(&self.get(name, i))
    }
    fn f(&self, name: &str, i: Option<i64>) -> f64 {
        self.get(name, i).float()
    }
    fn s(&self, name: &str, i: Option<i64>) -> String {
        v_str(&self.get(name, i))
    }
    fn set(&mut self, name: &str, i: Option<i64>, v: V) {
        self.vals.insert(key(name, i), v);
    }
    fn seti(&mut self, name: &str, i: Option<i64>, v: i64) {
        self.set(name, i, V::Int(v.clamp(i32::MIN as i64, MAX_INT)));
    }
    fn add(&mut self, name: &str, i: Option<i64>, n: i64) {
        let cur = self.i(name, i);
        self.seti(name, i, cur.saturating_add(n));
    }
    fn b(&self, name: &str, i: i64) -> bool {
        self.i(name, Some(i)) != 0
    }

    /// AddLogMessage: "[hh:mm:ss] text", active messages marked with ';' (the two logs).
    fn log(&mut self, msg: &str, active: bool) {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) % 86_400;
        let ts = format!("[{:02}:{:02}:{:02}]", secs / 3600, (secs % 3600) / 60, secs % 60);
        self.log.push(format!("{}{ts} {msg}", if active { ";" } else { "" }));
    }
}

/// The scripts' host: variables from State, the engine's natives, the windows as objects.
struct ClickHost<'a>(&'a mut State);

const OBJ_WINDOW: u32 = 1;
const OBJ_TEXT: u32 = 2;

impl Host for ClickHost<'_> {
    fn call(&mut self, name: &str, args: &[V]) -> Option<V> {
        let st = &mut *self.0;
        let a = |i: usize| args.get(i).cloned().unwrap_or(V::None);
        Some(match name {
            "rand" => V::Int(st.rng.rand(a(0).int() as i32) as i64),
            "frand" => V::Float(st.rng.frand() as f64),
            "min" => V::Int(a(0).int().min(a(1).int())),
            "max" => V::Int(a(0).int().max(a(1).int())),
            "fmin" => V::Float(a(0).float().min(a(1).float())),
            "fmax" => V::Float(a(0).float().max(a(1).float())),
            "sprintf" => {
                let rest: Vec<String> = args.iter().skip(1).map(v_str).collect();
                let refs: Vec<&str> = rest.iter().map(|s| s.as_str()).collect();
                V::Str(fmtn(&v_str(&a(0)), &refs))
            }
            "buildnumberseparator" => V::Str(sep(a(0).int())),
            "arraycount" => {
                let n = v_str(&a(0));
                let size = CONFIG.iter().find(|c| c.0 == n).map(|c| c.1).filter(|s| *s > 0).unwrap_or(match n.as_str() {
                    "questcompletion" | "questgoal" => 25,
                    "bossready" | "isboss" | "targetname" | "targetgraphic" | "strlocationname" | "itemname" | "itemvalue" => 63,
                    "weaponvalue" | "weaponname" => 55,
                    _ => 16,
                });
                V::Int(size as i64)
            }
            "addlogmessage" => {
                st.log(&v_str(&a(0)), a(1).truthy());
                V::None
            }
            "addcredits" => {
                let n = a(0).int();
                if n != 0 {
                    st.add("credits", None, n);
                }
                st.unlocks_due = true;
                V::None
            }
            "addquest" => {
                let q = a(0).int();
                if !st.b("queststarted", q) {
                    st.seti("queststarted", Some(q), 1);
                }
                V::None
            }
            "completequest" => {
                st.completes.push(a(0).int());
                V::None
            }
            "shouldbeboss" => {
                let l = st.i("location", None);
                V::Bool(st.b("bossready", l) && !st.b("bossdefeated", l))
            }
            "updatemapwindow" | "updatestatswindow" | "updatetoolswindow" | "updateinventorywindow" | "updateweaponswindow" | "updatequestswindow"
            | "updatelogwindow" | "updateaugswindow" | "log" | "playsound" => V::None,
            _ => return None,
        })
    }
    fn get(&mut self, name: &str) -> V {
        match name {
            "clickerparent" | "winquests" | "winmap" | "musicplayer" | "player" | "winscroll" | "winstats" => V::Obj(OBJ_WINDOW),
            "wintext" => V::Obj(OBJ_TEXT),
            _ => self.0.get(name, None),
        }
    }
    fn get_index(&mut self, name: &str, index: i64, _field: Option<&str>) -> V {
        self.0.get(name, Some(index))
    }
    fn set(&mut self, path: &str, v: &V) {
        match path.split_once('[') {
            Some((n, rest)) => {
                let i = rest.trim_end_matches(']').parse().ok();
                self.0.set(n, i, v.clone());
            }
            None => self.0.set(path, None, v.clone()),
        }
    }
    fn method(&mut self, recv: &V, name: &str, args: &[V]) -> (V, Vec<(usize, V)>) {
        let st = &mut *self.0;
        let a = |i: usize| args.get(i).map(v_str).unwrap_or_default();
        let r = match name {
            "cr" => V::Str("\n".into()),
            "appendtext" if *recv == V::Obj(OBJ_TEXT) => {
                st.text.push_str(&a(0));
                V::None
            }
            "settext" if *recv == V::Obj(OBJ_TEXT) => {
                st.text = a(0);
                V::None
            }
            "setcurrentogg" => {
                st.music = Some(a(1));
                V::None
            }
            "completequest" => {
                st.completes.push(args.first().map(|v| v.int()).unwrap_or(0));
                V::None
            }
            "addlogmessage" => {
                st.log(&a(0), args.get(1).is_some_and(|v| v.truthy()));
                V::None
            }
            _ => V::None,
        };
        (r, Vec::new())
    }
    fn field(&mut self, _recv: &V, name: &str) -> V {
        match name.split_once('[') {
            Some((n, rest)) => {
                let i = rest.trim_end_matches(']').parse().ok();
                self.0.get(n, i)
            }
            None if name == "blistmade" => V::Bool(true),
            None => self.0.get(name, None),
        }
    }
}

pub struct Clicker {
    st: State,
    /// clickerWindow's script (the tables) and clickerWindowQuests's (the quest list).
    win: Script,
    quests: Script,
    timer: f32,
    rate: f32,
    reduction: f64,
    max_mods: i64,
    pub client: (i32, i32),
    tab: Tab,
    scroll: [i32; 8],
    title: Option<Arc<UiFont>>,
    small: Option<Arc<UiFont>>,
    headers: Option<Arc<UiFont>>,
    /// Panel strings: (class, key) -> text.
    strs: HashMap<String, String>,
    hover_note: String,
    quest_text: String,
    dirty: bool,
    save_t: f32,
}

fn save_path() -> std::path::PathBuf {
    unreal_pkg::paths::data_dir().join("TNMSave").join("clicker.ron")
}

impl Clicker {
    pub fn new(assets: &mut RuntimeAssets, seed: u64) -> Clicker {
        let mut defs = HashMap::new();
        for class in ["clickerWindowQuests", "clickerWindow"] {
            for p in assets.classes.defaults(class).iter() {
                let k = format!("{}[{}]", p.name.to_ascii_lowercase(), p.index);
                let v = to_v(&p.value);
                if p.index == 0 {
                    defs.insert(p.name.to_ascii_lowercase(), v.clone());
                }
                defs.insert(k, v);
            }
        }
        let mut strs = HashMap::new();
        for class in ["clickerWindowWeaponsNodal", "clickerWindowInventoryNodal", "clickerWindowToolsNodal", "clickerWindowStats", "clickerWindowLog", "clickerWindowInventory"] {
            for p in assets.classes.defaults(class).iter() {
                if let Value::Str(s) = &p.value {
                    strs.insert(format!("{class}.{}", p.name.to_ascii_lowercase()), s.clone());
                }
            }
            for k in ["Title"] {
                if let Some(Value::Str(s)) = unreal_pkg::props::find(&assets.classes.defaults(class), k) {
                    strs.insert(format!("{class}.title"), s.clone());
                }
            }
        }
        for class in ["clickerWindowMap", "clickerWindowWeapons", "clickerWindowTools", "clickerWindowQuests", "clickerWindowLogActive"] {
            if let Some(Value::Str(s)) = unreal_pkg::props::find(&assets.classes.defaults(class), "Title") {
                strs.insert(format!("{class}.title"), s.clone());
            }
        }
        let src = assets.classes.script_text("clickerWindow").unwrap_or_default();
        let mut win = Script::default();
        win.objects = true;
        win.add_source(&src);
        let mut quests = Script::default();
        quests.objects = true;
        quests.add_source(&assets.classes.script_text("clickerWindowQuests").unwrap_or_default());
        let mut st = State { defs, vals: HashMap::new(), log: Vec::new(), rng: Rng(seed | 1), completes: Vec::new(), text: String::new(), music: None, unlocks_due: false };
        // The kept progress (config), over the defaults.
        if let Some(saved) = std::fs::read_to_string(save_path()).ok().and_then(|t| ron::from_str::<HashMap<String, i64>>(&t).ok()) {
            for (k, v) in saved {
                st.vals.insert(k, V::Int(v));
            }
        }
        // InitWindow: the log cleared, the first target at the first place.
        st.seti("location", None, 0);
        st.seti("currentenemytype", None, 0);
        st.set("bossdifficulty", None, V::Float(0.0));
        let mut c = Clicker {
            st,
            win,
            quests,
            timer: 0.0,
            rate: 1.0,
            reduction: script_const(&src, "timerReductionPerMod").unwrap_or(0.00125),
            max_mods: script_const(&src, "MaxMods").unwrap_or(5.0) as i64,
            client: (800, 400),
            tab: Tab::Map,
            scroll: [0; 8],
            title: font(assets, "DeusExUI.FontMenuTitle"),
            small: font(assets, "DeusExUI.FontMenuSmall_DS").or_else(|| font(assets, "DeusExUI.FontMenuSmall")),
            headers: font(assets, "DeusExUI.FontMenuHeaders"),
            strs,
            hover_note: String::new(),
            quest_text: String::new(),
            dirty: false,
            save_t: 0.0,
        };
        let mh = c.max_health();
        c.st.seti("maxhealth", None, mh);
        c.st.seti("currenthealth", None, mh);
        c.rate = c.timer_rate();
        c.refresh_quests();
        c
    }

    fn call(&mut self, f: &str, args: &[V]) -> V {
        let mut h = ClickHost(&mut self.st);
        self.win.call(&mut h, f, args)
    }
    fn str_of(&self, class: &str, k: &str) -> String {
        self.strs.get(&format!("{class}.{}", k.to_ascii_lowercase())).cloned().unwrap_or_default()
    }

    fn location(&self) -> i64 {
        self.st.i("location", None)
    }
    fn enemy(&self) -> i64 {
        self.st.i("currentenemytype", None)
    }
    /// GetLocationDifficulty (the script's table).
    fn difficulty(&mut self) -> i64 {
        self.call("getlocationdifficulty", &[]).int()
    }
    /// calculateMaxHealth: 100 ^ (boss difficulty + 1 + 0.0685 x place difficulty), MaxInt past the top.
    fn max_health(&mut self) -> i64 {
        let d = self.difficulty() as f64;
        let boss = self.st.f("bossdifficulty", None);
        let h = 100f64.powf(boss + 1.0 + 0.0685 * d);
        if !h.is_finite() || h >= MAX_INT as f64 {
            MAX_INT
        } else {
            h as i64
        }
    }
    /// GetCreditsBonus.
    fn credits_bonus(&mut self) -> f64 {
        let d = self.difficulty() as f64;
        (100f64.powf(0.025 * d) * d).max(1.0)
    }
    fn attack(&self, click: bool) -> i64 {
        let (base, k) = if click { (5, "weaponclickdamage") } else { (1, "weaponidledamage") };
        let mut v: i64 = base;
        for i in 0..55 {
            if self.st.i("weaponquantity", Some(i)) > 0 {
                let d = self.st.i(k, Some(i));
                let t = d + d * self.st.i("weaponmodsappliedaccuracy", Some(i)) + d * self.st.i("weaponmodsappliedrange", Some(i));
                v = v.saturating_add(t);
            }
        }
        v
    }
    /// GetTimerRate.
    fn timer_rate(&self) -> f32 {
        let mut r = 1.0f64;
        for i in 0..55 {
            if self.st.i("weaponquantity", Some(i)) > 0 {
                r -= self.reduction * self.st.i("weaponmodsappliedclip", Some(i)) as f64;
                r -= self.reduction * self.st.i("weaponmodsappliedreload", Some(i)) as f64;
            }
        }
        r.max(0.05) as f32
    }

    fn add_credits(&mut self, n: i64) {
        if n != 0 {
            let c = self.st.i("credits", None);
            let v = if n > 0 && c > MAX_INT - n { MAX_INT } else { c + n };
            self.st.seti("credits", None, v);
        }
        self.call("updateweaponunlocks", &[]);
    }
    fn add_energy(&mut self, n: i64) {
        let e = (self.st.i("energy", None).saturating_add(n)).min(MAX_INT);
        let m = self.st.i("maxenergy", None);
        self.st.seti("energy", None, e.min(m));
    }

    /// The script's own follow-ups (AddCredits inside a table function, CompleteQuest from the quest list).
    fn settle(&mut self) {
        for _ in 0..8 {
            let completes = std::mem::take(&mut self.st.completes);
            let due = std::mem::replace(&mut self.st.unlocks_due, false);
            if completes.is_empty() && !due {
                break;
            }
            if due {
                self.call("updateweaponunlocks", &[]);
            }
            for q in completes {
                self.complete_quest(q);
            }
        }
        if self.st.log.len() > 3500 {
            let n = self.st.log.len();
            self.st.log.clear();
            let s = fmtn(&self.str_of("clickerWindowLog", "strLogCleared"), &[&n.to_string()]);
            self.st.log(&s, true);
        }
        if let Some(m) = self.st.music.take() {
            if !m.is_empty() {
                crate::sound::set_music(Some(&format!("ogg:{m}")));
            }
        }
        self.dirty = true;
    }

    /// CompleteQuest: the goal sound, the reward (the script's AwardQuest), new places.
    fn complete_quest(&mut self, q: i64) {
        if self.st.b("questcomplete", q) {
            return;
        }
        crate::tnmpc::sound("TNM.tnmLogGoalCompleted");
        self.st.seti("questcomplete", Some(q), 1);
        self.call("awardquest", &[V::Int(q)]);
        self.call("updatelocationunlocks", &[]);
    }

    /// clickerWindowQuests.Refresh: progress, completions, and the list's text.
    fn refresh_quests(&mut self) {
        self.st.text.clear();
        {
            let mut h = ClickHost(&mut self.st);
            self.quests.call(&mut h, "refresh", &[]);
        }
        self.quest_text = std::mem::take(&mut self.st.text);
        self.settle();
    }

    /// TargetTakeDamage.
    fn hit(&mut self, manual: bool) {
        let d = self.attack(manual);
        let h = self.st.i("currenthealth", None).saturating_sub(d);
        if h <= 0 {
            self.st.seti("currenthealth", None, 0);
            self.killed();
        } else {
            self.st.seti("currenthealth", None, h);
        }
    }

    /// DoLowHealthStuff: the kill counted, its loot, the next target.
    fn killed(&mut self) {
        let (t, l) = (self.enemy(), self.location());
        self.st.add("enemieskilledtype", Some(t), 1);
        self.st.add("enemieskilledlocation", Some(l), 1);
        self.kill_loot();
        if self.st.b("isboss", t) {
            self.st.seti("bossdefeated", Some(l), 1);
        }
        self.st.set("bossdifficulty", None, V::Float(0.0));
        self.check_boss();
        self.next_target();
        self.call("updatelocationunlocks", &[]);
        self.call("updatemusic", &[]);
        self.settle();
    }

    fn next_target(&mut self) {
        let e = self.call("getenemytype", &[]).int();
        self.st.seti("currentenemytype", None, e);
        let mh = self.max_health();
        self.st.seti("maxhealth", None, mh);
        self.st.seti("currenthealth", None, mh);
    }

    /// CheckBoss: a place's boss is ready once enough of its people fell (the quest goals).
    fn check_boss(&mut self) {
        let l = self.location();
        let k = |s: &Self, i: i64| s.st.i("enemieskilledlocation", Some(i));
        let goal = |s: &Self, q: i64| s.st.i("questgoal", Some(q));
        let started = |s: &Self, q: i64| s.st.b("queststarted", q);
        let ready = match l {
            1 => k(self, 1) >= goal(self, 1) - 1 && started(self, 1),
            7 => k(self, 7) >= 100,
            21 => k(self, 21) >= goal(self, 20) - 1,
            24 => k(self, 24) >= goal(self, 5) - 1,
            25 => k(self, 25) >= goal(self, 6) - 1,
            34 => k(self, 34) >= goal(self, 23) - 1 && started(self, 23),
            41 => (38..=42).map(|i| k(self, i)).sum::<i64>() >= 1000,
            45 => k(self, 45) >= goal(self, 12) - 1,
            46 => k(self, 46) >= goal(self, 13) - 1,
            _ => false,
        };
        if ready {
            self.st.seti("bossready", Some(l), 1);
        }
    }

    /// SetLocation (the map's choice).
    fn set_location(&mut self, l: i64) {
        self.st.seti("location", None, l);
        self.st.seti("locationvisited", Some(l), 1);
        self.st.set("bossdifficulty", None, V::Float(0.0));
        self.check_boss();
        self.next_target();
        self.call("updatelocationunlocks", &[]);
        self.call("updatemusic", &[]);
        self.settle();
    }

    /// One loot table rolled: each slot by its chance; -1 = credits (times the place's bonus), 17 = passwords here.
    fn roll_table(&mut self, prefix: &str, n: i64, boxed: bool) {
        let msg = if boxed { "strunboxed" } else { "strobtained" };
        for i in 0..16 {
            let tk = |s: &str| format!("{prefix}{n}{s}");
            let chance = self.st.f(&tk("chance"), Some(i));
            if chance <= 0.0 || self.st.rng.frand() as f64 > chance {
                continue;
            }
            let item = self.st.i(&tk("item"), Some(i));
            let (lo, hi) = (self.st.i(&tk("quantitymin"), Some(i)), self.st.i(&tk("quantitymax"), Some(i)));
            let mut count = self.st.rng.rand((hi - lo + 1) as i32) as i64 + lo;
            let fmt = self.st.s(msg, None);
            if item == -1 {
                count = (count as f64 * self.credits_bonus()) as i64;
                self.add_credits(count);
                let amount = if boxed { count.to_string() } else { sep(count) };
                let s = fmtn(&fmt, &[&amount, &self.st.s("strcredits", None)]);
                self.st.log(&s, boxed);
            } else {
                if item == 17 {
                    let l = self.location();
                    self.st.add("passwords", Some(l), count);
                    self.st.seti("itemdiscovered", Some(17), 1);
                } else {
                    self.st.add("itemquantity", Some(item), count);
                    self.st.seti("itemdiscovered", Some(item), 1);
                }
                let s = fmtn(&fmt, &[&count.to_string(), &self.st.s("itemname", Some(item))]);
                self.st.log(&s, boxed);
            }
        }
    }

    /// awardKillLoot: the defeat line, the kill's table (the script picks which; 1 otherwise), then unlocks.
    fn kill_loot(&mut self) {
        let name = self.st.s("targetname", Some(self.enemy()));
        let s = fmtn(&self.st.s("strdefeated", None), &[&name]);
        self.st.log(&s, false);
        let mut t = self.call("getkillloottable", &[]).int();
        if !(2..=11).contains(&t) {
            t = 1;
        }
        self.roll_table("killloottable", t, false);
        self.call("updateweaponunlocks", &[]);
        self.call("updatelocationunlocks", &[]);
    }

    /// awardLootbox: a lockpick or a password opens a box at this place.
    fn loot_box(&mut self, lockpick: bool) {
        let s = self.st.s(if lockpick { "strusedlockpick" } else { "strusedpassword" }, None);
        self.st.log(&s, true);
        let t = self.call("getlootboxloottable", &[]).int();
        if (1..=6).contains(&t) {
            self.roll_table("lootboxloottable", t, true);
        }
        // Freeing Kevo (the second day's DXO).
        if self.location() == 29 && self.st.b("queststarted", 9) {
            self.st.seti("questcomplete", Some(9), 1);
            self.call("updatelocationunlocks", &[]);
        }
    }

    /// ActivateItem (the Tools panel).
    fn activate(&mut self, id: i64) {
        match id {
            13 => {
                if self.st.i("energy", None) != self.st.i("maxenergy", None) {
                    self.add_energy(25);
                    self.st.add("itemquantity", Some(id), -1);
                }
            }
            14 => {
                let m = self.st.i("maxenergy", None).saturating_add(25).min(MAX_INT);
                self.st.seti("maxenergy", None, m);
                self.add_energy(25);
                self.st.add("itemquantity", Some(id), -1);
            }
            15 => {
                self.loot_box(true);
                self.st.add("itemquantity", Some(id), -1);
            }
            17 => {
                self.loot_box(false);
                let l = self.location();
                self.st.add("passwords", Some(l), -1);
            }
            _ => {}
        }
        self.settle();
    }
    fn item_count(&self, id: i64) -> i64 {
        if id == 17 {
            self.st.i("passwords", Some(self.location()))
        } else {
            self.st.i("itemquantity", Some(id))
        }
    }

    fn mod_cost(&self, id: i64, kind: usize) -> i64 {
        let (k, f) = [("weaponmodsappliedaccuracy", 2.0), ("weaponmodsappliedrange", 3.0), ("weaponmodsappliedclip", 1.0), ("weaponmodsappliedreload", 2.0)][kind];
        let n = self.st.i(k, Some(id)) as f64 + 1.0;
        let v = self.st.i("weaponvalue", Some(id)) as f64 * (1.0 + (f * n) * n);
        v.min(MAX_INT as f64) as i64
    }

    fn buy_weapon(&mut self, id: i64) {
        let cost = self.st.i("weaponvalue", Some(id));
        if self.st.i("credits", None) >= cost && self.st.i("weaponquantity", Some(id)) == 0 {
            self.st.add("weaponquantity", Some(id), 1);
            self.st.add("credits", None, -cost);
            crate::tnmpc::sound("TNMGUI.Tetris_Rotate");
            let s = fmtn(&self.str_of("clickerWindowWeaponsNodal", "strBoughtItem"), &[&self.st.s("weaponname", Some(id)), &cost.to_string()]);
            self.st.log(&s, true);
            self.rate = self.timer_rate();
            self.dirty = true;
        }
    }
    fn buy_mod(&mut self, id: i64, kind: usize) {
        let k = ["weaponmodsappliedaccuracy", "weaponmodsappliedrange", "weaponmodsappliedclip", "weaponmodsappliedreload"][kind];
        let cost = self.mod_cost(id, kind);
        if self.st.i("weaponquantity", Some(id)) == 0 || self.st.i(k, Some(id)) >= self.max_mods || self.st.i("credits", None) < cost {
            return;
        }
        self.st.add("credits", None, -cost);
        let what = self.str_of("clickerWindowWeaponsNodal", ["strAccuracy", "strRange", "strClip", "strReload"][kind]);
        let s = fmtn(&self.str_of("clickerWindowWeaponsNodal", "strBoughtMod"), &[&what, &self.st.s("weaponname", Some(id)), &cost.to_string()]);
        self.st.log(&s, true);
        self.st.add(k, Some(id), 1);
        crate::tnmpc::sound("TNMGUI.Tetris_Rotate");
        self.rate = self.timer_rate();
        self.dirty = true;
    }
    fn sell(&mut self, id: i64, all: bool) {
        let q = self.st.i("itemquantity", Some(id));
        if q <= 0 {
            return;
        }
        let v = self.st.i("itemvalue", Some(id));
        let n = if all { q } else { 1 };
        let gain = v.saturating_mul(n);
        let c = self.st.i("credits", None);
        if gain > 0 && c > MAX_INT - gain {
            return;
        }
        self.add_credits(gain);
        self.st.add("itemquantity", Some(id), -n);
        crate::tnmpc::sound(if all { "TNMGUI.Tetris_Fall" } else { "TNMGUI.Tetris_Rotate" });
        self.settle();
    }

    fn save(&self) {
        let mut out: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
        for (name, n) in CONFIG {
            if n == 0 {
                out.insert(name.to_string(), self.st.i(name, None));
            } else {
                for i in 0..n as i64 {
                    let v = self.st.i(name, Some(i));
                    let d = self.st.defs.get(&key(name, Some(i))).map(v_int).unwrap_or(0);
                    if v != d || self.st.vals.contains_key(&key(name, Some(i))) {
                        out.insert(key(name, Some(i)), v);
                    }
                }
            }
        }
        let p = save_path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(t) = ron::to_string(&out) {
            let _ = std::fs::write(p, t);
        }
    }

    // ------------------------------------------------------------------------------------------------ frame

    pub fn tick(&mut self, dt: f32, _keys: &ButtonInput<KeyCode>, ptr: Pointer) {
        crate::tnmgames::played("IDLE", dt);
        // The idle attack loop (DoTimedAttack).
        self.timer += dt;
        let mut n = 0;
        while self.timer >= self.rate && n < 50 {
            self.timer -= self.rate;
            n += 1;
            self.hit(false);
            self.call("updatequests", &[]);
            self.refresh_quests();
        }
        if n >= 50 {
            self.timer = 0.0;
        }
        self.save_t += dt;
        if self.dirty && self.save_t > 5.0 {
            self.save_t = 0.0;
            self.dirty = false;
            self.save();
        }
        let Some(pos) = ptr.pos else { return };
        // Scrolling the panel.
        if ptr.wheel != 0.0 && pos.0 >= 600 {
            let t = TABS.iter().position(|t| *t == self.tab).unwrap_or(0);
            self.scroll[t] = (self.scroll[t] - (ptr.wheel * 24.0) as i32).max(0);
        }
        self.hover_note.clear();
        if !ptr.pressed {
            let _ = self.panel(pos, false);
            return;
        }
        // The target (winTarget: a button).
        let (tw, th) = self.target_size();
        let (tx, ty) = (300 - tw / 2, 200 - th / 2);
        if pos.0 >= tx && pos.0 < tx + tw && pos.1 >= ty && pos.1 < ty + th {
            self.hit(true);
            self.settle();
            return;
        }
        // Tabs.
        for (i, t) in TABS.iter().enumerate() {
            let (bx, by) = (604 + (i as i32 % 4) * 49, 4 + (i as i32 / 4) * 18);
            if pos.0 >= bx && pos.0 < bx + 47 && pos.1 >= by && pos.1 < by + 16 {
                self.tab = *t;
                crate::tnmpc::sound("DeusExSounds.Menu.Press");
                return;
            }
        }
        if let Some(act) = self.panel(pos, true) {
            self.act(act);
        }
    }

    fn target_size(&mut self) -> (i32, i32) {
        let w = self.call("gettargetsizewidth", &[]).int() as i32;
        let h = self.call("gettargetsizeheight", &[]).int() as i32;
        (if w > 0 { w } else { 64 }, if h > 0 { h } else { 64 })
    }

    fn act(&mut self, a: Act) {
        match a {
            Act::Go(l) => {
                if l != self.location() {
                    self.set_location(l);
                }
            }
            Act::Buy(id) => self.buy_weapon(id),
            Act::Mod(id, k) => self.buy_mod(id, k),
            Act::Sell(id, all) => self.sell(id, all),
            Act::SellAll => {
                for id in 0..13 {
                    self.sell(id, true);
                }
            }
            Act::Use(id, all) => {
                if self.item_count(id) > 0 {
                    crate::tnmpc::sound("TNMGUI.Tetris_RemoveLine");
                    let mut i = 0;
                    loop {
                        self.activate(id);
                        i += 1;
                        if !all || self.item_count(id) <= 0 || i >= 1000 {
                            break;
                        }
                    }
                }
            }
        }
    }

    /// The side panel's rows: (y, height) laid out from the top of the list; with `click`, the action under `pos`.
    fn panel(&mut self, pos: (i32, i32), click: bool) -> Option<Act> {
        let t = TABS.iter().position(|t| *t == self.tab).unwrap_or(0);
        let top = 44 - self.scroll[t];
        let inside = |x: i32, y: i32, w: i32, h: i32| pos.0 >= x && pos.0 < x + w && pos.1 >= y && pos.1 < y + h && pos.1 >= 44;
        match self.tab {
            Tab::Map => {
                let mut y = top;
                for l in 0..63i64 {
                    if self.st.b("locationunlocked", l) && !self.st.s("strlocationname", Some(l)).is_empty() {
                        if click && inside(604, y, 192, 14) {
                            return Some(Act::Go(l));
                        }
                        y += 15;
                    }
                }
            }
            Tab::Weapons => {
                let mut y = top;
                for id in 0..55i64 {
                    if !self.st.b("weapondiscovered", id) {
                        continue;
                    }
                    if inside(708, y + 4, 88, 14) {
                        if click {
                            return Some(Act::Buy(id));
                        }
                        self.hover_note = format!("{} {}c", self.str_of("clickerWindowWeaponsNodal", "strBuy"), sep(self.st.i("weaponvalue", Some(id))));
                    }
                    for k in 0..4 {
                        if inside(604 + k as i32 * 48, y + 42, 46, 14) {
                            if click {
                                return Some(Act::Mod(id, k));
                            }
                            let label = self.str_of("clickerWindowWeaponsNodal", ["strBuyAccuracy", "strBuyRange", "strBuyClip", "strBuyReload"][k]);
                            self.hover_note = format!("{label}: {}c", sep(self.mod_cost(id, k)));
                        }
                    }
                    y += 60;
                }
            }
            Tab::Inventory => {
                let mut y = top;
                for id in 0..13i64 {
                    if !self.st.b("itemdiscovered", id) {
                        continue;
                    }
                    if click && inside(604, y + 30, 92, 14) {
                        return Some(Act::Sell(id, false));
                    }
                    if click && inside(700, y + 30, 92, 14) {
                        return Some(Act::Sell(id, true));
                    }
                    y += 48;
                }
                if click && inside(604, y + 4, 188, 16) {
                    return Some(Act::SellAll);
                }
            }
            Tab::Tools => {
                let mut y = top;
                for id in 13..18i64 {
                    if !self.st.b("itemdiscovered", id) {
                        continue;
                    }
                    if click && inside(604, y + 42, 92, 14) {
                        return Some(Act::Use(id, false));
                    }
                    if click && inside(700, y + 42, 92, 14) {
                        return Some(Act::Use(id, true));
                    }
                    y += 60;
                }
            }
            _ => {}
        }
        None
    }

    pub fn draw<W: KitWid>(&mut self, u: &mut Ui<W>, x: i32, y: i32) {
        // The play area: the place's background, the target, its health, name and the place.
        let bg = v_str(&self.call("getbackground", &[]));
        if let Some(t) = u.hud.tex(u.assets, &bg) {
            crate::tnmpc::stretch(u, &bg, (x as f32, y as f32, 600.0, 400.0), Some((0, 0, t.w as i32, t.h as i32)), Style::Normal, [255, 255, 255]);
        }
        let (tw, th) = self.target_size();
        let (tx, ty) = (x + 300 - tw / 2, y + 200 - th / 2);
        let g = self.st.s("targetgraphic", Some(self.enemy()));
        if let Some(t) = u.hud.tex(u.assets, &g) {
            crate::tnmpc::stretch(u, &g, (tx as f32, ty as f32, tw as f32, th as f32), Some((0, 0, t.w as i32, t.h as i32)), Style::Masked, [255, 255, 255]);
        }
        // textColName (the defaults' colour struct: green).
        let name_col = [0u8, 255, 0];
        if let Some(f) = self.small.clone() {
            let (cur, max) = (self.st.i("currenthealth", None), self.st.i("maxhealth", None));
            let text = format!("{} / {}", sep(cur), sep(max));
            let w = f.chars_width(&text.chars().collect::<Vec<_>>()).max(tw);
            let bx = tx + tw / 2 - w / 2;
            let by = ty + th + 5;
            // ProgressBarWindow (scaled colour, drawn background).
            u.c.fill(bx, by, w, f.height, Style::Normal, [32, 32, 32]);
            let frac = if max > 0 { cur as f32 / max as f32 } else { 0.0 };
            let c = crate::uicanvas::color_scaled(frac);
            u.c.fill(bx, by, (w as f32 * frac) as i32, f.height, Style::Normal, c);
            u.c.text(&f, bx, by, w, f.height, HAlign::Center, VAlign::Center, [0, 0, 0], &text);
            if let Some(tf) = self.title.clone() {
                let name = self.st.s("targetname", Some(self.enemy()));
                u.c.text(&tf, tx - 100, by + f.height, tw + 200, tf.height, HAlign::Center, VAlign::Center, name_col, &name);
                let place = self.st.s("strlocationname", Some(self.location()));
                u.c.text(&tf, x + 8, y + 4, 584, tf.height, HAlign::Center, VAlign::Top, name_col, &place);
            }
        }
        self.draw_panel(u, x + 600, y);
    }

    fn draw_panel<W: KitWid>(&mut self, u: &mut Ui<W>, x: i32, y: i32) {
        u.c.fill(x, y, 200, 400, Style::Normal, [12, 12, 16]);
        let Some(f) = self.small.clone() else { return };
        let white = [255, 255, 255];
        let grey = [128, 128, 128];
        let red = [192, 0, 0];
        // Tabs (the panels' window titles).
        let titles = [
            self.str_of("clickerWindowMap", "title"),
            self.str_of("clickerWindowWeapons", "title"),
            self.str_of("clickerWindowInventory", "title"),
            self.str_of("clickerWindowTools", "title"),
            self.str_of("clickerWindowQuests", "title"),
            self.str_of("clickerWindowStats", "title"),
            self.str_of("clickerWindowLog", "title"),
            self.str_of("clickerWindowLogActive", "title"),
        ];
        for (i, t) in TABS.iter().enumerate() {
            let (bx, by) = (x + 4 + (i as i32 % 4) * 49, y + 4 + (i as i32 / 4) * 18);
            let on = *t == self.tab;
            u.c.fill(bx, by, 47, 16, Style::Translucent, if on { [80, 80, 110] } else { [40, 40, 52] });
            let label = titles[i].rsplit(" - ").next().unwrap_or(&titles[i]).to_string();
            u.c.text(&f, bx, by, 47, 16, HAlign::Center, VAlign::Center, if on { white } else { grey }, &label);
        }
        let t = TABS.iter().position(|t| *t == self.tab).unwrap_or(0);
        let top = y + 44 - self.scroll[t];
        let clip_top = y + 44;
        let visible = |yy: i32, h: i32| yy >= clip_top && yy + h <= y + 384;
        let button = |u: &mut Ui<W>, bx: i32, by: i32, w: i32, label: &str, live: bool, solid_red: bool| {
            if !visible(by, 14) {
                return;
            }
            let fill = if solid_red { [96, 16, 16] } else { [48, 48, 48] };
            u.c.fill(bx, by, w, 14, Style::Normal, fill);
            u.c.text(&f, bx, by, w, 14, HAlign::Center, VAlign::Center, if live { white } else { red }, label);
        };
        match self.tab {
            Tab::Map => {
                let mut yy = top;
                for l in 0..63i64 {
                    let name = self.st.s("strlocationname", Some(l));
                    if self.st.b("locationunlocked", l) && !name.is_empty() {
                        if visible(yy, 14) {
                            let here = l == self.location();
                            if here {
                                u.c.fill(x + 4, yy, 192, 14, Style::Translucent, [60, 60, 90]);
                            }
                            if let Some(hf) = self.headers.clone() {
                                u.c.text(&hf, x + 6, yy, 188, 14, HAlign::Left, VAlign::Center, if here { white } else { [200, 200, 200] }, &name);
                            }
                        }
                        yy += 15;
                    }
                }
            }
            Tab::Weapons => {
                let mut yy = top;
                for id in 0..55i64 {
                    if !self.st.b("weapondiscovered", id) {
                        continue;
                    }
                    if visible(yy, 58) {
                        u.c.fill(x + 2, yy, 196, 58, Style::Translucent, [24, 24, 30]);
                        let icon = self.st.s("weapongraphics", Some(id));
                        if u.hud.tex(u.assets, &icon).is_some() {
                            crate::tnmpc::stretch(u, &icon, ((x + 4) as f32, (yy + 4) as f32, 36.0, 36.0), None, Style::Masked, white);
                        }
                        u.c.text(&f, x + 42, yy + 4, 66, f.height * 2, HAlign::Left, VAlign::Top, white, &self.st.s("weaponname", Some(id)));
                        let mods = |k: &str| self.st.i(k, Some(id));
                        let dmg = (mods("weaponmodsappliedaccuracy") + mods("weaponmodsappliedrange")) * 100;
                        let spd = (mods("weaponmodsappliedreload") + mods("weaponmodsappliedclip")) as f64 * self.reduction;
                        let l1 = fmtn(&self.str_of("clickerWindowWeaponsNodal", "strDamage").replace("%%", "%"), &[&dmg.to_string()]);
                        let l2 = fmtn(&self.str_of("clickerWindowWeaponsNodal", "strSpeed"), &[&format!("{spd:.4}")]);
                        u.c.text(&f, x + 42, yy + 22, 154, f.height, HAlign::Left, VAlign::Top, grey, &l1);
                        u.c.text(&f, x + 42, yy + 30, 154, f.height, HAlign::Left, VAlign::Top, grey, &l2);
                        let owned = self.st.i("weaponquantity", Some(id)) > 0;
                        let credits = self.st.i("credits", None);
                        let value = self.st.i("weaponvalue", Some(id));
                        let label = if owned { self.str_of("clickerWindowWeaponsNodal", "strBought") } else { format!("{}c", sep(value)) };
                        button(u, x + 108, yy + 4, 88, &label, !owned && credits >= value, false);
                        for k in 0..4usize {
                            let key = ["weaponmodsappliedaccuracy", "weaponmodsappliedrange", "weaponmodsappliedclip", "weaponmodsappliedreload"][k];
                            let n = self.st.i(key, Some(id));
                            let live = owned && n < self.max_mods && credits >= self.mod_cost(id, k);
                            let name = self.str_of("clickerWindowWeaponsNodal", ["strAccuracy", "strRange", "strClip", "strReload"][k]);
                            let short: String = name.split_whitespace().next().unwrap_or("").chars().take(5).collect();
                            button(u, x + 4 + k as i32 * 48, yy + 42, 46, &format!("{short} {n}"), live, k < 2);
                        }
                    }
                    yy += 60;
                }
            }
            Tab::Inventory => {
                let mut yy = top;
                let mut total = 0i64;
                for id in 0..13i64 {
                    if !self.st.b("itemdiscovered", id) {
                        continue;
                    }
                    let q = self.st.i("itemquantity", Some(id));
                    let v = self.st.i("itemvalue", Some(id));
                    total = total.saturating_add(q.saturating_mul(v));
                    if visible(yy, 46) {
                        u.c.fill(x + 2, yy, 196, 46, Style::Translucent, [24, 24, 30]);
                        let icon = self.st.s("itemgraphics", Some(id));
                        if u.hud.tex(u.assets, &icon).is_some() {
                            crate::tnmpc::stretch(u, &icon, ((x + 4) as f32, (yy + 2) as f32, 26.0, 26.0), None, Style::Masked, white);
                        }
                        let count = format!("{} {}", self.str_of("clickerWindowInventoryNodal", "strCount"), sep(q));
                        u.c.text(&f, x + 34, yy + 4, 160, f.height, HAlign::Left, VAlign::Top, white, &self.st.s("itemname", Some(id)));
                        u.c.text(&f, x + 34, yy + 14, 160, f.height, HAlign::Left, VAlign::Top, grey, &count);
                        let one = format!("{} {}c", self.str_of("clickerWindowInventoryNodal", "strSellOne"), sep(v));
                        let all = format!("{} {}c", self.str_of("clickerWindowInventoryNodal", "strSellAll"), sep(v.saturating_mul(q)));
                        button(u, x + 4, yy + 30, 92, &one, q > 0, false);
                        button(u, x + 100, yy + 30, 92, &all, q > 0, true);
                    }
                    yy += 48;
                }
                let label = fmtn(&self.str_of("clickerWindowInventory", "strSellAll"), &[&total.to_string()]);
                button(u, x + 4, yy + 4, 188, &label, total > 0, true);
            }
            Tab::Tools => {
                let mut yy = top;
                for id in 13..18i64 {
                    if !self.st.b("itemdiscovered", id) {
                        continue;
                    }
                    let q = self.item_count(id);
                    if visible(yy, 58) {
                        u.c.fill(x + 2, yy, 196, 58, Style::Translucent, [24, 24, 30]);
                        let icon = self.st.s("itemgraphics", Some(id));
                        if u.hud.tex(u.assets, &icon).is_some() {
                            crate::tnmpc::stretch(u, &icon, ((x + 4) as f32, (yy + 2) as f32, 26.0, 26.0), None, Style::Masked, white);
                        }
                        u.c.text(&f, x + 34, yy + 2, 160, f.height, HAlign::Left, VAlign::Top, white, &format!("{} ({})", self.st.s("itemname", Some(id)), sep(q)));
                        let desc = self.st.s("itemdescription", Some(id));
                        for (k, l) in crate::uicanvas::wrap(&f, &desc, 160).iter().take(3).enumerate() {
                            u.c.text(&f, x + 34, yy + 12 + k as i32 * f.height, 160, f.height, HAlign::Left, VAlign::Top, grey, l);
                        }
                        button(u, x + 4, yy + 42, 92, &self.str_of("clickerWindowToolsNodal", "strUseOne"), q > 0, false);
                        button(u, x + 100, yy + 42, 92, &self.str_of("clickerWindowToolsNodal", "strUseAll"), q > 0, true);
                    }
                    yy += 60;
                }
            }
            Tab::Quests => {
                let hf = self.headers.clone().unwrap_or(f.clone());
                let mut yy = top;
                for para in self.quest_text.split('\n') {
                    for l in crate::uicanvas::wrap(&hf, para, 188) {
                        if visible(yy, hf.height) {
                            u.c.text(&hf, x + 6, yy, 188, hf.height, HAlign::Left, VAlign::Top, white, &l);
                        }
                        yy += hf.height;
                    }
                    if para.is_empty() {
                        yy += 2;
                    }
                }
            }
            Tab::Stats => {
                // clickerWindowStats.Refresh
                let c = "clickerWindowStats";
                let rate = format!("{:.4}", self.rate);
                let lines = [
                    format!("{} {}", self.str_of(c, "strCredits"), sep(self.st.i("credits", None))),
                    fmtn(&self.str_of(c, "strEnemiesKilled"), &[&sep(self.st.i("enemieskilledlocation", Some(self.location())))]),
                    format!("{} {}", self.str_of(c, "strIdle"), sep(self.attack(false))),
                    format!("{} {}", self.str_of(c, "strManual"), sep(self.attack(true))),
                    fmtn(&self.str_of(c, "strRate"), &[&rate]),
                    fmtn(&self.str_of(c, "strEnergy"), &[&self.st.i("energy", None).to_string(), &self.st.i("maxenergy", None).to_string()]),
                    fmtn(&self.str_of(c, "strEnergyUse"), &["0"]),
                ];
                let hf = self.headers.clone().unwrap_or(f.clone());
                let mut yy = top + 4;
                for l in lines {
                    for w in crate::uicanvas::wrap(&hf, &l, 184) {
                        if visible(yy, hf.height) {
                            u.c.text(&hf, x + 8, yy, 184, hf.height, HAlign::Left, VAlign::Top, white, &w);
                        }
                        yy += hf.height;
                    }
                }
            }
            Tab::Log | Tab::Active => {
                // Newest at the bottom; the idle log takes lines without ';', the active one those with it.
                let active = self.tab == Tab::Active;
                let lines: Vec<String> = self
                    .st
                    .log
                    .iter()
                    .filter(|l| l.starts_with(';') == active)
                    .map(|l| l.trim_start_matches(';').to_string())
                    .collect();
                let mut yy = y + 384 + self.scroll[t];
                'up: for l in lines.iter().rev() {
                    let wrapped = crate::uicanvas::wrap(&f, l, 188);
                    for w in wrapped.iter().rev() {
                        yy -= f.height;
                        if yy < clip_top {
                            break 'up;
                        }
                        if yy + f.height <= y + 384 {
                            u.c.text(&f, x + 6, yy, 188, f.height, HAlign::Left, VAlign::Top, white, w);
                        }
                    }
                }
            }
        }
        // The note under the panel (a button's cost).
        if !self.hover_note.is_empty() {
            u.c.fill(x, y + 386, 200, 14, Style::Normal, [0, 0, 0]);
            u.c.text(&f, x + 4, y + 386, 192, 14, HAlign::Left, VAlign::Center, white, &self.hover_note);
        }
    }
}

#[derive(Clone, Copy)]
enum Act {
    Go(i64),
    Buy(i64),
    Mod(i64, usize),
    Sell(i64, bool),
    SellAll,
    Use(i64, bool),
}

impl Drop for Clicker {
    fn drop(&mut self) {
        // CloseScreen: SaveConfig.
        self.save();
    }
}

#[cfg(test)]
mod tests {

    /// A few minutes of play: the idle loop and clicks kill targets, loot comes in, credits buy a weapon.
    #[test]
    fn plays_a_while() {
        let Some(mut assets) = crate::tnmweb::tests::test_assets() else { return };
        let dir = std::env::temp_dir().join("dxv_clicker_test");
        unreal_pkg::paths::set_data_dir(dir.clone());
        let _ = std::fs::remove_file(save_path());
        let mut c = Clicker::new(&mut assets, 9);
        let keys = ButtonInput::<KeyCode>::default();
        let (tw, th) = c.target_size();
        let click = Pointer { pos: Some((300, 200)), down: true, pressed: true, wheel: 0.0 };
        let _ = (tw, th);
        for i in 0..2000 {
            let p = if i % 2 == 0 { click } else { Pointer::default() };
            c.tick(0.1, &keys, p);
        }
        let kills = c.st.i("enemieskilledlocation", Some(0));
        let credits = c.st.i("credits", None);
        println!("clicker: {kills} kills, {credits} credits, {} log lines", c.st.log.len());
        assert!(kills > 10 && !c.st.log.is_empty());
        c.st.seti("credits", None, 1000);
        c.st.seti("weapondiscovered", Some(0), 1);
        c.buy_weapon(0);
        assert_eq!(c.st.i("weaponquantity", Some(0)), 1);
        assert!(c.attack(true) > 5);
        drop(c);
        assert!(save_path().exists());
    }
    use super::*;

    #[test]
    fn separators() {
        assert_eq!(sep(5), "5");
        assert_eq!(sep(1000), "1,000");
        assert_eq!(sep(1234567), "1,234,567");
        assert_eq!(script_const("const MaxMods = 5;", "MaxMods"), Some(5.0));
    }

    /// The game's tables run from the mod's script: targets have sizes, the first kill rolls loot, a place change
    /// picks a target there. TNM_SYS=<folder with TNMGUI.u>.
    #[test]
    fn tables_from_the_mod() {
        let Ok(dir) = std::env::var("TNM_SYS") else { return };
        unreal_pkg::classes::set_extra_packages(unreal_pkg::tnm::PACKAGES);
        let mut db = unreal_pkg::classes::ClassDb::open(std::path::Path::new(&dir));
        let mut defs = HashMap::new();
        for class in ["clickerWindowQuests", "clickerWindow"] {
            for p in db.defaults(class).iter() {
                if p.index == 0 {
                    defs.insert(p.name.to_ascii_lowercase(), to_v(&p.value));
                }
                defs.insert(format!("{}[{}]", p.name.to_ascii_lowercase(), p.index), to_v(&p.value));
            }
        }
        let mut win = Script::default();
        win.objects = true;
        win.add_source(&db.script_text("clickerWindow").unwrap_or_default());
        let mut st = State { defs, vals: HashMap::new(), log: Vec::new(), rng: Rng(7), completes: Vec::new(), text: String::new(), music: None, unlocks_due: false };
        st.seti("location", None, 0);
        st.seti("currentenemytype", None, 0);
        let mut sizes = Vec::new();
        for e in 0..48 {
            st.seti("currentenemytype", None, e);
            let w = win.call(&mut ClickHost(&mut st), "gettargetsizewidth", &[]).int();
            sizes.push(w);
        }
        println!("target widths {sizes:?}");
        assert!(sizes.iter().filter(|w| **w > 0).count() > 30);
        let mut bgs = std::collections::HashSet::new();
        let mut diffs = Vec::new();
        for l in 0..47 {
            st.seti("location", None, l);
            bgs.insert(v_str(&win.call(&mut ClickHost(&mut st), "getbackground", &[])));
            diffs.push(win.call(&mut ClickHost(&mut st), "getlocationdifficulty", &[]).int());
        }
        println!("{} backgrounds, difficulties {diffs:?}", bgs.len());
        assert!(bgs.len() > 5 && diffs.iter().any(|d| *d > 10));
        st.seti("location", None, 2);
        let e = win.call(&mut ClickHost(&mut st), "getenemytype", &[]).int();
        assert!(e > 0, "enemy at the slums {e}");
        st.seti("queststarted", Some(1), 1);
        win.call(&mut ClickHost(&mut st), "updatelocationunlocks", &[]);
        assert_eq!(st.i("locationunlocked", Some(1)), 1);
    }
}
