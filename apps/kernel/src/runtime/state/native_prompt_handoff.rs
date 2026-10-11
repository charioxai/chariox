//! MP-08 / MP-10 / MP-11: shared conversation handoff for directly typed native turns.
use super::*;

impl crate::app::NativePromptHandoffPolicy for KernelRuntimeOwnedState {
    fn prepare_context(
        &self,
        session_id: &str,
        agent_id: &str,
        run_id: &str,
        prompt_id: &str,
        steering: bool,
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
        let base = self.room_secret_observations.scrub(session_id, base)?;
        let handoff =
            self.context_handoff_for_dispatch(session_id, agent_id, &run, prompt_id, steering)?;
        let hidden = match handoff {
            Some(handoff) => {
                let handoff =
                    self.revalidate_context_handoff(handoff, session_id, agent_id, prompt_id)?;
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

    fn protect_inputs(
        &self,
        session_id: &str,
        prompt: String,
        hidden: String,
        attachments: Vec<crate::session::PromptAttachment>,
    ) -> Result<(String, String, Vec<crate::session::PromptAttachment>), DaemonError> {
        self.room_secret_observations
            .scrub(session_id, (prompt, hidden, attachments))
    }

    fn accepted(&self, session_id: &str, agent_id: &str, run_id: &str) -> Result<(), DaemonError> {
        let run = self.ensure_provider_run_in_session(session_id, run_id)?;
        self.consume_pending_context_handoff(session_id, agent_id, &run);
        Ok(())
    }
}

impl KernelRuntimeOwnedState {
    /// MP-08 / MP-10 / MP-11: protect all native dispatch fields under the Room barrier.
    pub(super) fn protect_native_dispatch(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
        handoff: Option<PendingAgentContextHandoff>,
    ) -> Result<
        (
            crate::app::KernelPromptDispatch,
            Option<PendingAgentContextHandoff>,
        ),
        DaemonError,
    > {
        let mut protected = dispatch.clone();
        let (prompt, hidden, attachments) = self.room_secret_observations.scrub(
            &dispatch.session_id,
            (
                protected.prompt,
                protected.hidden_system_context,
                protected.attachments,
            ),
        )?;
        protected.prompt = prompt;
        protected.hidden_system_context = hidden;
        protected.attachments = attachments;
        let handoff = handoff
            .map(|handoff| {
                self.revalidate_context_handoff(
                    handoff,
                    &dispatch.session_id,
                    &dispatch.agent_id,
                    &dispatch.prompt_id,
                )
            })
            .transpose()?;
        Ok((protected, handoff))
    }
}
