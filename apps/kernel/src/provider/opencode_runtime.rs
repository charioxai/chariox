use crate::error::DaemonError;
use crate::provider::run_actor::ProviderNativeInteractionBridge;
use crate::terminal::TerminalOutputKind;
use std::collections::BTreeMap;
use std::sync::mpsc::TryRecvError;

use super::{OpenCodeClient, OpenCodeEvent, OpenCodeEventSubscription};

mod permission;
mod snapshot;
mod transcript;

use permission::handle_permission_request;
use snapshot::{
    collect_new_completed_assistant_messages, latest_assistant_usage_tokens,
    opencode_message_completes_active_prompt, opencode_messages_complete_active_prompt,
    record_snapshot_message_metadata, refresh_opencode_message_metadata,
    render_snapshot_output_chunks,
};
use transcript::{
    format_session_status, render_session_error_transcript_update, render_tool_transcript_update,
};

#[cfg(test)]
use transcript::ToolTranscriptUpdate;

const OPENCODE_EVENT_RESUBSCRIBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const OPENCODE_EVENT_RESUBSCRIBE_RETRY_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(100);
const OPENCODE_EVENT_DRAIN_MAX_EVENTS: usize = 256;
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
    pub terminal_failure: Option<String>,
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
pub(crate) struct OpenCodeRuntimeState {
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
    active_user_message_id: Option<String>,
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
            active_user_message_id: None,
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

    pub(super) fn note_prompt_submitted(&mut self, user_message_id: String) {
        self.active_user_message_id = Some(user_message_id);
    }
}

pub(super) fn drain_opencode_events(
    run: &crate::provider::RuntimeProviderRun,
    state: &mut OpenCodeRuntimeState,
    native_interaction_bridge: Option<std::sync::Arc<dyn ProviderNativeInteractionBridge>>,
) -> Result<OpenCodeEventDrainResult, DaemonError> {
    let provider_run_id = run.id();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut notices = Vec::new();
    let mut resolved_model = None;
    let mut resolved_model_source = None;
    let mut resolved_variant = None;
    let mut resolved_usage_tokens_total = None;

    for _ in 0..OPENCODE_EVENT_DRAIN_MAX_EVENTS {
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
                }
                if opencode_message_completes_active_prompt(state, &info) {
                    let client = OpenCodeClient::new(provider_run_id, &state.base_url)?;
                    if let Ok(messages) = client.messages(&state.session_id) {
                        if let Some(total_tokens) = latest_assistant_usage_tokens(&messages) {
                            resolved_usage_tokens_total = Some(total_tokens);
                        }
                        record_snapshot_message_metadata(state, &messages);
                        chunks.extend(render_snapshot_output_chunks(state, &messages).chunks);
                        let snapshot_completions =
                            collect_new_completed_assistant_messages(state, &messages);
                        completions.extend(snapshot_completions);
                    }
                    prompt_completed = true;
                    state.active_user_message_id = None;
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
                            "text" => {
                                chunks.push(OpenCodeOutputChunk {
                                    kind: TerminalOutputKind::ProviderOutput,
                                    merge_key: Some(part.id.clone()),
                                    bytes: delta.into_bytes(),
                                });
                            }
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
                    terminal_failure = Some(message.clone());
                    notices.push(message);
                    prompt_completed = true;
                    state.active_user_message_id = None;
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
                    if state.active_user_message_id.is_some() {
                        let client = OpenCodeClient::new(provider_run_id, &state.base_url)?;
                        if let Ok(messages) = client.messages(&state.session_id) {
                            if let Some(total_tokens) = latest_assistant_usage_tokens(&messages) {
                                resolved_usage_tokens_total = Some(total_tokens);
                            }
                            record_snapshot_message_metadata(state, &messages);
                            let snapshot_chunks = render_snapshot_output_chunks(state, &messages);
                            chunks.extend(snapshot_chunks.chunks);
                            let status_completions =
                                collect_new_completed_assistant_messages(state, &messages);
                            if !status_completions.is_empty() {
                                completions.extend(status_completions);
                            }
                            if opencode_messages_complete_active_prompt(state, &messages) {
                                prompt_completed = true;
                                state.active_user_message_id = None;
                            }
                        }
                    }
                }
            }
            Ok(OpenCodeEvent::PermissionAsked { request }) => {
                if request.session_id != state.session_id {
                    continue;
                }
                handle_permission_request(
                    run,
                    state,
                    provider_run_id,
                    native_interaction_bridge.clone(),
                    &request,
                )?;
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
                    let snapshot_completions =
                        collect_new_completed_assistant_messages(state, &snapshot.messages);
                    if !snapshot_completions.is_empty() {
                        completions.extend(snapshot_completions);
                    }
                    if opencode_messages_complete_active_prompt(state, &snapshot.messages) {
                        prompt_completed = true;
                        state.active_user_message_id = None;
                    }
                }
            }
        }
    }

    if state.active_user_message_id.is_some() && !prompt_completed {
        let client = OpenCodeClient::new(provider_run_id, &state.base_url)?;
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
            let snapshot_completions =
                collect_new_completed_assistant_messages(state, &snapshot.messages);
            if !snapshot_completions.is_empty() {
                completions.extend(snapshot_completions);
            }
            if state.last_status_kind.as_deref() != Some(snapshot.status.as_str()) {
                state.last_status_kind = Some(snapshot.status.clone());
                chunks.push(OpenCodeOutputChunk {
                    kind: TerminalOutputKind::ProviderStatus,
                    merge_key: Some("__provider_status__".to_string()),
                    bytes: format_session_status(&snapshot.status).into_bytes(),
                });
            }
            if opencode_messages_complete_active_prompt(state, &snapshot.messages) {
                prompt_completed = true;
                state.active_user_message_id = None;
            }
        }
    }

    Ok(OpenCodeEventDrainResult {
        chunks,
        completions,
        prompt_completed,
        terminal_failure,
        notices,
        resolved_model,
        resolved_model_source,
        resolved_variant,
        resolved_usage_tokens_total,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use serde_json::json;

    use crate::provider::{
        opencode_client::{OpenCodeMessage, OpenCodePart, OpenCodeToolState},
        AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
    };
    use crate::terminal::TerminalOutputKind;

    use super::{
        drain_opencode_events, render_tool_transcript_update,
        snapshot::latest_assistant_usage_tokens, snapshot::render_snapshot_output_chunks,
        OpenCodeAssistantCompletion, OpenCodeRuntimeState, ToolTranscriptUpdate,
    };

    fn test_run() -> RuntimeProviderRun {
        RuntimeProviderRun::new(
            "provider-run-1",
            &LaunchProviderRequest::new(
                "session-1",
                "opencode",
                "opencode",
                "default",
                "opencode/test-model",
            )
            .with_agent_id("agent-1"),
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "opencode:test".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: Default::default(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: Some("http://localhost:1".to_string()),
            },
        )
    }

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
        let rendered = render_snapshot_output_chunks(
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
        );
        let chunks = rendered.chunks;

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
    fn terminal_assistant_for_active_prompt_completes_prompt() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );
        state.note_prompt_submitted("msg_user".to_string());

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                info: serde_json::from_value(json!({
                    "id": "message-1",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "parentID": "msg_user",
                    "finish": "stop",
                    "time": { "completed": 1 }
                }))
                .expect("message info should deserialize"),
            },
        )
        .expect("message update should send");

        let first = drain_opencode_events(&test_run(), &mut state, None)
            .expect("first drain should succeed");
        assert_eq!(
            first.completions,
            vec![OpenCodeAssistantCompletion {
                message_id: "message-1".to_string(),
                completed_at_ms: 1,
            }]
        );
        assert!(first.prompt_completed);
        assert!(state.active_user_message_id.is_none());
    }

    #[test]
    fn idle_status_without_submitted_prompt_does_not_complete_prompt() {
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
            drain_opencode_events(&test_run(), &mut state, None).expect("drain should succeed");

        assert!(!result.prompt_completed);
        assert!(result.completions.is_empty());
        assert!(result
            .chunks
            .iter()
            .any(|chunk| chunk.kind == TerminalOutputKind::ProviderStatus));
    }

    #[test]
    fn idle_status_after_submitted_prompt_without_response_does_not_complete_prompt() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );
        state.note_prompt_submitted("msg_user".to_string());

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::SessionStatus {
                session_id: "session-1".to_string(),
                kind: "idle".to_string(),
            },
        )
        .expect("idle status should send");

        let result =
            drain_opencode_events(&test_run(), &mut state, None).expect("drain should succeed");

        assert!(!result.prompt_completed);
        assert_eq!(state.active_user_message_id.as_deref(), Some("msg_user"));
    }

    #[test]
    fn idle_status_after_assistant_text_does_not_complete_without_terminal_assistant() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );
        state.note_prompt_submitted("msg_user".to_string());

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                info: serde_json::from_value(json!({
                    "id": "message-1",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "parentID": "msg_user"
                }))
                .expect("message info should deserialize"),
            },
        )
        .expect("message update should send");
        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessagePartUpdated {
                part: Box::new(OpenCodePart {
                    id: "text-1".to_string(),
                    session_id: "session-1".to_string(),
                    message_id: "message-1".to_string(),
                    kind: "text".to_string(),
                    text: "answer".to_string(),
                    tool: String::new(),
                    state: None,
                    time: None,
                }),
            },
        )
        .expect("text part should send");
        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::SessionStatus {
                session_id: "session-1".to_string(),
                kind: "idle".to_string(),
            },
        )
        .expect("idle status should send");

        let result =
            drain_opencode_events(&test_run(), &mut state, None).expect("drain should succeed");

        assert!(!result.prompt_completed);
        assert_eq!(state.active_user_message_id.as_deref(), Some("msg_user"));
    }

    #[test]
    fn tool_call_assistant_blocks_prompt_completion_until_final_assistant() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );
        state.note_prompt_submitted("msg_user".to_string());

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                info: serde_json::from_value(json!({
                    "id": "message-1",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "parentID": "msg_user",
                    "finish": "tool-calls",
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

        let first = drain_opencode_events(&test_run(), &mut state, None)
            .expect("first drain should succeed");
        assert!(!first.prompt_completed);
        assert!(first
            .chunks
            .iter()
            .any(|chunk| chunk.kind == TerminalOutputKind::ProviderTool));

        let second = drain_opencode_events(&test_run(), &mut state, None)
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

        let third = drain_opencode_events(&test_run(), &mut state, None)
            .expect("third drain should succeed");
        assert!(!third.prompt_completed);

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                info: serde_json::from_value(json!({
                    "id": "message-2",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "parentID": "msg_user",
                    "finish": "stop",
                    "time": { "completed": 2 }
                }))
                .expect("message info should deserialize"),
            },
        )
        .expect("final assistant update should send");

        let fourth = drain_opencode_events(&test_run(), &mut state, None)
            .expect("fourth drain should succeed");
        assert!(fourth.prompt_completed);
    }

    #[test]
    fn idle_status_after_tool_call_only_assistant_does_not_complete_prompt() {
        let (tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            "http://localhost:1".to_string(),
            "session-1".to_string(),
            crate::provider::opencode_client::OpenCodeEventSubscription::for_tests(rx),
        );
        state.note_prompt_submitted("msg_user".to_string());

        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::MessageUpdated {
                info: serde_json::from_value(json!({
                    "id": "message-tool-calls",
                    "sessionID": "session-1",
                    "role": "assistant",
                    "parentID": "msg_user",
                    "finish": "tool-calls",
                    "time": { "completed": 1 }
                }))
                .expect("message info should deserialize"),
            },
        )
        .expect("tool-call assistant update should send");
        tx.send(
            crate::provider::opencode_client::OpenCodeEvent::SessionStatus {
                session_id: "session-1".to_string(),
                kind: "idle".to_string(),
            },
        )
        .expect("idle status should send");

        let result =
            drain_opencode_events(&test_run(), &mut state, None).expect("drain should succeed");
        assert!(!result.prompt_completed);
        assert_eq!(state.active_user_message_id.as_deref(), Some("msg_user"));
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

        let first = drain_opencode_events(&test_run(), &mut state, None)
            .expect("first drain should succeed");
        assert!(first.completions.is_empty());
        assert!(!first.prompt_completed);
        assert!(state.active_user_message_id.is_none());
    }
}
