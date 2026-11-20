//! Local 1013: an owner's kernel makes a state bundle key in its Vault and answers Cloud's recovery challenge; replies carry the recipient and a proof time, never the key.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn state_bundle_key_1013_snapshot_and_hash() {
    let bytes = include_bytes!("state-bundle-key-1013.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "0f0cb684c66ebce97a74a88f8a60554db7463366ef93b410638e72bc2b997b5a"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local"], LOCAL_DAEMON_PROTOCOL_VERSION);
    for wire in fixture["requests"].as_array().unwrap() {
        let parsed: LocalDaemonRequest = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *wire);
    }
    for wire in fixture["responses"].as_array().unwrap() {
        let parsed: LocalDaemonResponse = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *wire);
    }
}
