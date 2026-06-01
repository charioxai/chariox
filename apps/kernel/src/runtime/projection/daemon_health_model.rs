use serde::{Deserialize, Serialize};

use super::ProjectionMetadata;
use crate::agent::{AgentInstance, AgentState};
use crate::extension::{ExtensionKind, RemoteExtensionManifestSyncState};
use crate::runtime::capability_executor::CapabilityExecutorHealthSnapshot;
use crate::runtime::process_health::KernelProcessHealthSnapshot;
use crate::runtime::workspace_coordinator::WorkspaceOperationClaimSnapshot;
use crate::slice::{SliceOperationStatus, SliceRecord, SliceStatus};
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

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProviderRunHealthSnapshot {
    pub projected_runs: usize,
    pub active_runs: usize,
    pub arroba_active_runs: usize,
    pub native_tui_active_runs: usize,
    pub duplicate_arroba_agent_bindings: Vec<ProviderRunAgentBindingConflict>,
    pub multi_interface_agent_bindings: Vec<ProviderRunAgentBindingConflict>,
    pub orphaned_active_runs: Vec<ProviderRunIdentityIssue>,
    pub session_active_run_mismatches: Vec<ProviderRunSessionPointerIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRunAgentBindingConflict {
    pub session_id: String,
    pub agent_id: String,
    pub provider_run_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRunIdentityIssue {
    pub provider_run_id: String,
    pub session_id: String,
    pub agent_id: Option<String>,
    pub details: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRunSessionPointerIssue {
    pub session_id: String,
    pub active_provider_run_id: Option<String>,
    pub details: String,
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
pub struct WorkspaceLiveSyncHealthSnapshot {
    pub active_reservations: usize,
    pub active_reservation_artifacts: usize,
    pub managed_mode: WorkspaceLiveSyncManagedModeHealthSnapshot,
    pub workspace_identity:
        crate::runtime::workspace_identity_monitor::WorkspaceIdentityMonitorHealthSnapshot,
    pub external_changes: crate::io::ArtifactExternalChangeHealthSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceLiveSyncManagedModeHealthSnapshot {
    pub write_fence_supported: bool,
    pub write_fence_backend: Option<String>,
    pub unavailable_reason: Option<String>,
}

impl WorkspaceLiveSyncManagedModeHealthSnapshot {
    pub fn current() -> Self {
        Self {
            write_fence_supported: crate::provider::workspace_write_fence_supported(),
            write_fence_backend: crate::provider::workspace_write_fence_backend()
                .map(ToString::to_string),
            unavailable_reason: crate::provider::workspace_write_fence_unavailable_reason()
                .map(ToString::to_string),
        }
    }
}

impl Default for WorkspaceLiveSyncHealthSnapshot {
    fn default() -> Self {
        Self {
            active_reservations: 0,
            active_reservation_artifacts: 0,
            managed_mode: WorkspaceLiveSyncManagedModeHealthSnapshot::current(),
            workspace_identity:
                crate::runtime::workspace_identity_monitor::WorkspaceIdentityMonitorHealthSnapshot {
                    tracked_provider_runs: 0,
                    identity_changed_provider_runs: 0,
                    invalid_provider_runs: 0,
                    current_generation_total: 0,
                    issues: Vec::new(),
                },
            external_changes: crate::io::ArtifactExternalChangeHealthSnapshot {
                tracked_artifacts: 0,
                externally_changed_artifacts: 0,
                external_change_events: 0,
                live_watcher_started: false,
                live_watcher_scans: 0,
                live_watcher_scan_errors: 0,
                issues: Vec::new(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SliceLifecycleHealthSnapshot {
    pub total_slices: usize,
    pub running_slices: usize,
    pub starting_slices: usize,
    pub stopping_slices: usize,
    pub stopped_slices: usize,
    pub unhealthy_slices: usize,
    pub attached_agents: usize,
    pub failed_operations: usize,
    pub in_progress_operations: usize,
    pub issues: Vec<SliceLifecycleIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceLifecycleIssue {
    pub slice_id: String,
    pub name: String,
    pub status: String,
    pub last_operation: Option<String>,
    pub last_operation_status: Option<String>,
    pub last_error: Option<String>,
    pub session_ids: Vec<String>,
    pub agent_ids: Vec<String>,
    pub worktree_id: Option<String>,
}

impl SliceLifecycleHealthSnapshot {
    pub(crate) fn from_slices(slices: &[SliceRecord]) -> Self {
        let mut snapshot = Self {
            total_slices: slices.len(),
            ..Self::default()
        };
        for slice in slices {
            match slice.status {
                SliceStatus::Running => snapshot.running_slices += 1,
                SliceStatus::Starting => snapshot.starting_slices += 1,
                SliceStatus::Stopping => snapshot.stopping_slices += 1,
                SliceStatus::Stopped => snapshot.stopped_slices += 1,
                SliceStatus::Unhealthy => snapshot.unhealthy_slices += 1,
            }
            snapshot.attached_agents += slice.agent_ids.len();
            match slice.last_operation_status {
                Some(SliceOperationStatus::Failed) => snapshot.failed_operations += 1,
                Some(SliceOperationStatus::InProgress) => snapshot.in_progress_operations += 1,
                _ => {}
            }
            if slice.status == SliceStatus::Unhealthy
                || slice.last_operation_status == Some(SliceOperationStatus::Failed)
            {
                snapshot.issues.push(SliceLifecycleIssue {
                    slice_id: slice.id.clone(),
                    name: slice.name.clone(),
                    status: slice_status_key(&slice.status).to_string(),
                    last_operation: slice.last_operation.clone(),
                    last_operation_status: slice
                        .last_operation_status
                        .as_ref()
                        .map(slice_operation_status_key)
                        .map(ToString::to_string),
                    last_error: slice.last_error.clone(),
                    session_ids: slice.session_ids.clone(),
                    agent_ids: slice.agent_ids.clone(),
                    worktree_id: slice.worktree_id.clone(),
                });
            }
        }
        snapshot
    }
}

fn slice_status_key(status: &SliceStatus) -> &'static str {
    match status {
        SliceStatus::Stopped => "stopped",
        SliceStatus::Starting => "starting",
        SliceStatus::Stopping => "stopping",
        SliceStatus::Running => "running",
        SliceStatus::Unhealthy => "unhealthy",
    }
}

fn slice_operation_status_key(status: &SliceOperationStatus) -> &'static str {
    match status {
        SliceOperationStatus::Accepted => "accepted",
        SliceOperationStatus::InProgress => "in_progress",
        SliceOperationStatus::Completed => "completed",
        SliceOperationStatus::Failed => "failed",
        SliceOperationStatus::Reconciled => "reconciled",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RemoteExtensionSyncHealthSnapshot {
    pub remote_agents: usize,
    pub home_proxy_agents: usize,
    pub home_proxy_grants: usize,
    pub manifest_missing_agents: usize,
    pub synced_agents: usize,
    pub syncing_agents: usize,
    pub pending_agents: usize,
    pub failed_agents: usize,
    pub stale_agents: usize,
    pub pending_revoke_agents: usize,
    pub issues: Vec<RemoteExtensionSyncIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteExtensionSyncIssue {
    pub session_id: String,
    pub agent_id: String,
    pub agent_ref: String,
    pub worker_kernel_id: String,
    pub worker_machine_id: String,
    pub execution_lease_id: String,
    pub leased_agent_id: String,
    pub active_worker_provider_run_id: Option<String>,
    pub state: String,
    pub manifest_hash: Option<String>,
    pub last_error: Option<String>,
    pub pending_revoke: bool,
    pub home_proxy_grants: Vec<String>,
    pub worktree_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RemoteExecutionHealthSnapshot {
    pub remote_agents: usize,
    pub active_remote_agents: usize,
    pub missing_active_worker_runs: usize,
    pub malformed_bindings: usize,
    pub issues: Vec<RemoteExecutionIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteExecutionIssue {
    pub kind: String,
    pub session_id: String,
    pub agent_id: String,
    pub agent_ref: String,
    pub worker_kernel_id: String,
    pub worker_machine_id: String,
    pub execution_lease_id: String,
    pub leased_agent_id: String,
    pub active_worker_provider_run_id: Option<String>,
    pub state: String,
    pub is_processing: bool,
    pub worktree_id: Option<String>,
    pub details: String,
}

impl RemoteExecutionHealthSnapshot {
    pub(crate) fn from_agents(agents: &[AgentInstance]) -> Self {
        let mut snapshot = Self::default();
        for agent in agents {
            let Some(remote_execution) = agent.remote_execution() else {
                continue;
            };
            snapshot.remote_agents += 1;

            let active = agent.is_processing() || agent.state() == AgentState::Working;
            if active {
                snapshot.active_remote_agents += 1;
            }

            let mut malformed_fields = Vec::new();
            if remote_execution.worker_kernel_id.is_empty() {
                malformed_fields.push("worker_kernel_id");
            }
            if remote_execution.worker_machine_id.is_empty() {
                malformed_fields.push("worker_machine_id");
            }
            if remote_execution.execution_lease_id.is_empty() {
                malformed_fields.push("execution_lease_id");
            }
            if remote_execution.leased_agent_id.is_empty() {
                malformed_fields.push("leased_agent_id");
            }
            if !malformed_fields.is_empty() {
                snapshot.malformed_bindings += 1;
                snapshot.issues.push(remote_execution_issue(
                    agent,
                    remote_execution,
                    "malformed_binding",
                    format!(
                        "remote execution binding is missing {}",
                        malformed_fields.join(", ")
                    ),
                ));
            }

            if active && remote_execution.active_worker_provider_run_id.is_none() {
                snapshot.missing_active_worker_runs += 1;
                snapshot.issues.push(remote_execution_issue(
                    agent,
                    remote_execution,
                    "missing_active_worker_provider_run",
                    "active remote agent has no active worker provider run id".to_string(),
                ));
            }
        }
        snapshot
    }
}

fn remote_execution_issue(
    agent: &AgentInstance,
    remote_execution: &crate::agent::RemoteAgentBinding,
    kind: &str,
    details: String,
) -> RemoteExecutionIssue {
    RemoteExecutionIssue {
        kind: kind.to_string(),
        session_id: agent.session_id().to_string(),
        agent_id: agent.id().to_string(),
        agent_ref: agent.agent_ref().to_string(),
        worker_kernel_id: remote_execution.worker_kernel_id.clone(),
        worker_machine_id: remote_execution.worker_machine_id.clone(),
        execution_lease_id: remote_execution.execution_lease_id.clone(),
        leased_agent_id: remote_execution.leased_agent_id.clone(),
        active_worker_provider_run_id: remote_execution.active_worker_provider_run_id.clone(),
        state: agent_state_key(agent.state()).to_string(),
        is_processing: agent.is_processing(),
        worktree_id: agent.worktree_id().map(ToString::to_string),
        details,
    }
}

fn agent_state_key(state: AgentState) -> &'static str {
    match state {
        AgentState::Idle => "idle",
        AgentState::Working => "working",
        AgentState::Focused => "focused",
        AgentState::Error => "error",
    }
}

impl RemoteExtensionSyncHealthSnapshot {
    pub(crate) fn from_agents(agents: &[AgentInstance]) -> Self {
        let mut snapshot = Self::default();
        for agent in agents {
            let Some(remote_execution) = agent.remote_execution() else {
                continue;
            };
            snapshot.remote_agents += 1;
            let home_proxy_grants = agent
                .extension_grants()
                .iter()
                .filter(|grant| grant.kind != ExtensionKind::Skill)
                .map(|grant| format!("{}:{}", grant.kind.as_str(), grant.name))
                .collect::<Vec<_>>();
            let home_proxy_grant_count = home_proxy_grants.len();
            if home_proxy_grant_count == 0 {
                continue;
            }
            snapshot.home_proxy_agents += 1;
            snapshot.home_proxy_grants += home_proxy_grant_count;
            let Some(status) = agent.remote_extension_manifest_sync() else {
                snapshot.manifest_missing_agents += 1;
                snapshot.issues.push(remote_extension_sync_issue(
                    agent,
                    remote_execution,
                    "missing",
                    None,
                    None,
                    false,
                    home_proxy_grants,
                ));
                continue;
            };
            match status.state {
                RemoteExtensionManifestSyncState::Synced => snapshot.synced_agents += 1,
                RemoteExtensionManifestSyncState::Syncing => snapshot.syncing_agents += 1,
                RemoteExtensionManifestSyncState::Pending => snapshot.pending_agents += 1,
                RemoteExtensionManifestSyncState::Failed => snapshot.failed_agents += 1,
                RemoteExtensionManifestSyncState::Stale => snapshot.stale_agents += 1,
            }
            let pending_revoke = status.pending_revoke.unwrap_or(false);
            if pending_revoke {
                snapshot.pending_revoke_agents += 1;
            }
            if matches!(
                status.state,
                RemoteExtensionManifestSyncState::Failed | RemoteExtensionManifestSyncState::Stale
            ) || pending_revoke
            {
                snapshot.issues.push(remote_extension_sync_issue(
                    agent,
                    remote_execution,
                    remote_extension_sync_state_key(status.state),
                    status.manifest_hash.clone(),
                    status.last_error.clone(),
                    pending_revoke,
                    home_proxy_grants,
                ));
            }
        }
        snapshot
    }
}

fn remote_extension_sync_issue(
    agent: &AgentInstance,
    remote_execution: &crate::agent::RemoteAgentBinding,
    state: &str,
    manifest_hash: Option<String>,
    last_error: Option<String>,
    pending_revoke: bool,
    home_proxy_grants: Vec<String>,
) -> RemoteExtensionSyncIssue {
    RemoteExtensionSyncIssue {
        session_id: agent.session_id().to_string(),
        agent_id: agent.id().to_string(),
        agent_ref: agent.agent_ref().to_string(),
        worker_kernel_id: remote_execution.worker_kernel_id.clone(),
        worker_machine_id: remote_execution.worker_machine_id.clone(),
        execution_lease_id: remote_execution.execution_lease_id.clone(),
        leased_agent_id: remote_execution.leased_agent_id.clone(),
        active_worker_provider_run_id: remote_execution.active_worker_provider_run_id.clone(),
        state: state.to_string(),
        manifest_hash,
        last_error,
        pending_revoke,
        home_proxy_grants,
        worktree_id: agent.worktree_id().map(ToString::to_string),
    }
}

fn remote_extension_sync_state_key(state: RemoteExtensionManifestSyncState) -> &'static str {
    match state {
        RemoteExtensionManifestSyncState::Synced => "synced",
        RemoteExtensionManifestSyncState::Syncing => "syncing",
        RemoteExtensionManifestSyncState::Pending => "pending",
        RemoteExtensionManifestSyncState::Failed => "failed",
        RemoteExtensionManifestSyncState::Stale => "stale",
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
    pub process: KernelProcessHealthSnapshot,
    pub capability_executor: CapabilityExecutorHealthSnapshot,
    pub session_projection: SessionProjectionHealthSnapshot,
    pub agent_runtime_projection: AgentRuntimeProjectionHealthSnapshot,
    pub provider_catalog: ProviderCatalogHealthSnapshot,
    pub provider_runs: ProviderRunHealthSnapshot,
    pub transport: super::TransportHealthSnapshot,
    pub terminal_stream: TerminalStreamHealthSnapshot,
    pub slice_lifecycle: SliceLifecycleHealthSnapshot,
    pub remote_execution: RemoteExecutionHealthSnapshot,
    pub remote_extension_sync: RemoteExtensionSyncHealthSnapshot,
    pub workspace_coordination: WorkspaceCoordinationHealthSnapshot,
    pub workspace_live_sync: WorkspaceLiveSyncHealthSnapshot,
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
        process: KernelProcessHealthSnapshot,
        capability_executor: CapabilityExecutorHealthSnapshot,
        mut session_projection: SessionProjectionHealthSnapshot,
        agent_runtime_projection: AgentRuntimeProjectionHealthSnapshot,
        provider_catalog: ProviderCatalogHealthSnapshot,
        provider_runs: ProviderRunHealthSnapshot,
        transport: super::TransportHealthSnapshot,
        terminal_stream: TerminalStreamHealthSnapshot,
        slice_lifecycle: SliceLifecycleHealthSnapshot,
        remote_execution: RemoteExecutionHealthSnapshot,
        remote_extension_sync: RemoteExtensionSyncHealthSnapshot,
        workspace_coordination: WorkspaceCoordinationHealthSnapshot,
        workspace_live_sync: WorkspaceLiveSyncHealthSnapshot,
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
            process,
            capability_executor,
            session_projection,
            agent_runtime_projection,
            provider_catalog,
            provider_runs,
            transport,
            terminal_stream,
            slice_lifecycle,
            remote_execution,
            remote_extension_sync,
            workspace_coordination,
            workspace_live_sync,
            projection_invariants,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ActorQueueSnapshot, AgentRuntimeProjectionHealthSnapshot, DaemonHealthProjection,
        ProjectionInvariantHealthSnapshot, ProviderCatalogHealthSnapshot,
        ProviderRunActorHealthSnapshot, ProviderRunAgentBindingConflict, ProviderRunHealthSnapshot,
        ProviderRunIdentityIssue, ProviderRunSessionPointerIssue, RemoteExecutionHealthSnapshot,
        RemoteExtensionSyncHealthSnapshot, RemoteExtensionSyncIssue,
        SessionProjectionHealthSnapshot, SliceLifecycleHealthSnapshot, SliceLifecycleIssue,
        WorkspaceCoordinationHealthSnapshot, WorkspaceLiveSyncHealthSnapshot,
        WorkspaceLiveSyncManagedModeHealthSnapshot, WorktreeClaimSnapshot,
    };
    use crate::agent::{AgentInstance, AgentState, GridPosition, RemoteAgentBinding};
    use crate::extension::{
        ExtensionGrant, ExtensionKind, RemoteExtensionManifestSyncState,
        RemoteExtensionManifestSyncStatus,
    };
    use crate::runtime::capability_executor::CapabilityExecutorHealthSnapshot;
    use crate::runtime::process_health::KernelProcessHealthSnapshot;
    use crate::runtime::projection::TransportHealthSnapshot;
    use crate::slice::SliceRecord;
    use crate::terminal::TerminalStreamHealthSnapshot;

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
            KernelProcessHealthSnapshot {
                process_id: 42,
                current_resident_set_bytes: Some(64 * 1024 * 1024),
                peak_resident_set_bytes: Some(128 * 1024 * 1024),
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
            ProviderRunHealthSnapshot {
                projected_runs: 4,
                active_runs: 3,
                arroba_active_runs: 2,
                native_tui_active_runs: 1,
                duplicate_arroba_agent_bindings: vec![ProviderRunAgentBindingConflict {
                    session_id: "session-1".to_string(),
                    agent_id: "agent-1".to_string(),
                    provider_run_ids: vec![
                        "provider-run-1".to_string(),
                        "provider-run-2".to_string(),
                    ],
                }],
                multi_interface_agent_bindings: vec![ProviderRunAgentBindingConflict {
                    session_id: "session-2".to_string(),
                    agent_id: "agent-2".to_string(),
                    provider_run_ids: vec![
                        "provider-run-3:arroba".to_string(),
                        "provider-run-4:native_tui".to_string(),
                    ],
                }],
                orphaned_active_runs: vec![ProviderRunIdentityIssue {
                    provider_run_id: "provider-run-orphan".to_string(),
                    session_id: "missing-session".to_string(),
                    agent_id: None,
                    details: "provider run points at a missing session".to_string(),
                }],
                session_active_run_mismatches: vec![ProviderRunSessionPointerIssue {
                    session_id: "session-1".to_string(),
                    active_provider_run_id: Some("provider-run-missing".to_string()),
                    details: "active provider run is not projected".to_string(),
                }],
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
            SliceLifecycleHealthSnapshot {
                total_slices: 5,
                running_slices: 1,
                starting_slices: 1,
                stopping_slices: 1,
                stopped_slices: 1,
                unhealthy_slices: 1,
                attached_agents: 3,
                failed_operations: 1,
                in_progress_operations: 2,
                issues: vec![SliceLifecycleIssue {
                    slice_id: "slice-1".to_string(),
                    name: "dev".to_string(),
                    status: "unhealthy".to_string(),
                    last_operation: Some("start".to_string()),
                    last_operation_status: Some("failed".to_string()),
                    last_error: Some("worker kernel discovery timed out".to_string()),
                    session_ids: vec!["session-1".to_string()],
                    agent_ids: vec!["agent-1".to_string(), "agent-2".to_string()],
                    worktree_id: Some("/repo".to_string()),
                }],
            },
            RemoteExecutionHealthSnapshot {
                remote_agents: 4,
                active_remote_agents: 1,
                missing_active_worker_runs: 1,
                malformed_bindings: 0,
                issues: vec![super::RemoteExecutionIssue {
                    kind: "missing_active_worker_provider_run".to_string(),
                    session_id: "session-1".to_string(),
                    agent_id: "agent-remote".to_string(),
                    agent_ref: "agent-remote".to_string(),
                    worker_kernel_id: "worker-kernel".to_string(),
                    worker_machine_id: "worker-machine".to_string(),
                    execution_lease_id: "lease-1".to_string(),
                    leased_agent_id: "leased-agent-1".to_string(),
                    active_worker_provider_run_id: None,
                    state: "working".to_string(),
                    is_processing: true,
                    worktree_id: Some("/repo".to_string()),
                    details: "active remote agent has no active worker provider run id".to_string(),
                }],
            },
            RemoteExtensionSyncHealthSnapshot {
                remote_agents: 4,
                home_proxy_agents: 3,
                home_proxy_grants: 5,
                manifest_missing_agents: 1,
                synced_agents: 1,
                syncing_agents: 0,
                pending_agents: 0,
                failed_agents: 1,
                stale_agents: 0,
                pending_revoke_agents: 1,
                issues: vec![RemoteExtensionSyncIssue {
                    session_id: "session-1".to_string(),
                    agent_id: "agent-failed".to_string(),
                    agent_ref: "agent-failed".to_string(),
                    worker_kernel_id: "worker-kernel".to_string(),
                    worker_machine_id: "worker-machine".to_string(),
                    execution_lease_id: "lease-1".to_string(),
                    leased_agent_id: "leased-agent-1".to_string(),
                    active_worker_provider_run_id: Some("worker-run-1".to_string()),
                    state: "failed".to_string(),
                    manifest_hash: Some("hash-failed".to_string()),
                    last_error: Some("relay offline".to_string()),
                    pending_revoke: true,
                    home_proxy_grants: vec!["connector:status-api".to_string()],
                    worktree_id: Some("/repo".to_string()),
                }],
            },
            WorkspaceCoordinationHealthSnapshot {
                active_worktree_claims: vec![WorktreeClaimSnapshot {
                    workspace_id: "workspace-1".to_string(),
                    worktree_id: "worktree-1".to_string(),
                    session_ids: vec!["session-1".to_string(), "session-2".to_string()],
                }],
                worktree_collisions: vec![WorktreeClaimSnapshot {
                    workspace_id: "workspace-1".to_string(),
                    worktree_id: "worktree-1".to_string(),
                    session_ids: vec!["session-1".to_string(), "session-2".to_string()],
                }],
                active_operation_claims: Vec::new(),
            },
            WorkspaceLiveSyncHealthSnapshot {
                active_reservations: 2,
                active_reservation_artifacts: 1,
                managed_mode: WorkspaceLiveSyncManagedModeHealthSnapshot {
                    write_fence_supported: true,
                    write_fence_backend: Some("macos-seatbelt".to_string()),
                    unavailable_reason: None,
                },
                workspace_identity:
                    crate::runtime::workspace_identity_monitor::WorkspaceIdentityMonitorHealthSnapshot {
                        tracked_provider_runs: 3,
                        identity_changed_provider_runs: 1,
                        invalid_provider_runs: 1,
                        current_generation_total: 2,
                        issues: vec![
                            crate::runtime::workspace_identity_monitor::WorkspaceIdentityIssue {
                                provider_run_id: "provider-run-identity".to_string(),
                                root: "/repo".to_string(),
                                generation: 2,
                                valid: false,
                                baseline_fingerprint: "root-a".to_string(),
                                current_fingerprint: "root-b".to_string(),
                                baseline_branch: Some("main".to_string()),
                                current_branch: Some("feature".to_string()),
                                baseline_head_commit: Some("abc123".to_string()),
                                current_head_commit: Some("def456".to_string()),
                                baseline_repo_url: Some("git@example.com:repo.git".to_string()),
                                current_repo_url: Some("git@example.com:repo.git".to_string()),
                            },
                        ],
                    },
                external_changes: crate::io::ArtifactExternalChangeHealthSnapshot {
                    tracked_artifacts: 4,
                    externally_changed_artifacts: 2,
                    external_change_events: 5,
                    live_watcher_started: true,
                    live_watcher_scans: 7,
                    live_watcher_scan_errors: 0,
                    issues: Vec::new(),
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
        assert_eq!(projection.process.process_id, 42);
        assert_eq!(
            projection.process.current_resident_set_bytes,
            Some(64 * 1024 * 1024)
        );
        assert_eq!(
            projection.process.peak_resident_set_bytes,
            Some(128 * 1024 * 1024)
        );
        assert_eq!(projection.capability_executor.submitted_jobs, 8);
        assert_eq!(projection.capability_executor.running_jobs, 1);
        assert_eq!(projection.session_projection.active_prompts, 1);
        assert_eq!(projection.session_projection.queued_prompts, 2);
        assert_eq!(projection.agent_runtime_projection.projected_agents, 3);
        assert_eq!(projection.agent_runtime_projection.active_prompts, 1);
        assert!(projection.provider_catalog.cached);
        assert_eq!(projection.provider_runs.projected_runs, 4);
        assert_eq!(projection.provider_runs.active_runs, 3);
        assert_eq!(
            projection
                .provider_runs
                .duplicate_arroba_agent_bindings
                .len(),
            1
        );
        assert_eq!(
            projection
                .provider_runs
                .multi_interface_agent_bindings
                .len(),
            1
        );
        assert_eq!(projection.provider_runs.orphaned_active_runs.len(), 1);
        assert_eq!(
            projection.provider_runs.session_active_run_mismatches.len(),
            1
        );
        assert_eq!(projection.transport.active_connections, 2);
        assert_eq!(projection.transport.slow_consumer_closes, 1);
        assert_eq!(projection.terminal_stream.pending_output_records, 4);
        assert_eq!(
            projection.terminal_stream.trimmed_pending_output_recipients,
            1
        );
        assert_eq!(projection.slice_lifecycle.total_slices, 5);
        assert_eq!(projection.slice_lifecycle.running_slices, 1);
        assert_eq!(projection.slice_lifecycle.unhealthy_slices, 1);
        assert_eq!(projection.slice_lifecycle.attached_agents, 3);
        assert_eq!(projection.slice_lifecycle.failed_operations, 1);
        assert_eq!(projection.slice_lifecycle.in_progress_operations, 2);
        assert_eq!(projection.slice_lifecycle.issues.len(), 1);
        assert_eq!(
            projection.slice_lifecycle.issues[0].last_error.as_deref(),
            Some("worker kernel discovery timed out")
        );
        assert_eq!(projection.remote_extension_sync.remote_agents, 4);
        assert_eq!(projection.remote_execution.remote_agents, 4);
        assert_eq!(projection.remote_execution.active_remote_agents, 1);
        assert_eq!(projection.remote_execution.missing_active_worker_runs, 1);
        assert_eq!(
            projection.remote_execution.issues[0].kind,
            "missing_active_worker_provider_run"
        );
        assert_eq!(projection.remote_extension_sync.home_proxy_agents, 3);
        assert_eq!(projection.remote_extension_sync.home_proxy_grants, 5);
        assert_eq!(projection.remote_extension_sync.manifest_missing_agents, 1);
        assert_eq!(projection.remote_extension_sync.failed_agents, 1);
        assert_eq!(projection.remote_extension_sync.pending_revoke_agents, 1);
        assert_eq!(projection.remote_extension_sync.issues.len(), 1);
        assert_eq!(
            projection.remote_extension_sync.issues[0].agent_ref,
            "agent-failed"
        );
        assert_eq!(
            projection.workspace_coordination.worktree_collisions.len(),
            1
        );
        assert_eq!(projection.workspace_live_sync.active_reservations, 2);
        assert_eq!(
            projection.workspace_live_sync.active_reservation_artifacts,
            1
        );
        assert!(
            projection
                .workspace_live_sync
                .managed_mode
                .write_fence_supported
        );
        assert_eq!(
            projection
                .workspace_live_sync
                .managed_mode
                .write_fence_backend
                .as_deref(),
            Some("macos-seatbelt")
        );
        assert_eq!(
            projection
                .workspace_live_sync
                .workspace_identity
                .invalid_provider_runs,
            1
        );
        assert_eq!(
            projection
                .workspace_live_sync
                .workspace_identity
                .issues
                .len(),
            1
        );
        assert_eq!(
            projection
                .workspace_live_sync
                .external_changes
                .tracked_artifacts,
            4
        );
        assert_eq!(
            projection
                .workspace_live_sync
                .external_changes
                .external_change_events,
            5
        );
        assert!(projection
            .workspace_live_sync
            .external_changes
            .issues
            .is_empty());
        assert!(
            projection
                .workspace_live_sync
                .external_changes
                .live_watcher_started
        );
        assert_eq!(projection.projection_invariants.checked_agents, 3);
        assert!(projection.projection_invariants.mismatches.is_empty());
    }

    #[test]
    fn slice_lifecycle_health_identifies_unhealthy_and_failed_slices() {
        let snapshot = SliceLifecycleHealthSnapshot::from_slices(&[
            slice_record(
                "slice-ok",
                "dev-ok",
                crate::slice::SliceStatus::Running,
                None,
                None,
            ),
            slice_record(
                "slice-bad",
                "dev-bad",
                crate::slice::SliceStatus::Unhealthy,
                Some(crate::slice::SliceOperationStatus::Failed),
                Some("worker kernel discovery timed out"),
            ),
        ]);

        assert_eq!(snapshot.total_slices, 2);
        assert_eq!(snapshot.running_slices, 1);
        assert_eq!(snapshot.unhealthy_slices, 1);
        assert_eq!(snapshot.failed_operations, 1);
        assert_eq!(snapshot.issues.len(), 1);
        let issue = &snapshot.issues[0];
        assert_eq!(issue.slice_id, "slice-bad");
        assert_eq!(issue.name, "dev-bad");
        assert_eq!(issue.status, "unhealthy");
        assert_eq!(issue.last_operation.as_deref(), Some("start"));
        assert_eq!(issue.last_operation_status.as_deref(), Some("failed"));
        assert_eq!(
            issue.last_error.as_deref(),
            Some("worker kernel discovery timed out")
        );
        assert_eq!(issue.session_ids, vec!["session-1"]);
        assert_eq!(issue.agent_ids, vec!["agent-1"]);
        assert_eq!(issue.worktree_id.as_deref(), Some("/repo"));
    }

    #[test]
    fn remote_extension_sync_health_counts_only_home_proxy_grants() {
        let mut synced = remote_agent("agent-synced");
        synced.grant_extension(ExtensionGrant::new(ExtensionKind::Mcp, "filesystem"));
        synced.grant_extension(ExtensionGrant::new(ExtensionKind::Skill, "review"));
        synced.set_remote_extension_manifest_sync(Some(RemoteExtensionManifestSyncStatus::synced(
            "hash-synced".to_string(),
        )));

        let mut failed = remote_agent("agent-failed");
        failed.grant_extension(ExtensionGrant::new(ExtensionKind::Connector, "status-api"));
        failed.set_remote_extension_manifest_sync(Some(
            RemoteExtensionManifestSyncStatus::pending("hash-failed".to_string(), true)
                .failed("relay offline"),
        ));

        let mut stale = remote_agent("agent-stale");
        stale.grant_extension(ExtensionGrant::new(ExtensionKind::Script, "release"));
        stale.set_remote_extension_manifest_sync(Some(RemoteExtensionManifestSyncStatus {
            state: RemoteExtensionManifestSyncState::Stale,
            manifest_hash: Some("hash-stale".to_string()),
            last_attempted_at_ms: None,
            last_synced_at_ms: None,
            last_error: Some("worker behind".to_string()),
            pending_revoke: None,
        }));

        let mut missing = remote_agent("agent-missing");
        missing.grant_extension(ExtensionGrant::new(ExtensionKind::Mcp, "github"));

        let mut skill_only = remote_agent("agent-skill");
        skill_only.grant_extension(ExtensionGrant::new(ExtensionKind::Skill, "docs"));

        let local = local_agent("agent-local");
        let snapshot = RemoteExtensionSyncHealthSnapshot::from_agents(&[
            synced, failed, stale, missing, skill_only, local,
        ]);

        assert_eq!(snapshot.remote_agents, 5);
        assert_eq!(snapshot.home_proxy_agents, 4);
        assert_eq!(snapshot.home_proxy_grants, 4);
        assert_eq!(snapshot.synced_agents, 1);
        assert_eq!(snapshot.failed_agents, 1);
        assert_eq!(snapshot.stale_agents, 1);
        assert_eq!(snapshot.manifest_missing_agents, 1);
        assert_eq!(snapshot.pending_revoke_agents, 1);
        assert_eq!(snapshot.issues.len(), 3);
        assert_eq!(
            snapshot
                .issues
                .iter()
                .map(|issue| (issue.agent_id.as_str(), issue.state.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("agent-failed", "failed"),
                ("agent-stale", "stale"),
                ("agent-missing", "missing"),
            ]
        );
        assert_eq!(
            snapshot.issues[0].home_proxy_grants,
            vec!["connector:status-api"]
        );
        assert_eq!(snapshot.issues[0].pending_revoke, true);
        assert_eq!(
            snapshot.issues[0].active_worker_provider_run_id.as_deref(),
            Some("worker-run-1")
        );
    }

    #[test]
    fn remote_execution_health_reports_active_agent_without_worker_run() {
        let mut healthy_idle = remote_agent("agent-idle");
        healthy_idle.set_remote_execution_active_worker_provider_run_id(None);

        let mut missing_run = remote_agent("agent-working");
        missing_run.set_remote_execution_active_worker_provider_run_id(None);
        missing_run.set_state(AgentState::Working);
        missing_run.set_processing(true);

        let mut malformed = remote_agent("agent-malformed");
        malformed.set_remote_execution(Some(RemoteAgentBinding {
            worker_kernel_id: String::new(),
            worker_machine_id: "worker-machine".to_string(),
            execution_lease_id: String::new(),
            leased_agent_id: "leased-agent-1".to_string(),
            active_worker_provider_run_id: Some("worker-run-1".to_string()),
            relay_url: None,
            relay_token: None,
        }));

        let snapshot = RemoteExecutionHealthSnapshot::from_agents(&[
            healthy_idle,
            missing_run,
            malformed,
            local_agent("agent-local"),
        ]);

        assert_eq!(snapshot.remote_agents, 3);
        assert_eq!(snapshot.active_remote_agents, 1);
        assert_eq!(snapshot.missing_active_worker_runs, 1);
        assert_eq!(snapshot.malformed_bindings, 1);
        assert_eq!(
            snapshot
                .issues
                .iter()
                .map(|issue| (issue.agent_id.as_str(), issue.kind.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("agent-working", "missing_active_worker_provider_run"),
                ("agent-malformed", "malformed_binding"),
            ]
        );
        assert!(snapshot.issues[0].is_processing);
        assert_eq!(snapshot.issues[0].state, "working");
        assert_eq!(snapshot.issues[0].worktree_id.as_deref(), Some("/repo"));
        assert!(snapshot.issues[1]
            .details
            .contains("worker_kernel_id, execution_lease_id"));
    }

    fn local_agent(id: &str) -> AgentInstance {
        AgentInstance::new(
            id,
            id,
            "session-1",
            None,
            "codex",
            None,
            None,
            None,
            GridPosition::new(0, 0, 1, 1),
        )
    }

    fn remote_agent(id: &str) -> AgentInstance {
        let mut agent = local_agent(id);
        agent.set_remote_execution(Some(RemoteAgentBinding {
            worker_kernel_id: "worker-kernel".to_string(),
            worker_machine_id: "worker-machine".to_string(),
            execution_lease_id: "lease-1".to_string(),
            leased_agent_id: "leased-agent-1".to_string(),
            active_worker_provider_run_id: Some("worker-run-1".to_string()),
            relay_url: None,
            relay_token: None,
        }));
        agent.set_worktree_id(Some("/repo".to_string()));
        agent
    }

    fn slice_record(
        id: &str,
        name: &str,
        status: crate::slice::SliceStatus,
        operation_status: Option<crate::slice::SliceOperationStatus>,
        last_error: Option<&str>,
    ) -> SliceRecord {
        SliceRecord {
            id: id.to_string(),
            name: name.to_string(),
            owner_kernel_id: "kernel-home".to_string(),
            owner_machine_id: "machine-home".to_string(),
            session_id: Some("session-1".to_string()),
            session_ids: vec!["session-1".to_string()],
            agent_ids: vec!["agent-1".to_string()],
            backend: crate::slice::SliceBackendKind::LocalDocker,
            os: "linux".to_string(),
            display_mode: crate::slice::SliceDisplayMode::Headless,
            status,
            last_operation: operation_status.as_ref().map(|_| "start".to_string()),
            last_operation_status: operation_status,
            last_error: last_error.map(ToString::to_string),
            last_operation_at_ms: Some(100),
            workspace_id: Some("/repo".to_string()),
            worktree_id: Some("/repo".to_string()),
            workspace_mount: Some("/workspace".to_string()),
            worker_kernel_ref: format!("slice:{id}"),
            worker_kernel_id: Some("kernel-slice".to_string()),
            worker_machine_id: Some("machine-slice".to_string()),
            relay_endpoint: None,
            local_docker_ports: None,
            providers: vec!["codex".to_string()],
            provider_auth: Vec::new(),
            display_endpoint: None,
            created_at_ms: 1,
            updated_at_ms: 2,
        }
    }
}
