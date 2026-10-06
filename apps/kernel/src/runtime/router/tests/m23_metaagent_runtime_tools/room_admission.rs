// MP-08 / MP-10 / MP-11, A01: supplementary red-capable admission checks.
use super::*;

#[test]
fn room_admission_regular_spawn_and_direct_child_control() {
    run_large_stack_async_test(
        "room-admission-lineage",
        regular_spawn_and_direct_child_control,
    );
}

async fn regular_spawn_and_direct_child_control() {
    let env = TestMetaRuntimeEnv::new("room-admission");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut config = DaemonConfig::for_tests();
    config.user_config.workflow.session_default_max_agents = Some(1);
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, a) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .unwrap();
    let run = launch_test_provider(
        &mut app,
        session.id(),
        a.id(),
        "dev-stub",
        "dev-stub",
        "room-model",
    );
    let auth = run.runtime_mcp_auth_token().unwrap().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let specs = router.runtime_tool_specs_for_auth_token(&auth);
    assert!(
        specs.iter().any(|s| s.name == "chariox.room.run_command"),
        "A01: ordinary agent has room tools"
    );
    let created = router
        .dispatch_authenticated_runtime_tool_call(
            &auth,
            "chariox.room.run_command",
            serde_json::json!({"command":"agent spawn child --provider dev-stub"}),
        )
        .await
        .unwrap();
    assert!(created.ok, "{created:?}");
    let child = &created.payload["response"]["agent"];
    assert_eq!(child["spawned_by_agent_id"], a.id());
    let alias = router
        .dispatch_authenticated_runtime_tool_call(
            &auth,
            "chariox.room.run_command",
            serde_json::json!({"command":"agent alias child renamed"}),
        )
        .await
        .unwrap();
    assert!(alias.ok, "{alias:?}");
    let self_delete = router
        .dispatch_authenticated_runtime_tool_call(
            &auth,
            "chariox.room.run_command",
            serde_json::json!({"command":format!("agent delete {}", a.id())}),
        )
        .await
        .unwrap();
    assert!(!self_delete.ok);
}

#[test]
fn room_admission_legacy_controller_is_not_creator() {
    let mut parent = crate::agent::AgentInstance::new(
        "a",
        "aaaa",
        "room",
        None,
        "dev-stub",
        None,
        None,
        None,
        crate::agent::GridPosition::new(0, 0, 1, 1),
    );
    parent.set_controlled_by_metaagent_id(Some("legacy".into()));
    let value = serde_json::to_value(&parent).unwrap();
    assert!(
        value.get("spawned_by_agent_id").is_some(),
        "A01: lineage is an explicit unknown, never inferred from controller"
    );
    assert!(value["spawned_by_agent_id"].is_null());
}

#[test]
fn room_admission_grandchild_peer_and_workflow_boundaries() {
    run_large_stack_async_test("room-admission-boundaries", room_boundaries);
}

async fn room_command(
    router: &CommandRouter,
    auth: &str,
    command: &str,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    router
        .dispatch_authenticated_runtime_tool_call(
            auth,
            "chariox.room.run_command",
            serde_json::json!({"command":command}),
        )
        .await
        .unwrap()
}

async fn room_boundaries() {
    let env = TestMetaRuntimeEnv::new("room-boundaries");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, a) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .unwrap();
    let peer = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("peer"))
        .unwrap();
    let a_run = launch_test_provider(
        &mut app,
        session.id(),
        a.id(),
        "dev-stub",
        "dev-stub",
        "room-model",
    );
    let peer_run = launch_test_provider(
        &mut app,
        session.id(),
        peer.id(),
        "dev-stub",
        "dev-stub",
        "room-model",
    );
    let auth_a = a_run.runtime_mcp_auth_token().unwrap().to_string();
    let auth_peer = peer_run.runtime_mcp_auth_token().unwrap().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let listed = room_command(&router, &auth_a, "agent list").await;
    assert!(listed.ok);
    assert!(
        listed.payload["response"]["agents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|agent| agent["id"] == peer.id()),
        "room listing includes pre-existing peer"
    );
    assert!(
        room_command(&router, &auth_a, "agent spawn b --provider dev-stub")
            .await
            .ok
    );
    let auth_b = {
        let mut app = app.lock().await;
        let b = app
            .agents()
            .get_session_agents(session.id())
            .into_iter()
            .find(|a| a.alias() == Some("b"))
            .unwrap();
        launch_test_provider(
            &mut app,
            session.id(),
            b.id(),
            "dev-stub",
            "dev-stub",
            "room-model",
        )
        .runtime_mcp_auth_token()
        .unwrap()
        .to_string()
    };
    assert!(
        room_command(&router, &auth_b, "agent spawn d --provider dev-stub")
            .await
            .ok
    );
    for command in [
        "agent alias d stolen",
        "agent delete d",
        "agent delete peer",
        "agent focus peer",
    ] {
        assert!(
            !room_command(&router, &auth_a, command).await.ok,
            "{command}"
        );
    }
    assert!(
        room_command(&router, &auth_b, "agent alias d direct-child")
            .await
            .ok
    );
    assert!(
        room_command(&router, &auth_b, "agent delete direct-child")
            .await
            .ok
    );
    assert!(
        room_command(&router, &auth_b, "workflow new child-flow")
            .await
            .ok
    );
    assert!(
        room_command(&router, &auth_a, "workflow alias child-flow parent-edited")
            .await
            .ok
    );
    assert!(
        room_command(&router, &auth_peer, "workflow new peer-flow")
            .await
            .ok
    );
    assert!(
        room_command(&router, &auth_a, "workflow resolve peer-flow")
            .await
            .ok
    );
    assert!(
        !room_command(&router, &auth_a, "workflow alias peer-flow stolen")
            .await
            .ok
    );
    assert!(
        room_command(&router, &auth_a, "workflow new own-flow")
            .await
            .ok
    );
    assert!(
        room_command(&router, &auth_a, "workflow node add own-flow peer")
            .await
            .ok
    );
    // MP-08/MP-11 A01/G18: invalid queue rejects before admission, not an open intent.
    let node = room_command(&router, &auth_a, "workflow resolve own-flow").await;
    let node_id = node.payload["response"]["workflow"]["nodes"][0]["id"]
        .as_str()
        .unwrap();
    assert!(
        room_command(
            &router,
            &auth_a,
            &format!("workflow endpoint new own-flow {node_id} start")
        )
        .await
        .ok
    );
    let (failed, _) = router
        .runtime_state
        .execute_workflow_request(
            LocalDaemonRequest::InvokeWorkflowEndpoint(
                crate::local::InvokeWorkflowEndpointRequest {
                    session_id: session.id().into(),
                    workflow_ref: "own-flow".into(),
                    endpoint_ref: "start".into(),
                    queue_ref: Some("missing-queue".into()),
                    prompt: Some("never dispatched".into()),
                    publication_invocation: None,
                },
            ),
            a.owner_user_id().into(),
            Some(a.id().into()),
        )
        .await;
    assert!(failed.is_err());
    let app_guard = app.lock().await;
    let db = rusqlite::Connection::open(app_guard.durable_state_store().path()).unwrap();
    let rejected: i64 = db.query_row("SELECT count(*) FROM durable_state_events WHERE kind='room.obligation.dispatch_receipt' AND json_extract(payload_json,'$.dispatch_state')='rejected'", [], |r|r.get(0)).unwrap();
    assert_eq!(
        rejected, 1,
        "known queue rejection must close only its dispatch intent"
    );
    drop(app_guard);
    let peer_alias = room_command(&router, &auth_a, "agent alias peer stolen").await;
    assert!(!peer_alias.ok);
    // The raw/native typed request path sees the same mutation fence.
    assert!(router
        .runtime_state
        .authorize_room_agent_request(
            a.id(),
            &LocalDaemonRequest::AliasAgent(crate::local::AliasAgentRequest {
                session_id: session.id().into(),
                agent_id: peer.id().into(),
                alias: "stolen".into()
            })
        )
        .is_err());
    // MP-11: raw/native host-shaped requests cannot broaden descendant rights.
    for value in [
        serde_json::json!({"EndSession":{"session_id":session.id()}}),
        serde_json::json!({"DeleteSession":{"session_ref":session.id(),"workspace_id":null}}),
        serde_json::json!({"UpdateAgentProfile":{"session_id":session.id(),"agent_id":peer.id(),"model":"takeover"}}),
    ] {
        let request: LocalDaemonRequest = serde_json::from_value(value).unwrap();
        assert!(router
            .runtime_state
            .authorize_room_agent_request(a.id(), &request)
            .is_err());
    }
    // Neither aliases nor a stale provider-run credential can revive an actor.
    {
        let app = app.lock().await;
        assert_eq!(
            app.agents().get_agent(peer.id()).unwrap().alias(),
            Some("peer")
        );
    }
    router
        .runtime_state
        .authorize_room_provider_epoch(Some(a.id()), Some("stale-run"))
        .unwrap_err();
}

// MP-08 / MP-11, A01/G18: fail the real writer before the creation side effect.
#[test]
fn room_admission_writer_failure_prevents_agent_creation() {
    let env = TestMetaRuntimeEnv::new("room-writer-failure");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, actor) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .unwrap();
    let connection = rusqlite::Connection::open(app.durable_state_store().path()).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_room_intent BEFORE INSERT ON durable_state_events WHEN NEW.kind = 'room.obligation.registered' BEGIN SELECT RAISE(FAIL, 'injected room registration failure'); END;").unwrap();
    // Base compatibility: old decoders ignore the authoritative creator field.
    let mut value = serde_json::to_value(
        CreateAgentRequest::new(session.id(), "dev-stub").with_alias("not-created"),
    )
    .unwrap();
    value["spawned_by_agent_id"] = serde_json::json!(actor.id());
    let request = serde_json::from_value(value).unwrap();
    let failed = crate::app::KernelSessionService::new(&mut app).spawn_agent(request);
    assert!(
        failed.is_err(),
        "A01/G18: no success before durable registration"
    );
    assert_eq!(app.agents().get_session_agents(session.id()).len(), 1);
    connection
        .execute_batch("DROP TRIGGER fail_room_intent;")
        .unwrap();
    let mut value = serde_json::to_value(
        CreateAgentRequest::new(session.id(), "dev-stub").with_alias("created-on-retry"),
    )
    .unwrap();
    value["spawned_by_agent_id"] = serde_json::json!(actor.id());
    let created = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(serde_json::from_value(value).unwrap())
        .unwrap();
    assert_eq!(app.agents().get_session_agents(session.id()).len(), 2);
    let count: i64 = connection
        .query_row(
            "SELECT count(*) FROM durable_state_events WHERE kind = 'room.obligation.registered'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "one committed creation intent after retry");
    let json = serde_json::to_value(created).unwrap();
    assert_eq!(json["spawned_by_agent_id"], actor.id());
}

// MP-08 / MP-11, A01/G10: a bounded trace wait cannot return on a replaced run.
#[test]
fn room_admission_trace_result_rechecks_provider_epoch() {
    run_large_stack_async_test("room-stale-trace-result", stale_trace_result);
}

async fn stale_trace_result() {
    let env = TestMetaRuntimeEnv::new("room-stale-trace");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, actor) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .unwrap();
    let peer = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("peer"))
        .unwrap();
    let run = launch_test_provider(
        &mut app,
        session.id(),
        actor.id(),
        "dev-stub",
        "dev-stub",
        "room-model",
    );
    let auth = run.runtime_mcp_auth_token().unwrap().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let subscribed = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &auth,
            "chariox.trace.subscribe",
            serde_json::json!({"agent_ref":peer.id()}),
        )
        .await
        .unwrap();
    assert!(subscribed.ok);
    let bound = router
        .runtime_state
        .with_room_provider_origin(Some(actor.id()), Some(run.id()));
    let future = bound.dispatch_meta_runtime_tool_call_for_agent(
        session.id(),
        actor.id(),
        "chariox.trace.wait",
        serde_json::json!({
            "subscription_id":subscribed.payload["subscription"]["subscription_id"],
            "wait_ms":100, "until":"worker_output", "limit":1
        }),
    );
    tokio::pin!(future);
    assert!(
        futures_util::poll!(future.as_mut()).is_pending(),
        "wait must cross a real async boundary"
    );
    {
        let mut app = app.lock().await;
        app.providers_mut()
            .terminate_run_provider_only(session.id(), run.id())
            .unwrap();
        launch_test_provider(
            &mut app,
            session.id(),
            actor.id(),
            "dev-stub",
            "dev-stub",
            "replacement-model",
        );
    }
    assert!(
        future.await.is_err(),
        "A01/G10: a stale provider run must not receive a post-wait result"
    );
}

// MP-08 / MP-11, A01/G18: distinguish rejected creation from committed effects.
#[test]
fn room_admission_receipt_failure_preserves_created_resource_identity() {
    let env = TestMetaRuntimeEnv::new("room-receipt-failure");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, actor) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .unwrap();
    let db = rusqlite::Connection::open(app.durable_state_store().path()).unwrap();
    db.execute_batch("CREATE TRIGGER fail_receipt BEFORE INSERT ON durable_state_events WHEN NEW.kind='room.obligation.dispatch_receipt' BEGIN SELECT RAISE(FAIL, 'receipt failure'); END;").unwrap();
    let failed = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("created-despite-receipt-error")
                .with_spawned_by_agent_id(actor.id()),
        )
        .unwrap_err();
    let child = app
        .agents()
        .get_session_agents(session.id())
        .into_iter()
        .find(|a| a.alias() == Some("created-despite-receipt-error"))
        .unwrap();
    let message = failed.to_string();
    assert!(
        message.contains(child.id()),
        "accepted resource identity missing: {message}"
    );
    assert!(
        message.contains("accepted") && message.contains("obligation-"),
        "dispatch disposition missing: {message}"
    );
}

#[test]
fn room_admission_known_creation_rejection_records_failed_intent() {
    let env = TestMetaRuntimeEnv::new("room-rejected-creation");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, actor) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .unwrap();
    let request = || {
        CreateAgentRequest::new(session.id(), "dev-stub")
            .with_alias("duplicate")
            .with_spawned_by_agent_id(actor.id())
    };
    crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(request())
        .unwrap();
    assert!(crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(request())
        .is_err());
    let db = rusqlite::Connection::open(app.durable_state_store().path()).unwrap();
    let rejected: i64 = db.query_row("SELECT count(*) FROM durable_state_events WHERE kind='room.obligation.dispatch_receipt' AND json_extract(payload_json,'$.dispatch_state')='rejected'", [], |r|r.get(0)).unwrap();
    assert_eq!(
        rejected, 1,
        "known rejection must not leave an open dispatch intent"
    );
}
