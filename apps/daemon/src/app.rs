use std::collections::BTreeMap;
use std::path::PathBuf;

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
use crate::provider::{LaunchProviderRequest, ProviderProcessService, RuntimeProviderRun};
use crate::pty::PtyManager;
use crate::session::{
    PromptCompletion, PromptSubmissionOutcome, RuntimeSession, SessionConfigState, SessionService,
    SessionStatus,
};
use crate::terminal::{TerminalOutputRecord, TerminalStreamService};

pub struct DaemonApp {
    config: DaemonConfig,
    attachments: AttachmentService,
    capabilities: ShellCommandService,
    directory_tree: DirectoryTreeService,
    file_capabilities: FileCapabilityService,
    git_capabilities: GitCapabilityService,
    screenshot_capabilities: ScreenshotCapabilityService,
    transfer_capabilities: FileTransferService,
    pty: PtyManager,
    providers: ProviderProcessService,
    sessions: SessionService,
    terminal: TerminalStreamService,
}

impl DaemonApp {
    pub fn bootstrap(config: DaemonConfig) -> Result<Self, DaemonError> {
        config.validate()?;

        Ok(Self {
            attachments: AttachmentService::new(),
            capabilities: ShellCommandService::new(),
            directory_tree: DirectoryTreeService::new(),
            file_capabilities: FileCapabilityService::new(),
            git_capabilities: GitCapabilityService::new(),
            screenshot_capabilities: ScreenshotCapabilityService::new(),
            transfer_capabilities: FileTransferService::new(),
            pty: PtyManager::new(),
            providers: ProviderProcessService::new(),
            sessions: SessionService::new(&config),
            terminal: TerminalStreamService::new(),
            config,
        })
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

    pub(crate) fn terminal_mut(&mut self) -> &mut TerminalStreamService {
        &mut self.terminal
    }

    pub fn pty(&self) -> &PtyManager {
        &self.pty
    }

    pub fn attach(&mut self, request: AttachRequest) -> Result<RuntimeAttachment, DaemonError> {
        self.attachments.attach(&mut self.sessions, request)
    }

    pub fn detach(&mut self, attachment_id: &str) -> Result<RuntimeAttachment, DaemonError> {
        let (attachment, effect) = self
            .attachments
            .detach_with_effect(&mut self.sessions, attachment_id)?;

        if effect.removed_queued_prompt_count > 0 {
            self.terminal.record_notice(
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
            self.terminal.record_notice(
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

        Ok(attachment)
    }

    pub fn end_session(&mut self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        let session = self.sessions.get_session(session_id)?;

        if session.status() == SessionStatus::Ended {
            return self.sessions.end_session(session_id);
        }

        self.attachments.remove_session_attachments(session_id);
        let terminated_runs = self
            .providers
            .terminate_session_runs(&mut self.sessions, session_id)?;
        for run in terminated_runs {
            self.pty.remove_process(run.id())?;
        }
        self.sessions.end_session(session_id)
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
        if request.adapter_key == "opencode" && request.working_directory.is_none() {
            request.working_directory = Some(PathBuf::from(
                self.sessions
                    .get_session(&request.session_id)?
                    .worktree_id(),
            ));
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
        if let Err(error) = self.pty.spawn_for_run(&run) {
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
                        self.terminal.record_notice(
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
                        self.terminal.record_notice(
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
        Ok(run)
    }

    pub fn submit_prompt(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        prompt: &str,
    ) -> Result<PromptSubmissionOutcome, DaemonError> {
        self.ensure_attachment_in_session(session_id, attachment_id)?;
        let session_before = self.sessions.get_session(session_id)?;
        let provider_run_id = self
            .sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?
            .to_string();

        let (_session, outcome) = self
            .sessions
            .submit_prompt(session_id, attachment_id, prompt)?;

        match &outcome {
            PromptSubmissionOutcome::Started { prompt } => {
                if let Err(error) = self.send_provider_input(
                    session_id,
                    &provider_run_id,
                    prompt.source_attachment_id(),
                    prompt.prompt().as_bytes(),
                ) {
                    let _ = self.sessions.cancel_active_prompt(session_id, prompt.id());
                    return Err(error);
                }
            }
            PromptSubmissionOutcome::Queued { prompt } => {
                self.terminal.record_notice(
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

    pub fn complete_active_prompt(
        &mut self,
        session_id: &str,
    ) -> Result<PromptCompletion, DaemonError> {
        let (_session, completed) = self.sessions.complete_active_prompt(session_id)?;
        let started_next = self.advance_next_queued_prompt(session_id)?;

        Ok(PromptCompletion {
            completed,
            started_next,
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

        self.terminal.record_notice(
            session_id,
            None,
            self.other_attachment_ids(session_id, attachment_id),
            format!(
                "Session config updated to version {} by attachment `{}`.",
                config.version(),
                attachment_id
            ),
        );

        Ok(config)
    }

    pub fn send_provider_input(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        attachment_id: &str,
        bytes: &[u8],
    ) -> Result<(), DaemonError> {
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
    ) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
        let provider_run_id = self
            .sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?
            .to_string();
        let recipient_attachment_ids = self.attachments.list_session_attachment_ids(session_id);

        self.pump_provider_output(session_id, &provider_run_id, recipient_attachment_ids)
    }

    pub fn pump_provider_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
    ) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;

        if provider_run.state() == crate::provider::ProviderRunState::Ended {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: provider_run_id.to_string(),
                state: provider_run.state(),
                operation: "pump terminal output",
            });
        }

        let chunks = self.pty.drain_output(provider_run_id)?;

        Ok(chunks
            .into_iter()
            .map(|chunk| {
                self.terminal.fan_out_output(
                    session_id,
                    provider_run_id,
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
                self.terminal.record_notice(
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

            if let Err(error) = self.send_provider_input(
                session_id,
                &provider_run_id,
                next.source_attachment_id(),
                next.prompt().as_bytes(),
            ) {
                self.terminal.record_notice(
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

            return Ok(Some(self.sessions.activate_prompt(session_id, next)?.1));
        }
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
