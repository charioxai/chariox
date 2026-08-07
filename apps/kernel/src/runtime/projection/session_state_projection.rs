use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::{Mutex as StdMutex, RwLock as StdRwLock};

use tokio::sync::Notify;

use crate::error::DaemonError;
use crate::runtime::workspace_coordinator::WorkspaceOperationClaimSnapshot;
use crate::session::{PromptQueueItem, RuntimeSession, SessionStatus};

use super::{
    AgentActiveTurnProjection, AgentPromptRuntimeStatus, AgentRuntimeActivity,
    AgentRuntimeProjectionStore, AgentRuntimeStatus, AgentTurnRuntimePhase,
    ProjectionInvariantHealthSnapshot, SessionProjectionHealthSnapshot,
    WorkspaceCoordinationHealthSnapshot,
};

mod invariant;
mod resolution;
mod workspace_coordination;

#[derive(Clone, Default)]
pub(crate) struct SessionStateProjectionStore {
    state: Arc<StdRwLock<SessionProjectionState>>,
    changes: Arc<SessionProjectionChangeSignal>,
    session_changes: Arc<StdMutex<HashMap<String, Arc<SessionProjectionChangeSignal>>>>,
}

#[derive(Default)]
struct SessionProjectionState {
    session_states: HashMap<String, Arc<RuntimeSession>>,
    session_list: Option<Arc<[Arc<RuntimeSession>]>>,
    // Read-model-only provider activity. This must never feed RuntimeSession,
    // PromptStateOwner, prompt admission, or prompt settlement authority.
    external_observed_active_prompts: HashMap<(String, String, String), PromptQueueItem>,
}

impl SessionStateProjectionStore {
    pub(crate) fn get(&self, session_id: &str) -> Option<RuntimeSession> {
        self.get_shared(session_id)
            .map(|session| session.as_ref().clone())
    }

    pub(crate) fn list(&self) -> Option<Vec<RuntimeSession>> {
        self.list_shared().map(|sessions| {
            sessions
                .iter()
                .map(|session| session.as_ref().clone())
                .collect()
        })
    }

    pub(crate) fn get_shared(&self, session_id: &str) -> Option<Arc<RuntimeSession>> {
        self.state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .session_states
            .get(session_id)
            .cloned()
    }

    pub(crate) fn list_shared(&self) -> Option<Arc<[Arc<RuntimeSession>]>> {
        self.state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .session_list
            .clone()
    }

    pub(crate) fn has_warmed_list(&self) -> bool {
        self.state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .session_list
            .is_some()
    }

    pub(crate) fn update(&self, session: RuntimeSession) {
        let session_id = session.id().to_string();
        let session = Arc::new(session);
        let changed = {
            let mut state = self
                .state
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let external_activity_changed =
                prune_external_observed_activity(&mut state, session.as_ref());
            let list_changed = upsert_session(&mut state.session_list, session.clone());
            let session_changed = state
                .session_states
                .get(&session_id)
                .is_none_or(|existing| existing.as_ref() != session.as_ref());
            if session_changed {
                state.session_states.insert(session_id.clone(), session);
            }
            list_changed || session_changed || external_activity_changed
        };
        if !changed {
            return;
        }
        self.changes.record_change();
        self.session_change_signal(&session_id).record_change();
    }

    pub(crate) fn update_list(&self, sessions: Vec<RuntimeSession>) {
        let sessions = sessions.into_iter().map(Arc::new).collect::<Vec<_>>();
        let changed_session_ids = sessions
            .iter()
            .map(|session| session.id().to_string())
            .collect::<Vec<_>>();
        let changed = {
            let mut state = self
                .state
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut changed = false;
            for session in &sessions {
                changed |= prune_external_observed_activity(&mut state, session.as_ref());
                if state
                    .session_states
                    .get(session.id())
                    .is_none_or(|existing| existing.as_ref() != session.as_ref())
                {
                    state
                        .session_states
                        .insert(session.id().to_string(), session.clone());
                    changed = true;
                }
            }
            let session_list = Arc::<[Arc<RuntimeSession>]>::from(
                sessions
                    .into_iter()
                    .filter(|session| !session.is_hidden())
                    .collect::<Vec<_>>(),
            );
            if state.session_list.as_deref() != Some(session_list.as_ref()) {
                state.session_list = Some(session_list);
                changed = true;
            }
            changed
        };
        if !changed {
            return;
        }
        self.changes.record_change();
        for session_id in changed_session_ids {
            self.session_change_signal(&session_id).record_change();
        }
    }

    pub(crate) fn remove(&self, session_id: &str) {
        {
            let mut state = self
                .state
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.session_states.remove(session_id);
            state
                .external_observed_active_prompts
                .retain(|(projected_session_id, _, _), _| projected_session_id != session_id);
            if let Some(session_list) = state.session_list.take() {
                let mut session_list = session_list.iter().cloned().collect::<Vec<_>>();
                session_list.retain(|session| session.id() != session_id);
                state.session_list = Some(session_list.into());
            }
        }
        self.changes.record_change();
        self.session_change_signal(session_id).record_change();
        self.session_changes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(session_id);
    }

    pub(crate) fn sync_external_observed_active_prompt(
        &self,
        session_id: &str,
        agent_id: &str,
        external_session_id: &str,
        active_prompt: Option<PromptQueueItem>,
    ) -> bool {
        let key = (
            session_id.to_string(),
            agent_id.to_string(),
            external_session_id.to_string(),
        );
        let changed = {
            let mut state = self
                .state
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match active_prompt {
                Some(active_prompt)
                    if state.session_states.get(session_id).is_some_and(|session| {
                        session.status() != SessionStatus::Ended
                            && session.agents().iter().any(|agent| agent.id() == agent_id)
                    }) =>
                {
                    if state
                        .external_observed_active_prompts
                        .get(&key)
                        .is_some_and(|current| current.id() == active_prompt.id())
                    {
                        false
                    } else {
                        state
                            .external_observed_active_prompts
                            .insert(key, active_prompt);
                        true
                    }
                }
                Some(_) | None => state
                    .external_observed_active_prompts
                    .remove(&key)
                    .is_some(),
            }
        };
        if changed {
            self.changes.record_change();
            self.session_change_signal(session_id).record_change();
        }
        changed
    }

    pub(crate) fn project_external_observed_activity(
        &self,
        session_id: &str,
        activity: &mut BTreeMap<String, AgentRuntimeActivity>,
    ) {
        let state = self
            .state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (agent_id, agent_activity) in activity {
            let Some((_, prompt)) = state
                .external_observed_active_prompts
                .iter()
                .filter(|((projected_session_id, projected_agent_id, _), _)| {
                    projected_session_id == session_id && projected_agent_id == agent_id
                })
                .max_by(|left, right| {
                    left.1
                        .created_at_ms()
                        .cmp(&right.1.created_at_ms())
                        .then_with(|| left.1.id().cmp(right.1.id()))
                })
            else {
                continue;
            };
            if agent_activity.status == AgentRuntimeStatus::Error
                || agent_activity.busy
                || agent_activity.active_turn.is_some()
            {
                continue;
            }
            let Some(external) = prompt.external_observed_id() else {
                continue;
            };
            agent_activity.status = AgentRuntimeStatus::Working;
            agent_activity.prompt_status = AgentPromptRuntimeStatus::Running;
            agent_activity.busy = true;
            agent_activity.active_prompt_count = agent_activity.active_prompt_count.max(1);
            agent_activity.unread_idle_output = false;
            for control in agent_activity
                .queued_prompt_controls
                .values_mut()
                .filter(|control| control.status == "queued")
            {
                control.can_steer = false;
                control.steer_disabled_reason = Some(
                    super::queued_prompt_controls::QUEUED_PROMPT_STEER_EXTERNAL_REASON.to_string(),
                );
            }
            agent_activity.active_turn = Some(AgentActiveTurnProjection {
                prompt_id: prompt.id().to_string(),
                provider_run_id: None,
                source_attachment_id: Some(prompt.source_attachment_id().to_string()),
                prompt_origin: Some(prompt.prompt_origin()),
                external_provider: Some(external.provider),
                external_provider_session_id: Some(external.provider_session_id),
                external_provider_turn_id: Some(external.provider_turn_id),
                status: AgentPromptRuntimeStatus::Running,
                phase: AgentTurnRuntimePhase::Streaming,
                started_at_ms: Some(prompt.created_at_ms()),
            });
        }
    }

    pub(crate) fn external_observed_working_agents(&self) -> BTreeMap<String, BTreeSet<String>> {
        let state = self
            .state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut agents = BTreeMap::<String, BTreeSet<String>>::new();
        for (session_id, agent_id, _) in state.external_observed_active_prompts.keys() {
            agents
                .entry(session_id.clone())
                .or_default()
                .insert(agent_id.clone());
        }
        agents
    }

    pub(crate) fn change_sequence(&self) -> u64 {
        self.changes.sequence()
    }

    pub(crate) async fn wait_for_change_after(&self, sequence: u64) {
        self.changes.wait_for_change_after(sequence).await;
    }

    pub(crate) fn session_change_sequence(&self, session_id: &str) -> u64 {
        self.session_change_signal(session_id).sequence()
    }

    pub(crate) async fn wait_for_session_change_after(&self, session_id: &str, sequence: u64) {
        self.session_change_signal(session_id)
            .wait_for_change_after(sequence)
            .await;
    }

    pub(crate) fn health_snapshot(&self) -> SessionProjectionHealthSnapshot {
        let state = self
            .state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (active_prompts, queued_prompts) = projected_session_prompt_counts(
            state
                .session_states
                .values()
                .map(|session| session.as_ref()),
        );
        SessionProjectionHealthSnapshot {
            projected_sessions: state.session_states.len(),
            projected_session_list_entries: state.session_list.as_ref().map(|list| list.len()),
            active_prompts,
            queued_prompts,
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
        active_turns: &std::collections::BTreeMap<String, crate::app::ActiveTurnState>,
        provider_runs: &[crate::provider::RuntimeProviderRun],
    ) -> ProjectionInvariantHealthSnapshot {
        invariant::snapshot(
            self.projected_sessions(),
            agent_runtime,
            canonical_agents,
            active_turns,
            provider_runs,
        )
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
        self.projected_sessions_shared()
            .into_iter()
            .map(|session| session.as_ref().clone())
            .collect()
    }

    pub(crate) fn projected_sessions_shared(&self) -> Vec<Arc<RuntimeSession>> {
        let state = self
            .state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.session_list.as_ref().map_or_else(
            || state.session_states.values().cloned().collect(),
            |sessions| sessions.iter().cloned().collect(),
        )
    }

    fn session_change_signal(&self, session_id: &str) -> Arc<SessionProjectionChangeSignal> {
        self.session_changes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(session_id.to_string())
            .or_default()
            .clone()
    }
}

#[derive(Debug, Default)]
struct SessionProjectionChangeSignal {
    sequence: AtomicU64,
    notify: Notify,
}

impl SessionProjectionChangeSignal {
    fn sequence(&self) -> u64 {
        self.sequence.load(Ordering::Acquire)
    }

    fn record_change(&self) {
        self.sequence.fetch_add(1, Ordering::AcqRel);
        self.notify.notify_waiters();
    }

    async fn wait_for_change_after(&self, sequence: u64) {
        if self.sequence() != sequence {
            return;
        }
        let notified = self.notify.notified();
        if self.sequence() != sequence {
            return;
        }
        notified.await;
    }
}

fn prune_external_observed_activity(
    state: &mut SessionProjectionState,
    session: &RuntimeSession,
) -> bool {
    let previous_len = state.external_observed_active_prompts.len();
    state.external_observed_active_prompts.retain(
        |(projected_session_id, projected_agent_id, _), _| {
            projected_session_id != session.id()
                || (session.status() != SessionStatus::Ended
                    && session
                        .agents()
                        .iter()
                        .any(|agent| agent.id() == projected_agent_id))
        },
    );
    previous_len != state.external_observed_active_prompts.len()
}

fn upsert_session(
    session_list: &mut Option<Arc<[Arc<RuntimeSession>]>>,
    session: Arc<RuntimeSession>,
) -> bool {
    let Some(existing_list) = session_list.as_ref() else {
        return false;
    };
    let mut updated_list = existing_list.iter().cloned().collect::<Vec<_>>();
    if session.is_hidden() {
        let before_len = updated_list.len();
        updated_list.retain(|existing| existing.id() != session.id());
        if updated_list.len() == before_len {
            return false;
        }
        *session_list = Some(updated_list.into());
        return true;
    }
    if let Some(existing) = updated_list
        .iter_mut()
        .find(|existing| existing.id() == session.id())
    {
        if existing.as_ref() == session.as_ref() {
            return false;
        }
        *existing = session;
    } else {
        updated_list.push(session);
    }
    *session_list = Some(updated_list.into());
    true
}

fn projected_session_prompt_counts<'a>(
    sessions: impl IntoIterator<Item = &'a RuntimeSession>,
) -> (usize, usize) {
    sessions
        .into_iter()
        .flat_map(|session| session.prompt_states().values())
        .fold((0, 0), |(active_count, queued_count), prompt_state| {
            (
                active_count + usize::from(prompt_state.active_prompt().is_some()),
                queued_count + prompt_state.queued_prompts().len(),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::runtime::projection::test_support::{
        attach_cli, launch_dev_stub_provider, submit_prompt,
    };
    use crate::{DaemonApp, DaemonConfig};

    fn session(id: &str) -> RuntimeSession {
        RuntimeSession::new(id, None, "workspace", "worktree", "machine", "daemon")
    }

    fn agent(session_id: &str, agent_id: &str) -> crate::agent::AgentInstance {
        crate::agent::AgentInstance::new(
            agent_id,
            agent_id,
            session_id,
            None,
            "codex",
            None,
            None,
            None,
            crate::agent::GridPosition::new(0, 0, 1, 1),
        )
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

    #[test]
    fn identical_session_update_does_not_publish_projection_change() {
        let store = SessionStateProjectionStore::default();
        let session = session("session-1");
        store.update(session.clone());
        let global_sequence = store.change_sequence();
        let session_sequence = store.session_change_sequence(session.id());

        store.update(session.clone());

        assert_eq!(store.get(session.id()), Some(session));
        assert_eq!(store.change_sequence(), global_sequence);
        assert_eq!(
            store.session_change_sequence("session-1"),
            session_sequence,
            "identical session snapshots must not wake scoped subscribers"
        );
    }

    #[test]
    fn identical_warmed_list_update_does_not_publish_projection_change() {
        let store = SessionStateProjectionStore::default();
        let first = session("session-1");
        let second = session("session-2");
        store.update_list(vec![first.clone(), second.clone()]);
        let global_sequence = store.change_sequence();
        let first_sequence = store.session_change_sequence(first.id());
        let second_sequence = store.session_change_sequence(second.id());

        store.update_list(vec![first.clone(), second.clone()]);

        assert_eq!(store.list(), Some(vec![first, second]));
        assert_eq!(store.change_sequence(), global_sequence);
        assert_eq!(store.session_change_sequence("session-1"), first_sequence);
        assert_eq!(store.session_change_sequence("session-2"), second_sequence);
    }

    #[test]
    fn warmed_list_reads_reuse_one_immutable_snapshot() {
        let store = SessionStateProjectionStore::default();
        store.update_list(vec![session("session-1"), session("session-2")]);

        let first = store.list_shared().expect("list should be warmed");
        let second = store.list_shared().expect("list should remain warmed");

        assert!(Arc::ptr_eq(&first, &second));
        assert!(Arc::ptr_eq(&first[0], &second[0]));
    }

    #[test]
    fn warming_list_publishes_even_when_session_snapshot_already_exists() {
        let store = SessionStateProjectionStore::default();
        let session = session("session-1");
        store.update(session.clone());
        let global_sequence = store.change_sequence();

        store.update_list(vec![session.clone()]);

        assert_eq!(store.list(), Some(vec![session]));
        assert!(
            store.change_sequence() > global_sequence,
            "warming the session list changes projection availability"
        );
    }

    #[test]
    fn health_snapshot_counts_projected_prompt_state() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace",
                "worktree",
            ))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment_id = attach_cli(&mut app, &session_id, "cli-health-counts");
        launch_dev_stub_provider(&mut app, &session_id, &agent_id);
        submit_prompt(&mut app, &session_id, &attachment_id, &agent_id, "active");
        submit_prompt(&mut app, &session_id, &attachment_id, &agent_id, "queued 1");
        submit_prompt(&mut app, &session_id, &attachment_id, &agent_id, "queued 2");
        submit_prompt(&mut app, &session_id, &attachment_id, &agent_id, "queued 3");
        let session = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(&session_id)
            .expect("session snapshot should load");
        let store = SessionStateProjectionStore::default();
        store.update(session);

        let snapshot = store.health_snapshot();

        assert_eq!(snapshot.projected_sessions, 1);
        assert_eq!(snapshot.active_prompts, 1);
        assert_eq!(snapshot.queued_prompts, 3);
    }

    #[tokio::test]
    async fn session_scoped_wait_wakes_for_matching_session_update() {
        let store = SessionStateProjectionStore::default();
        let sequence = store.session_change_sequence("session-1");
        let waiter = {
            let store = store.clone();
            tokio::spawn(async move {
                store
                    .wait_for_session_change_after("session-1", sequence)
                    .await;
            })
        };

        store.update(session("session-1"));

        tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .expect("matching session projection waiter should wake")
            .expect("matching session projection waiter task should complete");
        assert!(store.session_change_sequence("session-1") > sequence);
    }

    #[tokio::test]
    async fn session_scoped_wait_does_not_wake_for_unrelated_session_update() {
        let store = SessionStateProjectionStore::default();
        let sequence = store.session_change_sequence("session-1");

        store.update(session("session-2"));

        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(50),
                store.wait_for_session_change_after("session-1", sequence),
            )
            .await
            .is_err(),
            "unrelated session projection update should not wake scoped waiter"
        );
    }

    #[tokio::test]
    async fn external_observed_activity_publishes_only_identity_changes() {
        let store = SessionStateProjectionStore::default();
        let mut projected_session = session("session-1");
        projected_session.set_agents(vec![agent("session-1", "agent-1")]);
        store.update(projected_session);
        let prompt = PromptQueueItem::external_observed_running(
            "codex",
            "provider-session-1",
            "provider-turn-1",
            "agent-1",
            "external prompt",
        );

        let global_sequence = store.change_sequence();
        let scoped_sequence = store.session_change_sequence("session-1");
        let global_waiter = {
            let store = store.clone();
            tokio::spawn(async move { store.wait_for_change_after(global_sequence).await })
        };
        let scoped_waiter = {
            let store = store.clone();
            tokio::spawn(async move {
                store
                    .wait_for_session_change_after("session-1", scoped_sequence)
                    .await
            })
        };
        tokio::task::yield_now().await;

        assert!(store.sync_external_observed_active_prompt(
            "session-1",
            "agent-1",
            "provider-session-1",
            Some(prompt.clone()),
        ));
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            global_waiter
                .await
                .expect("global projection waiter task should complete");
            scoped_waiter
                .await
                .expect("scoped projection waiter task should complete");
        })
        .await
        .expect("external activity should wake global and scoped waiters");
        assert!(store.change_sequence() > global_sequence);
        assert!(store.session_change_sequence("session-1") > scoped_sequence);

        let active_global_sequence = store.change_sequence();
        let active_scoped_sequence = store.session_change_sequence("session-1");
        assert!(!store.sync_external_observed_active_prompt(
            "session-1",
            "agent-1",
            "provider-session-1",
            Some(prompt),
        ));
        assert_eq!(store.change_sequence(), active_global_sequence);
        assert_eq!(
            store.session_change_sequence("session-1"),
            active_scoped_sequence
        );

        let global_waiter = {
            let store = store.clone();
            tokio::spawn(async move { store.wait_for_change_after(active_global_sequence).await })
        };
        let scoped_waiter = {
            let store = store.clone();
            tokio::spawn(async move {
                store
                    .wait_for_session_change_after("session-1", active_scoped_sequence)
                    .await
            })
        };
        tokio::task::yield_now().await;

        assert!(store.sync_external_observed_active_prompt(
            "session-1",
            "agent-1",
            "provider-session-1",
            None,
        ));
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            global_waiter
                .await
                .expect("global projection waiter task should complete after clear");
            scoped_waiter
                .await
                .expect("scoped projection waiter task should complete after clear");
        })
        .await
        .expect("clearing external activity should wake global and scoped waiters");
        assert!(store.change_sequence() > active_global_sequence);
        assert!(store.session_change_sequence("session-1") > active_scoped_sequence);
    }

    #[test]
    fn external_observed_activity_is_bounded_to_live_session_agents() {
        let store = SessionStateProjectionStore::default();
        let prompt = PromptQueueItem::external_observed_running(
            "codex",
            "provider-session-1",
            "provider-turn-1",
            "agent-1",
            "external prompt",
        );
        assert!(!store.sync_external_observed_active_prompt(
            "session-1",
            "agent-1",
            "provider-session-1",
            Some(prompt.clone()),
        ));

        let mut projected_session = session("session-1");
        projected_session.set_agents(vec![agent("session-1", "agent-1")]);
        store.update(projected_session.clone());
        assert!(store.sync_external_observed_active_prompt(
            "session-1",
            "agent-1",
            "provider-session-1",
            Some(prompt.clone()),
        ));
        let mut activity = BTreeMap::from([(
            "agent-1".to_string(),
            AgentRuntimeActivity {
                status: AgentRuntimeStatus::Idle,
                prompt_status: AgentPromptRuntimeStatus::None,
                busy: false,
                active_prompt_count: 0,
                queued_prompt_count: 1,
                unread_idle_output: true,
                queued_prompt_controls: BTreeMap::from([(
                    "queued-1".to_string(),
                    crate::runtime::projection::AgentQueuedPromptControlProjection {
                        prompt_id: "queued-1".to_string(),
                        status: "queued".to_string(),
                        can_steer: true,
                        can_cancel: true,
                        steer_disabled_reason: None,
                        cancel_disabled_reason: None,
                    },
                )]),
                active_turn: None,
                last_completed_turn: None,
            },
        )]);
        store.project_external_observed_activity("session-1", &mut activity);
        assert_eq!(
            store.external_observed_working_agents(),
            BTreeMap::from([(
                "session-1".to_string(),
                BTreeSet::from(["agent-1".to_string()]),
            )])
        );
        assert_eq!(
            activity["agent-1"]
                .active_turn
                .as_ref()
                .and_then(|turn| turn.source_attachment_id.as_deref()),
            Some("external:codex")
        );
        assert!(
            !activity["agent-1"].unread_idle_output,
            "working external activity must clear the idle-only unread state"
        );
        let queued_control = &activity["agent-1"].queued_prompt_controls["queued-1"];
        assert!(
            !queued_control.can_steer,
            "queued prompts cannot steer a provider turn started outside Arroba"
        );
        assert_eq!(
            queued_control.steer_disabled_reason.as_deref(),
            Some(crate::runtime::projection::QUEUED_PROMPT_STEER_EXTERNAL_REASON)
        );

        projected_session.set_agents(Vec::new());
        store.update(projected_session.clone());
        assert!(store.external_observed_working_agents().is_empty());
        assert!(!store.sync_external_observed_active_prompt(
            "session-1",
            "agent-1",
            "provider-session-1",
            None,
        ));
        assert!(!store.sync_external_observed_active_prompt(
            "session-1",
            "agent-1",
            "provider-session-1",
            Some(prompt.clone()),
        ));

        projected_session.set_agents(vec![agent("session-1", "agent-1")]);
        assert!(projected_session.transition_to(SessionStatus::Ended));
        store.update(projected_session);
        assert!(!store.sync_external_observed_active_prompt(
            "session-1",
            "agent-1",
            "provider-session-1",
            Some(prompt),
        ));
    }
}
