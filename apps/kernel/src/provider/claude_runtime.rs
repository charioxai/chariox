use std::collections::BTreeMap;
use std::sync::mpsc::TryRecvError;

use serde_json::json;

use crate::error::DaemonError;
use crate::session::PromptAttachment;

use super::{
    claude::claude_launch_args_for_run, AgentExecutionMode, AgentPermissionLevel,
    ProviderPromptSignalBatch, RuntimeProviderRun,
};

const CLAUDE_EVENT_DRAIN_MAX_MESSAGES: usize = 256;

mod events;
mod input;
mod process;
mod state;

use events::apply_claude_message;
use input::claude_user_content;
use process::{spawn_claude_child, stop_child, write_json_line, ClaudeRuntimeMessage};
pub(crate) use state::{ClaudeRunSelection, ClaudeRuntimeBinding, ClaudeRuntimeState};

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
            model: Some(run.model().to_string()),
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

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use serde_json::json;

    use crate::provider::{AgentExecutionMode, AgentPermissionLevel};
    use crate::session::PromptAttachment;
    use crate::terminal::TerminalOutputKind;

    use super::{
        events::apply_claude_message, input::claude_user_content, ClaudeRuntimeState,
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
