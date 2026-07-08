use super::*;

#[tokio::test]
async fn metaagent_trace_subscription_drains_live_worker_output() {
    let env = TestMetaRuntimeEnv::new("trace-subscription");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, worker) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    mark_test_agent_controlled_by_metaagent(&mut app, worker.id(), metaagent.id());
    let worker_run = launch_test_provider(
        &mut app,
        session.id(),
        worker.id(),
        "dev-stub",
        "dev-stub",
        "worker-model",
    );
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let subscribed = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_SUBSCRIBE_TRACE_TOOL,
            serde_json::json!({ "agent_ref": worker.id() }),
        )
        .await
        .expect("subscribe_trace should dispatch");
    assert!(subscribed.ok, "{:?}", subscribed.payload);
    let subscription_id = subscribed
        .payload
        .pointer("/subscription/subscription_id")
        .and_then(serde_json::Value::as_str)
        .expect("subscribe_trace should return subscription id")
        .to_string();

    {
        let mut app = app.lock().await;
        app.fan_out_output(
            session.id(),
            worker_run.id(),
            crate::terminal::TerminalOutputKind::ProviderTool,
            None,
            Vec::new(),
            serde_json::json!({
                "tool": "bash",
                "status": "running",
                "input": {"command": "printf trace-visible"}
            })
            .to_string()
            .as_bytes(),
        );
    }

    let polled = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_POLL_TRACE_TOOL,
            serde_json::json!({ "subscription_id": subscription_id, "limit": 10 }),
        )
        .await
        .expect("poll_trace should dispatch");
    assert!(polled.ok, "{:?}", polled.payload);
    assert_eq!(
        polled
            .payload
            .pointer("/items/0/title")
            .and_then(serde_json::Value::as_str),
        Some("bash · RUNNING"),
        "{:?}",
        polled.payload
    );
    assert!(
        polled
            .payload
            .pointer("/items/0/summary")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|summary| summary.contains("printf trace-visible")),
        "{:?}",
        polled.payload
    );
    assert!(
        polled
            .payload
            .pointer("/supervision/last_meaningful_output/summary")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|summary| summary.contains("printf trace-visible")),
        "{:?}",
        polled.payload
    );

    {
        let mut app = app.lock().await;
        app.fan_out_output(
            session.id(),
            worker_run.id(),
            crate::terminal::TerminalOutputKind::ProviderTool,
            None,
            Vec::new(),
            serde_json::json!({
                "tool": "bash",
                "status": "running",
                "input": {"command": "printf trace-visible"}
            })
            .to_string()
            .as_bytes(),
        );
    }

    let duplicate = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_POLL_TRACE_TOOL,
            serde_json::json!({ "subscription_id": subscription_id, "limit": 10 }),
        )
        .await
        .expect("duplicate poll_trace should dispatch");
    assert!(duplicate.ok, "{:?}", duplicate.payload);
    assert_eq!(
        duplicate
            .payload
            .get("empty")
            .and_then(serde_json::Value::as_bool),
        Some(true),
        "compact trace should suppress repeated identical lifecycle records: {:?}",
        duplicate.payload
    );
    assert_eq!(
        duplicate
            .payload
            .get("suppressed_count")
            .and_then(serde_json::Value::as_u64),
        Some(1),
        "{:?}",
        duplicate.payload
    );
    assert_eq!(
        duplicate
            .payload
            .pointer("/supervision/message")
            .and_then(serde_json::Value::as_str),
        Some("no meaningful worker output yet"),
        "{:?}",
        duplicate.payload
    );

    let drained = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_POLL_TRACE_TOOL,
            serde_json::json!({ "agent_ref": worker.id() }),
        )
        .await
        .expect("second poll_trace should dispatch");
    assert!(drained.ok, "{:?}", drained.payload);
    assert_eq!(
        drained
            .payload
            .get("empty")
            .and_then(serde_json::Value::as_bool),
        Some(true),
        "{:?}",
        drained.payload
    );

    {
        let mut app = app.lock().await;
        for _ in 0..2 {
            app.fan_out_output(
                session.id(),
                worker_run.id(),
                crate::terminal::TerminalOutputKind::PromptEcho,
                None,
                Vec::new(),
                b"worker prompt echo",
            );
        }
        app.fan_out_output(
            session.id(),
            worker_run.id(),
            crate::terminal::TerminalOutputKind::ProviderOutput,
            None,
            Vec::new(),
            b"worker output trace-visible",
        );
    }

    let waited = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_WAIT_TRACE_TOOL,
            serde_json::json!({
                "subscription_id": subscription_id,
                "until": "worker_output",
                "wait_ms": 1000,
                "limit": 10
            }),
        )
        .await
        .expect("wait_trace should dispatch");
    assert!(waited.ok, "{:?}", waited.payload);
    assert_eq!(
        waited
            .payload
            .get("matched")
            .and_then(serde_json::Value::as_bool),
        Some(true),
        "{:?}",
        waited.payload
    );
    let items = waited
        .payload
        .get("items")
        .and_then(serde_json::Value::as_array)
        .expect("wait_trace should return items");
    assert_eq!(
        items
            .iter()
            .filter(
                |item| item.get("kind").and_then(serde_json::Value::as_str) == Some("prompt_echo")
            )
            .count(),
        1,
        "{:?}",
        waited.payload
    );
    assert!(
        items.iter().any(|item| {
            item.get("kind").and_then(serde_json::Value::as_str) == Some("provider_output")
                && item
                    .get("summary")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|summary| summary.contains("worker output trace-visible"))
        }),
        "{:?}",
        waited.payload
    );
    assert_eq!(
        waited
            .payload
            .pointer("/supervision/matched")
            .and_then(serde_json::Value::as_bool),
        Some(true),
        "{:?}",
        waited.payload
    );
    assert!(
        waited
            .payload
            .pointer("/supervision/last_meaningful_output/summary")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|summary| summary.contains("worker output trace-visible")),
        "{:?}",
        waited.payload
    );
}

#[tokio::test]
async fn metaagent_wait_trace_wakes_when_worker_output_arrives_after_wait_starts() {
    let env = TestMetaRuntimeEnv::new("trace-wait-notify");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, worker) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    mark_test_agent_controlled_by_metaagent(&mut app, worker.id(), metaagent.id());
    let worker_run = launch_test_provider(
        &mut app,
        session.id(),
        worker.id(),
        "dev-stub",
        "dev-stub",
        "worker-model",
    );
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let subscribed = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_SUBSCRIBE_TRACE_TOOL,
            serde_json::json!({ "agent_ref": worker.id() }),
        )
        .await
        .expect("subscribe_trace should dispatch");
    assert!(subscribed.ok, "{:?}", subscribed.payload);
    let subscription_id = subscribed
        .payload
        .pointer("/subscription/subscription_id")
        .and_then(serde_json::Value::as_str)
        .expect("subscribe_trace should return subscription id")
        .to_string();

    let wait_runtime = router.runtime_state.clone();
    let wait_auth_token = meta_auth_token.clone();
    let wait_subscription_id = subscription_id.clone();
    let wait_task = tokio::spawn(async move {
        wait_runtime
            .dispatch_authenticated_runtime_tool_call(
                &wait_auth_token,
                crate::transport::runtime_tools::META_WAIT_TRACE_TOOL,
                serde_json::json!({
                    "subscription_id": wait_subscription_id,
                    "until": "worker_output",
                    "wait_ms": 100,
                    "limit": 10
                }),
            )
            .await
    });

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    {
        let mut app = app.lock().await;
        app.fan_out_output(
            session.id(),
            worker_run.id(),
            crate::terminal::TerminalOutputKind::ProviderOutput,
            None,
            Vec::new(),
            b"worker output after wait started",
        );
    }

    let waited = tokio::time::timeout(std::time::Duration::from_millis(150), wait_task)
        .await
        .expect("wait_trace should wake promptly from terminal fanout")
        .expect("wait task should join")
        .expect("wait_trace should dispatch");
    assert!(waited.ok, "{:?}", waited.payload);
    assert_eq!(
        waited
            .payload
            .get("matched")
            .and_then(serde_json::Value::as_bool),
        Some(true),
        "{:?}",
        waited.payload
    );
    assert!(
        waited
            .payload
            .get("items")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|items| items.iter().any(|item| {
                item.get("kind").and_then(serde_json::Value::as_str) == Some("provider_output")
                    && item
                        .get("summary")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|summary| summary.contains("after wait started"))
            })),
        "{:?}",
        waited.payload
    );
    assert!(
        waited
            .payload
            .pointer("/supervision/suggested_next_action")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|action| action.contains("complete") || action.contains("supervision")),
        "{:?}",
        waited.payload
    );
}

#[tokio::test]
async fn remote_runtime_projection_records_metaagent_turn_completion_event() {
    let env = TestMetaRuntimeEnv::new("remote-projection-completion-event");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, worker) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("attachment should attach");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    let submitted = app
        .submit_prompt(
            session.id(),
            attachment.id(),
            Some(worker.id()),
            "remote worker prompt",
            Vec::new(),
        )
        .expect("worker prompt should submit");
    assert!(matches!(
        submitted,
        crate::session::PromptSubmissionOutcome::Started { .. }
    ));
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    router
        .runtime_state
        .project_relay_remote_runtime_projection(
            session.id(),
            worker.id(),
            "remote:worker:provider-run-1",
            None,
            Vec::new(),
            vec![crate::transport::relay_peer::RelayProjectedOutputChunk {
                kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                merge_key: Some("assistant-1".to_string()),
                bytes: b"remote output".to_vec(),
            }],
            Vec::new(),
            vec![crate::transport::relay_peer::RelayProjectedCompletion {
                message_id: "assistant-msg-1".to_string(),
                completed_at_ms: 1234,
            }],
        )
        .await
        .expect("runtime projection should succeed");

    let events = app.lock().await.metaagent_event_store().list(
        metaagent.id(),
        Some("agent.turn.completed"),
        None,
        10,
    );
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].source_agent_id.as_deref(), Some(worker.id()));
    assert_eq!(
        events[0]
            .detail
            .get("completed_agent_id")
            .and_then(serde_json::Value::as_str),
        Some(worker.id())
    );
}
