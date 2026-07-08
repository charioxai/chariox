use super::*;

#[test]
fn local_request_api_runs_shell_command_capability() {
    run_workspace_capability_test(
        "local_request_api_runs_shell_command_capability",
        local_request_api_runs_shell_command_capability_inner,
    );
}

fn local_request_api_runs_shell_command_capability_inner() {
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
    run_workspace_capability_test(
        "local_request_api_rejects_shell_command_for_unauthorized_attachment",
        local_request_api_rejects_shell_command_for_unauthorized_attachment_inner,
    );
}

fn local_request_api_rejects_shell_command_for_unauthorized_attachment_inner() {
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
    run_workspace_capability_test(
        "local_request_api_rejects_file_capability_for_unauthorized_attachment",
        local_request_api_rejects_file_capability_for_unauthorized_attachment_inner,
    );
}

fn local_request_api_rejects_file_capability_for_unauthorized_attachment_inner() {
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
    run_workspace_capability_test(
        "local_request_api_reads_directory_tree_file_and_git_status",
        local_request_api_reads_directory_tree_file_and_git_status_inner,
    );
}

fn local_request_api_reads_directory_tree_file_and_git_status_inner() {
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
    run_workspace_capability_test(
        "local_request_api_inspects_workspace_git_overview",
        local_request_api_inspects_workspace_git_overview_inner,
    );
}

fn local_request_api_inspects_workspace_git_overview_inner() {
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
    run_workspace_capability_test(
        "local_request_api_lists_workspace_repo_files",
        local_request_api_lists_workspace_repo_files_inner,
    );
}

fn local_request_api_lists_workspace_repo_files_inner() {
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
    run_workspace_capability_test(
        "local_request_api_reads_workspace_file_content",
        local_request_api_reads_workspace_file_content_inner,
    );
}

fn local_request_api_reads_workspace_file_content_inner() {
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
