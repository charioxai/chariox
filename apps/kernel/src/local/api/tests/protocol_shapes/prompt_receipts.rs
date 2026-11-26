use super::*;

#[test]
fn prompt_control_receipts_match_the_protocol_fixture() {
    let bytes = include_bytes!("prompt-control-receipts.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "0f17e29790230d2be007c79f0e25e3d87c88a0cfddc1ede7f982208545cb9599"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(
        fixture["peer"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    for wire in fixture["responses"].as_array().unwrap() {
        let response: LocalDaemonResponse = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(response).unwrap(), *wire);
    }
}
