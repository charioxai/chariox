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
                crate::logging::debug_with_fields(
                    "daemon.provider.codex",
                    "codex turn interrupt snapshot trace",
                    json!({"provider_run_id":provider_run_id,"turn_id":snapshot_id}),
                );
                // The validator may advance while the reread is in flight. Fresh
                // lifecycle events and the current in-progress record supersede
                // the ID in the rejection, which is already an older observation.
                let actual_id = fresh_lifecycle
                    .then(|| state.turn_tracker.provider_active_turn_id.clone())
                    .flatten()
                    .filter(|id| {
                        snapshot_id.as_deref().is_none_or(|current| current == id)
                            || response
                                .as_ref()
                                .is_some_and(|response| codex_turn_record(response, id).is_none())
                    })
                    .or(snapshot_id)
                    .or_else(|| {
                        stale_id
                            .filter(|id| {
                                response.as_ref().is_some_and(|response| {
                                    codex_turn_record(response, id).is_some()
                                })
                            })
                            .map(str::to_string)
                    });
                // A terminal submitted record cannot settle a newer lifecycle
                // turn that is still missing from the snapshot.
                if response.as_ref().is_some_and(|response| {
                    codex_active_turn_id(response).is_none()
                        && actual_id.as_deref().is_none_or(|id| {
                            terminal_interruption_is_settled(response, id, state)
                        })
                        && terminal_interruption_is_settled(response, &submitted_turn_id, state)
                }) {
                    note_codex_turn_interrupt_accepted(
                        &mut state.active_turn_id,
                        &mut state.turn_tracker,
                        &mut state.buffered_notifications,
                    );
                    return Ok(());
                }
                let target = actual_id
                    .filter(|id| {
                        response
                            .as_ref()
                            .is_none_or(|response| !codex_turn_is_terminal(response, id))
                    })
                    .unwrap_or_else(|| submitted_turn_id.clone());
                if !wait_for_provider_turn_start(
                    provider_run_id,
                    &client,
                    state,
                    &target,
                    deadline,
                )? {
                    note_codex_turn_interrupt_accepted(
                        &mut state.active_turn_id,
                        &mut state.turn_tracker,
                        &mut state.buffered_notifications,
                    );
                    return Ok(());
                }
                crate::logging::debug_with_fields(
                    "daemon.provider.codex",
                    "codex turn interrupt identity refreshed",
                    json!({"provider_run_id":provider_run_id,"turn_id":target,"previous_active_turn_id":turn_id}),
                );
                turn_id = target;
                if stale_id.is_some() {
                    retried_stale_id = true;
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

fn terminal_interruption_is_settled(
    response: &Value,
    turn_id: &str,
    state: &CodexRuntimeState,
) -> bool {
    if !codex_turn_is_terminal(response, turn_id) {
        return false;
    }
    // Codex can normalize an admitted, not-yet-started record to interrupted.
    // That status needs actual completion evidence, rather than hiding a queued turn.
    let interrupted = codex_turn_record(response, turn_id)
        .and_then(|turn| turn.get("status"))
        .and_then(Value::as_str)
        == Some("interrupted");
    !interrupted || state.turn_tracker.has_terminal_for(turn_id)
        || state.buffered_notifications.iter().any(|event| {
            matches!(event, CodexNotification::TurnCompleted { turn_id: completed, .. } if completed == turn_id)
        })
}

fn wait_for_provider_turn_start(
    provider_run_id: &str,
    client: &CodexClient,
    state: &mut CodexRuntimeState,
    turn_id: &str,
    deadline: Instant,
) -> Result<bool, DaemonError> {
    // A turn/start reply and the rollout snapshot can precede the listener's
    // interrupt validator. Only the lifecycle event proves this turn started.
    observe_buffered_turns(&mut state.turn_tracker, &state.buffered_notifications);
    loop {
        if state.turn_tracker.has_terminal_for(turn_id) || state.buffered_notifications.iter().any(|event| {
            matches!(event, CodexNotification::TurnCompleted { turn_id: completed, .. } if completed == turn_id)
        }) {
            return Ok(false);
        }
        if state.turn_tracker.provider_active_turn_id.as_deref() == Some(turn_id) {
            crate::logging::debug_with_fields(
                "daemon.provider.codex",
                "codex turn interrupt provider started trace",
                json!({"provider_run_id":provider_run_id,"turn_id":turn_id}),
            );
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Err(DaemonError::ProviderProtocol {
                provider_run_id: provider_run_id.to_string(),
                operation: "turn/interrupt",
                message: "timed out waiting for the current provider turn to start".to_string(),
            });
        }
        if let Some(notification) =
            client.read_notification(&mut state.socket, INTERRUPT_RETRY_INTERVAL)?
        {
            state.turn_tracker.observe_provider_turn(&notification);
            state.buffered_notifications.push(notification);
        } else {
            sleep(INTERRUPT_RETRY_INTERVAL);
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
    // thread/turns/list defaults to newest first. During admission, history
    // can retain an older in-progress record alongside the current turn.
    let active = response
        .get("data")?
        .as_array()?
        .iter()
        .find(|turn| turn.get("status").and_then(Value::as_str) == Some("inProgress"))?;
    let id = active.get("id")?.as_str()?.trim();
    (!id.is_empty()).then(|| id.to_string())
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
