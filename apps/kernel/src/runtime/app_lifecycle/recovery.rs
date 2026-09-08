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
        let first = state.first_next;
        let cursor = if first {
            state.first_cursor.clone()
        } else {
            state.cursor.clone()
        };
        drop(state);
        let service = self.clone();
        let task_runtime = runtime.clone();
        runtime.spawn_blocking(move || {
            let _permit = permit;
            service.persist_pending_manual_stops();
            service.reap_finished();
            let after = cursor
                .as_ref()
                .map(|(owner, id)| (owner.as_str(), id.as_str()));
            let rows = if first {
                service
                    .0
                    .store
                    .first_app_install_recovery_candidates(after)
                    .map_err(LifecycleError::from)
            } else {
                service
                    .0
                    .store
                    .app_worker_recovery_candidates(after)
                    .map_err(LifecycleError::from)
            };
            let mut next = cursor;
            if let Ok(rows) = rows {
                next = rows.last().cloned();
                for (owner, installation) in &rows {
                    if service.0.stopped.load(Ordering::Acquire) {
                        break;
                    }
                    if first {
                        let _ =
                            service.start_first_blocking(owner, installation, task_runtime.clone());
                    } else {
                        let _ = service.start(owner, installation, true, task_runtime.clone());
                    }
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
            if first {
                state.first_cursor = next;
            } else {
                state.cursor = next;
            }
            // One page total per pass; pending first installs cannot starve
            // current-generation recovery or cause an unbounded second scan.
            state.first_next = !first;
        });
    }
}
