use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentUtilityKind {
    WorkspaceCommitMessage,
    SemanticHistorySearch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceCommitMessageUtilityInput {
    pub workspace_id: String,
    pub worktree_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compare_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticHistorySearchUtilityInput {
    pub query: String,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub workflow_id: Option<String>,
    pub machine_id: Option<String>,
    pub repo_root: Option<String>,
    pub worktree_path: Option<String>,
    pub kind: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentUtilityInput {
    WorkspaceCommitMessage(WorkspaceCommitMessageUtilityInput),
    SemanticHistorySearch(SemanticHistorySearchUtilityInput),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunAgentUtilityRequest {
    pub session_id: String,
    pub agent_id: String,
    pub kind: AgentUtilityKind,
    pub input: AgentUtilityInput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentUtilityOutput {
    WorkspaceCommitMessage {
        message: String,
    },
    SemanticHistorySearch {
        answer: String,
        matches: Vec<SemanticHistoryMatch>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentUtilityResult {
    pub utility_run_id: String,
    pub session_id: String,
    pub agent_id: String,
    pub kind: AgentUtilityKind,
    pub output: AgentUtilityOutput,
    pub generated_at_ms: u64,
}
