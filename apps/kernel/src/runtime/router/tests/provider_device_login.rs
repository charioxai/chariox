//! A copied login remains usable while its replacement device sign-in is pending.
use super::*;
use std::os::unix::fs::PermissionsExt;

#[tokio::test]
async fn signed_in_codex_device_login_waits_for_matching_completion() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let root = crate::test_support::TestWorktree::new("codex-device-login");
    std::env::set_var("CHARIOX_HOME", root.path().join("state"));
    std::env::set_var("HOME", root.path().join("home"));
    struct Helpers;
    impl Drop for Helpers {
        fn drop(&mut self) {
            crate::provider::shutdown_codex_account_endpoints();
        }
    }
    let _helpers = Helpers;
    let executable = root.path().join("codex");
    let approval = root.path().join("approve");
    let launches = root.path().join("launches");
    let quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    std::fs::write(&executable, format!(
        "#!/bin/sh\nif [ \"$1\" = --version ]; then echo fixture; exit; fi\nwhile [ \"$#\" -gt 0 ]; do if [ \"$1\" = --listen ]; then shift; export CTXFIX_ACCOUNT_LISTEN=\"$1\"; break; fi; shift; done\nexport CTXFIX_ACCOUNT_LOG={}\nexport CTXFIX_ACCOUNT_LOGIN_COMPLETE={}\nexec {} --exact local::provider_requests::catalog::codex_refresh_tests::account_endpoint_child --ignored --nocapture\n",
        quote(launches.to_str().unwrap()), quote(approval.to_str().unwrap()),
        quote(std::env::current_exe().unwrap().to_str().unwrap())
    )).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::env::set_var("CHARIOX_CODEX_BIN", executable);
    let config = DaemonConfig::for_tests().with_session_history_root(root.path().join("history"));
    let app = Arc::new(Mutex::new(DaemonApp::bootstrap(config).unwrap()));
    let router = CommandRouter::with_interactive_capacity(app, 4);
    let profiles = &router.provider_account_profiles;
    let profile = profiles
        .create_managed("local", "codex", "Interim copy")
        .unwrap();
    let environment = profiles
        .resolve_environment("local", "codex", &profile.profile_id)
        .unwrap();
    let auth = std::path::Path::new(&environment["CODEX_HOME"]).join("auth.json");
    std::fs::write(
        &auth,
        r#"{"tokens":{"access_token":"synthetic-accepted-login-copy"}}"#,
    )
    .unwrap();
    profiles
        .update_observation(
            "local",
            "codex",
            &profile.profile_id,
            crate::account_profile::ProviderAccountAuthState::Authenticated,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    async fn request(router: &CommandRouter, value: serde_json::Value) -> LocalDaemonResponse {
        let request: LocalDaemonRequest = serde_json::from_value(value).unwrap();
        let mut command = KernelCommand::from_local_request(
            format!("device-login-{}", rand::random::<u64>()),
            None,
            None,
            &request,
        );
        command.caller = router
            .local_terminal_caller(
                crate::runtime::command::KernelCommandSource::LocalIpc,
                "device-login",
            )
            .await;
        router.dispatch(command, request).await.unwrap()
    }
    let started = request(
        &router,
        serde_json::json!({"StartProviderLogin":{
            "provider":"codex", "account_profile":profile.profile_id, "method":"device_code"
        }}),
    )
    .await;
    let LocalDaemonResponse::ProviderLoginStarted { login } = started else {
        panic!("expected device sign-in")
    };
    let id = login.login_id.unwrap();
    let status = serde_json::json!({"GetProviderLoginStatus":{"login_id":id}});
    let observed = request(&router, status.clone()).await;
    assert!(matches!(observed, LocalDaemonResponse::ProviderLoginStatus { login }
        if login.state == crate::local::ProviderLoginProcessState::Running),
        "a pre-existing authenticated copy and another attempt's completion cannot complete this login");
    let before = std::fs::read_to_string(&launches).unwrap();
    std::fs::write(
        &auth,
        r#"{"tokens":{"access_token":"synthetic-accepted-login-refreshed"}}"#,
    )
    .unwrap();
    request(
        &router,
        serde_json::json!({"GetProviderAuthStatus":{
            "provider":"codex", "account_profile":profile.profile_id
        }}),
    )
    .await;
    assert_eq!(
        std::fs::read_to_string(&launches).unwrap(),
        before,
        "a native refresh cannot retire the helper holding an unfinished device challenge"
    );
    let observed = request(&router, status.clone()).await;
    assert!(
        matches!(observed, LocalDaemonResponse::ProviderLoginStatus { login }
        if login.state == crate::local::ProviderLoginProcessState::Running)
    );
    std::fs::write(approval, "approve").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if matches!(request(&router, status.clone()).await, LocalDaemonResponse::ProviderLoginStatus { login }
            if login.state == crate::local::ProviderLoginProcessState::Succeeded)
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the matching official completion must finish this sign-in"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(std::fs::read_to_string(auth)
        .unwrap()
        .contains("new-device"));
}
