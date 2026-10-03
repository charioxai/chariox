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
