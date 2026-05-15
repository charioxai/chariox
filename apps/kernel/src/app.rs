use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::runtime::{Handle, Runtime};

mod kernel_agent;
mod kernel_session;
mod prompt_activity;
mod prompt_lifecycle;
mod prompt_state_owner;
mod provider_activation;
mod provider_focus;
mod provider_launch_policy;
mod provider_liveness;
pub(crate) mod provider_output;
mod provider_processes;
mod provider_run_read;
mod provider_runtime;
mod provider_tracking;
mod remote_kernel_selection;
mod remote_lease;
mod session_runtime;
mod terminal_fanout;
pub(crate) mod terminal_input;
mod workflow_design_events;
pub(crate) mod workflow_runtime;

pub(crate) use prompt_activity::{
    ActivePromptState, ActiveTurnState, ActiveTurnStore, PromptActivityStore,
    PromptWorkspaceClaimStore,
};
pub(crate) use prompt_lifecycle::{
    serialize_remote_prompt_attachments, KernelPreparedPromptSubmission, KernelPromptAbortDispatch,
    KernelPromptCancellation, KernelPromptDispatch, KernelPromptSubmission,
    KernelRemotePromptDispatch,
};
pub(crate) use provider_tracking::{
    ProviderCatalogCacheStore, ProviderProcessTrackingStore, TrackedProviderProcess,
};
use remote_kernel_selection::{
    ensure_kernel_can_host_provider, kernel_presence_matches_ref, select_remote_kernel,
};
pub(crate) use workflow_design_events::WorkflowDesignEventStore;

use arroba_relay::protocol::{ClientTarget, DaemonRegistration, RelayKernelPresence};

use crate::agent::{
    AgentInstance, AgentService, AgentServiceStore, CreateAgentRequest, RemoteAgentBinding,
};
use crate::attachment::{AttachmentService, AttachmentServiceStore};
use crate::config::{DaemonConfig, HistoryArchiveMode};
use crate::durable_snapshot::{DurableKernelSnapshotPayload, DurableSnapshotScheduler};
use crate::durable_state::{DurableKernelStateStore, DurableStateEvent};
use crate::error::DaemonError;
use crate::execution_lease::{ExecutionLease, LeasedAgent, LeasedWorkflowTurnBinding};
use crate::history::{OperationalHistoryStore, SessionHistoryStore};
use crate::provider::{
    OpenCodeProviderCatalog, ProviderProcessInfo, ProviderProcessService,
    ProviderProcessServiceStore, ProviderRunOperationLanes, RuntimeProviderRun,
};
use crate::pty::PtyManager;
use crate::runtime::projection::{
    AgentRuntimeProjectionStore, DaemonConfigProjectionStore, ProviderCatalogProjectionStore,
    ProviderProcessProjectionStore, ProviderRunProjectionStore,
    RemoteRelayInventoryProjectionStore, SessionHistoryProjectionStore,
    SessionStateProjectionStore, TransportHealthStore,
};
use crate::runtime::prompt_state::PromptStateOwner;
use crate::runtime::workspace_coordinator::WorkspaceCoordinator;
use crate::session::{CreateSessionRequest, RuntimeSession, SessionService, SessionStateStore};
use crate::terminal::{TerminalStreamHealthStore, TerminalStreamStore};
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_client::RelayClientState;
use crate::transport::relay_discovery;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
pub(crate) use kernel_agent::KernelAgentService;
pub(crate) use kernel_session::{KernelSessionReadService, KernelSessionService};
pub(crate) use prompt_lifecycle::{ProviderPromptDispatcher, RemoteWorkflowTurnContextResolver};
pub(crate) use provider_activation::StartedProviderLaunch;
pub(crate) use provider_launch_policy::{
    failed_codex_resume_state_replacement, generate_runtime_mcp_auth_token,
    sanitize_resume_state_for_launch,
};
pub(crate) use provider_liveness::ProviderRunExitSessionSummary;
pub(crate) use provider_processes::ProviderLaunchProcessRuntime;
pub(crate) use provider_run_read::ProviderRunReadService;
pub(crate) use remote_lease::RemoteLeaseRuntime;

fn decode_durable_payload_field<T>(
    event: &DurableStateEvent,
    field: &'static str,
    operation: &'static str,
) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned,
{
    let value = event
        .payload
        .get(field)
        .cloned()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation,
            message: format!(
                "durable state event {} ({}) missing payload field {field}",
                event.event_id, event.kind
            ),
        })?;
    serde_json::from_value(value).map_err(|error| DaemonError::LocalTransport {
        operation,
        message: format!(
            "durable state event {} ({}) has invalid payload field {field}: {error}",
            event.event_id, event.kind
        ),
    })
}

pub struct DaemonApp {
    config: DaemonConfig,
    started_at_ms: u64,
    relay_client_state: Arc<tokio::sync::RwLock<RelayClientState>>,
    pub(crate) agents: AgentServiceStore,
    pub(crate) attachments: AttachmentServiceStore,
    pty: PtyManager,
    pub(crate) providers: ProviderProcessServiceStore,
    pub(crate) provider_catalog_cache: ProviderCatalogCacheStore,
    pub(crate) provider_process_tracking: ProviderProcessTrackingStore,
    pub(crate) active_turns: ActiveTurnStore,
    pub(crate) prompt_activity: PromptActivityStore,
    prompt_workspace_claims: PromptWorkspaceClaimStore,
    prompt_state_owner: PromptStateOwner,
    pub(crate) sessions: SessionStateStore,
    history: SessionHistoryStore,
    operational_history: OperationalHistoryStore,
    durable_state: DurableKernelStateStore,
    config_projection: DaemonConfigProjectionStore,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    history_projection: SessionHistoryProjectionStore,
    provider_catalog_projection: ProviderCatalogProjectionStore,
    provider_run_projection: ProviderRunProjectionStore,
    provider_process_projection: ProviderProcessProjectionStore,
    remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
    transport_health: TransportHealthStore,
    workspace_coordinator: WorkspaceCoordinator,
    terminal: TerminalStreamStore,
    workflow_design_events: WorkflowDesignEventStore,
    pending_structured_output_records: provider_output::StructuredOutputRecordStore,
    execution_leases: BTreeMap<String, ExecutionLease>,
    leased_agents: BTreeMap<String, LeasedAgent>,
    leased_workflow_turns: BTreeMap<String, LeasedWorkflowTurnBinding>,
    remote_git_turn_snapshots: crate::git_observer::GitTurnSnapshotStore,
    slices: crate::slice::SliceStore,
    next_execution_lease_number: u64,
    next_leased_agent_number: u64,
}

impl DaemonApp {
    pub(crate) fn artifact_attachment_segment(attachment_id: &str) -> String {
        attachment_id
            .chars()
            .map(|ch| match ch {
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
                ch if ch.is_control() => '_',
                ch => ch,
            })
            .collect()
    }

    pub(crate) fn attachment_artifact_root(
        session_id: &str,
        attachment_id: &str,
        category: &str,
    ) -> PathBuf {
        std::env::temp_dir()
            .join("arroba-session-artifacts")
            .join(session_id)
            .join(category)
            .join(Self::artifact_attachment_segment(attachment_id))
    }

    pub(crate) fn attachment_artifact_roots(session_id: &str, attachment_id: &str) -> [PathBuf; 2] {
        [
            Self::attachment_artifact_root(session_id, attachment_id, "screenshots"),
            Self::attachment_artifact_root(session_id, attachment_id, "transfers"),
        ]
    }

    pub fn bootstrap(config: DaemonConfig) -> Result<Self, DaemonError> {
        config.validate()?;

        let mut app = Self {
            agents: AgentServiceStore::new(AgentService::new()),
            attachments: AttachmentServiceStore::new(AttachmentService::new()),
            pty: PtyManager::new(),
            providers: ProviderProcessServiceStore::new(ProviderProcessService::new()),
            provider_catalog_cache: ProviderCatalogCacheStore::default(),
            provider_process_tracking: ProviderProcessTrackingStore::default(),
            active_turns: ActiveTurnStore::default(),
            prompt_activity: PromptActivityStore::default(),
            prompt_workspace_claims: PromptWorkspaceClaimStore::default(),
            prompt_state_owner: PromptStateOwner::default(),
            sessions: SessionStateStore::new(SessionService::new(&config)),
            history: SessionHistoryStore::new_with_read_delay(
                config.session_history_root.clone(),
                config.session_history_read_delay_ms,
            )?,
            operational_history: OperationalHistoryStore::open(config.operational_history_path())?,
            durable_state: DurableKernelStateStore::open(config.durable_state_path())?,
            config_projection: DaemonConfigProjectionStore::new(config.clone()),
            session_projection: SessionStateProjectionStore::default(),
            agent_runtime_projection: AgentRuntimeProjectionStore::default(),
            history_projection: SessionHistoryProjectionStore::default(),
            provider_catalog_projection: ProviderCatalogProjectionStore::default(),
            provider_run_projection: ProviderRunProjectionStore::default(),
            provider_process_projection: ProviderProcessProjectionStore::default(),
            remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore::default(),
            transport_health: TransportHealthStore::default(),
            workspace_coordinator: WorkspaceCoordinator::default(),
            terminal: TerminalStreamStore::new(),
            workflow_design_events: WorkflowDesignEventStore::default(),
            pending_structured_output_records:
                provider_output::StructuredOutputRecordStore::default(),
            execution_leases: BTreeMap::new(),
            leased_agents: BTreeMap::new(),
            leased_workflow_turns: BTreeMap::new(),
            remote_git_turn_snapshots: crate::git_observer::GitTurnSnapshotStore::default(),
            slices: crate::slice::SliceStore::default(),
            next_execution_lease_number: 0,
            next_leased_agent_number: 0,
            started_at_ms: crate::session::unix_epoch_ms(),
            relay_client_state: Arc::new(tokio::sync::RwLock::new(RelayClientState::default())),
            config,
        };
        app.restore_durable_state()?;
        Ok(app)
    }

    fn restore_durable_state(&mut self) -> Result<(), DaemonError> {
        let replay_after_sequence = match self.durable_state.latest_snapshot()? {
            Some(snapshot) => {
                self.restore_durable_state_snapshot(snapshot.payload)?;
                snapshot.sequence
            }
            None => 0,
        };
        for event in self
            .durable_state
            .load_events_after(replay_after_sequence)?
        {
            self.restore_durable_state_event(event)?;
        }
        self.reconcile_restored_runtime_state_after_restart()?;
        Ok(())
    }

    fn restore_durable_state_snapshot(
        &mut self,
        payload: serde_json::Value,
    ) -> Result<(), DaemonError> {
        let snapshot: DurableKernelSnapshotPayload =
            serde_json::from_value(payload).map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.restore_snapshot",
                message: error.to_string(),
            })?;
        let restored_session_ids: std::collections::BTreeSet<String> = snapshot
            .sessions
            .iter()
            .filter(|session| self.session_belongs_to_current_kernel(session))
            .map(|session| session.id().to_string())
            .collect();
        for session in snapshot.sessions {
            if !restored_session_ids.contains(session.id()) {
                continue;
            }
            self.sessions.restore_session(session);
        }
        let restored_slices = snapshot
            .slices
            .into_iter()
            .filter(|slice| slice.owner_kernel_id == self.config.daemon_id)
            .collect::<Vec<_>>();
        self.slices.restore_records(restored_slices);
        for agent in snapshot.agents {
            if !restored_session_ids.contains(agent.session_id()) {
                continue;
            }
            self.agents.restore_agent(agent);
        }
        self.refresh_restored_session_projections()?;
        Ok(())
    }

    fn session_belongs_to_current_kernel(&self, session: &RuntimeSession) -> bool {
        session.host_daemon_id() == self.config.daemon_id
    }

    fn refresh_restored_session_projections(&self) -> Result<(), DaemonError> {
        let sessions = self.sessions.read().store().list();
        for mut session in sessions {
            let agents = self.agents.get_session_agents(session.id());
            session.set_agents(agents);
            self.update_session_projection(session);
        }
        Ok(())
    }

    fn refresh_restored_agent_session_projection(
        &self,
        session_id: &str,
    ) -> Result<(), DaemonError> {
        let mut session = self.sessions.get_session(session_id)?;
        let agents = self.agents.get_session_agents(session_id);
        session.set_agents(agents);
        self.update_session_projection(session);
        Ok(())
    }

    fn reconcile_restored_runtime_state_after_restart(&self) -> Result<(), DaemonError> {
        let sessions = self.sessions.read().store().list();
        for mut session in sessions {
            let reconciliation = session.reconcile_after_kernel_restart();
            if !reconciliation.changed() {
                continue;
            }
            let agents = self.agents.get_session_agents(session.id());
            session.set_agents(agents);
            self.sessions.restore_session(session.clone());
            self.update_session_projection(session.clone());
            crate::logging::info_with_fields(
                "durable_state.restore",
                "reconciled runtime state after kernel restart",
                serde_json::json!({
                    "session_id": session.id(),
                    "cleared_active_provider_run": reconciliation.cleared_active_provider_run,
                    "interrupted_prompt_count": reconciliation.interrupted_prompt_count,
                    "stopped_workflow_run_count": reconciliation.stopped_workflow_run_count,
                }),
            );
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) fn save_durable_state_snapshot(&self) -> Result<(), DaemonError> {
        let sequence = self.durable_state.latest_event_sequence()?;
        let payload =
            DurableKernelSnapshotPayload::capture(&self.sessions, &self.agents, &self.slices);
        self.durable_state.save_snapshot(
            sequence,
            serde_json::to_value(payload).map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.encode_snapshot_payload",
                message: error.to_string(),
            })?,
        )?;
        Ok(())
    }

    pub(crate) fn durable_snapshot_scheduler(&self) -> Option<DurableSnapshotScheduler> {
        let interval_events = self.config.user_config.state.snapshot_interval_events? as u64;
        Some(DurableSnapshotScheduler::new(
            self.durable_state_store(),
            self.session_state_store(),
            self.agents.clone(),
            self.slices.clone(),
            interval_events,
        ))
    }

    fn restore_durable_state_event(&mut self, event: DurableStateEvent) -> Result<(), DaemonError> {
        match event.kind.as_str() {
            "session.created" => {
                let session: RuntimeSession = decode_durable_payload_field(
                    &event,
                    "session",
                    "durable_state.restore_session",
                )?;
                if !self.session_belongs_to_current_kernel(&session) {
                    return Ok(());
                }
                let default_agent: AgentInstance = decode_durable_payload_field(
                    &event,
                    "default_agent",
                    "durable_state.restore_default_agent",
                )?;
                self.sessions.restore_session(session.clone());
                self.agents.restore_agent(default_agent);
                self.update_session_projection(session);
            }
            "session.updated" => {
                let session: RuntimeSession = decode_durable_payload_field(
                    &event,
                    "session",
                    "durable_state.restore_session_update",
                )?;
                if !self.session_belongs_to_current_kernel(&session) {
                    return Ok(());
                }
                self.sessions.restore_session(session.clone());
                self.update_session_projection(session);
            }
            "agent.created" => {
                let agent: AgentInstance =
                    decode_durable_payload_field(&event, "agent", "durable_state.restore_agent")?;
                let session_id = agent.session_id().to_string();
                if self.sessions.get_session(&session_id).is_err() {
                    return Ok(());
                }
                self.agents.restore_agent(agent);
                self.refresh_restored_agent_session_projection(&session_id)?;
            }
            "agent.mcp_granted"
            | "agent.mcp_revoked"
            | "agent.skill_granted"
            | "agent.skill_revoked"
            | "agent.runtime_profile_updated"
            | "agent.updated" => {
                let agent: AgentInstance = decode_durable_payload_field(
                    &event,
                    "agent",
                    "durable_state.restore_agent_update",
                )?;
                let session_id = agent.session_id().to_string();
                if self.sessions.get_session(&session_id).is_err() {
                    return Ok(());
                }
                self.agents.restore_agent(agent);
                self.refresh_restored_agent_session_projection(&session_id)?;
            }
            "session.ended" => {
                let mut session: RuntimeSession = decode_durable_payload_field(
                    &event,
                    "session",
                    "durable_state.restore_ended_session",
                )?;
                if !self.session_belongs_to_current_kernel(&session) {
                    return Ok(());
                }
                self.agents.remove_session_agents(session.id());
                session.set_agents(Vec::new());
                self.sessions.restore_session(session.clone());
                self.update_session_projection(session);
            }
            "session.deleted" => {
                let mut session: RuntimeSession = decode_durable_payload_field(
                    &event,
                    "session",
                    "durable_state.restore_deleted_session",
                )?;
                if !self.session_belongs_to_current_kernel(&session) {
                    return Ok(());
                }
                self.agents.remove_session_agents(session.id());
                session.set_agents(Vec::new());
                self.sessions.remove_restored_session(session.id());
                self.session_projection.remove(session.id());
                self.history_projection.remove(session.id());
                self.agent_runtime_projection.update_session(&session);
            }
            "slice.created" | "slice.updated" => {
                let slice: crate::slice::SliceRecord =
                    decode_durable_payload_field(&event, "slice", "durable_state.restore_slice")?;
                if slice.owner_kernel_id == self.config.daemon_id {
                    let mut slices = self.slices.list();
                    slices.retain(|record| record.id != slice.id);
                    slices.push(slice);
                    self.slices.restore_records(slices);
                }
            }
            "slice.deleted" => {
                let slice: crate::slice::SliceRecord = decode_durable_payload_field(
                    &event,
                    "slice",
                    "durable_state.restore_deleted_slice",
                )?;
                if slice.owner_kernel_id == self.config.daemon_id {
                    let mut slices = self.slices.list();
                    slices.retain(|record| record.id != slice.id);
                    self.slices.restore_records(slices);
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn provider_run_operation_lanes(&self) -> ProviderRunOperationLanes {
        self.providers.run_operation_lanes()
    }

    pub fn config(&self) -> &DaemonConfig {
        &self.config
    }

    pub(crate) fn slices(&self) -> crate::slice::SliceStore {
        self.slices.clone()
    }

    pub(crate) fn relay_client_state(&self) -> Arc<tokio::sync::RwLock<RelayClientState>> {
        Arc::clone(&self.relay_client_state)
    }

    pub(crate) fn config_projection_store(&self) -> DaemonConfigProjectionStore {
        self.config_projection.clone()
    }

    pub(crate) fn configure_relay(
        &mut self,
        relay_url: Option<String>,
        relay_token: Option<String>,
    ) -> Result<(), DaemonError> {
        self.config.relay_url = relay_url
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        self.config.relay_token = relay_token
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        self.config.validate()?;
        self.config.persist_relay_config()?;
        self.config_projection.update(self.config.clone());
        Ok(())
    }

    pub(crate) fn persist_cloud_relay_profile(
        &mut self,
        profile: Option<crate::config::PersistedCloudRelayProfile>,
    ) -> Result<(), DaemonError> {
        self.config.persist_cloud_relay_profile(profile)?;
        self.config_projection.update(self.config.clone());
        Ok(())
    }

    pub(crate) fn set_user_config_value(
        &mut self,
        path: impl AsRef<str>,
        value: impl Into<String>,
    ) -> Result<(), DaemonError> {
        self.config.set_user_config_value(path, value)?;
        self.config_projection.update(self.config.clone());
        Ok(())
    }

    pub(crate) fn unset_user_config_value(
        &mut self,
        path: impl AsRef<str>,
    ) -> Result<(), DaemonError> {
        self.config.unset_user_config_value(path)?;
        self.config_projection.update(self.config.clone());
        Ok(())
    }

    pub(crate) fn session_state_store(&self) -> SessionStateStore {
        self.sessions.clone()
    }

    pub fn sessions(&self) -> SessionService {
        self.sessions.snapshot()
    }

    pub(crate) fn history_store(&self) -> SessionHistoryStore {
        self.history.clone()
    }

    pub(crate) fn operational_history_store(&self) -> OperationalHistoryStore {
        self.operational_history.clone()
    }

    pub(crate) fn durable_state_store(&self) -> DurableKernelStateStore {
        self.durable_state.clone()
    }

    pub(crate) fn history_archive_enabled(&self) -> bool {
        self.config.user_config.history.archive.mode == HistoryArchiveMode::External
    }

    pub(crate) fn load_session_history_entries(
        &self,
        session: &RuntimeSession,
        agent_id: Option<&str>,
    ) -> Result<Vec<crate::history::SessionHistoryEntry>, DaemonError> {
        let operational_entries = self
            .operational_history
            .load_session_history_entries(session.id(), agent_id)?;
        if !operational_entries.is_empty() {
            return Ok(operational_entries);
        }
        if self.operational_history.has_session_events(session.id())?
            || self
                .operational_history
                .legacy_fallback_disabled(session.id())?
        {
            return Ok(Vec::new());
        }
        let legacy_entries = self.history.load(session)?;
        Ok(match agent_id {
            Some(agent_id) => legacy_entries
                .into_iter()
                .filter(|entry| entry.agent_id.as_deref() == Some(agent_id))
                .collect(),
            None => legacy_entries,
        })
    }

    pub(crate) fn session_state_projection_store(&self) -> SessionStateProjectionStore {
        self.session_projection.clone()
    }

    pub(crate) fn agent_runtime_projection_store(&self) -> AgentRuntimeProjectionStore {
        self.agent_runtime_projection.clone()
    }

    pub(crate) fn prompt_state_owner(&self) -> PromptStateOwner {
        self.prompt_state_owner.clone()
    }

    pub(crate) fn prompt_id_allocator(&self) -> crate::session::PromptIdAllocator {
        self.sessions.prompt_id_allocator()
    }

    pub(crate) fn update_session_projection(&self, session: RuntimeSession) {
        self.agent_runtime_projection.update_session(&session);
        self.session_projection.update(session);
    }

    pub(crate) fn session_history_projection_store(&self) -> SessionHistoryProjectionStore {
        self.history_projection.clone()
    }

    pub(crate) fn provider_process_tracking_store(&self) -> ProviderProcessTrackingStore {
        self.provider_process_tracking.clone()
    }

    pub(crate) fn provider_catalog_projection_store(&self) -> ProviderCatalogProjectionStore {
        self.provider_catalog_projection.clone()
    }

    pub(crate) fn remote_relay_inventory_projection_store(
        &self,
    ) -> RemoteRelayInventoryProjectionStore {
        self.remote_relay_inventory_projection.clone()
    }

    pub(crate) fn update_provider_catalog_projection(&self, catalog: OpenCodeProviderCatalog) {
        self.provider_catalog_projection.update(catalog);
    }

    pub(crate) fn invalidate_provider_catalog_projection(&self) {
        self.provider_catalog_projection.invalidate();
    }

    pub(crate) fn provider_run_projection_store(&self) -> ProviderRunProjectionStore {
        self.provider_run_projection.clone()
    }

    pub(crate) fn provider_process_projection_store(&self) -> ProviderProcessProjectionStore {
        self.provider_process_projection.clone()
    }

    pub(crate) fn transport_health_store(&self) -> TransportHealthStore {
        self.transport_health.clone()
    }

    pub(crate) fn prompt_activity_store(&self) -> PromptActivityStore {
        self.prompt_activity.clone()
    }

    pub(crate) fn active_turn_store(&self) -> ActiveTurnStore {
        self.active_turns.clone()
    }

    pub(crate) fn prompt_workspace_claim_store(&self) -> PromptWorkspaceClaimStore {
        self.prompt_workspace_claims.clone()
    }

    pub(crate) fn structured_output_record_store(
        &self,
    ) -> provider_output::StructuredOutputRecordStore {
        self.pending_structured_output_records.clone()
    }

    pub(crate) fn workspace_coordinator(&self) -> WorkspaceCoordinator {
        self.workspace_coordinator.clone()
    }

    pub(crate) fn acquire_workflow_node_workspace_claim(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<(), DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let workspace_id = session.workspace_id().to_string();
        let worktree_id = self
            .agents
            .get_agent(agent_id)
            .ok()
            .and_then(|agent| agent.worktree_id().map(str::to_string))
            .unwrap_or_else(|| session.worktree_id().to_string());
        let claim = self.workspace_coordinator.acquire_worktree_write_claim(
            workspace_id,
            worktree_id,
            session_id,
            Some(format!("{}:{}", workflow_run_id, workflow_node_run_id)),
            "workflow_node_dispatch",
        )?;
        self.prompt_workspace_claims
            .insert(provider_run_id.to_string(), claim);
        Ok(())
    }

    pub(crate) fn release_prompt_workspace_claim(&mut self, provider_run_id: &str) -> bool {
        self.prompt_workspace_claims.remove(provider_run_id)
    }

    pub(crate) fn release_workflow_node_workspace_claim(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> bool {
        let owner = format!("{workflow_run_id}:{workflow_node_run_id}");
        self.prompt_workspace_claims.remove_matching(|claim| {
            claim.session_id == session_id
                && claim.attachment_id.as_deref() == Some(owner.as_str())
                && claim.operation == "workflow_node_dispatch"
        }) > 0
    }

    pub(crate) fn update_provider_run_projection(&self, run: RuntimeProviderRun) {
        self.provider_run_projection.update(run);
        self.provider_process_projection.invalidate();
    }

    pub(crate) fn update_provider_process_projection(&self, processes: Vec<ProviderProcessInfo>) {
        self.provider_process_projection.update_list(processes);
    }

    pub fn sessions_mut(&self) -> std::sync::MutexGuard<'_, SessionService> {
        self.sessions.write()
    }

    pub fn agents(&self) -> &AgentServiceStore {
        &self.agents
    }

    pub fn agents_mut(&self) -> std::sync::MutexGuard<'_, AgentService> {
        self.agents.write()
    }

    pub fn attachments(&self) -> &AttachmentServiceStore {
        &self.attachments
    }

    pub fn attachments_mut(&self) -> std::sync::MutexGuard<'_, AttachmentService> {
        self.attachments.write()
    }

    pub fn providers(&self) -> &ProviderProcessServiceStore {
        &self.providers
    }

    pub fn providers_mut(&self) -> std::sync::MutexGuard<'_, ProviderProcessService> {
        self.providers.write()
    }

    pub fn terminal(&self) -> &TerminalStreamStore {
        &self.terminal
    }

    pub(crate) fn terminal_health_store(&self) -> TerminalStreamHealthStore {
        self.terminal.health_store()
    }

    pub(crate) fn terminal_stream_store(&self) -> TerminalStreamStore {
        self.terminal.clone()
    }

    pub(crate) fn workflow_design_event_store(&self) -> WorkflowDesignEventStore {
        self.workflow_design_events.clone()
    }

    pub(crate) fn terminal_mut(&mut self) -> &TerminalStreamStore {
        &self.terminal
    }

    pub fn pty(&self) -> &PtyManager {
        &self.pty
    }

    pub(crate) fn pty_mut(&mut self) -> &mut PtyManager {
        &mut self.pty
    }

    #[doc(hidden)]
    pub fn create_session(
        &mut self,
        request: CreateSessionRequest,
    ) -> Result<(RuntimeSession, AgentInstance), DaemonError> {
        KernelSessionService::new(self).create_session(request)
    }

    #[doc(hidden)]
    pub fn attach(
        &mut self,
        request: crate::attachment::AttachRequest,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        KernelSessionService::new(self).attach(request)
    }

    #[doc(hidden)]
    pub fn detach(
        &mut self,
        attachment_id: &str,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        KernelSessionService::new(self).detach(attachment_id)
    }

    #[doc(hidden)]
    pub fn end_session(&mut self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        KernelSessionService::new(self).end_session(session_id)
    }

    #[doc(hidden)]
    pub fn spawn_agent(
        &mut self,
        request: CreateAgentRequest,
    ) -> Result<AgentInstance, DaemonError> {
        KernelSessionService::new(self).spawn_agent(request)
    }

    #[doc(hidden)]
    pub fn focus_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<AgentInstance, DaemonError> {
        KernelSessionService::new(self).focus_agent(session_id, agent_id)
    }

    #[doc(hidden)]
    pub fn resize_terminal(
        &mut self,
        session_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), DaemonError> {
        KernelSessionService::new(self).resize_terminal(session_id, cols, rows)
    }

    #[doc(hidden)]
    pub fn send_terminal_input(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        provider_run_id: Option<&str>,
        bytes: &[u8],
    ) -> Result<(), DaemonError> {
        let provider_run_id = match provider_run_id {
            Some(provider_run_id) => {
                crate::app::ProviderRunReadService::new(self)
                    .ensure_provider_run_in_session(session_id, provider_run_id)?;
                provider_run_id.to_string()
            }
            None => self
                .sessions()
                .get_session(session_id)?
                .active_provider_run_id()
                .ok_or_else(|| DaemonError::NoActiveProviderRun {
                    session_id: session_id.to_string(),
                })?
                .to_string(),
        };
        crate::app::terminal_input::ProviderTerminalInput::new(self).send_provider_input(
            session_id,
            &provider_run_id,
            attachment_id,
            bytes,
        )
    }

    #[doc(hidden)]
    pub fn pump_active_prompt_outputs(&mut self) {
        crate::app::provider_output::pump_active_prompt_outputs(self);
    }

    #[doc(hidden)]
    pub fn session_history_page(
        &self,
        session_id: &str,
        agent_id: Option<&str>,
        round_count: Option<usize>,
        max_chars: Option<usize>,
        before_entry_index: Option<usize>,
        before_entry_char_offset: Option<usize>,
    ) -> Result<crate::session_history_page::SessionHistoryPage, DaemonError> {
        let session = self.sessions().get_session(session_id)?;
        let entries = self.load_session_history_entries(&session, agent_id)?;
        self.session_history_projection_store()
            .update_entries(session.id(), entries.clone());
        Ok(crate::runtime::projection::page_history_entries(
            entries,
            agent_id,
            round_count,
            max_chars,
            before_entry_index,
            before_entry_char_offset,
        ))
    }

    pub fn relay_registration(&mut self) -> DaemonRegistration {
        let available_providers = self.providers.registry().registered_adapter_keys();
        DaemonRegistration {
            auth_token: self.config.relay_token.clone().unwrap_or_default(),
            daemon_id: self.config.daemon_id.clone(),
            machine_id: self.config.host_machine_id.clone(),
            machine_alias: self.config.host_machine_alias.clone(),
            os_name: Some(self.config.os_name.clone()),
            kernel_started_at_ms: self.started_at_ms,
            daemon_alias: self.config.daemon_alias.clone(),
            kernel_alias: self.config.daemon_alias.clone(),
            public_key: self.config.relay_public_key.clone(),
            capabilities: vec![
                "kernel_websocket".to_string(),
                "relay_request_proxy".to_string(),
                "relay_peer_transport".to_string(),
                "execution_lease_management".to_string(),
            ],
            available_providers,
            accepting_remote_leases: self.config.accept_remote_leases,
            leased_agent_count: self.leased_agents.len() as u32,
            local_session_count: self.sessions().list_sessions().len() as u32,
        }
    }

    pub(crate) fn kernel_ref_is_local(&self, kernel_ref: &str) -> bool {
        let kernel_ref = kernel_ref.trim();
        !kernel_ref.is_empty()
            && (self.config.daemon_id == kernel_ref
                || self.config.daemon_alias.as_deref() == Some(kernel_ref))
    }

    pub(crate) fn spawn_worker_agent(
        &mut self,
        mut request: CreateAgentRequest,
        kernel_ref: &str,
    ) -> Result<AgentInstance, DaemonError> {
        let relay_override = self.slice_relay_config_for_kernel_ref(kernel_ref);
        let relay_config = relay_override.as_ref().unwrap_or(&self.config);
        let worker_kernel = self.select_remote_kernel_by_ref_with_config(
            kernel_ref,
            &request.provider,
            relay_config,
        )?;
        request.kernel_ref = None;
        request.worktree_id = None;
        request.worktree_placement = None;
        let session_store = self.session_state_store();
        let agent = {
            let mut sessions = session_store.write();
            self.agents.create_agent(request, &mut sessions)?
        };
        let remote_setup =
            self.bind_remote_agent_to_worker(&agent, &worker_kernel, None, relay_override);
        if remote_setup.is_err() {
            let mut sessions = session_store.write();
            let _ = self.agents.destroy_agent(agent.id(), &mut sessions);
        }
        remote_setup
    }

    fn bind_remote_agent_to_worker(
        &mut self,
        agent: &AgentInstance,
        worker_kernel: &RelayKernelPresence,
        worktree_placement: Option<crate::agent::GitWorktreePlacement>,
        relay_override: Option<DaemonConfig>,
    ) -> Result<AgentInstance, DaemonError> {
        let relay_config = relay_override.as_ref().unwrap_or(&self.config);
        let session = self.sessions().get_session(agent.session_id())?;
        let effective_config =
            crate::session::effective_agent_execution_config(&session, Some(agent));
        let target = ClientTarget {
            daemon_id: Some(worker_kernel.kernel_id.clone()),
            daemon_alias: None,
        };
        let lease = match self.block_on_relay_future(send_peer_request_via_temporary_connection(
            relay_config,
            target.clone(),
            RelayPeerRequest::CreateExecutionLease {
                home_kernel_id: self.config.daemon_id.clone(),
                home_session_id: agent.session_id().to_string(),
                home_agent_id: agent.id().to_string(),
                owner_user_id: agent.owner_user_id().to_string(),
            },
        ))? {
            RelayPeerResponse::ExecutionLeaseCreated { lease } => lease,
            other => {
                return Err(DaemonError::LocalTransport {
                    operation: "create remote execution lease",
                    message: format!("unexpected peer response: {other:?}"),
                });
            }
        };
        let leased_agent =
            match self.block_on_relay_future(send_peer_request_via_temporary_connection(
                relay_config,
                target.clone(),
                RelayPeerRequest::SpawnLeasedAgent {
                    lease_id: lease.id.clone(),
                    provider: agent.provider().to_string(),
                    model: agent.model().map(ToOwned::to_owned),
                    effort: agent.effort().map(ToOwned::to_owned),
                    execution_mode: Some(effective_config.mode),
                    permission_level: Some(effective_config.permission_level),
                    worktree_id: agent.worktree_id().map(ToOwned::to_owned),
                    worktree_placement,
                },
            )) {
                Ok(RelayPeerResponse::LeasedAgentSpawned { leased_agent }) => leased_agent,
                Ok(other) => {
                    let _ = self.block_on_relay_future(send_peer_request_via_temporary_connection(
                        relay_config,
                        target,
                        RelayPeerRequest::DestroyExecutionLease {
                            lease_id: lease.id.clone(),
                        },
                    ));
                    return Err(DaemonError::LocalTransport {
                        operation: "spawn remote leased agent",
                        message: format!("unexpected peer response: {other:?}"),
                    });
                }
                Err(error) => {
                    let _ = self.block_on_relay_future(send_peer_request_via_temporary_connection(
                        relay_config,
                        target,
                        RelayPeerRequest::DestroyExecutionLease {
                            lease_id: lease.id.clone(),
                        },
                    ));
                    return Err(error);
                }
            };
        let bound = self.agents.bind_remote_execution(
            agent.id(),
            RemoteAgentBinding {
                worker_kernel_id: worker_kernel.kernel_id.clone(),
                worker_machine_id: worker_kernel.machine_id.clone(),
                execution_lease_id: lease.id,
                leased_agent_id: leased_agent.id,
                relay_url: relay_override
                    .as_ref()
                    .and_then(|config| config.relay_url.clone()),
                relay_token: relay_override
                    .as_ref()
                    .and_then(|config| config.relay_token.clone()),
            },
        )?;
        self.ensure_remote_agent_skill_packages(&bound)?;
        self.ensure_remote_agent_mcp_availability(&bound)?;
        Ok(bound)
    }

    pub(crate) fn refresh_remote_agent_binding(
        &mut self,
        agent_id: &str,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self.agents.get_agent(agent_id)?;
        let Some(remote_execution) = agent.remote_execution().cloned() else {
            return Err(DaemonError::LocalTransport {
                operation: "refresh remote agent binding",
                message: format!("agent `{agent_id}` is not remote-backed"),
            });
        };
        let worker_kernel = self.select_remote_kernel_for_machine(
            &remote_execution.worker_machine_id,
            agent.provider(),
        )?;
        let rebound = self.bind_remote_agent_to_worker(&agent, &worker_kernel, None, None)?;
        self.durable_state_store().append_event(
            "agent.updated",
            Some(rebound.id().to_string()),
            serde_json::json!({
                "agent": &rebound,
                "source": "remote_agent_binding_refreshed",
            }),
        )?;
        if let Ok(session) = self.sessions.get_session(rebound.session_id()) {
            self.update_session_projection(session);
        }
        Ok(rebound)
    }

    pub(crate) fn move_agent_to_remote(
        &mut self,
        session_id: &str,
        agent_ref: &str,
        machine_ref: &str,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .agents
            .get_agent(agent_ref)
            .or_else(|_| self.agents.get_agent_by_ref(agent_ref))?;
        if agent.session_id() != session_id {
            return Err(DaemonError::LocalTransport {
                operation: "move agent to remote",
                message: format!("agent `{agent_ref}` does not belong to session `{session_id}`"),
            });
        }
        if agent.remote_execution().is_some() {
            return Err(DaemonError::LocalTransport {
                operation: "move agent to remote",
                message: format!("agent `{agent_ref}` is already remote-backed"),
            });
        }
        if self
            .providers
            .get_run_for_agent(session_id, agent.id())
            .is_some()
        {
            return Err(DaemonError::LocalTransport {
                operation: "move agent to remote",
                message:
                    "agent has a provider run; stop or destroy the provider run before moving it"
                        .to_string(),
            });
        }
        let worker_kernel = self.select_remote_kernel_for_machine(machine_ref, agent.provider())?;
        self.bind_remote_agent_to_worker(&agent, &worker_kernel, None, None)
    }

    fn ensure_remote_agent_skill_packages(
        &mut self,
        agent: &AgentInstance,
    ) -> Result<(), DaemonError> {
        let Some(remote_execution) = agent.remote_execution() else {
            return Ok(());
        };
        if agent.skill_grants().is_empty() {
            return Ok(());
        }
        let session = self.sessions.get_session(agent.session_id())?;
        let mut roots = vec![crate::skill::ArrobaSkillRegistry::project_root(
            session.workspace_id(),
        )];
        if let Some(user_root) = crate::skill::ArrobaSkillRegistry::user_root() {
            roots.push(user_root);
        }
        let registry = crate::skill::ArrobaSkillRegistry::new(roots);
        let packages = agent
            .skill_grants()
            .iter()
            .map(|grant| {
                registry
                    .package(grant)?
                    .ok_or_else(|| DaemonError::LocalTransport {
                        operation: "ensure remote agent skill packages",
                        message: format!("skill `{grant}` is granted but is not installed"),
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if packages.is_empty() {
            return Ok(());
        }
        let relay_config = self.relay_config_for_remote_execution(remote_execution);
        let response = self.block_on_relay_future(send_peer_request_via_temporary_connection(
            &relay_config,
            ClientTarget {
                daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::EnsureRemoteSkillPackages {
                context: crate::transport::relay_peer::RemoteSkillSyncContext {
                    home_kernel_id: self.config.daemon_id.clone(),
                    home_session_id: agent.session_id().to_string(),
                    home_agent_id: agent.id().to_string(),
                    leased_agent_id: remote_execution.leased_agent_id.clone(),
                },
                packages,
            },
        ))?;
        match response {
            RelayPeerResponse::RemoteSkillPackagesEnsured { .. } => Ok(()),
            other => Err(DaemonError::LocalTransport {
                operation: "ensure remote agent skill packages",
                message: format!("unexpected remote skill sync response: {other:?}"),
            }),
        }
    }

    fn ensure_remote_agent_mcp_availability(
        &mut self,
        agent: &AgentInstance,
    ) -> Result<(), DaemonError> {
        let Some(remote_execution) = agent.remote_execution() else {
            return Ok(());
        };
        if agent.mcp_grants().is_empty() {
            return Ok(());
        }
        let session = self.sessions.get_session(agent.session_id())?;
        let mut roots = vec![crate::mcp::ArrobaMcpRegistry::project_root(
            session.workspace_id(),
        )];
        if let Some(user_root) = crate::mcp::ArrobaMcpRegistry::user_root() {
            roots.push(user_root);
        }
        let registry = crate::mcp::ArrobaMcpRegistry::new(roots);
        let required_mcps = agent
            .mcp_grants()
            .iter()
            .map(|grant| {
                let config = registry
                    .get(grant)?
                    .ok_or_else(|| DaemonError::LocalTransport {
                        operation: "ensure remote agent MCP availability",
                        message: format!("MCP `{grant}` is granted but is not installed"),
                    })?;
                let definition_hash = config.definition_hash()?;
                Ok(crate::transport::relay_peer::RequiredRemoteMcp {
                    config,
                    definition_hash,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if required_mcps.is_empty() {
            return Ok(());
        }
        let relay_config = self.relay_config_for_remote_execution(remote_execution);
        let response = self.block_on_relay_future(send_peer_request_via_temporary_connection(
            &relay_config,
            ClientTarget {
                daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::CheckRemoteMcpAvailability {
                context: crate::transport::relay_peer::RemoteMcpCheckContext {
                    home_kernel_id: self.config.daemon_id.clone(),
                    home_session_id: agent.session_id().to_string(),
                    home_agent_id: agent.id().to_string(),
                    leased_agent_id: remote_execution.leased_agent_id.clone(),
                },
                required_mcps,
            },
        ))?;
        match response {
            RelayPeerResponse::RemoteMcpAvailabilityChecked { results } => {
                let unavailable = results
                    .iter()
                    .filter(|result| {
                        !matches!(
                            result.status,
                            crate::transport::relay_peer::RemoteMcpAvailabilityStatus::Available
                        )
                    })
                    .collect::<Vec<_>>();
                if unavailable.is_empty() {
                    Ok(())
                } else {
                    Err(DaemonError::LocalTransport {
                        operation: "ensure remote agent MCP availability",
                        message: format!(
                            "remote MCP unavailable on worker. Install the matching MCP definition in the worker project or user registry, then retry: {unavailable:?}"
                        ),
                    })
                }
            }
            other => Err(DaemonError::LocalTransport {
                operation: "ensure remote agent MCP availability",
                message: format!("unexpected remote MCP availability response: {other:?}"),
            }),
        }
    }

    fn select_remote_kernel_for_machine(
        &self,
        machine_ref: &str,
        provider: &str,
    ) -> Result<RelayKernelPresence, DaemonError> {
        let machine_ref = crate::config::DaemonConfig::resolve_registered_machine_ref(machine_ref)
            .unwrap_or_else(|| machine_ref.to_string());
        let (_, projected_kernels) = self.remote_relay_inventory_projection_store().snapshot();
        if let Some(kernel) = select_remote_kernel(projected_kernels, &machine_ref, provider) {
            return Ok(kernel);
        }
        let kernels = self.block_on_relay_future(
            relay_discovery::list_live_kernels_for_machine(&self.config, &machine_ref),
        )?;
        select_remote_kernel(kernels, &machine_ref, provider).ok_or_else(|| {
            DaemonError::NoRemoteKernelAvailable {
                machine_ref,
                provider: provider.to_string(),
            }
        })
    }

    fn select_remote_kernel_by_ref_with_config(
        &self,
        kernel_ref: &str,
        provider: &str,
        relay_config: &DaemonConfig,
    ) -> Result<RelayKernelPresence, DaemonError> {
        let kernel_ref = kernel_ref.trim();
        if relay_config.relay_url == self.config.relay_url
            && relay_config.relay_token == self.config.relay_token
        {
            let (_, projected_kernels) = self.remote_relay_inventory_projection_store().snapshot();
            if let Some(kernel) = projected_kernels
                .into_iter()
                .find(|kernel| kernel_presence_matches_ref(kernel, kernel_ref))
            {
                return ensure_kernel_can_host_provider(kernel, kernel_ref, provider);
            }
        }
        let kernel =
            self.block_on_relay_future(relay_discovery::get_live_kernel(relay_config, kernel_ref))?;
        ensure_kernel_can_host_provider(kernel, kernel_ref, provider)
    }

    fn slice_relay_config_for_kernel_ref(&self, kernel_ref: &str) -> Option<DaemonConfig> {
        let slice = self.slices.resolve_by_worker_kernel_ref(kernel_ref)?;
        let relay = crate::slice::local_docker_private_relay(&slice);
        let mut config = self.config.clone();
        config.relay_url = Some(relay.relay_url);
        config.relay_token = Some(relay.relay_token);
        config.cloud_relay = None;
        Some(config)
    }

    pub(crate) fn relay_config_for_remote_execution(
        &self,
        remote_execution: &RemoteAgentBinding,
    ) -> DaemonConfig {
        let mut config = self.config.clone();
        if let (Some(relay_url), Some(relay_token)) = (
            remote_execution.relay_url.clone(),
            remote_execution.relay_token.clone(),
        ) {
            config.relay_url = Some(relay_url);
            config.relay_token = Some(relay_token);
            config.cloud_relay = None;
        }
        config
    }

    pub(crate) fn block_on_relay_future<F, T>(&self, future: F) -> Result<T, DaemonError>
    where
        F: std::future::Future<Output = Result<T, DaemonError>>,
    {
        if let Ok(handle) = Handle::try_current() {
            tokio::task::block_in_place(|| handle.block_on(future))
        } else {
            Runtime::new()
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "create relay runtime",
                    message: error.to_string(),
                })?
                .block_on(future)
        }
    }

    pub fn startup_message(&self) -> String {
        format!(
            "arroba daemon {} ready on machine {} ({})",
            self.config.daemon_id,
            self.config.host_machine_id,
            self.config.kernel_websocket_url()
        )
    }

    pub fn shutdown_cleanup(&mut self) -> Result<(), DaemonError> {
        let session_ids = self
            .sessions
            .list_sessions()
            .into_iter()
            .map(|session| session.id().to_string())
            .collect::<Vec<_>>();
        let mut first_error = None;

        for session_id in session_ids {
            if let Err(error) = self.shutdown_cleanup_session_runtime(&session_id) {
                crate::logging::error_with_fields(
                    "daemon.shutdown",
                    "failed to clean session runtime during daemon shutdown",
                    serde_json::json!({
                        "session_id": session_id,
                        "error": error.to_string(),
                    }),
                );
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }

        if let Some(error) = first_error {
            return Err(error);
        }

        Ok(())
    }

    fn shutdown_cleanup_session_runtime(&mut self, session_id: &str) -> Result<(), DaemonError> {
        let removed_attachments = self.attachments.remove_session_attachments(session_id);
        for attachment in &removed_attachments {
            match self
                .sessions
                .write()
                .remove_attachment_from_session(session_id, attachment.id())
            {
                Ok(_) | Err(DaemonError::AttachmentNotInSession { .. }) => {}
                Err(error) => return Err(error),
            }
        }

        let terminated_runs = self
            .providers
            .terminate_session_runs_provider_only(session_id)?;
        let terminated_run_ids = terminated_runs
            .runs()
            .iter()
            .map(|outcome| outcome.run().id().to_string())
            .collect::<Vec<_>>();
        for outcome in terminated_runs.into_runs() {
            let run = outcome.into_run();
            if self
                .sessions
                .get_session(session_id)?
                .active_provider_run_id()
                == Some(run.id())
            {
                self.sessions.set_active_provider_run(session_id, None)?;
            }
            self.update_provider_run_projection(run.clone());
            provider_runtime::ProviderProcessTracker::new(self).remove_run(run.id())?;
        }

        for run in self.providers.list_runs() {
            if run.session_id() == session_id {
                crate::transport::flow_control::clear_prompt_activity(self, run.id());
            }
        }
        self.prompt_owner_remove_session(session_id);

        let mut session = self.sessions.get_session(session_id)?;
        let reconciliation = session.reconcile_after_kernel_restart();
        let agents = self.agents.get_session_agents(session_id);
        session.set_agents(agents);
        self.sessions.restore_session(session.clone());
        self.update_session_projection(session.clone());

        crate::logging::info_with_fields(
            "daemon.shutdown",
            "session runtime cleaned for daemon shutdown",
            serde_json::json!({
                "session_id": session_id,
                "session_status": session.status(),
                "removed_attachment_ids": removed_attachments
                    .iter()
                    .map(|attachment| attachment.id().to_string())
                    .collect::<Vec<_>>(),
                "terminated_provider_run_ids": terminated_run_ids,
                "cleared_active_provider_run": reconciliation.cleared_active_provider_run,
                "interrupted_prompt_count": reconciliation.interrupted_prompt_count,
                "stopped_workflow_run_count": reconciliation.stopped_workflow_run_count,
            }),
        );

        Ok(())
    }

    pub async fn run(self) -> Result<(), DaemonError> {
        let app = Arc::new(tokio::sync::Mutex::new(self));
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let relay_state = {
            let app = app.lock().await;
            app.relay_client_state()
        };
        let relay_task = tokio::spawn(crate::transport::relay_client::run_daemon_relay_connector(
            Arc::clone(&app),
            relay_state,
            shutdown_rx,
        ));

        let result =
            crate::runtime_transport::run_kernel_websocket_server(Arc::clone(&app), async {
                let _ = tokio::signal::ctrl_c().await;
                let _ = shutdown_tx.send(true);
            })
            .await;

        let _ = shutdown_tx.send(true);
        let _ = relay_task.await;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::LaunchProviderRequest;
    use crate::session::CreateSessionRequest;

    #[test]
    fn durable_restore_keeps_sessions_bound_to_their_kernel_id() {
        let state_path = std::env::temp_dir().join("arroba-tests").join(format!(
            "shared-kernel-state-{}.db",
            crate::session::unix_epoch_ms()
        ));
        let mut config_a = DaemonConfig::for_tests();
        config_a.daemon_id = "kernel-a".to_string();
        config_a.user_config.state.path = Some(state_path.display().to_string());
        let session_id = {
            let mut app = DaemonApp::bootstrap(config_a.clone()).expect("kernel a should boot");
            let (session, _) = app
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should create");
            session.id().to_string()
        };

        let mut config_b = DaemonConfig::for_tests();
        config_b.daemon_id = "kernel-b".to_string();
        config_b.user_config.state.path = Some(state_path.display().to_string());
        let app_b = DaemonApp::bootstrap(config_b).expect("kernel b should boot");
        assert!(app_b.sessions().list_sessions().is_empty());

        let app_a = DaemonApp::bootstrap(config_a).expect("kernel a should reboot");
        assert!(app_a.sessions().get_session(&session_id).is_ok());

        let _ = std::fs::remove_file(state_path);
    }

    #[test]
    fn durable_restore_keeps_slices_bound_to_their_owner_kernel_id() {
        let state_path = std::env::temp_dir().join("arroba-tests").join(format!(
            "shared-slice-state-{}.db",
            crate::session::unix_epoch_ms()
        ));
        let mut config_a = DaemonConfig::for_tests();
        config_a.daemon_id = "kernel-a".to_string();
        config_a.user_config.state.path = Some(state_path.display().to_string());
        let slice_id = {
            let app = DaemonApp::bootstrap(config_a.clone()).expect("kernel a should boot");
            let slice = app
                .slices()
                .create(
                    &app.config().daemon_id,
                    &app.config().host_machine_id,
                    crate::slice::CreateSliceInput {
                        name: "linux-dev".to_string(),
                        backend: crate::slice::SliceBackendKind::LocalDocker,
                        os: "linux".to_string(),
                        workspace_mount: Some("/repo".to_string()),
                        worker_kernel_ref: None,
                        display_url: Some("http://127.0.0.1:6080".to_string()),
                        now_ms: 42,
                    },
                )
                .expect("slice should create");
            app.durable_state_store()
                .append_event(
                    "slice.created",
                    Some(slice.id.clone()),
                    serde_json::json!({ "slice": &slice }),
                )
                .expect("slice event should persist");
            slice.id
        };

        let mut config_b = DaemonConfig::for_tests();
        config_b.daemon_id = "kernel-b".to_string();
        config_b.user_config.state.path = Some(state_path.display().to_string());
        let app_b = DaemonApp::bootstrap(config_b).expect("kernel b should boot");
        assert!(app_b.slices().list().is_empty());

        let app_a = DaemonApp::bootstrap(config_a).expect("kernel a should reboot");
        assert_eq!(
            app_a
                .slices()
                .resolve("linux-dev")
                .expect("slice should restore")
                .id,
            slice_id
        );

        let _ = std::fs::remove_file(state_path);
    }

    #[test]
    fn slice_worker_kernel_refs_resolve_to_private_relay_config() {
        let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let slice = app
            .slices()
            .create(
                &app.config().daemon_id,
                &app.config().host_machine_id,
                crate::slice::CreateSliceInput {
                    name: "linux-dev".to_string(),
                    backend: crate::slice::SliceBackendKind::LocalDocker,
                    os: "linux".to_string(),
                    workspace_mount: Some("/repo".to_string()),
                    worker_kernel_ref: None,
                    display_url: Some("http://127.0.0.1:6080".to_string()),
                    now_ms: 42,
                },
            )
            .expect("slice should create");

        let relay_config = app
            .slice_relay_config_for_kernel_ref(&slice.worker_kernel_ref)
            .expect("slice worker ref should have relay config");

        assert_eq!(
            relay_config.relay_url.as_deref(),
            Some("ws://127.0.0.1:43130")
        );
        assert_eq!(
            relay_config.relay_token.as_deref(),
            Some("slice-local-daemon-test-slice-1")
        );
        assert!(relay_config.cloud_relay.is_none());
    }

    #[test]
    fn daemon_restart_restores_sessions_after_shutdown_cleanup() {
        let state_path = std::env::temp_dir().join("arroba-tests").join(format!(
            "restart-preserves-sessions-{}.db",
            crate::session::unix_epoch_ms()
        ));
        let mut config = DaemonConfig::for_tests();
        config.user_config.state.path = Some(state_path.display().to_string());

        let session_id = {
            let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
            let (session, _) = app
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should create");
            app.shutdown_cleanup()
                .expect("shutdown should clean runtime without ending session");
            session.id().to_string()
        };

        let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
        let restored = app
            .sessions()
            .get_session(&session_id)
            .expect("session should restore after daemon restart");
        assert_ne!(restored.status(), crate::session::SessionStatus::Ended);
        assert!(
            app.agents().get_session_agents(&session_id).len() == 1,
            "default agent should restore for preserved session"
        );

        let _ = std::fs::remove_file(state_path);
    }

    #[test]
    fn durable_restore_republishes_agent_runtime_profile_to_session_projection() {
        let state_path = std::env::temp_dir().join("arroba-tests").join(format!(
            "restart-agent-projection-{}.db",
            crate::session::unix_epoch_ms()
        ));
        let mut config = DaemonConfig::for_tests();
        config.user_config.state.path = Some(state_path.display().to_string());

        let session_id = {
            let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
            let (session, agent) = app
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should create");
            app.launch_provider(
                LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(agent.id()),
            )
            .expect("provider should launch and persist runtime profile");
            session.id().to_string()
        };

        let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
        let projected = app
            .session_projection
            .get(&session_id)
            .expect("session projection should restore");
        let projected_agent = projected
            .agents()
            .first()
            .expect("projected session should include restored agent");

        assert_eq!(projected_agent.provider(), "claude-code");
        assert_eq!(projected_agent.model(), Some("sonnet"));

        let _ = std::fs::remove_file(state_path);
    }
}
