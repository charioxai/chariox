use crate::error::DaemonError;
use crate::execution_lease::RemoteWorkflowTurnContext;
use crate::transport::relay_peer::{
    RemoteNativeInteractionContext, RemoteWorkspaceLiveSyncContext,
};

use super::RemoteLeaseRuntime;

impl<'a> RemoteLeaseRuntime<'a> {
    pub(crate) fn native_interaction_context_for_backing_agent(
        &mut self,
        backing_session_id: &str,
        backing_agent_id: &str,
        worker_provider_run_id: &str,
    ) -> Option<(String, RemoteNativeInteractionContext)> {
        let leased_agent = self
            .app
            .leased_agents
            .values()
            .find(|agent| {
                agent.backing_session_id == backing_session_id
                    && agent.backing_agent_id == backing_agent_id
            })?
            .clone();
        let lease = self
            .app
            .execution_leases
            .get(&leased_agent.lease_id)?
            .clone();
        Some((
            lease.home_kernel_id,
            RemoteNativeInteractionContext {
                home_session_id: lease.home_session_id,
                home_agent_id: lease.home_agent_id,
                leased_agent_id: leased_agent.id,
                worker_provider_run_id: worker_provider_run_id.to_string(),
            },
        ))
    }

    pub(crate) fn leased_agent_provider_run_id(
        &self,
        leased_agent_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        Ok(self
            .app
            .providers
            .get_run_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )
            .or_else(|| {
                self.app.providers.get_latest_run_for_agent(
                    &leased_agent.backing_session_id,
                    &leased_agent.backing_agent_id,
                )
            })
            .map(|run| run.id().to_string()))
    }

    pub(crate) fn leased_workflow_turn_context_for_provider_run(
        &self,
        provider_run_id: &str,
    ) -> Option<RemoteWorkflowTurnContext> {
        self.app
            .leased_workflow_turns
            .get(provider_run_id)
            .map(|binding| binding.context.clone())
    }

    pub(crate) fn leased_workspace_live_sync_context_for_provider_run(
        &self,
        provider_run_id: &str,
        worker_workspace_identity: crate::io::WorkspaceIdentity,
    ) -> Option<RemoteWorkspaceLiveSyncContext> {
        let leased_agent = self.app.leased_agents.values().find(|leased_agent| {
            self.app
                .providers
                .get_run_for_agent(
                    &leased_agent.backing_session_id,
                    &leased_agent.backing_agent_id,
                )
                .map(|run| run.id() == provider_run_id)
                .unwrap_or(false)
        })?;
        let lease = self.app.execution_leases.get(&leased_agent.lease_id)?;
        Some(RemoteWorkspaceLiveSyncContext {
            home_kernel_id: lease.home_kernel_id.clone(),
            home_session_id: lease.home_session_id.clone(),
            home_agent_id: lease.home_agent_id.clone(),
            leased_agent_id: leased_agent.id.clone(),
            worker_provider_run_id: provider_run_id.to_string(),
            worker_workspace_identity,
        })
    }
}
