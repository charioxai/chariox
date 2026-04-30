use std::collections::BTreeMap;
use std::thread::sleep;
use std::time::Duration;
use std::time::Instant;

use serde::Serialize;
use serde_json::{json, Value};

use crate::error::DaemonError;
use crate::session::unix_epoch_ms;
use crate::session::PromptAttachment;
use crate::terminal::TerminalOutputKind;

use super::{
    codex_client::codex_endpoint_is_healthy, CodexClient, CodexNotification, CodexRunSelection,
    CodexSocket, ProviderNativeInteractionBridge, ProviderResumeState, ProviderRunTokenUsage,
    RuntimeProviderRun,
};

const CODEX_EVENT_DRAIN_READ_TIMEOUT: Duration = Duration::from_millis(1);
const CODEX_EVENT_DRAIN_MAX_LIVE_NOTIFICATIONS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexPollResult {
    pub chunks: Vec<CodexOutputChunk>,
    pub completions: Vec<CodexAssistantCompletion>,
    pub prompt_completed: bool,
    pub terminal_failure: Option<String>,
    pub notices: Vec<String>,
    pub resolved_usage: Option<ProviderRunTokenUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexOutputChunk {
    pub kind: TerminalOutputKind,
    pub merge_key: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexAssistantCompletion {
    pub message_id: String,
    pub completed_at_ms: u64,
}

#[derive(Debug, Clone)]
struct CodexToolTranscriptState {
    item: Value,
    streamed_output: String,
    progress_messages: Vec<String>,
    last_emitted: Option<String>,
}

#[derive(Debug, Clone)]
struct CodexPendingTurnCompletion {
    completion: Option<CodexAssistantCompletion>,
    terminal_failure: Option<String>,
    notice: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CodexToolTranscriptUpdate {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    raw: Option<String>,
}

pub struct CodexRuntimeState {
    endpoint: String,
    thread_id: String,
    socket: CodexSocket,
    next_request_id: u64,
    buffered_notifications: Vec<CodexNotification>,
    active_turn_id: Option<String>,
    pending_turn_completion: Option<CodexPendingTurnCompletion>,
    tool_items: BTreeMap<String, CodexToolTranscriptState>,
}

impl std::fmt::Debug for CodexRuntimeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexRuntimeState")
            .field("endpoint", &self.endpoint)
            .field("thread_id", &self.thread_id)
            .field("next_request_id", &self.next_request_id)
            .field("buffered_notifications", &self.buffered_notifications)
            .field("active_turn_id", &self.active_turn_id)
            .field("tool_items", &self.tool_items)
            .finish()
    }
}

impl CodexRuntimeState {
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }
}

pub struct CodexRuntimeBinding {
    pub state: CodexRuntimeState,
    pub selection: CodexRunSelection,
    pub resume_state: ProviderResumeState,
}

fn codex_client_for_run(
    run: &RuntimeProviderRun,
    endpoint: &str,
    native_interaction_bridge: Option<std::sync::Arc<dyn ProviderNativeInteractionBridge>>,
) -> Result<CodexClient, DaemonError> {
    Ok(CodexClient::new(run.id(), endpoint)?
        .with_runtime_context(Some(run.session_id()), run.agent_instance_id())
        .with_runtime_mcp_binding(run.runtime_mcp_server_url(), run.runtime_mcp_auth_token())
        .with_native_interaction_bridge(native_interaction_bridge)
        .with_mcp_servers(run.mcp_servers())
        .with_write_access_mode(run.write_access_mode()))
}

pub fn initialize_codex_runtime(
    run: &RuntimeProviderRun,
) -> Result<CodexRuntimeBinding, DaemonError> {
    let endpoint = run
        .structured_endpoint()
        .ok_or_else(|| DaemonError::ProviderProtocol {
            provider_run_id: run.id().to_string(),
            operation: "codex_endpoint_missing",
            message: "codex run did not expose a structured endpoint".to_string(),
        })?
        .to_string();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !codex_endpoint_is_healthy(&endpoint) {
        if Instant::now() >= deadline {
            return Err(DaemonError::ProviderProtocol {
                provider_run_id: run.id().to_string(),
                operation: "codex_endpoint_unhealthy",
                message: format!(
                    "timed out waiting for Codex app-server to become healthy at `{endpoint}`"
                ),
            });
        }
        sleep(Duration::from_millis(100));
    }
    let client = codex_client_for_run(run, &endpoint, None)?;
    let mut socket = client.connect_initialized()?;
    let mut next_request_id = 1;
    let cwd = run
        .working_directory()
        .map(|path| path.to_string_lossy().to_string());
    let model = normalize_codex_model(run.model());
    let resumable_thread_id = run.resume_state().codex_thread_id().map(str::to_string);
    let (thread_id, selection) = match resumable_thread_id {
        Some(thread_id) => {
            crate::logging::info_with_fields(
                "daemon.provider.codex",
                "reusing codex thread",
                serde_json::json!({
                    "provider_run_id": run.id(),
                    "thread_id": thread_id,
                }),
            );
            let resume = client.thread_resume(
                &mut socket,
                &mut next_request_id,
                &thread_id,
                cwd.as_deref(),
                model.as_deref(),
                run.write_access_mode(),
                run.execution_mode(),
                run.permission_level(),
            );
            match resume {
                Ok(thread) => (
                    thread.thread.id,
                    CodexRunSelection {
                        model: Some(format!("codex/{}", thread.model)),
                        variant: thread.reasoning_effort,
                    },
                ),
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.provider.codex",
                        "codex thread resume failed; creating a new thread",
                        serde_json::json!({
                            "provider_run_id": run.id(),
                            "thread_id": thread_id,
                            "error": error.to_string(),
                        }),
                    );
                    socket = client.connect_initialized()?;
                    next_request_id = 1;
                    let thread = client.thread_start(
                        &mut socket,
                        &mut next_request_id,
                        cwd.as_deref(),
                        model.as_deref(),
                        run.write_access_mode(),
                        run.execution_mode(),
                        run.permission_level(),
                    )?;
                    (
                        thread.thread.id,
                        CodexRunSelection {
                            model: Some(format!("codex/{}", thread.model)),
                            variant: thread.reasoning_effort,
                        },
                    )
                }
            }
        }
        None => {
            let thread = client.thread_start(
                &mut socket,
                &mut next_request_id,
                cwd.as_deref(),
                model.as_deref(),
                run.write_access_mode(),
                run.execution_mode(),
                run.permission_level(),
            )?;
            (
                thread.thread.id,
                CodexRunSelection {
                    model: Some(format!("codex/{}", thread.model)),
                    variant: thread.reasoning_effort,
                },
            )
        }
    };
    Ok(CodexRuntimeBinding {
        state: CodexRuntimeState {
            endpoint,
            thread_id: thread_id.clone(),
            socket,
            next_request_id,
            buffered_notifications: Vec::new(),
            active_turn_id: None,
            pending_turn_completion: None,
            tool_items: BTreeMap::new(),
        },
        selection,
        resume_state: ProviderResumeState::from_codex_thread_id(thread_id),
    })
}

pub fn submit_codex_prompt(
    run: &RuntimeProviderRun,
    state: &mut CodexRuntimeState,
    prompt: &str,
    attachments: &[PromptAttachment],
) -> Result<(), DaemonError> {
    let client = codex_client_for_run(run, state.endpoint(), None)?;
    let cwd = run
        .working_directory()
        .map(|path| path.to_string_lossy().to_string());
    let model = normalize_codex_model(run.model());
    let effort = normalize_variant(run.variant());
    let input = codex_input(prompt, attachments);
    let thread_id = state.thread_id.clone();
    let response = match client.turn_start(
        &mut state.socket,
        &mut state.next_request_id,
        &thread_id,
        cwd.as_deref(),
        model.as_deref(),
        effort.as_deref(),
        run.write_access_mode(),
        run.execution_mode(),
        run.permission_level(),
        input,
        &mut state.buffered_notifications,
    ) {
        Ok(response) => response,
        Err(error) => {
            state.buffered_notifications.push(CodexNotification::Error {
                message: error.to_string(),
            });
            return Ok(());
        }
    };
    if let Some(turn_id) = codex_turn_id_from_start_response(&response) {
        state.active_turn_id = Some(turn_id);
    }
    crate::logging::debug_with_fields(
        "daemon.provider.codex",
        "codex turn start response trace",
        json!({
            "provider_run_id": run.id(),
            "active_turn_id": state.active_turn_id,
            "response": response,
        }),
    );
    Ok(())
}

pub fn abort_codex_turn(
    provider_run_id: &str,
    state: &mut CodexRuntimeState,
) -> Result<(), DaemonError> {
    let Some(turn_id) = state.active_turn_id.clone() else {
        return Ok(());
    };
    let client = CodexClient::new(provider_run_id, state.endpoint())?;
    client.turn_interrupt(&mut state.socket, &mut state.next_request_id, &turn_id)
}

pub fn drain_codex_events(
    run: &RuntimeProviderRun,
    state: &mut CodexRuntimeState,
    native_interaction_bridge: Option<std::sync::Arc<dyn ProviderNativeInteractionBridge>>,
) -> Result<CodexPollResult, DaemonError> {
    let client = codex_client_for_run(run, state.endpoint(), native_interaction_bridge)?;
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    for notification in std::mem::take(&mut state.buffered_notifications) {
        apply_notification(
            notification,
            &mut state.active_turn_id,
            &mut state.pending_turn_completion,
            &mut state.tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
    }

    for _ in 0..CODEX_EVENT_DRAIN_MAX_LIVE_NOTIFICATIONS {
        let Some(notification) =
            client.read_notification(&mut state.socket, CODEX_EVENT_DRAIN_READ_TIMEOUT)?
        else {
            break;
        };
        apply_notification(
            notification,
            &mut state.active_turn_id,
            &mut state.pending_turn_completion,
            &mut state.tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
    }

    Ok(CodexPollResult {
        chunks,
        completions,
        prompt_completed,
        terminal_failure,
        notices,
        resolved_usage,
    })
}

fn apply_notification(
    notification: CodexNotification,
    active_turn_id: &mut Option<String>,
    pending_turn_completion: &mut Option<CodexPendingTurnCompletion>,
    tool_items: &mut BTreeMap<String, CodexToolTranscriptState>,
    chunks: &mut Vec<CodexOutputChunk>,
    completions: &mut Vec<CodexAssistantCompletion>,
    notices: &mut Vec<String>,
    prompt_completed: &mut bool,
    terminal_failure: &mut Option<String>,
    resolved_usage: &mut Option<ProviderRunTokenUsage>,
) {
    match notification {
        CodexNotification::AgentMessageDelta { item_id, delta } => {
            if delta.is_empty() {
                return;
            }
            chunks.push(CodexOutputChunk {
                kind: TerminalOutputKind::ProviderOutput,
                merge_key: Some(normalize_merge_key(&item_id, "codex-agent-message")),
                bytes: delta.into_bytes(),
            });
        }
        CodexNotification::ReasoningTextDelta { item_id, delta }
        | CodexNotification::ReasoningSummaryTextDelta { item_id, delta } => {
            if delta.is_empty() {
                return;
            }
            chunks.push(CodexOutputChunk {
                kind: TerminalOutputKind::ProviderReasoning,
                merge_key: Some(normalize_merge_key(&item_id, "codex-reasoning")),
                bytes: delta.into_bytes(),
            });
        }
        CodexNotification::ReasoningSummaryPartAdded {
            item_id,
            summary_index,
        } => {
            if summary_index == 0 {
                return;
            }
            chunks.push(CodexOutputChunk {
                kind: TerminalOutputKind::ProviderReasoning,
                merge_key: Some(normalize_merge_key(&item_id, "codex-reasoning")),
                bytes: b"\n\n".to_vec(),
            });
        }
        CodexNotification::ItemStarted { item } | CodexNotification::ItemCompleted { item } => {
            trace_codex_tool_item("item_lifecycle", &item);
            if let Some(chunk) = sync_tool_item(tool_items, &item) {
                chunks.push(chunk);
            }
            maybe_complete_pending_turn(
                pending_turn_completion,
                tool_items,
                completions,
                notices,
                prompt_completed,
                terminal_failure,
            );
        }
        CodexNotification::CommandExecutionOutputDelta { item_id, delta } => {
            if let Some(chunk) =
                append_tool_output_delta(tool_items, &item_id, "commandExecution", &delta)
            {
                chunks.push(chunk);
            }
        }
        CodexNotification::FileChangeOutputDelta { item_id, delta } => {
            if let Some(chunk) =
                append_tool_output_delta(tool_items, &item_id, "fileChange", &delta)
            {
                chunks.push(chunk);
            }
        }
        CodexNotification::McpToolCallProgress { item_id, message } => {
            if let Some(chunk) = append_tool_progress(tool_items, &item_id, &message) {
                chunks.push(chunk);
            }
        }
        CodexNotification::TokenUsageUpdated { usage, .. } => {
            *resolved_usage = Some(usage);
        }
        CodexNotification::TurnStarted { turn_id } => {
            crate::logging::debug_with_fields(
                "daemon.provider.codex",
                "codex turn started trace",
                json!({
                    "previous_active_turn_id": active_turn_id,
                    "turn_id": turn_id,
                }),
            );
            if !turn_id.is_empty() {
                *active_turn_id = Some(turn_id);
            }
        }
        CodexNotification::TurnCompleted {
            turn_id,
            status,
            error_message,
        } => {
            if active_turn_id.as_deref() != Some(turn_id.as_str()) {
                crate::logging::debug_with_fields(
                    "daemon.provider.codex",
                    "codex turn completion ignored by active turn mismatch",
                    json!({
                        "active_turn_id": active_turn_id,
                        "turn_id": turn_id,
                        "status": status,
                        "error_message": error_message,
                    }),
                );
                return;
            }
            crate::logging::debug_with_fields(
                "daemon.provider.codex",
                "codex turn completion accepted",
                json!({
                    "turn_id": turn_id,
                    "status": status,
                    "has_running_tool_items": has_running_tool_items(tool_items),
                }),
            );
            if !turn_id.is_empty() {
                let completion = CodexAssistantCompletion {
                    message_id: format!("codex-turn:{turn_id}"),
                    completed_at_ms: unix_epoch_ms(),
                };
                let pending = CodexPendingTurnCompletion {
                    completion: Some(completion),
                    terminal_failure: if status == "failed" {
                        Some(
                            error_message
                                .clone()
                                .unwrap_or_else(|| "Codex turn failed".to_string()),
                        )
                    } else {
                        None
                    },
                    notice: error_message
                        .clone()
                        .or_else(|| (status == "failed").then(|| "Codex turn failed".to_string())),
                };
                if has_running_tool_items(tool_items) {
                    crate::logging::debug_with_fields(
                        "daemon.provider.codex",
                        "codex turn completion deferred by running tool items",
                        json!({
                            "turn_id": turn_id,
                            "status": status,
                            "running_tool_items": running_tool_item_summaries(tool_items),
                        }),
                    );
                    *pending_turn_completion = Some(pending);
                } else {
                    complete_pending_turn(
                        pending,
                        completions,
                        notices,
                        prompt_completed,
                        terminal_failure,
                    );
                }
            }
            if !turn_id.is_empty() {
                *active_turn_id = None;
            }
        }
        CodexNotification::Error { message } => {
            *terminal_failure = Some(message.clone());
            *prompt_completed = true;
            notices.push(message);
        }
    }
}

fn maybe_complete_pending_turn(
    pending_turn_completion: &mut Option<CodexPendingTurnCompletion>,
    tool_items: &BTreeMap<String, CodexToolTranscriptState>,
    completions: &mut Vec<CodexAssistantCompletion>,
    notices: &mut Vec<String>,
    prompt_completed: &mut bool,
    terminal_failure: &mut Option<String>,
) {
    if has_running_tool_items(tool_items) {
        return;
    }
    if let Some(pending) = pending_turn_completion.take() {
        complete_pending_turn(
            pending,
            completions,
            notices,
            prompt_completed,
            terminal_failure,
        );
    }
}

fn complete_pending_turn(
    pending: CodexPendingTurnCompletion,
    completions: &mut Vec<CodexAssistantCompletion>,
    notices: &mut Vec<String>,
    prompt_completed: &mut bool,
    terminal_failure: &mut Option<String>,
) {
    if let Some(completion) = pending.completion {
        completions.push(completion);
    }
    if let Some(message) = pending.terminal_failure {
        *terminal_failure = Some(message);
    }
    if let Some(message) = pending.notice {
        notices.push(message);
    }
    *prompt_completed = true;
}

fn has_running_tool_items(tool_items: &BTreeMap<String, CodexToolTranscriptState>) -> bool {
    tool_items.values().any(|state| {
        normalize_codex_tool_status(
            state
                .item
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        ) == "running"
    })
}

fn running_tool_item_summaries(
    tool_items: &BTreeMap<String, CodexToolTranscriptState>,
) -> Vec<Value> {
    tool_items
        .iter()
        .filter_map(|(item_id, state)| {
            let raw_status = state
                .item
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default();
            (normalize_codex_tool_status(raw_status) == "running").then(|| {
                json!({
                    "id": item_id,
                    "type": state.item.get("type").and_then(Value::as_str),
                    "status": raw_status,
                    "command": state.item.get("command").and_then(Value::as_str),
                    "streamed_output_len": state.streamed_output.len(),
                })
            })
        })
        .collect()
}

fn trace_codex_tool_item(label: &str, item: &Value) {
    crate::logging::debug_with_fields(
        "daemon.provider.codex",
        "codex tool item trace",
        json!({
            "label": label,
            "id": item.get("id").and_then(Value::as_str),
            "type": item.get("type").and_then(Value::as_str),
            "status": item.get("status").and_then(Value::as_str),
            "command": item.get("command").and_then(Value::as_str),
            "exit_code": item.get("exitCode").and_then(Value::as_i64),
            "process_id": item.get("processId").and_then(Value::as_str),
            "aggregated_output_len": item.get("aggregatedOutput").and_then(Value::as_str).map(str::len),
        }),
    );
}

fn codex_turn_id_from_start_response(response: &Value) -> Option<String> {
    response
        .get("turn")
        .and_then(|turn| turn.get("id"))
        .and_then(Value::as_str)
        .or_else(|| response.get("id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn sync_tool_item(
    tool_items: &mut BTreeMap<String, CodexToolTranscriptState>,
    item: &Value,
) -> Option<CodexOutputChunk> {
    if !is_codex_tool_item(item) {
        return None;
    }
    let item_id = item.get("id").and_then(Value::as_str)?.to_string();
    let entry = tool_items
        .entry(item_id.clone())
        .or_insert_with(|| CodexToolTranscriptState {
            item: item.clone(),
            streamed_output: String::new(),
            progress_messages: Vec::new(),
            last_emitted: None,
        });
    entry.item = item.clone();
    render_tool_chunk_if_changed(&item_id, entry)
}

fn append_tool_output_delta(
    tool_items: &mut BTreeMap<String, CodexToolTranscriptState>,
    item_id: &str,
    item_type: &str,
    delta: &str,
) -> Option<CodexOutputChunk> {
    if delta.is_empty() {
        return None;
    }
    let entry = tool_items
        .entry(item_id.to_string())
        .or_insert_with(|| CodexToolTranscriptState {
            item: placeholder_tool_item(item_id, item_type),
            streamed_output: String::new(),
            progress_messages: Vec::new(),
            last_emitted: None,
        });
    entry.streamed_output.push_str(delta);
    render_tool_chunk_if_changed(item_id, entry)
}

fn append_tool_progress(
    tool_items: &mut BTreeMap<String, CodexToolTranscriptState>,
    item_id: &str,
    message: &str,
) -> Option<CodexOutputChunk> {
    if message.trim().is_empty() {
        return None;
    }
    let entry = tool_items
        .entry(item_id.to_string())
        .or_insert_with(|| CodexToolTranscriptState {
            item: placeholder_tool_item(item_id, "mcpToolCall"),
            streamed_output: String::new(),
            progress_messages: Vec::new(),
            last_emitted: None,
        });
    entry.progress_messages.push(message.trim().to_string());
    render_tool_chunk_if_changed(item_id, entry)
}

fn render_tool_chunk_if_changed(
    item_id: &str,
    state: &mut CodexToolTranscriptState,
) -> Option<CodexOutputChunk> {
    let rendered = render_codex_tool_transcript_update(state)?;
    if state.last_emitted.as_deref() == Some(rendered.as_str()) {
        return None;
    }
    state.last_emitted = Some(rendered.clone());
    Some(CodexOutputChunk {
        kind: TerminalOutputKind::ProviderTool,
        merge_key: Some(item_id.to_string()),
        bytes: rendered.into_bytes(),
    })
}

fn render_codex_tool_transcript_update(state: &CodexToolTranscriptState) -> Option<String> {
    let item = &state.item;
    let id = item.get("id").and_then(Value::as_str)?.to_string();
    let item_type = item.get("type").and_then(Value::as_str)?;
    let status = normalize_codex_tool_status(
        item.get("status")
            .and_then(Value::as_str)
            .unwrap_or("updated"),
    );

    let update = match item_type {
        "commandExecution" => CodexToolTranscriptUpdate {
            id,
            tool: Some("bash".to_string()),
            status: Some(status),
            title: None,
            description: item
                .get("cwd")
                .and_then(Value::as_str)
                .and_then(non_empty)
                .map(|cwd| format!("cwd {cwd}")),
            text: None,
            input: Some(json!({
                "command": item.get("command").and_then(Value::as_str).unwrap_or_default(),
                "cwd": item.get("cwd").and_then(Value::as_str).unwrap_or_default(),
            })),
            output: prefer_output(
                item.get("aggregatedOutput").and_then(Value::as_str),
                &state.streamed_output,
            ),
            error: command_execution_error(item),
            raw: command_execution_raw(item),
        },
        "fileChange" => CodexToolTranscriptUpdate {
            id,
            tool: Some("apply_patch".to_string()),
            status: Some(status),
            title: item
                .get("changes")
                .and_then(Value::as_array)
                .map(|changes| format!("{} file changes", changes.len()))
                .filter(|title| !title.starts_with('0')),
            description: None,
            text: None,
            input: None,
            output: prefer_output(None, &state.streamed_output),
            error: None,
            raw: item
                .get("changes")
                .filter(|value| !is_empty_json_value(value))
                .map(render_json_value),
        },
        "mcpToolCall" => CodexToolTranscriptUpdate {
            id,
            tool: item
                .get("tool")
                .and_then(Value::as_str)
                .and_then(non_empty)
                .map(str::to_string)
                .or_else(|| Some("mcp".to_string())),
            status: Some(status),
            title: item
                .get("server")
                .and_then(Value::as_str)
                .and_then(non_empty)
                .map(str::to_string),
            description: None,
            text: (!state.progress_messages.is_empty()).then(|| state.progress_messages.join("\n")),
            input: item
                .get("arguments")
                .filter(|value| !is_empty_json_value(value))
                .cloned(),
            output: item
                .get("result")
                .filter(|value| !is_empty_json_value(value))
                .map(render_json_value),
            error: item
                .get("error")
                .and_then(|value| {
                    value
                        .get("message")
                        .and_then(Value::as_str)
                        .or_else(|| value.as_str())
                })
                .and_then(non_empty)
                .map(str::to_string),
            raw: None,
        },
        "dynamicToolCall" => CodexToolTranscriptUpdate {
            id,
            tool: item
                .get("tool")
                .and_then(Value::as_str)
                .and_then(non_empty)
                .map(str::to_string)
                .or_else(|| Some("tool".to_string())),
            status: Some(status),
            title: None,
            description: None,
            text: None,
            input: item
                .get("arguments")
                .filter(|value| !is_empty_json_value(value))
                .cloned(),
            output: item
                .get("contentItems")
                .filter(|value| !is_empty_json_value(value))
                .map(render_json_value),
            error: item
                .get("success")
                .and_then(Value::as_bool)
                .filter(|success| !success)
                .map(|_| "Dynamic tool call failed".to_string()),
            raw: None,
        },
        "collabAgentToolCall" => CodexToolTranscriptUpdate {
            id,
            tool: item
                .get("tool")
                .and_then(Value::as_str)
                .and_then(non_empty)
                .map(str::to_string)
                .or_else(|| Some("collab".to_string())),
            status: Some(status),
            title: None,
            description: None,
            text: None,
            input: Some(json!({
                "prompt": item.get("prompt").cloned().unwrap_or(Value::Null),
                "receiverThreadIds": item.get("receiverThreadIds").cloned().unwrap_or(Value::Null),
                "model": item.get("model").cloned().unwrap_or(Value::Null),
                "reasoningEffort": item.get("reasoningEffort").cloned().unwrap_or(Value::Null),
            })),
            output: item
                .get("agentsStates")
                .filter(|value| !is_empty_json_value(value))
                .map(render_json_value),
            error: None,
            raw: None,
        },
        _ => return None,
    };

    serde_json::to_string(&update).ok()
}

fn placeholder_tool_item(item_id: &str, item_type: &str) -> Value {
    json!({
        "id": item_id,
        "type": item_type,
        "status": "inProgress",
    })
}

fn is_codex_tool_item(item: &Value) -> bool {
    matches!(
        item.get("type").and_then(Value::as_str),
        Some(
            "commandExecution"
                | "fileChange"
                | "mcpToolCall"
                | "dynamicToolCall"
                | "collabAgentToolCall"
        )
    )
}

fn normalize_merge_key(item_id: &str, fallback: &str) -> String {
    non_empty(item_id).unwrap_or(fallback).to_string()
}

fn normalize_codex_tool_status(status: &str) -> String {
    match status {
        "inProgress" => "running".to_string(),
        "completed" => "completed".to_string(),
        "failed" => "error".to_string(),
        "declined" => "declined".to_string(),
        other => non_empty(other).unwrap_or("updated").to_string(),
    }
}

fn command_execution_error(item: &Value) -> Option<String> {
    let status = item
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if status != "failed" && status != "declined" {
        return None;
    }
    item.get("aggregatedOutput")
        .and_then(Value::as_str)
        .and_then(non_empty)
        .map(str::to_string)
        .or_else(|| Some(format!("Command {status}")))
}

fn command_execution_raw(item: &Value) -> Option<String> {
    let exit_code = item.get("exitCode").and_then(Value::as_i64);
    let duration = item.get("durationMs").and_then(Value::as_i64);
    let process_id = item
        .get("processId")
        .and_then(Value::as_str)
        .and_then(non_empty);
    let mut lines = Vec::new();
    if let Some(exit_code) = exit_code {
        lines.push(format!("exit_code: {exit_code}"));
    }
    if let Some(duration) = duration {
        lines.push(format!("duration_ms: {duration}"));
    }
    if let Some(process_id) = process_id {
        lines.push(format!("process_id: {process_id}"));
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn prefer_output(primary: Option<&str>, streamed_output: &str) -> Option<String> {
    primary
        .and_then(non_empty)
        .map(str::to_string)
        .or_else(|| non_empty(streamed_output).map(str::to_string))
}

fn render_json_value(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn is_empty_json_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Array(items) => items.is_empty(),
        Value::Object(items) => items.is_empty(),
        Value::String(text) => text.trim().is_empty(),
        _ => false,
    }
}

fn codex_input(prompt: &str, attachments: &[PromptAttachment]) -> Vec<Value> {
    let mut input = Vec::new();
    if !prompt.trim().is_empty() {
        input.push(json!({
            "type": "text",
            "text": prompt,
        }));
    }

    let mut attachment_notes = Vec::new();
    for attachment in attachments {
        if attachment.mime().starts_with("image/") {
            if let Some(local_path) = resolve_local_attachment_path(attachment.url()) {
                input.push(json!({
                    "type": "localImage",
                    "path": local_path,
                }));
            } else {
                input.push(json!({
                    "type": "image",
                    "url": attachment.url(),
                }));
            }
            continue;
        }
        let label = attachment
            .filename()
            .map(str::to_string)
            .unwrap_or_else(|| attachment.url().to_string());
        attachment_notes.push(format!(
            "Attachment: {label} ({}) at {}",
            attachment.mime(),
            attachment.url()
        ));
    }

    if !attachment_notes.is_empty() {
        input.push(json!({
            "type": "text",
            "text": attachment_notes.join("\n"),
        }));
    }

    input
}

fn resolve_local_attachment_path(url: &str) -> Option<String> {
    if url.starts_with('/') {
        return Some(url.to_string());
    }

    let stripped = url
        .strip_prefix("file://localhost")
        .or_else(|| url.strip_prefix("file://"))?;
    if !stripped.starts_with('/') {
        return None;
    }

    Some(percent_decode_path(stripped))
}

fn percent_decode_path(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hi = decode_hex_nibble(bytes[index + 1]);
            let lo = decode_hex_nibble(bytes[index + 2]);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                decoded.push((hi << 4) | lo);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn decode_hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn normalize_codex_model(model: &str) -> Option<String> {
    let model = model.trim();
    if model.is_empty() || model == "default" {
        return None;
    }
    Some(model.strip_prefix("codex/").unwrap_or(model).to_string())
}

fn normalize_variant(variant: Option<&str>) -> Option<String> {
    variant
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "default")
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::{json, Value};

    use crate::provider::ProviderRunTokenUsage;
    use crate::session::PromptAttachment;
    use crate::terminal::TerminalOutputKind;

    use super::{
        apply_notification, codex_input, render_codex_tool_transcript_update,
        resolve_local_attachment_path, CodexNotification, CodexOutputChunk,
        CodexToolTranscriptState,
    };

    #[test]
    fn codex_input_treats_file_url_images_as_local_images() {
        let input = codex_input(
            "describe this image",
            &[PromptAttachment::new(
                "file:///tmp/capture%20one.png",
                "image/png",
                Some("capture one.png".to_string()),
            )],
        );

        assert_eq!(
            input,
            vec![
                json!({
                    "type": "text",
                    "text": "describe this image",
                }),
                json!({
                    "type": "localImage",
                    "path": "/tmp/capture one.png",
                }),
            ]
        );
    }

    #[test]
    fn resolve_local_attachment_path_accepts_file_urls_and_decodes_percent_escapes() {
        assert_eq!(
            resolve_local_attachment_path("file:///tmp/a%20b.png"),
            Some("/tmp/a b.png".to_string())
        );
        assert_eq!(
            resolve_local_attachment_path("file://localhost/tmp/a%20b.png"),
            Some("/tmp/a b.png".to_string())
        );
        assert_eq!(
            resolve_local_attachment_path("/tmp/a b.png"),
            Some("/tmp/a b.png".to_string())
        );
        assert_eq!(
            resolve_local_attachment_path("https://example.com/a.png"),
            None
        );
    }

    #[test]
    fn reasoning_and_agent_deltas_preserve_item_merge_keys() {
        let mut active_turn_id = None;
        let mut pending_turn_completion = None;
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::ReasoningTextDelta {
                item_id: "reason-1".to_string(),
                delta: "thinking".to_string(),
            },
            &mut active_turn_id,
            &mut pending_turn_completion,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        apply_notification(
            CodexNotification::AgentMessageDelta {
                item_id: "msg-1".to_string(),
                delta: "answer".to_string(),
            },
            &mut active_turn_id,
            &mut pending_turn_completion,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );

        assert_eq!(
            chunks
                .iter()
                .map(|chunk| (
                    chunk.kind.clone(),
                    chunk.merge_key.clone().unwrap_or_default(),
                    String::from_utf8_lossy(&chunk.bytes).into_owned()
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    TerminalOutputKind::ProviderReasoning,
                    "reason-1".to_string(),
                    "thinking".to_string()
                ),
                (
                    TerminalOutputKind::ProviderOutput,
                    "msg-1".to_string(),
                    "answer".to_string()
                ),
            ]
        );
    }

    #[test]
    fn token_usage_notification_is_projected() {
        let mut active_turn_id = Some("turn-1".to_string());
        let mut pending_turn_completion = None;
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::TokenUsageUpdated {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                usage: ProviderRunTokenUsage {
                    total_tokens: Some(42_100),
                    last_tokens: Some(8_900),
                    context_window: Some(128_000),
                },
            },
            &mut active_turn_id,
            &mut pending_turn_completion,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );

        assert_eq!(
            resolved_usage,
            Some(ProviderRunTokenUsage {
                total_tokens: Some(42_100),
                last_tokens: Some(8_900),
                context_window: Some(128_000),
            })
        );
        assert!(chunks.is_empty());
        assert!(completions.is_empty());
        assert!(notices.is_empty());
        assert!(!prompt_completed);
        assert!(terminal_failure.is_none());
    }

    #[test]
    fn command_execution_updates_are_rendered_cumulatively() {
        let mut active_turn_id = None;
        let mut pending_turn_completion = None;
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::ItemStarted {
                item: json!({
                    "type": "commandExecution",
                    "id": "cmd-1",
                    "command": "ls -la",
                    "cwd": "/tmp",
                    "status": "inProgress",
                    "commandActions": [],
                    "aggregatedOutput": null,
                    "exitCode": null,
                    "durationMs": null,
                    "processId": "pty-1",
                }),
            },
            &mut active_turn_id,
            &mut pending_turn_completion,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        apply_notification(
            CodexNotification::CommandExecutionOutputDelta {
                item_id: "cmd-1".to_string(),
                delta: "alpha\n".to_string(),
            },
            &mut active_turn_id,
            &mut pending_turn_completion,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        apply_notification(
            CodexNotification::CommandExecutionOutputDelta {
                item_id: "cmd-1".to_string(),
                delta: "beta\n".to_string(),
            },
            &mut active_turn_id,
            &mut pending_turn_completion,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        apply_notification(
            CodexNotification::ItemCompleted {
                item: json!({
                    "type": "commandExecution",
                    "id": "cmd-1",
                    "command": "ls -la",
                    "cwd": "/tmp",
                    "status": "completed",
                    "commandActions": [],
                    "aggregatedOutput": "alpha\nbeta\n",
                    "exitCode": 0,
                    "durationMs": 42,
                    "processId": "pty-1",
                }),
            },
            &mut active_turn_id,
            &mut pending_turn_completion,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );

        let tool_chunks = chunks
            .into_iter()
            .filter(|chunk| chunk.kind == TerminalOutputKind::ProviderTool)
            .collect::<Vec<CodexOutputChunk>>();
        assert_eq!(tool_chunks.len(), 4);

        let second = parse_tool_chunk(&tool_chunks[1]);
        assert_eq!(second["tool"], "bash");
        assert_eq!(second["status"], "running");
        assert_eq!(second["output"], "alpha");

        let third = parse_tool_chunk(&tool_chunks[2]);
        assert_eq!(third["output"], "alpha\nbeta");

        let fourth = parse_tool_chunk(&tool_chunks[3]);
        assert_eq!(fourth["status"], "completed");
        assert_eq!(fourth["output"], "alpha\nbeta");
    }

    #[test]
    fn mcp_tool_progress_is_projected_into_tool_text() {
        let rendered = render_codex_tool_transcript_update(&CodexToolTranscriptState {
            item: json!({
                "type": "mcpToolCall",
                "id": "tool-1",
                "server": "arroba-runtime",
                "tool": "validate_workflow_output",
                "status": "inProgress",
                "arguments": { "value": 1 },
                "result": null,
                "error": null,
                "durationMs": null
            }),
            streamed_output: String::new(),
            progress_messages: vec!["checking schema".to_string()],
            last_emitted: None,
        })
        .expect("payload should render");

        let parsed = serde_json::from_str::<Value>(&rendered).expect("payload should deserialize");
        assert_eq!(parsed["tool"], "validate_workflow_output");
        assert_eq!(parsed["title"], "arroba-runtime");
        assert_eq!(parsed["text"], "checking schema");
    }

    #[test]
    fn only_turn_completed_marks_the_prompt_as_complete() {
        let mut active_turn_id = Some("turn-1".to_string());
        let mut pending_turn_completion = None;
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::ItemCompleted {
                item: json!({
                    "type": "commandExecution",
                    "id": "cmd-1",
                    "command": "ls",
                    "status": "completed",
                }),
            },
            &mut active_turn_id,
            &mut pending_turn_completion,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        assert!(!prompt_completed);
        assert_eq!(active_turn_id.as_deref(), Some("turn-1"));

        apply_notification(
            CodexNotification::TurnCompleted {
                turn_id: "turn-1".to_string(),
                status: "completed".to_string(),
                error_message: None,
            },
            &mut active_turn_id,
            &mut pending_turn_completion,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        assert!(prompt_completed);
        assert_eq!(active_turn_id, None);
        assert_eq!(completions.len(), 1);
        assert_eq!(completions[0].message_id, "codex-turn:turn-1");
    }

    #[test]
    fn turn_completion_waits_for_running_command_execution() {
        let mut active_turn_id = Some("turn-1".to_string());
        let mut pending_turn_completion = None;
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::ItemStarted {
                item: json!({
                    "type": "commandExecution",
                    "id": "cmd-1",
                    "command": "pnpm test",
                    "status": "inProgress",
                }),
            },
            &mut active_turn_id,
            &mut pending_turn_completion,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        apply_notification(
            CodexNotification::TurnCompleted {
                turn_id: "turn-1".to_string(),
                status: "completed".to_string(),
                error_message: None,
            },
            &mut active_turn_id,
            &mut pending_turn_completion,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );

        assert!(!prompt_completed);
        assert!(completions.is_empty());
        assert!(pending_turn_completion.is_some());

        apply_notification(
            CodexNotification::ItemCompleted {
                item: json!({
                    "type": "commandExecution",
                    "id": "cmd-1",
                    "command": "pnpm test",
                    "status": "completed",
                    "aggregatedOutput": "ok",
                    "exitCode": 0,
                }),
            },
            &mut active_turn_id,
            &mut pending_turn_completion,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );

        assert!(prompt_completed);
        assert!(pending_turn_completion.is_none());
        assert_eq!(completions.len(), 1);
        assert_eq!(completions[0].message_id, "codex-turn:turn-1");
    }

    #[test]
    fn stale_turn_completion_does_not_complete_prompt() {
        let mut active_turn_id = Some("current-turn".to_string());
        let mut pending_turn_completion = None;
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::TurnCompleted {
                turn_id: "stale-turn".to_string(),
                status: "completed".to_string(),
                error_message: None,
            },
            &mut active_turn_id,
            &mut pending_turn_completion,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );

        assert!(!prompt_completed);
        assert!(completions.is_empty());
        assert_eq!(active_turn_id.as_deref(), Some("current-turn"));
    }

    #[test]
    fn interrupted_turn_is_treated_as_terminal_cancellation() {
        let mut active_turn_id = Some("turn-2".to_string());
        let mut pending_turn_completion = None;
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::TurnCompleted {
                turn_id: "turn-2".to_string(),
                status: "interrupted".to_string(),
                error_message: Some("Aborted".to_string()),
            },
            &mut active_turn_id,
            &mut pending_turn_completion,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );

        assert!(prompt_completed);
        assert_eq!(active_turn_id, None);
        assert_eq!(completions.len(), 1);
        assert_eq!(completions[0].message_id, "codex-turn:turn-2");
        assert_eq!(notices, vec!["Aborted".to_string()]);
    }

    #[test]
    fn failed_turn_records_terminal_failure() {
        let mut active_turn_id = Some("turn-3".to_string());
        let mut pending_turn_completion = None;
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::TurnCompleted {
                turn_id: "turn-3".to_string(),
                status: "failed".to_string(),
                error_message: Some("model rejected".to_string()),
            },
            &mut active_turn_id,
            &mut pending_turn_completion,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );

        assert!(prompt_completed);
        assert_eq!(terminal_failure.as_deref(), Some("model rejected"));
        assert_eq!(notices, vec!["model rejected".to_string()]);
    }

    #[test]
    fn error_notification_without_active_turn_records_terminal_failure() {
        let mut active_turn_id = None;
        let mut pending_turn_completion = None;
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::Error {
                message: "unsupported model gpt-5.2-codex".to_string(),
            },
            &mut active_turn_id,
            &mut pending_turn_completion,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );

        assert!(prompt_completed);
        assert_eq!(
            terminal_failure.as_deref(),
            Some("unsupported model gpt-5.2-codex")
        );
        assert_eq!(notices, vec!["unsupported model gpt-5.2-codex".to_string()]);
    }

    fn parse_tool_chunk(chunk: &CodexOutputChunk) -> Value {
        serde_json::from_slice(&chunk.bytes).expect("tool chunk should be JSON")
    }
}
