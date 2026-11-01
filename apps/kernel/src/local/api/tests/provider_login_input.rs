//! MP-08 / MP-09 / MP-10 / MP-11: retry through the public provider login requests.
use super::*;
use base64::Engine as _;

#[test]
#[cfg(unix)]
fn claude_login_retries_keep_the_profile_and_process_after_an_incomplete_paste() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let _executable = crate::test_support::TestClaudeExecutable::new();
    let harness = LocalRouterTestHarness::new();
    let create = || match harness
        .dispatch(LocalDaemonRequest::CreateProviderAccountProfile(
            crate::local::api::CreateProviderAccountProfileRequest {
                provider: "claude".into(),
                label: "chariox".into(),
            },
        ))
        .unwrap()
    {
        LocalDaemonResponse::ProviderAccountProfile { profile } => profile,
        _ => panic!("expected profile"),
    };
    let first = create();
    let start = || match harness
        .dispatch(LocalDaemonRequest::StartProviderLogin(
            crate::local::StartProviderLoginRequest {
                provider: "claude".into(),
                account_profile: first.profile_id.clone(),
                method: Some("terminal".into()),
            },
        ))
        .unwrap()
    {
        LocalDaemonResponse::ProviderLoginStarted { login } => login,
        _ => panic!("expected login"),
    };
    let login = start();
    assert_eq!(create().profile_id, first.profile_id);
    assert_eq!(
        start().login_id,
        login.login_id,
        "reattach the same active attempt"
    );
    let login_id = login.login_id.unwrap();
    let send = |input: &str| {
        harness.dispatch(LocalDaemonRequest::SendProviderLoginInput(
            crate::local::SendProviderLoginInputRequest {
                login_id: login_id.clone(),
                data_base64: base64::engine::general_purpose::STANDARD.encode(input),
            },
        ))
    };
    let error = send("abc123\r").unwrap_err();
    assert_eq!(
        crate::transport::kernel_protocol::map_kernel_error(&error).code,
        "provider_login_code_incomplete"
    );
    let status = harness
        .dispatch(LocalDaemonRequest::GetProviderLoginStatus(
            crate::local::GetProviderLoginStatusRequest {
                login_id: login_id.clone(),
            },
        ))
        .unwrap();
    assert!(
        matches!(status, LocalDaemonResponse::ProviderLoginStatus { login } if login.state == crate::local::ProviderLoginProcessState::Running)
    );
    let sent = send("Your code: abc123#def456 \r\n").unwrap();
    assert!(matches!(
        sent,
        LocalDaemonResponse::ProviderLoginInputSent { byte_count: 14, .. }
    ));
    harness
        .dispatch(LocalDaemonRequest::CancelProviderLogin(
            crate::local::CancelProviderLoginRequest { login_id },
        ))
        .unwrap();
    assert_eq!(
        create().profile_id,
        first.profile_id,
        "cancelled enrollment can reuse its name"
    );
}

#[test]
#[cfg(unix)]
fn simultaneous_claude_login_starts_reattach_one_process() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let _executable = crate::test_support::TestClaudeExecutable::new();
    let harness = LocalRouterTestHarness::new();
    let profile = harness.with_app_mut(|app| {
        app.provider_account_profile_registry()
            .create_managed("local", "claude", "Concurrent")
            .unwrap()
    });
    let barrier = std::sync::Barrier::new(2);
    let start = || {
        barrier.wait();
        harness
            .dispatch(LocalDaemonRequest::StartProviderLogin(
                crate::local::StartProviderLoginRequest {
                    provider: "claude".into(),
                    account_profile: profile.profile_id.clone(),
                    method: Some("terminal".into()),
                },
            ))
            .unwrap()
    };
    let (first, second) = std::thread::scope(|scope| {
        let first = scope.spawn(start);
        let second = scope.spawn(start);
        (first.join().unwrap(), second.join().unwrap())
    });
    let LocalDaemonResponse::ProviderLoginStarted { login: first } = first else {
        panic!("expected login")
    };
    let LocalDaemonResponse::ProviderLoginStarted { login: second } = second else {
        panic!("expected login")
    };
    assert_eq!(first.login_id, second.login_id);
    harness
        .dispatch(LocalDaemonRequest::CancelProviderLogin(
            crate::local::CancelProviderLoginRequest {
                login_id: first.login_id.unwrap(),
            },
        ))
        .unwrap();
}

#[test]
#[cfg(unix)]
fn claude_rejected_code_restarts_the_same_attempt_without_native_status_refresh() {
    use std::os::unix::fs::PermissionsExt;
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let _executable = crate::test_support::TestClaudeExecutable::new();
    let root = crate::test_support::TestWorktree::new("claude-code-retry");
    let executable = root.path().join("claude");
    std::fs::write(
        &executable,
        r#"#!/bin/sh
if [ "$1" = --version ]; then echo 2.2.0; exit 0; fi
if [ "$1" = auth ] && [ "$2" = status ]; then
  echo checked >> "$CLAUDE_CONFIG_DIR/status-checks"
  printf '%s\n' '{"loggedIn":false}'
  exit 1
fi
[ "$1" = auth ] && [ "$2" = login ] || exit 90
attempt=0
[ ! -f "$CLAUDE_CONFIG_DIR/attempts" ] || attempt=$(cat "$CLAUDE_CONFIG_DIR/attempts")
attempt=$((attempt + 1))
printf '%s\n' "$attempt" > "$CLAUDE_CONFIG_DIR/attempts"
printf 'https://claude.com/cai/oauth/authorize?state=attempt-%s\n' "$attempt"
printf 'Paste code here if prompted > '
IFS= read -r response
case "$response" in environment*) echo 'Network is unavailable'; exit 2 ;; esac
printf 'Login failed: Request failed with status code 400\n'
exit 1
"#,
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::env::set_var("CHARIOX_CLAUDE_BIN", &executable);
    let harness = LocalRouterTestHarness::new();
    let profile = harness.with_app_mut(|app| {
        app.provider_account_profile_registry()
            .create_managed("local", "claude", "Retry")
            .unwrap()
    });
    let environment = harness.with_app(|app| {
        app.provider_account_profile_registry()
            .resolve_environment("local", "claude", &profile.profile_id)
            .unwrap()
    });
    let config_dir = std::path::PathBuf::from(&environment["CLAUDE_CONFIG_DIR"]);
    let response = harness
        .dispatch(LocalDaemonRequest::StartProviderLogin(
            crate::local::StartProviderLoginRequest {
                provider: "claude".into(),
                account_profile: profile.profile_id.clone(),
                method: Some("terminal".into()),
            },
        ))
        .unwrap();
    let LocalDaemonResponse::ProviderLoginStarted { login } = response else {
        panic!("expected login")
    };
    let login_id = login.login_id.unwrap();
    let status = || match harness
        .dispatch(LocalDaemonRequest::GetProviderLoginStatus(
            crate::local::GetProviderLoginStatusRequest {
                login_id: login_id.clone(),
            },
        ))
        .unwrap()
    {
        LocalDaemonResponse::ProviderLoginStatus { login } => login,
        _ => panic!("expected status"),
    };
    let wait = |ready: &dyn Fn(&crate::local::ProviderLoginStatus) -> bool| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let current = status();
            if ready(&current) {
                break current;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Claude callback retry did not settle"
            );
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
    };
    let text = |status: &crate::local::ProviderLoginStatus| {
        String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(&status.terminal_output_base64)
                .unwrap(),
        )
        .unwrap()
    };
    let first = wait(&|status| text(status).contains("state=attempt-1"));
    let send = |value: &str| {
        harness
            .dispatch(LocalDaemonRequest::SendProviderLoginInput(
                crate::local::SendProviderLoginInputRequest {
                    login_id: login_id.clone(),
                    data_base64: base64::engine::general_purpose::STANDARD.encode(value),
                },
            ))
            .unwrap()
    };
    send("abc123#def456\r");
    let retry =
        wait(&|status| status.message.is_some() && text(status).contains("state=attempt-2"));
    assert_eq!(
        retry.state,
        crate::local::ProviderLoginProcessState::Running
    );
    assert_eq!(retry.login_id, login_id);
    assert_eq!(
        retry.started_at_ms, first.started_at_ms,
        "retry must not renew the ten-minute timeout"
    );
    assert!(!text(&retry).contains("state=attempt-1"));
    assert!(!text(&retry).contains("Login failed"));
    assert!(
        !config_dir.join("status-checks").exists(),
        "a rejected callback must not refresh native auth"
    );
    send("environment#state\r");
    let failed = wait(&|status| status.state == crate::local::ProviderLoginProcessState::Failed);
    assert_eq!(
        failed.message.as_deref(),
        Some("Sign-in could not finish. Please try again.")
    );
    assert_eq!(
        std::fs::read_to_string(config_dir.join("attempts"))
            .unwrap()
            .trim(),
        "2",
        "environmental errors must not restart the CLI"
    );
    assert!(!config_dir.join("status-checks").exists());
}

#[test]
#[cfg(unix)]
fn claude_reattach_preserves_the_running_method_and_replace_choice() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let _executable = crate::test_support::TestClaudeExecutable::new();
    let harness = LocalRouterTestHarness::new();
    let profile = harness.with_app_mut(|app| {
        app.provider_account_profile_registry()
            .create_managed("local", "claude", "Replacement")
            .unwrap()
    });
    let enroll = || {
        harness
            .dispatch(LocalDaemonRequest::SetProviderAccountCredential(
                crate::local::SetProviderAccountCredentialRequest {
                    session_id: None,
                    agent_id: None,
                    provider: "claude".into(),
                    account_profile: profile.profile_id.clone(),
                    value: String::new(),
                    run: true,
                    overwrite: false,
                },
            ))
            .unwrap()
    };
    let LocalDaemonResponse::ProviderLoginStarted { login: first } = enroll() else {
        panic!("expected login")
    };
    let LocalDaemonResponse::ProviderLoginStarted { login: again } = enroll() else {
        panic!("expected login")
    };
    assert_eq!(first.login_id, again.login_id);
    for method in ["setup_token", "terminal"] {
        let error = harness
            .dispatch(LocalDaemonRequest::StartProviderLogin(
                crate::local::StartProviderLoginRequest {
                    provider: "claude".into(),
                    account_profile: profile.profile_id.clone(),
                    method: Some(method.into()),
                },
            ))
            .unwrap_err();
        assert!(error.to_string().contains(if method == "setup_token" {
            "different replacement choice"
        } else {
            "running login"
        }));
    }
    harness
        .dispatch(LocalDaemonRequest::CancelProviderLogin(
            crate::local::CancelProviderLoginRequest {
                login_id: first.login_id.unwrap(),
            },
        ))
        .unwrap();
}

#[test]
#[cfg(unix)]
fn claude_login_interaction_reprompts_after_an_incomplete_callback() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let _executable = crate::test_support::TestClaudeExecutable::new();
    let root = crate::test_support::TestWorktree::new("claude-interaction-retry");
    let executable = root.path().join("claude");
    std::fs::write(
        &executable,
        r#"#!/bin/sh
if [ "$1" = --version ]; then echo 2.2.0; exit 0; fi
printf 'https://claude.com/cai/oauth/authorize?state=interaction\r\nPaste code here if prompted > '
while IFS= read -r line; do :; done
"#,
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::env::set_var("CHARIOX_CLAUDE_BIN", executable);
    let harness = LocalRouterTestHarness::new();
    let LocalDaemonResponse::SessionCreated { session, agent, .. } = harness
        .dispatch(LocalDaemonRequest::CreateSession(root.session_request()))
        .unwrap()
    else {
        panic!("expected session")
    };
    let profile = harness.with_app_mut(|app| {
        app.provider_account_profile_registry()
            .create_managed("local", "claude", "Interaction")
            .unwrap()
    });
    let response = harness
        .dispatch(LocalDaemonRequest::SetProviderAccountCredential(
            crate::local::SetProviderAccountCredentialRequest {
                session_id: Some(session.id().into()),
                agent_id: Some(agent.id().into()),
                provider: "claude".into(),
                account_profile: profile.profile_id.clone(),
                value: String::new(),
                run: true,
                overwrite: false,
            },
        ))
        .unwrap();
    let LocalDaemonResponse::ProviderLoginStarted { login } = response else {
        panic!("expected login")
    };
    let login_id = login.login_id.unwrap();
    let wait = |matches: &dyn Fn(&crate::session::RuntimeInteraction) -> bool| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let found = harness.with_app(|app| {
                app.sessions()
                    .get_session(session.id())
                    .unwrap()
                    .active_interactions()
                    .iter()
                    .find(|interaction| {
                        interaction.id().starts_with("provider-auth-recovery:")
                            && matches(interaction)
                    })
                    .cloned()
            });
            if let Some(interaction) = found {
                break interaction;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "login interaction did not reprompt"
            );
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
    };
    let first = wait(&|interaction| {
        interaction
            .provider_login()
            .is_some_and(|login| login.login.auth_url.is_some())
    });
    let answer = |interaction: &crate::session::RuntimeInteraction, value: &str| {
        harness
            .dispatch(LocalDaemonRequest::RespondToInteraction(
                crate::local::RespondToInteractionRequest {
                    session_id: session.id().into(),
                    interaction_id: interaction.id().into(),
                    choice_id: interaction.custom_choice().unwrap().id().into(),
                    custom_reply: Some(value.into()),
                    passkey: None,
                    passkey_remember_minutes: None,
                },
            ))
            .unwrap()
    };
    answer(&first, "abc123");
    let retry = wait(&|interaction| interaction.message().contains("incomplete"));
    assert_eq!(retry.id(), first.id());
    answer(&retry, "abc123#def456");
    wait(&|interaction| !interaction.message().contains("incomplete"));
    let response = harness
        .dispatch(LocalDaemonRequest::GetProviderLoginStatus(
            crate::local::GetProviderLoginStatusRequest {
                login_id: login_id.clone(),
            },
        ))
        .unwrap();
    assert!(
        matches!(response, LocalDaemonResponse::ProviderLoginStatus { login } if login.state == crate::local::ProviderLoginProcessState::Running && login.message.is_none())
    );
    harness
        .dispatch(LocalDaemonRequest::CancelProviderLogin(
            crate::local::CancelProviderLoginRequest { login_id },
        ))
        .unwrap();
}

#[test]
#[cfg(unix)]
fn claude_setup_token_auto_retries_rejected_code_and_preserves_menu_input() {
    use std::os::unix::fs::PermissionsExt;
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let _executable = crate::test_support::TestClaudeExecutable::new();
    let root = crate::test_support::TestWorktree::new("claude-setup-code-retry");
    let executable = root.path().join("claude");
    std::fs::write(&executable, r#"#!/bin/sh
if [ "$1" = --version ]; then echo 2.2.0; exit 0; fi
[ "$1" = setup-token ] || exit 90
printf 'started\n' >> "$CLAUDE_CONFIG_DIR/starts"
printf 'Choose theme: '
IFS= read -r theme
[ "$theme" = 1 ] || exit 91
attempt=1
while :; do
  printf '\033[2J\033[Hhttps://claude.com/cai/oauth/authorize?state=attempt-%s\r\nPaste code here if prompted > ' "$attempt"
  IFS= read -r response
  printf '\r\nOAuth error: Request failed with status code 400\r\nPress Enter to retry.'
  IFS= read -r retry
  [ -z "$retry" ] || exit 92
  attempt=$((attempt + 1))
done
"#).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::env::set_var("CHARIOX_CLAUDE_BIN", executable);
    let harness = LocalRouterTestHarness::new();
    let profile = harness.with_app_mut(|app| {
        app.provider_account_profile_registry()
            .create_managed("local", "claude", "Setup retry")
            .unwrap()
    });
    let environment = harness.with_app(|app| {
        app.provider_account_profile_registry()
            .resolve_environment("local", "claude", &profile.profile_id)
            .unwrap()
    });
    let config_dir = std::path::PathBuf::from(&environment["CLAUDE_CONFIG_DIR"]);
    let LocalDaemonResponse::ProviderLoginStarted { login } = harness
        .dispatch(LocalDaemonRequest::StartProviderLogin(
            crate::local::StartProviderLoginRequest {
                provider: "claude".into(),
                account_profile: profile.profile_id.clone(),
                method: Some("setup_token".into()),
            },
        ))
        .unwrap()
    else {
        panic!("expected login")
    };
    let login_id = login.login_id.unwrap();
    let wait = |ready: &dyn Fn(&crate::local::ProviderLoginStatus) -> bool| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let LocalDaemonResponse::ProviderLoginStatus { login: status } = harness
                .dispatch(LocalDaemonRequest::GetProviderLoginStatus(
                    crate::local::GetProviderLoginStatusRequest {
                        login_id: login_id.clone(),
                    },
                ))
                .unwrap()
            else {
                panic!("expected status")
            };
            if ready(&status) {
                break status;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "setup-token callback retry did not settle"
            );
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
    };
    let text = |status: &crate::local::ProviderLoginStatus| {
        String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(&status.terminal_output_base64)
                .unwrap(),
        )
        .unwrap()
    };
    let first = wait(&|status| text(status).contains("Choose theme"));
    let send = |value: &str| {
        harness
            .dispatch(LocalDaemonRequest::SendProviderLoginInput(
                crate::local::SendProviderLoginInputRequest {
                    login_id: login_id.clone(),
                    data_base64: base64::engine::general_purpose::STANDARD.encode(value),
                },
            ))
            .unwrap()
    };
    send("1\r");
    wait(&|status| {
        status
            .interaction
            .as_ref()
            .and_then(|interaction| interaction.provider_login())
            .and_then(|login| login.login.auth_url.as_ref())
            .is_some_and(|url| url.contains("state=attempt-1"))
    });
    send("abc123#def456\r");
    let retry = wait(&|status| {
        status.message.is_some()
            && status
                .interaction
                .as_ref()
                .and_then(|interaction| interaction.provider_login())
                .and_then(|login| login.login.auth_url.as_ref())
                .is_some_and(|url| url.contains("state=attempt-2"))
    });
    assert_eq!(
        retry.state,
        crate::local::ProviderLoginProcessState::Running
    );
    assert_eq!(retry.started_at_ms, first.started_at_ms);
    assert!(!text(&retry).contains("state=attempt-1"));
    assert_eq!(
        std::fs::read_to_string(config_dir.join("starts"))
            .unwrap()
            .lines()
            .count(),
        1,
        "setup-token retries within its existing official CLI process"
    );
    harness
        .dispatch(LocalDaemonRequest::CancelProviderLogin(
            crate::local::CancelProviderLoginRequest { login_id },
        ))
        .unwrap();
}
