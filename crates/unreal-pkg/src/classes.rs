//! Class default properties ("defaultproperties" blocks), resolved through the inheritance chain.
//!
//! A class export stores, at its very end, a tagged property list holding only the values that class
//! changes relative to its parent. The bytes before it (bytecode, state tables, dependency lists) are
//! variable-length and would need a full bytecode walker to skip, so instead we find the list by trying
//! every start offset and keeping the earliest one that decodes cleanly to the export's last byte using
//! only real property names. Placed actors in a map store only the values the designer changed, so
//! `ClassDb::actor_properties` overlays them on the class chain to get every effective value.

use crate::props::{self, Property};
use crate::{Export, Package, Reader};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Code packages loaded up front (their property names validate default lists).
pub const CODE_PACKAGES: &[&str] = &[
    "Core", "Engine", "Extension", "Fire", "ConSys", "DeusEx", "DeusExCharacters", "DeusExDeco",
    "DeusExItems", "DeusExUI", "UWindow", "IpDrv",
];

/// Mods' code packages, loaded after the game's when present (through a mod folder, paths.rs): the Lay D Denton
/// Project's FemJC (its NPCs and JC's female carcass).
pub const MOD_PACKAGES: &[&str] = &["FemJC"];

pub struct ClassDb {
    dir: PathBuf,
    pub packages: HashMap<String, Package>,
    property_names: HashSet<String>,
    cache: HashMap<String, Vec<Property>>, // key: "package.class" lowercase
    /// Declared properties (name -> tag type) of each class including its ancestors.
    decl_cache: HashMap<String, HashMap<String, u8>>,
    /// Localized strings from each package's `.int` file: package -> section (class, lowercase) -> (key, index, value).
    int_cache: HashMap<String, HashMap<String, Vec<(String, u32, String)>>>,
}

impl ClassDb {
    /// `system_dir` is the game's `System` folder.
    pub fn open(system_dir: &Path) -> ClassDb {
        let mut db = ClassDb { dir: system_dir.to_path_buf(), packages: HashMap::new(), property_names: HashSet::new(), cache: HashMap::new(), decl_cache: HashMap::new(), int_cache: HashMap::new() };
        for p in CODE_PACKAGES.iter().chain(MOD_PACKAGES) {
            db.load(p);
        }
        db
    }

    fn load(&mut self, name: &str) -> bool {
        let key = name.to_ascii_lowercase();
        if self.packages.contains_key(&key) {
            return true;
        }
        let Ok(pkg) = Package::open(self.dir.join(format!("{name}.u"))) else { return false };
        for e in &pkg.exports {
            if pkg.class_name(e.class_index).ends_with("Property") {
                self.property_names.insert(e.name.to_ascii_lowercase());
            }
        }
        self.packages.insert(key, pkg);
        true
    }

    /// The class's embedded UnrealScript source (its ScriptText TextBuffer).
    pub fn script_text(&mut self, class: &str) -> Option<String> {
        let key = format!("#src.{}", class.to_ascii_lowercase());
        if let Some(v) = self.int_cache.get(&key) {
            return v.get("").and_then(|l| l.first()).map(|x| x.2.clone());
        }
        let pkg_name = self.package_of(class)?;
        let pkg = self.packages.get(&pkg_name)?;
        let text = pkg.script_texts().into_iter().find(|t| t.owner.eq_ignore_ascii_case(class)).map(|t| t.text);
        let mut m = HashMap::new();
        if let Some(t) = &text {
            m.insert(String::new(), vec![(String::new(), 0, t.clone())]);
        }
        self.int_cache.insert(key, m);
        text
    }

    /// The full path of an object the class's package refers to by its bare name (Texture'Barrel1Tex1' in a
    /// script): an import or export of that name whose class is `kind`.
    pub fn resolve_ref(&self, class: &str, name: &str, kind: &str) -> Option<String> {
        let pkg = self.packages.get(&self.package_of(class)?)?;
        for (i, imp) in pkg.imports.iter().enumerate() {
            if imp.object_name.eq_ignore_ascii_case(name) && imp.class_name.eq_ignore_ascii_case(kind) {
                return Some(pkg.object_path(-(i as i32) - 1));
            }
        }
        None
    }

    /// Find which loaded package defines class `class` (search order: CODE_PACKAGES).
    pub fn package_of(&self, class: &str) -> Option<String> {
        for p in CODE_PACKAGES.iter().chain(MOD_PACKAGES) {
            let k = p.to_ascii_lowercase();
            if let Some(pkg) = self.packages.get(&k) {
                if pkg.exports.iter().any(|e| e.class_index == 0 && e.name.eq_ignore_ascii_case(class)) {
                    return Some(k);
                }
            }
        }
        None
    }

    /// The class and its ancestors, lowercase, nearest first (`["terroristcommander", "humanthug", ...,
    /// "actor", "object"]`) - for IsA / ClassIsChildOf. Empty for an unknown class.
    pub fn ancestry(&mut self, class: &str) -> Vec<String> {
        let mut out = Vec::new();
        let Some(mut pkg_name) = self.package_of(class) else { return out };
        let mut cls = class.to_string();
        for _ in 0..32 {
            out.push(cls.to_ascii_lowercase());
            let parent = {
                let Some(pkg) = self.packages.get(&pkg_name) else { break };
                let Some(e) = pkg.exports.iter().find(|e| e.class_index == 0 && e.name.eq_ignore_ascii_case(&cls)) else { break };
                super_class(pkg, e)
            };
            let Some((ppkg, pclass)) = parent else { break };
            if !self.load_by_name(&ppkg) {
                out.push(pclass.to_ascii_lowercase());
                break;
            }
            pkg_name = ppkg.to_ascii_lowercase();
            cls = pclass;
        }
        out
    }

    /// Effective defaults of `class` (searching the code packages), parents first then overrides.
    pub fn defaults(&mut self, class: &str) -> Vec<Property> {
        let Some(pkg_name) = self.package_of(class) else { return Vec::new() };
        self.defaults_in(&pkg_name, class)
    }

    fn defaults_in(&mut self, pkg_name: &str, class: &str) -> Vec<Property> {
        let key = format!("{pkg_name}.{}", class.to_ascii_lowercase());
        if let Some(v) = self.cache.get(&key) {
            return v.clone();
        }
        self.cache.insert(key.clone(), Vec::new()); // cycle guard
        let declared = self.declared(pkg_name, class);
        let (mut own, parent) = {
            let Some(pkg) = self.packages.get(pkg_name) else { return Vec::new() };
            let Some(e) = pkg.exports.iter().find(|e| e.class_index == 0 && e.name.eq_ignore_ascii_case(class)) else {
                return Vec::new();
            };
            let mut own = own_defaults_typed(pkg, e, &declared)
                .or_else(|| own_defaults(pkg, e, &self.property_names))
                .unwrap_or_default();
            resolve_objects(pkg, &mut own);
            let parent = super_class(pkg, e);
            (own, parent)
        };
        // Localized properties come from the package's .int file (the compiled defaults hold the English text
        // only for some of them).
        let localized: Vec<Property> = self
            .int_section(pkg_name, class)
            .into_iter()
            .filter(|(k, _, _)| declared.get(&k.to_ascii_lowercase()) == Some(&props::T_STR))
            .map(|(name, index, v)| Property { name, index, value: props::Value::Str(v) })
            .collect();
        overlay(&mut own, localized);
        let mut merged = match parent {
            Some((ppkg, pclass)) => {
                if self.load_by_name(&ppkg) {
                    self.defaults_in(&ppkg.to_ascii_lowercase(), &pclass)
                } else {
                    Vec::new()
                }
            }
            None => Vec::new(),
        };
        overlay(&mut merged, own);
        self.cache.insert(key, merged.clone());
        merged
    }

    /// The `[Class]` section of a package's `.int` localization file.
    fn int_section(&mut self, pkg_name: &str, class: &str) -> Vec<(String, u32, String)> {
        if !self.int_cache.contains_key(pkg_name) {
            let parsed = crate::paths::find_ci(&self.dir, &format!("{pkg_name}.int"))
                .and_then(|p| std::fs::read(p).ok())
                .map(|b| parse_int(&b))
                .unwrap_or_default();
            self.int_cache.insert(pkg_name.to_string(), parsed);
        }
        self.int_cache[pkg_name].get(&class.to_ascii_lowercase()).cloned().unwrap_or_default()
    }

    /// Every property declared by `class` and its ancestors, with its tag type.
    pub fn declared(&mut self, pkg_name: &str, class: &str) -> HashMap<String, u8> {
        let key = format!("{pkg_name}.{}", class.to_ascii_lowercase());
        if let Some(v) = self.decl_cache.get(&key) {
            return v.clone();
        }
        self.decl_cache.insert(key.clone(), HashMap::new()); // cycle guard
        let (mut own, parent) = {
            let Some(pkg) = self.packages.get(pkg_name) else { return HashMap::new() };
            let Some(ci) = pkg.exports.iter().position(|e| e.class_index == 0 && e.name.eq_ignore_ascii_case(class)) else {
                return HashMap::new();
            };
            let mut own = HashMap::new();
            for e in &pkg.exports {
                if e.package != ci as i32 + 1 {
                    continue;
                }
                let t = match pkg.class_name(e.class_index) {
                    "ByteProperty" => props::T_BYTE,
                    "IntProperty" => props::T_INT,
                    "BoolProperty" => props::T_BOOL,
                    "FloatProperty" => props::T_FLOAT,
                    "ObjectProperty" | "ClassProperty" => props::T_OBJECT,
                    "NameProperty" => props::T_NAME,
                    "StrProperty" | "StringProperty" => props::T_STR,
                    "StructProperty" => props::T_STRUCT,
                    "ArrayProperty" => props::T_ARRAY,
                    _ => continue,
                };
                own.insert(e.name.to_ascii_lowercase(), t);
            }
            (own, super_class(pkg, &pkg.exports[ci]))
        };
        if let Some((ppkg, pclass)) = parent {
            if self.load_by_name(&ppkg) {
                for (k, v) in self.declared(&ppkg.to_ascii_lowercase(), &pclass) {
                    own.entry(k).or_insert(v);
                }
            }
        }
        self.decl_cache.insert(key, own.clone());
        own
    }

    fn load_by_name(&mut self, name: &str) -> bool {
        self.load(name)
    }

    /// Class defaults overlaid with the properties stored on a placed actor.
    pub fn actor_properties(&mut self, map: &Package, e: &Export) -> Option<Vec<Property>> {
        let mut r = Reader::new(map.export_data(e), 0);
        if e.flags & props::RF_HAS_STACK != 0 {
            props::skip_state_frame(&mut r).ok()?;
        }
        let mut own = props::read_properties(map, &mut r).ok()?;
        resolve_objects(map, &mut own);
        let class = map.class_name(e.class_index).to_string();
        let mut merged = self.defaults(&class);
        overlay(&mut merged, own);
        Some(merged)
    }
}

/// Turn package-local object indices into full paths so lists from different packages can be merged.
pub fn resolve_objects(pkg: &Package, list: &mut [Property]) {
    let obj_path = |i: i32| -> String {
        if i == 0 {
            "None".to_string()
        } else if i > 0 {
            format!("{}.{}", pkg.self_name.clone().unwrap_or_default(), pkg.object_path(i))
        } else {
            pkg.object_path(i)
        }
    };
    for p in list.iter_mut() {
        // Structs holding object or name references must be resolved in their own package too.
        if let props::Value::Struct(name, bytes) = &p.value {
            let mut r = Reader::new(bytes, 0);
            match name.as_str() {
                "InventoryItem" | "InventoryItemCarcass" => {
                    if let (Ok(o), Ok(c)) = (r.compact_index(), r.i32()) {
                        p.value = props::Value::Item(obj_path(o), c);
                    }
                }
                "InitialAllianceInfo" => {
                    if let (Ok(n), Ok(level), Ok(b)) = (r.compact_index(), r.f32(), r.u8()) {
                        let name = pkg.names.get(n.max(0) as usize).cloned().unwrap_or_default();
                        p.value = props::Value::Alliance(name, level, b != 0);
                    }
                }
                _ => {}
            }
            continue;
        }
        if let props::Value::Object(i) = p.value {
            p.value = props::Value::Path(if i == 0 {
                "None".to_string()
            } else if i > 0 {
                format!("{}.{}", pkg.self_name.clone().unwrap_or_default(), pkg.object_path(i))
            } else {
                pkg.object_path(i)
            });
        }
    }
}

/// Replace or append each (name, index) of `top` into `base`.
pub fn overlay(base: &mut Vec<Property>, top: Vec<Property>) {
    for p in top {
        match base.iter_mut().find(|b| b.index == p.index && b.name.eq_ignore_ascii_case(&p.name)) {
            Some(b) => *b = p,
            None => base.push(p),
        }
    }
}

/// Parent class of a class export as (package, class name).
fn super_class(pkg: &Package, e: &Export) -> Option<(String, String)> {
    let s = e.super_index;
    if s == 0 {
        return None;
    }
    let path = pkg.object_path(s);
    if s > 0 {
        // Same package: path is just the class name (classes are top level).
        let own = pkg_self_name(pkg)?;
        return Some((own, path));
    }
    let (p, c) = path.split_once('.')?;
    Some((p.to_string(), c.to_string()))
}

/// The package's own name, recovered from any import that refers to it is not possible; instead the
/// caller always knows it. We look for the common case: a class in this package whose name is unique.
fn pkg_self_name(pkg: &Package) -> Option<String> {
    pkg.self_name.clone()
}

fn value_type(v: &props::Value) -> u8 {
    use props::Value::*;
    match v {
        Byte(_) => props::T_BYTE,
        Int(_) => props::T_INT,
        Bool(_) => props::T_BOOL,
        Float(_) => props::T_FLOAT,
        Object(_) | Path(_) => props::T_OBJECT,
        Name(_) => props::T_NAME,
        Str(_) => props::T_STR,
        Vector(_) | Rotator(_) | Struct(..) | Item(..) | Alliance(..) => props::T_STRUCT,
        Raw(t, _) => *t,
    }
}

/// Decode a class's defaults, accepting only properties the class hierarchy declares, with matching types.
pub fn own_defaults_typed(pkg: &Package, e: &Export, declared: &HashMap<String, u8>) -> Option<Vec<Property>> {
    if declared.is_empty() {
        return None;
    }
    let data = pkg.export_data(e);
    let n = data.len();
    for start in 0..n {
        let mut r = Reader::new(data, start);
        let Ok(list) = props::read_properties(pkg, &mut r) else { continue };
        if r.pos() != n {
            continue;
        }
        let ok = list.iter().all(|p| match declared.get(&p.name.to_ascii_lowercase()) {
            Some(&t) => {
                let vt = value_type(&p.value);
                vt == t || (t == props::T_STR && vt == props::T_STRING) || (t == props::T_OBJECT && vt == props::T_CLASS)
            }
            None => false,
        });
        if ok {
            return Some(list);
        }
    }
    None
}

/// Decode a class's own default property list (see module docs).
pub fn own_defaults(pkg: &Package, e: &Export, valid: &HashSet<String>) -> Option<Vec<Property>> {
    let data = pkg.export_data(e);
    let n = data.len();
    if n == 0 {
        return None;
    }
    for start in 0..n {
        let mut r = Reader::new(data, start);
        let Ok(list) = props::read_properties(pkg, &mut r) else { continue };
        if r.pos() != n {
            continue;
        }
        if list.iter().all(|p| valid.contains(&p.name.to_ascii_lowercase())) {
            return Some(list);
        }
    }
    None
}

/// Parse an Unreal `.int` file: `[Section]` headers and `Key="value"` / `Key[2]=value` lines (Latin-1 or UTF-16).
pub fn parse_int(bytes: &[u8]) -> HashMap<String, Vec<(String, u32, String)>> {
    let text: String = if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let u: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&u)
    } else {
        bytes.iter().map(|&b| b as char).collect()
    };
    let mut out: HashMap<String, Vec<(String, u32, String)>> = HashMap::new();
    let mut section = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_ascii_lowercase();
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        if section.is_empty() || k.is_empty() || k.starts_with(';') {
            continue;
        }
        let (name, index) = match k.split_once('[') {
            Some((n, rest)) => (n.trim().to_string(), rest.trim_end_matches(']').trim().parse().unwrap_or(0)),
            None => match k.split_once('(') {
                Some((n, rest)) => (n.trim().to_string(), rest.trim_end_matches(')').trim().parse().unwrap_or(0)),
                None => (k.trim().to_string(), 0),
            },
        };
        let v = v.trim();
        let v = v.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(v);
        out.entry(section.clone()).or_default().push((name, index, v.to_string()));
    }
    out
}

#[cfg(test)]
mod int_tests {
    #[test]
    fn parses_sections_arrays_and_quotes() {
        let m = super::parse_int(b"[AugSpeed]\r\nAugmentationName=\"Speed Enhancement\"\r\nAugLocsText[2]=Torso\r\n[Other]\nX=1");
        let a = &m["augspeed"];
        assert_eq!(a[0], ("AugmentationName".to_string(), 0, "Speed Enhancement".to_string()));
        assert_eq!(a[1], ("AugLocsText".to_string(), 2, "Torso".to_string()));
        assert_eq!(m["other"][0].2, "1");
    }
}
