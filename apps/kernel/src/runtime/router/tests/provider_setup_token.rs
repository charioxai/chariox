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
    setup_token_fake_cli_fixture(false).await;
}

#[tokio::test]
async fn setup_token_vault_steps_publish_structured_instructions_and_retry_feedback() {
    setup_token_fake_cli_fixture(true).await;
}

async fn setup_token_fake_cli_fixture(vault: bool) {
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
    let vault_path = root.join("vault.enc");
    if vault {
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path = vault_path.display().to_string();
    }
    let app = Arc::new(Mutex::new(DaemonApp::bootstrap(config.clone()).unwrap()));
    let router = CommandRouter::with_interactive_capacity(app.clone(), 1);
    let profile = router
        .provider_account_profiles
        .create_managed("local", "claude", "fixture")
        .unwrap();
    let token = format!("sk-ant-oat01-{}", "A".repeat(96));
    let mut stored = false;
    let scenarios = [
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
    ];
    let vault_scenarios = [("ok", false, "succeeded"), ("ok", true, "succeeded")];
    for &(mode, replace, expected) in if vault {
        &vault_scenarios[..]
    } else {
        &scenarios[..]
    } {
        if vault && stored {
            crate::secret::lock_chariox_encrypted_vault(&vault_path).unwrap();
            crate::secret::clear_vault_secret_process_cache().unwrap();
        }
        let script = format!(
            r#"#!/bin/sh
if [ "$1" = --version ]; then echo 2.2.0; exit 0; fi
if [ "$1" = -p ]; then
  printf probe >> '{probe_marker}'
  [ '{mode}' = probe-error ] && exit 3
  if [ '{mode}' = observation-failure ]; then
    mv '{registry_path}' '{registry_path}.backup'; mkdir '{registry_path}'
  fi
  printf '%s\n' '{{"type":"result","subtype":"success","is_error":false,"duration_api_ms":1,"num_turns":1,"total_cost_usd":0.00001,"usage":{{"input_tokens":1,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":7}},"result":"CHARIOX_AUTH_OK"}}'
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
        let mut command = KernelCommand::from_local_request(
            format!("setup-{mode}-{replace}"),
            None,
            None,
            &request,
        );
        command.caller.connection_class = Some(crate::local::KernelConnectionClass::Terminal);
        let response = router.dispatch(command, request).await;
        if stored && !replace {
            assert!(
                response.is_err_and(|error| error.to_string().contains("Choose Log in")),
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
        let mut vault_step = 0;
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
            if vault
                && status
                    .interaction
                    .as_ref()
                    .and_then(|interaction| interaction.custom_choice())
                    .is_some_and(|choice| choice.id() == "passphrase")
            {
                let (expected, passphrase) = if stored {
                    match vault_step {
                        0 => ("Enter your Chariox Vault passphrase", "wrong-passphrase"),
                        1 => ("passphrase was not accepted", "correct-passphrase"),
                        _ => panic!("unexpected unlock step"),
                    }
                } else {
                    match vault_step {
                        0 => ("Choose a Chariox Vault passphrase", "first-passphrase"),
                        1 => ("again to confirm", "mismatched-passphrase"),
                        2 => ("did not match", "correct-passphrase"),
                        3 => ("again to confirm", "correct-passphrase"),
                        _ => panic!("unexpected create step"),
                    }
                };
                assert!(
                    status
                        .message
                        .as_deref()
                        .is_some_and(|message| message.contains(expected)),
                    "the clean UI needs the current Vault instructions: {expected}"
                );
                assert_eq!(
                    status.interaction.as_ref().unwrap().message(),
                    status.message.as_deref().unwrap()
                );
                vault_step += 1;
                crate::runtime::provider_auth_control::execute_send_provider_login_input_request(
                    &router.runtime_state,
                    "local",
                    crate::local::SendProviderLoginInputRequest {
                        login_id: id.clone(),
                        data_base64: base64::engine::general_purpose::STANDARD
                            .encode(format!("{passphrase}\r")),
                    },
                )
                .await
                .unwrap();
                continue;
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
                    crate::runtime::provider_auth_control::execute_send_provider_login_input_request(&router.runtime_state, "local", crate::local::SendProviderLoginInputRequest { login_id: id.clone(), data_base64: base64::engine::general_purpose::STANDARD.encode(b"fixture-hidden-code#fixture-state\r") }).await.unwrap();
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
        if vault {
            assert_eq!(vault_step, if stored { 2 } else { 4 });
        }
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
        if vault {
            crate::secret::unlock_chariox_encrypted_vault(
                &vault_path,
                "correct-passphrase",
                crate::secret::VaultUnlockLease::KernelShutdown,
            )
            .unwrap();
        }
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
    echo '{{"type":"result","subtype":"success","is_error":false,"num_turns":1,"usage":{{"output_tokens":7}},"result":"CHARIOX_AUTH_OK"}}'; exit 0
  fi
  if [ "$CLAUDE_CODE_OAUTH_TOKEN" = '{unavailable}' ]; then exit 3; fi
  if [ "$CLAUDE_CODE_OAUTH_TOKEN" = '{expired}' ]; then
    echo '{{"type":"result","is_error":true,"api_error_status":403,"result":"Invalid authentication credentials"}}'; exit 1
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
        error.contains("rejected the setup token") && error.contains("Choose Log in"),
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
        let turns_before = std::fs::read_to_string(root.join("model-turns")).unwrap();
        for _ in 0..2 {
            crate::local::provider_requests::refresh_provider_account_profile_with_config(
                &router.provider_account_profiles,
                "local",
                "claude",
                &profile.profile_id,
                &config,
            )
            .unwrap();
            assert_eq!(
                auth_state(&router),
                crate::account_profile::ProviderAccountAuthState::Unknown,
                "automatic refresh must retain unverified legacy state"
            );
        }
        assert_eq!(
            std::fs::read_to_string(root.join("model-turns")).unwrap(),
            turns_before,
            "automatic refresh must not spend a verification turn"
        );
    }
    if scenario == "legacy" {
        let (session, agent) = {
            let mut app = router.app.lock().await;
            crate::app::KernelSessionService::new(&mut app)
                .create_session(crate::session::CreateSessionRequest::new(
                    root.to_string_lossy(),
                    root.to_string_lossy(),
                ))
                .unwrap()
        };
        let before = std::fs::read_to_string(root.join("model-turns"))
            .unwrap()
            .lines()
            .count();
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude-headless",
            &profile.profile_id,
            "claude-sonnet",
        )
        .with_agent_id(agent.id());
        for _ in 0..2 {
            router
                .runtime_state
                .prepare_provider_launch_request_with_vault(request.clone(), "test first use")
                .await
                .unwrap();
        }
        let after = std::fs::read_to_string(root.join("model-turns"))
            .unwrap()
            .lines()
            .count();
        assert_eq!(
            after - before,
            1,
            "MP-08/MP-10/MP-11 unchecked token must be checked once automatically at first use"
        );
        assert_eq!(
            auth_state(&router),
            crate::account_profile::ProviderAccountAuthState::Authenticated
        );
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
        let success_line = script
            .lines()
            .find(|line| line.contains("CHARIOX_AUTH_OK"))
            .expect("valid model-turn fixture");
        let script = script.replace(success_line, &format!("{fault}{success_line}"));
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
            error.contains("already exists") && error.contains("Choose Log in"),
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
        (unavailable.as_str(), "Check the network and Claude CLI"),
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

// MP-08/MP-10/MP-11: supplementary regression seams; never live acceptance.
#[tokio::test]
async fn setup_token_unchecked_rejection_reports_definitive_recovery() {
    first_use_fixture("rejected").await;
}

#[tokio::test]
async fn setup_token_inconclusive_first_use_checks_once_without_asking_for_login() {
    first_use_fixture("network").await;
}

#[tokio::test]
async fn setup_token_previously_expired_account_reports_definitive_recovery() {
    first_use_fixture("expired").await;
}

#[tokio::test]
async fn setup_token_recovery_does_not_hold_an_operation_vault_lease() {
    first_use_fixture("expired-locked").await;
}

#[tokio::test]
async fn setup_token_expired_observation_overrides_cached_first_use_success() {
    first_use_fixture("expired-cached").await;
}

#[tokio::test]
async fn setup_token_account_enrollment_projects_the_shared_oauth_interaction() {
    first_use_fixture("enrollment").await;
}

#[tokio::test]
async fn valid_enrolled_token_wins_over_working_native_login() {
    first_use_fixture("valid-native").await;
}

#[tokio::test]
async fn refused_enrolled_token_uses_native_login_with_renewal_notice() {
    first_use_fixture("stale-native").await;
}

#[tokio::test]
async fn expired_enrollment_uses_native_without_unlocking_a_locked_vault() {
    first_use_fixture("expired-native-locked").await;
}

#[tokio::test]
async fn refused_enrollment_keeps_using_native_after_the_vault_relocks() {
    first_use_fixture("stale-native-relocked").await;
}

#[tokio::test]
async fn setup_token_session_rejected_code_keeps_the_new_challenge_usable() {
    first_use_fixture("enrollment-retry").await;
}

#[tokio::test]
async fn setup_token_session_starts_reattach_without_cancelling_the_driver() {
    first_use_fixture("enrollment-reattach").await;
}

async fn first_use_fixture(mode: &str) {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!(
        "chariox-first-use-{}-{}",
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
    let _cleanup = FixtureCleanup {
        root: root.clone(),
        environment: names
            .iter()
            .copied()
            .map(|name| (name, std::env::var_os(name)))
            .collect(),
    };
    std::env::set_var("HOME", root.join("home"));
    std::env::set_var("CHARIOX_HOME", root.join("state"));
    std::env::set_var("CHARIOX_LOG_DIR", root.join("logs"));
    std::env::set_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT", "1");
    let binary = root.join("claude");
    std::env::set_var("CHARIOX_CLAUDE_BIN", &binary);
    std::fs::write(&binary, format!(r#"#!/bin/sh
if [ "$1" = --version ]; then echo 2.2.0; exit 0; fi
if [ "$1" = -p ]; then
  echo check >> '{root}/checks'
  if [ -z "$CLAUDE_CODE_OAUTH_TOKEN" ]; then
    echo native >> '{root}/native-checks'
    if [ '{mode}' = stale-native ] || [ '{mode}' = valid-native ] || [ '{mode}' = expired-native-locked ] || [ '{mode}' = stale-native-relocked ]; then
      echo '{{"type":"result","subtype":"success","is_error":false,"num_turns":1,"usage":{{"output_tokens":7}},"result":"CHARIOX_AUTH_OK"}}'; exit 0
    fi
  elif [ '{mode}' = valid-native ]; then
    echo '{{"type":"result","subtype":"success","is_error":false,"num_turns":1,"usage":{{"output_tokens":7}},"result":"CHARIOX_AUTH_OK"}}'; exit 0
  fi
  [ '{mode}' = network ] && exit 3
  if [ '{mode}' = expired-cached ] && [ ! -f '{root}/expired' ]; then
    echo '{{"type":"result","subtype":"success","is_error":false,"num_turns":1,"usage":{{"output_tokens":7}},"result":"CHARIOX_AUTH_OK"}}'; exit 0
  fi
  echo '{{"type":"result","is_error":true,"api_error_status":401,"result":"API Error: 401 Invalid authentication credentials"}}'
  exit 1
fi
[ "$1" = setup-token ] || exit 90
echo login >> '{root}/logins'
printf '%s\n' 'https://claude.com/cai/oauth/authorize?code=true&client_id=fixture'
printf 'Paste code here if prompted > '
IFS= read -r response
if [ '{mode}' = enrollment-reattach ]; then
  echo received > '{root}/received'
  IFS= read -r response
fi
if [ '{mode}' = enrollment-retry ]; then
  printf '\033[2J\033[HOAuth error: Request failed with status code 400\r\nPress Enter to retry.'
  IFS= read -r response
  printf '\r\nhttps://claude.com/cai/oauth/authorize?state=retry\r\nPaste code here if prompted > '
  IFS= read -r response
  echo received > '{root}/retried'
  IFS= read -r response
fi
exit 1
"#, root = root.display())).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut config = DaemonConfig::for_tests().with_session_history_root(root.join("history"));
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::ProcessMemory;
    let vault_path = root.join("vault.enc");
    if mode.ends_with("-locked") || mode.ends_with("-relocked") {
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.unlock_policy = if mode == "stale-native-relocked" {
            crate::config::CredentialVaultUnlockPolicy::KernelInit
        } else {
            crate::config::CredentialVaultUnlockPolicy::Always
        };
        config.user_config.credential_vault.path = vault_path.display().to_string();
        crate::secret::unlock_chariox_encrypted_vault(
            &vault_path,
            "test-passphrase",
            crate::secret::VaultUnlockLease::KernelShutdown,
        )
        .unwrap();
    }
    let mut app = DaemonApp::bootstrap(config.clone()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            root.to_string_lossy(),
            root.to_string_lossy(),
        ))
        .unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let profile = router
        .provider_account_profiles
        .create_managed("local", "claude", "fixture")
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
    if mode == "stale-native-relocked" {
        crate::secret::unlock_chariox_encrypted_vault(
            &vault_path,
            "test-passphrase",
            crate::secret::VaultUnlockLease::KernelShutdown,
        )
        .unwrap();
    }
    if mode.ends_with("-locked") {
        crate::secret::lock_chariox_encrypted_vault(&vault_path).unwrap();
        crate::secret::clear_vault_secret_process_cache().unwrap();
    }
    if mode == "expired-cached" {
        let first = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude-headless",
            &profile.profile_id,
            "claude-sonnet",
        )
        .with_agent_id(agent.id());
        router
            .runtime_state
            .prepare_provider_launch_request_with_vault(first, "test first use")
            .await
            .unwrap();
        std::fs::write(root.join("expired"), "expired").unwrap();
    }
    if mode.starts_with("expired") {
        router
            .provider_account_profiles
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
    }
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude-headless",
        &profile.profile_id,
        "claude-sonnet",
    )
    .with_agent_id(agent.id());
    if mode.ends_with("-native")
        || mode == "expired-native-locked"
        || mode == "stale-native-relocked"
    {
        let prepared = tokio::time::timeout(
            Duration::from_secs(5),
            router
                .runtime_state
                .prepare_provider_launch_request_with_vault(
                    request.clone(),
                    "test credential precedence",
                ),
        )
        .await
        .expect("native fallback must not wait for an unlock interaction")
        .unwrap();
        if mode == "stale-native-relocked" {
            crate::secret::lock_chariox_encrypted_vault(&vault_path).unwrap();
            crate::secret::clear_vault_secret_process_cache().unwrap();
            let second = tokio::time::timeout(
                Duration::from_secs(5),
                router
                    .runtime_state
                    .prepare_provider_launch_request_with_vault(
                        request,
                        "test refused enrollment after relock",
                    ),
            )
            .await
            .expect("cached rejection must not request Vault unlock")
            .unwrap();
            assert!(second.provider_credential_env.is_empty());
        }
        if mode.ends_with("-locked") || mode.ends_with("-relocked") {
            assert!(
                !crate::secret::chariox_encrypted_vault_status(&vault_path)
                    .unwrap()
                    .unlocked,
                "known rejection must not ask to unlock the Vault"
            );
            assert!(router
                .app
                .lock()
                .await
                .sessions()
                .get_session(session.id())
                .unwrap()
                .active_interactions()
                .is_empty());
        }
        let notices = router
            .app
            .lock()
            .await
            .terminal_stream_store()
            .notice_records();
        if mode == "valid-native" {
            assert!(
                !prepared.provider_credential_env.is_empty(),
                "enrolled token must win"
            );
            assert!(
                !root.join("native-checks").exists(),
                "native login must not replace a valid enrollment"
            );
            assert!(!notices
                .iter()
                .any(|notice| notice.message.contains("Renew the enrolled token")));
        } else {
            assert!(
                prepared.provider_credential_env.is_empty(),
                "native fallback must omit refused enrollment"
            );
            assert!(root.join("native-checks").exists());
            assert!(notices
                .iter()
                .any(|notice| notice.message.contains("native Claude login")
                    && notice.message.contains("Renew the enrolled token")));
        }
        assert!(
            !root.join("logins").exists(),
            "launch must never wait for a new login"
        );
        return;
    }
    if mode == "network" {
        for _ in 0..2 {
            router
                .runtime_state
                .prepare_provider_launch_request_with_vault(request.clone(), "test first use")
                .await
                .unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(root.join("checks"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        assert!(!root.join("logins").exists());
        return;
    }
    if !mode.starts_with("enrollment") {
        router
            .provider_account_profiles
            .require_authenticated(
                "local",
                "claude-headless",
                &profile.profile_id,
                None,
                "test first-use admission",
            )
            .expect(
                "MP-08/MP-10/MP-11 token observations must reach kernel first-use verification",
            );
    }
    let mut prepared = Box::pin(async {
        if mode.starts_with("enrollment") {
            let start =
                LocalDaemonRequest::StartProviderLogin(crate::local::StartProviderLoginRequest {
                    provider: "claude".into(),
                    account_profile: profile.profile_id.clone(),
                    method: Some("setup_token".into()),
                });
            let mut command = KernelCommand::from_local_request("enroll", None, None, &start);
            command.session_id = Some(session.id().into());
            command.agent_id = Some(agent.id().into());
            router.dispatch(command, start).await?;
            // The dispatched account workflow owns the interaction and cancellation.
            while router
                .runtime_state
                .provider_login_process_store()
                .has_running_for_profile("local", "claude", &profile.profile_id)
            {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(DaemonError::InvalidConfig {
                field: "test cancellation",
                message: "login cancelled",
            })
        } else {
            router
                .runtime_state
                .prepare_provider_launch_request_with_vault(request, "test first use")
                .await
        }
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let interaction = loop {
        tokio::select! {
            result = &mut prepared => {
                assert!(!mode.starts_with("enrollment"), "explicit enrollment must retain its interaction");
                let error = result.expect_err("refused token without native login must fail").to_string();
                assert!(error.contains("no working native Claude login"), "{error}");
                assert!(error.contains("Provider Accounts"), "recovery must be actionable");
                assert!(!root.join("logins").exists(), "launch must not wait for interactive login");
                if mode.ends_with("-locked") {
                    assert!(!crate::secret::chariox_encrypted_vault_status(&vault_path).unwrap().unlocked);
                }
                return;
            },
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
        let current = router
            .app
            .lock()
            .await
            .sessions()
            .get_session(session.id())
            .unwrap();
        if mode.ends_with("-locked") {
            if let Some(interaction) = current
                .active_interactions()
                .iter()
                .find(|interaction| interaction.title() == Some("Unlock Chariox Vault"))
            {
                let choice = interaction.custom_choice().unwrap().id();
                let reply = LocalDaemonRequest::RespondToInteraction(
                    crate::local::RespondToInteractionRequest {
                        session_id: session.id().into(),
                        interaction_id: interaction.id().into(),
                        choice_id: choice.into(),
                        custom_reply: Some("test-passphrase".into()),
                        passkey: None,
                        passkey_remember_minutes: None,
                    },
                );
                let source = crate::runtime::command::KernelCommandSource::LocalCli;
                let mut caller = crate::runtime::command::KernelCaller::for_source(&source)
                    .with_connection_class(crate::local::KernelConnectionClass::Terminal);
                caller.user_id = Some("local".into());
                let command = KernelCommand::from_local_request_with_caller(
                    "unlock", source, caller, None, None, &reply,
                );
                router.dispatch(command, reply).await.unwrap();
                continue;
            }
        }
        if let Some(interaction) = current.active_interactions().iter().find(|interaction| {
            interaction
                .provider_login()
                .is_some_and(|login| login.login.auth_url.is_some())
        }) {
            break interaction.clone();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "shared OAuth link did not appear"
        );
    };
    if mode.ends_with("-locked") {
        assert!(
            !crate::secret::chariox_encrypted_vault_status(&vault_path)
                .unwrap()
                .unlocked,
            "MP-11 operation lease must end before human OAuth consent"
        );
    }
    if mode == "enrollment-retry" {
        router
            .runtime_state
            .resolve_runtime_interaction(
                session.id(),
                interaction.id(),
                "provider-response",
                Some("test-code#test-state"),
            )
            .await
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        loop {
            tokio::select! {
                result = &mut prepared => panic!("driver stopped before the new challenge: {}", result.unwrap_err()),
                _ = tokio::time::sleep(Duration::from_millis(50)) => {}
            }
            let current = router
                .app
                .lock()
                .await
                .sessions()
                .get_session(session.id())
                .unwrap();
            if current.active_interactions().iter().any(|item| {
                item.message().contains("That code was not accepted")
                    && item.provider_login().is_some_and(|login| {
                        login
                            .login
                            .auth_url
                            .as_deref()
                            .is_some_and(|url| url.contains("state=retry"))
                    })
            }) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "new authorization challenge was not projected"
            );
        }
        router
            .runtime_state
            .resolve_runtime_interaction(
                session.id(),
                interaction.id(),
                "provider-response",
                Some("second-code#new-state"),
            )
            .await
            .unwrap();
        loop {
            tokio::select! {
                result = &mut prepared => panic!("driver stopped before the new code reached Claude: {}", result.unwrap_err()),
                _ = tokio::time::sleep(Duration::from_millis(50)) => {}
            }
            let current = router
                .app
                .lock()
                .await
                .sessions()
                .get_session(session.id())
                .unwrap();
            if root.join("retried").exists()
                && current
                    .active_interactions()
                    .iter()
                    .any(|item| item.id() == interaction.id())
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the new challenge did not accept another code"
            );
        }
    }
    if mode == "enrollment-reattach" {
        let start = |adapter: bool| {
            let request = if adapter {
                LocalDaemonRequest::SetProviderAccountCredential(
                    crate::local::SetProviderAccountCredentialRequest {
                        provider: "claude".into(),
                        account_profile: profile.profile_id.clone(),
                        value: String::new(),
                        run: true,
                        overwrite: true,
                        session_id: Some(session.id().into()),
                        agent_id: Some(agent.id().into()),
                    },
                )
            } else {
                LocalDaemonRequest::StartProviderLogin(crate::local::StartProviderLoginRequest {
                    provider: "claude".into(),
                    account_profile: profile.profile_id.clone(),
                    method: Some("setup_token".into()),
                })
            };
            let mut command = KernelCommand::from_local_request("reattach", None, None, &request);
            command.session_id = Some(session.id().into());
            command.agent_id = Some(agent.id().into());
            (command, request)
        };
        let login_id = interaction
            .provider_login()
            .unwrap()
            .login
            .login_id
            .clone()
            .unwrap();
        let (command, request) = start(false);
        let repeated = router.dispatch(command, request).await.unwrap();
        let (one_command, one_request) = start(false);
        let (two_command, two_request) = start(true);
        let (one, two) = tokio::join!(
            router.dispatch(one_command, one_request),
            router.dispatch(two_command, two_request)
        );
        for response in [repeated, one.unwrap(), two.unwrap()] {
            let LocalDaemonResponse::ProviderLoginStarted { login } = response else {
                panic!("expected reattached login")
            };
            assert_eq!(login.login_id.as_deref(), Some(login_id.as_str()));
        }
        tokio::time::sleep(Duration::from_millis(700)).await;
        let current = router
            .app
            .lock()
            .await
            .sessions()
            .get_session(session.id())
            .unwrap();
        assert!(
            current
                .active_interactions()
                .iter()
                .any(|item| item.id() == interaction.id()),
            "reattach destroyed the original interaction"
        );
        assert!(
            router.app.lock().await.pty_mut().has_process(&login_id),
            "reattach killed the provider process"
        );
        router
            .runtime_state
            .resolve_runtime_interaction(
                session.id(),
                interaction.id(),
                "provider-response",
                Some("test-code#test-state"),
            )
            .await
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !root.join("received").exists() {
            tokio::select! { _ = &mut prepared => panic!("reattached driver stopped"), _ = tokio::time::sleep(Duration::from_millis(50)) => {} }
            assert!(
                std::time::Instant::now() < deadline,
                "original provider process did not receive the code"
            );
        }
        loop {
            let current = router
                .app
                .lock()
                .await
                .sessions()
                .get_session(session.id())
                .unwrap();
            if current
                .active_interactions()
                .iter()
                .any(|item| item.id() == interaction.id())
            {
                break;
            }
            tokio::select! { _ = &mut prepared => panic!("reattached driver stopped"), _ = tokio::time::sleep(Duration::from_millis(50)) => {} }
            assert!(
                std::time::Instant::now() < deadline,
                "driver did not continue after sending input"
            );
        }
    }
    assert!(interaction.message().contains("authorization link"));
    assert!(!interaction.message().contains("--replace"));
    assert!(!interaction.message().contains("claude setup-token"));
    router
        .runtime_state
        .resolve_runtime_interaction(session.id(), interaction.id(), "cancel", None)
        .await
        .unwrap();
    assert!(prepared.await.is_err());
    assert_eq!(
        std::fs::read_to_string(root.join("logins"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    let checks = std::fs::read_to_string(root.join("checks"))
        .unwrap_or_default()
        .lines()
        .count();
    assert_eq!(
        checks,
        usize::from(
            !mode.starts_with("enrollment")
                && (!mode.starts_with("expired") || mode == "expired-cached")
        )
    );
    assert!(!router
        .runtime_state
        .provider_login_process_store()
        .has_running_for_profile("local", "claude", &profile.profile_id));
}
