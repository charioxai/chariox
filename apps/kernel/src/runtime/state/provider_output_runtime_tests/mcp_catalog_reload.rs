//! Existing provider reload/continuation queues, without launching a provider.
use super::*;

#[tokio::test]
async fn idle_native_capacity_and_lane_contention_do_not_invalidate_catalog() {
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
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "fixture-workspace",
            "fixture-worktree",
        ))
        .unwrap();
    app.prompt_owner_submit_prepared_prompt(
        session.id(),
        crate::session::PromptQueueItem::new(
            "catalog-refresh-fixture",
            "fixture-source",
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

    runtime.remember_pending_runtime_tools_continuation(
        session.id(),
        agent.id(),
        "fixture-source",
        "active turn",
    );
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
