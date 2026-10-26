//! Input accepted for a PTY and not yet written to it, and the cancelling of
//! that input when an interrupt overtakes it.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct PtyInputBacklog {
    /// Requests queued or mid-write.
    requests: AtomicUsize,
    /// Raised by an interrupt: requests queued under an older value are
    /// dropped instead of written.
    generation: AtomicU64,
    /// Bytes dropped by interrupts and not yet reported.
    dropped_bytes: AtomicUsize,
}

impl PtyInputBacklog {
    pub(super) fn is_pending(&self) -> bool {
        self.requests.load(Ordering::SeqCst) > 0
    }

    /// Account for a request about to be queued; returns its generation.
    pub(super) fn queued(&self) -> u64 {
        self.requests.fetch_add(1, Ordering::SeqCst);
        self.generation.load(Ordering::SeqCst)
    }

    /// The request could not be queued after all.
    pub(super) fn unqueued(&self) {
        self.requests.fetch_sub(1, Ordering::SeqCst);
    }

    pub(super) fn is_cancelled(&self, generation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) != generation
    }

    /// The writer is done with a request that an interrupt cancelled, with
    /// `left` of its bytes never written.
    pub(super) fn finished_cancelled(&self, left: usize) {
        self.dropped_bytes.fetch_add(left, Ordering::SeqCst);
        self.requests.fetch_sub(1, Ordering::SeqCst);
    }

    /// The writer is done with a request, written or failed.
    pub(super) fn finished(&self) {
        self.requests.fetch_sub(1, Ordering::SeqCst);
    }

    /// Cancel everything queued so far, including the rest of the request
    /// being written.
    pub(super) fn cancel(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Wait for the writer to drop what `cancel` cancelled.
    pub(super) fn wait_until_idle(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while self.is_pending() {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        true
    }

    pub(super) fn take_dropped_bytes(&self) -> usize {
        self.dropped_bytes.swap(0, Ordering::SeqCst)
    }
}
