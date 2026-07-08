use super::*;

#[tokio::test]
async fn provider_completed_signal_settles_matching_active_prompt() {
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
            "client-1",
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
    let first_settlement = runtime
        .settle_owned_provider_prompt(session.id(), run.id(), true, false, false)
        .await
        .expect("provider completion signal should be accepted");
    assert!(first_settlement.had_active_prompt);
    assert!(!first_settlement.started_next_prompt);
    assert!(
        runtime
            .owned
            .session_store
            .get_session(session.id())
            .expect("session should exist")
            .active_prompt_for_agent(agent.id())
            .is_none()
    );
}

#[tokio::test]
async fn provider_completion_signal_preserves_external_active_prompt_and_queue() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-external-settlement",
            "worktree-external-settlement",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-external-settlement",
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
    let (external_prompt_id, queued_prompt_id) =
        sync_external_active_prompt_and_queue_arroba_prompt(
            &mut app,
            session.id(),
            attachment.id(),
            agent.id(),
        );

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let settlement = runtime
        .settle_owned_provider_prompt(session.id(), run.id(), true, false, false)
        .await
        .expect("provider completion signal should be accepted");
    assert!(!settlement.had_active_prompt);
    assert!(!settlement.started_next_prompt);
    assert_external_active_prompt_and_queued_arroba_prompt(
        &runtime,
        session.id(),
        agent.id(),
        &external_prompt_id,
        &queued_prompt_id,
    );
}

#[tokio::test]
async fn provider_terminal_failure_preserves_external_active_prompt_and_queue() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-external-terminal-failure",
            "worktree-external-terminal-failure",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-external-terminal-failure",
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
        "provider-run-external-terminal-failure",
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
    let (external_prompt_id, queued_prompt_id) =
        sync_external_active_prompt_and_queue_arroba_prompt(
            &mut app,
            session.id(),
            attachment.id(),
            agent.id(),
        );

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch {
                terminal_failure: Some("external provider stderr".to_string()),
                prompt_completed: true,
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("terminal failure batch should be accepted");
    assert_external_active_prompt_and_queued_arroba_prompt(
        &runtime,
        session.id(),
        agent.id(),
        &external_prompt_id,
        &queued_prompt_id,
    );
}

#[tokio::test]
async fn provider_completion_with_output_settles_after_fanning_out_records() {
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
            "client-completion-drain",
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
    let records = runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                    merge_key: Some("assistant-final".to_string()),
                    bytes: b"final output".to_vec(),
                }],
                completions: vec![crate::provider::ProviderAssistantCompletion {
                    message_id: "assistant-final".to_string(),
                    completed_at_ms: crate::session::unix_epoch_ms(),
                }],
                prompt_completed: true,
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("completion batch with output should be accepted");
    assert_eq!(records.len(), 1);

    let settled_session = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(
        settled_session
            .active_prompt_for_agent(agent.id())
            .is_none(),
        "final output is fanned out before settlement, so a completed structured prompt must not wait for another poll"
    );
}

#[tokio::test]
async fn provider_output_records_carry_active_external_prompt_origin() {
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
            "client-external-origin",
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
    sync_external_active_prompt_and_queue_arroba_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        agent.id(),
    );

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let records = runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                    merge_key: Some("external-output".to_string()),
                    bytes: b"external output".to_vec(),
                }],
                prompt_completed: false,
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("external active output batch should be accepted");

    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].prompt_origin,
        Some(crate::session::PromptOrigin::External)
    );

    let history_entries = runtime
        .owned
        .operational_history_store
        .load_session_history_entries(session.id(), Some(agent.id()))
        .expect("canonical operational history should load");
    let output_entry = history_entries
        .iter()
        .find(|entry| entry.merge_key.as_deref() == Some("external-output"))
        .expect("structured output should be persisted as history");
    assert_eq!(
        output_entry.prompt_origin,
        Some(crate::session::PromptOrigin::External)
    );
}
