use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use crate::error::DaemonError;
use crate::runtime::workspace_coordinator::WorkspaceOperationClaimSnapshot;
use crate::session::{PromptQueueItem, RuntimeSession, SessionStatus};

use super::{
    AgentRuntimeProjectionStore, ProjectionInvariantHealthSnapshot, ProjectionInvariantMismatch,
    SessionProjectionHealthSnapshot, WorkspaceCoordinationHealthSnapshot, WorktreeClaimSnapshot,
};

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
        state.session_list = Some(sessions);
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
        let state = self
            .state
            .lock()
            .expect("session projection lock should not be poisoned");
        let sessions = state
            .session_list
            .as_ref()
            .cloned()
            .unwrap_or_else(|| state.session_states.values().cloned().collect());
        workspace_coordination_snapshot(sessions, active_operation_claims)
    }

    pub(crate) fn invariant_snapshot(
        &self,
        agent_runtime: &AgentRuntimeProjectionStore,
    ) -> ProjectionInvariantHealthSnapshot {
        let sessions = self.projected_sessions();
        let mut agent_projections = agent_runtime
            .list()
            .into_iter()
            .map(|projection| (projection.agent_id.clone(), projection))
            .collect::<BTreeMap<_, _>>();
        let mut checked_agents = 0;
        let mut mismatches = Vec::new();

        for session in &sessions {
            let mut expected_prompt_states = BTreeMap::new();
            for agent in session.agents() {
                let prompt_state = session.prompt_states().get(agent.id());
                expected_prompt_states.insert(
                    agent.id().to_string(),
                    (
                        prompt_state.and_then(|state| state.active_prompt().cloned()),
                        prompt_state.and_then(|state| state.queued_prompts().front().cloned()),
                        prompt_state
                            .map(|state| state.queued_prompts().len())
                            .unwrap_or(0),
                    ),
                );
            }
            for (agent_id, prompt_state) in session.prompt_states() {
                expected_prompt_states
                    .entry(agent_id.clone())
                    .or_insert_with(|| {
                        (
                            prompt_state.active_prompt().cloned(),
                            prompt_state.queued_prompts().front().cloned(),
                            prompt_state.queued_prompts().len(),
                        )
                    });
            }

            for (agent_id, (active_prompt, next_queued_prompt, queued_prompt_count)) in
                expected_prompt_states
            {
                checked_agents += 1;
                let Some(projection) = agent_projections.remove(&agent_id) else {
                    mismatches.push(ProjectionInvariantMismatch {
                        kind: "missing_agent_runtime_projection".to_string(),
                        session_id: session.id().to_string(),
                        agent_id: Some(agent_id),
                        details: "session projection has no matching agent runtime projection"
                            .to_string(),
                    });
                    continue;
                };
                if projection.session_id != session.id() {
                    mismatches.push(ProjectionInvariantMismatch {
                        kind: "agent_runtime_session_mismatch".to_string(),
                        session_id: session.id().to_string(),
                        agent_id: Some(agent_id.clone()),
                        details: format!(
                            "agent runtime projection points at session {}",
                            projection.session_id
                        ),
                    });
                }
                if projection.active_prompt != active_prompt {
                    mismatches.push(ProjectionInvariantMismatch {
                        kind: "active_prompt_mismatch".to_string(),
                        session_id: session.id().to_string(),
                        agent_id: Some(agent_id.clone()),
                        details: format!(
                            "session active {}, agent runtime active {}",
                            describe_projected_prompt(&active_prompt),
                            describe_projected_prompt(&projection.active_prompt)
                        ),
                    });
                }
                if projection.next_queued_prompt != next_queued_prompt {
                    mismatches.push(ProjectionInvariantMismatch {
                        kind: "queue_front_mismatch".to_string(),
                        session_id: session.id().to_string(),
                        agent_id: Some(agent_id.clone()),
                        details: format!(
                            "session queue front {}, agent runtime queue front {}",
                            describe_projected_prompt(&next_queued_prompt),
                            describe_projected_prompt(&projection.next_queued_prompt)
                        ),
                    });
                }
                if projection.queued_prompt_count != queued_prompt_count {
                    mismatches.push(ProjectionInvariantMismatch {
                        kind: "queued_prompt_count_mismatch".to_string(),
                        session_id: session.id().to_string(),
                        agent_id: Some(agent_id),
                        details: format!(
                            "session queued count {}, agent runtime queued count {}",
                            queued_prompt_count, projection.queued_prompt_count
                        ),
                    });
                }
            }
        }

        for projection in agent_projections.into_values() {
            mismatches.push(ProjectionInvariantMismatch {
                kind: "orphaned_agent_runtime_projection".to_string(),
                session_id: projection.session_id.clone(),
                agent_id: Some(projection.agent_id.clone()),
                details: "agent runtime projection has no matching projected session agent"
                    .to_string(),
            });
        }

        ProjectionInvariantHealthSnapshot {
            checked_sessions: sessions.len(),
            checked_agents,
            mismatches,
        }
    }

    pub(crate) fn resolve_session_ref_id(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Option<String> {
        let normalized_ref = session_ref.trim().to_lowercase();
        if normalized_ref.is_empty() {
            return None;
        }
        let sessions = self.projected_sessions();
        let visible_sessions = sessions
            .iter()
            .filter(|session| session.status() != SessionStatus::Ended)
            .collect::<Vec<_>>();
        let workspace_sessions = visible_sessions
            .iter()
            .copied()
            .filter(|session| {
                workspace_id.is_none_or(|workspace| session.workspace_id() == workspace)
            })
            .collect::<Vec<_>>();

        if let Some(session) = visible_sessions
            .iter()
            .find(|session| session.id() == normalized_ref)
        {
            return Some(session.id().to_string());
        }
        if let Some(session) = workspace_sessions
            .iter()
            .find(|session| session.alias() == Some(normalized_ref.as_str()))
        {
            return Some(session.id().to_string());
        }

        let id_matches = visible_sessions
            .iter()
            .filter(|session| session.id().starts_with(&normalized_ref))
            .collect::<Vec<_>>();
        if id_matches.len() == 1 {
            return Some(id_matches[0].id().to_string());
        }

        let alias_matches = workspace_sessions
            .iter()
            .filter(|session| {
                session
                    .alias()
                    .is_some_and(|alias| alias.starts_with(normalized_ref.as_str()))
            })
            .collect::<Vec<_>>();
        if alias_matches.len() == 1 {
            return Some(alias_matches[0].id().to_string());
        }

        None
    }

    pub(crate) fn resolve_session_ref_id_from_warmed_list(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Option<Result<String, DaemonError>> {
        if !self.has_warmed_list() {
            return None;
        }
        Some(resolve_session_ref_id_from_sessions(
            session_ref,
            workspace_id,
            self.projected_sessions(),
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

    fn projected_sessions(&self) -> Vec<RuntimeSession> {
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

fn resolve_session_ref_id_from_sessions(
    session_ref: &str,
    workspace_id: Option<&str>,
    sessions: Vec<RuntimeSession>,
) -> Result<String, DaemonError> {
    let normalized_ref = session_ref.trim().to_lowercase();
    if normalized_ref.is_empty() {
        return Err(DaemonError::SessionNotFound {
            session_id: normalized_ref,
        });
    }
    let visible_sessions = sessions
        .iter()
        .filter(|session| session.status() != SessionStatus::Ended)
        .collect::<Vec<_>>();
    let workspace_sessions = visible_sessions
        .iter()
        .copied()
        .filter(|session| workspace_id.is_none_or(|workspace| session.workspace_id() == workspace))
        .collect::<Vec<_>>();

    if let Some(session) = visible_sessions
        .iter()
        .find(|session| session.id() == normalized_ref)
    {
        return Ok(session.id().to_string());
    }
    if let Some(session) = workspace_sessions
        .iter()
        .find(|session| session.alias() == Some(normalized_ref.as_str()))
    {
        return Ok(session.id().to_string());
    }

    let id_matches = visible_sessions
        .iter()
        .filter(|session| session.id().starts_with(&normalized_ref))
        .collect::<Vec<_>>();
    if id_matches.len() == 1 {
        return Ok(id_matches[0].id().to_string());
    }
    if id_matches.len() > 1 {
        return Err(DaemonError::AmbiguousSessionRef {
            session_ref: normalized_ref,
            matches: id_matches
                .into_iter()
                .map(|session| describe_projected_session_match(session))
                .collect(),
        });
    }

    let alias_matches = workspace_sessions
        .iter()
        .filter(|session| {
            session
                .alias()
                .is_some_and(|alias| alias.starts_with(normalized_ref.as_str()))
        })
        .collect::<Vec<_>>();
    if alias_matches.len() == 1 {
        return Ok(alias_matches[0].id().to_string());
    }
    if alias_matches.len() > 1 {
        return Err(DaemonError::AmbiguousSessionRef {
            session_ref: normalized_ref,
            matches: alias_matches
                .into_iter()
                .map(|session| describe_projected_session_match(session))
                .collect(),
        });
    }

    Err(DaemonError::SessionNotFound {
        session_id: normalized_ref,
    })
}

fn describe_projected_session_match(session: &RuntimeSession) -> String {
    match session.alias() {
        Some(alias) => format!("{} ({alias})", session.id()),
        None => session.id().to_string(),
    }
}

fn describe_projected_prompt(prompt: &Option<PromptQueueItem>) -> String {
    prompt
        .as_ref()
        .map(|prompt| prompt.id().to_string())
        .unwrap_or_else(|| "none".to_string())
}

fn upsert_session(session_list: &mut Option<Vec<RuntimeSession>>, session: RuntimeSession) {
    let Some(session_list) = session_list.as_mut() else {
        return;
    };
    if let Some(existing) = session_list
        .iter_mut()
        .find(|existing| existing.id() == session.id())
    {
        *existing = session;
    } else {
        session_list.push(session);
    }
}

fn workspace_coordination_snapshot(
    sessions: Vec<RuntimeSession>,
    active_operation_claims: Vec<WorkspaceOperationClaimSnapshot>,
) -> WorkspaceCoordinationHealthSnapshot {
    let mut claims_by_worktree: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for session in sessions {
        if session.status() == crate::session::SessionStatus::Ended {
            continue;
        }
        claims_by_worktree
            .entry((
                session.workspace_id().to_string(),
                session.worktree_id().to_string(),
            ))
            .or_default()
            .push(session.id().to_string());
    }

    let mut active_worktree_claims = Vec::new();
    let mut worktree_collisions = Vec::new();
    for ((workspace_id, worktree_id), mut session_ids) in claims_by_worktree {
        session_ids.sort();
        let claim = WorktreeClaimSnapshot {
            workspace_id,
            worktree_id,
            session_ids,
        };
        if claim.session_ids.len() > 1 {
            worktree_collisions.push(claim.clone());
        }
        active_worktree_claims.push(claim);
    }

    WorkspaceCoordinationHealthSnapshot {
        active_worktree_claims,
        worktree_collisions,
        active_operation_claims,
    }
}

#[cfg(test)]
mod tests {
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::runtime::projection::test_support::{launch_dev_stub_provider, submit_prompt};
    use crate::runtime::projection::{AgentRuntimeProjectionStore, SessionStateProjectionStore};
    use crate::session::CreateSessionRequest;
    use crate::{DaemonApp, DaemonConfig};

    #[test]
    fn projection_invariant_health_reports_agent_runtime_drift() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                &session_id,
                "cli-projection-invariant",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        launch_dev_stub_provider(&mut app, &session_id, &agent_id);
        submit_prompt(
            &mut app,
            &session_id,
            attachment.id(),
            &agent_id,
            "active prompt",
        );
        submit_prompt(
            &mut app,
            &session_id,
            attachment.id(),
            &agent_id,
            "queued prompt",
        );

        let session = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(&session_id)
            .expect("session snapshot should load");
        let session_store = SessionStateProjectionStore::default();
        let agent_store = AgentRuntimeProjectionStore::default();
        session_store.update(session.clone());
        agent_store.update_session(&session);
        let clean_snapshot = session_store.invariant_snapshot(&agent_store);
        assert_eq!(clean_snapshot.checked_sessions, 1);
        assert_eq!(clean_snapshot.checked_agents, 1);
        assert!(clean_snapshot.mismatches.is_empty());

        let projection = agent_store
            .get(&agent_id)
            .expect("agent projection should exist before drift injection");
        agent_store.update_agent_prompt_state(
            &session_id,
            &agent_id,
            projection.active_prompt,
            None,
            0,
        );

        let drift_snapshot = session_store.invariant_snapshot(&agent_store);
        assert!(drift_snapshot
            .mismatches
            .iter()
            .any(|mismatch| mismatch.kind == "queue_front_mismatch"));
        assert!(drift_snapshot
            .mismatches
            .iter()
            .any(|mismatch| mismatch.kind == "queued_prompt_count_mismatch"));
    }

    #[test]
    fn workspace_coordination_snapshot_reports_worktree_collisions() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (first, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-1", "shared-worktree"))
            .expect("first session should be created");
        let (second, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-1", "shared-worktree"))
            .expect("second session should be created");
        let (other_workspace, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-2", "shared-worktree"))
            .expect("other workspace session should be created");

        let store = SessionStateProjectionStore::default();
        store.update_list(vec![first.clone(), second.clone(), other_workspace]);

        let snapshot = store.workspace_coordination_snapshot(Vec::new());
        assert_eq!(snapshot.active_worktree_claims.len(), 2);
        assert_eq!(snapshot.worktree_collisions.len(), 1);
        assert!(snapshot.active_operation_claims.is_empty());
        assert_eq!(
            snapshot.worktree_collisions[0].session_ids,
            vec![first.id().to_string(), second.id().to_string()]
        );
    }
}
