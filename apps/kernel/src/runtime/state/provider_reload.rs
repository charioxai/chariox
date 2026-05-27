//! Provider reload policy for launch-time runtime changes.
//!
//! This module owns the shared decision and relaunch path for changes that require a provider
//! process to be started with different launch inputs.

use super::*;

#[derive(Debug, Clone)]
pub(crate) enum ProviderReloadTrigger {
    AgentMcpChanged {
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
            ProviderReloadTrigger::AgentMcpChanged {
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
            if !adapter_supports_policy_reload(run.adapter_key()) {
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
            launch_request = launch_request.with_workspace_live_sync_mode(
                crate::provider::provider_workspace_live_sync_mode_by_default(
                    run.provider(),
                    &config,
                ),
            );
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
}

fn user_config_path_requires_provider_reload(path: &str) -> bool {
    path == "providers.workspace_live_sync"
}

fn adapter_supports_policy_reload(adapter_key: &str) -> bool {
    matches!(adapter_key, "claude" | "codex" | "opencode")
}

#[cfg(test)]
mod tests {
    use super::adapter_supports_policy_reload;

    #[test]
    fn provider_reload_policy_includes_structured_real_provider_adapters() {
        for adapter in ["claude", "codex", "opencode"] {
            assert!(
                adapter_supports_policy_reload(adapter),
                "{adapter} should relaunch when launch-time MCP config changes"
            );
        }
    }

    #[test]
    fn provider_reload_policy_excludes_unmanaged_or_stub_adapters() {
        for adapter in ["dev-stub", "unknown"] {
            assert!(
                !adapter_supports_policy_reload(adapter),
                "{adapter} should not use provider relaunch policy"
            );
        }
    }
}
