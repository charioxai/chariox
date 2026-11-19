//! Local 1012 / relay peer 449: a watcher is a notification source the kernel runs, made and repaired by an agent.
use super::*;
use crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION;
use sha2::{Digest, Sha256};

#[test]
fn watcher_1012_449_snapshot_and_hash() {
    let bytes = include_bytes!("watcher-1012.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "5ed9db9b68f3fb0f9492163b5d27ca737f801584c2e3fa1f2689b4a96a8908ff"
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
