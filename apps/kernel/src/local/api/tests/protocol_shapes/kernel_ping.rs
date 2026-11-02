use super::*;
#[test]
fn mp08_kernel_ping_protocol_snapshot_and_hash() {
    let bytes = include_bytes!("kernel-ping-951.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "175f226cc65dc7fff6df5bc8905f3ac606a74d6ab60f918ca695d02e031197c1"
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
