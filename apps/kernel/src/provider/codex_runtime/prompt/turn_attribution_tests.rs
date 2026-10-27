// MP-08/MP-10: a cancelled provider turn must never write into its retry.
use super::super::events::apply_notification;
use super::super::turn::CodexTurnTracker;
use super::note_codex_turn_interrupt_accepted;
use crate::provider::codex_client::fixture_codex_notification;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn assert_late_output_rejected(method: &str, params: Value) {
    assert_late_output_rejected_for(method, params, Some("retry"));
}

fn assert_late_output_rejected_for(method: &str, params: Value, retry: Option<&str>) {
    let mut active = Some("original".to_string());
    let mut tracker = CodexTurnTracker::default();
    let mut buffered = Vec::new();
    note_codex_turn_interrupt_accepted(&mut active, &mut tracker, &mut buffered);
    if retry.is_some() {
        super::note_codex_turn_start_response(
            &mut active,
            &mut tracker,
            &json!({"turn":{"id":"retry"}}),
            false,
        );
    }
    let mut text = BTreeMap::new();
    let mut tools = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut completed = false;
    let mut failure = None;
    let mut usage = None;
    apply_notification(
        fixture_codex_notification(method, params),
        &mut active,
        &mut tracker,
        &mut text,
        &mut tools,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut completed,
        &mut failure,
        &mut usage,
    );
    assert!(
        chunks.is_empty(),
        "late original output was projected into retry: {chunks:?}"
    );
    assert!(text.is_empty() && tools.is_empty());
    assert_eq!(tracker.active_tool_count(), 0);
    assert!(!completed);
    assert!(failure.is_none() && notices.is_empty() && usage.is_none());
    assert_eq!(active.as_deref(), retry);
    active = Some("retry".to_string());
    apply_notification(
        fixture_codex_notification(
            "item/completed",
            json!({
                "turnId":"retry", "item":{"type":"agentMessage", "id":"retry-answer", "text":"RETRY_COMPLETE"}
            }),
        ),
        &mut active,
        &mut tracker,
        &mut text,
        &mut tools,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut completed,
        &mut failure,
        &mut usage,
    );
    assert_eq!(
        chunks
            .iter()
            .flat_map(|c| c.bytes.clone())
            .collect::<Vec<_>>(),
        b"RETRY_COMPLETE"
    );
}

#[test]
fn mp08_mp10_cancel_retry_rejects_late_command_completion() {
    assert_late_output_rejected(
        "item/completed",
        json!({
            "turnId":"original", "item":{"type":"commandExecution", "id":"old-command", "status":"completed", "command":"sleep 120", "aggregatedOutput":"SHOULD_NOT_FINISH", "exitCode":0}
        }),
    );
}

#[test]
fn mp08_mp10_cancel_retry_rejects_late_command_delta() {
    assert_late_output_rejected(
        "item/commandExecution/outputDelta",
        json!({
            "turnId":"original", "itemId":"old-command", "delta":"SHOULD_NOT_FINISH"
        }),
    );
}

#[test]
fn mp08_mp10_cancel_retry_rejects_correlated_legacy_completion() {
    assert_late_output_rejected(
        "codex/event/exec_command_end",
        json!({
            "turn_id":"original", "msg":{"call_id":"old-command", "command":"sleep 120", "aggregated_output":"SHOULD_NOT_FINISH", "exit_code":0}
        }),
    );
}

#[test]
fn mp08_mp10_cancel_retry_rejects_late_tool_start_and_progress() {
    assert_late_output_rejected(
        "item/started",
        json!({"turnId":"original", "item":{"type":"commandExecution", "id":"old-command", "status":"inProgress", "command":"sleep 120"}}),
    );
    assert_late_output_rejected(
        "item/mcpToolCall/progress",
        json!({"turnId":"original", "itemId":"old-command", "message":"late progress"}),
    );
}

#[test]
fn mp08_mp10_cancel_retry_rejects_late_assistant_and_reasoning() {
    for method in [
        "item/agentMessage/delta",
        "item/reasoning/textDelta",
        "item/reasoning/summaryTextDelta",
    ] {
        assert_late_output_rejected(
            method,
            json!({"turnId":"original", "itemId":"old-answer", "delta":"SHOULD_NOT_FINISH"}),
        );
    }
}

#[test]
fn mp08_mp10_cancel_retry_rejects_late_error_and_abort() {
    assert_late_output_rejected(
        "error",
        json!({"turnId":"original", "error":{"message":"old error"}}),
    );
    assert_late_output_rejected(
        "codex/event/turn_aborted",
        json!({"turn_id":"original", "msg":{"reason":"old abort"}}),
    );
}

#[test]
fn mp08_mp10_cancel_retry_rejects_late_usage() {
    assert_late_output_rejected(
        "thread/tokenUsage/updated",
        json!({"turnId":"original", "threadId":"thread", "tokenUsage":{"total":{"totalTokens":123}, "last":{"totalTokens":123}, "modelContextWindow":1000}}),
    );
}

#[test]
fn mp08_mp10_uncorrelated_legacy_output_remains_supported() {
    let parsed = fixture_codex_notification(
        "codex/event/exec_command_end",
        json!({"msg":{"call_id":"legacy", "command":"pwd", "aggregated_output":"/tmp", "exit_code":0}}),
    );
    assert!(matches!(
        parsed,
        crate::provider::CodexNotification::ExecCommandCompleted { .. }
    ));
}

#[test]
fn mp08_mp10_cancel_without_retry_rejects_late_output() {
    assert_late_output_rejected_for(
        "item/agentMessage/delta",
        json!({"turnId":"original", "itemId":"late-answer", "delta":"SHOULD_NOT_FINISH"}),
        None,
    );
}
