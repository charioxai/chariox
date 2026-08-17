//! Startup reconciliation for durable prompt work left active by a previous kernel process.

use super::*;

/// Prefix stamped onto the `source_attachment_id` of every kernel-internal
/// restart-recovery dispatch. It identifies an envelope that carries provider
/// resume text (not user input) so downstream fanout can suppress it.
pub(crate) const KERNEL_RECOVERY_ATTACHMENT_PREFIX: &str = "kernel-recovery:";

/// Whether an attachment id belongs to a kernel-internal restart-recovery
/// dispatch. Kept as a shared helper so every fanout/persistence boundary
/// checks the same marker.
pub(crate) fn is_internal_recovery_prompt_attachment(attachment_id: &str) -> bool {
    attachment_id.starts_with(KERNEL_RECOVERY_ATTACHMENT_PREFIX)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DurableRestartRecoverySummary {
    pub(crate) cancelled_local_prompts_finalized: usize,
    pub(crate) accepted_local_redispatched: usize,
    pub(crate) uncertain_original_redispatched: usize,
    pub(crate) provider_continuations_dispatched: usize,
    pub(crate) remote_reconciliations_started: usize,
    pub(crate) uncertain_local_prompts_preserved: usize,
    pub(crate) transcript_recovery_pending: usize,
    pub(crate) queued_local_prompts_started: usize,
    pub(crate) orphaned_workflow_prompts_finalized: usize,
    pub(crate) failed_reconciliations: usize,
}

enum UncertainLocalRecoveryOutcome {
    OriginalRedispatched,
    ContinuationDispatched,
    Preserved,
    TranscriptPending,
}

type DurableRestartRecoveryTarget = (String, String, String);

impl KernelRuntimeState {
    pub(crate) fn spawn_durable_restart_recovery(&self) {
        // Recovery belongs only to work that survived this kernel restart.
        // Keep that identity set fixed across the retry window so prompts
        // accepted after startup can never be mistaken for orphaned work.
        let recovery_targets = self.durable_restart_recovery_targets();
        let queued_recovery_targets = self.durable_restart_queued_recovery_targets();
        crate::logging::info_with_fields(
            "durable_state.recovery",
            "captured durable restart recovery targets",
            serde_json::json!({
                "active_prompt_targets": recovery_targets.len(),
                "queued_publication_targets": queued_recovery_targets.len(),
            }),
        );
        let state = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            let mut attempt = 0_u32;
            let summary = loop {
                let summary = state
                    .recover_durable_runtime_after_restart_targets(
                        &recovery_targets,
                        &queued_recovery_targets,
                    )
                    .await;
                if (summary.transcript_recovery_pending == 0 && summary.failed_reconciliations == 0)
                    || attempt >= 299
                {
                    break summary;
                }
                attempt = attempt.saturating_add(1);
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            };
            crate::logging::info_with_fields(
                "durable_state.recovery",
                "reconciled durable runtime work after kernel restart",
                serde_json::json!({
                    "cancelled_local_prompts_finalized": summary.cancelled_local_prompts_finalized,
                    "accepted_local_redispatched": summary.accepted_local_redispatched,
                    "uncertain_original_redispatched": summary.uncertain_original_redispatched,
                    "provider_continuations_dispatched": summary.provider_continuations_dispatched,
                    "remote_reconciliations_started": summary.remote_reconciliations_started,
                    "uncertain_local_prompts_preserved": summary.uncertain_local_prompts_preserved,
                    "transcript_recovery_pending": summary.transcript_recovery_pending,
                    "queued_local_prompts_started": summary.queued_local_prompts_started,
                    "orphaned_workflow_prompts_finalized": summary.orphaned_workflow_prompts_finalized,
                    "failed_reconciliations": summary.failed_reconciliations,
                }),
            );
        });
    }

    pub(crate) async fn recover_durable_runtime_after_restart(
        &self,
    ) -> DurableRestartRecoverySummary {
        let recovery_targets = self.durable_restart_recovery_targets();
        let queued_recovery_targets = self.durable_restart_queued_recovery_targets();
        self.recover_durable_runtime_after_restart_targets(
            &recovery_targets,
            &queued_recovery_targets,
        )
        .await
    }

    fn durable_restart_recovery_targets(&self) -> BTreeSet<DurableRestartRecoveryTarget> {
        let mut targets = BTreeSet::new();
        for session in self.owned.session_store.list_all_sessions() {
            for (agent_id, prompt_state) in session.prompt_states() {
                let Some(prompt) = prompt_state.active_prompt() else {
                    continue;
                };
                let Ok(agent) = self.owned.agent_store.get_agent(agent_id) else {
                    continue;
                };
                let local_workspace_available = agent.remote_execution().is_some()
                    || std::path::Path::new(
                        agent.worktree_id().unwrap_or_else(|| session.worktree_id()),
                    )
                    .exists();
                if !local_workspace_available {
                    continue;
                }
                targets.insert((
                    session.id().to_string(),
                    agent_id.to_string(),
                    prompt.id().to_string(),
                ));
            }
        }
        targets
    }

    fn durable_restart_queued_recovery_targets(&self) -> BTreeSet<DurableRestartRecoveryTarget> {
        self.owned
            .session_store
            .list_all_sessions()
            .into_iter()
            .flat_map(|session| {
                session
                    .prompt_states()
                    .iter()
                    .filter(|(_, prompt_state)| prompt_state.active_prompt().is_none())
                    .filter_map(|(agent_id, prompt_state)| {
                        let prompt = prompt_state.queued_prompts().front()?;
                        recoverable_queued_publication_prompt(&session, prompt).then(|| {
                            (
                                session.id().to_string(),
                                agent_id.to_string(),
                                prompt.id().to_string(),
                            )
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    async fn recover_durable_runtime_after_restart_targets(
        &self,
        recovery_targets: &BTreeSet<DurableRestartRecoveryTarget>,
        queued_recovery_targets: &BTreeSet<DurableRestartRecoveryTarget>,
    ) -> DurableRestartRecoverySummary {
        let mut summary = DurableRestartRecoverySummary::default();
        let sessions = self.owned.session_store.list_all_sessions();
        // Publication work is autonomous and already durably admitted. Resume it before
        // transcript reconciliation for unrelated interactive sessions, which may require slow
        // provider scans or retries.
        for (session_id, agent_id, prompt_id) in queued_recovery_targets {
            match self
                .recover_queued_local_prompt_after_restart(session_id, agent_id, prompt_id, true)
            {
                Ok(Some(dispatches)) => {
                    summary.queued_local_prompts_started += 1;
                    self.spawn_workflow_prompt_dispatches(dispatches);
                }
                Ok(None) => {}
                Err(error) => {
                    summary.failed_reconciliations += 1;
                    log_restart_recovery_failure(session_id, agent_id, prompt_id, &error);
                }
            }
        }
        for session in &sessions {
            for (agent_id, prompt_state) in session.prompt_states() {
                let Some(prompt) = prompt_state.active_prompt().cloned() else {
                    continue;
                };
                if !recovery_targets.contains(&(
                    session.id().to_string(),
                    agent_id.to_string(),
                    prompt.id().to_string(),
                )) {
                    continue;
                }
                let delivery_phase = prompt.durable_delivery_phase();
                let agent = match self.owned.agent_store.get_agent(agent_id) {
                    Ok(agent) => agent,
                    Err(error) => {
                        summary.failed_reconciliations += 1;
                        log_restart_recovery_failure(session.id(), agent_id, prompt.id(), &error);
                        continue;
                    }
                };
                if prompt.workflow_run_id().is_some()
                    && self
                        .owned
                        .session_store
                        .read()
                        .resolve_workflow_run_ref(
                            session.id(),
                            prompt.workflow_run_id().expect("checked above"),
                        )
                        .is_err()
                {
                    match self
                        .finalize_orphaned_workflow_prompt_after_restart(
                            session.id(),
                            agent_id,
                            &prompt,
                        )
                        .await
                    {
                        Ok(()) => summary.orphaned_workflow_prompts_finalized += 1,
                        Err(error) => {
                            summary.failed_reconciliations += 1;
                            log_restart_recovery_failure(
                                session.id(),
                                agent_id,
                                prompt.id(),
                                &error,
                            );
                        }
                    }
                    continue;
                }
                if agent.remote_execution().is_some() {
                    match self
                        .recover_remote_prompt_after_kernel_restart(
                            session.id(),
                            agent_id,
                            delivery_phase,
                            prompt.durable_delivery_provider_run_id(),
                        )
                        .await
                    {
                        Ok(true) => summary.remote_reconciliations_started += 1,
                        Ok(false) => summary.failed_reconciliations += 1,
                        Err(error) => {
                            summary.failed_reconciliations += 1;
                            log_restart_recovery_failure(
                                session.id(),
                                agent_id,
                                prompt.id(),
                                &error,
                            );
                        }
                    }
                    continue;
                }
                if prompt.status() == crate::session::PromptStatus::Cancelling {
                    match self
                        .finalize_cancelled_local_prompt_after_restart(session.id(), agent_id)
                        .await
                    {
                        Ok(()) => summary.cancelled_local_prompts_finalized += 1,
                        Err(error) => {
                            summary.failed_reconciliations += 1;
                            log_restart_recovery_failure(
                                session.id(),
                                agent_id,
                                prompt.id(),
                                &error,
                            );
                        }
                    }
                    continue;
                }
                match delivery_phase {
                    Some(crate::session::DurablePromptDeliveryPhase::Accepted) => match self
                        .redispatch_local_prompt(session.id(), agent_id, &prompt)
                        .await
                    {
                        Ok(()) => summary.accepted_local_redispatched += 1,
                        Err(error) => {
                            summary.failed_reconciliations += 1;
                            log_restart_recovery_failure(
                                session.id(),
                                agent_id,
                                prompt.id(),
                                &error,
                            );
                        }
                    },
                    Some(
                        crate::session::DurablePromptDeliveryPhase::Dispatching
                        | crate::session::DurablePromptDeliveryPhase::Delivered,
                    ) => match self
                        .reconcile_uncertain_local_prompt(
                            session.id(),
                            &agent,
                            &prompt,
                            delivery_phase.expect("matched delivery phase"),
                        )
                        .await
                    {
                        Ok(UncertainLocalRecoveryOutcome::OriginalRedispatched) => {
                            summary.uncertain_original_redispatched += 1;
                        }
                        Ok(UncertainLocalRecoveryOutcome::ContinuationDispatched) => {
                            summary.provider_continuations_dispatched += 1;
                        }
                        Ok(UncertainLocalRecoveryOutcome::Preserved) => {
                            summary.uncertain_local_prompts_preserved += 1;
                        }
                        Ok(UncertainLocalRecoveryOutcome::TranscriptPending) => {
                            summary.transcript_recovery_pending += 1;
                        }
                        Err(error) => {
                            summary.failed_reconciliations += 1;
                            log_restart_recovery_failure(
                                session.id(),
                                agent_id,
                                prompt.id(),
                                &error,
                            );
                        }
                    },
                    None => summary.uncertain_local_prompts_preserved += 1,
                }
            }
        }
        for session in sessions {
            self.spawn_workflow_prompt_dispatches(
                self.owned
                    .workflow_maybe_start_next_queued_prompt(session.id()),
            );
        }
        // Workspace claims are process-local, while blocked workflow nodes are durable.  After
        // a restart the old claim cannot still be held, so retry those nodes explicitly instead
        // of leaving event-delivery runs parked in `BlockedOnWorkspaceClaim` forever.
        self.spawn_workflow_prompt_dispatches(self.owned.workflow_retry_blocked_claims());
        summary
    }

    async fn finalize_orphaned_workflow_prompt_after_restart(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<(), DaemonError> {
        let session_id_owned = session_id.to_string();
        let agent_id_owned = agent_id.to_string();
        let prompt_id = prompt.id().to_string();
        self.with_app_side_effect(move |app| {
            let cancelled =
                app.prompt_owner_cancel_active_prompt_only(&session_id_owned, &agent_id_owned)?;
            crate::logging::warn_with_fields(
                "durable_state.recovery",
                "finalized workflow prompt whose run was not durable",
                serde_json::json!({
                    "session_id": session_id_owned,
                    "agent_id": agent_id_owned,
                    "prompt_id": prompt_id,
                    "cancelled_prompt_id": cancelled.id(),
                }),
            );
            Ok(())
        })
        .await?;

        // The workflow run already exists; only its provider prompt was orphaned.
        // Recover the next provider prompt directly instead of asking the workflow
        // queue to create another run.  The latter leaves a Ready node stranded
        // because the invocation was already claimed before the restart.
        let queued_prompt_id = self
            .owned
            .session_store
            .get_session(session_id)
            .ok()
            .and_then(|session| {
                self.owned
                    .prompt_state_owner
                    .state_parts(&session, agent_id)
                    .1
                    .front()
                    .map(|prompt| prompt.id().to_string())
            });
        if let Some(queued_prompt_id) = queued_prompt_id {
            if let Some(dispatches) = self.recover_queued_local_prompt_after_restart(
                session_id,
                agent_id,
                &queued_prompt_id,
                false,
            )? {
                self.spawn_workflow_prompt_dispatches(dispatches);
            }
        }
        Ok(())
    }

    #[allow(clippy::result_large_err)]
    fn recover_queued_local_prompt_after_restart(
        &self,
        session_id: &str,
        agent_id: &str,
        expected_prompt_id: &str,
        require_publication: bool,
    ) -> Result<Option<WorkflowPromptDispatches>, DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        if self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .is_some()
        {
            return Ok(None);
        }
        let Some(prompt) = self
            .owned
            .prompt_state_owner
            .state_parts(&session, agent_id)
            .1
            .front()
            .cloned()
        else {
            return Ok(None);
        };
        if prompt.id() != expected_prompt_id {
            return Ok(None);
        }
        if require_publication && !recoverable_queued_publication_prompt(&session, &prompt) {
            return Ok(None);
        }
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.remote_execution().is_some() {
            return Ok(None);
        }

        let (event_reply_enabled, event_context_enabled) = self
            .owned
            .workflow_event_capabilities_for_prompt(session_id, &prompt)?;
        let provider_run_id = self.owned.workflow_ensure_provider_run(
            session_id,
            agent_id,
            event_reply_enabled,
            event_context_enabled,
        )?;
        let provider_run = self
            .owned
            .ensure_provider_run_in_session(session_id, &provider_run_id)?;
        let mut dispatches = WorkflowPromptDispatches::default();
        match provider_run.state() {
            crate::provider::ProviderRunState::Starting => {
                dispatches.starting_provider_runs.push(provider_run_id);
            }
            crate::provider::ProviderRunState::Running => {
                if let Some(dispatch) = self.owned.advance_next_queued_prompt_dispatch(
                    session_id,
                    agent_id,
                    &provider_run_id,
                )? {
                    dispatches.local.push(dispatch);
                } else {
                    return Ok(None);
                }
            }
            crate::provider::ProviderRunState::Parked
            | crate::provider::ProviderRunState::Ended => {
                return Err(DaemonError::InvalidProviderRunState {
                    provider_run_id,
                    state: provider_run.state(),
                    operation: "recover queued prompt after restart",
                });
            }
        }
        Ok(Some(dispatches))
    }

    async fn finalize_cancelled_local_prompt_after_restart(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let provider_run_id = self
            .owned
            .provider_store
            .get_run_for_agent(session_id, agent_id)
            .map(|run| run.id().to_string());
        let cancellation = self
            .owned
            .finalize_local_prompt_cancellation_with_queued_advance(
                session_id,
                agent_id,
                provider_run_id.as_deref(),
            )?;
        self.owned
            .workflow_cancel_prompt(session_id, &cancellation.cancellation.prompt)?;
        if cancellation.released_claim {
            self.spawn_workflow_prompt_dispatches(self.owned.workflow_retry_blocked_claims());
        }
        if let Some(dispatch) = cancellation.dispatch {
            if let Err(error) = self
                .enqueue_prompt_dispatch_after_liveness(&dispatch, &self.owned)
                .await
            {
                let _ = self.fail_prompt_dispatch(dispatch, error).await;
            }
        }
        Ok(())
    }

    async fn redispatch_local_prompt(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<(), DaemonError> {
        let session_id_owned = session_id.to_string();
        let agent_id_owned = agent_id.to_string();
        let provider_run_id = self
            .with_app_side_effect(move |app| {
                app.ensure_prompt_provider_run_for_agent(&session_id_owned, &agent_id_owned)
            })
            .await?;
        let dispatch = crate::app::KernelPromptDispatch {
            session_id: session_id.to_string(),
            provider_run_id,
            agent_id: agent_id.to_string(),
            prompt_id: prompt.id().to_string(),
            target_active_prompt_id: None,
            source_attachment_id: prompt.source_attachment_id().to_string(),
            prompt: prompt.prompt().to_string(),
            hidden_system_context: prompt.hidden_system_context().to_string(),
            attachments: prompt.attachments().to_vec(),
            prompt_origin: prompt.prompt_origin(),
            external_provider: prompt.external_provider().map(str::to_string),
            external_provider_session_id: prompt.external_provider_session_id().map(str::to_string),
            external_provider_turn_id: prompt.external_provider_turn_id().map(str::to_string),
            steering: false,
        };
        self.enqueue_prompt_dispatch(&dispatch).await
    }

    async fn reconcile_uncertain_local_prompt(
        &self,
        session_id: &str,
        agent: &crate::agent::AgentInstance,
        prompt: &crate::session::PromptQueueItem,
        delivery_phase: crate::session::DurablePromptDeliveryPhase,
    ) -> Result<UncertainLocalRecoveryOutcome, DaemonError> {
        let adapter_key = crate::provider::adapter_key_for_provider(agent.provider());
        if adapter_key == "dev-stub" {
            self.redispatch_local_prompt(session_id, agent.id(), prompt)
                .await?;
            return Ok(UncertainLocalRecoveryOutcome::OriginalRedispatched);
        }
        if !crate::provider::ExternalProviderObservationPolicy::for_provider(adapter_key)
            .is_configured()
        {
            return Ok(UncertainLocalRecoveryOutcome::Preserved);
        }
        let existing_recovery_operation =
            prompt.durable_recovery_operation_id().map(str::to_string);
        let prompt_text = prompt.prompt().to_string();
        let worktree_path = agent.worktree_id().map(str::to_string).or_else(|| {
            self.owned
                .session_store
                .get_session(session_id)
                .ok()
                .map(|session| session.worktree_id().to_string())
        });
        let mut matched = None;
        for scan_attempt in 0..5 {
            let adapter_key_owned = adapter_key.to_string();
            let prompt_text = prompt_text.clone();
            let worktree_path = worktree_path.clone();
            let recovery_operation_for_scan = existing_recovery_operation.clone();
            matched = tokio::task::spawn_blocking(move || {
                crate::app::find_external_provider_prompt_recovery_match(
                    &adapter_key_owned,
                    &prompt_text,
                    worktree_path.as_deref(),
                    recovery_operation_for_scan.as_deref(),
                )
            })
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "scan provider transcript for restart recovery",
                message: error.to_string(),
            })?;
            if matched.is_some() || scan_attempt == 4 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        let provider_session_id = prompt
            .durable_delivery_provider_session_id()
            .map(str::to_string)
            .or_else(|| {
                agent
                    .provider_resume_state()
                    .provider_session_id(adapter_key)
                    .map(str::to_string)
            })
            .or_else(|| {
                matched
                    .as_ref()
                    .map(|matched| matched.provider_session_id.clone())
            });
        let Some(provider_session_id) = provider_session_id else {
            if delivery_phase == crate::session::DurablePromptDeliveryPhase::Dispatching
                && existing_recovery_operation.is_none()
            {
                self.redispatch_local_prompt(session_id, agent.id(), prompt)
                    .await?;
                return Ok(UncertainLocalRecoveryOutcome::OriginalRedispatched);
            }
            return Ok(UncertainLocalRecoveryOutcome::TranscriptPending);
        };
        if let Some(operation_id) = existing_recovery_operation.as_deref() {
            let operation_observed = matched
                .as_ref()
                .is_some_and(|matched| matched.recovery_operation_observed);
            if operation_observed {
                self.owned.mark_active_prompt_recovery_phase(
                    session_id,
                    agent.id(),
                    prompt.id(),
                    operation_id,
                    crate::session::DurablePromptDeliveryPhase::Delivered,
                )?;
            } else if prompt.durable_recovery_phase()
                != Some(crate::session::DurablePromptDeliveryPhase::Accepted)
            {
                return Ok(UncertainLocalRecoveryOutcome::TranscriptPending);
            }
        }
        self.persist_recovered_provider_session(agent, adapter_key, &provider_session_id)?;
        let recovery_prompt =
            self.owned
                .begin_active_prompt_recovery(session_id, agent.id(), prompt.id())?;
        let operation_id = recovery_prompt
            .durable_recovery_operation_id()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "begin provider restart continuation",
                message: "recovery operation did not receive an id".to_string(),
            })?
            .to_string();
        self.owned.mark_active_prompt_recovery_phase(
            session_id,
            agent.id(),
            prompt.id(),
            &operation_id,
            crate::session::DurablePromptDeliveryPhase::Dispatching,
        )?;
        let session_id_owned = session_id.to_string();
        let agent_id_owned = agent.id().to_string();
        let provider_run_id = match self
            .with_app_side_effect(move |app| {
                app.ensure_prompt_provider_run_for_agent(&session_id_owned, &agent_id_owned)
            })
            .await
        {
            Ok(provider_run_id) => provider_run_id,
            Err(error) => {
                let _ = self.owned.mark_active_prompt_recovery_phase(
                    session_id,
                    agent.id(),
                    prompt.id(),
                    &operation_id,
                    crate::session::DurablePromptDeliveryPhase::Accepted,
                );
                return Err(error);
            }
        };
        let structured = self
            .owned
            .provider_store
            .get_run(&provider_run_id)
            .is_ok_and(|run| {
                self.owned
                    .provider_store
                    .run_uses_structured_prompt_io(&run)
            });
        let continuation = provider_restart_continuation_prompt(&operation_id);
        let dispatch = crate::app::KernelPromptDispatch {
            session_id: session_id.to_string(),
            provider_run_id,
            agent_id: agent.id().to_string(),
            prompt_id: prompt.id().to_string(),
            target_active_prompt_id: None,
            source_attachment_id: format!("{KERNEL_RECOVERY_ATTACHMENT_PREFIX}{operation_id}"),
            prompt: continuation,
            hidden_system_context: String::new(),
            attachments: Vec::new(),
            prompt_origin: crate::session::PromptOrigin::Chariox,
            external_provider: None,
            external_provider_session_id: None,
            external_provider_turn_id: None,
            steering: false,
        };
        if let Err(error) = self.enqueue_prompt_dispatch(&dispatch).await {
            let _ = self.owned.mark_active_prompt_recovery_phase(
                session_id,
                agent.id(),
                prompt.id(),
                &operation_id,
                crate::session::DurablePromptDeliveryPhase::Accepted,
            );
            return Err(error);
        }
        if !structured {
            self.owned.mark_active_prompt_recovery_phase(
                session_id,
                agent.id(),
                prompt.id(),
                &operation_id,
                crate::session::DurablePromptDeliveryPhase::Delivered,
            )?;
        }
        Ok(UncertainLocalRecoveryOutcome::ContinuationDispatched)
    }

    fn persist_recovered_provider_session(
        &self,
        agent: &crate::agent::AgentInstance,
        adapter_key: &str,
        provider_session_id: &str,
    ) -> Result<(), DaemonError> {
        if agent
            .provider_resume_state()
            .provider_session_id(adapter_key)
            == Some(provider_session_id)
        {
            return Ok(());
        }
        let mut resume_state = agent.provider_resume_state().clone();
        if !resume_state.set_provider_session_id(adapter_key, provider_session_id.to_string()) {
            return Err(DaemonError::LocalTransport {
                operation: "persist provider restart session",
                message: format!("provider `{adapter_key}` has no resumable session identity"),
            });
        }
        let updated = self
            .owned
            .agent_store
            .set_agent_runtime_profile_with_account_profile(
                agent.id(),
                agent.provider(),
                agent.model().map(str::to_string),
                agent.effort().map(str::to_string),
                Some(agent.provider_account_profile().to_string()),
                resume_state,
            )?;
        self.owned.durable_state_store.append_event(
            "agent.runtime_profile_updated",
            Some(updated.id().to_string()),
            serde_json::json!({
                "agent": &updated,
                "reason": "provider_restart_transcript_reconciled",
            }),
        )?;
        Ok(())
    }
}

fn recoverable_queued_publication_prompt(
    session: &crate::session::RuntimeSession,
    prompt: &crate::session::PromptQueueItem,
) -> bool {
    if !std::path::Path::new(session.worktree_id()).exists() {
        return false;
    }
    let (Some(workflow_run_id), Some(workflow_node_run_id)) =
        (prompt.workflow_run_id(), prompt.workflow_node_run_id())
    else {
        return false;
    };
    session
        .workflow_runs()
        .iter()
        .find(|run| run.id() == workflow_run_id)
        .is_some_and(|run| {
            run.publication_invocation().is_some_and(|invocation| {
                session.workflow_publications().iter().any(|publication| {
                    publication.id() == invocation.publication_id && publication.enabled()
                })
            }) && matches!(
                run.status(),
                crate::session::WorkflowRunStatus::Created
                    | crate::session::WorkflowRunStatus::Running
                    | crate::session::WorkflowRunStatus::Waiting
            ) && run
                .node_runs()
                .iter()
                .find(|node_run| node_run.id() == workflow_node_run_id)
                .is_some_and(|node_run| {
                    !matches!(
                        node_run.status(),
                        crate::session::WorkflowNodeRunStatus::Completed
                            | crate::session::WorkflowNodeRunStatus::Failed
                            | crate::session::WorkflowNodeRunStatus::Stopped
                    )
                })
        })
}

fn provider_restart_continuation_prompt(operation_id: &str) -> String {
    format!(
        "[Chariox recovery operation {operation_id}] Continue the active task from the current provider session state. Do not repeat completed tool calls or external side effects. If the task already completed, return its final response from the existing results."
    )
}

fn log_restart_recovery_failure(
    session_id: &str,
    agent_id: &str,
    prompt_id: &str,
    error: &DaemonError,
) {
    crate::logging::warn_with_fields(
        "durable_state.recovery",
        "durable prompt restart reconciliation failed",
        serde_json::json!({
            "session_id": session_id,
            "agent_id": agent_id,
            "prompt_id": prompt_id,
            "error": error.to_string(),
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::CreateAgentRequest;
    use crate::app::KernelSessionService;
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::config::DaemonConfig;
    use crate::provider::LaunchProviderRequest;
    use crate::session::{CreateSessionRequest, PromptQueueItem, PromptStatus};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn runtime_with_active_prompt(
        delivery_phase: crate::session::DurablePromptDeliveryPhase,
    ) -> (KernelRuntimeState, String, String, String) {
        runtime_with_active_prompt_in_worktree(
            delivery_phase,
            std::env::current_dir()
                .expect("test workspace should resolve")
                .to_string_lossy()
                .as_ref(),
        )
    }

    fn runtime_with_active_prompt_in_worktree(
        delivery_phase: crate::session::DurablePromptDeliveryPhase,
        worktree: &str,
    ) -> (KernelRuntimeState, String, String, String) {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-restart-recovery",
                worktree,
            ))
            .expect("session should create");
        let agent = KernelSessionService::new(&mut app)
            .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_worktree(worktree))
            .expect("agent should create");
        let attachment = KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "attachment-restart-recovery",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let provider_run = app
            .launch_provider(
                LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "dev-stub",
                    "default",
                    "sonnet",
                )
                .with_agent_id(agent.id()),
            )
            .expect("provider should launch");
        let prompt = PromptQueueItem::new(
            "pending-restart-recovery",
            attachment.id(),
            agent.id(),
            "continue after restart",
            PromptStatus::Queued,
        )
        .with_durable_operation("command-restart-recovery", "fingerprint-restart-recovery");
        let outcome = app
            .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
            .expect("prompt should be accepted");
        let prompt = match outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt } => prompt,
            crate::session::PromptSubmissionOutcome::Queued { .. } => {
                panic!("prompt should start")
            }
        };
        app.mark_active_prompt_delivery(
            session.id(),
            agent.id(),
            prompt.id(),
            delivery_phase,
            (delivery_phase != crate::session::DurablePromptDeliveryPhase::Accepted)
                .then(|| provider_run.id().to_string()),
            None,
        )
        .expect("delivery phase should persist");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let prompt_id = prompt.id().to_string();
        app.attachments().remove_session_attachments(&session_id);
        let mut restored = app
            .sessions()
            .get_session(&session_id)
            .expect("session should load before simulated restart");
        restored.reconcile_after_kernel_restart();
        app.sessions_mut().restore_session(restored);
        let app = Arc::new(Mutex::new(app));
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            app,
            crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
        );
        (router.runtime_state(), session_id, agent_id, prompt_id)
    }

    fn runtime_with_queued_metaagent_task() -> (KernelRuntimeState, String, String) {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-restart-meta-queue",
                "worktree-restart-meta-queue",
            ))
            .expect("session should create");
        let attachment = KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "attachment-restart-meta-queue",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.launch_provider(
            LaunchProviderRequest::new(session.id(), "dev-stub", "dev-stub", "default", "sonnet")
                .with_agent_id(agent.id()),
        )
        .expect("provider should launch");
        app.sessions_mut()
            .enqueue_metaagent_task(
                session.id(),
                agent.id(),
                attachment.id(),
                "resume queued Meta work",
                Vec::new(),
            )
            .expect("Meta task should queue");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            app,
            crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
        );
        (router.runtime_state(), session_id, agent_id)
    }

    fn runtime_with_queued_prompt() -> (KernelRuntimeState, String, String) {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let workspace = std::env::current_dir().expect("test workspace should resolve");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                workspace.to_string_lossy(),
                workspace.to_string_lossy(),
            ))
            .expect("session should create");
        let mut session_with_agents = session.clone();
        session_with_agents.set_agents(vec![agent.clone()]);
        app.sessions_mut().restore_session(session_with_agents);
        let _attachment = KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "attachment-restart-prompt-queue",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let workflow = app
            .sessions_mut()
            .create_workflow(session.id(), Some("restart-publication".to_string()))
            .expect("workflow should create");
        let node = app
            .sessions_mut()
            .add_workflow_node(session.id(), workflow.id(), agent.id())
            .expect("workflow node should create");
        let endpoint = app
            .sessions_mut()
            .create_workflow_endpoint(
                session.id(),
                workflow.id(),
                node.id(),
                Some("entry".to_string()),
            )
            .expect("workflow endpoint should create");
        let publication = app
            .sessions_mut()
            .create_workflow_publication(
                session.id(),
                workflow.id(),
                endpoint.id(),
                Some("default".to_string()),
                Some("restart-publication".to_string()),
                Some(crate::session::WORKFLOW_PUBLICATION_KIND_INGRESS.to_string()),
                Some("/restart".to_string()),
                vec!["POST".to_string()],
                None,
                None,
                None,
                None,
                Some("async".to_string()),
                None,
                None,
                "local".to_string(),
            )
            .expect("workflow publication should create");
        let publication_invocation = crate::session::WorkflowPublicationInvocationEnvelope {
            publication_id: publication.id().to_string(),
            hook_id: None,
            invocation_id: "invocation-restart".to_string(),
            transport: "event".to_string(),
            endpoint_id: endpoint.id().to_string(),
            queue_ref: Some("default".to_string()),
            input: serde_json::json!({"prompt": "resume queued work"}),
            artifacts: Vec::new(),
            mode: None,
            caller: serde_json::json!({"type": "event"}),
        };
        let workflow_run = app
            .sessions_mut()
            .invoke_workflow_endpoint_with_publication_invocation(
                session.id(),
                workflow.id(),
                endpoint.id(),
                Some("resume queued work".to_string()),
                Some(publication_invocation),
            )
            .expect("published workflow run should create");
        let node_run_id = workflow_run.node_runs()[0].id().to_string();
        app.sessions_mut()
            .prepare_workflow_turn(
                session.id(),
                workflow_run.id(),
                &node_run_id,
                format!("workflow-ack:{node_run_id}"),
                "resume queued work".to_string(),
                None,
                None,
            )
            .expect("workflow turn should prepare");
        let prompt = PromptQueueItem::new(
            "pending-restart-prompt-queue",
            crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
            agent.id(),
            "resume queued work",
            PromptStatus::Queued,
        )
        .with_workflow_context(workflow_run.id(), &node_run_id);
        let outcome = app
            .prompt_owner_submit_prepared_prompt(session.id(), prompt, true)
            .expect("prompt should queue");
        assert!(matches!(
            outcome,
            crate::session::PromptSubmissionOutcome::Queued { .. }
        ));
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            app,
            crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
        );
        (router.runtime_state(), session_id, agent_id)
    }

    #[tokio::test]
    async fn queued_metaagent_task_starts_after_restart_without_an_active_prompt() {
        let (runtime, session_id, agent_id) = runtime_with_queued_metaagent_task();

        let summary = runtime.recover_durable_runtime_after_restart().await;

        assert_eq!(summary, DurableRestartRecoverySummary::default());
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let session = runtime
                    .owned
                    .session_store
                    .get_session(&session_id)
                    .expect("session should remain available");
                if session.queued_metaagent_tasks().is_empty()
                    && session.metaagent_task(&agent_id).is_some_and(|task| {
                        task.status() == crate::session::MetaagentTaskStatus::Active
                    })
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("queued Meta task should restart");
    }

    #[tokio::test]
    async fn queued_local_prompt_starts_provider_after_restart() {
        let (runtime, session_id, agent_id) = runtime_with_queued_prompt();

        let summary = runtime.recover_durable_runtime_after_restart().await;

        assert_eq!(summary.queued_local_prompts_started, 1);
        assert_eq!(summary.failed_reconciliations, 0);
        let run = runtime
            .owned
            .provider_store
            .get_run_for_agent(&session_id, &agent_id)
            .expect("queued prompt recovery should launch its provider");
        assert!(matches!(
            run.state(),
            crate::provider::ProviderRunState::Starting
                | crate::provider::ProviderRunState::Running
        ));
    }

    #[tokio::test]
    async fn accepted_prompt_is_redispatched_after_restart() {
        let (runtime, session_id, agent_id, prompt_id) =
            runtime_with_active_prompt(crate::session::DurablePromptDeliveryPhase::Accepted);

        let summary = runtime.recover_durable_runtime_after_restart().await;

        assert_eq!(summary.accepted_local_redispatched, 1);
        assert_eq!(summary.failed_reconciliations, 0);
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should load");
        let prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .expect("prompt should remain active");
        assert_eq!(prompt.id(), prompt_id);
        assert_eq!(
            prompt.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Delivered)
        );
    }

    #[tokio::test]
    async fn restart_recovery_retry_ignores_prompt_outside_startup_snapshot() {
        let (runtime, session_id, agent_id, prompt_id) =
            runtime_with_active_prompt(crate::session::DurablePromptDeliveryPhase::Accepted);

        let summary = runtime
            .recover_durable_runtime_after_restart_targets(&BTreeSet::new(), &BTreeSet::new())
            .await;

        assert_eq!(summary, DurableRestartRecoverySummary::default());
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should load");
        let prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .expect("post-startup prompt should remain active");
        assert_eq!(prompt.id(), prompt_id);
        assert_eq!(
            prompt.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Accepted)
        );
    }

    #[tokio::test]
    async fn restart_recovery_skips_local_prompt_when_its_workspace_is_gone() {
        let missing_worktree = std::env::temp_dir().join(format!(
            "chariox-missing-restart-recovery-{}",
            std::process::id()
        ));
        let (runtime, session_id, agent_id, prompt_id) = runtime_with_active_prompt_in_worktree(
            crate::session::DurablePromptDeliveryPhase::Delivered,
            missing_worktree.to_string_lossy().as_ref(),
        );

        let summary = runtime.recover_durable_runtime_after_restart().await;

        assert_eq!(summary, DurableRestartRecoverySummary::default());
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should remain available");
        let prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .expect("unrecoverable prompt should remain preserved");
        assert_eq!(prompt.id(), prompt_id);
    }

    #[tokio::test]
    async fn uncertain_dev_stub_prompt_is_redispatched_after_restart() {
        let (runtime, session_id, agent_id, prompt_id) =
            runtime_with_active_prompt(crate::session::DurablePromptDeliveryPhase::Dispatching);

        let summary = runtime.recover_durable_runtime_after_restart().await;

        assert_eq!(summary.accepted_local_redispatched, 0);
        assert_eq!(summary.uncertain_original_redispatched, 1);
        assert_eq!(summary.uncertain_local_prompts_preserved, 0);
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should load");
        let prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .expect("redispatched prompt should remain active");
        assert_eq!(prompt.id(), prompt_id);
        assert_eq!(
            prompt.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Delivered)
        );
    }

    #[tokio::test]
    async fn cancelling_local_prompt_is_finalized_instead_of_resumed_after_restart() {
        let (runtime, session_id, agent_id, _prompt_id) =
            runtime_with_active_prompt(crate::session::DurablePromptDeliveryPhase::Delivered);
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should load");
        runtime
            .owned
            .prompt_state_owner
            .begin_cancelling_active_prompt(&session, &agent_id)
            .expect("prompt should begin cancelling");
        let (active_prompt, queued_prompts) = runtime
            .owned
            .prompt_state_owner
            .state_parts(&session, &agent_id);
        runtime
            .owned
            .mirror_prompt_owner_agent_state(&session_id, &agent_id, active_prompt, queued_prompts)
            .expect("cancelling state should persist");

        let summary = runtime.recover_durable_runtime_after_restart().await;

        assert_eq!(summary.cancelled_local_prompts_finalized, 1);
        assert_eq!(summary.provider_continuations_dispatched, 0);
        assert_eq!(summary.failed_reconciliations, 0);
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should remain available");
        assert!(
            runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &agent_id)
                .is_none(),
            "cancelled prompt must not be resumed after restart"
        );
    }

    #[test]
    fn recovery_operation_reuses_accepted_generation_and_advances_after_delivery() {
        let mut prompt = PromptQueueItem::new(
            "prompt-1",
            "attachment-1",
            "agent-1",
            "recover",
            PromptStatus::Running,
        );

        let first = prompt.begin_durable_recovery_operation();
        assert_eq!(prompt.begin_durable_recovery_operation(), first);
        assert!(prompt.mark_durable_recovery_phase(
            &first,
            crate::session::DurablePromptDeliveryPhase::Delivered,
        ));
        let second = prompt.begin_durable_recovery_operation();

        assert_eq!(first, "chariox-recovery:prompt-1:1");
        assert_eq!(second, "chariox-recovery:prompt-1:2");
        assert_eq!(
            prompt.durable_recovery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Accepted)
        );
    }

    #[tokio::test]
    async fn internal_recovery_prompt_is_not_recorded_as_user_terminal_input() {
        let (runtime, session_id, agent_id, prompt_id) =
            runtime_with_active_prompt(crate::session::DurablePromptDeliveryPhase::Dispatching);
        let provider_run_id = runtime
            .owned
            .provider_store
            .get_run_for_agent(&session_id, &agent_id)
            .expect("provider run should exist")
            .id()
            .to_string();
        let operation_id = "chariox-recovery:prompt-hidden:1";
        let dispatch = crate::app::KernelPromptDispatch {
            session_id: session_id.clone(),
            provider_run_id: provider_run_id.clone(),
            agent_id: agent_id.clone(),
            prompt_id,
            target_active_prompt_id: None,
            source_attachment_id: format!("{KERNEL_RECOVERY_ATTACHMENT_PREFIX}{operation_id}"),
            prompt: provider_restart_continuation_prompt(operation_id),
            hidden_system_context: String::new(),
            attachments: Vec::new(),
            prompt_origin: crate::session::PromptOrigin::Chariox,
            external_provider: None,
            external_provider_session_id: None,
            external_provider_turn_id: None,
            steering: false,
        };

        runtime
            .enqueue_prompt_dispatch(&dispatch)
            .await
            .expect("internal continuation should dispatch");

        assert!(runtime
            .owned
            .terminal_stream
            .input_records()
            .iter()
            .all(|record| !String::from_utf8_lossy(&record.bytes).contains(operation_id)));
    }

    #[tokio::test]
    async fn internal_recovery_prompt_is_not_echoed_to_other_attachments() {
        // The dispatch fanout is the boundary where a recovery envelope would
        // become user-visible terminal output on subscribed attachments. The
        // local dispatch runtime guards its call site, but remote-lease
        // dispatchers also invoke this helper and any future caller could
        // regress the invariant. Assert the fanout helper itself refuses to
        // surface a `kernel-recovery:*` envelope regardless of caller.
        let (runtime, session_id, agent_id, _prompt_id) =
            runtime_with_active_prompt(crate::session::DurablePromptDeliveryPhase::Dispatching);
        let provider_run_id = runtime
            .owned
            .provider_store
            .get_run_for_agent(&session_id, &agent_id)
            .expect("provider run should exist")
            .id()
            .to_string();
        let observer_attachment_id = runtime
            .with_app_side_effect(|app| {
                crate::app::KernelSessionService::new(app)
                    .attach(AttachRequest::new(
                        &session_id,
                        "attachment-restart-recovery-observer",
                        ClientCapabilityLevel::FullTerminal,
                    ))
                    .expect("observer attachment should attach")
                    .id()
                    .to_string()
            })
            .await;
        let operation_id = "chariox-recovery:prompt-hidden:1";
        let recovery_source_attachment =
            format!("{KERNEL_RECOVERY_ATTACHMENT_PREFIX}{operation_id}");
        let recovery_prompt = provider_restart_continuation_prompt(operation_id);

        runtime.owned.echo_prompt_to_other_attachments(
            &session_id,
            &provider_run_id,
            "prompt-hidden",
            &recovery_source_attachment,
            &recovery_prompt,
            &[],
        );

        let leaked_records: Vec<_> = runtime
            .owned
            .terminal_stream
            .output_records()
            .into_iter()
            .filter(|record| {
                record
                    .recipient_attachment_ids
                    .iter()
                    .any(|id| id == &observer_attachment_id)
                    && String::from_utf8_lossy(&record.bytes).contains(operation_id)
            })
            .collect();
        assert!(
            leaked_records.is_empty(),
            "kernel-recovery envelope must never be echoed to other attachments; leaked records = {leaked_records:#?}",
        );
    }
}
