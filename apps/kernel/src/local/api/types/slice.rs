use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListSlicesRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateSliceRequest {
    pub name: String,
    #[serde(default)]
    pub backend: SliceBackendKind,
    #[serde(default = "default_slice_os")]
    pub os: String,
    #[serde(default)]
    pub display_mode: crate::slice::SliceDisplayMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_mount: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_kernel_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_auth: Vec<crate::slice_provider_auth::SliceProviderAuthSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_saved_state: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceRefRequest {
    pub slice_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceStateSaveRequest {
    pub slice_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<SliceStateSaveMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SliceStateSaveMode {
    RestartAgents,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceStateStatusRequest {
    pub slice_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceStateResetRequest {
    pub slice_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateSliceBackupRequest {
    pub slice_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetSliceLogsRequest {
    pub slice_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail_lines: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListSliceAuditRequest {
    pub slice_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportSliceProviderAuthRequest {
    pub slice_ref: String,
    pub provider: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveSliceProviderAuthRequest {
    pub slice_ref: String,
    pub provider: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartSliceProviderLoginRequest {
    pub slice_ref: String,
    pub provider: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetSliceProviderAuthAliasRequest {
    pub slice_ref: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

fn default_slice_os() -> String {
    "linux".to_string()
}
