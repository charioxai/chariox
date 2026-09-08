//! Bounded restart scan on the existing kernel runtime pump.
use super::*;
impl AppLifecycleService {
    /// The existing runtime pump calls this only after its publication gate.
    /// One short blocking scan runs at a time; no worker/native I/O is awaited
    /// by the async coordinator or by this recovery scan.
    pub(crate) fn schedule_recovery(&self, runtime: Handle) {
        if self.0.stopped.load(Ordering::Acquire) {
            return;
        }
        let Ok(mut state) = self.0.maintenance.lock() else {
            return;
        };
        if state.running || Instant::now() < state.next {
            return;
        }
        let Ok(permit) = self.0.admission.clone().try_acquire_owned() else {
            return;
        };
        state.running = true;
        state.next = Instant::now() + RECOVERY_INTERVAL;
        let cursor = state.cursor.clone();
        drop(state);
        let service = self.clone();
        let task_runtime = runtime.clone();
        runtime.spawn_blocking(move || {
            let _permit = permit;
            service.reap_finished();
            let rows = service.0.store.app_worker_recovery_candidates(
                cursor
                    .as_ref()
                    .map(|(owner, installation)| (owner.as_str(), installation.as_str())),
            );
            let mut next = cursor;
            if let Ok(rows) = rows {
                next = rows.last().cloned();
                for (owner, installation) in &rows {
                    if service.0.stopped.load(Ordering::Acquire) {
                        break;
                    }
                    let _ = service.start(owner, installation, true, task_runtime.clone());
                }
                if rows.len() < 8 {
                    next = None;
                }
            }
            let mut state = service
                .0
                .maintenance
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.running = false;
            state.cursor = next;
        });
    }
}
