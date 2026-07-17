use super::*;

#[test]
fn leased_projection_history_completion_is_not_blocked_by_notice() {
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

    let duplicate = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
        .expect("second projection drain should succeed");
    assert!(
        duplicate.is_none(),
        "live output already matched to history must not replay from history"
    );
}

#[test]
fn leased_projection_recovers_output_from_history_when_terminal_records_are_missing() {
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
        .create_execution_lease(
            "home-kernel",
            "session-1",
            "agent-home-1",
            false,
            "user-home",
        )
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
