//! MP-08/MP-10/MP-11: account and agent references at local 810 / peer 360.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn usage_snapshot_810_contract_roundtrips_without_private_profiles() {
    let bytes = include_bytes!("usage-snapshot-810.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "35fd1ea4eab6b07d92f5e536c20ee08a3b59e4a6b3cc14c365be7eccdf9bf664"
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
