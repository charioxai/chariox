//! Local 1011 / relay peer 448: the Room list has its own names on the wire.
use super::*;
use crate::transport::kernel_protocol::{
    kernel_subscription_scope, KernelEvent, KernelSubscriptionScope,
};
use crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION;
use sha2::{Digest, Sha256};

#[test]
fn room_list_1011_snapshot_and_hash() {
    let bytes = include_bytes!("room-list-1011.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "adbbf8c88bb7a19393071dd0e51db134635c56df97b23b478210efea16317b15"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(fixture["relayPeer"], RELAY_PEER_PROTOCOL_VERSION);
    let session = crate::session::RuntimeSession::new(
        "room",
        crate::session::ProjectId::test(),
        None,
        "/workspace",
        "/worktree",
        "machine",
        "kernel",
    );
    let wire = serde_json::to_value(session).unwrap();
    let retired_field = fixture["retired_session_field"].as_str().unwrap();
    assert!(wire.get(retired_field).is_none());
    assert_eq!(wire["agents"], serde_json::json!([]));
    for wire in fixture["requests"].as_array().unwrap() {
        let parsed: LocalDaemonRequest = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *wire);
    }
    let scope = fixture["subscription_scope"].as_str();
    assert_eq!(
        kernel_subscription_scope(scope),
        KernelSubscriptionScope::RoomList
    );
    let event: KernelEvent = serde_json::from_value(fixture["events"][0].clone()).unwrap();
    assert!(matches!(event, KernelEvent::RoomListChanged { .. }));
    assert_eq!(serde_json::to_value(event).unwrap(), fixture["events"][0]);
}
