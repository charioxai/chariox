use super::*;

#[tokio::test]
async fn leased_forwarding_accepts_only_the_current_home_origin() {
    let worktree = crate::test_support::TestWorktree::new("home-turn-origin");
    let mut app =
        crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "origin-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "dev-stub",
        "dev-stub",
        "default",
        "model",
    )
    .with_agent_id(agent.id());
    let run = crate::provider::RuntimeProviderRun::new(
        "worker-run",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "origin-fixture".into(),
            pty_target: None,
            pty_program: None,
            pty_args: vec![],
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: None,
            structured_endpoint: None,
        },
    );
    let context = crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext {
        home_kernel_id: "home-kernel".into(),
        home_session_id: "home-room".into(),
        home_agent_id: "home-agent".into(),
        home_prompt_id: Some("home-origin".into()),
        leased_agent_id: "leased-agent".into(),
        worker_kernel_id: "worker-kernel".into(),
        worker_machine_id: "worker-machine".into(),
        worker_provider_run_id: run.id().into(),
        worker_worktree_path: session.worktree_id().into(),
        worker_workspace_identity: crate::io::WorkspaceIdentity::local(session.worktree_id()),
    };
    let prompt = crate::session::PromptQueueItem::new(
        "worker-origin",
        attachment.id(),
        agent.id(),
        "read metadata",
        crate::session::PromptStatus::Queued,
    )
    .with_leased_skills(Default::default(), Some(context.clone()));
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .unwrap();
    let app = std::sync::Arc::new(tokio::sync::Mutex::new(app));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
        std::sync::Arc::clone(&app),
        1,
    );
    let state = router.runtime_state();
    let args = serde_json::json!({"origin_prompt_id":"home-origin", "message":"metadata"});
    assert_eq!(
        state
            .arguments_for_home_turn(&run, &context, &args)
            .unwrap(),
        Some(args.clone())
    );
    for origin in ["worker-origin", "old-home-origin", "foreign-home-origin"] {
        let mut stale = args.clone();
        stale["origin_prompt_id"] = origin.into();
        assert!(state
            .arguments_for_home_turn(&run, &context, &stale)
            .unwrap()
            .is_none());
    }
    app.lock()
        .await
        .prompt_owner_complete_active_prompt_only(session.id(), agent.id())
        .unwrap();
    assert!(state
        .arguments_for_home_turn(&run, &context, &args)
        .unwrap()
        .is_none());
}

#[test]
fn a_missing_home_binding_never_uses_the_worker_origin() {
    let prompt = crate::session::PromptQueueItem::new(
        "local-or-worker-origin",
        "attachment",
        "agent",
        "metadata",
        crate::session::PromptStatus::Running,
    );
    assert_eq!(prompt.runtime_origin_prompt_id(false).unwrap(), prompt.id());
    assert!(prompt.runtime_origin_prompt_id(true).is_err());
}
