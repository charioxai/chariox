use crate::agent::AgentServiceStore;
use crate::app::DaemonApp;
use crate::attachment::AttachmentServiceStore;
use crate::history::{
    HistoryEventTurnContext, OperationalHistoryStore, SessionHistoryEntry, SessionHistoryEntryKind,
    SessionHistoryStore,
};
use crate::provider::ProviderProcessServiceStore;
use crate::runtime::projection::SessionHistoryProjectionStore;
use crate::runtime::prompt_state::PromptStateOwner;
use crate::session::SessionStateStore;
use crate::terminal::{
    RuntimeNoticeRecord, TerminalOutputKind, TerminalOutputRecord, TerminalStreamStore,
};

pub(crate) struct ProviderOutputFanout {
    provider_store: ProviderProcessServiceStore,
    prompt_state_owner: PromptStateOwner,
    agent_store: AgentServiceStore,
    attachment_store: AttachmentServiceStore,
    session_store: SessionStateStore,
    history_store: SessionHistoryStore,
    operational_history_store: OperationalHistoryStore,
    archive_enabled: bool,
    history_projection: SessionHistoryProjectionStore,
    terminal: TerminalStreamStore,
    metaagent_trace_subscriptions: crate::runtime::metaagent_trace::MetaagentTraceSubscriptionStore,
}

impl ProviderOutputFanout {
    pub(crate) fn new(app: &DaemonApp) -> Self {
        Self {
            provider_store: app.providers.clone(),
            prompt_state_owner: app.prompt_state_owner(),
            agent_store: app.agents.clone(),
            attachment_store: app.attachments.clone(),
            session_store: app.sessions.clone(),
            history_store: app.history_store(),
            operational_history_store: app.operational_history_store(),
            archive_enabled: app.history_archive_enabled(),
            history_projection: app.session_history_projection_store(),
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
        let recipient_attachment_ids =
            self.private_recipient_attachment_ids(agent_id, recipient_attachment_ids);
        let recipient_attachment_ids =
            self.with_metaagent_trace_recipient_ids(session_id, agent_id, recipient_attachment_ids);
        let record = self.terminal.fan_out_output(
            session_id,
            provider_run_id,
            agent_id,
            kind.clone(),
            merge_key.clone(),
            recipient_attachment_ids,
            bytes,
        );
        self.notify_metaagent_trace_activity(session_id, agent_id);
        if kind != TerminalOutputKind::PromptEcho {
            let text = String::from_utf8_lossy(bytes).into_owned();
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
            let agent_id = entry.agent_id.clone().or_else(|| {
                provider_run
                    .as_ref()
                    .and_then(|run| run.agent_instance_id().map(str::to_string))
            });
            let active_prompt = agent_id.as_deref().and_then(|agent_id| {
                self.prompt_state_owner
                    .active_prompt_for_agent(&session, agent_id)
            });
            let prompt_id = active_prompt.as_ref().map(|prompt| prompt.id().to_string());
            let context = HistoryEventTurnContext {
                session_id: Some(entry.session_id.clone()),
                agent_id,
                provider: provider_run.as_ref().map(|run| run.provider().to_string()),
                model: provider_run.as_ref().map(|run| run.model().to_string()),
                turn_id: prompt_id.clone(),
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
                ..HistoryEventTurnContext::default()
            };
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
            self.history_projection.append(entry);
        }
    }
}

fn is_unread_output_history_entry(entry: &SessionHistoryEntry) -> bool {
    matches!(
        entry.kind,
        SessionHistoryEntryKind::ProviderOutput
            | SessionHistoryEntryKind::ProviderReasoning
            | SessionHistoryEntryKind::ProviderTool
            | SessionHistoryEntryKind::ProviderError
            | SessionHistoryEntryKind::Notice
    )
}
