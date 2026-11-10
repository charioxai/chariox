//! A cancel that an agent issues (a peer, a parent) ends the target's task, not the target: unlike
//! the owner's stop, later messages still run. A turn's task settles as cancelled only after the
//! provider aborts, so whoever began the cancellation is recorded on the task at that moment.
use super::*;
use crate::durable_state::agent_lifecycle::{CancelInitiator, Operation};

/// The seats the kernel opens for agents that cancel through the room commands and the peer
/// protocol; an owner's client never uses them.
fn is_agent_seat(client_id: &str) -> bool {
    client_id.starts_with("agent:") || client_id.starts_with("metaagent:")
}

impl KernelRuntimeOwnedState {
    pub(super) fn cancel_initiator(&self, attachment_id: &str) -> CancelInitiator {
        match self.attachment_store.get_attachment(attachment_id) {
            Ok(attachment) if is_agent_seat(attachment.client_id()) => CancelInitiator::Agent,
            _ => CancelInitiator::Owner,
        }
    }

    /// Records who stopped the turn of `prompt`. `began`: this call moved the prompt to
    /// cancelling; an agent that did not begin it never claims it, while the owner's Stop on a
    /// turn an agent began takes it over, so the owner wins in either order.
    pub(super) fn note_turn_cancel(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
        attachment_id: &str,
        began: bool,
    ) {
        let initiator = self.cancel_initiator(attachment_id);
        if began != (initiator == CancelInitiator::Agent) {
            return;
        }
        let noted = self
            .durable_state_store
            .agent_turn_or_continuation_task(room, agent, prompt)
            .and_then(|task| match task {
                Some(task) => self
                    .durable_state_store
                    .agent_lifecycle(Operation::NoteTurnCancel {
                        task: task.task_id,
                        initiator,
                    })
                    .map(drop),
                None => Ok(()),
            });
        if let Err(error) = noted {
            crate::logging::warn_with_fields(
                "daemon.agent_task",
                "turn cancel initiator not recorded",
                serde_json::json!({"session_id": room, "agent_id": agent, "prompt_id": prompt, "error": error.to_string()}),
            );
        }
    }
}
