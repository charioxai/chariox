use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn launch_provider_request_from_local_request(
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
        let config = self.config_projection.snapshot();
        if crate::provider::provider_requires_workspace_live_sync_by_default(
            &launch_request.provider,
            &config,
        ) {
            launch_request = launch_request.with_workspace_live_sync_required();
        }
        if let Some(agent_id) = request.agent_id.clone().or_else(|| {
            self.session_store
                .get_session(&request.session_id)
                .ok()
                .and_then(|session| session.focused_agent_id().map(str::to_string))
                .or_else(|| {
                    self.agent_store
                        .get_focused_agent(&request.session_id)
                        .map(|agent| agent.id().to_string())
                })
        }) {
            launch_request = if let Ok(agent) = self.agent_store.get_agent(&agent_id) {
                let session = self.session_store.get_session(&request.session_id).ok();
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
            let session = self.session_store.get_session(&request.session_id).ok();
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

    pub(super) fn prepare_provider_launch_request(
        &self,
        mut request: crate::provider::LaunchProviderRequest,
        runtime_mcp_url: String,
    ) -> Result<crate::provider::LaunchProviderRequest, DaemonError> {
        let session = self.session_store.get_session(&request.session_id)?;
        if request.agent_id.is_none() {
            request.agent_id = self
                .session_store
                .get_session(&request.session_id)?
                .focused_agent_id()
                .map(str::to_string)
                .or_else(|| {
                    self.agent_store
                        .get_focused_agent(&request.session_id)
                        .map(|agent| agent.id().to_string())
                });
        }
        let agent = request
            .agent_id
            .as_deref()
            .and_then(|agent_id| self.agent_store.get_agent(agent_id).ok());
        if let Some(agent) = agent.as_ref() {
            if agent.remote_execution().is_some() {
                return Err(DaemonError::LocalTransport {
                    operation: "launch provider run",
                    message: format!(
                        "agent `{}` is remote-backed and must launch its provider on the worker kernel",
                        agent.id()
                    ),
                });
            }
        }
        let effective_config =
            crate::session::effective_agent_execution_config(&session, agent.as_ref());
        if request.execution_mode.is_none() {
            request = request.with_execution_mode(effective_config.mode);
        }
        if request.permission_level.is_none() {
            request = request.with_permission_level(effective_config.permission_level);
        }
        if request.resume_state.is_none() {
            if let Some(agent) = agent.as_ref() {
                let resume_state = crate::app::sanitize_resume_state_for_launch(&request, agent);
                if !resume_state.is_empty() {
                    request = request.with_resume_state(resume_state);
                }
            }
        }
        if request.working_directory.is_none() {
            let agent_worktree = agent
                .as_ref()
                .and_then(|agent| agent.worktree_id().map(std::path::PathBuf::from));
            request.working_directory = Some(
                agent_worktree.unwrap_or_else(|| std::path::PathBuf::from(session.worktree_id())),
            );
        }
        if request.runtime_mcp_binding.is_none() {
            let shared_auth_token = request
                .agent_id
                .is_none()
                .then(|| {
                    self.provider_store
                        .get_session_run_for_provider(&request.session_id, &request.provider)
                        .and_then(|run| run.runtime_mcp_auth_token().map(str::to_string))
                })
                .flatten();
            request = request.with_runtime_mcp_binding(crate::provider::RuntimeMcpBinding::new(
                runtime_mcp_url,
                shared_auth_token.unwrap_or_else(crate::app::generate_runtime_mcp_auth_token),
            ));
        }
        if request.provider_env_remove.is_empty() {
            let credential_env_names = crate::credential::load_user_credentials()
                .map(|credentials| {
                    crate::secret::RuntimeSecretService::credential_env_names_from(&credentials)
                })
                .unwrap_or_default();
            request = request.with_provider_env_remove(credential_env_names.into_iter().collect());
        }
        if request.mcp_servers.is_empty() {
            if let Some(agent) = agent.as_ref() {
                request =
                    request.with_mcp_servers(crate::app::granted_mcp_servers_for_agent_launch(
                        "provider.launch.mcps",
                        &session,
                        agent,
                    )?);
            }
        }
        let config = self.config_projection.snapshot();
        let mcp_servers = std::mem::take(&mut request.mcp_servers);
        request = request.with_mcp_servers(crate::app::resolve_mcp_credentials_for_launch(
            &config,
            mcp_servers,
        )?);
        Ok(request)
    }
}
