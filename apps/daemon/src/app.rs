use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::runtime::{Handle, Runtime};

mod kernel_agent;
mod kernel_session;
mod kernel_workflow;
mod prompt_lifecycle;
mod prompt_state_owner;
pub(crate) mod provider_output;
mod provider_runtime;
mod remote_lease;
mod session_runtime;
mod terminal_fanout;
pub(crate) mod terminal_input;
pub(crate) mod workflow_runtime;

pub(crate) use prompt_lifecycle::{
    serialize_remote_prompt_attachments, KernelPreparedPromptSubmission, KernelPromptAbortDispatch,
    KernelPromptCancellation, KernelPromptDispatch, KernelPromptSubmission,
    KernelRemotePromptDispatch,
};

use arroba_relay::protocol::{ClientTarget, DaemonRegistration, RelayKernelPresence};

use crate::agent::{
    AgentInstance, AgentService, AgentServiceStore, CreateAgentRequest, RemoteAgentBinding,
};
use crate::attachment::{AttachmentService, AttachmentServiceStore};
use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::execution_lease::{ExecutionLease, LeasedAgent, LeasedWorkflowTurnBinding};
use crate::history::SessionHistoryStore;
use crate::kernel::projection::{
    AgentRuntimeProjectionStore, DaemonConfigProjectionStore, ProviderCatalogProjectionStore,
    ProviderProcessProjectionStore, ProviderRunProjectionStore, SessionHistoryProjectionStore,
    SessionStateProjectionStore, TransportHealthStore,
};
use crate::kernel::prompt_state::PromptStateOwner;
use crate::kernel::workspace_coordinator::{WorkspaceClaimGuard, WorkspaceCoordinator};
use crate::provider::{
    OpenCodeProviderCatalog, ProviderProcessInfo, ProviderProcessService,
    ProviderRunOperationLanes, RuntimeProviderRun,
};
use crate::pty::PtyManager;
use crate::session::{RuntimeSession, SessionService, SessionStateStore};
use crate::terminal::{TerminalStreamHealthStore, TerminalStreamStore};
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_client::RelayClientState;
use crate::transport::relay_discovery;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
pub(crate) use kernel_agent::KernelAgentService;
pub(crate) use kernel_session::{KernelSessionReadService, KernelSessionService};
pub(crate) use kernel_workflow::KernelWorkflowService;
pub(crate) use prompt_lifecycle::{ProviderPromptDispatcher, RemoteWorkflowTurnContextResolver};
pub(crate) use provider_runtime::{ProviderRunReadService, StartedProviderLaunch};
pub(crate) use remote_lease::RemoteLeaseRuntime;

pub struct DaemonApp {
    config: DaemonConfig,
    started_at_ms: u64,
    relay_client_state: Arc<tokio::sync::RwLock<RelayClientState>>,
    pub(crate) agents: AgentServiceStore,
    pub(crate) attachments: AttachmentServiceStore,
    pty: PtyManager,
    pub(crate) providers: ProviderProcessService,
    pub(crate) provider_catalog_cache: Option<(Instant, OpenCodeProviderCatalog)>,
    pub(crate) provider_process_tracking: ProviderProcessTrackingStore,
    pub(crate) prompt_activity: PromptActivityStore,
    prompt_workspace_claims: BTreeMap<String, WorkspaceClaimGuard>,
    pub(crate) prompt_idle_timeout: Duration,
    prompt_state_owner: PromptStateOwner,
    pub(crate) sessions: SessionStateStore,
    history: SessionHistoryStore,
    config_projection: DaemonConfigProjectionStore,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    history_projection: SessionHistoryProjectionStore,
    provider_catalog_projection: ProviderCatalogProjectionStore,
    provider_run_projection: ProviderRunProjectionStore,
    provider_process_projection: ProviderProcessProjectionStore,
    transport_health: TransportHealthStore,
    workspace_coordinator: WorkspaceCoordinator,
    terminal: TerminalStreamStore,
    pending_structured_output_records: provider_output::StructuredOutputRecordStore,
    execution_leases: BTreeMap<String, ExecutionLease>,
    leased_agents: BTreeMap<String, LeasedAgent>,
    leased_workflow_turns: BTreeMap<String, LeasedWorkflowTurnBinding>,
    next_execution_lease_number: u64,
    next_leased_agent_number: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct ActivePromptState {
    pub(crate) last_output_at: Option<Instant>,
    pub(crate) saw_response_content: bool,
    pub(crate) completion_recorded: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PromptActivityStore {
    inner: Arc<Mutex<BTreeMap<String, ActivePromptState>>>,
}

impl PromptActivityStore {
    pub(crate) fn read(&self) -> MutexGuard<'_, BTreeMap<String, ActivePromptState>> {
        self.inner.lock().expect("prompt activity mutex poisoned")
    }

    pub(crate) fn write(&self) -> MutexGuard<'_, BTreeMap<String, ActivePromptState>> {
        self.inner.lock().expect("prompt activity mutex poisoned")
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TrackedProviderProcess {
    pub(crate) process_id: String,
    pub(crate) pid: Option<u32>,
    pub(crate) endpoint_mode: crate::provider::AgentEndpointMode,
    pub(crate) process_label: String,
    pub(crate) started_at_ms: u64,
    pub(crate) owner_provider_run_ids: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProviderProcessTrackingStore {
    inner: Arc<Mutex<ProviderProcessTrackingState>>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProviderProcessTrackingState {
    pub(crate) processes: BTreeMap<String, TrackedProviderProcess>,
    pub(crate) run_processes: BTreeMap<String, String>,
}

impl ProviderProcessTrackingStore {
    pub(crate) fn read(&self) -> MutexGuard<'_, ProviderProcessTrackingState> {
        self.inner
            .lock()
            .expect("provider process tracking mutex poisoned")
    }

    pub(crate) fn write(&self) -> MutexGuard<'_, ProviderProcessTrackingState> {
        self.inner
            .lock()
            .expect("provider process tracking mutex poisoned")
    }

    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> ProviderProcessTrackingState {
        self.read().clone()
    }
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

        Ok(Self {
            agents: AgentServiceStore::new(AgentService::new()),
            attachments: AttachmentServiceStore::new(AttachmentService::new()),
            pty: PtyManager::new(),
            providers: ProviderProcessService::new(),
            provider_catalog_cache: None,
            provider_process_tracking: ProviderProcessTrackingStore::default(),
            prompt_activity: PromptActivityStore::default(),
            prompt_workspace_claims: BTreeMap::new(),
            prompt_idle_timeout: prompt_idle_timeout(),
            prompt_state_owner: PromptStateOwner::default(),
            sessions: SessionStateStore::new(SessionService::new(&config)),
            history: SessionHistoryStore::new_with_read_delay(
                config.session_history_root.clone(),
                config.session_history_read_delay_ms,
            )?,
            config_projection: DaemonConfigProjectionStore::new(config.clone()),
            session_projection: SessionStateProjectionStore::default(),
            agent_runtime_projection: AgentRuntimeProjectionStore::default(),
            history_projection: SessionHistoryProjectionStore::default(),
            provider_catalog_projection: ProviderCatalogProjectionStore::default(),
            provider_run_projection: ProviderRunProjectionStore::default(),
            provider_process_projection: ProviderProcessProjectionStore::default(),
            transport_health: TransportHealthStore::default(),
            workspace_coordinator: WorkspaceCoordinator::default(),
            terminal: TerminalStreamStore::new(),
            pending_structured_output_records:
                provider_output::StructuredOutputRecordStore::default(),
            execution_leases: BTreeMap::new(),
            leased_agents: BTreeMap::new(),
            leased_workflow_turns: BTreeMap::new(),
            next_execution_lease_number: 0,
            next_leased_agent_number: 0,
            started_at_ms: crate::session::unix_epoch_ms(),
            relay_client_state: Arc::new(tokio::sync::RwLock::new(RelayClientState::default())),
            config,
        })
    }

    pub(crate) fn provider_run_operation_lanes(&self) -> ProviderRunOperationLanes {
        self.providers.run_operation_lanes()
    }

    pub fn config(&self) -> &DaemonConfig {
        &self.config
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

    pub(crate) fn session_state_store(&self) -> SessionStateStore {
        self.sessions.clone()
    }

    pub fn sessions(&self) -> SessionService {
        self.sessions.snapshot()
    }

    pub(crate) fn history_store(&self) -> SessionHistoryStore {
        self.history.clone()
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

    pub(crate) fn remove_session_projection(&self, session_id: &str) {
        self.session_projection.remove(session_id);
        self.agent_runtime_projection.remove_session(session_id);
    }

    pub(crate) fn session_history_projection_store(&self) -> SessionHistoryProjectionStore {
        self.history_projection.clone()
    }

    pub(crate) fn provider_catalog_projection_store(&self) -> ProviderCatalogProjectionStore {
        self.provider_catalog_projection.clone()
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

    pub(crate) fn workspace_coordinator(&self) -> WorkspaceCoordinator {
        self.workspace_coordinator.clone()
    }

    pub(crate) fn acquire_prompt_workspace_claim(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        attachment_id: Option<&str>,
    ) -> Result<(), DaemonError> {
        if self.prompt_workspace_claims.contains_key(provider_run_id) {
            return Ok(());
        }
        let session = self.sessions.get_session(session_id)?;
        let workspace_id = session.workspace_id().to_string();
        let worktree_id = self
            .agents
            .get_agent(agent_id)
            .ok()
            .and_then(|agent| agent.worktree_id().map(str::to_string))
            .unwrap_or_else(|| session.worktree_id().to_string());
        let claim = self.workspace_coordinator.acquire_provider_prompt_claim(
            workspace_id,
            worktree_id,
            session_id,
            attachment_id.map(str::to_string),
        )?;
        self.prompt_workspace_claims
            .insert(provider_run_id.to_string(), claim);
        Ok(())
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
        self.prompt_workspace_claims
            .remove(provider_run_id)
            .is_some()
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

    pub fn providers(&self) -> &ProviderProcessService {
        &self.providers
    }

    pub fn providers_mut(&mut self) -> &mut ProviderProcessService {
        &mut self.providers
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

    pub(crate) fn terminal_mut(&mut self) -> &TerminalStreamStore {
        &self.terminal
    }

    pub fn pty(&self) -> &PtyManager {
        &self.pty
    }

    pub fn pump_active_prompt_outputs(&mut self) {
        self.reap_structured_prompt_jobs();
        let sessions = self.sessions.list_sessions();
        for session in sessions {
            let recipient_attachment_ids =
                self.attachments.list_session_attachment_ids(session.id());
            let mut agent_ids = session
                .agents()
                .iter()
                .map(|agent| agent.id().to_string())
                .collect::<Vec<_>>();
            agent_ids.extend(session.prompt_states().keys().cloned());
            agent_ids.sort();
            agent_ids.dedup();
            for agent_id in agent_ids {
                if self
                    .prompt_state_owner
                    .active_prompt_for_agent_snapshot(&session, &agent_id)
                    .is_none()
                {
                    continue;
                }
                let Some(provider_run_id) = self
                    .providers
                    .get_run_for_agent(session.id(), &agent_id)
                    .map(|run| run.id().to_string())
                else {
                    continue;
                };
                if let Err(error) = provider_output::ProviderOutputPump::new(self)
                    .pump_provider_output(provider_output::ProviderOutputPumpRequest {
                        session_id: session.id(),
                        provider_run_id: &provider_run_id,
                        recipient_attachment_ids: recipient_attachment_ids.clone(),
                    })
                {
                    crate::logging::warn_with_fields(
                        "daemon.app",
                        "background prompt pump failed",
                        serde_json::json!({
                            "session_id": session.id(),
                            "provider_run_id": provider_run_id,
                            "agent_id": agent_id,
                            "error": error.to_string(),
                        }),
                    );
                }
            }
        }
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

    fn spawn_remote_agent(
        &mut self,
        request: CreateAgentRequest,
        machine_ref: &str,
    ) -> Result<AgentInstance, DaemonError> {
        let worker_kernel =
            self.select_remote_kernel_for_machine(machine_ref, &request.provider)?;
        let session_store = self.session_state_store();
        let agent = {
            let mut sessions = session_store.write();
            self.agents.create_agent(request, &mut sessions)?
        };
        let remote_setup = self.bind_remote_agent_to_worker(&agent, &worker_kernel);
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
    ) -> Result<AgentInstance, DaemonError> {
        let target = ClientTarget {
            daemon_id: Some(worker_kernel.kernel_id.clone()),
            daemon_alias: None,
        };
        let lease = match self.block_on_relay_future(send_peer_request_via_temporary_connection(
            &self.config,
            target.clone(),
            RelayPeerRequest::CreateExecutionLease {
                home_kernel_id: self.config.daemon_id.clone(),
                home_session_id: agent.session_id().to_string(),
                home_agent_id: agent.id().to_string(),
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
                &self.config,
                target.clone(),
                RelayPeerRequest::SpawnLeasedAgent {
                    lease_id: lease.id.clone(),
                    provider: agent.provider().to_string(),
                    model: agent.model().map(ToOwned::to_owned),
                    effort: agent.effort().map(ToOwned::to_owned),
                },
            )) {
                Ok(RelayPeerResponse::LeasedAgentSpawned { leased_agent }) => leased_agent,
                Ok(other) => {
                    let _ = self.block_on_relay_future(send_peer_request_via_temporary_connection(
                        &self.config,
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
                        &self.config,
                        target,
                        RelayPeerRequest::DestroyExecutionLease {
                            lease_id: lease.id.clone(),
                        },
                    ));
                    return Err(error);
                }
            };
        self.agents.bind_remote_execution(
            agent.id(),
            RemoteAgentBinding {
                worker_kernel_id: worker_kernel.kernel_id.clone(),
                worker_machine_id: worker_kernel.machine_id.clone(),
                execution_lease_id: lease.id,
                leased_agent_id: leased_agent.id,
            },
        )
    }

    fn select_remote_kernel_for_machine(
        &self,
        machine_ref: &str,
        provider: &str,
    ) -> Result<RelayKernelPresence, DaemonError> {
        let machine_ref = crate::config::DaemonConfig::resolve_registered_machine_ref(machine_ref)
            .unwrap_or_else(|| machine_ref.to_string());
        let kernels = self.block_on_relay_future(
            relay_discovery::list_live_kernels_for_machine(&self.config, &machine_ref),
        )?;
        kernels
            .into_iter()
            .filter(|kernel| kernel.accepting_remote_leases)
            .filter(|kernel| {
                kernel
                    .available_providers
                    .iter()
                    .any(|candidate| candidate == provider)
            })
            .min_by_key(|kernel| {
                (
                    kernel.leased_agent_count,
                    kernel.local_session_count,
                    kernel.kernel_id.clone(),
                )
            })
            .ok_or_else(|| DaemonError::NoRemoteKernelAvailable {
                machine_ref,
                provider: provider.to_string(),
            })
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
            if let Err(error) = KernelSessionService::new(self).end_session(&session_id) {
                crate::logging::error_with_fields(
                    "daemon.shutdown",
                    "failed to end session during daemon shutdown",
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
            crate::kernel_transport::run_kernel_websocket_server(Arc::clone(&app), async {
                let _ = tokio::signal::ctrl_c().await;
                let _ = shutdown_tx.send(true);
            })
            .await;

        let _ = shutdown_tx.send(true);
        let _ = relay_task.await;
        result
    }
}

fn prompt_idle_timeout() -> Duration {
    Duration::from_millis(
        std::env::var("ARROBA_PROMPT_IDLE_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(750),
    )
}
