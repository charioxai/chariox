use super::*;
use crate::local::{
    ForgetGatewayMachineRequest, GatewayAttachmentRef, GatewayMachine, GatewayMachineStatus,
    GatewayMachineVia, HeartbeatAttachmentsRequest, JoinMachineLinkRequest,
    ListGatewayMachinesRequest, ListMachineDirectoryRequest, MachineDirectoryEntry,
    MachineFileKind, MachineFileStat, OnBehalfOf, ReadMachineFileRequest, StatMachineFileRequest,
    WriteMachineFileRequest, MACHINE_FILE_MAX_BYTES, MACHINE_FILE_MAX_CHUNK_BYTES,
};
use crate::transport::kernel_protocol::{KernelEvent, KernelIncomingFrame, KernelOutgoingFrame};

// EA2-04/05: the shapes an external agent and a forwarding kernel depend on.
#[test]
fn machine_gateway_protocol_568_shapes_and_hash() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let list = LocalDaemonRequest::ListGatewayMachines(ListGatewayMachinesRequest {});
    let listed = LocalDaemonResponse::GatewayMachinesListed {
        machines: vec![GatewayMachine {
            name: "b3".into(),
            kernel_id: "kernel-b3".into(),
            machine_id: "machine-b3".into(),
            machine_alias: Some("ovh-b3".into()),
            status: GatewayMachineStatus::Online,
            last_seen_ms: 1_000,
            gateway_ready: true,
            via: GatewayMachineVia::Hosted,
        }],
    };
    let frame: KernelIncomingFrame = serde_json::from_value(serde_json::json!({
        "type": "request", "request_id": "r1", "command_id": "c1", "machine": "b3",
        "request": {"ListGatewayMachines": {}},
    }))
    .unwrap();
    let KernelIncomingFrame::Request { machine, .. } = &frame else {
        panic!("a request frame")
    };
    assert_eq!(machine.as_deref(), Some("b3"));
    let plain: KernelIncomingFrame = serde_json::from_value(serde_json::json!({
        "type": "request", "request_id": "r2", "request": {"ListGatewayMachines": {}},
    }))
    .unwrap();
    assert!(matches!(
        plain,
        KernelIncomingFrame::Request { machine: None, .. }
    ));
    let on_behalf_of = OnBehalfOf {
        origin_kernel: "kernel-mac".into(),
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
        "stat": LocalDaemonRequest::StatMachineFile(StatMachineFileRequest { path: "/srv/b.bundle".into(), with_sha256: true }),
        "list": LocalDaemonRequest::ListMachineDirectory(ListMachineDirectoryRequest { path: "~/".into() }),
        "read": LocalDaemonRequest::ReadMachineFile(ReadMachineFileRequest { path: "/srv/b.bundle".into(), offset: 0, length: 4 }),
        "write": LocalDaemonRequest::WriteMachineFile(WriteMachineFileRequest {
            path: "/srv/b.bundle".into(), offset: 0, data_base64: "AAEC".into(), total_size: 3,
            sha256: Some("ab".repeat(32)), overwrite: false,
        }),
        "stat_answer": LocalDaemonResponse::MachineFileStat { stat: MachineFileStat {
            path: "/srv/b.bundle".into(), exists: true, kind: Some(MachineFileKind::File), size: Some(3),
            modified_ms: Some(5), sha256: None, partial_size: None,
        } },
        "listed": LocalDaemonResponse::MachineDirectoryListed { path: "/".into(), entries: vec![MachineDirectoryEntry {
            name: "a".into(), kind: MachineFileKind::Directory, size: 0, modified_ms: None,
        }], truncated: false },
        "chunk": LocalDaemonResponse::MachineFileChunk { path: "/srv/b.bundle".into(), offset: 0, data_base64: "AAEC".into(), size: 3, eof: true },
        "written": LocalDaemonResponse::MachineFileWritten { path: "/srv/b.bundle".into(), received: 3, complete: true, sha256: None },
        "caps": [MACHINE_FILE_MAX_BYTES, MACHINE_FILE_MAX_CHUNK_BYTES],
    });
    let pairing = serde_json::json!({
        "join": LocalDaemonRequest::JoinMachineLink(JoinMachineLinkRequest {
            link: "chariox-terminal-pair-v1.x".into(), relay_token: Some("t".into()),
        }),
        "forget": LocalDaemonRequest::ForgetGatewayMachine(ForgetGatewayMachineRequest { name: "b3".into() }),
        "forgotten": LocalDaemonResponse::GatewayMachineForgotten { kernel_id: "kernel-b3".into() },
    });
    let subscribe: KernelIncomingFrame = serde_json::from_value(serde_json::json!({
        "type": "subscribe", "request_id": "r3", "session_id": "s1", "attachment_id": "a1",
        "resume_from_event_id": 7, "machine": "b3",
    }))
    .unwrap();
    let KernelIncomingFrame::Subscribe { machine, .. } = &subscribe else {
        panic!("a subscribe frame")
    };
    assert_eq!(machine.as_deref(), Some("b3"));
    let event = serde_json::to_value(KernelOutgoingFrame::Event {
        event_id: 8,
        machine: Some("b3".into()),
        event: Box::new(KernelEvent::Heartbeat {
            session_id: "s1".into(),
        }),
    })
    .unwrap();
    assert_eq!(event["machine"], "b3");
    let snapshot = serde_json::json!({
        "list": list, "listed": listed, "frame": frame, "on_behalf_of": on_behalf_of,
        "heartbeat": heartbeat, "heartbeated": heartbeated, "files": files, "pairing": pairing, "subscribe": subscribe, "event": event,
        "capability": crate::local::RUNTIME_CONTROL_CAPABILITIES.contains(&"machine_gateway_v1"),
    });
    assert_eq!(snapshot["capability"], true);
    assert_eq!(snapshot["frame"]["machine"], "b3");
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "0e989a3d3eb859810756c5b5c294c17f7f9187ca7a82e9d2493a170c8a52466d"
    );
}
