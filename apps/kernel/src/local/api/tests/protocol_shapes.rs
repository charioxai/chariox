use super::*;

#[test]
fn local_daemon_protocol_provider_run_usage_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 45);

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

    let substitute_request =
        LocalDaemonRequest::UpdateAgentSubstitutes(UpdateAgentSubstitutesRequest {
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            action: AgentSubstituteAction::Add {
                provider: "codex".to_string(),
                model: "gpt-5.4".to_string(),
                variant: Some("medium".to_string()),
                kernel_id: Some("kernel-1".to_string()),
                worktree_id: Some("/repo/sub".to_string()),
            },
        });
    let substitute_snapshot =
        serde_json::to_value(substitute_request).expect("substitute request should serialize");
    assert_eq!(
        substitute_snapshot.pointer("/UpdateAgentSubstitutes/action/Add/kernel_id"),
        Some(&serde_json::json!("kernel-1"))
    );
    assert_eq!(
        substitute_snapshot.pointer("/UpdateAgentSubstitutes/action/Add/worktree_id"),
        Some(&serde_json::json!("/repo/sub"))
    );
    let substitute_add_snapshot = substitute_snapshot
        .pointer("/UpdateAgentSubstitutes/action/Add")
        .expect("substitute add payload should serialize");
    let serialized =
        serde_json::to_string(substitute_add_snapshot).expect("substitute payload should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "9b0859c53bee6ebd06bec8f4fc4d5181876cbf407433f0f54f6aa7f29e2f3fec"
    );

    let layout_request = serde_json::to_value(LocalDaemonRequest::UpdateWorkflowCanvasLayout(
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
                    waypoints: vec![crate::session::WorkflowCanvasPoint { x: 220, y: 80 }],
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

    let design_op_request = serde_json::to_value(LocalDaemonRequest::ApplyWorkflowDesignOp(
        super::ApplyWorkflowDesignOpRequest {
            session_id: "session-1".to_string(),
            origin_client_id: "web-client-1".to_string(),
            op_id: "op-1".to_string(),
            op: super::WorkflowDesignOp::NodeAdd {
                workflow_id: "workflow-1".to_string(),
                node: super::WorkflowDesignNode {
                    id: "node-1".to_string(),
                    agent_id: "agent-1".to_string(),
                    label: None,
                    instructions: Some("Review the change".to_string()),
                    can_complete_workflow_run: None,
                    can_emit_intermediate_run_output: None,
                    intermediate_output_schema_ref: None,
                    max_turns: Some(3),
                },
                position: Some(super::WorkflowDesignPoint { x: 120, y: 80 }),
            },
        },
    ))
    .expect("design op request should serialize");
    let design_op_payload = design_op_request
        .pointer("/ApplyWorkflowDesignOp")
        .expect("design op payload should serialize");
    assert_eq!(
        design_op_payload.pointer("/op/workflow_id"),
        Some(&serde_json::json!("workflow-1"))
    );
    assert_eq!(
        design_op_payload.pointer("/op/position/x"),
        Some(&serde_json::json!(120))
    );
    let serialized =
        serde_json::to_string(design_op_payload).expect("design op snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "6beed0034e0a7717008a1e7269e1d01f096ce14af5a65c94efe7d111cdc52e94"
    );

    let custom_interaction = crate::session::RuntimeInteraction::new(
        "interaction-1",
        "agent-1",
        crate::session::RuntimeInteractionKind::Choice,
        crate::session::RuntimeInteractionLevel::Info,
        Some("Pick a color".to_string()),
        "Choose a color or type another one.",
        vec![
            crate::session::RuntimeInteractionChoice::new("green", "Green", "Green", None),
            crate::session::RuntimeInteractionChoice::new("red", "Red", "Red", None),
        ],
        Some(crate::session::RuntimeInteractionCustomChoice::new(
            "custom",
            "Other",
            Some("Type a color".to_string()),
            Some(1),
            Some(120),
        )),
        None,
        None,
    );
    let custom_interaction_snapshot =
        serde_json::to_value(custom_interaction).expect("custom interaction should serialize");
    assert_eq!(
        custom_interaction_snapshot.pointer("/custom_choice/id"),
        Some(&serde_json::json!("custom"))
    );
    assert_eq!(
        custom_interaction_snapshot.pointer("/custom_choice/placeholder"),
        Some(&serde_json::json!("Type a color"))
    );
    let custom_response_request = serde_json::to_value(LocalDaemonRequest::RespondToInteraction(
        super::RespondToInteractionRequest {
            session_id: "session-1".to_string(),
            interaction_id: "interaction-1".to_string(),
            choice_id: "custom".to_string(),
            custom_reply: Some("Blue".to_string()),
        },
    ))
    .expect("custom interaction response should serialize");
    let custom_response_payload = custom_response_request
        .pointer("/RespondToInteraction")
        .expect("custom interaction response payload should serialize");
    assert_eq!(
        custom_response_payload.pointer("/custom_reply"),
        Some(&serde_json::json!("Blue"))
    );
    let serialized = serde_json::to_string(&serde_json::json!({
        "custom_choice": custom_interaction_snapshot
            .pointer("/custom_choice")
            .expect("custom choice should serialize"),
        "response": custom_response_payload,
    }))
    .expect("custom interaction snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "f1ded2949999d324de8a29805cbe0f0841625106e63ab556c3c6076fcf3f640d"
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

    let delete_worktree_request = serde_json::to_value(
        LocalDaemonRequest::DeleteWorkspaceWorktree(DeleteWorkspaceWorktreeRequest {
            workspace_id: "workspace-1".to_string(),
            worktree_id: "worktree-1".to_string(),
            force: true,
        }),
    )
    .expect("delete worktree request should serialize");
    let delete_worktree_payload = delete_worktree_request
        .pointer("/DeleteWorkspaceWorktree")
        .expect("delete worktree payload should serialize");
    assert_eq!(
        delete_worktree_payload.pointer("/worktree_id"),
        Some(&serde_json::json!("worktree-1"))
    );
    let serialized = serde_json::to_string(delete_worktree_payload)
        .expect("delete worktree request snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "d3df9ce72d0e27572f5ce66e5e29ba809d6e00d73c5508deda2aa810969f40e8"
    );

    let create_pr_request = serde_json::to_value(LocalDaemonRequest::CreateWorkspacePullRequest(
        CreateWorkspacePullRequestRequest {
            workspace_id: "workspace-1".to_string(),
            worktree_id: "worktree-1".to_string(),
            title: Some("Ship feature".to_string()),
            body: Some("Body".to_string()),
            base_ref: Some("main".to_string()),
            draft: true,
        },
    ))
    .expect("create pull request request should serialize");
    let create_pr_payload = create_pr_request
        .pointer("/CreateWorkspacePullRequest")
        .expect("create pull request payload should serialize");
    assert_eq!(
        create_pr_payload.pointer("/base_ref"),
        Some(&serde_json::json!("main"))
    );
    let serialized = serde_json::to_string(create_pr_payload)
        .expect("create pull request request snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "318e776a7f78bd7b0a8543028eea8d1acd44865c3f49a7bd22a7596c77b22471"
    );

    let pull_request = WorkspacePullRequestRecord {
        workspace_id: "workspace-1".to_string(),
        worktree_id: "worktree-1".to_string(),
        branch: "feature".to_string(),
        base_ref: "main".to_string(),
        url: "https://github.com/example/repo/pull/1".to_string(),
        title: Some("Ship feature".to_string()),
        draft: true,
        generated_at_ms: 1237,
    };
    let pr_response =
        serde_json::to_value(LocalDaemonResponse::WorkspacePullRequestCreated { pull_request })
            .expect("pull request response should serialize");
    let pr_response_payload = pr_response
        .pointer("/WorkspacePullRequestCreated/pull_request")
        .expect("pull request response payload should serialize");
    assert_eq!(
        pr_response_payload.pointer("/url"),
        Some(&serde_json::json!("https://github.com/example/repo/pull/1"))
    );
    let serialized = serde_json::to_string(pr_response_payload)
        .expect("pull request response snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "4354e2b9a67c08033d5306739f223f09af4ca44c747a01c30499526766bca00f"
    );
}

#[test]
fn local_daemon_protocol_active_turn_phase_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 45);

    let active_turn = crate::runtime::projection::AgentActiveTurnProjection {
        prompt_id: "prompt-1".to_string(),
        provider_run_id: Some("provider-run-1".to_string()),
        status: crate::runtime::projection::AgentPromptRuntimeStatus::Running,
        phase: crate::runtime::projection::AgentTurnRuntimePhase::AwaitingFirstOutput,
        started_at_ms: Some(1234),
    };

    let snapshot = serde_json::to_value(active_turn).expect("active turn should serialize");
    assert_eq!(
        snapshot.pointer("/phase"),
        Some(&serde_json::json!("awaiting_first_output"))
    );
    assert_eq!(
        snapshot.pointer("/started_at_ms"),
        Some(&serde_json::json!(1234))
    );

    let serialized = serde_json::to_string(&snapshot).expect("active turn snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "8798828b0a7e69332c3f369d8eb489bd9988d7488eaf56d713e3d23af9f7f40f"
    );
}

#[test]
fn local_daemon_protocol_native_provider_interaction_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 45);

    let request = LocalDaemonRequest::RequestNativeProviderInteraction(
        RequestNativeProviderInteractionRequest::allow_deny(
            "session-1",
            "agent-1",
            "native-permission-1",
            Some("Approve Claude Code Bash?".to_string()),
            "Claude Code wants to run:\n\n`echo hello`",
            Some(300),
        ),
    );
    let snapshot = serde_json::to_value(request).expect("request should serialize");
    assert_eq!(
        snapshot.pointer("/RequestNativeProviderInteraction/interaction_id"),
        Some(&serde_json::json!("native-permission-1"))
    );
    assert_eq!(
        snapshot.pointer("/RequestNativeProviderInteraction/choices/0/id"),
        Some(&serde_json::json!("allow_once"))
    );
    assert_eq!(
        snapshot.pointer("/RequestNativeProviderInteraction/default_on_timeout"),
        Some(&serde_json::json!("deny"))
    );
    let response = LocalDaemonResponse::NativeProviderInteractionResolved {
        resolution: super::NativeProviderInteractionResolution {
            status: "answered".to_string(),
            choice_id: Some("allow_once".to_string()),
            reply: Some("allow".to_string()),
        },
    };
    let response_snapshot = serde_json::to_value(response).expect("response should serialize");
    assert_eq!(
        response_snapshot.pointer("/NativeProviderInteractionResolved/resolution/reply"),
        Some(&serde_json::json!("allow"))
    );
    let serialized = serde_json::to_string(&serde_json::json!({
        "request": snapshot,
        "response": response_snapshot,
    }))
    .expect("native provider interaction snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "de3b26de0ee408204a7afaf15173bf02180358db6c10ea566f0f9f22b0d32031"
    );
}

#[test]
fn local_daemon_protocol_kernel_targeted_spawn_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 45);

    let request = LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
        session_id: "session-1".to_string(),
        alias: Some("worker".to_string()),
        provider: Some("codex".to_string()),
        model: Some("gpt-5".to_string()),
        effort: Some("medium".to_string()),
        execution_mode: Some(AgentExecutionMode::Build),
        permission_level: Some(AgentPermissionLevel::Required),
        worktree_id: None,
        kernel_ref: Some("kernel-worker".to_string()),
        slice_ref: None,
        worktree_placement: None,
    });
    let snapshot = serde_json::to_value(request).expect("request should serialize");
    assert_eq!(
        snapshot.pointer("/SpawnAgent/kernel_ref"),
        Some(&serde_json::json!("kernel-worker"))
    );
    assert_eq!(snapshot.pointer("/SpawnAgent/machine_ref"), None);
    let serialized =
        serde_json::to_string(&snapshot).expect("kernel-targeted spawn snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "712cecc5815da7fa33661de8db724f62e1aa90cfdbe56e332a5d13fbc8f4b848"
    );
}

#[test]
fn local_daemon_protocol_slice_targeted_spawn_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 45);

    let request = LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
        session_id: "session-1".to_string(),
        alias: Some("worker".to_string()),
        provider: Some("codex".to_string()),
        model: Some("gpt-5".to_string()),
        effort: Some("medium".to_string()),
        execution_mode: Some(AgentExecutionMode::Build),
        permission_level: Some(AgentPermissionLevel::Required),
        worktree_id: None,
        kernel_ref: None,
        slice_ref: Some("linux-dev".to_string()),
        worktree_placement: None,
    });
    let snapshot = serde_json::to_value(request).expect("request should serialize");
    assert_eq!(
        snapshot.pointer("/SpawnAgent/slice_ref"),
        Some(&serde_json::json!("linux-dev"))
    );
    assert_eq!(
        snapshot.pointer("/SpawnAgent/kernel_ref"),
        Some(&serde_json::Value::Null)
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("slice-targeted spawn snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "a7bd0c2bb693c63aa515a2e89f98a5b6144bb750fdcd266bc87e9d4903d4a1d4"
    );
}

#[test]
fn local_daemon_protocol_slice_targeted_create_session_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 45);

    let request = LocalDaemonRequest::CreateSession(
        CreateSessionRequest::new("workspace-1", "worktree-1")
            .with_alias("slice-session")
            .with_slice_ref("linux-dev"),
    );
    let snapshot = serde_json::to_value(request).expect("request should serialize");
    assert_eq!(
        snapshot.pointer("/CreateSession/slice_ref"),
        Some(&serde_json::json!("linux-dev"))
    );
    let serialized = serde_json::to_string(&snapshot)
        .expect("slice-targeted create session snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "0d6c7b4337eedcd0f3f33f231a81f639e9b3285e729e2fd32a00abb1d8901db1"
    );
}

#[test]
fn local_daemon_protocol_slice_record_relay_endpoint_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 45);

    let response = LocalDaemonResponse::Slice {
        slice: crate::slice::SliceRecord {
            id: "slice-1".to_string(),
            name: "linux-dev".to_string(),
            owner_kernel_id: "home-kernel".to_string(),
            owner_machine_id: "home-machine".to_string(),
            session_id: Some("session-1".to_string()),
            backend: crate::slice::SliceBackendKind::LocalDocker,
            os: "linux".to_string(),
            status: crate::slice::SliceStatus::Running,
            workspace_mount: Some("/repo".to_string()),
            worker_kernel_ref: "slice:linux-dev".to_string(),
            worker_kernel_id: Some("slice-worker".to_string()),
            worker_machine_id: Some("slice:slice-1".to_string()),
            relay_endpoint: Some(crate::slice::SliceRelayEndpoint {
                url: "ws://127.0.0.1:43130".to_string(),
                private: true,
            }),
            providers: vec!["codex".to_string(), "opencode".to_string()],
            display_endpoint: None,
            created_at_ms: 1000,
            updated_at_ms: 2000,
        },
    };
    let snapshot = serde_json::to_value(response).expect("response should serialize");
    assert_eq!(
        snapshot.pointer("/Slice/slice/relay_endpoint/url"),
        Some(&serde_json::json!("ws://127.0.0.1:43130"))
    );
    assert_eq!(
        snapshot.pointer("/Slice/slice/relay_endpoint/private"),
        Some(&serde_json::json!(true))
    );
    assert_eq!(
        snapshot.pointer("/Slice/slice/session_id"),
        Some(&serde_json::json!("session-1"))
    );
    let serialized = serde_json::to_string(&snapshot).expect("slice record snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "5bcb928faa08bb698d8b4a513a155cd732735f80cc728e86741b21e2ed014892"
    );
}

#[test]
fn local_daemon_protocol_semantic_history_search_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 45);

    let request = LocalDaemonRequest::SemanticSearchHistory(SemanticSearchHistoryRequest {
        query: "why did the build fail".to_string(),
        mode: Some(crate::local::SemanticSearchHistoryMode::Agent),
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        provider: Some("codex".to_string()),
        model: Some("gpt-5".to_string()),
        workflow_id: None,
        machine_id: None,
        repo_root: None,
        worktree_path: None,
        kind: Some("provider_output".to_string()),
        cursor: Some("cursor-0".to_string()),
        limit: Some(12),
    });
    let request_snapshot = serde_json::to_value(request).expect("request should serialize");
    assert_eq!(
        request_snapshot.pointer("/SemanticSearchHistory/query"),
        Some(&serde_json::json!("why did the build fail"))
    );
    assert_eq!(
        request_snapshot.pointer("/SemanticSearchHistory/mode"),
        Some(&serde_json::json!("agent"))
    );
    assert_eq!(
        request_snapshot.pointer("/SemanticSearchHistory/session_id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        request_snapshot.pointer("/SemanticSearchHistory/limit"),
        Some(&serde_json::json!(12))
    );
    assert_eq!(
        request_snapshot.pointer("/SemanticSearchHistory/cursor"),
        Some(&serde_json::json!("cursor-0"))
    );

    let event = crate::history::HistoryEvent {
        event_id: "event-1".to_string(),
        sequence: 7,
        timestamp_ms: 1234,
        workspace_id: None,
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        agent_alias: None,
        provider: Some("codex".to_string()),
        model: Some("gpt-5".to_string()),
        turn_id: None,
        prompt_id: None,
        provider_run_id: None,
        provider_session_id: None,
        workflow_id: None,
        workflow_run_id: None,
        workflow_node_id: None,
        machine_id: None,
        repo_root: None,
        worktree_path: None,
        kind: crate::history::HistoryEventKind::ProviderOutput,
        role: Some(crate::history::HistoryEventRole::Assistant),
        content: Some("the build failed because tests failed".to_string()),
        content_ref: None,
        metadata: BTreeMap::new(),
        candidate_agent_ids: Vec::new(),
        candidate_prompt_ids: Vec::new(),
        candidate_turn_ids: Vec::new(),
        attribution_confidence: None,
        caused_by_event_id: None,
    };
    let response = LocalDaemonResponse::SemanticHistoryEvents {
        results: vec![SemanticHistoryMatch {
            event,
            score_millis: Some(914),
            chunk_index: Some(0),
            chunk_text: Some("build failed because tests failed".to_string()),
            reason: Some("high: direct match".to_string()),
        }],
        next_cursor: Some("cursor-1".to_string()),
        unavailable_reason: None,
        answer: Some("The build failed because tests failed.".to_string()),
    };
    let response_snapshot = serde_json::to_value(response).expect("response should serialize");
    assert_eq!(
        response_snapshot.pointer("/SemanticHistoryEvents/results/0/score_millis"),
        Some(&serde_json::json!(914))
    );
    assert_eq!(
        response_snapshot.pointer("/SemanticHistoryEvents/results/0/chunk_text"),
        Some(&serde_json::json!("build failed because tests failed"))
    );
    assert_eq!(
        response_snapshot.pointer("/SemanticHistoryEvents/results/0/reason"),
        Some(&serde_json::json!("high: direct match"))
    );
    assert_eq!(
        response_snapshot.pointer("/SemanticHistoryEvents/answer"),
        Some(&serde_json::json!("The build failed because tests failed."))
    );
    assert_eq!(
        response_snapshot.pointer("/SemanticHistoryEvents/unavailable_reason"),
        Some(&serde_json::Value::Null)
    );

    let serialized = serde_json::to_string(&serde_json::json!({
        "request": request_snapshot,
        "response": response_snapshot,
    }))
    .expect("semantic history snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "6f8e40065757d56ae36fd7f8609d4aaed543d76a582dcffe20c2dae514688196"
    );
}

#[test]
fn local_daemon_protocol_query_history_context_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 45);

    let request = LocalDaemonRequest::QueryHistory(QueryHistoryRequest {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        provider: None,
        model: None,
        workflow_id: None,
        machine_id: None,
        repo_root: None,
        worktree_path: None,
        kind: Some("provider_output".to_string()),
        text: None,
        after_sequence: None,
        before_sequence: Some(42),
        limit: Some(10),
    });
    let snapshot = serde_json::to_value(request).expect("request should serialize");
    assert_eq!(
        snapshot.pointer("/QueryHistory/before_sequence"),
        Some(&serde_json::json!(42))
    );

    let serialized =
        serde_json::to_string(&snapshot).expect("query history snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "e58f083c2b36e276158bcde1483d734b76fd29067f613ea61b51a3ea1eda3a7d"
    );
}

#[test]
fn local_daemon_protocol_agent_config_workspace_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 45);

    let request = LocalDaemonRequest::UpdateAgentConfig(UpdateAgentConfigRequest {
        session_id: "session-1".to_string(),
        agent_id: "agent-1".to_string(),
        execution_mode: Some(AgentExecutionMode::Build),
        clear_execution_mode: false,
        permission_level: Some(AgentPermissionLevel::Required),
        clear_permission_level: false,
        workspace_id: Some("/repo".to_string()),
        clear_workspace_id: false,
        worktree_id: Some("/repo-feature".to_string()),
        clear_worktree_id: false,
    });
    let snapshot = serde_json::to_value(request).expect("request should serialize");
    assert_eq!(
        snapshot.pointer("/UpdateAgentConfig/workspace_id"),
        Some(&serde_json::json!("/repo"))
    );
    assert_eq!(
        snapshot.pointer("/UpdateAgentConfig/worktree_id"),
        Some(&serde_json::json!("/repo-feature"))
    );
    let serialized = serde_json::to_string(&snapshot).expect("agent config snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "826ea52fcd9a136d573384f51126a8b59e5829b8fd6160d6601e5d5759d5f6a2"
    );
}

#[test]
fn local_daemon_protocol_native_tui_provider_selection_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 45);

    let request =
        LocalDaemonRequest::UpdateProviderRunSelection(UpdateProviderRunSelectionRequest {
            session_id: "session-1".to_string(),
            provider_run_id: "provider-run-1".to_string(),
            model: Some("openai/gpt-5.4".to_string()),
            variant: Some("high".to_string()),
            clear_variant: false,
        });
    let snapshot = serde_json::to_value(request).expect("request should serialize");
    assert_eq!(
        snapshot.pointer("/UpdateProviderRunSelection/model"),
        Some(&serde_json::json!("openai/gpt-5.4"))
    );
    assert_eq!(
        snapshot.pointer("/UpdateProviderRunSelection/variant"),
        Some(&serde_json::json!("high"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("provider selection snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "bce42e6fc169c8747199a98a5dc059e5b40ac7f8aafb0f9a7a67f4b336ef57e5"
    );
}

#[test]
fn local_daemon_protocol_terminal_input_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 45);

    let request = LocalDaemonRequest::SendTerminalInput(SendTerminalInputRequest {
        session_id: "session-1".to_string(),
        attachment_id: "attachment-1".to_string(),
        provider_run_id: Some("provider-run-1".to_string()),
        data_base64: "aGVsbG8N".to_string(),
    });
    let snapshot = serde_json::to_value(request).expect("request should serialize");
    assert_eq!(
        snapshot.pointer("/SendTerminalInput/session_id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        snapshot.pointer("/SendTerminalInput/attachment_id"),
        Some(&serde_json::json!("attachment-1"))
    );
    assert_eq!(
        snapshot.pointer("/SendTerminalInput/provider_run_id"),
        Some(&serde_json::json!("provider-run-1"))
    );
    assert_eq!(
        snapshot.pointer("/SendTerminalInput/data_base64"),
        Some(&serde_json::json!("aGVsbG8N"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("terminal input snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "95c089680846665e95d16114c5c6245b2f4e49c0f0b1dfdc390c62a9f1ff836a"
    );
}
