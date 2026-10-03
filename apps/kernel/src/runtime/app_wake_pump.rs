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
        let last_ms = self.last_ms.load(Ordering::Acquire);
        let clock_corrected = last_ms.saturating_sub(now_ms) >= MIN_INTERVAL_MS;
        if (!clock_corrected && now_ms.saturating_sub(last_ms) < MIN_INTERVAL_MS)
            || self.busy.swap(true, Ordering::AcqRel)
        {
            return None;
        }
        self.last_ms.store(now_ms, Ordering::Release);
        Some(WakePass(self.busy.clone()))
    }

    /// Whether this pass should also prune stale dormant catalogs (once a minute).
    pub(crate) fn prune_due(&self, now_ms: u64) -> bool {
        let last_ms = self.last_prune_ms.load(Ordering::Acquire);
        let clock_corrected = last_ms.saturating_sub(now_ms) >= MIN_INTERVAL_MS;
        if !clock_corrected && now_ms.saturating_sub(last_ms) < PRUNE_INTERVAL_MS {
            return false;
        }
        self.last_prune_ms.store(now_ms, Ordering::Release);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backward_clock_corrections_do_not_stall_or_overlap_periodic_passes() {
        let pump = AppWakePump::default();
        let held = pump.try_begin(10_000).unwrap();
        assert!(pump.try_begin(100).is_none());
        drop(held);
        assert!(pump.try_begin(9_999).is_none());
        drop(
            pump.try_begin(100)
                .expect("backward correction resets the throttle"),
        );
        assert!(pump.try_begin(500).is_none());
        assert!(pump.try_begin(1_100).is_some());
        assert!(pump.prune_due(100_000));
        assert!(!pump.prune_due(99_999));
        assert!(pump.prune_due(100));
        assert!(!pump.prune_due(101));
    }
}
