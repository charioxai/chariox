//! MP-08 / MP-10 / MP-11 A10-S04: a turn whose workspace was removed can no
//! longer complete. The sweep ends it as failed, with a typed outcome that names
//! the directory, instead of letting the provider run on against nothing.
use super::*;

impl KernelRuntimeState {
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
            let mut seen = self
                .owned
                .git_workspaces_seen
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if directory.join(".git").exists() {
                seen.insert(run.id().to_string());
            } else if !directory.exists() || seen.contains(run.id()) {
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
