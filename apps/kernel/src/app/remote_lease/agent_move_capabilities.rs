//! Move uses the ordinary worker capability checks before it admits execution.
use super::RemoteLeaseRuntime;
use crate::agent_move::AgentMoveRequirements;
use crate::error::DaemonError;

impl RemoteLeaseRuntime<'_> {
    pub(crate) fn prepare_agent_move_capabilities(
        &mut self,
        identity: &crate::agent_move::AgentMoveIdentity,
        leased_agent_id: &str,
        requirements: &AgentMoveRequirements,
    ) -> Result<(), DaemonError> {
        let leased = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.into(),
            })?;
        let target = self
            .app
            .agent_moves
            .target(&identity.operation_id)
            .ok_or_else(|| crate::agent_move::error("Move target is missing"))?;
        let lease = self
            .app
            .execution_leases
            .get(&leased.lease_id)
            .ok_or_else(|| crate::agent_move::error("Move backing lease is missing"))?;
        if target.active
            || target.retired
            || target.capture.identity != *identity
            || target.execution_lease_id.as_deref() != Some(leased.lease_id.as_str())
            || lease.home_kernel_id != identity.home_kernel_id
            || lease.home_session_id != identity.home_session_id
            || lease.owner_user_id != identity.owner_user_id
            || leased.home_agent_id != identity.home_agent_id
        {
            return Err(crate::agent_move::error(
                "Move capabilities require this generation's exact backing lease",
            ));
        }
        self.ensure_home_proxy_manifest_has_no_worker_collisions(
            &leased,
            &[],
            &requirements.extensions,
            "prepare agent Move",
        )
    }
}
