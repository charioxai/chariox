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
        let owned = &self.owned;
        if owned
            .agent_store
            .get_agent(target_agent_id)?
            .remote_execution()
            .is_none()
        {
            if let Some(steer) =
                owned.steer_queued_prompt(session_id, target_agent_id, attachment_id, prompt_id)?
            {
                return Ok(steer);
            }
            return Err(DaemonError::LocalTransport {
                operation: "steer queued prompt",
                message: "local queued prompt steer did not produce a dispatch".to_string(),
            });
        }
        let prepared = owned
            .prepare_remote_queued_prompt_steer(
                session_id,
                target_agent_id,
                attachment_id,
                prompt_id,
            )?
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "steer remote queued prompt",
                message: format!("agent `{target_agent_id}` is no longer remote"),
            })?;
        let (remote_prompt, required_skills) = self
            .prepare_remote_prompt_skill_context(&prepared.agent, prepared.prompt.prompt())
            .await?;
        let prompt_attachments = prepared.prompt.attachments().to_vec();
        let attachments = tokio::task::spawn_blocking(move || {
            crate::app::serialize_remote_prompt_attachments(&prompt_attachments)
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "serialize remote steering prompt attachments",
            message: error.to_string(),
        })??;
        let payload = RemoteQueuedPromptSteerPayload {
            steer_id: prepared.prompt.id().to_string(),
            target_home_prompt_id: prepared.target_active_prompt_id.clone(),
            prompt: remote_prompt,
            hidden_system_context: prepared.prompt.hidden_system_context().to_string(),
            attachments,
            required_skills,
        };
        let session_id = session_id.to_string();
        let target_agent_id = target_agent_id.to_string();
        let attachment_id = attachment_id.to_string();
        let prompt_id = prompt_id.to_string();
        self.with_app_side_effect(|app| {
            let current = owned
                .prepare_remote_queued_prompt_steer(
                    &session_id,
                    &target_agent_id,
                    &attachment_id,
                    &prompt_id,
                )?
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "steer remote queued prompt",
                    message: format!("agent `{target_agent_id}` is no longer remote"),
                })?;
            if current.target_active_prompt_id != payload.target_home_prompt_id {
                return Err(DaemonError::LocalTransport {
                    operation: "steer remote queued prompt",
                    message: "active prompt changed before remote steering delivery".to_string(),
                });
            }
            let mut remote_execution = current.remote_execution;
            let mut response = send_remote_queued_prompt_steer(
                app,
                &remote_execution,
                &payload,
            );
            if response
                .as_ref()
                .is_err_and(super::remote_prompt_worker_submission_runtime::remote_prompt_error_should_refresh_binding)
            {
                let refreshed = app.refresh_remote_agent_binding(&target_agent_id)?;
                remote_execution = refreshed
                    .remote_execution()
                    .cloned()
                    .ok_or_else(|| DaemonError::LocalTransport {
                        operation: "refresh remote queued prompt steer binding",
                        message: format!(
                            "agent `{target_agent_id}` did not have remote execution after binding refresh"
                        ),
                    })?;
                response = send_remote_queued_prompt_steer(app, &remote_execution, &payload);
            }
            match response? {
                RelayPeerResponse::LeasedPromptSteered { steer_id, .. }
                    if steer_id == payload.steer_id => {}
                other => {
                    return Err(DaemonError::LocalTransport {
                        operation: "steer remote queued prompt",
                        message: format!("unexpected remote prompt steer response: {other:?}"),
                    });
                }
            }
            owned.finish_remote_queued_prompt_steer(
                &session_id,
                &target_agent_id,
                &attachment_id,
                &prompt_id,
                &payload.target_home_prompt_id,
            )
        })
        .await
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

    pub(crate) async fn update_queued_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
        prompt_id: &str,
        prompt: &str,
    ) -> Result<crate::app::KernelQueuedPromptUpdate, DaemonError> {
        {
            self.owned.update_queued_prompt(
                session_id,
                target_agent_id,
                attachment_id,
                prompt_id,
                prompt,
            )
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

#[derive(Clone)]
struct RemoteQueuedPromptSteerPayload {
    steer_id: String,
    target_home_prompt_id: String,
    prompt: String,
    hidden_system_context: String,
    attachments: Vec<crate::transport::relay_peer::RelayPromptAttachment>,
    required_skills: Option<Vec<crate::transport::relay_peer::RequiredRemoteSkill>>,
}

fn send_remote_queued_prompt_steer(
    app: &mut crate::app::DaemonApp,
    remote_execution: &crate::agent::RemoteAgentBinding,
    payload: &RemoteQueuedPromptSteerPayload,
) -> Result<RelayPeerResponse, DaemonError> {
    let relay_config = app.relay_config_for_remote_execution(remote_execution);
    app.block_on_relay_future(
        crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
            &relay_config,
            ClientTarget {
                daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::SteerLeasedPrompt {
                leased_agent_id: remote_execution.leased_agent_id.clone(),
                steer_id: payload.steer_id.clone(),
                target_home_prompt_id: payload.target_home_prompt_id.clone(),
                prompt: payload.prompt.clone(),
                hidden_system_context: payload.hidden_system_context.clone(),
                attachments: payload.attachments.clone(),
                required_skills: payload.required_skills.clone(),
            },
            crate::transport::relay_client::LEASED_PROMPT_SUBMIT_RESPONSE_TIMEOUT,
        ),
    )
}
