//! MP-08 / MP-09 / MP-10 / MP-11: a rejected code keeps the login alive.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn provider_login_810_retry_status_contract_roundtrips() {
    let bytes = include_bytes!("provider-login-810.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "cea99877e49906af06bdb51fc3a5d1a98e036237adaf09fd3c5332bb4ffb96b4"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    let request: LocalDaemonRequest = serde_json::from_value(fixture["request"].clone()).unwrap();
    let response: LocalDaemonResponse =
        serde_json::from_value(fixture["response"].clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), fixture["request"]);
    assert_eq!(serde_json::to_value(response).unwrap(), fixture["response"]);
    const {
        assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 810);
    };
    const {
        assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 360);
    };
}
