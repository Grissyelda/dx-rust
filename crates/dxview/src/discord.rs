//! Enhancement "Discord Rich Presence" (ours): shows the map, the mission and the difficulty in Discord.
//!
//! Discord's local RPC: a named pipe (Windows `\\.\pipe\discord-ipc-N`) or Unix socket
//! (`$XDG_RUNTIME_DIR/discord-ipc-N`, also under the Flatpak / Snap folders). Frames are op (u32 LE), length
//! (u32 LE), JSON. Handshake op 0 `{"v":1,"client_id":...}`, then op 1 SET_ACTIVITY. Everything runs on its own
//! thread; nothing waits for Discord, and with Discord closed it quietly retries every 20 s.
//!
//! The application id is the game's Discord application (built in: BUILT_IN_APP_ID); `DiscordAppId=<id>` in
//! launcher.txt or DXVIEW_DISCORD_ID override it. Its Rich Presence art asset named `dxrust` is the large image.

use std::io::{Read, Write};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// (details, state) to show; set by `set`, read by the thread.
static WANT: Mutex<Option<(String, String)>> = Mutex::new(None);
static STARTED: std::sync::Once = std::sync::Once::new();

/// The project's own Discord application (built in, so players don't set anything up). Empty = none yet.
const BUILT_IN_APP_ID: &str = "1564865124123152444";

fn app_id() -> Option<String> {
    if let Ok(v) = std::env::var("DXVIEW_DISCORD_ID") {
        if !v.trim().is_empty() {
            return Some(v.trim().to_string());
        }
    }
    let dir = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())).unwrap_or_default();
    std::fs::read_to_string(dir.join("launcher.txt"))
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.strip_prefix("DiscordAppId=").map(|v| v.trim().to_string()))
        .filter(|v| !v.is_empty())
        .or_else(|| (!BUILT_IN_APP_ID.is_empty()).then(|| BUILT_IN_APP_ID.to_string()))
}

/// What the presence says now (the map loader calls this on every arrival).
pub fn set(details: &str, state: &str) {
    if let Ok(mut w) = WANT.lock() {
        *w = Some((details.to_string(), state.to_string()));
    }
    STARTED.call_once(|| {
        if let Some(id) = app_id() {
            let _ = std::thread::Builder::new().name("discord".into()).spawn(move || run(id));
        }
    });
}

fn esc(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

trait Pipe: Read + Write + Send {}
impl<T: Read + Write + Send> Pipe for T {}

fn connect() -> Option<Box<dyn Pipe>> {
    for n in 0..10 {
        #[cfg(windows)]
        {
            if let Ok(f) = std::fs::OpenOptions::new().read(true).write(true).open(format!(r"\\.\pipe\discord-ipc-{n}")) {
                return Some(Box::new(f));
            }
        }
        #[cfg(unix)]
        {
            let mut dirs: Vec<String> = Vec::new();
            for v in ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"] {
                if let Ok(d) = std::env::var(v) {
                    dirs.push(d.clone());
                    dirs.push(format!("{d}/app/com.discordapp.Discord"));
                    dirs.push(format!("{d}/snap.discord"));
                }
            }
            dirs.push("/tmp".into());
            for d in dirs {
                if let Ok(s) = std::os::unix::net::UnixStream::connect(format!("{d}/discord-ipc-{n}")) {
                    return Some(Box::new(s));
                }
            }
        }
    }
    None
}

fn send(p: &mut dyn Pipe, op: u32, json: &str) -> std::io::Result<()> {
    let mut b = Vec::with_capacity(8 + json.len());
    b.extend_from_slice(&op.to_le_bytes());
    b.extend_from_slice(&(json.len() as u32).to_le_bytes());
    b.extend_from_slice(json.as_bytes());
    p.write_all(&b)?;
    p.flush()
}

fn recv(p: &mut dyn Pipe) -> std::io::Result<()> {
    let mut h = [0u8; 8];
    p.read_exact(&mut h)?;
    let len = u32::from_le_bytes([h[4], h[5], h[6], h[7]]) as usize;
    let mut body = vec![0u8; len.min(1 << 20)];
    p.read_exact(&mut body)
}

fn run(id: String) {
    let start = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut nonce = 0u64;
    loop {
        let Some(mut p) = connect() else {
            std::thread::sleep(Duration::from_secs(20));
            continue;
        };
        if send(&mut *p, 0, &format!(r#"{{"v":1,"client_id":"{}"}}"#, esc(&id))).is_err() || recv(&mut *p).is_err() {
            std::thread::sleep(Duration::from_secs(20));
            continue;
        }
        let mut shown: Option<(String, String)> = None;
        loop {
            let want = WANT.lock().ok().and_then(|w| w.clone());
            let off = crate::settings::get("DiscordPresence") < 0.5;
            let want = if off { None } else { want };
            if want != shown {
                nonce += 1;
                let activity = match &want {
                    Some((d, s)) => format!(
                        r#"{{"details":"{}","state":"{}","timestamps":{{"start":{start}}},"assets":{{"large_image":"dxrust","large_text":"DX RUST"}}}}"#,
                        esc(d),
                        esc(s)
                    ),
                    None => "null".into(),
                };
                let msg = format!(
                    r#"{{"cmd":"SET_ACTIVITY","args":{{"pid":{},"activity":{activity}}},"nonce":"{nonce}"}}"#,
                    std::process::id()
                );
                if send(&mut *p, 1, &msg).is_err() || recv(&mut *p).is_err() {
                    break;
                }
                shown = want;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        std::thread::sleep(Duration::from_secs(20));
    }
}
