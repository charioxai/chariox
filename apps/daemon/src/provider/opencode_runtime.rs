use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc::TryRecvError;
use std::time::{Duration, Instant};

use crate::error::DaemonError;
use crate::provider::opencode_client::OpenCodePart;
use crate::terminal::TerminalOutputKind;

use super::{OpenCodeClient, OpenCodeEvent, OpenCodeEventSubscription, OpenCodeMessage};

const OPENCODE_EVENT_RESUBSCRIBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const OPENCODE_EVENT_RESUBSCRIBE_RETRY_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(100);
const PROMPT_COMPLETION_SETTLE_WINDOW: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCodeOutputChunk {
    pub kind: TerminalOutputKind,
    pub merge_key: Option<String>,
    pub bytes: Vec<u8>,
}

pub(super) struct OpenCodeEventDrainResult {
    pub chunks: Vec<OpenCodeOutputChunk>,
    pub completions: Vec<OpenCodeAssistantCompletion>,
    pub prompt_completed: bool,
    pub notices: Vec<String>,
    pub resolved_model: Option<String>,
    pub resolved_model_source: Option<&'static str>,
    pub resolved_variant: Option<String>,
    pub resolved_usage_tokens_total: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCodeAssistantCompletion {
    pub message_id: String,
    pub completed_at_ms: u64,
}

#[derive(Debug)]
pub(super) struct OpenCodeRuntimeState {
    base_url: String,
    session_id: String,
    emitted_text_offsets: BTreeMap<String, usize>,
    emitted_tool_summaries: BTreeMap<String, String>,
    buffered_text_deltas: BTreeMap<String, Vec<String>>,
    message_roles: BTreeMap<String, String>,
    part_kinds: BTreeMap<String, String>,
    part_message_ids: BTreeMap<String, String>,
    event_subscription: OpenCodeEventSubscription,
    last_status_kind: Option<String>,
    last_completed_assistant_message_id: Option<String>,
    pending_prompt_completion: bool,
    pending_prompt_completion_quiet_since: Option<Instant>,
    active_tool_part_ids: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct ToolTranscriptUpdate {
    id: String,
    tool: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    raw: Option<String>,
}

impl OpenCodeRuntimeState {
    pub(super) fn new(
        base_url: String,
        session_id: String,
        event_subscription: OpenCodeEventSubscription,
    ) -> Self {
        Self {
            base_url,
            session_id,
            emitted_text_offsets: BTreeMap::new(),
            emitted_tool_summaries: BTreeMap::new(),
            buffered_text_deltas: BTreeMap::new(),
            message_roles: BTreeMap::new(),
            part_kinds: BTreeMap::new(),
            part_message_ids: BTreeMap::new(),
            event_subscription,
            last_status_kind: None,
            last_completed_assistant_message_id: None,
            pending_prompt_completion: false,
            pending_prompt_completion_quiet_since: None,
            active_tool_part_ids: BTreeSet::new(),
        }
    }

    pub(super) fn base_url(&self) -> &str {
        &self.base_url
    }

    pub(super) fn session_id(&self) -> &str {
        &self.session_id
    }

    pub(super) fn stop(self) {
        self.event_subscription.stop();
    }
}

pub(super) fn drain_opencode_events(
    state: &mut OpenCodeRuntimeState,
    provider_run_id: &str,
) -> Result<OpenCodeEventDrainResult, DaemonError> {
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut prompt_completed = false;
    let mut saw_completion_candidate = false;
    let mut notices = Vec::new();
    let mut resolved_model = None;
    let mut resolved_model_source = None;
    let mut resolved_variant = None;
    let mut resolved_usage_tokens_total = None;

    loop {
        match state.event_subscription.receiver.try_recv() {
            Ok(OpenCodeEvent::MessageUpdated { info }) => {
                if resolved_model.is_none() {
                    resolved_model = info.resolved_model();
                    if resolved_model.is_some() {
                        resolved_model_source = Some("message.updated");
                    }
                }
                if resolved_variant.is_none() {
                    resolved_variant = info.resolved_variant();
                }
                if info.role == "assistant" && info.session_id == state.session_id {
                    let total_tokens = info.total_tokens();
                    if total_tokens > 0 {
                        resolved_usage_tokens_total = Some(total_tokens);
                    }
                }
                state
                    .message_roles
                    .insert(info.id.clone(), info.role.clone());
                if info.session_id == state.session_id
                    && info.role == "assistant"
                    && info.time.completed.is_some()
                    && !info.is_tool_call_only_completion()
                    && state.last_completed_assistant_message_id.as_deref()
                        != Some(info.id.as_str())
                {
                    state.last_completed_assistant_message_id = Some(info.id.clone());
                    completions.push(OpenCodeAssistantCompletion {
                        message_id: info.id.clone(),
                        completed_at_ms: info.time.completed.unwrap_or_default(),
                    });
                    state.pending_prompt_completion = true;
                    state.pending_prompt_completion_quiet_since = None;
                    saw_completion_candidate = true;
                }
            }
            Ok(OpenCodeEvent::MessagePartDelta {
                session_id,
                message_id,
                part_id,
                field,
                delta,
                ..
            }) => {
                if session_id != state.session_id || field != "text" || delta.is_empty() {
                    continue;
                }
                state
                    .part_message_ids
                    .insert(part_id.clone(), message_id.clone());
                if !state.message_roles.contains_key(&message_id) {
                    refresh_opencode_message_metadata(state, provider_run_id)?;
                }
                let Some(role) = state.message_roles.get(&message_id).map(String::as_str) else {
                    state
                        .buffered_text_deltas
                        .entry(part_id)
                        .or_default()
                        .push(delta);
                    continue;
                };
                if role != "assistant" {
                    continue;
                }
                match state.part_kinds.get(&part_id).map(String::as_str) {
                    Some("reasoning") => {
                        let emitted = state
                            .emitted_text_offsets
                            .entry(part_id.clone())
                            .or_insert(0);
                        *emitted += delta.len();
                        chunks.push(OpenCodeOutputChunk {
                            kind: TerminalOutputKind::ProviderReasoning,
                            merge_key: Some(part_id.clone()),
                            bytes: delta.into_bytes(),
                        });
                    }
                    Some("text") => {
                        let emitted = state
                            .emitted_text_offsets
                            .entry(part_id.clone())
                            .or_insert(0);
                        *emitted += delta.len();
                        chunks.push(OpenCodeOutputChunk {
                            kind: TerminalOutputKind::ProviderOutput,
                            merge_key: Some(part_id.clone()),
                            bytes: delta.into_bytes(),
                        });
                    }
                    Some(_) => {}
                    None => {
                        state
                            .buffered_text_deltas
                            .entry(part_id)
                            .or_default()
                            .push(delta);
                    }
                }
            }
            Ok(OpenCodeEvent::MessagePartUpdated { part }) => {
                let part = *part;
                if part.session_id != state.session_id {
                    continue;
                }
                state
                    .part_message_ids
                    .insert(part.id.clone(), part.message_id.clone());
                state.part_kinds.insert(part.id.clone(), part.kind.clone());
                if !state.message_roles.contains_key(&part.message_id) {
                    refresh_opencode_message_metadata(state, provider_run_id)?;
                }
                let role = state
                    .message_roles
                    .get(&part.message_id)
                    .map(String::as_str);
                if let Some(buffered_deltas) = state.buffered_text_deltas.remove(&part.id) {
                    for delta in buffered_deltas {
                        if role != Some("assistant") {
                            continue;
                        }
                        let emitted = state
                            .emitted_text_offsets
                            .entry(part.id.clone())
                            .or_insert(0);
                        *emitted += delta.len();
                        match part.kind.as_str() {
                            "reasoning" => chunks.push(OpenCodeOutputChunk {
                                kind: TerminalOutputKind::ProviderReasoning,
                                merge_key: Some(part.id.clone()),
                                bytes: delta.into_bytes(),
                            }),
                            "text" => chunks.push(OpenCodeOutputChunk {
                                kind: TerminalOutputKind::ProviderOutput,
                                merge_key: Some(part.id.clone()),
                                bytes: delta.into_bytes(),
                            }),
                            _ => {}
                        }
                    }
                }
                match part.kind.as_str() {
                    "text" => {
                        if role != Some("assistant") || part.text.is_empty() {
                            continue;
                        }
                        let emitted = state
                            .emitted_text_offsets
                            .entry(part.id.clone())
                            .or_insert(0);
                        let start = (*emitted).min(part.text.len());
                        if start == part.text.len() {
                            continue;
                        }
                        chunks.push(OpenCodeOutputChunk {
                            kind: TerminalOutputKind::ProviderOutput,
                            merge_key: Some(part.id.clone()),
                            bytes: part.text.as_bytes()[start..].to_vec(),
                        });
                        *emitted = part.text.len();
                    }
                    "reasoning" => {
                        if role != Some("assistant") || part.text.is_empty() {
                            continue;
                        }
                        let emitted = state
                            .emitted_text_offsets
                            .entry(part.id.clone())
                            .or_insert(0);
                        let start = (*emitted).min(part.text.len());
                        if start == part.text.len() {
                            continue;
                        }
                        chunks.push(OpenCodeOutputChunk {
                            kind: TerminalOutputKind::ProviderReasoning,
                            merge_key: Some(part.id.clone()),
                            bytes: part.text.as_bytes()[start..].to_vec(),
                        });
                        *emitted = part.text.len();
                    }
                    "tool" => {
                        if role != Some("assistant") {
                            continue;
                        }
                        update_active_tool_part_ids(state, &part);
                        let summary = render_tool_transcript_update(&part);
                        let previous = state.emitted_tool_summaries.get(&part.id);
                        if previous.map(String::as_str) != Some(summary.as_str()) {
                            state
                                .emitted_tool_summaries
                                .insert(part.id.clone(), summary.clone());
                            chunks.push(OpenCodeOutputChunk {
                                kind: TerminalOutputKind::ProviderTool,
                                merge_key: Some(part.id.clone()),
                                bytes: summary.into_bytes(),
                            });
                        }
                    }
                    _ => {}
                }
            }
            Ok(OpenCodeEvent::SessionError {
                session_id,
                message,
            }) => {
                if session_id == state.session_id {
                    chunks.push(OpenCodeOutputChunk {
                        kind: TerminalOutputKind::ProviderError,
                        merge_key: None,
                        bytes: render_session_error_transcript_update(&message).into_bytes(),
                    });
                    notices.push(message);
                    prompt_completed = true;
                }
            }
            Ok(OpenCodeEvent::SessionStatus { session_id, kind }) => {
                if session_id == state.session_id {
                    if state.last_status_kind.as_deref() != Some(kind.as_str()) {
                        state.last_status_kind = Some(kind.clone());
                        chunks.push(OpenCodeOutputChunk {
                            kind: TerminalOutputKind::ProviderStatus,
                            merge_key: Some("__provider_status__".to_string()),
                            bytes: format_session_status(&kind).into_bytes(),
                        });
                    }
                }
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                let client = OpenCodeClient::new(provider_run_id, &state.base_url)?;
                state.event_subscription = client.subscribe_events_with_retry(
                    OPENCODE_EVENT_RESUBSCRIBE_TIMEOUT,
                    OPENCODE_EVENT_RESUBSCRIBE_RETRY_INTERVAL,
                )?;
                if let Ok(snapshot) = client.snapshot(&state.session_id) {
                    if resolved_model.is_none() {
                        resolved_model = snapshot
                            .messages
                            .iter()
                            .rev()
                            .find_map(|message| message.info.resolved_model());
                        if resolved_model.is_some() {
                            resolved_model_source = Some("snapshot");
                        }
                    }
                    if resolved_variant.is_none() {
                        resolved_variant = snapshot
                            .messages
                            .iter()
                            .rev()
                            .find_map(|message| message.info.resolved_variant());
                    }
                    if let Some(total_tokens) = latest_assistant_usage_tokens(&snapshot.messages) {
                        resolved_usage_tokens_total = Some(total_tokens);
                    }
                    record_snapshot_message_metadata(state, &snapshot.messages);
                    let snapshot_chunks = render_snapshot_output_chunks(state, &snapshot.messages);
                    chunks.extend(snapshot_chunks.chunks);
                    if state.last_status_kind.as_deref() != Some(snapshot.status.as_str()) {
                        state.last_status_kind = Some(snapshot.status.clone());
                        chunks.push(OpenCodeOutputChunk {
                            kind: TerminalOutputKind::ProviderStatus,
                            merge_key: Some("__provider_status__".to_string()),
                            bytes: format_session_status(&snapshot.status).into_bytes(),
                        });
                    }
                    if snapshot.messages.iter().any(|message| {
                        let is_new_completed = message.info.session_id == state.session_id
                            && message.info.role == "assistant"
                            && message.info.time.completed.is_some()
                            && !message.info.is_tool_call_only_completion()
                            && state.last_completed_assistant_message_id.as_deref()
                                != Some(message.info.id.as_str());
                        if is_new_completed {
                            state.last_completed_assistant_message_id =
                                Some(message.info.id.clone());
                            completions.push(OpenCodeAssistantCompletion {
                                message_id: message.info.id.clone(),
                                completed_at_ms: message.info.time.completed.unwrap_or_default(),
                            });
                        }
                        is_new_completed
                    }) {
                        state.pending_prompt_completion = true;
                        state.pending_prompt_completion_quiet_since = None;
                        saw_completion_candidate = true;
                    }
                }
            }
        }
    }

    if state.pending_prompt_completion {
        if saw_completion_candidate || !chunks.is_empty() || !state.active_tool_part_ids.is_empty()
        {
            state.pending_prompt_completion_quiet_since = None;
        } else if let Some(quiet_since) = state.pending_prompt_completion_quiet_since {
            if quiet_since.elapsed() >= PROMPT_COMPLETION_SETTLE_WINDOW {
                prompt_completed = true;
                state.pending_prompt_completion = false;
                state.pending_prompt_completion_quiet_since = None;
            }
        } else {
            state.pending_prompt_completion_quiet_since = Some(Instant::now());
        }
    }

    Ok(OpenCodeEventDrainResult {
        chunks,
        completions,
        prompt_completed,
        notices,
        resolved_model,
        resolved_model_source,
        resolved_variant,
        resolved_usage_tokens_total,
    })
}

struct SnapshotRenderResult {
    chunks: Vec<OpenCodeOutputChunk>,
}

fn refresh_opencode_message_metadata(
    state: &mut OpenCodeRuntimeState,
    provider_run_id: &str,
) -> Result<(), DaemonError> {
    let client = OpenCodeClient::new(provider_run_id, &state.base_url)?;
    if let Ok(messages) = client.messages(&state.session_id) {
        record_snapshot_message_metadata(state, &messages);
    }
    Ok(())
}

fn record_snapshot_message_metadata(
    state: &mut OpenCodeRuntimeState,
    messages: &[OpenCodeMessage],
) {
    for message in messages {
        state
            .message_roles
            .insert(message.info.id.clone(), message.info.role.clone());
        for part in &message.parts {
            state
                .part_message_ids
                .insert(part.id.clone(), part.message_id.clone());
            state.part_kinds.insert(part.id.clone(), part.kind.clone());
        }
    }
}

fn latest_assistant_usage_tokens(messages: &[OpenCodeMessage]) -> Option<u64> {
    messages.iter().rev().find_map(|message| {
        (message.info.role == "assistant")
            .then(|| message.info.total_tokens())
            .filter(|total| *total > 0)
    })
}

fn render_snapshot_output_chunks(
    state: &mut OpenCodeRuntimeState,
    messages: &[OpenCodeMessage],
) -> SnapshotRenderResult {
    let mut chunks = Vec::new();
    for message in messages
        .iter()
        .filter(|message| message.info.role == "assistant")
    {
        for part in &message.parts {
            match part.kind.as_str() {
                "text" | "reasoning" => {
                    if part.text.is_empty() {
                        continue;
                    }
                    let emitted = state
                        .emitted_text_offsets
                        .entry(part.id.clone())
                        .or_insert(0);
                    let start = (*emitted).min(part.text.len());
                    if start == part.text.len() {
                        continue;
                    }
                    chunks.push(OpenCodeOutputChunk {
                        kind: if part.kind == "reasoning" {
                            TerminalOutputKind::ProviderReasoning
                        } else {
                            TerminalOutputKind::ProviderOutput
                        },
                        merge_key: Some(part.id.clone()),
                        bytes: part.text.as_bytes()[start..].to_vec(),
                    });
                    *emitted = part.text.len();
                }
                "tool" => {
                    update_active_tool_part_ids(state, part);
                    let summary = render_tool_transcript_update(part);
                    let previous = state.emitted_tool_summaries.get(&part.id);
                    if previous.map(String::as_str) != Some(summary.as_str()) {
                        state
                            .emitted_tool_summaries
                            .insert(part.id.clone(), summary.clone());
                        chunks.push(OpenCodeOutputChunk {
                            kind: TerminalOutputKind::ProviderTool,
                            merge_key: Some(part.id.clone()),
                            bytes: summary.into_bytes(),
                        });
                    }
                }
                _ => {}
            }
        }
    }
    SnapshotRenderResult { chunks }
}

fn update_active_tool_part_ids(state: &mut OpenCodeRuntimeState, part: &OpenCodePart) {
    let Some(tool_state) = part.state.as_ref() else {
        return;
    };
    let status = tool_state.status.trim().to_ascii_lowercase();
    if status.is_empty() || is_terminal_tool_status(&status) {
        state.active_tool_part_ids.remove(&part.id);
    } else {
        state.active_tool_part_ids.insert(part.id.clone());
    }
}

fn is_terminal_tool_status(status: &str) -> bool {
    matches!(status, "completed" | "error" | "cancelled")
}

fn render_tool_transcript_update(part: &OpenCodePart) -> String {
    let tool_name = if part.tool.is_empty() {
        "tool"
    } else {
        part.tool.as_str()
    };
    let status = part
        .state
        .as_ref()
        .map(|state| state.status.as_str())
        .filter(|status: &&str| !status.is_empty())
        .unwrap_or("updated");
    let rendered_text = (!part.text.trim().is_empty()).then(|| part.text.trim().to_string());
    let input = part.state.as_ref().and_then(|state| {
        (!state.input.is_null() && !is_empty_json_value(&state.input)).then(|| state.input.clone())
    });
    let output = part
        .state
        .as_ref()
        .and_then(|state| non_empty(state.output.as_str()).map(str::to_string))
        .or_else(|| tool_metadata_field(part, &["output", "stdout"]));
    let description = tool_metadata_field(part, &["description"]);
    let title = part
        .state
        .as_ref()
        .and_then(|state| non_empty(state.title.as_str()).map(str::to_string));
    let error = part
        .state
        .as_ref()
        .and_then(|state| non_empty(state.error.as_str()).map(str::to_string));
    let raw = part
        .state
        .as_ref()
        .and_then(|state| non_empty(state.raw.as_str()))
        .map(render_tool_raw_detail)
        .filter(|value| {
            rendered_text.as_deref() != Some(value.as_str())
                && output.as_deref() != Some(value.as_str())
        });

    serde_json::to_string(&ToolTranscriptUpdate {
        id: part.id.clone(),
        tool: tool_name.to_string(),
        status: status.to_string(),
        title,
        description,
        text: rendered_text,
        input,
        output,
        error,
        raw,
    })
    .unwrap_or_else(|_| {
        format!(
            "{{\"id\":{id:?},\"tool\":{tool:?},\"status\":{status:?}}}",
            id = part.id,
            tool = tool_name,
            status = status,
        )
    })
}

fn render_session_error_transcript_update(message: &str) -> String {
    let message = non_empty(message).unwrap_or("OpenCode reported an unknown session error.");
    format!("**OpenCode error**\n\n{message}")
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn is_empty_json_value(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => true,
        serde_json::Value::Array(items) => items.is_empty(),
        serde_json::Value::Object(items) => items.is_empty(),
        _ => false,
    }
}

fn tool_metadata_field(part: &OpenCodePart, keys: &[&str]) -> Option<String> {
    let metadata = part.state.as_ref()?.metadata.as_object()?;
    keys.iter().find_map(|key| {
        metadata
            .get(*key)
            .and_then(serde_json::Value::as_str)
            .and_then(non_empty)
            .map(str::to_string)
    })
}

fn render_tool_raw_detail(raw: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| raw.to_string()),
        Err(_) => raw.to_string(),
    }
}

fn format_session_status(kind: &str) -> String {
    match kind {
        "busy" => "OpenCode is thinking...".to_string(),
        "idle" => "OpenCode is idle.".to_string(),
        other => format!("OpenCode status: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Instant;

    use serde_json::json;

    use crate::provider::opencode_client::{OpenCodeMessage, OpenCodePart, OpenCodeToolState};
    use crate::terminal::TerminalOutputKind;

    use super::{
        drain_opencode_events, latest_assistant_usage_tokens, render_snapshot_output_chunks,
        render_tool_transcript_update, OpenCodeAssistantCompletion, OpenCodeRuntimeState,
        ToolTranscriptUpdate, PROMPT_COMPLETION_SETTLE_WINDOW,
    };

    #[test]
    fn renders_structured_tool_update_with_input_and_output() {
        let payload = render_tool_transcript_update(&OpenCodePart {
            id: "part-1".to_string(),
            session_id: "session-1".to_string(),
            message_id: "message-1".to_string(),
            kind: "tool".to_string(),
            text: String::new(),
            tool: "bash".to_string(),
            state: Some(OpenCodeToolState {
                status: "completed".to_string(),
                input: json!({ "command": "git status" }),
                output: String::new(),
                title: String::new(),
                metadata: json!({
                    "output": "On branch main",
                    "description": "Shows working tree status"
                }),
                error: String::new(),
                raw: String::new(),
            }),
            time: None,
        });

        let parsed: ToolTranscriptUpdate =
            serde_json::from_str(&payload).expect("tool payload should deserialize");
        assert_eq!(parsed.id, "part-1");
        assert_eq!(parsed.tool, "bash");
        assert_eq!(parsed.status, "completed");
        assert_eq!(
            parsed.description.as_deref(),
            Some("Shows working tree status")
        );
        assert_eq!(parsed.output.as_deref(), Some("On branch main"));
        assert_eq!(parsed.input, Some(json!({ "command": "git status" })));
    }

    #[test]
    fn snapshot_rendering_preserves_reasoning_and_text_order() {
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(
                std::sync::mpsc::channel().1,
            ),
        );
        let chunks = render_snapshot_output_chunks(
            &mut state,
            &[crate::provider::OpenCodeMessage {
                info: serde_json::from_value(json!({
                    "id": "message-1",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "time": { "completed": 1 }
                }))
                .expect("message info should deserialize"),
                parts: vec![
                    OpenCodePart {
                        id: "part-1".to_string(),
                        session_id: "session-1".to_string(),
                        message_id: "message-1".to_string(),
                        kind: "reasoning".to_string(),
                        text: "first thought\n".to_string(),
                        tool: String::new(),
                        state: None,
                        time: None,
                    },
                    OpenCodePart {
                        id: "part-2".to_string(),
                        session_id: "session-1".to_string(),
                        message_id: "message-1".to_string(),
                        kind: "text".to_string(),
                        text: "first answer\n".to_string(),
                        tool: String::new(),
                        state: None,
                        time: None,
                    },
                    OpenCodePart {
                        id: "part-3".to_string(),
                        session_id: "session-1".to_string(),
                        message_id: "message-1".to_string(),
                        kind: "reasoning".to_string(),
                        text: "second thought\n".to_string(),
                        tool: String::new(),
                        state: None,
                        time: None,
                    },
                ],
            }],
        )
        .chunks;

        assert_eq!(
            chunks
                .iter()
                .map(|chunk| (
                    chunk.kind.clone(),
                    String::from_utf8_lossy(&chunk.bytes).into_owned()
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    TerminalOutputKind::ProviderReasoning,
                    "first thought\n".to_string()
                ),
                (
                    TerminalOutputKind::ProviderOutput,
                    "first answer\n".to_string()
                ),
                (
                    TerminalOutputKind::ProviderReasoning,
                    "second thought\n".to_string()
                ),
            ]
        );
    }

    #[test]
    fn latest_assistant_usage_tokens_uses_the_newest_assistant_with_tokens() {
        let messages = vec![
            serde_json::from_value::<OpenCodeMessage>(json!({
                "info": {
                    "id": "message-1",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "tokens": {
                        "input": 100,
                        "output": 20,
                        "reasoning": 5,
                        "cache": { "read": 10, "write": 5 }
                    },
                    "time": { "completed": 1 }
                },
                "parts": []
            }))
            .expect("message should deserialize"),
            serde_json::from_value::<OpenCodeMessage>(json!({
                "info": {
                    "id": "message-2",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "tokens": {
                        "input": 200,
                        "output": 40,
                        "reasoning": 10,
                        "cache": { "read": 20, "write": 10 }
                    },
                    "time": { "completed": 2 }
                },
                "parts": []
            }))
            .expect("message should deserialize"),
        ];

        assert_eq!(latest_assistant_usage_tokens(&messages), Some(280));
    }

    #[test]
    fn completed_assistant_requires_a_quiet_drain_before_completing_prompt() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                info: serde_json::from_value(json!({
                    "id": "message-1",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "time": { "completed": 1 }
                }))
                .expect("message info should deserialize"),
            },
        )
        .expect("message update should send");

        let first = drain_opencode_events(&mut state, "provider-run-1")
            .expect("first drain should succeed");
        assert!(!first.prompt_completed);
        assert_eq!(
            first.completions,
            vec![OpenCodeAssistantCompletion {
                message_id: "message-1".to_string(),
                completed_at_ms: 1,
            }]
        );

        state.pending_prompt_completion_quiet_since = Some(elapsed_quiet_since());
        let second = drain_opencode_events(&mut state, "provider-run-1")
            .expect("second drain should succeed");
        assert!(second.prompt_completed);
    }

    #[test]
    fn idle_status_is_not_treated_as_prompt_completion() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::SessionStatus {
                session_id: "session-1".to_string(),
                kind: "idle".to_string(),
            },
        )
        .expect("idle status should send");

        let result =
            drain_opencode_events(&mut state, "provider-run-1").expect("drain should succeed");

        assert!(!result.prompt_completed);
        assert!(result.completions.is_empty());
        assert!(result
            .chunks
            .iter()
            .any(|chunk| chunk.kind == TerminalOutputKind::ProviderStatus));
    }

    #[test]
    fn running_tools_block_prompt_completion_until_they_finish_and_the_stream_goes_quiet() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                info: serde_json::from_value(json!({
                    "id": "message-1",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "time": { "completed": 1 }
                }))
                .expect("message info should deserialize"),
            },
        )
        .expect("message update should send");
        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessagePartUpdated {
                part: Box::new(OpenCodePart {
                    id: "tool-1".to_string(),
                    session_id: "session-1".to_string(),
                    message_id: "message-1".to_string(),
                    kind: "tool".to_string(),
                    text: String::new(),
                    tool: "bash".to_string(),
                    state: Some(OpenCodeToolState {
                        status: "running".to_string(),
                        input: json!({ "command": "git status" }),
                        output: String::new(),
                        title: String::new(),
                        metadata: json!({}),
                        error: String::new(),
                        raw: String::new(),
                    }),
                    time: None,
                }),
            },
        )
        .expect("running tool update should send");

        let first = drain_opencode_events(&mut state, "provider-run-1")
            .expect("first drain should succeed");
        assert!(!first.prompt_completed);
        assert!(first
            .chunks
            .iter()
            .any(|chunk| chunk.kind == TerminalOutputKind::ProviderTool));

        let second = drain_opencode_events(&mut state, "provider-run-1")
            .expect("second drain should succeed");
        assert!(!second.prompt_completed);

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessagePartUpdated {
                part: Box::new(OpenCodePart {
                    id: "tool-1".to_string(),
                    session_id: "session-1".to_string(),
                    message_id: "message-1".to_string(),
                    kind: "tool".to_string(),
                    text: String::new(),
                    tool: "bash".to_string(),
                    state: Some(OpenCodeToolState {
                        status: "completed".to_string(),
                        input: json!({ "command": "git status" }),
                        output: "On branch main".to_string(),
                        title: String::new(),
                        metadata: json!({}),
                        error: String::new(),
                        raw: String::new(),
                    }),
                    time: None,
                }),
            },
        )
        .expect("completed tool update should send");

        let third = drain_opencode_events(&mut state, "provider-run-1")
            .expect("third drain should succeed");
        assert!(!third.prompt_completed);

        state.pending_prompt_completion_quiet_since = Some(elapsed_quiet_since());
        let fourth = drain_opencode_events(&mut state, "provider-run-1")
            .expect("fourth drain should succeed");
        assert!(fourth.prompt_completed);
    }

    #[test]
    fn resumed_output_resets_the_quiet_settle_window_before_prompt_completion() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                info: serde_json::from_value(json!({
                    "id": "message-1",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "time": { "completed": 1 }
                }))
                .expect("message info should deserialize"),
            },
        )
        .expect("message update should send");

        let first = drain_opencode_events(&mut state, "provider-run-1")
            .expect("first drain should succeed");
        assert!(!first.prompt_completed);

        let second = drain_opencode_events(&mut state, "provider-run-1")
            .expect("second drain should succeed");
        assert!(!second.prompt_completed);
        assert!(state.pending_prompt_completion_quiet_since.is_some());

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessagePartUpdated {
                part: Box::new(OpenCodePart {
                    id: "part-1".to_string(),
                    session_id: "session-1".to_string(),
                    message_id: "message-1".to_string(),
                    kind: "text".to_string(),
                    text: "late output".to_string(),
                    tool: String::new(),
                    state: None,
                    time: None,
                }),
            },
        )
        .expect("late text update should send");

        let third = drain_opencode_events(&mut state, "provider-run-1")
            .expect("third drain should succeed");
        assert!(!third.prompt_completed);
        assert_eq!(third.chunks.len(), 1);
        assert_eq!(state.pending_prompt_completion_quiet_since, None);

        let fourth = drain_opencode_events(&mut state, "provider-run-1")
            .expect("fourth drain should succeed");
        assert!(!fourth.prompt_completed);
        assert!(state.pending_prompt_completion_quiet_since.is_some());

        state.pending_prompt_completion_quiet_since = Some(elapsed_quiet_since());
        let fifth = drain_opencode_events(&mut state, "provider-run-1")
            .expect("fifth drain should succeed");
        assert!(fifth.prompt_completed);
    }

    #[test]
    fn tool_call_only_message_completion_does_not_complete_prompt() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                info: serde_json::from_value(json!({
                    "id": "message-tool-calls",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "finish": "tool-calls",
                    "time": { "completed": 1 }
                }))
                .expect("message info should deserialize"),
            },
        )
        .expect("message update should send");

        let first = drain_opencode_events(&mut state, "provider-run-1")
            .expect("first drain should succeed");
        assert!(first.completions.is_empty());
        assert!(!first.prompt_completed);

        state.pending_prompt_completion_quiet_since = Some(elapsed_quiet_since());
        let second = drain_opencode_events(&mut state, "provider-run-1")
            .expect("second drain should succeed");
        assert!(!second.prompt_completed);
    }

    fn elapsed_quiet_since() -> Instant {
        Instant::now()
            .checked_sub(PROMPT_COMPLETION_SETTLE_WINDOW)
            .unwrap_or_else(Instant::now)
    }
}
