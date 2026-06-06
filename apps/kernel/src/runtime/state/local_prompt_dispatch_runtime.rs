//! Local provider prompt dispatch and abort execution.
//!
//! This module owns writing admitted prompts or cancellation signals to local provider runtimes,
//! including structured prompt I/O and provider-runtime lane spawning.

use super::*;

impl KernelRuntimeState {
    pub(super) async fn enqueue_prompt_dispatch(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
    ) -> Result<(), DaemonError> {
        {
            let owned = &self.owned;
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
            self.enqueue_prompt_dispatch_after_liveness(dispatch, owned)
                .await
        }
    }

    pub(super) async fn enqueue_prompt_dispatch_after_liveness(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
        owned: &KernelRuntimeOwnedState,
    ) -> Result<(), DaemonError> {
        owned.echo_prompt_to_other_attachments(
            &dispatch.session_id,
            &dispatch.provider_run_id,
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
            owned.note_prompt_started(&dispatch.provider_run_id);
            let prompt_with_handoff = owned.prompt_with_pending_context_handoff(
                &dispatch.session_id,
                &dispatch.agent_id,
                &dispatch.source_attachment_id,
                &dispatch.prompt,
            );
            let granted_skill_context = owned.granted_skill_hidden_context(
                &dispatch.session_id,
                &dispatch.agent_id,
                &prompt_with_handoff,
            )?;
            let hidden_system_context =
                join_hidden_context(&dispatch.hidden_system_context, &granted_skill_context);
            return owned.provider_store.enqueue_structured_prompt_submit(
                dispatch.session_id.clone(),
                dispatch.provider_run_id.clone(),
                dispatch.agent_id.clone(),
                &provider_run,
                &prompt_with_handoff,
                &hidden_system_context,
                &dispatch.attachments,
            );
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
        let has_managed_process = owned
            .provider_process_tracking
            .read()
            .run_processes
            .contains_key(&dispatch.provider_run_id);
        if !has_managed_process {
            owned.note_prompt_started(&dispatch.provider_run_id);
            return Ok(());
        }
        self.with_app_side_effect(|app| {
            app.write_provider_pty_input_for_runtime(
                &dispatch.provider_run_id,
                provider_prompt.as_bytes(),
            )
        })
        .await?;
        owned.note_prompt_started(&dispatch.provider_run_id);
        Ok(())
    }

    pub(super) async fn fail_prompt_dispatch(
        &self,
        dispatch: crate::app::KernelPromptDispatch,
        error: DaemonError,
    ) -> Result<(), DaemonError> {
        {
            let owned = &self.owned;
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
