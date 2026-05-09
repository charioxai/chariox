//! Provider reload policy for launch-time runtime changes.
//!
//! This module owns the shared decision and relaunch path for changes that require a provider
//! process to be started with different launch inputs.

use super::*;

#[derive(Debug, Clone)]
pub(crate) enum ProviderReloadTrigger {
    AgentMcpGrant {
        session_id: String,
        agent_id: String,
        name: String,
    },
    UserConfigChanged {
        path: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderReloadOutcome {
    Unaffected,
    Reloaded,
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderLaunchFingerprint {
    runtime_mcp_server_url: Option<String>,
    mcp_servers: Vec<crate::mcp::ArrobaMcpServerConfig>,
    provider_env_remove: Vec<String>,
    write_access_mode: crate::provider::ProviderWriteAccessMode,
    execution_mode: crate::provider::AgentExecutionMode,
    permission_level: crate::provider::AgentPermissionLevel,
}

impl ProviderLaunchFingerprint {
    fn from_run(run: &crate::provider::RuntimeProviderRun) -> Self {
        Self {
            runtime_mcp_server_url: run.runtime_mcp_server_url().map(str::to_string),
            mcp_servers: run.mcp_servers().to_vec(),
            provider_env_remove: run.pty_env_remove().to_vec(),
            write_access_mode: run.write_access_mode(),
            execution_mode: run.execution_mode(),
            permission_level: run.permission_level(),
        }
    }

    fn from_request(request: &crate::provider::LaunchProviderRequest) -> Self {
        Self {
            runtime_mcp_server_url: request
                .runtime_mcp_binding
                .as_ref()
                .map(|binding| binding.server_url.clone()),
            mcp_servers: request.mcp_servers.clone(),
            provider_env_remove: request.provider_env_remove.clone(),
            write_access_mode: request.write_access_mode,
            execution_mode: request.execution_mode.unwrap_or_default(),
            permission_level: request.permission_level.unwrap_or_default(),
        }
    }
}

impl KernelRuntimeState {
    pub(crate) async fn apply_provider_reload_policy(
        &self,
        trigger: ProviderReloadTrigger,
    ) -> Result<Vec<ProviderReloadOutcome>, DaemonError> {
        let mut outcomes = Vec::new();
        match trigger {
            ProviderReloadTrigger::AgentMcpGrant {
                session_id,
                agent_id,
                name,
            } => {
                let reason = format!("MCP `{name}`");
                outcomes.push(
                    self.reload_agent_provider_for_policy(&session_id, &agent_id, &reason)
                        .await?,
                );
            }
            ProviderReloadTrigger::UserConfigChanged { path } => {
                if !user_config_path_requires_provider_reload(&path) {
                    outcomes.push(ProviderReloadOutcome::Unaffected);
                    return Ok(outcomes);
                }
                for run in self.owned.provider_store.list_runs() {
                    let Some(agent_id) = run.agent_instance_id().map(str::to_string) else {
                        continue;
                    };
                    if !matches!(
                        run.state(),
                        crate::provider::ProviderRunState::Running
                            | crate::provider::ProviderRunState::Starting
                    ) {
                        continue;
                    }
                    let reason = format!("config `{path}`");
                    outcomes.push(
                        self.reload_agent_provider_for_policy(run.session_id(), &agent_id, &reason)
                            .await?,
                    );
                }
                if outcomes.is_empty() {
                    outcomes.push(ProviderReloadOutcome::Unaffected);
                }
            }
        }
        Ok(outcomes)
    }

    pub(super) async fn reload_agent_provider_for_policy(
        &self,
        session_id: &str,
        agent_id: &str,
        reason: &str,
    ) -> Result<ProviderReloadOutcome, DaemonError> {
        match self.reload_agent_provider_if_idle(session_id, agent_id, reason)? {
            ProviderReloadOutcome::Deferred => {
                self.remember_pending_provider_reload(session_id, agent_id, reason);
                Ok(ProviderReloadOutcome::Deferred)
            }
            outcome => Ok(outcome),
        }
    }

    pub(super) fn reload_agent_provider_if_idle(
        &self,
        session_id: &str,
        agent_id: &str,
        reason: &str,
    ) -> Result<ProviderReloadOutcome, DaemonError> {
        let (launch_request, runtime_init_delay_ms, terminated_run_id) = {
            let owned = &self.owned;
            if owned
                .prompt_state_owner
                .active_prompt_for_agent(&owned.session_store.get_session(session_id)?, agent_id)
                .is_some()
            {
                if let Some(run) = owned.provider_store.get_run_for_agent(session_id, agent_id) {
                    owned.record_notice(
                        session_id,
                        Some(run.id()),
                        owned.attachment_store.list_session_attachment_ids(session_id),
                        format!(
                            "Provider reload for {reason} is pending until agent `{agent_id}` is idle."
                        ),
                    );
                }
                return Ok(ProviderReloadOutcome::Deferred);
            }
            let Some(run) = owned.provider_store.get_run_for_agent(session_id, agent_id) else {
                return Ok(ProviderReloadOutcome::Unaffected);
            };
            if !matches!(run.adapter_key(), "codex" | "opencode") {
                return Ok(ProviderReloadOutcome::Unaffected);
            }
            let config = owned.config_projection.snapshot();
            let mut launch_request = crate::provider::LaunchProviderRequest::new(
                session_id,
                run.adapter_key(),
                run.provider(),
                run.account_profile(),
                run.model(),
            )
            .with_agent_id(agent_id)
            .with_owner_user_id(run.owner_user_id().to_string())
            .with_variant(run.variant().map(str::to_string))
            .with_resume_state(run.resume_state().clone());
            if crate::provider::provider_requires_managed_io_by_default(run.provider(), &config) {
                launch_request = launch_request.with_managed_io_required();
            }
            let launch_request =
                owned.prepare_provider_launch_request(launch_request, config.runtime_mcp_url())?;
            if ProviderLaunchFingerprint::from_run(&run)
                == ProviderLaunchFingerprint::from_request(&launch_request)
            {
                return Ok(ProviderReloadOutcome::Unaffected);
            }

            let mut terminated_run_id = None;
            if run.state() != crate::provider::ProviderRunState::Ended {
                terminated_run_id = Some(run.id().to_string());
                let outcome = owned
                    .provider_store
                    .terminate_run_provider_only(session_id, run.id())?;
                owned.clear_active_provider_run_session_pointer(session_id, outcome.run().id())?;
                owned.provider_run_projection.update(outcome.into_run());
            }
            owned.record_notice(
                session_id,
                None,
                owned
                    .attachment_store
                    .list_session_attachment_ids(session_id),
                format!(
                    "Reloading provider conversation for agent `{agent_id}` after {reason} changed."
                ),
            );
            (
                launch_request,
                config.provider_runtime_init_delay_ms,
                terminated_run_id,
            )
        };

        self.spawn_provider_relaunch(
            launch_request,
            runtime_init_delay_ms,
            terminated_run_id,
            12_000,
        );
        Ok(ProviderReloadOutcome::Reloaded)
    }

    pub(super) async fn activate_next_agent_substitute_after_failure(
        &self,
        session_id: &str,
        agent_id: &str,
        reason: &str,
    ) -> Result<bool, DaemonError> {
        let (launch_request, runtime_init_delay_ms, agent) = {
            let owned = &self.owned;
            let current = owned.agent_store.get_agent(agent_id)?;
            if current.remote_execution().is_some() {
                return Ok(false);
            }
            let Some(substitute_index) = next_substitute_index(&current) else {
                return Ok(false);
            };
            let (agent, profile) = owned.agent_store.activate_agent_substitute(
                agent_id,
                substitute_index,
                reason.to_string(),
            )?;
            let adapter_key = match profile.provider.as_str() {
                "default" => "opencode",
                value => value,
            };
            let provider = adapter_key;
            let config = owned.config_projection.snapshot();
            let mut launch_request = crate::provider::LaunchProviderRequest::new(
                session_id,
                adapter_key,
                provider,
                "default",
                profile.model.clone(),
            )
            .with_agent_id(agent_id)
            .with_owner_user_id(agent.owner_user_id().to_string())
            .with_variant(profile.variant.clone());
            if crate::provider::provider_requires_managed_io_by_default(provider, &config) {
                launch_request = launch_request.with_managed_io_required();
            }
            let launch_request =
                owned.prepare_provider_launch_request(launch_request, config.runtime_mcp_url())?;
            owned.record_notice(
                session_id,
                None,
                owned
                    .attachment_store
                    .list_session_attachment_ids(session_id),
                format!(
                    "Activating substitute {} for agent `{agent_id}` after {reason}.",
                    substitute_index
                ),
            );
            let _ = owned.session_snapshot(session_id)?;
            (launch_request, config.provider_runtime_init_delay_ms, agent)
        };

        self.append_agent_durable_event("agent.substitute_activated", &agent, None)
            .await?;
        self.spawn_provider_relaunch(launch_request, runtime_init_delay_ms, None, 0);
        Ok(true)
    }

    fn remember_pending_provider_reload(&self, session_id: &str, agent_id: &str, reason: &str) {
        self.owned.pending_provider_reloads.write().insert(
            agent_id.to_string(),
            PendingProviderReload {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
                reason: reason.to_string(),
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
                    let pending = {
                        let mut pending = state.owned.pending_provider_reloads.write();
                        pending.remove(&agent_id)
                    };
                    if let Some(pending) = pending {
                        if let Err(error) = state.reload_agent_provider_if_idle(
                            &pending.session_id,
                            &pending.agent_id,
                            &pending.reason,
                        ) {
                            crate::logging::warn_with_fields(
                                "daemon.provider",
                                "pending provider reload failed",
                                serde_json::json!({
                                    "session_id": pending.session_id,
                                    "agent_id": pending.agent_id,
                                    "error": error.to_string(),
                                }),
                            );
                        }
                    }
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        });
    }

    fn spawn_provider_relaunch(
        &self,
        launch_request: crate::provider::LaunchProviderRequest,
        runtime_init_delay_ms: u64,
        terminated_run_id: Option<String>,
        launch_delay_ms: u64,
    ) {
        let state = self.clone();
        let app = self.app.clone();
        tokio::spawn(async move {
            if let Some(terminated_run_id) = terminated_run_id {
                let (_, process_key) = state
                    .with_app_side_effect(|app| {
                        crate::app::ProviderLaunchProcessRuntime::new(app)
                            .remove_run(&terminated_run_id)
                    })
                    .await
                    .unwrap_or((false, None));
                state
                    .owned
                    .remove_provider_process_tracking_for_run(&terminated_run_id, process_key);
            }
            if launch_delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(launch_delay_ms)).await;
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
            let spawn_result = {
                let mut app = app.lock().await;
                crate::app::ProviderLaunchProcessRuntime::new(&mut app)
                    .spawn_for_launch(&started.run)
            };
            if let Err(error) = spawn_result {
                state.fail_provider_launch(&started, &error).await;
                return;
            }
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

fn next_substitute_index(agent: &crate::agent::AgentInstance) -> Option<usize> {
    let next = agent
        .active_substitute_index()
        .map(|index| index.saturating_add(1))
        .unwrap_or(0);
    (next < agent.substitutes().len()).then_some(next)
}

fn user_config_path_requires_provider_reload(path: &str) -> bool {
    path == "providers.managed_io"
}
