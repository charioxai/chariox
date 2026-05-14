//! Codex turn settlement tracking and terminal completion gating.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::session::unix_epoch_ms;

use super::transcript::{
    codex_item_id, codex_item_status_is_terminal, is_codex_tool_item, normalize_codex_item_type,
    text_from_content_value,
};
use super::CodexAssistantCompletion;

#[derive(Debug, Clone, Default)]
pub(super) struct CodexTurnTracker {
    active_tool_ids: BTreeSet<String>,
    pending_terminal: Option<CodexPendingTerminal>,
    tool_started: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CodexTerminalSignal {
    pub(super) turn_id: String,
    pub(super) status: String,
    pub(super) error_message: Option<String>,
}

#[derive(Debug, Clone)]
struct CodexPendingTerminal {
    signal: CodexTerminalSignal,
}

impl CodexTurnTracker {
    pub(super) fn reset_for_started(&mut self) {
        self.active_tool_ids.clear();
        self.pending_terminal = None;
        self.tool_started = false;
    }

    pub(super) fn note_tool_started(&mut self, tool_id: &str) {
        if !tool_id.is_empty() {
            self.active_tool_ids.insert(tool_id.to_string());
        }
        self.tool_started = true;
        self.note_activity();
    }

    pub(super) fn note_tool_completed(&mut self, tool_id: &str) {
        if !tool_id.is_empty() {
            self.active_tool_ids.remove(tool_id);
        }
        self.note_activity();
    }

    pub(super) fn note_terminal(&mut self, signal: CodexTerminalSignal) {
        self.pending_terminal = Some(CodexPendingTerminal { signal });
    }

    pub(super) fn note_activity(&mut self) {}

    pub(super) fn note_assistant_content(&mut self) {
        self.note_activity();
    }

    pub(super) fn has_pending_terminal(&self) -> bool {
        self.pending_terminal.is_some()
    }

    pub(super) fn active_tool_count(&self) -> usize {
        self.active_tool_ids.len()
    }

    #[cfg(test)]
    pub(super) fn force_pending_terminal_quiet_for_tests(&mut self) {}
}

pub(super) fn note_tool_item_started(turn_tracker: &mut CodexTurnTracker, item: &Value) {
    if !is_codex_tool_item(item) {
        return;
    }
    if codex_item_status_is_terminal(item) {
        return;
    }
    if let Some(item_id) = codex_item_id(item) {
        turn_tracker.note_tool_started(item_id);
    }
}

pub(super) fn note_tool_item_completed(turn_tracker: &mut CodexTurnTracker, item: &Value) {
    if !is_codex_tool_item(item) {
        return;
    }
    if let Some(item_id) = codex_item_id(item) {
        turn_tracker.note_tool_completed(item_id);
    }
}

pub(super) fn note_assistant_item_completed(
    turn_tracker: &mut CodexTurnTracker,
    item: &Value,
) -> bool {
    let item_type = item
        .get("type")
        .and_then(Value::as_str)
        .and_then(normalize_codex_item_type)
        .unwrap_or_default();
    if item_type != "agentMessage" {
        return false;
    }
    let has_text = item
        .get("text")
        .and_then(Value::as_str)
        .is_some_and(|text| !text.is_empty())
        || text_from_content_value(item.get("content")).is_some_and(|text| !text.is_empty());
    if has_text {
        turn_tracker.note_assistant_content();
    }
    has_text
}

pub(super) fn maybe_finalize_terminal_signal(
    active_turn_id: &mut Option<String>,
    turn_tracker: &mut CodexTurnTracker,
    completions: &mut Vec<CodexAssistantCompletion>,
    notices: &mut Vec<String>,
    prompt_completed: &mut bool,
    terminal_failure: &mut Option<String>,
) {
    if turn_tracker.active_tool_count() > 0 {
        return;
    }
    if !turn_tracker.has_pending_terminal() {
        return;
    }
    let Some(pending) = turn_tracker.pending_terminal.take() else {
        return;
    };
    let signal = pending.signal;
    let completion = CodexAssistantCompletion {
        message_id: format!("codex-turn:{}", signal.turn_id),
        completed_at_ms: unix_epoch_ms(),
    };
    completions.push(completion);
    if signal.status == "failed" {
        *terminal_failure = Some(
            signal
                .error_message
                .clone()
                .unwrap_or_else(|| "Codex turn failed".to_string()),
        );
    }
    if let Some(message) = signal
        .error_message
        .clone()
        .or_else(|| (signal.status == "failed").then(|| "Codex turn failed".to_string()))
    {
        notices.push(message);
    }
    *prompt_completed = true;
    *active_turn_id = None;
}
