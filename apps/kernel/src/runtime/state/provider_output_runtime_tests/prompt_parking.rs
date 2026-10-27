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
    assert!(matches!(
        runtime
            .owned
            .terminate_idle_remote_provider_projection_for_agent_before_local_move(
                session.id(),
                &remote_agent,
            ),
        Err(crate::error::DaemonError::LocalTransport {
            operation: "move agent to local",
            ..
        })
    ));
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
