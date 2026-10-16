//! MP-08/MP-10/MP-11 credfix: a login copy is never revoked from this machine. Fake official CLI.
use super::*;
use std::os::unix::fs::PermissionsExt;

struct Fixture {
    root: std::path::PathBuf,
    router: CommandRouter,
    profile: crate::account_profile::ProviderAccountProfile,
    auth: std::path::PathBuf,
    marker: std::path::PathBuf,
    _env: crate::env_lock::EnvGuard,
    old: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for (name, value) in &self.old {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn fixture() -> Fixture {
    let env = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!(
        "chariox-credfix-logout-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let names = [
        "HOME",
        "CHARIOX_HOME",
        "CHARIOX_CODEX_BIN",
        "CHARIOX_CLAUDE_BIN",
        "CHARIOX_LOG_DIR",
    ];
    let old = names
        .iter()
        .map(|name| (*name, std::env::var_os(name)))
        .collect();
    std::env::set_var("HOME", root.join("home"));
    std::env::set_var("CHARIOX_HOME", root.join("state"));
    std::env::set_var("CHARIOX_LOG_DIR", root.join("logs"));
    let marker = root.join("provider-calls");
    let binary = root.join("codex");
    // The official CLI revokes the stored login; the fixture records that it was asked to.
    std::fs::write(
        &binary,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nrm -f \"$CODEX_HOME/auth.json\"\n",
            marker.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var("CHARIOX_CODEX_BIN", &binary);
    let config = DaemonConfig::for_tests().with_session_history_root(root.join("history"));
    let app = DaemonApp::bootstrap(config).unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let profile = router
        .provider_account_profiles
        .create_managed("local", "codex", "Copy")
        .unwrap();
    let home = std::path::PathBuf::from(
        &router
            .provider_account_profiles
            .resolve_environment("local", "codex", &profile.profile_id)
            .unwrap()["CODEX_HOME"],
    );
    std::fs::create_dir_all(&home).unwrap();
    let auth = home.join("auth.json");
    std::fs::write(&auth, br#"{"tokens":{"refresh_token":"synthetic"}}"#).unwrap();
    Fixture {
        root,
        router,
        profile,
        auth,
        marker,
        _env: env,
        old,
    }
}

impl Fixture {
    fn mark_as_received_copy(&self) {
        let materialization = self
            .router
            .provider_account_profiles
            .export_materialization("local", "codex", &self.profile.profile_id)
            .unwrap();
        self.router
            .provider_account_profiles
            .record_credential_copy("local", &materialization, "source-machine")
            .unwrap();
    }

    async fn logout(&self, end_everywhere: bool) -> LocalDaemonResponse {
        let request = LocalDaemonRequest::LogoutProvider(crate::local::LogoutProviderRequest {
            provider: "codex".into(),
            account_profile: self.profile.profile_id.clone(),
            end_everywhere,
        });
        let command = KernelCommand::from_local_request("logout", None, None, &request);
        self.router.dispatch(command, request).await.unwrap()
    }

    fn provider_calls(&self) -> String {
        std::fs::read_to_string(&self.marker).unwrap_or_default()
    }
}

#[tokio::test]
async fn shared_codex_logout_removes_only_the_local_login() {
    crate::test_support::isolated_env_test!();
    let fixture = fixture();
    fixture.mark_as_received_copy();
    let response = fixture.logout(false).await;
    assert_eq!(
        fixture.provider_calls(),
        "",
        "`codex logout` revokes the refresh token every other holder of the copy uses"
    );
    assert!(!fixture.auth.exists(), "the local copy is still removed");
    assert!(
        matches!(
            response,
            LocalDaemonResponse::ProviderLoggedOut {
                local_only: true,
                ..
            }
        ),
        "{response:?}"
    );
    let profile = fixture
        .router
        .provider_account_profiles
        .get("local", "codex", &fixture.profile.profile_id)
        .unwrap();
    assert_eq!(
        profile.auth_state,
        crate::account_profile::ProviderAccountAuthState::NotConfigured
    );
}

#[tokio::test]
async fn sole_holder_codex_logout_still_signs_out_at_the_provider() {
    crate::test_support::isolated_env_test!();
    let fixture = fixture();
    let response = fixture.logout(false).await;
    assert_eq!(fixture.provider_calls().trim(), "logout");
    assert!(
        matches!(
            response,
            LocalDaemonResponse::ProviderLoggedOut {
                local_only: false,
                ..
            }
        ),
        "{response:?}"
    );
}

#[tokio::test]
async fn ending_the_session_everywhere_is_an_explicit_choice() {
    crate::test_support::isolated_env_test!();
    let fixture = fixture();
    fixture.mark_as_received_copy();
    let response = fixture.logout(true).await;
    assert_eq!(fixture.provider_calls().trim(), "logout");
    assert!(
        matches!(
            response,
            LocalDaemonResponse::ProviderLoggedOut {
                local_only: false,
                ..
            }
        ),
        "{response:?}"
    );
}

/// Claude's terminal sign-in is the one that rewrites `.credentials.json`; the fake records
/// what it finds there when it starts.
#[tokio::test]
async fn shared_claude_terminal_sign_in_starts_without_the_stored_login() {
    crate::test_support::isolated_env_test!();
    let fixture = fixture();
    let profiles = &fixture.router.provider_account_profiles;
    let claude = profiles.create_managed("local", "claude", "Copy").unwrap();
    let config_dir = std::path::PathBuf::from(
        &profiles
            .resolve_environment("local", "claude", &claude.profile_id)
            .unwrap()["CLAUDE_CONFIG_DIR"],
    );
    std::fs::create_dir_all(&config_dir).unwrap();
    let credentials = config_dir.join(".credentials.json");
    std::fs::write(
        &credentials,
        br#"{"claudeAiOauth":{"refreshToken":"synthetic"}}"#,
    )
    .unwrap();
    let materialization = profiles
        .export_materialization("local", "claude", &claude.profile_id)
        .unwrap();
    profiles
        .record_credential_copy("local", &materialization, "source-machine")
        .unwrap();
    let binary = fixture.root.join("claude");
    let seen = fixture.root.join("claude-saw");
    std::fs::write(
        &binary,
        format!(
            "#!/bin/sh\n{{ printf '%s ' \"$*\"; if [ -e \"$CLAUDE_CONFIG_DIR/.credentials.json\" ]; then echo present; else echo absent; fi; }} >> '{}'\nsleep 3\n",
            seen.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var("CHARIOX_CLAUDE_BIN", &binary);
    let request = LocalDaemonRequest::StartProviderLogin(crate::local::StartProviderLoginRequest {
        provider: "claude".into(),
        account_profile: claude.profile_id.clone(),
        method: Some("terminal".into()),
    });
    let command = KernelCommand::from_local_request("login", None, None, &request);
    fixture.router.dispatch(command, request).await.unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !seen.exists() && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(
        std::fs::read_to_string(&seen).unwrap().trim(),
        "auth login absent",
        "a sign-in that finds the shared login stored would revoke it for every holder"
    );
    assert!(!credentials.exists());
}

#[tokio::test]
async fn shared_claude_logout_removes_only_the_local_login() {
    crate::test_support::isolated_env_test!();
    let fixture = fixture();
    let profiles = &fixture.router.provider_account_profiles;
    let claude = profiles.create_managed("local", "claude", "Copy").unwrap();
    let config_dir = std::path::PathBuf::from(
        &profiles
            .resolve_environment("local", "claude", &claude.profile_id)
            .unwrap()["CLAUDE_CONFIG_DIR"],
    );
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join(".credentials.json"),
        br#"{"claudeAiOauth":{"refreshToken":"synthetic"}}"#,
    )
    .unwrap();
    let materialization = profiles
        .export_materialization("local", "claude", &claude.profile_id)
        .unwrap();
    profiles
        .record_credential_copy("local", &materialization, "source-machine")
        .unwrap();
    let binary = fixture.root.join("claude");
    std::fs::write(
        &binary,
        format!("#!/bin/sh\necho \"$*\" >> '{}'\n", fixture.marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var("CHARIOX_CLAUDE_BIN", &binary);
    let request = LocalDaemonRequest::LogoutProvider(crate::local::LogoutProviderRequest {
        provider: "claude".into(),
        account_profile: claude.profile_id.clone(),
        end_everywhere: false,
    });
    let command = KernelCommand::from_local_request("logout", None, None, &request);
    let response = fixture.router.dispatch(command, request).await.unwrap();
    assert_eq!(fixture.provider_calls(), "");
    assert!(!config_dir.join(".credentials.json").exists());
    assert!(matches!(
        response,
        LocalDaemonResponse::ProviderLoggedOut {
            local_only: true,
            ..
        }
    ));
}
