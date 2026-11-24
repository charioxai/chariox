use serde::{Deserialize, Serialize};

use crate::provider::{CodexNotification, ModelTokenCounts};

/// Codex reports a thread lifetime total and the most recent model request.
/// Keep the baseline across polls and across a hot handover of the same turn.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct CodexUsageTracker {
    turn_id: String,
    baseline: Option<ModelTokenCounts>,
    latest: Option<ModelTokenCounts>,
}

impl CodexUsageTracker {
    pub(super) fn observe(
        &mut self,
        notification: &CodexNotification,
        expected_thread: &str,
        active_turn: Option<&str>,
    ) -> Option<ModelTokenCounts> {
        let CodexNotification::TokenUsageUpdated {
            thread_id,
            turn_id,
            counters: Some((total, last)),
            ..
        } = notification
        else {
            return None;
        };
        if thread_id != expected_thread || Some(turn_id.as_str()) != active_turn {
            return None;
        }
        if self.latest == Some(*total) {
            return None;
        }
        if self.turn_id != *turn_id {
            self.turn_id.clone_from(turn_id);
            // A reopened thread can already have usage from earlier turns.
            self.baseline = total.checked_delta_from(*last);
        }
        self.latest = Some(*total);
        total.checked_delta_from(self.baseline?)
    }
}
