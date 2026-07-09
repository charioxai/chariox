//! Local provider prompt dispatch and abort execution.
//!
//! This module owns writing admitted prompts or cancellation signals to local provider runtimes,
//! including structured prompt I/O and provider-runtime lane spawning.

use super::*;

impl KernelRuntimeOwnedState {
    fn prompt_dispatch_matches_active_prompt(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
    ) -> Result<bool, DaemonError> {
        let session = self.session_store.get_session(&dispatch.session_id)?;
        let prompt_is_dispatch_prompt = |prompt: &crate::session::PromptQueueItem| {
            if !prompt.is_arroba_owned() {
                return false;
            }
            if dispatch.steering {
                return dispatch
                    .target_active_prompt_id
                    .as_deref()
                    .is_some_and(|target_prompt_id| target_prompt_id == prompt.id());
            }
            prompt.id() == dispatch.prompt_id
        };
        Ok(self
            .prompt_state_owner
            .active_prompt_for_agent(&session, &dispatch.agent_id)
            .is_some_and(|prompt| prompt_is_dispatch_prompt(&prompt)))
    }

    fn ensure_prompt_dispatch_matches_active_prompt(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
    ) -> Result<bool, DaemonError> {
        let matches = self.prompt_dispatch_matches_active_prompt(dispatch)?;
        if matches || !dispatch.steering {
            return Ok(matches);
        }
        Err(DaemonError::LocalTransport {
            operation: "steer queued prompt",
            message: "queued prompt steer dispatch no longer matches the active prompt".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::KernelSessionService;
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::config::DaemonConfig;
    use crate::provider::LaunchProviderRequest;
    use crate::session::{
        CreateSessionRequest, PromptQueueItem, PromptStatus, PromptSubmissionOutcome,
    };
    use std::sync::Arc;
    use tokio::sync::Mutex;

    async fn owned_runtime_state(app: &Arc<Mutex<DaemonApp>>) -> KernelRuntimeState {
        let (
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            history_store,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        ) = {
            let app_locked = app.lock().await;
            (
                app_locked.config_projection_store(),
                app_locked.session_state_store(),
                app_locked.agents().clone(),
                app_locked.attachments().clone(),
                app_locked.providers().clone(),
                app_locked.provider_process_tracking_store(),
                app_locked.slices(),
                app_locked.session_state_projection_store(),
                app_locked.provider_run_projection_store(),
                app_locked.history_store(),
                app_locked.operational_history_store(),
                app_locked.durable_state_store(),
                app_locked.prompt_state_owner(),
                app_locked.active_turn_store(),
                app_locked.prompt_activity_store(),
                app_locked.prompt_workspace_claim_store(),
                app_locked.structured_output_record_store(),
                app_locked.terminal_stream_store(),
                app_locked.workflow_design_event_store(),
                app_locked.metaagent_event_store(),
                app_locked.workspace_coordinator(),
            )
        };
        KernelRuntimeState::new_with_owned_state(
            Arc::clone(app),
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            history_store,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        )
    }

    async fn runtime_with_active_prompt(
    ) -> (KernelRuntimeState, String, String, String, String, String) {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-steering-dispatch",
                "worktree-steering-dispatch",
            ))
            .expect("session should create");
        let attachment = KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "attachment-steering-dispatch",
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
            .expect("provider launch should succeed");
        app.update_provider_run_projection(provider_run.clone());
        let prompt = PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            attachment.id(),
            agent.id(),
            "active prompt",
            PromptStatus::Queued,
        );
        let PromptSubmissionOutcome::Started { prompt } = app
            .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
            .expect("active prompt should submit")
        else {
            panic!("prompt should start");
        };
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment_id = attachment.id().to_string();
        let active_prompt_id = prompt.id().to_string();
        let provider_run_id = provider_run.id().to_string();
        let app = Arc::new(Mutex::new(app));
        (
            owned_runtime_state(&app).await,
            session_id,
            agent_id,
            attachment_id,
            active_prompt_id,
            provider_run_id,
        )
    }

    fn dispatch(
        session_id: &str,
        agent_id: &str,
        attachment_id: &str,
        provider_run_id: &str,
        prompt_id: &str,
        prompt: &str,
        target_active_prompt_id: Option<String>,
        steering: bool,
    ) -> crate::app::KernelPromptDispatch {
        crate::app::KernelPromptDispatch {
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            agent_id: agent_id.to_string(),
            prompt_id: prompt_id.to_string(),
            target_active_prompt_id,
            source_attachment_id: attachment_id.to_string(),
            prompt: prompt.to_string(),
            hidden_system_context: String::new(),
            attachments: Vec::new(),
            prompt_origin: crate::session::PromptOrigin::Arroba,
            external_provider: None,
            external_provider_session_id: None,
            external_provider_turn_id: None,
            steering,
        }
    }

    #[tokio::test]
    async fn steering_dispatch_matches_target_active_prompt() {
        let (runtime, session_id, agent_id, attachment_id, active_prompt_id, provider_run_id) =
            runtime_with_active_prompt().await;
        let steering_dispatch = dispatch(
            &session_id,
            &agent_id,
            &attachment_id,
            &provider_run_id,
            "queued-steering-prompt",
            "steer now",
            Some(active_prompt_id),
            true,
        );

        assert!(runtime
            .owned
            .prompt_dispatch_matches_active_prompt(&steering_dispatch)
            .expect("dispatch match should evaluate"));
    }

    #[tokio::test]
    async fn dispatch_match_uses_prompt_owner_when_session_mirror_is_stale() {
        let (runtime, session_id, agent_id, attachment_id, active_prompt_id, provider_run_id) =
            runtime_with_active_prompt().await;
        runtime
            .owned
            .session_store
            .mirror_agent_prompt_state(
                &session_id,
                &agent_id,
                None,
                std::collections::VecDeque::new(),
            )
            .expect("test drift should clear stale session prompt mirror");
        assert!(
            runtime
                .owned
                .session_store
                .get_session(&session_id)
                .expect("session should load")
                .active_prompt_for_agent(&agent_id)
                .is_none(),
            "session mirror should not expose the active prompt"
        );
        let steering_dispatch = dispatch(
            &session_id,
            &agent_id,
            &attachment_id,
            &provider_run_id,
            "queued-steering-prompt",
            "steer now",
            Some(active_prompt_id),
            true,
        );

        assert!(runtime
            .owned
            .prompt_dispatch_matches_active_prompt(&steering_dispatch)
            .expect("dispatch match should use prompt owner"));
    }

    #[tokio::test]
    async fn stale_steering_dispatch_is_rejected() {
        let (runtime, session_id, agent_id, attachment_id, _, provider_run_id) =
            runtime_with_active_prompt().await;
        let steering_dispatch = dispatch(
            &session_id,
            &agent_id,
            &attachment_id,
            &provider_run_id,
            "queued-steering-prompt",
            "steer now",
            Some("stale-active-prompt".to_string()),
            true,
        );

        let error = runtime
            .owned
            .ensure_prompt_dispatch_matches_active_prompt(&steering_dispatch)
            .expect_err("stale steering dispatch should fail");
        assert!(
            error
                .to_string()
                .contains("no longer matches the active prompt"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn steering_dispatch_records_provider_input() {
        let (runtime, session_id, agent_id, attachment_id, active_prompt_id, provider_run_id) =
            runtime_with_active_prompt().await;
        let steering_text = "STEERING_DELIVERY_PROOF";
        let steering_dispatch = dispatch(
            &session_id,
            &agent_id,
            &attachment_id,
            &provider_run_id,
            "queued-steering-prompt",
            steering_text,
            Some(active_prompt_id),
            true,
        );

        runtime
            .enqueue_prompt_dispatch(&steering_dispatch)
            .await
            .expect("steering dispatch should deliver");

        let input_records = runtime.owned.terminal_stream.input_records();
        assert!(
            input_records.iter().any(|record| {
                record.provider_run_id == provider_run_id
                    && String::from_utf8_lossy(&record.bytes).contains(steering_text)
            }),
            "steering prompt should be recorded as provider input: {input_records:?}"
        );
    }
}

impl KernelRuntimeState {
    pub(super) async fn enqueue_prompt_dispatch(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
    ) -> Result<(), DaemonError> {
        {
            let owned = &self.owned;
            if !owned.ensure_prompt_dispatch_matches_active_prompt(dispatch)? {
                return Ok(());
            }
            let has_managed_process = owned
                .provider_process_tracking
                .read()
                .run_processes
                .contains_key(&dispatch.provider_run_id);
            if has_managed_process {
                let _ = self
                    .reconcile_provider_run_exit(&dispatch.session_id, &dispatch.provider_run_id)
                    .await?;
            }
            let result = self
                .enqueue_prompt_dispatch_after_liveness(dispatch, owned)
                .await;
            if result.is_ok() {
                owned.update_metaagent_event_prompt_delivery_for_prompt(
                    &dispatch.prompt_id,
                    crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Delivered,
                    None,
                );
            }
            result
        }
    }

    pub(super) async fn enqueue_prompt_dispatch_after_liveness(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
        owned: &KernelRuntimeOwnedState,
    ) -> Result<(), DaemonError> {
        if !owned.ensure_prompt_dispatch_matches_active_prompt(dispatch)? {
            return Ok(());
        }
        owned.echo_prompt_to_other_attachments(
            &dispatch.session_id,
            &dispatch.provider_run_id,
            &dispatch.prompt_id,
            &dispatch.source_attachment_id,
            &dispatch.prompt,
            &dispatch.attachments,
        );
        let provider_run = owned
            .ensure_provider_run_in_session(&dispatch.session_id, &dispatch.provider_run_id)?;
        if provider_run.state() != crate::provider::ProviderRunState::Running {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: dispatch.provider_run_id.clone(),
                state: provider_run.state(),
                operation: "submit prompt",
            });
        }
        if owned
            .provider_store
            .run_uses_structured_prompt_io(&provider_run)
        {
            self.observe_git_before_prompt_dispatch(dispatch, &provider_run)
                .await;
            if !dispatch.steering {
                owned.note_prompt_started(&dispatch.provider_run_id);
            }
            let prompt_with_handoff = owned.prompt_with_pending_context_handoff(
                &dispatch.session_id,
                &dispatch.agent_id,
                &dispatch.source_attachment_id,
                &provider_run,
                &dispatch.prompt,
            );
            let granted_skill_context = owned.granted_skill_hidden_context(
                &dispatch.session_id,
                &dispatch.agent_id,
                &prompt_with_handoff,
            )?;
            let hidden_system_context =
                join_hidden_context(&dispatch.hidden_system_context, &granted_skill_context);
            let mode = if owned
                .agent_store
                .get_agent(&dispatch.agent_id)?
                .is_metaagent()
            {
                crate::prompt_assembly::PromptAssemblyMode::MetaagentProviderTurn
            } else {
                crate::prompt_assembly::PromptAssemblyMode::NormalProviderTurn
            };
            let result = owned.provider_store.enqueue_structured_prompt_submit(
                dispatch.session_id.clone(),
                dispatch.provider_run_id.clone(),
                dispatch.agent_id.clone(),
                &provider_run,
                &prompt_with_handoff,
                &hidden_system_context,
                &dispatch.attachments,
                mode,
                dispatch.steering,
            );
            if result.is_ok() {
                owned.consume_pending_context_handoff(
                    &dispatch.session_id,
                    &dispatch.agent_id,
                    &provider_run,
                );
            }
            return result;
        }
        if !crate::scheduler::runtime::is_workflow_prompt_attachment(&dispatch.source_attachment_id)
        {
            let attachment = owned
                .attachment_store
                .get_attachment(&dispatch.source_attachment_id)?;
            if attachment.session_id() != dispatch.session_id {
                return Err(DaemonError::AttachmentNotInSession {
                    session_id: dispatch.session_id.clone(),
                    attachment_id: dispatch.source_attachment_id.clone(),
                });
            }
        }
        let prompt_with_handoff = owned.prompt_with_pending_context_handoff(
            &dispatch.session_id,
            &dispatch.agent_id,
            &dispatch.source_attachment_id,
            &provider_run,
            &dispatch.prompt,
        );
        let prompt_with_hidden_context =
            join_hidden_context(&dispatch.hidden_system_context, &prompt_with_handoff);
        let provider_prompt = owned.apply_granted_skill_summary(
            &dispatch.session_id,
            &dispatch.agent_id,
            &prompt_with_hidden_context,
        )?;
        self.observe_git_before_prompt_dispatch(dispatch, &provider_run)
            .await;
        owned.terminal_stream.record_input(
            &dispatch.session_id,
            &dispatch.provider_run_id,
            &dispatch.source_attachment_id,
            provider_prompt.as_bytes(),
        );
        let mut has_managed_process = owned
            .provider_process_tracking
            .read()
            .run_processes
            .contains_key(&dispatch.provider_run_id);
        if crate::provider::provider_run_is_claude_headless(&provider_run) && !has_managed_process {
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(10_000);
            while tokio::time::Instant::now() < deadline {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                has_managed_process = owned
                    .provider_process_tracking
                    .read()
                    .run_processes
                    .contains_key(&dispatch.provider_run_id);
                if has_managed_process {
                    break;
                }
            }
            if !has_managed_process {
                return Err(DaemonError::LocalTransport {
                    operation: "submit Claude headless prompt",
                    message: format!(
                        "provider process for `{}` was not ready",
                        dispatch.provider_run_id
                    ),
                });
            }
        }
        if !has_managed_process {
            if !dispatch.steering {
                owned.note_prompt_started(&dispatch.provider_run_id);
            }
            return Ok(());
        }
        if crate::provider::provider_run_uses_claude_native_bridge(&provider_run) {
            let dispatch_with_handoff = crate::app::KernelPromptDispatch {
                session_id: dispatch.session_id.clone(),
                provider_run_id: dispatch.provider_run_id.clone(),
                agent_id: dispatch.agent_id.clone(),
                prompt_id: dispatch.prompt_id.clone(),
                target_active_prompt_id: dispatch.target_active_prompt_id.clone(),
                source_attachment_id: dispatch.source_attachment_id.clone(),
                prompt: dispatch.prompt.clone(),
                hidden_system_context: owned.hidden_context_with_pending_context_handoff(
                    &dispatch.session_id,
                    &dispatch.agent_id,
                    &provider_run,
                    &dispatch.hidden_system_context,
                ),
                attachments: dispatch.attachments.clone(),
                prompt_origin: dispatch.prompt_origin,
                external_provider: dispatch.external_provider.clone(),
                external_provider_session_id: dispatch.external_provider_session_id.clone(),
                external_provider_turn_id: dispatch.external_provider_turn_id.clone(),
                steering: dispatch.steering,
            };
            let provider_run = provider_run.clone();
            // Claude-headless confirms injection asynchronously via the
            // context-file marker; retry with the app lock released between
            // attempts so a slow provider cannot stall the whole daemon.
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(12_000);
            loop {
                let attempt = self
                    .with_app_side_effect(|app| {
                        app.process_claude_native_prompt_dispatch_attempt_for_runtime(
                            &dispatch.session_id,
                            &dispatch.provider_run_id,
                            &provider_run,
                            &dispatch_with_handoff,
                        )
                    })
                    .await?;
                match attempt {
                    crate::app::ClaudeNativeDispatchAttempt::Completed => break,
                    crate::app::ClaudeNativeDispatchAttempt::AwaitingInjection => {
                        if tokio::time::Instant::now() >= deadline {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                }
            }
            owned.consume_pending_context_handoff(
                &dispatch.session_id,
                &dispatch.agent_id,
                &provider_run,
            );
            if !dispatch.steering {
                owned.note_prompt_started(&dispatch.provider_run_id);
            }
            return Ok(());
        }
        self.with_app_side_effect(|app| {
            app.write_provider_pty_input_for_runtime(
                &dispatch.provider_run_id,
                provider_prompt.as_bytes(),
            )
        })
        .await?;
        owned.consume_pending_context_handoff(
            &dispatch.session_id,
            &dispatch.agent_id,
            &provider_run,
        );
        if !dispatch.steering {
            owned.note_prompt_started(&dispatch.provider_run_id);
        }
        Ok(())
    }

    pub(super) async fn fail_prompt_dispatch(
        &self,
        dispatch: crate::app::KernelPromptDispatch,
        error: DaemonError,
    ) -> Result<(), DaemonError> {
        {
            let owned = &self.owned;
            owned.update_metaagent_event_prompt_delivery_for_prompt(
                &dispatch.prompt_id,
                crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Failed,
                Some(error.to_string()),
            );
            let failed_prompt = owned.prompt_state_owner.active_prompt_for_agent(
                &owned.session_store.get_session(&dispatch.session_id)?,
                &dispatch.agent_id,
            );
            if let Some(failed_prompt) = failed_prompt.as_ref() {
                let _ = self.inject_metaagent_turn_failure_event(
                    &dispatch.session_id,
                    &dispatch.agent_id,
                    failed_prompt,
                    Some(&dispatch.provider_run_id),
                    &error.to_string(),
                );
            }
            let _ = owned.cancel_active_prompt_only(&dispatch.session_id, &dispatch.agent_id);
            let released_claim = owned.clear_prompt_activity(&dispatch.provider_run_id);
            let _ = owned.session_snapshot(&dispatch.session_id);
            let recipients = owned
                .attachment_store
                .list_session_attachment_ids(&dispatch.session_id);
            owned.record_notice(
                &dispatch.session_id,
                Some(&dispatch.provider_run_id),
                recipients,
                format!("Prompt dispatch failed after acknowledgement: {error}"),
            );
            if released_claim {
                self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
            }
            Err(error)
        }
    }

    pub(super) fn spawn_workflow_prompt_dispatches(&self, dispatches: WorkflowPromptDispatches) {
        for provider_run_id in dispatches.starting_provider_runs {
            self.spawn_detached_workflow_provider_launch(provider_run_id);
        }
        for dispatch in dispatches.local {
            self.spawn_prompt_dispatch(dispatch, self.provider_runtime_lanes.clone());
        }
        for dispatch in dispatches.remote {
            self.spawn_remote_prompt_dispatch(dispatch);
        }
    }

    fn spawn_detached_workflow_provider_launch(&self, provider_run_id: String) {
        let state = self.clone();
        tokio::spawn(async move {
            let run = match state.owned.provider_store.get_run(&provider_run_id) {
                Ok(run) if run.state() == crate::provider::ProviderRunState::Starting => run,
                _ => return,
            };
            let started = crate::app::StartedProviderLaunch {
                run: run.clone(),
                previous_active_run_id: None,
            };
            let spawn_result = state
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).spawn_for_launch(&run)
                })
                .await;
            if let Err(error) = spawn_result {
                state.fail_provider_launch(&started, &error).await;
                return;
            }
            state.owned.provider_run_projection.update(run.clone());
            let runtime_init_delay_ms = state
                .owned
                .config_projection
                .snapshot()
                .provider_runtime_init_delay_ms;
            if runtime_init_delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(runtime_init_delay_ms)).await;
            }
            let binding = tokio::task::spawn_blocking(move || {
                crate::provider::ProviderProcessService::initialize_runtime_binding(&run)
            })
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "initialize workflow provider runtime",
                message: error.to_string(),
            });
            match binding {
                Ok(Ok(binding)) => state.finish_provider_launch(&started, binding).await,
                Ok(Err(error)) | Err(error) => {
                    state.fail_provider_launch(&started, &error).await;
                }
            }
        });
    }

    pub(super) async fn enqueue_prompt_abort(
        &self,
        dispatch: &crate::app::KernelPromptAbortDispatch,
    ) -> Result<(), DaemonError> {
        {
            let owned = &self.owned;
            owned.reap_structured_prompt_jobs();
            self.reconcile_provider_run_exit(&dispatch.session_id, &dispatch.provider_run_id)
                .await?;
            let provider_run = owned
                .ensure_provider_run_in_session(&dispatch.session_id, &dispatch.provider_run_id)?;
            if provider_run.state() != crate::provider::ProviderRunState::Running {
                return Err(DaemonError::InvalidProviderRunState {
                    provider_run_id: dispatch.provider_run_id.clone(),
                    state: provider_run.state(),
                    operation: "submit prompt",
                });
            }
            if owned
                .provider_store
                .run_uses_structured_prompt_io(&provider_run)
            {
                return owned.provider_store.enqueue_structured_prompt_abort(
                    dispatch.session_id.clone(),
                    dispatch.provider_run_id.clone(),
                );
            }
            owned.terminal_stream.record_input(
                &dispatch.session_id,
                &dispatch.provider_run_id,
                &dispatch.source_attachment_id,
                b"\x03",
            );
            self.with_app_side_effect(|app| {
                app.write_provider_pty_input_for_runtime(&dispatch.provider_run_id, b"\x03")
            })
            .await?;
            Ok(())
        }
    }

    pub(super) async fn structured_prompt_io_in_flight(&self, provider_run_id: &str) -> bool {
        {
            let owned = &self.owned;
            owned
                .provider_store
                .structured_prompt_io_in_flight(provider_run_id)
        }
    }

    pub(super) async fn fail_prompt_abort(
        &self,
        dispatch: crate::app::KernelPromptAbortDispatch,
        error: DaemonError,
    ) -> Result<(), DaemonError> {
        {
            let owned = &self.owned;
            let recipients = owned
                .attachment_store
                .list_session_attachment_ids(&dispatch.session_id);
            owned.record_notice(
                &dispatch.session_id,
                Some(&dispatch.provider_run_id),
                recipients,
                format!("Prompt cancellation dispatch failed after acknowledgement: {error}"),
            );
            Err(error)
        }
    }

    pub(crate) fn spawn_prompt_dispatch(
        &self,
        dispatch: crate::app::KernelPromptDispatch,
        provider_runtime_lanes: ProviderRunOperationLanes,
    ) {
        let state = self.clone();
        tokio::spawn(async move {
            let _permit = provider_runtime_lanes
                .acquire(&dispatch.provider_run_id)
                .await;
            if let Err(error) = state.enqueue_prompt_dispatch(&dispatch).await {
                let _ = state.fail_prompt_dispatch(dispatch, error).await;
            }
        });
    }

    pub(crate) fn spawn_queued_prompt_steer_dispatch(
        &self,
        dispatch: crate::app::KernelPromptDispatch,
        provider_runtime_lanes: ProviderRunOperationLanes,
    ) {
        let state = self.clone();
        tokio::spawn(async move {
            let _permit = provider_runtime_lanes
                .acquire(&dispatch.provider_run_id)
                .await;
            if let Err(error) = state.enqueue_prompt_dispatch(&dispatch).await {
                let recipients = state
                    .owned
                    .attachment_store
                    .list_session_attachment_ids(&dispatch.session_id);
                state.owned.record_notice(
                    &dispatch.session_id,
                    Some(&dispatch.provider_run_id),
                    recipients,
                    format!("Queued prompt steer dispatch failed: {error}"),
                );
            }
        });
    }

    pub(crate) fn spawn_prompt_abort(
        &self,
        dispatch: crate::app::KernelPromptAbortDispatch,
        provider_runtime_lanes: ProviderRunOperationLanes,
    ) {
        let state = self.clone();
        tokio::spawn(async move {
            let _permit = provider_runtime_lanes
                .acquire(&dispatch.provider_run_id)
                .await;
            loop {
                let outcome = match state.enqueue_prompt_abort(&dispatch).await {
                    Ok(()) => PromptAbortDispatchOutcome::Done,
                    Err(_)
                        if state
                            .structured_prompt_io_in_flight(&dispatch.provider_run_id)
                            .await =>
                    {
                        PromptAbortDispatchOutcome::Retry
                    }
                    Err(error) => {
                        let _ = state.fail_prompt_abort(dispatch.clone(), error).await;
                        PromptAbortDispatchOutcome::Done
                    }
                };
                match outcome {
                    PromptAbortDispatchOutcome::Done => break,
                    PromptAbortDispatchOutcome::Retry => {
                        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                    }
                }
            }
        });
    }
}

fn join_hidden_context(first: &str, second: &str) -> String {
    match (first.trim(), second.trim()) {
        ("", "") => String::new(),
        (first, "") => first.to_string(),
        ("", second) => second.to_string(),
        (first, second) => format!("{first}\n\n{second}"),
    }
}

enum PromptAbortDispatchOutcome {
    Done,
    Retry,
}
