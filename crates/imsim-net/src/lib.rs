//! Online services for the game: the lobby server and its client (std only, no engine).
//!
//! The lobby hands out room codes, keeps the list of public rooms and tells a joining player where the host is.
//! It carries no game traffic: players connect to the host directly once they have found it. It is a small
//! line-based protocol over TCP:
//!
//! ```text
//! client                                  server
//! HELLO 1 <game version>             ->   WELCOME 1 <server name>
//! HOST <mode> <public> <max> <look> <tough> <game port> <key> <lan ip:port | -> <name>
//!                                    ->   ROOM <code> <token>      (key = the room's public key, hex)
//! UPDATE <token> <players> <mode> <public> <max> <look> <tough>
//!                                    ->   OK
//! CLOSE <token>                      ->   OK
//! LIST                               ->   R <code> <mode> <look> <players> <max> <tough> <host ms> <name>   (each)
//!                                         END
//! JOIN <code>                        ->   PEER <host ip:port> <mode> <look> <players> <max> <tough> <key>
//!                                              <lan | -> <host udp | -> <ticket>
//!                                         (the host's connection gets JOINER <ip> <ticket>)
//! PING <n>                           ->   PONG <n>
//! anything wrong                     ->   ERR <reason>
//! ```
//!
//! The same port number on UDP finds a way between host and joiner (hole punching, then a relay as a last
//! resort; see `game`). The lobby never sees inside game packets: they are encrypted end to end.
//!
//! A room lives as long as its host's connection (hosts PING every few seconds; silent connections are dropped).
//! Rooms are listed only when public; private rooms are found by code alone. Codes are four characters from an
//! alphabet without look-alikes (no 0 / O, 1 / I).

pub mod client;
#[cfg(feature = "game")]
pub mod game;
pub mod server;
pub mod upnp;

/// The protocol version in HELLO / WELCOME.
pub const PROTOCOL: u32 = 2;
/// The lobby's default TCP port.
pub const DEFAULT_PORT: u16 = 27962;
/// The admin port (localhost only) the server controller talks to.
pub const ADMIN_PORT: u16 = 27963;
/// Longest line either side accepts.
pub const MAX_LINE: usize = 512;
/// Most players a room can hold.
pub const MAX_PLAYERS: u8 = 16;

// UDP packet kinds (see `game` for the whole list). The lobby's UDP port (the same number as its TCP port)
// takes REG / PUNCH / RELAY and sends HOSTAT / JOINAT / SEEN / RELAY.
pub const K_HS_INIT: u8 = 0x01;
pub const K_HS_RESP: u8 = 0x02;
pub const K_DATA: u8 = 0x03;
pub const K_PROBE: u8 = 0x04;
pub const K_RELAY: u8 = 0x05;
pub const K_REG: u8 = 0x10;
pub const K_PUNCH: u8 = 0x11;
pub const K_HOSTAT: u8 = 0x12;
pub const K_JOINAT: u8 = 0x13;
pub const K_SEEN: u8 = 0x14;

/// Room-code characters: letters and digits without 0, O, 1 and I.
pub const CODE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// Whether `s` is a well-formed room code (4 characters from the alphabet, any case).
pub fn valid_code(s: &str) -> bool {
    s.len() == 4 && s.bytes().all(|b| CODE_ALPHABET.contains(&b.to_ascii_uppercase()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Coop,
    Deathmatch,
}

impl Mode {
    pub fn word(self) -> &'static str {
        match self {
            Mode::Coop => "coop",
            Mode::Deathmatch => "dm",
        }
    }
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "coop" => Some(Mode::Coop),
            "dm" => Some(Mode::Deathmatch),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Classic,
    Modern,
}

impl Look {
    pub fn word(self) -> &'static str {
        match self {
            Look::Classic => "classic",
            Look::Modern => "modern",
        }
    }
    pub fn parse(s: &str) -> Option<Look> {
        match s {
            "classic" => Some(Look::Classic),
            "modern" => Some(Look::Modern),
            _ => None,
        }
    }
}

/// What a host chooses for its room.
#[derive(Clone, Debug, PartialEq)]
pub struct RoomSettings {
    pub mode: Mode,
    pub public: bool,
    pub max_players: u8,
    pub look: Look,
    /// Enemies tougher with more players (co-op).
    pub tough: bool,
    /// The room's name in the browser (the host's name).
    pub name: String,
}

impl Default for RoomSettings {
    fn default() -> Self {
        RoomSettings { mode: Mode::Coop, public: true, max_players: 4, look: Look::Modern, tough: false, name: String::new() }
    }
}

/// A room as the browser lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct RoomInfo {
    pub code: String,
    pub mode: Mode,
    pub look: Look,
    pub players: u8,
    pub max_players: u8,
    pub tough: bool,
    /// The host's round trip to the lobby (ms), for the browser's ping estimate.
    pub host_ms: u32,
    pub name: String,
}

/// Names travel as one field: spaces become underscores, anything outside printable ASCII is dropped, at most 24.
pub fn clean_name(s: &str) -> String {
    let n: String = s.chars().filter(|c| c.is_ascii_graphic() || *c == ' ').take(24).collect();
    let n = n.trim().replace(' ', "_");
    if n.is_empty() {
        "Player".to_string()
    } else {
        n
    }
}

/// A name as shown (underscores back to spaces).
pub fn show_name(s: &str) -> String {
    s.replace('_', " ")
}

fn flag(b: bool) -> &'static str {
    if b {
        "1"
    } else {
        "0"
    }
}

impl RoomInfo {
    pub fn line(&self) -> String {
        format!(
            "R {} {} {} {} {} {} {} {}",
            self.code,
            self.mode.word(),
            self.look.word(),
            self.players,
            self.max_players,
            flag(self.tough),
            self.host_ms,
            clean_name(&self.name)
        )
    }

    pub fn parse(line: &str) -> Option<RoomInfo> {
        let mut w = line.split_whitespace();
        if w.next()? != "R" {
            return None;
        }
        Some(RoomInfo {
            code: w.next()?.to_string(),
            mode: Mode::parse(w.next()?)?,
            look: Look::parse(w.next()?)?,
            players: w.next()?.parse().ok()?,
            max_players: w.next()?.parse().ok()?,
            tough: w.next()? == "1",
            host_ms: w.next()?.parse().ok()?,
            name: show_name(w.next().unwrap_or("Player")),
        })
    }
}

/// Settings as HOST / UPDATE send them (after the command word and token).
pub fn settings_fields(s: &RoomSettings) -> String {
    format!(
        "{} {} {} {} {}",
        s.mode.word(),
        flag(s.public),
        s.max_players.clamp(2, MAX_PLAYERS),
        s.look.word(),
        flag(s.tough)
    )
}

/// Parses `mode public max look tough` from a word iterator.
pub fn parse_settings<'a>(w: &mut impl Iterator<Item = &'a str>) -> Option<RoomSettings> {
    let mode = Mode::parse(w.next()?)?;
    let public = w.next()? == "1";
    let max_players: u8 = w.next()?.parse().ok()?;
    if !(2..=MAX_PLAYERS).contains(&max_players) {
        return None;
    }
    let look = Look::parse(w.next()?)?;
    let tough = w.next()? == "1";
    Some(RoomSettings { mode, public, max_players, look, tough, name: String::new() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes() {
        assert!(valid_code("DX12".replace('1', "3").as_str()));
        assert!(valid_code("dx23"));
        assert!(!valid_code("DX10")); // 1 and 0 are left out
        assert!(!valid_code("DX2"));
        assert!(!valid_code("DXO2"));
    }

    #[test]
    fn room_lines_round_trip() {
        let r = RoomInfo {
            code: "DX23".into(),
            mode: Mode::Coop,
            look: Look::Classic,
            players: 3,
            max_players: 8,
            tough: true,
            host_ms: 42,
            name: "JC Denton".into(),
        };
        assert_eq!(RoomInfo::parse(&r.line()), Some(r));
        let s = RoomSettings { mode: Mode::Deathmatch, public: false, max_players: 16, look: Look::Modern, tough: false, name: String::new() };
        let f = settings_fields(&s);
        assert_eq!(parse_settings(&mut f.split_whitespace()), Some(s));
        assert_eq!(clean_name("  Paul \u{e9} Denton!  "), "Paul__Denton!");
    }
}
