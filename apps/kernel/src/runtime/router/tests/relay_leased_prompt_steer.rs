use super::*;

fn remote_git_context(home_prompt_id: &str) -> crate::transport::relay_peer::RemoteGitTurnContext {
    crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: "home-session-steer".to_string(),
        home_agent_id: "home-agent-steer".to_string(),
        home_prompt_id: home_prompt_id.to_string(),
        home_turn_id: home_prompt_id.to_string(),
        source_attachment_id: None,
        workspace_live_sync_mode: None,
        prompt_origin: Some(PromptOrigin::Chariox),
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        prompt_summary: "remote active prompt".to_string(),
    }
}

#[tokio::test]
async fn leased_prompt_steer_delivers_once_and_resets_for_the_next_turn() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let lease = crate::app::RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel-steer",
            "home-session-steer",
            "home-agent-steer",
            false,
            "home-user-steer",
        )
        .expect("execution lease should create");
    let leased_agent = crate::app::RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            Some("default".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased agent should create");
    let (provider_run_id, outcome) = crate::app::RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt_with_workflow_context(
            &leased_agent.id,
            "first active prompt",
            Vec::new(),
            None,
            Some(remote_git_context("home-prompt-1")),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .expect("leased prompt should submit");
    assert!(matches!(outcome, PromptSubmissionOutcome::Started { .. }));

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let (first_run_id, first_replayed) = router
        .relay_steer_leased_prompt(
            &leased_agent.id,
            "home-queued-prompt-1",
            "home-prompt-1",
            "REMOTE_STEER_ONCE",
            "",
            Vec::new(),
            None,
        )
        .await
        .expect("first remote steer should deliver");
    assert_eq!(first_run_id, provider_run_id);
    assert!(!first_replayed);

    let (replayed_run_id, replayed) = router
        .relay_steer_leased_prompt(
            &leased_agent.id,
            "home-queued-prompt-1",
            "home-prompt-1",
            "REMOTE_STEER_ONCE",
            "",
            Vec::new(),
            None,
        )
        .await
        .expect("duplicate remote steer should replay its acknowledgement");
    assert_eq!(replayed_run_id, provider_run_id);
    assert!(replayed);

    {
        let mut app = app.lock().await;
        crate::app::RemoteLeaseRuntime::new(&mut app)
            .complete_leased_prompt(&leased_agent.id)
            .expect("first leased prompt should complete");
        let (_, outcome) = crate::app::RemoteLeaseRuntime::new(&mut app)
            .submit_leased_prompt_with_workflow_context(
                &leased_agent.id,
                "second active prompt",
                Vec::new(),
                None,
                Some(remote_git_context("home-prompt-2")),
                Vec::new(),
                None,
                crate::extension::RemoteExtensionManifest::default(),
            )
            .expect("second leased prompt should submit");
        assert!(matches!(outcome, PromptSubmissionOutcome::Started { .. }));
    }

    let (_, next_turn_replayed) = router
        .relay_steer_leased_prompt(
            &leased_agent.id,
            "home-queued-prompt-1",
            "home-prompt-2",
            "REMOTE_STEER_ONCE",
            "",
            Vec::new(),
            None,
        )
        .await
        .expect("the next turn should accept a fresh steer namespace");
    assert!(!next_turn_replayed);

    let app = app.lock().await;
    let deliveries = app
        .terminal()
        .input_records()
        .into_iter()
        .filter(|record| String::from_utf8_lossy(&record.bytes).contains("REMOTE_STEER_ONCE"))
        .count();
    assert_eq!(
        deliveries, 2,
        "each active turn should receive one delivery"
    );
}

#[tokio::test]
async fn leased_provider_tool_list_exposes_event_reply_for_fresh_and_reused_discovery() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config).expect("daemon should boot"),
    ));
    let leased_agent = {
        let mut app_guard = app.try_lock().expect("app should be available");
        let lease = crate::app::RemoteLeaseRuntime::new(&mut app_guard)
            .create_execution_lease(
                "home-kernel-tools",
                "home-session-tools",
                "home-agent-tools",
                false,
                "home-user-tools",
            )
            .expect("execution lease should create");
        let leased_agent = crate::app::RemoteLeaseRuntime::new(&mut app_guard)
            .create_leased_agent(
                &lease.id,
                "managed-dev-stub",
                Some("default".to_string()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .expect("leased agent should create");
        leased_agent
    };
    let workflow_context = crate::execution_lease::RemoteWorkflowTurnContext {
        home_kernel_id: "home-kernel-tools".to_string(),
        home_session_id: "home-session-tools".to_string(),
        home_agent_id: "home-agent-tools".to_string(),
        workflow_run_id: "workflow-run-tools".to_string(),
        workflow_node_run_id: "workflow-node-tools".to_string(),
        delivery_token: "delivery-token-tools".to_string(),
        event_reply_enabled: true,
    };
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let fresh_discovery_saw_reply = Arc::new(AtomicBool::new(false));
    let probe_state = router.runtime_state.clone();
    let probe_flag = Arc::clone(&fresh_discovery_saw_reply);
    let provider_run_projection = app
        .try_lock()
        .expect("app should be available")
        .provider_run_projection_store();
    provider_run_projection.install_leased_provider_run_probe(move |provider_run_id| {
        if let Some(auth_token) =
            probe_state.runtime_mcp_auth_token_for_provider_run(provider_run_id)
        {
            if probe_state
                .runtime_tool_specs_for_auth_token(&auth_token)
                .iter()
                .any(|spec| {
                    spec.name == crate::transport::runtime_tools::REPLY_TO_EVENT_TOOL_QUALIFIED
                })
            {
                probe_flag.store(true, Ordering::SeqCst);
            }
        }
    });
    let (provider_run_id, outcome) = {
        let mut app_guard = app.try_lock().expect("app should be available");
        crate::app::RemoteLeaseRuntime::new(&mut app_guard)
            .submit_leased_prompt_with_workflow_context(
                &leased_agent.id,
                "event-triggered leased prompt",
                Vec::new(),
                Some(workflow_context),
                None,
                Vec::new(),
                None,
                crate::extension::RemoteExtensionManifest::default(),
            )
            .expect("leased workflow prompt should submit")
    };
    assert!(matches!(outcome, PromptSubmissionOutcome::Started { .. }));
    assert!(
        fresh_discovery_saw_reply.load(Ordering::SeqCst),
        "fresh provider discovery must expose reply_to_event before launch returns"
    );
    let leased_token = {
        let app_guard = app.try_lock().expect("app should be available");
        app_guard
            .providers()
            .get_run(&provider_run_id)
            .expect("leased provider run should exist")
            .runtime_mcp_auth_token()
            .expect("leased provider run should expose runtime MCP auth")
            .to_string()
    };

    let (ordinary_token, reused_provider_run_id) = {
        let mut app_guard = app.try_lock().expect("app should be available");
        let (ordinary_session, ordinary_agent) =
            crate::app::KernelSessionService::new(&mut app_guard)
                .create_session(CreateSessionRequest::new(
                    "ordinary-workflow-tools",
                    "ordinary-workflow-tools",
                ))
                .expect("ordinary session should create");
        let ordinary_run = launch_test_provider(
            &mut app_guard,
            ordinary_session.id(),
            ordinary_agent.id(),
            "dev-stub",
            "dev-stub",
            "ordinary-tools-model",
        );
        let ordinary_token = ordinary_run
            .runtime_mcp_auth_token()
            .expect("ordinary provider run should expose runtime MCP auth")
            .to_string();
        let (reused_provider_run_id, reused_outcome) =
            crate::app::RemoteLeaseRuntime::new(&mut app_guard)
                .submit_leased_prompt_with_workflow_context(
                    &leased_agent.id,
                    "second event-triggered leased prompt",
                    Vec::new(),
                    None,
                    None,
                    Vec::new(),
                    None,
                    crate::extension::RemoteExtensionManifest::default(),
                )
                .expect("reused leased workflow prompt should submit");
        assert!(matches!(
            reused_outcome,
            PromptSubmissionOutcome::Started { .. } | PromptSubmissionOutcome::Queued { .. }
        ));
        (ordinary_token, reused_provider_run_id)
    };
    assert_eq!(provider_run_id, reused_provider_run_id);
    let specs = router
        .runtime_state
        .runtime_tool_specs_for_auth_token(&leased_token);
    assert!(specs.iter().any(|spec| {
        spec.name == crate::transport::runtime_tools::REPLY_TO_EVENT_TOOL_QUALIFIED
    }));
    let _app_guard = app
        .try_lock()
        .expect("the app mutex should be available for the contention check");
    let contended_specs = router
        .runtime_state
        .runtime_tool_specs_for_auth_token(&leased_token);
    assert!(
        contended_specs.iter().any(|spec| {
            spec.name == crate::transport::runtime_tools::REPLY_TO_EVENT_TOOL_QUALIFIED
        }),
        "provider discovery must retain reply_to_event while the app mutex is contended"
    );
    let ordinary_specs = router
        .runtime_state
        .runtime_tool_specs_for_auth_token(&ordinary_token);
    assert!(!ordinary_specs
        .iter()
        .any(|spec| { spec.name == crate::transport::runtime_tools::ACK_WORKFLOW_TURN_TOOL }));
    assert!(!ordinary_specs.iter().any(|spec| {
        spec.name == crate::transport::runtime_tools::REPLY_TO_EVENT_TOOL_QUALIFIED
    }));
}
