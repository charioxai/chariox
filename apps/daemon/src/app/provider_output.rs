use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::history::{SessionHistoryEntry, SessionHistoryStore};
use crate::kernel::projection::SessionHistoryProjectionStore;
use crate::provider::{AgentEndpointMode, ProviderProcessServiceStore, ProviderRunState};
use crate::provider::{ProviderPromptSignalBatch, RuntimeProviderRun};
use crate::pty::PtyOutputChunk;
use crate::session::{PromptStatus, SessionStateStore};
use crate::terminal::{TerminalOutputKind, TerminalOutputRecord, TerminalStreamStore};

#[derive(Clone, Default)]
pub(crate) struct StructuredOutputRecordStore {
    records: Arc<Mutex<BTreeMap<String, Vec<TerminalOutputRecord>>>>,
}

impl StructuredOutputRecordStore {
    pub(crate) fn take(&self, provider_run_id: &str) -> Vec<TerminalOutputRecord> {
        self.records
            .lock()
            .expect("structured output record store poisoned")
            .remove(provider_run_id)
            .unwrap_or_default()
    }

    pub(crate) fn append(&self, provider_run_id: String, records: Vec<TerminalOutputRecord>) {
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
    pub(crate) initial_liveness_already_checked: bool,
}

pub(crate) fn pump_terminal_output_for_attachment(
    app: &mut DaemonApp,
    session_id: &str,
    attachment_id: &str,
) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
    app.reap_structured_prompt_jobs();
    crate::app::KernelSessionReadService::new(app)
        .ensure_attachment_in_session(session_id, attachment_id)?;
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
            initial_liveness_already_checked: false,
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
        if !request.initial_liveness_already_checked
            && self
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
    provider_store: ProviderProcessServiceStore,
    pending_structured_output_records: StructuredOutputRecordStore,
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

struct ProviderOutputLiveness<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> ProviderOutputLiveness<'a> {
    fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    fn reconcile_exit(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        super::provider_runtime::ProviderRunLivenessRuntime::new(self.app)
            .reconcile_provider_run_exit(session_id, provider_run_id)
    }
}

struct ProviderOutputFanout {
    provider_store: ProviderProcessServiceStore,
    session_store: SessionStateStore,
    history_store: SessionHistoryStore,
    history_projection: SessionHistoryProjectionStore,
    terminal: TerminalStreamStore,
}

impl ProviderOutputFanout {
    fn new(app: &DaemonApp) -> Self {
        Self {
            provider_store: app.providers.clone(),
            session_store: app.sessions.clone(),
            history_store: app.history_store(),
            history_projection: app.session_history_projection_store(),
            terminal: app.terminal.clone(),
        }
    }

    fn fan_out(
        &self,
        session_id: &str,
        provider_run_id: &str,
        kind: TerminalOutputKind,
        merge_key: Option<String>,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        let agent_id = self
            .provider_store
            .get_run(provider_run_id)
            .ok()
            .and_then(|run| run.agent_instance_id().map(str::to_string));
        let record = self.terminal.fan_out_output(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            kind.clone(),
            merge_key.clone(),
            recipient_attachment_ids,
            bytes,
        );
        if kind != TerminalOutputKind::PromptEcho {
            self.append_history_entry(
                session_id,
                SessionHistoryEntry::provider_output(
                    session_id,
                    provider_run_id,
                    agent_id.as_deref(),
                    kind,
                    merge_key,
                    String::from_utf8_lossy(bytes).into_owned(),
                ),
            );
        }
        record
    }

    fn append_history_entry(&self, session_id: &str, entry: SessionHistoryEntry) {
        let session = match self.session_store.get_session(session_id) {
            Ok(session) => session,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.history",
                    "skipping provider-output history append because session lookup failed",
                    serde_json::json!({
                        "session_id": session_id,
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
        if let Err(error) = self.history_store.append(&session, &entry) {
            crate::logging::warn_with_fields(
                "daemon.history",
                "failed to append provider-output session history",
                serde_json::json!({
                    "session_id": session_id,
                    "error": error.to_string(),
                }),
            );
        } else {
            self.history_projection.append(entry);
        }
    }
}

impl<'a> ProviderOutputPumpContext<'a> {
    fn new(app: &'a mut DaemonApp) -> Self {
        Self {
            provider_store: app.providers.clone(),
            pending_structured_output_records: app.pending_structured_output_records.clone(),
            app,
        }
    }

    fn reap_structured_prompt_jobs(&mut self) {
        self.app.reap_structured_prompt_jobs();
    }

    fn reconcile_provider_run_exit(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        ProviderOutputLiveness::new(self.app).reconcile_exit(session_id, provider_run_id)
    }

    fn ensure_provider_run_in_session(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let provider_run = self.provider_store.get_run(provider_run_id)?;
        if provider_run.session_id() != session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: session_id.to_string(),
                provider_run_id: provider_run_id.to_string(),
            });
        }
        Ok(provider_run)
    }

    fn run_uses_structured_prompt_io(&self, provider_run: &RuntimeProviderRun) -> bool {
        self.provider_store
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
        let mut records = self.pending_structured_output_records.take(provider_run_id);
        records.extend(self.drain_finished_structured_output_jobs_for_run(
            session_id,
            provider_run_id,
            recipient_attachment_ids.clone(),
        )?);
        self.provider_store
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
            .provider_store
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
                        self.provider_store
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
            let provider_run = match self.provider_store.get_run(&provider_run_id) {
                Ok(run) => run,
                Err(_) => continue,
            };
            let session_id = provider_run.session_id().to_string();
            let recipient_attachment_ids = if is_requested_run {
                requested_recipient_attachment_ids.clone()
            } else {
                self.recipient_attachment_ids_for_session(&session_id)
            };
            let records = self.apply_structured_output_batch(
                &session_id,
                &provider_run_id,
                recipient_attachment_ids,
                poll_result,
            )?;
            if is_requested_run {
                requested_records.extend(records);
            } else {
                self.pending_structured_output_records
                    .append(provider_run_id, records);
            }
        }
        Ok(requested_records)
    }

    fn apply_structured_output_batch(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
        poll_result: ProviderPromptSignalBatch,
    ) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
        self.provider_store
            .apply_structured_output_metadata(provider_run_id, &poll_result)?;
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
        self.app
            .update_provider_run_projection(provider_run.clone());
        for notice in &poll_result.notices {
            self.app.record_notice(
                session_id,
                Some(provider_run_id),
                recipient_attachment_ids.clone(),
                notice.to_string(),
            );
        }
        let saw_response_content = poll_result.chunks.iter().any(|chunk| {
            matches!(
                chunk.kind,
                TerminalOutputKind::ProviderOutput | TerminalOutputKind::ProviderReasoning
            )
        });
        let saw_runtime_activity = poll_result.chunks.iter().any(|chunk| {
            matches!(
                chunk.kind,
                TerminalOutputKind::ProviderOutput
                    | TerminalOutputKind::ProviderReasoning
                    | TerminalOutputKind::ProviderTool
                    | TerminalOutputKind::ProviderStatus
            )
        });
        if saw_response_content {
            crate::transport::flow_control::note_prompt_response_content(self.app, provider_run_id);
        } else if saw_runtime_activity {
            crate::transport::flow_control::note_prompt_output(self.app, provider_run_id);
        }
        for completion in &poll_result.completions {
            self.app.record_assistant_message_completion(
                session_id,
                provider_run_id,
                recipient_attachment_ids.clone(),
                &completion.message_id,
                completion.completed_at_ms,
            );
            crate::transport::flow_control::mark_prompt_completion_recorded(
                self.app,
                provider_run_id,
            );
        }
        let prompt_completed = poll_result.prompt_completed;
        let records = poll_result
            .chunks
            .into_iter()
            .map(|chunk| {
                self.fan_out_terminal_output(
                    session_id,
                    provider_run_id,
                    chunk.kind,
                    chunk.merge_key,
                    recipient_attachment_ids.clone(),
                    &chunk.bytes,
                )
            })
            .collect();
        let exited = self.reconcile_provider_run_exit(session_id, provider_run_id)?;
        if exited {
            return Ok(records);
        }
        let agent_id =
            provider_run
                .agent_instance_id()
                .ok_or_else(|| DaemonError::AgentNotFound {
                    agent_id: "provider run has no agent".to_string(),
                })?;
        let active_prompt_status = self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
            .map(|prompt| prompt.status());
        if active_prompt_status == Some(PromptStatus::Cancelling) {
            if prompt_completed {
                let _ = self.app.finalize_active_prompt_cancellation(
                    session_id,
                    agent_id,
                    Some(provider_run_id),
                )?;
            }
        } else if prompt_completed && active_prompt_status.is_some() {
            let _ = self
                .app
                .complete_active_prompt(session_id, agent_id, Some(provider_run_id))?;
        } else if !prompt_completed && active_prompt_status == Some(PromptStatus::Cancelling) {
            crate::transport::flow_control::maybe_complete_active_prompt(
                self.app,
                session_id,
                provider_run_id,
            )?;
        }
        Ok(records)
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
        self.fan_out_terminal_output(
            session_id,
            provider_run_id,
            TerminalOutputKind::ProviderOutput,
            None,
            recipient_attachment_ids,
            bytes,
        )
    }

    fn fan_out_terminal_output(
        &self,
        session_id: &str,
        provider_run_id: &str,
        kind: TerminalOutputKind,
        merge_key: Option<String>,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        ProviderOutputFanout::new(self.app).fan_out(
            session_id,
            provider_run_id,
            kind,
            merge_key,
            recipient_attachment_ids,
            bytes,
        )
    }
}
