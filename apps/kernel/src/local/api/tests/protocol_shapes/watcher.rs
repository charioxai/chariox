//! Local 1012 / relay peer 449: a watcher is a notification source the kernel runs, made and repaired by an agent.
use super::*;
use crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION;
use sha2::{Digest, Sha256};

#[test]
fn watcher_1012_449_snapshot_and_hash() {
    let bytes = include_bytes!("watcher-1012.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "e5d1247bd7d4c433f73e337341e5ee946c714de417593da3c72405849e3fe5c4"
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
