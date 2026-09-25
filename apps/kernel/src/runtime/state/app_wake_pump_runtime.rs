//! Kernel-owned App wakes: deliver due wakes to live workers and start stopped
//! workers on demand. The App needs no resident process to wait for a due time.
use super::KernelRuntimeState;
use crate::durable_state::app_wakes::{AppWakeOperation, AppWakeOutcome};
use crate::durable_state::app_worker_lifecycle::StartGate;
use crate::runtime::app_lifecycle::LifecycleError;
use chariox_app_runtime::managed_state::DueWake;
use std::{collections::BTreeMap, time::Duration};

const PAGE: usize = 8;
/// A wake delivered this long after its due time is reported as overdue.
const OVERDUE_AFTER_MS: u64 = 60_000;
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(30);
/// Waiting wakes step aside for this long so they cannot starve later ones.
const START_WAIT_MS: u64 = 2_000;
/// A user-stopped App keeps its wakes until the user starts it again.
const STOPPED_WAIT_MS: u64 = 60_000;
/// A worker with no tool call or wake for this long stops; its tools stay
/// discoverable and its next use starts it again.
const IDLE_AFTER_MS: u64 = 10 * 60_000;

/// Outcome of one on-demand start request for an installation.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Start {
    /// Starting, already starting, or admission busy: wait without an attempt.
    Pending,
    /// The user stopped the App: keep wakes without spending attempts.
    UserStopped,
    /// Failed generation, revocation or inactive installation: bounded attempts.
    Refused,
}

/// Pure pass planning: deliver to live workers, start each stopped
/// installation at most once, and never let waiting wakes block the page.
fn plan(
    due: Vec<DueWake>,
    now_ms: u64,
    is_live: impl Fn(&DueWake) -> bool,
    mut start: impl FnMut(&DueWake) -> Start,
) -> (Vec<DueWake>, Vec<AppWakeOperation>) {
    let mut deliver = Vec::new();
    let mut records = Vec::new();
    let mut started: BTreeMap<(String, String), Start> = BTreeMap::new();
    for wake in due {
        if is_live(&wake) {
            deliver.push(wake);
            continue;
        }
        let key = (wake.owner_id.clone(), wake.installation_id.clone());
        let outcome = *started.entry(key).or_insert_with(|| start(&wake));
        records.push(match outcome {
            Start::Pending => AppWakeOperation::Postponed {
                wake,
                until_ms: now_ms.saturating_add(START_WAIT_MS),
            },
            Start::UserStopped => AppWakeOperation::Postponed {
                wake,
                until_ms: now_ms.saturating_add(STOPPED_WAIT_MS),
            },
            Start::Refused => AppWakeOperation::Failed { wake, now_ms },
        });
    }
    (deliver, records)
}

/// A delivered wake completes. A failed one waits without spending an
/// attempt while an update drained its worker (the new generation gets it);
/// any other failure, including the App's own handler error, counts.
fn after_delivery(
    wake: DueWake,
    delivered: bool,
    update_pending: bool,
    now_ms: u64,
) -> AppWakeOperation {
    if delivered {
        AppWakeOperation::Delivered(wake)
    } else if update_pending {
        AppWakeOperation::Postponed {
            wake,
            until_ms: now_ms.saturating_add(START_WAIT_MS),
        }
    } else {
        AppWakeOperation::Failed { wake, now_ms }
    }
}

impl KernelRuntimeState {
    /// Coordinator wiring only: one bounded pass per reservation.
    pub(crate) fn schedule_app_wake_pump(&self) {
        let now_ms = crate::session::unix_epoch_ms();
        if self
            .owned
            .durable_state_store
            .require_writer_healthy()
            .is_err()
        {
            return;
        }
        let Some(pass) = self.app_control().wake_pump().try_begin(now_ms) else {
            return;
        };
        let runtime = self.clone();
        tokio::spawn(async move {
            let _pass = pass;
            runtime.app_wake_pass(now_ms).await;
            runtime.stop_idle_apps(now_ms).await;
            if runtime.app_control().wake_pump().prune_due(now_ms) {
                runtime.prune_dormant_apps().await;
            }
        });
    }

    /// Forget dormant catalogs whose installation may no longer start (revoked,
    /// paused, uninstalled or failed), so they leave the catalog and live cap.
    async fn prune_dormant_apps(&self) {
        let control = self.app_control().clone();
        let store = self.owned.durable_state_store.clone();
        let _ = tokio::task::spawn_blocking(move || {
            for (owner, installation) in control.dormant_app_keys() {
                if matches!(
                    store.app_worker_start_gate(&owner, &installation),
                    Ok(StartGate::Refused)
                ) {
                    control.forget_app_dormant(&owner, &installation);
                }
            }
        })
        .await;
    }

    async fn stop_idle_apps(&self, now_ms: u64) {
        let control = self.app_control();
        let idle: Vec<_> = control
            .active_app_leases(None, 16)
            .into_iter()
            .filter(|lease| lease.idle_ms(now_ms) >= IDLE_AFTER_MS)
            .collect();
        for lease in idle {
            let lifecycle = control.lifecycle().clone();
            let owner = lease.owner().to_owned();
            let catalog = lease.catalog().clone();
            drop(lease);
            let current = control.clone();
            let store = self.owned.durable_state_store.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let installation = catalog.installation_id().to_owned();
                lifecycle.idle_stop_blocking(&owner, catalog, || {
                    // Undelivered events need the live lease: not idle yet.
                    current
                        .active_app_lease(&owner, &installation)
                        .is_some_and(|lease| {
                            lease.idle_ms(crate::session::unix_epoch_ms()) >= IDLE_AFTER_MS
                        })
                        && !store.has_deliverable_app_events(&owner, &installation)
                })
            })
            .await;
        }
    }

    async fn app_wake_pass(&self, now_ms: u64) {
        let store = self.owned.durable_state_store.clone();
        let due = tokio::task::spawn_blocking(move || {
            store.app_wakes(AppWakeOperation::Due {
                now_ms,
                limit: PAGE,
            })
        })
        .await;
        let Ok(Ok(AppWakeOutcome::Due(due))) = due else {
            return;
        };
        let control = self.app_control().clone();
        let handle = tokio::runtime::Handle::current();
        let planning = control.clone();
        let Ok((deliver, mut records)) = tokio::task::spawn_blocking(move || {
            let lifecycle = planning.lifecycle().clone();
            plan(
                due,
                now_ms,
                |wake| {
                    planning
                        .active_app_lease(&wake.owner_id, &wake.installation_id)
                        .is_some()
                },
                |wake| match lifecycle.start_on_demand_blocking(
                    &wake.owner_id,
                    &wake.installation_id,
                    handle.clone(),
                ) {
                    Ok(_) | Err(LifecycleError::Busy | LifecycleError::LiveLimit) => Start::Pending,
                    Err(LifecycleError::Stopped) => Start::UserStopped,
                    Err(_) => {
                        planning.forget_app_dormant(&wake.owner_id, &wake.installation_id);
                        Start::Refused
                    }
                },
            )
        })
        .await
        else {
            return;
        };
        for wake in deliver {
            let overdue = now_ms.saturating_sub(wake.wake.due_at_ms) > OVERDUE_AFTER_MS;
            // The worker left since planning (drained for an update or
            // stopped): wait for the next one without spending an attempt.
            let Some(lease) = control.active_app_lease(&wake.owner_id, &wake.installation_id)
            else {
                records.push(AppWakeOperation::Postponed {
                    wake,
                    until_ms: now_ms.saturating_add(START_WAIT_MS),
                });
                continue;
            };
            let delivered = lease
                .deliver_wake(&wake.wake, overdue, DELIVERY_TIMEOUT)
                .await
                .is_ok();
            let update_pending = !delivered
                && self
                    .app_update_pending(&wake.owner_id, &wake.installation_id)
                    .await;
            records.push(after_delivery(wake, delivered, update_pending, now_ms));
        }
        let store = self.owned.durable_state_store.clone();
        let _ = tokio::task::spawn_blocking(move || {
            for record in records {
                let _ = store.app_wakes(record);
            }
        })
        .await;
    }

    /// Whether an update of the installation is staged (not yet committed or
    /// aborted).
    async fn app_update_pending(&self, owner: &str, installation: &str) -> bool {
        let store = self.owned.durable_state_store.clone();
        let (owner, installation) = (owner.to_owned(), installation.to_owned());
        tokio::task::spawn_blocking(move || {
            store
                .get_app_installation(&owner, &installation)
                .is_ok_and(|installation| installation.pending_generation.is_some())
        })
        .await
        .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chariox_app_runtime::managed_state::Wake;
    use std::cell::RefCell;

    fn due(installation: &str, id: &str) -> DueWake {
        DueWake {
            owner_id: "alice".into(),
            installation_id: installation.into(),
            wake: Wake {
                id: id.into(),
                due_at_ms: 1,
                revision: String::new(),
            },
            attempts: 0,
        }
    }

    #[test]
    fn stopped_installations_start_once_per_pass_and_waiting_wakes_step_aside() {
        let starts = RefCell::new(Vec::new());
        let (deliver, records) = plan(
            vec![due("stopped", "a"), due("live", "b"), due("stopped", "c")],
            100,
            |wake| wake.installation_id == "live",
            |wake| {
                starts.borrow_mut().push(wake.installation_id.clone());
                Start::Pending
            },
        );
        assert_eq!(starts.into_inner(), ["stopped"]);
        assert_eq!(deliver.len(), 1);
        assert_eq!(deliver[0].wake.id, "b");
        assert_eq!(records.len(), 2);
        assert!(records.iter().all(|record| matches!(
            record,
            AppWakeOperation::Postponed { until_ms, .. } if *until_ms == 100 + START_WAIT_MS
        )));
    }

    #[test]
    fn user_stopped_apps_keep_their_wakes_without_spending_attempts() {
        let (_, records) = plan(
            vec![due("stopped", "a")],
            100,
            |_| false,
            |_| Start::UserStopped,
        );
        assert!(matches!(
            records[..],
            [AppWakeOperation::Postponed { until_ms, .. }] if until_ms == 100 + STOPPED_WAIT_MS
        ));
    }

    #[test]
    fn refused_starts_consume_bounded_attempts_instead_of_restarting() {
        let (deliver, records) = plan(
            vec![due("user-stopped", "a"), due("user-stopped", "b")],
            100,
            |_| false,
            |_| Start::Refused,
        );
        assert!(deliver.is_empty());
        assert_eq!(records.len(), 2);
        assert!(records
            .iter()
            .all(|record| matches!(record, AppWakeOperation::Failed { now_ms: 100, .. })));
    }

    #[test]
    fn a_wake_interrupted_by_an_update_waits_without_spending_an_attempt() {
        assert!(matches!(
            after_delivery(due("a", "w"), true, false, 10),
            AppWakeOperation::Delivered(_)
        ));
        assert!(matches!(
            after_delivery(due("a", "w"), false, true, 10),
            AppWakeOperation::Postponed { until_ms, .. } if until_ms == 10 + START_WAIT_MS
        ));
        assert!(matches!(
            after_delivery(due("a", "w"), false, false, 10),
            AppWakeOperation::Failed { now_ms: 10, .. }
        ));
    }
}
