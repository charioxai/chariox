use super::*;

// Local 973: the owner's client gives a kernel its own Vault, also one that opens with another kernel's passphrase.
#[test]
fn credential_vault_creation_973_shapes_are_pinned() {
    const { assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 973) };
    let typed =
        LocalDaemonRequest::CreateCredentialVault(crate::local::CreateCredentialVaultRequest {
            origin: crate::local::CredentialVaultOrigin::Passphrase {
                passphrase: crate::local::VaultPassphrase::new("typed by the owner"),
            },
        });
    assert_eq!(
        serde_json::to_value(&typed).unwrap(),
        serde_json::json!({"CreateCredentialVault": {"origin": {"kind": "passphrase", "passphrase": "typed by the owner"}}})
    );
    let seal =
        LocalDaemonRequest::SealCredentialVaultKey(crate::local::SealCredentialVaultKeyRequest {
            target_kernel_id: "kernel-2".into(),
            target_public_key: "public".into(),
        });
    assert_eq!(
        serde_json::to_value(&seal).unwrap(),
        serde_json::json!({"SealCredentialVaultKey": {"targetKernelId": "kernel-2", "targetPublicKey": "public"}})
    );
    for refused in [
        serde_json::json!({"CreateCredentialVault": {}}),
        serde_json::json!({"CreateCredentialVault": {"origin": {"kind": "passphrase"}}}),
        serde_json::json!({"CreateCredentialVault": {"origin": {"kind": "seed", "seed": {}}}}),
        serde_json::json!({"CreateCredentialVault": {"origin": {"kind": "other"}}}),
        serde_json::json!({"SealCredentialVaultKey": {"targetKernelId": "k"}}),
    ] {
        assert!(
            serde_json::from_value::<LocalDaemonRequest>(refused.clone()).is_err(),
            "{refused}"
        );
    }
}

#[test]
fn a_sealed_vault_key_travels_as_an_opaque_seed_that_round_trips() {
    let source_private = crate::transport::relay_crypto::generate_private_key_base64();
    let target_private = crate::transport::relay_crypto::generate_private_key_base64();
    let target_public =
        crate::transport::relay_crypto::public_key_from_private_key_base64(&target_private)
            .unwrap();
    let dir = std::env::temp_dir().join(format!("vault-seed-shape-{:016x}", rand::random::<u64>()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("vault.json");
    crate::secret::create_chariox_encrypted_vault_for_test(&path, "correct horse").unwrap();
    crate::secret::unlock_chariox_encrypted_vault(
        &path,
        "correct horse",
        crate::secret::VaultUnlockLease::Operation,
    )
    .unwrap();
    let seed = crate::secret::export_vault_key_seed(
        &path,
        "kernel-1",
        &source_private,
        "kernel-2",
        &target_public,
    )
    .unwrap();
    let response = LocalDaemonResponse::CredentialVaultKeySealed { seed: seed.clone() };
    let value = serde_json::to_value(&response).unwrap();
    let sealed = &value["CredentialVaultKeySealed"]["seed"];
    let mut fields = sealed
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    fields.sort();
    assert_eq!(
        fields,
        [
            "kdf",
            "sealedKey",
            "sourceKernelId",
            "sourceKeyThumbprint",
            "targetKernelId",
            "targetKeyThumbprint"
        ]
    );
    assert!(!value.to_string().contains("correct horse"));
    assert!(
        !format!("{response:?}").contains("sealedKey\":"),
        "Debug shows no sealed key"
    );
    let again: LocalDaemonRequest = serde_json::from_value(
        serde_json::json!({"CreateCredentialVault": {"origin": {"kind": "seed", "seed": sealed}}}),
    )
    .unwrap();
    assert!(
        matches!(again, LocalDaemonRequest::CreateCredentialVault(request) if matches!(request.origin, crate::local::CredentialVaultOrigin::Seed { seed: ref echoed } if *echoed == seed))
    );
    let _ = std::fs::remove_dir_all(dir);
}
