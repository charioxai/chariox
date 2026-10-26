//! MP-08 / MP-10 / MP-11: first-prompt native status checks (supplementary).
use super::*;
use crate::account_profile::ProviderAccountAuthState;
use std::os::unix::fs::PermissionsExt;

#[test]
fn unknown_account_first_prompt_checks_once_and_continues() {
    crate::test_support::isolated_env_test!();
    first_prompt_account_check(ProviderAccountAuthState::Unknown, true);
}

#[test]
fn unknown_account_first_prompt_refuses_native_logout() {
    crate::test_support::isolated_env_test!();
    first_prompt_account_check(ProviderAccountAuthState::Unknown, false);
}

#[test]
fn known_account_first_prompt_does_not_refresh() {
    crate::test_support::isolated_env_test!();
    for state in [
        ProviderAccountAuthState::Authenticated,
        ProviderAccountAuthState::Expired,
        ProviderAccountAuthState::Error,
        ProviderAccountAuthState::NotConfigured,
    ] {
        first_prompt_account_check(state, true);
    }
}

fn first_prompt_account_check(initial: ProviderAccountAuthState, logged_in: bool) {
    let _env = crate::env_lock::lock();
    let worktree = crate::test_support::TestWorktree::new("unknown-account-first-prompt");
    let root = worktree.path();
    std::env::set_var("CHARIOX_HOME", root.join("state"));
    std::env::set_var("HOME", root.join("home"));
    fs::create_dir_all(root.join("home")).unwrap();
    fs::set_permissions(root.join("home"), fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(root.join("home/.credentials.json"), r#"{"claudeAiOauth":{"accessToken":"synthetic-native-login","refreshToken":"synthetic-native-refresh"}}"#).unwrap();
    let source = root.join("claude-source");
    let binary = root.join("claude");
    let checks = root.join("checks");
    fs::write(&source, format!(r#"#!/bin/sh
case "$1" in
--version) echo 2.2.0; exit 0;;
auth)
  echo check >> '{}'
  echo '{{"loggedIn":{logged_in},"authMethod":"claude.ai"}}'
  {};;
esac
if [ "$2" = /usage ]; then
  echo '{{"type":"result","subtype":"success","is_error":false,"duration_api_ms":0,"num_turns":0,"total_cost_usd":0,"result":"Current session: 1% used","usage":{{"input_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":0}}}}'
  exit 0
fi
while IFS= read -r line; do
  echo '{{"type":"result","subtype":"success","is_error":false,"result":"fixture reply","usage":{{"output_tokens":1}}}}'
done
"#, checks.display(), if logged_in { "exit 0" } else { "exit 1" })).unwrap();
    assert!(std::process::Command::new("install")
        .args(["-m", "700"])
        .arg(&source)
        .arg(&binary)
        .status()
        .unwrap()
        .success());
    std::env::set_var("CHARIOX_CLAUDE_BIN", &binary);
    let harness = LocalRouterTestHarness::new();
    let (session, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .unwrap()
    {
        LocalDaemonResponse::SessionCreated { session, agent, .. } => (session, agent),
        _ => panic!("expected session"),
    };
    let profile = harness.with_app(|app| {
        let registry = app.provider_account_profile_registry();
        let profile = registry
            .link_existing("local", "claude", "new native account", &root.join("home"))
            .unwrap();
        // Isolate first-use authentication from the existing usage-refresh retry.
        let mut usage = crate::account_profile::ProviderAccountUsageSnapshot::unavailable(
            &profile.profile_id,
            "claude",
        );
        usage.observed_at_ms = Some(crate::session::unix_epoch_ms());
        registry
            .update_observation(
                "local",
                "claude",
                &profile.profile_id,
                initial,
                None,
                None,
                None,
                Some(usage),
            )
            .unwrap()
    });
    harness
        .dispatch(LocalDaemonRequest::UpdateAgentProfile(
            UpdateAgentProfileRequest {
                session_id: session.id().into(),
                agent_id: agent.id().into(),
                provider: Some("claude-headless".into()),
                account_profile: Some(profile.profile_id.clone()),
                model: Some("claude-haiku-5-5".into()),
                effort: None,
                clear_effort: false,
            },
        ))
        .unwrap();
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().into(),
                client_id: "first-prompt".into(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .unwrap()
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("expected attachment"),
    };
    let submit = || {
        harness.dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().into(),
            attachment_id: attachment.id().into(),
            target_agent_id: Some(agent.id().into()),
            prompt: "reply briefly".into(),
            attachments: Vec::new(),
        }))
    };
    let first = submit();
    let succeeds = initial == ProviderAccountAuthState::Authenticated
        || (initial == ProviderAccountAuthState::Unknown && logged_in);
    if succeeds {
        assert!(matches!(
            first.unwrap(),
            LocalDaemonResponse::PromptSubmitted { .. }
        ));
        assert!(matches!(
            submit().unwrap(),
            LocalDaemonResponse::PromptSubmitted { .. }
        ));
    } else {
        assert!(matches!(
            first.unwrap_err(),
            DaemonError::ProviderProfileRefused {
                reason: crate::error::ProviderProfileRefusal::Unauthenticated,
                ..
            }
        ));
        assert!(matches!(
            submit().unwrap_err(),
            DaemonError::ProviderProfileRefused { .. }
        ));
    }
    let count = fs::read_to_string(&checks)
        .unwrap_or_default()
        .lines()
        .count();
    assert_eq!(
        count,
        usize::from(initial == ProviderAccountAuthState::Unknown),
        "MP-08: only an unknown profile runs the official status check, once across both prompts"
    );
    let observed = harness.with_app(|app| {
        app.provider_account_profile_registry()
            .get("local", "claude", &profile.profile_id)
            .unwrap()
    });
    assert_eq!(
        observed.auth_state,
        if initial == ProviderAccountAuthState::Unknown {
            if logged_in {
                ProviderAccountAuthState::Authenticated
            } else {
                ProviderAccountAuthState::NotConfigured
            }
        } else {
            initial
        }
    );
    harness
        .dispatch(LocalDaemonRequest::EndSession(EndSessionRequest {
            session_id: session.id().into(),
        }))
        .unwrap();
}
