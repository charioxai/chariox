use super::*;

#[test]
fn mp11_room_pin_588_194_snapshot_and_hash() {
    let bytes = include_bytes!("room-pin-588.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "0349bb8d5502db2a90e011aae506f7d2115aeca8df5ac83ad6d2250f1c21e74d"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local_protocol"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(
        fixture["peer_protocol"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    for key in ["pin", "unpin"] {
        let request: LocalDaemonRequest = serde_json::from_value(fixture[key].clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), fixture[key]);
    }
}
