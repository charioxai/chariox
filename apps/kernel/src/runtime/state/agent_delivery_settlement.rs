//! A delivery whose provider receipt never came (a restart or a lost turn) settles in the
//! kernel: delivered when the turn took the event, otherwise sent again as the same event.
//! No person is asked about something only the kernel knows.
use super::*;
use crate::durable_state::agent_lifecycle::{InboxEvent, Operation, DELIVERY_TIMEOUT_MS};

impl KernelRuntimeState {
    /// Settles every delivery whose attempt began a delivery window ago and has no receipt.
    /// Each event is settled once: companions of one batch share their lead's receipt and are
    /// found settled when their turn comes. One event that cannot be settled never stops the
    /// others; the next lifecycle pass tries it again.
    pub(super) fn settle_interrupted_deliveries(&self, now: u64) -> Result<(), DaemonError> {
        let before = now.saturating_sub(DELIVERY_TIMEOUT_MS);
        let store = &self.owned.durable_state_store;
        for event in store.agent_interrupted_deliveries(before)? {
            let still_interrupted = store.agent_inbox_event(event.sequence).is_ok_and(|row| {
                row.is_some_and(|(row, _)| {
                    matches!(row.state.as_str(), "submitting" | "uncertain" | "blocked")
                })
            });
            if !still_interrupted {
                continue;
            }
            let state = if self.turn_took_delivery(&event) {
                "accepted"
            } else {
                "recovered"
            };
            let settled = store.agent_lifecycle(Operation::Receipt {
                room: event.room_id.clone(),
                agent: event.agent_id.clone(),
                sequence: event.sequence,
                state: state.into(),
                now,
            });
            if let Err(error) = settled {
                tracing::warn!(%error, sequence = event.sequence, room_id = %event.room_id,
                    "interrupted delivery could not be settled; the next sweep retries it");
            }
        }
        Ok(())
    }

    /// The turn took the event when its prompt is queued, running, or owns a task: from there
    /// the prompt queue and the task ledger carry it, and a second delivery would duplicate it.
    /// A steer belongs to the turn it targeted, which a restart ended, so it is sent again.
    fn turn_took_delivery(&self, event: &InboxEvent) -> bool {
        let Some(prompt) = event.prompt_id.as_deref() else {
            return false;
        };
        if event.target_prompt_id.is_some() {
            return false;
        }
        let queued_or_running = self
            .owned
            .session_store
            .get_session(&event.room_id)
            .is_ok_and(|session| {
                self.owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, &event.agent_id)
                    .is_some_and(|active| active.id() == prompt)
                    || session
                        .queued_prompts_for_agent(&event.agent_id)
                        .is_some_and(|queue| queue.iter().any(|item| item.id() == prompt))
            });
        queued_or_running
            || self
                .owned
                .durable_state_store
                .agent_tasks(Some(&event.room_id), Some(&event.agent_id))
                .is_ok_and(|tasks| tasks.iter().any(|task| task.prompt_id == prompt))
    }
}
