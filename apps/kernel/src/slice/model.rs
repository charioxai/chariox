use serde::{Deserialize, Serialize};

use crate::slice_provider_auth::SliceProviderAuthSummary;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SliceBackendKind {
    LocalDocker,
    SshDocker,
}

impl Default for SliceBackendKind {
    fn default() -> Self {
        Self::LocalDocker
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SliceStatus {
    Stopped,
    Starting,
    Stopping,
    Running,
    Unhealthy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SliceDisplayMode {
    Headless,
    Headed,
}

impl Default for SliceDisplayMode {
    fn default() -> Self {
        Self::Headless
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceProviderLoginStart {
    pub provider: String,
    pub login_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_code: Option<String>,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceRecord {
    pub id: String,
    pub name: String,
    pub owner_kernel_id: String,
    pub owner_machine_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agent_ids: Vec<String>,
    pub backend: SliceBackendKind,
    pub os: String,
    #[serde(default)]
    pub display_mode: SliceDisplayMode,
    pub status: SliceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<String>,
    pub workspace_mount: Option<String>,
    pub worker_kernel_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_kernel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_machine_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_endpoint: Option<SliceRelayEndpoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_docker_ports: Option<SliceLocalDockerPorts>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_auth: Vec<SliceProviderAuthSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_endpoint: Option<SliceDisplayEndpoint>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceRelayEndpoint {
    pub url: String,
    #[serde(default)]
    pub private: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceLocalDockerPorts {
    pub codex: u16,
    pub opencode: u16,
    pub kernel: u16,
    pub mcp: u16,
    pub relay: u16,
    pub novnc: u16,
    pub codex_range_start: u16,
    pub opencode_range_start: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SliceDisplayEndpointKind {
    Novnc,
    ArrobaViewer,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SliceDisplayEndpointAccess {
    Local,
    Tunnel,
    Public,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceDisplayEndpoint {
    pub slice_id: String,
    pub kind: SliceDisplayEndpointKind,
    pub url: String,
    pub access: SliceDisplayEndpointAccess,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceLogEntry {
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub text: String,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct CreateSliceInput {
    pub name: String,
    pub backend: SliceBackendKind,
    pub os: String,
    pub display_mode: SliceDisplayMode,
    pub workspace_id: Option<String>,
    pub worktree_id: Option<String>,
    pub workspace_mount: Option<String>,
    pub worker_kernel_ref: Option<String>,
    pub display_url: Option<String>,
    pub provider_auth: Vec<SliceProviderAuthSummary>,
    pub now_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalDockerSliceAction {
    Provision,
    ImportProviderAuth,
    RemoveProviderAuth,
    Stop,
    Destroy,
}
