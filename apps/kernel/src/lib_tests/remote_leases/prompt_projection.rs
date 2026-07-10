use super::*;

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
fn leased_projection_pull_replays_a_completion_lost_after_worker_drain() {
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
    let home_prompt_id = "home-prompt-1";
    let (provider_run_id, outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt_with_workflow_context(
            &leased_agent.id,
            "remote leased prompt\n",
            Vec::new(),
            None,
            Some(crate::transport::relay_peer::RemoteGitTurnContext {
                home_session_id: "session-1".to_string(),
                home_agent_id: "agent-home-1".to_string(),
                home_prompt_id: home_prompt_id.to_string(),
                home_turn_id: home_prompt_id.to_string(),
                source_attachment_id: None,
                workspace_live_sync_mode: None,
                prompt_origin: Some(PromptOrigin::Arroba),
                external_provider: None,
                external_provider_session_id: None,
                external_provider_turn_id: None,
                prompt_summary: "remote leased prompt".to_string(),
            }),
            Vec::new(),
            crate::extension::RemoteExtensionManifest::default(),
        )
        .expect("leased prompt should submit");
    assert!(matches!(outcome, PromptSubmissionOutcome::Started { .. }));
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
    .expect("backing prompt should settle");

    let first = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
        .expect("first projection drain should succeed")
        .expect("first projection should carry completion");
    let RelayPeerEvent::LeasedRuntimeProjection {
        completions: first_completions,
        ..
    } = first.1;
    assert!(!first_completions.is_empty());
    assert!(first_completions
        .iter()
        .all(|completion| { completion.home_prompt_id.as_deref() == Some(home_prompt_id) }));

    let replay = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection_with_recovery(
            &leased_agent.id,
            &provider_run_id,
            false,
            true,
        )
        .expect("recovery projection drain should succeed")
        .expect("a lost completion should remain replayable");
    let RelayPeerEvent::LeasedRuntimeProjection {
        completions: replayed_completions,
        ..
    } = replay.1;
    assert_eq!(replayed_completions.len(), 1);
    assert_eq!(
        replayed_completions[0].home_prompt_id.as_deref(),
        Some(home_prompt_id)
    );

    let replay_again = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection_with_recovery(
            &leased_agent.id,
            &provider_run_id,
            false,
            true,
        )
        .expect("second recovery projection drain should succeed")
        .expect("completion replay should remain available until home settles");
    let RelayPeerEvent::LeasedRuntimeProjection {
        completions: replayed_again,
        ..
    } = replay_again.1;
    assert_eq!(replayed_again, replayed_completions);
}

#[test]
fn leased_projection_does_not_reflect_home_origin_prompt_back_to_home() {
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
fn leased_projection_does_not_promote_passive_external_observation_to_prompt_or_output() {
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
    let provider_run_id = "provider-run-1";
    for (kind, text, turn_id) in [
        (
            crate::history::SessionHistoryEntryKind::UserPrompt,
            "passive provider prompt",
            "turn-user",
        ),
        (
            crate::history::SessionHistoryEntryKind::ProviderOutput,
            "passive provider output",
            "turn-output",
        ),
    ] {
        app.append_history_entry(
            &leased_agent.backing_session_id,
            crate::history::SessionHistoryEntry::external_provider_observed(
                &leased_agent.backing_session_id,
                Some(provider_run_id),
                &leased_agent.backing_agent_id,
                kind,
                text,
                "codex",
                "thread-1",
                Some(turn_id.to_string()),
                None,
            ),
        );
    }

    let projection = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased_agent.id, provider_run_id, false)
        .expect("projection drain should succeed");
    if let Some((
        _target,
        RelayPeerEvent::LeasedRuntimeProjection {
            prompts,
            output_chunks,
            ..
        },
    )) = projection
    {
        assert!(prompts.is_empty());
        assert!(output_chunks.is_empty());
    }
}

#[test]
fn leased_projection_pump_forwards_completion_after_provider_run_ends() {
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
