use super::*;

#[test]
fn user_agent_lifecycle_events_notify_metaagent_but_meta_commands_do_not() {
    run_large_stack_async_test(
        "user-agent-lifecycle-events",
        user_agent_lifecycle_events_notify_metaagent_but_meta_commands_do_not_inner,
    );
}

async fn user_agent_lifecycle_events_notify_metaagent_but_meta_commands_do_not_inner() {
    let env = TestMetaRuntimeEnv::new("agent-lifecycle-events");
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

    let human_spawn = LocalDaemonRequest::SpawnAgent(crate::local::SpawnAgentRequest {
        session_id: session.id().to_string(),
        alias: Some("human-worker".to_string()),
        provider: Some("dev-stub".to_string()),
        account_profile: None,
        model: Some("default".to_string()),
        effort: None,
        execution_mode: None,
        permission_level: None,
        worktree_id: Some(workspace.to_string_lossy().to_string()),
        kernel_ref: None,
        slice_ref: None,
        worktree_placement: None,
        metaagent: false,
    });
    let spawned = router
        .dispatch(
            KernelCommand::from_local_request("human-spawn-worker", None, None, &human_spawn),
            human_spawn,
        )
        .await
        .expect("human spawn should dispatch");
    let LocalDaemonResponse::AgentSpawned {
        agent: human_worker,
    } = spawned
    else {
        panic!("unexpected human spawn response");
    };

    let events = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
            serde_json::json!({ "kind": "agent.spawned" }),
        )
        .await
        .expect("metaagent should list lifecycle events");
    assert!(events.ok, "{events:?}");
    assert_eq!(
        events
            .payload
            .get("events")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(1),
        "{:?}",
        events.payload
    );

    let meta_spawn = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "agent spawn quiet-worker" }),
        )
        .await
        .expect("metaagent spawn command should dispatch");
    assert!(meta_spawn.ok, "{meta_spawn:?}");
    let events_after_meta_spawn = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
            serde_json::json!({ "kind": "agent.spawned" }),
        )
        .await
        .expect("metaagent should list lifecycle events");
    assert_eq!(
        events_after_meta_spawn
            .payload
            .get("events")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(1),
        "{:?}",
        events_after_meta_spawn.payload
    );

    let human_delete = LocalDaemonRequest::DestroyAgent(crate::local::DestroyAgentRequest {
        session_id: session.id().to_string(),
        agent_id: human_worker.id().to_string(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("human-delete-worker", None, None, &human_delete),
            human_delete,
        )
        .await
        .expect("human delete should dispatch");
    let deleted_events = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
            serde_json::json!({ "kind": "agent.deleted" }),
        )
        .await
        .expect("metaagent should list delete lifecycle events");
    assert_eq!(
        deleted_events
            .payload
            .get("events")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(1),
        "{:?}",
        deleted_events.payload
    );
}

#[tokio::test]
async fn forged_metaagent_caller_id_does_not_suppress_lifecycle_events() {
    let env = TestMetaRuntimeEnv::new("forged-metaagent-caller");
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
    let metaagent_id = metaagent.id().to_string();
    let session_id = session.id().to_string();
    let worktree_id = workspace.to_string_lossy().to_string();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);

    let request = LocalDaemonRequest::SpawnAgent(crate::local::SpawnAgentRequest {
        account_profile: None,
        session_id,
        alias: Some("forged-worker".to_string()),
        provider: Some("dev-stub".to_string()),
        model: Some("default".to_string()),
        effort: None,
        execution_mode: None,
        permission_level: None,
        worktree_id: Some(worktree_id),
        kernel_ref: None,
        slice_ref: None,
        worktree_placement: None,
        metaagent: false,
    });
    let mut command =
        KernelCommand::from_local_request("forged-metaagent-spawn-worker", None, None, &request);
    command.caller.caller_id = format!("metaagent:{metaagent_id}");
    router
        .dispatch(command, request)
        .await
        .expect("forged caller id should dispatch as a normal user command");

    let events = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
            serde_json::json!({ "kind": "agent.spawned" }),
        )
        .await
        .expect("metaagent should list lifecycle events");
    assert_eq!(
        events
            .payload
            .get("events")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(1),
        "{:?}",
        events.payload
    );
}

#[test]
fn metaagent_run_command_returns_structured_denials_for_forbidden_commands() {
    run_large_stack_async_test(
        "metaagent-run-command-denials",
        metaagent_run_command_returns_structured_denials_for_forbidden_commands_inner,
    );
}

async fn metaagent_run_command_returns_structured_denials_for_forbidden_commands_inner() {
    let env = TestMetaRuntimeEnv::new("run-command-deny");
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
    let prompt_flag_worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("prompt-flag-worker")
                .with_owner_user_id(metaagent.owner_user_id()),
        )
        .expect("prompt flag worker should spawn");
    mark_test_agent_controlled_by_metaagent(&mut app, prompt_flag_worker.id(), metaagent.id());
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

    let denied = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "session new"
            }),
        )
        .await
        .expect("meta run_command denials should be structured tool results");

    assert!(!denied.ok);
    assert!(
        denied
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("cannot create")),
        "{:?}",
        denied.payload
    );

    let docs = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_COMMAND_DOCS_TOOL,
            serde_json::json!({
                "command": "mcp list"
            }),
        )
        .await
        .expect("meta command docs should dispatch");
    assert!(docs.ok);
    assert_eq!(
        docs.payload
            .get("metaagent_policy")
            .and_then(serde_json::Value::as_str),
        Some("allow")
    );
    assert_eq!(
        docs.payload
            .get("routed")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );

    for command in ["mcp list", "skill list", "credential list", "slice list"] {
        let routed = router
            .dispatch_authenticated_runtime_tool_call(
                &meta_auth_token,
                crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
                serde_json::json!({
                    "command": command
                }),
            )
            .await
            .expect("safe registered commands should dispatch");
        assert!(routed.ok, "{command}: {:?}", routed.payload);
    }

    let not_routed = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "mcp install test --command node"
            }),
        )
        .await
        .expect("registry-backed not-routed commands should return structured tool results");
    assert!(!not_routed.ok);
    assert!(
        not_routed
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("mcp install-json")),
        "{:?}",
        not_routed.payload
    );

    let prompt_flag_denied = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": format!("prompt {} --wait inspect this", prompt_flag_worker.id())
            }),
        )
        .await
        .expect("prompt flag denial should return a structured tool result");
    assert!(!prompt_flag_denied.ok);
    assert!(
        prompt_flag_denied
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("does not support blocking reply flags")),
        "{:?}",
        prompt_flag_denied.payload
    );

    let audit_events = app
        .lock()
        .await
        .durable_state_store()
        .load_events_after(0)
        .expect("durable audit events should load");
    let denied_audit = audit_events
        .iter()
        .find(|event| {
            event.kind == "metaagent.command.executed"
                && event.payload["metaagent_id"] == metaagent.id()
                && event.payload["command"] == "session new"
                && event.payload["status"] == "denied"
        })
        .expect("denied metaagent commands should be audited");
    assert_eq!(denied_audit.payload["provider_run_id"], meta_run.id());
    assert_eq!(denied_audit.payload["causation_id"], meta_run.id());
    let denied_correlation_id = denied_audit
        .payload
        .get("correlation_id")
        .and_then(serde_json::Value::as_str)
        .expect("denied command audit should include a correlation id");
    assert!(
        denied_correlation_id.starts_with(&format!("metaagent:{}:command:", metaagent.id())),
        "{denied_correlation_id}"
    );
}

#[test]
fn regular_agent_turn_completion_injects_metaagent_event_and_inbox_entry() {
    run_large_stack_async_test(
        "regular-agent-turn-completion-meta-event",
        regular_agent_turn_completion_injects_metaagent_event_and_inbox_entry_inner,
    );
}

async fn regular_agent_turn_completion_injects_metaagent_event_and_inbox_entry_inner() {
    let env = TestMetaRuntimeEnv::new("turn-event");
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
    mark_test_agent_controlled_by_metaagent(&mut app, worker.id(), metaagent.id());
    let _worker_run = launch_test_provider(
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
    let attach = attach_request(session.id(), "client-1");
    let attachment_id = match router
        .dispatch(
            KernelCommand::from_local_request("attach", None, None, &attach),
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
        attachment_id,
        target_agent_id: Some(worker.id().to_string()),
        prompt: "finish this test turn".to_string(),
        attachments: Vec::new(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("submit", None, None, &submit),
            submit,
        )
        .await
        .expect("worker prompt should submit");
    let complete = LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
        session_id: session.id().to_string(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("complete", None, None, &complete),
            complete,
        )
        .await
        .expect("worker prompt should complete and notify metaagent");

    let listed = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
            serde_json::json!({ "kind": "agent.turn.completed" }),
        )
        .await
        .expect("meta list_events should dispatch");
    assert!(listed.ok);
    let event = listed
        .payload
        .get("events")
        .and_then(serde_json::Value::as_array)
        .and_then(|events| events.first())
        .expect("turn completion event should be listed");
    let event_id = event
        .get("event_id")
        .and_then(serde_json::Value::as_str)
        .expect("event should have id")
        .to_string();
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
        "event should record injected prompt id"
    );
    assert!(
        event
            .get("prompt_delivery_status")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|status| matches!(status, "submitted" | "delivered")),
        "event should expose visible prompt delivery status: {event:?}"
    );

    let read = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_READ_EVENT_TOOL,
            serde_json::json!({ "event_id": event_id }),
        )
        .await
        .expect("meta read_event should dispatch");
    assert!(read.ok);
    assert_eq!(
        read.payload
            .pointer("/event/read_at_ms")
            .and_then(serde_json::Value::as_u64)
            .is_some(),
        true
    );
    assert!(
        read.payload
            .pointer("/event/prompt_delivery_status")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        "{:?}",
        read.payload
    );
    let acked = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_ACK_EVENT_TOOL,
            serde_json::json!({ "event_id": event_id }),
        )
        .await
        .expect("meta ack_event should dispatch");
    assert!(acked.ok);
    assert_eq!(
        acked
            .payload
            .get("acked")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(1)
    );
}

#[test]
fn idle_metaagent_turn_with_active_task_injects_orphaned_task_event() {
    run_large_stack_async_test(
        "metaagent-orphaned-task-event",
        idle_metaagent_turn_with_active_task_injects_orphaned_task_event_inner,
    );
}

async fn idle_metaagent_turn_with_active_task_injects_orphaned_task_event_inner() {
    let env = TestMetaRuntimeEnv::new("metaagent-orphaned-task-event");
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
    let attach = attach_request(session.id(), "client-meta-orphaned-task");
    let attachment_id = match router
        .dispatch(
            KernelCommand::from_local_request("attach", None, None, &attach),
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
        attachment_id,
        target_agent_id: Some(metaagent.id().to_string()),
        prompt: "Start a task, then stop without marking it complete.".to_string(),
        attachments: Vec::new(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("submit-meta", None, None, &submit),
            submit,
        )
        .await
        .expect("metaagent prompt should submit");
    let complete = LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
        session_id: session.id().to_string(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("complete-meta", None, None, &complete),
            complete,
        )
        .await
        .expect("metaagent completion should inject orphan recovery prompt");

    let listed = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
            serde_json::json!({ "kind": "metaagent.task.orphaned" }),
        )
        .await
        .expect("meta list_events should dispatch");
    assert!(listed.ok);
    let event = listed
        .payload
        .get("events")
        .and_then(serde_json::Value::as_array)
        .and_then(|events| events.first())
        .expect("orphaned task event should be listed");
    assert_eq!(
        event.get("kind").and_then(serde_json::Value::as_str),
        Some("metaagent.task.orphaned")
    );
    assert!(
        event
            .get("injected_prompt_id")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        "orphaned task event should inject a continuation prompt: {event:?}"
    );
    assert!(
        event
            .get("prompt_delivery_status")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|status| matches!(status, "submitted" | "delivered")),
        "orphaned task event should be delivered to the metaagent: {event:?}"
    );
    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session.id().to_string(),
    });
    let session_after = router
        .dispatch(
            KernelCommand::from_local_request("session-get", None, None, &state_request),
            state_request,
        )
        .await
        .expect("session should load");
    let LocalDaemonResponse::SessionState { session, .. } = session_after else {
        panic!("unexpected session response: {session_after:?}");
    };
    assert_eq!(
        session
            .metaagent_task(metaagent.id())
            .map(|task| task.status()),
        Some(crate::session::MetaagentTaskStatus::Active)
    );
    assert!(
        session.active_prompt_for_agent(metaagent.id()).is_some(),
        "orphan recovery prompt should leave the metaagent active"
    );

    let complete_again = LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
        session_id: session.id().to_string(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("complete-meta-again", None, None, &complete_again),
            complete_again,
        )
        .await
        .expect("second idle metaagent completion should not inject duplicate orphan recovery");
    let listed_again = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
            serde_json::json!({ "kind": "metaagent.task.orphaned" }),
        )
        .await
        .expect("meta list_events should dispatch");
    assert!(listed_again.ok);
    let duplicate_count = listed_again
        .payload
        .get("events")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    assert_eq!(
        duplicate_count, 1,
        "same task revision should receive at most one orphan recovery event"
    );
}

#[test]
fn metaagent_turn_with_active_worker_does_not_inject_orphaned_task_event() {
    run_large_stack_async_test(
        "metaagent-active-worker-no-orphaned-task-event",
        metaagent_turn_with_active_worker_does_not_inject_orphaned_task_event_inner,
    );
}

async fn metaagent_turn_with_active_worker_does_not_inject_orphaned_task_event_inner() {
    let env = TestMetaRuntimeEnv::new("metaagent-active-worker-no-orphaned");
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
    mark_test_agent_controlled_by_metaagent(&mut app, worker.id(), metaagent.id());
    let _worker_run = launch_test_provider(
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
    let attach = attach_request(session.id(), "client-meta-active-worker");
    let attachment_id = match router
        .dispatch(
            KernelCommand::from_local_request("attach", None, None, &attach),
            attach,
        )
        .await
        .expect("client should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment.id().to_string(),
        other => panic!("unexpected attach response: {other:?}"),
    };
    let submit_worker = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session.id().to_string(),
        attachment_id: attachment_id.clone(),
        target_agent_id: Some(worker.id().to_string()),
        prompt: "keep working".to_string(),
        attachments: Vec::new(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("submit-worker", None, None, &submit_worker),
            submit_worker,
        )
        .await
        .expect("worker prompt should submit");
    let submit_meta = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session.id().to_string(),
        attachment_id,
        target_agent_id: Some(metaagent.id().to_string()),
        prompt: "Start a task while the worker is still active.".to_string(),
        attachments: Vec::new(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("submit-meta", None, None, &submit_meta),
            submit_meta,
        )
        .await
        .expect("metaagent prompt should submit");
    let complete_meta = LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
        session_id: session.id().to_string(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("complete-meta", None, None, &complete_meta),
            complete_meta,
        )
        .await
        .expect("metaagent completion should not inject orphan recovery while worker is active");

    let listed = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
            serde_json::json!({ "kind": "metaagent.task.orphaned" }),
        )
        .await
        .expect("meta list_events should dispatch");
    assert!(listed.ok);
    assert_eq!(
        listed
            .payload
            .get("events")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(0),
        "active worker should suppress orphan recovery: {:?}",
        listed.payload
    );
}
