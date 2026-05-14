use serde::{Deserialize, Serialize};

use super::ProjectionMetadata;
use crate::runtime::capability_executor::CapabilityExecutorHealthSnapshot;
use crate::runtime::workspace_coordinator::WorkspaceOperationClaimSnapshot;
use crate::terminal::TerminalStreamHealthSnapshot;

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
    pub transport: super::TransportHealthSnapshot,
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
        transport: super::TransportHealthSnapshot,
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
