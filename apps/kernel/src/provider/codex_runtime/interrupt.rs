//! MP-08 / MP-10: reconcile Codex's lifecycle identity before interrupting.
use std::thread::sleep;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::error::DaemonError;
use crate::provider::{CodexClient, CodexNotification};

use super::turn::CodexTurnTracker;
use super::CodexRuntimeState;

const INTERRUPT_RETRY_TIMEOUT: Duration = Duration::from_secs(5);
const INTERRUPT_RETRY_INTERVAL: Duration = Duration::from_millis(50);

pub fn abort_codex_turn(
    provider_run_id: &str,
    state: &mut CodexRuntimeState,
) -> Result<(), DaemonError> {
    let Some(submitted_turn_id) = state.active_turn_id.clone() else {
        return Ok(());
    };
    observe_buffered_turns(&mut state.turn_tracker, &state.buffered_notifications);
    let mut turn_id = state
        .turn_tracker
        .provider_active_turn_id
        .clone()
        .unwrap_or_else(|| submitted_turn_id.clone());
    let thread_id = state.thread_id().to_string();
    let client = CodexClient::new(provider_run_id, state.endpoint())?;
    let deadline = Instant::now() + INTERRUPT_RETRY_TIMEOUT;
    let mut retried_stale_id = false;
    loop {
        crate::logging::debug_with_fields(
            "daemon.provider.codex",
            "codex turn interrupt requested trace",
            json!({"provider_run_id":provider_run_id,"turn_id":turn_id,"active_turn_id":state.active_turn_id}),
        );
        match client.turn_interrupt(
            &mut state.socket,
            &mut state.next_request_id,
            &thread_id,
            &turn_id,
            &mut state.buffered_notifications,
        ) {
            Ok(()) => {
                note_codex_turn_interrupt_accepted(
                    &mut state.active_turn_id,
                    &mut state.turn_tracker,
                    &mut state.buffered_notifications,
                );
                return Ok(());
            }
            Err(error) => {
                let stale_id = codex_interrupt_actual_turn_id(&error);
                let waiting_for_start = codex_turn_interrupt_is_waiting_for_task_start(&error);
                if (!waiting_for_start && stale_id.is_none())
                    || (stale_id.is_some() && retried_stale_id)
                {
                    return Err(error);
                }
                // Both the thread snapshot and interleaved lifecycle events belong
                // to this actor-owned socket. Never select a turn from another run.
                let notification_offset = state.buffered_notifications.len();
                let response = if state.ephemeral {
                    None
                } else {
                    Some(client.thread_turns_list(
                        &mut state.socket,
                        &mut state.next_request_id,
                        &thread_id,
                        &mut state.buffered_notifications,
                    )?)
                };
                let fresh_notifications = &state.buffered_notifications[notification_offset..];
                let fresh_lifecycle = fresh_notifications.iter().any(|event| {
                    matches!(
                        event,
                        CodexNotification::TurnStarted { .. }
                            | CodexNotification::TurnCompleted { .. }
                    )
                });
                observe_buffered_turns(&mut state.turn_tracker, fresh_notifications);
                let snapshot_id = response.as_ref().and_then(codex_active_turn_id);
                // The interrupt validator's listener can still name the previous
                // turn while Core has admitted the resumed turn. A fresh lifecycle
                // event supersedes that validator; otherwise use its exact ID only
                // after the same-thread reread confirms the record exists.
                let actual_id = fresh_lifecycle
                    .then(|| state.turn_tracker.provider_active_turn_id.clone())
                    .flatten()
                    .or_else(|| {
                        stale_id
                            .filter(|id| {
                                response.as_ref().is_some_and(|response| {
                                    codex_turn_record(response, id).is_some()
                                })
                            })
                            .map(str::to_string)
                    })
                    .or(snapshot_id);
                if let Some(actual_id) = actual_id {
                    crate::logging::debug_with_fields(
                        "daemon.provider.codex",
                        "codex turn interrupt identity refreshed",
                        json!({"provider_run_id":provider_run_id,"turn_id":actual_id,"previous_active_turn_id":turn_id}),
                    );
                    state.turn_tracker.provider_active_turn_id = Some(actual_id.clone());
                    turn_id = actual_id;
                    if stale_id.is_some() {
                        retried_stale_id = true;
                    }
                } else if waiting_for_start
                    && response.as_ref().is_some_and(|response| {
                        codex_turn_is_terminal(response, &submitted_turn_id)
                    })
                {
                    note_codex_turn_interrupt_accepted(
                        &mut state.active_turn_id,
                        &mut state.turn_tracker,
                        &mut state.buffered_notifications,
                    );
                    return Ok(());
                } else if stale_id.is_some() {
                    return Err(error);
                }
                if Instant::now() >= deadline {
                    return Err(error);
                }
                if waiting_for_start {
                    sleep(INTERRUPT_RETRY_INTERVAL);
                }
            }
        }
    }
}

fn observe_buffered_turns(tracker: &mut CodexTurnTracker, notifications: &[CodexNotification]) {
    for notification in notifications {
        tracker.observe_provider_turn(notification);
    }
}

pub(super) fn codex_turn_interrupt_is_waiting_for_task_start(error: &DaemonError) -> bool {
    matches!(error, DaemonError::ProviderProtocol {operation:"turn/interrupt",message,..} if message.contains("no active turn to interrupt"))
}

fn codex_interrupt_actual_turn_id(error: &DaemonError) -> Option<&str> {
    let DaemonError::ProviderProtocol {
        operation: "turn/interrupt",
        message,
        ..
    } = error
    else {
        return None;
    };
    let (_, actual) = message
        .strip_prefix("expected active turn id ")?
        .split_once(" but found ")?;
    let actual = actual.trim().trim_matches('`');
    (!actual.is_empty()
        && actual
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
    .then_some(actual)
}

fn codex_turn_record<'a>(response: &'a Value, turn_id: &str) -> Option<&'a Value> {
    response
        .get("data")?
        .as_array()?
        .iter()
        .find(|turn| turn.get("id").and_then(Value::as_str) == Some(turn_id))
}

fn codex_active_turn_id(response: &Value) -> Option<String> {
    let mut active = response
        .get("data")?
        .as_array()?
        .iter()
        .filter(|turn| turn.get("status").and_then(Value::as_str) == Some("inProgress"));
    let id = active.next()?.get("id")?.as_str()?.trim();
    (!id.is_empty() && active.next().is_none()).then(|| id.to_string())
}

pub(super) fn codex_turn_is_terminal(response: &Value, turn_id: &str) -> bool {
    codex_turn_record(response, turn_id)
        .and_then(|turn| turn.get("status"))
        .and_then(Value::as_str)
        .is_some_and(|status| {
            matches!(
                status,
                "completed" | "failed" | "interrupted" | "cancelled" | "canceled"
            )
        })
}

pub(super) fn note_codex_turn_interrupt_accepted(
    active_turn_id: &mut Option<String>,
    turn_tracker: &mut CodexTurnTracker,
    buffered_notifications: &mut Vec<CodexNotification>,
) {
    observe_buffered_turns(turn_tracker, buffered_notifications);
    *active_turn_id = None;
    turn_tracker.reset_for_started();
    // Output before the ACK belongs to the cancelled prompt, never its retry.
    buffered_notifications.clear();
}
