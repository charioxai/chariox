//! MP-08/MP-10/MP-11: numeric-only recovery phases; no identities or payloads.
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

static NEXT: AtomicU64 = AtomicU64::new(1);

pub(in crate::runtime::state) struct ResumeTiming {
    id: u64,
    epoch_ms: u64,
    start: Instant,
}

impl ResumeTiming {
    pub(in crate::runtime::state) fn start() -> Option<Self> {
        if std::env::var("CHARIOX_BROWSER_DISPLAY_TIMING").as_deref() != Ok("1") {
            return None;
        }
        Some(Self {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            epoch_ms: crate::session::unix_epoch_ms(),
            start: Instant::now(),
        })
    }

    pub(in crate::runtime::state) fn mark(&self, stage: &'static str) {
        eprintln!(
            "[chariox.room_display_resume] {} {} {} {} {}",
            self.id,
            stage,
            self.epoch_ms,
            crate::session::unix_epoch_ms(),
            self.start.elapsed().as_millis()
        );
    }
}
