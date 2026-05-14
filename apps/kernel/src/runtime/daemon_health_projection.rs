use crate::local::provider_requests::PROVIDER_CATALOG_CACHE_TTL;
use crate::provider::ProviderRunOperationLanes;
use crate::runtime::agent_actor::AgentRuntime;
use crate::runtime::capability_executor::CapabilityExecutorHealthStore;
use crate::runtime::projection::{
    AgentRuntimeProjectionStore, DaemonHealthProjection, ProviderCatalogProjectionStore,
    SessionStateProjectionStore, TransportHealthStore,
};
use crate::runtime::session_actor::SessionRuntime;
use crate::runtime::state::KernelRuntimeState;
use crate::runtime::workflow_actor::WorkflowRuntime;
use crate::runtime::workspace_coordinator::WorkspaceCoordinator;
use crate::terminal::TerminalStreamHealthStore;

pub(crate) struct DaemonHealthProjectionInput<'a> {
    pub(crate) last_event_id: u64,
    pub(crate) session_runtime: &'a SessionRuntime,
    pub(crate) agent_runtime: &'a AgentRuntime,
    pub(crate) workflow_runtime: &'a WorkflowRuntime,
    pub(crate) provider_runtime_lanes: &'a ProviderRunOperationLanes,
    pub(crate) capability_health: &'a CapabilityExecutorHealthStore,
    pub(crate) session_projection: &'a SessionStateProjectionStore,
    pub(crate) agent_runtime_projection: &'a AgentRuntimeProjectionStore,
    pub(crate) provider_catalog_projection: &'a ProviderCatalogProjectionStore,
    pub(crate) transport_health: &'a TransportHealthStore,
    pub(crate) terminal_health: &'a TerminalStreamHealthStore,
    pub(crate) workspace_coordinator: &'a WorkspaceCoordinator,
    pub(crate) runtime_state: &'a KernelRuntimeState,
}

pub(crate) async fn build_daemon_health_projection(
    input: DaemonHealthProjectionInput<'_>,
) -> DaemonHealthProjection {
    DaemonHealthProjection::new(
        input.last_event_id,
        input.session_runtime.queue_snapshots().await,
        input.agent_runtime.queue_snapshots().await,
        input.workflow_runtime.queue_snapshots().await,
        input.provider_runtime_lanes.queue_snapshots(),
        input.provider_runtime_lanes.health_snapshot(),
        input.capability_health.snapshot(),
        input.session_projection.health_snapshot(),
        input.agent_runtime_projection.health_snapshot(),
        input
            .provider_catalog_projection
            .health_snapshot(PROVIDER_CATALOG_CACHE_TTL),
        input.transport_health.snapshot(
            crate::runtime_transport::RECENT_EVENT_LIMIT,
            crate::runtime_transport::COMMAND_RESULT_CACHE_LIMIT,
            crate::runtime_transport::INBOUND_REQUEST_LIMIT,
        ),
        input.terminal_health.snapshot(),
        input
            .session_projection
            .workspace_coordination_snapshot(input.workspace_coordinator.active_claims()),
        input.runtime_state.managed_io_health_snapshot().await,
        input
            .session_projection
            .invariant_snapshot(input.agent_runtime_projection),
    )
}
