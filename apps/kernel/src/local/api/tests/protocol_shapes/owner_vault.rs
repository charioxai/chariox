//! MP-08 / MP-10 / MP-11: protocol 958/407 owner-only Vault shapes.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn owner_vault_protocol_snapshot_and_hash() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../../fixtures/owner-vault-protocol.json"
    ))
    .unwrap();
    assert_eq!(fixture["local"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(
        fixture["peer"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    for expected in fixture["requests"].as_array().unwrap() {
        let request: LocalDaemonRequest = serde_json::from_value(expected.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), *expected);
        let mut extra = expected.clone();
        extra["OwnerVault"]["agent_id"] = serde_json::json!("agent");
        assert!(serde_json::from_value::<LocalDaemonRequest>(extra).is_err());
    }
    for expected in fixture["responses"].as_array().unwrap() {
        let response: LocalDaemonResponse = serde_json::from_value(expected.clone()).unwrap();
        assert_eq!(serde_json::to_value(response).unwrap(), *expected);
    }
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&fixture).unwrap())
        ),
        "5010478c444895826e935c44645da336485562934292115a7b9642cd7672455f"
    );
}
