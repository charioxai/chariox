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
                if crate::scheduler::runtime::is_workflow_prompt_attachment(&attachment_id) {
                    owned.workflow_ensure_provider_run(&session_id, &target_agent_id)?;
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
            if owned
                .agent_store
                .get_agent(target_agent_id)?
                .remote_execution()
                .is_some()
            {
                let remote_execution = owned
                    .agent_store
                    .get_agent(target_agent_id)?
                    .remote_execution()
                    .cloned()
                    .expect("remote execution checked above");
                match self
                    .with_app_side_effect(|app| {
                        app.block_on_relay_future(
                            crate::transport::relay_client::send_peer_request_via_temporary_connection(
                                app.config(),
                                ClientTarget {
                                    daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                                    daemon_alias: None,
                                },
                                RelayPeerRequest::CancelLeasedPrompt {
                                    leased_agent_id: remote_execution.leased_agent_id.clone(),
                                },
                            ),
                        )
                    })
                    .await?
                {
                    RelayPeerResponse::LeasedPromptCancelled { .. } => {
                        return owned.begin_remote_prompt_cancellation(
                            session_id,
                            target_agent_id,
                            attachment_id,
                        );
                    }
                    other => {
                        return Err(DaemonError::LocalTransport {
                            operation: "cancel remote prompt",
                            message: format!(
                                "unexpected remote prompt cancellation response: {other:?}"
                            ),
                        });
                    }
                }
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
        {
            if let Some(remote_execution) = owned
                .agent_store
                .get_agent(target_agent_id)?
                .remote_execution()
                .cloned()
            {
                let remote_provider_run_id = match self
                    .with_app_side_effect(|app| {
                        app.block_on_relay_future(
                            crate::transport::relay_client::send_peer_request_via_temporary_connection(
                                app.config(),
                                ClientTarget {
                                    daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                                    daemon_alias: None,
                                },
                                RelayPeerRequest::CompleteLeasedPrompt {
                                    leased_agent_id: remote_execution.leased_agent_id.clone(),
                                },
                            ),
                        )
                    })
                    .await?
                {
                    RelayPeerResponse::LeasedPromptCompleted {
                        provider_run_id, ..
                    } => provider_run_id
                        .unwrap_or_else(|| "remote-provider-run-completed".to_string()),
                    other => {
                        return Err(DaemonError::LocalTransport {
                            operation: "complete remote prompt",
                            message: format!(
                                "unexpected remote prompt completion response: {other:?}"
                            ),
                        });
                    }
                };
                let completion = owned.complete_remote_prompt_owner(
                    session_id,
                    target_agent_id,
                    &remote_provider_run_id,
                    next_queued_prompt,
                )?;
                if completion.completed.workflow_run_id().is_some() {
                    let dispatches = owned.workflow_complete_prompt(
                        session_id,
                        &completion.completed,
                        Some(&remote_provider_run_id),
                    )?;
                    self.spawn_workflow_prompt_dispatches(dispatches);
                }
                if let Some(started_next) = completion.started_next.as_ref() {
                    let agent = self.owned.agent_store.get_agent(target_agent_id)?;
                    let materialized = self.ensure_remote_skill_packages_for_agent(&agent).await?;
                    let remote_prompt = self.apply_remote_materialized_skill_prompt_context(
                        &agent,
                        started_next.prompt(),
                        &materialized,
                    )?;
                    let attachments = self
                        .with_app_side_effect(|app| {
                            app.serialize_remote_prompt_attachments(started_next.attachments())
                        })
                        .await?;
                    let workflow_context =
                        if crate::scheduler::runtime::is_workflow_prompt_attachment(
                            started_next.source_attachment_id(),
                        ) {
                            Some(
                                self.with_app_side_effect(|app| {
                                    crate::app::RemoteWorkflowTurnContextResolver::new(app)
                                        .remote_workflow_turn_context_for_prompt(
                                            session_id,
                                            target_agent_id,
                                            started_next,
                                        )
                                })
                                .await?,
                            )
                        } else {
                            None
                        };
                    let submit_result = self
                        .with_app_side_effect(|app| {
                            app.block_on_relay_future(
                                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                                    app.config(),
                                    ClientTarget {
                                        daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                                        daemon_alias: None,
                                    },
                                    RelayPeerRequest::SubmitLeasedPrompt {
                                        leased_agent_id: remote_execution.leased_agent_id.clone(),
                                        prompt: remote_prompt,
                                        attachments,
                                        workflow_context,
                                        required_mcps: Vec::new(),
                                    },
                                ),
                            )
                        })
                        .await?;
                    if let RelayPeerResponse::LeasedPromptSubmitted {
                        provider_run_id, ..
                    } = submit_result
                    {
                        owned.echo_prompt_to_other_attachments(
                            session_id,
                            &provider_run_id,
                            started_next.source_attachment_id(),
                            started_next.prompt(),
                            started_next.attachments(),
                        );
                    }
                }
                return Ok(completion);
            }
        }
        if next_queued_prompt.is_none() {
            {
                let owned = &self.owned;
                if let Some(completion) = owned.complete_local_prompt_without_advance(
                    session_id,
                    target_agent_id,
                    owned_provider_run_id.as_deref(),
                )? {
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

    pub(super) async fn reconcile_provider_run_exit(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        let owned = &self.owned;

        if let Some(exit) = owned.reconcile_provider_run_liveness_provider_phase(
            session_id,
            provider_run_id,
            None,
        )? {
            let (_, process_key) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(provider_run_id)
                })
                .await
                .unwrap_or((false, None));
            owned.remove_provider_process_tracking_for_run(provider_run_id, process_key);
            return Ok(exit.already_ended);
        }

        let process_running = self
            .with_app_side_effect(|app| {
                crate::app::ProviderLaunchProcessRuntime::new(app).poll_running(provider_run_id)
            })
            .await?;
        let Some(exit) = owned.reconcile_provider_run_liveness_provider_phase(
            session_id,
            provider_run_id,
            Some(process_running),
        )?
        else {
            return Ok(false);
        };
        let (_, process_key) = self
            .with_app_side_effect(|app| {
                crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(provider_run_id)
            })
            .await
            .unwrap_or((false, None));
        owned.remove_provider_process_tracking_for_run(provider_run_id, process_key);
        if exit.already_ended {
            return Ok(true);
        }

        let session_outcome = self
            .settle_owned_provider_prompt(session_id, provider_run_id, false, true)
            .await?;
        let recipients = owned
            .attachment_store
            .list_session_attachment_ids(session_id);
        owned.record_notice(
            session_id,
            Some(provider_run_id),
            recipients,
            format!(
                "Provider run `{}` for `{}` ended unexpectedly. {}",
                provider_run_id,
                exit.ended_run.provider(),
                if session_outcome.had_active_prompt {
                    if session_outcome.started_next_prompt {
                        "The active prompt was closed and Arroba advanced the queued backlog onto the next available provider run."
                    } else {
                        "The active prompt was closed without starting the queued backlog."
                    }
                } else {
                    "No active prompt was running."
                }
            ),
        );
        Ok(true)
    }

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
            owned.note_prompt_started(&dispatch.provider_run_id);
            let provider_prompt = owned.apply_granted_skill_summary(
                &dispatch.session_id,
                &dispatch.agent_id,
                &dispatch.prompt,
            )?;
            return owned.provider_store.enqueue_structured_prompt_submit(
                dispatch.session_id.clone(),
                dispatch.provider_run_id.clone(),
                dispatch.agent_id.clone(),
                &provider_run,
                &provider_prompt,
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
        let provider_prompt = owned.apply_granted_skill_summary(
            &dispatch.session_id,
            &dispatch.agent_id,
            &dispatch.prompt,
        )?;
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
        return Ok(());
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
        for dispatch in dispatches.local {
            let state = self.clone();
            tokio::spawn(async move {
                if let Err(error) = state.enqueue_prompt_dispatch(&dispatch).await {
                    let _ = state.fail_prompt_dispatch(dispatch, error).await;
                }
            });
        }
        for dispatch in dispatches.remote {
            self.spawn_remote_prompt_dispatch(dispatch);
        }
    }

    pub(super) async fn finish_remote_prompt_dispatch(
        &self,
        dispatch: crate::app::KernelRemotePromptDispatch,
        result: Result<String, DaemonError>,
    ) -> Result<(), DaemonError> {
        {
            let owned = &self.owned;
            match result {
                Ok(remote_provider_run_id) => {
                    owned.echo_prompt_to_other_attachments(
                        &dispatch.session_id,
                        &remote_provider_run_id,
                        &dispatch.source_attachment_id,
                        &dispatch.prompt,
                        &dispatch.attachments,
                    );
                    Ok(())
                }
                Err(error) => {
                    let _ =
                        owned.cancel_active_prompt_only(&dispatch.session_id, &dispatch.agent_id);
                    let _ = owned.session_snapshot(&dispatch.session_id);
                    let recipients = owned
                        .attachment_store
                        .list_session_attachment_ids(&dispatch.session_id);
                    owned.record_notice(
                        &dispatch.session_id,
                        None,
                        recipients,
                        format!("Remote prompt dispatch failed after acknowledgement: {error}"),
                    );
                    Err(error)
                }
            }
        }
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

    pub(crate) fn spawn_remote_prompt_dispatch(
        &self,
        dispatch: crate::app::KernelRemotePromptDispatch,
    ) {
        let state = self.clone();
        tokio::spawn(async move {
            let agent = match state.owned.agent_store.get_agent(&dispatch.agent_id) {
                Ok(agent) => agent,
                Err(error) => {
                    let _ = state
                        .finish_remote_prompt_dispatch(dispatch, Err(error))
                        .await;
                    return;
                }
            };
            let materialized = match state.ensure_remote_skill_packages_for_agent(&agent).await {
                Ok(materialized) => materialized,
                Err(error) => {
                    let _ = state
                        .finish_remote_prompt_dispatch(dispatch, Err(error))
                        .await;
                    return;
                }
            };
            let prompt = match state.apply_remote_materialized_skill_prompt_context(
                &agent,
                &dispatch.prompt,
                &materialized,
            ) {
                Ok(prompt) => prompt,
                Err(error) => {
                    let _ = state
                        .finish_remote_prompt_dispatch(dispatch, Err(error))
                        .await;
                    return;
                }
            };
            let required_mcps = match state.required_remote_mcps_for_agent(&agent) {
                Ok(required_mcps) => required_mcps,
                Err(error) => {
                    let _ = state
                        .finish_remote_prompt_dispatch(dispatch, Err(error))
                        .await;
                    return;
                }
            };
            let config = state.config_snapshot().await;
            let attachments = dispatch.attachments.clone();
            let serialized_attachments = match tokio::task::spawn_blocking(move || {
                crate::app::serialize_remote_prompt_attachments(&attachments)
            })
            .await
            {
                Ok(result) => result,
                Err(error) => Err(DaemonError::LocalTransport {
                    operation: "serialize remote prompt attachments",
                    message: error.to_string(),
                }),
            };
            let result = match serialized_attachments {
                Ok(attachments) => {
                    match crate::transport::relay_client::send_peer_request_via_temporary_connection(
                        &config,
                        ClientTarget {
                            daemon_id: Some(dispatch.worker_kernel_id.clone()),
                            daemon_alias: None,
                        },
                        RelayPeerRequest::SubmitLeasedPrompt {
                            leased_agent_id: dispatch.leased_agent_id.clone(),
                            prompt: prompt.clone(),
                            attachments,
                            workflow_context: dispatch.workflow_context.clone(),
                            required_mcps,
                        },
                    )
                    .await
                    {
                        Ok(RelayPeerResponse::LeasedPromptSubmitted {
                            provider_run_id, ..
                        }) => Ok(provider_run_id),
                        Ok(other) => Err(DaemonError::LocalTransport {
                            operation: "submit remote prepared prompt",
                            message: format!("unexpected remote prompt response: {other:?}"),
                        }),
                        Err(error) => Err(error),
                    }
                }
                Err(error) => Err(error),
            };
            let _ = state.finish_remote_prompt_dispatch(dispatch, result).await;
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

enum PromptAbortDispatchOutcome {
    Done,
    Retry,
}
