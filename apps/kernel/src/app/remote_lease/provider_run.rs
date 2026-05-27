use crate::error::DaemonError;
use crate::execution_lease::LeasedAgent;
use crate::provider::LaunchProviderRequest;
use crate::transport::relay_peer::RequiredRemoteMcp;

use super::mcp_availability::provider_run_mcp_set_matches;
use super::RemoteLeaseRuntime;

impl<'a> RemoteLeaseRuntime<'a> {
    pub(super) fn ensure_leased_provider_run_matches_mcps(
        &mut self,
        leased_agent: &LeasedAgent,
        required_mcps: &[RequiredRemoteMcp],
    ) -> Result<String, DaemonError> {
        let lease = self
            .app
            .execution_leases
            .get(&leased_agent.lease_id)
            .cloned()
            .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                lease_id: leased_agent.lease_id.clone(),
            })?;
        let existing = self.app.providers.get_run_for_agent(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        );
        if let Some(run) = existing.as_ref() {
            if provider_run_mcp_set_matches(run, required_mcps)? {
                return Ok(run.id().to_string());
            }
            if self
                .app
                .prompt_owner_active_prompt_for_agent(
                    &leased_agent.backing_session_id,
                    &leased_agent.backing_agent_id,
                )?
                .is_some()
            {
                return Err(DaemonError::LocalTransport {
                    operation: "remote MCP provider reload",
                    message: format!(
                        "remote worker provider run `{}` does not have the required MCP set and is currently busy; retry after the active turn completes",
                        run.id()
                    ),
                });
            }
            let run_id = run.id().to_string();
            let _ = crate::app::provider_runtime::ProviderProcessTracker::new(self.app)
                .remove_run(&run_id);
            if let Ok(outcome) = self
                .app
                .providers
                .terminate_run_provider_only(run.session_id(), run.id())
            {
                let _ = self
                    .app
                    .sessions
                    .set_active_provider_run(outcome.run().session_id(), None);
                self.app.update_provider_run_projection(outcome.into_run());
            }
        }

        let mut request = LaunchProviderRequest::new(
            &leased_agent.backing_session_id,
            &leased_agent.provider,
            &leased_agent.provider,
            "default",
            leased_agent
                .model
                .clone()
                .unwrap_or_else(|| "default".to_string()),
        )
        .with_agent_id(&leased_agent.backing_agent_id)
        .with_owner_user_id(lease.owner_user_id)
        .with_working_directory(std::path::PathBuf::from(
            self.app
                .sessions
                .get_session(&leased_agent.backing_session_id)?
                .worktree_id(),
        ))
        .with_mcp_servers(
            required_mcps
                .iter()
                .map(|required| required.config.clone())
                .collect(),
        );
        if let Some(execution_mode) = leased_agent.execution_mode {
            request = request.with_execution_mode(execution_mode);
        }
        if let Some(permission_level) = leased_agent.permission_level {
            request = request.with_permission_level(permission_level);
        }
        if leased_agent.effort.is_some() {
            request = request.with_variant(leased_agent.effort.clone());
        }
        if let Some(run) = existing.as_ref() {
            request = request.with_resume_state(run.resume_state().clone());
            if request.variant.is_none() {
                request = request.with_variant(run.variant().map(str::to_string));
            }
        }
        request = request.with_workspace_live_sync_mode(
            crate::provider::provider_workspace_live_sync_mode_by_default(
                &leased_agent.provider,
                self.app.config(),
            ),
        );
        let run = self.app.launch_provider(request)?;
        Ok(run.id().to_string())
    }
}
