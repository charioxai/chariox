use super::*;

#[test]
fn local_daemon_protocol_workspace_live_sync_status_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 83);

    let request = LocalDaemonRequest::GetWorkspaceLiveSyncStatus(
        crate::local::GetWorkspaceLiveSyncStatusRequest {
            session_id: "session-1".to_string(),
        },
    );
    let mode_request = LocalDaemonRequest::SetWorkspaceLiveSyncMode(
        crate::local::SetWorkspaceLiveSyncModeRequest {
            session_id: "session-1".to_string(),
            mode: crate::config::WorkspaceLiveSyncMode::Tracked,
        },
    );
    let response = LocalDaemonResponse::WorkspaceLiveSyncStatus {
        status: crate::local::WorkspaceLiveSyncStatus {
            session_id: "session-1".to_string(),
            mode: crate::config::WorkspaceLiveSyncMode::Tracked,
            footer_state: crate::local::WorkspaceLiveSyncFooterState::Tracked,
            sync_groups: vec![crate::local::WorkspaceLiveSyncGroupStatus {
                group_id: "workspace-link-1".to_string(),
                group_name: "shared".to_string(),
                target_count: 1,
                ready_targets: 1,
                degraded_targets: 0,
                conflicted_targets: 0,
            }],
            targets: vec![crate::local::WorkspaceLiveSyncTargetStatus {
                link_id: "workspace-link-1".to_string(),
                link_name: "shared".to_string(),
                user_id: "user-1".to_string(),
                machine_id: "machine-1".to_string(),
                kernel_id: "kernel-1".to_string(),
                repo_root: "/repo".to_string(),
                branch: Some("main".to_string()),
                repo_fingerprint: Some("fingerprint-1".to_string()),
                status: crate::local::WorkspaceLiveSyncTargetState::Ready,
                attached_at_ms: 42,
            }],
            conflicts: vec![crate::local::WorkspaceLiveSyncConflictSummary {
                conflict_id: "conflict-1".to_string(),
                link_id: "workspace-link-1".to_string(),
                source_agent_id: "agent-1".to_string(),
                target_user_id: "user-2".to_string(),
                target_repo_root: "/repo-2".to_string(),
                path: "src/lib.rs".to_string(),
                next_action: "Assign a resolver agent.".to_string(),
            }],
            ignore: crate::local::WorkspaceLiveSyncIgnoreStatus {
                ignore_file: Some(".arrobaignore".to_string()),
                rules: vec!["ignored/**".to_string(), "*.secret".to_string()],
                force_excludes: vec![".git/**".to_string(), ".arroba/**".to_string()],
            },
        },
    };
    let mut mode_session = crate::session::RuntimeSession::new(
        "session-1",
        None,
        "/repo",
        "/repo",
        "machine-1",
        "daemon-1",
    );
    mode_session.set_workspace_live_sync_mode(Some(crate::config::WorkspaceLiveSyncMode::Tracked));
    let mode_response = LocalDaemonResponse::WorkspaceLiveSyncModeUpdated {
        session: mode_session,
    };

    let mut snapshot = serde_json::json!([request, mode_request, response, mode_response]);
    *snapshot
        .pointer_mut("/3/WorkspaceLiveSyncModeUpdated/session/created_at_ms")
        .expect("session created_at_ms should encode") = serde_json::json!(42);
    *snapshot
        .pointer_mut("/3/WorkspaceLiveSyncModeUpdated/session/last_used_at_ms")
        .expect("session last_used_at_ms should encode") = serde_json::json!(42);
    assert_eq!(
        snapshot.pointer("/0/GetWorkspaceLiveSyncStatus/session_id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        snapshot.pointer("/1/SetWorkspaceLiveSyncMode/session_id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        snapshot.pointer("/1/SetWorkspaceLiveSyncMode/mode"),
        Some(&serde_json::json!("tracked"))
    );
    assert_eq!(
        snapshot.pointer("/2/WorkspaceLiveSyncStatus/status/footer_state"),
        Some(&serde_json::json!("tracked"))
    );
    assert_eq!(
        snapshot.pointer("/2/WorkspaceLiveSyncStatus/status/sync_groups/0/group_id"),
        Some(&serde_json::json!("workspace-link-1"))
    );
    assert_eq!(
        snapshot.pointer("/2/WorkspaceLiveSyncStatus/status/targets/0/status"),
        Some(&serde_json::json!("ready"))
    );
    assert_eq!(
        snapshot.pointer("/3/WorkspaceLiveSyncModeUpdated/session/id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        snapshot.pointer("/3/WorkspaceLiveSyncModeUpdated/session/workspace_live_sync_mode"),
        Some(&serde_json::json!("tracked"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("workspace live sync status should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "b8ed6e7aa0a63e96550ed94e0dc08175823c800f1bcf5e22c179c49f92eb0787"
    );
}

#[test]
fn relay_workspace_live_sync_apply_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 83);

    let context = crate::transport::relay_peer::RemoteWorkspaceLiveSyncApplyContext {
        home_session_id: "session-1".to_string(),
        link_id: "workspace-link-1".to_string(),
        link_name: "team-sync".to_string(),
        source_agent_id: "agent-1".to_string(),
        source_worktree_path: "/home/user/project".to_string(),
        target_user_id: "user-2".to_string(),
        target_machine_id: "machine-2".to_string(),
        target_kernel_id: "kernel-2".to_string(),
        target_repo_root: "/remote/user/project".to_string(),
    };
    let change = crate::git_observer::WorkspaceLiveSyncChange {
        session_id: "session-1".to_string(),
        agent_id: "agent-1".to_string(),
        provider_run_id: "provider-run-1".to_string(),
        prompt_id: "prompt-1".to_string(),
        repo_root: "/home/user/project".to_string(),
        worktree_path: "/home/user/project".to_string(),
        branch: Some("main".to_string()),
        changed_paths: vec!["src/lib.rs".to_string()],
        file_changes: vec![crate::git_observer::WorkspaceLiveSyncFileChange {
            path: "src/lib.rs".to_string(),
            previous_path: None,
            kind: crate::git_observer::WorkspaceLiveSyncFileChangeKind::Modified,
            before_content_base64: Some("b2xkCg==".to_string()),
            after_content_base64: Some("bmV3Cg==".to_string()),
            binary: false,
        }],
        status_fingerprint: " M src/lib.rs".to_string(),
    };
    let target_result = crate::git_observer::WorkspaceLiveSyncTargetResult {
        session_id: "session-1".to_string(),
        link_id: "workspace-link-1".to_string(),
        link_name: "team-sync".to_string(),
        source_agent_id: "agent-1".to_string(),
        source_worktree_path: "/home/user/project".to_string(),
        target_user_id: "user-2".to_string(),
        target_machine_id: "machine-2".to_string(),
        target_kernel_id: "kernel-2".to_string(),
        target_repo_root: "/remote/user/project".to_string(),
        path_results: vec![crate::git_observer::WorkspaceLiveSyncPathApplyResult {
            path: "src/lib.rs".to_string(),
            status: crate::git_observer::WorkspaceLiveSyncApplyStatus::Rebased,
            message: "applied after non-overlap rebase".to_string(),
        }],
    };
    let request = crate::transport::relay_peer::RelayPeerRequest::ApplyWorkspaceLiveSyncChange {
        context,
        change,
    };
    let response =
        crate::transport::relay_peer::RelayPeerResponse::WorkspaceLiveSyncChangeApplied {
            target_result,
        };

    let snapshot = serde_json::json!([request, response]);
    assert_eq!(
        snapshot.pointer("/0/kind"),
        Some(&serde_json::json!("apply_workspace_live_sync_change"))
    );
    assert_eq!(
        snapshot.pointer("/0/context/link_id"),
        Some(&serde_json::json!("workspace-link-1"))
    );
    assert_eq!(
        snapshot.pointer("/0/change/file_changes/0/kind"),
        Some(&serde_json::json!("modified"))
    );
    assert_eq!(
        snapshot.pointer("/1/kind"),
        Some(&serde_json::json!("workspace_live_sync_change_applied"))
    );
    assert_eq!(
        snapshot.pointer("/1/target_result/path_results/0/status"),
        Some(&serde_json::json!("rebased"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("relay workspace live sync shape should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "4b55d5a1dd6ef7e5132a20004156f84c70f88cd11385dc8cb93fb68ddc258107"
    );
}

#[test]
fn relay_home_extension_invocation_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 83);

    let context = crate::transport::relay_peer::RemoteExtensionInvocationContext {
        home_kernel_id: "home-kernel".to_string(),
        home_session_id: "session-1".to_string(),
        home_agent_id: "agent-1".to_string(),
        leased_agent_id: "leased-agent-1".to_string(),
        worker_provider_run_id: "provider-run-1".to_string(),
        worker_kernel_id: Some("worker-kernel".to_string()),
        worker_machine_id: Some("worker-machine".to_string()),
    };
    let metadata = crate::extension::RemoteExtensionInvocationMetadata {
        invocation_id: "invoke-1".to_string(),
        provider_tool_call_id: Some("tool-call-1".to_string()),
        attempt: 1,
        idempotency_key: Some("idem-1".to_string()),
        started_at_ms: 42,
    };
    let tool = crate::extension::RemoteExtensionTool {
        kind: crate::extension::ExtensionKind::Script,
        name: "home_lookup".to_string(),
        tool_name: "home_lookup".to_string(),
        description: "Home lookup".to_string(),
        input_schema: serde_json::json!({"type": "object"}),
        authority: crate::extension::ExtensionAuthority::Home,
        definition_origin: crate::extension::ExtensionDefinitionOrigin::Home,
        execution_location: crate::extension::ExtensionExecutionLocation::Home,
        safety: Some("read".to_string()),
        timeout_sec: Some(10),
        version_hash: Some("hash-1".to_string()),
    };
    let request = crate::transport::relay_peer::RelayPeerRequest::InvokeHomeExtensionTool {
        context: context.clone(),
        metadata: metadata.clone(),
        tool,
        arguments: serde_json::json!({"query": "status"}),
    };
    let mcp_request = crate::transport::relay_peer::RelayPeerRequest::InvokeHomeMcpProxy {
        context: context.clone(),
        metadata: metadata.clone(),
        name: "home_browser".to_string(),
        tool: crate::extension::RemoteExtensionTool {
            kind: crate::extension::ExtensionKind::Mcp,
            name: "home_browser".to_string(),
            tool_name: "home_browser".to_string(),
            description: "Home MCP".to_string(),
            input_schema: serde_json::json!({}),
            authority: crate::extension::ExtensionAuthority::Home,
            definition_origin: crate::extension::ExtensionDefinitionOrigin::Home,
            execution_location: crate::extension::ExtensionExecutionLocation::Home,
            safety: None,
            timeout_sec: Some(15),
            version_hash: Some("mcp-hash-1".to_string()),
        },
        payload: serde_json::json!({
            "jsonrpc": "2.0",
            "id": "rpc-1",
            "method": "tools/list"
        }),
    };
    let cancel_request =
        crate::transport::relay_peer::RelayPeerRequest::CancelHomeExtensionInvocation {
            context,
            metadata: metadata.clone(),
        };
    let response = crate::transport::relay_peer::RelayPeerResponse::HomeExtensionToolHandled {
        result: crate::transport::runtime_tools::RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({"status": "ok"}),
        },
    };
    let mcp_response = crate::transport::relay_peer::RelayPeerResponse::HomeMcpProxyHandled {
        response: serde_json::json!({
            "jsonrpc": "2.0",
            "id": "rpc-1",
            "result": {"tools": []}
        }),
    };
    let cancel_response =
        crate::transport::relay_peer::RelayPeerResponse::HomeExtensionInvocationCancelled {
            invocation_id: metadata.invocation_id,
            cancelled: true,
        };
    let snapshot = serde_json::json!([
        request,
        mcp_request,
        cancel_request,
        response,
        mcp_response,
        cancel_response
    ]);
    assert_eq!(
        snapshot.pointer("/0/kind"),
        Some(&serde_json::json!("invoke_home_extension_tool"))
    );
    assert_eq!(
        snapshot.pointer("/0/context/worker_kernel_id"),
        Some(&serde_json::json!("worker-kernel"))
    );
    assert_eq!(
        snapshot.pointer("/0/metadata/idempotency_key"),
        Some(&serde_json::json!("idem-1"))
    );
    assert_eq!(
        snapshot.pointer("/0/tool/execution_location"),
        Some(&serde_json::json!("home"))
    );
    assert_eq!(
        snapshot.pointer("/1/kind"),
        Some(&serde_json::json!("invoke_home_mcp_proxy"))
    );
    assert_eq!(
        snapshot.pointer("/1/tool/version_hash"),
        Some(&serde_json::json!("mcp-hash-1"))
    );
    assert_eq!(
        snapshot.pointer("/2/kind"),
        Some(&serde_json::json!("cancel_home_extension_invocation"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("relay home extension shape should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "f95aa81795cca480486b387383662637b3d7e7bdad60077df3cc31141ad6e5d1"
    );
}

#[test]
fn local_daemon_protocol_extension_install_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 83);

    let mcp = LocalDaemonRequest::InstallMcpServer(crate::local::InstallMcpServerRequest {
        workspace_id: Some("/repo".to_string()),
        config: crate::mcp::ArrobaMcpServerConfig {
            name: "github".to_string(),
            transport: crate::mcp::ArrobaMcpTransportConfig::Stdio {
                command: "npx".to_string(),
                args: vec![
                    "-y".to_string(),
                    "@modelcontextprotocol/server-github".to_string(),
                ],
                env: Default::default(),
                credential_env: std::collections::BTreeMap::from([(
                    "GITHUB_TOKEN".to_string(),
                    crate::mcp::ArrobaMcpCredentialBinding {
                        credential: "github-token".to_string(),
                    },
                )]),
                env_vars: Vec::new(),
                cwd: None,
            },
            enabled: true,
            required: false,
            startup_timeout_sec: None,
            tool_timeout_sec: Some(30),
            enabled_tools: None,
            disabled_tools: None,
            tools: Default::default(),
        },
    });
    let skill = LocalDaemonRequest::UpsertSkill(crate::local::UpsertSkillRequest {
        workspace_id: Some("/repo".to_string()),
        source: crate::local::SkillInstallSource::Url {
            url: "https://github.com/example/skills/tree/main/review".to_string(),
        },
    });
    let connector = LocalDaemonRequest::UpsertConnector(crate::local::UpsertConnectorRequest {
        connector: crate::connector::ArrobaConnectorDefinition {
            kind: "connector".to_string(),
            name: "status-api".to_string(),
            description: "Read status".to_string(),
            adapter: "http".to_string(),
            credential: Some(crate::connector::ConnectorCredentialPolicy { required: true }),
            timeout_ms: 30000,
            max_response_bytes: 1048576,
            operations: vec![crate::connector::ConnectorOperation {
                name: "get".to_string(),
                description: "Read status".to_string(),
                safety: crate::connector::ConnectorSafety::Read,
                input_schema: serde_json::json!({"type":"object"}),
                config: serde_json::json!({"method":"GET","base_url":"https://example.test","path":"/status"}),
            }],
        },
    });
    let sync = LocalDaemonRequest::SyncRemoteExtensionManifest(
        crate::local::SyncRemoteExtensionManifestRequest {
            agent_ref: "agent-1".to_string(),
        },
    );
    let audit =
        LocalDaemonRequest::ListHomeExtensionAudit(crate::local::ListHomeExtensionAuditRequest {
            agent_ref: "agent-1".to_string(),
            limit: Some(10),
        });

    let snapshot = serde_json::json!([mcp, skill, connector, sync, audit]);
    assert_eq!(
        snapshot
            .pointer("/0/InstallMcpServer/config/transport/credential_env/GITHUB_TOKEN/credential"),
        Some(&serde_json::json!("github-token"))
    );
    assert_eq!(
        snapshot.pointer("/1/UpsertSkill/source/type"),
        Some(&serde_json::json!("url"))
    );
    assert_eq!(
        snapshot.pointer("/2/UpsertConnector/connector/operations/0/config/path"),
        Some(&serde_json::json!("/status"))
    );
    assert_eq!(
        snapshot.pointer("/3/SyncRemoteExtensionManifest/agent_ref"),
        Some(&serde_json::json!("agent-1"))
    );
    assert_eq!(
        snapshot.pointer("/4/ListHomeExtensionAudit/limit"),
        Some(&serde_json::json!(10))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("extension install snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "2409b4d10bab0b296bd880e060b8931116f30f2452e5c9205e7701f9ccbe0108"
    );
}

#[test]
fn local_daemon_protocol_provider_run_usage_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 83);

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

    let edge_request = serde_json::to_value(LocalDaemonRequest::AddWorkflowEdge(
        super::AddWorkflowEdgeRequest {
            session_id: "session-1".to_string(),
            workflow_ref: "workflow-1".to_string(),
            from_node_id: "node-1".to_string(),
            to_node_id: "node-2".to_string(),
            handoff_schema_ref: None,
            validation_policy: None,
            source_side: Some(crate::session::WorkflowEdgeEndpointSide::Right),
            target_side: Some(crate::session::WorkflowEdgeEndpointSide::Left),
            expected_workflow_revision: Some(7),
        },
    ))
    .expect("edge request should serialize");
    let edge_payload = edge_request
        .pointer("/AddWorkflowEdge")
        .expect("edge request payload should serialize");
    assert_eq!(
        edge_payload.pointer("/source_side"),
        Some(&serde_json::json!("right"))
    );
    assert_eq!(
        edge_payload.pointer("/target_side"),
        Some(&serde_json::json!("left"))
    );
    let serialized =
        serde_json::to_string(edge_payload).expect("edge request snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "10955426ce27ba4a006a9c3ec20ebb73964eaf275f93c490e79fd109fdb6b123"
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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 83);

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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 83);

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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 83);

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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 83);

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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 83);

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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 83);

    let response = LocalDaemonResponse::Slice {
        slice: crate::slice::SliceRecord {
            id: "slice-1".to_string(),
            name: "linux-dev".to_string(),
            owner_kernel_id: "home-kernel".to_string(),
            owner_machine_id: "home-machine".to_string(),
            session_id: Some("session-1".to_string()),
            session_ids: vec!["session-1".to_string(), "session-2".to_string()],
            agent_ids: vec!["agent-1".to_string(), "agent-2".to_string()],
            backend: crate::slice::SliceBackendKind::LocalDocker,
            os: "linux".to_string(),
            display_mode: crate::slice::SliceDisplayMode::Headed,
            status: crate::slice::SliceStatus::Running,
            last_operation: Some("start".to_string()),
            last_operation_status: Some(crate::slice::SliceOperationStatus::Completed),
            last_error: None,
            last_operation_at_ms: Some(1900),
            workspace_id: Some("workspace-1".to_string()),
            worktree_id: Some("worktree-1".to_string()),
            workspace_mount: Some("/repo".to_string()),
            worker_kernel_ref: "slice:linux-dev".to_string(),
            worker_kernel_id: Some("slice-worker".to_string()),
            worker_machine_id: Some("slice:slice-1".to_string()),
            relay_endpoint: Some(crate::slice::SliceRelayEndpoint {
                url: "ws://127.0.0.1:43130".to_string(),
                private: true,
            }),
            local_docker_ports: Some(crate::slice::SliceLocalDockerPorts {
                codex: 44000,
                opencode: 44300,
                kernel: 44600,
                mcp: 44900,
                relay: 45200,
                novnc: 45500,
                codex_range_start: 46000,
                opencode_range_start: 51200,
            }),
            providers: vec!["codex".to_string(), "opencode".to_string()],
            provider_auth: vec![crate::slice_provider_auth::SliceProviderAuthSummary {
                provider: "codex".to_string(),
                state: crate::slice_provider_auth::SliceProviderAuthState::Configured,
                auth_type: Some("chatgpt".to_string()),
                account_id: Some("acct-1".to_string()),
                email: None,
                organization_id: None,
                organization_name: None,
                subscription_type: None,
                alias: Some("work".to_string()),
                source: "home_codex_auth_json".to_string(),
            }],
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
    assert_eq!(
        snapshot.pointer("/Slice/slice/local_docker_ports/novnc"),
        Some(&serde_json::json!(45500))
    );
    let serialized = serde_json::to_string(&snapshot).expect("slice record snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "5629172de2a00b695c47a638a3b794b2935fd6be171b30275ac2f29fa40669d6"
    );
}

#[test]
fn local_daemon_protocol_slice_auth_alias_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 83);

    let request = LocalDaemonRequest::SetSliceProviderAuthAlias(
        crate::local::SetSliceProviderAuthAliasRequest {
            slice_ref: "linux-dev".to_string(),
            provider: "codex".to_string(),
            alias: Some("work".to_string()),
        },
    );
    let response = LocalDaemonResponse::SliceProviderAuthAliasSet {
        slice: crate::slice::SliceRecord {
            id: "slice-1".to_string(),
            name: "linux-dev".to_string(),
            owner_kernel_id: "home-kernel".to_string(),
            owner_machine_id: "home-machine".to_string(),
            session_id: None,
            session_ids: Vec::new(),
            agent_ids: Vec::new(),
            backend: crate::slice::SliceBackendKind::LocalDocker,
            os: "linux".to_string(),
            display_mode: crate::slice::SliceDisplayMode::Headed,
            status: crate::slice::SliceStatus::Running,
            last_operation: None,
            last_operation_status: None,
            last_error: None,
            last_operation_at_ms: None,
            workspace_id: Some("workspace-1".to_string()),
            worktree_id: Some("worktree-1".to_string()),
            workspace_mount: Some("/repo".to_string()),
            worker_kernel_ref: "slice:linux-dev".to_string(),
            worker_kernel_id: Some("slice-worker".to_string()),
            worker_machine_id: Some("slice:slice-1".to_string()),
            relay_endpoint: None,
            local_docker_ports: None,
            providers: vec!["codex".to_string()],
            provider_auth: vec![crate::slice_provider_auth::SliceProviderAuthSummary {
                provider: "codex".to_string(),
                state: crate::slice_provider_auth::SliceProviderAuthState::Configured,
                auth_type: Some("chatgpt".to_string()),
                account_id: Some("acct-1".to_string()),
                email: None,
                organization_id: None,
                organization_name: None,
                subscription_type: None,
                alias: Some("work".to_string()),
                source: "home_codex_auth_json".to_string(),
            }],
            display_endpoint: None,
            created_at_ms: 1000,
            updated_at_ms: 2000,
        },
        provider: "codex".to_string(),
        alias: Some("work".to_string()),
    };
    let remove_request =
        LocalDaemonRequest::RemoveSliceProviderAuth(crate::local::RemoveSliceProviderAuthRequest {
            slice_ref: "linux-dev".to_string(),
            provider: "codex".to_string(),
        });
    let remove_response = LocalDaemonResponse::SliceProviderAuthRemoved {
        slice: crate::slice::SliceRecord {
            id: "slice-1".to_string(),
            name: "linux-dev".to_string(),
            owner_kernel_id: "home-kernel".to_string(),
            owner_machine_id: "home-machine".to_string(),
            session_id: None,
            session_ids: Vec::new(),
            agent_ids: Vec::new(),
            backend: crate::slice::SliceBackendKind::LocalDocker,
            os: "linux".to_string(),
            display_mode: crate::slice::SliceDisplayMode::Headed,
            status: crate::slice::SliceStatus::Running,
            last_operation: None,
            last_operation_status: None,
            last_error: None,
            last_operation_at_ms: None,
            workspace_id: Some("workspace-1".to_string()),
            worktree_id: Some("worktree-1".to_string()),
            workspace_mount: Some("/repo".to_string()),
            worker_kernel_ref: "slice:linux-dev".to_string(),
            worker_kernel_id: Some("slice-worker".to_string()),
            worker_machine_id: Some("slice:slice-1".to_string()),
            relay_endpoint: None,
            local_docker_ports: None,
            providers: vec!["codex".to_string()],
            provider_auth: Vec::new(),
            display_endpoint: None,
            created_at_ms: 1000,
            updated_at_ms: 2001,
        },
        provider: "codex".to_string(),
        status: "removed".to_string(),
    };
    let snapshot = serde_json::json!([request, response, remove_request, remove_response]);
    assert_eq!(
        snapshot.pointer("/0/SetSliceProviderAuthAlias/slice_ref"),
        Some(&serde_json::json!("linux-dev"))
    );
    assert_eq!(
        snapshot.pointer("/0/SetSliceProviderAuthAlias/alias"),
        Some(&serde_json::json!("work"))
    );
    assert_eq!(
        snapshot.pointer("/1/SliceProviderAuthAliasSet/provider"),
        Some(&serde_json::json!("codex"))
    );
    assert_eq!(
        snapshot.pointer("/2/RemoveSliceProviderAuth/slice_ref"),
        Some(&serde_json::json!("linux-dev"))
    );
    assert_eq!(
        snapshot.pointer("/3/SliceProviderAuthRemoved/status"),
        Some(&serde_json::json!("removed"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("slice auth alias snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "af66cd4f4c5d7017c102118c67da70491197def0055b197731c1c536fba2e28e"
    );
}

#[test]
fn local_daemon_protocol_slice_provider_login_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 83);

    let request =
        LocalDaemonRequest::StartSliceProviderLogin(crate::local::StartSliceProviderLoginRequest {
            slice_ref: "linux-dev".to_string(),
            provider: "codex".to_string(),
        });
    let response = LocalDaemonResponse::SliceProviderLoginStarted {
        slice: crate::slice::SliceRecord {
            id: "slice-1".to_string(),
            name: "linux-dev".to_string(),
            owner_kernel_id: "home-kernel".to_string(),
            owner_machine_id: "home-machine".to_string(),
            session_id: None,
            session_ids: Vec::new(),
            agent_ids: Vec::new(),
            backend: crate::slice::SliceBackendKind::LocalDocker,
            os: "linux".to_string(),
            display_mode: crate::slice::SliceDisplayMode::Headed,
            status: crate::slice::SliceStatus::Running,
            last_operation: None,
            last_operation_status: None,
            last_error: None,
            last_operation_at_ms: None,
            workspace_id: Some("workspace-1".to_string()),
            worktree_id: Some("worktree-1".to_string()),
            workspace_mount: Some("/repo".to_string()),
            worker_kernel_ref: "slice:linux-dev".to_string(),
            worker_kernel_id: Some("slice-worker".to_string()),
            worker_machine_id: Some("slice:slice-1".to_string()),
            relay_endpoint: None,
            local_docker_ports: None,
            providers: vec!["codex".to_string()],
            provider_auth: Vec::new(),
            display_endpoint: None,
            created_at_ms: 1000,
            updated_at_ms: 2000,
        },
        login: crate::slice::SliceProviderLoginStart {
            provider: "codex".to_string(),
            login_kind: "device".to_string(),
            auth_url: Some("https://auth.example".to_string()),
            verification_url: Some("https://auth.example".to_string()),
            user_code: Some("ABCD-EFGH".to_string()),
            status: "started".to_string(),
            message: "Open https://auth.example and enter ABCD-EFGH".to_string(),
        },
    };
    let snapshot = serde_json::json!([request, response]);
    assert_eq!(
        snapshot.pointer("/0/StartSliceProviderLogin/slice_ref"),
        Some(&serde_json::json!("linux-dev"))
    );
    assert_eq!(
        snapshot.pointer("/1/SliceProviderLoginStarted/login/user_code"),
        Some(&serde_json::json!("ABCD-EFGH"))
    );
    assert_eq!(
        snapshot.pointer("/1/SliceProviderLoginStarted/login/verification_url"),
        Some(&serde_json::json!("https://auth.example"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("slice provider login snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "681ecfa58bcb97453a041a6da4528d70d29ab0aa877a498110c2a3a129cde03e"
    );
}

#[test]
fn local_daemon_protocol_slice_logs_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 83);

    let request = LocalDaemonRequest::GetSliceLogs(crate::local::GetSliceLogsRequest {
        slice_ref: "linux-dev".to_string(),
        tail_lines: Some(50),
    });
    let response = LocalDaemonResponse::SliceLogs {
        slice: crate::slice::SliceRecord {
            id: "slice-1".to_string(),
            name: "linux-dev".to_string(),
            owner_kernel_id: "home-kernel".to_string(),
            owner_machine_id: "home-machine".to_string(),
            session_id: None,
            session_ids: Vec::new(),
            agent_ids: Vec::new(),
            backend: crate::slice::SliceBackendKind::LocalDocker,
            os: "linux".to_string(),
            display_mode: crate::slice::SliceDisplayMode::Headless,
            status: crate::slice::SliceStatus::Running,
            last_operation: None,
            last_operation_status: None,
            last_error: None,
            last_operation_at_ms: None,
            workspace_id: Some("workspace-1".to_string()),
            worktree_id: Some("worktree-1".to_string()),
            workspace_mount: Some("/repo".to_string()),
            worker_kernel_ref: "slice:linux-dev".to_string(),
            worker_kernel_id: Some("slice-worker".to_string()),
            worker_machine_id: Some("slice:slice-1".to_string()),
            relay_endpoint: None,
            local_docker_ports: None,
            providers: vec!["codex".to_string()],
            provider_auth: Vec::new(),
            display_endpoint: None,
            created_at_ms: 1000,
            updated_at_ms: 2000,
        },
        entries: vec![crate::slice::SliceLogEntry {
            source: "provision".to_string(),
            path: Some("/tmp/arroba-slice.log".to_string()),
            text: "slice booted".to_string(),
            truncated: false,
        }],
    };
    let snapshot = serde_json::json!([request, response]);
    assert_eq!(
        snapshot.pointer("/0/GetSliceLogs/tail_lines"),
        Some(&serde_json::json!(50))
    );
    assert_eq!(
        snapshot.pointer("/1/SliceLogs/entries/0/source"),
        Some(&serde_json::json!("provision"))
    );
    let serialized = serde_json::to_string(&snapshot).expect("slice logs snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "258b21b42d5b468b610fd895ff58ee043afb80b253e95e8e2daf733515d0b472"
    );
}

#[test]
fn local_daemon_protocol_semantic_recall_search_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 83);

    let request = LocalDaemonRequest::SemanticSearchRecall(SemanticSearchRecallRequest {
        query: "why did the build fail".to_string(),
        mode: Some(crate::local::SemanticSearchRecallMode::Agent),
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
        request_snapshot.pointer("/SemanticSearchRecall/query"),
        Some(&serde_json::json!("why did the build fail"))
    );
    assert_eq!(
        request_snapshot.pointer("/SemanticSearchRecall/mode"),
        Some(&serde_json::json!("agent"))
    );
    assert_eq!(
        request_snapshot.pointer("/SemanticSearchRecall/session_id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        request_snapshot.pointer("/SemanticSearchRecall/limit"),
        Some(&serde_json::json!(12))
    );
    assert_eq!(
        request_snapshot.pointer("/SemanticSearchRecall/cursor"),
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
    let response = LocalDaemonResponse::SemanticRecallEvents {
        results: vec![SemanticRecallMatch {
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
        response_snapshot.pointer("/SemanticRecallEvents/results/0/score_millis"),
        Some(&serde_json::json!(914))
    );
    assert_eq!(
        response_snapshot.pointer("/SemanticRecallEvents/results/0/chunk_text"),
        Some(&serde_json::json!("build failed because tests failed"))
    );
    assert_eq!(
        response_snapshot.pointer("/SemanticRecallEvents/results/0/reason"),
        Some(&serde_json::json!("high: direct match"))
    );
    assert_eq!(
        response_snapshot.pointer("/SemanticRecallEvents/answer"),
        Some(&serde_json::json!("The build failed because tests failed."))
    );
    assert_eq!(
        response_snapshot.pointer("/SemanticRecallEvents/unavailable_reason"),
        Some(&serde_json::Value::Null)
    );

    let serialized = serde_json::to_string(&serde_json::json!({
        "request": request_snapshot,
        "response": response_snapshot,
    }))
    .expect("semantic recall snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "5c28f67e35c7840b0f754e4d635fce1670dba9064756a90893204415418f5d2a"
    );
}

#[test]
fn local_daemon_protocol_query_recall_context_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 83);

    let request = LocalDaemonRequest::QueryRecall(QueryRecallRequest {
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
        snapshot.pointer("/QueryRecall/before_sequence"),
        Some(&serde_json::json!(42))
    );

    let serialized = serde_json::to_string(&snapshot).expect("query recall snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "4e8f0791f65d7a0d7df9d983cafe6584d2e155bd59d0f0a51f176d6f50ba7485"
    );
}

#[test]
fn local_daemon_protocol_agent_config_workspace_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 83);

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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 83);

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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 83);

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
