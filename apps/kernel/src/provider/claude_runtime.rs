use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Child, ChildStdin};
use std::sync::mpsc::{Receiver, TryRecvError};

use serde_json::{json, Value};

use crate::error::DaemonError;
use crate::session::{unix_epoch_ms, PromptAttachment};
use crate::terminal::TerminalOutputKind;

use super::{
    claude::claude_launch_args_for_run, AgentExecutionMode, AgentPermissionLevel,
    ProviderAssistantCompletion, ProviderPromptChunk, ProviderPromptSignalBatch,
    ProviderResumeState, ProviderRunTokenUsage, RuntimeProviderRun,
};

const CLAUDE_EVENT_DRAIN_MAX_MESSAGES: usize = 256;

mod input;
mod process;

use input::claude_user_content;
use process::{spawn_claude_child, stop_child, write_json_line, ClaudeRuntimeMessage};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClaudeRunSelection {
    pub model: Option<String>,
    pub variant: Option<String>,
}

pub(crate) struct ClaudeRuntimeBinding {
    pub state: ClaudeRuntimeState,
    pub selection: ClaudeRunSelection,
}

pub struct ClaudeRuntimeState {
    program: String,
    env: BTreeMap<String, String>,
    env_remove: Vec<String>,
    working_directory: Option<PathBuf>,
    child: Child,
    stdin: ChildStdin,
    receiver: Receiver<ClaudeRuntimeMessage>,
    active_model: String,
    active_variant: Option<String>,
    active_execution_mode: AgentExecutionMode,
    active_permission_level: AgentPermissionLevel,
    session_id: Option<String>,
    active_turn_id: Option<String>,
    cancelled_turn_pending_settlement: bool,
    next_turn_number: u64,
    result_number: u64,
    emitted_text_offsets: BTreeMap<String, usize>,
    saw_text_delta: bool,
    exit_reported: bool,
}

impl std::fmt::Debug for ClaudeRuntimeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeRuntimeState")
            .field("program", &self.program)
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
            .field("next_turn_number", &self.next_turn_number)
            .field("result_number", &self.result_number)
            .field("emitted_text_offsets", &self.emitted_text_offsets)
            .field("saw_text_delta", &self.saw_text_delta)
            .field("exit_reported", &self.exit_reported)
            .finish()
    }
}

impl Drop for ClaudeRuntimeState {
    fn drop(&mut self) {
        stop_child(&mut self.child);
    }
}

pub(crate) fn initialize_claude_runtime(
    run: &RuntimeProviderRun,
) -> Result<ClaudeRuntimeBinding, DaemonError> {
    let program = run
        .pty_program()
        .ok_or_else(|| DaemonError::ProviderProtocol {
            provider_run_id: run.id().to_string(),
            operation: "claude_executable_missing",
            message: "Claude provider run did not include an executable".to_string(),
        })?
        .to_string();
    let args = run.pty_args().to_vec();
    let env = run.pty_env().clone();
    let env_remove = run.pty_env_remove().to_vec();
    let working_directory = run.working_directory().cloned();
    let (child, stdin, receiver) = spawn_claude_child(
        run.id(),
        &program,
        &args,
        &env,
        &env_remove,
        working_directory.as_ref(),
        "initialize_claude_runtime",
    )?;

    Ok(ClaudeRuntimeBinding {
        state: ClaudeRuntimeState {
            program,
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
            session_id: run.resume_state().claude_session_id().map(str::to_string),
            active_turn_id: None,
            cancelled_turn_pending_settlement: false,
            next_turn_number: 1,
            result_number: 1,
            emitted_text_offsets: BTreeMap::new(),
            saw_text_delta: false,
            exit_reported: false,
        },
        selection: ClaudeRunSelection {
            model: Some(format!("claude/{}", run.model())),
            variant: run.variant().map(str::to_string),
        },
    })
}

pub(crate) fn submit_claude_prompt(
    run: &RuntimeProviderRun,
    state: &mut ClaudeRuntimeState,
    prompt: &str,
    attachments: &[PromptAttachment],
) -> Result<(), DaemonError> {
    if claude_runtime_selection_changed(run, state) {
        restart_claude_runtime(run, state, "claude_restart_for_selection_change")?;
    }
    let turn_id = format!("turn-{}", state.next_turn_number);
    state.next_turn_number += 1;
    state.active_turn_id = Some(turn_id);
    state.saw_text_delta = false;
    state.emitted_text_offsets.clear();
    let message = json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": claude_user_content(prompt, attachments)
        }
    });
    write_json_line(&mut state.stdin, &message)
}

pub(crate) fn abort_claude_turn(
    run: &RuntimeProviderRun,
    state: &mut ClaudeRuntimeState,
) -> Result<(), DaemonError> {
    let message = json!({
        "type": "control_request",
        "request_id": format!("arroba-claude-interrupt-{}", run.id()),
        "request": { "subtype": "interrupt" }
    });
    let _ = write_json_line(&mut state.stdin, &message);
    state.active_turn_id = None;
    state.cancelled_turn_pending_settlement = true;
    restart_claude_runtime(run, state, "claude_restart_after_abort")
}

pub(crate) fn drain_claude_events(
    run: &RuntimeProviderRun,
    state: &mut ClaudeRuntimeState,
) -> Result<ProviderPromptSignalBatch, DaemonError> {
    let mut batch = ProviderPromptSignalBatch::default();
    for _ in 0..CLAUDE_EVENT_DRAIN_MAX_MESSAGES {
        match state.receiver.try_recv() {
            Ok(ClaudeRuntimeMessage::Stdout(value)) => {
                apply_claude_message(run.id(), state, value, &mut batch);
            }
            Ok(ClaudeRuntimeMessage::StdoutParseError(error)) => {
                batch
                    .notices
                    .push(format!("Claude stdout parse warning: {error}"));
            }
            Ok(ClaudeRuntimeMessage::Stderr(line)) => {
                batch.notices.push(format!("Claude stderr: {line}"));
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
                        "Claude Code exited before completing the active turn: {status}"
                    ));
                    batch.prompt_completed = state.active_turn_id.is_some();
                    state.active_turn_id = None;
                }
            }
            Ok(None) => {}
            Err(error) => {
                state.exit_reported = true;
                batch.terminal_failure =
                    Some(format!("failed to poll Claude Code process: {error}"));
            }
        }
    }
    if !batch.prompt_completed && state.cancelled_turn_pending_settlement {
        state.cancelled_turn_pending_settlement = false;
        batch.prompt_completed = true;
    }

    Ok(batch)
}

fn claude_runtime_selection_changed(run: &RuntimeProviderRun, state: &ClaudeRuntimeState) -> bool {
    state.active_model != run.model()
        || state.active_variant.as_deref() != run.variant()
        || state.active_execution_mode != run.execution_mode()
        || state.active_permission_level != run.permission_level()
}

fn restart_claude_runtime(
    run: &RuntimeProviderRun,
    state: &mut ClaudeRuntimeState,
    operation: &'static str,
) -> Result<(), DaemonError> {
    stop_child(&mut state.child);
    let resume_session_id = state
        .session_id
        .as_deref()
        .or_else(|| run.resume_state().claude_session_id());
    let args = claude_launch_args_for_run(run, resume_session_id)?;
    let (child, stdin, receiver) = spawn_claude_child(
        run.id(),
        &state.program,
        &args,
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
    state.emitted_text_offsets.clear();
    state.saw_text_delta = false;
    state.exit_reported = false;
    Ok(())
}

fn apply_claude_message(
    provider_run_id: &str,
    state: &mut ClaudeRuntimeState,
    value: Value,
    batch: &mut ProviderPromptSignalBatch,
) {
    let Some(kind) = value.get("type").and_then(Value::as_str) else {
        return;
    };
    match kind {
        "system" => apply_system_message(state, &value, batch),
        "stream_event" => apply_stream_event(provider_run_id, state, &value, batch),
        "assistant" => apply_assistant_message(provider_run_id, state, &value, batch),
        "result" => apply_result_message(state, &value, batch),
        _ => {}
    }
}

fn apply_system_message(
    state: &mut ClaudeRuntimeState,
    value: &Value,
    batch: &mut ProviderPromptSignalBatch,
) {
    if let Some(session_id) =
        string_field(value, "session_id").or_else(|| string_field(value, "sessionId"))
    {
        record_claude_session_id(state, batch, session_id);
    }
    if batch.resolved_model.is_none() {
        if let Some(model) = string_field(value, "model") {
            batch.resolved_model = Some(format_claude_model(&model));
            batch.resolved_model_source = Some("claude.system");
        }
    }
}

fn apply_stream_event(
    provider_run_id: &str,
    state: &mut ClaudeRuntimeState,
    value: &Value,
    batch: &mut ProviderPromptSignalBatch,
) {
    let event = value.get("event").unwrap_or(value);
    let event_kind = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if event_kind == "message_start" {
        if let Some(model) = event
            .get("message")
            .and_then(|message| message.get("model"))
            .and_then(Value::as_str)
        {
            batch.resolved_model = Some(format_claude_model(model));
            batch.resolved_model_source = Some("claude.stream_event");
        }
    }
    if event_kind == "content_block_start" {
        if let Some(text) = event
            .get("content_block")
            .and_then(|block| block.get("text"))
            .and_then(Value::as_str)
        {
            push_text_chunk(provider_run_id, batch, text);
            state.saw_text_delta = true;
        }
    }
    if event_kind == "content_block_delta" {
        let Some(delta) = event.get("delta") else {
            return;
        };
        match delta
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "text_delta" => {
                if let Some(text) = delta.get("text").and_then(Value::as_str) {
                    push_text_chunk(provider_run_id, batch, text);
                    state.saw_text_delta = true;
                }
            }
            "thinking_delta" => {
                if let Some(text) = delta
                    .get("thinking")
                    .or_else(|| delta.get("text"))
                    .and_then(Value::as_str)
                {
                    push_reasoning_chunk(provider_run_id, batch, text);
                }
            }
            _ => {}
        }
    }
    if event_kind == "message_delta" {
        if let Some(usage) = event
            .get("usage")
            .or_else(|| {
                event
                    .get("message")
                    .and_then(|message| message.get("usage"))
            })
            .and_then(usage_from_value)
        {
            batch.resolved_usage_tokens_total = usage.total_tokens;
            batch.resolved_usage = Some(usage);
        }
    }
}

fn apply_assistant_message(
    provider_run_id: &str,
    state: &mut ClaudeRuntimeState,
    value: &Value,
    batch: &mut ProviderPromptSignalBatch,
) {
    let message = value.get("message").unwrap_or(value);
    if batch.resolved_model.is_none() {
        if let Some(model) = message.get("model").and_then(Value::as_str) {
            batch.resolved_model = Some(format_claude_model(model));
            batch.resolved_model_source = Some("claude.assistant");
        }
    }
    if let Some(usage) = message.get("usage").and_then(usage_from_value) {
        batch.resolved_usage_tokens_total = usage.total_tokens;
        batch.resolved_usage = Some(usage);
    }
    if state.saw_text_delta {
        return;
    }
    let message_id = message
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("assistant");
    if let Some(content) = message.get("content").and_then(Value::as_array) {
        for (index, block) in content.iter().enumerate() {
            let block_kind = block
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let Some(text) = block.get("text").and_then(Value::as_str) else {
                continue;
            };
            let key = format!("{message_id}:{index}");
            emit_text_suffix(provider_run_id, state, batch, &key, block_kind, text);
        }
    }
}

fn apply_result_message(
    state: &mut ClaudeRuntimeState,
    value: &Value,
    batch: &mut ProviderPromptSignalBatch,
) {
    if let Some(session_id) = string_field(value, "session_id") {
        record_claude_session_id(state, batch, session_id);
    }
    if let Some(usage) = value.get("usage").and_then(usage_from_value) {
        batch.resolved_usage_tokens_total = usage.total_tokens;
        batch.resolved_usage = Some(usage);
    }
    let subtype = value
        .get("subtype")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let is_error = value
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(subtype != "success");
    if is_error {
        batch.terminal_failure = Some(
            value
                .get("error")
                .and_then(Value::as_str)
                .or_else(|| value.get("result").and_then(Value::as_str))
                .unwrap_or("Claude Code reported an error")
                .to_string(),
        );
    }
    let message_id = state
        .session_id
        .as_ref()
        .map(|session_id| format!("claude:{session_id}:{}", state.result_number))
        .unwrap_or_else(|| format!("claude:result:{}", state.result_number));
    state.result_number += 1;
    batch.completions.push(ProviderAssistantCompletion {
        message_id,
        completed_at_ms: unix_epoch_ms(),
    });
    batch.prompt_completed = true;
    state.active_turn_id = None;
}

fn record_claude_session_id(
    state: &mut ClaudeRuntimeState,
    batch: &mut ProviderPromptSignalBatch,
    session_id: String,
) {
    if state.session_id.as_deref() != Some(session_id.as_str()) {
        state.session_id = Some(session_id.clone());
    }
    batch.resolved_resume_state = Some(ProviderResumeState::from_claude_session_id(session_id));
}

fn emit_text_suffix(
    provider_run_id: &str,
    state: &mut ClaudeRuntimeState,
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
        "thinking" => push_reasoning_chunk(provider_run_id, batch, suffix),
        _ => push_text_chunk(provider_run_id, batch, suffix),
    }
}

fn push_text_chunk(provider_run_id: &str, batch: &mut ProviderPromptSignalBatch, text: &str) {
    if text.is_empty() {
        return;
    }
    batch.chunks.push(ProviderPromptChunk {
        kind: TerminalOutputKind::ProviderOutput,
        merge_key: Some(format!("claude:{provider_run_id}:assistant")),
        bytes: text.as_bytes().to_vec(),
    });
}

fn push_reasoning_chunk(provider_run_id: &str, batch: &mut ProviderPromptSignalBatch, text: &str) {
    if text.is_empty() {
        return;
    }
    batch.chunks.push(ProviderPromptChunk {
        kind: TerminalOutputKind::ProviderReasoning,
        merge_key: Some(format!("claude:{provider_run_id}:reasoning")),
        bytes: text.as_bytes().to_vec(),
    });
}

fn string_field(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn format_claude_model(model: &str) -> String {
    let model = model.trim();
    if model.starts_with("claude/") {
        model.to_string()
    } else {
        format!("claude/{model}")
    }
}

fn usage_from_value(value: &Value) -> Option<ProviderRunTokenUsage> {
    let input = u64_field(value, "input_tokens")
        .or_else(|| u64_field(value, "input"))
        .unwrap_or_default();
    let output = u64_field(value, "output_tokens")
        .or_else(|| u64_field(value, "output"))
        .unwrap_or_default();
    let cache_create = u64_field(value, "cache_creation_input_tokens").unwrap_or_default();
    let cache_read = u64_field(value, "cache_read_input_tokens").unwrap_or_default();
    let total = u64_field(value, "total_tokens")
        .unwrap_or_else(|| input + output + cache_create + cache_read);
    (total > 0).then_some(ProviderRunTokenUsage {
        total_tokens: Some(total),
        last_tokens: Some(output),
        context_tokens: Some(input + cache_create + cache_read),
        context_window: None,
    })
}

fn u64_field(value: &Value, field: &str) -> Option<u64> {
    value.get(field).and_then(Value::as_u64)
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use serde_json::json;

    use crate::provider::{AgentExecutionMode, AgentPermissionLevel};
    use crate::session::PromptAttachment;
    use crate::terminal::TerminalOutputKind;

    use super::{
        apply_claude_message, input::claude_user_content, ClaudeRuntimeState,
        ProviderPromptSignalBatch,
    };

    fn parser_state() -> (ClaudeRuntimeState, ProviderPromptSignalBatch) {
        let mut child = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg("cat >/dev/null")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("fixture child should spawn");
        let stdin = child.stdin.take().expect("fixture stdin should exist");
        let (_tx, receiver) = std::sync::mpsc::channel();
        (
            ClaudeRuntimeState {
                program: "/bin/sh".to_string(),
                env: Default::default(),
                env_remove: Vec::new(),
                working_directory: None,
                child,
                stdin,
                receiver,
                active_model: "sonnet".to_string(),
                active_variant: Some("low".to_string()),
                active_execution_mode: AgentExecutionMode::Build,
                active_permission_level: AgentPermissionLevel::Yolo,
                session_id: None,
                active_turn_id: Some("turn-1".to_string()),
                cancelled_turn_pending_settlement: false,
                next_turn_number: 1,
                result_number: 1,
                emitted_text_offsets: Default::default(),
                saw_text_delta: false,
                exit_reported: false,
            },
            ProviderPromptSignalBatch::default(),
        )
    }

    #[test]
    fn captures_system_session_and_model() {
        let (mut state, mut batch) = parser_state();

        apply_claude_message(
            "run-1",
            &mut state,
            json!({
                "type": "system",
                "subtype": "init",
                "session_id": "claude-session-1",
                "model": "claude-sonnet-4-6"
            }),
            &mut batch,
        );

        assert_eq!(state.session_id.as_deref(), Some("claude-session-1"));
        assert_eq!(
            batch.resolved_model.as_deref(),
            Some("claude/claude-sonnet-4-6")
        );
        assert_eq!(batch.resolved_model_source, Some("claude.system"));
    }

    #[test]
    fn parses_stream_text_delta() {
        let (mut state, mut batch) = parser_state();

        apply_claude_message(
            "run-1",
            &mut state,
            json!({
                "type": "stream_event",
                "event": {
                    "type": "content_block_delta",
                    "delta": { "type": "text_delta", "text": "hello" }
                }
            }),
            &mut batch,
        );

        assert_eq!(batch.chunks.len(), 1);
        assert_eq!(batch.chunks[0].kind, TerminalOutputKind::ProviderOutput);
        assert_eq!(batch.chunks[0].bytes, b"hello");
    }

    #[test]
    fn marks_result_completion_and_usage() {
        let (mut state, mut batch) = parser_state();

        apply_claude_message(
            "run-1",
            &mut state,
            json!({
                "type": "result",
                "subtype": "success",
                "is_error": false,
                "session_id": "claude-session-1",
                "usage": {
                    "input_tokens": 3,
                    "output_tokens": 5,
                    "cache_creation_input_tokens": 2
                }
            }),
            &mut batch,
        );

        assert!(batch.prompt_completed);
        assert_eq!(batch.completions.len(), 1);
        assert_eq!(batch.resolved_usage_tokens_total, Some(10));
        assert_eq!(state.active_turn_id, None);
    }

    #[test]
    fn user_content_includes_text_attachment_contents() {
        let attachment = PromptAttachment::new(
            "artifact://note",
            "text/plain",
            Some("note.txt".to_string()),
        )
        .with_contents_base64(base64::engine::general_purpose::STANDARD.encode("attached marker"));

        let content = claude_user_content("read this", &[attachment]);

        assert_eq!(content[0]["text"], "read this");
        assert!(content[1]["text"]
            .as_str()
            .expect("attachment should render as text")
            .contains("attached marker"));
    }

    #[test]
    fn user_content_falls_back_to_attachment_reference_for_opaque_data() {
        let attachment = PromptAttachment::new(
            "artifact://archive",
            "application/octet-stream",
            Some("archive.bin".to_string()),
        )
        .with_contents_base64(base64::engine::general_purpose::STANDARD.encode([0, 1, 2]));

        let content = claude_user_content("", &[attachment]);

        assert_eq!(content.len(), 1);
        assert!(content[0]["text"]
            .as_str()
            .expect("opaque attachment should render as reference")
            .contains("archive.bin"));
    }
}
