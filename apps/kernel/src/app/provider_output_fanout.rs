use crate::agent::AgentServiceStore;
use crate::app::ActiveTurnStore;
use crate::app::DaemonApp;
use crate::attachment::AttachmentServiceStore;
use crate::history::{OperationalHistoryStore, SessionHistoryEntry, SessionHistoryStore};
use crate::provider::ProviderProcessServiceStore;
use crate::provider_output_policy::output_bounds::{
    bounded_terminal_output_bytes, should_log_provider_output_truncation,
    terminal_output_delta_bytes,
};
use crate::provider_output_policy::tool_history::{
    is_unread_output_history_entry, should_persist_provider_tool_history,
};
use crate::runtime::prompt_state::PromptStateOwner;
use crate::session::SessionStateStore;
use crate::terminal::{
    RuntimeNoticeRecord, TerminalOutputKind, TerminalOutputRecord, TerminalStreamStore,
};
pub(crate) struct ProviderOutputFanout {
    provider_store: ProviderProcessServiceStore,
    prompt_state_owner: PromptStateOwner,
    active_turns: ActiveTurnStore,
    agent_store: AgentServiceStore,
    attachment_store: AttachmentServiceStore,
    session_store: SessionStateStore,
    history_store: SessionHistoryStore,
    operational_history_store: OperationalHistoryStore,
    archive_enabled: bool,
    terminal: TerminalStreamStore,
    metaagent_trace_subscriptions: crate::runtime::metaagent_trace::MetaagentTraceSubscriptionStore,
}

#[derive(Debug, Clone, Default)]
struct ActivePromptTranscriptMetadata {
    prompt_origin: Option<crate::session::PromptOrigin>,
    source_attachment_id: Option<String>,
}

impl ProviderOutputFanout {
    pub(crate) fn new(app: &DaemonApp) -> Self {
        Self {
            provider_store: app.providers.clone(),
            prompt_state_owner: app.prompt_state_owner(),
            active_turns: app.active_turn_store(),
            agent_store: app.agents.clone(),
            attachment_store: app.attachments.clone(),
            session_store: app.sessions.clone(),
            history_store: app.history_store(),
            operational_history_store: app.operational_history_store(),
            archive_enabled: app.history_archive_enabled(),
            terminal: app.terminal.clone(),
            metaagent_trace_subscriptions: app.metaagent_trace_subscription_store(),
        }
    }

    pub(crate) fn fan_out(
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
        self.fan_out_for_agent(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            kind,
            merge_key,
            recipient_attachment_ids,
            bytes,
        )
    }

    pub(crate) fn fan_out_for_agent(
        &self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: Option<&str>,
        kind: TerminalOutputKind,
        merge_key: Option<String>,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        let delta_bytes = terminal_output_delta_bytes(
            session_id,
            provider_run_id,
            agent_id,
            &kind,
            &merge_key,
            bytes,
        );
        let bounded_bytes = bounded_terminal_output_bytes(&kind, &delta_bytes);
        if bounded_bytes.len() < delta_bytes.len() {
            let kind_label = format!("{:?}", kind);
            if let Some(suppressed_logs) = should_log_provider_output_truncation(
                session_id,
                provider_run_id,
                agent_id,
                &kind_label,
                delta_bytes.len(),
            ) {
                crate::logging::warn_with_fields(
                    "daemon.provider_output",
                    "truncated oversized provider terminal output",
                    serde_json::json!({
                        "session_id": session_id,
                        "provider_run_id": provider_run_id,
                        "agent_id": agent_id,
                        "kind": kind_label,
                        "original_bytes": delta_bytes.len(),
                        "bounded_bytes": bounded_bytes.len(),
                        "omitted_bytes": delta_bytes.len().saturating_sub(bounded_bytes.len()),
                        "suppressed_logs": suppressed_logs,
                    }),
                );
            }
        }
        if bounded_bytes.is_empty() {
            return TerminalOutputRecord {
                record_id: None,
                timestamp_ms: crate::session::unix_epoch_ms(),
                session_id: session_id.to_string(),
                provider_run_id: provider_run_id.to_string(),
                agent_id: agent_id.map(str::to_string),
                prompt_id: None,
                prompt_origin: None,
                source_attachment_id: None,
                kind,
                merge_key,
                recipient_attachment_ids: Vec::new(),
                pending_recipient_attachment_ids: Vec::new(),
                bytes: Vec::new(),
                external_observation_metadata: None,
            };
        }
        let recipient_attachment_ids =
            self.private_recipient_attachment_ids(agent_id, recipient_attachment_ids);
        let recipient_attachment_ids = if kind == TerminalOutputKind::ProviderTerminal {
            recipient_attachment_ids
        } else {
            self.with_metaagent_trace_recipient_ids(session_id, agent_id, recipient_attachment_ids)
        };
        let prompt_metadata =
            self.active_prompt_transcript_metadata_for_agent(session_id, agent_id);
        let record = self.terminal.fan_out_output_with_prompt_metadata(
            session_id,
            provider_run_id,
            agent_id,
            kind.clone(),
            merge_key.clone(),
            prompt_metadata.prompt_origin,
            prompt_metadata.source_attachment_id.clone(),
            recipient_attachment_ids,
            &bounded_bytes,
        );
        if kind != TerminalOutputKind::ProviderTerminal {
            self.notify_metaagent_trace_activity(session_id, agent_id);
        }
        if kind != TerminalOutputKind::PromptEcho && kind != TerminalOutputKind::ProviderTerminal {
            let text = String::from_utf8_lossy(&bounded_bytes).into_owned();
            if kind == TerminalOutputKind::ProviderReasoning {
                if let Some(agent_id) = agent_id {
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
                    agent_id,
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

    fn active_prompt_transcript_metadata_for_agent(
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

    pub(crate) fn record_notice(
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
        self.record_notice_for_agent(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            recipient_attachment_ids,
            message,
        )
    }

    pub(crate) fn record_notice_for_agent(
        &self,
        session_id: &str,
        provider_run_id: Option<&str>,
        agent_id: Option<&str>,
        recipient_attachment_ids: Vec<String>,
        message: impl Into<String>,
    ) -> RuntimeNoticeRecord {
        let message = message.into();
        let recipient_attachment_ids =
            self.private_recipient_attachment_ids(agent_id, recipient_attachment_ids);
        let recipient_attachment_ids =
            self.with_metaagent_trace_recipient_ids(session_id, agent_id, recipient_attachment_ids);
        let record = self.terminal.record_notice(
            session_id,
            provider_run_id,
            agent_id,
            recipient_attachment_ids,
            message.clone(),
        );
        self.notify_metaagent_trace_activity(session_id, agent_id);
        self.append_history_entry(
            session_id,
            SessionHistoryEntry::notice(session_id, provider_run_id, agent_id, message),
        );
        record
    }

    pub(crate) fn record_assistant_message_completion(
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
        self.record_assistant_message_completion_for_agent(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            recipient_attachment_ids,
            message_id,
            completed_at_ms,
        );
    }

    pub(crate) fn record_assistant_message_completion_for_agent(
        &self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: Option<&str>,
        recipient_attachment_ids: Vec<String>,
        message_id: &str,
        completed_at_ms: u64,
    ) {
        let recipient_attachment_ids =
            self.private_recipient_attachment_ids(agent_id, recipient_attachment_ids);
        let recipient_attachment_ids =
            self.with_metaagent_trace_recipient_ids(session_id, agent_id, recipient_attachment_ids);
        self.terminal.record_assistant_message_completion(
            session_id,
            provider_run_id,
            agent_id,
            recipient_attachment_ids,
            message_id,
            completed_at_ms,
        );
        self.notify_metaagent_trace_activity(session_id, agent_id);
    }

    fn private_recipient_attachment_ids(
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

    fn with_metaagent_trace_recipient_ids(
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

    fn notify_metaagent_trace_activity(&self, session_id: &str, agent_id: Option<&str>) {
        if let Some(agent_id) = agent_id {
            self.metaagent_trace_subscriptions
                .record_target_activity(session_id, agent_id);
        }
    }

    fn append_history_entry(&self, session_id: &str, entry: SessionHistoryEntry) {
        if !should_persist_provider_tool_history(&entry) {
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
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
        let context = crate::app::HistoryEventContextResolver::new(
            self.provider_store.clone(),
            self.session_store.clone(),
            self.prompt_state_owner.clone(),
            self.active_turns.clone(),
        )
        .resolve(&entry);
        // Make authoritative history visible before readers can import the legacy copy.
        match self
            .operational_history_store
            .append_transcript(&entry, context)
        {
            Ok(event) => {
                if is_unread_output_history_entry(&entry) {
                    if let Some(agent_id) = entry.agent_id.as_deref() {
                        let _ = self.session_store.note_agent_output_sequence(
                            session_id,
                            agent_id,
                            event.sequence,
                        );
                    }
                }
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
}
