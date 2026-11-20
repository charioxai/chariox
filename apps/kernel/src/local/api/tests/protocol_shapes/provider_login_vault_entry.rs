//! Local 1012: a running API-key provider login takes a Vault entry by handle; the answer never carries the value.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn provider_login_vault_entry_1012_snapshot_and_hash() {
    let bytes = include_bytes!("provider-login-vault-entry-1012.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "26513cb4edb96e7e21b2ca00c1e54f5d463bb634ed23fed8f52425b2fb45f980"
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
