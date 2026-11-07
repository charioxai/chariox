//! MP-08 / MP-10 / MP-11 A10-S04: a turn whose workspace was removed can no
//! longer complete. The sweep ends it as failed, with a typed outcome that names
//! the directory, instead of letting the provider run on against nothing.
use super::*;

/// A repository, or a gitfile pointing at one. The provider's command sandbox
/// mounts an empty `.git` directory over a plain folder while a command runs
/// and removes it afterwards; that placeholder holds no worktree.
fn holds_git_worktree(directory: &std::path::Path) -> bool {
    let git = directory.join(".git");
    match std::fs::symlink_metadata(&git) {
        Ok(metadata) if metadata.is_dir() => git.join("HEAD").exists(),
        Ok(_) => true,
        Err(_) => false,
    }
}

impl KernelRuntimeState {
    /// The run's directory is gone, or no longer holds the git worktree the sweep
    /// saw there. A substitute would run in the same directory, so a lost
    /// workspace never reruns the turn.
    pub(super) fn workspace_of_run_is_lost(
        &self,
        run: &crate::provider::RuntimeProviderRun,
    ) -> bool {
        let Some(directory) = run.working_directory() else {
            return false;
        };
        !holds_git_worktree(directory)
            && (!directory.exists()
                || self
                    .owned
                    .git_workspaces_seen
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .contains(run.id()))
    }

    /// Fails the active turn of every local provider run whose working directory
    /// is gone: removed, or emptied of the git worktree the sweep saw earlier.
    pub(super) async fn end_turns_on_lost_workspace(&self) {
        let mut running = BTreeSet::new();
        let mut lost = Vec::new();
        for agent in self.owned.agent_store.list_agents() {
            if agent.remote_execution().is_some() {
                continue;
            }
            let Some(run) = self
                .owned
                .provider_store
                .get_run_for_agent(agent.session_id(), agent.id())
                .filter(|run| run.state() == crate::provider::ProviderRunState::Running)
            else {
                continue;
            };
            let Some(directory) = run.working_directory() else {
                continue;
            };
            running.insert(run.id().to_string());
            if holds_git_worktree(directory) {
                self.owned
                    .git_workspaces_seen
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(run.id().to_string());
            } else if self.workspace_of_run_is_lost(&run) {
                lost.push((
                    agent.session_id().to_string(),
                    run.id().to_string(),
                    directory.clone(),
                ));
            }
        }
        self.owned
            .git_workspaces_seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|run| running.contains(run));
        for (session_id, run_id, directory) in lost {
            let message = format!(
                "The workspace `{}` of this turn was removed, so the turn can no longer complete",
                directory.display()
            );
            if let Err(error) = self
                .fail_owned_provider_prompt_with_termination(
                    &session_id,
                    &run_id,
                    &message,
                    true,
                    Some(crate::provider::ProviderRunTermination::host_lost(
                        &message,
                        crate::session::unix_epoch_ms(),
                    )),
                )
                .await
            {
                tracing::warn!(%error, %session_id, %run_id,
                    "MP-08/MP-10/MP-11 A10-S04: turn on a removed workspace kept for the next sweep");
            }
        }
    }
}
