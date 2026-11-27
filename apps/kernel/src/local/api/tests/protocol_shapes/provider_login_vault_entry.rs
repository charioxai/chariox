//! Local 1012: a running API-key provider login takes a Vault entry by handle; the answer never carries the value.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn provider_login_vault_entry_1012_snapshot_and_hash() {
    let bytes = include_bytes!("provider-login-vault-entry-1012.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "4e50c8f9e8596b10b66a3a2f206c6e6ca7f4e3d252f72c73b285c74340cf9aa9"
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
