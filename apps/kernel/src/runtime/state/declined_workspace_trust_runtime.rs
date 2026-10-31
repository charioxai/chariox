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
        let mut abort = None;
        if let Some((agent_id, prompt)) = &stopped {
            abort = owned
                .cancel_local_prompt_if_matches(
                    session_id,
                    agent_id,
                    prompt.source_attachment_id(),
                    Some(prompt.id()),
                )?
                .and_then(|cancellation| cancellation.dispatch);
        }
        // The denial is remembered by this run's context file, so the next turn starts a new run.
        // The run ends before the stopped turn settles: settling promotes the queue only onto a
        // running provider, and a queued prompt must start on the replacement, not on this run.
        self.retire_owned_provider_run_after_terminal_failure(session_id, provider_run_id)
            .await;
        if let Some(dispatch) = abort {
            self.settle_stopped_native_prompt(&dispatch).await;
        }
        if let Some((agent_id, _)) = stopped {
            self.start_queued_prompt_on_replacement_run(session_id, &agent_id)
                .await?;
        }
        Ok(())
    }

    async fn start_queued_prompt_on_replacement_run(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let owned = &self.owned;
        let session = owned.session_store.get_session(session_id)?;
        if owned
            .prompt_state_owner
            .peek_next_queued_prompt(&session, agent_id)
            .is_none()
        {
            return Ok(());
        }
        let (session_for_launch, agent_for_launch) = (session_id.to_string(), agent_id.to_string());
        let replacement_provider_run_id = self
            .with_app_side_effect(move |app| {
                app.ensure_prompt_provider_run_for_agent(&session_for_launch, &agent_for_launch)
            })
            .await?;
        if let Some(dispatch) = owned.advance_next_queued_prompt_dispatch(
            session_id,
            agent_id,
            &replacement_provider_run_id,
        )? {
            self.spawn_prompt_dispatch(dispatch, self.provider_runtime_lanes.clone());
        }
        Ok(())
    }
}
