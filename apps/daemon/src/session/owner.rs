use std::collections::BTreeMap;

use crate::error::DaemonError;

use super::{CreateSessionRequest, RuntimeSession, SessionConfigState, SessionService};

pub(crate) struct SessionStateReader<'a> {
    sessions: &'a SessionService,
}

impl<'a> SessionStateReader<'a> {
    pub(crate) fn new(sessions: &'a SessionService) -> Self {
        Self { sessions }
    }

    pub(crate) fn get_session(&self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        self.sessions.get_session(session_id)
    }

    pub(crate) fn list_sessions(&self) -> Vec<RuntimeSession> {
        self.sessions.list_sessions()
    }

    pub(crate) fn resolve_session_ref(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<RuntimeSession, DaemonError> {
        self.sessions.resolve_session_ref(session_ref, workspace_id)
    }
}

pub(crate) struct SessionStateOwner<'a> {
    sessions: &'a mut SessionService,
}

impl<'a> SessionStateOwner<'a> {
    pub(crate) fn new(sessions: &'a mut SessionService) -> Self {
        Self { sessions }
    }

    pub(crate) fn create_session(
        &mut self,
        request: CreateSessionRequest,
    ) -> Result<RuntimeSession, DaemonError> {
        self.sessions.create_session(request)
    }

    pub(crate) fn assign_session_alias(
        &mut self,
        session_id: &str,
        alias: String,
    ) -> Result<RuntimeSession, DaemonError> {
        self.sessions.assign_session_alias(session_id, alias)
    }

    pub(crate) fn update_config(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        values: BTreeMap<String, String>,
        requires_idle: bool,
    ) -> Result<(RuntimeSession, SessionConfigState), DaemonError> {
        self.sessions
            .update_config(session_id, attachment_id, values, requires_idle)
    }

    pub(crate) fn end_session(&mut self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        self.sessions.end_session(session_id)
    }

    pub(crate) fn delete_session(
        &mut self,
        session_id: &str,
    ) -> Result<RuntimeSession, DaemonError> {
        self.sessions.delete_session(session_id)
    }

    pub(crate) fn set_active_provider_run(
        &mut self,
        session_id: &str,
        provider_run_id: Option<String>,
    ) -> Result<RuntimeSession, DaemonError> {
        self.sessions
            .set_active_provider_run(session_id, provider_run_id)
    }
}
