use crate::agent::AgentInstance;
use crate::app::DaemonApp;
use crate::attachment::{AttachRequest, RuntimeAttachment};
use crate::error::DaemonError;
use crate::provider::{AgentEndpointMode, ProviderRunState};

pub(crate) struct KernelSessionService<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> KernelSessionService<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn attach(
        &mut self,
        request: AttachRequest,
    ) -> Result<RuntimeAttachment, DaemonError> {
        self.app.attach(request)
    }

    pub(crate) fn detach(&mut self, attachment_id: &str) -> Result<RuntimeAttachment, DaemonError> {
        self.app.detach(attachment_id)
    }

    pub(crate) fn focus_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .app
            .agents
            .focus_agent(session_id, agent_id, &mut self.app.sessions)?;
        if !self
            .app
            .should_defer_provider_run_sync_for_focus_change(session_id, agent_id)?
        {
            self.app
                .sync_active_provider_run_for_agent(session_id, agent_id)?;
        }
        Ok(agent)
    }

    pub(crate) fn cycle_agent_focus(
        &mut self,
        session_id: &str,
    ) -> Result<Option<AgentInstance>, DaemonError> {
        let agent = self
            .app
            .agents
            .cycle_focus(session_id, &mut self.app.sessions)?;
        if let Some(focused) = agent.as_ref() {
            if !self
                .app
                .should_defer_provider_run_sync_for_focus_change(session_id, focused.id())?
            {
                self.app
                    .sync_active_provider_run_for_agent(session_id, focused.id())?;
            }
        }
        Ok(agent)
    }

    pub(crate) fn resize_terminal(
        &mut self,
        session_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), DaemonError> {
        let provider_run_id = self
            .app
            .sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?
            .to_string();

        self.resize_provider_terminal(session_id, &provider_run_id, cols, rows)
    }

    pub(crate) fn resize_provider_terminal(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), DaemonError> {
        let _ = self
            .app
            .reconcile_provider_run_exit(session_id, provider_run_id)?;
        let provider_run = self
            .app
            .ensure_provider_run_in_session(session_id, provider_run_id)?;

        if provider_run.state() == ProviderRunState::Ended {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: provider_run_id.to_string(),
                state: provider_run.state(),
                operation: "resize terminal",
            });
        }

        if provider_run.endpoint_mode() == AgentEndpointMode::External {
            return Ok(());
        }

        self.app.pty.resize(provider_run_id, cols, rows)
    }
}
