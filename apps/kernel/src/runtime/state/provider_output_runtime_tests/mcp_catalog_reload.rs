//! Existing provider reload/continuation queues, without launching a provider.
use super::*;

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
    assert_eq!(reason, ProviderReloadReason::RuntimeToolCatalog);
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
    assert!(runtime.owned.pending_provider_reloads.write().is_empty());
    assert!(runtime.owned.provider_store.list_runs().is_empty());
}
