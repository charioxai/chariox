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
            self.baseline = difference(*total, *last);
        }
        self.latest = Some(*total);
        difference(*total, self.baseline?)
    }
}

fn difference(total: ModelTokenCounts, earlier: ModelTokenCounts) -> Option<ModelTokenCounts> {
    let tokens = ModelTokenCounts {
        input_tokens: total.input_tokens.checked_sub(earlier.input_tokens)?,
        output_tokens: total.output_tokens.checked_sub(earlier.output_tokens)?,
        cached_input_tokens: total
            .cached_input_tokens
            .checked_sub(earlier.cached_input_tokens)?,
        cache_write_input_tokens: total
            .cache_write_input_tokens
            .checked_sub(earlier.cache_write_input_tokens)?,
    };
    tokens
        .input_tokens
        .checked_sub(tokens.cached_input_tokens)?
        .checked_sub(tokens.cache_write_input_tokens)?;
    Some(tokens)
}
