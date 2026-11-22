//! Local 1012 / relay peer 449: a watcher is a notification source the kernel runs, made and repaired by an agent.
use super::*;
use crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION;
use sha2::{Digest, Sha256};

#[test]
fn watcher_1012_449_snapshot_and_hash() {
    let bytes = include_bytes!("watcher-1012.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "d0c15058a195677b55fb72bd98d9be9e0d902d1acf5bb12938c1327c876a814b"
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
}
