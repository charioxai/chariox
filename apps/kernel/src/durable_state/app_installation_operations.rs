//! Idempotent first installs and local updates on the existing kernel writer.
//! This records no client assertion of trust, approval, containment or worker
//! health.
mod api;
mod commit;
mod deployment_consent;
mod public_ops;
mod store;
#[cfg(test)]
mod tests;
mod transitions;
pub(crate) use deployment_consent::{
    ConsentStatus, ConsentedApp, DeploymentConsent, CONSENT_TTL_MS,
};
pub(crate) use public_ops::{InstallApprovalChallenge, InstallReviewDisposition};

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
    #[error("app_update_schema_downgrade")]
    SchemaDowngrade,
}
type Result<T> = std::result::Result<T, InstallOperationError>;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InstallPhase {
    Preparing,
    AwaitingApproval,
    Starting,
    Committed,
    Cancelled,
    Failed,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstallOperation {
    pub(crate) input: Option<InstallInput>,
    pub(crate) review: Option<serde_json::Value>,
    pub(crate) interaction_id: Option<String>,
    pub(crate) request_id: String,
    pub(crate) token: StageToken,
    pub(crate) package_digest: String,
    pub(crate) phase: InstallPhase,
    pub(crate) attempt: Option<String>,
    pub(crate) failure: Option<String>,
    pub(crate) cleanup_pending: bool,
    /// The owner approved its capabilities (recorded with the staged release).
    pub(crate) approved: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstallInput {
    pub(crate) session_id: String,
    pub(crate) upload_handle: String,
    /// A local replacement of an existing installation; `None` installs anew.
    pub(crate) update: Option<UpdateTarget>,
    /// Protocol 367: a deployment copy's install from the local release store
    /// (`upload_handle` is empty).
    pub(crate) deployment: Option<DeploymentInstall>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeploymentInstall {
    /// The interaction of the owner's deployment consent.
    pub(crate) consent: String,
    pub(crate) deployment_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpdateTarget {
    pub(crate) installation_id: String,
    pub(crate) expected_generation: u64,
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
    Public(public_ops::PublicCommand),
    Consent(deployment_consent::ConsentCommand),
    Replay {
        owner: String,
        request_id: String,
        digest: String,
        budget: AppOperationBudget,
    },
    #[allow(
        dead_code,
        reason = "Keep the existing typed writer operation or receipt payload for API and regression compatibility"
    )]
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
    Review(InstallReviewDisposition),
    Operation(InstallOperation),
    Approved(ApprovedFirstInstall),
    Committed(FirstInstallCommitted),
    Consent(DeploymentConsent),
    Done,
}
#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) struct CommitTestFault {
    pub(crate) fail_reconciliation: bool,
}
pub(super) fn initialize(connection: &Connection) -> std::result::Result<(), DaemonError> {
    store::initialize(connection)
        .and_then(|()| deployment_consent::initialize(connection))
        .map_err(|_| DaemonError::LocalTransport {
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
