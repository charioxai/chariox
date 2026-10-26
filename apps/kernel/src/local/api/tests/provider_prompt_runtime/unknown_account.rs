//! MP-08 / MP-10 / MP-11: first-prompt native status checks (supplementary).
use super::*;
use crate::account_profile::ProviderAccountAuthState;
use std::os::unix::fs::PermissionsExt;

#[test]
fn unknown_account_first_prompt_checks_once_and_continues() {
    crate::test_support::isolated_env_test!();
    first_prompt_account_check(ProviderAccountAuthState::Unknown, true, UsageProbe::Windows);
}

#[test]
fn unknown_account_first_prompt_ignores_failing_optional_usage() {
    crate::test_support::isolated_env_test!();
    for usage in [UsageProbe::ExitFailure, UsageProbe::MissingWindows] {
        first_prompt_account_check(ProviderAccountAuthState::Unknown, true, usage);
    }
}

#[test]
fn unknown_account_first_prompt_refuses_native_logout() {
    crate::test_support::isolated_env_test!();
    first_prompt_account_check(
        ProviderAccountAuthState::Unknown,
        false,
        UsageProbe::Windows,
    );
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
        first_prompt_account_check(state, true, UsageProbe::Windows);
    }
}

#[derive(Clone, Copy)]
enum UsageProbe {
    Windows,
    ExitFailure,
    MissingWindows,
}

struct ClaudePromptFixture {
    harness: LocalRouterTestHarness,
    worktree: crate::test_support::TestWorktree,
    profile_id: String,
    checks: std::path::PathBuf,
}

struct PromptTarget {
    session_id: String,
    agent_id: String,
    attachment_id: String,
}

impl ClaudePromptFixture {
    fn new(
        label: &str,
        initial: ProviderAccountAuthState,
        logged_in: bool,
        usage: UsageProbe,
        status_delay_secs: u32,
    ) -> Self {
        let worktree = crate::test_support::TestWorktree::new(label);
        let root = worktree.path();
        std::env::set_var("CHARIOX_HOME", root.join("state"));
        std::env::set_var("HOME", root.join("home"));
        fs::create_dir_all(root.join("home")).unwrap();
        fs::set_permissions(root.join("home"), fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(root.join("home/.credentials.json"), r#"{"claudeAiOauth":{"accessToken":"synthetic-native-login","refreshToken":"synthetic-native-refresh"}}"#).unwrap();
        let source = root.join("claude-source");
        let binary = root.join("claude");
        let checks = root.join("checks");
        let usage_reply = match usage {
            UsageProbe::Windows => {
                r#"echo '{"type":"result","subtype":"success","is_error":false,"duration_api_ms":0,"num_turns":0,"total_cost_usd":0,"result":"Current session: 1% used","usage":{"input_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":0}}'; exit 0"#
            }
            UsageProbe::ExitFailure => "echo 'usage backend unavailable' >&2; exit 1",
            UsageProbe::MissingWindows => {
                r#"echo '{"type":"result","subtype":"success","is_error":false,"result":"no windows here"}'; exit 0"#
            }
        };
        fs::write(&source, format!(r#"#!/bin/sh
case "$1" in
--version) echo 2.2.0; exit 0;;
auth)
  echo check >> '{}'
  sleep {status_delay_secs}
  echo '{{"loggedIn":{logged_in},"authMethod":"claude.ai"}}'
  {};;
esac
if [ "$2" = /usage ]; then
  {usage_reply}
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
        Self {
            harness,
            worktree,
            profile_id: profile.profile_id,
            checks,
        }
    }

    /// A session whose agent uses the shared profile; `label` keeps worktrees apart.
    fn target(&self, label: &str) -> PromptTarget {
        let (session, agent) = match self
            .harness
            .dispatch(LocalDaemonRequest::CreateSession(if label.is_empty() {
                self.worktree.session_request()
            } else {
                self.harness.fixture_session_request(label)
            }))
            .unwrap()
        {
            LocalDaemonResponse::SessionCreated { session, agent, .. } => (session, agent),
            _ => panic!("expected session"),
        };
        self.harness
            .dispatch(LocalDaemonRequest::UpdateAgentProfile(
                UpdateAgentProfileRequest {
                    session_id: session.id().into(),
                    agent_id: agent.id().into(),
                    provider: Some("claude-headless".into()),
                    account_profile: Some(self.profile_id.clone()),
                    model: Some("claude-haiku-5-5".into()),
                    effort: None,
                    clear_effort: false,
                },
            ))
            .unwrap();
        let attachment = match self
            .harness
            .dispatch(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().into(),
                    client_id: format!("first-prompt-{label}"),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .unwrap()
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("expected attachment"),
        };
        PromptTarget {
            session_id: session.id().into(),
            agent_id: agent.id().into(),
            attachment_id: attachment.id().into(),
        }
    }

    fn submit(&self, target: &PromptTarget) -> Result<LocalDaemonResponse, DaemonError> {
        self.harness
            .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: target.session_id.clone(),
                attachment_id: target.attachment_id.clone(),
                target_agent_id: Some(target.agent_id.clone()),
                prompt: "reply briefly".into(),
                attachments: Vec::new(),
            }))
    }

    fn checks(&self) -> usize {
        fs::read_to_string(&self.checks)
            .unwrap_or_default()
            .lines()
            .count()
    }

    fn observed(&self) -> crate::account_profile::ProviderAccountProfile {
        self.harness.with_app(|app| {
            app.provider_account_profile_registry()
                .get("local", "claude", &self.profile_id)
                .unwrap()
        })
    }

    fn end(&self, target: &PromptTarget) {
        self.harness
            .dispatch(LocalDaemonRequest::EndSession(EndSessionRequest {
                session_id: target.session_id.clone(),
            }))
            .unwrap();
    }
}

#[test]
fn concurrent_first_prompts_on_a_shared_new_profile_run_one_check() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let fixture = ClaudePromptFixture::new(
        "unknown-account-shared",
        ProviderAccountAuthState::Unknown,
        true,
        UsageProbe::Windows,
        1,
    );
    let first = fixture.target("");
    let second = fixture.target("unknown-account-shared-second");
    std::thread::scope(|scope| {
        let a = scope.spawn(|| fixture.submit(&first));
        let b = scope.spawn(|| fixture.submit(&second));
        for result in [a.join().unwrap(), b.join().unwrap()] {
            assert!(matches!(
                result.unwrap(),
                LocalDaemonResponse::PromptSubmitted { .. }
            ));
        }
    });
    assert_eq!(
        fixture.checks(),
        1,
        "MP-08: prompts sharing a new profile share one official status check"
    );
    assert_eq!(
        fixture.observed().auth_state,
        ProviderAccountAuthState::Authenticated
    );
    fixture.end(&first);
    fixture.end(&second);
}

fn first_prompt_account_check(
    initial: ProviderAccountAuthState,
    logged_in: bool,
    usage: UsageProbe,
) {
    let _env = crate::env_lock::lock();
    let fixture =
        ClaudePromptFixture::new("unknown-account-first-prompt", initial, logged_in, usage, 0);
    let target = fixture.target("");
    let first = fixture.submit(&target);
    let succeeds = initial == ProviderAccountAuthState::Authenticated
        || (initial == ProviderAccountAuthState::Unknown && logged_in);
    if succeeds {
        assert!(matches!(
            first.unwrap(),
            LocalDaemonResponse::PromptSubmitted { .. }
        ));
        assert!(matches!(
            fixture.submit(&target).unwrap(),
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
            fixture.submit(&target).unwrap_err(),
            DaemonError::ProviderProfileRefused { .. }
        ));
    }
    assert_eq!(
        fixture.checks(),
        usize::from(initial == ProviderAccountAuthState::Unknown),
        "MP-08: only an unknown profile runs the official status check, once across both prompts"
    );
    assert_eq!(
        fixture.observed().auth_state,
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
    fixture.end(&target);
}
