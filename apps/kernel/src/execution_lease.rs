use serde::{Deserialize, Serialize};

use crate::session::{unix_epoch_ms, DEFAULT_LOCAL_USER_ID};

fn default_lease_owner_user_id() -> String {
    DEFAULT_LOCAL_USER_ID.to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionLease {
    pub id: String,
    pub home_kernel_id: String,
    pub home_session_id: String,
    pub home_agent_id: String,
    #[serde(default)]
    pub home_agent_metaagent: bool,
    #[serde(default = "default_lease_owner_user_id")]
    pub owner_user_id: String,
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
        home_agent_metaagent: bool,
        owner_user_id: String,
        worker_kernel_id: String,
        machine_id: String,
    ) -> Self {
        let now = unix_epoch_ms();
        Self {
            id,
            home_kernel_id,
            home_session_id,
            home_agent_id,
            home_agent_metaagent,
            owner_user_id,
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
    pub execution_mode: Option<crate::provider::AgentExecutionMode>,
    pub permission_level: Option<crate::provider::AgentPermissionLevel>,
    pub backing_session_id: String,
    pub backing_agent_id: String,
    pub backing_attachment_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projected_prompt_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projected_completion_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projected_output_history_keys: Vec<String>,
    #[serde(skip)]
    pub projected_provider_run: Option<(String, crate::provider::ProviderRunState)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_home_prompt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_home_prompt_started_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub applied_home_steer_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replayable_completion: Option<LeasedCompletionReplay>,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeasedCompletionReplay {
    pub provider_run_id: String,
    pub message_id: String,
    pub completed_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_prompt_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteWorkflowTurnContext {
    pub home_kernel_id: String,
    pub home_session_id: String,
    pub home_agent_id: String,
    pub workflow_run_id: String,
    pub workflow_node_run_id: String,
    pub delivery_token: String,
    /// Capability snapshot selected by the home workflow event binding.
    /// Older peers default to disabled, preserving the safe behavior.
    #[serde(default)]
    pub event_reply_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeasedWorkflowTurnBinding {
    pub leased_agent_id: String,
    pub provider_run_id: String,
    /// Home/backing prompt that owns this context. Multiple queued prompts may
    /// share one provider run, so the provider run is not a sufficient key.
    pub home_prompt_id: String,
    /// Prompt id in the worker/session queue. This is the stable promotion key
    /// even though the queue item is not marked with a workflow source.
    pub backing_prompt_id: String,
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
        execution_mode: Option<crate::provider::AgentExecutionMode>,
        permission_level: Option<crate::provider::AgentPermissionLevel>,
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
            execution_mode,
            permission_level,
            backing_session_id,
            backing_agent_id,
            backing_attachment_id,
            projected_prompt_ids: Vec::new(),
            projected_completion_keys: Vec::new(),
            projected_output_history_keys: Vec::new(),
            projected_provider_run: None,
            active_home_prompt_id: None,
            active_home_prompt_started_at_ms: None,
            applied_home_steer_ids: Vec::new(),
            replayable_completion: None,
            created_at_ms: unix_epoch_ms(),
        }
    }
}
