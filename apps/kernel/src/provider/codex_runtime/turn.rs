//! Codex turn settlement tracking and terminal completion gating.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

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
    legacy_completion_hint: bool,
    tool_started: bool,
    assistant_content_observed: bool,
    assistant_item_completed: bool,
    assistant_content_after_tool_activity: bool,
    last_activity_at: Option<Instant>,
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
        self.legacy_completion_hint = false;
        self.tool_started = false;
        self.assistant_content_observed = false;
        self.assistant_item_completed = false;
        self.assistant_content_after_tool_activity = false;
        self.last_activity_at = Some(Instant::now());
    }

    pub(super) fn note_tool_started(&mut self, tool_id: &str) {
        self.note_activity();
        if !tool_id.is_empty() {
            self.active_tool_ids.insert(tool_id.to_string());
        }
        self.tool_started = true;
        self.assistant_content_after_tool_activity = false;
    }

    pub(super) fn note_tool_completed(&mut self, tool_id: &str) {
        self.note_activity();
        if !tool_id.is_empty() {
            self.active_tool_ids.remove(tool_id);
        }
        self.assistant_content_after_tool_activity = false;
    }

    pub(super) fn note_terminal(&mut self, signal: CodexTerminalSignal) {
        self.note_activity();
        self.pending_terminal = Some(CodexPendingTerminal { signal });
    }

    pub(super) fn note_legacy_completion_hint(&mut self) {
        self.note_activity();
        self.legacy_completion_hint = true;
    }

    pub(super) fn has_legacy_completion_hint(&self) -> bool {
        self.legacy_completion_hint
    }

    pub(super) fn clear_legacy_completion_hint(&mut self) {
        self.legacy_completion_hint = false;
    }

    pub(super) fn note_activity(&mut self) {
        self.last_activity_at = Some(Instant::now());
    }

    pub(super) fn note_assistant_content(&mut self) {
        self.note_activity();
        self.assistant_content_observed = true;
        if self.tool_started {
            self.assistant_content_after_tool_activity = true;
        }
    }

    pub(super) fn note_assistant_item_completed(&mut self) {
        self.assistant_item_completed = true;
    }

    pub(super) fn has_pending_terminal(&self) -> bool {
        self.pending_terminal.is_some()
    }

    pub(super) fn active_tool_count(&self) -> usize {
        self.active_tool_ids.len()
    }

    pub(super) fn has_terminal_assistant_evidence(&self) -> bool {
        // A managed turn can finish with a plain assistant answer and no tool call.
        // When tools did run, require content after the tool activity so commentary
        // emitted before the tool cannot trigger an authoritative completion lookup.
        self.assistant_content_observed
            && ((!self.tool_started && self.assistant_item_completed)
                || self.assistant_content_after_tool_activity)
    }

    pub(super) fn has_quiet_terminal_assistant_evidence(&self, quiet_for: Duration) -> bool {
        self.has_terminal_assistant_evidence()
            && self
                .last_activity_at
                .is_some_and(|last_activity_at| last_activity_at.elapsed() >= quiet_for)
    }

    /// A managed app-server can finish a turn without emitting the legacy
    /// `turn/completed` notification.  A completed tool followed by a quiet
    /// socket is still safe evidence to ask the server for the authoritative
    /// turn record; `backfill_completed_turn` only settles when that record is
    /// terminal and contains final assistant output (or an error).
    pub(super) fn has_quiet_completed_tool_activity(&self, quiet_for: Duration) -> bool {
        self.tool_started
            && self.active_tool_count() == 0
            && self
                .last_activity_at
                .is_some_and(|last_activity_at| last_activity_at.elapsed() >= quiet_for)
    }

    #[cfg(test)]
    pub(super) fn force_pending_terminal_quiet_for_tests(&mut self) {}

    #[cfg(test)]
    pub(super) fn force_assistant_evidence_quiet_for_tests(&mut self, quiet_for: Duration) {
        self.last_activity_at = Instant::now().checked_sub(quiet_for);
    }
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
        turn_tracker.note_assistant_item_completed();
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
    let Some(pending_turn_id) = turn_tracker
        .pending_terminal
        .as_ref()
        .map(|pending| pending.signal.turn_id.as_str())
    else {
        return;
    };
    if active_turn_id.as_deref() != Some(pending_turn_id) {
        turn_tracker.pending_terminal = None;
        return;
    }
    if turn_tracker.active_tool_count() > 0 && !turn_tracker.has_terminal_assistant_evidence() {
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
