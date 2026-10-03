//! Kernel-owned App wakes: deliver due wakes to live workers and start stopped
//! workers on demand. The App needs no resident process to wait for a due time.
use super::KernelRuntimeState;
use crate::durable_state::app_wakes::{AppWakeOperation, AppWakeOutcome};
use crate::durable_state::app_worker_lifecycle::StartGate;
use crate::runtime::app_lifecycle::LifecycleError;
use crate::runtime::app_worker::DeliveryError;
use chariox_app_runtime::managed_state::DueWake;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

mod cadence;
use cadence::WakeCadence;

pub(super) const PAGE: usize = 8;
/// A wake delivered this long after its due time is reported as overdue.
const OVERDUE_AFTER_MS: u64 = 60_000;
pub(super) const DELIVERY_TIMEOUT: Duration = Duration::from_secs(30);
/// Waiting wakes step aside for this long so they cannot starve later ones.
pub(super) const START_WAIT_MS: u64 = 2_000;
/// A user-stopped App keeps its wakes until the user starts it again.
const STOPPED_WAIT_MS: u64 = 60_000;
/// A worker with no use for this long stops (a tool call, an inbound event or
/// a wake armed during either; see `AppWorkerLease::deliver_wake`). Its tools
/// stay discoverable and its next use or due wake starts it again.
const IDLE_AFTER_MS: u64 = 10 * 60_000;

/// Outcome of one on-demand start request for an installation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Start {
    /// Starting, already starting, or admission busy: wait without an attempt.
    Pending,
    /// Every live worker slot is taken: wait without an attempt while the
    /// pass makes room, as a tool call does, by stopping an idle worker.
    AtLiveLimit,
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

pub(super) type Installations = BTreeSet<(String, String)>;
type DeliveryPlan<T> = (Vec<T>, Vec<(T, Settle)>, Installations);

/// Pure pass planning: deliver to live workers, start each stopped
/// installation at most once, and return deduplicated eviction requests.
pub(super) fn plan<T>(
    due: Vec<T>,
    now_ms: u64,
    key: impl Fn(&T) -> (String, String),
    is_live: impl Fn(&T) -> bool,
    mut start: impl FnMut(&T) -> Start,
) -> DeliveryPlan<T> {
    let mut deliver = Vec::new();
    let mut records = Vec::new();
    let mut at_live_limit = BTreeSet::new();
    let mut started: BTreeMap<(String, String), Start> = BTreeMap::new();
    for item in due {
        if is_live(&item) {
            deliver.push(item);
            continue;
        }
        let outcome = *started.entry(key(&item)).or_insert_with(|| start(&item));
        if outcome == Start::AtLiveLimit {
            at_live_limit.insert(key(&item));
        }
        let settle = match outcome {
            Start::Pending | Start::AtLiveLimit => {
                Settle::Postponed(now_ms.saturating_add(START_WAIT_MS))
            }
            Start::UserStopped => Settle::Postponed(now_ms.saturating_add(STOPPED_WAIT_MS)),
            Start::Refused => Settle::Failed,
        };
        records.push((item, settle));
    }
    (deliver, records, at_live_limit)
}

/// Run both delivery pages before any idle-worker shutdown. Even a slow
/// shutdown cannot block the page's live work, including the inbox page.
async fn deliver_before_eviction<F: std::future::Future<Output = ()>>(
    wakes: impl std::future::Future<Output = (bool, Installations)>,
    inbox: impl std::future::Future<Output = (bool, Installations)>,
    mut evict: impl FnMut(String, String) -> F,
) -> bool {
    let (wakes_full, mut at_live_limit) = wakes.await;
    let (inbox_full, inbox_evictions) = inbox.await;
    at_live_limit.extend(inbox_evictions);
    for (owner, installation) in at_live_limit {
        evict(owner, installation).await;
    }
    wakes_full || inbox_full
}

/// A delivered item completes. A failed one waits without spending an
/// attempt while an update drained its worker (the new generation gets it);
/// any other failure, including the App's own handler error, counts.
/// A pass that filled its page and delivered something asks for another at
/// once: the rest of a backlog (an update's held work, events that arrived
/// while the kernel was down) would otherwise wait a periodic tick per page.
/// A page that only waited asks for nothing, so held work cannot spin.
pub(super) fn page_wants_rerun(page: usize, delivered: usize) -> bool {
    page == PAGE && delivered > 0
}

pub(super) fn after_delivery(delivered: bool, update_pending: bool, now_ms: u64) -> Settle {
    if delivered {
        Settle::Delivered
    } else if update_pending {
        Settle::Postponed(now_ms.saturating_add(START_WAIT_MS))
    } else {
        Settle::Failed
    }
}

fn wake_record(wake: DueWake, settle: Settle, now_ms: u64, reason: &str) -> AppWakeOperation {
    match settle {
        Settle::Delivered => AppWakeOperation::Delivered(wake),
        Settle::Postponed(until_ms) => AppWakeOperation::Postponed { wake, until_ms },
        Settle::Failed => AppWakeOperation::Failed {
            wake,
            now_ms,
            reason: reason.to_owned(),
        },
    }
}

impl KernelRuntimeState {
    /// One kernel-owned delivery lane, independent of transport reconciliation.
    /// Each bounded page settles before the next deadline is read. A due
    /// backlog drains immediately, without the maintenance pass throttle.
    pub(crate) async fn run_app_wake_scheduler(&self) {
        let Some(_scheduler) = self.app_control().reserve_wake_scheduler() else {
            return;
        };
        let mut cadence = WakeCadence::default();
        loop {
            self.owned.durable_state_store.wait_for_app_wake().await;
            if !self
                .app_wake_pass(crate::session::unix_epoch_ms(), &mut cadence)
                .await
            {
                // A transient read/settlement error must not spin on the same
                // durable item or race its outstanding writer work.
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            tokio::task::yield_now().await;
        }
    }

    /// Coordinator wiring only: one bounded pass per reservation.
    pub(crate) fn schedule_app_maintenance_pump(&self) {
        self.begin_app_wake_pump(|pump, now_ms| pump.try_begin(now_ms));
    }

    /// Newly accepted work: deliver it now rather than on the next periodic
    /// tick, which an idle kernel runs only every five seconds.
    pub(crate) fn request_app_wake_pump(&self) {
        self.begin_app_wake_pump(|pump, now_ms| pump.try_begin_requested(now_ms));
    }

    fn begin_app_wake_pump(
        &self,
        begin: impl FnOnce(
            &crate::runtime::app_wake_pump::AppWakePump,
            u64,
        ) -> Option<crate::runtime::app_wake_pump::WakePass>,
    ) {
        let now_ms = crate::session::unix_epoch_ms();
        if self
            .owned
            .durable_state_store
            .require_writer_healthy()
            .is_err()
        {
            return;
        }
        let Some(mut pass) = begin(self.app_control().wake_pump(), now_ms) else {
            return;
        };
        let runtime = self.clone();
        tokio::spawn(async move {
            let mut now_ms = now_ms;
            loop {
                let wants_rerun = deliver_before_eviction(
                    async { (false, BTreeSet::new()) },
                    runtime.app_inbox_pass(now_ms),
                    |owner, installation| {
                        let runtime = &runtime;
                        async move {
                            runtime.evict_idle_app(&owner, &installation).await;
                        }
                    },
                )
                .await;
                if wants_rerun {
                    pass.again();
                }
                runtime.stop_idle_apps(now_ms).await;
                if runtime.app_control().wake_pump().prune_due(now_ms) {
                    runtime.prune_dormant_apps().await;
                }
                // Work accepted while this pass ran gets a pass of its own.
                let started_ms = now_ms;
                match pass.finish(crate::session::unix_epoch_ms()) {
                    Some(next) => pass = next,
                    None => break,
                }
                let gap = crate::runtime::app_wake_pump::REQUESTED_GAP_MS
                    .saturating_sub(crate::session::unix_epoch_ms().saturating_sub(started_ms));
                if gap > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(gap)).await;
                }
                // Read after the handoff: the rerun's clock is never earlier
                // than an occurrence accepted before it, so that one is due.
                now_ms = crate::session::unix_epoch_ms();
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
                lifecycle.request_idle_stop_blocking(&owner, catalog, || {
                    // Undelivered events need the live lease: not idle yet.
                    current
                        .active_app_lease(&owner, &installation)
                        .is_some_and(|lease| {
                            lease.idle_ms(crate::session::unix_epoch_ms()) >= IDLE_AFTER_MS
                        })
                        && !store.has_deliverable_app_events(&owner, &installation)
                        && !store.has_due_app_wakes(
                            &owner,
                            &installation,
                            crate::session::unix_epoch_ms(),
                        )
                })
            })
            .await;
        }
    }

    async fn app_wake_pass(&self, now_ms: u64, cadence: &mut WakeCadence) -> bool {
        let store = self.owned.durable_state_store.clone();
        let due = tokio::task::spawn_blocking(move || {
            store.app_wakes(AppWakeOperation::Due {
                now_ms,
                limit: PAGE,
            })
        })
        .await;
        let Ok(Ok(AppWakeOutcome::Due(due))) = due else {
            return false;
        };
        let Ok((deliver, planned, at_live_limit)) = self
            .plan_app_delivery(due, now_ms, |wake| {
                (wake.owner_id.clone(), wake.installation_id.clone())
            })
            .await
        else {
            return false;
        };
        let (deliver, mut records) = cadence.split(
            deliver,
            crate::session::unix_epoch_ms(),
            std::time::Instant::now(),
        );
        records.extend(
            planned
                .into_iter()
                .map(|(wake, settle)| wake_record(wake, settle, now_ms, "the App could not start")),
        );
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
            cadence.record_attempt(&wake, std::time::Instant::now());
            let delivered = lease
                .deliver_wake(&wake.wake, overdue, wake.counts_as_use, DELIVERY_TIMEOUT)
                .await;
            // An idle stop began first: nothing reached the App.
            if matches!(delivered, Err(DeliveryError::NotAdmitted)) {
                records.push(AppWakeOperation::Postponed {
                    wake,
                    until_ms: now_ms.saturating_add(START_WAIT_MS),
                });
                continue;
            }
            let update_pending = delivered.is_err()
                && self
                    .app_update_pending(&wake.owner_id, &wake.installation_id)
                    .await;
            let reason = delivered
                .as_ref()
                .err()
                .map(ToString::to_string)
                .unwrap_or_default();
            records.push(wake_record(
                wake,
                after_delivery(delivered.is_ok(), update_pending, now_ms),
                now_ms,
                &reason,
            ));
        }
        let store = self.owned.durable_state_store.clone();
        let recorded = tokio::task::spawn_blocking(move || {
            let mut recorded = true;
            for record in records {
                recorded &= store.app_wakes(record).is_ok();
            }
            recorded
        })
        .await
        .unwrap_or(false);
        for (owner, installation) in at_live_limit {
            self.evict_idle_app(&owner, &installation).await;
        }
        recorded
    }

    /// Splits due work into items for live workers and items that wait,
    /// starting each stopped installation on demand at most once.
    pub(super) async fn plan_app_delivery<T: Send + 'static>(
        &self,
        due: Vec<T>,
        now_ms: u64,
        key: fn(&T) -> (String, String),
    ) -> Result<DeliveryPlan<T>, tokio::task::JoinError> {
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
                        Ok(_) | Err(LifecycleError::Busy) => Start::Pending,
                        Err(LifecycleError::LiveLimit) => Start::AtLiveLimit,
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

    #[tokio::test]
    async fn app_wake_planning_panic_is_reported_for_backoff() {
        use crate::{app::DaemonApp, runtime::router::CommandRouter, DaemonConfig};
        let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
        let router = CommandRouter::with_interactive_capacity(
            std::sync::Arc::new(tokio::sync::Mutex::new(app)),
            4,
        );
        let result = router
            .runtime_state()
            .plan_app_delivery(vec![()], 1, |_| panic!("injected planning panic"))
            .await;
        assert!(result.unwrap_err().is_panic());
    }

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
            counts_as_use: false,
        }
    }

    fn key(wake: &DueWake) -> (String, String) {
        (wake.owner_id.clone(), wake.installation_id.clone())
    }

    #[test]
    fn stopped_installations_start_once_per_pass_and_waiting_wakes_step_aside() {
        let starts = RefCell::new(Vec::new());
        let (deliver, records, _) = plan(
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
    fn wakes_at_the_live_limit_wait_without_spending_attempts() {
        let starts = RefCell::new(0);
        let (deliver, records, _) = plan(
            vec![due("dormant", "a"), due("dormant", "b")],
            100,
            key,
            |_| false,
            |_| {
                *starts.borrow_mut() += 1;
                Start::AtLiveLimit
            },
        );
        assert_eq!(starts.into_inner(), 1);
        assert!(deliver.is_empty());
        assert!(records
            .iter()
            .all(|(_, settle)| *settle == Settle::Postponed(100 + START_WAIT_MS)));
    }

    #[tokio::test]
    async fn both_delivery_pages_finish_before_a_blocked_eviction() {
        use std::sync::{Arc, Mutex};
        let served = Arc::new(Mutex::new(Vec::new()));
        let wake_served = served.clone();
        let inbox_served = served.clone();
        let (shutdown_started, mut shutdown_observed) = tokio::sync::mpsc::channel(1);
        let (release_shutdown, shutdown_gate) = tokio::sync::oneshot::channel();
        let mut shutdown_gate = Some(shutdown_gate);
        let pump = tokio::spawn(async move {
            deliver_before_eviction(
                async {
                    wake_served.lock().unwrap().push("wake");
                    (false, BTreeSet::from([("alice".into(), "dormant".into())]))
                },
                async {
                    inbox_served.lock().unwrap().push("inbox");
                    (false, BTreeSet::from([("alice".into(), "dormant".into())]))
                },
                move |_, _| {
                    let started = shutdown_started.clone();
                    let gate = shutdown_gate.take().expect("duplicate eviction");
                    async move {
                        started.send(()).await.unwrap();
                        gate.await.unwrap();
                    }
                },
            )
            .await;
        });
        tokio::time::timeout(Duration::from_secs(2), shutdown_observed.recv())
            .await
            .unwrap()
            .unwrap();
        // Shutdown is still blocked. Both live pages must already have run.
        assert_eq!(*served.lock().unwrap(), ["wake", "inbox"]);
        assert!(!pump.is_finished());
        release_shutdown.send(()).unwrap();
        pump.await.unwrap();
    }

    #[tokio::test]
    async fn delivery_before_eviction_preserves_either_pages_backlog_request() {
        for (wakes, inbox) in [(false, false), (true, false), (false, true)] {
            let rerun = deliver_before_eviction(
                async { (wakes, BTreeSet::new()) },
                async { (inbox, BTreeSet::new()) },
                |_, _| async { panic!("no eviction requested") },
            )
            .await;
            assert_eq!(rerun, wakes || inbox);
        }
    }

    #[test]
    fn eviction_requests_are_deduplicated_and_live_work_stays_deliverable() {
        let (deliver, records, at_live_limit) = plan(
            vec![
                due("dormant", "a"),
                due("live", "b"),
                due("dormant", "c"),
                due("busy", "d"),
            ],
            100,
            key,
            |wake| wake.installation_id == "live",
            |wake| {
                if wake.installation_id == "dormant" {
                    Start::AtLiveLimit
                } else {
                    Start::Pending
                }
            },
        );
        assert_eq!(
            at_live_limit,
            BTreeSet::from([("alice".into(), "dormant".into())])
        );
        assert_eq!(deliver.len(), 1);
        assert_eq!(deliver[0].wake.id, "b");
        assert_eq!(records.len(), 3);
        assert!(records
            .iter()
            .all(|(_, settle)| *settle == Settle::Postponed(100 + START_WAIT_MS)));
    }

    #[test]
    fn after_an_idle_stop_every_due_wake_starts_its_app_on_time() {
        // The App's own wake and a tool-armed one plan alike: each stopped
        // worker starts on demand and its wake is delivered on the next pass,
        // spending no attempt. Only use differs once delivered.
        let own = due("own", "a");
        let mut armed = due("armed", "b");
        armed.counts_as_use = true;
        let starts = RefCell::new(Vec::new());
        let (deliver, records, _) = plan(
            vec![own, armed],
            100,
            key,
            |_| false,
            |wake| {
                starts.borrow_mut().push(wake.installation_id.clone());
                Start::Pending
            },
        );
        assert_eq!(starts.into_inner(), ["own", "armed"]);
        assert!(deliver.is_empty());
        assert!(records
            .iter()
            .all(|(_, settle)| *settle == Settle::Postponed(100 + START_WAIT_MS)));
    }

    #[test]
    fn user_stopped_apps_keep_their_wakes_without_spending_attempts() {
        let (_, records, _) = plan(
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
        let (deliver, records, _) = plan(
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
            wake_record(
                due("a", "w"),
                Settle::Failed,
                100,
                "the App could not start"
            ),
            AppWakeOperation::Failed { now_ms: 100, .. }
        ));
    }

    #[test]
    fn only_a_full_page_that_delivered_asks_for_another_pass() {
        assert!(page_wants_rerun(PAGE, 1));
        assert!(page_wants_rerun(PAGE, PAGE));
        // Everything waited (a worker starting, an update held it): no spin.
        assert!(!page_wants_rerun(PAGE, 0));
        // A short page was the whole backlog.
        assert!(!page_wants_rerun(PAGE - 1, PAGE - 1));
        assert!(!page_wants_rerun(0, 0));
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
