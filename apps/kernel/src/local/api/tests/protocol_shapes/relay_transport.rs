//! MP-08 / MP-10 / MP-11: allocated multiplex transport 532 / peer 140.
use super::*;
#[test]
fn mp08_mp10_mp11_relay_transport_532_140_snapshot() {
    use chariox_relay::frame_transport::*;
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        crate::test_support::expected_relay_peer_protocol_version()
    );
    let bytes = include_bytes!("relay-transport-532.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "dd49c2d3f1da05dceb925d3389022f4a1c52eadd9add9c7b1e2e4044eb099bc2"
    );
    let snapshot: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(snapshot["local"], 532);
    assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 532);
    assert_eq!(
        snapshot["peer"],
        140
    );
    assert_eq!(snapshot["capability"], TRANSPORT_QUERY);
    assert_eq!(snapshot["bypass_scope"], DISPLAY_EVENT_SCOPE);
    for (name, value) in [
        ("chunk_bytes", CHUNK_BYTES),
        ("window_bytes", WINDOW_BYTES),
        ("max_message_bytes", MAX_MESSAGE_BYTES),
        ("max_transfers", MAX_TRANSFERS),
        ("max_assembly_bytes", MAX_ASSEMBLY_BYTES),
    ] {
        assert_eq!(snapshot[name], value);
    }
    for name in ["chunk", "binary_checkpoint", "ack"] {
        let typed: TransportFrame = serde_json::from_value(snapshot[name].clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), snapshot[name]);
    }
}
