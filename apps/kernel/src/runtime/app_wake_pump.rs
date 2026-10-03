//! Single-flight, throttled reservation for App inbox and idle maintenance.
//! Periodic ticks are throttled; newly accepted work requests a pass at once.
use std::sync::{Arc, Mutex};

const MIN_INTERVAL_MS: u64 = 1_000;
const PRUNE_INTERVAL_MS: u64 = 60_000;
/// Floor between back-to-back requested passes, so a steady stream of
/// accepted work cannot busy-loop the pump.
pub(crate) const REQUESTED_GAP_MS: u64 = 50;

#[derive(Default)]
struct State {
    busy: bool,
    requested: bool,
    last_ms: u64,
    last_prune_ms: u64,
}

#[derive(Clone, Default)]
pub(crate) struct AppWakePump(Arc<Mutex<State>>);

/// Held for one pass; dropping it releases the pump even if the pass panics.
pub(crate) struct WakePass(Option<Arc<Mutex<State>>>);
impl Drop for WakePass {
    fn drop(&mut self) {
        if let Some(state) = self.0.take() {
            lock(&state).busy = false;
        }
    }
}
impl WakePass {
    /// Ends this pass. If work was requested while it ran, the pass is handed
    /// straight to a rerun under the same lock, so no request is lost.
    pub(crate) fn finish(mut self, now_ms: u64) -> Option<WakePass> {
        let shared = self.0.take()?;
        let mut state = lock(&shared);
        if !state.requested {
            state.busy = false;
            return None;
        }
        state.requested = false;
        state.last_ms = now_ms;
        drop(state);
        Some(WakePass(Some(shared)))
    }

    /// Asks for one more pass as soon as this one finishes: it filled its
    /// page, so more work is likely due already.
    pub(crate) fn again(&self) {
        if let Some(state) = &self.0 {
            lock(state).requested = true;
        }
    }
}

fn lock(state: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl AppWakePump {
    pub(crate) fn try_begin(&self, now_ms: u64) -> Option<WakePass> {
        let mut state = lock(&self.0);
        let clock_corrected = state.last_ms.saturating_sub(now_ms) >= MIN_INTERVAL_MS;
        if state.busy
            || (!clock_corrected && now_ms.saturating_sub(state.last_ms) < MIN_INTERVAL_MS)
        {
            return None;
        }
        state.busy = true;
        state.last_ms = now_ms;
        Some(WakePass(Some(self.0.clone())))
    }

    /// Work was just accepted (an inbox occurrence): a pass runs now, past
    /// the throttle, or, while one runs, once more right after it. Retries of
    /// undelivered work keep their durable due times either way.
    pub(crate) fn try_begin_requested(&self, now_ms: u64) -> Option<WakePass> {
        let mut state = lock(&self.0);
        if state.busy {
            state.requested = true;
            return None;
        }
        state.busy = true;
        state.last_ms = now_ms;
        Some(WakePass(Some(self.0.clone())))
    }

    /// Whether this pass should also prune stale dormant catalogs (once a minute).
    pub(crate) fn prune_due(&self, now_ms: u64) -> bool {
        let mut state = lock(&self.0);
        let clock_corrected = state.last_prune_ms.saturating_sub(now_ms) >= MIN_INTERVAL_MS;
        if !clock_corrected && now_ms.saturating_sub(state.last_prune_ms) < PRUNE_INTERVAL_MS {
            return false;
        }
        state.last_prune_ms = now_ms;
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
        assert!(pump.try_begin_requested(10_550).is_none());
        // ...but the runner reruns once for it as its pass ends,
        let rerun = tick.finish(10_600).unwrap();
        assert!(pump.try_begin(12_000).is_none());
        // and nothing else is pending once it ran.
        assert!(rerun.finish(10_700).is_none());
        // Idle within the throttle window: accepted work runs at once.
        assert!(pump.try_begin(10_800).is_none());
        let requested = pump.try_begin_requested(10_800).unwrap();
        assert!(requested.finish(10_900).is_none());
        // Ticks are throttled from the last pass of either kind.
        assert!(pump.try_begin(11_500).is_none());
        assert!(pump.try_begin(11_800).is_some());
    }

    #[test]
    fn a_pass_that_filled_its_page_is_rerun_once() {
        let pump = AppWakePump::default();
        let tick = pump.try_begin(10_000).unwrap();
        tick.again();
        // Finishing hands the pump straight to one rerun, which holds it...
        let rerun = tick.finish(10_100).unwrap();
        assert!(pump.try_begin(12_000).is_none());
        // ...and asks for nothing more by itself.
        assert!(rerun.finish(10_200).is_none());
        assert!(pump.try_begin_requested(10_300).is_some());
    }

    #[test]
    fn a_dropped_pass_releases_the_pump() {
        let pump = AppWakePump::default();
        drop(pump.try_begin(10_000).unwrap());
        assert!(pump.try_begin_requested(10_100).is_some());
    }

    #[test]
    fn concurrent_requests_are_never_lost() {
        use std::sync::atomic::{AtomicU64, Ordering};
        let pump = AppWakePump::default();
        let requests = Arc::new(AtomicU64::new(0));
        let served = Arc::new(AtomicU64::new(0));
        let runners: Vec<_> = (0..8)
            .map(|_| {
                let (pump, requests, served) = (pump.clone(), requests.clone(), served.clone());
                std::thread::spawn(move || {
                    for _ in 0..2_000 {
                        requests.fetch_add(1, Ordering::SeqCst);
                        let Some(mut pass) = pump.try_begin_requested(0) else {
                            continue;
                        };
                        loop {
                            // A pass serves every request made before it starts.
                            served.store(requests.load(Ordering::SeqCst), Ordering::SeqCst);
                            match pass.finish(0) {
                                Some(next) => pass = next,
                                None => break,
                            }
                        }
                    }
                })
            })
            .collect();
        for runner in runners {
            runner.join().unwrap();
        }
        assert_eq!(
            served.load(Ordering::SeqCst),
            requests.load(Ordering::SeqCst)
        );
    }
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
