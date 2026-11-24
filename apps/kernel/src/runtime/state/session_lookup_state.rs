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

    pub(crate) async fn resolve_session_ref_id_for_delete(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<String, DaemonError> {
        Ok(self
            .owned
            .session_store
            .read()
            .resolve_session_ref_for_delete(session_ref, workspace_id)?
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

    /// The room and its agents as stored: enough for reads that only ask which agents a
    /// room has and who may see them. A full snapshot also reads the room's tasks, wakes and
    /// queued messages from the durable store and republishes it (b3, 2026-11-18: ~800 ms
    /// each under load, twice per history outline, on the kernel's async workers).
    pub(crate) fn session_with_agents(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        let mut session = self.owned.session_store.get_session(session_id)?;
        session.set_agents(self.owned.agent_store.get_session_agents(session_id));
        Ok(session)
    }

    /// Identical reads in flight share one read (`in_flight_reads`).
    pub(crate) async fn read_once<F, Fut>(
        &self,
        key: String,
        read: F,
    ) -> Result<LocalDaemonResponse, DaemonError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<LocalDaemonResponse, DaemonError>>,
    {
        self.in_flight_reads.read(key, read).await
    }

    #[cfg(test)]
    pub(crate) fn take_session_snapshot_builds() -> usize {
        super::session::call_counts::SNAPSHOT_BUILDS.with(|count| count.replace(0))
    }

    pub(crate) fn list_session_snapshots(&self) -> Vec<crate::session::RuntimeSession> {
        self.owned
            .session_store
            .list_sessions()
            .into_iter()
            .filter_map(|session| {
                self.owned
                    .session_snapshot_without_projection_update(session.id())
                    .ok()
            })
            .collect()
    }

    pub(crate) fn list_session_health_snapshots(&self) -> Vec<crate::session::RuntimeSession> {
        self.owned
            .session_store
            .list_sessions()
            .into_iter()
            .map(|mut session| {
                session.set_agents(self.owned.agent_store.get_session_agents(session.id()));
                session
            })
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
        // The label is read before the session, so a state is never older than its revision.
        let agent_activity_revision = self.owned.session_projection.change_sequence();
        let session = self.session_for_state_read(&request.session_id)?;
        Ok(LocalDaemonResponse::SessionState {
            room_workflows: crate::runtime::projection::RoomWorkflowInventory::project(
                &session,
                crate::session::DEFAULT_LOCAL_USER_ID,
            ),
            agent_activity: self.agent_activity_for_session(&session),
            agent_activity_revision,
            jobs: self.session_jobs(session.id()),
            processes: self.room_agent_processes(&session),
            session,
        })
    }

    pub(crate) fn session_for_state_read(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.owned.session_snapshot(session_id)
    }

    pub(crate) fn room_agent_processes(
        &self,
        session: &crate::session::RuntimeSession,
    ) -> Vec<crate::runtime::process_resources::SessionProcess> {
        self.session_processes(session.id())
            .into_iter()
            .filter(|process| process.belongs_to_room_agent(session))
            .collect()
    }

    pub(crate) fn list_agents_response(
        &self,
        request: crate::local::ListAgentsRequest,
    ) -> LocalDaemonResponse {
        LocalDaemonResponse::AgentsListed {
            agents: self
                .owned
                .agent_store
                .get_session_agents(&request.session_id)
                .into_iter()
                .filter(|agent| !agent.archived())
                .collect(),
        }
    }
}
