use crate::session::unix_epoch_ms;
use serde::{Deserialize, Serialize};

mod agent_runtime_projection;
mod config_projection;
mod daemon_health_model;
mod provider_projection;
mod remote_relay_inventory_projection;
mod session_history_projection;
mod session_snapshot_projection;
mod session_state_projection;
#[cfg(test)]
mod test_support;
mod transport_health;

pub(crate) use agent_runtime_projection::{AgentRuntimeProjection, AgentRuntimeProjectionStore};
pub(crate) use config_projection::DaemonConfigProjectionStore;
pub use daemon_health_model::{
    ActorQueueSnapshot, AgentRuntimeProjectionHealthSnapshot, DaemonHealthProjection,
    ProjectionInvariantHealthSnapshot, ProjectionInvariantMismatch, ProviderCatalogHealthSnapshot,
    ProviderRunActorHealthSnapshot, ProviderRunAgentBindingConflict, ProviderRunHealthSnapshot,
    ProviderRunIdentityIssue, ProviderRunSessionPointerIssue, SessionProjectionHealthSnapshot,
    WorkspaceCoordinationHealthSnapshot, WorkspaceLiveSyncHealthSnapshot, WorktreeClaimSnapshot,
};
pub(crate) use provider_projection::{
    ProviderCatalogProjectionStore, ProviderProcessProjectionStore, ProviderRunProjectionStore,
};
pub(crate) use remote_relay_inventory_projection::RemoteRelayInventoryProjectionStore;
pub(crate) use session_history_projection::{page_history_entries, SessionHistoryProjectionStore};
pub(crate) use session_snapshot_projection::agent_activity_for_session_projection;
pub use session_snapshot_projection::{
    AgentActiveTurnProjection, AgentPromptRuntimeStatus, AgentRuntimeActivity, AgentRuntimeStatus,
    AgentTurnRuntimePhase, SessionSnapshotProjection,
};
pub(crate) use session_state_projection::SessionStateProjectionStore;
pub(crate) use transport_health::{TransportHealthSnapshot, TransportHealthStore};

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
