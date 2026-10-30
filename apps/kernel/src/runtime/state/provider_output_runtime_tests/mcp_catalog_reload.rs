//! Existing provider reload/continuation queues, without launching a provider.
use super::*;

// MP-08 / MP-10 / MP-11: a passive catalog refresh must not replay a firing.
#[tokio::test]
async fn passive_catalog_change_during_trigger_turn_reloads_without_replay() {
    crate::test_support::isolated_env_test!();
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            crate::session::CreateSessionRequest::new("catalog-trigger", "catalog-trigger")
                .with_agent_defaults(crate::session::SessionAgentDefaults::new("codex")),
        )
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "agent-schedules:catalog-trigger",
            crate::attachment::ClientCapabilityLevel::AutomationOnly,
        ))
        .unwrap();
    app.prompt_owner_submit_prepared_prompt(
        session.id(),
        crate::session::PromptQueueItem::new(
            "just-fired-trigger",
            attachment.id(),
            agent.id(),
            "one firing only",
            crate::session::PromptStatus::Queued,
        )
        .with_agent_trigger("trigger-before-restart"),
        false,
    )
    .unwrap();
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-6-luna",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "catalog-trigger-run",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "metadata-only".into(),
            pty_target: None,
            pty_program: None,
            pty_args: vec![],
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.set_runtime_mcp_auth_token(Some("synthetic-catalog-trigger".into()));
    app.providers().write().insert_run_for_test(run);
    let app = Arc::new(Mutex::new(app));
    let mut runtime = owned_runtime_state(&app).await;
    runtime.owned.pending_mcp_continuations = PendingMcpContinuationStore::default();
    runtime.runtime_tool_catalog_changed_for_auth_token("synthetic-catalog-trigger");
    assert!(
        runtime.owned.pending_mcp_continuations.write().is_empty(),
        "passive changes must not manufacture a second turn from a completed trigger"
    );
    assert!(
        runtime
            .owned
            .pending_provider_reloads
            .write()
            .contains_key(agent.id()),
        "the catalog refresh must remain pending until idle"
    );
    runtime
        .owned
        .pending_provider_reloads
        .write()
        .remove(agent.id());
    runtime.remember_pending_runtime_tools_continuation(
        session.id(),
        agent.id(),
        "one firing only",
    );
    runtime.runtime_tool_catalog_changed_for_auth_token("synthetic-catalog-trigger");
    assert_eq!(
        runtime.owned.pending_mcp_continuations.write().len(),
        1,
        "an explicit loader still owns its one interrupted continuation"
    );
    assert!(
        runtime.owned.pending_provider_reloads.write().is_empty(),
        "the catalog monitor must not start a competing reload for that continuation"
    );
}

#[tokio::test]
async fn idle_native_capacity_and_lane_contention_do_not_invalidate_catalog() {
    crate::test_support::isolated_env_test!();
    use crate::provider::{
        AgentEndpointMode, LaunchProviderRequest, ProviderClientInterface, ProviderLaunchResult,
        RuntimeProviderRun,
    };
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "catalog-idle",
            "catalog-idle",
        ))
        .unwrap();
    // Fixed run metadata only. Held admission prevents any provider hook or
    // process launch; the actual native refresh method supplies the Deferred.
    let request = LaunchProviderRequest::new(session.id(), "codex", "codex", "default", "model")
        .with_agent_id(agent.id())
        .with_client_interface(ProviderClientInterface::NativeTui);
    let run = RuntimeProviderRun::new(
        "native-contention-fixture",
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "metadata-only".into(),
            pty_target: None,
            pty_program: None,
            pty_args: vec![],
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: None,
            structured_endpoint: None,
        },
    );
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let changes = runtime
        .owned
        .provider_run_projection
        .catalog_changes()
        .clone();
    let watch = changes.subscribe(run.id()).unwrap();
    let initial = watch.current().desired;
    let lane = runtime
        .owned
        .provider_store
        .run_operation_lanes()
        .try_acquire(run.id())
        .unwrap();
    for _ in 0..100 {
        assert_eq!(
            runtime
                .refresh_native_runtime_catalog(run.clone())
                .await
                .unwrap(),
            ProviderReloadOutcome::Deferred
        );
    }
    assert_eq!(watch.current().desired, initial);
    drop(lane);
    let held = (0..8)
        .map(|i| changes.begin_refresh(&format!("held-refresh-{i}")).unwrap())
        .collect::<Vec<_>>();
    for _ in 0..100 {
        assert_eq!(
            runtime
                .refresh_native_runtime_catalog(run.clone())
                .await
                .unwrap(),
            ProviderReloadOutcome::Deferred
        );
    }
    assert_eq!(watch.current().desired, initial);
    assert!(runtime
        .owned
        .provider_store
        .run_operation_lanes()
        .try_acquire(run.id())
        .is_some());
    drop(held);
    assert!(runtime.owned.provider_store.list_runs().is_empty());
}

#[tokio::test]
async fn busy_catalog_refresh_retains_typed_reason_and_self_grant_uses_one_continuation() {
    crate::test_support::isolated_env_test!();
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "fixture-workspace",
            "fixture-worktree",
        ))
        .unwrap();
    let source = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "catalog-refresh-requester",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    app.prompt_owner_submit_prepared_prompt(
        session.id(),
        crate::session::PromptQueueItem::new(
            "catalog-refresh-fixture",
            source.id(),
            agent.id(),
            "active turn",
            crate::session::PromptStatus::Queued,
        ),
        false,
    )
    .unwrap();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    assert_eq!(
        runtime
            .refresh_agent_runtime_tool_catalog(session.id(), agent.id())
            .await
            .unwrap(),
        ProviderReloadOutcome::Deferred
    );
    // A later unrelated launch setting must not discard the required refresh.
    runtime
        .reload_agent_provider_for_policy(session.id(), agent.id(), "permissions")
        .await
        .unwrap();
    let reason = runtime
        .owned
        .pending_provider_reloads
        .write()
        .get(agent.id())
        .unwrap()
        .reason
        .clone();
    assert_eq!(
        reason,
        ProviderReloadReason::RuntimeToolCatalogAndLaunchInputs("permissions".into())
    );
    // Repeated changes merge into the existing poller, retaining its deadline.
    for _ in 0..100 {
        runtime.remember_pending_provider_reload(
            session.id(),
            agent.id(),
            ProviderReloadReason::RuntimeToolCatalog,
        );
        assert!(runtime
            .owned
            .pending_provider_reloads
            .pollers
            .claim(agent.id())
            .is_none());
    }
    assert!(runtime.owned.provider_store.list_runs().is_empty());
    runtime
        .owned
        .pending_provider_reloads
        .write()
        .remove(agent.id());

    runtime.remember_pending_runtime_tools_continuation(session.id(), agent.id(), "active turn");
    let mut pending = runtime.owned.pending_mcp_continuations.write();
    let continuation = pending.remove(agent.id()).unwrap();
    assert_eq!(
        continuation.reload_reason,
        ProviderReloadReason::RuntimeToolCatalog
    );
    assert_eq!(continuation.mcp_name, "chariox-runtime");
    assert!(runtime
        .owned
        .pending_mcp_continuations
        .pollers
        .claim(agent.id())
        .is_none());
    assert!(runtime.owned.pending_provider_reloads.write().is_empty());
    assert!(runtime.owned.provider_store.list_runs().is_empty());
}

#[tokio::test]
async fn browser_loader_reports_deferred_tools_and_retains_one_kernel_continuation() {
    crate::test_support::isolated_env_test!();
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            crate::session::CreateSessionRequest::new("loader-catalog", "loader-catalog")
                .with_agent_defaults(crate::session::SessionAgentDefaults::new("codex")),
        )
        .unwrap();
    let source = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "loader-owner",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    app.prompt_owner_submit_prepared_prompt(
        session.id(),
        crate::session::PromptQueueItem::new(
            "loader-catalog-request",
            source.id(),
            agent.id(),
            "open my browser",
            crate::session::PromptStatus::Queued,
        ),
        false,
    )
    .unwrap();
    let app = Arc::new(Mutex::new(app));
    let mut runtime = owned_runtime_state(&app).await;
    runtime.owned.pending_mcp_continuations = PendingMcpContinuationStore::default();
    for already_loaded in [false, true] {
        let result = runtime.runtime_catalog_loader_result(
            &agent,
            &[],
            already_loaded,
            serde_json::json!({"loaded":"chariox.kernel_browser"}),
        );
        assert!(result.ok);
        assert_eq!(result.payload["effective"], "after_provider_reload");
        assert_eq!(result.payload["requires_provider_restart"], true);
        assert!(result.payload["note"]
            .as_str()
            .unwrap()
            .contains("End this turn"));
        let queued = runtime.owned.pending_mcp_continuations.write();
        assert_eq!(queued.len(), 1);
        assert_eq!(
            queued.get(agent.id()).unwrap().previous_prompt,
            "open my browser"
        );
    }
    let session = runtime
        .owned
        .session_store
        .get_session(session.id())
        .unwrap();
    let prompt = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, agent.id())
        .unwrap();
    let (task, correction) = runtime
        .owned
        .settle_agent_task(
            session.id(),
            agent.id(),
            &prompt,
            "catalog-fixture-run",
            false,
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        task.pending_prompt_id.as_deref(),
        Some("task-catalog:loader-catalog-request"),
        "MP-08/MP-10/MP-11: refresh must retain its source task before completion supervision"
    );
    assert!(!correction);
    assert!(!task.correction_used);
    assert_eq!(
        task.state,
        crate::durable_state::agent_lifecycle::ExecutionState::Working
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
    assert!(
        runtime.owned.provider_store.list_runs().is_empty(),
        "catalog fixture launches no provider"
    );
}
