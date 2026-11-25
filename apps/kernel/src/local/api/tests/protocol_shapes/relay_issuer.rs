//! MP-11: protocol 1015/452 adds the self-hosted relay issuer (kernel-signed client relay tokens).
use super::*;

#[test]
fn relay_issuer_1015_snapshot() {
    use sha2::{Digest, Sha256};
    const {
        assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 1015);
    };
    const {
        assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 452);
    };
    let bytes = include_bytes!("relay-issuer-1015.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "a390be6b3bccdecdec28dd026c2f3802b1debc61438f1324829239fcba4b03c7"
    );
    let snapshot: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    for value in snapshot["requests"].as_array().unwrap() {
        let request: crate::local::LocalDaemonRequest =
            serde_json::from_value(value.clone()).unwrap();
        assert_eq!(&serde_json::to_value(&request).unwrap(), value);
    }
    for value in snapshot["responses"].as_array().unwrap() {
        let response: crate::local::LocalDaemonResponse =
            serde_json::from_value(value.clone()).unwrap();
        assert_eq!(&serde_json::to_value(&response).unwrap(), value);
    }
    let unknown_scope = serde_json::json!({ "IssueRelayClientToken": {
        "client_id": "cli-1", "public_key_thumbprint": "a", "scope": "daemon"
    }});
    assert!(serde_json::from_value::<crate::local::LocalDaemonRequest>(unknown_scope).is_err());
}
