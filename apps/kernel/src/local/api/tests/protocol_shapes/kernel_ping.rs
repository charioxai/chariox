use super::*;
#[test]
fn mp08_kernel_ping_protocol_snapshot_and_hash() {
    let bytes = include_bytes!("kernel-ping-951.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "9db66ba8e82e572289b755e393945cbc15119ec7565183792059763a1ed10987"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local_protocol"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(
        fixture["peer_protocol"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    let request: LocalDaemonRequest = serde_json::from_value(fixture["request"].clone()).unwrap();
    let response: LocalDaemonResponse =
        serde_json::from_value(fixture["response"].clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), fixture["request"]);
    assert_eq!(serde_json::to_value(response).unwrap(), fixture["response"]);
}
