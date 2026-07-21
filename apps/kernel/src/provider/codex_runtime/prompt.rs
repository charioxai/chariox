//! Codex prompt turn start, interruption, and turn-id extraction.

use std::thread::sleep;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::error::DaemonError;
use crate::prompt_assembly::PromptEnvelope;
use crate::provider::{CodexClient, CodexNotification, RuntimeProviderRun};

use super::input::codex_input;
use super::run_config::{codex_client_for_run, normalize_codex_model, normalize_variant};
use super::turn::CodexTurnTracker;
use super::CodexRuntimeState;

const CODEX_MCP_THREAD_INIT_RETRY_TIMEOUT: Duration = Duration::from_secs(150);
const CODEX_MCP_THREAD_INIT_RETRY_INTERVAL: Duration = Duration::from_millis(500);

pub fn submit_codex_prompt(
    run: &RuntimeProviderRun,
    state: &mut CodexRuntimeState,
    envelope: &PromptEnvelope,
) -> Result<(), DaemonError> {
    let client = codex_client_for_run(run, state.endpoint(), None)?;
    let cwd = run
        .working_directory()
        .map(|path| path.to_string_lossy().to_string());
    let model = normalize_codex_model(run.model());
    let effort = normalize_variant(run.variant());
    if let Err(error) = ensure_codex_thread_ready(
        &client,
        run,
        state,
        cwd.as_deref(),
        model.as_deref(),
        hidden_context_for_provider(&envelope.hidden_system_context),
        envelope.steering,
    ) {
        state.buffered_notifications.push(CodexNotification::Error {
            message: error.to_string(),
        });
        return Ok(());
    }
    let turn_input_prompt = codex_turn_input_prompt(
        &envelope.visible_user_prompt,
        &envelope.hidden_system_context,
        state.turn_input_includes_hidden_context(),
    );
    let input = codex_input(&turn_input_prompt, &envelope.attachments);
    let thread_id = state.thread_id().to_string();
    let active_steering_turn_id = envelope
        .steering
        .then(|| state.active_turn_id.clone())
        .flatten();
    let response_result = match active_steering_turn_id.as_deref() {
        Some(active_turn_id) => client.turn_steer(
            &mut state.socket,
            &mut state.next_request_id,
            &thread_id,
            active_turn_id,
            input,
            &mut state.buffered_notifications,
        ),
        None => client.turn_start(
            &mut state.socket,
            &mut state.next_request_id,
            &thread_id,
            cwd.as_deref(),
            model.as_deref(),
            effort.as_deref(),
            run.write_access_mode(),
            run.execution_mode(),
            run.permission_level(),
            hidden_context_for_provider(&envelope.hidden_system_context),
            input,
            &mut state.buffered_notifications,
        ),
    };
    let response = match response_result {
        Ok(response) => response,
        Err(error) => {
            state.buffered_notifications.push(CodexNotification::Error {
                message: error.to_string(),
            });
            return Ok(());
        }
    };
    note_codex_turn_start_response(
        &mut state.active_turn_id,
        &mut state.turn_tracker,
        &response,
        envelope.steering,
    );
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

pub(super) fn note_codex_turn_start_response(
    active_turn_id: &mut Option<String>,
    turn_tracker: &mut CodexTurnTracker,
    response: &Value,
    steering: bool,
) {
    let preserve_active_turn = steering && active_turn_id.is_some();
    if let Some(turn_id) = codex_turn_id_from_start_response(response) {
        if !preserve_active_turn {
            *active_turn_id = Some(turn_id);
        }
    }
    if !preserve_active_turn {
        *turn_tracker = CodexTurnTracker::default();
    }
}

fn ensure_codex_thread_ready(
    client: &CodexClient,
    run: &RuntimeProviderRun,
    state: &mut CodexRuntimeState,
    cwd: Option<&str>,
    model: Option<&str>,
    developer_instructions: Option<&str>,
    steering: bool,
) -> Result<(), DaemonError> {
    if codex_active_steering_preserves_thread(
        state.thread_ready(),
        state.active_turn_id.is_some(),
        steering,
    ) {
        return Ok(());
    }
    let desired_fingerprint = developer_instructions_fingerprint(developer_instructions);
    if state.thread_ready()
        && state.developer_instructions_fingerprint() == Some(desired_fingerprint.as_str())
    {
        return Ok(());
    }
    if state.thread_ready() && !state.context_hot_reload_enabled() {
        return Ok(());
    }
    if state.thread_ready() {
        if state.active_turn_id.is_some() {
            return Err(DaemonError::ProviderProtocol {
                provider_run_id: run.id().to_string(),
                operation: "thread/hot-reload",
                message: "cannot hot reload Codex hidden context while a turn is active"
                    .to_string(),
            });
        }
        crate::logging::info_with_fields(
            "daemon.provider.codex",
            "hot reloading codex thread for changed hidden context",
            serde_json::json!({
                "provider_run_id": run.id(),
                "previous_thread_id": state.thread_id(),
            }),
        );
    }
    let deadline = Instant::now() + CODEX_MCP_THREAD_INIT_RETRY_TIMEOUT;
    loop {
        let result = if state.thread_ready() {
            client.thread_start(
                &mut state.socket,
                &mut state.next_request_id,
                cwd,
                model,
                run.write_access_mode(),
                run.execution_mode(),
                run.permission_level(),
                developer_instructions,
            )
        } else if let Some(thread_id) = state.pending_thread_id().map(str::to_string) {
            client.thread_resume(
                &mut state.socket,
                &mut state.next_request_id,
                &thread_id,
                cwd,
                model,
                run.write_access_mode(),
                run.execution_mode(),
                run.permission_level(),
                developer_instructions,
            )
        } else {
            client.thread_start(
                &mut state.socket,
                &mut state.next_request_id,
                cwd,
                model,
                run.write_access_mode(),
                run.execution_mode(),
                run.permission_level(),
                developer_instructions,
            )
        };
        match result {
            Ok(thread) => {
                if state.thread_ready() {
                    state.replace_thread(thread.thread.id, Some(desired_fingerprint));
                } else {
                    state.mark_thread_ready(thread.thread.id, Some(desired_fingerprint));
                }
                return Ok(());
            }
            Err(error) if is_codex_mcp_handshake_timeout(&error) && Instant::now() < deadline => {
                crate::logging::warn_with_fields(
                    "daemon.provider.codex",
                    "retrying codex thread init after MCP handshake timeout",
                    serde_json::json!({
                        "provider_run_id": run.id(),
                        "error": error.to_string(),
                    }),
                );
                sleep(CODEX_MCP_THREAD_INIT_RETRY_INTERVAL);
            }
            Err(error) => return Err(error),
        }
    }
}

fn codex_active_steering_preserves_thread(
    thread_ready: bool,
    active_turn: bool,
    steering: bool,
) -> bool {
    thread_ready && active_turn && steering
}

#[cfg(test)]
mod prompt_tests {
    use super::{
        codex_active_steering_preserves_thread, note_codex_turn_interrupt_accepted,
        CodexNotification, CodexTurnTracker,
    };

    #[test]
    fn active_steering_preserves_the_existing_codex_thread() {
        assert!(codex_active_steering_preserves_thread(true, true, true));
        assert!(!codex_active_steering_preserves_thread(true, true, false));
        assert!(!codex_active_steering_preserves_thread(true, false, true));
        assert!(!codex_active_steering_preserves_thread(false, true, true));
    }

    #[test]
    fn accepted_interrupt_releases_runtime_for_the_next_fifo_prompt() {
        let mut active_turn_id = Some("turn-cancelled".to_string());
        let mut turn_tracker = CodexTurnTracker::default();
        turn_tracker.note_tool_started("tool-still-active");
        let mut buffered_notifications = vec![CodexNotification::TurnCompleted {
            turn_id: "turn-cancelled".to_string(),
            status: "interrupted".to_string(),
            error_message: None,
            items: Vec::new(),
        }];

        note_codex_turn_interrupt_accepted(
            &mut active_turn_id,
            &mut turn_tracker,
            &mut buffered_notifications,
        );

        assert_eq!(active_turn_id, None);
        assert_eq!(turn_tracker.active_tool_count(), 0);
        assert!(!turn_tracker.has_pending_terminal());
        assert!(buffered_notifications.is_empty());
    }
}

fn developer_instructions_fingerprint(value: Option<&str>) -> String {
    format!("{:x}", Sha256::digest(value.unwrap_or_default().as_bytes()))
}

fn hidden_context_for_provider(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn codex_turn_input_prompt(
    visible_user_prompt: &str,
    hidden_system_context: &str,
    include_hidden_context: bool,
) -> String {
    if !include_hidden_context || hidden_system_context.trim().is_empty() {
        return visible_user_prompt.to_string();
    }
    match (hidden_system_context.trim(), visible_user_prompt.trim()) {
        ("", visible) => visible.to_string(),
        (hidden, "") => hidden.to_string(),
        // Provider-native and resumed threads cannot reliably retrofit developer
        // instructions. Keep the handoff first and the workflow contract last so
        // the raw payload does not become the turn's final instruction.
        (hidden, visible) => format!("{visible}\n\n{hidden}"),
    }
}

fn is_codex_mcp_handshake_timeout(error: &DaemonError) -> bool {
    let DaemonError::ProviderProtocol {
        operation, message, ..
    } = error
    else {
        return false;
    };

    matches!(*operation, "thread/start" | "thread/resume")
        && message.contains("required MCP servers failed to initialize")
        && message.contains("timed out handshaking with MCP server")
}

pub fn abort_codex_turn(
    provider_run_id: &str,
    state: &mut CodexRuntimeState,
) -> Result<(), DaemonError> {
    let Some(turn_id) = state.active_turn_id.clone() else {
        return Ok(());
    };
    let thread_id = state.thread_id().to_string();
    let client = CodexClient::new(provider_run_id, state.endpoint())?;
    client.turn_interrupt(
        &mut state.socket,
        &mut state.next_request_id,
        &thread_id,
        &turn_id,
        &mut state.buffered_notifications,
    )?;
    note_codex_turn_interrupt_accepted(
        &mut state.active_turn_id,
        &mut state.turn_tracker,
        &mut state.buffered_notifications,
    );
    Ok(())
}

fn note_codex_turn_interrupt_accepted(
    active_turn_id: &mut Option<String>,
    turn_tracker: &mut CodexTurnTracker,
    buffered_notifications: &mut Vec<CodexNotification>,
) {
    *active_turn_id = None;
    turn_tracker.reset_for_started();
    // These notifications were received before the interrupt acknowledgement and belong to the
    // cancelled turn. Carrying them into the next FIFO submit would project stale output onto the
    // promoted prompt, which the kernel has already made authoritative.
    buffered_notifications.clear();
}

pub(super) fn codex_turn_id_from_start_response(response: &Value) -> Option<String> {
    response
        .get("turn")
        .and_then(|turn| turn.get("id"))
        .and_then(Value::as_str)
        .or_else(|| response.get("id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::codex_turn_input_prompt;

    #[test]
    fn attached_or_resumed_codex_thread_receives_hidden_context_in_turn_input() {
        let prompt = codex_turn_input_prompt(
            "<workflow-handoff-payloads>20</workflow-handoff-payloads>",
            "<node-level-prompt>subtract 9</node-level-prompt>",
            true,
        );

        assert_eq!(
            prompt,
            "<workflow-handoff-payloads>20</workflow-handoff-payloads>\n\n<node-level-prompt>subtract 9</node-level-prompt>"
        );
    }

    #[test]
    fn new_managed_codex_thread_keeps_hidden_context_in_developer_instructions() {
        let prompt = codex_turn_input_prompt("visible handoff", "hidden instructions", false);

        assert_eq!(prompt, "visible handoff");
    }
}
