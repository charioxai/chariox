use super::*;

#[tokio::test]
async fn pending_prompt_protects_its_agents_provider_after_session_focus_moves() {
    let worktree = crate::test_support::TestWorktree::new("pending-prompt-focus-move");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let (session, first_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let pending_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(crate::agent::CreateAgentRequest::new(
            session.id(),
            "dev-stub",
        ))
        .expect("pending agent should spawn");
    let first_run = app
        .providers
        .launch_run_detached(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "default",
            )
            .with_agent_id(first_agent.id()),
        )
        .expect("first provider should launch");
    let pending_run = app
        .providers
        .launch_run_detached(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "default",
            )
            .with_agent_id(pending_agent.id()),
        )
        .expect("pending provider should launch");
    let prompt = crate::session::PromptQueueItem::new(
        "pending-prompt",
        crate::scheduler::runtime::workflow_prompt_source_attachment_id("pending-run"),
        pending_agent.id(),
        "pending branch",
        crate::session::PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Started { prompt } = app
        .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("pending prompt should be admitted")
    else {
        panic!("prompt should start");
    };
    assert!(prompt.durable_delivery_provider_run_id().is_none());
    // Another branch can move the session-wide projection while this prompt's
    // provider is prepared but not yet bound by the delivery acknowledgement.
    app.sessions
        .set_active_provider_run(session.id(), Some(first_run.id().to_string()))
        .expect("other branch should become the projected provider");
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    assert!(runtime
        .owned
        .provider_run_has_prompt_work(session.id(), &pending_run)
        .expect("pending prompt should protect its current provider"));
    assert!(app
        .lock()
        .await
        .provider_run_has_prompt_work(session.id(), &pending_run)
        .expect("app-side lifecycle must protect the same provider"));

    {
        let mut app = app.lock().await;
        let queued = crate::session::PromptQueueItem::new(
            "queued-prompt",
            crate::scheduler::runtime::workflow_prompt_source_attachment_id("pending-run"),
            pending_agent.id(),
            "queued branch",
            crate::session::PromptStatus::Queued,
        );
        app.prompt_owner_submit_prepared_prompt(session.id(), queued, true)
            .expect("next prompt should queue");
        app.prompt_owner_complete_active_prompt_only(session.id(), pending_agent.id())
            .expect("only the queued prompt should remain before launch promotion");
    }
    assert!(!runtime
        .owned
        .provider_run_has_active_prompt(session.id(), &pending_run)
        .expect("queued work is distinct from an active prompt"));
    runtime
        .owned
        .session_store
        .set_active_provider_run(session.id(), Some(pending_run.id().to_string()))
        .expect("finishing launch should project the pending provider");
    runtime
        .owned
        .sync_focused_provider_run_if_idle(session.id())
        .expect("owned idle reconciliation should preserve queued delivery");
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(pending_run.id())
            .unwrap()
            .state(),
        crate::provider::ProviderRunState::Running
    );
    {
        let mut app = app.lock().await;
        app.sessions
            .set_active_provider_run(session.id(), Some(pending_run.id().to_string()))
            .expect("app-side reconciliation should see the same launch handoff");
        app.sync_focused_provider_run_if_idle(session.id())
            .expect("app idle reconciliation should preserve queued delivery");
        assert_eq!(
            app.providers.get_run(pending_run.id()).unwrap().state(),
            crate::provider::ProviderRunState::Running
        );
    }

    let replacement = runtime
        .owned
        .provider_store
        .launch_run_detached(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "default",
            )
            .with_agent_id(pending_agent.id()),
        )
        .expect("replacement provider should launch");
    assert!(!runtime
        .owned
        .provider_run_has_prompt_work(session.id(), &pending_run)
        .expect("queued prompt should not protect an obsolete provider"));
    assert!(runtime
        .owned
        .provider_run_has_prompt_work(session.id(), &replacement)
        .expect("queued prompt should protect its replacement provider"));

    // Exit settlement must still recognize an unbound active prompt after its
    // selected run has ended, even if an older run for this agent remains alive.
    let session_state = runtime
        .owned
        .session_store
        .get_session(session.id())
        .unwrap();
    let active = runtime
        .owned
        .prompt_state_owner
        .activate_next_queued_prompt(&session_state, pending_agent.id(), None)
        .expect("queued prompt should promote")
        .expect("there should be queued work");
    assert!(active.durable_delivery_provider_run_id().is_none());
    runtime
        .owned
        .session_store
        .set_active_provider_run(session.id(), Some(replacement.id().to_string()))
        .expect("replacement should be the selected provider");
    let ended = runtime
        .owned
        .provider_store
        .mark_run_ended_provider_only(session.id(), replacement.id())
        .expect("replacement should end")
        .into_run();
    assert!(runtime
        .owned
        .provider_run_has_active_prompt(session.id(), &ended)
        .expect("owned exit settlement must retain pending prompt ownership"));
    assert!(app
        .lock()
        .await
        .provider_run_has_active_prompt(session.id(), &ended)
        .expect("app exit settlement must retain the same ownership"));
}

#[tokio::test]
async fn projected_unacknowledged_prompt_blocks_local_move_and_protects_its_run() {
    let worktree = crate::test_support::TestWorktree::new("projected-pending-focus");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    app.agents
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "pending-worker".to_string(),
                worker_machine_id: "pending-machine".to_string(),
                execution_lease_id: "pending-lease".to_string(),
                leased_agent_id: "pending-leased-agent".to_string(),
                active_worker_provider_run_id: Some("pending-worker-run".to_string()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .expect("agent should bind to its worker");
    let mut worker_run = crate::provider::RuntimeProviderRun::from_control_capability_inference(
        "pending-worker-run",
        "worker-session".to_string(),
        Some("pending-leased-agent".to_string()),
        "dev-stub".to_string(),
    );
    worker_run.mark_running();
    let projected_id = crate::provider::projected_leased_provider_run_id(
        "pending-leased-agent",
        "pending-worker-run",
    );
    let projected =
        worker_run.projected_for_home_agent_with_id(&projected_id, session.id(), agent.id());
    app.update_provider_run_projection(projected.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(projected_id.clone()))
        .expect("worker run should be selected");
    let prompt = crate::session::PromptQueueItem::new(
        "projected-pending-prompt",
        crate::scheduler::runtime::workflow_prompt_source_attachment_id("projected-pending-run"),
        agent.id(),
        "pending remote branch",
        crate::session::PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Started { prompt } = app
        .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("remote prompt should be admitted")
    else {
        panic!("remote prompt should start");
    };
    assert!(prompt.durable_delivery_provider_run_id().is_none());
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    assert!(runtime
        .owned
        .provider_store
        .get_run_for_agent(session.id(), agent.id())
        .is_none());
    assert!(runtime
        .owned
        .provider_run_has_active_prompt(session.id(), &projected)
        .unwrap());
    assert!(app
        .lock()
        .await
        .provider_run_has_active_prompt(session.id(), &projected)
        .unwrap());
    let remote_agent = runtime.owned.agent_store.get_agent(agent.id()).unwrap();
    // The unacknowledged prompt keeps the agent busy, so the Move waits for it instead of starting.
    let waiting = runtime
        .move_agent_transaction(session.id(), remote_agent, None)
        .await
        .expect("a Move requested while the prompt is unacknowledged is deferred");
    assert_eq!(
        waiting.agent_move().map(|status| &status.stage),
        Some(&crate::agent_move::AgentMoveStage::WaitingForTurn)
    );
    assert_eq!(
        runtime
            .owned
            .provider_run_projection
            .get(&projected_id)
            .unwrap()
            .state(),
        crate::provider::ProviderRunState::Running
    );
    // A sibling branch can move the session-wide projection before parking
    // checks the captured worker run; its per-agent projection still owns it.
    runtime
        .owned
        .session_store
        .set_active_provider_run(session.id(), Some("other-branch-run".to_string()))
        .expect("sibling branch should move the projection");
    assert!(runtime
        .owned
        .provider_run_has_prompt_work(session.id(), &projected)
        .unwrap());
    assert!(app
        .lock()
        .await
        .provider_run_has_prompt_work(session.id(), &projected)
        .unwrap());
}

#[tokio::test]
async fn move_waiting_for_the_turn_holds_queued_work_until_it_is_cancelled() {
    let worktree = crate::test_support::TestWorktree::new("move-waiting-queue-hold");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let prompt = |id: &str| {
        crate::session::PromptQueueItem::new(
            id,
            crate::scheduler::runtime::workflow_prompt_source_attachment_id("move-wait-run"),
            agent.id(),
            id,
            crate::session::PromptStatus::Queued,
        )
    };
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt("active-prompt"), false)
        .expect("first prompt should start");
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt("queued-prompt"), true)
        .expect("second prompt should queue behind the turn");
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;

    let waiting = runtime
        .move_agent_transaction(session.id(), agent.clone(), Some("target".to_string()))
        .await
        .expect("a Move requested during a turn is deferred");
    assert_eq!(
        waiting.agent_move().map(|status| &status.stage),
        Some(&crate::agent_move::AgentMoveStage::WaitingForTurn)
    );
    app.lock()
        .await
        .prompt_owner_complete_active_prompt_only(session.id(), agent.id())
        .expect("the turn ends");

    // The queue behind the turn belongs to the Move until it runs or is cancelled.
    let session_state = runtime
        .owned
        .session_store
        .get_session(session.id())
        .unwrap();
    let owner = &runtime.owned.prompt_state_owner;
    assert!(owner
        .peek_next_queued_prompt(&session_state, agent.id())
        .is_none());
    assert!(owner
        .activate_next_queued_prompt_for_dispatch(&session_state, agent.id(), None)
        .unwrap()
        .is_none());
    assert!(owner
        .try_claim_idle_queue_promotion(&session_state, agent.id())
        .is_none());

    runtime
        .abort_agent_move(session.id(), agent.agent_ref(), agent.owner_user_id())
        .await
        .expect("a waiting Move can be cancelled");
    assert!(
        owner
            .peek_next_queued_prompt(&session_state, agent.id())
            .is_some(),
        "cancelling the wait releases the queue"
    );
}

/// A Move waits for a running turn and a queued prompt; the caller ends the turn.
async fn deferred_move_behind_a_turn(
    worktree: &crate::test_support::TestWorktree,
) -> (
    Arc<Mutex<DaemonApp>>,
    KernelRuntimeState,
    crate::agent::AgentInstance,
    String,
) {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let prompt = |id: &str| {
        crate::session::PromptQueueItem::new(
            id,
            crate::scheduler::runtime::workflow_prompt_source_attachment_id("move-fence-run"),
            agent.id(),
            id,
            crate::session::PromptStatus::Queued,
        )
    };
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt("active-prompt"), false)
        .expect("first prompt should start");
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt("queued-prompt"), true)
        .expect("second prompt should queue behind the turn");
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .move_agent_transaction(session.id(), agent.clone(), Some("target".to_string()))
        .await
        .expect("a Move requested during a turn is deferred");
    (app, runtime, agent, session.id().to_string())
}

async fn failed_move_reason(runtime: &KernelRuntimeState, agent_id: &str) -> String {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let agent = runtime.owned.agent_store.get_agent(agent_id).unwrap();
        if let Some(crate::agent_move::AgentMoveStage::Failed { reason }) =
            agent.agent_move().map(|status| &status.stage)
        {
            return reason.clone();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the deferred Move ends with a status"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn deferred_move_keeps_its_fence_until_it_claims_the_agent() {
    let worktree = crate::test_support::TestWorktree::new("move-waiting-fence-handoff");
    let (app, runtime, agent, session_id) = deferred_move_behind_a_turn(&worktree).await;

    // The turn ends while the app is busy, so the Move cannot publish its Moving status yet.
    let mut held_app = app.lock().await;
    held_app
        .prompt_owner_complete_active_prompt_only(&session_id, agent.id())
        .expect("the turn ends");
    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    let session_state = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .unwrap();
    let owner = &runtime.owned.prompt_state_owner;
    assert!(
        owner
            .try_claim_idle_queue_promotion(&session_state, agent.id())
            .is_none(),
        "the queue promoter must not take an agent whose Move has not claimed it yet"
    );
    let late = held_app
        .prompt_owner_submit_prepared_prompt(
            &session_id,
            crate::session::PromptQueueItem::new(
                "late-prompt",
                crate::scheduler::runtime::workflow_prompt_source_attachment_id("move-fence-run"),
                agent.id(),
                "late",
                crate::session::PromptStatus::Queued,
            ),
            false,
        )
        .expect("a prompt is admitted");
    assert!(
        matches!(late, crate::session::PromptSubmissionOutcome::Queued { .. }),
        "a prompt must wait for the Move instead of starting on the source: {late:?}"
    );
    drop(held_app);

    let reason = failed_move_reason(&runtime, agent.id()).await;
    assert!(
        !reason.contains("active turn") && !reason.contains("preparing queued work"),
        "the Move must claim the agent after its turn instead of failing the claim: {reason}"
    );
}

#[tokio::test]
async fn deferred_move_cancelled_before_it_claims_the_agent_does_not_move() {
    let worktree = crate::test_support::TestWorktree::new("move-waiting-cancel-handoff");
    let (app, runtime, agent, session_id) = deferred_move_behind_a_turn(&worktree).await;

    let mut held_app = app.lock().await;
    held_app
        .prompt_owner_complete_active_prompt_only(&session_id, agent.id())
        .expect("the turn ends");
    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    assert!(
        runtime
            .owned
            .agent_moves
            .waiting
            .cancel(&crate::agent_move::wait_key(&session_id, agent.id())),
        "the Move still waits until it claims the agent"
    );
    drop(held_app);

    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    let agent = runtime.owned.agent_store.get_agent(agent.id()).unwrap();
    assert!(
        !matches!(
            agent.agent_move().map(|status| &status.stage),
            Some(crate::agent_move::AgentMoveStage::Failed { .. })
        ),
        "a cancelled Move must stop before it claims the agent: {:?}",
        agent.agent_move()
    );
    assert!(runtime
        .owned
        .agent_moves
        .for_agent(&session_id, agent.id())
        .is_none());
}
