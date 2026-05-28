use super::*;

#[test]
fn local_request_api_sets_workspace_live_sync_mode_through_dedicated_request() {
    let harness = LocalRouterTestHarness::new();

    let updated = match harness
        .dispatch(LocalDaemonRequest::SetWorkspaceLiveSyncMode(
            SetWorkspaceLiveSyncModeRequest {
                mode: crate::config::WorkspaceLiveSyncMode::Tracked,
            },
        ))
        .expect("workspace live sync mode update should succeed")
    {
        LocalDaemonResponse::UserConfigUpdated {
            config, effects, ..
        } => {
            assert_eq!(
                effects.first().map(|effect| effect.path.as_str()),
                Some("providers.workspace_live_sync")
            );
            config
        }
        _ => panic!("unexpected local response"),
    };

    assert_eq!(
        updated.providers.workspace_live_sync.mode,
        crate::config::WorkspaceLiveSyncMode::Tracked
    );
}

#[test]
fn local_request_api_manages_session_workspace_links() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "/tmp/arroba-worktree-a"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let session_id = session.id().to_string();

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

    let attached = match harness
        .dispatch(LocalDaemonRequest::AttachWorkspaceLink(
            AttachWorkspaceLinkRequest {
                session_id: session_id.clone(),
                link_ref: "shared".to_string(),
                repo_root: Some("/tmp/arroba-worktree-a".to_string()),
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
            assert_eq!(attachment.repo_root(), "/tmp/arroba-worktree-a");
            link
        }
        _ => panic!("unexpected local response"),
    };
    assert_eq!(attached.attachments().len(), 1);

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
    assert_eq!(status.targets.len(), 1);
    assert_eq!(status.targets[0].repo_root, "/tmp/arroba-worktree-a");
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
                repo_root: Some("/tmp/arroba-worktree-a".to_string()),
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
}

#[test]
fn workspace_link_mutations_preserve_spawned_agents_in_session_projection() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "/tmp/arroba-worktree-a"),
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
    assert!(created_session
        .agents()
        .iter()
        .any(|agent| agent.id() == spawned.id()));

    let attached_session = match harness
        .dispatch(LocalDaemonRequest::AttachWorkspaceLink(
            AttachWorkspaceLinkRequest {
                session_id: session_id.clone(),
                link_ref: "shared".to_string(),
                repo_root: Some("/tmp/arroba-worktree-a".to_string()),
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
    assert!(attached_session
        .agents()
        .iter()
        .any(|agent| agent.id() == spawned.id()));

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
}

#[test]
fn attach_workspace_link_infers_git_identity_when_missing() {
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

#[test]
fn local_request_api_runs_shell_command_capability() {
    let worktree_root = std::env::temp_dir().join("arroba-shell-local-api-test");
    std::fs::create_dir_all(&worktree_root).expect("worktree dir should exist");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-shell".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let response = harness
        .dispatch(LocalDaemonRequest::RunShellCommand(
            RunShellCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                command: "/bin/sh".to_string(),
                args: vec!["-lc".to_string(), "printf capability".to_string()],
                working_directory: None,
                timeout_ms: None,
            },
        ))
        .expect("shell capability should succeed");

    match response {
        LocalDaemonResponse::ShellCommandCompleted { result } => {
            assert_eq!(result.exit_code, 0);
            assert_eq!(result.stdout, "capability");
        }
        _ => panic!("unexpected shell response"),
    }
}

#[test]
fn local_request_api_rejects_shell_command_for_unauthorized_attachment() {
    let worktree_root = std::env::temp_dir().join("arroba-shell-local-api-denied-test");
    std::fs::create_dir_all(&worktree_root).expect("worktree dir should exist");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-automation".to_string(),
                capability_level: ClientCapabilityLevel::AutomationOnly,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let error = harness
        .dispatch(LocalDaemonRequest::RunShellCommand(
            RunShellCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                command: "/bin/sh".to_string(),
                args: vec!["-lc".to_string(), "printf denied".to_string()],
                working_directory: None,
                timeout_ms: None,
            },
        ))
        .expect_err("automation-only attachment should not run shell commands");

    match error {
        DaemonError::AttachmentCapabilityDenied { session_id, .. } => {
            assert_eq!(session_id, session.id());
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn local_request_api_rejects_file_capability_for_unauthorized_attachment() {
    let worktree_root = std::env::temp_dir().join("arroba-file-local-api-denied-test");
    let _ = std::fs::remove_dir_all(&worktree_root);
    std::fs::create_dir_all(&worktree_root).expect("worktree dir should exist");
    std::fs::write(worktree_root.join("notes.txt"), "hello").expect("file should exist");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-automation".to_string(),
                capability_level: ClientCapabilityLevel::AutomationOnly,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let error = harness
        .dispatch(LocalDaemonRequest::ReadFile(ReadFileCapabilityRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            path: worktree_root.join("notes.txt"),
        }))
        .expect_err("automation-only attachment should not read files");

    match error {
        DaemonError::AttachmentCapabilityDenied { session_id, .. } => {
            assert_eq!(session_id, session.id());
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn local_request_api_reads_directory_tree_file_and_git_status() {
    let worktree_root = std::env::temp_dir().join("arroba-capability-local-api-test");
    let _ = std::fs::remove_dir_all(&worktree_root);
    std::fs::create_dir_all(worktree_root.join("src")).expect("worktree should exist");
    std::fs::write(worktree_root.join("README.md"), "hello").expect("file should exist");
    std::fs::write(worktree_root.join("src/lib.rs"), "before").expect("file should exist");
    std::process::Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(&worktree_root)
        .output()
        .expect("git init should work");

    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-capability".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let tree = harness
        .dispatch(LocalDaemonRequest::ReadDirectoryTree(
            ReadDirectoryTreeCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                path: None,
                max_depth: 2,
            },
        ))
        .expect("tree read should succeed");
    let file = harness
        .dispatch(LocalDaemonRequest::ReadFile(ReadFileCapabilityRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            path: worktree_root.join("src/lib.rs"),
        }))
        .expect("file read should succeed");
    let edit = harness
        .dispatch(LocalDaemonRequest::EditFile(EditFileCapabilityRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            path: worktree_root.join("src/lib.rs"),
            contents: "after".to_string(),
        }))
        .expect("file edit should succeed");
    let git = harness
        .dispatch(LocalDaemonRequest::InspectGit(
            InspectGitCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                working_directory: None,
            },
        ))
        .expect("git inspect should succeed");

    match tree {
        LocalDaemonResponse::DirectoryTreeRead { result } => {
            assert!(result
                .entries
                .iter()
                .any(|entry| entry.relative_path == "README.md"));
        }
        _ => panic!("unexpected tree response"),
    }
    match file {
        LocalDaemonResponse::FileRead { result } => assert_eq!(result.contents, "before"),
        _ => panic!("unexpected file response"),
    }
    match edit {
        LocalDaemonResponse::FileEdited { result } => {
            assert_eq!(result.bytes_written, 5);
            assert_eq!(result.old_size, 6);
            assert_eq!(result.new_size, 5);
            assert!(result.changed);
        }
        _ => panic!("unexpected edit response"),
    }
    match git {
        LocalDaemonResponse::GitInspected { result } => assert!(result.status.contains("main")),
        _ => panic!("unexpected git response"),
    }
}

#[test]
fn local_request_api_inspects_workspace_git_overview() {
    let worktree_root = std::env::temp_dir().join("arroba-workspace-git-overview-test");
    let _ = std::fs::remove_dir_all(&worktree_root);
    std::fs::create_dir_all(&worktree_root).expect("worktree should exist");
    std::fs::write(worktree_root.join("README.md"), "hello\n").expect("file should exist");
    run_test_git(&worktree_root, &["init", "-b", "main"]);
    run_test_git(
        &worktree_root,
        &["config", "user.email", "agent@example.com"],
    );
    run_test_git(&worktree_root, &["config", "user.name", "Agent"]);
    run_test_git(&worktree_root, &["add", "README.md"]);
    run_test_git(&worktree_root, &["commit", "-m", "seed"]);
    std::fs::write(worktree_root.join("README.md"), "hello\nworld\n").expect("file should update");
    std::fs::write(worktree_root.join("new.txt"), "new\n").expect("new file should exist");

    let harness = LocalRouterTestHarness::new();
    let response = harness
        .dispatch(LocalDaemonRequest::GetWorkspaceGitOverview(
            GetWorkspaceGitOverviewRequest {
                workspace_id: worktree_root.display().to_string(),
                worktree_id: worktree_root.display().to_string(),
                compare_ref: Some("HEAD".to_string()),
            },
        ))
        .expect("workspace git overview should succeed");

    match response {
        LocalDaemonResponse::WorkspaceGitOverview { overview } => {
            assert_eq!(overview.branch.as_deref(), Some("main"));
            assert_eq!(overview.compare_ref, "HEAD");
            assert_eq!(overview.totals.files, 2);
            assert_eq!(overview.totals.additions, 2);
            assert!(overview
                .compare_refs
                .iter()
                .any(|reference| reference.name == "HEAD" && reference.selected));
            assert!(overview
                .files
                .iter()
                .any(|file| file.path == "README.md" && file.additions == 1));
            assert!(overview
                .files
                .iter()
                .any(|file| file.path == "new.txt" && file.status == "untracked"));
        }
        _ => panic!("unexpected local response"),
    }
}

#[test]
fn local_request_api_lists_workspace_repo_files() {
    let worktree_root = std::env::temp_dir().join("arroba-workspace-files-test");
    let _ = std::fs::remove_dir_all(&worktree_root);
    std::fs::create_dir_all(worktree_root.join("src/app")).expect("worktree should exist");
    std::fs::write(worktree_root.join("README.md"), "hello\n").expect("file should exist");
    std::fs::write(worktree_root.join("src/app/main.rs"), "fn main() {}\n")
        .expect("file should exist");
    run_test_git(&worktree_root, &["init", "-b", "main"]);
    run_test_git(
        &worktree_root,
        &["config", "user.email", "agent@example.com"],
    );
    run_test_git(&worktree_root, &["config", "user.name", "Agent"]);
    run_test_git(&worktree_root, &["add", "."]);
    run_test_git(&worktree_root, &["commit", "-m", "seed"]);
    std::fs::write(
        worktree_root.join("src/app/main.rs"),
        "fn main() {}\nfn changed() {}\n",
    )
    .expect("file should update");

    let harness = LocalRouterTestHarness::new();
    let root_response = harness
        .dispatch(LocalDaemonRequest::ListWorkspaceFiles(
            ListWorkspaceFilesRequest {
                workspace_id: worktree_root.display().to_string(),
                worktree_id: worktree_root.display().to_string(),
                path_prefix: None,
                compare_ref: Some("HEAD".to_string()),
                limit: None,
            },
        ))
        .expect("workspace files should list");

    match root_response {
        LocalDaemonResponse::WorkspaceFilesListed { listing } => {
            assert_eq!(listing.path_prefix, "");
            assert_eq!(listing.compare_ref, "HEAD");
            assert_eq!(listing.total_entries, 2);
            assert!(!listing.truncated);
            assert!(listing
                .entries
                .iter()
                .any(|entry| entry.name == "src" && entry.kind == "directory" && entry.changed));
        }
        _ => panic!("unexpected local response"),
    }

    let nested_response = harness
        .dispatch(LocalDaemonRequest::ListWorkspaceFiles(
            ListWorkspaceFilesRequest {
                workspace_id: worktree_root.display().to_string(),
                worktree_id: worktree_root.display().to_string(),
                path_prefix: Some("src/app".to_string()),
                compare_ref: Some("HEAD".to_string()),
                limit: None,
            },
        ))
        .expect("workspace nested files should list");

    match nested_response {
        LocalDaemonResponse::WorkspaceFilesListed { listing } => {
            assert_eq!(listing.path_prefix, "src/app");
            assert_eq!(listing.compare_ref, "HEAD");
            assert_eq!(listing.total_entries, 1);
            assert!(!listing.truncated);
            assert!(listing.entries.iter().any(|entry| {
                entry.name == "main.rs"
                    && entry.kind == "file"
                    && entry.status.as_deref() == Some("modified")
                    && entry.additions == 1
            }));
        }
        _ => panic!("unexpected local response"),
    }

    let limited_response = harness
        .dispatch(LocalDaemonRequest::ListWorkspaceFiles(
            ListWorkspaceFilesRequest {
                workspace_id: worktree_root.display().to_string(),
                worktree_id: worktree_root.display().to_string(),
                path_prefix: None,
                compare_ref: Some("HEAD".to_string()),
                limit: Some(1),
            },
        ))
        .expect("limited workspace files should list");

    match limited_response {
        LocalDaemonResponse::WorkspaceFilesListed { listing } => {
            assert_eq!(listing.total_entries, 2);
            assert!(listing.truncated);
            assert_eq!(listing.entries.len(), 1);
        }
        _ => panic!("unexpected local response"),
    }
}

#[test]
fn local_request_api_reads_workspace_file_content() {
    let worktree_root = std::env::temp_dir().join("arroba-workspace-file-content-test");
    let _ = std::fs::remove_dir_all(&worktree_root);
    std::fs::create_dir_all(worktree_root.join("src/app")).expect("worktree should exist");
    std::fs::write(worktree_root.join("README.md"), "# hello\n").expect("file should exist");
    std::fs::write(worktree_root.join("src/app/main.rs"), "fn main() {}\n")
        .expect("file should exist");
    run_test_git(&worktree_root, &["init", "-b", "main"]);
    run_test_git(
        &worktree_root,
        &["config", "user.email", "agent@example.com"],
    );
    run_test_git(&worktree_root, &["config", "user.name", "Agent"]);
    run_test_git(&worktree_root, &["add", "."]);
    run_test_git(&worktree_root, &["commit", "-m", "seed"]);
    std::fs::write(
        worktree_root.join("src/app/main.rs"),
        "fn main() {}\nfn changed() {}\n",
    )
    .expect("file should update");

    let harness = LocalRouterTestHarness::new();
    let response = harness
        .dispatch(LocalDaemonRequest::GetWorkspaceFileContent(
            GetWorkspaceFileContentRequest {
                workspace_id: worktree_root.display().to_string(),
                worktree_id: worktree_root.display().to_string(),
                path: "src/app/main.rs".to_string(),
                compare_ref: Some("HEAD".to_string()),
                known_fingerprint: None,
                max_bytes: None,
            },
        ))
        .expect("workspace file content should load");

    let fingerprint = match response {
        LocalDaemonResponse::WorkspaceFileContent { content } => {
            assert_eq!(content.path, "src/app/main.rs");
            assert_eq!(content.name, "main.rs");
            assert_eq!(content.language, "rust");
            assert_eq!(content.encoding, "utf-8");
            assert_eq!(
                content.content_text.as_deref(),
                Some("fn main() {}\nfn changed() {}\n")
            );
            assert_eq!(content.compare_ref, "HEAD");
            assert_eq!(content.status.as_deref(), Some("modified"));
            assert_eq!(content.additions, 1);
            assert!(!content.truncated);
            assert!(content.sha256.is_some());
            content.fingerprint
        }
        _ => panic!("unexpected local response"),
    };

    let not_modified_response = harness
        .dispatch(LocalDaemonRequest::GetWorkspaceFileContent(
            GetWorkspaceFileContentRequest {
                workspace_id: worktree_root.display().to_string(),
                worktree_id: worktree_root.display().to_string(),
                path: "src/app/main.rs".to_string(),
                compare_ref: Some("HEAD".to_string()),
                known_fingerprint: Some(fingerprint.clone()),
                max_bytes: None,
            },
        ))
        .expect("workspace file content fingerprint should be honored");
    match not_modified_response {
        LocalDaemonResponse::WorkspaceFileContentNotModified {
            path,
            fingerprint: response_fingerprint,
            ..
        } => {
            assert_eq!(path, "src/app/main.rs");
            assert_eq!(response_fingerprint, fingerprint);
        }
        _ => panic!("unexpected local response"),
    }

    let truncated_response = harness
        .dispatch(LocalDaemonRequest::GetWorkspaceFileContent(
            GetWorkspaceFileContentRequest {
                workspace_id: worktree_root.display().to_string(),
                worktree_id: worktree_root.display().to_string(),
                path: "src/app/main.rs".to_string(),
                compare_ref: Some("HEAD".to_string()),
                known_fingerprint: None,
                max_bytes: Some(5),
            },
        ))
        .expect("workspace file content should truncate");
    match truncated_response {
        LocalDaemonResponse::WorkspaceFileContent { content } => {
            assert!(content.truncated);
            assert_eq!(content.content_text.as_deref(), Some("fn ma"));
        }
        _ => panic!("unexpected local response"),
    }
}

#[test]
fn local_request_api_commits_workspace_changes() {
    let worktree_root = std::env::temp_dir().join("arroba-workspace-commit-test");
    let _ = std::fs::remove_dir_all(&worktree_root);
    std::fs::create_dir_all(&worktree_root).expect("worktree should exist");
    std::fs::write(worktree_root.join("README.md"), "hello\n").expect("file should exist");
    run_test_git(&worktree_root, &["init", "-b", "main"]);
    run_test_git(
        &worktree_root,
        &["config", "user.email", "agent@example.com"],
    );
    run_test_git(&worktree_root, &["config", "user.name", "Agent"]);
    run_test_git(&worktree_root, &["add", "."]);
    run_test_git(&worktree_root, &["commit", "-m", "seed"]);
    std::fs::write(worktree_root.join("README.md"), "hello\nworld\n").expect("file should update");

    let harness = LocalRouterTestHarness::new();
    let response = harness
        .dispatch(LocalDaemonRequest::CommitWorkspaceChanges(
            CommitWorkspaceChangesRequest {
                workspace_id: worktree_root.display().to_string(),
                worktree_id: worktree_root.display().to_string(),
                message: "Update README".to_string(),
            },
        ))
        .expect("workspace commit should succeed");

    match response {
        LocalDaemonResponse::WorkspaceGitActionCompleted { result } => {
            assert_eq!(result.action, "commit");
            assert!(result.commit_sha.is_some());
        }
        _ => panic!("unexpected local response"),
    }
    let subject = git_test_output(&worktree_root, &["log", "-1", "--pretty=%s"]);
    assert_eq!(subject.trim(), "Update README");
    assert_eq!(
        git_test_output(&worktree_root, &["status", "--porcelain"]).trim(),
        ""
    );
}

#[test]
fn local_request_api_push_without_upstream_fails_loudly() {
    let worktree_root = std::env::temp_dir().join("arroba-workspace-push-no-upstream-test");
    let _ = std::fs::remove_dir_all(&worktree_root);
    std::fs::create_dir_all(&worktree_root).expect("worktree should exist");
    std::fs::write(worktree_root.join("README.md"), "hello\n").expect("file should exist");
    run_test_git(&worktree_root, &["init", "-b", "main"]);
    run_test_git(
        &worktree_root,
        &["config", "user.email", "agent@example.com"],
    );
    run_test_git(&worktree_root, &["config", "user.name", "Agent"]);
    run_test_git(&worktree_root, &["add", "."]);
    run_test_git(&worktree_root, &["commit", "-m", "seed"]);

    let harness = LocalRouterTestHarness::new();
    let error = harness
        .dispatch(LocalDaemonRequest::PushWorkspaceBranch(
            PushWorkspaceBranchRequest {
                workspace_id: worktree_root.display().to_string(),
                worktree_id: worktree_root.display().to_string(),
                force_with_lease: false,
            },
        ))
        .expect_err("push without upstream should fail");
    assert!(error.to_string().contains("no upstream"));
}

#[test]
fn local_request_api_deletes_unused_workspace_worktree() {
    let workspace_root = std::env::temp_dir().join("arroba-workspace-delete-worktree-test");
    let feature_root = std::env::temp_dir().join("arroba-workspace-delete-worktree-test-feature");
    let _ = std::fs::remove_dir_all(&workspace_root);
    let _ = std::fs::remove_dir_all(&feature_root);
    std::fs::create_dir_all(&workspace_root).expect("workspace should exist");
    std::fs::write(workspace_root.join("README.md"), "hello\n").expect("file should exist");
    run_test_git(&workspace_root, &["init", "-b", "main"]);
    run_test_git(
        &workspace_root,
        &["config", "user.email", "agent@example.com"],
    );
    run_test_git(&workspace_root, &["config", "user.name", "Agent"]);
    run_test_git(&workspace_root, &["add", "."]);
    run_test_git(&workspace_root, &["commit", "-m", "seed"]);

    let harness = LocalRouterTestHarness::new();
    let create = harness
        .dispatch(LocalDaemonRequest::CreateWorkspaceWorktree(
            CreateWorkspaceWorktreeRequest {
                workspace_id: workspace_root.display().to_string(),
                path: Some(feature_root.display().to_string()),
                branch: Some("arroba/delete-test".to_string()),
                base_ref: Some("main".to_string()),
            },
        ))
        .expect("worktree create should succeed");
    let worktree_path = match create {
        LocalDaemonResponse::WorkspaceWorktreeCreated { worktree, .. } => worktree.path,
        _ => panic!("unexpected local response"),
    };

    let delete = harness
        .dispatch(LocalDaemonRequest::DeleteWorkspaceWorktree(
            DeleteWorkspaceWorktreeRequest {
                workspace_id: workspace_root.display().to_string(),
                worktree_id: worktree_path.clone(),
                force: false,
            },
        ))
        .expect("unused worktree delete should succeed");
    match delete {
        LocalDaemonResponse::WorkspaceWorktreeDeleted { path, .. } => {
            assert!(path.ends_with("arroba-workspace-delete-worktree-test-feature"));
            assert!(worktree_path.ends_with("arroba-workspace-delete-worktree-test-feature"));
        }
        _ => panic!("unexpected local response"),
    }
    assert!(!feature_root.exists());
}

#[test]
fn local_request_api_refuses_to_delete_runtime_owned_worktree() {
    let workspace_root = std::env::temp_dir().join("arroba-workspace-delete-owned-test");
    let feature_root = std::env::temp_dir().join("arroba-workspace-delete-owned-test-feature");
    let _ = std::fs::remove_dir_all(&workspace_root);
    let _ = std::fs::remove_dir_all(&feature_root);
    std::fs::create_dir_all(&workspace_root).expect("workspace should exist");
    std::fs::write(workspace_root.join("README.md"), "hello\n").expect("file should exist");
    run_test_git(&workspace_root, &["init", "-b", "main"]);
    run_test_git(
        &workspace_root,
        &["config", "user.email", "agent@example.com"],
    );
    run_test_git(&workspace_root, &["config", "user.name", "Agent"]);
    run_test_git(&workspace_root, &["add", "."]);
    run_test_git(&workspace_root, &["commit", "-m", "seed"]);
    run_test_git(
        &workspace_root,
        &[
            "worktree",
            "add",
            "-b",
            "arroba/owned-delete-test",
            feature_root.to_str().expect("feature path should encode"),
            "main",
        ],
    );

    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                workspace_root.display().to_string(),
                feature_root.display().to_string(),
            ),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let error = harness
        .dispatch(LocalDaemonRequest::DeleteWorkspaceWorktree(
            DeleteWorkspaceWorktreeRequest {
                workspace_id: workspace_root.display().to_string(),
                worktree_id: feature_root.display().to_string(),
                force: true,
            },
        ))
        .expect_err("runtime-owned worktree delete should fail");
    assert!(error.to_string().contains(session.id()));
    assert!(feature_root.exists());
}

fn run_test_git(cwd: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git should run");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_test_output(cwd: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git should run");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn local_request_api_rejects_conflicting_workspace_write_claims() {
    let worktree_root = std::env::temp_dir().join("arroba-workspace-claim-local-api-test");
    let _ = std::fs::remove_dir_all(&worktree_root);
    std::fs::create_dir_all(worktree_root.join("src")).expect("worktree should exist");
    std::fs::write(worktree_root.join("src/lib.rs"), "before").expect("file should exist");

    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-workspace-claim".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let _claim = harness.with_app_mut(|app| {
        app.workspace_coordinator()
            .acquire_worktree_write_claim(
                session.workspace_id().to_string(),
                worktree_root.display().to_string(),
                "other-session",
                Some("other-attachment".to_string()),
                "file_edit",
            )
            .expect("existing claim should acquire")
    });

    let health = harness
        .dispatch(LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest))
        .expect("health should be available while claim is active");
    match health {
        LocalDaemonResponse::DaemonHealth { projection } => {
            assert_eq!(
                projection
                    .workspace_coordination
                    .active_operation_claims
                    .len(),
                1
            );
        }
        _ => panic!("unexpected health response"),
    }

    let error = harness
        .dispatch(LocalDaemonRequest::EditFile(EditFileCapabilityRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            path: worktree_root.join("src/lib.rs"),
            contents: "after".to_string(),
        }))
        .expect_err("conflicting write should be rejected");

    match error {
        DaemonError::WorkspaceClaimConflict {
            requested_session_id,
            existing_session_id,
            ..
        } => {
            assert_eq!(requested_session_id, session.id());
            assert_eq!(existing_session_id, "other-session");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn local_request_api_returns_structured_screenshot_unavailable_result() {
    let _guard = crate::env_lock::lock();
    std::env::set_var("ARROBA_SCREENSHOT_DISABLE", "1");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", std::env::temp_dir().display().to_string()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-screenshot".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let response = harness
        .dispatch(LocalDaemonRequest::CaptureScreenshot(
            CaptureScreenshotCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
            },
        ))
        .expect("screenshot request should succeed with unavailable result");
    std::env::remove_var("ARROBA_SCREENSHOT_DISABLE");

    match response {
        LocalDaemonResponse::ScreenshotCaptured { result } => {
            assert_eq!(
                result.status,
                crate::capability::ScreenshotStatus::Unavailable
            );
        }
        _ => panic!("unexpected screenshot response"),
    }
}

#[test]
fn local_request_api_stores_transferred_file_under_session_artifacts() {
    let worktree_root = std::env::temp_dir().join("arroba-transfer-local-api-test");
    let _ = std::fs::remove_dir_all(&worktree_root);
    std::fs::create_dir_all(&worktree_root).expect("worktree should exist");
    let source = worktree_root.join("artifact.txt");
    std::fs::write(&source, "artifact").expect("file should exist");

    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-transfer".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let response = harness
        .dispatch(LocalDaemonRequest::StoreTransferredFile(
            StoreTransferredFileCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                source_path: source,
                display_name: None,
            },
        ))
        .expect("transfer should succeed");

    match response {
        LocalDaemonResponse::FileTransferred { result } => {
            assert!(result
                .stored_path
                .to_string_lossy()
                .contains("arroba-session-artifacts"));
            assert_eq!(result.bytes, 8);
        }
        _ => panic!("unexpected transfer response"),
    }
}
