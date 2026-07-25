use super::*;

#[tokio::test]
async fn codex_cancellation_waits_for_abort_ack_before_promoting_queued_prompt() {
    assert_structured_cancellation_waits_for_abort_ack("codex").await;
}

#[tokio::test]
async fn claude_cancellation_waits_for_abort_ack_before_promoting_queued_prompt() {
    assert_structured_cancellation_waits_for_abort_ack("claude").await;
}

async fn assert_structured_cancellation_waits_for_abort_ack(adapter_key: &str) {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-codex-cancellation",
            "worktree-codex-cancellation",
        ))
        .expect("session should be created");
    let requester = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "cancellation-requester",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("requester should attach");
    let observer = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "cancellation-observer",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("observer should attach");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        adapter_key,
        adapter_key,
        "default",
        "gpt-5.6",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        &format!("provider-run-{adapter_key}-cancellation"),
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: format!("test-{adapter_key}"),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some(format!("ws://test-{adapter_key}-runtime")),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());
    let active_prompt = crate::session::PromptQueueItem::new(
        "prompt-codex-cancellation",
        requester.id(),
        agent.id(),
        "run a blocking tool",
        crate::session::PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Started { prompt } = app
        .prompt_owner_submit_prepared_prompt(session.id(), active_prompt, false)
        .expect("prompt should start")
    else {
        panic!("prompt should start immediately");
    };
    let prompt_id = prompt.id().to_string();
    let queued_prompt = crate::session::PromptQueueItem::new(
        "queued-after-cancellation",
        requester.id(),
        agent.id(),
        "run after the blocked tool is aborted",
        crate::session::PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Queued {
        prompt: queued_prompt,
    } = app
        .prompt_owner_submit_prepared_prompt(session.id(), queued_prompt, false)
        .expect("second prompt should queue")
    else {
        panic!("second prompt should remain queued");
    };
    let queued_pending_id = queued_prompt.id().to_string();

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime.owned.note_prompt_started(run.id());
    runtime.owned.note_prompt_response_content(run.id());
    runtime.owned.append_history_entries(
        session.id(),
        vec![crate::history::SessionHistoryEntry::provider_output(
            session.id(),
            run.id(),
            Some(agent.id()),
            crate::terminal::TerminalOutputKind::ProviderTool,
            Some("exec-blocked".to_string()),
            r#"{"id":"exec-blocked","tool":"bash","status":"running"}"#,
        )],
    );

    let started_at = std::time::Instant::now();
    let cancellation = runtime
        .owned
        .cancel_local_prompt(session.id(), agent.id(), requester.id())
        .expect("cancellation should succeed")
        .expect("local prompt should be cancelled");

    assert!(started_at.elapsed() < std::time::Duration::from_secs(1));
    assert_eq!(cancellation.cancellation.prompt.id(), prompt_id);
    assert!(cancellation.cancellation.started_next.is_none());
    assert!(cancellation.dispatch.is_some());
    let snapshot = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    let active_prompt = snapshot
        .active_prompt_for_agent(agent.id())
        .expect("interrupted prompt should remain authoritative until abort acknowledgement");
    assert_eq!(active_prompt.id(), prompt_id);
    assert_eq!(
        active_prompt.status(),
        crate::session::PromptStatus::Cancelling
    );
    let queued = snapshot
        .queued_prompts_for_agent(agent.id())
        .expect("queued prompt should remain queued");
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].id(), queued_pending_id);
    assert!(runtime.owned.prompt_activity.read().contains_key(run.id()));

    let notices = runtime
        .owned
        .terminal_stream
        .drain_notice_records(session.id(), observer.id());
    assert_eq!(
        notices
            .iter()
            .filter(|notice| notice.message.contains("requested cancellation"))
            .count(),
        1
    );
}

#[tokio::test]
async fn paused_metaagent_cancellation_promotes_queued_user_prompt_after_abort_ack() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-paused-meta-cancellation",
            "worktree-paused-meta-cancellation",
        ))
        .expect("session should be created");
    let agent = app
        .agents_mut()
        .activate_agent_meta_mode(agent.id(), None)
        .expect("agent should enter meta mode");
    app.sessions_mut()
        .start_or_update_metaagent_task(session.id(), agent.id(), "keep queued work paused")
        .expect("meta task should start");
    app.sessions_mut()
        .set_metaagent_task_status(
            session.id(),
            agent.id(),
            crate::session::MetaagentTaskStatus::Paused,
        )
        .expect("meta task should pause");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "paused-meta-cancellation-requester",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("requester should attach");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.6",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-paused-meta-cancellation",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-paused-meta-codex".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("ws://test-paused-meta-codex-runtime".to_string()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());
    let active = crate::session::PromptQueueItem::new(
        "prompt-paused-meta-cancellation",
        attachment.id(),
        agent.id(),
        "active meta turn",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), active, false)
        .expect("active prompt should start");
    let queued = crate::session::PromptQueueItem::new(
        "queued-paused-meta-user-followup",
        attachment.id(),
        agent.id(),
        "queued user follow-up",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), queued, false)
        .expect("user follow-up should queue");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let cancellation = runtime
        .owned
        .cancel_local_prompt(session.id(), agent.id(), attachment.id())
        .expect("cancellation should succeed")
        .expect("local cancellation should be owned");
    assert!(cancellation.cancellation.started_next.is_none());
    let snapshot = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session should remain available");
    assert_eq!(
        snapshot
            .active_prompt_for_agent(agent.id())
            .expect("interrupted prompt should await abort acknowledgement")
            .status(),
        crate::session::PromptStatus::Cancelling
    );
    let queued = snapshot
        .queued_prompts_for_agent(agent.id())
        .expect("queued user prompt should remain");
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].prompt(), "queued user follow-up");

    let cancellation = runtime
        .owned
        .finalize_local_prompt_cancellation_with_queued_advance(
            session.id(),
            agent.id(),
            Some(run.id()),
        )
        .expect("provider abort acknowledgement should finalize cancellation");
    let started_next = cancellation
        .cancellation
        .started_next
        .expect("paused Meta task must yield to the queued user prompt");
    assert_eq!(started_next.prompt(), "queued user follow-up");
    let snapshot = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session should remain available");
    assert_eq!(
        snapshot
            .active_prompt_for_agent(agent.id())
            .expect("queued user prompt should become authoritative")
            .id(),
        started_next.id()
    );
}
