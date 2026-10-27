//! Home owns the move boundary. Artifacts and target publications never grant dispatch.
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::agent::{AgentInstance, RemoteAgentBinding};
use crate::managed_context::development::{
    DevelopmentImportedRepository, DevelopmentSourceRepositoryBinding,
};
use crate::provider::NativeCheckpointBinding;

mod artifacts;
mod composition;
pub(crate) use composition::{agent_configuration_sha256, AgentMoveComposition};
pub use composition::{AgentMoveCommitReceipt, AgentMoveReadiness, AgentMoveRequirements};
pub mod peer;
mod store;
pub use artifacts::AgentMoveArtifactKind;
pub(crate) use artifacts::MOVE_CHUNK_BYTES;
pub(crate) use store::AgentMoveStore;

pub(crate) fn error(message: impl Into<String>) -> crate::error::DaemonError {
    crate::error::DaemonError::LocalTransport {
        operation: "agent.move",
        message: message.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMoveIdentity {
    pub operation_id: String,
    pub generation: u64,
    pub home_kernel_id: String,
    pub home_session_id: String,
    pub home_agent_id: String,
    pub owner_user_id: String,
    pub source_kernel_id: String,
    pub target_kernel_id: String,
    pub home_key_thumbprint: String,
    pub source_key_thumbprint: String,
    pub target_key_thumbprint: String,
    pub target_public_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMoveArtifact {
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMoveCapture {
    pub identity: AgentMoveIdentity,
    pub source_project_id: String,
    pub archive_project_id: String,
    pub source_provider_run_id: Option<String>,
    pub source_repositories: Vec<DevelopmentSourceRepositoryBinding>,
    pub workspace: AgentMoveArtifact,
    pub native: Option<AgentMoveArtifact>,
    pub checkpoint_binding: Option<NativeCheckpointBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMoveMaterialization {
    pub identity: AgentMoveIdentity,
    pub archive_sha256: String,
    pub source_project_id: String,
    pub target_project_id: String,
    pub destination_root: PathBuf,
    pub primary_repository_id: String,
    pub repositories: Vec<DevelopmentImportedRepository>,
}

impl AgentMoveMaterialization {
    pub(crate) fn primary_path(&self) -> Result<&std::path::Path, crate::error::DaemonError> {
        self.repositories
            .iter()
            .find(|repository| {
                repository.repository_id == self.primary_repository_id
                    && repository.role
                        == crate::managed_context::development::DevelopmentRepositoryRole::Primary
            })
            .map(|repository| repository.destination_path.as_path())
            .ok_or_else(|| error("Move receipt has no primary workspace; retry the import"))
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum HomeMovePhase {
    Fenced,
    SourceStopped,
    Cancelling,
    Captured {
        capture: AgentMoveCapture,
    },
    Materialized {
        capture: AgentMoveCapture,
        receipt: AgentMoveMaterialization,
    },
    Bound {
        capture: AgentMoveCapture,
        receipt: AgentMoveMaterialization,
        remote: Option<RemoteAgentBinding>,
    },
    Committed {
        receipt: AgentMoveMaterialization,
    },
    Complete {
        receipt: AgentMoveMaterialization,
    },
    RolledBack {
        reason: String,
    },
}

impl HomeMovePhase {
    pub(crate) fn is_pending(&self) -> bool {
        !matches!(self, Self::Complete { .. } | Self::RolledBack { .. })
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HomeMoveOperation {
    #[serde(default)]
    pub composition: AgentMoveComposition,
    pub identity: AgentMoveIdentity,
    pub revision: u64,
    pub original_agent: AgentInstance,
    pub target_ref: Option<String>,
    pub source_remote: Option<RemoteAgentBinding>,
    pub phase: HomeMovePhase,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MoveSource {
    pub identity: AgentMoveIdentity,
    pub session_id: String,
    pub agent_id: String,
    pub provider_run_id: Option<String>,
    pub stopped: bool,
    pub released: bool,
    pub capture: Option<AgentMoveCapture>,
    pub retired: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MoveTarget {
    #[serde(default)]
    pub composition: AgentMoveComposition,
    pub capture: AgentMoveCapture,
    pub destination_root: PathBuf,
    pub receipt: Option<AgentMoveMaterialization>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_lease_id: Option<String>,
    pub backing_session_id: Option<String>,
    pub backing_agent_id: Option<String>,
    pub active: bool,
    pub ready: bool,
    pub target_account_profile: Option<String>,
    pub backing: Option<MoveBacking>,
    pub retired: bool,
    pub project: Option<crate::session::RuntimeProject>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MoveBacking {
    pub lease: crate::execution_lease::ExecutionLease,
    pub leased_agent: crate::execution_lease::LeasedAgent,
    pub session: crate::session::RuntimeSession,
    pub agent: AgentInstance,
    pub attachment: crate::attachment::RuntimeAttachment,
    pub attachment_owner_user_id: String,
    pub execution_caller: Option<crate::app::LeaseCallerBinding>,
    pub agent_caller: Option<crate::app::LeaseCallerBinding>,
}
