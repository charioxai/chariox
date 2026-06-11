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
fn destroying_one_shared_session_leased_agent_preserves_other_leases() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease("home-kernel", "session-1", "agent-home-1", "user-home")
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
fn leased_projection_does_not_reflect_home_origin_prompt_back_to_home() {
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
    app.terminal_mut().fan_out_output(
        &leased_agent.backing_session_id,
        &provider_run_id,
        Some(&leased_agent.backing_agent_id),
        crate::terminal::TerminalOutputKind::ProviderOutput,
        Some("assistant-output".to_string()),
        vec![leased_agent.backing_attachment_id.clone()],
        b"hello from worker",
    );

    let (_target_kernel_id, event) = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
        .expect("projection drain should succeed")
        .expect("output projection should be emitted");
    let RelayPeerEvent::LeasedRuntimeProjection {
        prompts,
        output_chunks,
        completions,
        ..
    } = event;
    assert!(
        prompts.is_empty(),
        "home-origin prompt must not be reflected"
    );
    assert_eq!(output_chunks.len(), 1);
    assert_eq!(
        completions.len(),
        1,
        "current provider output should settle non-workflow leased prompts"
    );
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
fn leased_projection_pump_settles_quiet_non_workflow_prompt() {
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
    {
        let mut prompt_activity = app.prompt_activity.write();
        let state = prompt_activity
            .get_mut(&provider_run_id)
            .expect("active leased turn should be tracked");
        state.saw_response_content = true;
        state.last_output_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
    }

    let (_target_kernel_id, event) = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, true)
        .expect("projection drain should succeed")
        .expect("quiet prompt completion should be projected");
    let RelayPeerEvent::LeasedRuntimeProjection { completions, .. } = event;
    assert_eq!(completions.len(), 1);
    assert!(app
        .prompt_owner_active_prompt_for_agent_snapshot(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        )
        .expect("active prompt should load")
        .is_none());
}

#[test]
fn leased_projection_history_completion_is_not_blocked_by_notice() {
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
    app.append_history_entry(
        &leased_agent.backing_session_id,
        crate::history::SessionHistoryEntry::provider_output(
            &leased_agent.backing_session_id,
            &provider_run_id,
            Some(&leased_agent.backing_agent_id),
            TerminalOutputKind::ProviderOutput,
            Some("assistant-1".to_string()),
            "remote output".to_string(),
        ),
    );
    app.terminal_mut().record_notice(
        &leased_agent.backing_session_id,
        Some(&provider_run_id),
        Some(&leased_agent.backing_agent_id),
        vec![leased_agent.backing_attachment_id.clone()],
        "remote notice",
    );
    app.terminal_mut().fan_out_output(
        &leased_agent.backing_session_id,
        &provider_run_id,
        Some(&leased_agent.backing_agent_id),
        TerminalOutputKind::ProviderOutput,
        Some("assistant-1".to_string()),
        vec![leased_agent.backing_attachment_id.clone()],
        b"remote output",
    );

    let (_target_kernel_id, event) = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
        .expect("projection drain should succeed")
        .expect("history-backed completion should be projected with notice");
    let RelayPeerEvent::LeasedRuntimeProjection {
        notices,
        output_chunks,
        completions,
        ..
    } = event;
    assert_eq!(notices, vec!["remote notice".to_string()]);
    assert_eq!(output_chunks.len(), 1);
    assert_eq!(completions.len(), 1);
    assert!(completions[0]
        .message_id
        .contains(&format!("leased-{provider_run_id}-completion")));
}

#[test]
fn leased_projection_recovers_output_from_history_when_terminal_records_are_missing() {
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
    app.append_history_entry(
        &leased_agent.backing_session_id,
        crate::history::SessionHistoryEntry::provider_output(
            &leased_agent.backing_session_id,
            &provider_run_id,
            Some(&leased_agent.backing_agent_id),
            TerminalOutputKind::ProviderOutput,
            Some("assistant-1".to_string()),
            "remote output from history".to_string(),
        ),
    );

    let (_target_kernel_id, event) = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
        .expect("projection drain should succeed")
        .expect("history-backed output and completion should be projected");
    let RelayPeerEvent::LeasedRuntimeProjection {
        output_chunks,
        completions,
        ..
    } = event;
    assert_eq!(output_chunks.len(), 1);
    assert_eq!(
        output_chunks[0].bytes,
        b"remote output from history".to_vec()
    );
    assert_eq!(completions.len(), 1);

    let duplicate = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
        .expect("second projection drain should succeed");
    assert!(
        duplicate.is_none(),
        "history fallback output should not be projected twice"
    );
}

#[test]
fn leased_projection_completion_dedupe_is_prompt_scoped_when_provider_run_is_reused() {
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
        .submit_leased_prompt(&leased_agent.id, "first remote leased prompt\n", Vec::new())
        .expect("first leased prompt should submit");
    assert!(matches!(outcome, PromptSubmissionOutcome::Started { .. }));
    app.append_history_entry(
        &leased_agent.backing_session_id,
        crate::history::SessionHistoryEntry::provider_output(
            &leased_agent.backing_session_id,
            &provider_run_id,
            Some(&leased_agent.backing_agent_id),
            TerminalOutputKind::ProviderOutput,
            Some("assistant-first".to_string()),
            "first remote output".to_string(),
        ),
    );

    let first_projection = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
        .expect("first projection drain should succeed")
        .expect("first prompt should project output and completion");
    let RelayPeerEvent::LeasedRuntimeProjection {
        output_chunks: first_chunks,
        completions: first_completions,
        ..
    } = first_projection.1;
    assert_eq!(first_chunks.len(), 1);
    assert_eq!(first_completions.len(), 1);

    let (reused_provider_run_id, second_outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt(
            &leased_agent.id,
            "second remote leased prompt\n",
            Vec::new(),
        )
        .expect("second leased prompt should submit");
    assert_eq!(
        reused_provider_run_id, provider_run_id,
        "leased agents should reuse the provider run across turns"
    );
    assert!(matches!(
        second_outcome,
        PromptSubmissionOutcome::Started { .. }
    ));
    app.append_history_entry(
        &leased_agent.backing_session_id,
        crate::history::SessionHistoryEntry::provider_output(
            &leased_agent.backing_session_id,
            &provider_run_id,
            Some(&leased_agent.backing_agent_id),
            TerminalOutputKind::ProviderOutput,
            Some("assistant-second".to_string()),
            "second remote output".to_string(),
        ),
    );

    let second_projection = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
        .expect("second projection drain should succeed")
        .expect("second prompt should project a distinct completion");
    let RelayPeerEvent::LeasedRuntimeProjection {
        output_chunks: second_chunks,
        completions: second_completions,
        ..
    } = second_projection.1;
    assert_eq!(second_chunks.len(), 1);
    assert_eq!(second_chunks[0].bytes, b"second remote output".to_vec());
    assert_eq!(second_completions.len(), 1);
    assert_ne!(
        second_completions[0].message_id, first_completions[0].message_id,
        "completion dedupe keys must be turn-scoped for reused provider runs"
    );
}

#[test]
fn leased_projection_recovers_history_output_when_tool_chunks_are_drained() {
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
    app.terminal_mut().fan_out_output(
        &leased_agent.backing_session_id,
        &provider_run_id,
        Some(&leased_agent.backing_agent_id),
        TerminalOutputKind::ProviderTool,
        Some("tool-1".to_string()),
        vec![leased_agent.backing_attachment_id.clone()],
        b"remote tool output",
    );
    app.append_history_entry(
        &leased_agent.backing_session_id,
        crate::history::SessionHistoryEntry::provider_output(
            &leased_agent.backing_session_id,
            &provider_run_id,
            Some(&leased_agent.backing_agent_id),
            TerminalOutputKind::ProviderOutput,
            Some("assistant-1".to_string()),
            "remote assistant output".to_string(),
        ),
    );

    let (_target_kernel_id, event) = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
        .expect("projection drain should succeed")
        .expect("tool chunks and history-backed output should be projected");
    let RelayPeerEvent::LeasedRuntimeProjection { output_chunks, .. } = event;
    assert!(output_chunks.iter().any(|chunk| {
        chunk.kind == TerminalOutputKind::ProviderTool && chunk.bytes == b"remote tool output"
    }));
    assert!(output_chunks.iter().any(|chunk| {
        chunk.kind == TerminalOutputKind::ProviderOutput
            && chunk.bytes == b"remote assistant output"
    }));
}

#[test]
fn leased_projection_history_dedupe_is_scoped_to_backing_session() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease("home-kernel", "session-1", "agent-home-1", "user-home")
        .expect("execution lease should be created");
    let first_worktree = std::env::temp_dir().join(format!(
        "arroba-leased-history-dedupe-a-{}",
        crate::session::unix_epoch_ms()
    ));
    let second_worktree = std::env::temp_dir().join(format!(
        "arroba-leased-history-dedupe-b-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&first_worktree).expect("first leased worktree should exist");
    std::fs::create_dir_all(&second_worktree).expect("second leased worktree should exist");
    let first = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            Some(first_worktree.display().to_string()),
            None,
        )
        .expect("first leased agent should be created");
    let second = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            Some(second_worktree.display().to_string()),
            None,
        )
        .expect("second leased agent should be created");

    let (first_provider_run_id, first_outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt(&first.id, "first remote leased prompt\n", Vec::new())
        .expect("first leased prompt should submit");
    let (second_provider_run_id, second_outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt(&second.id, "second remote leased prompt\n", Vec::new())
        .expect("second leased prompt should submit");
    assert!(matches!(
        first_outcome,
        PromptSubmissionOutcome::Started { .. }
    ));
    assert!(matches!(
        second_outcome,
        PromptSubmissionOutcome::Started { .. }
    ));
    for (leased_agent, provider_run_id, text) in [
        (&first, &first_provider_run_id, "first output"),
        (&second, &second_provider_run_id, "second output"),
    ] {
        app.append_history_entry(
            &leased_agent.backing_session_id,
            crate::history::SessionHistoryEntry::provider_output(
                &leased_agent.backing_session_id,
                provider_run_id,
                Some(&leased_agent.backing_agent_id),
                TerminalOutputKind::ProviderOutput,
                Some(format!("assistant-{text}")),
                text.to_string(),
            ),
        );
    }

    let first_projection = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&first.id, &first_provider_run_id, false)
        .expect("first projection drain should succeed")
        .expect("first history output should project");
    let RelayPeerEvent::LeasedRuntimeProjection {
        output_chunks: first_chunks,
        ..
    } = first_projection.1;
    assert_eq!(first_chunks[0].bytes, b"first output".to_vec());

    RemoteLeaseRuntime::new(&mut app).push_projected_output_history_key_for_test(
        &second.id,
        format!("{}:{second_provider_run_id}", first.backing_session_id),
    );
    let second_projection = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&second.id, &second_provider_run_id, false)
        .expect("second projection drain should succeed")
        .expect("second history output should project despite same run id from another session");
    let RelayPeerEvent::LeasedRuntimeProjection {
        output_chunks: second_chunks,
        ..
    } = second_projection.1;
    assert_eq!(second_chunks[0].bytes, b"second output".to_vec());
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
