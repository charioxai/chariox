//! MP-08 / MP-09 / MP-10 / MP-11: bounded cancellation and physical provider settlement.
use super::*;
const PROMPT_ABORT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
enum PromptAbortDispatchOutcome {
    Done,
    Retry,
}
impl KernelRuntimeState {
    pub(super) async fn wait_for_structured_prompt_abort_settlement(
        &self,
        dispatch: &crate::app::KernelPromptAbortDispatch,
        completion_signal: &crate::provider::ProviderRunActorCompletionSignal,
        mut completion_sequence: u64,
    ) {
        loop {
            if self
                .owned
                .cancelling_prompt_for_abort(
                    &dispatch.session_id,
                    &dispatch.provider_run_id,
                    &dispatch.prompt_id,
                )
                .is_none()
            {
                return;
            }
            // MP-08/MP-10/MP-11: another output pump may settle the owner after
            // its actor completion was already observed. Recheck that exact
            // prompt even when there is no further provider event. This delay
            // never acknowledges an abort or settles a still-cancelling turn.
            let _ = tokio::time::timeout(
                std::time::Duration::from_millis(25),
                completion_signal.wait_for_change_after(completion_sequence),
            )
            .await;
            completion_sequence = completion_signal.sequence();
            self.reap_structured_prompt_jobs_and_dispatch();
            let prompt_is_still_cancelling = self
                .owned
                .cancelling_prompt_for_abort(
                    &dispatch.session_id,
                    &dispatch.provider_run_id,
                    &dispatch.prompt_id,
                )
                .is_some();
            if !prompt_is_still_cancelling {
                self.spawn_workflow_prompt_dispatches(self.owned.workflow_retry_blocked_claims());
                let _ = self.owned.session_snapshot(&dispatch.session_id);
                return;
            }
        }
    }

    pub(crate) fn spawn_prompt_abort(
        &self,
        dispatch: crate::app::KernelPromptAbortDispatch,
        provider_runtime_lanes: ProviderRunOperationLanes,
    ) {
        let state = self.clone();
        tokio::spawn(async move {
            let attempt = async {
                let _permit = provider_runtime_lanes
                    .acquire(&dispatch.provider_run_id)
                    .await;
                let structured = state
                    .owned
                    .provider_store
                    .get_run(&dispatch.provider_run_id)
                    .is_ok_and(|run| {
                        state
                            .owned
                            .provider_store
                            .run_uses_structured_prompt_io(&run)
                    });
                loop {
                    let completion_signal = structured
                        .then(|| state.owned.provider_store.run_actor_completion_signal());
                    let completion_sequence = completion_signal
                        .as_ref()
                        .map(|signal| signal.sequence())
                        .unwrap_or_default();
                    let outcome = match state.enqueue_prompt_abort(&dispatch).await {
                        Ok(()) if !structured => PromptAbortDispatchOutcome::Done,
                        Ok(()) => {
                            state
                                .wait_for_structured_prompt_abort_settlement(
                                    &dispatch,
                                    completion_signal
                                        .as_ref()
                                        .expect("structured abort should have a completion signal"),
                                    completion_sequence,
                                )
                                .await;
                            PromptAbortDispatchOutcome::Done
                        }
                        Err(_)
                            if state
                                .structured_prompt_io_in_flight(&dispatch.provider_run_id)
                                .await =>
                        {
                            PromptAbortDispatchOutcome::Retry
                        }
                        Err(error) => {
                            state.record_prompt_abort_failure(&dispatch, &error);
                            return true;
                        }
                    };
                    match outcome {
                        PromptAbortDispatchOutcome::Done => break,
                        PromptAbortDispatchOutcome::Retry => {
                            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                        }
                    }
                }
                false
            };
            // The deadline covers provider I/O and lane admission, never physical cleanup.
            let replace_run = tokio::time::timeout(PROMPT_ABORT_TIMEOUT, attempt)
                .await
                .unwrap_or(true);
            if let Err(error) = state
                .finish_owned_prompt_abort(&dispatch, replace_run)
                .await
            {
                state.record_prompt_abort_failure(&dispatch, &error);
            }
        });
    }
    pub(super) fn record_prompt_abort_failure(
        &self,
        dispatch: &crate::app::KernelPromptAbortDispatch,
        error: &DaemonError,
    ) {
        self.owned.record_notice(
            &dispatch.session_id,
            Some(&dispatch.provider_run_id),
            self.owned
                .attachment_store
                .list_session_attachment_ids(&dispatch.session_id),
            format!("Provider cancellation cleanup failed: {error}"),
        );
        crate::logging::warn_with_fields(
            "daemon.prompt_cancellation",
            "provider cancellation cleanup failed",
            serde_json::json!({"session_id":dispatch.session_id,"provider_run_id":dispatch.provider_run_id,"prompt_id":dispatch.prompt_id,"error":error.to_string()}),
        );
    }

    pub(super) async fn finish_owned_prompt_abort(
        &self,
        dispatch: &crate::app::KernelPromptAbortDispatch,
        replace_run: bool,
    ) -> Result<(), DaemonError> {
        if self
            .owned
            .cancelling_prompt_for_abort(
                &dispatch.session_id,
                &dispatch.provider_run_id,
                &dispatch.prompt_id,
            )
            .is_none()
        {
            return Ok(());
        }
        // The protocol ACK can precede tool exit. Retire the owned process group
        // before releasing the turn; its persisted provider identity resumes on the next prompt.
        let finished = self.with_app_side_effect(|app| {
            let session = self.owned.session_store.get_session(&dispatch.session_id)?;
            let agent = self.owned.provider_store.get_run(&dispatch.provider_run_id)?
                .agent_instance_id().map(str::to_string)
                .ok_or_else(|| DaemonError::NoActiveProviderRun { session_id: dispatch.session_id.clone() })?;
            self.owned.prompt_state_owner.with_cancelling_prompt_if_matches(
                &session, &agent, &dispatch.prompt_id, || {
                    if self.owned.provider_store.get_run_for_agent(&dispatch.session_id, &agent)
                        .is_none_or(|run| run.id() != dispatch.provider_run_id) {
                        return Ok(false);
                    }
                    let process_key = app.pty().process_key(&dispatch.provider_run_id).ok().or_else(||
                        self.owned.provider_process_tracking.read().run_processes.get(&dispatch.provider_run_id).cloned());
                    app.pty_mut().terminate_owned_processes(&dispatch.provider_run_id)?;
                    let (_, removed_key) = crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(&dispatch.provider_run_id)?;
                    if process_key.as_ref().is_some_and(|key| app.pty().has_process_key(key)) {
                        return Err(DaemonError::PtyCleanup { provider_run_id: dispatch.provider_run_id.clone(),
                            message: "the provider process is still shared; cancellation has not physically settled".into() });
                    }
                    self.owned.remove_provider_process_tracking_for_run(&dispatch.provider_run_id, removed_key);
                    // Borrowed endpoints keep their documented interrupt semantics.
                    // A timed-out run must be replaced even if launch has not materialized its PTY yet.
                    if process_key.is_some() || replace_run {
                        let ended = self.owned.provider_store.mark_run_ended_provider_only(&dispatch.session_id, &dispatch.provider_run_id)?;
                        self.owned.provider_run_projection.update(ended.into_run());
                        self.owned.clear_active_provider_run_session_pointer(&dispatch.session_id, &dispatch.provider_run_id)?;
                    }
                    Ok(true)
                })
        }).await?;
        if finished != Some(true) {
            return Ok(());
        }
        let Some((agent_id, prompt)) = self.owned.cancelling_prompt_for_abort(
            &dispatch.session_id,
            &dispatch.provider_run_id,
            &dispatch.prompt_id,
        ) else {
            return Ok(());
        };
        let settlement = self.owned.settle_agent_task(
            &dispatch.session_id,
            &agent_id,
            &prompt,
            &dispatch.provider_run_id,
            true,
        )?;
        self.owned
            .workflow_cancel_prompt(&dispatch.session_id, &prompt)?;
        let cancellation = match self
            .owned
            .finalize_local_prompt_cancellation_with_queued_advance(
                &dispatch.session_id,
                &agent_id,
                &dispatch.prompt_id,
                Some(&dispatch.provider_run_id),
            ) {
            Ok(cancellation) => cancellation,
            Err(DaemonError::NoActivePrompt { .. }) => {
                // A concurrent completion already released this exact cancelled turn.
                Box::pin(self.finish_agent_task_settlement(settlement, &prompt)).await?;
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        if let Err(error) = Box::pin(self.finish_agent_task_settlement(settlement, &prompt)).await {
            self.record_prompt_abort_failure(dispatch, &error);
        }
        if cancellation.released_claim {
            self.spawn_workflow_prompt_dispatches(self.owned.workflow_retry_blocked_claims());
        }
        if let Some(next) = cancellation.dispatch {
            if let Err(error) = self
                .enqueue_prompt_dispatch_after_liveness(&next, &self.owned)
                .await
            {
                let _ = self.fail_prompt_dispatch(next, error).await;
            }
        } else {
            Box::pin(
                self.advance_project_queued_prompt_after_settlement(
                    &dispatch.session_id,
                    &agent_id,
                ),
            )
            .await;
        }
        Ok(())
    }
}
