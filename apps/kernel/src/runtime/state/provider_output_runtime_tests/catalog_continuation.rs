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
    loader_turn_promised_continuation_starts(false).await;
}

// P2-4: the turn's client can leave as the turn ends. The kernel owns the
// promised continuation, so it must not depend on any client staying attached.
#[tokio::test]
async fn loader_turn_continuation_survives_the_last_client_leaving() {
    loader_turn_promised_continuation_starts(true).await;
}

async fn loader_turn_promised_continuation_starts(requester_leaves_after_turn: bool) {
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
    if requester_leaves_after_turn {
        // The continuation task is spawned on this runtime and has not run yet.
        runtime.owned.detach(source.id()).unwrap();
    }
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

// P2-4: a cold provider start behind the relaunch delay takes longer than the
// old 30 s wait; the continuation must still find the provider Running.
#[tokio::test(start_paused = true)]
async fn continuation_outlasts_a_slow_provider_relaunch() {
    crate::test_support::isolated_env_test!();
    let worktree = crate::test_support::TestWorktree::new("catalog-slow-relaunch");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            worktree
                .session_request()
                .with_agent_defaults(crate::session::SessionAgentDefaults::new("dev-stub")),
        )
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let relaunch = async {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        app.lock()
            .await
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
    };
    let (waited, ()) = tokio::join!(
        runtime.wait_for_agent_provider_relaunch(session.id(), agent.id(), None),
        relaunch
    );
    waited.expect("a provider that starts within two minutes must receive the continuation");
}

// P2-4: the kernel keeps the session attached from the moment it takes a
// continuation, so no client leaving mid-relaunch can park the new provider run.
#[tokio::test]
async fn taken_continuation_keeps_the_session_attached_through_the_relaunch() {
    crate::test_support::isolated_env_test!();
    let worktree = crate::test_support::TestWorktree::new("catalog-attached-relaunch");
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
    let app = Arc::new(Mutex::new(app));
    let mut runtime = owned_runtime_state(&app).await;
    runtime.owned.pending_mcp_continuations = PendingMcpContinuationStore::default();
    runtime.owned.pending_mcp_continuations.write().insert(
        agent.id().into(),
        PendingMcpContinuation {
            session_id: session.id().into(),
            agent_id: agent.id().into(),
            source_prompt_id: None,
            mcp_name: "chariox-runtime".into(),
            previous_prompt: "finish the request".into(),
            reload_reason: ProviderReloadReason::RuntimeToolCatalog,
        },
    );
    assert!(runtime
        .owned
        .attachment_store
        .list_session_attachment_ids(session.id())
        .is_empty());
    // The continuation has no task intent, so it ends in a reported failure;
    // the session must nevertheless have been held for the reload before that.
    let _ = runtime
        .run_pending_mcp_continuation_after_completion(session.id(), agent.id())
        .await;
    assert!(
        !runtime
            .owned
            .attachment_store
            .list_session_attachment_ids(session.id())
            .is_empty(),
        "the kernel must hold the session while it relaunches the provider"
    );
}

// P2-4: whatever stops the provider refresh, the agent's promise is kept or
// the owner is told; a logged warning alone leaves the Room idle and silent.
#[tokio::test]
async fn failed_provider_refresh_tells_the_owner_instead_of_dropping_the_continuation() {
    crate::test_support::isolated_env_test!();
    let worktree = crate::test_support::TestWorktree::new("catalog-refresh-failure");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            worktree
                .session_request()
                .with_agent_defaults(crate::session::SessionAgentDefaults::new("claude")),
        )
        .unwrap();
    // A reloadable provider run with no account behind it: the relaunch cannot be prepared.
    let mut run = crate::provider::RuntimeProviderRun::from_control_capability_inference(
        "refresh-failure-fixture",
        session.id().into(),
        Some(agent.id().into()),
        "claude".into(),
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run);
    let app = Arc::new(Mutex::new(app));
    let mut runtime = owned_runtime_state(&app).await;
    runtime.owned.pending_mcp_continuations = PendingMcpContinuationStore::default();
    runtime.owned.pending_mcp_continuations.write().insert(
        agent.id().into(),
        PendingMcpContinuation {
            session_id: session.id().into(),
            agent_id: agent.id().into(),
            source_prompt_id: None,
            mcp_name: "chariox-runtime".into(),
            previous_prompt: "finish the request".into(),
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
        "a continuation whose provider refresh failed must tell the owner"
    );
}
