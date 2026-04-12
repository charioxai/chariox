use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::history::SessionHistoryEntry;
use crate::kernel::workspace_coordinator::WorkspaceOperationClaimSnapshot;
use crate::provider::{OpenCodeProviderCatalog, ProviderProcessInfo, RuntimeProviderRun};
use crate::session::{unix_epoch_ms, RuntimeSession};
use crate::session_history_page::{paginate_session_history, SessionHistoryPage};
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
        self.state
            .lock()
            .expect("session projection lock should not be poisoned")
            .session_list = Some(sessions);
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
        let sessions = state.session_states.values().collect::<Vec<_>>();
        let active_prompts = sessions
            .iter()
            .flat_map(|session| session.prompt_states().values())
            .filter(|state| state.active_prompt().is_some())
            .count();
        let queued_prompts = sessions
            .iter()
            .flat_map(|session| session.prompt_states().values())
            .map(|state| state.queued_prompts().len())
            .sum();
        SessionProjectionHealthSnapshot {
            projected_sessions: state.session_states.len(),
            projected_session_list_entries: state.session_list.as_ref().map(Vec::len),
            active_prompts,
            queued_prompts,
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
        Ok(Self {
            metadata: ProjectionMetadata::new(1, last_event_id),
            session,
            provider_run,
        })
    }
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
    pub provider_runtime_lanes: Vec<ActorQueueSnapshot>,
    pub session_projection: SessionProjectionHealthSnapshot,
    pub provider_catalog: ProviderCatalogHealthSnapshot,
    pub transport: TransportHealthSnapshot,
    pub workspace_coordination: WorkspaceCoordinationHealthSnapshot,
}

impl DaemonHealthProjection {
    pub fn new(
        last_event_id: u64,
        session_command_lanes: Vec<ActorQueueSnapshot>,
        agent_command_lanes: Vec<ActorQueueSnapshot>,
        provider_runtime_lanes: Vec<ActorQueueSnapshot>,
        session_projection: SessionProjectionHealthSnapshot,
        provider_catalog: ProviderCatalogHealthSnapshot,
        transport: TransportHealthSnapshot,
        workspace_coordination: WorkspaceCoordinationHealthSnapshot,
    ) -> Self {
        Self {
            metadata: ProjectionMetadata::new(1, last_event_id),
            session_command_lanes,
            agent_command_lanes,
            provider_runtime_lanes,
            session_projection,
            provider_catalog,
            transport,
            workspace_coordination,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::kernel::projection::{
        ActorQueueSnapshot, DaemonHealthProjection, ProviderCatalogHealthSnapshot,
        SessionProjectionHealthSnapshot, SessionSnapshotProjection, SessionStateProjectionStore,
        TransportHealthSnapshot, WorkspaceCoordinationHealthSnapshot,
    };
    use crate::session::CreateSessionRequest;
    use crate::{DaemonApp, DaemonConfig};

    #[test]
    fn session_snapshot_projection_includes_metadata_and_agents() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");

        let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
            .expect("projection should build");

        assert_eq!(projection.metadata.projection_version, 1);
        assert_eq!(projection.metadata.last_event_id, 42);
        assert_eq!(projection.session.id(), session.id());
        assert_eq!(projection.session.agents().len(), 1);
    }

    #[test]
    fn daemon_health_projection_records_actor_queue_snapshots() {
        let projection = DaemonHealthProjection::new(
            7,
            vec![ActorQueueSnapshot::new("session-1", 128, 2)],
            vec![ActorQueueSnapshot::new("agent-1", 128, 1)],
            vec![ActorQueueSnapshot::new("provider-run-1", 1, 1)],
            SessionProjectionHealthSnapshot {
                projected_sessions: 3,
                projected_session_list_entries: Some(3),
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
            WorkspaceCoordinationHealthSnapshot {
                active_worktree_claims: vec![crate::kernel::projection::WorktreeClaimSnapshot {
                    workspace_id: "workspace-1".to_string(),
                    worktree_id: "worktree-1".to_string(),
                    session_ids: vec!["session-1".to_string(), "session-2".to_string()],
                }],
                worktree_collisions: vec![crate::kernel::projection::WorktreeClaimSnapshot {
                    workspace_id: "workspace-1".to_string(),
                    worktree_id: "worktree-1".to_string(),
                    session_ids: vec!["session-1".to_string(), "session-2".to_string()],
                }],
                active_operation_claims: Vec::new(),
            },
        );

        assert_eq!(projection.metadata.last_event_id, 7);
        assert_eq!(projection.session_command_lanes[0].lane_id, "session-1");
        assert_eq!(projection.session_command_lanes[0].queued_commands, 2);
        assert_eq!(projection.agent_command_lanes[0].lane_id, "agent-1");
        assert_eq!(projection.agent_command_lanes[0].queued_commands, 1);
        assert_eq!(
            projection.provider_runtime_lanes[0].lane_id,
            "provider-run-1"
        );
        assert_eq!(projection.session_projection.active_prompts, 1);
        assert!(projection.provider_catalog.cached);
        assert_eq!(projection.transport.active_connections, 2);
        assert_eq!(projection.transport.slow_consumer_closes, 1);
        assert_eq!(
            projection.workspace_coordination.worktree_collisions.len(),
            1
        );
    }

    #[test]
    fn workspace_coordination_snapshot_reports_worktree_collisions() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (first, _) = app
            .create_session(CreateSessionRequest::new("workspace-1", "shared-worktree"))
            .expect("first session should be created");
        let (second, _) = app
            .create_session(CreateSessionRequest::new("workspace-1", "shared-worktree"))
            .expect("second session should be created");
        let (other_workspace, _) = app
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
