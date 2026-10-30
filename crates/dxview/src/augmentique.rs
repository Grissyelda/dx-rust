//! Mod support: Augmentique (outfits), from the player's own Augmentique.u (launcher Mods page). A port of its
//! OutfitManager / Outfit / PartsGroup / OutfitPart / OutfitCustom / NPCOutfitGroup and WeaponSkinManager.
//!
//! The mod's outfit, NPC and weapon-skin LISTS exist only as UnrealScript calls (PopulateOutfitsList,
//! PopulateNPCOutfitsList, WeaponSkinManager.Init), so those functions are read from the player's package and run by
//! `uscript` as a data loader (approved for mods only, see CLAUDE.md); every call that builds something is answered
//! here, ported by hand: GlobalAddPart, AddPart, AddNewPartsMesh, GroupAddParts, GroupTranspose2,
//! OutfitAddPartReference, BeginNewPartsGroup, GetPartsGroup, BeginNewOutfit, AddDefaultReference,
//! BeginNPCOutfitGroup, AddNPCGroupException, AddNPCGroupClass, _NPCSkin, ADDNPCHologramOverride, AddSkin,
//! AddSkinOwnerClass, AddSkinTex, Add3rdSkinTex, AddProjectileSkin, AddProjectileSkinTex, AddSkinIcons.
//! Everything that happens in play is Rust: equipping (EquipOutfit / EquipCustomOutfit / ApplyOutfitToActor), the
//! custom outfit's part cycling, unlocking, NPC outfits (SetupNPCOutfits / AddMember / ApplyOutfits through
//! ClassDb's instance hook as a map is read), weapon skins, saving.
//!
//! Deviations: outfits are unlocked by picking up the OutfitSpawner boxes GMDX Augmented Edition places in its maps
//! (read from that mod's folder when it's set) - without GMDX's maps every outfit is unlocked from the start, as
//! there's nowhere to find them (setting AugAllOutfits forces that). NPC randomness is seeded per playthrough so a
//! reloaded map dresses people the same way.

use crate::uscript::{Host, Script, V};
use std::collections::HashMap;
use std::sync::RwLock;
use unreal_pkg::props::{find, Property, Value};

const PINK: &str = "DeusExItems.Skins.PinkMaskTex";

/// OutfitManagerBase.PartSlot.
const PART_SLOTS: &[&str] = &[
    "PS_Hack", "PS_Body_M", "PS_Body_F", "PS_Main", "PS_Glasses", "PS_Hat", "PS_Helmet", "PS_Mask", "PS_Trench", "PS_Trench_F",
    "PS_Trench_M", "PS_Trench_Shirt", "PS_Trench_Shirt_M", "PS_Trench_Shirt_F", "PS_Torso_M", "PS_Torso_M_B", "PS_Torso_F",
    "PS_Skirt", "PS_Legs", "PS_Legs_M", "PS_Legs_F", "PS_DressLegs", "PS_Ponytail",
];

#[derive(Clone, Debug)]
pub struct Part {
    pub id: String,
    pub name: String,
    /// Texture names as the script gives them ("skin", "default" -> resolved when worn): MultiSkins 0-7, then Texture.
    pub tex: [Option<String>; 9],
    pub slot: i64,
    pub accessory: bool,
}

#[derive(Clone, Debug)]
pub struct Group {
    /// (mesh, carcass mesh, menu name), full paths.
    pub meshes: Vec<(String, String, String)>,
    pub allow_male: bool,
    pub allow_female: bool,
    pub parts: Vec<Part>,
    pub defaults: Vec<String>,
}

impl Group {
    fn correct_gender(&self, female: bool) -> bool {
        (self.allow_male && !female) || (self.allow_female && female)
    }
    fn part(&self, id: &str) -> Option<usize> {
        self.parts.iter().position(|p| p.id == id)
    }
    fn add(&mut self, p: Part) {
        match self.parts.iter().position(|q| q.id == p.id) {
            Some(i) => self.parts[i] = p,
            None => self.parts.push(p),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Outfit {
    pub id: String,
    pub name: String,
    pub desc: String,
    pub pickup_name: String,
    pub pickup_message: String,
    pub article: String,
    pub group: usize,
    pub mesh_id: usize,
    /// Indices into the group's parts.
    pub parts: Vec<usize>,
}

#[derive(Clone, Debug, Default)]
struct NpcGroup {
    /// (class or bind name, unique, seed table indices per part list; None = random).
    classes: Vec<(String, bool, Vec<Option<usize>>)>,
    excepted: Vec<String>,
    /// Part lists by PartSlot: (skin part, textures).
    parts: Vec<Vec<(bool, [Option<String>; 9])>>,
    overrides: Vec<(usize, String)>,
}

#[derive(Clone, Debug, Default)]
pub struct WeaponSkin {
    pub id: String,
    pub weapon: String,
    pub name: String,
    pub tex: [Option<String>; 9],
    pub tex3: [Option<String>; 9],
    pub owners: Vec<String>,
    pub unlocked_default: bool,
}

#[derive(Default)]
pub struct Data {
    pub groups: Vec<Group>,
    /// Outfit 0 is the custom one (filled from the save), as in the original.
    pub outfits: Vec<Outfit>,
    npc: Vec<NpcGroup>,
    pub skins: Vec<WeaponSkin>,
    /// (id, textures) for projectiles fired by a skinned weapon.
    proj_skins: Vec<(String, [Option<String>; 9])>,
    /// Localized texts.
    pub texts: HashMap<String, String>,
    pub part_labels: Vec<String>,
}

/// What the player has (saved with the game).
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AugSave {
    /// The custom outfit's name (the outfit it was made from + " (Custom)").
    pub custom_name: String,
    pub unlocked: Vec<String>,
    /// The outfit on (savedOutfitIndex): 0 the custom outfit, None the default one.
    pub equipped: Option<usize>,
    pub custom_group: Option<usize>,
    pub custom_mesh: usize,
    pub custom_parts: Vec<String>,
    pub no_accessories: bool,
    pub new_outfits: Vec<String>,
    pub seed: u64,
    pub skins_unlocked: Vec<String>,
    /// Weapon class -> skin id ("default" for none).
    pub weapon_skins: Vec<(String, String)>,
}

static DATA: RwLock<Option<Data>> = RwLock::new(None);
static STATE: RwLock<Option<AugSave>> = RwLock::new(None);
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Every outfit unlocked (no GMDX maps to find them in, or the setting).
static ALL_UNLOCKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
/// Texture / mesh existence cache.
static EXISTS: RwLock<Option<HashMap<String, bool>>> = RwLock::new(None);
static CLASSDB_DIR: RwLock<Option<std::path::PathBuf>> = RwLock::new(None);

pub fn active() -> bool {
    unreal_pkg::mods::active("augmentique").is_some() && DATA.read().map(|d| d.is_some()).unwrap_or(false)
}

/// Bumped whenever what JC wears changes (mirror body, conversation puppet rebuild).
pub fn generation() -> u64 {
    GENERATION.load(std::sync::atomic::Ordering::Relaxed)
}

fn bump() {
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

fn with_db<R>(f: impl FnOnce(&unreal_pkg::classes::ClassDb) -> R) -> Option<R> {
    thread_local! {
        static DB: std::cell::RefCell<Option<unreal_pkg::classes::ClassDb>> = const { std::cell::RefCell::new(None) };
    }
    let dir = CLASSDB_DIR.read().ok()?.clone()?;
    DB.with(|db| {
        let mut b = db.borrow_mut();
        if b.is_none() {
            *b = Some(unreal_pkg::classes::ClassDb::open(&dir));
        }
        b.as_ref().map(f)
    })
}

/// Does an object of this class kind ("Texture", "Mesh") exist at "Pkg.[Group.]Name"?
fn exists(path: &str, kind: &str) -> bool {
    let key = format!("{kind}:{}", path.to_ascii_lowercase());
    if let Ok(g) = EXISTS.read() {
        if let Some(v) = g.as_ref().and_then(|m| m.get(&key)) {
            return *v;
        }
    }
    let v = path
        .split_once('.')
        .and_then(|(pk, rest)| with_db(|db| db.package(pk).is_some_and(|p| p.find_export_of_class(rest, kind).is_some())))
        .unwrap_or(false);
    if let Ok(mut g) = EXISTS.write() {
        g.get_or_insert_with(HashMap::new).insert(key, v);
    }
    v
}

/// OutfitManager.findMesh.
fn find_mesh(m: &str) -> Option<String> {
    if m.is_empty() {
        return None;
    }
    [format!("Augmentique.{m}"), m.to_string(), format!("DeusExCharacters.{m}"), format!("FemJC.{m}")]
        .into_iter()
        .find(|p| p.contains('.') && exists(p, "Mesh"))
}

/// OutfitManager.findNPCTexture.
fn find_npc_texture(t: &str) -> Option<String> {
    if t.is_empty() || t.eq_ignore_ascii_case("none") {
        return None;
    }
    [format!("DeusExCharacters.Skins.{t}"), format!("DeusExItems.Skins.{t}"), format!("Augmentique.{t}")]
        .into_iter()
        .find(|p| exists(p, "Texture"))
}

/// The player's face (MultiSkins[0]) and default skins.
fn player_defaults() -> [Option<String>; 9] {
    let mut out: [Option<String>; 9] = Default::default();
    let d = with_db_mut_defaults();
    for p in &d {
        if p.name.eq_ignore_ascii_case("MultiSkins") && p.index < 8 {
            if let Value::Path(t) = &p.value {
                out[p.index as usize] = Some(t.clone());
            }
        }
    }
    out
}

fn with_db_mut_defaults() -> Vec<Property> {
    thread_local! {
        static DB: std::cell::RefCell<Option<unreal_pkg::classes::ClassDb>> = const { std::cell::RefCell::new(None) };
    }
    let Some(dir) = CLASSDB_DIR.read().ok().and_then(|d| d.clone()) else { return Vec::new() };
    DB.with(|db| {
        let mut b = db.borrow_mut();
        if b.is_none() {
            *b = Some(unreal_pkg::classes::ClassDb::open(&dir));
        }
        crate::travel::jc_base_defaults(b.as_mut().unwrap())
    })
}

/// OutfitManager.findTexture for the player: "skin" is his face, "default" his own texture in that slot.
fn find_texture(t: &str, slot: usize, defaults: &[Option<String>; 9]) -> Option<String> {
    if t.eq_ignore_ascii_case("skin") {
        return defaults[0].clone();
    }
    if t.eq_ignore_ascii_case("default") {
        return defaults[slot.min(8)].clone();
    }
    if t.is_empty() || t.eq_ignore_ascii_case("none") {
        return None;
    }
    let k = crate::travel::skin();
    let mut cands = vec![format!("Augmentique.{t}_S{k}"), format!("Augmentique.{t}"), format!("{t}_S{k}"), t.to_string()];
    cands.push(format!("DeusExCharacters.Skins.{t}"));
    cands.push(format!("DeusExItems.Skins.{t}"));
    cands.into_iter().find(|p| p.contains('.') && exists(p, "Texture"))
}

fn female() -> bool {
    crate::travel::female()
}

// ---- Building the lists ----

struct Builder<'a> {
    d: Data,
    globals: Vec<Part>,
    cur_group: Option<usize>,
    cur_outfit: Option<usize>,
    cur_npc: Option<usize>,
    cur_skin: Option<usize>,
    /// Next index into the playthrough's random table (AddNPCGroupClass for unique NPCs).
    rand_i: usize,
    names: &'a [Property],
    skin_names: &'a [Property],
}

fn arg(a: &[V], i: usize) -> V {
    a.get(i).cloned().unwrap_or(V::None)
}

fn tex_args(a: &[V], from: usize) -> [Option<String>; 9] {
    let mut t: [Option<String>; 9] = Default::default();
    for (k, slot) in t.iter_mut().enumerate() {
        let s = arg(a, from + k).str();
        if !s.is_empty() {
            *slot = Some(s);
        }
    }
    t
}

fn str_at(list: &[Property], name: &str, i: i64) -> String {
    list.iter()
        .find(|p| p.name.eq_ignore_ascii_case(name) && p.index as i64 == i)
        .map(|p| match &p.value {
            Value::Str(s) => s.clone(),
            _ => String::new(),
        })
        .unwrap_or_default()
}

/// LocalizedOutfitInfo is six strings packed as fstrings: Name, Desc, HighlightName, PickupName, PickupMessage,
/// Article.
fn outfit_info(list: &[Property], i: i64) -> [String; 6] {
    let mut out: [String; 6] = Default::default();
    let Some(Value::Struct(_, bytes)) = list.iter().find(|p| p.name.eq_ignore_ascii_case("outfitInfos") && p.index as i64 == i).map(|p| &p.value)
    else {
        return out;
    };
    let mut r = unreal_pkg::Reader::new(bytes, 0);
    for s in out.iter_mut() {
        match r.fstring() {
            Ok(v) => *s = v,
            Err(_) => break,
        }
    }
    out
}

impl Host for Builder<'_> {
    fn call(&mut self, name: &str, a: &[V]) -> Option<V> {
        let part = |a: &[V]| Part {
            slot: arg(a, 0).int(),
            name: arg(a, 1).str(),
            accessory: arg(a, 2).truthy(),
            id: arg(a, 3).str(),
            tex: tex_args(a, 4),
        };
        match name {
            "debuglog" | "log" | "syncfromstoreddata" | "copyunlockstoconfig" | "refreshallweapons" | "rolltable" => Some(V::None),
            "globaladdpart" => {
                self.globals.push(part(a));
                Some(V::None)
            }
            "addpart" => {
                if let Some(g) = self.cur_group {
                    self.d.groups[g].add(part(a));
                }
                Some(V::None)
            }
            "addnewpartsmesh" => {
                if let (Some(g), Some(m)) = (self.cur_group, find_mesh(&arg(a, 0).str())) {
                    let cm = find_mesh(&arg(a, 1).str()).unwrap_or_default();
                    self.d.groups[g].meshes.push((m, cm, arg(a, 2).str()));
                }
                Some(V::None)
            }
            "groupaddparts" => {
                let slot = arg(a, 0).int();
                if let Some(g) = self.cur_group {
                    for p in self.globals.iter().filter(|p| p.slot == slot).cloned().collect::<Vec<_>>() {
                        self.d.groups[g].add(p);
                    }
                }
                Some(V::None)
            }
            "grouptranspose2" => {
                // PartsGroup.AddTransposePart: texture k of the global part goes to slot_k (slot0 -1 means 0).
                let (from, to) = (arg(a, 0).int(), arg(a, 1).int());
                let slots: Vec<i64> = (0..9).map(|k| arg(a, 2 + k).int()).collect();
                if let Some(g) = self.cur_group {
                    for p in self.globals.iter().filter(|p| p.slot == from).cloned().collect::<Vec<_>>() {
                        let mut t: [Option<String>; 9] = Default::default();
                        for (k, &s) in slots.iter().enumerate() {
                            let dst = if k == 0 && s == -1 { Some(0) } else if s > 0 { Some(s as usize) } else { None };
                            if let Some(d) = dst.filter(|d| *d < 9) {
                                t[d] = p.tex[k].clone();
                            }
                        }
                        self.d.groups[g].add(Part { tex: t, slot: to, ..p });
                    }
                }
                Some(V::None)
            }
            "outfitaddpartreference" => {
                let id = arg(a, 0).str();
                if let Some(o) = self.cur_outfit {
                    let g = self.d.outfits[o].group;
                    if let Some(pi) = self.d.groups[g].part(&id) {
                        if self.d.outfits[o].parts.len() < 20 {
                            self.d.outfits[o].parts.push(pi);
                        }
                    }
                }
                Some(V::None)
            }
            "beginnewpartsgroup" => {
                let Some(m) = find_mesh(&arg(a, 0).str()) else { return Some(V::None) };
                if self.get_parts_group(&m) {
                    return Some(V::None);
                }
                let cm = find_mesh(&arg(a, 1).str()).unwrap_or_default();
                let regular = self.d.texts.get("bodytyperegular").cloned().unwrap_or_default();
                self.d.groups.push(Group {
                    meshes: vec![(m, cm, regular)],
                    allow_male: arg(a, 2).truthy(),
                    allow_female: arg(a, 3).truthy(),
                    parts: Vec::new(),
                    defaults: Vec::new(),
                });
                self.cur_group = Some(self.d.groups.len() - 1);
                Some(V::None)
            }
            "getpartsgroup" => {
                let ok = find_mesh(&arg(a, 0).str()).is_some_and(|m| self.get_parts_group(&m));
                Some(V::Bool(ok))
            }
            "adddefaultreference" => {
                if let Some(g) = self.cur_group {
                    self.d.groups[g].defaults.push(arg(a, 0).str());
                }
                Some(V::None)
            }
            "beginnewoutfit" => {
                let Some(g) = self.cur_group else { return Some(V::None) };
                let name = arg(a, 1).str();
                let mut pickup_name = arg(a, 5).str();
                if pickup_name.is_empty() {
                    pickup_name = name.clone();
                }
                let mut article = arg(a, 7).str();
                if article.is_empty() {
                    article = "a".into();
                }
                if article == "-" {
                    article.clear();
                }
                let mut msg = arg(a, 6).str();
                if msg.is_empty() {
                    msg = self.d.texts.get("defaultpickupmessage").cloned().unwrap_or_default();
                }
                self.d.outfits.push(Outfit {
                    id: arg(a, 0).str(),
                    name,
                    desc: arg(a, 3).str(),
                    pickup_name,
                    pickup_message: msg,
                    article,
                    group: g,
                    mesh_id: arg(a, 2).int().max(0) as usize,
                    parts: Vec::new(),
                });
                let o = self.d.outfits.len() - 1;
                self.cur_outfit = Some(o);
                for r in self.d.groups[g].defaults.clone() {
                    if let Some(pi) = self.d.groups[g].part(&r) {
                        self.d.outfits[o].parts.push(pi);
                    }
                }
                Some(V::None)
            }
            "beginnpcoutfitgroup" => {
                self.d.npc.push(NpcGroup { parts: vec![Vec::new(); 30], ..Default::default() });
                self.cur_npc = Some(self.d.npc.len() - 1);
                Some(V::None)
            }
            "addnpcgroupexception" => {
                if let Some(n) = self.cur_npc {
                    self.d.npc[n].excepted.push(arg(a, 0).str());
                }
                Some(V::None)
            }
            "addnpcgroupclass" => {
                let Some(n) = self.cur_npc else { return Some(V::None) };
                let (class, unique, no_carcass) = (arg(a, 0).str(), arg(a, 1).truthy(), arg(a, 2).truthy());
                let seeds: Vec<Option<usize>> = (0..30)
                    .map(|_| {
                        if unique {
                            let k = self.rand_i;
                            self.rand_i = (self.rand_i + 1) % 1000;
                            Some(k)
                        } else {
                            None
                        }
                    })
                    .collect();
                // NPCOutfitGroup.AddClass also adds its carcass class.
                self.d.npc[n].classes.push((class.clone(), unique, seeds.clone()));
                if !no_carcass {
                    self.d.npc[n].classes.push((format!("{class}Carcass"), unique, seeds));
                }
                Some(V::None)
            }
            "_npcskin" => {
                let Some(n) = self.cur_npc else { return Some(V::None) };
                let slot = arg(a, 0).int().clamp(0, 29) as usize;
                let mut t: [Option<String>; 9] = Default::default();
                for (k, s) in t.iter_mut().enumerate() {
                    *s = find_npc_texture(&arg(a, 2 + k).str());
                }
                self.d.npc[n].parts[slot].push((arg(a, 1).truthy(), t));
                Some(V::None)
            }
            "addnpchologramoverride" => {
                let Some(n) = self.cur_npc else { return Some(V::None) };
                let slot = arg(a, 0).int();
                if !(0..=8).contains(&slot) {
                    return Some(V::None);
                }
                let mut tex = arg(a, 1).str();
                if tex.is_empty() {
                    tex = "PinkMaskTex".into();
                }
                if let Some(t) = find_texture(&tex, 0, &Default::default()) {
                    self.d.npc[n].overrides.push((slot as usize, t));
                }
                Some(V::None)
            }
            // WeaponSkinManager.
            "addskin" => {
                let id = format!("{}_{}", arg(a, 0).str(), arg(a, 1).str());
                self.d.skins.push(WeaponSkin {
                    id,
                    weapon: arg(a, 1).str(),
                    name: arg(a, 2).str(),
                    unlocked_default: arg(a, 3).truthy(),
                    ..Default::default()
                });
                self.cur_skin = Some(self.d.skins.len() - 1);
                Some(V::None)
            }
            "addskinownerclass" => {
                if let Some(s) = self.cur_skin {
                    self.d.skins[s].owners.push(arg(a, 0).str());
                }
                Some(V::None)
            }
            "addskintex" | "add3rdskintex" => {
                let (k, t) = (arg(a, 0).int(), arg(a, 1).str());
                if let (Some(s), true) = (self.cur_skin, (0..9).contains(&k)) {
                    let sk = &mut self.d.skins[s];
                    let arr = if name == "addskintex" { &mut sk.tex } else { &mut sk.tex3 };
                    arr[k as usize] = Some(t);
                }
                Some(V::None)
            }
            "addprojectileskin" => {
                self.d.proj_skins.push((format!("{}_{}", arg(a, 0).str(), arg(a, 1).str()), Default::default()));
                Some(V::None)
            }
            "addprojectileskintex" => {
                let (k, t) = (arg(a, 0).int(), arg(a, 1).str());
                if let (Some(p), true) = (self.d.proj_skins.last_mut(), (0..9).contains(&k)) {
                    p.1[k as usize] = Some(t);
                }
                Some(V::None)
            }
            "addskinicons" | "unlockskinbyid" => Some(V::None),
            _ => None,
        }
    }
    fn get(&mut self, name: &str) -> V {
        if let Some(i) = PART_SLOTS.iter().position(|s| s.eq_ignore_ascii_case(name)) {
            return V::Int(i as i64);
        }
        match name {
            "bissetup" | "binited" | "bdebugmode" => V::Bool(false),
            "currentnpcoutfitgroup" => V::Int(self.cur_npc.unwrap_or(0) as i64),
            _ => self.d.texts.get(name).map(|s| V::Str(s.clone())).unwrap_or(V::None),
        }
    }
    fn get_index(&mut self, name: &str, i: i64, field: Option<&str>) -> V {
        match (name, field) {
            ("partnames", None) => V::Str(str_at(self.names, "partNames", i)),
            ("weaponskinnames", None) => V::Str(str_at(self.skin_names, "WeaponSkinNames", i)),
            ("outfitinfos", Some(f)) => {
                let info = outfit_info(self.names, i);
                let k = ["name", "desc", "highlightname", "pickupname", "pickupmessage", "article"].iter().position(|x| *x == f).unwrap_or(0);
                V::Str(info[k].clone())
            }
            ("defaulttextures", None) => V::Str("default".into()),
            _ => V::None,
        }
    }
}

impl Builder<'_> {
    fn get_parts_group(&mut self, mesh: &str) -> bool {
        match self.d.groups.iter().position(|g| g.meshes.iter().any(|m| m.0.eq_ignore_ascii_case(mesh) || m.1.eq_ignore_ascii_case(mesh))) {
            Some(i) => {
                self.cur_group = Some(i);
                true
            }
            None => false,
        }
    }
}

/// Read the mod (after `mods::activate`).
pub fn init(root: &std::path::Path) {
    if unreal_pkg::mods::active("augmentique").is_none() {
        return;
    }
    let sys = root.join("System");
    if let Ok(mut d) = CLASSDB_DIR.write() {
        *d = Some(sys.clone());
    }
    let t0 = std::time::Instant::now();
    let mut db = unreal_pkg::classes::ClassDb::open(&sys);
    if std::env::var("DXVIEW_AUGLOG").is_ok() {
        println!("augmentique: ClassDb {:.1} s", t0.elapsed().as_secs_f32());
    }
    let Some(src) = db.script_text("OutfitManager") else {
        println!("augmentique: no OutfitManager in Augmentique.u");
        return;
    };
    let names = db.defaults("OutfitManager");
    let skin_names = db.defaults("WeaponSkinManager");
    let mut script = Script::default();
    script.add_source(&src);
    let mut b = Builder {
        d: Data::default(),
        globals: Vec::new(),
        cur_group: None,
        cur_outfit: None,
        cur_npc: None,
        cur_skin: None,
        rand_i: 0,
        names: &names,
        skin_names: &skin_names,
    };
    for k in [
        "NothingName",
        "BodyTypeRegular",
        "BodyTypeFat",
        "BodyTypeSkinny",
        "CustomOutfitName",
        "DefaultPickupMessage",
        "DefaultPickupMessage2",
        "MsgOutfitUnlocked",
    ] {
        b.d.texts.insert(k.to_ascii_lowercase(), str_at(&names, k, 0));
    }
    for k in ["msgDefault", "msgUnlocked"] {
        b.d.texts.insert(k.to_ascii_lowercase(), str_at(&skin_names, k, 0));
    }
    // Outfit 0: the custom outfit (its parts come from the save).
    b.d.outfits.push(Outfit {
        id: "custom".into(),
        name: "Custom".into(),
        desc: String::new(),
        pickup_name: String::new(),
        pickup_message: String::new(),
        article: String::new(),
        group: 0,
        mesh_id: 0,
        parts: Vec::new(),
    });
    script.call(&mut b, "PopulateOutfitsList", &[]);
    script.call(&mut b, "PopulateNPCOutfitsList", &[]);
    if let Some(ws) = db.script_text("WeaponSkinManager") {
        let mut s2 = Script::default();
        s2.add_source(&ws);
        s2.call(&mut b, "Init", &[V::None]);
    }
    // PersonaScreenOutfits.PartButtonNames.
    let screen = db.defaults("PersonaScreenOutfits");
    b.d.part_labels = (0..11).map(|i| str_at(&screen, "PartButtonNames", i)).collect();
    for k in ["OutfitsTitleText", "EquipButtonLabel", "EditButtonLabel", "ChangeCameraLabel", "SettingsButtonLabel", "ShowAccessoriesLabel", "ShowDescriptionsLabel", "NewLabel", "ModelVariantLabel"] {
        b.d.texts.insert(k.to_ascii_lowercase(), str_at(&screen, k, 0));
    }
    println!(
        "augmentique: {} parts groups, {} outfits, {} NPC outfit groups, {} weapon skins ({:.1} s)",
        b.d.groups.len(),
        b.d.outfits.len() - 1,
        b.d.npc.len(),
        b.d.skins.len(),
        t0.elapsed().as_secs_f32()
    );
    let data = b.d;
    if let Ok(mut d) = DATA.write() {
        *d = Some(data);
    }
    // Outfits found in GMDX Augmented's maps (else all are open).
    let gmdx_maps = unreal_pkg::mods::active("gmdx").and_then(|d| unreal_pkg::mods::find_rel(&d, "Maps"));
    // Without GMDX's maps: our own boxes in the original maps (setting OutfitBoxes), else all open.
    let own_boxes = gmdx_maps.is_none() && crate::settings::get("OutfitBoxes") > 0.5;
    ALL_UNLOCKED.store(
        (gmdx_maps.is_none() && !own_boxes) || crate::settings::get("AugAllOutfits") > 0.5,
        std::sync::atomic::Ordering::Relaxed,
    );
    unreal_pkg::classes::set_instance_hook(Some(Box::new(npc_hook)));
    if let Some(maps) = gmdx_maps {
        init_spawners(&sys, maps);
    } else if own_boxes {
        init_own_boxes(&mut db, root);
    }
    // Dev aids: DXVIEW_AUGLOG lists the outfits; DXVIEW_OUTFIT=<id> wears one from the start.
    if std::env::var("DXVIEW_AUGLOG").is_ok() {
        if let Ok(g) = DATA.read() {
            for (i, o) in g.as_ref().unwrap().outfits.iter().enumerate() {
                let grp = &g.as_ref().unwrap().groups[o.group];
                println!("AUG outfit {i} {} '{}' group {} {:?} parts {}", o.id, o.name, o.group, grp.meshes.first().map(|m| &m.0), o.parts.len());
            }
        }
    }
    if let Ok(i) = std::env::var("DXVIEW_OUTFIT") {
        state(|s| s.equipped = i.parse().ok());
    }
}

// ---- Outfit boxes (GMDX Augmented's OutfitSpawner / 2 / 3 placements) ----

struct Spawners {
    maps: std::path::PathBuf,
    /// Class -> its GMDX defaults (Mesh, DrawScale, collision, LookupTexture) on DeusExDecoration's.
    classes: HashMap<String, Vec<Property>>,
}

static SPAWNERS: RwLock<Option<Spawners>> = RwLock::new(None);

fn init_spawners(sys: &std::path::Path, maps: std::path::PathBuf) {
    let Some(code) = unreal_pkg::mods::active("gmdx").and_then(|d| unreal_pkg::mods::find_rel(&d, "System/DeusEx.u")) else { return };
    let mut gm = unreal_pkg::classes::ClassDb::open_replacing(sys, &[("DeusEx", code)]);
    let mut base = unreal_pkg::classes::ClassDb::open(sys);
    let deco = base.defaults("DeusExDecoration");
    let mut classes = HashMap::new();
    for c in ["OutfitSpawner", "OutfitSpawner2", "OutfitSpawner3"] {
        let d = gm.base_defaults(c);
        let mut props = deco.clone();
        for p in d.into_iter().filter(|p| {
            ["Mesh", "DrawScale", "CollisionRadius", "CollisionHeight", "LookupTexture", "Texture", "Skin", "Mass", "bInvincible", "bPushable"]
                .iter()
                .any(|n| p.name.eq_ignore_ascii_case(n))
        }) {
            props.retain(|q| !(q.name.eq_ignore_ascii_case(&p.name) && q.index == p.index));
            props.push(p);
        }
        classes.insert(c.to_ascii_lowercase(), props);
    }
    if let Ok(mut s) = SPAWNERS.write() {
        *s = Some(Spawners { maps, classes });
    }
    dx_level::actors::set_extra_actors(Some(Box::new(spawner_actors)));
}

/// The outfit boxes GMDX Augmented places in this map whose outfits aren't found yet (ValidateSpawn).
fn spawner_actors(map: &str) -> Vec<(String, String, Vec<Property>)> {
    let mut out = Vec::new();
    if ALL_UNLOCKED.load(std::sync::atomic::Ordering::Relaxed) {
        return out;
    }
    let Ok(g) = SPAWNERS.read() else { return out };
    let Some(sp) = g.as_ref() else { return out };
    let Some(file) = unreal_pkg::mods::find_rel(&sp.maps, &format!("{map}.dx")) else { return out };
    let Ok(pkg) = unreal_pkg::Package::open(&file) else { return out };
    for e in &pkg.exports {
        let class = pkg.class_name(e.class_index).to_string();
        let Some(defaults) = sp.classes.get(&class.to_ascii_lowercase()) else { continue };
        let mut r = unreal_pkg::Reader::new(pkg.export_data(e), 0);
        if e.flags & unreal_pkg::props::RF_HAS_STACK != 0 && unreal_pkg::props::skip_state_frame(&mut r).is_err() {
            continue;
        }
        let Ok(own) = unreal_pkg::props::read_properties(&pkg, &mut r) else { continue };
        let s = |n: &str| match find(&own, n) {
            Some(Value::Str(v)) => v.clone(),
            _ => String::new(),
        };
        let id = s("Id");
        // ValidateSpawn: a real outfit for this JC, not found yet. (requiredFlag isn't checked: kept, see the
        // module notes.)
        let (pickup_name, ok) = match DATA.read() {
            Ok(dg) => match dg.as_ref().and_then(|d| d.outfits.iter().skip(1).find(|o| o.id == id && d.groups[o.group].correct_gender(female()))) {
                Some(o) => (o.pickup_name.clone(), true),
                None => (String::new(), false),
            },
            Err(_) => (String::new(), false),
        };
        if !ok || is_unlocked(&id) {
            continue;
        }
        let mut props = defaults.clone();
        let mut set = |n: &str, v: Value| {
            props.retain(|p| !p.name.eq_ignore_ascii_case(n));
            props.push(Property { name: n.into(), index: 0, value: v });
        };
        for n in ["Location", "Rotation"] {
            if let Some(v) = find(&own, n) {
                set(n, v.clone());
            }
        }
        set("Id", Value::Str(id));
        set("ItemName", Value::Str(pickup_name));
        set("Tag", Value::Name(format!("AugSpawner_{}", e.name)));
        set("Physics", Value::Byte(0));
        // SetupOutfitSpawners: Skin and Texture from LookupTexture (Augmentique's box art).
        let look = match find(defaults, "LookupTexture") {
            Some(Value::Str(t)) => t.clone(),
            _ => String::new(),
        };
        if let Some(t) = find_texture(&look, 0, &Default::default()) {
            set("Skin", Value::Path(t.clone()));
            set("Texture", Value::Path(t));
        }
        if std::env::var("DXVIEW_AUGLOG").is_ok() {
            println!("AUG spawner {map} {} {class} at {:?}", e.name, find(&props, "Location"));
        }
        out.push((class, format!("AugSpawner_{}", e.name), props));
    }
    out
}

// ---- Our own outfit boxes in the original maps (Enhancement "Outfit Boxes In The Original Maps", ROADMAP v2 #36) ----
//
// GMDX Augmented hides Augmentique's outfits in boxes in its own versions of the maps. Without GMDX the outfits are
// all open from the start; with this setting they are spread over the campaign instead, one or two to a map, each in
// a box (GMDX's OutfitSpawner2 look: the dart box at x3 with Augmentique's BoxTex1) set down in front of a clothes
// rack, a couch, a chair or the like - the first map in `OWN_MAPS` that has one of those gets the outfits due there.

/// Maps that take boxes, in the order the campaign reaches them.
const OWN_MAPS: &[&str] = &[
    "01_NYC_UNATCOHQ", "02_NYC_Hotel", "02_NYC_Smug", "02_NYC_FreeClinic", "02_NYC_Bar", "03_NYC_AirfieldHeliBase",
    "03_NYC_UNATCOHQ", "04_NYC_Hotel", "04_NYC_Smug", "04_NYC_NSFHQ", "05_NYC_UNATCOHQ", "06_HongKong_WanChai_Market",
    "06_HongKong_WanChai_Street", "06_HongKong_WanChai_Underworld", "06_HongKong_VersaLife", "06_HongKong_TongBase",
    "06_HongKong_Storage", "08_NYC_Hotel", "08_NYC_Smug", "08_NYC_Bar", "09_NYC_Ship", "09_NYC_Dockyard",
    "10_Paris_Club", "10_Paris_Chateau", "10_Paris_Metro", "11_Paris_Everett", "11_Paris_Cathedral",
    "12_Vandenberg_Cmd", "14_Vandenberg_Sub", "14_OceanLab_Lab", "15_Area51_Bunker", "15_Area51_Final",
];

/// Things a box is set down in front of, best first.
const OWN_ANCHORS: &[&str] = &[
    "ClothesRack", "CouchLeather", "ChairLeather", "Couch", "Bed", "Toilet", "Toilet2", "OfficeChair", "Chair1",
    "HKChair", "Plant1", "Plant2", "Plant3", "BoxSmall", "CrateUnbreakableSmall", "TrashCan1",
];

struct OwnBoxes {
    maps_dir: std::path::PathBuf,
    /// The box's properties on top of DeusExDecoration's.
    props: Vec<Property>,
    /// Anchor class -> (collision radius, height) from its defaults.
    anchors: HashMap<String, (f64, f64)>,
}

static OWN: RwLock<Option<OwnBoxes>> = RwLock::new(None);

fn init_own_boxes(db: &mut unreal_pkg::classes::ClassDb, root: &std::path::Path) {
    let mut props = db.defaults("DeusExDecoration");
    let mut set = |n: &str, v: Value| {
        props.retain(|p| !p.name.eq_ignore_ascii_case(n));
        props.push(Property { name: n.into(), index: 0, value: v });
    };
    set("Mesh", Value::Path("DeusExItems.AmmoDart".into()));
    set("DrawScale", Value::Float(3.0));
    set("CollisionRadius", Value::Float(25.5));
    set("CollisionHeight", Value::Float(6.0));
    set("Mass", Value::Float(20.0));
    set("bPushable", Value::Bool(false));
    set("bInvincible", Value::Bool(true));
    if let Some(t) = find_texture("BoxTex1", 0, &Default::default()) {
        set("Skin", Value::Path(t.clone()));
        set("Texture", Value::Path(t));
    }
    let mut anchors = HashMap::new();
    for a in OWN_ANCHORS {
        let d = db.defaults(a);
        let f = |n: &str, dflt: f64| match find(&d, n) {
            Some(Value::Float(v)) => *v as f64,
            _ => dflt,
        };
        anchors.insert(a.to_ascii_lowercase(), (f("CollisionRadius", 20.0), f("CollisionHeight", 20.0)));
    }
    if let Ok(mut o) = OWN.write() {
        *o = Some(OwnBoxes { maps_dir: root.join("Maps"), props, anchors });
    }
    dx_level::actors::set_extra_actors(Some(Box::new(own_box_actors)));
}

fn hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

/// The outfits due in `map` (an even share of the campaign, in order) still to be found.
fn own_outfits_for(map: &str) -> Vec<(String, String)> {
    let Some(mi) = OWN_MAPS.iter().position(|m| m.eq_ignore_ascii_case(map)) else { return Vec::new() };
    let Ok(dg) = DATA.read() else { return Vec::new() };
    let Some(d) = dg.as_ref() else { return Vec::new() };
    let mut all: Vec<&Outfit> = d
        .outfits
        .iter()
        .skip(1)
        .filter(|o| o.id != "default" && !o.pickup_name.is_empty() && d.groups[o.group].correct_gender(female()))
        .collect();
    all.sort_by(|a, b| a.id.cmp(&b.id));
    all.dedup_by(|a, b| a.id == b.id);
    let n = all.len();
    all.into_iter()
        .enumerate()
        .filter(|(j, _)| j * OWN_MAPS.len() / n.max(1) == mi)
        .map(|(_, o)| (o.id.clone(), o.pickup_name.clone()))
        .filter(|(id, _)| !is_unlocked(id))
        .collect()
}

fn own_box_actors(map: &str) -> Vec<(String, String, Vec<Property>)> {
    let mut out = Vec::new();
    if ALL_UNLOCKED.load(std::sync::atomic::Ordering::Relaxed) {
        return out;
    }
    let due = own_outfits_for(map);
    if due.is_empty() {
        return out;
    }
    let Ok(g) = OWN.read() else { return out };
    let Some(own) = g.as_ref() else { return out };
    let Some(file) = unreal_pkg::paths::find_ci(&own.maps_dir, &format!("{map}.dx")) else { return out };
    let Ok(pkg) = unreal_pkg::Package::open(&file) else { return out };
    // The map's furniture: (tier, location, yaw, radius, height).
    let mut spots: Vec<(usize, [f32; 3], i32, f64, f64)> = Vec::new();
    for e in &pkg.exports {
        let class = pkg.class_name(e.class_index).to_ascii_lowercase();
        let Some(tier) = OWN_ANCHORS.iter().position(|a| a.eq_ignore_ascii_case(&class)) else { continue };
        let mut r = unreal_pkg::Reader::new(pkg.export_data(e), 0);
        if e.flags & unreal_pkg::props::RF_HAS_STACK != 0 && unreal_pkg::props::skip_state_frame(&mut r).is_err() {
            continue;
        }
        let Ok(own_p) = unreal_pkg::props::read_properties(&pkg, &mut r) else { continue };
        let Some(Value::Vector(loc)) = find(&own_p, "Location") else { continue };
        let yaw = match find(&own_p, "Rotation") {
            Some(Value::Rotator(r)) => r[1],
            _ => 0,
        };
        let (dr, dh) = own.anchors.get(&class).copied().unwrap_or((20.0, 20.0));
        let f = |n: &str, d: f64| match find(&own_p, n) {
            Some(Value::Float(v)) => *v as f64,
            _ => d,
        };
        spots.push((tier, *loc, yaw, f("CollisionRadius", dr), f("CollisionHeight", dh)));
    }
    if spots.is_empty() {
        return out;
    }
    spots.sort_by_key(|s| s.0);
    let best = spots[0].0;
    let mut pool: Vec<_> = spots.iter().filter(|s| s.0 <= best + 2).cloned().collect();
    for (id, pickup_name) in due {
        if pool.is_empty() {
            break;
        }
        let k = (hash(&id) % pool.len() as u64) as usize;
        let (_, loc, yaw, rad, height) = pool.remove(k);
        // In front of it (its yaw), on the floor (just above, and falling the rest).
        let a = yaw as f64 / 65536.0 * std::f64::consts::TAU;
        let fwd = [a.cos(), a.sin()];
        let d = rad + 30.0;
        let at = [
            loc[0] as f64 + fwd[0] * d,
            loc[1] as f64 + fwd[1] * d,
            loc[2] as f64 - height + 6.0 + 4.0,
        ];
        let mut props = own.props.clone();
        let mut set = |n: &str, v: Value| {
            props.retain(|p| !p.name.eq_ignore_ascii_case(n));
            props.push(Property { name: n.into(), index: 0, value: v });
        };
        set("Location", Value::Vector([at[0] as f32, at[1] as f32, at[2] as f32]));
        set("Rotation", Value::Rotator([0, yaw.wrapping_add(16384), 0]));
        set("Physics", Value::Byte(2));
        set("Id", Value::Str(id.clone()));
        set("ItemName", Value::Str(pickup_name));
        let tag = format!("AugBox_{id}");
        set("Tag", Value::Name(tag.clone()));
        if std::env::var("DXVIEW_AUGLOG").is_ok() {
            println!("AUG own box {map} {id} at {at:?}");
        }
        out.push(("OutfitSpawner2".to_string(), tag, props));
    }
    out
}

/// An outfit box (frob it to unlock the outfit).
pub fn is_spawner(class: &str) -> bool {
    class.to_ascii_lowercase().starts_with("outfitspawner")
}

/// OutfitManager.SpawnerPickup: the message, the box's pickup sound; the caller removes the box.
pub fn spawner_pickup(props: &[Property]) -> Option<String> {
    let id = match find(props, "Id") {
        Some(Value::Str(v)) => v.clone(),
        _ => return None,
    };
    let msg = unlock(&id)?;
    crate::sound::play_player("Augmentique.outfits.BoxPickup", 1.0, 0.0);
    Some(msg)
}

// ---- State ----

fn state<R>(f: impl FnOnce(&mut AugSave) -> R) -> R {
    let mut g = STATE.write().unwrap();
    let s = g.get_or_insert_with(|| AugSave { seed: fresh_seed(), ..Default::default() });
    f(s)
}

fn fresh_seed() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1) | 1
}

/// A new game: a fresh playthrough (outfits found stay found only through the global list).
pub fn new_game() {
    if let Ok(mut g) = STATE.write() {
        *g = Some(AugSave { seed: fresh_seed(), unlocked: global_unlocks(), ..Default::default() });
    }
    bump();
}

pub fn save() -> AugSave {
    state(|s| s.clone())
}

pub fn restore(s: AugSave) {
    if let Ok(mut g) = STATE.write() {
        *g = Some(s);
    }
    bump();
}

fn global_file() -> std::path::PathBuf {
    std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("augmentique.txt"))).unwrap_or_else(|| "augmentique.txt".into())
}

/// Outfits unlocked in finished playthroughs (DeusExPlayer.unlockedOutfits, globalconfig).
fn global_unlocks() -> Vec<String> {
    std::fs::read_to_string(global_file())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.strip_prefix("Outfit=").map(|s| s.trim().to_string()))
        .collect()
}

/// CopyOutfitsToPlayer: finishing the game (or training) makes this playthrough's finds permanent.
pub fn copy_to_global() {
    let mut all = global_unlocks();
    for u in state(|s| s.unlocked.clone()) {
        if !all.contains(&u) {
            all.push(u);
        }
    }
    let text: String = all.iter().map(|u| format!("Outfit={u}\n")).collect();
    let _ = std::fs::write(global_file(), text);
}

pub fn is_unlocked(id: &str) -> bool {
    id == "default" || ALL_UNLOCKED.load(std::sync::atomic::Ordering::Relaxed) || state(|s| s.unlocked.iter().any(|u| u == id))
}

/// OutfitManager.Unlock (an outfit box picked up): Some(message) when it's new.
pub fn unlock(id: &str) -> Option<String> {
    if is_unlocked(id) {
        return None;
    }
    state(|s| {
        s.unlocked.push(id.to_string());
        s.new_outfits.push(id.to_string());
    });
    let g = DATA.read().ok()?;
    let d = g.as_ref()?;
    let o = d.outfits.iter().find(|o| o.id == id && d.groups.get(o.group).is_some_and(|g| g.correct_gender(female())))?;
    let msg = if o.pickup_message.contains("%s") {
        if o.article.is_empty() {
            o.pickup_message.replacen("%s", &o.pickup_name, 1)
        } else {
            o.pickup_message.replacen("%s", &o.article, 1).replacen("%s", &o.pickup_name, 1)
        }
    } else {
        o.pickup_message.clone()
    };
    Some(msg)
}

// ---- Wearing ----

/// The outfit being worn, as (group, mesh id, parts) - None for no mod / HDTP's JC (HDTP wins, as in GMDX).
fn worn() -> Option<(usize, usize, Vec<usize>)> {
    let g = DATA.read().ok()?;
    let d = g.as_ref()?;
    if !crate::travel::female() && hdtp_jc() {
        return None;
    }
    let fem = female();
    let (eq, cg, cm, cp) = state(|s| (s.equipped, s.custom_group, s.custom_mesh, s.custom_parts.clone()));
    if eq == Some(0) {
        let gi = cg?;
        let grp = d.groups.get(gi)?;
        if !grp.correct_gender(fem) {
            return None;
        }
        let parts = cp.iter().filter_map(|id| grp.part(id)).collect();
        return Some((gi, cm, parts));
    }
    let o = match eq.and_then(|i| d.outfits.get(i)).filter(|o| d.groups[o.group].correct_gender(fem)) {
        Some(o) => o,
        None => d.outfits.iter().skip(1).find(|o| o.id == "default" && d.groups[o.group].correct_gender(fem))?,
    };
    Some((o.group, o.mesh_id, o.parts.clone()))
}

fn hdtp_jc() -> bool {
    unreal_pkg::classes::override_layer("hdtp").iter().any(|(c, n, ..)| c.eq_ignore_ascii_case("JCDentonMale") && n.eq_ignore_ascii_case("Mesh"))
}

/// Outfit.ApplyOutfitToActor: (mesh, carcass mesh, Texture, MultiSkins).
fn look(carcass: bool) -> Option<(String, String, String, [String; 8])> {
    let (gi, mesh_id, parts) = worn()?;
    let g = DATA.read().ok()?;
    let d = g.as_ref()?;
    let grp = &d.groups[gi];
    let (mesh, cmesh, _) = grp.meshes.get(mesh_id).or(grp.meshes.first())?.clone();
    let defaults = player_defaults();
    let no_acc = state(|s| s.no_accessories);
    let mut tex = PINK.to_string();
    let mut skins: [String; 8] = std::array::from_fn(|_| PINK.to_string());
    for &pi in &parts {
        let Some(p) = grp.parts.get(pi) else { continue };
        if p.accessory && no_acc {
            continue;
        }
        for (s, t) in p.tex.iter().enumerate() {
            let Some(t) = t.as_deref().and_then(|t| find_texture(t, s, &defaults)) else { continue };
            if t.eq_ignore_ascii_case(PINK) {
                continue;
            }
            if s == 8 {
                tex = t;
            } else {
                skins[s] = t;
            }
        }
    }
    let _ = carcass;
    Some((mesh, cmesh, tex, skins))
}

/// The outfit's mesh for JC (third person, reflections, conversations).
pub fn player_mesh() -> Option<String> {
    look(false).map(|l| l.0)
}

/// Put the worn outfit on a property list (JC's own; `carcass` for his body: the carcass meshes).
pub fn dress_player(d: &mut Vec<Property>, carcass: bool) {
    let Some((mesh, cmesh, tex, skins)) = look(carcass) else { return };
    let set = |d: &mut Vec<Property>, n: &str, i: u32, v: Value| {
        d.retain(|p| !(p.name.eq_ignore_ascii_case(n) && p.index == i));
        d.push(Property { name: n.into(), index: i, value: v });
    };
    if carcass {
        set(d, "Mesh", 0, Value::Path(cmesh.clone()));
        set(d, "Mesh2", 0, Value::Path(format!("{cmesh}B")));
        set(d, "Mesh3", 0, Value::Path(format!("{cmesh}C")));
    } else {
        set(d, "Mesh", 0, Value::Path(mesh));
    }
    set(d, "Texture", 0, Value::Path(tex));
    set(d, "Skin", 0, Value::Path("None".into()));
    for (i, t) in skins.into_iter().enumerate() {
        set(d, "MultiSkins", i as u32, Value::Path(t));
    }
}

/// The "augmentique" class layer: JC's double and his carcass (ApplyCurrentOutfit's AllActors part).
pub fn build_layer() {
    let mut list = Vec::new();
    if active() {
        for (class, carcass) in [("JCDouble", false), ("JCDentonMaleCarcass", true)] {
            let mut d = Vec::new();
            dress_player(&mut d, carcass);
            for p in d {
                list.push((class.to_string(), p.name, p.index, p.value));
            }
        }
    }
    unreal_pkg::classes::set_override_layer("augmentique", list);
}

// ---- The Outfits screen's view of things ----

/// One row of the outfits list: (outfit index, name, equipped, new).
pub fn list() -> Vec<(usize, String, bool, bool)> {
    let Ok(g) = DATA.read() else { return Vec::new() };
    let Some(d) = g.as_ref() else { return Vec::new() };
    let fem = female();
    let (eq, news) = state(|s| (s.equipped, s.new_outfits.clone()));
    let def = d.outfits.iter().position(|o| o.id == "default" && d.groups[o.group].correct_gender(fem));
    let eq = eq.filter(|&i| i == 0 || d.outfits.get(i).is_some_and(|o| d.groups[o.group].correct_gender(fem))).or(def);
    let mut rows = Vec::new();
    for (i, o) in d.outfits.iter().enumerate() {
        if i == 0 {
            if eq == Some(0) {
                rows.push((0, state(|s| s.custom_name.clone()), true, false));
            }
            continue;
        }
        if !d.groups[o.group].correct_gender(fem) || !is_unlocked(&o.id) {
            continue;
        }
        let new = news.contains(&o.id);
        rows.push((i, o.name.clone(), Some(i) == eq, new));
    }
    // Custom first, new next, default, then by name (PopulateOutfitsList's sort column).
    rows.sort_by_key(|r| {
        let o = &d.outfits[r.0];
        let k = if r.0 == 0 { 0 } else if r.3 { 1 } else if o.id == "default" { 2 } else { 3 };
        (k, r.1.to_ascii_lowercase())
    });
    rows
}

pub fn description(i: usize) -> String {
    DATA.read().ok().and_then(|g| g.as_ref().and_then(|d| d.outfits.get(i).map(|o| o.desc.clone()))).unwrap_or_default()
}

pub fn equipped_index() -> Option<usize> {
    list().iter().find(|r| r.2).map(|r| r.0)
}

/// EquipOutfit.
pub fn equip(i: usize) {
    let id = DATA.read().ok().and_then(|g| g.as_ref().and_then(|d| d.outfits.get(i).map(|o| o.id.clone())));
    let Some(id) = id else { return };
    state(|s| {
        s.new_outfits.retain(|n| *n != id);
        s.equipped = Some(i);
    });
    if let Ok(mut f) = FLAGS.lock() {
        f.push(format!("JCOutfits_Equipped_{id}"));
    }
    applied();
}

/// Flags for EquipOutfit (JCOutfits_Equipped_<id>), set by the mission code's flag owner.
pub static FLAGS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn applied() {
    build_layer();
    bump();
}

/// EquipCustomOutfit: the outfit on becomes the custom one (its parts copied), ready to edit.
pub fn equip_custom() {
    let Some((gi, mesh_id, parts)) = worn() else { return };
    let ids: Vec<String> = DATA
        .read()
        .ok()
        .and_then(|g| g.as_ref().map(|d| parts.iter().filter_map(|&p| d.groups[gi].parts.get(p).map(|p| p.id.clone())).collect()))
        .unwrap_or_default();
    let name = list().iter().find(|r| r.2).map(|r| r.1.clone()).unwrap_or_default();
    let custom = text("CustomOutfitName");
    state(|s| {
        if s.equipped == Some(0) {
            return;
        }
        s.custom_name = format!("{name} {custom}").trim().to_string();
        s.equipped = Some(0);
        s.custom_group = Some(gi);
        s.custom_mesh = mesh_id;
        s.custom_parts = ids;
    });
    applied();
}

/// The custom outfit's part buttons: (part slot, label, current part name) for slots with more than one unlocked
/// part (UpdateOutfitPartButton), in PersonaScreenOutfits' order.
pub fn custom_rows() -> Vec<(i64, String, String)> {
    let Ok(g) = DATA.read() else { return Vec::new() };
    let Some(d) = g.as_ref() else { return Vec::new() };
    let Some(gi) = state(|s| s.custom_group) else { return Vec::new() };
    let Some(grp) = d.groups.get(gi) else { return Vec::new() };
    let parts = state(|s| s.custom_parts.clone());
    // UpdateOutfitPartButtons: slot -> PartButtonNames index.
    const LABELS: [(i64, i64); 23] = [
        (0, -1), (1, 0), (2, 0), (3, 1), (4, 6), (5, 8), (6, 7), (7, 9), (8, 2), (9, 2), (10, 2), (11, 3), (12, 3), (13, 3), (14, 3),
        (15, 3), (16, 3), (17, 5), (18, 4), (19, 4), (20, 4), (21, 4), (22, 10),
    ];
    let mut out = Vec::new();
    for (slot, li) in LABELS {
        let count = grp.parts.iter().filter(|p| p.slot == slot).count();
        if count <= 1 {
            continue;
        }
        let cur = parts.iter().filter_map(|id| grp.parts.iter().find(|p| &p.id == id && p.slot == slot)).last();
        let label = if li < 0 { String::new() } else { d.part_labels.get(li as usize).cloned().unwrap_or_default() };
        out.push((slot, label, cur.map(|p| p.name.clone()).unwrap_or_default()));
    }
    out
}

/// Next / PreviousCustomOutfitSlot.
pub fn cycle_part(slot: i64, dir: i32) {
    equip_custom();
    let Ok(g) = DATA.read() else { return };
    let Some(d) = g.as_ref() else { return };
    let Some(gi) = state(|s| s.custom_group) else { return };
    let grp = &d.groups[gi];
    let mut parts = state(|s| s.custom_parts.clone());
    let Some(pos) = parts.iter().rposition(|id| grp.parts.iter().any(|p| &p.id == id && p.slot == slot)) else { return };
    let start = grp.parts.iter().position(|p| p.id == parts[pos] && p.slot == slot).unwrap_or(0);
    let n = grp.parts.len();
    let mut i = start;
    loop {
        i = if dir > 0 { (i + 1) % n } else { (i + n - 1) % n };
        if i == start || grp.parts[i].slot == slot {
            break;
        }
    }
    // Outfit.ReplacePart replaces every part of that slot.
    for id in parts.iter_mut() {
        if grp.parts.iter().any(|p| &p.id == id && p.slot == slot) {
            *id = grp.parts[i].id.clone();
        }
    }
    drop(g);
    state(|s| s.custom_parts = parts);
    applied();
}

/// Body-type variants of the custom outfit: (label, count).
pub fn variant() -> Option<(String, usize)> {
    let g = DATA.read().ok()?;
    let d = g.as_ref()?;
    let (eq, gi, m) = state(|s| (s.equipped, s.custom_group, s.custom_mesh));
    if eq != Some(0) {
        return None;
    }
    let grp = d.groups.get(gi?)?;
    (grp.meshes.len() > 1).then(|| (grp.meshes.get(m).map(|x| x.2.clone()).unwrap_or_default(), grp.meshes.len()))
}

pub fn cycle_variant(dir: i32) {
    let count = variant().map(|v| v.1).unwrap_or(0);
    if count < 2 {
        return;
    }
    state(|s| s.custom_mesh = ((s.custom_mesh as i32 + dir).rem_euclid(count as i32)) as usize);
    applied();
}

pub fn has_accessories() -> bool {
    let Some((gi, _, parts)) = worn() else { return false };
    DATA.read().ok().and_then(|g| g.as_ref().map(|d| parts.iter().any(|&p| d.groups[gi].parts.get(p).is_some_and(|p| p.accessory)))).unwrap_or(false)
}

pub fn no_accessories() -> bool {
    state(|s| s.no_accessories)
}

pub fn set_no_accessories(v: bool) {
    state(|s| s.no_accessories = v);
    applied();
}

pub fn text(key: &str) -> String {
    DATA.read().ok().and_then(|g| g.as_ref().and_then(|d| d.texts.get(&key.to_ascii_lowercase()).cloned())).unwrap_or_default()
}

// ---- NPC outfits ----

fn table(seed: u64, k: usize) -> i64 {
    // RollTable: Rand(101) per entry, from the playthrough's seed.
    let mut x = seed ^ (k as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    (x % 101) as i64
}

fn hash_str(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

/// ClassDb's instance hook: SetupNPCOutfits / NPCOutfitGroup.AddMember / ApplyOutfits for a pawn or carcass as the
/// map is read. Setting AugNPCs = iEquipNPCs (0 none, 1 everyone but unique characters, 2 everyone).
fn npc_hook(map: &str, class: &str, name: &str, props: &mut Vec<Property>) {
    let level = crate::settings::get("AugNPCs").round() as i32;
    if level <= 0 {
        return;
    }
    let Ok(g) = DATA.read() else { return };
    let Some(d) = g.as_ref() else { return };
    // HDTP people keep their own look (ScriptedPawn.IsHDTP).
    if let Some(Value::Path(m)) = find(props, "Mesh") {
        if m.to_ascii_lowercase().starts_with("hdtp") {
            return;
        }
    }
    let full = format!("DeusEx.{class}");
    let bind = match find(props, "BindName") {
        Some(Value::Str(s)) | Some(Value::Name(s)) => s.clone(),
        _ => String::new(),
    };
    let is_carcass = class.to_ascii_lowercase().ends_with("carcass");
    let seed = state(|s| s.seed);
    let mut textures: [Option<String>; 9] = Default::default();
    let mut unique = false;
    let mut any = false;
    for grp in &d.npc {
        let m = |c: &str| c.eq_ignore_ascii_case(&full) || c.eq_ignore_ascii_case(class) || (!is_carcass && !bind.is_empty() && c.eq_ignore_ascii_case(&bind));
        if grp.excepted.iter().any(|c| m(c)) {
            continue;
        }
        let Some(info) = grp.classes.iter().find(|(c, ..)| m(c)) else { continue };
        any = true;
        unique |= info.1;
        for (i, list) in grp.parts.iter().enumerate() {
            if list.is_empty() {
                continue;
            }
            let rando = match info.2.get(i).copied().flatten() {
                Some(k) => table(seed, k) as usize % list.len(),
                None => (hash_str(&format!("{seed}:{map}:{name}:{i}")) as usize) % list.len(),
            };
            let (skin_part, tex) = &list[rando];
            if info.1 && *skin_part {
                continue;
            }
            for (k, t) in tex.iter().enumerate() {
                if let Some(t) = t {
                    textures[k] = Some(t.clone());
                }
            }
        }
        // Holograms (STY_Translucent).
        if matches!(find(props, "Style"), Some(Value::Byte(3))) {
            for (slot, t) in &grp.overrides {
                textures[*slot] = Some(t.clone());
            }
        }
    }
    if !any || (unique && level < 2) {
        return;
    }
    if std::env::var("DXVIEW_AUGLOG").is_ok() {
        println!("AUG npc {map} {name} ({class}) {:?}", textures);
    }
    for (k, t) in textures.iter().enumerate() {
        let Some(t) = t else { continue };
        let (n, i) = if k == 8 { ("Texture", 0) } else { ("MultiSkins", k as u32) };
        props.retain(|p| !(p.name.eq_ignore_ascii_case(n) && p.index == i));
        props.push(Property { name: n.into(), index: i, value: Value::Path(t.clone()) });
    }
}

// ---- Weapon skins ----

fn skin_of(class: &str) -> String {
    state(|s| s.weapon_skins.iter().find(|(c, _)| c.eq_ignore_ascii_case(class)).map(|x| x.1.clone())).unwrap_or_else(|| "default".into())
}

fn skin_unlocked(d: &Data, i: usize) -> bool {
    d.skins[i].unlocked_default || state(|s| s.skins_unlocked.contains(&d.skins[i].id))
}

/// The skin's textures for a weapon (first person or third / pickup): (slot 0-7 or 8 = Texture, path). Only slots
/// the weapon leaves empty take them (ApplyWeaponSkin). HDTP models keep their own (IsHDTP).
fn skin_textures(class: &str, skin: &str, first: bool) -> Vec<(usize, String)> {
    let Ok(g) = DATA.read() else { return Vec::new() };
    let Some(d) = g.as_ref() else { return Vec::new() };
    let full = format!("DeusEx.{class}");
    let Some(s) = d.skins.iter().find(|s| s.id == skin && s.weapon.eq_ignore_ascii_case(&full)) else { return Vec::new() };
    let arr = if first { &s.tex } else { &s.tex3 };
    // UpdateWeaponSkinTextures copies slot 4 into the first-person Texture (8) - kept as in the mod.
    let mut out: Vec<(usize, String)> = arr.iter().enumerate().take(8).filter_map(|(k, t)| t.clone().map(|t| (k, t))).collect();
    let main = if first { arr[4].clone() } else { arr[8].clone() };
    if let Some(m) = main {
        out.push((8, m));
    }
    out.retain(|(_, t)| exists(t, "Texture"));
    out
}

/// Put a weapon skin on property lists (empty slots only).
pub fn apply_weapon_skin(props: &mut Vec<Property>, class: &str, skin: &str, first: bool) {
    if !active() || skin == "default" || skin.is_empty() {
        return;
    }
    if let Some(Value::Path(m)) = find(props, if first { "PlayerViewMesh" } else { "Mesh" }) {
        if m.to_ascii_lowercase().starts_with("hdtpitems.") {
            return;
        }
    }
    for (k, t) in skin_textures(class, skin, first) {
        if k == 8 {
            if !props.iter().any(|p| p.name.eq_ignore_ascii_case("Texture") && matches!(&p.value, Value::Path(x) if x != "None" && !x.starts_with("Engine.S_"))) {
                props.retain(|p| !p.name.eq_ignore_ascii_case("Texture"));
                props.push(Property { name: "Texture".into(), index: 0, value: Value::Path(t) });
            }
            continue;
        }
        let taken = props.iter().any(|p| p.name.eq_ignore_ascii_case("MultiSkins") && p.index == k as u32 && matches!(&p.value, Value::Path(x) if x != "None"));
        if !taken {
            props.push(Property { name: "MultiSkins".into(), index: k as u32, value: Value::Path(t.clone()) });
        }
        if k == 0 && !props.iter().any(|p| p.name.eq_ignore_ascii_case("Skin")) {
            props.push(Property { name: "Skin".into(), index: 0, value: Value::Path(t) });
        }
    }
}

/// The player's skin for a weapon class ("default" when none).
pub fn player_skin(class: &str) -> String {
    if !active() {
        return "default".into();
    }
    skin_of(class)
}

/// SetDefaultSkin: the skin an NPC's weapon has (its class is among the skin's owners).
pub fn npc_skin(npc_class: &str, weapon_class: &str) -> String {
    let Ok(g) = DATA.read() else { return "default".into() };
    let Some(d) = g.as_ref() else { return "default".into() };
    let (o1, o2) = (format!("DeusEx.{npc_class}"), format!("DeusEx.{npc_class}Carcass"));
    let w = format!("DeusEx.{weapon_class}");
    let mut out = "default".to_string();
    for s in &d.skins {
        if s.weapon.eq_ignore_ascii_case(&w) && s.owners.iter().any(|o| o.eq_ignore_ascii_case(&o1) || o.eq_ignore_ascii_case(&o2) || o.eq_ignore_ascii_case(npc_class)) {
            out = s.id.clone();
        }
    }
    out
}

/// GetSkinFromCarcass / UnlockSkin: a weapon taken from a body. Some(message) when its skin is new; with
/// bSwitchToNewSkins (on) the player's weapon of that class takes it.
pub fn take_weapon_from(npc_class: &str, weapon_class: &str, item_name: &str) -> Option<String> {
    let skin = npc_skin(npc_class, weapon_class);
    if skin == "default" {
        return None;
    }
    let (name, fresh) = {
        let g = DATA.read().ok()?;
        let d = g.as_ref()?;
        let i = d.skins.iter().position(|s| s.id == skin)?;
        (d.skins[i].name.clone(), !skin_unlocked(d, i))
    };
    if !fresh {
        return None;
    }
    state(|s| {
        s.skins_unlocked.push(skin.clone());
        s.weapon_skins.retain(|(c, _)| !c.eq_ignore_ascii_case(weapon_class));
        s.weapon_skins.push((weapon_class.to_string(), skin.clone()));
    });
    bump();
    let msg = text("msgUnlocked").replacen("%s", &name, 1);
    Some(if item_name.is_empty() { msg } else { format!("{msg} [{item_name}]") })
}

/// GetSkinCountFor (unlocked, plus the default).
pub fn skin_count(weapon_class: &str) -> usize {
    let Ok(g) = DATA.read() else { return 1 };
    let Some(d) = g.as_ref() else { return 1 };
    let w = format!("DeusEx.{weapon_class}");
    1 + (0..d.skins.len()).filter(|&i| d.skins[i].weapon.eq_ignore_ascii_case(&w) && skin_unlocked(d, i)).count()
}

/// GetSkinName.
pub fn skin_name(weapon_class: &str) -> String {
    let id = skin_of(weapon_class);
    DATA.read().ok().and_then(|g| g.as_ref().and_then(|d| d.skins.iter().find(|s| s.id == id).map(|s| s.name.clone()))).unwrap_or_else(|| text("msgDefault"))
}

/// SelectNextSkin / SelectPreviousSkin.
pub fn cycle_skin(weapon_class: &str, dir: i32) {
    let ids: Vec<String> = {
        let Ok(g) = DATA.read() else { return };
        let Some(d) = g.as_ref() else { return };
        let w = format!("DeusEx.{weapon_class}");
        let mut v = vec!["default".to_string()];
        v.extend((0..d.skins.len()).filter(|&i| d.skins[i].weapon.eq_ignore_ascii_case(&w) && skin_unlocked(d, i)).map(|i| d.skins[i].id.clone()));
        v
    };
    let cur = skin_of(weapon_class);
    let i = ids.iter().position(|x| *x == cur).unwrap_or(0) as i32;
    let next = ids[((i + dir).rem_euclid(ids.len() as i32)) as usize].clone();
    state(|s| {
        s.weapon_skins.retain(|(c, _)| !c.eq_ignore_ascii_case(weapon_class));
        s.weapon_skins.push((weapon_class.to_string(), next));
    });
    bump();
}

/// Projectile skin textures for a projectile fired from a skinned weapon (ApplyProjectileSkinFrom): (slot, path).
pub fn projectile_skin(weapon_class: &str) -> Vec<(usize, String)> {
    let skin = player_skin(weapon_class);
    if skin == "default" {
        return Vec::new();
    }
    let Ok(g) = DATA.read() else { return Vec::new() };
    let Some(d) = g.as_ref() else { return Vec::new() };
    d.proj_skins
        .iter()
        .find(|(id, _)| id.eq_ignore_ascii_case(&skin))
        .map(|(_, t)| t.iter().enumerate().filter_map(|(k, t)| t.clone().map(|t| (k, t))).filter(|(_, t)| exists(t, "Texture")).collect())
        .unwrap_or_default()
}

/// ApplyProjectileSkin: a projectile fired from the player's skinned weapon (empty slots, Skin, Texture).
pub fn apply_projectile_skin(props: &mut Vec<Property>, weapon_class: &str) {
    if !active() {
        return;
    }
    for (k, t) in projectile_skin(weapon_class) {
        let (n, i) = if k == 8 { ("Texture", 0) } else { ("MultiSkins", k as u32) };
        props.retain(|p| !(p.name.eq_ignore_ascii_case(n) && p.index == i));
        props.push(Property { name: n.into(), index: i, value: Value::Path(t.clone()) });
        if k == 0 {
            props.retain(|p| !p.name.eq_ignore_ascii_case("Skin"));
            props.push(Property { name: "Skin".into(), index: 0, value: Value::Path(t) });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transpose_slots() {
        let names = Vec::new();
        let mut b = Builder {
            d: Data::default(),
            globals: Vec::new(),
            cur_group: None,
            cur_outfit: None,
            cur_npc: None,
            cur_skin: None,
            rand_i: 0,
            names: &names,
            skin_names: &names,
        };
        b.d.groups.push(Group { meshes: vec![], allow_male: true, allow_female: false, parts: vec![], defaults: vec![] });
        b.cur_group = Some(0);
        b.call("globaladdpart", &[V::Int(8), V::Str("Coat".into()), V::Bool(false), V::Str("c_t".into()), V::Str("A".into()), V::Str("B".into())]);
        // GroupTranspose(PS_Trench, 1, 5): texture 0 -> slot 1, texture 1 -> slot 5.
        b.call("grouptranspose2", &[V::Int(8), V::Int(8), V::Int(1), V::Int(5)]);
        let p = &b.d.groups[0].parts[0];
        assert_eq!(p.tex[1].as_deref(), Some("A"));
        assert_eq!(p.tex[5].as_deref(), Some("B"));
        assert!(p.tex[0].is_none());
        // GroupTranspose2(PS_Mask, PS_Body_M, -1, 3): texture 0 -> slot 0, texture 1 -> slot 3.
        b.call("globaladdpart", &[V::Int(7), V::Str("Mask".into()), V::Bool(false), V::Str("m_b".into()), V::Str("X".into()), V::Str("Y".into())]);
        b.call("grouptranspose2", &[V::Int(7), V::Int(1), V::Int(-1), V::Int(3)]);
        let p = b.d.groups[0].parts.iter().find(|p| p.id == "m_b").unwrap();
        assert_eq!(p.slot, 1);
        assert_eq!(p.tex[0].as_deref(), Some("X"));
        assert_eq!(p.tex[3].as_deref(), Some("Y"));
    }
}
