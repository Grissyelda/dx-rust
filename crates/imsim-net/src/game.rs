//! Game traffic between players (feature `game`): UDP, end-to-end encrypted, host-authoritative star (every
//! joiner talks only to the host).
//!
//! Safety: each link is a Noise NK session (`Noise_NK_25519_ChaChaPoly_BLAKE2s`). The host makes a fresh key
//! pair for every room and publishes the public half through the lobby, so a joiner only completes a handshake
//! with the real host, and nobody on the way (the lobby's relay included) can read or change the traffic.
//! Every packet carries its own nonce; a sliding window throws away replays. Handshakes are rate limited per
//! address and cost the host nothing until the joiner's first message decrypts.
//!
//! Latency: the host and joiner reach each other directly when they can: the joiner tries the host's LAN
//! address, the address the lobby saw and the UDP address the lobby punched open (both sides send at once, so
//! most home routers let the packets through). Only when nothing answers within RELAY_AFTER does the link go
//! through the lobby's UDP relay (still encrypted end to end).
//!
//! Packets (first byte):
//! ```text
//! 0x01 HS_INIT  u32 joiner sid, Noise message 1
//! 0x02 HS_RESP  u32 joiner sid, u32 host sid, Noise message 2
//! 0x03 DATA     u32 receiver sid, u64 nonce, ciphertext
//! 0x04 PROBE    (opens a NAT mapping; ignored)
//! 0x05 RELAY    u64 ticket, inner packet           (to / from the lobby)
//! 0x10 REG      room token                         (host -> lobby: here is my UDP address)
//! 0x11 PUNCH    u64 ticket                         (joiner -> lobby)
//! 0x12 HOSTAT   u64 ticket, "ip:port"              (lobby -> joiner: the host's UDP address)
//! 0x13 JOINAT   u64 ticket, "ip:port"              (lobby -> host: probe this joiner)
//! 0x14 SEEN     "ip:port"                          (lobby -> host: your public UDP address)
//! ```
//! Inside DATA (plaintext): u16 packet seq, u16 latest seq received, u32 bits for the 32 before it, then records:
//! `1 u16 id u16 len bytes` (reliable, ordered; bytes = flag byte (1 = last piece) + piece), `2 u16 len bytes`
//! (unreliable), `3 u32 ms` ping, `4 u32 ms` pong, `5` goodbye.

use snow::{Builder, HandshakeState, StatelessTransportState};
use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

pub const NOISE: &str = "Noise_NK_25519_ChaChaPoly_BLAKE2s";
/// Largest UDP payload sent (fits every path's MTU, relay header included).
pub const MTU: usize = 1200;
const RELAY_HDR: usize = 9;
const DATA_HDR: usize = 1 + 4 + 8;
const TAG: usize = 16;
/// Plaintext bytes per packet.
const PLAIN_MAX: usize = MTU - RELAY_HDR - DATA_HDR - TAG;
/// Largest piece of a reliable message.
const PIECE: usize = 1000;
/// Largest reassembled reliable message.
pub const MAX_MESSAGE: usize = 4 * 1024 * 1024;
/// Silence that ends a link.
pub const TIMEOUT: Duration = Duration::from_secs(15);
/// No answer from any direct address for this long: go through the relay too.
pub const RELAY_AFTER: Duration = Duration::from_millis(2500);
/// Joining gives up after this long.
pub const JOIN_TIMEOUT: Duration = Duration::from_secs(14);
const PING_EVERY: Duration = Duration::from_millis(500);
/// Reliable pieces waiting for an ack before the link counts as broken.
const MAX_UNACKED: usize = 8192;
/// Bytes sent per flush at most (a burst cap; the rest waits for the next flush).
const FLUSH_BYTES: usize = 96 * 1024;

pub use crate::{K_DATA, K_HOSTAT, K_HS_INIT, K_HS_RESP, K_JOINAT, K_PROBE, K_PUNCH, K_REG, K_RELAY, K_SEEN};

/// A player on the link: 0 is the host, joiners are 1..=15.
pub type PeerId = u8;

/// A fresh key pair (private, public) for a room.
pub fn keypair() -> ([u8; 32], [u8; 32]) {
    let k = Builder::new(NOISE.parse().expect("noise params")).generate_keypair().expect("keypair");
    let mut a = [0u8; 32];
    let mut b = [0u8; 32];
    a.copy_from_slice(&k.private);
    b.copy_from_slice(&k.public);
    (a, b)
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut o = [0u8; 32];
    for (i, o) in o.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(o)
}

/// The address other machines on the LAN reach this one at (no packet is sent to find it).
pub fn lan_ip() -> Option<IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

fn rand_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(N.fetch_add(1, Ordering::Relaxed));
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    h.finish()
}

fn seq_gt(a: u16, b: u16) -> bool {
    (a.wrapping_sub(b) as i16) > 0
}

// ---------------------------------------------------------------------------------------------------------------
// Reliability (no crypto, no sockets: testable on its own).

struct Out {
    id: u16,
    data: Vec<u8>,
    sent_at: Option<Instant>,
}

/// Ordered reliable + unreliable messages over lossy packets.
pub struct Channel {
    next_id: u16,
    out: VecDeque<Out>,
    unrel: Vec<Vec<u8>>,
    in_next: u16,
    in_buf: HashMap<u16, Vec<u8>>,
    assembling: Vec<u8>,
    seq: u16,
    /// Reliable ids per packet seq (by seq % 1024).
    sent: Vec<Option<(u16, Vec<u16>)>>,
    recv_latest: Option<u16>,
    recv_bits: u32,
    ack_due: bool,
    /// Smoothed round trip (ms).
    pub rtt: f32,
    last_ping: Option<Instant>,
    pings: u32,
    pong_due: Option<u32>,
    pub bye: bool,
    bye_sent: bool,
    /// Bytes sent / received (plaintext), for the network readout.
    pub bytes_out: u64,
    pub bytes_in: u64,
    epoch: Instant,
}

impl Default for Channel {
    fn default() -> Self {
        Channel::new()
    }
}

impl Channel {
    pub fn new() -> Channel {
        Channel {
            next_id: 0,
            out: VecDeque::new(),
            unrel: Vec::new(),
            in_next: 0,
            in_buf: HashMap::new(),
            assembling: Vec::new(),
            seq: 0,
            sent: (0..1024).map(|_| None).collect(),
            recv_latest: None,
            recv_bits: 0,
            ack_due: false,
            rtt: 100.0,
            last_ping: None,
            pings: 0,
            pong_due: None,
            bye: false,
            bye_sent: false,
            bytes_out: 0,
            bytes_in: 0,
            epoch: Instant::now(),
        }
    }

    pub fn send_reliable(&mut self, data: &[u8]) {
        let mut pieces = data.chunks(PIECE).peekable();
        if data.is_empty() {
            self.push_piece(&[1]);
            return;
        }
        while let Some(p) = pieces.next() {
            let mut v = Vec::with_capacity(p.len() + 1);
            v.push(if pieces.peek().is_none() { 1 } else { 0 });
            v.extend_from_slice(p);
            self.push_piece(&v);
        }
    }

    fn push_piece(&mut self, v: &[u8]) {
        self.out.push_back(Out { id: self.next_id, data: v.to_vec(), sent_at: None });
        self.next_id = self.next_id.wrapping_add(1);
    }

    /// Unreliable: sent with the next flush or dropped (larger than a packet: sent reliably instead).
    pub fn send_unreliable(&mut self, data: &[u8]) {
        if data.len() + 3 > PLAIN_MAX - 8 {
            self.send_reliable(data);
        } else {
            self.unrel.push(data.to_vec());
        }
    }

    /// Too much unacknowledged: the other side isn't keeping up.
    pub fn broken(&self) -> bool {
        self.out.len() > MAX_UNACKED
    }

    pub fn unacked(&self) -> usize {
        self.out.len()
    }

    fn ms(&self, now: Instant) -> u32 {
        now.duration_since(self.epoch).as_millis() as u32
    }

    /// The packets to send now (plaintexts).
    pub fn flush(&mut self, now: Instant) -> Vec<Vec<u8>> {
        let mut packets = Vec::new();
        let resend = Duration::from_millis((self.rtt * 1.5 + 25.0).clamp(60.0, 1000.0) as u64);
        let mut budget = FLUSH_BYTES;
        let mut cur: Option<(Vec<u8>, Vec<u16>)> = None;
        let mut ping = None;
        if self.last_ping.is_none_or(|t| now.duration_since(t) >= PING_EVERY) {
            self.last_ping = Some(now);
            self.pings += 1;
            ping = Some(self.ms(now));
        }
        let pong = self.pong_due.take();
        let bye = self.bye && !self.bye_sent;
        if bye {
            self.bye_sent = true;
        }
        let finish = |cur: &mut Option<(Vec<u8>, Vec<u16>)>, packets: &mut Vec<Vec<u8>>, me: &mut Channel| {
            if let Some((mut p, ids)) = cur.take() {
                let seq = me.seq;
                me.seq = me.seq.wrapping_add(1);
                p[0..2].copy_from_slice(&seq.to_le_bytes());
                me.sent[seq as usize % 1024] = Some((seq, ids));
                me.bytes_out += p.len() as u64;
                packets.push(p);
            }
        };
        let header = |me: &Channel| {
            let mut p = Vec::with_capacity(PLAIN_MAX);
            p.extend_from_slice(&[0, 0]);
            p.extend_from_slice(&me.recv_latest.unwrap_or(0).to_le_bytes());
            p.extend_from_slice(&if me.recv_latest.is_some() { me.recv_bits } else { 0 }.to_le_bytes());
            p.push(me.recv_latest.is_some() as u8);
            p
        };
        // Reliable pieces that are due.
        let mut i = 0;
        while i < self.out.len() && budget > 0 {
            let due = self.out[i].sent_at.is_none_or(|t| now.duration_since(t) >= resend);
            if due {
                let need = 5 + self.out[i].data.len();
                if cur.as_ref().is_some_and(|(p, _)| p.len() + need > PLAIN_MAX) {
                    finish(&mut cur, &mut packets, self);
                }
                if cur.is_none() {
                    cur = Some((header(self), Vec::new()));
                }
                let o = &mut self.out[i];
                let (p, ids) = cur.as_mut().unwrap();
                p.push(1);
                p.extend_from_slice(&o.id.to_le_bytes());
                p.extend_from_slice(&(o.data.len() as u16).to_le_bytes());
                p.extend_from_slice(&o.data);
                ids.push(o.id);
                o.sent_at = Some(now);
                budget = budget.saturating_sub(need);
            }
            i += 1;
        }
        let unrel = std::mem::take(&mut self.unrel);
        for u in &unrel {
            let need = 3 + u.len();
            if cur.as_ref().is_some_and(|(p, _)| p.len() + need > PLAIN_MAX) {
                finish(&mut cur, &mut packets, self);
            }
            if cur.is_none() {
                cur = Some((header(self), Vec::new()));
            }
            let (p, _) = cur.as_mut().unwrap();
            p.push(2);
            p.extend_from_slice(&(u.len() as u16).to_le_bytes());
            p.extend_from_slice(u);
        }
        let extra = [ping.map(|m| (3u8, m)), pong.map(|m| (4u8, m))];
        for (tag, m) in extra.into_iter().flatten() {
            if cur.as_ref().is_some_and(|(p, _)| p.len() + 5 > PLAIN_MAX) {
                finish(&mut cur, &mut packets, self);
            }
            if cur.is_none() {
                cur = Some((header(self), Vec::new()));
            }
            let (p, _) = cur.as_mut().unwrap();
            p.push(tag);
            p.extend_from_slice(&m.to_le_bytes());
        }
        if bye {
            if cur.is_none() {
                cur = Some((header(self), Vec::new()));
            }
            cur.as_mut().unwrap().0.push(5);
        }
        if cur.is_none() && self.ack_due {
            cur = Some((header(self), Vec::new()));
        }
        finish(&mut cur, &mut packets, self);
        if !packets.is_empty() {
            self.ack_due = false;
        }
        packets
    }

    fn acked(&mut self, seq: u16) {
        let slot = &mut self.sent[seq as usize % 1024];
        if let Some((s, ids)) = slot.take() {
            if s != seq {
                *slot = Some((s, ids));
                return;
            }
            if !ids.is_empty() {
                self.out.retain(|o| !ids.contains(&o.id));
            }
        }
    }

    /// One packet in; returns (reliable messages, unreliable messages).
    pub fn receive(&mut self, now: Instant, p: &[u8]) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
        let mut rel = Vec::new();
        let mut unrel = Vec::new();
        if p.len() < 9 {
            return (rel, unrel);
        }
        self.bytes_in += p.len() as u64;
        let seq = u16::from_le_bytes([p[0], p[1]]);
        let ack = u16::from_le_bytes([p[2], p[3]]);
        let bits = u32::from_le_bytes([p[4], p[5], p[6], p[7]]);
        if p[8] != 0 {
            self.acked(ack);
            for b in 0..32u16 {
                if bits & (1 << b) != 0 {
                    self.acked(ack.wrapping_sub(b + 1));
                }
            }
        }
        // Duplicate packet: its acks are taken, its records were delivered already.
        let dup = match self.recv_latest {
            None => {
                self.recv_latest = Some(seq);
                self.recv_bits = 0;
                false
            }
            Some(l) if seq == l => true,
            Some(l) if seq_gt(seq, l) => {
                let d = seq.wrapping_sub(l) as u32;
                self.recv_bits = if d >= 32 { 0 } else { (self.recv_bits << d) | (1 << (d - 1)) };
                self.recv_latest = Some(seq);
                false
            }
            Some(l) => {
                let d = l.wrapping_sub(seq) as u32;
                if d > 32 || self.recv_bits & (1 << (d - 1)) != 0 {
                    true
                } else {
                    self.recv_bits |= 1 << (d - 1);
                    false
                }
            }
        };
        let mut i = 9;
        let mut any = false;
        while i < p.len() {
            let tag = p[i];
            i += 1;
            match tag {
                1 => {
                    if i + 4 > p.len() {
                        break;
                    }
                    let id = u16::from_le_bytes([p[i], p[i + 1]]);
                    let len = u16::from_le_bytes([p[i + 2], p[i + 3]]) as usize;
                    i += 4;
                    if i + len > p.len() || len == 0 {
                        break;
                    }
                    any = true;
                    let piece = &p[i..i + len];
                    i += len;
                    if id == self.in_next {
                        self.take_piece(piece, &mut rel);
                        self.in_next = self.in_next.wrapping_add(1);
                        while let Some(next) = self.in_buf.remove(&self.in_next) {
                            self.take_piece(&next, &mut rel);
                            self.in_next = self.in_next.wrapping_add(1);
                        }
                    } else if seq_gt(id, self.in_next) && id.wrapping_sub(self.in_next) < 16384 {
                        self.in_buf.entry(id).or_insert_with(|| piece.to_vec());
                    }
                }
                2 => {
                    if i + 2 > p.len() {
                        break;
                    }
                    let len = u16::from_le_bytes([p[i], p[i + 1]]) as usize;
                    i += 2;
                    if i + len > p.len() {
                        break;
                    }
                    if !dup {
                        unrel.push(p[i..i + len].to_vec());
                    }
                    i += len;
                }
                3 | 4 => {
                    if i + 4 > p.len() {
                        break;
                    }
                    let m = u32::from_le_bytes([p[i], p[i + 1], p[i + 2], p[i + 3]]);
                    i += 4;
                    if tag == 3 {
                        self.pong_due = Some(m);
                    } else {
                        let r = self.ms(now).wrapping_sub(m) as f32;
                        if r < 10_000.0 {
                            self.rtt = if self.pings <= 2 { r } else { self.rtt * 0.85 + r * 0.15 };
                        }
                    }
                }
                5 => self.bye = true,
                _ => break,
            }
        }
        if any || !dup {
            self.ack_due = true;
        }
        (rel, unrel)
    }

    fn take_piece(&mut self, piece: &[u8], rel: &mut Vec<Vec<u8>>) {
        if self.assembling.len() + piece.len() > MAX_MESSAGE {
            self.assembling.clear();
            return;
        }
        self.assembling.extend_from_slice(&piece[1..]);
        if piece[0] & 1 != 0 {
            rel.push(std::mem::take(&mut self.assembling));
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Encrypted links and the endpoint.

struct Replay {
    max: u64,
    bits: u64,
    any: bool,
}

impl Replay {
    /// Whether `n` is new (and remember it).
    fn check(&mut self, n: u64) -> bool {
        if !self.any {
            self.any = true;
            self.max = n;
            self.bits = 0;
            return true;
        }
        if n > self.max {
            let d = n - self.max;
            self.bits = if d >= 64 { 0 } else { (self.bits << d) | (1 << (d - 1)) };
            self.max = n;
            true
        } else if n == self.max {
            false
        } else {
            let d = self.max - n;
            if d > 64 || self.bits & (1 << (d - 1)) != 0 {
                false
            } else {
                self.bits |= 1 << (d - 1);
                true
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Direct(SocketAddr),
    /// Through the lobby's relay (lobby UDP address, ticket).
    Relay(SocketAddr, u64),
}

struct Peer {
    id: PeerId,
    route: Route,
    local_sid: u32,
    remote_sid: u32,
    crypt: StatelessTransportState,
    nonce: u64,
    replay: Replay,
    ch: Channel,
    last_recv: Instant,
    /// Host: the handshake answer, sent again when the joiner repeats its first message.
    hs_resp: Vec<u8>,
    /// Bye sent: drop after the next flush.
    leaving: Option<String>,
}

struct Pending {
    hs: HandshakeState,
    local_sid: u32,
    init: Vec<u8>,
    targets: Vec<SocketAddr>,
    started: Instant,
    last_sent: Option<Instant>,
    last_punch: Option<Instant>,
    relay: bool,
    force_relay: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A link is up (host: a joiner arrived; joiner: the host answered).
    Joined(PeerId, Route),
    Message { peer: PeerId, data: Vec<u8>, reliable: bool },
    Left(PeerId, String),
    /// Host: the lobby says this is our public UDP address.
    PublicAddr(SocketAddr),
}

/// Where to find a host (from the lobby's PEER answer).
#[derive(Clone, Debug)]
pub struct JoinInfo {
    pub host_key: [u8; 32],
    /// Addresses to try directly (LAN, the one the lobby saw).
    pub targets: Vec<SocketAddr>,
    /// The lobby's UDP address (punching and the relay), if any.
    pub lobby: Option<SocketAddr>,
    pub ticket: u64,
    /// Tests: go through the relay only.
    pub force_relay: bool,
}

pub struct Endpoint {
    sock: UdpSocket,
    is_host: bool,
    private: Option<[u8; 32]>,
    peers: Vec<Peer>,
    pending: Option<Pending>,
    lobby: Option<SocketAddr>,
    token: Option<String>,
    last_reg: Option<Instant>,
    /// Host: joiners to probe (address, probes left, next).
    probes: Vec<(SocketAddr, u8, Instant)>,
    events: Vec<Event>,
    pub max_peers: u8,
    hs_rate: HashMap<IpAddr, (Instant, u32)>,
    /// Tests: drop this share of outgoing packets.
    pub sim_loss: f32,
    loss_rng: u64,
    buf: Vec<u8>,
}

impl Endpoint {
    /// A host on `port` (0 = any; tries the next 20 when taken). `lobby`/`token`: register with the lobby's UDP.
    pub fn host(port: u16, private: [u8; 32], lobby: Option<SocketAddr>, token: Option<String>) -> std::io::Result<Endpoint> {
        let mut last = None;
        for p in 0..=20u16 {
            let try_port = if port == 0 { 0 } else { port + p };
            match UdpSocket::bind(("0.0.0.0", try_port)) {
                Ok(s) => return Endpoint::new(s, true, Some(private), lobby, token),
                Err(e) => last = Some(e),
            }
            if port == 0 {
                break;
            }
        }
        Err(last.unwrap())
    }

    /// A joiner (any local port): starts the handshake at once.
    pub fn join(info: JoinInfo) -> std::io::Result<Endpoint> {
        let s = UdpSocket::bind(("0.0.0.0", 0))?;
        let mut e = Endpoint::new(s, false, None, info.lobby, None)?;
        let params = NOISE.parse().expect("noise params");
        let mut hs = Builder::new(params)
            .remote_public_key(&info.host_key)
            .build_initiator()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let mut msg = vec![0u8; 256];
        let n = hs.write_message(&[], &mut msg).map_err(|e| std::io::Error::other(e.to_string()))?;
        let local_sid = rand_u64() as u32 | 1;
        let mut init = vec![K_HS_INIT];
        init.extend_from_slice(&local_sid.to_le_bytes());
        init.extend_from_slice(&msg[..n]);
        e.pending = Some(Pending {
            hs,
            local_sid,
            init,
            targets: info.targets,
            started: Instant::now(),
            last_sent: None,
            last_punch: None,
            relay: info.force_relay,
            force_relay: info.force_relay,
        });
        e.token = Some(format!("{}", info.ticket));
        Ok(e)
    }

    fn new(sock: UdpSocket, is_host: bool, private: Option<[u8; 32]>, lobby: Option<SocketAddr>, token: Option<String>) -> std::io::Result<Endpoint> {
        sock.set_nonblocking(true)?;
        Ok(Endpoint {
            sock,
            is_host,
            private,
            peers: Vec::new(),
            pending: None,
            lobby,
            token,
            last_reg: None,
            probes: Vec::new(),
            events: Vec::new(),
            max_peers: 15,
            hs_rate: HashMap::new(),
            sim_loss: 0.0,
            loss_rng: rand_u64() | 1,
            buf: vec![0u8; 2048],
        })
    }

    /// Host: register with the lobby's UDP port (its room token), so joiners can be told where we are.
    pub fn register(&mut self, lobby: SocketAddr, token: String) {
        self.lobby = Some(lobby);
        self.token = Some(token);
        self.last_reg = None;
    }

    pub fn local_port(&self) -> u16 {
        self.sock.local_addr().map(|a| a.port()).unwrap_or(0)
    }

    pub fn is_host(&self) -> bool {
        self.is_host
    }

    /// Joiner: still finding the host.
    pub fn connecting(&self) -> bool {
        self.pending.is_some()
    }

    pub fn peers(&self) -> Vec<PeerId> {
        self.peers.iter().filter(|p| p.leaving.is_none()).map(|p| p.id).collect()
    }

    pub fn route(&self, id: PeerId) -> Option<Route> {
        self.peers.iter().find(|p| p.id == id).map(|p| p.route)
    }

    /// Round trip to a peer (ms).
    pub fn rtt(&self, id: PeerId) -> Option<f32> {
        self.peers.iter().find(|p| p.id == id).map(|p| p.ch.rtt)
    }

    /// (bytes out, bytes in, reliable pieces waiting) for a peer.
    pub fn stats(&self, id: PeerId) -> Option<(u64, u64, usize)> {
        self.peers.iter().find(|p| p.id == id).map(|p| (p.ch.bytes_out, p.ch.bytes_in, p.ch.unacked()))
    }

    pub fn send(&mut self, id: PeerId, data: &[u8], reliable: bool) {
        if let Some(p) = self.peers.iter_mut().find(|p| p.id == id && p.leaving.is_none()) {
            if reliable {
                p.ch.send_reliable(data);
            } else {
                p.ch.send_unreliable(data);
            }
        }
    }

    pub fn broadcast(&mut self, data: &[u8], reliable: bool, except: Option<PeerId>) {
        for p in self.peers.iter_mut().filter(|p| Some(p.id) != except && p.leaving.is_none()) {
            if reliable {
                p.ch.send_reliable(data);
            } else {
                p.ch.send_unreliable(data);
            }
        }
    }

    /// Close one link (or all with None), saying goodbye.
    pub fn kick(&mut self, id: Option<PeerId>, why: &str) {
        for p in self.peers.iter_mut().filter(|p| id.is_none_or(|i| i == p.id)) {
            p.ch.bye = true;
            p.leaving = Some(why.to_string());
        }
    }

    fn raw(&mut self, to: SocketAddr, b: &[u8]) {
        if self.sim_loss > 0.0 {
            self.loss_rng ^= self.loss_rng << 13;
            self.loss_rng ^= self.loss_rng >> 7;
            self.loss_rng ^= self.loss_rng << 17;
            if (self.loss_rng % 10_000) as f32 / 10_000.0 < self.sim_loss {
                return;
            }
        }
        let _ = self.sock.send_to(b, to);
    }

    fn send_route(&mut self, route: Route, b: &[u8]) {
        match route {
            Route::Direct(a) => self.raw(a, b),
            Route::Relay(l, t) => {
                let mut v = Vec::with_capacity(b.len() + RELAY_HDR);
                v.push(K_RELAY);
                v.extend_from_slice(&t.to_le_bytes());
                v.extend_from_slice(b);
                self.raw(l, &v);
            }
        }
    }

    /// Read everything waiting, run the timers, send what's due. Returns what happened.
    pub fn poll(&mut self) -> Vec<Event> {
        let now = Instant::now();
        loop {
            let (n, from) = match self.sock.recv_from(&mut self.buf) {
                Ok(x) => x,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                // Windows reports an earlier send's ICMP "port unreachable" here: keep reading.
                Err(_) => continue,
            };
            let pkt = self.buf[..n].to_vec();
            self.incoming(now, from, &pkt);
        }
        self.timers(now);
        self.flush(now);
        std::mem::take(&mut self.events)
    }

    fn incoming(&mut self, now: Instant, from: SocketAddr, pkt: &[u8]) {
        if pkt.is_empty() {
            return;
        }
        let from_lobby = self.lobby == Some(from);
        match pkt[0] {
            K_RELAY if from_lobby && pkt.len() > RELAY_HDR => {
                let t = u64::from_le_bytes(pkt[1..9].try_into().unwrap());
                let inner = pkt[RELAY_HDR..].to_vec();
                if inner.first() == Some(&K_RELAY) {
                    return;
                }
                self.incoming_on(now, Route::Relay(from, t), &inner);
            }
            K_SEEN if from_lobby => {
                if let Some(a) = std::str::from_utf8(&pkt[1..]).ok().and_then(|s| s.parse().ok()) {
                    self.events.push(Event::PublicAddr(a));
                }
            }
            K_HOSTAT if from_lobby && pkt.len() > 9 => {
                if let (Some(p), Some(a)) = (self.pending.as_mut(), std::str::from_utf8(&pkt[9..]).ok().and_then(|s| s.parse::<SocketAddr>().ok())) {
                    if !p.targets.contains(&a) && !p.force_relay {
                        p.targets.push(a);
                        p.last_sent = None;
                    }
                }
            }
            K_JOINAT if from_lobby && self.is_host && pkt.len() > 9 => {
                if let Some(a) = std::str::from_utf8(&pkt[9..]).ok().and_then(|s| s.parse::<SocketAddr>().ok()) {
                    if self.probes.len() < 64 {
                        self.probes.push((a, 8, now));
                    }
                }
            }
            _ if from_lobby => {}
            _ => self.incoming_on(now, Route::Direct(from), pkt),
        }
    }

    fn incoming_on(&mut self, now: Instant, route: Route, pkt: &[u8]) {
        match pkt[0] {
            K_HS_INIT if self.is_host && pkt.len() > 5 => self.handshake_init(now, route, pkt),
            K_HS_RESP if !self.is_host && pkt.len() > 9 => self.handshake_resp(now, route, pkt),
            K_DATA if pkt.len() > DATA_HDR + TAG => {
                let sid = u32::from_le_bytes(pkt[1..5].try_into().unwrap());
                let nonce = u64::from_le_bytes(pkt[5..13].try_into().unwrap());
                let Some(idx) = self.peers.iter().position(|p| p.local_sid == sid) else { return };
                let mut plain = vec![0u8; pkt.len()];
                let p = &mut self.peers[idx];
                let Ok(n) = p.crypt.read_message(nonce, &pkt[DATA_HDR..], &mut plain) else { return };
                if !p.replay.check(nonce) {
                    return;
                }
                p.last_recv = now;
                // A joiner whose NAT mapping changed (or who went direct after the relay) is followed.
                if matches!(route, Route::Direct(_)) && p.route != route && self.is_host {
                    p.route = route;
                }
                let (rel, unrel) = p.ch.receive(now, &plain[..n]);
                let id = p.id;
                for d in rel {
                    self.events.push(Event::Message { peer: id, data: d, reliable: true });
                }
                for d in unrel {
                    self.events.push(Event::Message { peer: id, data: d, reliable: false });
                }
                let p = &mut self.peers[idx];
                if p.ch.bye && p.leaving.is_none() {
                    p.leaving = Some("left".into());
                    p.ch.bye_sent = true;
                }
            }
            _ => {}
        }
    }

    fn handshake_init(&mut self, now: Instant, route: Route, pkt: &[u8]) {
        let remote_sid = u32::from_le_bytes(pkt[1..5].try_into().unwrap());
        // A repeat (or the same joiner knocking on another of our addresses): answer again, there too. The joiner
        // takes the first answer; data then comes in on its route and the link follows it.
        if let Some(p) = self.peers.iter().find(|p| p.remote_sid == remote_sid) {
            let r = p.hs_resp.clone();
            self.send_route(route, &r);
            return;
        }
        let ip = match route {
            Route::Direct(a) => a.ip(),
            Route::Relay(l, _) => l.ip(),
        };
        let e = self.hs_rate.entry(ip).or_insert((now, 0));
        if now.duration_since(e.0) > Duration::from_secs(1) {
            *e = (now, 0);
        }
        e.1 += 1;
        if e.1 > 20 {
            return;
        }
        if self.hs_rate.len() > 4096 {
            self.hs_rate.retain(|_, v| now.duration_since(v.0) < Duration::from_secs(2));
        }
        if self.peers.iter().filter(|p| p.leaving.is_none()).count() >= self.max_peers as usize {
            return;
        }
        let Some(private) = self.private else { return };
        let params = NOISE.parse().expect("noise params");
        let Ok(mut hs) = Builder::new(params).local_private_key(&private).build_responder() else { return };
        let mut scratch = vec![0u8; 256];
        if hs.read_message(&pkt[5..], &mut scratch).is_err() {
            return;
        }
        let mut msg = vec![0u8; 256];
        let Ok(n) = hs.write_message(&[], &mut msg) else { return };
        let Ok(crypt) = hs.into_stateless_transport_mode() else { return };
        let local_sid = loop {
            let s = rand_u64() as u32 | 1;
            if !self.peers.iter().any(|p| p.local_sid == s) {
                break s;
            }
        };
        let Some(id) = (1..=15u8).find(|i| !self.peers.iter().any(|p| p.id == *i)) else { return };
        let mut resp = vec![K_HS_RESP];
        resp.extend_from_slice(&remote_sid.to_le_bytes());
        resp.extend_from_slice(&local_sid.to_le_bytes());
        resp.extend_from_slice(&msg[..n]);
        self.send_route(route, &resp);
        self.peers.push(Peer {
            id,
            route,
            local_sid,
            remote_sid,
            crypt,
            nonce: 0,
            replay: Replay { max: 0, bits: 0, any: false },
            ch: Channel::new(),
            last_recv: now,
            hs_resp: resp,
            leaving: None,
        });
        self.events.push(Event::Joined(id, route));
    }

    fn handshake_resp(&mut self, now: Instant, route: Route, pkt: &[u8]) {
        let Some(mut p) = self.pending.take() else { return };
        let sid = u32::from_le_bytes(pkt[1..5].try_into().unwrap());
        if sid != p.local_sid {
            self.pending = Some(p);
            return;
        }
        let remote_sid = u32::from_le_bytes(pkt[5..9].try_into().unwrap());
        let mut scratch = vec![0u8; 256];
        if p.hs.read_message(&pkt[9..], &mut scratch).is_err() {
            self.pending = Some(p);
            return;
        }
        let Ok(crypt) = p.hs.into_stateless_transport_mode() else { return };
        self.peers.push(Peer {
            id: 0,
            route,
            local_sid: p.local_sid,
            remote_sid,
            crypt,
            nonce: 0,
            replay: Replay { max: 0, bits: 0, any: false },
            ch: Channel::new(),
            last_recv: now,
            hs_resp: Vec::new(),
            leaving: None,
        });
        self.events.push(Event::Joined(0, route));
    }

    fn timers(&mut self, now: Instant) {
        // Host: tell the lobby where we are (also keeps our NAT mapping to it open).
        if self.is_host {
            if let (Some(l), Some(t)) = (self.lobby, self.token.clone()) {
                if self.last_reg.is_none_or(|r| now.duration_since(r) >= Duration::from_secs(2)) {
                    self.last_reg = Some(now);
                    let mut v = vec![K_REG];
                    v.extend_from_slice(t.as_bytes());
                    self.raw(l, &v);
                }
            }
            let mut probes = std::mem::take(&mut self.probes);
            for (a, left, next) in probes.iter_mut() {
                if now >= *next && *left > 0 {
                    *left -= 1;
                    *next = now + Duration::from_millis(120);
                    self.raw(*a, &[K_PROBE]);
                }
            }
            probes.retain(|p| p.1 > 0);
            self.probes = probes;
        }
        // Joiner: keep knocking.
        if let Some(mut p) = self.pending.take() {
            if now.duration_since(p.started) > JOIN_TIMEOUT {
                self.events.push(Event::Left(0, "The host didn't answer.".into()));
                return;
            }
            let ticket: u64 = self.token.as_deref().and_then(|t| t.parse().ok()).unwrap_or(0);
            if let Some(l) = self.lobby {
                if p.last_punch.is_none_or(|t| now.duration_since(t) >= Duration::from_millis(500)) {
                    p.last_punch = Some(now);
                    let mut v = vec![K_PUNCH];
                    v.extend_from_slice(&ticket.to_le_bytes());
                    self.raw(l, &v);
                }
                if !p.relay && now.duration_since(p.started) >= RELAY_AFTER {
                    p.relay = true;
                }
            }
            if p.last_sent.is_none_or(|t| now.duration_since(t) >= Duration::from_millis(250)) {
                p.last_sent = Some(now);
                let init = p.init.clone();
                if !p.force_relay {
                    for a in p.targets.clone() {
                        self.raw(a, &init);
                    }
                }
                if p.relay {
                    if let Some(l) = self.lobby {
                        self.send_route(Route::Relay(l, ticket), &init);
                    }
                }
            }
            self.pending = Some(p);
        }
        // Silent links end.
        for p in &mut self.peers {
            if p.leaving.is_none() && now.duration_since(p.last_recv) > TIMEOUT {
                p.leaving = Some("timed out".into());
            }
            if p.leaving.is_none() && p.ch.broken() {
                p.leaving = Some("couldn't keep up".into());
            }
        }
    }

    fn flush(&mut self, now: Instant) {
        let mut out: Vec<(Route, Vec<u8>)> = Vec::new();
        let mut done = Vec::new();
        for p in &mut self.peers {
            for plain in p.ch.flush(now) {
                let mut pkt = vec![0u8; DATA_HDR + plain.len() + TAG];
                pkt[0] = K_DATA;
                pkt[1..5].copy_from_slice(&p.remote_sid.to_le_bytes());
                pkt[5..13].copy_from_slice(&p.nonce.to_le_bytes());
                if let Ok(n) = p.crypt.write_message(p.nonce, &plain, &mut pkt[DATA_HDR..]) {
                    pkt.truncate(DATA_HDR + n);
                    out.push((p.route, pkt));
                }
                p.nonce += 1;
            }
            if let Some(why) = &p.leaving {
                done.push((p.id, why.clone()));
            }
        }
        for (r, b) in out {
            self.send_route(r, &b);
        }
        for (id, why) in done {
            self.peers.retain(|p| p.id != id);
            self.events.push(Event::Left(id, why));
        }
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        self.kick(None, "closed");
        let now = Instant::now();
        self.flush(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pump(a: &mut Endpoint, b: &mut Endpoint, ms: u64) -> (Vec<Event>, Vec<Event>) {
        let mut ea = Vec::new();
        let mut eb = Vec::new();
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            ea.extend(a.poll());
            eb.extend(b.poll());
            std::thread::sleep(Duration::from_millis(2));
        }
        (ea, eb)
    }

    fn msgs(ev: &[Event], reliable: bool) -> Vec<Vec<u8>> {
        ev.iter()
            .filter_map(|e| match e {
                Event::Message { data, reliable: r, .. } if *r == reliable => Some(data.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn channel_orders_under_loss() {
        let mut a = Channel::new();
        let mut b = Channel::new();
        for i in 0..300u32 {
            a.send_reliable(&i.to_le_bytes());
        }
        let big: Vec<u8> = (0..50_000u32).map(|i| (i % 251) as u8).collect();
        a.send_reliable(&big);
        let mut got = Vec::new();
        let mut rng = 12345u64;
        let mut t = Instant::now();
        for _ in 0..400 {
            t += Duration::from_millis(30);
            for p in a.flush(t) {
                rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                if (rng >> 33) % 10 < 3 {
                    continue; // 30% lost
                }
                got.extend(b.receive(t, &p).0);
            }
            for p in b.flush(t) {
                rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                if (rng >> 33) % 10 < 3 {
                    continue;
                }
                a.receive(t, &p);
            }
        }
        assert_eq!(got.len(), 301, "all reliable messages arrive");
        for (i, m) in got.iter().take(300).enumerate() {
            assert_eq!(m, &(i as u32).to_le_bytes(), "in order");
        }
        assert_eq!(got[300], big);
        assert_eq!(a.unacked(), 0);
    }

    #[test]
    fn replay_window() {
        let mut r = Replay { max: 0, bits: 0, any: false };
        assert!(r.check(5));
        assert!(!r.check(5));
        assert!(r.check(3));
        assert!(!r.check(3));
        assert!(r.check(100));
        assert!(!r.check(20)); // too old
        assert!(r.check(99));
    }

    #[test]
    fn direct_link_with_loss() {
        let (private, public) = keypair();
        let mut host = Endpoint::host(0, private, None, None).unwrap();
        let port = host.local_port();
        let mut guest = Endpoint::join(JoinInfo {
            host_key: public,
            targets: vec![SocketAddr::from(([127, 0, 0, 1], port))],
            lobby: None,
            ticket: 0,
            force_relay: false,
        })
        .unwrap();
        let (eh, eg) = pump(&mut host, &mut guest, 300);
        assert!(eh.iter().any(|e| matches!(e, Event::Joined(1, _))), "host sees the joiner: {eh:?}");
        assert!(eg.iter().any(|e| matches!(e, Event::Joined(0, _))), "joiner reaches the host: {eg:?}");
        host.sim_loss = 0.25;
        guest.sim_loss = 0.25;
        for i in 0..200u32 {
            guest.send(0, &i.to_le_bytes(), true);
            host.send(1, &(1000 + i).to_le_bytes(), true);
        }
        let (eh, eg) = pump(&mut host, &mut guest, 3000);
        let hr = msgs(&eh, true);
        let gr = msgs(&eg, true);
        assert_eq!(hr.len(), 200);
        assert_eq!(gr.len(), 200);
        assert!(hr.iter().enumerate().all(|(i, m)| m == &(i as u32).to_le_bytes()));
        assert!(gr.iter().enumerate().all(|(i, m)| m == &(1000 + i as u32).to_le_bytes()));
        assert!(host.rtt(1).is_some());
        // A wrong key never connects.
        let (_, wrong) = keypair();
        let mut bad = Endpoint::join(JoinInfo {
            host_key: wrong,
            targets: vec![SocketAddr::from(([127, 0, 0, 1], port))],
            lobby: None,
            ticket: 0,
            force_relay: false,
        })
        .unwrap();
        let (eh, eb) = pump(&mut host, &mut bad, 400);
        assert!(!eh.iter().any(|e| matches!(e, Event::Joined(..))));
        assert!(!eb.iter().any(|e| matches!(e, Event::Joined(..))));
        // Goodbye reaches the other side.
        host.sim_loss = 0.0;
        guest.sim_loss = 0.0;
        guest.kick(None, "bye");
        let (eh, _) = pump(&mut host, &mut guest, 300);
        assert!(eh.iter().any(|e| matches!(e, Event::Left(1, _))), "{eh:?}");
    }

    #[test]
    fn through_the_lobby() {
        use crate::client::Client;
        use crate::server::{run, Config};
        let port = 41000 + (std::process::id() % 2000) as u16;
        std::thread::spawn(move || {
            let _ = run(Config { port, admin_port: port + 1, name: "Test".into(), log: None, admin_key: "k0123456789abcdef".into() });
        });
        std::thread::sleep(Duration::from_millis(300));
        let lobby_tcp = format!("127.0.0.1:{port}");
        let lobby_udp: SocketAddr = lobby_tcp.parse().unwrap();
        let (private, public) = keypair();
        let mut c = Client::connect(&lobby_tcp, Duration::from_secs(2)).unwrap();
        let s = crate::RoomSettings::default();
        // The host's game socket: an unknown port, so only the lobby can tell the joiner about it.
        let mut host_ep = Endpoint::host(0, private, Some(lobby_udp), None).unwrap();
        let (code, token) = c.host(&s, 1, &hex(&public), None).unwrap();
        host_ep.token = Some(token);
        let mut seen = false;
        for _ in 0..100 {
            seen |= host_ep.poll().iter().any(|e| matches!(e, Event::PublicAddr(_)));
            if seen {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(seen, "the lobby answers the host's REG");
        for force_relay in [false, true] {
            let mut g = Client::connect(&lobby_tcp, Duration::from_secs(2)).unwrap();
            let j = g.join(&code).unwrap();
            assert_eq!(unhex32(&j.key), Some(public));
            let mut guest = Endpoint::join(JoinInfo {
                host_key: unhex32(&j.key).unwrap(),
                targets: j.udp.into_iter().collect(),
                lobby: Some(lobby_udp),
                ticket: j.ticket,
                force_relay,
            })
            .unwrap();
            let (eh, eg) = pump(&mut host_ep, &mut guest, 800);
            let route = eg.iter().find_map(|e| match e {
                Event::Joined(0, r) => Some(*r),
                _ => None,
            });
            assert!(route.is_some(), "joined (relay {force_relay}): {eg:?} / {eh:?}");
            assert_eq!(matches!(route, Some(Route::Relay(..))), force_relay);
            let id = eh.iter().find_map(|e| match e {
                Event::Joined(id, _) => Some(*id),
                _ => None,
            });
            let id = id.expect("host saw the joiner");
            guest.send(0, b"hello host", true);
            host_ep.send(id, b"hello guest", false);
            let (eh, eg) = pump(&mut host_ep, &mut guest, 300);
            assert_eq!(msgs(&eh, true), vec![b"hello host".to_vec()]);
            assert_eq!(msgs(&eg, false), vec![b"hello guest".to_vec()]);
            guest.kick(None, "done");
            pump(&mut host_ep, &mut guest, 100);
        }
    }
}
