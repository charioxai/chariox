use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::mcp::ArrobaMcpServerConfig;
use crate::session::DEFAULT_LOCAL_USER_ID;

use super::types::{AgentEndpointMode, ProviderClientInterface};

pub(super) fn default_provider_owner_user_id() -> String {
    DEFAULT_LOCAL_USER_ID.to_string()
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderResumeState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    opencode_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    codex_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    claude_session_id: Option<String>,
}

impl ProviderResumeState {
    pub fn is_empty(&self) -> bool {
        self.opencode_session_id.is_none()
            && self.codex_thread_id.is_none()
            && self.claude_session_id.is_none()
    }

    pub fn from_opencode_session_id(session_id: impl Into<String>) -> Self {
        let mut state = Self::default();
        state.set_opencode_session_id(session_id);
        state
    }

    pub fn from_codex_thread_id(thread_id: impl Into<String>) -> Self {
        let mut state = Self::default();
        state.set_codex_thread_id(thread_id);
        state
    }

    pub fn from_claude_session_id(session_id: impl Into<String>) -> Self {
        let mut state = Self::default();
        state.set_claude_session_id(session_id);
        state
    }

    pub fn opencode_session_id(&self) -> Option<&str> {
        self.opencode_session_id.as_deref()
    }

    pub fn codex_thread_id(&self) -> Option<&str> {
        self.codex_thread_id.as_deref()
    }

    pub fn claude_session_id(&self) -> Option<&str> {
        self.claude_session_id.as_deref()
    }

    pub fn set_opencode_session_id(&mut self, session_id: impl Into<String>) {
        self.opencode_session_id = Some(session_id.into());
    }

    pub fn set_codex_thread_id(&mut self, thread_id: impl Into<String>) {
        self.codex_thread_id = Some(thread_id.into());
    }

    pub fn set_claude_session_id(&mut self, session_id: impl Into<String>) {
        self.claude_session_id = Some(session_id.into());
    }

    pub fn without_opencode_session_id(&self) -> Self {
        Self {
            opencode_session_id: None,
            codex_thread_id: self.codex_thread_id.clone(),
            claude_session_id: self.claude_session_id.clone(),
        }
    }

    pub fn without_codex_thread_id(&self) -> Self {
        Self {
            opencode_session_id: self.opencode_session_id.clone(),
            codex_thread_id: None,
            claude_session_id: self.claude_session_id.clone(),
        }
    }

    pub fn without_claude_session_id(&self) -> Self {
        Self {
            opencode_session_id: self.opencode_session_id.clone(),
            codex_thread_id: self.codex_thread_id.clone(),
            claude_session_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchProviderRequest {
    pub session_id: String,
    pub agent_id: Option<String>,
    #[serde(default = "default_provider_owner_user_id")]
    pub owner_user_id: String,
    pub adapter_key: String,
    pub provider: String,
    pub account_profile: String,
    pub model: String,
    pub variant: Option<String>,
    pub working_directory: Option<PathBuf>,
    pub runtime_mcp_binding: Option<RuntimeMcpBinding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<ArrobaMcpServerConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_env_remove: Vec<String>,
    #[serde(
        default,
        skip_serializing_if = "ProviderWriteAccessMode::is_unrestricted"
    )]
    pub write_access_mode: ProviderWriteAccessMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<AgentExecutionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_level: Option<AgentPermissionLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_state: Option<ProviderResumeState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "ProviderClientInterface::is_arroba")]
    pub client_interface: ProviderClientInterface,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentExecutionMode {
    #[default]
    Build,
    Plan,
}

impl AgentExecutionMode {
    pub fn is_build(&self) -> bool {
        matches!(self, Self::Build)
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "build" => Some(Self::Build),
            "plan" => Some(Self::Plan),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Plan => "plan",
        }
    }
}

impl fmt::Display for AgentExecutionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentPermissionLevel {
    Required,
    #[default]
    Yolo,
}

impl AgentPermissionLevel {
    pub fn is_yolo(&self) -> bool {
        matches!(self, Self::Yolo)
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "required" => Some(Self::Required),
            "yolo" => Some(Self::Yolo),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::Yolo => "yolo",
        }
    }
}

impl fmt::Display for AgentPermissionLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderWriteAccessMode {
    #[default]
    Unrestricted,
    WorkspaceLiveSyncRequired,
}

impl ProviderWriteAccessMode {
    pub fn is_unrestricted(&self) -> bool {
        matches!(self, Self::Unrestricted)
    }

    pub fn requires_workspace_live_sync(&self) -> bool {
        matches!(self, Self::WorkspaceLiveSyncRequired)
    }
}

impl LaunchProviderRequest {
    pub fn new(
        session_id: impl Into<String>,
        adapter_key: impl Into<String>,
        provider: impl Into<String>,
        account_profile: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            agent_id: None,
            owner_user_id: default_provider_owner_user_id(),
            adapter_key: adapter_key.into(),
            provider: provider.into(),
            account_profile: account_profile.into(),
            model: model.into(),
            variant: None,
            working_directory: None,
            runtime_mcp_binding: None,
            mcp_servers: Vec::new(),
            provider_env_remove: Vec::new(),
            write_access_mode: ProviderWriteAccessMode::Unrestricted,
            execution_mode: None,
            permission_level: None,
            resume_state: None,
            structured_endpoint: None,
            client_interface: ProviderClientInterface::Arroba,
        }
    }

    pub fn with_variant(mut self, variant: Option<String>) -> Self {
        self.variant = variant.and_then(|value| {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_string())
        });
        self
    }

    pub fn with_agent_id(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = Some(agent_id.into());
        self
    }

    pub fn with_owner_user_id(mut self, owner_user_id: impl Into<String>) -> Self {
        self.owner_user_id = owner_user_id.into();
        self
    }

    pub fn with_working_directory(mut self, working_directory: PathBuf) -> Self {
        self.working_directory = Some(working_directory);
        self
    }

    pub fn with_runtime_mcp_binding(mut self, binding: RuntimeMcpBinding) -> Self {
        self.runtime_mcp_binding = Some(binding);
        self
    }

    pub fn with_mcp_servers(mut self, mcp_servers: Vec<ArrobaMcpServerConfig>) -> Self {
        self.mcp_servers = mcp_servers;
        self
    }

    pub fn with_provider_env_remove(mut self, env_remove: Vec<String>) -> Self {
        self.provider_env_remove = env_remove;
        self
    }

    pub fn with_workspace_live_sync_required(mut self) -> Self {
        self.write_access_mode = ProviderWriteAccessMode::WorkspaceLiveSyncRequired;
        self
    }

    pub fn with_execution_mode(mut self, execution_mode: AgentExecutionMode) -> Self {
        self.execution_mode = Some(execution_mode);
        self
    }

    pub fn with_permission_level(mut self, permission_level: AgentPermissionLevel) -> Self {
        self.permission_level = Some(permission_level);
        self
    }

    pub fn requires_workspace_live_sync(&self) -> bool {
        self.write_access_mode.requires_workspace_live_sync()
    }

    pub fn with_resume_state(mut self, resume_state: ProviderResumeState) -> Self {
        self.resume_state = (!resume_state.is_empty()).then_some(resume_state);
        self
    }

    pub fn with_structured_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.structured_endpoint = Some(endpoint.into());
        self
    }

    pub fn with_client_interface(mut self, client_interface: ProviderClientInterface) -> Self {
        self.client_interface = client_interface;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeMcpBinding {
    pub server_url: String,
    pub auth_token: String,
}

impl RuntimeMcpBinding {
    pub fn new(server_url: impl Into<String>, auth_token: impl Into<String>) -> Self {
        Self {
            server_url: server_url.into(),
            auth_token: auth_token.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderLaunchResult {
    pub endpoint_mode: AgentEndpointMode,
    pub process_label: String,
    pub pty_target: Option<String>,
    pub pty_program: Option<String>,
    pub pty_args: Vec<String>,
    pub pty_env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pty_env_remove: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub structured_endpoint: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::LaunchProviderRequest;

    #[test]
    fn launch_request_tracks_required_workspace_live_sync_mode() {
        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "default")
                .with_workspace_live_sync_required();

        assert!(request.requires_workspace_live_sync());
        let json = serde_json::to_value(&request).expect("request should serialize");
        assert_eq!(json["write_access_mode"], "workspace_live_sync_required");
    }
}
