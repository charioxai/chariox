//! MP-08 / MP-10 / MP-11: native confirmation before profile commit survives.
use super::*;
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, ProviderPromptSignalBatch,
    ProviderResumeState, RuntimeProviderRun,
};

#[tokio::test]
async fn native_confirmation_before_same_account_model_commit_survives() {
    assert_native_confirmation_survives("model").await;
}

#[tokio::test]
async fn native_confirmation_before_same_account_effort_commit_survives() {
    assert_native_confirmation_survives("effort").await;
}

#[tokio::test]
async fn native_confirmation_before_selection_only_account_pin_survives() {
    assert_native_confirmation_survives("pin").await;
}

async fn assert_native_confirmation_survives(change: &str) {
    for initial_session in [None, Some("initial-native-session")] {
        let root = crate::test_support::TestWorktree::new("profile-native-confirmation");
        let config = crate::config::DaemonConfig::for_tests();
        let (app, runtime, session, agent) = agent_config_runtime_in_worktree(
            config.clone(),
            crate::session::DEFAULT_LOCAL_USER_ID,
            root.session_request(),
        )
        .await;
        let owner = crate::session::DEFAULT_LOCAL_USER_ID;
        let account = runtime
            .owned
            .provider_account_profiles
            .get(owner, "claude", "default")
            .unwrap()
            .profile_id;
        runtime
            .owned
            .agent_store
            .set_agent_runtime_profile_with_account_profile(
                &agent,
                "claude-headless",
                Some("haiku".into()),
                None,
                Some("default".into()),
                initial_session
                    .map(ProviderResumeState::from_claude_session_id)
                    .unwrap_or_default(),
            )
            .unwrap();
        let mut request = LaunchProviderRequest::new(
            &session,
            "claude",
            "claude-headless",
            &account,
            "claude-haiku-5-5",
        )
        .with_agent_id(&agent);
        crate::account_profile::bind_provider_account_authority(
            &mut request,
            owner.into(),
            "default".into(),
        );
        let mut run = RuntimeProviderRun::new(
            "current-native-run",
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "metadata-only-profile-native-confirmation".into(),
                pty_target: None,
                pty_program: None,
                pty_args: vec![],
                pty_env: Default::default(),
                pty_env_remove: vec![],
                working_directory: None,
                structured_endpoint: None,
            },
        );
        run.mark_running();
        runtime
            .owned
            .provider_store
            .write()
            .insert_run_for_test(run.clone());
        let confirming_app = Arc::clone(&app);
        let confirming_run = run.clone();
        // The actual profile writer has copied A (or no SID). Now the current
        // native run durably confirms B before that writer can retire it.
        super::super::super::agent_profile_owned_state::AFTER_PROFILE_VALIDATION.with(|gate| {
            *gate.borrow_mut() = Some(Box::new(move || {
                crate::app::persist_resolved_resume_state(
                    &mut confirming_app.try_lock().unwrap(),
                    &confirming_run,
                    &ProviderPromptSignalBatch {
                        resolved_resume_state: Some(ProviderResumeState::from_claude_session_id(
                            "confirmed-native-session",
                        )),
                        ..Default::default()
                    },
                )
                .unwrap();
            }));
        });
        let updated = runtime
            .update_agent_profile(
                &session,
                &agent,
                owner,
                None,
                (change == "pin").then(|| account.clone()),
                (change == "model").then(|| "sonnet".into()),
                (change == "effort").then(|| Some("low".into())),
            )
            .await
            .unwrap();
        let retained_run = runtime
            .owned
            .provider_store
            .get_run_for_agent(&session, &agent);
        assert_eq!(retained_run.is_some(), change == "pin");
        if let Some(retained_run) = retained_run {
            assert_eq!(retained_run.id(), run.id());
        }
        assert_eq!(updated.provider_account_profile(), account);
        assert_eq!(
            updated.model(),
            Some(if change == "model" { "sonnet" } else { "haiku" })
        );
        assert_eq!(updated.effort(), (change == "effort").then_some("low"));
        assert_eq!(
            updated.provider_resume_state().claude_session_id(),
            Some("confirmed-native-session"),
            "{change} restored the stale validation snapshot {initial_session:?}"
        );
        let events = runtime
            .owned
            .durable_state_store
            .load_subject_events(&agent, 100)
            .unwrap();
        let profile_event = events
            .iter()
            .rev()
            .find(|event| event.kind == "agent.updated")
            .unwrap();
        let durable: crate::agent::AgentInstance =
            serde_json::from_value(profile_event.payload["agent"].clone()).unwrap();
        assert_eq!(
            durable.provider_resume_state(),
            updated.provider_resume_state()
        );
        drop(runtime);
        drop(app);
        let restored = DaemonApp::bootstrap(config).unwrap();
        let restored_agent = restored.agents().get_agent(&agent).unwrap();
        assert_eq!(
            restored_agent.provider_resume_state(),
            updated.provider_resume_state()
        );
        assert_eq!(restored_agent.model(), updated.model());
        assert_eq!(restored_agent.effort(), updated.effort());
        assert_eq!(restored_agent.provider_account_profile(), account);
    }
}
