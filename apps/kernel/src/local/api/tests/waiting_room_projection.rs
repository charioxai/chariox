use super::*;

#[test]
fn waiting_room_inventory_includes_session_workspace_display_labels() {
    let workspace_root = std::env::temp_dir().join("arroba-waiting-room-session-label-test");
    let _ = std::fs::remove_dir_all(&workspace_root);
    std::fs::create_dir_all(&workspace_root).expect("workspace should exist");
    std::process::Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(&workspace_root)
        .output()
        .expect("git init should work");
    std::process::Command::new("git")
        .args([
            "remote",
            "add",
            "origin",
            "git@github.com:mgutierrez09/arroba.git",
        ])
        .current_dir(&workspace_root)
        .output()
        .expect("git remote add should work");

    let harness = LocalRouterTestHarness::new();
    let created = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                workspace_root.display().to_string(),
                workspace_root.display().to_string(),
            )
            .with_alias("main"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected response: {other:?}"),
    };

    let snapshot = match harness
        .dispatch(LocalDaemonRequest::GetWaitingRoomInventory(
            GetWaitingRoomInventoryRequest,
        ))
        .expect("waiting room inventory should succeed")
    {
        LocalDaemonResponse::WaitingRoomInventory { snapshot } => snapshot,
        other => panic!("unexpected response: {other:?}"),
    };

    let session = snapshot
        .sessions
        .iter()
        .find(|candidate| candidate.id == created.id())
        .expect("created session should be in waiting-room inventory");
    assert_eq!(
        session.workspace_label.as_deref(),
        Some("mgutierrez09/arroba")
    );
    let workspace_path = workspace_root.display().to_string();
    assert_eq!(session.directory.as_deref(), Some(workspace_path.as_str()));
}

#[test]
fn waiting_room_public_snapshot_omits_private_runtime_session_payload() {
    let harness = LocalRouterTestHarness::new();
    let (created, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                "/tmp/arroba-public-snapshot-workspace",
                "/tmp/arroba-public-snapshot-worktree",
            ),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        other => panic!("unexpected response: {other:?}"),
    };

    let snapshot = match harness
        .dispatch(LocalDaemonRequest::GetWaitingRoomPublicSnapshot(
            GetWaitingRoomPublicSnapshotRequest,
        ))
        .expect("waiting room public snapshot should succeed")
    {
        LocalDaemonResponse::WaitingRoomPublicSnapshot { snapshot } => snapshot,
        other => panic!("unexpected response: {other:?}"),
    };

    assert_eq!(snapshot.schema_version, 5);
    assert!(snapshot.generated_at_ms > 0);
    let session = snapshot
        .sessions
        .iter()
        .find(|candidate| candidate.id == created.id())
        .expect("created session should be in public snapshot");
    assert_eq!(
        session.workspace_id,
        "/tmp/arroba-public-snapshot-workspace"
    );
    assert_eq!(session.worktree_id, "/tmp/arroba-public-snapshot-worktree");
    assert_eq!(session.connected_cli_count, 0);
    assert_eq!(session.activity.agent_count, 1);
    assert_eq!(session.activity.working_agent_count, 0);
    assert_eq!(session.activity.active_prompt_count, 0);
    assert_eq!(session.activity.queued_prompt_count, 0);
    assert_eq!(session.activity.error_agent_count, 0);
    assert_eq!(session.agents.len(), 1);
    assert_eq!(session.agents[0].id, agent.id());
    assert_eq!(session.agents[0].agent_ref, agent.agent_ref());
    assert_eq!(session.agents[0].provider, agent.primary_provider());
    assert_eq!(session.agents[0].worktree_id, session.worktree_id);
    assert!(session.workflows.is_empty());

    let serialized =
        serde_json::to_value(session).expect("public session summary should serialize");
    assert!(
        serialized.get("attachment_ids").is_none(),
        "public summary must not expose CLI attachment ids"
    );
    assert!(serialized.pointer("/agents/0/id").is_some());
    assert!(serialized.pointer("/agents/0/agent_ref").is_some());
    assert!(serialized.pointer("/agents/0/provider").is_some());
    assert_eq!(
        serialized.pointer("/agents/0/mode"),
        Some(&serde_json::Value::String("build".to_string()))
    );
    assert!(serialized
        .pointer("/agents/0/provider_resume_state")
        .is_none());
    assert!(serialized.pointer("/agents/0/active_prompt").is_none());
    assert!(
        serialized.get("active_prompt").is_none(),
        "public summary must not expose active prompt internals"
    );
}

#[test]
fn waiting_room_agent_workspace_update_drill_updates_public_projection() {
    let harness = LocalRouterTestHarness::new();
    let (session, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("/tmp/arroba-workspace-a", "/tmp/arroba-workspace-a"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        other => panic!("unexpected response: {other:?}"),
    };

    harness
        .dispatch(LocalDaemonRequest::UpdateAgentConfig(
            UpdateAgentConfigRequest {
                session_id: session.id().to_string(),
                agent_id: agent.id().to_string(),
                execution_mode: None,
                clear_execution_mode: false,
                permission_level: None,
                clear_permission_level: false,
                workspace_id: Some("/tmp/arroba-workspace-b".to_string()),
                clear_workspace_id: false,
                worktree_id: Some("/tmp/arroba-workspace-b-feature".to_string()),
                clear_worktree_id: false,
            },
        ))
        .expect("agent workspace update should succeed");

    let snapshot = match harness
        .dispatch(LocalDaemonRequest::GetWaitingRoomPublicSnapshot(
            GetWaitingRoomPublicSnapshotRequest,
        ))
        .expect("waiting room public snapshot should succeed")
    {
        LocalDaemonResponse::WaitingRoomPublicSnapshot { snapshot } => snapshot,
        other => panic!("unexpected response: {other:?}"),
    };
    let public_agent = snapshot
        .sessions
        .iter()
        .find(|candidate| candidate.id == session.id())
        .and_then(|session| {
            session
                .agents
                .iter()
                .find(|candidate| candidate.id == agent.id())
        })
        .expect("updated agent should be in public snapshot");

    assert_eq!(public_agent.workspace_id, "/tmp/arroba-workspace-b");
    assert_eq!(
        public_agent.directory.as_deref(),
        Some("/tmp/arroba-workspace-b")
    );
    assert_eq!(public_agent.worktree_id, "/tmp/arroba-workspace-b-feature");
}

#[test]
fn waiting_room_public_snapshot_includes_public_workflow_summaries() {
    let harness = LocalRouterTestHarness::new();
    let (session, first_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                "/tmp/arroba-public-workflow-workspace",
                "/tmp/arroba-public-workflow-worktree",
            ),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        other => panic!("unexpected response: {other:?}"),
    };
    let second_agent = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("second".to_string()),
            provider: Some("dev-stub".to_string()),
            model: Some("model-b".to_string()),
            effort: Some("low".to_string()),
            execution_mode: Some(crate::provider::AgentExecutionMode::Plan),
            permission_level: Some(crate::provider::AgentPermissionLevel::Required),
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
        }))
        .expect("second agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        other => panic!("unexpected response: {other:?}"),
    };
    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("review".to_string()),
        }))
        .expect("workflow should create")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        other => panic!("unexpected response: {other:?}"),
    };
    let first_node = match harness
        .dispatch(LocalDaemonRequest::AddWorkflowNode(
            AddWorkflowNodeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                agent_id: first_agent.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("first workflow node should add")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        other => panic!("unexpected response: {other:?}"),
    };
    let second_node = match harness
        .dispatch(LocalDaemonRequest::AddWorkflowNode(
            AddWorkflowNodeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                agent_id: second_agent.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("second workflow node should add")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        other => panic!("unexpected response: {other:?}"),
    };
    harness
        .dispatch(LocalDaemonRequest::AddWorkflowEdge(
            AddWorkflowEdgeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                from_node_id: first_node.id().to_string(),
                to_node_id: second_node.id().to_string(),
                handoff_schema_ref: None,
                validation_policy: None,
                expected_workflow_revision: None,
                source_side: None,
                target_side: None,
            },
        ))
        .expect("workflow edge should add");
    harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: first_node.id().to_string(),
                alias: Some("start".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow endpoint should create");

    let snapshot = match harness
        .dispatch(LocalDaemonRequest::GetWaitingRoomPublicSnapshot(
            GetWaitingRoomPublicSnapshotRequest,
        ))
        .expect("waiting room public snapshot should succeed")
    {
        LocalDaemonResponse::WaitingRoomPublicSnapshot { snapshot } => snapshot,
        other => panic!("unexpected response: {other:?}"),
    };
    let summary = snapshot
        .sessions
        .iter()
        .find(|candidate| candidate.id == session.id())
        .expect("created session should be in public snapshot");
    assert_eq!(summary.agents.len(), 2);
    assert_eq!(summary.agents[0].id, first_agent.id());
    assert_eq!(summary.agents[0].agent_ref, first_agent.agent_ref());
    assert_eq!(summary.agents[1].id, second_agent.id());
    assert_eq!(summary.agents[1].agent_ref, second_agent.agent_ref());
    assert_eq!(summary.agents[1].alias.as_deref(), Some("second"));
    assert_eq!(summary.agents[1].provider, "dev-stub");
    assert_eq!(summary.agents[1].model.as_deref(), Some("model-b"));
    assert_eq!(summary.agents[1].variant.as_deref(), Some("low"));
    assert_eq!(summary.agents[1].mode, "plan");
    assert_eq!(summary.agents[1].permission.as_deref(), Some("required"));
    assert_eq!(summary.workflows.len(), 1);
    assert_eq!(summary.workflows[0].id, workflow.id());
    assert_eq!(summary.workflows[0].alias.as_deref(), Some("review"));
    assert_eq!(summary.workflows[0].nodes.len(), 2);
    assert_eq!(summary.workflows[0].edges.len(), 1);
    assert_eq!(summary.workflows[0].endpoints.len(), 1);

    let serialized =
        serde_json::to_value(summary).expect("public session summary should serialize");
    assert!(serialized
        .pointer("/workflows/0/nodes/0/agent_id")
        .is_some());
    assert!(serialized
        .pointer("/workflows/0/edges/0/from_node_id")
        .is_some());
    assert!(serialized
        .pointer("/workflows/0/endpoints/0/entry_node_id")
        .is_some());
    assert!(
        serialized
            .pointer("/workflows/0/nodes/0/instructions")
            .is_none(),
        "public workflow summary must not expose private node instructions"
    );
}

#[test]
fn waiting_room_public_snapshot_includes_public_session_activity_counts() {
    let harness = LocalRouterTestHarness::new();
    let (session, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                "/tmp/arroba-public-activity-workspace",
                "/tmp/arroba-public-activity-worktree",
            ),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        other => panic!("unexpected response: {other:?}"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-activity".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        other => panic!("unexpected response: {other:?}"),
    };
    harness.with_app_mut(|app| {
        app.launch_provider(
            LaunchProviderRequest::new(session.id(), "dev-stub", "default", "default", "model")
                .with_agent_id(agent.id()),
        )
        .expect("provider launch should succeed");
    });
    let submitted = harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(agent.id().to_string()),
            prompt: "keep working\n".to_string(),
            attachments: Vec::new(),
        }))
        .expect("prompt should submit");
    assert!(
        matches!(
            submitted,
            LocalDaemonResponse::PromptSubmitted {
                outcome: PromptSubmissionOutcome::Started { .. },
                ..
            }
        ),
        "prompt should start immediately"
    );

    let snapshot = match harness
        .dispatch(LocalDaemonRequest::GetWaitingRoomPublicSnapshot(
            GetWaitingRoomPublicSnapshotRequest,
        ))
        .expect("waiting room public snapshot should succeed")
    {
        LocalDaemonResponse::WaitingRoomPublicSnapshot { snapshot } => snapshot,
        other => panic!("unexpected response: {other:?}"),
    };
    let summary = snapshot
        .sessions
        .iter()
        .find(|candidate| candidate.id == session.id())
        .expect("created session should be in public snapshot");
    assert_eq!(summary.activity.agent_count, 1);
    assert_eq!(summary.activity.working_agent_count, 1);
    assert_eq!(summary.activity.active_prompt_count, 1);
    assert_eq!(summary.activity.queued_prompt_count, 0);
    assert_eq!(summary.activity.error_agent_count, 0);

    let serialized =
        serde_json::to_value(summary).expect("public session summary should serialize");
    assert!(
        serialized
            .pointer("/activity/working_agent_count")
            .is_some(),
        "public summary should expose aggregate activity"
    );
    assert!(
        serialized.get("active_prompt").is_none(),
        "public summary must not expose active prompt internals"
    );
}

#[test]
fn terminal_pairing_link_adds_terminal_to_waiting_room_inventory() {
    let mut config = DaemonConfig::for_tests();
    config.relay_url = Some("ws://relay.local".to_string());
    config.relay_token = Some("relay-token".to_string());
    let harness = LocalRouterTestHarness::with_config(config);

    let pairing = match harness
        .dispatch(LocalDaemonRequest::CreateTerminalPairingLink(
            CreateTerminalPairingLinkRequest {
                terminal_type: Some(TerminalType::Web),
                alias: Some("browser".to_string()),
                expires_in_ms: Some(60_000),
            },
        ))
        .expect("terminal pairing link should be created")
    {
        LocalDaemonResponse::TerminalPairingLinkCreated { pairing } => pairing,
        other => panic!("unexpected response: {other:?}"),
    };

    assert!(pairing.pairing_link.starts_with("arroba-terminal-pair-v1."));
    assert_eq!(pairing.terminal_type, TerminalType::Web);
    assert_eq!(pairing.relay_url, "ws://relay.local");
    assert_eq!(pairing.pairing_code.len(), "ABCD-EFGH".len());

    let snapshot = match harness
        .dispatch(LocalDaemonRequest::GetWaitingRoomInventory(
            GetWaitingRoomInventoryRequest,
        ))
        .expect("waiting room inventory should succeed")
    {
        LocalDaemonResponse::WaitingRoomInventory { snapshot } => snapshot,
        other => panic!("unexpected response: {other:?}"),
    };
    let terminal = snapshot
        .terminals
        .iter()
        .find(|terminal| terminal.terminal_id == pairing.terminal_id)
        .expect("paired terminal should be listed");
    assert_eq!(terminal.terminal_type, TerminalType::Web);

    let joined = match harness
        .dispatch(LocalDaemonRequest::JoinTerminalPairingLink(
            JoinTerminalPairingLinkRequest {
                pairing_link: pairing.pairing_link,
                terminal_id: None,
                terminal_type: None,
                alias: Some("browser paired".to_string()),
            },
        ))
        .expect("terminal pairing link should redeem")
    {
        LocalDaemonResponse::TerminalPairingLinkJoined { terminal, .. } => terminal,
        other => panic!("unexpected response: {other:?}"),
    };
    assert_eq!(joined.terminal_id, pairing.terminal_id);
    assert_eq!(joined.terminal_type, TerminalType::Web);
}
