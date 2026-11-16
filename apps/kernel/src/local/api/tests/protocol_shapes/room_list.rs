//! Local 1059 / relay peer 497: the Room list has its own names on the wire; the waiting room names are gone.
use super::*;
use crate::transport::kernel_protocol::{
    kernel_subscription_scope, KernelEvent, KernelSubscriptionScope,
};
use crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION;
use sha2::{Digest, Sha256};

#[test]
fn room_list_1059_snapshot_and_hash() {
    let bytes = include_bytes!("room-list-1059.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "f0e39cd59a84b7a26bfcf1707659ab4b7ecc81d4e260508ff01f45e770702ad9"
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

#[test]
fn the_waiting_room_names_are_not_requests() {
    let fixture: serde_json::Value =
        serde_json::from_slice(include_bytes!("room-list-1059.json")).unwrap();
    for name in fixture["retired"].as_array().unwrap() {
        let wire = serde_json::json!({ name.as_str().unwrap(): null });
        assert!(
            serde_json::from_value::<LocalDaemonRequest>(wire).is_err(),
            "{name} must not parse"
        );
    }
    assert_eq!(
        kernel_subscription_scope(Some("waiting_room_inventory")),
        KernelSubscriptionScope::Session,
        "the old scope name is just an unknown scope now"
    );
}
