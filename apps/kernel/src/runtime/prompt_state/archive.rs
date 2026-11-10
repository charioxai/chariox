//! A launch commits under the prompt owner's lock, so it cannot start for an agent whose archived flag is set.
use super::*;

impl PromptStateOwner {
    pub(crate) fn agent_is_archived(&self, session: &RuntimeSession, agent_id: &str) -> bool {
        session
            .agents()
            .iter()
            .any(|agent| agent.id() == agent_id && agent.archived())
    }

    /// The callback must not reenter this owner or await provider I/O.
    pub(crate) fn with_unarchived_agent<T>(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        commit: impl FnOnce() -> Result<T, DaemonError>,
    ) -> Result<T, DaemonError> {
        let _owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if session
            .agents()
            .iter()
            .any(|agent| agent.id() == agent_id && agent.archived())
        {
            return Err(DaemonError::LocalTransport {
                operation: "launch provider",
                message: "Unarchive this agent before starting its provider.".into(),
            });
        }
        commit()
    }
}
