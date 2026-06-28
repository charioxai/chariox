use super::*;

#[test]
fn daemon_app_bootstrap_wires_runtime_services() {
    let config = DaemonConfig::for_tests();
    let app = DaemonApp::bootstrap(config.clone()).expect("bootstrap should succeed");

    assert_eq!(app.config(), &config);
    assert_eq!(app.sessions().active_session_count(), 0);
    assert!(app.attachments().list_events().is_empty());
    assert!(app.providers().registry().registered_adapter_count() >= 1);
    assert!(app.terminal().input_records().is_empty());
    assert!(app.terminal().output_records().is_empty());
    assert!(app.terminal().notice_records().is_empty());
    assert_eq!(
        app.startup_message(),
        format!(
            "arroba daemon daemon-test ready on machine machine-test ({})",
            config.kernel_websocket_url()
        )
    );
}

#[test]
fn daemon_config_rejects_empty_identifiers() {
    let error = match DaemonApp::bootstrap(DaemonConfig::new("", "machine-local", "miguel")) {
        Ok(_) => panic!("empty daemon id should be rejected"),
        Err(error) => error,
    };

    match error {
        DaemonError::InvalidConfig { field, .. } => assert_eq!(field, "daemon_id"),
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn ending_session_via_app_removes_runtime_attachments() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");

    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let ended = crate::app::KernelSessionService::new(&mut app)
        .end_session(session.id())
        .expect("session should end cleanly through the app");

    assert_eq!(ended.id(), session.id());
    assert!(app.attachments().get_attachment(attachment.id()).is_err());
}

#[test]
fn failed_kernel_prompt_abort_finalizes_cancelling_prompt() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = app
        .launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");

    let _ = crate::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "cancel me\n",
        Vec::new(),
    )
    .expect("prompt should start");
    let cancellation = crate::transport::TransportService::cancel_active_prompt(
        &mut app,
        session.id(),
        attachment.id(),
    )
    .expect("prompt should enter cancellation");
    assert_eq!(cancellation.prompt.status(), PromptStatus::Cancelling);

    let result = app.finish_kernel_prompt_abort(
        session.id().to_string(),
        run.id().to_string(),
        Err(DaemonError::LocalTransport {
            operation: "abort test",
            message: "abort dispatch failed".to_string(),
        }),
    );

    assert!(result.is_err());
    let session_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert!(session_state.active_prompt_for_agent(agent.id()).is_none());
}

#[test]
fn shutdown_cleanup_preserves_sessions_and_clears_runtime_state() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "claude-code",
            "default",
            "sonnet",
        ))
        .expect("provider run should launch");

    assert_eq!(app.sessions().active_session_count(), 1);
    assert!(app
        .provider_process_tracking
        .snapshot()
        .processes
        .values()
        .any(|process| process.owner_provider_run_ids == vec![run.id().to_string()]));
    app.prompt_activity_store().write().insert(
        run.id().to_string(),
        crate::app::ActivePromptState {
            last_output_at: None,
            saw_response_content: false,
            completion_recorded: false,
            settlement_requested: false,
        },
    );
    app.active_turn_store()
        .start(crate::app::ActiveTurnState::new(
            session.id().to_string(),
            run.agent_instance_id()
                .expect("provider run should be bound to an agent")
                .to_string(),
            "prompt-before-shutdown".to_string(),
            run.id().to_string(),
        ));

    app.shutdown_cleanup()
        .expect("shutdown cleanup should preserve sessions");

    assert_eq!(app.sessions().active_session_count(), 1);
    let cleaned_session = app
        .sessions()
        .get_session(session.id())
        .expect("session should remain joinable after shutdown cleanup");
    assert_ne!(cleaned_session.status(), SessionStatus::Ended);
    assert_eq!(cleaned_session.active_provider_run_id(), None);
    assert!(
        cleaned_session.attachment_ids().is_empty(),
        "runtime attachments must not survive daemon shutdown"
    );
    assert!(app
        .provider_process_tracking
        .snapshot()
        .processes
        .is_empty());
    assert!(app
        .provider_process_tracking
        .snapshot()
        .run_processes
        .is_empty());
    assert!(
        !app.prompt_activity_store().read().contains_key(run.id()),
        "prompt activity must not survive daemon shutdown"
    );
    assert!(
        !app.active_turn_store().snapshot().contains_key(run.id()),
        "active turns must not survive daemon shutdown"
    );
    assert!(app.attachments().get_attachment(attachment.id()).is_err());
    assert!(app
        .durable_state_store()
        .load_events_after(0)
        .expect("durable events should load")
        .iter()
        .all(|event| event.kind != "session.ended"));
}
