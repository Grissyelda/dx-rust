//! The lobby server (see the crate docs for the protocol). One thread per connection: the lobby sees a handful of
//! players at a time, and every connection is a few short lines a second at most.
//!
//! Safety: lines are capped at MAX_LINE, each connection may send at most RATE_LIMIT lines a second and is
//! dropped after IDLE_SECS of silence, an address may hold MAX_PER_IP connections, a connection may host one room
//! and only with the room's secret token can it be changed or closed. The admin port answers only on localhost and
//! only after the key from the key file.

use crate::*;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const RATE_LIMIT: u32 = 30;
const IDLE_SECS: u64 = 30;
const MAX_PER_IP: usize = 8;
const MAX_CONNS: usize = 512;
const MAX_ROOMS: usize = 1024;

pub struct Config {
    pub port: u16,
    pub admin_port: u16,
    /// Shown to players in WELCOME.
    pub name: String,
    /// The log file (appended), if any.
    pub log: Option<PathBuf>,
    /// The admin key (the controller reads the same file).
    pub admin_key: String,
}

struct Room {
    settings: RoomSettings,
    token: String,
    host: SocketAddr,
    game_port: u16,
    players: u8,
    host_ms: u32,
    owner: u64,
    /// Pushes to the host's connection (JOINER lines).
    to_host: Sender<String>,
    created: Instant,
    /// The room's public key (hex), for joiners to check the host with.
    key: String,
    /// The host's LAN address ("-" when unknown).
    lan: String,
    /// The host's game socket as the lobby's UDP port sees it (after REG).
    udp: Option<SocketAddr>,
}

/// One join: lets the joiner and the host find each other over UDP and, failing that, relay through the lobby.
struct Ticket {
    code: String,
    joiner: Option<SocketAddr>,
    created: Instant,
    window: Instant,
    bytes: usize,
}

/// Relay bytes per ticket per second at most (a co-op link uses a few dozen KB/s).
const RELAY_RATE: usize = 512 * 1024;
const MAX_TICKETS: usize = 8192;

#[derive(Default)]
struct Lobby {
    rooms: HashMap<String, Room>,
    per_ip: HashMap<IpAddr, usize>,
    conns: usize,
    total_conns: u64,
    joins: u64,
    hosted: u64,
    tickets: HashMap<u64, Ticket>,
    relayed: u64,
}

struct Shared {
    lobby: Mutex<Lobby>,
    stop: AtomicBool,
    /// Bumped by RESET: every connection made before it closes.
    reset: AtomicU64,
    started: Instant,
    seq: AtomicU64,
    cfg: Config,
    log: Mutex<Option<std::fs::File>>,
}

impl Shared {
    fn log(&self, msg: &str) {
        let t = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let line = format!("{t} {msg}\n");
        if let Ok(mut f) = self.log.lock() {
            if let Some(f) = f.as_mut() {
                let _ = f.write_all(line.as_bytes());
            }
        }
        eprint!("{line}");
    }
}

/// A fresh random value: the standard library's per-process random SipHash keys over a counter and the clock.
fn random_u64(sh: &Shared) -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(sh.seq.fetch_add(1, Ordering::Relaxed));
    h.write_u128(SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    h.finish()
}

fn new_code(sh: &Shared, lobby: &Lobby) -> Option<String> {
    for _ in 0..200 {
        let mut v = random_u64(sh);
        let code: String = (0..4)
            .map(|_| {
                let c = CODE_ALPHABET[(v % 32) as usize] as char;
                v /= 32;
                c
            })
            .collect();
        if !lobby.rooms.contains_key(&code) {
            return Some(code);
        }
    }
    None
}

/// Run the lobby until STOP from the admin port (blocks).
pub fn run(cfg: Config) -> std::io::Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", cfg.port))?;
    let admin = TcpListener::bind(("127.0.0.1", cfg.admin_port))?;
    let log = cfg
        .log
        .as_ref()
        .and_then(|p| std::fs::OpenOptions::new().create(true).append(true).open(p).ok());
    let sh = Arc::new(Shared {
        lobby: Mutex::new(Lobby::default()),
        stop: AtomicBool::new(false),
        reset: AtomicU64::new(0),
        started: Instant::now(),
        seq: AtomicU64::new(1),
        cfg,
        log: Mutex::new(log),
    });
    sh.log(&format!("lobby up on port {} TCP + UDP (admin {} on localhost)", sh.cfg.port, sh.cfg.admin_port));
    match UdpSocket::bind(("0.0.0.0", sh.cfg.port)) {
        Ok(u) => {
            let sh = sh.clone();
            std::thread::spawn(move || udp_loop(&sh, u));
        }
        Err(e) => sh.log(&format!("UDP port {} unavailable ({e}): players will need direct connections", sh.cfg.port)),
    }
    {
        let sh = sh.clone();
        std::thread::spawn(move || {
            for s in admin.incoming().flatten() {
                let sh = sh.clone();
                std::thread::spawn(move || admin_conn(&sh, s));
            }
        });
    }
    listener.set_nonblocking(true)?;
    while !sh.stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((s, addr)) => {
                let ok = {
                    let mut l = sh.lobby.lock().unwrap_or_else(|e| e.into_inner());
                    let n = l.per_ip.get(&addr.ip()).copied().unwrap_or(0);
                    if l.conns >= MAX_CONNS || n >= MAX_PER_IP {
                        false
                    } else {
                        l.conns += 1;
                        l.total_conns += 1;
                        *l.per_ip.entry(addr.ip()).or_insert(0) += 1;
                        true
                    }
                };
                if !ok {
                    let mut s = s;
                    let _ = s.write_all(b"ERR busy\n");
                    continue;
                }
                let sh = sh.clone();
                std::thread::spawn(move || {
                    let id = sh.seq.fetch_add(1, Ordering::Relaxed);
                    conn(&sh, s, addr, id);
                    let mut l = sh.lobby.lock().unwrap_or_else(|e| e.into_inner());
                    l.conns = l.conns.saturating_sub(1);
                    if let Some(n) = l.per_ip.get_mut(&addr.ip()) {
                        *n = n.saturating_sub(1);
                        if *n == 0 {
                            l.per_ip.remove(&addr.ip());
                        }
                    }
                    let closed: Vec<String> = l.rooms.iter().filter(|(_, r)| r.owner == id).map(|(c, _)| c.clone()).collect();
                    for c in closed {
                        l.rooms.remove(&c);
                        drop(l);
                        sh.log(&format!("room {c} closed (host left)"));
                        l = sh.lobby.lock().unwrap_or_else(|e| e.into_inner());
                    }
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => {
                sh.log(&format!("accept failed: {e}"));
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
    sh.log("lobby stopped");
    Ok(())
}

fn send(s: &mut TcpStream, line: &str) -> bool {
    let mut b = line.as_bytes().to_vec();
    b.push(b'\n');
    s.write_all(&b).is_ok()
}

/// One player's connection.
fn conn(sh: &Shared, s: TcpStream, addr: SocketAddr, id: u64) {
    let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = s.set_nodelay(true);
    let Ok(mut out) = s.try_clone() else { return };
    let mut rd = BufReader::new(s);
    let gen = sh.reset.load(Ordering::Relaxed);
    let (tx, rx): (Sender<String>, Receiver<String>) = channel();
    let mut last = Instant::now();
    let mut window = Instant::now();
    let mut lines_in_window = 0u32;
    let mut greeted = false;
    let mut buf = Vec::new();
    loop {
        if sh.stop.load(Ordering::Relaxed) || sh.reset.load(Ordering::Relaxed) != gen {
            let _ = send(&mut out, "ERR closing");
            return;
        }
        while let Ok(p) = rx.try_recv() {
            if !send(&mut out, &p) {
                return;
            }
        }
        if last.elapsed() > Duration::from_secs(IDLE_SECS) {
            let _ = send(&mut out, "ERR idle");
            return;
        }
        // (A timeout keeps what has arrived of a line in `buf`; the next read goes on from there.)
        match (&mut rd).take(MAX_LINE as u64 + 1).read_until(b'\n', &mut buf) {
            Ok(0) => return,
            Ok(_) => {}
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                if buf.len() > MAX_LINE {
                    let _ = send(&mut out, "ERR line");
                    return;
                }
                continue;
            }
            Err(_) => return,
        }
        if buf.len() > MAX_LINE {
            let _ = send(&mut out, "ERR line");
            return;
        }
        if !buf.ends_with(b"\n") {
            continue;
        }
        let taken = std::mem::take(&mut buf);
        last = Instant::now();
        if window.elapsed() >= Duration::from_secs(1) {
            window = Instant::now();
            lines_in_window = 0;
        }
        lines_in_window += 1;
        if lines_in_window > RATE_LIMIT {
            let _ = send(&mut out, "ERR slow");
            return;
        }
        let Ok(line) = std::str::from_utf8(&taken) else {
            let _ = send(&mut out, "ERR utf8");
            return;
        };
        let line = line.trim();
        let mut w = line.split_whitespace();
        let cmd = w.next().unwrap_or("");
        if !greeted && cmd != "HELLO" {
            let _ = send(&mut out, "ERR hello");
            return;
        }
        let reply = match cmd {
            "HELLO" => {
                if w.next().and_then(|v| v.parse::<u32>().ok()) != Some(PROTOCOL) {
                    let _ = send(&mut out, "ERR version");
                    return;
                }
                greeted = true;
                format!("WELCOME {PROTOCOL} {}", clean_name(&sh.cfg.name))
            }
            "PING" => {
                let n = w.next().unwrap_or("0").to_string();
                if let Some(ms) = w.next().and_then(|v| v.parse::<u32>().ok()) {
                    let mut l = sh.lobby.lock().unwrap_or_else(|e| e.into_inner());
                    for r in l.rooms.values_mut().filter(|r| r.owner == id) {
                        r.host_ms = ms.min(9999);
                    }
                }
                format!("PONG {n}")
            }
            "HOST" => host(sh, &mut w, addr, id, &tx),
            "UPDATE" => update(sh, &mut w, id),
            "CLOSE" => {
                let token = w.next().unwrap_or("");
                let mut l = sh.lobby.lock().unwrap_or_else(|e| e.into_inner());
                let code = l.rooms.iter().find(|(_, r)| r.token == token && r.owner == id).map(|(c, _)| c.clone());
                match code {
                    Some(c) => {
                        l.rooms.remove(&c);
                        drop(l);
                        sh.log(&format!("room {c} closed"));
                        "OK".to_string()
                    }
                    None => "ERR noroom".to_string(),
                }
            }
            "LIST" => {
                let l = sh.lobby.lock().unwrap_or_else(|e| e.into_inner());
                let mut v: Vec<(&String, &Room)> = l.rooms.iter().filter(|(_, r)| r.settings.public).collect();
                v.sort_by_key(|(_, r)| r.created);
                let mut s = String::new();
                for (code, r) in v.iter().take(200) {
                    let info = RoomInfo {
                        code: (*code).clone(),
                        mode: r.settings.mode,
                        look: r.settings.look,
                        players: r.players,
                        max_players: r.settings.max_players,
                        tough: r.settings.tough,
                        host_ms: r.host_ms,
                        name: r.settings.name.clone(),
                        detail: r.settings.detail.clone(),
                    };
                    s.push_str(&info.line());
                    s.push('\n');
                }
                s.push_str("END");
                s
            }
            "JOIN" => {
                let code = w.next().unwrap_or("").to_ascii_uppercase();
                if !valid_code(&code) {
                    "ERR badcode".to_string()
                } else {
                    let mut l = sh.lobby.lock().unwrap_or_else(|e| e.into_inner());
                    let found = l.rooms.get(&code).map(|r| {
                        (SocketAddr::new(r.host.ip(), r.game_port), r.settings.clone(), r.players, r.key.clone(), r.lan.clone(), r.udp, r.to_host.clone())
                    });
                    match found {
                        Some((_, s, p, ..)) if p >= s.max_players => "ERR full".to_string(),
                        Some((peer, s, p, key, lan, udp, to_host)) => {
                            l.joins += 1;
                            let ticket = loop {
                                let t = random_u64(sh) | 1;
                                if !l.tickets.contains_key(&t) {
                                    break t;
                                }
                            };
                            if l.tickets.len() >= MAX_TICKETS {
                                let oldest = l.tickets.iter().min_by_key(|(_, t)| t.created).map(|(k, _)| *k);
                                if let Some(k) = oldest {
                                    l.tickets.remove(&k);
                                }
                            }
                            let now = Instant::now();
                            l.tickets.insert(ticket, Ticket { code: code.clone(), joiner: None, created: now, window: now, bytes: 0 });
                            drop(l);
                            let _ = to_host.send(format!("JOINER {} {ticket}", addr.ip()));
                            sh.log(&format!("join {code} from {}", addr.ip()));
                            format!(
                                "PEER {peer} {} {} {} {} {} {key} {lan} {} {ticket}",
                                s.mode.word(),
                                s.look.word(),
                                p,
                                s.max_players,
                                flag(s.tough),
                                udp.map_or("-".to_string(), |u| u.to_string())
                            )
                        }
                        None => "ERR noroom".to_string(),
                    }
                }
            }
            _ => "ERR command".to_string(),
        };
        if !send(&mut out, &reply) {
            return;
        }
    }
}

fn flag(b: bool) -> &'static str {
    if b {
        "1"
    } else {
        "0"
    }
}

fn host<'a>(sh: &Shared, w: &mut impl Iterator<Item = &'a str>, addr: SocketAddr, id: u64, tx: &Sender<String>) -> String {
    let Some(mut settings) = parse_settings(w) else { return "ERR settings".to_string() };
    let Some(game_port) = w.next().and_then(|p| p.parse::<u16>().ok()) else { return "ERR port".to_string() };
    let key = w.next().unwrap_or("").to_ascii_lowercase();
    if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
        return "ERR key".to_string();
    }
    let lan = match w.next() {
        Some(a) if a.parse::<SocketAddr>().is_ok() => a.to_string(),
        _ => "-".to_string(),
    };
    settings.name = clean_name(&w.collect::<Vec<_>>().join(" "));
    let mut l = sh.lobby.lock().unwrap_or_else(|e| e.into_inner());
    if l.rooms.values().any(|r| r.owner == id) {
        return "ERR hosting".to_string();
    }
    if l.rooms.len() >= MAX_ROOMS {
        return "ERR full".to_string();
    }
    let Some(code) = new_code(sh, &l) else { return "ERR full".to_string() };
    let token = format!("{:016x}{:016x}", random_u64(sh), random_u64(sh));
    let line = format!("ROOM {code} {token}");
    let desc = format!("room {code} hosted by {} ({} {} {}/{}{})", addr.ip(), settings.mode.word(), settings.look.word(), 1, settings.max_players, if settings.public { "" } else { ", private" });
    l.rooms.insert(
        code,
        Room {
            settings,
            token,
            host: addr,
            game_port,
            players: 1,
            host_ms: 0,
            owner: id,
            to_host: tx.clone(),
            created: Instant::now(),
            key,
            lan,
            udp: None,
        },
    );
    l.hosted += 1;
    drop(l);
    sh.log(&desc);
    line
}

fn update<'a>(sh: &Shared, w: &mut impl Iterator<Item = &'a str>, id: u64) -> String {
    let token = w.next().unwrap_or("").to_string();
    let Some(players) = w.next().and_then(|p| p.parse::<u8>().ok()) else { return "ERR players".to_string() };
    let Some(settings) = parse_settings(w) else { return "ERR settings".to_string() };
    let mut l = sh.lobby.lock().unwrap_or_else(|e| e.into_inner());
    match l.rooms.values_mut().find(|r| r.token == token && r.owner == id) {
        Some(r) => {
            let name = std::mem::take(&mut r.settings.name);
            r.settings = RoomSettings { name, ..settings };
            r.players = players.clamp(1, r.settings.max_players);
            "OK".to_string()
        }
        None => "ERR noroom".to_string(),
    }
}

/// The UDP side: hosts say where their game socket is (REG), joiners ask for it (PUNCH) and both sides are told
/// to send to each other; RELAY packets pass between the two ends of a ticket (encrypted end to end).
fn udp_loop(sh: &Shared, u: UdpSocket) {
    let _ = u.set_read_timeout(Some(Duration::from_millis(250)));
    let mut buf = vec![0u8; 2048];
    let mut prune = Instant::now();
    while !sh.stop.load(Ordering::Relaxed) {
        let (n, from) = match u.recv_from(&mut buf) {
            Ok(x) => x,
            Err(_) => continue,
        };
        if n == 0 || n > 1400 {
            continue;
        }
        let p = &buf[..n];
        let mut l = sh.lobby.lock().unwrap_or_else(|e| e.into_inner());
        if prune.elapsed() > Duration::from_secs(30) {
            prune = Instant::now();
            let Lobby { rooms, tickets, .. } = &mut *l;
            tickets.retain(|_, t| rooms.contains_key(&t.code));
        }
        match p[0] {
            K_REG => {
                let Ok(token) = std::str::from_utf8(&p[1..]) else { continue };
                if let Some(r) = l.rooms.values_mut().find(|r| r.token == token) {
                    r.udp = Some(from);
                    let mut v = vec![K_SEEN];
                    v.extend_from_slice(from.to_string().as_bytes());
                    let _ = u.send_to(&v, from);
                }
            }
            K_PUNCH if n == 9 => {
                let t = u64::from_le_bytes(p[1..9].try_into().unwrap());
                let Lobby { rooms, tickets, .. } = &mut *l;
                let Some(tk) = tickets.get_mut(&t) else { continue };
                if tk.joiner.is_some_and(|j| j.ip() != from.ip()) {
                    continue;
                }
                tk.joiner = Some(from);
                if let Some(h) = rooms.get(&tk.code).and_then(|r| r.udp) {
                    let mut v = vec![K_HOSTAT];
                    v.extend_from_slice(&t.to_le_bytes());
                    v.extend_from_slice(h.to_string().as_bytes());
                    let _ = u.send_to(&v, from);
                    let mut v = vec![K_JOINAT];
                    v.extend_from_slice(&t.to_le_bytes());
                    v.extend_from_slice(from.to_string().as_bytes());
                    let _ = u.send_to(&v, h);
                }
            }
            K_RELAY if n > 9 => {
                let t = u64::from_le_bytes(p[1..9].try_into().unwrap());
                let Lobby { rooms, tickets, relayed, .. } = &mut *l;
                let Some(tk) = tickets.get_mut(&t) else { continue };
                let Some(h) = rooms.get(&tk.code).and_then(|r| r.udp) else { continue };
                if tk.window.elapsed() >= Duration::from_secs(1) {
                    tk.window = Instant::now();
                    tk.bytes = 0;
                }
                tk.bytes += n;
                if tk.bytes > RELAY_RATE {
                    continue;
                }
                let to = if from == h {
                    match tk.joiner {
                        Some(j) => j,
                        None => continue,
                    }
                } else {
                    if tk.joiner.is_some_and(|j| j.ip() != from.ip()) {
                        continue;
                    }
                    tk.joiner = Some(from);
                    h
                };
                *relayed += n as u64;
                let _ = u.send_to(p, to);
            }
            _ => {}
        }
    }
}

/// The controller's connection (localhost only): KEY first, then STATUS / ROOMS / RESET / STOP.
fn admin_conn(sh: &Shared, s: TcpStream) {
    if !s.peer_addr().is_ok_and(|a| a.ip().is_loopback()) {
        return;
    }
    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    let Ok(mut out) = s.try_clone() else { return };
    let rd = BufReader::new(s);
    let mut authed = false;
    for line in rd.lines() {
        let Ok(line) = line else { return };
        if line.len() > MAX_LINE {
            return;
        }
        let mut w = line.split_whitespace();
        let cmd = w.next().unwrap_or("");
        let reply = if !authed {
            if cmd == "KEY" && w.next() == Some(sh.cfg.admin_key.as_str()) {
                authed = true;
                "OK".to_string()
            } else {
                let _ = send(&mut out, "ERR key");
                return;
            }
        } else {
            match cmd {
                "STATUS" => {
                    let l = sh.lobby.lock().unwrap_or_else(|e| e.into_inner());
                    format!(
                        "STATUS up={} port={} rooms={} public={} conns={} total={} hosted={} joins={} relayed={}",
                        sh.started.elapsed().as_secs(),
                        sh.cfg.port,
                        l.rooms.len(),
                        l.rooms.values().filter(|r| r.settings.public).count(),
                        l.conns,
                        l.total_conns,
                        l.hosted,
                        l.joins,
                        l.relayed
                    )
                }
                "ROOMS" => {
                    let l = sh.lobby.lock().unwrap_or_else(|e| e.into_inner());
                    let mut s = String::new();
                    for (c, r) in &l.rooms {
                        s.push_str(&format!(
                            "R {c} {} {} {}/{} {} {} {}s {}\n",
                            r.settings.mode.word(),
                            r.settings.look.word(),
                            r.players,
                            r.settings.max_players,
                            if r.settings.public { "public" } else { "private" },
                            r.host.ip(),
                            r.created.elapsed().as_secs(),
                            clean_name(&r.settings.name)
                        ));
                    }
                    s.push_str("END");
                    s
                }
                "RESET" => {
                    sh.reset.fetch_add(1, Ordering::Relaxed);
                    let n = {
                        let mut l = sh.lobby.lock().unwrap_or_else(|e| e.into_inner());
                        let n = l.rooms.len();
                        l.rooms.clear();
                        n
                    };
                    sh.log(&format!("reset: {n} rooms cleared, every player disconnected"));
                    "OK".to_string()
                }
                "STOP" => {
                    sh.stop.store(true, Ordering::Relaxed);
                    let _ = send(&mut out, "OK");
                    return;
                }
                _ => "ERR command".to_string(),
            }
        };
        if !send(&mut out, &reply) {
            return;
        }
    }
}

/// The admin key in `path`, made (random) if the file doesn't exist.
pub fn admin_key(path: &std::path::Path) -> String {
    if let Ok(k) = std::fs::read_to_string(path) {
        let k = k.trim().to_string();
        if k.len() >= 16 {
            return k;
        }
    }
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    let a = h.finish();
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(a ^ std::process::id() as u64);
    let k = format!("{:016x}{:016x}", a, h.finish());
    let _ = std::fs::write(path, &k);
    k
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::Client;

    #[test]
    fn host_list_join_close() {
        let port = 39000 + (std::process::id() % 2000) as u16;
        let admin = port + 1;
        let key = "test-key-0123456789".to_string();
        std::thread::spawn(move || {
            let _ = run(Config { port, admin_port: admin, name: "Test".into(), log: None, admin_key: key });
        });
        std::thread::sleep(Duration::from_millis(300));
        let addr = format!("127.0.0.1:{port}");
        let mut host = Client::connect(&addr, Duration::from_secs(2)).expect("host connects");
        let s = RoomSettings { mode: Mode::Coop, public: true, max_players: 4, look: Look::Classic, tough: true, name: "JC Denton".into(), detail: String::new() };
        let key = "ab".repeat(32);
        let (code, token) = host.host(&s, 7790, &key, None).expect("room");
        assert!(valid_code(&code));
        let mut guest = Client::connect(&addr, Duration::from_secs(2)).expect("guest connects");
        let rooms = guest.list().expect("list");
        assert!(rooms.iter().any(|r| r.code == code && r.name == "JC Denton" && r.tough && r.look == Look::Classic));
        let j = guest.join(&code.to_ascii_lowercase()).expect("join");
        assert_eq!(j.addr.port(), 7790);
        assert_eq!(j.key, key);
        assert!(j.ticket != 0);
        assert!(guest.join("ZZZZ").is_err());
        // Private rooms aren't listed but can be joined by code.
        let p = RoomSettings { public: false, ..s.clone() };
        host.update(&token, 2, &p).expect("update");
        assert!(!guest.list().unwrap().iter().any(|r| r.code == code));
        assert!(guest.join(&code).is_ok());
        host.close(&token).expect("close");
        assert!(guest.join(&code).is_err());
        assert!(guest.ping().is_ok());
    }
}
