use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use tokio::runtime::{Handle, Runtime};

mod kernel_agent;
mod kernel_session;
mod prompt_lifecycle;
mod prompt_state_owner;
pub(crate) mod provider_output;
mod provider_runtime;
mod session_runtime;
mod terminal_fanout;
pub(crate) mod workflow_runtime;

pub(crate) use prompt_lifecycle::KernelPreparedPromptSubmission;

use arroba_relay::protocol::{ClientTarget, DaemonRegistration, RelayKernelPresence};

use crate::agent::{AgentInstance, AgentService, CreateAgentRequest, RemoteAgentBinding};
use crate::attachment::{
    AttachRequest, AttachmentService, ClientCapabilityLevel, RuntimeAttachment,
};
use crate::capability::{
    CaptureScreenshotRequest, CaptureScreenshotResult, DirectoryTreeService, EditFileRequest,
    EditFileResult, FileCapabilityService, FileTransferService, GitCapabilityService,
    InspectGitRequest, InspectGitResult, ReadDirectoryTreeRequest, ReadDirectoryTreeResult,
    ReadFileRequest, ReadFileResult, RunShellCommandRequest, RunShellCommandResult,
    ScreenshotCapabilityService, ShellCommandService, StoreTransferredFileRequest,
    StoredTransferArtifact,
};
use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::execution_lease::{
    ExecutionLease, LeasedAgent, LeasedWorkflowTurnBinding, RemoteWorkflowTurnContext,
};
use crate::history::{SessionHistoryEntry, SessionHistoryStore};
use crate::kernel::projection::{
    page_history_entries, AgentRuntimeProjectionStore, DaemonConfigProjectionStore,
    ProviderCatalogProjectionStore, ProviderProcessProjectionStore, ProviderRunProjectionStore,
    SessionHistoryProjectionStore, SessionStateProjectionStore, TransportHealthStore,
};
use crate::kernel::prompt_state::PromptStateOwner;
use crate::kernel::workspace_coordinator::{WorkspaceClaimGuard, WorkspaceCoordinator};
use crate::provider::{
    LaunchProviderRequest, OpenCodeProviderCatalog, ProviderProcessInfo, ProviderProcessService,
    ProviderRunOperationLanes, RuntimeProviderRun,
};
use crate::pty::PtyManager;
use crate::session::{
    CreateSessionRequest, PromptAttachment, RuntimeSession, SessionConfigState, SessionService,
};
pub use crate::session_history_page::{
    SessionHistoryCursor, SessionHistoryPage, SessionHistoryPageEntry,
};
use crate::terminal::{TerminalOutputKind, TerminalStreamHealthStore, TerminalStreamStore};
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_client::RelayClientState;
use crate::transport::relay_discovery;
use crate::transport::relay_peer::{
    RelayPeerEvent, RelayPeerRequest, RelayPeerResponse, RelayProjectedCompletion,
    RelayProjectedOutputChunk, RelayPromptAttachment,
};
pub(crate) use kernel_agent::KernelAgentService;
pub(crate) use kernel_session::KernelSessionService;

pub struct DaemonApp {
    config: DaemonConfig,
    started_at_ms: u64,
    relay_client_state: Arc<tokio::sync::RwLock<RelayClientState>>,
    pub(crate) agents: AgentService,
    pub(crate) attachments: AttachmentService,
    capabilities: ShellCommandService,
    directory_tree: DirectoryTreeService,
    file_capabilities: FileCapabilityService,
    git_capabilities: GitCapabilityService,
    screenshot_capabilities: ScreenshotCapabilityService,
    transfer_capabilities: FileTransferService,
    pty: PtyManager,
    pub(crate) providers: ProviderProcessService,
    pub(crate) provider_catalog_cache: Option<(Instant, OpenCodeProviderCatalog)>,
    pub(crate) tracked_provider_processes: BTreeMap<String, TrackedProviderProcess>,
    pub(crate) tracked_provider_run_processes: BTreeMap<String, String>,
    pub(crate) prompt_activity: BTreeMap<String, ActivePromptState>,
    prompt_workspace_claims: BTreeMap<String, WorkspaceClaimGuard>,
    pub(crate) prompt_idle_timeout: Duration,
    prompt_state_owner: PromptStateOwner,
    pub(crate) sessions: SessionService,
    history: SessionHistoryStore,
    config_projection: DaemonConfigProjectionStore,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    history_projection: SessionHistoryProjectionStore,
    provider_catalog_projection: ProviderCatalogProjectionStore,
    provider_run_projection: ProviderRunProjectionStore,
    provider_process_projection: ProviderProcessProjectionStore,
    transport_health: TransportHealthStore,
    workspace_coordinator: WorkspaceCoordinator,
    terminal: TerminalStreamStore,
    pending_structured_output_records: provider_output::StructuredOutputRecordStore,
    execution_leases: BTreeMap<String, ExecutionLease>,
    leased_agents: BTreeMap<String, LeasedAgent>,
    leased_workflow_turns: BTreeMap<String, LeasedWorkflowTurnBinding>,
    next_execution_lease_number: u64,
    next_leased_agent_number: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct ActivePromptState {
    pub(crate) last_output_at: Option<Instant>,
    pub(crate) saw_response_content: bool,
    pub(crate) completion_recorded: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct CapabilityRuntimeContext {
    pub(crate) workspace_id: String,
    pub(crate) worktree_root: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct TrackedProviderProcess {
    pub(crate) process_id: String,
    pub(crate) pid: Option<u32>,
    pub(crate) endpoint_mode: crate::provider::AgentEndpointMode,
    pub(crate) process_label: String,
    pub(crate) started_at_ms: u64,
    pub(crate) owner_provider_run_ids: Vec<String>,
}

impl DaemonApp {
    pub(crate) fn kernel_agents(&mut self) -> KernelAgentService<'_> {
        KernelAgentService::new(self)
    }

    pub(crate) fn kernel_sessions(&mut self) -> KernelSessionService<'_> {
        KernelSessionService::new(self)
    }

    pub(crate) fn artifact_attachment_segment(attachment_id: &str) -> String {
        attachment_id
            .chars()
            .map(|ch| match ch {
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
                ch if ch.is_control() => '_',
                ch => ch,
            })
            .collect()
    }

    pub(crate) fn attachment_artifact_root(
        session_id: &str,
        attachment_id: &str,
        category: &str,
    ) -> PathBuf {
        std::env::temp_dir()
            .join("arroba-session-artifacts")
            .join(session_id)
            .join(category)
            .join(Self::artifact_attachment_segment(attachment_id))
    }

    pub(crate) fn attachment_artifact_roots(session_id: &str, attachment_id: &str) -> [PathBuf; 2] {
        [
            Self::attachment_artifact_root(session_id, attachment_id, "screenshots"),
            Self::attachment_artifact_root(session_id, attachment_id, "transfers"),
        ]
    }

    pub fn bootstrap(config: DaemonConfig) -> Result<Self, DaemonError> {
        config.validate()?;

        Ok(Self {
            agents: AgentService::new(),
            attachments: AttachmentService::new(),
            capabilities: ShellCommandService::new(),
            directory_tree: DirectoryTreeService::new(),
            file_capabilities: FileCapabilityService::new(),
            git_capabilities: GitCapabilityService::new(),
            screenshot_capabilities: ScreenshotCapabilityService::new(),
            transfer_capabilities: FileTransferService::new(),
            pty: PtyManager::new(),
            providers: ProviderProcessService::new(),
            provider_catalog_cache: None,
            tracked_provider_processes: BTreeMap::new(),
            tracked_provider_run_processes: BTreeMap::new(),
            prompt_activity: BTreeMap::new(),
            prompt_workspace_claims: BTreeMap::new(),
            prompt_idle_timeout: prompt_idle_timeout(),
            prompt_state_owner: PromptStateOwner::default(),
            sessions: SessionService::new(&config),
            history: SessionHistoryStore::new_with_read_delay(
                config.session_history_root.clone(),
                config.session_history_read_delay_ms,
            )?,
            config_projection: DaemonConfigProjectionStore::new(config.clone()),
            session_projection: SessionStateProjectionStore::default(),
            agent_runtime_projection: AgentRuntimeProjectionStore::default(),
            history_projection: SessionHistoryProjectionStore::default(),
            provider_catalog_projection: ProviderCatalogProjectionStore::default(),
            provider_run_projection: ProviderRunProjectionStore::default(),
            provider_process_projection: ProviderProcessProjectionStore::default(),
            transport_health: TransportHealthStore::default(),
            workspace_coordinator: WorkspaceCoordinator::default(),
            terminal: TerminalStreamStore::new(),
            pending_structured_output_records:
                provider_output::StructuredOutputRecordStore::default(),
            execution_leases: BTreeMap::new(),
            leased_agents: BTreeMap::new(),
            leased_workflow_turns: BTreeMap::new(),
            next_execution_lease_number: 0,
            next_leased_agent_number: 0,
            started_at_ms: crate::session::unix_epoch_ms(),
            relay_client_state: Arc::new(tokio::sync::RwLock::new(RelayClientState::default())),
            config,
        })
    }

    pub(crate) fn provider_run_operation_lanes(&self) -> ProviderRunOperationLanes {
        self.providers.run_operation_lanes()
    }

    /// Create a new session with a default agent
    pub fn create_session(
        &mut self,
        request: CreateSessionRequest,
    ) -> Result<(RuntimeSession, AgentInstance), DaemonError> {
        self.kernel_sessions().create_session(request)
    }

    pub fn config(&self) -> &DaemonConfig {
        &self.config
    }

    pub(crate) fn relay_client_state(&self) -> Arc<tokio::sync::RwLock<RelayClientState>> {
        Arc::clone(&self.relay_client_state)
    }

    pub(crate) fn config_projection_store(&self) -> DaemonConfigProjectionStore {
        self.config_projection.clone()
    }

    pub(crate) fn configure_relay(
        &mut self,
        relay_url: Option<String>,
        relay_token: Option<String>,
    ) -> Result<(), DaemonError> {
        self.config.relay_url = relay_url
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        self.config.relay_token = relay_token
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        self.config.validate()?;
        self.config.persist_relay_config()?;
        self.config_projection.update(self.config.clone());
        Ok(())
    }

    pub fn sessions(&self) -> &SessionService {
        &self.sessions
    }

    pub(crate) fn history_store(&self) -> SessionHistoryStore {
        self.history.clone()
    }

    pub(crate) fn session_state_projection_store(&self) -> SessionStateProjectionStore {
        self.session_projection.clone()
    }

    pub(crate) fn agent_runtime_projection_store(&self) -> AgentRuntimeProjectionStore {
        self.agent_runtime_projection.clone()
    }

    pub(crate) fn prompt_state_owner(&self) -> PromptStateOwner {
        self.prompt_state_owner.clone()
    }

    pub(crate) fn prompt_id_allocator(&self) -> crate::session::PromptIdAllocator {
        self.sessions.prompt_id_allocator()
    }

    pub(crate) fn update_session_projection(&self, session: RuntimeSession) {
        self.agent_runtime_projection.update_session(&session);
        self.session_projection.update(session);
    }

    pub(crate) fn remove_session_projection(&self, session_id: &str) {
        self.session_projection.remove(session_id);
        self.agent_runtime_projection.remove_session(session_id);
    }

    pub(crate) fn publish_session_projection(
        &self,
        session_id: &str,
    ) -> Result<RuntimeSession, DaemonError> {
        let session = self.local_api_session_snapshot(session_id)?;
        self.update_session_projection(session.clone());
        Ok(session)
    }

    pub(crate) fn session_history_projection_store(&self) -> SessionHistoryProjectionStore {
        self.history_projection.clone()
    }

    pub(crate) fn provider_catalog_projection_store(&self) -> ProviderCatalogProjectionStore {
        self.provider_catalog_projection.clone()
    }

    pub(crate) fn update_provider_catalog_projection(&self, catalog: OpenCodeProviderCatalog) {
        self.provider_catalog_projection.update(catalog);
    }

    pub(crate) fn invalidate_provider_catalog_projection(&self) {
        self.provider_catalog_projection.invalidate();
    }

    pub(crate) fn provider_run_projection_store(&self) -> ProviderRunProjectionStore {
        self.provider_run_projection.clone()
    }

    pub(crate) fn provider_process_projection_store(&self) -> ProviderProcessProjectionStore {
        self.provider_process_projection.clone()
    }

    pub(crate) fn transport_health_store(&self) -> TransportHealthStore {
        self.transport_health.clone()
    }

    pub(crate) fn workspace_coordinator(&self) -> WorkspaceCoordinator {
        self.workspace_coordinator.clone()
    }

    pub(crate) fn acquire_prompt_workspace_claim(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        attachment_id: Option<&str>,
    ) -> Result<(), DaemonError> {
        if self.prompt_workspace_claims.contains_key(provider_run_id) {
            return Ok(());
        }
        let session = self.sessions.get_session(session_id)?;
        let workspace_id = session.workspace_id().to_string();
        let worktree_id = self
            .agents
            .get_agent(agent_id)
            .ok()
            .and_then(|agent| agent.worktree_id().map(str::to_string))
            .unwrap_or_else(|| session.worktree_id().to_string());
        let claim = self.workspace_coordinator.acquire_provider_prompt_claim(
            workspace_id,
            worktree_id,
            session_id,
            attachment_id.map(str::to_string),
        )?;
        self.prompt_workspace_claims
            .insert(provider_run_id.to_string(), claim);
        Ok(())
    }

    pub(crate) fn acquire_workflow_node_workspace_claim(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<(), DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let workspace_id = session.workspace_id().to_string();
        let worktree_id = self
            .agents
            .get_agent(agent_id)
            .ok()
            .and_then(|agent| agent.worktree_id().map(str::to_string))
            .unwrap_or_else(|| session.worktree_id().to_string());
        let claim = self.workspace_coordinator.acquire_worktree_write_claim(
            workspace_id,
            worktree_id,
            session_id,
            Some(format!("{}:{}", workflow_run_id, workflow_node_run_id)),
            "workflow_node_dispatch",
        )?;
        self.prompt_workspace_claims
            .insert(provider_run_id.to_string(), claim);
        Ok(())
    }

    pub(crate) fn release_prompt_workspace_claim(&mut self, provider_run_id: &str) -> bool {
        self.prompt_workspace_claims
            .remove(provider_run_id)
            .is_some()
    }

    pub(crate) fn update_provider_run_projection(&self, run: RuntimeProviderRun) {
        self.provider_run_projection.update(run);
        self.provider_process_projection.invalidate();
    }

    pub(crate) fn update_provider_process_projection(&self, processes: Vec<ProviderProcessInfo>) {
        self.provider_process_projection.update_list(processes);
    }

    pub fn sessions_mut(&mut self) -> &mut SessionService {
        &mut self.sessions
    }

    pub fn agents(&self) -> &AgentService {
        &self.agents
    }

    pub fn agents_mut(&mut self) -> &mut AgentService {
        &mut self.agents
    }

    pub fn attachments(&self) -> &AttachmentService {
        &self.attachments
    }

    pub fn attachments_mut(&mut self) -> &mut AttachmentService {
        &mut self.attachments
    }

    pub fn capabilities(&self) -> &ShellCommandService {
        &self.capabilities
    }

    pub fn providers(&self) -> &ProviderProcessService {
        &self.providers
    }

    pub fn providers_mut(&mut self) -> &mut ProviderProcessService {
        &mut self.providers
    }

    pub fn terminal(&self) -> &TerminalStreamStore {
        &self.terminal
    }

    pub(crate) fn terminal_health_store(&self) -> TerminalStreamHealthStore {
        self.terminal.health_store()
    }

    pub(crate) fn terminal_stream_store(&self) -> TerminalStreamStore {
        self.terminal.clone()
    }

    /// Spawn a new agent in a session
    pub fn spawn_agent(
        &mut self,
        request: crate::agent::CreateAgentRequest,
    ) -> Result<AgentInstance, DaemonError> {
        if let Some(machine_ref) = request.machine_ref.clone() {
            return self.spawn_remote_agent(request, &machine_ref);
        }
        self.agents.create_agent(request, &mut self.sessions)
    }

    /// Destroy an agent
    pub fn destroy_agent(&mut self, agent_id: &str) -> Result<AgentInstance, DaemonError> {
        let agent = self.agents.get_agent(agent_id)?;
        if let Some(remote) = agent.remote_execution().cloned() {
            let target = ClientTarget {
                daemon_id: Some(remote.worker_kernel_id.clone()),
                daemon_alias: None,
            };
            self.block_on_relay_future(send_peer_request_via_temporary_connection(
                &self.config,
                target.clone(),
                RelayPeerRequest::DestroyLeasedAgent {
                    leased_agent_id: remote.leased_agent_id.clone(),
                },
            ))?;
            self.block_on_relay_future(send_peer_request_via_temporary_connection(
                &self.config,
                target,
                RelayPeerRequest::DestroyExecutionLease {
                    lease_id: remote.execution_lease_id.clone(),
                },
            ))?;
        }
        self.agents.destroy_agent(agent_id, &mut self.sessions)
    }

    /// Focus a specific agent in a session
    pub fn focus_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<AgentInstance, DaemonError> {
        self.kernel_sessions().focus_agent(session_id, agent_id)
    }

    /// Cycle focus to next agent in session
    pub fn cycle_agent_focus(
        &mut self,
        session_id: &str,
    ) -> Result<Option<AgentInstance>, DaemonError> {
        self.kernel_sessions().cycle_agent_focus(session_id)
    }

    /// Get all agents in a session
    pub fn list_session_agents(&self, session_id: &str) -> Vec<AgentInstance> {
        self.agents.get_session_agents(session_id)
    }

    pub fn session_history(
        &self,
        session_id: &str,
    ) -> Result<Vec<SessionHistoryEntry>, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let entries = self.history.load(&session)?;
        self.history_projection
            .update_entries(session.id(), entries.clone());
        Ok(entries)
    }

    pub fn session_history_page(
        &self,
        session_id: &str,
        agent_id: Option<&str>,
        round_count: Option<usize>,
        max_chars: Option<usize>,
        before_entry_index: Option<usize>,
        before_entry_char_offset: Option<usize>,
    ) -> Result<SessionHistoryPage, DaemonError> {
        let entries = self.session_history(session_id)?;
        Ok(page_history_entries(
            entries,
            agent_id,
            round_count,
            max_chars,
            before_entry_index,
            before_entry_char_offset,
        ))
    }

    pub(crate) fn terminal_mut(&mut self) -> &TerminalStreamStore {
        &self.terminal
    }

    pub fn pty(&self) -> &PtyManager {
        &self.pty
    }

    pub fn resolve_session_ref(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<RuntimeSession, DaemonError> {
        self.sessions.resolve_session_ref(session_ref, workspace_id)
    }

    pub fn run_shell_command(
        &self,
        request: RunShellCommandRequest,
    ) -> Result<RunShellCommandResult, DaemonError> {
        let worktree_root =
            self.capability_worktree_root(&request.session_id, &request.attachment_id, "shell")?;

        let mut request = request;
        request.worktree_root = worktree_root;
        self.capabilities.run(request)
    }

    pub fn read_directory_tree(
        &self,
        session_id: &str,
        attachment_id: &str,
        path: Option<PathBuf>,
        max_depth: usize,
    ) -> Result<ReadDirectoryTreeResult, DaemonError> {
        let worktree_root =
            self.capability_worktree_root(session_id, attachment_id, "directory_tree")?;
        self.directory_tree.read_tree(ReadDirectoryTreeRequest::new(
            session_id,
            attachment_id,
            worktree_root,
            path,
            max_depth,
        ))
    }

    pub fn read_file(
        &self,
        session_id: &str,
        attachment_id: &str,
        path: PathBuf,
    ) -> Result<ReadFileResult, DaemonError> {
        let worktree_root =
            self.capability_worktree_root(session_id, attachment_id, "file_read")?;
        self.file_capabilities.read_file(ReadFileRequest::new(
            session_id,
            attachment_id,
            worktree_root,
            path,
        ))
    }

    pub fn edit_file(
        &self,
        session_id: &str,
        attachment_id: &str,
        path: PathBuf,
        contents: String,
    ) -> Result<EditFileResult, DaemonError> {
        let context = self.capability_context(session_id, attachment_id, "file_edit")?;
        let _claim = self.workspace_coordinator.acquire_worktree_write_claim(
            context.workspace_id,
            context.worktree_root.display().to_string(),
            session_id.to_string(),
            Some(attachment_id.to_string()),
            "file_edit",
        )?;
        self.file_capabilities.edit_file(EditFileRequest::new(
            session_id,
            attachment_id,
            context.worktree_root,
            path,
            contents,
        ))
    }

    pub fn inspect_git(
        &self,
        session_id: &str,
        attachment_id: &str,
        working_directory: Option<PathBuf>,
    ) -> Result<InspectGitResult, DaemonError> {
        let worktree_root =
            self.capability_worktree_root(session_id, attachment_id, "git_inspect")?;
        self.git_capabilities.inspect(InspectGitRequest::new(
            session_id,
            attachment_id,
            worktree_root,
            working_directory,
        ))
    }

    pub fn capture_screenshot(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<CaptureScreenshotResult, DaemonError> {
        let _ = self.capability_worktree_root(session_id, attachment_id, "screenshot")?;
        self.screenshot_capabilities
            .capture(CaptureScreenshotRequest::new(
                session_id,
                attachment_id,
                Self::attachment_artifact_root(session_id, attachment_id, "screenshots"),
            ))
    }

    pub fn store_transferred_file(
        &self,
        session_id: &str,
        attachment_id: &str,
        source_path: PathBuf,
        display_name: Option<String>,
    ) -> Result<StoredTransferArtifact, DaemonError> {
        let context = self.capability_context(session_id, attachment_id, "transfer_store")?;
        let _claim = self.workspace_coordinator.acquire_worktree_write_claim(
            context.workspace_id,
            context.worktree_root.display().to_string(),
            session_id.to_string(),
            Some(attachment_id.to_string()),
            "transfer_store",
        )?;
        self.transfer_capabilities
            .store_file(StoreTransferredFileRequest::new(
                session_id,
                attachment_id,
                context.worktree_root,
                Self::attachment_artifact_root(session_id, attachment_id, "transfers"),
                source_path,
                display_name,
            ))
    }

    pub fn update_session_config(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        values: BTreeMap<String, String>,
        requires_idle: bool,
    ) -> Result<SessionConfigState, DaemonError> {
        self.ensure_attachment_in_session(session_id, attachment_id)?;
        let (_session, config) =
            self.sessions
                .update_config(session_id, attachment_id, values, requires_idle)?;

        let recipient_attachment_ids = self.other_attachment_ids(session_id, attachment_id);
        if !recipient_attachment_ids.is_empty() {
            let active_provider_run_id = self
                .sessions
                .get_session(session_id)?
                .active_provider_run_id()
                .map(str::to_string);
            self.record_notice(
                session_id,
                active_provider_run_id.as_deref(),
                recipient_attachment_ids,
                format!(
                    "Attachment `{attachment_id}` updated configuration for session `{session_id}`."
                ),
            );
        }

        Ok(config)
    }

    pub fn send_provider_input(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        attachment_id: &str,
        bytes: &[u8],
    ) -> Result<(), DaemonError> {
        let _ = provider_runtime::ProviderRunLivenessRuntime::new(self)
            .reconcile_provider_run_exit(session_id, provider_run_id)?;
        if !crate::scheduler::runtime::is_workflow_prompt_attachment(attachment_id) {
            self.ensure_attachment_in_session(session_id, attachment_id)?;
        }
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;

        if provider_run.state() != crate::provider::ProviderRunState::Running {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: provider_run_id.to_string(),
                state: provider_run.state(),
                operation: "send terminal input",
            });
        }

        self.terminal
            .record_input(session_id, provider_run_id, attachment_id, bytes);
        self.pty.write_input(provider_run_id, bytes)
    }

    pub fn send_terminal_input(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        bytes: &[u8],
    ) -> Result<(), DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let provider_run_id = session
            .active_provider_run_id()
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?
            .to_string();

        self.send_provider_input(session_id, &provider_run_id, attachment_id, bytes)
    }

    pub fn resize_provider_terminal(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), DaemonError> {
        self.kernel_sessions()
            .resize_provider_terminal(session_id, provider_run_id, cols, rows)
    }

    pub fn resize_terminal(
        &mut self,
        session_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), DaemonError> {
        self.kernel_sessions()
            .resize_terminal(session_id, cols, rows)
    }

    pub fn pump_active_prompt_outputs(&mut self) {
        self.reap_structured_prompt_jobs();
        let sessions = self.sessions.list_sessions();
        for session in sessions {
            let recipient_attachment_ids =
                self.attachments.list_session_attachment_ids(session.id());
            let mut agent_ids = session
                .agents()
                .iter()
                .map(|agent| agent.id().to_string())
                .collect::<Vec<_>>();
            agent_ids.extend(session.prompt_states().keys().cloned());
            agent_ids.sort();
            agent_ids.dedup();
            for agent_id in agent_ids {
                if self
                    .prompt_state_owner
                    .active_prompt_for_agent_snapshot(&session, &agent_id)
                    .is_none()
                {
                    continue;
                }
                let Some(provider_run_id) = self
                    .providers
                    .get_run_for_agent(session.id(), &agent_id)
                    .map(|run| run.id().to_string())
                else {
                    continue;
                };
                if let Err(error) = provider_output::ProviderOutputPump::new(self)
                    .pump_provider_output(provider_output::ProviderOutputPumpRequest {
                        session_id: session.id(),
                        provider_run_id: &provider_run_id,
                        recipient_attachment_ids: recipient_attachment_ids.clone(),
                    })
                {
                    crate::logging::warn_with_fields(
                        "daemon.app",
                        "background prompt pump failed",
                        serde_json::json!({
                            "session_id": session.id(),
                            "provider_run_id": provider_run_id,
                            "agent_id": agent_id,
                            "error": error.to_string(),
                        }),
                    );
                }
            }
        }
    }

    pub(crate) fn ensure_attachment_in_session(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<RuntimeAttachment, DaemonError> {
        let attachment = self.attachments.get_attachment(attachment_id)?;
        if attachment.session_id() != session_id {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }
        Ok(attachment)
    }

    pub(crate) fn ensure_provider_run_in_session(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let provider_run = self.providers.get_run(provider_run_id)?;

        if provider_run.session_id() != session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: session_id.to_string(),
                provider_run_id: provider_run_id.to_string(),
            });
        }

        Ok(provider_run)
    }

    pub(crate) fn dispatch_prompt_to_provider(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        attachment_id: &str,
        prompt: &str,
        attachments: &[PromptAttachment],
    ) -> Result<(), DaemonError> {
        let _ = provider_runtime::ProviderRunLivenessRuntime::new(self)
            .reconcile_provider_run_exit(session_id, provider_run_id)?;
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
        if provider_run.state() != crate::provider::ProviderRunState::Running {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: provider_run_id.to_string(),
                state: provider_run.state(),
                operation: "submit prompt",
            });
        }

        if self.providers.run_uses_structured_prompt_io(&provider_run) {
            let agent_id = provider_run
                .agent_instance_id()
                .ok_or_else(|| DaemonError::AgentNotFound {
                    agent_id: "provider run has no agent".to_string(),
                })?
                .to_string();
            self.providers.enqueue_structured_prompt_submit(
                session_id.to_string(),
                provider_run_id.to_string(),
                agent_id,
                &provider_run,
                prompt,
                attachments,
            )?;
            return Ok(());
        }

        self.send_provider_input(
            session_id,
            provider_run_id,
            attachment_id,
            prompt.as_bytes(),
        )
    }

    pub fn create_execution_lease(
        &mut self,
        home_kernel_id: &str,
        home_session_id: &str,
        home_agent_id: &str,
    ) -> Result<ExecutionLease, DaemonError> {
        if !self.config.accept_remote_leases {
            return Err(DaemonError::RemoteLeasesDisabled {
                machine_id: self.config.host_machine_id.clone(),
            });
        }
        self.next_execution_lease_number = self.next_execution_lease_number.wrapping_add(1);
        let lease_id = format!(
            "lease-{:016x}",
            crate::session::unix_epoch_ms() ^ self.next_execution_lease_number.rotate_left(11)
        );
        let lease = ExecutionLease::new(
            lease_id.clone(),
            home_kernel_id.to_string(),
            home_session_id.to_string(),
            home_agent_id.to_string(),
            self.config.daemon_id.clone(),
            self.config.host_machine_id.clone(),
        );
        self.execution_leases.insert(lease_id, lease.clone());
        Ok(lease)
    }

    pub fn destroy_execution_lease(
        &mut self,
        lease_id: &str,
    ) -> Result<ExecutionLease, DaemonError> {
        self.leased_agents
            .retain(|_, agent| agent.lease_id != lease_id);
        self.execution_leases
            .remove(lease_id)
            .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                lease_id: lease_id.to_string(),
            })
    }

    pub fn create_leased_agent(
        &mut self,
        lease_id: &str,
        provider: &str,
        model: Option<String>,
        effort: Option<String>,
    ) -> Result<LeasedAgent, DaemonError> {
        let lease = self
            .execution_leases
            .get(lease_id)
            .cloned()
            .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                lease_id: lease_id.to_string(),
            })?;
        if self.providers.registry().resolve(provider).is_none() {
            return Err(DaemonError::ProviderAdapterNotFound {
                adapter_key: provider.to_string(),
            });
        }
        let worktree = std::env::current_dir()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "resolve leased agent working directory",
                message: error.to_string(),
            })?
            .display()
            .to_string();
        let session = self.sessions.create_session(
            CreateSessionRequest::new(format!("remote-lease:{}", lease.home_session_id), worktree)
                .with_hidden(true),
        )?;
        let attachment = self.attachments.attach(
            &mut self.sessions,
            AttachRequest::new(
                session.id(),
                format!("leased-agent:{}", lease.home_agent_id),
                ClientCapabilityLevel::MessageTransport,
            ),
        )?;
        let backing_agent = self.agents.create_agent(
            CreateAgentRequest::new(session.id(), provider)
                .with_worktree(session.worktree_id())
                .with_model(model.clone().unwrap_or_else(|| "default".to_string()))
                .with_effort(effort.clone().unwrap_or_else(|| "medium".to_string())),
            &mut self.sessions,
        )?;
        self.next_leased_agent_number = self.next_leased_agent_number.wrapping_add(1);
        let agent_id = format!(
            "leased-agent-{:016x}",
            crate::session::unix_epoch_ms() ^ self.next_leased_agent_number.rotate_left(13)
        );
        let agent = LeasedAgent::new(
            agent_id.clone(),
            lease_id.to_string(),
            lease.home_agent_id.clone(),
            provider.to_string(),
            model,
            effort,
            session.id().to_string(),
            backing_agent.id().to_string(),
            attachment.id().to_string(),
        );
        self.leased_agents.insert(agent_id, agent.clone());
        Ok(agent)
    }

    pub fn destroy_leased_agent(
        &mut self,
        leased_agent_id: &str,
    ) -> Result<LeasedAgent, DaemonError> {
        let agent = self.leased_agents.remove(leased_agent_id).ok_or_else(|| {
            DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            }
        })?;
        self.leased_workflow_turns
            .retain(|_, binding| binding.leased_agent_id != leased_agent_id);
        let _ = self
            .attachments
            .detach(&mut self.sessions, &agent.backing_attachment_id);
        let _ = self
            .agents
            .destroy_agent(&agent.backing_agent_id, &mut self.sessions);
        let _ = self.sessions.end_session(&agent.backing_session_id);
        let _ = self.sessions.delete_session(&agent.backing_session_id);
        self.history_projection.remove(&agent.backing_session_id);
        Ok(agent)
    }

    pub fn submit_leased_prompt(
        &mut self,
        leased_agent_id: &str,
        prompt: &str,
        attachments: Vec<RelayPromptAttachment>,
    ) -> Result<(String, crate::session::PromptSubmissionOutcome), DaemonError> {
        self.submit_leased_prompt_with_workflow_context(leased_agent_id, prompt, attachments, None)
    }

    pub fn submit_leased_prompt_with_workflow_context(
        &mut self,
        leased_agent_id: &str,
        prompt: &str,
        attachments: Vec<RelayPromptAttachment>,
        workflow_context: Option<RemoteWorkflowTurnContext>,
    ) -> Result<(String, crate::session::PromptSubmissionOutcome), DaemonError> {
        let leased_agent = self
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        let materialized_attachments =
            self.materialize_leased_prompt_attachments(&leased_agent, attachments)?;
        let provider_run_id = if let Some(run) = self.providers.get_run_for_agent(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        ) {
            run.id().to_string()
        } else {
            let run = self.launch_provider(
                LaunchProviderRequest::new(
                    &leased_agent.backing_session_id,
                    &leased_agent.provider,
                    &leased_agent.provider,
                    "default",
                    leased_agent
                        .model
                        .clone()
                        .unwrap_or_else(|| "default".to_string()),
                )
                .with_agent_id(&leased_agent.backing_agent_id),
            )?;
            run.id().to_string()
        };
        let outcome = self.submit_prompt(
            &leased_agent.backing_session_id,
            &leased_agent.backing_attachment_id,
            Some(&leased_agent.backing_agent_id),
            prompt,
            materialized_attachments,
        )?;
        if let Some(context) = workflow_context {
            self.leased_workflow_turns.insert(
                provider_run_id.clone(),
                LeasedWorkflowTurnBinding {
                    leased_agent_id: leased_agent_id.to_string(),
                    provider_run_id: provider_run_id.clone(),
                    context,
                },
            );
        }
        Ok((provider_run_id, outcome))
    }

    pub fn leased_workflow_turn_binding_for_provider_run(
        &self,
        provider_run_id: &str,
    ) -> Option<LeasedWorkflowTurnBinding> {
        self.leased_workflow_turns.get(provider_run_id).cloned()
    }

    pub(crate) fn remote_workflow_turn_context_for_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<RemoteWorkflowTurnContext, DaemonError> {
        let workflow_run_id =
            prompt
                .workflow_run_id()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "dispatch remote workflow prompt",
                    message: "remote workflow prompt is missing workflow run id".to_string(),
                })?;
        let workflow_node_run_id =
            prompt
                .workflow_node_run_id()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "dispatch remote workflow prompt",
                    message: "remote workflow prompt is missing workflow node run id".to_string(),
                })?;
        let workflow_run = self
            .sessions()
            .resolve_workflow_run_ref(session_id, workflow_run_id)?;
        let delivery_token = workflow_run
            .node_runs()
            .iter()
            .find(|node_run| node_run.id() == workflow_node_run_id)
            .and_then(|node_run| node_run.turn_envelope())
            .map(|envelope| envelope.delivery_token().to_string())
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "dispatch remote workflow prompt",
                message: format!(
                    "workflow node run `{workflow_node_run_id}` has no prepared turn envelope"
                ),
            })?;
        Ok(RemoteWorkflowTurnContext {
            home_kernel_id: self.config().daemon_id.clone(),
            home_session_id: session_id.to_string(),
            home_agent_id: target_agent_id.to_string(),
            workflow_run_id: workflow_run.id().to_string(),
            workflow_node_run_id: workflow_node_run_id.to_string(),
            delivery_token,
        })
    }

    pub fn complete_leased_prompt(
        &mut self,
        leased_agent_id: &str,
    ) -> Result<crate::session::PromptCompletion, DaemonError> {
        let leased_agent = self
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        let provider_run_id = self
            .providers
            .get_run_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )
            .map(|run| run.id().to_string());
        let completion = self.complete_active_prompt(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
            provider_run_id.as_deref(),
        )?;
        if let Some(provider_run_id) = provider_run_id {
            self.leased_workflow_turns.remove(&provider_run_id);
        }
        Ok(completion)
    }

    pub fn cancel_leased_prompt(
        &mut self,
        leased_agent_id: &str,
    ) -> Result<crate::session::PromptCancellation, DaemonError> {
        let leased_agent = self
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        let cancellation = self.cancel_active_prompt_internal(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
            None,
        )?;
        self.leased_workflow_turns
            .retain(|_, binding| binding.leased_agent_id != leased_agent_id);
        Ok(cancellation)
    }

    pub fn leased_agent_provider_run_id(
        &self,
        leased_agent_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        let leased_agent = self
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        Ok(self
            .providers
            .get_run_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )
            .map(|run| run.id().to_string()))
    }

    pub fn leased_agent_active_prompt_attachments(
        &self,
        leased_agent_id: &str,
    ) -> Result<Vec<crate::session::PromptAttachment>, DaemonError> {
        let leased_agent = self
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        Ok(self
            .prompt_owner_active_prompt_for_agent_snapshot(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )?
            .map(|prompt| prompt.attachments().to_vec())
            .unwrap_or_default())
    }

    fn materialize_leased_prompt_attachments(
        &self,
        leased_agent: &LeasedAgent,
        attachments: Vec<RelayPromptAttachment>,
    ) -> Result<Vec<crate::session::PromptAttachment>, DaemonError> {
        attachments
            .into_iter()
            .enumerate()
            .map(|(index, attachment)| {
                if let Some(contents_base64) = attachment.contents_base64 {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(contents_base64)
                        .map_err(|error| DaemonError::LocalTransport {
                            operation: "decode remote prompt attachment",
                            message: error.to_string(),
                        })?;
                    let filename = attachment
                        .filename
                        .clone()
                        .unwrap_or_else(|| format!("attachment-{index}"));
                    let root = std::env::temp_dir()
                        .join("arroba-remote-prompt-attachments")
                        .join(&leased_agent.backing_session_id)
                        .join(&leased_agent.id);
                    fs::create_dir_all(&root).map_err(|error| DaemonError::LocalTransport {
                        operation: "create remote prompt attachment directory",
                        message: error.to_string(),
                    })?;
                    let path = root.join(format!(
                        "{}-{}-{}",
                        crate::session::unix_epoch_ms(),
                        index,
                        filename
                    ));
                    fs::write(&path, bytes).map_err(|error| DaemonError::LocalTransport {
                        operation: "write remote prompt attachment",
                        message: error.to_string(),
                    })?;
                    Ok(crate::session::PromptAttachment::new(
                        format!("file://{}", path.display()),
                        attachment.mime,
                        Some(filename),
                    ))
                } else {
                    Ok(crate::session::PromptAttachment::new(
                        attachment.url,
                        attachment.mime,
                        attachment.filename,
                    ))
                }
            })
            .collect()
    }

    pub fn drain_leased_runtime_projection(
        &mut self,
        leased_agent_id: &str,
        provider_run_id: &str,
        pump_output: bool,
    ) -> Result<Option<(String, RelayPeerEvent)>, DaemonError> {
        let leased_agent = self
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        let lease = self
            .execution_leases
            .get(&leased_agent.lease_id)
            .cloned()
            .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                lease_id: leased_agent.lease_id.clone(),
            })?;
        if pump_output {
            let _ = provider_output::pump_terminal_output_for_attachment(
                self,
                &leased_agent.backing_session_id,
                &leased_agent.backing_attachment_id,
            )?;
        }
        let output_chunks = self
            .terminal
            .drain_output_records(
                &leased_agent.backing_session_id,
                &leased_agent.backing_attachment_id,
            )
            .into_iter()
            .map(|record| RelayProjectedOutputChunk {
                kind: record.kind,
                merge_key: record.merge_key,
                bytes: record.bytes,
            })
            .collect::<Vec<_>>();
        let notices = self
            .terminal
            .drain_notice_records(
                &leased_agent.backing_session_id,
                &leased_agent.backing_attachment_id,
            )
            .into_iter()
            .map(|record| record.message)
            .collect::<Vec<_>>();
        let completions = self
            .terminal
            .drain_completion_records(
                &leased_agent.backing_session_id,
                &leased_agent.backing_attachment_id,
            )
            .into_iter()
            .map(|record| RelayProjectedCompletion {
                message_id: record.message_id,
                completed_at_ms: record.completed_at_ms,
            })
            .collect::<Vec<_>>();
        if output_chunks.is_empty() && notices.is_empty() && completions.is_empty() {
            return Ok(None);
        }
        Ok(Some((
            lease.home_kernel_id,
            RelayPeerEvent::LeasedRuntimeProjection {
                home_session_id: lease.home_session_id,
                home_agent_id: lease.home_agent_id,
                provider_run_id: provider_run_id.to_string(),
                output_chunks,
                notices,
                completions,
            },
        )))
    }

    pub fn pump_leased_runtime_projections(
        &mut self,
    ) -> Result<Vec<(String, RelayPeerEvent)>, DaemonError> {
        let leased_agents = self.leased_agents.values().cloned().collect::<Vec<_>>();
        let mut events = Vec::new();
        for leased_agent in leased_agents {
            let Some(provider_run_id) = self
                .providers
                .get_run_for_agent(
                    &leased_agent.backing_session_id,
                    &leased_agent.backing_agent_id,
                )
                .map(|run| run.id().to_string())
            else {
                continue;
            };
            let _ = provider_output::ProviderOutputPump::new(self).pump_provider_output(
                provider_output::ProviderOutputPumpRequest {
                    session_id: &leased_agent.backing_session_id,
                    provider_run_id: &provider_run_id,
                    recipient_attachment_ids: vec![leased_agent.backing_attachment_id.clone()],
                },
            )?;
            if let Some(event) =
                self.drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)?
            {
                events.push(event);
            }
        }
        Ok(events)
    }

    pub fn project_remote_runtime_projection(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
        output_chunks: Vec<RelayProjectedOutputChunk>,
        notices: Vec<String>,
        completions: Vec<RelayProjectedCompletion>,
    ) -> Result<(), DaemonError> {
        let _ = self.sessions.get_session(session_id)?;
        let recipient_attachment_ids = self.attachments.list_session_attachment_ids(session_id);
        let saw_completion = !completions.is_empty();
        for chunk in output_chunks {
            self.terminal.fan_out_output(
                session_id,
                provider_run_id,
                Some(agent_id),
                chunk.kind.clone(),
                chunk.merge_key.clone(),
                recipient_attachment_ids.clone(),
                &chunk.bytes,
            );
            if chunk.kind != TerminalOutputKind::PromptEcho {
                self.append_history_entry(
                    session_id,
                    SessionHistoryEntry::provider_output(
                        session_id,
                        provider_run_id,
                        Some(agent_id),
                        chunk.kind,
                        chunk.merge_key,
                        String::from_utf8_lossy(&chunk.bytes).into_owned(),
                    ),
                );
            }
        }
        for notice in notices {
            self.terminal.record_notice(
                session_id,
                Some(provider_run_id),
                Some(agent_id),
                recipient_attachment_ids.clone(),
                notice.clone(),
            );
            self.append_history_entry(
                session_id,
                SessionHistoryEntry::notice(
                    session_id,
                    Some(provider_run_id),
                    Some(agent_id),
                    notice,
                ),
            );
        }
        for completion in completions {
            self.terminal.record_assistant_message_completion(
                session_id,
                provider_run_id,
                Some(agent_id),
                recipient_attachment_ids.clone(),
                &completion.message_id,
                completion.completed_at_ms,
            );
        }
        if saw_completion
            && self
                .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
                .is_some()
        {
            let remote_execution = self.agents.get_agent(agent_id)?.remote_execution().cloned();
            let completed = self.prompt_owner_complete_active_prompt_only(session_id, agent_id)?;
            self.complete_workflow_prompt_from_runtime(
                session_id,
                &completed,
                Some(provider_run_id),
            )?;
            if let Some(remote_execution) = remote_execution {
                if self
                    .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
                    .is_none()
                {
                    let started_next = self.advance_next_queued_prompt_remote(
                        session_id,
                        agent_id,
                        &remote_execution.worker_kernel_id,
                        &remote_execution.leased_agent_id,
                    )?;
                    if started_next.is_none() {
                        self.sync_focused_provider_run_if_idle(session_id)?;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn execution_lease_count(&self) -> usize {
        self.execution_leases.len()
    }

    pub fn leased_agent_count(&self) -> usize {
        self.leased_agents.len()
    }

    fn ensure_attachment_can_run_capability(
        &self,
        session_id: &str,
        attachment: &RuntimeAttachment,
        capability: &'static str,
    ) -> Result<(), DaemonError> {
        if matches!(
            attachment.capability_level(),
            crate::attachment::ClientCapabilityLevel::FullTerminal
                | crate::attachment::ClientCapabilityLevel::InteractiveStructured
        ) {
            Ok(())
        } else {
            Err(DaemonError::AttachmentCapabilityDenied {
                session_id: session_id.to_string(),
                attachment_id: attachment.id().to_string(),
                capability,
            })
        }
    }

    pub(crate) fn capability_worktree_root(
        &self,
        session_id: &str,
        attachment_id: &str,
        capability: &'static str,
    ) -> Result<PathBuf, DaemonError> {
        Ok(self
            .capability_context(session_id, attachment_id, capability)?
            .worktree_root)
    }

    pub(crate) fn capability_context(
        &self,
        session_id: &str,
        attachment_id: &str,
        capability: &'static str,
    ) -> Result<CapabilityRuntimeContext, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let attachment = self.ensure_attachment_in_session(session_id, attachment_id)?;
        self.ensure_attachment_can_run_capability(session_id, &attachment, capability)?;
        Ok(CapabilityRuntimeContext {
            workspace_id: session.workspace_id().to_string(),
            worktree_root: PathBuf::from(session.worktree_id()),
        })
    }

    pub fn relay_registration(&mut self) -> DaemonRegistration {
        let available_providers = self.providers.registry().registered_adapter_keys();
        DaemonRegistration {
            auth_token: self.config.relay_token.clone().unwrap_or_default(),
            daemon_id: self.config.daemon_id.clone(),
            machine_id: self.config.host_machine_id.clone(),
            machine_alias: self.config.host_machine_alias.clone(),
            os_name: Some(self.config.os_name.clone()),
            kernel_started_at_ms: self.started_at_ms,
            daemon_alias: self.config.daemon_alias.clone(),
            kernel_alias: self.config.daemon_alias.clone(),
            public_key: self.config.relay_public_key.clone(),
            capabilities: vec![
                "kernel_websocket".to_string(),
                "relay_request_proxy".to_string(),
                "relay_peer_transport".to_string(),
                "execution_lease_management".to_string(),
            ],
            available_providers,
            accepting_remote_leases: self.config.accept_remote_leases,
            leased_agent_count: self.leased_agent_count() as u32,
            local_session_count: self.sessions().list_sessions().len() as u32,
        }
    }

    fn spawn_remote_agent(
        &mut self,
        request: CreateAgentRequest,
        machine_ref: &str,
    ) -> Result<AgentInstance, DaemonError> {
        let worker_kernel =
            self.select_remote_kernel_for_machine(machine_ref, &request.provider)?;
        let agent = self.agents.create_agent(request, &mut self.sessions)?;
        let remote_setup = self.bind_remote_agent_to_worker(&agent, &worker_kernel);
        if remote_setup.is_err() {
            let _ = self.agents.destroy_agent(agent.id(), &mut self.sessions);
        }
        remote_setup
    }

    fn bind_remote_agent_to_worker(
        &mut self,
        agent: &AgentInstance,
        worker_kernel: &RelayKernelPresence,
    ) -> Result<AgentInstance, DaemonError> {
        let target = ClientTarget {
            daemon_id: Some(worker_kernel.kernel_id.clone()),
            daemon_alias: None,
        };
        let lease = match self.block_on_relay_future(send_peer_request_via_temporary_connection(
            &self.config,
            target.clone(),
            RelayPeerRequest::CreateExecutionLease {
                home_kernel_id: self.config.daemon_id.clone(),
                home_session_id: agent.session_id().to_string(),
                home_agent_id: agent.id().to_string(),
            },
        ))? {
            RelayPeerResponse::ExecutionLeaseCreated { lease } => lease,
            other => {
                return Err(DaemonError::LocalTransport {
                    operation: "create remote execution lease",
                    message: format!("unexpected peer response: {other:?}"),
                });
            }
        };
        let leased_agent =
            match self.block_on_relay_future(send_peer_request_via_temporary_connection(
                &self.config,
                target.clone(),
                RelayPeerRequest::SpawnLeasedAgent {
                    lease_id: lease.id.clone(),
                    provider: agent.provider().to_string(),
                    model: agent.model().map(ToOwned::to_owned),
                    effort: agent.effort().map(ToOwned::to_owned),
                },
            )) {
                Ok(RelayPeerResponse::LeasedAgentSpawned { leased_agent }) => leased_agent,
                Ok(other) => {
                    let _ = self.block_on_relay_future(send_peer_request_via_temporary_connection(
                        &self.config,
                        target,
                        RelayPeerRequest::DestroyExecutionLease {
                            lease_id: lease.id.clone(),
                        },
                    ));
                    return Err(DaemonError::LocalTransport {
                        operation: "spawn remote leased agent",
                        message: format!("unexpected peer response: {other:?}"),
                    });
                }
                Err(error) => {
                    let _ = self.block_on_relay_future(send_peer_request_via_temporary_connection(
                        &self.config,
                        target,
                        RelayPeerRequest::DestroyExecutionLease {
                            lease_id: lease.id.clone(),
                        },
                    ));
                    return Err(error);
                }
            };
        self.agents.bind_remote_execution(
            agent.id(),
            RemoteAgentBinding {
                worker_kernel_id: worker_kernel.kernel_id.clone(),
                worker_machine_id: worker_kernel.machine_id.clone(),
                execution_lease_id: lease.id,
                leased_agent_id: leased_agent.id,
            },
        )
    }

    fn select_remote_kernel_for_machine(
        &self,
        machine_ref: &str,
        provider: &str,
    ) -> Result<RelayKernelPresence, DaemonError> {
        let machine_ref = crate::config::DaemonConfig::resolve_registered_machine_ref(machine_ref)
            .unwrap_or_else(|| machine_ref.to_string());
        let kernels = self.block_on_relay_future(
            relay_discovery::list_live_kernels_for_machine(&self.config, &machine_ref),
        )?;
        kernels
            .into_iter()
            .filter(|kernel| kernel.accepting_remote_leases)
            .filter(|kernel| {
                kernel
                    .available_providers
                    .iter()
                    .any(|candidate| candidate == provider)
            })
            .min_by_key(|kernel| {
                (
                    kernel.leased_agent_count,
                    kernel.local_session_count,
                    kernel.kernel_id.clone(),
                )
            })
            .ok_or_else(|| DaemonError::NoRemoteKernelAvailable {
                machine_ref,
                provider: provider.to_string(),
            })
    }

    pub(crate) fn block_on_relay_future<F, T>(&self, future: F) -> Result<T, DaemonError>
    where
        F: std::future::Future<Output = Result<T, DaemonError>>,
    {
        if let Ok(handle) = Handle::try_current() {
            tokio::task::block_in_place(|| handle.block_on(future))
        } else {
            Runtime::new()
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "create relay runtime",
                    message: error.to_string(),
                })?
                .block_on(future)
        }
    }

    pub fn startup_message(&self) -> String {
        format!(
            "arroba daemon {} ready on machine {} ({})",
            self.config.daemon_id,
            self.config.host_machine_id,
            self.config.kernel_websocket_url()
        )
    }

    pub fn shutdown_cleanup(&mut self) -> Result<(), DaemonError> {
        let session_ids = self
            .sessions
            .list_sessions()
            .into_iter()
            .map(|session| session.id().to_string())
            .collect::<Vec<_>>();
        let mut first_error = None;

        for session_id in session_ids {
            if let Err(error) = self.end_session(&session_id) {
                crate::logging::error_with_fields(
                    "daemon.shutdown",
                    "failed to end session during daemon shutdown",
                    serde_json::json!({
                        "session_id": session_id,
                        "error": error.to_string(),
                    }),
                );
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }

        if let Some(error) = first_error {
            return Err(error);
        }

        Ok(())
    }

    pub async fn run(self) -> Result<(), DaemonError> {
        let app = Arc::new(tokio::sync::Mutex::new(self));
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let relay_state = {
            let app = app.lock().await;
            app.relay_client_state()
        };
        let relay_task = tokio::spawn(crate::transport::relay_client::run_daemon_relay_connector(
            Arc::clone(&app),
            relay_state,
            shutdown_rx,
        ));

        let result =
            crate::kernel_transport::run_kernel_websocket_server(Arc::clone(&app), async {
                let _ = tokio::signal::ctrl_c().await;
                let _ = shutdown_tx.send(true);
            })
            .await;

        let _ = shutdown_tx.send(true);
        let _ = relay_task.await;
        result
    }
}

fn prompt_idle_timeout() -> Duration {
    Duration::from_millis(
        std::env::var("ARROBA_PROMPT_IDLE_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(750),
    )
}
