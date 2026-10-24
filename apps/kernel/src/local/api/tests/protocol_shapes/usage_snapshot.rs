//! MP-08/MP-10/MP-11: account and agent references at local 589 / peer 195.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn usage_snapshot_589_contract_roundtrips_without_private_profiles() {
    let bytes = include_bytes!("usage-snapshot-589.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "7ff5d2a15cf349b1b36a4dfeaf20c13bfe5e731adb2438af49d9613853de6153"
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
        assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 589);
    };
    const {
        assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 195);
    };
}
