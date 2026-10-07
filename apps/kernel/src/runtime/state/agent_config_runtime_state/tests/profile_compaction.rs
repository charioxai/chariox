use super::*;
use crate::provider::{
    AgentEndpointMode, AgentExecutionMode, AgentPermissionLevel, LaunchProviderRequest,
    ProviderLaunchResult, ProviderResumeState, ProviderRunState, ProviderRunTokenUsage,
    RuntimeProviderRun,
};

async fn compact_fixture() -> (
    crate::test_support::TestWorktree,
    Arc<Mutex<DaemonApp>>,
    KernelRuntimeState,
    String,
    String,
    RuntimeProviderRun,
) {
    let root = crate::test_support::TestWorktree::new("profile-compact");
    let (app, runtime, session, agent) = agent_config_runtime().await;
    let account = runtime
        .owned
        .provider_account_profiles
        .get(crate::session::DEFAULT_LOCAL_USER_ID, "claude", "default")
        .unwrap()
        .profile_id;
    runtime
        .owned
        .agent_store
        .set_agent_runtime_profile_with_account_profile(
            &agent,
            "claude",
            Some("claude-sonnet-5-5[1m]".into()),
            None,
            Some(account.clone()),
            ProviderResumeState::default(),
        )
        .unwrap();
    let request = LaunchProviderRequest::new(
        &session,
        "claude",
        "claude",
        &account,
        "claude-sonnet-5-5[1m]",
    )
    .with_agent_id(&agent);
    let marker = root.path().join("started");
    let release = root.path().join("release");
    let mut run = RuntimeProviderRun::new("compact-run", &request, ProviderLaunchResult {
        endpoint_mode: AgentEndpointMode::Managed, process_label: "fixture".into(), pty_target: None, pty_program: Some("/bin/sh".into()),
        pty_args: vec!["-c".into(), format!("while IFS= read -r line; do touch '{}'; n=0; while [ ! -f '{}' ] && [ $n -lt 30 ]; do sleep .1; n=$((n+1)); done; printf '%s\\n' '{{\"type\":\"result\",\"subtype\":\"success\"}}'; done", marker.display(), release.display())],
        pty_env: Default::default(), pty_env_remove: vec![], working_directory: None, structured_endpoint: None,
    });
    run.set_execution_config(AgentExecutionMode::Plan, AgentPermissionLevel::Required);
    run.mark_running();
    run.set_usage(ProviderRunTokenUsage {
        context_tokens: Some(320_000),
        ..Default::default()
    });
    runtime
        .owned
        .provider_store
        .write()
        .insert_run_for_test(run.clone());
    runtime
        .owned
        .provider_store
        .initialize_runtime(&run)
        .unwrap();
    runtime.owned.provider_run_projection.update(run.clone());
    (root, app, runtime, session, agent, run)
}

#[tokio::test]
async fn profile_compaction_reserves_local_agent_until_the_profile_is_committed() {
    let (root, app, runtime, session, agent, run) = compact_fixture().await;
    let state = runtime.clone();
    let s = session.clone();
    let a = agent.clone();
    let update = tokio::spawn(async move {
        state
            .update_agent_profile(
                &s,
                &a,
                crate::session::DEFAULT_LOCAL_USER_ID,
                None,
                Some("default".into()),
                Some("haiku".into()),
                None,
            )
            .await
    });
    let marker = root.path().join("started");
    for _ in 0..100 {
        if marker.exists() || update.is_finished() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let started = marker.exists();
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .unwrap()
            .state(),
        ProviderRunState::Running
    );
    let snapshot = runtime.owned.session_store.get_session(&session).unwrap();
    let outcome = runtime
        .owned
        .prompt_state_owner
        .submit_prepared_prompt(
            &snapshot,
            crate::session::PromptQueueItem::new(
                "during-compact",
                "fixture",
                &agent,
                "request during compact",
                crate::session::PromptStatus::Queued,
            ),
            false,
        )
        .unwrap();
    std::fs::write(root.path().join("release"), "").unwrap();
    let result = update.await.unwrap();
    assert!(
        started,
        "profile update should compact before retiring the source run: {result:?}"
    );
    assert!(
        matches!(
            outcome,
            crate::session::PromptSubmissionOutcome::Queued { .. }
        ),
        "prompt must queue during compaction"
    );
    assert_eq!(result.unwrap().model(), Some("haiku"));
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .unwrap()
            .state(),
        ProviderRunState::Ended
    );
    drop(app);
}

#[tokio::test]
async fn profile_compaction_validates_busy_and_unknown_account_before_compacting() {
    let (root, app, runtime, session, agent, run) = compact_fixture().await;
    let invalid = runtime
        .update_agent_profile(
            &session,
            &agent,
            crate::session::DEFAULT_LOCAL_USER_ID,
            None,
            Some("missing-account".into()),
            Some("haiku".into()),
            None,
        )
        .await;
    assert!(invalid.is_err());
    assert!(!root.path().join("started").exists());
    sync_active_prompt(&app, &session, &agent).await;
    let busy = runtime
        .update_agent_profile(
            &session,
            &agent,
            crate::session::DEFAULT_LOCAL_USER_ID,
            None,
            None,
            Some("haiku".into()),
            None,
        )
        .await;
    assert!(busy.is_err());
    assert!(!root.path().join("started").exists());
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .unwrap()
            .state(),
        ProviderRunState::Running
    );
    runtime
        .owned
        .provider_store
        .terminate_run_provider_only(&session, run.id())
        .unwrap();
}

#[tokio::test]
async fn profile_compaction_session_command_accepts_empty_output() {
    let (root, _app, runtime, _session, _agent, run) = compact_fixture().await;
    std::fs::write(root.path().join("release"), "").unwrap();
    let output = runtime
        .run_structured_provider_utility_prompt(
            run.clone(),
            "/compact".into(),
            String::new(),
            std::time::Duration::from_secs(2),
            crate::provider::ProviderUtilityExecutionPolicy::SessionCommand,
        )
        .await
        .unwrap();
    assert_eq!(output, "");
    assert!(root.path().join("started").exists());
    runtime
        .owned
        .provider_store
        .terminate_run_provider_only(run.session_id(), run.id())
        .unwrap();
}

#[tokio::test]
async fn profile_compaction_does_not_run_on_the_home_for_a_remote_agent() {
    let (root, app, runtime, session, agent, run) = compact_fixture().await;
    app.lock()
        .await
        .agents_mut()
        .bind_remote_execution(
            &agent,
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "unavailable-worker".into(),
                worker_machine_id: "worker-machine".into(),
                execution_lease_id: "lease".into(),
                leased_agent_id: "leased-agent".into(),
                active_worker_provider_run_id: Some("worker-run".into()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: None,
            },
        )
        .unwrap();
    let result = runtime
        .update_agent_profile(
            &session,
            &agent,
            crate::session::DEFAULT_LOCAL_USER_ID,
            None,
            None,
            Some("haiku".into()),
            None,
        )
        .await;
    assert!(
        result.is_err(),
        "unavailable worker must leave the home profile unchanged"
    );
    assert!(
        !root.path().join("started").exists(),
        "home must not compact a worker-owned session"
    );
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .unwrap()
            .state(),
        ProviderRunState::Running
    );
    runtime
        .owned
        .provider_store
        .terminate_run_provider_only(&session, run.id())
        .unwrap();
}
