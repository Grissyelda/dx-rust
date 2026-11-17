//! A log file per run: everything the game prints (its own lines and the engine's) also goes into
//! `logs/dxview-<date>-<time>.log` in the data folder, the newest ten kept. RecompLauncher shows the newest one
//! when the game crashes and sends it with the player's report; anyone else can attach it to a bug report.
//!
//! How: stdout and stderr are pointed at a pipe, and a thread copies what comes through to the console (when
//! there is one) and to the file.

use std::io::Write;
use std::path::PathBuf;

/// Logs kept (older ones are deleted at start).
const KEEP: usize = 10;

static PATH: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// This run's log file (once `start` ran).
pub fn path() -> Option<&'static PathBuf> {
    PATH.get()
}

/// The logs folder in the data folder.
pub fn dir() -> PathBuf {
    unreal_pkg::paths::data_dir().join("logs")
}

/// "YYYYMMDD-HHMMSS" (UTC) for a unix time.
pub fn stamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", rem / 3600, rem / 60 % 60, rem % 60)
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Delete all but the newest `keep - 1` logs (the new one makes `keep`).
fn prune(dir: &std::path::Path, keep: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut logs: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("dxview-")) && p.extension().is_some_and(|e| e == "log"))
        .collect();
    logs.sort();
    while logs.len() >= keep {
        let _ = std::fs::remove_file(logs.remove(0));
    }
}

/// Start this run's log (desktop; Android logs to logcat and startup.log). DXVIEW_NO_RUNLOG=1 leaves it off.
pub fn start() {
    if std::env::var_os("DXVIEW_NO_RUNLOG").is_some() || cfg!(target_os = "android") {
        return;
    }
    let dir = dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    prune(&dir, KEEP);
    let path = dir.join(format!("dxview-{}.log", stamp(now())));
    let Ok(mut file) = std::fs::File::create(&path) else { return };
    let _ = writeln!(
        file,
        "DX RUST {} ({} {}) started {} UTC, args {:?}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        stamp(now()),
        std::env::args().skip(1).collect::<Vec<_>>()
    );
    if tee(file) {
        let _ = PATH.set(path);
    }
}

/// A copy of the crash message next to the run logs (the run log has it too, through stderr).
pub fn crash(msg: &str) {
    let dir = dir();
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(dir.join(format!("crash-{}.log", stamp(now()))), msg);
    }
}

#[cfg(unix)]
fn tee(file: std::fs::File) -> bool {
    use std::io::Read;
    use std::os::fd::FromRawFd;
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    unsafe {
        let mut fds = [0i32; 2];
        if libc::pipe(fds.as_mut_ptr()) != 0 {
            return false;
        }
        // The console the game was started from, kept to copy to.
        let console = libc::dup(1);
        if libc::dup2(fds[1], 1) < 0 || libc::dup2(fds[1], 2) < 0 {
            return false;
        }
        libc::close(fds[1]);
        let mut reader = std::fs::File::from_raw_fd(fds[0]);
        let mut out = (console >= 0).then(|| std::fs::File::from_raw_fd(console));
        let mut file = file;
        std::thread::Builder::new()
            .name("runlog".into())
            .spawn(move || {
                let mut buf = [0u8; 8192];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    if let Some(o) = out.as_mut() {
                        let _ = o.write_all(&buf[..n]);
                    }
                    let _ = file.write_all(&buf[..n]);
                    let _ = file.flush();
                }
            })
            .is_ok()
    }
}

#[cfg(windows)]
fn tee(file: std::fs::File) -> bool {
    use std::io::Read;
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Console::{GetStdHandle, SetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE};
    use windows_sys::Win32::System::Pipes::CreatePipe;
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    unsafe {
        let mut read: HANDLE = std::ptr::null_mut();
        let mut write: HANDLE = std::ptr::null_mut();
        if CreatePipe(&mut read, &mut write, std::ptr::null(), 0) == 0 {
            return false;
        }
        let console = GetStdHandle(STD_OUTPUT_HANDLE);
        // Rust's stdout / stderr look the handles up on every write, so new writes go into the pipe.
        if SetStdHandle(STD_OUTPUT_HANDLE, write) == 0 || SetStdHandle(STD_ERROR_HANDLE, write) == 0 {
            return false;
        }
        let mut reader = std::fs::File::from_raw_handle(read as _);
        let mut out = (!console.is_null() && console != INVALID_HANDLE_VALUE).then(|| std::fs::File::from_raw_handle(console as _));
        let mut file = file;
        std::thread::Builder::new()
            .name("runlog".into())
            .spawn(move || {
                let mut buf = [0u8; 8192];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    if let Some(o) = out.as_mut() {
                        let _ = o.write_all(&buf[..n]);
                    }
                    let _ = file.write_all(&buf[..n]);
                    let _ = file.flush();
                }
            })
            .is_ok()
    }
}

#[cfg(not(any(unix, windows)))]
fn tee(_file: std::fs::File) -> bool {
    false
}

#[cfg(test)]
mod tests {
    #[test]
    fn stamps() {
        assert_eq!(super::stamp(0), "19700101-000000");
        assert_eq!(super::stamp(1_794_922_838), "20261117-134038");
    }
}
