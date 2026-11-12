//! MP-08 / MP-09 / MP-10 / MP-11: S8 Vault factor metadata, never values.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn totp_protocol_945_394_snapshot_and_hash() {
    let snapshot: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../../fixtures/totp-protocol.json"
    ))
    .unwrap();
    assert_eq!(
        snapshot["local"],
        serde_json::json!(LOCAL_DAEMON_PROTOCOL_VERSION)
    );
    assert_eq!(
        snapshot["relayPeer"],
        serde_json::json!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION)
    );
    let credentials: Vec<crate::config::UserCredentialConfig> =
        serde_json::from_value(snapshot["credentials"].clone()).unwrap();
    crate::config::validate_credentials(&credentials).unwrap();
    assert_eq!(
        serde_json::to_value(credentials).unwrap(),
        snapshot["credentials"]
    );
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "f578a58046250b8c559c36f270711585593d2338da4da848afc9b298a4a1647c"
    );
}
