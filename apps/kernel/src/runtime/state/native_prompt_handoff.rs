//! MP-08 / MP-10 / MP-11: shared conversation handoff for directly typed native turns.
use super::*;

impl crate::app::NativePromptHandoffPolicy for KernelRuntimeOwnedState {
    fn prepare_context(
        &self,
        session_id: &str,
        agent_id: &str,
        run_id: &str,
        base: String,
    ) -> Result<String, DaemonError> {
        self.room_secret_observations.require(session_id, false)?;
        // SessionStart updates the store before UserPromptSubmit; the pump's
        // earlier run snapshot may still describe the previous native session.
        let run = self.ensure_provider_run_in_session(session_id, run_id)?;
        if run.agent_instance_id() != Some(agent_id) {
            return Err(DaemonError::LocalTransport {
                operation: "claude_native_handoff",
                message: "Native context run does not own this agent".into(),
            });
        }
        let handoff = self.context_handoff_for_dispatch(session_id, agent_id, &run, "", false);
        let hidden = match handoff {
            Some(handoff) => {
                let rendered = handoff
                    .render_hidden(crate::provider::claude_native_hidden_context_room(&base));
                if rendered.is_empty() {
                    return Err(DaemonError::ProviderProtocol {
                        provider_run_id: run_id.into(),
                        operation: "claude_hidden_context_size",
                        message:
                            "Native hidden context leaves no room for the conversation handoff"
                                .into(),
                    });
                }
                rendered
            }
            None => String::new(),
        };
        let context = [base.trim(), hidden.trim()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        self.room_secret_observations.scrub(session_id, context)
    }

    fn rejected(
        &self,
        session_id: &str,
        agent_id: &str,
        run_id: &str,
        prompt_id: &str,
    ) -> Result<Option<crate::session::PromptQueueItem>, DaemonError> {
        let run = self.ensure_provider_run_in_session(session_id, run_id)?;
        let session = self.session_store.get_session(session_id)?;
        let Some(prompt) = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
        else {
            return Ok(None);
        };
        if run.agent_instance_id() != Some(agent_id)
            || prompt.id() != prompt_id
            || prompt.is_external()
        {
            return Ok(None);
        }
        let message = crate::app::CLAUDE_NATIVE_CONTEXT_REJECTION_MESSAGE;
        if prompt.workflow_run_id().is_some() {
            self.workflow_fail_provider_prompt_without_queue_advance(
                session_id,
                &prompt,
                Some(run_id),
                message,
            )?;
        }
        if let Some(failed) = self.fail_local_prompt_without_advance_with_termination_if_matches(
            session_id,
            agent_id,
            Some(run_id),
            Some(prompt_id),
            None,
        )? {
            self.record_provider_failure_output(session_id, run_id, agent_id, message);
            self.record_failed_request(
                session_id,
                run_id,
                run.adapter_key(),
                agent_id,
                &prompt,
                message,
            );
            self.agent_store.clear_local_prompt_error(agent_id)?;
            self.agent_store
                .set_agent_state(agent_id, crate::agent::AgentState::Idle)?;
            let _ = self.session_snapshot(session_id)?;
            return Ok(Some(failed.completion.completed));
        }
        Ok(None)
    }

    fn accepted(&self, session_id: &str, agent_id: &str, run_id: &str) -> Result<(), DaemonError> {
        let run = self.ensure_provider_run_in_session(session_id, run_id)?;
        self.consume_pending_context_handoff(session_id, agent_id, &run);
        Ok(())
    }
}
