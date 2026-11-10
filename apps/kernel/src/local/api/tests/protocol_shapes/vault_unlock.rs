use super::*;

// Local 923 / relay peer 374: the owner's client unlocks the Vault with no session; local 984 / peer 425 add the duration.
#[test]
fn unlock_credential_vault_912_363_shapes_are_pinned() {
    const { assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 984) };
    const { assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 425) };
    let request =
        LocalDaemonRequest::UnlockCredentialVault(crate::local::UnlockCredentialVaultRequest {
            passphrase: crate::local::VaultPassphrase::new("typed by the owner"),
            duration: crate::local::VaultUnlockDuration::Minutes { minutes: 30 },
            surface: crate::local::VaultUnlockSurface::VaultWindow,
        });
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        serde_json::json!({"UnlockCredentialVault": {
            "passphrase": "typed by the owner",
            "duration": {"kind": "minutes", "minutes": 30},
            "surface": "vault_window"
        }})
    );
    assert!(serde_json::from_value::<LocalDaemonRequest>(
        serde_json::json!({"UnlockCredentialVault": {"passphrase": "x", "duration": {"kind": "once"}, "surface": "terminal", "session_id": "s"}})
    )
    .is_err());
    assert!(serde_json::from_value::<LocalDaemonRequest>(
        serde_json::json!({"UnlockCredentialVault": {}})
    )
    .is_err());
    // The owner always picks a duration; a request without one is refused.
    assert!(serde_json::from_value::<LocalDaemonRequest>(
        serde_json::json!({"UnlockCredentialVault": {"passphrase": "x", "surface": "terminal"}})
    )
    .is_err());
    // The owner's client says where it asked from; a request without it is refused.
    assert!(serde_json::from_value::<LocalDaemonRequest>(
        serde_json::json!({"UnlockCredentialVault": {"passphrase": "x", "duration": {"kind": "once"}}})
    )
    .is_err());
    for (duration, wire) in [
        (
            crate::local::VaultUnlockDuration::Once,
            serde_json::json!({"kind": "once"}),
        ),
        (
            crate::local::VaultUnlockDuration::UntilKernelStops,
            serde_json::json!({"kind": "until_kernel_stops"}),
        ),
    ] {
        assert_eq!(serde_json::to_value(duration).unwrap(), wire);
    }
    let status = LocalDaemonResponse::CredentialVaultStatus {
        status: crate::secret::CharioxVaultUnlockStatus {
            awaiting_passphrase: false,
            path: "/vault.json".into(),
            unlocked: false,
            expires_at_ms: None,
            single_use: false,
        },
        unlock: crate::local::VaultUnlockOptions {
            durations: vec![
                crate::local::VaultUnlockDuration::Once,
                crate::local::VaultUnlockDuration::Minutes { minutes: 60 },
                crate::local::VaultUnlockDuration::UntilKernelStops,
            ],
            default: crate::local::VaultUnlockDuration::Minutes { minutes: 60 },
        },
    };
    assert_eq!(
        serde_json::to_value(&status).unwrap(),
        serde_json::json!({"CredentialVaultStatus": {
            "status": {"path": "/vault.json", "unlocked": false},
            "unlock": {
                "durations": [{"kind": "once"}, {"kind": "minutes", "minutes": 60}, {"kind": "until_kernel_stops"}],
                "default": {"kind": "minutes", "minutes": 60}
            }
        }})
    );
    let response = LocalDaemonResponse::CredentialVaultUnlocked {
        status: crate::secret::CharioxVaultUnlockStatus {
            awaiting_passphrase: false,
            path: "/vault.json".into(),
            unlocked: true,
            expires_at_ms: Some(7),
            single_use: false,
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
    // Local 1002: a Just this once unlock says so; the next Vault read ends it.
    let single_use = crate::secret::CharioxVaultUnlockStatus {
        awaiting_passphrase: false,
        path: "/vault.json".into(),
        unlocked: true,
        expires_at_ms: None,
        single_use: true,
    };
    assert_eq!(
        serde_json::to_value(&single_use).unwrap(),
        serde_json::json!({"path": "/vault.json", "unlocked": true, "single_use": true})
    );
    assert_eq!(
        serde_json::from_value::<crate::secret::CharioxVaultUnlockStatus>(
            serde_json::json!({"path": "/vault.json", "unlocked": true, "single_use": true})
        )
        .unwrap(),
        single_use
    );
    // A kernel with no Vault yet says so; the field is absent otherwise.
    let awaiting = crate::secret::CharioxVaultUnlockStatus {
        path: "/vault.json".into(),
        unlocked: false,
        expires_at_ms: None,
        single_use: false,
        awaiting_passphrase: true,
    };
    assert_eq!(
        serde_json::to_value(&awaiting).unwrap(),
        serde_json::json!({"path": "/vault.json", "unlocked": false, "awaiting_passphrase": true})
    );
}
