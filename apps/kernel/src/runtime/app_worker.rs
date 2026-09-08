//! One blocking lifecycle owner constructs its peer from the actual process FD.
//! AppControl publishes weak callable handles only after that peer's exact SDK
//! report and a durable active-generation proof. Neither is permanent authority.

mod call;
mod lifecycle;
pub(crate) use lifecycle::AppWorkerDrain;
mod startup;
#[cfg(test)]
mod tests;
pub(crate) use call::{
    AppCallSlot, AppToolError, AppToolReply, AppToolResponse, PreparedAppToolCall,
};
pub(crate) use startup::RegisteredAppWorker;

use chariox_app_runtime::{
    app_outbox::EventCatalog,
    worker_peer::{Broker, PeerTask, WorkerPeer},
    worker_process::{WorkerCancellation, WorkerProcess},
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
    #[error("app_worker_deadline")]
    Deadline,
    #[error("app_worker_request_invalid")]
    Invalid,
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
    broker: Weak<dyn Broker>,
    broker_draining: std::sync::atomic::AtomicBool,
}
impl Admission {
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
            *phase == Phase::Active
                || (*phase == Phase::Draining
                    && matches!(
                        method,
                        "state.get"
                            | "state.list"
                            | "state.transaction"
                            | "events.emit"
                            | "events.status"
                            | "events.retry"
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
        if let Some(task) = self.peer_task.take() {
            // Admitted writer/effect reservations survive cancellation. Drain
            // them before releasing this owner and joining the native process.
            let _ = self.runtime.block_on(task.join());
        }
        let result = self
            .process
            .take()
            .ok_or(chariox_app_runtime::worker_process::WorkerError::Supervisor)?
            .shutdown_blocking();
        self.live.take();
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
            return Err(AppWorkerError::Unavailable);
        }
        Ok(())
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
    pub(crate) fn catalog(&self) -> &Arc<EventCatalog> {
        &self.0.catalog
    }
    pub(crate) fn is_stopped(&self) -> bool {
        self.0.available().is_err()
    }
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
