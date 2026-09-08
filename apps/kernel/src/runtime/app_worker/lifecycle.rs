//! Lifecycle calls use the owner's existing SDK channel and registration. The
//! draining phase stops tool/pump admission while a final handler flushes state.
use super::*;
use chariox_app_runtime::{
    wire::{Message, Outcome},
    worker_process::{WorkerError, WorkerExit},
};
use std::time::Duration;

/// A stop request can withdraw callable handles even while the blocking owner
/// is waiting for a durable response. It never retains the process/peer owner.
#[derive(Clone)]
pub(crate) struct AppWorkerDrain(Weak<Admission>);
impl AppWorkerDrain {
    pub(crate) fn begin(&self) {
        let Some(admission) = self.0.upgrade() else {
            return;
        };
        let _ = admission.begin_draining();
    }
}

impl AppWorkerOwner {
    pub(crate) fn drain_handle(&self) -> AppWorkerDrain {
        AppWorkerDrain(Arc::downgrade(&self.admission))
    }
    pub(crate) fn is_closed(&self) -> bool {
        self.peer.is_closed()
            || self
                .admission
                .phase
                .lock()
                .map_or(true, |phase| *phase == Phase::Stopped)
    }
    pub(crate) fn startup_blocking(&self) -> Result<(), AppWorkerError> {
        if self
            .registration
            .as_ref()
            .is_some_and(|r| r.supports_lifecycle("startup"))
        {
            self.lifecycle_blocking("startup", Duration::from_secs(10))
        } else {
            Ok(())
        }
    }
    pub(crate) fn drain_blocking(&self) -> Result<(), AppWorkerError> {
        self.admission.begin_draining()?;
        // Even without a registered callback, the trusted bootstrap consumes
        // this response and exits only after its channel write has drained.
        self.lifecycle_blocking("shutdown", Duration::from_secs(3))
    }
    pub(super) fn lifecycle_blocking(
        &self,
        event: &'static str,
        timeout: Duration,
    ) -> Result<(), AppWorkerError> {
        if self.peer.is_closed() {
            return Err(AppWorkerError::Unavailable);
        }
        let slot = self
            .peer
            .reserve(timeout)
            .map_err(|_| AppWorkerError::Busy)?;
        let response = self
            .runtime
            .block_on(slot.request(
                "lifecycle.dispatch",
                serde_json::json!({"event":event,"data":null}),
                None,
            ))
            .map_err(|_| AppWorkerError::Deadline)?;
        match response {
            Message::Response {
                outcome: Outcome::Success(_),
                ..
            } => Ok(()),
            _ => Err(AppWorkerError::Unavailable),
        }
    }
    /// Consumes the retained owner only after broker drain and actual native
    /// reap, returning bounded diagnostic bytes for the kernel's log policy.
    pub(crate) fn finish_blocking(mut self) -> Result<WorkerExit, WorkerError> {
        self.shutdown()
    }
}
