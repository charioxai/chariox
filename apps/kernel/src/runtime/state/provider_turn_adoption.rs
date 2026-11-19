//! Claude runs turns no prompt asked for: when a `run_in_background` command ends, it starts a
//! turn of its own to report it. The kernel adopts that turn as the agent's running turn with a
//! task, so the turn's tools act for it and its end settles that task like any other turn's.
use super::*;

impl KernelRuntimeState {
    pub(super) fn adopt_provider_started_turn(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<(), DaemonError> {
        let owned = &self.owned;
        let run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let Some(agent_id) = run.agent_instance_id() else {
            return Ok(());
        };
        // A child held for its background calls reports them in this turn under its held task.
        if owned.background_holds.holds_run(provider_run_id) {
            return Ok(());
        }
        let session = owned.session_store.get_session(session_id)?;
        let agent = owned.agent_store.get_agent(agent_id)?;
        let prompt = crate::session::PromptQueueItem::new(
            owned.session_store.reserve_prompt_id(),
            self.ensure_agent_message_attachment(session_id, &agent)?,
            agent_id,
            "",
            crate::session::PromptStatus::Running,
        );
        if !owned
            .prompt_state_owner
            .adopt_provider_turn(&session, prompt.clone())
        {
            return Ok(());
        }
        let (active, queued) = owned.prompt_state_owner.state_parts(&session, agent_id);
        owned.mirror_prompt_owner_agent_state(session_id, agent_id, active, queued)?;
        owned.note_prompt_started(provider_run_id);
        if owned.config_projection.snapshot().room_agent_tools {
            owned.ensure_agent_task(session_id, agent_id, &prompt, Some(provider_run_id))?;
        }
        crate::logging::info_with_fields(
            "runtime.agent_lifecycle",
            "adopted a turn the provider started on its own",
            serde_json::json!({
                "session_id": session_id,
                "agent_id": agent_id,
                "prompt_id": prompt.id(),
                "provider_run_id": provider_run_id,
            }),
        );
        Ok(())
    }
}
