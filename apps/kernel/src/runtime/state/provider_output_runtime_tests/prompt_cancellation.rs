use super::*;

#[tokio::test]
async fn codex_cancellation_settles_once_without_waiting_for_in_flight_tool_abort() {
    assert_structured_cancellation_orders_abort_before_queued_prompt("codex").await;
}

#[tokio::test]
async fn claude_cancellation_preserves_abort_before_queued_prompt() {
    assert_structured_cancellation_orders_abort_before_queued_prompt("claude").await;
}

async fn assert_structured_cancellation_orders_abort_before_queued_prompt(adapter_key: &str) {
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
    let started_next = cancellation
        .cancellation
        .started_next
        .as_ref()
        .expect("queued prompt should start");
    assert_ne!(started_next.id(), queued_pending_id);
    assert_eq!(
        started_next.prompt(),
        "run after the blocked tool is aborted"
    );
    assert!(cancellation.dispatch.is_none());
    let active_prompt = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist")
        .active_prompt_for_agent(agent.id())
        .cloned()
        .expect("queued prompt should be promoted");
    assert_eq!(active_prompt.id(), started_next.id());
    assert_eq!(
        active_prompt.status(),
        crate::session::PromptStatus::Dispatching
    );
    assert_eq!(
        active_prompt.durable_delivery_phase(),
        Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
    );
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

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
    let mut abort_count = 0;
    let mut submit_count = 0;
    loop {
        let (aborts, submits) = {
            let mut providers = runtime.owned.provider_store.write();
            (
                providers
                    .drain_finished_structured_prompt_abort_jobs()
                    .len(),
                providers
                    .drain_finished_structured_prompt_submit_jobs()
                    .len(),
            )
        };
        abort_count += aborts;
        submit_count += submits;
        if (abort_count > 0 && submit_count > 0) || tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(abort_count, 1);
    assert_eq!(submit_count, 1);
}
