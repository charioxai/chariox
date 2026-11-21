use super::*;

#[test]
fn prompt_control_receipts_match_the_protocol_fixture() {
    let bytes = include_bytes!("prompt-control-receipts.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "479d6a9a593bc069b59638d8ff66ef188686a45fec20285fb1d3c44207005973"
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
