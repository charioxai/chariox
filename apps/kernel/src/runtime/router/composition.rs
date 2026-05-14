use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, RwLock};

use crate::app::{DaemonApp, PromptActivityStore};
use crate::history::{OperationalHistoryStore, SessionHistoryStore};
use crate::runtime::projection::{
    AgentRuntimeProjectionStore, DaemonConfigProjectionStore, ProviderCatalogProjectionStore,
    ProviderProcessProjectionStore, ProviderRunProjectionStore,
    RemoteRelayInventoryProjectionStore, SessionHistoryProjectionStore,
    SessionStateProjectionStore,
};
use crate::runtime::prompt_state::PromptStateOwner;
use crate::runtime::workspace_coordinator::WorkspaceCoordinator;
use crate::session::PromptIdAllocator;
use crate::terminal::{TerminalStreamHealthStore, TerminalStreamStore};
use crate::transport::relay_client::RelayClientState;

pub(super) struct RouterProjectionStores {
    pub(super) history_store: SessionHistoryStore,
    pub(super) operational_history_store: OperationalHistoryStore,
    pub(super) session_projection: SessionStateProjectionStore,
    pub(super) history_projection: SessionHistoryProjectionStore,
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
    pub(super) slice_store: crate::slice::SliceStore,
    pub(super) active_turns: crate::app::ActiveTurnStore,
    pub(super) prompt_activity: PromptActivityStore,
    pub(super) prompt_workspace_claims: crate::app::PromptWorkspaceClaimStore,
    pub(super) structured_output_records: crate::app::provider_output::StructuredOutputRecordStore,
    pub(super) durable_state_store: crate::durable_state::DurableKernelStateStore,
    pub(super) relay_state: Arc<RwLock<RelayClientState>>,
    pub(super) terminal_health: TerminalStreamHealthStore,
    pub(super) terminal_stream: TerminalStreamStore,
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
        history_projection: app.session_history_projection_store(),
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
        slice_store: app.slices(),
        active_turns: app.active_turn_store(),
        prompt_activity: app.prompt_activity_store(),
        prompt_workspace_claims: app.prompt_workspace_claim_store(),
        structured_output_records: app.structured_output_record_store(),
        durable_state_store: app.durable_state_store(),
        relay_state: app.relay_client_state(),
        terminal_health: app.terminal_health_store(),
        terminal_stream: app.terminal_stream_store(),
        workspace_coordinator: app.workspace_coordinator(),
        prompt_state_owner: app.prompt_state_owner(),
        prompt_id_allocator: app.prompt_id_allocator(),
    }
}
