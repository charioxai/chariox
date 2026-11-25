//! The Room environment needs no person switch for the owner's agents: only an archived agent is refused.
use super::*;
impl KernelRuntimeState {
    pub(super) fn require_room_computer_actor(&self, actor: &str) -> Result<(), DaemonError> {
        if let Some(agent_id) = actor.strip_prefix("agent:") {
            if self.owned.agent_store.get_agent(agent_id)?.archived() {
                return Err(DaemonError::LocalTransport {
                    operation: "computer input",
                    message: "Unarchive this agent before using the Room environment.".into(),
                });
            }
        }
        Ok(())
    }
}
impl KernelRuntimeOwnedState {
    /// A removed agent id never returns, so the person's leased desktop grant is dropped with it.
    pub(super) fn forget_leased_computer_grant(&self, agent_id: &str) {
        self.leased_computer_grants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(agent_id);
    }
}
