//! A blocking lobby client (run it on a thread of its own; every call waits for the server's answer).

use crate::*;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

pub struct Client {
    out: TcpStream,
    rd: BufReader<TcpStream>,
    /// The server's name from WELCOME.
    pub server_name: String,
    ping_n: u64,
    /// The last round trip measured by `ping` (sent with the next one, for hosts: the browser's ping estimate).
    pub last_ms: u32,
    /// Lines the server pushed between answers (JOINER <ip> to a host).
    pushed: Vec<String>,
}

/// A room found by code.
#[derive(Clone, Debug)]
pub struct Joined {
    pub addr: SocketAddr,
    pub mode: Mode,
    pub look: Look,
    pub players: u8,
    pub max_players: u8,
    pub tough: bool,
    /// The room's public key (hex).
    pub key: String,
    /// The host's LAN address and its game socket as the lobby's UDP port saw it, when known.
    pub lan: Option<SocketAddr>,
    pub udp: Option<SocketAddr>,
    /// This join's ticket (punching and the relay).
    pub ticket: u64,
}

/// What went wrong, in words for the player.
pub fn explain(err: &str) -> String {
    match err {
        "noroom" => "No room has that code.".into(),
        "full" => "That room is full.".into(),
        "badcode" => "Room codes are four letters and digits.".into(),
        "busy" => "Online services are busy. Try again in a moment.".into(),
        "version" => "This version of the game can't talk to these online services.".into(),
        "closing" => "Online services were restarted.".into(),
        "key" => "The room couldn't be opened (bad room key).".into(),
        e => e.to_string(),
    }
}

impl Client {
    /// Connect and say hello. `addr` is "host:port" (a name or an IP).
    pub fn connect(addr: &str, timeout: Duration) -> Result<Client, String> {
        let addrs: Vec<SocketAddr> = addr.to_socket_addrs().map_err(|e| format!("can't find {addr}: {e}"))?.collect();
        let mut last = String::from("no address");
        for a in addrs {
            match TcpStream::connect_timeout(&a, timeout) {
                Ok(s) => {
                    let _ = s.set_nodelay(true);
                    let _ = s.set_read_timeout(Some(timeout));
                    let _ = s.set_write_timeout(Some(timeout));
                    let rd = BufReader::new(s.try_clone().map_err(|e| e.to_string())?);
                    let mut c = Client { out: s, rd, server_name: String::new(), ping_n: 0, last_ms: 0, pushed: Vec::new() };
                    c.send(&format!("HELLO {PROTOCOL} {}", env!("CARGO_PKG_VERSION")))?;
                    let w = c.answer()?;
                    let mut it = w.split_whitespace();
                    if it.next() != Some("WELCOME") {
                        return Err(format!("unexpected answer: {w}"));
                    }
                    let _ = it.next();
                    c.server_name = show_name(it.next().unwrap_or(""));
                    return Ok(c);
                }
                Err(e) => last = e.to_string(),
            }
        }
        Err(last)
    }

    fn send(&mut self, line: &str) -> Result<(), String> {
        let mut b = line.as_bytes().to_vec();
        b.push(b'\n');
        self.out.write_all(&b).map_err(|e| e.to_string())
    }

    fn read_line(&mut self) -> Result<String, String> {
        let mut s = String::new();
        match self.rd.read_line(&mut s) {
            Ok(0) => Err("the connection closed".into()),
            Ok(_) => Ok(s.trim().to_string()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// The next answer (pushed lines are put aside); ERR becomes Err(reason).
    fn answer(&mut self) -> Result<String, String> {
        loop {
            let l = self.read_line()?;
            if l.starts_with("JOINER ") {
                self.pushed.push(l);
                continue;
            }
            if let Some(e) = l.strip_prefix("ERR ") {
                return Err(e.to_string());
            }
            return Ok(l);
        }
    }

    /// Open a room: (code, token).
    /// `key`: the room's public key (hex); `lan`: the host's game address on its LAN.
    pub fn host(&mut self, s: &RoomSettings, game_port: u16, key: &str, lan: Option<SocketAddr>) -> Result<(String, String), String> {
        let lan = lan.map_or("-".to_string(), |a| a.to_string());
        self.send(&format!("HOST {} {} {key} {lan} {}", settings_fields(s), game_port, clean_name(&s.name)))?;
        let a = self.answer()?;
        let mut w = a.split_whitespace();
        match (w.next(), w.next(), w.next()) {
            (Some("ROOM"), Some(code), Some(token)) => Ok((code.to_string(), token.to_string())),
            _ => Err(format!("unexpected answer: {a}")),
        }
    }

    pub fn update(&mut self, token: &str, players: u8, s: &RoomSettings) -> Result<(), String> {
        self.send(&format!("UPDATE {token} {players} {}", settings_fields(s)))?;
        self.answer().map(|_| ())
    }

    pub fn close(&mut self, token: &str) -> Result<(), String> {
        self.send(&format!("CLOSE {token}"))?;
        self.answer().map(|_| ())
    }

    pub fn list(&mut self) -> Result<Vec<RoomInfo>, String> {
        self.send("LIST")?;
        let mut v = Vec::new();
        loop {
            let l = self.answer()?;
            if l == "END" {
                return Ok(v);
            }
            if let Some(r) = RoomInfo::parse(&l) {
                v.push(r);
            }
        }
    }

    pub fn join(&mut self, code: &str) -> Result<Joined, String> {
        if !valid_code(code) {
            return Err("badcode".into());
        }
        self.send(&format!("JOIN {}", code.to_ascii_uppercase()))?;
        let a = self.answer()?;
        let mut w = a.split_whitespace();
        if w.next() != Some("PEER") {
            return Err(format!("unexpected answer: {a}"));
        }
        let addr: SocketAddr = w.next().and_then(|x| x.parse().ok()).ok_or("bad address")?;
        let mode = w.next().and_then(Mode::parse).ok_or("bad mode")?;
        let look = w.next().and_then(Look::parse).ok_or("bad look")?;
        let players = w.next().and_then(|x| x.parse().ok()).unwrap_or(1);
        let max_players = w.next().and_then(|x| x.parse().ok()).unwrap_or(MAX_PLAYERS);
        let tough = w.next() == Some("1");
        let key = w.next().unwrap_or("").to_string();
        let lan = w.next().and_then(|x| x.parse().ok());
        let udp = w.next().and_then(|x| x.parse().ok());
        let ticket = w.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        Ok(Joined { addr, mode, look, players, max_players, tough, key, lan, udp, ticket })
    }

    /// A round trip to the lobby (also keeps the connection alive): milliseconds.
    pub fn ping(&mut self) -> Result<u32, String> {
        self.ping_n += 1;
        let t = Instant::now();
        let n = self.ping_n;
        self.send(&format!("PING {n} {}", self.last_ms))?;
        loop {
            let a = self.answer()?;
            if a == format!("PONG {n}") {
                break;
            }
        }
        self.last_ms = t.elapsed().as_millis().min(9999) as u32;
        Ok(self.last_ms)
    }

    /// Lines pushed by the server since the last call (JOINER <ip> for a host).
    pub fn take_pushed(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pushed)
    }
}
