use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

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
use serde_json::Value;

const MAX_PROVIDER_OUTPUT_RECORD_BYTES: usize = 256 * 1024;
const MAX_PROVIDER_TOOL_STRING_BYTES: usize = 64 * 1024;
const PROVIDER_TOOL_HISTORY_GATE_LIMIT: usize = 4096;

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
        let bounded_bytes = bounded_terminal_output_bytes(&kind, bytes);
        if bounded_bytes.len() < bytes.len() {
            crate::logging::warn_with_fields(
                "daemon.provider_output",
                "truncated oversized provider terminal output",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "kind": format!("{:?}", kind),
                    "original_bytes": bytes.len(),
                    "bounded_bytes": bounded_bytes.len(),
                    "omitted_bytes": bytes.len().saturating_sub(bounded_bytes.len()),
                }),
            );
        }
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
            &bounded_bytes,
        );
        self.notify_metaagent_trace_activity(session_id, agent_id);
        if kind != TerminalOutputKind::PromptEcho {
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

fn bounded_terminal_output_bytes(kind: &TerminalOutputKind, bytes: &[u8]) -> Vec<u8> {
    if bytes.len() <= MAX_PROVIDER_OUTPUT_RECORD_BYTES {
        return bytes.to_vec();
    }
    if matches!(kind, TerminalOutputKind::ProviderTool) {
        if let Some(value) = bounded_provider_tool_json(bytes) {
            return value;
        }
    }
    bounded_text_bytes(bytes)
}

fn bounded_provider_tool_json(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut value = serde_json::from_slice::<Value>(bytes).ok()?;
    let mut truncated = false;
    truncate_json_strings(&mut value, &mut truncated);
    if truncated {
        if let Value::Object(object) = &mut value {
            object.insert("arroba_truncated".to_string(), Value::Bool(true));
            object.insert(
                "arroba_original_bytes".to_string(),
                Value::Number((bytes.len() as u64).into()),
            );
        }
    }
    let encoded = serde_json::to_vec(&value).ok()?;
    if encoded.len() <= MAX_PROVIDER_OUTPUT_RECORD_BYTES {
        return Some(encoded);
    }
    let mut fallback = serde_json::Map::new();
    if let Value::Object(object) = &value {
        for key in ["id", "tool", "status", "name", "command"] {
            if let Some(value) = object.get(key).cloned() {
                fallback.insert(key.to_string(), value);
            }
        }
    }
    fallback.insert("arroba_truncated".to_string(), Value::Bool(true));
    fallback.insert(
        "arroba_original_bytes".to_string(),
        Value::Number((bytes.len() as u64).into()),
    );
    fallback.insert(
        "message".to_string(),
        Value::String(
            "provider tool payload omitted because it exceeded the terminal event size limit"
                .to_string(),
        ),
    );
    serde_json::to_vec(&Value::Object(fallback)).ok()
}

fn truncate_json_strings(value: &mut Value, truncated: &mut bool) {
    match value {
        Value::String(text) => {
            if text.len() > MAX_PROVIDER_TOOL_STRING_BYTES {
                let original_len = text.len();
                *text = bounded_text_string(
                    text.as_bytes(),
                    MAX_PROVIDER_TOOL_STRING_BYTES,
                    original_len,
                );
                *truncated = true;
            }
        }
        Value::Array(items) => {
            for item in items {
                truncate_json_strings(item, truncated);
            }
        }
        Value::Object(object) => {
            for value in object.values_mut() {
                truncate_json_strings(value, truncated);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn bounded_text_bytes(bytes: &[u8]) -> Vec<u8> {
    bounded_text_string(bytes, MAX_PROVIDER_OUTPUT_RECORD_BYTES, bytes.len()).into_bytes()
}

fn bounded_text_string(bytes: &[u8], limit: usize, original_len: usize) -> String {
    let marker = format!(
        "\n\n[arroba: output truncated, omitted {} bytes]\n",
        original_len.saturating_sub(limit),
    );
    let marker_bytes = marker.as_bytes();
    let keep_limit = limit.saturating_sub(marker_bytes.len());
    let mut keep = keep_limit.min(bytes.len());
    while keep > 0 && std::str::from_utf8(&bytes[..keep]).is_err() {
        keep -= 1;
    }
    let mut text = String::from_utf8_lossy(&bytes[..keep]).into_owned();
    text.push_str(&marker);
    text
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderToolHistoryState {
    status: String,
}

fn should_persist_provider_tool_history(entry: &SessionHistoryEntry) -> bool {
    if entry.kind != SessionHistoryEntryKind::ProviderTool {
        return true;
    }
    let Some(key) = provider_tool_history_key(entry) else {
        return true;
    };
    let Some(state) = provider_tool_history_state(entry) else {
        return true;
    };
    let mut gate = provider_tool_history_gate()
        .lock()
        .expect("provider tool history gate lock should not be poisoned");
    let should_persist = gate.get(&key) != Some(&state);
    if should_persist {
        gate.insert(key, state);
        while gate.len() > PROVIDER_TOOL_HISTORY_GATE_LIMIT {
            let Some(oldest_key) = gate.keys().next().cloned() else {
                break;
            };
            gate.remove(&oldest_key);
        }
    }
    should_persist
}

fn provider_tool_history_key(entry: &SessionHistoryEntry) -> Option<String> {
    let provider_run_id = entry.provider_run_id.as_deref()?;
    let tool_id = entry
        .merge_key
        .clone()
        .or_else(|| provider_tool_json_string_field(&entry.text, "id"))
        .or_else(|| provider_tool_json_string_field(&entry.text, "call_id"))?;
    Some(format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}",
        entry.session_id,
        provider_run_id,
        entry.agent_id.as_deref().unwrap_or(""),
        tool_id
    ))
}

fn provider_tool_history_state(entry: &SessionHistoryEntry) -> Option<ProviderToolHistoryState> {
    provider_tool_json_string_field(&entry.text, "status")
        .map(|status| ProviderToolHistoryState { status })
}

fn provider_tool_json_string_field(text: &str, field: &str) -> Option<String> {
    serde_json::from_str::<Value>(text)
        .ok()?
        .get(field)?
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

fn provider_tool_history_gate() -> &'static Mutex<BTreeMap<String, ProviderToolHistoryState>> {
    static GATE: OnceLock<Mutex<BTreeMap<String, ProviderToolHistoryState>>> = OnceLock::new();
    GATE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

#[cfg(test)]
fn reset_provider_tool_history_gate_for_tests() {
    provider_tool_history_gate()
        .lock()
        .expect("provider tool history gate lock should not be poisoned")
        .clear();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_terminal_output_truncates_plain_output_with_marker() {
        let bytes = vec![b'x'; MAX_PROVIDER_OUTPUT_RECORD_BYTES + 1024];

        let bounded = bounded_terminal_output_bytes(&TerminalOutputKind::ProviderOutput, &bytes);

        assert!(bounded.len() <= MAX_PROVIDER_OUTPUT_RECORD_BYTES);
        let text = String::from_utf8(bounded).expect("bounded output should remain utf8");
        assert!(text.contains("[arroba: output truncated"));
    }

    #[test]
    fn bounded_terminal_output_preserves_provider_tool_json() {
        let payload = serde_json::json!({
            "id": "call-1",
            "tool": "shell",
            "status": "running",
            "output": "x".repeat(MAX_PROVIDER_OUTPUT_RECORD_BYTES + 1024),
        });
        let bytes = serde_json::to_vec(&payload).expect("payload should encode");

        let bounded = bounded_terminal_output_bytes(&TerminalOutputKind::ProviderTool, &bytes);

        assert!(bounded.len() <= MAX_PROVIDER_OUTPUT_RECORD_BYTES);
        let value: Value =
            serde_json::from_slice(&bounded).expect("bounded tool should remain json");
        assert_eq!(value.get("id").and_then(Value::as_str), Some("call-1"));
        assert_eq!(
            value.get("arroba_truncated").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn provider_tool_history_persists_first_running_and_status_transition() {
        reset_provider_tool_history_gate_for_tests();
        let running = provider_tool_entry("running");
        let completed = provider_tool_entry("completed");

        assert!(should_persist_provider_tool_history(&running));
        assert!(!should_persist_provider_tool_history(&running));
        assert!(should_persist_provider_tool_history(&completed));
        assert!(!should_persist_provider_tool_history(&completed));
    }

    #[test]
    fn provider_tool_history_requires_tool_identity_and_status() {
        reset_provider_tool_history_gate_for_tests();
        let no_status = SessionHistoryEntry::provider_output(
            "session-1",
            "provider-run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderTool,
            Some("call-1".to_string()),
            serde_json::json!({ "id": "call-1", "tool": "shell" }).to_string(),
        );
        let no_identity = SessionHistoryEntry::provider_output(
            "session-1",
            "provider-run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderTool,
            None,
            serde_json::json!({ "status": "running", "tool": "shell" }).to_string(),
        );

        assert!(should_persist_provider_tool_history(&no_status));
        assert!(should_persist_provider_tool_history(&no_status));
        assert!(should_persist_provider_tool_history(&no_identity));
        assert!(should_persist_provider_tool_history(&no_identity));
    }

    fn provider_tool_entry(status: &str) -> SessionHistoryEntry {
        SessionHistoryEntry::provider_output(
            "session-1",
            "provider-run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderTool,
            Some("call-1".to_string()),
            serde_json::json!({
                "id": "call-1",
                "tool": "shell",
                "status": status,
                "output": "snapshot",
            })
            .to_string(),
        )
    }
}
