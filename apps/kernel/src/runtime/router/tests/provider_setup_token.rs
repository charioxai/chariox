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

#[tokio::test]
async fn setup_token_unchecked_vault_observation_wins_over_authenticated_native() {
    pasted_setup_token_fixture("legacy-native").await;
}

#[tokio::test]
async fn setup_token_pre_upgrade_native_observation_does_not_verify_legacy_token() {
    pasted_setup_token_fixture("legacy-native-observed").await;
}

#[tokio::test]
async fn setup_token_pre_upgrade_native_observation_launch_verifies_legacy_token() {
    pasted_setup_token_fixture("legacy-native-observed-direct").await;
}

#[tokio::test]
async fn setup_token_pre_upgrade_native_observation_cannot_gate_vault_usage() {
    pasted_setup_token_fixture("legacy-native-observed-admission").await;
}

#[tokio::test]
async fn setup_token_pre_upgrade_native_observation_cold_prompt_checks_before_spawn() {
    pasted_setup_token_fixture("legacy-native-observed-cold-prompt").await;
}

#[tokio::test]
async fn setup_token_verified_cold_prompt_does_not_repeat_credential_check() {
    pasted_setup_token_fixture("verified-cold-prompt").await;
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
if [ "$1" = auth ] && [ "$2" = status ]; then
  case '{scenario}' in legacy-native*)
    echo '{{"loggedIn":true,"authMethod":"oauth","email":"other@example.test","subscriptionType":"max"}}'; exit 0
  esac
  echo '{{"loggedIn":false,"authMethod":"none"}}'; exit 1
fi
if [ "$1" = -p ] && [ "$2" = /usage ]; then
  echo usage >> "$(dirname "$0")/usage-probes"
  echo '{{"type":"result","subtype":"success","is_error":false,"duration_api_ms":0,"num_turns":0,"total_cost_usd":0,"usage":{{"input_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":0}},"result":"Total cost: $0.0000"}}'
  exit 0
fi
case '{scenario}' in *-cold-prompt)
  # A failed ordinary spawn must still be preceded by the credential check.
  if [ "$1" = -p ] && [ "$2" != 'Reply with OK.' ]; then exit 90; fi
esac
if [ "$1" = -p ]; then
  echo turn >> "$(dirname "$0")/model-turns"
  if [ "$CLAUDE_CODE_OAUTH_TOKEN" = '{good}' ]; then
    echo '{{"type":"result","subtype":"success","is_error":false,"result":"OK"}}'; exit 0
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
        error.contains("rejected the setup token") && error.contains("authorization link"),
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

    if scenario.starts_with("legacy-native-observed") {
        // Before the upgrade the native login ran agents and authenticated
        // this profile; the registered token was an unverified fallback.
        let mut native_usage = profile.usage.clone();
        native_usage.source = "native-login".into();
        native_usage.availability =
            crate::account_profile::ProviderAccountUsageAvailability::Available;
        native_usage.observed_at_ms = Some(crate::session::unix_epoch_ms());
        native_usage
            .meters
            .push(crate::account_profile::ProviderAccountUsageMeter {
                meter_id: "native-limit".into(),
                label: "Native login allowance".into(),
                service_id: None,
                kind: crate::account_profile::ProviderAccountUsageMeterKind::RollingLimit,
                scope: crate::account_profile::ProviderAccountUsageMeterScope::Account,
                used_percent: Some(100.0),
                used: None,
                remaining: None,
                total: None,
                unit: None,
                window_duration_minutes: None,
                resets_at_ms: None,
                state: crate::account_profile::ProviderAccountUsageMeterState::Exhausted,
                source: "native-login".into(),
                observed_at_ms: crate::session::unix_epoch_ms(),
            });
        if scenario.ends_with("-admission") {
            // Claude may use paid credits after its subscription allowance.
            // Both native capacities must be exhausted to exercise admission.
            let mut credits = native_usage.meters.last().unwrap().clone();
            credits.meter_id = "native-credits".into();
            credits.label = "Native login credits".into();
            credits.kind = crate::account_profile::ProviderAccountUsageMeterKind::CreditBalance;
            native_usage.meters.push(credits);
        }
        router
            .provider_account_profiles
            .update_observation(
                "local",
                "claude",
                &profile.profile_id,
                crate::account_profile::ProviderAccountAuthState::Authenticated,
                Some("other@example.test".into()),
                Some("max".into()),
                None,
                Some(native_usage),
            )
            .unwrap();
    }
    if scenario.starts_with("legacy") {
        crate::provider::store_provider_account_credential(
            &config,
            "local",
            "claude",
            &profile.profile_id,
            &good,
            false,
        )
        .unwrap();
        if scenario.ends_with("-admission") {
            router
                .provider_account_profiles
                .require_authenticated(
                    "local",
                    "claude",
                    &profile.profile_id,
                    Some("claude-sonnet"),
                    "test legacy admission",
                )
                .expect(
                    "MP-08/MP-10/MP-11 native usage cannot gate the selected unchecked Vault token",
                );
        }
        if !scenario.ends_with("-direct") && !scenario.ends_with("-admission") {
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
                let command =
                    KernelCommand::from_local_request("legacy-status", None, None, &request);
                let LocalDaemonResponse::ProviderAuthStatus { status } =
                    router.dispatch(command, request).await.unwrap()
                else {
                    panic!("expected auth status");
                };
                assert_eq!(status.auth_state, "unknown");
                assert_eq!(
                    (status.identity_summary, status.plan),
                    (None, None),
                    "MP-08/MP-10/MP-11 native identity is not the unchecked token's"
                );
                assert!(
                    !root.join("usage-probes").exists(),
                    "MP-08/MP-10/MP-11 refresh must not probe the unselected native account"
                );
                assert_eq!(
                    router
                        .provider_account_profiles
                        .get("local", "claude", &profile.profile_id)
                        .unwrap()
                        .usage,
                    profile.usage
                );
                let hint = status.login_hint.unwrap();
                assert!(
                    hint.contains("checked automatically") && !hint.contains("--replace"),
                    "{hint}"
                );
            }
        }
    }
    if scenario == "verified-cold-prompt" {
        let (command, request) = paste(&good);
        router.dispatch(command, request).await.unwrap();
    }
    if scenario.ends_with("-cold-prompt") {
        let (session, agent, attachment) = {
            let mut app = router.app.lock().await;
            let (session, _) = crate::app::KernelSessionService::new(&mut app)
                .create_session(crate::session::CreateSessionRequest::new(
                    root.to_string_lossy(),
                    root.to_string_lossy(),
                ))
                .unwrap();
            let agent = crate::app::KernelSessionService::new(&mut app)
                .spawn_agent(
                    crate::agent::CreateAgentRequest::new(session.id(), "claude-p")
                        .with_model("claude-sonnet")
                        .with_account_profile(profile.profile_id.clone()),
                )
                .unwrap();
            app.focus_agent(session.id(), agent.id()).unwrap();
            let attachment = crate::app::KernelSessionService::new(&mut app)
                .attach(crate::attachment::AttachRequest::new(
                    session.id(),
                    "cold-prompt-client",
                    crate::attachment::ClientCapabilityLevel::FullTerminal,
                ))
                .unwrap();
            (session, agent, attachment)
        };
        let count = || {
            std::fs::read_to_string(root.join("model-turns"))
                .unwrap()
                .lines()
                .count()
        };
        let before = count();
        let request: LocalDaemonRequest = serde_json::from_value(serde_json::json!({
            "SubmitPrompt": {"session_id": session.id(), "attachment_id": attachment.id(),
                "target_agent_id": agent.id(), "prompt": "MP-08 / MP-10 / MP-11 cold ordinary prompt",
                "attachments": []}
        })).unwrap();
        let command = KernelCommand::from_local_request("cold-prompt", None, None, &request);
        // The fake provider deliberately rejects ordinary execution. This check
        // tests credential admission before any spawn, not a fake successful turn.
        let _ = router.dispatch(command, request).await;
        let expected = if scenario.starts_with("legacy") { 1 } else { 0 };
        assert_eq!(
            count() - before,
            expected,
            "MP-08/MP-10/MP-11 real SubmitPrompt checks an unchecked token once before cold spawn"
        );
        assert!(
            crate::provider::provider_account_credential_verification(
                "local",
                "claude",
                &profile.profile_id
            )
            .unwrap()
            .verified
        );
        return;
    }
    if scenario.starts_with("legacy") {
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
        for launch in 0..2 {
            if launch == 1 {
                // The credential's verification survives a new runtime/cache.
                drop(router);
                router = CommandRouter::with_interactive_capacity(
                    Arc::new(Mutex::new(DaemonApp::bootstrap(config.clone()).unwrap())),
                    1,
                );
            }
            let prepared = router
                .runtime_state
                .prepare_provider_launch_request_with_vault(request.clone(), "test first use")
                .await
                .unwrap();
            assert!(
                prepared
                    .provider_credential_env
                    .iter()
                    .any(|(_, value)| value == good),
                "MP-08/MP-10/MP-11 launch must use the checked Vault token"
            );
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
        let verified = router
            .provider_account_profiles
            .get("local", "claude", &profile.profile_id)
            .unwrap();
        assert_eq!(verified.usage, profile.usage);
        assert_eq!(verified.plan, None);
        assert_eq!(
            (verified.auth_state, verified.identity_summary),
            (
                crate::account_profile::ProviderAccountAuthState::Authenticated,
                None
            )
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
            error.contains("already exists") && error.contains("Choose Log in"),
            "{error}"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("model-turns")).unwrap(),
            turns_before,
            "a denied replacement must never invoke Claude"
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
        // Exercise the actual launch/unlock selector in the live runtime and
        // after the old owner exited. Direct resolution bypasses that selector.
        let (launch_session, launch_agent) = {
            let mut app = router.app.lock().await;
            crate::app::KernelSessionService::new(&mut app)
                .create_session(crate::session::CreateSessionRequest::new(
                    root.to_string_lossy(),
                    root.to_string_lossy(),
                ))
                .unwrap()
        };
        for provider in ["claude-p", "claude-headless"] {
            let request = crate::provider::LaunchProviderRequest::new(
                launch_session.id(),
                "claude",
                provider,
                &profile.profile_id,
                "claude-sonnet",
            )
            .with_agent_id(launch_agent.id());
            let prepared = router
                .runtime_state
                .prepare_provider_launch_request_with_vault(
                    request,
                    "verified launch after restart",
                )
                .await
                .unwrap();
            assert!(
                !prepared.provider_credential_env.is_empty(),
                "verified Vault token must reach {provider} (restart={restart})"
            );
        }
    }
}

// MP-08/MP-10/MP-11: supplementary regression seams; never live acceptance.
#[tokio::test]
async fn setup_token_unchecked_rejection_opens_shared_oauth_without_terminal_commands() {
    first_use_fixture("rejected").await;
}

#[tokio::test]
async fn setup_token_inconclusive_first_use_checks_once_without_asking_for_login() {
    first_use_fixture("network").await;
}

#[tokio::test]
async fn setup_token_previously_expired_account_uses_the_same_oauth_interaction() {
    first_use_fixture("expired").await;
}

#[tokio::test]
async fn setup_token_expired_vault_observation_wins_over_authenticated_native() {
    first_use_fixture("expired-native").await;
}

#[tokio::test]
async fn setup_token_first_use_other_account_launch_and_replacement_do_not_wait() {
    first_use_fixture("concurrent").await;
}

#[tokio::test]
async fn setup_token_oauth_does_not_hold_an_operation_vault_lease() {
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
async fn setup_token_enrollment_verification_failure_publishes_retry_notice() {
    first_use_fixture("enrollment-failed").await;
}

#[tokio::test]
async fn setup_token_enrollment_busy_subject_publishes_retry_notice() {
    first_use_fixture("enrollment-busy").await;
}

#[tokio::test]
async fn setup_token_two_login_callers_share_login_and_reload_repaired_account() {
    first_use_fixture("enrollment-two").await;
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
if [ "$1" = auth ] && [ "$2" = status ]; then
  echo '{{"loggedIn":true,"authMethod":"oauth","email":"other@example.test","subscriptionType":"max"}}'; exit 0
fi
if [ "$1" = -p ]; then
  echo check >> '{root}/checks'
  if [ '{mode}' = concurrent ]; then
    if [ "$CLAUDE_CODE_OAUTH_TOKEN" = 'sk-ant-oat01-{stalled}' ]; then
      touch '{root}/checking-a'
      while [ ! -f '{root}/release-a' ]; do sleep 0.05; done
    fi
    echo '{{"type":"result","is_error":false,"result":"OK"}}'; exit 0
  fi
  if [ "$CLAUDE_CODE_OAUTH_TOKEN" = 'sk-ant-oat01-{replacement}' ] && [ '{mode}' = enrollment-two ]; then
    echo '{{"type":"result","is_error":false,"result":"OK"}}'; exit 0
  fi
  [ '{mode}' = network ] && exit 3
  if [ '{mode}' = expired-cached ] && [ ! -f '{root}/expired' ]; then
    echo '{{"type":"result","is_error":false,"result":"OK"}}'; exit 0
  fi
  echo '{{"type":"result","is_error":true,"api_error_status":401,"result":"API Error: 401 Invalid authentication credentials"}}'
  exit 1
fi
[ "$1" = setup-token ] || exit 90
echo login >> '{root}/logins'
printf '%s\n' 'https://claude.com/cai/oauth/authorize?code=true&client_id=fixture'
printf 'Paste code here if prompted > '
IFS= read -r response
if [ '{mode}' = enrollment-failed ] || [ '{mode}' = enrollment-two ]; then
  printf '%s\n' 'sk-ant-oat01-{replacement}'; exit 0
fi
exit 1
"#, root = root.display(), replacement = "B".repeat(96), stalled = "A".repeat(96))).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut config = DaemonConfig::for_tests().with_session_history_root(root.join("history"));
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::ProcessMemory;
    let vault_path = root.join("vault.enc");
    if mode.ends_with("-locked") {
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.unlock_policy =
            crate::config::CredentialVaultUnlockPolicy::Always;
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
        let verification = crate::provider::provider_account_credential_verification(
            "local",
            "claude",
            &profile.profile_id,
        )
        .unwrap();
        assert!(crate::provider::mark_provider_account_credential_verified(
            "local",
            &profile.profile_id,
            verification.revision
        )
        .unwrap());
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
    if mode == "expired-native" {
        for refresh in [false, true] {
            let request: LocalDaemonRequest = if refresh {
                serde_json::from_value(serde_json::json!({"RefreshProviderAccountProfile": {
                    "provider": "claude", "account_profile": profile.profile_id
                }}))
                .unwrap()
            } else {
                LocalDaemonRequest::GetProviderAuthStatus(
                    crate::local::GetProviderAuthStatusRequest {
                        provider: "claude".into(),
                        account_profile: profile.profile_id.clone(),
                    },
                )
            };
            let command = KernelCommand::from_local_request("expired-native", None, None, &request);
            router.dispatch(command, request).await.unwrap();
            let observed = router
                .provider_account_profiles
                .get("local", "claude", &profile.profile_id)
                .unwrap();
            assert_eq!(
                observed.auth_state,
                crate::account_profile::ProviderAccountAuthState::Expired,
                "MP-08/MP-10/MP-11 selected expired Vault token must survive status/refresh"
            );
            assert_eq!(observed.identity_summary, profile.identity_summary);
            assert_eq!(observed.plan, profile.plan);
            assert!(
                !root.join("checks").exists(),
                "refresh must not probe native usage"
            );
        }
    }
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude-headless",
        &profile.profile_id,
        "claude-sonnet",
    )
    .with_agent_id(agent.id());
    if mode == "concurrent" {
        let router = Arc::new(router);
        let runtime = router.runtime_state.clone();
        let same_account = request.clone();
        let a = tokio::spawn(async move {
            runtime
                .prepare_provider_launch_request_with_vault(request, "account A")
                .await
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        while !root.join("checking-a").exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "A never reached verifier"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let runtime = router.runtime_state.clone();
        let a_duplicate = tokio::spawn(async move {
            runtime
                .prepare_provider_launch_request_with_vault(same_account, "account A duplicate")
                .await
        });
        let b = router
            .provider_account_profiles
            .create_managed("local", "claude", "other")
            .unwrap();
        let token = format!("sk-ant-oat01-{}", "B".repeat(96));
        crate::provider::store_provider_account_credential(
            &config,
            "local",
            "claude",
            &b.profile_id,
            &token,
            false,
        )
        .unwrap();
        let b_launch = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude-headless",
            &b.profile_id,
            "claude-sonnet",
        )
        .with_agent_id(agent.id());
        let launched = tokio::time::timeout(
            Duration::from_secs(3),
            router
                .runtime_state
                .prepare_provider_launch_request_with_vault(b_launch, "account B"),
        )
        .await;
        let replacement = LocalDaemonRequest::SetProviderAccountCredential(
            crate::local::SetProviderAccountCredentialRequest {
                provider: "claude".into(),
                account_profile: b.profile_id.clone(),
                value: token,
                run: false,
                overwrite: true,
                session_id: None,
                agent_id: None,
            },
        );
        let command = KernelCommand::from_local_request("replace B", None, None, &replacement);
        let replaced = tokio::time::timeout(
            Duration::from_secs(3),
            router.dispatch(command, replacement),
        )
        .await;
        // Always release and settle owned verifiers before asserting a RED.
        std::fs::write(root.join("release-a"), "release").unwrap();
        tokio::time::timeout(Duration::from_secs(8), a)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(8), a_duplicate)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            launched.is_ok_and(|value| value.is_ok()),
            "MP-08/MP-10/MP-11 B first use waited on A's stalled verifier"
        );
        assert!(
            replaced.is_ok_and(|value| value.is_ok()),
            "MP-08/MP-10/MP-11 B replacement waited on A's stalled verifier"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("checks"))
                .unwrap()
                .lines()
                .count(),
            3,
            "A must be checked once; B first use and replacement each once"
        );
        router.app.lock().await.shutdown_cleanup().unwrap();
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
    let busy_receiver = if mode == "enrollment-busy" {
        Some(
            router
                .runtime_state
                .create_runtime_interaction(
                    session.id(),
                    crate::session::RuntimeInteraction::new(
                        "existing-permission",
                        agent.id(),
                        crate::session::RuntimeInteractionKind::Choice,
                        crate::session::RuntimeInteractionLevel::Warning,
                        Some("Existing permission".into()),
                        "A permission is pending",
                        Vec::new(),
                        None,
                        Some(60),
                        None,
                    ),
                )
                .await
                .unwrap(),
        )
    } else {
        None
    };
    let mut prepared = Box::pin(async {
        if mode.starts_with("enrollment") {
            let start = LocalDaemonRequest::SetProviderAccountCredential(
                crate::local::SetProviderAccountCredentialRequest {
                    provider: "claude".into(),
                    account_profile: profile.profile_id.clone(),
                    value: String::new(),
                    run: true,
                    overwrite: true,
                    session_id: Some(session.id().into()),
                    agent_id: Some(agent.id().into()),
                },
            );
            let command = KernelCommand::from_local_request("enroll", None, None, &start);
            let response = router.dispatch(command, start.clone()).await?;
            if mode == "enrollment-two" {
                let second = router
                    .dispatch(
                        KernelCommand::from_local_request("enroll-second", None, None, &start),
                        start,
                    )
                    .await
                    .expect("another recovery must join the account login instead of failing busy");
                let LocalDaemonResponse::ProviderLoginStarted { login: first } = response else {
                    panic!("first login missing")
                };
                let LocalDaemonResponse::ProviderLoginStarted { login: second } = second else {
                    panic!("second login missing")
                };
                assert_eq!(first.login_id, second.login_id, "one login per account");
            }
            // The dispatched account workflow owns the interaction and cancellation.
            while router
                .runtime_state
                .provider_login_process_store()
                .has_running_for_profile("local", "claude", &profile.profile_id)
            {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            if mode == "enrollment-two" {
                return router
                    .runtime_state
                    .prepare_provider_launch_request_with_vault(request, "repaired account")
                    .await;
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
    if mode == "enrollment-busy" {
        assert!(tokio::time::timeout(Duration::from_secs(8), &mut prepared)
            .await
            .unwrap()
            .is_err());
        wait_for_enrollment_notice(
            &router,
            &session,
            agent.id(),
            "another interaction is pending",
        )
        .await;
        let current = router
            .app
            .lock()
            .await
            .sessions()
            .get_session(session.id())
            .unwrap();
        assert_eq!(
            current
                .active_interaction_for_agent(agent.id())
                .unwrap()
                .id(),
            "existing-permission"
        );
        drop(busy_receiver);
        router.app.lock().await.shutdown_cleanup().unwrap();
        return;
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let interaction = loop {
        tokio::select! {
            result = &mut prepared => panic!("MP-08/MP-10/MP-11 rejected token did not await OAuth: {}", result.is_ok()),
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
    assert!(interaction.message().contains("authorization link"));
    assert!(!interaction.message().contains("--replace"));
    assert!(!interaction.message().contains("claude setup-token"));
    if mode == "enrollment-failed" || mode == "enrollment-two" {
        let choice = interaction.custom_choice().unwrap().id();
        router
            .runtime_state
            .resolve_runtime_interaction(
                session.id(),
                interaction.id(),
                choice,
                Some("synthetic-secret-code"),
            )
            .await
            .unwrap();
        let result = tokio::time::timeout(Duration::from_secs(10), &mut prepared)
            .await
            .unwrap();
        if mode == "enrollment-failed" {
            assert!(result.is_err());
            wait_for_enrollment_notice(&router, &session, agent.id(), "verification failed").await;
        } else {
            let prepared = result.expect("repaired account must prepare a launch");
            let replacement = format!("sk-ant-oat01-{}", "B".repeat(96));
            assert!(prepared
                .provider_credential_env
                .iter()
                .any(|(_, value)| value == replacement));
            for provider in ["claude-p", "claude-headless"] {
                let request = crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "claude",
                    provider,
                    &profile.profile_id,
                    "claude-sonnet",
                )
                .with_agent_id(agent.id());
                let prepared = router
                    .runtime_state
                    .prepare_provider_launch_request_with_vault(request, "second repaired launch")
                    .await
                    .unwrap();
                assert!(prepared
                    .provider_credential_env
                    .iter()
                    .any(|(_, value)| value == replacement));
            }
        }
        assert_eq!(
            std::fs::read_to_string(root.join("logins"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        router.app.lock().await.shutdown_cleanup().unwrap();
        return;
    }
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
            mode != "enrollment" && (!mode.starts_with("expired") || mode == "expired-cached")
        )
    );
    assert!(!router
        .runtime_state
        .provider_login_process_store()
        .has_running_for_profile("local", "claude", &profile.profile_id));
}

async fn wait_for_enrollment_notice(
    router: &CommandRouter,
    session: &crate::session::RuntimeSession,
    agent_id: &str,
    reason: &str,
) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let entries = router
                .app
                .lock()
                .await
                .load_session_history_entries(session, Some(agent_id))
                .unwrap();
            if entries.iter().any(|entry| {
                entry.text.contains(reason) && entry.text.contains("Provider Accounts")
            }) {
                assert!(!entries
                    .iter()
                    .any(|entry| entry.text.contains("synthetic-secret-code")));
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("automatic enrollment must publish a sanitized failure and retry notice");
}

// MP-08/MP-10/MP-11: preserve the existing encrypted home-worker credential path.
#[tokio::test]
async fn setup_token_worker_launch_keeps_home_verified_credential_without_registration() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!(
        "chariox-token-worker-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let names = ["HOME", "CHARIOX_HOME", "CHARIOX_CLAUDE_BIN"];
    let _cleanup = FixtureCleanup {
        root: root.clone(),
        environment: names
            .iter()
            .map(|name| (*name, std::env::var_os(name)))
            .collect(),
    };
    std::env::set_var("HOME", root.join("home"));
    std::env::set_var("CHARIOX_HOME", root.join("state"));
    std::env::set_var("CHARIOX_CLAUDE_BIN", root.join("must-not-verify-on-worker"));
    let config = DaemonConfig::for_tests().with_session_history_root(root.join("history"));
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let profile = app
        .provider_account_profile_registry()
        .create_managed("local", "claude", "worker")
        .unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            root.to_string_lossy(),
            root.to_string_lossy(),
        ))
        .unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let mut credentials = crate::provider::ProviderCredentialEnvironment::default();
    credentials.insert(
        crate::provider::CLAUDE_OAUTH_TOKEN_ENV,
        zeroize::Zeroizing::new("home-verified-test-credential".to_string()),
    );
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude-headless",
        &profile.profile_id,
        "claude-sonnet",
    )
    .with_agent_id(agent.id())
    .with_provider_credential_env(credentials);
    let prepared = router
        .runtime_state
        .prepare_provider_launch_request_with_vault(request, "home-verified worker launch")
        .await
        .unwrap();
    assert_eq!(
        prepared.provider_credential_env.iter().collect::<Vec<_>>(),
        vec![(
            crate::provider::CLAUDE_OAUTH_TOKEN_ENV,
            "home-verified-test-credential"
        )]
    );
}
