// MP-08 / MP-09 / MP-10 / MP-11, peer 442: `chariox.file.copy` reads and writes a leased agent's
// workspace one slice at a time over the execution-lease link.
#[test]
fn mp11_leased_agent_file_copy_442_is_pinned() {
    const { assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 442) };
    use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse, RemoteFileBytes};
    let requests = [
        RelayPeerRequest::ReadLeasedAgentFile {
            leased_agent_id: "leased".into(),
            path: "out/x.bundle".into(),
            offset: 4_194_304,
        },
        RelayPeerRequest::WriteLeasedAgentFile {
            leased_agent_id: "leased".into(),
            path: "in/base.bundle".into(),
            offset: 0,
            total_size: 3,
            sha256: "ab".repeat(32),
            overwrite: false,
            content: RemoteFileBytes::encode(b"abc"),
        },
        RelayPeerRequest::DiscardLeasedAgentFile {
            leased_agent_id: "leased".into(),
            path: "in/base.bundle".into(),
        },
    ];
    let responses = [
        RelayPeerResponse::LeasedAgentFileRead {
            size_bytes: 3,
            sha256: Some("ab".repeat(32)),
            revision: "2049:17:3:1.5:1.5".into(),
            content: RemoteFileBytes::encode(b"abc"),
        },
        RelayPeerResponse::LeasedAgentFileRead {
            size_bytes: 3,
            sha256: None,
            revision: "2049:17:3:1.5:1.5".into(),
            content: RemoteFileBytes::encode(b""),
        },
        RelayPeerResponse::LeasedAgentFileWritten {
            received: 3,
            sha256: Some("ab".repeat(32)),
        },
        RelayPeerResponse::LeasedAgentFileWritten {
            received: 0,
            sha256: None,
        },
        RelayPeerResponse::LeasedAgentFileDiscarded,
        RelayPeerResponse::LeasedAgentFileRefused {
            reason: "`.git/hooks/pre-commit` is inside .git".into(),
        },
    ];
    let sha = "ab".repeat(32);
    assert_eq!(
        requests
            .iter()
            .map(|request| serde_json::to_value(request).unwrap())
            .collect::<Vec<_>>(),
        [
            serde_json::json!({"kind": "read_leased_agent_file", "leased_agent_id": "leased",
                "path": "out/x.bundle", "offset": 4_194_304}),
            serde_json::json!({"kind": "write_leased_agent_file", "leased_agent_id": "leased",
                "path": "in/base.bundle", "offset": 0, "total_size": 3, "sha256": sha,
                "overwrite": false, "content": "YWJj"}),
            serde_json::json!({"kind": "discard_leased_agent_file", "leased_agent_id": "leased",
                "path": "in/base.bundle"}),
        ]
    );
    assert_eq!(
        responses
            .iter()
            .map(|response| serde_json::to_value(response).unwrap())
            .collect::<Vec<_>>(),
        [
            serde_json::json!({"kind": "leased_agent_file_read", "size_bytes": 3, "sha256": sha,
                "revision": "2049:17:3:1.5:1.5", "content": "YWJj"}),
            serde_json::json!({"kind": "leased_agent_file_read", "size_bytes": 3,
                "revision": "2049:17:3:1.5:1.5", "content": ""}),
            serde_json::json!({"kind": "leased_agent_file_written", "received": 3, "sha256": sha}),
            serde_json::json!({"kind": "leased_agent_file_written", "received": 0}),
            serde_json::json!({"kind": "leased_agent_file_discarded"}),
            serde_json::json!({"kind": "leased_agent_file_refused",
                "reason": "`.git/hooks/pre-commit` is inside .git"}),
        ]
    );
    for request in requests {
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(
            serde_json::from_value::<RelayPeerRequest>(value).unwrap(),
            request
        );
    }
    for response in responses {
        let value = serde_json::to_value(&response).unwrap();
        assert_eq!(
            serde_json::from_value::<RelayPeerResponse>(value).unwrap(),
            response
        );
    }
}
