//! MP-08 / MP-10: reconcile Codex's lifecycle identity before interrupting.
use std::collections::BTreeSet;
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
    let thread_id = state.thread_id().to_string();
    let client = CodexClient::new(provider_run_id, state.endpoint())?;
    let deadline = Instant::now() + INTERRUPT_RETRY_TIMEOUT;
    let mut reconciliation = InterruptReconciliation::new(submitted_turn_id);
    let mut rejected_turns = BTreeSet::new();
    loop {
        // Use the same reconciliation before RPCs, after either RPC result, and
        // after every start-wait read. Drain ready events before terminal settlement.
        loop {
            match client.read_notification(&mut state.socket, Duration::from_millis(1)) {
                Ok(Some(notification)) => {
                    let closed = matches!(notification, CodexNotification::Error { .. });
                    state.buffered_notifications.push(notification);
                    if closed || Instant::now() >= deadline {
                        break;
                    }
                }
                Ok(None) => break,
                // A peer may close immediately after its ACK. Reconciliation
                // still decides whether that ACK covers the current identity.
                Err(_) if !reconciliation.acknowledged.is_empty() => break,
                Err(error) => return Err(error),
            }
        }
        let decision = reconciliation.reconcile(state);
        match decision {
            InterruptDecision::Settled => {
                note_codex_turn_interrupt_accepted(
                    &mut state.active_turn_id,
                    &mut state.turn_tracker,
                    &mut state.buffered_notifications,
                );
                return Ok(());
            }
            _ if Instant::now() >= deadline => {
                return Err(DaemonError::ProviderProtocol {
                    provider_run_id: provider_run_id.to_string(),
                    operation: "turn/interrupt",
                    message: "timed out waiting for the current provider turn to start".to_string(),
                });
            }
            InterruptDecision::Wait => sleep(INTERRUPT_RETRY_INTERVAL),
            InterruptDecision::Interrupt(turn_id) => {
                crate::logging::debug_with_fields(
                    "daemon.provider.codex",
                    "codex turn interrupt requested trace",
                    json!({"provider_run_id":provider_run_id,"turn_id":turn_id,"active_turn_id":state.active_turn_id}),
                );
                reconciliation.require_start = true;
                match client.turn_interrupt(
                    &mut state.socket,
                    &mut state.next_request_id,
                    &thread_id,
                    &turn_id,
                    &mut state.buffered_notifications,
                ) {
                    Ok(()) => {
                        reconciliation.acknowledged.insert(turn_id);
                        // The ACK is newer than the last snapshot. A different
                        // lifecycle identity still has to pass reconciliation.
                        reconciliation.snapshot = None;
                    }
                    Err(error) => {
                        let stale_id = codex_interrupt_actual_turn_id(&error);
                        let waiting = codex_turn_interrupt_is_waiting_for_task_start(&error);
                        // Record lifecycle changes even when the provider rejects
                        // an unrecoverable RPC; cancellation remains owned on error.
                        reconciliation.reconcile(state);
                        if (!waiting && stale_id.is_none())
                            || (stale_id.is_some() && !rejected_turns.insert(turn_id.clone()))
                        {
                            return Err(error);
                        }
                        if !state.ephemeral {
                            match client.thread_turns_list(
                                &mut state.socket,
                                &mut state.next_request_id,
                                &thread_id,
                                &mut state.buffered_notifications,
                            ) {
                                Ok(response) => {
                                    reconciliation.snapshot =
                                        Some((response, state.buffered_notifications.len()))
                                }
                                Err(error) => {
                                    reconciliation.reconcile(state);
                                    return Err(error);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum InterruptDecision {
    Interrupt(String),
    Wait,
    Settled,
}

// One cancellation owns one admitted prompt, but Codex's lifecycle ID can
// change across admission. Snapshots locate a candidate; only turn/started
// authorizes a retry. Completion and ACK apply to their exact identity.
struct InterruptReconciliation {
    target: String,
    observed: usize,
    last_start: Option<usize>,
    lifecycle_id: Option<String>,
    completed: BTreeSet<String>,
    acknowledged: BTreeSet<String>,
    snapshot: Option<(Value, usize)>,
    require_start: bool,
}

impl InterruptReconciliation {
    fn new(target: String) -> Self {
        Self {
            target,
            observed: 0,
            last_start: None,
            lifecycle_id: None,
            completed: BTreeSet::new(),
            acknowledged: BTreeSet::new(),
            snapshot: None,
            require_start: false,
        }
    }

    fn reconcile(&mut self, state: &mut CodexRuntimeState) -> InterruptDecision {
        for (index, notification) in state
            .buffered_notifications
            .iter()
            .enumerate()
            .skip(self.observed)
        {
            state.turn_tracker.observe_provider_turn(notification);
            match notification {
                CodexNotification::TurnStarted { turn_id } => {
                    self.last_start = Some(index);
                    self.lifecycle_id = Some(turn_id.clone());
                    self.completed.remove(turn_id);
                }
                CodexNotification::TurnCompleted { turn_id, .. } => {
                    self.completed.insert(turn_id.clone());
                }
                _ => {}
            }
        }
        self.observed = state.buffered_notifications.len();
        let active = state.turn_tracker.provider_active_turn_id.as_deref();
        let snapshot_id = self
            .snapshot
            .as_ref()
            .and_then(|(response, _)| codex_active_turn_id(response));
        // A listed older start cannot override the newest snapshot record.
        // A missing identity or a start received after the snapshot supersedes it.
        // The tracker can retain a previous prompt's identity. Only fresh
        // lifecycle evidence or the snapshot may rebind this cancellation.
        let lifecycle_id = self.lifecycle_id.as_deref().filter(|id| {
            self.snapshot.as_ref().is_none_or(|(response, offset)| {
                snapshot_id.as_deref().is_none_or(|current| current == *id)
                    || codex_turn_record(response, id).is_none()
                    || self.last_start.is_some_and(|index| index >= *offset)
            })
        });
        if let Some(id) = lifecycle_id.map(str::to_string).or(snapshot_id) {
            self.target = id;
        }
        let target = &self.target;
        let terminal_snapshot = self.snapshot.as_ref().is_some_and(|(response, _)| {
            codex_active_turn_id(response).is_none()
                && codex_turn_is_terminal(response, target)
                // An admitted turn normalized to interrupted can still run.
                // Lifecycle completion, checked below, is required in that case.
                && codex_turn_record(response, target)
                    .and_then(|turn| turn.get("status"))
                    .and_then(Value::as_str) != Some("interrupted")
                && lifecycle_id.is_none()
        });
        let completed = self.completed.contains(target)
            || state.turn_tracker.has_terminal_for(target)
            || terminal_snapshot;
        // Before the first RPC, a terminal old identity alone does not prove
        // the newly admitted prompt is cancelled. Ask the provider validator.
        if self.acknowledged.contains(target) || self.require_start && completed {
            InterruptDecision::Settled
        } else if !self.require_start || active == Some(target) {
            InterruptDecision::Interrupt(target.clone())
        } else {
            InterruptDecision::Wait
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
