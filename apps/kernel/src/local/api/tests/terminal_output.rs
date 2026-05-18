use super::*;

fn launch_slow_structured_run(app: &mut DaemonApp, session_id: &str, agent_id: &str) -> String {
    app.launch_provider(
        LaunchProviderRequest::new(
            session_id,
            "dev-stub",
            "slow-structured",
            "default",
            "default",
        )
        .with_agent_id(agent_id),
    )
    .expect("slow structured provider run should launch")
    .id()
    .to_string()
}

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
fn append_native_provider_output_fans_out_and_records_history() {
    let harness = LocalRouterTestHarness::new();
    let (session, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attachment should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    let provider_run_id = harness.with_app_mut(|app| {
        app.launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "slow-structured",
                "default",
                "default",
            )
            .with_agent_id(agent.id())
            .with_client_interface(ProviderClientInterface::NativeTui),
        )
        .expect("native provider run should launch")
        .id()
        .to_string()
    });

    let records = match harness
        .dispatch(LocalDaemonRequest::AppendNativeProviderOutput(
            super::AppendNativeProviderOutputRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                provider_run_id: provider_run_id.clone(),
                kind: TerminalOutputKind::ProviderOutput,
                merge_key: Some("native-output".to_string()),
                text: "hello from native tui\n".to_string(),
            },
        ))
        .expect("native provider output should append")
    {
        LocalDaemonResponse::TerminalOutput { records } => records,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].bytes, b"hello from native tui\n");
    let history = harness
        .with_app(|app| app.load_session_history_entries(&session, Some(agent.id())))
        .expect("history should load");
    assert!(history.iter().any(|entry| {
        entry.provider_run_id.as_deref() == Some(provider_run_id.as_str())
            && entry.text.contains("hello from native tui")
    }));
}

#[test]
fn terminal_output_drain_streams_parallel_agent_prompts_for_same_attachment() {
    let harness = LocalRouterTestHarness::new();
    let (session, default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attachment should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    let spawned = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("parallel".to_string()),
            provider: Some("dev-stub".to_string()),
            model: Some("claude-code".to_string()),
            effort: Some("default".to_string()),
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
        }))
        .expect("spawn should succeed")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    let (default_run_id, spawned_run_id) = harness.with_app_mut(|app| {
        (
            launch_slow_structured_run(app, session.id(), default_agent.id()),
            launch_slow_structured_run(app, session.id(), spawned.id()),
        )
    });

    for agent_id in [default_agent.id(), spawned.id()] {
        match harness
            .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                target_agent_id: Some(agent_id.to_string()),
                prompt: format!("parallel prompt for {agent_id}\n"),
                attachments: Vec::new(),
            }))
            .expect("prompt should start")
        {
            LocalDaemonResponse::PromptSubmitted {
                outcome: PromptSubmissionOutcome::Started { .. },
                ..
            } => {}
            _ => panic!("unexpected local response"),
        }
    }

    harness.with_app_mut(|app| {
        for (provider_run_id, agent_id) in [
            (default_run_id.clone(), default_agent.id().to_string()),
            (spawned_run_id.clone(), spawned.id().to_string()),
        ] {
            app.providers_mut()
                .push_finished_structured_output_poll_for_test(
                    provider_run_id,
                    Ok(Some(ProviderPromptSignalBatch {
                        chunks: vec![ProviderPromptChunk {
                            kind: TerminalOutputKind::ProviderOutput,
                            merge_key: Some(format!("parallel-{agent_id}")),
                            bytes: format!("parallel output for {agent_id}\n").into_bytes(),
                        }],
                        ..ProviderPromptSignalBatch::default()
                    })),
                );
        }
    });

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut seen_agents = std::collections::BTreeSet::new();
    while Instant::now() < deadline && seen_agents.len() < 2 {
        let records = harness.with_app_mut(|app| {
            crate::app::provider_output::pump_terminal_output_for_attachment(
                app,
                session.id(),
                attachment.id(),
            )
            .expect("terminal output should keep pumping")
        });
        for record in records {
            if let Some(agent_id) = record.agent_id {
                seen_agents.insert(agent_id);
            }
        }
        thread::sleep(Duration::from_millis(10));
    }

    assert!(
        seen_agents.contains(default_agent.id()) && seen_agents.contains(spawned.id()),
        "expected output from both active agent prompts, saw {:?}",
        seen_agents
    );
}
