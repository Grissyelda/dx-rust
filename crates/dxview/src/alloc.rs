//! The heap: mimalloc (a fast multi-threaded allocator, much quicker than the Windows heap for Bevy's many small
//! short-lived allocations), wrapped so profile runs (DXVIEW_PROF) can count allocations - per frame and per
//! profiled system (prof::scope records the calling thread's count at its start and end). Counting is two relaxed
//! thread-local adds per allocation and only happens while profiling.

use std::alloc::{GlobalAlloc, Layout};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub struct Counting;

#[global_allocator]
static GLOBAL: Counting = Counting;

static COUNT: AtomicBool = AtomicBool::new(false);
static TOTAL: AtomicU64 = AtomicU64::new(0);
static TOTAL_BYTES: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static THREAD: Cell<u64> = const { Cell::new(0) };
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
        unsafe { mimalloc::MiMalloc.alloc(layout) }
    }
    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        unsafe { mimalloc::MiMalloc.alloc_zeroed(layout) }
    }
    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { mimalloc::MiMalloc.dealloc(ptr, layout) }
    }
    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note(new_size);
        unsafe { mimalloc::MiMalloc.realloc(ptr, layout, new_size) }
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
