use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasAgentRequest {
    pub session_id: String,
    pub agent_id: String,
    pub alias: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnAgentRequest {
    pub session_id: String,
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<crate::provider::AgentExecutionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_level: Option<crate::provider::AgentPermissionLevel>,
    pub worktree_id: Option<String>,
    #[serde(default)]
    pub kernel_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slice_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_placement: Option<crate::agent::GitWorktreePlacement>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub metaagent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnAgentsRequest {
    pub session_id: String,
    pub agents: Vec<SpawnAgentsRequestItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnAgentsRequestItem {
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<crate::provider::AgentExecutionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_level: Option<crate::provider::AgentPermissionLevel>,
    pub worktree_id: Option<String>,
    #[serde(default)]
    pub kernel_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slice_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_placement: Option<crate::agent::GitWorktreePlacement>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub metaagent: bool,
}

impl SpawnAgentsRequestItem {
    pub fn into_spawn_agent_request(self, session_id: String) -> SpawnAgentRequest {
        SpawnAgentRequest {
            session_id,
            alias: self.alias,
            provider: self.provider,
            model: self.model,
            effort: self.effort,
            execution_mode: self.execution_mode,
            permission_level: self.permission_level,
            worktree_id: self.worktree_id,
            kernel_ref: self.kernel_ref,
            slice_ref: self.slice_ref,
            worktree_placement: self.worktree_placement,
            metaagent: self.metaagent,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct UndoTurnRequest {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ForkAgentRequest {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_agent_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnUndoResult {
    pub session_id: String,
    pub agent_id: String,
    pub turn_id: String,
    pub prompt_id: String,
    pub provider_run_id: String,
    pub reverted_paths: Vec<String>,
    pub path_results: Vec<crate::workspace_live_sync_journal::WorkspaceLiveSyncPathApplyResult>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveAgentToRemoteRequest {
    pub session_id: String,
    pub agent_ref: String,
    pub machine_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveAgentToLocalRequest {
    pub session_id: String,
    pub agent_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestroyAgentRequest {
    pub session_id: String,
    pub agent_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FocusAgentRequest {
    pub session_id: String,
    pub agent_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcknowledgeAgentOutputSeenRequest {
    pub session_id: String,
    pub agent_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycleAgentFocusRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListAgentsRequest {
    pub session_id: String,
}
