use super::*;

#[test]
fn session_worktree_paths_reject_relative_and_missing_directories_without_mutation() {
    let workspace = crate::test_support::TestWorktree::new("session-path-validation");
    let harness = LocalRouterTestHarness::new();
    let valid = workspace.path().display().to_string();
    let relative = format!("chariox-invalid-relative-session-{}", std::process::id());
    let cwd_target = std::env::current_dir().unwrap().join(&relative);
    assert!(!cwd_target.exists());
    let missing = workspace.path().join("missing").display().to_string();
    let file = workspace.path().join("file");
    fs::write(&file, "fixture").unwrap();
    let file = file.display().to_string();
    for (workspace_id, worktree_id, expected) in [
        (valid.as_str(), "main", "absolute"),
        (valid.as_str(), relative.as_str(), "absolute"),
        ("workspace", valid.as_str(), "absolute"),
        (valid.as_str(), missing.as_str(), "does not exist"),
        (missing.as_str(), valid.as_str(), "does not exist"),
        (valid.as_str(), file.as_str(), "not a directory"),
        (file.as_str(), valid.as_str(), "not a directory"),
    ] {
        let error = harness
            .dispatch(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new(workspace_id, worktree_id),
            ))
            .expect_err("invalid session paths must be refused");
        assert!(error.to_string().contains(expected), "{error}");
        harness.with_app_mut(|app| {
            assert!(app.sessions().list_sessions().is_empty());
        });
        assert!(!workspace.path().join("missing").exists());
        assert!(
            !cwd_target.exists(),
            "invalid paths must not create kernel-cwd directories"
        );
    }
}

#[test]
fn session_worktree_paths_accept_absolute_and_preserve_existing_session() {
    let worktree = crate::test_support::TestWorktree::new("session-valid-paths");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .expect("absolute existing paths should be accepted")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => {
            assert_eq!(agent.worktree_id(), Some(session.worktree_id()));
            session
        }
        other => panic!("unexpected response: {other:?}"),
    };
    for target in [None, Some(worktree.path().display().to_string())] {
        harness
            .dispatch(LocalDaemonRequest::SpawnAgent(spawn_request(
                session.id(),
                target,
            )))
            .expect("existing session should still accept agents");
    }
    harness.with_app_mut(|app| {
        let unchanged = app.sessions().get_session(session.id()).unwrap();
        assert_eq!(unchanged.workspace_id(), session.workspace_id());
        assert_eq!(unchanged.worktree_id(), session.worktree_id());
        assert_eq!(app.agents().get_session_agents(session.id()).len(), 3);
    });
}

#[test]
fn session_worktree_paths_reject_invalid_agent_overrides_without_mutation() {
    let worktree = crate::test_support::TestWorktree::new("agent-path-validation");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .unwrap()
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected response: {other:?}"),
    };
    let missing = worktree.path().join("missing").display().to_string();
    for (target, expected) in [("main", "absolute"), (missing.as_str(), "does not exist")] {
        let error = harness
            .dispatch(LocalDaemonRequest::SpawnAgent(spawn_request(
                session.id(),
                Some(target.to_string()),
            )))
            .expect_err("invalid agent paths must be refused");
        assert!(error.to_string().contains(expected), "{error}");
        harness.with_app_mut(|app| {
            assert_eq!(app.agents().get_session_agents(session.id()).len(), 1);
        });
        assert!(!worktree.path().join("missing").exists());
    }
}

fn spawn_request(session_id: &str, worktree_id: Option<String>) -> SpawnAgentRequest {
    SpawnAgentRequest {
        session_id: session_id.to_string(),
        alias: None,
        provider: None,
        account_profile: None,
        model: None,
        effort: None,
        execution_mode: None,
        permission_level: None,
        worktree_id,
        kernel_ref: None,
        slice_ref: None,
        worktree_placement: None,
        metaagent: false,
    }
}

#[test]
fn session_worktree_paths_validate_placement_base_before_creating_worktree() {
    let workspace = crate::test_support::TestWorktree::new("session-placement-validation");
    let harness = LocalRouterTestHarness::new();
    let target = workspace.path().join("new-worktree");
    let error = harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(workspace.path().display().to_string(), "main")
                .with_worktree_placement(crate::agent::GitWorktreePlacement {
                    target_directory: Some(target.display().to_string()),
                    branch: Some("feature".to_string()),
                    from_ref: None,
                }),
        ))
        .expect_err("relative placement base must be refused before git runs");
    assert!(error.to_string().contains("absolute"), "{error}");
    assert!(!target.exists());
}

#[test]
fn session_worktree_paths_accept_linked_worktree_and_relative_creation_target() {
    let root = crate::test_support::TestWorktree::new("session-git-paths");
    let repo = root.path().join("repo");
    fs::create_dir(&repo).unwrap();
    test_git(&repo, &["init", "-b", "main"]);
    test_git(
        &repo,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--allow-empty",
            "-m",
            "seed",
        ],
    );
    let linked = root.path().join("linked");
    test_git(
        &repo,
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    );
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(repo.display().to_string(), linked.display().to_string()),
        ))
        .expect("linked worktree should be accepted")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected response: {other:?}"),
    };
    assert_eq!(session.workspace_id(), repo.to_str().unwrap());
    assert_eq!(session.worktree_id(), linked.to_str().unwrap());
    let mut request = spawn_request(session.id(), None);
    request.worktree_placement = Some(crate::agent::GitWorktreePlacement {
        target_directory: Some("../created".to_string()),
        branch: Some("created".to_string()),
        from_ref: None,
    });
    let response = harness
        .dispatch(LocalDaemonRequest::SpawnAgent(request))
        .expect("relative creation target should resolve against the absolute session worktree");
    let created = root.path().join("created").canonicalize().unwrap();
    match response {
        LocalDaemonResponse::AgentSpawned { agent, .. } => {
            assert_eq!(
                std::path::Path::new(agent.worktree_id().unwrap())
                    .canonicalize()
                    .unwrap(),
                created
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

fn test_git(directory: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(directory)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn session_worktree_paths_validate_explicit_local_kernel_reference() {
    let workspace = crate::test_support::TestWorktree::new("session-local-kernel-paths");
    let config = DaemonConfig::for_tests();
    let kernel_id = config.daemon_id.clone();
    let harness = LocalRouterTestHarness::with_config(config);
    let missing = workspace.path().join("missing");
    let error = harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                missing.display().to_string(),
                workspace.path().display().to_string(),
            )
            .with_kernel_ref(kernel_id.clone()),
        ))
        .expect_err("explicit local kernel must still validate workspace paths");
    assert!(error.to_string().contains("does not exist"), "{error}");
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            workspace
                .session_request()
                .with_kernel_ref(kernel_id.clone()),
        ))
        .expect("valid local kernel reference should create a local session")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected response: {other:?}"),
    };
    let mut request = spawn_request(session.id(), Some("main".to_string()));
    request.kernel_ref = Some(kernel_id);
    let error = harness
        .dispatch(LocalDaemonRequest::SpawnAgent(request))
        .expect_err("explicit local kernel must still reject relative agent paths");
    assert!(error.to_string().contains("absolute"), "{error}");
}

#[test]
fn session_worktree_paths_preserve_valid_session_after_durable_replay() {
    let root = crate::test_support::TestWorktree::new("session-path-replay");
    let mut config = DaemonConfig::for_tests();
    config.user_config.state.path = Some(root.path().join("state.db").display().to_string());
    let harness = LocalRouterTestHarness::with_config(config.clone());
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(root.session_request()))
        .unwrap()
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected response: {other:?}"),
    };
    drop(harness);
    let restored = LocalRouterTestHarness::with_config(config);
    match restored
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("valid persisted session should load")
    {
        LocalDaemonResponse::SessionState {
            session: replayed, ..
        } => {
            assert_eq!(replayed.workspace_id(), session.workspace_id());
            assert_eq!(replayed.worktree_id(), session.worktree_id());
        }
        other => panic!("unexpected response: {other:?}"),
    }
    restored
        .dispatch(LocalDaemonRequest::SpawnAgent(spawn_request(
            session.id(),
            None,
        )))
        .expect("valid restored session should still accept agents");
}
