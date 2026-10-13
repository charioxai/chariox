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

    /// Runs `hook` once, between this drain reading terminal output and
    /// reading the backing prompt state.
    pub(crate) fn after_leased_output_drain_for_test(
        &mut self,
        hook: impl FnOnce(&mut crate::app::DaemonApp) + Send + 'static,
    ) {
        self.app.leased_projection_after_output_drain = Some(Box::new(hook));
    }
}

pub(super) fn run_after_output_drain(app: &mut crate::app::DaemonApp) {
    if let Some(hook) = app.leased_projection_after_output_drain.take() {
        hook(app);
    }
}

pub(super) fn is_paused(app: &crate::app::DaemonApp, leased_agent_id: &str) -> bool {
    app.leased_runtime_projection_pauses
        .get(leased_agent_id)
        .is_some_and(|hold| hold.strong_count() > 0)
}
