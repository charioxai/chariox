use super::*;

struct TestWorkspace(std::path::PathBuf);

impl TestWorkspace {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-remote-message-origin-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).expect("test worktree should be created");
        Self(root)
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn forwarded_agent_message_requires_current_home_sender_prompt() {
    let workspace = TestWorkspace::new();
    let workspace_path = workspace.0.to_string_lossy().to_string();
    let config = DaemonConfig::for_tests();
    let home_kernel_id = config.daemon_id.clone();
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let (session, sender) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(&workspace_path, &workspace_path))
        .expect("session should be created");
    let target = spawn_test_agent(&mut app, session.id(), "stopped-target", "dev-stub");
    let late_target = spawn_test_agent(&mut app, session.id(), "stopped-after-turn", "dev-stub");
    app.agents()
        .bind_remote_execution(
            sender.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel".to_string(),
                worker_machine_id: "worker-machine".to_string(),
                execution_lease_id: "lease-1".to_string(),
                leased_agent_id: "leased-agent-1".to_string(),
                active_worker_provider_run_id: Some("worker-run-1".to_string()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .expect("sender should be remote-backed");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "remote-message-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("test client should attach");
    let active = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        sender.id(),
        "active sender turn",
        crate::session::PromptStatus::Queued,
    );
    let home_prompt_id = match app
        .prompt_owner_submit_prepared_prompt(session.id(), active, false)
        .expect("sender turn should become active")
    {
        crate::session::PromptSubmissionOutcome::Started { prompt } => prompt.id().to_string(),
        other => panic!("sender turn should start: {other:?}"),
    };
    let context = crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext {
        home_kernel_id,
        home_session_id: session.id().to_string(),
        home_agent_id: sender.id().to_string(),
        home_prompt_id: Some("settled-home-prompt".to_string()),
        leased_agent_id: "leased-agent-1".to_string(),
        worker_kernel_id: "worker-kernel".to_string(),
        worker_machine_id: "worker-machine".to_string(),
        worker_provider_run_id: "worker-run-1".to_string(),
        worker_worktree_path: workspace_path.clone(),
        worker_workspace_identity: crate::io::WorkspaceIdentity::local(workspace_path),
    };
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let tool = crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL.to_string();
    let args = serde_json::json!({
        "agent": target.id(),
        "message": "Check the task.",
        "origin_prompt_id": home_prompt_id,
    });

    for stale in [
        context.clone(),
        crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext {
            home_prompt_id: None,
            ..context.clone()
        },
        crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext {
            home_prompt_id: Some(home_prompt_id.clone()),
            leased_agent_id: "old-lease".to_string(),
            ..context.clone()
        },
        crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext {
            home_prompt_id: Some(home_prompt_id.clone()),
            worker_provider_run_id: "old-run".to_string(),
            ..context.clone()
        },
    ] {
        let denied = router
            .dispatch_forwarded_capability_runtime_tool_call(stale, tool.clone(), args.clone())
            .await;
        assert!(
            denied.is_err(),
            "stale sender must not dispatch: {denied:?}"
        );
        let snapshot = router
            .runtime_state
            .session_snapshot(session.id())
            .await
            .unwrap();
        assert!(snapshot.active_prompt_for_agent(target.id()).is_none());
    }

    let alias_denied = router
        .dispatch_forwarded_capability_runtime_tool_call(
            context.clone(),
            "mcp__chariox__send_agent_message".to_string(),
            args.clone(),
        )
        .await
        .expect_err("the MCP alias must use the same sender origin check");
    assert!(alias_denied
        .to_string()
        .contains("sender prompt or leased worker binding is no longer current"));

    let valid_context = crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext {
        home_prompt_id: Some(home_prompt_id.clone()),
        ..context
    };
    let valid = router
        .dispatch_forwarded_capability_runtime_tool_call(valid_context.clone(), tool.clone(), args)
        .await
        .expect("current leased sender may message an idle recipient");
    assert!(valid.0.ok, "{:?}", valid.0.payload);
    assert_eq!(valid.0.payload["status"], "started");
    assert!(router
        .runtime_state
        .session_snapshot(session.id())
        .await
        .unwrap()
        .active_prompt_for_agent(target.id())
        .is_some());

    app.lock()
        .await
        .prompt_owner_complete_active_prompt_only(session.id(), sender.id())
        .expect("sender turn should settle");
    let settled = router
        .dispatch_forwarded_capability_runtime_tool_call(
            valid_context,
            tool,
            serde_json::json!({
                "agent": late_target.id(),
                "message": "Late message.",
                "origin_prompt_id": home_prompt_id,
            }),
        )
        .await
        .expect_err("a settled sender turn must not restart an idle agent");
    assert!(settled
        .to_string()
        .contains("sender prompt or leased worker binding is no longer current"));
    assert!(router
        .runtime_state
        .session_snapshot(session.id())
        .await
        .unwrap()
        .active_prompt_for_agent(late_target.id())
        .is_none());
}
