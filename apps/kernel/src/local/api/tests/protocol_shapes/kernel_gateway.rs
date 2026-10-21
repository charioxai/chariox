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
        "3392afcfcfdb6b3ad9ef9e89adc30abee92ee909715109eb6d926426ccb61d44"
    );
}
