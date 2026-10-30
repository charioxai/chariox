//! Hot reload: a running Codex turn continues in the next kernel program. Its
//! app-server keeps running the turn while no client is connected, and
//! `thread/resume` on a new connection rejoins it (measured with codex-cli
//! 0.159.3), but what it streamed meanwhile is not replayed. So a text item
//! the new program did not see start takes its text from the completed item,
//! after the part the previous program already showed.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::provider::CodexNotification;

use super::transcript::{codex_item_id, CodexTextTranscriptState};
use super::CodexRuntimeState;

/// What the next kernel program needs to continue a Codex turn; the
/// app-server process itself is handed over as a PTY.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct CodexHandover {
    endpoint: String,
    thread_id: String,
    turn_id: String,
    text_items: BTreeMap<String, CodexTextTranscriptState>,
}

impl CodexHandover {
    pub(crate) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub(super) fn thread_id(&self) -> &str {
        &self.thread_id
    }
}

/// The run's turn, when one is running on a thread the app-server keeps.
pub(crate) fn codex_turn_handover(state: &CodexRuntimeState) -> Option<CodexHandover> {
    let turn_id = state.active_turn_id.clone()?;
    if !state.thread_ready() || state.ephemeral || state.notification_only {
        return None;
    }
    Some(CodexHandover {
        endpoint: state.endpoint().to_string(),
        thread_id: state.thread_id().to_string(),
        turn_id,
        text_items: state.text_items.clone(),
    })
}

/// Continues the handed-over turn on this program's connection, once the
/// thread is resumed on it.
pub(super) fn continue_turn(state: &mut CodexRuntimeState, turn: CodexHandover) {
    state.turn_tracker.reset_for_submitted();
    state.turn_tracker.provider_active_turn_id = Some(turn.turn_id.clone());
    state.turn_tracker.reset_for_started();
    state.active_turn_id = Some(turn.turn_id);
    state.text_items = turn.text_items;
    state.resumed_turn = Some(ResumedTurn::default());
}

/// The text items of a resumed turn whose start this program saw: only their
/// deltas are complete.
#[derive(Debug, Default)]
pub(super) struct ResumedTurn {
    started_here: BTreeSet<String>,
}

impl ResumedTurn {
    pub(super) fn skips(&mut self, notification: &CodexNotification) -> bool {
        match notification {
            CodexNotification::TurnScoped { notification, .. } => self.skips(notification),
            CodexNotification::ItemStarted { item } => {
                if let Some(item_id) = codex_item_id(item) {
                    self.started_here.insert(item_id.to_string());
                }
                false
            }
            CodexNotification::AgentMessageDelta { item_id, .. }
            | CodexNotification::ReasoningTextDelta { item_id, .. }
            | CodexNotification::ReasoningSummaryTextDelta { item_id, .. }
            | CodexNotification::ReasoningSummaryPartAdded { item_id, .. } => {
                !self.started_here.contains(item_id)
            }
            _ => false,
        }
    }
}
