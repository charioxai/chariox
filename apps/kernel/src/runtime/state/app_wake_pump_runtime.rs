//! Kernel-owned App wakes: deliver due wakes to live workers and start stopped
//! workers on demand. The App needs no resident process to wait for a due time.
use super::KernelRuntimeState;
use crate::durable_state::app_wakes::{AppWakeOperation, AppWakeOutcome};
use crate::durable_state::app_worker_lifecycle::StartGate;
use crate::runtime::app_lifecycle::LifecycleError;
use chariox_app_runtime::managed_state::DueWake;
use std::{collections::BTreeMap, time::Duration};

pub(super) const PAGE: usize = 8;
/// A wake delivered this long after its due time is reported as overdue.
const OVERDUE_AFTER_MS: u64 = 60_000;
pub(super) const DELIVERY_TIMEOUT: Duration = Duration::from_secs(30);
/// Waiting wakes step aside for this long so they cannot starve later ones.
pub(super) const START_WAIT_MS: u64 = 2_000;
/// A user-stopped App keeps its wakes until the user starts it again.
const STOPPED_WAIT_MS: u64 = 60_000;
/// A worker with no tool call or wake for this long stops; its tools stay
/// discoverable and its next use starts it again.
const IDLE_AFTER_MS: u64 = 10 * 60_000;

/// Outcome of one on-demand start request for an installation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Start {
    /// Starting, already starting, or admission busy: wait without an attempt.
    Pending,
    /// The user stopped the App: keep work without spending attempts.
    UserStopped,
    /// Failed generation, revocation or inactive installation: bounded attempts.
    Refused,
}

/// What happens to one due item that was not delivered in this pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Settle {
    Delivered,
    /// Wait without spending an attempt.
    Postponed(u64),
    /// Spend one bounded attempt.
    Failed,
}

/// Pure pass planning: deliver to live workers, start each stopped
/// installation at most once, and never let waiting items block the page.
pub(super) fn plan<T>(
    due: Vec<T>,
    now_ms: u64,
    key: impl Fn(&T) -> (String, String),
    is_live: impl Fn(&T) -> bool,
    mut start: impl FnMut(&T) -> Start,
) -> (Vec<T>, Vec<(T, Settle)>) {
    let mut deliver = Vec::new();
    let mut records = Vec::new();
    let mut started: BTreeMap<(String, String), Start> = BTreeMap::new();
    for item in due {
        if is_live(&item) {
            deliver.push(item);
            continue;
        }
        let outcome = *started.entry(key(&item)).or_insert_with(|| start(&item));
        let settle = match outcome {
            Start::Pending => Settle::Postponed(now_ms.saturating_add(START_WAIT_MS)),
            Start::UserStopped => Settle::Postponed(now_ms.saturating_add(STOPPED_WAIT_MS)),
            Start::Refused => Settle::Failed,
        };
        records.push((item, settle));
    }
    (deliver, records)
}

/// A delivered item completes. A failed one waits without spending an
/// attempt while an update drained its worker (the new generation gets it);
/// any other failure, including the App's own handler error, counts.
pub(super) fn after_delivery(delivered: bool, update_pending: bool, now_ms: u64) -> Settle {
    if delivered {
        Settle::Delivered
    } else if update_pending {
        Settle::Postponed(now_ms.saturating_add(START_WAIT_MS))
    } else {
        Settle::Failed
    }
}

fn wake_record(wake: DueWake, settle: Settle, now_ms: u64) -> AppWakeOperation {
    match settle {
        Settle::Delivered => AppWakeOperation::Delivered(wake),
        Settle::Postponed(until_ms) => AppWakeOperation::Postponed { wake, until_ms },
        Settle::Failed => AppWakeOperation::Failed { wake, now_ms },
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
            runtime.app_inbox_pass(now_ms).await;
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
        let (deliver, planned) = self
            .plan_app_delivery(due, now_ms, |wake| {
                (wake.owner_id.clone(), wake.installation_id.clone())
            })
            .await;
        let mut records: Vec<_> = planned
            .into_iter()
            .map(|(wake, settle)| wake_record(wake, settle, now_ms))
            .collect();
        let control = self.app_control().clone();
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
            records.push(wake_record(
                wake,
                after_delivery(delivered, update_pending, now_ms),
                now_ms,
            ));
        }
        let store = self.owned.durable_state_store.clone();
        let _ = tokio::task::spawn_blocking(move || {
            for record in records {
                let _ = store.app_wakes(record);
            }
        })
        .await;
    }

    /// Splits due work into items for live workers and items that wait,
    /// starting each stopped installation on demand at most once.
    pub(super) async fn plan_app_delivery<T: Send + 'static>(
        &self,
        due: Vec<T>,
        now_ms: u64,
        key: fn(&T) -> (String, String),
    ) -> (Vec<T>, Vec<(T, Settle)>) {
        let planning = self.app_control().clone();
        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let lifecycle = planning.lifecycle().clone();
            plan(
                due,
                now_ms,
                key,
                |item| {
                    let (owner, installation) = key(item);
                    planning.active_app_lease(&owner, &installation).is_some()
                },
                |item| {
                    let (owner, installation) = key(item);
                    match lifecycle.start_on_demand_blocking(&owner, &installation, handle.clone())
                    {
                        Ok(_) | Err(LifecycleError::Busy | LifecycleError::LiveLimit) => {
                            Start::Pending
                        }
                        Err(LifecycleError::Stopped) => Start::UserStopped,
                        Err(_) => {
                            planning.forget_app_dormant(&owner, &installation);
                            Start::Refused
                        }
                    }
                },
            )
        })
        .await
        .unwrap_or_default()
    }

    /// Whether an update of the installation is staged (not yet committed or
    /// aborted).
    pub(super) async fn app_update_pending(&self, owner: &str, installation: &str) -> bool {
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

    fn key(wake: &DueWake) -> (String, String) {
        (wake.owner_id.clone(), wake.installation_id.clone())
    }

    #[test]
    fn stopped_installations_start_once_per_pass_and_waiting_wakes_step_aside() {
        let starts = RefCell::new(Vec::new());
        let (deliver, records) = plan(
            vec![due("stopped", "a"), due("live", "b"), due("stopped", "c")],
            100,
            key,
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
        assert!(records
            .iter()
            .all(|(_, settle)| *settle == Settle::Postponed(100 + START_WAIT_MS)));
    }

    #[test]
    fn user_stopped_apps_keep_their_wakes_without_spending_attempts() {
        let (_, records) = plan(
            vec![due("stopped", "a")],
            100,
            key,
            |_| false,
            |_| Start::UserStopped,
        );
        assert_eq!(records[0].1, Settle::Postponed(100 + STOPPED_WAIT_MS));
    }

    #[test]
    fn refused_starts_consume_bounded_attempts_instead_of_restarting() {
        let (deliver, records) = plan(
            vec![due("user-stopped", "a"), due("user-stopped", "b")],
            100,
            key,
            |_| false,
            |_| Start::Refused,
        );
        assert!(deliver.is_empty());
        assert_eq!(records.len(), 2);
        assert!(records.iter().all(|(_, settle)| *settle == Settle::Failed));
        assert!(matches!(
            wake_record(due("a", "w"), Settle::Failed, 100),
            AppWakeOperation::Failed { now_ms: 100, .. }
        ));
    }

    #[test]
    fn work_interrupted_by_an_update_waits_without_spending_an_attempt() {
        assert_eq!(after_delivery(true, false, 10), Settle::Delivered);
        assert_eq!(
            after_delivery(false, true, 10),
            Settle::Postponed(10 + START_WAIT_MS)
        );
        assert_eq!(after_delivery(false, false, 10), Settle::Failed);
    }
}
