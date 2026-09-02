use super::super::drain::{codex_authoritative_backfill_due, codex_turn_should_backfill};
use super::super::events::codex_completed_turn_has_settlement_evidence;
use super::super::prompt::note_codex_turn_start_response;
use super::super::turn::CodexTerminalSignal;
use super::*;

#[test]
fn delayed_previous_turn_start_cannot_replace_the_submitted_turn() {
    let mut active_turn_id = Some("turn-new".to_string());
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::TurnStarted {
            turn_id: "turn-previous".to_string(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    apply_notification(
        CodexNotification::TurnCompleted {
            turn_id: "turn-previous".to_string(),
            status: "completed".to_string(),
            error_message: None,
            items: Vec::new(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    flush_quiet_terminal_for_test(
        &mut active_turn_id,
        &mut turn_tracker,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
    );

    assert_eq!(active_turn_id.as_deref(), Some("turn-new"));
    assert!(!prompt_completed);
    assert!(completions.is_empty());
}

#[test]
fn legacy_task_complete_arms_authoritative_turn_backfill() {
    let mut active_turn_id = Some("turn-1".to_string());
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::TaskComplete {
            turn_id: Some("turn-1".to_string()),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );

    assert!(turn_tracker.has_legacy_completion_hint());
    assert!(!turn_tracker.has_pending_terminal());
    assert!(codex_turn_should_backfill(
        crate::provider::AgentEndpointMode::Managed,
        true,
        &turn_tracker,
        false,
    ));
    assert!(!prompt_completed);
    assert!(completions.is_empty());
}

#[test]
fn delayed_previous_turn_start_cannot_arm_the_next_prompt_before_submission() {
    let mut active_turn_id = None;
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::TurnStarted {
            turn_id: "turn-previous".to_string(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    apply_notification(
        CodexNotification::TurnCompleted {
            turn_id: "turn-previous".to_string(),
            status: "completed".to_string(),
            error_message: None,
            items: Vec::new(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    flush_quiet_terminal_for_test(
        &mut active_turn_id,
        &mut turn_tracker,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
    );

    assert_eq!(active_turn_id, None);
    assert!(!prompt_completed);
    assert!(completions.is_empty());
}

#[test]
fn terminal_signal_is_discarded_after_backfill_clears_the_active_turn() {
    let mut active_turn_id = Some("turn-1".to_string());
    let mut turn_tracker = CodexTurnTracker::default();
    turn_tracker.note_terminal(CodexTerminalSignal {
        turn_id: "turn-1".to_string(),
        status: "completed".to_string(),
        error_message: None,
    });
    let settled_turn_id = active_turn_id.take();
    assert_eq!(settled_turn_id.as_deref(), Some("turn-1"));

    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    flush_quiet_terminal_for_test(
        &mut active_turn_id,
        &mut turn_tracker,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
    );

    assert_eq!(active_turn_id, None);
    assert!(!turn_tracker.has_pending_terminal());
    assert!(!prompt_completed);
    assert!(completions.is_empty());
    assert!(notices.is_empty());
    assert_eq!(terminal_failure, None);
}

#[test]
fn stale_terminal_signal_cannot_complete_a_newer_active_turn() {
    let mut active_turn_id = Some("turn-2".to_string());
    let mut turn_tracker = CodexTurnTracker::default();
    turn_tracker.note_terminal(CodexTerminalSignal {
        turn_id: "turn-1".to_string(),
        status: "completed".to_string(),
        error_message: None,
    });

    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    flush_quiet_terminal_for_test(
        &mut active_turn_id,
        &mut turn_tracker,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
    );

    assert_eq!(active_turn_id.as_deref(), Some("turn-2"));
    assert!(!turn_tracker.has_pending_terminal());
    assert!(!prompt_completed);
    assert!(completions.is_empty());
    assert!(notices.is_empty());
    assert_eq!(terminal_failure, None);
}

#[test]
fn only_turn_completed_marks_the_prompt_as_complete() {
    let mut active_turn_id = Some("turn-1".to_string());
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::ItemCompleted {
            item: json!({
                "type": "commandExecution",
                "id": "cmd-1",
                "command": "ls",
                "status": "completed",
            }),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    assert!(!prompt_completed);
    assert_eq!(active_turn_id.as_deref(), Some("turn-1"));

    apply_notification(
        CodexNotification::TurnCompleted {
            turn_id: "turn-1".to_string(),
            status: "completed".to_string(),
            error_message: None,
            items: Vec::new(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    flush_quiet_terminal_for_test(
        &mut active_turn_id,
        &mut turn_tracker,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
    );
    assert!(prompt_completed);
    assert_eq!(active_turn_id, None);
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].message_id, "codex-turn:turn-1");
}

#[test]
fn turn_completed_reconciles_full_assistant_item_after_partial_stream() {
    let mut active_turn_id = Some("turn-1".to_string());
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::AgentMessageDelta {
            item_id: "message-1".to_string(),
            delta: "```".to_string(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    apply_notification(
        CodexNotification::TurnCompleted {
            turn_id: "turn-1".to_string(),
            status: "completed".to_string(),
            error_message: None,
            items: vec![json!({
                "type": "agentMessage",
                "id": "message-1",
                "text": "```json\n{\"summary\":\"done\",\"output\":{\"message\":\"20\"}}\n```",
            })],
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );

    assert_eq!(
        chunks
            .iter()
            .filter(|chunk| chunk.kind == TerminalOutputKind::ProviderOutput)
            .map(|chunk| String::from_utf8_lossy(&chunk.bytes))
            .collect::<String>(),
        "```json\n{\"summary\":\"done\",\"output\":{\"message\":\"20\"}}\n```"
    );
}

#[test]
fn steering_turn_start_preserves_original_active_turn_tracking() {
    let mut active_turn_id = Some("original-turn".to_string());
    let mut turn_tracker = CodexTurnTracker::default();
    turn_tracker.note_tool_started("sleep-tool");

    note_codex_turn_start_response(
        &mut active_turn_id,
        &mut turn_tracker,
        &json!({
            "turn": {
                "id": "steering-turn"
            }
        }),
        true,
    );

    assert_eq!(active_turn_id.as_deref(), Some("original-turn"));
    assert_eq!(turn_tracker.active_tool_count(), 1);

    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::ItemCompleted {
            item: json!({
                "type": "commandExecution",
                "id": "sleep-tool",
                "command": "sleep 30",
                "status": "completed",
            }),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    apply_notification(
        CodexNotification::TurnCompleted {
            turn_id: "original-turn".to_string(),
            status: "completed".to_string(),
            error_message: None,
            items: Vec::new(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    flush_quiet_terminal_for_test(
        &mut active_turn_id,
        &mut turn_tracker,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
    );

    assert!(prompt_completed);
    assert_eq!(active_turn_id, None);
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].message_id, "codex-turn:original-turn");
}

#[test]
fn turn_completion_waits_for_socket_quiet_before_prompt_completion() {
    let mut active_turn_id = Some("turn-1".to_string());
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::TurnCompleted {
            turn_id: "turn-1".to_string(),
            status: "completed".to_string(),
            error_message: None,
            items: Vec::new(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );

    assert!(!prompt_completed);
    assert_eq!(active_turn_id.as_deref(), Some("turn-1"));
    assert!(completions.is_empty());

    flush_quiet_terminal_for_test(
        &mut active_turn_id,
        &mut turn_tracker,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
    );
    assert!(prompt_completed);
    assert_eq!(active_turn_id, None);
}

#[test]
fn terminal_completion_waits_for_late_tool_output_before_prompt_completion() {
    let mut active_turn_id = Some("turn-1".to_string());
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::TurnCompleted {
            turn_id: "turn-1".to_string(),
            status: "completed".to_string(),
            error_message: None,
            items: Vec::new(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    assert!(!prompt_completed);
    assert_eq!(active_turn_id.as_deref(), Some("turn-1"));
    assert!(completions.is_empty());

    apply_notification(
        CodexNotification::ItemStarted {
            item: json!({
                "type": "commandExecution",
                "id": "cmd-1",
                "command": "echo still running",
                "status": "inProgress",
            }),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    flush_quiet_terminal_for_test(
        &mut active_turn_id,
        &mut turn_tracker,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
    );
    assert!(!prompt_completed);
    assert_eq!(active_turn_id.as_deref(), Some("turn-1"));
    assert!(completions.is_empty());

    apply_notification(
        CodexNotification::ItemCompleted {
            item: json!({
                "type": "commandExecution",
                "id": "cmd-1",
                "command": "echo still running",
                "status": "completed",
                "aggregatedOutput": "ok",
                "exitCode": 0,
            }),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    assert!(!prompt_completed);
    assert_eq!(active_turn_id.as_deref(), Some("turn-1"));
    assert!(completions.is_empty());

    flush_quiet_terminal_for_test(
        &mut active_turn_id,
        &mut turn_tracker,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
    );

    assert!(prompt_completed);
    assert_eq!(active_turn_id, None);
    assert_eq!(completions.len(), 1);
}

#[test]
fn turn_completion_waits_for_running_command_execution() {
    let mut active_turn_id = Some("turn-1".to_string());
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::ItemStarted {
            item: json!({
                "type": "commandExecution",
                "id": "cmd-1",
                "command": "pnpm test",
                "status": "inProgress",
            }),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    apply_notification(
        CodexNotification::TurnCompleted {
            turn_id: "turn-1".to_string(),
            status: "completed".to_string(),
            error_message: None,
            items: Vec::new(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );

    assert!(!prompt_completed);
    assert_eq!(active_turn_id.as_deref(), Some("turn-1"));
    assert!(completions.is_empty());

    apply_notification(
        CodexNotification::ItemCompleted {
            item: json!({
                "type": "commandExecution",
                "id": "cmd-1",
                "command": "pnpm test",
                "status": "completed",
                "aggregatedOutput": "ok",
                "exitCode": 0,
            }),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    flush_quiet_terminal_for_test(
        &mut active_turn_id,
        &mut turn_tracker,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
    );
    assert!(prompt_completed);
    assert_eq!(active_turn_id, None);
    assert_eq!(completions.len(), 1);

    apply_notification(
        CodexNotification::AgentMessageDelta {
            item_id: "msg-1".to_string(),
            delta: "done".to_string(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    apply_notification(
        CodexNotification::TurnCompleted {
            turn_id: "turn-1".to_string(),
            status: "completed".to_string(),
            error_message: None,
            items: Vec::new(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    flush_quiet_terminal_for_test(
        &mut active_turn_id,
        &mut turn_tracker,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
    );

    assert!(prompt_completed);
    assert_eq!(active_turn_id, None);
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].message_id, "codex-turn:turn-1");
}

#[test]
fn stale_turn_completion_before_tool_finish_does_not_settle_after_tool_finishes() {
    let mut active_turn_id = Some("turn-1".to_string());
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::ExecCommandStarted {
            call_id: "cmd-1".to_string(),
            command: json!("pnpm test"),
            cwd: None,
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    apply_notification(
        CodexNotification::TurnCompleted {
            turn_id: "stale-turn".to_string(),
            status: "completed".to_string(),
            error_message: None,
            items: Vec::new(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    apply_notification(
        CodexNotification::ExecCommandCompleted {
            call_id: "cmd-1".to_string(),
            command: json!("pnpm test"),
            cwd: None,
            output: Some("ok\n".to_string()),
            exit_code: Some(0),
            success: Some(true),
            stderr: None,
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    flush_quiet_terminal_for_test(
        &mut active_turn_id,
        &mut turn_tracker,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
    );

    assert!(!prompt_completed);
    assert_eq!(active_turn_id.as_deref(), Some("turn-1"));
    assert!(completions.is_empty());
}

#[test]
fn completed_assistant_item_after_tools_does_not_infer_prompt_completion() {
    let mut active_turn_id = Some("turn-1".to_string());
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::ExecCommandStarted {
            call_id: "cmd-1".to_string(),
            command: json!("echo ok"),
            cwd: None,
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    apply_notification(
        CodexNotification::ExecCommandCompleted {
            call_id: "cmd-1".to_string(),
            command: json!("echo ok"),
            cwd: None,
            output: Some("ok\n".to_string()),
            exit_code: Some(0),
            success: Some(true),
            stderr: None,
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    apply_notification(
        CodexNotification::ItemCompleted {
            item: json!({
                "type": "agentMessage",
                "id": "msg-1",
                "text": "done",
            }),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );

    assert!(!prompt_completed);
    assert_eq!(active_turn_id.as_deref(), Some("turn-1"));

    flush_quiet_terminal_for_test(
        &mut active_turn_id,
        &mut turn_tracker,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
    );

    assert!(!prompt_completed);
    assert_eq!(active_turn_id.as_deref(), Some("turn-1"));
    assert!(completions.is_empty());
}

#[test]
fn streamed_assistant_content_after_tools_does_not_infer_prompt_completion() {
    let mut active_turn_id = Some("turn-1".to_string());
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::ExecCommandStarted {
            call_id: "cmd-1".to_string(),
            command: json!("echo ok"),
            cwd: None,
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    apply_notification(
        CodexNotification::ExecCommandCompleted {
            call_id: "cmd-1".to_string(),
            command: json!("echo ok"),
            cwd: None,
            output: Some("ok\n".to_string()),
            exit_code: Some(0),
            success: Some(true),
            stderr: None,
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    apply_notification(
        CodexNotification::AgentMessageDelta {
            item_id: "msg-1".to_string(),
            delta: "done".to_string(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );

    assert!(!prompt_completed);
    flush_quiet_terminal_for_test(
        &mut active_turn_id,
        &mut turn_tracker,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
    );

    assert!(!prompt_completed);
    assert_eq!(active_turn_id.as_deref(), Some("turn-1"));
    assert!(completions.is_empty());
}

#[test]
fn terminal_with_final_assistant_text_settles_despite_stale_tool_tracking() {
    let mut active_turn_id = Some("turn-1".to_string());
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::ExecCommandStarted {
            call_id: "cmd-1".to_string(),
            command: json!("echo ok"),
            cwd: None,
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    apply_notification(
        CodexNotification::AgentMessageDelta {
            item_id: "msg-1".to_string(),
            delta: "```json\n{\"summary\":\"done\"}\n```".to_string(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    apply_notification(
        CodexNotification::TurnCompleted {
            turn_id: "turn-1".to_string(),
            status: "completed".to_string(),
            error_message: None,
            items: Vec::new(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    flush_quiet_terminal_for_test(
        &mut active_turn_id,
        &mut turn_tracker,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
    );

    assert!(prompt_completed);
    assert_eq!(active_turn_id, None);
    assert_eq!(completions.len(), 1);
}

#[test]
fn tool_start_after_assistant_content_still_requires_terminal_completion() {
    let mut active_turn_id = Some("turn-1".to_string());
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::ItemCompleted {
            item: json!({
                "type": "agentMessage",
                "id": "msg-1",
                "text": "I will inspect that.",
            }),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    apply_notification(
        CodexNotification::ItemStarted {
            item: json!({
                "type": "commandExecution",
                "id": "cmd-1",
                "command": "ls",
                "status": "inProgress",
            }),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    flush_quiet_terminal_for_test(
        &mut active_turn_id,
        &mut turn_tracker,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
    );

    assert!(!prompt_completed);
    assert_eq!(active_turn_id.as_deref(), Some("turn-1"));
    assert!(completions.is_empty());
}

#[test]
fn stale_turn_completion_does_not_complete_prompt() {
    let mut active_turn_id = Some("current-turn".to_string());
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::TurnCompleted {
            turn_id: "stale-turn".to_string(),
            status: "completed".to_string(),
            error_message: None,
            items: Vec::new(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );

    assert!(!prompt_completed);
    assert!(completions.is_empty());
    assert_eq!(active_turn_id.as_deref(), Some("current-turn"));
}

#[test]
fn interrupted_turn_is_treated_as_terminal_cancellation() {
    let mut active_turn_id = Some("turn-2".to_string());
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::TurnCompleted {
            turn_id: "turn-2".to_string(),
            status: "interrupted".to_string(),
            error_message: Some("Aborted".to_string()),
            items: Vec::new(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    flush_quiet_terminal_for_test(
        &mut active_turn_id,
        &mut turn_tracker,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
    );

    assert!(prompt_completed);
    assert_eq!(active_turn_id, None);
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].message_id, "codex-turn:turn-2");
    assert_eq!(notices, vec!["Aborted".to_string()]);
}

#[test]
fn legacy_aborted_turn_clears_unfinished_tools_and_settles() {
    let mut active_turn_id = Some("turn-legacy-abort".to_string());
    let mut turn_tracker = CodexTurnTracker::default();
    turn_tracker.note_tool_started("tool-still-running");
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::TurnAborted {
            reason: Some("interrupted".to_string()),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    flush_quiet_terminal_for_test(
        &mut active_turn_id,
        &mut turn_tracker,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
    );

    assert!(prompt_completed);
    assert_eq!(active_turn_id, None);
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].message_id, "codex-turn:turn-legacy-abort");
    assert_eq!(notices, vec!["interrupted".to_string()]);
}

#[test]
fn failed_turn_records_terminal_failure() {
    let mut active_turn_id = Some("turn-3".to_string());
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::TurnCompleted {
            turn_id: "turn-3".to_string(),
            status: "failed".to_string(),
            error_message: Some("model rejected".to_string()),
            items: Vec::new(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );
    flush_quiet_terminal_for_test(
        &mut active_turn_id,
        &mut turn_tracker,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
    );

    assert!(prompt_completed);
    assert_eq!(terminal_failure.as_deref(), Some("model rejected"));
    assert_eq!(notices, vec!["model rejected".to_string()]);
}

#[test]
fn error_notification_clears_active_turn_and_records_terminal_failure() {
    let mut active_turn_id = Some("turn-auth-failure".to_string());
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::Error {
            message: "unsupported model gpt-5.2-codex".to_string(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );

    assert!(prompt_completed);
    assert_eq!(active_turn_id, None);
    assert_eq!(
        terminal_failure.as_deref(),
        Some("unsupported model gpt-5.2-codex")
    );
    assert_eq!(notices, vec!["unsupported model gpt-5.2-codex".to_string()]);
}

#[test]
fn reconnect_progress_keeps_active_turn_running() {
    let mut active_turn_id = Some("turn-reconnecting".to_string());
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::Error {
            message: "Reconnecting... 2/5".to_string(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );

    assert!(!prompt_completed);
    assert_eq!(active_turn_id.as_deref(), Some("turn-reconnecting"));
    assert!(terminal_failure.is_none());
    assert!(notices.is_empty());
    assert_eq!(chunks.len(), 1);
    assert_eq!(
        chunks[0].kind,
        crate::terminal::TerminalOutputKind::ProviderStatus
    );
    assert_eq!(
        chunks[0].merge_key.as_deref(),
        Some(crate::provider::PROVIDER_CONNECTION_RETRY_MERGE_KEY)
    );
    assert_eq!(
        String::from_utf8_lossy(&chunks[0].bytes),
        "Codex connection interrupted — retrying (2/5)."
    );
}

#[test]
fn malformed_reconnect_error_remains_terminal() {
    let mut active_turn_id = Some("turn-reconnecting".to_string());
    let mut turn_tracker = CodexTurnTracker::default();
    let mut text_items = BTreeMap::new();
    let mut tool_items = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    apply_notification(
        CodexNotification::Error {
            message: "Reconnecting failed".to_string(),
        },
        &mut active_turn_id,
        &mut turn_tracker,
        &mut text_items,
        &mut tool_items,
        &mut chunks,
        &mut completions,
        &mut notices,
        &mut prompt_completed,
        &mut terminal_failure,
        &mut resolved_usage,
    );

    assert!(prompt_completed);
    assert_eq!(active_turn_id, None);
    assert_eq!(terminal_failure.as_deref(), Some("Reconnecting failed"));
}

#[test]
fn completed_turn_backfill_requires_final_answer_or_error_evidence() {
    let empty_items = Vec::new();
    assert!(!codex_completed_turn_has_settlement_evidence(
        Some(&empty_items),
        None
    ));
    assert!(!codex_completed_turn_has_settlement_evidence(None, None));
    assert!(!codex_completed_turn_has_settlement_evidence(
        Some(&vec![json!({
            "type": "task_started",
            "turn_id": "turn-1"
        })]),
        None
    ));
    assert!(codex_completed_turn_has_settlement_evidence(
        Some(&vec![json!({ "type": "agentMessage", "text": "done" })]),
        None
    ));
    assert!(!codex_completed_turn_has_settlement_evidence(
        Some(&vec![json!({
            "type": "agentMessage",
            "phase": "commentary",
            "text": "I am still working"
        })]),
        None
    ));
    assert!(codex_completed_turn_has_settlement_evidence(
        Some(&vec![json!({
            "type": "agentMessage",
            "phase": "finalAnswer",
            "text": "done"
        })]),
        None
    ));
    assert!(!codex_completed_turn_has_settlement_evidence(
        Some(&vec![json!({
            "type": "commandExecution",
            "status": "completed"
        })]),
        None
    ));
    assert!(codex_completed_turn_has_settlement_evidence(
        Some(&empty_items),
        Some("model rejected")
    ));
}

#[test]
fn managed_turn_backfills_after_completed_tool_and_final_output_without_terminal_notification() {
    use std::time::Duration;

    let mut turn_tracker = CodexTurnTracker::default();
    turn_tracker.note_tool_started("workflow-ack-call");
    turn_tracker.note_tool_completed("workflow-ack-call");
    turn_tracker.note_assistant_content();

    assert_eq!(turn_tracker.active_tool_count(), 0);
    assert!(!turn_tracker.has_pending_terminal());
    assert!(!codex_turn_should_backfill(
        crate::provider::AgentEndpointMode::Managed,
        true,
        &turn_tracker,
        false,
    ));
    assert!(!codex_turn_should_backfill(
        crate::provider::AgentEndpointMode::Managed,
        true,
        &turn_tracker,
        true,
    ));
    turn_tracker.force_assistant_evidence_quiet_for_tests(Duration::from_millis(249));
    assert!(!codex_turn_should_backfill(
        crate::provider::AgentEndpointMode::Managed,
        true,
        &turn_tracker,
        true,
    ));
    turn_tracker.force_assistant_evidence_quiet_for_tests(Duration::from_millis(250));
    assert!(codex_turn_should_backfill(
        crate::provider::AgentEndpointMode::Managed,
        true,
        &turn_tracker,
        true,
    ));
}

#[test]
fn managed_turn_backfills_after_completed_tool_when_final_message_has_no_item_event() {
    use std::time::Duration;

    let mut turn_tracker = CodexTurnTracker::default();
    turn_tracker.note_tool_started("review-call");
    turn_tracker.note_tool_completed("review-call");
    turn_tracker.force_assistant_evidence_quiet_for_tests(Duration::from_millis(250));

    assert!(turn_tracker.has_quiet_completed_tool_activity(Duration::from_millis(250)));
    assert!(codex_turn_should_backfill(
        crate::provider::AgentEndpointMode::Managed,
        true,
        &turn_tracker,
        true,
    ));
}

#[test]
fn managed_turn_backfills_after_quiet_final_output_without_terminal_notification() {
    use std::time::Duration;

    let mut turn_tracker = CodexTurnTracker::default();
    turn_tracker.note_assistant_content();
    turn_tracker.note_assistant_item_completed();
    turn_tracker.force_assistant_evidence_quiet_for_tests(Duration::from_millis(250));

    assert!(turn_tracker.has_terminal_assistant_evidence());
    assert!(codex_turn_should_backfill(
        crate::provider::AgentEndpointMode::Managed,
        true,
        &turn_tracker,
        true,
    ));
}

#[test]
fn managed_turn_does_not_backfill_from_pre_tool_commentary() {
    use std::time::Duration;

    let mut turn_tracker = CodexTurnTracker::default();
    turn_tracker.note_assistant_content();
    turn_tracker.force_assistant_evidence_quiet_for_tests(Duration::from_secs(1));

    assert!(!turn_tracker.has_terminal_assistant_evidence());
    assert!(!codex_turn_should_backfill(
        crate::provider::AgentEndpointMode::Managed,
        true,
        &turn_tracker,
        true,
    ));
}

#[test]
fn authoritative_backfill_is_due_even_when_the_notification_drain_is_not_quiet() {
    assert!(codex_authoritative_backfill_due(true, None));
    assert!(!codex_authoritative_backfill_due(false, None));
}
