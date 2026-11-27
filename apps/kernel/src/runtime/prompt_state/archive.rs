//! A launch refuses an agent whose archived flag is set.
use super::*;

impl PromptStateOwner {
    pub(crate) fn agent_is_archived(&self, session: &RuntimeSession, agent_id: &str) -> bool {
        session
            .agents()
            .iter()
            .any(|agent| agent.id() == agent_id && agent.archived())
    }

    /// A launch refuses an archived agent. It checks without holding the owner across its
    /// planning: the lock order is sessions, then prompts, and a launch sets the session
    /// pointer afterwards. Only tests archive agents.
    pub(crate) fn ensure_unarchived_agent(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        if self.agent_is_archived(session, agent_id) {
            return Err(DaemonError::LocalTransport {
                operation: "launch provider",
                message: "Unarchive this agent before starting its provider.".into(),
            });
        }
        Ok(())
    }

    /// Holds the owner while `commit` runs: a test stands in for any holder that waits for
    /// the session store, which the kernel's lock order (sessions, then prompts) forbids.
    #[cfg(test)]
    pub(crate) fn with_unarchived_agent<T>(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        commit: impl FnOnce() -> Result<T, DaemonError>,
    ) -> Result<T, DaemonError> {
        let _owner = crate::runtime::checked_lock::lock_watched("prompt_state_owner", &self.state)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.ensure_unarchived_agent(session, agent_id)?;
        commit()
    }
}
