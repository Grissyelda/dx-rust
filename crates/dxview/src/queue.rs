//! Cross-system queues (a `Mutex<Vec<T>>` filled by any system, emptied by one) without a fresh Vec each frame:
//! the consumer swaps its own cleared buffer in, so both buffers keep their capacity and a queue that fills every
//! frame (held lights, held sounds) stops reallocating.

use std::sync::Mutex;

/// Move everything queued into `buf` (cleared first) and leave `buf`'s old storage in the queue.
pub fn swap_out<T>(q: &Mutex<Vec<T>>, buf: &mut Vec<T>) {
    buf.clear();
    if let Ok(mut g) = q.lock() {
        std::mem::swap(&mut *g, buf);
    }
}
