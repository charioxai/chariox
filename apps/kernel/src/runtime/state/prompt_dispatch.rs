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
                let completion_response = self
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
                    .await;
                let (remote_provider_run_id, provider_diagnostic) = match completion_response {
                    Ok(RelayPeerResponse::LeasedPromptCompleted {
                        provider_run_id,
                        provider_diagnostic,
                        git_observations,
                        ..
                    }) => {
                        if let Err(error) = crate::git_observer::append_observations(
                            &owned.operational_history_store,
                            git_observations,
                        ) {
                            crate::logging::warn_with_fields(
                                "daemon.git_observer",
                                "failed to append remote git observations",
                                serde_json::json!({
                                    "session_id": session_id,
                                    "agent_id": target_agent_id,
                                    "error": error.to_string(),
                                }),
                            );
                        }
                        (
                            provider_run_id
                                .unwrap_or_else(|| "remote-provider-run-completed".to_string()),
                            provider_diagnostic,
                        )
                    }
                    Err(error) if remote_prompt_completion_should_treat_as_settled(&error) => {
                        crate::logging::warn_with_fields(
                            "daemon.remote_prompt_dispatch",
                            "remote prompt completion already settled on worker",
                            serde_json::json!({
                                "session_id": session_id,
                                "agent_id": target_agent_id,
                                "worker_kernel_id": remote_execution.worker_kernel_id,
                                "leased_agent_id": remote_execution.leased_agent_id,
                                "error": error.to_string(),
                            }),
                        );
                        (
                            owned_provider_run_id
                                .clone()
                                .unwrap_or_else(|| "remote-provider-run-completed".to_string()),
                            None,
                        )
                    }
                    Err(error) => return Err(error),
                    Ok(other) => {
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
                    if let Some(diagnostic) = provider_diagnostic.as_deref() {
                        owned.workflow_fail_provider_prompt(
                            session_id,
                            &completion.completed,
                            Some(&remote_provider_run_id),
                            diagnostic,
                        )?;
                    } else {
                        let dispatches = owned.workflow_complete_prompt(
                            session_id,
                            &completion.completed,
                            Some(&remote_provider_run_id),
                        )?;
                        self.spawn_workflow_prompt_dispatches(dispatches);
                    }
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
                                        git_context: Some(remote_git_turn_context_for_prompt(
                                            session_id,
                                            target_agent_id,
                                            started_next,
                                        )),
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
                    if let Some(provider_run_id) = owned_provider_run_id.as_deref() {
                        self.observe_git_after_prompt_completion(
                            provider_run_id,
                            &completion.completion.completed,
                        )
                        .await;
                    }
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
                if let Some(provider_run_id) = owned_provider_run_id.as_deref() {
                    self.observe_git_after_prompt_completion(
                        provider_run_id,
                        &completion_result.completed,
                    )
                    .await;
                }
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
        if session_outcome.had_active_prompt {
            if let Some(agent_id) = exit.ended_run.agent_instance_id() {
                let reason = format!(
                    "provider run `{}` for `{}` ended unexpectedly",
                    provider_run_id,
                    exit.ended_run.provider()
                );
                if let Err(error) = self
                    .activate_next_agent_substitute_after_failure(session_id, agent_id, &reason)
                    .await
                {
                    crate::logging::warn_with_fields(
                        "daemon.provider",
                        "automatic substitute activation after provider exit failed",
                        serde_json::json!({
                            "session_id": session_id,
                            "agent_id": agent_id,
                            "provider_run_id": provider_run_id,
                            "error": error.to_string(),
                        }),
                    );
                }
            }
        }
        Ok(true)
    }

    async fn observe_git_before_prompt_dispatch(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
        provider_run: &crate::provider::RuntimeProviderRun,
    ) {
        let Some(worktree_path) = provider_run.working_directory().cloned() else {
            return;
        };
        let context = crate::git_observer::GitTurnContext {
            session_id: dispatch.session_id.clone(),
            agent_id: dispatch.agent_id.clone(),
            provider: provider_run.provider().to_string(),
            model: provider_run.model().to_string(),
            provider_run_id: dispatch.provider_run_id.clone(),
            provider_session_id: provider_run.provider_session_id().map(str::to_string),
            prompt_id: dispatch.prompt_id.clone(),
            turn_id: dispatch.prompt_id.clone(),
            worktree_path,
            machine_id: None,
            prompt_summary: crate::prompt_transcript::render_prompt_transcript(
                &dispatch.prompt,
                &dispatch.attachments,
            ),
        };
        match tokio::task::spawn_blocking(move || {
            crate::git_observer::capture_turn_snapshot(context)
        })
        .await
        {
            Ok(Some(snapshot)) => {
                self.owned.git_turn_snapshots.insert(snapshot);
            }
            Ok(None) => {}
            Err(error) => crate::logging::warn_with_fields(
                "daemon.git_observer",
                "failed to join pre-turn git snapshot task",
                serde_json::json!({
                    "session_id": dispatch.session_id,
                    "agent_id": dispatch.agent_id,
                    "provider_run_id": dispatch.provider_run_id,
                    "prompt_id": dispatch.prompt_id,
                    "error": error.to_string(),
                }),
            ),
        }
    }

    async fn observe_git_after_prompt_completion(
        &self,
        provider_run_id: &str,
        completed_prompt: &crate::session::PromptQueueItem,
    ) {
        let Some(before) = self
            .owned
            .git_turn_snapshots
            .remove(provider_run_id, completed_prompt.id())
        else {
            return;
        };
        let candidates = self.owned.git_turn_snapshots.candidates_for(&before);
        let after_context = crate::git_observer::GitTurnContext {
            session_id: before.session_id.clone(),
            agent_id: before.agent_id.clone(),
            provider: before.provider.clone(),
            model: before.model.clone(),
            provider_run_id: before.provider_run_id.clone(),
            provider_session_id: before.provider_session_id.clone(),
            prompt_id: before.prompt_id.clone(),
            turn_id: before.turn_id.clone(),
            worktree_path: std::path::PathBuf::from(before.worktree_path.clone()),
            machine_id: before.machine_id.clone(),
            prompt_summary: before.prompt_summary.clone(),
        };
        let history = self.owned.operational_history_store.clone();
        let observation = tokio::task::spawn_blocking(move || {
            let after = crate::git_observer::capture_turn_snapshot(after_context);
            after.map(|after| {
                crate::git_observer::observe_after_turn(before, after, candidates, &history)
            })
        })
        .await;
        match observation {
            Ok(Some(Ok(events))) => {
                if !events.is_empty() {
                    crate::logging::info_with_fields(
                        "daemon.git_observer",
                        "recorded git history events after agent turn",
                        serde_json::json!({
                            "provider_run_id": provider_run_id,
                            "prompt_id": completed_prompt.id(),
                            "event_count": events.len(),
                        }),
                    );
                }
            }
            Ok(Some(Err(error))) => crate::logging::warn_with_fields(
                "daemon.git_observer",
                "failed to record git history events after agent turn",
                serde_json::json!({
                    "provider_run_id": provider_run_id,
                    "prompt_id": completed_prompt.id(),
                    "error": error.to_string(),
                }),
            ),
            Ok(None) => {}
            Err(error) => crate::logging::warn_with_fields(
                "daemon.git_observer",
                "failed to join post-turn git observation task",
                serde_json::json!({
                    "provider_run_id": provider_run_id,
                    "prompt_id": completed_prompt.id(),
                    "error": error.to_string(),
                }),
            ),
        }
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
            self.observe_git_before_prompt_dispatch(dispatch, &provider_run)
                .await;
            owned.note_prompt_started(&dispatch.provider_run_id);
            let prompt_with_handoff = owned.prompt_with_pending_context_handoff(
                &dispatch.session_id,
                &dispatch.agent_id,
                &dispatch.source_attachment_id,
                &dispatch.prompt,
            );
            let provider_prompt = owned.apply_granted_skill_summary(
                &dispatch.session_id,
                &dispatch.agent_id,
                &prompt_with_handoff,
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
        let prompt_with_handoff = owned.prompt_with_pending_context_handoff(
            &dispatch.session_id,
            &dispatch.agent_id,
            &dispatch.source_attachment_id,
            &dispatch.prompt,
        );
        let provider_prompt = owned.apply_granted_skill_summary(
            &dispatch.session_id,
            &dispatch.agent_id,
            &prompt_with_handoff,
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
        mut dispatch: crate::app::KernelRemotePromptDispatch,
    ) {
        let state = self.clone();
        tokio::spawn(async move {
            crate::logging::info_with_fields(
                "daemon.remote_prompt_dispatch",
                "remote prompt dispatch starting",
                serde_json::json!({
                    "session_id": dispatch.session_id,
                    "agent_id": dispatch.agent_id,
                    "worker_kernel_id": dispatch.worker_kernel_id,
                    "leased_agent_id": dispatch.leased_agent_id,
                    "source_attachment_id": dispatch.source_attachment_id,
                }),
            );
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
            let attachments = match serialized_attachments {
                Ok(attachments) => attachments,
                Err(error) => {
                    let _ = state
                        .finish_remote_prompt_dispatch(dispatch, Err(error))
                        .await;
                    return;
                }
            };
            let result = match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                    &config,
                    ClientTarget {
                        daemon_id: Some(dispatch.worker_kernel_id.clone()),
                        daemon_alias: None,
                    },
                    RelayPeerRequest::SubmitLeasedPrompt {
                        leased_agent_id: dispatch.leased_agent_id.clone(),
                        prompt: prompt.clone(),
                        attachments: attachments.clone(),
                        workflow_context: dispatch.workflow_context.clone(),
                        git_context: Some(remote_git_turn_context(&dispatch)),
                        required_mcps: required_mcps.clone(),
                    },
                ),
            )
            .await
            {
                Ok(response) => match response {
                    Ok(RelayPeerResponse::LeasedPromptSubmitted {
                        provider_run_id, ..
                    }) => Ok(provider_run_id),
                    Ok(other) => Err(DaemonError::LocalTransport {
                        operation: "submit remote prepared prompt",
                        message: format!("unexpected remote prompt response: {other:?}"),
                    }),
                    Err(error) => Err(error),
                },
                Err(_) => Err(DaemonError::LocalTransport {
                    operation: "submit remote prepared prompt",
                    message: "remote prompt dispatch timed out waiting for worker response"
                        .to_string(),
                }),
            };
            let result = if remote_prompt_dispatch_should_refresh_binding(&result) {
                crate::logging::warn_with_fields(
                    "daemon.remote_prompt_dispatch",
                    "remote prompt lease stale; refreshing binding",
                    serde_json::json!({
                        "session_id": dispatch.session_id,
                        "agent_id": dispatch.agent_id,
                        "worker_kernel_id": dispatch.worker_kernel_id,
                        "leased_agent_id": dispatch.leased_agent_id,
                    }),
                );
                match state
                    .with_app_side_effect(|app| {
                        app.refresh_remote_agent_binding(&dispatch.agent_id)
                    })
                    .await
                {
                    Ok(agent) => match agent.remote_execution().cloned() {
                        Some(remote_execution) => {
                            dispatch.worker_kernel_id = remote_execution.worker_kernel_id;
                            dispatch.leased_agent_id = remote_execution.leased_agent_id;
                            match tokio::time::timeout(
                                std::time::Duration::from_secs(5),
                                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                                &config,
                                ClientTarget {
                                    daemon_id: Some(dispatch.worker_kernel_id.clone()),
                                    daemon_alias: None,
                                },
                                RelayPeerRequest::SubmitLeasedPrompt {
                                    leased_agent_id: dispatch.leased_agent_id.clone(),
                                    prompt,
                                    attachments,
                                    workflow_context: dispatch.workflow_context.clone(),
                                    git_context: Some(remote_git_turn_context(&dispatch)),
                                    required_mcps,
                                },
                            ))
                            .await
                            {
                                Ok(Ok(RelayPeerResponse::LeasedPromptSubmitted {
                                    provider_run_id, ..
                                })) => Ok(provider_run_id),
                                Ok(Ok(other)) => Err(DaemonError::LocalTransport {
                                    operation: "submit remote prepared prompt",
                                    message: format!("unexpected remote prompt response after binding refresh: {other:?}"),
                                }),
                                Ok(Err(error)) => Err(error),
                                Err(_) => Err(DaemonError::LocalTransport {
                                    operation: "submit remote prepared prompt",
                                    message: "remote prompt dispatch timed out after binding refresh".to_string(),
                                }),
                            }
                        }
                        None => Err(DaemonError::LocalTransport {
                            operation: "refresh remote prompt binding",
                            message: format!(
                                "agent `{}` did not have remote execution after binding refresh",
                                dispatch.agent_id
                            ),
                        }),
                    },
                    Err(error) => Err(error),
                }
            } else {
                result
            };
            match &result {
                Ok(provider_run_id) => crate::logging::info_with_fields(
                    "daemon.remote_prompt_dispatch",
                    "remote prompt dispatch submitted",
                    serde_json::json!({
                        "session_id": dispatch.session_id,
                        "agent_id": dispatch.agent_id,
                        "worker_kernel_id": dispatch.worker_kernel_id,
                        "leased_agent_id": dispatch.leased_agent_id,
                        "remote_provider_run_id": provider_run_id,
                    }),
                ),
                Err(error) => crate::logging::warn_with_fields(
                    "daemon.remote_prompt_dispatch",
                    "remote prompt dispatch failed",
                    serde_json::json!({
                        "session_id": dispatch.session_id,
                        "agent_id": dispatch.agent_id,
                        "worker_kernel_id": dispatch.worker_kernel_id,
                        "leased_agent_id": dispatch.leased_agent_id,
                        "error": error.to_string(),
                    }),
                ),
            }
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

fn remote_prompt_dispatch_should_refresh_binding(result: &Result<String, DaemonError>) -> bool {
    let Err(error) = result else {
        return false;
    };
    match error {
        DaemonError::LeasedAgentNotFound { .. } | DaemonError::ExecutionLeaseNotFound { .. } => {
            true
        }
        DaemonError::LocalTransport { message, .. } => {
            message.contains("leased agent") && message.contains("was not found")
                || message.contains("execution lease") && message.contains("was not found")
                || message.contains("leased_agent_not_found")
                || message.contains("execution_lease_not_found")
                || message.contains("timed out waiting for worker response")
        }
        _ => false,
    }
}

fn remote_prompt_completion_should_treat_as_settled(error: &DaemonError) -> bool {
    match error {
        DaemonError::NoActivePrompt { .. } => true,
        DaemonError::LocalTransport { message, .. } => {
            message.contains("no active prompt")
                || message.contains("NoActivePrompt")
                || message.contains("no_active_prompt")
        }
        _ => false,
    }
}

fn remote_git_turn_context(
    dispatch: &crate::app::KernelRemotePromptDispatch,
) -> crate::transport::relay_peer::RemoteGitTurnContext {
    crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: dispatch.session_id.clone(),
        home_agent_id: dispatch.agent_id.clone(),
        home_prompt_id: dispatch.prompt_id.clone(),
        home_turn_id: dispatch.prompt_id.clone(),
        prompt_summary: crate::prompt_transcript::render_prompt_transcript(
            &dispatch.prompt,
            &dispatch.attachments,
        ),
    }
}

fn remote_git_turn_context_for_prompt(
    session_id: &str,
    agent_id: &str,
    prompt: &crate::session::PromptQueueItem,
) -> crate::transport::relay_peer::RemoteGitTurnContext {
    crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: session_id.to_string(),
        home_agent_id: agent_id.to_string(),
        home_prompt_id: prompt.id().to_string(),
        home_turn_id: prompt.id().to_string(),
        prompt_summary: crate::prompt_transcript::render_prompt_transcript(
            prompt.prompt(),
            prompt.attachments(),
        ),
    }
}

enum PromptAbortDispatchOutcome {
    Done,
    Retry,
}
