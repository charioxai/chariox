use super::*;
use std::os::unix::fs::PermissionsExt;

#[tokio::test]
async fn copied_provider_logout_does_not_revoke_source_login() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("provider-copy-logout");
    let binary = root.path().join("codex");
    std::fs::write(
        &binary,
        "#!/bin/sh\n[ \"$1\" = logout ] || exit 90\n: > \"$CODEX_HOME/revoked\"\nrm -f \"$CODEX_HOME/auth.json\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::env::set_var("HOME", root.path().join("home"));
    std::env::set_var("CHARIOX_HOME", root.path().join("state"));
    std::env::set_var("CHARIOX_LOG_DIR", root.path().join("logs"));
    std::env::set_var("CHARIOX_CODEX_BIN", &binary);

    let config = DaemonConfig::for_tests().with_session_history_root(root.path().join("history"));
    let app = DaemonApp::bootstrap(config).unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let registry = &router.provider_account_profiles;
    let profile = registry.create_managed("local", "codex", "Copy").unwrap();
    let environment = registry
        .resolve_environment("local", "codex", &profile.profile_id)
        .unwrap();
    let home = std::path::Path::new(&environment["CODEX_HOME"]);
    std::fs::create_dir_all(home).unwrap();
    let credential = home.join("auth.json");
    std::fs::write(&credential, br#"{"tokens":{"refresh_token":"synthetic"}}"#).unwrap();
    let materialization = registry
        .export_materialization("local", "codex", &profile.profile_id)
        .unwrap();
    registry
        .record_credential_copy("local", &materialization, "source-machine")
        .unwrap();

    let request = LocalDaemonRequest::LogoutProvider(crate::local::LogoutProviderRequest {
        provider: "codex".into(),
        account_profile: profile.profile_id.clone(),
        end_everywhere: false,
    });
    let command = KernelCommand::from_local_request("logout", None, None, &request);
    let response = router.dispatch(command, request).await.unwrap();

    assert!(matches!(
        response,
        crate::local::LocalDaemonResponse::ProviderLoggedOut { .. }
    ));
    assert!(
        !home.join("revoked").exists(),
        "provider logout revokes the shared login"
    );
    assert!(
        !credential.exists(),
        "the local credential should be removed"
    );
    assert_eq!(
        registry
            .get("local", "codex", &profile.profile_id)
            .unwrap()
            .auth_state,
        crate::account_profile::ProviderAccountAuthState::NotConfigured
    );
}

#[tokio::test]
async fn copied_provider_logout_removes_the_transferred_vault_login() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("provider-copy-vault-logout");
    std::env::set_var("HOME", root.path().join("home"));
    std::env::set_var("CHARIOX_HOME", root.path().join("state"));
    std::env::set_var("CHARIOX_LOG_DIR", root.path().join("logs"));
    std::env::set_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT", "1");
    let mut config =
        DaemonConfig::for_tests().with_session_history_root(root.path().join("history"));
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::ProcessMemory;
    let app = DaemonApp::bootstrap(config.clone()).unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let registry = &router.provider_account_profiles;
    let profile = registry.create_managed("local", "claude", "Copy").unwrap();
    registry
        .record_transfer_origin(
            "local",
            &crate::account_profile::ManagedContextProviderAccountReceipt {
                context_id: "fixture".into(),
                package_sha256: "fixture".into(),
                materialization_sha256: "fixture".into(),
                provider: "claude".into(),
                profile_id: profile.profile_id.clone(),
            },
            "fixture-source",
            "fixture-profile",
        )
        .unwrap();
    crate::provider::store_provider_account_credential(
        &config,
        "local",
        "claude",
        &profile.profile_id,
        &format!("sk-ant-oat01-{}", "A".repeat(96)),
        false,
    )
    .unwrap();

    let request = LocalDaemonRequest::LogoutProvider(crate::local::LogoutProviderRequest {
        provider: "claude".into(),
        account_profile: profile.profile_id.clone(),
        end_everywhere: false,
    });
    let command = KernelCommand::from_local_request("logout", None, None, &request);
    let response = router.dispatch(command, request).await.unwrap();

    assert!(matches!(
        response,
        crate::local::LocalDaemonResponse::ProviderLoggedOut { .. }
    ));
    assert!(
        !crate::provider::provider_account_credential_registered(
            "local",
            "claude",
            &profile.profile_id,
        )
        .unwrap(),
        "a copied Vault login must no longer be available on this machine"
    );
}

#[tokio::test]
async fn issued_received_and_sole_holder_logout_preserve_explicit_revocation_choice() {
    crate::test_support::isolated_env_test!();
    for (holder, end_everywhere, local_only) in [
        ("issued", false, true),
        ("received", false, true),
        ("sole", false, false),
        ("issued", true, false),
        ("received", true, false),
        ("sole", true, false),
    ] {
        let root = crate::test_support::TestWorktree::new("shared-login-logout-choice");
        let binary = root.path().join("codex");
        std::fs::write(
            &binary,
            "#!/bin/sh\n[ \"$1\" = logout ] || exit 90\n: > \"$CODEX_HOME/revoked\"\nrm -f \"$CODEX_HOME/auth.json\"\n",
        ).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::env::set_var("HOME", root.path().join("home"));
        std::env::set_var("CHARIOX_HOME", root.path().join("state"));
        std::env::set_var("CHARIOX_LOG_DIR", root.path().join("logs"));
        std::env::set_var("CHARIOX_CODEX_BIN", &binary);
        let config = DaemonConfig::for_tests().with_session_history_root(root.path().join("history"));
        let app = DaemonApp::bootstrap(config).unwrap();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
        let registry = &router.provider_account_profiles;
        let profile = registry.create_managed("local", "codex", "Holder").unwrap();
        let environment = registry.resolve_environment("local", "codex", &profile.profile_id).unwrap();
        let home = std::path::PathBuf::from(&environment["CODEX_HOME"]);
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("auth.json"), br#"{"tokens":{"refresh_token":"synthetic"}}"#).unwrap();
        let materialization = registry.export_materialization("local", "codex", &profile.profile_id).unwrap();
        match holder {
            "issued" => registry.record_credential_issued("local", &materialization).unwrap(),
            "received" => registry.record_credential_copy("local", &materialization, "source-machine").unwrap(),
            "sole" => {},
            _ => unreachable!(),
        }
        let request = LocalDaemonRequest::LogoutProvider(crate::local::LogoutProviderRequest {
            provider: "codex".into(),
            account_profile: profile.profile_id.clone(),
            end_everywhere,
        });
        let command = KernelCommand::from_local_request("logout", None, None, &request);
        let response = router.dispatch(command, request).await.unwrap();
        assert_eq!(response, LocalDaemonResponse::ProviderLoggedOut {
            provider: "codex".into(),
            account_profile: profile.profile_id.clone(),
            local_only,
        }, "holder={holder}, end_everywhere={end_everywhere}");
        assert_eq!(home.join("revoked").exists(), !local_only, "holder={holder}, end_everywhere={end_everywhere}");
        assert!(!home.join("auth.json").exists());
        assert_eq!(registry.get("local", "codex", &profile.profile_id).unwrap().auth_state,
            crate::account_profile::ProviderAccountAuthState::NotConfigured);
    }
}
