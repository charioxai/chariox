//! One blocking lifecycle owner constructs its peer from the actual process FD.
//! AppControl publishes weak callable handles only after that peer's exact SDK
//! report and a durable active-generation proof. Neither is permanent authority.

mod call;
mod lifecycle;
pub(crate) use lifecycle::AppWorkerDrain;
mod startup;
#[cfg(test)]
mod tests;
pub(crate) use call::{AppCallSlot, AppToolError, AppToolReply, AppToolResponse};
pub(crate) use startup::{FirstInstallHealth, RegisteredAppWorker};

use chariox_app_runtime::{
    app_outbox::EventCatalog,
    worker_peer::{Broker, PeerTask, WorkerPeer},
    worker_process::{WorkerCancellation, WorkerEnding, WorkerProcess},
    worker_readiness::RegisteredHandlers,
};
use std::sync::{Arc, Mutex, Weak};
use tokio::{runtime::Handle, sync::watch};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum AppWorkerError {
    #[error("app_worker_unavailable")]
    Unavailable,
    #[error("app_worker_activation_mismatch")]
    Identity,
    #[error("app_worker_busy")]
    Busy,
    #[error("app_worker_cancelled")]
    Cancelled,
    #[error("app_worker_deadline")]
    Deadline,
    #[error("app_worker_request_invalid")]
    Invalid,
    /// The worker was stopped for using more memory than its limit.
    #[error("app_worker_memory_limit")]
    MemoryLimit,
    /// A local update is replacing this worker's generation.
    #[error("app_worker_updating")]
    Updating,
}

/// How long a call that lost its worker waits for the monitor to name why.
/// The monitor decides within one poll of the kill or exit it observes.
const ENDING_WAIT: std::time::Duration = std::time::Duration::from_secs(1);

/// Why a wake or event delivery did not complete.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DeliveryError {
    #[error(transparent)]
    Worker(#[from] AppWorkerError),
    /// The App's handler answered with an error: its code and message.
    #[error("app_handler_failed: {0}")]
    Handler(String),
    /// Nothing was sent: the worker began stopping first. The item waits for
    /// the next worker without spending an attempt.
    #[error("app_worker_stopping")]
    NotAdmitted,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Starting,
    Active,
    Draining,
    Stopped,
}

struct Admission {
    phase: Mutex<Phase>,
    changed: watch::Sender<Phase>,
    cancellation: WorkerCancellation,
    ending: WorkerEnding,
    broker: Weak<dyn Broker>,
    broker_draining: std::sync::atomic::AtomicBool,
    /// Set for a staged worker that migrates its data before it registers.
    migrating: std::sync::atomic::AtomicBool,
    /// Set when a local update drains this generation.
    updating: std::sync::atomic::AtomicBool,
    /// Wake deliveries in flight. They are not use, but no idle stop or
    /// eviction may interrupt one; see `begin_idle_draining`.
    deliveries: std::sync::atomic::AtomicUsize,
}
/// Held for one wake delivery. Admitted under the phase lock, so an idle stop
/// either sees it and keeps the worker or has already refused it.
pub(crate) struct DeliveryHold(Arc<Admission>);
impl Drop for DeliveryHold {
    fn drop(&mut self) {
        self.0
            .deliveries
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}
impl Admission {
    /// Why this worker no longer admits calls.
    fn closed_error(&self) -> AppWorkerError {
        if self.updating.load(std::sync::atomic::Ordering::Acquire) {
            AppWorkerError::Updating
        } else {
            AppWorkerError::Unavailable
        }
    }
    fn admit_delivery(self: &Arc<Self>) -> Result<DeliveryHold, AppWorkerError> {
        let phase = self.phase.lock().map_err(|_| AppWorkerError::Unavailable)?;
        if *phase != Phase::Active {
            return Err(AppWorkerError::Unavailable);
        }
        self.deliveries
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        drop(phase);
        Ok(DeliveryHold(self.clone()))
    }
    fn delivering(&self) -> bool {
        self.deliveries.load(std::sync::atomic::Ordering::Acquire) > 0
    }
    /// The idle stop's final step: begin draining unless a wake delivery is in
    /// flight, atomically with `admit_delivery`. A worker that is no longer
    /// active admits none, so its stop proceeds.
    fn begin_idle_draining(&self) -> bool {
        {
            let mut phase = self
                .phase
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if *phase != Phase::Active {
                return true;
            }
            if self.delivering() {
                return false;
            }
            *phase = Phase::Draining;
            self.changed.send_replace(Phase::Draining);
        }
        self.notify_broker_draining();
        true
    }
    fn stop(&self) {
        // Poison must never skip cleanup. Queue admission uses the same guard.
        match self.phase.lock() {
            Ok(mut phase) => *phase = Phase::Stopped,
            Err(poisoned) => *poisoned.into_inner() = Phase::Stopped,
        }
        self.changed.send_replace(Phase::Stopped);
        self.cancellation.cancel();
        self.notify_broker_draining();
    }
    fn notify_broker_draining(&self) {
        if !self
            .broker_draining
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            // Never call an external delegate while holding the phase mutex.
            if let Some(broker) = self.broker.upgrade() {
                broker.begin_draining();
            }
        }
    }
    fn begin_draining(&self) -> Result<(), AppWorkerError> {
        {
            let mut phase = self
                .phase
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !matches!(*phase, Phase::Active | Phase::Draining) {
                return Err(AppWorkerError::Unavailable);
            }
            *phase = Phase::Draining;
            self.changed.send_replace(Phase::Draining);
        }
        self.notify_broker_draining();
        Ok(())
    }
    fn active(&self) -> bool {
        self.phase.lock().is_ok_and(|phase| *phase == Phase::Active)
    }
    fn broker_open(&self, method: &str) -> bool {
        self.phase.lock().is_ok_and(|phase| {
            // Migrations only read and write structured state, before ready.
            (*phase == Phase::Starting
                && self.migrating.load(std::sync::atomic::Ordering::Acquire)
                && matches!(method, "state.get" | "state.transaction" | "migration.step"))
                || *phase == Phase::Active
                || (*phase == Phase::Draining
                    && matches!(
                        method,
                        "state.get"
                            | "state.list"
                            | "state.transaction"
                            | "events.emit"
                            | "events.status"
                            | "events.automations"
                            | "events.retry"
                            | "schedule.set"
                            | "schedule.cancel"
                            | "schedule.list"
                            | "files.atomic_replace"
                    ))
        })
    }
}

struct LiveWorker {
    owner: String,
    catalog: Arc<EventCatalog>,
    peer: WorkerPeer,
    admission: Arc<Admission>,
    residency: Residency,
}

/// Kernel-initiated use of one live worker, for the idle stop: a tool call, an
/// inbound event, or a wake armed during either. The App's own timers, broker
/// calls and self-armed wakes do not keep it running (owner decision 6).
struct Residency {
    last_used_ms: std::sync::atomic::AtomicU64,
    used: std::sync::atomic::AtomicBool,
    self_woken: std::sync::atomic::AtomicBool,
}
impl Residency {
    fn new(started_ms: u64) -> Self {
        Self {
            last_used_ms: started_ms.into(),
            used: false.into(),
            self_woken: false.into(),
        }
    }
    fn touch(&self, now_ms: u64) {
        use std::sync::atomic::Ordering;
        self.last_used_ms.store(now_ms, Ordering::Release);
        self.used.store(true, Ordering::Release);
    }
    fn self_woken(&self) {
        self.self_woken
            .store(true, std::sync::atomic::Ordering::Release);
    }
    /// Idle time counts from the last use, or from the start. A worker that
    /// has served only the App's own wakes since it started has no use to
    /// wait for: it stops once its due work is delivered.
    fn idle_ms(&self, now_ms: u64) -> u64 {
        use std::sync::atomic::Ordering;
        if self.self_woken.load(Ordering::Acquire) && !self.used.load(Ordering::Acquire) {
            return u64::MAX;
        }
        now_ms.saturating_sub(self.last_used_ms.load(Ordering::Acquire))
    }
}

/// Cloning a projection does not retain the native owner or revive authority.
#[derive(Clone)]
pub(crate) struct ActivatedApp(Weak<LiveWorker>);

/// Retained operation context; owner invalidation still stops further admission.
pub(crate) struct AppWorkerLease(Arc<LiveWorker>);

/// Must live and drop on AppControl's bounded blocking lifecycle owner, never an
/// async coordinator. Creation is only through start_blocking in startup.rs.
pub(crate) struct AppWorkerOwner {
    process: Option<WorkerProcess>,
    peer: WorkerPeer,
    peer_task: Option<PeerTask>,
    admission: Arc<Admission>,
    catalog: Arc<EventCatalog>,
    live: Option<Arc<LiveWorker>>,
    registration: Option<RegisteredHandlers>,
    runtime: Handle,
}
impl AppWorkerOwner {
    pub(crate) fn stop(&self) {
        self.admission.stop();
        self.peer.close();
    }
    #[cfg(test)]
    pub(crate) fn shutdown_blocking(mut self) {
        let _ = self.shutdown();
    }
    fn shutdown(
        &mut self,
    ) -> Result<
        chariox_app_runtime::worker_process::WorkerExit,
        chariox_app_runtime::worker_process::WorkerError,
    > {
        self.stop();
        let mut unresponsive = false;
        if let Some(task) = self.peer_task.take() {
            // Admitted writer/effect reservations survive cancellation. Drain
            // them before releasing this owner and joining the native process.
            unresponsive = matches!(
                self.runtime.block_on(task.join()),
                Err(chariox_app_runtime::worker_peer::PeerError::Unresponsive)
            );
        }
        let mut result = self
            .process
            .take()
            .ok_or(chariox_app_runtime::worker_process::WorkerError::Supervisor)?
            .shutdown_blocking();
        self.live.take();
        // A blocked worker is terminated here; its exit names that cause.
        if let Ok(exit) = &mut result {
            if unresponsive {
                exit.failure = Some(chariox_app_runtime::worker_process::WorkerError::Unresponsive);
            }
        }
        result
    }
}
impl Drop for AppWorkerOwner {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
impl LiveWorker {
    fn available(&self) -> Result<(), AppWorkerError> {
        if !self.admission.active() || self.peer.is_closed() {
            return Err(self.admission.closed_error());
        }
        Ok(())
    }
    /// A call that lost this worker names an update that drained it or a stop
    /// at its memory limit; any other loss keeps the call's own error.
    async fn lost(&self, error: AppWorkerError) -> AppWorkerError {
        if error != AppWorkerError::Unavailable {
            return error;
        }
        if self.admission.closed_error() == AppWorkerError::Updating {
            return AppWorkerError::Updating;
        }
        match self.admission.ending.failure(ENDING_WAIT).await {
            Some(chariox_app_runtime::worker_process::WorkerError::MemoryLimit) => {
                AppWorkerError::MemoryLimit
            }
            _ => error,
        }
    }
}
impl ActivatedApp {
    pub(crate) fn lease(&self, trusted_owner: &str) -> Result<AppWorkerLease, AppWorkerError> {
        let live = self.0.upgrade().ok_or(AppWorkerError::Unavailable)?;
        if live.owner != trusted_owner {
            return Err(AppWorkerError::Unavailable);
        }
        live.available()?;
        Ok(AppWorkerLease(live))
    }
}
impl AppWorkerLease {
    pub(crate) fn owner(&self) -> &str {
        &self.0.owner
    }
    pub(crate) fn touch(&self) {
        self.0.residency.touch(crate::session::unix_epoch_ms());
    }
    /// While a wake is being delivered the worker is not idle; that does not
    /// move its deadline.
    pub(crate) fn idle_ms(&self, now_ms: u64) -> u64 {
        if self.0.admission.delivering() {
            return 0;
        }
        self.0.residency.idle_ms(now_ms)
    }
    #[cfg(test)]
    pub(crate) fn fixture_hold_delivery(&self) -> DeliveryHold {
        self.0.admission.admit_delivery().unwrap()
    }
    pub(crate) fn catalog(&self) -> &Arc<EventCatalog> {
        &self.0.catalog
    }
    pub(crate) fn is_stopped(&self) -> bool {
        self.0.available().is_err()
    }
    #[cfg(test)]
    pub(crate) async fn cancelled(&self) {
        let mut changed = self.0.admission.changed.subscribe();
        loop {
            if self.is_stopped() {
                return;
            }
            tokio::select! {
                _ = self.0.peer.closed() => return,
                result = changed.changed() => if result.is_err() { return; },
            }
        }
    }
}
