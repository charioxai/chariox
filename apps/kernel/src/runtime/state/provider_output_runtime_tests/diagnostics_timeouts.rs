use super::*;

#[tokio::test]
async fn codex_tool_output_text_does_not_classify_as_terminal_failure() {
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
            "client-tool-output",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.5",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-codex",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-codex".to_string(),
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

    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "read tool output\n",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start");
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let records = runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderTool,
                    merge_key: Some("tool-call-1".to_string()),
                    bytes: br#"{"tool":"bash","status":"completed","output":"Check if a CLAUDE.md file exists in the project root. If it does not exist, create it. This text mentions model context but is normal tool output."}"#.to_vec(),
                }],
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("structured tool output should be accepted");

    assert_eq!(records.len(), 1);
    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(
        session_state.active_prompt_for_agent(agent.id()).is_some(),
        "normal Codex tool output must not settle the active prompt"
    );
    let agent_activity = runtime
        .agent_activity_for_session(&session_state)
        .get(agent.id())
        .cloned()
        .expect("agent activity should be projected");
    assert!(agent_activity.busy);
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .expect("provider run should exist")
            .terminal_diagnostic(),
        None
    );
}

#[tokio::test]
async fn structured_terminal_failure_records_single_clean_notice() {
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
            "client-provider-error",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.3-codex-spark",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-codex-error",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-codex".to_string(),
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

    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "trigger provider error\n",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start");
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());

    let raw_error = r#"{
  "type": "error",
  "error": {
    "type": "invalid_request_error",
    "code": "unsupported_parameter",
    "message": "Unsupported parameter: 'reasoning.summary' is not supported with the 'gpt-5.3-codex-spark' model.",
    "param": "reasoning.summary"
  },
  "status": 400
}"#;
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch {
                notices: vec![raw_error.to_string(), raw_error.to_string()],
                terminal_failure: Some(raw_error.to_string()),
                prompt_completed: true,
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("structured terminal failure should be accepted");

    let notices = runtime
        .owned
        .terminal_stream
        .drain_notice_records(session.id(), attachment.id());
    assert_eq!(notices.len(), 1);
    assert_eq!(
        notices[0].message,
        "Provider prompt dispatch failed: Unsupported parameter: 'reasoning.summary' is not supported with the 'gpt-5.3-codex-spark' model."
    );
    let durable_notices = runtime
        .owned
        .operational_history_store
        .load_session_events(session.id(), Some(agent.id()))
        .expect("canonical operational history should load")
        .into_iter()
        .filter(|event| {
            event.kind == crate::history::HistoryEventKind::Notice
                && event.content.as_deref()
                    == Some(
                        "Provider prompt dispatch failed: Unsupported parameter: 'reasoning.summary' is not supported with the 'gpt-5.3-codex-spark' model.",
                    )
        })
        .collect::<Vec<_>>();
    assert_eq!(durable_notices.len(), 1);
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .expect("provider run should exist")
            .terminal_diagnostic(),
        Some(
            "Provider prompt dispatch failed: Unsupported parameter: 'reasoning.summary' is not supported with the 'gpt-5.3-codex-spark' model."
        )
    );
}

#[tokio::test]
async fn first_output_timeout_records_diagnostic_and_closes_prompt() {
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
            "client-first-output-timeout",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.5",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-first-output-timeout",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-codex-timeout".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-codex-timeout-runtime".to_string()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());

    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "start but never answer\n",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
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

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .pump_owned_provider_output(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            true,
        )
        .await
        .expect("provider output pump should reap silent timeout");

    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(
        session_state.active_prompt_for_agent(agent.id()).is_none(),
        "silent provider timeout must close the active prompt"
    );
    let run = runtime
        .owned
        .provider_store
        .get_run(run.id())
        .expect("provider run should still exist");
    assert!(run
        .terminal_diagnostic()
        .expect("timeout diagnostic should be recorded")
        .contains("Provider prompt produced no output"));
    let notices = runtime
        .owned
        .terminal_stream
        .drain_notice_records(session.id(), attachment.id());
    assert!(
        notices.iter().any(|record| record
            .message
            .contains("Provider prompt produced no output")),
        "timeout diagnostic should be visible to attached clients"
    );
}

#[tokio::test]
async fn provider_inactivity_timeout_records_diagnostic_and_closes_prompt() {
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
            "client-inactivity-timeout",
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
        "provider-run-inactivity-timeout",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-opencode-timeout".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-opencode-runtime".to_string()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());

    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "start, emit a tool, then stall\n",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start");
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());
    crate::transport::flow_control::note_prompt_response_content(&mut app, run.id());
    app.active_turns.mark_streaming(run.id());
    if let Some(state) = app.prompt_activity.write().get_mut(run.id()) {
        state.last_output_at =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(11 * 60));
        state.saw_response_content = true;
    } else {
        panic!("prompt activity should exist for the active run");
    }

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .pump_owned_provider_output(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            true,
        )
        .await
        .expect("provider output pump should reap inactive provider turn");

    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(
        session_state.active_prompt_for_agent(agent.id()).is_none(),
        "inactive provider timeout must close the active prompt"
    );
    let run = runtime
        .owned
        .provider_store
        .get_run(run.id())
        .expect("provider run should still exist");
    assert!(run
        .terminal_diagnostic()
        .expect("timeout diagnostic should be recorded")
        .contains("Provider prompt produced no output"));
    let notices = runtime
        .owned
        .terminal_stream
        .drain_notice_records(session.id(), attachment.id());
    assert!(
        notices
            .iter()
            .any(|record| record.message.contains("after its last activity")),
        "inactivity timeout diagnostic should be visible to attached clients"
    );
}

#[tokio::test]
async fn meta_mode_activation_registers_pending_provider_reload_when_agent_busy() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-meta-reload",
            "worktree-meta-reload",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-meta-reload",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "model",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");
    app.update_provider_run_projection(run.clone());
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "busy before meta mode\n",
        Vec::new(),
    )
    .expect("prompt should start");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .activate_meta_mode_for_prompt(session.id(), agent.id(), "delegate the queued task")
        .await
        .expect("meta mode should activate");

    assert!(
        runtime
            .owned
            .pending_provider_reloads
            .write()
            .contains_key(agent.id()),
        "busy meta mode activation must register a pending provider reload"
    );
    let agent = runtime
        .owned
        .agent_store
        .get_agent(agent.id())
        .expect("agent should still exist");
    assert!(agent.is_metaagent());
}

#[tokio::test]
async fn metaagent_receives_required_failed_turn_event_on_provider_timeout() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-meta-failure",
            "worktree-meta-failure",
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "codex").with_alias("meta"),
        )
        .expect("metaagent should spawn");
    let metaagent = app
        .agents_mut()
        .activate_agent_meta_mode(metaagent.id(), None)
        .expect("agent should enter meta mode");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-meta-failure",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let worker_request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.5",
    )
    .with_agent_id(agent.id());
    let mut worker_run = crate::provider::RuntimeProviderRun::new(
        "provider-run-meta-failure-worker",
        &worker_request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-meta-failure-worker".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-meta-failure-worker-runtime".to_string()),
        },
    );
    worker_run.mark_running();
    app.providers_mut().insert_run_for_test(worker_run.clone());
    app.update_provider_run_projection(worker_run.clone());

    let meta_request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.5",
    )
    .with_agent_id(metaagent.id());
    let mut meta_run = crate::provider::RuntimeProviderRun::new(
        "provider-run-meta-failure-meta",
        &meta_request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-meta-failure-meta".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-meta-failure-meta-runtime".to_string()),
        },
    );
    meta_run.mark_running();
    app.providers_mut().insert_run_for_test(meta_run.clone());
    app.update_provider_run_projection(meta_run.clone());

    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "start but never answer\n",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start");
    crate::transport::flow_control::note_prompt_started(&mut app, worker_run.id());
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
        worker_run.id().to_string(),
    )
    .with_phase(crate::app::ActiveTurnPhase::AwaitingFirstOutput);
    timed_out_turn.started_at_ms = crate::session::unix_epoch_ms().saturating_sub(11 * 60 * 1000);
    app.active_turn_store().start(timed_out_turn);

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .pump_owned_provider_output(
            session.id(),
            worker_run.id(),
            vec![attachment.id().to_string()],
            true,
        )
        .await
        .expect("provider output pump should reap silent timeout");

    let events =
        runtime
            .owned
            .metaagent_events
            .list(metaagent.id(), Some("agent.turn.failed"), None, 10);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].source_agent_id.as_deref(), Some(agent.id()));
    assert!(events[0]
        .summary
        .contains("Provider prompt produced no output"));
    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(
        session_state
            .active_prompt_for_agent(metaagent.id())
            .is_some(),
        "failed-turn event should start an inline metaagent prompt when the metaagent is idle"
    );
}
