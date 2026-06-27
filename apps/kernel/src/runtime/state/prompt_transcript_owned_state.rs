//! Prompt transcript fan-out to terminal streams and history stores.

use super::*;

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
        self.history_projection.append(entry);
    }

    pub(super) fn append_operational_history_entry(
        &self,
        entry: &crate::history::SessionHistoryEntry,
        prompt_id_override: Option<&str>,
        workflow_run_id_override: Option<&str>,
        workflow_node_run_id_override: Option<&str>,
    ) {
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
            .and_then(|provider_run_id| self.active_turns.snapshot().remove(provider_run_id));
        let prompt_id = prompt_id_override
            .map(str::to_string)
            .or_else(|| active_turn.as_ref().map(|turn| turn.prompt_id.clone()))
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
        };
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
        let entry = crate::history::SessionHistoryEntry::user_prompt_with_attachments(
            session_id,
            source_attachment_id,
            agent_id,
            crate::prompt_transcript::render_prompt_transcript(prompt, attachments),
            attachments,
        );
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
        self.history_projection.append(entry);
        Ok(())
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
