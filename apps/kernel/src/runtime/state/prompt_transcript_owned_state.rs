//! Prompt transcript fan-out to terminal streams and history stores.

use super::*;

pub(super) struct TerminalOutputBatchAppend {
    pub(super) provider_run_id: String,
    pub(super) agent_id: Option<String>,
    pub(super) kind: crate::terminal::TerminalOutputKind,
    pub(super) merge_key: Option<String>,
    pub(super) bytes: Vec<u8>,
    pub(super) history_text: Option<String>,
}

impl KernelRuntimeOwnedState {
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
        let mut terminal_outputs = Vec::with_capacity(outputs.len());
        for output in outputs {
            let agent_id = output.agent_id;
            let scoped_recipient_attachment_ids = recipient_scope_cache
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
                .clone();
            if let Some(agent_id) = agent_id.as_deref() {
                trace_agent_ids.insert(agent_id.to_string());
            }
            if output.kind == crate::terminal::TerminalOutputKind::ProviderReasoning {
                if let Some(agent_id) = agent_id.as_deref() {
                    let message = output
                        .history_text
                        .clone()
                        .unwrap_or_else(|| String::from_utf8_lossy(&output.bytes).into_owned());
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
                kind: output.kind,
                merge_key: output.merge_key,
                recipient_attachment_ids: scoped_recipient_attachment_ids,
                bytes: output.bytes,
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
        let recipient_attachment_ids =
            self.private_recipient_attachment_ids(agent_id.as_deref(), recipient_attachment_ids);
        let recipient_attachment_ids = self.with_metaagent_trace_recipient_ids(
            session_id,
            agent_id.as_deref(),
            recipient_attachment_ids,
        );
        let record = self.terminal_stream.fan_out_output(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            kind.clone(),
            merge_key.clone(),
            recipient_attachment_ids,
            bytes,
        );
        self.notify_metaagent_trace_activity(session_id, agent_id.as_deref());
        if kind != crate::terminal::TerminalOutputKind::PromptEcho {
            let text = String::from_utf8_lossy(bytes).into_owned();
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
                ),
            );
        }
        record
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
        }
        self.append_operational_history_entry(&entry, None, None, None);
    }

    pub(super) fn append_history_entries(
        &self,
        session_id: &str,
        entries: Vec<SessionHistoryEntry>,
    ) {
        if entries.is_empty() {
            return;
        }
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
        self.append_operational_history_entries(&entries);
    }

    pub(super) fn append_operational_history_entry(
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
        let provider_run = entry
            .provider_run_id
            .as_deref()
            .and_then(|provider_run_id| self.provider_store.get_run(provider_run_id).ok());
        let agent_id = entry.agent_id.clone().or_else(|| {
            provider_run
                .as_ref()
                .and_then(|run| run.agent_instance_id().map(str::to_string))
        });
        let session = self.session_store.get_session(&entry.session_id).ok();
        let active_prompt = session.as_ref().and_then(|session| {
            agent_id.as_deref().and_then(|agent_id| {
                self.prompt_state_owner
                    .active_prompt_for_agent(session, agent_id)
            })
        });
        let active_turn = entry
            .provider_run_id
            .as_deref()
            .and_then(|provider_run_id| self.active_turns.get(provider_run_id));
        let prompt_id = prompt_id_override
            .map(str::to_string)
            .or_else(|| active_turn.as_ref().map(|turn| turn.prompt_id.clone()))
            .or_else(|| active_prompt.as_ref().map(|prompt| prompt.id().to_string()));
        let turn_id = active_turn
            .as_ref()
            .map(|turn| turn.trace_id.clone())
            .or_else(|| prompt_id.clone());
        crate::history::HistoryEventTurnContext {
            session_id: Some(entry.session_id.clone()),
            agent_id,
            provider: provider_run.as_ref().map(|run| run.provider().to_string()),
            model: provider_run.as_ref().map(|run| run.model().to_string()),
            turn_id,
            prompt_id,
            provider_run_id: entry.provider_run_id.clone(),
            provider_session_id: provider_run
                .as_ref()
                .and_then(|run| run.provider_session_id().map(str::to_string)),
            workflow_run_id: workflow_run_id_override.map(str::to_string).or_else(|| {
                active_prompt
                    .as_ref()
                    .and_then(|prompt| prompt.workflow_run_id().map(str::to_string))
            }),
            workflow_node_id: workflow_node_run_id_override
                .map(str::to_string)
                .or_else(|| {
                    active_prompt
                        .as_ref()
                        .and_then(|prompt| prompt.workflow_node_run_id().map(str::to_string))
                }),
            worktree_path: provider_run.as_ref().and_then(|run| {
                run.working_directory()
                    .map(|path| path.display().to_string())
            }),
            ..crate::history::HistoryEventTurnContext::default()
        }
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
        let mut active_turns = self.active_turns.snapshot();
        let mut prepared = Vec::with_capacity(entries.len());
        for entry in entries {
            let provider_run = entry
                .provider_run_id
                .as_deref()
                .and_then(|provider_run_id| self.provider_store.get_run(provider_run_id).ok());
            let agent_id = entry.agent_id.clone().or_else(|| {
                provider_run
                    .as_ref()
                    .and_then(|run| run.agent_instance_id().map(str::to_string))
            });
            let session = self.session_store.get_session(&entry.session_id).ok();
            let active_prompt = session.as_ref().and_then(|session| {
                agent_id.as_deref().and_then(|agent_id| {
                    self.prompt_state_owner
                        .active_prompt_for_agent(session, agent_id)
                })
            });
            let active_turn = entry
                .provider_run_id
                .as_deref()
                .and_then(|provider_run_id| active_turns.remove(provider_run_id));
            let prompt_id = active_turn
                .as_ref()
                .map(|turn| turn.prompt_id.clone())
                .or_else(|| active_prompt.as_ref().map(|prompt| prompt.id().to_string()));
            let turn_id = active_turn
                .as_ref()
                .map(|turn| turn.trace_id.clone())
                .or_else(|| prompt_id.clone());
            let context = crate::history::HistoryEventTurnContext {
                session_id: Some(entry.session_id.clone()),
                agent_id,
                provider: provider_run.as_ref().map(|run| run.provider().to_string()),
                model: provider_run.as_ref().map(|run| run.model().to_string()),
                turn_id,
                prompt_id,
                provider_run_id: entry.provider_run_id.clone(),
                provider_session_id: provider_run
                    .as_ref()
                    .and_then(|run| run.provider_session_id().map(str::to_string)),
                workflow_run_id: active_prompt
                    .as_ref()
                    .and_then(|prompt| prompt.workflow_run_id().map(str::to_string)),
                workflow_node_id: active_prompt
                    .as_ref()
                    .and_then(|prompt| prompt.workflow_node_run_id().map(str::to_string)),
                worktree_path: provider_run.as_ref().and_then(|run| {
                    run.working_directory()
                        .map(|path| path.display().to_string())
                }),
                ..crate::history::HistoryEventTurnContext::default()
            };
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
        prompt_id: Option<&str>,
        workflow_run_id: Option<&str>,
        workflow_node_run_id: Option<&str>,
    ) -> Result<(), DaemonError> {
        let session = self.session_snapshot(session_id)?;
        let mut entry = crate::history::SessionHistoryEntry::user_prompt_with_attachments(
            session_id,
            source_attachment_id,
            agent_id,
            crate::prompt_transcript::render_prompt_transcript(prompt, attachments),
            attachments,
        );
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
        self.append_operational_history_entry(
            &entry,
            prompt_id,
            workflow_run_id,
            workflow_node_run_id,
        );
        Ok(())
    }

    pub(super) fn replace_user_prompt_history_by_prompt_id(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) {
        let merge_key = user_prompt_history_merge_key(prompt.id());
        let mut entry = crate::history::SessionHistoryEntry::user_prompt_with_attachments(
            session_id,
            prompt.source_attachment_id(),
            prompt.target_agent_id(),
            crate::prompt_transcript::render_prompt_transcript(
                prompt.prompt(),
                prompt.attachments(),
            ),
            prompt.attachments(),
        );
        entry.merge_key = Some(merge_key.clone());
        let session = match self.session_store.get_session(session_id) {
            Ok(session) => session,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.history",
                    "skipping queued prompt history replacement because session lookup failed",
                    serde_json::json!({
                        "session_id": session_id,
                        "prompt_id": prompt.id(),
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
        match self
            .history_store
            .replace_by_merge_key(&session, &merge_key, &entry)
        {
            Ok(true) => {}
            Ok(false) => {
                if let Err(error) = self.history_store.append(&session, &entry) {
                    crate::logging::warn_with_fields(
                        "daemon.history",
                        "failed to append queued prompt history after replacement miss",
                        serde_json::json!({
                            "session_id": session_id,
                            "prompt_id": prompt.id(),
                            "error": error.to_string(),
                        }),
                    );
                }
            }
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.history",
                    "failed to replace queued prompt legacy history",
                    serde_json::json!({
                        "session_id": session_id,
                        "prompt_id": prompt.id(),
                        "error": error.to_string(),
                    }),
                );
            }
        }

        let context = self.operational_history_context(
            &entry,
            Some(prompt.id()),
            prompt.workflow_run_id(),
            prompt.workflow_node_run_id(),
        );
        match self
            .operational_history_store
            .replace_transcript_by_merge_key(
                session_id,
                entry.agent_id.as_deref(),
                &merge_key,
                &entry,
                context,
            ) {
            Ok(Some(event)) => {
                if self.history_archive_enabled() {
                    self.enqueue_history_archive_event(&event);
                }
            }
            Ok(None) => {
                self.append_operational_history_entry(
                    &entry,
                    Some(prompt.id()),
                    prompt.workflow_run_id(),
                    prompt.workflow_node_run_id(),
                );
            }
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.history",
                    "failed to replace queued prompt operational history",
                    serde_json::json!({
                        "session_id": session_id,
                        "prompt_id": prompt.id(),
                        "error": error.to_string(),
                    }),
                );
            }
        }
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
        let recipient_attachment_ids = self
            .attachment_store
            .list_session_attachment_ids(session_id)
            .into_iter()
            .filter(|attachment_id| attachment_id != source_attachment_id)
            .collect::<Vec<_>>();
        if recipient_attachment_ids.is_empty() {
            return;
        }
        let mut bytes =
            crate::prompt_transcript::render_prompt_transcript(prompt, attachments).into_bytes();
        if !bytes.ends_with(b"\n") {
            bytes.push(b'\n');
        }
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
        self.terminal_stream.fan_out_prompt_output(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            prompt_id,
            source_attachment_id,
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

fn user_prompt_history_merge_key(prompt_id: &str) -> String {
    format!("prompt:{prompt_id}")
}

fn is_unread_output_history_entry(entry: &crate::history::SessionHistoryEntry) -> bool {
    matches!(
        entry.kind,
        crate::history::SessionHistoryEntryKind::ProviderOutput
            | crate::history::SessionHistoryEntryKind::ProviderReasoning
            | crate::history::SessionHistoryEntryKind::ProviderTool
            | crate::history::SessionHistoryEntryKind::ProviderError
            | crate::history::SessionHistoryEntryKind::Notice
    )
}
