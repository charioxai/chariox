use super::*;

impl KernelRuntimeState {
    pub(crate) async fn start_provider_launch(
        &self,
        request: crate::local::LaunchProviderRunRequest,
        caller_user_id: String,
    ) -> Result<(crate::app::StartedProviderLaunch, u64), DaemonError> {
        let launch_request = self
            .owned
            .launch_provider_request_from_local_request(request);
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
}
