use serde::{Deserialize, Serialize};

use crate::session::unix_epoch_ms;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionLease {
    pub id: String,
    pub home_kernel_id: String,
    pub home_session_id: String,
    pub home_agent_id: String,
    pub worker_kernel_id: String,
    pub machine_id: String,
    pub created_at_ms: u64,
    pub last_heartbeat_at_ms: u64,
}

impl ExecutionLease {
    pub fn new(
        id: String,
        home_kernel_id: String,
        home_session_id: String,
        home_agent_id: String,
        worker_kernel_id: String,
        machine_id: String,
    ) -> Self {
        let now = unix_epoch_ms();
        Self {
            id,
            home_kernel_id,
            home_session_id,
            home_agent_id,
            worker_kernel_id,
            machine_id,
            created_at_ms: now,
            last_heartbeat_at_ms: now,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeasedAgent {
    pub id: String,
    pub lease_id: String,
    pub home_agent_id: String,
    pub provider: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub backing_session_id: String,
    pub backing_agent_id: String,
    pub backing_attachment_id: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteWorkflowTurnContext {
    pub home_kernel_id: String,
    pub home_session_id: String,
    pub home_agent_id: String,
    pub workflow_run_id: String,
    pub workflow_node_run_id: String,
    pub delivery_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeasedWorkflowTurnBinding {
    pub leased_agent_id: String,
    pub provider_run_id: String,
    pub context: RemoteWorkflowTurnContext,
}

impl LeasedAgent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        lease_id: String,
        home_agent_id: String,
        provider: String,
        model: Option<String>,
        effort: Option<String>,
        backing_session_id: String,
        backing_agent_id: String,
        backing_attachment_id: String,
    ) -> Self {
        Self {
            id,
            lease_id,
            home_agent_id,
            provider,
            model,
            effort,
            backing_session_id,
            backing_agent_id,
            backing_attachment_id,
            created_at_ms: unix_epoch_ms(),
        }
    }
}
