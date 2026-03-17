use std::collections::BTreeMap;

use crate::config::DaemonConfig;
use crate::error::DaemonError;

use super::{
    CreateSessionRequest, PromptDetachEffect, PromptQueueItem, PromptSubmissionOutcome,
    RuntimeSession, SessionConfigState, SessionStatus, SessionStore,
};

#[derive(Debug, Clone)]
pub struct SessionService {
    store: SessionStore,
    host_machine_id: String,
    host_daemon_id: String,
    next_prompt_number: u64,
}

impl SessionService {
    pub fn new(config: &DaemonConfig) -> Self {
        Self {
            store: SessionStore::new(),
            host_machine_id: config.host_machine_id.clone(),
            host_daemon_id: config.daemon_id.clone(),
            next_prompt_number: 0,
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
    ) -> Result<(RuntimeSession, PromptDetachEffect), DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "detach")?;

        if !session.remove_attachment(attachment_id) {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }

        let removed_active_prompt = session
            .active_prompt()
            .map(|prompt| prompt.source_attachment_id() == attachment_id)
            .unwrap_or(false);
        if removed_active_prompt {
            let _ = session.complete_active_prompt_only();
        }
        let removed_queued_prompt_count =
            session.remove_queued_prompts_by_attachment(attachment_id);

        Ok((
            session.clone(),
            PromptDetachEffect {
                removed_active_prompt,
                removed_queued_prompt_count,
            },
        ))
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

    pub fn submit_prompt(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        prompt: impl Into<String>,
    ) -> Result<(RuntimeSession, PromptSubmissionOutcome), DaemonError> {
        let prompt_id = self.next_prompt_id();
        let prompt = PromptQueueItem::new(
            prompt_id,
            attachment_id,
            prompt,
            super::PromptStatus::Queued,
        );
        let session = self.get_session_mut_for_operation(session_id, "submit prompt")?;

        if !session.has_attachment(attachment_id) {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }

        let outcome = session.submit_prompt(prompt);
        Ok((session.clone(), outcome))
    }

    pub fn cancel_active_prompt(
        &mut self,
        session_id: &str,
        prompt_id: &str,
    ) -> Result<RuntimeSession, DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "cancel prompt")?;
        let _ = session.clear_active_prompt_if(prompt_id);
        Ok(session.clone())
    }

    pub fn complete_active_prompt(
        &mut self,
        session_id: &str,
    ) -> Result<(RuntimeSession, super::PromptQueueItem), DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "complete prompt")?;
        let completed =
            session
                .complete_active_prompt_only()
                .ok_or_else(|| DaemonError::NoActivePrompt {
                    session_id: session_id.to_string(),
                })?;
        Ok((session.clone(), completed))
    }

    pub fn peek_next_queued_prompt(
        &self,
        session_id: &str,
    ) -> Result<Option<super::PromptQueueItem>, DaemonError> {
        let session = self.get_session(session_id)?;
        Ok(session.peek_next_queued_prompt())
    }

    pub fn activate_next_queued_prompt(
        &mut self,
        session_id: &str,
    ) -> Result<(RuntimeSession, Option<super::PromptQueueItem>), DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "activate next prompt")?;
        let next = session
            .pop_next_queued_prompt()
            .map(|prompt| session.activate_prompt(prompt));
        Ok((session.clone(), next))
    }

    pub fn activate_prompt(
        &mut self,
        session_id: &str,
        prompt: super::PromptQueueItem,
    ) -> Result<(RuntimeSession, super::PromptQueueItem), DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "activate prompt")?;
        let active = session.activate_prompt(prompt);
        Ok((session.clone(), active))
    }

    pub fn pop_next_queued_prompt(
        &mut self,
        session_id: &str,
    ) -> Result<(RuntimeSession, Option<super::PromptQueueItem>), DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "pop next prompt")?;
        let next = session.pop_next_queued_prompt();
        Ok((session.clone(), next))
    }

    pub fn update_config(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        values: BTreeMap<String, String>,
        requires_idle: bool,
    ) -> Result<(RuntimeSession, SessionConfigState), DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "update config")?;

        if !session.has_attachment(attachment_id) {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }

        if requires_idle && session.active_prompt().is_some() {
            return Err(DaemonError::ConfigChangeRejectedWhilePromptRunning {
                session_id: session_id.to_string(),
            });
        }

        session.apply_config_changes(values, attachment_id);
        Ok((session.clone(), session.config_state().clone()))
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

    fn next_prompt_id(&mut self) -> String {
        self.next_prompt_number += 1;
        format!("prompt-{}", self.next_prompt_number)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::SessionService;
    use crate::config::DaemonConfig;
    use crate::error::DaemonError;
    use crate::session::{
        CreateSessionRequest, PromptSubmissionOutcome, SchedulerState, SessionStatus,
        WorktreeIsolationMode,
    };

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
        assert!(created.active_prompt().is_none());
        assert!(created.queued_prompts().is_empty());
        assert_eq!(created.scheduler_state(), SchedulerState::Idle);
        assert_eq!(created.config_state().version(), 0);
        assert_eq!(created.worktree_assignments().len(), 1);
        assert_eq!(
            created.worktree_assignments()[0].isolation_mode(),
            WorktreeIsolationMode::SharedSession
        );
        assert_eq!(service.active_session_count(), 1);

        let fetched = service
            .get_session(created.id())
            .expect("lookup should succeed");
        assert_eq!(fetched, created);
        assert_eq!(service.list_sessions(), vec![created]);
    }

    #[test]
    fn prompt_queue_starts_then_queues_then_advances() {
        let mut service = SessionService::new(&test_config());
        let created = service
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        service
            .add_attachment_to_session(created.id(), "attachment-1")
            .expect("attachment should be added");
        service
            .add_attachment_to_session(created.id(), "attachment-2")
            .expect("attachment should be added");

        let (_, first) = service
            .submit_prompt(created.id(), "attachment-1", "first prompt")
            .expect("first prompt should start");
        let (_, second) = service
            .submit_prompt(created.id(), "attachment-2", "second prompt")
            .expect("second prompt should queue");

        match first {
            PromptSubmissionOutcome::Started { prompt } => assert_eq!(prompt.id(), "prompt-1"),
            _ => panic!("expected running prompt"),
        }
        match second {
            PromptSubmissionOutcome::Queued { prompt } => assert_eq!(prompt.id(), "prompt-2"),
            _ => panic!("expected queued prompt"),
        }

        assert_eq!(
            service
                .get_session(created.id())
                .expect("session should exist")
                .scheduler_state(),
            SchedulerState::Waiting
        );

        let (_session, completed) = service
            .complete_active_prompt(created.id())
            .expect("active prompt should complete");
        assert_eq!(completed.id(), "prompt-1");
        let (session, started_next) = service
            .activate_next_queued_prompt(created.id())
            .expect("next prompt should activate");
        assert_eq!(
            started_next.expect("next prompt should start").id(),
            "prompt-2"
        );
        assert_eq!(
            session.active_prompt().expect("active prompt exists").id(),
            "prompt-2"
        );
        assert_eq!(session.scheduler_state(), SchedulerState::Running);
    }

    #[test]
    fn config_updates_are_versioned() {
        let mut service = SessionService::new(&test_config());
        let created = service
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        service
            .add_attachment_to_session(created.id(), "attachment-1")
            .expect("attachment should be added");

        let mut changes = BTreeMap::new();
        changes.insert("theme".to_string(), "compact".to_string());
        let (_, config) = service
            .update_config(created.id(), "attachment-1", changes, false)
            .expect("config should update");

        assert_eq!(config.version(), 1);
        assert_eq!(
            config.values().get("theme").map(String::as_str),
            Some("compact")
        );
        assert_eq!(config.updated_by_attachment_id(), Some("attachment-1"));
    }

    #[test]
    fn rejects_idle_required_config_update_while_prompt_running() {
        let mut service = SessionService::new(&test_config());
        let created = service
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        service
            .add_attachment_to_session(created.id(), "attachment-1")
            .expect("attachment should be added");
        service
            .submit_prompt(created.id(), "attachment-1", "first prompt")
            .expect("prompt should start");

        let error = service
            .update_config(created.id(), "attachment-1", BTreeMap::new(), true)
            .expect_err("idle-required config change should be rejected");

        match error {
            DaemonError::ConfigChangeRejectedWhilePromptRunning { session_id } => {
                assert_eq!(session_id, created.id())
            }
            other => panic!("unexpected error: {other}"),
        }
    }
}
