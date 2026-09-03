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
pub enum SliceOperationStatus {
    Accepted,
    InProgress,
    Completed,
    Failed,
    Reconciled,
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
    /// Durable reservation of this physical browser/profile for one Room.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_session_id: Option<String>,
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
    pub last_operation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_operation_status: Option<SliceOperationStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_operation_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<String>,
    pub workspace_mount: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub development: Option<crate::managed_context::package::ManagedContextDevelopmentSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub development_storage_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub development_publication: Option<SliceDevelopmentPublication>,
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
    pub saved_state_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_state_status: Option<SliceSavedStateStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_state_updated_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_endpoint: Option<SliceDisplayEndpoint>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SliceDevelopmentPublication {
    pub publication_id: String,
    pub destination_root: String,
    pub primary_repository_path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repository_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SliceSavedStateStatus {
    Saved,
    Missing,
    Failed,
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
    Selkies,
    CharioxViewer,
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SliceDisplayBackend {
    #[default]
    Novnc,
    Selkies,
}

impl SliceDisplayBackend {
    pub fn is_novnc(&self) -> bool {
        *self == Self::Novnc
    }

    pub fn as_env_value(self) -> &'static str {
        match self {
            Self::Novnc => "novnc",
            Self::Selkies => "selkies",
        }
    }
}

impl SliceRecord {
    pub fn display_backend(&self) -> SliceDisplayBackend {
        match self
            .display_endpoint
            .as_ref()
            .map(|endpoint| &endpoint.kind)
        {
            Some(SliceDisplayEndpointKind::Selkies) => SliceDisplayBackend::Selkies,
            _ => SliceDisplayBackend::Novnc,
        }
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_protocol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_public_key: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceSavedStateRecord {
    pub id: String,
    pub slice_name: String,
    pub source_slice_id: String,
    pub backend: SliceBackendKind,
    pub os: String,
    pub image_ref: String,
    pub home_archive_path: String,
    pub manifest_path: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_operation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_operation_status: Option<SliceOperationStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceBackupRecord {
    pub id: String,
    pub name: String,
    pub source_slice_id: String,
    pub source_state_id: String,
    pub image_ref: String,
    pub home_archive_path: String,
    pub manifest_path: String,
    pub created_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_archive_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CreateSliceInput {
    pub name: String,
    pub backend: SliceBackendKind,
    pub os: String,
    pub display_mode: SliceDisplayMode,
    pub display_backend: SliceDisplayBackend,
    pub workspace_id: Option<String>,
    pub worktree_id: Option<String>,
    pub workspace_mount: Option<String>,
    pub development: Option<crate::managed_context::package::ManagedContextDevelopmentSelection>,
    pub worker_kernel_ref: Option<String>,
    pub display_url: Option<String>,
    pub provider_auth: Vec<SliceProviderAuthSummary>,
    pub from_saved_state: Option<SliceSavedStateRecord>,
    pub now_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalDockerSliceAction {
    Provision,
    RestoreState,
    Recover,
    ImportProviderAuth,
    RemoveProviderAuth,
    Stop,
    Destroy,
}
