use super::*;

#[tokio::test]
async fn codex_cancellation_waits_for_abort_ack_before_promoting_queued_prompt() {
    assert_structured_cancellation_waits_for_abort_ack("codex").await;
}

#[tokio::test]
async fn claude_cancellation_waits_for_abort_ack_before_promoting_queued_prompt() {
    assert_structured_cancellation_waits_for_abort_ack("claude").await;
}

#[tokio::test]
async fn cancelling_a_workflow_prompt_promotes_the_next_run_before_provider_dispatch() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-workflow-cancellation-promotion",
            "worktree-workflow-cancellation-promotion",
        ))
        .expect("session should be created");
    let requester = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "workflow-cancellation-requester",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("requester should attach");
    let workflow = app
        .sessions_mut()
        .create_workflow(
            session.id(),
            Some("workflow-cancellation-promotion".to_string()),
        )
        .expect("workflow should be created");
    let node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), agent.id())
        .expect("workflow node should be created");
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("events".to_string()),
        )
        .expect("workflow endpoint should be created");

    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "dev-stub",
        "dev-stub",
        "default",
        "workflow-test",
    )
    .with_agent_id(agent.id());
    let mut provider_run = crate::provider::RuntimeProviderRun::new(
        "provider-run-workflow-cancellation-promotion",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "workflow-cancellation-promotion".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    provider_run.mark_running();
    app.providers_mut()
        .insert_run_for_test(provider_run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(provider_run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(provider_run.clone());

    let active = crate::session::PromptQueueItem::new(
        "workflow-cancellation-active",
        requester.id(),
        agent.id(),
        "active workflow prompt",
        crate::session::PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Started { .. } = app
        .prompt_owner_submit_prepared_prompt(session.id(), active, false)
        .expect("active prompt should start")
    else {
        panic!("active prompt should start immediately");
    };
    let queued_run = app
        .sessions_mut()
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("queued workflow event".to_string()),
        )
        .expect("queued workflow run should be created");
    let queued_node_run_id = queued_run.node_runs()[0].id().to_string();
    app.sessions_mut()
        .prepare_workflow_turn(
            session.id(),
            queued_run.id(),
            &queued_node_run_id,
            format!("workflow-ack:{queued_node_run_id}"),
            "queued workflow prompt".to_string(),
            None,
            None,
        )
        .expect("workflow turn should be prepared before provider dispatch");
    let queued_prompt = crate::session::PromptQueueItem::new(
        "workflow-cancellation-queued",
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(queued_run.id()),
        agent.id(),
        "queued workflow prompt",
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(queued_run.id(), &queued_node_run_id);
    let crate::session::PromptSubmissionOutcome::Queued { .. } = app
        .prompt_owner_submit_prepared_prompt(session.id(), queued_prompt, false)
        .expect("workflow prompt should queue behind the active prompt")
    else {
        panic!("workflow prompt should remain queued");
    };
    let agent_runtime_projection = app.agent_runtime_projection_store();
    assert!(
        agent_runtime_projection
            .get(agent.id())
            .and_then(|projection| projection.active_prompt)
            .is_some(),
        "the active prompt should be present before cancellation"
    );

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let cancellation = runtime
        .owned
        .cancel_local_prompt(session.id(), agent.id(), requester.id())
        .expect("cancellation should succeed")
        .expect("local cancellation should be owned");
    assert!(cancellation.cancellation.started_next.is_none());
    let cancellation = runtime
        .owned
        .finalize_local_prompt_cancellation_with_queued_advance(
            session.id(),
            agent.id(),
            cancellation.cancellation.prompt.id(),
            Some(provider_run.id()),
        )
        .expect("provider abort acknowledgement should promote queued workflow prompt");
    assert!(cancellation.cancellation.started_next.is_some());
    assert!(
        agent_runtime_projection
            .get(agent.id())
            .and_then(|projection| projection.active_prompt)
            .is_some_and(|prompt| prompt.workflow_run_id() == Some(queued_run.id())),
        "runtime-owned cancellation settlement must refresh the promoted workflow prompt projection"
    );

    let snapshot = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    let run = snapshot
        .workflow_runs()
        .iter()
        .find(|run| run.id() == queued_run.id())
        .expect("queued workflow run should remain in the session");
    assert_eq!(run.status(), crate::session::WorkflowRunStatus::Running);
    assert_eq!(
        run.node_runs()[0].status(),
        crate::session::WorkflowNodeRunStatus::Running
    );
    assert!(run.node_runs()[0].started_at_ms().is_some());
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
        format!("provider-run-{adapter_key}-cancellation"),
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
            cancellation.cancellation.prompt.id(),
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

#[tokio::test]
async fn paused_metaagent_cancellation_holds_queued_private_event_after_abort_ack() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-paused-meta-private-event",
            "worktree-paused-meta-private-event",
        ))
        .expect("session should be created");
    let agent = app
        .agents_mut()
        .activate_agent_meta_mode(agent.id(), None)
        .expect("agent should enter meta mode");
    app.sessions_mut()
        .start_or_update_metaagent_task(session.id(), agent.id(), "keep private events paused")
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
            "paused-meta-private-event-source",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("event source should attach");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.6",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-paused-meta-private-event",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-paused-meta-private-event".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("ws://test-paused-meta-private-event".to_string()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());
    let active = crate::session::PromptQueueItem::new(
        "prompt-paused-meta-private-event",
        attachment.id(),
        agent.id(),
        "active meta turn",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), active, false)
        .expect("active prompt should start");
    let queued = crate::session::PromptQueueItem::new(
        "queued-paused-meta-private-event",
        attachment.id(),
        agent.id(),
        crate::scheduler::prompt_injection::METAAGENT_EVENT_VISIBLE_PROMPT,
        crate::session::PromptStatus::Queued,
    )
    .with_hidden_system_context("<metaagent-event>worker completed</metaagent-event>");
    app.prompt_owner_submit_prepared_prompt(session.id(), queued, false)
        .expect("private event should queue");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let cancellation = runtime
        .owned
        .cancel_local_prompt(session.id(), agent.id(), attachment.id())
        .expect("cancellation should succeed")
        .expect("local cancellation should be owned");
    assert!(cancellation.cancellation.started_next.is_none());

    let cancellation = runtime
        .owned
        .finalize_local_prompt_cancellation_with_queued_advance(
            session.id(),
            agent.id(),
            cancellation.cancellation.prompt.id(),
            Some(run.id()),
        )
        .expect("provider abort acknowledgement should finalize cancellation");
    assert!(
        cancellation.cancellation.started_next.is_none(),
        "paused Meta task must not start a queued private event"
    );
    let snapshot = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session should remain available");
    assert!(
        snapshot.active_prompt_for_agent(agent.id()).is_none(),
        "paused Meta task must remain idle"
    );
    let queued = snapshot
        .queued_prompts_for_agent(agent.id())
        .expect("private event should remain queued");
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued[0].prompt(),
        crate::scheduler::prompt_injection::METAAGENT_EVENT_VISIBLE_PROMPT
    );
}

#[tokio::test]
async fn paused_metaagent_queues_new_private_event_without_dispatch() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, metaagent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-paused-meta-new-event",
            "worktree-paused-meta-new-event",
        ))
        .expect("session should be created");
    let metaagent = app
        .agents_mut()
        .activate_agent_meta_mode(metaagent.id(), None)
        .expect("agent should enter meta mode");
    let worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub").with_alias("worker"),
        )
        .expect("worker should spawn");
    app.sessions_mut()
        .start_or_update_metaagent_task(session.id(), metaagent.id(), "wait for worker")
        .expect("meta task should start");
    app.sessions_mut()
        .set_metaagent_task_status(
            session.id(),
            metaagent.id(),
            crate::session::MetaagentTaskStatus::Paused,
        )
        .expect("meta task should pause");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "paused-meta-new-event-source",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("event source should attach");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.6",
    )
    .with_agent_id(metaagent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-paused-meta-new-event",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-paused-meta-new-event".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("ws://test-paused-meta-new-event".to_string()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run);

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let dispatches = runtime.owned.metaagent_event_prompt_for_metaagent(
        session.id(),
        &metaagent,
        "agent.turn.completed",
        Some(worker.id()),
        attachment.id(),
        "worker completed",
        "worker completed while the Meta task was paused",
        serde_json::json!({ "worker_id": worker.id() }),
        worker.agent_ref().to_string(),
    );
    assert!(dispatches.local.is_empty());
    assert!(dispatches.remote.is_empty());

    let snapshot = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session should remain available");
    assert!(
        snapshot.active_prompt_for_agent(metaagent.id()).is_none(),
        "a new private event must not start the paused Meta agent"
    );
    let queued = snapshot
        .queued_prompts_for_agent(metaagent.id())
        .expect("new private event should remain queued");
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued[0].prompt(),
        crate::scheduler::prompt_injection::METAAGENT_EVENT_VISIBLE_PROMPT
    );
}

#[tokio::test]
async fn structured_abort_waiter_releases_provider_lane_after_external_owner_settlement() {
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
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.6",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-abort-owner-wait",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-abort-owner-wait".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("ws://test-abort-owner-wait-runtime".to_string()),
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

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let cancellation = runtime
        .owned
        .cancel_local_prompt(session.id(), agent.id(), requester.id())
        .unwrap()
        .unwrap();
    let dispatch = cancellation.dispatch.unwrap();
    let completion = runtime.owned.provider_store.run_actor_completion_signal();
    let sequence = completion.sequence();
    let entered = Arc::new(tokio::sync::Notify::new());
    let mut waiter = tokio::spawn({
        let runtime = runtime.clone();
        let entered = entered.clone();
        let completion = completion.clone();
        async move {
            let _permit = runtime
                .provider_runtime_lanes
                .acquire(&dispatch.provider_run_id)
                .await;
            entered.notify_one();
            runtime
                .wait_for_structured_prompt_abort_settlement(&dispatch, &completion, sequence)
                .await;
        }
    });
    entered.notified().await;
    assert!(runtime
        .provider_runtime_lanes
        .try_acquire(run.id())
        .is_none());
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        !waiter.is_finished(),
        "Observation alone must not settle an unacknowledged cancellation"
    );
    let still_cancelling = runtime
        .owned
        .session_store
        .get_session(session.id())
        .unwrap();
    assert_eq!(
        runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&still_cancelling, agent.id())
            .unwrap()
            .status(),
        crate::session::PromptStatus::Cancelling
    );
    // An output pump reaps a previously acknowledged abort. Ownership settlement
    // itself does not produce a new provider-actor completion event.
    runtime
        .owned
        .finalize_local_prompt_cancellation_with_queued_advance(
            session.id(),
            agent.id(),
            &prompt_id,
            Some(run.id()),
        )
        .unwrap();
    assert_eq!(completion.sequence(), sequence);
    let finished = tokio::time::timeout(std::time::Duration::from_secs(1), &mut waiter).await;
    if finished.is_err() {
        waiter.abort();
        let _ = waiter.await;
    }
    assert!(finished.is_ok(), "MP-08/MP-10 a settled cancellation keeps the provider lane locked without another actor completion; the next Stop cannot acquire it");
    finished
        .unwrap()
        .expect("The cancellation waiter must not panic");
    assert!(runtime
        .provider_runtime_lanes
        .try_acquire(run.id())
        .is_some());
}

// MP-08/MP-10/MP-11: a provider that never answers Stop cannot hold the next turn.
#[tokio::test]
async fn unresponsive_structured_abort_terminates_provider_and_settles_task() {
    assert_unresponsive_abort_settles(false, false, AbortProviderExit::None).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn native_cancellation_reaps_a_tool_in_its_own_process_group() {
    assert_unresponsive_abort_settles(true, false, AbortProviderExit::None).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn cancelling_one_shared_pty_alias_preserves_the_other_running_owner() {
    let worktree = crate::test_support::TestWorktree::new("shared-pty-stop");
    let mut app = DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
    let (session, first) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let second = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(crate::agent::CreateAgentRequest::new(session.id(), "codex"))
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "shared-stop-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let mut runs = Vec::new();
    for (id, agent) in [
        ("shared-stop-first", first.id()),
        ("shared-stop-second", second.id()),
    ] {
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "dev-stub",
            "default",
            "stub",
        )
        .with_agent_id(agent);
        let mut run = crate::provider::RuntimeProviderRun::new(
            id,
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "shared dev-stub".into(),
                pty_target: Some("shared-stop-pty".into()),
                pty_program: Some("/bin/sh".into()),
                pty_args: vec!["-c".into(), "exec sleep 300".into()],
                pty_env: Default::default(),
                pty_env_remove: vec![],
                working_directory: None,
                structured_endpoint: None,
            },
        );
        run.mark_running();
        app.pty_mut().spawn_for_run(&run).unwrap();
        app.providers_mut().insert_run_for_test(run.clone());
        runs.push(run);
    }
    let shared_pid = app.pty().process_id(runs[0].id()).unwrap().unwrap();
    assert_eq!(
        app.pty().process_id(runs[1].id()).unwrap(),
        Some(shared_pid)
    );
    app.prompt_owner_submit_prepared_prompt(
        session.id(),
        crate::session::PromptQueueItem::new(
            "shared-stop-prompt",
            attachment.id(),
            first.id(),
            "stop only this alias",
            crate::session::PromptStatus::Queued,
        ),
        false,
    )
    .unwrap();
    let app = Arc::new(Mutex::new(app));
    let _first_cleanup = InertPtyCleanup {
        app: app.clone(),
        provider_run_id: runs[0].id().into(),
    };
    let _second_cleanup = InertPtyCleanup {
        app: app.clone(),
        provider_run_id: runs[1].id().into(),
    };
    let runtime = owned_runtime_state(&app).await;
    runtime.owned.note_prompt_started(runs[0].id());
    let dispatch = runtime
        .owned
        .cancel_local_prompt(session.id(), first.id(), attachment.id())
        .unwrap()
        .unwrap()
        .dispatch
        .unwrap();
    runtime.spawn_prompt_abort(dispatch, runtime.provider_runtime_lanes.clone());
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while runtime
            .owned
            .session_snapshot(session.id())
            .unwrap()
            .active_prompt_for_agent(first.id())
            .is_some()
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Stop must settle its alias while another alias still owns the shared endpoint");
    let app = app.lock().await;
    assert!(!app.pty().has_process(runs[0].id()));
    assert_eq!(
        app.pty().process_id(runs[1].id()).unwrap(),
        Some(shared_pid)
    );
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(runs[1].id())
            .unwrap()
            .state(),
        crate::provider::ProviderRunState::Running
    );
    let stat = std::fs::read(format!("/proc/{shared_pid}/stat")).unwrap();
    let end = stat.windows(2).rposition(|pair| pair == b") ").unwrap();
    assert!(
        !matches!(stat.get(end + 2), Some(b'Z' | b'X')),
        "Stop must not interrupt or terminate the other alias's live process"
    );
}

#[tokio::test]
async fn pending_provider_reap_keeps_cancellation_owned_until_retry_succeeds() {
    assert_unresponsive_abort_settles(false, true, AbortProviderExit::None).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn cancellation_retains_detached_tool_ownership_after_provider_exit_is_cached() {
    assert_unresponsive_abort_settles(true, false, AbortProviderExit::Observed).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn cancellation_retains_detached_tool_ownership_for_an_already_ended_provider() {
    assert_unresponsive_abort_settles(true, false, AbortProviderExit::AlreadyEnded).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn unrelated_non_utf8_process_name_does_not_block_stop_before_interrupt() {
    struct UnrelatedProcess(std::process::Child);
    impl Drop for UnrelatedProcess {
        fn drop(&mut self) {
            if matches!(self.0.try_wait(), Ok(None)) {
                self.0.kill().expect("stop the exact fixture child");
            }
            let _ = self.0.wait();
        }
    }
    let worktree = crate::test_support::TestWorktree::new("non-utf8-stop-neighbor");
    let ready = worktree.path().join("ready");
    let child = std::process::Command::new("python3")
        .args([
            "-c",
            "import ctypes,os,sys,time\nassert ctypes.CDLL(None).prctl(15,b'foreign-\\xff',0,0,0)==0\nopen(sys.argv[1],'w').write(str(os.getpid()))\ntime.sleep(30)",
        ])
        .arg(&ready)
        .spawn()
        .expect("start an unrelated process with a legal non-UTF-8 Linux comm");
    let mut child = UnrelatedProcess(child);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !ready.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the foreign process must install its name before Stop");
    assert_unresponsive_abort_settles(false, false, AbortProviderExit::None).await;
    assert!(
        child.0.try_wait().unwrap().is_none(),
        "Stop must leave the unrelated exact fixture process running"
    );
}

enum AbortProviderExit {
    None,
    Observed,
    AlreadyEnded,
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn claude_stream_json_cancellation_settles_its_detached_tool_before_the_turn() {
    assert_claude_stream_json_cancellation_settles(false, false, false).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn claude_stream_json_cancellation_settles_while_the_actor_holds_its_state() {
    assert_claude_stream_json_cancellation_settles(true, false, false).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn claude_stream_json_cancellation_settles_a_busy_already_ended_run() {
    assert_claude_stream_json_cancellation_settles(true, true, false).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn pending_claude_retirement_keeps_other_provider_runs_readable() {
    assert_claude_stream_json_cancellation_settles(false, true, true).await;
}

#[cfg(target_os = "linux")]
async fn assert_claude_stream_json_cancellation_settles(
    busy_actor: bool,
    already_ended: bool,
    pending_cleanup: bool,
) {
    use crate::durable_state::agent_lifecycle::{ExecutionState, Operation};
    let worktree = crate::test_support::TestWorktree::new("stream-json-stop");
    let marker = worktree.path().join("tool-alive");
    let tool_pid_file = worktree.path().join("tool-pid");
    let root_pid_file = worktree.path().join("root-pid");
    std::fs::write(&marker, "alive").unwrap();
    let mut config = crate::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "stream-stop-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude",
        "default",
        "sonnet",
    )
    .with_agent_id(agent.id());
    let script = r#"import os,sys,time,signal
if not os.path.exists(sys.argv[1]):
 open(sys.argv[3],'w').write(str(os.getpid()))
 pid=os.fork()
 if pid==0:
  os.setsid()
  signal.signal(signal.SIGINT,signal.SIG_IGN)
  open(sys.argv[1],'w').write(str(os.getpid()))
  while os.path.exists(sys.argv[2]): time.sleep(.01)
  os._exit(0)
for line in sys.stdin: pass
"#;
    let mut run = crate::provider::RuntimeProviderRun::new(
        "stream-json-stop-provider",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "claude:stream-json".into(),
            pty_target: None,
            pty_program: Some("python3".into()),
            pty_args: vec![
                "-c".into(),
                script.into(),
                tool_pid_file.display().to_string(),
                marker.display().to_string(),
                root_pid_file.display().to_string(),
            ],
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: Some(worktree.path().to_path_buf()),
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.providers_mut().initialize_runtime(&run).unwrap();
    let prompt = crate::session::PromptQueueItem::new(
        "stream-json-stop-prompt",
        attachment.id(),
        agent.id(),
        "run the owned tool",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    struct Cleanup {
        providers: crate::provider::ProviderProcessServiceStore,
        run_id: String,
        marker: std::path::PathBuf,
    }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.marker);
            self.providers.clear_runtime(&self.run_id);
        }
    }
    let _cleanup = Cleanup {
        providers: runtime.owned.provider_store.clone(),
        run_id: run.id().into(),
        marker: marker.clone(),
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !tool_pid_file.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the real stream-json child must start its detached tool");
    let tool_pid: u32 = std::fs::read_to_string(&tool_pid_file)
        .unwrap()
        .parse()
        .unwrap();
    let root_pid: u32 = std::fs::read_to_string(&root_pid_file)
        .unwrap()
        .parse()
        .unwrap();
    if busy_actor {
        runtime
            .owned
            .provider_store
            .set_output_poll_delay_for_tests(run.id(), std::time::Duration::from_secs(6));
        assert!(runtime
            .owned
            .provider_store
            .enqueue_structured_output_poll(run.id())
            .unwrap());
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while runtime
                .owned
                .provider_store
                .structured_runtime_state_bound_for_tests(run.id())
            {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the actual actor must lease the Claude state before Stop");
    }
    runtime.owned.note_prompt_started(run.id());
    let cancellation = runtime
        .owned
        .cancel_local_prompt(session.id(), agent.id(), attachment.id())
        .unwrap()
        .unwrap();
    let dispatch = cancellation.dispatch.unwrap();
    runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Begin {
            owner: session.owner_user_id().into(),
            room: session.id().into(),
            agent: agent.id().into(),
            prompt: dispatch.prompt_id.clone(),
            run: Some(run.id().into()),
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    let mut other_run_readable_while_cleanup_pending = true;
    if pending_cleanup {
        let process = runtime
            .owned
            .provider_store
            .read()
            .claude_owned_process(run.id())
            .unwrap();
        let pending = process.defer_cleanup_for_test();
        struct ReleasePending(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for ReleasePending {
            fn drop(&mut self) {
                self.0.store(false, std::sync::atomic::Ordering::Release);
            }
        }
        let release = ReleasePending(pending);
        let mut other = crate::provider::RuntimeProviderRun::from_control_capability_inference(
            "other-room-provider",
            "other-room".into(),
            Some("other-agent".into()),
            "codex".into(),
        );
        other.mark_running();
        runtime
            .owned
            .provider_store
            .write()
            .insert_run_for_test(other);
        let ending = {
            let store = runtime.owned.provider_store.clone();
            let room = session.id().to_string();
            let id = run.id().to_string();
            tokio::task::spawn_blocking(move || store.mark_run_ended_provider_only(&room, &id))
        };
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while !process.retiring_for_test() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("end-run must reach physical retirement while cleanup remains pending");
        let reading = {
            let store = runtime.owned.provider_store.clone();
            tokio::task::spawn_blocking(move || store.get_run("other-room-provider"))
        };
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        other_run_readable_while_cleanup_pending = reading.is_finished();
        let active = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(
                &runtime
                    .owned
                    .session_store
                    .get_session(session.id())
                    .unwrap(),
                agent.id(),
            )
            .unwrap();
        assert_eq!(active.status(), crate::session::PromptStatus::Cancelling);
        drop(release);
        let ended = tokio::time::timeout(std::time::Duration::from_secs(3), ending)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        runtime
            .owned
            .provider_run_projection
            .update(ended.into_run());
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(3), reading)
                .await
                .unwrap()
                .unwrap()
                .unwrap()
                .id(),
            "other-room-provider"
        );
    } else if already_ended {
        let ended = runtime
            .owned
            .provider_store
            .mark_run_ended_provider_only(session.id(), run.id())
            .unwrap();
        runtime
            .owned
            .provider_run_projection
            .update(ended.into_run());
    }
    runtime.spawn_prompt_abort(dispatch, runtime.provider_runtime_lanes.clone());
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while runtime
            .owned
            .session_snapshot(session.id())
            .unwrap()
            .active_prompt_for_agent(agent.id())
            .is_some()
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Stop must settle without waiting for a busy provider actor");
    let running = |pid| {
        std::fs::read(format!("/proc/{pid}/stat"))
            .ok()
            .is_some_and(|bytes| {
                bytes
                    .windows(2)
                    .rposition(|pair| pair == b") ")
                    .is_some_and(|end| !matches!(bytes.get(end + 2), Some(b'Z' | b'X')))
            })
    };
    let (root_live, tool_live) = (running(root_pid), running(tool_pid));
    std::fs::remove_file(&marker).unwrap();
    assert!(!root_live && !tool_live,
        "stream-json Stop released its turn before physical settlement: root_live={root_live}, tool_live={tool_live}, busy_actor={busy_actor}");
    let tasks = runtime
        .owned
        .durable_state_store
        .agent_tasks(Some(session.id()), Some(agent.id()))
        .unwrap();
    let task = tasks
        .iter()
        .find(|task| task.prompt_id == "stream-json-stop-prompt")
        .expect("the exact cancelled stream-json task must remain recorded");
    assert_eq!(task.state, ExecutionState::Cancelled);
    assert!(other_run_readable_while_cleanup_pending, "a pending physical Claude cleanup must not hold the shared provider lock against another room");
}

async fn assert_unresponsive_abort_settles(
    detached_tool: bool,
    deferred_reap: bool,
    provider_exit: AbortProviderExit,
) {
    use crate::durable_state::agent_lifecycle::{ExecutionState, Operation};
    let worktree = crate::test_support::TestWorktree::new("unresponsive-stop");
    let mut config = crate::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "stop-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let mut run = crate::provider::RuntimeProviderRun::from_control_capability_inference(
        "unresponsive-stop-provider",
        session.id().into(),
        Some(agent.id().into()),
        "codex".into(),
    );
    run.mark_running();
    let sentinel = worktree.path().join("tool-alive");
    let pid_file = worktree.path().join("tool-pid");
    let mut tool_pid = None;
    if detached_tool {
        std::fs::write(&sentinel, b"alive").unwrap();
        let script = "import os,time,sys,signal\npid=os.fork()\nif pid==0:\n os.setsid()\n signal.signal(signal.SIGINT,signal.SIG_IGN)\n open(sys.argv[1],'w').write(str(os.getpid()))\n while os.path.exists(sys.argv[2]): time.sleep(.02)\nelse: os.waitpid(pid,0)";
        app.pty_mut()
            .spawn(crate::pty::PtySpawnRequest {
                process_key: run.id().into(),
                provider_run_id: run.id().into(),
                program: "python3".into(),
                args: vec![
                    "-c".into(),
                    script.into(),
                    pid_file.display().to_string(),
                    sentinel.display().to_string(),
                ],
                env: Default::default(),
                env_remove: vec![],
                working_directory: None,
                cols: 80,
                rows: 24,
            })
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while !pid_file.exists() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        tool_pid = Some(
            std::fs::read_to_string(&pid_file)
                .unwrap()
                .trim()
                .parse::<u32>()
                .unwrap(),
        );
    } else {
        spawn_inert_pty_for_run(&mut app, run.id());
    }
    let reap_ready = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(!deferred_reap));
    if deferred_reap {
        app.pty_mut()
            .defer_reap_for_test(run.id(), reap_ready.clone());
    }
    app.providers_mut().insert_run_for_test(run.clone());
    let prompt = crate::session::PromptQueueItem::new(
        "unresponsive-stop-prompt",
        attachment.id(),
        agent.id(),
        "run a tool that never answers",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let _cleanup = InertPtyCleanup {
        app: app.clone(),
        provider_run_id: run.id().into(),
    };
    let runtime = owned_runtime_state(&app).await;
    runtime.owned.note_prompt_started(run.id());
    for (id, status) in [("tool-active", "running"), ("tool-finished", "completed")] {
        let bytes =
            serde_json::json!({"id": id, "tool": "bash", "status": status, "output": "partial"})
                .to_string();
        runtime
            .owned
            .note_prompt_tool_output(run.id(), Some(id), bytes.as_bytes());
    }
    let cancellation = runtime
        .owned
        .cancel_local_prompt(session.id(), agent.id(), attachment.id())
        .unwrap()
        .unwrap();
    let dispatch = cancellation.dispatch.unwrap();
    runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Begin {
            owner: session.owner_user_id().into(),
            room: session.id().into(),
            agent: agent.id().into(),
            prompt: dispatch.prompt_id.clone(),
            run: Some(run.id().into()),
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    // A hung provider call can hold this lane before the abort is even enqueued.
    let _held_lane = runtime.provider_runtime_lanes.acquire(run.id()).await;
    runtime.spawn_prompt_abort(dispatch, runtime.provider_runtime_lanes.clone());
    if !matches!(provider_exit, AbortProviderExit::None) {
        // The provider lane stays occupied while another output pump observes
        // exit. Its detached tool must still belong to the cancelling turn.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        app.lock()
            .await
            .pty_mut()
            .write_input(run.id(), b"\x03")
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if app
                    .lock()
                    .await
                    .pty_mut()
                    .poll_process_state(run.id())
                    .unwrap()
                    .is_exited()
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("provider must exit and cache its status before cleanup");
        if matches!(provider_exit, AbortProviderExit::AlreadyEnded) {
            let ended = runtime
                .owned
                .provider_store
                .mark_run_ended_provider_only(session.id(), run.id())
                .unwrap();
            runtime
                .owned
                .provider_run_projection
                .update(ended.into_run());
        }
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                match runtime
                    .reconcile_provider_run_exit_for_output(session.id(), run.id())
                    .await
                {
                    Ok(true) => break,
                    Ok(false) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
                    Err(DaemonError::PtyCleanup { .. }) => {
                        let current = runtime.owned.session_snapshot(session.id()).unwrap();
                        assert_eq!(
                            current
                                .active_prompt_for_agent(agent.id())
                                .unwrap()
                                .status(),
                            crate::session::PromptStatus::Cancelling,
                            "pending descendant cleanup released the turn"
                        );
                        assert!(
                            app.lock().await.pty().has_process(run.id()),
                            "pending exit reconciliation discarded the retained witnesses"
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                    Err(error) => panic!("exit reconciliation failed: {error}"),
                }
            }
        })
        .await
        .expect("exit reconciliation must settle its retained descendants");
    }
    if deferred_reap {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let failures = runtime
                    .owned
                    .operational_history_store
                    .query_events(crate::history::HistoryEventQuery {
                        session_id: Some(session.id().into()),
                        kind: Some("notice".into()),
                        text: Some("Provider cancellation cleanup failed".into()),
                        ..Default::default()
                    })
                    .unwrap();
                if !failures.is_empty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("pending cleanup must report its reason");
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let active = runtime.owned.session_snapshot(session.id()).unwrap();
        assert_eq!(
            active.active_prompt_for_agent(agent.id()).unwrap().status(),
            crate::session::PromptStatus::Cancelling
        );
        assert!(
            app.lock().await.pty().has_process(run.id()),
            "failed reap lost physical ownership"
        );
        assert!(
            !runtime
                .owned
                .terminal_stream
                .output_records()
                .iter()
                .any(|record| record.kind == crate::terminal::TerminalOutputKind::ProviderTool),
            "tools must remain pending until physical cleanup succeeds"
        );
        let failures = runtime
            .owned
            .operational_history_store
            .query_events(crate::history::HistoryEventQuery {
                session_id: Some(session.id().into()),
                kind: Some("notice".into()),
                text: Some("Provider cancellation cleanup failed".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            failures.len(),
            1,
            "cleanup retries must not flood the transcript"
        );
        reap_ready.store(true, std::sync::atomic::Ordering::Release);
    }

    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if runtime
                .owned
                .session_snapshot(session.id())
                .unwrap()
                .active_prompt_for_agent(agent.id())
                .is_none()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Stop must settle within seconds even while the provider lane is hung");
    let snapshot = runtime.owned.session_snapshot(session.id()).unwrap();
    assert!(snapshot.active_prompt_for_agent(agent.id()).is_none());
    let tasks = runtime
        .owned
        .durable_state_store
        .agent_tasks(Some(session.id()), Some(agent.id()))
        .unwrap();
    assert_eq!(tasks[0].state, ExecutionState::Cancelled);
    let tools = runtime
        .owned
        .terminal_stream
        .output_records()
        .into_iter()
        .filter(|record| {
            record.provider_run_id == run.id()
                && record.kind == crate::terminal::TerminalOutputKind::ProviderTool
        })
        .map(|record| serde_json::from_slice::<serde_json::Value>(&record.bytes).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        tools.len(),
        1,
        "Stop must publish one terminal update for each unfinished tool"
    );
    assert_eq!(tools[0]["id"], "tool-active");
    assert_eq!(tools[0]["status"], "cancelled");
    let history = runtime
        .owned
        .operational_history_store
        .query_events(crate::history::HistoryEventQuery {
            session_id: Some(session.id().into()),
            kind: Some("provider_tool".into()),
            ..Default::default()
        })
        .unwrap();
    assert!(
        history.iter().any(
            |event| event.prompt_id.as_deref() == Some("unresponsive-stop-prompt")
                && event
                    .content
                    .as_deref()
                    .is_some_and(|text| text.contains("cancelled"))
        ),
        "cancelled tool update must survive transcript hydration"
    );

    if let Some(pid) = tool_pid {
        let live = std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .is_some_and(|stat| {
                stat.split_once(") ")
                    .is_some_and(|(_, rest)| !rest.starts_with('Z'))
            });
        std::fs::remove_file(&sentinel).unwrap();
        assert!(!live, "Stop left a tool alive in its own process group");
    }
    assert!(
        app.lock().await.pty().process_id(run.id()).is_err(),
        "Stop must terminate the owned provider process before releasing the turn"
    );
}

#[tokio::test]
async fn tool_updates_received_during_cancellation_cleanup_keep_their_final_status() {
    assert_tool_updates_received_during_cleanup(CancellationOutputBoundary::Applied).await;
}

#[tokio::test]
async fn received_actor_poll_is_applied_before_cancellation_records_tool_statuses() {
    assert_tool_updates_received_during_cleanup(CancellationOutputBoundary::Mailbox).await;
}

#[tokio::test]
async fn a_claimed_output_batch_keeps_cancellation_owned_until_history_publication() {
    assert_tool_updates_received_during_cleanup(CancellationOutputBoundary::Claimed).await;
}

enum CancellationOutputBoundary {
    Applied,
    Mailbox,
    Claimed,
}

async fn assert_tool_updates_received_during_cleanup(mode: CancellationOutputBoundary) {
    let queued = matches!(mode, CancellationOutputBoundary::Mailbox);
    let claimed = matches!(mode, CancellationOutputBoundary::Claimed);
    let worktree = crate::test_support::TestWorktree::new("cancel-tool-completion");
    let mut app = DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "tool-completion-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let mut run = crate::provider::RuntimeProviderRun::from_control_capability_inference(
        "tool-completion-provider",
        session.id().into(),
        Some(agent.id().into()),
        "codex".into(),
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    if queued || claimed {
        spawn_inert_pty_for_run(&mut app, run.id());
    }
    app.prompt_owner_submit_prepared_prompt(
        session.id(),
        crate::session::PromptQueueItem::new(
            "tool-completion-prompt",
            attachment.id(),
            agent.id(),
            "finish a tool",
            crate::session::PromptStatus::Queued,
        ),
        false,
    )
    .unwrap();
    if claimed {
        let prompt_id = app
            .session_snapshot(session.id())
            .unwrap()
            .active_prompt_for_agent(agent.id())
            .unwrap()
            .id()
            .to_string();
        app.mark_active_prompt_delivery(
            session.id(),
            agent.id(),
            &prompt_id,
            crate::session::DurablePromptDeliveryPhase::Delivered,
            Some(run.id().to_string()),
            None,
        )
        .unwrap();
    }
    let app = Arc::new(Mutex::new(app));
    let mut runtime = owned_runtime_state(&app).await;
    let _cleanup = InertPtyCleanup {
        app: app.clone(),
        provider_run_id: run.id().into(),
    };
    runtime.owned.note_prompt_started(run.id());
    runtime.owned.note_prompt_tool_output(
        run.id(),
        Some("tool-a"),
        br#"{"id":"tool-a","tool":"bash","status":"running"}"#,
    );
    let dispatch = runtime
        .owned
        .cancel_local_prompt(session.id(), agent.id(), attachment.id())
        .unwrap()
        .unwrap()
        .dispatch
        .unwrap();
    if claimed {
        let batch = crate::provider::ProviderPromptSignalBatch {
            chunks: vec![
                crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderTool,
                    merge_key: Some("tool-a".into()),
                    bytes: br#"{"id":"tool-a","tool":"bash","status":"completed"}"#.to_vec(),
                },
                crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderTool,
                    merge_key: Some("tool-b".into()),
                    bytes: br#"{"id":"tool-b","tool":"bash","status":"running"}"#.to_vec(),
                },
            ],
            ..Default::default()
        };
        // Hold real global history capacity: the normal pump dequeues and
        // updates activity, then waits before it can publish any history.
        let mut capacity = Vec::new();
        for n in 0..64 {
            capacity.push(
                runtime
                    .reserve_provider_history(&format!("capacity-holder-{n}"), true)
                    .await
                    .unwrap(),
            );
        }
        runtime
            .owned
            .structured_output_records
            .mark_poll_enqueued(run.id(), Some(dispatch.prompt_id.clone()));
        runtime
            .owned
            .provider_store
            .write()
            .push_finished_structured_output_poll_for_test(run.id().into(), Ok(Some(batch)));
        let pump = {
            let state = runtime.clone();
            let session_id = session.id().to_string();
            let run_id = run.id().to_string();
            let attachment_id = attachment.id().to_string();
            tokio::spawn(async move {
                state
                    .pump_owned_provider_output(&session_id, &run_id, vec![attachment_id], true)
                    .await
            })
        };
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while !runtime
                .owned
                .prompt_activity
                .read()
                .get(run.id())
                .is_some_and(|state| state.active_tool_ids.contains("tool-b"))
            {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the normal output pump must claim the batch and reach history capacity");
        let finishing = {
            let state = runtime.clone();
            tokio::spawn(async move {
                loop {
                    match state.finish_owned_prompt_abort(&dispatch, false).await {
                        Err(DaemonError::PtyCleanup { .. }) => {
                            tokio::time::sleep(std::time::Duration::from_millis(10)).await
                        }
                        result => return result,
                    }
                }
            })
        };
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        let before_publication = runtime
            .owned
            .session_snapshot(session.id())
            .unwrap()
            .active_prompt_for_agent(agent.id())
            .map(|prompt| prompt.status());
        drop(capacity);
        let pumped = tokio::time::timeout(std::time::Duration::from_secs(3), pump)
            .await
            .unwrap()
            .unwrap();
        let finished = tokio::time::timeout(std::time::Duration::from_secs(3), finishing)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(before_publication, Some(crate::session::PromptStatus::Cancelling),
            "Stop released the exact cancelled turn while the normal pump still held unpublished output");
        pumped.unwrap();
        finished.unwrap();
        assert!(runtime
            .owned
            .session_snapshot(session.id())
            .unwrap()
            .active_prompt_for_agent(agent.id())
            .is_none());
        assert_cancelled_tool_history(&runtime, session.id());
        return;
    }
    // The finisher has read its activity, but physical cleanup awaits this lock.
    let entered = Arc::new(tokio::sync::Notify::new());
    runtime.observe_app_lock_wait_for_test(entered.clone());
    let held = app.lock().await;
    let cancelling_prompt_id = dispatch.prompt_id.clone();
    let finishing = {
        let state = runtime.clone();
        tokio::spawn(async move {
            loop {
                match state.finish_owned_prompt_abort(&dispatch, false).await {
                    Err(DaemonError::PtyCleanup { .. }) => {
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await
                    }
                    result => return result,
                }
            }
        })
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), entered.notified())
        .await
        .unwrap();
    let batch = crate::provider::ProviderPromptSignalBatch {
        chunks: vec![
            crate::provider::ProviderPromptChunk {
                kind: crate::terminal::TerminalOutputKind::ProviderTool,
                merge_key: Some("tool-a".into()),
                bytes: br#"{"id":"tool-a","tool":"bash","status":"completed"}"#.to_vec(),
            },
            crate::provider::ProviderPromptChunk {
                kind: crate::terminal::TerminalOutputKind::ProviderTool,
                merge_key: Some("tool-b".into()),
                bytes: br#"{"id":"tool-b","tool":"bash","status":"running"}"#.to_vec(),
            },
        ],
        ..Default::default()
    };
    if queued {
        runtime
            .owned
            .structured_output_records
            .mark_poll_enqueued(run.id(), Some(cancelling_prompt_id));
        runtime
            .owned
            .provider_store
            .write()
            .push_finished_structured_output_poll_for_test(run.id().into(), Ok(Some(batch)));
        runtime
            .owned
            .provider_store
            .write()
            .push_finished_structured_output_poll_for_test("unrelated-ready-run".into(), Ok(None));
    } else {
        runtime
            .apply_owned_structured_output_batch(
                session.id(),
                run.id(),
                vec![attachment.id().into()],
                batch,
            )
            .await
            .unwrap();
    }
    drop(held);
    tokio::time::timeout(std::time::Duration::from_secs(2), finishing)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    if queued {
        runtime
            .pump_owned_provider_output(session.id(), run.id(), vec![attachment.id().into()], true)
            .await
            .unwrap();
        assert_eq!(
            runtime
                .owned
                .provider_store
                .drain_finished_structured_output_poll_jobs_for_run("unrelated-ready-run")
                .len(),
            1,
            "cancellation must not consume another run's received output"
        );
    }
    assert_cancelled_tool_history(&runtime, session.id());
}

fn assert_cancelled_tool_history(runtime: &KernelRuntimeState, session_id: &str) {
    let updates: Vec<serde_json::Value> = runtime
        .owned
        .terminal_stream
        .output_records()
        .into_iter()
        .filter(|record| record.kind == crate::terminal::TerminalOutputKind::ProviderTool)
        .map(|record| serde_json::from_slice(&record.bytes).unwrap())
        .collect();
    assert!(
        !updates
            .iter()
            .any(|row| row["id"] == "tool-a" && row["status"] == "cancelled"),
        "completion received during cleanup was overwritten: {updates:?}"
    );
    assert!(
        updates
            .iter()
            .any(|row| row["id"] == "tool-b" && row["status"] == "cancelled"),
        "a tool started during cleanup was missed: {updates:?}"
    );
    let history = runtime
        .owned
        .operational_history_store
        .query_events(crate::history::HistoryEventQuery {
            session_id: Some(session_id.into()),
            kind: Some("provider_tool".into()),
            ..Default::default()
        })
        .unwrap();
    let rows: Vec<serde_json::Value> = history
        .iter()
        .filter_map(|event| event.content.as_deref())
        .map(|text| serde_json::from_str(text).unwrap())
        .collect();
    assert!(
        !rows
            .iter()
            .any(|row| row["id"] == "tool-a" && row["status"] == "cancelled"),
        "{rows:?}"
    );
    assert!(
        rows.iter()
            .any(|row| row["id"] == "tool-b" && row["status"] == "cancelled"),
        "{rows:?}"
    );
}
