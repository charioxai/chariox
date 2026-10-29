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
