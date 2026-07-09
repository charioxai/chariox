//! Prompt activity, transcript, terminal fan-out, history, and workspace-claim side effects.
//!
//! Prompt lifecycle state transitions stay in `prompt`; this module owns the observable side
//! effects and activity bookkeeping those transitions rely on.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn prompt_completion_recorded(&self, provider_run_id: &str) -> bool {
        self.prompt_activity
            .read()
            .get(provider_run_id)
            .map(|state| state.completion_recorded)
            .unwrap_or(false)
    }

    pub(super) fn prompt_completion_settlement_pending(&self, provider_run_id: &str) -> bool {
        self.prompt_activity
            .read()
            .get(provider_run_id)
            .map(|state| state.completion_recorded && state.settlement_requested)
            .unwrap_or(false)
    }

    pub(super) fn prompt_output_quiet_after_response(
        &self,
        provider_run_id: &str,
        quiet_for: std::time::Duration,
    ) -> bool {
        self.prompt_activity
            .read()
            .get(provider_run_id)
            .is_some_and(|state| {
                state.saw_response_content
                    && state
                        .last_output_at
                        .is_some_and(|last_output_at| last_output_at.elapsed() >= quiet_for)
            })
    }

    pub(super) fn mark_prompt_completion_recorded(&self, provider_run_id: &str) {
        if let Some(state) = self.prompt_activity.write().get_mut(provider_run_id) {
            state.completion_recorded = true;
        }
    }

    pub(super) fn reap_structured_prompt_jobs(&self) {
        self.provider_store
            .apply_finished_provider_run_selection_sync_jobs();
        for finished in self
            .provider_store
            .drain_finished_structured_prompt_submit_jobs()
        {
            if let Err(error) = finished.result {
                let diagnostic = format!("Provider prompt dispatch failed: {error}");
                if let Ok(run) = self
                    .provider_store
                    .record_terminal_diagnostic(&finished.provider_run_id, diagnostic.clone())
                {
                    self.provider_run_projection.update(run);
                }
                if let Ok(session) = self.session_store.get_session(&finished.session_id) {
                    if let Some(prompt) = self
                        .prompt_state_owner
                        .active_prompt_for_agent_or_restore(&session, &finished.agent_id)
                    {
                        if prompt.workflow_run_id().is_some() {
                            let _ = self.workflow_fail_provider_prompt(
                                &finished.session_id,
                                &prompt,
                                Some(&finished.provider_run_id),
                                &diagnostic,
                            );
                        }
                    }
                }
                let _ = self.cancel_active_prompt_only(&finished.session_id, &finished.agent_id);
                let _ = self.session_snapshot(&finished.session_id);
                let recipients = self
                    .attachment_store
                    .list_session_attachment_ids(&finished.session_id);
                self.record_notice(
                    &finished.session_id,
                    Some(&finished.provider_run_id),
                    recipients,
                    format!("Prompt dispatch failed after acknowledgement: {error}"),
                );
            }
        }
        for finished in self
            .provider_store
            .drain_finished_structured_prompt_abort_jobs()
        {
            if let Err(error) = finished.result {
                let recipients = self
                    .attachment_store
                    .list_session_attachment_ids(&finished.session_id);
                self.record_notice(
                    &finished.session_id,
                    Some(&finished.provider_run_id),
                    recipients,
                    format!("Prompt cancellation dispatch failed after acknowledgement: {error}"),
                );
            } else if let Ok(provider_run) = self.provider_store.get_run(&finished.provider_run_id)
            {
                if crate::provider::provider_run_finalizes_cancellation_on_abort_dispatch(
                    &provider_run,
                ) {
                    if let Some(agent_id) = provider_run.agent_instance_id() {
                        let _ = self.finalize_local_prompt_cancellation_with_queued_advance(
                            &finished.session_id,
                            agent_id,
                            Some(&finished.provider_run_id),
                        );
                    }
                }
            }
        }
    }

    pub(super) fn clear_prompt_activity(&self, provider_run_id: &str) -> bool {
        let prompt_activity = self.prompt_activity.write().remove(provider_run_id);
        let active_turn = self.active_turns.get(provider_run_id);
        if prompt_activity.is_some() || active_turn.is_some() {
            if let Ok(run) = self.provider_store.get_run(provider_run_id) {
                crate::runtime::command_latency::log_provider_turn_completed(
                    &run,
                    active_turn.as_ref(),
                    prompt_activity.as_ref(),
                );
            }
        }
        self.active_turns.clear(provider_run_id);
        self.prompt_workspace_claims.remove(provider_run_id)
    }

    pub(super) fn clear_session_prompt_runtime_state(&self, session_id: &str) {
        let provider_run_ids = self
            .provider_store
            .list_runs()
            .into_iter()
            .filter(|run| run.session_id() == session_id)
            .map(|run| run.id().to_string())
            .collect::<Vec<_>>();
        for provider_run_id in provider_run_ids {
            let _ = self.clear_prompt_activity(&provider_run_id);
        }
        self.active_turns.clear_session(session_id);
        let _ = self
            .prompt_workspace_claims
            .remove_matching(|claim| claim.session_id == session_id);
    }

    pub(super) fn clear_agent_prompt_runtime_state(&self, session_id: &str, agent_id: &str) {
        let provider_run_ids = self
            .provider_store
            .list_runs()
            .into_iter()
            .filter(|run| {
                run.session_id() == session_id && run.agent_instance_id() == Some(agent_id)
            })
            .map(|run| run.id().to_string())
            .collect::<Vec<_>>();
        for provider_run_id in provider_run_ids {
            let _ = self.clear_prompt_activity(&provider_run_id);
        }
        self.active_turns.clear_agent(session_id, agent_id);
    }

    pub(super) fn release_workflow_node_workspace_claim(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> bool {
        let owner = format!("{workflow_run_id}:{workflow_node_run_id}");
        self.prompt_workspace_claims.remove_matching(|claim| {
            claim.session_id == session_id
                && claim.attachment_id.as_deref() == Some(owner.as_str())
                && claim.operation == "workflow_node_dispatch"
        }) > 0
    }

    pub(super) fn note_prompt_started(&self, provider_run_id: &str) {
        self.prompt_activity.write().insert(
            provider_run_id.to_string(),
            crate::app::ActivePromptState {
                last_output_at: None,
                saw_response_content: false,
                completion_recorded: false,
                settlement_requested: false,
            },
        );
        let active_turn = self
            .provider_store
            .get_run(provider_run_id)
            .ok()
            .and_then(|run| {
                let session_id = run.session_id().to_string();
                let agent_id = run.agent_instance_id()?.to_string();
                let session = self.session_store.get_session(&session_id).ok()?;
                let prompt = self
                    .prompt_state_owner
                    .active_prompt_for_agent_or_restore(&session, &agent_id)?;
                let prompt_id = prompt.id().to_string();
                Some(
                    crate::app::ActiveTurnState::new(
                        session_id,
                        agent_id,
                        prompt_id,
                        provider_run_id.to_string(),
                    )
                    .with_prompt_metadata(&prompt)
                    .with_phase(crate::app::ActiveTurnPhase::AwaitingFirstOutput),
                )
            });
        if let Some(turn) = active_turn {
            self.active_turns.start(turn);
            self.active_turns
                .mark_awaiting_first_output(provider_run_id);
        }
    }

    pub(super) fn note_prompt_output(&self, provider_run_id: &str) {
        if let Some(state) = self.prompt_activity.write().get_mut(provider_run_id) {
            state.last_output_at = Some(Instant::now());
        }
        self.active_turns.mark_streaming(provider_run_id);
    }

    pub(super) fn note_prompt_response_content(&self, provider_run_id: &str) {
        let first_response_content = {
            let mut prompt_activity = self.prompt_activity.write();
            if let Some(state) = prompt_activity.get_mut(provider_run_id) {
                let first_response_content = !state.saw_response_content;
                state.last_output_at = Some(Instant::now());
                state.saw_response_content = true;
                first_response_content
            } else {
                false
            }
        };
        if first_response_content {
            self.active_turns.mark_streaming(provider_run_id);
            if let Ok(run) = self.provider_store.get_run(provider_run_id) {
                let active_turn = self.active_turns.get(provider_run_id);
                crate::runtime::command_latency::log_provider_first_response_content(
                    &run,
                    active_turn.as_ref(),
                );
            }
        }
    }

    pub(super) fn note_prompt_settlement_requested(&self, provider_run_id: &str) {
        self.active_turns.mark_settling(provider_run_id);
        self.prompt_activity
            .write()
            .entry(provider_run_id.to_string())
            .and_modify(|state| {
                state.last_output_at = Some(Instant::now());
                state.saw_response_content = true;
                state.settlement_requested = true;
            })
            .or_insert(crate::app::ActivePromptState {
                last_output_at: Some(Instant::now()),
                saw_response_content: true,
                completion_recorded: false,
                settlement_requested: true,
            });
    }

    pub(super) fn acquire_workflow_node_workspace_claim(
        &self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<(), DaemonError> {
        if self.prompt_workspace_claims.contains(provider_run_id) {
            return Ok(());
        }
        let session = self.session_store.get_session(session_id)?;
        let workspace_id = session.workspace_id().to_string();
        let worktree_id = self
            .agent_store
            .get_agent(agent_id)
            .ok()
            .and_then(|agent| agent.worktree_id().map(str::to_string))
            .unwrap_or_else(|| session.worktree_id().to_string());
        let claim = self.workspace_coordinator.acquire_worktree_write_claim(
            workspace_id,
            worktree_id,
            session_id,
            Some(format!("{workflow_run_id}:{workflow_node_run_id}")),
            "workflow_node_dispatch",
        )?;
        self.prompt_workspace_claims
            .insert(provider_run_id.to_string(), claim);
        Ok(())
    }
}
