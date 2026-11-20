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
