use crate::agent::{AgentInstance, CreateAgentRequest};
use crate::app::{DaemonApp, KernelSessionService, ProviderRunReadService};
use crate::error::DaemonError;
use crate::session::{CreateSessionRequest, RuntimeSession};

impl DaemonApp {
    #[doc(hidden)]
    pub fn create_session(
        &mut self,
        request: CreateSessionRequest,
    ) -> Result<(RuntimeSession, AgentInstance), DaemonError> {
        KernelSessionService::new(self).create_session(request)
    }

    #[doc(hidden)]
    pub fn attach(
        &mut self,
        request: crate::attachment::AttachRequest,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        KernelSessionService::new(self).attach(request)
    }

    #[doc(hidden)]
    pub fn detach(
        &mut self,
        attachment_id: &str,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        KernelSessionService::new(self).detach(attachment_id)
    }

    #[doc(hidden)]
    pub fn end_session(&mut self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        KernelSessionService::new(self).end_session(session_id)
    }

    #[doc(hidden)]
    pub fn spawn_agent(
        &mut self,
        request: CreateAgentRequest,
    ) -> Result<AgentInstance, DaemonError> {
        KernelSessionService::new(self).spawn_agent(request)
    }

    #[doc(hidden)]
    pub fn focus_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<AgentInstance, DaemonError> {
        KernelSessionService::new(self).focus_agent(session_id, agent_id)
    }

    #[doc(hidden)]
    pub fn resize_terminal(
        &mut self,
        session_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), DaemonError> {
        KernelSessionService::new(self).resize_terminal(session_id, cols, rows)
    }

    #[doc(hidden)]
    pub fn send_terminal_input(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        provider_run_id: Option<&str>,
        bytes: &[u8],
    ) -> Result<(), DaemonError> {
        let provider_run_id = match provider_run_id {
            Some(provider_run_id) => {
                ProviderRunReadService::new(self)
                    .ensure_provider_run_in_session(session_id, provider_run_id)?;
                provider_run_id.to_string()
            }
            None => self
                .sessions()
                .get_session(session_id)?
                .active_provider_run_id()
                .ok_or_else(|| DaemonError::NoActiveProviderRun {
                    session_id: session_id.to_string(),
                })?
                .to_string(),
        };
        crate::app::terminal_input::ProviderTerminalInput::new(self).send_provider_input(
            session_id,
            &provider_run_id,
            attachment_id,
            bytes,
        )
    }

    #[doc(hidden)]
    pub fn pump_active_prompt_outputs(&mut self) {
        crate::app::provider_output::pump_active_prompt_outputs(self);
    }
}
