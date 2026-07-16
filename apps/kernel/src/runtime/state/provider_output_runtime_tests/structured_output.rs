use super::*;

#[test]
fn publication_output_rejects_opencode_model_substitution_before_run_mutation() {
    let request = crate::provider::LaunchProviderRequest::new(
        "session-publication-model-lock",
        "opencode",
        "opencode",
        "default",
        "opencode/gpt-5.2",
    );
    let run = crate::provider::RuntimeProviderRun::new(
        "provider-run-publication-model-lock",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-opencode-publication-model-lock".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-opencode-runtime".to_string()),
        },
    );
    let mut batch = crate::provider::ProviderPromptSignalBatch {
        resolved_model: Some("opencode/big-pickle".to_string()),
        resolved_model_source: Some("message.updated"),
        ..crate::provider::ProviderPromptSignalBatch::default()
    };

    let failure = super::super::structured_provider_output_runtime::reject_workflow_publication_opencode_model_substitution(
        true,
        &run,
        &mut batch,
    )
    .expect("publication model drift should be rejected");

    assert!(failure.contains("substitution is disabled"));
    assert_eq!(run.model(), "opencode/gpt-5.2");
    assert_eq!(batch.resolved_model, None);
    assert_eq!(batch.resolved_model_source, None);
    assert_eq!(batch.terminal_failure.as_deref(), Some(failure.as_str()));
    assert!(batch.prompt_completed);
}

#[test]
fn interactive_output_keeps_opencode_selection_sync_behavior() {
    let request = crate::provider::LaunchProviderRequest::new(
        "session-interactive-model-sync",
        "opencode",
        "opencode",
        "default",
        "opencode/gpt-5.2",
    );
    let run = crate::provider::RuntimeProviderRun::new(
        "provider-run-interactive-model-sync",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-opencode-interactive-model-sync".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-opencode-runtime".to_string()),
        },
    );
    let mut batch = crate::provider::ProviderPromptSignalBatch {
        resolved_model: Some("opencode/big-pickle".to_string()),
        resolved_model_source: Some("message.updated"),
        ..crate::provider::ProviderPromptSignalBatch::default()
    };

    assert_eq!(
        super::super::structured_provider_output_runtime::reject_workflow_publication_opencode_model_substitution(
            false,
            &run,
            &mut batch,
        ),
        None,
    );
    assert_eq!(batch.resolved_model.as_deref(), Some("opencode/big-pickle"));
    assert_eq!(batch.terminal_failure, None);
}

async fn assert_owned_output_pump_drains_pending_record_after_run_state_change(
    state: crate::provider::ProviderRunState,
) {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-pending-structured-output",
            "worktree-pending-structured-output",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-pending-structured-output",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "opencode",
        "opencode",
        "default",
        "zen",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-pending-structured-output",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-opencode-pending-output".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-opencode-runtime".to_string()),
        },
    );
    match state {
        crate::provider::ProviderRunState::Parked => run.mark_parked(),
        crate::provider::ProviderRunState::Ended => run.mark_ended(),
        other => panic!("unsupported terminal test state: {other:?}"),
    }
    app.providers_mut().insert_run_for_test(run.clone());
    app.update_provider_run_projection(run.clone());
    let expected = crate::terminal::TerminalOutputRecord {
        record_id: None,
        timestamp_ms: 1_000,
        session_id: session.id().to_string(),
        provider_run_id: run.id().to_string(),
        agent_id: Some(agent.id().to_string()),
        prompt_id: None,
        prompt_origin: None,
        source_attachment_id: None,
        kind: crate::terminal::TerminalOutputKind::ProviderOutput,
        merge_key: None,
        recipient_attachment_ids: vec![attachment.id().to_string()],
        pending_recipient_attachment_ids: vec![attachment.id().to_string()],
        bytes: b"completed output".to_vec(),
        external_observation_metadata: None,
    };
    let output_store = app.structured_output_record_store();
    output_store.append(run.id().to_string(), vec![expected.clone()]);
    output_store.schedule_next_poll(run.id().to_string(), 2_000);

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let records = runtime
        .pump_owned_provider_output(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            true,
        )
        .await
        .expect("owned provider output pump should succeed");

    assert_eq!(records, vec![expected], "state was {state:?}");
    assert_eq!(output_store.poll_due_at_ms(run.id()), None);
    assert!(output_store.take(run.id()).is_empty());
}

#[tokio::test]
async fn owned_output_pump_drains_completed_pending_output_after_run_quiesces() {
    assert_owned_output_pump_drains_pending_record_after_run_state_change(
        crate::provider::ProviderRunState::Parked,
    )
    .await;
    assert_owned_output_pump_drains_pending_record_after_run_state_change(
        crate::provider::ProviderRunState::Ended,
    )
    .await;
}

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
    runtime.start_active_turn_with_trace_id(
        session.id(),
        agent.id(),
        active_prompt.id(),
        run.id(),
        "trace-structured-history-turn",
    );
    let active_turn = runtime
        .owned
        .active_turns
        .get(run.id())
        .expect("active turn should be tracked");
    assert_eq!(
        active_turn.source_attachment_id.as_deref(),
        Some(active_prompt.source_attachment_id())
    );
    assert_eq!(
        active_turn.prompt_origin,
        Some(active_prompt.prompt_origin())
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
async fn active_turn_trace_metadata_uses_prompt_owner_when_session_mirror_is_stale() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-stale-active-turn",
            "worktree-stale-active-turn",
        ))
        .expect("session should be created");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "codex",
                "default",
                "gpt-test",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");
    let external_prompt = crate::session::PromptQueueItem::external_observed_running(
        "codex",
        "codex-thread-stale-active-turn",
        "codex-turn-stale-active-turn",
        agent.id(),
        "external prompt with owner-only metadata",
    );
    let external_prompt_id = external_prompt.id().to_string();
    app.prompt_owner_sync_external_active_prompt(session.id(), agent.id(), Some(external_prompt))
        .expect("external active prompt should sync");
    app.sessions_mut()
        .mirror_agent_prompt_state(
            session.id(),
            agent.id(),
            None,
            std::collections::VecDeque::new(),
        )
        .expect("test drift should clear stale session prompt mirror");
    assert!(
        app.sessions()
            .get_session(session.id())
            .expect("session should load")
            .active_prompt_for_agent(agent.id())
            .is_none(),
        "session mirror should not expose the active prompt"
    );

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime.start_active_turn_with_trace_id(
        session.id(),
        agent.id(),
        &external_prompt_id,
        run.id(),
        "trace-stale-active-turn",
    );

    let active_turn = runtime
        .owned
        .active_turns
        .get(run.id())
        .expect("active turn should be tracked");
    assert_eq!(
        active_turn.source_attachment_id.as_deref(),
        Some("external:codex")
    );
    assert_eq!(
        active_turn.prompt_origin,
        Some(crate::session::PromptOrigin::External)
    );
    let external_observed_id = active_turn
        .external_observed_id
        .expect("external observed id should come from prompt owner");
    assert_eq!(external_observed_id.provider, "codex");
    assert_eq!(
        external_observed_id.provider_session_id,
        "codex-thread-stale-active-turn"
    );
    assert_eq!(
        external_observed_id.provider_turn_id,
        "codex-turn-stale-active-turn"
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
