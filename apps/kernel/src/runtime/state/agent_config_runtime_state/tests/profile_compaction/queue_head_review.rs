//! MP-08 / MP-10 / MP-11: actual preparation/expected-head regression, supplementary fixture.
use super::*;

#[tokio::test]
async fn cancelling_preparing_head_promotes_the_surviving_prompt_without_new_submission() {
    let (root, app, mut runtime, session, agent, run) = compact_fixture().await;
    let account = runtime
        .owned
        .agent_store
        .get_agent(&agent)
        .unwrap()
        .provider_account_profile()
        .to_string();
    crate::test_support::authenticate_provider_account(
        &runtime.owned.provider_account_profiles,
        crate::session::DEFAULT_LOCAL_USER_ID,
        "claude",
        &account,
    )
    .unwrap();
    let attachment = runtime
        .with_app_side_effect(|app| {
            crate::app::KernelSessionService::new(app).attach(
                crate::attachment::AttachRequest::new(
                    &session,
                    "preparation-cancel-observer",
                    crate::attachment::ClientCapabilityLevel::FullTerminal,
                ),
            )
        })
        .await
        .unwrap();
    let snapshot = runtime.owned.session_store.get_session(&session).unwrap();
    for (id, text) in [
        ("head-a", "cancel this preparing request"),
        ("head-b", "surviving queued request"),
    ] {
        runtime
            .owned
            .prompt_state_owner
            .submit_prepared_prompt(
                &snapshot,
                crate::session::PromptQueueItem::new(
                    id,
                    attachment.id(),
                    &agent,
                    text,
                    crate::session::PromptStatus::Queued,
                ),
                true,
            )
            .unwrap();
    }
    let (active, queued) = runtime
        .owned
        .prompt_state_owner
        .state_parts(&snapshot, &agent);
    let head_id = queued
        .front()
        .expect("preparation queue head")
        .id()
        .to_string();
    runtime
        .owned
        .mirror_prompt_owner_agent_state(&session, &agent, active, queued)
        .unwrap();
    let probe = Arc::new(tokio::sync::Notify::new());
    runtime.observe_app_lock_wait_for_test(probe.clone());
    let guard = app.lock().await;
    let promoter = runtime.clone();
    let promoter_session = session.clone();
    let promoter_agent = agent.clone();
    let task = tokio::spawn(async move {
        promoter
            .advance_project_queued_prompt_after_settlement(&promoter_session, &promoter_agent)
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), probe.notified())
        .await
        .unwrap();
    runtime
        .owned
        .cancel_queued_prompt(&session, &agent, attachment.id(), &head_id)
        .unwrap();
    std::fs::write(root.path().join("release"), "").unwrap();
    drop(guard);
    let promoted = tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    let submitted = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !root.path().join("surviving-submitted").exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok();
    runtime
        .owned
        .provider_store
        .terminate_run_provider_only(&session, run.id())
        .unwrap();
    assert!(
        promoted.is_some_and(|p| p.prompt() == "surviving queued request"),
        "cancelled preparation head stranded the surviving queued request"
    );
    assert!(
        submitted,
        "surviving queued request did not reach the actual provider fixture input"
    );
}

#[tokio::test]
async fn readiness_while_the_starting_promoter_exits_cannot_strand_the_head() {
    let (_root, app, mut runtime, session, agent, old) = compact_fixture().await;
    runtime
        .owned
        .provider_store
        .terminate_run_provider_only(&session, old.id())
        .unwrap();
    let request = LaunchProviderRequest::new(
        &session,
        "claude",
        "claude",
        old.account_profile(),
        old.model(),
    )
    .with_agent_id(&agent);
    let mut starting = RuntimeProviderRun::new(
        "starting-race-run",
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "readiness-test".into(),
            pty_target: None,
            pty_program: Some("/bin/cat".into()),
            pty_args: vec![],
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: None,
            structured_endpoint: None,
        },
    );
    crate::test_support::authenticate_provider_account(
        &runtime.owned.provider_account_profiles,
        crate::session::DEFAULT_LOCAL_USER_ID,
        "claude",
        old.account_profile(),
    )
    .unwrap();
    runtime
        .owned
        .provider_store
        .write()
        .insert_run_for_test(starting.clone());
    runtime
        .owned
        .provider_store
        .initialize_runtime(&starting)
        .unwrap();
    let attachment = runtime
        .with_app_side_effect(|app| {
            crate::app::KernelSessionService::new(app).attach(
                crate::attachment::AttachRequest::new(
                    &session,
                    "readiness-race-observer",
                    crate::attachment::ClientCapabilityLevel::FullTerminal,
                ),
            )
        })
        .await
        .unwrap();
    let snapshot = runtime.owned.session_store.get_session(&session).unwrap();
    runtime
        .owned
        .prompt_state_owner
        .submit_prepared_prompt(
            &snapshot,
            crate::session::PromptQueueItem::new(
                "ready-head",
                attachment.id(),
                &agent,
                "surviving queued request",
                crate::session::PromptStatus::Queued,
            ),
            true,
        )
        .unwrap();
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    runtime.queue_promotion_exit_probe = Some((entered.clone(), release.clone()));
    let promoter = runtime.clone();
    let promoter_session = session.clone();
    let promoter_agent = agent.clone();
    let task = tokio::spawn(async move {
        promoter
            .advance_project_queued_prompt_after_settlement(&promoter_session, &promoter_agent)
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    assert!(runtime
        .owned
        .prompt_state_owner
        .has_idle_queue_promotion(&session, &agent));
    starting.mark_running();
    runtime
        .owned
        .provider_store
        .write()
        .insert_run_for_test(starting.clone());
    // This is the readiness decision in finish_provider_launch_success: the
    // held claim suppresses its advance. It must be recovered by the promoter.
    assert!(runtime
        .owned
        .prompt_state_owner
        .has_idle_queue_promotion(&session, &agent));
    release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    let promoted = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&snapshot, &agent)
                .is_some()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok();
    app.lock()
        .await
        .end_agent_provider_run(&session, &agent)
        .unwrap();
    assert!(
        promoted,
        "ready provider left its idle queued head stranded"
    );
}
