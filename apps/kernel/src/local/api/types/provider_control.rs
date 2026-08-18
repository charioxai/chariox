use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchProviderRunRequest {
    pub session_id: String,
    pub agent_id: Option<String>,
    pub adapter_key: String,
    pub provider: String,
    pub account_profile: String,
    pub model: String,
    pub variant: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_session_id: Option<String>,
    #[serde(default)]
    pub native_tui: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchProviderRunsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrency: Option<usize>,
    pub launches: Vec<LaunchProviderRunRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRunBatchLaunchResult {
    pub index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub provider_run: RuntimeProviderRun,
    #[serde(default)]
    pub reused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateAgentConfigRequest {
    pub session_id: String,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<crate::provider::AgentExecutionMode>,
    #[serde(default)]
    pub clear_execution_mode: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_level: Option<crate::provider::AgentPermissionLevel>,
    #[serde(default)]
    pub clear_permission_level: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub clear_workspace_id: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<String>,
    #[serde(default)]
    pub clear_worktree_id: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateAgentProfileRequest {
    pub session_id: String,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default)]
    pub clear_effort: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentSubstituteAction {
    Add {
        provider: String,
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        variant: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kernel_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        worktree_id: Option<String>,
    },
    Remove {
        index: usize,
    },
    Clear {},
    SetTimeout {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    Activate {
        index: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Primary {},
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateAgentSubstitutesRequest {
    pub session_id: String,
    pub agent_id: String,
    pub action: AgentSubstituteAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetProviderRunRequest {
    pub provider_run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateProviderRunSelectionRequest {
    pub session_id: String,
    pub provider_run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(default)]
    pub clear_variant: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetProviderCatalogRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetProviderCommandCatalogsRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetProviderAuthStatusRequest {
    pub provider: String,
    pub account_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartProviderLoginRequest {
    pub provider: String,
    pub account_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogoutProviderRequest {
    pub provider: String,
    pub account_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListProviderAccountProfilesRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetProviderAccountProfileRequest {
    pub provider: String,
    pub account_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateProviderAccountProfileRequest {
    pub provider: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkProviderAccountProfileRequest {
    pub provider: String,
    pub label: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenameProviderAccountProfileRequest {
    pub provider: String,
    pub account_profile: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetDefaultProviderAccountProfileRequest {
    pub provider: String,
    pub account_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshProviderAccountProfileRequest {
    pub provider: String,
    pub account_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveProviderAccountProfileRequest {
    pub provider: String,
    pub account_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteProviderAccountProfileDataRequest {
    pub provider: String,
    pub account_profile: String,
    pub confirmation_profile_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListProviderProcessesRequest {
    pub provider: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeardownProviderProcessesRequest {
    pub provider: Option<String>,
    #[serde(default)]
    pub force: bool,
}
