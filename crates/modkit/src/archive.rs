//! What a downloaded mod file is, and unpacking it (pure Rust, so it works the same on Windows, a Steam Deck, a
//! Mac or Android).
//!
//! Handled: `.zip`, `.7z`, and Windows installers that are really one of those with a small program in front - the
//! 7-Zip self-extracting installers (7zSD / 7zS, as The Nameless Mod's installer is: its own readme says to rename
//! it to `.7z`) and zip self-extractors. Recognised but not unpacked (the player is told what to do): RAR archives,
//! Inno Setup and NSIS installers, MSI packages.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

/// What a file turned out to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A zip archive (from `offset` on: a zip self-extractor has its program in front).
    Zip { offset: u64 },
    /// A 7z archive starting at `offset` (0 for a .7z; past the program for a self-extracting .exe).
    SevenZ { offset: u64 },
    Rar,
    InnoSetup,
    Nsis,
    Msi,
    /// A Windows program with no archive we know inside.
    Exe,
    Unknown,
}

impl Kind {
    pub fn can_unpack(self) -> bool {
        matches!(self, Kind::Zip { .. } | Kind::SevenZ { .. })
    }

    /// For the player: what it is, and what to do when it can't be unpacked here.
    pub fn describe(self) -> &'static str {
        match self {
            Kind::Zip { offset: 0 } => "a zip archive",
            Kind::Zip { .. } => "a self-extracting zip installer",
            Kind::SevenZ { offset: 0 } => "a 7z archive",
            Kind::SevenZ { .. } => "a 7-Zip self-extracting installer",
            Kind::Rar => "a RAR archive. RAR can't be unpacked here: extract it with 7-Zip (or Ark on a Steam Deck), then add the folder",
            Kind::InnoSetup => "an Inno Setup installer. Run it once (on Windows, or with Proton on a Steam Deck), or unpack it with innoextract, then add the folder it made",
            Kind::Nsis => "an NSIS installer. Open it with 7-Zip (or run it on Windows or with Proton), then add the folder it made",
            Kind::Msi => "an MSI installer. Run it on Windows, then add the folder it made",
            Kind::Exe => "a Windows program with nothing inside it that can be unpacked here. Run it on Windows (or with Proton on a Steam Deck), then add the folder it made",
            Kind::Unknown => "not an archive or installer this game knows",
        }
    }
}

const SEVENZ_SIG: [u8; 6] = [b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C];
/// How far into an .exe to look for an archive (the 7-Zip and zip self-extractor programs are a few hundred KB).
const EXE_SCAN: u64 = 16 << 20;

/// A real 7z start header at `buf[i..]`: the signature, a known version, and the start header's CRC over the next
/// 20 bytes (a signature on its own can turn up inside a program by chance).
fn sevenz_header_at(buf: &[u8], i: usize) -> bool {
    if buf.len() < i + 32 || buf[i..i + 6] != SEVENZ_SIG || buf[i + 6] != 0 || buf[i + 7] > 4 {
        return false;
    }
    let crc = u32::from_le_bytes(buf[i + 8..i + 12].try_into().unwrap());
    crc32fast::hash(&buf[i + 12..i + 32]) == crc
}

/// Find text in the first part of a program (installer brands).
fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// What `path` is (reads its start, and for a program its first 16 MB).
pub fn sniff(path: &Path) -> io::Result<Kind> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    let mut head = [0u8; 8];
    let n = f.read(&mut head)?;
    let head = &head[..n];
    if head.starts_with(&SEVENZ_SIG) {
        return Ok(Kind::SevenZ { offset: 0 });
    }
    if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        return Ok(Kind::Zip { offset: 0 });
    }
    if head.starts_with(b"Rar!\x1a\x07") {
        return Ok(Kind::Rar);
    }
    if head.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) {
        return Ok(Kind::Msi);
    }
    if !head.starts_with(b"MZ") {
        return Ok(Kind::Unknown);
    }
    // A program: read its first part and look for an archive or an installer's brand.
    f.seek(SeekFrom::Start(0))?;
    let mut buf = Vec::new();
    Read::by_ref(&mut f).take(EXE_SCAN.min(len)).read_to_end(&mut buf)?;
    // A 7z archive after the program (7-Zip's self-extractors append it, usually on a 512-byte or 4-byte
    // boundary, but any offset is checked: the header's CRC makes a false find all but impossible).
    let mut i = 0;
    while let Some(p) = buf[i..].windows(6).position(|w| w == SEVENZ_SIG) {
        let at = i + p;
        if sevenz_header_at(&buf, at) {
            return Ok(Kind::SevenZ { offset: at as u64 });
        }
        i = at + 1;
    }
    if contains(&buf, b"Inno Setup Setup Data") || contains(&buf, b"Inno Setup") && contains(&buf, b"JR.Inno.Setup") {
        return Ok(Kind::InnoSetup);
    }
    if contains(&buf, b"NullsoftInst") {
        return Ok(Kind::Nsis);
    }
    // A zip after the program: the zip reader finds the central directory from the end.
    if zip::ZipArchive::new(File::open(path)?).is_ok() {
        let first = buf.windows(4).position(|w| w == b"PK\x03\x04").unwrap_or(0) as u64;
        return Ok(Kind::Zip { offset: first });
    }
    Ok(Kind::Exe)
}

/// A file seen from `base` on (a 7z archive after a self-extractor's program).
struct Offset<R> {
    inner: R,
    base: u64,
}

impl<R: Read> Read for Offset<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<R: Seek> Seek for Offset<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let p = match pos {
            SeekFrom::Start(s) => self.inner.seek(SeekFrom::Start(self.base + s))?,
            other => self.inner.seek(other)?,
        };
        Ok(p.saturating_sub(self.base))
    }
}

/// A path inside an archive made safe to write under the destination: no absolute paths, no `..`, Windows
/// separators taken as folders.
pub fn safe_rel(name: &str) -> Option<PathBuf> {
    let name = name.replace('\\', "/");
    let mut out = PathBuf::new();
    for c in Path::new(&name).components() {
        match c {
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// Progress while unpacking: done 0..1 and the file being written.
pub type Progress<'a> = &'a mut dyn FnMut(f32, &str);

/// Unpack an archive of a known kind into `dest` (made if missing). Returns the number of files written.
pub fn extract(path: &Path, kind: Kind, dest: &Path, progress: Progress) -> Result<usize, String> {
    std::fs::create_dir_all(dest).map_err(|e| format!("can't make {}: {e}", dest.display()))?;
    match kind {
        Kind::SevenZ { offset } => extract_7z(path, offset, dest, progress),
        Kind::Zip { .. } => extract_zip(path, dest, progress),
        other => Err(format!("This file is {}.", other.describe())),
    }
}

fn extract_7z(path: &Path, offset: u64, dest: &Path, progress: Progress) -> Result<usize, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut reader = Offset { inner: io::BufReader::with_capacity(1 << 20, file), base: offset };
    reader.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut ar = sevenz_rust2::ArchiveReader::new(reader, sevenz_rust2::Password::empty())
        .map_err(|e| format!("not a 7z archive this can read: {e}"))?;
    let total: u64 = ar.archive().files.iter().map(|f| f.size()).sum::<u64>().max(1);
    let mut done = 0u64;
    let mut count = 0usize;
    let mut err: Option<String> = None;
    let mut buf = vec![0u8; 1 << 20];
    ar.for_each_entries(|entry, r| {
        let Some(rel) = safe_rel(entry.name()) else { return Ok(true) };
        let out = dest.join(&rel);
        if entry.is_directory() {
            let _ = std::fs::create_dir_all(&out);
            return Ok(true);
        }
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p)?;
        }
        let mut w = io::BufWriter::with_capacity(1 << 20, File::create(&out)?);
        loop {
            let n = r.read(&mut buf)?;
            if n == 0 {
                break;
            }
            w.write_all(&buf[..n])?;
            done += n as u64;
            progress(done as f32 / total as f32, entry.name());
        }
        w.flush()?;
        count += 1;
        Ok(true)
    })
    .map_err(|e| {
        err = Some(e.to_string());
        e.to_string()
    })?;
    if let Some(e) = err {
        return Err(e);
    }
    Ok(count)
}

fn extract_zip(path: &Path, dest: &Path, progress: Progress) -> Result<usize, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(io::BufReader::new(file)).map_err(|e| format!("not a zip archive this can read: {e}"))?;
    let total: u64 = (0..z.len()).filter_map(|i| z.by_index_raw(i).ok().map(|f| f.size())).sum::<u64>().max(1);
    let mut done = 0u64;
    let mut count = 0usize;
    let mut buf = vec![0u8; 1 << 20];
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| e.to_string())?;
        let name = f.name().to_string();
        let Some(rel) = safe_rel(&name) else { continue };
        let out = dest.join(rel);
        if f.is_dir() {
            let _ = std::fs::create_dir_all(&out);
            continue;
        }
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        let mut w = io::BufWriter::new(File::create(&out).map_err(|e| format!("{}: {e}", out.display()))?);
        loop {
            let n = f.read(&mut buf).map_err(|e| format!("{name}: {e}"))?;
            if n == 0 {
                break;
            }
            w.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            done += n as u64;
            progress(done as f32 / total as f32, &name);
        }
        w.flush().map_err(|e| e.to_string())?;
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_paths_are_refused() {
        assert_eq!(safe_rel("TNM2\\System\\TNM.u"), Some(PathBuf::from("TNM2/System/TNM.u")));
        assert_eq!(safe_rel("../evil.txt"), None);
        assert_eq!(safe_rel("/etc/passwd"), None);
        assert_eq!(safe_rel("a/../../b"), None);
        assert_eq!(safe_rel("./ok/file"), Some(PathBuf::from("ok/file")));
    }
}
