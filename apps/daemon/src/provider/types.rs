use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

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
}

impl ProviderResumeState {
    pub fn is_empty(&self) -> bool {
        self.opencode_session_id.is_none() && self.codex_thread_id.is_none()
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

    pub fn opencode_session_id(&self) -> Option<&str> {
        self.opencode_session_id.as_deref()
    }

    pub fn codex_thread_id(&self) -> Option<&str> {
        self.codex_thread_id.as_deref()
    }

    pub fn set_opencode_session_id(&mut self, session_id: impl Into<String>) {
        self.opencode_session_id = Some(session_id.into());
    }

    pub fn set_codex_thread_id(&mut self, thread_id: impl Into<String>) {
        self.codex_thread_id = Some(thread_id.into());
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchProviderRequest {
    pub session_id: String,
    pub agent_id: Option<String>,
    pub adapter_key: String,
    pub provider: String,
    pub account_profile: String,
    pub model: String,
    pub variant: Option<String>,
    pub working_directory: Option<PathBuf>,
    pub runtime_mcp_binding: Option<RuntimeMcpBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_state: Option<ProviderResumeState>,
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
            adapter_key: adapter_key.into(),
            provider: provider.into(),
            account_profile: account_profile.into(),
            model: model.into(),
            variant: None,
            working_directory: None,
            runtime_mcp_binding: None,
            resume_state: None,
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

    pub fn with_working_directory(mut self, working_directory: PathBuf) -> Self {
        self.working_directory = Some(working_directory);
        self
    }

    pub fn with_runtime_mcp_binding(mut self, binding: RuntimeMcpBinding) -> Self {
        self.runtime_mcp_binding = Some(binding);
        self
    }

    pub fn with_resume_state(mut self, resume_state: ProviderResumeState) -> Self {
        self.resume_state = (!resume_state.is_empty()).then_some(resume_state);
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
    pub working_directory: Option<PathBuf>,
    pub structured_endpoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeProviderRun {
    id: String,
    session_id: String,
    agent_instance_id: Option<String>,
    adapter_key: String,
    provider: String,
    account_profile: String,
    model: String,
    variant: Option<String>,
    usage_tokens_total: Option<u64>,
    state: ProviderRunState,
    endpoint_mode: AgentEndpointMode,
    process_label: String,
    pty_target: Option<String>,
    pty_program: Option<String>,
    pty_args: Vec<String>,
    pty_env: BTreeMap<String, String>,
    working_directory: Option<PathBuf>,
    structured_endpoint: Option<String>,
    runtime_mcp_auth_token: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    control_capabilities: Vec<ControlCapability>,
    #[serde(default, skip_serializing_if = "ProviderResumeState::is_empty")]
    resume_state: ProviderResumeState,
}

impl RuntimeProviderRun {
    pub fn new(
        id: impl Into<String>,
        request: &LaunchProviderRequest,
        launch_result: ProviderLaunchResult,
    ) -> Self {
        Self {
            id: id.into(),
            session_id: request.session_id.clone(),
            agent_instance_id: request.agent_id.clone(),
            adapter_key: request.adapter_key.clone(),
            provider: request.provider.clone(),
            account_profile: request.account_profile.clone(),
            model: request.model.clone(),
            variant: request.variant.clone(),
            usage_tokens_total: None,
            state: ProviderRunState::Starting,
            endpoint_mode: launch_result.endpoint_mode,
            process_label: launch_result.process_label,
            pty_target: launch_result.pty_target,
            pty_program: launch_result.pty_program,
            pty_args: launch_result.pty_args,
            pty_env: launch_result.pty_env,
            working_directory: launch_result.working_directory,
            structured_endpoint: launch_result.structured_endpoint,
            runtime_mcp_auth_token: request
                .runtime_mcp_binding
                .as_ref()
                .map(|binding| binding.auth_token.clone()),
            control_capabilities: default_control_capabilities(
                &request.adapter_key,
                launch_result.endpoint_mode,
                request.runtime_mcp_binding.is_some(),
            ),
            resume_state: request.resume_state.clone().unwrap_or_default(),
        }
    }

    pub fn from_control_capability_inference(
        id: impl Into<String>,
        session_id: String,
        agent_instance_id: Option<String>,
        adapter_key: String,
    ) -> Self {
        let inferred_has_runtime_mcp_binding = matches!(adapter_key.as_str(), "codex" | "opencode");
        Self {
            id: id.into(),
            session_id,
            agent_instance_id,
            adapter_key: adapter_key.clone(),
            provider: adapter_key.clone(),
            account_profile: "default".to_string(),
            model: String::new(),
            variant: None,
            usage_tokens_total: None,
            state: ProviderRunState::Starting,
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "inferred-control-capabilities".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: BTreeMap::new(),
            working_directory: None,
            structured_endpoint: None,
            runtime_mcp_auth_token: inferred_has_runtime_mcp_binding
                .then(|| "inferred-managed-mcp".to_string()),
            control_capabilities: default_control_capabilities(
                &adapter_key,
                AgentEndpointMode::Managed,
                inferred_has_runtime_mcp_binding,
            ),
            resume_state: ProviderResumeState::default(),
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

    pub fn set_usage_tokens_total(&mut self, usage_tokens_total: Option<u64>) {
        self.usage_tokens_total = usage_tokens_total;
    }

    pub fn state(&self) -> ProviderRunState {
        self.state
    }
    pub fn endpoint_mode(&self) -> AgentEndpointMode {
        self.endpoint_mode
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
    pub fn working_directory(&self) -> Option<&PathBuf> {
        self.working_directory.as_ref()
    }
    pub fn structured_endpoint(&self) -> Option<&str> {
        self.structured_endpoint.as_deref()
    }
    pub fn runtime_mcp_auth_token(&self) -> Option<&str> {
        self.runtime_mcp_auth_token.as_deref()
    }

    pub fn resume_state(&self) -> &ProviderResumeState {
        &self.resume_state
    }

    pub fn set_resume_state(&mut self, resume_state: ProviderResumeState) {
        self.resume_state = resume_state;
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
}

fn default_control_capabilities(
    adapter_key: &str,
    endpoint_mode: AgentEndpointMode,
    has_runtime_mcp_binding: bool,
) -> Vec<ControlCapability> {
    let mut capabilities = Vec::new();

    if matches!(adapter_key, "codex" | "opencode") {
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
    } else if endpoint_mode == AgentEndpointMode::Managed && has_runtime_mcp_binding {
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
    pub notices: Vec<String>,
}
