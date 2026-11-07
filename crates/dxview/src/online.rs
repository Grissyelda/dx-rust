//! Online services from the game's side: the connection to the lobby server (imsim-net), made only when the
//! player opens Multiplayer, on a thread of its own so the menus never wait on the network.
//!
//! The menus ask for things (connect, host a room, change its settings, list the public rooms, find a room by
//! code) and read the answers back from here each frame. The server's address: env DXVIEW_LOBBY, else
//! `LobbyServer=host:port` in launcher.txt beside the exe, else this PC (127.0.0.1, the default port).

use imsim_net::client::{explain, Client, Joined};
use imsim_net::{RoomInfo, RoomSettings};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The connection, as the menus see it.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Offline,
    Connecting,
    Online { server: String, ms: u32 },
    Failed(String),
}

/// The room this player hosts.
#[derive(Clone, Debug)]
pub struct Hosted {
    pub code: String,
    pub settings: RoomSettings,
    pub players: u8,
}

/// The public room list.
#[derive(Clone, Debug, PartialEq)]
pub enum ListState {
    Idle,
    Loading,
    Ready,
    Failed(String),
}

enum Req {
    Host(RoomSettings),
    Update(RoomSettings),
    CloseRoom,
    List,
    Join(String),
}

struct State {
    status: Status,
    hosted: Option<Hosted>,
    /// Hosting asked for, answer not back yet.
    hosting: bool,
    rooms: Vec<RoomInfo>,
    list: ListState,
    /// Our own round trip when the list came (the browser's ping = ours + the host's).
    list_ms: u32,
    join: Option<Result<Joined, String>>,
    joining: bool,
    tx: Option<Sender<Req>>,
    /// Players who found our room (their IPs), newest last.
    joiners: Vec<String>,
    /// Why the last Host failed.
    host_error: Option<String>,
    /// Bumped by connect / disconnect: a worker from an older connection says nothing more.
    gen: u64,
}

static STATE: Mutex<State> = Mutex::new(State {
    status: Status::Offline,
    hosted: None,
    hosting: false,
    rooms: Vec::new(),
    list: ListState::Idle,
    list_ms: 0,
    join: None,
    joining: false,
    tx: None,
    joiners: Vec::new(),
    host_error: None,
    gen: 0,
});

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut g)
}

/// Where the lobby server is.
pub fn server_addr() -> String {
    if let Ok(v) = std::env::var("DXVIEW_LOBBY") {
        if !v.trim().is_empty() {
            return v.trim().to_string();
        }
    }
    std::fs::read_to_string(unreal_pkg::paths::data_dir().join("launcher.txt"))
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.strip_prefix("LobbyServer=").map(|v| v.trim().to_string()))
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| format!("127.0.0.1:{}", imsim_net::DEFAULT_PORT))
}

/// Point the game at another lobby (the menus' Online Services): launcher.txt's `LobbyServer=` line is rewritten, or
/// added, every other line kept as it was (line endings too); an empty address removes it (this PC). Env
/// DXVIEW_LAUNCHER_TXT names another file (tests).
pub fn set_server_addr(addr: &str) -> std::io::Result<()> {
    let path = std::env::var_os("DXVIEW_LAUNCHER_TXT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| unreal_pkg::paths::data_dir().join("launcher.txt"));
    let old = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let nl = if old.contains("\r\n") { "\r\n" } else { "\n" };
    let addr = addr.trim();
    let mut out = String::with_capacity(old.len() + addr.len() + 16);
    let mut done = false;
    for l in old.lines() {
        if l.starts_with("LobbyServer=") {
            if !done && !addr.is_empty() {
                out.push_str("LobbyServer=");
                out.push_str(addr);
                out.push_str(nl);
            }
            done = true;
            continue;
        }
        out.push_str(l);
        out.push_str(nl);
    }
    if !done && !addr.is_empty() {
        out.push_str("LobbyServer=");
        out.push_str(addr);
        out.push_str(nl);
    }
    std::fs::write(&path, out)
}

/// Connect to online services (nothing happens if connected or connecting already).
pub fn connect() {
    let start = with(|s| {
        if matches!(s.status, Status::Connecting | Status::Online { .. }) {
            return false;
        }
        s.status = Status::Connecting;
        s.list = ListState::Idle;
        true
    });
    if !start {
        return;
    }
    let (tx, rx) = channel();
    let gen = with(|s| {
        s.tx = Some(tx);
        s.gen += 1;
        s.gen
    });
    let addr = server_addr();
    let _ = std::thread::Builder::new().name("online".into()).spawn(move || worker(addr, rx, gen));
}

/// Leave online services (closes a hosted room).
pub fn disconnect() {
    with(|s| {
        s.gen += 1;
        s.tx = None;
        s.status = Status::Offline;
        s.hosted = None;
        s.hosting = false;
        s.joiners.clear();
    });
}

pub fn status() -> Status {
    with(|s| s.status.clone())
}

pub fn hosted() -> Option<Hosted> {
    with(|s| s.hosted.clone())
}

/// Hosting asked for and not answered yet.
pub fn hosting() -> bool {
    with(|s| s.hosting)
}

fn ask(r: Req) {
    with(|s| {
        if let Some(tx) = &s.tx {
            let _ = tx.send(r);
        }
    });
}

/// Open a room (its code shows up in `hosted()`).
pub fn host(settings: RoomSettings) {
    with(|s| {
        s.hosting = true;
        s.host_error = None;
    });
    ask(Req::Host(settings));
}

/// Change the hosted room's settings.
pub fn update(settings: RoomSettings) {
    if crate::netplay::is_host() {
        crate::netplay::set_rules(rules_of(&settings));
    }
    with(|s| {
        if let Some(h) = s.hosted.as_mut() {
            h.settings = RoomSettings { name: h.settings.name.clone(), ..settings.clone() };
        }
    });
    ask(Req::Update(settings));
}

/// Why the last Host Room failed (taken once).
pub fn take_host_error() -> Option<String> {
    with(|s| s.host_error.take())
}

/// The game rules a room's settings make (friendly fire stays the host's netplay toggle).
pub fn rules_of(s: &RoomSettings) -> crate::netplay::Rules {
    let old = crate::netplay::rules().unwrap_or_default();
    crate::netplay::Rules {
        coop: s.mode == imsim_net::Mode::Coop,
        classic: s.look == imsim_net::Look::Classic,
        tough: s.tough,
        max_players: s.max_players,
        friendly_fire: old.friendly_fire,
        difficulty: crate::travel::combat_difficulty(),
        dm: crate::deathmatch::rules(),
    }
}

/// The game's own socket and the room's key (netplay), before the lobby is asked for a code.
fn game_side(s: &RoomSettings) -> Result<(u16, String, Option<std::net::SocketAddr>), String> {
    crate::netplay::open_host(rules_of(s))
}

fn host_registered(token: &str) {
    crate::netplay::host_registered(token);
}

pub fn close_room() {
    crate::netplay::leave("");
    with(|s| {
        s.hosted = None;
        s.hosting = false;
    });
    ask(Req::CloseRoom);
}

/// Fetch the public room list again (clears the old one).
pub fn refresh_list() {
    with(|s| {
        s.rooms.clear();
        s.list = ListState::Loading;
    });
    ask(Req::List);
}

/// The public rooms, the list's state, and our own round trip to the lobby when it came.
pub fn rooms() -> (Vec<RoomInfo>, ListState, u32) {
    with(|s| (s.rooms.clone(), s.list.clone(), s.list_ms))
}

/// Look a room up by code (the answer comes from `take_join`).
pub fn join(code: &str) {
    with(|s| {
        s.join = None;
        s.joining = true;
    });
    ask(Req::Join(code.to_string()));
}

pub fn joining() -> bool {
    with(|s| s.joining)
}

pub fn take_join() -> Option<Result<Joined, String>> {
    with(|s| s.join.take())
}

/// Players who looked our room up (IPs). Unused since the host room shows netplay's player list.
#[allow(dead_code)]
pub fn joiners() -> Vec<String> {
    with(|s| s.joiners.clone())
}

fn worker(addr: String, rx: Receiver<Req>, gen: u64) {
    let mut c = match Client::connect(&addr, Duration::from_secs(5)) {
        Ok(c) => c,
        Err(e) => {
            with(|s| {
                if s.gen == gen {
                    s.status = Status::Failed(format!("Could not reach online services at {addr} ({e})."));
                    s.tx = None;
                }
            });
            return;
        }
    };
    let ms = c.ping().unwrap_or(0);
    let server = c.server_name.clone();
    // Cancelled (or connected again) while this one was connecting: it goes quietly.
    let current = with(|s| {
        if s.gen == gen {
            s.status = Status::Online { server: server.clone(), ms };
            true
        } else {
            false
        }
    });
    if !current {
        return;
    }
    let mut token: Option<String> = None;
    let mut last_ping = Instant::now();
    loop {
        // Disconnected by the menus: the sender is gone.
        let req = match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(r) => Some(r),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
            Err(_) => {
                if let Some(t) = token.take() {
                    let _ = c.close(&t);
                }
                return;
            }
        };
        let res: Result<(), String> = match req {
            Some(Req::Host(settings)) => match game_side(&settings).and_then(|(port, key, lan)| c.host(&settings, port, &key, lan).map_err(|e| explain(&e))) {
                Ok((code, t)) => {
                    host_registered(&t);
                    token = Some(t);
                    with(|s| {
                        s.hosted = Some(Hosted { code, settings, players: 1 });
                        s.hosting = false;
                    });
                    Ok(())
                }
                Err(e) => {
                    // The game socket couldn't open, or the lobby said no: the menus show why, the link stays.
                    with(|s| {
                        s.hosting = false;
                        s.host_error = Some(e);
                    });
                    Ok(())
                }
            },
            Some(Req::Update(settings)) => match &token {
                Some(t) => {
                    let players = with(|s| s.hosted.as_ref().map_or(1, |h| h.players));
                    c.update(t, players, &settings)
                }
                None => Ok(()),
            },
            Some(Req::CloseRoom) => {
                // A room asked for just before closing answers first: it is closed too, so it isn't shown.
                with(|s| s.hosted = None);
                match token.take() {
                    Some(t) => c.close(&t),
                    None => Ok(()),
                }
            }
            Some(Req::List) => {
                let ms = c.ping().unwrap_or(0);
                match c.list() {
                    Ok(v) => {
                        with(|s| {
                            s.rooms = v;
                            s.list = ListState::Ready;
                            s.list_ms = ms;
                        });
                        Ok(())
                    }
                    Err(e) => {
                        with(|s| s.list = ListState::Failed(explain(&e)));
                        Ok(())
                    }
                }
            }
            Some(Req::Join(code)) => {
                let r = c.join(&code).map_err(|e| explain(&e));
                with(|s| {
                    s.join = Some(r);
                    s.joining = false;
                });
                Ok(())
            }
            None => {
                if last_ping.elapsed() > Duration::from_secs(5) {
                    last_ping = Instant::now();
                    c.ping().map(|ms| {
                        with(|s| {
                            if let Status::Online { ms: m, .. } = &mut s.status {
                                *m = ms;
                            }
                        })
                    })
                } else {
                    Ok(())
                }
            }
        };
        for p in c.take_pushed() {
            if let Some(ip) = p.strip_prefix("JOINER ") {
                with(|s| {
                    s.joiners.push(ip.split_whitespace().next().unwrap_or(ip).to_string());
                    if s.joiners.len() > 32 {
                        s.joiners.remove(0);
                    }
                });
            }
        }
        if let Err(e) = res {
            // A refused request (a bad code, a full room) is answered above; anything here lost the connection.
            with(|s| {
                s.status = Status::Failed(format!("Lost online services ({}).", explain(&e)));
                s.tx = None;
                s.hosted = None;
                s.hosting = false;
            });
            return;
        }
    }
}
