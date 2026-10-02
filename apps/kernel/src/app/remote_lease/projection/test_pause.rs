//! Fixture-owned lease projection gate; dropping the handle resumes normal pumping.

use super::RemoteLeaseRuntime;
use std::sync::Arc;

impl RemoteLeaseRuntime<'_> {
    pub(crate) fn hold_leased_runtime_projection_for_test(
        &mut self,
        leased_agent_id: &str,
    ) -> Arc<()> {
        assert!(self.app.leased_agents.contains_key(leased_agent_id));
        let hold = Arc::new(());
        self.app
            .leased_runtime_projection_pauses
            .insert(leased_agent_id.to_string(), Arc::downgrade(&hold));
        hold
    }
}

pub(super) fn is_paused(app: &crate::app::DaemonApp, leased_agent_id: &str) -> bool {
    app.leased_runtime_projection_pauses
        .get(leased_agent_id)
        .is_some_and(|hold| hold.strong_count() > 0)
}

impl crate::app::DaemonApp {
    pub(in crate::app) fn leased_provider_run_projection_is_paused_for_test(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> bool {
        let Ok(run) = self.providers.get_run(provider_run_id) else {
            return false;
        };
        self.leased_agents.iter().any(|(id, leased)| {
            leased.backing_session_id == session_id
                && Some(leased.backing_agent_id.as_str()) == run.agent_instance_id()
                && is_paused(self, id)
        })
    }
}
