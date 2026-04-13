use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::ProviderRunState;
use crate::terminal::{TerminalOutputKind, TerminalOutputRecord};

pub(crate) struct ProviderOutputPumpRequest<'a> {
    pub(crate) session_id: &'a str,
    pub(crate) provider_run_id: &'a str,
    pub(crate) recipient_attachment_ids: Vec<String>,
}

pub(crate) struct ProviderOutputPump<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> ProviderOutputPump<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn pump_provider_output(
        &mut self,
        request: ProviderOutputPumpRequest<'_>,
    ) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
        self.app.reap_structured_prompt_jobs();
        if self
            .app
            .reconcile_provider_run_exit(request.session_id, request.provider_run_id)?
        {
            return Ok(Vec::new());
        }
        let provider_run = self
            .app
            .ensure_provider_run_in_session(request.session_id, request.provider_run_id)?;
        if provider_run.state() == ProviderRunState::Ended {
            return Ok(Vec::new());
        }
        // Parked runs should not be polled for output.
        if provider_run.state() == ProviderRunState::Parked {
            return Ok(Vec::new());
        }

        if self
            .app
            .providers
            .run_uses_structured_prompt_io(&provider_run)
        {
            return self.app.pump_structured_output(
                request.session_id,
                request.provider_run_id,
                request.recipient_attachment_ids,
            );
        }

        let chunks = match self.app.pty.drain_output(request.provider_run_id) {
            Ok(chunks) => chunks,
            Err(error) => {
                if self
                    .app
                    .reconcile_provider_run_exit(request.session_id, request.provider_run_id)?
                {
                    return Ok(Vec::new());
                }
                return Err(error);
            }
        };
        if !chunks.is_empty() {
            crate::transport::flow_control::note_prompt_response_content(
                self.app,
                request.provider_run_id,
            );
        }
        let exited = self
            .app
            .reconcile_provider_run_exit(request.session_id, request.provider_run_id)?;
        if !exited {
            crate::transport::flow_control::maybe_complete_active_prompt(
                self.app,
                request.session_id,
                request.provider_run_id,
            )?;
        }

        Ok(chunks
            .into_iter()
            .map(|chunk| {
                self.app.fan_out_output(
                    request.session_id,
                    request.provider_run_id,
                    TerminalOutputKind::ProviderOutput,
                    None,
                    request.recipient_attachment_ids.clone(),
                    &chunk.bytes,
                )
            })
            .collect())
    }
}
