//! MP-08/MP-10/MP-11: explicit status needs token proof, not a cached observation.
use super::*;

#[test]
fn explicit_status_verifies_the_current_encrypted_vault_account() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let root = crate::test_support::TestWorktree::new("claude-explicit-vault-status");
    let executable = root.path().join("claude");
    const CANARY: &str = "sk-ant-oat01-TESTONLY-status-verifier-aaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    fs::write(&executable, r#"#!/bin/sh
case "$1" in
  --version) printf 'Claude test version\n' ;;
  auth) printf '%s\n' '{"loggedIn":false}' ;;
  *) [ "${CLAUDE_CODE_OAUTH_TOKEN:-}" = "__CANARY__" ] || exit 42
     printf 'checked\n' >> "$CLAUDE_CONFIG_DIR/verified"
     printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"CHARIOX_AUTH_OK","usage":{"output_tokens":7}}' ;;
esac
"#.replace("__CANARY__", CANARY)).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var("CHARIOX_CLAUDE_BIN", executable);
    let registry = crate::account_profile::ProviderAccountProfileRegistry::open(
        root.path().join("profiles.json"),
    )
    .unwrap();
    registry
        .migrate_effective_defaults("local", &root.path().join("home"))
        .unwrap();
    let profile = registry
        .create_managed("local", "claude", "Explicit status fixture")
        .unwrap();
    let environment = registry
        .resolve_environment("local", "claude", &profile.profile_id)
        .unwrap();
    let marker = std::path::PathBuf::from(&environment["CLAUDE_CONFIG_DIR"]).join("verified");
    fs::create_dir_all(marker.parent().unwrap()).unwrap();
    assert_ne!(
        claude_auth_status("claude-p", &profile.profile_id, &environment)
            .unwrap()
            .auth_state,
        "authenticated"
    );
    let vault = root.path().join("vault.enc");
    let mut config = DaemonConfig::for_tests();
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::CharioxEncrypted;
    config.user_config.credential_vault.path = vault.display().to_string();
    crate::secret::unlock_chariox_encrypted_vault(
        &vault,
        "test-only-vault-passphrase",
        crate::secret::VaultUnlockLease::Operation,
    )
    .unwrap();
    crate::provider::store_provider_account_credential(
        &config,
        "local",
        "claude",
        &profile.profile_id,
        CANARY,
        false,
    )
    .unwrap();
    registry
        .update_observation(
            "local",
            "claude",
            &profile.profile_id,
            crate::account_profile::ProviderAccountAuthState::Expired,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    assert_eq!(
        observe_provider_auth_status(&registry, "local", "claude-p", &profile.profile_id, &config)
            .unwrap()
            .auth_state,
        "authenticated"
    );
    assert_eq!(
        fs::read_to_string(&marker).unwrap(),
        "checked\n",
        "explicit status must verify the actual Vault secret"
    );
    assert_eq!(
        registry
            .get("local", "claude", &profile.profile_id)
            .unwrap()
            .auth_state,
        crate::account_profile::ProviderAccountAuthState::Authenticated
    );

    crate::secret::lock_chariox_encrypted_vault(&vault).unwrap();
    crate::secret::clear_vault_secret_process_cache().unwrap();
    assert!(observe_provider_auth_status(
        &registry,
        "local",
        "claude-p",
        &profile.profile_id,
        &config
    )
    .is_err());
    assert_eq!(
        refresh_provider_account_profile_with_config(
            &registry,
            "local",
            "claude",
            &profile.profile_id,
            &config
        )
        .unwrap()
        .auth_state,
        crate::account_profile::ProviderAccountAuthState::Authenticated,
        "local Vault locking must retain the saved proof"
    );
    assert_eq!(
        fs::read_to_string(&marker).unwrap(),
        "checked\n",
        "a locked secret cannot be verified or replaced by a cached observation"
    );
    registry
        .mark_logged_out("local", "claude", &profile.profile_id)
        .unwrap();
    assert_eq!(
        refresh_provider_account_profile_with_config(
            &registry,
            "local",
            "claude",
            &profile.profile_id,
            &config
        )
        .unwrap()
        .auth_state,
        crate::account_profile::ProviderAccountAuthState::NotConfigured
    );
    registry
        .require_authenticated(
            "local",
            "claude",
            &profile.profile_id,
            None,
            "MP-11 logout fence",
        )
        .expect_err("automatic refresh cannot revive a logged-out Vault account");
}
