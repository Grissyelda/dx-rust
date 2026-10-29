//! Reader for Unreal Engine 1 packages (.u, .utx, .uax, .umx, .unr and Deus Ex .dx maps).
//!
//! Verified against Deus Ex 1112fm: every package is version 68, licensee 0.
//! This reads the table structure and the embedded UnrealScript source text.
//! Property and object decoding live in the `props` module.

pub mod classes;
pub mod paths;
pub mod mods;
pub mod gamedir;
pub mod mesh;
pub mod objects;
pub mod props;

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

pub const PACKAGE_TAG: u32 = 0x9E2A_83C1;

#[derive(Debug)]
pub enum PkgError {
    Io(std::io::Error),
    BadTag(u32),
    Eof(usize),
    Unsupported(String),
}

impl fmt::Display for PkgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PkgError::Io(e) => write!(f, "io error: {e}"),
            PkgError::BadTag(t) => write!(f, "not an Unreal package (tag {t:#010x})"),
            PkgError::Eof(p) => write!(f, "unexpected end of data at offset {p}"),
            PkgError::Unsupported(s) => write!(f, "unsupported: {s}"),
        }
    }
}

impl std::error::Error for PkgError {}

impl From<std::io::Error> for PkgError {
    fn from(e: std::io::Error) -> Self {
        PkgError::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, PkgError>;

/// Little-endian cursor over a byte slice.
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8], pos: usize) -> Self {
        Self { data, pos }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn seek(&mut self, pos: usize) {
        self.pos = pos;
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(PkgError::Eof(self.pos))?;
        if end > self.data.len() {
            return Err(PkgError::Eof(self.pos));
        }
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }

    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }

    /// Unreal "compact index": sign bit 0x80, continue bit 0x40 on the first byte
    /// (6 data bits), then continue bit 0x80 and 7 data bits on later bytes.
    pub fn compact_index(&mut self) -> Result<i32> {
        let b = self.u8()? as i32;
        let neg = b & 0x80 != 0;
        let mut v = b & 0x3f;
        if b & 0x40 != 0 {
            let mut shift = 6;
            loop {
                let b = self.u8()? as i32;
                v |= (b & 0x7f).wrapping_shl(shift);
                shift += 7;
                if b & 0x80 == 0 {
                    break;
                }
                if shift > 34 {
                    return Err(PkgError::Unsupported("compact index too long".into()));
                }
            }
        }
        Ok(if neg { v.wrapping_neg() } else { v })
    }

    /// Length-prefixed (compact index) NUL-terminated Latin-1 string, as used by version 68.
    pub fn fstring(&mut self) -> Result<String> {
        let n = self.compact_index()?;
        if n < 0 {
            // A negative length marks a "Unicode" string: 2 x |n| bytes. Deus Ex's long conversation comments
            // carry it but hold single-byte text (UTF-16LE when every other byte is zero).
            let bytes = self.take(n.unsigned_abs() as usize * 2)?;
            let utf16 = bytes.chunks_exact(2).all(|c| c[1] == 0);
            let s: String = if utf16 {
                bytes.chunks_exact(2).map(|c| c[0] as char).collect()
            } else {
                bytes.iter().map(|&b| b as char).collect()
            };
            // The text ends at its NUL; the rest of the buffer is garbage the editor saved with it.
            return Ok(s.split('\0').next().unwrap_or("").to_string());
        }
        if n == 0 {
            return Ok(String::new());
        }
        let bytes = self.take(n as usize)?;
        let bytes = match bytes.split_last() {
            Some((0, rest)) => rest,
            _ => bytes,
        };
        Ok(bytes.iter().map(|&b| b as char).collect())
    }
}

#[derive(Debug, Clone)]
pub struct Import {
    pub class_package: String,
    pub class_name: String,
    pub package: i32,
    pub object_name: String,
}

#[derive(Debug, Clone)]
pub struct Export {
    pub class_index: i32,
    pub super_index: i32,
    pub package: i32,
    pub name: String,
    pub flags: u32,
    pub size: usize,
    pub offset: usize,
}

/// One class's UnrealScript source, as embedded by the original compiler.
#[derive(Debug, Clone)]
pub struct ScriptText {
    pub owner: String,
    pub text: String,
}

pub struct Package {
    pub version: u16,
    pub licensee: u16,
    pub names: Vec<String>,
    pub imports: Vec<Import>,
    pub exports: Vec<Export>,
    /// Package name from the file name (set by `open`), used to resolve same-package references.
    pub self_name: Option<String>,
    data: std::sync::Arc<Vec<u8>>,
}

/// File contents shared by every Package opened on the same file (HDTP's packages are hundreds of MB, and many
/// parts of the game open their own Package / ClassDb).
static FILES: std::sync::Mutex<Vec<(PathBuf, std::sync::Weak<Vec<u8>>)>> = std::sync::Mutex::new(Vec::new());

fn read_shared(real: &Path) -> std::io::Result<std::sync::Arc<Vec<u8>>> {
    if let Ok(mut f) = FILES.lock() {
        f.retain(|(_, w)| w.strong_count() > 0);
        if let Some(a) = f.iter().find(|(p, _)| p == real).and_then(|(_, w)| w.upgrade()) {
            return Ok(a);
        }
        let a = std::sync::Arc::new(std::fs::read(real)?);
        f.push((real.to_path_buf(), std::sync::Arc::downgrade(&a)));
        return Ok(a);
    }
    Ok(std::sync::Arc::new(std::fs::read(real)?))
}

impl Package {
    pub fn open(path: impl AsRef<Path>) -> Result<Package> {
        let real = crate::paths::resolve(path.as_ref());
        let mut p = Self::from_shared(read_shared(&real)?)?;
        p.self_name = path.as_ref().file_stem().map(|s| s.to_string_lossy().into_owned());
        Ok(p)
    }

    pub fn from_bytes(data: Vec<u8>) -> Result<Package> {
        Self::from_shared(std::sync::Arc::new(data))
    }

    fn from_shared(data: std::sync::Arc<Vec<u8>>) -> Result<Package> {
        let (version, licensee, names, imports, exports) = {
            let mut r = Reader::new(&data, 0);
            let tag = r.u32()?;
            if tag != PACKAGE_TAG {
                return Err(PkgError::BadTag(tag));
            }
            let ver = r.u32()?;
            let _flags = r.u32()?;
            let name_count = r.u32()? as usize;
            let name_offset = r.u32()? as usize;
            let export_count = r.u32()? as usize;
            let export_offset = r.u32()? as usize;
            let import_count = r.u32()? as usize;
            let import_offset = r.u32()? as usize;

            r.seek(name_offset);
            let mut names = Vec::with_capacity(name_count);
            for _ in 0..name_count {
                names.push(r.fstring()?);
                let _name_flags = r.u32()?;
            }

            let lookup = |i: i32| -> Result<String> {
                names
                    .get(i as usize)
                    .cloned()
                    .ok_or_else(|| PkgError::Unsupported(format!("bad name index {i}")))
            };

            r.seek(import_offset);
            let mut imports = Vec::with_capacity(import_count);
            for _ in 0..import_count {
                let class_package = lookup(r.compact_index()?)?;
                let class_name = lookup(r.compact_index()?)?;
                let package = r.i32()?;
                let object_name = lookup(r.compact_index()?)?;
                imports.push(Import { class_package, class_name, package, object_name });
            }

            r.seek(export_offset);
            let mut exports = Vec::with_capacity(export_count);
            for _ in 0..export_count {
                let class_index = r.compact_index()?;
                let super_index = r.compact_index()?;
                let package = r.i32()?;
                let name = lookup(r.compact_index()?)?;
                let flags = r.u32()?;
                let size = r.compact_index()?.max(0) as usize;
                let offset = if size > 0 { r.compact_index()?.max(0) as usize } else { 0 };
                exports.push(Export { class_index, super_index, package, name, flags, size, offset });
            }

            ((ver & 0xffff) as u16, (ver >> 16) as u16, names, imports, exports)
        };
        Ok(Package { version, licensee, names, imports, exports, self_name: None, data })
    }

    /// Name of the class an object index refers to. 0 means the built-in `Class`,
    /// positive values are exports (1-based), negative values are imports.
    pub fn class_name(&self, index: i32) -> &str {
        if index == 0 {
            "Class"
        } else if index > 0 {
            self.exports.get(index as usize - 1).map(|e| e.name.as_str()).unwrap_or("?")
        } else {
            self.imports.get((-index) as usize - 1).map(|i| i.object_name.as_str()).unwrap_or("?")
        }
    }

    /// Name of any object reference (export or import), or "None" for 0.
    pub fn object_name(&self, index: i32) -> String {
        if index == 0 {
            "None".into()
        } else if index > 0 {
            self.exports.get(index as usize - 1).map(|e| e.name.clone()).unwrap_or_else(|| "?".into())
        } else {
            self.imports.get((-index) as usize - 1).map(|i| i.object_name.clone()).unwrap_or_else(|| "?".into())
        }
    }

    /// Full dotted path of an object reference, following the outer chain
    /// (for example `CoreTexBrick.Stone.StoneWall1` for an import).
    pub fn object_path(&self, index: i32) -> String {
        let mut parts = Vec::new();
        let mut cur = index;
        for _ in 0..16 {
            if cur == 0 {
                break;
            }
            if cur > 0 {
                let Some(e) = self.exports.get(cur as usize - 1) else { break };
                parts.push(e.name.clone());
                cur = e.package;
            } else {
                let Some(i) = self.imports.get((-cur) as usize - 1) else { break };
                parts.push(i.object_name.clone());
                cur = i.package;
            }
        }
        parts.reverse();
        parts.join(".")
    }

    /// Find an export by exact object path within this package (`Group.Name` or `Name`).
    pub fn find_export_by_path(&self, path: &str) -> Option<&Export> {
        let want = path.to_ascii_lowercase();
        (1..=self.exports.len() as i32)
            .find(|&i| self.object_path(i).to_ascii_lowercase() == want)
            .map(|i| &self.exports[i as usize - 1])
    }

    /// Find an export by its bare object name, in any group (scripts often name an imported object without
    /// its group).
    pub fn find_export_by_name(&self, name: &str) -> Option<&Export> {
        (1..=self.exports.len() as i32).find(|&i| self.object_name(i).eq_ignore_ascii_case(name)).map(|i| &self.exports[i as usize - 1])
    }

    /// Like `find_export_by_path`, but only exports whose class name contains `class_part` (a Palette and a
    /// FireTexture can share a name).
    pub fn find_export_of_class(&self, path: &str, class_part: &str) -> Option<&Export> {
        let want = path.to_ascii_lowercase();
        (1..=self.exports.len() as i32)
            .find(|&i| self.class_name(self.exports[i as usize - 1].class_index).contains(class_part) && self.object_path(i).to_ascii_lowercase() == want)
            .map(|i| &self.exports[i as usize - 1])
            .or_else(|| {
                // "Package.Name" without the group (DynamicLoadObject finds it in any group: mods such as Lay D
                // Denton load "FemJC.JCDentonFemaleTex0", which lives in FemJC.Characters).
                if want.contains('.') {
                    return None;
                }
                self.exports.iter().find(|e| self.class_name(e.class_index).contains(class_part) && e.name.eq_ignore_ascii_case(path))
            })
    }

    pub fn export_data(&self, e: &Export) -> &[u8] {
        let end = (e.offset + e.size).min(self.data.len());
        self.data.get(e.offset..end).unwrap_or(&[])
    }

    /// Count of exported objects by class name (for example `Terrorist: 29`).
    pub fn export_counts(&self) -> BTreeMap<String, usize> {
        let mut m = BTreeMap::new();
        for e in &self.exports {
            *m.entry(self.class_name(e.class_index).to_string()).or_insert(0) += 1;
        }
        m
    }

    /// A named TextBuffer object's text (DeusExText.u holds datacube, email and book texts this way).
    pub fn text_named(&self, name: &str) -> Option<String> {
        let e = self.exports.iter().find(|e| e.name.eq_ignore_ascii_case(name) && matches!(self.class_name(e.class_index), "TextBuffer" | "ExtString"))?;
        let data = self.export_data(e);
        let mut r = Reader::new(data, 0);
        r.compact_index().ok()?; // "None" property-list terminator
        if self.class_name(e.class_index) == "TextBuffer" {
            r.u32().ok()?;
            r.u32().ok()?;
            return r.fstring().ok();
        }
        // ExtString: compact byte count, then Latin-1 text.
        let n = r.compact_index().ok()?.max(0) as usize;
        let start = data.len().checked_sub(n).filter(|&s| s <= data.len())?;
        Some(data[start..].iter().map(|&b| b as char).collect())
    }

    /// Embedded UnrealScript source for every class in this package.
    pub fn script_texts(&self) -> Vec<ScriptText> {
        let mut out = Vec::new();
        for e in &self.exports {
            if e.size < 12 || e.package <= 0 || self.class_name(e.class_index) != "TextBuffer" {
                continue;
            }
            let Some(owner) = self.exports.get(e.package as usize - 1) else { continue };
            let mut r = Reader::new(self.export_data(e), 0);
            let parsed = (|| -> Result<String> {
                r.compact_index()?; // "None" property-list terminator
                r.u32()?; // Pos
                r.u32()?; // Top
                r.fstring()
            })();
            if let Ok(text) = parsed {
                out.push(ScriptText { owner: owner.name.clone(), text });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_index_small_and_negative() {
        assert_eq!(Reader::new(&[0x05], 0).compact_index().unwrap(), 5);
        assert_eq!(Reader::new(&[0x85], 0).compact_index().unwrap(), -5);
    }

    #[test]
    fn compact_index_multibyte() {
        // 0x4b, 0x03 decodes to 203 (seen as a script text length in DeusEx.u).
        assert_eq!(Reader::new(&[0x4b, 0x03], 0).compact_index().unwrap(), 203);
    }

    #[test]
    fn fstring_strips_nul() {
        let mut r = Reader::new(&[0x03, b'a', b'b', 0], 0);
        assert_eq!(r.fstring().unwrap(), "ab");
    }

    #[test]
    fn rejects_bad_tag() {
        assert!(matches!(Package::from_bytes(vec![0; 64]), Err(PkgError::BadTag(0))));
    }
}
