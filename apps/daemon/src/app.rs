use crate::attachment::{AttachRequest, AttachmentService, RuntimeAttachment};
use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::provider::{LaunchProviderRequest, ProviderProcessService, RuntimeProviderRun};
use crate::session::{RuntimeSession, SessionService, SessionStatus};

#[derive(Debug)]
pub struct DaemonApp {
    config: DaemonConfig,
    attachments: AttachmentService,
    providers: ProviderProcessService,
    sessions: SessionService,
}

impl DaemonApp {
    pub fn bootstrap(config: DaemonConfig) -> Result<Self, DaemonError> {
        config.validate()?;

        Ok(Self {
            attachments: AttachmentService::new(),
            providers: ProviderProcessService::new(),
            sessions: SessionService::new(&config),
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

    pub fn attach(&mut self, request: AttachRequest) -> Result<RuntimeAttachment, DaemonError> {
        self.attachments.attach(&mut self.sessions, request)
    }

    pub fn end_session(&mut self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        let session = self.sessions.get_session(session_id)?;

        if session.status() == SessionStatus::Ended {
            return self.sessions.end_session(session_id);
        }

        self.attachments.remove_session_attachments(session_id);
        self.sessions.end_session(session_id)
    }

    pub fn launch_provider(
        &mut self,
        request: LaunchProviderRequest,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.providers.launch_run(&mut self.sessions, request)
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
