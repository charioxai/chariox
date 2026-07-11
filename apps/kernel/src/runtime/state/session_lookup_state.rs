use super::*;

impl KernelRuntimeState {
    pub(crate) async fn active_prompt_agent_id(
        &self,
        session_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        Ok(self
            .owned
            .prompt_state_owner
            .active_prompt_agent_id(&session))
    }

    pub(crate) async fn focused_agent_id(
        &self,
        session_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        Ok(session.focused_agent_id().map(str::to_string))
    }

    pub(crate) async fn resolve_session_ref_id(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<String, DaemonError> {
        Ok(self
            .owned
            .session_store
            .read()
            .resolve_session_ref(session_ref, workspace_id)?
            .id()
            .to_string())
    }

    pub(crate) async fn attachment_session_id(
        &self,
        attachment_id: &str,
    ) -> Result<String, DaemonError> {
        Ok(self
            .owned
            .attachment_store
            .get_attachment(attachment_id)?
            .session_id()
            .to_string())
    }

    pub(crate) async fn attachment_owner_user_id(
        &self,
        attachment_id: &str,
    ) -> Result<String, DaemonError> {
        Ok(self
            .owned
            .attachment_store
            .get_attachment(attachment_id)?
            .owner_user_id()
            .to_string())
    }

    pub(crate) async fn session_snapshot(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.owned.session_snapshot(session_id)
    }

    pub(crate) fn list_session_snapshots(&self) -> Vec<crate::session::RuntimeSession> {
        self.owned
            .session_store
            .list_sessions()
            .into_iter()
            .filter_map(|session| self.owned.session_snapshot_without_projection_update(session.id()).ok())
            .collect()
    }

    pub(crate) fn resolve_session_snapshot(
        &self,
        request: crate::local::ResolveSessionRequest,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        let session_id = self
            .owned
            .session_store
            .read()
            .resolve_session_ref(&request.session_ref, request.workspace_id.as_deref())?
            .id()
            .to_string();
        self.owned.session_snapshot(&session_id)
    }

    pub(crate) fn session_state_response(
        &self,
        request: crate::local::GetSessionStateRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let session = self.owned.session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::SessionState {
            agent_activity: self.agent_activity_for_session(&session),
            agent_activity_revision: self.owned.session_projection.change_sequence(),
            session,
        })
    }

    pub(crate) fn list_agents_response(
        &self,
        request: crate::local::ListAgentsRequest,
    ) -> LocalDaemonResponse {
        LocalDaemonResponse::AgentsListed {
            agents: self
                .owned
                .agent_store
                .get_session_agents(&request.session_id),
        }
    }
}
