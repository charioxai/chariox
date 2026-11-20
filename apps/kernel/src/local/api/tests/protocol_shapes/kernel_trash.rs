//! Local 1012 / relay peer 449: Move to Trash and Undo for the files of a machine.
use super::*;
use crate::local::{KernelFileFailure, TrashedKernelFile};
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use sha2::{Digest, Sha256};

#[test]
fn kernel_trash_1012_snapshot_and_hash() {
    let bytes = include_bytes!("kernel-trash-1012.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "563ac71a8bc2ade688db4ec61cba1a3faa3874d74b5c463cdb8f91d6dd5e0a64"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(
        fixture["relayPeer"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    for wire in fixture["requests"].as_array().unwrap() {
        let parsed: LocalDaemonRequest = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *wire);
    }
    let trashed = || {
        vec![TrashedKernelFile {
            path: "/home/me/a.txt".into(),
            trash_path: "/home/me/.local/share/Trash/files/a.txt".into(),
        }]
    };
    let responses = [
        LocalDaemonResponse::KernelFilesTrashed {
            trashed: trashed(),
            failed: vec![KernelFileFailure {
                path: "~/old".into(),
                message: "/home/me/old does not exist; check the path with a directory listing."
                    .into(),
            }],
        },
        LocalDaemonResponse::KernelTrashRestored {
            restored: vec!["/home/me/a.txt".into()],
            failed: vec![],
        },
    ];
    assert_eq!(
        serde_json::to_value(responses).unwrap(),
        fixture["responses"]
    );
    for wire in fixture["peerRequests"].as_array().unwrap() {
        let parsed: RelayPeerRequest = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *wire);
    }
    for wire in fixture["peerResponses"].as_array().unwrap() {
        let parsed: RelayPeerResponse = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *wire);
    }
}

/// A restore names the original path and the trash path; anything else on the wire is refused.
#[test]
fn a_trash_request_with_an_unknown_field_is_refused() {
    assert!(serde_json::from_value::<LocalDaemonRequest>(
        serde_json::json!({"TrashKernelFiles": {"paths": ["/a"], "permanent": true}})
    )
    .is_err());
    assert!(serde_json::from_value::<LocalDaemonRequest>(
        serde_json::json!({"RestoreKernelTrash": {"items": [{"path": "/a"}]}})
    )
    .is_err());
}
