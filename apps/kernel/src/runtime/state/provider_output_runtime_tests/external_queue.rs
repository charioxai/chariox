use super::*;

#[tokio::test]
async fn unavailable_provider_account_defers_explicit_queue_advances_without_losing_work() {
    let worktree = crate::test_support::TestWorktree::new("external-queue-unavailable-account");
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "unavailable-account-queue-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("client should attach");
    let account = app
        .provider_account_profile_registry()
        .create_managed(
            crate::session::DEFAULT_LOCAL_USER_ID,
            "codex",
            "Queued work account",
        )
        .expect("account profile should register");
    crate::test_support::authenticate_provider_account(
        &app.provider_account_profile_registry(),
        crate::session::DEFAULT_LOCAL_USER_ID,
        "codex",
        &account.profile_id,
    )
    .expect("account should start authenticated");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .update_agent_profile(
            session.id(),
            agent.id(),
            crate::session::DEFAULT_LOCAL_USER_ID,
            Some("codex".to_string()),
            Some(account.profile_id.clone()),
            Some("gpt-5.4".to_string()),
            Some(Some("low".to_string())),
        )
        .await
        .expect("authenticated account should be assigned");
    let (provider_run_id, queued_prompt_id) = {
        let mut app = app.lock().await;
        let run = app
            .launch_provider(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "codex",
                    &account.profile_id,
                    "gpt-5.4",
                )
                .with_agent_id(agent.id()),
            )
            .expect("provider fixture should launch");
        app.update_provider_run_projection(run.clone());
        let queued = crate::session::PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            attachment.id(),
            agent.id(),
            "preserve this queued prompt",
            crate::session::PromptStatus::Queued,
        );
        let crate::session::PromptSubmissionOutcome::Queued { prompt } = app
            .prompt_owner_submit_prepared_prompt(session.id(), queued, true)
            .expect("prompt should queue")
        else {
            panic!("forced queue submission should remain queued");
        };
        (run.id().to_string(), prompt.id().to_string())
    };
    app.lock()
        .await
        .provider_account_profile_registry()
        .update_observation(
            crate::session::DEFAULT_LOCAL_USER_ID,
            "codex",
            &account.profile_id,
            crate::account_profile::ProviderAccountAuthState::Expired,
            None,
            None,
            None,
            None,
        )
        .expect("account should expire before queue advancement");

    assert!(runtime
        .owned
        .activate_next_queued_prompt_for_agent(session.id(), agent.id(), None)
        .expect("unavailable account should defer direct queue activation")
        .is_none());
    assert!(runtime
        .owned
        .advance_next_queued_prompt_dispatch(session.id(), agent.id(), &provider_run_id)
        .expect("unavailable account should defer provider queue dispatch")
        .is_none());
    app.lock()
        .await
        .provider_account_profile_registry()
        .remove_registration(
            crate::session::DEFAULT_LOCAL_USER_ID,
            "codex",
            &account.profile_id,
        )
        .expect("fixture should simulate a missing bound account");
    assert!(runtime
        .owned
        .activate_next_queued_prompt_for_agent(session.id(), agent.id(), None)
        .expect("missing account should fail closed and defer queue activation")
        .is_none());
    assert!(runtime
        .owned
        .advance_next_queued_prompt_dispatch(session.id(), agent.id(), &provider_run_id)
        .expect("missing account should fail closed and defer provider dispatch")
        .is_none());

    let snapshot = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should remain available");
    assert!(snapshot.active_prompt_for_agent(agent.id()).is_none());
    let queued = snapshot
        .queued_prompts_for_agent(agent.id())
        .expect("queued work should remain durable");
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].id(), queued_prompt_id);
}

#[tokio::test]
async fn completed_metaagent_task_starts_queued_task_despite_stale_session_prompt_mirror() {
    let worktree = crate::test_support::TestWorktree::new("external-queue-metaagent");
    let mut app =
        DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "metaagent-fifo-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("client should attach");
    app.sessions_mut()
        .start_or_update_metaagent_task(session.id(), agent.id(), "first Meta task")
        .expect("first Meta task should start");
    app.sessions_mut()
        .complete_metaagent_task(session.id(), agent.id(), Some("done".to_string()))
        .expect("first Meta task should complete");
    let queued = app
        .sessions_mut()
        .enqueue_metaagent_task(
            session.id(),
            agent.id(),
            attachment.id(),
            "second Meta task",
            Vec::new(),
        )
        .expect("second Meta task should queue");
    app.sessions_mut()
        .submit_prompt(
            session.id(),
            attachment.id(),
            agent.id(),
            "stale completed prompt mirror",
            Vec::new(),
        )
        .expect("legacy session mirror should contain a stale active prompt");
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;

    let dispatches = runtime
        .owned
        .workflow_maybe_start_next_queued_prompt(session.id());

    assert_eq!(dispatches.starting_metaagent_tasks.len(), 1);
    assert_eq!(dispatches.starting_metaagent_tasks[0].id(), queued.id());
}

#[tokio::test]
async fn paused_workflow_prompt_cannot_be_promoted_after_provider_launch() {
    let worktree = crate::test_support::TestWorktree::new("external-queue-paused-workflow");
    let mut app =
        DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");
    app.update_provider_run_projection(run.clone());
    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), Some("paused-queue".to_string()))
        .expect("workflow should be created");
    let node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), agent.id())
        .expect("workflow node should be added");
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");
    let workflow_run = app
        .sessions_mut()
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("run once".to_string()),
        )
        .expect("workflow run should be created");
    let node_run_id = workflow_run.node_runs()[0].id().to_string();
    app.sessions_mut()
        .prepare_workflow_turn(
            session.id(),
            workflow_run.id(),
            &node_run_id,
            format!("workflow-ack:{node_run_id}"),
            "queued workflow turn".to_string(),
            None,
            None,
        )
        .expect("workflow turn should be prepared");
    let queued = crate::session::PromptQueueItem::new(
        "pending-paused-workflow",
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
        agent.id(),
        "queued workflow turn",
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(workflow_run.id(), &node_run_id);
    let crate::session::PromptSubmissionOutcome::Queued { .. } = app
        .prompt_owner_submit_prepared_prompt(session.id(), queued, true)
        .expect("workflow prompt should remain queued while provider launch settles")
    else {
        panic!("workflow prompt should be queued");
    };
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .execute_workflow_interrupt_run(session.id(), workflow_run.id(), true)
        .await
        .expect("workflow should pause through the authoritative runtime path");
    let dispatch = runtime
        .owned
        .advance_next_queued_prompt_dispatch(session.id(), agent.id(), run.id())
        .expect("paused workflow queue cleanup should not fail");

    assert!(
        dispatch.is_none(),
        "a prompt owned by a paused workflow must never reach the provider"
    );
    let snapshot = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should remain available");
    assert!(snapshot.active_prompt_for_agent(agent.id()).is_none());
    assert!(
        snapshot
            .queued_prompts_for_agent(agent.id())
            .is_none_or(|queued| queued.is_empty()),
        "the stale paused-workflow prompt should be removed from the authoritative queue"
    );
}

#[tokio::test]
async fn external_active_prompt_blocks_queue_until_observer_settles_it() {
    let worktree = crate::test_support::TestWorktree::new("external-queue-settlement");
    let mut app =
        DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-external-queue-settlement",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");
    app.update_provider_run_projection(run.clone());
    let (external_prompt_id, queued_prompt_id) =
        sync_external_active_prompt_and_queue_chariox_prompt(
            &mut app,
            session.id(),
            attachment.id(),
            agent.id(),
        );

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    assert_external_active_prompt_and_queued_chariox_prompt(
        &runtime,
        session.id(),
        agent.id(),
        &external_prompt_id,
        &queued_prompt_id,
    );

    let blocked = runtime
        .owned
        .advance_next_queued_prompt_dispatch(session.id(), agent.id(), run.id())
        .expect("active external prompt should not error while blocking queue dispatch");
    assert!(
        blocked.is_none(),
        "queued Chariox prompt must not dispatch while external prompt is active"
    );

    {
        let mut app = app.lock().await;
        let changed = app
            .prompt_owner_sync_external_active_prompt(session.id(), agent.id(), None)
            .expect("observer settlement should clear external active prompt");
        assert!(changed);
    }

    let dispatch = runtime
        .owned
        .advance_next_queued_prompt_dispatch(session.id(), agent.id(), run.id())
        .expect("settled external prompt should release queued prompt")
        .expect("queued prompt should dispatch after external settlement");
    assert_eq!(dispatch.session_id, session.id());
    assert_eq!(dispatch.provider_run_id, run.id());
    assert_eq!(dispatch.agent_id, agent.id());
    assert_eq!(dispatch.prompt, "queued from Chariox\n");
    assert!(!dispatch.steering);

    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    let active_prompt = session_state
        .active_prompt_for_agent(agent.id())
        .expect("queued prompt should now be active");
    assert_eq!(active_prompt.id(), dispatch.prompt_id);
    assert_eq!(
        active_prompt.prompt_origin(),
        crate::session::PromptOrigin::Chariox
    );
    assert_eq!(active_prompt.prompt(), "queued from Chariox\n");
    assert!(
        session_state
            .queued_prompts_for_agent(agent.id())
            .map(|queued| queued.is_empty())
            .unwrap_or(true),
        "queue should be empty after deterministic promotion"
    );
}

#[tokio::test]
async fn external_active_prompt_rejects_queued_prompt_steering() {
    let worktree = crate::test_support::TestWorktree::new("external-queue-steering");
    let mut app =
        DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-external-steering",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");
    app.update_provider_run_projection(run);
    let (external_prompt_id, queued_prompt_id) =
        sync_external_active_prompt_and_queue_chariox_prompt(
            &mut app,
            session.id(),
            attachment.id(),
            agent.id(),
        );

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    assert_external_active_prompt_and_queued_chariox_prompt(
        &runtime,
        session.id(),
        agent.id(),
        &external_prompt_id,
        &queued_prompt_id,
    );

    let error = match runtime.owned.steer_queued_prompt(
        session.id(),
        agent.id(),
        attachment.id(),
        &queued_prompt_id,
    ) {
        Ok(_) => panic!("external active prompt should reject steering"),
        Err(error) => error,
    };
    match error {
        DaemonError::LocalTransport { operation, message } => {
            assert_eq!(operation, "steer queued prompt");
            assert_eq!(
                message,
                "queued prompts cannot be steered into externally started provider turns"
            );
        }
        other => panic!("expected LocalTransport steering error, got {other:?}"),
    }

    assert_external_active_prompt_and_queued_chariox_prompt(
        &runtime,
        session.id(),
        agent.id(),
        &external_prompt_id,
        &queued_prompt_id,
    );
}

#[tokio::test]
async fn late_launch_completion_preserves_active_local_prompt_and_queued_successor() {
    let worktree = crate::test_support::TestWorktree::new("late-launch-active-local");
    let mut app =
        DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "late-launch-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("client should attach");
    // Prepare a real idle managed provider, but deliberately hold its launch
    // completion until the ordinary prompt owner has an active turn and a queue.
    let started = app
        .start_provider_launch(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "native-tui-idle",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider launch should start");
    assert_eq!(
        started.run.state(),
        crate::provider::ProviderRunState::Starting
    );
    let mut admitted = Vec::new();
    for (id, prompt, force_queue) in [
        ("late-launch-active", "active local prompt", false),
        ("late-launch-queued", "queued local successor", true),
    ] {
        let item = crate::session::PromptQueueItem::new(
            id,
            attachment.id(),
            agent.id(),
            prompt,
            crate::session::PromptStatus::Queued,
        );
        let outcome = app
            .prompt_owner_submit_prepared_prompt(session.id(), item, force_queue)
            .expect("ordinary prompt admission should succeed");
        assert_eq!(
            matches!(
                outcome,
                crate::session::PromptSubmissionOutcome::Queued { .. }
            ),
            force_queue
        );
        let accepted = match outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt }
            | crate::session::PromptSubmissionOutcome::Queued { prompt } => prompt,
        };
        admitted.push(accepted.id().to_string());
    }
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime.finish_provider_launch(&started, None).await;
    let run = runtime
        .owned
        .provider_store
        .get_run(started.run.id())
        .expect("run remains queryable");
    let snapshot = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session remains queryable");
    // Always reap this test's real idle shell, even when the regression is red.
    app.lock()
        .await
        .teardown_provider_processes(None, true)
        .expect("fixture cleanup");
    assert_eq!(
        run.state(),
        crate::provider::ProviderRunState::Running,
        "queue occupancy is not a provider initialization failure"
    );
    assert_eq!(
        snapshot.active_prompt_for_agent(agent.id()).map(|p| p.id()),
        Some(admitted[0].as_str())
    );
    let queued = snapshot
        .queued_prompts_for_agent(agent.id())
        .expect("queue exists");
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].id(), admitted[1]);
    assert!(runtime
        .owned
        .complete_local_prompt_without_advance(session.id(), agent.id(), Some(started.run.id()),)
        .expect("active turn should complete normally")
        .is_some());
    let dispatch = runtime
        .owned
        .advance_next_queued_prompt_dispatch(session.id(), agent.id(), started.run.id())
        .expect("deferred successor should remain eligible")
        .expect("completed active turn should release the queued successor");
    assert_eq!(dispatch.prompt, "queued local successor");
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(started.run.id())
            .unwrap()
            .state(),
        crate::provider::ProviderRunState::Running
    );
}

#[tokio::test]
async fn losing_queue_claim_does_not_release_the_winning_workflow_workspace_claim() {
    let worktree = crate::test_support::TestWorktree::new("queue-workspace-claim-contention");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "native-tui-idle",
            )
            .with_agent_id(agent.id()),
        )
        .unwrap();
    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), Some("claim-retention".to_string()))
        .unwrap();
    let node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), agent.id())
        .unwrap();
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .unwrap();
    let workflow_run = app
        .sessions_mut()
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("one turn".to_string()),
        )
        .unwrap();
    let node_run_id = workflow_run.node_runs()[0].id().to_string();
    app.sessions_mut()
        .prepare_workflow_turn(
            session.id(),
            workflow_run.id(),
            &node_run_id,
            format!("workflow-ack:{node_run_id}"),
            "claimed workflow turn".to_string(),
            None,
            None,
        )
        .unwrap();
    let queued = crate::session::PromptQueueItem::new(
        "claim-retention-queued",
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
        agent.id(),
        "claimed workflow turn",
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(workflow_run.id(), &node_run_id);
    let crate::session::PromptSubmissionOutcome::Queued { prompt: observed } = app
        .prompt_owner_submit_prepared_prompt(session.id(), queued, true)
        .unwrap()
    else {
        panic!("workflow turn must start queued");
    };
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let winner = runtime
        .owned
        .advance_next_queued_prompt_dispatch(session.id(), agent.id(), run.id())
        .expect("winning dispatcher succeeds")
        .expect("queued workflow turn is claimed");
    let claim_id =
        runtime
            .owned
            .workflow_dispatch_claim_id(session.id(), workflow_run.id(), &node_run_id);
    let before_loser = runtime.owned.prompt_workspace_claims.contains(&claim_id);
    let mut loser_prepared = false;
    // The loser retained the same observed head before the winner activated it.
    // Its preparation must not acquire (or later release) the winner's claim.
    let loser = runtime
        .owned
        .prompt_state_owner
        .try_activate_next_queued_prompt_with_prompt_id(
            &session,
            agent.id(),
            observed.id(),
            "losing-dispatch".to_string(),
            |prompt| {
                loser_prepared = true;
                runtime
                    .owned
                    .ensure_workflow_prompt_workspace_claim(session.id(), prompt)?;
                Ok(())
            },
        )
        .expect("a losing opportunistic claim is ordinary contention");
    let after_loser = runtime.owned.prompt_workspace_claims.contains(&claim_id);
    let active = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent_snapshot(&session, agent.id());
    runtime.owned.release_workflow_node_workspace_claim(
        session.id(),
        workflow_run.id(),
        &node_run_id,
    );
    app.lock()
        .await
        .teardown_provider_processes(None, true)
        .unwrap();
    assert!(
        before_loser && after_loser,
        "the winning workflow retains its actual workspace claim"
    );
    assert!(
        !loser_prepared,
        "only the eligible queue winner may prepare a workspace claim"
    );
    assert!(loser.is_none());
    assert_eq!(active.unwrap().id(), winner.prompt_id);
}
