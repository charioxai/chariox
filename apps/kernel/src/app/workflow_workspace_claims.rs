use crate::app::DaemonApp;
use crate::error::DaemonError;

impl DaemonApp {
    pub(crate) fn acquire_workflow_node_workspace_claim(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<(), DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let agent = self.agents.get_agent(agent_id)?;
        let workspace_id = session.workspace_id().to_string();
        let worktree_id = agent
            .worktree_id()
            .unwrap_or_else(|| session.worktree_id())
            .to_string();
        let claim = self.workspace_coordinator.acquire_worktree_write_claim(
            workspace_id,
            worktree_id,
            session_id,
            Some(format!("{}:{}", workflow_run_id, workflow_node_run_id)),
            "workflow_node_dispatch",
        )?;
        self.prompt_workspace_claims
            .insert(provider_run_id.to_string(), claim);
        Ok(())
    }

    pub(crate) fn release_prompt_workspace_claim(&mut self, provider_run_id: &str) -> bool {
        self.prompt_workspace_claims.remove(provider_run_id)
    }

    pub(crate) fn release_workflow_node_workspace_claim(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> bool {
        let owner = format!("{workflow_run_id}:{workflow_node_run_id}");
        self.prompt_workspace_claims.remove_matching(|claim| {
            claim.session_id == session_id
                && claim.attachment_id.as_deref() == Some(owner.as_str())
                && claim.operation == "workflow_node_dispatch"
        }) > 0
    }
}
