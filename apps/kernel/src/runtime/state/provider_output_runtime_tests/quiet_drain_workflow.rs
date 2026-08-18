use super::*;

async fn wait_for_workflow_prompt_delivery(
    runtime: &KernelRuntimeState,
    session_id: &str,
    agent_id: &str,
    workflow_run_id: &str,
    node_run_index: usize,
) -> crate::session::RuntimeSession {
    for _ in 0..200 {
        let session = runtime
            .owned
            .session_snapshot(session_id)
            .expect("session snapshot should exist");
        let node_run = session
            .workflow_run(workflow_run_id)
            .and_then(|run| run.node_runs().get(node_run_index));
        let delivered = node_run.is_some_and(|node_run| {
            node_run.status() == crate::session::WorkflowNodeRunStatus::Running
                && session
                    .active_prompt_for_agent(agent_id)
                    .is_some_and(|prompt| {
                        prompt.workflow_node_run_id() == Some(node_run.id())
                            && prompt.durable_delivery_phase()
                                == Some(crate::session::DurablePromptDeliveryPhase::Delivered)
                    })
        });
        if delivered {
            return session;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("workflow prompt should be dispatched and delivered");
}

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

    tokio::time::sleep(std::time::Duration::from_millis(60)).await;
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
async fn codex_completion_output_does_not_settle_before_authoritative_turn_completion() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-codex-terminal-gate",
            "worktree-codex-terminal-gate",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-codex-terminal-gate",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.6",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-codex-terminal-gate",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-codex-terminal-gate".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("ws://test-codex-runtime".to_string()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());
    let prompt = crate::session::PromptQueueItem::new(
        "prompt-codex-terminal-gate",
        attachment.id(),
        agent.id(),
        "complete only after turn/completed",
        crate::session::PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Started { prompt } = app
        .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start")
    else {
        panic!("prompt should start immediately");
    };
    let active_prompt_id = prompt.id().to_string();
    app.mark_active_prompt_delivery(
        session.id(),
        agent.id(),
        prompt.id(),
        crate::session::DurablePromptDeliveryPhase::Delivered,
        Some(run.id().to_string()),
        run.provider_session_id().map(str::to_string),
    )
    .expect("active prompt should be delivered before provider completion");
    let queued_prompt = crate::session::PromptQueueItem::new(
        "prompt-codex-terminal-gate-next",
        attachment.id(),
        agent.id(),
        "run only after turn/completed",
        crate::session::PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Queued { .. } = app
        .prompt_owner_submit_prepared_prompt(session.id(), queued_prompt, false)
        .expect("second prompt should queue")
    else {
        panic!("second prompt should remain queued");
    };

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime.owned.mark_prompt_completion_recorded(run.id());
    if let Some(activity) = runtime.owned.prompt_activity.write().get_mut(run.id()) {
        activity.saw_response_content = true;
        activity.last_output_at =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
        activity.settlement_requested = true;
    }
    runtime
        .settle_owned_provider_prompt(session.id(), run.id(), false, false, false)
        .await
        .expect("quiet completion evidence should be accepted");
    assert_eq!(
        runtime
            .owned
            .session_snapshot(session.id())
            .expect("session snapshot should exist")
            .active_prompt_for_agent(agent.id())
            .map(|prompt| prompt.id().to_string()),
        Some(active_prompt_id),
        "Codex assistant output must keep the current prompt active before turn/completed"
    );

    runtime
        .settle_owned_provider_prompt(session.id(), run.id(), true, false, false)
        .await
        .expect("authoritative turn completion should be accepted");
    let settled_session = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert_eq!(
        settled_session
            .active_prompt_for_agent(agent.id())
            .map(|prompt| prompt.prompt().to_string()),
        Some("run only after turn/completed".to_string()),
        "turn/completed should release the current prompt and advance exactly one queued prompt"
    );
}

#[tokio::test]
async fn metaagent_quiet_drain_settlement_recovers_orphaned_task() {
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

    tokio::time::sleep(std::time::Duration::from_millis(75)).await;
    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch::default(),
        )
        .await
        .expect("quiet drain should settle prompt");

    assert_eq!(
        runtime
            .owned
            .metaagent_events
            .list(agent.id(), Some("metaagent.task.orphaned"), None, 10)
            .len(),
        1,
        "a settled Meta turn with an active task must request a final task decision"
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
async fn workflow_prompt_with_completed_tool_advances_when_output_pump_consumed_completion_signal()
{
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
    let crate::session::PromptSubmissionOutcome::Started { prompt } = app
        .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("workflow prompt should start")
    else {
        panic!("workflow prompt should start immediately");
    };
    app.mark_active_prompt_delivery(
        session.id(),
        first_agent.id(),
        prompt.id(),
        crate::session::DurablePromptDeliveryPhase::Delivered,
        Some(run.id().to_string()),
        run.provider_session_id().map(str::to_string),
    )
    .expect("workflow prompt should be delivered before provider completion");
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            Vec::new(),
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![
                    crate::provider::ProviderPromptChunk {
                        kind: crate::terminal::TerminalOutputKind::ProviderTool,
                        merge_key: Some("workflow-ack-call".to_string()),
                        bytes: br#"{"id":"workflow-ack-call","tool":"ack_workflow_turn","status":"running"}"#
                            .to_vec(),
                    },
                    crate::provider::ProviderPromptChunk {
                        kind: crate::terminal::TerminalOutputKind::ProviderTool,
                        merge_key: Some("workflow-ack-call".to_string()),
                        bytes: br#"{"id":"workflow-ack-call","tool":"ack_workflow_turn","status":"completed"}"#
                            .to_vec(),
                    },
                    crate::provider::ProviderPromptChunk {
                        kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                        merge_key: Some("assistant-final".to_string()),
                        bytes: b"```".to_vec(),
                    },
                ],
                completions: vec![crate::provider::ProviderAssistantCompletion {
                    message_id: "assistant-final".to_string(),
                    completed_at_ms: crate::session::unix_epoch_ms(),
                }],
                prompt_completed: true,
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("partial structured output should be accepted");

    tokio::time::sleep(std::time::Duration::from_millis(60)).await;
    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            Vec::new(),
            crate::provider::ProviderPromptSignalBatch::default(),
        )
        .await
        .expect("quiet poll should keep waiting for late structured output");

    let partial_session = runtime
        .owned
        .session_snapshot(session.id())
        .expect("partial session snapshot should exist");
    assert!(
        partial_session
            .active_prompt_for_agent(first_agent.id())
            .is_some(),
        "a partial structured block must not fail or settle the workflow prompt"
    );

    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            Vec::new(),
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                    merge_key: Some("assistant-final".to_string()),
                    bytes: br#"json
{"summary":"sent","output":{"message":"{\"value\":1842}"}}
```"#
                        .to_vec(),
                }],
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("late structured output should be accepted");
    let provider_output = runtime
        .owned
        .operational_history_store
        .load_session_history_entries(session.id(), Some(first_agent.id()))
        .expect("workflow output history should load")
        .into_iter()
        .filter(|entry| {
            entry.provider_run_id.as_deref() == Some(run.id())
                && entry.kind == crate::history::SessionHistoryEntryKind::ProviderOutput
        })
        .map(|entry| entry.text)
        .collect::<String>();
    assert_eq!(
        provider_output,
        "```json\n{\"summary\":\"sent\",\"output\":{\"message\":\"{\\\"value\\\":1842}\"}}\n```"
    );
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    runtime
        .owned
        .structured_output_records
        .mark_poll_enqueued(run.id(), Some("prompt-1".to_string()));
    runtime
        .owned
        .provider_store
        .write()
        .push_finished_structured_output_poll_for_test(run.id().to_string(), Ok(None));
    runtime
        .pump_owned_structured_provider_output(session.id(), run.id(), Vec::new())
        .await
        .expect("output pump should settle after consuming the completion signal");

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
    assert_eq!(
        resolved_run.node_runs()[0].status(),
        crate::session::WorkflowNodeRunStatus::Completed
    );
    assert_eq!(
        resolved_run.status(),
        crate::session::WorkflowRunStatus::Waiting
    );
    assert_eq!(resolved_run.node_runs().len(), 2);
    assert_eq!(
        resolved_run.node_runs()[1].status(),
        crate::session::WorkflowNodeRunStatus::Ready
    );
    assert!(
        !app.lock().await.pty().has_process(run.id()),
        "a completed workflow provider run must release its managed PTY before a later run can launch with fresh runtime credentials"
    );

    runtime
        .owned
        .durable_state_store
        .load_events_by_kind("workflow.runtime.updated")
        .expect("durable workflow events should load")
        .into_iter()
        .rev()
        .find(|event| {
            event.subject_id.as_deref() == Some(session.id())
                && event
                    .payload
                    .get("reason")
                    .and_then(serde_json::Value::as_str)
                    == Some("workflow_prompt_completed")
        })
        .expect("workflow completion should persist a bounded transition");
    let durable_run = runtime
        .owned
        .durable_state_store
        .resolve_workflow_run(session.host_daemon_id(), session.id(), workflow_run.id())
        .expect("durable workflow run should load")
        .expect("durable workflow run should exist");
    assert_eq!(
        durable_run.node_runs()[0].status(),
        crate::session::WorkflowNodeRunStatus::Completed,
        "a kernel restart must not restore the completed node as running"
    );
    assert_eq!(
        durable_run.status(),
        crate::session::WorkflowRunStatus::Waiting
    );
    assert_eq!(durable_run.node_runs().len(), 2);
    assert_eq!(
        durable_run.node_runs()[1].status(),
        crate::session::WorkflowNodeRunStatus::Ready,
        "a kernel restart must preserve the advanced downstream node"
    );
}

#[tokio::test]
async fn workflow_prompt_without_structured_output_schedules_a_corrective_turn() {
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
        .create_workflow(session.id(), Some("missing-output-retry".to_string()))
        .expect("workflow should be created");
    let node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), agent.id())
        .expect("node should be added");
    app.sessions_mut()
        .set_workflow_node_can_complete_run(session.id(), workflow.id(), node.id(), true)
        .expect("node should be allowed to complete the run");
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
            Some("return challenge RUNTIME-RETRY".to_string()),
        )
        .expect("workflow run should be created");
    let first_node_run_id = workflow_run.node_runs()[0].id().to_string();
    app.sessions_mut()
        .prepare_workflow_turn(
            session.id(),
            workflow_run.id(),
            &first_node_run_id,
            format!("workflow-ack:{first_node_run_id}"),
            "workflow node prompt".to_string(),
            None,
            None,
        )
        .expect("workflow turn should be prepared");
    app.sessions_mut()
        .start_workflow_node_run(session.id(), workflow_run.id(), &first_node_run_id)
        .expect("workflow node run should start");
    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
        agent.id(),
        "workflow node prompt".to_string(),
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(workflow_run.id(), &first_node_run_id);
    let crate::session::PromptSubmissionOutcome::Started { prompt } = app
        .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("workflow prompt should start")
    else {
        panic!("workflow prompt should start immediately");
    };
    app.mark_active_prompt_delivery(
        session.id(),
        agent.id(),
        prompt.id(),
        crate::session::DurablePromptDeliveryPhase::Delivered,
        Some(run.id().to_string()),
        run.provider_session_id().map(str::to_string),
    )
    .expect("workflow prompt should be delivered before provider completion");
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
                    merge_key: Some("assistant-without-structured-output".to_string()),
                    bytes: b"I finished the workflow.".to_vec(),
                }],
                completions: vec![crate::provider::ProviderAssistantCompletion {
                    message_id: "assistant-without-structured-output".to_string(),
                    completed_at_ms: crate::session::unix_epoch_ms(),
                }],
                prompt_completed: false,
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("plain completion should be accepted while output drains");
    tokio::time::sleep(std::time::Duration::from_millis(
        crate::app::provider_output::STRUCTURED_OUTPUT_EMPTY_POLL_BACKOFF_MS + 75,
    ))
    .await;
    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            Vec::new(),
            crate::provider::ProviderPromptSignalBatch::default(),
        )
        .await
        .expect("quiet drain should settle and schedule a correction");

    let session_state =
        wait_for_workflow_prompt_delivery(&runtime, session.id(), agent.id(), workflow_run.id(), 1)
            .await;
    let resolved_run = session_state
        .workflow_run(workflow_run.id())
        .expect("workflow run should exist");
    assert_eq!(resolved_run.node_runs().len(), 2);
    assert_eq!(
        resolved_run.node_runs()[0].status(),
        crate::session::WorkflowNodeRunStatus::Failed
    );
    let correction_node_run = &resolved_run.node_runs()[1];
    assert_eq!(
        correction_node_run.status(),
        crate::session::WorkflowNodeRunStatus::Running
    );
    let correction_prompt = correction_node_run
        .turn_envelope()
        .and_then(|envelope| envelope.rendered_prompt())
        .expect("corrective workflow turn should preserve its rendered prompt");
    assert!(correction_prompt.contains("return challenge RUNTIME-RETRY"));
    assert!(correction_prompt.contains(
        "The previous workflow turn ended without the required validated structured output"
    ));
    assert!(correction_prompt.contains("validate_and_submit_workflow_run_output"));
    assert!(resolved_run.final_output().is_none());
    assert!(resolved_run.failure_events().iter().any(|event| {
        event.kind() == crate::session::WorkflowFailureKind::MissingStructuredOutput
            && event.source_node_run_id() == first_node_run_id
    }));
    let active_prompt = session_state
        .active_prompt_for_agent(agent.id())
        .expect("corrective workflow prompt should be active");
    assert_eq!(
        active_prompt.workflow_node_run_id(),
        Some(correction_node_run.id())
    );
    assert!(active_prompt.prompt().contains(
        "The previous workflow turn ended without the required validated structured output"
    ));
    let correction_prompt_id = active_prompt.id().to_string();
    let correction_provider_run = runtime
        .owned
        .provider_store
        .get_run_for_agent(session.id(), agent.id())
        .expect("corrective workflow provider should be active");
    let correction_provider_run_id = correction_provider_run.id().to_string();
    runtime
        .owned
        .session_store
        .write()
        .submit_workflow_run_final_output(
            session.id(),
            workflow_run.id(),
            correction_node_run.id(),
            crate::session::WorkflowOutputPayload::new(
                r#"{"summary":"corrected","output":{"message":"done"}}"#,
                Vec::<crate::session::WorkflowArtifactRef>::new(),
            ),
            true,
            None,
        )
        .expect("corrective workflow output should validate");

    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            &correction_provider_run_id,
            Vec::new(),
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                    merge_key: Some("corrective-structured-output".to_string()),
                    bytes: b"```json\n{\"summary\":\"corrected\",\"output\":{\"message\":\"done\"}}\n```"
                        .to_vec(),
                }],
                completions: vec![crate::provider::ProviderAssistantCompletion {
                    message_id: "corrective-structured-output".to_string(),
                    completed_at_ms: crate::session::unix_epoch_ms(),
                }],
                prompt_completed: true,
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("corrective structured output should begin settling");
    runtime
        .settle_owned_provider_prompt(
            session.id(),
            &correction_provider_run_id,
            true,
            false,
            false,
        )
        .await
        .expect("corrective workflow completion should request settlement");
    tokio::time::sleep(std::time::Duration::from_millis(
        crate::app::provider_output::STRUCTURED_OUTPUT_EMPTY_POLL_BACKOFF_MS + 75,
    ))
    .await;
    runtime
        .settle_owned_provider_prompt(
            session.id(),
            &correction_provider_run_id,
            false,
            false,
            true,
        )
        .await
        .expect("corrective workflow prompt should settle after output drains");

    let settled_session = runtime
        .owned
        .session_snapshot(session.id())
        .expect("settled session snapshot should exist");
    assert!(settled_session
        .active_prompt_for_agent(agent.id())
        .is_none());
    assert_eq!(
        settled_session
            .workflow_run(workflow_run.id())
            .expect("workflow run should exist")
            .status(),
        crate::session::WorkflowRunStatus::Completed
    );
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(&correction_provider_run_id)
            .expect("provider run should exist")
            .state(),
        crate::provider::ProviderRunState::Ended
    );
    {
        let app = app.lock().await;
        assert!(
            !app.pty().has_process(run.id()),
            "retiring a workflow provider run must remove its managed PTY before the agent can be launched again"
        );
        assert!(
            !app.provider_process_tracking
                .read()
                .run_processes
                .contains_key(run.id()),
            "retiring a workflow provider run must remove its managed process tracking"
        );
    }
    assert!(runtime.owned.active_turns.get(run.id()).is_none());

    let settlement_events = runtime
        .owned
        .operational_history_store
        .load_session_events_for_agent_sequence_range(session.id(), agent.id(), 0, i64::MAX as u64)
        .expect("operational prompt history should load");
    assert!(settlement_events.iter().any(|event| {
        event.prompt_id.as_deref() == Some(correction_prompt_id.as_str())
            && event
                .metadata
                .contains_key(crate::history::PROMPT_SETTLED_AT_MS_METADATA_KEY)
    }));

    let durable_run = runtime
        .owned
        .durable_state_store
        .resolve_workflow_run(session.host_daemon_id(), session.id(), workflow_run.id())
        .expect("durable workflow run should load")
        .expect("workflow settlement should persist the run");
    assert_eq!(
        durable_run.status(),
        crate::session::WorkflowRunStatus::Completed
    );
}

#[tokio::test]
async fn runtime_owned_invalid_handoff_schedules_one_classifier_correction() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, classifier_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let specialist_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "codex").with_alias("specialist"),
        )
        .expect("specialist should spawn");
    let provider_run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(classifier_agent.id()),
        )
        .expect("provider run should launch");
    app.update_provider_run_projection(provider_run.clone());

    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), Some("runtime-handoff-correction".to_string()))
        .expect("workflow should be created");
    let classifier = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), classifier_agent.id())
        .expect("classifier node should be added");
    let specialist = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), specialist_agent.id())
        .expect("specialist node should be added");
    let schema = std::env::temp_dir().join(format!(
        "chariox-runtime-owned-handoff-{}-{}.json",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::write(
        &schema,
        r#"{"type":"object","required":["task"],"properties":{"task":{"type":"string"}},"additionalProperties":false}"#,
    )
    .expect("schema should write");
    let edge = app
        .sessions_mut()
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            classifier.id(),
            specialist.id(),
            Some(schema.to_string_lossy().to_string()),
            Some(crate::session::WorkflowHandoffValidationPolicy::Halt),
        )
        .expect("classifier should connect to specialist");
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            classifier.id(),
            Some("entry".to_string()),
        )
        .expect("endpoint should be created");
    let workflow_run = app
        .sessions_mut()
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("classify runtime task".to_string()),
        )
        .expect("workflow run should be created");
    let first_node_run_id = workflow_run.node_runs()[0].id().to_string();
    app.sessions_mut()
        .prepare_workflow_turn(
            session.id(),
            workflow_run.id(),
            &first_node_run_id,
            format!("workflow-ack:{first_node_run_id}"),
            "workflow classifier prompt".to_string(),
            None,
            None,
        )
        .expect("classifier turn should be prepared");
    app.sessions_mut()
        .start_workflow_node_run(session.id(), workflow_run.id(), &first_node_run_id)
        .expect("classifier should start");
    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
        classifier_agent.id(),
        "workflow classifier prompt".to_string(),
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(workflow_run.id(), &first_node_run_id);
    let crate::session::PromptSubmissionOutcome::Started { prompt } = app
        .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("classifier prompt should start")
    else {
        panic!("classifier prompt should start immediately");
    };
    app.mark_active_prompt_delivery(
        session.id(),
        classifier_agent.id(),
        prompt.id(),
        crate::session::DurablePromptDeliveryPhase::Delivered,
        Some(provider_run.id().to_string()),
        provider_run.provider_session_id().map(str::to_string),
    )
    .expect("classifier prompt should be delivered before provider completion");
    crate::transport::flow_control::note_prompt_started(&mut app, provider_run.id());

    let output = format!(
        "```json\n{{\"summary\":\"classified\",\"workflow_handoffs\":[{{\"edge_id\":\"{}\",\"output\":{{\"message\":{{\"wrong\":true}}}}}}],\"output\":{{\"message\":\"plain classifier note\"}}}}\n```",
        edge.id()
    );
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            provider_run.id(),
            Vec::new(),
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                    merge_key: Some("invalid-handoff-output".to_string()),
                    bytes: output.into_bytes(),
                }],
                completions: vec![crate::provider::ProviderAssistantCompletion {
                    message_id: "invalid-handoff-output".to_string(),
                    completed_at_ms: crate::session::unix_epoch_ms(),
                }],
                prompt_completed: true,
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("invalid handoff output should begin settling");
    runtime
        .settle_owned_provider_prompt(session.id(), provider_run.id(), true, false, false)
        .await
        .expect("invalid handoff completion should request settlement");
    tokio::time::sleep(std::time::Duration::from_millis(
        crate::app::provider_output::STRUCTURED_OUTPUT_EMPTY_POLL_BACKOFF_MS + 75,
    ))
    .await;
    runtime
        .settle_owned_provider_prompt(session.id(), provider_run.id(), false, false, true)
        .await
        .expect("invalid handoff should settle into correction");

    let session_state = wait_for_workflow_prompt_delivery(
        &runtime,
        session.id(),
        classifier_agent.id(),
        workflow_run.id(),
        1,
    )
    .await;
    let resolved_run = session_state
        .workflow_run(workflow_run.id())
        .expect("workflow run should exist");
    assert_eq!(
        resolved_run.status(),
        crate::session::WorkflowRunStatus::Running
    );
    assert_eq!(resolved_run.node_runs().len(), 2);
    assert_eq!(
        resolved_run.node_runs()[0].status(),
        crate::session::WorkflowNodeRunStatus::Failed
    );
    assert_eq!(resolved_run.node_runs()[1].node_id(), classifier.id());
    assert_eq!(
        resolved_run
            .node_runs()
            .iter()
            .filter(|node_run| node_run.node_id() == specialist.id())
            .count(),
        0
    );
    assert_eq!(
        resolved_run
            .messages()
            .iter()
            .filter(|message| message.message_type() == "handoff")
            .count(),
        0
    );
    let correction_prompt = session_state
        .active_prompt_for_agent(classifier_agent.id())
        .expect("classifier correction should be active")
        .prompt();
    assert_eq!(
        correction_prompt.matches("classify runtime task").count(),
        1
    );
    assert!(resolved_run.failure_events().iter().any(|event| {
        event.kind() == crate::session::WorkflowFailureKind::OutputValidationFailed
            && event.source_node_run_id() == first_node_run_id
    }));
    std::fs::remove_file(schema).ok();
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
