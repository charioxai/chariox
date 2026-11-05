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
        let (_, queued) = self.prompt_state_owner.state_parts(&session, agent_id);
        let expired: Vec<_> = queued
            .into_iter()
            .filter(|prompt| prompt.notification_expired(now) && !prompt.remote_steer_reserved())
            .collect();
        if expired.is_empty() {
            return Ok(());
        }
        for prompt in expired {
            self.prompt_state_owner
                .remove_queued_prompt(&session, agent_id, prompt.id());
        }
        let (active, queued) = self.prompt_state_owner.state_parts(&session, agent_id);
        // The ordinary durable prompt transition also settles the associated App receipt.
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active, queued)
    }
}
