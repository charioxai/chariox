//! Provider prompt failure forwarding, local settlement, and substitute reruns.

use super::*;

impl KernelRuntimeState {
    pub(super) async fn fail_owned_provider_prompt(
        &self,
        session_id: &str,
        provider_run_id: &str,
        message: &str,
        project_failure_output: bool,
    ) -> Result<(), DaemonError> {
        self.fail_owned_provider_prompt_with_termination(
            session_id,
            provider_run_id,
            message,
            project_failure_output,
            None,
        )
        .await
    }

    pub(super) async fn fail_owned_provider_prompt_with_termination(
        &self,
        session_id: &str,
        provider_run_id: &str,
        message: &str,
        project_failure_output: bool,
        provider_termination: Option<crate::provider::ProviderRunTermination>,
    ) -> Result<(), DaemonError> {
        self.fail_owned_provider_prompt_with_termination_if_matches(
            session_id,
            provider_run_id,
            message,
            project_failure_output,
            None,
            provider_termination,
        )
        .await
        .map(|_| ())
    }

    pub(super) async fn fail_owned_provider_prompt_with_termination_if_matches(
        &self,
        session_id: &str,
        provider_run_id: &str,
        message: &str,
        project_failure_output: bool,
        expected_prompt_id: Option<&str>,
        provider_termination: Option<crate::provider::ProviderRunTermination>,
    ) -> Result<bool, DaemonError> {
        let owned = &self.owned;
        let provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let agent_id = provider_run
            .agent_instance_id()
            .map(str::to_string)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "provider run has no agent".to_string(),
            })?;
        // MP-08/MP-10: liveness may already have replaced this run and
        // promoted its backlog before an asynchronous dispatch reports failure.
        let session = owned.session_store.get_session(session_id)?;
        let Some(active_prompt) = owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
        else {
            return Ok(false);
        };
        let bound_to_another_run = active_prompt
            .durable_delivery_provider_run_id()
            .is_some_and(|current| current != provider_run_id);
        let ended_run_replaced = provider_run.state() == crate::provider::ProviderRunState::Ended
            && owned
                .provider_store
                .get_run_for_agent(session_id, &agent_id)
                .is_some_and(|current| current.id() != provider_run_id);
        if bound_to_another_run || ended_run_replaced {
            return Ok(false);
        }
        let guarded_prompt_id = active_prompt.id().to_string();
        if self
            .try_provider_auth_recovery(&provider_run, message, expected_prompt_id)
            .await?
        {
            return Ok(true);
        }
        let safe_message = crate::provider::sanitize_provider_diagnostic(message);
        let safe_message = if safe_message.is_empty() {
            "provider reported an error".to_string()
        } else {
            safe_message
        };

        let failed_attempt = FailedProviderAttempt {
            provider_run: &provider_run,
            agent_id: &agent_id,
            message,
            safe_message: &safe_message,
            project_failure_output,
            record_diagnostic: expected_prompt_id.is_some(),
            termination: provider_termination.as_ref(),
        };
        let mut expected_active_prompt = None;
        let mut expected_completion = None;
        let mut attempt_recorded = false;
        if let Some(expected_prompt_id) = expected_prompt_id {
            let session = owned.session_store.get_session(session_id)?;
            let Some(active_prompt) = owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &agent_id)
            else {
                return Ok(false);
            };
            if active_prompt.id() != expected_prompt_id || active_prompt.is_external() {
                return Ok(false);
            }
            if self
                .settle_leased_workflow_provider_failure_if_matches(
                    session_id,
                    &agent_id,
                    provider_run_id,
                    Some(expected_prompt_id),
                    provider_termination.clone(),
                )
                .await?
            {
                let diagnosed = owned
                    .provider_store
                    .record_terminal_diagnostic(provider_run_id, message.to_string())?;
                owned.provider_run_projection.update(diagnosed);
                self.clear_failed_provider_resume_state_from_message(&provider_run, message)?;
                self.retire_owned_provider_run_after_terminal_failure(session_id, provider_run_id)
                    .await;
                return Ok(true);
            }
            // Reruns dispatch to a new provider run whose output pump can fail
            // back into this path; keep that future off the caller's stack.
            match Box::pin(self.rerun_failed_turn_on_substitute(&failed_attempt, &active_prompt))
                .await?
            {
                SubstituteRerun::Started => return Ok(true),
                SubstituteRerun::Exhausted => attempt_recorded = true,
                SubstituteRerun::NotApplicable => {}
            }
            let completion = match owned
                .fail_local_prompt_without_advance_with_termination_if_matches(
                    session_id,
                    &agent_id,
                    Some(provider_run_id),
                    Some(expected_prompt_id),
                    provider_termination.clone(),
                ) {
                Ok(completion) => completion,
                Err(DaemonError::NoActivePrompt { .. }) => return Ok(false),
                Err(error) => return Err(error),
            };
            let Some(completion) = completion else {
                return Ok(false);
            };
            expected_active_prompt = Some(active_prompt);
            expected_completion = Some(completion);
        } else if self
            .settle_leased_workflow_provider_failure(
                session_id,
                &agent_id,
                provider_run_id,
                provider_termination.clone(),
            )
            .await?
        {
            self.clear_failed_provider_resume_state_from_message(&provider_run, message)?;
            self.retire_owned_provider_run_after_terminal_failure(session_id, provider_run_id)
                .await;
            return Ok(true);
        }

        let session = owned.session_store.get_session(session_id)?;
        let active_prompt = if let Some(active_prompt) = expected_active_prompt {
            active_prompt
        } else {
            let Some(active_prompt) = owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &agent_id)
            else {
                self.clear_failed_provider_resume_state_from_message(&provider_run, message)?;
                return Ok(false);
            };
            if active_prompt.id() != guarded_prompt_id {
                return Ok(false);
            }
            if active_prompt.is_external() {
                self.clear_failed_provider_resume_state_from_message(&provider_run, message)?;
                let _ = owned.clear_prompt_activity(provider_run_id);
                let _ = owned.sync_focused_provider_run_if_idle(session_id);
                let _ = owned.session_snapshot(session_id);
                return Ok(true);
            }
            // Reruns dispatch to a new provider run whose output pump can fail
            // back into this path; keep that future off the caller's stack.
            match Box::pin(self.rerun_failed_turn_on_substitute(&failed_attempt, &active_prompt))
                .await?
            {
                SubstituteRerun::Started => return Ok(true),
                SubstituteRerun::Exhausted => attempt_recorded = true,
                SubstituteRerun::NotApplicable => {}
            }
            active_prompt
        };
        if !attempt_recorded {
            self.record_failed_provider_attempt(&failed_attempt).await?;
        }
        let _ = self.inject_metaagent_turn_failure_event(
            session_id,
            &agent_id,
            &active_prompt,
            Some(provider_run_id),
            &safe_message,
        );
        // Provider EOF/error cannot promote a partial answer to done. Task
        // bookkeeping is best-effort: the prompt failure path below always runs.
        let task_settlement = if owned.config_projection.snapshot().room_agent_tools {
            let settle = || -> Result<_, DaemonError> {
                let prepared = crate::app::KernelPreparedPromptSubmission {
                    session_id: session_id.into(),
                    prompt: active_prompt.clone(),
                    force_queue: false,
                    refresh_projection: false,
                };
                owned.admit_agent_task(&prepared)?;
                let task = owned
                    .durable_state_store
                    .agent_tasks(Some(session_id), Some(&agent_id))?
                    .into_iter()
                    .find(|t| t.prompt_id == active_prompt.id())
                    .ok_or_else(|| {
                        crate::durable_state::agent_lifecycle::error("failed turn has no task")
                    })?;
                crate::durable_state::agent_lifecycle::block_failed_turn(&owned.durable_state_store,&task,format!("Provider run failed: {safe_message}. Owner must reconcile and resume or cancel"))?;
                Ok(owned
                    .durable_state_store
                    .agent_tasks(Some(session_id), Some(&agent_id))?
                    .into_iter()
                    .find(|t| t.task_id == task.task_id))
            };
            match settle() {
                Ok(current) => current.map(|t| (t, false)),
                Err(error) => {
                    tracing::warn!(error=%crate::secret_redaction::redact_secrets(&error.to_string()), "MP-08/MP-09/MP-10/MP-11 A02: failed turn task disposition retained");
                    None
                }
            }
        } else {
            None
        };
        let workflow_failed = active_prompt.workflow_run_id().is_some();
        if workflow_failed {
            owned.workflow_fail_provider_prompt_without_queue_advance(
                session_id,
                &active_prompt,
                Some(provider_run_id),
                &safe_message,
            )?;
        }
        let completion = if let Some(completion) = expected_completion {
            Some(completion)
        } else {
            owned.fail_local_prompt_without_advance_with_termination_if_matches(
                session_id,
                &agent_id,
                Some(provider_run_id),
                Some(active_prompt.id()),
                provider_termination,
            )?
        };
        if completion.is_none() {
            return Ok(false);
        }
        Box::pin(self.finish_agent_task_settlement(task_settlement, &active_prompt)).await?;
        // Recorded before any queued prompt or substitute is started, so the
        // agent's next turn carries the note.
        owned.record_failed_request(
            session_id,
            provider_run_id,
            provider_run.adapter_key(),
            &agent_id,
            &active_prompt,
            message,
        );
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
        if queued_prompt_pending {
            let started_next = self
                .with_app_side_effect(|app| {
                    app.ensure_prompt_provider_run_for_agent(session_id, &agent_id)?;
                    app.advance_next_queued_prompt(session_id, &agent_id)
                })
                .await?;
            if started_next.is_some() {
                owned.agent_store.clear_local_prompt_error(&agent_id)?;
                owned
                    .agent_store
                    .set_agent_state(&agent_id, crate::agent::AgentState::Working)?;
                let _ = owned.session_snapshot(session_id)?;
            }
        }
        if workflow_failed {
            // Workflow failures bypass the non-workflow claim-release retry above. Sweep after
            // queued-prompt/provider recovery so newly eligible work is not left pending.
            self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
        }
        Ok(true)
    }

    /// Keeps a failed provider attempt visible and retires its run.
    pub(super) async fn record_failed_provider_attempt(
        &self,
        attempt: &FailedProviderAttempt<'_>,
    ) -> Result<(), DaemonError> {
        let provider_run = attempt.provider_run;
        let session_id = provider_run.session_id();
        self.clear_failed_provider_resume_state_from_message(provider_run, attempt.message)?;
        if attempt.record_diagnostic {
            let diagnosed = self
                .owned
                .provider_store
                .record_terminal_diagnostic(provider_run.id(), attempt.message.to_string())?;
            self.owned.provider_run_projection.update(diagnosed);
        }
        if attempt.project_failure_output {
            self.owned.record_provider_failure_output(
                session_id,
                provider_run.id(),
                attempt.agent_id,
                attempt.safe_message,
            );
        }
        self.retire_owned_provider_run_after_terminal_failure(session_id, provider_run.id())
            .await;
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

    pub(super) async fn retire_owned_provider_run_after_terminal_failure(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) {
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
        provider_termination: Option<crate::provider::ProviderRunTermination>,
    ) -> Result<bool, DaemonError> {
        self.settle_leased_workflow_provider_failure_if_matches(
            session_id,
            agent_id,
            provider_run_id,
            None,
            provider_termination,
        )
        .await
    }

    async fn settle_leased_workflow_provider_failure_if_matches(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
        expected_prompt_id: Option<&str>,
        provider_termination: Option<crate::provider::ProviderRunTermination>,
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
        if let Some(expected_prompt_id) = expected_prompt_id {
            let session = self.owned.session_store.get_session(session_id)?;
            let Some(active_prompt) = self
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, agent_id)
            else {
                return Ok(false);
            };
            if active_prompt.id() != expected_prompt_id {
                return Ok(false);
            }
        }
        // The home learns failure through the same correlated, replayable runtime
        // projection as completion. Worker settlement must not wait for the home
        // to be reachable or admit another turn on this failed provider.
        let completion = match self
            .owned
            .fail_local_prompt_without_advance_with_termination_if_matches(
                session_id,
                agent_id,
                Some(provider_run_id),
                expected_prompt_id,
                provider_termination,
            ) {
            Ok(completion) => completion,
            Err(DaemonError::NoActivePrompt { .. }) if expected_prompt_id.is_some() => {
                return Ok(false)
            }
            Err(error) => return Err(error),
        };
        if completion.is_none() {
            return Ok(false);
        }
        Ok(true)
    }

    pub(super) async fn settle_repeated_structured_poll_failure_if_matches(
        &self,
        session_id: &str,
        provider_run_id: &str,
        expected_prompt_id: Option<&str>,
        error: &DaemonError,
    ) -> Result<bool, DaemonError> {
        let Some(expected_prompt_id) = expected_prompt_id else {
            return Ok(false);
        };
        let diagnostic =
            format!("Structured provider output polling failed after repeated failures: {error}");
        let termination = crate::provider::ProviderRunTermination::explicit_provider_error(
            &diagnostic,
            crate::session::unix_epoch_ms(),
        );
        self.fail_owned_provider_prompt_with_termination_if_matches(
            session_id,
            provider_run_id,
            &diagnostic,
            false,
            Some(expected_prompt_id),
            Some(termination),
        )
        .await
    }
}

/// One provider attempt at a turn that the provider failed.
pub(super) struct FailedProviderAttempt<'a> {
    pub(super) provider_run: &'a crate::provider::RuntimeProviderRun,
    pub(super) agent_id: &'a str,
    pub(super) message: &'a str,
    pub(super) safe_message: &'a str,
    pub(super) project_failure_output: bool,
    pub(super) record_diagnostic: bool,
    pub(super) termination: Option<&'a crate::provider::ProviderRunTermination>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SubstituteRerun {
    /// The turn is not a local provider failure with a substitute left.
    NotApplicable,
    /// The turn is running again on a substitute.
    Started,
    /// The failed attempt was recorded, but no substitute could start.
    Exhausted,
}
