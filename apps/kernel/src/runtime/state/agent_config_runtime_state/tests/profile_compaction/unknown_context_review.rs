//! MP-08 / MP-10 / MP-11: relaunch/ended Claude context cannot silently downshift.
use super::*;

#[tokio::test]
async fn downshift_after_native_relaunch_with_unknown_context_starts_fresh() {
    check_unknown_native_downshift(false, "claude", false).await;
}

#[tokio::test]
async fn downshift_without_a_live_run_starts_fresh_from_protected_history() {
    check_unknown_native_downshift(true, "claude", false).await;
}

#[tokio::test]
async fn cold_claude_p_downshift_clears_the_canonical_resume() {
    check_unknown_native_downshift(true, "claude-p", false).await;
}

#[tokio::test]
async fn relaunched_claude_p_downshift_clears_the_canonical_resume() {
    check_unknown_native_downshift(false, "claude-p", false).await;
}

#[tokio::test]
async fn cold_claude_headless_downshift_clears_the_canonical_resume() {
    check_unknown_native_downshift(true, "claude-headless", false).await;
}

#[tokio::test]
async fn starting_claude_headless_downshift_clears_the_canonical_resume() {
    check_unknown_native_downshift(false, "claude-headless", false).await;
}

#[tokio::test]
async fn live_native_tui_downshift_preserves_the_session_without_structured_usage() {
    check_unknown_native_downshift(false, "claude", true).await;
}

#[tokio::test]
async fn r5_review_live_headless_downshift_preserves_the_session_without_structured_usage() {
    check_unknown_native_downshift(false, "claude-headless", true).await;
}

#[tokio::test]
async fn cold_account_context_cap_preserves_an_effectively_equal_window() {
    crate::test_support::isolated_env_test!();
    let previous = std::env::var_os("CLAUDE_CODE_DISABLE_1M_CONTEXT");
    std::env::set_var("CLAUDE_CODE_DISABLE_1M_CONTEXT", "1");
    check_unknown_native_downshift(true, "claude-p", true).await;
    match previous {
        Some(value) => std::env::set_var("CLAUDE_CODE_DISABLE_1M_CONTEXT", value),
        None => std::env::remove_var("CLAUDE_CODE_DISABLE_1M_CONTEXT"),
    }
}

async fn check_unknown_native_downshift(ended: bool, provider: &str, native_live: bool) {
    let (root, _app, runtime, session, agent, mut old) = compact_fixture().await;
    runtime
        .owned
        .agent_store
        .write()
        .set_agent_provider_resume_state(
            &agent,
            ProviderResumeState::from_claude_session_id("large-native-session"),
        )
        .unwrap();
    runtime
        .owned
        .agent_store
        .set_agent_runtime_profile_with_account_profile(
            &agent,
            provider,
            Some(old.model().into()),
            None,
            Some(old.account_profile().into()),
            ProviderResumeState::from_claude_session_id("large-native-session"),
        )
        .unwrap();
    old.set_provider_session_id(Some("large-native-session".into()));
    runtime
        .owned
        .provider_store
        .write()
        .insert_run_for_test(old.clone());
    runtime
        .owned
        .provider_store
        .terminate_run_provider_only(&session, old.id())
        .unwrap();
    if !ended {
        // Same metadata the provider relaunch/recovery creates before its first API call.
        let request = LaunchProviderRequest::new(
            &session,
            "claude",
            provider,
            old.account_profile(),
            old.model(),
        )
        .with_agent_id(&agent)
        .with_resume_state(ProviderResumeState::from_claude_session_id(
            "large-native-session",
        ));
        let request = if native_live && provider != "claude-headless" {
            request.with_client_interface(crate::provider::ProviderClientInterface::NativeTui)
        } else {
            request
        };
        let mut cold = RuntimeProviderRun::new(
            "cold-claude-run",
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "cold-native".into(),
                pty_target: None,
                pty_program: None,
                pty_args: vec![],
                pty_env: Default::default(),
                pty_env_remove: vec![],
                working_directory: None,
                structured_endpoint: None,
            },
        );
        if provider != "claude-headless" || native_live {
            cold.mark_running();
        }
        assert!(cold.usage().context_tokens.is_none());
        if native_live && provider == "claude-headless" {
            assert_eq!(
                cold.client_interface(),
                crate::provider::ProviderClientInterface::Chariox,
                "real headless runs use the Chariox interface"
            );
            assert!(crate::provider::provider_run_uses_claude_native_bridge(
                &cold
            ));
            assert!(!crate::provider::provider_run_uses_structured_prompt_io(
                &cold
            ));
        }
        runtime
            .owned
            .provider_store
            .write()
            .insert_run_for_test(cold);
    }
    let snapshot = runtime.owned.agent_store.get_agent(&agent).unwrap();
    runtime
        .compact_before_window_downshift(&session, &snapshot, None, None, Some("haiku"))
        .await
        .unwrap();
    let target = runtime.owned.agent_store.get_agent(&agent).unwrap();
    assert_eq!(
        target
            .provider_resume_state()
            .provider_session_id("claude")
            .is_none(),
        !native_live,
        "unknown source context retained the oversized native session"
    );
    assert!(
        !root.path().join("started").exists(),
        "unknown context must use protected recovery, not an unbounded source utility"
    );
}
