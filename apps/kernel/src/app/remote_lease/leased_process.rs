use super::*;

// MP-08/MP-09/MP-10/MP-11 A10: execution coordinates are derived from the current lease.
impl RemoteLeaseRuntime<'_> {
    pub(crate) fn leased_process_target(
        &mut self,
        binding: &crate::durable_state::agent_lifecycle::LeasedProcessBinding,
        wake: &crate::durable_state::agent_lifecycle::AgentWake,
        starting: bool,
    ) -> Result<(String, String, Option<String>), DaemonError> {
        let agent = self
            .app
            .leased_agents
            .get(&binding.leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: binding.leased_agent_id.clone(),
            })?;
        let lease = self
            .app
            .execution_leases
            .get(&agent.lease_id)
            .cloned()
            .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                lease_id: agent.lease_id.clone(),
            })?;
        if agent.lease_id != binding.execution_lease_id
            || lease.worker_kernel_id != binding.worker_kernel_id
            || lease.machine_id != binding.worker_machine_id
            || lease.home_session_id != wake.room_id
            || lease.home_agent_id != wake.agent_id
            || agent.home_agent_id != wake.agent_id
            || self.app.config.daemon_id != binding.worker_kernel_id
            || self.app.config.host_machine_id != binding.worker_machine_id
        {
            return Err(crate::durable_state::agent_lifecycle::error(
                "leased process placement or origin is no longer current",
            ));
        }
        if starting
            && (agent.active_home_prompt_id.as_deref() != Some(binding.home_prompt_id.as_str())
                || self
                    .leased_agent_provider_run_id(&binding.leased_agent_id)?
                    .as_deref()
                    != Some(binding.worker_provider_run_id.as_str()))
        {
            return Err(crate::durable_state::agent_lifecycle::error(
                "leased process start requires the exact active home prompt and worker run",
            ));
        }
        let prompt = self
            .app
            .prompt_owner_active_prompt_for_agent(
                &agent.backing_session_id,
                &agent.backing_agent_id,
            )?
            .map(|p| p.id().to_string());
        Ok((
            agent.backing_session_id.clone(),
            agent.backing_agent_id.clone(),
            prompt,
        ))
    }
}
