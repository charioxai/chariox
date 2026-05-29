//! Codex prompt turn start, interruption, and turn-id extraction.

use serde_json::{json, Value};

use crate::error::DaemonError;
use crate::prompt_assembly::PromptEnvelope;
use crate::provider::{CodexClient, CodexNotification, RuntimeProviderRun};

use super::input::codex_input;
use super::run_config::{codex_client_for_run, normalize_codex_model, normalize_variant};
use super::turn::CodexTurnTracker;
use super::CodexRuntimeState;

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
    let input = codex_input(&envelope.visible_user_prompt, &envelope.attachments);
    let thread_id = state.thread_id().to_string();
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
        hidden_context_for_provider(&envelope.hidden_system_context),
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
    state.turn_tracker = CodexTurnTracker::default();
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

fn hidden_context_for_provider(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
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
    )
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
