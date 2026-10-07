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
    crate::test_support::isolated_env_test!();
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
    let old: Vec<_> = names.iter().map(std::env::var_os).collect();
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
    let token = format!("sk-ant-oat01-{}", "A".repeat(96));
    let mut stored = false;
    for (mode, replace, expected) in [
        ("ok", false, "succeeded"),
        ("ok", false, "failed"),
        ("ok", true, "succeeded"),
        ("multiple", true, "failed"),
        ("short", true, "failed"),
        ("probe-error", true, "failed"),
        ("error", true, "failed"),
        ("missing", true, "failed"),
        ("cancel", true, "cancelled"),
        ("observation-failure", true, "succeeded"),
    ] {
        let script = format!(
            r#"#!/bin/sh
if [ "$1" = --version ]; then echo 2.2.0; exit 0; fi
if [ "$1" = -p ]; then
  printf probe >> '{probe_marker}'
  [ '{mode}' = probe-error ] && exit 3
  if [ '{mode}' = observation-failure ]; then
    mv '{registry_path}' '{registry_path}.backup'; mkdir '{registry_path}'
  fi
  printf '%s\n' '{{"type":"result","subtype":"success","is_error":false,"duration_api_ms":0,"num_turns":0,"total_cost_usd":0,"usage":{{"input_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":0}},"result":"Current session: 17% used\nCurrent week (all models): 41% used"}}'
  exit 0
fi
[ "$1" = setup-token ] || exit 90
printf '%s\n' 'https://claude.com/cai/oauth/authorize?code=true&client_id=fixture'
printf 'Paste code: '
IFS= read -r response
printf '%s\n' "$response"
[ '{mode}' = cancel ] && sleep 20
[ '{mode}' = missing ] && exit 0
printf 'sk-ant-'; sleep 0.01; printf 'oat01-'
limit=96; [ '{mode}' = short ] && limit=64
i=0; while [ "$i" -lt "$limit" ]; do printf A; i=$((i+1)); done
printf '\n'
if [ '{mode}' = multiple ]; then printf 'sk-ant-oat01-'; i=0; while [ "$i" -lt 96 ]; do printf B; i=$((i+1)); done; printf '\n'; fi
[ '{mode}' = error ] && exit 3
exit 0
"#,
            probe_marker = root.join("probe-marker").display(),
            registry_path = config.account_profile_registry_path().display(),
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
        assert_eq!(
            login.login_kind, "terminal_setup_token",
            "MP-08 --run must reuse StartProviderLogin setup_token"
        );
        let id = login.login_id.unwrap();
        assert!(
            router
                .runtime_state
                .provider_login_process_store()
                .record_for_owner("local", &id)
                .unwrap()
                .setup_token
                .is_some(),
            "MP-08 --run must reuse vt100 capture"
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        let mut sent_input = false;
        let mut saw_real_url = false;
        let probes_before = std::fs::read(root.join("probe-marker"))
            .unwrap_or_default()
            .len();
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
            if String::from_utf8_lossy(&output)
                .contains("https://claude.com/cai/oauth/authorize?code=true")
            {
                saw_real_url = true;
            }
            if status.state != crate::local::ProviderLoginProcessState::Running {
                break status;
            }
            if !sent_input && String::from_utf8_lossy(&output).contains("Paste code:") {
                sent_input = true;
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
        assert!(
            saw_real_url,
            "MP-08 real Claude authorization URL must be projected"
        );
        let probes_after = std::fs::read(root.join("probe-marker"))
            .unwrap_or_default()
            .len();
        assert_eq!(
            probes_after > probes_before,
            matches!(mode, "ok" | "probe-error" | "observation-failure"),
            "MP-08 complete successful captures must run the Claude credential verification turn"
        );
        assert_eq!(
            serde_json::to_value(status.state).unwrap(),
            serde_json::json!(expected),
            "MP-08 fixture mode {mode} replacement {replace}"
        );
        if mode == "observation-failure" {
            let path = config.account_profile_registry_path();
            assert!(path.is_dir(), "the publication fault must actually fire");
            std::fs::remove_dir(&path).unwrap();
            std::fs::rename(format!("{}.backup", path.display()), &path).unwrap();
        }
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

/// A pasted setup token must reach a provider API turn before it is stored,
/// mark the account authenticated, and stay authenticated across auth
/// status checks, refreshes and a kernel restart. A rejected token stores
/// nothing and says how to recover.
#[tokio::test]
async fn pasted_setup_token_is_verified_and_stays_authenticated() {
    pasted_setup_token_fixture("normal").await;
}

#[tokio::test]
async fn pasted_setup_token_succeeds_when_observation_write_fails() {
    pasted_setup_token_fixture("observation-failure").await;
}

#[tokio::test]
async fn legacy_setup_token_is_unknown_until_verified() {
    pasted_setup_token_fixture("legacy").await;
}

async fn pasted_setup_token_fixture(scenario: &str) {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!(
        "chariox-pastetok-{}-{}",
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
    let old: Vec<_> = names.iter().map(std::env::var_os).collect();
    let _cleanup = FixtureCleanup {
        root: root.clone(),
        environment: names.iter().copied().zip(old).collect(),
    };
    std::env::set_var("HOME", root.join("home"));
    std::env::set_var("CHARIOX_HOME", root.join("state"));
    std::env::set_var("CHARIOX_LOG_DIR", root.join("logs"));
    std::env::set_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT", "1");
    let good = format!("sk-ant-oat01-{}", "G".repeat(96));
    let bad = format!("sk-ant-oat01-{}", "B".repeat(96));
    let expired = format!("sk-ant-oat01-{}", "E".repeat(96));
    let unavailable = format!("sk-ant-oat01-{}", "U".repeat(96));
    // Like the real CLI: `auth status` cannot see an environment token here,
    // and `/usage` answers locally for any token. Only a model turn checks it.
    let binary = root.join("claude");
    std::fs::write(
        &binary,
        format!(
            r#"#!/bin/sh
if [ "$1" = --version ]; then echo 2.2.0; exit 0; fi
if [ "$1" = auth ] && [ "$2" = status ]; then echo '{{"loggedIn":false,"authMethod":"none"}}'; exit 1; fi
if [ "$1" = -p ] && [ "$2" = /usage ]; then
  echo '{{"type":"result","subtype":"success","is_error":false,"duration_api_ms":0,"num_turns":0,"total_cost_usd":0,"usage":{{"input_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":0}},"result":"Total cost: $0.0000"}}'
  exit 0
fi
if [ "$1" = -p ]; then
  echo turn >> "$(dirname "$0")/model-turns"
  if [ "$CLAUDE_CODE_OAUTH_TOKEN" = '{good}' ]; then
    echo '{{"type":"result","subtype":"success","is_error":false,"result":"OK"}}'; exit 0
  fi
  if [ "$CLAUDE_CODE_OAUTH_TOKEN" = '{unavailable}' ]; then exit 3; fi
  if [ "$CLAUDE_CODE_OAUTH_TOKEN" = '{expired}' ]; then
    echo '{{"type":"result","is_error":true,"api_error_status":403}}'; exit 1
  fi
  echo '{{"type":"result","subtype":"success","is_error":true,"api_error_status":401,"result":"Failed to authenticate. API Error: 401 OAuth access token is invalid."}}'
  exit 1
fi
exit 90
"#
        ),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::env::set_var("CHARIOX_CLAUDE_BIN", &binary);
    let mut config = DaemonConfig::for_tests().with_session_history_root(root.join("history"));
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::ProcessMemory;
    let mut router = CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(DaemonApp::bootstrap(config.clone()).unwrap())),
        1,
    );
    let profile = router
        .provider_account_profiles
        .create_managed("local", "claude", "headless")
        .unwrap();
    let paste = |value: &str| {
        let request = LocalDaemonRequest::SetProviderAccountCredential(
            crate::local::SetProviderAccountCredentialRequest {
                provider: "claude".into(),
                account_profile: profile.profile_id.clone(),
                value: format!("{value}\n"),
                run: false,
                overwrite: true,
                session_id: None,
                agent_id: None,
            },
        );
        (
            KernelCommand::from_local_request("paste".to_string(), None, None, &request),
            request,
        )
    };
    let auth_state = |router: &CommandRouter| {
        router
            .provider_account_profiles
            .get("local", "claude", &profile.profile_id)
            .unwrap()
            .auth_state
    };

    let (command, request) = paste(&bad);
    let error = router
        .dispatch(command, request)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("rejected the setup token") && error.contains("claude setup-token"),
        "an invalid token needs an actionable error: {error}"
    );
    assert!(!crate::provider::provider_account_credential_registered(
        "local",
        "claude",
        &profile.profile_id
    )
    .unwrap());
    assert_ne!(
        auth_state(&router),
        crate::account_profile::ProviderAccountAuthState::Authenticated
    );

    if scenario == "legacy" {
        crate::provider::store_provider_account_credential(
            &config,
            "local",
            "claude",
            &profile.profile_id,
            &good,
            false,
        )
        .unwrap();
        for refresh in [false, true] {
            if refresh {
                let request: LocalDaemonRequest = serde_json::from_value(serde_json::json!({
                    "RefreshProviderAccountProfile": { "provider": "claude", "account_profile": profile.profile_id }
                })).unwrap();
                let command =
                    KernelCommand::from_local_request("legacy-refresh", None, None, &request);
                router.dispatch(command, request).await.unwrap();
                assert_eq!(
                    auth_state(&router),
                    crate::account_profile::ProviderAccountAuthState::Unknown
                );
            }
            let request = LocalDaemonRequest::GetProviderAuthStatus(
                crate::local::GetProviderAuthStatusRequest {
                    provider: "claude".into(),
                    account_profile: profile.profile_id.clone(),
                },
            );
            let command = KernelCommand::from_local_request("legacy-status", None, None, &request);
            let LocalDaemonResponse::ProviderAuthStatus { status } =
                router.dispatch(command, request).await.unwrap()
            else {
                panic!("expected auth status");
            };
            assert_eq!(status.auth_state, "unknown");
            let hint = status.login_hint.unwrap();
            assert!(
                hint.contains("not been verified") && hint.contains("--replace"),
                "{hint}"
            );
        }
    }
    if scenario == "observation-failure" {
        // The registry is cached in memory; force publication to fail only
        // after CLI verification, independently of the credential registry.
        let path = config.account_profile_registry_path();
        let script = std::fs::read_to_string(&binary).unwrap();
        let fault = format!(
            "mv '{}' '{}'; mkdir '{}'\n",
            path.display(),
            path.with_extension("backup").display(),
            path.display()
        );
        let script = script.replace("echo '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"OK\"}'", &format!("{fault}echo '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"OK\"}}'"));
        std::fs::write(&binary, script).unwrap();
    }
    let (command, request) = paste(&good);
    router
        .dispatch(command, request)
        .await
        .expect("a stored token must succeed despite an observation write failure");
    if scenario == "observation-failure" {
        assert!(
            config.account_profile_registry_path().is_dir(),
            "fault must actually fire"
        );
        assert!(crate::provider::provider_account_credential_registered(
            "local",
            "claude",
            &profile.profile_id
        )
        .unwrap());
        return;
    }
    assert_eq!(
        auth_state(&router),
        crate::account_profile::ProviderAccountAuthState::Authenticated,
        "a verified setup token must authenticate the account"
    );

    // MP-08/MP-11: refusing replacement must precede any billed model turn.
    let turns_before = std::fs::read_to_string(root.join("model-turns")).unwrap();
    for run in [false, true] {
        let request = LocalDaemonRequest::SetProviderAccountCredential(
            crate::local::SetProviderAccountCredentialRequest {
                provider: "claude".into(),
                account_profile: profile.profile_id.clone(),
                value: if run { String::new() } else { bad.clone() },
                run,
                overwrite: false,
                session_id: None,
                agent_id: None,
            },
        );
        let command = KernelCommand::from_local_request("duplicate", None, None, &request);
        let error = router
            .dispatch(command, request)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("already exists") && error.contains("--replace"),
            "{error}"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("model-turns")).unwrap(),
            turns_before,
            "a denied replacement must never invoke Claude"
        );
    }

    for mode in ["claude", "claude-p", "claude-headless"] {
        assert!(
            crate::provider::provider_account_credential_registered(
                "local",
                mode,
                &profile.profile_id
            )
            .unwrap(),
            "the credential must be registered for {mode}"
        );
        let environment = crate::provider::resolve_provider_account_credentials(
            &config,
            "local",
            mode,
            &profile.profile_id,
        )
        .unwrap();
        assert!(
            !environment.is_empty(),
            "{mode} must resolve the account token"
        );
    }
    for (value, message) in [
        (expired.as_str(), "rejected the setup token"),
        (unavailable.as_str(), "Check the network and the Claude CLI"),
    ] {
        let (command, request) = paste(value);
        let error = router
            .dispatch(command, request)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(message),
            "actionable credential failure: {error}"
        );
        assert_eq!(
            auth_state(&router),
            crate::account_profile::ProviderAccountAuthState::Authenticated,
            "a failed replacement must preserve the working account"
        );
    }

    for restart in [false, true] {
        if restart {
            drop(router);
            router = CommandRouter::with_interactive_capacity(
                Arc::new(Mutex::new(
                    crate::test_support::bootstrap_after_test_owner_exit(config.clone()).await,
                )),
                1,
            );
        }
        let request =
            LocalDaemonRequest::GetProviderAuthStatus(crate::local::GetProviderAuthStatusRequest {
                provider: "claude".into(),
                account_profile: profile.profile_id.clone(),
            });
        let command = KernelCommand::from_local_request("status", None, None, &request);
        let LocalDaemonResponse::ProviderAuthStatus { status } =
            router.dispatch(command, request).await.unwrap()
        else {
            panic!("expected auth status")
        };
        assert_eq!(status.auth_state, "authenticated", "restart {restart}");
        let request: LocalDaemonRequest = serde_json::from_value(serde_json::json!({
            "RefreshProviderAccountProfile": {
                "provider": "claude",
                "account_profile": profile.profile_id,
            }
        }))
        .unwrap();
        let command = KernelCommand::from_local_request("refresh", None, None, &request);
        router.dispatch(command, request).await.unwrap();
        assert_eq!(
            auth_state(&router),
            crate::account_profile::ProviderAccountAuthState::Authenticated,
            "status checks must not downgrade a setup-token account (restart {restart})"
        );
    }
}
