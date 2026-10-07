use super::*;

#[tokio::test]
async fn metaagent_event_prompts_retry_after_provider_launch() {
    let env = TestMetaRuntimeEnv::new("event-retry-after-launch");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("worker"))
        .expect("worker should spawn");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    router
        .runtime_state
        .inject_metaagent_agent_lifecycle_event_for_agent(session.id(), &worker, "agent.spawned")
        .await
        .expect("metaagent event should record even before provider launch");
    let failed_event = app
        .lock()
        .await
        .metaagent_event_store()
        .list(metaagent.id(), Some("agent.spawned"), Some("failed"), 10)
        .into_iter()
        .next()
        .expect("event prompt should fail while no metaagent provider route exists");
    let failed_prompt_id = failed_event.injected_prompt_id.clone();

    let started = app
        .lock()
        .await
        .start_provider_launch(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "meta-model",
            )
            .with_agent_id(metaagent.id()),
        )
        .expect("metaagent provider launch should start");
    router
        .runtime_state
        .finish_provider_launch(&started, None)
        .await;

    let retried_event = app
        .lock()
        .await
        .metaagent_event_store()
        .read(metaagent.id(), &failed_event.event_id)
        .expect("event should still be readable after retry");
    assert_ne!(
        retried_event.prompt_delivery_status,
        crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Failed,
        "provider launch should retry failed metaagent event prompts: {retried_event:?}"
    );
    assert_ne!(
        retried_event.injected_prompt_id, failed_prompt_id,
        "retry should use a fresh prompt id for the replayed visible event prompt"
    );
    assert!(
        matches!(
            retried_event.prompt_delivery_status.as_str(),
            "submitted" | "queued" | "delivered"
        ),
        "retried event should be re-admitted through normal prompt delivery: {retried_event:?}"
    );
}

#[test]
fn metaagent_turn_overview_and_blob_are_scoped_to_owned_regular_agents() {
    run_large_stack_async_test(
        "metaagent-turn-overview-and-blob-are-scoped",
        metaagent_turn_overview_and_blob_are_scoped_to_owned_regular_agents_inner,
    );
}

async fn metaagent_turn_overview_and_blob_are_scoped_to_owned_regular_agents_inner() {
    let env = TestMetaRuntimeEnv::new("turn-trace");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("worker"))
        .expect("worker should spawn");
    let peer_worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("peer-worker")
                .with_owner_user_id("user-2"),
        )
        .expect("peer worker should spawn");
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
    let peer_worker_run = launch_test_provider(
        &mut app,
        session.id(),
        peer_worker.id(),
        "dev-stub",
        "dev-stub",
        "peer-worker-model",
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
    let attach = attach_request(session.id(), "client-trace");
    let attachment_id = match router
        .dispatch(
            KernelCommand::from_local_request("attach-trace", None, None, &attach),
            attach,
        )
        .await
        .expect("client should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment.id().to_string(),
        other => panic!("unexpected attach response: {other:?}"),
    };
    let submit = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session.id().to_string(),
        attachment_id: attachment_id.clone(),
        target_agent_id: Some(worker.id().to_string()),
        prompt: "summarize the trace".to_string(),
        attachments: Vec::new(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("submit-trace", None, None, &submit),
            submit,
        )
        .await
        .expect("worker prompt should submit");
    let tool_entry = crate::history::SessionHistoryEntry::provider_output(
        session.id(),
        worker_run.id(),
        Some(worker.id()),
        crate::terminal::TerminalOutputKind::ProviderTool,
        None,
        serde_json::json!({
            "tool": "shell",
            "status": "completed",
            "input": {"command": "cargo test"}
        })
        .to_string(),
    );
    router
        .operational_history_store
        .append_transcript(
            &tool_entry,
            crate::history::HistoryEventTurnContext {
                session_id: Some(session.id().to_string()),
                agent_id: Some(worker.id().to_string()),
                provider: Some(worker_run.provider().to_string()),
                model: Some(worker_run.model().to_string()),
                provider_run_id: Some(worker_run.id().to_string()),
                turn_id: Some("trace-turn".to_string()),
                ..crate::history::HistoryEventTurnContext::default()
            },
        )
        .expect("provider tool output should append to operational history");
    let peer_prompt_entry = crate::history::SessionHistoryEntry::user_prompt(
        session.id(),
        "peer-attachment",
        peer_worker.id(),
        "peer private prompt",
    );
    router
        .operational_history_store
        .append_transcript(
            &peer_prompt_entry,
            crate::history::HistoryEventTurnContext {
                session_id: Some(session.id().to_string()),
                agent_id: Some(peer_worker.id().to_string()),
                provider: Some(peer_worker_run.provider().to_string()),
                model: Some(peer_worker_run.model().to_string()),
                provider_run_id: Some(peer_worker_run.id().to_string()),
                turn_id: Some("peer-trace-turn".to_string()),
                ..crate::history::HistoryEventTurnContext::default()
            },
        )
        .expect("peer prompt should append to operational history");
    let peer_tool_entry = crate::history::SessionHistoryEntry::provider_output(
        session.id(),
        peer_worker_run.id(),
        Some(peer_worker.id()),
        crate::terminal::TerminalOutputKind::ProviderTool,
        None,
        serde_json::json!({
            "tool": "shell",
            "status": "completed",
            "input": {"command": "cat secret-peer-file"}
        })
        .to_string(),
    );
    router
        .operational_history_store
        .append_transcript(
            &peer_tool_entry,
            crate::history::HistoryEventTurnContext {
                session_id: Some(session.id().to_string()),
                agent_id: Some(peer_worker.id().to_string()),
                provider: Some(peer_worker_run.provider().to_string()),
                model: Some(peer_worker_run.model().to_string()),
                provider_run_id: Some(peer_worker_run.id().to_string()),
                turn_id: Some("peer-trace-turn".to_string()),
                ..crate::history::HistoryEventTurnContext::default()
            },
        )
        .expect("peer provider tool output should append to operational history");

    let overview = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_TURN_OVERVIEW_TOOL,
            serde_json::json!({ "agent_ref": "worker" }),
        )
        .await
        .expect("turn overview should dispatch");
    assert!(overview.ok, "{:?}", overview.payload);
    let blob_id = overview
        .payload
        .pointer("/turns/0/items")
        .and_then(serde_json::Value::as_array)
        .and_then(|items| items.iter().find_map(|item| item.get("blob_id")))
        .and_then(serde_json::Value::as_str)
        .expect("overview should include provider tool blob id")
        .to_string();

    let blob = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_TURN_BLOB_TOOL,
            serde_json::json!({ "blob_id": blob_id }),
        )
        .await
        .expect("turn blob should dispatch");
    assert!(blob.ok, "{:?}", blob.payload);
    assert_eq!(
        blob.payload
            .pointer("/agent/id")
            .and_then(serde_json::Value::as_str),
        Some(worker.id())
    );
    assert!(
        blob.payload
            .pointer("/entries/0/entry/text")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|text| text.contains("cargo test")),
        "{:?}",
        blob.payload
    );

    let denied = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_TURN_OVERVIEW_TOOL,
            serde_json::json!({ "agent_ref": "meta" }),
        )
        .await
        .expect("turn overview denial should dispatch");
    assert!(!denied.ok);
    assert!(
        denied
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("not an owned regular agent")),
        "{:?}",
        denied.payload
    );

    let peer_overview_denied = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_TURN_OVERVIEW_TOOL,
            serde_json::json!({ "agent_ref": "peer-worker" }),
        )
        .await
        .expect("peer turn overview denial should dispatch");
    assert!(!peer_overview_denied.ok);
    assert!(
        peer_overview_denied
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("not an owned regular agent")),
        "{:?}",
        peer_overview_denied.payload
    );

    let peer_history = crate::runtime::history_requests::execute_session_history_outline_request(
        router.operational_history_store.clone(),
        crate::local::GetSessionHistoryOutlineRequest {
            session_id: session.id().to_string(),
            agent_ids: Some(vec![peer_worker.id().to_string()]),
            latest_prompt_count: Some(1),
            cursor: None,
        },
    )
    .await
    .expect("peer history outline should load");
    let crate::local::LocalDaemonResponse::SessionHistoryOutline { agents } = peer_history else {
        panic!("unexpected peer history response");
    };
    let peer_blob_id = agents
        .first()
        .and_then(|agent| agent.turns.first())
        .and_then(|turn| turn.blobs.first())
        .map(|blob| blob.blob_id.clone())
        .expect("peer history should include a blob");
    let peer_blob_denied = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_TURN_BLOB_TOOL,
            serde_json::json!({ "blob_id": peer_blob_id }),
        )
        .await
        .expect("peer blob denial should dispatch");
    assert!(!peer_blob_denied.ok);
    assert!(
        peer_blob_denied
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("owned regular agent")),
        "{:?}",
        peer_blob_denied.payload
    );
}

#[tokio::test]
async fn metaagent_event_subscriptions_persist_and_can_be_removed() {
    let env = TestMetaRuntimeEnv::new("event-subscriptions");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
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
            crate::transport::runtime_tools::META_SUBSCRIBE_EVENTS_TOOL,
            serde_json::json!({ "kind": "workflow.output.final" }),
        )
        .await
        .expect("subscribe should dispatch");
    assert!(subscribed.ok);
    let subscription_id = subscribed
        .payload
        .pointer("/subscription/subscription_id")
        .and_then(serde_json::Value::as_str)
        .expect("subscription id should be returned")
        .to_string();

    let listed = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_SUBSCRIPTIONS_TOOL,
            serde_json::json!({}),
        )
        .await
        .expect("list subscriptions should dispatch");
    assert!(listed.ok);
    let subscriptions = listed
        .payload
        .get("subscriptions")
        .and_then(serde_json::Value::as_array)
        .expect("subscriptions should be listed");
    let valid_event_kinds = listed
        .payload
        .get("valid_event_kinds")
        .and_then(serde_json::Value::as_array)
        .expect("valid event kinds should be listed");
    assert!(valid_event_kinds.iter().any(|kind| {
        kind.as_str()
            == Some(crate::transport::runtime_tools::META_EVENT_KIND_WORKFLOW_OUTPUT_FINAL)
    }));
    assert!(subscriptions.iter().any(|subscription| {
        subscription.get("kind").and_then(serde_json::Value::as_str) == Some("agent.turn.completed")
            && subscription
                .get("required")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
    }));
    assert!(subscriptions.iter().any(|subscription| {
        subscription
            .get("subscription_id")
            .and_then(serde_json::Value::as_str)
            == Some(subscription_id.as_str())
            && subscription.get("kind").and_then(serde_json::Value::as_str)
                == Some("workflow.output.final")
    }));

    let removed = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_UNSUBSCRIBE_EVENTS_TOOL,
            serde_json::json!({ "subscription_id": subscription_id }),
        )
        .await
        .expect("unsubscribe should dispatch");
    assert!(removed.ok);
    assert_eq!(
        removed
            .payload
            .get("status")
            .and_then(serde_json::Value::as_str),
        Some("removed")
    );
}

#[tokio::test]
async fn metaagent_event_subscription_rejects_unknown_kinds_with_suggestions() {
    let env = TestMetaRuntimeEnv::new("event-subscription-validation");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
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

    let rejected = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_SUBSCRIBE_EVENTS_TOOL,
            serde_json::json!({ "kind": "workflow_output" }),
        )
        .await
        .expect("subscribe should dispatch");

    assert!(!rejected.ok);
    assert_eq!(
        rejected
            .payload
            .get("kind")
            .and_then(serde_json::Value::as_str),
        Some("workflow_output")
    );
    let suggestions = rejected
        .payload
        .get("suggestions")
        .and_then(serde_json::Value::as_array)
        .expect("suggestions should be returned");
    assert!(suggestions.iter().any(|suggestion| {
        suggestion.as_str()
            == Some(crate::transport::runtime_tools::META_EVENT_KIND_WORKFLOW_OUTPUT_FINAL)
    }));
    let valid_event_kinds = rejected
        .payload
        .get("valid_event_kinds")
        .and_then(serde_json::Value::as_array)
        .expect("valid event kinds should be returned");
    assert!(valid_event_kinds.iter().any(|kind| {
        kind.as_str() == Some(crate::transport::runtime_tools::META_EVENT_KIND_AGENT_TURN_COMPLETED)
    }));
}

#[test]
fn subscribed_collaborator_workflow_output_records_and_injects_metaagent_event() {
    run_large_stack_async_test(
        "subscribed-collaborator-workflow-output",
        subscribed_collaborator_workflow_output_records_and_injects_metaagent_event_inner,
    );
}

async fn subscribed_collaborator_workflow_output_records_and_injects_metaagent_event_inner() {
    let env = TestMetaRuntimeEnv::new("workflow-output-event");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("worker")
                .with_owner_user_id("user-2"),
        )
        .expect("worker should spawn");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
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
    let worker_auth_token = worker_run
        .runtime_mcp_auth_token()
        .expect("worker run should expose runtime MCP auth token")
        .to_string();
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_SUBSCRIBE_EVENTS_TOOL,
            serde_json::json!({ "kind": "workflow.output.final" }),
        )
        .await
        .expect("metaagent should subscribe to workflow final outputs");

    let create_workflow = LocalDaemonRequest::CreateWorkflow(crate::local::CreateWorkflowRequest {
        session_id: session.id().to_string(),
        alias: Some("review".to_string()),
    });
    let workflow = match router
        .dispatch(
            KernelCommand::from_local_request("create-workflow", None, None, &create_workflow),
            create_workflow,
        )
        .await
        .expect("workflow should be created")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        other => panic!("unexpected workflow create response: {other:?}"),
    };
    {
        let app = app.lock().await;
        app.sessions_mut()
            .set_workflow_flush_agent_context_before_run(session.id(), workflow.id(), false)
            .expect("event fixture should preserve the authenticated worker run");
    }
    let add_node = LocalDaemonRequest::AddWorkflowNode(crate::local::AddWorkflowNodeRequest {
        session_id: session.id().to_string(),
        workflow_ref: workflow.id().to_string(),
        agent_id: worker.id().to_string(),
        expected_workflow_revision: None,
    });
    let node = match router
        .dispatch(
            KernelCommand::from_local_request("add-workflow-node", None, None, &add_node),
            add_node,
        )
        .await
        .expect("workflow node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        other => panic!("unexpected workflow node response: {other:?}"),
    };
    let set_completion = LocalDaemonRequest::SetWorkflowNodeCanCompleteRun(
        crate::local::SetWorkflowNodeCanCompleteRunRequest {
            session_id: session.id().to_string(),
            workflow_ref: workflow.id().to_string(),
            node_id: node.id().to_string(),
            can_complete_workflow_run: true,
            expected_workflow_revision: None,
        },
    );
    router
        .dispatch(
            KernelCommand::from_local_request("set-workflow-complete", None, None, &set_completion),
            set_completion,
        )
        .await
        .expect("workflow node should be allowed to complete run");
    let create_endpoint =
        LocalDaemonRequest::CreateWorkflowEndpoint(crate::local::CreateWorkflowEndpointRequest {
            session_id: session.id().to_string(),
            workflow_ref: workflow.id().to_string(),
            entry_node_id: node.id().to_string(),
            alias: Some("entry".to_string()),
            expected_workflow_revision: None,
        });
    let endpoint = match router
        .dispatch(
            KernelCommand::from_local_request(
                "create-workflow-endpoint",
                None,
                None,
                &create_endpoint,
            ),
            create_endpoint,
        )
        .await
        .expect("workflow endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        other => panic!("unexpected workflow endpoint response: {other:?}"),
    };
    let invoke =
        LocalDaemonRequest::InvokeWorkflowEndpoint(crate::local::InvokeWorkflowEndpointRequest {
            session_id: session.id().to_string(),
            workflow_ref: workflow.id().to_string(),
            endpoint_ref: endpoint.id().to_string(),
            prompt: Some("produce final output".to_string()),
            queue_ref: None,
            publication_invocation: None,
        });
    let workflow_run = match router
        .dispatch(
            KernelCommand::from_local_request("invoke-workflow", None, None, &invoke),
            invoke,
        )
        .await
        .expect("workflow should be invoked")
    {
        LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
        other => panic!("unexpected workflow invoke response: {other:?}"),
    };

    let submitted = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &worker_auth_token,
            crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL,
            serde_json::json!({
                "workflow_output_json": "{\"summary\":\"done\"}"
            }),
        )
        .await
        .expect("workflow final output tool should dispatch");
    assert!(submitted.ok, "{:?}", submitted.payload);
    assert_eq!(
        submitted
            .payload
            .get("workflow_run_id")
            .and_then(serde_json::Value::as_str),
        Some(workflow_run.id())
    );

    let listed = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
            serde_json::json!({ "kind": "workflow.output.final" }),
        )
        .await
        .expect("workflow final output event should list");
    assert!(listed.ok);
    let event = listed
        .payload
        .get("events")
        .and_then(serde_json::Value::as_array)
        .and_then(|events| events.first())
        .expect("workflow output event should be recorded");
    assert_eq!(
        event
            .get("source_agent_id")
            .and_then(serde_json::Value::as_str),
        Some(worker.id())
    );
    assert!(
        event
            .get("injected_prompt_id")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        "workflow output event should record prompt injection id"
    );
}
