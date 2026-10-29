//! Verified preparation is distinct from the durable activation acknowledgment.
use super::{error, AgentMoveIdentity};
use crate::error::DaemonError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMoveRequirements {
    pub agent_configuration_sha256: String,
    pub extensions: crate::extension::RemoteExtensionManifest,
    pub skills: Vec<crate::transport::relay_peer::RequiredRemoteSkill>,
    pub environment_definition: Option<crate::session::ProjectEnvironmentDefinition>,
}

impl AgentMoveRequirements {
    pub(crate) fn sha256(&self) -> Result<String, DaemonError> {
        digest(self)
    }
}

pub(crate) fn agent_configuration_sha256(
    agent: &crate::agent::AgentInstance,
) -> Result<String, DaemonError> {
    digest(&(
        agent.provider(),
        agent.provider_account_profile(),
        agent.model(),
        agent.effort(),
        agent.extension_grants(),
        agent.execution_mode_override(),
        agent.permission_level_override(),
    ))
}

fn digest(value: &impl Serialize) -> Result<String, DaemonError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| error("Cannot bind Move requirements; retry preparation"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMoveReadiness {
    pub identity: AgentMoveIdentity,
    pub requirements_sha256: String,
    pub workspace_sha256: String,
    pub native_sha256: Option<String>,
    pub target_project_id: String,
    pub target_account_profile: String,
    pub backing_session_id: String,
    pub backing_agent_id: String,
    pub environment_setup_operation_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMoveCommitReceipt {
    pub readiness: AgentMoveReadiness,
    pub committed_at_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentMoveComposition {
    pub requirements: Option<AgentMoveRequirements>,
    pub readiness: Option<AgentMoveReadiness>,
    pub commit: Option<AgentMoveCommitReceipt>,
}
