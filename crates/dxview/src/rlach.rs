//! Recomp Achievements (RecompLauncher's achievement system, github.com/elripalda/RecompAchievements): our own
//! achievements reported to the launcher as code events, the "events" kind of list. Written in Rust from the
//! channel's published spec (docs/SPEC.md there, version 1): when RecompLauncher starts the game it sets
//! RL_SESSION to a pipe (Windows) or socket (Linux, macOS) name; both sides send one JSON object per line.
//!
//! We send `hello` once, then `event` (name = our achievement id) for every unlock, `save_loaded` when a saved
//! game is loaded, `casual` when cheats are used, and `presence` (where the player is). The launcher may send
//! `quiet`: it shows the unlock toast itself, so ours stays hidden. Without RL_SESSION nothing here does anything.
//!
//! The list that maps our ids to Recomp Achievements entries is tools/recomp-achievements/dx-rust.json.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};

/// The SDK protocol version we speak.
const SDK: u32 = 1;

static TX: OnceLock<Mutex<Sender<String>>> = OnceLock::new();
static QUIET: AtomicBool = AtomicBool::new(false);
static FRAME: AtomicU64 = AtomicU64::new(0);
static PRESENCE: Mutex<String> = Mutex::new(String::new());

/// A JSON string literal.
fn js(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn send(line: String) {
    if let Some(tx) = TX.get() {
        if let Ok(tx) = tx.lock() {
            let _ = tx.send(line);
        }
    }
}

/// Connected to RecompLauncher's achievement channel.
pub fn connected() -> bool {
    TX.get().is_some()
}

/// The launcher shows unlock toasts itself: hide ours.
pub fn quiet() -> bool {
    QUIET.load(Ordering::Relaxed)
}

/// Connect when RecompLauncher gave a session (once, at start).
pub fn init() {
    let Some(session) = std::env::var("RL_SESSION").ok().filter(|s| !s.is_empty()) else { return };
    let Some(conn) = Conn::open(&session) else {
        eprintln!("achievements: RL_SESSION {session} can't be opened");
        return;
    };
    let (tx, rx) = channel::<String>();
    let hello = format!(
        "{{\"t\":\"hello\",\"sdk\":{SDK},\"console\":\"\",\"memory\":false,\"game\":\"dx-rust\",\"version\":{}}}",
        js(env!("CARGO_PKG_VERSION"))
    );
    let _ = tx.send(hello);
    if TX.set(Mutex::new(tx)).is_err() {
        return;
    }
    println!("achievements: connected to RecompLauncher ({session})");
    std::thread::Builder::new()
        .name("rl-achievements".into())
        .spawn(move || {
            let mut conn = conn;
            let mut inbuf: Vec<u8> = Vec::new();
            loop {
                // Lines out.
                loop {
                    match rx.try_recv() {
                        Ok(l) => {
                            if !conn.write_line(&l) {
                                return;
                            }
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
                    }
                }
                // Lines in: only `quiet` matters to us (`list` / `peek` are for memory rules).
                match conn.read_some(&mut inbuf) {
                    None => return,
                    Some(_) => {
                        while let Some(nl) = inbuf.iter().position(|&b| b == b'\n') {
                            let line: Vec<u8> = inbuf.drain(..=nl).collect();
                            let l = String::from_utf8_lossy(&line);
                            if l.contains("\"t\":\"quiet\"") || l.contains("\"t\": \"quiet\"") {
                                QUIET.store(true, Ordering::Relaxed);
                            }
                        }
                        if inbuf.len() > 1 << 20 {
                            inbuf.clear();
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
        })
        .ok();
}

/// Once a frame (the frame number goes with each message).
pub fn frame() {
    FRAME.fetch_add(1, Ordering::Relaxed);
}

fn frame_no() -> u64 {
    FRAME.load(Ordering::Relaxed)
}

/// One of our achievements unlocked: its id is the event name in the list.
pub fn event(name: &str) {
    if connected() {
        send(format!("{{\"t\":\"event\",\"name\":{},\"frame\":{}}}", js(name), frame_no()));
    }
}

/// A saved game was loaded (unlocks just after can't count as Ranked).
pub fn save_loaded() {
    if connected() {
        send(format!("{{\"t\":\"save_loaded\",\"frame\":{}}}", frame_no()));
    }
}

/// Cheats (the debug menu, god mode...) were used: unlocks from now on are Casual.
pub fn casual(reason: &str) {
    if connected() {
        send(format!("{{\"t\":\"casual\",\"reason\":{}}}", js(reason)));
    }
}

/// What the player is doing ("Liberty Island", "Main menu"); sent only when it changes.
pub fn presence(text: &str) {
    if !connected() {
        return;
    }
    if let Ok(mut p) = PRESENCE.lock() {
        if *p == text {
            return;
        }
        *p = text.to_string();
    }
    send(format!("{{\"t\":\"presence\",\"text\":{}}}", js(text)));
}

// ------------------------------------------------------------------------------------------------ transport

#[cfg(unix)]
struct Conn(std::os::unix::net::UnixStream);

#[cfg(unix)]
impl Conn {
    fn open(name: &str) -> Option<Conn> {
        let s = std::os::unix::net::UnixStream::connect(name).ok()?;
        s.set_nonblocking(true).ok()?;
        Some(Conn(s))
    }
    fn write_line(&mut self, l: &str) -> bool {
        use std::io::Write;
        let mut data = Vec::with_capacity(l.len() + 1);
        data.extend_from_slice(l.as_bytes());
        data.push(b'\n');
        let mut off = 0;
        let start = std::time::Instant::now();
        while off < data.len() {
            match self.0.write(&data[off..]) {
                Ok(0) => return false,
                Ok(n) => off += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::Interrupted => {
                    if start.elapsed().as_secs() > 2 {
                        return false;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                Err(_) => return false,
            }
        }
        true
    }
    /// Reads what's waiting. None: the launcher closed the channel.
    fn read_some(&mut self, buf: &mut Vec<u8>) -> Option<usize> {
        use std::io::Read;
        let mut tmp = [0u8; 4096];
        match self.0.read(&mut tmp) {
            Ok(0) => None,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                Some(n)
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::Interrupted => Some(0),
            Err(_) => None,
        }
    }
}

#[cfg(windows)]
struct Conn(std::fs::File);

#[cfg(windows)]
impl Conn {
    fn open(name: &str) -> Option<Conn> {
        let path = if name.starts_with(r"\\.\pipe\") { name.to_string() } else { format!(r"\\.\pipe\{name}") };
        // The launcher keeps a spare pipe instance; while it swaps, the pipe can be busy for a moment.
        for _ in 0..4 {
            if let Ok(f) = std::fs::OpenOptions::new().read(true).write(true).open(&path) {
                return Some(Conn(f));
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        None
    }
    fn write_line(&mut self, l: &str) -> bool {
        use std::io::Write;
        self.0.write_all(l.as_bytes()).is_ok() && self.0.write_all(b"\n").is_ok()
    }
    fn read_some(&mut self, buf: &mut Vec<u8>) -> Option<usize> {
        use std::io::Read;
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::Pipes::PeekNamedPipe;
        // A synchronous pipe handle serialises reads and writes: read only what's already there.
        let mut avail: u32 = 0;
        let ok = unsafe {
            PeekNamedPipe(self.0.as_raw_handle() as _, std::ptr::null_mut(), 0, std::ptr::null_mut(), &mut avail, std::ptr::null_mut())
        };
        if ok == 0 {
            return None;
        }
        if avail == 0 {
            return Some(0);
        }
        let mut tmp = vec![0u8; avail.min(65536) as usize];
        match self.0.read(&mut tmp) {
            Ok(0) => None,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                Some(n)
            }
            Err(_) => None,
        }
    }
}

#[cfg(not(any(unix, windows)))]
struct Conn;

#[cfg(not(any(unix, windows)))]
impl Conn {
    fn open(_: &str) -> Option<Conn> {
        None
    }
    fn write_line(&mut self, _: &str) -> bool {
        false
    }
    fn read_some(&mut self, _: &mut Vec<u8>) -> Option<usize> {
        None
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::io::{BufRead, Write};

    /// A fake launcher on a socket: the hello, an event and a presence arrive as JSON lines; `quiet` is taken in.
    #[test]
    fn talks_to_a_fake_launcher() {
        let path = std::env::temp_dir().join(format!("rl-test-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        std::env::set_var("RL_SESSION", &path);
        super::init();
        let (mut s, _) = listener.accept().unwrap();
        s.write_all(b"{\"t\":\"quiet\"}\n").unwrap();
        super::frame();
        super::event("lady");
        super::presence("Liberty Island");
        super::presence("Liberty Island");
        let mut r = std::io::BufReader::new(s.try_clone().unwrap());
        let mut lines = Vec::new();
        for _ in 0..3 {
            let mut l = String::new();
            r.read_line(&mut l).unwrap();
            lines.push(l);
        }
        assert!(lines[0].starts_with("{\"t\":\"hello\",\"sdk\":1,"), "{}", lines[0]);
        assert!(lines[0].contains("\"game\":\"dx-rust\""));
        assert_eq!(lines[1].trim(), "{\"t\":\"event\",\"name\":\"lady\",\"frame\":1}");
        assert_eq!(lines[2].trim(), "{\"t\":\"presence\",\"text\":\"Liberty Island\"}");
        for _ in 0..50 {
            if super::quiet() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(super::quiet());
        let _ = std::fs::remove_file(&path);
    }
}
