use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use crate::error::DaemonError;
use crate::runtime::workspace_coordinator::WorkspaceOperationClaimSnapshot;
use crate::session::RuntimeSession;

use super::{
    AgentRuntimeProjectionStore, ProjectionInvariantHealthSnapshot,
    SessionProjectionHealthSnapshot, WorkspaceCoordinationHealthSnapshot,
};

mod invariant;
mod resolution;
mod workspace_coordination;

#[derive(Clone, Default)]
pub(crate) struct SessionStateProjectionStore {
    state: Arc<StdMutex<SessionProjectionState>>,
}

#[derive(Default)]
struct SessionProjectionState {
    session_states: HashMap<String, RuntimeSession>,
    session_list: Option<Vec<RuntimeSession>>,
}

impl SessionStateProjectionStore {
    pub(crate) fn get(&self, session_id: &str) -> Option<RuntimeSession> {
        self.state
            .lock()
            .expect("session projection lock should not be poisoned")
            .session_states
            .get(session_id)
            .cloned()
    }

    pub(crate) fn list(&self) -> Option<Vec<RuntimeSession>> {
        self.state
            .lock()
            .expect("session projection lock should not be poisoned")
            .session_list
            .clone()
    }

    pub(crate) fn has_warmed_list(&self) -> bool {
        self.state
            .lock()
            .expect("session projection lock should not be poisoned")
            .session_list
            .is_some()
    }

    pub(crate) fn update(&self, session: RuntimeSession) {
        let mut state = self
            .state
            .lock()
            .expect("session projection lock should not be poisoned");
        upsert_session(&mut state.session_list, session.clone());
        state
            .session_states
            .insert(session.id().to_string(), session);
    }

    pub(crate) fn update_list(&self, sessions: Vec<RuntimeSession>) {
        let mut state = self
            .state
            .lock()
            .expect("session projection lock should not be poisoned");
        for session in &sessions {
            state
                .session_states
                .insert(session.id().to_string(), session.clone());
        }
        state.session_list = Some(
            sessions
                .into_iter()
                .filter(|session| !session.is_hidden())
                .collect(),
        );
    }

    pub(crate) fn remove(&self, session_id: &str) {
        let mut state = self
            .state
            .lock()
            .expect("session projection lock should not be poisoned");
        state.session_states.remove(session_id);
        if let Some(session_list) = state.session_list.as_mut() {
            session_list.retain(|session| session.id() != session_id);
        }
    }

    pub(crate) fn health_snapshot(&self) -> SessionProjectionHealthSnapshot {
        let state = self
            .state
            .lock()
            .expect("session projection lock should not be poisoned");
        SessionProjectionHealthSnapshot {
            projected_sessions: state.session_states.len(),
            projected_session_list_entries: state.session_list.as_ref().map(Vec::len),
            active_prompts: 0,
            queued_prompts: 0,
        }
    }

    pub(crate) fn workspace_coordination_snapshot(
        &self,
        active_operation_claims: Vec<WorkspaceOperationClaimSnapshot>,
    ) -> WorkspaceCoordinationHealthSnapshot {
        workspace_coordination::snapshot(self.projected_sessions(), active_operation_claims)
    }

    pub(crate) fn invariant_snapshot(
        &self,
        agent_runtime: &AgentRuntimeProjectionStore,
        canonical_agents: &[crate::agent::AgentInstance],
    ) -> ProjectionInvariantHealthSnapshot {
        invariant::snapshot(self.projected_sessions(), agent_runtime, canonical_agents)
    }

    pub(crate) fn resolve_session_ref_id(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Option<String> {
        resolution::resolve_session_ref_id(self.projected_sessions(), session_ref, workspace_id)
    }

    pub(crate) fn resolve_session_ref_id_from_warmed_list(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Option<Result<String, DaemonError>> {
        if !self.has_warmed_list() {
            return None;
        }
        Some(resolution::resolve_session_ref_id_from_sessions(
            self.projected_sessions(),
            session_ref,
            workspace_id,
        ))
    }

    pub(crate) fn resolve_session_ref(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Option<RuntimeSession> {
        let session_id = self.resolve_session_ref_id(session_ref, workspace_id)?;
        self.get(&session_id)
    }

    pub(crate) fn session_id_for_attachment(&self, attachment_id: &str) -> Option<String> {
        self.projected_sessions()
            .into_iter()
            .find(|session| session.has_attachment(attachment_id))
            .map(|session| session.id().to_string())
    }

    pub(crate) fn projected_sessions(&self) -> Vec<RuntimeSession> {
        let state = self
            .state
            .lock()
            .expect("session projection lock should not be poisoned");
        state
            .session_list
            .as_ref()
            .cloned()
            .unwrap_or_else(|| state.session_states.values().cloned().collect())
    }
}

fn upsert_session(session_list: &mut Option<Vec<RuntimeSession>>, session: RuntimeSession) {
    let Some(session_list) = session_list.as_mut() else {
        return;
    };
    if session.is_hidden() {
        session_list.retain(|existing| existing.id() != session.id());
        return;
    }
    if let Some(existing) = session_list
        .iter_mut()
        .find(|existing| existing.id() == session.id())
    {
        *existing = session;
    } else {
        session_list.push(session);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str) -> RuntimeSession {
        RuntimeSession::new(id, None, "workspace", "worktree", "machine", "daemon")
    }

    #[test]
    fn hidden_sessions_remain_addressable_but_do_not_enter_warmed_list() {
        let store = SessionStateProjectionStore::default();
        let visible = session("visible");
        store.update_list(vec![visible.clone()]);

        let mut hidden = session("hidden");
        hidden.set_hidden(true);
        store.update(hidden.clone());

        assert_eq!(store.get("hidden"), Some(hidden));
        assert_eq!(store.list(), Some(vec![visible]));
    }

    #[test]
    fn hidden_sessions_are_filtered_when_list_is_warmed_from_mixed_snapshot() {
        let store = SessionStateProjectionStore::default();
        let visible = session("visible");
        let mut hidden = session("hidden");
        hidden.set_hidden(true);

        store.update_list(vec![visible.clone(), hidden.clone()]);

        assert_eq!(store.get("hidden"), Some(hidden));
        assert_eq!(store.list(), Some(vec![visible]));
    }
}
