//! Running the build tools: each job is a list of commands whose output streams into its log pane (and a log file
//! in dist\), one command at a time, with Cancel killing the one that is running.

use eframe::egui;
use std::io::{Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Status {
    #[default]
    Idle,
    Queued,
    Running(String),
    Done(f32),
    Failed(String),
    Cancelled,
}

impl Status {
    pub fn busy(&self) -> bool {
        matches!(self, Status::Queued | Status::Running(_))
    }
}

/// Log lines kept for the pane (the log file keeps everything).
const KEEP: usize = 8000;

#[derive(Default)]
pub struct Job {
    pub status: Status,
    pub log: Vec<String>,
    pub started: Option<Instant>,
    /// "Compiling ..." lines seen in this run: a rough progress count.
    pub crates: usize,
    file: Option<std::fs::File>,
}

pub type Shared = Arc<Mutex<Job>>;

pub fn lock(j: &Shared) -> std::sync::MutexGuard<'_, Job> {
    j.lock().unwrap_or_else(|e| e.into_inner())
}

impl Job {
    pub fn begin(&mut self, log_file: Option<std::path::PathBuf>) {
        self.log.clear();
        self.crates = 0;
        self.started = Some(Instant::now());
        self.file = log_file.and_then(|p| std::fs::File::create(p).ok());
    }
    pub fn line(&mut self, s: &str) {
        let t = s.trim_start();
        if t.starts_with("Compiling ") {
            self.crates += 1;
        }
        if let Some(f) = &mut self.file {
            let _ = writeln!(f, "{s}");
        }
        self.log.push(s.to_string());
        if self.log.len() > KEEP {
            self.log.drain(..KEEP / 8);
        }
    }
    pub fn set(&mut self, st: Status) {
        if let Status::Failed(why) = &st {
            let w = format!("== FAILED: {why}");
            self.line(&w);
        }
        self.status = st;
    }
    pub fn elapsed(&self) -> f32 {
        self.started.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0)
    }
    pub fn finish(&mut self) {
        let secs = self.elapsed();
        self.line(&format!("== done in {}", mmss(secs)));
        self.status = Status::Done(secs);
        self.file = None;
    }
}

pub fn mmss(secs: f32) -> String {
    let s = secs as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// What a job's thread shares with the window: Cancel and the command running now.
#[derive(Default)]
pub struct Ctl {
    pub cancel: AtomicBool,
    pub child: Mutex<Option<Child>>,
}

impl Ctl {
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(c) = self.child.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            let _ = c.kill();
        }
    }
}

/// A command with no console window of its own, and plain (uncoloured, UTF-8) output from cargo and wsl.exe.
pub fn cmd(program: &str) -> Command {
    let mut c = Command::new(program);
    c.env("CARGO_TERM_COLOR", "never").env("WSL_UTF8", "1").env("GH_PROMPT_DISABLED", "1").env("NO_COLOR", "1");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// Bytes of one output line as text. wsl.exe's own messages can come as UTF-16: the zero bytes are dropped.
fn decode(mut b: Vec<u8>) -> String {
    if b.iter().filter(|x| **x == 0).count() * 3 > b.len() {
        b.retain(|x| *x != 0);
    }
    let s = String::from_utf8_lossy(&b).into_owned();
    strip_ansi(&s)
}

fn strip_ansi(s: &str) -> String {
    if !s.contains('\u{1b}') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\u{1b}' {
            if it.peek() == Some(&'[') {
                it.next();
                for d in it.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn pump(mut r: impl Read + Send + 'static, job: Shared, ctx: egui::Context) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        let mut cur = Vec::new();
        loop {
            let n = match r.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            for &b in &buf[..n] {
                if b == b'\n' || b == b'\r' {
                    if cur.iter().any(|x| *x != 0) {
                        let s = decode(std::mem::take(&mut cur));
                        if !s.trim().is_empty() {
                            lock(&job).line(&s);
                        }
                    }
                    cur.clear();
                } else {
                    cur.push(b);
                }
            }
            ctx.request_repaint();
        }
        if cur.iter().any(|x| *x != 0) {
            let s = decode(cur);
            if !s.trim().is_empty() {
                lock(&job).line(&s);
            }
        }
        ctx.request_repaint();
    })
}

/// Runs `c` to the end with its output in the job's log. Err on a failure exit, a start error or Cancel.
pub fn run(job: &Shared, ctl: &Ctl, ctx: &egui::Context, mut c: Command) -> Result<(), String> {
    if ctl.cancelled() {
        return Err("cancelled".into());
    }
    let shown = describe(&c);
    lock(job).line(&format!("> {shown}"));
    c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c.spawn().map_err(|e| format!("couldn't start {}: {e}", c.get_program().to_string_lossy()))?;
    let a = child.stdout.take().map(|o| pump(o, job.clone(), ctx.clone()));
    let b = child.stderr.take().map(|o| pump(o, job.clone(), ctx.clone()));
    *ctl.child.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
    let status: Option<ExitStatus> = loop {
        std::thread::sleep(Duration::from_millis(100));
        let mut g = ctl.child.lock().unwrap_or_else(|e| e.into_inner());
        let Some(ch) = g.as_mut() else { break None };
        match ch.try_wait() {
            Ok(Some(st)) => {
                *g = None;
                break Some(st);
            }
            Ok(None) => {}
            Err(_) => {
                *g = None;
                break None;
            }
        }
    };
    for h in [a, b].into_iter().flatten() {
        let _ = h.join();
    }
    if ctl.cancelled() {
        return Err("cancelled".into());
    }
    match status {
        Some(st) if st.success() => Ok(()),
        Some(st) => Err(format!("{} stopped ({st})", c.get_program().to_string_lossy())),
        None => Err("lost track of the command".into()),
    }
}

/// Runs `c` and returns what it printed on stdout (for the quick checks; no log, no window). None on a failure exit
/// or when it takes longer than 25 s (a WSL Linux that was never opened can sit waiting for its first-run setup).
pub fn capture(c: Command) -> Option<String> {
    match capture_timed(c, Duration::from_secs(25)) {
        Some((true, out, _)) => Some(out),
        _ => None,
    }
}

/// Like capture, but the output whatever the exit code (stdout, then stderr).
pub fn capture_any(c: Command) -> (bool, String) {
    match capture_timed(c, Duration::from_secs(120)) {
        Some((okay, out, err)) => (okay, format!("{out}\n{err}").trim().to_string()),
        None => (false, "timed out".into()),
    }
}

fn capture_timed(mut c: Command, limit: Duration) -> Option<(bool, String, String)> {
    c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c.spawn().ok()?;
    let read = |r: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut b = Vec::new();
            if let Some(mut r) = r {
                let _ = r.read_to_end(&mut b);
            }
            b
        })
    };
    let a = read(child.stdout.take().map(|o| Box::new(o) as Box<dyn Read + Send>));
    let e = read(child.stderr.take().map(|o| Box::new(o) as Box<dyn Read + Send>));
    let t0 = Instant::now();
    let st = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if t0.elapsed() < limit => std::thread::sleep(Duration::from_millis(30)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let out = decode(a.join().unwrap_or_default()).trim().to_string();
    let err = decode(e.join().unwrap_or_default()).trim().to_string();
    Some((st.success(), out, err))
}

fn describe(c: &Command) -> String {
    let mut s = c.get_program().to_string_lossy().into_owned();
    for a in c.get_args() {
        let a = a.to_string_lossy();
        s.push(' ');
        if a.contains(' ') {
            s.push('"');
            s.push_str(&a);
            s.push('"');
        } else {
            s.push_str(&a);
        }
    }
    s
}

/// Single-quoted for bash.
pub fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
