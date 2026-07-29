//! Provider relaunch task orchestration.

use super::*;

impl KernelRuntimeState {
    pub(super) fn spawn_provider_relaunch(
        &self,
        launch_request: crate::provider::LaunchProviderRequest,
        runtime_init_delay_ms: u64,
        terminated_run_id: Option<String>,
        launch_delay_ms: u64,
    ) {
        let state = self.clone();
        tokio::spawn(async move {
            let terminated_run_id_for_policy = terminated_run_id.clone();
            if let Some(terminated_run_id) = terminated_run_id.as_deref() {
                let (_, process_key) = state
                    .with_app_side_effect(|app| {
                        crate::app::ProviderLaunchProcessRuntime::new(app)
                            .remove_run(terminated_run_id)
                    })
                    .await
                    .unwrap_or((false, None));
                state
                    .owned
                    .remove_provider_process_tracking_for_run(terminated_run_id, process_key);
            }
            if launch_delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(launch_delay_ms)).await;
            }
            if let Some(agent_id) = launch_request.agent_id.as_deref() {
                if let Some(current_run) = state
                    .owned
                    .provider_store
                    .get_run_for_agent(&launch_request.session_id, agent_id)
                {
                    let stale_relaunch = terminated_run_id_for_policy
                        .as_deref()
                        .is_none_or(|terminated| current_run.id() != terminated);
                    if stale_relaunch {
                        crate::logging::info_with_fields(
                            "daemon.provider",
                            "skipping stale provider policy relaunch",
                            serde_json::json!({
                                "session_id": launch_request.session_id,
                                "agent_id": agent_id,
                                "current_provider_run_id": current_run.id(),
                                "terminated_provider_run_id": terminated_run_id_for_policy,
                            }),
                        );
                        return;
                    }
                }
            }
            let started = match state.owned.start_provider_launch(launch_request) {
                Ok(started) => started,
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.provider",
                        "provider policy relaunch failed",
                        serde_json::json!({ "error": error.to_string() }),
                    );
                    return;
                }
            };
            if runtime_init_delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(runtime_init_delay_ms)).await;
            }
            let spawn_result = state
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app)
                        .spawn_for_launch(&started.run)
                })
                .await;
            if let Err(error) = spawn_result {
                state.fail_provider_launch(&started, &error).await;
                return;
            }
            state
                .owned
                .provider_run_projection
                .update(started.run.clone());
            let run = started.run.clone();
            let binding = tokio::task::spawn_blocking(move || {
                crate::provider::ProviderProcessService::initialize_runtime_binding(&run)
            })
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "initialize provider runtime",
                message: error.to_string(),
            });
            match binding {
                Ok(Ok(binding)) => {
                    state.finish_provider_launch(&started, binding).await;
                }
                Ok(Err(error)) | Err(error) => {
                    state.fail_provider_launch(&started, &error).await;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[tokio::test]
    async fn delayed_policy_relaunch_aborts_when_agent_has_newer_run() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-1",
                "worktree-1",
            ))
            .expect("session should be created");
        let first = app
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
            .expect("first provider should launch");
        app.update_provider_run_projection(first.clone());

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let ended = runtime
            .owned
            .provider_store
            .terminate_run_provider_only(session.id(), first.id())
            .expect("first provider should terminate");
        runtime
            .owned
            .clear_active_provider_run_session_pointer(session.id(), first.id())
            .expect("active pointer should clear");
        runtime
            .owned
            .provider_run_projection
            .update(ended.into_run());

        runtime.spawn_provider_relaunch(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
            0,
            Some(first.id().to_string()),
            25,
        );

        let newer = runtime
            .owned
            .start_provider_launch(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(agent.id()),
            )
            .expect("newer prompt-owned provider launch should start")
            .run;
        runtime.owned.provider_run_projection.update(newer.clone());

        tokio::time::sleep(std::time::Duration::from_millis(75)).await;

        let live_runs = runtime
            .owned
            .provider_store
            .list_runs()
            .into_iter()
            .filter(|run| {
                run.session_id() == session.id()
                    && run.agent_instance_id() == Some(agent.id())
                    && run.state() != crate::provider::ProviderRunState::Ended
            })
            .map(|run| run.id().to_string())
            .collect::<Vec<_>>();
        assert_eq!(live_runs, vec![newer.id().to_string()]);
    }
}
