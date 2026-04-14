use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::RuntimeProviderRun;
use crate::provider::{AgentEndpointMode, ProviderRunState};
use crate::pty::PtyOutputChunk;
use crate::terminal::{TerminalOutputKind, TerminalOutputRecord};

#[derive(Clone, Default)]
pub(crate) struct StructuredOutputRecordStore {
    records: Arc<Mutex<BTreeMap<String, Vec<TerminalOutputRecord>>>>,
}

impl StructuredOutputRecordStore {
    fn take(&self, provider_run_id: &str) -> Vec<TerminalOutputRecord> {
        self.records
            .lock()
            .expect("structured output record store poisoned")
            .remove(provider_run_id)
            .unwrap_or_default()
    }

    fn append(&self, provider_run_id: String, records: Vec<TerminalOutputRecord>) {
        if records.is_empty() {
            return;
        }
        self.records
            .lock()
            .expect("structured output record store poisoned")
            .entry(provider_run_id)
            .or_default()
            .extend(records);
    }
}

pub(crate) struct ProviderOutputPumpRequest<'a> {
    pub(crate) session_id: &'a str,
    pub(crate) provider_run_id: &'a str,
    pub(crate) recipient_attachment_ids: Vec<String>,
}

pub(crate) fn pump_terminal_output_for_attachment(
    app: &mut DaemonApp,
    session_id: &str,
    attachment_id: &str,
) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
    app.reap_structured_prompt_jobs();
    app.ensure_attachment_in_session(session_id, attachment_id)?;
    let provider_run_id = app
        .sessions
        .get_session(session_id)?
        .active_provider_run_id()
        .map(str::to_string);

    if let Some(provider_run_id) = provider_run_id {
        let recipient_attachment_ids =
            ProviderOutputRecipientResolver::new(app).session_attachment_ids(session_id);
        let _ = ProviderOutputPump::new(app).pump_provider_output(ProviderOutputPumpRequest {
            session_id,
            provider_run_id: &provider_run_id,
            recipient_attachment_ids,
        })?;
    }
    Ok(app.terminal.drain_output_records(session_id, attachment_id))
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

struct ProviderOutputRecipientResolver<'a> {
    app: &'a DaemonApp,
}

impl<'a> ProviderOutputRecipientResolver<'a> {
    fn new(app: &'a DaemonApp) -> Self {
        Self { app }
    }

    fn session_attachment_ids(&self, session_id: &str) -> Vec<String> {
        self.app.attachments.list_session_attachment_ids(session_id)
    }
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
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
        // Parked runs should not be polled for output.
        if provider_run.state() == ProviderRunState::Parked {
            return Ok(Vec::new());
        }
        if provider_run.endpoint_mode() != AgentEndpointMode::External {
            if let Err(error) = self.drain_pty_output(provider_run_id) {
                if self.reconcile_provider_run_exit(session_id, provider_run_id)? {
                    return Ok(Vec::new());
                }
                if !matches!(error, DaemonError::PtyProcessNotFound { .. }) {
                    return Err(error);
                }
            }
        }
        let mut records = self
            .app
            .pending_structured_output_records
            .take(provider_run_id);
        records.extend(self.drain_finished_structured_output_jobs_for_run(
            session_id,
            provider_run_id,
            recipient_attachment_ids.clone(),
        )?);
        self.app
            .providers
            .enqueue_structured_output_poll(provider_run_id)?;
        Ok(records)
    }

    fn drain_finished_structured_output_jobs_for_run(
        &mut self,
        requested_session_id: &str,
        requested_provider_run_id: &str,
        requested_recipient_attachment_ids: Vec<String>,
    ) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
        let mut requested_records = Vec::new();
        for finished in self
            .app
            .providers
            .drain_finished_structured_output_poll_jobs()
        {
            let provider_run_id = finished.provider_run_id.clone();
            let is_requested_run = provider_run_id == requested_provider_run_id;
            let poll_result = match finished.result {
                Ok(Some(poll_result)) => poll_result,
                Ok(None) => continue,
                Err(error) => {
                    let reconcile_result = if is_requested_run {
                        self.reconcile_provider_run_exit(
                            requested_session_id,
                            requested_provider_run_id,
                        )
                    } else {
                        self.app
                            .providers
                            .get_run(&provider_run_id)
                            .and_then(|run| {
                                let session_id = run.session_id().to_string();
                                self.reconcile_provider_run_exit(&session_id, &provider_run_id)
                            })
                    };
                    match reconcile_result {
                        Ok(true) => continue,
                        Ok(false) if is_requested_run => return Err(error),
                        Ok(false) => {
                            crate::logging::error_with_fields(
                                "daemon.app",
                                "background structured output poll failed",
                                serde_json::json!({
                                    "provider_run_id": provider_run_id,
                                    "error": error.to_string(),
                                }),
                            );
                            continue;
                        }
                        Err(reconcile_error) if is_requested_run => return Err(reconcile_error),
                        Err(reconcile_error) => {
                            crate::logging::error_with_fields(
                                "daemon.app",
                                "background structured output poll reconciliation failed",
                                serde_json::json!({
                                    "provider_run_id": provider_run_id,
                                    "error": reconcile_error.to_string(),
                                }),
                            );
                            continue;
                        }
                    }
                }
            };
            let provider_run = match self.app.providers.get_run(&provider_run_id) {
                Ok(run) => run,
                Err(_) => continue,
            };
            let session_id = provider_run.session_id().to_string();
            let recipient_attachment_ids = if is_requested_run {
                requested_recipient_attachment_ids.clone()
            } else {
                self.recipient_attachment_ids_for_session(&session_id)
            };
            let records = self.app.apply_structured_output_batch(
                &session_id,
                &provider_run_id,
                recipient_attachment_ids,
                poll_result,
            )?;
            if is_requested_run {
                requested_records.extend(records);
            } else {
                self.app
                    .pending_structured_output_records
                    .append(provider_run_id, records);
            }
        }
        Ok(requested_records)
    }

    fn drain_pty_output(
        &mut self,
        provider_run_id: &str,
    ) -> Result<Vec<PtyOutputChunk>, DaemonError> {
        self.app.pty.drain_output(provider_run_id)
    }

    fn recipient_attachment_ids_for_session(&self, session_id: &str) -> Vec<String> {
        ProviderOutputRecipientResolver::new(self.app).session_attachment_ids(session_id)
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
