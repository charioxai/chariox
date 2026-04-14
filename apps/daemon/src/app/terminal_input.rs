use crate::app::provider_runtime::ProviderRunLivenessRuntime;
use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::ProviderRunState;

pub(crate) struct ProviderTerminalInput<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> ProviderTerminalInput<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn send_provider_input(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        attachment_id: &str,
        bytes: &[u8],
    ) -> Result<(), DaemonError> {
        let _ = ProviderRunLivenessRuntime::new(self.app)
            .reconcile_provider_run_exit(session_id, provider_run_id)?;
        if !crate::scheduler::runtime::is_workflow_prompt_attachment(attachment_id) {
            self.app
                .ensure_attachment_in_session(session_id, attachment_id)?;
        }
        let provider_run = self
            .app
            .ensure_provider_run_in_session(session_id, provider_run_id)?;

        if provider_run.state() != ProviderRunState::Running {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: provider_run_id.to_string(),
                state: provider_run.state(),
                operation: "send terminal input",
            });
        }

        self.app
            .terminal
            .record_input(session_id, provider_run_id, attachment_id, bytes);
        self.app.pty.write_input(provider_run_id, bytes)
    }
}
