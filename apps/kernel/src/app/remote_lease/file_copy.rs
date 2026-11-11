use super::*;

impl RemoteLeaseRuntime<'_> {
    /// The workspace a leased agent works in, as `chariox.file.copy` may use it: the folder of
    /// its backing session (the checkout the worker created for it, or the folder it was given).
    pub(crate) fn leased_agent_workspace(
        &self,
        leased_agent_id: &str,
    ) -> Result<crate::agent_file_copy::Workspace, DaemonError> {
        let leased = self.app.leased_agents.get(leased_agent_id).ok_or_else(|| {
            DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            }
        })?;
        let session = self.app.sessions.get_session(&leased.backing_session_id)?;
        Ok(crate::agent_file_copy::Workspace {
            root: std::path::PathBuf::from(session.worktree_id()),
            read_only: leased.execution_mode == Some(crate::provider::AgentExecutionMode::Plan),
        })
    }
}
