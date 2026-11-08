//! DX RUST online services: the lobby server and the window that controls it.
//!
//! `dxlobby` opens the controller: it starts the server in the background (a separate `dxlobby --serve` process
//! without a window, so closing the controller leaves it running), shows what it's doing (players connected, rooms
//! open, the room list) and stops, restarts or resets it (reset closes every room and disconnects everyone).
//! `dxlobby --serve` runs the server alone (no window; also how it runs on a Linux box). Files beside the exe:
//! dxlobby.log (what the server did) and dxlobby_admin.key (the key the controller proves itself with; the admin
//! port only answers on this PC).
//!
//! Headless (GitHub issue #46): built without the `gui` feature (`cargo build --release --no-default-features`)
//! there is no window code at all - no egui, OpenGL or X11 - and `dxlobby` just runs the server, logging to stderr
//! and dxlobby.log. Either build also takes these commands, which talk to a running server over its admin port:
//! `dxlobby status`, `dxlobby rooms`, `dxlobby reset` (close every room, disconnect everyone), `dxlobby stop`, and
//! `dxlobby upnp` (ask the router to forward the port). Options: `--port N`, `--admin-port N`, `--log PATH`
//! (`--log none` for no file), `--name TEXT` (the name players are shown), `--key-file PATH`.
#![cfg_attr(all(windows, feature = "gui"), windows_subsystem = "windows")]

#[cfg(feature = "gui")]
use eframe::egui;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::PathBuf;
#[cfg(feature = "gui")]
use std::sync::{Arc, Mutex};
use std::time::Duration;
#[cfg(feature = "gui")]
use std::time::Instant;

fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf())).unwrap_or_else(|| PathBuf::from("."))
}

fn arg_port(args: &[String], name: &str, default: u16) -> u16 {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn arg_str<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(|s| s.as_str())
}

/// Runs the lobby server in this process until a `stop` (blocks).
fn serve(args: &[String], port: u16, admin_port: u16, key: String) -> ! {
    let dir = exe_dir();
    let log = match arg_str(args, "--log") {
        Some("none") | Some("-") => None,
        Some(p) => Some(PathBuf::from(p)),
        None => Some(dir.join("dxlobby.log")),
    };
    let cfg = imsim_net::server::Config {
        port,
        admin_port,
        name: arg_str(args, "--name").unwrap_or("DX RUST Online").into(),
        log: log.clone(),
        admin_key: key,
    };
    if let Err(e) = imsim_net::server::run(cfg) {
        eprintln!("could not start: {e}");
        if let Some(l) = log {
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(l)
                .and_then(|mut f| writeln!(f, "could not start: {e}"));
        }
        std::process::exit(1);
    }
    std::process::exit(0);
}

/// `dxlobby status | rooms | reset | stop | upnp`: tell a running server (or the router) and print the answer.
fn command(cmd: &str, port: u16, admin_port: u16, key: &str) -> ! {
    let answer = match cmd {
        "upnp" => Ok(vec![open_router_port(port)]),
        _ => admin(admin_port, key, &cmd.to_ascii_uppercase()),
    };
    match answer {
        Ok(lines) => {
            for l in lines {
                println!("{l}");
            }
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("no answer from the server on admin port {admin_port}: {e}");
            std::process::exit(1);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let port = arg_port(&args, "--port", imsim_net::DEFAULT_PORT);
    let admin_port = arg_port(&args, "--admin-port", imsim_net::ADMIN_PORT);
    let dir = exe_dir();
    let key_file = arg_str(&args, "--key-file").map(PathBuf::from).unwrap_or_else(|| dir.join("dxlobby_admin.key"));
    let key = imsim_net::server::admin_key(&key_file);
    if let Some(cmd) = args.iter().skip(1).find(|a| matches!(a.as_str(), "status" | "rooms" | "reset" | "stop" | "upnp")) {
        command(cmd, port, admin_port, &key);
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("dxlobby [--serve] [--port N] [--admin-port N] [--log PATH|none] [--name TEXT] [--key-file PATH]");
        println!("dxlobby status | rooms | reset | stop | upnp   (talk to a running server)");
        return;
    }
    if !cfg!(feature = "gui") || args.iter().any(|a| a == "--serve") {
        serve(&args, port, admin_port, key);
    }
    #[cfg(feature = "gui")]
    run_gui(port, admin_port, key);
}

#[cfg(feature = "gui")]
fn run_gui(port: u16, admin_port: u16, key: String) {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([720.0, 520.0]).with_title("DX RUST Online Services"),
        ..Default::default()
    };
    let shared = Arc::new(Mutex::new(Seen::default()));
    {
        let shared = shared.clone();
        let key = key.clone();
        std::thread::spawn(move || poll_loop(shared, admin_port, key));
    }
    let _ = eframe::run_native(
        "DX RUST Online Services",
        opts,
        Box::new(move |_cc| Ok(Box::new(Controller { shared, key, port, admin_port, note: String::new(), confirm_reset: false, auto_started: false, upnp: Default::default(), upnp_busy: false }))),
    );
}

#[cfg(feature = "gui")]
/// What the controller last heard from the server.
#[derive(Default, Clone)]
struct Seen {
    running: bool,
    status: Vec<(String, String)>,
    rooms: Vec<String>,
    checked: Option<Instant>,
}

/// Ask the server (admin port): KEY, then `cmd`; the answer's lines up to END (or the one line).
fn admin(admin_port: u16, key: &str, cmd: &str) -> Result<Vec<String>, String> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], admin_port));
    let s = TcpStream::connect_timeout(&addr, Duration::from_millis(400)).map_err(|e| e.to_string())?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    let mut out = s.try_clone().map_err(|e| e.to_string())?;
    let mut rd = BufReader::new(s);
    writeln!(out, "KEY {key}").map_err(|e| e.to_string())?;
    let mut l = String::new();
    rd.read_line(&mut l).map_err(|e| e.to_string())?;
    if l.trim() != "OK" {
        return Err(format!("the server refused the key ({})", l.trim()));
    }
    writeln!(out, "{cmd}").map_err(|e| e.to_string())?;
    let mut lines = Vec::new();
    loop {
        let mut l = String::new();
        if rd.read_line(&mut l).map_err(|e| e.to_string())? == 0 {
            break;
        }
        let l = l.trim().to_string();
        let end = l == "END" || !(cmd == "ROOMS");
        if l != "END" {
            lines.push(l);
        }
        if end {
            break;
        }
    }
    Ok(lines)
}

#[cfg(feature = "gui")]
fn poll_loop(shared: Arc<Mutex<Seen>>, admin_port: u16, key: String) {
    loop {
        let mut seen = Seen { checked: Some(Instant::now()), ..Default::default() };
        if let Ok(st) = admin(admin_port, &key, "STATUS") {
            seen.running = true;
            if let Some(l) = st.first() {
                seen.status = l
                    .split_whitespace()
                    .skip(1)
                    .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
                    .collect();
            }
            seen.rooms = admin(admin_port, &key, "ROOMS").unwrap_or_default();
        }
        if let Ok(mut s) = shared.lock() {
            *s = seen;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[cfg(feature = "gui")]
/// Start `dxlobby --serve` in the background (no window, not tied to this one).
fn start_server(port: u16, admin_port: u16) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut c = std::process::Command::new(exe);
    c.args(["--serve", "--port", &port.to_string(), "--admin-port", &admin_port.to_string()]);
    c.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW | DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
        c.creation_flags(0x0800_0000 | 0x0000_0008 | 0x0000_0200);
    }
    c.spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// This PC's address on its local network (no packet is sent).
fn lan_ip() -> String {
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| s.connect("8.8.8.8:80").map(|_| s))
        .and_then(|s| s.local_addr())
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "?".into())
}

#[cfg(feature = "gui")]
/// The Windows Startup folder entry that starts the server at sign-in.
#[allow(dead_code)]
fn startup_file() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(appdata).join("Microsoft").join("Windows").join("Start Menu").join("Programs").join("Startup").join("DX RUST Online Services.bat"))
}

#[cfg(feature = "gui")]
struct Controller {
    shared: Arc<Mutex<Seen>>,
    key: String,
    port: u16,
    admin_port: u16,
    note: String,
    confirm_reset: bool,
    /// The server was started once by opening the controller (it starts it if it isn't running).
    auto_started: bool,
    /// What the router said when asked to open the port (UPnP), filled in by its thread.
    upnp: Arc<Mutex<Option<String>>>,
    upnp_busy: bool,
}

/// Ask the router (UPnP) to forward `port` TCP and UDP to this PC for good, and find the public address.
fn open_router_port(port: u16) -> String {
    use imsim_net::upnp::{find_gateway, public_ip, Proto};
    let g = match find_gateway(Duration::from_millis(3000)) {
        Ok(g) => g,
        Err(e) => {
            return format!(
                "No router answered ({e}). Turn on UPnP in the router's settings, or forward port {port} TCP and UDP to {} by hand.",
                lan_ip()
            )
        }
    };
    let mut fails = Vec::new();
    for (p, name) in [(Proto::Tcp, "TCP"), (Proto::Udp, "UDP")] {
        if let Err(e) = g.add_port(p, port, 0, "DX RUST online services") {
            fails.push(format!("{name}: {e}"));
        }
    }
    let ip = g.external_ip().ok().filter(|a| !a.is_private()).map(|a| a.to_string()).or_else(|| public_ip().ok().map(|a| a.to_string()));
    let addr = ip.map(|ip| format!(" Players outside your network connect to {ip}:{port}.")).unwrap_or_default();
    if fails.is_empty() {
        format!("Port {port} (TCP and UDP) now forwarded to this PC ({}).{addr}", g.local_ip)
    } else {
        format!("The router refused: {}. Forward port {port} TCP and UDP to {} by hand.{addr}", fails.join(", "), lan_ip())
    }
}

#[cfg(feature = "gui")]
impl eframe::App for Controller {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(500));
        let seen = self.shared.lock().map(|s| s.clone()).unwrap_or_default();
        // Opening the controller starts the server if it isn't running (once: Stop means stopped).
        if !self.auto_started && seen.checked.is_some() {
            self.auto_started = true;
            if !seen.running {
                self.note = match start_server(self.port, self.admin_port) {
                    Ok(()) => "Started the server.".into(),
                    Err(e) => format!("Could not start the server: {e}"),
                };
            }
        }
        let get = |k: &str| seen.status.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone()).unwrap_or_default();
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("DX RUST Online Services");
            ui.add_space(4.0);
            if seen.running {
                let up: u64 = get("up").parse().unwrap_or(0);
                ui.label(
                    egui::RichText::new(format!("Running on port {} - up {}h {:02}m {:02}s", get("port"), up / 3600, up / 60 % 60, up % 60))
                        .color(egui::Color32::from_rgb(120, 220, 120)),
                );
                ui.label(format!(
                    "{} players connected   {} rooms open ({} public)   {} connections since start, {} rooms hosted, {} joins",
                    get("conns"),
                    get("rooms"),
                    get("public"),
                    get("total"),
                    get("hosted"),
                    get("joins")
                ));
            } else {
                ui.label(egui::RichText::new("Stopped").color(egui::Color32::from_rgb(230, 120, 100)));
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(!seen.running, egui::Button::new("Start")).clicked() {
                    self.note = match start_server(self.port, self.admin_port) {
                        Ok(()) => "Starting...".into(),
                        Err(e) => format!("Could not start the server: {e}"),
                    };
                }
                if ui.add_enabled(seen.running, egui::Button::new("Stop")).clicked() {
                    self.note = match admin(self.admin_port, &self.key, "STOP") {
                        Ok(_) => "Stopped.".into(),
                        Err(e) => format!("Could not stop it: {e}"),
                    };
                }
                if ui.add_enabled(seen.running, egui::Button::new("Restart")).clicked() {
                    let _ = admin(self.admin_port, &self.key, "STOP");
                    std::thread::sleep(Duration::from_millis(600));
                    self.note = match start_server(self.port, self.admin_port) {
                        Ok(()) => "Restarted.".into(),
                        Err(e) => format!("Could not start the server: {e}"),
                    };
                }
                if ui.add_enabled(seen.running, egui::Button::new("Reset...")).clicked() {
                    self.confirm_reset = true;
                }
                if ui.button("Open Log").clicked() {
                    let log = exe_dir().join("dxlobby.log");
                    #[cfg(windows)]
                    let _ = std::process::Command::new("notepad").arg(&log).spawn();
                    #[cfg(not(windows))]
                    let _ = std::process::Command::new("xdg-open").arg(&log).spawn();
                }
            });
            if self.confirm_reset {
                ui.horizontal(|ui| {
                    ui.label("Close every room and disconnect every player?");
                    if ui.button("Reset now").clicked() {
                        self.note = match admin(self.admin_port, &self.key, "RESET") {
                            Ok(_) => "Reset: every room closed, every player disconnected.".into(),
                            Err(e) => format!("Could not reset: {e}"),
                        };
                        self.confirm_reset = false;
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm_reset = false;
                    }
                });
            }
            #[cfg(windows)]
            if let Some(f) = startup_file() {
                let mut on = f.exists();
                if ui.checkbox(&mut on, "Start the server when I sign in to Windows").changed() {
                    self.note = if on {
                        let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
                        match std::fs::write(&f, format!("@echo off\r\nstart \"\" \"{exe}\" --serve\r\n")) {
                            Ok(()) => "The server will start when you sign in.".into(),
                            Err(e) => format!("Could not add it to Startup: {e}"),
                        }
                    } else {
                        match std::fs::remove_file(&f) {
                            Ok(()) => "Removed from Startup.".into(),
                            Err(e) => format!("Could not remove it: {e}"),
                        }
                    };
                }
            }
            if let Some(r) = self.upnp.lock().ok().and_then(|mut u| u.take()) {
                self.note = r;
                self.upnp_busy = false;
            }
            ui.horizontal(|ui| {
                let label = if self.upnp_busy { "Asking the router..." } else { "Open port on router (UPnP)" };
                if ui.add_enabled(!self.upnp_busy, egui::Button::new(label)).clicked() {
                    self.upnp_busy = true;
                    let (port, out) = (self.port, self.upnp.clone());
                    std::thread::spawn(move || {
                        let r = open_router_port(port);
                        if let Ok(mut u) = out.lock() {
                            *u = Some(r);
                        }
                    });
                }
            });
            if !self.note.is_empty() {
                ui.label(&self.note);
            }
            ui.separator();
            ui.label(format!(
                "Players on this PC connect by themselves. On your network: {}:{}. From the internet: forward port {} (TCP and UDP) on your router to this PC and give players your public address (LobbyServer=<address>:{} in launcher.txt beside the game).",
                lan_ip(),
                self.port,
                self.port,
                self.port
            ));
            ui.label("Closing this window leaves the server running in the background.");
            ui.separator();
            ui.label("Rooms:");
            egui::ScrollArea::vertical().show(ui, |ui| {
                if seen.rooms.is_empty() {
                    ui.label("(none)");
                }
                for r in &seen.rooms {
                    // R code mode look players/max public|private ip age name
                    let w: Vec<&str> = r.split_whitespace().collect();
                    if w.len() >= 9 {
                        ui.label(format!(
                            "{}   {}   {}   {} players   {}   host {}   open {}   {}",
                            w[1],
                            if w[2] == "coop" { "Co-op" } else { "Deathmatch" },
                            if w[3] == "modern" { "Modern" } else { "Classic" },
                            w[4],
                            w[5],
                            w[6],
                            w[7],
                            w[8..].join(" ").replace('_', " ")
                        ));
                    }
                }
            });
        });
    }
}
