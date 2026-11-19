//! Claude runs turns no prompt asked for: when a `run_in_background` command ends, it starts a
//! turn of its own to report it. The kernel adopts that turn as the agent's running turn with a
//! task, so the turn's tools act for it and its end settles that task like any other turn's.
//!
//! States (prompt_state `ProviderOwnTurn`), driven by each output batch's
//! `provider_own_turn_running` level and by prompt settlement:
//! - no prompt active or queued -> Adopted at once;
//! - an earlier prompt still settling, or one queued -> Pending; adopted when that prompt's
//!   settlement leaves the agent with no prompt, or by the next batch that still reports it;
//! - the own turn ends (a batch without the level) -> a Pending turn is dropped and logged; an
//!   Adopted one completes through the batch's own settlement.
//!
//! A child held for its background calls has a task already: its notification turn runs under
//! that task's prompt, so the one task carries the turn's tools and its finish.
use super::*;
use crate::runtime::prompt_state::ProviderTurnAdoption;

impl KernelRuntimeState {
    /// Follows one batch's report of whether the provider runs a turn of its own.
    pub(super) fn follow_provider_own_turn(
        &self,
        session_id: &str,
        provider_run_id: &str,
        running: bool,
    ) -> Result<(), DaemonError> {
        if running {
            return self.adopt_provider_started_turn(session_id, provider_run_id);
        }
        let owned = &self.owned;
        let run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let Some(agent_id) = run.agent_instance_id() else {
            return Ok(());
        };
        let session = owned.session_store.get_session(session_id)?;
        if let Some(pending_run) = owned
            .prompt_state_owner
            .end_provider_turn(&session, agent_id)
        {
            crate::logging::info_with_fields(
                "runtime.agent_lifecycle",
                "the provider's own turn ended while an earlier prompt was still settling; that prompt's task holds its output",
                serde_json::json!({
                    "session_id": session_id,
                    "agent_id": agent_id,
                    "provider_run_id": pending_run,
                }),
            );
        }
        Ok(())
    }

    /// A settlement may have left the agent with no prompt while its provider runs a turn of
    /// its own that waited for it: that turn becomes the agent's running turn now.
    pub(super) fn adopt_pending_provider_turn(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<(), DaemonError> {
        let owned = &self.owned;
        let run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let Some(agent_id) = run.agent_instance_id() else {
            return Ok(());
        };
        let session = owned.session_store.get_session(session_id)?;
        if !owned
            .prompt_state_owner
            .provider_turn_pending(&session, agent_id, provider_run_id)
        {
            return Ok(());
        }
        self.adopt_provider_started_turn(session_id, provider_run_id)
    }

    fn adopt_provider_started_turn(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<(), DaemonError> {
        let owned = &self.owned;
        let run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let Some(agent_id) = run.agent_instance_id() else {
            return Ok(());
        };
        let session = owned.session_store.get_session(session_id)?;
        if owned
            .prompt_state_owner
            .provider_turn_running(&session, agent_id)
        {
            return Ok(());
        }
        let agent = owned.agent_store.get_agent(agent_id)?;
        let held_prompt_id =
            owned.held_prompt_for_notification_turn(session_id, agent_id, provider_run_id)?;
        let prompt_id = held_prompt_id
            .clone()
            .unwrap_or_else(|| owned.session_store.reserve_prompt_id());
        let prompt = crate::session::PromptQueueItem::new(
            prompt_id,
            self.ensure_agent_message_attachment(session_id, &agent)?,
            agent_id,
            "",
            crate::session::PromptStatus::Running,
        );
        match owned.prompt_state_owner.adopt_provider_turn(
            &session,
            prompt.clone(),
            provider_run_id,
        ) {
            ProviderTurnAdoption::Pending { newly } => {
                if newly {
                    crate::logging::info_with_fields(
                        "runtime.agent_lifecycle",
                        "the provider started a turn on its own while an earlier prompt is still settling; it is adopted when that prompt leaves",
                        serde_json::json!({
                            "session_id": session_id,
                            "agent_id": agent_id,
                            "provider_run_id": provider_run_id,
                        }),
                    );
                }
                return Ok(());
            }
            ProviderTurnAdoption::Adopted => {}
        }
        let (active, queued) = owned.prompt_state_owner.state_parts(&session, agent_id);
        owned.mirror_prompt_owner_agent_state(session_id, agent_id, active, queued)?;
        owned.note_prompt_started(provider_run_id);
        // A held child's task exists and stays Working; any other turn begins its own task.
        if held_prompt_id.is_none() && owned.config_projection.snapshot().room_agent_tools {
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
                "held_task": held_prompt_id.is_some(),
            }),
        );
        Ok(())
    }
}
