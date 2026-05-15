use std::path::PathBuf;

use crate::error::DaemonError;
use crate::provider::{
    LaunchProviderRequest, ProviderClientInterface, ProviderResumeState, RuntimeProviderRun,
};

use super::RemoteLeaseRuntime;

impl<'a> RemoteLeaseRuntime<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn launch_leased_native_provider_run(
        &mut self,
        leased_agent_id: &str,
        adapter_key: &str,
        provider: &str,
        account_profile: &str,
        model: &str,
        variant: Option<String>,
        structured_endpoint: Option<String>,
        provider_session_id: Option<String>,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        let lease = self
            .app
            .execution_leases
            .get(&leased_agent.lease_id)
            .cloned()
            .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                lease_id: leased_agent.lease_id.clone(),
            })?;
        let backing_session = self
            .app
            .sessions
            .get_session(&leased_agent.backing_session_id)?;
        let mut request = LaunchProviderRequest::new(
            leased_agent.backing_session_id.clone(),
            adapter_key,
            provider,
            account_profile,
            model,
        )
        .with_agent_id(leased_agent.backing_agent_id.clone())
        .with_owner_user_id(lease.owner_user_id)
        .with_working_directory(PathBuf::from(backing_session.worktree_id()))
        .with_client_interface(ProviderClientInterface::NativeTui)
        .with_variant(variant);
        if let Some(execution_mode) = leased_agent.execution_mode {
            request = request.with_execution_mode(execution_mode);
        }
        if let Some(permission_level) = leased_agent.permission_level {
            request = request.with_permission_level(permission_level);
        }
        if let Some(endpoint) = structured_endpoint {
            request = request.with_structured_endpoint(endpoint);
        }
        if let Some(provider_session_id) = provider_session_id {
            request = match adapter_key {
                "codex" => request.with_resume_state(ProviderResumeState::from_codex_thread_id(
                    provider_session_id,
                )),
                "opencode" => request.with_resume_state(
                    ProviderResumeState::from_opencode_session_id(provider_session_id),
                ),
                "claude" => request.with_resume_state(ProviderResumeState::from_claude_session_id(
                    provider_session_id,
                )),
                _ => request,
            };
        }
        self.app.launch_provider(request)
    }
}
