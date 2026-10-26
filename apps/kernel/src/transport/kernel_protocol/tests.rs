use super::*;
use crate::agent::RemoteAgentBinding;
use crate::agent::{AgentInstance, GridPosition};
use crate::provider::RuntimeProviderRun;
use crate::runtime::projection::ProjectionMetadata;
use crate::session::{
    PromptQueueItem, PromptStatus, RuntimeInteraction, RuntimeInteractionChoice,
    RuntimeInteractionChoiceStyle, RuntimeInteractionKind, RuntimeInteractionLevel, RuntimeSession,
    WorkflowRunStatus,
};
use crate::terminal::TerminalOutputKind;

#[test]
fn credential_vault_locked_uses_a_stable_transport_error_code() {
    let error = DaemonError::LocalTransport {
        operation: "credential_vault_locked",
        message: "Chariox vault is locked".to_string(),
    };

    let mapped = map_kernel_error(&error);

    assert_eq!(mapped.code, "credential_vault_locked");
    assert!(!mapped.retryable);
}

#[test]
fn a_leased_backing_session_refusal_keeps_its_own_code_for_clients() {
    let error = DaemonError::LeasedBackingSessionRefused {
        session_id: "backing".to_string(),
        home_session_id: "home".to_string(),
    };

    let mapped = map_kernel_error(&error);

    assert_eq!(mapped.code, "leased_backing_session_refused");
    assert!(mapped.message.contains("home"));
    assert!(!mapped.retryable);
}

#[test]
fn relay_transport_errors_keep_the_stable_local_transport_projection() {
    let error = DaemonError::RelayTransport {
        operation: "read relay peer response",
        code: "target_not_connected".to_string(),
        message: "target daemon is not connected to relay".to_string(),
        retryable: true,
    };

    let mapped = map_kernel_error(&error);

    assert_eq!(mapped.code, "local_transport_error");
    assert_eq!(mapped.message, error.to_string());
    assert!(mapped.retryable);

    let business_error = DaemonError::RelayTransport {
        operation: "read relay peer response",
        code: "leased_agent_not_found".to_string(),
        message: "worker does not have the leased agent".to_string(),
        retryable: false,
    };
    let mapped = map_kernel_error(&business_error);

    assert_eq!(mapped.code, "local_transport_error");
    assert_eq!(mapped.message, business_error.to_string());
    assert!(!mapped.retryable);
}

#[test]
fn verified_cleanup_absence_keeps_existing_wire_error_codes() {
    let error = DaemonError::RelayPeerCleanupAbsent {
        worker_kernel_id: "worker".into(),
        resource_id: "agent".into(),
        code: "leased_agent_not_found",
    };
    let local = map_kernel_error(&error);
    assert_eq!(local.code, "local_transport_error");
    assert!(!local.retryable);
}

#[test]
fn terminal_output_event_batches_stay_under_json_byte_cap() {
    let records = (0..20)
        .map(|index| TerminalOutputRecord {
            record_id: Some(index as u64),
            sequence: None,
            sequence_end: None,
            sequence_byte_lengths: Vec::new(),
            timestamp_ms: 1_000 + index as u64,
            session_id: "session-1".to_string(),
            provider_run_id: "provider-run-1".to_string(),
            agent_id: Some("agent-1".to_string()),
            prompt_id: None,
            prompt_origin: None,
            source_attachment_id: None,
            kind: TerminalOutputKind::ProviderOutput,
            merge_key: Some(format!("chunk-{index}")),
            recipient_attachment_ids: vec!["attachment-1".to_string()],
            pending_recipient_attachment_ids: vec!["attachment-1".to_string()],
            bytes: vec![b'x'; 16 * 1024],
            external_observation_metadata: None,
        })
        .collect::<Vec<_>>();

    let batches = terminal_output_event_batches(records);

    assert!(batches.len() > 1);
    for batch in batches {
        assert!(terminal_output_event_json_bytes(&batch) <= MAX_TERMINAL_OUTPUT_EVENT_JSON_BYTES);
    }
}

#[test]
fn session_snapshot_frame_redacts_remote_relay_credentials() {
    let mut projection = session_snapshot_with_agent();
    let mut agent = projection.session.agents()[0].clone();
    agent.set_remote_execution(Some(RemoteAgentBinding {
        worker_kernel_id: "worker-1".to_string(),
        worker_machine_id: "machine-1".to_string(),
        execution_lease_id: "lease-1".to_string(),
        leased_agent_id: "agent-a".to_string(),
        active_worker_provider_run_id: None,
        relay_url: Some("wss://relay.example".to_string()),
        relay_token: Some("snapshot-secret".to_string()),
        relay_peer_protocol_version: Some(
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        ),
    }));
    projection.session.set_agents(vec![agent]);
    let frame = KernelOutgoingFrame::Event {
        event_id: 1,
        event: Box::new(KernelEvent::SessionSnapshot {
            room_workflows: projection.room_workflows,
            session: Box::new(projection.session),
            provider_run: Box::new(None),
            agent_activity: Box::new(projection.agent_activity),
            agent_activity_revision: 0,
        }),
    };

    let encoded = serialize_frame(&frame).expect("session snapshot should serialize");
    let value: serde_json::Value = serde_json::from_str(&encoded).expect("valid JSON frame");
    assert!(value
        .pointer("/event/session/agents/0/remote_execution/relay_token")
        .is_none());
    assert_eq!(
        value
            .pointer("/event/session/agents/0/remote_execution/relay_url")
            .and_then(serde_json::Value::as_str),
        Some("wss://relay.example")
    );
}

#[test]
fn terminal_output_event_batch_size_accounting_matches_serialized_json() {
    let records = (0..5)
        .map(|index| TerminalOutputRecord {
            record_id: Some(index as u64),
            sequence: None,
            sequence_end: None,
            sequence_byte_lengths: Vec::new(),
            timestamp_ms: 2_000 + index as u64,
            session_id: format!("session-{index}"),
            provider_run_id: format!("provider-run-{index}"),
            agent_id: Some(format!("agent-{index}")),
            prompt_id: None,
            prompt_origin: None,
            source_attachment_id: None,
            kind: TerminalOutputKind::ProviderOutput,
            merge_key: Some(format!("chunk-{index}")),
            recipient_attachment_ids: vec!["attachment-1".to_string()],
            pending_recipient_attachment_ids: vec!["attachment-1".to_string()],
            bytes: vec![b'x'; 256 + index],
            external_observation_metadata: None,
        })
        .collect::<Vec<_>>();
    let accounted_bytes =
        records
            .iter()
            .fold(empty_terminal_output_event_json_bytes(), |bytes, record| {
                let comma = usize::from(bytes != empty_terminal_output_event_json_bytes());
                bytes + terminal_output_record_json_bytes(record) + comma
            });

    assert_eq!(accounted_bytes, terminal_output_event_json_bytes(&records));
}

#[test]
fn waiting_room_rows_changed_event_sends_only_changed_and_removed_rows() {
    let previous = waiting_room_snapshot(
        "inventory-a",
        vec![
            session_summary("session-a", 1),
            session_summary("session-b", 1),
        ],
    );
    let current = waiting_room_snapshot(
        "inventory-b",
        vec![
            session_summary("session-a", 2),
            session_summary("session-c", 1),
        ],
    );

    let event = waiting_room_rows_changed_event(current, Some(&previous))
        .expect("inventory version change should produce event");

    match event {
        KernelEvent::WaitingRoomRowsChanged {
            inventory_version,
            schema_version,
            sessions,
            removed_session_ids,
            ..
        } => {
            assert_eq!(inventory_version, "inventory-b");
            assert_eq!(schema_version, 11);
            assert_eq!(
                sessions
                    .iter()
                    .map(|session| session.id.as_str())
                    .collect::<Vec<_>>(),
                vec!["session-a", "session-c"]
            );
            assert_eq!(removed_session_ids, vec!["session-b"]);
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn waiting_room_inventory_changed_event_refreshes_for_provider_accounts() {
    let previous = waiting_room_snapshot("inventory-a", Vec::new());
    let mut current = previous.clone();
    current.inventory_version = "inventory-b".to_string();
    current.provider_accounts.push(provider_account_profile());

    assert!(matches!(
        waiting_room_inventory_changed_event(&current, Some(&previous)),
        Some(KernelEvent::WaitingRoomInventoryChanged { inventory_version })
            if inventory_version == "inventory-b"
    ));
}

#[test]
fn waiting_room_inventory_event_projection_tracks_full_and_row_refreshes_once() {
    let mut projection = WaitingRoomInventoryEventProjection::default();
    let initial = waiting_room_snapshot("inventory-a", Vec::new());

    let initial_events = projection.project(initial.clone());
    assert_eq!(initial_events.len(), 2);
    assert!(matches!(
        initial_events.as_slice(),
        [
            KernelEvent::WaitingRoomRowsChanged { .. },
            KernelEvent::WaitingRoomInventoryChanged { .. }
        ]
    ));

    let mut account_changed = initial;
    account_changed.inventory_version = "inventory-b".to_string();
    account_changed
        .provider_accounts
        .push(provider_account_profile());
    let account_events = projection.project(account_changed.clone());
    assert!(matches!(
        account_events.as_slice(),
        [KernelEvent::WaitingRoomInventoryChanged { inventory_version }]
            if inventory_version == "inventory-b"
    ));
    assert!(projection.project(account_changed).is_empty());

    let mut row_changed =
        waiting_room_snapshot("inventory-c", vec![session_summary("session-a", 1)]);
    row_changed
        .provider_accounts
        .push(provider_account_profile());
    assert!(matches!(
        projection.project(row_changed).as_slice(),
        [KernelEvent::WaitingRoomRowsChanged { inventory_version, .. }]
            if inventory_version == "inventory-c"
    ));
}

#[test]
fn waiting_room_inventory_changed_event_skips_row_only_changes() {
    let previous = waiting_room_snapshot("inventory-a", vec![session_summary("session-a", 1)]);
    let current = waiting_room_snapshot("inventory-b", vec![session_summary("session-a", 2)]);

    assert!(waiting_room_inventory_changed_event(&current, Some(&previous)).is_none());
}

#[test]
fn waiting_room_rows_changed_event_sends_project_upserts_and_removals() {
    let mut previous = waiting_room_snapshot("inventory-a", Vec::new());
    previous.projects = vec![
        project_summary("project-a", "A"),
        project_summary("project-b", "B"),
    ];
    let mut current = waiting_room_snapshot("inventory-b", Vec::new());
    current.projects = vec![
        project_summary("project-a", "A renamed"),
        project_summary("project-c", "C"),
    ];

    let event = waiting_room_rows_changed_event(current, Some(&previous))
        .expect("project changes should produce a row event");
    match event {
        KernelEvent::WaitingRoomRowsChanged {
            projects,
            removed_project_ids,
            ..
        } => {
            assert_eq!(
                projects
                    .iter()
                    .map(|project| project.id.as_str())
                    .collect::<Vec<_>>(),
                vec!["project-a", "project-c"]
            );
            assert_eq!(removed_project_ids, vec!["project-b"]);
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn waiting_room_rows_changed_event_keeps_archived_ended_session_and_restores_it() {
    let mut active =
        waiting_room_snapshot("inventory-active", vec![session_summary("session-a", 1)]);
    active.projects = vec![project_summary("project-a", "A")];

    let mut ended = session_summary("session-a", 1);
    ended.status = crate::session::SessionStatus::Ended;
    let mut archived_project = project_summary("project-a", "A");
    archived_project.status = crate::session::RuntimeProjectStatus::Archived;
    archived_project.archived_at_ms = Some(2);
    let mut archived = waiting_room_snapshot("inventory-archived", vec![ended]);
    archived.projects = vec![archived_project];

    let archived_event = waiting_room_rows_changed_event(archived.clone(), Some(&active))
        .expect("archiving should upsert the ended drill-down row");
    match archived_event {
        KernelEvent::WaitingRoomRowsChanged {
            sessions,
            removed_session_ids,
            projects,
            ..
        } => {
            assert_eq!(sessions.len(), 1);
            assert_eq!(sessions[0].id, "session-a");
            assert_eq!(sessions[0].status, crate::session::SessionStatus::Ended);
            assert!(removed_session_ids.is_empty());
            assert_eq!(
                projects[0].status,
                crate::session::RuntimeProjectStatus::Archived
            );
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let mut parked = session_summary("session-a", 1);
    parked.status = crate::session::SessionStatus::Parked;
    let mut restored = waiting_room_snapshot("inventory-restored", vec![parked]);
    restored.projects = vec![project_summary("project-a", "A")];
    let restored_event = waiting_room_rows_changed_event(restored, Some(&archived))
        .expect("restoring should upsert the parked drill-down row");
    match restored_event {
        KernelEvent::WaitingRoomRowsChanged {
            sessions,
            removed_session_ids,
            projects,
            ..
        } => {
            assert_eq!(sessions.len(), 1);
            assert_eq!(sessions[0].status, crate::session::SessionStatus::Parked);
            assert!(removed_session_ids.is_empty());
            assert_eq!(
                projects[0].status,
                crate::session::RuntimeProjectStatus::Active
            );
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn waiting_room_rows_changed_event_skips_unchanged_rows() {
    let previous = waiting_room_snapshot("inventory-a", vec![session_summary("session-a", 1)]);
    let current = waiting_room_snapshot("inventory-b", vec![session_summary("session-a", 1)]);

    assert!(waiting_room_rows_changed_event(current, Some(&previous)).is_none());
}

#[test]
fn workflow_run_only_changes_can_skip_session_snapshot() {
    let previous = session_snapshot_with_workflow_status(WorkflowRunStatus::Created);
    let mut current = previous.clone();
    current
        .session
        .workflow_run_mut("workflow-run-a")
        .expect("workflow run should exist")
        .set_status(WorkflowRunStatus::Running);

    assert!(workflow_run_only_changed(&current, Some(&previous)));
    let events = workflow_run_updated_events(&current, Some(&previous));

    assert_eq!(events.len(), 1);
    match &events[0] {
        KernelEvent::WorkflowRunUpdated {
            session_id,
            workflow_run,
        } => {
            assert_eq!(session_id, "session-a");
            assert_eq!(workflow_run.id(), "workflow-run-a");
            assert_eq!(workflow_run.status(), WorkflowRunStatus::Running);
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn provider_run_only_changes_can_skip_session_snapshot() {
    let mut previous = session_snapshot_with_workflow_status(WorkflowRunStatus::Created);
    previous.provider_run = Some(RuntimeProviderRun::from_control_capability_inference(
        "provider-run-a",
        "session-a".to_string(),
        Some("agent-a".to_string()),
        "codex".to_string(),
    ));
    let mut current = previous.clone();
    current
        .provider_run
        .as_mut()
        .expect("provider run should exist")
        .mark_running();

    let event = provider_run_changed_event(&current, Some(&previous))
        .expect("provider run only change should produce event");

    match event {
        KernelEvent::ProviderRunChanged {
            session_id,
            provider_run,
        } => {
            assert_eq!(session_id, "session-a");
            let provider_run = provider_run.expect("provider run should be included");
            assert_eq!(provider_run.id(), "provider-run-a");
            assert_eq!(
                provider_run.state(),
                crate::provider::ProviderRunState::Running
            );
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn prompt_runtime_activity_changes_can_skip_session_snapshot() {
    let previous = session_snapshot_with_agent();
    let mut current = previous.clone();
    let active_prompt = PromptQueueItem::new(
        "prompt-a",
        "attachment-a",
        "agent-a",
        "run it",
        PromptStatus::Running,
    );
    let active_prompt_value =
        serde_json::to_value(&active_prompt).expect("serialize active prompt");
    let mut session_value = serde_json::to_value(&current.session).expect("serialize session");
    session_value["prompt_runtime"] = serde_json::json!({
        "prompt_states": {
            "agent-a": {
                "active_prompt": active_prompt_value,
                "queued_prompts": [],
            },
        },
        "active_prompt": active_prompt_value,
        "queued_prompts": [],
        "scheduler_state": "Running",
    });
    current.session = serde_json::from_value(session_value).expect("deserialize session");
    current.agent_activity.insert(
        "agent-a".to_string(),
        AgentRuntimeActivity {
            status: crate::runtime::projection::AgentRuntimeStatus::Working,
            prompt_status: crate::runtime::projection::AgentPromptRuntimeStatus::Running,
            busy: true,
            active_prompt_count: 1,
            queued_prompt_count: 0,
            unread_idle_output: false,
            queued_prompt_controls: BTreeMap::new(),
            active_turn: Some(crate::runtime::projection::AgentActiveTurnProjection {
                prompt_id: "prompt-a".to_string(),
                provider_run_id: None,
                source_attachment_id: Some("attachment-a".to_string()),
                status: crate::runtime::projection::AgentPromptRuntimeStatus::Running,
                phase: crate::runtime::projection::AgentTurnRuntimePhase::Accepted,
                prompt_origin: Some(crate::session::PromptOrigin::Chariox),
                external_provider: None,
                external_provider_session_id: None,
                external_provider_turn_id: None,
                started_at_ms: None,
            }),
            last_completed_turn: None,
        },
    );

    let event = agent_activity_changed_event(&current, Some(&previous))
        .expect("prompt runtime activity change should produce agent activity delta");

    match event {
        KernelEvent::AgentActivityChanged {
            session_id,
            agent_activity,
            ..
        } => {
            assert_eq!(session_id, "session-a");
            assert_eq!(
                agent_activity
                    .get("agent-a")
                    .expect("agent activity")
                    .prompt_status,
                crate::runtime::projection::AgentPromptRuntimeStatus::Running
            );
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn session_metadata_only_changes_can_skip_session_snapshot() {
    let previous = session_snapshot_with_workflow_status(WorkflowRunStatus::Created);
    let mut current = previous.clone();
    current.session.set_alias(Some("daily".to_string()));
    current
        .session
        .set_workspace_live_sync_mode(Some(WorkspaceLiveSyncMode::Tracked));

    let event = session_metadata_changed_event(&current, Some(&previous))
        .expect("metadata only change should produce event");

    match event {
        KernelEvent::SessionMetadataChanged {
            session_id,
            metadata,
        } => {
            assert_eq!(session_id, "session-a");
            assert_eq!(metadata.alias.as_deref(), Some("daily"));
            assert_eq!(
                metadata.workspace_live_sync_mode,
                Some(WorkspaceLiveSyncMode::Tracked)
            );
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn sudo_window_changes_cannot_be_hidden_by_projection_deltas() {
    let mut previous = session_snapshot_with_workflow_status(WorkflowRunStatus::Created);
    let window = crate::local::KernelSudoTurn {
        entry_id: "sudo:window-a".into(),
        session_id: "session-a".into(),
        agent_id: "agent-a".into(),
        owner_user_id: "local".into(),
        terminal_id: "owner-a".into(),
        requester: None,
        prompt_id: Some("work-a".into()),
        provider_run_id: Some("run-a".into()),
        task_id: Some("work-a".into()),
        duration_minutes: 60,
        expires_at_ms: Some(3_600_000),
        revision: 1,
        warning_sent: false,
        deadline: None,
        placement: None,
    };
    previous.session.set_sudo_windows(vec![window.clone()]);
    assert!(can_skip_session_snapshot(&previous, Some(&previous), true));
    for change in ["warning", "extension", "expiry"] {
        let mut current = previous.clone();
        let mut changed = window.clone();
        match change {
            "warning" => changed.warning_sent = true,
            "extension" => {
                changed.revision = 2;
                changed.expires_at_ms = Some(7_200_000);
            }
            "expiry" => {}
            _ => unreachable!(),
        }
        current.session.set_sudo_windows(if change == "expiry" {
            vec![]
        } else {
            vec![changed]
        });
        current
            .session
            .add_active_interaction(RuntimeInteraction::new(
                "owner-decision",
                "agent-a",
                RuntimeInteractionKind::Permission,
                RuntimeInteractionLevel::Warning,
                None,
                "Owner decision",
                vec![RuntimeInteractionChoice::new(
                    "cancel", "Cancel", "cancel", None,
                )],
                None,
                None,
                None,
            ));
        assert!(runtime_interactions_changed_event(&current, Some(&previous)).is_some());
        assert!(
            !can_skip_session_snapshot(&current, Some(&previous), true),
            "{change}: an interaction delta cannot replace the changed sudo window"
        );
    }
}

#[test]
fn runtime_interactions_only_changes_can_skip_session_snapshot() {
    let previous = session_snapshot_with_workflow_status(WorkflowRunStatus::Created);
    let mut current = previous.clone();
    current
        .session
        .add_active_interaction(RuntimeInteraction::new(
            "interaction-a",
            "agent-a",
            RuntimeInteractionKind::Permission,
            RuntimeInteractionLevel::Warning,
            Some("Approve command".to_string()),
            "Allow shell command?",
            vec![
                RuntimeInteractionChoice::new(
                    "approve",
                    "Approve",
                    "approve",
                    Some(RuntimeInteractionChoiceStyle::Primary),
                ),
                RuntimeInteractionChoice::new(
                    "deny",
                    "Deny",
                    "deny",
                    Some(RuntimeInteractionChoiceStyle::Danger),
                ),
            ],
            None,
            None,
            None,
        ));

    let event = runtime_interactions_changed_event(&current, Some(&previous))
        .expect("runtime interaction only change should produce event");

    match event {
        KernelEvent::RuntimeInteractionsChanged {
            session_id,
            active_interactions,
        } => {
            assert_eq!(session_id, "session-a");
            assert_eq!(active_interactions.len(), 1);
            let interaction = active_interactions.first().expect("interaction");
            assert_eq!(interaction.id(), "interaction-a");
            assert_eq!(interaction.agent_id(), Some("agent-a"));
            assert_eq!(interaction.kind(), RuntimeInteractionKind::Permission);
            assert_eq!(interaction.choices().len(), 2);
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

fn session_snapshot_with_agent() -> SessionSnapshotProjection {
    let mut session = RuntimeSession::new(
        "session-a",
        Some("Test".to_string()),
        "workspace-a",
        "worktree-a",
        "machine-a",
        "daemon-a",
    );
    session.set_agents(vec![AgentInstance::new(
        "agent-a",
        "agent-a",
        "session-a",
        None,
        "codex",
        Some("model".to_string()),
        None,
        Some("worktree-a".to_string()),
        GridPosition::new(0, 0, 1, 1),
    )]);
    let mut agent_activity = BTreeMap::new();
    agent_activity.insert(
        "agent-a".to_string(),
        AgentRuntimeActivity {
            status: crate::runtime::projection::AgentRuntimeStatus::Idle,
            prompt_status: crate::runtime::projection::AgentPromptRuntimeStatus::None,
            busy: false,
            active_prompt_count: 0,
            queued_prompt_count: 0,
            unread_idle_output: false,
            queued_prompt_controls: BTreeMap::new(),
            active_turn: None,
            last_completed_turn: None,
        },
    );
    SessionSnapshotProjection {
        room_workflows: crate::runtime::projection::RoomWorkflowInventory::project(
            &session,
            crate::session::DEFAULT_LOCAL_USER_ID,
        ),
        metadata: ProjectionMetadata::new(2, 0),
        session,
        provider_run: None,
        agent_activity,
    }
}

fn session_snapshot_with_workflow_status(status: WorkflowRunStatus) -> SessionSnapshotProjection {
    let mut session = RuntimeSession::new(
        "session-a",
        Some("Test".to_string()),
        "workspace-a",
        "worktree-a",
        "machine-a",
        "daemon-a",
    );
    let mut workflow_run = WorkflowRun::new(
        "workflow-run-a",
        "workflow-a",
        "endpoint-a",
        "node-a",
        Some("run it".to_string()),
        None,
        Vec::new(),
        Vec::new(),
    );
    workflow_run.set_status(status);
    session.create_workflow_run(workflow_run);
    SessionSnapshotProjection {
        room_workflows: crate::runtime::projection::RoomWorkflowInventory::project(
            &session,
            crate::session::DEFAULT_LOCAL_USER_ID,
        ),
        metadata: ProjectionMetadata::new(2, 0),
        session,
        provider_run: None,
        agent_activity: BTreeMap::new(),
    }
}

fn waiting_room_snapshot(
    inventory_version: &str,
    sessions: Vec<WaitingRoomPublicSessionSummary>,
) -> WaitingRoomPublicSnapshot {
    WaitingRoomPublicSnapshot {
        provider_accounts: Vec::new(),
        git_credentials: Vec::new(),
        schema_version: 11,
        inventory_version: inventory_version.to_string(),
        structural_version: format!("structural-{inventory_version}"),
        activity_revision: format!("activity-{inventory_version}"),
        generated_at_ms: 100,
        sessions,
        projects: Vec::new(),
        external_provider_sessions: Vec::new(),
        external_provider_sessions_has_more: false,
        external_provider_sessions_next_cursor: None,
        relay_status: RelayStatus {
            runtime_process_identity: None,
            capabilities: Vec::new(),
            configured: false,
            connected: false,
            relay_url: None,
            relay_token_configured: false,
            daemon_id: "daemon-1".to_string(),
            daemon_alias: None,
            machine_id: "machine-1".to_string(),
            machine_alias: None,
        },
        remote_machines: Vec::new(),
        remote_kernels: Vec::new(),
        terminals: Vec::new(),
        launch_target: None,
    }
}

fn provider_account_profile() -> crate::account_profile::ProviderAccountProfile {
    crate::account_profile::ProviderAccountProfile {
        owner_user_id: crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
        provider: "claude".to_string(),
        profile_id: "managed-claude".to_string(),
        label: "Managed Claude".to_string(),
        origin: crate::account_profile::ProviderAccountProfileOrigin::Linked,
        is_default: false,
        auth_state: crate::account_profile::ProviderAccountAuthState::Authenticated,
        identity_summary: None,
        plan: Some("pro".to_string()),
        detected_provider_version: None,
        last_validated_at_ms: Some(100),
        services: Vec::new(),
        usage: crate::account_profile::ProviderAccountUsageSnapshot::unavailable(
            "managed-claude",
            "claude",
        ),
        credential_kind: None,
        credential_kind_not_reported_reason: Some(
            "provider did not report credential type".to_string(),
        ),
        materializations: Vec::new(),
    }
}

fn session_summary(id: &str, last_used_at_ms: u64) -> WaitingRoomPublicSessionSummary {
    WaitingRoomPublicSessionSummary {
        repository_state: None,
        id: id.to_string(),
        project_id: "project-a".to_string(),
        alias: None,
        workspace_id: "workspace-1".to_string(),
        worktree_id: "worktree-1".to_string(),
        workspace_label: None,
        directory: None,
        worktree_label: None,
        workspace_live_sync_mode: None,
        created_at_ms: 1,
        last_used_at_ms: Some(last_used_at_ms),
        last_prompt_sent_at_ms: None,
        status: crate::session::SessionStatus::Active,
        connected_cli_count: 0,
        joined_collaborator_count: 0,
        pending_collaboration_invite_count: 0,
        activity: crate::local::WaitingRoomSessionActivitySummary::default(),
        agents: Vec::new(),
        workflows: Vec::new(),
    }
}

fn project_summary(id: &str, name: &str) -> crate::local::WaitingRoomPublicProjectSummary {
    crate::local::WaitingRoomPublicProjectSummary {
        pinned: false,
        id: id.to_string(),
        owner_user_id: crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
        workspace_id: "workspace-1".to_string(),
        workspace_ids: vec!["workspace-1".to_string()],
        name: name.to_string(),
        kind: crate::session::RuntimeProjectKind::Named,
        status: crate::session::RuntimeProjectStatus::Active,
        created_at_ms: 1,
        updated_at_ms: 1,
        archived_at_ms: None,
        session_count: 0,
        last_session_activity_at_ms: None,
        joined_collaborator_count: 0,
        pending_collaboration_invite_count: 0,
    }
}

#[test]
fn mp08_mp10_terminal_workflow_updates_have_one_authoritative_stream() {
    // MP-08 / MP-10 / MP-11: the behavior landed in 435 and remains required
    // by later protocols; exact shape/version pins live in snapshot tests.
    let previous = session_snapshot_with_workflow_status(WorkflowRunStatus::Running);
    for status in [
        WorkflowRunStatus::Completed,
        WorkflowRunStatus::Failed,
        WorkflowRunStatus::Stopped,
    ] {
        let mut current = previous.clone();
        current
            .session
            .workflow_run_mut("workflow-run-a")
            .unwrap()
            .set_status(status);
        assert!(workflow_run_updated_events(&current, Some(&previous)).is_empty());
        assert!(workflow_run_updated_events(&current, None).is_empty());
    }
}

#[test]
fn user_domain_refusals_protocol_493_snapshot() {
    use crate::error::UserDomainRefusalReason as Reason;
    assert_eq!(
        crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        crate::test_support::expected_relay_peer_protocol_version()
    );
    let values = [
        Reason::NotRequested,
        Reason::NotFocusedAgent,
        Reason::ForeignOwner,
        Reason::StaleEpoch,
        Reason::StaleReference,
        Reason::NotGranted,
        Reason::SensitiveRequiresFocus,
    ];
    let actual: Vec<_> = values.into_iter().map(|reason|serde_json::json!({"reason":reason,"error":map_kernel_error(&DaemonError::UserDomainRefused {reason})})).collect();
    use sha2::{Digest, Sha256};
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("user-domain-refusals-443.json"))
        ),
        "a97facd41c511caaf1fbbecfc7f33eacb76dd49800b09187968e88ee28a7eb52"
    );
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("user-domain-refusals-443.json")).unwrap();
    assert_eq!(serde_json::json!(actual), expected);
    // Neither unknown controller text nor a generic local error can acquire authority.
    let generic = map_kernel_error(&DaemonError::LocalTransport {
        operation: "kernel_browser",
        message: "user_domain_not_granted".into(),
    });
    assert_eq!(generic.code, "local_transport_error");
    assert!(generic.retryable);
}

// MP-08 / MP-09 / MP-10 / MP-11 A12-S02: compact task state must reach every terminal.
#[test]
fn a12_task_change_is_a_compact_projection_event() {
    let previous = session_snapshot_with_agent();
    let mut current = previous.clone();
    let task = serde_json::from_value(serde_json::json!({
        "task_id":"task", "room_id":current.session.id(), "owner_user_id":"local",
        "agent_id":"agent-a", "prompt_id":"prompt", "provider_run_id":null,
        "revision":2, "blocked_revision":0, "state":"working", "reason":"work",
        "obligations":[], "wait":null, "last_progress_at_ms":1, "progress_sequence":0,
        "no_progress_wakes":0, "correction_used":false, "pending_prompt_id":null
    }))
    .unwrap();
    current.session.set_agent_tasks(vec![task]);
    let events = session_projection_changed_events(&current, Some(&previous));
    let payloads: Vec<_> = events
        .iter()
        .map(|e| serde_json::to_value(e).unwrap())
        .collect();
    assert!(
        payloads.iter().any(|e| e["event"] == "agent_task_updated"),
        "a changed task cannot depend on a full room snapshot"
    );
    assert!(payloads.iter().all(|e| e.get("session").is_none()));
    assert!(can_skip_session_snapshot(&current, Some(&previous), true));
    assert!(session_projection_changed_events(&current, Some(&current)).is_empty());
    assert!(session_projection_changed_events(&current, None).is_empty());
}

// MP-08 / MP-10 A12-S02: both subscription writers use this shared selection.
fn review971c_task_snapshot() -> SessionSnapshotProjection {
    let mut snapshot = session_snapshot_with_agent();
    let task = serde_json::from_value(serde_json::json!({
        "task_id":"task-a", "room_id":"session-a", "owner_user_id":"local",
        "agent_id":"agent-a", "prompt_id":"prompt-a", "provider_run_id":null,
        "revision":1, "blocked_revision":0, "state":"working", "reason":"work",
        "obligations":[], "wait":null, "last_progress_at_ms":1, "progress_sequence":0,
        "no_progress_wakes":0, "correction_used":false, "pending_prompt_id":null
    }))
    .unwrap();
    snapshot.session.set_agent_tasks(vec![task]);
    snapshot
}

fn review971c_assert_full_snapshot(change: impl FnOnce(&mut SessionSnapshotProjection)) {
    let previous = review971c_task_snapshot();
    let mut current = previous.clone();
    let mut tasks = current.session.agent_tasks().to_vec();
    tasks[0].revision += 1;
    current.session.set_agent_tasks(tasks);
    change(&mut current);
    let events = session_projection_changed_events(&current, Some(&previous));
    assert!(events
        .iter()
        .any(|event| matches!(event, KernelEvent::AgentTaskUpdated { .. })));
    assert!(
        !can_skip_session_snapshot(&current, Some(&previous), !events.is_empty()),
        "a task event must not hide other session changes before advancing the subscription baseline"
    );
}

#[test]
fn review971c_task_and_prompt_queue_keep_session_snapshot() {
    review971c_assert_full_snapshot(|current| {
        current.session.mirror_agent_prompt_state(
            "agent-a",
            None,
            std::collections::VecDeque::from([PromptQueueItem::new(
                "queued",
                "attachment-a",
                "agent-a",
                "next",
                PromptStatus::Queued,
            )]),
        );
        assert_eq!(
            current
                .session
                .queued_prompts_for_agent("agent-a")
                .unwrap()
                .len(),
            1
        );
    });
}

#[test]
fn review971c_task_and_activity_keep_session_snapshot() {
    review971c_assert_full_snapshot(|current| {
        current.agent_activity.get_mut("agent-a").unwrap().busy = true;
    });
}

#[test]
fn review971c_task_and_provider_run_keep_session_snapshot() {
    review971c_assert_full_snapshot(|current| {
        current.provider_run = Some(RuntimeProviderRun::from_control_capability_inference(
            "run-a",
            "session-a".into(),
            Some("agent-a".into()),
            "codex".into(),
        ));
    });
}

#[test]
fn review971c_task_and_metadata_keep_session_snapshot() {
    review971c_assert_full_snapshot(|current| current.session.set_alias(Some("renamed".into())));
}

#[test]
fn review971c_task_and_new_agent_keep_session_snapshot() {
    review971c_assert_full_snapshot(|current| {
        let mut agents = current.session.agents().to_vec();
        agents.push(AgentInstance::new(
            "agent-b",
            "second",
            "session-a",
            None,
            "codex",
            None,
            None,
            Some("worktree-a".into()),
            GridPosition::new(0, 1, 1, 1),
        ));
        current.session.set_agents(agents);
        let mut tasks = current.session.agent_tasks().to_vec();
        let mut task = tasks[0].clone();
        task.task_id = "task-b".into();
        task.agent_id = "agent-b".into();
        tasks.push(task);
        current.session.set_agent_tasks(tasks);
    });
}

#[test]
fn review971c_task_and_interaction_keep_session_snapshot() {
    review971c_assert_full_snapshot(|current| {
        current
            .session
            .add_active_interaction(RuntimeInteraction::new(
                "decision",
                "agent-a",
                RuntimeInteractionKind::Permission,
                RuntimeInteractionLevel::Warning,
                None,
                "Allow?",
                vec![RuntimeInteractionChoice::new("deny", "Deny", "deny", None)],
                None,
                None,
                None,
            ));
    });
}

#[test]
fn review971c_removed_task_requires_session_snapshot() {
    let previous = review971c_task_snapshot();
    let mut current = previous.clone();
    let mut task = current.session.agent_tasks()[0].clone();
    task.task_id = "replacement".into();
    current.session.set_agent_tasks(vec![task]);
    let events = session_projection_changed_events(&current, Some(&previous));
    assert_eq!(events.len(), 1);
    assert!(!can_skip_session_snapshot(&current, Some(&previous), true));
}

#[test]
fn room_workflow_inventory_delta_uses_semantic_revision_and_explicit_empty() {
    let mut previous = session_snapshot_with_agent();
    previous
        .session
        .create_workflow(crate::session::WorkflowDefinition::new("blank", None));
    previous.room_workflows =
        crate::runtime::projection::RoomWorkflowInventory::project(&previous.session, "local");
    assert!(super::room_workflows_changed_event(&previous, None).is_none());
    assert!(super::room_workflows_changed_event(&previous, Some(&previous)).is_none());
    let mut current = previous.clone();
    current.session.remove_workflow("blank");
    current.room_workflows =
        crate::runtime::projection::RoomWorkflowInventory::project(&current.session, "local");
    let event = super::room_workflows_changed_event(&current, Some(&previous)).unwrap();
    assert_eq!(super::event_session_id(&event), Some("session-a"));
    assert_eq!(super::kernel_event_name(&event), "room_workflows_changed");
    match event {
        KernelEvent::RoomWorkflowsChanged { inventory } => assert_eq!(inventory.workflow_count, 0),
        other => panic!("unexpected event: {other:?}"),
    }
}

// MP-08 / MP-10 / MP-11: do not turn a missing account into a retry loop.
#[test]
fn missing_account_resolution_is_definitive_on_kernel_transport() {
    let error = DaemonError::LocalTransport {
        operation: "resolve account profile",
        message: "account is absent on this kernel; copy login or sign in".into(),
    };
    let mapped = map_kernel_error(&error);
    assert_eq!(mapped.code, "local_transport_error");
    assert!(!mapped.retryable);
    assert_eq!(mapped.message, error.to_string());
    let transient = DaemonError::LocalTransport {
        operation: "read account profile registry",
        message: "temporary read failure".into(),
    };
    assert!(map_kernel_error(&transient).retryable);
}

// MP-08 / MP-10 / MP-11: a missing Project is a permanent lookup miss.
#[test]
fn env_ux01_project_lookup_absence_is_not_retryable() {
    let error = crate::error::DaemonError::LocalTransport {
        operation: "project.get",
        message: "project was not found".into(),
    };
    let mapped = super::map_kernel_error(&error);
    assert_eq!(mapped.code, "local_transport_error");
    assert!(!mapped.retryable);
}

// MP-08 / MP-10 / MP-11: only definitive Project misses lose retryability.
#[test]
fn env_ux01_unrelated_transport_failures_remain_retryable() {
    let error = crate::error::DaemonError::LocalTransport {
        operation: "project.environment.get",
        message: "connection interrupted".into(),
    };
    assert!(super::map_kernel_error(&error).retryable);
}

fn ledger_task(
    task_id: &str,
    state: &str,
    revision: u64,
) -> crate::durable_state::agent_lifecycle::AgentTaskExecution {
    serde_json::from_value(serde_json::json!({
        "task_id": task_id, "room_id": "session-a", "agent_id": "agent-a", "prompt_id": task_id,
        "provider_run_id": null, "revision": revision, "state": state, "reason": "",
        "obligations": [], "wait": null, "last_progress_at_ms": 1, "progress_sequence": 0,
        "no_progress_wakes": 0, "correction_used": false, "pending_prompt_id": null,
    }))
    .unwrap()
}

fn ledger_wake(state: &str) -> crate::durable_state::agent_lifecycle::AgentWake {
    serde_json::from_value(serde_json::json!({
        "id": "wake-a", "task_id": "task-a", "room_id": "session-a", "agent_id": "agent-a",
        "registration_id": "reg-a", "kind": "timer", "label": "tick", "state": state,
        "created_at_ms": 1, "verified_at_ms": 1, "next_due_ms": 9, "interval_ms": null,
        "command": [], "match_text": null, "matched_at_ms": null, "pid": null, "exit_code": null,
            "fire_count": 0, "missed_fires": 0, "last_fired_at_ms": null, "last_sequence": null,
        "last_delivery": null, "last_delivered_at_ms": null, "last_acknowledged_at_ms": null,
        "alerted_sequence": null,
    }))
    .unwrap()
}

#[test]
fn a_changed_agent_task_is_one_small_event_and_skips_the_full_snapshot() {
    let mut previous = session_snapshot_with_workflow_status(WorkflowRunStatus::Created);
    previous.session.set_agent_tasks(vec![
        ledger_task("task-a", "working", 1),
        ledger_task("task-b", "done", 2),
    ]);
    let mut current = previous.clone();
    current.session.set_agent_tasks(vec![
        ledger_task("task-a", "waiting", 2),
        ledger_task("task-b", "done", 2),
    ]);

    let events = session_projection_changed_events(&current, Some(&previous));

    assert!(can_skip_session_snapshot(&current, Some(&previous), true));
    match events.as_slice() {
        [KernelEvent::AgentTaskUpdated { session_id, task }] => {
            assert_eq!(
                (session_id.as_str(), task.task_id.as_str()),
                ("session-a", "task-a")
            );
            assert_eq!(task.revision, 2);
        }
        other => panic!("expected one task event, got {other:?}"),
    }
    let wire = serde_json::to_value(&events[0]).unwrap();
    assert_eq!(wire["event"], "agent_task_updated");
    assert_eq!(wire["task"]["state"], "waiting");
    assert!(session_projection_changed_events(&current, None).is_empty());
}

#[test]
fn a_wake_change_or_a_vanished_task_keeps_the_full_snapshot() {
    let mut previous = session_snapshot_with_workflow_status(WorkflowRunStatus::Created);
    previous
        .session
        .set_agent_tasks(vec![ledger_task("task-a", "working", 1)]);
    previous
        .session
        .set_agent_wakes(vec![ledger_wake("scheduled")]);

    let mut woken = previous.clone();
    woken.session.set_agent_wakes(vec![ledger_wake("fired")]);
    assert!(!can_skip_session_snapshot(&woken, Some(&previous), true));

    let mut aged_out = previous.clone();
    aged_out.session.set_agent_tasks(Vec::new());
    assert!(!can_skip_session_snapshot(&aged_out, Some(&previous), true));

    assert!(can_skip_session_snapshot(
        &previous.clone(),
        Some(&previous),
        true
    ));
    assert!(!can_skip_session_snapshot(&previous, None, true));
}

// MP-08/MP-10/MP-11: a task delta covers no concurrent prompt or provider state.
#[test]
fn task_deltas_keep_concurrent_session_changes_in_the_snapshot() {
    let mut previous = session_snapshot_with_agent();
    previous
        .session
        .set_agent_tasks(vec![ledger_task("task-a", "working", 1)]);
    let mut task_only = previous.clone();
    task_only
        .session
        .set_agent_tasks(vec![ledger_task("task-a", "done", 2)]);

    let mut prompt_change = task_only.clone();
    let mut session_value = serde_json::to_value(&prompt_change.session).unwrap();
    let next_prompt = PromptQueueItem::new(
        "next-prompt",
        "attachment-a",
        "agent-a",
        "next",
        PromptStatus::Running,
    );
    session_value["prompt_states"] = serde_json::json!({"agent-a": {
        "active_prompt": next_prompt, "queued_prompts": [],
    }});
    session_value["active_prompt"] = serde_json::to_value(next_prompt).unwrap();
    session_value["scheduler_state"] = serde_json::json!("Running");
    prompt_change.session = serde_json::from_value(session_value).unwrap();

    let mut provider_change = task_only.clone();
    provider_change.provider_run = Some(RuntimeProviderRun::from_control_capability_inference(
        "provider-run-next",
        "session-a".into(),
        Some("agent-a".into()),
        "codex".into(),
    ));
    let mut metadata_change = task_only.clone();
    metadata_change.session.set_alias(Some("renamed".into()));

    for current in [provider_change, prompt_change, metadata_change] {
        assert!(
            current.session != previous.session || current.provider_run != previous.provider_run,
            "each fixture must contain a concurrent change"
        );
        assert!(session_projection_changed_events(&current, Some(&previous))
            .iter()
            .any(|event| matches!(event, KernelEvent::AgentTaskUpdated { .. })));
        assert!(
            !can_skip_session_snapshot(&current, Some(&previous), true),
            "a task event must not suppress concurrent session changes"
        );
    }
    assert!(can_skip_session_snapshot(&task_only, Some(&previous), true));
}
