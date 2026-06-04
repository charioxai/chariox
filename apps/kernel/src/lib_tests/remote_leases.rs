use super::*;

#[test]
fn execution_leases_require_opt_in_and_can_be_destroyed() {
    let mut disabled =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let error = RemoteLeaseRuntime::new(&mut disabled)
        .create_execution_lease("home-kernel", "session-1", "agent-1", "user-home")
        .expect_err("remote leases should require opt-in");
    match error {
        DaemonError::RemoteLeasesDisabled { .. } => {}
        other => panic!("unexpected error: {other}"),
    }

    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config.clone()).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease("home-kernel", "session-1", "agent-1", "user-home")
        .expect("execution lease should be created");
    assert_eq!(lease.worker_kernel_id, config.daemon_id);
    assert_eq!(lease.machine_id, config.host_machine_id);
    assert_eq!(RemoteLeaseRuntime::new(&mut app).execution_lease_count(), 1);

    let removed = RemoteLeaseRuntime::new(&mut app)
        .destroy_execution_lease(&lease.id)
        .expect("execution lease should be removed");
    assert_eq!(removed.id, lease.id);
    assert_eq!(RemoteLeaseRuntime::new(&mut app).execution_lease_count(), 0);
}

#[test]
fn leased_agents_require_existing_lease_and_can_be_destroyed() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease("home-kernel", "session-1", "agent-home-1", "user-home")
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
        .create_execution_lease("home-kernel", "session-1", "agent-home-1", "user-home")
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
fn leased_agents_reject_missing_working_directory() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease("home-kernel", "session-1", "agent-home-1", "user-home")
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
        .create_execution_lease("home-kernel", "session-1", "agent-home-1", "user-home")
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

#[test]
fn leased_agents_can_submit_and_complete_prompts_through_backing_session() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    config.user_config.providers.workspace_live_sync =
        crate::config::WorkspaceLiveSyncConfig::from_mode(
            crate::config::WorkspaceLiveSyncMode::Managed,
        );
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease("home-kernel", "session-1", "agent-home-1", "user-home")
        .expect("execution lease should be created");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased agent should be created");

    let hidden_backing_session = app
        .sessions()
        .get_session(&leased_agent.backing_session_id)
        .expect("backing session should exist");
    assert!(hidden_backing_session.is_hidden());
    let backing_attachment = app
        .attachments()
        .get_attachment(&leased_agent.backing_attachment_id)
        .expect("leased backing attachment should exist");
    assert_eq!(backing_attachment.owner_user_id(), "user-home");
    assert!(app
        .sessions()
        .list_sessions()
        .into_iter()
        .all(|session| session.id() != leased_agent.backing_session_id));

    let (provider_run_id, outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt(&leased_agent.id, "remote leased prompt\n", Vec::new())
        .expect("leased prompt should submit");
    match outcome {
        PromptSubmissionOutcome::Started { .. } => {}
        other => panic!("unexpected prompt submission outcome: {other:?}"),
    }

    let provider_run = app
        .providers()
        .get_run(&provider_run_id)
        .expect("provider run should exist");
    assert!(provider_run.requires_workspace_live_sync());
    assert_eq!(provider_run.session_id(), leased_agent.backing_session_id);
    assert_eq!(
        provider_run.agent_instance_id(),
        Some(leased_agent.backing_agent_id.as_str())
    );

    let completion = RemoteLeaseRuntime::new(&mut app)
        .complete_leased_prompt(&leased_agent.id)
        .expect("leased prompt should complete");
    assert_eq!(
        completion.completed.target_agent_id(),
        leased_agent.backing_agent_id
    );
}

#[test]
fn leased_projection_forwards_completion_when_backing_prompt_already_settled() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease("home-kernel", "session-1", "agent-home-1", "user-home")
        .expect("execution lease should be created");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased agent should be created");

    let (provider_run_id, outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt(&leased_agent.id, "remote leased prompt\n", Vec::new())
        .expect("leased prompt should submit");
    match outcome {
        PromptSubmissionOutcome::Started { .. } => {}
        other => panic!("unexpected prompt submission outcome: {other:?}"),
    }
    app.terminal_mut().record_assistant_message_completion(
        &leased_agent.backing_session_id,
        &provider_run_id,
        Some(&leased_agent.backing_agent_id),
        vec![leased_agent.backing_attachment_id.clone()],
        "assistant-msg-1",
        1234,
    );
    app.complete_active_prompt(
        &leased_agent.backing_session_id,
        &leased_agent.backing_agent_id,
        Some(&provider_run_id),
    )
    .expect("backing prompt should settle first");

    let (_target_kernel_id, event) = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
        .expect("settled backing prompt should not block completion projection")
        .expect("completion projection should be emitted");
    let RelayPeerEvent::LeasedRuntimeProjection { completions, .. } = event;
    assert!(completions
        .iter()
        .any(|completion| completion.message_id == "assistant-msg-1"));
}

#[test]
fn leased_projection_pump_forwards_completion_after_provider_run_ends() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease("home-kernel", "session-1", "agent-home-1", "user-home")
        .expect("execution lease should be created");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased agent should be created");

    let (provider_run_id, outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt(&leased_agent.id, "remote leased prompt\n", Vec::new())
        .expect("leased prompt should submit");
    match outcome {
        PromptSubmissionOutcome::Started { .. } => {}
        other => panic!("unexpected prompt submission outcome: {other:?}"),
    }
    app.complete_active_prompt(
        &leased_agent.backing_session_id,
        &leased_agent.backing_agent_id,
        Some(&provider_run_id),
    )
    .expect("backing prompt should settle first");
    let ended = app
        .providers_mut()
        .terminate_run_provider_only(&leased_agent.backing_session_id, &provider_run_id)
        .expect("provider run should end")
        .into_run();
    app.update_provider_run_projection(ended);

    let events = RemoteLeaseRuntime::new(&mut app)
        .pump_leased_runtime_projections()
        .expect("leased projection pump should run");

    assert_eq!(events.len(), 1);
    let (_target_kernel_id, event) = &events[0];
    let RelayPeerEvent::LeasedRuntimeProjection { completions, .. } = event;
    assert_eq!(completions.len(), 1);
}

#[test]
fn remote_runtime_projection_records_output_and_completion_on_home_session() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("attachment should attach");
    let prompt = app
        .submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "remote prompt",
            Vec::new(),
        )
        .expect("prompt should start");
    assert!(matches!(prompt, PromptSubmissionOutcome::Started { .. }));

    RemoteLeaseRuntime::new(&mut app)
        .project_remote_runtime_projection(
            session.id(),
            agent.id(),
            "remote:worker:provider-run-1",
            Vec::new(),
            vec![RelayProjectedOutputChunk {
                kind: TerminalOutputKind::ProviderOutput,
                merge_key: Some("assistant-1".to_string()),
                bytes: b"remote output".to_vec(),
            }],
            vec!["remote notice".to_string()],
            vec![RelayProjectedCompletion {
                message_id: "assistant-msg-1".to_string(),
                completed_at_ms: 1234,
            }],
        )
        .expect("projection should succeed");

    let outputs = app
        .terminal_mut()
        .drain_output_records(session.id(), attachment.id());
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0].agent_id.as_deref(), Some(agent.id()));
    assert_eq!(outputs[0].bytes, b"remote output".to_vec());

    let notices = app
        .terminal_mut()
        .drain_notice_records(session.id(), attachment.id());
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].agent_id.as_deref(), Some(agent.id()));
    assert_eq!(notices[0].message, "remote notice");

    let completions = app
        .terminal_mut()
        .drain_completion_records(session.id(), attachment.id());
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].agent_id.as_deref(), Some(agent.id()));
    assert_eq!(completions[0].message_id, "assistant-msg-1");

    let projected = app
        .session_state_projection_store()
        .get(session.id())
        .expect("projection should refresh");
    assert!(projected
        .prompt_states()
        .get(agent.id())
        .and_then(|state| state.active_prompt())
        .is_none());
}
