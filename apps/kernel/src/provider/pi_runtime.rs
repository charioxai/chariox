use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use serde_json::{json, Value};

use crate::error::DaemonError;
use crate::prompt_assembly::PromptEnvelope;
use crate::provider::{
    AgentExecutionMode, AgentPermissionLevel, ProviderAssistantCompletion, ProviderPromptChunk,
    ProviderPromptSignalBatch, ProviderResumeState, ProviderRunTokenUsage, RuntimeProviderRun,
};
use crate::session::unix_epoch_ms;
use crate::terminal::TerminalOutputKind;

const PI_EVENT_DRAIN_MAX_MESSAGES: usize = 256;

pub(crate) struct PiRuntimeBinding {
    pub state: PiRuntimeState,
    pub selection: PiRunSelection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PiRunSelection {
    pub model: Option<String>,
    pub variant: Option<String>,
}

pub struct PiRuntimeState {
    program: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    env_remove: Vec<String>,
    working_directory: Option<PathBuf>,
    child: Child,
    stdin: ChildStdin,
    receiver: Receiver<PiRuntimeMessage>,
    active_model: String,
    active_variant: Option<String>,
    active_execution_mode: AgentExecutionMode,
    active_permission_level: AgentPermissionLevel,
    session_id: Option<String>,
    active_turn_id: Option<String>,
    cancelled_turn_pending_settlement: bool,
    next_request_number: u64,
    result_number: u64,
    emitted_text_offsets: BTreeMap<String, usize>,
    saw_text_delta: bool,
    exit_reported: bool,
}

impl std::fmt::Debug for PiRuntimeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PiRuntimeState")
            .field("program", &self.program)
            .field("args", &self.args)
            .field("working_directory", &self.working_directory)
            .field("active_model", &self.active_model)
            .field("active_variant", &self.active_variant)
            .field("active_execution_mode", &self.active_execution_mode)
            .field("active_permission_level", &self.active_permission_level)
            .field("session_id", &self.session_id)
            .field("active_turn_id", &self.active_turn_id)
            .field(
                "cancelled_turn_pending_settlement",
                &self.cancelled_turn_pending_settlement,
            )
            .field("next_request_number", &self.next_request_number)
            .field("result_number", &self.result_number)
            .field("emitted_text_offsets", &self.emitted_text_offsets)
            .field("saw_text_delta", &self.saw_text_delta)
            .field("exit_reported", &self.exit_reported)
            .finish()
    }
}

impl Drop for PiRuntimeState {
    fn drop(&mut self) {
        stop_child(&mut self.child);
    }
}

enum PiRuntimeMessage {
    Stdout(Value),
    StdoutParseError(String),
    Stderr(String),
}

pub(crate) fn initialize_pi_runtime(
    run: &RuntimeProviderRun,
) -> Result<PiRuntimeBinding, DaemonError> {
    let program = run
        .pty_program()
        .ok_or_else(|| DaemonError::ProviderProtocol {
            provider_run_id: run.id().to_string(),
            operation: "pi_executable_missing",
            message: "Pi provider run did not include an executable".to_string(),
        })?
        .to_string();
    let args = run.pty_args().to_vec();
    let env = run.pty_env().clone();
    let env_remove = run.pty_env_remove().to_vec();
    let working_directory = run.working_directory().cloned();
    let (child, stdin, receiver) = spawn_pi_child(
        run.id(),
        &program,
        &args,
        &env,
        &env_remove,
        working_directory.as_ref(),
        "initialize_pi_runtime",
    )?;

    let session_id = run.resume_state().pi_session_id().map(str::to_string);
    Ok(PiRuntimeBinding {
        state: PiRuntimeState {
            program,
            args,
            env,
            env_remove,
            working_directory,
            child,
            stdin,
            receiver,
            active_model: run.model().to_string(),
            active_variant: run.variant().map(str::to_string),
            active_execution_mode: run.execution_mode(),
            active_permission_level: run.permission_level(),
            session_id,
            active_turn_id: None,
            cancelled_turn_pending_settlement: false,
            next_request_number: 1,
            result_number: 1,
            emitted_text_offsets: BTreeMap::new(),
            saw_text_delta: false,
            exit_reported: false,
        },
        selection: PiRunSelection {
            model: Some(run.model().to_string()),
            variant: run.variant().map(str::to_string),
        },
    })
}

pub(crate) fn submit_pi_prompt(
    run: &RuntimeProviderRun,
    state: &mut PiRuntimeState,
    envelope: &PromptEnvelope,
) -> Result<(), DaemonError> {
    if pi_runtime_selection_changed(run, state) || pi_runtime_child_exited(state) {
        restart_pi_runtime(run, state, "pi_restart_for_selection_change")?;
    }
    let request_id = next_request_id(run.id(), state);
    state.active_turn_id = Some(request_id.clone());
    state.saw_text_delta = false;
    state.emitted_text_offsets.clear();
    let prompt_text = pi_prompt_text(envelope);
    let message = if envelope.steering {
        json!({
            "id": request_id,
            "type": "steer",
            "message": prompt_text,
        })
    } else {
        json!({
            "id": request_id,
            "type": "prompt",
            "message": prompt_text,
        })
    };
    write_json_line(&mut state.stdin, &message)?;
    request_pi_state(run.id(), state)
}

pub(crate) fn abort_pi_turn(
    run: &RuntimeProviderRun,
    state: &mut PiRuntimeState,
) -> Result<(), DaemonError> {
    let request_id = next_request_id(run.id(), state);
    let _ = write_json_line(
        &mut state.stdin,
        &json!({
            "id": request_id,
            "type": "abort",
        }),
    );
    state.active_turn_id = None;
    state.cancelled_turn_pending_settlement = true;
    Ok(())
}

pub(crate) fn drain_pi_events(
    run: &RuntimeProviderRun,
    state: &mut PiRuntimeState,
) -> Result<ProviderPromptSignalBatch, DaemonError> {
    let mut batch = ProviderPromptSignalBatch::default();
    for _ in 0..PI_EVENT_DRAIN_MAX_MESSAGES {
        match state.receiver.try_recv() {
            Ok(PiRuntimeMessage::Stdout(value)) => {
                apply_pi_message(run.id(), state, value, &mut batch)
            }
            Ok(PiRuntimeMessage::StdoutParseError(error)) => {
                batch
                    .notices
                    .push(format!("Pi stdout parse warning: {error}"));
            }
            Ok(PiRuntimeMessage::Stderr(line)) => {
                batch.notices.push(format!("Pi stderr: {line}"));
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => break,
        }
    }

    if !batch.prompt_completed && !state.exit_reported {
        match state.child.try_wait() {
            Ok(Some(status)) => {
                state.exit_reported = true;
                if !status.success() || state.active_turn_id.is_some() {
                    batch.terminal_failure = Some(format!(
                        "Pi exited before completing the active turn: {status}"
                    ));
                    batch.prompt_completed = state.active_turn_id.is_some();
                    state.active_turn_id = None;
                }
            }
            Ok(None) => {}
            Err(error) => {
                state.exit_reported = true;
                batch.terminal_failure = Some(format!("failed to poll Pi process: {error}"));
            }
        }
    }
    if !batch.prompt_completed && state.cancelled_turn_pending_settlement {
        state.cancelled_turn_pending_settlement = false;
        batch.prompt_completed = true;
    }

    Ok(batch)
}

fn pi_prompt_text(envelope: &PromptEnvelope) -> String {
    envelope.visible_user_prompt.clone()
}

fn apply_pi_message(
    provider_run_id: &str,
    state: &mut PiRuntimeState,
    value: Value,
    batch: &mut ProviderPromptSignalBatch,
) {
    let Some(kind) = value.get("type").and_then(Value::as_str) else {
        return;
    };
    match kind {
        "response" => apply_pi_response(provider_run_id, state, &value, batch),
        "agent_start" | "turn_start" => {
            if let Some(model) = value.pointer("/data/model/id").and_then(Value::as_str) {
                batch.resolved_model = Some(format_pi_model(model));
                batch.resolved_model_source = Some("pi.event");
            }
        }
        "message_update" => apply_pi_message_update(provider_run_id, state, &value, batch),
        "message_end" => apply_pi_message_end(provider_run_id, state, &value, batch),
        "turn_end" => apply_pi_turn_end(provider_run_id, state, &value, batch),
        "agent_end" => apply_pi_agent_end(state, &value, batch),
        "tool_execution_start" | "tool_execution_update" | "tool_execution_end" => {
            apply_pi_tool_event(provider_run_id, &value, batch)
        }
        "auto_retry_start"
        | "auto_retry_end"
        | "compaction_start"
        | "compaction_end"
        | "queue_update"
        | "extension_ui_request" => {
            batch.notices.push(format!("Pi event: {kind}"));
        }
        "extension_error" => {
            batch.terminal_failure = Some(
                value
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("Pi extension error")
                    .to_string(),
            );
        }
        _ => {}
    }
}

fn apply_pi_response(
    provider_run_id: &str,
    state: &mut PiRuntimeState,
    value: &Value,
    batch: &mut ProviderPromptSignalBatch,
) {
    let command = value
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if value.get("success").and_then(Value::as_bool) == Some(false) {
        let message = value
            .get("error")
            .and_then(Value::as_str)
            .or_else(|| value.pointer("/data/error").and_then(Value::as_str))
            .unwrap_or("Pi rejected the command")
            .to_string();
        push_error_chunk(provider_run_id, batch, &message);
        batch.terminal_failure = Some(message);
        if matches!(command, "prompt" | "steer" | "follow_up" | "abort") {
            batch.prompt_completed = state.active_turn_id.is_some();
            state.active_turn_id = None;
        }
    }
    if command == "get_state" {
        apply_pi_state_response(state, value, batch);
    }
    if let Some(session_id) = value
        .pointer("/data/sessionId")
        .or_else(|| value.pointer("/data/sessionFile"))
        .and_then(Value::as_str)
    {
        record_pi_session_id(state, batch, session_id.to_string());
    }
}

fn apply_pi_state_response(
    state: &mut PiRuntimeState,
    value: &Value,
    batch: &mut ProviderPromptSignalBatch,
) {
    if let Some(session_id) = value.pointer("/data/sessionId").and_then(Value::as_str) {
        record_pi_session_id(state, batch, session_id.to_string());
    }
    if let Some(model) = format_pi_state_model(value.pointer("/data/model")) {
        batch.resolved_model = Some(model);
        batch.resolved_model_source = Some("pi.get_state");
    }
}

fn format_pi_state_model(model: Option<&Value>) -> Option<String> {
    let model = model?;
    let id = model.get("id").and_then(Value::as_str)?;
    let provider = model.get("provider").and_then(Value::as_str);
    match provider {
        Some(provider) if !id.contains('/') => Some(format_pi_model(&format!("{provider}/{id}"))),
        _ => Some(format_pi_model(id)),
    }
}

fn apply_pi_message_update(
    provider_run_id: &str,
    state: &mut PiRuntimeState,
    value: &Value,
    batch: &mut ProviderPromptSignalBatch,
) {
    let event = value
        .get("assistantMessageEvent")
        .or_else(|| value.get("event"))
        .unwrap_or(value);
    let event_kind = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if let Some(model) = value
        .pointer("/message/model/id")
        .or_else(|| value.pointer("/message/modelId"))
        .and_then(Value::as_str)
    {
        batch.resolved_model = Some(format_pi_model(model));
        batch.resolved_model_source = Some("pi.message_update");
    }
    match event_kind {
        "text_delta" => {
            if let Some(text) = event
                .get("delta")
                .or_else(|| event.get("text"))
                .and_then(Value::as_str)
            {
                push_text_chunk(provider_run_id, batch, text);
                state.saw_text_delta = true;
            }
        }
        "thinking_delta" => {
            if let Some(text) = event
                .get("delta")
                .or_else(|| event.get("thinking"))
                .or_else(|| event.get("text"))
                .and_then(Value::as_str)
            {
                push_reasoning_chunk(provider_run_id, batch, text);
            }
        }
        "error" => {
            batch.terminal_failure = Some(
                event
                    .get("error")
                    .and_then(Value::as_str)
                    .or_else(|| event.get("message").and_then(Value::as_str))
                    .unwrap_or("Pi message stream error")
                    .to_string(),
            );
        }
        _ => {}
    }
}

fn apply_pi_message_end(
    provider_run_id: &str,
    state: &mut PiRuntimeState,
    value: &Value,
    batch: &mut ProviderPromptSignalBatch,
) {
    if state.saw_text_delta {
        return;
    }
    let message = value.get("message").unwrap_or(value);
    let message_id = message
        .get("id")
        .or_else(|| message.get("entryId"))
        .and_then(Value::as_str)
        .unwrap_or("assistant");
    emit_message_text_suffix(provider_run_id, state, batch, message_id, message);
}

fn apply_pi_turn_end(
    provider_run_id: &str,
    state: &mut PiRuntimeState,
    value: &Value,
    batch: &mut ProviderPromptSignalBatch,
) {
    if let Some(message) = value.get("message") {
        apply_pi_message_end(
            provider_run_id,
            state,
            &json!({ "message": message }),
            batch,
        );
    }
    if let Some(usage) = value
        .get("usage")
        .or_else(|| value.pointer("/message/usage"))
        .and_then(usage_from_value)
    {
        batch.resolved_usage_tokens_total = usage.total_tokens;
        batch.resolved_usage = Some(usage);
    }
    push_completion(state, batch);
}

fn apply_pi_agent_end(
    state: &mut PiRuntimeState,
    value: &Value,
    batch: &mut ProviderPromptSignalBatch,
) {
    if let Some(session_id) = value
        .get("sessionId")
        .or_else(|| value.get("sessionFile"))
        .and_then(Value::as_str)
    {
        record_pi_session_id(state, batch, session_id.to_string());
    }
    if !batch.prompt_completed && state.active_turn_id.is_some() {
        push_completion(state, batch);
    }
}

fn apply_pi_tool_event(
    provider_run_id: &str,
    value: &Value,
    batch: &mut ProviderPromptSignalBatch,
) {
    let tool_name = value
        .get("toolName")
        .or_else(|| value.get("tool"))
        .and_then(Value::as_str)
        .unwrap_or("tool");
    let status = match value.get("type").and_then(Value::as_str) {
        Some("tool_execution_start") => "started",
        Some("tool_execution_update") => "running",
        Some("tool_execution_end") => "completed",
        _ => "unknown",
    };
    let payload = json!({
        "tool": tool_name,
        "status": status,
        "id": value.get("toolCallId").cloned().unwrap_or(Value::Null),
        "input": value.get("args").cloned().unwrap_or(Value::Null),
        "output": value.get("result").or_else(|| value.get("partialResult")).cloned().unwrap_or(Value::Null),
        "is_error": value.get("isError").and_then(Value::as_bool).unwrap_or(false),
    });
    match serde_json::to_vec(&payload) {
        Ok(bytes) => batch.chunks.push(ProviderPromptChunk {
            kind: TerminalOutputKind::ProviderTool,
            merge_key: Some(format!(
                "pi:{provider_run_id}:tool:{}",
                value
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .unwrap_or(tool_name)
            )),
            bytes,
        }),
        Err(error) => batch
            .notices
            .push(format!("Pi tool event serialization warning: {error}")),
    }
}

fn emit_message_text_suffix(
    provider_run_id: &str,
    state: &mut PiRuntimeState,
    batch: &mut ProviderPromptSignalBatch,
    message_id: &str,
    message: &Value,
) {
    if let Some(text) = message.get("text").and_then(Value::as_str) {
        emit_text_suffix(provider_run_id, state, batch, message_id, "text", text);
    }
    if let Some(content) = message.get("content").and_then(Value::as_array) {
        for (index, block) in content.iter().enumerate() {
            let block_kind = block.get("type").and_then(Value::as_str).unwrap_or("text");
            if let Some(text) = block
                .get("text")
                .or_else(|| block.get("content"))
                .and_then(Value::as_str)
            {
                emit_text_suffix(
                    provider_run_id,
                    state,
                    batch,
                    &format!("{message_id}:{index}"),
                    block_kind,
                    text,
                );
            }
        }
    }
}

fn emit_text_suffix(
    provider_run_id: &str,
    state: &mut PiRuntimeState,
    batch: &mut ProviderPromptSignalBatch,
    key: &str,
    block_kind: &str,
    text: &str,
) {
    let offset = state
        .emitted_text_offsets
        .entry(key.to_string())
        .or_default();
    if *offset >= text.len() {
        return;
    }
    let suffix = &text[*offset..];
    *offset = text.len();
    match block_kind {
        "thinking" | "reasoning" => push_reasoning_chunk(provider_run_id, batch, suffix),
        _ => push_text_chunk(provider_run_id, batch, suffix),
    }
}

fn push_text_chunk(provider_run_id: &str, batch: &mut ProviderPromptSignalBatch, text: &str) {
    if text.is_empty() {
        return;
    }
    batch.chunks.push(ProviderPromptChunk {
        kind: TerminalOutputKind::ProviderOutput,
        merge_key: Some(format!("pi:{provider_run_id}:assistant")),
        bytes: text.as_bytes().to_vec(),
    });
}

fn push_reasoning_chunk(provider_run_id: &str, batch: &mut ProviderPromptSignalBatch, text: &str) {
    if text.is_empty() {
        return;
    }
    batch.chunks.push(ProviderPromptChunk {
        kind: TerminalOutputKind::ProviderReasoning,
        merge_key: Some(format!("pi:{provider_run_id}:reasoning")),
        bytes: text.as_bytes().to_vec(),
    });
}

fn push_error_chunk(provider_run_id: &str, batch: &mut ProviderPromptSignalBatch, text: &str) {
    if text.is_empty() {
        return;
    }
    batch.chunks.push(ProviderPromptChunk {
        kind: TerminalOutputKind::ProviderError,
        merge_key: Some(format!("pi:{provider_run_id}:error")),
        bytes: text.as_bytes().to_vec(),
    });
}

fn push_completion(state: &mut PiRuntimeState, batch: &mut ProviderPromptSignalBatch) {
    let message_id = state
        .session_id
        .as_ref()
        .map(|session_id| format!("pi:{session_id}:{}", state.result_number))
        .unwrap_or_else(|| format!("pi:result:{}", state.result_number));
    state.result_number += 1;
    batch.completions.push(ProviderAssistantCompletion {
        message_id,
        completed_at_ms: unix_epoch_ms(),
    });
    batch.prompt_completed = true;
    state.active_turn_id = None;
}

fn record_pi_session_id(
    state: &mut PiRuntimeState,
    batch: &mut ProviderPromptSignalBatch,
    session_id: String,
) {
    if state.session_id.as_deref() != Some(session_id.as_str()) {
        state.session_id = Some(session_id.clone());
    }
    batch.resolved_resume_state = Some(ProviderResumeState::from_pi_session_id(session_id));
}

fn format_pi_model(model: &str) -> String {
    let model = model.trim();
    if model.starts_with("pi/") {
        model.to_string()
    } else {
        format!("pi/{model}")
    }
}

fn usage_from_value(value: &Value) -> Option<ProviderRunTokenUsage> {
    let input = u64_field(value, "input_tokens")
        .or_else(|| u64_field(value, "input"))
        .unwrap_or_default();
    let output = u64_field(value, "output_tokens")
        .or_else(|| u64_field(value, "output"))
        .unwrap_or_default();
    let cache_read = u64_field(value, "cacheRead")
        .or_else(|| u64_field(value, "cache_read"))
        .unwrap_or_default();
    let cache_write = u64_field(value, "cacheWrite")
        .or_else(|| u64_field(value, "cache_write"))
        .unwrap_or_default();
    let total = u64_field(value, "total_tokens")
        .or_else(|| u64_field(value, "total"))
        .unwrap_or_else(|| input + output + cache_read + cache_write);
    (total > 0).then_some(ProviderRunTokenUsage {
        total_tokens: Some(total),
        last_tokens: Some(output),
        context_tokens: Some(input + cache_read + cache_write),
        context_window: u64_field(value, "contextWindow")
            .or_else(|| u64_field(value, "context_window")),
    })
}

fn u64_field(value: &Value, field: &str) -> Option<u64> {
    value.get(field).and_then(Value::as_u64)
}

fn next_request_id(run_id: &str, state: &mut PiRuntimeState) -> String {
    let id = format!("arroba-pi-{run_id}-{}", state.next_request_number);
    state.next_request_number += 1;
    id
}

fn request_pi_state(run_id: &str, state: &mut PiRuntimeState) -> Result<(), DaemonError> {
    let state_request_id = next_request_id(run_id, state);
    write_json_line(
        &mut state.stdin,
        &json!({
            "id": state_request_id,
            "type": "get_state",
        }),
    )
}

fn pi_runtime_selection_changed(run: &RuntimeProviderRun, state: &PiRuntimeState) -> bool {
    state.active_model != run.model()
        || state.active_variant.as_deref() != run.variant()
        || state.active_execution_mode != run.execution_mode()
        || state.active_permission_level != run.permission_level()
}

fn pi_runtime_child_exited(state: &mut PiRuntimeState) -> bool {
    matches!(state.child.try_wait(), Ok(Some(_)))
}

fn restart_pi_runtime(
    run: &RuntimeProviderRun,
    state: &mut PiRuntimeState,
    operation: &'static str,
) -> Result<(), DaemonError> {
    stop_child(&mut state.child);
    let (child, stdin, receiver) = spawn_pi_child(
        run.id(),
        &state.program,
        &state.args,
        &state.env,
        &state.env_remove,
        state.working_directory.as_ref(),
        operation,
    )?;
    state.child = child;
    state.stdin = stdin;
    state.receiver = receiver;
    state.active_model = run.model().to_string();
    state.active_variant = run.variant().map(str::to_string);
    state.active_execution_mode = run.execution_mode();
    state.active_permission_level = run.permission_level();
    state.active_turn_id = None;
    state.cancelled_turn_pending_settlement = false;
    state.saw_text_delta = false;
    state.exit_reported = false;
    state.emitted_text_offsets.clear();
    Ok(())
}

fn spawn_pi_child(
    provider_run_id: &str,
    program: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
    env_remove: &[String],
    working_directory: Option<&PathBuf>,
    operation: &'static str,
) -> Result<(Child, ChildStdin, Receiver<PiRuntimeMessage>), DaemonError> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in env_remove {
        command.env_remove(name);
    }
    for (name, value) in env {
        command.env(name, value);
    }
    if let Some(working_directory) = working_directory {
        command.current_dir(working_directory);
    }
    let mut child = command
        .spawn()
        .map_err(|error| DaemonError::LocalTransport {
            operation,
            message: format!("failed to start Pi for `{provider_run_id}`: {error}"),
        })?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation,
            message: "Pi did not expose stdin".to_string(),
        })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation,
            message: "Pi did not expose stdout".to_string(),
        })?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation,
            message: "Pi did not expose stderr".to_string(),
        })?;
    let (tx, receiver) = mpsc::channel();
    {
        let tx = tx.clone();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line {
                    Ok(line) if line.trim().is_empty() => {}
                    Ok(line) => match serde_json::from_str::<Value>(&line) {
                        Ok(value) => {
                            let _ = tx.send(PiRuntimeMessage::Stdout(value));
                        }
                        Err(error) => {
                            let _ = tx.send(PiRuntimeMessage::StdoutParseError(
                                pi_stdout_parse_error_notice(&error, &line),
                            ));
                        }
                    },
                    Err(error) => {
                        let _ = tx.send(PiRuntimeMessage::StdoutParseError(error.to_string()));
                        break;
                    }
                }
            }
        });
    }
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            match line {
                Ok(line) if line.trim().is_empty() => {}
                Ok(line) => {
                    let _ = tx.send(PiRuntimeMessage::Stderr(line));
                }
                Err(error) => {
                    let _ = tx.send(PiRuntimeMessage::Stderr(error.to_string()));
                    break;
                }
            }
        }
    });

    Ok((child, stdin, receiver))
}

fn write_json_line(stdin: &mut ChildStdin, value: &Value) -> Result<(), DaemonError> {
    serde_json::to_writer(&mut *stdin, value).map_err(|error| DaemonError::LocalTransport {
        operation: "pi_write_stdin",
        message: error.to_string(),
    })?;
    stdin
        .write_all(b"\n")
        .and_then(|_| stdin.flush())
        .map_err(|error| DaemonError::LocalTransport {
            operation: "pi_write_stdin",
            message: error.to_string(),
        })
}

fn pi_stdout_parse_error_notice(error: &serde_json::Error, line: &str) -> String {
    format!(
        "{error}; dropped malformed stdout line ({} bytes)",
        line.len()
    )
}

fn stop_child(child: &mut Child) {
    match child.try_wait() {
        Ok(Some(_)) => {}
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
        }
        Err(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::provider::pi_runtime::{
        apply_pi_message, pi_prompt_text, pi_stdout_parse_error_notice, PiRuntimeState,
    };

    #[test]
    fn pi_text_delta_maps_to_provider_output() {
        let mut state = test_state();
        let mut batch = crate::provider::ProviderPromptSignalBatch::default();

        apply_pi_message(
            "run-1",
            &mut state,
            json!({
                "type": "message_update",
                "assistantMessageEvent": { "type": "text_delta", "delta": "hello" }
            }),
            &mut batch,
        );

        assert_eq!(batch.chunks.len(), 1);
        assert_eq!(
            batch.chunks[0].kind,
            crate::terminal::TerminalOutputKind::ProviderOutput
        );
        assert_eq!(String::from_utf8_lossy(&batch.chunks[0].bytes), "hello");
    }

    #[test]
    fn pi_agent_end_completes_active_turn() {
        let mut state = test_state();
        state.active_turn_id = Some("turn-1".to_string());
        let mut batch = crate::provider::ProviderPromptSignalBatch::default();

        apply_pi_message(
            "run-1",
            &mut state,
            json!({ "type": "agent_end" }),
            &mut batch,
        );

        assert!(batch.prompt_completed);
    }

    #[test]
    fn pi_prompt_rejection_surfaces_error_output() {
        let mut state = test_state();
        state.active_turn_id = Some("turn-1".to_string());
        let mut batch = crate::provider::ProviderPromptSignalBatch::default();

        apply_pi_message(
            "run-1",
            &mut state,
            json!({
                "type": "response",
                "command": "prompt",
                "success": false,
                "error": "No API key found for openai."
            }),
            &mut batch,
        );

        assert!(batch.prompt_completed);
        assert_eq!(
            batch.terminal_failure.as_deref(),
            Some("No API key found for openai.")
        );
        assert_eq!(batch.chunks.len(), 1);
        assert_eq!(
            batch.chunks[0].kind,
            crate::terminal::TerminalOutputKind::ProviderError
        );
        assert_eq!(
            String::from_utf8_lossy(&batch.chunks[0].bytes),
            "No API key found for openai."
        );
    }

    #[test]
    fn pi_get_state_response_records_resume_state_and_model() {
        let mut state = test_state();
        let mut batch = crate::provider::ProviderPromptSignalBatch::default();

        apply_pi_message(
            "run-1",
            &mut state,
            json!({
                "type": "response",
                "command": "get_state",
                "success": true,
                "data": {
                    "sessionId": "session-1",
                    "model": {
                        "provider": "anthropic",
                        "id": "claude-sonnet-4-6"
                    }
                }
            }),
            &mut batch,
        );

        assert_eq!(
            batch
                .resolved_resume_state
                .as_ref()
                .and_then(|state| state.pi_session_id()),
            Some("session-1")
        );
        assert_eq!(
            batch.resolved_model.as_deref(),
            Some("pi/anthropic/claude-sonnet-4-6")
        );
        assert_eq!(batch.resolved_model_source.as_deref(), Some("pi.get_state"));
    }

    #[test]
    fn pi_prompt_text_omits_hidden_context() {
        let envelope = crate::prompt_assembly::PromptEnvelope::new(
            "visible",
            "hidden",
            Vec::new(),
            crate::prompt_assembly::PromptManifest::default(),
        );

        assert_eq!(pi_prompt_text(&envelope), "visible");
    }

    #[test]
    fn pi_parse_warning_does_not_echo_raw_provider_stdout() {
        let raw = "not-json secret-token prompt text";
        let error = serde_json::from_str::<serde_json::Value>(raw).expect_err("fixture invalid");

        let notice = pi_stdout_parse_error_notice(&error, raw);

        assert!(notice.contains("dropped malformed stdout line"));
        assert!(notice.contains(&format!("{} bytes", raw.len())));
        assert!(!notice.contains("secret-token"));
        assert!(!notice.contains("prompt text"));
    }

    fn test_state() -> PiRuntimeState {
        let mut child = std::process::Command::new("sh")
            .arg("-c")
            .arg("sleep 60")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("fixture child should spawn");
        let stdin = child.stdin.take().expect("stdin should exist");
        let (_tx, receiver) = std::sync::mpsc::channel();
        PiRuntimeState {
            program: "sh".to_string(),
            args: Vec::new(),
            env: Default::default(),
            env_remove: Vec::new(),
            working_directory: None,
            child,
            stdin,
            receiver,
            active_model: "openai/gpt-5.4".to_string(),
            active_variant: None,
            active_execution_mode: crate::provider::AgentExecutionMode::Build,
            active_permission_level: crate::provider::AgentPermissionLevel::Required,
            session_id: None,
            active_turn_id: None,
            cancelled_turn_pending_settlement: false,
            next_request_number: 1,
            result_number: 1,
            emitted_text_offsets: Default::default(),
            saw_text_delta: false,
            exit_reported: false,
        }
    }
}
