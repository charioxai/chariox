use super::*;

// MP-08 / MP-10 / MP-11: a missing home task must never silently discard work.
#[tokio::test]
async fn catalog_reload_without_task_intent_projects_failure_without_replay() {
    let scratch = std::env::temp_dir().join(format!(
        "chariox-catalog-missing-{:032x}",
        rand::random::<u128>()
    ));
    std::fs::create_dir_all(&scratch).unwrap();
    let mut config =
        crate::config::DaemonConfig::for_tests().with_session_history_root(scratch.join("history"));
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            crate::session::CreateSessionRequest::new(
                scratch.to_string_lossy(),
                scratch.to_string_lossy(),
            )
            .with_agent_defaults(crate::session::SessionAgentDefaults::new("dev-stub")),
        )
        .unwrap();
    app.launch_provider(
        crate::provider::LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "dev-stub",
            "default",
            "default",
        )
        .with_agent_id(agent.id()),
    )
    .unwrap();
    let app = Arc::new(Mutex::new(app));
    let mut runtime = owned_runtime_state(&app).await;
    runtime.owned.pending_mcp_continuations = PendingMcpContinuationStore::default();
    runtime.owned.pending_mcp_continuations.write().insert(
        agent.id().into(),
        PendingMcpContinuation {
            session_id: session.id().into(),
            agent_id: agent.id().into(),
            source_prompt_id: Some("untracked-source".into()),
            mcp_name: "chariox-runtime".into(),
            previous_prompt: "finish the original request".into(),
            reload_reason: ProviderReloadReason::RuntimeToolCatalog,
        },
    );
    let result = runtime
        .run_pending_mcp_continuation_after_completion(session.id(), agent.id())
        .await;
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("home-owned task intent is missing"));
    let entries = runtime
        .owned
        .operational_history_store
        .load_session_history_entries(session.id(), Some(agent.id()))
        .unwrap();
    assert!(
        entries.iter().any(|e| e
            .text
            .contains("could not safely continue the interrupted request")),
        "missing intent must produce a user-visible failure"
    );
    assert!(
        runtime
            .owned
            .durable_state_store
            .agent_tasks(Some(session.id()), Some(agent.id()))
            .unwrap()
            .is_empty(),
        "a worker or stale continuation must never manufacture a new task"
    );
    assert!(runtime
        .owned
        .session_store
        .get_session(session.id())
        .unwrap()
        .active_prompt_for_agent(agent.id())
        .is_none());
    drop(runtime);
    drop(app);
    std::fs::remove_dir_all(scratch).unwrap();
}

#[tokio::test]
async fn accepted_catalog_continuation_survives_source_provider_retirement() {
    crate::test_support::isolated_env_test!();
    let worktree = crate::test_support::TestWorktree::new("catalog-retired-source");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            worktree
                .session_request()
                .with_agent_defaults(crate::session::SessionAgentDefaults::new("dev-stub")),
        )
        .unwrap();
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
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let mut runtime = owned_runtime_state(&app).await;
    runtime.owned.pending_mcp_continuations = PendingMcpContinuationStore::default();
    let prompt = crate::session::PromptQueueItem::new(
        "catalog-source",
        "kernel-owner",
        agent.id(),
        "finish the request",
        crate::session::PromptStatus::Queued,
    );
    runtime
        .owned
        .ensure_agent_task(session.id(), agent.id(), &prompt, Some(run.id()))
        .unwrap();
    runtime
        .owned
        .durable_state_store
        .agent_lifecycle(
            crate::durable_state::agent_lifecycle::Operation::PrepareCatalogContinuation {
                room: session.id().into(),
                agent: agent.id().into(),
                prompt: prompt.id().into(),
                run: run.id().into(),
                next_prompt: format!("task-catalog:{}", prompt.id()),
                now: crate::session::unix_epoch_ms(),
            },
        )
        .unwrap();
    runtime.owned.pending_mcp_continuations.write().insert(
        agent.id().into(),
        PendingMcpContinuation {
            session_id: session.id().into(),
            agent_id: agent.id().into(),
            source_prompt_id: Some(prompt.id().into()),
            mcp_name: "chariox-runtime".into(),
            previous_prompt: prompt.prompt().into(),
            reload_reason: ProviderReloadReason::RuntimeToolCatalog,
        },
    );
    let scoped = runtime.with_room_provider_origin(Some(agent.id()), Some(run.id()));
    runtime
        .owned
        .provider_store
        .terminate_run_provider_only(session.id(), run.id())
        .unwrap();
    scoped
        .run_pending_mcp_continuation_after_completion(session.id(), agent.id())
        .await
        .expect("accepted catalog work belongs to its task after the source provider retires");
    let resumed = runtime
        .owned
        .session_store
        .get_session(session.id())
        .unwrap();
    let active = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&resumed, agent.id());
    assert_eq!(
        active.as_ref().map(|p| p.id()),
        Some("task-catalog:catalog-source")
    );
    assert_eq!(
        runtime
            .owned
            .durable_state_store
            .agent_tasks(Some(session.id()), Some(agent.id()))
            .unwrap()
            .len(),
        1
    );
}

// P2-4: a loader promises "Chariox will continue your request"; the whole
// completion path (settlement, reload, task replay) must keep that promise.
#[tokio::test]
async fn loader_turn_completion_schedules_the_promised_continuation() {
    crate::test_support::isolated_env_test!();
    let worktree = crate::test_support::TestWorktree::new("catalog-loader-continuation");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    // The agent's provider reloads its tool catalog; the fixture run is inert.
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
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
        .unwrap();
    let source = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "loader-requester",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let crate::session::PromptSubmissionOutcome::Started { prompt } = app
        .submit_prompt(
            session.id(),
            source.id(),
            Some(agent.id()),
            "open example.com in my browser",
            Vec::new(),
        )
        .unwrap()
    else {
        panic!("the loader turn must start");
    };
    let continuation_id = format!("task-catalog:{}", prompt.id());
    let app = Arc::new(Mutex::new(app));
    let mut runtime = owned_runtime_state(&app).await;
    runtime.owned.pending_mcp_continuations = PendingMcpContinuationStore::default();
    let result = runtime.runtime_catalog_loader_result(
        &agent,
        &[],
        false,
        serde_json::json!({"loaded":"chariox.kernel_browser"}),
    );
    assert_eq!(result.payload["requires_provider_restart"], true);

    runtime
        .settle_owned_provider_prompt(session.id(), run.id(), true, false, true)
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let sequence = runtime
                .owned
                .session_projection
                .session_change_sequence(session.id());
            let state = runtime
                .owned
                .session_store
                .get_session(session.id())
                .unwrap();
            if state
                .active_prompt_for_agent(agent.id())
                .is_some_and(|prompt| prompt.id() == continuation_id)
            {
                break;
            }
            runtime
                .owned
                .session_projection
                .wait_for_session_change_after(session.id(), sequence)
                .await;
        }
    })
    .await
    .expect("the promised continuation turn must start after the loader turn ends");
}

// P2-4: when the refreshed task cannot resume, the owner is told instead of
// the Room staying idle after the agent promised a continuation.
#[tokio::test]
async fn catalog_continuation_without_a_retained_intent_projects_failure() {
    crate::test_support::isolated_env_test!();
    let worktree = crate::test_support::TestWorktree::new("catalog-no-retained-intent");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            worktree
                .session_request()
                .with_agent_defaults(crate::session::SessionAgentDefaults::new("dev-stub")),
        )
        .unwrap();
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
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let mut runtime = owned_runtime_state(&app).await;
    runtime.owned.pending_mcp_continuations = PendingMcpContinuationStore::default();
    let prompt = crate::session::PromptQueueItem::new(
        "catalog-unretained",
        "kernel-owner",
        agent.id(),
        "finish the request",
        crate::session::PromptStatus::Queued,
    );
    // The turn is admitted but its catalog hand-off never retained a continuation.
    runtime
        .owned
        .ensure_agent_task(session.id(), agent.id(), &prompt, Some(run.id()))
        .unwrap();
    runtime.owned.pending_mcp_continuations.write().insert(
        agent.id().into(),
        PendingMcpContinuation {
            session_id: session.id().into(),
            agent_id: agent.id().into(),
            source_prompt_id: Some(prompt.id().into()),
            mcp_name: "chariox-runtime".into(),
            previous_prompt: prompt.prompt().into(),
            reload_reason: ProviderReloadReason::RuntimeToolCatalog,
        },
    );
    assert!(runtime
        .run_pending_mcp_continuation_after_completion(session.id(), agent.id())
        .await
        .is_err());
    let entries = runtime
        .owned
        .operational_history_store
        .load_session_history_entries(session.id(), Some(agent.id()))
        .unwrap();
    assert!(
        entries.iter().any(|e| e
            .text
            .contains("could not safely continue the interrupted request")),
        "a continuation that cannot resume must tell the owner"
    );
}

// A catalog continuation that was never delivered leaves its task Working
// with the continuation pending. Settling such a task again changes nothing.
struct StaleCatalogTask {
    runtime: KernelRuntimeState,
    session_id: String,
    agent_id: String,
    run_id: String,
    owner_id: String,
    _app: Arc<Mutex<DaemonApp>>,
}

async fn stale_catalog_task_on_a_parked_run(name: &str) -> StaleCatalogTask {
    let worktree = crate::test_support::TestWorktree::new(name);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            worktree
                .session_request()
                .with_agent_defaults(crate::session::SessionAgentDefaults::new("dev-stub")),
        )
        .unwrap();
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
        .unwrap();
    let owner = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "owner",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let mut runtime = owned_runtime_state(&app).await;
    runtime.owned.pending_mcp_continuations = PendingMcpContinuationStore::default();
    let long_ago = crate::session::unix_epoch_ms() - 600_000;
    let store = &runtime.owned.durable_state_store;
    store
        .agent_lifecycle(crate::durable_state::agent_lifecycle::Operation::Begin {
            owner: "local-user".into(),
            room: session.id().into(),
            agent: agent.id().into(),
            prompt: "catalog-source".into(),
            run: Some(run.id().into()),
            now: long_ago,
        })
        .unwrap();
    store
        .agent_lifecycle(
            crate::durable_state::agent_lifecycle::Operation::PrepareCatalogContinuation {
                room: session.id().into(),
                agent: agent.id().into(),
                prompt: "catalog-source".into(),
                run: run.id().into(),
                next_prompt: "task-catalog:catalog-source".into(),
                now: long_ago,
            },
        )
        .unwrap();
    let mut asked = crate::history::SessionHistoryEntry::user_prompt(
        session.id(),
        "att",
        agent.id(),
        "finish the request",
    );
    asked.merge_key = Some("prompt:catalog-source".into());
    let answered = crate::history::SessionHistoryEntry::provider_output(
        session.id(),
        run.id(),
        Some(agent.id()),
        crate::terminal::TerminalOutputKind::ProviderOutput,
        Some("answer".into()),
        "Chariox will continue your request automatically.",
    );
    runtime
        .owned
        .append_history_entries(session.id(), vec![asked, answered]);
    runtime
        .owned
        .provider_store
        .park_run_provider_only(session.id(), run.id())
        .unwrap();
    // The history writer is asynchronous; the answer must be readable.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    StaleCatalogTask {
        session_id: session.id().into(),
        agent_id: agent.id().into(),
        run_id: run.id().into(),
        owner_id: owner.id().into(),
        runtime,
        _app: app,
    }
}

impl StaleCatalogTask {
    fn notices(&self) -> Vec<String> {
        self.runtime
            .owned
            .operational_history_store
            .load_session_history_entries(&self.session_id, Some(&self.agent_id))
            .unwrap()
            .into_iter()
            .filter(|e| e.kind == crate::history::SessionHistoryEntryKind::Notice)
            .map(|e| e.text)
            .collect()
    }

    fn catalog_task(&self) -> crate::durable_state::agent_lifecycle::AgentTaskExecution {
        self.runtime
            .owned
            .durable_state_store
            .agent_tasks(Some(&self.session_id), Some(&self.agent_id))
            .unwrap()
            .into_iter()
            .find(|task| task.prompt_id == "catalog-source")
            .unwrap()
    }
}

// A prompt reaches a parked agent whose earlier catalog continuation was lost.
// Once that prompt ends, the sweep found the stale task, "settled" it into
// itself and woke the sweep again: a notice storm at CPU speed.
#[tokio::test]
async fn prompt_to_a_parked_agent_with_a_stale_catalog_task_posts_no_notice_storm() {
    crate::test_support::isolated_env_test!();
    let fixture = stale_catalog_task_on_a_parked_run("catalog-parked-storm").await;
    let runtime = &fixture.runtime;
    let second = crate::session::PromptQueueItem::new(
        "second-prompt",
        &fixture.owner_id,
        &fixture.agent_id,
        "are you there?",
        crate::session::PromptStatus::Queued,
    );
    let submission = runtime
        .submit_prepared_prompt_with_queue_policy(
            crate::app::KernelPreparedPromptSubmission {
                session_id: fixture.session_id.clone(),
                prompt: second,
                force_queue: false,
                refresh_projection: true,
            },
            true,
        )
        .await
        .unwrap();
    assert!(matches!(
        submission.outcome,
        crate::session::PromptSubmissionOutcome::Started { .. }
    ));
    runtime.detach(&fixture.owner_id).await.unwrap();
    runtime
        .settle_owned_provider_prompt(&fixture.session_id, &fixture.run_id, true, false, true)
        .await
        .unwrap();
    runtime.sweep_agent_lifecycle_for_test().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    let notices = fixture.notices();
    assert!(
        notices.len() <= 3,
        "notice storm: {} notices, first {:?}",
        notices.len(),
        &notices[..notices.len().min(3)]
    );
    // The sweep ends the lost continuation instead of settling it again.
    let task = fixture.catalog_task();
    assert_ne!(
        task.state,
        crate::durable_state::agent_lifecycle::ExecutionState::Working,
        "{:?} {notices:?}",
        task.reason
    );
}

// Settling a task whose continuation is still pending changes nothing, so it
// is no settlement for the sweep to act on.
#[tokio::test]
async fn pending_catalog_continuation_is_not_an_unsettled_turn() {
    crate::test_support::isolated_env_test!();
    let fixture = stale_catalog_task_on_a_parked_run("catalog-parked-unsettled").await;
    let task = fixture.catalog_task();
    assert!(fixture
        .runtime
        .owned
        .settle_answered_unsettled_turn(&task)
        .unwrap()
        .is_none());
}

// A settlement that is retried for the same task revision is not news: one
// notice, however many times the sweep or a retry path comes back to it.
#[tokio::test]
async fn retried_task_settlement_announces_a_revision_once() {
    crate::test_support::isolated_env_test!();
    let fixture = stale_catalog_task_on_a_parked_run("catalog-parked-retry").await;
    let task = fixture.catalog_task();
    let source = crate::session::PromptQueueItem::new(
        "catalog-source",
        "kernel-supervisor",
        &fixture.agent_id,
        "",
        crate::session::PromptStatus::Completed,
    );
    for _ in 0..5 {
        fixture
            .runtime
            .finish_agent_task_settlement(Some((task.clone(), false)), &source)
            .await
            .unwrap();
    }
    let working = |notices: Vec<String>| {
        notices
            .into_iter()
            .filter(|text| text.ends_with(": Working."))
            .count()
    };
    assert_eq!(working(fixture.notices()), 1);
}
