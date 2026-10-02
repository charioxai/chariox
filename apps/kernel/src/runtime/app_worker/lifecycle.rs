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
    pub(crate) fn startup_blocking(
        &self,
        cancelled: impl Fn() -> bool,
    ) -> Result<(), AppWorkerError> {
        if self
            .registration
            .as_ref()
            .is_some_and(|r| r.supports_lifecycle("startup"))
        {
            self.dispatch_blocking(
                "startup",
                serde_json::Value::Null,
                Duration::from_secs(10),
                cancelled,
            )
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
    /// Only the retained owner thread dispatches callbacks. An interrupted or
    /// failed callback is terminal: the SDK can retain exclusion after timeout.
    pub(crate) fn notify_blocking(
        &self,
        event: &'static str,
        data: serde_json::Value,
        cancelled: impl Fn() -> bool,
    ) -> Result<(), AppWorkerError> {
        if cancelled() {
            return Err(AppWorkerError::Unavailable);
        }
        if !self
            .registration
            .as_ref()
            .is_some_and(|r| r.supports_lifecycle(event))
        {
            return Ok(());
        }
        self.dispatch_blocking(event, data, Duration::from_secs(30), cancelled)
    }
    pub(crate) fn begin_draining(&self) -> Result<(), AppWorkerError> {
        self.admission.begin_draining()
    }
    pub(super) fn lifecycle_blocking(
        &self,
        event: &'static str,
        timeout: Duration,
    ) -> Result<(), AppWorkerError> {
        self.dispatch_blocking(event, serde_json::Value::Null, timeout, || false)
    }
    fn dispatch_blocking(
        &self,
        event: &'static str,
        data: serde_json::Value,
        timeout: Duration,
        cancelled: impl Fn() -> bool,
    ) -> Result<(), AppWorkerError> {
        if self.peer.is_closed() {
            return Err(AppWorkerError::Unavailable);
        }
        let slot = self
            .peer
            .reserve(timeout)
            .map_err(|_| AppWorkerError::Busy)?;
        let response = self.runtime.block_on(async {
            let request = slot.request(
                "lifecycle.dispatch", serde_json::json!({"event":event,"data":data}), None,
            );
            tokio::pin!(request);
            loop {
                if cancelled() { return Err(AppWorkerError::Unavailable); }
                tokio::select! {
                    biased;
                    response = &mut request => return response.map_err(|_| AppWorkerError::Deadline),
                    _ = tokio::time::sleep(Duration::from_millis(10)) => {}
                }
            }
        })?;
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
