use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::mcp::ArrobaMcpServerConfig;
use crate::session::{unix_epoch_ms, DEFAULT_LOCAL_USER_ID};
use crate::terminal::TerminalOutputKind;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCommandDescriptor {
    pub id: String,
    pub name: String,
    pub description: String,
    pub value: String,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCommandCatalogSource {
    Shipped,
    Discovered,
    Merged,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCommandCatalogDiscovery {
    None,
    ProviderApi,
    CustomFiles,
    Driver,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCommandCatalog {
    pub provider: String,
    pub source: ProviderCommandCatalogSource,
    pub discovery: ProviderCommandCatalogDiscovery,
    #[serde(default)]
    pub commands: Vec<ProviderCommandDescriptor>,
}

pub fn default_provider_command_catalogs() -> BTreeMap<String, ProviderCommandCatalog> {
    BTreeMap::from([
        (
            "opencode".to_string(),
            ProviderCommandCatalog {
                provider: "opencode".to_string(),
                source: ProviderCommandCatalogSource::Shipped,
                discovery: ProviderCommandCatalogDiscovery::None,
                commands: Vec::new(),
            },
        ),
        (
            "claude".to_string(),
            ProviderCommandCatalog {
                provider: "claude".to_string(),
                source: ProviderCommandCatalogSource::Shipped,
                discovery: ProviderCommandCatalogDiscovery::None,
                commands: Vec::new(),
            },
        ),
        (
            "codex".to_string(),
            ProviderCommandCatalog {
                provider: "codex".to_string(),
                source: ProviderCommandCatalogSource::Shipped,
                discovery: ProviderCommandCatalogDiscovery::None,
                commands: Vec::new(),
            },
        ),
    ])
}

pub(crate) fn provider_requires_managed_io_by_default(
    provider: &str,
    config: &crate::config::DaemonConfig,
) -> bool {
    config.provider_requires_managed_io(provider)
}

fn default_provider_owner_user_id() -> String {
    DEFAULT_LOCAL_USER_ID.to_string()
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlOperation {
    InterruptTurn,
    CancelPrompt,
    AckWorkflowTurn,
    ValidateWorkflowOutput,
    AttachFile,
    RequestMemoryUpdate,
    RequestCompactionSummary,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlCapabilityMode {
    Native,
    Mcp,
    AdapterEmulated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlCapability {
    operation: ControlOperation,
    mode: ControlCapabilityMode,
}

impl ControlCapability {
    pub fn new(operation: ControlOperation, mode: ControlCapabilityMode) -> Self {
        Self { operation, mode }
    }

    pub fn operation(&self) -> ControlOperation {
        self.operation
    }

    pub fn mode(&self) -> ControlCapabilityMode {
        self.mode
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderRunState {
    Starting,
    Running,
    Parked,
    Ended,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentEndpointMode {
    Managed,
    External,
}

impl fmt::Display for AgentEndpointMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Managed => "managed",
            Self::External => "external",
        };

        write!(f, "{value}")
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderClientInterface {
    #[default]
    Arroba,
    NativeTui,
}

impl ProviderClientInterface {
    pub fn is_arroba(&self) -> bool {
        matches!(self, Self::Arroba)
    }
}

impl fmt::Display for ProviderClientInterface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Arroba => "arroba",
            Self::NativeTui => "native_tui",
        };
        write!(f, "{value}")
    }
}

impl fmt::Display for ProviderRunState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Parked => "parked",
            Self::Ended => "ended",
        };

        write!(f, "{value}")
    }
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
    ManagedIoRequired,
}

impl ProviderWriteAccessMode {
    pub fn is_unrestricted(&self) -> bool {
        matches!(self, Self::Unrestricted)
    }

    pub fn requires_managed_io(&self) -> bool {
        matches!(self, Self::ManagedIoRequired)
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

    pub fn with_managed_io_required(mut self) -> Self {
        self.write_access_mode = ProviderWriteAccessMode::ManagedIoRequired;
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

    pub fn requires_managed_io(&self) -> bool {
        self.write_access_mode.requires_managed_io()
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeProviderRun {
    id: String,
    session_id: String,
    agent_instance_id: Option<String>,
    #[serde(default = "default_provider_owner_user_id")]
    owner_user_id: String,
    adapter_key: String,
    provider: String,
    account_profile: String,
    model: String,
    variant: Option<String>,
    usage_tokens_total: Option<u64>,
    #[serde(default, skip_serializing_if = "ProviderRunTokenUsage::is_empty")]
    usage: ProviderRunTokenUsage,
    state: ProviderRunState,
    endpoint_mode: AgentEndpointMode,
    #[serde(default, skip_serializing_if = "ProviderClientInterface::is_arroba")]
    client_interface: ProviderClientInterface,
    process_label: String,
    pty_target: Option<String>,
    pty_program: Option<String>,
    pty_args: Vec<String>,
    pty_env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pty_env_remove: Vec<String>,
    working_directory: Option<PathBuf>,
    structured_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    runtime_mcp_server_url: Option<String>,
    runtime_mcp_auth_token: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    mcp_servers: Vec<ArrobaMcpServerConfig>,
    #[serde(
        default,
        skip_serializing_if = "ProviderWriteAccessMode::is_unrestricted"
    )]
    write_access_mode: ProviderWriteAccessMode,
    #[serde(default, skip_serializing_if = "AgentExecutionMode::is_build")]
    execution_mode: AgentExecutionMode,
    #[serde(default, skip_serializing_if = "AgentPermissionLevel::is_yolo")]
    permission_level: AgentPermissionLevel,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    control_capabilities: Vec<ControlCapability>,
    #[serde(default, skip_serializing_if = "ProviderResumeState::is_empty")]
    resume_state: ProviderResumeState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal_diagnostic: Option<String>,
    started_at_ms: u64,
    last_activity_at_ms: u64,
}

impl RuntimeProviderRun {
    pub fn new(
        id: impl Into<String>,
        request: &LaunchProviderRequest,
        launch_result: ProviderLaunchResult,
    ) -> Self {
        let now = unix_epoch_ms();
        Self {
            id: id.into(),
            session_id: request.session_id.clone(),
            agent_instance_id: request.agent_id.clone(),
            owner_user_id: request.owner_user_id.clone(),
            adapter_key: request.adapter_key.clone(),
            provider: request.provider.clone(),
            account_profile: request.account_profile.clone(),
            model: request.model.clone(),
            variant: request.variant.clone(),
            usage_tokens_total: None,
            usage: ProviderRunTokenUsage::default(),
            state: ProviderRunState::Starting,
            endpoint_mode: launch_result.endpoint_mode,
            client_interface: request.client_interface,
            process_label: launch_result.process_label,
            pty_target: launch_result.pty_target,
            pty_program: launch_result.pty_program,
            pty_args: launch_result.pty_args,
            pty_env: launch_result.pty_env,
            pty_env_remove: launch_result.pty_env_remove,
            working_directory: launch_result.working_directory,
            structured_endpoint: launch_result.structured_endpoint,
            runtime_mcp_server_url: request
                .runtime_mcp_binding
                .as_ref()
                .map(|binding| binding.server_url.clone()),
            runtime_mcp_auth_token: request
                .runtime_mcp_binding
                .as_ref()
                .map(|binding| binding.auth_token.clone()),
            mcp_servers: request.mcp_servers.clone(),
            write_access_mode: request.write_access_mode,
            execution_mode: request.execution_mode.unwrap_or_default(),
            permission_level: request.permission_level.unwrap_or_default(),
            control_capabilities: default_control_capabilities(
                &request.adapter_key,
                launch_result.endpoint_mode,
                request.runtime_mcp_binding.is_some(),
            ),
            resume_state: request.resume_state.clone().unwrap_or_default(),
            provider_session_id: request
                .resume_state
                .as_ref()
                .and_then(|state| {
                    state
                        .opencode_session_id()
                        .or_else(|| state.codex_thread_id())
                        .or_else(|| state.claude_session_id())
                })
                .map(str::to_string),
            terminal_diagnostic: None,
            started_at_ms: now,
            last_activity_at_ms: now,
        }
    }

    pub fn from_control_capability_inference(
        id: impl Into<String>,
        session_id: String,
        agent_instance_id: Option<String>,
        adapter_key: String,
    ) -> Self {
        let inferred_has_runtime_mcp_binding =
            matches!(adapter_key.as_str(), "claude" | "codex" | "opencode");
        let now = unix_epoch_ms();
        Self {
            id: id.into(),
            session_id,
            agent_instance_id,
            owner_user_id: default_provider_owner_user_id(),
            adapter_key: adapter_key.clone(),
            provider: adapter_key.clone(),
            account_profile: "default".to_string(),
            model: String::new(),
            variant: None,
            usage_tokens_total: None,
            usage: ProviderRunTokenUsage::default(),
            state: ProviderRunState::Starting,
            endpoint_mode: AgentEndpointMode::Managed,
            client_interface: ProviderClientInterface::Arroba,
            process_label: "inferred-control-capabilities".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
            runtime_mcp_server_url: None,
            runtime_mcp_auth_token: inferred_has_runtime_mcp_binding
                .then(|| "inferred-managed-mcp".to_string()),
            mcp_servers: Vec::new(),
            write_access_mode: ProviderWriteAccessMode::Unrestricted,
            execution_mode: AgentExecutionMode::default(),
            permission_level: AgentPermissionLevel::default(),
            control_capabilities: default_control_capabilities(
                &adapter_key,
                AgentEndpointMode::Managed,
                inferred_has_runtime_mcp_binding,
            ),
            resume_state: ProviderResumeState::default(),
            provider_session_id: None,
            terminal_diagnostic: None,
            started_at_ms: now,
            last_activity_at_ms: now,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    pub fn agent_instance_id(&self) -> Option<&str> {
        self.agent_instance_id.as_deref()
    }
    pub fn owner_user_id(&self) -> &str {
        &self.owner_user_id
    }
    pub fn adapter_key(&self) -> &str {
        &self.adapter_key
    }
    pub fn provider(&self) -> &str {
        &self.provider
    }
    pub fn account_profile(&self) -> &str {
        &self.account_profile
    }
    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn variant(&self) -> Option<&str> {
        self.variant.as_deref()
    }

    pub fn control_capabilities(&self) -> &[ControlCapability] {
        &self.control_capabilities
    }

    pub fn supports_control_operation(&self, operation: ControlOperation) -> bool {
        self.control_capabilities
            .iter()
            .any(|capability| capability.operation() == operation)
    }

    pub fn set_model(&mut self, model: impl Into<String>) {
        self.model = model.into();
    }

    pub fn set_variant(&mut self, variant: Option<String>) {
        self.variant = variant.and_then(|value| {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_string())
        });
    }

    pub fn usage_tokens_total(&self) -> Option<u64> {
        self.usage_tokens_total
    }

    pub fn usage(&self) -> ProviderRunTokenUsage {
        self.usage
    }

    pub fn set_usage_tokens_total(&mut self, usage_tokens_total: Option<u64>) {
        self.usage_tokens_total = usage_tokens_total;
        self.usage.total_tokens = usage_tokens_total;
    }

    pub fn set_usage(&mut self, usage: ProviderRunTokenUsage) {
        self.usage = usage;
        self.usage_tokens_total = usage.total_tokens;
    }

    pub fn state(&self) -> ProviderRunState {
        self.state
    }
    pub fn endpoint_mode(&self) -> AgentEndpointMode {
        self.endpoint_mode
    }
    pub fn client_interface(&self) -> ProviderClientInterface {
        self.client_interface
    }
    pub fn process_label(&self) -> &str {
        &self.process_label
    }
    pub fn pty_target(&self) -> Option<&str> {
        self.pty_target.as_deref()
    }
    pub fn pty_program(&self) -> Option<&str> {
        self.pty_program.as_deref()
    }
    pub fn pty_args(&self) -> &[String] {
        &self.pty_args
    }
    pub fn pty_env(&self) -> &BTreeMap<String, String> {
        &self.pty_env
    }

    pub fn pty_env_remove(&self) -> &[String] {
        &self.pty_env_remove
    }
    pub fn working_directory(&self) -> Option<&PathBuf> {
        self.working_directory.as_ref()
    }
    pub fn structured_endpoint(&self) -> Option<&str> {
        self.structured_endpoint.as_deref()
    }
    pub fn runtime_mcp_auth_token(&self) -> Option<&str> {
        self.runtime_mcp_auth_token.as_deref()
    }

    pub fn runtime_mcp_server_url(&self) -> Option<&str> {
        self.runtime_mcp_server_url.as_deref()
    }

    pub fn mcp_servers(&self) -> &[ArrobaMcpServerConfig] {
        &self.mcp_servers
    }

    pub fn write_access_mode(&self) -> ProviderWriteAccessMode {
        self.write_access_mode
    }

    pub fn execution_mode(&self) -> AgentExecutionMode {
        self.execution_mode
    }

    pub fn permission_level(&self) -> AgentPermissionLevel {
        self.permission_level
    }

    pub fn set_execution_config(
        &mut self,
        execution_mode: AgentExecutionMode,
        permission_level: AgentPermissionLevel,
    ) {
        self.execution_mode = execution_mode;
        self.permission_level = permission_level;
        self.touch_activity();
    }

    pub fn requires_managed_io(&self) -> bool {
        self.write_access_mode.requires_managed_io()
    }

    pub fn resume_state(&self) -> &ProviderResumeState {
        &self.resume_state
    }

    pub fn set_resume_state(&mut self, resume_state: ProviderResumeState) {
        self.resume_state = resume_state;
    }

    pub fn provider_session_id(&self) -> Option<&str> {
        self.provider_session_id.as_deref()
    }

    pub fn set_provider_session_id(&mut self, provider_session_id: Option<String>) {
        self.provider_session_id = provider_session_id;
    }

    pub fn terminal_diagnostic(&self) -> Option<&str> {
        self.terminal_diagnostic.as_deref()
    }

    pub fn set_terminal_diagnostic(&mut self, diagnostic: impl Into<String>) {
        let diagnostic = diagnostic.into();
        if !diagnostic.trim().is_empty() {
            self.terminal_diagnostic = Some(diagnostic);
        }
    }

    pub fn set_runtime_mcp_auth_token(&mut self, auth_token: Option<String>) {
        self.runtime_mcp_auth_token = auth_token;
    }

    pub fn set_control_capabilities(&mut self, capabilities: Vec<ControlCapability>) {
        self.control_capabilities = capabilities;
    }

    pub fn started_at_ms(&self) -> u64 {
        self.started_at_ms
    }

    pub fn last_activity_at_ms(&self) -> u64 {
        self.last_activity_at_ms
    }

    pub fn touch_activity(&mut self) {
        self.last_activity_at_ms = unix_epoch_ms();
    }

    pub fn mark_running(&mut self) {
        self.state = ProviderRunState::Running;
    }

    pub fn mark_parked(&mut self) {
        self.state = ProviderRunState::Parked;
    }

    pub fn mark_ended(&mut self) {
        self.state = ProviderRunState::Ended;
    }

    pub fn owned_by(&self, user_id: &str) -> bool {
        self.owner_user_id == user_id
    }

    pub(crate) fn projected_for_home_agent(
        mut self,
        session_id: impl Into<String>,
        agent_id: impl Into<String>,
    ) -> Self {
        self.session_id = session_id.into();
        self.agent_instance_id = Some(agent_id.into());
        self
    }
}

fn default_control_capabilities(
    adapter_key: &str,
    _endpoint_mode: AgentEndpointMode,
    has_runtime_mcp_binding: bool,
) -> Vec<ControlCapability> {
    let mut capabilities = Vec::new();

    if matches!(adapter_key, "claude" | "codex" | "opencode") {
        capabilities.push(ControlCapability::new(
            ControlOperation::InterruptTurn,
            ControlCapabilityMode::Native,
        ));
        capabilities.push(ControlCapability::new(
            ControlOperation::CancelPrompt,
            ControlCapabilityMode::Native,
        ));
    }

    if adapter_key == "dev-stub" {
        capabilities.push(ControlCapability::new(
            ControlOperation::AckWorkflowTurn,
            ControlCapabilityMode::AdapterEmulated,
        ));
        capabilities.push(ControlCapability::new(
            ControlOperation::ValidateWorkflowOutput,
            ControlCapabilityMode::AdapterEmulated,
        ));
    } else if has_runtime_mcp_binding {
        capabilities.push(ControlCapability::new(
            ControlOperation::AckWorkflowTurn,
            ControlCapabilityMode::Mcp,
        ));
        capabilities.push(ControlCapability::new(
            ControlOperation::ValidateWorkflowOutput,
            ControlCapabilityMode::Mcp,
        ));
    }

    capabilities
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRunTokenUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
}

impl ProviderRunTokenUsage {
    pub fn from_total_tokens(total_tokens: u64) -> Self {
        Self {
            total_tokens: Some(total_tokens),
            last_tokens: None,
            context_tokens: None,
            context_window: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.total_tokens.is_none()
            && self.last_tokens.is_none()
            && self.context_tokens.is_none()
            && self.context_window.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderPromptChunk {
    pub kind: TerminalOutputKind,
    pub merge_key: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAssistantCompletion {
    pub message_id: String,
    pub completed_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderPromptSignalBatch {
    pub chunks: Vec<ProviderPromptChunk>,
    pub completions: Vec<ProviderAssistantCompletion>,
    pub prompt_completed: bool,
    pub terminal_failure: Option<String>,
    pub notices: Vec<String>,
    pub resolved_model: Option<String>,
    pub resolved_model_source: Option<&'static str>,
    pub resolved_variant: Option<String>,
    pub resolved_usage_tokens_total: Option<u64>,
    pub resolved_usage: Option<ProviderRunTokenUsage>,
    pub resolved_resume_state: Option<ProviderResumeState>,
}

pub(crate) fn classify_provider_terminal_failure_text(
    adapter_key: &str,
    text: &str,
) -> Option<String> {
    if !matches!(adapter_key, "codex" | "opencode") {
        return None;
    }
    if let Some(failure) = classify_provider_substitutable_failure_text(adapter_key, text) {
        return Some(failure);
    }
    let normalized = text.to_lowercase();
    let fatal_model_error = normalized.contains("unsupported model")
        || normalized.contains("invalid model")
        || normalized.contains("model_not_found")
        || normalized.contains("model not found")
        || (normalized.contains("model") && normalized.contains("does not exist"))
        || (normalized.contains("model") && normalized.contains("not supported"))
        || (normalized.contains("model")
            && (normalized.contains("http 400")
                || normalized.contains("status 400")
                || normalized.contains("400 bad request")));
    if !fatal_model_error {
        return None;
    }
    Some(format!(
        "Provider reported a terminal model error: {}",
        compact_provider_error_snippet(text)
    ))
}

pub(crate) fn classify_provider_substitutable_failure_text(
    adapter_key: &str,
    text: &str,
) -> Option<String> {
    if !matches!(adapter_key, "codex" | "opencode") {
        return None;
    }
    let normalized = text.to_lowercase();
    let quota_or_billing = normalized.contains("insufficient_quota")
        || normalized.contains("quota exceeded")
        || normalized.contains("exceeded your current quota")
        || normalized.contains("billing hard limit")
        || normalized.contains("billing limit")
        || normalized.contains("spend limit")
        || normalized.contains("usage limit")
        || normalized.contains("monthly limit")
        || normalized.contains("no credits")
        || normalized.contains("not enough credits")
        || normalized.contains("credits exhausted")
        || normalized.contains("credit balance")
        || normalized.contains("out of credits");
    let rate_or_run_limit = normalized.contains("rate_limit_exceeded")
        || normalized.contains("rate limit exceeded")
        || normalized.contains("rate limited")
        || normalized.contains("too many requests")
        || normalized.contains("http 429")
        || normalized.contains("status 429")
        || normalized.contains("429 too many requests")
        || normalized.contains("run limit")
        || normalized.contains("runs limit")
        || normalized.contains("turn limit");
    if !(quota_or_billing || rate_or_run_limit) {
        return None;
    }
    Some(format!(
        "Provider reported a substitutable resource limit: {}",
        compact_provider_error_snippet(text)
    ))
}

fn compact_provider_error_snippet(text: &str) -> String {
    let mut snippet = text.split_whitespace().collect::<Vec<_>>().join(" ");
    const MAX_CHARS: usize = 500;
    if snippet.chars().count() > MAX_CHARS {
        snippet = snippet.chars().take(MAX_CHARS).collect::<String>();
        snippet.push_str("...");
    }
    snippet
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderProcessStatus {
    Active,
    Idle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderProcessInfo {
    pub process_id: String,
    pub provider: String,
    pub process_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    pub endpoint_mode: AgentEndpointMode,
    pub status: ProviderProcessStatus,
    pub started_at_ms: u64,
    pub last_activity_at_ms: u64,
    #[serde(default)]
    pub provider_session_ids: Vec<String>,
    #[serde(default)]
    pub owner_session_ids: Vec<String>,
    #[serde(default)]
    pub owner_provider_run_ids: Vec<String>,
    #[serde(default)]
    pub attached_session_ids: Vec<String>,
    #[serde(default)]
    pub active_workflow_run_ids: Vec<String>,
    pub teardown_safe: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub teardown_blockers: Vec<String>,
}

impl ProviderProcessInfo {
    pub fn from_runs(
        process_id: String,
        runs: &[RuntimeProviderRun],
        attached_session_ids: BTreeSet<String>,
        active_workflow_run_ids: BTreeSet<String>,
        teardown_safe: bool,
        teardown_blockers: Vec<String>,
    ) -> Option<Self> {
        let first = runs
            .iter()
            .find(|run| run.endpoint_mode() == AgentEndpointMode::Managed)
            .or_else(|| runs.first())?;
        let status = if runs.iter().any(|run| {
            matches!(
                run.state(),
                ProviderRunState::Starting | ProviderRunState::Running
            )
        }) {
            ProviderProcessStatus::Active
        } else {
            ProviderProcessStatus::Idle
        };
        let started_at_ms = runs
            .iter()
            .map(RuntimeProviderRun::started_at_ms)
            .min()
            .unwrap_or_else(unix_epoch_ms);
        let last_activity_at_ms = runs
            .iter()
            .map(RuntimeProviderRun::last_activity_at_ms)
            .max()
            .unwrap_or_else(unix_epoch_ms);
        let provider_session_ids = runs
            .iter()
            .filter_map(|run| {
                run.provider_session_id().map(str::to_string).or_else(|| {
                    run.resume_state()
                        .opencode_session_id()
                        .or_else(|| run.resume_state().codex_thread_id())
                        .or_else(|| run.resume_state().claude_session_id())
                        .map(str::to_string)
                })
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let owner_session_ids = runs
            .iter()
            .map(|run| run.session_id().to_string())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let owner_provider_run_ids = runs
            .iter()
            .map(|run| run.id().to_string())
            .collect::<Vec<_>>();
        Some(Self {
            process_id,
            provider: first.provider().to_string(),
            process_label: first.process_label().to_string(),
            pid: None,
            endpoint_mode: first.endpoint_mode(),
            status,
            started_at_ms,
            last_activity_at_ms,
            provider_session_ids,
            owner_session_ids,
            owner_provider_run_ids,
            attached_session_ids: attached_session_ids.into_iter().collect(),
            active_workflow_run_ids: active_workflow_run_ids.into_iter().collect(),
            teardown_safe,
            teardown_blockers,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        classify_provider_substitutable_failure_text, classify_provider_terminal_failure_text,
        AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, ProviderProcessInfo,
        ProviderResumeState, RuntimeProviderRun,
    };
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn runtime_provider_run_initializes_explicit_provider_session_id_from_resume_state() {
        let request =
            LaunchProviderRequest::new("session-1", "opencode", "opencode", "default", "default")
                .with_resume_state(ProviderResumeState::from_opencode_session_id(
                    "open-session-1",
                ));
        let launch_result = ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "opencode".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        };
        let run = RuntimeProviderRun::new("provider-run-1", &request, launch_result);
        assert_eq!(run.provider_session_id(), Some("open-session-1"));
    }

    #[test]
    fn launch_request_tracks_required_managed_io_mode() {
        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "default")
                .with_managed_io_required();

        assert!(request.requires_managed_io());
        let json = serde_json::to_value(&request).expect("request should serialize");
        assert_eq!(json["write_access_mode"], "managed_io_required");
    }

    #[test]
    fn provider_process_info_prefers_explicit_provider_session_ids() {
        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "default");
        let launch_result = ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "codex".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        };
        let mut run = RuntimeProviderRun::new("provider-run-1", &request, launch_result);
        run.set_provider_session_id(Some("thread-123".to_string()));
        run.mark_running();
        let info = ProviderProcessInfo::from_runs(
            "process-1".to_string(),
            &[run],
            BTreeSet::new(),
            BTreeSet::new(),
            true,
            Vec::new(),
        )
        .expect("process info should be built");
        assert_eq!(info.status, super::ProviderProcessStatus::Active);
        assert_eq!(info.provider_session_ids, vec!["thread-123".to_string()]);
    }

    #[test]
    fn classifier_detects_provider_model_rejection_text() {
        let failure = classify_provider_terminal_failure_text(
            "codex",
            "Error: HTTP 400 Bad Request: unsupported model gpt-5.2-codex",
        )
        .expect("model rejection text should be classified");

        assert!(failure.contains("terminal model error"));
        assert!(failure.contains("gpt-5.2-codex"));
    }

    #[test]
    fn classifier_ignores_non_provider_text() {
        assert!(classify_provider_terminal_failure_text(
            "dev-stub",
            "unsupported model gpt-5.2-codex"
        )
        .is_none());
        assert!(
            classify_provider_terminal_failure_text("codex", "normal assistant output").is_none()
        );
    }

    #[test]
    fn substitute_classifier_detects_shared_quota_and_limit_errors() {
        let codex_failure = classify_provider_substitutable_failure_text(
            "codex",
            "Error: insufficient_quota: You exceeded your current quota.",
        )
        .expect("codex quota error should be substitutable");
        assert!(codex_failure.contains("substitutable resource limit"));

        let opencode_failure = classify_provider_substitutable_failure_text(
            "opencode",
            "OpenCode error: No credits available for this account",
        )
        .expect("opencode credit error should be substitutable");
        assert!(opencode_failure.contains("No credits"));
    }

    #[test]
    fn substitute_classifier_ignores_model_auth_and_network_errors() {
        assert!(classify_provider_substitutable_failure_text(
            "codex",
            "HTTP 400 Bad Request: unsupported model gpt-5.2-codex"
        )
        .is_none());
        assert!(classify_provider_substitutable_failure_text(
            "opencode",
            "Authentication required. Please login."
        )
        .is_none());
        assert!(
            classify_provider_substitutable_failure_text("codex", "connection refused").is_none()
        );
    }
}
