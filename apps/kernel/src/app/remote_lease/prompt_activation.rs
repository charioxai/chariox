//! MP-08 / MP-10 / MP-11: activate the exact ordinary leased queue binding.
use super::RemoteLeaseRuntime;

impl RemoteLeaseRuntime<'_> {
    pub(crate) fn activate_leased_home_prompt_for_dispatch(
        &mut self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        provider_run_id: &str,
    ) {
        let Some(leased) = self
            .app
            .leased_agents
            .values()
            .find(|leased| {
                leased.backing_session_id == session_id && leased.backing_agent_id == agent_id
            })
            .cloned()
        else {
            return;
        };
        let Some(home) = self
            .app
            .worker_prompt_receipts
            .home_prompt_for_backing(&leased.id, &leased.lease_id, provider_run_id, prompt_id)
            .map(str::to_string)
        else {
            return;
        };
        let Some(active) = self
            .app
            .prompt_owner_active_prompt_for_agent_snapshot(session_id, agent_id)
            .ok()
            .flatten()
            .filter(|active| active.id() == prompt_id)
        else {
            return;
        };
        if let Some(agent) = self.app.leased_agents.get_mut(&leased.id) {
            if agent.active_home_prompt_id.as_deref() != Some(home.as_str()) {
                agent.applied_home_steer_ids.clear();
                agent.replayable_completion = None;
            }
            agent.active_home_prompt_id = Some(home);
            agent.active_home_prompt_started_at_ms = Some(active.created_at_ms());
        }
    }
}
