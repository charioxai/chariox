//! Prompt transcript fan-out to terminal streams and history stores.

use super::*;
use crate::provider_output_policy::output_bounds::{
    bounded_terminal_output_bytes, should_log_provider_output_truncation,
    terminal_output_delta_bytes,
};
use crate::provider_output_policy::tool_history::{
    is_unread_output_history_entry, should_persist_provider_tool_history,
};

pub(super) struct TerminalOutputBatchAppend {
    pub(super) provider_run_id: String,
    pub(super) agent_id: Option<String>,
    pub(super) kind: crate::terminal::TerminalOutputKind,
    pub(super) merge_key: Option<String>,
    pub(super) bytes: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ActivePromptTranscriptMetadata {
    pub(super) prompt_origin: Option<crate::session::PromptOrigin>,
    pub(super) source_attachment_id: Option<String>,
}

impl KernelRuntimeOwnedState {
    pub(super) fn record_provider_failure_output(
        &self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        message: &str,
    ) {
        let prompt_metadata =
            self.active_prompt_transcript_metadata_for_agent(session_id, Some(agent_id));
        self.fan_out_terminal_outputs(
            session_id,
            vec![TerminalOutputBatchAppend {
                provider_run_id: provider_run_id.to_string(),
                agent_id: Some(agent_id.to_string()),
                kind: crate::terminal::TerminalOutputKind::ProviderError,
                merge_key: None,
                bytes: message.as_bytes().to_vec(),
            }],
        );
        self.append_history_entries(
            session_id,
            vec![crate::history::SessionHistoryEntry::provider_output(
                session_id,
                provider_run_id,
                Some(agent_id),
                crate::terminal::TerminalOutputKind::ProviderError,
                None,
                message,
            )
            .with_prompt_origin(prompt_metadata.prompt_origin)
            .with_source_attachment_id(prompt_metadata.source_attachment_id)],
        );
    }

    pub(super) fn other_attachment_ids(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Vec<String> {
        self.attachment_store
            .list_session_attachment_ids(session_id)
            .into_iter()
            .filter(|id| id != attachment_id)
            .collect()
    }

    pub(super) fn record_assistant_message_completion(
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
        let recipient_attachment_ids =
            self.private_recipient_attachment_ids(agent_id.as_deref(), recipient_attachment_ids);
        let recipient_attachment_ids = self.with_metaagent_trace_recipient_ids(
            session_id,
            agent_id.as_deref(),
            recipient_attachment_ids,
        );
        self.terminal_stream.record_assistant_message_completion(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            recipient_attachment_ids,
            message_id,
            completed_at_ms,
        );
        self.notify_metaagent_trace_activity(session_id, agent_id.as_deref());
    }

    pub(super) fn fan_out_terminal_outputs(
        &self,
        session_id: &str,
        outputs: Vec<TerminalOutputBatchAppend>,
    ) -> Vec<crate::terminal::TerminalOutputRecord> {
        let recipient_attachment_ids = self
            .attachment_store
            .list_session_attachment_ids(session_id);
        self.fan_out_terminal_outputs_to_recipients(session_id, recipient_attachment_ids, outputs)
    }

    pub(super) fn fan_out_terminal_outputs_to_recipients(
        &self,
        session_id: &str,
        recipient_attachment_ids: Vec<String>,
        outputs: Vec<TerminalOutputBatchAppend>,
    ) -> Vec<crate::terminal::TerminalOutputRecord> {
        if outputs.is_empty() {
            return Vec::new();
        }
        let mut trace_agent_ids = std::collections::BTreeSet::new();
        let mut recipient_scope_cache =
            std::collections::BTreeMap::<Option<String>, std::sync::Arc<[String]>>::new();
        let mut prompt_metadata_cache =
            std::collections::BTreeMap::<Option<String>, ActivePromptTranscriptMetadata>::new();
        let mut terminal_outputs = Vec::with_capacity(outputs.len());
        for output in outputs {
            let agent_id = output.agent_id;
            let delta_bytes = terminal_output_delta_bytes(
                session_id,
                &output.provider_run_id,
                agent_id.as_deref(),
                &output.kind,
                &output.merge_key,
                &output.bytes,
            );
            let bounded_bytes = bounded_terminal_output_bytes(&output.kind, &delta_bytes);
            self.log_provider_output_truncation(
                session_id,
                &output.provider_run_id,
                agent_id.as_deref(),
                &output.kind,
                delta_bytes.len(),
                bounded_bytes.len(),
            );
            if bounded_bytes.is_empty() {
                continue;
            }
            let prompt_metadata = prompt_metadata_cache
                .entry(agent_id.clone())
                .or_insert_with(|| {
                    self.active_prompt_transcript_metadata_for_agent(
                        session_id,
                        agent_id.as_deref(),
                    )
                })
                .clone();
            let provider_terminal =
                output.kind == crate::terminal::TerminalOutputKind::ProviderTerminal;
            let scoped_recipient_attachment_ids = if provider_terminal {
                std::sync::Arc::from(self.private_recipient_attachment_ids(
                    agent_id.as_deref(),
                    recipient_attachment_ids.clone(),
                ))
            } else {
                recipient_scope_cache
                    .entry(agent_id.clone())
                    .or_insert_with(|| {
                        let mut scoped_recipient_attachment_ids = self
                            .private_recipient_attachment_ids(
                                agent_id.as_deref(),
                                recipient_attachment_ids.clone(),
                            );
                        scoped_recipient_attachment_ids = self.with_metaagent_trace_recipient_ids(
                            session_id,
                            agent_id.as_deref(),
                            scoped_recipient_attachment_ids,
                        );
                        std::sync::Arc::from(scoped_recipient_attachment_ids)
                    })
                    .clone()
            };
            if !provider_terminal {
                if let Some(agent_id) = agent_id.as_deref() {
                    trace_agent_ids.insert(agent_id.to_string());
                }
            }
            if output.kind == crate::terminal::TerminalOutputKind::ProviderReasoning {
                if let Some(agent_id) = agent_id.as_deref() {
                    let message = String::from_utf8_lossy(&bounded_bytes).into_owned();
                    self.record_workflow_thinking_trace(
                        session_id,
                        &output.provider_run_id,
                        agent_id,
                        message,
                    );
                }
            }
            terminal_outputs.push(crate::terminal::TerminalOutputAppend {
                session_id: session_id.to_string(),
                provider_run_id: output.provider_run_id,
                agent_id,
                prompt_origin: prompt_metadata.prompt_origin,
                source_attachment_id: prompt_metadata.source_attachment_id,
                kind: output.kind,
                merge_key: output.merge_key,
                recipient_attachment_ids: scoped_recipient_attachment_ids,
                bytes: bounded_bytes,
            });
        }
        let records = self.terminal_stream.fan_out_outputs(terminal_outputs);
        for agent_id in trace_agent_ids {
            self.notify_metaagent_trace_activity(session_id, Some(agent_id.as_str()));
        }
        records
    }

    pub(super) fn fan_out_terminal_output(
        &self,
        session_id: &str,
        provider_run_id: &str,
        kind: crate::terminal::TerminalOutputKind,
        merge_key: Option<String>,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> crate::terminal::TerminalOutputRecord {
        let agent_id = self
            .provider_store
            .get_run(provider_run_id)
            .ok()
            .and_then(|run| run.agent_instance_id().map(str::to_string));
        let prompt_metadata =
            self.active_prompt_transcript_metadata_for_agent(session_id, agent_id.as_deref());
        let delta_bytes = terminal_output_delta_bytes(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            &kind,
            &merge_key,
            bytes,
        );
        let bounded_bytes = bounded_terminal_output_bytes(&kind, &delta_bytes);
        self.log_provider_output_truncation(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            &kind,
            delta_bytes.len(),
            bounded_bytes.len(),
        );
        if bounded_bytes.is_empty() {
            return crate::terminal::TerminalOutputRecord {
                record_id: None,
                timestamp_ms: crate::session::unix_epoch_ms(),
                session_id: session_id.to_string(),
                provider_run_id: provider_run_id.to_string(),
                agent_id,
                prompt_id: None,
                prompt_origin: prompt_metadata.prompt_origin,
                source_attachment_id: prompt_metadata.source_attachment_id,
                kind,
                merge_key,
                recipient_attachment_ids: Vec::new(),
                pending_recipient_attachment_ids: Vec::new(),
                bytes: Vec::new(),
                external_observation_metadata: None,
            };
        }
        let recipient_attachment_ids =
            self.private_recipient_attachment_ids(agent_id.as_deref(), recipient_attachment_ids);
        let recipient_attachment_ids =
            if kind == crate::terminal::TerminalOutputKind::ProviderTerminal {
                recipient_attachment_ids
            } else {
                self.with_metaagent_trace_recipient_ids(
                    session_id,
                    agent_id.as_deref(),
                    recipient_attachment_ids,
                )
            };
        let record = self.terminal_stream.fan_out_output_with_prompt_metadata(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            kind.clone(),
            merge_key.clone(),
            prompt_metadata.prompt_origin,
            prompt_metadata.source_attachment_id.clone(),
            recipient_attachment_ids,
            &bounded_bytes,
        );
        if kind != crate::terminal::TerminalOutputKind::ProviderTerminal {
            self.notify_metaagent_trace_activity(session_id, agent_id.as_deref());
        }
        if kind != crate::terminal::TerminalOutputKind::PromptEcho
            && kind != crate::terminal::TerminalOutputKind::ProviderTerminal
        {
            let text = String::from_utf8_lossy(&bounded_bytes).into_owned();
            if kind == crate::terminal::TerminalOutputKind::ProviderReasoning {
                if let Some(agent_id) = agent_id.as_deref() {
                    self.record_workflow_thinking_trace(
                        session_id,
                        provider_run_id,
                        agent_id,
                        text.clone(),
                    );
                }
            }
            self.append_history_entry(
                session_id,
                SessionHistoryEntry::provider_output(
                    session_id,
                    provider_run_id,
                    agent_id.as_deref(),
                    kind,
                    merge_key,
                    text,
                )
                .with_prompt_origin(prompt_metadata.prompt_origin)
                .with_source_attachment_id(prompt_metadata.source_attachment_id),
            );
        }
        record
    }

    fn log_provider_output_truncation(
        &self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: Option<&str>,
        kind: &crate::terminal::TerminalOutputKind,
        original_bytes: usize,
        bounded_bytes: usize,
    ) {
        if bounded_bytes >= original_bytes {
            return;
        }
        let kind_label = format!("{kind:?}");
        let Some(suppressed_logs) = should_log_provider_output_truncation(
            session_id,
            provider_run_id,
            agent_id,
            &kind_label,
            original_bytes,
        ) else {
            return;
        };
        crate::logging::warn_with_fields(
            "daemon.provider_output",
            "truncated oversized provider terminal output",
            serde_json::json!({
                "session_id": session_id,
                "provider_run_id": provider_run_id,
                "agent_id": agent_id,
                "kind": kind_label,
                "original_bytes": original_bytes,
                "bounded_bytes": bounded_bytes,
                "omitted_bytes": original_bytes.saturating_sub(bounded_bytes),
                "suppressed_logs": suppressed_logs,
            }),
        );
    }

    pub(super) fn active_prompt_transcript_metadata_for_agent(
        &self,
        session_id: &str,
        agent_id: Option<&str>,
    ) -> ActivePromptTranscriptMetadata {
        let Some(agent_id) = agent_id else {
            return ActivePromptTranscriptMetadata::default();
        };
        self.session_store
            .get_session(session_id)
            .ok()
            .and_then(|session| {
                self.prompt_state_owner
                    .active_prompt_for_agent(&session, agent_id)
            })
            .map(|prompt| ActivePromptTranscriptMetadata {
                prompt_origin: Some(prompt.prompt_origin()),
                source_attachment_id: Some(prompt.source_attachment_id().to_string()),
            })
            .unwrap_or_default()
    }

    fn record_workflow_thinking_trace(
        &self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        message: String,
    ) {
        let workflow_node_run_id = self
            .session_store
            .get_session(session_id)
            .ok()
            .and_then(|session| {
                self.prompt_state_owner
                    .active_prompt_for_agent(&session, agent_id)
            })
            .and_then(|prompt| prompt.workflow_node_run_id().map(str::to_string));
        let Some(workflow_node_run_id) = workflow_node_run_id else {
            return;
        };
        if let Err(error) = self
            .session_store
            .record_workflow_node_thinking_trace_for_node_run(
                session_id,
                &workflow_node_run_id,
                message,
            )
        {
            crate::logging::warn_with_fields(
                "daemon.workflow",
                "failed to record workflow thinking trace",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "workflow_node_run_id": workflow_node_run_id,
                    "error": error.to_string(),
                }),
            );
        }
    }

    pub(super) fn append_history_entry(&self, session_id: &str, entry: SessionHistoryEntry) {
        let Some(entry) = bounded_history_entry(entry) else {
            return;
        };
        let _append_guard = self.transcript_history_append_guard();
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
        // Make authoritative history visible before readers can import the legacy copy.
        self.append_operational_history_entry_unlocked(&entry, None, None, None);
        if let Err(error) = self.history_store.append(&session, &entry) {
            crate::logging::warn_with_fields(
                "daemon.history",
                "failed to append provider-output session history",
                serde_json::json!({
                    "session_id": session_id,
                    "error": error.to_string(),
                }),
            );
        }
    }

    pub(super) fn append_history_entries(
        &self,
        session_id: &str,
        entries: Vec<SessionHistoryEntry>,
    ) {
        let entries = entries
            .into_iter()
            .filter_map(bounded_history_entry)
            .collect::<Vec<_>>();
        if entries.is_empty() {
            return;
        }
        let _append_guard = self.transcript_history_append_guard();
        let session = match self.session_store.get_session(session_id) {
            Ok(session) => session,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.history",
                    "skipping provider-output history append because session lookup failed",
                    serde_json::json!({
                        "session_id": session_id,
                        "entry_count": entries.len(),
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
        // Make authoritative history visible before readers can import the legacy copy.
        self.append_operational_history_entries(&entries);
        if let Err(error) = self.history_store.append_many(&session, &entries) {
            crate::logging::warn_with_fields(
                "daemon.history",
                "failed to append provider-output session history batch",
                serde_json::json!({
                    "session_id": session_id,
                    "entry_count": entries.len(),
                    "error": error.to_string(),
                }),
            );
        }
    }

    pub(super) fn append_operational_history_entry(
        &self,
        entry: &crate::history::SessionHistoryEntry,
        prompt_id_override: Option<&str>,
        workflow_run_id_override: Option<&str>,
        workflow_node_run_id_override: Option<&str>,
    ) {
        let _append_guard = self.transcript_history_append_guard();
        self.append_operational_history_entry_unlocked(
            entry,
            prompt_id_override,
            workflow_run_id_override,
            workflow_node_run_id_override,
        );
    }

    fn append_operational_history_entry_unlocked(
        &self,
        entry: &crate::history::SessionHistoryEntry,
        prompt_id_override: Option<&str>,
        workflow_run_id_override: Option<&str>,
        workflow_node_run_id_override: Option<&str>,
    ) {
        let context = self.operational_history_context(
            entry,
            prompt_id_override,
            workflow_run_id_override,
            workflow_node_run_id_override,
        );
        match self
            .operational_history_store
            .append_transcript(entry, context)
        {
            Ok(event) => {
                if is_unread_output_history_entry(entry) {
                    if let Some(agent_id) = entry.agent_id.as_deref() {
                        let _ = self.session_store.note_agent_output_sequence(
                            entry.session_id.as_str(),
                            agent_id,
                            event.sequence,
                        );
                    }
                }
                if self.history_archive_enabled() {
                    self.enqueue_history_archive_event(&event);
                }
            }
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.history",
                    "failed to append operational history",
                    serde_json::json!({
                        "session_id": entry.session_id.as_str(),
                        "error": error.to_string(),
                    }),
                );
            }
        }
    }

    fn operational_history_context(
        &self,
        entry: &crate::history::SessionHistoryEntry,
        prompt_id_override: Option<&str>,
        workflow_run_id_override: Option<&str>,
        workflow_node_run_id_override: Option<&str>,
    ) -> crate::history::HistoryEventTurnContext {
        let active_turn = entry
            .provider_run_id
            .as_deref()
            .and_then(|provider_run_id| self.active_turns.get(provider_run_id));
        self.operational_history_context_with_active_turn(
            entry,
            prompt_id_override,
            workflow_run_id_override,
            workflow_node_run_id_override,
            active_turn.as_ref(),
        )
    }

    fn operational_history_context_with_active_turn(
        &self,
        entry: &crate::history::SessionHistoryEntry,
        prompt_id_override: Option<&str>,
        workflow_run_id_override: Option<&str>,
        workflow_node_run_id_override: Option<&str>,
        active_turn: Option<&crate::app::ActiveTurnState>,
    ) -> crate::history::HistoryEventTurnContext {
        crate::app::HistoryEventContextResolver::new(
            self.provider_store.clone(),
            self.session_store.clone(),
            self.prompt_state_owner.clone(),
            self.active_turns.clone(),
        )
        .resolve_with_overrides(
            entry,
            crate::app::HistoryEventContextOverrides {
                prompt_id: prompt_id_override,
                workflow_run_id: workflow_run_id_override,
                workflow_node_run_id: workflow_node_run_id_override,
            },
            active_turn,
        )
    }

    fn history_archive_enabled(&self) -> bool {
        self.config_projection
            .snapshot()
            .user_config
            .history
            .archive
            .mode
            == crate::config::HistoryArchiveMode::External
    }

    fn enqueue_history_archive_event(&self, event: &crate::history::HistoryEvent) {
        if let Err(error) = self
            .operational_history_store
            .enqueue_archive_events(std::slice::from_ref(event))
        {
            crate::logging::warn_with_fields(
                "daemon.history",
                "failed to enqueue history archive event",
                serde_json::json!({
                    "session_id": event.session_id.as_deref(),
                    "error": error.to_string(),
                }),
            );
        }
    }

    fn append_operational_history_entries(&self, entries: &[crate::history::SessionHistoryEntry]) {
        if entries.is_empty() {
            return;
        }
        let active_turns = self.active_turns.snapshot();
        let mut prepared = Vec::with_capacity(entries.len());
        for entry in entries {
            let active_turn = entry
                .provider_run_id
                .as_deref()
                .and_then(|provider_run_id| active_turns.get(provider_run_id));
            let context = self.operational_history_context_with_active_turn(
                entry,
                None,
                None,
                None,
                active_turn,
            );
            prepared.push((entry, context));
        }
        match self.operational_history_store.append_transcripts(prepared) {
            Ok(events) => {
                for (entry, event) in entries.iter().zip(events.iter()) {
                    if is_unread_output_history_entry(entry) {
                        if let Some(agent_id) = entry.agent_id.as_deref() {
                            let _ = self.session_store.note_agent_output_sequence(
                                entry.session_id.as_str(),
                                agent_id,
                                event.sequence,
                            );
                        }
                    }
                }
                if self.history_archive_enabled() {
                    if let Err(error) = self
                        .operational_history_store
                        .enqueue_archive_events(&events)
                    {
                        crate::logging::warn_with_fields(
                            "daemon.history",
                            "failed to enqueue history archive events",
                            serde_json::json!({
                                "session_id": entries.first().map(|entry| entry.session_id.as_str()),
                                "entry_count": entries.len(),
                                "error": error.to_string(),
                            }),
                        );
                    }
                }
            }
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.history",
                    "failed to append operational history batch",
                    serde_json::json!({
                        "session_id": entries.first().map(|entry| entry.session_id.as_str()),
                        "entry_count": entries.len(),
                        "error": error.to_string(),
                    }),
                );
            }
        }
    }

    pub(super) fn append_user_prompt_history(
        &self,
        session_id: &str,
        source_attachment_id: &str,
        agent_id: &str,
        prompt: &str,
        attachments: &[crate::session::PromptAttachment],
        prompt_origin: crate::session::PromptOrigin,
        prompt_id: Option<&str>,
        workflow_run_id: Option<&str>,
        workflow_node_run_id: Option<&str>,
        timestamp_ms: Option<u64>,
    ) -> Result<(), DaemonError> {
        let _append_guard = self.transcript_history_append_guard();
        let session = self.session_snapshot_without_projection_update(session_id)?;
        let mut entry = crate::history::SessionHistoryEntry::user_prompt_with_attachments(
            session_id,
            source_attachment_id,
            agent_id,
            crate::prompt_transcript::render_prompt_transcript(prompt, attachments),
            attachments,
        )
        .with_prompt_origin(prompt_origin);
        if let Some(timestamp_ms) = timestamp_ms {
            entry.timestamp_ms = timestamp_ms;
        }
        if let Some(prompt_id) = prompt_id {
            entry.merge_key = Some(user_prompt_history_merge_key(prompt_id));
        }
        if let Err(error) = self.history_store.append(&session, &entry) {
            crate::logging::warn_with_fields(
                "daemon.history",
                "failed to append prompt session history",
                serde_json::json!({
                    "session_id": session_id,
                    "error": error.to_string(),
                }),
            );
        }
        self.append_operational_history_entry_unlocked(
            &entry,
            prompt_id,
            workflow_run_id,
            workflow_node_run_id,
        );
        Ok(())
    }

    pub(super) fn record_started_user_prompt(
        &self,
        session_id: &str,
        source_attachment_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<u64, DaemonError> {
        // A queued prompt's creation time is its queue-admission time, not its
        // turn start. Persist the activation time here so history hydration
        // cannot place a promoted prompt inside the preceding active turn.
        let prompt_sent_at_ms = crate::session::unix_epoch_ms();
        let prompt_text = crate::prompt_transcript::workflow_prompt_history_text(prompt);
        self.append_user_prompt_history(
            session_id,
            source_attachment_id,
            prompt.target_agent_id(),
            &prompt_text,
            prompt.attachments(),
            prompt.prompt_origin(),
            Some(prompt.id()),
            prompt.workflow_run_id(),
            prompt.workflow_node_run_id(),
            Some(prompt_sent_at_ms),
        )?;
        self.agent_store
            .note_prompt_sent_at(prompt.target_agent_id(), prompt_sent_at_ms)?;
        self.session_store.note_prompt_sent(
            session_id,
            prompt.target_agent_id(),
            prompt_sent_at_ms,
        )?;
        Ok(prompt_sent_at_ms)
    }

    pub(super) fn append_steering_prompt_history(
        &self,
        session_id: &str,
        provider_run_id: &str,
        active_prompt_id: &str,
        source_attachment_id: &str,
        agent_id: &str,
        prompt_id: &str,
        prompt: &str,
        attachments: &[crate::session::PromptAttachment],
    ) -> Result<(), DaemonError> {
        let _append_guard = self.transcript_history_append_guard();
        let agent = self.agent_store.get_agent(agent_id)?;
        let (history_provider_run_id, provider_run) =
            if let Some(remote_execution) = agent.remote_execution() {
                let projected_provider_run_id = crate::provider::projected_leased_provider_run_id(
                    &remote_execution.leased_agent_id,
                    provider_run_id,
                );
                let provider_run = self
                    .provider_run_projection
                    .get(&projected_provider_run_id)
                    .or_else(|| {
                        self.provider_run_projection
                            .get_for_agent(session_id, agent_id)
                    })
                    .filter(|run| run.session_id() == session_id);
                (projected_provider_run_id, provider_run)
            } else {
                let provider_run = self
                    .provider_store
                    .get_run(provider_run_id)
                    .ok()
                    .or_else(|| self.provider_run_projection.get(provider_run_id));
                if provider_run
                    .as_ref()
                    .is_some_and(|run| run.session_id() != session_id)
                {
                    return Err(DaemonError::ProviderRunNotInSession {
                        session_id: session_id.to_string(),
                        provider_run_id: provider_run_id.to_string(),
                    });
                }
                (provider_run_id.to_string(), provider_run)
            };
        let mut metadata = std::collections::BTreeMap::new();
        metadata.insert(
            "merge_key".to_string(),
            serde_json::Value::String(crate::history::steering_prompt_merge_key(prompt_id)),
        );
        metadata.insert(
            "source_attachment_id".to_string(),
            serde_json::Value::String(source_attachment_id.to_string()),
        );
        if !attachments.is_empty() {
            metadata.insert(
                "attachments".to_string(),
                serde_json::to_value(
                    attachments
                        .iter()
                        .map(crate::history::SessionHistoryPromptAttachment::from_prompt_attachment)
                        .collect::<Vec<_>>(),
                )
                .unwrap_or(serde_json::Value::Null),
            );
        }
        let active_turn = self.active_turns.get(&history_provider_run_id);
        let context = crate::history::HistoryEventTurnContext {
            session_id: Some(session_id.to_string()),
            agent_id: Some(agent_id.to_string()),
            provider: Some(
                provider_run
                    .as_ref()
                    .map(|run| run.provider())
                    .unwrap_or_else(|| agent.provider())
                    .to_string(),
            ),
            model: provider_run
                .as_ref()
                .map(|run| run.model().to_string())
                .or_else(|| agent.model().map(str::to_string)),
            turn_id: active_turn
                .as_ref()
                .map(|turn| turn.trace_id.clone())
                .or_else(|| Some(active_prompt_id.to_string())),
            prompt_id: Some(active_prompt_id.to_string()),
            provider_run_id: Some(history_provider_run_id),
            provider_session_id: provider_run
                .as_ref()
                .and_then(|run| run.provider_session_id())
                .map(str::to_string),
            worktree_path: provider_run
                .as_ref()
                .and_then(|run| run.working_directory())
                .map(|path| path.display().to_string()),
            ..crate::history::HistoryEventTurnContext::default()
        };
        let event = self.operational_history_store.append_operational_event(
            crate::history::HistoryEventKind::UserPrompt,
            Some(crate::history::HistoryEventRole::User),
            Some(crate::prompt_transcript::render_prompt_transcript(
                prompt,
                attachments,
            )),
            metadata,
            context,
        )?;
        if self.history_archive_enabled() {
            self.enqueue_history_archive_event(&event);
        }
        Ok(())
    }

    fn transcript_history_append_guard(&self) -> std::sync::MutexGuard<'_, ()> {
        self.transcript_history_append_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) fn echo_prompt_to_other_attachments(
        &self,
        session_id: &str,
        provider_run_id: &str,
        prompt_id: &str,
        source_attachment_id: &str,
        prompt: &str,
        attachments: &[crate::session::PromptAttachment],
    ) {
        // Kernel-internal recovery envelopes carry provider resume text, not
        // user input. The local dispatch runtime guards its own call site, but
        // remote-lease dispatchers reach this helper too; centralize the
        // guard so no caller can leak the envelope into prompt-echo output.
        if crate::runtime::state::is_internal_recovery_prompt_attachment(source_attachment_id) {
            return;
        }
        let recipient_attachment_ids = self
            .attachment_store
            .list_session_attachment_ids(session_id)
            .into_iter()
            .filter(|attachment_id| attachment_id != source_attachment_id)
            .collect::<Vec<_>>();
        self.echo_prompt_to_attachments(
            session_id,
            provider_run_id,
            None,
            prompt_id,
            source_attachment_id,
            prompt,
            attachments,
            recipient_attachment_ids,
            None,
            None,
        );
    }

    pub(super) fn echo_promoted_queued_prompt_to_attachments(
        &self,
        session_id: &str,
        provider_run_id: &str,
        prompt_id: &str,
        source_attachment_id: &str,
        prompt: &str,
        attachments: &[crate::session::PromptAttachment],
    ) {
        let recipient_attachment_ids = self
            .attachment_store
            .list_session_attachment_ids(session_id);
        self.echo_prompt_to_attachments(
            session_id,
            provider_run_id,
            None,
            prompt_id,
            source_attachment_id,
            prompt,
            attachments,
            recipient_attachment_ids,
            None,
            None,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn echo_steering_prompt_to_other_attachments(
        &self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        prompt_id: &str,
        prompt_source_attachment_id: &str,
        steering_attachment_id: &str,
        prompt: &str,
        attachments: &[crate::session::PromptAttachment],
        prompt_origin: crate::session::PromptOrigin,
    ) {
        let recipient_attachment_ids = self
            .attachment_store
            .list_session_attachment_ids(session_id)
            .into_iter()
            .filter(|attachment_id| attachment_id != steering_attachment_id)
            .collect::<Vec<_>>();
        self.echo_prompt_to_attachments(
            session_id,
            provider_run_id,
            Some(agent_id),
            prompt_id,
            prompt_source_attachment_id,
            prompt,
            attachments,
            recipient_attachment_ids,
            Some(prompt_origin),
            Some(crate::history::steering_prompt_merge_key(prompt_id)),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn echo_prompt_to_attachments(
        &self,
        session_id: &str,
        provider_run_id: &str,
        agent_id_override: Option<&str>,
        prompt_id: &str,
        source_attachment_id: &str,
        prompt: &str,
        attachments: &[crate::session::PromptAttachment],
        recipient_attachment_ids: Vec<String>,
        prompt_origin_override: Option<crate::session::PromptOrigin>,
        merge_key: Option<String>,
    ) {
        if recipient_attachment_ids.is_empty() {
            return;
        }
        let mut bytes =
            crate::prompt_transcript::render_prompt_transcript(prompt, attachments).into_bytes();
        if !bytes.ends_with(b"\n") {
            bytes.push(b'\n');
        }
        let agent_id = agent_id_override.map(str::to_string).or_else(|| {
            self.provider_store
                .get_run(provider_run_id)
                .ok()
                .and_then(|run| run.agent_instance_id().map(str::to_string))
        });
        let prompt_origin = prompt_origin_override.or_else(|| {
            agent_id.as_deref().and_then(|agent_id| {
                self.session_store
                    .get_session(session_id)
                    .ok()
                    .and_then(|session| {
                        self.prompt_state_owner
                            .active_prompt_for_agent(&session, agent_id)
                    })
                    .filter(|prompt| prompt.id() == prompt_id)
                    .map(|prompt| prompt.prompt_origin())
            })
        });
        let recipient_attachment_ids =
            self.private_recipient_attachment_ids(agent_id.as_deref(), recipient_attachment_ids);
        let recipient_attachment_ids = self.with_metaagent_trace_recipient_ids(
            session_id,
            agent_id.as_deref(),
            recipient_attachment_ids,
        );
        self.terminal_stream.fan_out_prompt_output_with_merge_key(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            prompt_id,
            prompt_origin,
            source_attachment_id,
            merge_key,
            recipient_attachment_ids,
            &bytes,
        );
        self.notify_metaagent_trace_activity(session_id, agent_id.as_deref());
    }

    pub(super) fn private_recipient_attachment_ids(
        &self,
        agent_id: Option<&str>,
        recipient_attachment_ids: Vec<String>,
    ) -> Vec<String> {
        let Some(agent_id) = agent_id else {
            return recipient_attachment_ids;
        };
        let Ok(agent) = self.agent_store.get_agent(agent_id) else {
            return Vec::new();
        };
        self.attachment_store
            .filter_attachment_ids_for_user(recipient_attachment_ids, agent.owner_user_id())
    }

    pub(super) fn with_metaagent_trace_recipient_ids(
        &self,
        session_id: &str,
        agent_id: Option<&str>,
        mut recipient_attachment_ids: Vec<String>,
    ) -> Vec<String> {
        let Some(agent_id) = agent_id else {
            return recipient_attachment_ids;
        };
        for recipient in self
            .metaagent_trace_subscriptions
            .recipient_attachment_ids_for_target(session_id, agent_id)
        {
            if !recipient_attachment_ids
                .iter()
                .any(|existing| existing == &recipient)
            {
                recipient_attachment_ids.push(recipient);
            }
        }
        recipient_attachment_ids
    }

    pub(super) fn notify_metaagent_trace_activity(&self, session_id: &str, agent_id: Option<&str>) {
        if let Some(agent_id) = agent_id {
            self.metaagent_trace_subscriptions
                .record_target_activity(session_id, agent_id);
        }
    }
}

fn bounded_history_entry(mut entry: SessionHistoryEntry) -> Option<SessionHistoryEntry> {
    let kind = match entry.kind {
        crate::history::SessionHistoryEntryKind::ProviderOutput => {
            Some(crate::terminal::TerminalOutputKind::ProviderOutput)
        }
        crate::history::SessionHistoryEntryKind::ProviderReasoning => {
            Some(crate::terminal::TerminalOutputKind::ProviderReasoning)
        }
        crate::history::SessionHistoryEntryKind::ProviderTool => {
            Some(crate::terminal::TerminalOutputKind::ProviderTool)
        }
        crate::history::SessionHistoryEntryKind::ProviderError => {
            Some(crate::terminal::TerminalOutputKind::ProviderError)
        }
        crate::history::SessionHistoryEntryKind::ProviderStatus => {
            Some(crate::terminal::TerminalOutputKind::ProviderStatus)
        }
        crate::history::SessionHistoryEntryKind::UserPrompt
        | crate::history::SessionHistoryEntryKind::Notice => None,
    };
    if let Some(kind) = kind {
        entry.text =
            String::from_utf8_lossy(&bounded_terminal_output_bytes(&kind, entry.text.as_bytes()))
                .into_owned();
    }
    should_persist_provider_tool_history(&entry).then_some(entry)
}

fn user_prompt_history_merge_key(prompt_id: &str) -> String {
    format!("prompt:{prompt_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider_tool_entry(status: &str, output: &str) -> SessionHistoryEntry {
        SessionHistoryEntry::provider_output(
            "runtime-bounded-history-session",
            "runtime-bounded-history-run",
            Some("runtime-bounded-history-agent"),
            crate::terminal::TerminalOutputKind::ProviderTool,
            Some("runtime-bounded-history-tool".to_string()),
            serde_json::json!({
                "id": "runtime-bounded-history-tool",
                "status": status,
                "output": output,
            })
            .to_string(),
        )
    }

    #[test]
    fn runtime_history_bounds_and_deduplicates_cumulative_tool_updates() {
        let first = bounded_history_entry(provider_tool_entry("running", &"x".repeat(1024 * 1024)))
            .expect("first running tool state should persist");
        assert!(
            first.text.len()
                <= crate::provider_output_policy::output_bounds::MAX_PROVIDER_OUTPUT_RECORD_BYTES
        );
        assert!(first.text.contains("\"arroba_truncated\":true"));

        assert!(
            bounded_history_entry(provider_tool_entry("running", &"y".repeat(2 * 1024 * 1024)))
                .is_none(),
            "same-status cumulative updates should not multiply history"
        );

        assert!(
            bounded_history_entry(provider_tool_entry("completed", "done")).is_some(),
            "terminal status transition should persist"
        );
    }
}
