//! MP-08 / MP-10 / MP-11: the home conversation survives a recreated worker lease.

use super::builder::build_recreated_lease_context;

const MAX_HANDOFF_BYTES: usize = 24_000;

impl super::super::KernelRuntimeOwnedState {
    pub(in crate::runtime::state) fn remote_prompt_hidden_context(
        &self,
        agent: &crate::agent::AgentInstance,
        leased_agent_id: &str,
        prompt_id: &str,
        hidden_context: &str,
        fresh_workflow: bool,
        native_provider_run: bool,
    ) -> Result<String, crate::error::DaemonError> {
        let hidden_context = self
            .room_secret_observations
            .scrub_runtime_result(agent.session_id(), hidden_context.to_string())?;
        let existing = || Ok(hidden_context.clone());
        if fresh_workflow
            || agent
                .remote_execution()
                .is_some_and(|binding| binding.active_worker_provider_run_id.is_some())
        {
            return existing();
        }
        let events = self
            .operational_history_store
            .load_session_events(agent.session_id(), Some(agent.id()))?;
        let latest = events
            .iter()
            .filter(|event| event.kind == crate::history::HistoryEventKind::ProviderOutput)
            .max_by_key(|event| event.sequence);
        if latest
            .and_then(|event| event.provider_run_id.as_deref())
            .and_then(|run| {
                crate::provider::worker_provider_run_id_from_projected_leased_id(
                    leased_agent_id,
                    run,
                )
            })
            .is_some()
        {
            return existing();
        }
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
        match (hidden_context.trim(), restored.trim()) {
            (_, "") => existing(),
            ("", restored) => Ok(restored.to_string()),
            (existing, restored) => Ok(format!("{existing}\n\n{restored}")),
        }
    }
}
