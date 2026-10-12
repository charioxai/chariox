//! MP-08 / MP-10 / MP-11: relaunch/ended Claude context cannot silently downshift.
use super::*;

#[tokio::test]
async fn downshift_after_native_relaunch_with_unknown_context_starts_fresh() {
    check_unknown_native_downshift(false).await;
}

#[tokio::test]
async fn downshift_without_a_live_run_starts_fresh_from_protected_history() {
    check_unknown_native_downshift(true).await;
}

async fn check_unknown_native_downshift(ended: bool) {
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
            "claude",
            old.account_profile(),
            old.model(),
        )
        .with_agent_id(&agent)
        .with_resume_state(ProviderResumeState::from_claude_session_id(
            "large-native-session",
        ));
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
        cold.mark_running();
        assert!(cold.usage().context_tokens.is_none());
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
    assert!(
        target
            .provider_resume_state()
            .provider_session_id("claude")
            .is_none(),
        "unknown source context retained the oversized native session"
    );
    assert!(
        !root.path().join("started").exists(),
        "unknown context must use protected recovery, not an unbounded source utility"
    );
}
