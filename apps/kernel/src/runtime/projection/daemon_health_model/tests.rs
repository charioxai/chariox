use super::{
    ActorQueueSnapshot, AgentRuntimeProjectionHealthSnapshot, DaemonHealthProjection,
    ProjectionInvariantHealthSnapshot, ProviderCatalogHealthSnapshot,
    ProviderRunActorHealthSnapshot, ProviderRunAgentBindingConflict, ProviderRunHealthSnapshot,
    ProviderRunIdentityIssue, ProviderRunSessionPointerIssue, ProviderRunTerminalDiagnosticIssue,
    RemoteExecutionHealthSnapshot, RemoteExtensionSyncHealthSnapshot, RemoteExtensionSyncIssue,
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
use std::collections::BTreeSet;

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
            chariox_active_runs: 2,
            native_tui_active_runs: 1,
            terminal_diagnostics: vec![ProviderRunTerminalDiagnosticIssue {
                provider_run_id: "provider-run-timeout".to_string(),
                session_id: "session-1".to_string(),
                agent_id: Some("agent-1".to_string()),
                provider: "codex".to_string(),
                state: "Running".to_string(),
                diagnostic: "provider produced no terminal output within 10m".to_string(),
            }],
            duplicate_chariox_agent_bindings: vec![ProviderRunAgentBindingConflict {
                session_id: "session-1".to_string(),
                agent_id: "agent-1".to_string(),
                provider_run_ids: vec!["provider-run-1".to_string(), "provider-run-2".to_string()],
            }],
            duplicate_native_tui_agent_bindings: vec![ProviderRunAgentBindingConflict {
                session_id: "session-native".to_string(),
                agent_id: "agent-native".to_string(),
                provider_run_ids: vec![
                    "provider-run-native-1".to_string(),
                    "provider-run-native-2".to_string(),
                ],
            }],
            multi_interface_agent_bindings: vec![ProviderRunAgentBindingConflict {
                session_id: "session-2".to_string(),
                agent_id: "agent-2".to_string(),
                provider_run_ids: vec![
                    "provider-run-3:chariox".to_string(),
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
            relay_reconnect_attempts: 2,
            relay_last_reconnect_reason: Some("relay heartbeat send failed".to_string()),
            relay_last_reconnect_delay_ms: Some(750),
            relay_last_reconnect_url: Some("wss://relay-b.example.test".to_string()),
            relay_last_connected_url: Some("wss://relay-a.example.test".to_string()),
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
            provider_auth_missing_slices: 0,
            provider_auth_unconfigured_slices: 0,
            provider_auth_issues: Vec::new(),
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
            .duplicate_chariox_agent_bindings
            .len(),
        1
    );
    assert_eq!(
        projection
            .provider_runs
            .duplicate_native_tui_agent_bindings
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
    assert_eq!(projection.transport.relay_reconnect_attempts, 2);
    assert_eq!(
        projection.transport.relay_last_reconnect_reason.as_deref(),
        Some("relay heartbeat send failed")
    );
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
        slice_record(
            "slice-stopped",
            "dev-stopped",
            crate::slice::SliceStatus::Stopped,
            None,
            None,
        ),
    ]);

    assert_eq!(snapshot.total_slices, 3);
    assert_eq!(snapshot.running_slices, 1);
    assert_eq!(snapshot.stopped_slices, 1);
    assert_eq!(snapshot.unhealthy_slices, 1);
    assert_eq!(snapshot.failed_operations, 1);
    assert_eq!(snapshot.issues.len(), 2);
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
    let stopped_issue = &snapshot.issues[1];
    assert_eq!(stopped_issue.slice_id, "slice-stopped");
    assert_eq!(stopped_issue.status, "stopped");
    assert_eq!(stopped_issue.agent_ids, vec!["agent-1"]);
}

#[test]
fn slice_lifecycle_health_identifies_provider_auth_issues_for_attached_slices() {
    let missing_auth = slice_record(
        "slice-missing-auth",
        "dev-missing-auth",
        crate::slice::SliceStatus::Running,
        None,
        None,
    );
    let mut stale_auth = slice_record(
        "slice-stale-auth",
        "dev-stale-auth",
        crate::slice::SliceStatus::Running,
        None,
        None,
    );
    stale_auth.provider_auth = vec![crate::slice_provider_auth::SliceProviderAuthSummary {
        provider: "codex".to_string(),
        state: crate::slice_provider_auth::SliceProviderAuthState::NotConfigured,
        auth_type: Some("chatgpt".to_string()),
        account_id: Some("acct-1".to_string()),
        email: None,
        organization_id: None,
        organization_name: None,
        subscription_type: None,
        alias: Some("work".to_string()),
        source: "slice".to_string(),
    }];

    let snapshot = SliceLifecycleHealthSnapshot::from_slices(&[missing_auth, stale_auth]);

    assert_eq!(snapshot.provider_auth_missing_slices, 1);
    assert_eq!(snapshot.provider_auth_unconfigured_slices, 1);
    assert_eq!(snapshot.provider_auth_issues.len(), 2);
    assert_eq!(
        snapshot.provider_auth_issues[0].details,
        "slice has attached agents but no codex provider account configured"
    );
    assert_eq!(
        snapshot.provider_auth_issues[0].provider.as_deref(),
        Some("codex")
    );
    assert_eq!(
        snapshot.provider_auth_issues[1].provider.as_deref(),
        Some("codex")
    );
    assert_eq!(
        snapshot.provider_auth_issues[1]
            .provider_auth_state
            .as_deref(),
        Some("not_configured")
    );
    assert_eq!(
        snapshot.provider_auth_issues[1].identity.as_deref(),
        Some("work")
    );
}

#[test]
fn slice_lifecycle_health_reports_partial_provider_auth_coverage() {
    let mut partial_auth = slice_record(
        "slice-partial-auth",
        "dev-partial-auth",
        crate::slice::SliceStatus::Running,
        None,
        None,
    );
    partial_auth.providers = vec!["codex".to_string(), "opencode".to_string()];
    partial_auth.provider_auth = vec![crate::slice_provider_auth::SliceProviderAuthSummary {
        provider: "codex".to_string(),
        state: crate::slice_provider_auth::SliceProviderAuthState::Configured,
        auth_type: Some("chatgpt".to_string()),
        account_id: Some("acct-1".to_string()),
        email: None,
        organization_id: None,
        organization_name: None,
        subscription_type: None,
        alias: Some("work".to_string()),
        source: "slice".to_string(),
    }];

    let snapshot = SliceLifecycleHealthSnapshot::from_slices(&[partial_auth]);

    assert_eq!(snapshot.provider_auth_missing_slices, 1);
    assert_eq!(snapshot.provider_auth_unconfigured_slices, 0);
    assert_eq!(snapshot.provider_auth_issues.len(), 1);
    assert_eq!(
        snapshot.provider_auth_issues[0].provider.as_deref(),
        Some("opencode")
    );
    assert_eq!(
        snapshot.provider_auth_issues[0].details,
        "slice has attached agents but no opencode provider account configured"
    );
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
fn remote_extension_sync_health_reports_pending_revoke_after_last_grant_removed() {
    let mut revoked = remote_agent("agent-revoked");
    revoked.set_remote_extension_manifest_sync(Some(
        RemoteExtensionManifestSyncStatus::pending("hash-empty-manifest".to_string(), true)
            .failed("worker offline"),
    ));

    let snapshot = RemoteExtensionSyncHealthSnapshot::from_agents(&[revoked]);

    assert_eq!(snapshot.remote_agents, 1);
    assert_eq!(snapshot.home_proxy_agents, 1);
    assert_eq!(snapshot.home_proxy_grants, 0);
    assert_eq!(snapshot.failed_agents, 1);
    assert_eq!(snapshot.pending_revoke_agents, 1);
    assert_eq!(snapshot.issues.len(), 1);
    assert_eq!(snapshot.issues[0].agent_id, "agent-revoked");
    assert_eq!(snapshot.issues[0].state, "failed");
    assert_eq!(snapshot.issues[0].pending_revoke, true);
    assert!(snapshot.issues[0].home_proxy_grants.is_empty());
}

#[test]
fn remote_execution_health_reports_active_agent_without_worker_run() {
    let mut healthy_idle = remote_agent("agent-idle");
    healthy_idle.set_remote_execution_active_worker_provider_run_id(None);

    let mut missing_run = remote_agent("agent-working");
    missing_run.set_remote_execution_active_worker_provider_run_id(None);
    missing_run.set_state(AgentState::Working);
    missing_run.set_processing(true);

    let mut empty_run = remote_agent("agent-empty-run");
    empty_run.set_remote_execution_active_worker_provider_run_id(Some(String::new()));
    empty_run.set_state(AgentState::Working);
    empty_run.set_processing(true);

    let mut malformed = remote_agent("agent-malformed");
    malformed.set_remote_execution(Some(RemoteAgentBinding {
        worker_kernel_id: String::new(),
        worker_machine_id: "worker-machine".to_string(),
        execution_lease_id: String::new(),
        leased_agent_id: "leased-agent-1".to_string(),
        active_worker_provider_run_id: Some("worker-run-1".to_string()),
        relay_url: None,
        relay_token: None,
        relay_peer_protocol_version: Some(
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        ),
    }));

    let active_agent_ids =
        BTreeSet::from(["agent-working".to_string(), "agent-empty-run".to_string()]);
    let snapshot = RemoteExecutionHealthSnapshot::from_agents_with_active_agent_ids(
        &[
            healthy_idle,
            missing_run,
            empty_run,
            malformed,
            local_agent("agent-local"),
        ],
        &active_agent_ids,
    );

    assert_eq!(snapshot.remote_agents, 4);
    assert_eq!(snapshot.active_remote_agents, 2);
    assert_eq!(snapshot.missing_active_worker_runs, 2);
    assert_eq!(snapshot.malformed_bindings, 2);
    assert_eq!(
        snapshot
            .issues
            .iter()
            .map(|issue| (issue.agent_id.as_str(), issue.kind.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("agent-working", "missing_active_worker_provider_run"),
            ("agent-empty-run", "malformed_binding"),
            ("agent-empty-run", "missing_active_worker_provider_run"),
            ("agent-malformed", "malformed_binding"),
        ]
    );
    assert!(snapshot.issues[0].is_processing);
    assert_eq!(snapshot.issues[0].state, "working");
    assert_eq!(snapshot.issues[0].worktree_id.as_deref(), Some("/repo"));
    assert!(snapshot.issues[1]
        .details
        .contains("active_worker_provider_run_id"));
    assert!(snapshot.issues[3]
        .details
        .contains("worker_kernel_id, execution_lease_id"));
}

#[test]
fn remote_execution_health_ignores_stale_legacy_worker_flags() {
    let mut stale = remote_agent("agent-stale-working");
    stale.set_remote_execution_active_worker_provider_run_id(None);
    stale.set_state(AgentState::Working);
    stale.set_processing(true);

    let snapshot = RemoteExecutionHealthSnapshot::from_agents_with_active_agent_ids(
        &[stale],
        &BTreeSet::new(),
    );

    assert_eq!(snapshot.remote_agents, 1);
    assert_eq!(snapshot.active_remote_agents, 0);
    assert_eq!(snapshot.missing_active_worker_runs, 0);
    assert!(snapshot.issues.is_empty());
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
        relay_peer_protocol_version: Some(
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        ),
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
        saved_state_ref: None,
        saved_state_status: None,
        saved_state_updated_at_ms: None,
        display_endpoint: None,
        created_at_ms: 1,
        updated_at_ms: 2,
    }
}
