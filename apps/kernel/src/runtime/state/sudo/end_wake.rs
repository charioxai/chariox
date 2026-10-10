//! MP-08/MP-10/MP-11: expiry wakes retry independently of removed authority.
use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, ExecutionState, Operation};

impl KernelRuntimeState {
    pub(super) fn queue_sudo_end_wake(&self, turn: &KernelSudoTurn, reason: &str) {
        if turn.task_id.is_none() {
            return;
        }
        self.owned
            .sudo_end_wakes
            .lock()
            .expect("sudo end wakes poisoned")
            .entry(turn.entry_id.clone())
            .or_insert_with(|| (turn.clone(), reason.to_owned()));
        if self.try_sudo_end_wake(turn, reason).is_err() {
            self.owned.record_notice_for_agent(&turn.session_id, None, Some(&turn.agent_id),
                self.owned.attachment_store.list_session_attachment_ids(&turn.session_id),
                "Alert: sudo authority ended, but its waiting-work wake could not be persisted. The kernel will retry; elevation remains unavailable.");
        }
    }

    fn try_sudo_end_wake(&self, turn: &KernelSudoTurn, reason: &str) -> Result<(), DaemonError> {
        let waiting = self
            .owned
            .durable_state_store
            .agent_tasks(Some(&turn.session_id), Some(&turn.agent_id))?
            .iter()
            .any(|task| {
                Some(task.task_id.as_str()) == turn.task_id.as_deref()
                    && task.state == ExecutionState::Waiting
            });
        if waiting {
            self.owned.durable_state_store.agent_lifecycle(Operation::Occur(ledger::occurrence(
                &turn.session_id, &turn.agent_id, &turn.entry_id,
                &format!("{}:{}:{reason}", turn.entry_id, turn.revision), "sudo_ended",
                serde_json::json!({"task_id": turn.task_id, "reason": reason,
                    "message": "Your sudo window ended. Re-evaluate the work as a regular agent; ask the owner to re-elevate if a privileged step remains."}),
            )))?;
        }
        self.owned
            .sudo_end_wakes
            .lock()
            .expect("sudo end wakes poisoned")
            .remove(&turn.entry_id);
        Ok(())
    }

    pub(super) fn retry_sudo_end_wakes(&self) {
        let pending: Vec<_> = self
            .owned
            .sudo_end_wakes
            .lock()
            .expect("sudo end wakes poisoned")
            .values()
            .cloned()
            .collect();
        for (turn, reason) in pending {
            let _ = self.try_sudo_end_wake(&turn, &reason);
        }
    }
}
