use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::TryRecvError;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::error::DaemonError;
use crate::prompt_assembly::PromptEnvelope;
use crate::terminal::TerminalOutputKind;

use super::{
    AgentExecutionMode, AgentPermissionLevel, ProviderPromptSignalBatch, RuntimeProviderRun,
};

const CLAUDE_EVENT_DRAIN_MAX_MESSAGES: usize = 256;
const DEFAULT_CLAUDE_TURN_STALL_TIMEOUT: Duration = Duration::from_secs(60);

mod events;
mod input;
mod process;
mod state;
mod watchdog;

use events::apply_claude_message;
use input::claude_user_content;
use process::{spawn_claude_child, stop_child, write_json_line, ClaudeRuntimeMessage};
pub(crate) use state::{ClaudeRunSelection, ClaudeRuntimeBinding, ClaudeRuntimeState};
use watchdog::ClaudeTurnStallAction;

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
    let context_file = env.get("ARROBA_CLAUDE_NATIVE_CONTEXT").map(PathBuf::from);
    let settings_file = env.get("ARROBA_CLAUDE_SETTINGS_FILE").map(PathBuf::from);
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
            args,
            env,
            env_remove,
            working_directory,
            context_file,
            settings_file,
            child,
            stdin,
            receiver,
            active_model: run.model().to_string(),
            active_variant: run.variant().map(str::to_string),
            active_execution_mode: run.execution_mode(),
            active_permission_level: run.permission_level(),
            session_id: run.resume_state().claude_session_id().map(str::to_string),
            active_turn_id: None,
            active_prompt_message: None,
            turn_watchdog: Default::default(),
            cancelled_turn_pending_settlement: false,
            next_turn_number: 1,
            result_number: 1,
            emitted_text_offsets: BTreeMap::new(),
            saw_text_delta: false,
            exit_reported: false,
        },
        selection: ClaudeRunSelection {
            model: Some(run.model().to_string()),
            variant: run.variant().map(str::to_string),
        },
    })
}

pub(crate) fn submit_claude_prompt(
    run: &RuntimeProviderRun,
    state: &mut ClaudeRuntimeState,
    envelope: &PromptEnvelope,
) -> Result<(), DaemonError> {
    if claude_runtime_selection_changed(run, state) || claude_runtime_child_exited(state) {
        restart_claude_runtime(run, state, "claude_restart_for_selection_change")?;
    }
    write_claude_hidden_context(run.id(), state, &envelope.hidden_system_context)?;
    let turn_id = format!("turn-{}", state.next_turn_number);
    state.next_turn_number += 1;
    let message = json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": claude_user_content(&envelope.visible_user_prompt, &envelope.attachments)
        }
    });
    write_json_line(&mut state.stdin, &message)?;
    state.active_turn_id = Some(turn_id);
    state.active_prompt_message = Some(message);
    state.turn_watchdog.begin(Instant::now());
    state.saw_text_delta = false;
    state.emitted_text_offsets.clear();
    Ok(())
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
    clear_active_claude_turn(state);
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
                state.turn_watchdog.record_runtime_message(Instant::now());
                handle_claude_tool_uses(run.id(), state, &value, &mut batch)?;
                apply_claude_message(run.id(), state, value, &mut batch);
            }
            Ok(ClaudeRuntimeMessage::StdoutParseError(error)) => {
                state.turn_watchdog.record_runtime_message(Instant::now());
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
                    clear_active_claude_turn(state);
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
    if batch.prompt_completed {
        clear_active_claude_turn(state);
    } else {
        apply_claude_turn_stall_policy(run, state, &mut batch)?;
    }

    Ok(batch)
}

fn apply_claude_turn_stall_policy(
    run: &RuntimeProviderRun,
    state: &mut ClaudeRuntimeState,
    batch: &mut ProviderPromptSignalBatch,
) -> Result<(), DaemonError> {
    match state
        .turn_watchdog
        .action(Instant::now(), claude_turn_stall_timeout())
    {
        ClaudeTurnStallAction::Wait => Ok(()),
        ClaudeTurnStallAction::Restart => retry_stalled_claude_turn(run, state, batch),
        ClaudeTurnStallAction::Fail => {
            stop_child(&mut state.child);
            clear_active_claude_turn(state);
            batch.terminal_failure = Some(
                "Claude Code stopped emitting runtime events; the active turn was ended after its bounded recovery attempt"
                    .to_string(),
            );
            batch.prompt_completed = true;
            Ok(())
        }
    }
}

fn retry_stalled_claude_turn(
    run: &RuntimeProviderRun,
    state: &mut ClaudeRuntimeState,
    batch: &mut ProviderPromptSignalBatch,
) -> Result<(), DaemonError> {
    let message =
        state
            .active_prompt_message
            .clone()
            .ok_or_else(|| DaemonError::ProviderProtocol {
                provider_run_id: run.id().to_string(),
                operation: "claude_stalled_turn_retry",
                message: "active Claude turn did not retain its prompt message".to_string(),
            })?;
    restart_claude_runtime(run, state, "claude_restart_after_unacknowledged_turn_stall")?;
    write_json_line(&mut state.stdin, &message)?;
    let turn_id = format!("turn-{}", state.next_turn_number);
    state.next_turn_number += 1;
    state.active_turn_id = Some(turn_id);
    state.active_prompt_message = Some(message);
    state.turn_watchdog.record_restart(Instant::now());
    batch.notices.push(
        "Claude Code emitted no runtime events; restarted it and retried the unacknowledged turn once"
            .to_string(),
    );
    Ok(())
}

fn clear_active_claude_turn(state: &mut ClaudeRuntimeState) {
    state.active_turn_id = None;
    state.active_prompt_message = None;
    state.turn_watchdog.settle();
}

fn claude_turn_stall_timeout() -> Duration {
    std::env::var("ARROBA_CLAUDE_TURN_STALL_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_CLAUDE_TURN_STALL_TIMEOUT)
}

fn handle_claude_tool_uses(
    provider_run_id: &str,
    state: &mut ClaudeRuntimeState,
    value: &serde_json::Value,
    batch: &mut ProviderPromptSignalBatch,
) -> Result<(), DaemonError> {
    let message = value.get("message").unwrap_or(value);
    let Some(content) = message.get("content").and_then(serde_json::Value::as_array) else {
        return Ok(());
    };
    let mut tool_results = Vec::new();
    for block in content
        .iter()
        .filter(|block| block.get("type").and_then(serde_json::Value::as_str) == Some("tool_use"))
    {
        let name = block
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        if is_unsupported_claude_stream_json_tool(name) {
            let Some(id) = block.get("id").and_then(serde_json::Value::as_str) else {
                continue;
            };
            tool_results.push(json!({
                "type": "tool_result",
                "tool_use_id": id,
                "is_error": true,
                "content": format!(
                    "Arroba does not execute Claude stream-json tool `{name}` in this runtime path. If this is an Arroba workflow turn, do not search for workflow tools; emit the required fenced JSON fallback directly."
                ),
            }));
            continue;
        }
        let payload = json!({
            "tool": name,
            "status": "completed",
            "input": block.get("input").cloned().unwrap_or(serde_json::Value::Null),
            "id": block.get("id").cloned().unwrap_or(serde_json::Value::Null),
        });
        let bytes =
            serde_json::to_vec(&payload).map_err(|error| DaemonError::ProviderProtocol {
                provider_run_id: provider_run_id.to_string(),
                operation: "claude_tool_use_serialize",
                message: error.to_string(),
            })?;
        batch.chunks.push(super::ProviderPromptChunk {
            kind: TerminalOutputKind::ProviderTool,
            merge_key: None,
            bytes,
        });
    }
    if !tool_results.is_empty() {
        let response = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": tool_results,
            }
        });
        write_json_line(&mut state.stdin, &response)?;
        batch.notices.push(format!(
            "Rejected unsupported Claude stream-json tool use for `{provider_run_id}`"
        ));
    }
    Ok(())
}

fn is_unsupported_claude_stream_json_tool(name: &str) -> bool {
    name == "ToolSearch"
}

fn claude_runtime_selection_changed(run: &RuntimeProviderRun, state: &ClaudeRuntimeState) -> bool {
    state.active_model != run.model()
        || state.active_variant.as_deref() != run.variant()
        || state.active_execution_mode != run.execution_mode()
        || state.active_permission_level != run.permission_level()
}

fn claude_runtime_child_exited(state: &mut ClaudeRuntimeState) -> bool {
    if state.active_turn_id.is_some() {
        return false;
    }
    match state.child.try_wait() {
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(_) => true,
    }
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
    let base_args = claude_args_without_resume(&state.args);
    let mut args = base_args.clone();
    if let Some(session_id) = resume_session_id {
        args.extend(["--resume".to_string(), session_id.to_string()]);
    }
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
    state.args = base_args;
    state.active_model = run.model().to_string();
    state.active_variant = run.variant().map(str::to_string);
    state.active_execution_mode = run.execution_mode();
    state.active_permission_level = run.permission_level();
    state.active_turn_id = None;
    state.active_prompt_message = None;
    state.turn_watchdog.settle();
    state.emitted_text_offsets.clear();
    state.saw_text_delta = false;
    state.exit_reported = false;
    Ok(())
}

fn claude_args_without_resume(args: &[String]) -> Vec<String> {
    let mut sanitized = Vec::with_capacity(args.len());
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == "--resume" {
            skip_next = true;
            continue;
        }
        sanitized.push(arg.clone());
    }
    sanitized
}

fn write_claude_hidden_context(
    provider_run_id: &str,
    state: &ClaudeRuntimeState,
    hidden_system_context: &str,
) -> Result<(), DaemonError> {
    let Some(path) = &state.context_file else {
        return Ok(());
    };
    std::fs::write(path, hidden_system_context.trim()).map_err(|error| {
        DaemonError::ProviderProtocol {
            provider_run_id: provider_run_id.to_string(),
            operation: "claude_hidden_context_write",
            message: error.to_string(),
        }
    })
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use serde_json::json;

    use crate::provider::{AgentExecutionMode, AgentPermissionLevel};
    use crate::session::PromptAttachment;
    use crate::terminal::TerminalOutputKind;

    use super::{
        claude_args_without_resume, events::apply_claude_message, handle_claude_tool_uses,
        input::claude_user_content, ClaudeRuntimeState, ProviderPromptSignalBatch,
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
                args: vec!["-c".to_string(), "cat >/dev/null".to_string()],
                env: Default::default(),
                env_remove: Vec::new(),
                working_directory: None,
                context_file: None,
                settings_file: None,
                child,
                stdin,
                receiver,
                active_model: "sonnet".to_string(),
                active_variant: Some("low".to_string()),
                active_execution_mode: AgentExecutionMode::Build,
                active_permission_level: AgentPermissionLevel::Yolo,
                session_id: None,
                active_turn_id: Some("turn-1".to_string()),
                active_prompt_message: None,
                turn_watchdog: Default::default(),
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
    fn claude_args_without_resume_removes_stale_session_argument() {
        let args = vec![
            "--model".to_string(),
            "sonnet".to_string(),
            "--resume".to_string(),
            "stale-session".to_string(),
            "--mcp-config".to_string(),
            "/tmp/mcp.json".to_string(),
        ];

        assert_eq!(
            claude_args_without_resume(&args),
            vec![
                "--model".to_string(),
                "sonnet".to_string(),
                "--mcp-config".to_string(),
                "/tmp/mcp.json".to_string(),
            ]
        );
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
    fn rejects_unsupported_claude_tool_use_without_completing_turn() {
        let (mut state, mut batch) = parser_state();

        handle_claude_tool_uses(
            "run-1",
            &mut state,
            &json!({
                "type": "assistant",
                "message": {
                    "content": [{
                        "type": "tool_use",
                        "id": "toolu_1",
                        "name": "ToolSearch",
                        "input": { "query": "arroba workflow tools" }
                    }]
                }
            }),
            &mut batch,
        )
        .expect("tool-use rejection should write a tool result");

        assert!(!batch.prompt_completed);
        assert!(batch
            .notices
            .iter()
            .any(|notice| notice.contains("Rejected unsupported Claude stream-json tool use")));
    }

    #[test]
    fn records_provider_native_claude_tool_use_without_rejecting_it() {
        let (mut state, mut batch) = parser_state();

        handle_claude_tool_uses(
            "run-1",
            &mut state,
            &json!({
                "type": "assistant",
                "message": {
                    "content": [{
                        "type": "tool_use",
                        "id": "toolu_browser",
                        "name": "browser_snapshot",
                        "input": { "random": "value" }
                    }]
                }
            }),
            &mut batch,
        )
        .expect("provider-native tool use should be recorded");

        assert!(batch.notices.is_empty());
        assert_eq!(batch.chunks.len(), 1);
        assert_eq!(batch.chunks[0].kind, TerminalOutputKind::ProviderTool);
        let payload: serde_json::Value =
            serde_json::from_slice(&batch.chunks[0].bytes).expect("tool payload should be JSON");
        assert_eq!(payload["tool"], "browser_snapshot");
        assert_eq!(payload["status"], "completed");
        assert_eq!(payload["input"]["random"], "value");
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
