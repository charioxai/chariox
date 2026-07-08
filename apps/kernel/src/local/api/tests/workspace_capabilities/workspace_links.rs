use super::*;

#[test]
fn local_request_api_manages_session_workspace_links() {
    run_workspace_capability_test(
        "local_request_api_manages_session_workspace_links",
        local_request_api_manages_session_workspace_links_inner,
    );
}

fn local_request_api_manages_session_workspace_links_inner() {
    let harness = LocalRouterTestHarness::new();
    let worktree = create_test_git_worktree("workspace-links");
    let worktree_id = worktree.to_string_lossy().to_string();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", &worktree_id),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let session_id = session.id().to_string();
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session_id.clone(),
                client_id: "client-workspace-link".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let denied = harness.dispatch_as_user(
        "stranger",
        LocalDaemonRequest::ListWorkspaceLinks(ListWorkspaceLinksRequest {
            session_id: session_id.clone(),
        }),
    );
    assert!(matches!(
        denied,
        Err(DaemonError::SessionAccessDenied { .. })
    ));

    let link = match harness
        .dispatch(LocalDaemonRequest::CreateWorkspaceLink(
            CreateWorkspaceLinkRequest {
                session_id: session_id.clone(),
                name: "shared-repo".to_string(),
            },
        ))
        .expect("workspace link create should succeed")
    {
        LocalDaemonResponse::WorkspaceLinkCreated { link, session } => {
            assert_eq!(session.workspace_links().len(), 1);
            link
        }
        _ => panic!("unexpected local response"),
    };
    assert_eq!(link.name(), "shared-repo");

    harness
        .dispatch(LocalDaemonRequest::SetWorkspaceLiveSyncMode(
            SetWorkspaceLiveSyncModeRequest {
                session_id: session_id.clone(),
                mode: crate::config::WorkspaceLiveSyncMode::Unrestricted,
            },
        ))
        .expect("workspace live sync mode should switch to unrestricted");

    let attached = match harness
        .dispatch(LocalDaemonRequest::AttachWorkspaceLink(
            AttachWorkspaceLinkRequest {
                session_id: session_id.clone(),
                link_ref: "shared".to_string(),
                repo_root: Some(worktree_id.clone()),
                branch: Some("main".to_string()),
                repo_fingerprint: Some("fingerprint-a".to_string()),
            },
        ))
        .expect("workspace link attach should succeed")
    {
        LocalDaemonResponse::WorkspaceLinkAttached {
            link,
            attachment,
            session,
        } => {
            assert_eq!(session.workspace_links()[0].attachments().len(), 1);
            assert_eq!(attachment.repo_root(), worktree_id);
            link
        }
        _ => panic!("unexpected local response"),
    };
    assert_eq!(attached.attachments().len(), 1);
    let notices = match harness
        .dispatch(LocalDaemonRequest::PollRuntimeNotices(
            PollRuntimeNoticesRequest {
                session_id: session_id.clone(),
                attachment_id: attachment.id().to_string(),
            },
        ))
        .expect("notice polling should succeed")
    {
        LocalDaemonResponse::RuntimeNotices { notices } => notices,
        _ => panic!("unexpected local response"),
    };
    let enrollment_notice = notices
        .iter()
        .find(|notice| {
            notice
                .message
                .contains("Workspace live sync link `shared-repo`")
        })
        .expect("workspace link attach should emit an enrollment notice");
    assert!(enrollment_notice.message.contains(
        "Mode choice: managed requires provider write fencing; tracked syncs at turn end"
    ));
    assert!(
        enrollment_notice
            .message
            .contains("live sync mode is unchanged")
    );
    assert!(enrollment_notice.message.contains("workspace sync managed"));
    assert!(enrollment_notice.message.contains("workspace sync tracked"));

    let status = match harness
        .dispatch(LocalDaemonRequest::GetWorkspaceLiveSyncStatus(
            GetWorkspaceLiveSyncStatusRequest {
                session_id: session_id.clone(),
            },
        ))
        .expect("workspace live sync status should succeed")
    {
        LocalDaemonResponse::WorkspaceLiveSyncStatus { status } => status,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(status.session_id, session_id);
    assert_eq!(status.sync_groups.len(), 1);
    assert_eq!(status.sync_groups[0].group_name, "shared-repo");
    assert_eq!(status.sync_groups[0].target_count, 1);
    assert_eq!(status.targets.len(), 1);
    assert_eq!(status.targets[0].repo_root, worktree_id);
    assert!(status.conflicts.is_empty());
    for pattern in [
        ".git/**",
        ".arroba/**",
        ".arrobaignore",
        ".env*",
        ".codex/**",
        ".opencode/**",
        ".claude/**",
        ".cursor/**",
        "*.sock",
        "*.socket",
        ".tmp-arroba/**",
        ".tmp-live-workspace-live-sync-drill/**",
        ".tmp-live-remote-workspace-live-sync-drill/**",
        "history/**",
        "session-history/**",
        "operational-history/**",
        "operational-history*",
        "node_modules/**",
        "target/**",
        ".cache/**",
        ".turbo/**",
        ".next/**",
        "dist/**",
        "build/**",
        ".venv/**",
        "venv/**",
        "__pycache__/**",
        ".pytest_cache/**",
        ".mypy_cache/**",
        ".ruff_cache/**",
        ".gradle/**",
        ".m2/**",
        ".pnpm-store/**",
    ] {
        assert!(
            status
                .ignore
                .force_excludes
                .iter()
                .any(|force_exclude| force_exclude == pattern),
            "{pattern} should be advertised as a workspace live sync force-exclude"
        );
    }

    let shown = match harness
        .dispatch(LocalDaemonRequest::ShowWorkspaceLink(
            ShowWorkspaceLinkRequest {
                session_id: session_id.clone(),
                link_ref: link.link_id().to_string(),
            },
        ))
        .expect("workspace link show should succeed")
    {
        LocalDaemonResponse::WorkspaceLinkShown { link } => link,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(shown.attachments().len(), 1);

    let listed = match harness
        .dispatch(LocalDaemonRequest::ListWorkspaceLinks(
            ListWorkspaceLinksRequest {
                session_id: session_id.clone(),
            },
        ))
        .expect("workspace links list should succeed")
    {
        LocalDaemonResponse::WorkspaceLinksListed { links } => links,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(listed.len(), 1);

    let detached = match harness
        .dispatch(LocalDaemonRequest::DetachWorkspaceLink(
            DetachWorkspaceLinkRequest {
                session_id,
                link_ref: "shared-repo".to_string(),
                repo_root: Some(worktree_id),
            },
        ))
        .expect("workspace link detach should succeed")
    {
        LocalDaemonResponse::WorkspaceLinkDetached { link, detached, .. } => {
            assert!(link.attachments().is_empty());
            detached
        }
        _ => panic!("unexpected local response"),
    };
    assert_eq!(detached.len(), 1);
    let _ = std::fs::remove_dir_all(worktree);
}

#[test]
fn workspace_link_mutations_preserve_spawned_agents_in_session_projection() {
    run_workspace_capability_test(
        "workspace_link_mutations_preserve_spawned_agents_in_session_projection",
        workspace_link_mutations_preserve_spawned_agents_in_session_projection_inner,
    );
}

fn workspace_link_mutations_preserve_spawned_agents_in_session_projection_inner() {
    let harness = LocalRouterTestHarness::new();
    let worktree = create_test_git_worktree("workspace-link-session-projection");
    let worktree_id = worktree.to_string_lossy().to_string();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", &worktree_id),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let session_id = session.id().to_string();

    let spawned = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session_id.clone(),
            alias: Some("reviewer".to_string()),
            provider: Some("codex".to_string()),
            model: Some("gpt-5.2".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }))
        .expect("agent spawn should succeed")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    let created_session = match harness
        .dispatch(LocalDaemonRequest::CreateWorkspaceLink(
            CreateWorkspaceLinkRequest {
                session_id: session_id.clone(),
                name: "shared-repo".to_string(),
            },
        ))
        .expect("workspace link create should succeed")
    {
        LocalDaemonResponse::WorkspaceLinkCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(created_session.agents().len(), 2);
    assert!(
        created_session
            .agents()
            .iter()
            .any(|agent| agent.id() == spawned.id())
    );

    let attached_session = match harness
        .dispatch(LocalDaemonRequest::AttachWorkspaceLink(
            AttachWorkspaceLinkRequest {
                session_id: session_id.clone(),
                link_ref: "shared".to_string(),
                repo_root: Some(worktree_id),
                branch: Some("main".to_string()),
                repo_fingerprint: Some("fingerprint-a".to_string()),
            },
        ))
        .expect("workspace link attach should succeed")
    {
        LocalDaemonResponse::WorkspaceLinkAttached { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(attached_session.agents().len(), 2);
    assert!(
        attached_session
            .agents()
            .iter()
            .any(|agent| agent.id() == spawned.id())
    );

    let session_state = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest { session_id },
        ))
        .expect("session state should load")
    {
        LocalDaemonResponse::SessionState { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(session_state.agents().len(), 2);
    let _ = std::fs::remove_dir_all(worktree);
}

#[test]
fn attach_workspace_link_rejects_non_git_worktree_targets() {
    run_workspace_capability_test(
        "attach_workspace_link_rejects_non_git_worktree_targets",
        attach_workspace_link_rejects_non_git_worktree_targets_inner,
    );
}

fn attach_workspace_link_rejects_non_git_worktree_targets_inner() {
    let root = std::env::temp_dir().join(format!(
        "arroba-workspace-link-invalid-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&root).expect("temp directory should be created");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", root.to_string_lossy()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    harness
        .dispatch(LocalDaemonRequest::CreateWorkspaceLink(
            CreateWorkspaceLinkRequest {
                session_id: session.id().to_string(),
                name: "shared-repo".to_string(),
            },
        ))
        .expect("workspace link create should succeed");

    let denied = harness.dispatch(LocalDaemonRequest::AttachWorkspaceLink(
        AttachWorkspaceLinkRequest {
            session_id: session.id().to_string(),
            link_ref: "shared".to_string(),
            repo_root: Some(root.to_string_lossy().to_string()),
            branch: Some("main".to_string()),
            repo_fingerprint: Some("fingerprint-a".to_string()),
        },
    ));

    assert!(matches!(denied, Err(DaemonError::LocalTransport { .. })));
    assert!(
        denied
            .expect_err("non-git worktree attach should fail")
            .to_string()
            .contains("must be a Git worktree root")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn attach_workspace_link_infers_git_identity_when_missing() {
    run_workspace_capability_test(
        "attach_workspace_link_infers_git_identity_when_missing",
        attach_workspace_link_infers_git_identity_when_missing_inner,
    );
}

fn attach_workspace_link_infers_git_identity_when_missing_inner() {
    let root = std::env::temp_dir().join(format!(
        "arroba-workspace-link-identity-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&root).expect("temp repo should be created");
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "agent@example.com"]);
    run_git(&root, &["config", "user.name", "Agent"]);
    std::fs::write(root.join("README.md"), "seed\n").expect("seed should write");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "seed"]);
    run_git(&root, &["checkout", "-b", "sync-main"]);

    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(root.to_string_lossy(), root.to_string_lossy()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let session_id = session.id().to_string();
    harness
        .dispatch(LocalDaemonRequest::CreateWorkspaceLink(
            CreateWorkspaceLinkRequest {
                session_id: session_id.clone(),
                name: "shared-repo".to_string(),
            },
        ))
        .expect("workspace link create should succeed");

    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachWorkspaceLink(
            AttachWorkspaceLinkRequest {
                session_id,
                link_ref: "shared".to_string(),
                repo_root: Some(root.to_string_lossy().to_string()),
                branch: None,
                repo_fingerprint: None,
            },
        ))
        .expect("workspace link attach should succeed")
    {
        LocalDaemonResponse::WorkspaceLinkAttached { attachment, .. } => attachment,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(attachment.branch(), Some("sync-main"));
    assert!(attachment.repo_fingerprint().is_some());

    let _ = std::fs::remove_dir_all(&root);
}
