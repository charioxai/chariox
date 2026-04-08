use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

mod prompt_lifecycle;
mod provider_runtime;
mod session_runtime;
mod terminal_fanout;
pub(crate) mod workflow_runtime;

use crate::agent::{AgentInstance, AgentService, CreateAgentRequest};
use crate::attachment::{AttachmentService, RuntimeAttachment};
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
use crate::history::{SessionHistoryEntry, SessionHistoryStore};
use crate::provider::{ProviderProcessService, RuntimeProviderRun};
use crate::pty::PtyManager;
use crate::session::{
    CreateSessionRequest, PromptAttachment, PromptStatus, RuntimeSession, SessionConfigState,
    SessionService,
};
use crate::session_history_page::paginate_session_history;
pub use crate::session_history_page::{
    SessionHistoryCursor, SessionHistoryPage, SessionHistoryPageEntry,
};
use crate::terminal::{TerminalOutputKind, TerminalOutputRecord, TerminalStreamService};

pub struct DaemonApp {
    config: DaemonConfig,
    agents: AgentService,
    attachments: AttachmentService,
    capabilities: ShellCommandService,
    directory_tree: DirectoryTreeService,
    file_capabilities: FileCapabilityService,
    git_capabilities: GitCapabilityService,
    screenshot_capabilities: ScreenshotCapabilityService,
    transfer_capabilities: FileTransferService,
    pty: PtyManager,
    providers: ProviderProcessService,
    pub(crate) tracked_provider_processes: BTreeMap<String, TrackedProviderProcess>,
    pub(crate) tracked_provider_run_processes: BTreeMap<String, String>,
    pub(crate) prompt_activity: BTreeMap<String, ActivePromptState>,
    pub(crate) prompt_idle_timeout: Duration,
    sessions: SessionService,
    history: SessionHistoryStore,
    terminal: TerminalStreamService,
}

#[derive(Debug, Clone)]
pub(crate) struct ActivePromptState {
    pub(crate) last_output_at: Option<Instant>,
    pub(crate) saw_response_content: bool,
    pub(crate) completion_recorded: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct TrackedProviderProcess {
    pub(crate) process_id: String,
    pub(crate) provider: String,
    pub(crate) pid: Option<u32>,
    pub(crate) endpoint_mode: crate::provider::AgentEndpointMode,
    pub(crate) process_label: String,
    pub(crate) started_at_ms: u64,
    pub(crate) owner_provider_run_ids: Vec<String>,
}

impl DaemonApp {
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
            tracked_provider_processes: BTreeMap::new(),
            tracked_provider_run_processes: BTreeMap::new(),
            prompt_activity: BTreeMap::new(),
            prompt_idle_timeout: prompt_idle_timeout(),
            sessions: SessionService::new(&config),
            history: SessionHistoryStore::new(config.session_history_root.clone())?,
            terminal: TerminalStreamService::new(),
            config,
        })
    }

    /// Create a new session with a default agent
    pub fn create_session(
        &mut self,
        request: CreateSessionRequest,
    ) -> Result<(RuntimeSession, AgentInstance), DaemonError> {
        // Create the session
        let session = self.sessions.create_session(request)?;

        // Create default agent automatically (without provider - user launches it separately)
        let agent_request = CreateAgentRequest::new(
            session.id(),
            "default", // Provider will be set when user launches it
        )
        .with_worktree(session.worktree_id());

        let agent = self
            .agents
            .create_agent(agent_request, &mut self.sessions)?;

        crate::logging::info_with_fields(
            "daemon.app",
            "session created with default agent",
            serde_json::json!({
                "session_id": session.id(),
                "agent_id": agent.id(),
                "agent_ref": agent.agent_ref(),
            }),
        );

        Ok((session, agent))
    }

    pub fn config(&self) -> &DaemonConfig {
        &self.config
    }

    pub fn sessions(&self) -> &SessionService {
        &self.sessions
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

    pub fn terminal(&self) -> &TerminalStreamService {
        &self.terminal
    }

    /// Spawn a new agent in a session
    pub fn spawn_agent(
        &mut self,
        request: crate::agent::CreateAgentRequest,
    ) -> Result<AgentInstance, DaemonError> {
        self.agents.create_agent(request, &mut self.sessions)
    }

    /// Destroy an agent
    pub fn destroy_agent(&mut self, agent_id: &str) -> Result<AgentInstance, DaemonError> {
        self.agents.destroy_agent(agent_id, &mut self.sessions)
    }

    /// Focus a specific agent in a session
    pub fn focus_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .agents
            .focus_agent(session_id, agent_id, &mut self.sessions)?;
        if !self.should_defer_provider_run_sync_for_focus_change(session_id, agent_id)? {
            self.sync_active_provider_run_for_agent(session_id, agent_id)?;
        }
        Ok(agent)
    }

    /// Cycle focus to next agent in session
    pub fn cycle_agent_focus(
        &mut self,
        session_id: &str,
    ) -> Result<Option<AgentInstance>, DaemonError> {
        let agent = self.agents.cycle_focus(session_id, &mut self.sessions)?;
        if let Some(focused) = agent.as_ref() {
            if !self.should_defer_provider_run_sync_for_focus_change(session_id, focused.id())? {
                self.sync_active_provider_run_for_agent(session_id, focused.id())?;
            }
        }
        Ok(agent)
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
        self.history.load(&session)
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
        let mut entries = self.session_history(session_id)?;
        if let Some(agent_id) = agent_id {
            entries.retain(|entry| {
                entry.agent_id.is_none() || entry.agent_id.as_deref() == Some(agent_id)
            });
        }
        Ok(paginate_session_history(
            &entries,
            round_count,
            max_chars,
            before_entry_index,
            before_entry_char_offset,
        ))
    }

    pub(crate) fn terminal_mut(&mut self) -> &mut TerminalStreamService {
        &mut self.terminal
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
        let session = self.sessions.get_session(&request.session_id)?;
        let attachment =
            self.ensure_attachment_in_session(&request.session_id, &request.attachment_id)?;
        self.ensure_attachment_can_run_capability(&request.session_id, &attachment, "shell")?;

        let mut request = request;
        request.worktree_root = std::path::PathBuf::from(session.worktree_id());
        self.capabilities.run(request)
    }

    pub fn read_directory_tree(
        &self,
        session_id: &str,
        attachment_id: &str,
        path: Option<PathBuf>,
        max_depth: usize,
    ) -> Result<ReadDirectoryTreeResult, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let attachment = self.ensure_attachment_in_session(session_id, attachment_id)?;
        self.ensure_attachment_can_run_capability(session_id, &attachment, "directory_tree")?;
        self.directory_tree.read_tree(ReadDirectoryTreeRequest::new(
            session_id,
            attachment_id,
            PathBuf::from(session.worktree_id()),
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
        let session = self.sessions.get_session(session_id)?;
        let attachment = self.ensure_attachment_in_session(session_id, attachment_id)?;
        self.ensure_attachment_can_run_capability(session_id, &attachment, "file_read")?;
        self.file_capabilities.read_file(ReadFileRequest::new(
            session_id,
            attachment_id,
            PathBuf::from(session.worktree_id()),
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
        let session = self.sessions.get_session(session_id)?;
        let attachment = self.ensure_attachment_in_session(session_id, attachment_id)?;
        self.ensure_attachment_can_run_capability(session_id, &attachment, "file_edit")?;
        self.file_capabilities.edit_file(EditFileRequest::new(
            session_id,
            attachment_id,
            PathBuf::from(session.worktree_id()),
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
        let session = self.sessions.get_session(session_id)?;
        let attachment = self.ensure_attachment_in_session(session_id, attachment_id)?;
        self.ensure_attachment_can_run_capability(session_id, &attachment, "git_inspect")?;
        self.git_capabilities.inspect(InspectGitRequest::new(
            session_id,
            attachment_id,
            PathBuf::from(session.worktree_id()),
            working_directory,
        ))
    }

    pub fn capture_screenshot(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<CaptureScreenshotResult, DaemonError> {
        let _session = self.sessions.get_session(session_id)?;
        let attachment = self.ensure_attachment_in_session(session_id, attachment_id)?;
        self.ensure_attachment_can_run_capability(session_id, &attachment, "screenshot")?;
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
        let session = self.sessions.get_session(session_id)?;
        let attachment = self.ensure_attachment_in_session(session_id, attachment_id)?;
        self.ensure_attachment_can_run_capability(session_id, &attachment, "transfer_store")?;
        self.transfer_capabilities
            .store_file(StoreTransferredFileRequest::new(
                session_id,
                attachment_id,
                PathBuf::from(session.worktree_id()),
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
        let _ = self.reconcile_provider_run_exit(session_id, provider_run_id)?;
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
        let _ = self.reconcile_provider_run_exit(session_id, provider_run_id)?;
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;

        if provider_run.state() == crate::provider::ProviderRunState::Ended {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: provider_run_id.to_string(),
                state: provider_run.state(),
                operation: "resize terminal",
            });
        }

        if provider_run.endpoint_mode() == crate::provider::AgentEndpointMode::External {
            return Ok(());
        }

        self.pty.resize(provider_run_id, cols, rows)
    }

    pub fn resize_terminal(
        &mut self,
        session_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), DaemonError> {
        let provider_run_id = self
            .sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?
            .to_string();

        self.resize_provider_terminal(session_id, &provider_run_id, cols, rows)
    }

    pub fn pump_terminal_output(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
        self.ensure_attachment_in_session(session_id, attachment_id)?;
        let provider_run_id = self
            .sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?
            .to_string();
        let recipient_attachment_ids = self.attachments.list_session_attachment_ids(session_id);

        let _ =
            self.pump_provider_output(session_id, &provider_run_id, recipient_attachment_ids)?;
        Ok(self
            .terminal
            .drain_output_records(session_id, attachment_id))
    }

    pub fn pump_provider_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
    ) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
        if self.reconcile_provider_run_exit(session_id, provider_run_id)? {
            return Ok(Vec::new());
        }
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
        if provider_run.state() == crate::provider::ProviderRunState::Ended {
            return Ok(Vec::new());
        }
        // Parked runs should not be polled for output
        if provider_run.state() == crate::provider::ProviderRunState::Parked {
            return Ok(Vec::new());
        }

        if provider_run.adapter_key() == "opencode" || provider_run.adapter_key() == "codex" {
            return self.pump_structured_output(
                session_id,
                provider_run_id,
                recipient_attachment_ids,
            );
        }

        let chunks = match self.pty.drain_output(provider_run_id) {
            Ok(chunks) => chunks,
            Err(error) => {
                if self.reconcile_provider_run_exit(session_id, provider_run_id)? {
                    return Ok(Vec::new());
                }
                return Err(error);
            }
        };
        if !chunks.is_empty() {
            crate::transport::flow_control::note_prompt_response_content(self, session_id);
        }
        let exited = self.reconcile_provider_run_exit(session_id, provider_run_id)?;
        if !exited {
            crate::transport::flow_control::maybe_complete_active_prompt(self, session_id)?;
        }

        Ok(chunks
            .into_iter()
            .map(|chunk| {
                self.fan_out_output(
                    session_id,
                    provider_run_id,
                    TerminalOutputKind::ProviderOutput,
                    None,
                    recipient_attachment_ids.clone(),
                    &chunk.bytes,
                )
            })
            .collect())
    }

    pub fn pump_active_prompt_outputs(&mut self) {
        let sessions = self.sessions.list_sessions();
        for session in sessions {
            if session.active_prompt().is_none() {
                continue;
            }
            let Some(provider_run_id) = session.active_provider_run_id() else {
                continue;
            };
            let recipient_attachment_ids =
                self.attachments.list_session_attachment_ids(session.id());
            if let Err(error) =
                self.pump_provider_output(session.id(), provider_run_id, recipient_attachment_ids)
            {
                crate::logging::warn_with_fields(
                    "daemon.app",
                    "background prompt pump failed",
                    serde_json::json!({
                        "session_id": session.id(),
                        "provider_run_id": provider_run_id,
                        "error": error.to_string(),
                    }),
                );
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

    fn ensure_provider_run_in_session(
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
        let _ = self.reconcile_provider_run_exit(session_id, provider_run_id)?;
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
        if provider_run.state() != crate::provider::ProviderRunState::Running {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: provider_run_id.to_string(),
                state: provider_run.state(),
                operation: "submit prompt",
            });
        }

        if self
            .providers
            .submit_structured_prompt(&provider_run, prompt, attachments)?
        {
            return Ok(());
        }

        self.send_provider_input(
            session_id,
            provider_run_id,
            attachment_id,
            prompt.as_bytes(),
        )
    }

    fn pump_structured_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
    ) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
        // Parked runs should not be polled for output
        if provider_run.state() == crate::provider::ProviderRunState::Parked {
            return Ok(Vec::new());
        }
        if provider_run.endpoint_mode() != crate::provider::AgentEndpointMode::External {
            if let Err(error) = self.pty.drain_output(provider_run_id) {
                if self.reconcile_provider_run_exit(session_id, provider_run_id)? {
                    return Ok(Vec::new());
                }
                if !matches!(error, DaemonError::PtyProcessNotFound { .. }) {
                    return Err(error);
                }
            }
        }
        let poll_result = match self.providers.poll_structured_output(provider_run_id) {
            Ok(Some(poll_result)) => poll_result,
            Ok(None) => return Ok(Vec::new()),
            Err(error) => {
                if self.reconcile_provider_run_exit(session_id, provider_run_id)? {
                    return Ok(Vec::new());
                }
                return Err(error);
            }
        };
        for notice in &poll_result.notices {
            self.record_notice(
                session_id,
                Some(provider_run_id),
                recipient_attachment_ids.clone(),
                notice.to_string(),
            );
        }
        let saw_response_content = poll_result.chunks.iter().any(|chunk| {
            matches!(
                chunk.kind,
                TerminalOutputKind::ProviderOutput | TerminalOutputKind::ProviderReasoning
            )
        });
        let saw_runtime_activity = poll_result.chunks.iter().any(|chunk| {
            matches!(
                chunk.kind,
                TerminalOutputKind::ProviderOutput
                    | TerminalOutputKind::ProviderReasoning
                    | TerminalOutputKind::ProviderTool
                    | TerminalOutputKind::ProviderStatus
            )
        });
        if saw_response_content {
            crate::transport::flow_control::note_prompt_response_content(self, session_id);
        } else if saw_runtime_activity {
            crate::transport::flow_control::note_prompt_output(self, session_id);
        }
        for completion in &poll_result.completions {
            self.record_assistant_message_completion(
                session_id,
                provider_run_id,
                recipient_attachment_ids.clone(),
                &completion.message_id,
                completion.completed_at_ms,
            );
            crate::transport::flow_control::mark_prompt_completion_recorded(self, session_id);
        }
        let prompt_completed = poll_result.prompt_completed;
        let records = poll_result
            .chunks
            .into_iter()
            .map(|chunk| {
                self.fan_out_output(
                    session_id,
                    provider_run_id,
                    chunk.kind,
                    chunk.merge_key,
                    recipient_attachment_ids.clone(),
                    &chunk.bytes,
                )
            })
            .collect();
        let exited = self.reconcile_provider_run_exit(session_id, provider_run_id)?;
        if exited {
            return Ok(records);
        }
        let active_prompt_status = self
            .sessions
            .get_session(session_id)?
            .active_prompt()
            .map(|prompt| prompt.status());
        if active_prompt_status == Some(PromptStatus::Cancelling) {
            if prompt_completed {
                let _ = self.finalize_active_prompt_cancellation(session_id)?;
            }
        } else if prompt_completed && active_prompt_status.is_some() {
            let _ = self.complete_active_prompt(session_id)?;
        } else if !prompt_completed && active_prompt_status == Some(PromptStatus::Cancelling) {
            crate::transport::flow_control::maybe_complete_active_prompt(self, session_id)?;
        }
        Ok(records)
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

    pub fn startup_message(&self) -> String {
        format!(
            "arroba daemon {} ready on machine {} ({})",
            self.config.daemon_id,
            self.config.host_machine_id,
            self.config.kernel_websocket_url()
        )
    }

    pub async fn run(self) -> Result<(), DaemonError> {
        crate::kernel_transport::run_kernel_websocket_server(self, async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
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
