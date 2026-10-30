use super::*;
use crate::local::{
    ForgetGatewayKernelRequest, GatewayAttachmentRef, GatewayKernel, GatewayKernelStatus,
    GatewayKernelVia, HeartbeatAttachmentsRequest, JoinKernelLinkRequest, KernelDirectoryEntry,
    KernelFileKind, KernelFileStat, ListGatewayKernelsRequest, ListKernelDirectoryRequest,
    OnBehalfOf, ReadKernelFileRequest, StatKernelFileRequest, WriteKernelFileRequest,
    KERNEL_FILE_MAX_BYTES, KERNEL_FILE_MAX_CHUNK_BYTES,
};
use crate::transport::kernel_protocol::{KernelEvent, KernelIncomingFrame, KernelOutgoingFrame};

// EA2-04/05: the shapes an external agent and a forwarding kernel depend on.
#[test]
fn kernel_gateway_protocol_568_shapes_and_hash() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let list = LocalDaemonRequest::ListGatewayKernels(ListGatewayKernelsRequest {});
    let listed = LocalDaemonResponse::GatewayKernelsListed {
        kernels: vec![GatewayKernel {
            name: "b3".into(),
            kernel_id: "kernel-b3".into(),
            label: "kernel-b3".into(),
            machine_id: "machine-b3".into(),
            machine_alias: Some("ovh-b3".into()),
            status: GatewayKernelStatus::Online,
            last_seen_ms: 1_000,
            gateway_ready: true,
            via: GatewayKernelVia::Hosted,
        }],
    };
    let frame: KernelIncomingFrame = serde_json::from_value(serde_json::json!({
        "type": "request", "request_id": "r1", "command_id": "c1", "kernel": "b3",
        "request": {"ListGatewayKernels": {}},
    }))
    .unwrap();
    let KernelIncomingFrame::Request { kernel, .. } = &frame else {
        panic!("a request frame")
    };
    assert_eq!(kernel.as_deref(), Some("b3"));
    let plain: KernelIncomingFrame = serde_json::from_value(serde_json::json!({
        "type": "request", "request_id": "r2", "request": {"ListGatewayKernels": {}},
    }))
    .unwrap();
    assert!(matches!(
        plain,
        KernelIncomingFrame::Request { kernel: None, .. }
    ));
    let on_behalf_of = OnBehalfOf {
        origin_kernel: "kernel-mac".into(),
        origin_name: Some("mac/main".into()),
        grant_id: "grant-1".into(),
        harness: "claude".into(),
    };
    let heartbeat = LocalDaemonRequest::HeartbeatAttachments(HeartbeatAttachmentsRequest {
        attachments: vec![GatewayAttachmentRef {
            session_id: "s1".into(),
            attachment_id: "a1".into(),
        }],
    });
    let heartbeated = LocalDaemonResponse::AttachmentsHeartbeated {
        lapsed: vec!["a2".into()],
    };
    let files = serde_json::json!({
        "stat": LocalDaemonRequest::StatKernelFile(StatKernelFileRequest { path: "/srv/b.bundle".into(), with_sha256: true }),
        "list": LocalDaemonRequest::ListKernelDirectory(ListKernelDirectoryRequest { path: "~/".into() }),
        "read": LocalDaemonRequest::ReadKernelFile(ReadKernelFileRequest { path: "/srv/b.bundle".into(), offset: 0, length: 4 }),
        "write": LocalDaemonRequest::WriteKernelFile(WriteKernelFileRequest {
            path: "/srv/b.bundle".into(), offset: 0, data_base64: "AAEC".into(), total_size: 3,
            sha256: Some("ab".repeat(32)), overwrite: false,
        }),
        "stat_answer": LocalDaemonResponse::KernelFileStat { stat: KernelFileStat {
            path: "/srv/b.bundle".into(), exists: true, kind: Some(KernelFileKind::File), size: Some(3),
            modified_ms: Some(5), sha256: None, partial_size: None,
        } },
        "listed": LocalDaemonResponse::KernelDirectoryListed { path: "/".into(), entries: vec![KernelDirectoryEntry {
            name: "a".into(), kind: KernelFileKind::Directory, size: 0, modified_ms: None,
        }], truncated: false },
        "chunk": LocalDaemonResponse::KernelFileChunk { path: "/srv/b.bundle".into(), offset: 0, data_base64: "AAEC".into(), size: 3, eof: true },
        "written": LocalDaemonResponse::KernelFileWritten { path: "/srv/b.bundle".into(), received: 3, complete: true, sha256: None },
        "caps": [KERNEL_FILE_MAX_BYTES, KERNEL_FILE_MAX_CHUNK_BYTES],
    });
    let pairing = serde_json::json!({
        "join": LocalDaemonRequest::JoinKernelLink(JoinKernelLinkRequest {
            link: "chariox-terminal-pair-v1.x".into(), relay_token: Some("t".into()),
        }),
        "forget": LocalDaemonRequest::ForgetGatewayKernel(ForgetGatewayKernelRequest { name: "b3".into() }),
        "forgotten": LocalDaemonResponse::GatewayKernelForgotten { kernel_id: "kernel-b3".into() },
    });
    let subscribe: KernelIncomingFrame = serde_json::from_value(serde_json::json!({
        "type": "subscribe", "request_id": "r3", "session_id": "s1", "attachment_id": "a1",
        "resume_from_event_id": 7, "kernel": "b3",
    }))
    .unwrap();
    let KernelIncomingFrame::Subscribe { kernel, .. } = &subscribe else {
        panic!("a subscribe frame")
    };
    assert_eq!(kernel.as_deref(), Some("b3"));
    let event = serde_json::to_value(KernelOutgoingFrame::Event {
        event_id: 8,
        kernel: Some("b3".into()),
        event: Box::new(KernelEvent::Heartbeat {
            session_id: "s1".into(),
        }),
    })
    .unwrap();
    assert_eq!(event["kernel"], "b3");
    let snapshot = serde_json::json!({
        "list": list, "listed": listed, "frame": frame, "on_behalf_of": on_behalf_of,
        "heartbeat": heartbeat, "heartbeated": heartbeated, "files": files, "pairing": pairing, "subscribe": subscribe, "event": event,
        "capability": crate::local::RUNTIME_CONTROL_CAPABILITIES.contains(&"kernel_gateway_v1"),
    });
    assert_eq!(snapshot["capability"], true);
    assert_eq!(snapshot["frame"]["kernel"], "b3");
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "102043fb7f6e4b46f4c9371d861b7ef60a004f86a4ba16d51210f34b9d90bb9e"
    );
}

// MP-08 / MP-09 / MP-10 / MP-11: outside events use short grant-bound requests.
#[test]
fn outside_event_requests_are_typed() {
    for value in [
        serde_json::json!({"SubscribeEvents":{"targets":[{"kernel":"local","session_id":"room","agent_id":"agent"}]}}),
        serde_json::json!({"UnsubscribeEvents":{"all":true}}),
        serde_json::json!({"GetEventSubscription":{}}),
        serde_json::json!({"ListEvents":{"since":4,"limit":100}}),
        serde_json::json!({"WaitEvents":{"since":4,"timeout_ms":540000,"limit":100}}),
        serde_json::json!({"AckEvents":{"cursor":4}}),
    ] {
        let request: super::super::super::LocalDaemonRequest =
            serde_json::from_value(value.clone())
                .unwrap_or_else(|error| panic!("outside event request {value}: {error}"));
        assert_eq!(serde_json::to_value(request).unwrap(), value);
    }
}

// MP-08 / MP-09 / MP-10 / MP-11: every outside event variant is a stable wire shape.
#[test]
fn outside_events_snapshot_and_hash() {
    let snapshot: serde_json::Value =
        serde_json::from_str(include_str!("outside-events.json")).unwrap();
    assert_eq!(snapshot["local_protocol"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(
        snapshot["peer_protocol"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    for value in snapshot["requests"].as_array().unwrap() {
        let request: LocalDaemonRequest = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), *value);
    }
    for value in snapshot["responses"].as_array().unwrap() {
        let response: LocalDaemonResponse = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(response).unwrap(), *value);
    }
    for (variant, body) in [
        (
            "CreateSession",
            serde_json::json!({"workspace_id":"workspace","worktree_id":"worktree","no_subscribe":true}),
        ),
        (
            "SpawnAgent",
            serde_json::json!({"session_id":"room","no_subscribe":true}),
        ),
        (
            "SpawnAgents",
            serde_json::json!({"session_id":"room","agents":[],"no_subscribe":true}),
        ),
    ] {
        let request: LocalDaemonRequest =
            serde_json::from_value(serde_json::json!({variant:body})).unwrap();
        assert_eq!(
            serde_json::to_value(request).unwrap()[variant]["no_subscribe"],
            snapshot["no_subscribe_requests"][variant]
        );
    }
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "5d2c5ca0123055fe9fb338984ed89140db2c2beec3e2716814212892ee6ce29d"
    );
}

#[test]
fn created_work_keeps_its_ids_and_separate_event_failure_on_the_wire() {
    let session = crate::session::RuntimeSession::new(
        "room",
        None,
        "workspace",
        "worktree",
        "machine",
        "kernel",
    );
    let agent = crate::agent::AgentInstance::new(
        "agent",
        "agent-ref",
        "room",
        None,
        "dev-stub",
        None,
        None,
        None,
        crate::agent::GridPosition::new(0, 0, 1, 1),
    );
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("outside-events.json")).unwrap();
    for mut response in [
        LocalDaemonResponse::SessionCreated {
            session,
            agent: agent.clone(),
            event_subscription_error: None,
        },
        LocalDaemonResponse::AgentSpawned {
            agent: agent.clone(),
            event_subscription_error: None,
        },
        LocalDaemonResponse::AgentsSpawned {
            agents: vec![agent],
            event_subscription_error: None,
        },
    ] {
        let before = serde_json::to_value(&response).unwrap();
        response.record_event_subscription_failure();
        let value = serde_json::to_value(response).unwrap();
        let mut without_issue = value.clone();
        let body = without_issue
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap();
        assert_eq!(
            body.as_object_mut()
                .unwrap()
                .remove("event_subscription_error")
                .unwrap(),
            fixture["creation_response_error"]
        );
        assert_eq!(without_issue, before);
        let decoded: LocalDaemonResponse = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), value);
    }
}
