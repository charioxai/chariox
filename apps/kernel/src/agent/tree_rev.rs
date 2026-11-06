//! The revision clients compare to know a sub-agent tree changed. One counter
//! for the kernel, seeded from the clock so it still grows after a restart.
use std::sync::atomic::{AtomicU64, Ordering};

static REV: AtomicU64 = AtomicU64::new(0);

pub(crate) fn current() -> u64 {
    let now = crate::session::unix_epoch_ms();
    // Seed once; `max` keeps a clock step back from lowering it.
    REV.fetch_max(now, Ordering::AcqRel).max(now)
}

pub(crate) fn bump() -> u64 {
    let base = current();
    REV.fetch_max(base + 1, Ordering::AcqRel);
    REV.load(Ordering::Acquire)
}
