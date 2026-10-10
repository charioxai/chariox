// MP-08 / MP-10 / MP-11, A01: supplementary red-capable admission checks.
use super::*;

// MP-08 / MP-11, P1: retain the caller epoch while a cold peer waits on the app.
#[test]
fn room_admission_delegated_prompt_rechecks_caller_after_app_wait() {
    run_large_stack_async_test("room-stale-delegated-prompt", stale_delegated_prompt);
}

async fn stale_delegated_prompt() {
    let env = TestMetaRuntimeEnv::new("room-stale-delegated-prompt");
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
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("cold-peer"))
        .unwrap();
    crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            format!("metaagent:{}:commands", actor.id()),
            crate::attachment::ClientCapabilityLevel::AutomationOnly,
        ))
        .unwrap();
    let run = launch_test_provider(
        &mut app,
        session.id(),
        actor.id(),
        "dev-stub",
        "dev-stub",
        "room-model",
    );
    let auth = run.runtime_mcp_auth_token().unwrap().to_owned();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    assert!(router
        .runtime_state
        .client_attachment_for_session(&format!("metaagent:{}:commands", actor.id()), session.id())
        .is_some());
    // The MCP bridge first checks remote forwarding under the app mutex. Start
    // after that preflight, with the exact local caller it retains for dispatch.
    let mut bound = router.clone();
    bound.runtime_state = router
        .runtime_state
        .with_room_provider_origin(Some(actor.id()), Some(run.id()));
    let mut guard = app.lock().await;
    let call = bound.dispatch_meta_run_command(
        &auth,
        serde_json::json!({"command":"prompt cold-peer never admitted"}),
    );
    tokio::pin!(call);
    assert!(
        futures_util::poll!(call.as_mut()).is_pending(),
        "cold peer launch must wait on the held app mutex"
    );
    assert_eq!(
        guard
            .durable_state_store()
            .load_events_by_kind("room.obligation.registered")
            .unwrap()
            .len(),
        1,
        "the wait must be inside delegated prompt admission, after registration"
    );
    guard
        .providers_mut()
        .terminate_run_provider_only(session.id(), run.id())
        .unwrap();
    launch_test_provider(
        &mut guard,
        session.id(),
        actor.id(),
        "dev-stub",
        "dev-stub",
        "replacement-model",
    );
    drop(guard);
    let result = call.await;
    let guard = app.lock().await;
    assert!(
        guard
            .providers()
            .get_run_for_agent(session.id(), peer.id())
            .is_none(),
        "stale invocation must not launch the cold peer: {result:?}"
    );
    assert!(
        !result.as_ref().is_ok_and(|result| result.ok),
        "replaced room caller must not delegate: {result:?}"
    );
    let session = guard.sessions().get_session(session.id()).unwrap();
    assert!(
        !serde_json::to_string(&session)
            .unwrap()
            .contains("never admitted"),
        "stale invocation must not admit a queued or active peer prompt"
    );
}

#[test]
fn room_admission_named_source_lookup_stays_in_room() {
    run_large_stack_async_test("room-inline-source-history", inline_source_history);
}

async fn inline_source_history() {
    let env = TestMetaRuntimeEnv::new("room-inline-source");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config.clone()).unwrap();
    let (room, actor) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .unwrap();
    let peer = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(room.id(), "dev-stub").with_alias("peer"),
        )
        .unwrap();
    let actor_run = launch_test_provider(
        &mut app,
        room.id(),
        actor.id(),
        "dev-stub",
        "dev-stub",
        "room-model",
    );
    let peer_run = launch_test_provider(
        &mut app,
        room.id(),
        peer.id(),
        "dev-stub",
        "dev-stub",
        "room-model",
    );
    let actor_auth = actor_run.runtime_mcp_auth_token().unwrap().to_owned();
    let peer_auth = peer_run.runtime_mcp_auth_token().unwrap().to_owned();
    let registry = crate::workflow_code::WorkflowCodeArtifactRegistry::new(vec![config
        .workflow_code_artifact_root()
        .join("rooms")
        .join(room.id())]);
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let source = format!(
        r#"workflow.define({{alias: "inline-review"}});
const review = workflow.node({{handle:"review",agent:workflow.existingAgent("{}"),canCompleteWorkflowRun:true}});
workflow.endpoint(review,{{handle:"entry"}});"#,
        peer.id()
    );
    let created = router
        .dispatch_authenticated_runtime_tool_call(
            &peer_auth,
            "chariox.workflow_code.create",
            serde_json::json!({"name":"peer-inline","source":source}),
        )
        .await
        .unwrap();
    assert!(created.ok, "{created:?}");
    let before = registry.get("peer-inline").unwrap().unwrap();
    let validation = router
        .dispatch_authenticated_runtime_tool_call(
            &actor_auth,
            "chariox.workflow_code.validate",
            serde_json::json!({"name":"peer-inline"}),
        )
        .await;
    assert!(
        validation.as_ref().is_ok_and(|v| v.ok),
        "A01 named validation must resolve the actual room artifact: {validation:?}"
    );
    let validation = validation.unwrap();
    assert_eq!(
        validation
            .payload
            .pointer("/WorkflowCodeValidated/result/validation/ok"),
        Some(&serde_json::json!(false))
    );
    assert!(validation
        .payload
        .to_string()
        .contains("unauthorized_existing_agent_binding"));
    assert_eq!(
        validation
            .payload
            .pointer("/WorkflowCodeValidated/result/definition/workflow/alias"),
        Some(&serde_json::json!("inline-review"))
    );
    let legacy = crate::workflow_code::WorkflowCodeArtifactRegistry::new(vec![
        config.workflow_code_artifact_root()
    ]);
    legacy
        .save(
            "legacy-only",
            before.metadata.language,
            &before.source,
            before.definition.clone(),
            before.metadata.validation.clone(),
            crate::workflow_code::WorkflowCodeArtifactActor::new(
                actor.owner_user_id(),
                Some(actor.id().to_owned()),
            ),
            crate::workflow_code::WorkflowCodeArtifactHistoryAction::Created,
        )
        .unwrap();
    let outside = router
        .dispatch_authenticated_runtime_tool_call(
            &actor_auth,
            "chariox.workflow_code.validate",
            serde_json::json!({"name":"legacy-only"}),
        )
        .await;
    assert!(
        outside.is_err(),
        "A01 room tools must not fall back to owner-wide legacy artifact roots"
    );
    for tool in ["chariox.workflow_code.apply", "chariox.workflow_code.run"] {
        let error = router
            .dispatch_authenticated_runtime_tool_call(
                &actor_auth,
                tool,
                serde_json::json!({"name":"peer-inline","source":source}),
            )
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("pass either name or source, not both"),
            "{error}"
        );
    }
    let after = registry.get("peer-inline").unwrap().unwrap();
    assert_eq!(
        after.metadata, before.metadata,
        "A01 inline source/name must not rewrite peer artifact metadata"
    );
}

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
    let grandchild = room_command(&router, &auth_b, "agent spawn d --provider dev-stub").await;
    assert!(grandchild.ok, "grandchild admission: {grandchild:?}");
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
    // MP-11 F3: a peer binding can reset the peer's provider context at run start.
    assert!(
        !room_command(&router, &auth_a, "workflow node add own-flow peer")
            .await
            .ok,
        "MP-11 F3: peers are not workflow execution resources"
    );
    assert!(
        room_command(&router, &auth_a, "workflow node add own-flow b")
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
    // MP-11 F3: saved runs retain their actual execution bindings even when a
    // newer definition is safe. Model a pre-admission snapshot at the store seam.
    let legacy_run = {
        let app = app.lock().await;
        let mut sessions = app.sessions_mut();
        let workflow = sessions
            .create_workflow_controlled_by_metaagent(
                session.id(),
                Some("legacy-peer-binding".into()),
                Some(a.id().into()),
            )
            .unwrap();
        let node = sessions
            .add_workflow_node(session.id(), workflow.id(), a.id())
            .unwrap();
        sessions
            .add_workflow_node(session.id(), workflow.id(), peer.id())
            .unwrap();
        let endpoint = sessions
            .create_workflow_endpoint(session.id(), workflow.id(), node.id(), None)
            .unwrap();
        let run = sessions
            .invoke_workflow_endpoint(session.id(), workflow.id(), endpoint.id(), None)
            .unwrap();
        let mut restored = sessions.get_session(session.id()).unwrap();
        let stored = restored.workflow_run_mut(run.id()).unwrap();
        *stored = stored.clone().with_creator(Some(a.id().into()));
        stored.set_status(crate::session::WorkflowRunStatus::Paused);
        sessions.restore_session(restored);
        run.id().to_string()
    };
    let (resumed, _) = router
        .runtime_state
        .execute_workflow_request(
            LocalDaemonRequest::ResumeWorkflowRun(crate::local::ResumeWorkflowRunRequest {
                session_id: session.id().into(),
                workflow_run_ref: legacy_run,
            }),
            a.owner_user_id().into(),
            Some(a.id().into()),
        )
        .await;
    assert!(
        resumed
            .unwrap_err()
            .to_string()
            .contains("immutable direct child"),
        "MP-11 F3: saved peer graph bindings cannot resume"
    );
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
    crate::test_support::admit_room_test_turn(&mut app, session.id(), actor.id());
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
    assert_post_creation_failure_identifies_resource("room.obligation.dispatch_receipt");
}

#[test]
fn room_admission_creation_event_failure_preserves_created_resource_identity() {
    assert_post_creation_failure_identifies_resource("agent.created");
}

fn assert_post_creation_failure_identifies_resource(kind: &str) {
    assert!(["agent.created", "room.obligation.dispatch_receipt"].contains(&kind));
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
    crate::test_support::admit_room_test_turn(&mut app, session.id(), actor.id());
    let db = rusqlite::Connection::open(app.durable_state_store().path()).unwrap();
    db.execute_batch(&format!("CREATE TRIGGER fail_receipt BEFORE INSERT ON durable_state_events WHEN NEW.kind='{kind}' BEGIN SELECT RAISE(FAIL, 'effect metadata failure'); END;")).unwrap();
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
    crate::test_support::admit_room_test_turn(&mut app, session.id(), actor.id());
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

// MP-08/MP-11 A01: binding a peer is not implicit capability administration.
#[test]
fn room_admission_workflow_binding_cannot_grant_peer_extensions() {
    let env = TestMetaRuntimeEnv::new("room-peer-capability");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    // Supplementary authored capability package in disposable test workspace, never acceptance.
    let skill = workspace.join(".chariox/skills/am1-source-regression-only");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(skill.join("SKILL.md"), "---\nname: am1-source-regression-only\ndescription: Source regression capability package.\n---\nRead-only source review.\n").unwrap();
    crate::runtime::capability_registry::ensure_skill_exists(
        Some(&workspace.to_string_lossy()),
        "am1-source-regression-only",
    )
    .unwrap();
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
    let definition: crate::workflow_code::WorkflowCodeDefinition = serde_json::from_value(serde_json::json!({
        "workflow":{"alias":"unadmitted-peer-grants"},
        "nodes":[{"handle":"review", "agent":{"kind":"existing","agent_ref":peer.id()},
            "can_complete_workflow_run":true, "extensions":[{"kind":"skill","name":"am1-source-regression-only"}]}],
        "endpoints":[{"handle":"entry","entry_node":"review"}],
    })).unwrap();
    // MP-08/MP-11: target validation must report the same provisioning fence as apply.
    let limits = crate::config::WorkflowCodeLimitsConfig::default();
    let (_, validation) = crate::app::KernelSessionService::new(&mut app)
        .validate_workflow_code_definition_with_rebindings(
            session.id(),
            &definition,
            &limits,
            &[],
            &[],
            Some(actor.id()),
        )
        .unwrap();
    assert!(
        !validation.ok,
        "room validation must reject unadmitted capability provisioning: {validation:?}"
    );
    assert!(validation
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "unauthorized_extension_provisioning"));
    let (_, owner_validation) = crate::app::KernelSessionService::new(&mut app)
        .validate_workflow_code_definition_with_rebindings(
            session.id(),
            &definition,
            &limits,
            &[],
            &[],
            None,
        )
        .unwrap();
    assert!(
        owner_validation.ok,
        "owner-authored validation retains its existing capability path: {owner_validation:?}"
    );
    let failed = crate::app::KernelSessionService::new(&mut app).apply_workflow_code_definition(
        session.id(),
        &definition,
        &crate::config::WorkflowCodeLimitsConfig::default(),
        actor.owner_user_id().into(),
        Some(actor.id().into()),
    );
    assert!(
        failed.is_err(),
        "regular room workflow cannot provision peer capabilities"
    );
    assert!(app
        .agents()
        .get_agent(peer.id())
        .unwrap()
        .extension_grants()
        .is_empty());
    assert!(app
        .sessions()
        .get_session(session.id())
        .unwrap()
        .workflows()
        .is_empty());
}

// MP-11 F7: a compiler wait must not lock unrelated runtime traffic.
#[test]
fn security_f7_compiler_wait_releases_global_app_mutex() {
    run_large_stack_async_test("security-f7-compile-lock", compiler_does_not_lock_app);
}

async fn compiler_does_not_lock_app() {
    let env = TestMetaRuntimeEnv::new("security-f7-compile-lock");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut daemon = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, _) = crate::app::KernelSessionService::new(&mut daemon)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .unwrap();
    let app = Arc::new(Mutex::new(daemon));
    let router = CommandRouter::with_interactive_capacity(app.clone(), 4);
    let marker = format!("// MP-11 F7 {}", workspace.display());
    let (started, release) = crate::workflow_code::compile_gate_for_test::install(&marker);
    let request =
        LocalDaemonRequest::ValidateWorkflowCode(crate::local::ValidateWorkflowCodeRequest {
            session_id: session.id().into(),
            node_path: "node".into(),
            source: marker + "\nworkflow.define({alias:'compile-lock'});",
            language: None,
            provider_rebindings: vec![],
            agent_rebindings: vec![],
        });
    let pending = tokio::spawn(async move {
        router
            .runtime_state
            .execute_workflow_request(request, "local-user".into(), None)
            .await
    });
    tokio::time::timeout(Duration::from_secs(10), started)
        .await
        .unwrap()
        .unwrap();
    let available = tokio::time::timeout(Duration::from_millis(200), app.lock())
        .await
        .is_ok();
    drop(release);
    let _ = pending.await.unwrap();
    assert!(available, "MP-11 F7: compiler holds the global app lock");
}

// MP-11 F7 / review R1: exercise authenticated Room create/update, not just validation.
#[test]
fn security_f7_room_artifact_create_timeout_keeps_other_rooms_responsive() {
    run_large_stack_async_test("security-f7-artifact-create", || artifact_timeout(false));
}

#[test]
fn security_f7_room_artifact_update_timeout_keeps_other_rooms_responsive() {
    run_large_stack_async_test("security-f7-artifact-update", || artifact_timeout(true));
}

async fn artifact_timeout(update: bool) {
    let env = TestMetaRuntimeEnv::new("security-f7-artifact-timeout");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    config.user_config.workflow.code = Some(crate::config::UserWorkflowCodeConfig {
        script_timeout_ms: Some(2_000),
        ..Default::default()
    });
    let mut daemon = DaemonApp::bootstrap(config.clone()).unwrap();
    let (room, actor) = crate::app::KernelSessionService::new(&mut daemon)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .unwrap();
    let actor_run = launch_test_provider(
        &mut daemon,
        room.id(),
        actor.id(),
        "dev-stub",
        "dev-stub",
        "room-model",
    );
    let auth = actor_run.runtime_mcp_auth_token().unwrap().to_owned();
    let (_, other) = crate::app::KernelSessionService::new(&mut daemon)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .unwrap();
    let other_run = launch_test_provider(
        &mut daemon,
        other.session_id(),
        other.id(),
        "dev-stub",
        "dev-stub",
        "room-model",
    );
    let other_auth = other_run.runtime_mcp_auth_token().unwrap().to_owned();
    let registry = crate::workflow_code::WorkflowCodeArtifactRegistry::new(vec![config
        .workflow_code_artifact_root()
        .join("rooms")
        .join(room.id())]);
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(daemon)), 4);
    if update {
        let created = router.dispatch_authenticated_runtime_tool_call(&auth, "chariox.workflow_code.create", serde_json::json!({"name":"timeout-check", "source":format!("workflow.define({{alias:'timeout-check'}});const node=workflow.node({{handle:'self',agent:workflow.existingAgent('{}'),canCompleteWorkflowRun:true}});workflow.endpoint(node,{{handle:'entry'}});",actor.id())})).await.unwrap();
        assert!(created.ok, "{created:?}");
    }
    let before = registry.get("timeout-check").unwrap();
    let marker = format!("// MP-11 F7 {}", workspace.display());
    let (started, release) = crate::workflow_code::compile_gate_for_test::install(&marker);
    let pending = tokio::spawn({
        let router = router.clone();
        async move {
            router.dispatch_authenticated_runtime_tool_call(&auth, if update {"chariox.workflow_code.update"} else {"chariox.workflow_code.create"}, serde_json::json!({"name":"timeout-check", "source":format!("{marker}\nwhile (true) {{}}") })).await
        }
    });
    tokio::time::timeout(Duration::from_secs(10), started)
        .await
        .unwrap()
        .unwrap();
    // Release the test gate: the actual isolated script runs to its real timeout.
    drop(release);
    let responsive = tokio::time::timeout(
        Duration::from_millis(500),
        router.dispatch_authenticated_runtime_tool_call(
            &other_auth,
            "chariox.workflow_code.list",
            serde_json::json!({}),
        ),
    )
    .await;
    let compiler_result = pending.await.unwrap();
    let failure = compiler_result.as_ref().err().map(ToString::to_string);
    assert!(failure.as_deref().is_some_and(|message| message.contains("timeout") || message.contains("timed out")), "MP-11 F7: script must reach its execution timeout, not fail during compiler startup: {compiler_result:?}");
    assert_eq!(
        registry.get("timeout-check").unwrap().map(|a| a.metadata),
        before.map(|a| a.metadata),
        "timed-out compilation must not change stored source"
    );
    assert!(
        responsive.is_ok_and(|result| result.is_ok_and(|result| result.ok)),
        "MP-11 F7: Room artifact compilation blocks another room's command"
    );
}

// MP-11 F5: a command admitted under a provider epoch must retain it in the lane.
#[test]
fn security_f5_workflow_lane_rechecks_queued_provider_epoch() {
    run_large_stack_async_test("security-f5-workflow-epoch", queued_workflow_epoch);
}

async fn queued_workflow_epoch() {
    let env = TestMetaRuntimeEnv::new("security-f5-workflow-epoch");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut daemon = DaemonApp::bootstrap(config).unwrap();
    let (session, actor) = crate::app::KernelSessionService::new(&mut daemon)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .unwrap();
    let run = launch_test_provider(
        &mut daemon,
        session.id(),
        actor.id(),
        "dev-stub",
        "dev-stub",
        "room-model",
    );
    let app = Arc::new(Mutex::new(daemon));
    let router = CommandRouter::with_interactive_capacity(app.clone(), 4);
    let mut state = router.runtime_state.clone();
    let probe = Arc::new(tokio::sync::Notify::new());
    state.observe_app_lock_wait_for_test(probe.clone());
    let lane = crate::runtime::workflow_actor::WorkflowRuntime::new(
        state.clone(),
        router.session_projection.clone(),
        router.agent_runtime_projection.clone(),
    );
    let mut guard = app.lock().await;
    let request =
        LocalDaemonRequest::ValidateWorkflowCode(crate::local::ValidateWorkflowCodeRequest {
            session_id: session.id().into(),
            node_path: "node".into(),
            source: "workflow.define({alias:'lane-blocker'});".into(),
            language: None,
            provider_rebindings: vec![],
            agent_rebindings: vec![],
        });
    let first = tokio::spawn({
        let lane = lane.clone();
        async move {
            let command = crate::runtime::command::KernelCommand::from_local_request(
                "f5-blocker",
                None,
                None,
                &request,
            );
            lane.dispatch_workflow_command(command, request).await
        }
    });
    tokio::time::timeout(Duration::from_secs(5), probe.notified())
        .await
        .unwrap();
    state
        .authorize_room_provider_epoch(Some(actor.id()), Some(run.id()))
        .unwrap();
    let request = LocalDaemonRequest::CreateWorkflow(crate::local::CreateWorkflowRequest {
        session_id: session.id().into(),
        alias: Some("stale-mutation".into()),
    });
    let mut command = crate::runtime::command::KernelCommand::from_local_request(
        "f5-stale", None, None, &request,
    );
    command.caller.metaagent_id = Some(actor.id().into());
    command.provider_run_id = Some(run.id().into());
    let second = tokio::spawn({
        let lane = lane.clone();
        async move { lane.dispatch_workflow_command(command, request).await }
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while lane
            .queue_snapshots()
            .await
            .iter()
            .all(|queue| queue.queued_commands == 0)
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    guard
        .providers_mut()
        .terminate_run_provider_only(session.id(), run.id())
        .unwrap();
    launch_test_provider(
        &mut guard,
        session.id(),
        actor.id(),
        "dev-stub",
        "dev-stub",
        "replacement",
    );
    drop(guard);
    let _ = first.await.unwrap();
    let response = second.await.unwrap();
    assert!(
        response.is_err(),
        "MP-11 F5: stale queued workflow mutation succeeded: {response:?}"
    );
    assert!(app
        .lock()
        .await
        .sessions()
        .resolve_workflow_ref(session.id(), "stale-mutation")
        .is_err());
}

// MP-11 R1: two authenticated Room callers must not publish the same name.
#[test]
fn room_admission_concurrent_registry_add_preserves_winner_identity() {
    run_large_stack_async_test("room-registry-add-race", concurrent_registry_add);
}

async fn concurrent_registry_add() {
    let env = TestMetaRuntimeEnv::new("room-registry-add-race");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    config.user_config.state.path = Some(env.root.join("state/state.db").display().to_string());
    let mut daemon = DaemonApp::bootstrap(config.clone()).unwrap();
    let (room, actor) = crate::app::KernelSessionService::new(&mut daemon)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .unwrap();
    let peer = crate::app::KernelSessionService::new(&mut daemon)
        .spawn_agent(CreateAgentRequest::new(room.id(), "dev-stub").with_alias("peer"))
        .unwrap();
    let mut auth = Vec::new();
    for agent in [&actor, &peer] {
        let run = launch_test_provider(
            &mut daemon,
            room.id(),
            agent.id(),
            "dev-stub",
            "dev-stub",
            "room-model",
        );
        auth.push(run.runtime_mcp_auth_token().unwrap().to_owned());
    }
    let registry_root = config
        .workflow_registry_root()
        .join("rooms")
        .join(room.id());
    let registry = crate::workflow_code::WorkflowRegistry::new(None, Some(registry_root.clone()));
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(daemon)), 4);
    let marker = format!("// MP-11 R1 {}", workspace.display());
    let source = format!("{marker}\nworkflow.define({{alias:'winner'}});const node=workflow.node({{handle:'self',agent:workflow.existingAgent('{}'),canCompleteWorkflowRun:true}});workflow.endpoint(node,{{handle:'entry'}});", actor.id());
    let losing_source = source
        .replace("winner", "loser")
        .replace(actor.id(), peer.id());
    let (started, release) = crate::workflow_code::compile_gate_for_test::install(&marker);
    let first = tokio::spawn({
        let router = router.clone();
        let auth = auth[0].clone();
        let source = source.clone();
        async move {
            router.dispatch_authenticated_runtime_tool_call(&auth, "chariox.workflow_registry.add", serde_json::json!({"name":"same-name", "source":{"kind":"single_file", "source":source}})).await
        }
    });
    tokio::time::timeout(Duration::from_secs(10), started)
        .await
        .unwrap()
        .unwrap();
    // The first job has written source and is compiling outside the app mutex.
    let second = tokio::time::timeout(Duration::from_secs(10),
        router.dispatch_authenticated_runtime_tool_call(&auth[1], "chariox.workflow_registry.add", serde_json::json!({"name":"same-name", "source":{"kind":"single_file", "source":losing_source}}))).await;
    drop(release);
    let first = first.await.unwrap();
    let second = second.expect("same-name addition should reject without waiting on compilation");
    assert!(
        second
            .as_ref()
            .err()
            .is_some_and(|e| e.to_string().contains("conflict")),
        "MP-11 R1: losing Room job needs a clear name conflict: {second:?}"
    );
    let first = first.expect("reserved first job must publish its own entry");
    assert!(first.ok, "{first:?}");
    let winner = registry
        .resolve("same-name")
        .expect("winner source and file hashes must resolve together");
    assert_eq!(winner.source, source);
    assert_eq!(
        winner.metadata.created_by_agent_id.as_deref(),
        Some(actor.id())
    );
    assert_eq!(
        winner.metadata.source_sha256,
        crate::workflow_code::sha256_hex(source.as_bytes())
    );
    let compile = crate::workflow_code::compile_workflow_code_javascript(
        "node",
        &source,
        &config.workflow_code_limits(),
    )
    .unwrap();
    assert_eq!(
        winner.metadata.definition_sha256,
        Some(crate::workflow_code::workflow_code_definition_sha256_hex(
            &compile.definition
        ))
    );
    assert_eq!(
        serde_json::to_value(&winner.metadata).unwrap(),
        first.payload["WorkflowRegistryEntryAdded"]["entry"]
    );
    assert!(
        !std::fs::read_dir(&registry_root).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".tmp-")),
        "each job must remove only its own staging directory"
    );
}
