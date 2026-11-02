//! MP-08 / MP-09 / MP-10 / MP-11: persisted Vault use bindings, 956/405.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn vault_use_956_405_snapshot_and_hash() {
    assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 956);
    assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 405);
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("vault-use-956.json")).unwrap();
    let bindings: Vec<crate::config::VaultUseBinding> =
        serde_json::from_value(expected.clone()).unwrap();
    assert_eq!(serde_json::to_value(&bindings).unwrap(), expected);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_string(&expected).unwrap().as_bytes())
        ),
        "703bae087865bd3554ddb36d4717ed0d4cdb0b2f9fae1ebcf39590badf5c3701"
    );
    for binding in bindings {
        binding.target.validate().unwrap();
        let request: LocalDaemonRequest = serde_json::from_value(serde_json::json!({
            "UpsertCredential":{"credential":{"id":"bound-entry","source":{"type":"vault","key":"bound-entry"},
            "injection":{"kind":"browser"},"metadata":{"use_binding":binding}}}
        })).unwrap();
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(
            value["UpsertCredential"]["credential"]["metadata"]["use_binding"],
            serde_json::to_value(binding).unwrap()
        );
    }
}
