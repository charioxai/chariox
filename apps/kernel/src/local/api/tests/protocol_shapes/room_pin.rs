use super::*;

#[test]
fn mp11_room_pin_588_194_snapshot_and_hash() {
    let bytes = include_bytes!("room-pin-588.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "3e6a4b1daf5f2efdb8c707d2498d5b812b727e1b76648a3c3dab1fc496ed32a4"
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
