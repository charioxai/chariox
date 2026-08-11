use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::error::DaemonError;

use super::{
    CreateSessionRequest, PromptQueueItem, RuntimeProject, RuntimeSession, SessionConfigState,
    SessionService,
};

#[derive(Debug, Clone)]
pub(crate) struct SessionStateStore {
    inner: Arc<RwLock<SessionService>>,
}

impl SessionStateStore {
    pub(crate) fn new(sessions: SessionService) -> Self {
        Self {
            inner: Arc::new(RwLock::new(sessions)),
        }
    }

    pub(crate) fn read(&self) -> RwLockReadGuard<'_, SessionService> {
        self.inner.read().expect("session state rwlock poisoned")
    }

    pub(crate) fn write(&self) -> RwLockWriteGuard<'_, SessionService> {
        self.inner.write().expect("session state rwlock poisoned")
    }

    pub(crate) fn snapshot(&self) -> SessionService {
        self.read().clone()
    }

    pub(crate) fn prompt_id_allocator(&self) -> super::PromptIdAllocator {
        self.read().prompt_id_allocator()
    }

    pub(crate) fn reserve_prompt_id(&self) -> String {
        self.read().reserve_prompt_id()
    }

    pub(crate) fn observe_prompt_number(&self, number: u64) {
        self.read().observe_prompt_number(number)
    }

    pub(crate) fn seed_prompt_ids_from_sessions(&self) {
        self.read().seed_prompt_ids_from_sessions()
    }

    pub(crate) fn get_session(&self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        self.read().get_session(session_id)
    }

    pub(crate) fn has_session(&self, session_id: &str) -> bool {
        self.read().has_session(session_id)
    }

    pub(crate) fn list_sessions(&self) -> Vec<RuntimeSession> {
        self.read().list_sessions()
    }

    pub(crate) fn list_non_ended_sessions_including_hidden(&self) -> Vec<RuntimeSession> {
        self.read().list_non_ended_sessions_including_hidden()
    }

    pub(crate) fn list_all_sessions(&self) -> Vec<RuntimeSession> {
        self.read().list_all_sessions()
    }

    pub(crate) fn list_projects(
        &self,
        owner_user_id: &str,
        include_archived: bool,
    ) -> Vec<RuntimeProject> {
        self.read().list_projects(owner_user_id, include_archived)
    }

    pub(crate) fn list_visible_projects(
        &self,
        caller_user_id: &str,
        include_archived: bool,
    ) -> Vec<RuntimeProject> {
        self.read()
            .list_visible_projects(caller_user_id, include_archived)
    }

    pub(crate) fn get_project(&self, project_id: &str) -> Result<RuntimeProject, DaemonError> {
        self.read().get_project(project_id)
    }

    pub(crate) fn sessions_in_project(&self, project_id: &str) -> Vec<RuntimeSession> {
        self.read().sessions_in_project(project_id)
    }

    pub(crate) fn durable_projects(&self) -> Vec<RuntimeProject> {
        self.read().durable_projects()
    }

    pub(crate) fn restore_projects(&self, projects: Vec<RuntimeProject>) {
        self.write().restore_projects(projects)
    }

    pub(crate) fn remove_projects_without_visible_sessions(&self) -> Vec<RuntimeProject> {
        self.write().remove_projects_without_visible_sessions()
    }

    pub(crate) fn rename_project(
        &self,
        project_id: &str,
        name: String,
        caller_user_id: &str,
    ) -> Result<RuntimeProject, DaemonError> {
        self.write()
            .rename_project(project_id, name, caller_user_id)
    }

    pub(crate) fn archive_project(
        &self,
        project_id: &str,
        caller_user_id: &str,
    ) -> Result<RuntimeProject, DaemonError> {
        self.write().archive_project(project_id, caller_user_id)
    }

    pub(crate) fn restore_project_status(
        &self,
        project_id: &str,
        caller_user_id: &str,
    ) -> Result<RuntimeProject, DaemonError> {
        self.write()
            .restore_project_status(project_id, caller_user_id)
    }

    pub(crate) fn delete_project_record(
        &self,
        project_id: &str,
        caller_user_id: &str,
    ) -> Result<RuntimeProject, DaemonError> {
        self.write()
            .delete_project_record(project_id, caller_user_id)
    }

    pub(crate) fn create_session(
        &self,
        request: CreateSessionRequest,
    ) -> Result<RuntimeSession, DaemonError> {
        self.write().create_session(request)
    }

    pub(crate) fn create_ephemeral_session(
        &self,
        request: CreateSessionRequest,
    ) -> Result<RuntimeSession, DaemonError> {
        self.write().create_ephemeral_session(request)
    }

    pub(crate) fn is_ephemeral_session(&self, session_id: &str) -> bool {
        self.read().is_ephemeral_session(session_id)
    }

    pub(crate) fn restore_session(&self, session: RuntimeSession) -> RuntimeSession {
        self.write().restore_session(session)
    }

    pub(crate) fn restore_session_with_default_project_name_hint(
        &self,
        session: RuntimeSession,
        default_project_name_hint: Option<&str>,
    ) -> RuntimeSession {
        self.write()
            .restore_session_with_default_project_name_hint(session, default_project_name_hint)
    }

    pub(crate) fn restore_ended_session(
        &self,
        session_id: &str,
    ) -> Result<RuntimeSession, DaemonError> {
        self.write()
            .transition_session(session_id, super::SessionStatus::Parked)
    }

    pub(crate) fn remove_restored_session(&self, session_id: &str) -> Option<RuntimeSession> {
        self.write().remove_restored_session(session_id)
    }

    pub(crate) fn replace_publication_runtime_workflows(
        &self,
        session_id: &str,
        workflows: Vec<super::WorkflowDefinition>,
        workflow_prompt_queues: Vec<super::WorkflowPromptQueueDefinition>,
        workflow_watchdogs: Vec<super::WorkflowWatchdogDefinition>,
    ) -> Result<RuntimeSession, DaemonError> {
        self.write().replace_publication_runtime_workflows(
            session_id,
            workflows,
            workflow_prompt_queues,
            workflow_watchdogs,
        )
    }

    pub(crate) fn end_session(&self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        self.write().end_session(session_id)
    }

    pub(crate) fn delete_session(&self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        self.write().delete_session(session_id)
    }

    pub(crate) fn delete_session_with_project_cleanup(
        &self,
        session_id: &str,
    ) -> Result<(RuntimeSession, Option<RuntimeProject>), DaemonError> {
        self.write().delete_session_with_project_cleanup(session_id)
    }

    pub(crate) fn set_active_provider_run(
        &self,
        session_id: &str,
        provider_run_id: Option<String>,
    ) -> Result<RuntimeSession, DaemonError> {
        self.write()
            .set_active_provider_run(session_id, provider_run_id)
    }

    pub(crate) fn mirror_agent_prompt_state(
        &self,
        session_id: &str,
        agent_id: &str,
        active_prompt: Option<PromptQueueItem>,
        queued_prompts: VecDeque<PromptQueueItem>,
    ) -> Result<RuntimeSession, DaemonError> {
        self.write()
            .mirror_agent_prompt_state(session_id, agent_id, active_prompt, queued_prompts)
    }

    pub(crate) fn note_prompt_sent(
        &self,
        session_id: &str,
        agent_id: &str,
        timestamp_ms: u64,
    ) -> Result<RuntimeSession, DaemonError> {
        self.write()
            .note_prompt_sent(session_id, agent_id, timestamp_ms)
    }

    pub(crate) fn note_agent_output_sequence(
        &self,
        session_id: &str,
        agent_id: &str,
        sequence: u64,
    ) -> Result<RuntimeSession, DaemonError> {
        self.write()
            .note_agent_output_sequence(session_id, agent_id, sequence)
    }

    pub(crate) fn record_workflow_node_thinking_trace_for_node_run(
        &self,
        session_id: &str,
        workflow_node_run_id: &str,
        message: impl Into<String>,
    ) -> Result<Option<RuntimeSession>, DaemonError> {
        self.write()
            .record_workflow_node_thinking_trace_for_node_run(
                session_id,
                workflow_node_run_id,
                message,
            )
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SessionStateReader {
    store: SessionStateStore,
}

impl SessionStateReader {
    pub(crate) fn new(store: SessionStateStore) -> Self {
        Self { store }
    }

    pub(crate) fn get_session(&self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        self.store.read().get_session(session_id)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SessionStateOwner {
    store: SessionStateStore,
}

impl SessionStateOwner {
    pub(crate) fn new(store: SessionStateStore) -> Self {
        Self { store }
    }

    pub(crate) fn create_session(
        &mut self,
        request: CreateSessionRequest,
    ) -> Result<RuntimeSession, DaemonError> {
        self.store.write().create_session(request)
    }

    pub(crate) fn assign_session_alias(
        &mut self,
        session_id: &str,
        alias: String,
    ) -> Result<RuntimeSession, DaemonError> {
        self.store.write().assign_session_alias(session_id, alias)
    }

    pub(crate) fn update_config(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        values: BTreeMap<String, String>,
        requires_idle: bool,
    ) -> Result<(RuntimeSession, SessionConfigState), DaemonError> {
        self.store
            .write()
            .update_config(session_id, attachment_id, values, requires_idle)
    }

    pub(crate) fn end_session(&mut self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        self.store.write().end_session(session_id)
    }

    pub(crate) fn set_active_provider_run(
        &mut self,
        session_id: &str,
        provider_run_id: Option<String>,
    ) -> Result<RuntimeSession, DaemonError> {
        self.store
            .write()
            .set_active_provider_run(session_id, provider_run_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_session_reads_can_hold_the_store_concurrently() {
        let store = SessionStateStore::new(SessionService::new(&crate::DaemonConfig::for_tests()));
        let first_reader = store.read();

        let second_reader = store
            .inner
            .try_read()
            .expect("a read must not exclude another read");

        drop(second_reader);
        drop(first_reader);
        assert!(store.inner.try_write().is_ok());
    }
}
