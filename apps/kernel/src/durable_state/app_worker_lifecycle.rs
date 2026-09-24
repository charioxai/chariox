//! Runtime health and restart intent on the sole kernel writer. These rows do
//! not approve installations or replace package, publisher or process proofs.
mod store;
#[cfg(test)]
mod tests;

use super::{DurableKernelStateStore, DurableWriterRequest};
use crate::{error::DaemonError, runtime::app_operation_budget::AppOperationBudget};
use chariox_app_runtime::{
    installation::StageTrustBinding, publisher_trust::TrustedPublisherSnapshot,
};
use rusqlite::Connection;
use std::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum LifecycleStoreError {
    #[error("app_lifecycle_stale")]
    Stale,
    #[error("app_lifecycle_stopped")]
    Stopped,
    #[error("app_lifecycle_storage")]
    Storage,
}
pub(crate) type Result<T> = std::result::Result<T, LifecycleStoreError>;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkerPhase {
    Starting,
    Running,
    Stopped,
    Failed,
}
impl WorkerPhase {
    fn name(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkerStatus {
    pub(crate) generation: u64,
    pub(crate) attempt: String,
    pub(crate) phase: WorkerPhase,
    pub(crate) desired_running: bool,
    pub(crate) failure: Option<String>,
    pub(crate) updated_ms: u64,
}
/// Only the current writer transaction can mint this retained start snapshot.
pub(crate) struct ActiveStartAdmission {
    owner: String,
    installation: String,
    attempt: String,
    binding: StageTrustBinding,
    trust: TrustedPublisherSnapshot,
}
impl ActiveStartAdmission {
    /// Initial desired-running state is atomic with first activation. Recovery
    /// cannot observe an active generation without its durable start intent.
    pub(super) fn first_committed_in(
        transaction: &rusqlite::Transaction<'_>,
        owner: &str,
        attempt: &str,
        binding: &StageTrustBinding,
        trust: &TrustedPublisherSnapshot,
    ) -> Result<Self> {
        store::first_committed(transaction, owner, attempt, binding, trust)
    }
    pub(crate) fn owner(&self) -> &str {
        &self.owner
    }
    pub(crate) fn installation(&self) -> &str {
        &self.installation
    }
    pub(crate) fn attempt(&self) -> &str {
        &self.attempt
    }
    pub(crate) fn binding(&self) -> &StageTrustBinding {
        &self.binding
    }
    pub(crate) fn trust(&self) -> &TrustedPublisherSnapshot {
        &self.trust
    }
}
pub(super) struct AppWorkerLifecycleRequest {
    command: Command,
    response: mpsc::Sender<Result<Reply>>,
}
impl std::fmt::Debug for AppWorkerLifecycleRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppWorkerLifecycleRequest")
            .finish_non_exhaustive()
    }
}
enum Command {
    Claim {
        owner: String,
        installation: String,
        attempt: String,
        recovery: bool,
        budget: AppOperationBudget,
    },
    Transition {
        owner: String,
        installation: String,
        attempt: String,
        binding: StageTrustBinding,
        trust: TrustedPublisherSnapshot,
        phase: WorkerPhase,
        desired_running: bool,
        failure: Option<String>,
        budget: AppOperationBudget,
    },
    Verify {
        owner: String,
        attempt: String,
        binding: StageTrustBinding,
        trust: TrustedPublisherSnapshot,
        budget: AppOperationBudget,
    },
    Stop {
        owner: String,
        installation: String,
        budget: AppOperationBudget,
    },
    FinishStop {
        owner: String,
        installation: String,
        budget: AppOperationBudget,
    },
}
enum Reply {
    Admitted(ActiveStartAdmission),
    Done,
}

impl DurableKernelStateStore {
    fn worker_lifecycle(&self, command: Command) -> Result<Reply> {
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppWorkerLifecycle(Box::new(
                AppWorkerLifecycleRequest { command, response },
            )))
            .map_err(|_| LifecycleStoreError::Storage)?;
        receiver.recv().map_err(|_| LifecycleStoreError::Storage)?
    }
    pub(crate) fn claim_active_app_start(
        &self,
        owner: &str,
        installation: &str,
        attempt: &str,
        recovery: bool,
        budget: AppOperationBudget,
    ) -> Result<ActiveStartAdmission> {
        match self.worker_lifecycle(Command::Claim {
            owner: owner.into(),
            installation: installation.into(),
            attempt: attempt.into(),
            recovery,
            budget,
        })? {
            Reply::Admitted(value) => Ok(value),
            _ => Err(LifecycleStoreError::Storage),
        }
    }
    pub(crate) fn verify_app_start(
        &self,
        admission: &ActiveStartAdmission,
        budget: AppOperationBudget,
    ) -> Result<()> {
        self.worker_lifecycle(Command::Verify {
            owner: admission.owner.clone(),
            attempt: admission.attempt.clone(),
            binding: admission.binding.clone(),
            trust: admission.trust.clone(),
            budget,
        })?;
        Ok(())
    }
    pub(crate) fn record_app_worker(
        &self,
        admission: &ActiveStartAdmission,
        phase: WorkerPhase,
        desired_running: bool,
        failure: Option<&str>,
        budget: AppOperationBudget,
    ) -> Result<()> {
        self.worker_lifecycle(Command::Transition {
            owner: admission.owner.clone(),
            installation: admission.installation.clone(),
            attempt: admission.attempt.clone(),
            binding: admission.binding.clone(),
            trust: admission.trust.clone(),
            phase,
            desired_running,
            failure: failure.map(str::to_owned),
            budget,
        })?;
        Ok(())
    }
    pub(crate) fn stop_app_worker_intent(
        &self,
        owner: &str,
        installation: &str,
        budget: AppOperationBudget,
    ) -> Result<()> {
        self.worker_lifecycle(Command::Stop {
            owner: owner.into(),
            installation: installation.into(),
            budget,
        })?;
        Ok(())
    }
    /// Called only after the serialized lifecycle owner has joined the worker.
    pub(crate) fn finish_app_worker_stop(
        &self,
        owner: &str,
        installation: &str,
        budget: AppOperationBudget,
    ) -> Result<()> {
        self.worker_lifecycle(Command::FinishStop {
            owner: owner.into(),
            installation: installation.into(),
            budget,
        })?;
        Ok(())
    }
    pub(crate) fn app_worker_status(
        &self,
        owner: &str,
        installation: &str,
    ) -> Result<Option<WorkerStatus>> {
        let connection = self
            .lock_connection("durable_state.app_worker_status")
            .map_err(|_| LifecycleStoreError::Storage)?;
        store::status(&connection, owner, installation)
    }
    /// Kernel recovery scans a bounded page of its own authoritative records;
    /// owner IDs are selected from the database, never supplied by an App.
    pub(crate) fn app_worker_recovery_candidates(
        &self,
        after: Option<(&str, &str)>,
    ) -> Result<Vec<(String, String)>> {
        let connection = self
            .lock_connection("durable_state.app_worker_recovery")
            .map_err(|_| LifecycleStoreError::Storage)?;
        store::candidates(&connection, after)
    }
}
pub(super) fn initialize(connection: &Connection) -> std::result::Result<(), DaemonError> {
    store::initialize(connection).map_err(|_| DaemonError::LocalTransport {
        operation: "durable_state.app_worker_lifecycle",
        message: "App worker lifecycle schema could not be initialized".into(),
    })
}
pub(super) fn execute(connection: &mut Connection, request: AppWorkerLifecycleRequest) {
    let result = store::apply(connection, request.command);
    let _ = request.response.send(result);
}
