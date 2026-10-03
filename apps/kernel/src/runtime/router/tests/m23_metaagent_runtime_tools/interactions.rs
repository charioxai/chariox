use super::*;

#[test]
fn metaagent_can_resolve_owned_regular_agent_interactions_but_not_its_own() {
    run_large_stack_async_test(
        "metaagent-can-resolve-owned-regular-agent-interactions-but-not-its-own",
        metaagent_can_resolve_owned_regular_agent_interactions_but_not_its_own_inner,
    );
}

async fn metaagent_can_resolve_owned_regular_agent_interactions_but_not_its_own_inner() {
    let env = TestMetaRuntimeEnv::new("interaction");
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
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let worker_run = launch_test_provider(
        &mut app,
        session.id(),
        worker.id(),
        "dev-stub",
        "dev-stub",
        "worker-model",
    );
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let attach = attach_request(session.id(), "client-meta-busy");
    let attachment_id = match router
        .dispatch(
            KernelCommand::from_local_request("attach-meta-busy", None, None, &attach),
            attach,
        )
        .await
        .expect("client should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment.id().to_string(),
        other => panic!("unexpected attach response: {other:?}"),
    };
    let meta_prompt = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session.id().to_string(),
        attachment_id,
        target_agent_id: Some(metaagent.id().to_string()),
        prompt: "stay busy while a worker asks for permission".to_string(),
        attachments: Vec::new(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("submit-meta-busy", None, None, &meta_prompt),
            meta_prompt,
        )
        .await
        .expect("metaagent prompt should start");
    let worker_interaction = RuntimeInteraction::new(
        "interaction-worker",
        worker.id(),
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Warning,
        Some("Allow command?".to_string()),
        "Allow command?",
        vec![
            RuntimeInteractionChoice::new(
                "allow_once",
                "Allow once",
                "allow",
                Some(RuntimeInteractionChoiceStyle::Primary),
            ),
            RuntimeInteractionChoice::new(
                "deny",
                "Deny",
                "deny",
                Some(RuntimeInteractionChoiceStyle::Danger),
            ),
        ],
        None,
        None,
        None,
    );
    let worker_resolution = router
        .runtime_state
        .create_runtime_interaction(session.id(), worker_interaction)
        .await
        .expect("worker interaction should register");
    let meta_queued_prompts = app
        .lock()
        .await
        .sessions()
        .get_session(session.id())
        .expect("session should load")
        .queued_prompts_for_agent(metaagent.id())
        .map(|queued| queued.len())
        .unwrap_or_default();
    assert_eq!(
        meta_queued_prompts, 0,
        "runtime interaction event prompts should steer an active metaagent instead of queueing"
    );
    let listed_events = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
            serde_json::json!({ "kind": "runtime.interaction" }),
        )
        .await
        .expect("required interaction event should be listed");
    assert!(listed_events.ok);
    let events = listed_events
        .payload
        .get("events")
        .and_then(serde_json::Value::as_array)
        .expect("interaction events should be returned");
    assert_eq!(events.len(), 1);
    assert_eq!(
        events
            .first()
            .and_then(|event| event.get("source_agent_id"))
            .and_then(serde_json::Value::as_str),
        Some(worker.id())
    );

    let resolved = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RESOLVE_RUNTIME_INTERACTION_TOOL,
            serde_json::json!({
                "interaction_id": "interaction-worker",
                "choice_id": "allow_once"
            }),
        )
        .await
        .expect("meta interaction resolution should dispatch");
    assert!(resolved.ok, "{:?}", resolved.payload);
    let resolution = tokio::time::timeout(std::time::Duration::from_secs(1), worker_resolution)
        .await
        .expect("resolution should be delivered")
        .expect("interaction responder should receive resolution");
    assert_eq!(resolution.choice_id.as_deref(), Some("allow_once"));
    assert_eq!(resolution.reply.as_deref(), Some("allow"));
    let audit_events = app
        .lock()
        .await
        .durable_state_store()
        .load_events_after(0)
        .expect("durable audit events should load");
    let resolution_audit = audit_events
        .iter()
        .find(|event| {
            event.kind == "metaagent.interaction.resolved"
                && event.payload["session_id"] == session.id()
                && event.payload["metaagent_id"] == metaagent.id()
                && event.payload["target_agent_id"] == worker.id()
                && event.payload["interaction_id"] == "interaction-worker"
                && event.payload["choice_id"] == "allow_once"
                && event.payload["causation_id"] == "interaction-worker"
                && event.payload["correlation_id"]
                    == format!(
                        "metaagent:{}:runtime-interaction:interaction-worker",
                        metaagent.id()
                    )
        })
        .expect("metaagent interaction resolution should include durable provenance");
    assert_eq!(resolution_audit.payload["provider_run_id"], worker_run.id());
    assert!(
        resolution_audit
            .payload
            .get("timestamp_ms")
            .and_then(serde_json::Value::as_u64)
            .is_some(),
        "{:?}",
        resolution_audit.payload
    );
    assert_eq!(resolution_audit.payload["input"], serde_json::Value::Null);

    let custom_interaction = RuntimeInteraction::new(
        "interaction-custom-worker",
        worker.id(),
        RuntimeInteractionKind::Choice,
        RuntimeInteractionLevel::Warning,
        Some("Explain approval".to_string()),
        "Explain approval",
        vec![RuntimeInteractionChoice::new(
            "cancel",
            "Cancel",
            "cancel",
            Some(RuntimeInteractionChoiceStyle::Danger),
        )],
        Some(crate::session::RuntimeInteractionCustomChoice::new(
            "custom_reason",
            "Custom reason",
            Some("Reason".to_string()),
            Some(3),
            Some(256),
        )),
        None,
        None,
    );
    let custom_resolution = router
        .runtime_state
        .create_runtime_interaction(session.id(), custom_interaction)
        .await
        .expect("custom worker interaction should register");
    let custom_reply = "ship after checking logs";
    let custom_resolved = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RESOLVE_RUNTIME_INTERACTION_TOOL,
            serde_json::json!({
                "interaction_id": "interaction-custom-worker",
                "choice_id": "custom_reason",
                "input": custom_reply
            }),
        )
        .await
        .expect("custom meta interaction resolution should dispatch");
    assert!(custom_resolved.ok, "{:?}", custom_resolved.payload);
    let custom_runtime_resolution =
        tokio::time::timeout(std::time::Duration::from_secs(1), custom_resolution)
            .await
            .expect("custom resolution should be delivered")
            .expect("custom interaction responder should receive resolution");
    assert_eq!(
        custom_runtime_resolution.choice_id.as_deref(),
        Some("custom_reason")
    );
    assert_eq!(
        custom_runtime_resolution.reply.as_deref(),
        Some(custom_reply)
    );
    let custom_audit_events = app
        .lock()
        .await
        .durable_state_store()
        .load_events_after(0)
        .expect("durable audit events should load");
    let custom_audit = custom_audit_events
        .iter()
        .find(|event| {
            event.kind == "metaagent.interaction.resolved"
                && event.payload["interaction_id"] == "interaction-custom-worker"
        })
        .expect("custom interaction resolution should include durable provenance");
    assert_eq!(custom_audit.payload["provider_run_id"], worker_run.id());
    assert_eq!(
        custom_audit.payload.pointer("/input/kind"),
        Some(&serde_json::json!("custom"))
    );
    assert_eq!(
        custom_audit.payload.pointer("/input/char_count"),
        Some(&serde_json::json!(custom_reply.chars().count()))
    );
    assert_eq!(
        custom_audit.payload["input"]["reply"],
        serde_json::Value::Null
    );

    let self_interaction = RuntimeInteraction::new(
        "interaction-meta",
        metaagent.id(),
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Warning,
        Some("Allow self?".to_string()),
        "Allow self?",
        vec![RuntimeInteractionChoice::new(
            "allow_once",
            "Allow once",
            "allow",
            Some(RuntimeInteractionChoiceStyle::Primary),
        )],
        None,
        None,
        None,
    );
    let _self_resolution = router
        .runtime_state
        .create_runtime_interaction(session.id(), self_interaction)
        .await
        .expect("self interaction should register");
    let denied = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RESOLVE_RUNTIME_INTERACTION_TOOL,
            serde_json::json!({
                "interaction_id": "interaction-meta",
                "choice_id": "allow_once"
            }),
        )
        .await
        .expect("self resolution denial should dispatch");
    assert!(!denied.ok);
    assert!(
        denied
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("cannot resolve their own")),
        "{:?}",
        denied.payload
    );
}

/// A Meta agent's `run_command` can wait on a person too: an App binding under
/// Ask. The router's runtime tool entry marks the calling run as waiting for
/// that whole time, so a Claude run's turn stall watchdog does not end it.
#[test]
fn a_meta_app_binding_approval_counts_as_a_runtime_tool_wait() {
    run_large_stack_async_test(
        "meta-app-binding-runtime-tool-wait",
        a_meta_app_binding_approval_counts_as_a_runtime_tool_wait_inner,
    );
}

async fn a_meta_app_binding_approval_counts_as_a_runtime_tool_wait_inner() {
    let env = TestMetaRuntimeEnv::new("app-binding-wait");
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
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_permission_level_override(crate::provider::AgentPermissionLevel::Required),
        )
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("worker")
                .with_controlled_by_metaagent_id(metaagent.id()),
        )
        .expect("worker should spawn");
    // The wait registry is per process, so this run's id must be unique
    // among the tests running beside it.
    let meta_run_id = format!("provider-run-meta-app-wait-{:016x}", rand::random::<u64>());
    let token = format!("meta-app-wait-token-{:016x}", rand::random::<u64>());
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "dev-stub",
        "dev-stub",
        "default",
        "meta-model",
    )
    .with_agent_id(metaagent.id())
    .with_runtime_mcp_binding(crate::provider::RuntimeMcpBinding::new(
        "http://127.0.0.1:1/mcp",
        token.clone(),
    ));
    let mut meta_run = crate::provider::RuntimeProviderRun::new(
        &meta_run_id,
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "dev-stub".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: Default::default(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("stdio://dev-stub".to_string()),
        },
    );
    meta_run.mark_running();
    app.providers_mut().insert_run_for_test(meta_run);
    let app = Arc::new(Mutex::new(app));
    let router = Arc::new(CommandRouter::with_interactive_capacity(
        Arc::clone(&app),
        4,
    ));
    assert!(!crate::provider::claude_runtime_tool_wait_pending(
        &meta_run_id
    ));

    let call_router = Arc::clone(&router);
    let call = tokio::spawn(async move {
        call_router
            .dispatch_authenticated_runtime_tool_call(
                &token,
                crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
                serde_json::json!({"command": "extension grant app worker installed"}),
            )
            .await
    });
    let interaction_id = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(id) = app
                .lock()
                .await
                .sessions()
                .get_session(session.id())
                .expect("session should exist")
                .active_interaction_for_agent(metaagent.id())
                .map(|interaction| interaction.id().to_string())
            {
                break id;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the App binding approval should open");
    assert!(
        crate::provider::claude_runtime_tool_wait_pending(&meta_run_id),
        "the run waits on the approval, so its watchdog must see activity"
    );

    router
        .runtime_state()
        .resolve_runtime_interaction(session.id(), &interaction_id, "deny", None)
        .await
        .expect("approval should resolve");
    let _ = call.await.expect("run_command should join");
    assert!(!crate::provider::claude_runtime_tool_wait_pending(
        &meta_run_id
    ));
}
