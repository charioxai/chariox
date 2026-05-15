//! Codex notification projection and external-turn backfill.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::error::DaemonError;
use crate::provider::{CodexClient, CodexNotification, ProviderRunTokenUsage};
use crate::session::unix_epoch_ms;
use crate::terminal::TerminalOutputKind;

use super::transcript::{
    append_text_delta, append_tool_output_delta, append_tool_progress, codex_exec_command_item,
    decode_codex_output_delta_chunk, sync_completed_text_item, sync_tool_item,
    CodexTextTranscriptState, CodexToolTranscriptState,
};
use super::turn::{
    note_assistant_item_completed, note_tool_item_completed, note_tool_item_started,
    CodexTerminalSignal, CodexTurnTracker,
};
use super::{CodexAssistantCompletion, CodexOutputChunk, CodexRuntimeState};

pub(super) fn apply_notification(
    notification: CodexNotification,
    active_turn_id: &mut Option<String>,
    turn_tracker: &mut CodexTurnTracker,
    text_items: &mut BTreeMap<String, CodexTextTranscriptState>,
    tool_items: &mut BTreeMap<String, CodexToolTranscriptState>,
    chunks: &mut Vec<CodexOutputChunk>,
    _completions: &mut Vec<CodexAssistantCompletion>,
    notices: &mut Vec<String>,
    prompt_completed: &mut bool,
    terminal_failure: &mut Option<String>,
    resolved_usage: &mut Option<ProviderRunTokenUsage>,
) {
    match notification {
        CodexNotification::AgentMessageDelta { item_id, delta } => {
            if !delta.is_empty() {
                turn_tracker.note_assistant_content();
            } else {
                turn_tracker.note_activity();
            }
            append_text_delta(
                text_items,
                &item_id,
                "codex-agent-message",
                TerminalOutputKind::ProviderOutput,
                &delta,
                chunks,
            );
        }
        CodexNotification::ReasoningTextDelta { item_id, delta }
        | CodexNotification::ReasoningSummaryTextDelta { item_id, delta } => {
            turn_tracker.note_activity();
            append_text_delta(
                text_items,
                &item_id,
                "codex-reasoning",
                TerminalOutputKind::ProviderReasoning,
                &delta,
                chunks,
            );
        }
        CodexNotification::ReasoningSummaryPartAdded {
            item_id,
            summary_index,
        } => {
            turn_tracker.note_activity();
            if summary_index == 0 {
                return;
            }
            append_text_delta(
                text_items,
                &item_id,
                "codex-reasoning",
                TerminalOutputKind::ProviderReasoning,
                "\n\n",
                chunks,
            );
        }
        CodexNotification::ItemStarted { item } => {
            turn_tracker.note_activity();
            trace_codex_tool_item("item_lifecycle", &item);
            note_tool_item_started(turn_tracker, &item);
            if let Some(chunk) = sync_tool_item(tool_items, &item) {
                chunks.push(chunk);
            }
        }
        CodexNotification::ItemCompleted { item } => {
            turn_tracker.note_activity();
            trace_codex_tool_item("item_lifecycle", &item);
            note_tool_item_completed(turn_tracker, &item);
            note_assistant_item_completed(turn_tracker, &item);
            if let Some(chunk) = sync_tool_item(tool_items, &item) {
                chunks.push(chunk);
            } else if let Some(chunk) = sync_completed_text_item(text_items, &item) {
                chunks.push(chunk);
            }
        }
        CodexNotification::ExecCommandStarted {
            call_id,
            command,
            cwd,
        } => {
            turn_tracker.note_tool_started(&call_id);
            if let Some(chunk) = sync_tool_item(
                tool_items,
                &codex_exec_command_item(&call_id, command, cwd, None),
            ) {
                chunks.push(chunk);
            }
        }
        CodexNotification::ExecCommandCompleted {
            call_id,
            command,
            cwd,
            output,
            exit_code,
            success,
            stderr,
        } => {
            turn_tracker.note_tool_completed(&call_id);
            let mut item = codex_exec_command_item(&call_id, command, cwd, exit_code);
            item["status"] = json!(if success == Some(false) {
                "failed"
            } else {
                "completed"
            });
            if let Some(output) = output.or(stderr) {
                item["aggregatedOutput"] = json!(output);
            }
            if let Some(exit_code) = exit_code {
                item["exitCode"] = json!(exit_code);
            }
            if let Some(chunk) = sync_tool_item(tool_items, &item) {
                chunks.push(chunk);
            }
        }
        CodexNotification::ExecCommandOutputDelta { call_id, chunk } => {
            turn_tracker.note_activity();
            let delta = decode_codex_output_delta_chunk(&chunk);
            if let Some(chunk) =
                append_tool_output_delta(tool_items, &call_id, "commandExecution", &delta)
            {
                chunks.push(chunk);
            }
        }
        CodexNotification::CommandExecutionOutputDelta { item_id, delta } => {
            turn_tracker.note_activity();
            if let Some(chunk) =
                append_tool_output_delta(tool_items, &item_id, "commandExecution", &delta)
            {
                chunks.push(chunk);
            }
        }
        CodexNotification::FileChangeOutputDelta { item_id, delta } => {
            turn_tracker.note_activity();
            if let Some(chunk) =
                append_tool_output_delta(tool_items, &item_id, "fileChange", &delta)
            {
                chunks.push(chunk);
            }
        }
        CodexNotification::McpToolCallProgress { item_id, message } => {
            turn_tracker.note_tool_started(&item_id);
            turn_tracker.note_activity();
            if let Some(chunk) = append_tool_progress(tool_items, &item_id, &message) {
                chunks.push(chunk);
            }
        }
        CodexNotification::TokenUsageUpdated { usage, .. } => {
            *resolved_usage = Some(usage);
        }
        CodexNotification::TurnStarted { turn_id } => {
            crate::logging::debug_with_fields(
                "daemon.provider.codex",
                "codex turn started trace",
                json!({
                    "previous_active_turn_id": active_turn_id,
                    "turn_id": turn_id,
                }),
            );
            if !turn_id.is_empty() {
                *active_turn_id = Some(turn_id);
            }
            turn_tracker.reset_for_started();
        }
        CodexNotification::TurnCompleted {
            turn_id,
            status,
            error_message,
        } => {
            if active_turn_id.as_deref() != Some(turn_id.as_str()) {
                crate::logging::debug_with_fields(
                    "daemon.provider.codex",
                    "codex turn completion ignored by active turn mismatch",
                    json!({
                        "active_turn_id": active_turn_id,
                        "turn_id": turn_id,
                        "status": status,
                        "error_message": error_message,
                    }),
                );
                return;
            }
            crate::logging::debug_with_fields(
                "daemon.provider.codex",
                "codex turn completion candidate accepted",
                json!({
                    "turn_id": turn_id,
                    "status": status,
                    "active_tool_count": turn_tracker.active_tool_count(),
                }),
            );
            turn_tracker.note_terminal(CodexTerminalSignal {
                turn_id,
                status,
                error_message,
            });
        }
        CodexNotification::Error { message } => {
            *terminal_failure = Some(message.clone());
            *prompt_completed = true;
            notices.push(message);
        }
    }
}

pub(super) fn backfill_external_completed_turn(
    client: &CodexClient,
    state: &mut CodexRuntimeState,
    chunks: &mut Vec<CodexOutputChunk>,
    completions: &mut Vec<CodexAssistantCompletion>,
    notices: &mut Vec<String>,
    prompt_completed: &mut bool,
    terminal_failure: &mut Option<String>,
) -> Result<(), DaemonError> {
    let Some(active_turn_id) = state.active_turn_id.clone() else {
        return Ok(());
    };
    let thread_id = state.thread_id().to_string();
    let response = client.thread_turns_list(
        &mut state.socket,
        &mut state.next_request_id,
        &thread_id,
        &mut state.buffered_notifications,
    )?;
    let Some(turn) = response
        .get("data")
        .and_then(Value::as_array)
        .and_then(|turns| {
            turns.iter().find(|turn| {
                turn.get("id").and_then(Value::as_str) == Some(active_turn_id.as_str())
            })
        })
    else {
        return Ok(());
    };
    let status = turn
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !matches!(status, "completed" | "failed" | "cancelled" | "canceled") {
        return Ok(());
    }
    if let Some(items) = turn.get("items").and_then(Value::as_array) {
        for item in items {
            if let Some(chunk) = sync_tool_item(&mut state.tool_items, item) {
                chunks.push(chunk);
            } else if let Some(chunk) = sync_completed_text_item(&mut state.text_items, item) {
                chunks.push(chunk);
            }
        }
    }
    if status == "failed" {
        *terminal_failure =
            Some(codex_turn_error_message(turn).unwrap_or_else(|| "Codex turn failed".to_string()));
    }
    if let Some(message) = codex_turn_error_message(turn) {
        notices.push(message);
    }
    completions.push(CodexAssistantCompletion {
        message_id: format!("codex-turn:{active_turn_id}"),
        completed_at_ms: codex_turn_completed_at_ms(turn).unwrap_or_else(unix_epoch_ms),
    });
    *prompt_completed = true;
    state.active_turn_id = None;
    Ok(())
}

fn codex_turn_completed_at_ms(turn: &Value) -> Option<u64> {
    turn.get("completedAt")
        .and_then(Value::as_u64)
        .map(|seconds| seconds.saturating_mul(1_000))
}

fn codex_turn_error_message(turn: &Value) -> Option<String> {
    let error = turn.get("error")?;
    if error.is_null() {
        return None;
    }
    if let Some(message) = error.as_str() {
        return (!message.is_empty()).then(|| message.to_string());
    }
    error
        .get("message")
        .or_else(|| error.get("details"))
        .and_then(Value::as_str)
        .filter(|message| !message.is_empty())
        .map(str::to_string)
}

fn trace_codex_tool_item(label: &str, item: &Value) {
    crate::logging::debug_with_fields(
        "daemon.provider.codex",
        "codex tool item trace",
        json!({
            "label": label,
            "id": item.get("id").and_then(Value::as_str),
            "type": item.get("type").and_then(Value::as_str),
            "status": item.get("status").and_then(Value::as_str),
            "command": item.get("command").and_then(Value::as_str),
            "exit_code": item.get("exitCode").and_then(Value::as_i64),
            "process_id": item.get("processId").and_then(Value::as_str),
            "aggregated_output_len": item.get("aggregatedOutput").and_then(Value::as_str).map(str::len),
        }),
    );
}
