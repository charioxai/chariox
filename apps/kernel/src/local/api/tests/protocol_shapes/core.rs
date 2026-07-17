use super::*;

#[test]
fn local_daemon_protocol_provider_targeted_terminal_resize_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 240);

    let request = LocalDaemonRequest::ResizeTerminal(crate::local::ResizeTerminalRequest {
        session_id: "session-1".to_string(),
        provider_run_id: Some("provider-run-2".to_string()),
        cols: 80,
        rows: 24,
    });
    assert_eq!(
        serde_json::to_value(request).expect("provider-targeted terminal resize should encode"),
        serde_json::json!({
            "ResizeTerminal": {
                "session_id": "session-1",
                "provider_run_id": "provider-run-2",
                "cols": 80,
                "rows": 24
            }
        })
    );

    let legacy: LocalDaemonRequest = serde_json::from_value(serde_json::json!({
        "ResizeTerminal": {
            "session_id": "session-1",
            "cols": 120,
            "rows": 40
        }
    }))
    .expect("legacy active-provider resize should still decode");
    assert!(matches!(
        legacy,
        LocalDaemonRequest::ResizeTerminal(crate::local::ResizeTerminalRequest {
            provider_run_id: None,
            ..
        })
    ));
}

#[test]
fn local_daemon_protocol_terminal_command_catalog_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 240);

    let request = LocalDaemonRequest::GetTerminalCommandCatalog(GetTerminalCommandCatalogRequest);
    assert_eq!(
        serde_json::to_value(request).expect("terminal command catalog request should encode"),
        serde_json::json!({ "GetTerminalCommandCatalog": null })
    );

    let response = LocalDaemonResponse::TerminalCommandCatalog {
        catalog: TerminalCommandCatalog {
            revision: "sha256:catalog".to_string(),
            nodes: vec![TerminalCommandCatalogNode {
                id: "meta".to_string(),
                label: "/meta".to_string(),
                description: "Start a temporary Meta mode task".to_string(),
                value: "/meta ".to_string(),
                kind: TerminalCommandCatalogNodeKind::PromptPrefix,
                execution_target: TerminalCommandCatalogExecutionTarget::PromptPrefix,
                surfaces: vec![TerminalCommandCatalogSurface::Session],
                search_aliases: vec!["delegate".to_string()],
                intents: vec!["coordinate workers".to_string()],
                examples: vec!["/meta Build this through workers".to_string()],
                dynamic_source: None,
                children: vec![TerminalCommandCatalogNode {
                    id: "meta-child".to_string(),
                    label: "child".to_string(),
                    description: "Child command".to_string(),
                    value: "/meta child".to_string(),
                    kind: TerminalCommandCatalogNodeKind::Command,
                    execution_target: TerminalCommandCatalogExecutionTarget::Kernel,
                    surfaces: vec![TerminalCommandCatalogSurface::Session],
                    search_aliases: Vec::new(),
                    intents: Vec::new(),
                    examples: Vec::new(),
                    dynamic_source: Some("test.dynamic".to_string()),
                    children: Vec::new(),
                }],
            }],
        },
    };

    assert_eq!(
        serde_json::to_value(response).expect("terminal command catalog response should encode"),
        serde_json::json!({
            "TerminalCommandCatalog": {
                "catalog": {
                    "revision": "sha256:catalog",
                    "nodes": [{
                        "id": "meta",
                        "label": "/meta",
                        "description": "Start a temporary Meta mode task",
                        "value": "/meta ",
                        "kind": "prompt_prefix",
                        "execution_target": "prompt_prefix",
                        "surfaces": ["session"],
                        "search_aliases": ["delegate"],
                        "intents": ["coordinate workers"],
                        "examples": ["/meta Build this through workers"],
                        "children": [{
                            "id": "meta-child",
                            "label": "child",
                            "description": "Child command",
                            "value": "/meta child",
                            "kind": "command",
                            "execution_target": "kernel",
                            "surfaces": ["session"],
                            "dynamic_source": "test.dynamic"
                        }]
                    }]
                }
            }
        })
    );
}

#[test]
fn local_daemon_protocol_waiting_room_activity_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 240);

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
        worker_extension_agent_count: 2,
        worker_extension_sync_issue_count: 1,
        worker_extension_pending_revoke_count: 1,
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
            "worker_extension_agent_count": 2,
            "worker_extension_sync_issue_count": 1,
            "worker_extension_pending_revoke_count": 1,
            "unread_idle_agent_count": 1
        })
    );
}

#[test]
fn local_daemon_protocol_transport_health_relay_reconnect_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 240);

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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 240);

    let active_cancel_request =
        LocalDaemonRequest::CancelActivePrompt(crate::local::CancelActivePromptRequest {
            session_id: "session-1".to_string(),
            attachment_id: "attachment-1".to_string(),
            target_agent_id: Some("agent-1".to_string()),
        });
    let active_cancel_snapshot =
        serde_json::to_value(active_cancel_request).expect("active cancel request should encode");
    assert_eq!(
        active_cancel_snapshot.pointer("/CancelActivePrompt/target_agent_id"),
        Some(&serde_json::json!("agent-1"))
    );

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

    let update_request =
        LocalDaemonRequest::UpdateQueuedPrompt(crate::local::UpdateQueuedPromptRequest {
            session_id: "session-1".to_string(),
            attachment_id: "attachment-1".to_string(),
            target_agent_id: "agent-1".to_string(),
            prompt_id: "prompt-queued".to_string(),
            prompt: "updated queued text".to_string(),
        });
    let update_snapshot =
        serde_json::to_value(update_request).expect("update request should encode");
    assert_eq!(
        update_snapshot.pointer("/UpdateQueuedPrompt/prompt"),
        Some(&serde_json::json!("updated queued text"))
    );

    let external_prompt = crate::session::PromptQueueItem::external_observed_running(
        "codex",
        "thread-1",
        "user-1",
        "agent-1",
        "external text",
    );
    let external_prompt_snapshot =
        serde_json::to_value(external_prompt).expect("external prompt should encode");
    assert_eq!(
        external_prompt_snapshot.pointer("/prompt_origin"),
        Some(&serde_json::json!("external"))
    );
    assert_eq!(
        external_prompt_snapshot.pointer("/external_provider"),
        Some(&serde_json::json!("codex"))
    );
    assert_eq!(
        external_prompt_snapshot.pointer("/external_provider_session_id"),
        Some(&serde_json::json!("thread-1"))
    );
    assert_eq!(
        external_prompt_snapshot.pointer("/external_provider_turn_id"),
        Some(&serde_json::json!("user-1"))
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

    let cancel_response = LocalDaemonResponse::QueuedPromptCancelled {
        prompt: prompt.clone(),
        session: session.clone(),
        agent_activity: std::collections::BTreeMap::new(),
        agent_activity_revision: 8,
    };
    let cancel_response_snapshot =
        serde_json::to_value(cancel_response).expect("cancel response should encode");
    assert_eq!(
        cancel_response_snapshot.pointer("/QueuedPromptCancelled/session/id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        cancel_response_snapshot.pointer("/QueuedPromptCancelled/agent_activity_revision"),
        Some(&serde_json::json!(8))
    );

    let update_response = LocalDaemonResponse::QueuedPromptUpdated {
        prompt,
        session,
        agent_activity: std::collections::BTreeMap::new(),
        agent_activity_revision: 9,
    };
    let update_response_snapshot =
        serde_json::to_value(update_response).expect("update response should encode");
    assert_eq!(
        update_response_snapshot.pointer("/QueuedPromptUpdated/prompt/prompt"),
        Some(&serde_json::json!("queued text"))
    );
    assert_eq!(
        update_response_snapshot.pointer("/QueuedPromptUpdated/agent_activity_revision"),
        Some(&serde_json::json!(9))
    );
}

#[test]
fn local_daemon_protocol_batch_launch_and_prompt_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 240);

    let launch_request = LocalDaemonRequest::LaunchProviderRuns(LaunchProviderRunsRequest {
        max_concurrency: Some(8),
        launches: vec![
            LaunchProviderRunRequest {
                session_id: "session-1".to_string(),
                agent_id: Some("agent-1".to_string()),
                adapter_key: "codex".to_string(),
                provider: "codex".to_string(),
                account_profile: "default".to_string(),
                model: "gpt-5".to_string(),
                variant: Some("medium".to_string()),
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: false,
            },
            LaunchProviderRunRequest {
                session_id: "session-1".to_string(),
                agent_id: Some("agent-2".to_string()),
                adapter_key: "opencode".to_string(),
                provider: "opencode".to_string(),
                account_profile: "work".to_string(),
                model: "gpt-5.1".to_string(),
                variant: None,
                structured_endpoint: Some("http://127.0.0.1:4567".to_string()),
                provider_session_id: None,
                native_tui: false,
            },
        ],
    });
    let prompt_request = LocalDaemonRequest::SubmitPrompts(SubmitPromptsRequest {
        session_id: "session-1".to_string(),
        attachment_id: "attachment-1".to_string(),
        max_concurrency: Some(4),
        prompts: vec![
            SubmitPromptsRequestItem {
                session_id: None,
                attachment_id: None,
                target_agent_id: "agent-1".to_string(),
                prompt: "review shard 1".to_string(),
                attachments: Vec::new(),
            },
            SubmitPromptsRequestItem {
                session_id: Some("session-2".to_string()),
                attachment_id: Some("attachment-2".to_string()),
                target_agent_id: "agent-2".to_string(),
                prompt: "review shard 2".to_string(),
                attachments: Vec::new(),
            },
        ],
    });
    let snapshot = serde_json::json!([launch_request, prompt_request]);
    assert_eq!(
        snapshot.pointer("/0/LaunchProviderRuns/max_concurrency"),
        Some(&serde_json::json!(8))
    );
    assert_eq!(
        snapshot.pointer("/0/LaunchProviderRuns/launches/1/structured_endpoint"),
        Some(&serde_json::json!("http://127.0.0.1:4567"))
    );
    assert_eq!(
        snapshot.pointer("/1/SubmitPrompts/prompts/0/target_agent_id"),
        Some(&serde_json::json!("agent-1"))
    );
    assert_eq!(
        snapshot.pointer("/1/SubmitPrompts/prompts/1/session_id"),
        Some(&serde_json::json!("session-2"))
    );
    assert_eq!(
        snapshot.pointer("/1/SubmitPrompts/prompts/1/attachment_id"),
        Some(&serde_json::json!("attachment-2"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("batch launch/prompt snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "baedf3e025266833349aeecb0c03c1e2daf959b6c42adc75dfa5baea591b2fe8"
    );
}

#[test]
fn local_daemon_protocol_move_agent_to_local_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 240);

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
