//! MP-08/MP-10/MP-11: account and agent references at local 932 / peer 380.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn usage_snapshot_770_contract_roundtrips_without_private_profiles() {
    let bytes = include_bytes!("usage-snapshot-770.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "9f6682f4d6ba5099211b7e2fa05e25b8a29d47d1db37e6c4829e40552af39853"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    for request in fixture["requests"].as_array().unwrap() {
        let decoded: LocalDaemonRequest = serde_json::from_value(request.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), *request);
    }
    for response in fixture["responses"].as_array().unwrap() {
        let decoded: LocalDaemonResponse = serde_json::from_value(response.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), *response);
    }
    const {
        assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 810);
    };
    const {
        assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 360);
    };
}
