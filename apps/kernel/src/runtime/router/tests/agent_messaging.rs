use super::*;

fn run_agent_message_test_with_large_stack<F, Fut>(name: &'static str, test: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(crate::runtime_transport::KERNEL_RUNTIME_THREAD_STACK_SIZE)
                .enable_all()
                .build()
                .expect("agent message test runtime should build")
                .block_on(test());
        })
        .expect("agent message test thread should spawn")
        .join()
        .unwrap_or_else(|error| std::panic::resume_unwind(error));
}

#[test]
fn runtime_mcp_agents_can_message_and_steer_each_other_by_unique_alias() {
    run_agent_message_test_with_large_stack("agent-message-alias", || {
        runtime_mcp_agents_can_message_and_steer_each_other_by_unique_alias_inner()
    });
}

async fn runtime_mcp_agents_can_message_and_steer_each_other_by_unique_alias_inner() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, sender) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-agent-messaging",
            "worktree-agent-messaging",
        ))
        .expect("session should be created");
    let sender = app
        .agents_mut()
        .alias_agent(sender.id(), Some("router".to_string()))
        .expect("sender alias should update");
    let reviewer = spawn_test_agent(&mut app, session.id(), "reviewer", "dev-stub");
    let sender_run = launch_test_provider(
        &mut app,
        session.id(),
        sender.id(),
        "dev-stub",
        "dev-stub",
        "sender-model",
    );
    let reviewer_run = launch_test_provider(
        &mut app,
        session.id(),
        reviewer.id(),
        "dev-stub",
        "dev-stub",
        "reviewer-model",
    );
    let sender_token = sender_run
        .runtime_mcp_auth_token()
        .expect("sender run should expose runtime MCP auth")
        .to_string();
    let reviewer_token = reviewer_run
        .runtime_mcp_auth_token()
        .expect("reviewer run should expose runtime MCP auth")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let specs = router
        .runtime_state
        .runtime_tool_specs_for_auth_token(&sender_token);
    assert!(
        specs
            .iter()
            .any(|spec| { spec.name == crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL }),
        "regular agents should receive the agent messaging tool"
    );
    assert!(specs
        .iter()
        .any(|spec| { spec.name == crate::transport::runtime_tools::LIST_SESSION_AGENTS_TOOL }));
    assert!(specs
        .iter()
        .any(|spec| { spec.name == crate::transport::runtime_tools::GET_SESSION_AGENT_TOOL }));

    let listed = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &sender_token,
            "mcp__chariox__list_session_agents",
            serde_json::json!({}),
        )
        .await
        .expect("session agent discovery should dispatch");
    assert!(listed.ok, "{:?}", listed.payload);
    assert_eq!(
        listed.payload["agents"]
            .as_array()
            .expect("agents should be an array")
            .len(),
        2
    );
    let listed_reviewer = listed.payload["agents"]
        .as_array()
        .expect("agents should be an array")
        .iter()
        .find(|agent| agent["alias"] == "reviewer")
        .expect("reviewer should be discoverable");
    assert_eq!(listed_reviewer["address"], "@reviewer");
    assert_eq!(listed_reviewer["provider"], "dev-stub");
    assert_eq!(listed_reviewer["model"], "reviewer-model");
    assert_eq!(listed_reviewer["location"]["kind"], "local");
    assert_eq!(listed_reviewer["is_self"], false);

    let first = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &sender_token,
            "mcp__chariox__send_agent_message",
            serde_json::json!({
                "agent": "@REVIEWER",
                "message": "Inspect package.json and report the package name.",
                "attachments": [{
                    "url": "data:image/png;base64,aGVsbG8=",
                    "mime": "image/png",
                    "filename": "diagram.png",
                    "contents_base64": "aGVsbG8="
                }],
                "idempotency_key": "review-package-name"
            }),
        )
        .await
        .expect("agent message should dispatch");
    assert!(first.ok, "{:?}", first.payload);
    assert_eq!(first.payload["status"], "started");
    assert_eq!(first.payload["target_agent_id"], reviewer.id());
    assert_eq!(first.payload["attachment_count"], 1);

    let retried = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &sender_token,
            "mcp__chariox__send_agent_message",
            serde_json::json!({
                "agent": "@REVIEWER",
                "message": "Inspect package.json and report the package name.",
                "attachments": [{
                    "url": "data:image/png;base64,aGVsbG8=",
                    "mime": "image/png",
                    "filename": "diagram.png",
                    "contents_base64": "aGVsbG8="
                }],
                "idempotency_key": "review-package-name"
            }),
        )
        .await
        .expect("idempotent agent message retry should replay");
    assert!(retried.ok, "{:?}", retried.payload);
    assert_eq!(retried.payload["prompt_id"], first.payload["prompt_id"]);

    let second = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &sender_token,
            crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL,
            serde_json::json!({
                "agent": reviewer.agent_ref(),
                "message": "Then report whether the package is private."
            }),
        )
        .await
        .expect("busy target message should steer into the active turn");
    assert!(second.ok, "{:?}", second.payload);
    assert_eq!(second.payload["status"], "steered");

    let inspected = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &sender_token,
            crate::transport::runtime_tools::GET_SESSION_AGENT_TOOL,
            serde_json::json!({ "agent": "@REVIEWER" }),
        )
        .await
        .expect("session agent inspection should dispatch");
    assert!(inspected.ok, "{:?}", inspected.payload);
    assert_eq!(inspected.payload["agent"]["id"], reviewer.id());
    assert_eq!(inspected.payload["agent"]["has_active_prompt"], true);
    assert_eq!(inspected.payload["agent"]["queued_prompt_count"], 0);
    assert_eq!(
        inspected.payload["agent"]["extensions"]["mcps"],
        serde_json::json!([])
    );
    assert!(inspected.payload["agent"]
        .get("provider_resume_state")
        .is_none());
    assert!(inspected.payload["agent"].get("relay_token").is_none());

    let reply = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &reviewer_token,
            crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL,
            serde_json::json!({
                "agent": "router",
                "message": "The review has started."
            }),
        )
        .await
        .expect("reviewer should be able to message the original sender");
    assert!(reply.ok, "{:?}", reply.payload);
    assert_eq!(reply.payload["status"], "started");
    assert_eq!(reply.payload["target_agent_id"], sender.id());

    let snapshot = router
        .runtime_state
        .session_snapshot(session.id())
        .await
        .expect("session should remain readable");
    let reviewer_prompt = snapshot
        .active_prompt_for_agent(reviewer.id())
        .expect("reviewer should have an active agent message");
    assert_eq!(
        reviewer_prompt.prompt(),
        "agent router message:\n\nInspect package.json and report the package name."
    );
    assert_eq!(reviewer_prompt.attachments().len(), 1);
    assert_eq!(reviewer_prompt.attachments()[0].mime(), "image/png");
    assert_eq!(
        reviewer_prompt.attachments()[0].filename(),
        Some("diagram.png")
    );
    assert_eq!(
        reviewer_prompt.attachments()[0].contents_base64(),
        Some("aGVsbG8=")
    );
    assert!(reviewer_prompt
        .hidden_system_context()
        .contains("chariox.send_agent_message"));
    assert_eq!(
        snapshot
            .queued_prompts_for_agent(reviewer.id())
            .expect("reviewer queue should exist")
            .front()
            .map(|prompt| prompt.prompt()),
        None
    );
    assert_eq!(
        snapshot
            .queued_prompts_for_agent(reviewer.id())
            .map(|prompts| prompts.len()),
        Some(0),
        "the idempotent retry must not append a duplicate prompt"
    );
    assert_eq!(
        snapshot
            .active_prompt_for_agent(sender.id())
            .map(|prompt| prompt.prompt()),
        Some("agent reviewer message:\n\nThe review has started.")
    );
}

#[test]
fn agent_message_to_busy_provider_does_not_queue_a_user_prompt() {
    run_agent_message_test_with_large_stack("agent-message-busy", || {
        agent_message_to_busy_provider_does_not_queue_a_user_prompt_inner()
    });
}

async fn agent_message_to_busy_provider_does_not_queue_a_user_prompt_inner() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, sender) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "message-direct",
            "message-direct",
        ))
        .expect("session should be created");
    let target = spawn_test_agent(&mut app, session.id(), "target", "dev-stub");
    let sender_run = launch_test_provider(
        &mut app,
        session.id(),
        sender.id(),
        "dev-stub",
        "dev-stub",
        "sender-model",
    );
    launch_test_provider(
        &mut app,
        session.id(),
        target.id(),
        "dev-stub",
        "dev-stub",
        "target-model",
    );
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-message-direct",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("test client should attach");
    let active = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        target.id(),
        "active task",
        crate::session::PromptStatus::Queued,
    );
    assert!(matches!(
        app.prompt_owner_submit_prepared_prompt(session.id(), active, false)
            .expect("target should start a prompt"),
        crate::session::PromptSubmissionOutcome::Started { .. }
    ));
    let token = sender_run
        .runtime_mcp_auth_token()
        .expect("sender needs runtime MCP auth");
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let result = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            token,
            crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL,
            serde_json::json!({ "agent": target.id(), "message": "new information" }),
        )
        .await
        .expect("agent message should dispatch");
    assert!(result.ok, "{:?}", result.payload);
    assert_eq!(result.payload["status"], "steered");
    let snapshot = router
        .runtime_state
        .session_snapshot(session.id())
        .await
        .expect("session should remain readable");
    assert_eq!(
        snapshot
            .queued_prompts_for_agent(target.id())
            .map(|queue| queue.len()),
        Some(0),
        "agent messages must never become queued user prompts"
    );
    let mut delivered = false;
    for _ in 0..40 {
        delivered = app
            .lock()
            .await
            .terminal()
            .input_records()
            .iter()
            .any(|record| String::from_utf8_lossy(&record.bytes).contains("new information"));
        if delivered {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(delivered, "active provider must receive the agent message");
}

#[tokio::test]
async fn agent_message_during_provider_startup_does_not_enter_the_user_queue() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, sender) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "message-provider-startup",
            "message-provider-startup",
        ))
        .expect("session should be created");
    let target = spawn_test_agent(&mut app, session.id(), "target", "dev-stub");
    let sender_run = launch_test_provider(
        &mut app,
        session.id(),
        sender.id(),
        "dev-stub",
        "dev-stub",
        "sender-model",
    );
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-message-provider-startup",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("test client should attach");
    let active = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        target.id(),
        "starting task",
        crate::session::PromptStatus::Queued,
    );
    assert!(matches!(
        app.prompt_owner_submit_prepared_prompt(session.id(), active, false)
            .expect("target should start a prompt"),
        crate::session::PromptSubmissionOutcome::Started { .. }
    ));
    let token = sender_run
        .runtime_mcp_auth_token()
        .expect("sender needs runtime MCP auth");
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let result = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            token,
            crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL,
            serde_json::json!({ "agent": target.id(), "message": "new information" }),
        )
        .await;
    let snapshot = router
        .runtime_state
        .session_snapshot(session.id())
        .await
        .expect("session should remain readable");
    assert_eq!(
        snapshot
            .queued_prompts_for_agent(target.id())
            .map(|queue| queue.len()),
        Some(0),
        "agent messages must never become queued user prompts"
    );
    assert!(result.is_err(), "provider startup must not claim delivery");
}

#[test]
fn failed_local_agent_message_delivery_can_retry_the_same_idempotency_key() {
    run_agent_message_test_with_large_stack("agent-message-retry", || {
        failed_local_agent_message_delivery_can_retry_the_same_idempotency_key_inner()
    });
}

async fn failed_local_agent_message_delivery_can_retry_the_same_idempotency_key_inner() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, sender) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("message-retry", "message-retry"))
        .expect("session should be created");
    let target = spawn_test_agent(&mut app, session.id(), "target", "dev-stub");
    let sender_run = launch_test_provider(
        &mut app,
        session.id(),
        sender.id(),
        "dev-stub",
        "dev-stub",
        "sender-model",
    );
    let target_run = launch_test_provider(
        &mut app,
        session.id(),
        target.id(),
        "dev-stub",
        "dev-stub",
        "target-model",
    );
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-message-retry",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("test client should attach");
    let first_active = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        target.id(),
        "first task",
        crate::session::PromptStatus::Queued,
    );
    assert!(matches!(
        app.prompt_owner_submit_prepared_prompt(session.id(), first_active, false)
            .expect("first task should start"),
        crate::session::PromptSubmissionOutcome::Started { .. }
    ));
    let sender_token = sender_run
        .runtime_mcp_auth_token()
        .expect("sender needs runtime MCP auth")
        .to_string();
    let session_id = session.id().to_string();
    let target_id = target.id().to_string();
    let attachment_id = attachment.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let lane = router.provider_runtime_lanes.acquire(target_run.id()).await;
    let args = serde_json::json!({
        "agent": target_id,
        "message": "MESSAGE_RETRY_PROOF",
        "idempotency_key": "retry-after-superseded-turn",
    });
    let runtime = router.runtime_state.clone();
    let first_args = args.clone();
    let first_send = tokio::spawn(async move {
        runtime
            .dispatch_authenticated_runtime_tool_call(
                &sender_token,
                crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL,
                first_args,
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let prepared = router
                .operational_history_store
                .load_session_events(&session_id, Some(&target_id))
                .expect("recipient history should remain readable")
                .iter()
                .any(|event| {
                    event
                        .content
                        .as_deref()
                        .is_some_and(|content| content.contains("MESSAGE_RETRY_PROOF"))
                });
            if prepared {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("sender should prepare the steer before the turn settles");
    assert!(
        !first_send.is_finished(),
        "sender must not receive success before provider delivery"
    );
    let completed = app
        .lock()
        .await
        .prompt_owner_complete_active_prompt_only(&session_id, &target_id)
        .expect("first turn should settle while the provider lane is held");
    assert_eq!(completed.prompt(), "first task");
    {
        let guard = app.lock().await;
        let session = guard.sessions().get_session(&session_id).unwrap();
        assert!(guard
            .prompt_state_owner()
            .active_prompt_for_agent(&session, &target_id)
            .is_none());
    }
    drop(lane);
    let first_result = first_send.await.expect("send task should join");
    assert!(
        first_result.is_err(),
        "superseded delivery must fail: {first_result:?}"
    );

    let second_active = crate::session::PromptQueueItem::new(
        app.lock().await.sessions_mut().reserve_prompt_id(),
        &attachment_id,
        &target_id,
        "second task",
        crate::session::PromptStatus::Queued,
    );
    assert!(matches!(
        app.lock()
            .await
            .prompt_owner_submit_prepared_prompt(&session_id, second_active, false)
            .expect("second task should start"),
        crate::session::PromptSubmissionOutcome::Started { .. }
    ));
    let token = app
        .lock()
        .await
        .providers()
        .get_run(sender_run.id())
        .expect("sender provider should remain")
        .runtime_mcp_auth_token()
        .expect("sender runtime auth should remain")
        .to_string();
    let retried = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &token,
            crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL,
            args,
        )
        .await
        .expect("same idempotency key should retry after failure");
    assert!(retried.ok, "{:?}", retried.payload);
    assert_eq!(retried.payload["status"], "steered");
    assert_eq!(
        app.lock()
            .await
            .terminal()
            .input_records()
            .iter()
            .filter(|record| String::from_utf8_lossy(&record.bytes).contains("MESSAGE_RETRY_PROOF"))
            .count(),
        1,
        "the provider should receive the retried message exactly once"
    );
}

#[tokio::test]
async fn runtime_mcp_agent_message_rejects_reused_idempotency_key_for_different_message() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, sender) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-agent-message-idempotency",
            "worktree-agent-message-idempotency",
        ))
        .expect("session should be created");
    let target = spawn_test_agent(&mut app, session.id(), "target", "dev-stub");
    let sender_run = launch_test_provider(
        &mut app,
        session.id(),
        sender.id(),
        "dev-stub",
        "dev-stub",
        "sender-model",
    );
    launch_test_provider(
        &mut app,
        session.id(),
        target.id(),
        "dev-stub",
        "dev-stub",
        "target-model",
    );
    let auth_token = sender_run
        .runtime_mcp_auth_token()
        .expect("sender run should expose runtime MCP auth")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let first = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &auth_token,
            crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL,
            serde_json::json!({
                "agent": "target",
                "message": "First message.",
                "idempotency_key": "shared-send"
            }),
        )
        .await
        .expect("first message should dispatch");
    assert!(first.ok, "{:?}", first.payload);

    let conflict = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &auth_token,
            crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL,
            serde_json::json!({
                "agent": "target",
                "message": "Different message.",
                "idempotency_key": "shared-send"
            }),
        )
        .await
        .expect("a reused key with another message should return a structured failure");
    assert!(!conflict.ok, "{:?}", conflict.payload);
    assert!(conflict.payload["error"]
        .as_str()
        .is_some_and(|error| error.contains("already used")));
}

#[tokio::test]
async fn runtime_mcp_agent_message_rejects_unknown_and_self_targets_without_prompt_state() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, sender) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-agent-message-errors",
            "worktree-agent-message-errors",
        ))
        .expect("session should be created");
    let sender_run = launch_test_provider(
        &mut app,
        session.id(),
        sender.id(),
        "dev-stub",
        "dev-stub",
        "sender-model",
    );
    let auth_token = sender_run
        .runtime_mcp_auth_token()
        .expect("sender run should expose runtime MCP auth")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    for agent in ["missing", sender.id()] {
        let result = router
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(
                &auth_token,
                crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL,
                serde_json::json!({
                    "agent": agent,
                    "message": "Do not dispatch this."
                }),
            )
            .await
            .expect("invalid target should return a structured tool failure");
        assert!(!result.ok, "{:?}", result.payload);
    }
    let missing = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &auth_token,
            crate::transport::runtime_tools::GET_SESSION_AGENT_TOOL,
            serde_json::json!({ "agent": "missing" }),
        )
        .await
        .expect("unknown discovery target should return a structured failure");
    assert!(!missing.ok, "{:?}", missing.payload);
    assert!(missing.payload["error"]
        .as_str()
        .is_some_and(|error| error.contains("available agents")));
    let snapshot = router
        .runtime_state
        .session_snapshot(session.id())
        .await
        .expect("session should remain readable");
    assert!(snapshot.active_prompt_for_agent(sender.id()).is_none());
    assert!(snapshot
        .queued_prompts_for_agent(sender.id())
        .into_iter()
        .flatten()
        .next()
        .is_none());
}

#[tokio::test]
async fn runtime_mcp_metaagent_can_message_an_existing_session_agent() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, metaagent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-metaagent-messaging",
            "worktree-metaagent-messaging",
        ))
        .expect("session should be created");
    let metaagent = app
        .agents_mut()
        .activate_agent_meta_mode(metaagent.id(), None)
        .expect("agent should enter Meta mode");
    let worker = spawn_test_agent(&mut app, session.id(), "worker", "dev-stub");
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    launch_test_provider(
        &mut app,
        session.id(),
        worker.id(),
        "dev-stub",
        "dev-stub",
        "worker-model",
    );
    let auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    assert!(router
        .runtime_state
        .runtime_tool_specs_for_auth_token(&auth_token)
        .iter()
        .any(|spec| { spec.name == crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL }));
    let result = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &auth_token,
            crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL,
            serde_json::json!({
                "agent": "worker",
                "message": "Perform the delegated check."
            }),
        )
        .await
        .expect("Meta agent message should dispatch");
    assert!(result.ok, "{:?}", result.payload);
    assert_eq!(result.payload["status"], "started");
}
