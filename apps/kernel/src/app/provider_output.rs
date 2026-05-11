use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::{json, Value};

use crate::app::{DaemonApp, PromptActivityStore};
use crate::error::DaemonError;
use crate::history::{
    HistoryEventTurnContext, OperationalHistoryStore, SessionHistoryEntry, SessionHistoryStore,
};
use crate::provider::{
    classify_provider_terminal_failure_text, ProviderPromptSignalBatch, RuntimeProviderRun,
};
use crate::provider::{AgentEndpointMode, ProviderProcessServiceStore, ProviderRunState};
use crate::pty::PtyOutputChunk;
use crate::runtime::projection::{AgentRuntimeProjectionStore, SessionHistoryProjectionStore};
use crate::session::{PromptQueueItem, PromptStatus, SessionStateStore};
use crate::terminal::{
    RuntimeNoticeRecord, TerminalOutputKind, TerminalOutputRecord, TerminalStreamStore,
};

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
    reap_structured_prompt_jobs(app);
    crate::app::KernelSessionReadService::new(app)
        .ensure_attachment_in_session(session_id, attachment_id)?;
    pump_session_active_prompt_outputs(app, session_id);
    Ok(app.terminal.drain_output_records(session_id, attachment_id))
}

pub(crate) fn reap_structured_prompt_jobs(app: &mut DaemonApp) {
    ProviderOutputStructuredPromptReaper::new(app).reap();
}

pub(crate) fn pump_active_prompt_outputs(app: &mut DaemonApp) {
    reap_structured_prompt_jobs(app);
    let sessions = app.sessions.list_sessions();
    for session in sessions {
        pump_session_active_prompt_outputs(app, session.id());
    }
}

fn pump_session_active_prompt_outputs(app: &mut DaemonApp, session_id: &str) {
    let Ok(session) = app.sessions.get_session(session_id) else {
        return;
    };
    let recipient_attachment_ids = app.attachments.list_session_attachment_ids(session.id());
    let mut provider_run_ids = BTreeSet::new();
    let mut agent_ids = session
        .agents()
        .iter()
        .map(|agent| agent.id().to_string())
        .collect::<Vec<_>>();
    agent_ids.extend(session.prompt_states().keys().cloned());
    agent_ids.sort();
    agent_ids.dedup();
    for agent_id in agent_ids {
        if app
            .prompt_state_owner
            .active_prompt_for_agent_snapshot(&session, &agent_id)
            .is_none()
        {
            continue;
        }
        if let Some(provider_run_id) = app
            .providers
            .get_run_for_agent(session.id(), &agent_id)
            .map(|run| run.id().to_string())
        {
            provider_run_ids.insert(provider_run_id);
        }
    }
    provider_run_ids.extend(
        app.providers
            .list_runs()
            .into_iter()
            .filter(|run| run.session_id() == session.id())
            .filter(|run| {
                matches!(
                    run.state(),
                    ProviderRunState::Starting | ProviderRunState::Running
                )
            })
            .map(|run| run.id().to_string()),
    );
    for provider_run_id in provider_run_ids {
        let agent_id = app
            .providers
            .get_run(&provider_run_id)
            .ok()
            .and_then(|run| run.agent_instance_id().map(str::to_string));
        if let Err(error) =
            ProviderOutputPump::new(app).pump_provider_output(ProviderOutputPumpRequest {
                session_id: session.id(),
                provider_run_id: &provider_run_id,
                recipient_attachment_ids: recipient_attachment_ids.clone(),
                initial_liveness_already_checked: false,
            })
        {
            crate::logging::warn_with_fields(
                "daemon.provider_output",
                "background prompt pump failed",
                serde_json::json!({
                    "session_id": session.id(),
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "error": error.to_string(),
                }),
            );
        }
    }
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
        let mut provider_run = self
            .context
            .ensure_provider_run_in_session(request.session_id, request.provider_run_id)?;
        if provider_run.state() == ProviderRunState::Ended {
            return Ok(Vec::new());
        }
        if provider_run.state() == ProviderRunState::Parked {
            if !self
                .context
                .provider_run_has_active_prompt(request.session_id, &provider_run)?
            {
                return Ok(Vec::new());
            }
            provider_run = self
                .context
                .resume_detached_provider_run(request.provider_run_id)?;
            crate::logging::warn_with_fields(
                "daemon.provider_output",
                "resumed parked provider run that still had an active prompt",
                serde_json::json!({
                    "session_id": request.session_id,
                    "provider_run_id": request.provider_run_id,
                    "agent_id": provider_run.agent_instance_id(),
                }),
            );
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
        let terminal_failure = classify_provider_terminal_failure_text(
            provider_run.adapter_key(),
            &chunks
                .iter()
                .map(|chunk| String::from_utf8_lossy(&chunk.bytes))
                .collect::<String>(),
        );
        if !chunks.is_empty() {
            self.context
                .note_prompt_response_content(request.provider_run_id);
        }

        let records = chunks
            .into_iter()
            .map(|chunk| {
                self.context.fan_out_provider_output(
                    request.session_id,
                    request.provider_run_id,
                    request.recipient_attachment_ids.clone(),
                    &chunk.bytes,
                )
            })
            .collect::<Vec<_>>();
        if let Some(message) = terminal_failure {
            let run = self
                .context
                .provider_store
                .record_terminal_diagnostic(request.provider_run_id, message.clone())?;
            self.context.app.update_provider_run_projection(run);
            self.context.fail_prompt_for_terminal_failure(
                request.session_id,
                request.provider_run_id,
                &message,
            )?;
            return Ok(records);
        }
        self.context
            .reconcile_provider_run_exit(request.session_id, request.provider_run_id)?;

        Ok(records)
    }
}

struct ProviderOutputPumpContext<'a> {
    app: &'a mut DaemonApp,
    provider_store: ProviderProcessServiceStore,
    pending_structured_output_records: StructuredOutputRecordStore,
    active_turns: crate::app::ActiveTurnStore,
    prompt_activity: PromptActivityStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
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

struct ProviderOutputPtyDrain<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> ProviderOutputPtyDrain<'a> {
    fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    fn drain_output(&mut self, provider_run_id: &str) -> Result<Vec<PtyOutputChunk>, DaemonError> {
        self.app.pty.drain_output(provider_run_id)
    }
}

struct ProviderOutputStructuredPromptReaper<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> ProviderOutputStructuredPromptReaper<'a> {
    fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    fn reap(&mut self) {
        self.app.reap_structured_prompt_jobs();
    }
}

struct ProviderOutputFanout {
    provider_store: ProviderProcessServiceStore,
    session_store: SessionStateStore,
    history_store: SessionHistoryStore,
    operational_history_store: OperationalHistoryStore,
    archive_enabled: bool,
    history_projection: SessionHistoryProjectionStore,
    terminal: TerminalStreamStore,
}

impl ProviderOutputFanout {
    fn new(app: &DaemonApp) -> Self {
        Self {
            provider_store: app.providers.clone(),
            session_store: app.sessions.clone(),
            history_store: app.history_store(),
            operational_history_store: app.operational_history_store(),
            archive_enabled: app.history_archive_enabled(),
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
        let provider_run = self.provider_store.get_run(provider_run_id).ok();
        let agent_id = provider_run
            .as_ref()
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

    fn record_notice(
        &self,
        session_id: &str,
        provider_run_id: Option<&str>,
        recipient_attachment_ids: Vec<String>,
        message: impl Into<String>,
    ) -> RuntimeNoticeRecord {
        let message = message.into();
        let agent_id = provider_run_id.and_then(|run_id| {
            self.provider_store
                .get_run(run_id)
                .ok()
                .and_then(|run| run.agent_instance_id().map(str::to_string))
        });
        let record = self.terminal.record_notice(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            recipient_attachment_ids,
            message.clone(),
        );
        self.append_history_entry(
            session_id,
            SessionHistoryEntry::notice(session_id, provider_run_id, agent_id.as_deref(), message),
        );
        record
    }

    fn record_assistant_message_completion(
        &self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
        message_id: &str,
        completed_at_ms: u64,
    ) {
        let agent_id = self
            .provider_store
            .get_run(provider_run_id)
            .ok()
            .and_then(|run| run.agent_instance_id().map(str::to_string));
        self.terminal.record_assistant_message_completion(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            recipient_attachment_ids,
            message_id,
            completed_at_ms,
        );
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
            let provider_run = entry
                .provider_run_id
                .as_deref()
                .and_then(|provider_run_id| self.provider_store.get_run(provider_run_id).ok());
            let context = HistoryEventTurnContext {
                session_id: Some(entry.session_id.clone()),
                agent_id: entry.agent_id.clone().or_else(|| {
                    provider_run
                        .as_ref()
                        .and_then(|run| run.agent_instance_id().map(str::to_string))
                }),
                provider: provider_run.as_ref().map(|run| run.provider().to_string()),
                model: provider_run.as_ref().map(|run| run.model().to_string()),
                provider_run_id: entry.provider_run_id.clone(),
                provider_session_id: provider_run
                    .as_ref()
                    .and_then(|run| run.provider_session_id().map(str::to_string)),
                worktree_path: provider_run.as_ref().and_then(|run| {
                    run.working_directory()
                        .map(|path| path.display().to_string())
                }),
                ..HistoryEventTurnContext::default()
            };
            match self
                .operational_history_store
                .append_transcript(&entry, context)
            {
                Ok(event) => {
                    if self.archive_enabled {
                        if let Err(error) = self
                            .operational_history_store
                            .enqueue_archive_events(std::slice::from_ref(&event))
                        {
                            crate::logging::warn_with_fields(
                                "daemon.history",
                                "failed to enqueue history archive event",
                                serde_json::json!({
                                    "session_id": session_id,
                                    "error": error.to_string(),
                                }),
                            );
                        }
                    }
                }
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.history",
                        "failed to append operational history",
                        serde_json::json!({
                            "session_id": session_id,
                            "error": error.to_string(),
                        }),
                    );
                }
            }
            self.history_projection.append(entry);
        }
    }
}

impl<'a> ProviderOutputPumpContext<'a> {
    fn new(app: &'a mut DaemonApp) -> Self {
        Self {
            provider_store: app.providers.clone(),
            pending_structured_output_records: app.pending_structured_output_records.clone(),
            active_turns: app.active_turns.clone(),
            prompt_activity: app.prompt_activity.clone(),
            agent_runtime_projection: app.agent_runtime_projection_store(),
            app,
        }
    }

    fn reap_structured_prompt_jobs(&mut self) {
        reap_structured_prompt_jobs(self.app);
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

    fn provider_run_has_active_prompt(
        &self,
        session_id: &str,
        provider_run: &RuntimeProviderRun,
    ) -> Result<bool, DaemonError> {
        self.app
            .provider_run_has_active_prompt(session_id, provider_run)
    }

    fn resume_detached_provider_run(
        &mut self,
        provider_run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let run = self.provider_store.resume_run_detached(provider_run_id)?;
        self.app.update_provider_run_projection(run.clone());
        Ok(run)
    }

    fn pump_structured_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
    ) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
        let mut provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
        if provider_run.state() == ProviderRunState::Parked {
            if !self.provider_run_has_active_prompt(session_id, &provider_run)? {
                return Ok(Vec::new());
            }
            provider_run = self.resume_detached_provider_run(provider_run_id)?;
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
        self.trace_structured_poll_batch(
            session_id,
            provider_run_id,
            "structured_poll_batch_received",
            &poll_result,
        );
        self.provider_store
            .apply_structured_output_metadata(provider_run_id, &poll_result)?;
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
        self.app
            .update_provider_run_projection(provider_run.clone());
        let terminal_sink = ProviderOutputFanout::new(self.app);
        for notice in &poll_result.notices {
            terminal_sink.record_notice(
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
            self.note_prompt_response_content(provider_run_id);
        } else if saw_runtime_activity {
            self.note_prompt_output(provider_run_id);
        }
        for completion in &poll_result.completions {
            terminal_sink.record_assistant_message_completion(
                session_id,
                provider_run_id,
                recipient_attachment_ids.clone(),
                &completion.message_id,
                completion.completed_at_ms,
            );
            self.mark_prompt_completion_recorded(provider_run_id);
        }
        let prompt_completed = poll_result.prompt_completed;
        let terminal_failure = poll_result.terminal_failure.clone().or_else(|| {
            let mut text = poll_result.notices.join("\n");
            text.push('\n');
            text.push_str(
                &poll_result
                    .chunks
                    .iter()
                    .map(|chunk| String::from_utf8_lossy(&chunk.bytes))
                    .collect::<String>(),
            );
            classify_provider_terminal_failure_text(provider_run.adapter_key(), &text)
        });
        if let Some(message) = terminal_failure.as_deref() {
            let run = self
                .provider_store
                .record_terminal_diagnostic(provider_run_id, message.to_string())?;
            self.app.update_provider_run_projection(run);
        }
        let records: Vec<TerminalOutputRecord> = poll_result
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
        self.trace_terminal_records(
            session_id,
            provider_run_id,
            "structured_poll_records_fanned_out",
            &records,
        );
        let exited = self.reconcile_provider_run_exit(session_id, provider_run_id)?;
        if exited {
            self.trace_prompt_state(session_id, provider_run_id, "structured_poll_provider_exited");
            return Ok(records);
        }
        if let Some(message) = terminal_failure {
            self.fail_prompt_for_terminal_failure(session_id, provider_run_id, &message)?;
            self.trace_prompt_state(session_id, provider_run_id, "structured_poll_terminal_failure_settled");
            return Ok(records);
        }
        self.trace_prompt_state(
            session_id,
            provider_run_id,
            "structured_poll_before_settlement",
        );
        let settlement = self.settle_structured_prompt_completion(
            session_id,
            provider_run_id,
            prompt_completed,
        );
        self.trace_prompt_state(
            session_id,
            provider_run_id,
            if settlement.is_ok() {
                "structured_poll_after_settlement"
            } else {
                "structured_poll_settlement_error"
            },
        );
        settlement?;
        Ok(records)
    }

    fn trace_structured_poll_batch(
        &self,
        session_id: &str,
        provider_run_id: &str,
        source: &str,
        poll_result: &ProviderPromptSignalBatch,
    ) {
        crate::debug_trace::record_terminal_turn(
            session_id,
            source,
            json!({
                "provider_run_id": provider_run_id,
                "prompt_completed": poll_result.prompt_completed,
                "terminal_failure": poll_result.terminal_failure.as_deref(),
                "completion_count": poll_result.completions.len(),
                "notice_count": poll_result.notices.len(),
                "chunk_count": poll_result.chunks.len(),
                "chunks": poll_result.chunks.iter().map(|chunk| {
                    json!({
                        "kind": &chunk.kind,
                        "merge_key": &chunk.merge_key,
                        "byte_len": chunk.bytes.len(),
                    })
                }).collect::<Vec<_>>(),
                "state": self.prompt_state_trace(session_id, provider_run_id),
            }),
        );
    }

    fn trace_terminal_records(
        &self,
        session_id: &str,
        provider_run_id: &str,
        source: &str,
        records: &[TerminalOutputRecord],
    ) {
        crate::debug_trace::record_terminal_turn(
            session_id,
            source,
            json!({
                "provider_run_id": provider_run_id,
                "record_count": records.len(),
                "records": records.iter().map(|record| {
                    json!({
                        "kind": &record.kind,
                        "agent_id": &record.agent_id,
                        "merge_key": &record.merge_key,
                        "byte_len": record.bytes.len(),
                        "pending_recipient_count": record.pending_recipient_attachment_ids.len(),
                    })
                }).collect::<Vec<_>>(),
                "state": self.prompt_state_trace(session_id, provider_run_id),
            }),
        );
    }

    fn trace_prompt_state(&self, session_id: &str, provider_run_id: &str, source: &str) {
        crate::debug_trace::record_terminal_turn(
            session_id,
            source,
            json!({
                "provider_run_id": provider_run_id,
                "state": self.prompt_state_trace(session_id, provider_run_id),
            }),
        );
    }

    fn prompt_state_trace(&self, session_id: &str, provider_run_id: &str) -> Value {
        let provider_run = self.provider_store.get_run(provider_run_id).ok();
        let agent_id = provider_run
            .as_ref()
            .and_then(|run| run.agent_instance_id())
            .map(str::to_string);
        let session = self.app.sessions.get_session(session_id).ok();
        let active_prompt = match (session.as_ref(), agent_id.as_deref()) {
            (Some(session), Some(agent_id)) => session.active_prompt_for_agent(agent_id),
            _ => None,
        };
        let active_turn = self.active_turns.snapshot().remove(provider_run_id);
        let prompt_activity = self.prompt_activity.read().get(provider_run_id).cloned();
        json!({
            "agent_id": agent_id,
            "provider_run_state": provider_run.as_ref().map(|run| format!("{:?}", run.state())),
            "session_active_provider_run_id": session.as_ref().and_then(|session| session.active_provider_run_id()).map(str::to_string),
            "active_prompt": active_prompt.map(|prompt| {
                json!({
                    "id": prompt.id().to_string(),
                    "status": prompt.status(),
                    "target_agent_id": prompt.target_agent_id().to_string(),
                    "workflow_run_id": prompt.workflow_run_id().map(str::to_string),
                    "workflow_node_run_id": prompt.workflow_node_run_id().map(str::to_string),
                })
            }),
            "active_turn": active_turn.map(|turn| {
                json!({
                    "agent_id": turn.agent_id,
                    "prompt_id": turn.prompt_id,
                    "provider_run_id": turn.provider_run_id,
                    "settlement_requested": turn.settlement_requested,
                })
            }),
            "prompt_activity": prompt_activity.map(|activity| {
                json!({
                    "last_output_seen": activity.last_output_at.is_some(),
                    "saw_response_content": activity.saw_response_content,
                    "completion_recorded": activity.completion_recorded,
                    "settlement_requested": activity.settlement_requested,
                })
            }),
        })
    }

    fn drain_pty_output(
        &mut self,
        provider_run_id: &str,
    ) -> Result<Vec<PtyOutputChunk>, DaemonError> {
        ProviderOutputPtyDrain::new(self.app).drain_output(provider_run_id)
    }

    fn recipient_attachment_ids_for_session(&self, session_id: &str) -> Vec<String> {
        ProviderOutputRecipientResolver::new(self.app).session_attachment_ids(session_id)
    }

    fn note_prompt_output(&self, provider_run_id: &str) {
        if let Some(state) = self.prompt_activity.write().get_mut(provider_run_id) {
            state.last_output_at = Some(Instant::now());
        }
    }

    fn note_prompt_response_content(&self, provider_run_id: &str) {
        if let Some(state) = self.prompt_activity.write().get_mut(provider_run_id) {
            state.last_output_at = Some(Instant::now());
            state.saw_response_content = true;
        }
    }

    fn mark_prompt_completion_recorded(&self, provider_run_id: &str) {
        if let Some(state) = self.prompt_activity.write().get_mut(provider_run_id) {
            state.completion_recorded = true;
        }
    }

    fn settle_structured_prompt_completion(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        prompt_completed: bool,
    ) -> Result<(), DaemonError> {
        let Some(active_prompt_status) = self
            .active_prompt_for_settlement(session_id, provider_run_id)?
            .map(|prompt| prompt.status())
        else {
            return Ok(());
        };
        if active_prompt_status == PromptStatus::Cancelling {
            if prompt_completed {
                let agent_id = self.provider_run_agent_id(provider_run_id)?;
                let _ = self.app.finalize_active_prompt_cancellation(
                    session_id,
                    &agent_id,
                    Some(provider_run_id),
                )?;
                self.clear_active_turn(provider_run_id);
            }
        } else if prompt_completed {
            if self.workflow_prompt_is_waiting_for_completion_output(session_id, provider_run_id)? {
                self.note_prompt_settlement_requested(provider_run_id);
                let _ = crate::app::KernelSessionReadService::new(self.app)
                    .session_snapshot(session_id);
                return Ok(());
            }
            self.settle_prompt_by_status(session_id, provider_run_id)?;
        }
        Ok(())
    }

    fn workflow_prompt_is_waiting_for_completion_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        let Some(prompt) = self.active_prompt_for_settlement(session_id, provider_run_id)? else {
            return Ok(false);
        };
        if prompt.workflow_run_id().is_none() || prompt.workflow_node_run_id().is_none() {
            return Ok(false);
        }
        Ok(
            !crate::app::workflow_runtime::workflow_prompt_has_completion_output_from_runtime(
                self.app,
                session_id,
                &prompt,
                Some(provider_run_id),
            ),
        )
    }

    fn note_prompt_settlement_requested(&self, provider_run_id: &str) {
        self.active_turns.mark_settling(provider_run_id);
        self.prompt_activity
            .write()
            .entry(provider_run_id.to_string())
            .and_modify(|state| {
                state.last_output_at = Some(Instant::now());
                state.saw_response_content = true;
                state.settlement_requested = true;
            })
            .or_insert(crate::app::ActivePromptState {
                last_output_at: Some(Instant::now()),
                saw_response_content: true,
                completion_recorded: false,
                settlement_requested: true,
            });
    }

    fn settle_prompt_by_status(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<(), DaemonError> {
        let Some(prompt) = self.active_prompt_for_settlement(session_id, provider_run_id)? else {
            self.clear_prompt_activity(provider_run_id);
            self.clear_active_turn(provider_run_id);
            return Ok(());
        };
        let agent_id = self.provider_run_agent_id(provider_run_id)?;
        if prompt.status() == PromptStatus::Cancelling {
            let _ = self.app.finalize_active_prompt_cancellation(
                session_id,
                &agent_id,
                Some(provider_run_id),
            )?;
        } else {
            let _ =
                self.app
                    .complete_active_prompt(session_id, &agent_id, Some(provider_run_id))?;
        }
        self.clear_active_turn(provider_run_id);
        Ok(())
    }

    fn fail_prompt_for_terminal_failure(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        message: &str,
    ) -> Result<(), DaemonError> {
        let Some(prompt) = self.active_prompt_for_settlement(session_id, provider_run_id)? else {
            self.clear_prompt_activity(provider_run_id);
            self.clear_active_turn(provider_run_id);
            return Ok(());
        };
        let agent_id = self.provider_run_agent_id(provider_run_id)?;
        if let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        {
            let failure = crate::session::WorkflowFailureEvent::new(
                crate::session::WorkflowFailureKind::ProviderFailure,
                workflow_node_run_id,
                Vec::new(),
                message,
            );
            let _ = self.app.sessions_mut().record_workflow_failure_event(
                session_id,
                workflow_run_id,
                failure,
            );
            let workflow_run = self.app.sessions_mut().fail_workflow_node_run(
                session_id,
                workflow_run_id,
                workflow_node_run_id,
            )?;
            self.app.record_notice(
                session_id,
                Some(provider_run_id),
                self.app.attachments.list_session_attachment_ids(session_id),
                format!(
                    "Workflow run `{}` failed after provider turn failure: {}",
                    workflow_run.id(),
                    message
                ),
            );
            let _ =
                crate::app::KernelSessionReadService::new(self.app).session_snapshot(session_id);
        }
        let _ = self
            .app
            .complete_active_prompt(session_id, &agent_id, Some(provider_run_id))?;
        self.clear_active_turn(provider_run_id);
        Ok(())
    }

    fn active_prompt_for_settlement(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        let agent_id = self.provider_run_agent_id(provider_run_id)?;
        if let Some(prompt) = self
            .agent_runtime_projection
            .get(&agent_id)
            .filter(|projection| projection.session_id == session_id)
            .and_then(|projection| projection.active_prompt)
        {
            return Ok(Some(prompt));
        }
        self.app
            .prompt_owner_active_prompt_for_agent(session_id, &agent_id)
    }

    fn provider_run_agent_id(&self, provider_run_id: &str) -> Result<String, DaemonError> {
        self.provider_store
            .get_run(provider_run_id)?
            .agent_instance_id()
            .map(str::to_string)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "provider run has no agent".to_string(),
            })
    }

    fn clear_prompt_activity(&mut self, provider_run_id: &str) {
        self.prompt_activity.write().remove(provider_run_id);
        if self.app.release_prompt_workspace_claim(provider_run_id) {
            crate::app::workflow_runtime::retry_blocked_workflow_claims_from_runtime(self.app);
        }
    }

    fn clear_active_turn(&self, provider_run_id: &str) {
        self.active_turns.clear(provider_run_id);
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
