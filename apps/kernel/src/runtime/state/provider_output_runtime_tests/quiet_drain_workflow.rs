use super::*;

#[tokio::test]
async fn provider_message_completion_without_prompt_completed_settles_after_quiet_drain() {
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
            "client-completion-quiet-drain",
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
                prompt_completed: false,
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("completion batch with output should be accepted");
    assert_eq!(records.len(), 1);
    assert!(
        runtime
            .owned
            .session_snapshot(session.id())
            .expect("session snapshot should exist")
            .active_prompt_for_agent(agent.id())
            .is_some(),
        "completion with fresh output should wait for a quiet drain"
    );

    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch::default(),
        )
        .await
        .expect("quiet drain should settle prompt");

    let settled_session = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(
        settled_session
            .active_prompt_for_agent(agent.id())
            .is_none(),
        "assistant completion without prompt_completed should settle after quiet drain"
    );
}

#[tokio::test]
async fn metaagent_quiet_drain_settlement_without_prompt_completed_does_not_inject_orphaned_task() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let agent = app
        .agents_mut()
        .activate_agent_meta_mode(agent.id(), None)
        .expect("agent should enter meta mode");
    app.sessions_mut()
        .start_or_update_metaagent_task(
            session.id(),
            agent.id(),
            "Verify quiet-drain settlement does not orphan an active Meta task.",
        )
        .expect("metaagent task should start");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-meta-completion-quiet-drain",
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
        "continue the active task\n",
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
                    merge_key: Some("assistant-final".to_string()),
                    bytes: b"meta turn output".to_vec(),
                }],
                completions: vec![crate::provider::ProviderAssistantCompletion {
                    message_id: "assistant-final".to_string(),
                    completed_at_ms: crate::session::unix_epoch_ms(),
                }],
                prompt_completed: false,
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("completion batch with output should be accepted");
    assert!(
        runtime
            .owned
            .metaagent_events
            .list(agent.id(), Some("metaagent.task.orphaned"), None, 10)
            .is_empty(),
        "fresh assistant output should not inject orphan recovery before provider completion"
    );

    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch::default(),
        )
        .await
        .expect("quiet drain should settle prompt");

    assert!(
        runtime
            .owned
            .metaagent_events
            .list(agent.id(), Some("metaagent.task.orphaned"), None, 10)
            .is_empty(),
        "quiet-drain settlement without prompt_completed must not resuscitate a Meta task"
    );
}

#[tokio::test]
async fn provider_quiet_gap_does_not_settle_without_completion_signal() {
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
            "client-quiet-gap",
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
        "long quiet turn\n",
        Vec::new(),
    )
    .expect("prompt should start");

    if let Some(state) = app.prompt_activity.write().get_mut(run.id()) {
        state.saw_response_content = true;
        state.last_output_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(5));
    } else {
        panic!("prompt activity should exist for the active run");
    }

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let settlement = runtime
        .settle_owned_provider_prompt(session.id(), run.id(), false, false, false)
        .await
        .expect("quiet provider poll should be accepted");
    assert!(settlement.had_active_prompt);
    assert!(!settlement.started_next_prompt);

    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(session_state.active_prompt_for_agent(agent.id()).is_some());
    let activity = runtime.agent_activity_for_session(&session_state);
    let agent_activity = activity
        .get(agent.id())
        .expect("agent activity should be projected");
    assert_eq!(
        agent_activity.status,
        crate::runtime::projection::AgentRuntimeStatus::Working
    );
    assert_eq!(
        agent_activity.prompt_status,
        crate::runtime::projection::AgentPromptRuntimeStatus::Running
    );
    assert!(agent_activity.busy);
    assert!(agent_activity.active_turn.is_some());
}

#[tokio::test]
async fn workflow_prompt_settles_after_structured_message_completion_drain() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, first_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let second_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "codex").with_alias("second"),
        )
        .expect("second agent should spawn");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(first_agent.id()),
        )
        .expect("provider run should launch");
    app.update_provider_run_projection(run.clone());

    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), Some("completion-gate".to_string()))
        .expect("workflow should be created");
    let first_node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), first_agent.id())
        .expect("first node should be added");
    let second_node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), second_agent.id())
        .expect("second node should be added");
    app.sessions_mut()
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            first_node.id(),
            second_node.id(),
            None,
            None,
        )
        .expect("edge should be added");
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            first_node.id(),
            Some("entry".to_string()),
        )
        .expect("endpoint should be created");
    let workflow_run = app
        .sessions_mut()
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("run the workflow".to_string()),
        )
        .expect("workflow run should be created");
    let node_run_id = workflow_run.node_runs()[0].id().to_string();
    app.sessions_mut()
        .prepare_workflow_turn(
            session.id(),
            workflow_run.id(),
            &node_run_id,
            format!("workflow-ack:{node_run_id}"),
            "workflow node prompt".to_string(),
            None,
            None,
        )
        .expect("workflow turn should be prepared");
    app.sessions_mut()
        .start_workflow_node_run(session.id(), workflow_run.id(), &node_run_id)
        .expect("workflow node run should start");
    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
        first_agent.id(),
        "workflow node prompt".to_string(),
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(workflow_run.id(), &node_run_id);
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("workflow prompt should start");
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            Vec::new(),
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                    merge_key: Some("assistant-final".to_string()),
                    bytes: br#"```json
{"summary":"sent","output":{"message":"{\"value\":1842}"}}
```"#
                        .to_vec(),
                }],
                completions: vec![crate::provider::ProviderAssistantCompletion {
                    message_id: "assistant-final".to_string(),
                    completed_at_ms: crate::session::unix_epoch_ms(),
                }],
                prompt_completed: false,
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("structured output should be accepted");
    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            Vec::new(),
            crate::provider::ProviderPromptSignalBatch::default(),
        )
        .await
        .expect("quiet poll should settle a drained completed workflow prompt");

    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(
        session_state
            .active_prompt_for_agent(first_agent.id())
            .is_none(),
        "workflow prompt must settle after assistant completion drains"
    );
    let resolved_run = session_state
        .workflow_run(workflow_run.id())
        .expect("workflow run should exist");
    assert_eq!(resolved_run.node_runs().len(), 2);
    assert_eq!(
        resolved_run.node_runs()[0].status(),
        crate::session::WorkflowNodeRunStatus::Completed
    );
    assert_eq!(
        resolved_run.node_runs()[1].status(),
        crate::session::WorkflowNodeRunStatus::Ready
    );
}

#[tokio::test]
async fn workflow_reasoning_records_thinking_from_prompt_owner_context() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
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

    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), Some("thinking-trace".to_string()))
        .expect("workflow should be created");
    let node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), agent.id())
        .expect("node should be added");
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("endpoint should be created");
    let workflow_run = app
        .sessions_mut()
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("run the workflow".to_string()),
        )
        .expect("workflow run should be created");
    let node_run_id = workflow_run.node_runs()[0].id().to_string();
    app.sessions_mut()
        .prepare_workflow_turn(
            session.id(),
            workflow_run.id(),
            &node_run_id,
            format!("workflow-ack:{node_run_id}"),
            "workflow node prompt".to_string(),
            None,
            None,
        )
        .expect("workflow turn should be prepared");
    app.sessions_mut()
        .start_workflow_node_run(session.id(), workflow_run.id(), &node_run_id)
        .expect("workflow node run should start");
    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
        agent.id(),
        "workflow node prompt".to_string(),
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(workflow_run.id(), &node_run_id);
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("workflow prompt should start");
    app.sessions
        .mirror_agent_prompt_state(session.id(), agent.id(), None, VecDeque::new())
        .expect("session mirror should be cleared for regression coverage");
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            Vec::new(),
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderReasoning,
                    merge_key: Some("thinking-1".to_string()),
                    bytes: b"real provider reasoning".to_vec(),
                }],
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("structured reasoning should be accepted");

    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    let node_run = session_state
        .workflow_run(workflow_run.id())
        .and_then(|run| {
            run.node_runs()
                .iter()
                .find(|node_run| node_run.id() == node_run_id)
        })
        .expect("workflow node run should exist");
    assert_eq!(node_run.thinking_traces().len(), 1);
    assert_eq!(
        node_run.thinking_traces()[0].message(),
        "real provider reasoning"
    );
}
