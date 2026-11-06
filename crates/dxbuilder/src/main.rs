//! DX RUST Release Builder: one window that builds the three releases into the project's dist\ folder.
//! - Windows: here, with Rust (what BUILD_RELEASE.bat did): dxrust-windows.zip.
//! - Linux / Steam Deck: in WSL on this PC (tools/release/build_linux.sh), or on GitHub: dxrust-steamdeck.tar.xz.
//! - macOS: on GitHub's Mac runners (.github/workflows/release.yml, through the GitHub CLI): dxrust-macos.zip.
//! Settings are kept in builder.txt beside the exe.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod run;

use eframe::egui;
use egui::{Color32, RichText};
use run::{capture, capture_any, cmd, lock, mmss, sq, Ctl, Shared, Status};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const TITLE: &str = "DX RUST Release Builder";
const WORKFLOW: &str = "release.yml";
const DEFAULT_REPO: &str = "Grissyelda/dx-rust";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Windows,
    Linux,
    Mac,
    GitHub,
    Setup,
}

#[derive(Clone)]
struct Cfg {
    root: String,
    version: String,
    repo: String,
    branch: String,
    distro: String,
    win: bool,
    linux: bool,
    mac: bool,
    /// Linux / Steam Deck on GitHub instead of in WSL.
    linux_github: bool,
    fast: bool,
}

impl Default for Cfg {
    fn default() -> Self {
        Cfg {
            root: find_root().map(|p| p.display().to_string()).unwrap_or_default(),
            version: String::new(),
            repo: DEFAULT_REPO.into(),
            branch: "main".into(),
            distro: String::new(),
            win: true,
            linux: true,
            mac: true,
            linux_github: false,
            fast: false,
        }
    }
}

fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| ".".into())
}

fn cfg_path() -> PathBuf {
    exe_dir().join("builder.txt")
}

fn is_project(p: &Path) -> bool {
    p.join("Cargo.toml").is_file() && p.join("crates").join("dxview").is_dir()
}

/// The project folder: the exe's folder or one above it, else the current folder or one above it.
fn find_root() -> Option<PathBuf> {
    let starts = [exe_dir(), std::env::current_dir().unwrap_or_default()];
    for s in starts {
        for d in s.ancestors().take(5) {
            if is_project(d) {
                return Some(d.to_path_buf());
            }
        }
    }
    None
}

impl Cfg {
    fn load() -> Cfg {
        let mut c = Cfg::default();
        let Ok(text) = std::fs::read_to_string(cfg_path()) else { return c };
        for l in text.lines() {
            let Some((k, v)) = l.split_once('=') else { continue };
            let v = v.trim().to_string();
            let b = v == "1";
            match k.trim() {
                "Root" if !v.is_empty() => c.root = v,
                "Version" => c.version = v,
                "Repo" if !v.is_empty() => c.repo = v,
                "Branch" if !v.is_empty() => c.branch = v,
                "Distro" => c.distro = v,
                "Windows" => c.win = b,
                "Linux" => c.linux = b,
                "Mac" => c.mac = b,
                "LinuxOnGitHub" => c.linux_github = b,
                "Fast" => c.fast = b,
                _ => {}
            }
        }
        c
    }
    fn save(&self) {
        let b = |x: bool| if x { "1" } else { "0" };
        let text = format!(
            "Root={}\nVersion={}\nRepo={}\nBranch={}\nDistro={}\nWindows={}\nLinux={}\nMac={}\nLinuxOnGitHub={}\nFast={}\n",
            self.root,
            self.version,
            self.repo,
            self.branch,
            self.distro,
            b(self.win),
            b(self.linux),
            b(self.mac),
            b(self.linux_github),
            b(self.fast)
        );
        let _ = std::fs::write(cfg_path(), text);
    }
    fn root(&self) -> PathBuf {
        PathBuf::from(self.root.trim())
    }
    fn dist(&self) -> PathBuf {
        self.root().join("dist")
    }
    fn version(&self) -> String {
        let v = self.version.trim().trim_start_matches('v');
        if v.is_empty() {
            "dev".into()
        } else {
            v.to_string()
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// What this PC has: Rust, WSL and its Linux tools, the GitHub CLI and its login.

#[derive(Default, Clone)]
struct Probe {
    running: bool,
    cargo: Option<String>,
    /// None = WSL isn't installed (or has no Linux yet).
    distros: Option<Vec<String>>,
    /// The chosen distro has Rust, zig and cargo-zigbuild.
    linux_ready: Option<bool>,
    gh: Option<String>,
    gh_user: Option<String>,
    commit: Option<String>,
    workflow: Option<bool>,
    last_release: Option<String>,
}

fn cargo_exe() -> String {
    if let Ok(home) = std::env::var("USERPROFILE") {
        let p = Path::new(&home).join(".cargo").join("bin").join("cargo.exe");
        if p.is_file() {
            return p.display().to_string();
        }
    }
    "cargo".into()
}

fn gh_exe() -> Option<String> {
    let mut tries = vec!["gh".to_string()];
    for v in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Ok(d) = std::env::var(v) {
            tries.push(Path::new(&d).join("GitHub CLI").join("gh.exe").display().to_string());
            tries.push(Path::new(&d).join("Programs").join("GitHub CLI").join("gh.exe").display().to_string());
        }
    }
    tries.into_iter().find(|g| {
        let mut c = cmd(g);
        c.arg("--version");
        capture(c).is_some()
    })
}

fn wsl_distros() -> Option<Vec<String>> {
    let mut c = cmd("wsl.exe");
    c.args(["-l", "-q"]);
    let out = capture(c)?;
    let v: Vec<String> = out
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.to_ascii_lowercase().starts_with("docker-desktop"))
        .collect();
    (!v.is_empty()).then_some(v)
}

fn linux_ready(distro: &str) -> bool {
    let mut c = cmd("wsl.exe");
    c.args(["-d", distro, "--exec", "bash", "-c"]).arg(
        ". ~/.cargo/env 2>/dev/null; command -v cargo-zigbuild >/dev/null && \
         (command -v zig >/dev/null || python3 -m ziglang version >/dev/null 2>&1) && \
         [ -f /usr/include/libudev.h ] && echo READY",
    );
    capture(c).map(|s| s.contains("READY")).unwrap_or(false)
}

fn probe(cfg: &Cfg) -> Probe {
    let mut p = Probe::default();
    let mut c = cmd(&cargo_exe());
    c.arg("--version");
    p.cargo = capture(c);
    p.distros = wsl_distros();
    if let Some(ds) = &p.distros {
        let d = if ds.contains(&cfg.distro) { cfg.distro.clone() } else { ds[0].clone() };
        p.linux_ready = Some(linux_ready(&d));
    }
    p.gh = gh_exe();
    if let Some(gh) = &p.gh {
        let mut c = cmd(gh);
        c.args(["api", "user", "--jq", ".login"]);
        p.gh_user = capture(c).filter(|s| !s.is_empty());
        if p.gh_user.is_some() {
            let mut c = cmd(gh);
            c.args([
                "api",
                &format!("repos/{}/commits/{}", cfg.repo, cfg.branch),
                "--jq",
                ".sha[0:7] + \"  \" + (.commit.message | split(\"\\n\")[0])",
            ]);
            p.commit = capture(c);
            let mut c = cmd(gh);
            c.args(["api", &format!("repos/{}/contents/.github/workflows/{WORKFLOW}?ref={}", cfg.repo, cfg.branch)]);
            c.args(["--jq", ".name"]);
            p.workflow = Some(capture(c).is_some());
            let mut c = cmd(gh);
            c.args(["release", "view", "-R", &cfg.repo, "--json", "tagName", "--jq", ".tagName"]);
            p.last_release = capture(c).filter(|s| !s.is_empty());
        }
    }
    p
}

/// "v0.11.0" -> "0.12.0".
fn next_minor(tag: &str) -> Option<String> {
    let v = tag.trim().trim_start_matches('v');
    let mut it = v.split('.');
    let major: u32 = it.next()?.parse().ok()?;
    let minor: u32 = it.next()?.parse().ok()?;
    Some(format!("{major}.{}.0", minor + 1))
}

/// C:\a\b -> /mnt/c/a/b (WSL's view of a Windows folder).
fn to_wsl(p: &Path) -> Option<String> {
    let s = p.display().to_string().replace('/', "\\");
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s).to_string();
    let b = s.as_bytes();
    if b.len() < 2 || b[1] != b':' || !b[0].is_ascii_alphabetic() {
        return None;
    }
    let rest = s[2..].replace('\\', "/");
    Some(format!("/mnt/{}{}", (b[0] as char).to_ascii_lowercase(), rest.trim_end_matches('/')))
}

// ---------------------------------------------------------------------------------------------------------------
// The builds.

struct Jobs {
    win: Shared,
    linux: Shared,
    mac: Shared,
    github: Shared,
    setup: Shared,
}

impl Jobs {
    fn all(&self) -> [&Shared; 5] {
        [&self.win, &self.linux, &self.mac, &self.github, &self.setup]
    }
    fn busy(&self) -> bool {
        self.all().iter().any(|j| lock(j).status.busy())
    }
}

fn fast_env(c: &mut std::process::Command, fast: bool) {
    if fast {
        c.env("CARGO_PROFILE_RELEASE_LTO", "thin").env("CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "16");
    }
}

/// BUILD_RELEASE.bat: the game and the launcher with the C runtime linked in, in their own build folders, then
/// dist\dxrust-windows.zip with the notices.
fn build_windows(job: &Shared, ctl: &Ctl, ctx: &egui::Context, cfg: &Cfg) -> Result<(), String> {
    let root = cfg.root();
    let dist = cfg.dist();
    let cargo = cargo_exe();
    lock(job).set(Status::Running("building the game".into()));
    let mut c = cmd(&cargo);
    c.current_dir(&root)
        .args(["build", "--release", "-p", "dxview"])
        .env("RUSTFLAGS", "-C target-feature=+crt-static")
        .env("CARGO_TARGET_DIR", root.join("target").join("dist-build"));
    fast_env(&mut c, cfg.fast);
    run::run(job, ctl, ctx, c)?;
    lock(job).set(Status::Running("building the launcher".into()));
    let mut c = cmd(&cargo);
    c.current_dir(root.join("crates").join("dxlauncher"))
        .args(["build", "--release"])
        .env("RUSTFLAGS", "-C target-feature=+crt-static")
        .env("CARGO_TARGET_DIR", root.join("target").join("dist-launcher"));
    fast_env(&mut c, cfg.fast);
    run::run(job, ctl, ctx, c)?;
    lock(job).set(Status::Running("packing the zip".into()));
    std::fs::create_dir_all(&dist).map_err(|e| e.to_string())?;
    let copies = [
        (root.join("target/dist-build/release/dxview.exe"), "dxview.exe"),
        (root.join("target/dist-launcher/release/dxlauncher.exe"), "dxlauncher.exe"),
        (root.join("LICENSE"), "LICENSE.txt"),
        (root.join("README.md"), "README.md"),
        (root.join("THIRD_PARTY.md"), "THIRD_PARTY.md"),
    ];
    let mut files = Vec::new();
    for (from, name) in copies {
        let to = dist.join(name);
        std::fs::copy(&from, &to).map_err(|e| format!("copy {}: {e}", from.display()))?;
        files.push(to);
    }
    let zip = dist.join("dxrust-windows.zip");
    let _ = std::fs::remove_file(&zip);
    let ps = |p: &Path| format!("'{}'", p.display().to_string().replace('\'', "''"));
    let list: Vec<String> = files.iter().map(|p| ps(p)).collect();
    let script = format!("Compress-Archive -LiteralPath {} -DestinationPath {} -Force", list.join(","), ps(&zip));
    let mut c = cmd("powershell.exe");
    c.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
    run::run(job, ctl, ctx, c)?;
    let size = std::fs::metadata(&zip).map(|m| m.len()).unwrap_or(0);
    lock(job).line(&format!("Built {} ({:.1} MB)", zip.display(), size as f64 / 1e6));
    Ok(())
}

fn wsl_distro(cfg: &Cfg, probe: &Probe) -> Option<String> {
    let ds = probe.distros.as_ref()?;
    Some(if ds.contains(&cfg.distro) { cfg.distro.clone() } else { ds[0].clone() })
}

/// tools/release/build_linux.sh in WSL, writing straight into this project's dist\.
fn build_linux_wsl(job: &Shared, ctl: &Ctl, ctx: &egui::Context, cfg: &Cfg, distro: &str) -> Result<(), String> {
    let root = to_wsl(&cfg.root()).ok_or("the project folder must be on a drive letter (C:\\...) for WSL")?;
    let out = format!("{root}/dist");
    std::fs::create_dir_all(cfg.dist()).map_err(|e| e.to_string())?;
    lock(job).set(Status::Running(format!("building in WSL ({distro})")));
    let script = format!(
        "sed 's/\\r$//' tools/release/build_linux.sh | DXR_ROOT={} DXR_FAST={} bash -s -- {}",
        sq(&root),
        if cfg.fast { 1 } else { 0 },
        sq(&out)
    );
    let mut c = cmd("wsl.exe");
    c.args(["-d", distro, "--cd", &root, "--exec", "bash", "-c", &script]);
    let r = run::run(job, ctl, ctx, c);
    if ctl.cancelled() {
        // Killing wsl.exe leaves the Linux side running: stop the compilers there too.
        let mut k = cmd("wsl.exe");
        k.args(["-d", distro, "--exec", "bash", "-c", "pkill -f cargo-zigbuild; pkill -f 'cargo zigbuild'; pkill rustc; true"]);
        let _ = capture_any(k);
    }
    r
}

/// The local builds, one after the other (two compiles at once only slow each other down).
fn local_thread(jobs: Arc<Jobs>, ctl: Arc<Ctl>, ctx: egui::Context, cfg: Cfg, probe: Probe, win: bool, linux: bool) {
    std::thread::spawn(move || {
        let dist = cfg.dist();
        let _ = std::fs::create_dir_all(&dist);
        if win {
            {
                let mut j = lock(&jobs.win);
                j.begin(Some(dist.join("build_windows.log")));
                j.line(&format!("== Windows release, {} ==", cfg.root().display()));
            }
            match build_windows(&jobs.win, &ctl, &ctx, &cfg) {
                Ok(()) => lock(&jobs.win).finish(),
                Err(e) if ctl.cancelled() => {
                    lock(&jobs.win).set(Status::Cancelled);
                    let _ = e;
                }
                Err(e) => lock(&jobs.win).set(Status::Failed(e)),
            }
            ctx.request_repaint();
        }
        if linux {
            {
                let mut j = lock(&jobs.linux);
                j.begin(Some(dist.join("build_linux.log")));
                j.line("== Linux / Steam Deck release (WSL) ==");
            }
            let res = match wsl_distro(&cfg, &probe) {
                None => Err("WSL isn't set up on this PC: see the Setup tab, or build Linux on GitHub".to_string()),
                Some(d) => build_linux_wsl(&jobs.linux, &ctl, &ctx, &cfg, &d),
            };
            match res {
                Ok(()) => lock(&jobs.linux).finish(),
                Err(_) if ctl.cancelled() => lock(&jobs.linux).set(Status::Cancelled),
                Err(e) => lock(&jobs.linux).set(Status::Failed(e)),
            }
            ctx.request_repaint();
        }
    });
}

fn gh_json(gh: &str, args: &[&str]) -> Option<serde_json::Value> {
    let mut c = cmd(gh);
    c.args(args);
    serde_json::from_str(&capture(c)?).ok()
}

/// One workflow run on GitHub for the macOS and / or Linux release: start it, follow its jobs, download what they
/// made into dist\.
#[allow(clippy::too_many_arguments)]
fn github_thread(jobs: Arc<Jobs>, ctl: Arc<Ctl>, ctx: egui::Context, cfg: Cfg, gh: String, mac: bool, linux: bool) {
    std::thread::spawn(move || {
        let dist = cfg.dist();
        let _ = std::fs::create_dir_all(&dist);
        let g = &jobs.github;
        lock(g).begin(Some(dist.join("build_github.log")));
        let mut targets: Vec<(&str, &Shared, &str, &str)> = Vec::new();
        if mac {
            targets.push(("macos", &jobs.mac, "dxrust-macos", "dxrust-macos.zip"));
        }
        if linux {
            targets.push(("linux", &jobs.linux, "dxrust-steamdeck", "dxrust-steamdeck.tar.xz"));
        }
        for (_, j, _, _) in &targets {
            let mut j = lock(j);
            j.begin(None);
            j.line("Building on GitHub: its log is in the GitHub tab.");
            j.set(Status::Running("starting on GitHub".into()));
        }
        let fail_all = |why: &str| {
            lock(g).set(Status::Failed(why.into()));
            for (_, j, _, _) in &targets {
                lock(j).set(Status::Failed(why.into()));
            }
            ctx.request_repaint();
        };
        let repo = cfg.repo.as_str();
        let platforms = targets.iter().map(|t| t.0).collect::<Vec<_>>().join(",");
        lock(g).line(&format!("== {repo} @ {}: {platforms} ==", cfg.branch));
        lock(g).set(Status::Running("starting the run".into()));
        // The runs that already exist, so the new one can be told apart.
        let list_args = ["run", "list", "-R", repo, "--workflow", WORKFLOW, "-L", "20", "--json", "databaseId"];
        let ids = |v: Option<serde_json::Value>| -> Vec<u64> {
            v.and_then(|v| v.as_array().cloned())
                .unwrap_or_default()
                .iter()
                .filter_map(|r| r["databaseId"].as_u64())
                .collect()
        };
        let before = ids(gh_json(&gh, &list_args));
        let mut c = cmd(&gh);
        c.args(["workflow", "run", WORKFLOW, "-R", repo, "--ref", &cfg.branch]);
        c.args(["-f", &format!("platforms={platforms}"), "-f", &format!("version={}", cfg.version())]);
        c.args(["-f", &format!("fast={}", cfg.fast)]);
        if let Err(e) = run::run(g, &ctl, &ctx, c) {
            return fail_all(&format!("couldn't start the GitHub run ({e}); is {WORKFLOW} on GitHub, and are you logged in?"));
        }
        let mut id = None;
        for _ in 0..30 {
            std::thread::sleep(Duration::from_secs(2));
            if ctl.cancelled() {
                break;
            }
            if let Some(n) = ids(gh_json(&gh, &list_args)).into_iter().filter(|i| !before.contains(i)).max() {
                id = Some(n);
                break;
            }
        }
        let Some(id) = id else {
            if ctl.cancelled() {
                lock(g).set(Status::Cancelled);
                return;
            }
            return fail_all("the run didn't show up on GitHub");
        };
        let ids_s = id.to_string();
        lock(g).line(&format!("Run {id}: https://github.com/{repo}/actions/runs/{id}"));
        let mut seen = std::collections::HashMap::<String, String>::new();
        let mut cancelled_sent = false;
        let result = loop {
            if ctl.cancelled() && !cancelled_sent {
                cancelled_sent = true;
                let mut c = cmd(&gh);
                c.args(["run", "cancel", &ids_s, "-R", repo]);
                let _ = capture_any(c);
                lock(g).line("Cancel sent to GitHub.");
            }
            let Some(v) = gh_json(&gh, &["run", "view", &ids_s, "-R", repo, "--json", "status,conclusion,jobs"]) else {
                std::thread::sleep(Duration::from_secs(10));
                continue;
            };
            for jv in v["jobs"].as_array().cloned().unwrap_or_default() {
                let name = jv["name"].as_str().unwrap_or("").to_string();
                let step = jv["steps"]
                    .as_array()
                    .and_then(|s| s.iter().find(|s| s["status"] == "in_progress").or(s.last()))
                    .and_then(|s| s["name"].as_str())
                    .unwrap_or("waiting for a machine")
                    .to_string();
                let state = format!("{} {}", jv["status"].as_str().unwrap_or(""), step);
                if seen.get(&name) != Some(&state) {
                    lock(g).line(&format!("{name}: {} - {step}", jv["status"].as_str().unwrap_or("")));
                    seen.insert(name.clone(), state);
                }
                if let Some((_, j, _, _)) = targets.iter().find(|t| t.0 == name) {
                    let mut j = lock(j);
                    if j.status.busy() {
                        j.status = Status::Running(format!("on GitHub: {step}"));
                    }
                }
            }
            ctx.request_repaint();
            if v["status"] == "completed" {
                break v;
            }
            std::thread::sleep(Duration::from_secs(10));
        };
        let mut any_failed = false;
        for (name, j, artifact, file) in &targets {
            let job = result["jobs"].as_array().and_then(|a| a.iter().find(|x| x["name"] == *name).cloned());
            let concl = job.as_ref().and_then(|x| x["conclusion"].as_str()).unwrap_or("").to_string();
            if concl != "success" {
                any_failed = true;
                lock(j).set(if ctl.cancelled() || concl == "cancelled" {
                    Status::Cancelled
                } else {
                    Status::Failed(format!("GitHub job {concl}"))
                });
                continue;
            }
            lock(j).set(Status::Running("downloading".into()));
            let _ = std::fs::remove_file(dist.join(file));
            let mut c = cmd(&gh);
            c.args(["run", "download", &ids_s, "-R", repo, "-n", artifact, "-D"]).arg(&dist);
            match run::run(g, &ctl, &ctx, c) {
                Ok(()) => {
                    let size = std::fs::metadata(dist.join(file)).map(|m| m.len()).unwrap_or(0);
                    let mut j = lock(j);
                    j.line(&format!("Built {} ({:.1} MB)", dist.join(file).display(), size as f64 / 1e6));
                    j.finish();
                }
                Err(e) => {
                    any_failed = true;
                    lock(j).set(Status::Failed(format!("download: {e}")));
                }
            }
        }
        if any_failed && !ctl.cancelled() {
            let mut c = cmd(&gh);
            c.args(["run", "view", &ids_s, "-R", repo, "--log-failed"]);
            let (_, text) = capture_any(c);
            let lines: Vec<&str> = text.lines().collect();
            let mut gj = lock(g);
            gj.line("== the end of the failed steps' log ==");
            for l in &lines[lines.len().saturating_sub(200)..] {
                gj.line(l);
            }
        }
        let mut gj = lock(g);
        if ctl.cancelled() {
            gj.set(Status::Cancelled);
        } else if any_failed {
            gj.set(Status::Failed("a GitHub job failed (its log is above)".into()));
        } else {
            gj.finish();
        }
        ctx.request_repaint();
    });
}

/// The CHANGELOG's "Unreleased" part, for the release notes.
fn unreleased_notes(root: &Path) -> String {
    let Ok(text) = std::fs::read_to_string(root.join("CHANGELOG.md")) else { return String::new() };
    let mut out = Vec::new();
    let mut on = false;
    for l in text.lines() {
        if l.starts_with("## ") {
            if on {
                break;
            }
            on = l.to_ascii_lowercase().contains("unreleased");
            continue;
        }
        if on {
            out.push(l);
        }
    }
    out.join("\n").trim().to_string()
}

/// A draft release on GitHub with the archives in dist\ (made by you, published by you on GitHub).
fn release_thread(jobs: Arc<Jobs>, ctl: Arc<Ctl>, ctx: egui::Context, cfg: Cfg, gh: String) {
    std::thread::spawn(move || {
        let g = &jobs.github;
        let dist = cfg.dist();
        lock(g).begin(Some(dist.join("release_github.log")));
        lock(g).set(Status::Running("making the draft release".into()));
        let tag = format!("v{}", cfg.version());
        let files: Vec<PathBuf> = ["dxrust-windows.zip", "dxrust-steamdeck.tar.xz", "dxrust-macos.zip"]
            .iter()
            .map(|f| dist.join(f))
            .filter(|p| p.is_file())
            .collect();
        if files.is_empty() {
            lock(g).set(Status::Failed("nothing in dist\\ to upload yet".into()));
            return;
        }
        let notes = dist.join("release_notes.md");
        let mut text = unreleased_notes(&cfg.root());
        if text.is_empty() {
            text = format!("DX RUST {tag}");
        }
        let _ = std::fs::write(&notes, text);
        let mut c = cmd(&gh);
        c.args(["release", "create", &tag, "-R", &cfg.repo, "--draft", "--target", &cfg.branch]);
        c.args(["--title", &format!("DX RUST {tag}"), "--notes-file"]).arg(&notes);
        c.args(&files);
        let r = run::run(g, &ctl, &ctx, c);
        let r = match r {
            Ok(()) => Ok(()),
            Err(_) if !ctl.cancelled() => {
                lock(g).line(&format!("{tag} may exist already: uploading the files to it instead."));
                let mut c = cmd(&gh);
                c.args(["release", "upload", &tag, "-R", &cfg.repo, "--clobber"]).args(&files);
                run::run(g, &ctl, &ctx, c)
            }
            Err(e) => Err(e),
        };
        match r {
            Ok(()) => {
                lock(g).line(&format!("Draft {tag} is on GitHub: https://github.com/{}/releases", cfg.repo));
                lock(g).finish();
            }
            Err(_) if ctl.cancelled() => lock(g).set(Status::Cancelled),
            Err(e) => lock(g).set(Status::Failed(e)),
        }
        ctx.request_repaint();
    });
}

/// Installs the Linux build tools in WSL (packages as root, then Rust / zig / cargo-zigbuild as the user).
fn setup_thread(jobs: Arc<Jobs>, ctl: Arc<Ctl>, ctx: egui::Context, cfg: Cfg, distro: String, probe: Arc<Mutex<Probe>>) {
    std::thread::spawn(move || {
        let j = &jobs.setup;
        lock(j).begin(Some(cfg.dist().join("setup_linux.log")));
        let res = (|| -> Result<(), String> {
            let root = to_wsl(&cfg.root()).ok_or("the project folder must be on a drive letter for WSL")?;
            for (who, step) in [("root", "system packages"), ("user", "Rust, zig and cargo-zigbuild")] {
                lock(j).set(Status::Running(format!("installing {step}")));
                let script = format!("sed 's/\\r$//' tools/release/setup_linux.sh | bash -s -- {who}");
                let mut c = cmd("wsl.exe");
                c.args(["-d", &distro]);
                if who == "root" {
                    c.args(["-u", "root"]);
                }
                c.args(["--cd", &root, "--exec", "bash", "-c", &script]);
                run::run(j, &ctl, &ctx, c)?;
            }
            Ok(())
        })();
        match res {
            Ok(()) => lock(j).finish(),
            Err(_) if ctl.cancelled() => lock(j).set(Status::Cancelled),
            Err(e) => lock(j).set(Status::Failed(e)),
        }
        let p = crate::probe(&cfg);
        *probe.lock().unwrap_or_else(|e| e.into_inner()) = p;
        ctx.request_repaint();
    });
}

/// Runs a program that asks questions (the GitHub login, an installer) in a console window of its own, then checks
/// the PC again once it has closed.
fn console_then_refresh(program: &str, args: &[&str], app_probe: Arc<Mutex<Probe>>, cfg: Cfg, ctx: egui::Context) {
    let mut c = std::process::Command::new(program);
    c.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        c.creation_flags(CREATE_NEW_CONSOLE);
    }
    let Ok(mut child) = c.spawn() else { return };
    std::thread::spawn(move || {
        let _ = child.wait();
        app_probe.lock().unwrap_or_else(|e| e.into_inner()).running = true;
        let p = probe(&cfg);
        *app_probe.lock().unwrap_or_else(|e| e.into_inner()) = p;
        ctx.request_repaint();
    });
}

// ---------------------------------------------------------------------------------------------------------------
// The window.

struct App {
    cfg: Cfg,
    probe: Arc<Mutex<Probe>>,
    jobs: Arc<Jobs>,
    ctl: Arc<Ctl>,
    tab: Tab,
    version_hint_used: bool,
}

impl App {
    fn new() -> App {
        let jobs = Jobs {
            win: Default::default(),
            linux: Default::default(),
            mac: Default::default(),
            github: Default::default(),
            setup: Default::default(),
        };
        App {
            cfg: Cfg::load(),
            probe: Default::default(),
            jobs: Arc::new(jobs),
            ctl: Arc::new(Ctl::default()),
            tab: Tab::Windows,
            version_hint_used: false,
        }
    }

    fn refresh(&self, ctx: &egui::Context) {
        {
            let mut p = self.probe.lock().unwrap_or_else(|e| e.into_inner());
            if p.running {
                return;
            }
            p.running = true;
        }
        let (cfg, slot, ctx) = (self.cfg.clone(), self.probe.clone(), ctx.clone());
        std::thread::spawn(move || {
            let p = probe(&cfg);
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = p;
            ctx.request_repaint();
        });
    }

    fn probe(&self) -> Probe {
        self.probe.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn fresh_ctl(&mut self) -> Arc<Ctl> {
        self.ctl = Arc::new(Ctl::default());
        self.ctl.clone()
    }

    fn build(&mut self, ctx: &egui::Context) {
        let p = self.probe();
        let cfg = self.cfg.clone();
        cfg.save();
        let ctl = self.fresh_ctl();
        let gh_mac = cfg.mac;
        let gh_linux = cfg.linux && cfg.linux_github;
        let local_linux = cfg.linux && !cfg.linux_github;
        for (on, j) in [(cfg.win, &self.jobs.win), (cfg.linux, &self.jobs.linux), (cfg.mac, &self.jobs.mac)] {
            if on {
                lock(j).set(Status::Queued);
            }
        }
        if gh_mac || gh_linux {
            match p.gh.clone() {
                Some(gh) if p.gh_user.is_some() => {
                    lock(&self.jobs.github).set(Status::Queued);
                    github_thread(self.jobs.clone(), ctl.clone(), ctx.clone(), cfg.clone(), gh, gh_mac, gh_linux);
                }
                _ => {
                    let why = "the GitHub CLI isn't installed or logged in (Setup tab)";
                    if gh_mac {
                        lock(&self.jobs.mac).set(Status::Failed(why.into()));
                    }
                    if gh_linux {
                        lock(&self.jobs.linux).set(Status::Failed(why.into()));
                    }
                }
            }
        }
        if cfg.win || local_linux {
            local_thread(self.jobs.clone(), ctl, ctx.clone(), cfg.clone(), p, cfg.win, local_linux);
        }
        self.tab = if cfg.win {
            Tab::Windows
        } else if local_linux {
            Tab::Linux
        } else {
            Tab::GitHub
        };
    }
}

fn status_line(job: &Shared) -> (String, Color32) {
    let j = lock(job);
    let t = mmss(j.elapsed());
    match &j.status {
        Status::Idle => ("-".into(), Color32::GRAY),
        Status::Queued => ("waiting".into(), Color32::LIGHT_GRAY),
        Status::Running(what) => {
            let crates = if j.crates > 0 { format!(", {} crates compiled", j.crates) } else { String::new() };
            (format!("{what}  {t}{crates}"), Color32::from_rgb(120, 180, 255))
        }
        Status::Done(s) => (format!("done in {}", mmss(*s)), Color32::from_rgb(110, 210, 110)),
        Status::Failed(why) => (format!("failed: {why}"), Color32::from_rgb(240, 110, 100)),
        Status::Cancelled => ("cancelled".into(), Color32::from_rgb(230, 190, 90)),
    }
}

fn ok(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(Color32::from_rgb(110, 210, 110)));
}
fn warn(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(Color32::from_rgb(230, 190, 90)));
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let p = self.probe();
        let busy = self.jobs.busy();
        if busy || p.running {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if !self.version_hint_used && self.cfg.version.trim().is_empty() {
            if let Some(v) = p.last_release.as_deref().and_then(next_minor) {
                self.cfg.version = v;
                self.version_hint_used = true;
            }
        }
        let root_ok = is_project(&self.cfg.root());
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.heading(TITLE);
            ui.add_space(4.0);
            egui::Grid::new("cfg").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label("Project folder");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.cfg.root).desired_width(420.0));
                    if ui.button("Browse...").clicked() {
                        if let Some(d) = rfd::FileDialog::new().set_directory(self.cfg.root()).pick_folder() {
                            self.cfg.root = d.display().to_string();
                        }
                    }
                    if root_ok {
                        ok(ui, "found");
                    } else {
                        warn(ui, "not the dx-rust folder");
                    }
                });
                ui.end_row();
                ui.label("Version");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.cfg.version).desired_width(90.0).hint_text("0.12.0"));
                    if let Some(r) = &p.last_release {
                        ui.label(RichText::new(format!("last release {r}")).weak());
                    }
                    ui.add_space(16.0);
                    ui.checkbox(&mut self.cfg.fast, "Faster build (thin LTO)")
                        .on_hover_text("Quicker to build and lighter on memory; the game runs a little slower.");
                });
                ui.end_row();
            });
            ui.separator();
            egui::Grid::new("plats").num_columns(3).spacing([14.0, 8.0]).show(ui, |ui| {
                // Windows
                ui.checkbox(&mut self.cfg.win, RichText::new("Windows").strong());
                ui.horizontal(|ui| {
                    ui.label("here, with Rust");
                    match &p.cargo {
                        Some(v) => ok(ui, v),
                        None if p.running => warn(ui, "checking..."),
                        None => warn(ui, "Rust not found (install it from rustup.rs)"),
                    };
                });
                let (s, c) = status_line(&self.jobs.win);
                ui.label(RichText::new(s).color(c));
                ui.end_row();
                // Linux / Steam Deck
                ui.checkbox(&mut self.cfg.linux, RichText::new("Linux / Steam Deck").strong());
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.cfg.linux_github, false, "in WSL here");
                    ui.radio_value(&mut self.cfg.linux_github, true, "on GitHub");
                    if !self.cfg.linux_github {
                        match (&p.distros, p.linux_ready) {
                            (None, _) if p.running => warn(ui, "checking..."),
                            (None, _) => warn(ui, "WSL not installed (Setup tab)"),
                            (Some(_), Some(true)) => ok(ui, "ready"),
                            (Some(_), _) => warn(ui, "needs its tools (Setup tab)"),
                        }
                    }
                });
                let (s, c) = status_line(&self.jobs.linux);
                ui.label(RichText::new(s).color(c));
                ui.end_row();
                // macOS
                ui.checkbox(&mut self.cfg.mac, RichText::new("macOS").strong());
                ui.horizontal(|ui| {
                    ui.label("on GitHub's Macs");
                    match (&p.gh, &p.gh_user) {
                        (_, Some(u)) => ok(ui, &format!("logged in as {u}")),
                        (None, _) if p.running => warn(ui, "checking..."),
                        (None, _) => warn(ui, "GitHub CLI not installed (Setup tab)"),
                        (Some(_), None) => warn(ui, "not logged in to GitHub (Setup tab)"),
                    }
                });
                let (s, c) = status_line(&self.jobs.mac);
                ui.label(RichText::new(s).color(c));
                ui.end_row();
            });
            if (self.cfg.mac || (self.cfg.linux && self.cfg.linux_github)) && p.gh_user.is_some() {
                let what = p.commit.clone().unwrap_or_else(|| "?".into());
                ui.label(
                    RichText::new(format!(
                        "GitHub builds use the code on GitHub ({} {}: {what}), not this folder.",
                        self.cfg.repo, self.cfg.branch
                    ))
                    .weak(),
                );
                if p.workflow == Some(false) {
                    warn(ui, &format!("{WORKFLOW} isn't on GitHub yet: push the project's .github folder first."));
                }
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let any = self.cfg.win || self.cfg.linux || self.cfg.mac;
                let go = ui.add_enabled(
                    !busy && root_ok && any,
                    egui::Button::new(RichText::new("  Build selected  ").strong()).min_size([150.0, 28.0].into()),
                );
                if go.clicked() {
                    self.build(ctx);
                }
                if ui.add_enabled(busy, egui::Button::new("Cancel").min_size([80.0, 28.0].into())).clicked() {
                    self.ctl.request_cancel();
                }
                if ui.add(egui::Button::new("Open dist folder").min_size([120.0, 28.0].into())).clicked() {
                    let d = self.cfg.dist();
                    let _ = std::fs::create_dir_all(&d);
                    let _ = std::process::Command::new("explorer").arg(&d).spawn();
                }
                let can_release = !busy && p.gh_user.is_some() && root_ok;
                let rel = ui
                    .add_enabled(can_release, egui::Button::new("Draft GitHub release").min_size([150.0, 28.0].into()))
                    .on_hover_text(format!(
                        "Makes a draft release v{} on GitHub with the archives in dist\\ and the CHANGELOG's \
                         Unreleased notes. It stays a draft until you publish it on GitHub.",
                        self.cfg.version()
                    ));
                if rel.clicked() {
                    self.cfg.save();
                    let ctl = self.fresh_ctl();
                    lock(&self.jobs.github).set(Status::Queued);
                    release_thread(self.jobs.clone(), ctl, ctx.clone(), self.cfg.clone(), p.gh.clone().unwrap_or_default());
                    self.tab = Tab::GitHub;
                }
                if ui.add_enabled(!p.running, egui::Button::new("Refresh").min_size([70.0, 28.0].into())).clicked() {
                    self.cfg.save();
                    self.refresh(ctx);
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                for (t, name, job) in [
                    (Tab::Windows, "Windows", &self.jobs.win),
                    (Tab::Linux, "Linux", &self.jobs.linux),
                    (Tab::Mac, "macOS", &self.jobs.mac),
                    (Tab::GitHub, "GitHub", &self.jobs.github),
                    (Tab::Setup, "Setup", &self.jobs.setup),
                ] {
                    let st = lock(job).status.clone();
                    let mark = match st {
                        Status::Running(_) | Status::Queued => " ...",
                        Status::Failed(_) => " !",
                        Status::Done(_) => " ok",
                        _ => "",
                    };
                    ui.selectable_value(&mut self.tab, t, format!("{name}{mark}"));
                }
            });
            ui.add_space(2.0);
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.tab == Tab::Setup {
                self.setup_ui(ui, ctx, &p, busy);
                ui.separator();
            }
            let job = match self.tab {
                Tab::Windows => &self.jobs.win,
                Tab::Linux => &self.jobs.linux,
                Tab::Mac => &self.jobs.mac,
                Tab::GitHub => &self.jobs.github,
                Tab::Setup => &self.jobs.setup,
            };
            let j = lock(job);
            let row = ui.text_style_height(&egui::TextStyle::Monospace);
            egui::ScrollArea::both().auto_shrink([false, false]).stick_to_bottom(true).show_rows(
                ui,
                row,
                j.log.len().max(1),
                |ui, range| {
                    if j.log.is_empty() {
                        ui.label(RichText::new("Nothing here yet.").weak());
                    }
                    for l in &j.log[range.start.min(j.log.len())..range.end.min(j.log.len())] {
                        let color = if l.starts_with("== FAILED") || l.starts_with("error") {
                            Color32::from_rgb(240, 120, 110)
                        } else if l.starts_with("warning") {
                            Color32::from_rgb(220, 190, 100)
                        } else if l.starts_with("==") || l.starts_with("Built ") || l.starts_with("> ") {
                            Color32::from_rgb(140, 190, 255)
                        } else {
                            Color32::from_gray(200)
                        };
                        ui.label(RichText::new(l).monospace().color(color));
                    }
                },
            );
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.cfg.save();
        self.ctl.request_cancel();
    }
}

impl App {
    fn setup_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, p: &Probe, busy: bool) {
        ui.label(RichText::new("Linux / Steam Deck in WSL").strong());
        match &p.distros {
            None => {
                ui.label(
                    "WSL runs Linux inside Windows; the Linux release is built there. Install it (Windows asks for \
                     permission, and may want a restart), then open Ubuntu from the Start menu once to choose a \
                     user name and password, then come back and press Refresh.",
                );
                if ui.button("Install WSL with Ubuntu").clicked() {
                    let _ = std::process::Command::new("powershell.exe")
                        .args([
                            "-NoProfile",
                            "-Command",
                            "Start-Process wsl.exe -ArgumentList '--install','-d','Ubuntu' -Verb RunAs",
                        ])
                        .spawn();
                }
            }
            Some(ds) => {
                ui.horizontal(|ui| {
                    ui.label("Linux to build in");
                    let cur = if ds.contains(&self.cfg.distro) { self.cfg.distro.clone() } else { ds[0].clone() };
                    egui::ComboBox::from_id_salt("distro").selected_text(&cur).show_ui(ui, |ui| {
                        for d in ds {
                            ui.selectable_value(&mut self.cfg.distro, d.clone(), d);
                        }
                    });
                    match p.linux_ready {
                        Some(true) => ok(ui, "build tools ready"),
                        _ => warn(ui, "build tools not installed yet"),
                    }
                });
                ui.label(
                    "Set Up installs the Linux packages, Rust, zig and cargo-zigbuild in that Linux (a few minutes, \
                     once). Ubuntu or Debian.",
                );
                if ui.add_enabled(!busy, egui::Button::new("Set up Linux build tools")).clicked() {
                    let ds = ds.clone();
                    let d = if ds.contains(&self.cfg.distro) { self.cfg.distro.clone() } else { ds[0].clone() };
                    self.cfg.save();
                    let ctl = self.fresh_ctl();
                    lock(&self.jobs.setup).set(Status::Queued);
                    setup_thread(self.jobs.clone(), ctl, ctx.clone(), self.cfg.clone(), d, self.probe.clone());
                }
            }
        }
        ui.add_space(8.0);
        ui.label(RichText::new("GitHub (macOS, and Linux on GitHub)").strong());
        ui.label(
            "macOS can only be built on a Mac, so it's built on GitHub's Macs and downloaded here. That needs the \
             GitHub CLI, logged in to an account that can run the project's Actions.",
        );
        ui.horizontal(|ui| {
            match (&p.gh, &p.gh_user) {
                (None, _) => {
                    if ui.button("Install GitHub CLI").clicked() {
                        let args = ["install", "--id", "GitHub.cli", "-e", "--accept-source-agreements"];
                        console_then_refresh("winget", &args, self.probe.clone(), self.cfg.clone(), ctx.clone());
                    }
                    ui.label(RichText::new("(or get it from cli.github.com)").weak());
                }
                (Some(gh), None) => {
                    if ui.button("Log in to GitHub").clicked() {
                        let args = ["auth", "login", "-h", "github.com", "-p", "https", "-w"];
                        console_then_refresh(gh, &args, self.probe.clone(), self.cfg.clone(), ctx.clone());
                    }
                    ui.label(RichText::new("(a browser opens; paste the code it shows)").weak());
                }
                (Some(_), Some(u)) => ok(ui, &format!("GitHub CLI logged in as {u}")),
            }
        });
        ui.horizontal(|ui| {
            ui.label("Repository");
            ui.add(egui::TextEdit::singleline(&mut self.cfg.repo).desired_width(200.0));
            ui.label("branch");
            ui.add(egui::TextEdit::singleline(&mut self.cfg.branch).desired_width(80.0));
        });
    }
}

fn main() -> eframe::Result<()> {
    let app = App::new();
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([980.0, 680.0])
            .with_min_inner_size([760.0, 480.0])
            .with_title(TITLE)
            .with_icon(Arc::new(egui::IconData {
                rgba: include_bytes!("../../../assets/dxrust_48.rgba").to_vec(),
                width: 48,
                height: 48,
            })),
        ..Default::default()
    };
    eframe::run_native(
        TITLE,
        opts,
        Box::new(move |cc| {
            app.refresh(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsl_paths() {
        assert_eq!(to_wsl(Path::new(r"E:\Games\dx-rust")).as_deref(), Some("/mnt/e/Games/dx-rust"));
        assert_eq!(to_wsl(Path::new(r"D:\a b\")).as_deref(), Some("/mnt/d/a b"));
        assert_eq!(to_wsl(Path::new(r"\\server\share")), None);
    }

    #[test]
    fn versions() {
        assert_eq!(next_minor("v0.11.0").as_deref(), Some("0.12.0"));
        assert_eq!(next_minor("1.2.3").as_deref(), Some("1.3.0"));
    }

    #[test]
    fn notes() {
        let d = std::env::temp_dir().join("dxbuilder_notes_test");
        let _ = std::fs::create_dir_all(&d);
        std::fs::write(d.join("CHANGELOG.md"), "# Changelog\n\n## Unreleased\n\n- a\n- b\n\n## v0.1\n- old\n").unwrap();
        assert_eq!(unreleased_notes(&d), "- a\n- b");
    }
}
