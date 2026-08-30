use super::*;

#[tokio::test]
async fn managed_activity_reaches_zero_only_after_prompt_settlement_is_durable() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-managed-activity-settlement",
            "worktree-managed-activity-settlement",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-managed-activity-settlement",
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
    let crate::session::PromptSubmissionOutcome::Started { prompt } = app
        .submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "finish the managed turn",
            Vec::new(),
        )
        .expect("prompt should start")
    else {
        panic!("first prompt should start immediately");
    };

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime.owned.note_prompt_started(run.id());
    assert_eq!(runtime.managed_running_agent_count(), 1);
    let response = "managed response persisted before idle";
    runtime.owned.fan_out_terminal_output(
        session.id(),
        run.id(),
        crate::terminal::TerminalOutputKind::ProviderOutput,
        Some("managed-activity-response".to_string()),
        vec![attachment.id().to_string()],
        response.as_bytes(),
    );
    runtime.owned.record_assistant_message_completion(
        session.id(),
        run.id(),
        vec![attachment.id().to_string()],
        "managed-activity-completion",
        crate::session::unix_epoch_ms(),
    );
    assert_eq!(runtime.managed_running_agent_count(), 1);
    let before_settlement = runtime.managed_activity_change_sequence();

    runtime
        .owned
        .complete_local_prompt_without_advance(session.id(), agent.id(), Some(run.id()))
        .expect("prompt completion should settle")
        .expect("active prompt should complete");

    assert!(runtime
        .owned
        .operational_history_store
        .load_prompt_settlement_event(session.id(), agent.id(), prompt.id())
        .expect("settlement history should load")
        .is_some());
    assert!(runtime
        .owned
        .operational_history_store
        .load_session_history_entries(session.id(), Some(agent.id()))
        .expect("durable response history should load")
        .iter()
        .any(|entry| entry.text.contains(response)));
    assert_eq!(runtime.managed_running_agent_count(), 0);
    assert!(runtime.managed_activity_change_sequence() > before_settlement);
}

#[tokio::test]
async fn provider_settlement_starts_metaagent_task_queued_behind_completed_turn() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-metaagent-settlement-fifo",
            "worktree-metaagent-settlement-fifo",
        ))
        .expect("session should be created");
    let agent = app
        .agents_mut()
        .activate_agent_meta_mode(agent.id(), None)
        .expect("agent should enter Meta mode");
    app.sessions_mut()
        .start_or_update_metaagent_task(session.id(), agent.id(), "first Meta task")
        .expect("first Meta task should start");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-metaagent-settlement-fifo",
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
        "complete the first Meta task",
        Vec::new(),
    )
    .expect("first Meta prompt should start");
    app.sessions_mut()
        .enqueue_metaagent_task(
            session.id(),
            agent.id(),
            attachment.id(),
            "second Meta task",
            Vec::new(),
        )
        .expect("second Meta task should queue");
    app.detach(attachment.id())
        .expect("the submitting browser attachment should be allowed to disappear");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .owned
        .session_store
        .write()
        .complete_metaagent_task(session.id(), agent.id(), Some("done".to_string()))
        .expect("first Meta task should complete");
    runtime
        .deactivate_meta_mode_for_terminal_task(session.id(), agent.id(), "test completion")
        .await
        .expect("terminal Meta task should deactivate");
    assert!(
        runtime
            .owned
            .agent_store
            .get_agent(agent.id())
            .expect("agent should remain available")
            .is_metaagent(),
        "Meta policy must remain loaded while another Meta task is queued"
    );

    runtime
        .settle_owned_provider_prompt(session.id(), run.id(), true, false, true)
        .await
        .expect("provider settlement should complete the active turn");

    let mut second_started = false;
    for _ in 0..50 {
        let snapshot = runtime
            .owned
            .session_store
            .get_session(session.id())
            .expect("session should remain available");
        second_started = snapshot
            .metaagent_task(agent.id())
            .is_some_and(|task| task.task_markdown() == "second Meta task")
            && snapshot.queued_metaagent_tasks().is_empty();
        if second_started {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        second_started,
        "provider settlement must start the next queued Meta task"
    );
}

#[tokio::test]
async fn duplicate_completion_before_promoted_workflow_dispatch_is_ignored() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-stale-poll-workflow-promotion",
            "worktree-stale-poll-workflow-promotion",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-stale-poll-workflow-promotion",
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
        "direct user prompt",
        Vec::new(),
    )
    .expect("direct prompt should start");
    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), Some("queued-after-user".to_string()))
        .expect("workflow should be created");
    let node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), agent.id())
        .expect("workflow node should be added");
    app.sessions_mut()
        .set_workflow_node_can_complete_run(session.id(), workflow.id(), node.id(), true)
        .expect("workflow node should complete the run");
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");
    let workflow_run = app
        .sessions_mut()
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("finish the workflow".to_string()),
        )
        .expect("workflow run should be created");
    let node_run_id = workflow_run.node_runs()[0].id().to_string();
    app.sessions_mut()
        .prepare_workflow_turn(
            session.id(),
            workflow_run.id(),
            &node_run_id,
            format!("workflow-ack:{node_run_id}"),
            "workflow prompt".to_string(),
            None,
            None,
        )
        .expect("workflow turn should be prepared");
    let workflow_prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
        agent.id(),
        "workflow prompt",
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(workflow_run.id(), &node_run_id)
    .with_durable_operation("workflow-operation-1", "workflow-fingerprint-1");
    let crate::session::PromptSubmissionOutcome::Queued { .. } = app
        .prompt_owner_submit_prepared_prompt(session.id(), workflow_prompt, false)
        .expect("workflow prompt should queue behind direct prompt")
    else {
        panic!("workflow prompt should remain queued");
    };

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime.owned.note_prompt_started(run.id());
    runtime.owned.mark_prompt_completion_recorded(run.id());
    runtime.owned.note_prompt_settlement_requested(run.id());
    let current_session = runtime
        .owned
        .session_store
        .get_session(session.id())
        .expect("session should exist");
    runtime
        .owned
        .prompt_state_owner
        .complete_active_prompt_only(&current_session, agent.id())
        .expect("direct prompt should complete");
    let promoted = runtime
        .owned
        .prompt_state_owner
        .activate_next_queued_prompt_with_prompt_id(
            &current_session,
            agent.id(),
            None,
            runtime.owned.session_store.reserve_prompt_id(),
        )
        .expect("workflow prompt promotion should succeed")
        .expect("workflow prompt should promote");
    assert_eq!(promoted.workflow_node_run_id(), Some(node_run_id.as_str()));
    assert_eq!(promoted.status(), crate::session::PromptStatus::Dispatching);
    assert_eq!(
        promoted.durable_delivery_phase(),
        Some(crate::session::DurablePromptDeliveryPhase::Accepted)
    );
    runtime
        .owned
        .prompt_state_owner
        .mark_active_prompt_running(&current_session, agent.id())
        .expect(
            "promoted prompt should be representable as running before provider acknowledgement",
        );
    let (active_prompt, queued_prompts) = runtime
        .owned
        .prompt_state_owner
        .state_parts(&current_session, agent.id());
    runtime
        .owned
        .mirror_prompt_owner_agent_state(session.id(), agent.id(), active_prompt, queued_prompts)
        .expect("running prompt state should persist");
    let promoted = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&current_session, agent.id())
        .expect("promoted workflow prompt should remain active");
    assert_eq!(promoted.status(), crate::session::PromptStatus::Running);
    let promoted_prompt_id = promoted.id().to_string();

    runtime
        .owned
        .structured_output_records
        .mark_poll_enqueued(run.id(), Some(promoted_prompt_id.clone()));
    runtime
        .owned
        .provider_store
        .write()
        .push_finished_structured_output_poll_for_test(
            run.id().to_string(),
            Ok(Some(crate::provider::ProviderPromptSignalBatch {
                completions: vec![crate::provider::ProviderAssistantCompletion {
                    message_id: "stale-direct-completion".to_string(),
                    completed_at_ms: crate::session::unix_epoch_ms(),
                }],
                prompt_completed: true,
                ..crate::provider::ProviderPromptSignalBatch::default()
            })),
        );
    runtime
        .pump_owned_structured_provider_output(session.id(), run.id(), Vec::new())
        .await
        .expect("duplicate completion should be drained before submit acknowledgement");
    tokio::time::sleep(std::time::Duration::from_millis(
        crate::app::provider_output::STRUCTURED_OUTPUT_EMPTY_POLL_BACKOFF_MS + 75,
    ))
    .await;
    runtime
        .settle_owned_provider_prompt(session.id(), run.id(), false, false, false)
        .await
        .expect("stale completion must not settle the promoted workflow prompt");

    let session_state = runtime
        .owned
        .session_store
        .get_session(session.id())
        .expect("session should exist");
    assert_eq!(
        runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session_state, agent.id())
            .map(|prompt| prompt.id().to_string()),
        Some(promoted_prompt_id.clone())
    );
    assert_eq!(
        session_state
            .workflow_run(workflow_run.id())
            .expect("workflow run should exist")
            .node_runs()[0]
            .status(),
        crate::session::WorkflowNodeRunStatus::Ready
    );
    runtime
        .owned
        .workflow_mark_prompt_started(session.id(), &promoted)
        .expect("promoted workflow prompt should enter workflow dispatch");
    let dispatched_session = runtime
        .owned
        .session_store
        .get_session(session.id())
        .expect("dispatched session should exist");
    assert_eq!(
        dispatched_session
            .workflow_run(workflow_run.id())
            .expect("workflow run should exist")
            .node_runs()[0]
            .turn_envelope()
            .expect("workflow turn envelope should exist")
            .state(),
        crate::session::WorkflowTurnRuntimeState::Dispatched
    );
    runtime
        .owned
        .mark_active_prompt_delivery(
            session.id(),
            agent.id(),
            promoted.id(),
            crate::session::DurablePromptDeliveryPhase::Dispatching,
            Some(run.id().to_string()),
            run.provider_session_id().map(str::to_string),
        )
        .expect("guarded workflow prompt should enter provider dispatch");
    runtime.owned.note_prompt_started(run.id());
    let started_session = runtime
        .owned
        .session_store
        .get_session(session.id())
        .expect("started session should exist");
    let started_node_run = &started_session
        .workflow_run(workflow_run.id())
        .expect("workflow run should exist")
        .node_runs()[0];
    assert_eq!(
        started_node_run.status(),
        crate::session::WorkflowNodeRunStatus::Running
    );
    assert!(started_node_run.started_at_ms().is_some());
}

#[tokio::test]
async fn provider_settlement_rotates_context_before_promoting_queued_workflow() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-runtime-fresh-queue",
            "worktree-runtime-fresh-queue",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "runtime-fresh-queue-client",
            crate::attachment::ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("client should attach");
    let agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("runtime-fresh-queue-agent")
                .with_model("test-model"),
        )
        .expect("workflow agent should be created");
    let stale_run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "test-model",
            )
            .with_agent_id(agent.id()),
        )
        .expect("existing provider run should launch");
    let direct_prompt = crate::session::PromptQueueItem::new(
        "runtime-user-before-workflow",
        attachment.id(),
        agent.id(),
        "finish the direct request first",
        crate::session::PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Started {
        prompt: direct_prompt,
    } = app
        .prompt_owner_submit_prepared_prompt(session.id(), direct_prompt, false)
        .expect("direct prompt should start")
    else {
        panic!("direct prompt should start immediately");
    };
    app.mark_active_prompt_delivery(
        session.id(),
        agent.id(),
        direct_prompt.id(),
        crate::session::DurablePromptDeliveryPhase::Delivered,
        Some(stale_run.id().to_string()),
        stale_run.provider_session_id().map(str::to_string),
    )
    .expect("direct prompt should be delivered");

    let workflow = app
        .sessions_mut()
        .create_workflow(
            session.id(),
            Some("fresh-after-runtime-settlement".to_string()),
        )
        .expect("workflow should be created");
    let node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), agent.id())
        .expect("workflow node should be created");
    let _endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");
    let (workflow_run, _, _) = app
        .invoke_workflow_endpoint_and_schedule(
            session.id(),
            workflow.id(),
            "entry",
            Some("run with a fresh provider context".to_string()),
        )
        .expect("workflow run should be admitted behind the direct prompt");
    let workflow_run_id = workflow_run.id().to_string();
    let workflow_node_run_id = workflow_run.node_runs()[0].id().to_string();

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let settlement = runtime
        .settle_owned_provider_prompt(session.id(), stale_run.id(), true, false, true)
        .await
        .expect("direct prompt should settle and advance the queued workflow");
    assert!(settlement.had_active_prompt);
    assert!(
        !settlement.started_next_prompt,
        "the queued workflow must not promote onto the stale provider"
    );

    let fresh_run = runtime
        .owned
        .provider_store
        .get_run_for_agent(session.id(), agent.id())
        .expect("fresh workflow provider should replace the stale provider");
    assert_ne!(fresh_run.id(), stale_run.id());
    assert!(fresh_run.workflow_tools_enabled());
    assert_eq!(
        fresh_run.workflow_fresh_context_node_run_id(),
        Some(workflow_node_run_id.as_str())
    );
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(stale_run.id())
            .expect("stale provider should remain auditable")
            .state(),
        crate::provider::ProviderRunState::Ended
    );
    let active = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(
            &runtime
                .owned
                .session_store
                .get_session(session.id())
                .expect("session should resolve"),
            agent.id(),
        )
        .expect("queued workflow should promote after provider rotation");
    assert_eq!(active.workflow_run_id(), Some(workflow_run_id.as_str()));
}

#[tokio::test]
async fn provider_completed_signal_settles_matching_active_prompt_after_quiet_interval() {
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
    assert!(runtime
        .owned
        .session_store
        .get_session(session.id())
        .expect("session should exist")
        .active_prompt_for_agent(agent.id())
        .is_some());

    tokio::time::sleep(std::time::Duration::from_millis(60)).await;
    let settled = runtime
        .settle_owned_provider_prompt(session.id(), run.id(), false, false, false)
        .await
        .expect("quiet completion follow-up should settle");
    assert!(settled.had_active_prompt);
    assert!(!settled.started_next_prompt);
    assert!(runtime
        .owned
        .session_store
        .get_session(session.id())
        .expect("session should exist")
        .active_prompt_for_agent(agent.id())
        .is_none());
    let settlement_events = runtime
        .owned
        .operational_history_store
        .load_session_events_for_agent_sequence_range(session.id(), agent.id(), 0, i64::MAX as u64)
        .expect("operational prompt history should load");
    let settlement = settlement_events
        .iter()
        .find(|event| {
            event
                .metadata
                .contains_key(crate::history::PROMPT_SETTLED_AT_MS_METADATA_KEY)
        })
        .expect("authoritative prompt settlement should be persisted");
    assert_eq!(settlement.prompt_id.as_deref(), Some("prompt-1"));
    assert_eq!(settlement.content, None, "settlement marker stays hidden");
}

#[tokio::test]
async fn detached_session_completes_active_and_two_queued_prompts_without_transient_backlog() {
    let owner_user_id = "user-detached-queue";
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            crate::session::CreateSessionRequest::new(
                "workspace-detached-queue",
                "worktree-detached-queue",
            )
            .with_owner_user_id(owner_user_id),
        )
        .expect("session should be created");
    let agent = app
        .agents_mut()
        .activate_agent_meta_mode(agent.id(), None)
        .expect("agent should enter Meta mode");
    let source_client_id = format!("metaagent:{}:detached-queue", agent.id());
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::for_user(
            session.id(),
            &source_client_id,
            crate::attachment::ClientCapabilityLevel::FullTerminal,
            owner_user_id,
        ))
        .expect("attachment should attach");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "default",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");
    app.update_provider_run_projection(run.clone());

    for prompt in [
        "first detached turn",
        "second detached turn",
        "third detached turn",
    ] {
        app.submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            prompt,
            Vec::new(),
        )
        .expect("prompt should be admitted");
    }
    app.detach(attachment.id())
        .expect("the only terminal should detach while prompts remain queued");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    assert!(runtime
        .owned
        .attachment_store
        .list_session_attachment_ids(session.id())
        .is_empty());

    for (index, expected_prompt) in [
        "first detached turn",
        "second detached turn",
        "third detached turn",
    ]
    .into_iter()
    .enumerate()
    {
        let current = runtime
            .owned
            .session_store
            .get_session(session.id())
            .expect("session should remain available");
        let active = current
            .active_prompt_for_agent(agent.id())
            .expect("the next detached prompt should be active");
        assert_eq!(active.prompt(), expected_prompt);
        assert_eq!(active.source_attachment_id(), attachment.id());
        assert_eq!(active.source_client_id(), Some(source_client_id.as_str()));
        assert_eq!(active.source_user_id(), Some(owner_user_id));
        let (resolved_client_id, resolved_user_id) = runtime
            .owned
            .active_prompt_source_attribution(session.id(), agent.id())
            .expect("structured dispatch attribution should resolve without a live attachment");
        assert_eq!(
            resolved_client_id.as_deref(),
            Some(source_client_id.as_str())
        );
        assert_eq!(resolved_user_id.as_deref(), Some(owner_user_id));
        assert_eq!(resolved_user_id.as_deref(), Some(agent.owner_user_id()));
        assert_eq!(
            crate::prompt_assembly::provider_turn_mode_for_prompt(
                agent.id(),
                agent.is_metaagent(),
                resolved_client_id.as_deref(),
                "",
            ),
            crate::prompt_assembly::PromptAssemblyMode::MetaagentProviderTurn,
            "detached structured dispatch should retain the original client mode",
        );

        let response_text = format!("completed {expected_prompt}");
        runtime.owned.fan_out_terminal_output(
            session.id(),
            run.id(),
            crate::terminal::TerminalOutputKind::ProviderOutput,
            Some(format!("detached-response-{index}")),
            Vec::new(),
            response_text.as_bytes(),
        );
        runtime.owned.record_assistant_message_completion(
            session.id(),
            run.id(),
            Vec::new(),
            &format!("detached-message-{index}"),
            crate::session::unix_epoch_ms(),
        );
        assert!(
            runtime.owned.terminal_stream.output_records().is_empty(),
            "zero-recipient terminal output must not accumulate transient records",
        );
        assert_eq!(
            runtime
                .owned
                .terminal_stream
                .health_store()
                .snapshot()
                .pending_completion_records,
            0,
            "zero-recipient assistant completions must not accumulate transient records",
        );

        let settled = runtime
            .settle_owned_provider_prompt(session.id(), run.id(), true, false, true)
            .await
            .expect("detached provider completion should settle and advance the queue");
        assert!(settled.had_active_prompt);
        assert_eq!(settled.started_next_prompt, index < 2);
    }

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let session_is_idle = runtime
                .owned
                .session_store
                .get_session(session.id())
                .expect("detached session should remain")
                .active_provider_run_id()
                .is_none();
            let provider_is_parked = runtime
                .owned
                .provider_store
                .get_run(run.id())
                .expect("detached provider run should remain")
                .state()
                == crate::provider::ProviderRunState::Parked;
            if session_is_idle && provider_is_parked {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("completed detached provider should park");

    let completed = runtime
        .owned
        .session_store
        .get_session(session.id())
        .expect("completed detached session should remain available");
    assert!(completed.active_prompt_for_agent(agent.id()).is_none());
    assert!(completed
        .queued_prompts_for_agent(agent.id())
        .is_none_or(|queued| queued.is_empty()));
    assert!(runtime.owned.terminal_stream.output_records().is_empty());
    assert_eq!(
        runtime
            .owned
            .terminal_stream
            .health_store()
            .snapshot()
            .pending_completion_records,
        0,
    );
    let history = runtime
        .owned
        .operational_history_store
        .load_session_history_entries(session.id(), Some(agent.id()))
        .expect("durable detached history should load");
    for prompt in [
        "first detached turn",
        "second detached turn",
        "third detached turn",
    ] {
        assert!(
            history.iter().any(|entry| entry.text.contains(prompt)),
            "durable history should contain `{prompt}`",
        );
    }
    for (index, response) in [
        "completed first detached turn",
        "completed second detached turn",
        "completed third detached turn",
    ]
    .into_iter()
    .enumerate()
    {
        let entry = history
            .iter()
            .find(|entry| entry.merge_key.as_deref() == Some(&format!("detached-response-{index}")))
            .expect("detached agent response should be recoverable from durable history");
        assert_eq!(entry.text, response);
        assert_eq!(entry.source_attachment_id.as_deref(), Some(attachment.id()));
    }
}

#[tokio::test]
async fn failed_prompt_dispatch_persists_terminal_prompt_settlement() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-dispatch-failure-settlement",
            "worktree-dispatch-failure-settlement",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-dispatch-failure-settlement",
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
        "prompt whose delivery will fail\n",
        Vec::new(),
    )
    .expect("prompt should start");
    let prompt_id = app
        .prompt_owner_active_prompt_for_agent(session.id(), agent.id())
        .expect("active prompt should load")
        .expect("prompt should be active")
        .id()
        .to_string();

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let dispatch = crate::app::KernelPromptDispatch {
        session_id: session.id().to_string(),
        provider_run_id: run.id().to_string(),
        agent_id: agent.id().to_string(),
        prompt_id: prompt_id.clone(),
        target_active_prompt_id: None,
        source_attachment_id: attachment.id().to_string(),
        prompt: "prompt whose delivery will fail\n".to_string(),
        hidden_system_context: String::new(),
        attachments: Vec::new(),
        prompt_origin: crate::session::PromptOrigin::Chariox,
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        steering: false,
    };
    let result = runtime
        .fail_prompt_dispatch(
            dispatch,
            crate::error::DaemonError::LocalTransport {
                operation: "test prompt dispatch",
                message: "provider did not acknowledge".to_string(),
            },
        )
        .await;
    assert!(result.is_err(), "dispatch failure should remain observable");
    assert!(runtime
        .owned
        .session_store
        .get_session(session.id())
        .expect("session should exist")
        .active_prompt_for_agent(agent.id())
        .is_none());

    let events = runtime
        .owned
        .operational_history_store
        .load_session_events_for_agent_sequence_range(session.id(), agent.id(), 0, i64::MAX as u64)
        .expect("operational prompt history should load");
    let settlement = events
        .iter()
        .find(|event| {
            event.prompt_id.as_deref() == Some(prompt_id.as_str())
                && event
                    .metadata
                    .contains_key(crate::history::PROMPT_SETTLED_AT_MS_METADATA_KEY)
        })
        .expect("failed dispatch should persist a terminal settlement marker");
    assert_eq!(
        settlement
            .metadata
            .get(crate::history::PROMPT_SETTLEMENT_STATUS_METADATA_KEY)
            .and_then(serde_json::Value::as_str),
        Some("cancelled")
    );
}

#[tokio::test]
async fn failed_prompt_dispatch_advances_the_next_queued_prompt() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-dispatch-failure-queue-advance",
            "worktree-dispatch-failure-queue-advance",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-dispatch-failure-queue-advance",
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
        "first prompt whose delivery will fail\n",
        Vec::new(),
    )
    .expect("first prompt should start");
    let first_prompt = app
        .prompt_owner_active_prompt_for_agent(session.id(), agent.id())
        .expect("active prompt should load")
        .expect("first prompt should be active");
    let queued = app
        .submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "second prompt should advance\n",
            Vec::new(),
        )
        .expect("second prompt should queue");
    let queued_prompt_id = match queued {
        crate::session::PromptSubmissionOutcome::Queued { prompt } => prompt.id().to_string(),
        other => panic!("expected queued prompt, got {other:?}"),
    };

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let dispatch = crate::app::KernelPromptDispatch {
        session_id: session.id().to_string(),
        provider_run_id: run.id().to_string(),
        agent_id: agent.id().to_string(),
        prompt_id: first_prompt.id().to_string(),
        target_active_prompt_id: None,
        source_attachment_id: attachment.id().to_string(),
        prompt: first_prompt.prompt().to_string(),
        hidden_system_context: String::new(),
        attachments: Vec::new(),
        prompt_origin: crate::session::PromptOrigin::Chariox,
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        steering: false,
    };
    let result = runtime
        .fail_prompt_dispatch(
            dispatch,
            crate::error::DaemonError::LocalTransport {
                operation: "test queued prompt dispatch",
                message: "provider did not acknowledge".to_string(),
            },
        )
        .await;
    assert!(
        result.is_err(),
        "original dispatch failure should remain observable"
    );

    let projected = runtime
        .owned
        .session_store
        .get_session(session.id())
        .expect("session should exist");
    let active = projected
        .active_prompt_for_agent(agent.id())
        .expect("queued prompt should become active");
    assert_ne!(active.id(), queued_prompt_id);
    assert_eq!(active.prompt(), "second prompt should advance\n");
    assert!(projected
        .queued_prompts_for_agent(agent.id())
        .is_none_or(|queued| queued.is_empty()));
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
        sync_external_active_prompt_and_queue_chariox_prompt(
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
    assert_external_active_prompt_and_queued_chariox_prompt(
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
        sync_external_active_prompt_and_queue_chariox_prompt(
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
    assert_external_active_prompt_and_queued_chariox_prompt(
        &runtime,
        session.id(),
        agent.id(),
        &external_prompt_id,
        &queued_prompt_id,
    );
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .expect("external provider run should remain available")
            .state(),
        crate::provider::ProviderRunState::Running,
        "observed external prompt failures must not mutate provider-owned lifecycle state"
    );
}

#[tokio::test]
async fn provider_completion_with_output_waits_for_a_quiet_poll_before_settling() {
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

    let streaming_session = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(
        streaming_session
            .active_prompt_for_agent(agent.id())
            .is_some(),
        "a completion signal cannot settle while provider output is still arriving"
    );

    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch::default(),
        )
        .await
        .expect("immediate quiet follow-up batch should be accepted");
    let still_draining_session = runtime
        .owned
        .session_snapshot(session.id())
        .expect("draining session snapshot should exist");
    assert!(still_draining_session
        .active_prompt_for_agent(agent.id())
        .is_some());

    tokio::time::sleep(std::time::Duration::from_millis(60)).await;
    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch::default(),
        )
        .await
        .expect("quiet follow-up after the drain interval should settle completion");
    let settled_session = runtime
        .owned
        .session_snapshot(session.id())
        .expect("settled session snapshot should exist");
    assert!(settled_session
        .active_prompt_for_agent(agent.id())
        .is_none());
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
    sync_external_active_prompt_and_queue_chariox_prompt(
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
    assert_eq!(
        records[0].source_attachment_id.as_deref(),
        Some("external:claude")
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
    assert_eq!(
        output_entry.source_attachment_id.as_deref(),
        Some("external:claude")
    );
}

fn spawn_shared_worktree_agent(app: &mut DaemonApp, session_id: &str, alias: &str) -> String {
    crate::app::KernelSessionService::new(app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session_id, "dev-stub")
                .with_alias(alias)
                .with_model("test-model")
                .with_worktree("worktree-blocked-retry-fifo"),
        )
        .expect("shared-worktree agent should spawn")
        .id()
        .to_string()
}

fn invoke_single_node_workflow(
    app: &mut DaemonApp,
    session_id: &str,
    alias: &str,
    agent_id: &str,
) -> (crate::session::WorkflowRun, String) {
    let workflow = app
        .sessions_mut()
        .create_workflow(session_id, Some(alias.to_string()))
        .expect("workflow should be created");
    app.sessions_mut()
        .set_workflow_flush_agent_context_before_run(session_id, workflow.id(), false)
        .expect("workflow flush context should update");
    let node = app
        .sessions_mut()
        .add_workflow_node(session_id, workflow.id(), agent_id)
        .expect("workflow node should be added");
    app.sessions_mut()
        .create_workflow_endpoint(
            session_id,
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");
    let (run, _, _) = app
        .invoke_workflow_endpoint_and_schedule(
            session_id,
            workflow.id(),
            "entry",
            Some("run".to_string()),
        )
        .expect("workflow run should be created and scheduled");
    let node_run_id = run
        .node_runs()
        .first()
        .expect("workflow run should contain its entry node")
        .id()
        .to_string();
    (run, node_run_id)
}

async fn settle_and_promote_next_queued_prompt(
    runtime: &KernelRuntimeState,
    session_id: &str,
    agent_id: &str,
    _provider_run_id: &str,
) -> crate::session::PromptCompletion {
    let expected_active = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(
            &runtime
                .owned
                .session_store
                .get_session(session_id)
                .expect("session should exist"),
            agent_id,
        )
        .expect("active prompt should exist before promotion");
    let next = runtime
        .owned
        .prompt_state_owner
        .peek_next_queued_prompt(
            &runtime
                .owned
                .session_store
                .get_session(session_id)
                .expect("session should exist"),
            agent_id,
        )
        .expect("next queued prompt should exist");
    let completion = runtime
        .owned
        .complete_local_prompt_with_queued_advance_if_matches(
            session_id,
            agent_id,
            None,
            &next,
            Some(expected_active.id()),
        )
        .expect("claim-aware completion should succeed")
        .expect("completion should match the active prompt");
    completion.completion
}

#[tokio::test]
async fn failed_workflow_dispatch_retries_a_node_blocked_on_its_released_claim() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-failed-dispatch-retry",
            "worktree-blocked-retry-fifo",
        ))
        .expect("session should be created");
    let holder = spawn_shared_worktree_agent(&mut app, session.id(), "failed-holder");
    let worker = spawn_shared_worktree_agent(&mut app, session.id(), "blocked-worker");

    let (holder_run, holder_node) =
        invoke_single_node_workflow(&mut app, session.id(), "wf-failed-holder", &holder);
    let holder_provider_run = app
        .providers()
        .get_run_for_agent(session.id(), &holder)
        .expect("holder provider run should exist");
    let holder_prompt = app
        .prompt_owner_active_prompt_for_agent(session.id(), &holder)
        .expect("holder prompt state should load")
        .expect("holder workflow prompt should be active");
    let (blocked_run, blocked_node) =
        invoke_single_node_workflow(&mut app, session.id(), "wf-blocked-worker", &worker);
    assert_eq!(
        app.sessions()
            .resolve_workflow_run_ref(session.id(), blocked_run.id())
            .expect("blocked run should resolve")
            .node_runs()
            .first()
            .expect("blocked run should have an entry node")
            .status(),
        crate::session::WorkflowNodeRunStatus::BlockedOnWorkspaceClaim,
    );

    let dispatch = crate::app::KernelPromptDispatch {
        session_id: session.id().to_string(),
        provider_run_id: holder_provider_run.id().to_string(),
        agent_id: holder.clone(),
        prompt_id: holder_prompt.id().to_string(),
        target_active_prompt_id: None,
        source_attachment_id: holder_prompt.source_attachment_id().to_string(),
        prompt: holder_prompt.prompt().to_string(),
        hidden_system_context: holder_prompt.hidden_system_context().to_string(),
        attachments: Vec::new(),
        prompt_origin: crate::session::PromptOrigin::Chariox,
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        steering: false,
    };
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;

    let result = runtime
        .fail_prompt_dispatch(
            dispatch,
            crate::error::DaemonError::LocalTransport {
                operation: "test failed workflow dispatch",
                message: "provider did not acknowledge".to_string(),
            },
        )
        .await;
    assert!(result.is_err(), "dispatch failure should remain observable");

    let projected = runtime
        .owned
        .session_store
        .get_session(session.id())
        .expect("session should still exist");
    let retried = projected
        .workflow_run(blocked_run.id())
        .expect("blocked run should remain in the hot session");
    assert_eq!(
        retried
            .node_runs()
            .iter()
            .find(|node| node.id() == blocked_node)
            .expect("blocked node should resolve")
            .status(),
        crate::session::WorkflowNodeRunStatus::Running,
        "asynchronous failure settlement must retry the node after releasing the claim",
    );
    let holder_claim_id =
        runtime
            .owned
            .workflow_dispatch_claim_id(session.id(), holder_run.id(), &holder_node);
    assert!(
        !runtime
            .owned
            .prompt_workspace_claims
            .contains(&holder_claim_id),
        "failed holder claim must be removed",
    );
}

#[tokio::test]
async fn blocked_claim_retry_queued_behind_work_advances_in_fifo_order() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-blocked-retry-fifo",
            "worktree-blocked-retry-fifo",
        ))
        .expect("session should be created");
    let holder = spawn_shared_worktree_agent(&mut app, session.id(), "holder");
    let worker = spawn_shared_worktree_agent(&mut app, session.id(), "worker");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-blocked-retry-fifo",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let _holder_provider_run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "sonnet",
            )
            .with_agent_id(&holder),
        )
        .expect("holder provider run should launch");
    let worker_provider_run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "sonnet",
            )
            .with_agent_id(&worker),
        )
        .expect("worker provider run should launch");

    let (holder_run, holder_node) =
        invoke_single_node_workflow(&mut app, session.id(), "wf-holder", &holder);
    let holder_session = app
        .sessions()
        .get_session(session.id())
        .expect("session should resolve");
    assert!(
        holder_session.active_prompt_for_agent(&holder).is_some(),
        "holder workflow prompt should start while the shared worktree is free"
    );

    // The worker is idle but the holder owns the shared-worktree write claim, so its
    // entry node blocks instead of queueing a prompt.
    let (blocked_run, blocked_node) =
        invoke_single_node_workflow(&mut app, session.id(), "wf-blocked", &worker);
    let blocked_before = app
        .sessions()
        .get_session(session.id())
        .expect("session should resolve");
    assert!(matches!(
        blocked_before
            .workflow_run(blocked_run.id())
            .expect("blocked run should exist")
            .node_runs()
            .iter()
            .find(|node| node.id() == blocked_node)
            .map(|node| node.status()),
        Some(crate::session::WorkflowNodeRunStatus::BlockedOnWorkspaceClaim),
    ));

    // Model holder settlement without immediately running the global retry pass. This
    // leaves the blocked node eligible while we first place older work in the worker's
    // own FIFO queue.
    assert!(app.release_workflow_node_workspace_claim(session.id(), holder_run.id(), &holder_node,));

    // Give the worker an active turn plus a queued workflow invocation so the later
    // retry lands at the tail of a non-empty per-agent queue.
    let crate::session::PromptSubmissionOutcome::Started {
        prompt: _first_user,
    } = app
        .submit_prompt(
            session.id(),
            attachment.id(),
            Some(&worker),
            "first user turn\n",
            Vec::new(),
        )
        .expect("worker user turn should start")
    else {
        panic!("worker user turn should start immediately");
    };
    let (_queued_run, _) =
        invoke_single_node_workflow(&mut app, session.id(), "wf-queued", &worker);
    assert_eq!(
        app.prompt_owner_queued_prompt_count_for_agent(session.id(), &worker)
            .expect("worker queue should resolve"),
        1
    );

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;

    // Retry the blocked claim while the worker still has queued work: the retried
    // prompt must queue behind `wf-queued` without retaining its workspace claim.
    let retries = runtime.owned.workflow_retry_blocked_claims();
    assert!(
        retries.local.is_empty(),
        "retry admission into a busy queue must not produce a concrete dispatch"
    );
    let blocked_claim_id =
        runtime
            .owned
            .workflow_dispatch_claim_id(session.id(), blocked_run.id(), &blocked_node);
    assert!(
        !runtime
            .owned
            .prompt_workspace_claims
            .contains(&blocked_claim_id),
        "a queued retry admission must release its workspace claim"
    );
    let retried_session = runtime
        .owned
        .session_store
        .get_session(session.id())
        .expect("session should exist");
    let (_, worker_queue) = runtime
        .owned
        .prompt_state_owner
        .state_parts(&retried_session, &worker);
    assert_eq!(worker_queue.len(), 2);
    assert_eq!(worker_queue[0].workflow_run_id(), Some(_queued_run.id()));
    assert_eq!(worker_queue[1].workflow_run_id(), Some(blocked_run.id()));

    // FIFO resume: the head (`wf-queued`) promotes first now that no stale claim holds
    // the shared worktree, then the retried prompt follows.
    let first_completion = settle_and_promote_next_queued_prompt(
        &runtime,
        session.id(),
        &worker,
        worker_provider_run.id(),
    )
    .await;
    assert_eq!(
        first_completion
            .started_next
            .as_ref()
            .and_then(|p| p.workflow_run_id()),
        Some(_queued_run.id()),
        "the earlier queued workflow must resume before the retried one"
    );

    let second_completion = settle_and_promote_next_queued_prompt(
        &runtime,
        session.id(),
        &worker,
        worker_provider_run.id(),
    )
    .await;
    assert_eq!(
        second_completion
            .started_next
            .as_ref()
            .and_then(|p| p.workflow_run_id()),
        Some(blocked_run.id()),
        "the retried workflow must resume after the earlier queued workflow"
    );

    let final_session = runtime
        .owned
        .session_store
        .get_session(session.id())
        .expect("session should exist");
    let final_active = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&final_session, &worker)
        .expect("retried workflow prompt should be active");
    assert_eq!(final_active.workflow_run_id(), Some(blocked_run.id()));
    let (_, final_queue) = runtime
        .owned
        .prompt_state_owner
        .state_parts(&final_session, &worker);
    assert!(final_queue.is_empty());
}
