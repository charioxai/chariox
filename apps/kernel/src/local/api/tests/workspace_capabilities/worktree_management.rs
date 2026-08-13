use super::*;

#[test]
fn local_request_api_commits_workspace_changes() {
    run_workspace_capability_test(
        "local_request_api_commits_workspace_changes",
        local_request_api_commits_workspace_changes_inner,
    );
}

fn local_request_api_commits_workspace_changes_inner() {
    let worktree_root = std::env::temp_dir().join("chariox-workspace-commit-test");
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
    run_workspace_capability_test(
        "local_request_api_push_without_upstream_fails_loudly",
        local_request_api_push_without_upstream_fails_loudly_inner,
    );
}

fn local_request_api_push_without_upstream_fails_loudly_inner() {
    let worktree_root = std::env::temp_dir().join("chariox-workspace-push-no-upstream-test");
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
    run_workspace_capability_test(
        "local_request_api_deletes_unused_workspace_worktree",
        local_request_api_deletes_unused_workspace_worktree_inner,
    );
}

fn local_request_api_deletes_unused_workspace_worktree_inner() {
    let workspace_root = std::env::temp_dir().join("chariox-workspace-delete-worktree-test");
    let feature_root = std::env::temp_dir().join("chariox-workspace-delete-worktree-test-feature");
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
                branch: Some("chariox/delete-test".to_string()),
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
            assert!(path.ends_with("chariox-workspace-delete-worktree-test-feature"));
            assert!(worktree_path.ends_with("chariox-workspace-delete-worktree-test-feature"));
        }
        _ => panic!("unexpected local response"),
    }
    assert!(!feature_root.exists());
}

#[test]
fn local_request_api_creates_session_worktree_without_duplicate_repo_prefix() {
    run_workspace_capability_test(
        "local_request_api_creates_session_worktree_without_duplicate_repo_prefix",
        local_request_api_creates_session_worktree_without_duplicate_repo_prefix_inner,
    );
}

fn local_request_api_creates_session_worktree_without_duplicate_repo_prefix_inner() {
    let parent = std::env::temp_dir().join("chariox-workspace-session-worktree-prefix-test");
    let workspace_root = parent.join("chariox-cloud");
    let expected_worktree_root = parent.join("chariox-cloud-session-1783622367");
    let duplicate_worktree_root =
        parent.join("chariox-cloud-chariox-chariox-cloud-session-1783622367");
    let _ = std::fs::remove_dir_all(&parent);
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
                path: None,
                branch: Some("chariox/chariox-cloud-session-1783622367".to_string()),
                base_ref: Some("main".to_string()),
            },
        ))
        .expect("worktree create should succeed");

    match create {
        LocalDaemonResponse::WorkspaceWorktreeCreated { worktree, .. } => {
            let returned_path = std::path::PathBuf::from(&worktree.path);
            assert_eq!(
                returned_path.file_name().and_then(std::ffi::OsStr::to_str),
                Some("chariox-cloud-session-1783622367")
            );
            assert!(returned_path.exists());
            assert!(expected_worktree_root.exists());
            assert!(!duplicate_worktree_root.exists());
        }
        _ => panic!("unexpected local response"),
    }

    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn local_request_api_refuses_to_delete_runtime_owned_worktree() {
    run_workspace_capability_test(
        "local_request_api_refuses_to_delete_runtime_owned_worktree",
        local_request_api_refuses_to_delete_runtime_owned_worktree_inner,
    );
}

fn local_request_api_refuses_to_delete_runtime_owned_worktree_inner() {
    let workspace_root = std::env::temp_dir().join("chariox-workspace-delete-owned-test");
    let feature_root = std::env::temp_dir().join("chariox-workspace-delete-owned-test-feature");
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
            "chariox/owned-delete-test",
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
