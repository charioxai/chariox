//! Notification retention follows queued prompts through recovery and manual steering.
use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn expire_queued_agent_notifications(
        &self,
        session_id: &str,
        agent_id: &str,
        now: u64,
    ) -> Result<(), DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        if self
            .prompt_state_owner
            .expire_queued_notifications(&session, agent_id, now)
            == 0
        {
            return Ok(());
        }
        let (active, queued) = self.prompt_state_owner.state_parts(&session, agent_id);
        // The ordinary durable prompt transition also settles the associated App receipt.
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active, queued)
    }
}
