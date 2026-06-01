use serde::{Deserialize, Serialize};

use super::ProjectionMetadata;
use crate::runtime::capability_executor::CapabilityExecutorHealthSnapshot;
use crate::runtime::process_health::KernelProcessHealthSnapshot;
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

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProviderRunHealthSnapshot {
    pub projected_runs: usize,
    pub active_runs: usize,
    pub arroba_active_runs: usize,
    pub native_tui_active_runs: usize,
    pub duplicate_arroba_agent_bindings: Vec<ProviderRunAgentBindingConflict>,
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
    pub workspace_identity:
        crate::runtime::workspace_identity_monitor::WorkspaceIdentityMonitorHealthSnapshot,
    pub external_changes: crate::io::ArtifactExternalChangeHealthSnapshot,
}

impl Default for WorkspaceLiveSyncHealthSnapshot {
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
    pub process: KernelProcessHealthSnapshot,
    pub capability_executor: CapabilityExecutorHealthSnapshot,
    pub session_projection: SessionProjectionHealthSnapshot,
    pub agent_runtime_projection: AgentRuntimeProjectionHealthSnapshot,
    pub provider_catalog: ProviderCatalogHealthSnapshot,
    pub provider_runs: ProviderRunHealthSnapshot,
    pub transport: super::TransportHealthSnapshot,
    pub terminal_stream: TerminalStreamHealthSnapshot,
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
        ProviderRunIdentityIssue, ProviderRunSessionPointerIssue, SessionProjectionHealthSnapshot,
        WorkspaceCoordinationHealthSnapshot, WorkspaceLiveSyncHealthSnapshot,
        WorktreeClaimSnapshot,
    };
    use crate::runtime::capability_executor::CapabilityExecutorHealthSnapshot;
    use crate::runtime::process_health::KernelProcessHealthSnapshot;
    use crate::runtime::projection::TransportHealthSnapshot;
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
                workspace_identity:
                    crate::runtime::workspace_identity_monitor::WorkspaceIdentityMonitorHealthSnapshot {
                        tracked_provider_runs: 3,
                        identity_changed_provider_runs: 1,
                        invalid_provider_runs: 1,
                        current_generation_total: 2,
                    },
                external_changes: crate::io::ArtifactExternalChangeHealthSnapshot {
                    tracked_artifacts: 4,
                    externally_changed_artifacts: 2,
                    external_change_events: 5,
                    live_watcher_started: true,
                    live_watcher_scans: 7,
                    live_watcher_scan_errors: 0,
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
        assert_eq!(
            projection.workspace_coordination.worktree_collisions.len(),
            1
        );
        assert_eq!(projection.workspace_live_sync.active_reservations, 2);
        assert_eq!(
            projection.workspace_live_sync.active_reservation_artifacts,
            1
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
        assert!(
            projection
                .workspace_live_sync
                .external_changes
                .live_watcher_started
        );
        assert_eq!(projection.projection_invariants.checked_agents, 3);
        assert!(projection.projection_invariants.mismatches.is_empty());
    }
}
