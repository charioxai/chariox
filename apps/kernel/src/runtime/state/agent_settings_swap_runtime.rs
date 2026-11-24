//! A turn goes on in a new provider process when its agent's settings changed while it ran.
//! The change is written to the agent at once; the process takes it at the next finished step.

use super::agent_settings_change::SettingChange;
use super::*;

impl KernelRuntimeOwnedState {
    /// Whether the run has a tool call open: the provider has not finished that step.
    fn run_has_open_tools(&self, provider_run_id: &str) -> bool {
        self.prompt_activity
            .read()
            .get(provider_run_id)
            .is_some_and(|state| !state.active_tools.is_empty())
    }
}

impl KernelRuntimeOwnedState {
    /// Whether a `run_in_background` call of the run has not ended: it is a child of the
    /// provider's process, so ending that process ends the call and its notification turn.
    pub(super) fn run_has_running_background_calls(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        Ok(crate::app::background_calls(
            self.operational_history_store
                .load_provider_tool_entries_for_run(session_id, provider_run_id)?,
        ) == crate::app::BackgroundCalls::Running)
    }
}

impl KernelRuntimeState {
    /// Called after each batch of a run's output: when the agent's settings changed while this
    /// process ran and its step is finished, ends the process and goes on with the same turn in
    /// a new one. Never ends a process with a tool call, question or permission wait open, and
    /// never one whose turn is ending.
    pub(super) async fn swap_turn_process_at_step_boundary(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        let owned = &self.owned;
        let Ok(run) = owned.provider_store.get_run(provider_run_id) else {
            return Ok(false);
        };
        let Some(agent_id) = run.agent_instance_id().map(str::to_string) else {
            return Ok(false);
        };
        // A process the next dispatch started on the new settings: the first finished step of
        // its turn says that a change which waited for a background command took effect.
        if !run.settings_stale()
            && run.state() != crate::provider::ProviderRunState::Ended
            && run.client_interface().is_chariox()
        {
            if let Some((changes, by)) = owned.pending_settings_notices.take(session_id, &agent_id)
            {
                owned.announce_settings_change(session_id, &agent_id, &changes, &by, false, false);
            }
        }
        if !run.settings_stale()
            || run.state() == crate::provider::ProviderRunState::Ended
            || !run.client_interface().is_chariox()
            || !owned
                .provider_store
                .adapter_resumes_conversation_in_new_process(run.adapter_key())
            || owned.run_has_open_tools(provider_run_id)
        {
            return Ok(false);
        }
        let agent = owned.agent_store.get_agent(&agent_id)?;
        let same_family = crate::provider::canonical_provider_family(run.provider())
            .unwrap_or(run.provider())
            == crate::provider::canonical_provider_family(agent.provider())
                .unwrap_or(agent.provider());
        if agent.remote_execution().is_some()
            || !same_family
            || !owned.can_relaunch_with_held_credential(&run, &agent)
        {
            return Ok(false);
        }
        let session = owned.session_store.get_session(session_id)?;
        let Some(prompt) = owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
        else {
            return Ok(false);
        };
        if prompt.status() != crate::session::PromptStatus::Running
            || prompt.delivery_pending()
            || prompt.is_external()
            || owned.prompt_completion_settlement_pending(provider_run_id)
            || !owned.provider_run_has_active_prompt(session_id, &run)?
        {
            return Ok(false);
        }
        // A background command is a child of this process: a new process would end it, and with
        // it the provider's notification turn. The batch just applied may hold its launch row.
        self.flush_provider_history(provider_run_id).await?;
        if owned.run_has_running_background_calls(session_id, provider_run_id)? {
            return Ok(false);
        }
        self.swap_turn_process(session_id, &agent, &run, &prompt)
            .await?;
        Ok(true)
    }

    async fn swap_turn_process(
        &self,
        session_id: &str,
        agent: &crate::agent::AgentInstance,
        old_run: &crate::provider::RuntimeProviderRun,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<(), DaemonError> {
        let owned = &self.owned;
        let agent_id = agent.id();
        let adapter_key = old_run.adapter_key().to_string();
        // The old process's native session goes on only under the same account; under another
        // the kernel carries the conversation, whose request the continuation then repeats.
        let keeps_native_session = agent
            .provider_resume_state()
            .provider_session_id(&adapter_key)
            .is_some()
            || (crate::account_profile::provider_account_selection_for_run(old_run)
                == agent.provider_account_profile()
                && old_run.provider_session_id().is_some());
        if keeps_native_session {
            if let Some(provider_session_id) = old_run.provider_session_id() {
                self.persist_recovered_provider_session(agent, &adapter_key, provider_session_id)?;
            }
        }
        let (changes, by) = owned
            .pending_settings_notices
            .take(session_id, agent_id)
            .unwrap_or_default();
        // Unbind the turn before the old process ends, so its exit cannot settle it.
        owned.mark_active_prompt_delivery(
            session_id,
            agent_id,
            prompt.id(),
            crate::session::DurablePromptDeliveryPhase::Accepted,
            None,
            None,
        )?;
        let outcome = owned
            .provider_store
            .terminate_run_provider_only(session_id, old_run.id())?;
        owned.clear_active_provider_run_session_pointer(session_id, outcome.run().id())?;
        let ended = outcome.into_run();
        owned.provider_run_projection.update(ended.clone());
        let (_, process_key) = self
            .with_app_side_effect(|app| {
                crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(ended.id())
            })
            .await
            .unwrap_or((false, None));
        owned.remove_provider_process_tracking_for_run(ended.id(), process_key);
        let request_text = (!keeps_native_session).then(|| prompt.prompt().to_string());
        let continued = self
            .dispatch_prompt_continuation(
                session_id,
                agent_id,
                prompt,
                prompt.hidden_system_context().to_string(),
                prompt.attachments().to_vec(),
                |operation_id| settings_continuation_prompt(operation_id, &changes, request_text),
            )
            .await;
        if let Err(failure) = continued {
            let message = format!(
                "The turn could not go on with the new settings: {}. Change the settings and send the request again.",
                crate::provider::sanitize_provider_diagnostic(&failure.error.to_string())
            );
            // The turn belongs to the process the continuation started, not to the one the swap
            // ended: settling against the ended one finds the turn bound elsewhere and leaves it.
            let owning_run_id = failure.started_run_id.as_deref().unwrap_or(old_run.id());
            self.fail_owned_provider_prompt(session_id, owning_run_id, &message, true)
                .await?;
            return Err(failure.error);
        }
        owned.announce_settings_change(session_id, agent_id, &changes, &by, false, true);
        Ok(())
    }
}

/// The note a turn goes on with in its new process. It starts like a restart's, so the
/// transcript scan of a restart that interrupts the swap recognises the operation.
fn settings_continuation_prompt(
    operation_id: &str,
    changes: &[SettingChange],
    request: Option<String>,
) -> String {
    let changed = changes
        .iter()
        .map(|change| format!("{} {} -> {}", change.field, change.old, change.new))
        .collect::<Vec<_>>()
        .join(", ");
    let changed = if changed.is_empty() {
        String::new()
    } else {
        format!(" ({changed})")
    };
    let mut note = format!(
        "[Chariox recovery operation {operation_id}] Your settings changed{changed}. Continue the active task from the current state. Do not repeat completed tool calls or external side effects."
    );
    if let Some(request) = request {
        note.push_str("\n\nThe request you are working on:\n");
        note.push_str(&request);
    }
    note
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_continuation_names_the_changes_and_repeats_the_request_only_for_a_new_conversation() {
        let changes = vec![SettingChange {
            field: "model",
            old: "haiku".into(),
            new: "sonnet".into(),
        }];
        let native = settings_continuation_prompt("op-1", &changes, None);
        assert!(native.starts_with(
            "[Chariox recovery operation op-1] Your settings changed (model haiku -> sonnet)."
        ));
        assert!(!native.contains("The request you are working on"));
        let carried = settings_continuation_prompt("op-1", &changes, Some("fix the bug".into()));
        assert!(carried.ends_with("The request you are working on:\nfix the bug"));
    }
}
