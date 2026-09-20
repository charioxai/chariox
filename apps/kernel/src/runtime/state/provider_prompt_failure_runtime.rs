//! Provider prompt failure forwarding, local settlement, and substitute activation.

use super::*;

#[cfg(test)]
#[derive(Clone)]
pub(super) struct ProviderRetirementTestBarrier {
    reached: Arc<Notify>,
    release: Arc<Notify>,
}

#[cfg(test)]
impl ProviderRetirementTestBarrier {
    pub(super) async fn wait_until_reached(&self) {
        self.reached.notified().await;
    }

    pub(super) fn release(&self) {
        self.release.notify_one();
    }
}

#[cfg(test)]
fn provider_retirement_test_barriers(
) -> &'static std::sync::Mutex<BTreeMap<String, ProviderRetirementTestBarrier>> {
    static BARRIERS: std::sync::OnceLock<
        std::sync::Mutex<BTreeMap<String, ProviderRetirementTestBarrier>>,
    > = std::sync::OnceLock::new();
    BARRIERS.get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
}

impl KernelRuntimeState {
    #[cfg(test)]
    pub(super) fn install_provider_retirement_test_barrier(
        &self,
        provider_run_id: &str,
    ) -> ProviderRetirementTestBarrier {
        let barrier = ProviderRetirementTestBarrier {
            reached: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
        };
        provider_retirement_test_barriers()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(provider_run_id.to_string(), barrier.clone());
        barrier
    }

    pub(super) async fn fail_owned_provider_prompt(
        &self,
        session_id: &str,
        provider_run_id: &str,
        message: &str,
        project_failure_output: bool,
    ) -> Result<(), DaemonError> {
        let owned = &self.owned;
        let provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let agent_id = provider_run
            .agent_instance_id()
            .map(str::to_string)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "provider run has no agent".to_string(),
            })?;

        // A delayed failure from a superseded run must not mutate the resume
        // state or prompt owned by its replacement. Durable prompt delivery is
        // authoritative when present; the session pointer covers prompts that
        // have started but have not reached durable provider dispatch yet.
        let initial_session = owned.session_store.get_session(session_id)?;
        let initial_active_prompt_id = owned
            .prompt_state_owner
            .active_prompt_for_agent(&initial_session, &agent_id)
            .map(|prompt| prompt.id().to_string());
        if initial_active_prompt_id.is_some()
            && !owned.provider_run_has_active_prompt(session_id, &provider_run)?
        {
            self.retire_owned_provider_run(session_id, provider_run_id)
                .await;
            return Ok(());
        }

        self.clear_failed_provider_resume_state_from_message(&provider_run, message)?;

        if self
            .settle_leased_workflow_provider_failure(
                session_id,
                &agent_id,
                provider_run_id,
                initial_active_prompt_id.as_deref(),
            )
            .await?
        {
            self.retire_owned_provider_run(session_id, provider_run_id)
                .await;
            return Ok(());
        }

        let substitution_reason = crate::provider::classify_provider_substitutable_failure_text(
            provider_run.adapter_key(),
            message,
        );
        let session = owned.session_store.get_session(session_id)?;
        let Some(active_prompt) = owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
        else {
            // Provider exit reconciliation can settle the prompt before a native
            // StopFailure hook is drained. The late authoritative failure must
            // still advance the substitute chain. Reserve prompt admission
            // before cleanup, then re-read profile and run ownership after the
            // asynchronous retirement so stale observations cannot replace a
            // prompt or provider run that arrived in the meantime.
            let profile_transition = match owned
                .prompt_state_owner
                .claim_idle_agent_profile_transition(&session, &agent_id)
            {
                Ok(claim) => claim,
                Err(_) => {
                    self.retire_owned_provider_run(session_id, provider_run_id)
                        .await;
                    return Ok(());
                }
            };
            self.retire_owned_provider_run(session_id, provider_run_id)
                .await;
            let session = owned.session_store.get_session(session_id)?;
            let agent = owned.agent_store.get_agent(&agent_id)?;
            let failed_profile_is_still_active = provider_run.provider()
                == crate::provider::provider_id_for_launch(agent.provider())
                && provider_run.account_profile() == agent.provider_account_profile()
                && provider_run.model() == agent.model().unwrap_or("default")
                && provider_run.variant() == agent.effort();
            let failed_run_still_owns_transition = session
                .active_provider_run_id()
                .is_none_or(|active_run_id| active_run_id == provider_run_id)
                && !owned.provider_store.list_runs().into_iter().any(|run| {
                    run.id() != provider_run_id
                        && run.session_id() == session_id
                        && run.agent_instance_id() == Some(agent_id.as_str())
                        && run.started_at_ms() >= provider_run.started_at_ms()
                });
            if let Some(reason) = substitution_reason
                .filter(|_| failed_profile_is_still_active && failed_run_still_owns_transition)
            {
                self.activate_substitute_after_provider_failure(
                    session_id,
                    &agent_id,
                    provider_run_id,
                    &reason,
                    Some(profile_transition),
                )
                .await;
            } else {
                self.finish_unlaunched_local_provider_failure_transition(
                    session_id,
                    &agent_id,
                    profile_transition,
                )
                .await?;
            }
            return Ok(());
        };
        if !owned.provider_run_has_active_prompt(session_id, &provider_run)? {
            self.retire_owned_provider_run(session_id, provider_run_id)
                .await;
            return Ok(());
        }
        if active_prompt.is_external() {
            let _ = owned.clear_prompt_activity(provider_run_id);
            let _ = owned.sync_focused_provider_run_if_idle(session_id);
            let _ = owned.session_snapshot(session_id);
            return Ok(());
        }
        let active_prompt_id = active_prompt.id().to_string();
        self.retire_owned_provider_run(session_id, provider_run_id)
            .await;
        let session = owned.session_store.get_session(session_id)?;
        let Some(active_prompt) = owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .filter(|prompt| prompt.id() == active_prompt_id)
        else {
            return Ok(());
        };
        if project_failure_output {
            owned.record_provider_failure_output(session_id, provider_run_id, &agent_id, message);
        }
        let _ = self.inject_metaagent_turn_failure_event(
            session_id,
            &agent_id,
            &active_prompt,
            Some(provider_run_id),
            message,
        );
        let workflow_failed = active_prompt.workflow_run_id().is_some();
        if workflow_failed {
            owned.workflow_fail_provider_prompt_without_queue_advance(
                session_id,
                &active_prompt,
                Some(provider_run_id),
                message,
            )?;
        }
        let completion = owned.fail_local_prompt_without_advance_if_matches(
            session_id,
            &agent_id,
            Some(provider_run_id),
            &active_prompt_id,
        )?;
        // Settle the failed turn first, then choose its successor provider before
        // preparing any queued work. Otherwise admission retries the exhausted
        // account and can return before automatic substitution is reached.
        let substituted = if let Some(reason) = substitution_reason {
            self.activate_substitute_after_provider_failure(
                session_id,
                &agent_id,
                provider_run_id,
                &reason,
                None,
            )
            .await
        } else {
            false
        };
        if workflow_failed {
            let dispatches = owned.workflow_maybe_start_next_queued_prompt(session_id);
            owned
                .persist_workflow_runtime_session(session_id, "workflow_provider_prompt_failed")?;
            self.spawn_workflow_prompt_dispatches(dispatches);
        }
        if completion
            .as_ref()
            .is_some_and(|completion| completion.released_claim)
            && active_prompt.workflow_run_id().is_none()
        {
            self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
        }
        let queued_prompt_pending = owned
            .prompt_state_owner
            .peek_next_queued_prompt(&owned.session_store.get_session(session_id)?, &agent_id)
            .is_some();
        if queued_prompt_pending && !substituted {
            self.with_app_side_effect(|app| {
                app.ensure_prompt_provider_run_for_agent(session_id, &agent_id)
            })
            .await?;
        }
        Ok(())
    }

    fn clear_failed_provider_resume_state_from_message(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        message: &str,
    ) -> Result<bool, DaemonError> {
        let Some(_) = crate::app::failed_provider_resume_state_replacement_from_message(
            provider_run,
            message,
        ) else {
            return Ok(false);
        };
        Ok(matches!(
            self.apply_provider_resume_state_replacement(
                provider_run,
                provider_run
                    .resume_state()
                    .provider_session_id(provider_run.adapter_key())
                    .expect("classified resume failure must retain its provider session"),
                "failed_provider_resume_state_cleared",
            )?,
            crate::agent::ProviderResumeClearOutcome::Cleared
                | crate::agent::ProviderResumeClearOutcome::AlreadyAbsent
        ))
    }

    pub(super) fn clear_unresponsive_provider_resume_state(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        expected_provider_session_id: &str,
    ) -> Result<crate::agent::ProviderResumeClearOutcome, DaemonError> {
        let Some(agent_id) = provider_run.agent_instance_id() else {
            return Ok(crate::agent::ProviderResumeClearOutcome::AlreadyAbsent);
        };
        self.clear_provider_resume_state_for_identity(
            provider_run.session_id(),
            agent_id,
            provider_run.adapter_key(),
            expected_provider_session_id,
            provider_run.id(),
            "unresponsive_provider_resume_state_cleared",
        )
    }

    pub(super) fn clear_provider_resume_state_for_identity(
        &self,
        session_id: &str,
        agent_id: &str,
        provider: &str,
        expected_provider_session_id: &str,
        provider_run_id: &str,
        reason: &'static str,
    ) -> Result<crate::agent::ProviderResumeClearOutcome, DaemonError> {
        let outcome = self
            .owned
            .agent_store
            .clear_provider_resume_state_durably_if_matches(
                &self.owned.durable_state_store,
                agent_id,
                provider,
                expected_provider_session_id,
                provider_run_id,
                reason,
            )?;
        if matches!(outcome, crate::agent::ProviderResumeClearOutcome::Cleared) {
            self.owned.record_notice(
                session_id,
                Some(provider_run_id),
                self.owned
                    .attachment_store
                    .list_session_attachment_ids(session_id),
                crate::provider::provider_resume_failure_notice(
                    provider,
                    expected_provider_session_id,
                )
                .unwrap_or_else(|| {
                    format!(
                        "Provider session `{expected_provider_session_id}` is no longer available. Chariox cleared it from the agent profile so the next prompt can start a new durable provider session."
                    )
                }),
            );
        }
        Ok(outcome)
    }

    fn apply_provider_resume_state_replacement(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        expected_provider_session_id: &str,
        reason: &'static str,
    ) -> Result<crate::agent::ProviderResumeClearOutcome, DaemonError> {
        let Some(agent_id) = provider_run.agent_instance_id() else {
            return Ok(crate::agent::ProviderResumeClearOutcome::AlreadyAbsent);
        };
        self.clear_provider_resume_state_for_identity(
            provider_run.session_id(),
            agent_id,
            provider_run.adapter_key(),
            expected_provider_session_id,
            provider_run.id(),
            reason,
        )
    }

    pub(super) async fn retire_owned_provider_run(&self, session_id: &str, provider_run_id: &str) {
        #[cfg(test)]
        let barrier = {
            provider_retirement_test_barriers()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(provider_run_id)
        };
        #[cfg(test)]
        if let Some(barrier) = barrier {
            barrier.reached.notify_one();
            barrier.release.notified().await;
        }
        let owned = &self.owned;
        if let Ok(outcome) = owned
            .provider_store
            .terminate_run_provider_only(session_id, provider_run_id)
        {
            let _ = owned.clear_active_provider_run_session_pointer(session_id, outcome.run().id());
            owned.provider_run_projection.update(outcome.into_run());
        }
        let (_, process_key) = self
            .with_app_side_effect(|app| {
                crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(provider_run_id)
            })
            .await
            .unwrap_or((false, None));
        owned.remove_provider_process_tracking_for_run(provider_run_id, process_key);
        owned
            .connector_adapter_processes
            .shutdown_run(provider_run_id)
            .await;
    }

    pub(super) async fn settle_leased_workflow_provider_failure(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
        expected_prompt_id: Option<&str>,
    ) -> Result<bool, DaemonError> {
        let leased_context = self
            .with_app_side_effect(|app| {
                crate::app::RemoteLeaseRuntime::new(app)
                    .leased_workflow_turn_context_for_provider_run(provider_run_id)
            })
            .await;
        let Some(_) = leased_context else {
            return Ok(false);
        };
        // The home learns failure through the same correlated, replayable runtime
        // projection as completion. Worker settlement must not wait for the home
        // to be reachable or admit another turn on this failed provider.
        if let Some(expected_prompt_id) = expected_prompt_id {
            self.owned.fail_local_prompt_without_advance_if_matches(
                session_id,
                agent_id,
                Some(provider_run_id),
                expected_prompt_id,
            )?;
        }
        Ok(true)
    }

    pub(super) async fn activate_substitute_after_provider_failure(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
        reason: &str,
        profile_transition: Option<crate::runtime::prompt_state::AgentProfileTransitionClaim>,
    ) -> bool {
        // Failure reconciliation is nested inside output/liveness/restart
        // polling. Keep the account-transfer and worker-confirmation future
        // off those callers' stacks, including when this branch is not taken.
        match Box::pin(
            self.activate_next_agent_substitute_after_failure_with_claim(
                session_id,
                agent_id,
                reason,
                profile_transition,
            ),
        )
        .await
        {
            Ok(activated) => activated,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.provider",
                    "automatic substitute activation after provider failure failed",
                    serde_json::json!({
                        "session_id": session_id,
                        "agent_id": agent_id,
                        "provider_run_id": provider_run_id,
                        "error": error.to_string(),
                    }),
                );
                false
            }
        }
    }
}
