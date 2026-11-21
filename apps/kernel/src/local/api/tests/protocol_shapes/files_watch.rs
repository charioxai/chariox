use super::*;
use crate::transport::kernel_protocol::KernelEvent;
use crate::transport::relay_peer::{
    RelayPeerRequest, RelayPeerResponse, RELAY_PEER_PROTOCOL_VERSION,
};

#[test]
fn files_watch_1011_448_snapshot_and_hash() {
    let bytes = include_bytes!("files-watch-1011.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "b60de432dc228c1ca3f6a825b3fd3408d33b4e4192387c8b3523791b3d81ab53"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(fixture["relayPeer"], RELAY_PEER_PROTOCOL_VERSION);
    for wire in fixture["requests"].as_array().unwrap() {
        let parsed: LocalDaemonRequest = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *wire);
    }
    for wire in fixture["responses"].as_array().unwrap() {
        let parsed: LocalDaemonResponse = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *wire);
    }
    for wire in fixture["events"].as_array().unwrap() {
        let parsed: KernelEvent = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *wire);
    }
    for wire in fixture["peerRequests"].as_array().unwrap() {
        let parsed: RelayPeerRequest = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *wire);
    }
    for wire in fixture["peerResponses"].as_array().unwrap() {
        let parsed: RelayPeerResponse = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *wire);
    }
}
