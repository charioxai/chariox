//! Startup reconciliation for durable prompt work left active by a previous kernel process.

use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DurableRestartRecoverySummary {
    pub(crate) accepted_local_redispatched: usize,
    pub(crate) uncertain_original_redispatched: usize,
    pub(crate) provider_continuations_dispatched: usize,
    pub(crate) remote_reconciliations_started: usize,
    pub(crate) uncertain_local_prompts_preserved: usize,
    pub(crate) transcript_recovery_pending: usize,
    pub(crate) failed_reconciliations: usize,
}

enum UncertainLocalRecoveryOutcome {
    OriginalRedispatched,
    ContinuationDispatched,
    Preserved,
    TranscriptPending,
}

impl KernelRuntimeState {
    pub(crate) fn spawn_durable_restart_recovery(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            let mut attempt = 0_u32;
            let summary = loop {
                let summary = state.recover_durable_runtime_after_restart().await;
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
                    "accepted_local_redispatched": summary.accepted_local_redispatched,
                    "uncertain_original_redispatched": summary.uncertain_original_redispatched,
                    "provider_continuations_dispatched": summary.provider_continuations_dispatched,
                    "remote_reconciliations_started": summary.remote_reconciliations_started,
                    "uncertain_local_prompts_preserved": summary.uncertain_local_prompts_preserved,
                    "transcript_recovery_pending": summary.transcript_recovery_pending,
                    "failed_reconciliations": summary.failed_reconciliations,
                }),
            );
        });
    }

    pub(crate) async fn recover_durable_runtime_after_restart(
        &self,
    ) -> DurableRestartRecoverySummary {
        let mut summary = DurableRestartRecoverySummary::default();
        for session in self.owned.session_store.list_all_sessions() {
            for (agent_id, prompt_state) in session.prompt_states() {
                let Some(prompt) = prompt_state.active_prompt().cloned() else {
                    continue;
                };
                let delivery_phase = prompt.durable_delivery_phase();
                let agent = match self.owned.agent_store.get_agent(agent_id) {
                    Ok(agent) => agent,
                    Err(error) => {
                        summary.failed_reconciliations += 1;
                        log_restart_recovery_failure(session.id(), agent_id, prompt.id(), &error);
                        continue;
                    }
                };
                if agent.remote_execution().is_some() {
                    match self
                        .recover_remote_prompt_after_kernel_restart(
                            session.id(),
                            agent_id,
                            delivery_phase,
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
        summary
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
            source_attachment_id: format!("kernel-recovery:{operation_id}"),
            prompt: continuation,
            hidden_system_context: String::new(),
            attachments: Vec::new(),
            prompt_origin: crate::session::PromptOrigin::Arroba,
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

fn provider_restart_continuation_prompt(operation_id: &str) -> String {
    format!(
        "[Arroba recovery operation {operation_id}] Continue the active task from the current provider session state. Do not repeat completed tool calls or external side effects. If the task already completed, return its final response from the existing results."
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
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-restart-recovery",
                "worktree-restart-recovery",
            ))
            .expect("session should create");
        let agent = KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_worktree("worktree-restart-recovery"),
            )
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

        assert_eq!(first, "arroba-recovery:prompt-1:1");
        assert_eq!(second, "arroba-recovery:prompt-1:2");
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
        let operation_id = "arroba-recovery:prompt-hidden:1";
        let dispatch = crate::app::KernelPromptDispatch {
            session_id: session_id.clone(),
            provider_run_id: provider_run_id.clone(),
            agent_id: agent_id.clone(),
            prompt_id,
            target_active_prompt_id: None,
            source_attachment_id: format!("kernel-recovery:{operation_id}"),
            prompt: provider_restart_continuation_prompt(operation_id),
            hidden_system_context: String::new(),
            attachments: Vec::new(),
            prompt_origin: crate::session::PromptOrigin::Arroba,
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
}
