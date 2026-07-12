//! Startup reconciliation for durable prompt work left active by a previous kernel process.

use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DurableRestartRecoverySummary {
    pub(crate) accepted_local_redispatched: usize,
    pub(crate) remote_reconciliations_started: usize,
    pub(crate) uncertain_local_prompts_preserved: usize,
    pub(crate) failed_reconciliations: usize,
}

impl KernelRuntimeState {
    pub(crate) fn spawn_durable_restart_recovery(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            let summary = state.recover_durable_runtime_after_restart().await;
            crate::logging::info_with_fields(
                "durable_state.recovery",
                "reconciled durable runtime work after kernel restart",
                serde_json::json!({
                    "accepted_local_redispatched": summary.accepted_local_redispatched,
                    "remote_reconciliations_started": summary.remote_reconciliations_started,
                    "uncertain_local_prompts_preserved": summary.uncertain_local_prompts_preserved,
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
                if delivery_phase != Some(crate::session::DurablePromptDeliveryPhase::Accepted) {
                    summary.uncertain_local_prompts_preserved += 1;
                    continue;
                }
                match self
                    .redispatch_accepted_local_prompt(session.id(), agent_id, &prompt)
                    .await
                {
                    Ok(()) => summary.accepted_local_redispatched += 1,
                    Err(error) => {
                        summary.failed_reconciliations += 1;
                        log_restart_recovery_failure(session.id(), agent_id, prompt.id(), &error);
                    }
                }
            }
        }
        summary
    }

    async fn redispatch_accepted_local_prompt(
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
                    "claude-code",
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
    async fn uncertain_local_prompt_is_preserved_without_redispatch() {
        let (runtime, session_id, agent_id, prompt_id) =
            runtime_with_active_prompt(crate::session::DurablePromptDeliveryPhase::Dispatching);

        let summary = runtime.recover_durable_runtime_after_restart().await;

        assert_eq!(summary.accepted_local_redispatched, 0);
        assert_eq!(summary.uncertain_local_prompts_preserved, 1);
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should load");
        let prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .expect("uncertain prompt should remain active");
        assert_eq!(prompt.id(), prompt_id);
        assert_eq!(
            prompt.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
        );
    }
}
