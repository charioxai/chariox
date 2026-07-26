use super::*;

#[tokio::test]
async fn metaagent_runtime_mcp_manages_scoped_task_artifacts() {
    tokio::spawn(metaagent_runtime_mcp_manages_scoped_task_artifacts_impl())
        .await
        .expect("metaagent task artifact test should join");
}

async fn metaagent_runtime_mcp_manages_scoped_task_artifacts_impl() {
    let env = TestMetaRuntimeEnv::new("task-artifacts");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, standard_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    let standard_run = launch_test_provider(
        &mut app,
        session.id(),
        standard_agent.id(),
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
    let standard_auth_token = standard_run
        .runtime_mcp_auth_token()
        .expect("standard run should expose runtime MCP auth token")
        .to_string();
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let meta_specs = router
        .runtime_state
        .runtime_tool_specs_for_auth_token(&meta_auth_token);
    assert!(
        meta_specs
            .iter()
            .any(|spec| spec.name == crate::transport::runtime_tools::META_READ_TASK_TOOL),
        "metaagents should see task artifact tools"
    );
    assert!(
        !router
            .runtime_state
            .runtime_tool_specs_for_auth_token(&standard_auth_token)
            .iter()
            .any(|spec| spec.name == crate::transport::runtime_tools::META_READ_TASK_TOOL),
        "standard agents must not see task artifact tools"
    );

    let initial = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_READ_TASK_TOOL,
            serde_json::json!({}),
        )
        .await
        .expect("metaagent should read empty task state");
    assert!(initial.ok, "{:?}", initial.payload);
    assert_eq!(
        initial
            .payload
            .pointer("/status")
            .and_then(serde_json::Value::as_str),
        Some("none")
    );

    let task = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_UPDATE_TASK_TOOL,
            serde_json::json!({ "markdown": "# Task\nPlan the work." }),
        )
        .await
        .expect("metaagent should update task");
    assert!(task.ok, "{:?}", task.payload);
    assert_eq!(
        task.payload
            .pointer("/task/status")
            .and_then(serde_json::Value::as_str),
        Some("active")
    );
    assert_eq!(
        task.payload
            .pointer("/task/task_markdown")
            .and_then(serde_json::Value::as_str),
        Some("# Task\nPlan the work.")
    );

    let plan = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_UPDATE_PLAN_TOOL,
            serde_json::json!({ "markdown": "1. Delegate implementation." }),
        )
        .await
        .expect("metaagent should update plan");
    assert!(plan.ok, "{:?}", plan.payload);
    assert_eq!(
        plan.payload
            .pointer("/plan_markdown")
            .and_then(serde_json::Value::as_str),
        Some("1. Delegate implementation.")
    );
    let app_guard = app.lock().await;
    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session.id().to_string(),
    });
    let state_command =
        KernelCommand::from_local_request("meta-task-projection-state", None, None, &state_request);
    let state_router = router.clone();
    let state_task =
        tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });
    tokio::task::yield_now().await;
    assert!(
        state_task.is_finished(),
        "meta task runtime tool updates should publish a complete session projection"
    );
    drop(app_guard);
    let state_response = state_task
        .await
        .expect("state task should join")
        .expect("state should resolve");
    match state_response {
        LocalDaemonResponse::SessionState { session, .. } => {
            assert!(
                session
                    .agents()
                    .iter()
                    .any(|agent| agent.id() == metaagent.id()),
                "projected session should retain agent membership"
            );
            assert_eq!(
                session
                    .metaagent_task(metaagent.id())
                    .map(|task| task.plan_markdown()),
                Some("1. Delegate implementation."),
                "projected session should retain the metaagent task"
            );
        }
        other => panic!("unexpected state response: {other:?}"),
    }

    let blocked = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_MARK_BLOCKED_TOOL,
            serde_json::json!({ "reason": "worker unavailable" }),
        )
        .await
        .expect("metaagent should mark task blocked");
    assert!(blocked.ok, "{:?}", blocked.payload);
    assert_eq!(
        blocked
            .payload
            .pointer("/task/status")
            .and_then(serde_json::Value::as_str),
        Some("blocked")
    );
    assert_eq!(
        blocked
            .payload
            .pointer("/task/blocked_reason")
            .and_then(serde_json::Value::as_str),
        Some("worker unavailable")
    );
    let metaagent_after_block = router
        .runtime_state
        .list_agents()
        .into_iter()
        .find(|agent| agent.id() == metaagent.id())
        .expect("metaagent should still exist");
    assert!(
        !metaagent_after_block.is_metaagent(),
        "terminal blocked state should exit meta mode"
    );
    assert!(
        router
            .runtime_state
            .runtime_tool_specs_for_auth_token(&meta_auth_token)
            .iter()
            .all(|spec| {
                spec.name != crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL
            }),
        "stale meta provider token should lose meta tools after terminal task state"
    );

    let denied = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &standard_auth_token,
            crate::transport::runtime_tools::META_UPDATE_TASK_TOOL,
            serde_json::json!({ "markdown": "not allowed" }),
        )
        .await
        .expect_err("standard agents should not call meta task tools");
    assert!(
        denied
            .to_string()
            .contains("exactly one active provider run for an agent in Meta mode"),
        "{denied:?}"
    );
}

#[tokio::test]
async fn metaagent_terminal_states_wait_for_controlled_work_to_settle() {
    let env = TestMetaRuntimeEnv::new("task-terminal-state-guard");
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
    launch_test_provider(
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
    app.sessions_mut()
        .start_or_update_metaagent_task(session.id(), metaagent.id(), "Delegate and verify.")
        .expect("meta task should start");
    let worker_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "meta-worker-task",
            crate::attachment::ClientCapabilityLevel::AutomationOnly,
        ))
        .expect("worker attachment should attach");
    let worker_prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        worker_attachment.id(),
        worker.id(),
        "Finish the delegated check.",
        crate::session::PromptStatus::Queued,
    );
    let outcome = app
        .prompt_owner_submit_prepared_prompt(session.id(), worker_prompt, false)
        .expect("worker prompt should submit");
    assert!(matches!(
        outcome,
        crate::session::PromptSubmissionOutcome::Started { .. }
    ));
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    for (tool, arguments) in [
        (
            crate::transport::runtime_tools::META_COMPLETE_TASK_TOOL,
            serde_json::json!({ "summary": "too early" }),
        ),
        (
            crate::transport::runtime_tools::META_MARK_BLOCKED_TOOL,
            serde_json::json!({ "reason": "too early" }),
        ),
    ] {
        let error = router
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(&meta_auth_token, tool, arguments)
            .await
            .expect_err("terminal task state must wait for controlled work");
        assert!(
            error
                .to_string()
                .contains("controlled agent or workflow still has active"),
            "{error:?}"
        );
    }

    let session_after = router
        .runtime_state
        .session_snapshot(session.id())
        .await
        .expect("session should remain available");
    assert_eq!(
        session_after
            .metaagent_task(metaagent.id())
            .map(|task| task.status()),
        Some(crate::session::MetaagentTaskStatus::Active)
    );
    assert!(router
        .runtime_state
        .list_agents()
        .into_iter()
        .find(|agent| agent.id() == metaagent.id())
        .is_some_and(|agent| agent.is_metaagent()));
}

#[tokio::test]
async fn terminal_metaagent_task_drops_private_event_prompts_but_keeps_user_queue() {
    let env = TestMetaRuntimeEnv::new("task-terminal-private-prompt-cleanup");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
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
    app.sessions_mut()
        .start_or_update_metaagent_task(session.id(), metaagent.id(), "Finish cleanly.")
        .expect("meta task should start");
    let task_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            format!("metaagent:{}:task", metaagent.id()),
            crate::attachment::ClientCapabilityLevel::AutomationOnly,
        ))
        .expect("meta task attachment should attach");
    let user_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "user-follow-up",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("user attachment should attach");
    for (attachment_id, prompt) in [
        (
            task_attachment.id(),
            crate::scheduler::prompt_injection::METAAGENT_EVENT_VISIBLE_PROMPT,
        ),
        (user_attachment.id(), "Keep this user follow-up queued."),
    ] {
        let queued = crate::session::PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            attachment_id,
            metaagent.id(),
            prompt,
            crate::session::PromptStatus::Queued,
        );
        let outcome = app
            .prompt_owner_submit_prepared_prompt(session.id(), queued, true)
            .expect("prompt should queue");
        assert!(matches!(
            outcome,
            crate::session::PromptSubmissionOutcome::Queued { .. }
        ));
    }
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
            crate::transport::runtime_tools::META_COMPLETE_TASK_TOOL,
            serde_json::json!({ "summary": "done" }),
        )
        .await
        .expect("Meta task should complete");

    let snapshot = router
        .runtime_state
        .session_snapshot(session.id())
        .await
        .expect("session should remain available");
    let queued = snapshot
        .queued_prompts_for_agent(metaagent.id())
        .expect("user follow-up should remain queued");
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued.front().map(|prompt| prompt.prompt()),
        Some("Keep this user follow-up queued.")
    );
}

#[tokio::test]
async fn prompt_to_metaagent_creates_task_without_overwriting_active_task() {
    let env = TestMetaRuntimeEnv::new("prompt-task-create");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let attach = attach_request(session.id(), "client-prompt-task-create");
    let attachment_id = match router
        .dispatch(
            KernelCommand::from_local_request("attach-prompt-task-create", None, None, &attach),
            attach,
        )
        .await
        .expect("client should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment.id().to_string(),
        other => panic!("unexpected attach response: {other:?}"),
    };

    let first_prompt = "figure out the repo and organize the work";
    let submit = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session.id().to_string(),
        attachment_id: attachment_id.clone(),
        target_agent_id: Some(metaagent.id().to_string()),
        prompt: first_prompt.to_string(),
        attachments: Vec::new(),
    });
    let first = router
        .dispatch(
            KernelCommand::from_local_request("submit-meta-task", None, None, &submit),
            submit,
        )
        .await
        .expect("metaagent prompt should submit");
    let LocalDaemonResponse::PromptSubmitted { session, .. } = first else {
        panic!("unexpected submit response: {first:?}");
    };
    assert_eq!(
        session
            .metaagent_task(metaagent.id())
            .map(|task| task.task_markdown()),
        Some(first_prompt)
    );

    let followup = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session.id().to_string(),
        attachment_id,
        target_agent_id: Some(metaagent.id().to_string()),
        prompt: "also keep the report short".to_string(),
        attachments: Vec::new(),
    });
    let second = router
        .dispatch(
            KernelCommand::from_local_request("submit-meta-task-followup", None, None, &followup),
            followup,
        )
        .await
        .expect("metaagent follow-up prompt should submit");
    let LocalDaemonResponse::PromptSubmitted { session, .. } = second else {
        panic!("unexpected submit response: {second:?}");
    };
    assert_eq!(
        session
            .metaagent_task(metaagent.id())
            .map(|task| task.task_markdown()),
        Some(first_prompt)
    );
}

#[test]
fn local_metaagent_task_update_notifies_metaagent() {
    run_large_stack_async_test(
        "local-metaagent-task-update-notifies-metaagent",
        local_metaagent_task_update_notifies_metaagent_inner,
    );
}

async fn local_metaagent_task_update_notifies_metaagent_inner() {
    let env = TestMetaRuntimeEnv::new("local-task-update-notify");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let update =
        LocalDaemonRequest::UpdateMetaagentTask(crate::local::UpdateMetaagentTaskRequest {
            session_id: session.id().to_string(),
            metaagent_id: metaagent.id().to_string(),
            task_markdown: Some("# Updated task".to_string()),
            plan_markdown: Some("1. Re-plan.".to_string()),
        });
    let response = router
        .dispatch(
            KernelCommand::from_local_request("update-meta-task", None, None, &update),
            update,
        )
        .await
        .expect("task update should dispatch");
    let LocalDaemonResponse::MetaagentTaskUpdated { session, task } = response else {
        panic!("unexpected task response: {response:?}");
    };
    assert_eq!(
        task.as_ref().map(|task| task.task_markdown()),
        Some("# Updated task")
    );
    let active = session
        .active_prompt_for_agent(metaagent.id())
        .expect("task update should notify the metaagent");
    assert_eq!(
        active.prompt(),
        "<metaagent-event/>",
        "task lifecycle notifications must remain private in visible history"
    );
    assert!(
        active
            .hidden_system_context()
            .contains("edited your task and plan"),
        "{}",
        active.hidden_system_context()
    );
    assert!(
        active.hidden_system_context().contains("# Updated task"),
        "{}",
        active.hidden_system_context()
    );
    let task_attachments = app
        .lock()
        .await
        .attachments()
        .list_client_attachments(&format!("metaagent:{}:task", metaagent.id()));
    assert_eq!(task_attachments.len(), 1);
    assert_eq!(task_attachments[0].session_id(), session.id());
}

#[test]
fn local_metaagent_task_pause_and_abort_cancel_active_prompt() {
    run_large_stack_async_test(
        "local-metaagent-task-pause-and-abort-cancel-active-prompt",
        local_metaagent_task_pause_and_abort_cancel_active_prompt_inner,
    );
}

async fn local_metaagent_task_pause_and_abort_cancel_active_prompt_inner() {
    let env = TestMetaRuntimeEnv::new("local-task-cancel");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let update =
        LocalDaemonRequest::UpdateMetaagentTask(crate::local::UpdateMetaagentTaskRequest {
            session_id: session.id().to_string(),
            metaagent_id: metaagent.id().to_string(),
            task_markdown: Some("# Active task".to_string()),
            plan_markdown: Some("1. Keep going.".to_string()),
        });
    router
        .dispatch(
            KernelCommand::from_local_request("update-meta-task-before-pause", None, None, &update),
            update,
        )
        .await
        .expect("task update should start notification prompt");

    let pause = LocalDaemonRequest::PauseMetaagentTask(crate::local::PauseMetaagentTaskRequest {
        session_id: session.id().to_string(),
        metaagent_id: metaagent.id().to_string(),
    });
    let paused = router
        .dispatch(
            KernelCommand::from_local_request("pause-meta-task", None, None, &pause),
            pause,
        )
        .await
        .expect("pause should dispatch");
    let LocalDaemonResponse::MetaagentTaskUpdated { session, task } = paused else {
        panic!("unexpected pause response: {paused:?}");
    };
    assert_eq!(
        task.as_ref().map(|task| task.status()),
        Some(crate::session::MetaagentTaskStatus::Paused)
    );
    let paused_prompt = session
        .active_prompt_for_agent(metaagent.id())
        .expect("pause should retain the cancelling meta task prompt");
    assert_eq!(
        paused_prompt.status(),
        crate::session::PromptStatus::Cancelling
    );
    let paused_prompt_id = paused_prompt.id().to_string();
    assert!(
        session
            .agents()
            .iter()
            .find(|agent| agent.id() == metaagent.id())
            .is_some_and(|agent| agent.is_metaagent()),
        "pause must keep the agent in meta mode"
    );

    let edit_while_paused =
        LocalDaemonRequest::UpdateMetaagentTask(crate::local::UpdateMetaagentTaskRequest {
            session_id: session.id().to_string(),
            metaagent_id: metaagent.id().to_string(),
            task_markdown: Some("# Edited while paused".to_string()),
            plan_markdown: None,
        });
    let edited = router
        .dispatch(
            KernelCommand::from_local_request(
                "edit-paused-meta-task",
                None,
                None,
                &edit_while_paused,
            ),
            edit_while_paused,
        )
        .await
        .expect("editing a paused task should queue its notification");
    let LocalDaemonResponse::MetaagentTaskUpdated { session, task } = edited else {
        panic!("unexpected paused edit response: {edited:?}");
    };
    assert_eq!(
        task.as_ref().map(|task| task.status()),
        Some(crate::session::MetaagentTaskStatus::Paused)
    );
    assert!(
        session
            .active_prompt_for_agent(metaagent.id())
            .is_none_or(|prompt| prompt.id() == paused_prompt_id),
        "editing while paused must not start another provider turn"
    );
    let queued = session
        .queued_prompts_for_agent(metaagent.id())
        .expect("paused task edit should queue one notification");
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued[0].prompt(),
        "<metaagent-event/>",
        "paused task edits must remain private in visible history"
    );
    assert!(
        queued[0]
            .hidden_system_context()
            .contains("Edited while paused"),
        "{}",
        queued[0].hidden_system_context()
    );

    let abort = LocalDaemonRequest::AbortMetaagentTask(crate::local::AbortMetaagentTaskRequest {
        session_id: session.id().to_string(),
        metaagent_id: metaagent.id().to_string(),
        reason: Some("user stopped the task".to_string()),
    });
    let aborted = router
        .dispatch(
            KernelCommand::from_local_request("abort-meta-task", None, None, &abort),
            abort,
        )
        .await
        .expect("abort should dispatch");
    let LocalDaemonResponse::MetaagentTaskUpdated { session, task } = aborted else {
        panic!("unexpected abort response: {aborted:?}");
    };
    assert_eq!(
        task.as_ref().map(|task| task.status()),
        Some(crate::session::MetaagentTaskStatus::Aborted)
    );
    if let Some(aborted_prompt) = session.active_prompt_for_agent(metaagent.id()) {
        assert_eq!(aborted_prompt.id(), paused_prompt_id);
        assert_eq!(
            aborted_prompt.status(),
            crate::session::PromptStatus::Cancelling
        );
    }
    assert!(
        session
            .agents()
            .iter()
            .find(|agent| agent.id() == metaagent.id())
            .is_some_and(|agent| !agent.is_metaagent()),
        "abort must restore the agent to regular mode"
    );
    let durable_events = app
        .lock()
        .await
        .durable_state_store()
        .load_events_after(0)
        .expect("durable task lifecycle events should load");
    for reason in ["metaagent_task_paused", "metaagent_task_aborted"] {
        assert!(
            durable_events.iter().any(|event| {
                event.kind == "session.updated"
                    && event.subject_id.as_deref() == Some(session.id())
                    && event.payload["reason"] == reason
            }),
            "{reason} must be durable across kernel restart"
        );
    }
}

#[test]
fn resumed_metaagent_task_prioritizes_preserved_user_prompt() {
    run_large_stack_async_test(
        "resumed-metaagent-task-prioritizes-preserved-user-prompt",
        resumed_metaagent_task_prioritizes_preserved_user_prompt_inner,
    );
}

async fn resumed_metaagent_task_prioritizes_preserved_user_prompt_inner() {
    let env = TestMetaRuntimeEnv::new("paused-task-queue");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    app.sessions_mut()
        .start_or_update_metaagent_task(session.id(), metaagent.id(), "# Active task")
        .expect("task should start");
    app.sessions_mut()
        .set_metaagent_task_status(
            session.id(),
            metaagent.id(),
            crate::session::MetaagentTaskStatus::Paused,
        )
        .expect("task should pause");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::for_user(
            session.id(),
            "paused-queue-user",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
            metaagent.owner_user_id(),
        ))
        .expect("user attachment should attach");
    let queued_text = "user follow-up must wait while paused";
    let queued = crate::session::PromptQueueItem::new(
        "pending:paused-user-followup",
        attachment.id(),
        metaagent.id(),
        queued_text,
        crate::session::PromptStatus::Queued,
    );
    let queued = app
        .prompt_owner_submit_prepared_prompt(session.id(), queued, true)
        .expect("user follow-up should submit");
    assert!(matches!(
        queued,
        crate::session::PromptSubmissionOutcome::Queued { .. }
    ));
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let resume =
        LocalDaemonRequest::ResumeMetaagentTask(crate::local::ResumeMetaagentTaskRequest {
            session_id: session.id().to_string(),
            metaagent_id: metaagent.id().to_string(),
        });
    let resumed = router
        .dispatch(
            KernelCommand::from_local_request("resume-meta-task-with-queue", None, None, &resume),
            resume,
        )
        .await
        .expect("resume should dispatch");
    let LocalDaemonResponse::MetaagentTaskUpdated { session, task } = resumed else {
        panic!("unexpected resume response: {resumed:?}");
    };
    assert_eq!(
        task.as_ref().map(|task| task.status()),
        Some(crate::session::MetaagentTaskStatus::Active)
    );
    let active = session
        .active_prompt_for_agent(metaagent.id())
        .expect("resume should promote the preserved user prompt");
    assert_eq!(active.prompt(), queued_text);
    assert_eq!(
        session
            .queued_prompts_for_agent(metaagent.id())
            .map(|prompts| prompts.len()),
        Some(1)
    );
}

#[test]
fn resumed_metaagent_task_uses_queued_edit_without_duplicate_notification() {
    run_large_stack_async_test(
        "resumed-metaagent-task-uses-queued-edit",
        resumed_metaagent_task_uses_queued_edit_without_duplicate_notification_inner,
    );
}

async fn resumed_metaagent_task_uses_queued_edit_without_duplicate_notification_inner() {
    let env = TestMetaRuntimeEnv::new("paused-task-edit");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    app.sessions_mut()
        .start_or_update_metaagent_task(session.id(), metaagent.id(), "# Original task")
        .expect("task should start");
    app.sessions_mut()
        .set_metaagent_task_status(
            session.id(),
            metaagent.id(),
            crate::session::MetaagentTaskStatus::Paused,
        )
        .expect("task should pause");
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let update =
        LocalDaemonRequest::UpdateMetaagentTask(crate::local::UpdateMetaagentTaskRequest {
            session_id: session.id().to_string(),
            metaagent_id: metaagent.id().to_string(),
            task_markdown: Some("# Edited task".to_string()),
            plan_markdown: None,
        });
    router
        .dispatch(
            KernelCommand::from_local_request("edit-paused-task", None, None, &update),
            update,
        )
        .await
        .expect("paused task edit should queue");

    let resume =
        LocalDaemonRequest::ResumeMetaagentTask(crate::local::ResumeMetaagentTaskRequest {
            session_id: session.id().to_string(),
            metaagent_id: metaagent.id().to_string(),
        });
    let resumed = router
        .dispatch(
            KernelCommand::from_local_request("resume-edited-task", None, None, &resume),
            resume,
        )
        .await
        .expect("edited task should resume");
    let LocalDaemonResponse::MetaagentTaskUpdated { session, task } = resumed else {
        panic!("unexpected resume response: {resumed:?}");
    };
    assert_eq!(
        task.as_ref().map(|task| task.status()),
        Some(crate::session::MetaagentTaskStatus::Active)
    );
    let active = session
        .active_prompt_for_agent(metaagent.id())
        .expect("the queued edit notification should become the active continuation");
    assert_eq!(
        active.prompt(),
        "<metaagent-event/>",
        "task lifecycle notifications must remain private in visible history"
    );
    assert!(
        active.hidden_system_context().contains("# Edited task"),
        "the private continuation must still tell the provider about the edited task"
    );
    assert_eq!(
        session
            .queued_prompts_for_agent(metaagent.id())
            .map(|prompts| prompts.len())
            .unwrap_or_default(),
        0,
        "resume must not add a duplicate continuation behind the queued edit"
    );
}

#[test]
fn resumed_metaagent_task_defers_private_continuation_behind_active_user_prompt() {
    run_large_stack_async_test(
        "resumed-metaagent-task-defers-private-continuation",
        resumed_metaagent_task_defers_private_continuation_behind_active_user_prompt_inner,
    );
}

async fn resumed_metaagent_task_defers_private_continuation_behind_active_user_prompt_inner() {
    let env = TestMetaRuntimeEnv::new("paused-task-active-user");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    app.sessions_mut()
        .start_or_update_metaagent_task(session.id(), metaagent.id(), "# Original task")
        .expect("task should start");
    app.sessions_mut()
        .set_metaagent_task_status(
            session.id(),
            metaagent.id(),
            crate::session::MetaagentTaskStatus::Paused,
        )
        .expect("task should pause");
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let attach = attach_request(session.id(), "client-active-user-before-resume");
    let attachment_id = match router
        .dispatch(
            KernelCommand::from_local_request(
                "attach-active-user-before-resume",
                None,
                None,
                &attach,
            ),
            attach,
        )
        .await
        .expect("client should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment.id().to_string(),
        other => panic!("unexpected attach response: {other:?}"),
    };

    let update =
        LocalDaemonRequest::UpdateMetaagentTask(crate::local::UpdateMetaagentTaskRequest {
            session_id: session.id().to_string(),
            metaagent_id: metaagent.id().to_string(),
            task_markdown: Some("# Edited task".to_string()),
            plan_markdown: None,
        });
    router
        .dispatch(
            KernelCommand::from_local_request("edit-paused-task-before-user", None, None, &update),
            update,
        )
        .await
        .expect("paused task edit should queue");

    let user_text = "user priority prompt is already running";
    let submit = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session.id().to_string(),
        attachment_id,
        target_agent_id: Some(metaagent.id().to_string()),
        prompt: user_text.to_string(),
        attachments: Vec::new(),
    });
    let submitted = router
        .dispatch(
            KernelCommand::from_local_request(
                "submit-active-user-before-resume",
                None,
                None,
                &submit,
            ),
            submit,
        )
        .await
        .expect("user prompt should submit");
    let LocalDaemonResponse::PromptSubmitted {
        outcome: crate::session::PromptSubmissionOutcome::Started { prompt },
        ..
    } = submitted
    else {
        panic!("user prompt should start ahead of the private continuation");
    };
    assert_eq!(prompt.prompt(), user_text);

    let resume =
        LocalDaemonRequest::ResumeMetaagentTask(crate::local::ResumeMetaagentTaskRequest {
            session_id: session.id().to_string(),
            metaagent_id: metaagent.id().to_string(),
        });
    let resumed = router
        .dispatch(
            KernelCommand::from_local_request(
                "resume-with-active-user-prompt",
                None,
                None,
                &resume,
            ),
            resume,
        )
        .await
        .expect("resume should not try to replace the active user prompt");
    let LocalDaemonResponse::MetaagentTaskUpdated { session, task } = resumed else {
        panic!("unexpected resume response: {resumed:?}");
    };
    assert_eq!(
        task.as_ref().map(|task| task.status()),
        Some(crate::session::MetaagentTaskStatus::Active)
    );
    assert_eq!(
        session
            .active_prompt_for_agent(metaagent.id())
            .map(|prompt| prompt.prompt()),
        Some(user_text)
    );
    let queued = session
        .queued_prompts_for_agent(metaagent.id())
        .expect("private continuation should remain queued");
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].prompt(), "<metaagent-event/>");
    assert!(queued[0].hidden_system_context().contains("# Edited task"));
}

#[tokio::test]
async fn runtime_mcp_shared_token_with_metaagent_stays_meta_only() {
    let env = TestMetaRuntimeEnv::new("shared-token-tool-visibility");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, standard_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    let shared_auth_token = "shared-meta-runtime-token".to_string();
    let standard_run = app
        .launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "worker-model",
            )
            .with_agent_id(standard_agent.id())
            .with_runtime_mcp_binding(crate::provider::RuntimeMcpBinding::new(
                "http://127.0.0.1:1",
                shared_auth_token.clone(),
            )),
        )
        .expect("standard provider run should launch");
    app.update_provider_run_projection(standard_run);
    let meta_run = app
        .launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "meta-model",
            )
            .with_agent_id(metaagent.id())
            .with_runtime_mcp_binding(crate::provider::RuntimeMcpBinding::new(
                "http://127.0.0.1:1",
                shared_auth_token.clone(),
            )),
        )
        .expect("meta provider run should launch");
    app.update_provider_run_projection(meta_run);
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let specs = router
        .runtime_state
        .runtime_tool_specs_for_auth_token(&shared_auth_token);
    assert!(
        specs
            .iter()
            .any(|spec| spec.name == crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL),
        "shared token with a metaagent run should expose metaagent tools"
    );
    assert!(
        specs
            .iter()
            .any(|spec| spec.name == crate::transport::runtime_tools::READ_ARTIFACT_TOOL),
        "shared token with a metaagent run should expose read-only context tools"
    );
    assert!(
        specs
            .iter()
            .all(|spec| spec.name.starts_with("arroba.meta.")
                || spec.name == crate::transport::runtime_tools::LIST_SESSION_AGENTS_TOOL
                || spec.name == crate::transport::runtime_tools::GET_SESSION_AGENT_TOOL
                || spec.name == crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL
                || spec.name == crate::transport::runtime_tools::READ_ARTIFACT_TOOL
                || spec.name == crate::transport::runtime_tools::SEARCH_RECALL_TOOL
                || spec.name == crate::transport::runtime_tools::QUERY_RECALL_TOOL),
        "shared token with a metaagent run should expose only meta, agent collaboration, read-only workspace, and recall tools: {specs:?}"
    );

    let overview = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &shared_auth_token,
            crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL,
            serde_json::json!({}),
        )
        .await
        .expect("shared meta token should dispatch meta tools");
    assert!(overview.ok, "{:?}", overview.payload);
    assert_eq!(
        overview
            .payload
            .get("metaagent")
            .and_then(|value| value.get("id"))
            .and_then(serde_json::Value::as_str),
        Some(metaagent.id())
    );

    let denied_direct_tool = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &shared_auth_token,
            crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL,
            serde_json::json!({ "path": "README.md", "content_text": "nope" }),
        )
        .await
        .expect("shared meta token mutation tools should return structured denials");
    assert!(!denied_direct_tool.ok, "{:?}", denied_direct_tool.payload);
}

#[tokio::test]
async fn forwarded_remote_metaagent_runtime_tools_use_home_scope() {
    let env = TestMetaRuntimeEnv::new("forwarded-remote");
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
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("remote-meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    let metaagent = app
        .agents()
        .bind_remote_execution(
            metaagent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel".to_string(),
                worker_machine_id: "worker-machine".to_string(),
                execution_lease_id: "lease-1".to_string(),
                leased_agent_id: "leased-agent-1".to_string(),
                active_worker_provider_run_id: Some("worker-run-1".to_string()),
                relay_url: None,
                relay_token: None,
            },
        )
        .expect("metaagent should be remote-backed");
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let context = crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext {
        home_kernel_id: "home-kernel".to_string(),
        home_session_id: session.id().to_string(),
        home_agent_id: metaagent.id().to_string(),
        leased_agent_id: "leased-agent-1".to_string(),
        worker_kernel_id: "worker-kernel".to_string(),
        worker_machine_id: "worker-machine".to_string(),
        worker_provider_run_id: "worker-run-1".to_string(),
        worker_worktree_path: workspace.to_string_lossy().to_string(),
        worker_workspace_identity: crate::io::WorkspaceIdentity::local(
            workspace.to_string_lossy().to_string(),
        ),
    };

    let overview = router
        .dispatch_forwarded_meta_runtime_tool_call(
            context.clone(),
            crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL.to_string(),
            serde_json::json!({}),
        )
        .await
        .expect("forwarded overview should dispatch home-side");
    assert!(overview.ok, "{overview:?}");
    assert_eq!(
        overview
            .payload
            .pointer("/metaagent/id")
            .and_then(serde_json::Value::as_str),
        Some(metaagent.id())
    );

    let command = router
        .dispatch_forwarded_meta_runtime_tool_call(
            context,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL.to_string(),
            serde_json::json!({ "command": "agent list" }),
        )
        .await
        .expect("forwarded run_command should dispatch through the router");
    assert!(command.ok, "{command:?}");
}

#[tokio::test]
async fn forwarded_remote_metaagent_runtime_tools_reject_forged_worker_context() {
    let env = TestMetaRuntimeEnv::new("forwarded-remote-forgery");
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
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("remote-meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    let metaagent = app
        .agents()
        .bind_remote_execution(
            metaagent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel".to_string(),
                worker_machine_id: "worker-machine".to_string(),
                execution_lease_id: "lease-1".to_string(),
                leased_agent_id: "leased-agent-1".to_string(),
                active_worker_provider_run_id: Some("worker-run-1".to_string()),
                relay_url: None,
                relay_token: None,
            },
        )
        .expect("metaagent should be remote-backed");
    let regular_remote = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("remote-worker"))
        .expect("regular remote agent should spawn");
    let regular_remote = app
        .agents()
        .bind_remote_execution(
            regular_remote.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel".to_string(),
                worker_machine_id: "worker-machine".to_string(),
                execution_lease_id: "lease-2".to_string(),
                leased_agent_id: "leased-agent-2".to_string(),
                active_worker_provider_run_id: Some("worker-run-2".to_string()),
                relay_url: None,
                relay_token: None,
            },
        )
        .expect("regular agent should be remote-backed");
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let context = crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext {
        home_kernel_id: "home-kernel".to_string(),
        home_session_id: session.id().to_string(),
        home_agent_id: metaagent.id().to_string(),
        leased_agent_id: "leased-agent-1".to_string(),
        worker_kernel_id: "worker-kernel".to_string(),
        worker_machine_id: "worker-machine".to_string(),
        worker_provider_run_id: "worker-run-1".to_string(),
        worker_worktree_path: workspace.to_string_lossy().to_string(),
        worker_workspace_identity: crate::io::WorkspaceIdentity::local(
            workspace.to_string_lossy().to_string(),
        ),
    };

    let mut wrong_lease = context.clone();
    wrong_lease.leased_agent_id = "leased-agent-forged".to_string();
    let lease_denied = router
        .dispatch_forwarded_meta_runtime_tool_call(
            wrong_lease,
            crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL.to_string(),
            serde_json::json!({}),
        )
        .await
        .expect_err("mismatched leased agent should be rejected home-side");
    assert!(
        lease_denied
            .to_string()
            .contains("forwarded metaagent context does not match"),
        "{lease_denied:?}"
    );

    let mut wrong_worker = context.clone();
    wrong_worker.worker_kernel_id = "worker-kernel-forged".to_string();
    let worker_denied = router
        .dispatch_forwarded_meta_runtime_tool_call(
            wrong_worker,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL.to_string(),
            serde_json::json!({ "command": "agent list" }),
        )
        .await
        .expect_err("mismatched worker kernel should be rejected home-side");
    assert!(
        worker_denied
            .to_string()
            .contains("forwarded metaagent context does not match"),
        "{worker_denied:?}"
    );

    let mut regular_context = context;
    regular_context.home_agent_id = regular_remote.id().to_string();
    regular_context.leased_agent_id = "leased-agent-2".to_string();
    regular_context.worker_provider_run_id = "worker-run-2".to_string();
    let regular_denied = router
        .dispatch_forwarded_meta_runtime_tool_call(
            regular_context,
            crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL.to_string(),
            serde_json::json!({}),
        )
        .await
        .expect_err("regular remote agents should not get forwarded meta tools");
    assert!(
        regular_denied
            .to_string()
            .contains("agents currently in Meta mode"),
        "{regular_denied:?}"
    );
}
