//! Local 1012: a running API-key provider login takes a Vault entry by handle; the answer never carries the value.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn provider_login_vault_entry_1012_snapshot_and_hash() {
    let bytes = include_bytes!("provider-login-vault-entry-1012.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "0bf47cd58a808de8f2c4f3669a5380c291807a2f5d0f4fedd1485b65de9741d5"
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
