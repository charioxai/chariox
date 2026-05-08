use std::collections::BTreeMap;
use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use crate::app::provider_output::{
    pump_active_prompt_outputs, ProviderOutputPump, ProviderOutputPumpRequest,
};
use crate::attachment::ClientCapabilityLevel;
use crate::local::test_support::LocalRouterTestHarness;
use crate::provider::{
    LaunchProviderRequest, ProviderPromptChunk, ProviderPromptSignalBatch, ProviderRunTokenUsage,
    RuntimeProviderRun,
};
use crate::session::{
    CreateSessionRequest, PromptSubmissionOutcome, WorkflowHandoffPayload, WorkflowNodeRunStatus,
    WorkflowOutputValidationPolicy, WorkflowTurnRuntimeState,
};
use crate::terminal::TerminalOutputKind;
use crate::{DaemonApp, DaemonConfig, DaemonError};
use arroba_relay::protocol::{RelayKernelPresence, RelayMachinePresence};
use sha2::{Digest, Sha256};

use super::{
    AckWorkflowTurnRequest, AddWorkflowEdgeRequest, AddWorkflowNodeRequest, AliasAgentRequest,
    AliasSessionRequest, AliasWorkflowEndpointRequest, AliasWorkflowRequest,
    AttachToSessionRequest, AttachWorkspaceLinkRequest, CancelActivePromptRequest,
    CancelWorkflowRunRequest, CaptureScreenshotCapabilityRequest, CommitWorkspaceChangesRequest,
    CompletePromptRequest, CreateSessionInviteRequest, CreateTerminalPairingLinkRequest,
    CreateWorkflowEndpointRequest, CreateWorkflowRequest, CreateWorkspaceLinkRequest,
    CycleAgentFocusRequest, DeleteSessionRequest, DetachFromSessionRequest,
    DetachWorkspaceLinkRequest, EditFileCapabilityRequest, EndSessionRequest, FocusAgentRequest,
    GetDaemonHealthRequest, GetSessionStateRequest, GetWaitingRoomInventoryRequest,
    GetWaitingRoomPublicSnapshotRequest, GetWorkflowRunRequest, GetWorkspaceFileContentRequest,
    GetWorkspaceGitOverviewRequest, InspectGitCapabilityRequest, InvokeWorkflowEndpointRequest,
    JoinSessionInviteRequest, JoinTerminalPairingLinkRequest, LaunchProviderRunRequest,
    ListAgentsRequest, ListRemoteMachineKernelsRequest, ListRemoteMachinesRequest,
    ListSessionMembersRequest, ListSessionsRequest, ListWorkflowRunsRequest, ListWorkflowsRequest,
    ListWorkspaceFilesRequest, ListWorkspaceLinksRequest, LocalDaemonRequest, LocalDaemonResponse,
    PollRuntimeNoticesRequest, PushWorkspaceBranchRequest, ReadDirectoryTreeCapabilityRequest,
    ReadFileCapabilityRequest, RemoveWorkflowEdgeRequest, RemoveWorkflowNodeRequest,
    ResolveSessionRequest, ResolveWorkflowRequest, ResumeWorkflowRunRequest,
    RevokeSessionInviteRequest, RunShellCapabilityRequest, ShowWorkspaceLinkRequest,
    SpawnAgentRequest, StoreTransferredFileCapabilityRequest, SubmitPromptRequest, TerminalType,
    UpdateAgentProfileRequest, UpdateSessionConfigRequest, UpdateWorkflowCanvasLayoutRequest,
    UpdateWorkflowNodeInstructionsRequest, WorkspaceFileContent, WorkspaceRepoFileEntry,
    WorkspaceRepoFileListing,
    LOCAL_DAEMON_PROTOCOL_VERSION,
};

#[test]
fn local_daemon_protocol_provider_run_usage_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 13);

    let mut provider_run = RuntimeProviderRun::from_control_capability_inference(
        "provider-run-1",
        "session-1".to_string(),
        Some("agent-1".to_string()),
        "codex".to_string(),
    );
    provider_run.set_usage(ProviderRunTokenUsage {
        total_tokens: Some(42_100),
        last_tokens: Some(8_900),
        context_tokens: Some(8_900),
        context_window: Some(128_000),
    });

    let response = LocalDaemonResponse::ProviderRun { provider_run };
    let snapshot = serde_json::to_value(response).expect("response should serialize");

    assert_eq!(
        snapshot.pointer("/ProviderRun/provider_run/usage/total_tokens"),
        Some(&serde_json::json!(42_100))
    );
    assert_eq!(
        snapshot.pointer("/ProviderRun/provider_run/usage/last_tokens"),
        Some(&serde_json::json!(8_900))
    );
    assert_eq!(
        snapshot.pointer("/ProviderRun/provider_run/usage/context_tokens"),
        Some(&serde_json::json!(8_900))
    );
    assert_eq!(
        snapshot.pointer("/ProviderRun/provider_run/usage/context_window"),
        Some(&serde_json::json!(128_000))
    );

    let usage_snapshot = snapshot
        .pointer("/ProviderRun/provider_run/usage")
        .expect("usage should serialize");
    let serialized = serde_json::to_string(usage_snapshot).expect("usage snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "bb7a57b01ed4658729be85e00a5e5ae23f877b8a19973ac9f007c01d45ca1335"
    );

    let listing = WorkspaceRepoFileListing {
        workspace_id: "workspace-1".to_string(),
        worktree_id: "worktree-1".to_string(),
        path_prefix: "src".to_string(),
        compare_ref: "origin/main".to_string(),
        total_entries: 2,
        truncated: true,
        entries: vec![WorkspaceRepoFileEntry {
            path: "src/app.rs".to_string(),
            name: "app.rs".to_string(),
            kind: "file".to_string(),
            changed: true,
            status: Some("modified".to_string()),
            additions: 3,
            deletions: 1,
        }],
        generated_at_ms: 1234,
    };
    let listing_snapshot =
        serde_json::to_value(LocalDaemonResponse::WorkspaceFilesListed { listing })
            .expect("workspace listing should serialize");
    let listing_payload = listing_snapshot
        .pointer("/WorkspaceFilesListed/listing")
        .expect("workspace listing payload should serialize");
    assert_eq!(
        listing_payload.pointer("/compare_ref"),
        Some(&serde_json::json!("origin/main"))
    );
    assert_eq!(
        listing_payload.pointer("/total_entries"),
        Some(&serde_json::json!(2))
    );
    assert_eq!(
        listing_payload.pointer("/truncated"),
        Some(&serde_json::json!(true))
    );
    let serialized =
        serde_json::to_string(listing_payload).expect("workspace listing snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "d53bd6870d6a9236c231fcfaafe4c99d893029c6fed44efd31642cdc57adc918"
    );

    let layout_request =
        serde_json::to_value(LocalDaemonRequest::UpdateWorkflowCanvasLayout(
            super::UpdateWorkflowCanvasLayoutRequest {
                session_id: "session-1".to_string(),
                workflow_ref: "workflow-1".to_string(),
                base_layout_revision: Some(7),
                patches: vec![
                    crate::session::WorkflowCanvasLayoutPatch::NodePosition {
                        node_id: "node-1".to_string(),
                        x: 120,
                        y: 80,
                    },
                    crate::session::WorkflowCanvasLayoutPatch::EndpointPosition {
                        endpoint_id: "endpoint-1".to_string(),
                        x: 140,
                        y: 42,
                    },
                    crate::session::WorkflowCanvasLayoutPatch::EdgeWaypoints {
                        edge_id: "edge-1".to_string(),
                        waypoints: vec![crate::session::WorkflowCanvasPoint {
                            x: 220,
                            y: 80,
                        }],
                    },
                ],
            },
        ))
        .expect("layout request should serialize");
    let layout_payload = layout_request
        .pointer("/UpdateWorkflowCanvasLayout")
        .expect("layout request payload should serialize");
    assert_eq!(
        layout_payload.pointer("/patches/0/kind"),
        Some(&serde_json::json!("node_position"))
    );
    let serialized =
        serde_json::to_string(layout_payload).expect("layout request snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "97cd77e97437355506f7025c38c0741de615733f823224cc850cb9d0e2885bfe"
    );

    let content = WorkspaceFileContent {
        workspace_id: "workspace-1".to_string(),
        worktree_id: "worktree-1".to_string(),
        path: "src/app.rs".to_string(),
        name: "app.rs".to_string(),
        language: "rust".to_string(),
        mime: "text/x-rust".to_string(),
        encoding: "utf-8".to_string(),
        content_text: Some("fn main() {}\n".to_string()),
        content_base64: None,
        size_bytes: 13,
        mtime_ms: 1235,
        fingerprint: "fingerprint-1".to_string(),
        sha256: Some("sha256-1".to_string()),
        truncated: false,
        status: Some("modified".to_string()),
        additions: 3,
        deletions: 1,
        compare_ref: "origin/main".to_string(),
        generated_at_ms: 1236,
    };
    let content_snapshot =
        serde_json::to_value(LocalDaemonResponse::WorkspaceFileContent { content })
            .expect("workspace file content should serialize");
    let content_payload = content_snapshot
        .pointer("/WorkspaceFileContent/content")
        .expect("workspace file content payload should serialize");
    assert_eq!(
        content_payload.pointer("/language"),
        Some(&serde_json::json!("rust"))
    );
    assert_eq!(
        content_payload.pointer("/encoding"),
        Some(&serde_json::json!("utf-8"))
    );
    assert_eq!(
        content_payload.pointer("/content_text"),
        Some(&serde_json::json!("fn main() {}\n"))
    );
    let serialized = serde_json::to_string(content_payload)
        .expect("workspace file content snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "a2bff9ada5aa65ea753652ae69c8b574759bfcd50962dd07015c5958908dfdd4"
    );
}

fn launch_slow_structured_run(app: &mut DaemonApp, session_id: &str, agent_id: &str) -> String {
    app.launch_provider(
        LaunchProviderRequest::new(
            session_id,
            "dev-stub",
            "slow-structured",
            "default",
            "default",
        )
        .with_agent_id(agent_id),
    )
    .expect("slow structured provider run should launch")
    .id()
    .to_string()
}

#[test]
fn local_request_api_supports_session_attach_and_end() {
    let harness = LocalRouterTestHarness::new();
    let (session, _default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };

    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let detached = match harness
        .dispatch(LocalDaemonRequest::DetachFromSession(
            DetachFromSessionRequest {
                attachment_id: attachment.id().to_string(),
            },
        ))
        .expect("detach should succeed")
    {
        LocalDaemonResponse::SessionDetached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let ended = match harness
        .dispatch(LocalDaemonRequest::EndSession(EndSessionRequest {
            session_id: session.id().to_string(),
        }))
        .expect("end session should succeed")
    {
        LocalDaemonResponse::SessionEnded { session } => session,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(detached.id(), attachment.id());
    assert_eq!(ended.id(), session.id());
    harness.with_app(|app| {
        assert!(app.attachments().get_attachment(detached.id()).is_err());
    });
}

#[test]
fn structured_output_pump_applies_finished_jobs_from_other_runs() {
    let harness = LocalRouterTestHarness::new();
    let (session, default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-structured-output", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-structured-output".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    let worker_agent = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("worker".to_string()),
            provider: Some("slow-structured".to_string()),
            model: Some("default".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            machine_ref: None,
            worktree_placement: None,
        }))
        .expect("worker agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    let (background_run_id, requested_records) = harness.with_app_mut(|app| {
        let background_run_id = launch_slow_structured_run(app, session.id(), default_agent.id());
        let requested_run_id = launch_slow_structured_run(app, session.id(), worker_agent.id());
        app.providers_mut()
            .push_finished_structured_output_poll_for_test(
                background_run_id.clone(),
                Ok(Some(ProviderPromptSignalBatch {
                    chunks: vec![ProviderPromptChunk {
                        kind: TerminalOutputKind::ProviderOutput,
                        merge_key: Some("background-chunk".to_string()),
                        bytes: b"background-run-output\n".to_vec(),
                    }],
                    ..ProviderPromptSignalBatch::default()
                })),
            );

        let recipient_attachment_ids = app.attachments().list_session_attachment_ids(session.id());
        let requested_records = ProviderOutputPump::new(app)
            .pump_provider_output(ProviderOutputPumpRequest {
                session_id: session.id(),
                provider_run_id: &requested_run_id,
                recipient_attachment_ids,
                initial_liveness_already_checked: false,
            })
            .expect("requested run pump should drain all finished structured jobs");
        (background_run_id, requested_records)
    });

    assert!(
        requested_records.is_empty(),
        "background run output should be buffered for recipients, not returned as requested-run output"
    );
    let buffered_records = harness.with_app_mut(|app| {
        app.terminal_mut()
            .drain_output_records(session.id(), attachment.id())
    });
    assert_eq!(buffered_records.len(), 1);
    assert_eq!(buffered_records[0].provider_run_id, background_run_id);
    assert_eq!(buffered_records[0].bytes, b"background-run-output\n");
}

#[test]
fn local_request_api_lists_live_remote_machines_and_kernels() {
    let config = DaemonConfig::for_tests();
    let host_machine_id = config.host_machine_id.clone();
    let harness = LocalRouterTestHarness::with_config(config);
    harness.with_app_mut(|app| {
        app.remote_relay_inventory_projection_store().update(
            crate::local::provider_requests::remote_machine_records(
                vec![RelayMachinePresence {
                    machine_id: "machine-1".to_string(),
                    machine_alias: Some("workstation".to_string()),
                    kernel_count: 1,
                    available_providers: vec!["codex".to_string(), "opencode".to_string()],
                }],
                &host_machine_id,
            ),
            vec![RelayKernelPresence {
                kernel_id: "daemon-1".to_string(),
                machine_id: "machine-1".to_string(),
                machine_alias: Some("workstation".to_string()),
                relay_alias: Some("mbp".to_string()),
                kernel_alias: Some("default".to_string()),
                available_providers: vec!["codex".to_string(), "opencode".to_string()],
                capabilities: vec!["kernel_ws".to_string()],
                accepting_remote_leases: true,
                leased_agent_count: 2,
                local_session_count: 3,
                public_key: "public-key".to_string(),
            }],
        );
    });

    let machines = match harness
        .dispatch(LocalDaemonRequest::ListRemoteMachines(
            ListRemoteMachinesRequest,
        ))
        .expect("remote machines request should succeed")
    {
        LocalDaemonResponse::RemoteMachinesListed { machines } => machines,
        other => panic!("unexpected response: {other:?}"),
    };
    let machine = machines
        .iter()
        .find(|machine| machine.machine_id == "machine-1")
        .expect("registered machine should be listed");
    assert_eq!(machine.machine_alias.as_deref(), Some("workstation"));
    assert_eq!(machine.display_name, "workstation");
    assert_eq!(machine.available_providers, vec!["codex", "opencode"]);

    let kernels = match harness
        .dispatch(LocalDaemonRequest::ListRemoteMachineKernels(
            ListRemoteMachineKernelsRequest {
                machine_ref: "workstation".to_string(),
            },
        ))
        .expect("remote machine kernels request should succeed")
    {
        LocalDaemonResponse::RemoteMachineKernelsListed { kernels, .. } => kernels,
        other => panic!("unexpected response: {other:?}"),
    };
    assert_eq!(kernels.len(), 1);
    assert_eq!(kernels[0].kernel_id, "daemon-1");
    assert_eq!(kernels[0].available_providers, vec!["codex", "opencode"]);
    assert!(kernels[0].accepting_remote_leases);
}

#[test]
fn waiting_room_inventory_includes_kernels_for_pending_visible_remote_machines() {
    let config = DaemonConfig::for_tests();
    let host_machine_id = config.host_machine_id.clone();
    let harness = LocalRouterTestHarness::with_config(config);
    harness.with_app_mut(|app| {
        app.remote_relay_inventory_projection_store().update(
            crate::local::provider_requests::remote_machine_records(
                vec![RelayMachinePresence {
                    machine_id: "machine-pending".to_string(),
                    machine_alias: Some("pending machine".to_string()),
                    kernel_count: 1,
                    available_providers: vec!["codex".to_string()],
                }],
                &host_machine_id,
            ),
            vec![RelayKernelPresence {
                kernel_id: "daemon-pending".to_string(),
                machine_id: "machine-pending".to_string(),
                machine_alias: Some("pending machine".to_string()),
                relay_alias: Some("pending-kernel".to_string()),
                kernel_alias: Some("default".to_string()),
                available_providers: vec!["codex".to_string()],
                capabilities: vec!["kernel_ws".to_string()],
                accepting_remote_leases: true,
                leased_agent_count: 0,
                local_session_count: 0,
                public_key: "public-key".to_string(),
            }],
        );
    });

    let snapshot = match harness
        .dispatch(LocalDaemonRequest::GetWaitingRoomInventory(
            GetWaitingRoomInventoryRequest,
        ))
        .expect("waiting room inventory should succeed")
    {
        LocalDaemonResponse::WaitingRoomInventory { snapshot } => snapshot,
        other => panic!("unexpected response: {other:?}"),
    };

    let machine = snapshot
        .remote_machines
        .iter()
        .find(|machine| machine.machine_id == "machine-pending")
        .expect("pending machine should be visible");
    assert!(
        machine.pending,
        "pending machine should remain marked pending"
    );
    assert_eq!(machine.kernel_count, 1);
    assert_eq!(snapshot.remote_kernels.len(), 1);
    assert_eq!(snapshot.remote_kernels[0].machine_id, "machine-pending");
    assert_eq!(snapshot.remote_kernels[0].kernel_id, "daemon-pending");
    assert!(
        snapshot.launch_target.is_some(),
        "waiting room inventory should include the inferred launch target"
    );
}

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

    assert_eq!(snapshot.schema_version, 3);
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
            execution_mode: None,
            permission_level: Some(crate::provider::AgentPermissionLevel::Required),
            worktree_id: None,
            machine_ref: None,
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
                output_schema_ref: None,
                validation_policy: None,
                expected_workflow_revision: None,
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

#[test]
fn local_request_api_resolves_and_deletes_sessions_by_ref() {
    let harness = LocalRouterTestHarness::new();
    let (session, _agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1").with_alias("main"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };

    let resolved = match harness
        .dispatch(LocalDaemonRequest::ResolveSession(ResolveSessionRequest {
            session_ref: "mai".to_string(),
            workspace_id: Some("workspace-1".to_string()),
        }))
        .expect("resolve should succeed")
    {
        LocalDaemonResponse::SessionResolved { session } => session,
        _ => panic!("unexpected local response"),
    };

    let deleted = match harness
        .dispatch(LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
            session_ref: session.id()[..8].to_string(),
            workspace_id: Some("workspace-1".to_string()),
        }))
        .expect("delete should succeed")
    {
        LocalDaemonResponse::SessionDeleted { session } => session,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(resolved.id(), session.id());
    assert_eq!(deleted.id(), session.id());
    assert_eq!(deleted.alias(), Some("main"));
    assert_eq!(deleted.status(), crate::session::SessionStatus::Ended);
    assert!(matches!(
        harness.dispatch(LocalDaemonRequest::ResolveSession(ResolveSessionRequest {
            session_ref: "main".to_string(),
            workspace_id: Some("workspace-1".to_string()),
        })),
        Err(DaemonError::SessionNotFound { .. })
    ));
    let listed = match harness
        .dispatch(LocalDaemonRequest::ListSessions(ListSessionsRequest))
        .expect("list should succeed")
    {
        LocalDaemonResponse::SessionsListed { sessions } => sessions,
        _ => panic!("unexpected local response"),
    };
    assert!(listed.is_empty());
}

#[test]
fn local_request_api_manages_session_invites_and_members() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    let session_id = session.id().to_string();
    let invite_record = match harness
        .dispatch(LocalDaemonRequest::CreateSessionInvite(
            CreateSessionInviteRequest {
                session_id: session_id.clone(),
                expires_in_ms: None,
                max_uses: Some(1),
            },
        ))
        .expect("session invite create should succeed")
    {
        LocalDaemonResponse::SessionInviteCreated { invite, session } => {
            assert_eq!(session.id(), session_id);
            invite
        }
        _ => panic!("unexpected local response"),
    };

    let joined = match harness
        .dispatch(LocalDaemonRequest::JoinSessionInvite(
            JoinSessionInviteRequest {
                invite_token: invite_record.invite_token.clone(),
                user_id: "user-2".to_string(),
            },
        ))
        .expect("session invite join should succeed")
    {
        LocalDaemonResponse::SessionInviteJoined { member, session } => {
            assert!(session.has_member("user-2"));
            member
        }
        _ => panic!("unexpected local response"),
    };
    assert_eq!(joined.user_id(), "user-2");

    let (members, invites) = match harness
        .dispatch(LocalDaemonRequest::ListSessionMembers(
            ListSessionMembersRequest {
                session_id: session_id.clone(),
            },
        ))
        .expect("session members should list")
    {
        LocalDaemonResponse::SessionMembersListed { members, invites } => (members, invites),
        _ => panic!("unexpected local response"),
    };
    assert_eq!(members.len(), 2);
    assert_eq!(invites.len(), 1);
    assert_eq!(invites[0].used_count(), 1);

    let revoked = match harness
        .dispatch(LocalDaemonRequest::RevokeSessionInvite(
            RevokeSessionInviteRequest {
                session_id,
                invite_ref: invite_record.invite.invite_id().to_string(),
            },
        ))
        .expect("session invite revoke should succeed")
    {
        LocalDaemonResponse::SessionInviteRevoked { invite, .. } => invite,
        _ => panic!("unexpected local response"),
    };
    assert!(revoked.is_revoked());
}

#[test]
fn local_request_api_manages_session_workspace_links() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "/tmp/arroba-worktree-a"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let session_id = session.id().to_string();

    let denied = harness.dispatch_as_user(
        "stranger",
        LocalDaemonRequest::ListWorkspaceLinks(ListWorkspaceLinksRequest {
            session_id: session_id.clone(),
        }),
    );
    assert!(matches!(
        denied,
        Err(DaemonError::SessionAccessDenied { .. })
    ));

    let link = match harness
        .dispatch(LocalDaemonRequest::CreateWorkspaceLink(
            CreateWorkspaceLinkRequest {
                session_id: session_id.clone(),
                name: "shared-repo".to_string(),
            },
        ))
        .expect("workspace link create should succeed")
    {
        LocalDaemonResponse::WorkspaceLinkCreated { link, session } => {
            assert_eq!(session.workspace_links().len(), 1);
            link
        }
        _ => panic!("unexpected local response"),
    };
    assert_eq!(link.name(), "shared-repo");

    let attached = match harness
        .dispatch(LocalDaemonRequest::AttachWorkspaceLink(
            AttachWorkspaceLinkRequest {
                session_id: session_id.clone(),
                link_ref: "shared".to_string(),
                repo_root: Some("/tmp/arroba-worktree-a".to_string()),
                branch: Some("main".to_string()),
                repo_fingerprint: Some("fingerprint-a".to_string()),
            },
        ))
        .expect("workspace link attach should succeed")
    {
        LocalDaemonResponse::WorkspaceLinkAttached {
            link,
            attachment,
            session,
        } => {
            assert_eq!(session.workspace_links()[0].attachments().len(), 1);
            assert_eq!(attachment.repo_root(), "/tmp/arroba-worktree-a");
            link
        }
        _ => panic!("unexpected local response"),
    };
    assert_eq!(attached.attachments().len(), 1);

    let shown = match harness
        .dispatch(LocalDaemonRequest::ShowWorkspaceLink(
            ShowWorkspaceLinkRequest {
                session_id: session_id.clone(),
                link_ref: link.link_id().to_string(),
            },
        ))
        .expect("workspace link show should succeed")
    {
        LocalDaemonResponse::WorkspaceLinkShown { link } => link,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(shown.attachments().len(), 1);

    let listed = match harness
        .dispatch(LocalDaemonRequest::ListWorkspaceLinks(
            ListWorkspaceLinksRequest {
                session_id: session_id.clone(),
            },
        ))
        .expect("workspace links list should succeed")
    {
        LocalDaemonResponse::WorkspaceLinksListed { links } => links,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(listed.len(), 1);

    let detached = match harness
        .dispatch(LocalDaemonRequest::DetachWorkspaceLink(
            DetachWorkspaceLinkRequest {
                session_id,
                link_ref: "shared-repo".to_string(),
                repo_root: Some("/tmp/arroba-worktree-a".to_string()),
            },
        ))
        .expect("workspace link detach should succeed")
    {
        LocalDaemonResponse::WorkspaceLinkDetached { link, detached, .. } => {
            assert!(link.attachments().is_empty());
            detached
        }
        _ => panic!("unexpected local response"),
    };
    assert_eq!(detached.len(), 1);
}

#[test]
fn local_request_api_aliases_sessions() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    let aliased = match harness
        .dispatch(LocalDaemonRequest::AliasSession(AliasSessionRequest {
            session_id: session.id().to_string(),
            alias: "alpha".to_string(),
        }))
        .expect("alias should succeed")
    {
        LocalDaemonResponse::SessionAliased { session } => session,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(aliased.alias(), Some("alpha"));

    let resolved = match harness
        .dispatch(LocalDaemonRequest::ResolveSession(ResolveSessionRequest {
            session_ref: "alpha".to_string(),
            workspace_id: Some("workspace-1".to_string()),
        }))
        .expect("alias resolve should succeed")
    {
        LocalDaemonResponse::SessionResolved { session } => session,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(resolved.id(), session.id());
}

#[test]
fn local_request_api_spawns_and_focuses_agents() {
    let harness = LocalRouterTestHarness::new();
    let (session, default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };

    let spawned = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("reviewer".to_string()),
            provider: Some("opencode".to_string()),
            model: Some("openai/gpt-5.4".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            machine_ref: None,
            worktree_placement: None,
        }))
        .expect("spawn should succeed")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    let (session_state, agent_activity) = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("session state should load")
    {
        LocalDaemonResponse::SessionState {
            session,
            agent_activity,
        } => (session, agent_activity),
        _ => panic!("unexpected local response"),
    };

    assert_eq!(session_state.agents().len(), 2);
    assert_eq!(
        agent_activity
            .get(default_agent.id())
            .expect("default agent activity should be projected")
            .status,
        crate::runtime::projection::AgentRuntimeStatus::Idle
    );
    assert_eq!(
        agent_activity
            .get(spawned.id())
            .expect("spawned agent activity should be projected")
            .status,
        crate::runtime::projection::AgentRuntimeStatus::Idle
    );
    assert_eq!(session_state.focused_agent_id(), Some(spawned.id()));
    assert_eq!(
        session_state
            .agents()
            .iter()
            .map(|agent| agent.id())
            .collect::<Vec<_>>(),
        vec![default_agent.id(), spawned.id()]
    );
    assert_eq!(
        session_state
            .agents()
            .iter()
            .find(|agent| agent.id() == default_agent.id())
            .expect("default agent should still exist")
            .state(),
        crate::agent::AgentState::Idle
    );
    assert_eq!(
        session_state
            .agents()
            .iter()
            .find(|agent| agent.id() == spawned.id())
            .expect("spawned agent should exist")
            .state(),
        crate::agent::AgentState::Focused
    );

    let renamed = match harness
        .dispatch(LocalDaemonRequest::AliasAgent(AliasAgentRequest {
            session_id: session.id().to_string(),
            agent_id: spawned.id().to_string(),
            alias: "web-reviewer".to_string(),
        }))
        .expect("agent alias update should succeed")
    {
        LocalDaemonResponse::AgentAliased { agent, session } => {
            assert_eq!(
                session
                    .agents()
                    .iter()
                    .find(|entry| entry.id() == spawned.id())
                    .and_then(|entry| entry.alias()),
                Some("web-reviewer")
            );
            agent
        }
        _ => panic!("unexpected local response"),
    };

    assert_eq!(renamed.alias(), Some("web-reviewer"));

    let profiled = match harness
        .dispatch(LocalDaemonRequest::UpdateAgentProfile(
            UpdateAgentProfileRequest {
                session_id: session.id().to_string(),
                agent_id: spawned.id().to_string(),
                provider: Some("codex".to_string()),
                model: Some("gpt-5.4".to_string()),
                effort: Some("low".to_string()),
                clear_effort: false,
            },
        ))
        .expect("agent profile update should succeed")
    {
        LocalDaemonResponse::AgentProfileUpdated { agent, session } => {
            let entry = session
                .agents()
                .iter()
                .find(|entry| entry.id() == spawned.id())
                .expect("updated agent should remain in session snapshot");
            assert_eq!(entry.provider(), "codex");
            assert_eq!(entry.model(), Some("gpt-5.4"));
            assert_eq!(entry.effort(), Some("low"));
            agent
        }
        _ => panic!("unexpected local response"),
    };

    assert_eq!(profiled.provider(), "codex");
    assert_eq!(profiled.model(), Some("gpt-5.4"));
    assert_eq!(profiled.effort(), Some("low"));

    let cleared = match harness
        .dispatch(LocalDaemonRequest::UpdateAgentProfile(
            UpdateAgentProfileRequest {
                session_id: session.id().to_string(),
                agent_id: spawned.id().to_string(),
                provider: None,
                model: None,
                effort: None,
                clear_effort: true,
            },
        ))
        .expect("agent profile clear should succeed")
    {
        LocalDaemonResponse::AgentProfileUpdated { agent, session } => {
            let entry = session
                .agents()
                .iter()
                .find(|entry| entry.id() == spawned.id())
                .expect("updated agent should remain in session snapshot");
            assert_eq!(entry.effort(), None);
            agent
        }
        _ => panic!("unexpected local response"),
    };

    assert_eq!(cleared.provider(), "codex");
    assert_eq!(cleared.model(), Some("gpt-5.4"));
    assert_eq!(cleared.effort(), None);

    let focused_default = match harness
        .dispatch(LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session.id().to_string(),
            agent_id: default_agent.id().to_string(),
        }))
        .expect("focus should succeed")
    {
        LocalDaemonResponse::AgentFocused { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(focused_default.id(), default_agent.id());

    let cycled = match harness
        .dispatch(LocalDaemonRequest::CycleAgentFocus(
            CycleAgentFocusRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("cycle should succeed")
    {
        LocalDaemonResponse::AgentFocusCycled { agent } => {
            agent.expect("cycle should return a focused agent")
        }
        _ => panic!("unexpected local response"),
    };

    assert_eq!(cycled.id(), spawned.id());

    let listed = match harness
        .dispatch(LocalDaemonRequest::ListAgents(ListAgentsRequest {
            session_id: session.id().to_string(),
        }))
        .expect("list should succeed")
    {
        LocalDaemonResponse::AgentsListed { agents } => agents,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(listed.len(), 2);
    assert_eq!(
        listed.iter().map(|agent| agent.id()).collect::<Vec<_>>(),
        vec![default_agent.id(), spawned.id()]
    );
    assert_eq!(
        listed
            .iter()
            .find(|agent| agent.id() == spawned.id())
            .expect("spawned agent should be listed")
            .state(),
        crate::agent::AgentState::Focused
    );
}

#[test]
fn local_request_api_manages_workflows_endpoints_and_graph_edits() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    let agent = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("reviewer".to_string()),
            provider: Some("dev-stub".to_string()),
            model: Some("default".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            machine_ref: None,
            worktree_placement: None,
        }))
        .expect("workflow agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("review".to_string()),
        }))
        .expect("workflow create should succeed")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };

    let listed = match harness
        .dispatch(LocalDaemonRequest::ListWorkflows(ListWorkflowsRequest {
            session_id: session.id().to_string(),
        }))
        .expect("workflow list should succeed")
    {
        LocalDaemonResponse::WorkflowsListed { workflows } => workflows,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(listed.len(), 1);

    let resolved = match harness
        .dispatch(LocalDaemonRequest::ResolveWorkflow(
            ResolveWorkflowRequest {
                session_id: session.id().to_string(),
                workflow_ref: "review".to_string(),
            },
        ))
        .expect("workflow resolve should succeed")
    {
        LocalDaemonResponse::WorkflowResolved { workflow } => workflow,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(resolved.id(), workflow.id());

    let node_a = match harness
        .dispatch(LocalDaemonRequest::AddWorkflowNode(
            AddWorkflowNodeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                agent_id: agent.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("first workflow node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        _ => panic!("unexpected local response"),
    };

    let duplicate_node = harness
        .dispatch(LocalDaemonRequest::AddWorkflowNode(
            AddWorkflowNodeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                agent_id: agent.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect_err("duplicate workflow node should be rejected");
    assert!(matches!(
        duplicate_node,
        DaemonError::WorkflowNodeConflict { .. }
    ));

    match harness
        .dispatch(LocalDaemonRequest::UpdateWorkflowNodeInstructions(
            UpdateWorkflowNodeInstructionsRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                node_id: node_a.id().to_string(),
                instructions: Some("You are the reviewer.".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow node instructions should update")
    {
        LocalDaemonResponse::WorkflowNodeInstructionsUpdated { node, .. } => {
            assert_eq!(node.instructions(), Some("You are the reviewer."));
        }
        _ => panic!("unexpected local response"),
    };

    let spawned = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("reviewer-2".to_string()),
            provider: Some("opencode".to_string()),
            model: None,
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            machine_ref: None,
            worktree_placement: None,
        }))
        .expect("spawn should succeed")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    let node_b = match harness
        .dispatch(LocalDaemonRequest::AddWorkflowNode(
            AddWorkflowNodeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                agent_id: spawned.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("second workflow node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        _ => panic!("unexpected local response"),
    };

    let endpoint = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: node_a.id().to_string(),
                alias: Some("entry".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(endpoint.entry_node_id(), node_a.id());

    let aliased_workflow = match harness
        .dispatch(LocalDaemonRequest::AliasWorkflow(AliasWorkflowRequest {
            session_id: session.id().to_string(),
            workflow_ref: workflow.id().to_string(),
            alias: "qa".to_string(),
            expected_workflow_revision: None,
        }))
        .expect("workflow alias should succeed")
    {
        LocalDaemonResponse::WorkflowAliased { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(aliased_workflow.alias(), Some("qa"));

    let aliased_endpoint = match harness
        .dispatch(LocalDaemonRequest::AliasWorkflowEndpoint(
            AliasWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                alias: "start".to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow endpoint alias should succeed")
    {
        LocalDaemonResponse::WorkflowEndpointAliased { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(aliased_endpoint.alias(), Some("start"));

    let edge = match harness
        .dispatch(LocalDaemonRequest::AddWorkflowEdge(
            AddWorkflowEdgeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                from_node_id: node_a.id().to_string(),
                to_node_id: node_b.id().to_string(),
                output_schema_ref: None,
                validation_policy: None,
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow edge should be added")
    {
        LocalDaemonResponse::WorkflowEdgeAdded { edge, .. } => edge,
        _ => panic!("unexpected local response"),
    };

    match harness
        .dispatch(LocalDaemonRequest::UpdateWorkflowCanvasLayout(
            UpdateWorkflowCanvasLayoutRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                base_layout_revision: None,
                patches: vec![
                    crate::session::WorkflowCanvasLayoutPatch::NodePosition {
                        node_id: node_a.id().to_string(),
                        x: 120,
                        y: 80,
                    },
                    crate::session::WorkflowCanvasLayoutPatch::NodePosition {
                        node_id: node_b.id().to_string(),
                        x: 420,
                        y: 80,
                    },
                    crate::session::WorkflowCanvasLayoutPatch::EndpointPosition {
                        endpoint_id: endpoint.id().to_string(),
                        x: 180,
                        y: 36,
                    },
                ],
            },
        ))
        .expect("workflow canvas layout should update")
    {
        LocalDaemonResponse::WorkflowCanvasLayoutUpdated {
            layout, workflow, ..
        } => {
            assert_eq!(layout.revision, 1);
            assert_eq!(layout.nodes.get(node_a.id()).map(|point| point.x), Some(120));
            assert_eq!(
                workflow
                    .canvas_layout()
                    .and_then(|stored| stored.endpoints.get(endpoint.id()))
                    .map(|point| point.y),
                Some(36)
            );
        }
        _ => panic!("unexpected local response"),
    }

    match harness
        .dispatch(LocalDaemonRequest::RemoveWorkflowEdge(
            RemoveWorkflowEdgeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                edge_id: edge.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow edge should be removed")
    {
        LocalDaemonResponse::WorkflowEdgeRemoved { .. } => {}
        _ => panic!("unexpected local response"),
    }

    match harness
        .dispatch(LocalDaemonRequest::RemoveWorkflowNode(
            RemoveWorkflowNodeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                node_id: node_a.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow node should be removed")
    {
        LocalDaemonResponse::WorkflowNodeRemoved { .. } => {}
        _ => panic!("unexpected local response"),
    }
}

#[test]
fn local_request_api_invokes_lists_gets_and_cancels_workflow_runs() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    let agent = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("reviewer".to_string()),
            provider: Some("dev-stub".to_string()),
            model: Some("default".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            machine_ref: None,
            worktree_placement: None,
        }))
        .expect("workflow agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("review".to_string()),
        }))
        .expect("workflow create should succeed")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };

    let node = match harness
        .dispatch(LocalDaemonRequest::AddWorkflowNode(
            AddWorkflowNodeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                agent_id: agent.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        _ => panic!("unexpected local response"),
    };

    let endpoint = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: node.id().to_string(),
                alias: Some("entry".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };

    match harness
        .dispatch(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session.id().to_string(),
                agent_id: Some(agent.id().to_string()),
                adapter_key: "dev-stub".to_string(),
                provider: "dev-stub".to_string(),
                account_profile: "default".to_string(),
                model: "default".to_string(),
                variant: None,
            },
        ))
        .expect("provider run should launch")
    {
        LocalDaemonResponse::ProviderRunLaunched { .. }
        | LocalDaemonResponse::ProviderRunLaunchAccepted { .. } => {}
        _ => panic!("unexpected local response"),
    }
    let _ = harness.wait_for_active_provider_run(session.id());

    let workflow_run = match harness
        .dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
            InvokeWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                prompt: Some("review this diff".to_string()),
            },
        ))
        .expect("workflow run invocation should succeed")
    {
        LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(workflow_run.workflow_id(), workflow.id());
    assert_eq!(workflow_run.endpoint_id(), endpoint.id());
    assert_eq!(format!("{:?}", workflow_run.status()), "Running");

    let listed = match harness
        .dispatch(LocalDaemonRequest::ListWorkflowRuns(
            ListWorkflowRunsRequest {
                session_id: session.id().to_string(),
                workflow_ref: Some(workflow.id().to_string()),
            },
        ))
        .expect("workflow runs should list")
    {
        LocalDaemonResponse::WorkflowRunsListed { workflow_runs } => workflow_runs,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id(), workflow_run.id());

    let resolved = match harness
        .dispatch(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
            session_id: session.id().to_string(),
            workflow_run_ref: workflow_run.id().to_string(),
        }))
        .expect("workflow run should resolve")
    {
        LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(resolved.id(), workflow_run.id());
    assert_eq!(format!("{:?}", resolved.status()), "Running");

    match harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("workflow-backed prompt should complete")
    {
        LocalDaemonResponse::PromptCompleted { .. } => {}
        _ => panic!("unexpected local response"),
    }

    let completed = match harness
        .dispatch(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
            session_id: session.id().to_string(),
            workflow_run_ref: workflow_run.id().to_string(),
        }))
        .expect("completed workflow run should resolve")
    {
        LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(format!("{:?}", completed.status()), "Completed");

    let second_run = match harness
        .dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
            InvokeWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                prompt: Some("review this diff again".to_string()),
            },
        ))
        .expect("second workflow run invocation should succeed")
    {
        LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
        _ => panic!("unexpected local response"),
    };

    let cancelled = match harness
        .dispatch(LocalDaemonRequest::CancelWorkflowRun(
            CancelWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: second_run.id().to_string(),
            },
        ))
        .expect("workflow run should cancel")
    {
        LocalDaemonResponse::WorkflowRunCancelled { workflow_run, .. } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(cancelled.id(), second_run.id());
    assert_eq!(format!("{:?}", cancelled.status()), "Stopped");
}

#[test]
fn local_request_api_routes_and_schedules_downstream_workflow_nodes() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    let first_agent = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("planner".to_string()),
            provider: Some("dev-stub".to_string()),
            model: Some("default".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            machine_ref: None,
            worktree_placement: None,
        }))
        .expect("first workflow agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    let second_agent = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("reviewer".to_string()),
            provider: Some("dev-stub".to_string()),
            model: Some("default".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            machine_ref: None,
            worktree_placement: None,
        }))
        .expect("second workflow agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("review".to_string()),
        }))
        .expect("workflow create should succeed")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
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
        .expect("first workflow node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        _ => panic!("unexpected local response"),
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
        .expect("second workflow node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        _ => panic!("unexpected local response"),
    };

    match harness
        .dispatch(LocalDaemonRequest::AddWorkflowEdge(
            AddWorkflowEdgeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                from_node_id: first_node.id().to_string(),
                to_node_id: second_node.id().to_string(),
                output_schema_ref: None,
                validation_policy: None,
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow edge should be added")
    {
        LocalDaemonResponse::WorkflowEdgeAdded { .. } => {}
        _ => panic!("unexpected local response"),
    }

    let endpoint = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: first_node.id().to_string(),
                alias: Some("entry".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };

    let workflow_run = match harness
        .dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
            InvokeWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                prompt: Some("route this workflow".to_string()),
            },
        ))
        .expect("workflow invoke should succeed")
    {
        LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(format!("{:?}", workflow_run.status()), "Running");
    assert_eq!(workflow_run.node_runs().len(), 1);
    let workflow_attachment_id =
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id());
    let provider_run_id = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session state should resolve")
            .active_provider_run_id()
            .expect("workflow invoke should activate a provider run")
            .to_string()
    });
    harness.with_app_mut(|app| {
        app.fan_out_output(
            session.id(),
            &provider_run_id,
            TerminalOutputKind::ProviderOutput,
            None,
            Vec::new(),
            b"```json\n{\"summary\":\"planner finished draft plan\",\"output\":{\"message\":\"Please review the attached generated plan and provide approval feedback.\"}}\n```\n",
        );
        app.fan_out_output(
            session.id(),
            &provider_run_id,
            TerminalOutputKind::ProviderTool,
            None,
            Vec::new(),
            b"{\"tool\":\"rg\",\"status\":\"ok\"}\n",
        );
    });
    let workflow_transfer_root =
        DaemonApp::attachment_artifact_root(session.id(), &workflow_attachment_id, "transfers");
    std::fs::create_dir_all(&workflow_transfer_root).expect("workflow transfer root should exist");
    let workflow_artifact_path = workflow_transfer_root.join("generated-plan.md");
    std::fs::write(&workflow_artifact_path, "# generated plan\n")
        .expect("workflow artifact should be written");

    match harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("entry workflow prompt should complete")
    {
        LocalDaemonResponse::PromptCompleted { .. } => {}
        _ => panic!("unexpected local response"),
    }

    let routed = match harness
        .dispatch(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
            session_id: session.id().to_string(),
            workflow_run_ref: workflow_run.id().to_string(),
        }))
        .expect("routed workflow run should resolve")
    {
        LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(format!("{:?}", routed.status()), "Running");
    assert_eq!(routed.node_runs().len(), 2);
    assert_eq!(routed.messages().len(), 2);
    assert_eq!(
        routed.active_node_run_id(),
        Some(routed.node_runs()[1].id())
    );
    assert_eq!(routed.node_runs()[1].node_id(), second_node.id());
    let completed_entry = routed
        .node_runs()
        .iter()
        .find(|node_run| node_run.node_id() == first_node.id())
        .expect("completed entry node should remain on the run");
    assert_eq!(format!("{:?}", completed_entry.status()), "Completed");
    assert!(completed_entry
        .summary()
        .is_some_and(|summary| summary.contains("planner finished draft plan")));
    let completion = completed_entry
        .completion()
        .expect("completed entry node should retain a generic completion snapshot");
    assert_eq!(completion.summary(), "planner finished draft plan");
    let output = completion
        .output()
        .expect("completed entry node should retain explicit downstream output");
    assert_eq!(
        output.message(),
        "Please review the attached generated plan and provide approval feedback."
    );
    assert_eq!(output.artifacts().len(), 1);
    assert_eq!(output.artifacts()[0].kind(), "transfer");
    assert_eq!(output.artifacts()[0].display_name(), "generated-plan.md");
    assert_eq!(
        output.artifacts()[0].path(),
        workflow_artifact_path.to_string_lossy()
    );
    let handoff_message = routed
        .messages()
        .iter()
        .find(|message| message.source_node_run_id() == Some(completed_entry.id()))
        .expect("downstream handoff message should exist");
    let handoff_payload: WorkflowHandoffPayload =
        serde_json::from_str(handoff_message.handoff_payload())
            .expect("handoff payload should deserialize");
    let handoff_completion = handoff_payload
        .completion()
        .expect("handoff payload should carry the generic completion snapshot");
    assert_eq!(handoff_completion.summary(), "planner finished draft plan");
    let handoff_output = handoff_completion
        .output()
        .expect("handoff payload should carry explicit downstream output");
    assert_eq!(
        handoff_output.message(),
        "Please review the attached generated plan and provide approval feedback."
    );
    assert_eq!(handoff_output.artifacts().len(), 1);
    assert_eq!(
        handoff_output.artifacts()[0].display_name(),
        "generated-plan.md"
    );

    match harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("downstream workflow prompt should complete")
    {
        LocalDaemonResponse::PromptCompleted { .. } => {}
        _ => panic!("unexpected local response"),
    }

    let completed = match harness
        .dispatch(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
            session_id: session.id().to_string(),
            workflow_run_ref: workflow_run.id().to_string(),
        }))
        .expect("completed workflow run should resolve")
    {
        LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(format!("{:?}", completed.status()), "Completed");
    assert_eq!(completed.node_runs().len(), 2);
    assert_eq!(
        completed
            .node_runs()
            .iter()
            .map(|node_run| format!("{:?}", node_run.status()))
            .collect::<Vec<_>>(),
        vec!["Completed".to_string(), "Completed".to_string()]
    );
}

#[test]
fn local_request_api_acks_workflow_turn_and_cleans_up_transient_inputs_after_validation_passes() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-ack", "worktree-ack"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    let first_agent = harness.spawn_workflow_test_agent(session.id(), "first");
    let second_agent = harness.spawn_workflow_test_agent(session.id(), "second");
    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("ack-flow".to_string()),
        }))
        .expect("workflow create should succeed")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
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
        .expect("first node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        _ => panic!("unexpected local response"),
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
        .expect("second node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        _ => panic!("unexpected local response"),
    };
    let _ = harness
        .dispatch(LocalDaemonRequest::UpdateWorkflowNodeInstructions(
            UpdateWorkflowNodeInstructionsRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                node_id: first_node.id().to_string(),
                instructions: Some("# First node\nProduce a tiny JSON payload.\n".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("first node instructions should be updated");
    let _ = harness
        .dispatch(LocalDaemonRequest::UpdateWorkflowNodeInstructions(
            UpdateWorkflowNodeInstructionsRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                node_id: second_node.id().to_string(),
                instructions: Some("# Second node\nSummarize the handoff.\n".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("second node instructions should be updated");
    let _ = harness
        .dispatch(LocalDaemonRequest::AddWorkflowEdge(
            AddWorkflowEdgeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                from_node_id: first_node.id().to_string(),
                to_node_id: second_node.id().to_string(),
                output_schema_ref: None,
                validation_policy: None,
                expected_workflow_revision: None,
            },
        ))
        .expect("edge should be added");
    let endpoint = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: first_node.id().to_string(),
                alias: Some("entry".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };

    let (workflow_run, invoke_session) = match harness
        .dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
            InvokeWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                prompt: Some("kick off the ack flow".to_string()),
            },
        ))
        .expect("workflow invoke should succeed")
    {
        LocalDaemonResponse::WorkflowRunInvoked {
            workflow_run,
            session,
            ..
        } => (workflow_run, session),
        _ => panic!("unexpected local response"),
    };
    let active_prompt = invoke_session
        .active_prompt()
        .expect("workflow invoke should create an active prompt");
    assert!(active_prompt
        .prompt()
        .contains("Endpoint prompt:\nkick off the ack flow"));
    assert!(active_prompt
        .prompt()
        .contains("Node instruction reference (daemon-managed):"));
    assert!(active_prompt.prompt().contains("`ack_workflow_turn`"));
    assert!(!active_prompt
        .prompt()
        .contains("Control mailbox (daemon-managed):"));

    let first_run_id = workflow_run.node_runs()[0].id().to_string();
    let first_token = "workflow-ack:".to_string() + &first_run_id;
    match harness
        .dispatch(LocalDaemonRequest::AckWorkflowTurn(
            AckWorkflowTurnRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run.id().to_string(),
                workflow_node_run_id: first_run_id.clone(),
                delivery_token: first_token,
            },
        ))
        .expect("workflow turn ack should succeed")
    {
        LocalDaemonResponse::WorkflowTurnAcknowledged { workflow_run, .. } => {
            let envelope = workflow_run.node_runs()[0]
                .turn_envelope()
                .expect("first turn envelope should exist");
            assert_eq!(envelope.state(), WorkflowTurnRuntimeState::Acknowledged);
        }
        _ => panic!("unexpected local response"),
    }

    let provider_run_id = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session state should resolve")
            .active_provider_run_id()
            .expect("provider run should be active")
            .to_string()
    });
    harness.with_app_mut(|app| {
        app.fan_out_output(
            session.id(),
            &provider_run_id,
            TerminalOutputKind::ProviderOutput,
            None,
            Vec::new(),
            b"```json\n{\"summary\":\"first finished\",\"output\":{\"message\":\"{\\\"value\\\":1}\"}}\n```\n",
        );
    });
    let _ = harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("first workflow prompt should complete");

    let routed = match harness
        .dispatch(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
            session_id: session.id().to_string(),
            workflow_run_ref: workflow_run.id().to_string(),
        }))
        .expect("workflow run should resolve")
    {
        LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    let first_completed = routed
        .node_runs()
        .iter()
        .find(|node_run| node_run.id() == first_run_id)
        .expect("first node run should remain");
    let first_envelope = first_completed
        .turn_envelope()
        .expect("first node run should retain its envelope");
    assert_eq!(
        first_envelope.state(),
        WorkflowTurnRuntimeState::ValidatedCompleted
    );
    assert!(first_envelope.rendered_prompt().is_none());
    assert!(first_envelope.handoff_payloads_json().is_none());
    assert_eq!(routed.messages().len(), 1);

    let second_active_prompt = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should resolve")
            .active_prompt()
            .cloned()
            .expect("second node prompt should be active")
    });
    assert!(second_active_prompt
        .prompt()
        .contains("Workflow handoff payloads (JSON array):"));
    assert!(second_active_prompt
        .prompt()
        .contains("`ack_workflow_turn`"));

    let second_run_id = routed
        .active_node_run_id()
        .expect("second node should be active")
        .to_string();
    let second_token = "workflow-ack:".to_string() + &second_run_id;
    let _ = harness
        .dispatch(LocalDaemonRequest::AckWorkflowTurn(
            AckWorkflowTurnRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run.id().to_string(),
                workflow_node_run_id: second_run_id.clone(),
                delivery_token: second_token,
            },
        ))
        .expect("second workflow turn ack should succeed");
    harness.with_app_mut(|app| {
        app.fan_out_output(
            session.id(),
            &provider_run_id,
            TerminalOutputKind::ProviderOutput,
            None,
            Vec::new(),
            b"```json\n{\"summary\":\"second finished\",\"output\":{\"message\":\"{\\\"done\\\":true}\"}}\n```\n",
        );
    });
    let _ = harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("second workflow prompt should complete");
    let completed = match harness
        .dispatch(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
            session_id: session.id().to_string(),
            workflow_run_ref: workflow_run.id().to_string(),
        }))
        .expect("completed workflow run should resolve")
    {
        LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(format!("{:?}", completed.status()), "Completed");
    let second_completed = completed
        .node_runs()
        .iter()
        .find(|node_run| node_run.id() == second_run_id)
        .expect("second node should complete");
    let second_envelope = second_completed
        .turn_envelope()
        .expect("second node turn envelope should exist");
    assert_eq!(
        second_envelope.state(),
        WorkflowTurnRuntimeState::ValidatedCompleted
    );
    assert!(completed.messages().is_empty());
}

#[test]
fn local_request_api_inlines_mailbox_content_and_retains_inputs_when_validation_warns() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-mailbox", "worktree-mailbox"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let first_agent = harness.spawn_workflow_test_agent(session.id(), "loop-a");
    let second_agent = harness.spawn_workflow_test_agent(session.id(), "loop-b");
    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("mailbox-flow".to_string()),
        }))
        .expect("workflow create should succeed")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
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
        .expect("first node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        _ => panic!("unexpected local response"),
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
        .expect("second node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        _ => panic!("unexpected local response"),
    };
    let schema_path = std::env::temp_dir().join(format!(
        "arroba-mailbox-schema-{}.json",
        crate::session::unix_epoch_ms()
    ));
    fs::write(
            &schema_path,
            "{\n  \"type\": \"object\",\n  \"required\": [\"ok\"],\n  \"properties\": {\"ok\": {\"type\": \"boolean\"}}\n}\n",
        )
        .expect("schema file should be written");
    let _ = harness
        .dispatch(LocalDaemonRequest::AddWorkflowEdge(
            AddWorkflowEdgeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                from_node_id: first_node.id().to_string(),
                to_node_id: second_node.id().to_string(),
                output_schema_ref: Some(schema_path.to_string_lossy().to_string()),
                validation_policy: Some(WorkflowOutputValidationPolicy::Warn),
                expected_workflow_revision: None,
            },
        ))
        .expect("first edge should be added");
    let _ = harness
        .dispatch(LocalDaemonRequest::AddWorkflowEdge(
            AddWorkflowEdgeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                from_node_id: second_node.id().to_string(),
                to_node_id: first_node.id().to_string(),
                output_schema_ref: None,
                validation_policy: None,
                expected_workflow_revision: None,
            },
        ))
        .expect("second edge should be added");
    let endpoint = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: first_node.id().to_string(),
                alias: Some("entry".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };
    let workflow_run = match harness
        .dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
            InvokeWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                prompt: Some("start loop".to_string()),
            },
        ))
        .expect("workflow invoke should succeed")
    {
        LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    let node_run_id = workflow_run.node_runs()[0].id().to_string();
    let _ = harness
        .dispatch(LocalDaemonRequest::AckWorkflowTurn(
            AckWorkflowTurnRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run.id().to_string(),
                workflow_node_run_id: node_run_id.clone(),
                delivery_token: format!("workflow-ack:{node_run_id}"),
            },
        ))
        .expect("ack should succeed");
    let provider_run_id = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should resolve")
            .active_provider_run_id()
            .expect("provider run should be active")
            .to_string()
    });
    harness.with_app_mut(|app| {
        app.fan_out_output(
            session.id(),
            &provider_run_id,
            TerminalOutputKind::ProviderOutput,
            None,
            Vec::new(),
            b"```json\n{\"summary\":\"warn branch\",\"output\":{\"message\":\"not-json\"}}\n```\n",
        );
    });
    let _ = harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("warning workflow prompt should complete");

    let after_warning = match harness
        .dispatch(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
            session_id: session.id().to_string(),
            workflow_run_ref: workflow_run.id().to_string(),
        }))
        .expect("updated workflow run should resolve")
    {
        LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    assert!(after_warning.failure_events().iter().any(|event| {
        matches!(
            event.kind(),
            crate::session::WorkflowFailureKind::OutputValidationFailed
        ) && event.message().contains("output.message is not valid JSON")
    }));
    let second_active_prompt = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should resolve")
            .active_prompt()
            .cloned()
            .expect("second node should be active")
    });
    assert!(second_active_prompt.prompt().contains("Control mailbox:"));
    assert!(second_active_prompt
        .prompt()
        .contains("output.message is not valid JSON"));
    let first_completed = after_warning
        .node_runs()
        .iter()
        .find(|run| run.id() == node_run_id)
        .expect("first node run should remain");
    assert_eq!(
        first_completed
            .turn_envelope()
            .expect("turn envelope should remain")
            .state(),
        WorkflowTurnRuntimeState::Acknowledged
    );
    assert!(first_completed
        .turn_envelope()
        .expect("turn envelope should remain")
        .rendered_prompt()
        .is_some());

    let second_run_id = after_warning
        .active_node_run_id()
        .expect("second node should now be active")
        .to_string();
    let _ = harness
        .dispatch(LocalDaemonRequest::AckWorkflowTurn(
            AckWorkflowTurnRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run.id().to_string(),
                workflow_node_run_id: second_run_id.clone(),
                delivery_token: format!("workflow-ack:{second_run_id}"),
            },
        ))
        .expect("second node ack should succeed");
    harness.with_app_mut(|app| {
        app.fan_out_output(
            session.id(),
            &provider_run_id,
            TerminalOutputKind::ProviderOutput,
            None,
            Vec::new(),
            b"```json\n{\"summary\":\"loop back\",\"output\":{\"message\":\"{\\\"ok\\\":true}\"}}\n```\n",
        );
    });
    let _ = harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("second node prompt should complete");

    let active_prompt = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should resolve")
            .active_prompt()
            .cloned()
            .expect("first node should be active again")
    });
    assert!(active_prompt.prompt().contains("Control mailbox:"));
    assert!(active_prompt
        .prompt()
        .contains("output.message is not valid JSON"));
    assert!(active_prompt
        .prompt()
        .contains("Treat the control mailbox as authoritative runtime feedback"));
    assert!(active_prompt.prompt().contains("Outgoing edge contracts:"));
    assert!(active_prompt
        .prompt()
        .contains(schema_path.to_string_lossy().as_ref()));
    assert!(!active_prompt
        .prompt()
        .contains("Control mailbox (daemon-managed):"));
}

#[test]
fn local_request_api_resumes_stopped_active_workflow_node_runs() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-resume", "worktree-resume"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let _attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "resume-client".to_string(),
                capability_level: ClientCapabilityLevel::InteractiveStructured,
            },
        ))
        .expect("attachment should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    let agent = harness.spawn_workflow_test_agent(session.id(), "resume-node");
    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("resume-flow".to_string()),
        }))
        .expect("workflow create should succeed")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };
    let node = harness.add_workflow_test_node(session.id(), workflow.id(), agent.id());
    let endpoint = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: node.id().to_string(),
                alias: Some("entry".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };
    let workflow_run = match harness
        .dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
            InvokeWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                prompt: Some("resume prompt".to_string()),
            },
        ))
        .expect("workflow invoke should succeed")
    {
        LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
        _ => panic!("unexpected local response"),
    };

    let cancelled = match harness
        .dispatch(LocalDaemonRequest::CancelWorkflowRun(
            CancelWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run.id().to_string(),
            },
        ))
        .expect("workflow run should stop")
    {
        LocalDaemonResponse::WorkflowRunCancelled { workflow_run, .. } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(
        cancelled.status(),
        crate::session::WorkflowRunStatus::Stopped
    );
    assert_eq!(
        harness.with_app(|app| {
            app.sessions()
                .get_session(session.id())
                .expect("session should resolve")
                .active_prompt()
                .expect("workflow prompt should be cancelling")
                .status()
        }),
        crate::session::PromptStatus::Cancelling
    );
    harness.with_app_mut(|app| {
        app.finalize_active_prompt_cancellation(session.id(), agent.id(), None)
            .expect("workflow cancellation should finalize");
    });
    assert!(harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should resolve")
            .active_prompt()
            .is_none()
    }));
    let stopped_run = harness.with_app(|app| {
        app.sessions()
            .resolve_workflow_run_ref(session.id(), workflow_run.id())
            .expect("workflow run should resolve after cancellation")
            .clone()
    });
    assert!(stopped_run.failure_events().iter().any(|event| {
        matches!(
            event.kind(),
            crate::session::WorkflowFailureKind::RunStopped
        ) && event
            .message()
            .contains("workflow node run was stopped before validated completion")
    }));

    let resumed = match harness
        .dispatch(LocalDaemonRequest::ResumeWorkflowRun(
            ResumeWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run.id().to_string(),
            },
        ))
        .expect("workflow run should resume")
    {
        LocalDaemonResponse::WorkflowRunResumed { workflow_run, .. } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    assert!(matches!(
        resumed.status(),
        crate::session::WorkflowRunStatus::Waiting
            | crate::session::WorkflowRunStatus::Running
            | crate::session::WorkflowRunStatus::Completed
    ));
    let active_prompt = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should resolve")
            .active_prompt()
            .cloned()
    });
    if let Some(active_prompt) = active_prompt {
        assert!(active_prompt.prompt().contains("resume prompt"));
    }
    let resumed_run = resumed
        .node_runs()
        .iter()
        .find(|node_run| node_run.id() == workflow_run.node_runs()[0].id())
        .expect("node run should remain");
    assert!(matches!(
        resumed_run.status(),
        crate::session::WorkflowNodeRunStatus::Ready
            | crate::session::WorkflowNodeRunStatus::Running
            | crate::session::WorkflowNodeRunStatus::Completed
    ));
    assert!(resumed_run
        .turn_envelope()
        .and_then(|envelope| envelope.rendered_prompt())
        .is_some());
}

#[test]
fn local_request_api_rejects_workflow_run_when_agent_lacks_required_control_capability() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-control", "worktree-control"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let unsupported_agent = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("unsupported-node".to_string()),
            provider: Some("dev-invalid-pty".to_string()),
            model: Some("default".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            machine_ref: None,
            worktree_placement: None,
        }))
        .expect("agent spawn should succeed")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };
    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("control-check".to_string()),
        }))
        .expect("workflow create should succeed")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };
    let node = harness.add_workflow_test_node(session.id(), workflow.id(), unsupported_agent.id());
    let endpoint = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: node.id().to_string(),
                alias: Some("entry".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("endpoint create should succeed")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };
    let error = harness
        .dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
            InvokeWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                prompt: Some("hello".to_string()),
            },
        ))
        .expect_err("workflow invoke should fail when controls are unsupported");
    assert!(matches!(
        error,
        DaemonError::WorkflowNodeControlUnsupported { operation, .. }
            if operation == "ack_workflow_turn"
    ));
}

#[test]
fn local_request_api_waits_for_all_join_inputs_before_scheduling_downstream_node() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    let entry_agent = harness.spawn_workflow_test_agent(session.id(), "entry");
    let branch_one_agent = harness.spawn_workflow_test_agent(session.id(), "branch-one");
    let branch_two_agent = harness.spawn_workflow_test_agent(session.id(), "branch-two");
    let join_agent = harness.spawn_workflow_test_agent(session.id(), "join");

    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("join".to_string()),
        }))
        .expect("workflow create should succeed")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };

    let entry_node = harness.add_workflow_test_node(session.id(), workflow.id(), entry_agent.id());
    let branch_one_node =
        harness.add_workflow_test_node(session.id(), workflow.id(), branch_one_agent.id());
    let branch_two_node =
        harness.add_workflow_test_node(session.id(), workflow.id(), branch_two_agent.id());
    let join_node = harness.add_workflow_test_node(session.id(), workflow.id(), join_agent.id());
    harness.add_workflow_test_edge(
        session.id(),
        workflow.id(),
        entry_node.id(),
        branch_one_node.id(),
    );
    harness.add_workflow_test_edge(
        session.id(),
        workflow.id(),
        entry_node.id(),
        branch_two_node.id(),
    );
    harness.add_workflow_test_edge(
        session.id(),
        workflow.id(),
        branch_one_node.id(),
        join_node.id(),
    );
    harness.add_workflow_test_edge(
        session.id(),
        workflow.id(),
        branch_two_node.id(),
        join_node.id(),
    );

    let endpoint = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: entry_node.id().to_string(),
                alias: Some("entry".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };

    let workflow_run = match harness
        .dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
            InvokeWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                prompt: Some("run the join drill".to_string()),
            },
        ))
        .expect("workflow invoke should succeed")
    {
        LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
        _ => panic!("unexpected local response"),
    };

    harness.complete_workflow_test_prompt(session.id(), "entry workflow prompt");
    let after_entry = harness.get_workflow_test_run(session.id(), workflow_run.id());
    assert_eq!(after_entry.node_runs().len(), 3);
    let session_after_entry = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should resolve after entry")
            .clone()
    });
    let active_branch_agents = [branch_one_agent.id(), branch_two_agent.id()]
        .into_iter()
        .filter(|agent_id| {
            session_after_entry
                .active_prompt_for_agent(agent_id)
                .is_some()
        })
        .collect::<Vec<_>>();
    let active_prompt_count = session_after_entry
        .prompt_states()
        .values()
        .filter(|state| state.active_prompt().is_some())
        .count();
    let queued_prompt_count = session_after_entry
        .prompt_states()
        .values()
        .map(|state| state.queued_prompts().len())
        .sum::<usize>();
    assert!(
        active_prompt_count >= 1,
        "expected at least one branch prompt to be active after entry completed"
    );
    assert_eq!(active_prompt_count + queued_prompt_count, 1);
    assert_eq!(active_branch_agents.len(), 1);
    assert_eq!(
        after_entry
            .node_runs()
            .iter()
            .filter(|node_run| {
                node_run.status() == WorkflowNodeRunStatus::BlockedOnWorkspaceClaim
            })
            .count(),
        1
    );

    harness.with_app_mut(|app| {
        app.complete_active_prompt(session.id(), active_branch_agents[0], None)
            .unwrap_or_else(|error| {
                panic!("first branch workflow prompt should complete: {error}")
            });
    });
    let after_first_branch = harness.get_workflow_test_run(session.id(), workflow_run.id());
    assert_eq!(after_first_branch.node_runs().len(), 3);
    assert!(after_first_branch
        .node_runs()
        .iter()
        .all(|node_run| node_run.node_id() != join_node.id()));
    let buffered_join_messages = after_first_branch
        .messages()
        .iter()
        .filter(|message| message.target_node_id() == join_node.id())
        .collect::<Vec<_>>();
    assert_eq!(buffered_join_messages.len(), 1);
    assert!(buffered_join_messages[0]
        .consumed_by_node_run_id()
        .is_none());
    let session_after_first_branch = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should resolve after first branch")
            .clone()
    });
    let remaining_active_branch_agents = [branch_one_agent.id(), branch_two_agent.id()]
        .into_iter()
        .filter(|agent_id| {
            session_after_first_branch
                .active_prompt_for_agent(agent_id)
                .is_some()
        })
        .collect::<Vec<_>>();
    assert_eq!(remaining_active_branch_agents.len(), 1);
    assert_eq!(session_after_first_branch.queued_prompts().len(), 0);

    harness.with_app_mut(|app| {
        app.complete_active_prompt(session.id(), remaining_active_branch_agents[0], None)
            .unwrap_or_else(|error| {
                panic!("second branch workflow prompt should complete: {error}")
            });
    });
    let after_second_branch = harness.get_workflow_test_run(session.id(), workflow_run.id());
    let join_runs = after_second_branch
        .node_runs()
        .iter()
        .filter(|node_run| node_run.node_id() == join_node.id())
        .collect::<Vec<_>>();
    assert_eq!(join_runs.len(), 1);
    let join_run = join_runs[0];
    let join_messages = after_second_branch
        .messages()
        .iter()
        .filter(|message| message.target_node_id() == join_node.id())
        .collect::<Vec<_>>();
    assert_eq!(join_messages.len(), 2);
    assert!(join_messages
        .iter()
        .all(|message| message.consumed_by_node_run_id() == Some(join_run.id())));

    harness.complete_workflow_test_prompt(session.id(), "join workflow prompt");
    let completed = harness.get_workflow_test_run(session.id(), workflow_run.id());
    assert_eq!(format!("{:?}", completed.status()), "Completed");
    assert_eq!(completed.node_runs().len(), 4);
}

#[test]
fn detaching_one_attachment_keeps_the_session_open_for_others() {
    let harness = LocalRouterTestHarness::new();
    let (session, _default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };

    let first = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("first attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let second = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-2".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("second attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let detached = match harness
        .dispatch(LocalDaemonRequest::DetachFromSession(
            DetachFromSessionRequest {
                attachment_id: first.id().to_string(),
            },
        ))
        .expect("detach should succeed")
    {
        LocalDaemonResponse::SessionDetached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let state = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("state request should succeed")
    {
        LocalDaemonResponse::SessionState { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(detached.id(), first.id());
    assert_eq!(state.status().to_string(), "created");
    assert_eq!(state.attachment_ids().len(), 1);
    assert!(state.has_attachment(second.id()));
    assert!(harness.with_app(|app| app.attachments().get_attachment(second.id()).is_ok()));
}

#[test]
fn focusing_another_agent_during_a_prompt_keeps_the_working_run_active() {
    let harness = LocalRouterTestHarness::new();
    let (session, default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let _default_run = harness.with_app_mut(|app| {
        app.launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(default_agent.id()),
        )
        .expect("default provider launch should succeed")
    });

    let spawned = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("reviewer".to_string()),
            provider: Some("claude-code".to_string()),
            model: None,
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            machine_ref: None,
            worktree_placement: None,
        }))
        .expect("spawn should succeed")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    let _focused_run = harness.with_app_mut(|app| {
        app.launch_provider(
            LaunchProviderRequest::new(session.id(), "dev-stub", "claude-code", "default", "opus")
                .with_agent_id(spawned.id()),
        )
        .expect("spawned provider launch should succeed")
    });

    let _ = harness
        .dispatch(LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session.id().to_string(),
            agent_id: default_agent.id().to_string(),
        }))
        .expect("focusing default agent should succeed");

    let started = harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: None,
            prompt: "keep streaming while focus changes\n".to_string(),
            attachments: Vec::new(),
        }))
        .expect("prompt should start");

    match started {
        LocalDaemonResponse::PromptSubmitted { outcome, .. } => match outcome {
            PromptSubmissionOutcome::Started { prompt } => {
                assert_eq!(prompt.target_agent_id(), default_agent.id());
            }
            _ => panic!("expected prompt to start immediately"),
        },
        _ => panic!("unexpected local response"),
    }

    let _ = harness
        .dispatch(LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session.id().to_string(),
            agent_id: spawned.id().to_string(),
        }))
        .expect("focusing spawned agent should succeed");

    let session_state = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("session state should load")
    {
        LocalDaemonResponse::SessionState { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(session_state.focused_agent_id(), Some(spawned.id()));
    assert_eq!(
        session_state.active_provider_run_id(),
        Some(_default_run.id())
    );

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut saw_output = false;
    while Instant::now() < deadline {
        let records = harness.with_app_mut(|app| {
            crate::app::provider_output::pump_terminal_output_for_attachment(
                app,
                session.id(),
                attachment.id(),
            )
            .expect("terminal output should keep pumping")
        });
        if !records.is_empty() {
            saw_output = true;
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    assert!(
        saw_output,
        "expected background agent output to continue while unfocused"
    );

    harness.with_app_mut(|app| {
        pump_active_prompt_outputs(app);
        crate::app::workflow_runtime::pump_workflow_watchdogs(app);
    });
    harness.with_app(|app| {
        let session_state = app
            .sessions()
            .get_session(session.id())
            .expect("session should still exist");
        assert_eq!(session_state.focused_agent_id(), Some(spawned.id()));
        assert_eq!(
            session_state.active_provider_run_id(),
            Some(_default_run.id())
        );
        assert!(
            session_state
                .active_prompt_for_agent(default_agent.id())
                .is_some(),
            "background prompt should remain owned by the original agent while unfocused"
        );
    });
}

#[test]
fn spawning_agent_during_active_prompt_keeps_snapshot_on_working_run() {
    let harness = LocalRouterTestHarness::new();
    let (session, default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attachment should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    let default_run = harness.with_app_mut(|app| {
        app.launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(default_agent.id()),
        )
        .expect("provider run should launch")
    });

    harness
        .with_app_mut(|app| {
            app.submit_prompt(
                session.id(),
                attachment.id(),
                Some(default_agent.id()),
                "keep working\n",
                Vec::new(),
            )
        })
        .expect("prompt should start");
    let spawned = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("observer".to_string()),
            provider: Some("claude-code".to_string()),
            model: None,
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            machine_ref: None,
            worktree_placement: None,
        }))
        .expect("spawn should succeed")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    let session_state = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("session state should load")
    {
        LocalDaemonResponse::SessionState { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(session_state.focused_agent_id(), Some(spawned.id()));
    assert_eq!(
        session_state.active_provider_run_id(),
        Some(default_run.id()),
        "snapshots must keep the still-running provider visible for recovery and stream routing"
    );
}

#[test]
fn terminal_output_drain_survives_missing_focused_provider_run() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-1",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    app.terminal_mut().fan_out_output(
        session.id(),
        "provider-run-stale",
        Some(default_agent.id()),
        crate::terminal::TerminalOutputKind::ProviderOutput,
        None,
        vec![attachment.id().to_string()],
        b"late output\n",
    );

    let records = crate::app::provider_output::pump_terminal_output_for_attachment(
        &mut app,
        session.id(),
        attachment.id(),
    )
    .expect("draining buffered output should not require an active focused provider run");

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].bytes, b"late output\n");
}

#[test]
fn terminal_output_drain_streams_parallel_agent_prompts_for_same_attachment() {
    let harness = LocalRouterTestHarness::new();
    let (session, default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attachment should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    let spawned = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("parallel".to_string()),
            provider: Some("dev-stub".to_string()),
            model: Some("claude-code".to_string()),
            effort: Some("default".to_string()),
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            machine_ref: None,
            worktree_placement: None,
        }))
        .expect("spawn should succeed")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    let (default_run_id, spawned_run_id) = harness.with_app_mut(|app| {
        (
            launch_slow_structured_run(app, session.id(), default_agent.id()),
            launch_slow_structured_run(app, session.id(), spawned.id()),
        )
    });

    for agent_id in [default_agent.id(), spawned.id()] {
        match harness
            .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                target_agent_id: Some(agent_id.to_string()),
                prompt: format!("parallel prompt for {agent_id}\n"),
                attachments: Vec::new(),
            }))
            .expect("prompt should start")
        {
            LocalDaemonResponse::PromptSubmitted {
                outcome: PromptSubmissionOutcome::Started { .. },
                ..
            } => {}
            _ => panic!("unexpected local response"),
        }
    }

    harness.with_app_mut(|app| {
        for (provider_run_id, agent_id) in [
            (default_run_id.clone(), default_agent.id().to_string()),
            (spawned_run_id.clone(), spawned.id().to_string()),
        ] {
            app.providers_mut()
                .push_finished_structured_output_poll_for_test(
                    provider_run_id,
                    Ok(Some(ProviderPromptSignalBatch {
                        chunks: vec![ProviderPromptChunk {
                            kind: TerminalOutputKind::ProviderOutput,
                            merge_key: Some(format!("parallel-{agent_id}")),
                            bytes: format!("parallel output for {agent_id}\n").into_bytes(),
                        }],
                        ..ProviderPromptSignalBatch::default()
                    })),
                );
        }
    });

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut seen_agents = std::collections::BTreeSet::new();
    while Instant::now() < deadline && seen_agents.len() < 2 {
        let records = harness.with_app_mut(|app| {
            crate::app::provider_output::pump_terminal_output_for_attachment(
                app,
                session.id(),
                attachment.id(),
            )
            .expect("terminal output should keep pumping")
        });
        for record in records {
            if let Some(agent_id) = record.agent_id {
                seen_agents.insert(agent_id);
            }
        }
        thread::sleep(Duration::from_millis(10));
    }

    assert!(
        seen_agents.contains(default_agent.id()) && seen_agents.contains(spawned.id()),
        "expected output from both active agent prompts, saw {:?}",
        seen_agents
    );
}

#[test]
fn attaching_the_same_client_replaces_its_stale_attachment() {
    let harness = LocalRouterTestHarness::new();
    let (session, _default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };

    let first = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("first attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let second = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("second attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let state = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("state request should succeed")
    {
        LocalDaemonResponse::SessionState { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    assert_ne!(first.id(), second.id());
    assert_eq!(state.attachment_ids().len(), 1);
    assert!(state.has_attachment(second.id()));
    assert!(harness.with_app(|app| app.attachments().get_attachment(first.id()).is_err()));
}

#[test]
fn local_request_api_auto_launches_provider_run_for_prompt() {
    let harness = LocalRouterTestHarness::new();
    let (session, _default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let prompt_agent = harness.spawn_workflow_test_agent(session.id(), "prompt-agent");
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let response = harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(prompt_agent.id().to_string()),
            prompt: "whoami".to_string(),
            attachments: Vec::new(),
        }))
        .expect("prompt submit should auto-launch a provider run");

    match response {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Started { prompt },
            session,
            ..
        } => {
            assert_eq!(prompt.status(), crate::session::PromptStatus::Running);
            assert!(session.active_provider_run_id().is_some());
            assert!(session.active_prompt_for_agent(prompt_agent.id()).is_some());
        }
        other => panic!("unexpected local response: {other:?}"),
    }
}

#[test]
fn direct_prompt_completion_resolves_unfocused_single_active_agent() {
    let harness = LocalRouterTestHarness::new();
    let (session, default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let prompt_agent = harness.spawn_workflow_test_agent(session.id(), "prompt-agent");
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    match harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(prompt_agent.id().to_string()),
            prompt: "whoami".to_string(),
            attachments: Vec::new(),
        }))
        .expect("prompt submit should start")
    {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Started { prompt },
            ..
        } => assert_eq!(prompt.target_agent_id(), prompt_agent.id()),
        other => panic!("unexpected local response: {other:?}"),
    }

    let _ = harness
        .dispatch(LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session.id().to_string(),
            agent_id: default_agent.id().to_string(),
        }))
        .expect("focus should move to the idle default agent");

    match harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("completion should resolve the single active agent")
    {
        LocalDaemonResponse::PromptCompleted { completion } => {
            assert_eq!(completion.completed.target_agent_id(), prompt_agent.id());
            assert!(completion.started_next.is_none());
        }
        other => panic!("unexpected local response: {other:?}"),
    }

    let session_state = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should still exist")
            .clone()
    });
    assert_eq!(session_state.focused_agent_id(), Some(default_agent.id()));
    assert!(session_state
        .active_prompt_for_agent(prompt_agent.id())
        .is_none());
}

#[test]
fn direct_prompt_cancel_resolves_unfocused_single_active_agent() {
    let harness = LocalRouterTestHarness::new();
    let (session, default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let prompt_agent = harness.spawn_workflow_test_agent(session.id(), "prompt-agent");
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    match harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(prompt_agent.id().to_string()),
            prompt: "whoami".to_string(),
            attachments: Vec::new(),
        }))
        .expect("prompt submit should start")
    {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Started { prompt },
            ..
        } => assert_eq!(prompt.target_agent_id(), prompt_agent.id()),
        other => panic!("unexpected local response: {other:?}"),
    }

    let _ = harness
        .dispatch(LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session.id().to_string(),
            agent_id: default_agent.id().to_string(),
        }))
        .expect("focus should move to the idle default agent");

    match harness
        .dispatch(LocalDaemonRequest::CancelActivePrompt(
            CancelActivePromptRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
            },
        ))
        .expect("cancel should resolve the single active agent")
    {
        LocalDaemonResponse::PromptCancelled { cancellation } => {
            assert_eq!(cancellation.prompt.target_agent_id(), prompt_agent.id());
            assert!(cancellation.started_next.is_none());
        }
        other => panic!("unexpected local response: {other:?}"),
    }

    let session_state = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should still exist")
            .clone()
    });
    assert_eq!(session_state.focused_agent_id(), Some(default_agent.id()));
    assert_eq!(
        session_state
            .active_prompt_for_agent(prompt_agent.id())
            .map(|prompt| prompt.status()),
        Some(crate::session::PromptStatus::Cancelling)
    );
}

#[test]
fn prompt_idle_fallback_completes_after_recorded_completion_without_response_text() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-completion-fallback",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(default_agent.id()),
        )
        .expect("provider run should launch");
    let _started = app
        .submit_prompt(
            session.id(),
            attachment.id(),
            Some(default_agent.id()),
            "tool only\n",
            Vec::new(),
        )
        .expect("prompt should start");

    crate::transport::flow_control::mark_prompt_completion_recorded(&mut app, run.id());
    if let Some(state) = app.prompt_activity.write().get_mut(run.id()) {
        state.last_output_at =
            Some(Instant::now() - app.prompt_idle_timeout - Duration::from_millis(1));
    } else {
        panic!("prompt activity should exist for the active run");
    }

    crate::transport::flow_control::maybe_complete_active_prompt(&mut app, session.id(), run.id())
        .expect("idle fallback should settle the active prompt");

    let session_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert!(session_state
        .active_prompt_for_agent(default_agent.id())
        .is_none());
}

#[test]
fn local_request_api_rejects_invalid_provider_adapter() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };

    let error = harness
        .dispatch(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session.id().to_string(),
                agent_id: None,
                adapter_key: "missing-adapter".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
            },
        ))
        .expect_err("unknown adapters should be rejected");

    match error {
        DaemonError::ProviderAdapterNotFound { adapter_key } => {
            assert_eq!(adapter_key, "missing-adapter")
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn local_request_api_exposes_queue_config_and_notices() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let a = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-a".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    let b = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-b".to_string(),
                capability_level: ClientCapabilityLevel::InteractiveStructured,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    harness.with_app_mut(|app| {
        app.launch_provider(LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "claude-code",
            "default",
            "sonnet",
        ))
        .expect("provider launch should succeed");
    });

    let first = harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: a.id().to_string(),
            target_agent_id: None,
            prompt: "first".to_string(),
            attachments: Vec::new(),
        }))
        .expect("first prompt should start");
    let second = harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: b.id().to_string(),
            target_agent_id: None,
            prompt: "second".to_string(),
            attachments: Vec::new(),
        }))
        .expect("second prompt should queue");
    let config = harness
        .dispatch(LocalDaemonRequest::UpdateSessionConfig(
            UpdateSessionConfigRequest {
                session_id: session.id().to_string(),
                attachment_id: a.id().to_string(),
                values: BTreeMap::from([("theme".to_string(), "compact".to_string())]),
                requires_idle: false,
            },
        ))
        .expect("config update should succeed");

    match first {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Started { .. },
            session,
            ..
        } => {
            assert!(session.active_prompt().is_some());
        }
        _ => panic!("unexpected first prompt response"),
    }
    match second {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Queued { .. },
            session,
            ..
        } => {
            assert_eq!(session.queued_prompts().len(), 1);
        }
        _ => panic!("unexpected second prompt response"),
    }
    match config {
        LocalDaemonResponse::SessionConfigUpdated { config, session } => {
            assert_eq!(config.version(), 1);
            assert_eq!(session.config_state().version(), 1);
        }
        _ => panic!("unexpected config response"),
    }

    let notices = harness
        .dispatch(LocalDaemonRequest::PollRuntimeNotices(
            PollRuntimeNoticesRequest {
                session_id: session.id().to_string(),
                attachment_id: b.id().to_string(),
            },
        ))
        .expect("notice polling should succeed");
    match notices {
        LocalDaemonResponse::RuntimeNotices { notices } => assert!(!notices.is_empty()),
        _ => panic!("unexpected notices response"),
    }

    let state = harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("state request should succeed");
    match state {
        LocalDaemonResponse::SessionState { session, .. } => {
            assert_eq!(session.queued_prompts().len(), 1);
            assert_eq!(session.config_state().version(), 1);
        }
        _ => panic!("unexpected state response"),
    }

    let completed = harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("complete prompt should succeed");
    match completed {
        LocalDaemonResponse::PromptCompleted { completion } => {
            assert!(completion.started_next.is_some())
        }
        _ => panic!("unexpected completion response"),
    }
}

#[test]
fn local_request_api_can_cancel_an_active_prompt() {
    let harness = LocalRouterTestHarness::new();

    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };

    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-a".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let prompt_agent = harness.spawn_workflow_test_agent(session.id(), "prompt-agent");

    let _ = harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(prompt_agent.id().to_string()),
            prompt: "first prompt\n".to_string(),
            attachments: Vec::new(),
        }))
        .expect("first prompt should start");
    let _ = harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(prompt_agent.id().to_string()),
            prompt: "second prompt\n".to_string(),
            attachments: Vec::new(),
        }))
        .expect("second prompt should queue");

    let response = harness
        .dispatch(LocalDaemonRequest::CancelActivePrompt(
            CancelActivePromptRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
            },
        ))
        .expect("cancel should succeed");

    match response {
        LocalDaemonResponse::PromptCancelled { cancellation } => {
            assert_eq!(
                cancellation.prompt.status(),
                crate::session::PromptStatus::Cancelling
            );
            assert!(cancellation.started_next.is_none());
        }
        _ => panic!("unexpected local response"),
    }
}

#[test]
fn local_request_api_runs_shell_command_capability() {
    let worktree_root = std::env::temp_dir().join("arroba-shell-local-api-test");
    std::fs::create_dir_all(&worktree_root).expect("worktree dir should exist");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-shell".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let response = harness
        .dispatch(LocalDaemonRequest::RunShellCommand(
            RunShellCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                command: "/bin/sh".to_string(),
                args: vec!["-lc".to_string(), "printf capability".to_string()],
                working_directory: None,
                timeout_ms: None,
            },
        ))
        .expect("shell capability should succeed");

    match response {
        LocalDaemonResponse::ShellCommandCompleted { result } => {
            assert_eq!(result.exit_code, 0);
            assert_eq!(result.stdout, "capability");
        }
        _ => panic!("unexpected shell response"),
    }
}

#[test]
fn local_request_api_rejects_shell_command_for_unauthorized_attachment() {
    let worktree_root = std::env::temp_dir().join("arroba-shell-local-api-denied-test");
    std::fs::create_dir_all(&worktree_root).expect("worktree dir should exist");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-automation".to_string(),
                capability_level: ClientCapabilityLevel::AutomationOnly,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let error = harness
        .dispatch(LocalDaemonRequest::RunShellCommand(
            RunShellCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                command: "/bin/sh".to_string(),
                args: vec!["-lc".to_string(), "printf denied".to_string()],
                working_directory: None,
                timeout_ms: None,
            },
        ))
        .expect_err("automation-only attachment should not run shell commands");

    match error {
        DaemonError::AttachmentCapabilityDenied { session_id, .. } => {
            assert_eq!(session_id, session.id());
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn local_request_api_rejects_file_capability_for_unauthorized_attachment() {
    let worktree_root = std::env::temp_dir().join("arroba-file-local-api-denied-test");
    let _ = std::fs::remove_dir_all(&worktree_root);
    std::fs::create_dir_all(&worktree_root).expect("worktree dir should exist");
    std::fs::write(worktree_root.join("notes.txt"), "hello").expect("file should exist");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-automation".to_string(),
                capability_level: ClientCapabilityLevel::AutomationOnly,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let error = harness
        .dispatch(LocalDaemonRequest::ReadFile(ReadFileCapabilityRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            path: worktree_root.join("notes.txt"),
        }))
        .expect_err("automation-only attachment should not read files");

    match error {
        DaemonError::AttachmentCapabilityDenied { session_id, .. } => {
            assert_eq!(session_id, session.id());
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn local_request_api_reads_directory_tree_file_and_git_status() {
    let worktree_root = std::env::temp_dir().join("arroba-capability-local-api-test");
    let _ = std::fs::remove_dir_all(&worktree_root);
    std::fs::create_dir_all(worktree_root.join("src")).expect("worktree should exist");
    std::fs::write(worktree_root.join("README.md"), "hello").expect("file should exist");
    std::fs::write(worktree_root.join("src/lib.rs"), "before").expect("file should exist");
    std::process::Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(&worktree_root)
        .output()
        .expect("git init should work");

    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-capability".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let tree = harness
        .dispatch(LocalDaemonRequest::ReadDirectoryTree(
            ReadDirectoryTreeCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                path: None,
                max_depth: 2,
            },
        ))
        .expect("tree read should succeed");
    let file = harness
        .dispatch(LocalDaemonRequest::ReadFile(ReadFileCapabilityRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            path: worktree_root.join("src/lib.rs"),
        }))
        .expect("file read should succeed");
    let edit = harness
        .dispatch(LocalDaemonRequest::EditFile(EditFileCapabilityRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            path: worktree_root.join("src/lib.rs"),
            contents: "after".to_string(),
        }))
        .expect("file edit should succeed");
    let git = harness
        .dispatch(LocalDaemonRequest::InspectGit(
            InspectGitCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                working_directory: None,
            },
        ))
        .expect("git inspect should succeed");

    match tree {
        LocalDaemonResponse::DirectoryTreeRead { result } => {
            assert!(result
                .entries
                .iter()
                .any(|entry| entry.relative_path == "README.md"));
        }
        _ => panic!("unexpected tree response"),
    }
    match file {
        LocalDaemonResponse::FileRead { result } => assert_eq!(result.contents, "before"),
        _ => panic!("unexpected file response"),
    }
    match edit {
        LocalDaemonResponse::FileEdited { result } => {
            assert_eq!(result.bytes_written, 5);
            assert_eq!(result.old_size, 6);
            assert_eq!(result.new_size, 5);
            assert!(result.changed);
        }
        _ => panic!("unexpected edit response"),
    }
    match git {
        LocalDaemonResponse::GitInspected { result } => assert!(result.status.contains("main")),
        _ => panic!("unexpected git response"),
    }
}

#[test]
fn local_request_api_inspects_workspace_git_overview() {
    let worktree_root = std::env::temp_dir().join("arroba-workspace-git-overview-test");
    let _ = std::fs::remove_dir_all(&worktree_root);
    std::fs::create_dir_all(&worktree_root).expect("worktree should exist");
    std::fs::write(worktree_root.join("README.md"), "hello\n").expect("file should exist");
    run_test_git(&worktree_root, &["init", "-b", "main"]);
    run_test_git(
        &worktree_root,
        &["config", "user.email", "agent@example.com"],
    );
    run_test_git(&worktree_root, &["config", "user.name", "Agent"]);
    run_test_git(&worktree_root, &["add", "README.md"]);
    run_test_git(&worktree_root, &["commit", "-m", "seed"]);
    std::fs::write(worktree_root.join("README.md"), "hello\nworld\n").expect("file should update");
    std::fs::write(worktree_root.join("new.txt"), "new\n").expect("new file should exist");

    let harness = LocalRouterTestHarness::new();
    let response = harness
        .dispatch(LocalDaemonRequest::GetWorkspaceGitOverview(
            GetWorkspaceGitOverviewRequest {
                workspace_id: worktree_root.display().to_string(),
                worktree_id: worktree_root.display().to_string(),
                compare_ref: Some("HEAD".to_string()),
            },
        ))
        .expect("workspace git overview should succeed");

    match response {
        LocalDaemonResponse::WorkspaceGitOverview { overview } => {
            assert_eq!(overview.branch.as_deref(), Some("main"));
            assert_eq!(overview.compare_ref, "HEAD");
            assert_eq!(overview.totals.files, 2);
            assert_eq!(overview.totals.additions, 2);
            assert!(overview
                .compare_refs
                .iter()
                .any(|reference| reference.name == "HEAD" && reference.selected));
            assert!(overview
                .files
                .iter()
                .any(|file| file.path == "README.md" && file.additions == 1));
            assert!(overview
                .files
                .iter()
                .any(|file| file.path == "new.txt" && file.status == "untracked"));
        }
        _ => panic!("unexpected local response"),
    }
}

#[test]
fn local_request_api_lists_workspace_repo_files() {
    let worktree_root = std::env::temp_dir().join("arroba-workspace-files-test");
    let _ = std::fs::remove_dir_all(&worktree_root);
    std::fs::create_dir_all(worktree_root.join("src/app")).expect("worktree should exist");
    std::fs::write(worktree_root.join("README.md"), "hello\n").expect("file should exist");
    std::fs::write(worktree_root.join("src/app/main.rs"), "fn main() {}\n")
        .expect("file should exist");
    run_test_git(&worktree_root, &["init", "-b", "main"]);
    run_test_git(
        &worktree_root,
        &["config", "user.email", "agent@example.com"],
    );
    run_test_git(&worktree_root, &["config", "user.name", "Agent"]);
    run_test_git(&worktree_root, &["add", "."]);
    run_test_git(&worktree_root, &["commit", "-m", "seed"]);
    std::fs::write(
        worktree_root.join("src/app/main.rs"),
        "fn main() {}\nfn changed() {}\n",
    )
    .expect("file should update");

    let harness = LocalRouterTestHarness::new();
    let root_response = harness
        .dispatch(LocalDaemonRequest::ListWorkspaceFiles(
            ListWorkspaceFilesRequest {
                workspace_id: worktree_root.display().to_string(),
                worktree_id: worktree_root.display().to_string(),
                path_prefix: None,
                compare_ref: Some("HEAD".to_string()),
                limit: None,
            },
        ))
        .expect("workspace files should list");

    match root_response {
        LocalDaemonResponse::WorkspaceFilesListed { listing } => {
            assert_eq!(listing.path_prefix, "");
            assert_eq!(listing.compare_ref, "HEAD");
            assert_eq!(listing.total_entries, 2);
            assert!(!listing.truncated);
            assert!(listing
                .entries
                .iter()
                .any(|entry| entry.name == "src" && entry.kind == "directory" && entry.changed));
        }
        _ => panic!("unexpected local response"),
    }

    let nested_response = harness
        .dispatch(LocalDaemonRequest::ListWorkspaceFiles(
            ListWorkspaceFilesRequest {
                workspace_id: worktree_root.display().to_string(),
                worktree_id: worktree_root.display().to_string(),
                path_prefix: Some("src/app".to_string()),
                compare_ref: Some("HEAD".to_string()),
                limit: None,
            },
        ))
        .expect("workspace nested files should list");

    match nested_response {
        LocalDaemonResponse::WorkspaceFilesListed { listing } => {
            assert_eq!(listing.path_prefix, "src/app");
            assert_eq!(listing.compare_ref, "HEAD");
            assert_eq!(listing.total_entries, 1);
            assert!(!listing.truncated);
            assert!(listing.entries.iter().any(|entry| {
                entry.name == "main.rs"
                    && entry.kind == "file"
                    && entry.status.as_deref() == Some("modified")
                    && entry.additions == 1
            }));
        }
        _ => panic!("unexpected local response"),
    }

    let limited_response = harness
        .dispatch(LocalDaemonRequest::ListWorkspaceFiles(
            ListWorkspaceFilesRequest {
                workspace_id: worktree_root.display().to_string(),
                worktree_id: worktree_root.display().to_string(),
                path_prefix: None,
                compare_ref: Some("HEAD".to_string()),
                limit: Some(1),
            },
        ))
        .expect("limited workspace files should list");

    match limited_response {
        LocalDaemonResponse::WorkspaceFilesListed { listing } => {
            assert_eq!(listing.total_entries, 2);
            assert!(listing.truncated);
            assert_eq!(listing.entries.len(), 1);
        }
        _ => panic!("unexpected local response"),
    }
}

#[test]
fn local_request_api_reads_workspace_file_content() {
    let worktree_root = std::env::temp_dir().join("arroba-workspace-file-content-test");
    let _ = std::fs::remove_dir_all(&worktree_root);
    std::fs::create_dir_all(worktree_root.join("src/app")).expect("worktree should exist");
    std::fs::write(worktree_root.join("README.md"), "# hello\n").expect("file should exist");
    std::fs::write(worktree_root.join("src/app/main.rs"), "fn main() {}\n")
        .expect("file should exist");
    run_test_git(&worktree_root, &["init", "-b", "main"]);
    run_test_git(
        &worktree_root,
        &["config", "user.email", "agent@example.com"],
    );
    run_test_git(&worktree_root, &["config", "user.name", "Agent"]);
    run_test_git(&worktree_root, &["add", "."]);
    run_test_git(&worktree_root, &["commit", "-m", "seed"]);
    std::fs::write(
        worktree_root.join("src/app/main.rs"),
        "fn main() {}\nfn changed() {}\n",
    )
    .expect("file should update");

    let harness = LocalRouterTestHarness::new();
    let response = harness
        .dispatch(LocalDaemonRequest::GetWorkspaceFileContent(
            GetWorkspaceFileContentRequest {
                workspace_id: worktree_root.display().to_string(),
                worktree_id: worktree_root.display().to_string(),
                path: "src/app/main.rs".to_string(),
                compare_ref: Some("HEAD".to_string()),
                known_fingerprint: None,
                max_bytes: None,
            },
        ))
        .expect("workspace file content should load");

    let fingerprint = match response {
        LocalDaemonResponse::WorkspaceFileContent { content } => {
            assert_eq!(content.path, "src/app/main.rs");
            assert_eq!(content.name, "main.rs");
            assert_eq!(content.language, "rust");
            assert_eq!(content.encoding, "utf-8");
            assert_eq!(
                content.content_text.as_deref(),
                Some("fn main() {}\nfn changed() {}\n")
            );
            assert_eq!(content.compare_ref, "HEAD");
            assert_eq!(content.status.as_deref(), Some("modified"));
            assert_eq!(content.additions, 1);
            assert!(!content.truncated);
            assert!(content.sha256.is_some());
            content.fingerprint
        }
        _ => panic!("unexpected local response"),
    };

    let not_modified_response = harness
        .dispatch(LocalDaemonRequest::GetWorkspaceFileContent(
            GetWorkspaceFileContentRequest {
                workspace_id: worktree_root.display().to_string(),
                worktree_id: worktree_root.display().to_string(),
                path: "src/app/main.rs".to_string(),
                compare_ref: Some("HEAD".to_string()),
                known_fingerprint: Some(fingerprint.clone()),
                max_bytes: None,
            },
        ))
        .expect("workspace file content fingerprint should be honored");
    match not_modified_response {
        LocalDaemonResponse::WorkspaceFileContentNotModified {
            path,
            fingerprint: response_fingerprint,
            ..
        } => {
            assert_eq!(path, "src/app/main.rs");
            assert_eq!(response_fingerprint, fingerprint);
        }
        _ => panic!("unexpected local response"),
    }

    let truncated_response = harness
        .dispatch(LocalDaemonRequest::GetWorkspaceFileContent(
            GetWorkspaceFileContentRequest {
                workspace_id: worktree_root.display().to_string(),
                worktree_id: worktree_root.display().to_string(),
                path: "src/app/main.rs".to_string(),
                compare_ref: Some("HEAD".to_string()),
                known_fingerprint: None,
                max_bytes: Some(5),
            },
        ))
        .expect("workspace file content should truncate");
    match truncated_response {
        LocalDaemonResponse::WorkspaceFileContent { content } => {
            assert!(content.truncated);
            assert_eq!(content.content_text.as_deref(), Some("fn ma"));
        }
        _ => panic!("unexpected local response"),
    }
}

#[test]
fn local_request_api_commits_workspace_changes() {
    let worktree_root = std::env::temp_dir().join("arroba-workspace-commit-test");
    let _ = std::fs::remove_dir_all(&worktree_root);
    std::fs::create_dir_all(&worktree_root).expect("worktree should exist");
    std::fs::write(worktree_root.join("README.md"), "hello\n").expect("file should exist");
    run_test_git(&worktree_root, &["init", "-b", "main"]);
    run_test_git(
        &worktree_root,
        &["config", "user.email", "agent@example.com"],
    );
    run_test_git(&worktree_root, &["config", "user.name", "Agent"]);
    run_test_git(&worktree_root, &["add", "."]);
    run_test_git(&worktree_root, &["commit", "-m", "seed"]);
    std::fs::write(worktree_root.join("README.md"), "hello\nworld\n").expect("file should update");

    let harness = LocalRouterTestHarness::new();
    let response = harness
        .dispatch(LocalDaemonRequest::CommitWorkspaceChanges(
            CommitWorkspaceChangesRequest {
                workspace_id: worktree_root.display().to_string(),
                worktree_id: worktree_root.display().to_string(),
                message: "Update README".to_string(),
            },
        ))
        .expect("workspace commit should succeed");

    match response {
        LocalDaemonResponse::WorkspaceGitActionCompleted { result } => {
            assert_eq!(result.action, "commit");
            assert!(result.commit_sha.is_some());
        }
        _ => panic!("unexpected local response"),
    }
    let subject = git_test_output(&worktree_root, &["log", "-1", "--pretty=%s"]);
    assert_eq!(subject.trim(), "Update README");
    assert_eq!(
        git_test_output(&worktree_root, &["status", "--porcelain"]).trim(),
        ""
    );
}

#[test]
fn local_request_api_push_without_upstream_fails_loudly() {
    let worktree_root = std::env::temp_dir().join("arroba-workspace-push-no-upstream-test");
    let _ = std::fs::remove_dir_all(&worktree_root);
    std::fs::create_dir_all(&worktree_root).expect("worktree should exist");
    std::fs::write(worktree_root.join("README.md"), "hello\n").expect("file should exist");
    run_test_git(&worktree_root, &["init", "-b", "main"]);
    run_test_git(
        &worktree_root,
        &["config", "user.email", "agent@example.com"],
    );
    run_test_git(&worktree_root, &["config", "user.name", "Agent"]);
    run_test_git(&worktree_root, &["add", "."]);
    run_test_git(&worktree_root, &["commit", "-m", "seed"]);

    let harness = LocalRouterTestHarness::new();
    let error = harness
        .dispatch(LocalDaemonRequest::PushWorkspaceBranch(
            PushWorkspaceBranchRequest {
                workspace_id: worktree_root.display().to_string(),
                worktree_id: worktree_root.display().to_string(),
                force_with_lease: false,
            },
        ))
        .expect_err("push without upstream should fail");
    assert!(error.to_string().contains("no upstream"));
}

fn run_test_git(cwd: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git should run");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_test_output(cwd: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git should run");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn local_request_api_rejects_conflicting_workspace_write_claims() {
    let worktree_root = std::env::temp_dir().join("arroba-workspace-claim-local-api-test");
    let _ = std::fs::remove_dir_all(&worktree_root);
    std::fs::create_dir_all(worktree_root.join("src")).expect("worktree should exist");
    std::fs::write(worktree_root.join("src/lib.rs"), "before").expect("file should exist");

    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-workspace-claim".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let _claim = harness.with_app_mut(|app| {
        app.workspace_coordinator()
            .acquire_worktree_write_claim(
                session.workspace_id().to_string(),
                worktree_root.display().to_string(),
                "other-session",
                Some("other-attachment".to_string()),
                "file_edit",
            )
            .expect("existing claim should acquire")
    });

    let health = harness
        .dispatch(LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest))
        .expect("health should be available while claim is active");
    match health {
        LocalDaemonResponse::DaemonHealth { projection } => {
            assert_eq!(
                projection
                    .workspace_coordination
                    .active_operation_claims
                    .len(),
                1
            );
        }
        _ => panic!("unexpected health response"),
    }

    let error = harness
        .dispatch(LocalDaemonRequest::EditFile(EditFileCapabilityRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            path: worktree_root.join("src/lib.rs"),
            contents: "after".to_string(),
        }))
        .expect_err("conflicting write should be rejected");

    match error {
        DaemonError::WorkspaceClaimConflict {
            requested_session_id,
            existing_session_id,
            ..
        } => {
            assert_eq!(requested_session_id, session.id());
            assert_eq!(existing_session_id, "other-session");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn workflow_node_dispatch_blocks_and_retries_on_workspace_claim_release() {
    let harness = LocalRouterTestHarness::new();
    let (interactive_session, interactive_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-shared"),
        ))
        .expect("interactive session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let interactive_attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: interactive_session.id().to_string(),
                client_id: "client-workflow-claim-owner".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("interactive attachment should join")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    harness.with_app_mut(|app| {
        app.launch_provider(
            LaunchProviderRequest::new(
                interactive_session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "default",
            )
            .with_agent_id(interactive_agent.id()),
        )
        .expect("interactive provider run should launch");
    });
    match harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: interactive_session.id().to_string(),
            attachment_id: interactive_attachment.id().to_string(),
            target_agent_id: Some(interactive_agent.id().to_string()),
            prompt: "hold the worktree".to_string(),
            attachments: Vec::new(),
        }))
        .expect("interactive prompt should start")
    {
        LocalDaemonResponse::PromptSubmitted { outcome, .. } => match outcome {
            PromptSubmissionOutcome::Started { .. } => {}
            _ => panic!("expected interactive prompt to start"),
        },
        _ => panic!("unexpected local response"),
    }

    let workflow_session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-shared"),
        ))
        .expect("workflow session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let workflow_agent = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: workflow_session.id().to_string(),
            alias: Some("workflow-worker".to_string()),
            provider: Some("dev-stub".to_string()),
            model: Some("default".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            machine_ref: None,
            worktree_placement: None,
        }))
        .expect("workflow agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };
    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: workflow_session.id().to_string(),
            alias: Some("blocked".to_string()),
        }))
        .expect("workflow should be created")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };
    let node = match harness
        .dispatch(LocalDaemonRequest::AddWorkflowNode(
            AddWorkflowNodeRequest {
                session_id: workflow_session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                agent_id: workflow_agent.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        _ => panic!("unexpected local response"),
    };
    let endpoint = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: workflow_session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: node.id().to_string(),
                alias: Some("entry".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };
    let blocked_run = match harness
        .dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
            InvokeWorkflowEndpointRequest {
                session_id: workflow_session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                prompt: Some("background work".to_string()),
            },
        ))
        .expect("workflow invoke should block instead of fail")
    {
        LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(
        blocked_run.status(),
        crate::session::WorkflowRunStatus::Waiting
    );
    assert_eq!(
        blocked_run.node_runs()[0].status(),
        WorkflowNodeRunStatus::BlockedOnWorkspaceClaim
    );

    match harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: interactive_session.id().to_string(),
        }))
        .expect("interactive prompt should complete")
    {
        LocalDaemonResponse::PromptCompleted { .. } => {}
        _ => panic!("unexpected local response"),
    }

    let deadline = Instant::now() + Duration::from_secs(2);
    let retried_run = loop {
        let workflow_run = match harness
            .dispatch(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
                session_id: workflow_session.id().to_string(),
                workflow_run_ref: blocked_run.id().to_string(),
            }))
            .expect("workflow run should resolve after retry")
        {
            LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
            _ => panic!("unexpected local response"),
        };
        if workflow_run.status() == crate::session::WorkflowRunStatus::Running
            || Instant::now() >= deadline
        {
            break workflow_run;
        }
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        retried_run.status(),
        crate::session::WorkflowRunStatus::Running
    );
    assert_eq!(
        retried_run.node_runs()[0].status(),
        WorkflowNodeRunStatus::Running
    );
    assert_eq!(
        retried_run.active_node_run_id(),
        Some(retried_run.node_runs()[0].id())
    );
}

#[test]
fn local_request_api_returns_structured_screenshot_unavailable_result() {
    let _guard = crate::env_lock::lock();
    std::env::set_var("ARROBA_SCREENSHOT_DISABLE", "1");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", std::env::temp_dir().display().to_string()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-screenshot".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let response = harness
        .dispatch(LocalDaemonRequest::CaptureScreenshot(
            CaptureScreenshotCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
            },
        ))
        .expect("screenshot request should succeed with unavailable result");
    std::env::remove_var("ARROBA_SCREENSHOT_DISABLE");

    match response {
        LocalDaemonResponse::ScreenshotCaptured { result } => {
            assert_eq!(
                result.status,
                crate::capability::ScreenshotStatus::Unavailable
            );
        }
        _ => panic!("unexpected screenshot response"),
    }
}

#[test]
fn local_request_api_stores_transferred_file_under_session_artifacts() {
    let worktree_root = std::env::temp_dir().join("arroba-transfer-local-api-test");
    let _ = std::fs::remove_dir_all(&worktree_root);
    std::fs::create_dir_all(&worktree_root).expect("worktree should exist");
    let source = worktree_root.join("artifact.txt");
    std::fs::write(&source, "artifact").expect("file should exist");

    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-transfer".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let response = harness
        .dispatch(LocalDaemonRequest::StoreTransferredFile(
            StoreTransferredFileCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                source_path: source,
                display_name: None,
            },
        ))
        .expect("transfer should succeed");

    match response {
        LocalDaemonResponse::FileTransferred { result } => {
            assert!(result
                .stored_path
                .to_string_lossy()
                .contains("arroba-session-artifacts"));
            assert_eq!(result.bytes, 8);
        }
        _ => panic!("unexpected transfer response"),
    }
}
