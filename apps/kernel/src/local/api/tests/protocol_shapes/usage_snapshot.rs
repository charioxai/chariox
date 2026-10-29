//! MP-08/MP-10/MP-11: account and agent references at local 910 / peer 361.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn usage_snapshot_770_contract_roundtrips_without_private_profiles() {
    let bytes = include_bytes!("usage-snapshot-770.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "ea1815c6cd0c391b7a07fa15987662ecdd2c87c3bfed8210a6c633624121c4bb"
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
