//! Prompt activity, transcript, terminal fan-out, history, and workspace-claim side effects.
//!
//! Prompt lifecycle state transitions stay in `prompt`; this module owns the observable side
//! effects and activity bookkeeping those transitions rely on.

use super::*;

const PROMPT_SETTLEMENT_RECHECK_DELAY: std::time::Duration = std::time::Duration::from_millis(50);

enum FailedPromptResumePreparation {
    NotRequired,
    Cleared(crate::runtime::prompt_state::PromptDeliverySettlementClaim),
    Superseded,
}

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
            let settlement_retry_attempt = finished.settlement_retry_attempt;
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
                                "session_id": &finished.session_id,
                                "agent_id": &finished.agent_id,
                                "prompt_id": &finished.prompt_id,
                                "provider_run_id": &finished.provider_run_id,
                                "error": error.to_string(),
                            }),
                        );
                        if crate::durable_state::is_retryable_durable_write_error(&error) {
                            self.provider_store
                                .schedule_finished_structured_prompt_submit_retry(
                                    crate::provider::FinishedProviderPromptSubmitJob {
                                        session_id: finished.session_id,
                                        provider_run_id: finished.provider_run_id,
                                        agent_id: finished.agent_id,
                                        prompt_id: finished.prompt_id,
                                        result: Ok(acknowledgement),
                                        settlement_retry_attempt,
                                    },
                                );
                        }
                    }
                }
                Err(error) => {
                    let provider_run = self.provider_store.get_run(&finished.provider_run_id).ok();
                    let mut _settlement_claim = None;
                    if let Some(provider_run) = provider_run.as_ref() {
                        match self.prepare_failed_prompt_resume_invalidation(
                            &finished.session_id,
                            &finished.agent_id,
                            &finished.prompt_id,
                            provider_run,
                            &error,
                        ) {
                            Ok(FailedPromptResumePreparation::NotRequired) => {}
                            Ok(FailedPromptResumePreparation::Cleared(claim)) => {
                                _settlement_claim = Some(claim);
                            }
                            Ok(FailedPromptResumePreparation::Superseded) => {
                                crate::logging::warn_with_fields(
                                    "daemon.prompt_delivery",
                                    "structured prompt failure was superseded by newer delivery state",
                                    serde_json::json!({
                                        "session_id": &finished.session_id,
                                        "agent_id": &finished.agent_id,
                                        "prompt_id": &finished.prompt_id,
                                        "provider_run_id": &finished.provider_run_id,
                                    }),
                                );
                                continue;
                            }
                            Err(clear_error) => {
                                crate::logging::warn_with_fields(
                                    "durable_state.recovery",
                                    "failed to durably invalidate provider resume after prompt dispatch failure",
                                    serde_json::json!({
                                        "session_id": &finished.session_id,
                                        "agent_id": &finished.agent_id,
                                        "prompt_id": &finished.prompt_id,
                                        "provider_run_id": &finished.provider_run_id,
                                        "error": clear_error.to_string(),
                                    }),
                                );
                                if crate::durable_state::is_retryable_durable_write_error(
                                    &clear_error,
                                ) {
                                    self.provider_store
                                        .schedule_finished_structured_prompt_submit_retry(
                                            crate::provider::FinishedProviderPromptSubmitJob {
                                                session_id: finished.session_id,
                                                provider_run_id: finished.provider_run_id,
                                                agent_id: finished.agent_id,
                                                prompt_id: finished.prompt_id,
                                                result: Err(error),
                                                settlement_retry_attempt,
                                            },
                                        );
                                }
                                continue;
                            }
                        }
                        if let Ok(outcome) = self.provider_store.terminate_run_provider_only(
                            &finished.session_id,
                            &finished.provider_run_id,
                        ) {
                            let _ = self.clear_active_provider_run_session_pointer(
                                &finished.session_id,
                                outcome.run().id(),
                            );
                            self.provider_run_projection.update(outcome.into_run());
                        }
                    }
                    let diagnostic = format!("Provider prompt dispatch failed: {error}");
                    if let Ok(run) = self
                        .provider_store
                        .record_terminal_diagnostic(&finished.provider_run_id, diagnostic.clone())
                    {
                        self.provider_run_projection.update(run);
                    }
                    self.record_provider_failure_output(
                        &finished.session_id,
                        &finished.provider_run_id,
                        &finished.agent_id,
                        &diagnostic,
                    );
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
            let abort_error = finished.result.err().map(|error| error.to_string());
            if let Some(error) = abort_error.as_deref() {
                let recipients = self
                    .attachment_store
                    .list_session_attachment_ids(&finished.session_id);
                self.record_notice(
                    &finished.session_id,
                    Some(&finished.provider_run_id),
                    recipients,
                    format!("Prompt cancellation dispatch failed after acknowledgement: {error}"),
                );
                if let Ok(provider_run) = self.provider_store.get_run(&finished.provider_run_id) {
                    if let Some(agent_id) = provider_run.agent_instance_id() {
                        if let Ok(session) = self.session_store.get_session(&finished.session_id) {
                            if let Some(prompt) = self
                                .prompt_state_owner
                                .active_prompt_for_agent(&session, agent_id)
                                .filter(|prompt| {
                                    prompt.status() == crate::session::PromptStatus::Cancelling
                                })
                            {
                                let _ = self.settle_failed_local_prompt_without_advance(
                                    &finished.session_id,
                                    agent_id,
                                    prompt.id(),
                                    &finished.provider_run_id,
                                    &format!("Provider prompt cancellation failed: {error}"),
                                );
                            }
                        }
                    }
                }
            } else if let Ok(provider_run) = self.provider_store.get_run(&finished.provider_run_id)
            {
                // A successful structured-provider abort RPC is the
                // authoritative acknowledgement that the interrupted turn no
                // longer owns the agent. Some providers do not emit a usable
                // terminal event afterwards, so settle the cancelling prompt
                // from this acknowledgement instead of leaving it WORKING.
                if let Some(agent_id) = provider_run.agent_instance_id() {
                    if let Ok(session) = self.session_store.get_session(&finished.session_id) {
                        if let Some(prompt) = self
                            .prompt_state_owner
                            .active_prompt_for_agent(&session, agent_id)
                        {
                            let _ = self.workflow_cancel_prompt(&finished.session_id, &prompt);
                        }
                    }
                    let _ = self.finalize_local_prompt_cancellation_with_queued_advance(
                        &finished.session_id,
                        agent_id,
                        Some(&finished.provider_run_id),
                    );
                }
            }
        }
    }

    fn prepare_failed_prompt_resume_invalidation(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        provider_run: &crate::provider::RuntimeProviderRun,
        error: &DaemonError,
    ) -> Result<FailedPromptResumePreparation, DaemonError> {
        if crate::app::failed_provider_resume_state_replacement(provider_run, error).is_none() {
            return Ok(FailedPromptResumePreparation::NotRequired);
        }
        let provider = provider_run.adapter_key();
        let session = self.session_store.get_session(session_id)?;
        let Some(settlement_claim) = self
            .prompt_state_owner
            .try_claim_active_prompt_delivery_settlement(
                &session,
                agent_id,
                prompt_id,
                provider_run.id(),
            )
        else {
            return Ok(FailedPromptResumePreparation::Superseded);
        };
        let Some(active_prompt) = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
        else {
            return Ok(FailedPromptResumePreparation::Superseded);
        };
        let Some(stale_provider_session_id) = active_prompt
            .durable_delivery_provider_session_id()
            .map(str::to_string)
        else {
            return Err(DaemonError::LocalTransport {
                operation: "invalidate provider resume after prompt delivery failure",
                message: format!(
                    "prompt `{prompt_id}` has no durable provider session identity for run `{}`",
                    provider_run.id()
                ),
            });
        };
        if self
            .compare_and_mark_active_prompt_delivery_failure(
                session_id,
                agent_id,
                prompt_id,
                provider_run.id(),
                &stale_provider_session_id,
                (
                    active_prompt.status(),
                    crate::session::PromptStatus::Cancelling,
                ),
            )?
            .is_none()
        {
            return Ok(FailedPromptResumePreparation::Superseded);
        }
        let cleared = self
            .agent_store
            .clear_provider_resume_state_durably_if_matches(
                &self.durable_state_store,
                agent_id,
                provider,
                &stale_provider_session_id,
                provider_run.id(),
                "failed_provider_resume_state_cleared",
            )?;
        match cleared {
            crate::agent::ProviderResumeClearOutcome::Cleared => {}
            crate::agent::ProviderResumeClearOutcome::AlreadyAbsent => {
                return Ok(FailedPromptResumePreparation::Cleared(settlement_claim));
            }
            crate::agent::ProviderResumeClearOutcome::Superseded {
                current_provider_session_id,
            } => {
                self.restore_active_prompt_after_resume_superseded(
                    session_id,
                    agent_id,
                    prompt_id,
                    provider_run.id(),
                    &stale_provider_session_id,
                    &current_provider_session_id,
                )?;
                return Ok(FailedPromptResumePreparation::Superseded);
            }
        }
        self.record_notice(
            provider_run.session_id(),
            Some(provider_run.id()),
            self.attachment_store
                .list_session_attachment_ids(provider_run.session_id()),
            crate::provider::provider_resume_failure_notice(provider, &stale_provider_session_id)
                .unwrap_or_else(|| {
                    format!(
                        "Provider session `{stale_provider_session_id}` is no longer available. Chariox cleared it from the agent profile so the next prompt can start a new durable provider session."
                    )
                }),
        );
        Ok(FailedPromptResumePreparation::Cleared(settlement_claim))
    }

    fn finish_structured_prompt_delivery(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        provider_run_id: &str,
        acknowledgement: &crate::provider::ProviderPromptSubmitAcknowledgement,
    ) -> Result<(), DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let Some(_settlement_claim) = self
            .prompt_state_owner
            .try_claim_active_prompt_delivery_settlement(
                &session,
                agent_id,
                prompt_id,
                provider_run_id,
            )
        else {
            return Ok(());
        };
        let run = self.provider_store.get_run(provider_run_id)?;
        if let Some(run_agent_id) = run.agent_instance_id() {
            self.agent_store.set_agent_runtime_profile_durably(
                &self.durable_state_store,
                run_agent_id,
                run.provider(),
                Some(run.model().to_string()),
                run.variant().map(str::to_string),
                Some(run.account_profile().to_string()),
                acknowledgement.resume_state.clone(),
                Some(run.id()),
                Some("prompt_delivery_acknowledged"),
            )?;
        }
        let run = self
            .provider_store
            .apply_prompt_submit_acknowledgement(provider_run_id, acknowledgement)?;
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
                active_tool_ids: std::collections::BTreeSet::new(),
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

    pub(super) fn note_prompt_tool_output(
        &self,
        provider_run_id: &str,
        merge_key: Option<&str>,
        bytes: &[u8],
    ) {
        if let Some(state) = self.prompt_activity.write().get_mut(provider_run_id) {
            state.observe_provider_tool(merge_key, bytes);
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
                active_tool_ids: std::collections::BTreeSet::new(),
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
        claim_id: &str,
        agent_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<(), DaemonError> {
        if self.prompt_workspace_claims.contains(claim_id) {
            return Ok(());
        }
        let session = self.session_store.get_session(session_id)?;
        let agent = self.agent_store.get_agent(agent_id)?;
        let workspace_id = session.workspace_id().to_string();
        let worktree_id = agent
            .worktree_id()
            .unwrap_or_else(|| session.worktree_id())
            .to_string();
        let claim = self.workspace_coordinator.acquire_worktree_write_claim(
            workspace_id,
            worktree_id,
            session_id,
            Some(format!("{workflow_run_id}:{workflow_node_run_id}")),
            "workflow_node_dispatch",
        )?;
        self.prompt_workspace_claims
            .insert(claim_id.to_string(), claim);
        Ok(())
    }

    pub(super) fn ensure_workflow_prompt_workspace_claim(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<Option<bool>, DaemonError> {
        let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(None);
        };
        let claim_id =
            self.workflow_dispatch_claim_id(session_id, workflow_run_id, workflow_node_run_id);
        if self.prompt_workspace_claims.contains(&claim_id) {
            return Ok(Some(false));
        }
        self.acquire_workflow_node_workspace_claim(
            session_id,
            &claim_id,
            prompt.target_agent_id(),
            workflow_run_id,
            workflow_node_run_id,
        )?;
        Ok(Some(true))
    }
}
