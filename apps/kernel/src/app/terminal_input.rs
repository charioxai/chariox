use crate::app::provider_runtime::ProviderRunLivenessRuntime;
use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::ProviderRunState;

pub(crate) fn provider_prompt_input(prompt: &str) -> Vec<u8> {
    let mut input = prompt.as_bytes().to_vec();
    if !input.ends_with(b"\n") && !input.ends_with(b"\r") {
        input.push(b'\n');
    }
    input
}

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
        self.send_provider_input_with_attachment_validation(
            session_id,
            provider_run_id,
            attachment_id,
            bytes,
            true,
        )
    }

    pub(crate) fn send_remote_provider_input(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        source_attachment_id: &str,
        bytes: &[u8],
    ) -> Result<(), DaemonError> {
        self.send_provider_input_with_attachment_validation(
            session_id,
            provider_run_id,
            source_attachment_id,
            bytes,
            false,
        )
    }

    fn send_provider_input_with_attachment_validation(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        source_attachment_id: &str,
        bytes: &[u8],
        validate_attachment_membership: bool,
    ) -> Result<(), DaemonError> {
        let _ = ProviderRunLivenessRuntime::new(self.app)
            .reconcile_provider_run_exit(session_id, provider_run_id)?;
        if validate_attachment_membership
            && !crate::scheduler::runtime::is_workflow_prompt_attachment(source_attachment_id)
        {
            crate::app::KernelSessionReadService::new(self.app)
                .ensure_attachment_in_session(session_id, source_attachment_id)?;
        }
        let provider_run = crate::app::ProviderRunReadService::new(self.app)
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
            .record_input(session_id, provider_run_id, source_attachment_id, bytes);
        self.app
            .pty
            .input_writer(provider_run_id)?
            .enqueue_input(bytes)
    }
}

impl DaemonApp {
    pub(crate) fn provider_pty_input_writer_for_runtime(
        &self,
        provider_run_id: &str,
    ) -> Result<crate::pty::PtyInputWriter, DaemonError> {
        self.pty.input_writer(provider_run_id)
    }

    pub(crate) fn write_provider_pty_input_for_runtime(
        &mut self,
        provider_run_id: &str,
        bytes: &[u8],
    ) -> Result<(), DaemonError> {
        self.pty.input_writer(provider_run_id)?.enqueue_input(bytes)
    }

    pub(crate) fn drain_provider_pty_output_for_runtime(
        &mut self,
        provider_run_id: &str,
    ) -> Result<Vec<crate::pty::PtyOutputChunk>, DaemonError> {
        self.pty.drain_output(provider_run_id)
    }
}

#[cfg(test)]
mod tests {
    use super::provider_prompt_input;

    #[test]
    fn provider_prompt_input_appends_one_submit_terminator() {
        assert_eq!(provider_prompt_input("hello"), b"hello\n");
        assert_eq!(provider_prompt_input("hello\n"), b"hello\n");
        assert_eq!(provider_prompt_input("hello\r"), b"hello\r");
    }
}
