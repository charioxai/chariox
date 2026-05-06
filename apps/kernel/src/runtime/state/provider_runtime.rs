//! Provider launch, structured-runtime binding, output pumping, and cancellation orchestration.
//!
//! These methods bridge owned runtime state with provider processes/endpoints and translate
//! provider runtime events back into prompt/session mutations.

use super::*;

impl KernelRuntimeState {
    pub(super) fn activate_agent_mcp_grants_if_idle(
        &self,
        session_id: &str,
        agent_id: &str,
        requested_mcp_name: &str,
    ) -> Result<bool, DaemonError> {
        let reason = format!("MCP `{requested_mcp_name}`");
        Ok(matches!(
            self.reload_agent_provider_if_idle(session_id, agent_id, &reason)?,
            ProviderReloadOutcome::Reloaded
        ))
    }

    pub(super) fn remember_pending_mcp_continuation(
        &self,
        session_id: &str,
        agent_id: &str,
        source_attachment_id: &str,
        mcp_name: &str,
        previous_prompt: &str,
    ) {
        self.owned.pending_mcp_continuations.write().insert(
            agent_id.to_string(),
            PendingMcpContinuation {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
                source_attachment_id: source_attachment_id.to_string(),
                mcp_name: mcp_name.to_string(),
                previous_prompt: previous_prompt.to_string(),
            },
        );
        let state = self.clone();
        let session_id = session_id.to_string();
        let agent_id = agent_id.to_string();
        tokio::spawn(async move {
            for _ in 0..240 {
                let is_idle = state
                    .owned
                    .session_store
                    .get_session(&session_id)
                    .ok()
                    .is_some_and(|session| {
                        state
                            .owned
                            .prompt_state_owner
                            .active_prompt_for_agent(&session, &agent_id)
                            .is_none()
                    });
                if is_idle {
                    if let Err(error) = state
                        .run_pending_mcp_continuation_after_completion(&session_id, &agent_id)
                        .await
                    {
                        crate::logging::warn_with_fields(
                            "daemon.provider",
                            "pending MCP continuation failed",
                            serde_json::json!({
                                "session_id": session_id,
                                "agent_id": agent_id,
                                "error": error.to_string(),
                            }),
                        );
                    }
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        });
    }

    async fn take_pending_mcp_continuation_after_completion(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<PendingMcpContinuation> {
        let mut pending = self.owned.pending_mcp_continuations.write();
        let continuation = pending.get(agent_id)?;
        if continuation.session_id != session_id {
            return None;
        }
        pending.remove(agent_id)
    }

    async fn run_pending_mcp_continuation_after_completion(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let Some(continuation) = self
            .take_pending_mcp_continuation_after_completion(session_id, agent_id)
            .await
        else {
            return Ok(());
        };
        let previous_provider_run_id = self
            .owned
            .provider_store
            .get_run_for_agent(&continuation.session_id, &continuation.agent_id)
            .and_then(|run| {
                if run.adapter_key() == "opencode" {
                    None
                } else {
                    Some(run.id().to_string())
                }
            });
        self.activate_agent_mcp_grants_if_idle(
            &continuation.session_id,
            &continuation.agent_id,
            &continuation.mcp_name,
        )?;
        self.wait_for_agent_provider_relaunch(
            &continuation.session_id,
            &continuation.agent_id,
            previous_provider_run_id.as_deref(),
        )
        .await?;

        let prompt = crate::session::PromptQueueItem::new(
            self.owned.session_store.reserve_prompt_id(),
            &continuation.source_attachment_id,
            &continuation.agent_id,
            format!(
                "MCP `{}` is now loaded. Continue this request exactly:\n\n{}\n\nUse the newly available provider-native MCP tool if requested, then complete any required Arroba managed-I/O file write before replying.",
                continuation.mcp_name, continuation.previous_prompt
            ),
            crate::session::PromptStatus::Queued,
        );
        let mut submission = self
            .submit_prepared_prompt(crate::app::KernelPreparedPromptSubmission {
                session_id: continuation.session_id,
                prompt,
                force_queue: false,
            })
            .await?;
        if let Some(dispatch) = submission.dispatch.take() {
            self.spawn_prompt_dispatch(dispatch, self.owned.provider_store.run_operation_lanes());
        }
        if let Some(dispatch) = submission.remote_dispatch.take() {
            self.spawn_remote_prompt_dispatch(dispatch);
        }
        Ok(())
    }

    async fn wait_for_agent_provider_relaunch(
        &self,
        session_id: &str,
        agent_id: &str,
        previous_provider_run_id: Option<&str>,
    ) -> Result<(), DaemonError> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let ready = self
                .owned
                .provider_store
                .get_run_for_agent(session_id, agent_id)
                .is_some_and(|run| {
                    run.state() == crate::provider::ProviderRunState::Running
                        && previous_provider_run_id.is_none_or(|previous| run.id() != previous)
                });
            if ready {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(DaemonError::LocalTransport {
                    operation: "wait_for_mcp_provider_relaunch",
                    message: format!(
                        "timed out waiting for provider relaunch for agent `{agent_id}`"
                    ),
                });
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }

    pub(crate) async fn start_provider_launch(
        &self,
        request: crate::local::LaunchProviderRunRequest,
        caller_user_id: String,
    ) -> Result<(crate::app::StartedProviderLaunch, u64), DaemonError> {
        let launch_request = self.launch_provider_request_from_owned_state(request);
        {
            let owned = &self.owned;
            if launch_request.owner_user_id != caller_user_id {
                return Err(DaemonError::OwnershipAccessDenied {
                    user_id: caller_user_id,
                    owner_user_id: launch_request.owner_user_id.clone(),
                    resource: format!(
                        "provider run for agent `{}`",
                        launch_request.agent_id.as_deref().unwrap_or("<focused>")
                    ),
                    operation: "launch provider run",
                });
            }
            let config = owned.config_projection.snapshot();
            let launch_request =
                owned.prepare_provider_launch_request(launch_request, config.runtime_mcp_url())?;
            crate::logging::info_with_fields(
                "daemon.app",
                "launching provider run",
                serde_json::json!({
                    "adapter_key": launch_request.adapter_key.clone(),
                    "agent_id": launch_request.agent_id.clone(),
                    "provider": launch_request.provider.clone(),
                    "session_id": launch_request.session_id.clone(),
                }),
            );
            let started = owned.start_provider_launch(launch_request)?;
            let run = started.run.clone();
            if let Some(previous_active_run_id) = started.previous_active_run_id.as_deref() {
                if let Ok(previous_run) = owned.provider_store.get_run(previous_active_run_id) {
                    owned.provider_run_projection.update(previous_run);
                }
            }
            crate::logging::info_with_fields(
                "daemon.app",
                "prepared provider run endpoint metadata",
                serde_json::json!({
                    "provider_run_id": run.id(),
                    "endpoint_mode": run.endpoint_mode().to_string(),
                    "session_id": run.session_id(),
                    "provider": run.provider(),
                }),
            );
            if let Err(error) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).spawn_for_launch(&run)
                })
                .await
            {
                crate::logging::error_with_fields(
                    "daemon.app",
                    "PTY spawn failed for provider run",
                    serde_json::json!({
                        "provider_run_id": run.id(),
                        "session_id": run.session_id(),
                        "error": error.to_string(),
                    }),
                );
                if let Ok(outcome) = owned
                    .provider_store
                    .terminate_run_provider_only(run.session_id(), run.id())
                {
                    let _ = owned.clear_active_provider_run_session_pointer(
                        run.session_id(),
                        outcome.run().id(),
                    );
                    owned.provider_run_projection.update(outcome.into_run());
                }
                if let Some(previous_active_run_id) = started.previous_active_run_id.as_deref() {
                    let recipients = owned
                        .attachment_store
                        .list_session_attachment_ids(run.session_id());
                    match owned
                        .resume_provider_run_for_session(run.session_id(), previous_active_run_id)
                    {
                        Ok(resumed_run) => {
                            owned.record_notice(
                                run.session_id(),
                                Some(resumed_run.id()),
                                recipients,
                                format!(
                                    "Provider switch failed for session `{}`. Arroba resumed the previous provider run `{}` automatically.",
                                    run.session_id(),
                                    resumed_run.id()
                                ),
                            );
                        }
                        Err(resume_error) => {
                            owned.record_notice(
                                run.session_id(),
                                None,
                                recipients,
                                format!(
                                    "Provider switch failed for session `{}` and Arroba could not resume the previous provider run: {}",
                                    run.session_id(),
                                    resume_error
                                ),
                            );
                        }
                    }
                }
                return Err(error);
            }
            owned.provider_run_projection.update(run);
            Ok((started, config.provider_runtime_init_delay_ms))
        }
    }

    pub(super) fn launch_provider_request_from_owned_state(
        &self,
        request: crate::local::LaunchProviderRunRequest,
    ) -> crate::provider::LaunchProviderRequest {
        let mut launch_request = crate::provider::LaunchProviderRequest::new(
            request.session_id.clone(),
            request.adapter_key,
            request.provider,
            request.account_profile,
            request.model,
        )
        .with_variant(request.variant);
        let config = self.owned.config_projection.snapshot();
        if crate::provider::provider_requires_managed_io_by_default(
            &launch_request.provider,
            &config,
        ) {
            launch_request = launch_request.with_managed_io_required();
        }
        if let Some(agent_id) = request.agent_id.clone().or_else(|| {
            self.owned
                .session_store
                .get_session(&request.session_id)
                .ok()
                .and_then(|session| session.focused_agent_id().map(str::to_string))
                .or_else(|| {
                    self.owned
                        .agent_store
                        .get_focused_agent(&request.session_id)
                        .map(|agent| agent.id().to_string())
                })
        }) {
            launch_request = if let Ok(agent) = self.owned.agent_store.get_agent(&agent_id) {
                let session = self
                    .owned
                    .session_store
                    .get_session(&request.session_id)
                    .ok();
                let execution_mode = agent.execution_mode_override().or_else(|| {
                    session
                        .as_ref()
                        .and_then(|session| session.config_state().values().get("agents.mode"))
                        .and_then(|value| crate::provider::AgentExecutionMode::parse(value))
                });
                let permission_level = agent.permission_level_override().or_else(|| {
                    session
                        .as_ref()
                        .and_then(|session| {
                            session.config_state().values().get("agents.permissions")
                        })
                        .and_then(|value| crate::provider::AgentPermissionLevel::parse(value))
                });
                launch_request
                    .with_agent_id(agent_id)
                    .with_owner_user_id(agent.owner_user_id().to_string())
                    .with_execution_mode(execution_mode.unwrap_or_default())
                    .with_permission_level(permission_level.unwrap_or_default())
            } else {
                launch_request.with_agent_id(agent_id)
            };
        } else {
            let session = self
                .owned
                .session_store
                .get_session(&request.session_id)
                .ok();
            let execution_mode = session
                .as_ref()
                .and_then(|session| session.config_state().values().get("agents.mode"))
                .and_then(|value| crate::provider::AgentExecutionMode::parse(value))
                .unwrap_or_default();
            let permission_level = session
                .as_ref()
                .and_then(|session| session.config_state().values().get("agents.permissions"))
                .and_then(|value| crate::provider::AgentPermissionLevel::parse(value))
                .unwrap_or_default();
            launch_request = launch_request
                .with_execution_mode(execution_mode)
                .with_permission_level(permission_level);
        }
        launch_request
    }

    pub(crate) async fn finish_provider_launch(
        &self,
        started: &crate::app::StartedProviderLaunch,
        binding: Option<crate::provider::ProviderRuntimeBinding>,
    ) {
        let mut durable_agent_update = None;
        {
            let owned = &self.owned;
            let result = owned.finish_provider_launch_success(started, binding);
            match result {
                Ok(run) => {
                    if let Some(agent_id) = run.agent_instance_id() {
                        durable_agent_update = owned.agent_store.get_agent(agent_id).ok();
                        match owned.advance_next_queued_prompt_dispatch(
                            run.session_id(),
                            agent_id,
                            run.id(),
                        ) {
                            Ok(Some(dispatch)) => {
                                if let Err(error) = self.enqueue_prompt_dispatch(&dispatch).await {
                                    let _ = self.fail_prompt_dispatch(dispatch, error).await;
                                }
                            }
                            Ok(None) => {}
                            Err(error) => {
                                self.fail_provider_launch(started, &error).await;
                                return;
                            }
                        }
                        let _ = owned.session_snapshot(run.session_id());
                    }
                }
                Err(error) => {
                    self.fail_provider_launch(started, &error).await;
                }
            }
        }
        if let Some(agent) = durable_agent_update {
            if let Err(error) = self
                .append_agent_durable_event("agent.runtime_profile_updated", &agent, None)
                .await
            {
                self.fail_provider_launch(started, &error).await;
            }
        }
    }

    pub(crate) async fn fail_provider_launch(
        &self,
        started: &crate::app::StartedProviderLaunch,
        error: &DaemonError,
    ) {
        {
            let owned = &self.owned;
            crate::logging::error_with_fields(
                "daemon.app",
                "provider runtime initialization failed",
                serde_json::json!({
                    "provider_run_id": started.run.id(),
                    "session_id": started.run.session_id(),
                    "error": error.to_string(),
                }),
            );
            let recipients = owned
                .attachment_store
                .list_session_attachment_ids(started.run.session_id());
            owned.record_notice(
                started.run.session_id(),
                Some(started.run.id()),
                recipients,
                format!(
                    "Provider launch `{}` failed before it became ready: {}",
                    started.run.id(),
                    error
                ),
            );
            let diagnostic = format!(
                "Provider launch `{}` failed before it became ready: {}",
                started.run.id(),
                error
            );
            if let Ok(run) = owned
                .provider_store
                .record_terminal_diagnostic(started.run.id(), diagnostic.clone())
            {
                owned.provider_run_projection.update(run);
            }
            let leased_context = self
                .with_app_side_effect(|app| {
                    crate::app::RemoteLeaseRuntime::new(app)
                        .leased_workflow_turn_context_for_provider_run(started.run.id())
                })
                .await;
            if let Some(context) = leased_context {
                let _ = self
                    .with_app_side_effect(|app| {
                        app.block_on_relay_future(
                            crate::transport::relay_client::send_peer_request_via_temporary_connection(
                                app.config(),
                                arroba_relay::protocol::ClientTarget {
                                    daemon_id: Some(context.home_kernel_id.clone()),
                                    daemon_alias: None,
                                },
                                crate::transport::relay_peer::RelayPeerRequest::ForwardWorkflowProviderFailure {
                                    context,
                                    message: diagnostic.clone(),
                                },
                            ),
                        )
                    })
                    .await;
                let _ = self
                    .with_app_side_effect(|app| {
                        crate::app::RemoteLeaseRuntime::new(app)
                            .complete_leased_workflow_prompt_for_provider_run(started.run.id())
                    })
                    .await;
            } else if let Some(agent_id) = started.run.agent_instance_id() {
                if let Ok(session) = owned.session_store.get_session(started.run.session_id()) {
                    if let Some(active_prompt) = owned
                        .prompt_state_owner
                        .active_prompt_for_agent(&session, agent_id)
                    {
                        if active_prompt.workflow_run_id().is_some() {
                            let _ = owned.workflow_fail_provider_prompt(
                                started.run.session_id(),
                                &active_prompt,
                                Some(started.run.id()),
                                &diagnostic,
                            );
                        }
                        let _ = owned.complete_local_prompt_without_advance(
                            started.run.session_id(),
                            agent_id,
                            Some(started.run.id()),
                        );
                    }
                }
            }
            let (_, process_key) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(started.run.id())
                })
                .await
                .unwrap_or((false, None));
            owned.remove_provider_process_tracking_for_run(started.run.id(), process_key);
            owned.provider_store.clear_runtime(started.run.id());
            if let Ok(outcome) = owned
                .provider_store
                .terminate_run_provider_only(started.run.session_id(), started.run.id())
            {
                let _ = owned.clear_active_provider_run_session_pointer(
                    started.run.session_id(),
                    outcome.run().id(),
                );
                owned.provider_run_projection.update(outcome.into_run());
            }
            if let Some(previous_active_run_id) = started.previous_active_run_id.as_deref() {
                let _ = owned.resume_provider_run_for_session(
                    started.run.session_id(),
                    previous_active_run_id,
                );
            }
            let _ = owned.session_snapshot(started.run.session_id());
        }
        if let (Some(agent_id), Some(reason)) = (
            started.run.agent_instance_id(),
            crate::provider::classify_provider_substitutable_failure_text(
                started.run.adapter_key(),
                &error.to_string(),
            ),
        ) {
            if let Err(substitute_error) = self
                .activate_next_agent_substitute_after_failure(
                    started.run.session_id(),
                    agent_id,
                    &reason,
                )
                .await
            {
                crate::logging::warn_with_fields(
                    "daemon.provider",
                    "automatic substitute activation after launch failure failed",
                    serde_json::json!({
                        "session_id": started.run.session_id(),
                        "agent_id": agent_id,
                        "provider_run_id": started.run.id(),
                        "error": substitute_error.to_string(),
                    }),
                );
            }
        }
    }

    pub(super) async fn settle_owned_provider_prompt(
        &self,
        session_id: &str,
        provider_run_id: &str,
        prompt_completed: bool,
        force: bool,
    ) -> Result<crate::app::ProviderRunExitSessionSummary, DaemonError> {
        let owned = &self.owned;
        let provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let agent_id = provider_run
            .agent_instance_id()
            .map(str::to_string)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "provider run has no agent".to_string(),
            })?;
        let active_prompt = owned
            .prompt_state_owner
            .active_prompt_for_agent(&owned.session_store.get_session(session_id)?, &agent_id);
        let Some(active_prompt) = active_prompt else {
            crate::logging::debug_with_fields(
                "daemon.provider",
                "settle provider prompt found no active prompt",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "prompt_completed": prompt_completed,
                    "force": force,
                }),
            );
            if owned.clear_prompt_activity(provider_run_id) {
                self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
            }
            let _ = owned.sync_focused_provider_run_if_idle(session_id);
            let _ = owned.session_snapshot(session_id);
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: false,
                started_next_prompt: false,
            });
        };

        if active_prompt.status() == crate::session::PromptStatus::Cancelling {
            if !force && !prompt_completed && !owned.prompt_should_settle(provider_run_id) {
                return Ok(crate::app::ProviderRunExitSessionSummary {
                    had_active_prompt: true,
                    started_next_prompt: false,
                });
            }
            let cancellation = owned.finalize_local_prompt_cancellation_with_queued_advance(
                session_id,
                &agent_id,
                Some(provider_run_id),
            )?;
            owned.workflow_cancel_prompt(session_id, &cancellation.cancellation.prompt)?;
            if cancellation.released_claim {
                self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
            }
            if let Some(dispatch) = cancellation.dispatch {
                if let Err(error) = self
                    .enqueue_prompt_dispatch_after_liveness(&dispatch, owned)
                    .await
                {
                    let _ = self.fail_prompt_dispatch(dispatch, error).await;
                }
            }
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: true,
                started_next_prompt: cancellation.cancellation.started_next.is_some(),
            });
        }

        if prompt_completed
            && !force
            && active_prompt.workflow_run_id().is_some()
            && active_prompt.workflow_node_run_id().is_some()
            && !owned.workflow_prompt_has_completion_output(
                session_id,
                active_prompt
                    .workflow_run_id()
                    .expect("workflow run id checked"),
                active_prompt
                    .workflow_node_run_id()
                    .expect("workflow node run id checked"),
                provider_run_id,
            )
        {
            owned.note_prompt_settlement_requested(provider_run_id);
            let _ = owned.session_snapshot(session_id);
            crate::logging::debug_with_fields(
                "daemon.provider",
                "provider completed workflow prompt before workflow output",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "prompt_id": active_prompt.id(),
                }),
            );
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: true,
                started_next_prompt: false,
            });
        }

        if !force && !prompt_completed && !owned.prompt_should_settle(provider_run_id) {
            crate::logging::debug_with_fields(
                "daemon.provider",
                "settle provider prompt skipped",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "prompt_completed": prompt_completed,
                    "force": force,
                    "active_prompt_status": active_prompt.status(),
                }),
            );
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: true,
                started_next_prompt: false,
            });
        }
        if !force {
            if let (Some(workflow_run_id), Some(workflow_node_run_id)) = (
                active_prompt.workflow_run_id(),
                active_prompt.workflow_node_run_id(),
            ) {
                if !owned.workflow_prompt_has_completion_output(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                    provider_run_id,
                ) {
                    let message = if prompt_completed {
                        "provider completed workflow turn without a validated workflow output"
                    } else {
                        "provider workflow turn settled without a validated workflow output"
                    };
                    owned.workflow_fail_provider_prompt(
                        session_id,
                        &active_prompt,
                        Some(provider_run_id),
                        message,
                    )?;
                    let _ = owned.complete_local_prompt_without_advance(
                        session_id,
                        &agent_id,
                        Some(provider_run_id),
                    )?;
                    return Ok(crate::app::ProviderRunExitSessionSummary {
                        had_active_prompt: true,
                        started_next_prompt: false,
                    });
                }
            }
        }
        let provider_run_state = provider_run.state();
        let next_queued_prompt = if provider_run_state == crate::provider::ProviderRunState::Running
        {
            owned
                .prompt_state_owner
                .peek_next_queued_prompt(&owned.session_store.get_session(session_id)?, &agent_id)
        } else {
            None
        };
        let completion = if let Some(next_queued_prompt) = next_queued_prompt.as_ref() {
            owned.complete_local_prompt_with_queued_advance(
                session_id,
                &agent_id,
                Some(provider_run_id),
                next_queued_prompt,
            )?
        } else {
            owned.complete_local_prompt_without_advance(
                session_id,
                &agent_id,
                Some(provider_run_id),
            )?
        }
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "settle provider prompt",
            message: "owned prompt runtime could not settle provider prompt".to_string(),
        })?;
        crate::logging::debug_with_fields(
            "daemon.provider",
            "settled provider prompt",
            serde_json::json!({
                "session_id": session_id,
                "provider_run_id": provider_run_id,
                "agent_id": agent_id,
                "prompt_completed": prompt_completed,
                "force": force,
                "started_next": completion.completion.started_next.is_some(),
                "released_claim": completion.released_claim,
            }),
        );
        if completion.completion.completed.workflow_run_id().is_some() {
            let dispatches = owned.workflow_complete_prompt(
                session_id,
                &completion.completion.completed,
                Some(provider_run_id),
            )?;
            self.spawn_workflow_prompt_dispatches(dispatches);
        }
        if let Some(started_next) = completion.completion.started_next.as_ref() {
            if crate::scheduler::runtime::is_workflow_prompt_attachment(
                started_next.source_attachment_id(),
            ) {
                owned.workflow_start_prompt(session_id, started_next)?;
            }
        }
        if completion.released_claim && completion.completion.completed.workflow_run_id().is_none()
        {
            self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
        }
        if let Some(dispatch) = completion.dispatch {
            if let Err(error) = self
                .enqueue_prompt_dispatch_after_liveness(&dispatch, owned)
                .await
            {
                let _ = self.fail_prompt_dispatch(dispatch, error).await;
            }
        }
        let state = self.clone();
        let session_id_for_continuation = session_id.to_string();
        let agent_id_for_continuation = agent_id.clone();
        let continuation: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> =
            Box::pin(async move {
                if let Err(error) = state
                    .run_pending_mcp_continuation_after_completion(
                        &session_id_for_continuation,
                        &agent_id_for_continuation,
                    )
                    .await
                {
                    crate::logging::warn_with_fields(
                        "daemon.provider",
                        "pending MCP continuation failed",
                        serde_json::json!({
                            "session_id": session_id_for_continuation,
                            "agent_id": agent_id_for_continuation,
                            "error": error.to_string(),
                        }),
                    );
                }
            });
        tokio::spawn(continuation);
        Ok(crate::app::ProviderRunExitSessionSummary {
            had_active_prompt: true,
            started_next_prompt: completion.completion.started_next.is_some(),
        })
    }

    async fn fail_owned_provider_prompt(
        &self,
        session_id: &str,
        provider_run_id: &str,
        message: &str,
    ) -> Result<(), DaemonError> {
        let owned = &self.owned;
        let provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let agent_id = provider_run
            .agent_instance_id()
            .map(str::to_string)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "provider run has no agent".to_string(),
            })?;

        let leased_context = self
            .with_app_side_effect(|app| {
                crate::app::RemoteLeaseRuntime::new(app)
                    .leased_workflow_turn_context_for_provider_run(provider_run_id)
            })
            .await;
        if let Some(context) = leased_context {
            let response = self
                .with_app_side_effect(|app| {
                    app.block_on_relay_future(
                        crate::transport::relay_client::send_peer_request_via_temporary_connection(
                            app.config(),
                            arroba_relay::protocol::ClientTarget {
                                daemon_id: Some(context.home_kernel_id.clone()),
                                daemon_alias: None,
                            },
                            crate::transport::relay_peer::RelayPeerRequest::ForwardWorkflowProviderFailure {
                                context,
                                message: message.to_string(),
                            },
                        ),
                    )
                })
                .await?;
            if !matches!(
                response,
                crate::transport::relay_peer::RelayPeerResponse::WorkflowProviderFailureHandled
            ) {
                return Err(DaemonError::LocalTransport {
                    operation: "forward workflow provider failure",
                    message: format!("unexpected workflow provider failure response: {response:?}"),
                });
            }
            let _ = self
                .with_app_side_effect(|app| {
                    crate::app::RemoteLeaseRuntime::new(app)
                        .complete_leased_workflow_prompt_for_provider_run(provider_run_id)
                })
                .await?;
            return Ok(());
        }

        let session = owned.session_store.get_session(session_id)?;
        let Some(active_prompt) = owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
        else {
            return Ok(());
        };
        if active_prompt.workflow_run_id().is_some() {
            owned.workflow_fail_provider_prompt(
                session_id,
                &active_prompt,
                Some(provider_run_id),
                message,
            )?;
        }
        let completion = owned.complete_local_prompt_without_advance(
            session_id,
            &agent_id,
            Some(provider_run_id),
        )?;
        if completion
            .as_ref()
            .is_some_and(|completion| completion.released_claim)
            && active_prompt.workflow_run_id().is_none()
        {
            self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
        }
        if let Some(reason) = crate::provider::classify_provider_substitutable_failure_text(
            provider_run.adapter_key(),
            message,
        ) {
            if let Err(error) = self
                .activate_next_agent_substitute_after_failure(session_id, &agent_id, &reason)
                .await
            {
                crate::logging::warn_with_fields(
                    "daemon.provider",
                    "automatic substitute activation after provider failure failed",
                    serde_json::json!({
                        "session_id": session_id,
                        "agent_id": agent_id,
                        "provider_run_id": provider_run_id,
                        "error": error.to_string(),
                    }),
                );
            }
        }
        Ok(())
    }

    pub(super) async fn pump_owned_provider_output(
        &self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
        initial_liveness_already_checked: bool,
    ) -> Result<Vec<crate::terminal::TerminalOutputRecord>, DaemonError> {
        let owned = &self.owned;
        owned.reap_structured_prompt_jobs();
        if !initial_liveness_already_checked
            && self
                .reconcile_provider_run_exit(session_id, provider_run_id)
                .await?
        {
            return Ok(Vec::new());
        }
        let mut provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        if provider_run.state() == crate::provider::ProviderRunState::Ended {
            return Ok(Vec::new());
        }
        if provider_run.state() == crate::provider::ProviderRunState::Parked {
            if !owned.provider_run_has_active_prompt(session_id, &provider_run)? {
                return Ok(Vec::new());
            }
            provider_run = owned.provider_store.resume_run_detached(provider_run_id)?;
            owned.provider_run_projection.update(provider_run.clone());
            crate::logging::warn_with_fields(
                "daemon.provider",
                "resumed parked provider run that still had an active prompt",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": provider_run.agent_instance_id(),
                }),
            );
        }

        if owned
            .provider_store
            .run_uses_structured_prompt_io(&provider_run)
        {
            return self
                .pump_owned_structured_provider_output(
                    session_id,
                    provider_run_id,
                    recipient_attachment_ids,
                )
                .await;
        }

        let chunks = match self
            .with_app_side_effect(|app| app.drain_provider_pty_output_for_runtime(provider_run_id))
            .await
        {
            Ok(chunks) => chunks,
            Err(error) => {
                if self
                    .reconcile_provider_run_exit(session_id, provider_run_id)
                    .await?
                {
                    return Ok(Vec::new());
                }
                return Err(error);
            }
        };
        if !chunks.is_empty() {
            owned.note_prompt_response_content(provider_run_id);
        }
        let terminal_failure = crate::provider::classify_provider_terminal_failure_text(
            provider_run.adapter_key(),
            &chunks
                .iter()
                .map(|chunk| String::from_utf8_lossy(&chunk.bytes))
                .collect::<String>(),
        );
        let records = chunks
            .into_iter()
            .map(|chunk| {
                owned.fan_out_terminal_output(
                    session_id,
                    provider_run_id,
                    crate::terminal::TerminalOutputKind::ProviderOutput,
                    None,
                    recipient_attachment_ids.clone(),
                    &chunk.bytes,
                )
            })
            .collect::<Vec<_>>();
        if let Some(message) = terminal_failure {
            let run = owned
                .provider_store
                .record_terminal_diagnostic(provider_run_id, message.clone())?;
            owned.provider_run_projection.update(run);
            self.fail_owned_provider_prompt(session_id, provider_run_id, &message)
                .await?;
            return Ok(records);
        }
        if !self
            .reconcile_provider_run_exit(session_id, provider_run_id)
            .await?
        {
            let _ = self
                .settle_owned_provider_prompt(session_id, provider_run_id, false, false)
                .await?;
        }
        Ok(records)
    }

    pub(super) async fn pump_owned_structured_provider_output(
        &self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
    ) -> Result<Vec<crate::terminal::TerminalOutputRecord>, DaemonError> {
        let owned = &self.owned;
        let mut provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        if provider_run.state() == crate::provider::ProviderRunState::Parked {
            if !owned.provider_run_has_active_prompt(session_id, &provider_run)? {
                return Ok(Vec::new());
            }
            provider_run = owned.provider_store.resume_run_detached(provider_run_id)?;
            owned.provider_run_projection.update(provider_run.clone());
        }
        if provider_run.endpoint_mode() != crate::provider::AgentEndpointMode::External {
            if let Err(error) = self
                .with_app_side_effect(|app| {
                    app.drain_provider_pty_output_for_runtime(provider_run_id)
                })
                .await
            {
                if self
                    .reconcile_provider_run_exit(session_id, provider_run_id)
                    .await?
                {
                    return Ok(Vec::new());
                }
                if !matches!(error, DaemonError::PtyProcessNotFound { .. }) {
                    return Err(error);
                }
            }
        }
        let mut records = owned.structured_output_records.take(provider_run_id);
        for finished in owned
            .provider_store
            .drain_finished_structured_output_poll_jobs()
        {
            let finished_run_id = finished.provider_run_id.clone();
            let is_requested_run = finished_run_id == provider_run_id;
            crate::logging::debug_with_fields(
                "daemon.provider",
                "drained finished structured output poll",
                serde_json::json!({
                    "requested_provider_run_id": provider_run_id,
                    "finished_provider_run_id": finished_run_id,
                    "is_requested_run": is_requested_run,
                }),
            );
            let poll_result = match finished.result {
                Ok(Some(poll_result)) => poll_result,
                Ok(None) => continue,
                Err(error) => {
                    let reconcile_result = if is_requested_run {
                        self.reconcile_provider_run_exit(session_id, provider_run_id)
                            .await
                    } else {
                        match owned.provider_store.get_run(&finished_run_id) {
                            Ok(run) => {
                                self.reconcile_provider_run_exit(run.session_id(), &finished_run_id)
                                    .await
                            }
                            Err(run_error) => Err(run_error),
                        }
                    };
                    match reconcile_result {
                        Ok(true) => continue,
                        Ok(false) if is_requested_run => return Err(error),
                        Ok(false) => {
                            crate::logging::error_with_fields(
                                "daemon.app",
                                "background structured output poll failed",
                                serde_json::json!({
                                    "provider_run_id": finished_run_id,
                                    "error": error.to_string(),
                                }),
                            );
                            continue;
                        }
                        Err(reconcile_error) if is_requested_run => return Err(reconcile_error),
                        Err(reconcile_error) => {
                            crate::logging::error_with_fields(
                                "daemon.app",
                                "background structured output poll reconciliation failed",
                                serde_json::json!({
                                    "provider_run_id": finished_run_id,
                                    "error": reconcile_error.to_string(),
                                }),
                            );
                            continue;
                        }
                    }
                }
            };
            let run = match owned.provider_store.get_run(&finished_run_id) {
                Ok(run) => run,
                Err(_) => continue,
            };
            let run_session_id = run.session_id().to_string();
            let recipients = if is_requested_run {
                recipient_attachment_ids.clone()
            } else {
                owned
                    .attachment_store
                    .list_session_attachment_ids(&run_session_id)
            };
            let applied = self
                .apply_owned_structured_output_batch(
                    &run_session_id,
                    &finished_run_id,
                    recipients,
                    poll_result,
                )
                .await?;
            if is_requested_run {
                records.extend(applied);
            } else {
                owned
                    .structured_output_records
                    .append(finished_run_id, applied);
            }
        }
        owned
            .provider_store
            .enqueue_structured_output_poll(provider_run_id)?;
        Ok(records)
    }

    pub(super) async fn apply_owned_structured_output_batch(
        &self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
        poll_result: crate::provider::ProviderPromptSignalBatch,
    ) -> Result<Vec<crate::terminal::TerminalOutputRecord>, DaemonError> {
        let owned = &self.owned;
        crate::logging::debug_with_fields(
            "daemon.provider",
            "applying structured output batch",
            serde_json::json!({
                "session_id": session_id,
                "provider_run_id": provider_run_id,
                "chunks": poll_result.chunks.len(),
                "completions": poll_result.completions.len(),
                "prompt_completed": poll_result.prompt_completed,
                "terminal_failure": poll_result.terminal_failure,
            }),
        );
        owned
            .provider_store
            .apply_structured_output_metadata(provider_run_id, &poll_result)?;
        let provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let adapter_key = provider_run.adapter_key().to_string();
        owned.provider_run_projection.update(provider_run);
        for notice in &poll_result.notices {
            owned.record_notice(
                session_id,
                Some(provider_run_id),
                recipient_attachment_ids.clone(),
                notice.to_string(),
            );
        }
        let saw_response_content = poll_result.chunks.iter().any(|chunk| {
            matches!(
                chunk.kind,
                crate::terminal::TerminalOutputKind::ProviderOutput
                    | crate::terminal::TerminalOutputKind::ProviderReasoning
            )
        });
        let saw_runtime_activity = poll_result.chunks.iter().any(|chunk| {
            matches!(
                chunk.kind,
                crate::terminal::TerminalOutputKind::ProviderOutput
                    | crate::terminal::TerminalOutputKind::ProviderReasoning
                    | crate::terminal::TerminalOutputKind::ProviderTool
                    | crate::terminal::TerminalOutputKind::ProviderStatus
            )
        });
        if saw_response_content {
            owned.note_prompt_response_content(provider_run_id);
        } else if saw_runtime_activity {
            owned.note_prompt_output(provider_run_id);
        }
        for completion in &poll_result.completions {
            owned.record_assistant_message_completion(
                session_id,
                provider_run_id,
                recipient_attachment_ids.clone(),
                &completion.message_id,
                completion.completed_at_ms,
            );
            owned.mark_prompt_completion_recorded(provider_run_id);
        }
        let prompt_completed = poll_result.prompt_completed;
        let terminal_failure = poll_result.terminal_failure.clone().or_else(|| {
            let mut text = poll_result.notices.join("\n");
            text.push('\n');
            text.push_str(
                &poll_result
                    .chunks
                    .iter()
                    .map(|chunk| String::from_utf8_lossy(&chunk.bytes))
                    .collect::<String>(),
            );
            crate::provider::classify_provider_terminal_failure_text(adapter_key.as_str(), &text)
        });
        if let Some(message) = terminal_failure.as_deref() {
            let run = owned
                .provider_store
                .record_terminal_diagnostic(provider_run_id, message.to_string())?;
            owned.provider_run_projection.update(run);
        }
        let records = poll_result
            .chunks
            .into_iter()
            .map(|chunk| {
                owned.fan_out_terminal_output(
                    session_id,
                    provider_run_id,
                    chunk.kind,
                    chunk.merge_key,
                    recipient_attachment_ids.clone(),
                    &chunk.bytes,
                )
            })
            .collect::<Vec<_>>();
        if let Some(message) = terminal_failure {
            self.fail_owned_provider_prompt(session_id, provider_run_id, &message)
                .await?;
            return Ok(records);
        }
        if !self
            .reconcile_provider_run_exit(session_id, provider_run_id)
            .await?
        {
            let _ = self
                .settle_owned_provider_prompt(session_id, provider_run_id, prompt_completed, false)
                .await?;
        }
        Ok(records)
    }

    pub(crate) async fn pump_terminal_output_with_snapshot(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<
        (
            Vec<crate::terminal::TerminalOutputRecord>,
            Option<crate::session::RuntimeSession>,
        ),
        DaemonError,
    > {
        let owned = &self.owned;
        owned.reap_structured_prompt_jobs();
        owned.ensure_attachment_in_session(session_id, attachment_id)?;
        let active_provider_run_id = owned
            .session_store
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_string);
        let mut provider_run_ids = BTreeSet::new();
        if let Some(provider_run_id) = active_provider_run_id {
            provider_run_ids.insert(provider_run_id);
        }
        provider_run_ids.extend(
            owned
                .provider_store
                .list_runs()
                .into_iter()
                .filter(|run| run.session_id() == session_id)
                .filter(|run| {
                    matches!(
                        run.state(),
                        crate::provider::ProviderRunState::Starting
                            | crate::provider::ProviderRunState::Running
                    )
                })
                .map(|run| run.id().to_string()),
        );
        let recipient_attachment_ids = owned
            .attachment_store
            .list_session_attachment_ids(session_id);
        for provider_run_id in provider_run_ids {
            let _ = self
                .pump_owned_provider_output(
                    session_id,
                    &provider_run_id,
                    recipient_attachment_ids.clone(),
                    false,
                )
                .await?;
        }
        let records = owned
            .terminal_stream
            .drain_output_records(session_id, attachment_id);
        let session = owned.session_snapshot(session_id).ok();
        Ok((records, session))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    async fn owned_runtime_state(app: &Arc<Mutex<DaemonApp>>) -> KernelRuntimeState {
        let app_locked = app.lock().await;
        KernelRuntimeState::new_with_owned_state(
            Arc::clone(app),
            app_locked.config_projection_store(),
            app_locked.session_state_store(),
            app_locked.agents().clone(),
            app_locked.attachments().clone(),
            app_locked.providers().clone(),
            app_locked.provider_process_tracking_store(),
            app_locked.session_state_projection_store(),
            app_locked.provider_run_projection_store(),
            app_locked.history_store(),
            app_locked.operational_history_store(),
            app_locked.durable_state_store(),
            app_locked.session_history_projection_store(),
            app_locked.prompt_state_owner(),
            app_locked.prompt_activity_store(),
            app_locked.prompt_idle_timeout(),
            app_locked.prompt_workspace_claim_store(),
            app_locked.structured_output_record_store(),
            app_locked.terminal_stream_store(),
            app_locked.workspace_coordinator(),
        )
    }

    #[tokio::test]
    async fn provider_switch_does_not_park_runs_with_active_prompts() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, first_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-1",
                "worktree-1",
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-1",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let second_agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "codex").with_alias("second"),
            )
            .expect("second agent should spawn");
        let idle_agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "codex").with_alias("idle"),
            )
            .expect("idle agent should spawn");

        let first_run = app
            .launch_provider(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(first_agent.id()),
            )
            .expect("first provider should launch");
        app.update_provider_run_projection(first_run.clone());
        app.submit_prompt(
            session.id(),
            attachment.id(),
            Some(first_agent.id()),
            "first prompt\n",
            Vec::new(),
        )
        .expect("first prompt should start");

        let second_run = app
            .launch_provider(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(second_agent.id()),
            )
            .expect("second provider should launch");
        app.update_provider_run_projection(second_run.clone());

        assert_eq!(
            app.providers
                .get_run(first_run.id())
                .expect("first run should exist")
                .state(),
            crate::provider::ProviderRunState::Running,
            "launching another provider must not park a run that owns an active prompt",
        );

        app.submit_prompt(
            session.id(),
            attachment.id(),
            Some(second_agent.id()),
            "second prompt\n",
            Vec::new(),
        )
        .expect("second prompt should start");
        crate::app::KernelSessionService::new(&mut app)
            .focus_agent(session.id(), idle_agent.id())
            .expect("idle agent focus should succeed");

        assert_eq!(
            app.providers
                .get_run(second_run.id())
                .expect("second run should exist")
                .state(),
            crate::provider::ProviderRunState::Running,
            "focusing an idle agent while multiple prompts are active must not park active work",
        );
        assert_eq!(
            app.sessions
                .get_session(session.id())
                .expect("session should exist")
                .active_provider_run_id(),
            Some(second_run.id()),
            "ambiguous multi-agent prompt work should keep the active provider pointer stable",
        );
    }

    #[tokio::test]
    async fn provider_completed_signal_settles_matching_active_prompt() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-1",
                "worktree-1",
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-1",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let run = app
            .launch_provider(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(agent.id()),
            )
            .expect("provider run should launch");
        app.update_provider_run_projection(run.clone());
        app.submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "status\n",
            Vec::new(),
        )
        .expect("prompt should start");

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let first_settlement = runtime
            .settle_owned_provider_prompt(session.id(), run.id(), true, false)
            .await
            .expect("provider completion signal should be accepted");
        assert!(first_settlement.had_active_prompt);
        assert!(!first_settlement.started_next_prompt);
        assert!(runtime
            .owned
            .session_store
            .get_session(session.id())
            .expect("session should exist")
            .active_prompt_for_agent(agent.id())
            .is_none());
    }
}
