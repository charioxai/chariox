use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::agent::{AgentInstance, AgentService, CreateAgentRequest};
use crate::attachment::{AttachRequest, AttachmentService, RuntimeAttachment};
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
use crate::history::{SessionHistoryEntry, SessionHistoryEntryKind, SessionHistoryStore};
use crate::provider::{
    LaunchProviderRequest, OpenCodePollResult, ProviderProcessService, RuntimeProviderRun,
};
use crate::pty::PtyManager;
use crate::session::{
    CreateSessionRequest, PromptAttachment, PromptCancellation, PromptCompletion, PromptStatus,
    PromptSubmissionOutcome, RuntimeSession, SessionConfigState, SessionService, SessionStatus,
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
    prompt_activity: BTreeMap<String, ActivePromptState>,
    prompt_idle_timeout: Duration,
    sessions: SessionService,
    history: SessionHistoryStore,
    terminal: TerminalStreamService,
}

const DEFAULT_SESSION_HISTORY_ROUND_COUNT: usize = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionHistoryCursor {
    pub before_entry_index: usize,
    pub before_entry_char_offset: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionHistoryPageEntry {
    pub entry_index: usize,
    pub fragment_start: usize,
    pub fragment_end: usize,
    pub total_chars: usize,
    pub entry: SessionHistoryEntry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHistoryPage {
    pub entries: Vec<SessionHistoryPageEntry>,
    pub next_cursor: Option<SessionHistoryCursor>,
}

#[derive(Debug, Clone)]
struct SessionHistorySlice {
    entry_index: usize,
    fragment_start: usize,
    fragment_end: usize,
    total_chars: usize,
    entry: SessionHistoryEntry,
}

#[derive(Debug, Clone)]
struct ActivePromptState {
    last_output_at: Option<Instant>,
}

impl DaemonApp {
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
            "default",  // Provider will be set when user launches it
        ).with_worktree(session.worktree_id());
        
        let agent = self.agents.create_agent(agent_request, &mut self.sessions)?;
        
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
    pub fn destroy_agent(
        &mut self,
        agent_id: &str,
    ) -> Result<AgentInstance, DaemonError> {
        self.agents.destroy_agent(agent_id, &mut self.sessions)
    }

    /// Focus a specific agent in a session
    pub fn focus_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self.agents.focus_agent(session_id, agent_id, &mut self.sessions)?;
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

    pub fn session_history(&self, session_id: &str) -> Result<Vec<SessionHistoryEntry>, DaemonError> {
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
        let mut slices = build_history_slices(
            &entries,
            before_entry_index,
            before_entry_char_offset,
        );

        if slices.is_empty() {
            return Ok(SessionHistoryPage {
                entries: Vec::new(),
                next_cursor: None,
            });
        }

        let mut start_index = history_start_for_recent_user_rounds_in_slices(
            &slices,
            round_count.unwrap_or(DEFAULT_SESSION_HISTORY_ROUND_COUNT),
        );

        if let Some(max_chars) = max_chars {
            start_index = trim_history_slices_to_budget(&mut slices, start_index, max_chars);
        }

        let page_slices = slices.split_off(start_index);
        let next_cursor = page_slices.first().and_then(next_history_cursor_for_slice);

        Ok(SessionHistoryPage {
            entries: page_slices
                .into_iter()
                .map(|slice| SessionHistoryPageEntry {
                    entry_index: slice.entry_index,
                    fragment_start: slice.fragment_start,
                    fragment_end: slice.fragment_end,
                    total_chars: slice.total_chars,
                    entry: slice.entry,
                })
                .collect(),
            next_cursor,
        })
    }

    pub(crate) fn terminal_mut(&mut self) -> &mut TerminalStreamService {
        &mut self.terminal
    }

    pub fn pty(&self) -> &PtyManager {
        &self.pty
    }

    pub fn attach(&mut self, request: AttachRequest) -> Result<RuntimeAttachment, DaemonError> {
        let session_id = request.session_id.clone();
        let client_id = request.client_id.clone();
        let capability_level = format!("{:?}", request.capability_level);
        let replaced_attachment_ids = self
            .attachments
            .list_client_attachments(&client_id)
            .into_iter()
            .map(|attachment| attachment.id().to_string())
            .collect::<Vec<_>>();
        for attachment_id in &replaced_attachment_ids {
            let _ = self.detach(attachment_id)?;
        }
        let attachment = self.attachments.attach(&mut self.sessions, request)?;
        
        // Create default agent if session has no agents (e.g., after session was ended and reattached)
        // Parked/active sessions that were never ended will retain their existing agents
        let session_agents = self.agents.get_session_agents(&session_id);
        if session_agents.is_empty() {
            let worktree_id = self.sessions.get_session(&session_id)?.worktree_id().to_string();
            let agent_request = CreateAgentRequest::new(&session_id, "default")
                .with_worktree(worktree_id);
            let _agent = self.agents.create_agent(agent_request, &mut self.sessions)?;
            crate::logging::info_with_fields(
                "daemon.app",
                "created default agent for session",
                serde_json::json!({
                    "session_id": session_id,
                    "reason": "session had no agents (possibly after being ended and reattached)",
                }),
            );
        }
        
        crate::logging::info_with_fields(
            "daemon.session",
            "attachment joined session",
            serde_json::json!({
                "session_id": session_id,
                "attachment_id": attachment.id(),
                "client_id": client_id,
                "capability_level": capability_level,
                "replaced_attachment_ids": replaced_attachment_ids,
            }),
        );
        Ok(attachment)
    }

    pub fn detach(&mut self, attachment_id: &str) -> Result<RuntimeAttachment, DaemonError> {
        let (attachment, effect) = self
            .attachments
            .detach_with_effect(&mut self.sessions, attachment_id)?;
        let session_after_detach = self.sessions.get_session(attachment.session_id())?;

        if effect.removed_queued_prompt_count > 0 {
            self.record_notice(
                attachment.session_id(),
                None,
                self.attachments
                    .list_session_attachment_ids(attachment.session_id()),
                format!(
                    "Removed {} queued prompt(s) from detached attachment `{}`.",
                    effect.removed_queued_prompt_count, attachment_id
                ),
            );
        }

        if effect.removed_active_prompt {
            self.record_notice(
                attachment.session_id(),
                None,
                self.attachments.list_session_attachment_ids(attachment.session_id()),
                format!(
                    "Removed the active prompt from detached attachment `{}` and advanced the queue.",
                    attachment_id
                ),
            );
            let _ = self.advance_next_queued_prompt(attachment.session_id())?;
        }

        let remaining_attachment_ids = self.attachments.list_session_attachment_ids(attachment.session_id());
        if remaining_attachment_ids.is_empty() {
            if session_after_detach.active_prompt().is_none() {
                let terminated_runs = self
                    .providers
                    .terminate_session_runs(&mut self.sessions, attachment.session_id())?;
                for run in terminated_runs {
                    self.pty.remove_process(run.id())?;
                }
                self.prompt_activity.remove(attachment.session_id());
            }
        }

        crate::logging::info_with_fields(
            "daemon.session",
            "attachment left session",
            serde_json::json!({
                "session_id": attachment.session_id(),
                "attachment_id": attachment.id(),
                "removed_queued_prompts": effect.removed_queued_prompt_count,
                "removed_active_prompt": effect.removed_active_prompt,
                "remaining_attachment_ids": remaining_attachment_ids,
            }),
        );

        Ok(attachment)
    }

    pub fn end_session(&mut self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        let session = self.sessions.get_session(session_id)?;

        if session.status() == SessionStatus::Ended {
            return self.sessions.end_session(session_id);
        }

        let removed_attachments = self.attachments.remove_session_attachments(session_id);
        let terminated_runs = self
            .providers
            .terminate_session_runs(&mut self.sessions, session_id)?;
        let terminated_run_ids = terminated_runs
            .iter()
            .map(|run| run.id().to_string())
            .collect::<Vec<_>>();
        for run in terminated_runs {
            self.pty.remove_process(run.id())?;
        }
        
        // Remove all agents for this session
        let removed_agents = self.agents.remove_session_agents(session_id);
        let removed_agent_ids: Vec<_> = removed_agents
            .iter()
            .map(|agent| format!("{} ({})", agent.agent_ref(), agent.id()))
            .collect();
        
        self.prompt_activity.remove(session_id);
        let ended = self.sessions.end_session(session_id)?;
        crate::logging::info_with_fields(
            "daemon.session",
            "session ended",
            serde_json::json!({
                "session_id": session_id,
                "removed_attachment_ids": removed_attachments.iter().map(|attachment| attachment.id().to_string()).collect::<Vec<_>>(),
                "terminated_provider_run_ids": terminated_run_ids,
                "removed_agents": removed_agent_ids,
            }),
        );
        Ok(ended)
    }

    pub fn resolve_session_ref(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<RuntimeSession, DaemonError> {
        self.sessions.resolve_session_ref(session_ref, workspace_id)
    }

    pub fn delete_session_ref(
        &mut self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<RuntimeSession, DaemonError> {
        let session = self.sessions.resolve_session_ref(session_ref, workspace_id)?;
        let ended = self.end_session(session.id())?;
        let deleted = self.sessions.delete_session(ended.id())?;
        crate::logging::info_with_fields(
            "daemon.session",
            "session deleted",
            serde_json::json!({
                "session_id": deleted.id(),
                "session_alias": deleted.alias(),
            }),
        );
        Ok(deleted)
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
                std::env::temp_dir()
                    .join("arroba-session-artifacts")
                    .join(session_id)
                    .join("screenshots"),
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
                source_path,
                display_name,
            ))
    }

    pub fn launch_provider(
        &mut self,
        mut request: LaunchProviderRequest,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        if request.agent_id.is_none() {
            request.agent_id = self
                .sessions
                .get_session(&request.session_id)?
                .focused_agent_id()
                .map(str::to_string);
        }
        crate::logging::info_with_fields(
            "daemon.app",
            "launching provider run",
            serde_json::json!({
                "adapter_key": request.adapter_key.clone(),
                "agent_id": request.agent_id.clone(),
                "provider": request.provider.clone(),
                "session_id": request.session_id.clone(),
            }),
        );
        if request.adapter_key == "opencode" && request.working_directory.is_none() {
            let agent_worktree = request.agent_id.as_deref().and_then(|agent_id| {
                self.agents
                    .get_agent(agent_id)
                    .ok()
                    .and_then(|agent| agent.worktree_id().map(PathBuf::from))
            });
            request.working_directory = Some(agent_worktree.unwrap_or_else(|| {
                PathBuf::from(
                    self.sessions
                        .get_session(&request.session_id)
                        .map(|session| session.worktree_id().to_string())
                        .unwrap_or_default(),
                )
            }));
        }
        let previous_active_run_id = self
            .sessions
            .get_session(&request.session_id)?
            .active_provider_run_id()
            .map(str::to_owned);
        let recipients = self
            .attachments
            .list_session_attachment_ids(&request.session_id);
        let run = self.providers.launch_run(&mut self.sessions, request)?;
        crate::logging::info_with_fields(
            "daemon.app",
            "spawned provider run metadata",
            serde_json::json!({
                "provider_run_id": run.id(),
                "session_id": run.session_id(),
                "provider": run.provider(),
            }),
        );
        if let Err(error) = self.pty.spawn_for_run(&run) {
            crate::logging::error_with_fields(
                "daemon.app",
                "PTY spawn failed for provider run",
                serde_json::json!({
                    "provider_run_id": run.id(),
                    "session_id": run.session_id(),
                    "error": error.to_string(),
                }),
            );
            let _ = self
                .providers
                .terminate_run(&mut self.sessions, run.session_id(), run.id());
            if let Some(previous_active_run_id) = previous_active_run_id.as_deref() {
                match self.providers.resume_run(
                    &mut self.sessions,
                    run.session_id(),
                    previous_active_run_id,
                ) {
                    Ok(resumed_run) => {
                        self.record_notice(
                            run.session_id(),
                            Some(resumed_run.id()),
                            recipients,
                            format!(
                                "Provider switch failed for session `{}`. Arroba resumed the previous provider run `{}` automatically.",
                                run.session_id(),
                                resumed_run.id()
                            ),
                        );
                    }
                    Err(resume_error) => {
                        self.record_notice(
                            run.session_id(),
                            None,
                            recipients,
                            format!(
                                "Provider switch failed for session `{}` and Arroba could not resume the previous provider run: {}",
                                run.session_id(),
                                resume_error
                            ),
                        );
                    }
                }
            }
            return Err(error);
        }
        crate::logging::info_with_fields(
            "daemon.app",
            "initializing provider runtime",
            serde_json::json!({
                "provider_run_id": run.id(),
                "session_id": run.session_id(),
            }),
        );
        if let Err(error) = self.providers.initialize_runtime(&run) {
            crate::logging::error_with_fields(
                "daemon.app",
                "provider runtime initialization failed",
                serde_json::json!({
                    "provider_run_id": run.id(),
                    "session_id": run.session_id(),
                    "error": error.to_string(),
                }),
            );
            let _ = self.pty.remove_process(run.id());
            self.providers.clear_runtime(run.id());
            let _ = self
                .providers
                .terminate_run(&mut self.sessions, run.session_id(), run.id());
            if let Some(previous_active_run_id) = previous_active_run_id.as_deref() {
                let _ = self.providers.resume_run(
                    &mut self.sessions,
                    run.session_id(),
                    previous_active_run_id,
                );
            }
            return Err(error);
        }
        crate::logging::info_with_fields(
            "daemon.app",
            "provider runtime initialized successfully",
            serde_json::json!({
                "provider_run_id": run.id(),
                "session_id": run.session_id(),
            }),
        );
        self.providers.get_run(run.id())
    }

    pub fn submit_prompt(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        prompt: &str,
        attachments: Vec<PromptAttachment>,
    ) -> Result<PromptSubmissionOutcome, DaemonError> {
        self.ensure_attachment_in_session(session_id, attachment_id)?;
        let session_before = self.sessions.get_session(session_id)?;
        
        // Get the focused agent to target the prompt
        let target_agent_id = session_before
            .focused_agent_id()
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "no focused agent".to_string(),
            })?
            .to_string();
        
        let provider_run_id = self.ensure_active_provider_run_for_agent(session_id, &target_agent_id)?;

        self.append_user_prompt_history(session_id, attachment_id, &target_agent_id, prompt, &attachments);

        let (_session, outcome) = self
            .sessions
            .submit_prompt(session_id, attachment_id, &target_agent_id, prompt, attachments.clone())?;

        match &outcome {
            PromptSubmissionOutcome::Started { prompt } => {
                self.echo_prompt_to_other_attachments(
                    session_id,
                    &provider_run_id,
                    prompt.source_attachment_id(),
                    prompt.prompt(),
                    prompt.attachments(),
                );
                if let Err(error) = self.dispatch_prompt_to_provider(
                    session_id,
                    &provider_run_id,
                    prompt.source_attachment_id(),
                    prompt.prompt(),
                    prompt.attachments(),
                ) {
                    let _ = self.sessions.cancel_active_prompt(session_id);
                    self.clear_prompt_activity(session_id);
                    return Err(error);
                }
                self.note_prompt_started(session_id);
            }
            PromptSubmissionOutcome::Queued { prompt } => {
                self.echo_prompt_to_other_attachments(
                    session_id,
                    &provider_run_id,
                    prompt.source_attachment_id(),
                    prompt.prompt(),
                    prompt.attachments(),
                );
                self.record_notice(
                    session_id,
                    Some(&provider_run_id),
                    self.other_attachment_ids(session_id, attachment_id),
                    format!(
                        "A queued message from attachment `{}` was added to session `{}` as `{}`. Queue depth is now {}.",
                        attachment_id,
                        session_id,
                        prompt.id(),
                        session_before.queued_prompts().len() + 1
                    ),
                );
            }
        }

        Ok(outcome)
    }

    fn sync_active_provider_run_for_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let current_active_run_id = self
            .sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_string);

        if let Some(current_active_run_id) = current_active_run_id.as_deref() {
            let active_run = self.providers.get_run(current_active_run_id)?;
            if active_run.agent_instance_id() != Some(agent_id)
                && active_run.state() == crate::provider::ProviderRunState::Running
            {
                self.providers
                    .park_run(&mut self.sessions, session_id, current_active_run_id)?;
            }
        }

        if let Some(agent_run) = self.providers.get_run_for_agent(session_id, agent_id) {
            match agent_run.state() {
                crate::provider::ProviderRunState::Running => {
                    self.sessions
                        .set_active_provider_run(session_id, Some(agent_run.id().to_string()))?;
                }
                crate::provider::ProviderRunState::Parked => {
                    self.providers
                        .resume_run(&mut self.sessions, session_id, agent_run.id())?;
                }
                crate::provider::ProviderRunState::Starting => {
                    self.sessions
                        .set_active_provider_run(session_id, Some(agent_run.id().to_string()))?;
                }
                crate::provider::ProviderRunState::Ended => {
                    self.sessions.set_active_provider_run(session_id, None)?;
                }
            }
        } else {
            self.sessions.set_active_provider_run(session_id, None)?;
        }

        Ok(())
    }

    fn should_defer_provider_run_sync_for_focus_change(
        &self,
        session_id: &str,
        target_agent_id: &str,
    ) -> Result<bool, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let Some(active_provider_run_id) = session.active_provider_run_id().map(str::to_string) else {
            return Ok(false);
        };
        let active_run = self.providers.get_run(&active_provider_run_id)?;
        if active_run.agent_instance_id() == Some(target_agent_id)
            || active_run.state() != crate::provider::ProviderRunState::Running
        {
            return Ok(false);
        }

        Ok(
            session.active_prompt().is_some()
                || session.agents().iter().any(|agent| agent.is_processing()),
        )
    }

    fn sync_focused_provider_run_if_idle(&mut self, session_id: &str) -> Result<(), DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        if session.active_prompt().is_some() || session.agents().iter().any(|agent| agent.is_processing()) {
            return Ok(());
        }

        let focused_agent_id = session.focused_agent_id().map(str::to_string);
        if let Some(focused_agent_id) = focused_agent_id {
            self.sync_active_provider_run_for_agent(session_id, &focused_agent_id)?;
        } else {
            self.sessions.set_active_provider_run(session_id, None)?;
        }

        Ok(())
    }

    fn ensure_active_provider_run_for_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<String, DaemonError> {
        self.sync_active_provider_run_for_agent(session_id, agent_id)?;
        if let Some(provider_run_id) = self
            .sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_string)
        {
            return Ok(provider_run_id);
        }

        Err(DaemonError::NoActiveProviderRun {
            session_id: session_id.to_string(),
        })
    }

    pub fn complete_active_prompt(
        &mut self,
        session_id: &str,
    ) -> Result<PromptCompletion, DaemonError> {
        let (_session, completed) = self.sessions.complete_active_prompt_only(session_id)?;
        self.clear_prompt_activity(session_id);
        let started_next = if self
            .sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .is_some()
        {
            self.advance_next_queued_prompt(session_id)?
        } else {
            None
        };
        if started_next.is_none() {
            self.sync_focused_provider_run_if_idle(session_id)?;
        }

        Ok(PromptCompletion {
            completed,
            started_next,
        })
    }

    pub fn cancel_active_prompt(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<PromptCancellation, DaemonError> {
        self.ensure_attachment_in_session(session_id, attachment_id)?;
        let active_prompt = self
            .sessions
            .get_session(session_id)?
            .active_prompt()
            .cloned()
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        if active_prompt.status() == PromptStatus::Cancelling {
            return Ok(PromptCancellation {
                prompt: active_prompt,
                started_next: None,
            });
        }
        let provider_run_id = self
            .sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?
            .to_string();
        let provider_run = self.ensure_provider_run_in_session(session_id, &provider_run_id)?;

        if !self.providers.abort_structured_runtime(&provider_run_id)? {
            self.send_provider_input(session_id, &provider_run_id, attachment_id, b"\x03")?;
        }

        let (_session, prompt) = self.sessions.begin_cancelling_active_prompt(session_id)?;
        self.note_prompt_settlement_requested(session_id);
        self.record_notice(
            session_id,
            Some(&provider_run_id),
            self.other_attachment_ids(session_id, attachment_id),
            format!(
                "Attachment `{attachment_id}` requested cancellation of active prompt `{}` on provider run `{}`.",
                active_prompt.id(),
                provider_run.id()
            ),
        );

        Ok(PromptCancellation {
            prompt,
            started_next: None,
        })
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
        self.ensure_attachment_in_session(session_id, attachment_id)?;
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

        let _ = self.pump_provider_output(session_id, &provider_run_id, recipient_attachment_ids)?;
        Ok(self.terminal.drain_output_records(session_id, attachment_id))
    }

    pub fn pump_provider_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
    ) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;

        if provider_run.adapter_key() == "opencode" {
            return self.pump_opencode_output(
                session_id,
                provider_run_id,
                recipient_attachment_ids,
            );
        }

        let chunks = self.pty.drain_output(provider_run_id)?;
        if !chunks.is_empty() {
            self.note_prompt_output(session_id);
        }
        let exited = self.reconcile_provider_run_exit(session_id, provider_run_id)?;
        if !exited {
            self.maybe_complete_active_prompt(session_id)?;
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

    fn other_attachment_ids(&self, session_id: &str, source_attachment_id: &str) -> Vec<String> {
        self.attachments
            .list_session_attachment_ids(session_id)
            .into_iter()
            .filter(|attachment_id| attachment_id != source_attachment_id)
            .collect()
    }

    fn append_user_prompt_history(
        &self,
        session_id: &str,
        source_attachment_id: &str,
        agent_id: &str,
        prompt: &str,
        attachments: &[PromptAttachment],
    ) {
        self.append_history_entry(
            session_id,
            SessionHistoryEntry::user_prompt(
                session_id,
                source_attachment_id,
                agent_id,
                render_prompt_transcript(prompt, attachments),
            ),
        );
    }

    fn fan_out_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        kind: TerminalOutputKind,
        merge_key: Option<String>,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        let agent_id = self
            .providers
            .get_run(provider_run_id)
            .ok()
            .and_then(|run| run.agent_instance_id().map(str::to_string));
        let record = self.terminal.fan_out_output(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            kind.clone(),
            merge_key.clone(),
            recipient_attachment_ids,
            bytes,
        );
        if kind != TerminalOutputKind::PromptEcho {
            self.append_history_entry(
                session_id,
                SessionHistoryEntry::provider_output(
                    session_id,
                    provider_run_id,
                    agent_id.as_deref(),
                    kind,
                    merge_key,
                    String::from_utf8_lossy(bytes).into_owned(),
                ),
            );
        }
        record
    }

    fn record_notice(
        &mut self,
        session_id: &str,
        provider_run_id: Option<&str>,
        recipient_attachment_ids: Vec<String>,
        message: impl Into<String>,
    ) -> crate::terminal::RuntimeNoticeRecord {
        let message = message.into();
        let agent_id = provider_run_id.and_then(|run_id| {
            self.providers
                .get_run(run_id)
                .ok()
                .and_then(|run| run.agent_instance_id().map(str::to_string))
        });
        let record = self.terminal.record_notice(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            recipient_attachment_ids,
            message.clone(),
        );
        self.append_history_entry(
            session_id,
            SessionHistoryEntry::notice(session_id, provider_run_id, agent_id.as_deref(), message),
        );
        record
    }

    fn append_history_entry(&self, session_id: &str, entry: SessionHistoryEntry) {
        let session = match self.sessions.get_session(session_id) {
            Ok(session) => session,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.history",
                    "skipping history append because session lookup failed",
                    serde_json::json!({
                        "session_id": session_id,
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
        if let Err(error) = self.history.append(&session, &entry) {
            crate::logging::warn_with_fields(
                "daemon.history",
                "failed to append session history",
                serde_json::json!({
                    "session_id": session_id,
                    "error": error.to_string(),
                }),
            );
        }
    }

    fn dispatch_prompt_to_provider(
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

    fn pump_opencode_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
    ) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
        if let Err(error) = self.pty.drain_output(provider_run_id) {
            if self.reconcile_provider_run_exit(session_id, provider_run_id)? {
                return Ok(Vec::new());
            }
            return Err(error);
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
                notice.clone(),
            );
        }
        let prompt_completed = poll_result.prompt_completed;
        let provider_idle = poll_result.provider_idle;
        let records = self.render_opencode_output(
            session_id,
            provider_run_id,
            recipient_attachment_ids,
            poll_result,
        );
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
            if provider_idle || prompt_completed {
                let _ = self.finalize_active_prompt_cancellation(session_id)?;
            }
        } else if prompt_completed && active_prompt_status.is_some() {
            let _ = self.complete_active_prompt(session_id)?;
        }
        Ok(records)
    }

    fn render_opencode_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
        poll_result: OpenCodePollResult,
    ) -> Vec<TerminalOutputRecord> {
        if poll_result.chunks.iter().any(|chunk| {
            matches!(
                chunk.kind,
                TerminalOutputKind::ProviderOutput | TerminalOutputKind::ProviderReasoning
            )
        }) {
            self.note_prompt_output(session_id);
        }

        poll_result
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
            .collect()
    }

    fn echo_prompt_to_other_attachments(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        source_attachment_id: &str,
        prompt: &str,
        attachments: &[PromptAttachment],
    ) {
        let recipient_attachment_ids = self.other_attachment_ids(session_id, source_attachment_id);
        if recipient_attachment_ids.is_empty() {
            return;
        }
        let mut bytes = render_prompt_transcript(prompt, attachments).into_bytes();
        if !bytes.ends_with(b"\n") {
            bytes.push(b'\n');
        }
        self.fan_out_output(
            session_id,
            provider_run_id,
            TerminalOutputKind::PromptEcho,
            None,
            recipient_attachment_ids,
            &bytes,
        );
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

    fn advance_next_queued_prompt(
        &mut self,
        session_id: &str,
    ) -> Result<Option<crate::session::PromptQueueItem>, DaemonError> {
        let provider_run_id = self
            .sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?
            .to_string();

        loop {
            let (_session, next_candidate) = self.sessions.pop_next_queued_prompt(session_id)?;
            let Some(next) = next_candidate else {
                return Ok(None);
            };

            if let Err(error) =
                self.ensure_attachment_in_session(session_id, next.source_attachment_id())
            {
                self.record_notice(
                    session_id,
                    Some(&provider_run_id),
                    self.attachments.list_session_attachment_ids(session_id),
                    format!(
                        "Skipped queued prompt `{}` because its source attachment is no longer active: {}",
                        next.id(),
                        error
                    ),
                );
                continue;
            }

            if let Err(error) = self.dispatch_prompt_to_provider(
                session_id,
                &provider_run_id,
                next.source_attachment_id(),
                next.prompt(),
                next.attachments(),
            ) {
                self.record_notice(
                    session_id,
                    Some(&provider_run_id),
                    self.attachments.list_session_attachment_ids(session_id),
                    format!(
                        "Skipped queued prompt `{}` after PTY delivery failure: {}",
                        next.id(),
                        error
                    ),
                );
                continue;
            }

            let active = self.sessions.activate_prompt(session_id, next)?.1;
            self.note_prompt_started(session_id);
            return Ok(Some(active));
        }
    }

    fn note_prompt_started(&mut self, session_id: &str) {
        self.prompt_activity.insert(
            session_id.to_string(),
            ActivePromptState {
                last_output_at: None,
            },
        );
    }

    fn note_prompt_output(&mut self, session_id: &str) {
        if let Some(state) = self.prompt_activity.get_mut(session_id) {
            state.last_output_at = Some(Instant::now());
        }
    }

    fn clear_prompt_activity(&mut self, session_id: &str) {
        self.prompt_activity.remove(session_id);
    }

    fn note_prompt_settlement_requested(&mut self, session_id: &str) {
        self.prompt_activity
            .entry(session_id.to_string())
            .and_modify(|state| state.last_output_at = Some(Instant::now()))
            .or_insert(ActivePromptState {
                last_output_at: Some(Instant::now()),
            });
    }

    fn maybe_complete_active_prompt(&mut self, session_id: &str) -> Result<(), DaemonError> {
        let should_complete = self
            .prompt_activity
            .get(session_id)
            .and_then(|state| state.last_output_at)
            .map(|last_output_at| last_output_at.elapsed() >= self.prompt_idle_timeout)
            .unwrap_or(false);

        if !should_complete {
            return Ok(());
        }

        if self
            .sessions
            .get_session(session_id)?
            .active_prompt()
            .is_none()
        {
            self.clear_prompt_activity(session_id);
            return Ok(());
        }

        if self
            .sessions
            .get_session(session_id)?
            .active_prompt()
            .map(|prompt| prompt.status())
            == Some(PromptStatus::Cancelling)
        {
            let _ = self.finalize_active_prompt_cancellation(session_id)?;
        } else {
            let _ = self.complete_active_prompt(session_id)?;
        }
        Ok(())
    }

    fn reconcile_provider_run_exit(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
        if provider_run.state() == crate::provider::ProviderRunState::Ended {
            if self
                .sessions
                .get_session(session_id)?
                .active_provider_run_id()
                == Some(provider_run_id)
            {
                self.sessions.set_active_provider_run(session_id, None)?;
            }
            let _ = self.pty.remove_process(provider_run_id)?;
            self.providers.clear_runtime(provider_run_id);
            return Ok(true);
        }

        let process_running = match self.pty.poll_process_state(provider_run_id) {
            Ok(crate::pty::PtyProcessState::Running) => true,
            Ok(crate::pty::PtyProcessState::Exited) => false,
            Err(DaemonError::PtyProcessNotFound { .. }) => {
                if self.providers.runtime_is_healthy(provider_run_id) {
                    return Ok(false);
                }
                return Ok(true);
            }
            Err(error) => return Err(error),
        };
        if process_running {
            return Ok(false);
        }
        if self.providers.runtime_is_healthy(provider_run_id) {
            let _ = self.pty.remove_process(provider_run_id);
            crate::logging::info_with_fields(
                "daemon.app",
                "provider pty exited but shared runtime remains healthy",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                }),
            );
            return Ok(false);
        }

        let had_active_prompt = self
            .sessions
            .get_session(session_id)?
            .active_prompt()
            .is_some();
        let ended_run =
            self.providers
                .mark_run_ended(&mut self.sessions, session_id, provider_run_id)?;
        let _ = self.pty.remove_process(provider_run_id)?;

        if had_active_prompt {
            let active_prompt_status = self
                .sessions
                .get_session(session_id)?
                .active_prompt()
                .map(|prompt| prompt.status());
            if active_prompt_status == Some(PromptStatus::Cancelling) {
                let _ = self
                    .sessions
                    .finalize_active_prompt_cancellation(session_id)?;
            } else {
                let _ = self.sessions.complete_active_prompt_only(session_id)?;
            }
            self.clear_prompt_activity(session_id);
        }
        self.providers.clear_runtime(provider_run_id);

        self.record_notice(
            session_id,
            Some(provider_run_id),
            self.attachments.list_session_attachment_ids(session_id),
            format!(
                "Provider run `{}` for `{}` ended unexpectedly. {}",
                provider_run_id,
                ended_run.provider(),
                if had_active_prompt {
                    "The active prompt was closed without starting the queued backlog."
                } else {
                    "No active prompt was running."
                }
            ),
        );

        Ok(true)
    }

    fn finalize_active_prompt_cancellation(
        &mut self,
        session_id: &str,
    ) -> Result<PromptCancellation, DaemonError> {
        let (_session, prompt) = self
            .sessions
            .finalize_active_prompt_cancellation(session_id)?;
        self.clear_prompt_activity(session_id);
        let started_next = if self
            .sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .is_some()
        {
            self.advance_next_queued_prompt(session_id)?
        } else {
            None
        };
        if started_next.is_none() {
            self.sync_focused_provider_run_if_idle(session_id)?;
        }

        Ok(PromptCancellation {
            prompt,
            started_next,
        })
    }

    pub fn startup_message(&self) -> String {
        format!(
            "arroba daemon {} ready on machine {} ({})",
            self.config.daemon_id,
            self.config.host_machine_id,
            self.config.local_socket_path.display()
        )
    }

    pub async fn run(self) -> Result<(), DaemonError> {
        crate::local::run_local_ipc_server(self, async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
    }
}

fn render_prompt_transcript(prompt: &str, attachments: &[PromptAttachment]) -> String {
    let text = prompt.trim_end_matches('\n');
    let _ = attachments;
    if text.is_empty() {
        String::new()
    } else {
        format!("{text}\n")
    }
}

#[cfg(test)]
fn history_start_for_recent_user_rounds(
    entries: &[SessionHistoryEntry],
    round_count: usize,
) -> usize {
    if round_count == 0 || entries.is_empty() {
        return 0;
    }

    let mut seen_user_prompts = 0usize;
    for (index, entry) in entries.iter().enumerate().rev() {
        if entry.kind == SessionHistoryEntryKind::UserPrompt {
            seen_user_prompts += 1;
            if seen_user_prompts == round_count {
                return index;
            }
        }
    }

    0
}

fn history_start_for_recent_user_rounds_in_slices(
    entries: &[SessionHistorySlice],
    round_count: usize,
) -> usize {
    if round_count == 0 || entries.is_empty() {
        return 0;
    }

    let mut seen_user_prompts = 0usize;
    for (index, entry) in entries.iter().enumerate().rev() {
        if entry.entry.kind == SessionHistoryEntryKind::UserPrompt {
            seen_user_prompts += 1;
            if seen_user_prompts == round_count {
                return index;
            }
        }
    }

    0
}

fn build_history_slices(
    entries: &[SessionHistoryEntry],
    before_entry_index: Option<usize>,
    before_entry_char_offset: Option<usize>,
) -> Vec<SessionHistorySlice> {
    let mut slices: Vec<SessionHistorySlice> = entries
        .iter()
        .cloned()
        .enumerate()
        .map(|(entry_index, entry)| {
            let total_chars = entry.text.chars().count();
            SessionHistorySlice {
                entry_index,
                fragment_start: 0,
                fragment_end: total_chars,
                total_chars,
                entry,
            }
        })
        .collect();

    let before_entry_index = before_entry_index.unwrap_or(slices.len()).min(slices.len());
    slices.truncate(before_entry_index);

    if let Some(slice) = slices.last_mut() {
        if let Some(before_entry_char_offset) = before_entry_char_offset {
            let fragment_end = before_entry_char_offset.min(slice.fragment_end);
            slice.fragment_end = fragment_end;
            slice.entry.text = text_prefix(&slice.entry.text, fragment_end);
        }
    }

    while matches!(slices.last(), Some(slice) if slice.fragment_end == 0) {
        slices.pop();
    }

    slices
}

fn trim_history_slices_to_budget(
    slices: &mut [SessionHistorySlice],
    mut start_index: usize,
    max_chars: usize,
) -> usize {
    let mut total_chars: usize = slices[start_index..]
        .iter()
        .map(|slice| slice.fragment_end.saturating_sub(slice.fragment_start))
        .sum();

    while total_chars > max_chars && start_index < slices.len() {
        let slice = &mut slices[start_index];
        let slice_chars = slice.fragment_end.saturating_sub(slice.fragment_start);
        let overflow = total_chars - max_chars;
        if slice_chars > overflow {
            slice.fragment_start += overflow;
            slice.entry.text = text_range(&slice.entry.text, overflow, slice_chars);
            break;
        }
        total_chars -= slice_chars;
        start_index += 1;
    }

    start_index
}

fn next_history_cursor_for_slice(slice: &SessionHistorySlice) -> Option<SessionHistoryCursor> {
    if slice.fragment_start > 0 {
        return Some(SessionHistoryCursor {
            before_entry_index: slice.entry_index + 1,
            before_entry_char_offset: Some(slice.fragment_start),
        });
    }
    if slice.entry_index > 0 {
        return Some(SessionHistoryCursor {
            before_entry_index: slice.entry_index,
            before_entry_char_offset: None,
        });
    }
    None
}

fn text_prefix(text: &str, char_count: usize) -> String {
    text.chars().take(char_count).collect()
}

fn text_range(text: &str, start: usize, char_count: usize) -> String {
    text.chars().skip(start).take(char_count - start).collect()
}

fn prompt_idle_timeout() -> Duration {
    Duration::from_millis(
        std::env::var("ARROBA_PROMPT_IDLE_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(750),
    )
}

#[cfg(test)]
mod tests {
    use super::{history_start_for_recent_user_rounds, SessionHistoryCursor, SessionHistoryPage, SessionHistoryPageEntry};
    use crate::history::{SessionHistoryEntry, SessionHistoryEntryKind};

    #[test]
    fn preserves_four_recent_user_rounds_when_trimming_history() {
        let entries = vec![
            history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 1"),
            history_entry(SessionHistoryEntryKind::ProviderOutput, "answer 1"),
            history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 2"),
            history_entry(SessionHistoryEntryKind::ProviderOutput, "answer 2"),
            history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 3"),
            history_entry(SessionHistoryEntryKind::ProviderOutput, "answer 3"),
            history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 4"),
            history_entry(SessionHistoryEntryKind::ProviderOutput, "answer 4"),
            history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 5"),
            history_entry(SessionHistoryEntryKind::ProviderOutput, "answer 5"),
        ];

        assert_eq!(history_start_for_recent_user_rounds(&entries, 4), 2);
    }

    #[test]
    fn returns_next_before_index_when_older_rounds_exist() {
        let entries = vec![
            history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 1"),
            history_entry(SessionHistoryEntryKind::ProviderOutput, "answer 1"),
            history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 2"),
            history_entry(SessionHistoryEntryKind::ProviderOutput, "answer 2"),
        ];

        let page = page_for_rounds(&entries, 1, None, None, None);

        assert_eq!(
            page,
            SessionHistoryPage {
                entries: vec![
                    page_entry(2, 0, 8, history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 2")),
                    page_entry(3, 0, 8, history_entry(SessionHistoryEntryKind::ProviderOutput, "answer 2")),
                ],
                next_cursor: Some(SessionHistoryCursor {
                    before_entry_index: 2,
                    before_entry_char_offset: None,
                }),
            }
        );
    }

    #[test]
    fn pages_history_from_the_previous_cursor() {
        let entries = vec![
            history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 1"),
            history_entry(SessionHistoryEntryKind::ProviderOutput, "answer 1"),
            history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 2"),
            history_entry(SessionHistoryEntryKind::ProviderOutput, "answer 2"),
            history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 3"),
            history_entry(SessionHistoryEntryKind::ProviderOutput, "answer 3"),
        ];

        let page = page_for_rounds(&entries, 1, None, Some(4), None);

        assert_eq!(
            page,
            SessionHistoryPage {
                entries: vec![
                    page_entry(2, 0, 8, history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 2")),
                    page_entry(3, 0, 8, history_entry(SessionHistoryEntryKind::ProviderOutput, "answer 2")),
                ],
                next_cursor: Some(SessionHistoryCursor {
                    before_entry_index: 2,
                    before_entry_char_offset: None,
                }),
            }
        );
    }

    #[test]
    fn slices_large_history_entries_without_truncation() {
        let page = page_for_rounds(
            &[
                history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 1"),
                history_entry(
                    SessionHistoryEntryKind::ProviderOutput,
                    &"x".repeat(24),
                ),
            ],
            1,
            Some(10),
            None,
            None,
        );

        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].entry.text, "xxxxxxxxxx");
        assert_eq!(page.entries[0].fragment_start, 14);
        assert_eq!(page.entries[0].fragment_end, 24);
        assert_eq!(page.next_cursor, Some(SessionHistoryCursor {
            before_entry_index: 2,
            before_entry_char_offset: Some(14),
        }));
    }

    #[test]
    fn continues_loading_the_older_part_of_a_partial_entry() {
        let entries = vec![
            history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 1"),
            history_entry(SessionHistoryEntryKind::ProviderOutput, &"x".repeat(24)),
        ];

        let page = page_for_rounds(&entries, 1, Some(10), Some(2), Some(14));

        assert_eq!(
            page,
            SessionHistoryPage {
                entries: vec![page_entry(
                    1,
                    4,
                    24,
                    history_entry(SessionHistoryEntryKind::ProviderOutput, "xxxxxxxxxx"),
                )],
                next_cursor: Some(SessionHistoryCursor {
                    before_entry_index: 2,
                    before_entry_char_offset: Some(4),
                }),
            }
        );
    }

    fn page_for_rounds(
        entries: &[SessionHistoryEntry],
        round_count: usize,
        max_chars: Option<usize>,
        before_entry_index: Option<usize>,
        before_entry_char_offset: Option<usize>,
    ) -> SessionHistoryPage {
        let mut slices = super::build_history_slices(entries, before_entry_index, before_entry_char_offset);
        let mut start_index = super::history_start_for_recent_user_rounds_in_slices(&slices, round_count);
        if let Some(max_chars) = max_chars {
            start_index = super::trim_history_slices_to_budget(&mut slices, start_index, max_chars);
        }
        let page_slices = slices.split_off(start_index);
        SessionHistoryPage {
            next_cursor: page_slices.first().and_then(super::next_history_cursor_for_slice),
            entries: page_slices
                .into_iter()
                .map(|slice| SessionHistoryPageEntry {
                    entry_index: slice.entry_index,
                    fragment_start: slice.fragment_start,
                    fragment_end: slice.fragment_end,
                    total_chars: slice.total_chars,
                    entry: slice.entry,
                })
                .collect(),
        }
    }

    fn page_entry(
        entry_index: usize,
        fragment_start: usize,
        total_chars: usize,
        entry: SessionHistoryEntry,
    ) -> SessionHistoryPageEntry {
        SessionHistoryPageEntry {
            entry_index,
            fragment_start,
            fragment_end: fragment_start + entry.text.chars().count(),
            total_chars,
            entry,
        }
    }

    fn history_entry(kind: SessionHistoryEntryKind, text: &str) -> SessionHistoryEntry {
        SessionHistoryEntry {
            session_id: "session-1".to_string(),
            provider_run_id: Some("run-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            source_attachment_id: Some("attachment-1".to_string()),
            kind,
            merge_key: None,
            text: text.to_string(),
            timestamp_ms: 0,
        }
    }
}
