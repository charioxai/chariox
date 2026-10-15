//! MP-08 / MP-10 / MP-11: the home conversation survives a recreated worker lease.

use super::builder::build_recreated_lease_context;

const MAX_HANDOFF_BYTES: usize = 24_000;

impl super::super::KernelRuntimeOwnedState {
    pub(in crate::runtime::state) fn clear_accepted_remote_lease_context(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
    ) {
        if let Err(error) = self
            .operational_history_store
            .clear_remote_lease_context_if_replayed(
                &dispatch.session_id,
                &dispatch.agent_id,
                &dispatch.worker_kernel_id,
                &dispatch.leased_agent_id,
                &dispatch.prompt_id,
            )
        {
            crate::logging::warn_with_fields(
                "daemon.remote_prompt_dispatch",
                "accepted lease context marker could not be cleared",
                serde_json::json!({"session_id": dispatch.session_id, "agent_id": dispatch.agent_id,
                    "error": error.to_string()}),
            );
        }
    }

    pub(in crate::runtime::state) fn remote_prompt_hidden_context(
        &self,
        agent: &crate::agent::AgentInstance,
        leased_agent_id: &str,
        prompt_id: &str,
        hidden_context: &str,
        fresh_workflow: bool,
        native_provider_run: bool,
    ) -> Result<String, crate::error::DaemonError> {
        if let Some(binding) = agent
            .remote_execution()
            .filter(|b| b.leased_agent_id == leased_agent_id)
        {
            if self
                .operational_history_store
                .remote_lease_context_pending(agent.session_id(), agent.id(), binding)
                .unwrap_or(false)
            {
                self.operational_history_store
                    .record_remote_lease_context_replay(
                        agent.session_id(),
                        agent.id(),
                        binding,
                        None,
                    )?;
            }
        }
        let hidden_context = match self
            .room_secret_observations
            .scrub_runtime_result(agent.session_id(), hidden_context.to_string())
        {
            Ok(context) => context,
            // Unknown protection withholds optional history replay, while the
            // current turn keeps its workflow, failure and steer instructions.
            Err(_) => return Ok(hidden_context.to_string()),
        };
        let existing = || Ok(hidden_context.clone());
        if fresh_workflow {
            return existing();
        }
        let Some(binding) = agent
            .remote_execution()
            .filter(|binding| binding.leased_agent_id == leased_agent_id)
        else {
            return existing();
        };
        // MP-08 / MP-10 / MP-11: this indexed, private marker describes actual
        // lease recreation. Raw worker output IDs are not lease identities.
        if !self
            .operational_history_store
            .remote_lease_context_pending(agent.session_id(), agent.id(), binding)
            .unwrap_or(false)
        {
            return existing();
        }
        let Ok(events) = self
            .operational_history_store
            .load_session_events(agent.session_id(), Some(agent.id()))
        else {
            return existing();
        };
        let room = if native_provider_run
            && crate::provider::adapter_key_for_provider(agent.provider()) == "claude"
        {
            crate::provider::claude_native_hidden_context_room(&hidden_context)
        } else {
            MAX_HANDOFF_BYTES.saturating_sub(hidden_context.trim().len() + 2)
        };
        let events = self.room_secret_observations.protect_history_events(events);
        let Some(restored) =
            build_recreated_lease_context(&events, prompt_id, room.min(MAX_HANDOFF_BYTES))
        else {
            return existing();
        };
        // Record what this exact submission carries before transport. Receipt
        // reconciliation can then acknowledge it after a home restart too.
        self.operational_history_store
            .record_remote_lease_context_replay(
                agent.session_id(),
                agent.id(),
                binding,
                Some(prompt_id),
            )?;
        match (hidden_context.trim(), restored.trim()) {
            (_, "") => existing(),
            ("", restored) => Ok(restored.to_string()),
            (existing, restored) => Ok(format!("{existing}\n\n{restored}")),
        }
    }
}
