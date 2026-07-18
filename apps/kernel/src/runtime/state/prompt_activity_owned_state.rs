//! Prompt activity, transcript, terminal fan-out, history, and workspace-claim side effects.
//!
//! Prompt lifecycle state transitions stay in `prompt`; this module owns the observable side
//! effects and activity bookkeeping those transitions rely on.

use super::*;

const PROMPT_SETTLEMENT_RECHECK_DELAY: std::time::Duration = std::time::Duration::from_millis(50);

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
            match finished.result {
                Ok(acknowledgement) => {
                    if let Err(error) = self.finish_structured_prompt_delivery(
                        &finished.session_id,
                        &finished.agent_id,
                        &finished.prompt_id,
                        &finished.provider_run_id,
                        &acknowledgement,
                    ) {
                        crate::logging::warn_with_fields(
                            "daemon.prompt_delivery",
                            "failed to persist structured prompt acknowledgement",
                            serde_json::json!({
                                "session_id": finished.session_id,
                                "agent_id": finished.agent_id,
                                "prompt_id": finished.prompt_id,
                                "provider_run_id": finished.provider_run_id,
                                "error": error.to_string(),
                            }),
                        );
                    }
                }
                Err(error) => {
                    let diagnostic = format!("Provider prompt dispatch failed: {error}");
                    if let Ok(run) = self
                        .provider_store
                        .record_terminal_diagnostic(&finished.provider_run_id, diagnostic.clone())
                    {
                        self.provider_run_projection.update(run);
                    }
                    match self.settle_failed_local_prompt_without_advance(
                        &finished.session_id,
                        &finished.agent_id,
                        &finished.prompt_id,
                        &finished.provider_run_id,
                        &diagnostic,
                    ) {
                        Ok(Some(_)) => {}
                        Ok(None) => {}
                        Err(settlement_error) => {
                            crate::logging::warn_with_fields(
                                "daemon.prompt_delivery",
                                "failed to settle structured prompt dispatch failure",
                                serde_json::json!({
                                    "session_id": finished.session_id,
                                    "agent_id": finished.agent_id,
                                    "prompt_id": finished.prompt_id,
                                    "provider_run_id": finished.provider_run_id,
                                    "error": settlement_error.to_string(),
                                }),
                            );
                            let _ = self.cancel_active_prompt_only(
                                &finished.session_id,
                                &finished.agent_id,
                            );
                            let _ = self.clear_prompt_activity(&finished.provider_run_id);
                            let _ = self.session_snapshot(&finished.session_id);
                        }
                    }
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

    fn finish_structured_prompt_delivery(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        provider_run_id: &str,
        acknowledgement: &crate::provider::ProviderPromptSubmitAcknowledgement,
    ) -> Result<(), DaemonError> {
        let run = self
            .provider_store
            .apply_prompt_submit_acknowledgement(provider_run_id, acknowledgement)?;
        if let Some(run_agent_id) = run.agent_instance_id() {
            let agent = self
                .agent_store
                .set_agent_runtime_profile_with_account_profile(
                    run_agent_id,
                    run.provider(),
                    Some(run.model().to_string()),
                    run.variant().map(str::to_string),
                    Some(run.account_profile().to_string()),
                    run.resume_state().clone(),
                )?;
            self.durable_state_store.append_event(
                "agent.runtime_profile_updated",
                Some(agent.id().to_string()),
                serde_json::json!({
                    "agent": &agent,
                    "provider_run_id": run.id(),
                    "reason": "prompt_delivery_acknowledged",
                }),
            )?;
        }
        self.provider_run_projection.update(run.clone());
        self.mark_active_prompt_delivery(
            session_id,
            agent_id,
            prompt_id,
            crate::session::DurablePromptDeliveryPhase::Delivered,
            Some(provider_run_id.to_string()),
            run.provider_session_id().map(str::to_string),
        )?;
        let session = self.session_store.get_session(session_id)?;
        if let Some(active) = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
        {
            if active.id() == prompt_id
                && active.durable_recovery_phase()
                    == Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
            {
                if let Some(operation_id) = active.durable_recovery_operation_id() {
                    self.mark_active_prompt_recovery_phase(
                        session_id,
                        agent_id,
                        prompt_id,
                        operation_id,
                        crate::session::DurablePromptDeliveryPhase::Delivered,
                    )?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn clear_prompt_activity(&self, provider_run_id: &str) -> bool {
        self.provider_output_deadlines.clear(provider_run_id);
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
                    .active_prompt_for_agent(&session, &agent_id)?;
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
            self.schedule_provider_output_timeout(provider_run_id);
        }
    }

    pub(super) fn note_prompt_output(&self, provider_run_id: &str) {
        let tracked = if let Some(state) = self.prompt_activity.write().get_mut(provider_run_id) {
            state.last_output_at = Some(Instant::now());
            true
        } else {
            false
        };
        self.active_turns.mark_streaming(provider_run_id);
        if tracked {
            self.schedule_provider_output_timeout(provider_run_id);
        }
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
        if self.prompt_activity.read().contains_key(provider_run_id) {
            self.schedule_provider_output_timeout(provider_run_id);
        }
    }

    pub(super) fn note_prompt_settlement_requested(&self, provider_run_id: &str) {
        self.active_turns.mark_settling(provider_run_id);
        self.prompt_activity
            .write()
            .entry(provider_run_id.to_string())
            .and_modify(|state| {
                state.request_settlement();
            })
            .or_insert(crate::app::ActivePromptState {
                last_output_at: Some(Instant::now()),
                saw_response_content: true,
                completion_recorded: false,
                settlement_requested: true,
            });
        self.schedule_provider_output_check_after(provider_run_id, PROMPT_SETTLEMENT_RECHECK_DELAY);
    }

    pub(super) fn schedule_provider_output_check_after(
        &self,
        provider_run_id: &str,
        delay: std::time::Duration,
    ) {
        self.provider_output_deadlines.schedule(
            provider_run_id,
            crate::session::unix_epoch_ms().saturating_add(delay.as_millis() as u64),
        );
    }

    pub(super) fn schedule_provider_output_check_when_quiet(
        &self,
        provider_run_id: &str,
        quiet_for: std::time::Duration,
    ) {
        let delay = self
            .prompt_activity
            .read()
            .get(provider_run_id)
            .and_then(|state| state.last_output_at)
            .map(|last_output_at| quiet_for.saturating_sub(last_output_at.elapsed()))
            .unwrap_or(quiet_for);
        self.schedule_provider_output_check_after(provider_run_id, delay);
    }

    pub(super) fn ensure_provider_output_timeout_scheduled(&self, provider_run_id: &str) {
        if !self.prompt_activity.read().contains_key(provider_run_id) {
            return;
        }
        self.provider_output_deadlines.schedule_if_absent(
            provider_run_id,
            crate::session::unix_epoch_ms().saturating_add(crate::app::PROVIDER_OUTPUT_TIMEOUT_MS),
        );
    }

    fn schedule_provider_output_timeout(&self, provider_run_id: &str) {
        self.provider_output_deadlines.schedule(
            provider_run_id,
            crate::session::unix_epoch_ms().saturating_add(crate::app::PROVIDER_OUTPUT_TIMEOUT_MS),
        );
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
