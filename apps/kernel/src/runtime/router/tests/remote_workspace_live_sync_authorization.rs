use super::*;

#[tokio::test]
async fn remote_workspace_live_sync_requests_require_membership_and_record_member_identity() {
    let denied_app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let denied_session = denied_app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-live-sync-auth",
            "/tmp/workspace-live-sync-auth-main",
        ))
        .expect("session should be created");
    let denied_session_id = denied_session.id().to_string();
    let denied_app = Arc::new(Mutex::new(denied_app));
    let denied_router = CommandRouter::with_interactive_capacity(Arc::clone(&denied_app), 2);

    let status_request =
        LocalDaemonRequest::GetWorkspaceLiveSyncStatus(GetWorkspaceLiveSyncStatusRequest {
            session_id: denied_session_id.clone(),
        });
    let denied = denied_router
        .dispatch(
            remote_command_for_request(&status_request, Some("user-2")),
            status_request,
        )
        .await
        .expect_err("non-member live sync status should be rejected");
    assert!(matches!(
        denied,
        DaemonError::SessionAccessDenied {
            session_id: ref denied_session,
            user_id: ref denied_user,
        } if denied_session == &denied_session_id && denied_user == "user-2"
    ));

    let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-live-sync-auth",
            "/tmp/workspace-live-sync-auth-main",
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let (_, invite) = app
        .sessions_mut()
        .create_session_invite(
            &session_id,
            "invite-user-2".to_string(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
            Some(1),
            crate::session::CollaborationLevel::Private,
        )
        .expect("invite should be created");
    app.sessions_mut()
        .join_session_invite(&session_id, invite.invite_id(), "user-2".to_string(), 1)
        .expect("user should join session");
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 2);

    let create_request = LocalDaemonRequest::CreateWorkspaceLink(CreateWorkspaceLinkRequest {
        session_id: session_id.clone(),
        name: "team-sync".to_string(),
    });
    let link = match router
        .dispatch(
            remote_command_for_request(&create_request, Some("user-2")),
            create_request,
        )
        .await
        .expect("member should create workspace link")
    {
        LocalDaemonResponse::WorkspaceLinkCreated { link, .. } => link,
        other => panic!("unexpected create response: {other:?}"),
    };
    assert_eq!(link.created_by_user_id(), "user-2");

    let user_2_worktree = create_test_git_worktree("workspace-live-sync-auth-user-2");
    let user_2_worktree_id = user_2_worktree.to_string_lossy().to_string();
    let attach_request = LocalDaemonRequest::AttachWorkspaceLink(AttachWorkspaceLinkRequest {
        session_id: session_id.clone(),
        link_ref: "team-sync".to_string(),
        repo_root: Some(user_2_worktree_id.clone()),
        branch: Some("tracked-peer".to_string()),
        repo_fingerprint: Some("repo-fingerprint-user-2".to_string()),
    });
    let attachment = match router
        .dispatch(
            remote_command_for_request(&attach_request, Some("user-2")),
            attach_request,
        )
        .await
        .expect("member should attach workspace live sync target")
    {
        LocalDaemonResponse::WorkspaceLinkAttached { attachment, .. } => attachment,
        other => panic!("unexpected attach response: {other:?}"),
    };
    assert_eq!(attachment.user_id(), "user-2");
    assert_eq!(attachment.repo_root(), user_2_worktree_id);
    assert_eq!(attachment.branch(), Some("tracked-peer"));
    assert_eq!(
        attachment.repo_fingerprint(),
        Some("repo-fingerprint-user-2")
    );

    let status_request =
        LocalDaemonRequest::GetWorkspaceLiveSyncStatus(GetWorkspaceLiveSyncStatusRequest {
            session_id: session_id.clone(),
        });
    let status = match router
        .dispatch(
            remote_command_for_request(&status_request, Some("user-2")),
            status_request,
        )
        .await
        .expect("member live sync status should succeed")
    {
        LocalDaemonResponse::WorkspaceLiveSyncStatus { status } => status,
        other => panic!("unexpected status response: {other:?}"),
    };
    assert_eq!(status.sync_groups.len(), 1);
    assert_eq!(status.sync_groups[0].target_count, 1);
    assert_eq!(status.targets.len(), 1);
    assert_eq!(status.targets[0].user_id, "user-2");
    assert_eq!(status.targets[0].repo_root, user_2_worktree_id);
    assert_eq!(status.targets[0].branch.as_deref(), Some("tracked-peer"));
    let _ = std::fs::remove_dir_all(user_2_worktree);
}

#[tokio::test]
async fn forwarded_workspace_live_sync_invocation_replays_completed_mutation_once() {
    let worktree = create_test_git_worktree("workspace-live-sync-replay");
    let src_dir = worktree.join("src");
    std::fs::create_dir_all(&src_dir).expect("src fixture should be created");
    let file_path = src_dir.join("lib.rs");
    std::fs::write(&file_path, "before\n").expect("fixture should be written");

    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-live-sync-replay",
            worktree.to_string_lossy(),
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent = spawn_test_agent(&mut app, &session_id, "sync-agent", "codex");
    let agent_id = agent.id().to_string();
    focus_test_agent(&mut app, &session_id, &agent_id);
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 2);

    let metadata = crate::transport::relay_peer::RemoteWorkspaceLiveSyncInvocationMetadata {
        invocation_id: "workspace-live-sync-replay-1".to_string(),
        provider_tool_call_id: Some("provider-call-1".to_string()),
        attempt: 1,
        idempotency_key: None,
    };
    let context = remote_workspace_live_sync_context(&session_id, &agent_id, &worktree);
    let arguments = serde_json::json!({
        "patch_text": "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n-before\n+after\n*** End Patch",
        "domain": "text"
    });
    let initial_states = vec![
        crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState {
            path: "src/lib.rs".to_string(),
            exists: true,
            domain: Some("text".to_string()),
            content_text: Some("before\n".to_string()),
            content_base64: None,
        },
    ];

    let (first_result, final_states) = router
        .dispatch_forwarded_workspace_live_sync_runtime_tool_call(
            context.clone(),
            metadata.clone(),
            crate::transport::runtime_tools::APPLY_PATCH_TOOL.to_string(),
            arguments.clone(),
            initial_states.clone(),
        )
        .await
        .expect("first forwarded live sync mutation should run");
    assert!(first_result.ok, "first result: {first_result:?}");
    assert_eq!(
        final_states
            .iter()
            .find(|state| state.path == "src/lib.rs")
            .and_then(|state| state.content_text.as_deref()),
        Some("after\n")
    );

    let (replayed_result, replayed_final_states) = router
        .dispatch_forwarded_workspace_live_sync_runtime_tool_call(
            context.clone(),
            metadata.clone(),
            crate::transport::runtime_tools::APPLY_PATCH_TOOL.to_string(),
            arguments.clone(),
            initial_states.clone(),
        )
        .await
        .expect("duplicate forwarded live sync mutation should replay");
    assert_eq!(replayed_result, first_result);
    assert_eq!(replayed_final_states, final_states);

    let (rejected_result, rejected_states) = router
        .dispatch_forwarded_workspace_live_sync_runtime_tool_call(
            context.clone(),
            metadata.clone(),
            crate::transport::runtime_tools::APPLY_PATCH_TOOL.to_string(),
            serde_json::json!({
                "patch_text": "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n-after\n+changed\n*** End Patch",
                "domain": "text"
            }),
            initial_states.clone(),
        )
        .await
        .expect("changed duplicate should be rejected as a tool result");
    assert!(!rejected_result.ok);
    assert_eq!(
        rejected_result
            .payload
            .pointer("/reason/kind")
            .and_then(|value| value.as_str()),
        Some("duplicate_remote_workspace_live_sync_invocation")
    );
    assert!(rejected_states.is_empty());

    router
        .finalize_forwarded_workspace_live_sync_runtime_tool_call(
            context.clone(),
            metadata.clone(),
            crate::transport::runtime_tools::APPLY_PATCH_TOOL.to_string(),
            arguments.clone(),
            initial_states.clone(),
            final_states.clone(),
        )
        .await
        .expect("first finalize should succeed");
    router
        .finalize_forwarded_workspace_live_sync_runtime_tool_call(
            context,
            metadata,
            crate::transport::runtime_tools::APPLY_PATCH_TOOL.to_string(),
            arguments,
            initial_states,
            final_states,
        )
        .await
        .expect("duplicate finalize should be a no-op");

    let _ = std::fs::remove_dir_all(worktree);
}

#[tokio::test]
async fn forwarded_workspace_live_sync_retry_waits_for_inflight_permission_result() {
    let worktree = create_test_git_worktree("workspace-live-sync-inflight-permission");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-live-sync-inflight-permission",
            worktree.to_string_lossy(),
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(&session_id, "codex")
                .with_alias("sync-agent")
                .with_permission_level_override(crate::provider::AgentPermissionLevel::Required),
        )
        .expect("required-permission agent should spawn");
    let agent_id = agent.id().to_string();
    focus_test_agent(&mut app, &session_id, &agent_id);
    let app = Arc::new(Mutex::new(app));
    let router = Arc::new(CommandRouter::with_interactive_capacity(
        Arc::clone(&app),
        4,
    ));
    let metadata = crate::transport::relay_peer::RemoteWorkspaceLiveSyncInvocationMetadata {
        invocation_id: "workspace-live-sync-inflight-permission-1".to_string(),
        provider_tool_call_id: Some("provider-call-permission-1".to_string()),
        attempt: 1,
        idempotency_key: None,
    };
    let context = remote_workspace_live_sync_context(&session_id, &agent_id, &worktree);
    let arguments = serde_json::json!({
        "path": "permission.txt",
        "content_text": "approved\n",
        "domain": "text"
    });
    let initial_states = vec![
        crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState {
            path: "permission.txt".to_string(),
            exists: false,
            domain: Some("text".to_string()),
            content_text: None,
            content_base64: None,
        },
    ];

    let first = tokio::spawn({
        let router = Arc::clone(&router);
        let context = context.clone();
        let metadata = metadata.clone();
        let arguments = arguments.clone();
        let initial_states = initial_states.clone();
        async move {
            router
                .dispatch_forwarded_workspace_live_sync_runtime_tool_call(
                    context,
                    metadata,
                    crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL.to_string(),
                    arguments,
                    initial_states,
                )
                .await
        }
    });
    let interaction_id = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(interaction_id) = {
                let app = app.lock().await;
                app.sessions()
                    .get_session(&session_id)
                    .expect("session should remain available")
                    .active_interaction_for_agent(&agent_id)
                    .map(|interaction| interaction.id().to_string())
            } {
                return interaction_id;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("forwarded write should project a permission interaction");

    let duplicate = tokio::spawn({
        let router = Arc::clone(&router);
        let context = context.clone();
        let mut metadata = metadata.clone();
        metadata.attempt = 2;
        let arguments = arguments.clone();
        let initial_states = initial_states.clone();
        async move {
            router
                .dispatch_forwarded_workspace_live_sync_runtime_tool_call(
                    context,
                    metadata,
                    crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL.to_string(),
                    arguments,
                    initial_states,
                )
                .await
        }
    });
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert!(
        !duplicate.is_finished(),
        "a transport retry should wait for the original permission result"
    );

    let response =
        LocalDaemonRequest::RespondToInteraction(crate::local::RespondToInteractionRequest {
            session_id: session_id.clone(),
            interaction_id,
            choice_id: "allow".to_string(),
            custom_reply: None,
        });
    router
        .dispatch(
            KernelCommand::from_local_request(
                "workspace-live-sync-inflight-permission-allow",
                None,
                None,
                &response,
            ),
            response,
        )
        .await
        .expect("permission response should succeed");

    let first_result = tokio::time::timeout(Duration::from_secs(2), first)
        .await
        .expect("original invocation should finish")
        .expect("original invocation task should not panic")
        .expect("original invocation should succeed");
    let duplicate_result = tokio::time::timeout(Duration::from_secs(2), duplicate)
        .await
        .expect("retry invocation should finish")
        .expect("retry invocation task should not panic")
        .expect("retry invocation should succeed");
    assert!(first_result.0.ok, "original result: {first_result:?}");
    assert_eq!(duplicate_result, first_result);

    let _ = std::fs::remove_dir_all(worktree);
}

fn remote_workspace_live_sync_context(
    session_id: &str,
    agent_id: &str,
    worktree: &std::path::Path,
) -> crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext {
    let fingerprint = worktree
        .canonicalize()
        .expect("worktree should canonicalize")
        .to_string_lossy()
        .to_string();
    crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext {
        home_kernel_id: "home-kernel".to_string(),
        home_session_id: session_id.to_string(),
        home_agent_id: agent_id.to_string(),
        leased_agent_id: "leased-agent-1".to_string(),
        worker_kernel_id: "worker-kernel".to_string(),
        worker_machine_id: "worker-machine".to_string(),
        worker_provider_run_id: "worker-provider-run".to_string(),
        worker_worktree_path: worktree.to_string_lossy().to_string(),
        worker_workspace_identity: crate::io::WorkspaceIdentity::local(fingerprint),
    }
}

fn create_test_git_worktree(label: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "chariox-{label}-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&root).expect("temp repo should be created");
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "agent@example.com"]);
    run_git(&root, &["config", "user.name", "Agent"]);
    run_git(&root, &["checkout", "-b", "tracked-peer"]);
    root
}

fn run_git(cwd: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .expect("git should run");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}
