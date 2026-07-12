use super::*;

fn remote_git_context(home_prompt_id: &str) -> crate::transport::relay_peer::RemoteGitTurnContext {
    crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: "home-session-steer".to_string(),
        home_agent_id: "home-agent-steer".to_string(),
        home_prompt_id: home_prompt_id.to_string(),
        home_turn_id: home_prompt_id.to_string(),
        source_attachment_id: None,
        workspace_live_sync_mode: None,
        prompt_origin: Some(PromptOrigin::Arroba),
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
