use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::ProviderRunState;
use crate::provider::RuntimeProviderRun;
use crate::pty::PtyOutputChunk;
use crate::terminal::{TerminalOutputKind, TerminalOutputRecord};

pub(crate) struct ProviderOutputPumpRequest<'a> {
    pub(crate) session_id: &'a str,
    pub(crate) provider_run_id: &'a str,
    pub(crate) recipient_attachment_ids: Vec<String>,
}

pub(crate) struct ProviderOutputPump<'a> {
    context: ProviderOutputPumpContext<'a>,
}

impl<'a> ProviderOutputPump<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self {
            context: ProviderOutputPumpContext::new(app),
        }
    }

    pub(crate) fn pump_provider_output(
        &mut self,
        request: ProviderOutputPumpRequest<'_>,
    ) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
        self.context.reap_structured_prompt_jobs();
        if self
            .context
            .reconcile_provider_run_exit(request.session_id, request.provider_run_id)?
        {
            return Ok(Vec::new());
        }
        let provider_run = self
            .context
            .ensure_provider_run_in_session(request.session_id, request.provider_run_id)?;
        if provider_run.state() == ProviderRunState::Ended {
            return Ok(Vec::new());
        }
        // Parked runs should not be polled for output.
        if provider_run.state() == ProviderRunState::Parked {
            return Ok(Vec::new());
        }

        if self.context.run_uses_structured_prompt_io(&provider_run) {
            return self.context.pump_structured_output(
                request.session_id,
                request.provider_run_id,
                request.recipient_attachment_ids,
            );
        }

        let chunks = match self.context.drain_pty_output(request.provider_run_id) {
            Ok(chunks) => chunks,
            Err(error) => {
                if self
                    .context
                    .reconcile_provider_run_exit(request.session_id, request.provider_run_id)?
                {
                    return Ok(Vec::new());
                }
                return Err(error);
            }
        };
        if !chunks.is_empty() {
            self.context
                .note_prompt_response_content(request.provider_run_id);
        }
        let exited = self
            .context
            .reconcile_provider_run_exit(request.session_id, request.provider_run_id)?;
        if !exited {
            self.context
                .maybe_complete_active_prompt(request.session_id, request.provider_run_id)?;
        }

        Ok(chunks
            .into_iter()
            .map(|chunk| {
                self.context.fan_out_provider_output(
                    request.session_id,
                    request.provider_run_id,
                    request.recipient_attachment_ids.clone(),
                    &chunk.bytes,
                )
            })
            .collect())
    }
}

struct ProviderOutputPumpContext<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> ProviderOutputPumpContext<'a> {
    fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    fn reap_structured_prompt_jobs(&mut self) {
        self.app.reap_structured_prompt_jobs();
    }

    fn reconcile_provider_run_exit(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        self.app
            .reconcile_provider_run_exit(session_id, provider_run_id)
    }

    fn ensure_provider_run_in_session(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.app
            .ensure_provider_run_in_session(session_id, provider_run_id)
    }

    fn run_uses_structured_prompt_io(&self, provider_run: &RuntimeProviderRun) -> bool {
        self.app
            .providers
            .run_uses_structured_prompt_io(provider_run)
    }

    fn pump_structured_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
    ) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
        self.app
            .pump_structured_output(session_id, provider_run_id, recipient_attachment_ids)
    }

    fn drain_pty_output(
        &mut self,
        provider_run_id: &str,
    ) -> Result<Vec<PtyOutputChunk>, DaemonError> {
        self.app.pty.drain_output(provider_run_id)
    }

    fn note_prompt_response_content(&mut self, provider_run_id: &str) {
        crate::transport::flow_control::note_prompt_response_content(self.app, provider_run_id);
    }

    fn maybe_complete_active_prompt(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<(), DaemonError> {
        crate::transport::flow_control::maybe_complete_active_prompt(
            self.app,
            session_id,
            provider_run_id,
        )
    }

    fn fan_out_provider_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        self.app.fan_out_output(
            session_id,
            provider_run_id,
            TerminalOutputKind::ProviderOutput,
            None,
            recipient_attachment_ids,
            bytes,
        )
    }
}
