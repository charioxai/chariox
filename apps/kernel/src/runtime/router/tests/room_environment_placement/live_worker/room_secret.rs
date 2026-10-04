// MP-08/MP-10/MP-11: real home/worker relay and controller routing with a
// synthetic CDP field. No official provider or durable account is involved.
use super::controller_worker_mcp::ScopedEnvironment;
use super::*;

pub(super) async fn check(
    fixture: &LiveWorker,
    caller: &crate::runtime::state::KernelRuntimeState,
    token: &str,
) {
    let credential_home = fixture.home_state.root.join("room-secret-home");
    let _environment = ScopedEnvironment::set([
        ("CHARIOX_HOME", credential_home.as_os_str().to_os_string()),
        ("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT", "1".into()),
    ]);
    let registry = crate::credential::CharioxCredentialRegistry::user().unwrap();
    let service = crate::secret::RuntimeSecretService::with_vault_config(
        Vec::new(),
        &fixture.home_state.config.user_config.credential_vault,
    )
    .unwrap();
    service
        .upsert_vault_backed_credential_with_secret(
            &registry,
            crate::config::UserCredentialConfig {
                id: "home-room-login".into(),
                description: None,
                source: crate::config::UserCredentialSourceConfig::Vault {
                    key: "room-regression".into(),
                },
                allowed_hosts: vec!["worker.test".into()],
                allowed_uses: vec![crate::config::UserCredentialUse::Browser],
                injection: crate::config::UserCredentialInjectionConfig::Browser,
                metadata: None,
            },
            "home-room-vault-regression-value",
            false,
        )
        .expect("seed the isolated home Vault");
    let mask = fixture._worker_state.root.join("secret-input-mode");
    std::fs::write(&mask, "masked fixture").unwrap();
    let status = caller
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
        .await
        .expect("discover the Room's masked field");
    let field = status.payload["browser"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["label"] == "Worker note")
        .unwrap()["field_id"]
        .clone();
    // A nonexistent credential makes ordering observable: host mismatch must
    // win before any Vault lookup, not pass because no credential was found.
    let error = caller
        .dispatch_authenticated_runtime_tool_call(
            token,
            "paste_secret_to_slice",
            json!({
                "credential_id":"missing-credential", "field_id":field,
                "expected_host":"wrong-host.test"
            }),
        )
        .await
        .expect_err("reject host mismatch before resolving the credential");
    assert!(
        error.to_string().contains("does not match expected host"),
        "{error}"
    );
    let state_file = fixture._worker_state.root.join("chromium-state.json");
    let before: Value = serde_json::from_slice(&std::fs::read(&state_file).unwrap()).unwrap();
    assert!(before["secretInputCount"].is_null());
    let pasted = caller
        .dispatch_authenticated_runtime_tool_call(
            token,
            "mcp__chariox__paste_secret_to_slice",
            json!({
                "credential_id":"home-room-login", "field_id":field,
                "expected_host":"worker.test"
            }),
        )
        .await
        .expect("insert through the home Room's Vault and bound slice");
    assert!(pasted.ok);
    assert!(!pasted
        .payload
        .to_string()
        .contains("home-room-vault-regression-value"));
    let after: Value = serde_json::from_slice(&std::fs::read(&state_file).unwrap()).unwrap();
    assert_eq!(after["secretInputMatches"], true);
    assert_eq!(after["secretInputCount"], 1);
    std::fs::remove_file(mask).unwrap();
    service.delete_vault_secret("room-regression").unwrap();
}

// MP-08/MP-10/MP-11: lifecycle revocation reaches both authenticated registry owners.
pub(super) async fn check_revocation(fixture: &LiveWorker) {
    use sha2::{Digest, Sha256};
    let room = &fixture.rooms[0];
    let marker = format!("{:x}", Sha256::digest(room.as_bytes()));
    for state in [&fixture.home_state, &fixture._worker_state] {
        let path = state
            .config
            .private_runtime_state_root()
            .join("room-observation-quarantine")
            .join(&marker)
            .with_extension("sealed");
        assert!(
            path.exists(),
            "synthetic Room value was sealed before revocation"
        );
    }
    fixture
        .home
        .runtime_state
        .revoke_vault_observation_values("room-regression")
        .await
        .unwrap();
    for state in [&fixture.home_state, &fixture._worker_state] {
        let path = state
            .config
            .private_runtime_state_root()
            .join("room-observation-quarantine")
            .join(&marker)
            .with_extension("sealed");
        let sealed = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let plaintext = crate::transport::relay_crypto::decrypt_payload_for_private_key_bound(
            &state.config.relay_private_key,
            &sealed,
            b"room-vault-observation-registry-v1",
            room.as_bytes(),
        )
        .unwrap();
        let registry: Value = serde_json::from_slice(&plaintext.plaintext).unwrap();
        assert!(
            registry["values"].as_array().unwrap().is_empty(),
            "retired synthetic values must be removed on both home and worker"
        );
        assert!(
            registry["unknown"].as_bool().unwrap(),
            "old page echoes remain fenced after revocation"
        );
    }
}
