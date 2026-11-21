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
        "9e07f370f051c3021693ba1dc790ecf0841cf363bbd0d1e4b1e7f2907df2e5b4"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(fixture["relayPeer"], RELAY_PEER_PROTOCOL_VERSION);
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
