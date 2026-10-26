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
    assert_unresponsive_abort_settles(false, false).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn native_cancellation_reaps_a_tool_in_its_own_process_group() {
    assert_unresponsive_abort_settles(true, false).await;
}

#[tokio::test]
async fn pending_provider_reap_keeps_cancellation_owned_until_retry_succeeds() {
    assert_unresponsive_abort_settles(false, true).await;
}

async fn assert_unresponsive_abort_settles(detached_tool: bool, deferred_reap: bool) {
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
        let script = "import os,time,sys\npid=os.fork()\nif pid==0:\n os.setsid()\n open(sys.argv[1],'w').write(str(os.getpid()))\n while os.path.exists(sys.argv[2]): time.sleep(.02)\nelse: os.waitpid(pid,0)";
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
