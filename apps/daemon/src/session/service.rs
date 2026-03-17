use crate::config::DaemonConfig;
use crate::error::DaemonError;

use super::{CreateSessionRequest, RuntimeSession, SessionStatus, SessionStore};

#[derive(Debug, Clone)]
pub struct SessionService {
    store: SessionStore,
    host_machine_id: String,
    host_daemon_id: String,
}

impl SessionService {
    pub fn new(config: &DaemonConfig) -> Self {
        Self {
            store: SessionStore::new(),
            host_machine_id: config.host_machine_id.clone(),
            host_daemon_id: config.daemon_id.clone(),
        }
    }

    pub fn create_session(
        &mut self,
        request: CreateSessionRequest,
    ) -> Result<RuntimeSession, DaemonError> {
        let session = RuntimeSession::new(
            self.store.next_session_id(),
            request.workspace_id,
            request.worktree_id,
            self.host_machine_id.clone(),
            self.host_daemon_id.clone(),
        );

        Ok(self.store.insert(session))
    }

    pub fn get_session(&self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        self.store
            .get(session_id)
            .cloned()
            .ok_or_else(|| DaemonError::SessionNotFound {
                session_id: session_id.to_string(),
            })
    }

    pub fn list_sessions(&self) -> Vec<RuntimeSession> {
        self.store.list()
    }

    pub fn transition_session(
        &mut self,
        session_id: &str,
        next_status: SessionStatus,
    ) -> Result<RuntimeSession, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;

        if !session.transition_to(next_status) {
            return Err(DaemonError::InvalidSessionTransition {
                session_id: session_id.to_string(),
                from: session.status(),
                to: next_status,
            });
        }

        Ok(session.clone())
    }

    pub fn end_session(&mut self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        self.transition_session(session_id, SessionStatus::Ended)
    }

    pub fn add_attachment_to_session(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<RuntimeSession, DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "attach")?;
        session.add_attachment(attachment_id);
        Ok(session.clone())
    }

    pub fn remove_attachment_from_session(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<(RuntimeSession, bool), DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "detach")?;
        let removed_was_controller = session.controller_attachment_id() == Some(attachment_id);

        if !session.remove_attachment(attachment_id) {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }

        Ok((session.clone(), removed_was_controller))
    }

    pub fn assign_controller(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<(RuntimeSession, Option<String>), DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "assign controller")?;

        if !session.has_attachment(attachment_id) {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }

        let previous = session.assign_controller(attachment_id);
        Ok((session.clone(), previous))
    }

    pub fn release_controller(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<(RuntimeSession, Option<String>), DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "release controller")?;

        if !session.has_attachment(attachment_id) {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }

        if session.controller_attachment_id() != Some(attachment_id) {
            return Err(DaemonError::AttachmentIsNotController {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }

        let previous = session.release_controller();
        Ok((session.clone(), previous))
    }

    pub fn set_active_provider_run(
        &mut self,
        session_id: &str,
        provider_run_id: Option<String>,
    ) -> Result<RuntimeSession, DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "set active provider run")?;

        session.set_active_provider_run(provider_run_id);

        let target_status = if session.active_provider_run_id().is_some() {
            SessionStatus::Active
        } else if session.status() == SessionStatus::Active {
            SessionStatus::Parked
        } else {
            session.status()
        };

        let _ = session.transition_to(target_status);

        Ok(session.clone())
    }

    pub fn store(&self) -> &SessionStore {
        &self.store
    }

    pub fn active_session_count(&self) -> usize {
        self.store.active_session_count()
    }

    fn get_session_mut_for_operation(
        &mut self,
        session_id: &str,
        operation: &'static str,
    ) -> Result<&mut RuntimeSession, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;

        if session.status() == SessionStatus::Ended {
            return Err(DaemonError::SessionOperationNotAllowed {
                session_id: session_id.to_string(),
                status: session.status(),
                operation,
            });
        }

        Ok(session)
    }
}

#[cfg(test)]
mod tests {
    use super::SessionService;
    use crate::config::DaemonConfig;
    use crate::error::DaemonError;
    use crate::session::{CreateSessionRequest, SessionStatus};

    fn test_config() -> DaemonConfig {
        DaemonConfig::for_tests()
    }

    #[test]
    fn creates_gets_and_lists_sessions() {
        let mut service = SessionService::new(&test_config());
        let created = service
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");

        assert_eq!(created.id(), "session-1");
        assert_eq!(created.workspace_id(), "workspace-1");
        assert_eq!(created.worktree_id(), "worktree-1");
        assert_eq!(created.host_machine_id(), "machine-test");
        assert_eq!(created.host_daemon_id(), "daemon-test");
        assert_eq!(created.status(), SessionStatus::Created);
        assert!(created.active_provider_run_id().is_none());
        assert!(created.attachment_ids().is_empty());
        assert!(created.controller_attachment_id().is_none());
        assert_eq!(service.active_session_count(), 1);

        let fetched = service
            .get_session(created.id())
            .expect("session lookup should succeed");
        assert_eq!(fetched, created);

        let listed = service.list_sessions();
        assert_eq!(listed, vec![created]);
    }

    #[test]
    fn ends_session_and_clears_runtime_ownership_fields() {
        let mut service = SessionService::new(&test_config());
        let created = service
            .create_session(CreateSessionRequest::new("workspace-2", "worktree-2"))
            .expect("session should be created");

        let active = service
            .transition_session(created.id(), SessionStatus::Active)
            .expect("session should become active");
        assert_eq!(active.status(), SessionStatus::Active);

        let ended = service
            .end_session(created.id())
            .expect("session should be ended");
        assert_eq!(ended.status(), SessionStatus::Ended);
        assert!(ended.active_provider_run_id().is_none());
        assert!(ended.attachment_ids().is_empty());
        assert!(ended.controller_attachment_id().is_none());
        assert_eq!(service.active_session_count(), 0);
    }

    #[test]
    fn rejects_invalid_session_transitions() {
        let mut service = SessionService::new(&test_config());
        let created = service
            .create_session(CreateSessionRequest::new("workspace-3", "worktree-3"))
            .expect("session should be created");

        let error = service
            .transition_session(created.id(), SessionStatus::Parked)
            .expect_err("created session cannot jump directly to parked");

        match error {
            DaemonError::InvalidSessionTransition {
                session_id,
                from,
                to,
            } => {
                assert_eq!(session_id, created.id());
                assert_eq!(from, SessionStatus::Created);
                assert_eq!(to, SessionStatus::Parked);
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn returns_not_found_for_unknown_session() {
        let service = SessionService::new(&test_config());

        let error = service
            .get_session("session-missing")
            .expect_err("missing session should return a structured error");

        match error {
            DaemonError::SessionNotFound { session_id } => {
                assert_eq!(session_id, "session-missing");
            }
            other => panic!("unexpected error: {other}"),
        }
    }
}
