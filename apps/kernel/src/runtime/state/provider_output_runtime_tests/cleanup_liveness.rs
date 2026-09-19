use super::*;

#[tokio::test]
async fn cancellation_acknowledgement_does_not_fail_an_already_stopped_workflow() {
    assert_stopped_workflow_cancellation_acknowledgement(false).await;
}

#[tokio::test]
async fn cancellation_acknowledgement_preserves_existing_workflow_failures() {
    assert_stopped_workflow_cancellation_acknowledgement(true).await;
}

async fn assert_stopped_workflow_cancellation_acknowledgement(existing_failure: bool) {
    let mut app = DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "cancel-ack",
            "cancel-ack",
        ))
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "cancel-ack-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), None)
        .unwrap();
    let node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), agent.id())
        .unwrap();
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(session.id(), workflow.id(), node.id(), None)
        .unwrap();
    let run = app
        .sessions_mut()
        .invoke_workflow_endpoint(session.id(), workflow.id(), endpoint.id(), None)
        .unwrap();
    let prompt = crate::session::PromptQueueItem::new(
        "cancel-ack-prompt",
        attachment.id(),
        agent.id(),
        "cancel",
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(run.id(), run.node_runs()[0].id());
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt.clone(), false)
        .unwrap();
    app.sessions_mut()
        .cancel_workflow_run(session.id(), run.id())
        .unwrap();
    if existing_failure {
        app.sessions_mut()
            .record_workflow_failure_event(
                session.id(),
                run.id(),
                crate::session::WorkflowFailureEvent::new(
                    crate::session::WorkflowFailureKind::ProviderFailure,
                    run.node_runs()[0].id(),
                    Vec::new(),
                    "provider failed before stop",
                ),
            )
            .unwrap();
    }
    let expected_failures = app
        .sessions()
        .resolve_workflow_run_ref(session.id(), run.id())
        .unwrap()
        .failure_events()
        .to_vec();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    for _ in 0..2 {
        runtime
            .owned
            .workflow_cancel_prompt(session.id(), &prompt)
            .unwrap();
        let mut app = app.lock().await;
        crate::app::workflow_runtime::cancel_workflow_prompt_from_runtime(
            &mut app,
            session.id(),
            &prompt,
        )
        .unwrap();
        let stopped = app
            .sessions()
            .resolve_workflow_run_ref(session.id(), run.id())
            .unwrap();
        assert_eq!(stopped.status(), crate::session::WorkflowRunStatus::Stopped);
        assert_eq!(
            stopped.failure_events(),
            expected_failures,
            "acknowledging an intentional stop must preserve failure events unchanged"
        );
    }
}

#[tokio::test]
async fn unexpected_owned_provider_exit_marks_active_agent_error() {
    assert_owned_provider_exit_state(false).await;
}

#[tokio::test]
async fn cancelled_owned_provider_exit_does_not_mark_agent_error() {
    assert_owned_provider_exit_state(true).await;
}

async fn assert_owned_provider_exit_state(cancelling: bool) {
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::DaemonConfig::for_tests())
            .expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-unexpected-exit",
            "worktree-unexpected-exit",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-unexpected-exit",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "codex",
                "default",
                "gpt-5",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider should launch");
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "do work\n",
        Vec::new(),
    )
    .expect("prompt should start");
    if cancelling {
        app.prompt_owner_begin_cancelling_active_prompt(session.id(), agent.id())
            .expect("cancellation should be recorded before provider exit");
    }
    let ended = app
        .providers_mut()
        .mark_run_ended_provider_only(session.id(), run.id())
        .expect("provider run should end")
        .into_run();
    app.update_provider_run_projection(ended);

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let outcome = runtime
        .settle_unexpected_provider_run_exit(
            session.id(),
            run.id(),
            agent.id(),
            crate::provider::ProviderRunTermination::process_exit(17, 42),
        )
        .await
        .expect("unexpected provider exit should settle");

    assert!(outcome.had_active_prompt);
    assert_eq!(outcome.cancelled_prompt, cancelling);
    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(session_state.active_prompt_for_agent(agent.id()).is_none());
    let agent_state = runtime
        .owned
        .agent_store
        .get_agent(agent.id())
        .expect("agent should remain available")
        .state();
    let completed = runtime
        .owned
        .completed_git_turn_snapshots
        .latest_projection_for_agent(session.id(), agent.id())
        .expect("settled turn should remain projected");
    if cancelling {
        assert_ne!(
            agent_state,
            crate::agent::AgentState::Error,
            "a deliberate cancellation must not become an unexpected provider failure"
        );
        assert_eq!(
            completed.provider_termination, None,
            "a deliberate cancellation must not record a provider failure termination"
        );
    } else {
        assert_eq!(agent_state, crate::agent::AgentState::Error);
        assert_eq!(
            completed.settlement_status,
            crate::git_observer::CompletedTurnSettlementStatus::Failed,
        );
        assert_eq!(
            completed.provider_termination,
            Some(crate::provider::ProviderRunTermination::process_exit(
                17, 42
            )),
        );
    }
}

#[tokio::test]
async fn unexpected_owned_provider_exit_promotes_queued_prompt_once_on_replacement_run() {
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::DaemonConfig::for_tests())
            .expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-unexpected-exit-queue",
            "worktree-unexpected-exit-queue",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-unexpected-exit-queue",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "codex",
                "default",
                "gpt-5",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider should launch");
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "first prompt\n",
        Vec::new(),
    )
    .expect("first prompt should start");
    match app
        .submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "queued prompt\n",
            Vec::new(),
        )
        .expect("second prompt should queue")
    {
        crate::session::PromptSubmissionOutcome::Queued { .. } => {}
        other => panic!("second prompt should queue, got {other:?}"),
    };
    let ended = app
        .providers_mut()
        .mark_run_ended_provider_only(session.id(), run.id())
        .expect("provider run should end")
        .into_run();
    app.update_provider_run_projection(ended);

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let outcome = runtime
        .settle_unexpected_provider_run_exit(
            session.id(),
            run.id(),
            agent.id(),
            crate::provider::ProviderRunTermination::process_exit(1, 43),
        )
        .await
        .expect("unexpected provider exit should settle and replace");

    assert!(outcome.had_active_prompt);
    assert!(outcome.started_next_prompt);
    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    let active_prompt = session_state
        .active_prompt_for_agent(agent.id())
        .expect("queued prompt should be active on the replacement run");
    assert_eq!(active_prompt.prompt(), "queued prompt\n");
    assert_eq!(
        runtime
            .owned
            .agent_store
            .get_agent(agent.id())
            .expect("replacement agent should remain available")
            .state(),
        crate::agent::AgentState::Working,
    );
    let active_prompt_id = active_prompt.id().to_string();
    assert!(session_state
        .queued_prompts_for_agent(agent.id())
        .is_none_or(std::collections::VecDeque::is_empty));

    let repeated = runtime
        .settle_unexpected_provider_run_exit(
            session.id(),
            run.id(),
            agent.id(),
            crate::provider::ProviderRunTermination::process_exit(1, 43),
        )
        .await
        .expect("repeated exit reconciliation should be idempotent");
    assert!(!repeated.had_active_prompt);
    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should still exist");
    assert_eq!(
        session_state
            .active_prompt_for_agent(agent.id())
            .map(crate::session::PromptQueueItem::id),
        Some(active_prompt_id.as_str()),
        "the queued prompt must be promoted exactly once",
    );
}

#[tokio::test]
async fn unexpected_owned_provider_exit_without_active_prompt_preserves_agent_state() {
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::DaemonConfig::for_tests())
            .expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-idle-exit",
            "worktree-idle-exit",
        ))
        .expect("session should be created");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "codex",
                "default",
                "gpt-5",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider should launch");
    let state_before = agent.state();
    crate::app::ProviderLaunchProcessRuntime::new(&mut app)
        .remove_run(run.id())
        .expect("idle provider process should stop");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let ended = runtime
        .reconcile_provider_run_exit(session.id(), run.id())
        .await
        .expect("idle provider exit should reconcile");

    assert!(ended);
    assert_eq!(
        runtime
            .owned
            .agent_store
            .get_agent(agent.id())
            .expect("agent should remain available")
            .state(),
        state_before,
    );
    let history = runtime
        .owned
        .operational_history_store
        .load_session_events(session.id(), Some(agent.id()))
        .expect("idle agent history should load");
    assert!(!history.iter().any(|event| {
        event.kind == crate::history::HistoryEventKind::Notice
            && event
                .content
                .as_deref()
                .is_some_and(|content| content.contains("ended unexpectedly"))
    }));
}

#[tokio::test]
async fn owned_end_session_clears_stale_prompt_runtime_state_for_already_ended_session() {
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::DaemonConfig::for_tests())
            .expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let run = app
        .launch_provider(crate::provider::LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "codex",
            "default",
            "gpt-5",
        ))
        .expect("provider should launch");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .end_session(session.id())
        .await
        .expect("session should end once");
    {
        let app = app.lock().await;
        app.prompt_activity_store().write().insert(
            run.id().to_string(),
            crate::app::ActivePromptState {
                last_output_at: Some(Instant::now()),
                saw_response_content: true,
                completion_recorded: true,
                settlement_requested: true,
                active_tool_ids: std::collections::BTreeSet::new(),
            },
        );
        app.active_turn_store().start(
            crate::app::ActiveTurnState::new(
                session.id().to_string(),
                agent.id().to_string(),
                "prompt-stale".to_string(),
                run.id().to_string(),
            )
            .with_phase(crate::app::ActiveTurnPhase::Settling),
        );
    }

    runtime
        .end_session(session.id())
        .await
        .expect("already ended session should clean stale runtime state");

    let app = app.lock().await;
    assert!(
        !app.prompt_activity_store().read().contains_key(run.id()),
        "prompt activity should not survive already-ended session cleanup"
    );
    assert!(
        !app.active_turn_store().snapshot().contains_key(run.id()),
        "active turn should not survive already-ended session cleanup"
    );
}

#[tokio::test]
async fn owned_liveness_reconciliation_settles_already_ended_active_prompt() {
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::DaemonConfig::for_tests())
            .expect("daemon should boot");
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
                "codex",
                "default",
                "gpt-5",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider should launch");
    app.update_provider_run_projection(run.clone());
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "do work\n",
        Vec::new(),
    )
    .expect("prompt should start");
    match app
        .submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "queued work\n",
            Vec::new(),
        )
        .expect("queued prompt should submit")
    {
        crate::session::PromptSubmissionOutcome::Queued { .. } => {}
        other => panic!("second prompt should queue, got {other:?}"),
    }
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());
    let ended = app
        .providers_mut()
        .mark_run_ended_provider_only(session.id(), run.id())
        .expect("provider run should be marked ended")
        .into_run();
    app.update_provider_run_projection(ended);

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let already_ended = runtime
        .reconcile_provider_run_exit(session.id(), run.id())
        .await
        .expect("already-ended liveness reconciliation should succeed");

    assert!(already_ended);
    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    let active_prompt = session_state
        .active_prompt_for_agent(agent.id())
        .expect("already-ended reconciliation should advance one queued prompt");
    assert_eq!(active_prompt.prompt(), "queued work\n");
    assert!(session_state
        .queued_prompts_for_agent(agent.id())
        .is_none_or(std::collections::VecDeque::is_empty));
    let completed = runtime
        .owned
        .completed_git_turn_snapshots
        .latest_projection_for_agent(session.id(), agent.id())
        .expect("dead-run settlement should remain projected");
    let termination = completed
        .provider_termination
        .expect("dead-run settlement should expose provider termination");
    assert!(termination.reason.contains("already ended"));
    assert!(
        !runtime.owned.provider_output_deadlines.contains(run.id()),
        "the prior provider output timer must be cleared"
    );
    let app = app.lock().await;
    assert!(
        !app.prompt_activity_store().read().contains_key(run.id()),
        "already-ended provider reconciliation should clear prompt activity"
    );
    assert!(
        !app.active_turn_store().snapshot().contains_key(run.id()),
        "already-ended provider reconciliation should clear active turn state"
    );
}

#[tokio::test]
async fn stale_provider_exit_does_not_settle_prompt_on_replacement_run() {
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::DaemonConfig::for_tests())
            .expect("daemon should boot");
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
    let stale_run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "codex",
                "default",
                "gpt-5",
            )
            .with_agent_id(agent.id()),
        )
        .expect("initial provider should launch");
    let ended = app
        .providers_mut()
        .mark_run_ended_provider_only(session.id(), stale_run.id())
        .expect("initial provider should end")
        .into_run();
    app.update_provider_run_projection(ended);
    let replacement_run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "codex",
                "default",
                "gpt-5",
            )
            .with_agent_id(agent.id()),
        )
        .expect("replacement provider should launch");
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "continue on the replacement\n",
        Vec::new(),
    )
    .expect("replacement prompt should start");
    let prompt_id = app
        .sessions()
        .get_session(session.id())
        .expect("session should exist")
        .active_prompt_for_agent(agent.id())
        .expect("replacement prompt should remain active")
        .id()
        .to_string();
    app.active_turn_store()
        .start(crate::app::ActiveTurnState::new(
            session.id().to_string(),
            agent.id().to_string(),
            prompt_id,
            replacement_run.id().to_string(),
        ));

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let already_ended = runtime
        .reconcile_provider_run_exit(session.id(), stale_run.id())
        .await
        .expect("stale provider reconciliation should succeed");

    assert!(already_ended);
    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(
        session_state.active_prompt_for_agent(agent.id()).is_some(),
        "stale provider reconciliation must not settle the replacement prompt"
    );
    assert!(
        runtime
            .owned
            .active_turns
            .snapshot()
            .contains_key(replacement_run.id()),
        "replacement active turn must remain tracked"
    );
}

#[tokio::test]
async fn stale_provider_exit_preserves_starting_cross_agent_workflow_handoff() {
    let mut app =
        DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, focused_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-cross-agent-handoff",
            "worktree-cross-agent-handoff",
        ))
        .expect("session should be created");
    let stale_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub").with_alias("stale"),
        )
        .expect("stale agent should spawn");
    let downstream_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("downstream"),
        )
        .expect("downstream agent should spawn");
    crate::app::KernelSessionService::new(&mut app)
        .focus_agent(session.id(), focused_agent.id())
        .expect("first agent should remain focused");

    let focused_run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "default",
            )
            .with_agent_id(focused_agent.id()),
        )
        .expect("focused provider should launch");
    let parked_focused_run = app
        .providers_mut()
        .park_run_provider_only(session.id(), focused_run.id())
        .expect("focused provider should park")
        .into_run();
    app.update_provider_run_projection(parked_focused_run);

    let stale_run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "default",
            )
            .with_agent_id(stale_agent.id()),
        )
        .expect("stale provider should launch");
    let ended_stale_run = app
        .providers_mut()
        .mark_run_ended_provider_only(session.id(), stale_run.id())
        .expect("stale provider should end")
        .into_run();
    app.update_provider_run_projection(ended_stale_run);
    app.sessions_mut()
        .set_active_provider_run(session.id(), None)
        .expect("ended stale provider should no longer be active");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let downstream_run = runtime
        .owned
        .start_provider_launch(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "default",
            )
            .with_agent_id(downstream_agent.id()),
        )
        .expect("downstream provider launch should start")
        .run;
    runtime
        .owned
        .provider_run_projection
        .update(downstream_run.clone());

    let already_ended = runtime
        .reconcile_provider_run_exit(session.id(), stale_run.id())
        .await
        .expect("stale provider reconciliation should succeed");

    assert!(already_ended);
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(downstream_run.id())
            .expect("downstream run should remain available")
            .state(),
        crate::provider::ProviderRunState::Starting,
        "stale settlement must not terminate a downstream provider launch",
    );
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(focused_run.id())
            .expect("focused run should remain available")
            .state(),
        crate::provider::ProviderRunState::Parked,
        "stale settlement must not resume the focused idle provider",
    );
    assert_eq!(
        runtime
            .owned
            .session_store
            .get_session(session.id())
            .expect("session should remain available")
            .active_provider_run_id(),
        Some(downstream_run.id()),
        "the downstream provider launch must remain active",
    );
}

#[tokio::test]
async fn owned_destroy_agent_clears_stale_prompt_runtime_state_for_ended_provider_runs() {
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::DaemonConfig::for_tests())
            .expect("daemon should boot");
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
                "codex",
                "default",
                "gpt-5",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider should launch");
    let ended = app
        .providers_mut()
        .terminate_run_provider_only(session.id(), run.id())
        .expect("provider run should end")
        .into_run();
    app.update_provider_run_projection(ended);
    app.prompt_activity_store().write().insert(
        run.id().to_string(),
        crate::app::ActivePromptState {
            last_output_at: Some(Instant::now()),
            saw_response_content: true,
            completion_recorded: true,
            settlement_requested: true,
            active_tool_ids: std::collections::BTreeSet::new(),
        },
    );
    app.active_turn_store().start(
        crate::app::ActiveTurnState::new(
            session.id().to_string(),
            agent.id().to_string(),
            "prompt-stale".to_string(),
            run.id().to_string(),
        )
        .with_phase(crate::app::ActiveTurnPhase::Settling),
    );

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .destroy_agent(agent.id(), crate::session::DEFAULT_LOCAL_USER_ID)
        .await
        .expect("agent should be destroyed");

    let app = app.lock().await;
    assert!(
        !app.prompt_activity_store().read().contains_key(run.id()),
        "destroying an agent should clear prompt activity for ended provider runs"
    );
    assert!(
        !app.active_turn_store().snapshot().contains_key(run.id()),
        "destroying an agent should clear active turns for ended provider runs"
    );
}

#[derive(Debug, Clone, Copy)]
struct MixedLivenessFakeClock {
    now_ms: u64,
    liveness_timeout_ms: u64,
}

impl MixedLivenessFakeClock {
    fn from_provider_runs(
        first: &crate::provider::RuntimeProviderRun,
        second: &crate::provider::RuntimeProviderRun,
    ) -> Self {
        Self {
            now_ms: first
                .last_activity_at_ms()
                .max(second.last_activity_at_ms()),
            // The production provider-liveness API consumes an explicit process observation.
            // Keep the elapsed-time decision deterministic here without changing that API.
            liveness_timeout_ms: 1_000,
        }
    }

    fn advance_past_liveness(&mut self) {
        self.now_ms = self
            .now_ms
            .saturating_add(self.liveness_timeout_ms.saturating_add(1));
    }

    fn is_past_liveness(&self, last_activity_at_ms: u64) -> bool {
        self.now_ms.saturating_sub(last_activity_at_ms) > self.liveness_timeout_ms
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MixedPromptActivityFingerprint {
    has_last_output: bool,
    saw_response_content: bool,
    completion_recorded: bool,
    settlement_requested: bool,
    active_tool_ids: std::collections::BTreeSet<String>,
}

fn mixed_prompt_activity_fingerprint(
    runtime: &KernelRuntimeState,
    provider_run_id: &str,
) -> Option<MixedPromptActivityFingerprint> {
    runtime
        .owned
        .prompt_activity
        .read()
        .get(provider_run_id)
        .map(|state| MixedPromptActivityFingerprint {
            has_last_output: state.last_output_at.is_some(),
            saw_response_content: state.saw_response_content,
            completion_recorded: state.completion_recorded,
            settlement_requested: state.settlement_requested,
            active_tool_ids: state.active_tool_ids.clone(),
        })
}

fn mixed_history(
    runtime: &KernelRuntimeState,
    session_id: &str,
    agent_id: &str,
) -> Vec<crate::history::SessionHistoryEntry> {
    runtime
        .owned
        .operational_history_store
        .load_session_history_entries(session_id, Some(agent_id))
        .expect("agent history should load")
}

fn mixed_queue_prompt_ids(
    session: &crate::session::RuntimeSession,
    agent_id: &str,
) -> Vec<String> {
    session
        .queued_prompts_for_agent(agent_id)
        .map(|prompts| prompts.iter().map(|prompt| prompt.id().to_string()).collect())
        .unwrap_or_default()
}

fn mixed_submit_active_and_queued_prompts(
    app: &mut DaemonApp,
    session_id: &str,
    attachment_id: &str,
    agent_id: &str,
    active_text: &str,
    queued_text: &str,
) -> (String, String) {
    app.submit_prompt(
        session_id,
        attachment_id,
        Some(agent_id),
        active_text,
        Vec::new(),
    )
    .expect("active prompt should submit");
    let active_prompt_id = app
        .sessions()
        .get_session(session_id)
        .expect("session should exist")
        .active_prompt_for_agent(agent_id)
        .expect("active prompt should be present")
        .id()
        .to_string();

    match app
        .submit_prompt(
            session_id,
            attachment_id,
            Some(agent_id),
            queued_text,
            Vec::new(),
        )
        .expect("queued prompt should submit")
    {
        crate::session::PromptSubmissionOutcome::Queued { .. } => {}
        other => panic!("second prompt should queue, got {other:?}"),
    }
    let queued_prompt_id = app
        .sessions()
        .get_session(session_id)
        .expect("session should exist")
        .queued_prompts_for_agent(agent_id)
        .and_then(|prompts| prompts.front())
        .expect("queued prompt should be present")
        .id()
        .to_string();
    (active_prompt_id, queued_prompt_id)
}

fn mixed_launch_provider(
    app: &mut DaemonApp,
    session_id: &str,
    agent_id: &str,
    model: &str,
) -> crate::provider::RuntimeProviderRun {
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session_id,
                "dev-stub",
                "dev-stub",
                "default",
                model,
            )
            .with_agent_id(agent_id),
        )
        .expect("dev-stub provider should launch");
    app.update_provider_run_projection(run.clone());
    run
}

fn assert_mixed_old_provider_runtime_cleared(
    runtime: &KernelRuntimeState,
    provider_run_id: &str,
) {
    assert!(
        !runtime
            .owned
            .prompt_activity
            .read()
            .contains_key(provider_run_id),
        "stale provider prompt activity should be cleared"
    );
    assert!(
        !runtime
            .active_turn_snapshot()
            .contains_key(provider_run_id),
        "stale provider active turn should be cleared"
    );
    assert!(
        !runtime.owned.provider_output_deadlines.contains(provider_run_id),
        "stale provider output timer should be stopped"
    );
}

fn mixed_activity_event_revision(
    event: &crate::transport::kernel_protocol::KernelEvent,
) -> u64 {
    match event {
        crate::transport::kernel_protocol::KernelEvent::AgentActivityChanged {
            agent_activity_revision,
            ..
        } => *agent_activity_revision,
        other => panic!("expected agent activity event, got {other:?}"),
    }
}

#[test]
fn kernel_mixed_01_reconciles_two_stale_providers_without_touching_healthy_agent() {
    run_kernel_mixed_01_with_large_stack(
        "kernel-mixed-01",
        kernel_mixed_01_reconciles_two_stale_providers_without_touching_healthy_agent_inner,
    );
}

fn run_kernel_mixed_01_with_large_stack<F, Fut>(name: &'static str, test: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(crate::runtime_transport::KERNEL_RUNTIME_THREAD_STACK_SIZE)
                .enable_all()
                .build()
                .expect("mixed liveness test runtime should build")
                .block_on(test());
        })
        .expect("mixed liveness test thread should spawn")
        .join()
        .unwrap_or_else(|error| std::panic::resume_unwind(error));
}

async fn kernel_mixed_01_reconciles_two_stale_providers_without_touching_healthy_agent_inner() {
    let mut app = crate::test_support::bootstrap_authenticated_app(
        crate::DaemonConfig::for_tests(),
    )
    .expect("daemon should boot");
    let (session, healthy_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-kernel-mixed-01",
            "worktree-kernel-mixed-01",
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let healthy_agent_id = healthy_agent.id().to_string();
    let stale_b = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("stale-b"),
        )
        .expect("stale B agent should spawn");
    let stale_b_id = stale_b.id().to_string();
    let stale_c = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("stale-c"),
        )
        .expect("stale C agent should spawn");
    let stale_c_id = stale_c.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-kernel-mixed-01",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let attachment_id = attachment.id().to_string();

    let healthy_run = mixed_launch_provider(&mut app, &session_id, &healthy_agent_id, "healthy");
    let stale_b_run = mixed_launch_provider(&mut app, &session_id, &stale_b_id, "stale-b");
    let stale_c_run = mixed_launch_provider(&mut app, &session_id, &stale_c_id, "stale-c");
    let (healthy_active_prompt_id, healthy_queued_prompt_id) =
        mixed_submit_active_and_queued_prompts(
            &mut app,
            &session_id,
            &attachment_id,
            &healthy_agent_id,
            "healthy active prompt\n",
            "healthy queued prompt\n",
        );
    let (stale_c_active_prompt_id, stale_c_queued_prompt_id) =
        mixed_submit_active_and_queued_prompts(
            &mut app,
            &session_id,
            &attachment_id,
            &stale_c_id,
            "stale C active prompt\n",
            "stale C queued prompt\n",
        );

    let app = Arc::new(Mutex::new(app));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        Arc::clone(&app),
        4,
    );
    let healthy_token = healthy_run
        .runtime_mcp_auth_token()
        .expect("healthy run should expose runtime MCP auth")
        .to_string();
    let message_arguments = serde_json::json!({
        "agent": "@stale-b",
        "message": "kernel mixed liveness message",
        "idempotency_key": "kernel-mixed-01-message",
    });
    let first_message = router
        .runtime_state()
        .dispatch_authenticated_runtime_tool_call(
            &healthy_token,
            crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL,
            message_arguments.clone(),
        )
        .await
        .expect("agent message should dispatch through the kernel");
    assert!(first_message.ok, "{:?}", first_message.payload);
    assert_eq!(first_message.payload["status"], "started");
    assert_eq!(
        first_message.payload["target_agent_id"],
        serde_json::json!(stale_b_id)
    );
    let message_prompt_id = first_message.payload["prompt_id"]
        .as_str()
        .expect("message should return a prompt id")
        .to_string();
    let runtime = router.runtime_state();
    let b_history_after_first = mixed_history(&runtime, &session_id, &stale_b_id);
    let retried_message = router
        .runtime_state()
        .dispatch_authenticated_runtime_tool_call(
            &healthy_token,
            crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL,
            message_arguments,
        )
        .await
        .expect("duplicate agent message should dispatch through the kernel");
    assert!(retried_message.ok, "{:?}", retried_message.payload);
    assert_eq!(
        retried_message.payload["prompt_id"],
        serde_json::json!(message_prompt_id)
    );
    assert_eq!(
        b_history_after_first,
        mixed_history(&runtime, &session_id, &stale_b_id),
        "the idempotency retry must not append another durable message"
    );

    let stale_b_active_prompt_id = runtime
        .owned
        .session_snapshot(&session_id)
        .expect("session snapshot should exist")
        .active_prompt_for_agent(&stale_b_id)
        .expect("agent message should be active for stale B")
        .id()
        .to_string();
    assert_eq!(stale_b_active_prompt_id, message_prompt_id);
    {
        let mut app = app.lock().await;
        match app
            .submit_prompt(
                &session_id,
                &attachment_id,
                Some(&stale_b_id),
                "stale B queued prompt\n",
                Vec::new(),
            )
            .expect("stale B queued prompt should submit")
        {
            crate::session::PromptSubmissionOutcome::Queued { .. } => {}
            other => panic!("stale B prompt should queue, got {other:?}"),
        }
    }
    let stale_b_queued_prompt_id = runtime
        .owned
        .session_snapshot(&session_id)
        .expect("session snapshot should exist")
        .queued_prompts_for_agent(&stale_b_id)
        .and_then(|prompts| prompts.front())
        .expect("stale B should have a queued prompt")
        .id()
        .to_string();

    runtime.owned.note_prompt_started(healthy_run.id());
    runtime.owned.note_prompt_started(stale_b_run.id());
    runtime.owned.note_prompt_started(stale_c_run.id());
    runtime.owned.note_prompt_response_content(healthy_run.id());
    let progress_before_clock = runtime.owned.fan_out_terminal_output(
        &session_id,
        healthy_run.id(),
        crate::terminal::TerminalOutputKind::ProviderOutput,
        Some("healthy-progress-1".to_string()),
        vec![attachment_id.clone()],
        b"healthy progress before stale reconciliation",
    );
    assert_eq!(
        progress_before_clock.bytes,
        b"healthy progress before stale reconciliation"
    );

    let mut fake_clock =
        MixedLivenessFakeClock::from_provider_runs(&stale_b_run, &stale_c_run);
    fake_clock.advance_past_liveness();
    assert!(fake_clock.is_past_liveness(stale_b_run.last_activity_at_ms()));
    assert!(fake_clock.is_past_liveness(stale_c_run.last_activity_at_ms()));
    // A continues to make progress after the clock crosses the stale threshold. This is the
    // healthy observation that keeps its run authoritative while B and C receive stale exits.
    runtime.owned.note_prompt_response_content(healthy_run.id());
    let progress_after_clock = runtime.owned.fan_out_terminal_output(
        &session_id,
        healthy_run.id(),
        crate::terminal::TerminalOutputKind::ProviderOutput,
        Some("healthy-progress-2".to_string()),
        vec![attachment_id.clone()],
        b"healthy progress after fake liveness",
    );
    assert_eq!(
        progress_after_clock.bytes,
        b"healthy progress after fake liveness"
    );

    let baseline_revision = runtime.session_projection_session_change_sequence(&session_id);
    let baseline_projection = runtime
        .session_snapshot_projection(&session_id, baseline_revision)
        .expect("baseline projection should load");
    let baseline_healthy_activity = baseline_projection
        .agent_activity
        .get(&healthy_agent_id)
        .cloned()
        .expect("healthy agent activity should project");
    let baseline_healthy_turn = runtime
        .active_turn_snapshot()
        .get(healthy_run.id())
        .cloned()
        .expect("healthy active turn should remain tracked");
    let baseline_healthy_activity_state = mixed_prompt_activity_fingerprint(&runtime, healthy_run.id())
        .expect("healthy prompt activity should remain tracked");
    let baseline_healthy_history = mixed_history(&runtime, &session_id, &healthy_agent_id);
    assert!(
        baseline_healthy_history
            .iter()
            .any(|entry| entry.text.contains("healthy progress after fake liveness")),
        "progress without final completion must be durable"
    );
    assert!(baseline_healthy_activity.active_turn.is_some());
    assert_eq!(
        baseline_healthy_activity.active_turn.as_ref().map(|turn| turn.prompt_id.as_str()),
        Some(healthy_active_prompt_id.as_str())
    );
    assert_eq!(
        mixed_queue_prompt_ids(&baseline_projection.session, &healthy_agent_id),
        vec![healthy_queued_prompt_id.clone()]
    );
    assert!(
        baseline_healthy_activity.last_completed_turn.is_none(),
        "healthy progress must not be mistaken for final completion"
    );
    let baseline_healthy_run = runtime
        .owned
        .provider_store
        .get_run(healthy_run.id())
        .expect("healthy provider run should remain available");
    let baseline_healthy_run_state = baseline_healthy_run.state();
    let baseline_healthy_last_activity = baseline_healthy_run.last_activity_at_ms();
    let baseline_healthy_timer = runtime.owned.provider_output_deadlines.contains(healthy_run.id());
    assert!(baseline_healthy_timer);

    let baseline_stale_b_history = mixed_history(&runtime, &session_id, &stale_b_id);
    let baseline_stale_c_history = mixed_history(&runtime, &session_id, &stale_c_id);
    assert!(!baseline_stale_b_history.is_empty(), "stale B history must be durable");
    assert!(!baseline_stale_c_history.is_empty(), "stale C history must be durable");
    assert_eq!(
        mixed_queue_prompt_ids(&baseline_projection.session, &stale_b_id),
        vec![stale_b_queued_prompt_id.clone()]
    );
    assert_eq!(
        mixed_queue_prompt_ids(&baseline_projection.session, &stale_c_id),
        vec![stale_c_queued_prompt_id.clone()]
    );

    {
        let mut app = app.lock().await;
        let stale_b_ended = app
            .providers_mut()
            .mark_run_ended_provider_only(&session_id, stale_b_run.id())
            .expect("stale B provider record should end")
            .into_run();
        app.update_provider_run_projection(stale_b_ended);
        let stale_c_ended = app
            .providers_mut()
            .mark_run_ended_provider_only(&session_id, stale_c_run.id())
            .expect("stale C provider record should end")
            .into_run();
        app.update_provider_run_projection(stale_c_ended);
    }

    let stale_b_reconciled = runtime
        .reconcile_provider_run_exit(&session_id, stale_b_run.id())
        .await
        .expect("stale B liveness should reconcile");
    let stale_c_reconciled = runtime
        .reconcile_provider_run_exit(&session_id, stale_c_run.id())
        .await
        .expect("stale C liveness should reconcile");
    assert!(stale_b_reconciled);
    assert!(stale_c_reconciled);

    let first_reconciled_revision = runtime.session_projection_session_change_sequence(&session_id);
    assert!(first_reconciled_revision > baseline_revision);
    let first_reconciled_projection = runtime
        .session_snapshot_projection(&session_id, first_reconciled_revision)
        .expect("reconciled projection should load");
    assert_eq!(
        first_reconciled_projection
            .agent_activity
            .get(&healthy_agent_id),
        Some(&baseline_healthy_activity),
        "B/C cleanup must not change A status, progress, timer, turn, or queue"
    );
    assert_eq!(
        runtime
            .active_turn_snapshot()
            .get(healthy_run.id()),
        Some(&baseline_healthy_turn)
    );
    assert_eq!(
        mixed_prompt_activity_fingerprint(&runtime, healthy_run.id()),
        Some(baseline_healthy_activity_state)
    );
    assert_eq!(
        mixed_history(&runtime, &session_id, &healthy_agent_id),
        baseline_healthy_history
    );
    let healthy_after_reconcile = runtime
        .owned
        .provider_store
        .get_run(healthy_run.id())
        .expect("healthy provider run should remain available after cleanup");
    assert_eq!(healthy_after_reconcile.id(), healthy_run.id());
    assert_eq!(healthy_after_reconcile.state(), baseline_healthy_run_state);
    assert_eq!(
        healthy_after_reconcile.last_activity_at_ms(),
        baseline_healthy_last_activity
    );
    assert_eq!(
        runtime.owned.provider_output_deadlines.contains(healthy_run.id()),
        baseline_healthy_timer
    );
    assert_eq!(
        mixed_queue_prompt_ids(
            &first_reconciled_projection.session,
            &healthy_agent_id
        ),
        vec![healthy_queued_prompt_id.clone()]
    );
    assert_eq!(
        first_reconciled_projection
            .agent_activity
            .get(&healthy_agent_id)
            .and_then(|activity| activity.active_turn.as_ref())
            .map(|turn| turn.prompt_id.as_str()),
        Some(healthy_active_prompt_id.as_str())
    );

    for (agent_id, old_run_id, old_active_prompt_id, queued_prompt_id) in [
        (
            stale_b_id.as_str(),
            stale_b_run.id(),
            stale_b_active_prompt_id.as_str(),
            stale_b_queued_prompt_id.as_str(),
        ),
        (
            stale_c_id.as_str(),
            stale_c_run.id(),
            stale_c_active_prompt_id.as_str(),
            stale_c_queued_prompt_id.as_str(),
        ),
    ] {
        let session_after = runtime
            .owned
            .session_snapshot(&session_id)
            .expect("session snapshot should exist after stale reconciliation");
        assert_eq!(
            session_after
                .active_prompt_for_agent(agent_id)
                .map(crate::session::PromptQueueItem::id),
            Some(queued_prompt_id),
            "one queued prompt must be promoted for each stale provider"
        );
        assert!(
            session_after
                .queued_prompts_for_agent(agent_id)
                .is_none_or(std::collections::VecDeque::is_empty)
        );
        let completed = runtime
            .owned
            .completed_git_turn_snapshots
            .latest_projection_for_agent(&session_id, agent_id)
            .expect("one stale turn termination should be projected");
        assert_eq!(completed.prompt_id, old_active_prompt_id);
        assert_eq!(completed.provider_run_id, old_run_id);
        assert_eq!(
            completed.settlement_status,
            crate::git_observer::CompletedTurnSettlementStatus::Failed
        );
        assert!(
            completed
                .provider_termination
                .as_ref()
                .is_some_and(|termination| termination.reason.contains("already ended")),
            "provider termination must be recorded exactly once for the stale run"
        );
        assert_mixed_old_provider_runtime_cleared(&runtime, old_run_id);
        assert_eq!(
            runtime
                .owned
                .provider_store
                .get_run(old_run_id)
                .expect("stale run should remain durable")
                .state(),
            crate::provider::ProviderRunState::Ended
        );
    }

    let first_stale_b_history = mixed_history(&runtime, &session_id, &stale_b_id);
    let first_stale_c_history = mixed_history(&runtime, &session_id, &stale_c_id);
    let first_stale_b_completion = runtime
        .owned
        .completed_git_turn_snapshots
        .latest_projection_for_agent(&session_id, &stale_b_id)
        .expect("stale B completion should remain projected");
    let first_stale_c_completion = runtime
        .owned
        .completed_git_turn_snapshots
        .latest_projection_for_agent(&session_id, &stale_c_id)
        .expect("stale C completion should remain projected");

    let stale_revision = baseline_revision;
    let delayed_working_projection = crate::runtime::projection::SessionSnapshotProjection {
        metadata: crate::runtime::projection::ProjectionMetadata::new(
            first_reconciled_projection.metadata.projection_version,
            stale_revision,
        ),
        session: first_reconciled_projection.session.clone(),
        provider_run: first_reconciled_projection.provider_run.clone(),
        agent_activity: baseline_projection.agent_activity.clone(),
    };
    let delayed_working_event =
        crate::transport::kernel_protocol::agent_activity_changed_event(
            &delayed_working_projection,
            Some(&first_reconciled_projection),
        )
        .expect("lower-revision WORKING event should be representable");
    let mut delayed_completion_activity = first_reconciled_projection.agent_activity.clone();
    for agent_id in [&stale_b_id, &stale_c_id] {
        let activity = delayed_completion_activity
            .get_mut(agent_id)
            .expect("stale agent activity should project");
        activity.status = crate::runtime::projection::AgentRuntimeStatus::Idle;
        activity.prompt_status = crate::runtime::projection::AgentPromptRuntimeStatus::None;
        activity.busy = false;
        activity.active_prompt_count = 0;
        activity.queued_prompt_count = 0;
        activity.queued_prompt_controls.clear();
        activity.active_turn = None;
    }
    let delayed_completion_projection = crate::runtime::projection::SessionSnapshotProjection {
        metadata: crate::runtime::projection::ProjectionMetadata::new(
            first_reconciled_projection.metadata.projection_version,
            stale_revision,
        ),
        session: first_reconciled_projection.session.clone(),
        provider_run: first_reconciled_projection.provider_run.clone(),
        agent_activity: delayed_completion_activity,
    };
    let delayed_completion_event =
        crate::transport::kernel_protocol::agent_activity_changed_event(
            &delayed_completion_projection,
            Some(&first_reconciled_projection),
        )
        .expect("lower-revision completion event should be representable");
    assert!(stale_revision < first_reconciled_revision);
    assert_eq!(
        mixed_activity_event_revision(&delayed_working_event),
        stale_revision
    );
    assert_eq!(
        mixed_activity_event_revision(&delayed_completion_event),
        stale_revision
    );

    let repeated_b = runtime
        .reconcile_provider_run_exit(&session_id, stale_b_run.id())
        .await
        .expect("repeated stale B exit should be ignored");
    let repeated_c = runtime
        .reconcile_provider_run_exit(&session_id, stale_c_run.id())
        .await
        .expect("repeated stale C exit should be ignored");
    assert!(repeated_b);
    assert!(repeated_c);
    let delayed_b_completion = runtime
        .settle_unexpected_provider_run_exit(
            &session_id,
            stale_b_run.id(),
            &stale_b_id,
            crate::provider::ProviderRunTermination::runtime_failure(
                "delayed lower-revision completion",
                fake_clock.now_ms,
            ),
        )
        .await
        .expect("delayed stale B completion should be ignored");
    let delayed_c_completion = runtime
        .settle_unexpected_provider_run_exit(
            &session_id,
            stale_c_run.id(),
            &stale_c_id,
            crate::provider::ProviderRunTermination::runtime_failure(
                "delayed lower-revision completion",
                fake_clock.now_ms,
            ),
        )
        .await
        .expect("delayed stale C completion should be ignored");
    assert!(!delayed_b_completion.had_active_prompt);
    assert!(!delayed_c_completion.had_active_prompt);

    let after_stale_revision = runtime.session_projection_session_change_sequence(&session_id);
    assert_eq!(after_stale_revision, first_reconciled_revision);
    let after_stale_projection = runtime
        .session_snapshot_projection(&session_id, after_stale_revision)
        .expect("projection should remain authoritative after stale events");
    assert_eq!(
        after_stale_projection.session,
        first_reconciled_projection.session
    );
    assert_eq!(
        after_stale_projection.provider_run,
        first_reconciled_projection.provider_run
    );
    assert_eq!(
        after_stale_projection.agent_activity,
        first_reconciled_projection.agent_activity
    );
    assert_eq!(
        mixed_history(&runtime, &session_id, &stale_b_id),
        first_stale_b_history
    );
    assert_eq!(
        mixed_history(&runtime, &session_id, &stale_c_id),
        first_stale_c_history
    );
    assert_eq!(
        runtime
            .owned
            .completed_git_turn_snapshots
            .latest_projection_for_agent(&session_id, &stale_b_id),
        Some(first_stale_b_completion)
    );
    assert_eq!(
        runtime
            .owned
            .completed_git_turn_snapshots
            .latest_projection_for_agent(&session_id, &stale_c_id),
        Some(first_stale_c_completion)
    );

    let reloaded = owned_runtime_state(&app).await;
    let reloaded_revision = reloaded.session_projection_session_change_sequence(&session_id);
    assert_eq!(reloaded_revision, first_reconciled_revision);
    let reloaded_projection = reloaded
        .session_snapshot_projection(&session_id, reloaded_revision)
        .expect("reloaded projection should load");
    assert_eq!(reloaded_projection.session, first_reconciled_projection.session);
    assert_eq!(
        reloaded_projection.provider_run,
        first_reconciled_projection.provider_run
    );
    for agent_id in [&healthy_agent_id, &stale_b_id, &stale_c_id] {
        assert_eq!(
            reloaded_projection.agent_activity.get(agent_id),
            first_reconciled_projection.agent_activity.get(agent_id),
            "reconnect/reload must preserve status, turn timer, queue, and completion state"
        );
        assert_eq!(
            mixed_history(&reloaded, &session_id, agent_id),
            mixed_history(&runtime, &session_id, agent_id),
            "reconnect/reload must preserve durable turn history"
        );
    }
    assert_eq!(
        reloaded.active_turn_snapshot(),
        runtime.active_turn_snapshot(),
        "reconnect/reload must preserve active-turn parity"
    );
    for provider_run_id in [healthy_run.id(), stale_b_run.id(), stale_c_run.id()] {
        assert_eq!(
            mixed_prompt_activity_fingerprint(&reloaded, provider_run_id),
            mixed_prompt_activity_fingerprint(&runtime, provider_run_id),
            "reconnect/reload must preserve prompt activity parity"
        );
        assert_eq!(
            reloaded.owned.provider_output_deadlines.contains(provider_run_id),
            runtime.owned.provider_output_deadlines.contains(provider_run_id),
            "reconnect/reload must preserve timer parity"
        );
    }
}
