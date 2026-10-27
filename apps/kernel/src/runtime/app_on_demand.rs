//! Bounded on-demand activation and idle eviction, shared by runtime callers.
use crate::{
    durable_state::DurableKernelStateStore, error::DaemonError,
    runtime::app_control::AppControlService,
};
use std::time::Duration;

pub(crate) async fn app_lease_on_demand(
    control: &AppControlService,
    store: &DurableKernelStateStore,
    owner: &str,
    installation: &str,
) -> Result<crate::runtime::app_worker::AppWorkerLease, DaemonError> {
    OnDemandApps { control, store }
        .lease(owner, installation)
        .await
}
pub(crate) async fn evict_idle_app(
    control: &AppControlService,
    store: &DurableKernelStateStore,
    owner: &str,
    installation: &str,
) -> bool {
    OnDemandApps { control, store }
        .evict_idle_app(owner, installation)
        .await
}
struct OnDemandApps<'a> {
    control: &'a AppControlService,
    store: &'a DurableKernelStateStore,
}
fn unavailable() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "app.tools",
        message: "App operation is not currently available to this agent".into(),
    }
}

// Covers a queued victim suspend/drain plus preparation, startup and both
// 30 s reactivation callbacks. Each callback still has its own shorter cap.
const ON_DEMAND_START: Duration = Duration::from_secs(180);
/// When the live-worker limit is full, a worker idle at least this long may
/// be stopped (and kept dormant) to admit an on-demand start.
const EVICTABLE_IDLE_MS: u64 = 60_000;
const EVICTION_RETRY: Duration = Duration::from_secs(1);

impl OnDemandApps<'_> {
    /// Dormant Apps and workers that should run after recovery start on demand.
    /// Pending starts wait for publication; user stops and failures stay refused.
    async fn lease(
        &self,
        owner: &str,
        installation: &str,
    ) -> Result<crate::runtime::app_worker::AppWorkerLease, DaemonError> {
        let control = self.control;
        if let Some(lease) = control.active_app_lease(owner, installation) {
            return Ok(lease);
        }
        let deadline = tokio::time::Instant::now() + ON_DEMAND_START;
        if !control.is_app_dormant(owner, installation) {
            if let Some(lease) = control
                .wait_for_app_lease(owner, installation, deadline)
                .await
            {
                return Ok(lease);
            }
            let status = self.app_worker_status(owner, installation).await;
            if !starts_on_demand(status.as_ref()) {
                return Err(unavailable());
            }
        }
        let mut started = false;
        let mut next_eviction = tokio::time::Instant::now();
        let mut eviction_queued = false;
        loop {
            if let Some(lease) = control.active_app_lease(owner, installation) {
                return Ok(lease);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(unavailable());
            }
            if !started {
                let lifecycle = control.lifecycle().clone();
                let (start_owner, start_installation) = (owner.to_owned(), installation.to_owned());
                let handle = tokio::runtime::Handle::current();
                match tokio::task::spawn_blocking(move || {
                    lifecycle.start_on_demand_blocking(&start_owner, &start_installation, handle)
                })
                .await
                .map_err(|_| unavailable())?
                {
                    Ok(_) => started = true,
                    // Transient contention (a concurrent start, preparation or
                    // admission) clears by itself: retry without evicting.
                    Err(crate::runtime::app_lifecycle::LifecycleError::Busy) => {}
                    // Retry queued eviction at a fixed rate if another start takes the slot.
                    Err(crate::runtime::app_lifecycle::LifecycleError::LiveLimit) => {
                        if tokio::time::Instant::now() >= next_eviction {
                            next_eviction = tokio::time::Instant::now() + EVICTION_RETRY;
                            let queued = self.evict_idle_app(owner, installation).await;
                            if !queued && !eviction_queued {
                                return Err(unavailable());
                            }
                            eviction_queued |= queued;
                        }
                    }
                    Err(_) => {
                        control.forget_app_dormant(owner, installation);
                        return Err(unavailable());
                    }
                }
            } else if self.app_start_failed(owner, installation).await {
                control.forget_app_dormant(owner, installation);
                return Err(unavailable());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Stop the least-recently-used idle worker (other than the target),
    /// keeping it dormant, so an on-demand start can take its live slot.
    async fn evict_idle_app(&self, owner: &str, installation: &str) -> bool {
        let control = self.control.clone();
        let now = crate::session::unix_epoch_ms();
        let mut leases = control.active_app_leases(None, 16);
        let candidates: Vec<_> = leases
            .iter()
            .map(|lease| {
                (
                    lease.owner().to_owned(),
                    lease.catalog().installation_id().to_owned(),
                    lease.idle_ms(now),
                )
            })
            .collect();
        let store = self.store.clone();
        let target = (owner.to_owned(), installation.to_owned());
        // Workers with undelivered events are not idle; rank only the others,
        // so one busy App cannot block eviction of an idle one.
        let Ok(Some(index)) = tokio::task::spawn_blocking(move || {
            let candidates: Vec<_> = candidates
                .iter()
                .map(|(owner, installation, idle)| (owner.as_str(), installation.as_str(), *idle))
                .collect();
            eviction_victim(
                &candidates,
                (&target.0, &target.1),
                |owner, installation| store.has_deliverable_app_events(owner, installation),
            )
        })
        .await
        else {
            return false;
        };
        let lease = leases.swap_remove(index);
        drop(leases);
        let lifecycle = control.lifecycle().clone();
        let store = self.store.clone();
        let victim_owner = lease.owner().to_owned();
        let catalog = lease.catalog().clone();
        drop(lease);
        tokio::task::spawn_blocking(move || {
            let installation = catalog.installation_id().to_owned();
            lifecycle.request_idle_stop_blocking(&victim_owner, catalog, || {
                control
                    .active_app_lease(&victim_owner, &installation)
                    .is_some_and(|lease| {
                        lease.idle_ms(crate::session::unix_epoch_ms()) >= EVICTABLE_IDLE_MS
                    })
                    && !store.has_deliverable_app_events(&victim_owner, &installation)
            })
        })
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or(false)
    }

    async fn app_start_failed(&self, owner: &str, installation: &str) -> bool {
        self.app_worker_status(owner, installation)
            .await
            .is_some_and(|status| {
                status.phase == crate::durable_state::app_worker_lifecycle::WorkerPhase::Failed
            })
    }
    async fn app_worker_status(
        &self,
        owner: &str,
        installation: &str,
    ) -> Option<crate::durable_state::app_worker_lifecycle::WorkerStatus> {
        let store = self.store.clone();
        let (owner, installation) = (owner.to_owned(), installation.to_owned());
        tokio::task::spawn_blocking(move || store.app_worker_status(&owner, &installation))
            .await
            .ok()
            .and_then(Result::ok)
            .flatten()
    }
}

/// The longest-idle worker other than `target`, if idle for at least
/// `EVICTABLE_IDLE_MS` and not `busy`. Candidates are `(owner, installation,
/// idle_ms)`; `busy` is checked only for otherwise eligible workers.
fn eviction_victim(
    candidates: &[(&str, &str, u64)],
    target: (&str, &str),
    busy: impl Fn(&str, &str) -> bool,
) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .filter(|(_, (owner, installation, idle))| {
            (*owner, *installation) != target
                && *idle >= EVICTABLE_IDLE_MS
                && !busy(owner, installation)
        })
        .max_by_key(|(_, (_, _, idle))| *idle)
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eviction_picks_the_longest_idle_other_worker_past_the_threshold() {
        let old = EVICTABLE_IDLE_MS;
        let candidates = [
            ("alice", "todo", old * 5),
            ("alice", "docs", old - 1),
            ("bob", "slack", old * 2),
            ("bob", "todo", old * 3),
        ];
        let idle = |_: &str, _: &str| false;
        // The target itself is never chosen, even when it is the idlest.
        assert_eq!(
            eviction_victim(&candidates, ("alice", "todo"), idle),
            Some(3)
        );
        assert_eq!(eviction_victim(&candidates, ("bob", "todo"), idle), Some(0));
        // Nothing idle long enough: no eviction.
        assert_eq!(eviction_victim(&candidates[1..2], ("x", "y"), idle), None);
        assert_eq!(eviction_victim(&[], ("x", "y"), idle), None);
        // A busy (undelivered events) idlest worker is skipped for the next one.
        let busy = |owner: &str, installation: &str| (owner, installation) == ("alice", "todo");
        assert_eq!(eviction_victim(&candidates, ("bob", "todo"), busy), Some(2));
    }
}

/// A worker that should be running but has none starts on demand; a user
/// stop (not desired) or a failed worker (restart backoff, quarantine) never.
fn starts_on_demand(
    status: Option<&crate::durable_state::app_worker_lifecycle::WorkerStatus>,
) -> bool {
    status.is_some_and(|status| {
        status.desired_running
            && status.phase != crate::durable_state::app_worker_lifecycle::WorkerPhase::Failed
    })
}

#[cfg(test)]
mod on_demand_tests {
    use super::starts_on_demand;
    use crate::durable_state::app_worker_lifecycle::{WorkerPhase, WorkerStatus};

    fn status(phase: WorkerPhase, desired_running: bool) -> WorkerStatus {
        WorkerStatus {
            generation: 1,
            attempt: "attempt".into(),
            phase,
            desired_running,
            dormant: false,
            failure: None,
            updated_ms: 1,
            failures: 0,
        }
    }

    #[test]
    fn a_worker_that_should_run_starts_on_demand_but_a_user_stop_or_failure_never_does() {
        // Stopped by a kernel restart (or never started by recovery): start it.
        assert!(starts_on_demand(Some(&status(WorkerPhase::Stopped, true))));
        // A start that never completed, after the caller waited for it.
        assert!(starts_on_demand(Some(&status(WorkerPhase::Starting, true))));
        assert!(!starts_on_demand(Some(&status(
            WorkerPhase::Stopped,
            false
        ))));
        assert!(!starts_on_demand(Some(&status(WorkerPhase::Failed, true))));
        assert!(!starts_on_demand(None));
    }
}
