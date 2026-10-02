//! MP-08/MP-10/MP-11 focused fake official-CLI drill. No provider credentials.
use super::*;
use base64::Engine as _;
use std::os::unix::fs::PermissionsExt;

struct FixtureCleanup {
    root: std::path::PathBuf,
    environment: Vec<(&'static str, Option<std::ffi::OsString>)>,
}
impl Drop for FixtureCleanup {
    fn drop(&mut self) {
        for (name, value) in &self.environment {
            if let Some(value) = value {
                std::env::set_var(name, value);
            } else {
                std::env::remove_var(name);
            }
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn setup_token_fake_cli_stores_privately_and_preserves_replace_policy() {
    let _env = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!(
        "chariox-setuptok-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let names = [
        "HOME",
        "CHARIOX_HOME",
        "CHARIOX_CLAUDE_BIN",
        "CHARIOX_LOG_DIR",
        "CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT",
    ];
    let old: Vec<_> = names.iter().map(|name| std::env::var_os(name)).collect();
    let _cleanup = FixtureCleanup {
        root: root.clone(),
        environment: names.iter().copied().zip(old).collect(),
    };
    std::env::set_var("HOME", root.join("home"));
    std::env::set_var("CHARIOX_HOME", root.join("state"));
    std::env::set_var("CHARIOX_LOG_DIR", root.join("logs"));
    std::env::set_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT", "1");
    let binary = root.join("claude");
    std::env::set_var("CHARIOX_CLAUDE_BIN", &binary);
    let mut config = DaemonConfig::for_tests().with_session_history_root(root.join("history"));
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::ProcessMemory;
    let app = Arc::new(Mutex::new(DaemonApp::bootstrap(config.clone()).unwrap()));
    let router = CommandRouter::with_interactive_capacity(app.clone(), 1);
    let profile = router
        .provider_account_profiles
        .create_managed("local", "claude", "fixture")
        .unwrap();
    let token = format!("sk-ant-oat01-{}", "A".repeat(64));
    let mut stored = false;
    for (mode, replace, expected) in [
        ("ok", false, "succeeded"),
        ("ok", false, "failed"),
        ("ok", true, "succeeded"),
        ("multiple", true, "failed"),
        ("error", true, "failed"),
        ("missing", true, "failed"),
        ("cancel", true, "cancelled"),
    ] {
        let script = format!(
            r#"#!/bin/sh
if [ "$1" = --version ]; then echo 2.2.0; exit 0; fi
[ "$1" = setup-token ] || exit 90
printf '%s\n' 'https://claude.ai/oauth/authorize?client_id=fixture&code=true'
printf 'Paste code: '
IFS= read -r response
printf '%s\n' "$response"
[ '{mode}' = cancel ] && sleep 20
[ '{mode}' = missing ] && exit 0
printf 'sk-ant-'; sleep 0.01; printf 'oat01-'
i=0; while [ "$i" -lt 64 ]; do printf A; i=$((i+1)); done
printf '\n'
if [ '{mode}' = multiple ]; then printf 'sk-ant-oat01-'; i=0; while [ "$i" -lt 64 ]; do printf B; i=$((i+1)); done; printf '\n'; fi
[ '{mode}' = error ] && exit 3
exit 0
"#
        );
        std::fs::write(&binary, script).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let request = LocalDaemonRequest::SetProviderAccountCredential(
            crate::local::SetProviderAccountCredentialRequest {
                provider: "claude".into(),
                account_profile: profile.profile_id.clone(),
                value: String::new(),
                run: true,
                overwrite: replace,
                session_id: None,
                agent_id: None,
            },
        );
        let command = KernelCommand::from_local_request(
            format!("setup-{mode}-{replace}"),
            None,
            None,
            &request,
        );
        let response = router.dispatch(command, request).await;
        if stored && !replace {
            assert!(
                response.is_err_and(|error| error.to_string().contains("--replace")),
                "replacement must be explicit before spawning CLI"
            );
            assert!(!router
                .runtime_state
                .provider_login_process_store()
                .has_running_for_profile("local", "claude", &profile.profile_id));
            continue;
        }
        let response = response.unwrap();
        let LocalDaemonResponse::ProviderLoginStarted { login } = response else {
            panic!("expected login workflow")
        };
        let id = login.login_id.unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        let status = loop {
            let response =
                crate::runtime::provider_auth_control::execute_get_provider_login_status_request(
                    &router.runtime_state,
                    "local",
                    crate::local::GetProviderLoginStatusRequest {
                        login_id: id.clone(),
                    },
                )
                .await
                .unwrap();
            let LocalDaemonResponse::ProviderLoginStatus { login: status } = response else {
                panic!("expected status")
            };
            let output = base64::engine::general_purpose::STANDARD
                .decode(&status.terminal_output_base64)
                .unwrap();
            assert!(
                !output
                    .windows(token.len())
                    .any(|part| part == token.as_bytes()),
                "token in client stream"
            );
            assert!(
                !String::from_utf8_lossy(&output).contains("fixture-hidden-code"),
                "hidden input echoed"
            );
            assert!(
                !serde_json::to_string(&status).unwrap().contains(&token),
                "token in serialized status"
            );
            if status.state != crate::local::ProviderLoginProcessState::Running {
                break status;
            }
            if String::from_utf8_lossy(&output).contains("hidden provider response") {
                if mode == "cancel" {
                    crate::runtime::provider_auth_control::execute_cancel_provider_login_request(
                        &router.runtime_state,
                        "local",
                        crate::local::CancelProviderLoginRequest {
                            login_id: id.clone(),
                        },
                    )
                    .await
                    .unwrap();
                } else {
                    crate::runtime::provider_auth_control::execute_send_provider_login_input_request(&router.runtime_state, "local", crate::local::SendProviderLoginInputRequest { login_id: id.clone(), data_base64: base64::engine::general_purpose::STANDARD.encode(b"fixture-hidden-code\r") }).await.unwrap();
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "fake CLI did not settle"
            );
            tokio::time::sleep(Duration::from_millis(80)).await;
        };
        assert_eq!(
            serde_json::to_value(status.state).unwrap(),
            serde_json::json!(expected)
        );
        if expected == "succeeded" {
            stored = true;
        }
        let diagnostic = app
            .lock()
            .await
            .pty_mut()
            .early_exit_diagnostic(&id)
            .unwrap_or_default();
        assert!(
            !diagnostic.contains(&token),
            "token in retained PTY diagnostic"
        );
        let credentials = crate::provider::resolve_provider_account_credentials(
            &config,
            "local",
            "claude",
            &profile.profile_id,
        )
        .unwrap();
        assert!(
            credentials.iter().any(|(_, value)| value == token),
            "Vault value absent or incorrectly replaced"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !app.lock().await.pty_mut().has_process(&id),
            "owned PTY remained"
        );
    }
    fn scan(path: &std::path::Path, token: &[u8]) {
        for item in std::fs::read_dir(path).unwrap() {
            let path = item.unwrap().path();
            if path.is_dir() {
                scan(&path, token);
            } else if path.is_file() {
                let data = std::fs::read(path).unwrap();
                assert!(
                    !data.windows(token.len()).any(|part| part == token),
                    "token leaked into state/log/history"
                );
            }
        }
    }
    scan(&root, token.as_bytes());
    drop(router);
    drop(app);
}
