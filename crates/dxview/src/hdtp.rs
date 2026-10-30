//! Mod support: HDTP (the HD Texture Project), from the player's own HDTP folder (launcher Mods page). What it
//! changes is read from its own DeusEx.u - never run in place of the game's - by comparing its class defaults with
//! the game's:
//!
//! * Items, decorations, weapons and effects ("HD Items And Objects"): new meshes and skins in the class defaults
//!   (Mesh, PlayerViewMesh, PickupViewMesh, ThirdPersonMesh, Skin, Texture, MultiSkins, DrawScale).
//! * People ("HD Characters"): ScriptedPawn / DeusExCarcass.UpdateHDTPsettings put HDTPMeshName (and a carcass's
//!   HDTPMesh2Name / HDTPMesh3Name) in place of the mesh, clear Texture and Skin, and set MultiSkins[i] for each
//!   non-empty HDTPMeshTex[i]; robots and animals change their Mesh in the defaults. JC, his double and carcass and
//!   Paul take the face (and hands) of the player's PlayerSkin (JCDentonMale / PaulDenton.SetSkin), as the original
//!   does with bHDTP_ALL = 1.
//! * Weapons in hand: each HDTP weapon's RenderOverlays sets MultiSkins for the first-person view (mod sections
//!   shown while fitted, the laser's dot while lasing, the hands texture by PlayerSkin, the muzzle-flash section),
//!   calls Super.RenderOverlays, then sets them again for the third-person / pickup mesh. That function's body is
//!   read from HDTP's script text at start-up and run here (`Prog`) for both views.
//!
//! Both switches (settings HDModels / HDCharacters) take effect when the next map loads.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use unreal_pkg::props::{find, Value};

const PINK: &str = "DeusExItems.Skins.PinkMaskTex";

/// What a RenderOverlays assignment puts in a MultiSkins slot.
#[derive(Clone, Debug, PartialEq)]
pub enum Skin {
    /// `none`: the mesh's own texture.
    Clear,
    Tex(String),
    /// GetWeaponHandTex(): the hands of the player's skin.
    Hand,
    /// muztex / GetMuzzleTex(): the muzzle flash (none between shots).
    Muzzle,
}

#[derive(Clone, Debug)]
enum Cond {
    Flag(String),
    IsA(String),
    Loaded,
    Not(Box<Cond>),
    And(Box<Cond>, Box<Cond>),
    Or(Box<Cond>, Box<Cond>),
    False,
}

#[derive(Clone, Debug)]
enum Stmt {
    Set(u32, Skin),
    If(Cond, Vec<Stmt>, Vec<Stmt>),
    Super,
}

/// A weapon's RenderOverlays, as far as MultiSkins go.
#[derive(Clone, Debug, Default)]
pub struct Prog(Vec<Stmt>);

/// The state RenderOverlays reads.
#[derive(Clone, Debug, Default)]
pub struct WeaponState {
    pub scope: bool,
    pub silencer: bool,
    pub laser: bool,
    pub lasing: bool,
    /// The loaded ammo's class and its ancestors, lowercase (for AmmoType.IsA).
    pub ammo: Vec<String>,
    /// Ammo in the gun (AmmoType.AmmoAmount > 0, ClipCount < ReloadCount).
    pub loaded: bool,
}

impl Prog {
    /// (first-person slots, world slots): the MultiSkins RenderOverlays leaves before Super.RenderOverlays and after.
    pub fn eval(&self, st: &WeaponState) -> (Vec<(u32, Skin)>, Vec<(u32, Skin)>) {
        let mut view = Vec::new();
        let mut world = Vec::new();
        let mut after = false;
        fn cond(c: &Cond, st: &WeaponState) -> bool {
            match c {
                Cond::Flag(f) => match f.as_str() {
                    "bhasscope" => st.scope,
                    "bhassilencer" => st.silencer,
                    "bhaslaser" => st.laser,
                    "blasing" => st.lasing,
                    _ => false,
                },
                Cond::IsA(c) => st.ammo.iter().any(|a| a == c),
                Cond::Loaded => st.loaded,
                Cond::Not(a) => !cond(a, st),
                Cond::And(a, b) => cond(a, st) && cond(b, st),
                Cond::Or(a, b) => cond(a, st) || cond(b, st),
                Cond::False => false,
            }
        }
        fn run(list: &[Stmt], st: &WeaponState, view: &mut Vec<(u32, Skin)>, world: &mut Vec<(u32, Skin)>, after: &mut bool) {
            for s in list {
                match s {
                    Stmt::Set(i, v) => {
                        let out = if *after { &mut *world } else { &mut *view };
                        out.retain(|(j, _)| j != i);
                        out.push((*i, v.clone()));
                    }
                    Stmt::If(c, a, b) => run(if cond(c, st) { a } else { b }, st, view, world, after),
                    Stmt::Super => *after = true,
                }
            }
        }
        run(&self.0, st, &mut view, &mut world, &mut after);
        // Slots the view set and the world part didn't put back keep the view's value on the actor (MultiSkins is
        // one array), except the hands and the muzzle flash, which every RenderOverlays clears again.
        for (i, v) in &view {
            if !world.iter().any(|(j, _)| j == i) && !matches!(v, Skin::Hand | Skin::Muzzle) {
                world.push((*i, v.clone()));
            }
        }
        (view, world)
    }
}

// ---- A small reader for the part of UnrealScript RenderOverlays uses ----

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Id(String),
    Num(i64),
    /// texture'Name' / 'Name': (prefix, name).
    Lit(String, String),
    P(&'static str),
}

fn tokenize(src: &str) -> Vec<Tok> {
    let b: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '/' && b.get(i + 1) == Some(&'/') {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && b.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == '*' && b[i + 1] == '/') {
                i += 1;
            }
            i += 2;
        } else if c.is_ascii_alphabetic() || c == '_' {
            let s = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == '_') {
                i += 1;
            }
            let id: String = b[s..i].iter().collect();
            if b.get(i) == Some(&'\'') {
                let s2 = i + 1;
                i = s2;
                while i < b.len() && b[i] != '\'' {
                    i += 1;
                }
                out.push(Tok::Lit(id.to_ascii_lowercase(), b[s2..i].iter().collect()));
                i += 1;
            } else {
                out.push(Tok::Id(id.to_ascii_lowercase()));
            }
        } else if c == '\'' {
            let s2 = i + 1;
            i = s2;
            while i < b.len() && b[i] != '\'' {
                i += 1;
            }
            out.push(Tok::Lit(String::new(), b[s2..i].iter().collect()));
            i += 1;
        } else if c.is_ascii_digit() {
            let s = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            out.push(Tok::Num(b[s..i].iter().collect::<String>().parse().unwrap_or(0)));
        } else {
            let two: String = b[i..(i + 2).min(b.len())].iter().collect();
            let p: &'static str = match two.as_str() {
                "&&" => "&&",
                "||" => "||",
                "!=" => "!=",
                "==" => "==",
                "<=" => "<=",
                ">=" => ">=",
                _ => "",
            };
            if !p.is_empty() {
                out.push(Tok::P(p));
                i += 2;
                continue;
            }
            let p: &'static str = match c {
                '(' => "(",
                ')' => ")",
                '{' => "{",
                '}' => "}",
                '[' => "[",
                ']' => "]",
                ';' => ";",
                '=' => "=",
                '!' => "!",
                '.' => ".",
                ',' => ",",
                '<' => "<",
                '>' => ">",
                _ => "?",
            };
            out.push(Tok::P(p));
            i += 1;
        }
    }
    out
}

struct Parser<'a> {
    t: &'a [Tok],
    i: usize,
    /// Texture literal -> full path.
    resolve: &'a dyn Fn(&str) -> String,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn is(&self, p: &str) -> bool {
        matches!(self.peek(), Some(Tok::P(q)) if *q == p)
    }
    fn is_id(&self, id: &str) -> bool {
        matches!(self.peek(), Some(Tok::Id(q)) if q == id)
    }
    fn skip_stmt(&mut self) {
        let mut depth = 0;
        while let Some(t) = self.peek() {
            match t {
                Tok::P("(") => depth += 1,
                Tok::P(")") => depth -= 1,
                Tok::P(";") if depth <= 0 => {
                    self.i += 1;
                    return;
                }
                Tok::P("}") if depth <= 0 => return,
                _ => {}
            }
            self.i += 1;
        }
    }
    fn block(&mut self) -> Vec<Stmt> {
        let mut out = Vec::new();
        while self.peek().is_some() && !self.is("}") {
            if let Some(s) = self.stmt() {
                out.push(s);
            }
        }
        out
    }
    fn body(&mut self) -> Vec<Stmt> {
        if self.is("{") {
            self.i += 1;
            let b = self.block();
            if self.is("}") {
                self.i += 1;
            }
            b
        } else {
            self.stmt().into_iter().collect()
        }
    }
    fn stmt(&mut self) -> Option<Stmt> {
        if self.is(";") {
            self.i += 1;
            return None;
        }
        if self.is("{") {
            let b = self.body();
            return Some(Stmt::If(Cond::Not(Box::new(Cond::False)), b, Vec::new()));
        }
        if self.is_id("if") {
            self.i += 1;
            let c = if self.is("(") {
                self.i += 1;
                let c = self.or();
                if self.is(")") {
                    self.i += 1;
                }
                c
            } else {
                Cond::False
            };
            let a = self.body();
            let b = if self.is_id("else") {
                self.i += 1;
                self.body()
            } else {
                Vec::new()
            };
            return Some(Stmt::If(c, a, b));
        }
        if self.is_id("super") {
            // super.RenderOverlays(canvas) / super(weapon).RenderOverlays(canvas)
            let s = self.i;
            self.skip_stmt();
            let toks = &self.t[s..self.i];
            if toks.iter().any(|t| matches!(t, Tok::Id(x) if x == "renderoverlays")) {
                return Some(Stmt::Super);
            }
            return None;
        }
        if self.is_id("multiskins") {
            let s = self.i;
            self.i += 1;
            if self.is("[") {
                self.i += 1;
                if let Some(Tok::Num(n)) = self.peek().cloned() {
                    self.i += 1;
                    if self.is("]") {
                        self.i += 1;
                        if self.is("=") {
                            self.i += 1;
                            let v = match self.peek().cloned() {
                                Some(Tok::Id(x)) if x == "none" => Some(Skin::Clear),
                                Some(Tok::Id(x)) if x == "getweaponhandtex" => Some(Skin::Hand),
                                Some(Tok::Id(x)) if x == "muztex" || x == "getmuzzletex" => Some(Skin::Muzzle),
                                Some(Tok::Lit(_, name)) => Some(Skin::Tex((self.resolve)(&name))),
                                _ => None,
                            };
                            self.skip_stmt();
                            return v.map(|v| Stmt::Set(n as u32, v));
                        }
                    }
                }
            }
            self.i = s;
        }
        self.skip_stmt();
        None
    }
    fn or(&mut self) -> Cond {
        let mut a = self.and();
        while self.is("||") {
            self.i += 1;
            let b = self.and();
            a = Cond::Or(Box::new(a), Box::new(b));
        }
        a
    }
    fn and(&mut self) -> Cond {
        let mut a = self.unary();
        while self.is("&&") {
            self.i += 1;
            let b = self.unary();
            a = Cond::And(Box::new(a), Box::new(b));
        }
        a
    }
    fn unary(&mut self) -> Cond {
        if self.is("!") {
            self.i += 1;
            return Cond::Not(Box::new(self.unary()));
        }
        if self.is("(") {
            self.i += 1;
            let c = self.or();
            if self.is(")") {
                self.i += 1;
            }
            return c;
        }
        // A comparison or call up to the next && / || / unmatched ).
        let s = self.i;
        let mut depth = 0;
        while let Some(t) = self.peek() {
            match t {
                Tok::P("(") => depth += 1,
                Tok::P(")") if depth == 0 => break,
                Tok::P(")") => depth -= 1,
                Tok::P("&&") | Tok::P("||") if depth == 0 => break,
                _ => {}
            }
            self.i += 1;
        }
        let toks = &self.t[s..self.i];
        let ids: Vec<&str> = toks.iter().filter_map(|t| if let Tok::Id(x) = t { Some(x.as_str()) } else { None }).collect();
        if ids.len() == 1 && toks.len() == 1 {
            return Cond::Flag(ids[0].to_string());
        }
        if ids.contains(&"isa") {
            if let Some(Tok::Lit(_, n)) = toks.iter().find(|t| matches!(t, Tok::Lit(..))) {
                return Cond::IsA(n.to_ascii_lowercase());
            }
        }
        // (AmmoType != None), (AmmoType.AmmoAmount > 0), (ClipCount < ReloadCount): the gun has ammo in it.
        if ids.iter().any(|x| *x == "ammotype" || *x == "clipcount" || *x == "ammoamount") {
            return Cond::Loaded;
        }
        Cond::False
    }
}

/// RenderOverlays of a class's script text, or None when it has none (or it doesn't touch MultiSkins).
pub fn parse_render_overlays(src: &str, resolve: &dyn Fn(&str) -> String) -> Option<Prog> {
    let t = tokenize(src);
    let start = t.windows(2).position(|w| w[0] == Tok::Id("function".into()) && w[1] == Tok::Id("renderoverlays".into()))?;
    let open = (start..t.len()).find(|&i| t[i] == Tok::P("{"))?;
    let mut p = Parser { t: &t, i: open + 1, resolve };
    let body = p.block();
    fn touches(s: &[Stmt]) -> bool {
        s.iter().any(|x| match x {
            Stmt::Set(..) => true,
            Stmt::If(_, a, b) => touches(a) || touches(b),
            Stmt::Super => false,
        })
    }
    touches(&body).then_some(Prog(body))
}

// ---- The mod's data, read once at start-up ----

#[derive(Default)]
struct Data {
    /// (class, property, index, value) for items / decorations / weapons / effects.
    models: Vec<(String, String, u32, Value)>,
    /// The same for people, robots and animals (and their carcasses).
    characters: Vec<(String, String, u32, Value)>,
    /// Weapon class (lowercase) -> its RenderOverlays.
    weapons: HashMap<String, Prog>,
    /// Weapon class (lowercase) -> its HDTP first-person mesh.
    view_mesh: HashMap<String, String>,
    /// Ammo class (lowercase) -> ancestry, for IsA.
    ammo: HashMap<String, Vec<String>>,
    /// Class (lowercase) -> default flags (bHasScope, bHasSilencer, bHasLaser) and ammo, for the world skins.
    weapon_defaults: HashMap<String, (bool, bool, bool, String)>,
}

static DATA: RwLock<Option<Data>> = RwLock::new(None);

pub fn active() -> bool {
    unreal_pkg::mods::active("hdtp").is_some()
}

fn models_on() -> bool {
    crate::settings::get("HDModels") > 0.5
}

fn characters_on() -> bool {
    // Helios's character textures replace HDTP's people (setting DdsCharacters).
    crate::settings::get("HDCharacters") > 0.5 && !dds_characters()
}

/// A DDS pack (Helios) is in use for the people.
pub fn dds_characters() -> bool {
    unreal_pkg::mods::active("helios").is_some() && crate::settings::get("DdsCharacters") > 0.5
}

/// Which packages the DDS packs replace, from the settings: people (DeusExCharacters), items and decorations
/// (DeusExItems, DeusExDeco, also used on the vanilla models HDTP doesn't replace), the rest of the world.
pub fn apply_dds_filter() {
    let on = |k: &str| crate::settings::get(k) > 0.5;
    let chars = ["deusexcharacters"];
    let items = ["deusexitems", "deusexdeco"];
    let mut deny: Vec<String> = Vec::new();
    if !on("DdsCharacters") {
        deny.extend(chars.iter().map(|s| s.to_string()));
    }
    if !on("DdsItems") {
        deny.extend(items.iter().map(|s| s.to_string()));
    }
    let allow = if on("DdsWorld") {
        None
    } else {
        Some(chars.iter().chain(items.iter()).map(|s| s.to_string()).collect())
    };
    unreal_pkg::paths::set_dds_filter(allow, deny);
    // Repainted faces: the closing lid samples skin, not the painted eye (dx_level::actors::LID_FIX).
    let helios_faces = on("DdsCharacters") && unreal_pkg::mods::active("helios").is_some();
    dx_level::actors::LID_FIX.store(helios_faces, std::sync::atomic::Ordering::Relaxed);
}

const VISUAL: &[&str] =
    &["Mesh", "PlayerViewMesh", "PickupViewMesh", "ThirdPersonMesh", "Skin", "Texture", "MultiSkins", "DrawScale", "Mesh2", "Mesh3"];

/// Read HDTP's DeusEx.u (after `mods::activate`).
pub fn init(root: &Path) {
    let Some(dir) = unreal_pkg::mods::active("hdtp") else { return };
    let Some(code) = unreal_pkg::mods::find_rel(&dir, "System/DeusEx.u") else {
        println!("dxview: HDTP folder has no System/DeusEx.u");
        return;
    };
    let t0 = std::time::Instant::now();
    let sys = root.join("System");
    let mut base = unreal_pkg::classes::ClassDb::open(&sys);
    let mut hd = unreal_pkg::classes::ClassDb::open_replacing(&sys, &[("DeusEx", PathBuf::from(&code))]);
    let mut data = Data::default();
    for class in hd.classes_in("DeusEx") {
        let anc = hd.ancestry(&class);
        let lc = class.to_ascii_lowercase();
        let is = |a: &str| anc.iter().any(|x| x == a);
        if is("ammo") {
            data.ammo.insert(lc.clone(), anc.clone());
        }
        let person = is("scriptedpawn") || is("deusexcarcass") || is("deusexplayer") || is("jcdouble");
        let b = base.base_defaults(&class);
        let d = hd.base_defaults(&class);
        let out = if person { &mut data.characters } else { &mut data.models };
        // UpdateHDTPsettings (pawns, carcasses, the player).
        let s = |n: &str| match find(&d, n) {
            Some(Value::Str(v)) if !v.is_empty() => Some(v.clone()),
            _ => None,
        };
        if let Some(mesh) = s("HDTPMeshName") {
            out.push((class.clone(), "Mesh".into(), 0, Value::Path(mesh)));
            if let Some(m2) = s("HDTPMesh2Name") {
                out.push((class.clone(), "Mesh2".into(), 0, Value::Path(m2)));
            }
            if let Some(m3) = s("HDTPMesh3Name") {
                out.push((class.clone(), "Mesh3".into(), 0, Value::Path(m3)));
            }
            if !is("deusexplayer") {
                out.push((class.clone(), "Texture".into(), 0, Value::Path("None".into())));
                out.push((class.clone(), "Skin".into(), 0, Value::Path("None".into())));
            }
            for p in d.iter().filter(|p| p.name.eq_ignore_ascii_case("HDTPMeshTex")) {
                if let Value::Str(t) = &p.value {
                    if !t.is_empty() {
                        out.push((class.clone(), "MultiSkins".into(), p.index, Value::Path(t.clone())));
                    }
                }
            }
        }
        // Default-property changes.
        for p in d.iter().filter(|p| VISUAL.iter().any(|v| v.eq_ignore_ascii_case(&p.name))) {
            let old = b.iter().find(|q| q.name.eq_ignore_ascii_case(&p.name) && q.index == p.index);
            if old.map(|o| &o.value) == Some(&p.value) {
                continue;
            }
            if out.iter().any(|(c, n, i, _)| c == &class && n.eq_ignore_ascii_case(&p.name) && *i == p.index) {
                continue;
            }
            out.push((class.clone(), p.name.clone(), p.index, p.value.clone()));
        }
        for q in b.iter().filter(|q| q.name.eq_ignore_ascii_case("MultiSkins") || q.name.eq_ignore_ascii_case("Skin")) {
            if !d.iter().any(|p| p.name.eq_ignore_ascii_case(&q.name) && p.index == q.index) {
                out.push((class.clone(), q.name.clone(), q.index, Value::Path("None".into())));
            }
        }
        // Weapons: RenderOverlays.
        if is("deusexweapon") {
            if let Some(Value::Path(m)) = find(&d, "PlayerViewMesh") {
                data.view_mesh.insert(lc.clone(), m.clone());
            }
            let flag = |n: &str| matches!(find(&d, n), Some(Value::Bool(true)));
            let ammo = match find(&d, "AmmoName") {
                Some(Value::Path(p)) => p.rsplit('.').next().unwrap_or("").to_ascii_lowercase(),
                _ => String::new(),
            };
            data.weapon_defaults.insert(lc.clone(), (flag("bHasScope"), flag("bHasSilencer"), flag("bHasLaser"), ammo));
            if let Some(src) = hd.script_text(&class) {
                let resolve = |name: &str| -> String {
                    if name.eq_ignore_ascii_case("pinkmasktex") {
                        return PINK.into();
                    }
                    if name.contains('.') {
                        return name.to_string();
                    }
                    hd.resolve_ref(&class, name, "Texture").unwrap_or_else(|| name.to_string())
                };
                if let Some(prog) = parse_render_overlays(&src, &resolve) {
                    data.weapons.insert(lc.clone(), prog);
                }
            }
        }
    }
    println!(
        "dxview: HDTP: {} model changes, {} character changes, {} weapons with sections ({:.1} s)",
        data.models.len(),
        data.characters.len(),
        data.weapons.len(),
        t0.elapsed().as_secs_f32()
    );
    if let Ok(mut w) = DATA.write() {
        *w = Some(data);
    }
}

fn ammo_state(data: &Data, ammo: &str) -> Vec<String> {
    let a = ammo.rsplit('.').next().unwrap_or(ammo).to_ascii_lowercase();
    data.ammo.get(&a).cloned().unwrap_or_else(|| vec![a])
}

/// HDTP's hands for the player's skin (DeusExWeapon.GetWeaponHandTex).
fn hand_tex() -> String {
    match crate::travel::skin() {
        1 => "HDTPItems.skins.weaponhandstexblack".into(),
        2 => "HDTPItems.skins.weaponhandstexlatino".into(),
        3 => "HDTPItems.skins.weaponhandstexginger".into(),
        4 => "HDTPItems.skins.weaponhandstexalbino".into(),
        _ => "DeusExItems.Skins.WeaponHandsTex".into(),
    }
}

/// The override layer ("hdtp") for this map: called at start-up and whenever the player's look is set.
pub fn build_layer() {
    let mut list = Vec::new();
    if let Ok(g) = DATA.read() {
        if let Some(data) = g.as_ref() {
            if models_on() {
                list.extend(data.models.iter().cloned());
                // Weapons' pickup / third-person sections as RenderOverlays leaves them, with the class's own mods.
                for (class, prog) in &data.weapons {
                    let Some((scope, silencer, laser, ammo)) = data.weapon_defaults.get(class) else { continue };
                    let st = WeaponState {
                        scope: *scope,
                        silencer: *silencer,
                        laser: *laser,
                        lasing: false,
                        ammo: ammo_state(data, ammo),
                        loaded: true,
                    };
                    let (_, world) = prog.eval(&st);
                    let cname = data
                        .models
                        .iter()
                        .find(|(c, ..)| c.eq_ignore_ascii_case(class))
                        .map(|(c, ..)| c.clone())
                        .unwrap_or_else(|| class.clone());
                    for (slot, v) in world {
                        let value = match v {
                            Skin::Tex(t) => t,
                            _ => "None".into(),
                        };
                        list.retain(|(c, n, i, _): &(String, String, u32, Value)| {
                            !(c.eq_ignore_ascii_case(&cname) && n == "MultiSkins" && *i == slot)
                        });
                        list.push((cname.clone(), "MultiSkins".into(), slot, Value::Path(value)));
                    }
                }
            }
            if characters_on() {
                let female = crate::travel::female();
                for (c, n, i, v) in &data.characters {
                    let lc = c.to_ascii_lowercase();
                    // HDTP has no female JC: Lay D Denton keeps hers.
                    if female && (lc.starts_with("jcdenton") || lc == "jcdouble") {
                        continue;
                    }
                    list.push((c.clone(), n.clone(), *i, v.clone()));
                }
                // JCDentonMale / JCDouble / JCDentonMaleCarcass / PaulDenton(Carcass).SetSkin.
                let k = crate::travel::skin().min(4);
                let set = |list: &mut Vec<(String, String, u32, Value)>, c: &str, i: u32, t: String| {
                    list.retain(|(cc, n, ii, _)| !(cc.eq_ignore_ascii_case(c) && n == "MultiSkins" && *ii == i));
                    list.push((c.to_string(), "MultiSkins".into(), i, Value::Path(t)));
                };
                if !female {
                    for c in ["JCDentonMale", "JCDouble", "JCDentonMaleCarcass"] {
                        set(&mut list, c, 0, format!("HDTPCharacters.Skins.HDTPJCFaceTex{k}"));
                        set(&mut list, c, 3, format!("HDTPCharacters.Skins.HDTPJCHandsTex{k}"));
                    }
                }
                let paul = match k {
                    0 => 0,
                    k => k + 1,
                };
                for c in ["PaulDenton", "PaulDentonCarcass"] {
                    set(&mut list, c, 0, format!("HDTPCharacters.Skins.HDTPPaulDentonTex{paul}"));
                    set(&mut list, c, 3, format!("HDTPCharacters.Skins.HDTPJCHandsTex{k}"));
                }
            }
        }
    }
    if let Ok(c) = std::env::var("DXVIEW_HDTPLOG") {
        for (cc, n, i, v) in list.iter().filter(|(cc, ..)| c == "*" || cc.eq_ignore_ascii_case(&c)) {
            println!("HDTP {cc}.{n}[{i}] = {v:?}");
        }
    }
    unreal_pkg::classes::set_override_layer("hdtp", list);
}

/// HDTP's first-person skins for a weapon in hand (None when HDTP isn't drawing it): (slot, texture or "None"),
/// and the muzzle-flash slot.
pub fn view_skins(class: &str, view_mesh: &str, st: &WeaponState) -> Option<(Vec<(u32, String)>, Option<u32>)> {
    let g = DATA.read().ok()?;
    let data = g.as_ref()?;
    let lc = class.to_ascii_lowercase();
    if !models_on() || !data.view_mesh.get(&lc).is_some_and(|m| m.eq_ignore_ascii_case(view_mesh)) {
        return None;
    }
    let mut st = st.clone();
    if let Some(a) = st.ammo.first().cloned() {
        st.ammo = ammo_state(data, &a);
    }
    let (view, _) = data.weapons.get(&lc).map(|p| p.eval(&st)).unwrap_or_default();
    let mut muzzle = None;
    let mut out = Vec::new();
    for (slot, v) in view {
        let t = match v {
            Skin::Clear => "None".to_string(),
            Skin::Tex(t) => t,
            Skin::Hand => hand_tex(),
            Skin::Muzzle => {
                muzzle = Some(slot);
                PINK.to_string()
            }
        };
        out.push((slot, t));
    }
    Some((out, muzzle))
}

/// HDTP's third-person skins for a weapon with these mods (JC's reflection holds his own gun).
pub fn world_skins(class: &str, st: &WeaponState) -> Option<Vec<(u32, String)>> {
    let g = DATA.read().ok()?;
    let data = g.as_ref()?;
    if !models_on() {
        return None;
    }
    let (_, world) = data.weapons.get(&class.to_ascii_lowercase())?.eval(st);
    Some(
        world
            .into_iter()
            .map(|(i, v)| {
                (
                    i,
                    match v {
                        Skin::Tex(t) => t,
                        _ => "None".into(),
                    },
                )
            })
            .collect(),
    )
}

/// The weapon classes HDTP draws (Visible Attachments leaves these alone: HDTP has its own mod sections).
pub fn weapon_classes() -> Vec<String> {
    if !models_on() {
        return Vec::new();
    }
    DATA.read().ok().and_then(|g| g.as_ref().map(|d| d.view_mesh.keys().cloned().collect())).unwrap_or_default()
}

/// HDTP's muzzle flashes (DeusExWeapon.GetMuzzleTex): eight large for automatic weapons, three small otherwise.
pub fn muzzle_textures(automatic: bool) -> Vec<String> {
    if automatic {
        (1..=8).map(|i| format!("HDTPItems.Skins.HDTPMuzzleflashlarge{i}")).collect()
    } else {
        (1..=3).map(|i| format!("HDTPItems.Skins.HDTPMuzzleflashSmall{i}")).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pistol_overlays() {
        let src = "simulated function renderoverlays(Canvas canvas)\n{\n   if(bHasSilencer)\n\t  multiskins[6] = none;\n   else\n\t  multiskins[6] = texture'pinkmasktex';\n\tmultiskins[2] = Getweaponhandtex();\n\tmultiskins[3] = muztex;\n   super.renderoverlays(canvas);\n\tmultiskins[2] = none;\n}\n";
        let p = parse_render_overlays(src, &|n| if n == "pinkmasktex" { PINK.into() } else { n.into() }).unwrap();
        let (v, w) = p.eval(&WeaponState { silencer: false, ..Default::default() });
        assert!(v.contains(&(6, Skin::Tex(PINK.into()))));
        assert!(v.contains(&(2, Skin::Hand)));
        assert!(v.contains(&(3, Skin::Muzzle)));
        assert!(w.contains(&(2, Skin::Clear)));
        assert!(w.contains(&(6, Skin::Tex(PINK.into()))));
        let (v, _) = p.eval(&WeaponState { silencer: true, ..Default::default() });
        assert!(v.contains(&(6, Skin::Clear)));
    }

    #[test]
    fn gep_ammo() {
        let src = "function RenderOverlays(Canvas canvas) { if(ammotype.isA('AmmoRocketWP')) { multiskins[4] = texture'pinkmasktex'; multiskins[5] = none; } else if(ammotype.isA('AmmoRocket')) { multiskins[4] = none; multiskins[5] = texture'pinkmasktex'; } super.renderoverlays(canvas); }";
        let p = parse_render_overlays(src, &|n| n.into()).unwrap();
        let wp = WeaponState { ammo: vec!["ammorocketwp".into(), "ammorocket".into()], ..Default::default() };
        let (v, _) = p.eval(&wp);
        assert!(v.contains(&(5, Skin::Clear)));
        let r = WeaponState { ammo: vec!["ammorocket".into()], ..Default::default() };
        let (v, _) = p.eval(&r);
        assert!(v.contains(&(4, Skin::Clear)));
    }

    #[test]
    fn crossbow_loaded() {
        let src = "function renderoverlays(canvas c) { if ((AmmoType != None) && (AmmoType.AmmoAmount > 0) && (ClipCount < ReloadCount)) { Multiskins[2] = texture'A'; } else Multiskins[2] = texture'pinkmasktex'; }";
        let p = parse_render_overlays(src, &|n| n.into()).unwrap();
        assert!(p.eval(&WeaponState { loaded: true, ..Default::default() }).0.contains(&(2, Skin::Tex("A".into()))));
        assert!(p.eval(&WeaponState::default()).0.contains(&(2, Skin::Tex("pinkmasktex".into()))));
    }
}
