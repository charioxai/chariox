//! MP-08 / MP-10: value-free timing evidence at the actual provider socket seam.
use serde_json::json;

use super::CodexNotification;

pub(super) fn notification_received(provider_run_id: &str, notification: &CodexNotification) {
    let (notification, turn_id) = match notification {
        CodexNotification::TurnScoped {
            turn_id,
            notification,
        } => (notification.as_ref(), Some(turn_id)),
        notification => (notification, None),
    };
    let (message, fields) = match notification {
        CodexNotification::TurnCompleted {
            turn_id, status, ..
        } => (
            "codex turn completion received trace",
            json!({"provider_run_id":provider_run_id,"turn_id":turn_id,"status":status}),
        ),
        CodexNotification::CommandExecutionOutputDelta { delta, .. } => (
            "codex command output received trace",
            json!({"provider_run_id":provider_run_id,"turn_id":turn_id,"output_bytes":delta.len()}),
        ),
        CodexNotification::ExecCommandOutputDelta { chunk, .. } => (
            "codex command output received trace",
            json!({"provider_run_id":provider_run_id,"turn_id":turn_id,"output_bytes":chunk.len()}),
        ),
        _ => return,
    };
    crate::logging::debug_with_fields("daemon.provider.codex", message, fields);
}
