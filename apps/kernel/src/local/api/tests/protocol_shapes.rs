use super::*;

#[test]
fn local_daemon_protocol_waiting_room_activity_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

    let summary = crate::local::WaitingRoomSessionActivitySummary {
        agent_count: 4,
        working_agent_count: 1,
        active_prompt_count: 1,
        queued_prompt_count: 2,
        error_agent_count: 1,
        remote_agent_count: 3,
        missing_worker_provider_run_count: 1,
        home_proxy_agent_count: 2,
        remote_extension_sync_issue_count: 1,
        remote_extension_pending_revoke_count: 1,
        unread_idle_agent_count: 1,
    };

    assert_eq!(
        serde_json::to_value(summary).expect("waiting-room activity summary should encode"),
        serde_json::json!({
            "agent_count": 4,
            "working_agent_count": 1,
            "active_prompt_count": 1,
            "queued_prompt_count": 2,
            "error_agent_count": 1,
            "remote_agent_count": 3,
            "missing_worker_provider_run_count": 1,
            "home_proxy_agent_count": 2,
            "remote_extension_sync_issue_count": 1,
            "remote_extension_pending_revoke_count": 1,
            "unread_idle_agent_count": 1
        })
    );
}

#[test]
fn local_daemon_protocol_transport_health_relay_reconnect_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

    let snapshot = crate::runtime::projection::TransportHealthSnapshot {
        active_connections: 1,
        active_subscriptions: 2,
        retained_event_limit: 256,
        command_result_cache_limit: 512,
        inbound_request_limit: 8,
        incoming_requests: 3,
        emitted_events: 4,
        replay_gaps: 5,
        inbound_overload_rejections: 6,
        duplicate_command_conflicts: 7,
        outgoing_queue_overflows: 8,
        slow_consumer_closes: 9,
        relay_reconnect_attempts: 10,
        relay_last_reconnect_reason: Some("relay heartbeat send failed".to_string()),
        relay_last_reconnect_delay_ms: Some(750),
        relay_last_reconnect_url: Some("wss://relay-b.example.test".to_string()),
        relay_last_connected_url: Some("wss://relay-a.example.test".to_string()),
    };

    assert_eq!(
        serde_json::to_value(snapshot).expect("transport health snapshot should encode"),
        serde_json::json!({
            "active_connections": 1,
            "active_subscriptions": 2,
            "retained_event_limit": 256,
            "command_result_cache_limit": 512,
            "inbound_request_limit": 8,
            "incoming_requests": 3,
            "emitted_events": 4,
            "replay_gaps": 5,
            "inbound_overload_rejections": 6,
            "duplicate_command_conflicts": 7,
            "outgoing_queue_overflows": 8,
            "slow_consumer_closes": 9,
            "relay_reconnect_attempts": 10,
            "relay_last_reconnect_reason": "relay heartbeat send failed",
            "relay_last_reconnect_delay_ms": 750,
            "relay_last_reconnect_url": "wss://relay-b.example.test",
            "relay_last_connected_url": "wss://relay-a.example.test"
        })
    );
}

#[test]
fn local_daemon_protocol_queued_prompt_controls_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

    let steer_request =
        LocalDaemonRequest::SteerQueuedPrompt(crate::local::SteerQueuedPromptRequest {
            session_id: "session-1".to_string(),
            attachment_id: "attachment-1".to_string(),
            target_agent_id: "agent-1".to_string(),
            prompt_id: "prompt-queued".to_string(),
        });
    let steer_snapshot = serde_json::to_value(steer_request).expect("steer request should encode");
    assert_eq!(
        steer_snapshot.pointer("/SteerQueuedPrompt/prompt_id"),
        Some(&serde_json::json!("prompt-queued"))
    );

    let cancel_request =
        LocalDaemonRequest::CancelQueuedPrompt(crate::local::CancelQueuedPromptRequest {
            session_id: "session-1".to_string(),
            attachment_id: "attachment-1".to_string(),
            target_agent_id: "agent-1".to_string(),
            prompt_id: "prompt-queued".to_string(),
        });
    let cancel_snapshot =
        serde_json::to_value(cancel_request).expect("cancel request should encode");
    assert_eq!(
        cancel_snapshot.pointer("/CancelQueuedPrompt/target_agent_id"),
        Some(&serde_json::json!("agent-1"))
    );

    let prompt = crate::session::PromptQueueItem::new(
        "prompt-queued",
        "attachment-1",
        "agent-1",
        "queued text",
        crate::session::PromptStatus::Queued,
    );
    let session = crate::session::RuntimeSession::new(
        "session-1",
        None,
        "workspace-1",
        "worktree-1",
        "machine-1",
        "daemon-1",
    );
    let steer_response = LocalDaemonResponse::QueuedPromptSteered {
        prompt: prompt.clone(),
        session: session.clone(),
        agent_activity: std::collections::BTreeMap::new(),
        agent_activity_revision: 7,
    };
    let steer_response_snapshot =
        serde_json::to_value(steer_response).expect("steer response should encode");
    assert_eq!(
        steer_response_snapshot.pointer("/QueuedPromptSteered/prompt/id"),
        Some(&serde_json::json!("prompt-queued"))
    );
    assert_eq!(
        steer_response_snapshot.pointer("/QueuedPromptSteered/prompt/prompt_origin"),
        Some(&serde_json::json!("arroba"))
    );
    assert_eq!(
        steer_response_snapshot.pointer("/QueuedPromptSteered/agent_activity_revision"),
        Some(&serde_json::json!(7))
    );

    let cancel_response = LocalDaemonResponse::QueuedPromptCancelled { prompt, session };
    let cancel_response_snapshot =
        serde_json::to_value(cancel_response).expect("cancel response should encode");
    assert_eq!(
        cancel_response_snapshot.pointer("/QueuedPromptCancelled/session/id"),
        Some(&serde_json::json!("session-1"))
    );
}

#[test]
fn local_daemon_protocol_move_agent_to_local_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

    let request = LocalDaemonRequest::MoveAgentToLocal(MoveAgentToLocalRequest {
        session_id: "session-1".to_string(),
        agent_ref: "agent-1".to_string(),
    });
    let mut agent_value = serde_json::to_value(crate::agent::AgentInstance::new(
        "agent-1",
        "agent-ref-1",
        "session-1",
        None,
        "codex",
        None,
        None,
        None,
        crate::agent::GridPosition::new(0, 0, 1, 1),
    ))
    .expect("agent snapshot should encode");
    agent_value["created_at_ms"] = serde_json::json!(1_000);
    agent_value["last_activity_at_ms"] = serde_json::json!(1_000);
    let response = LocalDaemonResponse::AgentMovedToLocal {
        agent: serde_json::from_value(agent_value).expect("agent snapshot should decode"),
    };

    let snapshot = serde_json::json!([request, response]);
    assert_eq!(
        snapshot.pointer("/0/MoveAgentToLocal/session_id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        snapshot.pointer("/1/AgentMovedToLocal/agent/id"),
        Some(&serde_json::json!("agent-1"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("move agent to local snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "f6e7e181738b182da73d2156f9f876698b70f464d03f1becd6e62d4dc21e3196"
    );
}

#[test]
fn local_daemon_protocol_workflow_publication_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

    let create_request = LocalDaemonRequest::CreateWorkflowPublication(
        crate::local::CreateWorkflowPublicationRequest {
            session_id: "session-1".to_string(),
            workflow_ref: "workflow-1".to_string(),
            endpoint_ref: "endpoint-1".to_string(),
            queue_ref: Some("priority".to_string()),
            alias: Some("public_qa".to_string()),
            route: Some("/qa/*".to_string()),
            methods: vec!["GET".to_string()],
            transport: Some(serde_json::json!({"kind": "human_http"})),
            parser: Some(serde_json::json!({"kind": "path_template", "template": "/qa/:task"})),
            input_schema: Some(serde_json::json!({"type": "object"})),
            trace_exposure: Some(serde_json::json!({
                "nodes": {
                    "node-1": ["output_summary", "assistant_messages", "tool_use"]
                }
            })),
            mode: Some("async".to_string()),
            sync_timeout_ms: Some(240_000),
            poll_ms: Some(250),
        },
    );
    let list_request = LocalDaemonRequest::ListWorkflowPublications(
        crate::local::ListWorkflowPublicationsRequest {
            session_id: "session-1".to_string(),
        },
    );
    let get_request =
        LocalDaemonRequest::GetWorkflowPublication(crate::local::GetWorkflowPublicationRequest {
            session_id: "session-1".to_string(),
            publication_ref: "publication-1".to_string(),
        });
    let export_request = LocalDaemonRequest::ExportWorkflowPublicationPackage(
        crate::local::ExportWorkflowPublicationPackageRequest {
            session_id: "session-1".to_string(),
            publication_ref: "publication-1".to_string(),
            kernel_url: Some("ws://127.0.0.1:43118".to_string()),
            agent_app: Some(serde_json::json!({
                "enabled": true,
                "assets": {
                    "public_dir": "app",
                    "index": "index.html"
                },
                "routes": [{
                    "path": "/add/*",
                    "hook_id": "publication-1-hook",
                    "prompt_source": "path_tail",
                    "response": "streaming_shell",
                    "required_role": "public",
                    "manipulation": {
                        "level": "state_and_overlay",
                        "scope": "session",
                        "allowed_paths": ["/generated/**"],
                        "protected_paths": ["/auth/**"],
                        "allowed_actions": ["cart.search", "cart.add"]
                    }
                }],
                "replicas": {
                    "count": 2,
                    "per_caller_ordering": true,
                    "max_queue_depth": 100
                },
                "persistent_patch": {
                    "enabled": false
                }
            })),
            agent_app_assets_dir: Some("/repo/dist".to_string()),
        },
    );
    let disable_request = LocalDaemonRequest::DisableWorkflowPublication(
        crate::local::DisableWorkflowPublicationRequest {
            session_id: "session-1".to_string(),
            publication_ref: "publication-1".to_string(),
        },
    );
    let register_endpoint_request = LocalDaemonRequest::RegisterWorkflowPublicationEndpoint(
        crate::local::RegisterWorkflowPublicationEndpointRequest {
            session_id: "session-1".to_string(),
            publication_ref: "publication-1".to_string(),
            local_url: "http://127.0.0.1:3000/".to_string(),
            runtime_session_id: Some("runtime-session-1".to_string()),
            ttl_ms: Some(600_000),
        },
    );
    let mut workflow =
        crate::session::WorkflowDefinition::new("workflow-1", Some("qa".to_string()));
    workflow.add_node(crate::session::WorkflowNodeDefinition::new(
        "node-1", "agent-1",
    ));
    let endpoint = workflow.add_endpoint(crate::session::WorkflowEndpointDefinition::new(
        "endpoint-1",
        Some("ask".to_string()),
        "node-1",
    ));
    let snapshot = crate::local::WorkflowPublicationSnapshot {
        schema_version: 1,
        captured_at_ms: Some(42),
        source_session: Some(crate::local::WorkflowPublicationSourceSessionSnapshot {
            id: Some("session-1".to_string()),
            alias: Some("authoring".to_string()),
            workspace_id: "/repo".to_string(),
            worktree_id: "/repo".to_string(),
        }),
        workflow: workflow.clone(),
        endpoint: Some(endpoint.clone()),
        queues: vec![crate::session::WorkflowPromptQueueDefinition::new(
            "workflow-1:default",
            "workflow-1",
            "default",
            0,
        )],
        watchdogs: vec![crate::session::WorkflowWatchdogDefinition::new(
            "watchdog-1",
            "workflow-1",
            "endpoint-1",
            60,
            "scheduled prompt",
            crate::session::WorkflowWatchdogPolicy::Queue,
            Some(3),
        )],
        agents: vec![crate::agent::AgentInstance::new(
            "agent-1",
            "agent-ref-1",
            "session-1",
            Some("worker".to_string()),
            "codex",
            Some("gpt-5".to_string()),
            None,
            Some("/repo".to_string()),
            crate::agent::GridPosition::new(0, 0, 1, 1),
        )],
    };
    let materialize_request = LocalDaemonRequest::MaterializeWorkflowPublication(
        crate::local::MaterializeWorkflowPublicationRequest {
            publication_id: "publication-1".to_string(),
            snapshot,
        },
    );
    let publication = crate::session::WorkflowPublicationDefinition::new(
        "publication-1",
        "session-1",
        "workflow-1",
        "endpoint-1",
        Some("priority".to_string()),
        Some("public_qa".to_string()),
        Some("/qa/*".to_string()),
        vec!["GET".to_string()],
        Some(serde_json::json!({"kind": "human_http"})),
        Some(serde_json::json!({"kind": "path_template", "template": "/qa/:task"})),
        Some(serde_json::json!({"type": "object"})),
        Some(serde_json::json!({
            "nodes": {
                "node-1": ["output_summary", "assistant_messages", "tool_use"]
            }
        })),
        Some("async".to_string()),
        Some(240_000),
        Some(250),
        "local",
    );
    let session = crate::session::RuntimeSession::new(
        "session-1",
        None,
        "/repo",
        "/repo",
        "machine-1",
        "daemon-1",
    );
    let mut served_publication = publication.clone();
    served_publication.mark_served(
        "running",
        "https://relay.example.test/display/publication-1/",
        serde_json::json!({
            "kind": "tunnel",
            "url": "https://relay.example.test/display/publication-1/",
            "local_url": "http://127.0.0.1:3000/",
            "runtime_session_id": "runtime-session-1",
            "expires_at_ms": 600_042
        }),
    );
    let mut materialized_session = crate::session::RuntimeSession::new(
        "runtime-session-1",
        None,
        "/repo",
        "/repo",
        "machine-1",
        "daemon-1",
    );
    materialized_session.set_hidden(true);
    let snapshot = serde_json::json!([
        create_request,
        list_request,
        get_request,
        export_request,
        disable_request,
        materialize_request,
        LocalDaemonResponse::WorkflowPublicationCreated {
            publication: publication.clone(),
            session: session.clone(),
        },
        LocalDaemonResponse::WorkflowPublicationsListed {
            publications: vec![publication.clone()],
        },
        LocalDaemonResponse::WorkflowPublication {
            publication: publication.clone(),
        },
        LocalDaemonResponse::WorkflowPublicationPackageExported {
            publication: publication.clone(),
            package_version: 2,
            package_digest: "sha256:abc123".to_string(),
            package_archive_base64: "YXJjaGl2ZQ==".to_string(),
            package_files: vec![crate::local::WorkflowPublicationPackageFile {
                path: "publication.json".to_string(),
                content_base64: "e30K".to_string(),
                executable: false,
            }],
        },
        LocalDaemonResponse::WorkflowPublicationDisabled {
            publication: publication.clone(),
            session: session.clone(),
        },
        LocalDaemonResponse::WorkflowPublicationMaterialized {
            publication_id: "publication-1".to_string(),
            session: materialized_session,
            agent_id_map: BTreeMap::from([("agent-1".to_string(), "agent-2".to_string(),)]),
        },
        register_endpoint_request,
        LocalDaemonResponse::WorkflowPublicationEndpointRegistered {
            publication: served_publication,
            open_url: "https://relay.example.test/display/publication-1/".to_string(),
            access: "tunnel".to_string(),
            expires_at_ms: Some(600_042),
        },
    ]);
    let mut snapshot = snapshot;
    for path in [
        "/5/MaterializeWorkflowPublication/snapshot/workflow/created_at_ms",
        "/5/MaterializeWorkflowPublication/snapshot/queues/0/created_at_ms",
        "/5/MaterializeWorkflowPublication/snapshot/queues/0/updated_at_ms",
        "/5/MaterializeWorkflowPublication/snapshot/watchdogs/0/created_at_ms",
        "/5/MaterializeWorkflowPublication/snapshot/watchdogs/0/next_run_at_ms",
        "/5/MaterializeWorkflowPublication/snapshot/watchdogs/0/updated_at_ms",
        "/5/MaterializeWorkflowPublication/snapshot/agents/0/created_at_ms",
        "/5/MaterializeWorkflowPublication/snapshot/agents/0/last_activity_at_ms",
        "/6/WorkflowPublicationCreated/publication/created_at_ms",
        "/6/WorkflowPublicationCreated/publication/updated_at_ms",
        "/6/WorkflowPublicationCreated/session/created_at_ms",
        "/6/WorkflowPublicationCreated/session/last_used_at_ms",
        "/7/WorkflowPublicationsListed/publications/0/created_at_ms",
        "/7/WorkflowPublicationsListed/publications/0/updated_at_ms",
        "/8/WorkflowPublication/publication/created_at_ms",
        "/8/WorkflowPublication/publication/updated_at_ms",
        "/9/WorkflowPublicationPackageExported/publication/created_at_ms",
        "/9/WorkflowPublicationPackageExported/publication/updated_at_ms",
        "/10/WorkflowPublicationDisabled/publication/created_at_ms",
        "/10/WorkflowPublicationDisabled/publication/updated_at_ms",
        "/10/WorkflowPublicationDisabled/session/created_at_ms",
        "/10/WorkflowPublicationDisabled/session/last_used_at_ms",
        "/11/WorkflowPublicationMaterialized/session/created_at_ms",
        "/11/WorkflowPublicationMaterialized/session/last_used_at_ms",
        "/13/WorkflowPublicationEndpointRegistered/publication/created_at_ms",
        "/13/WorkflowPublicationEndpointRegistered/publication/updated_at_ms",
    ] {
        *snapshot
            .pointer_mut(path)
            .expect("timestamp path should encode") = serde_json::json!(42);
    }

    assert_eq!(
        snapshot.pointer("/0/CreateWorkflowPublication/transport/kind"),
        Some(&serde_json::json!("human_http"))
    );
    assert_eq!(
        snapshot.pointer("/0/CreateWorkflowPublication/queue_ref"),
        Some(&serde_json::json!("priority"))
    );
    assert_eq!(
        snapshot.pointer("/6/WorkflowPublicationCreated/publication/queue_ref"),
        Some(&serde_json::json!("priority"))
    );
    assert_eq!(
        snapshot.pointer("/0/CreateWorkflowPublication/trace_exposure/nodes/node-1/1"),
        Some(&serde_json::json!("assistant_messages"))
    );
    assert_eq!(
        snapshot.pointer("/6/WorkflowPublicationCreated/publication/trace_exposure/nodes/node-1/2"),
        Some(&serde_json::json!("tool_use"))
    );
    assert_eq!(
        snapshot.pointer("/0/CreateWorkflowPublication/sync_timeout_ms"),
        Some(&serde_json::json!(240_000))
    );
    assert_eq!(
        snapshot.pointer("/6/WorkflowPublicationCreated/publication/poll_ms"),
        Some(&serde_json::json!(250))
    );
    assert_eq!(
        snapshot.pointer("/12/RegisterWorkflowPublicationEndpoint/local_url"),
        Some(&serde_json::json!("http://127.0.0.1:3000/"))
    );
    assert_eq!(
        snapshot.pointer("/3/ExportWorkflowPublicationPackage/agent_app/routes/0/path"),
        Some(&serde_json::json!("/add/*"))
    );
    assert_eq!(
        snapshot.pointer("/9/WorkflowPublicationPackageExported/package_version"),
        Some(&serde_json::json!(2))
    );
    assert_eq!(
        snapshot.pointer("/13/WorkflowPublicationEndpointRegistered/publication/status"),
        Some(&serde_json::json!("running"))
    );
    assert_eq!(
        snapshot.pointer("/13/WorkflowPublicationEndpointRegistered/publication/open_url"),
        Some(&serde_json::json!(
            "https://relay.example.test/display/publication-1/"
        ))
    );
    assert_eq!(snapshot.pointer("/0/CreateWorkflowPublication/auth"), None);
    assert_eq!(
        snapshot.pointer("/6/WorkflowPublicationCreated/publication/auth"),
        None
    );
    assert_eq!(
        snapshot.pointer("/6/WorkflowPublicationCreated/publication/pairing_codes"),
        None
    );
    assert_eq!(
        snapshot.pointer("/6/WorkflowPublicationCreated/publication/trusted_senders"),
        None
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("workflow publication shape should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "4410134a7f7d1b611ebbe8c75478d062ba90c4d1d7b13619fe131e7a03ed4f67"
    );
}

#[test]
fn local_daemon_protocol_publication_invocation_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

    let request =
        LocalDaemonRequest::InvokeWorkflowEndpoint(crate::local::InvokeWorkflowEndpointRequest {
            session_id: "runtime-session-1".to_string(),
            workflow_ref: "workflow-1".to_string(),
            endpoint_ref: "endpoint-1".to_string(),
            queue_ref: Some("default".to_string()),
            prompt: Some("make tea".to_string()),
            publication_invocation: Some(crate::session::WorkflowPublicationInvocationEnvelope {
                publication_id: "publication-1".to_string(),
                hook_id: Some("hook-1".to_string()),
                invocation_id: "req-1".to_string(),
                transport: "human_http".to_string(),
                endpoint_id: "endpoint-1".to_string(),
                queue_ref: Some("default".to_string()),
                input: serde_json::json!({ "prompt": "make tea" }),
                artifacts: vec![serde_json::json!({
                    "id": "artifact-1",
                    "name": "image.png",
                    "media_type": "image/png"
                })],
                mode: Some("sync".to_string()),
                caller: serde_json::json!({
                    "type": "anonymous",
                    "proof": { "transport": "human_http" }
                }),
            }),
        });
    let snapshot = serde_json::json!([request]);

    assert_eq!(
        snapshot.pointer("/0/InvokeWorkflowEndpoint/prompt"),
        Some(&serde_json::json!("make tea"))
    );
    assert_eq!(
        snapshot.pointer("/0/InvokeWorkflowEndpoint/publication_invocation/invocation_id"),
        Some(&serde_json::json!("req-1"))
    );
    assert_eq!(
        snapshot.pointer("/0/InvokeWorkflowEndpoint/publication_invocation/input/prompt"),
        Some(&serde_json::json!("make tea"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("publication invocation shape should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "b82f8e8a01b0c282bf05883bd81ae8aa21320eab0ab95a3f2dfa4059471daa67"
    );
}

#[test]
fn local_daemon_protocol_debug_bundle_export_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

    let request = LocalDaemonRequest::ExportDebugBundle(ExportDebugBundleRequest {
        session_id: "session-1".to_string(),
        bundle_label: Some("support".to_string()),
        limit: Some(500),
    });
    let response = LocalDaemonResponse::DebugBundleExported {
        bundle_dir: "/state/arroba/debug-bundles/session-1-support".to_string(),
        manifest_path: "/state/arroba/debug-bundles/session-1-support/manifest.json".to_string(),
        logs_path: "/state/arroba/debug-bundles/session-1-support/logs.ndjson".to_string(),
        log_root: "/state/arroba/logs".to_string(),
        record_count: 12,
        limit: 500,
    };
    let snapshot = serde_json::json!([request, response]);

    assert_eq!(
        snapshot.pointer("/0/ExportDebugBundle/session_id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        snapshot.pointer("/0/ExportDebugBundle/bundle_label"),
        Some(&serde_json::json!("support"))
    );
    assert_eq!(
        snapshot.pointer("/1/DebugBundleExported/record_count"),
        Some(&serde_json::json!(12))
    );
    let serialized = serde_json::to_string(&snapshot).expect("debug-bundle snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "554ddc4f229fd9eb7249eace067d171a837deebf13029664a75777948bc91171"
    );
}

#[test]
fn local_daemon_protocol_workspace_live_sync_status_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

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
        effects: vec![crate::local::UserConfigMutationEffect {
            kind: "provider_reload".to_string(),
            path: "session.workspace_live_sync_mode".to_string(),
            message:
                "session workspace live sync mode updated; provider reloads: 1 reloaded, 1 deferred, 0 unaffected"
                    .to_string(),
            provider_reload: Some(crate::local::UserConfigProviderReloadSummary {
                reloaded: 1,
                deferred: 1,
                unaffected: 0,
            }),
        }],
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
        "9950f133fd73a94870eba88b30b5b4de69ee2d07be1f2881533913aa72cf4a47"
    );
}

#[test]
fn local_daemon_protocol_session_history_outline_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

    let request = LocalDaemonRequest::GetSessionHistoryOutline(
        crate::local::GetSessionHistoryOutlineRequest {
            session_id: "session-1".to_string(),
            agent_ids: Some(vec!["agent-1".to_string(), "agent-2".to_string()]),
            latest_prompt_count: Some(4),
            cursor: Some(crate::local::SessionHistoryOutlineCursor {
                before_sequence: 10,
            }),
        },
    );
    let blob_request = LocalDaemonRequest::GetSessionHistoryBlobContent(
        crate::local::GetSessionHistoryBlobContentRequest {
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            blob_id: "history:11:11".to_string(),
        },
    );
    let mut user_prompt = history_page_entry(
        10,
        crate::history::SessionHistoryEntryKind::UserPrompt,
        "agent-1",
        "prompt",
    );
    user_prompt.entry.attachments = vec![crate::history::SessionHistoryPromptAttachment {
        url: "arroba-terminal://prompt-attachment/attachment-1/Screenshot.png".to_string(),
        mime: "image/png".to_string(),
        filename: Some("Screenshot.png".to_string()),
        preview_url: Some("data:image/png;base64,aW1hZ2U=".to_string()),
    }];
    let summary = history_page_entry(
        12,
        crate::history::SessionHistoryEntryKind::ProviderOutput,
        "agent-1",
        "summary",
    );
    let mut assistant_entry = history_page_entry(
        13,
        crate::history::SessionHistoryEntryKind::ProviderOutput,
        "agent-1",
        "assistant detail",
    );
    assistant_entry.entry.source =
        Some(crate::history::SessionHistoryEntrySource::ExternalProviderObserved);
    assistant_entry.entry.external_provider = Some("codex".to_string());
    assistant_entry.entry.external_provider_session_id = Some("thread-1".to_string());
    assistant_entry.entry.external_provider_turn_id = Some("turn-1".to_string());
    assistant_entry.entry.observed_at_ms = Some(42);
    let blob_entry = history_page_entry(
        11,
        crate::history::SessionHistoryEntryKind::ProviderTool,
        "agent-1",
        "{\"tool\":\"bash\",\"status\":\"completed\"}",
    );
    let outline = LocalDaemonResponse::SessionHistoryOutline {
        agents: vec![crate::local::SessionHistoryOutlineAgent {
            agent_id: "agent-1".to_string(),
            turns: vec![crate::local::SessionHistoryOutlineTurn {
                turn_id: "turn-1".to_string(),
                prompt_id: Some("prompt-1".to_string()),
                started_at_ms: 42,
                user_prompt,
                entries: vec![assistant_entry],
                summary: Some(summary),
                blobs: vec![crate::local::SessionHistoryOutlineBlob {
                    blob_id: "history:11:11".to_string(),
                    kind: crate::history::SessionHistoryEntryKind::ProviderTool,
                    title: "bash · COMPLETED".to_string(),
                    summary: "$ cargo test".to_string(),
                    sequence_start: 11,
                    sequence_end: 11,
                    entry_count: 1,
                    total_chars: 38,
                    timestamp_ms: 43,
                }],
            }],
            next_cursor: Some(crate::local::SessionHistoryOutlineCursor {
                before_sequence: 10,
            }),
        }],
    };
    let blob_response = LocalDaemonResponse::SessionHistoryBlobContent {
        blob_id: "history:11:11".to_string(),
        entries: vec![blob_entry],
    };

    let snapshot = serde_json::json!([request, blob_request, outline, blob_response]);
    assert_eq!(
        snapshot.pointer("/0/GetSessionHistoryOutline/latest_prompt_count"),
        Some(&serde_json::json!(4))
    );
    assert_eq!(
        snapshot.pointer("/0/GetSessionHistoryOutline/cursor/before_sequence"),
        Some(&serde_json::json!(10))
    );
    assert_eq!(
        snapshot.pointer("/2/SessionHistoryOutline/agents/0/turns/0/entries/0/entry/text"),
        Some(&serde_json::json!("assistant detail"))
    );
    assert_eq!(
        snapshot.pointer(
            "/2/SessionHistoryOutline/agents/0/turns/0/user_prompt/entry/attachments/0/filename"
        ),
        Some(&serde_json::json!("Screenshot.png"))
    );
    assert_eq!(
        snapshot.pointer(
            "/2/SessionHistoryOutline/agents/0/turns/0/user_prompt/entry/attachments/0/preview_url"
        ),
        Some(&serde_json::json!("data:image/png;base64,aW1hZ2U="))
    );
    assert_eq!(
        snapshot.pointer("/2/SessionHistoryOutline/agents/0/turns/0/entries/0/entry/source"),
        Some(&serde_json::json!("external_provider_observed"))
    );
    assert_eq!(
        snapshot.pointer("/2/SessionHistoryOutline/agents/0/turns/0/blobs/0/blob_id"),
        Some(&serde_json::json!("history:11:11"))
    );
    assert_eq!(
        snapshot.pointer("/3/SessionHistoryBlobContent/entries/0/entry/kind"),
        Some(&serde_json::json!("provider_tool"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("session history outline shape should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "37145a72de46a72852ee918d29bdbb17e8478aba214e3742a7f845f1a73625ff"
    );
}

#[test]
fn local_daemon_protocol_provider_process_memory_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

    let request =
        LocalDaemonRequest::ListProviderProcesses(crate::local::ListProviderProcessesRequest {
            provider: Some("codex".to_string()),
        });
    let response = LocalDaemonResponse::ProviderProcessesListed {
        processes: vec![crate::provider::ProviderProcessInfo {
            process_id: "managed:codex:process-1".to_string(),
            provider: "codex".to_string(),
            process_label: "codex:default".to_string(),
            pid: Some(4321),
            resident_set_bytes: Some(134_217_728),
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            status: crate::provider::ProviderProcessStatus::Active,
            started_at_ms: 1,
            last_activity_at_ms: 2,
            provider_session_ids: vec!["thread-1".to_string()],
            owner_session_ids: vec!["session-1".to_string()],
            owner_provider_run_ids: vec!["provider-run-1".to_string()],
            attached_session_ids: vec!["session-1".to_string()],
            active_workflow_run_ids: Vec::new(),
            teardown_safe: false,
            teardown_blockers: vec!["attached sessions: session-1".to_string()],
        }],
    };

    let snapshot = serde_json::json!([request, response]);
    assert_eq!(
        snapshot.pointer("/0/ListProviderProcesses/provider"),
        Some(&serde_json::json!("codex"))
    );
    assert_eq!(
        snapshot.pointer("/1/ProviderProcessesListed/processes/0/resident_set_bytes"),
        Some(&serde_json::json!(134_217_728))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("provider process memory shape should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "300ae3afacdf79ffa6507654b76db48818a563cff5cab6e715d8f8b887f6509a"
    );
}

#[test]
fn local_daemon_protocol_external_provider_session_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

    let request = LocalDaemonRequest::ListExternalProviderSessions(
        crate::local::ListExternalProviderSessionsRequest {
            provider: Some("codex".to_string()),
            cursor: Some("cursor-1".to_string()),
            limit: Some(25),
        },
    );
    let record = crate::local::ExternalProviderSessionRecord {
        external_session_id: "codex:thread-1".to_string(),
        provider: "codex".to_string(),
        provider_session_id: "thread-1".to_string(),
        title: Some("Investigate failing tests".to_string()),
        title_source: Some("provider_title".to_string()),
        first_prompt_preview: Some("Investigate failing tests in the CLI".to_string()),
        created_at_ms: Some(10),
        last_modified_at_ms: 20,
        worktree_path: Some("/repo".to_string()),
        account_profile: Some("default".to_string()),
        capabilities: crate::local::ExternalProviderSessionCapabilities {
            can_read_history: true,
        },
        attached_to_arroba: true,
        attached_session_ids: vec!["session-1".to_string()],
        attached_agent_ids: vec!["agent-1".to_string()],
    };
    let response = LocalDaemonResponse::ExternalProviderSessionsListed {
        page: crate::local::ExternalProviderSessionPage {
            sessions: vec![record],
            next_cursor: Some("cursor-2".to_string()),
            has_more: true,
            generated_at_ms: 30,
        },
    };

    let snapshot = serde_json::json!([request, response]);
    assert_eq!(
        snapshot.pointer("/0/ListExternalProviderSessions/provider"),
        Some(&serde_json::json!("codex"))
    );
    assert_eq!(
        snapshot.pointer(
            "/1/ExternalProviderSessionsListed/page/sessions/0/capabilities/can_read_history"
        ),
        Some(&serde_json::json!(true))
    );
    assert_eq!(
        snapshot.pointer("/1/ExternalProviderSessionsListed/page/sessions/0/attached_to_arroba"),
        None
    );
    assert_eq!(
        snapshot.pointer("/1/ExternalProviderSessionsListed/page/sessions/0/attached_agent_ids"),
        None
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("external provider session shape should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "6b01adcc578bdf90bbe162035d023da1b5e2b2805f81e9774148a48cd0ebab6a"
    );
}

#[test]
fn relay_workspace_live_sync_apply_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

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
fn relay_home_credential_proxy_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

    let context = crate::transport::relay_peer::RemoteExtensionInvocationContext {
        home_kernel_id: "home-kernel".to_string(),
        home_session_id: "session-1".to_string(),
        home_agent_id: "agent-1".to_string(),
        leased_agent_id: "leased-agent-1".to_string(),
        worker_provider_run_id: "provider-run-1".to_string(),
        worker_kernel_id: Some("worker-kernel".to_string()),
        worker_machine_id: Some("worker-machine".to_string()),
    };
    let list_request = crate::transport::relay_peer::RelayPeerRequest::InvokeHomeCredentialTool {
        context: context.clone(),
        tool_name: crate::transport::runtime_tools::LIST_CREDENTIAL_HANDLES_TOOL.to_string(),
        arguments: serde_json::json!({}),
    };
    let browser_secret_request =
        crate::transport::relay_peer::RelayPeerRequest::ResolveHomeCredentialSecret {
            context: context.clone(),
            credential_id: "gmail-password".to_string(),
            injection: crate::transport::relay_peer::RemoteCredentialSecretInjection::Browser {
                target_url: "https://accounts.google.com/signin".to_string(),
            },
        };
    let pty_secret_request =
        crate::transport::relay_peer::RelayPeerRequest::ResolveHomeCredentialSecret {
            context,
            credential_id: "ssh-password".to_string(),
            injection: crate::transport::relay_peer::RemoteCredentialSecretInjection::Pty,
        };
    let list_response =
        crate::transport::relay_peer::RelayPeerResponse::HomeCredentialToolHandled {
            result: crate::transport::runtime_tools::RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({
                    "credentials": [{
                        "id": "gmail-password",
                        "allowed_uses": ["browser"]
                    }]
                }),
            },
        };
    let secret_response =
        crate::transport::relay_peer::RelayPeerResponse::HomeCredentialSecretResolved {
            credential_id: "gmail-password".to_string(),
            secret_input: "redacted-by-test-fixture".to_string(),
        };
    let snapshot = serde_json::json!([
        list_request,
        browser_secret_request,
        pty_secret_request,
        list_response,
        secret_response
    ]);
    assert_eq!(
        snapshot.pointer("/0/kind"),
        Some(&serde_json::json!("invoke_home_credential_tool"))
    );
    assert_eq!(
        snapshot.pointer("/1/injection/kind"),
        Some(&serde_json::json!("browser"))
    );
    assert_eq!(
        snapshot.pointer("/2/injection/kind"),
        Some(&serde_json::json!("pty"))
    );
    assert_eq!(
        snapshot.pointer("/3/kind"),
        Some(&serde_json::json!("home_credential_tool_handled"))
    );
    assert_eq!(
        snapshot.pointer("/4/kind"),
        Some(&serde_json::json!("home_credential_secret_resolved"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("relay home credential shape should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "d249fdbf3b22f76b7606a17bd40c94031246932a9037ef0a7b7e4e0d11a31da7"
    );
}

#[test]
fn local_daemon_protocol_extension_install_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

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
fn local_daemon_protocol_provider_capability_import_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

    let request = LocalDaemonRequest::ImportProviderCapabilities(
        crate::local::ImportProviderCapabilitiesRequest {
            workspace_id: Some("/repo".to_string()),
            providers: vec!["codex".to_string(), "claude".to_string()],
            kind: Some("all".to_string()),
            name: Some("docs".to_string()),
            dry_run: true,
        },
    );
    let response = LocalDaemonResponse::ProviderCapabilitiesImported {
        report: crate::local::ProviderCapabilityImportReport {
            dry_run: true,
            providers: vec!["codex".to_string(), "claude".to_string()],
            summary: crate::local::ProviderCapabilityImportSummary {
                candidates: 2,
                imported: 0,
                updated: 0,
                already_installed: 1,
                deduped: 1,
                skipped: 0,
                errors: 0,
            },
            mcps: vec![crate::local::ProviderCapabilityImportEntry {
                kind: "mcp".to_string(),
                name: "docs".to_string(),
                provider: "claude".to_string(),
                source: "/repo/.mcp.json".to_string(),
                hash: Some("hash-1".to_string()),
                action: "would_import".to_string(),
                reason: "would import newest provider definition".to_string(),
                duplicates: vec![crate::local::ProviderCapabilityImportDuplicate {
                    provider: "codex".to_string(),
                    source: "/repo/.codex/config.toml".to_string(),
                    hash: Some("hash-0".to_string()),
                    reason: "different definition hash, older source".to_string(),
                }],
            }],
            skills: vec![crate::local::ProviderCapabilityImportEntry {
                kind: "skill".to_string(),
                name: "qa".to_string(),
                provider: "codex".to_string(),
                source: "/repo/.codex/skills/qa".to_string(),
                hash: Some("hash-2".to_string()),
                action: "already_installed".to_string(),
                reason: "matching skill package already installed in Arroba".to_string(),
                duplicates: Vec::new(),
            }],
        },
    };

    let snapshot = serde_json::json!([request, response]);
    assert_eq!(
        snapshot.pointer("/0/ImportProviderCapabilities/providers/1"),
        Some(&serde_json::json!("claude"))
    );
    assert_eq!(
        snapshot.pointer("/0/ImportProviderCapabilities/dry_run"),
        Some(&serde_json::json!(true))
    );
    assert_eq!(
        snapshot.pointer("/1/ProviderCapabilitiesImported/report/summary/deduped"),
        Some(&serde_json::json!(1))
    );
    assert_eq!(
        snapshot.pointer("/1/ProviderCapabilitiesImported/report/mcps/0/duplicates/0/provider"),
        Some(&serde_json::json!("codex"))
    );
    let serialized = serde_json::to_string(&snapshot)
        .expect("provider capability import snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "a215c9f199de50f93814a59df2fb0029b59dc3cd34359403584faf2b296555b0"
    );
}

#[test]
fn local_daemon_protocol_provider_run_usage_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

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
                    wait_for_all_inputs: None,
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

    let wait_for_all_inputs_request =
        serde_json::to_value(LocalDaemonRequest::SetWorkflowNodeWaitForAllInputs(
            super::SetWorkflowNodeWaitForAllInputsRequest {
                session_id: "session-1".to_string(),
                workflow_ref: "workflow-1".to_string(),
                node_id: "node-1".to_string(),
                wait_for_all_inputs: true,
                expected_workflow_revision: Some(7),
            },
        ))
        .expect("wait-for-all-inputs request should serialize");
    let wait_for_all_inputs_payload = wait_for_all_inputs_request
        .pointer("/SetWorkflowNodeWaitForAllInputs")
        .expect("wait-for-all-inputs payload should serialize");
    assert_eq!(
        wait_for_all_inputs_payload.pointer("/wait_for_all_inputs"),
        Some(&serde_json::json!(true))
    );
    let serialized = serde_json::to_string(wait_for_all_inputs_payload)
        .expect("wait-for-all-inputs request snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "f264411172709de929512f443cbafd5341808a4fc761b8541ae35557ab334ff7"
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

    let set_secret_request = serde_json::to_value(LocalDaemonRequest::SetCredentialSecret(
        crate::local::SetCredentialSecretRequest {
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            key: "gmail-password".to_string(),
            value: "secret-value".to_string(),
        },
    ))
    .expect("set credential secret request should serialize");
    let set_secret_payload = set_secret_request
        .pointer("/SetCredentialSecret")
        .expect("set credential secret payload should serialize");
    assert_eq!(
        set_secret_payload.pointer("/session_id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        set_secret_payload.pointer("/agent_id"),
        Some(&serde_json::json!("agent-1"))
    );
    let manage_vault_request = serde_json::to_value(LocalDaemonRequest::ManageCredentialVault(
        crate::local::ManageCredentialVaultRequest {
            session_id: "session-1".to_string(),
            agent_id: Some("agent-1".to_string()),
        },
    ))
    .expect("manage credential vault request should serialize");
    let manage_vault_payload = manage_vault_request
        .pointer("/ManageCredentialVault")
        .expect("manage credential vault payload should serialize");
    assert_eq!(
        manage_vault_payload.pointer("/session_id"),
        Some(&serde_json::json!("session-1"))
    );
    let serialized = serde_json::to_string(&serde_json::json!({
        "set_secret": set_secret_payload,
        "manage_vault": manage_vault_payload,
    }))
    .expect("credential vault request snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "8780d0be92d8154fd1ed4d5edc1831063bedf76e291e9f7519ae624198782ed6"
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

    let secret_interaction = crate::session::RuntimeInteraction::new(
        "credential-secret-1",
        "agent-1",
        crate::session::RuntimeInteractionKind::Choice,
        crate::session::RuntimeInteractionLevel::Critical,
        Some("Add secret".to_string()),
        "Enter a password to store in Arroba Vault.",
        vec![crate::session::RuntimeInteractionChoice::new(
            "cancel",
            "Cancel",
            "cancel",
            Some(crate::session::RuntimeInteractionChoiceStyle::Danger),
        )],
        Some(crate::session::RuntimeInteractionCustomChoice::secret(
            "secret",
            "Secret",
            Some("Password".to_string()),
            Some(8),
            Some(256),
        )),
        Some(300),
        Some("cancel".to_string()),
    );
    let mut secret_interaction_snapshot =
        serde_json::to_value(secret_interaction).expect("secret interaction should serialize");
    secret_interaction_snapshot["requested_at_ms"] = serde_json::json!(1234);
    assert_eq!(
        secret_interaction_snapshot.pointer("/custom_choice/input_kind"),
        Some(&serde_json::json!("secret"))
    );
    let serialized =
        serde_json::to_string(&secret_interaction_snapshot).expect("secret snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "0f5a200755e3df0e6b30984acdb4b176c0f52261ff2a6c6479ab6e3e97c152ff"
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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

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
        metaagent: false,
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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

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
        metaagent: false,
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
fn local_daemon_protocol_turn_undo_and_agent_fork_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

    let undo_request = LocalDaemonRequest::UndoTurn(crate::local::UndoTurnRequest {
        session_id: "session-1".to_string(),
        agent_ref: Some("worker".to_string()),
        turn_ref: Some("turn-1".to_string()),
    });
    let fork_request = LocalDaemonRequest::ForkAgent(crate::local::ForkAgentRequest {
        session_id: "session-1".to_string(),
        source_agent_ref: None,
        alias: Some("fork".to_string()),
    });
    let undo_response =
        LocalDaemonResponse::TurnUndone {
            result: crate::local::TurnUndoResult {
                session_id: "session-1".to_string(),
                agent_id: "agent-1".to_string(),
                turn_id: "turn-1".to_string(),
                prompt_id: "prompt-1".to_string(),
                provider_run_id: "provider-run-1".to_string(),
                reverted_paths: vec!["src/lib.rs".to_string()],
                path_results:
                    vec![crate::workspace_live_sync_journal::WorkspaceLiveSyncPathApplyResult {
                path: "src/lib.rs".to_string(),
                status: crate::workspace_live_sync_journal::WorkspaceLiveSyncApplyStatus::Applied,
                message: "restored".to_string(),
            }],
            },
        };
    let mut agent_value = serde_json::to_value(crate::agent::AgentInstance::new(
        "agent-2",
        "agent-ref-2",
        "session-1",
        Some("fork".to_string()),
        "codex",
        Some("gpt-5".to_string()),
        Some("medium".to_string()),
        Some("worktree-1".to_string()),
        crate::agent::GridPosition::new(0, 0, 1, 1),
    ))
    .expect("agent snapshot should encode");
    agent_value["created_at_ms"] = serde_json::json!(1_000);
    agent_value["last_activity_at_ms"] = serde_json::json!(1_000);
    let agent: crate::agent::AgentInstance =
        serde_json::from_value(agent_value).expect("agent snapshot should decode");
    let run_request = crate::provider::LaunchProviderRequest::new(
        "session-1",
        "codex",
        "codex",
        "default",
        "gpt-5",
    )
    .with_agent_id("agent-2")
    .with_variant(Some("medium".to_string()));
    let run = crate::provider::RuntimeProviderRun::new(
        "provider-run-2",
        &run_request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "codex".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    let mut run_value = serde_json::to_value(run).expect("provider run should encode");
    run_value["started_at_ms"] = serde_json::json!(1_000);
    run_value["last_activity_at_ms"] = serde_json::json!(1_000);
    let mut session_value = serde_json::to_value(crate::session::RuntimeSession::new(
        "session-1",
        None,
        "workspace-1",
        "worktree-1",
        "machine-1",
        "daemon-1",
    ))
    .expect("session snapshot should encode");
    session_value["created_at_ms"] = serde_json::json!(1_000);
    session_value["last_used_at_ms"] = serde_json::json!(1_000);
    let fork_response = LocalDaemonResponse::AgentForked {
        source_agent_id: "agent-1".to_string(),
        agent,
        provider_run: serde_json::from_value(run_value).expect("provider run should decode"),
        session: serde_json::from_value(session_value).expect("session snapshot should decode"),
    };

    let snapshot = serde_json::json!([undo_request, fork_request, undo_response, fork_response]);
    assert_eq!(
        snapshot.pointer("/0/UndoTurn/agent_ref"),
        Some(&serde_json::json!("worker"))
    );
    assert_eq!(snapshot.pointer("/1/ForkAgent/source_agent_ref"), None);
    assert_eq!(
        snapshot.pointer("/2/TurnUndone/result/path_results/0/status"),
        Some(&serde_json::json!("applied"))
    );
    assert_eq!(
        snapshot.pointer("/3/AgentForked/provider_run/agent_instance_id"),
        Some(&serde_json::json!("agent-2"))
    );
    let serialized = serde_json::to_string(&snapshot).expect("turn action snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "7987c2f41cee27e298723a4f77cb47f70a9dc7e311c5e660c0747f7369b7ca8b"
    );
}

#[test]
fn local_daemon_protocol_slice_targeted_create_session_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

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
        "d899a11d56955e8462792fe28e058cc2e4ad7b1588edb31a8ee8ab48e1d77ee0"
    );
}

#[test]
fn local_daemon_protocol_create_session_worktree_placement_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

    let request = LocalDaemonRequest::CreateSession(
        CreateSessionRequest::new("workspace-1", "worktree-1").with_worktree_placement(
            crate::agent::GitWorktreePlacement {
                target_directory: Some("../feature".to_string()),
                branch: Some("feature/session".to_string()),
                from_ref: Some("main".to_string()),
            },
        ),
    );
    let snapshot = serde_json::to_value(request).expect("request should serialize");
    assert_eq!(
        snapshot.pointer("/CreateSession/worktree_placement"),
        Some(&serde_json::json!({
            "target_directory": "../feature",
            "branch": "feature/session",
            "from_ref": "main",
        }))
    );
    let serialized = serde_json::to_string(&snapshot)
        .expect("worktree-placement create session snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "c8f50e8e2a7e469468ef4ee9a01b7093a4d85c3ce1a7401c17ae714aaf3b89a9"
    );
}

#[test]
fn local_daemon_protocol_kernel_targeted_create_session_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

    let request = LocalDaemonRequest::CreateSession(
        CreateSessionRequest::new("workspace-1", "worktree-1")
            .with_alias("remote-session")
            .with_kernel_ref("kernel-worker")
            .with_metaagent(true),
    );
    let snapshot = serde_json::to_value(request).expect("request should serialize");
    assert_eq!(
        snapshot.pointer("/CreateSession/kernel_ref"),
        Some(&serde_json::json!("kernel-worker"))
    );
    assert_eq!(snapshot.pointer("/CreateSession/machine_ref"), None);
    assert_eq!(
        snapshot.pointer("/CreateSession/metaagent"),
        Some(&serde_json::json!(true))
    );
    let serialized = serde_json::to_string(&snapshot)
        .expect("kernel-targeted create session snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "bf39557e7274d3c327ef768b93a1ee1e5d30a15a5ee7afda7bf6b458123a578d"
    );
}

#[test]
fn local_daemon_protocol_slice_record_relay_endpoint_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

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
            saved_state_ref: None,
            saved_state_status: None,
            saved_state_updated_at_ms: None,
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
fn local_daemon_protocol_slice_saved_state_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

    let create_request = LocalDaemonRequest::CreateSlice(crate::local::CreateSliceRequest {
        name: "linux-dev".to_string(),
        backend: crate::slice::SliceBackendKind::LocalDocker,
        os: "linux".to_string(),
        display_mode: crate::slice::SliceDisplayMode::Headed,
        workspace_id: Some("workspace-1".to_string()),
        worktree_id: Some("worktree-1".to_string()),
        workspace_mount: Some("/repo".to_string()),
        worker_kernel_ref: None,
        display_url: None,
        provider_auth: Vec::new(),
        from_saved_state: None,
        base: Some(crate::local::SliceCreateBase::Clean),
    });
    let save_request = LocalDaemonRequest::SaveSliceState(crate::local::SliceStateSaveRequest {
        slice_ref: "linux-dev".to_string(),
        mode: Some(crate::local::SliceStateSaveMode::RestartAgents),
        scope: Some(crate::local::SliceStateSaveScope::FutureSlices),
    });
    let status_request =
        LocalDaemonRequest::GetSliceStateStatus(crate::local::SliceStateStatusRequest {
            slice_ref: "linux-dev".to_string(),
        });
    let reset_request = LocalDaemonRequest::ResetSliceState(crate::local::SliceStateResetRequest {
        slice_ref: "linux-dev".to_string(),
    });
    let backup_request =
        LocalDaemonRequest::CreateSliceBackup(crate::local::CreateSliceBackupRequest {
            slice_ref: "linux-dev".to_string(),
            name: Some("gmail-ready".to_string()),
        });
    let saved_state = crate::slice::SliceSavedStateRecord {
        id: "linux-dev".to_string(),
        slice_name: "linux-dev".to_string(),
        source_slice_id: "slice-1".to_string(),
        backend: crate::slice::SliceBackendKind::LocalDocker,
        os: "linux".to_string(),
        image_ref: "arroba-slice-state:linux-dev".to_string(),
        home_archive_path: "/home/user/.arroba/slices/states/linux-dev/home.tar.zst".to_string(),
        manifest_path: "/home/user/.arroba/slices/states/linux-dev/manifest.json".to_string(),
        created_at_ms: 1000,
        updated_at_ms: 2000,
        size_bytes: Some(4096),
        last_operation: Some("state.save".to_string()),
        last_operation_status: Some(crate::slice::SliceOperationStatus::Completed),
        last_error: None,
    };
    let backup = crate::slice::SliceBackupRecord {
        id: "linux-dev-20260609-181500".to_string(),
        name: "gmail-ready".to_string(),
        source_slice_id: "slice-1".to_string(),
        source_state_id: "linux-dev".to_string(),
        image_ref: "arroba-slice-backup:linux-dev-20260609-181500".to_string(),
        home_archive_path:
            "/home/user/.arroba/slices/backups/linux-dev-20260609-181500/home.tar.zst".to_string(),
        manifest_path: "/home/user/.arroba/slices/backups/linux-dev-20260609-181500/manifest.json"
            .to_string(),
        created_at_ms: 3000,
        size_bytes: Some(4096),
    };
    let slice = crate::slice::SliceRecord {
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
        status: crate::slice::SliceStatus::Stopped,
        last_operation: Some("state.save".to_string()),
        last_operation_status: Some(crate::slice::SliceOperationStatus::Completed),
        last_error: None,
        last_operation_at_ms: Some(2000),
        workspace_id: Some("workspace-1".to_string()),
        worktree_id: Some("worktree-1".to_string()),
        workspace_mount: Some("/repo".to_string()),
        worker_kernel_ref: "slice:linux-dev".to_string(),
        worker_kernel_id: None,
        worker_machine_id: None,
        relay_endpoint: None,
        local_docker_ports: None,
        providers: Vec::new(),
        provider_auth: Vec::new(),
        saved_state_ref: Some("linux-dev".to_string()),
        saved_state_status: Some(crate::slice::SliceSavedStateStatus::Saved),
        saved_state_updated_at_ms: Some(2000),
        display_endpoint: None,
        created_at_ms: 1000,
        updated_at_ms: 2000,
    };
    let snapshot = serde_json::json!([
        create_request,
        save_request,
        status_request,
        reset_request,
        backup_request,
        LocalDaemonResponse::SliceStateSaved {
            slice: slice.clone(),
            state: saved_state.clone(),
        },
        LocalDaemonResponse::SliceStateStatus {
            slice: slice.clone(),
            state: Some(saved_state.clone()),
        },
        LocalDaemonResponse::SliceStateReset {
            slice: slice.clone(),
            removed_state: Some(saved_state),
        },
        LocalDaemonResponse::SliceBackupCreated {
            slice,
            backup,
            instructions: "swap backup directory with active state directory".to_string(),
        },
    ]);
    assert_eq!(
        snapshot.pointer("/0/CreateSlice/base"),
        Some(&serde_json::json!("clean"))
    );
    assert_eq!(
        snapshot.pointer("/1/SaveSliceState/slice_ref"),
        Some(&serde_json::json!("linux-dev"))
    );
    assert_eq!(
        snapshot.pointer("/1/SaveSliceState/mode"),
        Some(&serde_json::json!("restart_agents"))
    );
    assert_eq!(
        snapshot.pointer("/1/SaveSliceState/scope"),
        Some(&serde_json::json!("future_slices"))
    );
    assert_eq!(
        snapshot.pointer("/5/SliceStateSaved/state/image_ref"),
        Some(&serde_json::json!("arroba-slice-state:linux-dev"))
    );
    assert_eq!(
        snapshot.pointer("/6/SliceStateStatus/slice/saved_state_status"),
        Some(&serde_json::json!("saved"))
    );
    assert_eq!(
        snapshot.pointer("/8/SliceBackupCreated/backup/source_state_id"),
        Some(&serde_json::json!("linux-dev"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("slice saved state snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "cf405b93fcf238ea0bc96fdf347fc4193db5b50f766dbfbe349c5a6d50b72768"
    );
}

#[test]
fn local_daemon_protocol_slice_auth_alias_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

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
            saved_state_ref: None,
            saved_state_status: None,
            saved_state_updated_at_ms: None,
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
            saved_state_ref: None,
            saved_state_status: None,
            saved_state_updated_at_ms: None,
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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

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
            saved_state_ref: None,
            saved_state_status: None,
            saved_state_updated_at_ms: None,
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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

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
            saved_state_ref: None,
            saved_state_status: None,
            saved_state_updated_at_ms: None,
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
fn local_daemon_protocol_slice_audit_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

    let request = LocalDaemonRequest::ListSliceAudit(crate::local::ListSliceAuditRequest {
        slice_ref: "linux-dev".to_string(),
        limit: Some(25),
    });
    let response = LocalDaemonResponse::SliceAuditListed {
        events: vec![crate::durable_state::DurableStateEvent {
            sequence: 7,
            event_id: "state_evt_1".to_string(),
            kind: "slice.audit".to_string(),
            subject_id: Some("slice-1".to_string()),
            timestamp_ms: 42,
            payload: serde_json::json!({
                "slice_id": "slice-1",
                "slice_name": "linux-dev",
                "action": "start",
                "outcome": "completed",
                "provider": null,
                "status": "running",
                "display_mode": "headless",
                "worktree_id": "/repo",
                "agent_ids": [],
                "worker_kernel_id": "worker-1"
            }),
        }],
    };
    let snapshot = serde_json::json!([request, response]);
    assert_eq!(
        snapshot.pointer("/0/ListSliceAudit/slice_ref"),
        Some(&serde_json::json!("linux-dev"))
    );
    assert_eq!(
        snapshot.pointer("/0/ListSliceAudit/limit"),
        Some(&serde_json::json!(25))
    );
    assert_eq!(
        snapshot.pointer("/1/SliceAuditListed/events/0/kind"),
        Some(&serde_json::json!("slice.audit"))
    );
    assert_eq!(
        snapshot.pointer("/1/SliceAuditListed/events/0/payload/action"),
        Some(&serde_json::json!("start"))
    );
    let serialized = serde_json::to_string(&snapshot).expect("slice audit snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "b5e42a56330b7236af80d55b41820bea7a6acb850e07607078041fe1915b3eb0"
    );
}

#[test]
fn local_daemon_protocol_semantic_recall_search_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

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

#[test]
fn local_daemon_protocol_metaagent_event_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

    let search =
        LocalDaemonRequest::SearchMetaagentCommands(crate::local::SearchMetaagentCommandsRequest {
            session_id: "session-1".to_string(),
            metaagent_id: "meta-1".to_string(),
            query: Some("agent".to_string()),
            tag: Some("agent".to_string()),
            scope: Some("session".to_string()),
            mutates: Some(true),
            policy: Some("allow".to_string()),
            limit: Some(5),
        });
    let turn_overview = LocalDaemonRequest::GetMetaagentTurnOverview(
        crate::local::GetMetaagentTurnOverviewRequest {
            session_id: "session-1".to_string(),
            metaagent_id: "meta-1".to_string(),
            agent_ref: Some("worker".to_string()),
            turn_ref: Some("turn-1".to_string()),
            turns_back: Some(1),
            limit: Some(50),
        },
    );
    let turn_blob =
        LocalDaemonRequest::GetMetaagentTurnBlob(crate::local::GetMetaagentTurnBlobRequest {
            session_id: "session-1".to_string(),
            metaagent_id: "meta-1".to_string(),
            blob_id: "blob-1".to_string(),
        });
    let list = LocalDaemonRequest::ListMetaagentEvents(crate::local::ListMetaagentEventsRequest {
        session_id: "session-1".to_string(),
        metaagent_id: "meta-1".to_string(),
        limit: Some(20),
        status: Some("unread".to_string()),
        kind: Some("agent.turn.completed".to_string()),
    });
    let read = LocalDaemonRequest::ReadMetaagentEvent(crate::local::ReadMetaagentEventRequest {
        session_id: "session-1".to_string(),
        metaagent_id: "meta-1".to_string(),
        event_id: "event-1".to_string(),
    });
    let ack = LocalDaemonRequest::AckMetaagentEvents(crate::local::AckMetaagentEventsRequest {
        session_id: "session-1".to_string(),
        metaagent_id: "meta-1".to_string(),
        event_id: Some("event-1".to_string()),
        event_ids: Some(vec!["event-2".to_string()]),
        up_to_sequence: Some(7),
    });
    let update_task =
        LocalDaemonRequest::UpdateMetaagentTask(crate::local::UpdateMetaagentTaskRequest {
            session_id: "session-1".to_string(),
            metaagent_id: "meta-1".to_string(),
            task_markdown: Some("# Task".to_string()),
            plan_markdown: Some("1. Delegate.".to_string()),
        });
    let pause_task =
        LocalDaemonRequest::PauseMetaagentTask(crate::local::PauseMetaagentTaskRequest {
            session_id: "session-1".to_string(),
            metaagent_id: "meta-1".to_string(),
        });
    let resume_task =
        LocalDaemonRequest::ResumeMetaagentTask(crate::local::ResumeMetaagentTaskRequest {
            session_id: "session-1".to_string(),
            metaagent_id: "meta-1".to_string(),
        });
    let abort_task =
        LocalDaemonRequest::AbortMetaagentTask(crate::local::AbortMetaagentTaskRequest {
            session_id: "session-1".to_string(),
            metaagent_id: "meta-1".to_string(),
            reason: Some("user aborted".to_string()),
        });
    let event = serde_json::json!({
        "event_id": "event-1",
        "metaagent_id": "meta-1",
        "kind": "agent.turn.completed",
        "sequence": 7,
    });
    let listed = LocalDaemonResponse::MetaagentEventsListed {
        events: vec![event.clone()],
    };
    let searched = LocalDaemonResponse::MetaagentCommandsSearched {
        commands: vec![serde_json::json!({
            "name": "agent list",
            "usage": "agent list",
            "scope": "session",
            "policy": "allow",
        })],
    };
    let overview_response = LocalDaemonResponse::MetaagentTurnOverview {
        overview: serde_json::json!({
            "agent": { "id": "agent-1", "alias": "worker" },
            "turns": [{ "turn_id": "turn-1", "items": [] }],
        }),
    };
    let blob_response = LocalDaemonResponse::MetaagentTurnBlob {
        blob: serde_json::json!({
            "agent": { "id": "agent-1", "alias": "worker" },
            "blob_id": "blob-1",
            "entries": [],
        }),
    };
    let read_response = LocalDaemonResponse::MetaagentEventRead {
        event: event.clone(),
    };
    let acked = LocalDaemonResponse::MetaagentEventsAcked { acked: vec![event] };
    let mut task_session = crate::session::RuntimeSession::new(
        "session-1",
        None,
        "/repo",
        "/repo",
        "machine-1",
        "daemon-1",
    );
    task_session.update_metaagent_task_markdown("meta-1", "# Task");
    task_session.update_metaagent_plan_markdown("meta-1", "1. Delegate.");
    let task = task_session.metaagent_task("meta-1").cloned();
    let task_response = LocalDaemonResponse::MetaagentTaskUpdated {
        session: task_session,
        task,
    };
    let mut snapshot = serde_json::json!([
        search,
        turn_overview,
        turn_blob,
        list,
        read,
        ack,
        update_task,
        pause_task,
        resume_task,
        abort_task,
        listed,
        searched,
        overview_response,
        blob_response,
        read_response,
        acked,
        task_response
    ]);
    *snapshot
        .pointer_mut("/16/MetaagentTaskUpdated/session/created_at_ms")
        .expect("session created_at_ms should encode") = serde_json::json!(42);
    *snapshot
        .pointer_mut("/16/MetaagentTaskUpdated/session/last_used_at_ms")
        .expect("session last_used_at_ms should encode") = serde_json::json!(42);
    *snapshot
        .pointer_mut("/16/MetaagentTaskUpdated/session/metaagent_tasks/0/task_id")
        .expect("session task_id should encode") = serde_json::json!("metaagent-task-1");
    *snapshot
        .pointer_mut("/16/MetaagentTaskUpdated/session/metaagent_tasks/0/created_at_ms")
        .expect("session task created_at_ms should encode") = serde_json::json!(42);
    *snapshot
        .pointer_mut("/16/MetaagentTaskUpdated/session/metaagent_tasks/0/updated_at_ms")
        .expect("session task updated_at_ms should encode") = serde_json::json!(42);
    *snapshot
        .pointer_mut("/16/MetaagentTaskUpdated/task/task_id")
        .expect("response task_id should encode") = serde_json::json!("metaagent-task-1");
    *snapshot
        .pointer_mut("/16/MetaagentTaskUpdated/task/created_at_ms")
        .expect("response task created_at_ms should encode") = serde_json::json!(42);
    *snapshot
        .pointer_mut("/16/MetaagentTaskUpdated/task/updated_at_ms")
        .expect("response task updated_at_ms should encode") = serde_json::json!(42);

    assert_eq!(
        snapshot.pointer("/0/SearchMetaagentCommands/query"),
        Some(&serde_json::json!("agent"))
    );
    assert_eq!(
        snapshot.pointer("/1/GetMetaagentTurnOverview/agent_ref"),
        Some(&serde_json::json!("worker"))
    );
    assert_eq!(
        snapshot.pointer("/2/GetMetaagentTurnBlob/blob_id"),
        Some(&serde_json::json!("blob-1"))
    );
    assert_eq!(
        snapshot.pointer("/3/ListMetaagentEvents/metaagent_id"),
        Some(&serde_json::json!("meta-1"))
    );
    assert_eq!(
        snapshot.pointer("/4/ReadMetaagentEvent/event_id"),
        Some(&serde_json::json!("event-1"))
    );
    assert_eq!(
        snapshot.pointer("/5/AckMetaagentEvents/up_to_sequence"),
        Some(&serde_json::json!(7))
    );
    assert_eq!(
        snapshot.pointer("/6/UpdateMetaagentTask/task_markdown"),
        Some(&serde_json::json!("# Task"))
    );
    assert_eq!(
        snapshot.pointer("/7/PauseMetaagentTask/metaagent_id"),
        Some(&serde_json::json!("meta-1"))
    );
    assert_eq!(
        snapshot.pointer("/8/ResumeMetaagentTask/session_id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        snapshot.pointer("/9/AbortMetaagentTask/reason"),
        Some(&serde_json::json!("user aborted"))
    );
    assert_eq!(
        snapshot.pointer("/10/MetaagentEventsListed/events/0/kind"),
        Some(&serde_json::json!("agent.turn.completed"))
    );
    assert_eq!(
        snapshot.pointer("/11/MetaagentCommandsSearched/commands/0/name"),
        Some(&serde_json::json!("agent list"))
    );
    assert_eq!(
        snapshot.pointer("/12/MetaagentTurnOverview/overview/turns/0/turn_id"),
        Some(&serde_json::json!("turn-1"))
    );
    assert_eq!(
        snapshot.pointer("/13/MetaagentTurnBlob/blob/blob_id"),
        Some(&serde_json::json!("blob-1"))
    );
    assert_eq!(
        snapshot.pointer("/14/MetaagentEventRead/event/event_id"),
        Some(&serde_json::json!("event-1"))
    );
    assert_eq!(
        snapshot.pointer("/15/MetaagentEventsAcked/acked/0/sequence"),
        Some(&serde_json::json!(7))
    );
    assert_eq!(
        snapshot.pointer("/16/MetaagentTaskUpdated/task/plan_markdown"),
        Some(&serde_json::json!("1. Delegate."))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("metaagent event protocol shape should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "a6b9a8d6a7e59cee6c50fb17423cac10c41c62914026d84d31bbf69daafc304c"
    );
}

#[test]
fn local_daemon_protocol_remote_inventory_provider_accounts_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 168);

    let account = RelayProviderAccountSummary {
        provider: "codex".to_string(),
        state: "configured".to_string(),
        auth_type: Some("chatgpt".to_string()),
        account_id: Some("acct-remote-1".to_string()),
        email: None,
        organization_id: None,
        organization_name: None,
        subscription_type: None,
        alias: Some("remote-codex".to_string()),
    };
    let machine = crate::local::RemoteMachineRecord {
        machine_id: "machine-1".to_string(),
        machine_alias: Some("workstation".to_string()),
        registry_alias: Some("builder".to_string()),
        display_name: "builder".to_string(),
        trust_status: RemoteMachineTrustStatus::Approved,
        online: true,
        pending: false,
        kernel_count: 1,
        available_providers: vec!["codex".to_string()],
        provider_accounts: vec![account.clone()],
    };
    let kernel = RelayKernelPresence {
        kernel_id: "kernel-1".to_string(),
        machine_id: "machine-1".to_string(),
        machine_alias: Some("workstation".to_string()),
        relay_alias: Some("builder".to_string()),
        kernel_alias: Some("default".to_string()),
        available_providers: vec!["codex".to_string()],
        provider_accounts: vec![account],
        capabilities: vec!["kernel_ws".to_string()],
        accepting_remote_leases: true,
        leased_agent_count: 1,
        local_session_count: 2,
        public_key: "public-key".to_string(),
    };
    let snapshot = serde_json::json!([
        LocalDaemonResponse::RemoteMachinesListed {
            machines: vec![machine],
        },
        LocalDaemonResponse::RemoteMachineKernelsListed {
            machine_ref: "builder".to_string(),
            kernels: vec![kernel],
        },
    ]);
    assert_eq!(
        snapshot.pointer("/0/RemoteMachinesListed/machines/0/provider_accounts/0/alias"),
        Some(&serde_json::json!("remote-codex"))
    );
    assert_eq!(
        snapshot.pointer("/1/RemoteMachineKernelsListed/kernels/0/provider_accounts/0/account_id"),
        Some(&serde_json::json!("acct-remote-1"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("remote inventory account snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "aab66ad2af2311174b8fa36f3143c7760b078509028d29b17f6a89adf3d0c03d"
    );
}

fn history_page_entry(
    sequence: usize,
    kind: crate::history::SessionHistoryEntryKind,
    agent_id: &str,
    text: &str,
) -> crate::session_history_page::SessionHistoryPageEntry {
    crate::session_history_page::SessionHistoryPageEntry {
        entry_index: sequence,
        fragment_start: 0,
        fragment_end: text.chars().count(),
        total_chars: text.chars().count(),
        entry: crate::history::SessionHistoryEntry {
            session_id: "session-1".to_string(),
            provider_run_id: Some("provider-run-1".to_string()),
            agent_id: Some(agent_id.to_string()),
            source_attachment_id: Some("attachment-1".to_string()),
            kind,
            merge_key: None,
            source: None,
            external_provider: None,
            external_provider_session_id: None,
            external_provider_turn_id: None,
            observed_at_ms: None,
            attachments: Vec::new(),
            text: text.to_string(),
            timestamp_ms: 42 + sequence as u64,
        },
    }
}
