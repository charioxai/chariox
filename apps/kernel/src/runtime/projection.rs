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
mod transport_health;

pub(crate) use agent_runtime_projection::{AgentRuntimeProjection, AgentRuntimeProjectionStore};
pub(crate) use config_projection::DaemonConfigProjectionStore;
pub use daemon_health_model::{
    ActorQueueSnapshot, AgentRuntimeProjectionHealthSnapshot, DaemonHealthProjection,
    ManagedIoHealthSnapshot, ProjectionInvariantHealthSnapshot, ProjectionInvariantMismatch,
    ProviderCatalogHealthSnapshot, ProviderRunActorHealthSnapshot, SessionProjectionHealthSnapshot,
    WorkspaceCoordinationHealthSnapshot, WorktreeClaimSnapshot,
};
pub(crate) use provider_projection::{
    ProviderCatalogProjectionStore, ProviderProcessProjectionStore, ProviderRunProjectionStore,
};
pub(crate) use remote_relay_inventory_projection::RemoteRelayInventoryProjectionStore;
pub(crate) use session_history_projection::{page_history_entries, SessionHistoryProjectionStore};
pub(crate) use session_snapshot_projection::agent_activity_for_session_projection;
pub use session_snapshot_projection::{
    AgentActiveTurnProjection, AgentPromptRuntimeStatus, AgentRuntimeActivity, AgentRuntimeStatus,
    SessionSnapshotProjection,
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

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use crate::agent::CreateAgentRequest;
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::provider::{LaunchProviderRequest, RuntimeProviderRun};
    use crate::runtime::capability_executor::CapabilityExecutorHealthSnapshot;
    use crate::runtime::projection::{
        ActorQueueSnapshot, AgentPromptRuntimeStatus, AgentRuntimeProjectionHealthSnapshot,
        AgentRuntimeProjectionStore, AgentRuntimeStatus, DaemonHealthProjection,
        ManagedIoHealthSnapshot, ProjectionInvariantHealthSnapshot, ProviderCatalogHealthSnapshot,
        ProviderRunActorHealthSnapshot, RemoteRelayInventoryProjectionStore,
        SessionProjectionHealthSnapshot, SessionSnapshotProjection, SessionStateProjectionStore,
        TransportHealthSnapshot, WorkspaceCoordinationHealthSnapshot,
    };
    use crate::session::CreateSessionRequest;
    use crate::terminal::TerminalStreamHealthSnapshot;
    use crate::{DaemonApp, DaemonConfig};

    fn launch_dev_stub_provider(
        app: &mut DaemonApp,
        session_id: &str,
        agent_id: &str,
    ) -> RuntimeProviderRun {
        let provider_run = app
            .launch_provider(
                LaunchProviderRequest::new(
                    session_id,
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(agent_id),
            )
            .expect("provider run should launch");
        app.update_provider_run_projection(provider_run.clone());
        provider_run
    }

    fn submit_prompt(
        app: &mut DaemonApp,
        session_id: &str,
        attachment_id: &str,
        agent_id: &str,
        prompt: &str,
    ) {
        crate::app::KernelAgentService::new(app)
            .submit_prompt(
                session_id,
                attachment_id,
                Some(agent_id),
                prompt,
                Vec::new(),
            )
            .expect("prompt should submit");
    }

    fn attach_cli(app: &mut DaemonApp, session_id: &str, client_id: &str) -> String {
        let mut sessions = app.sessions_mut();
        let attachment = app
            .attachments()
            .attach(
                &mut sessions,
                AttachRequest::new(session_id, client_id, ClientCapabilityLevel::FullTerminal),
            )
            .expect("session should attach");
        attachment.id().to_string()
    }

    #[test]
    fn session_snapshot_projection_includes_metadata_and_agents() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");

        let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
            .expect("projection should build");

        assert_eq!(projection.metadata.projection_version, 2);
        assert_eq!(projection.metadata.last_event_id, 42);
        assert_eq!(projection.session.id(), session.id());
        assert_eq!(projection.session.agents().len(), 1);
    }

    #[test]
    fn session_snapshot_projection_marks_settling_prompt_as_working() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let provider_run = launch_dev_stub_provider(&mut app, session.id(), agent.id());
        let attachment_id = attach_cli(&mut app, session.id(), "cli-settling");
        submit_prompt(
            &mut app,
            session.id(),
            &attachment_id,
            agent.id(),
            "status check",
        );
        app.prompt_activity_store().write().insert(
            provider_run.id().to_string(),
            crate::app::ActivePromptState {
                last_output_at: None,
                saw_response_content: true,
                completion_recorded: true,
                settlement_requested: true,
            },
        );
        let active_turns = app.active_turn_store();
        active_turns.start(crate::app::ActiveTurnState::new(
            session.id().to_string(),
            agent.id().to_string(),
            "prompt-settling".to_string(),
            provider_run.id().to_string(),
        ));
        active_turns.mark_settling(provider_run.id());

        let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
            .expect("projection should build");
        let activity = projection
            .agent_activity
            .get(agent.id())
            .expect("agent activity should be projected");

        assert_eq!(activity.status, AgentRuntimeStatus::Working);
        assert_eq!(activity.prompt_status, AgentPromptRuntimeStatus::Settling);
        assert!(activity.busy);
    }

    #[test]
    fn session_snapshot_projection_keeps_active_turn_working_without_active_prompt_activity() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let provider_run = launch_dev_stub_provider(&mut app, session.id(), agent.id());
        app.active_turn_store()
            .start(crate::app::ActiveTurnState::new(
                session.id().to_string(),
                agent.id().to_string(),
                "prompt-restored".to_string(),
                provider_run.id().to_string(),
            ));

        let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
            .expect("projection should build");
        let activity = projection
            .agent_activity
            .get(agent.id())
            .expect("agent activity should be projected");

        assert_eq!(activity.status, AgentRuntimeStatus::Working);
        assert_eq!(activity.prompt_status, AgentPromptRuntimeStatus::Running);
        assert!(activity.busy);
        assert_eq!(
            activity
                .active_turn
                .as_ref()
                .map(|turn| turn.prompt_id.as_str()),
            Some("prompt-restored")
        );
    }

    #[test]
    fn session_snapshot_projection_ignores_prompt_activity_without_active_turn() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let provider_run = launch_dev_stub_provider(&mut app, session.id(), agent.id());
        app.prompt_activity_store().write().insert(
            provider_run.id().to_string(),
            crate::app::ActivePromptState {
                last_output_at: Some(Instant::now()),
                saw_response_content: true,
                completion_recorded: false,
                settlement_requested: true,
            },
        );

        let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
            .expect("projection should build");
        let activity = projection
            .agent_activity
            .get(agent.id())
            .expect("agent activity should be projected");

        assert_eq!(activity.status, AgentRuntimeStatus::Idle);
        assert_eq!(activity.prompt_status, AgentPromptRuntimeStatus::None);
        assert!(!activity.busy);
        assert!(activity.active_turn.is_none());
    }

    #[test]
    fn session_snapshot_projection_marks_completed_prompt_as_idle() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let provider_run = launch_dev_stub_provider(&mut app, session.id(), agent.id());
        let attachment_id = attach_cli(&mut app, session.id(), "cli-idle");
        submit_prompt(
            &mut app,
            session.id(),
            &attachment_id,
            agent.id(),
            "status check",
        );
        app.complete_active_prompt(session.id(), agent.id(), Some(provider_run.id()))
            .expect("prompt should complete");

        let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
            .expect("projection should build");
        let activity = projection
            .agent_activity
            .get(agent.id())
            .expect("agent activity should be projected");

        assert_eq!(activity.status, AgentRuntimeStatus::Idle);
        assert_eq!(activity.prompt_status, AgentPromptRuntimeStatus::None);
        assert!(!activity.busy);
    }

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
                active_worktree_claims: vec![crate::runtime::projection::WorktreeClaimSnapshot {
                    workspace_id: "workspace-1".to_string(),
                    worktree_id: "worktree-1".to_string(),
                    session_ids: vec!["session-1".to_string(), "session-2".to_string()],
                }],
                worktree_collisions: vec![crate::runtime::projection::WorktreeClaimSnapshot {
                    workspace_id: "workspace-1".to_string(),
                    worktree_id: "worktree-1".to_string(),
                    session_ids: vec!["session-1".to_string(), "session-2".to_string()],
                }],
                active_operation_claims: Vec::new(),
            },
            ManagedIoHealthSnapshot {
                active_reservations: 2,
                active_reservation_artifacts: 1,
                workspace_identity: crate::runtime::workspace_identity_monitor::WorkspaceIdentityMonitorHealthSnapshot {
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
        assert_eq!(projection.capability_executor.submitted_jobs, 8);
        assert_eq!(projection.capability_executor.running_jobs, 1);
        assert_eq!(projection.session_projection.active_prompts, 1);
        assert_eq!(projection.session_projection.queued_prompts, 2);
        assert_eq!(projection.agent_runtime_projection.projected_agents, 3);
        assert_eq!(projection.agent_runtime_projection.active_prompts, 1);
        assert!(projection.provider_catalog.cached);
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
        assert_eq!(projection.managed_io.active_reservations, 2);
        assert_eq!(projection.managed_io.active_reservation_artifacts, 1);
        assert_eq!(
            projection
                .managed_io
                .workspace_identity
                .invalid_provider_runs,
            1
        );
        assert_eq!(projection.managed_io.external_changes.tracked_artifacts, 4);
        assert_eq!(
            projection
                .managed_io
                .external_changes
                .external_change_events,
            5
        );
        assert!(projection.managed_io.external_changes.live_watcher_started);
        assert_eq!(projection.projection_invariants.checked_agents, 3);
        assert!(projection.projection_invariants.mismatches.is_empty());
    }

    #[test]
    fn agent_runtime_projection_reads_agent_prompt_state() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                &session_id,
                "cli-agent-runtime-projection",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attach should succeed");
        launch_dev_stub_provider(&mut app, &session_id, &agent_id);
        submit_prompt(
            &mut app,
            &session_id,
            attachment.id(),
            &agent_id,
            "first prompt",
        );
        submit_prompt(
            &mut app,
            &session_id,
            attachment.id(),
            &agent_id,
            "queued prompt",
        );

        let session = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(&session_id)
            .expect("session snapshot should load");
        let store = AgentRuntimeProjectionStore::default();
        store.update_session(&session);

        let projection = store
            .get(&agent_id)
            .expect("agent projection should be available");
        assert_eq!(projection.session_id, session_id);
        assert_eq!(projection.agent_id, agent_id);
        assert!(projection.active_prompt.is_some());
        assert_eq!(
            projection
                .next_queued_prompt
                .as_ref()
                .map(|prompt| prompt.prompt()),
            Some("queued prompt")
        );
        assert_eq!(projection.queued_prompt_count, 1);
        assert_eq!(
            store
                .next_queued_prompt(&projection.session_id, &projection.agent_id)
                .as_ref()
                .map(|prompt| prompt.prompt()),
            Some("queued prompt")
        );
        assert_eq!(
            store.list_for_session(&projection.session_id),
            vec![projection]
        );
        assert_eq!(store.health_snapshot().active_prompts, 1);
        assert_eq!(store.health_snapshot().queued_prompts, 1);
    }

    #[test]
    fn agent_runtime_projection_can_refresh_one_agent_without_stomping_peers() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, first_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let first_agent_id = first_agent.id().to_string();
        let second_agent_id = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(CreateAgentRequest::new(&session_id, "claude-code").with_alias("peer"))
            .expect("second agent should spawn")
            .id()
            .to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                &session_id,
                "cli-agent-runtime-one-agent-refresh",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attach should succeed");
        for agent_id in [&first_agent_id, &second_agent_id] {
            launch_dev_stub_provider(&mut app, &session_id, agent_id);
        }

        submit_prompt(
            &mut app,
            &session_id,
            attachment.id(),
            &first_agent_id,
            "first active",
        );
        let first_only_snapshot = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(&session_id)
            .expect("first snapshot should load");
        submit_prompt(
            &mut app,
            &session_id,
            attachment.id(),
            &second_agent_id,
            "second active",
        );
        let both_snapshot = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(&session_id)
            .expect("second snapshot should load");

        let store = AgentRuntimeProjectionStore::default();
        store.update_session(&both_snapshot);
        assert!(store
            .get(&second_agent_id)
            .and_then(|projection| projection.active_prompt)
            .is_some());

        store.update_agent_from_session(&first_only_snapshot, &first_agent_id);
        assert!(
            store
                .get(&second_agent_id)
                .and_then(|projection| projection.active_prompt)
                .is_some(),
            "single-agent refresh should not erase newer peer prompt state"
        );
    }

    #[test]
    fn projection_invariant_health_reports_agent_runtime_drift() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                &session_id,
                "cli-projection-invariant",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        launch_dev_stub_provider(&mut app, &session_id, &agent_id);
        submit_prompt(
            &mut app,
            &session_id,
            attachment.id(),
            &agent_id,
            "active prompt",
        );
        submit_prompt(
            &mut app,
            &session_id,
            attachment.id(),
            &agent_id,
            "queued prompt",
        );

        let session = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(&session_id)
            .expect("session snapshot should load");
        let session_store = SessionStateProjectionStore::default();
        let agent_store = AgentRuntimeProjectionStore::default();
        session_store.update(session.clone());
        agent_store.update_session(&session);
        let clean_snapshot = session_store.invariant_snapshot(&agent_store);
        assert_eq!(clean_snapshot.checked_sessions, 1);
        assert_eq!(clean_snapshot.checked_agents, 1);
        assert!(clean_snapshot.mismatches.is_empty());

        let projection = agent_store
            .get(&agent_id)
            .expect("agent projection should exist before drift injection");
        agent_store.update_agent_prompt_state(
            &session_id,
            &agent_id,
            projection.active_prompt,
            None,
            0,
        );

        let drift_snapshot = session_store.invariant_snapshot(&agent_store);
        assert!(drift_snapshot
            .mismatches
            .iter()
            .any(|mismatch| mismatch.kind == "queue_front_mismatch"));
        assert!(drift_snapshot
            .mismatches
            .iter()
            .any(|mismatch| mismatch.kind == "queued_prompt_count_mismatch"));
    }

    #[test]
    fn workspace_coordination_snapshot_reports_worktree_collisions() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (first, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-1", "shared-worktree"))
            .expect("first session should be created");
        let (second, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-1", "shared-worktree"))
            .expect("second session should be created");
        let (other_workspace, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-2", "shared-worktree"))
            .expect("other workspace session should be created");

        let store = SessionStateProjectionStore::default();
        store.update_list(vec![first.clone(), second.clone(), other_workspace]);

        let snapshot = store.workspace_coordination_snapshot(Vec::new());
        assert_eq!(snapshot.active_worktree_claims.len(), 2);
        assert_eq!(snapshot.worktree_collisions.len(), 1);
        assert!(snapshot.active_operation_claims.is_empty());
        assert_eq!(
            snapshot.worktree_collisions[0].session_ids,
            vec![first.id().to_string(), second.id().to_string()]
        );
    }

    #[test]
    fn remote_relay_inventory_projection_requests_refresh_when_empty_or_stale() {
        let projection = RemoteRelayInventoryProjectionStore::default();
        assert!(projection.should_request_refresh(10_000, 5_000, 1_000));
        assert!(
            !projection.should_request_refresh(10_500, 5_000, 1_000),
            "refresh should respect the cooldown while the projection remains empty"
        );
        assert!(
            projection.should_request_refresh(16_000, 5_000, 1_000),
            "stale empty projection should request another refresh after the cooldown"
        );
    }
}
