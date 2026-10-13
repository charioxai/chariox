//! MP-08 / MP-10 / MP-11: every refresh persists replay intent before binding publication.
use super::*;

impl DaemonApp {
    pub(super) fn commit_recreated_remote_execution(
        &mut self,
        agent: &AgentInstance,
        binding: RemoteAgentBinding,
    ) -> Result<AgentInstance, DaemonError> {
        self.operational_history_store()
            .mark_remote_lease_context_pending(agent.session_id(), agent.id(), &binding)?;
        self.agents.bind_remote_execution(agent.id(), binding)
    }
}
