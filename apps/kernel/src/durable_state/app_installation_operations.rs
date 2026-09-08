//! Idempotent first installs on the existing kernel writer. This records no
//! client assertion of trust, approval, containment or worker health.
mod api;
mod commit;
mod store;
#[cfg(test)]
mod tests;
mod transitions;

use super::{
    app_activation::CommittedAppActivation, app_worker_lifecycle::ActiveStartAdmission,
    DurableKernelStateStore, DurableWriterRequest,
};
use crate::{
    error::DaemonError,
    runtime::{app_operation_budget::AppOperationBudget, app_worker::FirstInstallHealth},
};
use chariox_app_runtime::{
    installation::{CapabilityApproval, StageToken, StageTrustBinding, VerifiedInstallCandidate},
    publisher_trust::TrustedPublisherSnapshot,
};
use rusqlite::Connection;
use std::sync::{mpsc, Arc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum InstallOperationError {
    #[error("app_install_invalid")]
    Invalid,
    #[error("app_install_not_found")]
    NotFound,
    #[error("app_install_conflict")]
    Conflict,
    #[error("app_install_approval_required")]
    ApprovalRequired,
    #[error("app_install_stale")]
    Stale,
    #[error("app_install_limit")]
    Limit,
    #[error("app_install_stopped")]
    Stopped,
    #[error("app_install_storage")]
    Storage,
    #[error("app_install_commit_unknown")]
    CommitUnknown,
}
type Result<T> = std::result::Result<T, InstallOperationError>;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InstallPhase {
    AwaitingApproval,
    Starting,
    Committed,
    Cancelled,
    Failed,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstallOperation {
    pub(crate) request_id: String,
    pub(crate) token: StageToken,
    pub(crate) package_digest: String,
    pub(crate) phase: InstallPhase,
    pub(crate) attempt: Option<String>,
    pub(crate) failure: Option<String>,
    pub(crate) cleanup_pending: bool,
}
/// Minted only after reading the existing exact staged capability approval and
/// current enrolled signer on the writer. Not serializable or caller-created.
pub(crate) struct ApprovedFirstInstall {
    owner: String,
    request_id: String,
    attempt: String,
    binding: StageTrustBinding,
    trust: TrustedPublisherSnapshot,
    approval: CapabilityApproval,
}
impl ApprovedFirstInstall {
    pub(crate) fn owner(&self) -> &str {
        &self.owner
    }
    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
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
pub(crate) struct FirstInstallCommitted {
    pub(crate) activation: CommittedAppActivation,
    pub(crate) active: ActiveStartAdmission,
}
pub(super) struct AppInstallationOperationRequest {
    command: Command,
    response: mpsc::Sender<Result<Reply>>,
}
impl std::fmt::Debug for AppInstallationOperationRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppInstallationOperationRequest")
            .finish_non_exhaustive()
    }
}
enum Command {
    Replay {
        owner: String,
        request_id: String,
        digest: String,
        budget: AppOperationBudget,
    },
    Begin {
        owner: String,
        request_id: String,
        candidate: VerifiedInstallCandidate,
        budget: AppOperationBudget,
    },
    Claim {
        owner: String,
        request_id: String,
        attempt: String,
        budget: AppOperationBudget,
    },
    Verify {
        admission: Arc<ApprovedFirstInstall>,
        budget: AppOperationBudget,
    },
    Commit {
        admission: Arc<ApprovedFirstInstall>,
        health: FirstInstallHealth,
        budget: AppOperationBudget,
        #[cfg(test)]
        fault: Option<CommitTestFault>,
    },
    Finish {
        admission: Arc<ApprovedFirstInstall>,
        cancelled: bool,
        failure: String,
        budget: AppOperationBudget,
    },
    Cancel {
        owner: String,
        request_id: String,
        budget: AppOperationBudget,
    },
}
enum Reply {
    Operation(InstallOperation),
    Approved(ApprovedFirstInstall),
    Committed(FirstInstallCommitted),
    Done,
}
#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) struct CommitTestFault {
    pub(crate) fail_reconciliation: bool,
}
pub(super) fn initialize(connection: &Connection) -> std::result::Result<(), DaemonError> {
    store::initialize(connection).map_err(|_| DaemonError::LocalTransport {
        operation: "durable_state.first_app_install",
        message: "App installation operation schema could not be initialized".into(),
    })
}
/// Unknown COMMIT fences the same writer using its existing fatal disposition;
/// no ordinary queued write may continue after irreconcilable uncertainty.
pub(super) fn execute(
    connection: &mut Connection,
    request: AppInstallationOperationRequest,
) -> super::app_event_delivery::WriterDisposition {
    let result = transitions::apply(connection, request.command);
    let disposition = if matches!(&result, Err(InstallOperationError::CommitUnknown)) {
        super::app_event_delivery::WriterDisposition::Stop
    } else {
        super::app_event_delivery::WriterDisposition::Continue
    };
    let _ = request.response.send(result);
    disposition
}
