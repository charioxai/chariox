use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::mcp::CharioxMcpServerConfig;
use crate::session::unix_epoch_ms;

use super::launch_contract::{
    default_provider_control_capabilities, default_provider_owner_user_id,
    provider_uses_inferred_runtime_mcp_binding, AgentExecutionMode, AgentPermissionLevel,
    ExternalProviderImportMetadata, LaunchProviderRequest, ProviderLaunchResult,
    ProviderResumeState, ProviderWriteAccessMode,
};
use super::types::{
    AgentEndpointMode, ControlCapability, ControlOperation, ProviderClientInterface,
    ProviderRunState,
};

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
    #[serde(default, skip_serializing_if = "ProviderClientInterface::is_chariox")]
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
    /// Workflow-only runtime actions are projected after this run is selected
    /// for workflow execution, keeping ordinary turns' tool surface small.
    #[serde(default)]
    workflow_tools_enabled: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    mcp_servers: Vec<CharioxMcpServerConfig>,
    #[serde(
        default,
        skip_serializing_if = "crate::extension::RemoteExtensionManifest::is_empty"
    )]
    remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    provider_config_overrides: BTreeMap<String, serde_json::Value>,
    #[serde(
        default,
        skip_serializing_if = "ProviderWriteAccessMode::is_unrestricted"
    )]
    write_access_mode: ProviderWriteAccessMode,
    #[serde(skip)]
    workspace_live_sync_roots: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "AgentExecutionMode::is_build")]
    execution_mode: AgentExecutionMode,
    #[serde(default, skip_serializing_if = "AgentPermissionLevel::is_yolo")]
    permission_level: AgentPermissionLevel,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    control_capabilities: Vec<ControlCapability>,
    #[serde(default, skip_serializing_if = "ProviderResumeState::is_empty")]
    resume_state: ProviderResumeState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    external_provider_import: Option<ExternalProviderImportMetadata>,
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
            workflow_tools_enabled: false,
            mcp_servers: request.mcp_servers.clone(),
            remote_extension_manifest: request.remote_extension_manifest.clone(),
            provider_config_overrides: request.provider_config_overrides.clone(),
            write_access_mode: request.write_access_mode,
            workspace_live_sync_roots: request.workspace_live_sync_roots.clone(),
            execution_mode: request.execution_mode.unwrap_or_default(),
            permission_level: request.permission_level.unwrap_or_default(),
            control_capabilities: default_provider_control_capabilities(
                &request.adapter_key,
                request.runtime_mcp_binding.is_some(),
            ),
            resume_state: request.resume_state.clone().unwrap_or_default(),
            external_provider_import: request.external_provider_import.clone(),
            provider_session_id: request
                .resume_state
                .as_ref()
                .and_then(|state| state.provider_session_id(&request.adapter_key))
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
            provider_uses_inferred_runtime_mcp_binding(&adapter_key);
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
            client_interface: ProviderClientInterface::Chariox,
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
            workflow_tools_enabled: false,
            mcp_servers: Vec::new(),
            remote_extension_manifest: crate::extension::RemoteExtensionManifest::default(),
            provider_config_overrides: BTreeMap::new(),
            write_access_mode: ProviderWriteAccessMode::Unrestricted,
            workspace_live_sync_roots: Vec::new(),
            execution_mode: AgentExecutionMode::default(),
            permission_level: AgentPermissionLevel::default(),
            control_capabilities: default_provider_control_capabilities(
                &adapter_key,
                inferred_has_runtime_mcp_binding,
            ),
            resume_state: ProviderResumeState::default(),
            external_provider_import: None,
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

    pub fn mcp_servers(&self) -> &[CharioxMcpServerConfig] {
        &self.mcp_servers
    }

    pub fn remote_extension_manifest(&self) -> &crate::extension::RemoteExtensionManifest {
        &self.remote_extension_manifest
    }

    pub fn provider_config_overrides(&self) -> &BTreeMap<String, serde_json::Value> {
        &self.provider_config_overrides
    }

    pub fn set_remote_extension_manifest(
        &mut self,
        manifest: crate::extension::RemoteExtensionManifest,
    ) {
        self.remote_extension_manifest = manifest;
        self.touch_activity();
    }

    pub fn write_access_mode(&self) -> ProviderWriteAccessMode {
        self.write_access_mode
    }

    pub fn workspace_live_sync_roots(&self) -> &[PathBuf] {
        &self.workspace_live_sync_roots
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

    pub fn requires_workspace_live_sync(&self) -> bool {
        self.write_access_mode.requires_workspace_live_sync()
    }

    pub fn tracks_workspace_live_sync(&self) -> bool {
        self.write_access_mode.tracks_workspace_live_sync()
    }

    pub fn uses_workspace_live_sync(&self) -> bool {
        self.write_access_mode.uses_workspace_live_sync()
    }

    pub fn resume_state(&self) -> &ProviderResumeState {
        &self.resume_state
    }

    pub fn set_resume_state(&mut self, resume_state: ProviderResumeState) {
        self.resume_state = resume_state;
    }

    pub fn external_provider_import(&self) -> Option<&ExternalProviderImportMetadata> {
        self.external_provider_import.as_ref()
    }

    pub fn set_external_provider_import(&mut self, import: Option<ExternalProviderImportMetadata>) {
        self.external_provider_import = import;
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

    pub fn clear_terminal_diagnostic(&mut self) {
        self.terminal_diagnostic = None;
    }

    pub fn set_runtime_mcp_auth_token(&mut self, auth_token: Option<String>) {
        self.runtime_mcp_auth_token = auth_token;
    }

    pub fn workflow_tools_enabled(&self) -> bool {
        self.workflow_tools_enabled
    }

    pub fn enable_workflow_tools(&mut self) {
        self.workflow_tools_enabled = true;
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

    pub(crate) fn active_selection_cmp(&self, other: &Self) -> Ordering {
        (
            provider_run_active_selection_rank(self.state),
            self.last_activity_at_ms,
            self.started_at_ms,
        )
            .cmp(&(
                provider_run_active_selection_rank(other.state),
                other.last_activity_at_ms,
                other.started_at_ms,
            ))
            .then_with(|| self.id.cmp(&other.id))
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

    pub(crate) fn projected_for_home_agent_with_id(
        mut self,
        projected_id: impl Into<String>,
        session_id: impl Into<String>,
        agent_id: impl Into<String>,
    ) -> Self {
        self.id = projected_id.into();
        self.projected_for_home_agent(session_id, agent_id)
    }

    pub(crate) fn project_leased_for_home_agent(
        self,
        leased_agent_id: &str,
        session_id: impl Into<String>,
        agent_id: impl Into<String>,
    ) -> (String, Self) {
        let worker_provider_run_id = self.id().to_string();
        let projected_id =
            projected_leased_provider_run_id(leased_agent_id, &worker_provider_run_id);
        let projected_run =
            self.projected_for_home_agent_with_id(projected_id, session_id, agent_id);
        (worker_provider_run_id, projected_run)
    }
}

pub(crate) fn projected_leased_provider_run_id(
    leased_agent_id: &str,
    worker_provider_run_id: &str,
) -> String {
    format!("leased:{leased_agent_id}:{worker_provider_run_id}")
}

pub(crate) fn worker_provider_run_id_from_projected_leased_id(
    leased_agent_id: &str,
    projected_provider_run_id: &str,
) -> Option<String> {
    projected_provider_run_id
        .strip_prefix(&format!("leased:{leased_agent_id}:"))
        .filter(|worker_provider_run_id| !worker_provider_run_id.is_empty())
        .map(str::to_string)
}

fn provider_run_active_selection_rank(state: ProviderRunState) -> u8 {
    match state {
        ProviderRunState::Running => 3,
        ProviderRunState::Parked => 2,
        ProviderRunState::Starting => 1,
        ProviderRunState::Ended => 0,
    }
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        projected_leased_provider_run_id, worker_provider_run_id_from_projected_leased_id,
        RuntimeProviderRun,
    };
    use crate::provider::{
        AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, ProviderResumeState,
    };

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
    fn runtime_provider_run_clears_a_previous_turn_diagnostic() {
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
        run.set_terminal_diagnostic("refresh token revoked");

        run.clear_terminal_diagnostic();

        assert!(run.terminal_diagnostic().is_none());
    }

    #[test]
    fn runtime_provider_run_keeps_workspace_live_sync_roots_internal() {
        let roots = vec![std::path::PathBuf::from("/repo/main")];
        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "default")
                .with_workspace_live_sync_managed()
                .with_workspace_live_sync_roots(roots.clone());
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
        let run = RuntimeProviderRun::new("provider-run-1", &request, launch_result);

        assert_eq!(run.workspace_live_sync_roots(), roots.as_slice());
        assert!(!serde_json::to_value(&run)
            .expect("run should serialize")
            .as_object()
            .expect("run should serialize to object")
            .contains_key("workspace_live_sync_roots"));
    }

    #[test]
    fn projected_leased_provider_run_ids_namespace_worker_ids() {
        let request =
            LaunchProviderRequest::new("worker-session", "codex", "codex", "default", "default");
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
        let worker_run = RuntimeProviderRun::new("provider-run-1", &request, launch_result);
        let projected_id = projected_leased_provider_run_id("home-agent-1", worker_run.id());

        let (worker_provider_run_id, projected_run) = worker_run.project_leased_for_home_agent(
            "home-agent-1",
            "home-session",
            "home-agent-1",
        );

        assert_eq!(projected_id, "leased:home-agent-1:provider-run-1");
        assert_eq!(worker_provider_run_id, "provider-run-1");
        assert_eq!(projected_run.id(), projected_id);
        assert_eq!(projected_run.session_id(), "home-session");
        assert_eq!(projected_run.agent_instance_id(), Some("home-agent-1"));
    }

    #[test]
    fn projected_leased_provider_run_ids_recover_worker_id() {
        assert_eq!(
            worker_provider_run_id_from_projected_leased_id(
                "home-agent-1",
                "leased:home-agent-1:provider-run-1",
            )
            .as_deref(),
            Some("provider-run-1"),
        );
        assert_eq!(
            worker_provider_run_id_from_projected_leased_id(
                "home-agent-2",
                "leased:home-agent-1:provider-run-1",
            ),
            None,
        );
        assert_eq!(
            worker_provider_run_id_from_projected_leased_id("home-agent-1", "provider-run-1"),
            None,
        );
    }
}
