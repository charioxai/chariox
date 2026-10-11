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

    fn accepted(&self, session_id: &str, agent_id: &str, run_id: &str) -> Result<(), DaemonError> {
        let run = self.ensure_provider_run_in_session(session_id, run_id)?;
        self.consume_pending_context_handoff(session_id, agent_id, &run);
        Ok(())
    }
}
