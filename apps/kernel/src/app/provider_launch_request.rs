use std::path::PathBuf;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::{LaunchProviderRequest, RuntimeMcpBinding};

use super::provider_launch_policy::{
    default_provider_env_remove, generate_runtime_mcp_auth_token,
    granted_mcp_servers_for_agent_launch, resolve_mcp_credentials_for_launch,
    sanitize_resume_state_for_launch,
};

impl DaemonApp {
    pub(crate) fn prepare_app_provider_launch_request(
        &self,
        mut request: LaunchProviderRequest,
        operation: &'static str,
    ) -> Result<LaunchProviderRequest, DaemonError> {
        let session = self.sessions.get_session(&request.session_id)?;
        if request.agent_id.is_none() {
            request.agent_id = session.focused_agent_id().map(str::to_string);
        }
        let agent = request
            .agent_id
            .as_deref()
            .and_then(|agent_id| self.agents.get_agent(agent_id).ok());
        if let Some(agent) = agent.as_ref() {
            if agent.remote_execution().is_some() {
                return Err(DaemonError::LocalTransport {
                    operation,
                    message: format!(
                        "agent `{}` is remote-backed and must launch its provider on the worker kernel",
                        agent.id()
                    ),
                });
            }
            request = request.with_owner_user_id(agent.owner_user_id().to_string());
        } else {
            request = request.with_owner_user_id(session.owner_user_id().to_string());
        }
        if request.resume_state.is_none() {
            if let Some(agent) = agent.as_ref() {
                let resume_state = sanitize_resume_state_for_launch(&request, agent);
                if !resume_state.is_empty() {
                    request = request.with_resume_state(resume_state);
                }
            }
        }
        if request.working_directory.is_none() {
            let working_directory = agent
                .as_ref()
                .and_then(|agent| agent.worktree_id().map(PathBuf::from))
                .unwrap_or_else(|| PathBuf::from(session.worktree_id()));
            request = request.with_working_directory(working_directory);
        }
        if request.runtime_mcp_binding.is_none() {
            let shared_auth_token = request
                .agent_id
                .is_none()
                .then(|| {
                    self.providers
                        .get_session_run_for_provider(&request.session_id, &request.provider)
                        .and_then(|run| run.runtime_mcp_auth_token().map(str::to_string))
                })
                .flatten();
            request = request.with_runtime_mcp_binding(RuntimeMcpBinding::new(
                self.config.runtime_mcp_url(),
                shared_auth_token.unwrap_or_else(generate_runtime_mcp_auth_token),
            ));
        }
        if request.provider_env_remove.is_empty() {
            request = request.with_provider_env_remove(default_provider_env_remove(&self.config));
        }
        if request.mcp_servers.is_empty() {
            if let Some(agent) = agent.as_ref() {
                request = request.with_mcp_servers(granted_mcp_servers_for_agent_launch(
                    operation, &session, agent,
                )?);
            }
        }
        let mcp_servers = std::mem::take(&mut request.mcp_servers);
        request = request.with_mcp_servers(resolve_mcp_credentials_for_launch(
            &self.config,
            mcp_servers,
        )?);
        Ok(request)
    }
}
