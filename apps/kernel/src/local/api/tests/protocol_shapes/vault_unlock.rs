use super::*;

// Local 923 / relay peer 374: the owner's client unlocks the Vault with no session.
#[test]
fn unlock_credential_vault_912_363_shapes_are_pinned() {
    const { assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 923) };
    const { assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 363) };
    let request =
        LocalDaemonRequest::UnlockCredentialVault(crate::local::UnlockCredentialVaultRequest {
            passphrase: crate::local::VaultPassphrase::new("typed by the owner"),
        });
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        serde_json::json!({"UnlockCredentialVault": {"passphrase": "typed by the owner"}})
    );
    assert!(serde_json::from_value::<LocalDaemonRequest>(
        serde_json::json!({"UnlockCredentialVault": {"passphrase": "x", "session_id": "s"}})
    )
    .is_err());
    assert!(serde_json::from_value::<LocalDaemonRequest>(
        serde_json::json!({"UnlockCredentialVault": {}})
    )
    .is_err());
    let response = LocalDaemonResponse::CredentialVaultUnlocked {
        status: crate::secret::CharioxVaultUnlockStatus {
            path: "/vault.json".into(),
            unlocked: true,
            expires_at_ms: Some(7),
        },
        action: "unlocked".into(),
    };
    assert_eq!(
        serde_json::to_value(&response).unwrap(),
        serde_json::json!({"CredentialVaultUnlocked": {
            "status": {"path": "/vault.json", "unlocked": true, "expires_at_ms": 7},
            "action": "unlocked"
        }})
    );
}
