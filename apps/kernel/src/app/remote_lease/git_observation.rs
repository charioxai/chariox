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
        let workspace_live_sync_tracked = git_context
            .workspace_live_sync_mode
            .is_some_and(|mode| mode == crate::config::WorkspaceLiveSyncMode::Tracked)
            || provider_run.tracks_workspace_live_sync();
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
            workspace_live_sync_tracked,
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
    ) -> Result<
        (
            Vec<RemoteGitObservation>,
            Option<crate::git_observer::WorkspaceLiveSyncChange>,
        ),
        DaemonError,
    > {
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
            return Ok((Vec::new(), None));
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
            workspace_live_sync_tracked: before.workspace_live_sync_tracked,
            machine_id: before.machine_id.clone(),
            prompt_summary: before.prompt_summary.clone(),
        };
        let retry_delays_ms: &[u64] = if before.workspace_live_sync_tracked {
            &[50, 150, 300, 500]
        } else {
            &[]
        };
        let mut attempts = 0usize;
        let (after, tracked_change) = loop {
            let Some(after) = crate::git_observer::capture_turn_snapshot(after_context.clone())
            else {
                if attempts >= retry_delays_ms.len() {
                    return Ok((Vec::new(), None));
                }
                std::thread::sleep(std::time::Duration::from_millis(retry_delays_ms[attempts]));
                attempts += 1;
                continue;
            };
            let tracked_change = if before.workspace_live_sync_tracked {
                crate::git_observer::tracked_workspace_live_sync_change_after_turn(&before, &after)
            } else {
                None
            };
            let should_retry = before.workspace_live_sync_tracked && tracked_change.is_none();
            if !should_retry || attempts >= retry_delays_ms.len() {
                break (after, tracked_change);
            }
            std::thread::sleep(std::time::Duration::from_millis(retry_delays_ms[attempts]));
            attempts += 1;
        };
        if let Some(change) = tracked_change.as_ref() {
            crate::logging::info_with_fields(
                "daemon.workspace_live_sync",
                "recorded remote tracked workspace live sync turn change",
                serde_json::json!({
                    "provider_run_id": provider_run_id,
                    "changed_path_count": change.changed_paths.len(),
                    "retry_attempts": attempts,
                }),
            );
        } else if before.workspace_live_sync_tracked {
            crate::logging::info_with_fields(
                "daemon.workspace_live_sync",
                "remote tracked workspace live sync turn had no changed paths",
                serde_json::json!({
                    "provider_run_id": provider_run_id,
                    "retry_attempts": attempts,
                    "before_status_fingerprint": before.status_fingerprint.as_str(),
                    "after_status_fingerprint": after.status_fingerprint.as_str(),
                }),
            );
        }
        Ok((
            crate::git_observer::observations_after_turn(before, after, candidates),
            tracked_change,
        ))
    }
}
