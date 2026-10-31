//! A Claude workspace-trust question the person denied, or left unanswered until it expired,
//! ends the turn like a Stop. The provider's wording stays in the log, never in the Room.
use super::*;

impl KernelRuntimeState {
    pub(super) async fn stop_turn_after_declined_workspace_trust(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<(), DaemonError> {
        let owned = &self.owned;
        let run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        crate::logging::info_with_fields(
            "daemon.provider_output",
            "Claude workspace trust was not given; the turn ends as stopped",
            serde_json::json!({ "session_id": session_id, "provider_run_id": provider_run_id }),
        );
        let session = owned.session_store.get_session(session_id)?;
        let stopped = run.agent_instance_id().and_then(|agent_id| {
            owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, agent_id)
                .map(|prompt| (agent_id.to_string(), prompt))
        });
        if let Some((agent_id, prompt)) = stopped {
            let cancellation = owned.cancel_local_prompt_if_matches(
                session_id,
                &agent_id,
                prompt.source_attachment_id(),
                Some(prompt.id()),
            )?;
            if let Some(dispatch) = cancellation.and_then(|cancellation| cancellation.dispatch) {
                self.settle_stopped_native_prompt(&dispatch).await;
            }
        }
        // The denial is remembered by this run's context file, so the next turn starts a new run.
        self.retire_owned_provider_run_after_terminal_failure(session_id, provider_run_id)
            .await;
        Ok(())
    }
}
