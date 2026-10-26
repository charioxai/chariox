use super::*;

#[tokio::test]
async fn panes_stream_ready_structured_output_survives_an_unrelated_app_lock() {
    // MP-08 / MP-10: a slow inventory/maintenance request must not hold an
    // already-read official-provider batch before its terminal record exists.
    let worktree = crate::test_support::TestWorktree::new("panes-stream-app-lock");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "panes-stream-lock-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "opencode",
        "opencode",
        "default",
        "opencode-go/gpt-6-luna",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-panes-stream-app-lock",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "panes-stream-lock-fixture".into(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: Default::default(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("http://127.0.0.1:1".into()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.update_provider_run_projection(run.clone());
    spawn_inert_pty_for_run(&mut app, run.id());
    app.structured_output_record_store()
        .mark_poll_enqueued(run.id(), None);
    app.providers_mut()
        .push_finished_structured_output_poll_for_test(
            run.id().to_string(),
            Ok(Some(crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                    merge_key: Some("text-1".into()),
                    bytes: b"CHECK:".to_vec(),
                }],
                ..Default::default()
            })),
        );
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let runtime_for_watch = runtime.clone();
    let terminal = runtime.owned.terminal_stream.clone();
    let before = terminal.attachment_change_sequence(session.id(), attachment.id());
    let cleanup = InertPtyCleanup {
        app: app.clone(),
        provider_run_id: run.id().to_string(),
    };
    let held = app.lock().await;
    let session_id = session.id().to_string();
    let run_id = run.id().to_string();
    let recipient = attachment.id().to_string();
    let pump = tokio::spawn(async move {
        runtime
            .pump_owned_provider_output(&session_id, &run_id, vec![recipient], false)
            .await
    });
    let ready = tokio::time::timeout(std::time::Duration::from_millis(150), async {
        while terminal.attachment_change_sequence(session.id(), attachment.id()) == before {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    })
    .await;
    let delivered = tokio::time::timeout(
        std::time::Duration::from_millis(150),
        runtime_for_watch.watch_relay_subscription_state(
            session.id(),
            attachment.id(),
            false,
            None,
            0,
        ),
    )
    .await;
    drop(held);
    let records = pump.await.unwrap().unwrap();
    drop(cleanup);
    assert!(records.iter().any(|record| record.bytes == b"CHECK:"));
    assert!(
        ready.is_ok(),
        "ready provider output waited for an unrelated app lock"
    );
    assert!(
        matches!(delivered,
            Ok(crate::runtime_transport::WatchResult::Ok { records, .. })
                if records.iter().any(|record| record.bytes == b"CHECK:")
        ),
        "terminal subscriber waited for an unrelated app lock after the record was ready"
    );
}

#[tokio::test]
async fn panes_stream_terminal_delivery_does_not_wait_for_history_writer() {
    // MP-08 / MP-10: ready-output jobs share a task. A synchronous history
    // wait must not stop that task from delivering an already-published record.
    let worktree = crate::test_support::TestWorktree::new("panes-stream-history-lock");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "panes-stream-history-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "opencode",
        "opencode",
        "default",
        "opencode-go/gpt-6-luna",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-panes-stream-history-lock",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "panes-stream-history-fixture".into(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: Default::default(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("http://127.0.0.1:1".into()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.update_provider_run_projection(run.clone());
    spawn_inert_pty_for_run(&mut app, run.id());
    app.structured_output_record_store()
        .mark_poll_enqueued(run.id(), None);
    app.providers_mut()
        .push_finished_structured_output_poll_for_test(
            run.id().to_string(),
            Ok(Some(crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                    merge_key: Some("text-1".into()),
                    bytes: b"CHECK:".to_vec(),
                }],
                ..Default::default()
            })),
        );
    let history_store = app.history_store();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;

    let append_lock = runtime.owned.transcript_history_append_lock.clone();
    let (locked_tx, locked_rx) = tokio::sync::oneshot::channel();
    let holder = std::thread::spawn(move || {
        let _held = append_lock.lock().unwrap();
        locked_tx.send(()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(350));
    });
    locked_rx.await.unwrap();
    let started = std::time::Instant::now();
    let terminal = runtime.owned.terminal_stream.clone();
    let before = terminal.attachment_change_sequence(session.id(), attachment.id());
    let cleanup = InertPtyCleanup {
        app: app.clone(),
        provider_run_id: run.id().to_string(),
    };
    let (output, delivery_ms) = tokio::join!(
        runtime.pump_owned_provider_output(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            false,
        ),
        async {
            while terminal.attachment_change_sequence(session.id(), attachment.id()) == before {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
            let delivered = runtime
                .watch_relay_subscription_state(session.id(), attachment.id(), false, None, 0)
                .await;
            assert!(matches!(delivered,
                crate::runtime_transport::WatchResult::Ok { records, .. }
                    if records.iter().any(|record| record.bytes == b"CHECK:")
            ));
            started.elapsed().as_millis()
        },
    );
    // MP-08 / MP-10: an attaching client must not read history before an
    // already-published asynchronous write becomes durable.
    let history_runtime = runtime.clone();
    let history_session = session.id().to_string();
    let history_agent = agent.id().to_string();
    let reading = tokio::spawn(async move {
        crate::runtime::history_executor::execute_history_request(
            history_store,
            history_runtime.owned.operational_history_store.clone(),
            &history_runtime,
            &history_runtime.owned.config_projection,
            crate::local::LocalDaemonRequest::GetSessionHistoryOutline(
                crate::local::GetSessionHistoryOutlineRequest {
                    session_id: history_session,
                    agent_ids: Some(vec![history_agent]),
                    latest_prompt_count: Some(4),
                    cursor: None,
                    include_prefetched_content: true,
                },
            ),
        )
        .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let read_waited = !reading.is_finished();
    holder.join().unwrap();
    reading.await.unwrap().unwrap();
    assert!(
        read_waited,
        "MP-08 / MP-10: initial history raced a published pending write"
    );
    drop(cleanup);
    assert!(output
        .unwrap()
        .iter()
        .any(|record| record.bytes == b"CHECK:"));
    assert!(
        delivery_ms < 150,
        "MP-08 / MP-10: ready terminal record waited {delivery_ms} ms for history persistence"
    );
}

#[tokio::test]
async fn panes_stream_history_handoff_keeps_the_published_turn_identity() {
    // MP-08 / MP-10: cancellation or replacement during the blocking handoff
    // must not attribute the previous output to the replacement prompt.
    let worktree = crate::test_support::TestWorktree::new("panes-stream-history-turn");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let run_id = "panes-stream-history-turn-run";
    runtime
        .owned
        .active_turns
        .start(crate::app::ActiveTurnState::new(
            session.id().into(),
            agent.id().into(),
            "published-prompt".into(),
            run_id.into(),
        ));
    let prepared = runtime.owned.prepare_history_entries(
        session.id(),
        vec![crate::history::SessionHistoryEntry::provider_output(
            session.id(),
            run_id,
            Some(agent.id()),
            crate::terminal::TerminalOutputKind::ProviderOutput,
            Some("published-text".into()),
            "published before replacement",
        )],
    );
    runtime
        .owned
        .active_turns
        .start(crate::app::ActiveTurnState::new(
            session.id().into(),
            agent.id().into(),
            "replacement-prompt".into(),
            run_id.into(),
        ));
    let history_owned = runtime.owned.clone();
    tokio::task::spawn_blocking(move || {
        history_owned.append_prepared_history_entries(prepared);
    })
    .await
    .unwrap();
    let events = runtime
        .owned
        .operational_history_store
        .load_session_events(session.id(), Some(agent.id()))
        .unwrap();
    let event = events
        .iter()
        .find(|event| event.content.as_deref() == Some("published before replacement"))
        .unwrap();
    assert_eq!(event.prompt_id.as_deref(), Some("published-prompt"));
    assert_ne!(event.prompt_id.as_deref(), Some("replacement-prompt"));
}

#[tokio::test]
async fn panes_stream_next_ready_batch_does_not_wait_for_previous_history_ack() {
    // MP-08 / MP-10: first words may span batches. Busy persistence must not
    // freeze the second batch; completion still waits for both history writes.
    let worktree = crate::test_support::TestWorktree::new("panes-stream-history-two-batches");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "panes-stream-history-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "opencode",
        "opencode",
        "default",
        "opencode-go/gpt-6-luna",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-panes-stream-history-two-batches",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "panes-stream-history-fixture".into(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: Default::default(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("http://127.0.0.1:1".into()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.update_provider_run_projection(run.clone());
    spawn_inert_pty_for_run(&mut app, run.id());
    app.structured_output_record_store()
        .mark_poll_enqueued(run.id(), None);
    app.providers_mut()
        .push_finished_structured_output_poll_for_test(
            run.id().to_string(),
            Ok(Some(crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                    merge_key: Some("text-1".into()),
                    bytes: b"CHECK:".to_vec(),
                }],
                ..Default::default()
            })),
        );
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;

    let append_lock = runtime.owned.transcript_history_append_lock.clone();
    let (locked_tx, locked_rx) = tokio::sync::oneshot::channel();
    let holder = std::thread::spawn(move || {
        let _held = append_lock.lock().unwrap();
        locked_tx.send(()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(350));
    });
    locked_rx.await.unwrap();
    let started = std::time::Instant::now();
    let cleanup = InertPtyCleanup {
        app: app.clone(),
        provider_run_id: run.id().into(),
    };
    let (output, delivery_ms) = tokio::join!(
        async {
            let mut records = Vec::new();
            for bytes in [b"CHECK".as_slice(), b":".as_slice()] {
                records.extend(
                    runtime
                        .apply_owned_structured_output_batch(
                            session.id(),
                            run.id(),
                            vec![attachment.id().into()],
                            crate::provider::ProviderPromptSignalBatch {
                                chunks: vec![crate::provider::ProviderPromptChunk {
                                    kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                                    merge_key: Some("two-batch-text".into()),
                                    bytes: bytes.to_vec(),
                                }],
                                ..Default::default()
                            },
                        )
                        .await
                        .unwrap(),
                );
            }
            runtime
                .apply_owned_structured_output_batch(
                    session.id(),
                    run.id(),
                    vec![attachment.id().into()],
                    crate::provider::ProviderPromptSignalBatch {
                        prompt_completed: true,
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            records
        },
        async {
            let mut prefix = Vec::new();
            loop {
                if let crate::runtime_transport::WatchResult::Ok { records, .. } = runtime
                    .watch_relay_subscription_state(session.id(), attachment.id(), false, None, 0)
                    .await
                {
                    for record in records {
                        prefix.extend(record.bytes);
                    }
                }
                if prefix == b"CHECK:" {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
            started.elapsed().as_millis()
        },
    );
    holder.join().unwrap();
    assert_eq!(
        output.into_iter().flat_map(|r| r.bytes).collect::<Vec<_>>(),
        b"CHECK:"
    );
    let events = runtime
        .owned
        .operational_history_store
        .load_session_history_entries(session.id(), Some(agent.id()))
        .unwrap();
    assert_eq!(
        events.iter().map(|e| e.text.as_str()).collect::<String>(),
        "CHECK:",
        "completion must retain both history chunks in order"
    );
    drop(cleanup);
    assert!(
        delivery_ms < 150,
        "MP-08 / MP-10: second ready batch waited {delivery_ms} ms for history ack"
    );
}

#[tokio::test]
async fn panes_stream_batch_reaches_an_attachment_registered_after_the_recipient_snapshot() {
    // MP-08 / MP-10: a view attached while the pump waited for its run lane reads
    // history that cannot yet contain the batch; it must receive the batch live.
    let worktree = crate::test_support::TestWorktree::new("panes-stream-late-attachment");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let late = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "panes-stream-late-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "opencode",
        "opencode",
        "default",
        "opencode-go/gpt-6-luna",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-panes-stream-late-attachment",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "panes-stream-late-fixture".into(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: Default::default(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("http://127.0.0.1:1".into()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.update_provider_run_projection(run.clone());
    spawn_inert_pty_for_run(&mut app, run.id());
    app.structured_output_record_store()
        .mark_poll_enqueued(run.id(), None);
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let _cleanup = InertPtyCleanup {
        app: app.clone(),
        provider_run_id: run.id().into(),
    };
    // The pump took its recipients before `late` attached.
    let stale_recipients = Vec::new();
    let records = runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            stale_recipients,
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                    merge_key: Some("late-text".into()),
                    bytes: b"LATE".to_vec(),
                }],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        records
            .into_iter()
            .flat_map(|r| r.bytes)
            .collect::<Vec<_>>(),
        b"LATE"
    );
    let crate::runtime_transport::WatchResult::Ok { records, .. } = runtime
        .watch_relay_subscription_state(session.id(), late.id(), false, None, 0)
        .await
    else {
        panic!("late attachment cannot watch");
    };
    assert_eq!(
        records
            .into_iter()
            .flat_map(|r| r.bytes)
            .collect::<Vec<_>>(),
        b"LATE"
    );
}

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
    let worktree = crate::test_support::TestWorktree::new("structured-output-pending");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
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
        sequence: None,
        sequence_end: None,
        sequence_byte_lengths: Vec::new(),
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
async fn live_structured_poll_failures_are_retried_then_surfaced() {
    let worktree = crate::test_support::TestWorktree::new("structured-output-poll-retry");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.6-luna",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-structured-poll-retry",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-codex-structured-poll-retry".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-codex-runtime".to_string()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());
    let output_store = app.structured_output_record_store();
    output_store.mark_poll_enqueued(run.id(), None);
    app.providers_mut()
        .push_finished_structured_output_poll_for_test(
            run.id().to_string(),
            Err(crate::error::DaemonError::ProviderProtocol {
                provider_run_id: run.id().to_string(),
                operation: "thread/turns/list",
                message: "new rollout is temporarily empty".to_string(),
            }),
        );

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let records = runtime
        .pump_owned_structured_provider_output(session.id(), run.id(), Vec::new())
        .await
        .expect("a live structured poll failure should be deferred");

    assert!(records.is_empty());
    assert!(
        output_store.poll_due_at_ms(run.id()).is_some(),
        "the live provider run must remain scheduled after a transient poll failure",
    );
    assert!(
        !output_store.poll_due(run.id(), crate::session::unix_epoch_ms()),
        "the retry should respect the empty-poll backoff",
    );

    for attempt in 2..=crate::app::provider_output::STRUCTURED_OUTPUT_POLL_FAILURE_RETRY_LIMIT {
        output_store.mark_poll_enqueued(run.id(), None);
        app.lock()
            .await
            .providers_mut()
            .push_finished_structured_output_poll_for_test(
                run.id().to_string(),
                Err(crate::error::DaemonError::ProviderProtocol {
                    provider_run_id: run.id().to_string(),
                    operation: "thread/turns/list",
                    message: "new rollout is temporarily empty".to_string(),
                }),
            );
        let result = runtime
            .pump_owned_structured_provider_output(session.id(), run.id(), Vec::new())
            .await;
        if attempt < crate::app::provider_output::STRUCTURED_OUTPUT_POLL_FAILURE_RETRY_LIMIT {
            assert!(result
                .expect("a transient structured poll failure should be deferred")
                .is_empty());
            assert!(output_store.poll_due_at_ms(run.id()).is_some());
        } else {
            assert!(matches!(
                result,
                Err(crate::error::DaemonError::ProviderProtocol { .. })
            ));
            assert!(output_store.poll_due_at_ms(run.id()).is_none());
            assert!(!output_store.poll_due(run.id(), u64::MAX));
        }
    }
}

#[tokio::test]
async fn structured_output_batch_fans_out_chunks_with_one_terminal_notification() {
    let worktree = crate::test_support::TestWorktree::new("structured-output-fanout");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
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
                completions: vec![crate::provider::ProviderAssistantCompletion {
                    message_id: "structured-batch-2".to_string(),
                    completed_at_ms: crate::session::unix_epoch_ms(),
                }],
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
        before + 2,
        "the output batch and its completion should each notify the attachment"
    );
    assert_eq!(
        terminal
            .drain_output_records(session.id(), attachment.id())
            .into_iter()
            .map(|record| record.bytes)
            .collect::<Vec<_>>(),
        vec![
            b"first".to_vec(),
            vec![0xff, b's', b'e', b'c', b'o', b'n', b'd']
        ]
    );
    assert_eq!(
        terminal
            .drain_completion_records(session.id(), attachment.id())
            .into_iter()
            .map(|completion| completion.message_id)
            .collect::<Vec<_>>(),
        vec!["structured-batch-2".to_string()]
    );
}

#[tokio::test]
async fn structured_output_usage_resolves_the_cloud_owners_local_account_authority() {
    let _environment = crate::env_lock::lock();
    let provider_home = crate::test_support::TestWorktree::new("synthetic-claude-account");
    std::env::set_var("HOME", provider_home.path());
    std::env::set_var("CHARIOX_HOME", provider_home.path().join(".chariox"));

    let worktree = crate::test_support::TestWorktree::new("structured-output-cloud-owner");
    let cloud_owner_user_id = "cloud-owner";
    let mut config = crate::config::DaemonConfig::for_tests();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        kernel_id: None,
        kernel_credential: None,
        kernel_public_key_thumbprint: None,
        user_id: cloud_owner_user_id.to_string(),
        ..Default::default()
    });
    let mut app = crate::test_support::bootstrap_authenticated_app(config)
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            worktree
                .session_request()
                .with_owner_user_id(cloud_owner_user_id),
        )
        .expect("cloud-owned session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::for_user(
            session.id(),
            "client-cloud-owner-usage",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
            cloud_owner_user_id,
        ))
        .expect("cloud owner should attach");
    let environment = app
        .provider_account_profile_registry()
        .resolve_environment(crate::session::DEFAULT_LOCAL_USER_ID, "claude", "default")
        .expect("local authority profile should resolve");
    std::fs::create_dir_all(provider_home.path().join(".claude")).unwrap();
    std::fs::write(
        environment
            .get("CLAUDE_CONFIG_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| provider_home.path().join(".claude"))
            .join(".credentials.json"),
        br#"{"claudeAiOauth":{"refreshToken":"synthetic-refresh-token"}}"#,
    )
    .unwrap();
    let run = app
        .launch_provider(
            // The account-authority mapping does not depend on the provider.
            // Codex avoids the Claude launch-credential gate, which needs a
            // vault or portable login this fixture does not have.
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "codex",
                "default",
                "gpt-5",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch through the local account authority");
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
    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                    merge_key: Some("cloud-owner-usage-output".to_string()),
                    bytes: b"continued output".to_vec(),
                }],
                account_usage: Some(crate::account_profile::ProviderAccountUsageSnapshot {
                    profile_id: "default".to_string(),
                    provider: "codex".to_string(),
                    availability:
                        crate::account_profile::ProviderAccountUsageAvailability::Available,
                    meters: Vec::new(),
                    observed_at_ms: Some(1_000),
                    source: "cloud-owner-usage-test".to_string(),
                    management_url: None,
                }),
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("usage should update without aborting the output batch");

    let profile = runtime
        .owned
        .provider_account_profiles
        .get(crate::session::DEFAULT_LOCAL_USER_ID, "codex", "default")
        .expect("the local account authority profile should remain resolvable");
    assert_eq!(profile.usage.source, "cloud-owner-usage-test");
    assert_eq!(
        runtime
            .owned
            .terminal_stream
            .drain_output_records(session.id(), attachment.id())
            .into_iter()
            .map(|record| record.bytes)
            .collect::<Vec<_>>(),
        vec![b"continued output".to_vec()]
    );
}

#[tokio::test]
async fn leased_run_usage_for_an_unregistered_home_profile_keeps_its_output() {
    // MP-08/MP-10: a leased worker run carries the home kernel's account
    // profile, which the worker registry does not contain. This kernel
    // registers no Claude profile at all.
    let worktree = crate::test_support::TestWorktree::new("structured-output-leased-usage");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-leased-usage",
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
    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                    merge_key: Some("leased-usage-output".to_string()),
                    bytes: b"capability loaded".to_vec(),
                }],
                account_usage: Some(crate::account_profile::ProviderAccountUsageSnapshot {
                    profile_id: "default".to_string(),
                    provider: "claude".to_string(),
                    availability:
                        crate::account_profile::ProviderAccountUsageAvailability::Available,
                    meters: Vec::new(),
                    observed_at_ms: Some(1_000),
                    source: "leased-usage-test".to_string(),
                    management_url: None,
                }),
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("unregistered usage must not abort the leased output batch");
    assert_eq!(
        runtime
            .owned
            .terminal_stream
            .drain_output_records(session.id(), attachment.id())
            .into_iter()
            .map(|record| record.bytes)
            .collect::<Vec<_>>(),
        vec![b"capability loaded".to_vec()]
    );
}

#[tokio::test]
async fn structured_output_and_completion_fanout_respect_collaborator_trace_visibility() {
    let worktree = crate::test_support::TestWorktree::new("structured-output-collaborator");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    {
        let mut sessions = app.sessions_mut();
        for (invite_id, user_id, level, now_ms) in [
            (
                "invite-transparent-trace-output",
                "user-transparent",
                crate::session::CollaborationLevel::Transparent,
                1,
            ),
            (
                "invite-full-trace-output",
                "user-full",
                crate::session::CollaborationLevel::Full,
                2,
            ),
            (
                "invite-private-trace-output",
                "user-private",
                crate::session::CollaborationLevel::Private,
                3,
            ),
        ] {
            let (_, invite) = sessions
                .create_session_invite(
                    session.id(),
                    invite_id.to_string(),
                    "local".to_string(),
                    None,
                    Some(1),
                    level,
                )
                .expect("collaborator invite should be created");
            sessions
                .join_session_invite(
                    session.id(),
                    invite.invite_id(),
                    user_id.to_string(),
                    now_ms,
                )
                .expect("collaborator should join");
        }
    }
    let owner_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::for_user(
            session.id(),
            "client-trace-owner",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
            "local",
        ))
        .expect("owner attachment should attach");
    let transparent_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::for_user(
            session.id(),
            "client-trace-transparent",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
            "user-transparent",
        ))
        .expect("transparent collaborator attachment should attach");
    let full_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::for_user(
            session.id(),
            "client-trace-full",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
            "user-full",
        ))
        .expect("full collaborator attachment should attach");
    let private_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::for_user(
            session.id(),
            "client-trace-private",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
            "user-private",
        ))
        .expect("private collaborator attachment should attach");
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
        owner_attachment.id(),
        Some(agent.id()),
        "status\n",
        Vec::new(),
    )
    .expect("prompt should start");

    let recipient_attachment_ids = vec![
        owner_attachment.id().to_string(),
        transparent_attachment.id().to_string(),
        full_attachment.id().to_string(),
        private_attachment.id().to_string(),
    ];
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let terminal = runtime.owned.terminal_stream.clone();
    for attachment_id in &recipient_attachment_ids {
        terminal.drain_output_records(session.id(), attachment_id);
    }
    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            recipient_attachment_ids.clone(),
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                    merge_key: Some("collaborator-semantic-output".to_string()),
                    bytes: b"semantic output".to_vec(),
                }],
                completions: vec![crate::provider::ProviderAssistantCompletion {
                    message_id: "collaborator-assistant-completion".to_string(),
                    completed_at_ms: 1_000,
                }],
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("structured output batch should be accepted");
    runtime.owned.fan_out_terminal_outputs_to_recipients(
        session.id(),
        recipient_attachment_ids,
        vec![
            super::super::prompt_transcript_owned_state::TerminalOutputBatchAppend {
                provider_run_id: run.id().to_string(),
                agent_id: Some(agent.id().to_string()),
                kind: crate::terminal::TerminalOutputKind::ProviderTerminal,
                merge_key: Some("collaborator-raw-terminal".to_string()),
                sequence: None,
                bytes: b"raw terminal paint".to_vec(),
            },
        ],
    );
    runtime.owned.echo_promoted_queued_prompt_to_attachments(
        session.id(),
        run.id(),
        "prompt-collaborator-trace-output",
        owner_attachment.id(),
        "shared owner prompt",
        &[],
    );

    let owner_records = terminal.drain_output_records(session.id(), owner_attachment.id());
    let transparent_records =
        terminal.drain_output_records(session.id(), transparent_attachment.id());
    let full_records = terminal.drain_output_records(session.id(), full_attachment.id());
    let private_records = terminal.drain_output_records(session.id(), private_attachment.id());

    assert_eq!(
        owner_records
            .iter()
            .map(|record| (record.kind.clone(), record.bytes.clone()))
            .collect::<Vec<_>>(),
        vec![
            (
                crate::terminal::TerminalOutputKind::ProviderOutput,
                b"semantic output".to_vec(),
            ),
            (
                crate::terminal::TerminalOutputKind::ProviderTerminal,
                b"raw terminal paint".to_vec(),
            ),
            (
                crate::terminal::TerminalOutputKind::PromptEcho,
                b"shared owner prompt\n".to_vec(),
            ),
        ]
    );
    for (label, records) in [("transparent", transparent_records), ("full", full_records)] {
        assert_eq!(records.len(), 2, "{label} collaborator output");
        assert_eq!(
            records[0].kind,
            crate::terminal::TerminalOutputKind::ProviderOutput,
            "{label} collaborator must receive semantic output"
        );
        assert_eq!(records[0].bytes, b"semantic output", "{label} output");
        assert_eq!(
            records[1].kind,
            crate::terminal::TerminalOutputKind::PromptEcho,
            "{label} collaborator must receive prompt echoes"
        );
        assert_eq!(records[1].bytes, b"shared owner prompt\n", "{label} echo");
    }
    assert!(
        private_records.is_empty(),
        "private collaborator must not receive another user's agent trace"
    );

    for (label, attachment_id, should_receive) in [
        ("owner", owner_attachment.id(), true),
        ("transparent", transparent_attachment.id(), true),
        ("full", full_attachment.id(), true),
        ("private", private_attachment.id(), false),
    ] {
        let completion_ids = terminal
            .drain_completion_records(session.id(), attachment_id)
            .into_iter()
            .map(|completion| completion.message_id)
            .collect::<Vec<_>>();
        let expected = if should_receive {
            vec!["collaborator-assistant-completion".to_string()]
        } else {
            Vec::new()
        };
        assert_eq!(
            completion_ids, expected,
            "{label} collaborator completion visibility"
        );
    }
}

#[tokio::test]
async fn structured_output_batch_persists_one_turn_id_for_all_chunks() {
    let worktree = crate::test_support::TestWorktree::new("structured-output-turn-id");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
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
    runtime.flush_provider_history(run.id()).await.unwrap();
    for record in &records {
        assert_eq!(
            record.prompt_id.as_deref(),
            Some(active_prompt.id()),
            "live provider chunks must identify their kernel-owned prompt"
        );
    }

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
    // MP-08 / MP-10 (panes R5): each live record carries the sequence of the
    // history row it was stored as, so clients order live and hydrated rows by
    // one key.
    let history_sequences = ["structured-history-turn-1", "structured-history-turn-2"]
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
                .map(|event| event.sequence)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        records
            .iter()
            .map(|record| record.sequence)
            .collect::<Vec<_>>(),
        history_sequences
    );
    assert!(history_sequences[0] < history_sequences[1]);
    // The per-attachment copies clients receive keep it too.
    assert_eq!(
        runtime
            .owned
            .terminal_stream
            .drain_output_records(session.id(), attachment.id())
            .iter()
            .filter(|record| record.kind != crate::terminal::TerminalOutputKind::PromptEcho)
            .map(|record| record.sequence)
            .collect::<Vec<_>>(),
        history_sequences
    );
}

#[tokio::test]
async fn active_turn_trace_metadata_uses_prompt_owner_when_session_mirror_is_stale() {
    let worktree = crate::test_support::TestWorktree::new("structured-output-stale-turn");
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
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
    let worktree = crate::test_support::TestWorktree::new("structured-output-pty");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
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
                "sleep 0.1; printf pty-one; sleep 0.05; printf pty-two; sleep 5".to_string(),
            ],
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.update_provider_run_projection(run.clone());
    app.pty_mut()
        .spawn_for_run(&run)
        .expect("pty-backed provider run should spawn");
    let pty_output_signal = app.pty_output_signal();
    let initial_pty_sequence = pty_output_signal.sequence();
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "status\n",
        Vec::new(),
    )
    .expect("prompt should start");

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while pty_output_signal.sequence() < initial_pty_sequence + 2 {
            let sequence = pty_output_signal.sequence();
            pty_output_signal.wait_for_change_after(sequence).await;
        }
    })
    .await
    .expect("both delayed PTY writes should be available before the batch pump");

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

#[tokio::test]
async fn idle_claude_native_tui_projects_startup_terminal_without_history() {
    let worktree = crate::test_support::TestWorktree::new("structured-output-claude-native");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-idle-claude-native-runtime",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude",
        "default",
        "sonnet",
    )
    .with_agent_id(agent.id())
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-idle-claude-native-runtime",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "claude:native-idle-runtime".to_string(),
            pty_target: Some("claude:native-idle-runtime".to_string()),
            pty_program: Some("/bin/sh".to_string()),
            pty_args: vec![
                "-c".to_string(),
                "printf '\\033[?2004hClaude Code\\n'; IFS= read -r ignored".to_string(),
            ],
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.update_provider_run_projection(run.clone());
    app.pty_mut()
        .spawn_for_run(&run)
        .expect("Claude native PTY should spawn");
    let history_count = app
        .load_session_history_entries(&session, Some(agent.id()))
        .expect("history should load")
        .len();

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let records = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let mut records = Vec::new();
        loop {
            records.extend(
                runtime
                    .pump_owned_provider_output(
                        session.id(),
                        run.id(),
                        vec![attachment.id().to_string()],
                        true,
                    )
                    .await
                    .expect("idle Claude native startup output should project"),
            );
            let output = records
                .iter()
                .flat_map(|record| record.bytes.clone())
                .collect::<Vec<_>>();
            let frame = b"\x1b[?2004hClaude Code";
            if output.windows(frame.len()).any(|bytes| bytes == frame) {
                break records;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Claude startup frame should reach the client");

    assert!(
        !records.is_empty(),
        "Claude startup frame should reach the client"
    );
    assert!(records
        .iter()
        .all(|record| record.kind == crate::terminal::TerminalOutputKind::ProviderTerminal));
    let output = records
        .iter()
        .flat_map(|record| record.bytes.clone())
        .collect::<Vec<_>>();
    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("Claude Code"));
    assert!(output.contains("\u{1b}[?2004h"));

    let mut app = app.lock().await;
    assert_eq!(
        app.load_session_history_entries(&session, Some(agent.id()))
            .expect("history should still load")
            .len(),
        history_count,
        "transient terminal paint must stay out of semantic history"
    );
    let _ = app.pty_mut().remove_process(run.id());
}

// MP-08 / MP-10 / MP-11: observed account usage is advisory, so a leased run's
// home account never has to be registered on the worker.
#[tokio::test]
async fn leased_output_usage_does_not_require_a_worker_account_registration() {
    for leased in [false, true] {
        let worktree = crate::test_support::TestWorktree::new("leased-output-usage");
        let mut app =
            DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).expect("bootstrap");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(worktree.session_request())
            .expect("session");
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude-p",
            "home-claude-only",
            "haiku",
        )
        .with_agent_id(agent.id());
        let mut run = crate::provider::RuntimeProviderRun::new(
            "provider-run-leased-usage",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::External,
                process_label: "leased-usage-test".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: Some("test-claude-runtime".to_string()),
            },
        );
        run.mark_running();
        app.providers_mut().insert_run_for_test(run.clone());
        app.update_provider_run_projection(run.clone());
        if leased {
            app.mark_leased_provider_run(run.id());
        }
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let batch = crate::provider::ProviderPromptSignalBatch {
            account_usage: Some(
                crate::account_profile::ProviderAccountUsageSnapshot::unavailable(
                    "home-claude-only",
                    "claude",
                ),
            ),
            resolved_resume_state: Some(
                crate::provider::ProviderResumeState::from_claude_session_id("worker-native"),
            ),
            ..Default::default()
        };
        let result = runtime
            .apply_owned_structured_output_batch(session.id(), run.id(), Vec::new(), batch)
            .await;
        // Observed usage is advisory metadata: an unregistered profile never
        // drops the run's native output, leased or ordinary.
        result.expect("unregistered account usage must not prevent native output or settlement");
        let run = runtime.owned.provider_store.get_run(run.id()).expect("run");
        assert_eq!(
            run.resume_state().claude_session_id(),
            Some("worker-native"),
            "leased: {leased}"
        );
    }
}

#[tokio::test]
async fn panes_stream_maintenance_cannot_strand_a_ready_sibling() {
    // MP-08 / MP-10: A waits for its lane while B has one finished poll and
    // emits nothing else. Maintenance must leave both signals to the scheduler.
    let worktree = crate::test_support::TestWorktree::new("panes-stream-maintenance-sibling");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, first_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let second_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(crate::agent::CreateAgentRequest::new(
            session.id(),
            "opencode",
        ))
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "sibling-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let mut run_ids = Vec::new();
    for (id, agent) in [("a-blocked", first_agent), ("b-ready", second_agent)] {
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "opencode",
            "opencode",
            "default",
            "opencode-go/gpt-6-luna",
        )
        .with_agent_id(agent.id());
        let mut run = crate::provider::RuntimeProviderRun::new(
            id,
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "panes-stream-sibling-fixture".into(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: Default::default(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: Some("http://127.0.0.1:1".into()),
            },
        );
        run.mark_running();
        app.providers_mut().insert_run_for_test(run.clone());
        app.update_provider_run_projection(run.clone());
        spawn_inert_pty_for_run(&mut app, id);
        app.structured_output_record_store()
            .mark_poll_enqueued(id, None);
        app.providers_mut()
            .push_finished_structured_output_poll_for_test(
                id.into(),
                Ok(Some(crate::provider::ProviderPromptSignalBatch {
                    chunks: vec![crate::provider::ProviderPromptChunk {
                        kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                        merge_key: Some(id.into()),
                        bytes: id.as_bytes().to_vec(),
                    }],
                    ..Default::default()
                })),
            );
        run_ids.push(id.to_string());
    }
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let cleanups = run_ids
        .iter()
        .map(|id| InertPtyCleanup {
            app: app.clone(),
            provider_run_id: id.clone(),
        })
        .collect::<Vec<_>>();
    // Keep GC outside this scheduler test and explicitly block only A.
    runtime
        .owned
        .next_provider_process_gc_at_ms
        .store(u64::MAX, std::sync::atomic::Ordering::Release);
    let held = runtime.provider_runtime_lanes.acquire("a-blocked").await;
    let acquire = runtime
        .provider_runtime_lanes
        .notify_on_next_acquire_for_tests("a-blocked");
    let maintenance_runtime = runtime.clone();
    let mut maintenance =
        tokio::spawn(async move { maintenance_runtime.pump_transport_runtime().await });
    // On the base, maintenance claims the ready set and blocks on A. With
    // sole ownership, maintenance finishes without touching either signal.
    let maintenance_finished = tokio::select! {
        _ = acquire.notified() => false,
        result = &mut maintenance => { result.unwrap(); true },
        _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => panic!("maintenance did not reach readiness seam"),
    };
    let pump_runtime = runtime.clone();
    let pump = tokio::spawn(async move { pump_runtime.run_provider_output_pump().await });
    let delivered = tokio::time::timeout(std::time::Duration::from_millis(150), async {
        loop {
            if let crate::runtime_transport::WatchResult::Ok { records, .. } = runtime
                .watch_relay_subscription_state(session.id(), attachment.id(), false, None, 0)
                .await
            {
                if records.iter().any(|record| record.bytes == b"b-ready") {
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    })
    .await;
    pump.abort();
    let _ = pump.await;
    if !maintenance_finished {
        maintenance.abort();
        let _ = maintenance.await;
    }
    drop(held);
    drop(cleanups);
    assert!(
        delivered.is_ok(),
        "MP-08 / MP-10 maintenance stranded B behind blocked A without another provider event"
    );
}
