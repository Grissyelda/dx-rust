//! Opening a port on the home router (UPnP Internet Gateway Device, std only) and finding this PC's address on
//! the internet (STUN), so a host can be reached from outside without anyone setting up port forwarding.
//!
//! UPnP: an SSDP search on the LAN finds the router; its device description names the WAN connection service's
//! control URL; AddPortMapping / GetExternalIPAddress are SOAP posts to it. Routers with UPnP switched off (or
//! behind another NAT) simply don't answer, and the player is told to forward the port by hand.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

/// TCP or UDP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proto {
    Tcp,
    Udp,
}

impl Proto {
    fn word(self) -> &'static str {
        match self {
            Proto::Tcp => "TCP",
            Proto::Udp => "UDP",
        }
    }
}

/// A router found on the LAN: its WAN connection service.
#[derive(Clone, Debug)]
pub struct Gateway {
    /// host:port of the control URL.
    addr: SocketAddr,
    path: String,
    service: String,
    /// Our LAN address as the router sees it.
    pub local_ip: Ipv4Addr,
}

/// Look for the router (waits up to `wait`).
pub fn find_gateway(wait: Duration) -> Result<Gateway, String> {
    let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    let _ = sock.set_read_timeout(Some(Duration::from_millis(250)));
    let search = |st: &str| {
        format!("M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: {st}\r\n\r\n")
    };
    let targets = [
        "urn:schemas-upnp-org:device:InternetGatewayDevice:1",
        "urn:schemas-upnp-org:device:InternetGatewayDevice:2",
        "urn:schemas-upnp-org:service:WANIPConnection:1",
    ];
    let end = Instant::now() + wait;
    let mut buf = vec![0u8; 2048];
    let mut sent = None::<Instant>;
    while Instant::now() < end {
        if sent.is_none_or(|t| t.elapsed() > Duration::from_millis(800)) {
            sent = Some(Instant::now());
            for st in targets {
                let _ = sock.send_to(search(st).as_bytes(), "239.255.255.250:1900");
            }
        }
        let Ok((n, _)) = sock.recv_from(&mut buf) else { continue };
        let text = String::from_utf8_lossy(&buf[..n]).to_string();
        let Some(loc) = text.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim().eq_ignore_ascii_case("location").then(|| v.trim().to_string())
        }) else {
            continue;
        };
        if let Ok(g) = describe(&loc) {
            return Ok(g);
        }
    }
    Err("no router answered (UPnP is off, or the network has no UPnP router)".into())
}

/// "http://host:port/path" -> (addr, path).
fn split_url(url: &str) -> Option<(SocketAddr, String)> {
    let rest = url.strip_prefix("http://")?;
    let (hostport, path) = rest.split_once('/').map(|(a, b)| (a, format!("/{b}"))).unwrap_or((rest, "/".into()));
    let hp = if hostport.contains(':') { hostport.to_string() } else { format!("{hostport}:80") };
    let addr = hp.to_socket_addrs().ok()?.find(|a| a.is_ipv4())?;
    Some((addr, path))
}

fn http(addr: SocketAddr, req: &str) -> Result<(TcpStream, String), String> {
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(3)).map_err(|e| e.to_string())?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(4)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(3)));
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match s.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                out.extend_from_slice(&chunk[..n]);
                if out.len() > 512 * 1024 {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    Ok((s, String::from_utf8_lossy(&out).to_string()))
}

fn tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let i = xml.find(&open)? + open.len();
    let j = xml[i..].find(&format!("</{name}>"))? + i;
    Some(xml[i..j].trim())
}

/// Read the device description and find the WAN IP / PPP connection service.
fn describe(location: &str) -> Result<Gateway, String> {
    let (addr, path) = split_url(location).ok_or("bad location")?;
    let req = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    let (s, body) = http(addr, &req)?;
    let local_ip = match s.local_addr().map_err(|e| e.to_string())? {
        SocketAddr::V4(a) => *a.ip(),
        _ => return Err("IPv6 router".into()),
    };
    for service in ["urn:schemas-upnp-org:service:WANIPConnection:1", "urn:schemas-upnp-org:service:WANIPConnection:2", "urn:schemas-upnp-org:service:WANPPPConnection:1"] {
        let Some(i) = body.find(&format!("<serviceType>{service}</serviceType>")) else { continue };
        let rest = &body[i..];
        let end = rest.find("</service>").unwrap_or(rest.len());
        let Some(ctl) = tag(&rest[..end], "controlURL") else { continue };
        let ctl_path = if ctl.starts_with("http://") { split_url(ctl).map(|(_, p)| p).unwrap_or_default() } else if ctl.starts_with('/') { ctl.to_string() } else { format!("/{ctl}") };
        return Ok(Gateway { addr, path: ctl_path, service: service.to_string(), local_ip });
    }
    Err("the router has no WAN connection service".into())
}

impl Gateway {
    fn soap(&self, action: &str, args: &str) -> Result<String, String> {
        let body = format!(
            "<?xml version=\"1.0\"?>\r\n<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} xmlns:u=\"{}\">{args}</u:{action}></s:Body></s:Envelope>",
            self.service
        );
        let req = format!(
            "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: text/xml; charset=\"utf-8\"\r\nSOAPAction: \"{}#{action}\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            self.path,
            self.addr,
            self.service,
            body.len()
        );
        let (_, resp) = http(self.addr, &req)?;
        if resp.starts_with("HTTP/1.1 200") || resp.starts_with("HTTP/1.0 200") {
            Ok(resp)
        } else {
            let code = tag(&resp, "errorDescription").or(tag(&resp, "errorCode")).unwrap_or("refused");
            Err(format!("the router said no ({code})"))
        }
    }

    /// The router's internet address.
    pub fn external_ip(&self) -> Result<Ipv4Addr, String> {
        let r = self.soap("GetExternalIPAddress", "")?;
        tag(&r, "NewExternalIPAddress").and_then(|s| s.parse().ok()).ok_or_else(|| "no address in the answer".into())
    }

    /// Forward `port` (same number outside and in) to this PC. `lease` seconds, 0 = until removed.
    pub fn add_port(&self, proto: Proto, port: u16, lease: u32, what: &str) -> Result<(), String> {
        let args = format!(
            "<NewRemoteHost></NewRemoteHost><NewExternalPort>{port}</NewExternalPort><NewProtocol>{}</NewProtocol><NewInternalPort>{port}</NewInternalPort><NewInternalClient>{}</NewInternalClient><NewEnabled>1</NewEnabled><NewPortMappingDescription>{what}</NewPortMappingDescription><NewLeaseDuration>{lease}</NewLeaseDuration>",
            proto.word(),
            self.local_ip
        );
        match self.soap("AddPortMapping", &args) {
            // Some routers only take permanent mappings (OnlyPermanentLeasesSupported).
            Err(_) if lease != 0 => self.add_port(proto, port, 0, what),
            r => r.map(|_| ()),
        }
    }

    pub fn remove_port(&self, proto: Proto, port: u16) -> Result<(), String> {
        let args = format!("<NewRemoteHost></NewRemoteHost><NewExternalPort>{port}</NewExternalPort><NewProtocol>{}</NewProtocol>", proto.word());
        self.soap("DeletePortMapping", &args).map(|_| ())
    }
}

/// This PC's address on the internet, asked of a public STUN server (RFC 5389 binding request).
pub fn public_ip() -> Result<Ipv4Addr, String> {
    let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    let _ = sock.set_read_timeout(Some(Duration::from_millis(700)));
    let servers = ["stun.l.google.com:19302", "stun.cloudflare.com:3478", "stun1.l.google.com:19302"];
    let mut txn = [0u8; 12];
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(1);
    for (i, b) in txn.iter_mut().enumerate() {
        *b = (t >> (i * 5)) as u8 ^ (i as u8).wrapping_mul(37);
    }
    let mut req = vec![0x00, 0x01, 0x00, 0x00, 0x21, 0x12, 0xA4, 0x42];
    req.extend_from_slice(&txn);
    let mut buf = [0u8; 512];
    for _round in 0..2 {
        for s in servers {
            let Some(a) = s.to_socket_addrs().ok().and_then(|mut x| x.find(|a| a.is_ipv4())) else { continue };
            let _ = sock.send_to(&req, a);
            let Ok((n, _)) = sock.recv_from(&mut buf) else { continue };
            if let Some(ip) = parse_binding(&buf[..n], &txn) {
                return Ok(ip);
            }
        }
    }
    Err("no STUN server answered".into())
}

fn parse_binding(p: &[u8], txn: &[u8; 12]) -> Option<Ipv4Addr> {
    if p.len() < 20 || p[0] != 0x01 || p[1] != 0x01 || &p[8..20] != txn {
        return None;
    }
    let mut i = 20;
    while i + 4 <= p.len() {
        let kind = u16::from_be_bytes([p[i], p[i + 1]]);
        let len = u16::from_be_bytes([p[i + 2], p[i + 3]]) as usize;
        let v = p.get(i + 4..i + 4 + len)?;
        if (kind == 0x0020 || kind == 0x0001) && len >= 8 && v[1] == 1 {
            let mut ip = [v[4], v[5], v[6], v[7]];
            if kind == 0x0020 {
                for (k, b) in ip.iter_mut().enumerate() {
                    *b ^= [0x21, 0x12, 0xA4, 0x42][k];
                }
            }
            return Some(Ipv4Addr::from(ip));
        }
        i += 4 + len.div_ceil(4) * 4;
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn stun_answer() {
        let txn = [7u8; 12];
        // XOR-MAPPED-ADDRESS 203.0.113.9 port 4000.
        let ip = [203u8 ^ 0x21, 0 ^ 0x12, 113 ^ 0xA4, 9 ^ 0x42];
        let mut p = vec![0x01, 0x01, 0x00, 12, 0x21, 0x12, 0xA4, 0x42];
        p.extend_from_slice(&txn);
        p.extend_from_slice(&[0x00, 0x20, 0x00, 0x08, 0x00, 0x01, 0x2E, 0xB2]);
        p.extend_from_slice(&ip);
        assert_eq!(super::parse_binding(&p, &txn), Some(std::net::Ipv4Addr::new(203, 0, 113, 9)));
    }

    #[test]
    fn urls() {
        let (a, p) = super::split_url("http://192.168.1.1:5000/rootDesc.xml").unwrap();
        assert_eq!(a.port(), 5000);
        assert_eq!(p, "/rootDesc.xml");
        assert_eq!(super::tag("<a><NewExternalIPAddress> 1.2.3.4 </NewExternalIPAddress>", "NewExternalIPAddress"), Some("1.2.3.4"));
    }
}
