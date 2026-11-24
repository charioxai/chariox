//! Persist the next provider profile while leaving the active turn's profile alone.

use super::agent_profile_owned_state::{AgentProfileSelections, ValidatedAgentProfile};
use super::*;
use crate::agent::{AgentInstance, PendingAgentProfile};
use crate::local::SettingsApplies;

impl KernelRuntimeState {
    pub(crate) fn read_agent_settings(
        &self,
        caller_user_id: &str,
        request: crate::local::GetAgentSettingsRequest,
    ) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
        let reference = request.agent.strip_prefix('@').unwrap_or(&request.agent);
        let mut matches = self
            .owned
            .agent_store
            .list_agents()
            .into_iter()
            .filter(|agent| {
                request
                    .session_id
                    .as_deref()
                    .is_none_or(|room| room == agent.session_id())
                    && !agent.archived()
                    && !self
                        .owned
                        .session_store
                        .is_ephemeral_session(agent.session_id())
                    && (agent.id() == reference
                        || agent.agent_ref() == reference
                        || agent.alias() == Some(reference))
                    && self.owned.acts_for_owner(
                        agent.session_id(),
                        caller_user_id,
                        agent.owner_user_id(),
                    )
            });
        let agent = matches.next().ok_or_else(|| DaemonError::AgentNotFound {
            agent_id: request.agent.clone(),
        })?;
        if matches.next().is_some() {
            return Err(DaemonError::LocalTransport {
                operation: "read agent settings",
                message: "More than one agent matches. Specify the Room or use the agent ID."
                    .to_string(),
            });
        }
        Ok(crate::local::LocalDaemonResponse::AgentSettings {
            agent: (&agent).into(),
        })
    }

    pub(super) async fn defer_active_agent_profile_update(
        &self,
        session: &crate::session::RuntimeSession,
        original: &AgentInstance,
        caller_user_id: &str,
        selections: AgentProfileSelections,
        context: &SettingsContext,
    ) -> Result<Option<AgentInstance>, DaemonError> {
        let owned = &self.owned;
        let agent_id = original.id();
        if original.remote_execution().is_some() {
            return Err(DaemonError::LocalTransport {
                operation: "defer agent profile update",
                message: "Use the ordinary profile update for an agent running on a leased kernel."
                    .to_string(),
            });
        }
        let validate = |selections| {
            owned.validate_agent_profile_update(session.id(), agent_id, caller_user_id, selections)
        };
        let target = validate(selections.clone())?;
        let selections = self
            .resolve_updated_agent_selection(caller_user_id, original, &target, selections)
            .await?;
        let ValidatedAgentProfile {
            provider,
            account_profile,
            model,
            effort,
            ..
        } = validate(selections)?;
        let pending = PendingAgentProfile {
            provider,
            account_profile,
            model,
            effort,
        };
        let staged = owned
            .prompt_state_owner
            .with_active_prompt(session, agent_id, |active| {
                if active.is_none() {
                    return Ok(None);
                }
                let mut agents = owned.agent_store.write();
                let current = agents.get_agent(agent_id)?;
                let requested_matches = pending.provider == current.provider()
                    && pending.model.as_deref() == current.model()
                    && pending.effort.as_deref() == current.effort()
                    && pending.account_profile == current.provider_account_profile();
                let mut updated = current.clone();
                updated.set_pending_profile((!requested_matches).then_some(pending));
                if updated != current {
                    owned.persist_agent_profile_record(&updated)?;
                }
                Ok(Some((current, agents.restore_agent(updated))))
            })?;
        let Some((current, updated)) = staged else {
            return Ok(None);
        };

        let Some(pending) = updated.pending_profile() else {
            return Ok(Some(updated));
        };
        let mut next = current.clone();
        next.set_provider(pending.provider.clone());
        next.set_account_profile(Some(pending.account_profile.clone()));
        next.set_model(pending.model.clone());
        next.set_effort(pending.effort.clone());
        let changes = owned
            .agent_settings_view(&current)?
            .changes_to(&owned.agent_settings_view(&next)?);
        owned.record_settings_change(
            agent_id,
            &changes,
            &context.origin,
            context.reason.as_deref(),
            SettingsApplies::AtTurnEnd,
        );
        owned.announce_settings_change(
            session.id(),
            agent_id,
            &changes,
            &context.origin,
            true,
            false,
        );
        Ok(Some(updated))
    }
}

impl KernelRuntimeOwnedState {
    pub(super) fn persist_agent_profile_record(
        &self,
        agent: &AgentInstance,
    ) -> Result<(), DaemonError> {
        self.durable_state_store.append_event(
            "agent.updated",
            Some(agent.id().to_string()),
            serde_json::json!({ "agent": agent, "capability_name": null }),
        )?;
        Ok(())
    }

    /// Prompt admission stays fenced until the updated profile is durable and the
    /// process for the old profile has been marked for replacement on its next use.
    pub(super) fn apply_agent_profile_after_turn(
        &self,
        session: &crate::session::RuntimeSession,
        agent_id: &str,
    ) -> Result<bool, DaemonError> {
        if session.active_prompt_for_agent(agent_id).is_some()
            || !self.agent_store.has_pending_profile(agent_id)
        {
            return Ok(false);
        }
        // The protected history reads the prompt state, so it is prepared before the prompt-state
        // fence below is taken. Prompt admission is already held by the pending profile.
        let before = self.agent_store.get_agent(agent_id)?;
        if let Some(pending) = before.pending_profile() {
            self.protect_deferred_window_downshift(session.id(), &before, pending)?;
        }
        let applied = self
            .prompt_state_owner
            .with_active_prompt(session, agent_id, |active| {
                if active.is_some() {
                    return Ok(false);
                }
                let agent = self.agent_store.get_agent(agent_id)?;
                let Some(pending) = agent.pending_profile() else {
                    return Ok(false);
                };
                let source_account = self.agent_profile_source_account(&agent);
                if let Some(run) = self
                    .provider_store
                    .get_run_for_agent(session.id(), agent_id)
                {
                    self.provider_store.mark_run_settings_stale(run.id())?;
                }

                let mut agents = self.agent_store.write();
                let mut current = agents.get_agent(agent_id)?;
                if current.pending_profile() != Some(pending) {
                    return Ok(false);
                }
                let resume = current.provider_resume_state().after_profile_change(
                    current.provider(),
                    &source_account,
                    &pending.provider,
                    &pending.account_profile,
                );
                current.set_provider(pending.provider.clone());
                current.set_account_profile(Some(pending.account_profile.clone()));
                current.set_model(pending.model.clone());
                current.set_effort(pending.effort.clone());
                current.set_provider_resume_state(resume);
                current.set_pending_profile(None);
                self.persist_agent_profile_record(&current)?;
                agents.restore_agent(current);
                Ok(true)
            })?;
        if applied {
            self.invalidate_workflow_copies_after_source_agent_change(session.id(), agent_id)?;
        }
        Ok(applied)
    }

    /// A deferred model change moves a settled session onto the new profile. When that is a
    /// Claude session moving to a smaller context window than its transcript fits, the next turn
    /// must not resume the oversized transcript. The turn has ended, so the source model cannot
    /// compact it here; take the protected fallback of a failed source compaction instead: keep
    /// the kernel's own history for the next turn and clear only this native session.
    fn protect_deferred_window_downshift(
        &self,
        session_id: &str,
        agent: &AgentInstance,
        pending: &crate::agent::PendingAgentProfile,
    ) -> Result<(), DaemonError> {
        let Some(run) = self
            .provider_store
            .get_run_for_agent(session_id, agent.id())
        else {
            return Ok(());
        };
        let Some(target_model) = pending.model.as_deref().or(agent.model()) else {
            return Ok(());
        };
        if !super::agent_profile_compaction::needs_compaction(
            &run,
            &pending.provider,
            &pending.account_profile,
            target_model,
        ) {
            return Ok(());
        }
        self.prepare_compacted_session_context_handoff(&run)?;
        if let Some(native_session) = run.provider_session_id().or_else(|| {
            agent
                .provider_resume_state()
                .provider_session_id(run.adapter_key())
        }) {
            match self
                .agent_store
                .clear_provider_resume_state_durably_if_matches(
                    &self.durable_state_store,
                    agent.id(),
                    run.adapter_key(),
                    native_session,
                    run.id(),
                    "deferred_window_downshift",
                )? {
                crate::agent::ProviderResumeClearOutcome::Required => {
                    return Err(
                        crate::provider::NativeCheckpointFailure::ResumeFailed.into_daemon_error()
                    );
                }
                crate::agent::ProviderResumeClearOutcome::Superseded { .. } => {
                    return Err(DaemonError::LocalTransport {
                        operation: "apply deferred agent profile",
                        message: "the native session changed while the deferred model was applied; select the model again".into(),
                    });
                }
                _ => {}
            }
        }
        self.record_notice(
            session_id,
            Some(run.id()),
            self.attachment_store.list_session_attachment_ids(session_id),
            "The next turn starts the smaller context window from the kernel's saved history instead of resuming the larger Claude session.",
        );
        Ok(())
    }

    pub(super) fn invalidate_workflow_copies_after_source_agent_change(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let affected_workflows = self
            .session_store
            .write()
            .invalidate_workflow_runtime_instances_for_agent_change(session_id, agent_id)?;
        if affected_workflows.is_empty() {
            return Ok(());
        }
        self.persist_workflow_runtime_session(session_id, "workflow_source_agent_changed")?;
        self.workflow_cleanup_runtime_instances_exclusive(session_id)
    }
}
