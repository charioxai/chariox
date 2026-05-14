use super::*;

impl KernelRuntimeState {
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
        if let Some(endpoint) = request.structured_endpoint {
            launch_request = launch_request.with_structured_endpoint(endpoint);
        }
        if request.native_tui {
            launch_request = launch_request
                .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
        }
        if let Some(provider_session_id) = request.provider_session_id {
            if launch_request.adapter_key == "codex" {
                launch_request = launch_request.with_resume_state(
                    crate::provider::ProviderResumeState::from_codex_thread_id(provider_session_id),
                );
            } else if launch_request.adapter_key == "opencode" {
                launch_request = launch_request.with_resume_state(
                    crate::provider::ProviderResumeState::from_opencode_session_id(
                        provider_session_id,
                    ),
                );
            } else if launch_request.adapter_key == "claude" {
                launch_request = launch_request.with_resume_state(
                    crate::provider::ProviderResumeState::from_claude_session_id(
                        provider_session_id,
                    ),
                );
            }
        }
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
                let effective_config = session
                    .as_ref()
                    .map(|session| {
                        crate::session::effective_agent_execution_config(session, Some(&agent))
                    })
                    .unwrap_or_default();
                launch_request
                    .with_agent_id(agent_id)
                    .with_owner_user_id(agent.owner_user_id().to_string())
                    .with_execution_mode(effective_config.mode)
                    .with_permission_level(effective_config.permission_level)
            } else {
                launch_request.with_agent_id(agent_id)
            };
        } else {
            let session = self
                .owned
                .session_store
                .get_session(&request.session_id)
                .ok();
            let effective_config = session
                .as_ref()
                .map(|session| crate::session::effective_agent_execution_config(session, None))
                .unwrap_or_default();
            launch_request = launch_request
                .with_execution_mode(effective_config.mode)
                .with_permission_level(effective_config.permission_level);
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
        let mut durable_agent_update = None;
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
            if let Some(agent) = clear_failed_codex_resume_state_for_runtime(owned, started, error)
            {
                durable_agent_update = Some(agent);
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
        if let Some(agent) = durable_agent_update.as_ref() {
            if let Err(error) = self
                .append_agent_durable_event("agent.runtime_profile_updated", agent, None)
                .await
            {
                crate::logging::warn_with_fields(
                    "daemon.provider",
                    "failed to persist cleared Codex resume state",
                    serde_json::json!({
                        "session_id": started.run.session_id(),
                        "agent_id": agent.id(),
                        "provider_run_id": started.run.id(),
                        "error": error.to_string(),
                    }),
                );
            }
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
}

fn clear_failed_codex_resume_state_for_runtime(
    owned: &KernelRuntimeOwnedState,
    started: &crate::app::StartedProviderLaunch,
    error: &DaemonError,
) -> Option<crate::agent::AgentInstance> {
    let replacement_resume_state =
        crate::app::failed_codex_resume_state_replacement(&started.run, error)?;
    let agent_id = started.run.agent_instance_id()?;
    let stale_thread_id = started.run.resume_state().codex_thread_id()?.to_string();
    let current = owned.agent_store.get_agent(agent_id).ok()?;
    if current.provider_resume_state().codex_thread_id() != Some(stale_thread_id.as_str()) {
        return None;
    }
    let agent = owned
        .agent_store
        .set_agent_runtime_profile(
            agent_id,
            started.run.provider(),
            Some(started.run.model().to_string()),
            started.run.variant().map(str::to_string),
            replacement_resume_state,
        )
        .ok()?;
    owned.record_notice(
        started.run.session_id(),
        Some(started.run.id()),
        owned
            .attachment_store
            .list_session_attachment_ids(started.run.session_id()),
        format!(
            "Codex resume thread `{stale_thread_id}` is no longer available. Arroba cleared it from the agent profile so the next prompt can start a new durable Codex thread."
        ),
    );
    Some(agent)
}
