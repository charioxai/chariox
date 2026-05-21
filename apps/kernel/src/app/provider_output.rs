use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::app::{DaemonApp, PromptActivityStore};
use crate::error::DaemonError;
use crate::provider::{
    classify_provider_terminal_failure_text, ProviderPromptSignalBatch, RuntimeProviderRun,
};
use crate::provider::{AgentEndpointMode, ProviderProcessServiceStore, ProviderRunState};
use crate::pty::PtyOutputChunk;
use crate::runtime::projection::AgentRuntimeProjectionStore;
use crate::terminal::{TerminalOutputKind, TerminalOutputRecord};

use super::provider_output_claude_native::ProviderOutputClaudeNativeBridge;
use super::provider_output_fanout::ProviderOutputFanout;
use super::provider_output_prompt_settlement::ProviderOutputPromptSettlement;
use super::provider_output_trace::ProviderOutputTrace;

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
        if provider_run.adapter_key() == "claude" && !provider_run.client_interface().is_arroba() {
            self.context.process_claude_native_tui_bridge(
                request.session_id,
                request.provider_run_id,
                &provider_run,
            )?;
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
        if provider_run.adapter_key() == "claude" && !provider_run.client_interface().is_arroba() {
            let rendered = chunks
                .iter()
                .map(|chunk| String::from_utf8_lossy(&chunk.bytes))
                .collect::<String>();
            self.context.process_claude_native_terminal_output_bridge(
                request.session_id,
                request.provider_run_id,
                &provider_run,
                &rendered,
            )?;
        }
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
        if records.is_empty() {
            self.context
                .settle_pty_prompt_if_quiet(request.session_id, request.provider_run_id)?;
        }

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
        self.persist_resolved_resume_state(&provider_run, &poll_result)?;
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
        let saw_settlement_blocking_activity = poll_result.chunks.iter().any(|chunk| {
            matches!(
                chunk.kind,
                TerminalOutputKind::ProviderOutput
                    | TerminalOutputKind::ProviderReasoning
                    | TerminalOutputKind::ProviderTool
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
        let terminal_failure = poll_result.terminal_failure.clone();
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
            self.trace_prompt_state(
                session_id,
                provider_run_id,
                "structured_poll_provider_exited",
            );
            return Ok(records);
        }
        if let Some(message) = terminal_failure {
            self.fail_prompt_for_terminal_failure(session_id, provider_run_id, &message)?;
            self.trace_prompt_state(
                session_id,
                provider_run_id,
                "structured_poll_terminal_failure_settled",
            );
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
            saw_settlement_blocking_activity,
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

    fn persist_resolved_resume_state(
        &mut self,
        provider_run: &RuntimeProviderRun,
        poll_result: &ProviderPromptSignalBatch,
    ) -> Result<(), DaemonError> {
        let Some(resume_state) = poll_result.resolved_resume_state.as_ref() else {
            return Ok(());
        };
        let Some(agent_id) = provider_run.agent_instance_id() else {
            return Ok(());
        };
        let agent = self.app.agents.set_agent_runtime_profile(
            agent_id,
            provider_run.provider(),
            Some(provider_run.model().to_string()),
            provider_run.variant().map(str::to_string),
            resume_state.clone(),
        )?;
        self.app.durable_state_store().append_event(
            "agent.runtime_profile_updated",
            Some(agent.id().to_string()),
            serde_json::json!({
                "agent": &agent,
                "provider_run_id": provider_run.id(),
            }),
        )?;
        let _ = crate::app::KernelSessionReadService::new(self.app)
            .session_snapshot(provider_run.session_id())?;
        Ok(())
    }

    fn trace_structured_poll_batch(
        &self,
        session_id: &str,
        provider_run_id: &str,
        source: &str,
        poll_result: &ProviderPromptSignalBatch,
    ) {
        self.trace()
            .structured_poll_batch(session_id, provider_run_id, source, poll_result);
    }

    fn trace_terminal_records(
        &self,
        session_id: &str,
        provider_run_id: &str,
        source: &str,
        records: &[TerminalOutputRecord],
    ) {
        self.trace()
            .terminal_records(session_id, provider_run_id, source, records);
    }

    fn trace_prompt_state(&self, session_id: &str, provider_run_id: &str, source: &str) {
        self.trace()
            .prompt_state_turn(session_id, provider_run_id, source);
    }

    fn trace(&self) -> ProviderOutputTrace {
        ProviderOutputTrace::new(
            self.app,
            self.provider_store.clone(),
            self.active_turns.clone(),
            self.prompt_activity.clone(),
        )
    }

    fn drain_pty_output(
        &mut self,
        provider_run_id: &str,
    ) -> Result<Vec<PtyOutputChunk>, DaemonError> {
        ProviderOutputPtyDrain::new(self.app).drain_output(provider_run_id)
    }

    fn process_claude_native_tui_bridge(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        provider_run: &RuntimeProviderRun,
    ) -> Result<(), DaemonError> {
        ProviderOutputClaudeNativeBridge::new(self.app).process(
            session_id,
            provider_run_id,
            provider_run,
            self.provider_store.native_interaction_bridge(),
        )
    }

    fn process_claude_native_terminal_output_bridge(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        provider_run: &RuntimeProviderRun,
        rendered: &str,
    ) -> Result<(), DaemonError> {
        ProviderOutputClaudeNativeBridge::new(self.app).process_terminal_output(
            session_id,
            provider_run_id,
            provider_run,
            self.provider_store.native_interaction_bridge(),
            rendered,
        )
    }

    fn recipient_attachment_ids_for_session(&self, session_id: &str) -> Vec<String> {
        ProviderOutputRecipientResolver::new(self.app).session_attachment_ids(session_id)
    }

    fn note_prompt_output(&self, provider_run_id: &str) {
        if let Some(state) = self.prompt_activity.write().get_mut(provider_run_id) {
            state.last_output_at = Some(Instant::now());
        }
        self.active_turns.mark_streaming(provider_run_id);
    }

    fn note_prompt_response_content(&self, provider_run_id: &str) {
        let first_response_content = {
            let mut prompt_activity = self.prompt_activity.write();
            if let Some(state) = prompt_activity.get_mut(provider_run_id) {
                let first_response_content = !state.saw_response_content;
                state.last_output_at = Some(Instant::now());
                state.saw_response_content = true;
                first_response_content
            } else {
                false
            }
        };
        if first_response_content {
            self.active_turns.mark_streaming(provider_run_id);
            if let Ok(run) = self.provider_store.get_run(provider_run_id) {
                let active_turn = self.active_turns.snapshot().remove(provider_run_id);
                crate::runtime::command_latency::log_provider_first_response_content(
                    &run,
                    active_turn.as_ref(),
                );
            }
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
        saw_settlement_blocking_activity: bool,
    ) -> Result<(), DaemonError> {
        self.prompt_settlement().settle_structured_completion(
            session_id,
            provider_run_id,
            prompt_completed,
            saw_settlement_blocking_activity,
        )
    }

    fn settle_pty_prompt_if_quiet(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<(), DaemonError> {
        self.prompt_settlement()
            .settle_pty_if_quiet(session_id, provider_run_id)
    }

    fn fail_prompt_for_terminal_failure(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        message: &str,
    ) -> Result<(), DaemonError> {
        self.prompt_settlement()
            .fail_for_terminal_failure(session_id, provider_run_id, message)
    }

    fn prompt_settlement(&mut self) -> ProviderOutputPromptSettlement<'_> {
        ProviderOutputPromptSettlement::new(
            self.app,
            self.provider_store.clone(),
            self.active_turns.clone(),
            self.prompt_activity.clone(),
            self.agent_runtime_projection.clone(),
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
