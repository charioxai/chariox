//! Single-flight, throttled reservation for kernel-owned App wake delivery.
//! Periodic ticks are throttled; newly accepted work requests a pass at once.
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
    requested: Arc<AtomicBool>,
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

    /// Work was just accepted (an inbox occurrence): a pass runs now, past
    /// the throttle, or, while one runs, once more right after it. Retries of
    /// undelivered work keep their durable due times either way.
    pub(crate) fn try_begin_requested(&self, now_ms: u64) -> Option<WakePass> {
        self.requested.store(true, Ordering::Release);
        self.begin_requested(now_ms)
    }

    /// The pass that requested work is waiting for, if any. A pass runner
    /// calls this after releasing its pass, so a request made while it ran is
    /// served by it or by the requester, never lost.
    pub(crate) fn begin_requested(&self, now_ms: u64) -> Option<WakePass> {
        if !self.requested.load(Ordering::Acquire) || self.busy.swap(true, Ordering::AcqRel) {
            return None;
        }
        // Everything requested before this point was committed first, so the
        // pass about to run sees it.
        self.requested.store(false, Ordering::Release);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requested_passes_skip_the_throttle_and_coalesce_behind_a_running_pass() {
        let pump = AppWakePump::default();
        let tick = pump.try_begin(10_000).unwrap();
        // Periodic ticks stay single-flight and throttled.
        assert!(pump.try_begin(10_500).is_none());
        // Work accepted during a pass cannot start a second one...
        assert!(pump.try_begin_requested(10_500).is_none());
        drop(tick);
        // ...but the runner picks it up as soon as its pass ends,
        let rerun = pump.begin_requested(10_600).unwrap();
        assert!(pump.begin_requested(10_600).is_none());
        drop(rerun);
        // and nothing else is pending once it ran.
        assert!(pump.begin_requested(10_700).is_none());
        // Idle within the throttle window: accepted work runs at once.
        assert!(pump.try_begin(10_800).is_none());
        let requested = pump.try_begin_requested(10_800).unwrap();
        drop(requested);
        assert!(pump.begin_requested(10_900).is_none());
        // Ticks are throttled from the last pass of either kind.
        assert!(pump.try_begin(11_500).is_none());
        assert!(pump.try_begin(11_800).is_some());
    }
}
