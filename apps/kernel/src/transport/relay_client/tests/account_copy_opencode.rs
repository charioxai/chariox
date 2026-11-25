//! Native OpenCode copy follows the public source router and encrypted target transaction.
use super::*;
use crate::account_profile::{ProviderAccountAuthState, ProviderAccountProfileRegistry};

pub(super) async fn check_native_copy(
    target: &DaemonConfig,
    source_router: &CommandRouter,
    source_profiles: &ProviderAccountProfileRegistry,
    target_profiles: &ProviderAccountProfileRegistry,
) {
    let canary = r#"{"fixture":{"type":"api","key":"synthetic-opencode-copy"}}"#;
    // The receiving Vault stays locked throughout: an OpenCode login never touches it.
    let target_vault = std::path::PathBuf::from(&target.user_config.credential_vault.path);
    crate::secret::lock_chariox_encrypted_vault(&target_vault).unwrap();
    let profile = source_profiles
        .create_managed("local", "opencode", "OpenCode source")
        .unwrap();
    let source_environment = source_profiles
        .resolve_environment("local", "opencode", &profile.profile_id)
        .unwrap();
    let source_auth = Path::new(&source_environment["XDG_DATA_HOME"]).join("opencode/auth.json");
    std::fs::create_dir_all(source_auth.parent().unwrap()).unwrap();
    std::fs::write(&source_auth, canary).unwrap();
    source_profiles
        .update_observation(
            "local",
            "opencode",
            &profile.profile_id,
            ProviderAccountAuthState::Authenticated,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let request = serde_json::json!({"CopyProviderAccountToKernel": {
        "provider":"opencode", "account_profile":profile.profile_id, "operation_id":"opencode-copy",
        "target_kernel_id":target.daemon_id,
        "target_key_thumbprint":crate::runtime::terminal_pairings::public_key_thumbprint(&target.relay_public_key),
        "replace":false
    }});
    let response = source_request(source_router, request.clone())
        .await
        .unwrap();
    assert!(!response.to_string().contains("synthetic-opencode-copy"));
    let id = response["ProviderAccountCopied"]["receipt"]["account_profile"]
        .as_str()
        .unwrap();
    let target_environment = target_profiles
        .resolve_environment("local", "opencode", id)
        .unwrap();
    assert_ne!(
        source_environment["XDG_DATA_HOME"],
        target_environment["XDG_DATA_HOME"]
    );
    let target_auth = Path::new(&target_environment["XDG_DATA_HOME"]).join("opencode/auth.json");
    assert_eq!(std::fs::read_to_string(&target_auth).unwrap(), canary);
    // Nothing on the receiver can check an OpenCode login, so it is not shown as signed in.
    assert_eq!(
        target_profiles
            .get("local", "opencode", id)
            .unwrap()
            .auth_state,
        ProviderAccountAuthState::Unknown
    );
    assert!(
        crate::provider::resolve_provider_account_credentials(target, "local", "opencode", id)
            .unwrap()
            .is_empty(),
        "an OpenCode login lives only in its own native store"
    );
    assert!(
        !crate::secret::chariox_encrypted_vault_status(&target_vault)
            .unwrap()
            .unlocked,
        "an OpenCode copy does not unlock the receiving Vault"
    );
    // The provider refreshes its login after the copy, and the acknowledgement was lost: the same
    // operation answers from its recorded receipt and leaves the refreshed login alone.
    std::fs::write(&target_auth, "receiver-refreshed-login").unwrap();
    std::fs::write(&source_auth, "source-rotated-login").unwrap();
    assert_eq!(
        source_request(source_router, request.clone())
            .await
            .unwrap(),
        response
    );
    assert_eq!(
        std::fs::read_to_string(&target_auth).unwrap(),
        "receiver-refreshed-login"
    );
    assert!(
        !crate::secret::chariox_encrypted_vault_status(&target_vault)
            .unwrap()
            .unlocked,
        "the replay did not unlock the receiving Vault either"
    );
    // Another operation for the same account does not overwrite the receiver's profile.
    let mut conflict = request.clone();
    conflict["CopyProviderAccountToKernel"]["operation_id"] = "opencode-conflict".into();
    assert_code(
        source_request(source_router, conflict).await,
        "provider_account_copy_conflict",
        "synthetic-opencode-copy",
    );
    assert_eq!(
        std::fs::read_to_string(&target_auth).unwrap(),
        "receiver-refreshed-login"
    );
}
