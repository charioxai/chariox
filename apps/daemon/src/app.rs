use crate::attachment::{AttachRequest, AttachmentService, RuntimeAttachment};
use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::provider::{LaunchProviderRequest, ProviderProcessService, RuntimeProviderRun};
use crate::pty::PtyManager;
use crate::session::{RuntimeSession, SessionService, SessionStatus};
use crate::terminal::{TerminalOutputRecord, TerminalStreamService};

pub struct DaemonApp {
    config: DaemonConfig,
    attachments: AttachmentService,
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

    pub fn providers(&self) -> &ProviderProcessService {
        &self.providers
    }

    pub fn providers_mut(&mut self) -> &mut ProviderProcessService {
        &mut self.providers
    }

    pub fn terminal(&self) -> &TerminalStreamService {
        &self.terminal
    }

    pub fn pty(&self) -> &PtyManager {
        &self.pty
    }

    pub fn attach(&mut self, request: AttachRequest) -> Result<RuntimeAttachment, DaemonError> {
        self.attachments.attach(&mut self.sessions, request)
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
            self.pty.remove_process(run.id());
        }
        self.sessions.end_session(session_id)
    }

    pub fn launch_provider(
        &mut self,
        request: LaunchProviderRequest,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let run = self.providers.launch_run(&mut self.sessions, request)?;
        if let Err(error) = self.pty.spawn_for_run(&run) {
            let _ = self
                .providers
                .terminate_run(&mut self.sessions, run.session_id(), run.id());
            return Err(error);
        }
        Ok(run)
    }

    pub fn send_terminal_input(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        bytes: &[u8],
    ) -> Result<(), DaemonError> {
        let session = self.sessions.get_session(session_id)?;

        if session.controller_attachment_id() != Some(attachment_id) {
            return Err(DaemonError::AttachmentIsNotController {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }

        let attachment = self.attachments.get_attachment(attachment_id)?;
        if attachment.session_id() != session_id {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }

        let provider_run_id = session
            .active_provider_run_id()
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?
            .to_string();

        self.terminal
            .record_input(session_id, &provider_run_id, attachment_id, bytes);
        self.pty.write_input(&provider_run_id, bytes)
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

        self.pty.resize(&provider_run_id, cols, rows)
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

    pub fn startup_message(&self) -> String {
        format!(
            "arroba daemon {} ready on machine {}",
            self.config.daemon_id, self.config.host_machine_id
        )
    }

    pub async fn run(&self) -> Result<(), DaemonError> {
        self.wait_for_shutdown_signal().await
    }

    async fn wait_for_shutdown_signal(&self) -> Result<(), DaemonError> {
        tokio::signal::ctrl_c()
            .await
            .map_err(DaemonError::ShutdownSignal)
    }
}
