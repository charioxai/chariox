//! Single-flight, throttled reservation for kernel-owned App wake delivery.
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};

const MIN_INTERVAL_MS: u64 = 1_000;
const PRUNE_INTERVAL_MS: u64 = 60_000;

#[derive(Clone, Default)]
pub(crate) struct AppWakePump {
    busy: Arc<AtomicBool>,
    last_ms: Arc<AtomicU64>,
    last_prune_ms: Arc<AtomicU64>,
}

/// Held for one pass; dropping it releases the pump even if the pass panics.
pub(crate) struct WakePass(Arc<AtomicBool>);
impl Drop for WakePass {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl AppWakePump {
    pub(crate) fn try_begin(&self, now_ms: u64) -> Option<WakePass> {
        if now_ms.saturating_sub(self.last_ms.load(Ordering::Acquire)) < MIN_INTERVAL_MS
            || self.busy.swap(true, Ordering::AcqRel)
        {
            return None;
        }
        self.last_ms.store(now_ms, Ordering::Release);
        Some(WakePass(self.busy.clone()))
    }

    /// Whether this pass should also prune stale dormant catalogs (once a minute).
    pub(crate) fn prune_due(&self, now_ms: u64) -> bool {
        if now_ms.saturating_sub(self.last_prune_ms.load(Ordering::Acquire)) < PRUNE_INTERVAL_MS {
            return false;
        }
        self.last_prune_ms.store(now_ms, Ordering::Release);
        true
    }
}
