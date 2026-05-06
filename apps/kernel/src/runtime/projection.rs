use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use arroba_relay::protocol::RelayKernelPresence;

use crate::agent::AgentState;
use crate::app::DaemonApp;
use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::history::SessionHistoryEntry;
use crate::local::RemoteMachineRecord;
use crate::provider::ProviderRunState;
use crate::provider::{OpenCodeProviderCatalog, ProviderProcessInfo, RuntimeProviderRun};
use crate::runtime::capability_executor::CapabilityExecutorHealthSnapshot;
use crate::runtime::workspace_coordinator::WorkspaceOperationClaimSnapshot;
use crate::session::{unix_epoch_ms, PromptQueueItem, PromptStatus, RuntimeSession, SessionStatus};
use crate::session_history_page::{paginate_session_history, SessionHistoryPage};
use crate::terminal::TerminalStreamHealthSnapshot;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionMetadata {
    pub projection_version: u64,
    pub last_event_id: u64,
    pub generated_at_ms: u64,
}

impl ProjectionMetadata {
    pub fn new(projection_version: u64, last_event_id: u64) -> Self {
        Self {
            projection_version,
            last_event_id,
            generated_at_ms: unix_epoch_ms(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSnapshotProjection {
    pub metadata: ProjectionMetadata,
    pub session: RuntimeSession,
    pub provider_run: Option<RuntimeProviderRun>,
    pub agent_activity: BTreeMap<String, AgentRuntimeActivity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRuntimeStatus {
    Idle,
    Working,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPromptRuntimeStatus {
    None,
    Queued,
    Running,
    Cancelling,
    Settling,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRuntimeActivity {
    pub status: AgentRuntimeStatus,
    pub prompt_status: AgentPromptRuntimeStatus,
    pub busy: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_turn: Option<AgentActiveTurnProjection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentActiveTurnProjection {
    pub prompt_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_run_id: Option<String>,
    pub status: AgentPromptRuntimeStatus,
}

#[derive(Clone)]
pub(crate) struct DaemonConfigProjectionStore {
    config: Arc<StdMutex<DaemonConfig>>,
}

impl DaemonConfigProjectionStore {
    pub(crate) fn new(config: DaemonConfig) -> Self {
        Self {
            config: Arc::new(StdMutex::new(config)),
        }
    }

    pub(crate) fn snapshot(&self) -> DaemonConfig {
        self.config
            .lock()
            .expect("daemon config projection lock should not be poisoned")
            .clone()
    }

    pub(crate) fn update(&self, config: DaemonConfig) {
        *self
            .config
            .lock()
            .expect("daemon config projection lock should not be poisoned") = config;
    }
}

#[derive(Clone, Default)]
pub(crate) struct RemoteRelayInventoryProjectionStore {
    state: Arc<StdMutex<RemoteRelayInventoryProjectionState>>,
}

#[derive(Debug, Clone, Default)]
struct RemoteRelayInventoryProjectionState {
    remote_machines: Vec<RemoteMachineRecord>,
    remote_kernels: Vec<RelayKernelPresence>,
    refreshed_at_ms: u64,
    refresh_requested_at_ms: u64,
}

impl RemoteRelayInventoryProjectionStore {
    pub(crate) fn snapshot(&self) -> (Vec<RemoteMachineRecord>, Vec<RelayKernelPresence>) {
        let state = self
            .state
            .lock()
            .expect("remote relay inventory projection lock should not be poisoned");
        (state.remote_machines.clone(), state.remote_kernels.clone())
    }

    pub(crate) fn should_request_refresh(
        &self,
        now_ms: u64,
        stale_after_ms: u64,
        cooldown_ms: u64,
    ) -> bool {
        let mut state = self
            .state
            .lock()
            .expect("remote relay inventory projection lock should not be poisoned");
        let empty = state.remote_machines.is_empty() && state.remote_kernels.is_empty();
        let stale = state.refreshed_at_ms == 0
            || now_ms.saturating_sub(state.refreshed_at_ms) >= stale_after_ms;
        let cooled_down = state.refresh_requested_at_ms == 0
            || now_ms.saturating_sub(state.refresh_requested_at_ms) >= cooldown_ms;
        if (empty || stale) && cooled_down {
            state.refresh_requested_at_ms = now_ms;
            return true;
        }
        false
    }

    pub(crate) fn update(
        &self,
        remote_machines: Vec<RemoteMachineRecord>,
        remote_kernels: Vec<RelayKernelPresence>,
    ) {
        let mut state = self
            .state
            .lock()
            .expect("remote relay inventory projection lock should not be poisoned");
        state.remote_machines = remote_machines;
        state.remote_kernels = remote_kernels;
        state.refreshed_at_ms = unix_epoch_ms();
        state.refresh_requested_at_ms = state.refreshed_at_ms;
    }

    pub(crate) fn clear(&self) {
        let mut state = self
            .state
            .lock()
            .expect("remote relay inventory projection lock should not be poisoned");
        state.remote_machines.clear();
        state.remote_kernels.clear();
        state.refreshed_at_ms = 0;
        state.refresh_requested_at_ms = 0;
    }
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRuntimeProjection {
    pub session_id: String,
    pub agent_id: String,
    pub active_prompt: Option<PromptQueueItem>,
    pub next_queued_prompt: Option<PromptQueueItem>,
    pub queued_prompt_count: usize,
}

#[derive(Clone, Default)]
pub(crate) struct AgentRuntimeProjectionStore {
    agents: Arc<StdMutex<HashMap<String, AgentRuntimeProjection>>>,
}

impl AgentRuntimeProjectionStore {
    pub(crate) fn get(&self, agent_id: &str) -> Option<AgentRuntimeProjection> {
        self.agents
            .lock()
            .expect("agent runtime projection lock should not be poisoned")
            .get(agent_id)
            .cloned()
    }

    pub(crate) fn list(&self) -> Vec<AgentRuntimeProjection> {
        let mut projections = self
            .agents
            .lock()
            .expect("agent runtime projection lock should not be poisoned")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        projections.sort_by(|left, right| {
            left.session_id
                .cmp(&right.session_id)
                .then_with(|| left.agent_id.cmp(&right.agent_id))
        });
        projections
    }

    pub(crate) fn list_for_session(&self, session_id: &str) -> Vec<AgentRuntimeProjection> {
        self.list()
            .into_iter()
            .filter(|projection| projection.session_id == session_id)
            .collect()
    }

    pub(crate) fn next_queued_prompt(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        self.get(agent_id)
            .filter(|projection| projection.session_id == session_id)
            .and_then(|projection| projection.next_queued_prompt)
    }

    pub(crate) fn update_session(&self, session: &RuntimeSession) {
        let mut agents = self
            .agents
            .lock()
            .expect("agent runtime projection lock should not be poisoned");
        agents.retain(|_, projection| projection.session_id != session.id());
        for agent in session.agents() {
            let prompt_state = session.prompt_states().get(agent.id());
            agents.insert(
                agent.id().to_string(),
                AgentRuntimeProjection {
                    session_id: session.id().to_string(),
                    agent_id: agent.id().to_string(),
                    active_prompt: prompt_state.and_then(|state| state.active_prompt().cloned()),
                    next_queued_prompt: prompt_state
                        .and_then(|state| state.queued_prompts().front().cloned()),
                    queued_prompt_count: prompt_state
                        .map(|state| state.queued_prompts().len())
                        .unwrap_or(0),
                },
            );
        }
        for (agent_id, prompt_state) in session.prompt_states() {
            agents
                .entry(agent_id.clone())
                .or_insert_with(|| AgentRuntimeProjection {
                    session_id: session.id().to_string(),
                    agent_id: agent_id.clone(),
                    active_prompt: prompt_state.active_prompt().cloned(),
                    next_queued_prompt: prompt_state.queued_prompts().front().cloned(),
                    queued_prompt_count: prompt_state.queued_prompts().len(),
                });
        }
    }

    #[cfg(test)]
    pub(crate) fn update_agent_from_session(&self, session: &RuntimeSession, agent_id: &str) {
        let mut agents = self
            .agents
            .lock()
            .expect("agent runtime projection lock should not be poisoned");
        let Some(projection) = agent_runtime_projection_from_session(session, agent_id) else {
            agents.remove(agent_id);
            return;
        };
        agents.insert(agent_id.to_string(), projection);
    }

    #[cfg(test)]
    pub(crate) fn update_agent_prompt_state(
        &self,
        session_id: &str,
        agent_id: &str,
        active_prompt: Option<PromptQueueItem>,
        next_queued_prompt: Option<PromptQueueItem>,
        queued_prompt_count: usize,
    ) {
        self.agents
            .lock()
            .expect("agent runtime projection lock should not be poisoned")
            .insert(
                agent_id.to_string(),
                AgentRuntimeProjection {
                    session_id: session_id.to_string(),
                    agent_id: agent_id.to_string(),
                    active_prompt,
                    next_queued_prompt,
                    queued_prompt_count,
                },
            );
    }

    pub(crate) fn remove_session(&self, session_id: &str) {
        self.agents
            .lock()
            .expect("agent runtime projection lock should not be poisoned")
            .retain(|_, projection| projection.session_id != session_id);
    }

    pub(crate) fn health_snapshot(&self) -> AgentRuntimeProjectionHealthSnapshot {
        let agents = self.list();
        AgentRuntimeProjectionHealthSnapshot {
            projected_agents: agents.len(),
            active_prompts: agents
                .iter()
                .filter(|projection| projection.active_prompt.is_some())
                .count(),
            queued_prompts: agents
                .iter()
                .map(|projection| projection.queued_prompt_count)
                .sum(),
        }
    }
}

#[cfg(test)]
fn agent_runtime_projection_from_session(
    session: &RuntimeSession,
    agent_id: &str,
) -> Option<AgentRuntimeProjection> {
    if !session.agents().iter().any(|agent| agent.id() == agent_id)
        && !session.prompt_states().contains_key(agent_id)
    {
        return None;
    }
    let prompt_state = session.prompt_states().get(agent_id);
    Some(AgentRuntimeProjection {
        session_id: session.id().to_string(),
        agent_id: agent_id.to_string(),
        active_prompt: prompt_state.and_then(|state| state.active_prompt().cloned()),
        next_queued_prompt: prompt_state.and_then(|state| state.queued_prompts().front().cloned()),
        queued_prompt_count: prompt_state
            .map(|state| state.queued_prompts().len())
            .unwrap_or(0),
    })
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

#[derive(Clone, Default)]
pub(crate) struct SessionHistoryProjectionStore {
    entries: Arc<StdMutex<HashMap<String, Vec<SessionHistoryEntry>>>>,
}

impl SessionHistoryProjectionStore {
    pub(crate) fn page(
        &self,
        session_id: &str,
        agent_id: Option<&str>,
        round_count: Option<usize>,
        max_chars: Option<usize>,
        before_entry_index: Option<usize>,
        before_entry_char_offset: Option<usize>,
    ) -> Option<SessionHistoryPage> {
        let entries = self
            .entries
            .lock()
            .expect("session history projection lock should not be poisoned")
            .get(session_id)
            .cloned()?;
        Some(page_history_entries(
            entries,
            agent_id,
            round_count,
            max_chars,
            before_entry_index,
            before_entry_char_offset,
        ))
    }

    pub(crate) fn update_entries(&self, session_id: &str, entries: Vec<SessionHistoryEntry>) {
        self.entries
            .lock()
            .expect("session history projection lock should not be poisoned")
            .insert(session_id.to_string(), entries);
    }

    pub(crate) fn append(&self, entry: SessionHistoryEntry) {
        let mut entries_by_session = self
            .entries
            .lock()
            .expect("session history projection lock should not be poisoned");
        if let Some(entries) = entries_by_session.get_mut(&entry.session_id) {
            entries.push(entry);
        }
    }

    pub(crate) fn remove(&self, session_id: &str) {
        self.entries
            .lock()
            .expect("session history projection lock should not be poisoned")
            .remove(session_id);
    }
}

pub(crate) fn page_history_entries(
    mut entries: Vec<SessionHistoryEntry>,
    agent_id: Option<&str>,
    round_count: Option<usize>,
    max_chars: Option<usize>,
    before_entry_index: Option<usize>,
    before_entry_char_offset: Option<usize>,
) -> SessionHistoryPage {
    if let Some(agent_id) = agent_id {
        entries.retain(|entry| {
            entry.agent_id.is_none() || entry.agent_id.as_deref() == Some(agent_id)
        });
    }
    paginate_session_history(
        &entries,
        round_count,
        max_chars,
        before_entry_index,
        before_entry_char_offset,
    )
}

#[derive(Clone, Default)]
pub(crate) struct ProviderRunProjectionStore {
    runs: Arc<StdMutex<HashMap<String, RuntimeProviderRun>>>,
}

impl ProviderRunProjectionStore {
    pub(crate) fn get(&self, provider_run_id: &str) -> Option<RuntimeProviderRun> {
        self.runs
            .lock()
            .expect("provider run projection lock should not be poisoned")
            .get(provider_run_id)
            .cloned()
    }

    pub(crate) fn get_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<RuntimeProviderRun> {
        self.runs
            .lock()
            .expect("provider run projection lock should not be poisoned")
            .values()
            .filter(|run| {
                run.session_id() == session_id
                    && run.agent_instance_id() == Some(agent_id)
                    && run.state() != crate::provider::ProviderRunState::Ended
            })
            .max_by_key(|run| match run.state() {
                crate::provider::ProviderRunState::Running => 3,
                crate::provider::ProviderRunState::Parked => 2,
                crate::provider::ProviderRunState::Starting => 1,
                crate::provider::ProviderRunState::Ended => 0,
            })
            .cloned()
    }

    pub(crate) fn update(&self, run: RuntimeProviderRun) {
        self.runs
            .lock()
            .expect("provider run projection lock should not be poisoned")
            .insert(run.id().to_string(), run);
    }
}

#[derive(Clone, Default)]
pub(crate) struct ProviderProcessProjectionStore {
    processes: Arc<StdMutex<Option<Vec<ProviderProcessInfo>>>>,
}

impl ProviderProcessProjectionStore {
    pub(crate) fn list(&self, provider: Option<&str>) -> Option<Vec<ProviderProcessInfo>> {
        let processes = self
            .processes
            .lock()
            .expect("provider process projection lock should not be poisoned")
            .clone()?;
        Some(filter_provider_processes(processes, provider))
    }

    pub(crate) fn update_list(&self, processes: Vec<ProviderProcessInfo>) {
        *self
            .processes
            .lock()
            .expect("provider process projection lock should not be poisoned") = Some(processes);
    }

    pub(crate) fn invalidate(&self) {
        *self
            .processes
            .lock()
            .expect("provider process projection lock should not be poisoned") = None;
    }
}

fn filter_provider_processes(
    processes: Vec<ProviderProcessInfo>,
    provider: Option<&str>,
) -> Vec<ProviderProcessInfo> {
    let Some(provider) = provider else {
        return processes;
    };
    processes
        .into_iter()
        .filter(|process| process.provider == provider)
        .collect()
}

#[derive(Clone, Default)]
pub(crate) struct ProviderCatalogProjectionStore {
    catalog: Arc<StdMutex<Option<CachedProviderCatalogProjection>>>,
}

#[derive(Clone)]
struct CachedProviderCatalogProjection {
    cached_at: Instant,
    catalog: OpenCodeProviderCatalog,
}

impl ProviderCatalogProjectionStore {
    pub(crate) fn get(&self, ttl: Duration) -> Option<OpenCodeProviderCatalog> {
        let cached = self
            .catalog
            .lock()
            .expect("provider catalog projection lock should not be poisoned")
            .clone()?;
        if cached.cached_at.elapsed() < ttl {
            Some(cached.catalog)
        } else {
            None
        }
    }

    pub(crate) fn update(&self, catalog: OpenCodeProviderCatalog) {
        *self
            .catalog
            .lock()
            .expect("provider catalog projection lock should not be poisoned") =
            Some(CachedProviderCatalogProjection {
                cached_at: Instant::now(),
                catalog,
            });
    }

    pub(crate) fn invalidate(&self) {
        *self
            .catalog
            .lock()
            .expect("provider catalog projection lock should not be poisoned") = None;
    }

    pub(crate) fn health_snapshot(&self, ttl: Duration) -> ProviderCatalogHealthSnapshot {
        let cached = self
            .catalog
            .lock()
            .expect("provider catalog projection lock should not be poisoned")
            .clone();
        let Some(cached) = cached else {
            return ProviderCatalogHealthSnapshot {
                cached: false,
                expired: false,
                age_ms: None,
                ttl_ms: ttl.as_millis() as u64,
            };
        };
        let age = cached.cached_at.elapsed();
        ProviderCatalogHealthSnapshot {
            cached: true,
            expired: age >= ttl,
            age_ms: Some(age.as_millis() as u64),
            ttl_ms: ttl.as_millis() as u64,
        }
    }
}

impl SessionSnapshotProjection {
    pub fn from_daemon_app(
        app: &mut DaemonApp,
        session_id: &str,
        last_event_id: u64,
    ) -> Result<Self, DaemonError> {
        let mut session = app.sessions().get_session(session_id)?;
        let agents = app.agents().get_session_agents(session_id);
        session.set_agents(agents);
        app.project_session_runtime_view(&mut session);
        let provider_run = session
            .active_provider_run_id()
            .and_then(|provider_run_id| app.providers().get_run(provider_run_id).ok());
        let agent_activity = agent_activity_for_session(app, &session);
        Ok(Self {
            metadata: ProjectionMetadata::new(2, last_event_id),
            session,
            provider_run,
            agent_activity,
        })
    }
}

fn agent_activity_for_session(
    app: &DaemonApp,
    session: &RuntimeSession,
) -> BTreeMap<String, AgentRuntimeActivity> {
    let prompt_activity = app.prompt_activity_store();
    let prompt_activity = prompt_activity.read();
    let mut activity = BTreeMap::new();

    for agent in session.agents() {
        let prompt_state = session.prompt_states().get(agent.id());
        let active_prompt = prompt_state.and_then(|state| state.active_prompt());
        let queued_prompt_count = prompt_state
            .map(|state| state.queued_prompts().len())
            .unwrap_or(0);
        let prompt_status = match active_prompt.map(PromptQueueItem::status) {
            Some(PromptStatus::Cancelling) => AgentPromptRuntimeStatus::Cancelling,
            Some(PromptStatus::Running) => {
                let settlement_requested = app
                    .providers()
                    .get_run_for_agent(session.id(), agent.id())
                    .and_then(|run| {
                        prompt_activity
                            .get(run.id())
                            .map(|state| state.settlement_requested)
                    })
                    .unwrap_or(false);
                if settlement_requested {
                    AgentPromptRuntimeStatus::Settling
                } else {
                    AgentPromptRuntimeStatus::Running
                }
            }
            Some(PromptStatus::Queued) => AgentPromptRuntimeStatus::Queued,
            Some(PromptStatus::Completed) | Some(PromptStatus::Cancelled) | None => {
                if queued_prompt_count > 0 {
                    AgentPromptRuntimeStatus::Queued
                } else {
                    AgentPromptRuntimeStatus::None
                }
            }
        };
        let provider_run = app.providers().get_run_for_agent(session.id(), agent.id());
        let provider_busy = provider_run.as_ref().is_some_and(|run| {
            matches!(
                run.state(),
                ProviderRunState::Starting | ProviderRunState::Running
            ) && active_prompt.is_some()
        });
        let active_turn = active_prompt.map(|prompt| AgentActiveTurnProjection {
            prompt_id: prompt.id().to_string(),
            provider_run_id: provider_run.as_ref().map(|run| run.id().to_string()),
            status: prompt_status.clone(),
        });
        let prompt_busy = !matches!(prompt_status, AgentPromptRuntimeStatus::None);
        let agent_busy =
            agent.is_processing() || agent.state() == AgentState::Working || provider_busy;
        let status = if agent.state() == AgentState::Error {
            AgentRuntimeStatus::Error
        } else if prompt_busy || agent_busy {
            AgentRuntimeStatus::Working
        } else {
            AgentRuntimeStatus::Idle
        };
        activity.insert(
            agent.id().to_string(),
            AgentRuntimeActivity {
                busy: status == AgentRuntimeStatus::Working,
                status,
                prompt_status,
                active_turn,
            },
        );
    }

    activity
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorQueueSnapshot {
    pub lane_id: String,
    pub queue_limit: usize,
    pub queued_commands: usize,
}

impl ActorQueueSnapshot {
    pub fn new(lane_id: impl Into<String>, queue_limit: usize, queued_commands: usize) -> Self {
        Self {
            lane_id: lane_id.into(),
            queue_limit,
            queued_commands,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionProjectionHealthSnapshot {
    pub projected_sessions: usize,
    pub projected_session_list_entries: Option<usize>,
    pub active_prompts: usize,
    pub queued_prompts: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRuntimeProjectionHealthSnapshot {
    pub projected_agents: usize,
    pub active_prompts: usize,
    pub queued_prompts: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProjectionInvariantHealthSnapshot {
    pub checked_sessions: usize,
    pub checked_agents: usize,
    pub mismatches: Vec<ProjectionInvariantMismatch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionInvariantMismatch {
    pub kind: String,
    pub session_id: String,
    pub agent_id: Option<String>,
    pub details: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProviderRunActorHealthSnapshot {
    pub enqueued_commands: u64,
    pub enqueue_rejections: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCatalogHealthSnapshot {
    pub cached: bool,
    pub expired: bool,
    pub age_ms: Option<u64>,
    pub ttl_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeClaimSnapshot {
    pub workspace_id: String,
    pub worktree_id: String,
    pub session_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WorkspaceCoordinationHealthSnapshot {
    pub active_worktree_claims: Vec<WorktreeClaimSnapshot>,
    pub worktree_collisions: Vec<WorktreeClaimSnapshot>,
    pub active_operation_claims: Vec<WorkspaceOperationClaimSnapshot>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedIoHealthSnapshot {
    pub active_reservations: usize,
    pub active_reservation_artifacts: usize,
    pub workspace_identity:
        crate::runtime::workspace_identity_monitor::WorkspaceIdentityMonitorHealthSnapshot,
    pub external_changes: crate::io::ArtifactExternalChangeHealthSnapshot,
}

impl Default for ManagedIoHealthSnapshot {
    fn default() -> Self {
        Self {
            active_reservations: 0,
            active_reservation_artifacts: 0,
            workspace_identity:
                crate::runtime::workspace_identity_monitor::WorkspaceIdentityMonitorHealthSnapshot {
                    tracked_provider_runs: 0,
                    identity_changed_provider_runs: 0,
                    invalid_provider_runs: 0,
                    current_generation_total: 0,
                },
            external_changes: crate::io::ArtifactExternalChangeHealthSnapshot {
                tracked_artifacts: 0,
                externally_changed_artifacts: 0,
                external_change_events: 0,
                live_watcher_started: false,
                live_watcher_scans: 0,
                live_watcher_scan_errors: 0,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportHealthSnapshot {
    pub active_connections: usize,
    pub active_subscriptions: usize,
    pub retained_event_limit: usize,
    pub command_result_cache_limit: usize,
    pub inbound_request_limit: usize,
    pub incoming_requests: u64,
    pub emitted_events: u64,
    pub replay_gaps: u64,
    pub inbound_overload_rejections: u64,
    pub duplicate_command_conflicts: u64,
    pub outgoing_queue_overflows: u64,
    pub slow_consumer_closes: u64,
}

impl Default for TransportHealthSnapshot {
    fn default() -> Self {
        Self {
            active_connections: 0,
            active_subscriptions: 0,
            retained_event_limit: 0,
            command_result_cache_limit: 0,
            inbound_request_limit: 0,
            incoming_requests: 0,
            emitted_events: 0,
            replay_gaps: 0,
            inbound_overload_rejections: 0,
            duplicate_command_conflicts: 0,
            outgoing_queue_overflows: 0,
            slow_consumer_closes: 0,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct TransportHealthStore {
    state: Arc<TransportHealthState>,
}

#[derive(Debug, Default)]
struct TransportHealthState {
    active_connections: AtomicUsize,
    active_subscriptions: AtomicUsize,
    incoming_requests: AtomicU64,
    emitted_events: AtomicU64,
    replay_gaps: AtomicU64,
    inbound_overload_rejections: AtomicU64,
    duplicate_command_conflicts: AtomicU64,
    outgoing_queue_overflows: AtomicU64,
    slow_consumer_closes: AtomicU64,
}

impl TransportHealthStore {
    pub(crate) fn record_connection_opened(&self) {
        self.state
            .active_connections
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_connection_closed(&self) {
        decrement_saturating(&self.state.active_connections);
    }

    pub(crate) fn record_subscription_opened(&self) {
        self.state
            .active_subscriptions
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_subscription_closed(&self) {
        decrement_saturating(&self.state.active_subscriptions);
    }

    pub(crate) fn record_incoming_request(&self) {
        self.state.incoming_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_emitted_event(&self) {
        self.state.emitted_events.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_replay_gap(&self) {
        self.state.replay_gaps.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_inbound_overload_rejection(&self) {
        self.state
            .inbound_overload_rejections
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_duplicate_command_conflict(&self) {
        self.state
            .duplicate_command_conflicts
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_outgoing_queue_overflow(&self) {
        self.state
            .outgoing_queue_overflows
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_slow_consumer_close(&self) {
        self.state
            .slow_consumer_closes
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn snapshot(
        &self,
        retained_event_limit: usize,
        command_result_cache_limit: usize,
        inbound_request_limit: usize,
    ) -> TransportHealthSnapshot {
        TransportHealthSnapshot {
            active_connections: self.state.active_connections.load(Ordering::Relaxed),
            active_subscriptions: self.state.active_subscriptions.load(Ordering::Relaxed),
            retained_event_limit,
            command_result_cache_limit,
            inbound_request_limit,
            incoming_requests: self.state.incoming_requests.load(Ordering::Relaxed),
            emitted_events: self.state.emitted_events.load(Ordering::Relaxed),
            replay_gaps: self.state.replay_gaps.load(Ordering::Relaxed),
            inbound_overload_rejections: self
                .state
                .inbound_overload_rejections
                .load(Ordering::Relaxed),
            duplicate_command_conflicts: self
                .state
                .duplicate_command_conflicts
                .load(Ordering::Relaxed),
            outgoing_queue_overflows: self.state.outgoing_queue_overflows.load(Ordering::Relaxed),
            slow_consumer_closes: self.state.slow_consumer_closes.load(Ordering::Relaxed),
        }
    }
}

fn decrement_saturating(value: &AtomicUsize) {
    let _ = value.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
        current.checked_sub(1)
    });
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonHealthProjection {
    pub metadata: ProjectionMetadata,
    pub session_command_lanes: Vec<ActorQueueSnapshot>,
    pub agent_command_lanes: Vec<ActorQueueSnapshot>,
    pub workflow_command_lanes: Vec<ActorQueueSnapshot>,
    pub provider_runtime_lanes: Vec<ActorQueueSnapshot>,
    pub provider_run_actor: ProviderRunActorHealthSnapshot,
    pub capability_executor: CapabilityExecutorHealthSnapshot,
    pub session_projection: SessionProjectionHealthSnapshot,
    pub agent_runtime_projection: AgentRuntimeProjectionHealthSnapshot,
    pub provider_catalog: ProviderCatalogHealthSnapshot,
    pub transport: TransportHealthSnapshot,
    pub terminal_stream: TerminalStreamHealthSnapshot,
    pub workspace_coordination: WorkspaceCoordinationHealthSnapshot,
    pub managed_io: ManagedIoHealthSnapshot,
    pub projection_invariants: ProjectionInvariantHealthSnapshot,
}

impl DaemonHealthProjection {
    pub fn new(
        last_event_id: u64,
        session_command_lanes: Vec<ActorQueueSnapshot>,
        agent_command_lanes: Vec<ActorQueueSnapshot>,
        workflow_command_lanes: Vec<ActorQueueSnapshot>,
        provider_runtime_lanes: Vec<ActorQueueSnapshot>,
        provider_run_actor: ProviderRunActorHealthSnapshot,
        capability_executor: CapabilityExecutorHealthSnapshot,
        mut session_projection: SessionProjectionHealthSnapshot,
        agent_runtime_projection: AgentRuntimeProjectionHealthSnapshot,
        provider_catalog: ProviderCatalogHealthSnapshot,
        transport: TransportHealthSnapshot,
        terminal_stream: TerminalStreamHealthSnapshot,
        workspace_coordination: WorkspaceCoordinationHealthSnapshot,
        managed_io: ManagedIoHealthSnapshot,
        projection_invariants: ProjectionInvariantHealthSnapshot,
    ) -> Self {
        // Compatibility: legacy clients may still read prompt counts from the
        // session projection object. The agent runtime projection is the
        // canonical health source for prompt work during the ownership
        // migration, so mirror its counts here until the old fields can be
        // retired from the wire shape.
        session_projection.active_prompts = agent_runtime_projection.active_prompts;
        session_projection.queued_prompts = agent_runtime_projection.queued_prompts;
        Self {
            metadata: ProjectionMetadata::new(1, last_event_id),
            session_command_lanes,
            agent_command_lanes,
            workflow_command_lanes,
            provider_runtime_lanes,
            provider_run_actor,
            capability_executor,
            session_projection,
            agent_runtime_projection,
            provider_catalog,
            transport,
            terminal_stream,
            workspace_coordination,
            managed_io,
            projection_invariants,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::agent::CreateAgentRequest;
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::provider::{LaunchProviderRequest, RuntimeProviderRun};
    use crate::runtime::capability_executor::CapabilityExecutorHealthSnapshot;
    use crate::runtime::projection::{
        ActorQueueSnapshot, AgentPromptRuntimeStatus, AgentRuntimeProjectionHealthSnapshot,
        AgentRuntimeProjectionStore, AgentRuntimeStatus, DaemonHealthProjection,
        ManagedIoHealthSnapshot, ProjectionInvariantHealthSnapshot, ProviderCatalogHealthSnapshot,
        ProviderRunActorHealthSnapshot, RemoteRelayInventoryProjectionStore,
        SessionProjectionHealthSnapshot, SessionSnapshotProjection, SessionStateProjectionStore,
        TransportHealthSnapshot, WorkspaceCoordinationHealthSnapshot,
    };
    use crate::session::CreateSessionRequest;
    use crate::terminal::TerminalStreamHealthSnapshot;
    use crate::{DaemonApp, DaemonConfig};

    fn launch_dev_stub_provider(
        app: &mut DaemonApp,
        session_id: &str,
        agent_id: &str,
    ) -> RuntimeProviderRun {
        let provider_run = app
            .launch_provider(
                LaunchProviderRequest::new(
                    session_id,
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(agent_id),
            )
            .expect("provider run should launch");
        app.update_provider_run_projection(provider_run.clone());
        provider_run
    }

    fn submit_prompt(
        app: &mut DaemonApp,
        session_id: &str,
        attachment_id: &str,
        agent_id: &str,
        prompt: &str,
    ) {
        crate::app::KernelAgentService::new(app)
            .submit_prompt(
                session_id,
                attachment_id,
                Some(agent_id),
                prompt,
                Vec::new(),
            )
            .expect("prompt should submit");
    }

    fn attach_cli(app: &mut DaemonApp, session_id: &str, client_id: &str) -> String {
        let mut sessions = app.sessions_mut();
        let attachment = app
            .attachments()
            .attach(
                &mut sessions,
                AttachRequest::new(session_id, client_id, ClientCapabilityLevel::FullTerminal),
            )
            .expect("session should attach");
        attachment.id().to_string()
    }

    #[test]
    fn session_snapshot_projection_includes_metadata_and_agents() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");

        let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
            .expect("projection should build");

        assert_eq!(projection.metadata.projection_version, 2);
        assert_eq!(projection.metadata.last_event_id, 42);
        assert_eq!(projection.session.id(), session.id());
        assert_eq!(projection.session.agents().len(), 1);
    }

    #[test]
    fn session_snapshot_projection_marks_settling_prompt_as_working() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let provider_run = launch_dev_stub_provider(&mut app, session.id(), agent.id());
        let attachment_id = attach_cli(&mut app, session.id(), "cli-settling");
        submit_prompt(
            &mut app,
            session.id(),
            &attachment_id,
            agent.id(),
            "status check",
        );
        app.prompt_activity_store().write().insert(
            provider_run.id().to_string(),
            crate::app::ActivePromptState {
                last_output_at: None,
                saw_response_content: true,
                completion_recorded: true,
                settlement_requested: true,
            },
        );

        let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
            .expect("projection should build");
        let activity = projection
            .agent_activity
            .get(agent.id())
            .expect("agent activity should be projected");

        assert_eq!(activity.status, AgentRuntimeStatus::Working);
        assert_eq!(activity.prompt_status, AgentPromptRuntimeStatus::Settling);
        assert!(activity.busy);
    }

    #[test]
    fn session_snapshot_projection_marks_completed_prompt_as_idle() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let provider_run = launch_dev_stub_provider(&mut app, session.id(), agent.id());
        let attachment_id = attach_cli(&mut app, session.id(), "cli-idle");
        submit_prompt(
            &mut app,
            session.id(),
            &attachment_id,
            agent.id(),
            "status check",
        );
        app.complete_active_prompt(session.id(), agent.id(), Some(provider_run.id()))
            .expect("prompt should complete");

        let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
            .expect("projection should build");
        let activity = projection
            .agent_activity
            .get(agent.id())
            .expect("agent activity should be projected");

        assert_eq!(activity.status, AgentRuntimeStatus::Idle);
        assert_eq!(activity.prompt_status, AgentPromptRuntimeStatus::None);
        assert!(!activity.busy);
    }

    #[test]
    fn daemon_health_projection_records_actor_queue_snapshots() {
        let projection = DaemonHealthProjection::new(
            7,
            vec![ActorQueueSnapshot::new("session-1", 128, 2)],
            vec![ActorQueueSnapshot::new("agent-1", 128, 1)],
            vec![ActorQueueSnapshot::new("workflow-session-1", 128, 3)],
            vec![ActorQueueSnapshot::new("provider-run-1", 1, 1)],
            ProviderRunActorHealthSnapshot {
                enqueued_commands: 5,
                enqueue_rejections: 1,
            },
            CapabilityExecutorHealthSnapshot {
                max_concurrent_jobs: 64,
                available_permits: 63,
                submitted_jobs: 8,
                running_jobs: 1,
                completed_jobs: 6,
                failed_jobs: 1,
                rejected_jobs: 0,
                join_errors: 0,
            },
            SessionProjectionHealthSnapshot {
                projected_sessions: 3,
                projected_session_list_entries: Some(3),
                active_prompts: 99,
                queued_prompts: 98,
            },
            AgentRuntimeProjectionHealthSnapshot {
                projected_agents: 3,
                active_prompts: 1,
                queued_prompts: 2,
            },
            ProviderCatalogHealthSnapshot {
                cached: true,
                expired: false,
                age_ms: Some(10),
                ttl_ms: 5_000,
            },
            TransportHealthSnapshot {
                active_connections: 2,
                active_subscriptions: 1,
                retained_event_limit: 256,
                command_result_cache_limit: 512,
                inbound_request_limit: 8,
                incoming_requests: 9,
                emitted_events: 4,
                replay_gaps: 1,
                inbound_overload_rejections: 1,
                duplicate_command_conflicts: 1,
                outgoing_queue_overflows: 1,
                slow_consumer_closes: 1,
            },
            TerminalStreamHealthSnapshot {
                pending_output_records: 4,
                pending_notice_records: 3,
                pending_completion_records: 2,
                pending_output_record_limit_per_attachment: 4096,
                trimmed_pending_output_recipients: 1,
            },
            WorkspaceCoordinationHealthSnapshot {
                active_worktree_claims: vec![crate::runtime::projection::WorktreeClaimSnapshot {
                    workspace_id: "workspace-1".to_string(),
                    worktree_id: "worktree-1".to_string(),
                    session_ids: vec!["session-1".to_string(), "session-2".to_string()],
                }],
                worktree_collisions: vec![crate::runtime::projection::WorktreeClaimSnapshot {
                    workspace_id: "workspace-1".to_string(),
                    worktree_id: "worktree-1".to_string(),
                    session_ids: vec!["session-1".to_string(), "session-2".to_string()],
                }],
                active_operation_claims: Vec::new(),
            },
            ManagedIoHealthSnapshot {
                active_reservations: 2,
                active_reservation_artifacts: 1,
                workspace_identity: crate::runtime::workspace_identity_monitor::WorkspaceIdentityMonitorHealthSnapshot {
                    tracked_provider_runs: 3,
                    identity_changed_provider_runs: 1,
                    invalid_provider_runs: 1,
                    current_generation_total: 2,
                },
                external_changes: crate::io::ArtifactExternalChangeHealthSnapshot {
                    tracked_artifacts: 4,
                    externally_changed_artifacts: 2,
                    external_change_events: 5,
                    live_watcher_started: true,
                    live_watcher_scans: 7,
                    live_watcher_scan_errors: 0,
                },
            },
            ProjectionInvariantHealthSnapshot {
                checked_sessions: 1,
                checked_agents: 3,
                mismatches: Vec::new(),
            },
        );

        assert_eq!(projection.metadata.last_event_id, 7);
        assert_eq!(projection.session_command_lanes[0].lane_id, "session-1");
        assert_eq!(projection.session_command_lanes[0].queued_commands, 2);
        assert_eq!(projection.agent_command_lanes[0].lane_id, "agent-1");
        assert_eq!(projection.agent_command_lanes[0].queued_commands, 1);
        assert_eq!(
            projection.workflow_command_lanes[0].lane_id,
            "workflow-session-1"
        );
        assert_eq!(projection.workflow_command_lanes[0].queued_commands, 3);
        assert_eq!(
            projection.provider_runtime_lanes[0].lane_id,
            "provider-run-1"
        );
        assert_eq!(projection.provider_run_actor.enqueued_commands, 5);
        assert_eq!(projection.provider_run_actor.enqueue_rejections, 1);
        assert_eq!(projection.capability_executor.submitted_jobs, 8);
        assert_eq!(projection.capability_executor.running_jobs, 1);
        assert_eq!(projection.session_projection.active_prompts, 1);
        assert_eq!(projection.session_projection.queued_prompts, 2);
        assert_eq!(projection.agent_runtime_projection.projected_agents, 3);
        assert_eq!(projection.agent_runtime_projection.active_prompts, 1);
        assert!(projection.provider_catalog.cached);
        assert_eq!(projection.transport.active_connections, 2);
        assert_eq!(projection.transport.slow_consumer_closes, 1);
        assert_eq!(projection.terminal_stream.pending_output_records, 4);
        assert_eq!(
            projection.terminal_stream.trimmed_pending_output_recipients,
            1
        );
        assert_eq!(
            projection.workspace_coordination.worktree_collisions.len(),
            1
        );
        assert_eq!(projection.managed_io.active_reservations, 2);
        assert_eq!(projection.managed_io.active_reservation_artifacts, 1);
        assert_eq!(
            projection
                .managed_io
                .workspace_identity
                .invalid_provider_runs,
            1
        );
        assert_eq!(projection.managed_io.external_changes.tracked_artifacts, 4);
        assert_eq!(
            projection
                .managed_io
                .external_changes
                .external_change_events,
            5
        );
        assert!(projection.managed_io.external_changes.live_watcher_started);
        assert_eq!(projection.projection_invariants.checked_agents, 3);
        assert!(projection.projection_invariants.mismatches.is_empty());
    }

    #[test]
    fn agent_runtime_projection_reads_agent_prompt_state() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                &session_id,
                "cli-agent-runtime-projection",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attach should succeed");
        launch_dev_stub_provider(&mut app, &session_id, &agent_id);
        submit_prompt(
            &mut app,
            &session_id,
            attachment.id(),
            &agent_id,
            "first prompt",
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
        let store = AgentRuntimeProjectionStore::default();
        store.update_session(&session);

        let projection = store
            .get(&agent_id)
            .expect("agent projection should be available");
        assert_eq!(projection.session_id, session_id);
        assert_eq!(projection.agent_id, agent_id);
        assert!(projection.active_prompt.is_some());
        assert_eq!(
            projection
                .next_queued_prompt
                .as_ref()
                .map(|prompt| prompt.prompt()),
            Some("queued prompt")
        );
        assert_eq!(projection.queued_prompt_count, 1);
        assert_eq!(
            store
                .next_queued_prompt(&projection.session_id, &projection.agent_id)
                .as_ref()
                .map(|prompt| prompt.prompt()),
            Some("queued prompt")
        );
        assert_eq!(
            store.list_for_session(&projection.session_id),
            vec![projection]
        );
        assert_eq!(store.health_snapshot().active_prompts, 1);
        assert_eq!(store.health_snapshot().queued_prompts, 1);
    }

    #[test]
    fn agent_runtime_projection_can_refresh_one_agent_without_stomping_peers() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, first_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let first_agent_id = first_agent.id().to_string();
        let second_agent_id = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(CreateAgentRequest::new(&session_id, "claude-code").with_alias("peer"))
            .expect("second agent should spawn")
            .id()
            .to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                &session_id,
                "cli-agent-runtime-one-agent-refresh",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attach should succeed");
        for agent_id in [&first_agent_id, &second_agent_id] {
            launch_dev_stub_provider(&mut app, &session_id, agent_id);
        }

        submit_prompt(
            &mut app,
            &session_id,
            attachment.id(),
            &first_agent_id,
            "first active",
        );
        let first_only_snapshot = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(&session_id)
            .expect("first snapshot should load");
        submit_prompt(
            &mut app,
            &session_id,
            attachment.id(),
            &second_agent_id,
            "second active",
        );
        let both_snapshot = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(&session_id)
            .expect("second snapshot should load");

        let store = AgentRuntimeProjectionStore::default();
        store.update_session(&both_snapshot);
        assert!(store
            .get(&second_agent_id)
            .and_then(|projection| projection.active_prompt)
            .is_some());

        store.update_agent_from_session(&first_only_snapshot, &first_agent_id);
        assert!(
            store
                .get(&second_agent_id)
                .and_then(|projection| projection.active_prompt)
                .is_some(),
            "single-agent refresh should not erase newer peer prompt state"
        );
    }

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

    #[test]
    fn remote_relay_inventory_projection_requests_refresh_when_empty_or_stale() {
        let projection = RemoteRelayInventoryProjectionStore::default();
        assert!(projection.should_request_refresh(10_000, 5_000, 1_000));
        assert!(
            !projection.should_request_refresh(10_500, 5_000, 1_000),
            "refresh should respect the cooldown while the projection remains empty"
        );
        assert!(
            projection.should_request_refresh(16_000, 5_000, 1_000),
            "stale empty projection should request another refresh after the cooldown"
        );
    }
}
