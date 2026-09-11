use super::*;
use crate::app::{LeaseCallerBinding, LeasedAgentCleanupPhase};

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

    RemoteLeaseRuntime::new(&mut app)
        .destroy_execution_lease(&lease.id)
        .expect("execution lease should be removed");
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
fn execution_lease_capacity_rejects_concurrent_lease_and_reopens_after_destroy() {
    let mut config = DaemonConfig::for_tests();
    config.remote_lease_capacity = Some(1);
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");

    assert!(app.relay_registration().accepting_remote_leases);
    let first = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease("home-kernel", "session-1", "agent-1", false, "user-home")
        .expect("first execution lease should fit capacity");
    assert!(!app.relay_registration().accepting_remote_leases);

    let error = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease("home-kernel", "session-2", "agent-2", false, "user-home")
        .expect_err("second concurrent execution lease should exceed capacity");
    assert!(matches!(error, DaemonError::RemoteLeasesDisabled { .. }));
    assert_eq!(RemoteLeaseRuntime::new(&mut app).execution_lease_count(), 1);

    RemoteLeaseRuntime::new(&mut app)
        .destroy_execution_lease(&first.id)
        .expect("first execution lease should be removed");
    assert!(app.relay_registration().accepting_remote_leases);

    RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease("home-kernel", "session-2", "agent-2", false, "user-home")
        .expect("capacity should reopen after lease destruction");
    assert!(!app.relay_registration().accepting_remote_leases);
}

#[test]
fn leased_agents_require_existing_lease_and_can_be_destroyed() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    config.kernel_runtime_role = crate::config::KernelRuntimeRole::RemoteLeaseWorker;
    config.remote_lease_capacity = Some(1);
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
    let caller = LeaseCallerBinding {
        home_kernel_id: "home-kernel".to_string(),
        owner_user_id: "user-home".to_string(),
        realm_id: "realm-home".to_string(),
        public_key_thumbprint: "key-home".to_string(),
    };
    RemoteLeaseRuntime::new(&mut app)
        .bind_execution_lease_caller(&lease.id, caller.clone())
        .expect("execution lease caller should bind");
    let worktree = std::env::temp_dir().join(format!(
        "chariox-leased-agent-worktree-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&worktree).expect("leased worktree should exist");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "opencode",
            "default",
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
    assert!(backing_session.is_hidden());
    assert_eq!(
        backing_session.worktree_id(),
        worktree.display().to_string()
    );
    assert_eq!(RemoteLeaseRuntime::new(&mut app).leased_agent_count(), 1);
    RemoteLeaseRuntime::new(&mut app)
        .authorize_leased_agent_caller(&leased_agent.id, &caller)
        .expect("leased agent should inherit the execution lease caller");
    let mut replacement_key = caller.clone();
    replacement_key.public_key_thumbprint = "replacement-key".to_string();
    assert!(matches!(
        RemoteLeaseRuntime::new(&mut app)
            .authorize_leased_agent_caller(&leased_agent.id, &replacement_key),
        Err(DaemonError::LeaseCallerUnauthorized { .. })
    ));

    RemoteLeaseRuntime::new(&mut app)
        .destroy_leased_agent(&leased_agent.id)
        .expect("leased agent should be removed");
    assert!(app
        .agents
        .get_agent(&leased_agent.backing_agent_id)
        .is_err());
    assert_eq!(RemoteLeaseRuntime::new(&mut app).leased_agent_count(), 0);
    RemoteLeaseRuntime::new(&mut app)
        .authorize_leased_agent_caller(&leased_agent.id, &caller)
        .expect("owner should retain idempotent deletion authority");
    assert!(matches!(
        RemoteLeaseRuntime::new(&mut app)
            .authorize_leased_agent_caller(&leased_agent.id, &replacement_key),
        Err(DaemonError::LeaseCallerUnauthorized { .. })
    ));
}

#[test]
fn leased_agent_cleanup_is_retryable_and_retains_authority_and_capacity() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    config.kernel_runtime_role = crate::config::KernelRuntimeRole::RemoteLeaseWorker;
    config.remote_lease_capacity = Some(1);
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let caller = LeaseCallerBinding {
        home_kernel_id: "home-kernel".to_string(),
        home_machine_id: "home-machine".to_string(),
        owner_user_id: "user-home".to_string(),
        realm_id: "realm-home".to_string(),
        public_key_thumbprint: "key-home".to_string(),
    };
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_bound_execution_lease(
            "home-kernel",
            "session-1",
            "agent-home-1",
            false,
            "user-home",
            caller.clone(),
        )
        .expect("bound execution lease should create atomically");
    let worktree = std::env::temp_dir().join(format!(
        "chariox-leased-agent-draining-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&worktree).expect("leased worktree should exist");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent_for_caller(
            &lease.id,
            &caller,
            "opencode",
            "default",
            None,
            None,
            None,
            None,
            None,
            Some(worktree.display().to_string()),
            None,
        )
        .expect("leased agent should create");

    for phase in [
        LeasedAgentCleanupPhase::Provider,
        LeasedAgentCleanupPhase::Attachment,
        LeasedAgentCleanupPhase::Agent,
        LeasedAgentCleanupPhase::BackingSessionEnd,
        LeasedAgentCleanupPhase::BackingSessionDelete,
    ] {
        RemoteLeaseRuntime::new(&mut app)
            .inject_next_leased_agent_cleanup_failure(&leased_agent.id, phase);
        let error = RemoteLeaseRuntime::new(&mut app)
            .destroy_leased_agent_for_caller(&leased_agent.id, &caller)
            .expect_err("injected cleanup failure must remain retryable");
        assert!(matches!(error, DaemonError::AgentWorkerCleanup { .. }));
        assert_eq!(RemoteLeaseRuntime::new(&mut app).execution_lease_count(), 1);
        assert_eq!(RemoteLeaseRuntime::new(&mut app).leased_agent_count(), 1);
        RemoteLeaseRuntime::new(&mut app)
            .authorize_leased_agent_caller(&leased_agent.id, &caller)
            .expect("draining agent must retain authenticated caller binding");
        assert!(!app.relay_registration().accepting_remote_leases);
    }

    RemoteLeaseRuntime::new(&mut app)
        .destroy_leased_agent_for_caller(&leased_agent.id, &caller)
        .expect("cleanup retry should finish");
    assert_eq!(RemoteLeaseRuntime::new(&mut app).leased_agent_count(), 0);
    RemoteLeaseRuntime::new(&mut app)
        .destroy_execution_lease_for_caller(&lease.id, &caller)
        .expect("lease cleanup should finish");
    assert!(app.relay_registration().accepting_remote_leases);
    std::fs::remove_dir_all(worktree).expect("leased worktree should clean up");
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
        "chariox-leased-agent-wls-mode-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&worktree).expect("leased worktree should exist");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "opencode",
            "default",
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
            "default",
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
        "chariox-shared-leased-agent-worktree-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&worktree).expect("leased worktree should exist");
    let first = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "opencode",
            "default",
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
            "default",
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

    RemoteLeaseRuntime::new(&mut app)
        .destroy_leased_agent(&first.id)
        .expect("first leased agent should be destroyed");
    assert!(app.agents.get_agent(&first.backing_agent_id).is_err());
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
        "chariox-missing-leased-agent-worktree-{}",
        crate::session::unix_epoch_ms()
    ));
    let error = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "opencode",
            "default",
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
    let root = std::env::temp_dir().join(format!(
        "chariox-remote-git-worktree-base-{}",
        crate::session::unix_epoch_ms()
    ));
    let target = std::env::temp_dir().join(format!(
        "chariox-remote-git-worktree-target-{}",
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&target);
    std::fs::create_dir_all(&root).expect("repo root should exist");
    run_test_git(&root, &["init", "-b", "main"]);
    run_test_git(&root, &["config", "user.email", "chariox@example.test"]);
    run_test_git(&root, &["config", "user.name", "Chariox Test"]);
    std::fs::write(root.join("README.md"), "remote worktree\n").expect("readme should write");
    run_test_git(&root, &["add", "README.md"]);
    run_test_git(&root, &["commit", "-m", "init"]);

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
        .create_leased_agent_from_base_directory(
            &root,
            &lease.id,
            "opencode",
            "default",
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

    assert!(target.join("README.md").exists());
    let backing_session = app
        .sessions()
        .get_session(&leased_agent.backing_session_id)
        .expect("backing session should exist");
    assert_eq!(backing_session.worktree_id(), target.display().to_string());
    let _ = std::fs::remove_dir_all(&target);
    let _ = std::fs::remove_dir_all(&root);
}
