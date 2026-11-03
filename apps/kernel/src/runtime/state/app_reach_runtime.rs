//! The Apps an agent can use follow its Room, not a per-agent grant.

use super::KernelRuntimeState;
use crate::agent::AgentInstance;
use crate::runtime::app_control::AppReach;

impl KernelRuntimeState {
    pub(crate) fn app_reach(&self, agent: &AgentInstance) -> AppReach {
        let session = self
            .owned
            .session_store
            .get_session(agent.session_id())
            .ok();
        AppReach::for_agent(agent, session.as_ref())
    }
}

impl KernelRuntimeState {
    /// Whether the agent reaches this installation.
    pub(crate) async fn agent_reaches_app(
        &self,
        agent: &AgentInstance,
        installation: &str,
    ) -> bool {
        let reach = self.app_reach(agent);
        let Ok(permit) = self.app_control().try_admit() else {
            return false;
        };
        let control = self.app_control().clone();
        let installation = installation.to_owned();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            control.can_reach(&reach, &installation)
        })
        .await
        .unwrap_or(false)
    }
}
