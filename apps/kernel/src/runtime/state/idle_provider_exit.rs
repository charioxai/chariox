//! A Claude process that ends while its agent is idle ends its run at once, so
//! the next prompt starts one fresh run instead of failing on the dead one first.
use super::*;

impl KernelRuntimeOwnedState {
    /// Ends every Running Claude run (or `only` that one) whose process exited
    /// while no turn ran, and returns the runs it ended.
    pub(super) fn end_runs_whose_idle_process_exited(&self, only: Option<&str>) -> Vec<String> {
        let mut ended = Vec::new();
        for run_id in self.provider_store.claude_runs_exited_while_idle(only) {
            let Ok(run) = self.provider_store.get_run(&run_id) else {
                continue;
            };
            if run.state() != crate::provider::ProviderRunState::Running {
                continue;
            }
            match self
                .provider_store
                .terminate_run_provider_only(run.session_id(), &run_id)
            {
                Ok(outcome) => {
                    if let Err(error) =
                        self.clear_active_provider_run_session_pointer(run.session_id(), &run_id)
                    {
                        crate::logging::warn_with_fields(
                            "daemon.provider_process_gc",
                            "could not clear the session pointer of a run whose idle process exited",
                            serde_json::json!({"provider_run_id": run_id, "error": error.to_string()}),
                        );
                    }
                    self.provider_run_projection.update(outcome.into_run());
                    crate::logging::info_with_fields(
                        "daemon.provider_process_gc",
                        "the provider process exited while idle; its run ended",
                        serde_json::json!({
                            "provider_run_id": run_id,
                            "session_id": run.session_id(),
                            "agent_id": run.agent_instance_id(),
                        }),
                    );
                    ended.push(run_id);
                }
                Err(error) => crate::logging::warn_with_fields(
                    "daemon.provider_process_gc",
                    "could not end a run whose idle process exited",
                    serde_json::json!({"provider_run_id": run_id, "error": error.to_string()}),
                ),
            }
        }
        ended
    }
}
