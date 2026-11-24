//! The Nameless Mod's character look (tnmMenuScreenNewGame: face, eyewear, coat, vest, boots) and what Trestkon
//! wears in play. The choices are cycled by the screen's own rules (toggleVisor / toggleCoat / toggleVest /
//! toggleBoots: the stock looks first, then each one unlocked - the accessory boxes and coat bags hidden in the
//! maps set Trestkon's bUnlockedAccessory* flags, which the mod keeps in its ini and we keep in
//! TNMSave/accessories.txt). Which mesh and textures a look means is only written as code in the mod
//! (tnmMenuScreenNewGame.RefreshPlayerModel), so that function is run from the player's own TNMGUI.u as a data
//! loader (uscript.rs) and what it assigns to Player.Mesh / Player.MultiSkins is kept.
// uc: tnmMenuScreenNewGame.togglevisor, tnmMenuScreenNewGame.togglecoat, tnmMenuScreenNewGame.toggleVest,
// uc: tnmMenuScreenNewGame.toggleBoots, tnmMenuScreenNewGame.RefreshPlayerModel, ABIAccessoryBox.Frob, ABICoatBag.Frob

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use unreal_pkg::props::{Property, Value};

use crate::uscript::V;

/// A look: portraitIndexmain (0-5), eyewear stage (0 none, 1 visor, 2 glasses) and EyewearID, bhascoat and CoatID,
/// VestID, BootsID.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Look {
    pub face: u8,
    pub eye_stage: u8,
    pub eye_id: u8,
    pub coat: bool,
    pub coat_id: u8,
    pub vest_id: u8,
    pub boots_id: u8,
}

impl Look {
    /// PlayerSkin = portraitIndexmain + portraitIndexcoat + portraitIndexvisor: the portrait shown.
    pub fn portrait(&self) -> usize {
        self.face as usize + if self.coat { 18 } else { 0 } + self.eye_stage as usize * 6
    }
    /// EyeType / CoatType / VestType / BootType as the enums' indices.
    fn eye_type(&self) -> i64 {
        self.eye_id as i64
    }
    fn coat_type(&self) -> i64 {
        self.coat_id as i64
    }
    fn vest_type(&self) -> i64 {
        // toggleVest: 1 = VT_MJ12 (the enum's third), 2 = VT_JC (its second).
        [0, 2, 1][self.vest_id.min(2) as usize]
    }
    fn boot_type(&self) -> i64 {
        self.boots_id as i64
    }
}

struct State {
    /// The look in play, and the one being chosen on the new-game screen.
    current: Look,
    pending: Look,
    unlocked: Option<BTreeSet<String>>,
    /// (look, mesh, skins) worked out last.
    dressed: Option<(Look, String, [Option<String>; 8])>,
}

static STATE: Mutex<State> = Mutex::new(State {
    current: Look { face: 0, eye_stage: 0, eye_id: 0, coat: false, coat_id: 0, vest_id: 0, boots_id: 0 },
    pending: Look { face: 0, eye_stage: 0, eye_id: 0, coat: false, coat_id: 0, vest_id: 0, boots_id: 0 },
    unlocked: None,
    dressed: None,
});
static GEN: AtomicU64 = AtomicU64::new(1);

fn lock() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Changes whenever the look in play does (travel::jc_mesh's cache).
pub fn generation() -> u64 {
    GEN.load(Ordering::Relaxed)
}

fn file() -> std::path::PathBuf {
    unreal_pkg::paths::data_dir().join("TNMSave").join("accessories.txt")
}

fn unlocked(g: &mut State) -> &mut BTreeSet<String> {
    g.unlocked.get_or_insert_with(|| {
        std::fs::read_to_string(file()).map(|t| t.lines().map(|l| l.trim().to_ascii_lowercase()).filter(|l| !l.is_empty()).collect()).unwrap_or_default()
    })
}

fn has_flag(g: &mut State, flag: &str) -> bool {
    flag.is_empty() || unlocked(g).contains(&flag.to_ascii_lowercase())
}

fn flags(d: &[Property], name: &str, n: usize) -> Vec<String> {
    let mut v = vec![String::new(); n];
    for p in d.iter().filter(|p| p.name.eq_ignore_ascii_case(name)) {
        if let (Some(s), Value::Str(t)) = (v.get_mut(p.index as usize), &p.value) {
            *s = t.clone();
        }
    }
    v
}

/// The next id after `id` whose flag is unlocked (0 = back to the stock look).
fn next_id(g: &mut State, list: &[String], id: u8) -> u8 {
    let mut k = id as usize + 1;
    while k < list.len() {
        if has_flag(g, &list[k]) {
            return k as u8;
        }
        k += 1;
    }
    0
}

pub fn pending() -> Look {
    lock().pending
}

pub fn current() -> Look {
    lock().current
}

/// The new-game screen opens: its defaults.
pub fn reset_pending() {
    lock().pending = Look::default();
}

pub fn step_face(by: i32) {
    let mut g = lock();
    g.pending.face = (g.pending.face as i32 + by).rem_euclid(6) as u8;
}

/// The Eyewear button: none, the visor, the glasses, then each unlocked pair, then none again.
pub fn toggle_eyewear(d: &[Property]) {
    let list = flags(d, "requiredFlagEyewear", 4);
    let mut g = lock();
    let mut l = g.pending;
    if l.eye_stage < 2 && l.eye_id == 0 {
        l.eye_stage += 1;
    } else {
        let id = next_id(&mut g, &list, l.eye_id);
        l.eye_id = id;
        // 1 sunglasses and 3 aviators are glasses, 2 (the vaporwave visor) a visor; none left = bare eyes.
        l.eye_stage = match id {
            0 => 0,
            2 => 1,
            _ => 2,
        };
    }
    g.pending = l;
}

pub fn toggle_coat(d: &[Property]) {
    let list = flags(d, "requiredFlagCoat", 11);
    let mut g = lock();
    let mut l = g.pending;
    if !l.coat {
        l.coat = true;
        l.coat_id = 0;
    } else {
        l.coat_id = next_id(&mut g, &list, l.coat_id);
        if l.coat_id == 0 {
            l.coat = false;
        }
    }
    g.pending = l;
}

pub fn toggle_vest(d: &[Property]) {
    let list = flags(d, "requiredFlagVest", 3);
    let mut g = lock();
    let id = g.pending.vest_id;
    g.pending.vest_id = next_id(&mut g, &list, id);
}

pub fn toggle_boots(d: &[Property]) {
    let list = flags(d, "requiredFlagBoots", 10);
    let mut g = lock();
    let id = g.pending.boots_id;
    g.pending.boots_id = next_id(&mut g, &list, id);
}

/// A vest is on offer once either is unlocked (CreateSkinButtons).
pub fn vest_offered() -> bool {
    let mut g = lock();
    let set = unlocked(&mut g);
    set.contains("bunlockedaccessorymj12vest") || set.contains("bunlockedaccessoryjcvest")
}

/// START: the look chosen is the one in play.
pub fn commit() {
    let mut g = lock();
    g.current = g.pending;
    g.dressed = None;
    GEN.fetch_add(1, Ordering::Relaxed);
}

pub fn restore(l: Look) {
    let mut g = lock();
    g.current = l;
    g.dressed = None;
    GEN.fetch_add(1, Ordering::Relaxed);
}

/// An accessory box or coat bag was taken: (class, AccessoryType) -> the flag it unlocks; true the first time.
pub fn unlock_accessory(class: &str, kind: u8) -> bool {
    const COATS: [&str; 12] = [
        "bUnlockedAccessoryCoatPDX", "bUnlockedAccessoryCoatWC", "bUnlockedAccessoryCoatGoat", "bUnlockedAccessoryCoatLlama",
        "bUnlockedAccessoryCoatShortBlack", "bUnlockedAccessoryCoatShortTrest", "bUnlockedAccessoryCoatDXI", "bUnlockedAccessoryCoatGold",
        "bUnlockedAccessoryCoatGS", "bUnlockedAccessoryCoatBum", "bUnlockedAccessoryMJ12Vest", "bUnlockedAccessoryJCVest",
    ];
    const EYES: [&str; 3] = ["bUnlockedAccessoryBlackSunglasses", "bUnlockedAccessoryVaporwaveVisor", "bUnlockedAccessoryAviators"];
    let flag = if class.eq_ignore_ascii_case("ABICoatBag") { COATS.get(kind as usize) } else { EYES.get(kind as usize) };
    let Some(flag) = flag else { return false };
    let mut g = lock();
    let set = unlocked(&mut g);
    let new = set.insert(flag.to_ascii_lowercase());
    if new {
        let text: Vec<&str> = set.iter().map(|s| s.as_str()).collect();
        let _ = std::fs::create_dir_all(unreal_pkg::paths::data_dir().join("TNMSave"));
        let _ = std::fs::write(file(), text.join("\n"));
    }
    new
}

pub fn is_accessory(class: &str) -> bool {
    crate::tnmmode::active() && (class.eq_ignore_ascii_case("ABICoatBag") || class.eq_ignore_ascii_case("ABIAccessoryBox"))
}

struct Dresser<'a> {
    look: Look,
    enums: &'a HashMap<String, i64>,
    mesh: String,
    skins: [Option<String>; 8],
}

impl crate::uscript::Host for Dresser<'_> {
    fn call(&mut self, _name: &str, _args: &[V]) -> Option<V> {
        None
    }
    fn get(&mut self, name: &str) -> V {
        let l = self.look;
        match name {
            "portraitindexmain" => V::Int(l.face as i64),
            "portraitindexcoat" => V::Int(if l.coat { 18 } else { 0 }),
            "portraitindexvisor" => V::Int(l.eye_stage as i64 * 6),
            "bhasvisor" => V::Bool(l.eye_stage == 1),
            "bhasglasses" => V::Bool(l.eye_stage == 2),
            "coattype" => V::Int(l.coat_type()),
            "vesttype" => V::Int(l.vest_type()),
            "boottype" => V::Int(l.boot_type()),
            "eyetype" => V::Int(l.eye_type()),
            _ => self.enums.get(name).map_or(V::None, |v| V::Int(*v)),
        }
    }
    fn get_index(&mut self, _name: &str, _index: i64, _field: Option<&str>) -> V {
        V::None
    }
    fn set(&mut self, path: &str, v: &V) {
        if path == "player.mesh" {
            self.mesh = v.str();
        } else if let Some(i) = path.strip_prefix("player.multiskins[").and_then(|r| r.trim_end_matches(']').parse::<usize>().ok()) {
            if let Some(s) = self.skins.get_mut(i) {
                *s = if v.is_none() { None } else { Some(v.str()) };
            }
        }
    }
}

/// Every `enum E... { A, B, ... }` of a script: name (lower case) -> index.
fn enums_of(src: &str) -> HashMap<String, i64> {
    let mut out = HashMap::new();
    let lower = src.to_ascii_lowercase();
    let mut at = 0;
    while let Some(p) = lower[at..].find("enum ") {
        let start = at + p;
        let Some(open) = lower[start..].find('{') else { break };
        let Some(close) = lower[start + open..].find('}') else { break };
        let body = &lower[start + open + 1..start + open + close];
        for (i, n) in body.split(',').map(|n| n.trim()).filter(|n| !n.is_empty()).enumerate() {
            let n = n.split_whitespace().next().unwrap_or(n);
            out.entry(n.to_string()).or_insert(i as i64);
        }
        at = start + open + close;
    }
    out
}

fn compute(classes: &mut unreal_pkg::classes::ClassDb, look: Look) -> Option<(String, [Option<String>; 8])> {
    let src = classes.script_text("tnmMenuScreenNewGame")?;
    let enums = enums_of(&classes.script_text("Trestkon").unwrap_or_default());
    let mut script = crate::uscript::Script::default();
    script.fields_by_name = true;
    script.add_source(&src);
    let mut host = Dresser { look, enums: &enums, mesh: String::new(), skins: Default::default() };
    script.call(&mut host, "refreshplayermodel", &[]);
    if host.mesh.is_empty() {
        return None;
    }
    Some((host.mesh, host.skins))
}

/// The mesh and MultiSkins of a look (cached for the last one asked).
pub fn dress_look(classes: &mut unreal_pkg::classes::ClassDb, look: Look) -> Option<(String, [Option<String>; 8])> {
    if let Some((l, m, s)) = lock().dressed.as_ref() {
        if *l == look {
            return Some((m.clone(), s.clone()));
        }
    }
    let (m, s) = compute(classes, look)?;
    lock().dressed = Some((look, m.clone(), s.clone()));
    Some((m, s))
}

/// Trestkon's class defaults with the look in play put on (travel::jc_base_defaults).
pub fn dress(classes: &mut unreal_pkg::classes::ClassDb, d: &mut Vec<Property>) {
    let look = current();
    let Some((mesh, skins)) = dress_look(classes, look) else { return };
    d.retain(|p| !p.name.eq_ignore_ascii_case("Mesh") && !p.name.eq_ignore_ascii_case("MultiSkins"));
    d.push(Property { name: "Mesh".into(), index: 0, value: Value::Path(mesh) });
    for (i, s) in skins.into_iter().enumerate() {
        if let Some(s) = s {
            d.push(Property { name: "MultiSkins".into(), index: i as u32, value: Value::Path(s) });
        }
    }
}

/// The body mesh of the look in play.
pub fn mesh(classes: &mut unreal_pkg::classes::ClassDb) -> Option<String> {
    dress_look(classes, current()).map(|x| x.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_and_portraits() {
        let e = enums_of("var x;\nenum ECoatType\n{\n CT_Default,\n CT_PDX,\n CT_WC\n};\nenum EVestType { VT_Default, VT_JC, VT_MJ12 };");
        assert_eq!(e["ct_wc"], 2);
        assert_eq!(e["vt_mj12"], 2);
        let l = Look { face: 3, eye_stage: 2, coat: true, ..Default::default() };
        assert_eq!(l.portrait(), 3 + 12 + 18);
        assert_eq!(Look { vest_id: 1, ..Default::default() }.vest_type(), 2);
    }

    /// With the mod's packages at hand (TNM_SYS): every stock look resolves to a mesh and a face.
    #[test]
    fn looks_from_the_mod() {
        let Ok(sys) = std::env::var("TNM_SYS") else { return };
        unreal_pkg::classes::set_extra_packages(&["TNM", "TNMGUI"]);
        let mut classes = unreal_pkg::classes::ClassDb::open(std::path::Path::new(&sys));
        for coat in [false, true] {
            for face in 0..6u8 {
                let l = Look { face, coat, ..Default::default() };
                let (mesh, skins) = compute(&mut classes, l).expect("a look");
                println!("{l:?}: {mesh} {skins:?}");
                assert!(!mesh.is_empty());
                assert!(skins.iter().any(|s| s.is_some()));
            }
        }
    }
}
