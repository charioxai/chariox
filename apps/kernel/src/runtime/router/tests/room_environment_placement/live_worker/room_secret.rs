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
