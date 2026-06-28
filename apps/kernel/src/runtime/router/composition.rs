use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, RwLock};

use crate::app::{
    AttachedProviderTranscriptCursorStore, DaemonApp, ExternalProviderSessionIndexStore,
    PromptActivityStore, WorkflowDesignEventStore,
};
use crate::history::{OperationalHistoryStore, SessionHistoryStore};
use crate::provider::ProviderRunOperationLanes;
use crate::runtime::agent_actor::AgentRuntime;
use crate::runtime::capability_executor::{CapabilityExecutorHealthStore, CapabilityRuntimeStore};
use crate::runtime::native_interaction_bridge::install_provider_native_interaction_bridge;
use crate::runtime::projection::{
    AgentRuntimeProjectionStore, DaemonConfigProjectionStore, ProviderCatalogProjectionStore,
    ProviderProcessProjectionStore, ProviderRunProjectionStore,
    RemoteRelayInventoryProjectionStore, SessionStateProjectionStore, TransportHealthStore,
};
use crate::runtime::prompt_state::PromptStateOwner;
use crate::runtime::provider_launch_executor::ProviderLaunchPendingTracker;
use crate::runtime::session_actor::{FocusedAgentProjection, SessionRuntime};
use crate::runtime::state::KernelRuntimeState;
use crate::runtime::terminal_output_executor::TerminalOutputExecutor;
use crate::runtime::workflow_actor::WorkflowRuntime;
use crate::runtime::workspace_coordinator::WorkspaceCoordinator;
use crate::session::PromptIdAllocator;
use crate::terminal::{TerminalStreamHealthStore, TerminalStreamStore};
use crate::transport::relay_client::RelayClientState;

use super::CommandRouter;

impl CommandRouter {
    #[cfg(test)]
    pub(crate) fn with_interactive_capacity(
        app: Arc<Mutex<DaemonApp>>,
        interactive_capacity: usize,
    ) -> Self {
        Self::with_interactive_capacity_and_provider_lanes(
            app,
            interactive_capacity,
            ProviderRunOperationLanes::default(),
        )
    }

    #[cfg(test)]
    pub(crate) fn with_interactive_and_session_capacity(
        app: Arc<Mutex<DaemonApp>>,
        interactive_capacity: usize,
        session_capacity: usize,
    ) -> Self {
        let _interactive_capacity = interactive_capacity;
        compose_command_router(
            app,
            ProviderRunOperationLanes::default(),
            session_capacity,
            TransportHealthStore::default(),
        )
    }

    pub(crate) fn with_interactive_capacity_and_provider_lanes(
        app: Arc<Mutex<DaemonApp>>,
        interactive_capacity: usize,
        provider_runtime_lanes: ProviderRunOperationLanes,
    ) -> Self {
        Self::with_interactive_capacity_provider_lanes_and_transport_health(
            app,
            interactive_capacity,
            provider_runtime_lanes,
            TransportHealthStore::default(),
        )
    }

    pub(crate) fn with_interactive_capacity_from_app(
        app: Arc<Mutex<DaemonApp>>,
        interactive_capacity: usize,
    ) -> Self {
        let started = Instant::now();
        let (provider_runtime_lanes, transport_health) = {
            let app = loop {
                if let Ok(app) = app.try_lock() {
                    break app;
                }
                if started.elapsed() >= Duration::from_secs(5) {
                    panic!("CommandRouter could not acquire the app lock during bootstrap");
                }
                std::thread::sleep(Duration::from_millis(2));
            };
            (
                app.provider_run_operation_lanes(),
                app.transport_health_store(),
            )
        };
        Self::with_interactive_capacity_provider_lanes_and_transport_health(
            app,
            interactive_capacity,
            provider_runtime_lanes,
            transport_health,
        )
    }

    pub(crate) fn with_interactive_capacity_provider_lanes_and_transport_health(
        app: Arc<Mutex<DaemonApp>>,
        interactive_capacity: usize,
        provider_runtime_lanes: ProviderRunOperationLanes,
        transport_health: TransportHealthStore,
    ) -> Self {
        let _interactive_capacity = interactive_capacity;
        compose_command_router(
            app,
            provider_runtime_lanes,
            crate::runtime::session_actor::SESSION_COMMAND_QUEUE_LIMIT,
            transport_health,
        )
    }
}

pub(super) struct RouterProjectionStores {
    pub(super) history_store: SessionHistoryStore,
    pub(super) operational_history_store: OperationalHistoryStore,
    pub(super) session_projection: SessionStateProjectionStore,
    pub(super) provider_catalog_projection: ProviderCatalogProjectionStore,
    pub(super) provider_run_projection: ProviderRunProjectionStore,
    pub(super) provider_process_projection: ProviderProcessProjectionStore,
    pub(super) remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
    pub(super) agent_runtime_projection: AgentRuntimeProjectionStore,
    pub(super) config_projection: DaemonConfigProjectionStore,
    pub(super) session_store: crate::session::SessionStateStore,
    pub(super) agent_store: crate::agent::AgentServiceStore,
    pub(super) attachment_store: crate::attachment::AttachmentServiceStore,
    pub(super) provider_store: crate::provider::ProviderProcessServiceStore,
    pub(super) provider_process_tracking: crate::app::ProviderProcessTrackingStore,
    pub(super) external_provider_sessions: ExternalProviderSessionIndexStore,
    pub(super) attached_provider_transcript_cursors: AttachedProviderTranscriptCursorStore,
    pub(super) slice_store: crate::slice::SliceStore,
    pub(super) active_turns: crate::app::ActiveTurnStore,
    pub(super) prompt_activity: PromptActivityStore,
    pub(super) prompt_workspace_claims: crate::app::PromptWorkspaceClaimStore,
    pub(super) structured_output_records: crate::app::provider_output::StructuredOutputRecordStore,
    pub(super) durable_state_store: crate::durable_state::DurableKernelStateStore,
    pub(super) relay_state: Arc<RwLock<RelayClientState>>,
    pub(super) terminal_health: TerminalStreamHealthStore,
    pub(super) terminal_stream: TerminalStreamStore,
    pub(super) workflow_design_events: WorkflowDesignEventStore,
    pub(super) metaagent_events: crate::runtime::metaagent_event::MetaagentEventStore,
    pub(super) metaagent_trace_subscriptions:
        crate::runtime::metaagent_trace::MetaagentTraceSubscriptionStore,
    pub(super) workspace_coordinator: WorkspaceCoordinator,
    pub(super) prompt_state_owner: PromptStateOwner,
    pub(super) prompt_id_allocator: PromptIdAllocator,
}

pub(super) fn router_projection_stores(app: &Arc<Mutex<DaemonApp>>) -> RouterProjectionStores {
    let started = Instant::now();
    let app = loop {
        if let Ok(app) = app.try_lock() {
            break app;
        }
        if started.elapsed() >= Duration::from_secs(5) {
            panic!("CommandRouter could not acquire the app lock during bootstrap");
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    RouterProjectionStores {
        history_store: app.history_store(),
        operational_history_store: app.operational_history_store(),
        session_projection: app.session_state_projection_store(),
        provider_catalog_projection: app.provider_catalog_projection_store(),
        provider_run_projection: app.provider_run_projection_store(),
        provider_process_projection: app.provider_process_projection_store(),
        remote_relay_inventory_projection: app.remote_relay_inventory_projection_store(),
        agent_runtime_projection: app.agent_runtime_projection_store(),
        config_projection: app.config_projection_store(),
        session_store: app.session_state_store(),
        agent_store: app.agents().clone(),
        attachment_store: app.attachments().clone(),
        provider_store: app.providers().clone(),
        provider_process_tracking: app.provider_process_tracking_store(),
        external_provider_sessions: app.external_provider_session_index_store(),
        attached_provider_transcript_cursors: app.attached_provider_transcript_cursor_store(),
        slice_store: app.slices(),
        active_turns: app.active_turn_store(),
        prompt_activity: app.prompt_activity_store(),
        prompt_workspace_claims: app.prompt_workspace_claim_store(),
        structured_output_records: app.structured_output_record_store(),
        durable_state_store: app.durable_state_store(),
        relay_state: app.relay_client_state(),
        terminal_health: app.terminal_health_store(),
        terminal_stream: app.terminal_stream_store(),
        workflow_design_events: app.workflow_design_event_store(),
        metaagent_events: app.metaagent_event_store(),
        metaagent_trace_subscriptions: app.metaagent_trace_subscription_store(),
        workspace_coordinator: app.workspace_coordinator(),
        prompt_state_owner: app.prompt_state_owner(),
        prompt_id_allocator: app.prompt_id_allocator(),
    }
}

pub(super) fn compose_command_router(
    app: Arc<Mutex<DaemonApp>>,
    provider_runtime_lanes: ProviderRunOperationLanes,
    session_capacity: usize,
    transport_health: TransportHealthStore,
) -> CommandRouter {
    let focus_projection = FocusedAgentProjection::default();
    let RouterProjectionStores {
        history_store,
        operational_history_store,
        session_projection,
        provider_catalog_projection,
        provider_run_projection,
        provider_process_projection,
        remote_relay_inventory_projection,
        agent_runtime_projection,
        config_projection,
        session_store,
        agent_store,
        attachment_store,
        provider_store,
        provider_process_tracking,
        external_provider_sessions,
        attached_provider_transcript_cursors,
        slice_store,
        active_turns,
        prompt_activity,
        prompt_workspace_claims,
        structured_output_records,
        durable_state_store,
        relay_state,
        terminal_health,
        terminal_stream,
        workflow_design_events,
        metaagent_events,
        metaagent_trace_subscriptions,
        workspace_coordinator,
        prompt_state_owner,
        prompt_id_allocator,
    } = router_projection_stores(&app);
    let runtime_state = KernelRuntimeState::new_with_owned_state_and_lanes(
        Arc::clone(&app),
        provider_runtime_lanes.clone(),
        config_projection.clone(),
        session_store.clone(),
        agent_store.clone(),
        attachment_store.clone(),
        provider_store.clone(),
        provider_process_tracking.clone(),
        external_provider_sessions.clone(),
        attached_provider_transcript_cursors.clone(),
        slice_store.clone(),
        session_projection.clone(),
        provider_run_projection.clone(),
        history_store.clone(),
        operational_history_store.clone(),
        durable_state_store.clone(),
        prompt_state_owner.clone(),
        active_turns.clone(),
        prompt_activity.clone(),
        prompt_workspace_claims.clone(),
        structured_output_records.clone(),
        terminal_stream.clone(),
        workflow_design_events.clone(),
        metaagent_events.clone(),
        metaagent_trace_subscriptions.clone(),
        workspace_coordinator.clone(),
    );
    install_provider_native_interaction_bridge(runtime_state.clone(), &provider_store);
    let provider_launch_pending = ProviderLaunchPendingTracker::default();
    let capability_runtime = CapabilityRuntimeStore::new(runtime_state.clone());
    let agent_runtime = AgentRuntime::new(
        runtime_state.clone(),
        provider_runtime_lanes.clone(),
        focus_projection.clone(),
        session_projection.clone(),
        agent_runtime_projection.clone(),
        prompt_state_owner.clone(),
        prompt_id_allocator.clone(),
    );
    let session_runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        runtime_state.clone(),
        session_capacity,
        focus_projection.clone(),
        session_projection.clone(),
        agent_runtime_projection.clone(),
        terminal_stream.clone(),
    );
    let workflow_runtime = WorkflowRuntime::new(
        runtime_state.clone(),
        session_projection.clone(),
        agent_runtime_projection.clone(),
    );
    let terminal_output_executor = TerminalOutputExecutor::new(
        runtime_state.clone(),
        provider_runtime_lanes.clone(),
        session_projection.clone(),
        agent_runtime_projection.clone(),
        provider_run_projection.clone(),
        terminal_stream.clone(),
    );
    CommandRouter {
        app,
        runtime_state,
        agent_runtime,
        session_runtime,
        workflow_runtime,
        provider_runtime_lanes,
        focus_projection,
        session_projection,
        agent_runtime_projection,
        history_store,
        operational_history_store,
        provider_catalog_projection,
        provider_run_projection,
        provider_process_projection,
        active_turns,
        remote_relay_inventory_projection,
        config_projection,
        relay_state,
        capability_health: CapabilityExecutorHealthStore::default(),
        capability_runtime,
        transport_health,
        terminal_health,
        terminal_output_executor,
        workspace_coordinator,
        provider_launch_pending,
    }
}
