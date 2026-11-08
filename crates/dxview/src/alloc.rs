//! The heap: mimalloc (a fast multi-threaded allocator, much quicker than the Windows heap for Bevy's many small
//! short-lived allocations), wrapped so profile runs (DXVIEW_PROF) can count allocations - per frame and per
//! profiled system (prof::scope records the calling thread's count at its start and end). Counting is two relaxed
//! thread-local adds per allocation and only happens while profiling.

use std::alloc::{GlobalAlloc, Layout};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub struct Counting;

// Android: the system heap (mimalloc's C build isn't set up for the NDK here).
#[cfg(not(target_os = "android"))]
use mimalloc::MiMalloc as Inner;
#[cfg(target_os = "android")]
use std::alloc::System as Inner;

#[global_allocator]
static GLOBAL: Counting = Counting;

static COUNT: AtomicBool = AtomicBool::new(false);
static TOTAL: AtomicU64 = AtomicU64::new(0);
static TOTAL_BYTES: AtomicU64 = AtomicU64::new(0);
/// Bytes allocated and not yet freed (counted from the start of a profile run; what was allocated before it and
/// freed later makes this read a little low).
static LIVE_BYTES: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

#[inline]
fn live(delta: i64) {
    if COUNT.load(Ordering::Relaxed) {
        LIVE_BYTES.fetch_add(delta, Ordering::Relaxed);
    }
}

/// The heap's live bytes (profile runs only).
pub fn live_bytes() -> i64 {
    LIVE_BYTES.load(Ordering::Relaxed)
}

thread_local! {
    static THREAD: Cell<u64> = const { Cell::new(0) };
}

// ---- DXVIEW_HEAPPROF=1: who holds the big blocks ----------------------------------------------------------------
// Every allocation of 256 KB or more is remembered with its call stack until it is freed; heap_report() prints the
// live total by stack. Big blocks are rare, so taking a stack for each costs nothing noticeable; small ones aren't
// tracked (they're counted by the per-frame counters above).

const BIG: usize = 256 << 10;
static HEAPPROF: AtomicBool = AtomicBool::new(false);
type Live = std::collections::HashMap<usize, (usize, std::backtrace::Backtrace)>;
static LIVE: std::sync::Mutex<Option<Live>> = std::sync::Mutex::new(None);

thread_local! {
    static IN_HOOK: Cell<bool> = const { Cell::new(false) };
}

pub fn enable_heap_profile() {
    HEAPPROF.store(true, Ordering::Relaxed);
}

#[inline]
fn track_alloc(ptr: *mut u8, size: usize) {
    if size < BIG || ptr.is_null() || !HEAPPROF.load(Ordering::Relaxed) {
        return;
    }
    let _ = IN_HOOK.try_with(|h| {
        if h.get() {
            return;
        }
        h.set(true);
        let bt = std::backtrace::Backtrace::force_capture();
        if let Ok(mut g) = LIVE.lock() {
            g.get_or_insert_with(Default::default).insert(ptr as usize, (size, bt));
        }
        h.set(false);
    });
}

#[inline]
fn track_free(ptr: *mut u8, size: usize) {
    if size < BIG || !HEAPPROF.load(Ordering::Relaxed) {
        return;
    }
    let _ = IN_HOOK.try_with(|h| {
        if h.get() {
            return;
        }
        h.set(true);
        let gone = LIVE.lock().ok().and_then(|mut g| g.as_mut().and_then(|m| m.remove(&(ptr as usize))));
        drop(gone);
        h.set(false);
    });
}

/// The live big blocks grouped by where they were made (the first frames of ours or the engine's), largest first.
pub fn heap_report(label: &str) {
    if !HEAPPROF.load(Ordering::Relaxed) {
        return;
    }
    IN_HOOK.with(|h| h.set(true));
    // Taken out of the lock while the stacks are resolved (resolving takes std's backtrace lock, which a thread
    // capturing a stack holds while it waits for this one), then put back.
    let taken = std::mem::take(&mut *LIVE.lock().unwrap_or_else(|e| e.into_inner())).unwrap_or_default();
    let snapshot: Vec<(usize, String)> = taken.values().map(|(sz, bt)| (*sz, site(bt))).collect();
    {
        let mut g = LIVE.lock().unwrap_or_else(|e| e.into_inner());
        let m = g.get_or_insert_with(Default::default);
        for (k, v) in taken {
            m.entry(k).or_insert(v);
        }
    }
    let mut by: std::collections::HashMap<String, (usize, usize)> = Default::default();
    for (sz, k) in snapshot {
        let e = by.entry(k).or_default();
        e.0 += sz;
        e.1 += 1;
    }
    let mut v: Vec<_> = by.into_iter().collect();
    v.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
    let total: usize = v.iter().map(|x| x.1 .0).sum();
    println!("HEAP {label}: {:.0} MB live in blocks of 256 KB or more", total as f64 / 1048576.0);
    for (k, (b, n)) in v.iter().take(25) {
        println!("HEAP   {:8.1} MB x{n:<5} {k}", *b as f64 / 1048576.0);
    }
    IN_HOOK.with(|h| h.set(false));
}

/// A call stack's signature: the first few frames that are ours (dxview / dx_level / unreal_pkg / imsim) plus the
/// engine frame just above them.
fn site(bt: &std::backtrace::Backtrace) -> String {
    let text = bt.to_string();
    let mut out: Vec<String> = Vec::new();
    let mut last_other = String::new();
    for line in text.lines() {
        let l = line.trim();
        let Some((_, name)) = l.split_once(": ") else { continue };
        if name.starts_with("std::") || name.starts_with("core::") || name.starts_with("alloc::") || name.contains("dxview::alloc") || name.contains("mimalloc") || name.starts_with("<alloc::") {
            continue;
        }
        let short: String = name.split("::h").next().unwrap_or(name).chars().take(90).collect();
        let ours = ["dxview::", "dx_level::", "unreal_pkg::", "imsim_"].iter().any(|p| short.contains(p));
        if ours {
            if out.is_empty() && !last_other.is_empty() {
                out.push(last_other.clone());
            }
            out.push(short);
            if out.len() >= 4 {
                break;
            }
        } else if out.is_empty() {
            last_other = short;
        }
    }
    if out.is_empty() {
        // Nothing of ours on the stack: the first few named frames.
        for line in text.lines() {
            let Some((_, name)) = line.trim().split_once(": ") else { continue };
            if name.starts_with("std::") || name.starts_with("core::") || name.starts_with("alloc::") || name.contains("dxview::alloc") || name.starts_with("<alloc::") || name.starts_with("__rustc") {
                continue;
            }
            out.push(name.split("::h").next().unwrap_or(name).chars().take(70).collect());
            if out.len() >= 4 {
                break;
            }
        }
    }
    out.join(" < ")
}

#[inline]
fn note(size: usize) {
    if COUNT.load(Ordering::Relaxed) {
        let _ = THREAD.try_with(|c| c.set(c.get() + 1));
        TOTAL.fetch_add(1, Ordering::Relaxed);
        TOTAL_BYTES.fetch_add(size as u64, Ordering::Relaxed);
    }
}

unsafe impl GlobalAlloc for Counting {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        live(layout.size() as i64);
        let p = unsafe { Inner.alloc(layout) };
        track_alloc(p, layout.size());
        p
    }
    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        live(layout.size() as i64);
        let p = unsafe { Inner.alloc_zeroed(layout) };
        track_alloc(p, layout.size());
        p
    }
    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        track_free(ptr, layout.size());
        live(-(layout.size() as i64));
        unsafe { Inner.dealloc(ptr, layout) }
    }
    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note(new_size);
        live(new_size as i64 - layout.size() as i64);
        track_free(ptr, layout.size());
        let p = unsafe { Inner.realloc(ptr, layout, new_size) };
        track_alloc(p, new_size);
        p
    }
}

/// Start counting (profile runs only).
pub fn enable() {
    COUNT.store(true, Ordering::Relaxed);
}

pub fn counting() -> bool {
    COUNT.load(Ordering::Relaxed)
}

/// Allocations made on this thread so far.
#[inline]
pub fn thread_count() -> u64 {
    THREAD.try_with(|c| c.get()).unwrap_or(0)
}

/// Allocations and bytes so far, every thread.
pub fn totals() -> (u64, u64) {
    (TOTAL.load(Ordering::Relaxed), TOTAL_BYTES.load(Ordering::Relaxed))
}

/// The process as the OS sees it: resident and committed memory (bytes) and CPU time (ms).
#[derive(Clone, Copy, Default, Debug)]
pub struct ProcInfo {
    pub elapsed_ms: usize,
    pub user_ms: usize,
    pub system_ms: usize,
    pub rss: usize,
    pub peak_rss: usize,
    pub commit: usize,
    pub peak_commit: usize,
    pub page_faults: usize,
}

#[cfg(not(target_os = "android"))]
pub fn process() -> Option<ProcInfo> {
    let mut p = ProcInfo::default();
    unsafe {
        libmimalloc_sys::mi_process_info(
            &mut p.elapsed_ms,
            &mut p.user_ms,
            &mut p.system_ms,
            &mut p.rss,
            &mut p.peak_rss,
            &mut p.commit,
            &mut p.peak_commit,
            &mut p.page_faults,
        );
    }
    // On Linux mimalloc reports its own committed bytes as "current RSS": the real figure is in /proc.
    #[cfg(target_os = "linux")]
    if let Ok(st) = std::fs::read_to_string("/proc/self/status") {
        let kb = |k: &str| st.lines().find(|l| l.starts_with(k)).and_then(|l| l.split_whitespace().nth(1)?.parse::<usize>().ok());
        if let Some(v) = kb("VmRSS:") {
            p.rss = v * 1024;
        }
        if let Some(v) = kb("VmHWM:") {
            p.peak_rss = v * 1024;
        }
    }
    Some(p)
}

#[cfg(target_os = "android")]
pub fn process() -> Option<ProcInfo> {
    None
}

/// Hand freed heap pages back to the OS (after a map swap: the old level's buffers are gone by then).
pub fn collect() {
    #[cfg(not(target_os = "android"))]
    unsafe {
        libmimalloc_sys::mi_collect(true);
    }
}
