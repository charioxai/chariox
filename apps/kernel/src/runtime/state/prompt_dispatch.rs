//! Async direct-prompt dispatch entry points.
//!
//! This layer validates runtime state, starts or queues prompts, and hands provider-specific
//! submission work to the provider runtime without exposing owned-state internals to transports.

use super::*;

impl KernelRuntimeState {
    pub(crate) async fn submit_prepared_prompt(
        &self,
        prepared: crate::app::KernelPreparedPromptSubmission,
    ) -> Result<crate::app::KernelPromptSubmission, DaemonError> {
        {
            let owned = &self.owned;
            if let Some(mut submission) = owned.submit_local_prepared_prompt(&prepared)? {
                self.finish_owned_prompt_submission_workflow_start(&mut submission)
                    .await?;
                return Ok(submission);
            }
            if let Some(mut submission) = owned.submit_remote_prepared_prompt(&prepared)? {
                self.finish_owned_prompt_submission_workflow_start(&mut submission)
                    .await?;
                self.spawn_remote_prompt_projection_drain_if_needed(&submission);
                return Ok(submission);
            }
            let session_id = prepared.session_id.clone();
            let target_agent_id = prepared.prompt.target_agent_id().to_string();
            let attachment_id = prepared.prompt.source_attachment_id().to_string();
            let has_active = owned
                .prompt_state_owner
                .active_prompt_for_agent(
                    &owned.session_store.get_session(&session_id)?,
                    &target_agent_id,
                )
                .is_some();
            let has_run = owned
                .provider_store
                .get_run_for_agent(&session_id, &target_agent_id)
                .is_some();
            if !has_active && !has_run {
                let is_remote_agent = owned
                    .agent_store
                    .get_agent(&target_agent_id)?
                    .remote_execution()
                    .is_some();
                if crate::scheduler::runtime::is_workflow_prompt_attachment(&attachment_id) {
                    owned.workflow_ensure_provider_run(&session_id, &target_agent_id)?;
                } else if is_remote_agent {
                    if let Some(mut submission) = owned.submit_remote_prepared_prompt(&prepared)? {
                        self.finish_owned_prompt_submission_workflow_start(&mut submission)
                            .await?;
                        self.spawn_remote_prompt_projection_drain_if_needed(&submission);
                        return Ok(submission);
                    }
                } else {
                    self.with_app_side_effect(|app| {
                        app.ensure_prompt_provider_run_for_agent(&session_id, &target_agent_id)
                    })
                    .await?;
                };
                if let Some(mut submission) = owned.submit_local_prepared_prompt(&prepared)? {
                    self.finish_owned_prompt_submission_workflow_start(&mut submission)
                        .await?;
                    return Ok(submission);
                }
            }
            Err(DaemonError::LocalTransport {
                operation: "submit prepared prompt",
                message:
                    "owned prompt runtime could not admit prompt without side-effect completion"
                        .to_string(),
            })
        }
    }

    pub(super) async fn finish_owned_prompt_submission_workflow_start(
        &self,
        submission: &mut crate::app::KernelPromptSubmission,
    ) -> Result<(), DaemonError> {
        let crate::session::PromptSubmissionOutcome::Started { prompt } = &submission.outcome
        else {
            return Ok(());
        };
        if !crate::scheduler::runtime::is_workflow_prompt_attachment(prompt.source_attachment_id())
        {
            return Ok(());
        }
        let session_id = submission.session.id().to_string();
        let prompt = prompt.clone();
        if let Some(remote_dispatch) = submission.remote_dispatch.as_mut() {
            remote_dispatch.workflow_context = Some(
                self.with_app_side_effect(|app| {
                    crate::app::RemoteWorkflowTurnContextResolver::new(app)
                        .remote_workflow_turn_context_for_prompt(
                            &session_id,
                            prompt.target_agent_id(),
                            &prompt,
                        )
                })
                .await?,
            );
        }
        self.owned.workflow_start_prompt(&session_id, &prompt)
    }

    pub(crate) async fn cancel_agent_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<crate::app::KernelPromptCancellation, DaemonError> {
        {
            let owned = &self.owned;
            if let Some(cancellation) = self
                .cancel_remote_agent_prompt_if_remote(session_id, target_agent_id, attachment_id)
                .await?
            {
                return Ok(cancellation);
            }
            if let Some(cancellation) =
                owned.cancel_local_prompt(session_id, target_agent_id, attachment_id)?
            {
                return Ok(cancellation);
            }
            Err(DaemonError::LocalTransport {
                operation: "cancel prompt",
                message:
                    "owned prompt runtime could not cancel prompt without side-effect completion"
                        .to_string(),
            })
        }
    }

    pub(crate) async fn steer_queued_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
        prompt_id: &str,
    ) -> Result<crate::app::KernelQueuedPromptSteer, DaemonError> {
        {
            let owned = &self.owned;
            if let Some(steer) =
                owned.steer_queued_prompt(session_id, target_agent_id, attachment_id, prompt_id)?
            {
                return Ok(steer);
            }
            Err(DaemonError::LocalTransport {
                operation: "steer queued prompt",
                message: "queued prompt steering for remote agents is not implemented".to_string(),
            })
        }
    }

    pub(crate) async fn dispatch_next_queued_prompt_after_external_settlement(
        &self,
        session_id: &str,
        target_agent_id: &str,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        let dispatch = {
            self.owned.advance_next_queued_prompt_dispatch(
                session_id,
                target_agent_id,
                provider_run_id,
            )?
        };
        let Some(dispatch) = dispatch else {
            return Ok(false);
        };
        self.spawn_prompt_dispatch(dispatch, self.provider_runtime_lanes.clone());
        Ok(true)
    }

    pub(crate) async fn cancel_queued_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
        prompt_id: &str,
    ) -> Result<crate::app::KernelQueuedPromptCancellation, DaemonError> {
        {
            self.owned
                .cancel_queued_prompt(session_id, target_agent_id, attachment_id, prompt_id)
        }
    }

    pub(crate) async fn complete_agent_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        next_queued_prompt: Option<&crate::session::PromptQueueItem>,
    ) -> Result<crate::session::PromptCompletion, DaemonError> {
        let owned = &self.owned;
        let owned_provider_run_id = owned
            .provider_run_projection
            .get_for_agent(session_id, target_agent_id)
            .or_else(|| {
                owned
                    .provider_store
                    .get_run_for_agent(session_id, target_agent_id)
            })
            .map(|run| run.id().to_string());
        if let Some(completion) = self
            .complete_remote_agent_prompt_if_remote(
                session_id,
                target_agent_id,
                owned_provider_run_id.clone(),
                next_queued_prompt,
            )
            .await?
        {
            self.inject_orphaned_metaagent_task_event_after_turn(
                session_id,
                target_agent_id,
                &completion,
            )?;
            return Ok(completion);
        }
        if next_queued_prompt.is_none() {
            {
                let owned = &self.owned;
                if let Some(completion) = owned.complete_local_prompt_without_advance(
                    session_id,
                    target_agent_id,
                    owned_provider_run_id.as_deref(),
                )? {
                    self.observe_git_after_completed_prompt(
                        Some(session_id),
                        owned_provider_run_id.as_deref(),
                        &completion.completion.completed,
                    )
                    .await;
                    if completion.completion.completed.workflow_run_id().is_some() {
                        let dispatches = owned.workflow_complete_prompt(
                            session_id,
                            &completion.completion.completed,
                            owned_provider_run_id.as_deref(),
                        )?;
                        self.spawn_workflow_prompt_dispatches(dispatches);
                    }
                    if completion.released_claim
                        && completion.completion.completed.workflow_run_id().is_none()
                    {
                        self.spawn_workflow_prompt_dispatches(
                            owned.workflow_retry_blocked_claims(),
                        );
                    }
                    self.inject_orphaned_metaagent_task_event_after_turn(
                        session_id,
                        target_agent_id,
                        &completion.completion,
                    )?;
                    return Ok(completion.completion);
                }
            }
        } else if let Some(next_queued_prompt) = next_queued_prompt {
            if let Some(completion) = owned.complete_local_prompt_with_queued_advance(
                session_id,
                target_agent_id,
                owned_provider_run_id.as_deref(),
                next_queued_prompt,
            )? {
                let completion_result = completion.completion;
                self.observe_git_after_completed_prompt(
                    Some(session_id),
                    owned_provider_run_id.as_deref(),
                    &completion_result.completed,
                )
                .await;
                if completion_result.completed.workflow_run_id().is_some() {
                    let dispatches = owned.workflow_complete_prompt(
                        session_id,
                        &completion_result.completed,
                        owned_provider_run_id.as_deref(),
                    )?;
                    self.spawn_workflow_prompt_dispatches(dispatches);
                }
                if let Some(started_next) = completion_result.started_next.as_ref() {
                    if crate::scheduler::runtime::is_workflow_prompt_attachment(
                        started_next.source_attachment_id(),
                    ) {
                        owned.workflow_start_prompt(session_id, started_next)?;
                    }
                }
                if let Some(dispatch) = completion.dispatch {
                    if let Err(error) = self.enqueue_prompt_dispatch(&dispatch).await {
                        let _ = self.fail_prompt_dispatch(dispatch, error).await;
                    }
                }
                self.inject_orphaned_metaagent_task_event_after_turn(
                    session_id,
                    target_agent_id,
                    &completion_result,
                )?;
                return Ok(completion_result);
            }
        }
        Err(DaemonError::LocalTransport {
            operation: "complete prompt",
            message:
                "owned prompt runtime could not complete prompt without side-effect completion"
                    .to_string(),
        })
    }
}
