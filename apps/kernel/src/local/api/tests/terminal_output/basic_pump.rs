use super::*;

#[test]
fn local_request_api_rejects_terminal_input_without_active_run() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-terminal-input", "worktree-terminal-input"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected response: {other:?}"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-terminal-input".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        other => panic!("unexpected response: {other:?}"),
    };

    let error = harness
        .dispatch(LocalDaemonRequest::SendTerminalInput(
            SendTerminalInputRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                provider_run_id: None,
                data_base64: "WA==".to_string(),
            },
        ))
        .expect_err("terminal input requires an active provider run");
    assert!(
        matches!(
            error,
            DaemonError::NoActiveProviderRun { ref session_id } if session_id == session.id()
        ),
        "unexpected error: {error:?}"
    );
}

#[test]
fn structured_output_pump_applies_finished_jobs_from_other_runs() {
    let harness = LocalRouterTestHarness::new();
    let (session, default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-structured-output", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-structured-output".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    let worker_agent = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("worker".to_string()),
            provider: Some("slow-structured".to_string()),
            model: Some("default".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }))
        .expect("worker agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    let (background_run_id, requested_records) = harness.with_app_mut(|app| {
        let background_run_id = launch_slow_structured_run(app, session.id(), default_agent.id());
        let requested_run_id = launch_slow_structured_run(app, session.id(), worker_agent.id());
        app.providers_mut()
            .push_finished_structured_output_poll_for_test(
                background_run_id.clone(),
                Ok(Some(ProviderPromptSignalBatch {
                    chunks: vec![ProviderPromptChunk {
                        kind: TerminalOutputKind::ProviderOutput,
                        merge_key: Some("background-chunk".to_string()),
                        bytes: b"background-run-output\n".to_vec(),
                    }],
                    ..ProviderPromptSignalBatch::default()
                })),
            );

        let recipient_attachment_ids = app.attachments().list_session_attachment_ids(session.id());
        let requested_records = ProviderOutputPump::new(app)
            .pump_provider_output(ProviderOutputPumpRequest {
                session_id: session.id(),
                provider_run_id: &requested_run_id,
                recipient_attachment_ids,
                initial_liveness_already_checked: false,
            })
            .expect("requested run pump should drain all finished structured jobs");
        (background_run_id, requested_records)
    });

    assert!(
        requested_records.is_empty(),
        "background run output should be buffered for recipients, not returned as requested-run output"
    );
    let buffered_records = harness.with_app_mut(|app| {
        app.terminal_mut()
            .drain_output_records(session.id(), attachment.id())
    });
    assert_eq!(buffered_records.len(), 1);
    assert_eq!(buffered_records[0].provider_run_id, background_run_id);
    assert_eq!(buffered_records[0].bytes, b"background-run-output\n");
}

#[test]
fn terminal_output_drain_survives_missing_focused_provider_run() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-1",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    app.terminal_mut().fan_out_output(
        session.id(),
        "provider-run-stale",
        Some(default_agent.id()),
        crate::terminal::TerminalOutputKind::ProviderOutput,
        None,
        vec![attachment.id().to_string()],
        b"late output\n",
    );

    let records = crate::app::provider_output::pump_terminal_output_for_attachment(
        &mut app,
        session.id(),
        attachment.id(),
    )
    .expect("draining buffered output should not require an active focused provider run");

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].bytes, b"late output\n");
}

#[test]
fn subscription_watch_can_skip_snapshot_while_draining_terminal_output() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-subscription-output",
            "worktree-subscription-output",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-subscription-output",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    app.terminal_mut().fan_out_output(
        session.id(),
        "provider-run-output-only",
        Some(default_agent.id()),
        crate::terminal::TerminalOutputKind::ProviderOutput,
        None,
        vec![attachment.id().to_string()],
        b"output-only update\n",
    );

    let result = crate::runtime_transport::watch_subscription_state(
        &mut app,
        session.id(),
        attachment.id(),
        false,
        None,
        0,
    );

    match result {
        crate::runtime_transport::WatchResult::Ok {
            records, snapshot, ..
        } => {
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].bytes, b"output-only update\n");
            assert!(
                snapshot.is_none(),
                "terminal-output-only watch should not build a session snapshot"
            );
        }
        crate::runtime_transport::WatchResult::Unavailable(message) => {
            panic!("subscription watch should stay available: {message}");
        }
    }
}

#[test]
fn compatibility_output_pump_reaps_first_output_timeout() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-first-output-timeout",
            "worktree-first-output-timeout",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-first-output-timeout",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = app
        .launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "slow-structured",
                "default",
                "default",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider should launch");
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "start but never answer\n",
        Vec::new(),
    )
    .expect("prompt should start");
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());
    let prompt_id = app
        .sessions()
        .get_session(session.id())
        .expect("session should exist")
        .active_prompt_for_agent(agent.id())
        .expect("active prompt should exist")
        .id()
        .to_string();
    let mut timed_out_turn = crate::app::ActiveTurnState::new(
        session.id().to_string(),
        agent.id().to_string(),
        prompt_id,
        run.id().to_string(),
    )
    .with_phase(crate::app::ActiveTurnPhase::AwaitingFirstOutput);
    timed_out_turn.started_at_ms = crate::session::unix_epoch_ms().saturating_sub(11 * 60 * 1000);
    app.active_turn_store().start(timed_out_turn);

    let records = crate::app::provider_output::pump_terminal_output_for_attachment(
        &mut app,
        session.id(),
        attachment.id(),
    )
    .expect("compatibility pump should reap silent timeout");

    assert!(
        records.is_empty(),
        "first-output timeout emits a runtime notice, not provider output"
    );
    let session_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert!(
        session_state.active_prompt_for_agent(agent.id()).is_none(),
        "silent provider timeout must close the active prompt"
    );
    let run = app
        .providers()
        .get_run(run.id())
        .expect("provider run should still exist");
    assert!(
        run.terminal_diagnostic()
            .expect("timeout diagnostic should be recorded")
            .contains("Provider prompt produced no output")
    );
    let notices = app
        .terminal_mut()
        .drain_notice_records(session.id(), attachment.id());
    assert!(
        notices.iter().any(|record| record
            .message
            .contains("Provider prompt produced no output")),
        "timeout diagnostic should be visible to attached clients"
    );
}
