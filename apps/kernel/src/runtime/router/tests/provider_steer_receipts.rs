use super::*;

/// Replaces a run with a structured provider that has no live runtime, so it
/// refuses a steer after the mailbox admitted it.
fn install_refusing_structured_run(
    app: &DaemonApp,
    session_id: &str,
    agent_id: &str,
    run_id: &str,
) {
    let request = crate::provider::LaunchProviderRequest::new(
        session_id, "codex", "codex", "default", "model",
    )
    .with_agent_id(agent_id);
    let mut run = crate::provider::RuntimeProviderRun::new(
        run_id,
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "steer-receipt".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test".to_string()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.update_provider_run_projection(run);
}

#[tokio::test]
async fn an_active_steer_the_provider_rejects_leaves_no_success_effects() {
    let worktree = crate::test_support::TestWorktree::new("active-steer-provider-rejection");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should create");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-active-steer",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = launch_test_provider(
        &mut app,
        session.id(),
        agent.id(),
        "dev-stub",
        "dev-stub",
        "steer-model",
    );
    install_refusing_structured_run(&app, session.id(), agent.id(), run.id());
    let turn = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "a turn in progress",
        PromptStatus::Queued,
    );
    let PromptSubmissionOutcome::Started { prompt: active } = app
        .prompt_owner_submit_prepared_prompt(session.id(), turn, false)
        .expect("the turn should become active")
    else {
        panic!("the turn should start on an idle agent");
    };
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let result = router
        .runtime_state()
        .steer_active_prompt(crate::local::SteerActivePromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: agent.id().to_string(),
            expected_active_prompt_id: active.id().to_string(),
            prompt: "REJECTED_ACTIVE_STEER".into(),
            attachments: vec![],
        })
        .await;

    assert!(
        result.is_err(),
        "a steer the provider rejected must not report success"
    );
    let app = app.lock().await;
    let history = crate::app::KernelSessionReadService::new(&app)
        .session_history(session.id())
        .expect("history should read");
    assert!(
        !format!("{history:?}").contains("REJECTED_ACTIVE_STEER"),
        "a rejected steer must not be recorded in history"
    );
}

#[tokio::test]
async fn a_leased_steer_the_provider_rejects_is_not_recorded_as_accepted() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let lease = crate::app::RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel-rejected-steer",
            "home-session-rejected-steer",
            "home-agent-rejected-steer",
            false,
            "home-user-rejected-steer",
        )
        .expect("execution lease should create");
    let leased_agent = crate::app::RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
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
            "active prompt",
            Vec::new(),
            None,
            Some(crate::transport::relay_peer::RemoteGitTurnContext {
                home_session_id: "home-session-rejected-steer".to_string(),
                home_agent_id: "home-agent-rejected-steer".to_string(),
                home_prompt_id: "home-prompt-rejected".to_string(),
                home_turn_id: "home-prompt-rejected".to_string(),
                source_attachment_id: None,
                workspace_live_sync_mode: None,
                prompt_origin: Some(PromptOrigin::Chariox),
                external_provider: None,
                external_provider_session_id: None,
                external_provider_turn_id: None,
                prompt_summary: "remote active prompt".to_string(),
            }),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .expect("leased prompt should submit");
    assert!(matches!(outcome, PromptSubmissionOutcome::Started { .. }));
    install_refusing_structured_run(
        &app,
        &leased_agent.backing_session_id,
        &leased_agent.backing_agent_id,
        &provider_run_id,
    );
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let result = router
        .relay_steer_leased_prompt(
            &leased_agent.id,
            "home-queued-rejected",
            "home-prompt-rejected",
            "REJECTED_LEASED_STEER",
            "",
            Vec::new(),
            None,
        )
        .await;

    assert!(
        result.is_err(),
        "the home kernel must see the provider's rejection, not a delivered steer"
    );
    let mut app = app.lock().await;
    let receipt = crate::app::RemoteLeaseRuntime::new(&mut app)
        .leased_agent_snapshot_for_test(&leased_agent.id)
        .expect("leased agent should remain")
        .home_steer_receipts
        .into_iter()
        .find(|receipt| receipt.steer_id == "home-queued-rejected")
        .expect("the worker should keep a receipt for the steer");
    assert_eq!(
        receipt.phase,
        crate::execution_lease::LeasedPromptSteerReceiptPhase::Rejected,
        "a rejected steer must stay recoverable at the home kernel"
    );
}
