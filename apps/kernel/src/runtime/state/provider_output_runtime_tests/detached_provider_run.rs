use super::*;

#[tokio::test]
async fn detaching_last_attachment_defers_starting_run_park_until_it_is_idle() {
    let mut app =
        DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-detached-starting-run",
            "worktree-detached-starting-run",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-a",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment_id = attachment.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let state = owned_runtime_state(&app).await;
    let started = state
        .owned
        .start_provider_launch(
            crate::provider::LaunchProviderRequest::new(
                &session_id,
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(&agent_id),
        )
        .expect("provider launch should enter starting state");

    state
        .detach(&attachment_id)
        .await
        .expect("detaching during provider startup should succeed");

    assert_eq!(
        state
            .owned
            .provider_store
            .get_run(started.run.id())
            .expect("starting provider run should remain")
            .state(),
        crate::provider::ProviderRunState::Starting
    );
    assert_eq!(
        state
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should remain")
            .active_provider_run_id(),
        Some(started.run.id())
    );

    state
        .owned
        .provider_store
        .mark_run_running(started.run.id())
        .expect("provider launch should become running");
    assert!(state
        .owned
        .park_detached_idle_provider_run(&session_id)
        .expect("idle detached provider should park"));
    assert_eq!(
        state
            .owned
            .provider_store
            .get_run(started.run.id())
            .expect("parked provider run should remain")
            .state(),
        crate::provider::ProviderRunState::Parked
    );
    assert_eq!(
        state
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should remain")
            .active_provider_run_id(),
        None
    );
}
