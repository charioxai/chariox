use std::path::PathBuf;

use crate::error::DaemonError;
use crate::execution_lease::LeasedAgent;
use crate::transport::relay_peer::{RemoteGitObservation, RemoteGitTurnContext};

use super::RemoteLeaseRuntime;

impl<'a> RemoteLeaseRuntime<'a> {
    pub(super) fn observe_leased_git_before(
        &mut self,
        leased_agent: &LeasedAgent,
        provider_run_id: &str,
        git_context: RemoteGitTurnContext,
    ) {
        let Some(lease) = self
            .app
            .execution_leases
            .get(&leased_agent.lease_id)
            .cloned()
        else {
            return;
        };
        let Ok(provider_run) = self.app.providers.get_run(provider_run_id) else {
            return;
        };
        let worktree_path = provider_run.working_directory().cloned().or_else(|| {
            self.app
                .sessions
                .get_session(&leased_agent.backing_session_id)
                .ok()
                .map(|session| PathBuf::from(session.worktree_id()))
        });
        let Some(worktree_path) = worktree_path else {
            return;
        };
        let context = crate::git_observer::GitTurnContext {
            session_id: git_context.home_session_id,
            agent_id: git_context.home_agent_id,
            provider: leased_agent.provider.clone(),
            model: leased_agent
                .model
                .clone()
                .unwrap_or_else(|| "default".to_string()),
            provider_run_id: provider_run_id.to_string(),
            provider_session_id: provider_run.provider_session_id().map(str::to_string),
            prompt_id: git_context.home_prompt_id,
            turn_id: git_context.home_turn_id,
            worktree_path,
            machine_id: Some(lease.machine_id),
            prompt_summary: git_context.prompt_summary,
        };
        if let Some(snapshot) = crate::git_observer::capture_turn_snapshot(context) {
            self.app.remote_git_turn_snapshots.insert(snapshot);
        }
    }

    pub(crate) fn observe_leased_git_after(
        &mut self,
        leased_agent_id: &str,
        provider_run_id: &str,
    ) -> Result<Vec<RemoteGitObservation>, DaemonError> {
        let _leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        let Some(before) = self
            .app
            .remote_git_turn_snapshots
            .remove_for_provider_run(provider_run_id)
        else {
            return Ok(Vec::new());
        };
        let candidates = self.app.remote_git_turn_snapshots.candidates_for(&before);
        let after_context = crate::git_observer::GitTurnContext {
            session_id: before.session_id.clone(),
            agent_id: before.agent_id.clone(),
            provider: before.provider.clone(),
            model: before.model.clone(),
            provider_run_id: before.provider_run_id.clone(),
            provider_session_id: before.provider_session_id.clone(),
            prompt_id: before.prompt_id.clone(),
            turn_id: before.turn_id.clone(),
            worktree_path: PathBuf::from(before.worktree_path.clone()),
            machine_id: before.machine_id.clone(),
            prompt_summary: before.prompt_summary.clone(),
        };
        let Some(after) = crate::git_observer::capture_turn_snapshot(after_context) else {
            return Ok(Vec::new());
        };
        Ok(crate::git_observer::observations_after_turn(
            before, after, candidates,
        ))
    }
}
