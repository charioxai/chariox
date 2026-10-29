mod bounds;
mod cache;
pub(crate) use cache::DaemonHealthCache;

use crate::error::DaemonError;
use crate::local::provider_requests::PROVIDER_CATALOG_CACHE_TTL;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::provider::ProviderRunOperationLanes;
use crate::runtime::agent_actor::AgentRuntime;
use crate::runtime::capability_executor::CapabilityExecutorHealthStore;
use crate::runtime::projection::{
    AgentRuntimeProjectionStore, DaemonHealthProjection, ProviderCatalogProjectionStore,
    ProviderRunProjectionStore, RemoteExecutionHealthSnapshot, RemoteExtensionSyncHealthSnapshot,
    SessionStateProjectionStore, SliceLifecycleHealthSnapshot, TransportHealthStore,
};
use crate::runtime::session_actor::SessionRuntime;
use crate::runtime::state::KernelRuntimeState;
use crate::runtime::workflow_actor::WorkflowRuntime;
use crate::runtime::workspace_coordinator::WorkspaceCoordinator;
use crate::terminal::TerminalStreamHealthStore;

#[derive(Clone)]
pub(crate) struct DaemonHealthProjectionInput {
    pub(crate) last_event_id: u64,
    pub(crate) request_metrics: crate::runtime::request_metrics::RequestMetrics,
    pub(crate) session_runtime: SessionRuntime,
    pub(crate) agent_runtime: AgentRuntime,
    pub(crate) workflow_runtime: WorkflowRuntime,
    pub(crate) provider_runtime_lanes: ProviderRunOperationLanes,
    pub(crate) capability_health: CapabilityExecutorHealthStore,
    pub(crate) session_projection: SessionStateProjectionStore,
    pub(crate) agent_runtime_projection: AgentRuntimeProjectionStore,
    pub(crate) provider_catalog_projection: ProviderCatalogProjectionStore,
    pub(crate) provider_run_projection: ProviderRunProjectionStore,
    pub(crate) transport_health: TransportHealthStore,
    pub(crate) terminal_health: TerminalStreamHealthStore,
    pub(crate) workspace_coordinator: WorkspaceCoordinator,
    pub(crate) runtime_state: KernelRuntimeState,
}

pub(crate) async fn build_daemon_health_projection(
    input: DaemonHealthProjectionInput,
) -> DaemonHealthProjection {
    let agents = input.runtime_state.list_agents();
    let active_turns = input.runtime_state.active_turn_snapshot();
    let provider_runs = input.provider_run_projection.list();
    let active_agent_ids = input
        .agent_runtime_projection
        .list()
        .into_iter()
        .filter(|projection| projection.active_prompt.is_some())
        .map(|projection| projection.agent_id)
        .collect::<std::collections::BTreeSet<_>>();
    let mut projection = DaemonHealthProjection::new(
        input.last_event_id,
        input.session_runtime.queue_snapshots().await,
        input.agent_runtime.queue_snapshots().await,
        input.workflow_runtime.queue_snapshots().await,
        input.provider_runtime_lanes.queue_snapshots(),
        input.provider_runtime_lanes.health_snapshot(),
        crate::runtime::process_health::KernelProcessHealthSnapshot::current(),
        input.capability_health.snapshot(),
        input.session_projection.health_snapshot(),
        input.agent_runtime_projection.health_snapshot(),
        input
            .provider_catalog_projection
            .health_snapshot(PROVIDER_CATALOG_CACHE_TTL),
        input
            .provider_run_projection
            .health_snapshot(input.runtime_state.list_session_health_snapshots()),
        input.transport_health.snapshot(
            crate::runtime_transport::RECENT_EVENT_LIMIT,
            crate::runtime_transport::COMMAND_RESULT_CACHE_LIMIT,
            crate::runtime_transport::process_inbound_request_limit(),
        ),
        input.terminal_health.snapshot(),
        SliceLifecycleHealthSnapshot::from_slices(&input.runtime_state.list_slices()),
        RemoteExecutionHealthSnapshot::from_agents_with_active_agent_ids(
            &agents,
            &active_agent_ids,
        ),
        RemoteExtensionSyncHealthSnapshot::from_agents(&agents),
        input
            .session_projection
            .workspace_coordination_snapshot(input.workspace_coordinator.active_claims()),
        input
            .runtime_state
            .workspace_live_sync_health_snapshot()
            .await,
        input.session_projection.invariant_snapshot(
            &input.agent_runtime_projection,
            &agents,
            &active_turns,
            &provider_runs,
        ),
    );
    projection.app_lock = crate::runtime::app_lock::app_lock_health_snapshot();
    projection.durable_writer = input
        .runtime_state
        .durable_state_store()
        .writer_health_snapshot();
    projection.request_timings = input.request_metrics.snapshot();
    bounds::bound(projection)
}

pub(crate) async fn execute_daemon_health_request(
    cache: &DaemonHealthCache,
    input: DaemonHealthProjectionInput,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::GetDaemonHealth(_) => Ok(LocalDaemonResponse::DaemonHealth {
            projection: cache.snapshot(input).await,
        }),
        _ => Err(DaemonError::LocalTransport {
            operation: "daemon health request",
            message: "unsupported daemon health request".to_string(),
        }),
    }
}
