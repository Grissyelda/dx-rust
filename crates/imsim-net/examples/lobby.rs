//! A lobby server on the default ports for tests (no controller): `cargo run -p imsim-net --example lobby`.
fn main() {
    let port = std::env::args().nth(1).and_then(|p| p.parse().ok()).unwrap_or(imsim_net::DEFAULT_PORT);
    let cfg = imsim_net::server::Config { port, admin_port: port + 1, name: "Test lobby".into(), log: None, admin_key: "test-key-0123456789".into() };
    if let Err(e) = imsim_net::server::run(cfg) {
        eprintln!("lobby: {e}");
    }
}
