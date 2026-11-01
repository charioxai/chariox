use super::*;
use sha2::{Digest, Sha256};

#[test]
fn room_exec_protocol_snapshots_and_hashes() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/room-exec-protocol.json"
    )))
    .unwrap();
    assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= fixture["local"].as_u64().unwrap() as u32);
    assert!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
            >= fixture["relayPeer"].as_u64().unwrap() as u32
    );
    for shape in fixture["shapes"].as_array().unwrap() {
        let wire = shape["wire"].clone();
        let encoded = if shape["kind"] == "request" {
            let request: LocalDaemonRequest = serde_json::from_value(wire.clone()).unwrap();
            serde_json::to_value(request).unwrap()
        } else {
            let response: LocalDaemonResponse = serde_json::from_value(wire.clone()).unwrap();
            serde_json::to_value(response).unwrap()
        };
        assert_eq!(encoded, wire);
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&encoded).unwrap())
            ),
            shape["sha256"].as_str().unwrap()
        );
    }
}
