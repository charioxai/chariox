use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::error::DaemonError;

use super::{
    CreateSessionRequest, PromptQueueItem, RuntimeSession, SessionConfigState, SessionService,
};

#[derive(Debug, Clone)]
pub(crate) struct SessionStateStore {
    inner: Arc<Mutex<SessionService>>,
}

impl SessionStateStore {
    pub(crate) fn new(sessions: SessionService) -> Self {
        Self {
            inner: Arc::new(Mutex::new(sessions)),
        }
    }

    pub(crate) fn read(&self) -> MutexGuard<'_, SessionService> {
        self.inner.lock().expect("session state mutex poisoned")
    }

    pub(crate) fn write(&self) -> MutexGuard<'_, SessionService> {
        self.inner.lock().expect("session state mutex poisoned")
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

    pub(crate) fn get_session(&self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        self.read().get_session(session_id)
    }

    pub(crate) fn list_sessions(&self) -> Vec<RuntimeSession> {
        self.read().list_sessions()
    }

    pub(crate) fn create_session(
        &self,
        request: CreateSessionRequest,
    ) -> Result<RuntimeSession, DaemonError> {
        self.write().create_session(request)
    }

    pub(crate) fn restore_session(&self, session: RuntimeSession) -> RuntimeSession {
        self.write().restore_session(session)
    }

    pub(crate) fn end_session(&self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        self.write().end_session(session_id)
    }

    pub(crate) fn delete_session(&self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        self.write().delete_session(session_id)
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

    pub(crate) fn list_sessions(&self) -> Vec<RuntimeSession> {
        self.store.read().list_sessions()
    }

    pub(crate) fn resolve_session_ref(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<RuntimeSession, DaemonError> {
        self.store
            .read()
            .resolve_session_ref(session_ref, workspace_id)
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
