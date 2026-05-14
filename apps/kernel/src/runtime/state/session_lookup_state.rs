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

    pub(crate) async fn session_snapshot(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.owned.session_snapshot(session_id)
    }
}
