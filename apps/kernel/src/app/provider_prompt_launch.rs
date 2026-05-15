use std::path::PathBuf;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::{LaunchProviderRequest, ProviderRunState};

impl DaemonApp {
    pub(crate) fn ensure_prompt_provider_run_for_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<String, DaemonError> {
        if let Some(agent_run) = self.providers.get_run_for_agent(session_id, agent_id) {
            return match agent_run.state() {
                ProviderRunState::Running | ProviderRunState::Starting => {
                    Ok(agent_run.id().to_string())
                }
                ProviderRunState::Parked => {
                    let resumed = self.providers.resume_run_detached(agent_run.id())?;
                    self.update_provider_run_projection(resumed.clone());
                    Ok(resumed.id().to_string())
                }
                ProviderRunState::Ended => Err(DaemonError::NoActiveProviderRun {
                    session_id: session_id.to_string(),
                }),
            };
        }

        let agent = self.agents.get_agent(agent_id)?;
        if agent.remote_execution().is_some() {
            return Err(DaemonError::LocalTransport {
                operation: "ensure prompt provider run for agent",
                message: format!(
                    "agent `{agent_id}` is remote-backed and must launch its provider on the worker kernel"
                ),
            });
        }
        let adapter_key = match agent.provider() {
            "default" => "opencode",
            value => value,
        };
        let provider = match agent.provider() {
            "default" => "opencode",
            value => value,
        };
        let session = self.sessions.get_session(session_id)?;
        let effective_config =
            crate::session::effective_agent_execution_config(&session, Some(&agent));
        let mut request = LaunchProviderRequest::new(
            session_id,
            adapter_key,
            provider,
            "default",
            agent.model().unwrap_or("default"),
        )
        .with_agent_id(agent.id().to_string())
        .with_variant(agent.effort().map(str::to_string))
        .with_execution_mode(effective_config.mode)
        .with_permission_level(effective_config.permission_level);
        if crate::provider::provider_requires_managed_io_by_default(provider, &self.config) {
            request = request.with_managed_io_required();
        }
        if let Some(worktree_id) = agent.worktree_id() {
            request = request.with_working_directory(PathBuf::from(worktree_id));
        }
        let provider_run = self.launch_provider_detached(request)?;
        Ok(provider_run.id().to_string())
    }
}
