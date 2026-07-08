use super::*;

#[test]
fn execution_leases_are_enabled_by_default_and_can_be_disabled() {
    let config = DaemonConfig::for_tests();
    let mut app = DaemonApp::bootstrap(config.clone()).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease("home-kernel", "session-1", "agent-1", false, "user-home")
        .expect("execution lease should be created by default");
    assert_eq!(lease.worker_kernel_id, config.daemon_id);
    assert_eq!(lease.machine_id, config.host_machine_id);
    assert_eq!(RemoteLeaseRuntime::new(&mut app).execution_lease_count(), 1);

    let removed = RemoteLeaseRuntime::new(&mut app)
        .destroy_execution_lease(&lease.id)
        .expect("execution lease should be removed");
    assert_eq!(removed.id, lease.id);
    assert_eq!(RemoteLeaseRuntime::new(&mut app).execution_lease_count(), 0);

    let mut disabled_config = DaemonConfig::for_tests();
    disabled_config.accept_remote_leases = false;
    let mut disabled =
        DaemonApp::bootstrap(disabled_config).expect("daemon bootstrap should succeed");
    let error = RemoteLeaseRuntime::new(&mut disabled)
        .create_execution_lease("home-kernel", "session-1", "agent-1", false, "user-home")
        .expect_err("remote leases should honor explicit disablement");
    match error {
        DaemonError::RemoteLeasesDisabled { .. } => {}
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn leased_agents_require_existing_lease_and_can_be_destroyed() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "session-1",
            "agent-home-1",
            false,
            "user-home",
        )
        .expect("execution lease should be created");
    let worktree = std::env::temp_dir().join(format!(
        "arroba-leased-agent-worktree-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&worktree).expect("leased worktree should exist");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "opencode",
            Some("kimi2.5".to_string()),
            None,
            None,
            None,
            None,
            Some(worktree.display().to_string()),
            None,
        )
        .expect("leased agent should be created");
    assert_eq!(leased_agent.lease_id, lease.id);
    assert_eq!(leased_agent.home_agent_id, "agent-home-1");
    assert_eq!(leased_agent.provider, "opencode");
    let backing_session = app
        .sessions()
        .get_session(&leased_agent.backing_session_id)
        .expect("backing session should exist");
    assert_eq!(
        backing_session.worktree_id(),
        worktree.display().to_string()
    );
    assert_eq!(RemoteLeaseRuntime::new(&mut app).leased_agent_count(), 1);

    let removed = RemoteLeaseRuntime::new(&mut app)
        .destroy_leased_agent(&leased_agent.id)
        .expect("leased agent should be removed");
    assert_eq!(removed.id, leased_agent.id);
    assert_eq!(RemoteLeaseRuntime::new(&mut app).leased_agent_count(), 0);
}

#[test]
fn leased_agents_project_workspace_live_sync_mode_to_backing_session() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "session-1",
            "agent-home-1",
            false,
            "user-home",
        )
        .expect("execution lease should be created");
    let worktree = std::env::temp_dir().join(format!(
        "arroba-leased-agent-wls-mode-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&worktree).expect("leased worktree should exist");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "opencode",
            Some("kimi2.5".to_string()),
            None,
            None,
            None,
            Some(crate::config::WorkspaceLiveSyncMode::Tracked),
            Some(worktree.display().to_string()),
            None,
        )
        .expect("leased agent should be created");
    let backing_session = app
        .sessions()
        .get_session(&leased_agent.backing_session_id)
        .expect("backing session should exist");
    assert_eq!(
        backing_session.workspace_live_sync_mode(),
        Some(crate::config::WorkspaceLiveSyncMode::Tracked)
    );

    let reused_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "opencode",
            Some("kimi2.5".to_string()),
            None,
            None,
            None,
            Some(crate::config::WorkspaceLiveSyncMode::Managed),
            Some(worktree.display().to_string()),
            None,
        )
        .expect("leased agent should reuse backing session");
    assert_eq!(
        reused_agent.backing_session_id,
        leased_agent.backing_session_id
    );
    let backing_session = app
        .sessions()
        .get_session(&reused_agent.backing_session_id)
        .expect("backing session should exist");
    assert_eq!(
        backing_session.workspace_live_sync_mode(),
        Some(crate::config::WorkspaceLiveSyncMode::Managed)
    );
}

#[test]
fn destroying_one_shared_session_leased_agent_preserves_other_leases() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "session-1",
            "agent-home-1",
            false,
            "user-home",
        )
        .expect("execution lease should be created");
    let worktree = std::env::temp_dir().join(format!(
        "arroba-shared-leased-agent-worktree-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&worktree).expect("leased worktree should exist");
    let first = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "opencode",
            Some("kimi2.5".to_string()),
            None,
            None,
            None,
            None,
            Some(worktree.display().to_string()),
            None,
        )
        .expect("first leased agent should be created");
    let second = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "opencode",
            Some("kimi2.5".to_string()),
            None,
            None,
            None,
            None,
            Some(worktree.display().to_string()),
            None,
        )
        .expect("second leased agent should reuse backing session");
    assert_eq!(first.backing_session_id, second.backing_session_id);

    let removed = RemoteLeaseRuntime::new(&mut app)
        .destroy_leased_agent(&first.id)
        .expect("first leased agent should be destroyed");
    assert_eq!(removed.id, first.id);
    assert_eq!(RemoteLeaseRuntime::new(&mut app).leased_agent_count(), 1);
    app.sessions()
        .get_session(&second.backing_session_id)
        .expect("shared backing session should remain for second lease");
    app.agents
        .get_agent(&second.backing_agent_id)
        .expect("second backing agent should remain");
}

#[test]
fn leased_agents_reject_missing_working_directory() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "session-1",
            "agent-home-1",
            false,
            "user-home",
        )
        .expect("execution lease should be created");
    let missing = std::env::temp_dir().join(format!(
        "arroba-missing-leased-agent-worktree-{}",
        crate::session::unix_epoch_ms()
    ));
    let error = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "opencode",
            Some("kimi2.5".to_string()),
            None,
            None,
            None,
            None,
            Some(missing.display().to_string()),
            None,
        )
        .expect_err("missing worker directory should be rejected");
    assert!(error.to_string().contains("remote working directory"));
}

#[test]
fn leased_agents_materialize_remote_git_worktree_before_creation() {
    let _guard = CURRENT_DIR_LOCK.lock().expect("current dir lock");
    let original_dir = std::env::current_dir().expect("current dir should resolve");
    let root = std::env::temp_dir().join(format!(
        "arroba-remote-git-worktree-base-{}",
        crate::session::unix_epoch_ms()
    ));
    let target = std::env::temp_dir().join(format!(
        "arroba-remote-git-worktree-target-{}",
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&target);
    std::fs::create_dir_all(&root).expect("repo root should exist");
    run_test_git(&root, &["init", "-b", "main"]);
    run_test_git(&root, &["config", "user.email", "arroba@example.test"]);
    run_test_git(&root, &["config", "user.name", "Arroba Test"]);
    std::fs::write(root.join("README.md"), "remote worktree\n").expect("readme should write");
    run_test_git(&root, &["add", "README.md"]);
    run_test_git(&root, &["commit", "-m", "init"]);

    std::env::set_current_dir(&root).expect("test should enter repo root");
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "session-1",
            "agent-home-1",
            false,
            "user-home",
        )
        .expect("execution lease should be created");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "opencode",
            Some("kimi2.5".to_string()),
            None,
            None,
            None,
            None,
            Some(target.display().to_string()),
            Some(GitWorktreePlacement {
                target_directory: Some(target.display().to_string()),
                branch: Some("feature/remote-worktree".to_string()),
                from_ref: Some("main".to_string()),
            }),
        )
        .expect("leased agent should be created in materialized worktree");
    std::env::set_current_dir(original_dir).expect("current dir should restore");

    assert!(target.join("README.md").exists());
    let backing_session = app
        .sessions()
        .get_session(&leased_agent.backing_session_id)
        .expect("backing session should exist");
    assert_eq!(backing_session.worktree_id(), target.display().to_string());
}
