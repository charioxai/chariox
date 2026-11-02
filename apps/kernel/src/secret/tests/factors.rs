//! MP-08 / MP-09 / MP-10 / MP-11: the public Vault storage/input entry points.
use super::*;

fn factor(id: &str, injection: serde_json::Value) -> UserCredentialConfig {
    serde_json::from_value(serde_json::json!({
        "id":id, "source":{"type":"vault","key":id},
        "allowed_hosts":["example.com"],
        "allowed_uses":["browser","pty","env"], "injection":injection
    }))
    .unwrap()
}

#[test]
fn capture_totp_uri_stores_seed_and_provisioning_parameters_atomically() {
    let root = crate::test_support::TestWorktree::new("totp-capture");
    let registry = CharioxCredentialRegistry::new(root.path().join("registry"));
    let store = Arc::new(MemoryVaultStore::default());
    let service = RuntimeSecretService::with_vault_store(Vec::new(), "totp-capture", store.clone());
    let uri = "otpauth://totp/Example:alice?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&algorithm=SHA256&digits=8&period=60&issuer=Example";
    let credential = factor("factor", serde_json::json!({"kind":"totp"}));
    let stored = service
        .upsert_vault_backed_credential_with_secret(&registry, credential, uri, false)
        .unwrap();
    assert_eq!(stored.credential_id, "factor");
    let registered = registry.get("factor").unwrap().unwrap();
    assert_eq!(
        registered.injection,
        UserCredentialInjectionConfig::Totp(crate::config::TotpConfig {
            algorithm: crate::config::TotpAlgorithm::Sha256,
            digits: 8,
            period_s: 60,
        })
    );
    let seed = store.get_secret("totp-capture", "factor").unwrap();
    assert!(seed == "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
    let reloaded =
        RuntimeSecretService::with_vault_store(vec![registered.clone()], "totp-capture", store);
    for code in [
        reloaded
            .browser_secret_input_for_target_url("factor", "https://example.com/login")
            .unwrap(),
        reloaded.terminal_secret_input("factor").unwrap(),
        reloaded.environment_secret_input("factor").unwrap(),
    ] {
        assert!(code.len() == 8 && code.bytes().all(|c| c.is_ascii_digit()));
        let handles = serde_json::to_string(&reloaded.list_handles()).unwrap();
        assert!(!handles.contains(&code) && !handles.contains(&seed));
    }
    assert!(service
        .upsert_vault_backed_credential_with_secret(
            &registry,
            registered.clone(),
            "PRIVATE-NOT-BASE32",
            true
        )
        .is_err());
    assert_eq!(registry.get("factor").unwrap().unwrap(), registered);
}

#[test]
fn factor_configuration_rejects_wrong_sources_and_uses() {
    let credential = factor("factor", serde_json::json!({"kind":"totp"}));
    for injection in [
        serde_json::json!({"kind":"totp","digits":7}),
        serde_json::json!({"kind":"totp","period_s":0}),
        serde_json::json!({"kind":"totp","period_s":3601}),
    ] {
        assert!(crate::config::validate_credentials(&[factor("factor", injection)]).is_err());
    }
    for usage in [
        vec![],
        vec![UserCredentialUse::Http],
        vec![UserCredentialUse::Provider],
    ] {
        let mut invalid = credential.clone();
        invalid.allowed_uses = usage;
        assert!(crate::config::validate_credentials(&[invalid]).is_err());
    }
    let mut invalid = credential;
    invalid.source = UserCredentialSourceConfig::Env {
        name: "SEED".into(),
    };
    assert!(crate::config::validate_credentials(&[invalid]).is_err());
}

#[test]
fn backup_codes_are_a_separate_durable_single_use_credential() {
    let root = crate::test_support::TestWorktree::new("backup-code-capture");
    let registry = CharioxCredentialRegistry::new(root.path().join("registry"));
    let store = Arc::new(MemoryVaultStore::default());
    let service =
        RuntimeSecretService::with_vault_store(Vec::new(), "backup-code-capture", store.clone());
    let credential = factor("recovery", serde_json::json!({"kind":"backup_codes"}));
    service
        .upsert_vault_backed_credential_with_secret(
            &registry,
            credential.clone(),
            "first-recovery\nsecond-recovery",
            false,
        )
        .unwrap();
    let threads = (0..2)
        .map(|_| {
            let store = store.clone();
            let credential = credential.clone();
            std::thread::spawn(move || {
                RuntimeSecretService::with_vault_store(
                    vec![credential],
                    "backup-code-capture",
                    store,
                )
                .browser_secret_input_for_target_url("recovery", "https://example.com/2fa")
                .unwrap()
            })
        })
        .collect::<Vec<_>>();
    let values = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        values.len() == 2,
        "concurrent fills must consume distinct codes"
    );
    let reloaded =
        RuntimeSecretService::with_vault_store(vec![credential], "backup-code-capture", store);
    let error = reloaded
        .environment_secret_input("recovery")
        .unwrap_err()
        .to_string();
    assert!(error.contains("no unused backup codes") && !error.contains("recovery\n"));
    let handles = serde_json::to_string(&reloaded.list_handles()).unwrap();
    assert!(values.iter().all(|code| !handles.contains(code)));
}

#[test]
fn factor_scope_checks_do_not_consume_recovery_codes() {
    let store = Arc::new(MemoryVaultStore::default());
    store
        .set_secret("backup-scope", "recovery", "only-recovery")
        .unwrap();
    let mut credential = factor("recovery", serde_json::json!({"kind":"backup_codes"}));
    credential.allowed_uses = vec![UserCredentialUse::Browser];
    let service = RuntimeSecretService::with_vault_store(vec![credential], "backup-scope", store);
    assert!(service
        .browser_secret_input_for_target_url("recovery", "https://other.example/2fa")
        .is_err());
    assert!(service.terminal_secret_input("recovery").is_err());
    assert!(service.environment_secret_input("recovery").is_err());
    assert!(
        service
            .browser_secret_input_for_target_url("recovery", "https://example.com/2fa")
            .unwrap()
            == "only-recovery"
    );
    assert!(service
        .browser_secret_input_for_target_url("recovery", "https://example.com/2fa")
        .is_err());
}
