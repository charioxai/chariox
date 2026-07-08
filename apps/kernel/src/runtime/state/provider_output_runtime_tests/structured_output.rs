use super::*;

#[tokio::test]
async fn structured_output_batch_fans_out_chunks_with_one_terminal_notification() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-structured-batch",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");
    app.update_provider_run_projection(run.clone());
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "status\n",
        Vec::new(),
    )
    .expect("prompt should start");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let terminal = runtime.owned.terminal_stream.clone();
    let before = terminal.attachment_change_sequence(session.id(), attachment.id());
    let records = runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![
                    crate::provider::ProviderPromptChunk {
                        kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                        merge_key: Some("structured-batch-1".to_string()),
                        bytes: b"first".to_vec(),
                    },
                    crate::provider::ProviderPromptChunk {
                        kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                        merge_key: Some("structured-batch-2".to_string()),
                        bytes: vec![0xff, b's', b'e', b'c', b'o', b'n', b'd'],
                    },
                ],
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("structured output batch should be accepted");

    assert_eq!(records.len(), 2);
    assert_eq!(records[0].bytes, b"first");
    assert_eq!(
        records[1].bytes,
        vec![0xff, b's', b'e', b'c', b'o', b'n', b'd']
    );
    assert_eq!(
        terminal.attachment_change_sequence(session.id(), attachment.id()),
        before + 1,
        "structured output chunks should use one terminal batch notification"
    );
}

#[tokio::test]
async fn structured_output_batch_persists_one_turn_id_for_all_chunks() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-structured-history-turn",
            "worktree-structured-history-turn",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-structured-history-turn",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");
    app.update_provider_run_projection(run.clone());
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "status\n",
        Vec::new(),
    )
    .expect("prompt should start");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let active_prompt = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should load")
        .active_prompt_for_agent(agent.id())
        .expect("active prompt should exist")
        .clone();
    runtime.owned.active_turns.start(
        crate::app::ActiveTurnState::new(
            session.id().to_string(),
            agent.id().to_string(),
            active_prompt.id().to_string(),
            run.id().to_string(),
        )
        .with_trace_id("trace-structured-history-turn"),
    );

    let records = runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![
                    crate::provider::ProviderPromptChunk {
                        kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                        merge_key: Some("structured-history-turn-1".to_string()),
                        bytes: b"first".to_vec(),
                    },
                    crate::provider::ProviderPromptChunk {
                        kind: crate::terminal::TerminalOutputKind::ProviderReasoning,
                        merge_key: Some("structured-history-turn-2".to_string()),
                        bytes: b"second".to_vec(),
                    },
                ],
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("structured output batch should be accepted");
    assert_eq!(records.len(), 2);

    let events = runtime
        .owned
        .operational_history_store
        .load_session_events(session.id(), Some(agent.id()))
        .expect("canonical operational events should load");
    let chunk_turn_ids = ["structured-history-turn-1", "structured-history-turn-2"]
        .into_iter()
        .map(|merge_key| {
            events
                .iter()
                .find(|event| {
                    event
                        .metadata
                        .get("merge_key")
                        .and_then(|value| value.as_str())
                        == Some(merge_key)
                })
                .unwrap_or_else(|| panic!("event for merge key {merge_key} should exist"))
                .turn_id
                .as_deref()
                .map(str::to_string)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        chunk_turn_ids,
        vec![
            Some("trace-structured-history-turn".to_string()),
            Some("trace-structured-history-turn".to_string())
        ]
    );
}

#[tokio::test]
async fn pty_output_pump_batches_chunks_with_one_terminal_notification() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-pty-batch",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "dev-stub",
        "claude-code",
        "default",
        "sonnet",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-pty-batch",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "dev-stub:pty-batch".to_string(),
            pty_target: Some("stub-pty:pty-batch".to_string()),
            pty_program: Some("/bin/sh".to_string()),
            pty_args: vec![
                "-lc".to_string(),
                "printf pty-one; sleep 0.05; printf pty-two; sleep 5".to_string(),
            ],
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.pty_mut()
        .spawn_for_run(&run)
        .expect("pty-backed provider run should spawn");

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let terminal = runtime.owned.terminal_stream.clone();
    let before = terminal.attachment_change_sequence(session.id(), attachment.id());
    let records = runtime
        .pump_owned_provider_output(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            true,
        )
        .await
        .expect("pty output pump should accept batched chunks");

    assert!(!records.is_empty(), "PTY output pump should return records");
    let output = records
        .iter()
        .flat_map(|record| record.bytes.clone())
        .collect::<Vec<u8>>();
    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("pty-one"));
    assert!(output.contains("pty-two"));
    assert_eq!(
        terminal.attachment_change_sequence(session.id(), attachment.id()),
        before + 1,
        "PTY output chunks should use one terminal batch notification"
    );

    let mut app = app.lock().await;
    let _ = app.pty_mut().remove_process(run.id());
}
