//! Provider-run cleanup after a session loses its final attachment.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn park_detached_idle_provider_run(
        &self,
        session_id: &str,
    ) -> Result<bool, DaemonError> {
        if !self
            .attachment_store
            .list_session_attachment_ids(session_id)
            .is_empty()
        {
            return Ok(false);
        }

        let session = self.session_store.get_session(session_id)?;
        if self.prompt_state_owner.has_any_active_prompt(&session) {
            return Ok(false);
        }
        let Some(active_provider_run_id) = session.active_provider_run_id().map(str::to_string)
        else {
            return Ok(false);
        };
        if self.active_provider_run_is_remote(session_id, &active_provider_run_id) {
            return Ok(false);
        }

        match self.provider_store.get_run(&active_provider_run_id) {
            Ok(run) if run.state() == crate::provider::ProviderRunState::Starting => {
                crate::logging::debug_with_fields(
                    "daemon.session",
                    "deferred detached provider park while launch is starting",
                    serde_json::json!({
                        "session_id": session_id,
                        "provider_run_id": active_provider_run_id,
                    }),
                );
                return Ok(false);
            }
            Ok(run) if run.state() == crate::provider::ProviderRunState::Ended => {
                self.clear_active_provider_run_session_pointer(session_id, run.id())?;
            }
            Ok(_) => {
                let outcome = self
                    .provider_store
                    .park_run_provider_only(session_id, &active_provider_run_id)?;
                self.clear_active_provider_run_session_pointer(session_id, outcome.run().id())?;
                self.provider_run_projection.update(outcome.into_run());
            }
            Err(DaemonError::ProviderRunNotFound { .. }) => {
                if let Some(mut projected) =
                    self.provider_run_projection.get(&active_provider_run_id)
                {
                    projected.mark_ended();
                    self.provider_run_projection.update(projected);
                }
                self.session_store
                    .set_active_provider_run(session_id, None)?;
            }
            Err(error) => return Err(error),
        }

        for run in self.provider_store.list_runs() {
            if run.session_id() == session_id {
                self.clear_prompt_activity(run.id());
            }
        }
        self.session_snapshot(session_id)?;
        Ok(true)
    }

    fn active_provider_run_is_remote(&self, session_id: &str, provider_run_id: &str) -> bool {
        self.agent_store
            .get_session_agents(session_id)
            .into_iter()
            .filter_map(|agent| agent.remote_execution().cloned())
            .filter_map(|remote| {
                remote.active_worker_provider_run_id.map(|worker_run_id| {
                    crate::provider::projected_leased_provider_run_id(
                        &remote.leased_agent_id,
                        &worker_run_id,
                    )
                })
            })
            .any(|projected_run_id| projected_run_id == provider_run_id)
    }
}
